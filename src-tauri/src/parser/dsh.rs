//! DSH 会话解析器。
//!
//! 格式要点（~/.dsh/sessions/.../<session-id>.jsonl[.zstd]）：
//! - 会话头行 `{"type":"session",...,"cwd":...,"parentSession":...,"origin":...}` 携带
//!   cwd / parentSession / is_subagent 信息；
//! - 正文为 JSONL 消息事件，映射规则：
//!   - `user/message` → 用户消息；`source.kind` 为 `"subagent-report"`/`"subagent-settled"`
//!     的行是子代理回报：剥掉 "Background subagent …" 包装文案后渲染为
//!     `[subagent_report]`/`[subagent_settled]` 标记文本行；
//!   - 子代理会话（header `origin=="subagent"` / `subagent/descriptor` 标记）内容完整解析，
//!     不进侧边栏列表（扫描层过滤），经父会话 subagent chip 导航打开；
//!   - 父会话 `subagent` 工具调用的 `tool/result` 文本 `"started subagent <id>"` 会建立
//!     `subagent_map`（tool_use_id → 子会话文件），供前端 chip 导航；
//!   - `assistant/message` → 助手消息，实际消息对象在 `data.message`（turn/step 包装）；
//!     `data.usage` 的 inputTokens/outputTokens/cacheReadTokens/cacheWriteTokens 映射为
//!     `TokenUsage`（`cacheWriteTokens` → `cache_creation_input_tokens`，写侧）；
//!   - `tool/call` → 助手消息（ToolUse），已被 assistant/message 暴露的调用去重；
//!   - `tool/result` → 用户消息（ToolResult），由前端并入对应工具组；
//!   - `assistant/chunk`/`text-chunks`/`reasoning-chunks`/`tool-call-chunks` → 流式增量
//!     按 `(turn,step)` 缓冲；`assistant/message` 装配时取代缓冲，`step/end` 未到时
//!     flush 成临时助手消息（中断流），`type:"usage"` 增量随 flush 挂到临时消息；
//! - 内容块（`text`/`reasoning`/`tool-call`/`tool-result`/`image`）映射为 `ContentPart`；
//! - Compaction：带 `surfaceOp:{op:"replace",...}` + `sourceEventSeqs` 的行（`user/message`
//!   摘要或 prune 重放）产生的新消息取代被引用 seq 产生的旧消息（拼接保持顺序）；
//! - 其余事件（turn/step 边界、请求元数据、`session/title` 等）为纯日志/元数据，不产生消息；
//! - 文件可为明文 JSONL 或 zstd 压缩（`.zstd`/`.zst` 后缀）。

use crate::error::AppResult;
use crate::parser::batch::BatchEmitter;
use crate::parser::shared::{SearchDocument, SearchScanProgress};
use crate::session::{self, ChatMessage, ContentPart, SessionLoadResult, TokenUsage};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

/// 判断文件是否为 zstd 压缩（`.zstd` / `.zst` 后缀，按文件名后缀匹配）。
fn is_zstd_path(path: &Path) -> bool {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    name.ends_with(".zstd") || name.ends_with(".zst")
}

/// 解析 DSH 会话日志的「格式代」文件名，返回代次（代 0 为最初的 `session.jsonl`）。
///
/// DSH 的会话日志按格式代命名：代 0 沿用最初的 `session.jsonl`，其后每一代在
/// `.jsonl` 扩展名前带一个小写数字版本段（`session.v3.jsonl`），压缩副本再叠加
/// `.zstd`/`.zst`。迁移到新一代后历史代文件**不删除**，作为只读归档留在同一会话
/// 目录里，只有当前代会被继续追加写入 —— 因此同一目录下可能并存多个代文件，代次
/// 最高者才是会话的当前内容。
///
/// 判定规则与 DSH 写端识别「已提交代」的规范保持一致：非规范名一律不认，包括大写
/// `V`、前导零（`v01`）、显式的 `v0`、以及 `session.1.v3.jsonl` 这类额外分段命名，
/// 避免把临时文件或损坏残留当代际候选而读到半成品内容。
pub(crate) fn parse_generation_log_filename(filename: &str) -> Option<u32> {
    let name = filename
        .strip_suffix(".zstd")
        .or_else(|| filename.strip_suffix(".zst"))
        .unwrap_or(filename);
    let generation = name.strip_suffix(".jsonl")?.strip_prefix("session")?;
    if generation.is_empty() {
        return Some(0);
    }
    // 规范名要求 `.v<正整数>`：小写 v、至少一位、无前导零（同时排除 v0）
    let digits = generation.strip_prefix(".v")?;
    if digits.is_empty() || digits.starts_with('0') || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse::<u32>().ok()
}

/// 列出会话目录内的日志候选文件，按「代次降序 + 同代内 zstd 优先」排序。
///
/// 排序首位即会话的当前内容文件；其余候选仅作顺位回退 —— 当前代文件被写坏或读到
/// 零字节时，宁可回退展示上一代的历史内容，也不让整条会话从列表里消失。
/// 同代内压缩副本优先与写端默认的压缩配置一致，避免代内选择与 DSH 自身读写分叉。
pub(crate) fn session_dir_candidates(session_dir: &Path) -> Vec<std::path::PathBuf> {
    let Ok(entries) = std::fs::read_dir(session_dir) else {
        return Vec::new();
    };
    let mut ranked: Vec<(u32, bool, std::path::PathBuf)> = Vec::new();
    for entry in entries.flatten() {
        if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
            continue;
        }
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some(generation) = parse_generation_log_filename(name) else {
            continue;
        };
        ranked.push((generation, is_zstd_path(&path), path));
    }
    ranked.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then(b.1.cmp(&a.1))
            .then_with(|| a.2.cmp(&b.2))
    });
    ranked.into_iter().map(|(_, _, path)| path).collect()
}

/// 读取会话文件全部 JSON 行。
/// `.zstd`/`.zst` 后缀走 zstd 解压，其余按明文 JSONL 读取；逐行保留完整 JSON 行。
pub(crate) fn read_session_lines(path: &str) -> AppResult<Vec<String>> {
    let file = File::open(path)?;
    let reader: Box<dyn BufRead> = if is_zstd_path(Path::new(path)) {
        Box::new(BufReader::new(zstd::stream::read::Decoder::new(file)?))
    } else {
        Box::new(BufReader::new(file))
    };

    let mut lines = Vec::new();
    for line in reader.lines() {
        lines.push(line?);
    }
    Ok(lines)
}

/// 流式读取会话文件头行（zstd/plain 自适应）：只解压/读取首个非空行，
/// 返回 `type=="session"` 的头部 JSON 值；不整文件解压（会话索引扫描建索引用）。
/// 目录可能同时含明文与 zstd 副本，调用方需自行按 zstd 优先选择文件。
pub(crate) fn read_header_line(path: &Path) -> Option<Value> {
    let file = File::open(path).ok()?;
    let reader: Box<dyn BufRead> = if is_zstd_path(path) {
        Box::new(BufReader::new(zstd::stream::read::Decoder::new(file).ok()?))
    } else {
        Box::new(BufReader::new(file))
    };
    for line in reader.lines() {
        let line = line.ok()?.trim().to_string();
        if line.is_empty() {
            continue;
        }
        let entry: Value = serde_json::from_str(&line).ok()?;
        return (entry.get("type").and_then(Value::as_str) == Some("session")).then_some(entry);
    }
    None
}

/// 会话头行字段提取（`type=="session"` 才放行）：cwd / parentSession / is_subagent。
/// 与 `read_header_line` 配合供会话索引扫描复用；`parse_session_header` 同源。
pub(crate) fn parse_session_header_value(
    entry: &Value,
) -> Option<(String, Option<String>, bool)> {
    if entry.get("type").and_then(|v| v.as_str()) != Some("session") {
        return None;
    }
    let cwd = entry.get("cwd").and_then(|v| v.as_str())?.to_string();
    let parent = entry
        .get("parentSession")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let is_subagent = entry.get("origin").and_then(|v| v.as_str()) == Some("subagent");
    Some((cwd, parent, is_subagent))
}

/// 解析会话头行：返回 `(cwd, parentSession, is_subagent)`。
/// 仅放行 `"type":"session"` 行；cwd 原样读取（不做小写化等改写）。
pub(crate) fn parse_session_header(line: &str) -> Option<(String, Option<String>, bool)> {
    let entry: Value = serde_json::from_str(line.trim()).ok()?;
    parse_session_header_value(&entry)
}

pub(crate) fn is_subagent_file(file_path: &str) -> bool {
    let Some(line) = crate::parser::shared::read_first_line_from_file(file_path) else {
        return false;
    };
    if let Some((_, _, is_subagent)) = parse_session_header(&line) {
        return is_subagent;
    }
    false
}

/// 一个 step 的流式 chunk 增量缓冲（其 `assistant/message` 尚未装配 / 永不装配）。
/// 按流 block index 键控，flush 时保持块相对顺序。
#[derive(Default)]
struct StepChunkBuf {
    text: BTreeMap<usize, String>,
    reasoning: BTreeMap<usize, String>,
    /// Block index → (call id, tool name, 拼接后的原始 arguments)。
    tool_calls: BTreeMap<usize, (String, Option<String>, String)>,
    /// 中断流的 usage 增量（`assistant/chunk` 的 `type:"usage"`）：flush 时挂到临时消息，
    /// 装配路径由 `assistant/message.usage` 提供（缓冲在装配时被丢弃）。
    usage: Option<TokenUsage>,
}

/// 跨行解析状态：chunk 缓冲、工具调用去重、消息输出与 compaction 拼接。
/// 行水位过滤仍由 `parse_rows_watermark` 承载（纯增量语义不进入此结构）。
#[derive(Default)]
struct ParseState {
    /// `subagent/descriptor.label`：父级为子代理选择的委托名（会话展示名/标题源）。
    #[allow(dead_code)] // 标题提取在后续任务接线（当前仅存储供测试断言）。
    descriptor_label: Option<String>,
    /// 当前会话文件路径（建 subagent_map 用；纯行解析入口为 None）。
    session_file_path: Option<std::path::PathBuf>,
    /// 父会话的 subagent 工具调用 → 子会话文件导航映射（tool_use_id → SubagentInfo）。
    subagent_map: HashMap<String, session::SubagentInfo>,
    /// `subagent` 工具调用的 call_id → arguments.description（建 map 时作 label）。
    subagent_call_descs: HashMap<String, String>,
    /// 按 `(turn, step)` 的 chunk 增量缓冲：step 的 `assistant/message` 到达时丢弃，
    /// 或 `step/end` 到来而 message 未到时 flush 成临时消息。
    chunk_bufs: HashMap<(u32, u32), StepChunkBuf>,
    /// 已装配 `assistant/message` 的 step；迟到的 chunk 行忽略，避免乱序日志重复文本。
    assembled_steps: HashSet<(u32, u32)>,
    /// 工具调用去重状态（`tool/call` 与 `assistant/message` tool-call 块共用）。
    seen_tool_use_ids: HashSet<String>,
    /// 输出消息（compaction 拼接的操作对象；末尾按行水位过滤）。
    messages: Vec<ChatMessage>,
    /// 与 `messages` 并行：产生该消息的行水位（`seq` 或行号）。
    message_watermarks: Vec<u64>,
    /// 与 `messages` 并行：产生该消息的表面事件 seq（无 `seq` 行为 None）。
    message_seqs: Vec<Option<u64>>,
    /// 表面事件 seq → 其产生的消息下标。驱动 compaction `replace` 拼接；
    /// 每次拼接后重建。
    seq_to_messages: HashMap<u64, Vec<usize>>,
    /// 当前行的水位（`push_message` 记录用；`parse_rows_watermark` 在分派前设置）。
    current_watermark: u64,
}

impl ParseState {
    /// 唯一消息追加点：记录行水位与产生 seq（`seq_to_messages`），保持
    /// `messages` / `message_watermarks` / `message_seqs` 三向量并行——compaction
    /// 拼接依赖该不变式。
    fn push_message(&mut self, message: ChatMessage, seq: Option<u64>) {
        debug_assert_eq!(self.messages.len(), self.message_watermarks.len());
        debug_assert_eq!(self.messages.len(), self.message_seqs.len());
        let index = self.messages.len();
        self.messages.push(message);
        self.message_watermarks.push(self.current_watermark);
        self.message_seqs.push(seq);
        if let Some(seq) = seq {
            self.seq_to_messages.entry(seq).or_default().push(index);
        }
    }
}

/// 解析入口：将 DSH JSONL 行序列解析为 `ChatMessage`。
pub(crate) fn parse_rows<S: AsRef<str>>(rows: Vec<S>) -> AppResult<Vec<ChatMessage>> {
    parse_rows_watermark(rows, None, None).map(|(messages, _, _)| messages)
}

