//! Antigravity 会话解析器：负责将 Antigravity JSONL 事件流解析为统一 ChatMessage 模型。
//! 包含宽松 JSON 解析、子代理关联、图片多模态提取、工具调用映射与增量/流式加载。

use crate::error::AppResult;
use crate::parser::batch::BatchEmitter;
use crate::parser::claude_entry::TOOL_RESULT_MAX_LEN;
use crate::parser::shared::truncate_preview_text;
use crate::session::{self, ChatMessage, ContentPart, SessionListMetadata, SessionLoadResult, SubagentInfo, UsageRecord};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, VecDeque};
use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Step {
    #[serde(default)]
    pub step_index: u64,
    #[serde(default)]
    pub source: String,
    #[serde(rename = "type", default)]
    pub step_type: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub created_at: String,
    pub content: Option<String>,
    pub thinking: Option<String>,
    pub tool_calls: Option<Vec<ToolCall>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ToolCall {
    pub name: String,
    pub args: Option<Value>,
}

// ─── 宽松 JSON 解析与反序列化 ───

const MAX_DECODE_DEPTH: usize = 6;

/// Split free-form text into the top-level `{...}` JSON object substrings it contains.
pub(crate) fn extract_top_level_json_objects(input: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'{' {
            i += 1;
            continue;
        }
        let start = i;
        let mut depth = 0i32;
        let mut in_string = false;
        let mut escape = false;
        while i < bytes.len() {
            let b = bytes[i];
            if in_string {
                if escape {
                    escape = false;
                } else if b == b'\\' {
                    escape = true;
                } else if b == b'"' {
                    in_string = false;
                }
            } else {
                match b {
                    b'"' => in_string = true,
                    b'{' => depth += 1,
                    b'}' => {
                        depth -= 1;
                        if depth == 0 {
                            i += 1;
                            if let Ok(slice) = std::str::from_utf8(&bytes[start..i]) {
                                out.push(slice.to_string());
                            }
                            break;
                        }
                    }
                    _ => {}
                }
            }
            i += 1;
        }
        if depth != 0 {
            break;
        }
    }
    out
}

/// Extract per-subagent Prompt strings from antigravity's `invoke_subagent` tool arguments.
pub(crate) fn invoke_subagent_prompts(subagents_value: Option<&Value>) -> Vec<String> {
    let Some(value) = subagents_value else {
        return Vec::new();
    };
    match value {
        Value::Array(arr) => arr
            .iter()
            .map(|sub| {
                sub.get("Prompt")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string()
            })
            .collect(),
        Value::String(raw) => extract_prompts_lenient(raw),
        _ => Vec::new(),
    }
}

fn extract_prompts_lenient(raw: &str) -> Vec<String> {
    const KEY: &str = "\"Prompt\"";
    let mut out = Vec::new();
    let bytes = raw.as_bytes();
    let mut cursor = 0usize;
    while let Some(rel) = raw[cursor..].find(KEY) {
        let key_end = cursor + rel + KEY.len();
        let mut i = key_end;
        while i < bytes.len() && (bytes[i] as char).is_whitespace() {
            i += 1;
        }
        if i >= bytes.len() || bytes[i] != b':' {
            cursor = key_end;
            continue;
        }
        i += 1;
        while i < bytes.len() && (bytes[i] as char).is_whitespace() {
            i += 1;
        }
        if i >= bytes.len() || bytes[i] != b'"' {
            cursor = key_end;
            continue;
        }
        let value_start = i + 1;
        let mut j = value_start;
        let mut escape = false;
        while j < bytes.len() {
            let b = bytes[j];
            if escape {
                escape = false;
                j += 1;
                continue;
            }
            match b {
                b'\\' => {
                    escape = true;
                    j += 1;
                }
                b'"' => break,
                _ => j += 1,
            }
        }
        if j >= bytes.len() {
            out.push(unescape_json_literals(&raw[value_start..]));
            break;
        }
        out.push(unescape_json_literals(&raw[value_start..j]));
        cursor = j + 1;
    }
    out
}

