//! 各 CLI 解析器共用的文件读取与文本清洗工具。
//!
//! 依赖方向：parser → session。

use serde_json::Value;
use std::fs::File;
use std::io::{BufRead, BufReader, Read as _, Seek, SeekFrom};
use std::path::Path;

#[derive(Debug, Clone)]
pub struct SearchDocument {
    pub message_index: usize,
    #[allow(dead_code)]
    pub role: String,
    pub search_text: String,
}

#[derive(Debug, Clone, Copy)]
pub struct SearchScanProgress {
    pub bytes_read: u64,
    pub file_size: u64,
}

/// 读取文件的第一行内容（供快速元数据识别/类型探针使用）。
pub(crate) fn read_first_line_from_file(file_path: &str) -> Option<String> {
    let file = File::open(file_path).ok()?;
    let mut reader = BufReader::new(file);
    let mut line = String::new();
    if reader.read_line(&mut line).ok()? > 0 {
        Some(line)
    } else {
        None
    }
}

/// 从文件尾部倒序检索最新的 timestamp（跳过不包含 timestamp 的元数据快照）。
pub(crate) fn read_last_timestamp_from_file(file: &mut File, file_size: u64) -> Option<String> {
    if file_size == 0 {
        return None;
    }

    let chunk_size: u64 = 65536;
    let max_search: u64 = 2 * 1024 * 1024; // 搜索至多 2MB
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
        // 倒序读取时，块最前面的第一行极大概率是被截断的，需要保存给下一个块补全
        if offset > 0 && !combined.starts_with('\n') && !lines.is_empty() {
            overlap = lines.remove(0).to_string();
        } else {
            overlap.clear();
        }

        for line in lines.into_iter().rev() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            // 💡 核心前缀过滤：直接跳过所有显然不包含 timestamp 的元数据快照！
            // 不调用昂贵的 serde_json，实现 0 开销穿越无用快照区
            if !trimmed.contains("\"timestamp\"") {
                continue;
            }

            let entry: Value = match serde_json::from_str(trimmed) {
                Ok(v) => v,
                Err(_) => continue,
            };
            if let Some(ts) = entry.get("timestamp").and_then(|v| v.as_str()) {
                return Some(ts.to_string());
            }
        }
    }
    None
}

/// 逐行扫描文件，用 extractor 抽取可搜索文本，并报告读取进度。
pub(crate) fn scan_search_docs_with_extractor<F, E>(
    file_path: &Path,
    extractor: E,
    mut on_progress: F,
) -> Option<Vec<SearchDocument>>
where
    F: FnMut(SearchScanProgress),
    E: Fn(&Value) -> Option<(String, String)>,
{
    let file = File::open(file_path).ok()?;
    let file_size = file.metadata().ok().map(|m| m.len()).unwrap_or(0);
    let mut reader = BufReader::new(file);
    let mut docs = Vec::new();
    let mut message_index = 0usize;
    let mut bytes_read = 0u64;
    let mut line = String::new();

    loop {
        line.clear();
        let read = match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(n) => n,
            Err(_) => continue,
        };
        bytes_read = bytes_read.saturating_add(read as u64);
        on_progress(SearchScanProgress {
            bytes_read,
            file_size,
        });

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let entry: Value = match serde_json::from_str(trimmed) {
            Ok(entry) => entry,
            Err(_) => continue,
        };

        if let Some((role, text)) = extractor(&entry) {
            docs.push(SearchDocument {
                message_index,
                role,
                search_text: text,
            });
            message_index += 1;
        }
    }

    on_progress(SearchScanProgress {
        bytes_read: file_size,
        file_size,
    });

    if docs.is_empty() {
        None
    } else {
        Some(docs)
    }
}

/// 从内存内容逐行抽取可搜索文本（归档兜底用）。
pub(crate) fn scan_search_docs_from_text<E>(content: &[u8], extractor: E) -> Option<Vec<SearchDocument>>
where
    E: Fn(&Value) -> Option<(String, String)>,
{
    let text = String::from_utf8_lossy(content);
    let mut docs = Vec::new();
    let mut message_index = 0usize;

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let entry: Value = match serde_json::from_str(trimmed) {
            Ok(entry) => entry,
            Err(_) => continue,
        };

        if let Some((role, text)) = extractor(&entry) {
            docs.push(SearchDocument {
                message_index,
                role,
                search_text: text,
            });
            message_index += 1;
        }
    }

    if docs.is_empty() {
        None
    } else {
        Some(docs)
    }
}

/// 截断预览文本：去除首尾空白，超长时截断并追加省略号。
pub(crate) fn truncate_preview_text(text: &str, max_chars: usize) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }

    let truncated: String = trimmed.chars().take(max_chars).collect();
    Some(if trimmed.chars().count() > max_chars {
        format!("{}...", truncated)
    } else {
        truncated
    })
}

