//! Grok CLI 会话解析器。
//!
//! 目录结构：
//! `~/.grok/sessions/<url-encoded-cwd>/<session-uuid>/`
//! 包含文件：
//! - `summary.json`: 会话元数据（id, cwd, generated_title/session_summary, timestamps, model, session_kind 等）
//! - `chat_history.jsonl`: 消息转录（user, assistant, reasoning, system, tool_result, backend_tool_call 等）
//! - `updates.jsonl`: 追加式 ACP 增量流（promptIndex/toolCallId 时间戳、分轮用量 turn_completed 等）
//! - `subagents/<child-id>/meta.json`: 子代理元数据关联（可选）

use crate::error::AppResult;
use crate::parser::batch::BatchEmitter;
use crate::parser::shared::{
    scan_search_docs_from_text, scan_search_docs_with_extractor, truncate_preview_text,
    SearchDocument, SearchScanProgress,
};
use crate::session::{
    ChatMessage, ContentPart, SessionListMetadata, SessionLoadResult, SubagentInfo, TokenUsage,
    UsageRecord,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::Path;

/// 会话摘要结构（`summary.json`）。
#[derive(Debug, Clone, Deserialize, Default)]
pub struct GrokSummary {
    pub id: Option<String>,
    pub info: Option<GrokSummaryInfo>,
    pub cwd: Option<String>,
    pub generated_title: Option<String>,
    pub session_summary: Option<String>,
    pub title: Option<String>,
    pub created_at: Option<Value>,
    pub updated_at: Option<Value>,
    pub last_active_at: Option<Value>,
    pub current_model_id: Option<String>,
    pub model: Option<String>,
    pub session_kind: Option<String>,
    pub parent_session_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct GrokSummaryInfo {
    pub id: Option<String>,
    pub cwd: Option<String>,
}

impl GrokSummary {
    pub fn session_id(&self, fallback: &str) -> String {
        self.id
            .as_deref()
            .or_else(|| self.info.as_ref().and_then(|i| i.id.as_deref()))
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(fallback)
            .to_string()
    }

    pub fn cwd(&self) -> Option<String> {
        self.cwd
            .clone()
            .or_else(|| self.info.as_ref().and_then(|i| i.cwd.clone()))
            .filter(|s| !s.trim().is_empty())
    }

    pub fn title(&self) -> Option<String> {
        self.generated_title
            .clone()
            .or_else(|| self.session_summary.clone())
            .or_else(|| self.title.clone())
            .filter(|s| !s.trim().is_empty())
    }

    pub fn model(&self) -> Option<String> {
        self.current_model_id
            .clone()
            .or_else(|| self.model.clone())
            .filter(|s| !s.trim().is_empty())
    }

    pub fn is_subagent(&self) -> bool {
        if let Some(kind) = &self.session_kind {
            if kind.starts_with("subagent") {
                return true;
            }
        }
        if let Some(parent) = &self.parent_session_id {
            if !parent.trim().is_empty() {
                return true;
            }
        }
        false
    }
}

/// 解析各种形态的时间戳（ISO 8601 字符串、秒级整数、毫秒级整数）。
pub fn parse_timestamp_value(val: &Value) -> Option<String> {
    if let Some(s) = val.as_str() {
        let trimmed = s.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }
    if let Some(n) = val.as_i64().or_else(|| val.as_u64().map(|u| u as i64)) {
        if n <= 0 {
            return None;
        }
        // 秒 vs 毫秒判定
        let millis = if n < 10_000_000_000 { n * 1000 } else { n };
        if let Some(dt) = DateTime::from_timestamp_millis(millis) {
            return Some(dt.to_rfc3339());
        }
    }
    if let Some(f) = val.as_f64() {
        if f > 0.0 {
            let millis = if f < 10_000_000_000.0 {
                (f * 1000.0) as i64
            } else {
                f as i64
            };
            if let Some(dt) = DateTime::from_timestamp_millis(millis) {
                return Some(dt.to_rfc3339());
            }
        }
    }
    None
}

/// URL 百分比编码目录解码器（多字节 UTF-8 安全保护）。
pub fn decode_percent_encoded_path(input: &str) -> String {
    let mut bytes = Vec::with_capacity(input.len());
    let mut iter = input.bytes();
    while let Some(b) = iter.next() {
        if b == b'%' {
            let h1 = iter.next();
            let h2 = iter.next();
            if let (Some(c1), Some(c2)) = (h1, h2) {
                let hex = [c1, c2];
                if let Ok(hex_str) = std::str::from_utf8(&hex) {
                    if let Ok(val) = u8::from_str_radix(hex_str, 16) {
                        bytes.push(val);
                        continue;
                    }
                }
                bytes.push(b'%');
                bytes.push(c1);
                bytes.push(c2);
            } else {
                bytes.push(b'%');
                if let Some(c1) = h1 {
                    bytes.push(c1);
                }
                if let Some(c2) = h2 {
                    bytes.push(c2);
                }
            }
        } else {
            bytes.push(b);
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

/// 读取 `summary.json`。
pub fn read_summary_file(session_dir: &Path) -> Option<GrokSummary> {
    let path = session_dir.join("summary.json");
    if !path.exists() {
        return None;
    }
    let file = File::open(path).ok()?;
    serde_json::from_reader(BufReader::new(file)).ok()
}

/// 解析 `updates.jsonl` 中积累的增量时间戳与用量信息。
#[derive(Debug, Default)]
struct GrokUpdatesState {
    prompt_timestamps: HashMap<usize, String>,
    prompt_usages: HashMap<usize, TokenUsage>,
    prompt_models: HashMap<usize, String>,
    tool_timestamps: HashMap<String, String>,
    turn_usages: Vec<GrokTurnUsage>,
}

#[derive(Debug, Clone)]
struct GrokTurnUsage {
    timestamp: Option<String>,
    model: Option<String>,
    usage: TokenUsage,
    duration_ms: Option<u64>,
}

fn parse_updates_file(session_dir: &Path) -> GrokUpdatesState {
    let mut state = GrokUpdatesState::default();
    let path = session_dir.join("updates.jsonl");
    if !path.exists() {
        return state;
    }

    let file = match File::open(path) {
        Ok(f) => f,
        Err(_) => return state,
    };

    let reader = BufReader::new(file);
    for line in reader.lines().flatten() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let val: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let ts = val
            .get("timestamp")
            .and_then(parse_timestamp_value)
            .or_else(|| val.get("created_at").and_then(parse_timestamp_value))
            .or_else(|| val.get("time").and_then(parse_timestamp_value));

        let model = val
            .get("model")
            .or_else(|| val.get("current_model_id"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let duration_ms = val
            .get("duration_ms")
            .or_else(|| val.get("duration"))
            .and_then(|v| v.as_u64());

        // 提取 TokenUsage
        let usage_obj = val
            .get("usage")
            .or_else(|| val.get("token_usage"))
            .or_else(|| val.get("tokens"));
        let token_usage = usage_obj.and_then(parse_token_usage_value);

        if let Some(idx) = val
            .get("promptIndex")
            .or_else(|| val.get("prompt_index"))
            .or_else(|| val.get("turnIndex"))
            .or_else(|| val.get("turn"))
            .and_then(|v| v.as_u64())
            .map(|u| u as usize)
        {
            if let Some(t) = &ts {
                state.prompt_timestamps.insert(idx, t.clone());
            }
            if let Some(u) = token_usage.clone() {
                state.prompt_usages.insert(idx, u);
            }
            if let Some(m) = &model {
                state.prompt_models.insert(idx, m.clone());
            }
        }

        if let Some(tool_id) = val
            .get("toolCallId")
            .or_else(|| val.get("tool_call_id"))
            .and_then(|v| v.as_str())
        {
            if let Some(t) = &ts {
                state.tool_timestamps.insert(tool_id.to_string(), t.clone());
            }
        }

        // turn_completed / usage 记录
        let event_type = val
            .get("type")
            .or_else(|| val.get("event"))
            .and_then(|v| v.as_str())
            .unwrap_or("");

        if event_type == "turn_completed" || event_type == "usage" || token_usage.is_some() {
            if let Some(u) = token_usage {
                state.turn_usages.push(GrokTurnUsage {
                    timestamp: ts,
                    model,
                    usage: u,
                    duration_ms,
                });
            }
        }
    }

    state
}

fn parse_token_usage_value(val: &Value) -> Option<TokenUsage> {
    let input = val
        .get("input_tokens")
        .or_else(|| val.get("inputTokens"))
        .or_else(|| val.get("prompt_tokens"))
        .or_else(|| val.get("promptTokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    let output = val
        .get("output_tokens")
        .or_else(|| val.get("outputTokens"))
        .or_else(|| val.get("completion_tokens"))
        .or_else(|| val.get("completionTokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    let cache_creation = val
        .get("cache_creation_input_tokens")
        .or_else(|| val.get("cacheCreationInputTokens"))
        .or_else(|| val.get("cache_write_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    let cache_read = val
        .get("cache_read_input_tokens")
        .or_else(|| val.get("cacheReadInputTokens"))
        .or_else(|| val.get("cache_read_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    if input == 0 && output == 0 && cache_creation == 0 && cache_read == 0 {
        None
    } else {
        Some(TokenUsage {
            input_tokens: input,
            output_tokens: output,
            cache_creation_input_tokens: cache_creation,
            cache_read_input_tokens: cache_read,
        })
    }
}

/// 解析单条 JSON 消息为 ChatMessage。
fn parse_grok_message_line(
    entry: &Value,
    turn_idx: usize,
    summary: &GrokSummary,
    updates: &GrokUpdatesState,
) -> Option<ChatMessage> {
    // 兼容外层包装或直接消息对象
    let msg = entry.get("message").unwrap_or(entry);

    let role_raw = msg
        .get("role")
        .or_else(|| entry.get("type"))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let role_clean = role_raw.to_lowercase();
    if role_clean.is_empty() {
        return None;
    }

    let prompt_idx = entry
        .get("promptIndex")
        .or_else(|| entry.get("prompt_index"))
        .or_else(|| msg.get("promptIndex"))
        .and_then(|v| v.as_u64())
        .map(|u| u as usize)
        .unwrap_or(turn_idx);

    // 时间戳优先级：消息自带 -> updates 对应 prompt -> summary 的 last_active_at/created_at
    let timestamp = msg
        .get("timestamp")
        .and_then(parse_timestamp_value)
        .or_else(|| entry.get("timestamp").and_then(parse_timestamp_value))
        .or_else(|| updates.prompt_timestamps.get(&prompt_idx).cloned())
        .or_else(|| summary.last_active_at.as_ref().and_then(parse_timestamp_value))
        .or_else(|| summary.created_at.as_ref().and_then(parse_timestamp_value))
        .unwrap_or_else(|| Utc::now().to_rfc3339());

    // 模型
    let model = msg
        .get("model")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| updates.prompt_models.get(&prompt_idx).cloned())
        .or_else(|| summary.model());

    // 用量
    let token_usage = msg
        .get("usage")
        .and_then(parse_token_usage_value)
        .or_else(|| updates.prompt_usages.get(&prompt_idx).cloned());

    let uuid = msg
        .get("id")
        .or_else(|| entry.get("id"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let mut content_parts = Vec::new();

    // 检查推理内容
    if role_clean == "reasoning" {
        let text = extract_content_text(msg);
        if !text.is_empty() {
            content_parts.push(ContentPart::Thinking { thinking: text });
        }
        return Some(ChatMessage {
            role: "assistant".to_string(),
            timestamp,
            model,
            token_usage,
            content_parts,
            is_meta: false,
            uuid,
        });
    }

    if let Some(reasoning) = msg
        .get("reasoning")
        .or_else(|| msg.get("thinking"))
        .and_then(|v| v.as_str())
    {
        if !reasoning.trim().is_empty() {
            content_parts.push(ContentPart::Thinking {
                thinking: reasoning.to_string(),
            });
        }
    }

    // 处理工具调用或工具结果角色
    if role_clean == "tool_result" || role_clean == "tool" {
        let tool_use_id = msg
            .get("tool_use_id")
            .or_else(|| msg.get("toolCallId"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let content = extract_content_text(msg);
        let is_error = msg
            .get("is_error")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        content_parts.push(ContentPart::ToolResult {
            summary: format!("Tool result ({})", tool_use_id.as_deref().unwrap_or("call")),
            content,
            is_error,
            tool_use_id,
            full_len: 0,
            truncated_preview: false,
            truncated: false,
            source_offset: 0,
        });
        return Some(ChatMessage {
            role: "user".to_string(),
            timestamp,
            model,
            token_usage,
            content_parts,
            is_meta: false,
            uuid,
        });
    }

    if role_clean == "backend_tool_call" || role_clean == "tool_use" {
        let tool_name = msg
            .get("tool_name")
            .or_else(|| msg.get("name"))
            .and_then(|v| v.as_str())
            .unwrap_or("tool")
            .to_string();
        let tool_use_id = msg
            .get("id")
            .or_else(|| msg.get("tool_use_id"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let input = msg
            .get("input")
            .or_else(|| msg.get("arguments"))
            .map(|v| {
                if let Some(s) = v.as_str() {
                    s.to_string()
                } else {
                    v.to_string()
                }
            })
            .unwrap_or_default();
        content_parts.push(ContentPart::ToolUse {
            summary: format!("Call {}", tool_name),
            tool_name,
            input,
            tool_use_id,
        });
        return Some(ChatMessage {
            role: "assistant".to_string(),
            timestamp,
            model,
            token_usage,
            content_parts,
            is_meta: false,
            uuid,
        });
    }

    // 解析常规 content 字段
    if let Some(content_val) = msg.get("content") {
        if let Some(text) = content_val.as_str() {
            if !text.is_empty() {
                content_parts.push(ContentPart::Text {
                    text: text.to_string(),
                });
            }
        } else if let Some(arr) = content_val.as_array() {
            for item in arr {
                if let Some(text) = item.as_str() {
                    if !text.is_empty() {
                        content_parts.push(ContentPart::Text {
                            text: text.to_string(),
                        });
                    }
                    continue;
                }
                let part_type = item.get("type").and_then(|v| v.as_str()).unwrap_or("text");
                match part_type {
                    "text" => {
                        let text = item
                            .get("text")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        if !text.is_empty() {
                            content_parts.push(ContentPart::Text { text });
                        }
                    }
                    "reasoning" | "thinking" => {
                        let thinking = item
                            .get("thinking")
                            .or_else(|| item.get("text"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        if !thinking.is_empty() {
                            content_parts.push(ContentPart::Thinking { thinking });
                        }
                    }
                    "tool_use" | "backend_tool_call" => {
                        let tool_name = item
                            .get("tool_name")
                            .or_else(|| item.get("name"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("tool")
                            .to_string();
                        let tool_use_id = item
                            .get("id")
                            .or_else(|| item.get("tool_use_id"))
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());
                        let input = item
                            .get("input")
                            .or_else(|| item.get("arguments"))
                            .map(|v| {
                                if let Some(s) = v.as_str() {
                                    s.to_string()
                                } else {
                                    v.to_string()
                                }
                            })
                            .unwrap_or_default();
                        content_parts.push(ContentPart::ToolUse {
                            summary: format!("Call {}", tool_name),
                            tool_name,
                            input,
                            tool_use_id,
                        });
                    }
                    "tool_result" => {
                        let tool_use_id = item
                            .get("tool_use_id")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());
                        let content = item
                            .get("content")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let is_error = item
                            .get("is_error")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false);
                        content_parts.push(ContentPart::ToolResult {
                            summary: "Tool result".to_string(),
                            content,
                            is_error,
                            tool_use_id,
                            full_len: 0,
                            truncated_preview: false,
                            truncated: false,
                            source_offset: 0,
                        });
                    }
                    _ => {
                        if let Some(text) = item.get("text").and_then(|v| v.as_str()) {
                            if !text.is_empty() {
                                content_parts.push(ContentPart::Text {
                                    text: text.to_string(),
                                });
                            }
                        }
                    }
                }
            }
        }
    } else if let Some(text) = msg.get("text").and_then(|v| v.as_str()) {
        if !text.is_empty() {
            content_parts.push(ContentPart::Text {
                text: text.to_string(),
            });
        }
    }

    // 检查是否有 tool_calls 字段（OpenAI 格式）
    if let Some(tool_calls) = msg
        .get("tool_calls")
        .or_else(|| msg.get("backend_tool_calls"))
        .and_then(|v| v.as_array())
    {
        for tc in tool_calls {
            let tool_name = tc
                .get("function")
                .and_then(|f| f.get("name"))
                .or_else(|| tc.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or("tool")
                .to_string();
            let tool_use_id = tc.get("id").and_then(|v| v.as_str()).map(|s| s.to_string());
            let input = tc
                .get("function")
                .and_then(|f| f.get("arguments"))
                .or_else(|| tc.get("input"))
                .map(|v| {
                    if let Some(s) = v.as_str() {
                        s.to_string()
                    } else {
                        v.to_string()
                    }
                })
                .unwrap_or_default();
            content_parts.push(ContentPart::ToolUse {
                summary: format!("Call {}", tool_name),
                tool_name,
                input,
                tool_use_id,
            });
        }
    }

    if content_parts.is_empty() {
        return None;
    }

    let final_role = match role_clean.as_str() {
        "user" => "user",
        "assistant" => "assistant",
        "system" => "system",
        _ => "assistant",
    };

    Some(ChatMessage {
        role: final_role.to_string(),
        timestamp,
        model,
        token_usage,
        content_parts,
        is_meta: final_role == "system",
        uuid,
    })
}

fn extract_content_text(msg: &Value) -> String {
    if let Some(s) = msg.get("content").and_then(|v| v.as_str()) {
        return s.to_string();
    }
    if let Some(s) = msg.get("text").and_then(|v| v.as_str()) {
        return s.to_string();
    }
    if let Some(arr) = msg.get("content").and_then(|v| v.as_array()) {
        let mut out = String::new();
        for item in arr {
            if let Some(s) = item.as_str() {
                out.push_str(s);
            } else if let Some(s) = item.get("text").and_then(|v| v.as_str()) {
                out.push_str(s);
            }
        }
        return out;
    }
    String::new()
}

/// 解析 Grok 会话文件（`chat_history.jsonl`）。
pub fn parse_session_file(file_path: &str) -> AppResult<Vec<ChatMessage>> {
    let path = Path::new(file_path);
    let session_dir = if path.is_file() {
        path.parent().unwrap_or(path)
    } else {
        path
    };

    let summary = read_summary_file(session_dir).unwrap_or_default();
    let updates = parse_updates_file(session_dir);

    let chat_file_path = if path.is_file() {
        path.to_path_buf()
    } else {
        session_dir.join("chat_history.jsonl")
    };

    let file = File::open(&chat_file_path)?;
    let reader = BufReader::new(file);

    let mut messages = Vec::new();
    let mut turn_idx = 0;

    for line in reader.lines() {
        let line = line?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let entry: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(_) => continue,
        };

        if let Some(msg) = parse_grok_message_line(&entry, turn_idx, &summary, &updates) {
            turn_idx += 1;
            messages.push(msg);
        }
    }

    Ok(messages)
}

/// 流式读取 Grok 会话文件。
pub fn parse_session_file_streaming<F>(
    file_path: &str,
    _skip_sidechain: bool,
    mut on_batch: F,
) -> AppResult<(u64, HashMap<String, SubagentInfo>)>
where
    F: FnMut(Vec<ChatMessage>) -> bool,
{
    let path = Path::new(file_path);
    let session_dir = if path.is_file() {
        path.parent().unwrap_or(path)
    } else {
        path
    };

    let summary = read_summary_file(session_dir).unwrap_or_default();
    let updates = parse_updates_file(session_dir);

    let chat_file_path = if path.is_file() {
        path.to_path_buf()
    } else {
        session_dir.join("chat_history.jsonl")
    };

    let file = File::open(&chat_file_path)?;
    let mut reader = BufReader::new(file);

    let mut emitter = BatchEmitter::new();
    let mut total_bytes = 0u64;
    let mut turn_idx = 0;
    let mut line_buf = String::new();

    loop {
        line_buf.clear();
        let bytes_read = match reader.read_line(&mut line_buf) {
            Ok(0) => break,
            Ok(n) => n as u64,
            Err(e) => return Err(e.into()),
        };
        total_bytes += bytes_read;

        let trimmed = line_buf.trim();
        if trimmed.is_empty() {
            continue;
        }

        let entry: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(_) => continue,
        };

        if let Some(msg) = parse_grok_message_line(&entry, turn_idx, &summary, &updates) {
            turn_idx += 1;
            emitter.push(msg);
            if !emitter.maybe_flush(&mut on_batch) {
                return Ok((total_bytes, discover_subagents(session_dir)));
            }
        }
    }

    emitter.flush_remaining(&mut on_batch);
    Ok((total_bytes, discover_subagents(session_dir)))
}

/// 增量读取 Grok 会话文件。
pub fn parse_session_incremental(
    file_path: &str,
    offset: u64,
    _skip_sidechain: bool,
) -> AppResult<SessionLoadResult> {
    let path = Path::new(file_path);
    let session_dir = if path.is_file() {
        path.parent().unwrap_or(path)
    } else {
        path
    };

    let summary = read_summary_file(session_dir).unwrap_or_default();
    let updates = parse_updates_file(session_dir);

    let chat_file_path = if path.is_file() {
        path.to_path_buf()
    } else {
        session_dir.join("chat_history.jsonl")
    };

    let mut file = File::open(&chat_file_path)?;
    let file_len = file.metadata()?.len();
    if offset >= file_len {
        return Ok(SessionLoadResult {
            messages: Vec::new(),
            offset: file_len,
            subagent_map: discover_subagents(session_dir),
        });
    }

    file.seek(SeekFrom::Start(offset))?;
    let reader = BufReader::new(file);

    let mut messages = Vec::new();
    let mut turn_idx = 0;

    for line in reader.lines() {
        let line = line?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let entry: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if let Some(msg) = parse_grok_message_line(&entry, turn_idx, &summary, &updates) {
            turn_idx += 1;
            messages.push(msg);
        }
    }

    Ok(SessionLoadResult {
        messages,
        offset: file_len,
        subagent_map: discover_subagents(session_dir),
    })
}

/// 从内存文本解析消息（归档兜底用）。
pub fn parse_from_content(content: &str) -> Vec<ChatMessage> {
    let summary = GrokSummary::default();
    let updates = GrokUpdatesState::default();

    let mut messages = Vec::new();
    let mut turn_idx = 0;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let entry: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if let Some(msg) = parse_grok_message_line(&entry, turn_idx, &summary, &updates) {
            turn_idx += 1;
            messages.push(msg);
        }
    }

    messages
}

/// 扫描发现 subagents 目录下的子代理会话。
fn discover_subagents(session_dir: &Path) -> HashMap<String, SubagentInfo> {
    let mut subagents = HashMap::new();
    let subagents_dir = session_dir.join("subagents");
    if !subagents_dir.is_dir() {
        return subagents;
    }

    let entries = match std::fs::read_dir(subagents_dir) {
        Ok(e) => e,
        Err(_) => return subagents,
    };

    for entry in entries.flatten() {
        let child_dir = entry.path();
        if !child_dir.is_dir() {
            continue;
        }
        let child_id = child_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();
        if child_id.is_empty() {
            continue;
        }

        let meta_path = child_dir.join("meta.json");
        let description = if meta_path.exists() {
            if let Ok(file) = File::open(&meta_path) {
                if let Ok(val) = serde_json::from_reader::<_, Value>(BufReader::new(file)) {
                    val.get("description")
                        .or_else(|| val.get("title"))
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        let child_history = child_dir.join("chat_history.jsonl");
        let path_str = if child_history.exists() {
            child_history.to_string_lossy().to_string()
        } else {
            child_dir.to_string_lossy().to_string()
        };

        subagents.insert(
            child_id.clone(),
            SubagentInfo {
                file_path: path_str,
                label: description.unwrap_or(child_id),
            },
        );
    }

    subagents
}

/// 读取首条用户消息。
pub fn read_first_user_message(file_path: &str) -> Option<String> {
    let path = Path::new(file_path);
    let chat_file_path = if path.is_file() {
        path.to_path_buf()
    } else {
        path.join("chat_history.jsonl")
    };

    let file = File::open(chat_file_path).ok()?;
    let reader = BufReader::new(file);

    for line in reader.lines().flatten() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let entry: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let msg = entry.get("message").unwrap_or(&entry);
        let role = msg
            .get("role")
            .or_else(|| entry.get("type"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if role.eq_ignore_ascii_case("user") {
            let text = extract_content_text(msg);
            let cleaned = text.trim();
            if !cleaned.is_empty() {
                return truncate_preview_text(cleaned, 120);
            }
        }
    }
    None
}

/// 读取会话标识（`summary.json` 中的 id，或退回会话目录名）。
pub fn read_session_id(file_path: &str) -> Option<String> {
    let path = Path::new(file_path);
    let session_dir = if path.is_file() {
        path.parent()?
    } else {
        path
    };

    if let Some(summary) = read_summary_file(session_dir) {
        let id = summary.session_id("");
        if !id.is_empty() {
            return Some(id);
        }
    }

    session_dir
        .file_name()
        .and_then(|n| n.to_str())
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.to_string())
}

/// 读取权威项目路径（优先读 `summary.json` 的 `cwd`，退回祖父目录名 URL 解码）。
pub fn read_project_path(file_path: &str) -> Option<String> {
    let path = Path::new(file_path);
    let session_dir = if path.is_file() {
        path.parent()?
    } else {
        path
    };

    if let Some(summary) = read_summary_file(session_dir) {
        if let Some(cwd) = summary.cwd() {
            return Some(cwd);
        }
    }

    // ~/.grok/sessions/<encoded-cwd>/<session-uuid>/
    let encoded_cwd = session_dir.parent()?.file_name()?.to_str()?;
    let decoded = decode_percent_encoded_path(encoded_cwd);
    if decoded.trim().is_empty() {
        None
    } else {
        Some(decoded)
    }
}

/// 快速扫描元数据（轻量读盘，优先读 `summary.json`）。
pub fn scan_session_metadata_only(path: &Path) -> Option<SessionListMetadata> {
    let session_dir = if path.is_file() {
        path.parent()?
    } else {
        path
    };

    let session_id = read_session_id(&path.to_string_lossy())?;
    let project_path = read_project_path(&path.to_string_lossy());

    let summary = read_summary_file(session_dir);
    let title = summary.as_ref().and_then(|s| s.title());

    let first_user_message = read_first_user_message(&path.to_string_lossy());

    let first_ts = summary
        .as_ref()
        .and_then(|s| s.created_at.as_ref())
        .and_then(parse_timestamp_value);
    let last_ts = summary
        .as_ref()
        .and_then(|s| s.last_active_at.as_ref().or(s.updated_at.as_ref()))
        .and_then(parse_timestamp_value)
        .or_else(|| first_ts.clone());

    let file_size = if path.is_file() {
        std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
    } else {
        let chat_path = session_dir.join("chat_history.jsonl");
        std::fs::metadata(&chat_path).map(|m| m.len()).unwrap_or(0)
    };

    Some(SessionListMetadata {
        session_id,
        project_path,
        title,
        first_user_message,
        first_timestamp: first_ts,
        last_timestamp: last_ts,
        git_branch: String::new(),
        file_size,
    })
}

/// 判定是否为子代理会话文件。
pub fn is_subagent_file(file_path: &str) -> bool {
    let path = Path::new(file_path);
    if path.components().any(|c| c.as_os_str() == "subagents") {
        return true;
    }
    let session_dir = if path.is_file() {
        path.parent().unwrap_or(path)
    } else {
        path
    };
    if let Some(summary) = read_summary_file(session_dir) {
        if summary.is_subagent() {
            return true;
        }
    }
    false
}

/// 提取用量记录（从 `updates.jsonl` 中读取）。
pub fn extract_usage_records(file_path: &str, project: &str) -> Vec<UsageRecord> {
    let path = Path::new(file_path);
    let session_dir = if path.is_file() {
        path.parent().unwrap_or(path)
    } else {
        path
    };

    let summary = read_summary_file(session_dir).unwrap_or_default();
    let updates = parse_updates_file(session_dir);

    let default_model = summary.model().unwrap_or_else(|| "grok".to_string());
    let default_date = summary
        .created_at
        .as_ref()
        .and_then(parse_timestamp_value)
        .and_then(|ts| ts.get(..10).map(|s| s.to_string()))
        .unwrap_or_else(|| Utc::now().format("%Y-%m-%d").to_string());

    let mut records = Vec::new();
    for turn in updates.turn_usages {
        let date = turn
            .timestamp
            .as_deref()
            .and_then(|ts| ts.get(..10))
            .map(|s| s.to_string())
            .unwrap_or_else(|| default_date.clone());

        let model = turn.model.unwrap_or_else(|| default_model.clone());

        records.push(UsageRecord {
            date,
            model,
            input_tokens: turn.usage.input_tokens,
            output_tokens: turn.usage.output_tokens,
            cache_creation_tokens: turn.usage.cache_creation_input_tokens,
            cache_read_tokens: turn.usage.cache_read_input_tokens,
            duration_ms: turn.duration_ms,
            project: project.to_string(),
        });
    }

    records
}

/// 全文检索文档抽取闭包。
fn grok_search_extractor(entry: &Value) -> Option<(String, String)> {
    let msg = entry.get("message").unwrap_or(entry);
    let role = msg
        .get("role")
        .or_else(|| entry.get("type"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let role_clean = role.to_lowercase();
    if role_clean.is_empty() || role_clean == "system" {
        return None;
    }

    let text = extract_content_text(msg);
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }

    let mapped_role = match role_clean.as_str() {
        "user" => "user",
        "assistant" | "reasoning" | "backend_tool_call" => "assistant",
        "tool_result" => "tool",
        _ => "assistant",
    };

    Some((mapped_role.to_string(), trimmed.to_string()))
}

pub fn scan_session_search_docs_with_progress<F>(
    file_path: &Path,
    on_progress: F,
) -> Option<Vec<SearchDocument>>
where
    F: FnMut(SearchScanProgress),
{
    scan_search_docs_with_extractor(file_path, grok_search_extractor, on_progress)
}

pub fn scan_session_search_docs_from_bytes(content: &[u8]) -> Option<Vec<SearchDocument>> {
    scan_search_docs_from_text(content, grok_search_extractor)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_percent_decoding() {
        assert_eq!(
            decode_percent_encoded_path("%2FUsers%2Ffoo%2Fbar"),
            "/Users/foo/bar"
        );
        assert_eq!(
            decode_percent_encoded_path("%E4%BD%A0%E5%A5%BD%E4%B8%96%E7%95%8C"),
            "你好世界"
        );
        assert_eq!(decode_percent_encoded_path("plain_path"), "plain_path");
        assert_eq!(decode_percent_encoded_path("%2"), "%2");
        assert_eq!(decode_percent_encoded_path("%ZZ"), "%ZZ");
    }

    #[test]
    fn test_parse_timestamp_value() {
        let iso = Value::String("2026-03-20T10:00:00Z".to_string());
        assert_eq!(
            parse_timestamp_value(&iso),
            Some("2026-03-20T10:00:00Z".to_string())
        );

        let sec = Value::Number(serde_json::Number::from(1710928800i64));
        assert!(parse_timestamp_value(&sec).is_some());

        let milli = Value::Number(serde_json::Number::from(1710928800000i64));
        assert!(parse_timestamp_value(&milli).is_some());
    }

    #[test]
    fn test_parse_grok_session_end_to_end() {
        let dir = tempdir().unwrap();
        let session_dir = dir.path().join("sess-uuid-1");
        std::fs::create_dir_all(&session_dir).unwrap();

        // summary.json
        let summary_json = r#"{
            "id": "sess-uuid-1",
            "cwd": "/Users/test/my-project",
            "generated_title": "Grok Session Title",
            "created_at": "2026-03-20T10:00:00Z",
            "current_model_id": "grok-2",
            "session_kind": "main"
        }"#;
        std::fs::write(session_dir.join("summary.json"), summary_json).unwrap();

        // updates.jsonl
        let updates_jsonl = concat!(
            r#"{"type":"turn_completed","promptIndex":1,"usage":{"input_tokens":100,"output_tokens":40},"timestamp":"2026-03-20T10:00:05Z"}"#,
            "\n",
        );
        std::fs::write(session_dir.join("updates.jsonl"), updates_jsonl).unwrap();

        // chat_history.jsonl
        let chat_jsonl = concat!(
            r#"{"role":"user","content":"Hello Grok"}"#, "\n",
            r#"{"role":"reasoning","content":"Analyzing request..."}"#, "\n",
            r#"{"role":"assistant","content":"Hello there!","promptIndex":1}"#, "\n",
        );
        let chat_path = session_dir.join("chat_history.jsonl");
        std::fs::write(&chat_path, chat_jsonl).unwrap();

        let path_str = chat_path.to_str().unwrap();

        // 1. Session ID & Project Path
        assert_eq!(read_session_id(path_str).as_deref(), Some("sess-uuid-1"));
        assert_eq!(
            read_project_path(path_str).as_deref(),
            Some("/Users/test/my-project")
        );
        assert_eq!(
            read_first_user_message(path_str).as_deref(),
            Some("Hello Grok")
        );

        // 2. Full Parse
        let msgs = parse_session_file(path_str).unwrap();
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[0].role, "user");
        assert_eq!(msgs[1].role, "assistant");
        assert!(matches!(msgs[1].content_parts[0], ContentPart::Thinking { .. }));
        assert_eq!(msgs[2].role, "assistant");
        assert_eq!(msgs[2].model.as_deref(), Some("grok-2"));
        assert!(msgs[2].token_usage.is_some());
        assert_eq!(msgs[2].token_usage.as_ref().unwrap().input_tokens, 100);

        // 3. Metadata Only
        let meta = scan_session_metadata_only(&chat_path).unwrap();
        assert_eq!(meta.session_id, "sess-uuid-1");
        assert_eq!(meta.title.as_deref(), Some("Grok Session Title"));

        // 4. Usage Records
        let usages = extract_usage_records(path_str, "/Users/test/my-project");
        assert_eq!(usages.len(), 1);
        assert_eq!(usages[0].input_tokens, 100);
        assert_eq!(usages[0].output_tokens, 40);

        // 5. Search Docs
        let docs = scan_session_search_docs_from_bytes(chat_jsonl.as_bytes()).unwrap();
        assert_eq!(docs.len(), 3);
        assert!(docs[0].search_text.contains("Hello Grok"));
        assert!(docs[2].search_text.contains("Hello there!"));
    }

    #[test]
    fn test_subagent_detection() {
        let dir = tempdir().unwrap();
        let session_dir = dir.path().join("sub-1");
        std::fs::create_dir_all(&session_dir).unwrap();

        let summary_json = r#"{
            "id": "sub-1",
            "session_kind": "subagent_fork",
            "parent_session_id": "root-1"
        }"#;
        std::fs::write(session_dir.join("summary.json"), summary_json).unwrap();
        let chat_path = session_dir.join("chat_history.jsonl");
        std::fs::write(&chat_path, "").unwrap();

        assert!(is_subagent_file(chat_path.to_str().unwrap()));
    }
}