/// 带行水位解析：逐行解析，返回 `(消息, 最高水位, subagent_map)`。
///
/// 行水位 = 行 `seq`（无 `seq` 时用行号）。`min_watermark` 为 Some(offset) 时只返回
/// 水位 `> offset` 的消息（增量语义），否则返回全部；返回水位恒 >= `offset`（单调不回落），
/// 保证"用返回值再调一次不产生重复"的追加幂等性。
/// `session_file_path` 提供时构建 subagent_map（父会话 subagent 调用 → 子会话文件）。
fn parse_rows_watermark<S: AsRef<str>>(
    rows: Vec<S>,
    min_watermark: Option<u64>,
    session_file_path: Option<&Path>,
) -> AppResult<(Vec<ChatMessage>, u64, HashMap<String, session::SubagentInfo>)> {
    let mut state = ParseState {
        session_file_path: session_file_path.map(|p| p.to_path_buf()),
        ..ParseState::default()
    };
    let mut max_watermark = min_watermark.unwrap_or(0);
    let mut line_no = 0u64;
    for row in rows {
        let line = row.as_ref().trim();
        if line.is_empty() {
            continue;
        }
        line_no += 1;
        let Ok(entry) = serde_json::from_str::<Value>(line) else {
            // 畸形行跳过（真实日志存在截断/损坏记录，不整体失败）。
            continue;
        };
        let watermark = entry
            .get("seq")
            .and_then(Value::as_u64)
            .unwrap_or(line_no);
        if watermark > max_watermark {
            max_watermark = watermark;
        }
        // 始终处理整行以维持工具去重状态、chunk 缓冲与 compaction 拼接；
        // 末尾仅按水位过滤输出，保证追加幂等。
        state.current_watermark = watermark;
        dispatch_row(&entry, &mut state);
    }
    let messages = match min_watermark {
        Some(offset) => {
            let mut out = Vec::new();
            for (message, wm) in state
                .messages
                .into_iter()
                .zip(state.message_watermarks.drain(..))
            {
                if wm > offset {
                    out.push(message);
                }
            }
            out
        }
        None => state.messages,
    };
    Ok((messages, max_watermark, state.subagent_map))
}

/// 单行事件分发：把该行产生的消息压入 `state.messages`。
/// 会话头/标题/纯日志事件不产生消息。行级 `surfaceOp.replace`（compaction 摘要或
/// prune 重放）在正常分发后把被引用 seq 的旧消息拼接掉。
fn dispatch_row(entry: &Value, state: &mut ParseState) {
    let event_type = entry.get("type").and_then(Value::as_str).unwrap_or("");
    // Compaction replace：被 `sourceEventSeqs` 引用的表面事件 seq 产生的消息将被
    // 本行产生的新消息取代（DSH 摘要 checkpoint 或 prune 重放均以此形态出现）。
    let replace_seqs: Option<Vec<u64>> = match entry.get("surfaceOp") {
        Some(op) if op.get("op").and_then(Value::as_str) == Some("replace") => entry
            .get("sourceEventSeqs")
            .and_then(Value::as_array)
            .map(|seqs| seqs.iter().filter_map(Value::as_u64).collect()),
        _ => None,
    };
    let produced_start = state.messages.len();
    match event_type {
        // 会话头与标题为元数据，本任务不产生消息。
        "session" | "session/title" => {}
        "user/message" => handle_user_message(entry, state),
        "assistant/message" => handle_assistant_message(entry, state),
        "tool/call" => handle_tool_call(entry, state),
        "tool/result" => handle_tool_result(entry, state),
        // 流式 chunk 行：增量拼进 (turn,step) 缓冲，本身不产生消息。
        "assistant/chunk" | "text-chunks" | "reasoning-chunks" | "tool-call-chunks" => {
            let data = entry.get("data").unwrap_or(entry);
            handle_chunk_event(event_type, data, state);
        }
        // step 结束而 assistant/message 未到时，把缓冲 flush 成临时消息。
        "step/end" => {
            let data = entry.get("data").unwrap_or(entry);
            if let (Some(turn), Some(step)) = (
                data.get("turn").and_then(Value::as_u64),
                data.get("step").and_then(Value::as_u64),
            ) {
                flush_step_chunks(state, turn as u32, step as u32, entry_seq(entry));
            }
        }
        // 子代理会话委托元数据：label 为父级选择的委托名（展示名/标题源）。
        "subagent/descriptor" => {
            let data = entry.get("data").unwrap_or(entry);
            if let Some(label) = data
                .get("label")
                .and_then(Value::as_str)
                .filter(|label| !label.trim().is_empty())
            {
                state.descriptor_label = Some(label.to_string());
            }
        }
        // 纯日志事件（turn/step 边界、请求元数据等）不产生消息。
        _ => {}
    }
    if let Some(shadowed_seqs) = replace_seqs {
        apply_surface_replace(state, produced_start, &shadowed_seqs);
    }
}

/// DSH 事件时间戳：毫秒 epoch → RFC3339 字符串；缺失时为空字符串。
fn dsh_timestamp(entry: &Value) -> String {
    entry
        .get("time")
        .and_then(Value::as_i64)
        .and_then(chrono::DateTime::from_timestamp_millis)
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_default()
}

/// 行的表面事件 seq（缺失时为 None；行水位过滤由 `parse_rows_watermark` 用行号兜底）。
fn entry_seq(entry: &Value) -> Option<u64> {
    entry.get("seq").and_then(Value::as_u64)
}

/// 映射 DSH usage 字段（camelCase）为 `TokenUsage`：
/// `inputTokens`/`outputTokens`/`cacheReadTokens`/`cacheWriteTokens`；
/// `cacheWriteTokens` 是写入侧，映射到 `cache_creation_input_tokens`。
/// 四个字段全缺时返回 None（区分"无 usage"与"全零 usage"）。
fn dsh_token_usage(usage: &Value) -> Option<TokenUsage> {
    let input = usage.get("inputTokens").and_then(Value::as_u64);
    let output = usage.get("outputTokens").and_then(Value::as_u64);
    let cache_read = usage.get("cacheReadTokens").and_then(Value::as_u64);
    let cache_write = usage.get("cacheWriteTokens").and_then(Value::as_u64);
    if input.is_none() && output.is_none() && cache_read.is_none() && cache_write.is_none() {
        return None;
    }
    Some(TokenUsage {
        input_tokens: input.unwrap_or(0),
        output_tokens: output.unwrap_or(0),
        cache_creation_input_tokens: cache_write.unwrap_or(0),
        cache_read_input_tokens: cache_read.unwrap_or(0),
    })
}

/// Compaction replace 拼接：本行产生的消息（`produced_start..`）取代每个被遮蔽
/// seq 产生的消息；无匹配时保留 replacement 于末尾。拼接后重建 seq → 消息下标映射。
fn apply_surface_replace(state: &mut ParseState, produced_start: usize, shadowed_seqs: &[u64]) {
    debug_assert_eq!(state.messages.len(), state.message_watermarks.len());
    debug_assert_eq!(state.messages.len(), state.message_seqs.len());
    let produced: Vec<ChatMessage> = state.messages.split_off(produced_start);
    let produced_watermarks: Vec<u64> = state.message_watermarks.split_off(produced_start);
    let produced_seqs: Vec<Option<u64>> = state.message_seqs.split_off(produced_start);
    let mut shadowed: Vec<usize> = shadowed_seqs
        .iter()
        .filter_map(|seq| state.seq_to_messages.get(seq))
        .flatten()
        .copied()
        .collect();
    shadowed.sort_unstable();
    shadowed.dedup();
    if shadowed.is_empty() {
        // 被遮蔽 seq 未产生过消息（如引用的纯日志行）：保持 replacement 于末尾。
        state.messages.extend(produced);
        state.message_watermarks.extend(produced_watermarks);
        state.message_seqs.extend(produced_seqs);
        return;
    }
    let insert_at = shadowed[0];
    let shadowed_set: HashSet<usize> = shadowed.iter().copied().collect();
    let mut kept = Vec::with_capacity(state.messages.len().saturating_sub(shadowed.len()));
    let mut kept_wm = Vec::with_capacity(
        state
            .message_watermarks
            .len()
            .saturating_sub(shadowed.len()),
    );
    let mut kept_seq = Vec::with_capacity(state.message_seqs.len().saturating_sub(shadowed.len()));
    for (index, ((message, wm), seq)) in state
        .messages
        .drain(..)
        .zip(state.message_watermarks.drain(..))
        .zip(state.message_seqs.drain(..))
        .enumerate()
    {
        if !shadowed_set.contains(&index) {
            kept.push(message);
            kept_wm.push(wm);
            kept_seq.push(seq);
        }
    }
    kept.splice(insert_at..insert_at, produced);
    kept_wm.splice(insert_at..insert_at, produced_watermarks);
    kept_seq.splice(insert_at..insert_at, produced_seqs);
    state.messages = kept;
    state.message_watermarks = kept_wm;
    state.message_seqs = kept_seq;
    rebuild_message_indexes(state);
}

/// 拼接后重建派生索引：`seq_to_messages` 的下标随拼接位移而失效。
fn rebuild_message_indexes(state: &mut ParseState) {
    state.seq_to_messages.clear();
    for (index, seq) in state.message_seqs.iter().enumerate() {
        if let Some(seq) = seq {
            state.seq_to_messages.entry(*seq).or_default().push(index);
        }
    }
}

/// 将 DSH 内容块数组映射为 `ContentPart` 列表（兼容裸字符串 content）。
fn parse_message_blocks(content: &Value) -> Vec<ContentPart> {
    if let Some(text) = content.as_str() {
        if text.is_empty() {
            return Vec::new();
        }
        return vec![ContentPart::Text {
            text: text.to_string(),
        }];
    }
    let Some(blocks) = content.as_array() else {
        return Vec::new();
    };
    let mut parts = Vec::new();
    for block in blocks {
        let block_type = block.get("type").and_then(Value::as_str).unwrap_or("");
        match block_type {
            "text" => {
                if let Some(text) = block.get("text").and_then(Value::as_str) {
                    if !text.is_empty() {
                        parts.push(ContentPart::Text {
                            text: text.to_string(),
                        });
                    }
                }
            }
            "reasoning" => {
                if let Some(text) = block.get("text").and_then(Value::as_str) {
                    if !text.is_empty() {
                        parts.push(ContentPart::Thinking {
                            thinking: text.to_string(),
                        });
                    }
                }
            }
            "tool-call" => {
                let name = block.get("name").and_then(Value::as_str).unwrap_or("tool");
                let tool_use_id = block.get("id").and_then(Value::as_str).map(str::to_string);
                let arguments = block.get("arguments").and_then(Value::as_str).unwrap_or("");
                parts.push(tool_use_part(name, arguments, tool_use_id));
            }
            "tool-result" => {
                let is_error = block.get("isError").and_then(Value::as_bool).unwrap_or(false);
                let content = block.get("content").map(extract_block_text).unwrap_or_default();
                if !content.is_empty() {
                    let summary = if is_error {
                        "[Tool Error]"
                    } else {
                        "[Tool Result]"
                    };
                    parts.push(ContentPart::tool_result(
                        summary.to_string(),
                        content,
                        is_error,
                        false,
                    ));
                }
            }
            "image" => {
                if let Some(source) = block.get("source") {
                    let source_type = source.get("type").and_then(Value::as_str).unwrap_or("");
                    if source_type == "base64" {
                        if let Some(data) = source.get("data").and_then(Value::as_str) {
                            let media_type = source
                                .get("media_type")
                                .and_then(Value::as_str)
                                .unwrap_or("image/png");
                            parts.push(ContentPart::Image {
                                media_type: media_type.to_string(),
                                data: data.to_string(),
                            });
                        }
                    }
                }
            }
            // 未知内容块跳过（兼容 DSH 后续新增块类型）。
            _ => {}
        }
    }
    parts
}

/// 从 DSH `tool-call` 字段构造 `ToolUse` content part。
fn tool_use_part(name: &str, arguments_raw: &str, tool_use_id: Option<String>) -> ContentPart {
    let input_val: Value = serde_json::from_str(arguments_raw)
        .unwrap_or_else(|_| Value::String(arguments_raw.to_string()));
    let input = match &input_val {
        Value::Object(_) => {
            serde_json::to_string_pretty(&input_val).unwrap_or_else(|_| arguments_raw.to_string())
        }
        _ => arguments_raw.to_string(),
    };
    let summary = session::tool_use_summary(name, &input_val);
    ContentPart::ToolUse {
        summary,
        tool_name: name.to_string(),
        input,
        tool_use_id,
    }
}

/// 提取内容块的纯文本：裸字符串原样返回；数组拼接 `text` 块并追加 `[Image]` 标记。
fn extract_block_text(content: &Value) -> String {
    if let Some(text) = content.as_str() {
        return text.to_string();
    }
    let mut parts = Vec::new();
    let mut image_count = 0usize;
    if let Some(blocks) = content.as_array() {
        for block in blocks {
            match block.get("type").and_then(Value::as_str).unwrap_or("") {
                "text" => {
                    if let Some(text) = block.get("text").and_then(Value::as_str) {
                        parts.push(text.to_string());
                    }
                }
                "image" => image_count += 1,
                _ => {}
            }
        }
    }
    for _ in 0..image_count {
        parts.push("[Image]".to_string());
    }
    parts.join("\n")
}