fn unescape_json_literals(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some('"') => out.push('"'),
            Some('\\') => out.push('\\'),
            Some('/') => out.push('/'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// Decode JSON-encoded strings inside Antigravity tool arguments.
pub(crate) fn decode_antigravity_value(value: &Value) -> Value {
    fn try_decode_string(raw: &str) -> Option<Value> {
        if let Ok(parsed) = serde_json::from_str::<Value>(raw) {
            return Some(parsed);
        }
        let trimmed = raw.trim_start();
        let first = trimmed.chars().next()?;
        if !matches!(first, '"' | '[' | '{') {
            return None;
        }
        let escaped = escape_control_chars_for_json(raw);
        if escaped != raw {
            if let Ok(parsed) = serde_json::from_str::<Value>(&escaped) {
                return Some(parsed);
            }
        }
        if first == '"' {
            return Some(Value::String(lenient_unwrap_json_string(raw)));
        }
        None
    }

    fn walk(value: &Value, depth: usize) -> Value {
        if depth >= MAX_DECODE_DEPTH {
            return value.clone();
        }
        match value {
            Value::String(raw) => match try_decode_string(raw) {
                Some(decoded) => walk(&decoded, depth + 1),
                None => value.clone(),
            },
            Value::Array(items) => {
                Value::Array(items.iter().map(|item| walk(item, depth + 1)).collect())
            }
            Value::Object(map) => {
                let mut next = serde_json::Map::with_capacity(map.len());
                for (key, val) in map {
                    next.insert(key.clone(), walk(val, depth + 1));
                }
                Value::Object(next)
            }
            _ => value.clone(),
        }
    }
    walk(value, 0)
}

fn escape_control_chars_for_json(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for ch in s.chars() {
        match ch {
            '\x08' => out.push_str("\\b"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\x0C' => out.push_str("\\f"),
            '\r' => out.push_str("\\r"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

fn lenient_unwrap_json_string(raw: &str) -> String {
    let mut inner = raw;
    if let Some(stripped) = inner.strip_prefix('"') {
        inner = stripped;
    }
    if inner.ends_with('"') && !inner.ends_with("\\\"") {
        inner = &inner[..inner.len() - 1];
    }

    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.peek() {
            Some(&'n') => {
                out.push('\n');
                chars.next();
            }
            Some(&'t') => {
                out.push('\t');
                chars.next();
            }
            Some(&'r') => {
                out.push('\r');
                chars.next();
            }
            Some(&'"') => {
                out.push('"');
                chars.next();
            }
            Some(&'\\') => {
                out.push('\\');
                chars.next();
            }
            Some(&'/') => {
                out.push('/');
                chars.next();
            }
            _ => out.push('\\'),
        }
    }
    out
}

// ─── 消息与元数据提取 ───

pub(crate) fn clean_user_content(content: &str) -> String {
    if let Some(start_idx) = content.find("<USER_REQUEST>") {
        if let Some(end_idx) = content.find("</USER_REQUEST>") {
            let start = start_idx + "<USER_REQUEST>".len();
            if end_idx > start {
                return content[start..end_idx].trim().to_string();
            }
        }
    }
    content.trim().to_string()
}

pub(crate) fn extract_uploaded_image_paths(content: &str) -> Vec<String> {
    let Some(start_idx) = content.find("<ADDITIONAL_METADATA>") else {
        return Vec::new();
    };
    let after_open = start_idx + "<ADDITIONAL_METADATA>".len();
    let body_end = content[after_open..]
        .find("</ADDITIONAL_METADATA>")
        .map(|off| after_open + off)
        .unwrap_or(content.len());
    let body = &content[after_open..body_end];

    let Some(header_idx) = body.find("The user has uploaded ") else {
        return Vec::new();
    };
    let after_header = match body[header_idx..].find('\n') {
        Some(off) => &body[header_idx + off + 1..],
        None => return Vec::new(),
    };
    let mut paths = Vec::new();
    for line in after_header.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            break;
        }
        let Some(rest) = trimmed.strip_prefix("- ") else {
            break;
        };
        let path = rest.trim();
        if !path.is_empty() {
            paths.push(path.to_string());
        }
    }
    paths
}

fn normalize_antigravity_model(model: &str) -> String {
    let lower = model.to_lowercase();
    lower
        .replace(" (high)", "")
        .replace(" (low)", "")
        .replace(" (medium)", "")
        .replace(" (balanced)", "")
        .replace(" flash", "-flash")
        .replace(" pro", "-pro")
        .replace(' ', "-")
}

pub(crate) fn extract_model_from_content(content: &str) -> Option<String> {
    let start_tag = "<USER_SETTINGS_CHANGE>";
    let end_tag = "</USER_SETTINGS_CHANGE>";
    let start_idx = content.find(start_tag)?;
    let end_idx = content.find(end_tag)?;
    if end_idx <= start_idx {
        return None;
    }
    let block = &content[start_idx + start_tag.len()..end_idx];

    let model_sel = "Model Selection";
    let pos = block.find(model_sel)?;
    let from_pos = block[pos..].find(" from ")?;
    let to_pos = block[pos + from_pos..].find(" to ")?;

    let model_start = pos + from_pos + to_pos + " to ".len();
    let rest = &block[model_start..];

    let mut chars = rest.chars().peekable();
    let mut model_len = 0;
    while let Some(c) = chars.next() {
        if c == '\n' || c == '`' {
            break;
        }
        if c == '.' {
            if let Some(&next_c) = chars.peek() {
                if next_c == ' ' || next_c == '\n' || next_c == '`' {
                    break;
                }
            } else {
                break;
            }
        }
        model_len += c.len_utf8();
    }
    let model_name = rest[..model_len].trim().to_string();
    if !model_name.is_empty() {
        Some(normalize_antigravity_model(&model_name))
    } else {
        None
    }
}

// ─── 子代理与工作区提取 ───

#[derive(Debug, Default, Clone)]
pub(crate) struct InvokeSubagentInfo {
    pub(crate) conversation_ids: Vec<String>,
    pub(crate) workspace: Option<String>,
}

#[derive(Debug, Deserialize)]
struct InvokeSubagentBlock {
    #[serde(alias = "conversationId", alias = "conversation_id")]
    conversation_id: Option<String>,
    #[serde(alias = "workspaceUris", alias = "workspace_uris", default)]
    workspace_uris: Vec<String>,
}

#[derive(Debug, Default, Clone)]
pub(crate) struct ManageSubagentsInfo {
    pub(crate) conversation_ids: Vec<String>,
    pub(crate) prompts: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ManageSubagentBlock {
    spec: Option<ManageSubagentSpec>,
    result: Option<ManageSubagentResult>,
}

#[derive(Debug, Deserialize)]
struct ManageSubagentSpec {
    #[serde(rename = "initialPrompt")]
    initial_prompt: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ManageSubagentResult {
    #[serde(rename = "conversationId")]
    conversation_id: Option<String>,
}

pub(crate) fn parse_invoke_subagent_content(content: &str) -> InvokeSubagentInfo {
    let mut info = InvokeSubagentInfo::default();
    for block in extract_top_level_json_objects(content) {
        let parsed: InvokeSubagentBlock = match serde_json::from_str(&block) {
            Ok(b) => b,
            Err(_) => continue,
        };
        if let Some(id) = parsed
            .conversation_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            if !info.conversation_ids.iter().any(|existing| existing == id) {
                info.conversation_ids.push(id.to_string());
            }
        }
        if info.workspace.is_none() {
            for uri in &parsed.workspace_uris {
                if let Some(path) = uri.strip_prefix("file://") {
                    let path = path.trim();
                    if !path.is_empty() {
                        info.workspace = Some(path.to_string());
                        break;
                    }
                }
            }
        }
    }
    info
}

pub(crate) fn parse_manage_subagents_content(content: &str) -> ManageSubagentsInfo {
    let mut info = ManageSubagentsInfo::default();
    for block in extract_top_level_json_objects(content) {
        let parsed: ManageSubagentBlock = match serde_json::from_str(&block) {
            Ok(b) => b,
            Err(_) => continue,
        };
        let Some(id) = parsed
            .result
            .as_ref()
            .and_then(|result| result.conversation_id.as_deref())
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        if info.conversation_ids.iter().any(|existing| existing == id) {
            continue;
        }
        let prompt = parsed
            .spec
            .as_ref()
            .and_then(|spec| spec.initial_prompt.as_deref())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("")
            .to_string();
        info.conversation_ids.push(id.to_string());
        info.prompts.push(prompt);
    }
    info
}

pub(crate) fn recipient_from_send_message(tool_call: &ToolCall) -> Option<String> {
    if tool_call.name != "send_message" {
        return None;
    }
    let raw = tool_call.args.as_ref()?.get("Recipient")?;
    match decode_antigravity_value(raw) {
        Value::String(decoded) => {
            let trimmed = decoded.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        _ => None,
    }
}

pub(crate) fn extract_absolute_paths_from_value(val: &Value, paths: &mut Vec<String>) {
    match val {
        Value::String(s) => {
            let trimmed = s.trim_matches('"').trim_matches('\'');
            if !trimmed.is_empty() && Path::new(trimmed).is_absolute() {
                paths.push(trimmed.to_string());
            }
        }
        Value::Array(arr) => {
            for item in arr {
                extract_absolute_paths_from_value(item, paths);
            }
        }
        Value::Object(obj) => {
            for (_, item) in obj {
                extract_absolute_paths_from_value(item, paths);
            }
        }
        _ => {}
    }
}

/// history.jsonl 的缓存快照：一次解析同时产出 conversationId→workspace 映射与
/// display→workspace 列表。扫描会话时会对每个 transcript 调用多次 history 查询，
/// 这里按 (路径, mtime) 缓存，避免 O(transcript数 × history行数) 的重复解析；
/// history 变更（mtime 变化）或数据目录覆盖变化时自动失效重读。
struct HistorySnapshot {
    workspaces: HashMap<String, String>,
    display_entries: Vec<(String, String)>,
}

static HISTORY_CACHE: LazyLock<
    Mutex<Option<(PathBuf, Option<std::time::SystemTime>, Arc<HistorySnapshot>)>>,
> = LazyLock::new(|| Mutex::new(None));

fn history_snapshot() -> Arc<HistorySnapshot> {
    // 与扫描端走同一 data_dir（尊重用户配置的 Antigravity 数据目录覆盖），
    // 否则覆盖配置下 history 读默认目录、transcript 扫覆盖目录，
    // contains_key 恒不命中，所有根会话都会被误判为子代理
    let path = crate::cli::history_path(crate::cli::CliKind::Antigravity)
        .unwrap_or_default();
    let mtime = std::fs::metadata(&path).and_then(|m| m.modified()).ok();

    if let Ok(guard) = HISTORY_CACHE.lock() {
        if let Some((cached_path, cached_mtime, snapshot)) = guard.as_ref() {
            if *cached_path == path && *cached_mtime == mtime {
                return snapshot.clone();
            }
        }
    }

    let mut snapshot = HistorySnapshot {
        workspaces: HashMap::new(),
        display_entries: Vec::new(),
    };
    if let Ok(file) = File::open(&path) {
        let reader = BufReader::new(file);
        for line in reader.lines().flatten() {
            if let Ok(val) = serde_json::from_str::<Value>(&line) {
                if let (Some(cid), Some(ws)) = (
                    val.get("conversationId").and_then(|v| v.as_str()),
                    val.get("workspace").and_then(|v| v.as_str()),
                ) {
                    snapshot.workspaces.insert(cid.to_string(), ws.to_string());
                }
                if let (Some(display), Some(ws)) = (
                    val.get("display").and_then(|v| v.as_str()),
                    val.get("workspace").and_then(|v| v.as_str()),
                ) {
                    snapshot.display_entries.push((display.to_string(), ws.to_string()));
                }
            }
        }
    }

    let snapshot = Arc::new(snapshot);
    if let Ok(mut guard) = HISTORY_CACHE.lock() {
        *guard = Some((path, mtime, snapshot.clone()));
    }
    snapshot
}

pub(crate) fn find_workspace_by_display_content(first_user_msg: &str) -> Option<String> {
    let snapshot = history_snapshot();
    snapshot
        .display_entries
        .iter()
        .find(|(display, _)| display.trim() == first_user_msg.trim())
        .map(|(_, ws)| ws.clone())
}

pub(crate) fn extract_workspace_from_user_info(content: &str) -> Option<String> {
    for line in content.lines() {
        let line = line.trim();
        if let Some((left, _)) = line.split_once(" -> ") {
            let path = left.trim();
            if Path::new(path).is_absolute() {
                return Some(path.to_string());
            }
        }
    }
    None
}

pub(crate) fn extract_project_path_from_transcript(file_path: &str) -> Option<String> {
    let Ok(file) = File::open(file_path) else {
        return None;
    };
    let reader = BufReader::new(file);
    let mut candidate_tool_cwds = Vec::new();

    for line in reader.lines().flatten() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(step) = serde_json::from_str::<Step>(trimmed) else {
            continue;
        };

        if let Some(ref content) = step.content {
            if let Some(ws) = extract_workspace_from_user_info(content) {
                return Some(ws);
            }
        }

        if let Some(ref tool_calls) = step.tool_calls {
            for tc in tool_calls {
                if let Some(ref args) = tc.args {
                    let decoded = decode_antigravity_value(args);
                    if let Some(cwd) = decoded.get("Cwd").and_then(Value::as_str) {
                        let cwd_trimmed = cwd.trim();
                        if !cwd_trimmed.is_empty() && Path::new(cwd_trimmed).is_absolute() {
                            candidate_tool_cwds.push(cwd_trimmed.to_string());
                        }
                    }
                }
            }
        }
    }

    candidate_tool_cwds.into_iter().next()
}

pub(crate) fn extract_conversation_id(path: &Path) -> Option<String> {
    let mut current = path.parent();
    while let Some(p) = current {
        if let Some(parent) = p.parent() {
            if parent.file_name().and_then(|n| n.to_str()) == Some("brain") {
                return p.file_name().and_then(|n| n.to_str()).map(ToString::to_string);
            }
        }
        current = p.parent();
    }
    path.parent()
        .and_then(|p| p.parent())
        .and_then(|p| p.parent())
        .and_then(|p| p.file_name())
        .and_then(|s| s.to_str())
        .map(ToString::to_string)
}

// ─── 工具调用摘要与转换 ───

pub(crate) fn map_tool_name(raw_name: &str) -> &'static str {
    match raw_name {
        "run_command" | "exec" | "bash" | "shell" => "Bash",
        "view_file" | "read_file" | "read" | "view" => "Read",
        "write_to_file" | "write_file" | "write" => "Write",
        "replace_file_content" | "multi_replace_file_content" | "edit" | "edit_file" => "Edit",
        "grep_search" | "search_file_content" | "grep" => "Grep",
        "find_by_name" | "list_dir" | "list_directory" | "glob" | "ls" => "Glob",
        "invoke_subagent" | "define_subagent" | "manage_subagents" | "manage_task" => "Task",
        "send_message" => "SendMessage",
        "schedule" => "ScheduleWakeup",
        "ask_question" => "AskUserQuestion",
        "read_url_content" | "web_fetch" => "WebFetch",
        "search_web" | "web_search" => "WebSearch",
        _ => "Tool",
    }
}

pub(crate) fn antigravity_tool_summary(name: &str, canonical: &str, args: &Value) -> String {
    match name {
        "run_command" | "exec" | "bash" | "shell" => {
            let cmd = args
                .get("CommandLine")
                .or_else(|| args.get("command"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let truncated = truncate_preview_text(cmd, 40).unwrap_or_else(|| cmd.to_string());
            format!("[Bash: {}]", truncated)
        }
        "view_file" | "read_file" | "read" | "view" => {
            let path = args
                .get("AbsolutePath")
                .or_else(|| args.get("path"))
                .or_else(|| args.get("file_path"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let filename = Path::new(path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(path);
            format!("[Read: {}]", filename)
        }
        "write_to_file" | "write_file" | "write" => {
            let path = args
                .get("TargetFile")
                .or_else(|| args.get("path"))
                .or_else(|| args.get("file_path"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let filename = Path::new(path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(path);
            format!("[Write: {}]", filename)
        }
        "replace_file_content" | "multi_replace_file_content" | "edit" | "edit_file" => {
            let path = args
                .get("TargetFile")
                .or_else(|| args.get("path"))
                .or_else(|| args.get("file_path"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let filename = Path::new(path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(path);
            format!("[Edit: {}]", filename)
        }
        "grep_search" | "search_file_content" | "grep" => {
            let query = args
                .get("Query")
                .or_else(|| args.get("query"))
                .or_else(|| args.get("pattern"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            format!("[Grep: {}]", query)
        }
        "find_by_name" | "list_dir" | "list_directory" | "glob" | "ls" => {
            let pat = args
                .get("Pattern")
                .or_else(|| args.get("DirectoryPath"))
                .or_else(|| args.get("pattern"))
                .or_else(|| args.get("path"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            format!("[Glob: {}]", pat)
        }
        "invoke_subagent" => {
            let prompts = invoke_subagent_prompts(args.get("Subagents"));
            let first = prompts.first().map(String::as_str).unwrap_or("subagent");
            let truncated = truncate_preview_text(first, 30).unwrap_or_else(|| first.to_string());
            format!("[Task: {}]", truncated)
        }
        "manage_subagents" | "manage_task" => {
            let action = args
                .get("Action")
                .or_else(|| args.get("action"))
                .and_then(|v| v.as_str())
                .unwrap_or("task");
            format!("[Task: {}]", action)
        }
        "send_message" => "[SendMessage]".to_string(),
        "schedule" => "[Schedule]".to_string(),
        "ask_question" => "[AskUser]".to_string(),
        _ => session::tool_use_summary(canonical, args),
    }
}

// ─── 扫描累加器 ───

struct PendingTool {
    message_index: usize,
    raw_name: String,
    tool_use_id: String,
    subagent_prompts: Vec<String>,
}

pub(crate) struct AntigravityScanAccum {
    pub(crate) messages: Vec<ChatMessage>,
    pending_tools: VecDeque<PendingTool>,
    pub(crate) subagent_map: HashMap<String, SubagentInfo>,
    pub(crate) candidate_paths: Vec<String>,
    pub(crate) first_user_msg: Option<String>,
    pub(crate) first_timestamp: Option<String>,
    pub(crate) last_timestamp: Option<String>,
    pub(crate) current_model: Option<String>,
    context_chars: usize,
    pub(crate) child_session_ids: Vec<String>,
    pub(crate) invoke_workspace: Option<String>,
    pub(crate) parent_from_send: Option<String>,
}

impl AntigravityScanAccum {
    pub(crate) fn new() -> Self {
        Self {
            messages: Vec::new(),
            pending_tools: VecDeque::new(),
            subagent_map: HashMap::new(),
            candidate_paths: Vec::new(),
            first_user_msg: None,
            first_timestamp: None,
            last_timestamp: None,
            current_model: None,
            context_chars: 0,
            child_session_ids: Vec::new(),
            invoke_workspace: None,
            parent_from_send: None,
        }
    }

    pub(crate) fn process_step(&mut self, step: &Step, conversation_id: &str, brain_dir: &Path) {
        if self.first_timestamp.is_none() && !step.created_at.is_empty() {
            self.first_timestamp = Some(step.created_at.clone());
        }
        if !step.created_at.is_empty() {
            self.last_timestamp = Some(step.created_at.clone());
        }

        if let Some(ref tool_calls) = step.tool_calls {
            for tc in tool_calls {
                if let Some(ref args) = tc.args {
                    extract_absolute_paths_from_value(args, &mut self.candidate_paths);
                }
                if self.parent_from_send.is_none() {
                    if let Some(recipient) = recipient_from_send_message(tc) {
                        if recipient != conversation_id {
                            self.parent_from_send = Some(recipient);
                        }
                    }
                }
            }
        }

        match step.step_type.as_str() {
            "USER_INPUT" => self.handle_user_input(step),
            "PLANNER_RESPONSE" => self.handle_planner_response(step),
            "CONVERSATION_HISTORY" => {}
            "INVOKE_SUBAGENT" => {
                // 独立收集子代理 id / workspace：不依赖 pending 工具队列配对，
                // 避免队列错位或 source 非 MODEL/SYSTEM 时子代理关联被静默丢弃
                // （child_session_ids 去重、invoke_workspace 仅取首个，幂等）。
                let content = step.content.as_deref().unwrap_or("");
                let info = parse_invoke_subagent_content(content);
                for id in &info.conversation_ids {
                    if id != conversation_id && !self.child_session_ids.contains(id) {
                        self.child_session_ids.push(id.clone());
                    }
                }
                if self.invoke_workspace.is_none() {
                    self.invoke_workspace = info.workspace.clone();
                }
                // 仍按工具结果走配对流程，维持 subagent_map / ToolResult 展示行为
                self.handle_tool_result(step, conversation_id, brain_dir);
            }
            _ => self.handle_tool_result(step, conversation_id, brain_dir),
        }
    }

    fn handle_user_input(&mut self, step: &Step) {
        let raw_content = step.content.clone().unwrap_or_default();
        if let Some(m) = extract_model_from_content(&raw_content) {
            self.current_model = Some(m);
        }
        let clean = clean_user_content(&raw_content);
        let image_paths = extract_uploaded_image_paths(&raw_content);

        let mut parts = Vec::new();
        for path in &image_paths {
            parts.push(ContentPart::ImageRef {
                path: path.clone(),
            });
        }
        if !clean.is_empty() {
            parts.push(ContentPart::Text { text: clean.clone() });
        } else if parts.is_empty() {
            parts.push(ContentPart::Text { text: String::new() });
        }

        self.context_chars += clean.len();
        if self.first_user_msg.is_none() && !clean.is_empty() {
            self.first_user_msg = Some(clean);
        }

        self.messages.push(ChatMessage {
            role: "user".to_string(),
            timestamp: step.created_at.clone(),
            model: self.current_model.clone(),
            token_usage: None,
            content_parts: parts,
            is_meta: false,
            uuid: None,
        });
    }

    fn handle_planner_response(&mut self, step: &Step) {
        let mut thinking_len = 0;
        let mut content_parts = Vec::new();

        if let Some(thinking) = &step.thinking {
            let trimmed = thinking.trim();
            if !trimmed.is_empty() {
                thinking_len = trimmed.len();
                content_parts.push(ContentPart::Thinking {
                    thinking: trimmed.to_string(),
                });
            }
        }

        let mut assistant_content_len = 0;
        if let Some(content) = &step.content {
            let trimmed = content.trim();
            if !trimmed.is_empty() {
                assistant_content_len = trimmed.len();
                content_parts.push(ContentPart::Text {
                    text: trimmed.to_string(),
                });
            }
        }

        let msg_idx = self.messages.len();

        if let Some(tool_calls) = &step.tool_calls {
            for (tc_idx, tc) in tool_calls.iter().enumerate() {
                let decoded_args = tc
                    .args
                    .as_ref()
                    .map(decode_antigravity_value)
                    .unwrap_or(Value::Object(serde_json::Map::new()));

                let subagent_prompts: Vec<String> = if tc.name == "invoke_subagent" {
                    invoke_subagent_prompts(tc.args.as_ref().and_then(|args| args.get("Subagents")))
                } else {
                    Vec::new()
                };

                let canonical = map_tool_name(&tc.name);
                let summary = antigravity_tool_summary(&tc.name, canonical, &decoded_args);
                let input_str = serde_json::to_string_pretty(&decoded_args).unwrap_or_default();
                self.context_chars += input_str.len();

                let tool_use_id = format!("call_{}_{}", step.step_index, tc_idx);

                content_parts.push(ContentPart::ToolUse {
                    summary,
                    tool_name: canonical.to_string(),
                    input: input_str,
                    tool_use_id: Some(tool_use_id.clone()),
                });

                self.pending_tools.push_back(PendingTool {
                    message_index: msg_idx,
                    raw_name: tc.name.clone(),
                    tool_use_id,
                    subagent_prompts,
                });
            }
        }

        if content_parts.is_empty() {
            return;
        }

        let input_tokens = (self.context_chars / 4).max(1) as u64;
        let output_tokens = ((thinking_len + assistant_content_len) / 4).max(1) as u64;
        let token_usage = Some(session::TokenUsage {
            input_tokens,
            output_tokens,
            cache_creation_input_tokens: 0,
            cache_read_input_tokens: 0,
        });

        self.context_chars += thinking_len + assistant_content_len;

        self.messages.push(ChatMessage {
            role: "assistant".to_string(),
            timestamp: step.created_at.clone(),
            model: self.current_model.clone(),
            token_usage,
            content_parts,
            is_meta: false,
            uuid: None,
        });
    }

    fn handle_tool_result(
        &mut self,
        step: &Step,
        conversation_id: &str,
        brain_dir: &Path,
    ) {
        if step.source == "MODEL" || step.source == "SYSTEM" {
            if let Some(pending) = self.pending_tools.pop_front() {
                let raw_content = step.content.clone().unwrap_or_default();
                let is_error = step.status == "ERROR";

                let mut display_content = raw_content.trim().to_string();
                let mut truncated = false;
                // 与 claude_entry 共用同一个上限：50 KB 是用户可见的截断行为，各写一份会各自漂移
                if display_content.len() > TOOL_RESULT_MAX_LEN {
                    let mut cut = TOOL_RESULT_MAX_LEN;
                    while cut > 0 && !display_content.is_char_boundary(cut) {
                        cut -= 1;
                    }
                    display_content.truncate(cut);
                    truncated = true;
                }

                let canonical = map_tool_name(&pending.raw_name);
                let summary = if is_error {
                    format!("[{} error]", canonical)
                } else {
                    format!("[{} result]", canonical)
                };

                // 针对 invoke_subagent / manage_subagents 关联 subagent_map
                let mut spawned_children: Vec<(String, String)> = Vec::new();

                if pending.raw_name == "invoke_subagent" {
                    let info = parse_invoke_subagent_content(&raw_content);
                    for (i, child_id) in info.conversation_ids.iter().enumerate() {
                        if child_id != conversation_id {
                            let label = pending
                                .subagent_prompts
                                .get(i)
                                .cloned()
                                .unwrap_or_else(|| "Subagent".to_string());
                            spawned_children.push((child_id.clone(), label));
                        }
                    }
                    if self.invoke_workspace.is_none() {
                        self.invoke_workspace = info.workspace.clone();
                    }
                } else if pending.raw_name == "manage_subagents" {
                    let manage_info = parse_manage_subagents_content(&raw_content);
                    for (id, prompt) in manage_info
                        .conversation_ids
                        .iter()
                        .zip(manage_info.prompts.iter())
                    {
                        if id != conversation_id {
                            let label = if prompt.is_empty() {
                                "Subagent".to_string()
                            } else {
                                prompt.clone()
                            };
                            spawned_children.push((id.clone(), label));
                        }
                    }
                }

                for (child_id, label) in spawned_children {
                    if !self.child_session_ids.contains(&child_id) {
                        self.child_session_ids.push(child_id.clone());
                    }
                    let child_transcript = find_child_transcript_path(brain_dir, &child_id);
                    self.subagent_map.insert(
                        pending.tool_use_id.clone(),
                        SubagentInfo {
                            file_path: child_transcript.to_string_lossy().to_string(),
                            label,
                        },
                    );
                }

                if let Some(msg) = self.messages.get_mut(pending.message_index) {
                    msg.content_parts.push(ContentPart::tool_result(
                        summary,
                        display_content,
                        is_error,
                        truncated,
                    ));
                }
            }
        }
    }
}

pub(crate) fn find_child_transcript_path(brain_dir: &Path, child_id: &str) -> PathBuf {
    let candidate1 = brain_dir
        .join(child_id)
        .join(".system_generated")
        .join("logs")
        .join("transcript.jsonl");
    if candidate1.exists() {
        return candidate1;
    }
    let candidate2 = brain_dir.join(child_id).join("transcript.jsonl");
    if candidate2.exists() {
        return candidate2;
    }
    candidate1
}

// ─── 统一分派解析入口 ───

pub(crate) fn parse_session_file(file_path: &str) -> AppResult<Vec<ChatMessage>> {
    parse_session_file_with_offset(file_path, false).map(|r| r.messages)
}

pub(crate) fn parse_session_file_with_offset(
    file_path: &str,
    _skip_sidechain: bool,
) -> AppResult<SessionLoadResult> {
    let path = Path::new(file_path);
    let file = File::open(path)?;
    let file_size = file.metadata()?.len();
    let reader = BufReader::with_capacity(256 * 1024, file);

    let conversation_id = extract_conversation_id(path).unwrap_or_default();
    let brain_dir = dirs::home_dir()
        .map(|h| h.join(".gemini").join("antigravity-cli").join("brain"))
        .unwrap_or_else(|| PathBuf::from("."));

    let mut accum = AntigravityScanAccum::new();
    for line in reader.lines().flatten() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(step) = serde_json::from_str::<Step>(trimmed) {
            accum.process_step(&step, &conversation_id, &brain_dir);
        }
    }

    Ok(SessionLoadResult {
        messages: accum.messages,
        offset: file_size,
        subagent_map: accum.subagent_map,
    })
}

pub(crate) fn parse_session_file_streaming<F>(
    file_path: &str,
    skip_sidechain: bool,
    mut on_batch: F,
) -> AppResult<(u64, HashMap<String, SubagentInfo>)>
where
    F: FnMut(Vec<ChatMessage>) -> bool,
{
    let res = parse_session_file_with_offset(file_path, skip_sidechain)?;
    let mut emitter = BatchEmitter::new();
    for msg in res.messages {
        emitter.push(msg);
        if !emitter.maybe_flush(&mut on_batch) {
            break;
        }
    }
    emitter.flush_remaining(&mut on_batch);
    Ok((res.offset, res.subagent_map))
}

pub(crate) fn parse_session_incremental(
    file_path: &str,
    offset: u64,
) -> AppResult<SessionLoadResult> {
    let mut file = File::open(file_path)?;
    let file_size = file.metadata()?.len();

    if file_size <= offset {
        return Ok(SessionLoadResult {
            messages: vec![],
            offset,
            subagent_map: HashMap::new(),
        });
    }

    file.seek(SeekFrom::Start(offset))?;
    let reader = BufReader::new(file);

    let path = Path::new(file_path);
    let conversation_id = extract_conversation_id(path).unwrap_or_default();
    let brain_dir = dirs::home_dir()
        .map(|h| h.join(".gemini").join("antigravity-cli").join("brain"))
        .unwrap_or_else(|| PathBuf::from("."));

    let mut accum = AntigravityScanAccum::new();
    for line in reader.lines().flatten() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(step) = serde_json::from_str::<Step>(trimmed) {
            accum.process_step(&step, &conversation_id, &brain_dir);
        }
    }

    Ok(SessionLoadResult {
        messages: accum.messages,
        offset: file_size,
        subagent_map: accum.subagent_map,
    })
}

pub(crate) fn parse_antigravity_session_from_string(content: &str) -> Vec<ChatMessage> {
    let conversation_id = "archived";
    let brain_dir = dirs::home_dir()
        .map(|h| h.join(".gemini").join("antigravity-cli").join("brain"))
        .unwrap_or_else(|| PathBuf::from("."));

    let mut accum = AntigravityScanAccum::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(step) = serde_json::from_str::<Step>(trimmed) {
            accum.process_step(&step, conversation_id, &brain_dir);
        }
    }
    accum.messages
}

pub(crate) fn read_first_user_message(file_path: &str) -> Option<String> {
    let Ok(file) = File::open(file_path) else {
        return None;
    };
    let reader = BufReader::new(file);
    for line in reader.lines().flatten() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(step) = serde_json::from_str::<Step>(trimmed) {
            if step.step_type == "USER_INPUT" {
                let raw = step.content.unwrap_or_default();
                let clean = clean_user_content(&raw);
                if !clean.is_empty() {
                    return Some(clean);
                }
            }
        }
    }
    None
}

pub(crate) fn read_session_id(file_path: &str) -> Option<String> {
    extract_conversation_id(Path::new(file_path))
}

/// 解析会话所属的项目路径。
///
/// 三个来源在**同一个出口**收口到同一条不变量：**空白不是路径**。这一列的缺席语义是
/// `None`（SQL `NULL`），放行空串会让它变成第三种表达 —— 而落库侧与读取侧都只按「非空」
/// 判缺席，空串会一路装成「有值」。逐分支各加一次过滤正是 codex 的 `session_meta` 分支
/// 漏掉这道过滤的成因，故这里放在出口处。
pub(crate) fn read_project_path(file_path: &str) -> Option<String> {
    read_project_path_raw(file_path).filter(|value| !value.trim().is_empty())
}

fn read_project_path_raw(file_path: &str) -> Option<String> {
    let path = Path::new(file_path);
    let conv_id = extract_conversation_id(path);

    let history = history_snapshot();
    if let Some(ref cid) = conv_id {
        if let Some(ws) = history.workspaces.get(cid) {
            return Some(ws.clone());
        }
    }

    if let Some(first_msg) = read_first_user_message(file_path) {
        if let Some(ws) = find_workspace_by_display_content(&first_msg) {
            return Some(ws);
        }
    }

    if let Some(ws) = extract_project_path_from_transcript(file_path) {
        return Some(ws);
    }

    None
}

pub(crate) fn read_first_timestamp(file_path: &str) -> Option<String> {
    let Ok(file) = File::open(file_path) else {
        return None;
    };
    let reader = BufReader::new(file);
    for line in reader.lines().flatten() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(step) = serde_json::from_str::<Step>(trimmed) {
            if !step.created_at.is_empty() {
                return Some(step.created_at);
            }
        }
    }
    None
}

pub(crate) fn read_last_timestamp(file_path: &str) -> Option<String> {
    let Ok(file) = File::open(file_path) else {
        return None;
    };
    let reader = BufReader::new(file);
    let mut last = None;
    for line in reader.lines().flatten() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(step) = serde_json::from_str::<Step>(trimmed) {
            if !step.created_at.is_empty() {
                last = Some(step.created_at);
            }
        }
    }
    last
}

pub(crate) fn extract_usage_records(file_path: &str, project: &str) -> Vec<UsageRecord> {
    let Ok(file) = File::open(file_path) else {
        return Vec::new();
    };
    // 与主解析路径一致的缓冲（256 KB），避免逐行 syscall
    let reader = BufReader::with_capacity(256 * 1024, file);
    let mut records = Vec::new();
    let mut current_model = "antigravity".to_string();

    for line in reader.lines().flatten() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(step) = serde_json::from_str::<Step>(trimmed) else {
            continue;
        };
        if step.step_type == "USER_INPUT" {
            if let Some(content) = &step.content {
                if let Some(m) = extract_model_from_content(content) {
                    current_model = m;
                }
            }
        } else if step.step_type == "PLANNER_RESPONSE" {
            let date = if step.created_at.len() >= 10 {
                step.created_at[..10].to_string()
            } else {
                "unknown".to_string()
            };
            let thinking_len = step.thinking.as_deref().map(str::len).unwrap_or(0);
            let content_len = step.content.as_deref().map(str::len).unwrap_or(0);
            let output_tokens = ((thinking_len + content_len) / 4).max(1) as u64;

            records.push(UsageRecord {
                date,
                model: current_model.clone(),
                input_tokens: 100,
                output_tokens,
                cache_creation_tokens: 0,
                cache_read_tokens: 0,
                duration_ms: None,
                project: project.to_string(),
            });
        }
    }

    records
}

pub(crate) fn scan_session_metadata_only(file_path: &Path) -> Option<SessionListMetadata> {
    let path_str = file_path.to_string_lossy().to_string();
    let session_id = extract_conversation_id(file_path)?;
    let file_metadata = std::fs::metadata(file_path).ok()?;
    let file_size = file_metadata.len();

    let first_user_message = read_first_user_message(&path_str);
    let project_path = read_project_path(&path_str);
    let first_timestamp = read_first_timestamp(&path_str);
    let last_timestamp = read_last_timestamp(&path_str);

    Some(SessionListMetadata {
        session_id,
        project_path,
        title: None,
        first_user_message,
        first_timestamp,
        last_timestamp,
        git_branch: String::new(),
        file_size,
    })
}

pub(crate) fn is_subagent_session(file_path: &str) -> bool {
    let path = Path::new(file_path);
    let Some(conv_id) = extract_conversation_id(path) else {
        return false;
    };

    let history = history_snapshot();
    // history.jsonl 仅记录用户主动在 CLI 启动的根会话，被明确记录的一定是根会话
    if history.workspaces.contains_key(&conv_id) {
        return false;
    }

    // history 未命中不代表一定是子代理：新建会话可能尚未 flush 进 history、
    // 会话可能由 MCP/IDE 等程序化方式启动（不写 history）、或 history 被轮转清空。
    // 此时回退到内容特征判定，避免根会话被误判为子代理而从列表中隐藏。
    if let Ok(file) = File::open(file_path) {
        let reader = BufReader::new(file);
        for line in reader.lines().flatten() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let Ok(step) = serde_json::from_str::<Step>(trimmed) else {
                continue;
            };
            if step.step_type == "USER_INPUT" {
                if let Some(ref content) = step.content {
                    let clean = clean_user_content(content);
                    if clean.starts_with("You are implementing Task")
                        || clean.starts_with("You are reviewing Task")
                        || clean.starts_with("You are the Final Code Reviewer")
                        || clean.starts_with("你正在执行")
                        || clean.starts_with("你正在对")
                        || clean.starts_with("你正在将")
                    {
                        return true;
                    }
                }
            }
            if let Some(ref tool_calls) = step.tool_calls {
                for tc in tool_calls {
                    if let Some(recipient) = recipient_from_send_message(tc) {
                        if recipient != conv_id {
                            return true;
                        }
                    }
                }
            }
        }
    }
    false
}

pub(crate) fn extract_antigravity_role_text(entry: &Value) -> Option<(String, String)> {
    let step_type = entry.get("type").and_then(|v| v.as_str())?;
    match step_type {
        "USER_INPUT" => {
            let content = entry.get("content").and_then(|v| v.as_str()).unwrap_or("");
            let clean = clean_user_content(content);
            if clean.is_empty() {
                None
            } else {
                Some(("user".to_string(), clean))
            }
        }
        "PLANNER_RESPONSE" => {
            let mut text = String::new();
            if let Some(c) = entry.get("content").and_then(|v| v.as_str()) {
                text.push_str(c);
            }
            if let Some(th) = entry.get("thinking").and_then(|v| v.as_str()) {
                if !text.is_empty() {
                    text.push(' ');
                }
                text.push_str(th);
            }
            if text.trim().is_empty() {
                None
            } else {
                Some(("assistant".to_string(), text))
            }
        }
        _ => None,
    }
}

// ─── 单元测试 ───

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_clean_user_content() {
        let raw = "<USER_REQUEST>\n请帮我优化代码\n</USER_REQUEST>\n<ADDITIONAL_METADATA>\ntime: 2026\n</ADDITIONAL_METADATA>";
        assert_eq!(clean_user_content(raw), "请帮我优化代码");
    }

    #[test]
    fn test_extract_uploaded_image_paths() {
        let content = r#"<USER_REQUEST>看这张图</USER_REQUEST>
<ADDITIONAL_METADATA>
The user has uploaded 2 image(s):
- /tmp/a.png
- /tmp/b.jpg
You can embed these images in an artifact.
</ADDITIONAL_METADATA>"#;
        let paths = extract_uploaded_image_paths(content);
        assert_eq!(paths, vec!["/tmp/a.png", "/tmp/b.jpg"]);
    }

    #[test]
    fn test_decode_antigravity_value() {
        let input = json!({
            "CommandLine": "\"ls -la\"",
            "WaitMsBeforeAsync": "1000",
            "Nested": {
                "Target": "\"foo.txt\""
            }
        });
        let decoded = decode_antigravity_value(&input);
        assert_eq!(decoded["CommandLine"], json!("ls -la"));
        assert_eq!(decoded["WaitMsBeforeAsync"], json!(1000));
        assert_eq!(decoded["Nested"]["Target"], json!("foo.txt"));
    }

    #[test]
    fn test_subagent_extraction() {
        let invoke_content = r#"Created the following subagents:
{
  "conversationId": "child-123",
  "workspaceUris": ["file:///tmp/my-project"]
}"#;
        let info = parse_invoke_subagent_content(invoke_content);
        assert_eq!(info.conversation_ids, vec!["child-123".to_string()]);
        assert_eq!(info.workspace.as_deref(), Some("/tmp/my-project"));
    }

    #[test]
    fn test_recipient_from_send_message() {
        let tc = ToolCall {
            name: "send_message".to_string(),
            args: Some(json!({
                "Recipient": "\"parent-uuid-999\"",
                "Message": "done"
            })),
        };
        assert_eq!(recipient_from_send_message(&tc).as_deref(), Some("parent-uuid-999"));
    }

    #[test]
    fn test_extract_workspace_from_user_info() {
        let content = r#"<user_information>
The USER's OS version is mac.
The user has 1 active workspaces, each defined by a URI and a CorpusName. Multiple URIs potentially map to the same CorpusName. The mapping is shown as follows in the format [URI] -> [CorpusName]:
/Users/user/codes/workspace/SeshBuddy -> huangy/SeshBuddy
Code relating to the user's requests should be written in the locations listed above.
</user_information>"#;
        assert_eq!(
            extract_workspace_from_user_info(content).as_deref(),
            Some("/Users/user/codes/workspace/SeshBuddy")
        );
    }

    #[test]
    fn test_invoke_subagent_populates_subagent_map() {
        let mut accum = AntigravityScanAccum::new();
        let step1 = Step {
            step_index: 0,
            source: "MODEL".to_string(),
            step_type: "PLANNER_RESPONSE".to_string(),
            status: "DONE".to_string(),
            created_at: "2026-08-31T09:00:00Z".to_string(),
            content: Some("Dispatching subagent".to_string()),
            thinking: None,
            tool_calls: Some(vec![ToolCall {
                name: "invoke_subagent".to_string(),
                args: Some(json!({
                    "Subagents": "[{\"Model\":\"inherit\",\"Prompt\":\"Implement Task 1\",\"Role\":\"Coder\",\"TypeName\":\"self\"}]"
                })),
            }]),
        };
        let dummy_brain = Path::new("/tmp/dummy_brain");
        accum.process_step(&step1, "parent-session-1", dummy_brain);

        let step2 = Step {
            step_index: 1,
            source: "MODEL".to_string(),
            step_type: "GENERIC".to_string(),
            status: "DONE".to_string(),
            created_at: "2026-08-31T09:00:01Z".to_string(),
            content: Some(r#"Created the following subagents:
{
  "conversationId": "subagent-session-abc",
  "workspaceUris": ["file:///Users/user/codes/workspace/SeshBuddy"]
}"#.to_string()),
            thinking: None,
            tool_calls: None,
        };
        accum.process_step(&step2, "parent-session-1", dummy_brain);

        assert_eq!(accum.child_session_ids, vec!["subagent-session-abc".to_string()]);
        assert_eq!(accum.invoke_workspace.as_deref(), Some("/Users/user/codes/workspace/SeshBuddy"));
        let subagent = accum.subagent_map.get("call_0_0").expect("subagent_map should have call_0_0");
        assert_eq!(subagent.label, "Implement Task 1");
    }
}
