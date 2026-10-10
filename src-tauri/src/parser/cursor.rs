//! Cursor CLI 会话解析器。
//!
//! Cursor CLI (`agent`) 会话转录以 JSONL 形式存储于：
//! `~/.cursor/projects/<project-key>/agent-transcripts/<sessionId>/<sessionId>.jsonl`
//! 子代理会话存储于：
//! `~/.cursor/projects/<project-key>/agent-transcripts/<sessionId>/subagents/<subagentId>.jsonl`
//! 对应的 Sidecar SQLite 状态数据库存储于：
//! `~/.cursor/chats/<md5-bucket>/<sessionId>/store.db`（同级可能带有 `meta.json`）。
//!
//! 本模块负责转录内容的增量/流式解析、用户指令与思考链提取、工具规范化、
//! 以及与 `store.db` / `meta.json` 的元数据穿透融合。

use crate::error::AppResult;
use crate::parser::batch::BatchEmitter;
use crate::parser::shared::{SearchDocument, SearchScanProgress};
use crate::session::{self, ChatMessage, ContentPart, SessionListMetadata, SessionLoadResult, SubagentInfo};
use serde_json::Value;
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// 从 store.db 与 meta.json 中抽取的增强元数据。
#[derive(Debug, Clone, Default)]
pub(crate) struct CursorStoreInfo {
    pub workspace_path: Option<String>,
    pub title: Option<String>,
    pub model: Option<String>,
    pub created_at_secs: Option<i64>,
}

// ─────────────────────────────────────────────────────────────────────────────
// 内容清洗与标签提取
// ─────────────────────────────────────────────────────────────────────────────

/// 剥离用户消息中的 XML 包装（`<user_query>` 等），剔除系统注入的 `<user_info>`。
pub(crate) fn clean_user_content(text: &str) -> String {
    let with_images = rewrite_image_files_block(text);
    let prompt = extract_tag_content(&with_images, "user_query")
        .map(str::to_string)
        .unwrap_or(with_images);
    let trimmed = prompt.trim();
    if trimmed.is_empty()
        || trimmed.starts_with("<user_info>")
        || trimmed.starts_with("<agent_transcripts>")
    {
        return String::new();
    }
    trimmed.to_string()
}

/// 提取 `<tag>...</tag>` 内的内容。
pub(crate) fn extract_tag_content<'a>(text: &'a str, tag: &str) -> Option<&'a str> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = text.find(&open)?;
    let after = &text[start + open.len()..];
    let end = after.find(&close)?;
    Some(&after[..end])
}

/// 将 `<image_files>` 块重写为统一的 `[Image: source: <path>]` 标记。
pub(crate) fn rewrite_image_files_block(text: &str) -> String {
    let Some(inner) = extract_tag_content(text, "image_files") else {
        return text.to_string();
    };
    let mut markers = Vec::new();
    for raw_line in inner.lines() {
        let line = raw_line.trim();
        let path = line
            .split_once('.')
            .map(|(prefix, rest)| (prefix.trim(), rest.trim()))
            .filter(|(prefix, _)| !prefix.is_empty() && prefix.chars().all(|c| c.is_ascii_digit()))
            .map(|(_, rest)| rest);
        if let Some(p) = path {
            if !p.is_empty() {
                markers.push(format!("[Image: source: {p}]"));
            }
        }
    }
    let before = text
        .split("<image_files>")
        .next()
        .unwrap_or("")
        .replace("[Image]", "")
        .trim_end()
        .to_string();
    let after = text
        .split("</image_files>")
        .nth(1)
        .unwrap_or("")
        .to_string();
    let mut out = String::new();
    if !before.is_empty() {
        out.push_str(&before);
        out.push('\n');
    }
    out.push_str(&markers.join("\n"));
    let after_trimmed = after.trim_start();
    if !after_trimmed.is_empty() {
        out.push('\n');
        out.push_str(after_trimmed);
    }
    out
}

/// 从助手消息中提取 `<think>...</think>` 思维链内容。
pub(crate) fn extract_think_content(text: &str) -> Option<String> {
    let start = text.find("<think>")?;
    let after = &text[start + "<think>".len()..];
    let end = after.find("</think>").unwrap_or(after.len());
    let thinking = after[..end].trim();
    if thinking.is_empty() {
        None
    } else {
        Some(thinking.to_string())
    }
}

