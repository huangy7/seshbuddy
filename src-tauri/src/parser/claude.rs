//! Claude 会话元数据扫描、可搜索文本提取辅助，以及 Claude JSONL 解析器。
//! 依赖方向：parser → session / parser → parser::shared。

use crate::error::{AppError, AppResult};
use crate::parser::batch::BatchEmitter;
use crate::parser::shared::{read_last_timestamp_from_file, read_typed_titles_from_tail};
use crate::session::{
    clean_user_message_text, ChatMessage, ContentPart, SessionListMetadata,
    SessionLoadResult, SubagentInfo, TokenUsage, UsageRecord,
};
#[cfg(test)]
use crate::session::tool_use_summary;
use serde_json::Value;
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::Path;

pub(crate) fn scan_claude_metadata_only(file_path: &Path) -> Option<SessionListMetadata> {
    let mut file = File::open(file_path).ok()?;
    let file_size = file.seek(SeekFrom::End(0)).ok()?;
    file.seek(SeekFrom::Start(0)).ok()?;

    let reader = BufReader::new(&file);
    let mut has_chat_messages = false;
    let mut raw_first_user_text = None;
    let mut head_custom_title = None;
    let mut head_ai_title = None;
    let mut first_timestamp = None;
    let mut project_path = None;
    let mut git_branch = String::new();

    // 头部窗口：只读前 50 行提取元数据与首条用户消息，
    // 大文件的 I/O 与逐行分配被严格限制在窗口内
    const HEAD_LINES: usize = 50;
    for (idx, line) in reader.lines().take(HEAD_LINES).enumerate() {
        let line = match line {
            Ok(line) => line,
            Err(_) => continue,
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        // 跳过异常巨大的行（如内嵌超大工具输出的条目），避免 serde_json 卡死
        if trimmed.len() > 256 * 1024 {
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

        if project_path.is_none() {
            project_path = extract_claude_project_path(&entry);
        }

        if git_branch.is_empty() && idx < 20 {
            if let Some(branch) = entry.get("gitBranch").and_then(|v| v.as_str()) {
                let branch = branch.trim();
                if !branch.is_empty() && branch != "HEAD" {
                    git_branch = branch.to_string();
                }
            }
        }

        let entry_type = entry.get("type").and_then(|v| v.as_str()).unwrap_or("");
        match entry_type {
            // 头部偶尔出现的标题行作为种子（尾部窗口的更新值会覆盖）
            "custom-title" => {
                head_custom_title =
                    crate::title_resolver::extract_typed_title(&entry, "customTitle", "custom_title");
            }
            "ai-title" => {
                head_ai_title =
                    crate::title_resolver::extract_typed_title(&entry, "aiTitle", "ai_title");
            }
            _ => {
                if !has_chat_messages {
                    if let Some((role, _)) = extract_claude_role_text(&entry) {
                        has_chat_messages = true;
                        if role == "user" {
                            raw_first_user_text = extract_claude_user_prompt_text(&entry);
                        }
                    }
                }
            }
        }
    }

    // 尾部窗口：custom-title/ai-title 由 Claude Code 追加在文件末尾，
    // 倒序扫描尾部 2MB（首个命中即最新，last-wins），避免全文件扫描
    let (tail_custom_title, tail_ai_title) = read_typed_titles_from_tail(&mut file, file_size);
    let custom_title = tail_custom_title.or(head_custom_title);
    let ai_title = tail_ai_title.or(head_ai_title);

    if !has_chat_messages && custom_title.is_none() && ai_title.is_none() {
        return None;
    }

    let last_timestamp = read_last_timestamp_from_file(&mut file, file_size);

    Some(SessionListMetadata {
        session_id: file_path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("")
            .to_string(),
        project_path,
        title: custom_title.or(ai_title),
        first_user_message: raw_first_user_text
            .as_deref()
            .and_then(crate::title_resolver::clean_fallback_user_text),
        first_timestamp: first_timestamp.clone(),
        last_timestamp: last_timestamp.or(first_timestamp),
        git_branch,
        file_size,
    })
}

pub(crate) fn extract_claude_project_path(entry: &Value) -> Option<String> {
    entry
        .get("cwd")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string())
}

pub(crate) fn extract_claude_role_text(entry: &Value) -> Option<(String, String)> {
    let msg_type = entry.get("type").and_then(|v| v.as_str()).unwrap_or("");
    if msg_type != "user" && msg_type != "assistant" {
        return None;
    }

    if entry
        .get("isSidechain")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        return None;
    }

    let message = entry.get("message")?;
    let role = message
        .get("role")
        .and_then(|v| v.as_str())
        .unwrap_or(msg_type)
        .to_string();
    let text = extract_claude_text(message.get("content")?)?;
    Some((role, text))
}

/// 仅供标题提取使用：取 user 条目中的真实用户文本。
/// 与 extract_claude_role_text 的区别：数组内容只保留 type=="text" 的项，
/// 按项跳过 tool_result（工具输出不应成为会话标题），混合条目中的真实文本不丢失。
pub(crate) fn extract_claude_user_prompt_text(entry: &Value) -> Option<String> {
    let msg_type = entry.get("type").and_then(|v| v.as_str()).unwrap_or("");
    if msg_type != "user" {
        return None;
    }
    if entry
        .get("isSidechain")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        return None;
    }
    let content = entry.get("message")?.get("content")?;
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
        if item.get("type").and_then(|v| v.as_str()) == Some("text") {
            collect_json_string_value(item.get("text"), &mut texts);
        }
    }
    if texts.is_empty() {
        None
    } else {
        Some(texts.join("\n"))
    }
}

pub(crate) fn extract_claude_text(content: &Value) -> Option<String> {
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
        if item.get("type").and_then(|v| v.as_str()) == Some("text") {
            collect_json_string_value(item.get("text"), &mut texts);
        }
    }

    if texts.is_empty() {
        None
    } else {
        Some(texts.join("\n"))
    }
}

pub(crate) fn collect_json_string_value(value: Option<&Value>, texts: &mut Vec<String>) {
    let Some(text) = value.and_then(|value| value.as_str()) else {
        return;
    };
    let trimmed = text.trim();
    if !trimmed.is_empty() {
        texts.push(trimmed.to_string());
    }
}

