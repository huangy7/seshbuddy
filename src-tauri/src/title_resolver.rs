use regex::Regex;
use std::collections::HashMap;
use std::sync::LazyLock;

static IMAGE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\[Image:[^\]]*\]").unwrap());

static PASTED_TEXT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\[Pasted text #[^\]]+\]").unwrap());

const SYSTEM_TAG_NAMES: &[&str] = &[
    "environment_context",
    "permissions",
    "instructions",
    // 注意：system-reminder 必须排在 system 之前（前缀匹配顺序敏感）
    "system-reminder",
    "system",
    "local-command-stdout",
    "local-command-caveat",
    "command-message",
    "command-name",
    "command-args",
];

pub(crate) struct TitleResolverOptions<'a> {
    pub codex_index_titles: Option<&'a HashMap<String, String>>,
}

/// 检查给定的行（trim后）是否为系统标签开头/结尾或包含系统标签
pub(crate) fn is_system_tag(trimmed: &str) -> bool {
    let t = trimmed.trim();
    if !t.starts_with('<') {
        return false;
    }
    let s = t.trim_start_matches('<').trim_start_matches('/');
    for &tag in SYSTEM_TAG_NAMES {
        if s.get(..tag.len()).map_or(false, |prefix| prefix.eq_ignore_ascii_case(tag)) {
            let rem = &s[tag.len()..];
            if rem.is_empty()
                || rem.starts_with('>')
                || rem.starts_with(' ')
                || rem.starts_with('/')
                || rem.starts_with('\t')
                || rem.starts_with('\n')
                || rem.starts_with(':')
            {
                return true;
            }
        }
    }
    false
}

/// 剔除 `[Image: ...]` 和 `[Pasted text #...]` 占位符
pub(crate) fn strip_image_markers(text: &str) -> String {
    let no_img = IMAGE_RE.replace_all(text, "");
    PASTED_TEXT_RE.replace_all(&no_img, "").to_string()
}

fn is_closing_system_tag(trimmed: &str) -> bool {
    trimmed.trim().starts_with("</")
}

fn is_self_contained_system_tag(trimmed: &str) -> bool {
    let t = trimmed.trim();
    t.starts_with('<') && (t.contains("</") || t.ends_with("/>"))
}

/// 剥离行首的自闭合系统标签（如 `<system>x</system>`），返回其后的残余文本。
/// 无法定位闭合标签时返回 None。
fn strip_leading_self_contained_tag(trimmed: &str) -> Option<&str> {
    if let Some(close_start) = trimmed.find("</") {
        let close_end = trimmed[close_start..].find('>')?;
        Some(trimmed[close_start + close_end + 1..].trim())
    } else if trimmed.ends_with("/>") {
        Some("")
    } else {
        None
    }
}

fn extract_first_sentence(text: &str) -> &str {
    let mut iter = text.char_indices().peekable();
    while let Some((byte_idx, ch)) = iter.next() {
        if ch == '。' || ch == '！' || ch == '？' || ch == '!' || ch == '?' || ch == '\n' {
            let end = byte_idx + ch.len_utf8();
            return &text[..end];
        }
        if ch == '.' {
            match iter.peek() {
                None => return &text[..byte_idx + 1],
                Some((_, next_ch)) if next_ch.is_whitespace() => return &text[..byte_idx + 1],
                _ => {}
            }
        }
    }
    text
}

/// 按行过滤系统标签、合并空白、删除图片标记，提取首句（限制 80 字符）
pub(crate) fn clean_fallback_user_text(text: &str) -> Option<String> {
    let mut in_system_block = false;

    for line in text.lines() {
        let trimmed = line.trim();

        if in_system_block {
            if is_system_tag(trimmed) && is_closing_system_tag(trimmed) {
                in_system_block = false;
            }
            continue;
        }

        // 系统标签处理：闭合标签跳过；开标签进入块跳过模式；
        // 自闭合标签剥离标签区间后继续处理同行残余内容
        // （`<system>x</system> 真实问题` → `真实问题`，不再整行丢弃）。
        let mut content_line = line;
        if is_system_tag(trimmed) {
            if is_closing_system_tag(trimmed) {
                continue;
            }
            if !is_self_contained_system_tag(trimmed) {
                in_system_block = true;
                continue;
            }
            let Some(remainder) = strip_leading_self_contained_tag(trimmed) else {
                continue;
            };
            if remainder.is_empty() {
                continue;
            }
            content_line = remainder;
        }

        let cleaned_line = strip_image_markers(content_line);
        let mut trimmed_start = cleaned_line.trim_start();
        while trimmed_start.starts_with('-')
            || trimmed_start.starts_with('=')
            || trimmed_start.starts_with('#')
            || trimmed_start.starts_with('*')
        {
            trimmed_start = trimmed_start
                .trim_start_matches(['-', '=', '#', '*'])
                .trim_start();
        }
        if trimmed_start.is_empty() {
            continue;
        }

        if trimmed_start.starts_with("AGENTS.md instructions for")
            || trimmed_start.starts_with("AGENTS.md instructions")
        {
            continue;
        }

        let raw_first_sentence = extract_first_sentence(trimmed_start);
        let collapsed = raw_first_sentence
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");

        if collapsed.is_empty() {
            continue;
        }

        let truncated = if let Some((idx, _)) = collapsed.char_indices().nth(80) {
            collapsed[..idx].to_string()
        } else {
            collapsed
        };

        return Some(truncated);
    }

    None
}

