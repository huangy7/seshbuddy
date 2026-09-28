//! Gemini 会话解析器。依赖方向：parser → session / parser → parser::shared。
//! 所有跨 CLI 的共享工具在 shared.rs，本模块只保留 Gemini 专用逻辑。

use crate::error::AppResult;
use crate::parser::batch::BatchEmitter;
use crate::parser::claude_entry::TOOL_RESULT_MAX_LEN;
use crate::parser::shared::{read_last_timestamp_from_file, truncate_preview_text};
use crate::session::{self, ChatMessage, ContentPart, UsageRecord};
use serde_json::Value;
use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::Path;

pub(crate) fn is_gemini_metadata_line(entry: &Value) -> bool {
    entry.get("$set").is_some() || (entry.get("sessionId").is_some() && entry.get("type").is_none())
}

pub(crate) fn parse_gemini_line(line: &str) -> Option<ChatMessage> {
    let entry: Value = serde_json::from_str(line.trim()).ok()?;
    if is_gemini_metadata_line(&entry) {
        return None;
    }

    let entry_type = entry.get("type").and_then(|v| v.as_str())?;
    let timestamp = entry
        .get("timestamp")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let model = entry
        .get("model")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    match entry_type {
        "user" => {
            let content_parts = parse_gemini_user_content(&entry);
            if content_parts.is_empty() {
                return None;
            }
            Some(ChatMessage {
                role: "user".to_string(),
                timestamp,
                model: None,
                token_usage: None,
                content_parts,
                is_meta: false,
                uuid: None,
            })
        }
        "gemini" => {
            let mut content_parts = Vec::new();

            if let Some(thoughts) = entry.get("thoughts").and_then(|v| v.as_array()) {
                for thought in thoughts {
                    let subject = thought
                        .get("subject")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let description = thought
                        .get("description")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let thinking = if subject.is_empty() {
                        description.to_string()
                    } else if description.is_empty() {
                        subject.to_string()
                    } else {
                        format!("{}: {}", subject, description)
                    };
                    if !thinking.trim().is_empty() {
                        content_parts.push(ContentPart::Thinking { thinking });
                    }
                }
            }

            if let Some(text) = entry.get("content").and_then(|v| v.as_str()) {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    content_parts.push(ContentPart::Text {
                        text: trimmed.to_string(),
                    });
                }
            }

            if let Some(tool_calls) = entry.get("toolCalls").and_then(|v| v.as_array()) {
                for tc in tool_calls {
                    if let Some(tool_use) = parse_gemini_tool_call(tc) {
                        content_parts.push(tool_use);
                    }
                    if let Some(tool_result) = parse_gemini_tool_result(tc) {
                        content_parts.push(tool_result);
                    }
                }
            }

            if content_parts.is_empty() {
                return None;
            }

            Some(ChatMessage {
                role: "assistant".to_string(),
                timestamp,
                model,
                token_usage: None,
                content_parts,
                is_meta: false,
                uuid: None,
            })
        }
        _ => None,
    }
}

pub(crate) fn parse_gemini_user_content(entry: &Value) -> Vec<ContentPart> {
    let display = entry.get("displayContent").and_then(|v| v.as_array());
    if let Some(items) = display {
        let mut parts = Vec::new();
        for item in items {
            if let Some(text) = item.get("text").and_then(|v| v.as_str()) {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    parts.push(ContentPart::Text {
                        text: trimmed.to_string(),
                    });
                }
            }
        }
        if !parts.is_empty() {
            return parts;
        }
    }

    if let Some(items) = entry.get("content").and_then(|v| v.as_array()) {
        let mut parts = Vec::new();
        for item in items {
            if let Some(text) = item.get("text").and_then(|v| v.as_str()) {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    parts.push(ContentPart::Text {
                        text: trimmed.to_string(),
                    });
                }
            }
        }
        if !parts.is_empty() {
            return parts;
        }
    }

    if let Some(text) = entry.get("content").and_then(|v| v.as_str()) {
        let trimmed = text.trim();
        if !trimmed.is_empty() {
            return vec![ContentPart::Text {
                text: trimmed.to_string(),
            }];
        }
    }

    Vec::new()
}

