use std::path::Path;
use rusqlite::{params, Connection, OpenFlags};
use serde_json::Value;

use crate::error::AppResult;
use crate::parser::claude_entry::TOOL_RESULT_MAX_LEN;
use crate::parser::shared::SearchDocument;
use crate::session::{ChatMessage, ContentPart, SessionListMetadata};

/// 只读打开 Goose 数据库。
pub(crate) fn open_goose_db(path: &Path) -> AppResult<Connection> {
    Ok(Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?)
}

/// 首字母大写归一化工具名（如 text_editor -> Text_editor，bash -> Bash）。
pub(crate) fn capitalize_first(name: &str) -> String {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// 解析 SQLite 时间戳。
/// Goose 中的时间戳可能为整数（秒或毫秒）或 RFC3339 字符串。
pub(crate) fn parse_timestamp(value: &Value) -> Option<String> {
    if let Some(num) = value.as_i64() {
        return if num <= 10_000_000_000 {
            chrono::DateTime::from_timestamp(num, 0).map(|dt| dt.to_rfc3339())
        } else {
            chrono::DateTime::from_timestamp_millis(num).map(|dt| dt.to_rfc3339())
        };
    }
    if let Some(s) = value.as_str() {
        if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
            return Some(dt.to_rfc3339());
        }
        if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S") {
            return Some(chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(naive, chrono::Utc).to_rfc3339());
        }
        return Some(s.to_string());
    }
    None
}

/// 从 rusqlite::Row 尝试解析时间戳字段（支持 INTEGER 或 TEXT）。
pub(crate) fn row_timestamp_to_rfc3339(row: &rusqlite::Row, idx: usize) -> Option<String> {
    if let Ok(num) = row.get::<_, i64>(idx) {
        return if num <= 10_000_000_000 {
            chrono::DateTime::from_timestamp(num, 0).map(|dt| dt.to_rfc3339())
        } else {
            chrono::DateTime::from_timestamp_millis(num).map(|dt| dt.to_rfc3339())
        };
    }
    if let Ok(s) = row.get::<_, String>(idx) {
        return parse_timestamp(&Value::String(s));
    }
    None
}

/// 非空文本压入 parts。
fn push_nonempty_text(text: Option<&str>, out: &mut Vec<ContentPart>) {
    if let Some(text) = text {
        if !text.trim().is_empty() {
            out.push(ContentPart::Text { text: text.to_string() });
        }
    }
}

/// 处理 tool_request 节点。
fn push_tool_request(data: &Value, out: &mut Vec<ContentPart>) {
    let call_id = data.get("id").and_then(Value::as_str).map(str::to_string);
    let tool_call = data.get("tool_call");

    let tool_name = tool_call
        .and_then(|tc| tc.get("name"))
        .or_else(|| data.get("name"))
        .and_then(Value::as_str)
        .unwrap_or("tool");
    let tool_name = capitalize_first(tool_name);

    let arguments = tool_call
        .and_then(|tc| tc.get("arguments"))
        .or_else(|| data.get("arguments"))
        .cloned()
        .unwrap_or(Value::Null);

    let summary = crate::session::tool_use_summary(&tool_name, &arguments);
    let input = serde_json::to_string_pretty(&arguments).unwrap_or_default();

    out.push(ContentPart::ToolUse {
        summary,
        tool_name,
        input,
        tool_use_id: call_id,
    });
}

