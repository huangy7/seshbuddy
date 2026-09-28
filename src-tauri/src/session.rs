use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectInfo {
    /// 项目分组键。Codex / Antigravity 一类没有独立目录名的 CLI 直接拿项目路径当键，
    /// 故与 `original_path` 同生共死：会话解析不出项目路径时两者一起为 `None`。
    pub encoded_dir: Option<String>,
    /// 项目路径。`None` 表示**没有这个值**（不是空串），兜底文案由前端按语言渲染。
    pub original_path: Option<String>,
    pub sessions: Vec<SessionInfo>,
}

/// Paginated response for scan_projects.
#[derive(Debug, Clone, Serialize)]
pub struct PaginatedProjects {
    pub projects: Vec<ProjectInfo>,
    pub has_more: bool,
    pub total_sessions: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    pub session_id: String,
    pub file_path: String,
    pub display_name: String,
    pub timestamp: String,
    pub file_size: u64,
    pub git_branch: String,
    pub has_archive_snapshot: bool,
    pub is_archived: bool,
    /// 会话归属的 CLI（CliKind::id()），前端 Tab 据此绑定操作上下文
    pub cli_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation_input_tokens: u64,
    pub cache_read_input_tokens: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub timestamp: String,
    pub model: Option<String>,
    pub token_usage: Option<TokenUsage>,
    pub content_parts: Vec<ContentPart>,
    #[serde(default)]
    pub is_meta: bool,
    /// Transcript entry uuid (Claude & WorkBuddy only) — stable anchor for fork-from-here.
    /// Claude: top-level `uuid`. WorkBuddy: `id` → `callId` (`parser/workbuddy.rs`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uuid: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ContentPart {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        summary: String,
        #[serde(default)]
        tool_name: String,
        #[serde(default)]
        input: String,
        tool_use_id: Option<String>,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        summary: String,
        content: String,
        is_error: bool,
        /// 与对应 ToolUse 配对的 id（Claude 数据本就有；取全文命令的防漂移校验用）。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tool_use_id: Option<String>,
        /// 截断后全文长度（≤ 50000）。懒加载开启后供「加载全文」展示与拉取预期。
        #[serde(default)]
        full_len: u64,
        /// true 表示 `content` 只是预览（前 TOOL_RESULT_PREVIEW_LEN 字符），
        /// 全文需经 `get_tool_result_full_content` 按 `source_offset` 拉取。
        /// false = 带全文（小内容或白名单 part），行为与引入懒加载前完全一致。
        #[serde(default)]
        truncated_preview: bool,
        /// true 表示 `content` 在 50 KB 处被截断、**正文本身不完整**（与
        /// `truncated_preview` 无关：后者说的是「只带了预览」）。
        /// 截断提示语由前端按 `common.toolResult.truncatedSuffix` 渲染 —— 后端不再往正文里拼中文。
        #[serde(default)]
        truncated: bool,
        /// 该行在会话文件中的字节 offset（append-only 下永久稳定，seek O(1) 定位）。
        #[serde(default)]
        source_offset: u64,
    },
    #[serde(rename = "thinking")]
    Thinking { thinking: String },
    #[serde(rename = "image")]
    Image { media_type: String, data: String },
    #[serde(rename = "image_ref")]
    ImageRef { path: String },
    #[serde(rename = "image_meta")]
    ImageMeta,
}

impl ContentPart {
    /// 构造一个「带全文」的 tool_result part（非 Claude CLI 与 Claude 白名单 part 用），
    /// 懒加载相关字段给缺省值（`full_len` 由调用方按需覆写）。
    /// `truncated` = 正文是否在 50 KB 处被截断（只有实施截断的解析器才传 `true`）。
    pub(crate) fn tool_result(
        summary: String,
        content: String,
        is_error: bool,
        truncated: bool,
    ) -> Self {
        ContentPart::ToolResult {
            summary,
            content,
            is_error,
            tool_use_id: None,
            full_len: 0,
            truncated_preview: false,
            truncated,
            source_offset: 0,
        }
    }
}

/// A single usage record extracted from an assistant turn.
#[derive(Debug, Clone, Serialize)]
pub struct UsageRecord {
    pub date: String,          // YYYY-MM-DD (local time)
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation_tokens: u64,
    pub cache_read_tokens: u64,
    pub duration_ms: Option<u64>,
    pub project: String,
}

