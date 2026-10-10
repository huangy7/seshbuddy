//! Aider 会话转录解析器。
//!
//! Aider 聊天历史采用 Markdown 格式（项目根目录下 `.aider.chat.history.md`）：
//! - `# aider chat started at YYYY-MM-DD HH:MM:SS` 标记会话启动时间与边界；
//! - `#### ` 前缀行代表用户输入（多行提示词每行均带有 `#### ` 前缀）；
//! - `> ` 前缀行代表工具执行、文件改动、Git 提交与环境反馈；
//! - 其余普通 Markdown 正文为助手回复。

use std::fs;
use std::path::Path;

use chrono::{Datelike, Local, NaiveDateTime, TimeZone, Timelike};
use sha2::{Digest, Sha256};

use crate::error::AppResult;
use crate::parser::shared::{SearchDocument, SearchScanProgress};
use crate::session::{ChatMessage, ContentPart, SessionListMetadata, TokenUsage};
use crate::title_resolver::clean_fallback_user_text;

/// 从会话路径计算稳定且具有辨识度的 session_id。
///
/// 若文件名是默认的 `.aider.chat.history.md`，单纯取文件名会导致多个项目的会话 ID 冲突。
/// 此时结合父目录（项目名）与全路径散列前 8 位生成唯一 ID（如 `my-app-aider-a1b2c3d4`）。
pub(crate) fn session_id_from_path(path: &Path) -> String {
    let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let parent = path
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .filter(|n| !n.is_empty())
        .unwrap_or("aider");

    if file_name == ".aider.chat.history.md" || file_name.ends_with(".chat.history.md") {
        let mut hasher = Sha256::new();
        hasher.update(path.to_string_lossy().as_bytes());
        let hash_hex = format!("{:x}", hasher.finalize());
        format!("{parent}-aider-{}", &hash_hex[..8])
    } else {
        path.file_stem()
            .and_then(|stem| stem.to_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("aider-session")
            .to_string()
    }
}

/// 解析 Aider 的时间戳标记行：`# aider chat started at YYYY-MM-DD HH:MM:SS`
pub(crate) fn parse_aider_timestamp(line: &str) -> Option<String> {
    let trimmed = line.trim();
    let marker = "# aider chat started at ";
    if !trimmed.starts_with(marker) {
        return None;
    }
    let rest = trimmed[marker.len()..].trim();
    if rest.len() < 19 {
        return None;
    }
    let dt_str = &rest[..19];
    let ndt = NaiveDateTime::parse_from_str(dt_str, "%Y-%m-%d %H:%M:%S").ok()?;
    match Local.from_local_datetime(&ndt).single() {
        Some(local_dt) => Some(local_dt.to_rfc3339()),
        None => Some(format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
            ndt.year(),
            ndt.month(),
            ndt.day(),
            ndt.hour(),
            ndt.minute(),
            ndt.second()
        )),
    }
}

/// 解析 Token 数量字符串（支持数字、千分位逗号与 `k`/`K` 后缀）。
fn parse_token_count(text: &str) -> Option<u64> {
    let cleaned = text.trim().replace(',', "");
    if cleaned.ends_with('k') || cleaned.ends_with('K') {
        let num_part = &cleaned[..cleaned.len() - 1];
        let val: f64 = num_part.parse().ok()?;
        Some((val * 1000.0) as u64)
    } else {
        cleaned.parse::<u64>().ok()
    }
}

/// 从工具输出行中解析 Token 用量统计（格式形如 `Tokens: 2.5k sent, 410 received.`）。
fn parse_tokens_from_line(line: &str) -> Option<TokenUsage> {
    let trimmed = line.trim();
    let lower = trimmed.to_ascii_lowercase();
    if !lower.starts_with("tokens:") {
        return None;
    }
    let rest = &trimmed["tokens:".len()..];
    let parts: Vec<&str> = rest.split(',').collect();
    if parts.len() < 2 {
        return None;
    }

    let sent_part = parts[0].trim();
    let recv_part = parts[1].trim();

    let input_tokens = sent_part
        .split_whitespace()
        .next()
        .and_then(parse_token_count)
        .unwrap_or(0);

    let output_tokens = recv_part
        .split_whitespace()
        .next()
        .and_then(parse_token_count)
        .unwrap_or(0);

    if input_tokens > 0 || output_tokens > 0 {
        Some(TokenUsage {
            input_tokens,
            output_tokens,
            cache_creation_input_tokens: 0,
            cache_read_input_tokens: 0,
        })
    } else {
        None
    }
}