/// 在会话结束时追加，倒序首个命中即最新值（last-wins）。
pub(crate) fn read_typed_titles_from_tail(
    file: &mut File,
    file_size: u64,
) -> (Option<String>, Option<String>) {
    let mut custom_title = None;
    let mut ai_title = None;
    if file_size == 0 {
        return (custom_title, ai_title);
    }

    let chunk_size: u64 = 65536;
    let max_search: u64 = 2 * 1024 * 1024; // 与 read_last_timestamp_from_file 一致
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
                Err(_) => return (custom_title, ai_title),
            }
        }

        let chunk_text = String::from_utf8_lossy(&buf[..total_read]);
        let combined = format!("{}{}", chunk_text, overlap);

        let mut lines: Vec<&str> = combined.lines().collect();
        // 倒序读取时，块最前面的第一行极大概率是被截断的，保存给下一个块补全
        if offset > 0 && !combined.starts_with('\n') && !lines.is_empty() {
            overlap = lines.remove(0).to_string();
        } else {
            overlap.clear();
        }

        for line in lines.into_iter().rev() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.len() > 256 * 1024 {
                continue;
            }

            let is_custom = trimmed.contains("\"custom-title\"");
            let is_ai = trimmed.contains("\"ai-title\"");
            if !is_custom && !is_ai {
                continue;
            }

            let entry: Value = match serde_json::from_str(trimmed) {
                Ok(entry) => entry,
                Err(_) => continue,
            };
            let entry_type = entry.get("type").and_then(|v| v.as_str()).unwrap_or("");
            if is_custom && entry_type == "custom-title" && custom_title.is_none() {
                custom_title = crate::title_resolver::extract_typed_title(
                    &entry,
                    "customTitle",
                    "custom_title",
                );
            }
            if is_ai && entry_type == "ai-title" && ai_title.is_none() {
                ai_title =
                    crate::title_resolver::extract_typed_title(&entry, "aiTitle", "ai_title");
            }

            if custom_title.is_some() && ai_title.is_some() {
                return (custom_title, ai_title);
            }
        }
    }

    (custom_title, ai_title)
}

/// 解析 OpenAI 风格工具调用入参（arguments/input 可为 JSON 对象、数组或 JSON 字符串）。
pub(crate) fn codex_tool_input_value(payload: &Value) -> Value {
    for key in ["arguments", "input"] {
        if let Some(value) = payload.get(key) {
            if value.is_object() || value.is_array() {
                return value.clone();
            }
            if let Some(text) = value.as_str() {
                let trimmed = text.trim();
                if trimmed.is_empty() {
                    continue;
                }
                if let Ok(parsed) = serde_json::from_str::<Value>(trimmed) {
                    return parsed;
                }
                return serde_json::json!({ key: trimmed });
            }
        }
    }

    Value::Object(serde_json::Map::new())
}

/// 判定工具结果是否错误：is_error 标志、error 字段、status 或内容特征。
pub(crate) fn codex_tool_result_is_error(payload: &Value, content: &str) -> bool {
    if payload
        .get("is_error")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        return true;
    }
    if payload.get("error").is_some() {
        return true;
    }
    if let Some(status) = payload.get("status").and_then(|v| v.as_str()) {
        let lower = status.to_lowercase();
        if lower.contains("error") || lower.contains("fail") {
            return true;
        }
    }
    let lower = content.to_lowercase();
    lower.contains("error:") || lower.contains("failed") || lower.contains("traceback")
}

/// 将 JSON 值递归展开为可读文本片段。
pub(crate) fn codex_value_to_text(value: &Value) -> String {
    let mut parts = Vec::new();
    collect_codex_text_fragments(value, &mut parts);
    parts.join("\n")
}

fn collect_codex_text_fragments(value: &Value, parts: &mut Vec<String>) {
    collect_codex_text_fragments_inner(value, parts, 64);
}

fn collect_codex_text_fragments_inner(value: &Value, parts: &mut Vec<String>, max_depth: usize) {
    if max_depth == 0 {
        return;
    }
    match value {
        Value::String(text) => {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                parts.push(trimmed.to_string());
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_codex_text_fragments_inner(item, parts, max_depth - 1);
            }
        }
        Value::Object(map) => {
            for key in [
                "text", "content", "output", "result", "summary", "stderr", "stdout", "message",
            ] {
                if let Some(inner) = map.get(key) {
                    collect_codex_text_fragments_inner(inner, parts, max_depth - 1);
                }
            }
        }
        _ => {}
    }
}