/// 剥掉子代理回报行的 DSH 包装前缀文案：
/// report 首行是 "Background subagent <id> reported:" 样板；settled 在此之上
/// 还多一行 "Its closing message:" 前缀。返回正文（trim 后）。
fn strip_subagent_wrapper<'a>(text: &'a str, source_kind: &str) -> &'a str {
    let mut body = text;
    if let Some((first, rest)) = body.split_once('\n') {
        if first.starts_with("Background subagent") {
            body = rest;
        }
    }
    let body = body.trim_start();
    if source_kind == "subagent-settled" {
        let body = body.strip_prefix("Its closing message:").unwrap_or(body);
        return body.trim();
    }
    body.trim()
}

/// `user/message` → 用户消息。
///
/// 系统注入的上下文转储（workspace 指令、运行时快照/目录/转述）折叠不渲染。
/// `source.kind` 为 `"subagent-report"`/`"subagent-settled"` 的行是子代理回报：
/// 剥掉 "Background subagent …" 包装文案后渲染为 `[subagent_report]`/`[subagent_settled]`
/// 标记的 assistant 文本行（前端无第三 role，归入助手消息列呈现）。
fn handle_user_message(entry: &Value, state: &mut ParseState) {
    let data = entry.get("data").unwrap_or(entry);
    let source_kind = data
        .pointer("/source/kind")
        .and_then(Value::as_str)
        .unwrap_or("");
    // 子代理回报行：剥包装后渲染为标记行
    if source_kind == "subagent-report" || source_kind == "subagent-settled" {
        let text = data.get("content").map(extract_block_text).unwrap_or_default();
        let tag = if source_kind == "subagent-report" {
            "[subagent_report]"
        } else {
            "[subagent_settled]"
        };
        let body = strip_subagent_wrapper(&text, source_kind);
        if !body.is_empty() {
            state.push_message(
                ChatMessage {
                    role: "assistant".to_string(),
                    timestamp: dsh_timestamp(entry),
                    model: None,
                    token_usage: None,
                    content_parts: vec![ContentPart::Text {
                        text: format!("{tag} {body}"),
                    }],
                    is_meta: false,
                    uuid: None,
                },
                entry_seq(entry),
            );
        }
        return;
    }
    // 系统注入上下文（workspace 指令 / 运行时快照）非会话内容，折叠。
    if source_kind == "agent-instructions" {
        return;
    }
    if source_kind == "plugin" {
        if let Some(form) = data.pointer("/source/form").and_then(Value::as_str) {
            if matches!(
                form,
                "snapshot" | "instructions" | "catalog" | "relay" | "recall"
            ) {
                return;
            }
        }
    }
    let Some(content) = data.get("content") else {
        return;
    };
    let parts = parse_message_blocks(content);
    if parts.is_empty() {
        return;
    }
    state.push_message(
        ChatMessage {
            role: "user".to_string(),
            timestamp: dsh_timestamp(entry),
            model: None,
            token_usage: None,
            content_parts: parts,
            is_meta: false,
            uuid: None,
        },
        entry_seq(entry),
    );
}

/// `assistant/message` → 助手消息。
///
/// 实际消息对象在 `data.message` 下（`turn`/`step` 包装）；兼容无包装的简化行
/// （content 直接在 `data` 下）。内容块中的 `tool-call` 调用 id 记入 `seen_tool_use_ids`，
/// 供 `tool/call` 事件去重；`subagent` 调用的 description 记入 `subagent_call_descs`，
/// 供 `"started subagent <id>"` 结果行建 subagent_map。装配完成后丢弃该 step 的 chunk
/// 缓冲并标记已装配（完整消息取代流式增量，迟到的 chunk 行不得重复文本）。
fn handle_assistant_message(entry: &Value, state: &mut ParseState) {
    let data = entry.get("data").unwrap_or(entry);
    let message = data.get("message").unwrap_or(data);
    // 装配好的消息取代 step 的原始 chunk 增量：丢弃缓冲并标记已装配。
    if let (Some(turn), Some(step)) = (
        data.get("turn").and_then(Value::as_u64),
        data.get("step").and_then(Value::as_u64),
    ) {
        let key = (turn as u32, step as u32);
        state.chunk_bufs.remove(&key);
        state.assembled_steps.insert(key);
    }
    let model = message
        .pointer("/source/model")
        .and_then(Value::as_str)
        .map(str::to_string);
    // usage 在 data 层（与 turn/step 并列）；映射为 TokenUsage 挂到本行助手消息。
    let usage = data.get("usage").and_then(dsh_token_usage);
    let content = message.get("content");
    let parts = content.map(parse_message_blocks).unwrap_or_default();
    for part in &parts {
        if let ContentPart::ToolUse {
            tool_name,
            input,
            tool_use_id: Some(id),
            ..
        } = part
        {
            state.seen_tool_use_ids.insert(id.clone());
            record_subagent_call_desc(state, tool_name, input, id);
        }
    }
    if parts.is_empty() {
        return;
    }
    state.push_message(
        ChatMessage {
            role: "assistant".to_string(),
            timestamp: dsh_timestamp(entry),
            model,
            token_usage: usage,
            content_parts: parts,
            is_meta: false,
            uuid: None,
        },
        entry_seq(entry),
    );
}

/// 记录 `subagent` 工具调用的 description（父级为委托起的名字），
/// 供 `"started subagent <id>"` 结果行建 subagent_map 时作 label。
fn record_subagent_call_desc(state: &mut ParseState, name: &str, arguments_raw: &str, call_id: &str) {
    if name != "subagent" {
        return;
    }
    let desc = serde_json::from_str::<Value>(arguments_raw)
        .ok()
        .and_then(|args| args.get("description").and_then(Value::as_str).map(str::to_string))
        .filter(|d| !d.trim().is_empty());
    if let Some(desc) = desc {
        state.subagent_call_descs.insert(call_id.to_string(), desc);
    }
}

/// `tool/call` → 助手消息（ToolUse content part）。
/// 若该调用已由 `assistant/message` 的 `tool-call` 块暴露，则跳过避免重复。
fn handle_tool_call(entry: &Value, state: &mut ParseState) {
    let data = entry.get("data").unwrap_or(entry);
    let Some(call_id) = data.get("callId").and_then(Value::as_str) else {
        return;
    };
    if state.seen_tool_use_ids.contains(call_id) {
        return;
    }
    state.seen_tool_use_ids.insert(call_id.to_string());
    let name = data.get("name").and_then(Value::as_str).unwrap_or("tool");
    let arguments = data
        .get("arguments")
        .and_then(Value::as_str)
        .unwrap_or("");
    record_subagent_call_desc(state, name, arguments, call_id);
    let parts = vec![tool_use_part(name, arguments, Some(call_id.to_string()))];
    state.push_message(
        ChatMessage {
            role: "assistant".to_string(),
            timestamp: dsh_timestamp(entry),
            model: None,
            token_usage: None,
            content_parts: parts,
            is_meta: false,
            uuid: None,
        },
        entry_seq(entry),
    );
}

/// `tool/result` → 用户消息（ToolResult content part）。
/// 前端将 tool_result-only 用户消息并入当前助手回合的工具组。
/// `subagent` 调用的结果文本 `"started subagent <id>"` 会建立 subagent_map
/// （tool_use_id → 同项目子会话文件），供前端 chip 导航打开子会话。
fn handle_tool_result(entry: &Value, state: &mut ParseState) {
    let data = entry.get("data").unwrap_or(entry);
    let Some(message) = data.get("message") else {
        return;
    };
    let Some(content) = message.get("content") else {
        return;
    };
    let parts = parse_message_blocks(content);
    if parts.is_empty() {
        return;
    }
    // subagent 委托结果：文本形如 "started subagent <子会话id>"（真实数据验证）。
    // 子会话是同项目下的兄弟目录，建立 tool_use_id → 子会话文件的导航映射。
    if let Some(call_id) = message.pointer("/source/callId").and_then(Value::as_str) {
        for part in &parts {
            if let ContentPart::ToolResult { content, .. } = part {
                maybe_map_subagent_session(state, call_id, content);
            }
        }
    }
    state.push_message(
        ChatMessage {
            role: "user".to_string(),
            timestamp: dsh_timestamp(entry),
            model: None,
            token_usage: None,
            content_parts: parts,
            is_meta: false,
            uuid: None,
        },
        entry_seq(entry),
    );
}

/// 若 result 文本是 `"started subagent <子会话id>"`，把该 tool_use_id 映射到
/// 同项目下的子会话文件（zstd 优先；不存在则跳过），label 取调用的 description。
fn maybe_map_subagent_session(state: &mut ParseState, call_id: &str, result_text: &str) {
    if state.subagent_map.contains_key(call_id) {
        return;
    }
    let Some(child_id) = result_text
        .trim()
        .strip_prefix("started subagent ")
        .and_then(|rest| rest.split_whitespace().next())
        .filter(|id| !id.is_empty())
    else {
        return;
    };
    let Some(session_path) = state.session_file_path.as_ref() else {
        return;
    };
    // <项目dir>/<父会话id>/session.jsonl.zstd → <项目dir>/<子会话id>/session.jsonl[.zstd]
    let Some(project_dir) = session_path.parent().and_then(Path::parent) else {
        return;
    };
    let child_dir = project_dir.join(child_id);
    let child_file = ["session.jsonl.zstd", "session.jsonl"]
        .iter()
        .map(|name| child_dir.join(name))
        .find(|path| path.is_file());
    let Some(child_file) = child_file else {
        return;
    };
    let label = state
        .subagent_call_descs
        .get(call_id)
        .cloned()
        .unwrap_or_else(|| {
            let short: String = child_id.chars().take(6).collect();
            format!("Subagent {short}")
        });
    state.subagent_map.insert(
        call_id.to_string(),
        session::SubagentInfo {
            file_path: child_file.to_string_lossy().to_string(),
            label,
        },
    );
}

/// 流式 chunk 行（`assistant/chunk` / `text-chunks` / `reasoning-chunks` /
/// `tool-call-chunks`）：把增量拼进 `(turn,step)` 缓冲，本身不产生消息。
///
/// 按流 block index 键控；`assistant/chunk` 的增量在 `data.chunk` 下
/// （`text-delta`/`reasoning-delta`/`tool-call-delta`），打包行
/// （`text-chunks`/`reasoning-chunks`/`tool-call-chunks`）的整块在 `data` 下
/// （`index` + `texts`/`args` 数组）。step 已装配 `assistant/message` 时忽略，
/// 避免乱序日志重复文本。
fn handle_chunk_event(event_type: &str, data: &Value, state: &mut ParseState) {
    let (Some(turn), Some(step)) = (
        data.get("turn").and_then(Value::as_u64),
        data.get("step").and_then(Value::as_u64),
    ) else {
        return;
    };
    let key = (turn as u32, step as u32);
    if state.assembled_steps.contains(&key) {
        return;
    }
    let buf = state.chunk_bufs.entry(key).or_default();
    match event_type {
        "assistant/chunk" => {
            let Some(chunk) = data.get("chunk") else {
                return;
            };
            // usage 增量不携带 block index：先于 index 门槛捕获，中断流 flush 时保留记账。
            if chunk.get("type").and_then(Value::as_str) == Some("usage") {
                if let Some(usage) = chunk.get("usage").and_then(dsh_token_usage) {
                    buf.usage = Some(usage);
                }
                return;
            }
            let Some(index) = chunk.get("index").and_then(Value::as_u64) else {
                return;
            };
            let index = index as usize;
            match chunk.get("type").and_then(Value::as_str).unwrap_or("") {
                "text-delta" => {
                    if let Some(text) = chunk.get("text").and_then(Value::as_str) {
                        buf.text.entry(index).or_default().push_str(text);
                    }
                }
                "reasoning-delta" => {
                    if let Some(text) = chunk.get("text").and_then(Value::as_str) {
                        buf.reasoning.entry(index).or_default().push_str(text);
                    }
                }
                "tool-call-delta" => {
                    let id = chunk
                        .get("id")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    let name = chunk
                        .get("name")
                        .and_then(Value::as_str)
                        .map(str::to_string);
                    let arguments_delta = chunk
                        .get("argumentsDelta")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    let entry = buf
                        .tool_calls
                        .entry(index)
                        .or_insert_with(|| (id.clone(), name.clone(), String::new()));
                    // 后续 delta 可能省略 id（仅首个 delta 携带）；不得以空串覆盖已存 id。
                    if !id.is_empty() {
                        entry.0 = id;
                    }
                    if name.is_some() {
                        entry.1 = name;
                    }
                    entry.2.push_str(arguments_delta);
                }
                // block-start / usage 等无增量文本的 chunk 类型跳过。
                _ => {}
            }
        }
        "text-chunks" | "reasoning-chunks" => {
            let Some(index) = data.get("index").and_then(Value::as_u64) else {
                return;
            };
            let joined: String = data
                .get("texts")
                .and_then(Value::as_array)
                .map(|texts| texts.iter().filter_map(Value::as_str).collect::<String>())
                .unwrap_or_default();
            if joined.is_empty() {
                return;
            }
            let index = index as usize;
            let target = if event_type == "text-chunks" {
                &mut buf.text
            } else {
                &mut buf.reasoning
            };
            target.entry(index).or_default().push_str(&joined);
        }
        "tool-call-chunks" => {
            let Some(index) = data.get("index").and_then(Value::as_u64) else {
                return;
            };
            let id = data
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let name = data.get("name").and_then(Value::as_str).map(str::to_string);
            let args: String = data
                .get("args")
                .and_then(Value::as_array)
                .map(|args| args.iter().filter_map(Value::as_str).collect::<String>())
                .unwrap_or_default();
            let index = index as usize;
            let entry = buf
                .tool_calls
                .entry(index)
                .or_insert_with(|| (id.clone(), name.clone(), String::new()));
            // 打包行的后续行可能省略 id（仅首个行携带）；不得以空串覆盖已存 id。
            if !id.is_empty() {
                entry.0 = id;
            }
            if name.is_some() {
                entry.1 = name;
            }
            entry.2.push_str(&args);
        }
        _ => {}
    }
}

