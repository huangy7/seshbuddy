use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A captured HTTP traffic record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrafficRecord {
    pub id: String,
    pub timestamp: String,
    pub method: String,
    pub path: String,
    pub req_headers: Option<String>,
    pub req_body: Option<String>,
    pub req_size: i64,
    pub status: Option<i32>,
    pub res_headers: Option<String>,
    pub res_body: Option<String>,
    /// 上游强制压缩且本进程未启用解压特性时的 `content-encoding`（正文因此未落库）。
    /// 只是事实记录：提示句由前端按当前语言渲染，这里不写任何面向用户的文案。
    pub compression: Option<String>,
    /// 我们自己的包装句所对应的**事实**（今天只有 `"upstream"`：上游请求失败）。
    /// 与 `compression` 同一条原则：**只是事实记录，提示句由前端按当前语言渲染，
    /// 这里不写任何面向用户的文案**。`res_body` 则装**第三方错误的原文**，前端原样展示。
    pub error_kind: Option<String>,
    pub res_size: i64,
    pub duration_ms: i64,
    pub session_id: Option<String>,
    pub cli_id: String,
}

/// Extract session id from request body for Claude / Codex style payloads.
pub fn extract_session_id(req_body: &str) -> Option<String> {
    let val: serde_json::Value = serde_json::from_str(req_body).ok()?;

    if let Some(user_id) = val
        .get("metadata")
        .and_then(|metadata| metadata.get("user_id"))
        .and_then(|v| v.as_str())
    {
        if user_id.trim_start().starts_with('{') {
            let inner: serde_json::Value = serde_json::from_str(user_id).ok()?;
            if let Some(session_id) = inner.get("session_id").and_then(|v| v.as_str()) {
                return Some(session_id.to_string());
            }
        }

        let marker = "_session_";
        if let Some(pos) = user_id.find(marker) {
            let uuid_part = &user_id[pos + marker.len()..];
            if uuid_part.len() >= 36 && uuid_part.as_bytes()[8] == b'-' {
                return Some(uuid_part[..36].to_string());
            }
        }
    }

    find_string_by_keys(&val, &["session_id", "conversation_id"])
}

fn find_string_by_keys(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
    match value {
        serde_json::Value::Object(map) => {
            for key in keys {
                if let Some(found) = map.get(*key).and_then(|v| v.as_str()) {
                    let trimmed = found.trim();
                    if !trimmed.is_empty() {
                        return Some(trimmed.to_string());
                    }
                }
            }
            for nested in map.values() {
                if let Some(found) = find_string_by_keys(nested, keys) {
                    return Some(found);
                }
            }
            None
        }
        serde_json::Value::Array(items) => items
            .iter()
            .find_map(|item| find_string_by_keys(item, keys)),
        _ => None,
    }
}

const SENSITIVE_HEADERS: &[&str] = &["authorization", "x-api-key", "anthropic-api-key"];

/// Sanitize headers by redacting sensitive values.
pub fn sanitize_headers(headers: &HashMap<String, String>) -> String {
    let sanitized: HashMap<&str, &str> = headers
        .iter()
        .map(|(k, v)| {
            let key = k.as_str();
            let val = if SENSITIVE_HEADERS
                .iter()
                .any(|s| key.eq_ignore_ascii_case(s))
            {
                "[REDACTED]"
            } else {
                v.as_str()
            };
            (key, val)
        })
        .collect();
    serde_json::to_string(&sanitized).unwrap_or_default()
}

/// Extract headers from reqwest response into a sanitized JSON string.
pub fn headers_to_map(headers: &reqwest::header::HeaderMap) -> HashMap<String, String> {
    headers
        .iter()
        .map(|(k, v)| (k.as_str().to_string(), v.to_str().unwrap_or("").to_string()))
        .collect()
}

/// Extract headers from hyper request into a HashMap.
pub fn hyper_headers_to_map(headers: &hyper::header::HeaderMap) -> HashMap<String, String> {
    headers
        .iter()
        .map(|(k, v)| (k.as_str().to_string(), v.to_str().unwrap_or("").to_string()))
        .collect()
}
