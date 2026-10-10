//! Kimi CLI 会话解析器。
//!
//! Kimi CLI 将会话日志写入 `~/.kimi-code/sessions/` 或 `~/.kimi/sessions/` 目录结构中：
//! `<session_dir>/agents/<agent_name>/wire.jsonl`
//! 同级伴随 `<session_dir>/state.json` 保存会话展示标题与子代理关系，
//! 根目录 `session_index.jsonl` 维护 `sessionId` / `sessionDir` 与工作目录 `workDir` 的映射。
//!
//! 协议记录包含：
//! - `metadata`: 会话创建元信息与 `created_at` 毫秒时间戳
//! - `config.update` / `profile.bind`: 模型别名与配置
//! - `context.append_message`: 完整消息记录（用户提问、助手回复或工具输出）
//! - `context.append_loop_event`: 事件流（内容块、工具调用 `tool.call`、工具结果 `tool.result`）
//! - `usage.record`: 单次调用 Token 用量

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use serde_json::Value;

use crate::error::AppResult;
use crate::parser::shared::{SearchDocument, SearchScanProgress};
use crate::session::{ChatMessage, ContentPart, SessionListMetadata, TokenUsage};

/// 从 `wire.jsonl` 文件路径推导会话 ID。
///
/// 路径形如 `<sessions_dir>/<wd_*>/<session_dir>/agents/<agent_name>/wire.jsonl`。
/// 若 agent 为 "main"，会话 ID 为 `<session_dir>`；
/// 若 agent 为子代理（如 "agent-0"），会话 ID 为 `<session_dir>:<agent_name>`。
pub(crate) fn session_id_from_path(path: &Path) -> String {
    let agent_name = path
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str());

    let session_dir = path
        .parent()
        .and_then(|p| p.parent())
        .and_then(|p| {
            if p.file_name().and_then(|n| n.to_str()) == Some("agents") {
                p.parent()
            } else {
                None
            }
        })
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str());

    match (session_dir, agent_name) {
        (Some(s), Some(a)) if a == "main" => s.to_string(),
        (Some(s), Some(a)) => format!("{s}:{a}"),
        (Some(s), None) => s.to_string(),
        _ => path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("kimi-session")
            .to_string(),
    }
}

/// 毫秒时间戳转换为 RFC3339 格式字符串。
pub(crate) fn epoch_ms_to_rfc3339(ms: i64) -> Option<String> {
    chrono::DateTime::from_timestamp_millis(ms).map(|dt| dt.to_rfc3339())
}

/// 解析单条 `context.append_message` 消息中的内容文本。
fn extract_message_content(entry: &Value) -> Option<String> {
    if let Some(content) = entry.get("content").and_then(Value::as_str) {
        return Some(content.to_string());
    }

    if let Some(parts) = entry.get("content").and_then(Value::as_array) {
        let mut text_parts = Vec::new();
        for part in parts {
            if let Some(t) = part.get("text").and_then(Value::as_str) {
                text_parts.push(t);
            } else if let Some(t) = part.as_str() {
                text_parts.push(t);
            }
        }
        if !text_parts.is_empty() {
            return Some(text_parts.join("\n"));
        }
    }

    None
}

