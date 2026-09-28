//! WorkBuddy 会话解析器。
//!
//! 格式要点（实测 ~/.workbuddy/projects/<cwd-slug>/<session-id>.jsonl）：
//! - 行信封：id/parentId/timestamp(毫秒数字)/type/sessionId/cwd
//! - type=="message" + role:user → content[] 的 input_text 块（真实提问在最后一个
//!   <user_query> 内，上下文包在 <system-reminder data-role="user-context">，
//!   压缩行以 <cb_summary> 开头）；assistant → output_text 块
//! - function_call / function_call_result：OpenAI 风格，callId 关联，arguments 为 JSON 字符串
//! - reasoning → rawContent[] 的 reasoning_text；ai-title → aiTitle（可能在文件尾部）

use crate::error::AppResult;
use crate::parser::batch::BatchEmitter;
use crate::parser::claude_entry::TOOL_RESULT_MAX_LEN;
use crate::parser::shared::{
    codex_tool_input_value, codex_tool_result_is_error, codex_value_to_text,
    read_typed_titles_from_tail, truncate_preview_text,
};
use crate::session::{self, ChatMessage, ContentPart, SessionListMetadata, SessionLoadResult};
use serde_json::Value;
use std::fs::File;
use std::io::{BufRead, BufReader, Read as _, Seek, SeekFrom};
use std::path::Path;

/// WorkBuddy 归档内容解析：与 parse_workbuddy_session_file 相同的行门与行解析。
pub(crate) fn parse_workbuddy_session_from_string(content: &str) -> Vec<session::ChatMessage> {
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
        if !workbuddy_line_candidate(&line[..check_len]) {
            continue;
        }
        if let Some(message) = parse_workbuddy_line(line) {
            messages.push(message);
        }
    }
    messages
}