/// Flush 一个 step 的缓冲 chunk 为临时助手消息（中断流）。
/// 仅用于 `assistant/message` 未装配的 step；正常路径在消息到达时丢弃缓冲。
/// 块按 index 顺序拼成单个助手消息的 `ContentPart`（与 `handle_assistant_message`
/// 的装配约定一致）；已由 `tool/call` 暴露的调用不再重复。usage 增量（若有）随消息
/// 保留。消息以 `seq`（`step/end` 行 seq）为 provenance，供 compaction 拼接遮蔽。
fn flush_step_chunks(state: &mut ParseState, turn: u32, step: u32, seq: Option<u64>) {
    let Some(buf) = state.chunk_bufs.remove(&(turn, step)) else {
        return;
    };
    let mut indices: Vec<usize> = buf
        .text
        .keys()
        .chain(buf.reasoning.keys())
        .chain(buf.tool_calls.keys())
        .copied()
        .collect();
    indices.sort_unstable();
    indices.dedup();
    let mut parts = Vec::new();
    for index in indices {
        if let Some(text) = buf
            .reasoning
            .get(&index)
            .filter(|text| !text.trim().is_empty())
        {
            parts.push(ContentPart::Thinking {
                thinking: text.clone(),
            });
        }
        if let Some(text) = buf.text.get(&index).filter(|text| !text.trim().is_empty()) {
            parts.push(ContentPart::Text { text: text.clone() });
        }
        if let Some((call_id, name, arguments)) = buf.tool_calls.get(&index) {
            if !state.seen_tool_use_ids.contains(call_id) {
                state.seen_tool_use_ids.insert(call_id.clone());
                parts.push(tool_use_part(
                    name.as_deref().unwrap_or("tool"),
                    arguments,
                    Some(call_id.clone()),
                ));
            }
        }
    }
    if parts.is_empty() {
        return;
    }
    state.push_message(
        ChatMessage {
            role: "assistant".to_string(),
            timestamp: String::new(),
            model: None,
            token_usage: buf.usage,
            content_parts: parts,
            is_meta: false,
            uuid: None,
        },
        seq,
    );
}

/// 全量解析一个 DSH 会话文件（zstd/plain 自适应）。
pub(crate) fn parse_dsh_session_file(file_path: &str) -> AppResult<Vec<ChatMessage>> {
    let lines = read_session_lines(file_path)?;
    parse_rows(lines)
}

/// 带 offset 的全量解析：返回全部消息与行水位（`seq` 优先，缺失用行号），
/// 与增量解析共用同一水位坐标，初始 offset 可直接喂给增量入口。
///
/// 生产链路走 `parse_dsh_session_incremental`，本函数仅供等价性测试使用。
#[cfg(test)]
pub(crate) fn parse_dsh_session_file_with_offset(
    file_path: &str,
) -> AppResult<SessionLoadResult> {
    let lines = read_session_lines(file_path)?;
    let (messages, watermark, subagent_map) =
        parse_rows_watermark(lines, None, Some(Path::new(file_path)))?;
    Ok(SessionLoadResult {
        messages,
        offset: watermark,
        subagent_map,
    })
}

/// 增量解析：按行水位过滤，只返回水位 `> offset` 的新增消息，并把 `result.offset` 推进到
/// 新水位。DSH 日志可能为 zstd 压缩，无法按字节 seek，故全量重读后按水位过滤（追加幂等：
/// 用返回值再调一次不产生重复）。subagent_map 每次全量重建，保持完整。
pub(crate) fn parse_dsh_session_incremental(
    file_path: &str,
    offset: u64,
) -> AppResult<SessionLoadResult> {
    let lines = read_session_lines(file_path)?;
    let (messages, watermark, subagent_map) =
        parse_rows_watermark(lines, Some(offset), Some(Path::new(file_path)))?;
    Ok(SessionLoadResult {
        messages,
        offset: watermark,
        subagent_map,
    })
}

/// 流式解析：全量解析后按 `batch_size` 分批发回调。
/// 返回的 offset 为行水位（末条消息的 `seq`，缺失用行号），与增量入口同一坐标系。
pub(crate) fn parse_dsh_session_file_streaming<F>(
    file_path: &str,
    mut on_batch: F,
) -> AppResult<(
    u64,
    std::collections::HashMap<String, session::SubagentInfo>,
)>
where
    F: FnMut(Vec<session::ChatMessage>) -> bool,
{
    let lines = read_session_lines(file_path)?;
    let (messages, offset, subagent_map) =
        parse_rows_watermark(lines, None, Some(Path::new(file_path)))?;
    let mut emitter = BatchEmitter::new();
    for message in messages {
        emitter.push(message);
        if !emitter.maybe_flush(&mut on_batch) {
            return Ok((offset, subagent_map));
        }
    }
    emitter.flush_remaining(&mut on_batch);
    Ok((offset, subagent_map))
}

/// 从内存内容解析 DSH 会话消息（归档兜底用）。
pub(crate) fn parse_dsh_session_from_string(content: &str) -> Vec<session::ChatMessage> {
    let rows: Vec<&str> = content.lines().collect();
    parse_rows(rows).unwrap_or_default()
}

/// 提取 DSH 会话的用量记录（会话统计/分析用）：逐行扫描 `assistant/message` 行的
/// `data.usage`（camelCase → `TokenUsage` 同映射）。只扫装配消息不扫 chunk 增量
/// （中断流的 usage chunk 量小且稀少，避免与装配消息重复计数）；被 compaction 拼接掉的
/// 消息行仍在日志中，其用量照常计入（遵循上下文折叠时全量用量累计语义）。
pub(crate) fn extract_usage_records(
    file_path: &str,
    project: &str,
) -> Vec<session::UsageRecord> {
    let Ok(lines) = read_session_lines(file_path) else {
        return Vec::new();
    };
    let mut records = Vec::new();
    for line in &lines {
        let line = line.trim();
        if !line.contains(r#""type":"assistant/message""#) {
            continue;
        }
        let Ok(entry) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let data = entry.get("data").unwrap_or(&entry);
        let Some(usage) = data.get("usage").and_then(dsh_token_usage) else {
            continue;
        };
        if usage.input_tokens == 0
            && usage.output_tokens == 0
            && usage.cache_creation_input_tokens == 0
            && usage.cache_read_input_tokens == 0
        {
            continue;
        }
        let model = data
            .pointer("/message/source/model")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        let date = crate::parser::claude::timestamp_to_local_date(&dsh_timestamp(&entry));
        records.push(session::UsageRecord {
            date,
            model,
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            cache_creation_tokens: usage.cache_creation_input_tokens,
            cache_read_tokens: usage.cache_read_input_tokens,
            duration_ms: None,
            project: project.to_string(),
        });
    }
    records
}

/// 判断会话是否含可展示的表面消息（`user/message` / `assistant/message` 事件）。
/// 流式逐行扫描、子串预过滤、命中即返回，不整文件解析；用于索引扫描过滤
/// "从未开始"的空会话（只有 header + 策略事件），无实质交互内容，不纳入有效会话列表：
/// "无 surfaced 消息不进列表"行为。
pub(crate) fn has_surface_messages(file_path: &Path) -> bool {
    let Ok(file) = File::open(file_path) else {
        return false;
    };
    let reader: Box<dyn BufRead> = if is_zstd_path(file_path) {
        match zstd::stream::read::Decoder::new(file) {
            Ok(decoder) => Box::new(BufReader::new(decoder)),
            Err(_) => return false,
        }
    } else {
        Box::new(BufReader::new(file))
    };
    for line in reader.lines() {
        let Ok(line) = line else { continue };
        if line.contains(r#""type":"user/message""#) || line.contains(r#""type":"assistant/message""#) {
            return true;
        }
    }
    false
}

/// 会话头 `createdAt`（毫秒 epoch）→ RFC3339 字符串；缺失时 None。
fn header_created_at_timestamp(header: &Value) -> Option<String> {
    header
        .get("createdAt")
        .and_then(Value::as_i64)
        .and_then(chrono::DateTime::from_timestamp_millis)
        .map(|dt| dt.to_rfc3339())
}

/// 首个真实用户消息文本：读头行后顺序扫描，返回首个 `user/message` 的
/// `data.content` 中第一个 `text` 块文本（复用消息映射的过滤语义：跳过
/// `agent-instructions`、plugin 快照等系统注入与子代理回报行）。无则 None。
/// 逐行流式读取（zstd/plain 自适应），命中首个用户消息即提前返回，不整文件解压。
pub(crate) fn read_first_user_message(file_path: &str) -> Option<String> {
    let path = Path::new(file_path);
    let file = File::open(path).ok()?;
    let reader: Box<dyn BufRead> = if is_zstd_path(path) {
        Box::new(BufReader::new(zstd::stream::read::Decoder::new(file).ok()?))
    } else {
        Box::new(BufReader::new(file))
    };
    for line in reader.lines() {
        let line = match line {
            Ok(line) => line,
            Err(_) => continue,
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(entry) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };
        match entry.get("type").and_then(Value::as_str) {
            // 会话头与标题为元数据，不产生用户消息。
            Some("session") | Some("session/title") => {}
            Some("user/message") => {
                let data = entry.get("data").unwrap_or(&entry);
                let source_kind = data
                    .pointer("/source/kind")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                if source_kind.starts_with("subagent") {
                    continue;
                }
                if source_kind == "agent-instructions" {
                    continue;
                }
                if source_kind == "plugin" {
                    if let Some(form) = data.pointer("/source/form").and_then(Value::as_str) {
                        if matches!(
                            form,
                            "snapshot" | "instructions" | "catalog" | "relay" | "recall"
                        ) {
                            continue;
                        }
                    }
                }
                let Some(content) = data.get("content") else {
                    continue;
                };
                if let Some(text) = first_text_block(content) {
                    return Some(text);
                }
            }
            // 其余事件（assistant/tool/纯日志）不产生用户消息。
            _ => {}
        }
    }
    None
}

/// 从 DSH 内容块数组中取首个 `text` 块文本（复用 `parse_message_blocks` 的块解析）。
fn first_text_block(content: &Value) -> Option<String> {
    parse_message_blocks(content).into_iter().find_map(|part| match part {
        ContentPart::Text { text } => Some(text),
        _ => None,
    })
}