#[derive(Debug, Clone)]
pub struct SessionListMetadata {
    pub session_id: String,
    pub project_path: Option<String>,
    /// Provider 原生标题（Claude custom-title/ai-title；Codex 由调用侧经索引 Map 注入）
    pub title: Option<String>,
    /// 清洗后的首条用户消息（标题兜底）
    pub first_user_message: Option<String>,
    pub first_timestamp: Option<String>,
    pub last_timestamp: Option<String>,
    pub git_branch: String,
    pub file_size: u64,
}

/// Decode an encoded project directory name back to a path.
///
/// 注意：Claude 的目录编码会把路径分隔符编码为 `-`，而真实目录名里的 `-`
/// 也会原样保留，因此单靠目录名反解在 Windows 下存在歧义（例如 `go-timer`）。
/// 这个函数只适合作为最后的兜底；正常情况下应优先使用会话文件里的
/// 真实 `cwd`，其次才是 `history.jsonl` 中的原始项目路径映射。
pub fn decode_project_dir(encoded: &str) -> String {
    if encoded.starts_with('-') {
        encoded.replace('-', "/")
    } else if encoded.len() >= 2 && encoded.as_bytes()[0].is_ascii_alphabetic() && encoded.as_bytes()[1] == b'-' {
        let drive = encoded.as_bytes()[0] as char;
        let rest = &encoded[2..];
        format!("{}:\\{}", drive, rest.replace('-', "\\"))
    } else {
        encoded.replace('-', "/")
    }
}

pub fn find_project_path_in_map<'a>(
    encoded: &str,
    project_map: &'a HashMap<String, String>,
) -> Option<&'a str> {
    project_map
        .get(encoded)
        .or_else(|| {
            if encoded.len() >= 2
                && encoded.as_bytes()[0].is_ascii_alphabetic()
                && encoded.as_bytes()[1] == b'-'
            {
                project_map
                    .iter()
                    .find(|(key, _)| key.eq_ignore_ascii_case(encoded))
                    .map(|(_, value)| value)
            } else {
                None
            }
        })
        .map(|value| value.as_str())
}

/// encoded 目录名里表示「没有这个值」的 ASCII 哨兵。
///
/// 这个字面量有**两个来源，别把下面两句读成互相矛盾**：
/// - **主流来源不在本仓库**：encoded 目录名由外部 CLI 写盘，我们只是读它
///   （`commands/session_index/scan.rs` 取 `file_name()`）。这一侧我们改不了，所以
///   「结构化」在这里无从谈起 —— 没法让外部 CLI 别写这个串。
/// - **本仓库自己也会造同形的串**：搜索路径上会话文件的父目录名取不到时，
///   `commands/session.rs` 用下面这个常量兜底，再把它交给本函数。
///   这一处是我们的串，但它与上一句说的是两件事，**不是**"我们生产主流来源"。
///
/// 两侧汇进同一个消费端收口点（`resolve_project_path`），与空值一样落 `None`，
/// 调用方才不会把它当路径落库。
///
/// ⚠️ **闸门看不见它**——规则 2 只认 CJK，而这是 ASCII，改错也不会报红。
/// 它是本计划删掉的 `未知项目` 哨兵的英文孪生（那一条靠 CJK 才被闸门捞出来）。
///
/// `pub(crate)`：两个生产者（本文件的读取侧与 `commands/session.rs` 的兜底侧）必须写
/// **同一个**值 —— 私有的话第二个生产者只能写裸字面量，改名时它不会跟着动。
pub(crate) const UNKNOWN_PROJECT_DIR: &str = "unknown";

/// 解析会话所属的项目路径，供列表分组与搜索结果展示。
///
/// 解析不出时返回 `None` —— **不造兜底值**。兜底串会被写进
/// `session_list_index.project_path`，下一轮扫描再把它当真实路径读回来，此后真假就再也分不开；
/// 「没有这个值」必须由 `None`/SQL `NULL` 表达，展示用的兜底句交给前端按语言渲染。
pub fn resolve_project_path(
    encoded: &str,
    cached_path: Option<&str>,
    project_map: Option<&HashMap<String, String>>,
) -> Option<String> {
    if encoded.trim().is_empty() || encoded == UNKNOWN_PROJECT_DIR {
        return None;
    }

    if let Some(path) = project_map
        .and_then(|map| find_project_path_in_map(encoded, map))
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Some(path.to_string());
    }

    // 索引里读回的旧值只在它是**真实路径**时才算数：空值意味着这一列没有值。
    if let Some(path) = cached_path.map(str::trim).filter(|value| !value.is_empty()) {
        return Some(path.to_string());
    }

    Some(decode_project_dir(encoded))
}