/// 剔除 `<think>...</think>` 标签及其包含的内容。
pub(crate) fn strip_think_tags(text: &str) -> String {
    if !text.contains("<think>") {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut remaining = text;
    while let Some(start) = remaining.find("<think>") {
        out.push_str(&remaining[..start]);
        remaining = &remaining[start + "<think>".len()..];
        match remaining.find("</think>") {
            Some(end) => remaining = &remaining[end + "</think>".len()..],
            None => {
                remaining = "";
                break;
            }
        }
    }
    out.push_str(remaining);
    out.trim().to_string()
}

/// 擦除服务端脱敏留下的 `[REDACTED]` 占位符并压实空行。
pub(crate) fn strip_redacted(text: &str) -> String {
    let cleaned = text.replace("[REDACTED]", "");
    cleaned
        .lines()
        .filter(|l| !l.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// 从 `<user_info>` 文本中提取工作区绝对路径。
pub(crate) fn extract_workspace_path_from_user_info(text: &str) -> Option<String> {
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("Workspace Path:") {
            let path = rest.trim();
            if !path.is_empty() {
                return Some(path.to_string());
            }
        }
    }
    None
}

// ─────────────────────────────────────────────────────────────────────────────
// 工具调用规范化
// ─────────────────────────────────────────────────────────────────────────────

/// 规范化工具名称。
pub(crate) fn map_tool_name(raw_name: &str) -> &'static str {
    match raw_name {
        "run_command" | "exec" | "bash" | "shell" | "Bash" => "Bash",
        "view_file" | "read_file" | "read" | "Read" => "Read",
        "write_to_file" | "write_file" | "write" | "Write" => "Write",
        "replace_file_content" | "edit" | "edit_file" | "Edit" => "Edit",
        "apply_patch" | "patch" | "ApplyPatch" => "ApplyPatch",
        "grep_search" | "search_file_content" | "grep" | "Grep" => "Grep",
        "find_by_name" | "list_dir" | "list_directory" | "glob" | "ls" | "Glob" => "Glob",
        "task" | "subagent" | "Task" | "Subagent" => "Task",
        _ => "Tool",
    }
}

/// 提取补丁文本中的目标文件路径。
fn extract_patch_file_path(patch: &str) -> String {
    for line in patch.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed
            .strip_prefix("*** Update File: ")
            .or_else(|| trimmed.strip_prefix("*** Add File: "))
            .or_else(|| trimmed.strip_prefix("*** Delete File: "))
        {
            return rest.trim().to_string();
        }
    }
    String::new()
}

/// 规范化工具参数为统一的 JSON 字符串。
pub(crate) fn remap_tool_args(canonical_name: &str, args: &Value) -> Value {
    if let Value::String(s) = args {
        if canonical_name == "ApplyPatch" {
            let file_path = extract_patch_file_path(s);
            return serde_json::json!({ "file_path": file_path, "patch": s });
        }
    }
    let Some(obj) = args.as_object() else {
        return args.clone();
    };

    let mut out = serde_json::Map::new();
    match canonical_name {
        "Bash" => {
            if let Some(cmd) = obj.get("command").or_else(|| obj.get("input")).and_then(|v| v.as_str()) {
                out.insert("command".to_string(), Value::String(cmd.to_string()));
            }
            if let Some(cwd) = obj.get("working_directory").or_else(|| obj.get("cwd")).and_then(|v| v.as_str()) {
                out.insert("cwd".to_string(), Value::String(cwd.to_string()));
            }
            if let Some(desc) = obj.get("description").and_then(|v| v.as_str()) {
                out.insert("description".to_string(), Value::String(desc.to_string()));
            }
        }
        "Read" => {
            if let Some(path) = obj.get("path").or_else(|| obj.get("file_path")).and_then(|v| v.as_str()) {
                out.insert("file_path".to_string(), Value::String(path.to_string()));
            }
            if let Some(limit) = obj.get("limit") {
                out.insert("limit".to_string(), limit.clone());
            }
            if let Some(offset) = obj.get("offset") {
                out.insert("offset".to_string(), offset.clone());
            }
        }
        "Write" => {
            if let Some(path) = obj.get("path").or_else(|| obj.get("file_path")).and_then(|v| v.as_str()) {
                out.insert("file_path".to_string(), Value::String(path.to_string()));
            }
            if let Some(content) = obj.get("contents").or_else(|| obj.get("content")).and_then(|v| v.as_str()) {
                out.insert("content".to_string(), Value::String(content.to_string()));
            }
        }
        "Edit" => {
            if let Some(path) = obj.get("path").or_else(|| obj.get("file_path")).and_then(|v| v.as_str()) {
                out.insert("file_path".to_string(), Value::String(path.to_string()));
            }
            if let Some(old) = obj
                .get("old_str")
                .or_else(|| obj.get("old_string"))
                .or_else(|| obj.get("old_text"))
                .and_then(|v| v.as_str())
            {
                out.insert("old_string".to_string(), Value::String(old.to_string()));
            }
            if let Some(new) = obj
                .get("new_str")
                .or_else(|| obj.get("new_string"))
                .or_else(|| obj.get("new_text"))
                .and_then(|v| v.as_str())
            {
                out.insert("new_string".to_string(), Value::String(new.to_string()));
            }
        }
        "Glob" => {
            if let Some(pattern) = obj.get("glob_pattern").or_else(|| obj.get("pattern")).and_then(|v| v.as_str()) {
                out.insert("pattern".to_string(), Value::String(pattern.to_string()));
            }
            if let Some(path) = obj.get("target_directory").or_else(|| obj.get("path")).and_then(|v| v.as_str()) {
                out.insert("path".to_string(), Value::String(path.to_string()));
            }
        }
        "Grep" => {
            if let Some(pattern) = obj.get("pattern").and_then(|v| v.as_str()) {
                out.insert("pattern".to_string(), Value::String(pattern.to_string()));
            }
            if let Some(path) = obj.get("path").and_then(|v| v.as_str()) {
                out.insert("path".to_string(), Value::String(path.to_string()));
            }
            if let Some(glob) = obj.get("glob").and_then(|v| v.as_str()) {
                out.insert("glob".to_string(), Value::String(glob.to_string()));
            }
        }
        "Task" => {
            for key in ["description", "prompt", "subagent_type"] {
                if let Some(v) = obj.get(key).and_then(|v| v.as_str()) {
                    out.insert(key.to_string(), Value::String(v.to_string()));
                }
            }
        }
        _ => return args.clone(),
    }

    Value::Object(out)
}