/// 会话 id：头行 `id`，缺失时回退到会话目录名（`.../<session-id>/session.jsonl*`）。
pub(crate) fn read_session_id(file_path: &str) -> Option<String> {
    let header = read_header_line(Path::new(file_path))?;
    header
        .get("id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|id| !id.trim().is_empty())
        .or_else(|| {
            Path::new(file_path)
                .parent()
                .and_then(|parent| parent.file_name())
                .and_then(|name| name.to_str())
                .map(str::to_string)
        })
}

/// 项目路径：头行 `cwd`。
pub(crate) fn read_project_path(file_path: &str) -> Option<String> {
    let header = read_header_line(Path::new(file_path))?;
    parse_session_header_value(&header).map(|(cwd, _, _)| cwd)
}

/// 会话列表元数据（缓存索引陈旧检查用）：id/cwd/createdAt 由头行派生；
/// 标题与 `build_dsh_session_metadata` 一致——title 为 None（DSH 扫描期不读
/// `session/title` LLM 标题，其在日志尾部读取太贵），展示名经 title_resolver
/// 回退到清洗后的首条用户消息。
/// 无会话头时返回 None（与其它 CLI 的 metadata-only 扫描无聊天消息返回 None 一致）。
pub(crate) fn scan_session_metadata_only(file_path: &Path) -> Option<session::SessionListMetadata> {
    let header = read_header_line(file_path)?;
    let file_size = std::fs::metadata(file_path).ok().map(|m| m.len()).unwrap_or(0);
    let cwd = parse_session_header_value(&header)
        .map(|(cwd, _, _)| cwd)
        .filter(|cwd| !cwd.trim().is_empty());
    let session_id = header
        .get("id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|id| !id.trim().is_empty())
        .or_else(|| {
            file_path
                .parent()
                .and_then(|parent| parent.file_name())
                .and_then(|name| name.to_str())
                .map(str::to_string)
        })?;
    let timestamp = header_created_at_timestamp(&header);
    let first_user_message = read_first_user_message(&file_path.to_string_lossy())
        .and_then(|text| crate::title_resolver::clean_fallback_user_text(&text));
    Some(session::SessionListMetadata {
        session_id,
        project_path: cwd,
        title: None,
        first_user_message,
        first_timestamp: timestamp.clone(),
        last_timestamp: timestamp,
        git_branch: String::new(),
        file_size,
    })
}

/// 从解析消息中提取 (role, text)：text 为 `text`/`thinking` 内容块拼接文本。
fn message_role_text(message: &ChatMessage) -> Option<(String, String)> {
    let texts: Vec<String> = message
        .content_parts
        .iter()
        .filter_map(|part| match part {
            ContentPart::Text { text } => Some(text.clone()),
            ContentPart::Thinking { thinking } => Some(thinking.clone()),
            _ => None,
        })
        .collect();
    let joined = texts.join("\n");
    if joined.trim().is_empty() {
        None
    } else {
        Some((message.role.clone(), joined))
    }
}

/// 会话搜索文档（带进度）：解析消息后按 text/thinking 块产出可搜索文本。
/// 与其它 CLI 的 extractor 分支输出同形：`Vec<SearchDocument>`，空时 None。
pub(crate) fn scan_search_docs_with_progress<F>(
    file_path: &Path,
    mut on_progress: F,
) -> Option<Vec<SearchDocument>>
where
    F: FnMut(SearchScanProgress),
{
    let path_str = file_path.to_string_lossy().to_string();
    let lines = read_session_lines(&path_str).ok()?;
    let file_size = std::fs::metadata(file_path).ok().map(|m| m.len()).unwrap_or(0);
    let messages = parse_rows(lines).ok()?;
    let mut docs = Vec::new();
    let mut message_index = 0usize;
    for message in messages {
        let Some((_, search_text)) = message_role_text(&message) else {
            continue;
        };
        docs.push(SearchDocument {
            message_index,
            role: message.role,
            search_text,
        });
        message_index += 1;
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

/// 从已解档内容构建会话搜索文档（归档兜底用，与 `scan_search_docs_with_progress` 同形）。
pub(crate) fn scan_search_docs_from_bytes(content: &[u8]) -> Option<Vec<SearchDocument>> {
    let text = String::from_utf8_lossy(content);
    let messages = parse_dsh_session_from_string(&text);
    let mut docs = Vec::new();
    let mut message_index = 0usize;
    for message in messages {
        let Some((_, search_text)) = message_role_text(&message) else {
            continue;
        };
        docs.push(SearchDocument {
            message_index,
            role: message.role,
            search_text,
        });
        message_index += 1;
    }
    if docs.is_empty() {
        None
    } else {
        Some(docs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_kind_dsh_path() {
        let kind = crate::parser::detect_kind("/Users/u/.dsh/sessions/--a--/sid/session.jsonl.zstd");
        assert!(matches!(kind, crate::parser::SessionKind::Dsh));
    }

    #[test]
    fn zstd_decompress_roundtrip() {
        let raw = b"{\"type\":\"session\"}\n{\"type\":\"user/message\"}\n";
        let c = zstd::encode_all(std::io::Cursor::new(&raw[..]), 3).unwrap();
        let mut d = zstd::stream::read::Decoder::new(&c[..]).unwrap();
        let mut out = String::new();
        std::io::Read::read_to_string(&mut d, &mut out).unwrap();
        assert_eq!(out.as_bytes(), &raw[..]);
    }

    #[test]
    fn read_header_line_reads_zstd_session_header() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl.zstd");
        let content = concat!(
            "{\"type\":\"session\",\"version\":0,\"id\":\"s1\",\"cwd\":\"/proj/a\",\"createdAt\":1786865077879}\n",
            "{\"type\":\"user/message\",\"seq\":1,\"data\":{\"content\":[{\"type\":\"text\",\"text\":\"hi\"}]}}\n",
        );
        let compressed = zstd::encode_all(std::io::Cursor::new(content.as_bytes()), 3).unwrap();
        std::fs::write(&path, compressed).unwrap();

        let header = super::read_header_line(&path).unwrap();
        assert_eq!(header.get("type").and_then(Value::as_str), Some("session"));
        assert_eq!(header.get("id").and_then(Value::as_str), Some("s1"));
        assert_eq!(header.get("cwd").and_then(Value::as_str), Some("/proj/a"));
        assert_eq!(header.get("createdAt").and_then(Value::as_i64), Some(1786865077879));
    }

    #[test]
    fn read_header_line_reads_plain_session_header() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        std::fs::write(
            &path,
            concat!(
                "{\"type\":\"session\",\"version\":0,\"id\":\"s1\",\"cwd\":\"/proj/b\"}\n",
                "{\"type\":\"user/message\",\"seq\":1}\n",
            ),
        )
        .unwrap();

        let header = super::read_header_line(&path).unwrap();
        assert_eq!(header.get("cwd").and_then(Value::as_str), Some("/proj/b"));
    }

    #[test]
    fn read_header_line_rejects_non_session_first_line() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl.zstd");
        let content = concat!(
            "{\"type\":\"user/message\",\"seq\":1}\n",
            "{\"type\":\"session\",\"cwd\":\"/proj/a\"}\n",
        );
        let compressed = zstd::encode_all(std::io::Cursor::new(content.as_bytes()), 3).unwrap();
        std::fs::write(&path, compressed).unwrap();

        assert!(
            super::read_header_line(&path).is_none(),
            "首个非空行非 session 头不应返回"
        );
    }

    #[test]
    fn header_extracts_cwd_parent_subagent() {
        let line = r#"{"type":"session","version":0,"id":"sid","createdAt":1786865077879,"cwd":"/proj/a","parentSession":"p1","origin":"subagent"}"#;
        let (cwd, parent, is_sub) = parse_session_header(line).unwrap();
        assert_eq!(cwd, "/proj/a");
        assert_eq!(parent, Some("p1".into()));
        assert!(is_sub);
    }

    #[test]
    fn parses_basic_user_assistant_text() {
        let rows = vec![
            r#"{"type":"session","version":0,"id":"s1","cwd":"/tmp/x","createdAt":1700000000000}"#,
            r#"{"type":"user/message","seq":1,"time":1700000000000,"data":{"content":[{"type":"text","text":"hi"}]}}"#,
            r#"{"type":"assistant/message","seq":2,"time":1700000000001,"data":{"content":[{"type":"text","text":"hello"}]}}"#,
        ];
        let msgs = parse_rows(rows).unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "user");
        assert_eq!(msgs[1].role, "assistant");
        assert!(matches!(msgs[1].content_parts[0], ContentPart::Text { .. }));
    }

    #[test]
    fn maps_content_blocks_to_content_parts() {
        let rows = vec![
            r#"{"type":"assistant/message","seq":1,"time":1700000000000,"data":{"message":{"content":[{"type":"reasoning","text":"let me think"},{"type":"text","text":"hi there"},{"type":"tool-call","id":"call_1","name":"bash","arguments":"{\"command\":\"ls\"}"}]}}}"#,
            r#"{"type":"tool/result","seq":2,"time":1700000000001,"data":{"message":{"source":{"kind":"tool","callId":"call_1"},"content":[{"type":"tool-result","toolCallId":"call_1","content":[{"type":"text","text":"file.txt"}],"isError":false}]}}}"#,
        ];
        let msgs = parse_rows(rows).unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "assistant");
        let parts = &msgs[0].content_parts;
        assert_eq!(parts.len(), 3);
        assert!(matches!(parts[0], ContentPart::Thinking { .. }));
        assert!(matches!(parts[1], ContentPart::Text { .. }));
        assert!(matches!(
            &parts[2],
            ContentPart::ToolUse {
                tool_name,
                tool_use_id,
                ..
            } if tool_name == "bash" && tool_use_id.as_deref() == Some("call_1")
        ));
        // tool/result → user 消息携带 ToolResult
        assert_eq!(msgs[1].role, "user");
        assert!(matches!(
            &msgs[1].content_parts[0],
            ContentPart::ToolResult { content, is_error, .. }
                if content == "file.txt" && !is_error
        ));
    }

    #[test]
    fn tool_call_event_does_not_duplicate_assistant_block() {
        let rows = vec![
            r#"{"type":"assistant/message","seq":1,"time":1700000000000,"data":{"message":{"content":[{"type":"tool-call","id":"call_1","name":"bash","arguments":"{}"}]}}}"#,
            r#"{"type":"tool/call","seq":2,"time":1700000000001,"data":{"callId":"call_1","name":"bash","arguments":"{}"}}"#,
        ];
        let msgs = parse_rows(rows).unwrap();
        assert_eq!(msgs.len(), 1, "tool/call 不得重复暴露已出现的调用");
    }

    #[test]
    fn subagent_report_rows_render_as_tagged_lines() {
        // 子代理回报处理：保留 subagent-report/settled 关键结果，剥掉 "Background subagent …"
        // 包装文案后渲染为 [subagent_report]/[subagent_settled] 标记的 assistant 文本行。
        let rows = vec![
            r#"{"type":"session","version":0,"id":"s1","cwd":"/tmp/x","createdAt":1700000000000}"#,
            r#"{"type":"user/message","seq":1,"time":1700000000000,"data":{"content":[{"type":"text","text":"hi"}],"source":{"kind":"user"}}}"#,
            r#"{"type":"user/message","seq":2,"time":1700000000001,"data":{"content":[{"type":"text","text":"Background subagent cb61a6b7 reported:"},{"type":"text","text":"Review complete."}],"source":{"kind":"subagent-report","form":"relay","senderSessionId":"cb61a6b7"}}}"#,
            r#"{"type":"user/message","seq":3,"time":1700000000002,"data":{"content":[{"type":"text","text":"Background subagent 036bdb9c finished and will do no further work unless you send it more."},{"type":"text","text":"Its closing message:"},{"type":"text","text":"Review complete.\nAll good."}],"source":{"kind":"subagent-settled","form":"notice","senderSessionId":"036bdb9c"}}}"#,
        ];
        let msgs = parse_rows(rows).unwrap();
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[0].role, "user");
        assert_eq!(msgs[1].role, "assistant");
        assert!(
            matches!(&msgs[1].content_parts[0], ContentPart::Text { text } if text == "[subagent_report] Review complete.")
        );
        assert_eq!(msgs[2].role, "assistant");
        assert!(
            matches!(&msgs[2].content_parts[0], ContentPart::Text { text } if text == "[subagent_settled] Review complete.\nAll good.")
        );
    }

    #[test]
    fn collapses_system_injected_context() {
        let rows = vec![
            r#"{"type":"user/message","seq":1,"time":1,"data":{"content":[{"type":"text","text":"real prompt"}],"source":{"kind":"user"}}}"#,
            r#"{"type":"user/message","seq":2,"time":2,"data":{"content":[{"type":"text","text":"AGENTS.md"}],"source":{"kind":"agent-instructions"}}}"#,
            r#"{"type":"user/message","seq":3,"time":3,"data":{"content":[{"type":"text","text":"snapshot"}],"source":{"kind":"plugin","form":"snapshot"}}}"#,
        ];
        let msgs = parse_rows(rows).unwrap();
        assert_eq!(msgs.len(), 1);
        assert!(matches!(&msgs[0].content_parts[0], ContentPart::Text { text } if text == "real prompt"));
    }

    #[test]
    fn assistant_model_and_timestamp_are_extracted() {
        let rows = vec![
            r#"{"type":"assistant/message","seq":1,"time":1700000000001,"data":{"message":{"role":"assistant","content":[{"type":"text","text":"hello"}],"source":{"kind":"model","model":"deepseek-v4-flash"}}}}"#,
        ];
        let msgs = parse_rows(rows).unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].role, "assistant");
        assert_eq!(msgs[0].model.as_deref(), Some("deepseek-v4-flash"));
        assert!(!msgs[0].timestamp.is_empty());
        assert!(msgs[0].timestamp.starts_with("2023-11-"));
    }

    #[test]
    fn incremental_respects_offset_watermark() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        let content = concat!(
            "{\"type\":\"session\",\"version\":0,\"id\":\"s1\",\"cwd\":\"/tmp/x\",\"createdAt\":1700000000000}\n",
            "{\"type\":\"user/message\",\"seq\":1,\"time\":1700000000000,\"data\":{\"content\":[{\"type\":\"text\",\"text\":\"hi\"}]}}\n",
            "{\"type\":\"assistant/message\",\"seq\":2,\"time\":1700000000001,\"data\":{\"content\":[{\"type\":\"text\",\"text\":\"hello\"}]}}\n",
        );
        std::fs::write(&path, content).unwrap();

        // 首次：返回全部 2 条，offset 推进到最高水位（seq=2）
        let first = parse_dsh_session_incremental(path.to_str().unwrap(), 0).unwrap();
        assert_eq!(first.messages.len(), 2);
        assert_eq!(first.offset, 2);

        // 用返回的 offset 再次调用：无新增，offset 不变（追加幂等）
        let second = parse_dsh_session_incremental(path.to_str().unwrap(), first.offset).unwrap();
        assert!(second.messages.is_empty());
        assert_eq!(second.offset, first.offset);

        // 追加新行后：只返回新消息，水位推进
        let mut file = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
        file.write_all(
            b"{\"type\":\"assistant/message\",\"seq\":3,\"time\":1700000000002,\"data\":{\"content\":[{\"type\":\"text\",\"text\":\"more\"}]}}\n",
        )
        .unwrap();
        drop(file);

        let third = parse_dsh_session_incremental(path.to_str().unwrap(), first.offset).unwrap();
        assert_eq!(third.messages.len(), 1);
        assert_eq!(third.messages[0].role, "assistant");
        assert!(matches!(
            &third.messages[0].content_parts[0],
            ContentPart::Text { text } if text == "more"
        ));
        assert_eq!(third.offset, 3);
    }

    #[test]
    fn watermark_falls_back_to_line_index_when_seq_absent() {
        let rows = vec![
            r#"{"type":"session","version":0,"id":"s1","cwd":"/tmp/x","createdAt":1700000000000}"#,
            r#"{"type":"user/message","time":1,"data":{"content":[{"type":"text","text":"hi"}]}}"#,
            r#"{"type":"assistant/message","time":2,"data":{"content":[{"type":"text","text":"hello"}]}}"#,
        ];
        let (messages, watermark, _) = parse_rows_watermark(rows.clone(), None, None).unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(watermark, 3);

        // 水位=3 时重放：无新增，水位不回落
        let (new_messages, new_watermark, _) =
            parse_rows_watermark(rows, Some(3), None).unwrap();
        assert!(new_messages.is_empty());
        assert_eq!(new_watermark, 3);
    }

    #[test]
    fn streaming_offset_feeds_incremental_without_dropping_new_rows() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        // 生产初始加载走 streaming：header + seq1 + seq2（行数为 3，但末条消息水位是 2）
        let content = concat!(
            "{\"type\":\"session\",\"version\":0,\"id\":\"s1\",\"cwd\":\"/tmp/x\",\"createdAt\":1700000000000}\n",
            "{\"type\":\"user/message\",\"seq\":1,\"time\":1700000000000,\"data\":{\"content\":[{\"type\":\"text\",\"text\":\"hi\"}]}}\n",
            "{\"type\":\"assistant/message\",\"seq\":2,\"time\":1700000000001,\"data\":{\"content\":[{\"type\":\"text\",\"text\":\"hello\"}]}}\n",
        );
        std::fs::write(&path, content).unwrap();

        let mut collected = Vec::new();
        let (stream_offset, _map) = parse_dsh_session_file_streaming(
            path.to_str().unwrap(),
            |batch| {
                collected.extend(batch);
                true
            },
        )
        .unwrap();
        assert_eq!(collected.len(), 2);
        assert_eq!(
            stream_offset, 2,
            "streaming offset 必须等于末条消息水位 seq=2，而不是行数 3（header 计入行数会漏掉 seq3）"
        );

        // 追加 seq=3 后，用 streaming 返回的 offset 走增量：恰好返回该新行
        let mut file = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
        file.write_all(
            b"{\"type\":\"assistant/message\",\"seq\":3,\"time\":1700000000002,\"data\":{\"content\":[{\"type\":\"text\",\"text\":\"more\"}]}}\n",
        )
        .unwrap();
        drop(file);

        let incremental =
            parse_dsh_session_incremental(path.to_str().unwrap(), stream_offset).unwrap();
        assert_eq!(incremental.messages.len(), 1);
        assert_eq!(incremental.messages[0].role, "assistant");
        assert!(matches!(
            &incremental.messages[0].content_parts[0],
            ContentPart::Text { text } if text == "more"
        ));
        assert_eq!(incremental.offset, 3);
    }

    #[test]
    fn chunk_stream_reassembles_then_places() {
        let rows = vec![
            r#"{"type":"assistant/chunk","seq":1,"data":{"turn":1,"step":1,"chunk":{"type":"text-delta","index":0,"text":"partial "}}}"#,
            r#"{"type":"assistant/chunk","seq":2,"data":{"turn":1,"step":1,"chunk":{"type":"text-delta","index":0,"text":"answer"}}}"#,
            r#"{"type":"assistant/message","seq":3,"data":{"turn":1,"step":1,"message":{"content":[{"type":"text","text":"部分答案"}]}}}"#,
        ];
        let msgs = parse_rows(rows).unwrap();
        assert_eq!(msgs.len(), 1, "chunk 增量不得产生消息，assistant/message 取代流式内容");
        assert_eq!(msgs[0].role, "assistant");
        assert!(matches!(
            &msgs[0].content_parts[0],
            ContentPart::Text { text } if text == "部分答案"
        ));
    }

    #[test]
    fn interrupted_chunks_flush_at_step_end() {
        // 流式 chunk 后无 assistant/message，step/end 时 flush 为临时消息。
        let rows = vec![
            r#"{"type":"assistant/chunk","seq":1,"data":{"turn":1,"step":1,"chunk":{"type":"text-delta","index":0,"text":"partial "}}}"#,
            r#"{"type":"assistant/chunk","seq":2,"data":{"turn":1,"step":1,"chunk":{"type":"text-delta","index":0,"text":"answer"}}}"#,
            r#"{"type":"step/end","seq":3,"data":{"turn":1,"step":1}}"#,
        ];
        let msgs = parse_rows(rows).unwrap();
        assert_eq!(msgs.len(), 1, "step/end 无 assistant/message 时 flush 缓冲为临时消息");
        assert_eq!(msgs[0].role, "assistant");
        assert!(matches!(
            &msgs[0].content_parts[0],
            ContentPart::Text { text } if text == "partial answer"
        ));
    }

    #[test]
    fn packed_chunk_rows_reassemble_in_block_order() {
        let rows = vec![
            r#"{"type":"reasoning-chunks","seq":1,"data":{"turn":1,"step":1,"index":0,"texts":["why ","not"]}}"#,
            r#"{"type":"text-chunks","seq":2,"data":{"turn":1,"step":1,"index":1,"texts":["partial ","answer"]}}"#,
            r#"{"type":"tool-call-chunks","seq":3,"data":{"turn":1,"step":1,"index":2,"id":"call_7","name":"bash","args":["{\"command\":\"ls\"}"]}}"#,
            r#"{"type":"step/end","seq":4,"data":{"turn":1,"step":1}}"#,
        ];
        let msgs = parse_rows(rows).unwrap();
        assert_eq!(msgs.len(), 1);
        let parts = &msgs[0].content_parts;
        assert_eq!(parts.len(), 3, "块按 index 顺序拼装：reasoning/text/tool-call");
        assert!(matches!(&parts[0], ContentPart::Thinking { thinking } if thinking == "why not"));
        assert!(matches!(&parts[1], ContentPart::Text { text } if text == "partial answer"));
        assert!(matches!(
            &parts[2],
            ContentPart::ToolUse { tool_name, tool_use_id, .. }
                if tool_name == "bash" && tool_use_id.as_deref() == Some("call_7")
        ));
    }

    #[test]
    fn late_chunk_rows_do_not_duplicate_assembled_text() {
        let rows = vec![
            r#"{"type":"assistant/message","seq":1,"data":{"turn":1,"step":1,"message":{"content":[{"type":"text","text":"assembled answer"}]}}}"#,
            r#"{"type":"text-chunks","seq":2,"data":{"turn":1,"step":1,"index":0,"texts":["assembled answer"]}}"#,
            r#"{"type":"step/end","seq":3,"data":{"turn":1,"step":1}}"#,
        ];
        let msgs = parse_rows(rows).unwrap();
        assert_eq!(msgs.len(), 1, "装配后迟到的 chunk 行不得重复文本");
        assert!(matches!(
            &msgs[0].content_parts[0],
            ContentPart::Text { text } if text == "assembled answer"
        ));
    }

    #[test]
    fn chunk_flush_respects_watermark_incremental() {
        let rows = vec![
            r#"{"type":"user/message","seq":1,"time":1,"data":{"content":[{"type":"text","text":"hi"}]}}"#,
            r#"{"type":"assistant/chunk","seq":2,"data":{"turn":1,"step":1,"chunk":{"type":"text-delta","index":0,"text":"partial "}}}"#,
            r#"{"type":"assistant/chunk","seq":3,"data":{"turn":1,"step":1,"chunk":{"type":"text-delta","index":0,"text":"answer"}}}"#,
            r#"{"type":"step/end","seq":4,"data":{"turn":1,"step":1}}"#,
        ];
        // 增量：offset=1（user 已送达），新 chunk + step/end → 只返回 flush 的 assistant 消息
        let (messages, watermark, _) = parse_rows_watermark(rows.clone(), Some(1), None).unwrap();
        assert_eq!(messages.len(), 1);
        assert!(matches!(
            &messages[0].content_parts[0],
            ContentPart::Text { text } if text == "partial answer"
        ));
        assert_eq!(watermark, 4);

        // 同水位重放：无新增、水位不回落（追加幂等）
        let (again, same_watermark, _) = parse_rows_watermark(rows, Some(4), None).unwrap();
        assert!(again.is_empty());
        assert_eq!(same_watermark, 4);
    }

    #[test]
    fn tool_call_delta_without_id_keeps_first_id_and_dedup() {
        // 后续 delta 省略 id（真实流式增量常见）：不得清空首块 id，否则
        // ① flush 出的 ToolUse 丢失 tool_use_id；② seen_tool_use_ids 记入空串，
        //    迟到的 tool/call(call_7) 不再被识别为已见而重复。
        let rows = vec![
            r#"{"type":"assistant/chunk","seq":1,"data":{"turn":1,"step":1,"chunk":{"type":"tool-call-delta","index":0,"id":"call_7","name":"bash","argumentsDelta":"{\"command\":\"ls\"}"}}}"#,
            r#"{"type":"assistant/chunk","seq":2,"data":{"turn":1,"step":1,"chunk":{"type":"tool-call-delta","index":0,"argumentsDelta":""}}}"#,
            r#"{"type":"step/end","seq":3,"data":{"turn":1,"step":1}}"#,
            r#"{"type":"tool/call","seq":4,"data":{"callId":"call_7","name":"bash","arguments":"{}"}}"#,
        ];
        let msgs = parse_rows(rows).unwrap();
        assert_eq!(msgs.len(), 1, "flush 已记入 call_7，迟到的 tool/call 不得重复暴露");
        assert!(matches!(
            &msgs[0].content_parts[0],
            ContentPart::ToolUse { tool_name, tool_use_id, .. }
                if tool_name == "bash" && tool_use_id.as_deref() == Some("call_7")
        ));
    }

    #[test]
    fn tool_call_chunks_without_id_keeps_first_id() {
        // tool-call-chunks 打包行的后续行省略 id（仅首个行携带）：flush 仍保留首块 id。
        let rows = vec![
            r#"{"type":"tool-call-chunks","seq":1,"data":{"turn":1,"step":1,"index":0,"id":"call_8","name":"grep","args":["{\"pattern\":\"x\"}"]}}"#,
            r#"{"type":"tool-call-chunks","seq":2,"data":{"turn":1,"step":1,"index":0,"args":[""]}}"#,
            r#"{"type":"step/end","seq":3,"data":{"turn":1,"step":1}}"#,
        ];
        let msgs = parse_rows(rows).unwrap();
        assert_eq!(msgs.len(), 1);
        assert!(matches!(
            &msgs[0].content_parts[0],
            ContentPart::ToolUse { tool_name, tool_use_id, .. }
                if tool_name == "grep" && tool_use_id.as_deref() == Some("call_8")
        ));
    }

    #[test]
    fn compaction_replace_shadows_old_messages_and_places_summary() {
        // DSH compaction：`user/message` 带 `surfaceOp.replace` + `sourceEventSeqs`
        // （引用的旧 seq 被摘要取代）。brief 指定：2 条真实 user/assistant 轮次 +
        // compaction-replace 行 → 输出以摘要取代旧消息且顺序保持。
        let rows = vec![
            r#"{"type":"session","version":0,"id":"s1","cwd":"/tmp/x","createdAt":1700000000000}"#,
            r#"{"type":"user/message","seq":1,"time":1,"data":{"content":[{"type":"text","text":"old prompt"}],"source":{"kind":"user"}}}"#,
            r#"{"type":"assistant/message","seq":2,"time":2,"data":{"content":[{"type":"text","text":"old answer"}]}}"#,
            r#"{"type":"user/message","seq":3,"time":3,"surfaceOp":{"op":"replace","start":1,"end":2},"sourceEventSeqs":[1,2],"data":{"content":[{"type":"text","text":"[checkpoint] condensed history"}],"source":{"kind":"plugin","plugin":"compact","compactionId":"c-1"}}}"#,
            r#"{"type":"user/message","seq":4,"time":4,"data":{"content":[{"type":"text","text":"new prompt"}],"source":{"kind":"user"}}}"#,
        ];
        let msgs = parse_rows(rows).unwrap();
        assert_eq!(msgs.len(), 2, "旧消息被摘要取代、新消息保留");
        assert_eq!(msgs[0].role, "user");
        assert!(matches!(
            &msgs[0].content_parts[0],
            ContentPart::Text { text } if text == "[checkpoint] condensed history"
        ));
        assert!(matches!(
            &msgs[1].content_parts[0],
            ContentPart::Text { text } if text == "new prompt"
        ));
    }

    #[test]
    fn compaction_replace_with_log_only_shadowed_seqs_keeps_summary_at_end() {
        // `sourceEventSeqs` 引用的纯日志 seq（未产生消息）无消息可遮蔽：摘要保持末尾。
        let rows = vec![
            r#"{"type":"user/message","seq":1,"time":1,"data":{"content":[{"type":"text","text":"hi"}],"source":{"kind":"user"}}}"#,
            r#"{"type":"user/message","seq":2,"time":2,"surfaceOp":{"op":"replace","start":1,"end":1},"sourceEventSeqs":[99],"data":{"content":[{"type":"text","text":"[checkpoint]"}],"source":{"kind":"plugin","plugin":"compact"}}}"#,
        ];
        let msgs = parse_rows(rows).unwrap();
        assert_eq!(msgs.len(), 2);
        assert!(matches!(
            &msgs[1].content_parts[0],
            ContentPart::Text { text } if text == "[checkpoint]"
        ));
    }

    #[test]
    fn second_compaction_splices_correctly_after_first() {
        // 连续两次 compaction：第二次引用首个 checkpoint 的 seq。回归
        // `seq_to_messages`/`message_seqs` 拼接位移后失配的问题。
        let rows = vec![
            r#"{"type":"user/message","seq":1,"time":1,"data":{"content":[{"type":"text","text":"old prompt"}],"source":{"kind":"user"}}}"#,
            r#"{"type":"assistant/message","seq":2,"time":2,"data":{"content":[{"type":"text","text":"old answer"}]}}"#,
            r#"{"type":"user/message","seq":3,"time":3,"surfaceOp":{"op":"replace","start":1,"end":2},"sourceEventSeqs":[1,2],"data":{"content":[{"type":"text","text":"[checkpoint one]"}],"source":{"kind":"plugin","plugin":"compact"}}}"#,
            r#"{"type":"user/message","seq":4,"time":4,"data":{"content":[{"type":"text","text":"mid prompt"}],"source":{"kind":"user"}}}"#,
            r#"{"type":"assistant/message","seq":5,"time":5,"data":{"content":[{"type":"text","text":"mid answer"}]}}"#,
            r#"{"type":"user/message","seq":6,"time":6,"surfaceOp":{"op":"replace","start":4,"end":5},"sourceEventSeqs":[3,4,5],"data":{"content":[{"type":"text","text":"[checkpoint two]"}],"source":{"kind":"plugin","plugin":"compact"}}}"#,
            r#"{"type":"user/message","seq":7,"time":7,"data":{"content":[{"type":"text","text":"new prompt"}],"source":{"kind":"user"}}}"#,
        ];
        let msgs = parse_rows(rows).unwrap();
        assert_eq!(msgs.len(), 2, "checkpoint two 必须遮蔽 checkpoint one 与中间轮次");
        assert!(matches!(
            &msgs[0].content_parts[0],
            ContentPart::Text { text } if text == "[checkpoint two]"
        ));
        assert!(matches!(
            &msgs[1].content_parts[0],
            ContentPart::Text { text } if text == "new prompt"
        ));
    }

    #[test]
    fn subagent_descriptor_session_parses_content_fully() {
        // descriptor 标记的子代理会话内容完整解析（user/assistant/tool_use/tool_result/image）
        // 均保留，不再整段跳过——子会话经父会话 chip 导航打开时必须可读。
        // label 是父级选择的委托名（展示名/标题源）。
        let descriptor = serde_json::from_str::<Value>(
            r#"{"type":"subagent/descriptor","seq":0,"time":1,"data":{"label":"deep review","mode":"one-shot","provider":"spawn"}}"#,
        )
        .unwrap();
        let mut state = ParseState::default();
        state.current_watermark = 0;
        dispatch_row(&descriptor, &mut state);
        assert_eq!(state.descriptor_label.as_deref(), Some("deep review"));

        let rows = vec![
            r#"{"type":"session","version":0,"id":"s1","cwd":"/tmp/x","createdAt":1700000000000}"#,
            r#"{"type":"subagent/descriptor","seq":1,"time":1,"data":{"label":"deep review","mode":"one-shot"}}"#,
            r#"{"type":"user/message","seq":2,"time":2,"data":{"content":[{"type":"text","text":"delegated task"}],"source":{"kind":"user"}}}"#,
            r#"{"type":"assistant/message","seq":3,"time":3,"data":{"content":[{"type":"text","text":"subagent work"}]}}"#,
            r#"{"type":"tool/call","seq":4,"time":4,"data":{"callId":"call_1","name":"bash","arguments":"{}"}}"#,
            r#"{"type":"tool/result","seq":5,"time":5,"data":{"message":{"source":{"kind":"tool","callId":"call_1"},"content":[{"type":"tool-result","toolCallId":"call_1","content":[{"type":"text","text":"out"}],"isError":false}]}}}"#,
        ];
        let msgs = parse_rows(rows).unwrap();
        assert_eq!(msgs.len(), 4, "子代理会话整段内容保留（user/assistant/tool 均出现）");
        assert_eq!(msgs[0].role, "user");
        assert_eq!(msgs[1].role, "assistant");
    }

    #[test]
    fn subagent_descriptor_session_flushes_interrupted_chunks() {
        // descriptor 会话 + 流式 chunk 无 assistant/message：step/end 正常 flush
        // 临时消息（子会话内容不再隐藏）。
        let rows = vec![
            r#"{"type":"session","version":0,"id":"s1","cwd":"/tmp/x","createdAt":1700000000000}"#,
            r#"{"type":"subagent/descriptor","seq":1,"time":1,"data":{"label":"deep review","mode":"one-shot"}}"#,
            r#"{"type":"assistant/chunk","seq":2,"data":{"turn":1,"step":1,"chunk":{"type":"text-delta","index":0,"text":"subagent "}}}"#,
            r#"{"type":"assistant/chunk","seq":3,"data":{"turn":1,"step":1,"chunk":{"type":"text-delta","index":0,"text":"work"}}}"#,
            r#"{"type":"step/end","seq":4,"data":{"turn":1,"step":1}}"#,
        ];
        let msgs = parse_rows(rows).unwrap();
        assert_eq!(msgs.len(), 1);
        assert!(matches!(
            &msgs[0].content_parts[0],
            ContentPart::Text { text } if text == "subagent work"
        ));
    }

    #[test]
    fn subagent_descriptor_session_assembled_message_supersedes_chunks() {
        // descriptor 会话：chunk 后 assistant/message 正常装配并取代流式增量，
        // step/end 不再重复 flush。
        let rows = vec![
            r#"{"type":"session","version":0,"id":"s1","cwd":"/tmp/x","createdAt":1700000000000}"#,
            r#"{"type":"subagent/descriptor","seq":1,"time":1,"data":{"label":"deep review","mode":"one-shot"}}"#,
            r#"{"type":"assistant/chunk","seq":2,"data":{"turn":1,"step":1,"chunk":{"type":"text-delta","index":0,"text":"subagent "}}}"#,
            r#"{"type":"assistant/message","seq":3,"data":{"turn":1,"step":1,"message":{"content":[{"type":"text","text":"subagent work"}]}}}"#,
            r#"{"type":"step/end","seq":4,"data":{"turn":1,"step":1}}"#,
        ];
        let msgs = parse_rows(rows).unwrap();
        assert_eq!(msgs.len(), 1, "装配消息保留、无重复 flush");
        assert!(matches!(
            &msgs[0].content_parts[0],
            ContentPart::Text { text } if text == "subagent work"
        ));
    }

    #[test]
    fn started_subagent_result_builds_subagent_map() {
        // 父会话 `subagent` 工具调用的结果行 "started subagent <id>" 建立
        // tool_use_id → 子会话文件映射（子会话为同项目兄弟目录，zstd 优先）；
        // label 取调用 arguments 的 description。
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("--tmp-proj--");
        let parent_dir = project.join("session-parent");
        let child_dir = project.join("2d8f7e50-74dc-49a5-8583-cf61a8fb8f75");
        std::fs::create_dir_all(&parent_dir).unwrap();
        std::fs::create_dir_all(&child_dir).unwrap();
        let child_file = child_dir.join("session.jsonl.zstd");
        std::fs::write(&child_file, b"placeholder").unwrap();
        let parent_file = parent_dir.join("session.jsonl");
        std::fs::write(
            &parent_file,
            concat!(
                "{\"type\":\"session\",\"version\":0,\"id\":\"session-parent\",\"cwd\":\"/tmp/proj\",\"createdAt\":1700000000000}\n",
                "{\"type\":\"assistant/message\",\"seq\":1,\"time\":1,\"data\":{\"message\":{\"content\":[{\"type\":\"tool-call\",\"id\":\"toolu_abc\",\"name\":\"subagent\",\"arguments\":\"{\\\"description\\\":\\\"Review standards compliance\\\",\\\"prompt\\\":\\\"…\\\"}\"}]}}}\n",
                "{\"type\":\"tool/result\",\"seq\":2,\"time\":2,\"data\":{\"message\":{\"source\":{\"kind\":\"tool\",\"callId\":\"toolu_abc\"},\"content\":[{\"type\":\"tool-result\",\"toolCallId\":\"toolu_abc\",\"content\":[{\"type\":\"text\",\"text\":\"started subagent 2d8f7e50-74dc-49a5-8583-cf61a8fb8f75\"}],\"isError\":false}]}}}\n",
            ),
        )
        .unwrap();

        let result = parse_dsh_session_file_with_offset(parent_file.to_str().unwrap()).unwrap();
        let info = result
            .subagent_map
            .get("toolu_abc")
            .expect("tool_use_id 应映射到子会话");
        assert_eq!(
            info.file_path,
            child_file.to_string_lossy(),
            "子会话文件为兄弟目录下的 zstd 副本"
        );
        assert_eq!(info.label, "Review standards compliance");
    }

    #[test]
    fn started_subagent_result_without_child_file_is_skipped() {
        // 结果行提到子会话 id 但磁盘无对应文件：不建映射（与 Claude 的 exists() 检查一致）。
        let dir = tempfile::tempdir().unwrap();
        let parent_dir = dir.path().join("--tmp-proj--").join("session-parent");
        std::fs::create_dir_all(&parent_dir).unwrap();
        let parent_file = parent_dir.join("session.jsonl");
        std::fs::write(
            &parent_file,
            concat!(
                "{\"type\":\"session\",\"version\":0,\"id\":\"session-parent\",\"cwd\":\"/tmp/proj\",\"createdAt\":1700000000000}\n",
                "{\"type\":\"tool/result\",\"seq\":1,\"time\":1,\"data\":{\"message\":{\"source\":{\"kind\":\"tool\",\"callId\":\"toolu_abc\"},\"content\":[{\"type\":\"tool-result\",\"toolCallId\":\"toolu_abc\",\"content\":[{\"type\":\"text\",\"text\":\"started subagent deadbeef-dead\"}],\"isError\":false}]}}}\n",
            ),
        )
        .unwrap();

        let result = parse_dsh_session_file_with_offset(parent_file.to_str().unwrap()).unwrap();
        assert!(result.subagent_map.is_empty());
        // 普通 tool/result 消息本身不受影响
        assert_eq!(result.messages.len(), 1);
    }

    #[test]
    fn maps_assistant_usage_to_token_usage() {
        // `data.usage`（inputTokens/outputTokens/cacheReadTokens/cacheWriteTokens）
        // 映射为 `TokenUsage`；cacheWriteTokens 是写入侧 → cache_creation_input_tokens。
        let rows = vec![
            r#"{"type":"assistant/message","seq":1,"time":1700000000001,"data":{"message":{"role":"assistant","content":[{"type":"text","text":"hello"}],"source":{"kind":"model","model":"deepseek-v4-flash"}},"usage":{"inputTokens":100,"outputTokens":25,"cacheReadTokens":50,"cacheWriteTokens":10}}}"#,
        ];
        let msgs = parse_rows(rows).unwrap();
        assert_eq!(msgs.len(), 1);
        let usage = msgs[0].token_usage.as_ref().expect("usage 挂到助手消息");
        assert_eq!(usage.input_tokens, 100);
        assert_eq!(usage.output_tokens, 25);
        assert_eq!(usage.cache_read_input_tokens, 50);
        assert_eq!(
            usage.cache_creation_input_tokens, 10,
            "cacheWriteTokens → cache_creation_input_tokens"
        );
    }

    #[test]
    fn usage_chunk_attaches_at_flush() {
        // 中断流的 `type:"usage"` chunk 随 step/end flush 挂到临时助手消息。
        let rows = vec![
            r#"{"type":"user/message","seq":1,"time":1,"data":{"content":[{"type":"text","text":"hi"}]}}"#,
            r#"{"type":"assistant/chunk","seq":2,"data":{"turn":1,"step":1,"chunk":{"type":"text-delta","index":0,"text":"partial "}}}"#,
            r#"{"type":"assistant/chunk","seq":3,"data":{"turn":1,"step":1,"chunk":{"type":"text-delta","index":0,"text":"answer"}}}"#,
            r#"{"type":"assistant/chunk","seq":4,"data":{"turn":1,"step":1,"chunk":{"type":"usage","usage":{"inputTokens":100,"outputTokens":20,"cacheReadTokens":40}}}}"#,
            r#"{"type":"step/end","seq":5,"data":{"turn":1,"step":1}}"#,
        ];
        let msgs = parse_rows(rows).unwrap();
        assert_eq!(msgs.len(), 2);
        let usage = msgs[1].token_usage.as_ref().expect("flush 消息带 usage");
        assert_eq!(usage.input_tokens, 100);
        assert_eq!(usage.output_tokens, 20);
        assert_eq!(usage.cache_read_input_tokens, 40);
    }

    #[test]
    fn extract_usage_records_folds_whole_event_stream() {
        // 用量记录：每行带 usage 的 assistant/message 产出一条 UsageRecord；
        // 被 compaction 遮蔽的消息行仍在日志中，历史用量照常计入；
        // 无 usage 的行与零 usage 不计。
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        let content = concat!(
            "{\"type\":\"session\",\"version\":0,\"id\":\"s1\",\"cwd\":\"/proj/a\",\"createdAt\":1700000000000}\n",
            "{\"type\":\"user/message\",\"seq\":1,\"time\":1700000000000,\"data\":{\"content\":[{\"type\":\"text\",\"text\":\"hi\"}]}}\n",
            "{\"type\":\"assistant/message\",\"seq\":2,\"time\":1700000000001,\"data\":{\"turn\":1,\"step\":1,\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"a\"}],\"source\":{\"kind\":\"model\",\"model\":\"deepseek-v4-flash\"}},\"usage\":{\"inputTokens\":100,\"outputTokens\":25,\"cacheReadTokens\":50,\"cacheWriteTokens\":10}}}\n",
            // 无 usage 的 assistant/message：不产生记录
            "{\"type\":\"assistant/message\",\"seq\":3,\"time\":1700000000002,\"data\":{\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"b\"}]}}}\n",
            // 将被 compaction 遮蔽的旧消息：用量仍计入
            "{\"type\":\"assistant/message\",\"seq\":4,\"time\":1700000000003,\"data\":{\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"old\"}],\"source\":{\"kind\":\"model\",\"model\":\"deepseek-v4-flash\"}},\"usage\":{\"inputTokens\":300,\"outputTokens\":50}}}\n",
            "{\"type\":\"user/message\",\"seq\":5,\"time\":1700000000004,\"surfaceOp\":{\"op\":\"replace\",\"start\":4,\"end\":4},\"sourceEventSeqs\":[4],\"data\":{\"content\":[{\"type\":\"text\",\"text\":\"[checkpoint]\"}],\"source\":{\"kind\":\"plugin\",\"plugin\":\"compact\"}}}\n",
        );
        std::fs::write(&path, content).unwrap();

        let records = extract_usage_records(path.to_str().unwrap(), "/proj/a");
        assert_eq!(records.len(), 2, "带 usage 的行各产出一条（含被 compaction 遮蔽的）");
        assert_eq!(records[0].model, "deepseek-v4-flash");
        assert_eq!(records[0].input_tokens, 100);
        assert_eq!(records[0].cache_creation_tokens, 10);
        assert_eq!(records[0].cache_read_tokens, 50);
        assert_eq!(records[1].input_tokens, 300);
        assert!(records[0].date.starts_with("20"), "日期应从 time 派生");
        assert!(records.iter().all(|r| r.project == "/proj/a"));
    }

    /// 构造 zstd 会话夹具：tempdir 下 `s1/session.jsonl.zstd`，含头行 + 首条用户 + 助手。
    fn write_zstd_session_fixture() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s1").join("session.jsonl.zstd");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let content = concat!(
            "{\"type\":\"session\",\"version\":0,\"id\":\"sess-1\",\"createdAt\":1700000000000,\"cwd\":\"/proj/a\"}\n",
            "{\"type\":\"user/message\",\"seq\":1,\"time\":1700000000000,\"data\":{\"content\":[{\"type\":\"text\",\"text\":\"first user prompt\"}]}}\n",
            "{\"type\":\"assistant/message\",\"seq\":2,\"time\":1700000000001,\"data\":{\"message\":{\"content\":[{\"type\":\"reasoning\",\"text\":\"thinking...\"},{\"type\":\"text\",\"text\":\"assistant answer\"}]}}}\n",
        );
        let compressed = zstd::encode_all(std::io::Cursor::new(content.as_bytes()), 3).unwrap();
        std::fs::write(&path, compressed).unwrap();
        (dir, path)
    }

    #[test]
    fn read_first_user_message_returns_first_user_text() {
        let (_dir, path) = write_zstd_session_fixture();
        assert_eq!(
            read_first_user_message(path.to_str().unwrap()).as_deref(),
            Some("first user prompt")
        );
    }

    #[test]
    fn read_first_user_message_skips_system_injected_context() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        let content = concat!(
            "{\"type\":\"session\",\"version\":0,\"id\":\"s1\",\"createdAt\":1700000000000,\"cwd\":\"/proj\"}\n",
            "{\"type\":\"user/message\",\"seq\":1,\"data\":{\"content\":[{\"type\":\"text\",\"text\":\"AGENTS.md\"}],\"source\":{\"kind\":\"agent-instructions\"}}}\n",
            "{\"type\":\"user/message\",\"seq\":2,\"data\":{\"content\":[{\"type\":\"text\",\"text\":\"real prompt\"}],\"source\":{\"kind\":\"user\"}}}\n",
        );
        std::fs::write(&path, content).unwrap();
        assert_eq!(
            read_first_user_message(path.to_str().unwrap()).as_deref(),
            Some("real prompt")
        );
    }

    #[test]
    fn read_session_id_from_header_with_dir_fallback() {
        let (_dir, path) = write_zstd_session_fixture();
        assert_eq!(
            read_session_id(path.to_str().unwrap()).as_deref(),
            Some("sess-1")
        );

        // 头行缺 id：回退到会话目录名
        let dir2 = tempfile::tempdir().unwrap();
        let path2 = dir2.path().join("sess-dir").join("session.jsonl");
        std::fs::create_dir_all(path2.parent().unwrap()).unwrap();
        std::fs::write(
            &path2,
            "{\"type\":\"session\",\"version\":0,\"createdAt\":1700000000000,\"cwd\":\"/proj\"}\n",
        )
        .unwrap();
        assert_eq!(
            read_session_id(path2.to_str().unwrap()).as_deref(),
            Some("sess-dir")
        );
    }

    #[test]
    fn read_project_path_from_header() {
        let (_dir, path) = write_zstd_session_fixture();
        assert_eq!(
            read_project_path(path.to_str().unwrap()).as_deref(),
            Some("/proj/a")
        );
    }

    #[test]
    fn scan_metadata_only_derives_from_header() {
        let (_dir, path) = write_zstd_session_fixture();
        let metadata = scan_session_metadata_only(&path).expect("metadata");
        assert_eq!(metadata.session_id, "sess-1");
        assert_eq!(metadata.project_path.as_deref(), Some("/proj/a"));
        // title 恒 None（不读尾部 session/title），展示名由 title_resolver
        // 回退到清洗后的首条用户消息
        assert_eq!(metadata.title, None);
        assert_eq!(
            metadata.first_user_message.as_deref(),
            Some("first user prompt")
        );
        assert!(metadata.first_timestamp.as_deref().unwrap_or("").starts_with("2023-11-"));
        assert_eq!(metadata.last_timestamp, metadata.first_timestamp);
    }

    #[test]
    fn scan_metadata_only_none_without_session_header() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        std::fs::write(
            &path,
            "{\"type\":\"user/message\",\"seq\":1,\"data\":{\"content\":[{\"type\":\"text\",\"text\":\"hi\"}]}}\n",
        )
        .unwrap();
        assert!(scan_session_metadata_only(&path).is_none());
    }

    #[test]
    fn search_docs_extract_text_from_parsed_messages() {
        let (_dir, path) = write_zstd_session_fixture();
        let docs = scan_search_docs_with_progress(&path, |_| {}).expect("docs");
        assert_eq!(docs.len(), 2);
        assert_eq!(docs[0].role, "user");
        assert_eq!(docs[0].message_index, 0);
        assert!(docs[0].search_text.contains("first user prompt"));
        assert_eq!(docs[1].role, "assistant");
        assert_eq!(docs[1].message_index, 1);
        assert!(docs[1].search_text.contains("assistant answer"));
        assert!(docs[1].search_text.contains("thinking..."));
    }

    #[test]
    fn search_docs_from_bytes_parses_archived_content() {
        let content = concat!(
            "{\"type\":\"session\",\"version\":0,\"id\":\"s1\",\"createdAt\":1700000000000,\"cwd\":\"/proj\"}\n",
            "{\"type\":\"user/message\",\"seq\":1,\"time\":1700000000000,\"data\":{\"content\":[{\"type\":\"text\",\"text\":\"archived needle\"}]}}\n",
            "{\"type\":\"assistant/message\",\"seq\":2,\"time\":1700000000001,\"data\":{\"content\":[{\"type\":\"text\",\"text\":\"archived answer\"}]}}\n",
        );
        let docs = scan_search_docs_from_bytes(content.as_bytes()).expect("docs");
        assert_eq!(docs.len(), 2);
        assert!(docs[0].search_text.contains("archived needle"));
        assert!(docs[1].search_text.contains("archived answer"));
    }

    #[test]
    fn generation_filename_accepts_only_canonical_generations() {
        // 代 0 保留最初命名，压缩副本同样视为代 0
        assert_eq!(parse_generation_log_filename("session.jsonl"), Some(0));
        assert_eq!(parse_generation_log_filename("session.jsonl.zstd"), Some(0));
        assert_eq!(parse_generation_log_filename("session.jsonl.zst"), Some(0));
        // 正规代次
        assert_eq!(parse_generation_log_filename("session.v1.jsonl"), Some(1));
        assert_eq!(parse_generation_log_filename("session.v3.jsonl"), Some(3));
        assert_eq!(parse_generation_log_filename("session.v3.jsonl.zstd"), Some(3));
        assert_eq!(parse_generation_log_filename("session.v12.jsonl"), Some(12));

        // 非规范名不得被当成代际候选：显式 v0、前导零、大写 V、额外分段、非日志文件
        assert_eq!(parse_generation_log_filename("session.v0.jsonl"), None);
        assert_eq!(parse_generation_log_filename("session.v01.jsonl"), None);
        assert_eq!(parse_generation_log_filename("session.V3.jsonl"), None);
        assert_eq!(parse_generation_log_filename("session.1.v3.jsonl"), None);
        assert_eq!(parse_generation_log_filename("session.v3.jsonl.tmp"), None);
        assert_eq!(parse_generation_log_filename("session.lock"), None);
        assert_eq!(parse_generation_log_filename("sessions.jsonl"), None);
        assert_eq!(parse_generation_log_filename("session.v.jsonl"), None);
        assert_eq!(parse_generation_log_filename("other.jsonl"), None);
    }

    #[test]
    fn session_dir_candidates_rank_newest_generation_then_zstd() {
        let dir = tempfile::tempdir().unwrap();
        for name in [
            "session.jsonl",
            "session.v2.jsonl",
            "session.v3.jsonl.zstd",
            "session.lock",
            "session.V9.jsonl",
        ] {
            std::fs::write(dir.path().join(name), b"x").unwrap();
        }

        let candidates = session_dir_candidates(dir.path());
        let names: Vec<String> = candidates
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        // 最高代在前；非规范名（大写 V）与 lock 文件不进入候选
        assert_eq!(
            names,
            vec!["session.v3.jsonl.zstd", "session.v2.jsonl", "session.jsonl"]
        );
    }

    #[test]
    fn session_dir_candidates_prefers_zstd_within_same_generation() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("session.jsonl"), b"plain").unwrap();
        std::fs::write(dir.path().join("session.jsonl.zstd"), b"zstd").unwrap();

        let candidates = session_dir_candidates(dir.path());
        assert_eq!(
            candidates[0].file_name().unwrap().to_string_lossy(),
            "session.jsonl.zstd"
        );
        assert_eq!(candidates.len(), 2);
    }
}