/// 从工具输出行提炼概要标题。
fn summarize_tool_output(lines: &[String]) -> String {
    if let Some(edit) = lines.iter().find(|l| l.trim().starts_with("Applied edit to ")) {
        return edit.trim().to_string();
    }
    if let Some(commit) = lines.iter().find(|l| l.trim().starts_with("Commit ")) {
        return commit.trim().to_string();
    }
    if let Some(added) = lines.iter().find(|l| l.trim().starts_with("Added ") && l.contains("to the chat")) {
        return added.trim().to_string();
    }
    lines
        .iter()
        .map(|s| s.trim())
        .find(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "Tool Output".to_string())
}

/// 解析 Aider 聊天历史 Markdown 内容为统一的消息序列。
pub(crate) fn parse_from_content(content: &str) -> Vec<ChatMessage> {
    let mut messages: Vec<ChatMessage> = Vec::new();
    let mut current_timestamp = String::new();
    let mut current_model: Option<String> = None;
    let mut latest_tokens: Option<TokenUsage> = None;

    let mut user_lines: Vec<String> = Vec::new();
    let mut assistant_parts: Vec<ContentPart> = Vec::new();
    let mut current_tool_lines: Vec<String> = Vec::new();
    let mut current_assistant_text: Vec<String> = Vec::new();
    let mut seen_user_message = false;

    let flush_tool = |parts: &mut Vec<ContentPart>, lines: &mut Vec<String>| {
        if lines.is_empty() {
            return;
        }
        let summary = summarize_tool_output(lines);
        let tool_content = lines.join("\n");
        let is_error = lines.iter().any(|l| {
            let lower = l.to_ascii_lowercase();
            lower.contains("error") || lower.contains("failed") || lower.contains("traceback")
        });
        lines.clear();
        parts.push(ContentPart::tool_result(summary, tool_content, is_error, false));
    };

    let flush_text = |parts: &mut Vec<ContentPart>, lines: &mut Vec<String>| {
        if lines.is_empty() {
            return;
        }
        let text = lines.join("\n").trim().to_string();
        lines.clear();
        if !text.is_empty() {
            parts.push(ContentPart::Text { text });
        }
    };

    let flush_assistant = |messages: &mut Vec<ChatMessage>,
                           parts: &mut Vec<ContentPart>,
                           tool_lines: &mut Vec<String>,
                           text_lines: &mut Vec<String>,
                           timestamp: &str,
                           model: Option<String>,
                           tokens: Option<TokenUsage>| {
        flush_tool(parts, tool_lines);
        flush_text(parts, text_lines);
        if !parts.is_empty() {
            let drained: Vec<ContentPart> = parts.drain(..).collect();
            messages.push(ChatMessage {
                role: "assistant".to_string(),
                timestamp: timestamp.to_string(),
                model,
                token_usage: tokens,
                content_parts: drained,
                is_meta: false,
                uuid: None,
            });
        }
    };

    let flush_user = |messages: &mut Vec<ChatMessage>,
                      user_lines: &mut Vec<String>,
                      timestamp: &str| {
        if user_lines.is_empty() {
            return;
        }
        let user_text = user_lines.join("\n").trim().to_string();
        user_lines.clear();
        if !user_text.is_empty() {
            messages.push(ChatMessage {
                role: "user".to_string(),
                timestamp: timestamp.to_string(),
                model: None,
                token_usage: None,
                content_parts: vec![ContentPart::Text { text: user_text }],
                is_meta: false,
                uuid: None,
            });
        }
    };

    for line in content.lines() {
        if line.starts_with("# aider chat started at ") {
            if let Some(ts) = parse_aider_timestamp(line) {
                flush_user(&mut messages, &mut user_lines, &current_timestamp);
                flush_assistant(
                    &mut messages,
                    &mut assistant_parts,
                    &mut current_tool_lines,
                    &mut current_assistant_text,
                    &current_timestamp,
                    current_model.clone(),
                    latest_tokens.take(),
                );
                current_timestamp = ts;
            }
            continue;
        }

        if let Some(user_content) = line.strip_prefix("#### ") {
            seen_user_message = true;
            flush_assistant(
                &mut messages,
                &mut assistant_parts,
                &mut current_tool_lines,
                &mut current_assistant_text,
                &current_timestamp,
                current_model.clone(),
                latest_tokens.take(),
            );
            user_lines.push(user_content.trim_end().to_string());
            continue;
        }

        if let Some(tool_content) = line.strip_prefix("> ") {
            let trimmed_tool = tool_content.trim();
            if trimmed_tool.starts_with("Main model: ") {
                let model_part = &trimmed_tool["Main model: ".len()..];
                let model_name = model_part
                    .split_whitespace()
                    .next()
                    .unwrap_or(model_part)
                    .trim()
                    .to_string();
                if !model_name.is_empty() {
                    current_model = Some(model_name);
                }
            }

            if let Some(tokens) = parse_tokens_from_line(trimmed_tool) {
                latest_tokens = Some(tokens);
            }

            // 首条用户消息之前的启动配置信息仅用于提炼元数据，不作为对话工具气泡输出
            if !seen_user_message {
                continue;
            }

            flush_user(&mut messages, &mut user_lines, &current_timestamp);
            flush_text(&mut assistant_parts, &mut current_assistant_text);
            current_tool_lines.push(tool_content.trim_end().to_string());
            continue;
        }

        // 首条用户消息之前的空白或标题不进正文
        if !seen_user_message {
            continue;
        }

        flush_user(&mut messages, &mut user_lines, &current_timestamp);
        flush_tool(&mut assistant_parts, &mut current_tool_lines);

        if !line.trim().is_empty() || !current_assistant_text.is_empty() {
            current_assistant_text.push(line.trim_end().to_string());
        }
    }

    flush_user(&mut messages, &mut user_lines, &current_timestamp);
    flush_assistant(
        &mut messages,
        &mut assistant_parts,
        &mut current_tool_lines,
        &mut current_assistant_text,
        &current_timestamp,
        current_model,
        latest_tokens,
    );

    messages
}

