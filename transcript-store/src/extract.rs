//! 轻量流式 jsonl 会话提取器：只抽 user/assistant 纯文本。
//! 与搜索索引解析器完全独立（投影分离设计，见 spec 2026-08-11）。

use std::io::BufRead;

pub const MAX_MESSAGE_CHARS: usize = 2000;

/// 会话 jsonl 的格式（规则与主进程 multi_cli_session 的多 CLI 解析一致）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliFormat {
    Claude,
    Codex,
    Gemini,
    WorkBuddy,
}

impl CliFormat {
    pub fn from_cli_id(id: &str) -> Self {
        match id {
            "codex" => CliFormat::Codex,
            "gemini" => CliFormat::Gemini,
            "workbuddy" => CliFormat::WorkBuddy,
            _ => CliFormat::Claude,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExtractedMessage {
    pub seq: u64,
    pub role: String,
    pub text: String,
}

/// 提取结果。追加模式下 messages 只含新增消息，raw_lines/message_lines 是全量口径的游标。
#[derive(Debug, Default)]
pub struct Extraction {
    pub messages: Vec<ExtractedMessage>,
    /// 最后一条 user/assistant 消息事件行的序号（无消息时为 base_seq）
    pub message_lines: u64,
    /// 文件总原始行数（含 skip_raw_lines）；口径为「以换行符结尾的完整行数」，
    /// 末尾无 `\n` 的半行不计入（下轮文件补完后它会作为完整行被重新读到）
    pub raw_lines: u64,
}

fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    format!("{}…", s.chars().take(max).collect::<String>())
}

fn join_text_parts(parts: &[serde_json::Value]) -> String {
    parts
        .iter()
        .filter(|p| p.get("type").and_then(|t| t.as_str()) == Some("text"))
        .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

/// 从 claude 消息事件行提取 (role, text)；无纯文本返回 None（序号仍被调用方占用）。
fn extract_text_claude(parsed: &serde_json::Value) -> Option<(String, String)> {
    let event_type = parsed.get("type")?.as_str()?;
    let content = parsed.get("message").and_then(|m| m.get("content"));
    match (event_type, content) {
        ("user", Some(serde_json::Value::String(s))) => {
            let t = s.trim();
            if t.is_empty() { None } else { Some(("user".to_string(), s.clone())) }
        }
        ("user", Some(serde_json::Value::Array(parts))) => {
            let text = join_text_parts(parts);
            if text.is_empty() { None } else { Some(("user".to_string(), text)) }
        }
        ("assistant", Some(serde_json::Value::Array(parts))) => {
            let text = join_text_parts(parts);
            if text.is_empty() { None } else { Some(("assistant".to_string(), text)) }
        }
        _ => None,
    }
}

/// codex：response_item / payload.message / role∈{user,assistant}；
/// content 为字符串或 [{type: input_text|output_text|text, text}] 数组。
fn extract_text_codex(parsed: &serde_json::Value) -> Option<(String, String)> {
    let payload = parsed.get("payload")?;
    let role = payload.get("role").and_then(|v| v.as_str())?;
    if role != "user" && role != "assistant" {
        return None;
    }
    let content = payload.get("content")?;
    if let Some(s) = content.as_str() {
        let t = s.trim();
        return if t.is_empty() { None } else { Some((role.to_string(), t.to_string())) };
    }
    let items = content.as_array()?;
    let text = items
        .iter()
        .filter(|i| {
            matches!(
                i.get("type").and_then(|t| t.as_str()),
                Some("input_text") | Some("output_text") | Some("text")
            )
        })
        .filter_map(|i| i.get("text").and_then(|t| t.as_str()))
        .map(|t| t.trim())
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    if text.is_empty() { None } else { Some((role.to_string(), text)) }
}

/// gemini：元数据行（$set 或无 type 的 sessionId 行）跳过；
/// user → displayContent/content 数组的 text 项；gemini(assistant) → content 字符串。
fn extract_text_gemini(parsed: &serde_json::Value) -> Option<(String, String)> {
    let entry_type = parsed.get("type").and_then(|v| v.as_str())?;
    let join_items = |key: &str| -> String {
        parsed
            .get(key)
            .and_then(|v| v.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|i| i.get("text").and_then(|t| t.as_str()))
                    .map(|t| t.trim())
                    .filter(|t| !t.is_empty())
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default()
    };
    match entry_type {
        "user" => {
            let mut text = join_items("displayContent");
            if text.is_empty() {
                text = join_items("content");
            }
            if text.is_empty() { None } else { Some(("user".to_string(), text)) }
        }
        "gemini" => {
            let text = parsed
                .get("content")
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .unwrap_or_default();
            if text.is_empty() { None } else { Some(("assistant".to_string(), text)) }
        }
        _ => None,
    }
}

/// workbuddy：type=="message" + 顶层 role；user → content[] 的 input_text 块
/// （真实提问在最后一个 <user_query> 内；<cb_summary> 压缩行跳过）；
/// assistant → output_text 块。
fn extract_text_workbuddy(parsed: &serde_json::Value) -> Option<(String, String)> {
    let role = parsed.get("role").and_then(|v| v.as_str())?;
    let block_type = match role {
        "user" => "input_text",
        "assistant" => "output_text",
        _ => return None,
    };
    let items = parsed.get("content").and_then(|v| v.as_array())?;
    let text = items
        .iter()
        .filter(|i| i.get("type").and_then(|t| t.as_str()) == Some(block_type))
        .filter_map(|i| i.get("text").and_then(|t| t.as_str()))
        .map(|t| t.trim())
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    if text.is_empty() {
        return None;
    }
    if role == "assistant" {
        return Some(("assistant".to_string(), text));
    }
    if text.trim_start().starts_with("<cb_summary") {
        return None;
    }
    // 最后一个 <user_query> 是真实提问
    let cleaned = match (text.rfind("<user_query>"), text.rfind("</user_query>")) {
        (Some(s), Some(e)) if e > s => text[s + "<user_query>".len()..e].trim().to_string(),
        _ => text,
    };
    if cleaned.is_empty() {
        None
    } else {
        Some(("user".to_string(), cleaned))
    }
}

/// 判断一行是否为「消息事件行」（占 seq 序号），并按格式提取文本。
/// 返回 None = 非消息行（不占序号）；Some((seq 占位, Option<(role, text)>))。
fn classify_line(format: CliFormat, parsed: &serde_json::Value) -> Option<Option<(String, String)>> {
    match format {
        CliFormat::Claude => {
            let t = parsed.get("type").and_then(|v| v.as_str())?;
            if t != "user" && t != "assistant" {
                return None;
            }
            Some(extract_text_claude(parsed))
        }
        CliFormat::Codex => {
            if parsed.get("type").and_then(|v| v.as_str()) != Some("response_item") {
                return None;
            }
            let payload = parsed.get("payload")?;
            if payload.get("type").and_then(|v| v.as_str()) != Some("message") {
                return None;
            }
            let role = payload.get("role").and_then(|v| v.as_str())?;
            if role != "user" && role != "assistant" {
                return None;
            }
            Some(extract_text_codex(parsed))
        }
        CliFormat::Gemini => {
            // 元数据行不占序号
            if parsed.get("$set").is_some()
                || (parsed.get("sessionId").is_some() && parsed.get("type").is_none())
            {
                return None;
            }
            let t = parsed.get("type").and_then(|v| v.as_str())?;
            if t != "user" && t != "gemini" {
                return None;
            }
            Some(extract_text_gemini(parsed))
        }
        CliFormat::WorkBuddy => {
            if parsed.get("type").and_then(|v| v.as_str()) != Some("message") {
                return None;
            }
            let role = parsed.get("role").and_then(|v| v.as_str())?;
            if role != "user" && role != "assistant" {
                return None;
            }
            Some(extract_text_workbuddy(parsed))
        }
    }
}

/// 流式提取。skip_raw_lines > 0 时前 N 个原始行不解析（jsonl append-only 前提），
/// 消息序号从 base_seq 续编。
pub fn extract(
    mut reader: impl BufRead,
    format: CliFormat,
    skip_raw_lines: u64,
    base_seq: u64,
) -> Extraction {
    let mut out = Extraction::default();
    let mut seq = base_seq;
    let mut raw = 0u64;
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        // 末尾无 `\n` 的半行（只会出现在 EOF）：不解析也不计入 raw_lines，
        // 否则追加游标会跳过这个位置，该行补完后对应消息永远进不了投影
        if !line.ends_with('\n') {
            break;
        }
        raw += 1;
        if raw <= skip_raw_lines {
            continue;
        }
        let line = line.trim_end_matches(['\n', '\r']);
        let Ok(parsed) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let Some(maybe_text) = classify_line(format, &parsed) else {
            continue;
        };
        seq += 1;
        if let Some((role, text)) = maybe_text {
            out.messages.push(ExtractedMessage {
                seq,
                role,
                text: truncate_chars(&text, MAX_MESSAGE_CHARS),
            });
        }
    }
    out.message_lines = seq;
    out.raw_lines = raw;
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn extracts_user_and_assistant_text_only() {
        let jsonl = concat!(
            r#"{"type":"user","message":{"content":"生成一份周报"}}"#, "\n",
            r#"{"type":"assistant","message":{"content":[{"type":"thinking","thinking":"..."},{"type":"text","text":"好的"}]}}"#, "\n",
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash","input":{"command":"x"}}]}}"#, "\n",
            r#"{"type":"user","message":{"content":[{"type":"tool_result","content":"x"}]}}"#, "\n",
            r#"{"type":"user","message":{"content":[{"type":"text","text":"追问"}]}}"#, "\n",
            r#"{"type":"system","subtype":"init"}"#, "\n",
        );
        let out = extract(Cursor::new(jsonl), CliFormat::Claude, 0, 0);
        assert_eq!(out.messages.len(), 3);
        assert_eq!(out.messages[0], ExtractedMessage { seq: 1, role: "user".into(), text: "生成一份周报".into() });
        assert_eq!(out.messages[1].seq, 2);
        assert_eq!(out.messages[1].role, "assistant");
        assert_eq!(out.messages[1].text, "好的");
        // 第 3 条（纯 tool_use）与第 4 条（纯 tool_result）无文本但占序号 → 第 5 条 seq=5
        assert_eq!(out.messages[2].seq, 5);
        assert_eq!(out.messages[2].text, "追问");
        assert_eq!(out.message_lines, 5);
        assert_eq!(out.raw_lines, 6);
    }

    #[test]
    fn skips_bad_json_lines_without_counting_seq() {
        let jsonl = "not json\n{\"type\":\"user\",\"message\":{\"content\":\"hi\"}}\n";
        let out = extract(Cursor::new(jsonl), CliFormat::Claude, 0, 0);
        assert_eq!(out.messages.len(), 1);
        assert_eq!(out.messages[0].seq, 1);
        assert_eq!(out.raw_lines, 2);
    }

    #[test]
    fn truncates_text_at_2000_chars() {
        let long = "字".repeat(5000);
        let jsonl = format!("{{\"type\":\"user\",\"message\":{{\"content\":\"{}\"}}}}\n", long);
        let out = extract(Cursor::new(jsonl), CliFormat::Claude, 0, 0);
        assert!(out.messages[0].text.chars().count() <= MAX_MESSAGE_CHARS + 1); // +省略号
    }

    #[test]
    fn append_mode_skips_raw_lines_and_continues_seq() {
        let old = "{\"type\":\"user\",\"message\":{\"content\":\"旧\"}}\n";
        let new = "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"新\"}]}}\n";
        let jsonl = format!("{}{}", old, new);
        // skip_raw_lines=1, base_seq=1：旧行不解析，新消息 seq 从 2 开始
        let out = extract(Cursor::new(jsonl), CliFormat::Claude, 1, 1);
        assert_eq!(out.messages.len(), 1);
        assert_eq!(out.messages[0].seq, 2);
        assert_eq!(out.messages[0].text, "新");
        assert_eq!(out.message_lines, 2);
        assert_eq!(out.raw_lines, 2);
    }

    #[test]
    fn eof_half_line_not_counted_and_recovered_after_completion() {
        let full = "{\"type\":\"user\",\"message\":{\"content\":\"一\"}}\n";
        let half = "{\"type\":\"user\",\"message\":{\"content\":\"二\"}}"; // 末尾无换行
        let jsonl = format!("{}{}", full, half);
        let out = extract(Cursor::new(jsonl.clone()), CliFormat::Claude, 0, 0);
        // 半行不解析、不计入 raw_lines（否则补完后追加游标会把它跳丢）
        assert_eq!(out.raw_lines, 1);
        assert_eq!(out.messages.len(), 1);
        assert_eq!(out.message_lines, 1);
        // 源文件补完该行（追加 \n）后走追加提取：skip=1, base_seq=1
        let completed = format!("{}\n", jsonl);
        let out2 = extract(Cursor::new(completed), CliFormat::Claude, out.raw_lines, out.message_lines);
        assert_eq!(out2.messages.len(), 1);
        assert_eq!(out2.messages[0].seq, 2);
        assert_eq!(out2.messages[0].text, "二");
        assert_eq!(out2.raw_lines, 2);
        assert_eq!(out2.message_lines, 2);
    }
}

#[cfg(test)]
mod multi_format_tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn codex_extracts_user_and_assistant_messages() {
        let jsonl = concat!(
            r#"{"timestamp":"t","type":"session_meta","payload":{"id":"x"}}"#, "\n",
            r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"查一下日志"}]}}"#, "\n",
            r#"{"type":"event_msg","payload":{"type":"task_started"}}"#, "\n",
            r#"{"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"看到了"}]}}"#, "\n",
            r#"{"type":"response_item","payload":{"type":"function_call","name":"shell"}}"#, "\n",
        );
        let out = extract(Cursor::new(jsonl), CliFormat::Codex, 0, 0);
        assert_eq!(out.messages.len(), 2);
        assert_eq!(out.messages[0], ExtractedMessage { seq: 1, role: "user".into(), text: "查一下日志".into() });
        assert_eq!(out.messages[1], ExtractedMessage { seq: 2, role: "assistant".into(), text: "看到了".into() });
        assert_eq!(out.raw_lines, 5);
    }

    #[test]
    fn codex_string_content_and_role_filter() {
        let jsonl = concat!(
            r#"{"type":"response_item","payload":{"type":"message","role":"user","content":"字符串内容"}}"#, "\n",
            r#"{"type":"response_item","payload":{"type":"message","role":"developer","content":"不算"}}"#, "\n",
        );
        let out = extract(Cursor::new(jsonl), CliFormat::Codex, 0, 0);
        assert_eq!(out.messages.len(), 1);
        assert_eq!(out.messages[0].text, "字符串内容");
    }

    #[test]
    fn gemini_extracts_user_display_content_and_assistant_text() {
        let jsonl = concat!(
            r#"{"sessionId":"abc"}"#, "\n",
            r#"{"type":"user","displayContent":[{"text":"帮我看下配置"}],"content":[]}"#, "\n",
            r#"{"type":"gemini","content":"好的，配置是这样","toolCalls":[{"name":"run_shell"}]}"#, "\n",
            r#"{"$set":{"x":1}}"#, "\n",
        );
        let out = extract(Cursor::new(jsonl), CliFormat::Gemini, 0, 0);
        assert_eq!(out.messages.len(), 2);
        assert_eq!(out.messages[0], ExtractedMessage { seq: 1, role: "user".into(), text: "帮我看下配置".into() });
        // gemini 助手文本不含工具调用名（干净对话口径）
        assert_eq!(out.messages[1], ExtractedMessage { seq: 2, role: "assistant".into(), text: "好的，配置是这样".into() });
        assert_eq!(out.raw_lines, 4);
    }

    #[test]
    fn workbuddy_extracts_user_query_and_assistant_text() {
        let jsonl = concat!(
            r#"{"timestamp":1786514191337,"type":"message","role":"user","content":[{"type":"input_text","text":"<system-reminder data-role=\"user-context\">ctx</system-reminder><user_query>查下配额</user_query>"}],"sessionId":"s1"}"#, "\n",
            r#"{"timestamp":1786514192000,"type":"function_call","name":"Read","arguments":"{}"}"#, "\n",
            r#"{"timestamp":1786514193000,"type":"message","role":"assistant","content":[{"type":"output_text","text":"看到了"}]}"#, "\n",
            r#"{"timestamp":1786514194000,"type":"message","role":"user","content":[{"type":"input_text","text":"<cb_summary>压缩</cb_summary>"}]}"#, "\n",
            r#"{"timestamp":1786514195000,"type":"ai-title","aiTitle":"标题"}"#, "\n",
        );
        let out = extract(Cursor::new(jsonl), CliFormat::WorkBuddy, 0, 0);
        assert_eq!(out.messages.len(), 2);
        assert_eq!(out.messages[0], ExtractedMessage { seq: 1, role: "user".into(), text: "查下配额".into() });
        assert_eq!(out.messages[1], ExtractedMessage { seq: 2, role: "assistant".into(), text: "看到了".into() });
        assert_eq!(out.raw_lines, 5);
    }

    #[test]
    fn from_cli_id_maps_formats() {
        assert_eq!(CliFormat::from_cli_id("claude"), CliFormat::Claude);
        assert_eq!(CliFormat::from_cli_id("codex"), CliFormat::Codex);
        assert_eq!(CliFormat::from_cli_id("gemini"), CliFormat::Gemini);
        assert_eq!(CliFormat::from_cli_id("workbuddy"), CliFormat::WorkBuddy);
    }
}
