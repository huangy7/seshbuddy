//! Pi 会话解析器。
//!
//! Pi CLI 会话转录以 JSONL 格式存放于 `~/.pi/agent/sessions/` 目录下。
//! 每份会话文件首行是 session 头信息，后续每行包含 `id` 与 `parentId` 构建的会话树结构。
//! 为完整复原用户会话上下文，解析时会自底向上追溯活跃分支，并处理思维链提取、工具调用与上下文压缩。

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{AppError, AppResult};
use crate::parser::shared::{SearchDocument, SearchScanProgress};
use crate::session::{
    self, ChatMessage, ContentPart, SessionListMetadata, SessionLoadResult,
    TokenUsage,
};

// ─────────────────────────────────────────────────────────────────────────────
// 数据模型定义
// ─────────────────────────────────────────────────────────────────────────────

/// 会话头信息（JSONL 首行）
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PiSessionHeader {
    #[serde(default = "default_session_version")]
    pub version: u32,
    pub id: String,
    pub timestamp: String,
    pub cwd: String,
    #[serde(rename = "parentSession", default)]
    pub parent_session: Option<String>,
}

fn default_session_version() -> u32 {
    1
}

/// 所有条目通用的基础字段
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PiEntryBase {
    pub id: String,
    #[serde(rename = "parentId", default)]
    pub parent_id: Option<String>,
    #[serde(default)]
    pub timestamp: String,
}