/// 从 JSON 条目提取标题字段（camelCase → "title" → snake_case 三级回退），空值忽略
pub(crate) fn extract_typed_title(
    entry: &serde_json::Value,
    camel_key: &str,
    snake_key: &str,
) -> Option<String> {
    entry
        .get(camel_key)
        .or_else(|| entry.get("title"))
        .or_else(|| entry.get(snake_key))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// 统一会话展示名解析链（全部列表/搜索/归档视图共用，禁止在调用侧另写优先级）：
/// 用户重命名 > 原生标题（原样） > 清洗后首条用户消息（原样） > history.display（清洗） > session_id
pub(crate) fn resolve_display_name(
    custom_name: Option<&str>,
    title: Option<&str>,
    first_user_message: Option<&str>,
    history_display: Option<&str>,
    session_id: &str,
) -> String {
    if let Some(name) = custom_name.map(str::trim).filter(|s| !s.is_empty()) {
        return name.to_string();
    }
    if let Some(t) = title.map(str::trim).filter(|s| !s.is_empty()) {
        return t.to_string();
    }
    if let Some(m) = first_user_message.map(str::trim).filter(|s| !s.is_empty()) {
        return m.to_string();
    }
    if let Some(cleaned) = history_display.and_then(clean_fallback_user_text) {
        return cleaned;
    }
    session_id.to_string()
}

/// 多 Provider 增强型会话元数据提取：返回 (原生标题, 清洗后首条用户消息, cwd)
pub(crate) fn extract_snapshot_metadata_enhanced(
    content: &[u8],
    cli_id: &str,
    session_path: &str,
    options: &TitleResolverOptions,
) -> (Option<String>, Option<String>, Option<String>) {
    if cli_id == "codex" {
        if let Some(map) = options.codex_index_titles {
            let stem = std::path::Path::new(session_path)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default();
            if let Some(t) = map.get(stem).or_else(|| map.get(session_path)) {
                let (native, user, cwd) = scan_content_lines(content);
                return (Some(t.clone()).or(native), user, cwd);
            }
        }
    }

    scan_content_lines(content)
}

fn scan_content_lines(content: &[u8]) -> (Option<String>, Option<String>, Option<String>) {
    let text = String::from_utf8_lossy(content);
    let mut custom_title: Option<String> = None;
    let mut ai_title: Option<String> = None;
    let mut user_title: Option<String> = None;
    let mut cwd: Option<String> = None;

    // 全行扫描（无行数窗口）：ai-title/custom-title 由 Claude Code 追加在 JSONL
    // 末尾，take(N) 窗口会漏掉长会话的原生标题。内容已在内存（归档快照本身需
    // 整体 gunzip，无法提前终止），memchr 子串预过滤使全行扫描开销可忽略。
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.len() > 256 * 1024 {
            continue;
        }

        // Fast-path 预检测过滤非标题/消息/cwd 行（避免无谓 serde_json 堆分配）
        if !line.contains("\"custom-title\"")
            && !line.contains("\"ai-title\"")
            && !line.contains("\"user\"")
            && !line.contains("\"response_item\"")
            && !line.contains("\"cwd\"")
        {
            continue;
        }

        let Ok(entry) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };

        if cwd.is_none() {
            cwd = entry
                .get("cwd")
                .and_then(|v| v.as_str())
                .or_else(|| {
                    entry
                        .get("payload")
                        .and_then(|p| p.get("cwd"))
                        .and_then(|v| v.as_str())
                })
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
        }

        let ty = entry.get("type").and_then(|v| v.as_str()).unwrap_or("");
        match ty {
            // last-wins：同一文件多次重命名/重新总结时，靠后的标题行为最新
            "custom-title" => {
                if let Some(t) = extract_typed_title(&entry, "customTitle", "custom_title") {
                    custom_title = Some(t);
                }
            }
            "ai-title" => {
                if let Some(t) = extract_typed_title(&entry, "aiTitle", "ai_title") {
                    ai_title = Some(t);
                }
            }
            "user" | "response_item" => {
                if user_title.is_none() {
                    if let Some(raw) = extract_raw_user_text(&entry) {
                        user_title = clean_fallback_user_text(&raw);
                    }
                }
            }
            _ => {}
        }
    }

    (custom_title.or(ai_title), user_title, cwd)
}