/// 生成工具摘要。
pub(crate) fn cursor_tool_summary(canonical_name: &str, args: &Value) -> String {
    match canonical_name {
        "Bash" => {
            let cmd = args
                .get("command")
                .or_else(|| args.get("input"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let preview = if cmd.chars().count() > 40 {
                format!("{}...", cmd.chars().take(40).collect::<String>())
            } else {
                cmd.to_string()
            };
            format!("[Bash: {preview}]")
        }
        "Read" => {
            let path = args
                .get("file_path")
                .or_else(|| args.get("path"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let filename = Path::new(path).file_name().and_then(|n| n.to_str()).unwrap_or(path);
            format!("[Read: {filename}]")
        }
        "Write" => {
            let path = args
                .get("file_path")
                .or_else(|| args.get("path"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let filename = Path::new(path).file_name().and_then(|n| n.to_str()).unwrap_or(path);
            format!("[Write: {filename}]")
        }
        "Edit" => {
            let path = args
                .get("file_path")
                .or_else(|| args.get("path"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let filename = Path::new(path).file_name().and_then(|n| n.to_str()).unwrap_or(path);
            format!("[Edit: {filename}]")
        }
        "ApplyPatch" => {
            let path = args.get("file_path").and_then(|v| v.as_str()).unwrap_or("");
            let filename = Path::new(path).file_name().and_then(|n| n.to_str()).unwrap_or(path);
            if !filename.is_empty() {
                format!("[ApplyPatch: {filename}]")
            } else {
                "[ApplyPatch]".to_string()
            }
        }
        "Glob" => {
            let pattern = args.get("pattern").and_then(|v| v.as_str()).unwrap_or("");
            format!("[Glob: {pattern}]")
        }
        "Grep" => {
            let pattern = args.get("pattern").and_then(|v| v.as_str()).unwrap_or("");
            format!("[Grep: {pattern}]")
        }
        "Task" => {
            let desc = args
                .get("description")
                .or_else(|| args.get("prompt"))
                .and_then(|v| v.as_str())
                .unwrap_or("subagent");
            format!("[Task: {desc}]")
        }
        _ => session::tool_use_summary(canonical_name, args),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 路径解码与 store.db 读取
// ─────────────────────────────────────────────────────────────────────────────

/// 还原 Cursor 对项目目录名的编码（将 `/` 替换为 `-`）。
///
/// 优先尝试全量替换，若路径不存在则采用贪心前缀匹配以兼容带有中划线的文件路径。
pub(crate) fn decode_project_key(key: &str) -> String {
    if key.is_empty() {
        return String::new();
    }
    let direct = format!("/{}", key.replace('-', "/"));
    if Path::new(&direct).is_dir() {
        return direct;
    }

    let segments: Vec<&str> = key.split('-').collect();
    let mut path = String::from("/");
    let mut i = 0;
    while i < segments.len() {
        let mut consumed = 0usize;
        for end in (i + 1..=segments.len()).rev() {
            let part = segments[i..end].join("-");
            let candidate = if path == "/" {
                format!("/{part}")
            } else {
                format!("{path}/{part}")
            };
            if Path::new(&candidate).exists() {
                path = candidate;
                consumed = end - i;
                break;
            }
        }
        if consumed == 0 {
            if path == "/" {
                path = format!("/{}", segments[i]);
            } else {
                path = format!("{}/{}", path, segments[i]);
            }
            i += 1;
        } else {
            i += consumed;
        }
    }
    path
}

/// 从转录文件路径逆向解析项目根路径。
pub(crate) fn project_path_from_transcript_path(path: &Path) -> Option<String> {
    let mut current = path.parent();
    while let Some(dir) = current {
        if dir.file_name().and_then(|n| n.to_str()) == Some("agent-transcripts") {
            if let Some(project_key_dir) = dir.parent() {
                let key = project_key_dir
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("");
                let decoded = decode_project_key(key);
                if !decoded.is_empty() {
                    return Some(decoded);
                }
            }
        }
        current = dir.parent();
    }
    None
}

/// 扫描 `~/.cursor/chats/<md5>/<sessionId>/store.db` 获取该会话的 store.db 绝对路径。
pub(crate) fn find_store_db_for_session(home_dir: &Path, session_id: &str) -> Option<PathBuf> {
    let chats_dir = home_dir.join(".cursor").join("chats");
    let Ok(buckets) = std::fs::read_dir(&chats_dir) else {
        return None;
    };
    for bucket in buckets.flatten() {
        let bucket_path = bucket.path();
        if !bucket_path.is_dir() {
            continue;
        }
        let candidate = bucket_path.join(session_id).join("store.db");
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// 从 `store.db` 及同级 `meta.json` 中提取元数据信息。
pub(crate) fn read_store_db_info(session_id: &str, session_file: &Path) -> CursorStoreInfo {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    // 若当前为子代理，其 store.db 归属于父会话
    let lookup_id = parent_id_for_subagent(session_file).unwrap_or_else(|| session_id.to_string());
    let Some(store_path) = find_store_db_for_session(&home, &lookup_id) else {
        return CursorStoreInfo::default();
    };

    let mut info = CursorStoreInfo::default();

    // 1. 读取同级 meta.json
    if let Some(session_dir) = store_path.parent() {
        let meta_json_path = session_dir.join("meta.json");
        if let Ok(meta_str) = std::fs::read_to_string(&meta_json_path) {
            if let Ok(val) = serde_json::from_str::<Value>(&meta_str) {
                if let Some(title) = val.get("title").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
                    info.title = Some(title.to_string());
                }
                if let Some(cwd) = val.get("cwd").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
                    info.workspace_path = Some(cwd.to_string());
                }
            }
        }
    }

    // 2. 读取 SQLite store.db
    let Ok(conn) = rusqlite::Connection::open_with_flags(
        &store_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) else {
        return info;
    };

    // 读取 meta 表
    if let Ok(meta_val_raw) = conn.query_row("SELECT value FROM meta LIMIT 1", [], |r| r.get::<_, String>(0)) {
        let json_text = decode_hex_or_raw(&meta_val_raw);
        if let Ok(meta_val) = serde_json::from_str::<Value>(&json_text) {
            if let Some(ms) = meta_val.get("createdAt").and_then(|v| v.as_i64()) {
                info.created_at_secs = Some(ms / 1000);
            }
            if let Some(m) = meta_val.get("lastUsedModel").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
                if m == "default" {
                    info.model = Some("Auto".to_string());
                } else {
                    info.model = Some(m.to_string());
                }
            }
        }
    }

    // 若尚无 workspace_path，尝试扫描 blobs 表中的 `<user_info>`
    if info.workspace_path.is_none() {
        if let Ok(mut stmt) = conn.prepare("SELECT data FROM blobs LIMIT 50") {
            if let Ok(mut rows) = stmt.query([]) {
                while let Ok(Some(row)) = rows.next() {
                    let bytes: Vec<u8> = row.get(0).unwrap_or_default();
                    let text = String::from_utf8_lossy(&bytes);
                    if let Some(ws) = extract_workspace_path_from_user_info(&text) {
                        info.workspace_path = Some(ws);
                        break;
                    }
                }
            }
        }
    }

    info
}

fn decode_hex_or_raw(raw: &str) -> String {
    if raw.len() % 2 == 0 && raw.chars().all(|c| c.is_ascii_hexdigit()) {
        if let Some(bytes) = decode_hex(raw) {
            if let Ok(s) = String::from_utf8(bytes) {
                return s;
            }
        }
    }
    raw.to_string()
}

fn decode_hex(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() / 2);
    let bytes = s.as_bytes();
    for chunk in bytes.chunks(2) {
        let hi = hex_nibble(chunk[0])?;
        let lo = hex_nibble(chunk[1])?;
        out.push((hi << 4) | lo);
    }
    Some(out)
}

fn hex_nibble(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 子代理发现与关联
// ─────────────────────────────────────────────────────────────────────────────

/// 判断该路径是否为子代理会话（位于 `<parentId>/subagents/<subagentId>.jsonl`）。
pub(crate) fn is_subagent_session(file_path: &str) -> bool {
    let path = Path::new(file_path);
    parent_id_for_subagent(path).is_some()
}

/// 获取子代理会话的父 Session ID。
pub(crate) fn parent_id_for_subagent(path: &Path) -> Option<String> {
    let subagents_dir = path.parent()?;
    if subagents_dir.file_name().and_then(|n| n.to_str()) != Some("subagents") {
        return None;
    }
    Some(
        subagents_dir
            .parent()?
            .file_name()?
            .to_string_lossy()
            .to_string(),
    )
}

/// 获取某个主会话目录下的全部子代理转录文件路径。
pub(crate) fn subagent_paths_under(session_dir: &Path) -> Vec<PathBuf> {
    let subagents_dir = session_dir.join("subagents");
    if !subagents_dir.is_dir() {
        return Vec::new();
    }
    let Ok(entries) = std::fs::read_dir(&subagents_dir) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("jsonl"))
        .collect()
}

// ─────────────────────────────────────────────────────────────────────────────
// 转录单行与消息解析
// ─────────────────────────────────────────────────────────────────────────────

fn extract_text_from_content_value(content_val: Option<&Value>) -> String {
    match content_val {
        Some(Value::String(s)) => {
            if s.trim_start().starts_with('[') {
                if let Ok(arr) = serde_json::from_str::<Vec<Value>>(s) {
                    return extract_text_from_parts(&arr);
                }
            }
            s.clone()
        }
        Some(Value::Array(arr)) => extract_text_from_parts(arr),
        _ => String::new(),
    }
}

fn extract_text_from_parts(arr: &[Value]) -> String {
    let mut chunks = Vec::new();
    for item in arr {
        if item.get("type").and_then(|v| v.as_str()) != Some("text") {
            continue;
        }
        let text = item.get("text").and_then(|v| v.as_str()).unwrap_or("");
        if text.trim() == "[REDACTED]" {
            continue;
        }
        if !text.is_empty() {
            chunks.push(text.to_string());
        }
    }
    chunks.join("\n")
}

fn parse_content_parts_array(content_val: Option<&Value>) -> Vec<Value> {
    match content_val {
        Some(Value::Array(arr)) => arr.clone(),
        Some(Value::String(s)) if s.trim_start().starts_with('[') => {
            serde_json::from_str::<Vec<Value>>(s).unwrap_or_default()
        }
        _ => Vec::new(),
    }
}

/// 从转录单行解析出一条 `ChatMessage`（若该行包含有效用户或助手内容）。
pub(crate) fn parse_transcript_line(
    line: &str,
    timestamp: &str,
    current_model: Option<&str>,
) -> Option<ChatMessage> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    let entry: Value = serde_json::from_str(trimmed).ok()?;
    let role = entry.get("role").and_then(|v| v.as_str()).unwrap_or("");
    let content_val = entry.get("message").and_then(|m| m.get("content"));

    match role {
        "user" => {
            let raw_text = extract_text_from_content_value(content_val);
            let cleaned = clean_user_content(&raw_text);
            if cleaned.is_empty() {
                return None;
            }
            Some(ChatMessage {
                role: "user".to_string(),
                timestamp: timestamp.to_string(),
                model: None,
                token_usage: None,
                content_parts: vec![ContentPart::Text { text: cleaned }],
                is_meta: false,
                uuid: None,
            })
        }
        "assistant" => {
            let mut parts = Vec::new();
            let raw_text = extract_text_from_content_value(content_val);
            let cleaned = strip_redacted(&raw_text);

            if let Some(thinking) = extract_think_content(&cleaned) {
                parts.push(ContentPart::Thinking { thinking });
            }

            let visible = strip_think_tags(&cleaned);
            if !visible.is_empty() {
                parts.push(ContentPart::Text { text: visible });
            }

            for part_val in parse_content_parts_array(content_val) {
                if part_val.get("type").and_then(|t| t.as_str()) == Some("tool_use") {
                    let raw_name = part_val.get("name").and_then(|n| n.as_str()).unwrap_or("tool");
                    let tool_id = part_val
                        .get("id")
                        .or_else(|| part_val.get("tool_use_id"))
                        .and_then(|v| v.as_str())
                        .map(str::to_string);
                    let raw_input = part_val.get("input").cloned().unwrap_or(Value::Null);
                    let canonical = map_tool_name(raw_name);
                    let remapped = remap_tool_args(canonical, &raw_input);
                    let summary = cursor_tool_summary(canonical, &remapped);

                    parts.push(ContentPart::ToolUse {
                        summary,
                        tool_name: canonical.to_string(),
                        input: serde_json::to_string(&remapped).unwrap_or_default(),
                        tool_use_id: tool_id,
                    });
                }
            }

            if parts.is_empty() {
                return None;
            }

            Some(ChatMessage {
                role: "assistant".to_string(),
                timestamp: timestamp.to_string(),
                model: current_model.map(str::to_string),
                token_usage: None,
                content_parts: parts,
                is_meta: false,
                uuid: None,
            })
        }
        _ => None,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 会话级主解析入口
// ─────────────────────────────────────────────────────────────────────────────

pub(crate) fn parse_session_file(file_path: &str) -> AppResult<Vec<ChatMessage>> {
    let result = parse_session_file_with_offset(file_path, 0, false)?;
    Ok(result.messages)
}

pub(crate) fn parse_session_file_with_offset(
    file_path: &str,
    start_offset: u64,
    _skip_sidechain: bool,
) -> AppResult<SessionLoadResult> {
    let path = Path::new(file_path);
    let session_id = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();

    let store_info = read_store_db_info(&session_id, path);
    let timestamp = store_info
        .created_at_secs
        .map(|sec| chrono::DateTime::from_timestamp(sec, 0).map(|dt| dt.to_rfc3339()).unwrap_or_default())
        .or_else(|| {
            std::fs::metadata(path)
                .ok()?
                .modified()
                .ok()?
                .duration_since(std::time::UNIX_EPOCH)
                .ok()
                .map(|dur| {
                    chrono::DateTime::from_timestamp(dur.as_secs() as i64, 0)
                        .map(|dt| dt.to_rfc3339())
                        .unwrap_or_default()
                })
        })
        .unwrap_or_default();

    let mut file = File::open(path)?;
    let file_size = file.metadata()?.len();
    if start_offset > 0 {
        file.seek(SeekFrom::Start(start_offset))?;
    }
    let reader = BufReader::with_capacity(128 * 1024, file);

    let mut messages = Vec::new();
    let mut subagent_map = HashMap::new();

    // 发现同会话目录下的子代理
    if let Some(parent_dir) = path.parent() {
        for sub_path in subagent_paths_under(parent_dir) {
            let sub_id = sub_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            let first_msg = read_first_user_message(&sub_path.to_string_lossy()).unwrap_or_default();
            subagent_map.insert(
                sub_id,
                SubagentInfo {
                    file_path: sub_path.to_string_lossy().to_string(),
                    label: first_msg,
                },
            );
        }
    }

    for line in reader.lines().flatten() {
        if let Some(msg) = parse_transcript_line(&line, &timestamp, store_info.model.as_deref()) {
            messages.push(msg);
        }
    }

    Ok(SessionLoadResult {
        messages,
        offset: file_size,
        subagent_map,
    })
}

pub(crate) fn parse_session_streaming(
    file_path: &str,
    skip_sidechain: bool,
    mut on_batch: &mut dyn FnMut(Vec<ChatMessage>) -> bool,
) -> AppResult<(u64, HashMap<String, SubagentInfo>)> {
    let result = parse_session_file_with_offset(file_path, 0, skip_sidechain)?;
    let mut emitter = BatchEmitter::new();
    for msg in result.messages {
        emitter.push(msg);
        if !emitter.maybe_flush(&mut on_batch) {
            break;
        }
    }
    emitter.flush_remaining(&mut on_batch);
    Ok((result.offset, result.subagent_map))
}

pub(crate) fn parse_session_incremental(
    file_path: &str,
    offset: u64,
    skip_sidechain: bool,
) -> AppResult<SessionLoadResult> {
    parse_session_file_with_offset(file_path, offset, skip_sidechain)
}

pub(crate) fn parse_from_content(content: &str) -> Vec<ChatMessage> {
    let mut messages = Vec::new();
    let timestamp = String::new();
    for line in content.lines() {
        if let Some(msg) = parse_transcript_line(line, &timestamp, None) {
            messages.push(msg);
        }
    }
    messages
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
        let Ok(entry) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };
        if entry.get("role").and_then(|v| v.as_str()) == Some("user") {
            let content_val = entry.get("message").and_then(|m| m.get("content"));
            let text = extract_text_from_content_value(content_val);
            let cleaned = clean_user_content(&text);
            if !cleaned.is_empty() {
                return Some(cleaned);
            }
        }
    }
    None
}

pub(crate) fn read_session_id(file_path: &str) -> Option<String> {
    let path = Path::new(file_path);
    path.file_stem().and_then(|s| s.to_str()).map(ToString::to_string)
}

pub(crate) fn read_project_path(file_path: &str) -> Option<String> {
    let path = Path::new(file_path);
    let session_id = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let store_info = read_store_db_info(session_id, path);
    if let Some(ws) = store_info.workspace_path {
        if !ws.trim().is_empty() {
            return Some(ws);
        }
    }

    // 从转录文件中的 `<user_info>` 兜底提取
    if let Ok(file) = File::open(path) {
        let reader = BufReader::new(file);
        for line in reader.lines().flatten().take(20) {
            if let Some(ws) = extract_workspace_path_from_user_info(&line) {
                if !ws.trim().is_empty() {
                    return Some(ws);
                }
            }
        }
    }

    // 目录名解码兜底
    project_path_from_transcript_path(path).filter(|p| !p.trim().is_empty())
}

pub(crate) fn scan_session_metadata_only(file_path: &Path) -> Option<SessionListMetadata> {
    let path_str = file_path.to_string_lossy().to_string();
    let session_id = file_path.file_stem().and_then(|s| s.to_str())?.to_string();
    let file_metadata = std::fs::metadata(file_path).ok()?;
    let file_size = file_metadata.len();

    let store_info = read_store_db_info(&session_id, file_path);
    let project_path = store_info
        .workspace_path
        .or_else(|| read_project_path(&path_str));

    let first_user_message = read_first_user_message(&path_str);
    let timestamp = store_info
        .created_at_secs
        .map(|sec| chrono::DateTime::from_timestamp(sec, 0).map(|dt| dt.to_rfc3339()).unwrap_or_default())
        .or_else(|| {
            file_metadata
                .modified()
                .ok()?
                .duration_since(std::time::UNIX_EPOCH)
                .ok()
                .map(|dur| {
                    chrono::DateTime::from_timestamp(dur.as_secs() as i64, 0)
                        .map(|dt| dt.to_rfc3339())
                        .unwrap_or_default()
                })
        });

    Some(SessionListMetadata {
        session_id,
        project_path,
        title: store_info.title,
        first_user_message,
        first_timestamp: timestamp.clone(),
        last_timestamp: timestamp,
        git_branch: String::new(),
        file_size,
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// 全文检索文档抽取
// ─────────────────────────────────────────────────────────────────────────────

pub(crate) fn extract_cursor_role_text(entry: &Value) -> Option<(String, String)> {
    let role = entry.get("role").and_then(|v| v.as_str())?.to_string();
    let content_val = entry.get("message").and_then(|m| m.get("content"));
    let raw = extract_text_from_content_value(content_val);
    let text = match role.as_str() {
        "user" => clean_user_content(&raw),
        "assistant" => strip_think_tags(&strip_redacted(&raw)),
        _ => return None,
    };
    if text.is_empty() {
        return None;
    }
    Some((role, text))
}

pub(crate) fn search_docs_from_text(content: &[u8]) -> Option<Vec<SearchDocument>> {
    crate::parser::shared::scan_search_docs_from_text(content, extract_cursor_role_text)
}

pub(crate) fn scan_search_docs(
    path: &Path,
    on_progress: &mut dyn FnMut(SearchScanProgress),
) -> Option<Vec<SearchDocument>> {
    crate::parser::shared::scan_search_docs_with_extractor(path, extract_cursor_role_text, on_progress)
}

// ─────────────────────────────────────────────────────────────────────────────
// 单元测试
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleans_user_query_and_ignores_user_info() {
        let input = "<user_info>\nWorkspace Path: /tmp/ws\n</user_info>\n<user_query>Hello Agent</user_query>";
        assert_eq!(clean_user_content(input), "Hello Agent");

        let system_only = "<user_info>\nSome info\n</user_info>";
        assert_eq!(clean_user_content(system_only), "");
    }

    #[test]
    fn extracts_thinking_block_and_strips_from_visible() {
        let text = "<think>Pondering problem</think>\nHere is the answer.";
        assert_eq!(extract_think_content(text), Some("Pondering problem".to_string()));
        assert_eq!(strip_think_tags(text), "Here is the answer.");
    }

    #[test]
    fn parses_user_assistant_and_tool_call() {
        let line_user = r#"{"role":"user","message":{"content":[{"type":"text","text":"<user_query>Read file</user_query>"}]}}"#;
        let line_asst = r#"{"role":"assistant","message":{"content":[{"type":"text","text":"<think>reading</think>done"},{"type":"tool_use","name":"Read","id":"call_1","input":{"path":"/foo/bar.rs"}}]}}"#;

        let user_msg = parse_transcript_line(line_user, "2026-01-01T00:00:00Z", None).unwrap();
        assert_eq!(user_msg.role, "user");
        assert_eq!(user_msg.content_parts.len(), 1);
        match &user_msg.content_parts[0] {
            ContentPart::Text { text } => assert_eq!(text, "Read file"),
            _ => panic!("expected text part"),
        }

        let asst_msg = parse_transcript_line(line_asst, "2026-01-01T00:00:00Z", Some("claude-3.5-sonnet")).unwrap();
        assert_eq!(asst_msg.role, "assistant");
        assert_eq!(asst_msg.model.as_deref(), Some("claude-3.5-sonnet"));
        assert_eq!(asst_msg.content_parts.len(), 3);
        match &asst_msg.content_parts[0] {
            ContentPart::Thinking { thinking } => assert_eq!(thinking, "reading"),
            _ => panic!("expected thinking part"),
        }
        match &asst_msg.content_parts[1] {
            ContentPart::Text { text } => assert_eq!(text, "done"),
            _ => panic!("expected text part"),
        }
        match &asst_msg.content_parts[2] {
            ContentPart::ToolUse { summary, tool_name, tool_use_id, .. } => {
                assert_eq!(tool_name, "Read");
                assert_eq!(summary, "[Read: bar.rs]");
                assert_eq!(tool_use_id.as_deref(), Some("call_1"));
            }
            _ => panic!("expected tool_use part"),
        }
    }

    #[test]
    fn decodes_sanitized_project_key() {
        let decoded = decode_project_key("Users-myuser-code-repo");
        assert_eq!(decoded, "/Users/myuser/code/repo");
    }

    #[test]
    fn remaps_various_tool_payloads_accurately() {
        // Edit with old_str / new_str
        let edit_args = serde_json::json!({
            "file_path": "/src/main.rs",
            "old_str": "let a = 1;",
            "new_str": "let a = 2;"
        });
        let remapped_edit = remap_tool_args("Edit", &edit_args);
        assert_eq!(remapped_edit.get("old_string").and_then(|v| v.as_str()), Some("let a = 1;"));
        assert_eq!(remapped_edit.get("new_string").and_then(|v| v.as_str()), Some("let a = 2;"));
        assert_eq!(cursor_tool_summary("Edit", &remapped_edit), "[Edit: main.rs]");

        // ApplyPatch with file_path extraction
        let patch_str = serde_json::Value::String("*** Update File: src/lib.rs\n@@ -1,2 +1,2 @@\n".to_string());
        let remapped_patch = remap_tool_args("ApplyPatch", &patch_str);
        assert_eq!(remapped_patch.get("file_path").and_then(|v| v.as_str()), Some("src/lib.rs"));
        assert_eq!(cursor_tool_summary("ApplyPatch", &remapped_patch), "[ApplyPatch: lib.rs]");

        // Glob and Grep
        let glob_args = serde_json::json!({ "glob_pattern": "*.ts", "target_directory": "src" });
        let remapped_glob = remap_tool_args("Glob", &glob_args);
        assert_eq!(cursor_tool_summary("Glob", &remapped_glob), "[Glob: *.ts]");

        let grep_args = serde_json::json!({ "pattern": "fn main" });
        let remapped_grep = remap_tool_args("Grep", &grep_args);
        assert_eq!(cursor_tool_summary("Grep", &remapped_grep), "[Grep: fn main]");
    }
}
