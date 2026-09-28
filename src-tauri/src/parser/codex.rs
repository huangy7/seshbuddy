//! Codex 会话解析器。依赖方向：parser → session / parser → parser::shared。
//! 所有跨 CLI 的共享工具在 shared.rs，本模块只保留 Codex 专用逻辑。

use crate::error::AppResult;
use crate::parser::batch::BatchEmitter;
use crate::parser::shared::read_last_timestamp_from_file;
use crate::session::{self, ChatMessage, ContentPart};
use serde_json::Value;
use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::Path;

// 原 shared.rs 中的 Codex 命名通用工具，经此 re-export 供分派器与调用侧使用
pub(crate) use crate::parser::shared::{
    codex_tool_input_value, codex_tool_result_is_error, codex_value_to_text,
};

/// 单次读取 `~/.codex/session_index.jsonl`（尊重 CLI 路径覆盖配置），
/// 构建 SessionID/路径 → thread_name 的内存 Map，供 Codex 原生标题 O(1) 查询。
/// 文件缺失或行解析失败时跳过该行，整体不报错（标题解析是增强而非硬依赖）。
pub(crate) fn load_codex_index_titles() -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    let index_path = match crate::cli::data_dir(crate::cli::CliKind::Codex) {
        Ok(dir) => dir.join("session_index.jsonl"),
        Err(_) => return map,
    };
    let content = match std::fs::read_to_string(&index_path) {
        Ok(content) => content,
        Err(_) => return map,
    };

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(entry) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };
        let Some(title) = entry
            .get("thread_name")
            .or_else(|| entry.get("threadName"))
            .or_else(|| entry.get("title"))
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
        else {
            continue;
        };

        // 收录所有可能的键形态：id/session_id/thread_id 及各路径字段与其 file_stem
        for key in ["id", "session_id", "sessionId", "thread_id", "threadId"] {
            if let Some(k) = entry.get(key).and_then(|v| v.as_str()) {
                if !k.is_empty() {
                    map.insert(k.to_string(), title.clone());
                }
            }
        }
        for key in ["path", "session_path", "file"] {
            if let Some(p) = entry.get(key).and_then(|v| v.as_str()) {
                if !p.is_empty() {
                    map.insert(p.to_string(), title.clone());
                    if let Some(stem) = Path::new(p).file_stem().and_then(|s| s.to_str()) {
                        if !stem.is_empty() {
                            map.insert(stem.to_string(), title.clone());
                        }
                    }
                }
            }
        }
    }

    map
}

pub(crate) fn parse_rfc3339_to_seconds(ts: Option<&str>) -> i64 {
    let Some(ts) = ts else { return 0 };
    chrono::DateTime::parse_from_rfc3339(ts)
        .map(|dt| dt.timestamp())
        .or_else(|_| ts.parse::<chrono::DateTime<chrono::Utc>>().map(|dt| dt.timestamp()))
        .unwrap_or(0)
}

pub(crate) fn is_codex_subagent_meta(payload: &Value) -> bool {
    payload.pointer("/source/subagent/thread_spawn").is_some()
        || payload.get("source").and_then(|s| s.get("subagent")).is_some()
}

pub(crate) fn is_codex_subagent_file(file_path: &str) -> bool {
    let Ok(file) = File::open(file_path) else {
        return false;
    };
    let reader = BufReader::new(file);
    for line in reader.lines().take(10) {
        let Ok(line) = line else { continue };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(entry) = serde_json::from_str::<Value>(trimmed) {
            if entry.get("type").and_then(Value::as_str) == Some("session_meta") {
                if let Some(payload) = entry.get("payload") {
                    return is_codex_subagent_meta(payload);
                }
            }
        }
    }
    false
}

fn find_subagent_file_recursive(dir: &Path, agent_thread_id: &str, depth: usize) -> Option<std::path::PathBuf> {
    if depth == 0 {
        return None;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return None;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_subagent_file_recursive(&path, agent_thread_id, depth - 1) {
                return Some(found);
            }
        } else if path.is_file() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name.ends_with(&format!("-{}.jsonl", agent_thread_id))
                    || name.ends_with(&format!("{}.jsonl", agent_thread_id))
                    || name.contains(agent_thread_id)
                {
                    return Some(path);
                }
            }
        }
    }
    None
}

pub(crate) fn find_codex_subagent_file(
    parent_session_path: Option<&Path>,
    agent_thread_id: &str,
) -> Option<std::path::PathBuf> {
    if agent_thread_id.trim().is_empty() {
        return None;
    }
    // 1. 优先在父会话同级目录查找
    if let Some(parent_file) = parent_session_path {
        if let Some(parent_dir) = parent_file.parent() {
            if let Ok(entries) = std::fs::read_dir(parent_dir) {
                for entry in entries.flatten() {
                    let p = entry.path();
                    if p.is_file() {
                        if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                            if name.ends_with(&format!("-{}.jsonl", agent_thread_id))
                                || name.ends_with(&format!("{}.jsonl", agent_thread_id))
                                || name.contains(agent_thread_id)
                            {
                                return Some(p);
                            }
                        }
                    }
                }
            }
        }
    }
    // 2. 全量递归 ~/.codex/sessions 目录查找
    if let Ok(sessions_dir) = crate::cli::sessions_dir(crate::cli::CliKind::Codex) {
        if sessions_dir.exists() {
            return find_subagent_file_recursive(&sessions_dir, agent_thread_id, 6);
        }
    }
    None
}