/// 处理 tool_response 节点。
fn push_tool_response(data: &Value, out: &mut Vec<ContentPart>) {
    let call_id = data.get("id").and_then(Value::as_str).map(str::to_string);
    let tool_result = data.get("tool_result").or_else(|| data.get("content"));

    let mut is_error = false;
    let mut text_output = String::new();

    if let Some(result) = tool_result {
        if let Some(err_val) = result.get("Err") {
            is_error = true;
            text_output = err_val.as_str().map(str::to_string).unwrap_or_else(|| err_val.to_string());
        } else if let Some(ok_val) = result.get("Ok") {
            if let Some(arr) = ok_val.as_array() {
                let parts: Vec<&str> = arr.iter().filter_map(|item| {
                    item.get("text").and_then(Value::as_str).or_else(|| item.as_str())
                }).collect();
                text_output = parts.join("\n");
            } else if let Some(s) = ok_val.as_str() {
                text_output = s.to_string();
            } else {
                text_output = ok_val.to_string();
            }
        } else if let Some(s) = result.as_str() {
            text_output = s.to_string();
        } else if let Some(arr) = result.as_array() {
            let parts: Vec<&str> = arr.iter().filter_map(|item| {
                item.get("text").and_then(Value::as_str).or_else(|| item.as_str())
            }).collect();
            text_output = parts.join("\n");
        } else {
            if let Some(err) = result.get("error").and_then(Value::as_str) {
                is_error = true;
                text_output = err.to_string();
            } else if let Some(status) = result.get("status").and_then(Value::as_str) {
                if status == "error" {
                    is_error = true;
                }
                if let Some(out) = result.get("output").and_then(Value::as_str) {
                    text_output = out.to_string();
                }
            } else {
                text_output = result.to_string();
            }
        }
    }

    let mut content = text_output.trim().to_string();
    let mut truncated = false;
    if content.len() > TOOL_RESULT_MAX_LEN {
        let mut cut = TOOL_RESULT_MAX_LEN;
        while cut > 0 && !content.is_char_boundary(cut) {
            cut -= 1;
        }
        content.truncate(cut);
        truncated = true;
    }

    let summary = if is_error {
        "[Tool Error]".to_string()
    } else {
        "[Tool Result]".to_string()
    };

    out.push(ContentPart::ToolResult {
        summary,
        content,
        is_error,
        tool_use_id: call_id,
        full_len: 0,
        truncated_preview: false,
        truncated,
        source_offset: 0,
    });
}

/// 解析单个 content block（支持内部或外部 tag）。
pub(crate) fn push_content_block(block: &Value, out: &mut Vec<ContentPart>) {
    if let Some(t) = block.get("type").and_then(Value::as_str) {
        match t {
            "text" => {
                push_nonempty_text(block.get("text").and_then(Value::as_str), out);
            }
            "thinking" => {
                let thinking = block
                    .get("thinking")
                    .or_else(|| block.get("text"))
                    .and_then(Value::as_str);
                if let Some(text) = thinking {
                    if !text.trim().is_empty() {
                        out.push(ContentPart::Thinking { thinking: text.to_string() });
                    }
                }
            }
            "tool_request" => {
                push_tool_request(block, out);
            }
            "tool_response" => {
                push_tool_response(block, out);
            }
            other => {
                out.push(ContentPart::Text { text: format!("[{other}]") });
            }
        }
        return;
    }

    // 外部 tag：例如 { "Text": { "text": "..." } } 或 { "ToolRequest": ... }
    if let Some(obj) = block.as_object() {
        if let Some(text_obj) = obj.get("Text") {
            let text = text_obj.get("text").and_then(Value::as_str).or_else(|| text_obj.as_str());
            push_nonempty_text(text, out);
            return;
        }
        if let Some(thinking_obj) = obj.get("Thinking") {
            let text = thinking_obj.get("thinking").or_else(|| thinking_obj.get("text")).and_then(Value::as_str).or_else(|| thinking_obj.as_str());
            if let Some(text) = text {
                if !text.trim().is_empty() {
                    out.push(ContentPart::Thinking { thinking: text.to_string() });
                }
            }
            return;
        }
        if let Some(req) = obj.get("ToolRequest") {
            push_tool_request(req, out);
            return;
        }
        if let Some(resp) = obj.get("ToolResponse") {
            push_tool_response(resp, out);
            return;
        }
    }

    // 顶层降级
    if let Some(text) = block.get("text").and_then(Value::as_str) {
        push_nonempty_text(Some(text), out);
    } else if block.get("tool_call").is_some() {
        push_tool_request(block, out);
    }
}