pub(crate) fn parse_gemini_tool_call(tc: &Value) -> Option<ContentPart> {
    let tool_name = tc.get("name").and_then(|v| v.as_str())?.to_string();
    let args = tc
        .get("args")
        .cloned()
        .unwrap_or(Value::Object(serde_json::Map::new()));
    let summary = tc
        .get("description")
        .and_then(|v| v.as_str())
        .or_else(|| tc.get("displayName").and_then(|v| v.as_str()))
        .map(|s| s.to_string())
        .unwrap_or_else(|| session::tool_use_summary(&tool_name, &args));
    let input = serde_json::to_string_pretty(&args).unwrap_or_default();
    let tool_use_id = tc.get("id").and_then(|v| v.as_str()).map(|s| s.to_string());

    Some(ContentPart::ToolUse {
        summary,
        tool_name,
        input,
        tool_use_id,
    })
}

pub(crate) fn parse_gemini_tool_result(tc: &Value) -> Option<ContentPart> {
    let result_array = tc.get("result").and_then(|v| v.as_array())?;
    let first = result_array.first()?;
    let response = first
        .get("functionResponse")
        .and_then(|v| v.get("response"))
        .and_then(|v| v.get("output"))
        .and_then(|v| v.as_str())?;
    let mut content = response.trim().to_string();
    let mut truncated = false;
    // 与 claude_entry 共用同一个上限：50 KB 是用户可见的截断行为，各写一份会各自漂移
    if content.len() > TOOL_RESULT_MAX_LEN {
        let mut cut = TOOL_RESULT_MAX_LEN;
        while cut > 0 && !content.is_char_boundary(cut) {
            cut -= 1;
        }
        content.truncate(cut);
        truncated = true;
    }
    if content.is_empty() {
        return None;
    }
    let status = tc
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("success");
    let is_error = status == "error" || status == "failure";
    let tool_name = tc.get("name").and_then(|v| v.as_str()).unwrap_or("Tool");
    let summary = if is_error {
        format!("[{} error]", tool_name)
    } else {
        format!("[{} result]", tool_name)
    };

    Some(ContentPart::tool_result(summary, content, is_error, truncated))
}