/// WorkBuddy 时间戳：毫秒数字 → RFC3339 字符串；兼容字符串格式漂移
pub(crate) fn workbuddy_timestamp(entry: &Value) -> String {
    if let Some(ms) = entry.get("timestamp").and_then(|v| v.as_i64()) {
        if let Some(dt) = chrono::DateTime::from_timestamp_millis(ms) {
            return dt.to_rfc3339();
        }
    }
    entry
        .get("timestamp")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

/// 清洗 WorkBuddy 用户文本：取最后一个 <user_query> 内容，剥离 system-reminder 等系统标签。
/// <cb_summary> 压缩检查点返回 None（不展示、不索引）。
pub(crate) fn workbuddy_clean_user_text(raw: &str) -> Option<String> {
    if raw.trim_start().starts_with("<cb_summary") {
        return None;
    }
    let text = match (raw.rfind("<user_query>"), raw.rfind("</user_query>")) {
        (Some(s), Some(e)) if e > s => raw[s + "<user_query>".len()..e].trim().to_string(),
        _ => raw.to_string(),
    };
    crate::title_resolver::clean_fallback_user_text(&text)
}

pub(crate) fn parse_workbuddy_line(line: &str) -> Option<ChatMessage> {
    let entry: Value = serde_json::from_str(line.trim()).ok()?;
    let entry_type = entry.get("type").and_then(|v| v.as_str())?;
    let timestamp = workbuddy_timestamp(&entry);
    // uuid 取值（fork 锚点，必须全局唯一且与后端 fork_truncate_lines 的匹配链一致）：
    // - function_call 行：优先 callId。一条 assistant message 发 N 个工具调用时，N 行共享
    //   message 级 `id`（真实样本 8/8 会话、691 行中 263 行重复），唯一锚是 callId。
    // - 其余行（message/reasoning/function_call_result）：优先 id。function_call_result 的
    //   callId 与对应 function_call 相同（配对键），若用它会引入新的重复；其 id 已实测唯一。
    let uuid = if entry_type == "function_call" {
        entry
            .get("callId")
            .or_else(|| entry.get("id"))
            .and_then(|v| v.as_str())
            .map(str::to_string)
    } else {
        entry
            .get("id")
            .or_else(|| entry.get("callId"))
            .and_then(|v| v.as_str())
            .map(str::to_string)
    };

    match entry_type {
        "message" => {
            let role = entry.get("role").and_then(|v| v.as_str())?;
            match role {
                "user" => {
                    let mut parts = Vec::new();
                    if let Some(content) = entry.get("content").and_then(|v| v.as_array()) {
                        for block in content {
                            match block.get("type").and_then(|v| v.as_str()).unwrap_or("") {
                                "input_text" => {
                                    if let Some(text) =
                                        block.get("text").and_then(|v| v.as_str())
                                    {
                                        if !text.trim().is_empty() {
                                            parts.push(ContentPart::Text {
                                                text: text.to_string(),
                                            });
                                        }
                                    }
                                }
                                "image_blob_ref" => {
                                    if let Some(path) =
                                        block.get("blob_path").and_then(|v| v.as_str())
                                    {
                                        if !path.is_empty() {
                                            parts.push(ContentPart::ImageRef {
                                                path: path.to_string(),
                                            });
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    // 压缩检查点（上下文摘要）不展示
                    let is_compaction = parts.iter().any(|p| {
                        matches!(p, ContentPart::Text { text } if text.trim_start().starts_with("<cb_summary"))
                    });
                    if parts.is_empty() || is_compaction {
                        return None;
                    }
                    Some(ChatMessage {
                        role: "user".to_string(),
                        timestamp,
                        model: None,
                        token_usage: None,
                        content_parts: parts,
                        is_meta: false,
                        uuid,
                    })
                }
                "assistant" => {
                    let mut parts = Vec::new();
                    if let Some(content) = entry.get("content").and_then(|v| v.as_array()) {
                        for block in content {
                            if block.get("type").and_then(|v| v.as_str()) == Some("output_text")
                            {
                                if let Some(text) = block.get("text").and_then(|v| v.as_str())
                                {
                                    if !text.trim().is_empty() {
                                        parts.push(ContentPart::Text {
                                            text: text.to_string(),
                                        });
                                    }
                                }
                            }
                        }
                    }
                    if parts.is_empty() {
                        return None;
                    }
                    let model = entry
                        .pointer("/providerData/model")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());
                    let token_usage =
                        entry.pointer("/message/usage").map(|u| session::TokenUsage {
                            input_tokens: u
                                .get("input_tokens")
                                .and_then(|v| v.as_u64())
                                .unwrap_or(0),
                            output_tokens: u
                                .get("output_tokens")
                                .and_then(|v| v.as_u64())
                                .unwrap_or(0),
                            cache_creation_input_tokens: u
                                .get("cache_creation_input_tokens")
                                .and_then(|v| v.as_u64())
                                .unwrap_or(0),
                            cache_read_input_tokens: u
                                .get("cache_read_input_tokens")
                                .and_then(|v| v.as_u64())
                                .unwrap_or(0),
                        });
                    Some(ChatMessage {
                        role: "assistant".to_string(),
                        timestamp,
                        model,
                        token_usage,
                        content_parts: parts,
                        is_meta: false,
                        uuid,
                    })
                }
                _ => None,
            }
        }
        "function_call" => {
            let tool_name = entry
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("Tool")
                .to_string();
            // arguments 是 JSON 字符串，复用 Codex 的解析（支持 object/array/JSON 字符串）
            let input_val = codex_tool_input_value(&entry);
            let summary = session::tool_use_summary(&tool_name, &input_val);
            let input = serde_json::to_string_pretty(&input_val).unwrap_or_default();
            let tool_use_id = entry
                .get("callId")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            Some(ChatMessage {
                role: "assistant".to_string(),
                timestamp,
                model: None,
                token_usage: None,
                content_parts: vec![ContentPart::ToolUse {
                    summary,
                    tool_name,
                    input,
                    tool_use_id,
                }],
                is_meta: false,
                uuid,
            })
        }
        "function_call_result" => {
            let tool_name = entry
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("Tool")
                .to_string();
            let mut content = entry
                .get("output")
                .map(codex_value_to_text)
                .unwrap_or_default()
                .trim()
                .to_string();
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
            let is_error = codex_tool_result_is_error(&entry, &content);
            let summary = if is_error {
                format!("[{} error]", tool_name)
            } else {
                format!("[{} result]", tool_name)
            };
            Some(ChatMessage {
                role: "assistant".to_string(),
                timestamp,
                model: None,
                token_usage: None,
                content_parts: vec![ContentPart::tool_result(summary, content, is_error, truncated)],
                is_meta: false,
                uuid,
            })
        }
        "reasoning" => {
            let mut parts = Vec::new();
            if let Some(raw) = entry.get("rawContent").and_then(|v| v.as_array()) {
                for block in raw {
                    if block.get("type").and_then(|v| v.as_str()) == Some("reasoning_text") {
                        if let Some(text) = block.get("text").and_then(|v| v.as_str()) {
                            if !text.trim().is_empty() {
                                parts.push(ContentPart::Thinking {
                                    thinking: text.to_string(),
                                });
                            }
                        }
                    }
                }
            }
            if parts.is_empty() {
                return None;
            }
            Some(ChatMessage {
                role: "assistant".to_string(),
                timestamp,
                model: None,
                token_usage: None,
                content_parts: parts,
                is_meta: false,
                uuid,
            })
        }
        _ => None,
    }
}

/// WorkBuddy 行前缀快筛：只放行可产出消息的行类型（两种空格变体）
pub(crate) fn workbuddy_line_candidate(prefix: &str) -> bool {
    for t in ["message", "function_call", "function_call_result", "reasoning"] {
        if prefix.contains(&format!(r#""type":"{}""#, t))
            || prefix.contains(&format!(r#""type": "{}""#, t))
        {
            return true;
        }
    }
    false
}

pub(crate) fn parse_workbuddy_session_file(file_path: &str) -> AppResult<SessionLoadResult> {
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
        if !workbuddy_line_candidate(&line[..check_len]) {
            continue;
        }
        if let Some(message) = parse_workbuddy_line(line) {
            messages.push(message);
        }
    }

    Ok(SessionLoadResult {
        messages,
        offset: file_size,
        subagent_map: std::collections::HashMap::new(),
    })
}

pub(crate) fn parse_workbuddy_session_incremental(
    file_path: &str,
    offset: u64,
) -> AppResult<SessionLoadResult> {
    let mut file = File::open(file_path)?;
    let file_size = file.metadata()?.len();

    if file_size <= offset {
        return Ok(SessionLoadResult {
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
        if !workbuddy_line_candidate(&line[..check_len]) {
            continue;
        }
        if let Some(message) = parse_workbuddy_line(line) {
            messages.push(message);
        }
    }

    Ok(SessionLoadResult {
        messages,
        offset: file_size,
        subagent_map: std::collections::HashMap::new(),
    })
}

pub(crate) fn parse_workbuddy_session_file_streaming<F>(
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
        if !workbuddy_line_candidate(&line[..check_len]) {
            continue;
        }
        if let Some(message) = parse_workbuddy_line(line) {
            emitter.push(message);
            if !emitter.maybe_flush(&mut on_batch) {
                return Ok((file_size, std::collections::HashMap::new()));
            }
        }
    }

    emitter.flush_remaining(&mut on_batch);

    Ok((file_size, std::collections::HashMap::new()))
}

pub(crate) fn read_workbuddy_header_field(file_path: &str, field: &str) -> Option<String> {
    let file = File::open(file_path).ok()?;
    let reader = BufReader::new(file);

    for line in reader.lines().take(5) {
        let line = match line {
            Ok(line) => line,
            Err(_) => continue,
        };
        let Ok(entry) = serde_json::from_str::<Value>(line.trim()) else {
            continue;
        };
        if let Some(value) = entry.get(field).and_then(|v| v.as_str()) {
            return Some(value.to_string());
        }
    }
    None
}

pub(crate) fn read_workbuddy_first_user_message(file_path: &str) -> Option<String> {
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
        let Some((role, text)) = extract_workbuddy_role_text(&entry) else {
            continue;
        };
        if role != "user" || text.is_empty() {
            continue;
        }
        // 报告预览惯例：extractor 已清洗（user_query/系统标签），再截断 30 字
        return truncate_preview_text(&text, 30);
    }
    None
}

pub(crate) fn extract_workbuddy_role_text(entry: &Value) -> Option<(String, String)> {
    let entry_type = entry.get("type").and_then(|v| v.as_str())?;
    match entry_type {
        "message" => {
            let role = entry.get("role").and_then(|v| v.as_str())?;
            let block_type = match role {
                "user" => "input_text",
                "assistant" => "output_text",
                _ => return None,
            };
            let text: String = entry
                .get("content")
                .and_then(|v| v.as_array())?
                .iter()
                .filter(|b| b.get("type").and_then(|v| v.as_str()) == Some(block_type))
                .filter_map(|b| b.get("text").and_then(|v| v.as_str()))
                .collect::<Vec<_>>()
                .join("\n");
            if text.trim().is_empty() {
                return None;
            }
            if role == "user" {
                // 只索引/展示真实提问，不索引包裹的系统上下文
                let cleaned = workbuddy_clean_user_text(&text)?;
                Some(("user".to_string(), cleaned))
            } else {
                Some(("assistant".to_string(), text))
            }
        }
        // 与 Gemini 惯例一致：工具名可被搜索
        "function_call" => entry
            .get("name")
            .and_then(|v| v.as_str())
            .map(|name| ("assistant".to_string(), name.to_string())),
        _ => None,
    }
}

/// read_last_timestamp_from_file 的 WorkBuddy 变体：接受毫秒数字时间戳
pub(crate) fn read_workbuddy_last_timestamp(file: &mut File, file_size: u64) -> Option<String> {
    if file_size == 0 {
        return None;
    }

    let chunk_size: u64 = 65536;
    let max_search: u64 = 2 * 1024 * 1024;
    let mut offset = file_size;
    let mut overlap = String::new();

    while offset > 0 && file_size.saturating_sub(offset) < max_search {
        let read_size = std::cmp::min(offset, chunk_size);
        offset -= read_size;

        if file.seek(SeekFrom::Start(offset)).is_err() {
            break;
        }

        let mut buf = vec![0u8; read_size as usize];
        let mut total_read = 0;
        while total_read < read_size as usize {
            match file.read(&mut buf[total_read..]) {
                Ok(0) => break,
                Ok(n) => total_read += n,
                Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => return None,
            }
        }

        let chunk_text = String::from_utf8_lossy(&buf[..total_read]);
        let combined = format!("{}{}", chunk_text, overlap);

        let mut lines: Vec<&str> = combined.lines().collect();
        if offset > 0 && !combined.starts_with('\n') && !lines.is_empty() {
            overlap = lines.remove(0).to_string();
        } else {
            overlap.clear();
        }

        for line in lines.into_iter().rev() {
            let trimmed = line.trim();
            if trimmed.is_empty() || !trimmed.contains("\"timestamp\"") {
                continue;
            }
            let entry: Value = match serde_json::from_str(trimmed) {
                Ok(v) => v,
                Err(_) => continue,
            };
            let ts = workbuddy_timestamp(&entry);
            if !ts.is_empty() {
                return Some(ts);
            }
        }
    }
    None
}

pub(crate) fn scan_workbuddy_metadata_only(file_path: &Path) -> Option<SessionListMetadata> {
    let mut file = File::open(file_path).ok()?;
    let file_size = file.seek(SeekFrom::End(0)).ok()?;
    file.seek(SeekFrom::Start(0)).ok()?;

    let reader = BufReader::new(&file);
    let mut has_chat_messages = false;
    let mut session_id = None;
    let mut project_path = None;
    let mut title = None;
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
        if project_path.is_none() {
            if let Some(cwd) = entry.get("cwd").and_then(|v| v.as_str()) {
                if !cwd.trim().is_empty() {
                    project_path = Some(cwd.to_string());
                }
            }
        }
        if first_timestamp.is_none() {
            let ts = workbuddy_timestamp(&entry);
            if !ts.is_empty() {
                first_timestamp = Some(ts);
            }
        }
        if title.is_none() {
            if entry.get("type").and_then(|v| v.as_str()) == Some("ai-title") {
                title = crate::title_resolver::extract_typed_title(&entry, "aiTitle", "ai_title");
            }
        }

        if !has_chat_messages {
            if let Some((role, text)) = extract_workbuddy_role_text(&entry) {
                has_chat_messages = true;
                if role == "user" {
                    // extractor 已做 user_query/系统标签清洗
                    first_user_message = Some(text);
                }
            }
        }
    }

    // ai-title 可能在会话结束时才追加到文件尾部，头部窗口未命中时做尾部扫描
    // （read_typed_titles_from_tail 同样匹配 "ai-title"/aiTitle 行结构，直接复用）
    if title.is_none() {
        let (_, ai_title) = read_typed_titles_from_tail(&mut file, file_size);
        title = ai_title;
    }

    if !has_chat_messages && title.is_none() {
        return None;
    }

    let last_timestamp = read_workbuddy_last_timestamp(&mut file, file_size);
    let fallback_session_id = file_path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("")
        .to_string();

    Some(SessionListMetadata {
        session_id: session_id.unwrap_or(fallback_session_id),
        project_path,
        title,
        first_user_message,
        first_timestamp: first_timestamp.clone(),
        last_timestamp: last_timestamp.or(first_timestamp),
        git_branch: String::new(),
        file_size,
    })
}

/// 单次读取 `~/.workbuddy/workbuddy.db`（尊重 CLI 路径覆盖配置），构建
/// SessionID → custom_title 的内存 Map，供 WorkBuddy 用户重命名 O(1) 查询。
/// WorkBuddy 把 UI 自定义会话名写在本地 SQLite（sessions.custom_title）而非
/// JSONL，JSONL 里只有 AI 生成的 ai-title；因此由调用侧在列表/搜索组装时覆盖。
/// DB 缺失、被占用或查询失败时返回空 Map（标题解析是增强而非硬依赖）。
pub(crate) fn load_workbuddy_custom_titles() -> std::collections::HashMap<String, String> {
    match crate::cli::data_dir(crate::cli::CliKind::WorkBuddy) {
        Ok(dir) => load_workbuddy_custom_titles_from(&dir.join("workbuddy.db")),
        Err(_) => std::collections::HashMap::new(),
    }
}

fn load_workbuddy_custom_titles_from(db_path: &Path) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    if !db_path.exists() {
        return map;
    }
    // 只读打开：WorkBuddy 运行时持 WAL 写连接，只读并发安全
    let Ok(conn) = rusqlite::Connection::open_with_flags(
        db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    ) else {
        return map;
    };
    let _ = conn.busy_timeout(std::time::Duration::from_millis(500));
    let Ok(mut stmt) = conn.prepare(
        "SELECT id, custom_title FROM sessions \
         WHERE custom_title IS NOT NULL AND TRIM(custom_title) != ''",
    ) else {
        return map;
    };
    if let Ok(rows) = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    }) {
        for row in rows.flatten() {
            let title = row.1.trim().to_string();
            if !row.0.is_empty() && !title.is_empty() {
                map.insert(row.0, title);
            }
        }
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn workbuddy_timestamp_converts_ms_to_rfc3339() {
        let entry = json!({"timestamp": 1786514191337i64});
        let ts = workbuddy_timestamp(&entry);
        assert!(ts.starts_with("2026-"), "unexpected ts: {}", ts);
        assert!(ts.contains('T'));
        // 兼容字符串格式漂移
        let entry2 = json!({"timestamp": "2026-08-12T01:23:45Z"});
        assert_eq!(workbuddy_timestamp(&entry2), "2026-08-12T01:23:45Z");
        let empty = json!({});
        assert_eq!(workbuddy_timestamp(&empty), "");
    }

    #[test]
    fn workbuddy_clean_user_text_takes_last_user_query() {
        let raw = "<system-reminder data-role=\"user-context\">ctx</system-reminder><user_query>旧问题</user_query>更多<user_query>新问题</user_query>";
        assert_eq!(workbuddy_clean_user_text(raw).as_deref(), Some("新问题"));
        // 无 user_query 时剥离系统标签后返回正文
        let plain = workbuddy_clean_user_text("<system-reminder data-role=\"user-context\">ctx</system-reminder>你好");
        assert_eq!(plain.as_deref(), Some("你好"));
        // cb_summary 压缩行丢弃
        assert!(workbuddy_clean_user_text("<cb_summary>摘要</cb_summary>").is_none());
    }

    #[test]
    fn parse_workbuddy_line_user_message_with_image() {
        let line = r#"{"id":"u1","timestamp":1786514191337,"type":"message","role":"user","content":[{"type":"input_text","text":"<user_query>hi</user_query>"},{"type":"image_blob_ref","blob_path":"/Users/x/.workbuddy/blobs/a.png"}],"sessionId":"s1","cwd":"/p"}"#;
        let msg = parse_workbuddy_line(line).expect("user message");
        assert_eq!(msg.role, "user");
        assert_eq!(msg.uuid.as_deref(), Some("u1"));
        assert_eq!(msg.content_parts.len(), 2);
        assert!(matches!(&msg.content_parts[0], ContentPart::Text { text } if text.contains("<user_query>")));
        assert!(matches!(&msg.content_parts[1], ContentPart::ImageRef { path } if path == "/Users/x/.workbuddy/blobs/a.png"));

        // cb_summary 用户行整体跳过
        let cb = r#"{"type":"message","role":"user","content":[{"type":"input_text","text":"<cb_summary>压缩</cb_summary>"}]}"#;
        assert!(parse_workbuddy_line(cb).is_none());
    }

    #[test]
    fn parse_workbuddy_line_assistant_with_usage_and_model() {
        let line = r#"{"type":"message","role":"assistant","timestamp":1786514217655,"content":[{"type":"output_text","text":"你好"}],"providerData":{"model":"hy3"},"message":{"usage":{"input_tokens":100,"output_tokens":20,"cache_read_input_tokens":5}}}"#;
        let msg = parse_workbuddy_line(line).expect("assistant message");
        assert_eq!(msg.role, "assistant");
        assert_eq!(msg.model.as_deref(), Some("hy3"));
        let usage = msg.token_usage.expect("usage");
        assert_eq!(usage.input_tokens, 100);
        assert_eq!(usage.output_tokens, 20);
        assert_eq!(usage.cache_read_input_tokens, 5);
        assert!(matches!(&msg.content_parts[0], ContentPart::Text { text } if text == "你好"));
    }

    #[test]
    fn parse_workbuddy_line_function_call_and_result() {
        let call = r#"{"type":"function_call","name":"Read","arguments":"{\"file_path\": \"/tmp/a.md\"}","callId":"call-1","timestamp":1786514192000}"#;
        let msg = parse_workbuddy_line(call).expect("function_call");
        assert!(matches!(
            &msg.content_parts[0],
            ContentPart::ToolUse { tool_name, tool_use_id, .. }
                if tool_name == "Read" && tool_use_id.as_deref() == Some("call-1")
        ));
        let result = r#"{"type":"function_call_result","name":"Read","callId":"call-1","status":"completed","output":{"type":"text","text":"文件内容"},"timestamp":1786514193000}"#;
        let msg = parse_workbuddy_line(result).expect("function_call_result");
        assert!(matches!(
            &msg.content_parts[0],
            ContentPart::ToolResult { content, is_error, .. } if content == "文件内容" && !is_error
        ));

        let err = r#"{"type":"function_call_result","name":"Bash","status":"error","output":{"type":"text","text":"boom"},"timestamp":1}"#;
        let msg = parse_workbuddy_line(err).expect("error result");
        assert!(matches!(
            &msg.content_parts[0],
            ContentPart::ToolResult { is_error, .. } if *is_error
        ));
    }

    #[test]
    fn workbuddy_function_call_uuid_uses_call_id_not_shared_message_id() {
        // 一条 assistant message 并发两个工具调用：两行 function_call 共享同一个 message 级
        // `id`（真实样本 8/8 会话存在），唯一锚是各自的 callId。
        let call1 = r#"{"id":"msg-1","type":"function_call","name":"Read","arguments":"{}","callId":"call-A","timestamp":1}"#;
        let call2 = r#"{"id":"msg-1","type":"function_call","name":"Bash","arguments":"{}","callId":"call-B","timestamp":2}"#;
        let m1 = parse_workbuddy_line(call1).expect("call1");
        let m2 = parse_workbuddy_line(call2).expect("call2");
        assert_eq!(m1.uuid.as_deref(), Some("call-A"));
        assert_eq!(m2.uuid.as_deref(), Some("call-B"));
        assert_ne!(m1.uuid, m2.uuid, "同一 message 的多个 function_call uuid 必须不同");

        // function_call_result 的 callId 与配对的 function_call 相同，不能用作 uuid；
        // 其行级 id 唯一，保持 id 优先。
        let result = r#"{"id":"res-1","type":"function_call_result","name":"Read","callId":"call-A","status":"completed","output":{"type":"text","text":"ok"},"timestamp":3}"#;
        let m3 = parse_workbuddy_line(result).expect("result");
        assert_eq!(m3.uuid.as_deref(), Some("res-1"));
        assert_ne!(m3.uuid, m1.uuid, "result 与 call 不能共享 uuid");

        // message 行（user/assistant）没有 callId，取 id —— 行为不变。
        let user = r#"{"id":"u1","type":"message","role":"user","content":[{"type":"input_text","text":"hi"}],"timestamp":4}"#;
        let m4 = parse_workbuddy_line(user).expect("user");
        assert_eq!(m4.uuid.as_deref(), Some("u1"));
    }

    #[test]
    fn parse_workbuddy_line_reasoning_and_skipped_types() {
        let reasoning = r#"{"type":"reasoning","rawContent":[{"type":"reasoning_text","text":"思考一下"}],"timestamp":1}"#;
        let msg = parse_workbuddy_line(reasoning).expect("reasoning");
        assert!(matches!(&msg.content_parts[0], ContentPart::Thinking { thinking } if thinking == "思考一下"));

        assert!(parse_workbuddy_line(r#"{"type":"ai-title","aiTitle":"t","timestamp":1}"#).is_none());
        assert!(parse_workbuddy_line(r#"{"type":"file-history-snapshot","timestamp":1}"#).is_none());
    }

    #[test]
    fn extract_workbuddy_role_text_cleans_user_and_keeps_tool_names() {
        let user = json!({"type":"message","role":"user","content":[{"type":"input_text","text":"<system-reminder data-role=\"user-context\">ctx</system-reminder><user_query>真实问题</user_query>"}]});
        let (role, text) = extract_workbuddy_role_text(&user).expect("user text");
        assert_eq!(role, "user");
        assert_eq!(text, "真实问题");

        let asst = json!({"type":"message","role":"assistant","content":[{"type":"output_text","text":"回答"}]});
        assert_eq!(extract_workbuddy_role_text(&asst).unwrap().1, "回答");

        let tool = json!({"type":"function_call","name":"Bash"});
        assert_eq!(extract_workbuddy_role_text(&tool).unwrap().1, "Bash");

        let cb = json!({"type":"message","role":"user","content":[{"type":"input_text","text":"<cb_summary>x</cb_summary>"}]});
        assert!(extract_workbuddy_role_text(&cb).is_none());
    }

    #[test]
    fn load_workbuddy_custom_titles_reads_sqlite_custom_title_column() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let db_path = std::env::temp_dir().join(format!("seshbuddy-wb-titles-{unique}.db"));

        {
            let conn = rusqlite::Connection::open(&db_path).unwrap();
            conn.execute(
                "CREATE TABLE sessions (id TEXT PRIMARY KEY, custom_title TEXT)",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO sessions (id, custom_title) VALUES ('sess-a', '日常聊天')",
                [],
            )
            .unwrap();
            conn.execute("INSERT INTO sessions (id, custom_title) VALUES ('sess-b', NULL)", [])
                .unwrap();
            conn.execute("INSERT INTO sessions (id, custom_title) VALUES ('sess-c', '  ')", [])
                .unwrap();
        }

        let map = load_workbuddy_custom_titles_from(&db_path);
        let _ = fs::remove_file(&db_path);

        assert_eq!(map.get("sess-a").map(String::as_str), Some("日常聊天"));
        assert!(!map.contains_key("sess-b"));
        assert!(!map.contains_key("sess-c"));
    }

    #[test]
    fn load_workbuddy_custom_titles_missing_db_returns_empty() {
        let map = load_workbuddy_custom_titles_from(Path::new(
            "/nonexistent/path/workbuddy.db",
        ));
        assert!(map.is_empty());
    }

    #[test]
    fn scan_workbuddy_metadata_reads_cwd_title_and_ms_timestamps() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("seshbuddy-wb-meta-{unique}.jsonl"));
        let content = concat!(
            "{\"id\":\"m1\",\"timestamp\":1786514191337,\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"<user_query>第一个问题</user_query>\"}],\"sessionId\":\"sess-abc\",\"cwd\":\"/Users/x/proj\"}\n",
            "{\"timestamp\":1786514192000,\"type\":\"function_call\",\"name\":\"Read\",\"arguments\":\"{}\",\"callId\":\"c1\",\"sessionId\":\"sess-abc\",\"cwd\":\"/Users/x/proj\"}\n",
            "{\"timestamp\":1786514358629,\"type\":\"ai-title\",\"aiTitle\":\"尾部标题\",\"sessionId\":\"sess-abc\",\"cwd\":\"/Users/x/proj\"}\n"
        );
        fs::write(&path, content).unwrap();

        let metadata = scan_workbuddy_metadata_only(&path).expect("metadata");
        let _ = fs::remove_file(&path);

        assert_eq!(metadata.session_id, "sess-abc");
        assert_eq!(metadata.project_path.as_deref(), Some("/Users/x/proj"));
        assert_eq!(metadata.title.as_deref(), Some("尾部标题"));
        assert_eq!(metadata.first_user_message.as_deref(), Some("第一个问题"));
        assert!(metadata.first_timestamp.as_deref().unwrap().contains('T'));
        assert!(metadata.last_timestamp.as_deref().unwrap().contains('T'));
    }
}