pub(crate) fn parse_session_file(path: &Path) -> AppResult<Vec<ChatMessage>> {
    let content = fs::read_to_string(path)?;
    Ok(parse_from_content(&content))
}

pub(crate) fn session_id(path: &Path) -> Option<String> {
    Some(session_id_from_path(path))
}

pub(crate) fn project_path_raw(path: &Path) -> Option<String> {
    path.parent()
        .map(|p| p.to_string_lossy().to_string())
        .filter(|p| !p.trim().is_empty())
}

pub(crate) fn is_subagent(_path: &Path) -> bool {
    false
}

pub(crate) fn first_user_message(path: &Path) -> Option<String> {
    let content = fs::read_to_string(path).ok()?;
    for line in content.lines() {
        if let Some(user_content) = line.strip_prefix("#### ") {
            if let Some(cleaned) = clean_fallback_user_text(user_content.trim()) {
                return Some(cleaned);
            }
        }
    }
    None
}

pub(crate) fn metadata_only(path: &Path) -> Option<SessionListMetadata> {
    let file_size = fs::metadata(path).ok()?.len();
    let content = fs::read_to_string(path).ok()?;

    let mut first_ts: Option<String> = None;
    let mut last_ts: Option<String> = None;
    let mut first_user: Option<String> = None;

    for line in content.lines() {
        if line.starts_with("# aider chat started at ") {
            if let Some(ts) = parse_aider_timestamp(line) {
                if first_ts.is_none() {
                    first_ts = Some(ts.clone());
                }
                last_ts = Some(ts);
            }
        } else if first_user.is_none() {
            if let Some(user_content) = line.strip_prefix("#### ") {
                if let Some(cleaned) = clean_fallback_user_text(user_content.trim()) {
                    first_user = Some(cleaned);
                }
            }
        }
    }

    let session_id = session_id_from_path(path);
    let project_path = project_path_raw(path);

    Some(SessionListMetadata {
        session_id,
        project_path,
        title: None,
        first_user_message: first_user,
        first_timestamp: first_ts.clone(),
        last_timestamp: last_ts.or(first_ts),
        git_branch: String::new(),
        file_size,
    })
}

pub(crate) fn search_docs_from_text(content: &[u8]) -> Option<Vec<SearchDocument>> {
    let text = std::str::from_utf8(content).ok()?;
    let messages = parse_from_content(text);
    if messages.is_empty() {
        return None;
    }

    let mut docs = Vec::new();
    for (idx, msg) in messages.iter().enumerate() {
        for part in &msg.content_parts {
            match part {
                ContentPart::Text { text } => {
                    if !text.trim().is_empty() {
                        docs.push(SearchDocument {
                            message_index: idx,
                            search_text: text.clone(),
                            role: msg.role.clone(),
                        });
                    }
                }
                ContentPart::ToolResult {
                    summary, content, ..
                } => {
                    let mut text = summary.clone();
                    if !content.trim().is_empty() {
                        text.push(' ');
                        text.push_str(content);
                    }
                    if !text.trim().is_empty() {
                        docs.push(SearchDocument {
                            message_index: idx,
                            search_text: text,
                            role: "tool".to_string(),
                        });
                    }
                }
                _ => {}
            }
        }
    }

    Some(docs)
}