/// Pi 会话行条目
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type")]
pub enum PiEntry {
    #[serde(rename = "session")]
    Session(PiSessionHeader),
    #[serde(rename = "message")]
    Message(PiMessageEntry),
    #[serde(rename = "model_change")]
    ModelChange(PiModelChangeEntry),
    #[serde(rename = "thinking_level_change")]
    ThinkingLevelChange(PiThinkingLevelChangeEntry),
    #[serde(rename = "usage")]
    Usage(PiUsageEntry),
    #[serde(rename = "compaction")]
    Compaction(PiCompactionEntry),
    #[serde(rename = "context_edit")]
    ContextEdit(PiContextEditEntry),
    #[serde(rename = "branch_summary")]
    BranchSummary(PiBranchSummaryEntry),
    #[serde(rename = "custom")]
    Custom(PiCustomEntry),
    #[serde(rename = "custom_message")]
    CustomMessage(PiCustomMessageEntry),
    #[serde(rename = "label")]
    Label(PiLabelEntry),
    #[serde(rename = "session_info")]
    SessionInfo(PiSessionInfoEntry),
    #[serde(skip)]
    Unparsed {
        id: String,
        parent_id: Option<String>,
    },
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PiMessageEntry {
    #[serde(flatten)]
    pub base: PiEntryBase,
    pub message: PiAgentMessage,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PiModelChangeEntry {
    #[serde(flatten)]
    pub base: PiEntryBase,
    #[serde(default)]
    pub provider: String,
    #[serde(rename = "modelId", default)]
    pub model_id: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PiThinkingLevelChangeEntry {
    #[serde(flatten)]
    pub base: PiEntryBase,
    #[serde(rename = "thinkingLevel", default)]
    pub thinking_level: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PiUsageEntry {
    #[serde(flatten)]
    pub base: PiEntryBase,
    #[serde(default)]
    pub model: String,
    pub usage: PiUsage,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PiCompactionEntry {
    #[serde(flatten)]
    pub base: PiEntryBase,
    #[serde(default)]
    pub summary: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PiContextEditEntry {
    #[serde(flatten)]
    pub base: PiEntryBase,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PiBranchSummaryEntry {
    #[serde(flatten)]
    pub base: PiEntryBase,
    #[serde(default)]
    pub summary: String,
    #[serde(rename = "fromId", default)]
    pub from_id: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PiCustomEntry {
    #[serde(flatten)]
    pub base: PiEntryBase,
    #[serde(rename = "customType", default)]
    pub custom_type: String,
    pub data: Option<Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PiCustomMessageEntry {
    #[serde(flatten)]
    pub base: PiEntryBase,
    #[serde(rename = "customType", default)]
    pub custom_type: String,
    pub content: PiContent,
    #[serde(default)]
    pub display: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PiLabelEntry {
    #[serde(flatten)]
    pub base: PiEntryBase,
    #[serde(rename = "targetId", default)]
    pub target_id: String,
    #[serde(default)]
    pub label: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PiSessionInfoEntry {
    #[serde(flatten)]
    pub base: PiEntryBase,
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "role")]
pub enum PiAgentMessage {
    #[serde(rename = "user")]
    User(PiUserMessage),
    #[serde(rename = "assistant")]
    Assistant(PiAssistantMessage),
    #[serde(rename = "toolResult")]
    ToolResult(PiToolResultMessage),
    #[serde(rename = "bashExecution")]
    BashExecution(PiBashExecutionMessage),
    #[serde(rename = "custom")]
    Custom(PiCustomMessage),
    #[serde(rename = "branchSummary")]
    BranchSummary(PiBranchSummaryMessage),
    #[serde(rename = "compactionSummary")]
    CompactionSummary(PiCompactionSummaryMessage),
    #[serde(rename = "system")]
    System(Value),
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PiUserMessage {
    pub content: PiContent,
    #[serde(default)]
    pub timestamp: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PiAssistantMessage {
    #[serde(default)]
    pub content: Vec<PiContentBlock>,
    pub api: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub usage: Option<PiUsage>,
    #[serde(default)]
    pub timestamp: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PiToolResultMessage {
    #[serde(rename = "toolCallId", default)]
    pub tool_call_id: String,
    #[serde(rename = "toolName", default)]
    pub tool_name: String,
    #[serde(default)]
    pub content: Vec<PiContentBlock>,
    #[serde(rename = "isError", default)]
    pub is_error: bool,
    #[serde(default)]
    pub timestamp: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PiBashExecutionMessage {
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub output: String,
    #[serde(rename = "exitCode")]
    pub exit_code: Option<i32>,
    #[serde(default)]
    pub cancelled: bool,
    #[serde(default)]
    pub timestamp: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PiCustomMessage {
    #[serde(rename = "customType", default)]
    pub custom_type: String,
    pub content: PiContent,
    #[serde(default)]
    pub display: bool,
    #[serde(default)]
    pub timestamp: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PiBranchSummaryMessage {
    #[serde(default)]
    pub summary: String,
    #[serde(rename = "fromId", default)]
    pub from_id: String,
    #[serde(default)]
    pub timestamp: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PiCompactionSummaryMessage {
    #[serde(default)]
    pub summary: String,
    #[serde(rename = "tokensBefore", default)]
    pub tokens_before: u64,
    #[serde(default)]
    pub timestamp: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum PiContent {
    Text(String),
    Blocks(Vec<PiContentBlock>),
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum PiContentBlock {
    Known(PiKnownContentBlock),
    Unknown(Value),
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type")]
pub enum PiKnownContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image")]
    Image {
        data: String,
        #[serde(rename = "mimeType", alias = "mime_type")]
        mime_type: String,
    },
    #[serde(rename = "thinking")]
    Thinking { thinking: String },
    #[serde(rename = "toolCall")]
    ToolCall {
        id: String,
        name: String,
        arguments: Value,
    },
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PiUsage {
    #[serde(default)]
    pub input: u64,
    #[serde(default)]
    pub output: u64,
    #[serde(rename = "cacheRead", default)]
    pub cache_read: u64,
    #[serde(rename = "cacheWrite", default)]
    pub cache_write: u64,
    #[serde(rename = "totalTokens", default)]
    pub total_tokens: u64,
}

// ─────────────────────────────────────────────────────────────────────────────
// 辅助与清洗函数
// ─────────────────────────────────────────────────────────────────────────────

/// 修复 JavaScript 环境由 `JSON.stringify` 导出的孤立 UTF-16 代理对字符。
///
/// 当文本包含孤立的高代理或低代理码元时，Rust `serde_json` 会直接拒绝解析；
/// 将其替换为 Unicode 替换字符 `\uFFFD`，确保消息主体仍能被正常解析与呈现。
pub(crate) fn repair_unpaired_surrogate_escapes(line: &str) -> Option<String> {
    let bytes = line.as_bytes();
    let parse_escape = |start: usize| -> Option<u16> {
        if bytes.get(start) != Some(&b'\\') || bytes.get(start + 1) != Some(&b'u') {
            return None;
        }
        let digits = std::str::from_utf8(bytes.get(start + 2..start + 6)?).ok()?;
        u16::from_str_radix(digits, 16).ok()
    };

    let mut repaired = String::with_capacity(line.len());
    let mut changed = false;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && bytes.get(i + 1) == Some(&b'\\') {
            repaired.push_str("\\\\");
            i += 2;
            continue;
        }
        if let Some(cp) = parse_escape(i) {
            if (0xD800..=0xDBFF).contains(&cp) {
                // 高代理项：检查其后是否紧接合法的低代理项
                if let Some(low) = parse_escape(i + 6) {
                    if (0xDC00..=0xDFFF).contains(&low) {
                        repaired.push_str(&line[i..i + 12]);
                        i += 12;
                        continue;
                    }
                }
                repaired.push('\u{FFFD}');
                i += 6;
                changed = true;
                continue;
            } else if (0xDC00..=0xDFFF).contains(&cp) {
                // 孤立低代理项
                repaired.push('\u{FFFD}');
                i += 6;
                changed = true;
                continue;
            }
        }
        repaired.push(bytes[i] as char);
        i += 1;
    }

    if changed {
        Some(repaired)
    } else {
        None
    }
}

/// 容错解析 Pi 单行 JSON。
pub(crate) fn parse_pi_json_value(line: &str) -> serde_json::Result<Value> {
    match serde_json::from_str(line) {
        Ok(val) => Ok(val),
        Err(err) => {
            if let Some(repaired) = repair_unpaired_surrogate_escapes(line) {
                serde_json::from_str(&repaired).map_err(|_| err)
            } else {
                Err(err)
            }
        }
    }
}

/// 将毫秒时间戳转换为 RFC3339 字符串格式。
pub(crate) fn format_millis_timestamp(ms: u64) -> String {
    if let Some(dt) = chrono::DateTime::from_timestamp_millis(ms as i64) {
        dt.to_rfc3339()
    } else {
        String::new()
    }
}

/// 提取 PiContent 中的纯文本。
pub(crate) fn extract_content_text(content: &PiContent) -> String {
    match content {
        PiContent::Text(text) => text.clone(),
        PiContent::Blocks(blocks) => {
            let mut chunks = Vec::new();
            for block in blocks {
                if let PiContentBlock::Known(PiKnownContentBlock::Text { text }) = block {
                    if !text.is_empty() {
                        chunks.push(text.as_str());
                    }
                }
            }
            chunks.join("\n")
        }
    }
}

/// 获取条目 ID
pub(crate) fn get_entry_id(entry: &PiEntry) -> Option<String> {
    match entry {
        PiEntry::Session(_) => None,
        PiEntry::Message(e) => Some(e.base.id.clone()),
        PiEntry::ModelChange(e) => Some(e.base.id.clone()),
        PiEntry::ThinkingLevelChange(e) => Some(e.base.id.clone()),
        PiEntry::Usage(e) => Some(e.base.id.clone()),
        PiEntry::Compaction(e) => Some(e.base.id.clone()),
        PiEntry::ContextEdit(e) => Some(e.base.id.clone()),
        PiEntry::BranchSummary(e) => Some(e.base.id.clone()),
        PiEntry::Custom(e) => Some(e.base.id.clone()),
        PiEntry::CustomMessage(e) => Some(e.base.id.clone()),
        PiEntry::Label(e) => Some(e.base.id.clone()),
        PiEntry::SessionInfo(e) => Some(e.base.id.clone()),
        PiEntry::Unparsed { id, .. } => Some(id.clone()),
    }
}

/// 获取父条目 ID
pub(crate) fn get_entry_parent_id(entry: &PiEntry) -> Option<String> {
    match entry {
        PiEntry::Session(_) => None,
        PiEntry::Message(e) => e.base.parent_id.clone(),
        PiEntry::ModelChange(e) => e.base.parent_id.clone(),
        PiEntry::ThinkingLevelChange(e) => e.base.parent_id.clone(),
        PiEntry::Usage(e) => e.base.parent_id.clone(),
        PiEntry::Compaction(e) => e.base.parent_id.clone(),
        PiEntry::ContextEdit(e) => e.base.parent_id.clone(),
        PiEntry::BranchSummary(e) => e.base.parent_id.clone(),
        PiEntry::Custom(e) => e.base.parent_id.clone(),
        PiEntry::CustomMessage(e) => e.base.parent_id.clone(),
        PiEntry::Label(e) => e.base.parent_id.clone(),
        PiEntry::SessionInfo(e) => e.base.parent_id.clone(),
        PiEntry::Unparsed { parent_id, .. } => parent_id.clone(),
    }
}

/// 从会话末尾叶子节点逆向回溯至根，构建活跃主分支。
pub(crate) fn build_active_branch(entries: &[PiEntry]) -> Vec<String> {
    let mut parent_map: HashMap<String, String> = HashMap::new();
    let mut leaf_id: Option<String> = None;

    for entry in entries {
        if let Some(id) = get_entry_id(entry) {
            leaf_id = Some(id.clone());
            if let Some(parent_id) = get_entry_parent_id(entry) {
                parent_map.insert(id, parent_id);
            }
        }
    }

    let mut branch = Vec::new();
    let mut current = leaf_id;
    let mut visited = HashSet::new();

    while let Some(id) = current {
        if !visited.insert(id.clone()) {
            // 防御环状引用死循环
            break;
        }
        branch.push(id.clone());
        current = parent_map.get(&id).cloned();
    }

    branch.reverse();
    branch
}

// ─────────────────────────────────────────────────────────────────────────────
// 解析与组装
// ─────────────────────────────────────────────────────────────────────────────

/// 从完整文本中解析 Pi 条目集合。
pub(crate) fn parse_entries_from_content(content: &str) -> Option<(PiSessionHeader, Vec<PiEntry>)> {
    let mut lines = content.lines();
    let first_line = lines.next()?.trim();
    if first_line.is_empty() {
        return None;
    }

    let header_val = parse_pi_json_value(first_line).ok()?;
    let header: PiSessionHeader = serde_json::from_value(header_val).ok()?;

    let mut entries = Vec::new();
    for line in lines {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(val) = parse_pi_json_value(trimmed) {
            let link = val.get("id").and_then(Value::as_str).map(|id| {
                let pid = val.get("parentId").and_then(Value::as_str).map(str::to_string);
                (id.to_string(), pid)
            });
            match serde_json::from_value::<PiEntry>(val) {
                Ok(entry) => entries.push(entry),
                Err(_) => {
                    if let Some((id, parent_id)) = link {
                        entries.push(PiEntry::Unparsed { id, parent_id });
                    }
                }
            }
        }
    }

    Some((header, entries))
}

/// 解析 Pi 会话文件得到消息流。
pub(crate) fn parse_messages_from_entries(_header: &PiSessionHeader, entries: &[PiEntry]) -> Vec<ChatMessage> {
    let active_branch = build_active_branch(entries);

    let entry_map: HashMap<String, &PiEntry> = entries
        .iter()
        .filter_map(|e| get_entry_id(e).map(|id| (id, e)))
        .collect();

    let mut messages = Vec::new();
    let mut current_model: Option<String> = None;

    for entry_id in active_branch {
        let Some(entry) = entry_map.get(entry_id.as_str()).copied() else {
            continue;
        };

        match entry {
            PiEntry::ModelChange(change) => {
                let model = if !change.provider.is_empty() {
                    format!("{}/{}", change.provider, change.model_id)
                } else {
                    change.model_id.clone()
                };
                current_model = Some(model);
            }
            PiEntry::Message(msg_entry) => {
                match &msg_entry.message {
                    PiAgentMessage::User(user) => {
                        let text = extract_content_text(&user.content);
                        if !text.trim().is_empty() {
                            let ts = if user.timestamp > 0 {
                                format_millis_timestamp(user.timestamp)
                            } else {
                                msg_entry.base.timestamp.clone()
                            };
                            messages.push(ChatMessage {
                                role: "user".to_string(),
                                timestamp: ts,
                                model: None,
                                token_usage: None,
                                content_parts: vec![ContentPart::Text { text }],
                                is_meta: false,
                                uuid: Some(msg_entry.base.id.clone()),
                            });
                        }
                    }
                    PiAgentMessage::Assistant(asst) => {
                        let ts = if asst.timestamp > 0 {
                            format_millis_timestamp(asst.timestamp)
                        } else {
                            msg_entry.base.timestamp.clone()
                        };
                        let model = asst.model.clone().or_else(|| current_model.clone());

                        let mut parts = Vec::new();
                        for block in &asst.content {
                            match block {
                                PiContentBlock::Known(PiKnownContentBlock::Text { text }) => {
                                    if !text.is_empty() {
                                        parts.push(ContentPart::Text { text: text.clone() });
                                    }
                                }
                                PiContentBlock::Known(PiKnownContentBlock::Thinking { thinking }) => {
                                    if !thinking.trim().is_empty() {
                                        parts.push(ContentPart::Thinking { thinking: thinking.clone() });
                                    }
                                }
                                PiContentBlock::Known(PiKnownContentBlock::ToolCall { id, name, arguments }) => {
                                    let summary = session::tool_use_summary(name, arguments);
                                    parts.push(ContentPart::ToolUse {
                                        summary,
                                        tool_name: name.clone(),
                                        input: arguments.to_string(),
                                        tool_use_id: Some(id.clone()),
                                    });
                                }
                                PiContentBlock::Known(PiKnownContentBlock::Image { data, mime_type }) => {
                                    parts.push(ContentPart::Image {
                                        media_type: mime_type.clone(),
                                        data: data.clone(),
                                    });
                                }
                                PiContentBlock::Unknown(_) => {}
                            }
                        }

                        let token_usage = asst.usage.as_ref().map(|u| TokenUsage {
                            input_tokens: u.input,
                            output_tokens: u.output,
                            cache_creation_input_tokens: u.cache_write,
                            cache_read_input_tokens: u.cache_read,
                        });

                        if !parts.is_empty() {
                            messages.push(ChatMessage {
                                role: "assistant".to_string(),
                                timestamp: ts,
                                model,
                                token_usage,
                                content_parts: parts,
                                is_meta: false,
                                uuid: Some(msg_entry.base.id.clone()),
                            });
                        }
                    }
                    PiAgentMessage::ToolResult(result) => {
                        let ts = if result.timestamp > 0 {
                            format_millis_timestamp(result.timestamp)
                        } else {
                            msg_entry.base.timestamp.clone()
                        };
                        let text_content = result
                            .content
                            .iter()
                            .filter_map(|block| match block {
                                PiContentBlock::Known(PiKnownContentBlock::Text { text }) => Some(text.as_str()),
                                _ => None,
                            })
                            .collect::<Vec<_>>()
                            .join("\n");

                        let summary = format!("[{}: Result]", result.tool_name);
                        messages.push(ChatMessage {
                            role: "tool".to_string(),
                            timestamp: ts,
                            model: None,
                            token_usage: None,
                            content_parts: vec![ContentPart::tool_result(
                                summary,
                                text_content,
                                result.is_error,
                                false,
                            )],
                            is_meta: false,
                            uuid: Some(msg_entry.base.id.clone()),
                        });
                    }
                    PiAgentMessage::BashExecution(bash) => {
                        let ts = if bash.timestamp > 0 {
                            format_millis_timestamp(bash.timestamp)
                        } else {
                            msg_entry.base.timestamp.clone()
                        };
                        let cmd_input = serde_json::json!({ "command": bash.command });
                        let summary = session::tool_use_summary("Bash", &cmd_input);
                        let is_error = bash.exit_code.map_or(false, |code| code != 0) || bash.cancelled;

                        messages.push(ChatMessage {
                            role: "assistant".to_string(),
                            timestamp: ts.clone(),
                            model: current_model.clone(),
                            token_usage: None,
                            content_parts: vec![
                                ContentPart::ToolUse {
                                    summary,
                                    tool_name: "Bash".to_string(),
                                    input: cmd_input.to_string(),
                                    tool_use_id: None,
                                },
                                ContentPart::tool_result(
                                    "[Bash Output]".to_string(),
                                    bash.output.clone(),
                                    is_error,
                                    false,
                                ),
                            ],
                            is_meta: false,
                            uuid: Some(msg_entry.base.id.clone()),
                        });
                    }
                    PiAgentMessage::CompactionSummary(compaction) => {
                        let ts = if compaction.timestamp > 0 {
                            format_millis_timestamp(compaction.timestamp)
                        } else {
                            msg_entry.base.timestamp.clone()
                        };
                        messages.push(ChatMessage {
                            role: "system".to_string(),
                            timestamp: ts,
                            model: None,
                            token_usage: None,
                            content_parts: vec![ContentPart::Text {
                                text: format!("[Compaction] {}", compaction.summary),
                            }],
                            is_meta: true,
                            uuid: Some(msg_entry.base.id.clone()),
                        });
                    }
                    PiAgentMessage::BranchSummary(summary) => {
                        let ts = if summary.timestamp > 0 {
                            format_millis_timestamp(summary.timestamp)
                        } else {
                            msg_entry.base.timestamp.clone()
                        };
                        messages.push(ChatMessage {
                            role: "system".to_string(),
                            timestamp: ts,
                            model: None,
                            token_usage: None,
                            content_parts: vec![ContentPart::Text {
                                text: format!("[Branch Summary] {}", summary.summary),
                            }],
                            is_meta: true,
                            uuid: Some(msg_entry.base.id.clone()),
                        });
                    }
                    PiAgentMessage::Custom(custom) => {
                        if custom.display {
                            let text = extract_content_text(&custom.content);
                            let ts = if custom.timestamp > 0 {
                                format_millis_timestamp(custom.timestamp)
                            } else {
                                msg_entry.base.timestamp.clone()
                            };
                            messages.push(ChatMessage {
                                role: "system".to_string(),
                                timestamp: ts,
                                model: None,
                                token_usage: None,
                                content_parts: vec![ContentPart::Text {
                                    text: format!("[{}] {}", custom.custom_type, text),
                                }],
                                is_meta: true,
                                uuid: Some(msg_entry.base.id.clone()),
                            });
                        }
                    }
                    PiAgentMessage::System(_) => {}
                }
            }
            PiEntry::Compaction(comp) => {
                messages.push(ChatMessage {
                    role: "system".to_string(),
                    timestamp: comp.base.timestamp.clone(),
                    model: None,
                    token_usage: None,
                    content_parts: vec![ContentPart::Text {
                        text: format!("[Compaction] {}", comp.summary),
                    }],
                    is_meta: true,
                    uuid: Some(comp.base.id.clone()),
                });
            }
            PiEntry::BranchSummary(branch_sum) => {
                messages.push(ChatMessage {
                    role: "system".to_string(),
                    timestamp: branch_sum.base.timestamp.clone(),
                    model: None,
                    token_usage: None,
                    content_parts: vec![ContentPart::Text {
                        text: format!("[Branch Summary] {}", branch_sum.summary),
                    }],
                    is_meta: true,
                    uuid: Some(branch_sum.base.id.clone()),
                });
            }
            _ => {}
        }
    }

    messages
}

/// 提取会话标题：优先活跃分支上的 `session_info` 自定义名称，回退到首条有效用户提问。
pub(crate) fn extract_session_title(entries: &[PiEntry], header: &PiSessionHeader) -> String {
    let active_branch = build_active_branch(entries);
    let branch_set: HashSet<&str> = active_branch.iter().map(String::as_str).collect();

    // 1. 最新 session_info 命名
    for entry in entries.iter().rev() {
        if let PiEntry::SessionInfo(info) = entry {
            if branch_set.contains(info.base.id.as_str()) {
                if let Some(name) = info.name.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
                    return name.to_string();
                }
            }
        }
    }

    // 2. 首条用户提问内容兜底
    for entry in entries {
        if let PiEntry::Message(msg) = entry {
            if branch_set.contains(msg.base.id.as_str()) {
                if let PiAgentMessage::User(user) = &msg.message {
                    let text = extract_content_text(&user.content);
                    let trimmed = text.trim();
                    if !trimmed.is_empty() {
                        if let Some(cleaned) = crate::title_resolver::clean_fallback_user_text(trimmed) {
                            return cleaned;
                        }
                    }
                }
            }
        }
    }

    // 3. 项目名称或默认标题
    Path::new(&header.cwd)
        .file_name()
        .and_then(|n| n.to_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "Untitled Session".to_string())
}

// ─────────────────────────────────────────────────────────────────────────────
// 对外公开解析入口
// ─────────────────────────────────────────────────────────────────────────────

pub(crate) fn parse_session_file(path: &Path) -> AppResult<Vec<ChatMessage>> {
    let content = std::fs::read_to_string(path).map_err(|e| AppError::from(e))?;
    let Some((header, entries)) = parse_entries_from_content(&content) else {
        return Ok(Vec::new());
    };
    Ok(parse_messages_from_entries(&header, &entries))
}

pub(crate) fn load_messages(path: &Path) -> AppResult<SessionLoadResult> {
    let content = std::fs::read_to_string(path).map_err(|e| AppError::from(e))?;
    let file_len = content.len() as u64;
    let Some((header, entries)) = parse_entries_from_content(&content) else {
        return Ok(SessionLoadResult {
            messages: Vec::new(),
            offset: file_len,
            subagent_map: HashMap::new(),
        });
    };
    let messages = parse_messages_from_entries(&header, &entries);
    Ok(SessionLoadResult {
        messages,
        offset: file_len,
        subagent_map: HashMap::new(),
    })
}

pub(crate) fn parse_from_content(content: &str) -> Vec<ChatMessage> {
    let Some((header, entries)) = parse_entries_from_content(content) else {
        return Vec::new();
    };
    parse_messages_from_entries(&header, &entries)
}

pub(crate) fn first_user_message(path: &Path) -> Option<String> {
    let file = File::open(path).ok()?;
    let reader = BufReader::new(file);

    let mut first_user = None;
    for line_res in reader.lines() {
        let Ok(line) = line_res else { break };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(val) = parse_pi_json_value(trimmed) {
            if val.get("type").and_then(Value::as_str) == Some("message") {
                if let Some(msg) = val.get("message") {
                    if msg.get("role").and_then(Value::as_str) == Some("user") {
                        if let Some(content) = msg.get("content") {
                            let text = if let Some(s) = content.as_str() {
                                s.to_string()
                            } else if let Some(arr) = content.as_array() {
                                arr.iter()
                                    .filter_map(|b| b.get("text").and_then(Value::as_str))
                                    .collect::<Vec<_>>()
                                    .join("\n")
                            } else {
                                String::new()
                            };
                            let trimmed = text.trim();
                            if !trimmed.is_empty() {
                                first_user = crate::title_resolver::clean_fallback_user_text(trimmed);
                                break;
                            }
                        }
                    }
                }
            }
        }
    }
    first_user
}

pub(crate) fn session_id(path: &Path) -> Option<String> {
    let file = File::open(path).ok()?;
    let mut reader = BufReader::new(file);
    let mut first_line = String::new();
    reader.read_line(&mut first_line).ok()?;
    let val = parse_pi_json_value(first_line.trim()).ok()?;
    if val.get("type").and_then(Value::as_str) == Some("session") {
        return val.get("id").and_then(Value::as_str).map(str::to_string);
    }
    None
}

pub(crate) fn project_path_raw(path: &Path) -> Option<String> {
    let file = File::open(path).ok()?;
    let mut reader = BufReader::new(file);
    let mut first_line = String::new();
    reader.read_line(&mut first_line).ok()?;
    let val = parse_pi_json_value(first_line.trim()).ok()?;
    if val.get("type").and_then(Value::as_str) == Some("session") {
        return val.get("cwd").and_then(Value::as_str).map(str::to_string);
    }
    None
}

pub(crate) fn is_subagent(path: &Path) -> bool {
    let Ok(file) = File::open(path) else { return false };
    let mut reader = BufReader::new(file);
    let mut first_line = String::new();
    if reader.read_line(&mut first_line).is_err() {
        return false;
    }
    let Ok(val) = parse_pi_json_value(first_line.trim()) else { return false };
    if val.get("type").and_then(Value::as_str) == Some("session") {
        let parent = val.get("parentSession").and_then(Value::as_str);
        return parent.map(|p| !p.trim().is_empty()).unwrap_or(false);
    }
    false
}

pub(crate) fn metadata_only(path: &Path) -> Option<SessionListMetadata> {
    let content = std::fs::read_to_string(path).ok()?;
    let (header, entries) = parse_entries_from_content(&content)?;

    let title = extract_session_title(&entries, &header);
    let sid = header.id.clone();
    let project_path = if header.cwd.trim().is_empty() {
        None
    } else {
        Some(header.cwd.clone())
    };

    let first_ts = header.timestamp.clone();
    let mut last_ts = first_ts.clone();

    for entry in entries.iter().rev() {
        if let Some(ts) = match entry {
            PiEntry::Message(m) => Some(m.base.timestamp.as_str()),
            PiEntry::ModelChange(m) => Some(m.base.timestamp.as_str()),
            PiEntry::ThinkingLevelChange(m) => Some(m.base.timestamp.as_str()),
            PiEntry::Usage(m) => Some(m.base.timestamp.as_str()),
            PiEntry::Compaction(m) => Some(m.base.timestamp.as_str()),
            PiEntry::ContextEdit(m) => Some(m.base.timestamp.as_str()),
            PiEntry::BranchSummary(m) => Some(m.base.timestamp.as_str()),
            PiEntry::Custom(m) => Some(m.base.timestamp.as_str()),
            PiEntry::CustomMessage(m) => Some(m.base.timestamp.as_str()),
            PiEntry::Label(m) => Some(m.base.timestamp.as_str()),
            PiEntry::SessionInfo(m) => Some(m.base.timestamp.as_str()),
            _ => None,
        } {
            if !ts.trim().is_empty() {
                last_ts = ts.to_string();
                break;
            }
        }
    }

    let file_size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);

    Some(SessionListMetadata {
        session_id: sid,
        project_path,
        title: Some(title),
        first_user_message: None,
        first_timestamp: Some(first_ts),
        last_timestamp: Some(last_ts),
        git_branch: String::new(),
        file_size,
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// 全文检索文档抽取
// ─────────────────────────────────────────────────────────────────────────────

pub(crate) fn extract_pi_role_text(val: &Value) -> Option<(String, String)> {
    if val.get("type").and_then(Value::as_str) != Some("message") {
        return None;
    }
    let msg = val.get("message")?;
    let role = msg.get("role").and_then(Value::as_str)?.to_string();

    let text = match role.as_str() {
        "user" => {
            let content = msg.get("content")?;
            if let Some(s) = content.as_str() {
                s.to_string()
            } else if let Some(arr) = content.as_array() {
                arr.iter()
                    .filter_map(|b| b.get("text").and_then(Value::as_str))
                    .collect::<Vec<_>>()
                    .join("\n")
            } else {
                String::new()
            }
        }
        "assistant" => {
            let mut chunks = Vec::new();
            if let Some(blocks) = msg.get("content").and_then(Value::as_array) {
                for b in blocks {
                    if let Some(t) = b.get("text").and_then(Value::as_str) {
                        chunks.push(t);
                    }
                }
            }
            chunks.join("\n")
        }
        _ => return None,
    };

    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some((role, trimmed.to_string()))
}

pub(crate) fn search_docs_from_text(content: &[u8]) -> Option<Vec<SearchDocument>> {
    crate::parser::shared::scan_search_docs_from_text(content, extract_pi_role_text)
}

pub(crate) fn scan_search_docs(
    path: &Path,
    on_progress: &mut dyn FnMut(SearchScanProgress),
) -> Option<Vec<SearchDocument>> {
    crate::parser::shared::scan_search_docs_with_extractor(path, extract_pi_role_text, on_progress)
}

// ─────────────────────────────────────────────────────────────────────────────
// 单元测试
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_session_header_and_entries() {
        let sample = concat!(
            r#"{"type":"session","version":3,"id":"pi-sess-1","timestamp":"2026-06-10T07:00:00Z","cwd":"/repo"}"#,
            "\n",
            r#"{"type":"message","id":"msg-1","parentId":null,"timestamp":"2026-06-10T07:00:01Z","message":{"role":"user","content":"How does this work?","timestamp":1781074801000}}"#,
            "\n",
            r#"{"type":"message","id":"msg-2","parentId":"msg-1","timestamp":"2026-06-10T07:00:02Z","message":{"role":"assistant","content":[{"type":"thinking","thinking":"Analyzing"},{"type":"text","text":"It works well."}],"model":"pi-model","timestamp":1781074802000}}"#,
            "\n"
        );

        let (header, entries) = parse_entries_from_content(sample).unwrap();
        assert_eq!(header.id, "pi-sess-1");
        assert_eq!(header.cwd, "/repo");
        assert_eq!(entries.len(), 2);

        let messages = parse_messages_from_entries(&header, &entries);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, "user");
        match &messages[0].content_parts[0] {
            ContentPart::Text { text } => assert_eq!(text, "How does this work?"),
            _ => panic!("expected text part"),
        }

        assert_eq!(messages[1].role, "assistant");
        assert_eq!(messages[1].content_parts.len(), 2);
        match &messages[1].content_parts[0] {
            ContentPart::Thinking { thinking } => assert_eq!(thinking, "Analyzing"),
            _ => panic!("expected thinking part"),
        }
        match &messages[1].content_parts[1] {
            ContentPart::Text { text } => assert_eq!(text, "It works well."),
            _ => panic!("expected text part"),
        }
    }

    #[test]
    fn repairs_surrogate_escapes_cleanly() {
        let raw = r#"{"type":"message","id":"msg-1","parentId":null,"timestamp":"2026-06-10T07:00:01Z","message":{"role":"user","content":"lone \uD83D char","timestamp":1000}}"#;
        let val = parse_pi_json_value(raw).expect("should repair surrogate and parse");
        let content = val["message"]["content"].as_str().unwrap();
        assert_eq!(content, "lone \u{FFFD} char");
    }

    #[test]
    fn custom_session_info_title_takes_precedence() {
        let sample = concat!(
            r#"{"type":"session","version":3,"id":"pi-sess-1","timestamp":"2026-06-10T07:00:00Z","cwd":"/repo"}"#,
            "\n",
            r#"{"type":"message","id":"msg-1","parentId":null,"timestamp":"2026-06-10T07:00:01Z","message":{"role":"user","content":"Initial prompt","timestamp":1000}}"#,
            "\n",
            r#"{"type":"session_info","id":"info-1","parentId":"msg-1","name":"Custom Pi Title","timestamp":"2026-06-10T07:00:02Z"}"#,
            "\n"
        );

        let (header, entries) = parse_entries_from_content(sample).unwrap();
        let title = extract_session_title(&entries, &header);
        assert_eq!(title, "Custom Pi Title");
    }
}