fn extract_raw_user_text(entry: &serde_json::Value) -> Option<String> {
    let ty = entry.get("type").and_then(|v| v.as_str()).unwrap_or("");
    match ty {
        "user" => {
            if entry
                .get("isSidechain")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                return None;
            }
            let content = entry
                .get("message")
                .and_then(|m| m.get("content"))
                .or_else(|| entry.get("content"))?;
            snapshot_content_text(content)
        }
        "response_item" => {
            let payload = entry.get("payload")?;
            if payload.get("type").and_then(|v| v.as_str()) != Some("message") {
                return None;
            }
            if payload.get("role").and_then(|v| v.as_str()) != Some("user") {
                return None;
            }
            snapshot_content_text(payload.get("content")?)
        }
        _ => None,
    }
}

fn snapshot_content_text(content: &serde_json::Value) -> Option<String> {
    if let Some(s) = content.as_str() {
        let t = s.trim();
        return if t.is_empty() {
            None
        } else {
            Some(t.to_string())
        };
    }
    let arr = content.as_array()?;
    let mut texts = Vec::new();
    for item in arr {
        if let Some(t) = item.get("text").and_then(|v| v.as_str()) {
            let t = t.trim();
            if !t.is_empty() {
                texts.push(t.to_string());
            }
        }
    }
    if texts.is_empty() {
        None
    } else {
        Some(texts.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_system_tag() {
        assert!(is_system_tag("<environment_context>"));
        assert!(is_system_tag("</environment_context>"));
        assert!(is_system_tag("<INSTRUCTIONS>"));
        assert!(is_system_tag("<permissions instructions=\"true\">"));
        assert!(is_system_tag("<system>"));
        assert!(is_system_tag("<system-reminder>"));
        assert!(is_system_tag("</system-reminder>"));
        assert!(is_system_tag("<local-command-stdout>"));
        assert!(is_system_tag("<local-command-caveat>"));
        assert!(is_system_tag("<command-message>"));
        assert!(is_system_tag("<command-name>"));
        assert!(!is_system_tag("Hello world"));
        assert!(!is_system_tag("<user>"));
        assert!(!is_system_tag("<systemish>"));
    }

    #[test]
    fn test_strip_image_markers() {
        let raw = "Hello [Image: source: /path/to/img.png] world!";
        assert_eq!(strip_image_markers(raw), "Hello  world!");
    }

    #[test]
    fn test_clean_fallback_user_text_strips_system_context_and_images() {
        let input = "<environment_context>\nsome context\n</environment_context>\nHello [Image: source: /path/to/img.png] world!";
        let cleaned = clean_fallback_user_text(input);
        assert_eq!(cleaned.as_deref(), Some("Hello world!"));
    }

    #[test]
    fn test_clean_fallback_user_text_system_reminder_block() {
        let input = "<system-reminder>\nhook injected context\n</system-reminder>\n真正的问题是什么";
        let cleaned = clean_fallback_user_text(input);
        assert_eq!(cleaned.as_deref(), Some("真正的问题是什么"));
    }

    #[test]
    fn test_clean_fallback_user_text_self_contained_tag_keeps_remainder() {
        let input = "<system>note</system> 帮我修复登录页的崩溃";
        let cleaned = clean_fallback_user_text(input);
        assert_eq!(cleaned.as_deref(), Some("帮我修复登录页的崩溃"));

        let input2 = "<environment_context>x</environment_context> real question";
        assert_eq!(
            clean_fallback_user_text(input2).as_deref(),
            Some("real question")
        );
    }

    #[test]
    fn test_clean_fallback_user_text_chinese_and_truncation() {
        let input = "这是一个非常长的中文测试文本，用于测试提取首句和八十个字符自动截断的功能。看看截断效果如何。";
        let cleaned = clean_fallback_user_text(input);
        assert_eq!(
            cleaned.as_deref(),
            Some("这是一个非常长的中文测试文本，用于测试提取首句和八十个字符自动截断的功能。")
        );
    }

    #[test]
    fn test_clean_fallback_user_text_empty() {
        let input = "<system>\nonly system info\n</system>";
        assert_eq!(clean_fallback_user_text(input), None);
    }

    #[test]
    fn test_extract_typed_title_key_precedence() {
        let entry = serde_json::json!({"customTitle": "Camel", "title": "Plain", "custom_title": "Snake"});
        assert_eq!(
            extract_typed_title(&entry, "customTitle", "custom_title").as_deref(),
            Some("Camel")
        );
        let entry = serde_json::json!({"title": "Plain", "custom_title": "Snake"});
        assert_eq!(
            extract_typed_title(&entry, "customTitle", "custom_title").as_deref(),
            Some("Plain")
        );
        let entry = serde_json::json!({"customTitle": "  "});
        assert_eq!(extract_typed_title(&entry, "customTitle", "custom_title"), None);
    }

    #[test]
    fn test_extract_claude_ai_title() {
        let line1 = r#"{"type":"user","message":{"content":"Hello world"}}"#;
        let line2 = r#"{"type":"ai-title","aiTitle":"Optimized Coredns Rules"}"#;
        let line3 = r#"{"type":"custom-title","customTitle":"My Custom Title"}"#;
        let content = format!("{line1}\n{line2}\n{line3}\n");
        let options = TitleResolverOptions {
            codex_index_titles: None,
        };
        let (title, user, _) = extract_snapshot_metadata_enhanced(
            content.as_bytes(),
            "claude",
            "/path/s1.jsonl",
            &options,
        );
        assert_eq!(title.as_deref(), Some("My Custom Title"));
        assert_eq!(user.as_deref(), Some("Hello world"));

        let content_ai_only = format!("{line1}\n{line2}\n");
        let (title_ai, _, _) = extract_snapshot_metadata_enhanced(
            content_ai_only.as_bytes(),
            "claude",
            "/path/s1.jsonl",
            &options,
        );
        assert_eq!(title_ai.as_deref(), Some("Optimized Coredns Rules"));
    }

    #[test]
    fn test_extract_late_emitted_title_beyond_60_lines() {
        // 回归：ai-title 追加在 JSONL 末尾，长会话（>60 行）也必须能取到
        let mut lines: Vec<String> = (0..500)
            .map(|i| format!(r#"{{"type":"assistant","message":{{"content":"line {i}"}}}}"#))
            .collect();
        lines.push(r#"{"type":"ai-title","aiTitle":"Late Title"}"#.to_string());
        let content = lines.join("\n");
        let options = TitleResolverOptions {
            codex_index_titles: None,
        };
        let (title, _, _) = extract_snapshot_metadata_enhanced(
            content.as_bytes(),
            "claude",
            "/path/s1.jsonl",
            &options,
        );
        assert_eq!(title.as_deref(), Some("Late Title"));
    }

    #[test]
    fn test_extract_custom_title_last_wins() {
        // 多次重命名：靠后的 custom-title 行为最新
        let content = concat!(
            r#"{"type":"custom-title","customTitle":"Old Name"}"#,
            "\n",
            r#"{"type":"custom-title","customTitle":"New Name"}"#,
            "\n",
            r#"{"type":"ai-title","aiTitle":"AI Title"}"#,
        );
        let options = TitleResolverOptions {
            codex_index_titles: None,
        };
        let (title, _, _) = extract_snapshot_metadata_enhanced(
            content.as_bytes(),
            "claude",
            "/path/s1.jsonl",
            &options,
        );
        assert_eq!(title.as_deref(), Some("New Name"));
    }

    #[test]
    fn test_extract_codex_index_title() {
        let content = r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"do task"}]}}"#;
        let mut index = HashMap::new();
        index.insert("s2".to_string(), "Codex Custom Thread".to_string());
        let options = TitleResolverOptions {
            codex_index_titles: Some(&index),
        };
        let (title, user, _) = extract_snapshot_metadata_enhanced(
            content.as_bytes(),
            "codex",
            "/path/s2.jsonl",
            &options,
        );
        assert_eq!(title.as_deref(), Some("Codex Custom Thread"));
        assert_eq!(user.as_deref(), Some("do task"));
    }

    #[test]
    fn test_resolve_display_name_precedence() {
        // 1. 用户重命名最高优先
        assert_eq!(
            resolve_display_name(Some("My Rename"), Some("Native"), Some("Msg"), Some("Hist"), "id"),
            "My Rename"
        );
        // 2. 原生标题次之（原样，不清洗）
        assert_eq!(
            resolve_display_name(None, Some("<WIP> Native"), Some("Msg"), Some("Hist"), "id"),
            "<WIP> Native"
        );
        // 3. 清洗后首条消息（原样）
        assert_eq!(
            resolve_display_name(None, None, Some("Msg"), Some("Hist"), "id"),
            "Msg"
        );
        // 4. history.display 需清洗
        assert_eq!(
            resolve_display_name(None, None, None, Some("<system>x</system> Hist"), "id"),
            "Hist"
        );
        // 5. 兜底 session_id
        assert_eq!(resolve_display_name(None, None, None, None, "id"), "id");
        // 空串按缺失处理
        assert_eq!(
            resolve_display_name(Some("  "), Some(""), None, None, "id"),
            "id"
        );
    }
}