pub(crate) fn scan_search_docs(
    path: &Path,
    on_progress: &mut dyn FnMut(SearchScanProgress),
) -> Option<Vec<SearchDocument>> {
    let bytes = fs::read(path).ok()?;
    let len = bytes.len() as u64;
    on_progress(SearchScanProgress {
        bytes_read: len,
        file_size: len,
    });
    search_docs_from_text(&bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_aider_timestamp() {
        let line = "# aider chat started at 2026-03-01 10:20:30";
        let ts = parse_aider_timestamp(line).expect("应能解析时间戳");
        assert!(ts.starts_with("2026-03-01T10:20:30"));
    }

    #[test]
    fn test_empty_content_returns_empty_messages() {
        let messages = parse_from_content("");
        assert!(messages.is_empty());
    }

    #[test]
    fn test_single_turn_user_and_assistant() {
        let content = concat!(
            "# aider chat started at 2026-03-01 10:00:00\n",
            "\n",
            "#### Hello Aider\n",
            "\n",
            "Hello! How can I help you today?\n"
        );
        let messages = parse_from_content(content);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, "user");
        match &messages[0].content_parts[0] {
            ContentPart::Text { text } => assert_eq!(text, "Hello Aider"),
            _ => panic!("Expected Text part"),
        }
        assert_eq!(messages[1].role, "assistant");
        match &messages[1].content_parts[0] {
            ContentPart::Text { text } => assert_eq!(text, "Hello! How can I help you today?"),
            _ => panic!("Expected Text part"),
        }
    }

    #[test]
    fn test_multiline_user_prompt() {
        let content = concat!(
            "# aider chat started at 2026-03-01 10:00:00\n",
            "#### Line one of prompt  \n",
            "#### Line two of prompt\n",
            "\n",
            "Understood.\n"
        );
        let messages = parse_from_content(content);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, "user");
        match &messages[0].content_parts[0] {
            ContentPart::Text { text } => {
                assert!(text.contains("Line one of prompt"));
                assert!(text.contains("Line two of prompt"));
            }
            _ => panic!("Expected Text part"),
        }
    }

    #[test]
    fn test_tool_result_and_tokens_parsing() {
        let content = concat!(
            "# aider chat started at 2026-03-01 10:00:00\n",
            "> Main model: claude-3-5-sonnet-20241022 with diff edit format\n",
            "\n",
            "#### Add login button\n",
            "\n",
            "> Added src/Login.vue to the chat.\n",
            "> Applied edit to src/Login.vue\n",
            "> Commit 9a8b7c6: Add login button\n",
            "> Tokens: 2.5k sent, 410 received.\n",
            "\n",
            "Added the login button to src/Login.vue.\n"
        );
        let messages = parse_from_content(content);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, "user");
        assert_eq!(messages[1].role, "assistant");
        assert_eq!(
            messages[1].model.as_deref(),
            Some("claude-3-5-sonnet-20241022")
        );

        let tokens = messages[1].token_usage.as_ref().expect("应当解析出用量");
        assert_eq!(tokens.input_tokens, 2500);
        assert_eq!(tokens.output_tokens, 410);

        assert_eq!(messages[1].content_parts.len(), 2);
        match &messages[1].content_parts[0] {
            ContentPart::ToolResult {
                summary, content, ..
            } => {
                assert_eq!(summary, "Applied edit to src/Login.vue");
                assert!(content.contains("Commit 9a8b7c6"));
            }
            _ => panic!("Expected ToolResult part"),
        }
    }

    #[test]
    fn test_search_docs_generation() {
        let content = concat!(
            "# aider chat started at 2026-03-01 10:00:00\n",
            "#### Find needle in haystack\n",
            "> Applied edit to search.rs\n",
            "Here is the needle found.\n"
        );
        let docs = search_docs_from_text(content.as_bytes()).expect("应当生成搜索文档");
        assert_eq!(docs.len(), 3);
        assert!(docs[0].search_text.contains("needle"));
        assert_eq!(docs[0].role, "user");
        assert!(docs[1].search_text.contains("Applied edit"));
        assert_eq!(docs[1].role, "tool");
        assert!(docs[2].search_text.contains("needle found"));
        assert_eq!(docs[2].role, "assistant");
    }

    #[test]
    fn test_session_id_from_path() {
        let path = Path::new("/workspace/project-foo/.aider.chat.history.md");
        let id = session_id_from_path(path);
        assert!(id.starts_with("project-foo-aider-"));

        let custom_path = Path::new("/workspace/project-foo/feature-bar.md");
        let custom_id = session_id_from_path(custom_path);
        assert_eq!(custom_id, "feature-bar");
    }
}