pub(crate) fn scan_codex_metadata_only(file_path: &Path) -> Option<session::SessionListMetadata> {
    let mut file = File::open(file_path).ok()?;
    let file_size = file.seek(SeekFrom::End(0)).ok()?;
    file.seek(SeekFrom::Start(0)).ok()?;

    let reader = BufReader::new(&file);
    let mut has_chat_messages = false;
    let mut session_id = None;
    let mut project_path = None;
    let mut first_user_message = None;
    let mut first_timestamp = None;

    const HEAD_LINES: usize = 50;
    for (_idx, line) in reader.lines().take(HEAD_LINES).enumerate() {
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

        if first_timestamp.is_none() {
            if let Some(ts) = entry.get("timestamp").and_then(|v| v.as_str()) {
                first_timestamp = Some(ts.to_string());
            }
        }

        let entry_type = entry.get("type").and_then(|v| v.as_str()).unwrap_or("");
        if entry_type == "session_meta" {
            if let Some(payload) = entry.get("payload") {
                // Subagent 会话直接排除，不进入侧边栏独立会话列表
                if is_codex_subagent_meta(payload) {
                    return None;
                }
                if session_id.is_none() {
                    session_id = payload
                        .get("id")
                        .and_then(|v| v.as_str())
                        .map(|v| v.to_string());
                }
                if project_path.is_none() {
                    // 与下方 `turn_context` 分支同一条不变量：空白 cwd 不是路径。
                    // 少了这道过滤就会把 `Some("")` 写进 `session_list_index.project_path`，
                    // 而这一列的缺席语义是 `NULL`，空串会变成第三种表达。
                    project_path = payload
                        .get("cwd")
                        .and_then(|v| v.as_str())
                        .filter(|v| !v.trim().is_empty())
                        .map(|v| v.to_string());
                }
            }
        } else if project_path.is_none() && entry_type == "turn_context" {
            project_path = entry
                .get("payload")
                .and_then(|payload| payload.get("cwd"))
                .and_then(|v| v.as_str())
                .filter(|v| !v.trim().is_empty())
                .map(|v| v.to_string());
        }

        if let Some((role, text)) = extract_codex_role_text(&entry) {
            has_chat_messages = true;
            if role == "user" && first_user_message.is_none() {
                first_user_message = crate::title_resolver::clean_fallback_user_text(&text);
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
        project_path,
        // Codex 的原生标题（thread_name）由调用侧经 session_index.jsonl 内存 Map 注入
        title: None,
        first_user_message,
        first_timestamp: first_timestamp.clone(),
        last_timestamp: last_timestamp.or(first_timestamp),
        git_branch: String::new(),
        file_size,
    })
}

#[derive(Default)]
pub(crate) struct CodexParser {
    pub(crate) is_subagent: bool,
    pub(crate) subagent_start_seconds: Option<i64>,
    pub(crate) skipping_fork_context: bool,
    pub(crate) session_meta_count: usize,
    pub(crate) subagent_map: std::collections::HashMap<String, session::SubagentInfo>,
    pub(crate) subagent_call_names: std::collections::HashMap<String, String>,
    pub(crate) current_model: Option<String>,
}

impl CodexParser {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn process_line(
        &mut self,
        line: &str,
        session_file_path: Option<&Path>,
        messages: &mut Vec<ChatMessage>,
    ) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return;
        }
        let Ok(entry) = serde_json::from_str::<Value>(trimmed) else {
            return;
        };

        let entry_type = entry.get("type").and_then(Value::as_str).unwrap_or("");
        let timestamp = entry
            .get("timestamp")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let payload = entry.get("payload");

        match entry_type {
            "session_meta" => {
                self.session_meta_count += 1;
                if let Some(payload) = payload {
                    if self.session_meta_count == 1 {
                        if is_codex_subagent_meta(payload) {
                            self.is_subagent = true;
                            let sub_ts = parse_rfc3339_to_seconds(
                                payload
                                    .get("timestamp")
                                    .and_then(Value::as_str)
                                    .or(Some(&timestamp)),
                            );
                            if sub_ts > 0 {
                                self.subagent_start_seconds = Some(sub_ts);
                            }
                        }
                    } else if self.is_subagent {
                        self.skipping_fork_context = true;
                    }
                }
            }
            "turn_context" => {
                if let Some(payload) = payload {
                    if let Some(model) = payload.get("model").and_then(Value::as_str) {
                        if !model.trim().is_empty() {
                            self.current_model = Some(model.to_string());
                        }
                    }
                }
            }
            "event_msg" => {
                if let Some(payload) = payload {
                    let event_type = payload.get("type").and_then(Value::as_str).unwrap_or("");
                    if self.skipping_fork_context && event_type == "task_started" {
                        let started_at = payload.get("started_at").and_then(Value::as_i64);
                        if let Some(sub_ts) = self.subagent_start_seconds {
                            if started_at.map_or(true, |s| s >= sub_ts) {
                                self.skipping_fork_context = false;
                            }
                        } else {
                            self.skipping_fork_context = false;
                        }
                    }

                    if !self.skipping_fork_context {
                        match event_type {
                            "agent_reasoning" => {
                                let text = payload
                                    .get("text")
                                    .and_then(Value::as_str)
                                    .unwrap_or("")
                                    .replace("<!-- -->", "");
                                let text = text.trim();
                                if !text.is_empty() {
                                    if let Some(last) = messages.last_mut() {
                                        if last.role == "assistant" {
                                            if let Some(ContentPart::Thinking { thinking }) =
                                                last.content_parts.last_mut()
                                            {
                                                thinking.push_str("\n\n");
                                                thinking.push_str(text);
                                                return;
                                            }
                                        }
                                    }
                                    messages.push(ChatMessage {
                                        role: "assistant".to_string(),
                                        timestamp,
                                        model: self.current_model.clone(),
                                        token_usage: None,
                                        content_parts: vec![ContentPart::Thinking {
                                            thinking: text.to_string(),
                                        }],
                                        is_meta: false,
                                        uuid: None,
                                    });
                                }
                            }
                            "sub_agent_activity" => {
                                let call_id = payload.get("event_id").and_then(Value::as_str);
                                let agent_thread_id =
                                    payload.get("agent_thread_id").and_then(Value::as_str);
                                let agent_path =
                                    payload.get("agent_path").and_then(Value::as_str);
                                if let (Some(call_id), Some(thread_id)) =
                                    (call_id, agent_thread_id)
                                {
                                    self.register_subagent(
                                        call_id,
                                        thread_id,
                                        agent_path,
                                        session_file_path,
                                    );
                                }
                            }
                            "collab_agent_spawn_end" => {
                                let call_id = payload.get("call_id").and_then(Value::as_str);
                                let agent_thread_id =
                                    payload.get("agent_thread_id").and_then(Value::as_str);
                                let agent_nickname =
                                    payload.get("agent_nickname").and_then(Value::as_str);
                                if let (Some(call_id), Some(thread_id)) =
                                    (call_id, agent_thread_id)
                                {
                                    self.register_subagent(
                                        call_id,
                                        thread_id,
                                        agent_nickname,
                                        session_file_path,
                                    );
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
            "compacted" => {
                if !self.skipping_fork_context {
                    if let Some(payload) = payload {
                        let message = payload
                            .get("message")
                            .and_then(Value::as_str)
                            .filter(|s| !s.is_empty());
                        let text = match message {
                            Some(msg) => format!("[context_compacted]\n{}", msg),
                            None => "[context_compacted]".to_string(),
                        };
                        messages.push(ChatMessage {
                            role: "system".to_string(),
                            timestamp,
                            model: None,
                            token_usage: None,
                            content_parts: vec![ContentPart::Text { text }],
                            is_meta: true,
                            uuid: None,
                        });
                    }
                }
            }
            "response_item" => {
                if let Some(payload) = payload {
                    let item_type = payload.get("type").and_then(Value::as_str).unwrap_or("");
                    if self.skipping_fork_context && item_type == "function_call_output" {
                        let output = codex_tool_result_content(payload);
                        if output.contains("newly spawned agent") {
                            self.skipping_fork_context = false;
                        }
                    }

                    if !self.skipping_fork_context {
                        match item_type {
                            "message" => {
                                let role =
                                    payload.get("role").and_then(Value::as_str).unwrap_or("");
                                if (role == "user" || role == "assistant") && role != "developer" {
                                    let content_parts =
                                        parse_codex_content(payload.get("content"));
                                    if !content_parts.is_empty() {
                                        let model = if role == "assistant" {
                                            self.current_model.clone()
                                        } else {
                                            None
                                        };
                                        messages.push(ChatMessage {
                                            role: role.to_string(),
                                            timestamp,
                                            model,
                                            token_usage: None,
                                            content_parts,
                                            is_meta: false,
                                            uuid: None,
                                        });
                                    }
                                }
                            }
                            "function_call" | "tool_call" | "custom_tool_call" => {
                                let call_id = payload
                                    .get("id")
                                    .or_else(|| payload.get("call_id"))
                                    .and_then(Value::as_str);
                                if let Some(cid) = call_id {
                                    let input_val = codex_tool_input_value(payload);
                                    let desc = input_val
                                        .get("task_name")
                                        .or_else(|| input_val.get("description"))
                                        .or_else(|| input_val.get("agent_type"))
                                        .or_else(|| input_val.get("prompt"))
                                        .and_then(Value::as_str)
                                        .map(str::trim)
                                        .filter(|s| !s.is_empty())
                                        .map(|s| s.chars().take(24).collect::<String>());
                                    if let Some(d) = desc {
                                        self.subagent_call_names.insert(cid.to_string(), d);
                                    }
                                }
                                if let Some(mut msg) =
                                    parse_codex_tool_use_message(payload, timestamp)
                                {
                                    msg.model = self.current_model.clone();
                                    messages.push(msg);
                                }
                            }
                            "function_call_output"
                            | "tool_result"
                            | "custom_tool_call_output"
                            | "tool_output" => {
                                let call_id = payload
                                    .get("call_id")
                                    .or_else(|| payload.get("id"))
                                    .and_then(Value::as_str);
                                if let Some(cid) = call_id {
                                    let content = codex_tool_result_content(payload);
                                    if let Ok(v) = serde_json::from_str::<Value>(&content) {
                                        if let Some(thread_id) = v
                                            .get("agent_thread_id")
                                            .or_else(|| v.get("agentId"))
                                            .or_else(|| v.get("session_id"))
                                            .and_then(Value::as_str)
                                        {
                                            self.register_subagent(
                                                cid,
                                                thread_id,
                                                None,
                                                session_file_path,
                                            );
                                        }
                                    }
                                }
                                if let Some(msg) =
                                    parse_codex_tool_result_message(payload, timestamp)
                                {
                                    messages.push(msg);
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn register_subagent(
        &mut self,
        call_id: &str,
        thread_id: &str,
        name_hint: Option<&str>,
        session_file_path: Option<&Path>,
    ) {
        if self.subagent_map.contains_key(call_id) {
            return;
        }
        let subagent_file = find_codex_subagent_file(session_file_path, thread_id);
        let Some(file_path) = subagent_file else {
            return;
        };

        let label = name_hint
            .filter(|n| !n.trim().is_empty())
            .map(|n| n.trim().to_string())
            .or_else(|| self.subagent_call_names.get(call_id).cloned())
            .unwrap_or_else(|| {
                if thread_id.len() >= 6 {
                    format!("Subagent {}", &thread_id[..6])
                } else {
                    format!("Subagent {}", thread_id)
                }
            });

        self.subagent_map.insert(
            call_id.to_string(),
            session::SubagentInfo {
                file_path: file_path.to_string_lossy().to_string(),
                label,
            },
        );
    }
}

/// Codex 归档内容解析：与 parse_codex_session_file 相同的行门与行解析。
pub(crate) fn parse_codex_session_from_string(content: &str) -> Vec<session::ChatMessage> {
    let mut parser = CodexParser::new();
    let mut messages = Vec::new();
    for line in content.lines() {
        parser.process_line(line, None, &mut messages);
    }
    messages
}

pub(crate) fn parse_codex_session_file(
    file_path: &str,
) -> AppResult<session::SessionLoadResult> {
    let file = File::open(file_path)?;
    let file_size = file.metadata()?.len();
    let mut reader = BufReader::new(file);
    let mut messages = Vec::new();
    let mut parser = CodexParser::new();
    let path = Path::new(file_path);
    let mut buf = String::new();

    loop {
        buf.clear();
        match reader.read_line(&mut buf) {
            Ok(0) => break,
            Err(_) => continue,
            Ok(_) => {}
        }
        parser.process_line(&buf, Some(path), &mut messages);
    }

    Ok(session::SessionLoadResult {
        messages,
        offset: file_size,
        subagent_map: parser.subagent_map,
    })
}

pub(crate) fn parse_codex_session_file_streaming<F>(
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
    let mut parser = CodexParser::new();
    let path = Path::new(file_path);
    let mut emitter = BatchEmitter::new();

    loop {
        buf.clear();
        match reader.read_line(&mut buf) {
            Ok(0) => break,
            Err(_) => continue,
            Ok(_) => {}
        }
        parser.process_line(&buf, Some(path), emitter.buffer_mut());
        if !emitter.maybe_flush(&mut on_batch) {
            return Ok((file_size, parser.subagent_map));
        }
    }

    emitter.flush_remaining(&mut on_batch);

    Ok((file_size, parser.subagent_map))
}

pub(crate) fn parse_codex_session_incremental(
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
    let mut parser = CodexParser::new();
    let path = Path::new(file_path);
    let mut buf = String::new();

    loop {
        buf.clear();
        match reader.read_line(&mut buf) {
            Ok(0) => break,
            Err(_) => continue,
            Ok(_) => {}
        }
        parser.process_line(&buf, Some(path), &mut messages);
    }

    Ok(session::SessionLoadResult {
        messages,
        offset: file_size,
        subagent_map: parser.subagent_map,
    })
}

pub(crate) fn parse_codex_content(content: Option<&Value>) -> Vec<ContentPart> {
    let Some(content) = content else {
        return Vec::new();
    };

    if let Some(text) = content.as_str() {
        let trimmed = text.trim();
        return if trimmed.is_empty() {
            Vec::new()
        } else {
            vec![ContentPart::Text {
                text: trimmed.to_string(),
            }]
        };
    }

    let Some(items) = content.as_array() else {
        return Vec::new();
    };

    let mut parts = Vec::new();
    for item in items {
        let item_type = item.get("type").and_then(|v| v.as_str()).unwrap_or("");
        if matches!(item_type, "input_text" | "output_text" | "text") {
            if let Some(text) = item.get("text").and_then(|v| v.as_str()) {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    parts.push(ContentPart::Text {
                        text: trimmed.to_string(),
                    });
                }
            }
        }
    }

    parts
}

pub(crate) fn parse_codex_tool_use_message(payload: &Value, timestamp: String) -> Option<ChatMessage> {
    let tool_name = codex_tool_name(payload)?;
    let input_value = codex_tool_input_value(payload);
    let summary = session::tool_use_summary(&tool_name, &input_value);
    let input = serde_json::to_string_pretty(&input_value).unwrap_or_default();

    Some(ChatMessage {
        role: "assistant".to_string(),
        timestamp,
        model: None,
        token_usage: None,
        content_parts: vec![ContentPart::ToolUse {
            summary,
            tool_name,
            input,
            tool_use_id: payload
                .get("id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
        }],
        is_meta: false,
        uuid: None,
    })
}

pub(crate) fn parse_codex_tool_result_message(payload: &Value, timestamp: String) -> Option<ChatMessage> {
    let content = codex_tool_result_content(payload);
    if content.trim().is_empty() {
        return None;
    }
    let is_error = codex_tool_result_is_error(payload, &content);
    let tool_name = codex_tool_name(payload).unwrap_or_else(|| "Tool".to_string());
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
        content_parts: vec![ContentPart::tool_result(summary, content, is_error, false)],
        is_meta: false,
        uuid: None,
    })
}

pub(crate) fn codex_tool_name(payload: &Value) -> Option<String> {
    payload
        .get("name")
        .and_then(|v| v.as_str())
        .or_else(|| payload.get("tool_name").and_then(|v| v.as_str()))
        .or_else(|| {
            payload
                .get("call")
                .and_then(|call| call.get("name"))
                .and_then(|v| v.as_str())
        })
        .map(|name| name.to_string())
}

pub(crate) fn codex_tool_result_content(payload: &Value) -> String {
    for key in ["output", "content", "result", "error"] {
        if let Some(value) = payload.get(key) {
            let text = codex_value_to_text(value);
            if !text.trim().is_empty() {
                return text;
            }
        }
    }
    String::new()
}

pub(crate) fn read_codex_first_user_message(file_path: &str) -> Option<String> {
    let file = File::open(file_path).ok()?;
    let reader = BufReader::new(file);
    let mut parser = CodexParser::new();
    let mut messages = Vec::new();
    let path = Path::new(file_path);

    for line in reader.lines().take(150) {
        let Ok(line) = line else { continue };
        parser.process_line(&line, Some(path), &mut messages);
        for msg in messages.iter().filter(|m| m.role == "user") {
            for part in &msg.content_parts {
                if let ContentPart::Text { text } = part {
                    if let Some(cleaned) = crate::title_resolver::clean_fallback_user_text(text) {
                        let truncated: String = cleaned.chars().take(30).collect();
                        return Some(if cleaned.chars().count() > 30 {
                            format!("{}...", truncated)
                        } else {
                            truncated
                        });
                    }
                }
            }
        }
    }

    None
}

pub(crate) fn extract_codex_role_text(entry: &Value) -> Option<(String, String)> {
    if entry.get("type").and_then(|v| v.as_str()) != Some("response_item") {
        return None;
    }

    let payload = entry.get("payload")?;
    if payload.get("type").and_then(|v| v.as_str()) != Some("message") {
        return None;
    }

    let role = payload.get("role").and_then(|v| v.as_str())?;
    if role != "user" && role != "assistant" {
        return None;
    }

    let text = extract_codex_text(payload.get("content")?)?;
    Some((role.to_string(), text))
}

pub(crate) fn extract_codex_text(content: &Value) -> Option<String> {
    if let Some(text) = content.as_str() {
        let trimmed = text.trim();
        return if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        };
    }

    let items = content.as_array()?;
    let mut texts = Vec::new();
    for item in items {
        let item_type = item.get("type").and_then(|v| v.as_str()).unwrap_or("");
        if matches!(item_type, "input_text" | "output_text" | "text") {
            if let Some(text) = item.get("text").and_then(|v| v.as_str()) {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    texts.push(trimmed.to_string());
                }
            }
        }
    }

    if texts.is_empty() {
        None
    } else {
        Some(texts.join("\n"))
    }
}

pub(crate) fn read_codex_session_meta_field<T, F>(file_path: &str, extractor: F) -> Option<T>
where
    F: Fn(&Value) -> Option<T>,
{
    let file = File::open(file_path).ok()?;
    let reader = BufReader::new(file);

    for line in reader.lines().take(40) {
        let line = match line {
            Ok(line) => line,
            Err(_) => continue,
        };
        let entry: Value = serde_json::from_str(line.trim()).ok()?;
        if entry.get("type").and_then(|v| v.as_str()) != Some("session_meta") {
            continue;
        }
        let payload = entry.get("payload")?;
        if let Some(value) = extractor(payload) {
            return Some(value);
        }
    }

    None
}

pub(crate) fn read_codex_turn_context_cwd(file_path: &str) -> Option<String> {
    let file = File::open(file_path).ok()?;
    let reader = BufReader::new(file);

    for line in reader.lines().take(80) {
        let line = match line {
            Ok(line) => line,
            Err(_) => continue,
        };
        let entry: Value = serde_json::from_str(line.trim()).ok()?;
        if entry.get("type").and_then(|v| v.as_str()) != Some("turn_context") {
            continue;
        }
        let payload = entry.get("payload")?;
        if let Some(cwd) = payload.get("cwd").and_then(|v| v.as_str()) {
            if !cwd.trim().is_empty() {
                return Some(cwd.to_string());
            }
        }
    }

    None
}

/// 提取 Codex 会话的用量记录（会话统计/分析用）：逐行扫描 `event_msg/token_count`
/// 事件，取 `info.last_token_usage`（本轮增量）映射为 `UsageRecord`：
/// input = input_tokens - min(cached, input)，cached → cache_read，cache_creation = 0
/// 会话 fork 用量映射：fork 会话会把父会话 usage 在首个 token_count 的同一秒重放，
/// 批量重放——这些属于父会话，跳过（重放段以"秒变化"结束）。
pub(crate) fn extract_usage_records(
    file_path: &str,
    project: &str,
) -> Vec<session::UsageRecord> {
    let file = match File::open(file_path) {
        Ok(file) => file,
        Err(_) => return Vec::new(),
    };
    // 与主解析路径一致的缓冲（256 KB），避免逐行 syscall
    let reader = BufReader::with_capacity(256 * 1024, file);
    let mut records = Vec::new();
    let mut current_model: Option<String> = None;
    let mut replay_skip = false;
    let mut replay_second: Option<String> = None;

    for line in reader.lines() {
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
        let entry_type = entry.get("type").and_then(Value::as_str).unwrap_or("");
        let payload = entry.get("payload").unwrap_or(&Value::Null);

        match entry_type {
            // fork 会话标记：开启重放跳过
            "session_meta" => {
                if payload.get("forked_from_id").and_then(Value::as_str).is_some() {
                    replay_skip = true;
                }
            }
            "turn_context" => {
                if let Some(model) = payload.get("model").and_then(Value::as_str) {
                    if !model.trim().is_empty() {
                        current_model = Some(model.to_string());
                    }
                }
            }
            "event_msg"
                if payload.get("type").and_then(Value::as_str) == Some("token_count") =>
            {
                let timestamp = entry
                    .get("timestamp")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                if replay_skip {
                    let second = timestamp.get(..19).unwrap_or(timestamp).to_string();
                    match replay_second.as_deref() {
                        None => replay_second = Some(second),
                        Some(replay) if replay == second => {}
                        Some(_) => replay_skip = false, // 秒变化：重放段结束
                    }
                    if replay_skip {
                        continue; // 重放的父会话用量，不计入
                    }
                }
                let Some(info) = payload.get("info") else {
                    continue;
                };
                let Some(usage) = info.get("last_token_usage") else {
                    continue;
                };
                let input = usage.get("input_tokens").and_then(Value::as_u64).unwrap_or(0);
                let cached = usage
                    .get("cached_input_tokens")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                let output = usage.get("output_tokens").and_then(Value::as_u64).unwrap_or(0);
                if input == 0 && cached == 0 && output == 0 {
                    continue;
                }
                let cache_read = cached.min(input);
                records.push(session::UsageRecord {
                    date: crate::parser::claude::timestamp_to_local_date(timestamp),
                    model: current_model.clone().unwrap_or_else(|| "unknown".to_string()),
                    input_tokens: input - cache_read,
                    output_tokens: output,
                    cache_creation_tokens: 0,
                    cache_read_tokens: cache_read,
                    duration_ms: None,
                    project: project.to_string(),
                });
            }
            _ => {}
        }
    }
    records
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_usage_records_maps_token_count_events() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rollout.jsonl");
        std::fs::write(
            &path,
            concat!(
                "{\"timestamp\":\"2026-08-17T07:20:00Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"s1\",\"cwd\":\"/proj/a\"}}\n",
                "{\"timestamp\":\"2026-08-17T07:20:01Z\",\"type\":\"turn_context\",\"payload\":{\"model\":\"gpt-5.6\"}}\n",
                // 无 token_count 的事件不产生记录
                "{\"timestamp\":\"2026-08-17T07:20:02Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[]}}\n",
                "{\"timestamp\":\"2026-08-17T07:21:00Z\",\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\",\"info\":{\"last_token_usage\":{\"input_tokens\":16951,\"cached_input_tokens\":16948,\"output_tokens\":226,\"reasoning_output_tokens\":0,\"total_tokens\":17177}}}}\n",
                // 全零 usage 跳过
                "{\"timestamp\":\"2026-08-17T07:22:00Z\",\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\",\"info\":{\"last_token_usage\":{\"input_tokens\":0,\"cached_input_tokens\":0,\"output_tokens\":0,\"reasoning_output_tokens\":0,\"total_tokens\":0}}}}\n",
                "{\"timestamp\":\"2026-08-17T07:23:00Z\",\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\",\"info\":{\"last_token_usage\":{\"input_tokens\":100,\"cached_input_tokens\":0,\"output_tokens\":10,\"reasoning_output_tokens\":5,\"total_tokens\":110}}}}\n",
            ),
        )
        .unwrap();

        let records = extract_usage_records(path.to_str().unwrap(), "/proj/a");
        assert_eq!(records.len(), 2, "全零 usage 与无 token_count 的行不产生记录");
        let first = &records[0];
        assert_eq!(first.model, "gpt-5.6");
        // input 扣掉 cached：16951 - 16948 = 3；cached 全记 cache_read
        assert_eq!(first.input_tokens, 3);
        assert_eq!(first.cache_read_tokens, 16948);
        assert_eq!(first.cache_creation_tokens, 0);
        assert_eq!(first.output_tokens, 226);
        assert_eq!(first.date, "2026-08-17");
        assert!(records.iter().all(|r| r.project == "/proj/a"));
    }

    #[test]
    fn extract_usage_records_skips_fork_replay_burst() {
        // fork 会话：父会话用量在首个 token_count 的同一秒批量重放，属父会话不计；
        // 过滤父会话重放带来的历史用量，秒变化后的才是本会话自身产生的用量。
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rollout-fork.jsonl");
        std::fs::write(
            &path,
            concat!(
                "{\"timestamp\":\"2026-04-10T10:00:00Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"sess-4\",\"cwd\":\"/tmp\",\"forked_from_id\":\"sess-0\"}}\n",
                "{\"timestamp\":\"2026-04-10T10:00:00Z\",\"type\":\"turn_context\",\"payload\":{\"model\":\"gpt-5.6\"}}\n",
                "{\"timestamp\":\"2026-04-10T10:00:00Z\",\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\",\"info\":{\"last_token_usage\":{\"input_tokens\":1000,\"cached_input_tokens\":0,\"output_tokens\":100,\"reasoning_output_tokens\":0,\"total_tokens\":1100}}}}\n",
                "{\"timestamp\":\"2026-04-10T10:00:00Z\",\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\",\"info\":{\"last_token_usage\":{\"input_tokens\":2000,\"cached_input_tokens\":0,\"output_tokens\":200,\"reasoning_output_tokens\":0,\"total_tokens\":2200}}}}\n",
                "{\"timestamp\":\"2026-04-10T10:05:00Z\",\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\",\"info\":{\"last_token_usage\":{\"input_tokens\":50,\"cached_input_tokens\":0,\"output_tokens\":5,\"reasoning_output_tokens\":0,\"total_tokens\":55}}}}\n",
            ),
        )
        .unwrap();

        let records = extract_usage_records(path.to_str().unwrap(), "/tmp");
        assert_eq!(records.len(), 1, "重放段（同秒）跳过，只剩秒变化后的记录");
        assert_eq!(records[0].input_tokens, 50);
        assert_eq!(records[0].output_tokens, 5);
    }

    #[test]
    fn scan_codex_metadata_only_rejects_subagent_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rollout-subagent.jsonl");
        std::fs::write(
            &path,
            concat!(
                "{\"timestamp\":\"2026-04-21T03:21:21.714Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"codex-sub-001\",\"cwd\":\"/tmp\",\"source\":{\"subagent\":{\"thread_spawn\":{\"parent_thread_id\":\"codex-parent-001\",\"depth\":1,\"agent_nickname\":\"Faraday\"}}}}}\n",
                "{\"timestamp\":\"2026-04-21T03:21:21.715Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"Subagent task\"}]}}\n",
            ),
        )
        .unwrap();

        assert!(is_codex_subagent_file(path.to_str().unwrap()));
        let meta = scan_codex_metadata_only(&path);
        assert!(meta.is_none(), "Subagent session must not be scanned into root session list");
    }

    #[test]
    fn scan_codex_metadata_only_accepts_normal_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rollout-main.jsonl");
        std::fs::write(
            &path,
            concat!(
                "{\"timestamp\":\"2026-04-21T03:21:21.714Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"codex-main-001\",\"cwd\":\"/tmp/my-project\",\"source\":\"cli\"}}\n",
                "{\"timestamp\":\"2026-04-21T03:21:22.000Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"Hello world\"}]}}\n",
            ),
        )
        .unwrap();

        assert!(!is_codex_subagent_file(path.to_str().unwrap()));
        let meta = scan_codex_metadata_only(&path).expect("normal session must be scanned");
        assert_eq!(meta.session_id, "codex-main-001");
        assert_eq!(meta.project_path, Some("/tmp/my-project".to_string()));
        assert_eq!(meta.first_user_message, Some("Hello world".to_string()));
    }

    #[test]
    fn parse_codex_subagent_skips_fork_context() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rollout-subagent-v2.jsonl");
        std::fs::write(
            &path,
            concat!(
                "{\"timestamp\":\"2026-04-21T03:21:21.714Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"codex-sub-v2-001\",\"forked_from_id\":\"codex-parent-v2-001\",\"timestamp\":\"2026-04-21T03:21:21.714Z\",\"cwd\":\"/home/user/demo\",\"source\":{\"subagent\":{\"thread_spawn\":{\"parent_thread_id\":\"codex-parent-v2-001\",\"depth\":1,\"agent_nickname\":\"Hume\"}}},\"agent_nickname\":\"Hume\"}}\n",
                "{\"timestamp\":\"2026-04-21T03:21:21.715Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"codex-parent-v2-001\",\"timestamp\":\"2026-04-21T02:37:09.779Z\",\"cwd\":\"/home/user/demo\",\"source\":\"cli\"}}\n",
                "{\"timestamp\":\"2026-04-21T03:21:21.716Z\",\"type\":\"event_msg\",\"payload\":{\"type\":\"task_started\",\"turn_id\":\"parent-turn-001\",\"started_at\":1776739093}}\n",
                "{\"timestamp\":\"2026-04-21T03:21:21.717Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"Parent's old message that must be skipped\"}]}}\n",
                "{\"timestamp\":\"2026-04-21T03:21:21.718Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"Parent's old reply that must be skipped\"}]}}\n",
                "{\"timestamp\":\"2026-04-21T03:21:21.755Z\",\"type\":\"event_msg\",\"payload\":{\"type\":\"task_started\",\"turn_id\":\"sub-turn-001\",\"started_at\":1776741681}}\n",
                "{\"timestamp\":\"2026-04-21T03:21:21.756Z\",\"type\":\"turn_context\",\"payload\":{\"turn_id\":\"sub-turn-001\",\"model\":\"gpt-5.4-mini\"}}\n",
                "{\"timestamp\":\"2026-04-21T03:21:21.757Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"Investigate module A and return a summary.\"}]}}\n",
                "{\"timestamp\":\"2026-04-21T03:21:30.000Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"Module A handles authentication via OAuth.\"}]}}\n",
            ),
        )
        .unwrap();

        let result = parse_codex_session_file(path.to_str().unwrap()).expect("parse subagent session");
        assert_eq!(result.messages.len(), 2, "Parent messages must be skipped, only 2 subagent messages survive");
        assert_eq!(result.messages[0].role, "user");
        match &result.messages[0].content_parts[0] {
            ContentPart::Text { text } => assert_eq!(text, "Investigate module A and return a summary."),
            _ => panic!("Expected Text part"),
        }
        assert_eq!(result.messages[1].role, "assistant");
        assert_eq!(result.messages[1].model.as_deref(), Some("gpt-5.4-mini"));
        match &result.messages[1].content_parts[0] {
            ContentPart::Text { text } => assert_eq!(text, "Module A handles authentication via OAuth."),
            _ => panic!("Expected Text part"),
        }
    }

    #[test]
    fn parse_codex_parent_session_builds_subagent_map() {
        let dir = tempfile::tempdir().unwrap();
        let subagent_id = "019f5ba3-c290-79d2-9a89-370b905ec017";
        let subagent_path = dir.path().join(format!("rollout-2026-04-21T03-21-21-{}.jsonl", subagent_id));
        std::fs::write(
            &subagent_path,
            format!(
                "{{\"timestamp\":\"2026-04-21T03:21:21.714Z\",\"type\":\"session_meta\",\"payload\":{{\"id\":\"{}\",\"source\":{{\"subagent\":{{\"thread_spawn\":{{\"parent_thread_id\":\"parent-01\",\"depth\":1,\"agent_nickname\":\"AuditAgent\"}}}}}}}}}}\n",
                subagent_id
            ),
        )
        .unwrap();

        let parent_path = dir.path().join("rollout-2026-04-21T03-00-00-parent.jsonl");
        std::fs::write(
            &parent_path,
            format!(
                concat!(
                    "{{\"timestamp\":\"2026-04-21T03:00:00.000Z\",\"type\":\"session_meta\",\"payload\":{{\"id\":\"parent-01\",\"cwd\":\"/tmp\"}}}}\n",
                    "{{\"timestamp\":\"2026-04-21T03:00:01.000Z\",\"type\":\"response_item\",\"payload\":{{\"type\":\"function_call\",\"call_id\":\"call_spawn_1\",\"name\":\"spawn_agent\",\"arguments\":\"{{\\\"task_name\\\":\\\"audit code\\\"}}\"}}}}\n",
                    "{{\"timestamp\":\"2026-04-21T03:00:02.000Z\",\"type\":\"event_msg\",\"payload\":{{\"type\":\"sub_agent_activity\",\"event_id\":\"call_spawn_1\",\"agent_thread_id\":\"{}\",\"agent_path\":\"AuditAgent\",\"kind\":\"started\"}}}}\n",
                ),
                subagent_id
            ),
        )
        .unwrap();

        let result = parse_codex_session_file(parent_path.to_str().unwrap()).expect("parse parent session");
        assert!(result.subagent_map.contains_key("call_spawn_1"), "subagent_map must contain call_spawn_1");
        let info = &result.subagent_map["call_spawn_1"];
        assert_eq!(info.file_path, subagent_path.to_str().unwrap());
        assert_eq!(info.label, "AuditAgent");
    }

    #[test]
    fn parse_codex_merges_consecutive_thinking_blocks_and_handles_compacted() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rollout-thinking.jsonl");
        std::fs::write(
            &path,
            concat!(
                "{\"timestamp\":\"2026-04-21T03:00:00Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"s1\",\"cwd\":\"/tmp\"}}\n",
                "{\"timestamp\":\"2026-04-21T03:00:01Z\",\"type\":\"event_msg\",\"payload\":{\"type\":\"agent_reasoning\",\"text\":\"Thinking part 1<!-- -->\"}}\n",
                "{\"timestamp\":\"2026-04-21T03:00:02Z\",\"type\":\"event_msg\",\"payload\":{\"type\":\"agent_reasoning\",\"text\":\"Thinking part 2\"}}\n",
                "{\"timestamp\":\"2026-04-21T03:00:03Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"Final Answer\"}]}}\n",
                "{\"timestamp\":\"2026-04-21T03:00:04Z\",\"type\":\"compacted\",\"payload\":{\"message\":\"Summarized previous conversation.\"}}\n",
            ),
        )
        .unwrap();

        let result = parse_codex_session_file(path.to_str().unwrap()).expect("parse session");
        assert_eq!(result.messages.len(), 3);
        // Message 0: Thinking merged
        assert_eq!(result.messages[0].role, "assistant");
        match &result.messages[0].content_parts[0] {
            ContentPart::Thinking { thinking } => assert_eq!(thinking, "Thinking part 1\n\nThinking part 2"),
            _ => panic!("Expected Thinking part"),
        }
        // Message 1: Assistant text
        assert_eq!(result.messages[1].role, "assistant");
        match &result.messages[1].content_parts[0] {
            ContentPart::Text { text } => assert_eq!(text, "Final Answer"),
            _ => panic!("Expected Text part"),
        }
        // Message 2: Compacted system message
        assert_eq!(result.messages[2].role, "system");
        assert!(result.messages[2].is_meta);
        match &result.messages[2].content_parts[0] {
            ContentPart::Text { text } => assert!(text.contains("Summarized previous conversation.")),
            _ => panic!("Expected Text part"),
        }
    }
}