/// 从 content_json 字段解析所有 ContentPart。
pub(crate) fn parse_content_json(raw: &str) -> Vec<ContentPart> {
    let mut parts = Vec::new();
    let Ok(val) = serde_json::from_str::<Value>(raw) else {
        return parts;
    };
    if let Some(arr) = val.as_array() {
        for item in arr {
            push_content_block(item, &mut parts);
        }
    } else if val.is_object() {
        push_content_block(&val, &mut parts);
    }
    parts
}

/// 读取指定会话的所有消息。
pub(crate) fn load_session_messages(conn: &Connection, session_id: &str) -> AppResult<Vec<ChatMessage>> {
    let mut stmt = conn.prepare(
        "SELECT id, role, content_json, created_timestamp, metadata_json \
         FROM messages WHERE session_id = ?1 ORDER BY created_timestamp, id",
    )?;

    let rows = stmt.query_map(params![session_id], |row| {
        let id = row.get::<_, i64>(0)?;
        let role = row.get::<_, String>(1)?;
        let content_json = row.get::<_, String>(2)?;
        let created_timestamp = row_timestamp_to_rfc3339(row, 3);
        let metadata_json = row.get::<_, Option<String>>(4)?;
        Ok((id, role, content_json, created_timestamp, metadata_json))
    })?
    .collect::<Result<Vec<_>, _>>()?;

    let mut messages = Vec::with_capacity(rows.len());
    for (_id, role, content_json, timestamp, metadata_json) in rows {
        let parts = parse_content_json(&content_json);
        let model = metadata_json.as_deref().and_then(|m| {
            let v: Value = serde_json::from_str(m).ok()?;
            v.get("model")
                .or_else(|| v.get("model_id"))
                .or_else(|| v.pointer("/model/id"))
                .and_then(Value::as_str)
                .map(str::to_string)
        });

        messages.push(ChatMessage {
            role,
            timestamp: timestamp.unwrap_or_default(),
            model,
            token_usage: None,
            content_parts: parts,
            is_meta: false,
            uuid: None,
        });
    }

    Ok(messages)
}

/// 从已解析消息构建检索文档。
pub(crate) fn search_docs_from_messages(messages: &[ChatMessage]) -> Vec<SearchDocument> {
    let mut docs = Vec::new();
    for (message_index, message) in messages.iter().enumerate() {
        let texts: Vec<&str> = message
            .content_parts
            .iter()
            .filter_map(|part| match part {
                ContentPart::Text { text } => Some(text.as_str()),
                ContentPart::Thinking { thinking } => Some(thinking.as_str()),
                _ => None,
            })
            .collect();
        let search_text = texts.join("\n");
        if search_text.trim().is_empty() {
            continue;
        }
        docs.push(SearchDocument {
            message_index,
            role: message.role.clone(),
            search_text,
        });
    }
    docs
}