/// Gemini 归档内容解析：与 parse_gemini_session_file 相同的行门与行解析。
pub(crate) fn parse_gemini_session_from_string(content: &str) -> Vec<session::ChatMessage> {
    let mut messages = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut check_len = std::cmp::min(line.len(), 1024);
        while check_len > 0 && !line.is_char_boundary(check_len) {
            check_len -= 1;
        }
        let prefix = &line[..check_len];
        if !prefix.contains(r#""type":"user""#)
            && !prefix.contains(r#""type": "user""#)
            && !prefix.contains(r#""type":"gemini""#)
            && !prefix.contains(r#""type": "gemini""#)
        {
            continue;
        }
        if let Some(message) = parse_gemini_line(line) {
            messages.push(message);
        }
    }
    messages
}

pub(crate) fn parse_gemini_session_file(file_path: &str) -> AppResult<session::SessionLoadResult> {
    let file = File::open(file_path)?;
    let file_size = file.metadata()?.len();
    let mut reader = BufReader::new(file);
    let mut messages = Vec::new();
    let mut buf = String::new();

    loop {
        buf.clear();
        match reader.read_line(&mut buf) {
            Ok(0) => break,
            Err(_) => continue,
            Ok(_) => {}
        }
        let line = buf.trim();
        if line.is_empty() {
            continue;
        }
        let mut check_len = std::cmp::min(line.len(), 1024);
        while check_len > 0 && !line.is_char_boundary(check_len) {
            check_len -= 1;
        }
        let prefix = &line[..check_len];
        if !prefix.contains(r#""type":"user""#)
            && !prefix.contains(r#""type": "user""#)
            && !prefix.contains(r#""type":"gemini""#)
            && !prefix.contains(r#""type": "gemini""#)
        {
            continue;
        }
        if let Some(message) = parse_gemini_line(line) {
            messages.push(message);
        }
    }

    Ok(session::SessionLoadResult {
        messages,
        offset: file_size,
        subagent_map: std::collections::HashMap::new(),
    })
}

pub(crate) fn parse_gemini_session_incremental(
    file_path: &str,
    offset: u64,
) -> AppResult<session::SessionLoadResult> {
    let mut file = File::open(file_path)?;
    let file_size = file.metadata()?.len();

    if file_size <= offset {
        return Ok(session::SessionLoadResult {
            messages: vec![],
            offset,
            subagent_map: std::collections::HashMap::new(),
        });
    }

    file.seek(SeekFrom::Start(offset))?;
    let mut reader = BufReader::new(file);
    let mut messages = Vec::new();
    let mut buf = String::new();

    loop {
        buf.clear();
        match reader.read_line(&mut buf) {
            Ok(0) => break,
            Err(_) => continue,
            Ok(_) => {}
        }
        let line = buf.trim();
        if line.is_empty() {
            continue;
        }
        let mut check_len = std::cmp::min(line.len(), 1024);
        while check_len > 0 && !line.is_char_boundary(check_len) {
            check_len -= 1;
        }
        let prefix = &line[..check_len];
        if !prefix.contains(r#""type":"user""#)
            && !prefix.contains(r#""type": "user""#)
            && !prefix.contains(r#""type":"gemini""#)
            && !prefix.contains(r#""type": "gemini""#)
        {
            continue;
        }
        if let Some(message) = parse_gemini_line(line) {
            messages.push(message);
        }
    }

    Ok(session::SessionLoadResult {
        messages,
        offset: file_size,
        subagent_map: std::collections::HashMap::new(),
    })
}

pub(crate) fn parse_gemini_session_file_streaming<F>(
    file_path: &str,
    mut on_batch: F,
) -> AppResult<(
    u64,
    std::collections::HashMap<String, session::SubagentInfo>,
)>
where
    F: FnMut(Vec<session::ChatMessage>) -> bool,
{
    let file = File::open(file_path)?;
    let file_size = file.metadata()?.len();
    let mut reader = BufReader::with_capacity(256 * 1024, file);
    let mut buf = String::new();
    let mut emitter = BatchEmitter::new();

    loop {
        buf.clear();
        match reader.read_line(&mut buf) {
            Ok(0) => break,
            Err(_) => continue,
            Ok(_) => {}
        }
        let line = buf.trim();
        if line.is_empty() {
            continue;
        }
        let mut check_len = std::cmp::min(line.len(), 1024);
        while check_len > 0 && !line.is_char_boundary(check_len) {
            check_len -= 1;
        }
        let prefix = &line[..check_len];
        if !prefix.contains(r#""type":"user""#)
            && !prefix.contains(r#""type": "user""#)
            && !prefix.contains(r#""type":"gemini""#)
            && !prefix.contains(r#""type": "gemini""#)
        {
            continue;
        }
        if let Some(message) = parse_gemini_line(line) {
            emitter.push(message);
            if !emitter.maybe_flush(&mut on_batch) {
                return Ok((file_size, std::collections::HashMap::new()));
            }
        }
    }

    emitter.flush_remaining(&mut on_batch);

    Ok((file_size, std::collections::HashMap::new()))
}

pub(crate) fn read_gemini_header_field(file_path: &str, field: &str) -> Option<String> {
    let file = File::open(file_path).ok()?;
    let reader = BufReader::new(file);

    for line in reader.lines().take(5) {
        let line = match line {
            Ok(line) => line,
            Err(_) => continue,
        };
        let entry: Value = serde_json::from_str(line.trim()).ok()?;
        if let Some(value) = entry.get(field).and_then(|v| v.as_str()) {
            return Some(value.to_string());
        }
    }
    None
}

pub(crate) fn read_gemini_first_user_message(file_path: &str) -> Option<String> {
    let file = File::open(file_path).ok()?;
    let reader = BufReader::new(file);

    for line in reader.lines().take(120) {
        let line = match line {
            Ok(line) => line,
            Err(_) => continue,
        };
        let entry: Value = match serde_json::from_str(line.trim()) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let Some((role, text)) = extract_gemini_role_text(&entry) else {
            continue;
        };
        if role != "user" || text.is_empty() {
            continue;
        }
        // 报告预览惯例：先清洗（系统标签/图片标记），再截断 30 字
        let cleaned = crate::title_resolver::clean_fallback_user_text(&text)?;
        return truncate_preview_text(&cleaned, 30);
    }
    None
}

pub(crate) fn extract_gemini_role_text(entry: &Value) -> Option<(String, String)> {
    if is_gemini_metadata_line(entry) {
        return None;
    }
    let entry_type = entry.get("type").and_then(|v| v.as_str())?;
    match entry_type {
        "user" => {
            let parts = parse_gemini_user_content(entry);
            let text: String = parts
                .iter()
                .filter_map(|p| match p {
                    ContentPart::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n");
            if text.is_empty() {
                None
            } else {
                Some(("user".to_string(), text))
            }
        }
        "gemini" => {
            let mut texts = Vec::new();
            if let Some(text) = entry.get("content").and_then(|v| v.as_str()) {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    texts.push(trimmed.to_string());
                }
            }
            if let Some(tool_calls) = entry.get("toolCalls").and_then(|v| v.as_array()) {
                for tc in tool_calls {
                    if let Some(name) = tc.get("name").and_then(|v| v.as_str()) {
                        texts.push(name.to_string());
                    }
                }
            }
            if texts.is_empty() {
                None
            } else {
                Some(("assistant".to_string(), texts.join("\n")))
            }
        }
        _ => None,
    }
}

pub(crate) fn extract_gemini_usage_records(file_path: &str, project: &str) -> Vec<UsageRecord> {
    let file = match File::open(file_path) {
        Ok(f) => f,
        Err(_) => return Vec::new(),
    };
    // 与主解析路径一致的缓冲（256 KB），避免逐行 syscall
    let reader = BufReader::with_capacity(256 * 1024, file);
    let mut records = Vec::new();

    for line in reader.lines() {
        let line = match line {
            Ok(line) => line,
            Err(_) => continue,
        };
        let entry: Value = match serde_json::from_str(line.trim()) {
            Ok(v) => v,
            Err(_) => continue,
        };

        if entry.get("type").and_then(|v| v.as_str()) != Some("gemini") {
            continue;
        }
        let Some(tokens) = entry.get("tokens") else {
            continue;
        };
        let model = entry
            .get("model")
            .and_then(|v| v.as_str())
            .unwrap_or("gemini-unknown")
            .to_string();
        let timestamp = entry
            .get("timestamp")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let date = if timestamp.len() >= 10 {
            timestamp[..10].to_string()
        } else {
            String::new()
        };

        records.push(UsageRecord {
            date,
            model,
            input_tokens: tokens.get("input").and_then(|v| v.as_u64()).unwrap_or(0),
            output_tokens: tokens.get("output").and_then(|v| v.as_u64()).unwrap_or(0),
            cache_creation_tokens: 0,
            cache_read_tokens: tokens.get("cached").and_then(|v| v.as_u64()).unwrap_or(0),
            duration_ms: None,
            project: project.to_string(),
        });
    }

    records
}

pub(crate) fn scan_gemini_metadata_only(file_path: &Path) -> Option<session::SessionListMetadata> {
    let mut file = File::open(file_path).ok()?;
    let file_size = file.seek(SeekFrom::End(0)).ok()?;
    file.seek(SeekFrom::Start(0)).ok()?;

    let reader = BufReader::new(&file);
    let mut has_chat_messages = false;
    let mut session_id = None;
    let mut first_user_message = None;
    let mut first_timestamp = None;

    const HEAD_LINES: usize = 50;
    for line in reader.lines().take(HEAD_LINES) {
        let line = match line {
            Ok(line) => line,
            Err(_) => continue,
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let entry: Value = match serde_json::from_str(trimmed) {
            Ok(entry) => entry,
            Err(_) => continue,
        };

        if session_id.is_none() {
            if let Some(sid) = entry.get("sessionId").and_then(|v| v.as_str()) {
                session_id = Some(sid.to_string());
            }
        }

        if first_timestamp.is_none() {
            if let Some(ts) = entry
                .get("timestamp")
                .or_else(|| entry.get("startTime"))
                .and_then(|v| v.as_str())
            {
                first_timestamp = Some(ts.to_string());
            }
        }

        if !has_chat_messages {
            if let Some((role, text)) = extract_gemini_role_text(&entry) {
                has_chat_messages = true;
                if role == "user" {
                    first_user_message = crate::title_resolver::clean_fallback_user_text(&text);
                }
            }
        }
    }

    if !has_chat_messages {
        return None;
    }

    let last_timestamp = read_last_timestamp_from_file(&mut file, file_size);
    let fallback_session_id = file_path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .and_then(|stem| stem.rsplit('-').next())
        .unwrap_or("")
        .to_string();

    Some(session::SessionListMetadata {
        session_id: session_id.unwrap_or(fallback_session_id),
        project_path: None,
        title: None,
        first_user_message,
        first_timestamp: first_timestamp.clone(),
        last_timestamp: last_timestamp.or(first_timestamp),
        git_branch: String::new(),
        file_size,
    })
}