/// Generate a tool_use summary from the tool name and input JSON.
pub fn tool_use_summary(name: &str, input: &serde_json::Value) -> String {
    match name {
        "Read" | "Write" | "Edit" => {
            let file_path = input
                .get("file_path")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let filename = Path::new(file_path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(file_path);
            format!("[{}: {}]", name, filename)
        }
        "Bash" => {
            let cmd = input
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let truncated = if cmd.chars().count() > 40 {
                let s: String = cmd.chars().take(40).collect();
                format!("{}...", s)
            } else {
                cmd.to_string()
            };
            format!("[Bash: {}]", truncated)
        }
        "Grep" => {
            let pattern = input
                .get("pattern")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            format!("[Grep: {}]", pattern)
        }
        "Glob" => {
            let pattern = input
                .get("pattern")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            format!("[Glob: {}]", pattern)
        }
        "Task" => {
            let desc = input
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            format!("[Task: {}]", desc)
        }
        "show_widget" => {
            let title = input
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if title.is_empty() {
                "[图表]".to_string()
            } else {
                format!("[图表: {}]", title)
            }
        }
        _ => format!("[{}]", name),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentInfo {
    pub file_path: String,
    pub label: String,
}

/// Result of incremental session loading.
#[derive(Debug, Clone, Serialize)]
pub struct SessionLoadResult {
    pub messages: Vec<ChatMessage>,
    pub offset: u64,
    pub subagent_map: HashMap<String, SubagentInfo>,
}

/// Clean user prompt text by stripping system caveat blocks (e.g. <local-command-caveat>) and XML tags.
pub fn clean_user_message_text(text: &str) -> Option<String> {
    let mut s = text.trim();
    if s.is_empty() {
        return None;
    }
    // 1. 如果包含 </local-command-caveat>，截取其后面的有效内容
    if let Some(idx) = s.find("</local-command-caveat>") {
        s = s[idx + "</local-command-caveat>".len()..].trim();
    }
    // 2. 如果包含 <command-message>，提取里面的内容
    if let (Some(start), Some(end)) = (s.find("<command-message>"), s.find("</command-message>")) {
        if end > start + "<command-message>".len() {
            let cmd_msg = s[start + "<command-message>".len()..end].trim();
            if !cmd_msg.is_empty() {
                return Some(cmd_msg.to_string());
            }
        }
    }
    // 3. 如果包含 XML 标签控制块，提取内部标签外的文本
    let cleaned_str = if s.starts_with('<') && s.contains('>') {
        let mut in_tag = false;
        let mut clean = String::new();
        for c in s.chars() {
            if c == '<' {
                in_tag = true;
            } else if c == '>' {
                in_tag = false;
            } else if !in_tag {
                clean.push(c);
            }
        }
        clean.trim().to_string()
    } else {
        s.to_string()
    };

    let res = cleaned_str.trim();
    if res.is_empty() {
        None
    } else {
        Some(res.to_string())
    }
}

/// 搜索结果命中的字段。**不是文案**——前端据此渲染本地化标签。
///
/// 四个变体各自对应一个**可命名的事实**：元数据命中（`Title`/`FirstMessage`）、
/// 会话 ID 命中（`SessionId`）、正文命中（`Content`）。
///
/// `None`（外层 `Option`）只剩「**无从分类**」一义：文档文本既不是标题、也不是首条消息，
/// 也不是可确认的会话 ID —— 它来自 `merge_title_and_id_search_matches` 的路径/ID LIKE 分支，
/// 且只在该分支的文本是**不含查询词的标题**时出现，那种文档的计数为 0、不会被选作 snippet 来源。
/// 展示用的标签由前端按语言渲染；`Content` 不渲染前缀（正文命中不需要说明命中在哪）。
///
/// ⚠️ 别把正文命中或会话 ID 命中重新塞回 `None`：那正是这次拆开的两件事，
/// 前端会因此把「命中的是会话 id」渲染成裸 id。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchedField {
    Title,
    FirstMessage,
    SessionId,
    Content,
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchResult {
    pub session_id: String,
    pub file_path: String,
    pub display_name: String,
    /// `None` = 解析不出项目路径，兜底句由前端按语言渲染（同 `ProjectInfo::original_path`）。
    pub project_path: Option<String>,
    pub snippet: String,
    /// 命中字段；`None` = 无从分类（见 `MatchedField` 的注释），前端不渲染标签。
    pub matched_field: Option<MatchedField>,
    pub match_count: usize,
    pub first_match_message_index: Option<usize>,
    /// 结果所属 CLI("claude"/"codex"/"gemini"/"workbuddy"),由 pipeline 盖戳
    pub cli_id: String,
}

/// Extract a snippet of approximately `context_chars` characters around the first
/// case-insensitive occurrence of `query` in `text`. Adds "..." at the boundaries
/// if the snippet is truncated.
pub fn extract_snippet(text: &str, query: &str, context_chars: usize) -> String {
    let text_lower = text.to_lowercase();
    let query_lower = query.to_lowercase();

    let pos = match text_lower.find(&query_lower) {
        Some(p) => p,
        None => return String::new(),
    };

    let start = if pos > context_chars {
        pos - context_chars
    } else {
        0
    };
    let end = std::cmp::min(text.len(), pos + query.len() + context_chars);

    // Ensure we don't split multi-byte characters
    let safe_start = if start == 0 {
        0
    } else {
        // Find a valid char boundary at or after `start`
        let mut s = start;
        while s < text.len() && !text.is_char_boundary(s) {
            s += 1;
        }
        s
    };
    let safe_end = if end >= text.len() {
        text.len()
    } else {
        let mut e = end;
        while e < text.len() && !text.is_char_boundary(e) {
            e += 1;
        }
        e
    };

    let slice = &text[safe_start..safe_end];
    // Replace newlines with spaces for a cleaner snippet
    let snippet = slice.replace('\n', " ").replace('\r', " ");

    let prefix = if safe_start > 0 { "..." } else { "" };
    let suffix = if safe_end < text.len() { "..." } else { "" };

    format!("{}{}{}", prefix, snippet.trim(), suffix)
}

/// 流式扫描时每个 chunk 元素：一条会话 + 它所属项目的元信息。
/// 前端拿到 chunk 后按 `encoded_dir` 增量合并到 ProjectInfo 列表。
#[derive(Debug, Clone, Serialize)]
pub struct ProjectSessionChunkItem {
    pub session: SessionInfo,
    /// 与 `ProjectInfo::encoded_dir` 同义，含 `None` 的缺席语义。
    pub encoded_dir: Option<String>,
    /// 与 `ProjectInfo::original_path` 同义，含 `None` 的缺席语义。
    pub original_path: Option<String>,
}

#[cfg(test)]
mod resolve_project_path_tests {
    use super::*;

    fn map(entries: &[(&str, &str)]) -> HashMap<String, String> {
        entries
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    /// 解析不出项目路径时必须回 `None`：这是「后端不再造占位串」的判据。
    /// 回落到某个非空占位串会让调用方把它当成真路径落库（见 `ProjectInfo::original_path`）。
    ///
    /// 第三行**刻意写字面量**而不是 `UNKNOWN_PROJECT_DIR`：判据与被判对象同源的话，
    /// 把常量值改掉这条仍会绿 —— 这里钉的是「哨兵就是 `unknown`」这个事实本身。
    #[test]
    fn unresolvable_encoded_dir_yields_none() {
        assert_eq!(resolve_project_path("", None, None), None);
        assert_eq!(resolve_project_path("   ", None, None), None);
        assert_eq!(resolve_project_path("unknown", None, None), None);
    }

    /// 索引里读回的空值不是路径，不能被当成缓存命中；此时应回落到目录名解码。
    /// 旧实现在这里比的是那个占位串 —— 占位串没了之后，守卫由「非空」承担。
    #[test]
    fn blank_cached_path_is_not_a_cache_hit() {
        assert_eq!(
            resolve_project_path("--Users-x-proj--", Some("  "), None),
            Some(decode_project_dir("--Users-x-proj--"))
        );
        assert_eq!(
            resolve_project_path("--Users-x-proj--", Some("/Users/x/from-index"), None),
            Some("/Users/x/from-index".to_string())
        );
    }

    /// history 映射优先于索引缓存：它是权威来源。
    #[test]
    fn history_map_wins_over_cached_path() {
        let project_map = map(&[("--Users-x-proj--", "/Users/x/from-history")]);
        assert_eq!(
            resolve_project_path("--Users-x-proj--", Some("/Users/x/from-index"), Some(&project_map)),
            Some("/Users/x/from-history".to_string())
        );
    }
}