/// 组装会话列表元数据。
pub(crate) fn session_list_metadata(
    session_id: String,
    title: Option<String>,
    project_path: Option<String>,
    first_timestamp: Option<String>,
    last_timestamp: Option<String>,
) -> SessionListMetadata {
    SessionListMetadata {
        session_id,
        project_path,
        title,
        first_user_message: None,
        first_timestamp,
        last_timestamp,
        git_branch: String::new(),
        file_size: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_test_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE sessions (
                id TEXT PRIMARY KEY,
                name TEXT,
                description TEXT,
                user_set_name BOOLEAN,
                session_type TEXT,
                working_dir TEXT,
                created_at TIMESTAMP,
                updated_at TIMESTAMP,
                total_tokens INTEGER,
                input_tokens INTEGER,
                output_tokens INTEGER,
                parent_session_id TEXT
            );
            CREATE TABLE messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                message_id TEXT,
                session_id TEXT REFERENCES sessions(id),
                role TEXT,
                content_json TEXT,
                created_timestamp INTEGER,
                metadata_json TEXT
            );",
        )
        .unwrap();
        conn
    }

    #[test]
    fn parse_internal_tagged_content_json() {
        let raw = r#"[
            {"type": "text", "text": "Hello Goose"},
            {"type": "thinking", "thinking": "Pondering the task..."},
            {"type": "tool_request", "id": "call_1", "tool_call": {"name": "developer__shell", "arguments": {"command": "cargo test"}}},
            {"type": "tool_response", "id": "call_1", "tool_result": {"Ok": [{"text": "test passed"}]}}
        ]"#;

        let parts = parse_content_json(raw);
        assert_eq!(parts.len(), 4);

        match &parts[0] {
            ContentPart::Text { text } => assert_eq!(text, "Hello Goose"),
            other => panic!("expected Text, got {other:?}"),
        }
        match &parts[1] {
            ContentPart::Thinking { thinking } => assert_eq!(thinking, "Pondering the task..."),
            other => panic!("expected Thinking, got {other:?}"),
        }
        match &parts[2] {
            ContentPart::ToolUse { tool_name, summary, tool_use_id, .. } => {
                assert_eq!(tool_name, "Developer__shell");
                assert_eq!(tool_use_id.as_deref(), Some("call_1"));
                assert!(!summary.is_empty());
            }
            other => panic!("expected ToolUse, got {other:?}"),
        }
        match &parts[3] {
            ContentPart::ToolResult { summary, content, is_error, tool_use_id, .. } => {
                assert_eq!(summary, "[Tool Result]");
                assert_eq!(content, "test passed");
                assert!(!*is_error);
                assert_eq!(tool_use_id.as_deref(), Some("call_1"));
            }
            other => panic!("expected ToolResult, got {other:?}"),
        }
    }

    #[test]
    fn parse_external_tagged_content_json_with_error() {
        let raw = r#"[
            {"Text": {"text": "What happened?"}},
            {"ToolRequest": {"id": "call_2", "name": "read_file", "arguments": {"path": "missing.txt"}}},
            {"ToolResponse": {"id": "call_2", "tool_result": {"Err": "File not found"}}}
        ]"#;

        let parts = parse_content_json(raw);
        assert_eq!(parts.len(), 3);

        match &parts[0] {
            ContentPart::Text { text } => assert_eq!(text, "What happened?"),
            other => panic!("expected Text, got {other:?}"),
        }
        match &parts[1] {
            ContentPart::ToolUse { tool_name, tool_use_id, .. } => {
                assert_eq!(tool_name, "Read_file");
                assert_eq!(tool_use_id.as_deref(), Some("call_2"));
            }
            other => panic!("expected ToolUse, got {other:?}"),
        }
        match &parts[2] {
            ContentPart::ToolResult { summary, content, is_error, .. } => {
                assert_eq!(summary, "[Tool Error]");
                assert_eq!(content, "File not found");
                assert!(*is_error);
            }
            other => panic!("expected ToolResult, got {other:?}"),
        }
    }

    #[test]
    fn load_session_messages_and_search_docs() {
        let conn = setup_test_db();
        conn.execute(
            "INSERT INTO sessions (id, name, working_dir, created_at, updated_at) \
             VALUES ('ses_goose_1', 'Goose Test Session', '/workspace/goose', 1710000000, 1710001000)",
            [],
        )
        .unwrap();

        conn.execute(
            "INSERT INTO messages (session_id, role, content_json, created_timestamp, metadata_json) \
             VALUES ('ses_goose_1', 'user', '[{\"type\": \"text\", \"text\": \"Write a Rust test\"}]', 1710000001, '{\"model\": \"gpt-4o\"}')",
            [],
        )
        .unwrap();

        conn.execute(
            "INSERT INTO messages (session_id, role, content_json, created_timestamp, metadata_json) \
             VALUES ('ses_goose_1', 'assistant', '[{\"type\": \"text\", \"text\": \"Here is your test\"}]', 1710000002, '{\"model\": \"gpt-4o\"}')",
            [],
        )
        .unwrap();

        let msgs = load_session_messages(&conn, "ses_goose_1").unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "user");
        assert_eq!(msgs[0].model.as_deref(), Some("gpt-4o"));
        assert_eq!(msgs[1].role, "assistant");

        let docs = search_docs_from_messages(&msgs);
        assert_eq!(docs.len(), 2);
        assert_eq!(docs[0].search_text, "Write a Rust test");
        assert_eq!(docs[1].search_text, "Here is your test");
    }
}