/// 解析单行记录中的 Token 用量。
fn extract_token_usage(entry: &Value) -> Option<TokenUsage> {
    let input = entry
        .get("inputTokens")
        .or_else(|| entry.get("input_tokens"))
        .or_else(|| entry.get("prompt_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let output = entry
        .get("outputTokens")
        .or_else(|| entry.get("output_tokens"))
        .or_else(|| entry.get("completion_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let cache_read = entry
        .get("cacheReadInputTokens")
        .or_else(|| entry.get("cache_read_input_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let cache_write = entry
        .get("cacheCreationInputTokens")
        .or_else(|| entry.get("cache_creation_input_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);

    if input > 0 || output > 0 || cache_read > 0 || cache_write > 0 {
        Some(TokenUsage {
            input_tokens: input,
            output_tokens: output,
            cache_creation_input_tokens: cache_write,
            cache_read_input_tokens: cache_read,
        })
    } else {
        None
    }
}

/// 尝试读取 `<session_dir>/state.json` 中的元数据。
fn read_state_json(path: &Path) -> (Option<String>, HashMap<String, String>) {
    let session_dir = path
        .parent()
        .and_then(|p| p.parent())
        .and_then(|p| {
            if p.file_name().and_then(|n| n.to_str()) == Some("agents") {
                p.parent()
            } else {
                None
            }
        });

    let Some(session_dir) = session_dir else {
        return (None, HashMap::new());
    };

    let state_file = session_dir.join("state.json");
    if !state_file.exists() {
        return (None, HashMap::new());
    }

    let Ok(content) = std::fs::read_to_string(&state_file) else {
        return (None, HashMap::new());
    };

    let Ok(json) = serde_json::from_str::<Value>(&content) else {
        return (None, HashMap::new());
    };

    let title = json
        .get("title")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .map(str::to_string);

    let mut agent_parents = HashMap::new();
    if let Some(agents) = json.get("agents").and_then(Value::as_object) {
        for (name, entry) in agents {
            if let Some(parent) = entry.get("parentAgentId").and_then(Value::as_str) {
                agent_parents.insert(name.clone(), parent.to_string());
            }
        }
    }

    (title, agent_parents)
}

/// 从会话文件所在目录向外查找 `session_index.jsonl`，解析该会话对应的工作目录 `workDir`。
pub(crate) fn lookup_work_dir(path: &Path) -> Option<String> {
    let session_dir = path
        .parent()
        .and_then(|p| p.parent())
        .and_then(|p| {
            if p.file_name().and_then(|n| n.to_str()) == Some("agents") {
                p.parent()
            } else {
                None
            }
        });

    let Some(session_dir) = session_dir else {
        return None;
    };

    let session_dir_name = session_dir.file_name().and_then(|n| n.to_str())?;

    let mut current = session_dir.parent();
    while let Some(dir) = current {
        let index_file = dir.join("session_index.jsonl");
        if index_file.is_file() {
            if let Ok(file) = File::open(&index_file) {
                for line in BufReader::new(file).lines().map_while(Result::ok) {
                    if line.trim().is_empty() {
                        continue;
                    }
                    if let Ok(v) = serde_json::from_str::<Value>(&line) {
                        let matches_id = v
                            .get("sessionId")
                            .and_then(Value::as_str)
                            .is_some_and(|id| id == session_dir_name);
                        let matches_dir = v
                            .get("sessionDir")
                            .and_then(Value::as_str)
                            .is_some_and(|d| d.ends_with(session_dir_name));

                        if matches_id || matches_dir {
                            if let Some(wd) = v.get("workDir").and_then(Value::as_str) {
                                if !wd.trim().is_empty() {
                                    return Some(wd.to_string());
                                }
                            }
                        }
                    }
                }
            }
        }
        current = dir.parent();
    }

    None
}

/// 解析 Kimi 会话文件中的全部消息。
pub(crate) fn parse_session_file(path: &Path) -> AppResult<Vec<ChatMessage>> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    parse_from_reader(reader)
}

/// 从字符流中解析 Kimi 消息列表。
pub(crate) fn parse_from_content(content: &str) -> Vec<ChatMessage> {
    parse_from_reader(content.as_bytes()).unwrap_or_default()
}

fn parse_from_reader<R: BufRead>(reader: R) -> AppResult<Vec<ChatMessage>> {
    let mut messages: Vec<ChatMessage> = Vec::new();
    let mut current_model: Option<String> = None;
    let mut fallback_timestamp: Option<String> = None;
    let mut msg_uuid_counter: u64 = 0;

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => continue,
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let entry: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let line_type = match entry.get("type").and_then(Value::as_str) {
            Some(t) => t,
            None => continue,
        };

        let timestamp = entry
            .get("time")
            .and_then(Value::as_i64)
            .and_then(epoch_ms_to_rfc3339)
            .or_else(|| fallback_timestamp.clone())
            .unwrap_or_default();

        match line_type {
            "metadata" => {
                if let Some(ms) = entry.get("created_at").and_then(Value::as_i64) {
                    fallback_timestamp = epoch_ms_to_rfc3339(ms);
                }
            }
            "config.update" | "profile.bind" => {
                if let Some(model) = entry
                    .get("modelAlias")
                    .or_else(|| entry.get("model"))
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                {
                    current_model = Some(model.to_string());
                }
            }
            "turn.prompt" => {
                let prompt_text = entry
                    .get("prompt")
                    .or_else(|| entry.get("text"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                if !prompt_text.trim().is_empty() {
                    msg_uuid_counter += 1;
                    messages.push(ChatMessage {
                        role: "user".to_string(),
                        timestamp: timestamp.clone(),
                        model: None,
                        token_usage: None,
                        content_parts: vec![ContentPart::Text {
                            text: prompt_text.to_string(),
                        }],
                        is_meta: false,
                        uuid: Some(format!("kimi-user-{msg_uuid_counter}")),
                    });
                }
            }
            "context.append_message" => {
                let role_str = entry
                    .get("role")
                    .or_else(|| entry.pointer("/message/role"))
                    .and_then(Value::as_str)
                    .unwrap_or("user");

                let content_str = extract_message_content(&entry)
                    .or_else(|| entry.get("message").and_then(extract_message_content));

                match role_str {
                    "user" => {
                        if let Some(text) = content_str {
                            if !text.trim().is_empty() {
                                msg_uuid_counter += 1;
                                messages.push(ChatMessage {
                                    role: "user".to_string(),
                                    timestamp: timestamp.clone(),
                                    model: None,
                                    token_usage: None,
                                    content_parts: vec![ContentPart::Text { text }],
                                    is_meta: false,
                                    uuid: Some(format!("kimi-user-{msg_uuid_counter}")),
                                });
                            }
                        }
                    }
                    "assistant" => {
                        let mut content_parts = Vec::new();
                        if let Some(text) = content_str {
                            if !text.trim().is_empty() {
                                content_parts.push(ContentPart::Text { text });
                            }
                        }

                        if let Some(tool_calls) = entry
                            .get("toolCalls")
                            .or_else(|| entry.pointer("/message/toolCalls"))
                            .and_then(Value::as_array)
                        {
                            for tc in tool_calls {
                                let name = tc
                                    .get("name")
                                    .or_else(|| tc.pointer("/function/name"))
                                    .and_then(Value::as_str)
                                    .unwrap_or("tool");
                                let input = tc
                                    .get("arguments")
                                    .or_else(|| tc.pointer("/function/arguments"))
                                    .map(|v| v.to_string())
                                    .unwrap_or_default();
                                let call_id = tc
                                    .get("id")
                                    .or_else(|| tc.get("callId"))
                                    .and_then(Value::as_str)
                                    .map(str::to_string);
                                content_parts.push(ContentPart::ToolUse {
                                    summary: format!("{name}()"),
                                    tool_name: name.to_string(),
                                    input,
                                    tool_use_id: call_id,
                                });
                            }
                        }

                        if !content_parts.is_empty() {
                            msg_uuid_counter += 1;
                            messages.push(ChatMessage {
                                role: "assistant".to_string(),
                                timestamp: timestamp.clone(),
                                model: current_model.clone(),
                                token_usage: None,
                                content_parts,
                                is_meta: false,
                                uuid: Some(format!("kimi-assistant-{msg_uuid_counter}")),
                            });
                        }
                    }
                    "tool" => {
                        let output = content_str.unwrap_or_default();
                        let call_id = entry
                            .get("callId")
                            .or_else(|| entry.get("toolCallId"))
                            .and_then(Value::as_str)
                            .map(str::to_string);
                        let full_len = output.len() as u64;
                        msg_uuid_counter += 1;
                        messages.push(ChatMessage {
                            role: "assistant".to_string(),
                            timestamp: timestamp.clone(),
                            model: current_model.clone(),
                            token_usage: None,
                            content_parts: vec![ContentPart::ToolResult {
                                summary: "[Tool Result]".to_string(),
                                content: output,
                                is_error: false,
                                tool_use_id: call_id,
                                full_len,
                                truncated_preview: false,
                                truncated: false,
                                source_offset: 0,
                            }],
                            is_meta: false,
                            uuid: Some(format!("kimi-tool-{msg_uuid_counter}")),
                        });
                    }
                    _ => {}
                }
            }
            "context.append_loop_event" => {
                let event = entry.get("event").unwrap_or(&entry);
                let event_type = event.get("type").and_then(Value::as_str).unwrap_or("");

                match event_type {
                    "content.part" => {
                        let text = event
                            .get("text")
                            .or_else(|| event.pointer("/part/text"))
                            .and_then(Value::as_str);
                        if let Some(t) = text {
                            if !t.trim().is_empty() {
                                let text_part = ContentPart::Text {
                                    text: t.to_string(),
                                };
                                if let Some(last) = messages.last_mut() {
                                    if last.role == "assistant" {
                                        last.content_parts.push(text_part);
                                        continue;
                                    }
                                }
                                msg_uuid_counter += 1;
                                messages.push(ChatMessage {
                                    role: "assistant".to_string(),
                                    timestamp: timestamp.clone(),
                                    model: current_model.clone(),
                                    token_usage: None,
                                    content_parts: vec![text_part],
                                    is_meta: false,
                                    uuid: Some(format!("kimi-assistant-{msg_uuid_counter}")),
                                });
                            }
                        }
                    }
                    "tool.call" => {
                        let name = event
                            .get("tool_name")
                            .or_else(|| event.get("name"))
                            .and_then(Value::as_str)
                            .unwrap_or("tool");
                        let input = event
                            .get("input")
                            .or_else(|| event.get("arguments"))
                            .map(|v| v.to_string())
                            .unwrap_or_default();
                        let call_id = event
                            .get("call_id")
                            .or_else(|| event.get("id"))
                            .and_then(Value::as_str)
                            .map(str::to_string);

                        let tool_part = ContentPart::ToolUse {
                            summary: format!("{name}()"),
                            tool_name: name.to_string(),
                            input,
                            tool_use_id: call_id,
                        };

                        if let Some(last) = messages.last_mut() {
                            if last.role == "assistant" {
                                last.content_parts.push(tool_part);
                                continue;
                            }
                        }
                        msg_uuid_counter += 1;
                        messages.push(ChatMessage {
                            role: "assistant".to_string(),
                            timestamp: timestamp.clone(),
                            model: current_model.clone(),
                            token_usage: None,
                            content_parts: vec![tool_part],
                            is_meta: false,
                            uuid: Some(format!("kimi-assistant-{msg_uuid_counter}")),
                        });
                    }
                    "tool.result" => {
                        let output = event
                            .get("output")
                            .or_else(|| event.get("result"))
                            .and_then(|v| {
                                if let Some(s) = v.as_str() {
                                    Some(s.to_string())
                                } else {
                                    Some(v.to_string())
                                }
                            })
                            .unwrap_or_default();
                        let call_id = event
                            .get("call_id")
                            .or_else(|| event.get("id"))
                            .and_then(Value::as_str)
                            .map(str::to_string);
                        let is_err = event.get("is_error").and_then(Value::as_bool).unwrap_or(false);
                        let full_len = output.len() as u64;

                        let res_part = ContentPart::ToolResult {
                            summary: if is_err { "[Tool Error]".to_string() } else { "[Tool Result]".to_string() },
                            content: output,
                            is_error: is_err,
                            tool_use_id: call_id,
                            full_len,
                            truncated_preview: false,
                            truncated: false,
                            source_offset: 0,
                        };

                        if let Some(last) = messages.last_mut() {
                            if last.role == "assistant" {
                                last.content_parts.push(res_part);
                                continue;
                            }
                        }
                        msg_uuid_counter += 1;
                        messages.push(ChatMessage {
                            role: "assistant".to_string(),
                            timestamp: timestamp.clone(),
                            model: current_model.clone(),
                            token_usage: None,
                            content_parts: vec![res_part],
                            is_meta: false,
                            uuid: Some(format!("kimi-tool-{msg_uuid_counter}")),
                        });
                    }
                    _ => {}
                }
            }
            "usage.record" => {
                if let Some(usage) = extract_token_usage(&entry) {
                    if let Some(last) = messages.last_mut() {
                        if last.role == "assistant" {
                            last.token_usage = Some(usage);
                        }
                    }
                }
                if let Some(m) = entry.get("model").and_then(Value::as_str) {
                    current_model = Some(m.to_string());
                }
            }
            _ => {}
        }
    }

    Ok(messages)
}

/// 提取第一条用户消息作为标题兜底。
pub(crate) fn first_user_message(path: &Path) -> Option<String> {
    let file = File::open(path).ok()?;
    let reader = BufReader::new(file);

    for line in reader.lines().map_while(Result::ok) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(entry) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };

        if let Some("turn.prompt") = entry.get("type").and_then(Value::as_str) {
            if let Some(p) = entry.get("prompt").and_then(Value::as_str) {
                if !p.trim().is_empty() {
                    return Some(p.trim().to_string());
                }
            }
        }

        if let Some("context.append_message") = entry.get("type").and_then(Value::as_str) {
            let role = entry
                .get("role")
                .or_else(|| entry.pointer("/message/role"))
                .and_then(Value::as_str);
            if role == Some("user") {
                if let Some(content) = extract_message_content(&entry) {
                    if !content.trim().is_empty() {
                        return Some(content.trim().to_string());
                    }
                }
            }
        }
    }

    None
}

/// 获取会话 ID。
pub(crate) fn session_id(path: &Path) -> Option<String> {
    Some(session_id_from_path(path))
}

/// 读取工作目录原始路径。
pub(crate) fn project_path_raw(path: &Path) -> Option<String> {
    lookup_work_dir(path)
}

/// 读取会话列表元数据。
pub(crate) fn metadata_only(path: &Path) -> Option<SessionListMetadata> {
    let file = File::open(path).ok()?;
    let file_metadata = file.metadata().ok()?;
    let file_size = file_metadata.len();
    let reader = BufReader::new(file);

    let (state_title, _) = read_state_json(path);
    let session_id = session_id_from_path(path);
    let project_path = lookup_work_dir(path);

    let mut first_ts = None;
    let mut last_ts = None;
    let mut first_user_msg = None;

    for line in reader.lines().map_while(Result::ok) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(entry) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };

        if let Some(ms) = entry
            .get("time")
            .or_else(|| entry.get("created_at"))
            .and_then(Value::as_i64)
        {
            if let Some(rfc) = epoch_ms_to_rfc3339(ms) {
                if first_ts.is_none() {
                    first_ts = Some(rfc.clone());
                }
                last_ts = Some(rfc);
            }
        }

        if first_user_msg.is_none() {
            if let Some("turn.prompt") = entry.get("type").and_then(Value::as_str) {
                if let Some(p) = entry.get("prompt").and_then(Value::as_str) {
                    if !p.trim().is_empty() {
                        first_user_msg = Some(p.trim().to_string());
                    }
                }
            } else if let Some("context.append_message") = entry.get("type").and_then(Value::as_str) {
                let role = entry
                    .get("role")
                    .or_else(|| entry.pointer("/message/role"))
                    .and_then(Value::as_str);
                if role == Some("user") {
                    if let Some(content) = extract_message_content(&entry) {
                        if !content.trim().is_empty() {
                            first_user_msg = Some(content.trim().to_string());
                        }
                    }
                }
            }
        }
    }

    Some(SessionListMetadata {
        session_id,
        project_path,
        title: state_title,
        first_user_message: first_user_msg,
        first_timestamp: first_ts,
        last_timestamp: last_ts,
        git_branch: String::new(),
        file_size,
    })
}

/// 判断是否为子代理。
pub(crate) fn is_subagent(path: &Path) -> bool {
    let agent_name = path
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str());

    if let Some(name) = agent_name {
        if name != "main" {
            return true;
        }
    }

    let (_, agent_parents) = read_state_json(path);
    if let Some(name) = agent_name {
        return agent_parents.get(name).is_some();
    }

    false
}

/// 从字节流中生成用于 Tantivy 索引的文档。
pub(crate) fn search_docs_from_text(content: &[u8]) -> Option<Vec<SearchDocument>> {
    let reader = BufReader::new(content);
    let mut docs = Vec::new();
    let mut doc_index: usize = 0;

    for line in reader.lines().map_while(Result::ok) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(entry) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };

        let line_type = entry.get("type").and_then(Value::as_str).unwrap_or("");

        if line_type == "turn.prompt" {
            if let Some(p) = entry.get("prompt").and_then(Value::as_str) {
                if !p.trim().is_empty() {
                    doc_index += 1;
                    docs.push(SearchDocument {
                        message_index: doc_index,
                        role: "user".to_string(),
                        search_text: p.to_string(),
                    });
                }
            }
        } else if line_type == "context.append_message" {
            let role = entry
                .get("role")
                .or_else(|| entry.pointer("/message/role"))
                .and_then(Value::as_str)
                .unwrap_or("user");
            if let Some(content) = extract_message_content(&entry) {
                if !content.trim().is_empty() {
                    doc_index += 1;
                    docs.push(SearchDocument {
                        message_index: doc_index,
                        role: role.to_string(),
                        search_text: content,
                    });
                }
            }
        } else if line_type == "context.append_loop_event" {
            let event = entry.get("event").unwrap_or(&entry);
            if let Some(t) = event
                .get("text")
                .or_else(|| event.pointer("/part/text"))
                .and_then(Value::as_str)
            {
                if !t.trim().is_empty() {
                    doc_index += 1;
                    docs.push(SearchDocument {
                        message_index: doc_index,
                        role: "assistant".to_string(),
                        search_text: t.to_string(),
                    });
                }
            }
        }
    }

    if docs.is_empty() {
        None
    } else {
        Some(docs)
    }
}

/// 扫描会话文件生成全文检索文档。
pub(crate) fn scan_search_docs(
    path: &Path,
    on_progress: &mut dyn FnMut(SearchScanProgress),
) -> Option<Vec<SearchDocument>> {
    let bytes = std::fs::read(path).ok()?;
    let total = bytes.len() as u64;
    on_progress(SearchScanProgress {
        bytes_read: total,
        file_size: total,
    });
    search_docs_from_text(&bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_kimi_sample_records() {
        let sample = r#"{"type":"metadata","protocol_version":"1.5","created_at":1710000000000}
{"type":"config.update","modelAlias":"moonshot-v1-32k","time":1710000001000}
{"type":"turn.prompt","prompt":"帮我写一个快速排序算法","time":1710000002000}
{"type":"context.append_loop_event","event":{"type":"content.part","text":"没问题，这是 Rust 实现的快速排序："},"time":1710000003000}
{"type":"usage.record","inputTokens":120,"outputTokens":85,"model":"moonshot-v1-32k","time":1710000004000}"#;

        let messages = parse_from_content(sample);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, "user");
        match &messages[0].content_parts[0] {
            ContentPart::Text { text } => assert_eq!(text, "帮我写一个快速排序算法"),
            _ => panic!("Expected text part"),
        }
        assert_eq!(messages[1].role, "assistant");
        match &messages[1].content_parts[0] {
            ContentPart::Text { text } => assert_eq!(text, "没问题，这是 Rust 实现的快速排序："),
            _ => panic!("Expected text part"),
        }
        assert_eq!(messages[1].model.as_deref(), Some("moonshot-v1-32k"));
        let usage = messages[1].token_usage.as_ref().unwrap();
        assert_eq!(usage.input_tokens, 120);
        assert_eq!(usage.output_tokens, 85);
    }

    #[test]
    fn derives_session_id_from_path() {
        let main_path = Path::new("/Users/test/.kimi-code/sessions/wd_1/ses_123/agents/main/wire.jsonl");
        assert_eq!(session_id_from_path(main_path), "ses_123");

        let subagent_path =
            Path::new("/Users/test/.kimi-code/sessions/wd_1/ses_123/agents/agent-0/wire.jsonl");
        assert_eq!(session_id_from_path(subagent_path), "ses_123:agent-0");
    }

    #[test]
    fn extracts_search_documents() {
        let sample = r#"{"type":"metadata","created_at":1710000000000}
{"type":"turn.prompt","prompt":"查找系统信息","time":1710000001000}
{"type":"context.append_loop_event","event":{"type":"content.part","text":"正在检测系统硬件..."},"time":1710000002000}"#;

        let docs = search_docs_from_text(sample.as_bytes()).expect("检索文档");
        assert_eq!(docs.len(), 2);
        assert_eq!(docs[0].role, "user");
        assert_eq!(docs[0].search_text, "查找系统信息");
        assert_eq!(docs[1].role, "assistant");
        assert_eq!(docs[1].search_text, "正在检测系统硬件...");
    }
}