/// Parse a single JSONL line into a ChatMessage if it's a user or assistant message.
///
/// 走 `claude_entry` 的类型化路径：`message.content` 以裸片段承接、正文边解析边截断，
/// **不再为每一行建 `serde_json::Value` 树**。样本实测 tool_result 正文占全文件 93%，
/// 而其中约 90% 会在 50 KB 截断处被丢弃 —— 这部分分配与反转义原本是纯浪费。
///
/// 语义与改造前逐字段一致；唯一的（有意）差异是坏转义不再拖垮整行，见 `claude_entry` 模块文档。
/// 改造前的实现保留在 `parse_jsonl_line_legacy`，作为等价性测试的金标准。
pub(crate) fn parse_jsonl_line(
    line: &str,
    skip_sidechain: bool,
    subagent_map: Option<&mut HashMap<String, SubagentInfo>>,
    session_file_path: Option<&Path>,
) -> Option<ChatMessage> {
    let entry = crate::parser::claude_entry::ClaudeEntry::parse(line)?;

    let msg_type = entry.entry_type();
    if msg_type != "user" && msg_type != "assistant" {
        return None;
    }

    // Skip sidechain messages unless skip_sidechain is true
    if !skip_sidechain && entry.is_sidechain() {
        return None;
    }

    let message = entry.message()?;

    let role = message.role().unwrap_or(msg_type).to_string();

    let content_parts = match message.content() {
        Some(content) => crate::parser::claude_entry::content_parts(content),
        None => Vec::new(),
    };

    // 注意顺序：子 Agent 注册发生在「内容为空即丢弃」之前，
    // 也就是空内容条目同样会登记 subagent（改造前如此，勿调换）
    if let (Some(map), Some(file_path)) = (subagent_map, session_file_path) {
        if role == "user" {
            if let Some(agent_id) = entry.subagent_id() {
                if let Some(tool_use_id) =
                    message.content().and_then(crate::parser::claude_entry::first_tool_use_id)
                {
                    if let (Some(parent), Some(stem)) = (file_path.parent(), file_path.file_stem()) {
                        let direct_file = parent.join(stem).join("subagents").join(format!("agent-{}.jsonl", agent_id));
                        let subagent_file = if direct_file.exists() {
                            Some(direct_file)
                        } else {
                            // 穿透检索兄弟会话目录：解决 fork 会话继承了父会话的子代理调用，
                            // 但子代理物理文件保存在原父会话 subagents 目录下的跨分支关联问题
                            let mut found = None;
                            if let Ok(entries) = std::fs::read_dir(parent) {
                                for entry in entries.flatten() {
                                    if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                                        let candidate = entry.path().join("subagents").join(format!("agent-{}.jsonl", agent_id));
                                        if candidate.exists() {
                                            found = Some(candidate);
                                            break;
                                        }
                                    }
                                }
                            }
                            found
                        };
                        if let Some(subagent_file) = subagent_file {
                            // agentId 可能短于 6 字节，直接切片会 panic
                            let short = agent_id.get(..6).unwrap_or(agent_id);
                            map.insert(tool_use_id, SubagentInfo {
                                file_path: subagent_file.to_string_lossy().to_string(),
                                label: format!("Subagent {}", short),
                            });
                        }
                    }
                }
            }
        }
    }

    if content_parts.is_empty() {
        return None;
    }

    let token_usage = message.usage().map(|u| TokenUsage {
        input_tokens: u.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
        output_tokens: u.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
        cache_creation_input_tokens: u.get("cache_creation_input_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
        cache_read_input_tokens: u.get("cache_read_input_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
    });

    Some(ChatMessage {
        role,
        timestamp: entry.timestamp(),
        model: message.model(),
        token_usage,
        content_parts,
        is_meta: entry.is_meta(),
        uuid: entry.uuid(),
    })
}

/// 改造前的实现（`serde_json::Value` 树），**仅用于等价性测试与性能对照**，不参与生产构建。
#[cfg(test)]
pub(crate) fn parse_jsonl_line_legacy(
    line: &str,
    skip_sidechain: bool,
    subagent_map: Option<&mut HashMap<String, SubagentInfo>>,
    session_file_path: Option<&Path>,
) -> Option<ChatMessage> {
    let entry: serde_json::Value = serde_json::from_str(line).ok()?;

    let msg_type = entry.get("type").and_then(|v| v.as_str()).unwrap_or("");
    if msg_type != "user" && msg_type != "assistant" {
        return None;
    }

    // Skip sidechain messages unless skip_sidechain is true
    if !skip_sidechain && entry
        .get("isSidechain")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        return None;
    }

    let timestamp = entry
        .get("timestamp")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let message = entry.get("message")?;

    let role = message
        .get("role")
        .and_then(|v| v.as_str())
        .unwrap_or(msg_type)
        .to_string();

    let model = message
        .get("model")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let content_parts = parse_content(message.get("content"));

    if let (Some(map), Some(file_path)) = (subagent_map, session_file_path) {
        if role == "user" {
            if let Some(agent_id) = entry.get("toolUseResult").and_then(|v| v.get("agentId")).and_then(|v| v.as_str()) {
                if let Some(content) = message.get("content").and_then(|c| c.as_array()) {
                    if let Some(first) = content.first() {
                        if let Some(tool_use_id) = first.get("tool_use_id").and_then(|v| v.as_str()) {
                            if let (Some(parent), Some(stem)) = (file_path.parent(), file_path.file_stem()) {
                                let direct_file = parent.join(stem).join("subagents").join(format!("agent-{}.jsonl", agent_id));
                                let subagent_file = if direct_file.exists() {
                                    Some(direct_file)
                                } else {
                                    let mut found = None;
                                    if let Ok(entries) = std::fs::read_dir(parent) {
                                        for entry in entries.flatten() {
                                            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                                                let candidate = entry.path().join("subagents").join(format!("agent-{}.jsonl", agent_id));
                                                if candidate.exists() {
                                                    found = Some(candidate);
                                                    break;
                                                }
                                            }
                                        }
                                    }
                                    found
                                };
                                if let Some(subagent_file) = subagent_file {
                                    // 此处刻意保留改造前的 `&agent_id[..6]`（agentId 短于 6 字节会 panic），
                                    // 让本函数保持「纯金标准」；生产路径已改为安全截取，见 parse_jsonl_line
                                    map.insert(tool_use_id.to_string(), SubagentInfo {
                                        file_path: subagent_file.to_string_lossy().to_string(),
                                        label: format!("Subagent {}", &agent_id[..6]),
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    if content_parts.is_empty() {
        return None;
    }

    let token_usage = message.get("usage").map(|u| TokenUsage {
        input_tokens: u.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
        output_tokens: u.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
        cache_creation_input_tokens: u.get("cache_creation_input_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
        cache_read_input_tokens: u.get("cache_read_input_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
    });

    Some(ChatMessage {
        role,
        timestamp,
        model,
        token_usage,
        content_parts,
        is_meta: entry.get("isMeta").and_then(|v| v.as_bool()).unwrap_or(false),
        uuid: entry
            .get("uuid")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
    })
}

pub(crate) fn has_chat_type_marker(text: &str) -> bool {
    text.contains(r#""type":"user""#)
        || text.contains(r#""type": "user""#)
        || text.contains(r#""type":"assistant""#)
        || text.contains(r#""type": "assistant""#)
}

pub(crate) fn chat_line_candidate(line: &str) -> bool {
    // Ensure prefix + suffix windows overlap by using at least half the line length.
    let window = std::cmp::max(1024, line.len() / 2 + 128);
    let mut prefix_end = std::cmp::min(window, line.len());
    while prefix_end > 0 && !line.is_char_boundary(prefix_end) {
        prefix_end -= 1;
    }
    if prefix_end > 0 && has_chat_type_marker(&line[..prefix_end]) {
        return true;
    }
    if line.len() <= window {
        return false;
    }
    let mut suffix_start = line.len().saturating_sub(window);
    while suffix_start < line.len() && !line.is_char_boundary(suffix_start) {
        suffix_start += 1;
    }
    if suffix_start < line.len() && suffix_start < prefix_end + 1024 {
        has_chat_type_marker(&line[suffix_start..])
    } else {
        false
    }
}

pub(crate) fn parse_session_file(file_path: &str) -> AppResult<Vec<ChatMessage>> {
    use std::io::{BufRead, BufReader};
    let file = std::fs::File::open(file_path)?;

    // 大文件走并行分块内核
    if file.metadata().map(|m| m.len()).unwrap_or(0) >= PARALLEL_MIN_FILE_SIZE {
        let file_size = file.metadata()?.len();
        let mut messages = Vec::new();
        parse_session_parallel_with(file_path, file_size, false, &mut |chunk| {
            messages.extend(chunk);
            true
        })?;
        return Ok(messages);
    }

    let mut reader = BufReader::with_capacity(256 * 1024, file);
    let mut messages = Vec::new();
    let mut buf = String::new();
    let mut pos = 0u64;

    loop {
        buf.clear();
        let line_offset = pos;
        match reader.read_line(&mut buf) {
            Ok(0) => break,
            Err(_) => continue,
            Ok(n) => pos += n as u64,
        }
        let line = buf.trim();
        if line.is_empty() {
            continue;
        }
        if !chat_line_candidate(line) {
            continue;
        }
        if let Some(mut msg) = parse_jsonl_line(line, false, None, None) {
            backfill_source_offset(&mut msg, line_offset);
            messages.push(msg);
        }
    }

    Ok(messages)
}

/// `String` 错误槽是空置的：函数体从不返回 `Err`（畸形行静默跳过）。
/// 保留 `AppResult` 是为了与同族入口同型，不是为了它的错误分支。
pub(crate) fn parse_session_from_string(content: &str) -> AppResult<Vec<ChatMessage>> {
    let mut messages = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(msg) = parse_jsonl_line(line, false, None, None) {
            messages.push(msg);
        }
    }
    Ok(messages)
}

/// 把该行的字节 offset 回填到消息内所有 tool_result part（懒加载定位符，
/// append-only 文件下永久稳定，`get_tool_result_full_content` 按它 seek 取全文）。
fn backfill_source_offset(msg: &mut ChatMessage, offset: u64) {
    for part in &mut msg.content_parts {
        if let ContentPart::ToolResult { source_offset, .. } = part {
            *source_offset = offset;
        }
    }
}

/// 懒加载回取：seek 到 `offset` 读一行，反漂移校验（part 类型 + tool_use_id）后
/// 返回该 tool_result 的全文（50 KB 截断后，与当初解析时的口径一致）。
///
/// 校验不过（offset 漂移 / 文件被外部改写 / 该行无此 part）返回明确错误，
/// 前端据此回退展示预览。
pub(crate) fn tool_result_full_content_at(
    file_path: &str,
    offset: u64,
    tool_use_id: Option<&str>,
) -> AppResult<String> {
    // 三条 I/O 失败把 OS 的报错文本放进 `detail`（R3：底层文本不进语言包，否则每个
    // 平台各写一份译文）；两条校验失败没有底层文本可带，故不带参数。
    let file = File::open(file_path)
        .map_err(|e| AppError::coded("parser.file_open_failed").with("detail", e.to_string()))?;
    let mut reader = BufReader::with_capacity(256 * 1024, file);
    reader
        .seek(SeekFrom::Start(offset))
        .map_err(|e| AppError::coded("parser.file_seek_failed").with("detail", e.to_string()))?;
    // tool_result 行可达 MB 级，read_line 一次读全
    let mut line = String::new();
    let read = reader
        .read_line(&mut line)
        .map_err(|e| AppError::coded("parser.file_read_failed").with("detail", e.to_string()))?;
    if read == 0 {
        return Err(AppError::coded("parser.offset_out_of_range"));
    }
    crate::parser::claude_entry::tool_result_full_content_in_line(line.trim_end(), tool_use_id)
        .ok_or_else(|| AppError::coded("parser.tool_result_mismatch"))
}

/// 并行分块解析的最小文件阈值：小于该值时线程启动/归并开销不划算，走单线程原路径。
const PARALLEL_MIN_FILE_SIZE: u64 = 16 * 1024 * 1024;
/// 后续稳态分块的目标大小（32MB）：保证多核 CPU 吞吐量拉满的同时，
/// 避免单块过大导致主线程 join 阻塞停顿。
const PARALLEL_STEADY_CHUNK_SIZE: u64 = 32 * 1024 * 1024;
/// 阶梯渐进切块序列（Ramp-up）：
/// - 块 0：4MB（包含完整前数轮对话，首屏瞬间铺满，耗时仅 1~3ms）
/// - 块 1：4MB（第二批紧随其后，数字平滑增长）
/// - 块 2：8MB（第三批平滑衔接）
/// - 块 3：16MB（第四批平滑衔接）
/// 之后进入 32MB 稳态并发。彻底消除“卡在 3 条消息死等数秒”的停顿体验。
const PARALLEL_RAMP_UP_SIZES: [u64; 4] = [
    4 * 1024 * 1024,
    4 * 1024 * 1024,
    8 * 1024 * 1024,
    16 * 1024 * 1024,
];

/// 解析一个字节范围块 `[start, end)`（Hadoop split 规则，保证每行恰好归属一个块）：
/// - 非首块无条件丢弃第一个 read_line：start 落在行中间时丢的是行尾（该行已由前一块
///   完整处理）；start 恰好是行起点时丢的是完整行 —— 它同样归前一块，因为前一块用
///   **`pos <= end`（闭区间）** 会把起点 == end 的行读完。
/// - 本块处理「skip 之后、行起点 <= end」的行；read_line 返回 0（EOF）自然终止。
/// 块内顺序 = 文件顺序；subagent_map 仅本块内构建。
fn parse_chunk_lines(
    file_path: &str,
    start: u64,
    end: u64,
    skip_sidechain: bool,
) -> AppResult<(Vec<ChatMessage>, HashMap<String, SubagentInfo>)> {
    use std::io::{BufRead, BufReader, Seek, SeekFrom};
    let mut file = std::fs::File::open(file_path)?;
    file.seek(SeekFrom::Start(start))?;
    let mut reader = BufReader::with_capacity(256 * 1024, file);
    let mut messages = Vec::new();
    let mut subagent_map = HashMap::new();
    let session_path = std::path::Path::new(file_path);
    let mut buf = String::new();
    let mut pos = start;

    if start > 0 {
        // 丢弃跨块边界的行（或其尾部）。'\n' 是 ASCII（0x0A），UTF-8 多字节序列的
        // 字节均 >= 0x80，字节级查找安全。
        // **必须用字节级 `read_until`**：start 可能落在多字节字符中间，
        // `read_line(&mut String)` 会做 UTF-8 校验并返回 InvalidData 错误，
        // 导致整块被静默丢弃（曾被偶发的「恰好落在字符边界」掩盖）。
        let mut skip_buf = Vec::new();
        match reader.read_until(b'\n', &mut skip_buf) {
            Ok(0) => return Ok((messages, subagent_map)), // start 之后已无内容
            Ok(n) => pos += n as u64,
            // 读错误无法安全续读（pos 不前进会死循环），按块终止处理；
            // 与单线程 `Err(_) => continue` 仅在 I/O 故障时才有理论差异。
            Err(_) => return Ok((messages, subagent_map)),
        }
    }

    // 注意是 `pos <= end`（闭区间）：起点恰好 == end 的行归本块；下一块的 skip
    // 会丢弃同一行。若错用 `<`，该行会被两边同时漏掉（off-by-one 丢行）。
    // skip 之后 pos 必然在行起点，后续整行读取都是合法 UTF-8，`read_line` 安全。
    while pos <= end {
        buf.clear();
        let line_offset = pos; // 行起点 offset（懒加载定位符）
        let n = match reader.read_line(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(_) => break, // 同上：避免 pos 不前进死循环
        };
        pos += n as u64;
        let line = buf.trim();
        if line.is_empty() || !chat_line_candidate(line) {
            continue;
        }
        if let Some(mut msg) = parse_jsonl_line(line, skip_sidechain, Some(&mut subagent_map), Some(session_path)) {
            backfill_source_offset(&mut msg, line_offset);
            messages.push(msg);
        }
    }

    Ok((messages, subagent_map))
}

/// 计算并行切块：`(start, end)` 列表，覆盖 `[0, file_size)`，按文件顺序排列。
/// 采用阶梯平滑渐进切块：
/// 1. 前置阶梯块（4MB、4MB、8MB、16MB）：确保前数批次以毫秒级平滑递增涌入前端，
///    首批 4MB 包含数轮完整对话，首屏瞬间填满，彻底告别卡死在 3 条消息的停顿体验；
/// 2. 剩余大区间按 [`PARALLEL_STEADY_CHUNK_SIZE`]（32MB）均分，跑满多核 CPU 吞吐量。
fn parallel_chunk_ranges(file_size: u64) -> Vec<(u64, u64)> {
    let mut ranges = Vec::new();
    let mut curr = 0u64;

    // 1. 前置阶梯平滑块：确保前几批次以毫秒级平滑递增涌入前端
    for &sz in &PARALLEL_RAMP_UP_SIZES {
        if curr + sz < file_size {
            ranges.push((curr, curr + sz));
            curr += sz;
        } else {
            break;
        }
    }

    // 2. 剩余区间按稳态大小均分（保底至少 1 块）
    let rem = file_size - curr;
    if rem > 0 {
        let count = ((rem + PARALLEL_STEADY_CHUNK_SIZE - 1) / PARALLEL_STEADY_CHUNK_SIZE).max(1);
        let chunk_sz = (rem + count - 1) / count;
        for i in 0..count {
            let start = curr + i * chunk_sz;
            let end = (curr + (i + 1) * chunk_sz).min(file_size);
            if start < end {
                ranges.push((start, end));
            }
        }
    }

    ranges
}

/// 并行分块全量解析：scoped 线程并行解析各块，主线程**按块序** join 并逐块回调
/// `on_chunk`（可在此流式 emit，保留首批体验）。
/// 等价性论证：
/// - 每行恰好归属一个块（Hadoop split）→ 块序归并后消息序列与单线程完全一致；
/// - `subagent_map` 在 `parse_jsonl_line` 中只写不读，唯一顺序语义是同 key last-wins
///   → 按块序 `extend` 合并与单线程一致；
/// - `BatchEmitter` 在主线程按归并顺序喂消息，批次边界可能与单线程不同，但消息顺序不变。
fn parse_session_parallel_with<F>(
    file_path: &str,
    file_size: u64,
    skip_sidechain: bool,
    on_chunk: &mut F,
) -> AppResult<HashMap<String, SubagentInfo>>
where
    // 返回 false 表示停止归并（对应单线程早退）；此时返回已收集的部分 subagent_map，
    // 与单线程早退时返回部分 map 的语义一致。已 spawn 的块线程在 scope 退出时跑完即弃。
    F: FnMut(Vec<ChatMessage>) -> bool,
{
    let ranges = parallel_chunk_ranges(file_size);
    let path = file_path.to_string();
    let mut subagent_map = HashMap::new();

    std::thread::scope(|s| -> AppResult<()> {
        let handles: Vec<_> = ranges
            .iter()
            .map(|&(start, end)| {
                let path = &path;
                s.spawn(move || parse_chunk_lines(path, start, end, skip_sidechain))
            })
            .collect();
        for h in handles {
            let (msgs, chunk_map) = h.join().expect("并行解析块 panic")?;
            subagent_map.extend(chunk_map); // 后块同 key 覆盖前块 = last-wins
            if !on_chunk(msgs) {
                break;
            }
        }
        Ok(())
    })?;

    Ok(subagent_map)
}

/// 全量解析一个会话 JSONL 文件，返回消息与文件水位。
///
/// 生产链路走 `parse_session_incremental` / `parse_session_file_streaming`，本函数仅供
/// 等价性对拍测试与 `parse_bench` 基准使用 —— 它保留的是未做流式分块的完整装配路径。
#[cfg(test)]
pub(crate) fn parse_session_file_with_offset(file_path: &str, skip_sidechain: bool) -> AppResult<SessionLoadResult> {
    use std::io::{BufRead, BufReader};
    let file = std::fs::File::open(file_path)?;
    let file_size = file.metadata()?.len();

    // 大文件走并行分块内核（行为零变更，等价性由对拍测试保证）
    if file_size >= PARALLEL_MIN_FILE_SIZE {
        let mut messages = Vec::new();
        let subagent_map = parse_session_parallel_with(file_path, file_size, skip_sidechain, &mut |chunk| {
            messages.extend(chunk);
            true
        })?;
        return Ok(SessionLoadResult {
            messages,
            offset: file_size,
            subagent_map,
        });
    }

    let mut reader = BufReader::with_capacity(256 * 1024, file);
    let mut messages = Vec::new();
    let mut subagent_map = HashMap::new();
    let session_path = std::path::Path::new(file_path);
    let mut buf = String::new();
    let mut pos = 0u64;

    loop {
        buf.clear();
        let line_offset = pos;
        match reader.read_line(&mut buf) {
            Ok(0) => break,
            Err(_) => continue,
            Ok(n) => pos += n as u64,
        }
        let line = buf.trim();
        if line.is_empty() {
            continue;
        }
        if !chat_line_candidate(line) {
            continue;
        }
        if let Some(mut msg) = parse_jsonl_line(line, skip_sidechain, Some(&mut subagent_map), Some(session_path)) {
            backfill_source_offset(&mut msg, line_offset);
            messages.push(msg);
        }
    }

    Ok(SessionLoadResult {
        messages,
        offset: file_size,
        subagent_map,
    })
}

/// Parse incremental content from a session file starting at the given byte offset.
pub(crate) fn parse_session_incremental(file_path: &str, offset: u64, skip_sidechain: bool) -> AppResult<SessionLoadResult> {
    use std::io::{BufRead, BufReader, Seek, SeekFrom};
    let mut file = std::fs::File::open(file_path)?;
    let file_size = file.metadata()?.len();

    if file_size <= offset {
        return Ok(SessionLoadResult {
            messages: vec![],
            offset,
            subagent_map: HashMap::new(),
        });
    }

    file.seek(SeekFrom::Start(offset))?;
    let mut reader = BufReader::with_capacity(256 * 1024, file);
    let mut messages = Vec::new();
    let mut subagent_map = HashMap::new();
    let session_path = std::path::Path::new(file_path);
    let mut buf = String::new();
    let mut pos = offset; // 增量读取：行 offset 从 seek 位置起累加

    loop {
        buf.clear();
        let line_offset = pos;
        match reader.read_line(&mut buf) {
            Ok(0) => break,
            Err(_) => continue,
            Ok(n) => pos += n as u64,
        }
        let line = buf.trim();
        if line.is_empty() {
            continue;
        }
        if !chat_line_candidate(line) {
            continue;
        }
        if let Some(mut msg) = parse_jsonl_line(line, skip_sidechain, Some(&mut subagent_map), Some(session_path)) {
            backfill_source_offset(&mut msg, line_offset);
            messages.push(msg);
        }
    }

    Ok(SessionLoadResult {
        messages,
        offset: file_size,
        subagent_map,
    })
}

/// 流式解析回调签名：每收到一批消息就调用一次（批次大小见 `parser::batch`，全 CLI 统一）。
/// 返回 `false` 表示调用方希望中止（目前未使用，留作未来取消支持）。
pub(crate) fn parse_session_file_streaming<F>(
    file_path: &str,
    skip_sidechain: bool,
    mut on_batch: F,
) -> AppResult<(u64, HashMap<String, SubagentInfo>)>
where
    F: FnMut(Vec<ChatMessage>) -> bool,
{
    use std::io::{BufRead, BufReader};
    let file = std::fs::File::open(file_path)?;
    let file_size = file.metadata()?.len();

    // 大文件走并行分块内核；按块序 join 逐块喂 BatchEmitter，
    // 首批延迟 ≈ 首块解析时间，流式体验与单线程一致。
    if file_size >= PARALLEL_MIN_FILE_SIZE {
        let mut emitter = BatchEmitter::new();
        let mut cancelled = false;
        let mut is_first_chunk = true;
        let subagent_map = parse_session_parallel_with(file_path, file_size, skip_sidechain, &mut |chunk| {
            for msg in chunk {
                emitter.push(msg);
                if !emitter.maybe_flush(&mut on_batch) {
                    cancelled = true;
                    return false;
                }
            }
            if is_first_chunk {
                is_first_chunk = false;
                // 首块（512KB）处理完毕后立即强制发车，确保首屏消息（通常数十条）
                // 在几毫秒内抵达前端，绝不将首屏消息积压在 buffer 中等待后续长任务。
                if !emitter.flush_now(&mut on_batch) {
                    cancelled = true;
                    return false;
                }
            }
            true
        })?;
        // 与单线程早退语义一致：取消时丢弃 emitter 缓冲（不 flush_remaining）
        if !cancelled {
            emitter.flush_remaining(&mut on_batch);
        }
        return Ok((file_size, subagent_map));
    }

    let mut reader = BufReader::with_capacity(256 * 1024, file);
    let mut subagent_map = HashMap::new();
    let session_path = std::path::Path::new(file_path);
    let mut buf = String::new();
    let mut emitter = BatchEmitter::new();
    let mut pos = 0u64;

    loop {
        buf.clear();
        let line_offset = pos;
        match reader.read_line(&mut buf) {
            Ok(0) => break,
            Err(_) => continue,
            Ok(n) => pos += n as u64,
        }
        let line = buf.trim();
        if line.is_empty() {
            continue;
        }
        if !chat_line_candidate(line)
        {
            continue;
        }
        if let Some(mut msg) = parse_jsonl_line(line, skip_sidechain, Some(&mut subagent_map), Some(session_path)) {
            backfill_source_offset(&mut msg, line_offset);
            emitter.push(msg);
            if !emitter.maybe_flush(&mut on_batch) {
                return Ok((file_size, subagent_map));
            }
        }
    }

    emitter.flush_remaining(&mut on_batch);

    Ok((file_size, subagent_map))
}

/// Detect skill content or system caveats and return a collapsible ToolResult.
pub(crate) fn try_parse_collapsible(text: &str) -> Option<ContentPart> {
    // Skill content: "Base directory for this skill: /path/to/skill-name\n..."
    if text.starts_with("Base directory for this skill:") {
        let summary = text
            .lines()
            .find(|l| l.starts_with("# "))
            .map(|l| l.trim_start_matches("# ").to_string())
            .unwrap_or_else(|| {
                text.lines()
                    .next()
                    .unwrap_or("")
                    .rsplit('/')
                    .next()
                    .unwrap_or("Skill")
                    .to_string()
            });
        return Some(ContentPart::tool_result(
            format!("[Skill: {}]", summary),
            text.to_string(),
            false,
            false,
        ));
    }

    // Local command caveat: "<local-command-caveat>..."
    if text.starts_with("<local-command-caveat>") {
        return Some(ContentPart::tool_result(
            "[System caveat]".to_string(),
            text.to_string(),
            false,
            false,
        ));
    }

    // CLI 粘贴图片时生成的 meta 引用行: "[Image: source: /path/to/img.png]"
    if text.starts_with("[Image: source: ") && text.trim_end().ends_with(']') {
        let trimmed = text.trim_end();
        let path = trimmed["[Image: source: ".len()..trimmed.len() - 1].trim();
        if !path.is_empty() {
            return Some(ContentPart::ImageRef {
                path: path.to_string(),
            });
        }
        return Some(ContentPart::ImageMeta);
    }

    None
}

/// 改造前的 `Value` 版内容解析，**仅供 `parse_jsonl_line_legacy` 与等价性测试使用**。
/// 生产路径已切到 `claude_entry::parse_content_raw`。
#[cfg(test)]
pub(crate) fn parse_content(content: Option<&serde_json::Value>) -> Vec<ContentPart> {
    let content = match content {
        Some(c) => c,
        None => return Vec::new(),
    };

    if let Some(text) = content.as_str() {
        if let Some(part) = try_parse_collapsible(text) {
            return vec![part];
        }
        return vec![ContentPart::Text {
            text: text.to_string(),
        }];
    }

    if let Some(arr) = content.as_array() {
        let mut parts = Vec::new();
        for item in arr {
            let item_type = item.get("type").and_then(|v| v.as_str()).unwrap_or("");
            match item_type {
                "text" => {
                    if let Some(text) = item.get("text").and_then(|v| v.as_str()) {
                        if !text.is_empty() {
                            if let Some(part) = try_parse_collapsible(text) {
                                parts.push(part);
                            } else {
                                parts.push(ContentPart::Text {
                                    text: text.to_string(),
                                });
                            }
                        }
                    }
                }
                "tool_use" => {
                    let name = item.get("name").and_then(|v| v.as_str()).unwrap_or("Tool");
                    let tool_use_id = item.get("id").and_then(|v| v.as_str()).map(|s| s.to_string());
                    let input_val = item
                        .get("input")
                        .cloned()
                        .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));
                    let summary = tool_use_summary(name, &input_val);
                    let input_str = serde_json::to_string_pretty(&input_val)
                        .unwrap_or_default();
                    parts.push(ContentPart::ToolUse {
                        summary,
                        tool_name: name.to_string(),
                        input: input_str,
                        tool_use_id,
                    });
                }
                "thinking" => {
                    if let Some(thinking) = item.get("thinking").and_then(|v| v.as_str()) {
                        if !thinking.is_empty() {
                            parts.push(ContentPart::Thinking {
                                thinking: thinking.to_string(),
                            });
                        }
                    }
                }
                "image" => {
                    if let Some(source) = item.get("source") {
                        let source_type = source.get("type").and_then(|v| v.as_str()).unwrap_or("");
                        if source_type == "base64" {
                            let media_type = source.get("media_type").and_then(|v| v.as_str()).unwrap_or("image/png");
                            if let Some(data) = source.get("data").and_then(|v| v.as_str()) {
                                parts.push(ContentPart::Image {
                                    media_type: media_type.to_string(),
                                    data: data.to_string(),
                                });
                            }
                        }
                    }
                }
                "tool_result" => {
                    let is_error = item.get("is_error").and_then(|v| v.as_bool()).unwrap_or(false);
                    let content_val = item.get("content");

                    let mut content_str = String::new();
                    if let Some(arr) = content_val.and_then(|v| v.as_array()) {
                        for sub in arr {
                            if let Some(text) = sub.get("text").and_then(|v| v.as_str()) {
                                content_str.push_str(text);
                                content_str.push('\n');
                            }
                        }
                    } else if let Some(text) = content_val.and_then(|v| v.as_str()) {
                        content_str = text.to_string();
                    }

                    if !content_str.is_empty() {
                        // 截断口径取自 typed 路径的同一函数：提示语已移出正文，只留
                        // `truncated` 标志，故对拍测试能逐字段一致（含该标志）。
                        let (content_str, truncated) =
                            crate::parser::claude_entry::truncate_tool_result(content_str);
                        let summary = if is_error { "[Tool Error]" } else { "[Tool Result]" }.to_string();
                        let full_len = content_str.len() as u64;
                        let tool_use_id = item
                            .get("tool_use_id")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());
                        // 与 typed 路径共用同一预览函数，保证对拍逐字段一致
                        let (content_str, truncated_preview) =
                            crate::parser::claude_entry::preview_tool_result(content_str);
                        parts.push(ContentPart::ToolResult {
                            summary,
                            content: content_str,
                            is_error,
                            tool_use_id,
                            full_len,
                            truncated_preview,
                            truncated,
                            source_offset: 0, // 由调用方回填（legacy 对照同步此约定）
                        });
                    }
                }
                _ => {}
            }
        }
        return parts;
    }

    Vec::new()
}

/// Read the first user message from a JSONL file (first 30 chars) for display name fallback.
pub(crate) fn read_first_user_message(file_path: &str) -> Option<String> {
    use std::io::{BufRead, BufReader};
    let file = std::fs::File::open(file_path).ok()?;
    let reader = BufReader::new(file);
    // Only check the first 50 lines to avoid reading huge files
    for line in reader.lines().take(50) {
        let line = match line {
            Ok(l) => l,
            Err(_) => continue,
        };
        let line = line.trim().to_string();
        if line.is_empty() {
            continue;
        }
        // 跳过异常巨大的行（如内嵌数百 MB 工具输出的 sidechain 条目），避免 serde_json 卡死
        if line.len() > 256 * 1024 {
            continue;
        }
        let entry: serde_json::Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let msg_type = entry.get("type").and_then(|v| v.as_str()).unwrap_or("");
        if msg_type != "user" {
            continue;
        }
        let message = entry.get("message")?;
        let content = message.get("content")?;
        let text = if let Some(s) = content.as_str() {
            s.to_string()
        } else if let Some(arr) = content.as_array() {
            arr.iter()
                .find_map(|item| {
                    if item.get("type").and_then(|v| v.as_str()) == Some("text") {
                        item.get("text").and_then(|v| v.as_str()).map(|s| s.to_string())
                    } else {
                        None
                    }
                })
                .unwrap_or_default()
        } else {
            continue;
        };
        let cleaned = match clean_user_message_text(&text) {
            Some(c) => c,
            None => continue,
        };
        let truncated: String = cleaned.chars().take(30).collect();
        let display = if cleaned.chars().count() > 30 {
            format!("{}...", truncated)
        } else {
            truncated
        };
        return Some(display);
    }
    None
}

/// Extract usage records from a JSONL session file.
/// Reads assistant entries for token usage and system/turn_duration for duration.
///
/// 性能注意：全文件约 93% 体积是 tool_result 正文，与 usage 完全无关。
/// - 缓冲与主解析路径一致（256 KB）；
/// - 先用子串预筛（`"usage"` / `turn_duration` 必含其一）跳过无关行，**零建树**；
///   注意不能复用 `chat_line_candidate`——它明确拒绝 system 行，而 duration 恰恰在 system 行上。
/// - 命中行走 `claude_entry` 类型化零拷贝：`message.content` 不建树。
/// 分流必须按解析后的 `entry_type`（不能按预筛命中）：assistant 行的 tool_result 正文
/// 可能恰好包含 "turn_duration" 字样。
pub(crate) fn extract_usage_records(file_path: &str, project: &str) -> Vec<UsageRecord> {
    use std::io::{BufRead, BufReader};
    let file = match std::fs::File::open(file_path) {
        Ok(f) => f,
        Err(_) => return Vec::new(),
    };
    let mut reader = BufReader::with_capacity(256 * 1024, file);
    let mut records: Vec<UsageRecord> = Vec::new();
    let mut line = String::new();

    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {}
            Err(_) => continue,
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if !trimmed.contains("\"usage\"") && !trimmed.contains("turn_duration") {
            continue;
        }

        // 与旧 Value 路径等价：`serde_json::from_str` 失败的行跳过。
        let Some(entry) = crate::parser::claude_entry::ClaudeEntry::parse(trimmed) else {
            continue;
        };
        match entry.entry_type() {
            // system/turn_duration 行极小，补一次 Value 解析取 subtype/durationMs
            //（ClaudeEntry 刻意不含这两个字段）。
            "system" => {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) {
                    if v.get("subtype").and_then(|s| s.as_str()) == Some("turn_duration") {
                        if let Some(dur) = v.get("durationMs").and_then(|d| d.as_u64()) {
                            // Attach duration to the last record (the assistant turn it follows)
                            if let Some(last) = records.last_mut() {
                                last.duration_ms = Some(dur);
                            }
                        }
                    }
                }
                continue;
            }
            "assistant" => {}
            _ => continue,
        }

        let Some(message) = entry.message() else {
            continue;
        };
        let Some(usage) = message.usage() else {
            continue;
        };

        let input_tokens = usage.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
        let output_tokens = usage.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
        let cache_creation_tokens = usage.get("cache_creation_input_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
        let cache_read_tokens = usage.get("cache_read_input_tokens").and_then(|v| v.as_u64()).unwrap_or(0);

        // Skip entries with zero usage
        if input_tokens == 0 && output_tokens == 0 && cache_creation_tokens == 0 && cache_read_tokens == 0 {
            continue;
        }

        let model = message.model().unwrap_or_else(|| "unknown".to_string());
        let timestamp = entry.timestamp();

        // Convert ISO timestamp to local date YYYY-MM-DD
        let date = timestamp_to_local_date(&timestamp);

        records.push(UsageRecord {
            date,
            model,
            input_tokens,
            output_tokens,
            cache_creation_tokens,
            cache_read_tokens,
            duration_ms: None,
            project: project.to_string(),
        });
    }

    records
}

/// Convert ISO 8601 timestamp to local date string YYYY-MM-DD.
pub(crate) fn timestamp_to_local_date(ts: &str) -> String {
    use chrono::{DateTime, Local, Utc};
    match ts.parse::<DateTime<Utc>>() {
        Ok(utc) => {
            let local: DateTime<Local> = utc.with_timezone(&Local);
            local.format("%Y-%m-%d").to_string()
        }
        Err(_) => "unknown".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    /// 取 `Coded` 错误的 code。
    ///
    /// 断言 code 而不是断言 `err.to_string()` 里含某段中文：`Coded` 的 `Display` 就是 code
    /// （见 `error.rs` 的 `#[error("{code}")]`），文案已移到语言包、随语言变，
    /// 而跨 IPC 的协议字段就是 code——断它才是断在契约上。
    fn coded_code(err: AppError) -> &'static str {
        match err {
            AppError::Coded { code, .. } => code,
            other => panic!("期望 Coded 错误，实际拿到 {other:?}"),
        }
    }

    /// ★ 行为级回归守卫（`parser/mod.rs` 那条守卫覆盖不到的**大文件内核**）：
    /// `parse_chunk_lines` 与 `parse_session_parallel_with` 只在 `file_size >=
    /// PARALLEL_MIN_FILE_SIZE`（16 MB）时才被走到，而那条跨入口守卫用的是 0 字节夹具，
    /// 所以这两个成员**只有这里能钉**。
    ///
    /// **为什么必须钉**：回退成 `Result<_, String>` 现在编译不过（**实测**：本测试覆盖的两个成员
    /// **一起**回退报 3 × `E0277` + 3 × `E0308`；只回退 `parse_chunk_lines` 报 2 × `E0277`
    /// + 2 × `E0308`，只回退 `parse_session_parallel_with` 报 1 + 1）；**但连函数体一起回退**
    /// 编译器看不见，16 MB 以上的会话加载失败时用户/日志拿到的就是裸 OS 英文。
    ///
    /// 只断言错误形状，不解析真实数据：不存在的文件必然在 `File::open` 失败。
    #[test]
    fn parallel_kernel_reports_io_failures_as_coded() {
        let missing = "/definitely/not/here-seshbuddy.jsonl";

        let err = parse_chunk_lines(missing, 0, 1, false).expect_err("不存在的文件必须失败");
        assert_eq!(coded_code(err.clone()), "internal.io");
        assert!(
            matches!(&err, AppError::Coded { params, .. }
                if params.get("detail").is_some_and(|d| !d.is_empty())),
            "必须带非空 params.detail"
        );

        // 并行内核：`file_size` 给到阈值以上才会真的切块并 spawn 线程。
        let err = parse_session_parallel_with(missing, 64 * 1024 * 1024, false, &mut |_| true)
            .expect_err("不存在的文件必须失败");
        assert_eq!(coded_code(err), "internal.io");
    }

    #[test]
    fn claude_search_text_excludes_tool_use_only_entries() {
        // 优化：tool_use 条目会产生巨型 N-gram token，已从搜索索引中剔除。
        // 纯 tool_use（无 text 项）的 assistant 消息不再被索引。
        let entry = json!({
            "type": "assistant",
            "message": {
                "role": "assistant",
                "content": [
                    {
                        "type": "tool_use",
                        "name": "Write",
                        "input": {
                            "file_path": "plan.md",
                            "content": "Step 1: 扩展 OpenTab 接口，增加滚动状态字段"
                        }
                    }
                ]
            }
        });
        // 纯 tool_use 无 text 项 → None（不进入索引，减少 N-gram 爆炸）
        assert!(extract_claude_role_text(&entry).is_none());

        // 但 assistant 回答中同时含 text 的情况，text 部分仍会被索引
        let entry_with_text = json!({
            "type": "assistant",
            "message": {
                "role": "assistant",
                "content": [
                    {"type": "text", "text": "好的，我来帮你扩展接口"},
                    {"type": "tool_use", "name": "Write", "input": {"content": "噪声数据"}}
                ]
            }
        });
        let (_role, text) = extract_claude_role_text(&entry_with_text).expect("text item should be indexed");
        assert!(text.contains("好的，我来帮你扩展接口"));
        assert!(!text.contains("噪声数据"));
    }

    #[test]
    fn claude_tool_result_entry_excluded_from_search_index() {
        // 优化：tool_result 输出（命令行日志、文件内容等）可达数 MB，
        // 经 Ngram(2..4) 分词后会产生数百万 token，严重拖慢全量索引构建速度。
        // 现在只索引 type=="text" 的用户/助手正文，tool_result 已从索引中剔除。
        let entry = json!({
            "type": "user",
            "message": {
                "role": "user",
                "content": [
                    {"type": "tool_result", "content": "error: linker command failed"}
                ]
            }
        });
        // 纯 tool_result 消息 → None，不占用索引空间
        assert!(extract_claude_role_text(&entry).is_none());

        // 混合消息（tool_result + text）→ 只有 text 部分被索引
        let mixed = json!({
            "type": "user",
            "message": {
                "role": "user",
                "content": [
                    {"type": "tool_result", "content": "error: linker command failed"},
                    {"type": "text", "text": "请帮我修复上面的链接错误"}
                ]
            }
        });
        let (_role, text) = extract_claude_role_text(&mixed).expect("text item should be indexed");
        assert!(text.contains("请帮我修复上面的链接错误"));
        assert!(!text.contains("linker command failed"));
    }

    #[test]
    fn claude_user_prompt_text_skips_tool_result_keeps_text_items() {
        // 混合条目：[tool_result, text] —— 标题路径按项过滤，真实用户文本不丢失
        let entry = json!({
            "type": "user",
            "message": {
                "role": "user",
                "content": [
                    {"type": "tool_result", "content": "tool output noise"},
                    {"type": "text", "text": "真正的问题"}
                ]
            }
        });

        let text = extract_claude_user_prompt_text(&entry).expect("mixed entry should yield text item");
        assert_eq!(text, "真正的问题");

        // 纯 tool_result 条目对标题不可见
        let tool_only = json!({
            "type": "user",
            "message": {
                "role": "user",
                "content": [{"type": "tool_result", "content": "noise"}]
            }
        });
        assert_eq!(extract_claude_user_prompt_text(&tool_only), None);
    }

    #[test]
    fn scan_claude_metadata_finds_late_title_via_tail_window() {
        // 回归：标题行追加在文件末尾（超过 64KB 多个 chunk），尾部窗口必须命中，且只读头+尾
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("seshbuddy-tail-title-{unique}.jsonl"));

        let mut content = String::from(
            "{\"type\":\"user\",\"timestamp\":\"2026-07-27T10:00:00Z\",\"cwd\":\"/p\",\"message\":{\"role\":\"user\",\"content\":\"帮我看下这个问题\"}}\n",
        );
        // 填充超过 64KB 的中间内容，确保标题行落在尾部第二个 chunk 之前
        let filler_line = format!(
            "{{\"type\":\"assistant\",\"message\":{{\"role\":\"assistant\",\"content\":\"{}\"}}}}\n",
            "x".repeat(1024)
        );
        while content.len() < 200 * 1024 {
            content.push_str(&filler_line);
        }
        content.push_str("{\"type\":\"ai-title\",\"aiTitle\":\"尾部 AI 标题\"}\n");
        content.push_str("{\"type\":\"custom-title\",\"customTitle\":\"尾部自定义标题\"}\n");
        fs::write(&path, &content).unwrap();

        let metadata = scan_claude_metadata_only(&path).expect("should scan metadata");
        let _ = fs::remove_file(&path);

        // last-wins：文件末尾的 custom-title 覆盖 ai-title
        assert_eq!(metadata.title.as_deref(), Some("尾部自定义标题"));
        assert_eq!(metadata.first_user_message.as_deref(), Some("帮我看下这个问题"));
    }

    #[test]
    fn scan_claude_metadata_reads_only_head_and_tail_of_large_file() {
        // 性能回归：头部窗口外的畸形行（非 JSON、超大行）不得导致扫描失败或全量解析
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("seshbuddy-head-scan-{unique}.jsonl"));

        let mut content = String::from(
            "{\"type\":\"user\",\"timestamp\":\"2026-07-27T10:00:00Z\",\"message\":{\"role\":\"user\",\"content\":\"first\"}}\n",
        );
        while content.len() < 100 * 1024 {
            content.push_str(&format!("not json at all {}\n", "y".repeat(512)));
        }
        fs::write(&path, &content).unwrap();

        let metadata = scan_claude_metadata_only(&path).expect("malformed middle lines must not fail scan");
        let _ = fs::remove_file(&path);

        assert_eq!(metadata.title, None);
        assert_eq!(metadata.first_user_message.as_deref(), Some("first"));
    }

    #[test]
    fn chat_line_candidate_finds_type_in_prefix() {
        let line = r#"{"parentUuid":"x","isSidechain":false,"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"hi"}]}}"#;
        assert!(chat_line_candidate(line));
    }

    /// 逐行 `Value` 实现，作为类型化路径的等价性黄金对照。
    fn extract_usage_records_legacy(file_path: &str, project: &str) -> Vec<UsageRecord> {        use std::io::{BufRead, BufReader};
        let file = match std::fs::File::open(file_path) {
            Ok(f) => f,
            Err(_) => return Vec::new(),
        };
        let reader = BufReader::new(file);
        let mut records: Vec<UsageRecord> = Vec::new();
        for line in reader.lines() {
            let line = match line {
                Ok(l) => l,
                Err(_) => continue,
            };
            let line = line.trim().to_string();
            if line.is_empty() {
                continue;
            }
            let entry: serde_json::Value = match serde_json::from_str(&line) {
                Ok(v) => v,
                Err(_) => continue,
            };
            let msg_type = entry.get("type").and_then(|v| v.as_str()).unwrap_or("");
            if msg_type == "system" {
                let subtype = entry.get("subtype").and_then(|v| v.as_str()).unwrap_or("");
                if subtype == "turn_duration" {
                    if let Some(dur) = entry.get("durationMs").and_then(|v| v.as_u64()) {
                        if let Some(last) = records.last_mut() {
                            last.duration_ms = Some(dur);
                        }
                    }
                }
                continue;
            }
            if msg_type != "assistant" {
                continue;
            }
            let message = match entry.get("message") {
                Some(m) => m,
                None => continue,
            };
            let usage = match message.get("usage") {
                Some(u) => u,
                None => continue,
            };
            let input_tokens = usage.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
            let output_tokens = usage.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
            let cache_creation_tokens =
                usage.get("cache_creation_input_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
            let cache_read_tokens =
                usage.get("cache_read_input_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
            if input_tokens == 0 && output_tokens == 0 && cache_creation_tokens == 0 && cache_read_tokens == 0 {
                continue;
            }
            let model = message.get("model").and_then(|v| v.as_str()).unwrap_or("unknown").to_string();
            let timestamp = entry.get("timestamp").and_then(|v| v.as_str()).unwrap_or("");
            let date = timestamp_to_local_date(timestamp);
            records.push(UsageRecord {
                date,
                model,
                input_tokens,
                output_tokens,
                cache_creation_tokens,
                cache_read_tokens,
                duration_ms: None,
                project: project.to_string(),
            });
        }
        records
    }

    #[test]
    fn extract_usage_records_edge_cases_match_legacy() {
        // 覆盖：turn_duration 附挂、assistant 行正文恰好含 "turn_duration" 字样
        //（分流陷阱）、纯 tool_result 行、usage:null、零 usage 跳过、坏 JSON 行。
        let big_tool_result = format!(
            r#"{{"type":"user","message":{{"role":"user","content":[{{"type":"tool_result","tool_use_id":"t1","content":"{} turn_duration 噪声 {}"}}]}}}}"#,
            "z".repeat(60_000),
            "y".repeat(60_000)
        );
        let lines = [
            r#"{"type":"assistant","timestamp":"2026-09-10T16:00:00Z","message":{"role":"assistant","model":"m1","usage":{"input_tokens":10,"output_tokens":5},"content":[{"type":"text","text":"a"}]}}"#,
            &big_tool_result,
            r#"{"type":"assistant","timestamp":"2026-09-10T16:01:00Z","message":{"role":"assistant","model":"m2","usage":{"input_tokens":1,"output_tokens":2},"content":[{"type":"tool_result","content":"含 turn_duration 字样但我是 assistant 行"}]}}"#,
            r#"{"type":"system","subtype":"turn_duration","durationMs":1234}"#,
            r#"{"type":"assistant","timestamp":"2026-09-10T16:02:00Z","message":{"role":"assistant","model":"m3","usage":null,"content":[]}}"#,
            r#"{"type":"assistant","timestamp":"2026-09-10T16:03:00Z","message":{"role":"assistant","model":"m4","usage":{"input_tokens":0,"output_tokens":0},"content":[]}}"#,
            r#"{"type":"system","subtype":"other","durationMs":999}"#,
            r#"not json at all"#,
            r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"hello"}]}}"#,
        ];
        let dir = std::env::temp_dir();
        let path = dir.join(format!("seshbuddy-usage-edge-{}.jsonl", std::process::id()));
        fs::write(&path, lines.join("\n")).unwrap();

        let legacy = extract_usage_records_legacy(&path.to_string_lossy(), "proj");
        let typed = extract_usage_records(&path.to_string_lossy(), "proj");
        let _ = fs::remove_file(&path);

        assert_eq!(format!("{legacy:?}"), format!("{typed:?}"), "新旧实现输出必须逐条一致");
        assert_eq!(typed.len(), 2, "只有两条非零 usage 的 assistant 行");
        assert_eq!(typed[0].model, "m1");
        assert_eq!(typed[1].model, "m2");
        assert_eq!(typed[1].duration_ms, Some(1234), "duration 附挂到最后一条 record");
        assert_eq!(typed[0].duration_ms, None);
    }

    #[test]
    #[ignore]
    fn extract_usage_records_matches_legacy_and_faster_on_sample() {
        let Some(path) = std::env::var("SESHBUDDY_BENCH_SESSION")
            .ok()
            .filter(|p| !p.trim().is_empty())
        else {
            eprintln!("跳过：未设置 SESHBUDDY_BENCH_SESSION=<会话文件路径>");
            return;
        };

        // 等价性：逐条 Debug 对比
        let legacy = extract_usage_records_legacy(&path, "proj");
        let typed = extract_usage_records(&path, "proj");
        assert_eq!(
            format!("{legacy:?}"),
            format!("{typed:?}"),
            "usage 记录新旧实现不一致（{} vs {} 条）",
            legacy.len(),
            typed.len()
        );

        // 基准：best-of-2
        let mut legacy_ms = u128::MAX;
        let mut typed_ms = u128::MAX;
        for _ in 0..2 {
            let t = std::time::Instant::now();
            std::hint::black_box(extract_usage_records_legacy(&path, "proj"));
            legacy_ms = legacy_ms.min(t.elapsed().as_millis());
            let t = std::time::Instant::now();
            std::hint::black_box(extract_usage_records(&path, "proj"));
            typed_ms = typed_ms.min(t.elapsed().as_millis());
        }
        eprintln!(
            "usage 提取基准（{} 条记录）：legacy {} ms → typed {} ms = {:.2}x",
            typed.len(),
            legacy_ms,
            typed_ms,
            legacy_ms as f64 / typed_ms.max(1) as f64
        );
    }

    #[test]
    fn chat_line_candidate_finds_type_in_suffix_long_line() {
        let padding = "x".repeat(4_000);
        let line = format!(
            r#"{{"parentUuid":"x","isSidechain":false,"message":{{"content":[{{"type":"tool_use","name":"Agent","input":{{"prompt":"{}"}}}}],"role":"assistant"}},"type":"assistant"}}"#,
            padding
        );
        let type_pos = line.find(r#""type":"assistant""#).unwrap();
        assert!(type_pos > 2048, "type must be beyond mid-window gap zone");
        assert!(chat_line_candidate(&line));
    }

    #[test]
    fn chat_line_candidate_finds_type_in_gap_zone() {
        // Line ~2500 bytes, type at ~1200 — captured by new overlapping windows
        let padding = "x".repeat(1_200);
        let line = format!(
            r#"{{"parentUuid":"x","isSidechain":false,"message":{{"content":[{{"type":"text","text":"{}"}}],"role":"user"}},"type":"user"}}"#,
            padding
        );
        let type_pos = line.find(r#""type":"user""#).unwrap();
        assert!(type_pos > 1024, "type should be beyond old 1024 prefix window");
        // Old algorithm missed this (type in gap between prefix and suffix),
        // new overlapping windows find it.
        assert!(chat_line_candidate(&line));
    }

    #[test]
    fn chat_line_candidate_rejects_non_chat_line() {
        let line = r#"{"type":"system","message":"internal event"}"#;
        assert!(!chat_line_candidate(line));
    }

    #[test]
    fn parse_jsonl_line_marks_is_meta_and_skips_image_reference() {
        let line = r#"{"parentUuid":"x","isSidechain":false,"isMeta":true,"type":"user","message":{"role":"user","content":[{"type":"text","text":"[Image: source: /Users/x/.claude/image-cache/abc/1.png]"}]}}"#;
        let msg = parse_jsonl_line(line, false, None, None).expect("should parse");
        assert!(msg.is_meta);
        assert_eq!(msg.content_parts.len(), 1);
        assert!(matches!(msg.content_parts[0], ContentPart::ImageRef { .. } | ContentPart::ImageMeta));
    }

    #[test]
    fn parse_jsonl_line_is_meta_defaults_to_false() {
        let line = r#"{"parentUuid":"x","isSidechain":false,"type":"user","message":{"role":"user","content":[{"type":"text","text":"hello"}]}}"#;
        let msg = parse_jsonl_line(line, false, None, None).expect("should parse");
        assert!(!msg.is_meta);
    }

    // -----------------------------------------------------------------------
    // 等价性：类型化路径 vs 逐行 Value 路径
    // -----------------------------------------------------------------------

    /// 两条路径必须对同一行给出**完全相同**的结论：要么都丢弃，
    /// 要么产出逐字段一致的 `ChatMessage`（用 `Debug` 串做全字段比对）。
    fn assert_same_as_legacy(line: &str) {
        for skip_sidechain in [false, true] {
            let legacy = parse_jsonl_line_legacy(line, skip_sidechain, None, None);
            let typed = parse_jsonl_line(line, skip_sidechain, None, None);
            match (&legacy, &typed) {
                (None, None) => {}
                (Some(a), Some(b)) => assert_eq!(
                    format!("{a:?}"),
                    format!("{b:?}"),
                    "skip_sidechain={skip_sidechain} 时字段不一致\n输入：{line}"
                ),
                _ => panic!(
                    "skip_sidechain={skip_sidechain} 时「是否产出消息」不一致（legacy={} typed={}）\n输入：{line}",
                    legacy.is_some(),
                    typed.is_some()
                ),
            }
        }
    }

    fn equivalence_corpus() -> Vec<String> {
        let long_ascii = "a".repeat(60_000);
        let long_escaped = "line\\n".repeat(20_000);
        // 3 字节字符恰好跨过 50 KB 上限
        let long_multibyte = format!("{}中中中中中", "b".repeat(49_999));
        let long_astral = format!("{}😀😀", "c".repeat(49_998));
        let crlf_ish = format!("{}\r\n{}", "d".repeat(30_000), "e".repeat(30_000));

        let short_result = serde_json::to_string("hello\nworld").unwrap();
        let long_ascii_json = serde_json::to_string(&long_ascii).unwrap();
        let long_escaped_json = serde_json::to_string(&long_escaped).unwrap();
        let long_mb_json = serde_json::to_string(&long_multibyte).unwrap();
        let long_astral_json = serde_json::to_string(&long_astral).unwrap();
        let crlf_json = serde_json::to_string(&crlf_ish).unwrap();

        let tool_result_line = |content_json: &str| {
            format!(
                r#"{{"type":"user","message":{{"role":"user","content":[{{"type":"tool_result","content":{content_json}}}]}}}}"#
            )
        };

        vec![
            // —— 基础形状 ——
            r#"{"type":"user","message":{"role":"user","content":"hi"}}"#.into(),
            r#"{"type":"user","message":{"role":"user","content":""}}"#.into(),
            r#"{"type":"user","message":{"role":"user","content":"   "}}"#.into(),
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"ok"}]}}"#.into(),
            r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":""}]}}"#.into(),
            // —— 顶层 content 不是字符串也不是数组 ——
            r#"{"type":"user","message":{"role":"user","content":123}}"#.into(),
            r#"{"type":"user","message":{"role":"user","content":null}}"#.into(),
            r#"{"type":"user","message":{"role":"user","content":{}}}"#.into(),
            r#"{"type":"user","message":{"role":"user","content":[]}}"#.into(),
            // —— 被丢弃的条目 ——
            r#"{"type":"system","message":{"role":"system","content":"x"}}"#.into(),
            r#"{"type":"summary"}"#.into(),
            r#"{"type":"user"}"#.into(),
            r#"not json at all"#.into(),
            r#"{"type":"user","message":"oops"}"#.into(),
            r#"{"type":42,"message":{"content":"x"}}"#.into(),
            // —— sidechain ——
            r#"{"type":"user","isSidechain":true,"message":{"role":"user","content":"side"}}"#.into(),
            r#"{"type":"user","isSidechain":"yes","message":{"role":"user","content":"odd"}}"#.into(),
            // —— 转义与 Unicode ——
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"a\"b\\c\nd\te\u4e2d\ud83d\ude00"}]}}"#.into(),
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"\u0000\u001f"}]}}"#.into(),
            // —— tool_use 的各种退化 ——
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"t1","name":"Read","input":{"file_path":"/a/b/plan.md"}}]}}"#.into(),
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","name":"Bash"}]}}"#.into(),
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","name":"Bash","input":null}]}}"#.into(),
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","name":7,"id":8,"input":[1,2]}]}}"#.into(),
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","name":"Grep","input":{"pattern":"中"}}]}}"#.into(),
            // —— thinking ——
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"thinking","thinking":"hmm"}]}}"#.into(),
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"thinking","thinking":""}]}}"#.into(),
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"thinking","thinking":42}]}}"#.into(),
            // —— image ——
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"image","source":{"type":"base64","media_type":"image/png","data":"AA=="}}]}}"#.into(),
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"image","source":{"type":"url","url":"http://x"}}]}}"#.into(),
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"image","source":{"type":"base64","data":"AA=="}}]}}"#.into(),
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"image"}]}}"#.into(),
            // —— 未知 / 畸形元素 ——
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"future","x":1},{"type":"text","text":"kept"}]}}"#.into(),
            r#"{"type":"assistant","message":{"role":"assistant","content":["bare",42,null,{"type":"text","text":"kept"}]}}"#.into(),
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"text":"no-type"}]}}"#.into(),
            // 内容块内的重复键：手工访问器与旧 Value 路径一致（后者胜出）
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"first","text":"second"}]}}"#.into(),
            // —— tool_result ——
            r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"short"}]}}"#.into(),
            r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","is_error":true,"content":"boom"}]}}"#.into(),
            r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":""}]}}"#.into(),
            r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":null}]}}"#.into(),
            r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result"}]}}"#.into(),
            r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":[{"type":"text","text":"l1"},{"type":"text","text":"l2"}]}]}}"#.into(),
            r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":[{"type":"text"},{"type":"text","text":"only"},{"text":"bare"}]}]}}"#.into(),
            r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":[]}]}}"#.into(),
            r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":42}]}}"#.into(),
            // —— 截断边界 ——
            tool_result_line(&serde_json::to_string(&"x".repeat(49_999)).unwrap()),
            tool_result_line(&serde_json::to_string(&"x".repeat(50_000)).unwrap()),
            tool_result_line(&serde_json::to_string(&"x".repeat(50_001)).unwrap()),
            tool_result_line(&short_result),
            tool_result_line(&long_ascii_json),
            tool_result_line(&long_escaped_json),
            tool_result_line(&long_mb_json),
            tool_result_line(&long_astral_json),
            tool_result_line(&crlf_json),
            tool_result_line(r#"[{"type":"text","text":"aa"},{"type":"text","text":"bb"}]"#),
            format!(
                r#"{{"type":"user","message":{{"role":"user","content":[{{"type":"tool_result","content":[{{"type":"text","text":{}}},{{"type":"text","text":{}}}]}}]}}}}"#,
                serde_json::to_string(&"中".repeat(30_000)).unwrap(),
                serde_json::to_string(&"尾".repeat(30_000)).unwrap()
            ),
            // —— usage 的四种存在形态 ——
            r#"{"type":"assistant","message":{"role":"assistant","content":"x","usage":{"input_tokens":1,"output_tokens":2,"cache_creation_input_tokens":3,"cache_read_input_tokens":4}}}"#.into(),
            r#"{"type":"assistant","message":{"role":"assistant","content":"x","usage":null}}"#.into(),
            r#"{"type":"assistant","message":{"role":"assistant","content":"x","usage":7}}"#.into(),
            r#"{"type":"assistant","message":{"role":"assistant","content":"x","usage":{}}}"#.into(),
            r#"{"type":"assistant","message":{"role":"assistant","content":"x","usage":{"input_tokens":"nope"}}}"#.into(),
            // —— 元字段 ——
            r#"{"type":"user","isMeta":true,"uuid":"u-1","timestamp":"2026-09-11T00:00:00Z","message":{"role":"user","content":"m"}}"#.into(),
            r#"{"type":"user","isMeta":null,"uuid":null,"timestamp":null,"message":{"role":"user","content":"m"}}"#.into(),
            // 注：重复键（如两个 "uuid"）**刻意不放进语料** —— 这是两条路径已知且有意接受的
            // 唯一分歧点（旧路径后者胜出，新路径按不可解析丢弃），见 claude_entry 的对应测试
            r#"{"type":"user","message":{"role":"user","model":"claude-x","content":"m"}}"#.into(),
            r#"{"type":"user","message":{"content":"no-role"}}"#.into(),
            // —— 复刻真实 Assistant 条目 ——
            r#"{"parentUuid":"p","isSidechain":false,"type":"assistant","uuid":"u","timestamp":"2026-09-11T00:00:00Z","message":{"role":"assistant","model":"claude-opus-4","usage":{"input_tokens":10,"output_tokens":20,"cache_creation_input_tokens":0,"cache_read_input_tokens":5},"content":[{"type":"thinking","thinking":"hmm"},{"type":"text","text":"ok"},{"type":"tool_use","id":"t","name":"Bash","input":{"command":"ls -la"}}]}}"#.into(),
        ]
    }

    #[test]
    fn typed_path_is_equivalent_to_value_path_on_corpus() {
        let corpus = equivalence_corpus();
        assert!(corpus.len() > 40, "语料太少会失去守护意义");
        for line in &corpus {
            assert_same_as_legacy(line);
        }
    }

    /// 子 Agent 注册分支：样本核对时 `subagent_map` 为 `None`，这条分支不会被覆盖，
    /// 因此用临时目录单独比对一次（含"空内容条目仍然登记 subagent"的顺序语义）。
    #[test]
    fn subagent_registration_matches_legacy() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("seshbuddy-subagent-{unique}"));
        let stem = "sess-1";
        let subagents = dir.join(stem).join("subagents");
        fs::create_dir_all(&subagents).unwrap();
        fs::write(subagents.join("agent-abcdef123456.jsonl"), "{}\n").unwrap();
        let session_file = dir.join(format!("{stem}.jsonl"));
        fs::write(&session_file, "").unwrap();
        let path = session_file.as_path();

        // 有内容：两边都应产出消息并登记 subagent
        let line = r#"{"type":"user","toolUseResult":{"agentId":"abcdef123456"},"message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"x"}]}}"#;
        let mut legacy_map = HashMap::new();
        let mut typed_map = HashMap::new();
        let legacy =
            parse_jsonl_line_legacy(line, false, Some(&mut legacy_map), Some(path)).expect("legacy");
        let typed =
            parse_jsonl_line(line, false, Some(&mut typed_map), Some(path)).expect("typed");
        assert_eq!(format!("{legacy:?}"), format!("{typed:?}"));
        assert_eq!(legacy_map.len(), 1);
        assert_eq!(typed_map.len(), 1);
        assert_eq!(typed_map["t1"].label, "Subagent abcdef");
        assert_eq!(legacy_map["t1"].label, typed_map["t1"].label);
        assert_eq!(legacy_map["t1"].file_path, typed_map["t1"].file_path);

        // 内容为空（tool_result 正文为空 → 整条消息被丢弃）时**仍要**登记 subagent：
        // 改造前 subagent 注册在「内容为空即 return None」之前，顺序不能调换
        let empty_content = r#"{"type":"user","toolUseResult":{"agentId":"abcdef123456"},"message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t2","content":""}]}}"#;
        let mut legacy_empty = HashMap::new();
        let mut typed_empty = HashMap::new();
        assert!(parse_jsonl_line_legacy(empty_content, false, Some(&mut legacy_empty), Some(path)).is_none());
        assert!(parse_jsonl_line(empty_content, false, Some(&mut typed_empty), Some(path)).is_none());
        assert_eq!(
            legacy_empty.keys().collect::<Vec<_>>(),
            typed_empty.keys().collect::<Vec<_>>(),
            "空内容条目的 subagent 登记行为必须一致"
        );
        assert!(typed_empty.contains_key("t2"));

        let _ = fs::remove_dir_all(&dir);
    }

    /// 非 user 角色 / 无 toolUseResult 时不应登记 subagent
    #[test]
    fn subagent_registration_requires_user_role_and_agent_id() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("seshbuddy-subagent-neg-{unique}"));
        let stem = "sess-2";
        let subagents = dir.join(stem).join("subagents");
        fs::create_dir_all(&subagents).unwrap();
        fs::write(subagents.join("agent-abcdef123456.jsonl"), "{}\n").unwrap();
        let session_file = dir.join(format!("{stem}.jsonl"));
        fs::write(&session_file, "").unwrap();
        let path = session_file.as_path();

        let lines = [
            // 非 user 角色
            r#"{"type":"assistant","toolUseResult":{"agentId":"abcdef123456"},"message":{"role":"assistant","content":[{"type":"tool_result","tool_use_id":"t1","content":"x"}]}}"#,
            // 无 toolUseResult
            r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"x"}]}}"#,
            // content 不是数组
            r#"{"type":"user","toolUseResult":{"agentId":"abcdef123456"},"message":{"role":"user","content":"plain"}}"#,
            // 首元素没有 tool_use_id
            r#"{"type":"user","toolUseResult":{"agentId":"abcdef123456"},"message":{"role":"user","content":[{"type":"tool_result","content":"x"}]}}"#,
        ];

        for line in lines {
            let mut legacy_map = HashMap::new();
            let mut typed_map = HashMap::new();
            let legacy = parse_jsonl_line_legacy(line, false, Some(&mut legacy_map), Some(path));
            let typed = parse_jsonl_line(line, false, Some(&mut typed_map), Some(path));
            assert_eq!(legacy.is_some(), typed.is_some(), "是否产出消息不一致：{line}");
            assert_eq!(
                legacy_map.keys().collect::<Vec<_>>(),
                typed_map.keys().collect::<Vec<_>>(),
                "subagent 登记集合不一致：{line}"
            );
            assert!(typed_map.is_empty(), "不该登记 subagent：{line}");
        }

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn subagent_registration_resolves_from_sibling_directory_in_fork() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("seshbuddy-subagent-fork-{unique}"));
        // 原始父会话目录及子代理文件
        let orig_stem = "sess-orig";
        let subagents = dir.join(orig_stem).join("subagents");
        fs::create_dir_all(&subagents).unwrap();
        let subagent_file = subagents.join("agent-forksub123.jsonl");
        fs::write(&subagent_file, "{}\n").unwrap();

        // Fork 出来的分叉会话文件
        let fork_file = dir.join("sess-fork.jsonl");
        fs::write(&fork_file, "").unwrap();

        let line = r#"{"type":"user","toolUseResult":{"agentId":"forksub123"},"message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t_fork","content":"done"}]}}"#;
        let mut typed_map = HashMap::new();
        let res = parse_jsonl_line(line, false, Some(&mut typed_map), Some(fork_file.as_path()));
        assert!(res.is_some());
        assert_eq!(typed_map.len(), 1);
        let info = typed_map.get("t_fork").expect("应在兄弟会话目录下找到子代理文件");
        assert_eq!(info.file_path, subagent_file.to_string_lossy());
        assert_eq!(info.label, "Subagent forksu");

        let _ = fs::remove_dir_all(&dir);
    }

    /// 真实 1 GB 样本上的全量等价性核对：
    ///
    /// ```sh
    /// SESHBUDDY_BENCH_SESSION=/path/to/big.jsonl \
    ///   cargo test --release --lib parse_jsonl_line_matches_legacy_on_sample -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore = "手动运行：需设置 SESHBUDDY_BENCH_SESSION=<会话文件路径>"]
    fn parse_jsonl_line_matches_legacy_on_sample() {
        let Some(path) = std::env::var("SESHBUDDY_BENCH_SESSION")
            .ok()
            .filter(|p| !p.trim().is_empty())
        else {
            eprintln!("跳过：未设置 SESHBUDDY_BENCH_SESSION=<会话文件路径>");
            return;
        };
        let file = std::fs::File::open(&path).expect("open sample");
        let mut reader = BufReader::with_capacity(256 * 1024, file);
        let mut buf = String::new();
        let mut line_no = 0usize;
        let mut candidates = 0usize;
        let mut matched = 0usize;
        let mut skipped_by_both = 0usize;

        loop {
            buf.clear();
            match reader.read_line(&mut buf) {
                Ok(0) => break,
                Err(_) => continue,
                Ok(_) => {}
            }
            line_no += 1;
            let line = buf.trim();
            if line.is_empty() || !chat_line_candidate(line) {
                continue;
            }
            candidates += 1;
            for skip_sidechain in [false, true] {
                let legacy = parse_jsonl_line_legacy(line, skip_sidechain, None, None);
                let typed = parse_jsonl_line(line, skip_sidechain, None, None);
                match (&legacy, &typed) {
                    (None, None) => skipped_by_both += 1,
                    (Some(a), Some(b)) => {
                        assert_eq!(
                            format!("{a:?}"),
                            format!("{b:?}"),
                            "第 {line_no} 行（skip_sidechain={skip_sidechain}）字段不一致"
                        );
                        matched += 1;
                    }
                    _ => panic!(
                        "第 {line_no} 行（skip_sidechain={skip_sidechain}）是否产出消息不一致（legacy={} typed={}）",
                        legacy.is_some(),
                        typed.is_some()
                    ),
                }
            }
        }

        assert!(candidates > 1000, "样本行数过少，等价性核对失去意义");
        eprintln!(
            "等价性核对通过：{candidates} 行候选 / {matched} 次两边都产出且逐字段一致 / {skipped_by_both} 次两边都丢弃"
        );
    }

    /// 对拍：搜索索引的 `message_index` 与消息数组下标相差多少。
    ///
    /// 两侧口径不同，必然不相等：
    /// - parse 侧产出条件：`parse_jsonl_line` 的 `content_parts` 非空（含 tool_result /
    ///   tool_use / thinking 等所有 part 类型），下标即消息数组位置
    /// - search 侧命中条件：`extract_claude_role_text` → `extract_claude_text` 只收集
    ///   `type == "text"` 的内容，`message_index` 只是「搜索命中序号」
    /// 纯工具消息（仅 tool_result / thinking，无 text）parse 占下标、search 不占，于是
    /// search 的 message_index 系统性落后 —— 它**不能**当消息数组下标使用。
    ///
    /// 因此前端不消费该值：`useChatSearch.ts` 的 `openSearchAt` 只接收 query，定位改用
    /// 会话内搜索的首个匹配（两边内容口径一致：都只搜 text、都排除工具输出）。
    /// 本用例是该结论的量化证据，需要真实样本，故不随 CI 跑。
    #[test]
    #[ignore]
    fn search_index_message_index_agrees_with_parse_on_sample() {
        let Some(path) = std::env::var("SESHBUDDY_BENCH_SESSION")
            .ok()
            .filter(|p| !p.trim().is_empty())
        else {
            eprintln!("跳过：未设置 SESHBUDDY_BENCH_SESSION=<会话文件路径>");
            return;
        };
        let file = std::fs::File::open(&path).expect("open sample");
        let mut reader = BufReader::with_capacity(256 * 1024, file);
        let mut buf = String::new();
        let mut line_no = 0usize;
        let mut parse_index = 0usize; // 消息数组下标（parse 侧）
        let mut search_index = 0usize; // extractor 命中序号（search 侧）
        let mut both = 0usize; // 两边都产出且下标一致
        let mut mismatch = 0usize; // 两边都产出但下标不一致（跳转必偏）
        let mut parse_only = 0usize; // parse 产出、search 未命中（search 落后的来源）
        let mut search_only = 0usize; // search 命中、parse 丢弃（下标越界级错误）
        let mut max_drift = 0isize; // 最大落后量（parse_index - search_index 的峰值）

        loop {
            buf.clear();
            match reader.read_line(&mut buf) {
                Ok(0) => break,
                Err(_) => continue,
                Ok(_) => {}
            }
            line_no += 1;
            let line = buf.trim();
            if line.is_empty() {
                continue;
            }
            // 两侧跳过条件对齐：都跳过 sidechain（extractor 无条件跳过，parse 传 true）。
            let parsed = parse_jsonl_line(line, true, None, None);
            let entry: serde_json::Value = match serde_json::from_str(line) {
                Ok(v) => v,
                Err(_) => continue,
            };
            let searched = extract_claude_role_text(&entry).is_some();

            match (&parsed, searched) {
                (Some(_), true) => {
                    if search_index == parse_index {
                        both += 1;
                    } else {
                        mismatch += 1;
                        max_drift = max_drift.max((parse_index as isize) - (search_index as isize));
                    }
                    parse_index += 1;
                    search_index += 1;
                }
                (Some(_), false) => {
                    parse_only += 1;
                    parse_index += 1;
                }
                (None, true) => {
                    search_only += 1;
                    search_index += 1;
                }
                (None, false) => {}
            }
        }

        eprintln!(
            "口径对拍（{line_no} 行，消息 {parse_index} 条 / 搜索命中 {search_index} 条）：\n  \
             下标一致: {both}\n  \
             下标不一致（跳转会偏）: {mismatch}\n  \
             仅 parse 产出（search 落后来源）: {parse_only}\n  \
             仅 search 命中（下标越界级）: {search_only}\n  \
             最大落后（parse - search）: {max_drift}"
        );
        // 按定义必然失败：两侧口径本就不同，差值不可能为 0。保留断言是让它作为
        // 「该值不可当下标用」的可执行证据；前端已不消费该值，故不构成缺陷。
        assert_eq!(
            (mismatch, parse_only, search_only),
            (0, 0, 0),
            "搜索索引 message_index 与消息数组下标口径不同（预期内），不可用作数组下标"
        );
    }

    // ─── 并行分块解析 ────────────────────────────────────────

    /// 单线程参考实现（与改造前 parse_session_file_with_offset 的循环逐语句一致），
    /// 作为并行版的等价性黄金对照。
    fn parse_session_single_thread_reference(
        file_path: &str,
        skip_sidechain: bool,
    ) -> (Vec<ChatMessage>, HashMap<String, SubagentInfo>) {
        use std::io::{BufRead, BufReader};
        let file = std::fs::File::open(file_path).unwrap();
        let mut reader = BufReader::with_capacity(256 * 1024, file);
        let mut messages = Vec::new();
        let mut subagent_map = HashMap::new();
        let session_path = std::path::Path::new(file_path);
        let mut buf = String::new();
        let mut pos = 0u64;
        loop {
            buf.clear();
            let line_offset = pos;
            match reader.read_line(&mut buf) {
                Ok(0) => break,
                Err(_) => continue,
                Ok(n) => pos += n as u64,
            }
            let line = buf.trim();
            if line.is_empty() || !chat_line_candidate(line) {
                continue;
            }
            if let Some(mut msg) =
                parse_jsonl_line(line, skip_sidechain, Some(&mut subagent_map), Some(session_path))
            {
                backfill_source_offset(&mut msg, line_offset);
                messages.push(msg);
            }
        }
        (messages, subagent_map)
    }

    /// 用指定 chunk_size 手动切块（串行调用 parse_chunk_lines，与线程层解耦），
    /// 按块序归并 —— 验证 Hadoop split 规则下「每行恰好归属一个块」。
    fn parse_session_chunked_reference(
        file_path: &str,
        file_size: u64,
        chunk_size: u64,
        skip_sidechain: bool,
    ) -> (Vec<ChatMessage>, HashMap<String, SubagentInfo>) {
        let mut messages = Vec::new();
        let mut subagent_map = HashMap::new();
        let mut start = 0u64;
        while start < file_size {
            let end = (start + chunk_size).min(file_size);
            let (msgs, map) =
                parse_chunk_lines(file_path, start, end, skip_sidechain).expect("chunk parse");
            messages.extend(msgs);
            subagent_map.extend(map);
            start = end;
        }
        (messages, subagent_map)
    }

    fn assert_messages_equal(a: &[ChatMessage], b: &[ChatMessage], ctx: &str) {
        assert_eq!(a.len(), b.len(), "{ctx}：消息条数不一致");
        for (i, (x, y)) in a.iter().zip(b.iter()).enumerate() {
            assert_eq!(format!("{x:?}"), format!("{y:?}"), "{ctx}：第 {i} 条消息不一致");
        }
    }

    fn assert_subagent_maps_equal(
        a: &HashMap<String, SubagentInfo>,
        b: &HashMap<String, SubagentInfo>,
        ctx: &str,
    ) {
        assert_eq!(a.len(), b.len(), "{ctx}：subagent_map 大小不一致");
        for (k, v) in a {
            let w = b.get(k).unwrap_or_else(|| panic!("{ctx}：subagent_map 缺 key {k}"));
            assert_eq!(v.file_path, w.file_path, "{ctx}：subagent {k} file_path 不一致");
            assert_eq!(v.label, w.label, "{ctx}：subagent {k} label 不一致");
        }
    }

    #[test]
    fn parallel_chunked_parse_matches_single_thread_on_synthetic_session() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("seshbuddy-parallel-{unique}"));
        let stem = "sess";
        let subagents = dir.join(stem).join("subagents");
        fs::create_dir_all(&subagents).unwrap();
        fs::write(subagents.join("agent-abcdef123456.jsonl"), "{}\n").unwrap();
        let session_file = dir.join(format!("{stem}.jsonl"));

        // 合成样本要点：
        // - 超长行（~200KB tool_result，必然跨多块）
        // - 多字节 UTF-8（中文/emoji）分布在行内与预期块边界附近
        // - 文件尾无换行的最后一行
        // - 空行、坏 JSON 行、sidechain 行、subagent 注册行（user + toolUseResult）
        let big_text = format!("{}中文字段🔥{}", "x".repeat(90_000), "y".repeat(90_000));
        let lines: Vec<String> = vec![
            r#"{"type":"user","uuid":"u1","timestamp":"2026-09-10T16:00:00Z","message":{"role":"user","content":[{"type":"text","text":"第一条"}]}}"#.to_string(),
            r#"{"type":"assistant","uuid":"a1","timestamp":"2026-09-10T16:00:01Z","message":{"role":"assistant","model":"m1","content":[{"type":"text","text":"回答一"}]}}"#.to_string(),
            String::new(), // 空行
            "not json at all".to_string(), // 坏行
            format!(r#"{{"type":"user","uuid":"u2","timestamp":"2026-09-10T16:00:02Z","message":{{"role":"user","content":[{{"type":"tool_result","tool_use_id":"tr1","content":"{big_text}"}}]}}}}"#),
            r#"{"type":"assistant","isSidechain":true,"uuid":"sc1","timestamp":"2026-09-10T16:00:03Z","message":{"role":"assistant","content":[{"type":"text","text":"sidechain 应被跳过"}]}}"#.to_string(),
            r#"{"type":"user","uuid":"u3","timestamp":"2026-09-10T16:00:04Z","toolUseResult":{"agentId":"abcdef123456"},"message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"子代理结果"}]}}"#.to_string(),
            r#"{"type":"assistant","uuid":"a2","timestamp":"2026-09-10T16:00:05Z","message":{"role":"assistant","model":"m2","content":[{"type":"text","text":"末尾无换行🔚"}]}}"#.to_string(),
        ];
        // join 只加分隔符、末尾不带 \n —— 恰好构造「最后一行无换行」的边界
        let content = lines.join("\n");
        fs::write(&session_file, &content).unwrap();
        let file_size = content.len() as u64;
        let path_str = session_file.to_string_lossy().to_string();

        for skip_sidechain in [true, false] {
            let (single_msgs, single_map) =
                parse_session_single_thread_reference(&path_str, skip_sidechain);

            // 多组切块边界：小块（密集跨边界）、中等块、大于文件的块、以及 1 字节块
            for chunk_size in [1u64, 977, 4097, 65_537, file_size.max(1)] {
                let (chunked_msgs, chunked_map) = parse_session_chunked_reference(
                    &path_str,
                    file_size,
                    chunk_size.max(1),
                    skip_sidechain,
                );
                let ctx = format!("skip={skip_sidechain} chunk={chunk_size}");
                assert_messages_equal(&single_msgs, &chunked_msgs, &ctx);
                assert_subagent_maps_equal(&single_map, &chunked_map, &ctx);
            }
        }

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn parse_chunk_lines_tolerates_mid_char_boundaries() {
        // 回归：非首块 skip 曾因 `read_line(&mut String)` 的 UTF-8 校验在
        // 「边界落在多字节字符中间」时返回 InvalidData → 整块静默丢失。
        // 对「中」（3 字节）的全部 3 个相位 + 附近偏移直调生产路径 parse_chunk_lines，
        // 断言 [0,off) + [off,end) 拼接后与单线程全量逐字段一致。
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("seshbuddy-midchar-{unique}"));
        fs::create_dir_all(&dir).unwrap();
        let session_file = dir.join("sess.jsonl");

        let filler = "中".repeat(2_000); // 6000 字节连续多字节区域
        let mut content = String::new();
        for i in 0..20 {
            content.push_str(&format!(
                "{{\"type\":\"user\",\"uuid\":\"u{i}\",\"timestamp\":\"2026-09-10T16:00:{i:02}Z\",\"message\":{{\"role\":\"user\",\"content\":[{{\"type\":\"text\",\"text\":\"问 {i} {filler}\"}}]}}}}\n"
            ));
        }
        fs::write(&session_file, &content).unwrap();
        let path_str = session_file.to_string_lossy().to_string();
        let file_size = content.len() as u64;

        let (single_msgs, single_map) = parse_session_single_thread_reference(&path_str, false);
        assert_eq!(single_msgs.len(), 20);

        // 第一行正文起始处附近连续扫 12 个偏移：必然覆盖 3 字节字符的全部相位
        let filler_start = content.find(&filler).unwrap() as u64;
        let scan = (filler_start + 100)..(filler_start + 112);
        assert!(
            scan.clone().any(|off| !content.is_char_boundary(off as usize)),
            "扫描区间必须包含非字符边界（否则测不到 skip 的 UTF-8 陷阱）"
        );
        for off in scan {
            let (mut a, map_a) = parse_chunk_lines(&path_str, 0, off, false).unwrap();
            let (b, map_b) = parse_chunk_lines(&path_str, off, file_size, false).unwrap();
            a.extend(b);
            let mut merged = map_a;
            merged.extend(map_b);
            assert_messages_equal(&single_msgs, &a, &format!("off={off}"));
            assert_subagent_maps_equal(&single_map, &merged, &format!("off={off}"));
        }

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn parallel_threaded_parse_matches_single_thread_on_synthetic_session() {
        // 线程层验证：文件 >= PARALLEL_MIN_FILE_SIZE 走真并行路径
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("seshbuddy-parallel-mt-{unique}"));
        let stem = "sess";
        fs::create_dir_all(&dir).unwrap();
        let session_file = dir.join(format!("{stem}.jsonl"));

        // 构造 > 16MB 的样本：重复若干轮正常消息 + 大块 tool_result
        let mut content = String::new();
        let filler = "中".repeat(30_000);
        let mut i = 0usize;
        while content.len() < (PARALLEL_MIN_FILE_SIZE as usize) + 1024 * 1024 {
            content.push_str(&format!(
                "{{\"type\":\"user\",\"uuid\":\"u{i}\",\"timestamp\":\"2026-09-10T16:00:{:02}Z\",\"message\":{{\"role\":\"user\",\"content\":[{{\"type\":\"text\",\"text\":\"问 {i} {filler}\"}}]}}}}\n",
                i % 60
            ));
            content.push_str(&format!(
                "{{\"type\":\"assistant\",\"uuid\":\"a{i}\",\"timestamp\":\"2026-09-10T16:01:{:02}Z\",\"message\":{{\"role\":\"assistant\",\"model\":\"m{}\",\"content\":[{{\"type\":\"tool_result\",\"tool_use_id\":\"t{i}\",\"content\":\"{filler}{filler}\"}}]}}}}\n",
                i % 60, i % 7
            ));
            i += 1;
        }
        // 末尾一行不带换行
        content.push_str(r#"{"type":"assistant","uuid":"last","timestamp":"2026-09-10T16:02:00Z","message":{"role":"assistant","content":[{"type":"text","text":"tail"}]}}"#);
        fs::write(&session_file, &content).unwrap();
        let path_str = session_file.to_string_lossy().to_string();

        let (single_msgs, single_map) = parse_session_single_thread_reference(&path_str, false);
        let file_size = std::fs::metadata(&path_str).unwrap().len();
        assert!(
            file_size >= PARALLEL_MIN_FILE_SIZE,
            "样本不足 16MB，并行路径未生效"
        );
        let result = parse_session_file_with_offset(&path_str, false).expect("parallel parse");
        assert!(result.messages.len() > 10, "样本消息数过少，比对失去意义");
        assert_messages_equal(&single_msgs, &result.messages, "threaded");
        assert_subagent_maps_equal(&single_map, &result.subagent_map, "threaded");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    #[ignore = "手动运行：需设置 SESHBUDDY_BENCH_SESSION=<会话文件路径>"]
    fn parallel_parse_matches_single_thread_and_faster_on_sample() {
        let Some(path) = std::env::var("SESHBUDDY_BENCH_SESSION")
            .ok()
            .filter(|p| !p.trim().is_empty())
        else {
            eprintln!("跳过：未设置 SESHBUDDY_BENCH_SESSION=<会话文件路径>");
            return;
        };
        let file_size = std::fs::metadata(&path).expect("stat sample").len();
        assert!(
            file_size >= PARALLEL_MIN_FILE_SIZE,
            "样本不足 16MB，并行路径不会生效"
        );

        // 等价性：消息逐字段 + subagent_map
        let (single_msgs, single_map) = parse_session_single_thread_reference(&path, false);
        let result = parse_session_file_with_offset(&path, false).expect("parallel parse");
        assert_messages_equal(&single_msgs, &result.messages, "sample");
        assert_subagent_maps_equal(&single_map, &result.subagent_map, "sample");

        // 基准：best-of-2
        let mut single_ms = u128::MAX;
        let mut parallel_ms = u128::MAX;
        for _ in 0..2 {
            let t = std::time::Instant::now();
            std::hint::black_box(parse_session_single_thread_reference(&path, false));
            single_ms = single_ms.min(t.elapsed().as_millis());
            let t = std::time::Instant::now();
            std::hint::black_box(parse_session_file_with_offset(&path, false).unwrap());
            parallel_ms = parallel_ms.min(t.elapsed().as_millis());
        }
        eprintln!(
            "P1 基准（{} 条消息 / {:.0} MB）：单线程 {} ms → 并行 {} ms = {:.2}x",
            result.messages.len(),
            file_size as f64 / 1024.0 / 1024.0,
            single_ms,
            parallel_ms,
            single_ms as f64 / parallel_ms.max(1) as f64
        );
    }

    #[test]
    fn tool_result_full_content_at_roundtrip_and_drift_guard() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let session_file = std::env::temp_dir().join(format!("seshbuddy-lazy-fetch-{unique}.jsonl"));

        let big_body = "中".repeat(5_000); // 15 KB，无白名单标记 → 解析时只会带预览
        let line_text = r#"{"type":"user","uuid":"u1","timestamp":"2026-09-10T16:00:00Z","message":{"role":"user","content":[{"type":"text","text":"开头"}]}}"#.to_string();
        let line_result = format!(
            r#"{{"type":"user","uuid":"u2","timestamp":"2026-09-10T16:00:01Z","message":{{"role":"user","content":[{{"type":"tool_result","tool_use_id":"tr9","content":"{big_body}"}}]}}}}"#
        );
        let content = format!("{line_text}\n{line_result}\n");
        fs::write(&session_file, &content).unwrap();
        let path_str = session_file.to_string_lossy().to_string();
        let result_offset = (line_text.len() + 1) as u64;

        // 解析路径：超长无标记 → 预览 + 定位信息
        let parsed = parse_session_file(&path_str).expect("parse");
        let part = parsed
            .iter()
            .flat_map(|m| &m.content_parts)
            .find_map(|p| match p {
                ContentPart::ToolResult {
                    content,
                    full_len,
                    truncated_preview,
                    source_offset,
                    tool_use_id,
                    ..
                } => Some((
                    content.clone(),
                    *full_len,
                    *truncated_preview,
                    *source_offset,
                    tool_use_id.clone(),
                )),
                _ => None,
            })
            .expect("tool_result part");
        assert!(part.2, "应只带预览");
        assert_eq!(part.1 as usize, big_body.len());
        assert_eq!(part.3, result_offset, "source_offset 应指向该行行首");
        assert_eq!(part.4.as_deref(), Some("tr9"));

        // 回取：offset + tool_use_id 校验通过 → 全文
        let full = tool_result_full_content_at(&path_str, result_offset, Some("tr9"))
            .expect("fetch full content");
        assert_eq!(full, big_body);

        // 反漂移 ①：tool_use_id 不匹配 → 明确错误
        assert_eq!(
            coded_code(
                tool_result_full_content_at(&path_str, result_offset, Some("trX")).unwrap_err()
            ),
            "parser.tool_result_mismatch"
        );
        // 反漂移 ②：offset 指到非 tool_result 行 → 明确错误
        assert_eq!(
            coded_code(tool_result_full_content_at(&path_str, 0, None).unwrap_err()),
            "parser.tool_result_mismatch"
        );
        // 反漂移 ③：offset 越过 EOF → 明确错误
        assert_eq!(
            coded_code(
                tool_result_full_content_at(&path_str, content.len() as u64 + 100, None)
                    .unwrap_err()
            ),
            "parser.offset_out_of_range"
        );

        let _ = fs::remove_file(&session_file);
    }

    #[test]
    fn parallel_chunk_ranges_progressive_ramp_up_preserves_coverage() {
        for file_size in [
            PARALLEL_MIN_FILE_SIZE,              // 16MB
            PARALLEL_MIN_FILE_SIZE + 1,
            30 * 1024 * 1024,                   // 30MB
            100 * 1024 * 1024,                  // 100MB
            1024 * 1024 * 1024,                 // 1GB
            3 * 1024 * 1024 * 1024,             // 3GB
        ] {
            let ranges = parallel_chunk_ranges(file_size);
            assert!(ranges.len() >= 2, "file_size={file_size}");
            // 完整覆盖 [0, file_size) 且首尾相接（每行恰好归一块的前提）
            assert_eq!(ranges[0].0, 0, "file_size={file_size}");
            assert_eq!(ranges.last().unwrap().1, file_size, "file_size={file_size}");
            for w in ranges.windows(2) {
                assert_eq!(w[0].1, w[1].0, "file_size={file_size} 块间必须相接");
            }
            for (s, e) in &ranges {
                assert!(s < e, "file_size={file_size} 不允许空块");
            }
            // 首块固定为 4MB（保证首屏数轮对话完整秒出）
            assert_eq!(ranges[0].1, PARALLEL_RAMP_UP_SIZES[0].min(file_size));
        }
    }

    #[test]
    #[ignore = "手动运行：需设置 SESHBUDDY_BENCH_SESSION=<会话文件路径>"]
    fn lazy_preview_ipc_payload_on_sample() {
        let Some(path) = std::env::var("SESHBUDDY_BENCH_SESSION")
            .ok()
            .filter(|p| !p.trim().is_empty())
        else {
            eprintln!("跳过：未设置 SESHBUDDY_BENCH_SESSION=<会话文件路径>");
            return;
        };
        let result = parse_session_file_with_offset(&path, false).expect("parse sample");

        // IPC 载荷估算：逐 part 累加将进入序列化的字符串字段长度。
        // 改造前估值 = 截断 part 的 content 换成 full_len（full_len 即改造前的 content 长度），
        // 其余字段两边一致，单次解析即可得出前后对比。
        let mut total_after: u64 = 0;
        let mut tool_result_after: u64 = 0;
        let mut tool_result_before: u64 = 0;
        let mut truncated_count: u64 = 0;
        let mut tool_result_count: u64 = 0;
        for msg in &result.messages {
            for part in &msg.content_parts {
                match part {
                    ContentPart::Text { text } => total_after += text.len() as u64,
                    ContentPart::Thinking { thinking } => total_after += thinking.len() as u64,
                    ContentPart::ToolUse { summary, input, tool_name, .. } => {
                        total_after += (summary.len() + input.len() + tool_name.len()) as u64
                    }
                    ContentPart::ToolResult {
                        summary,
                        content,
                        full_len,
                        truncated_preview,
                        ..
                    } => {
                        tool_result_count += 1;
                        total_after += (summary.len() + content.len()) as u64;
                        tool_result_after += content.len() as u64;
                        tool_result_before += if *truncated_preview {
                            truncated_count += 1;
                            *full_len
                        } else {
                            content.len() as u64
                        };
                    }
                    ContentPart::Image { data, .. } => total_after += data.len() as u64,
                    _ => {}
                }
            }
        }
        let total_before = total_after - tool_result_after + tool_result_before;
        let mb = |v: u64| v as f64 / 1024.0 / 1024.0;
        eprintln!(
            "IPC 载荷实测（{} 条消息 / {tool_result_count} 个 tool_result，其中 {truncated_count} 个截断）：\n  tool_result 正文：{:.1} MB → {:.1} MB\n  整体估算：      {:.1} MB → {:.1} MB（-{:.0}%）",
            result.messages.len(),
            mb(tool_result_before),
            mb(tool_result_after),
            mb(total_before),
            mb(total_after),
            100.0 * (1.0 - total_after as f64 / total_before.max(1) as f64),
        );
    }
}
