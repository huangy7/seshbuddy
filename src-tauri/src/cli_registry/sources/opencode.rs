//! OpenCode 源。
//!
//! 这是本仓库第一个**库型源**：会话是 SQLite 库（`opencode.db`）里的行，不是文件。
//!
//! `discover` / `metadata_only` / `exists` 与整个读族（`parse*` / `search_docs*` /
//! `first_user_message` / `session_id` / `project_path_raw` / `is_subagent` / `usage_records`）
//! 都已接上库查询（只读连接）；列表侧 `scan` 全量查询 `session` 表并驱动共享索引机器，
//! `snapshot` 读同一份索引装配列表。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use serde_json::Value;

use crate::app_db;
use crate::cli::CliKind;
use crate::commands::session::{build_codex_search_results, AggregatedSessionSearch};
use crate::commands::session_index::{
    build_projects_snapshot, RawSession, ScanAccumulator, ScanProjection, SnapshotProjection,
};
use crate::error::AppResult;
use crate::parser::batch::BatchEmitter;
use crate::parser::claude_entry::TOOL_RESULT_MAX_LEN;
use crate::parser::shared::{SearchDocument, SearchScanProgress};
use crate::session::{
    ChatMessage, ContentPart, PaginatedProjects, SearchResult, SessionListMetadata,
    SessionLoadResult, SubagentInfo, TokenUsage, UsageRecord,
};
use crate::title_resolver::clean_fallback_user_text;

use super::super::descriptor::{CliDescriptor, FileRoot};
use super::super::features::{LaunchFeature, LaunchPlan, LaunchRequest};
use super::super::source::{CliSource, SessionLocator, Truncation};
use super::super::Discovered;

pub(crate) struct OpencodeSource;

static DESCRIPTOR: CliDescriptor = CliDescriptor {
    name: "OpenCode",
    tray_label: "OpenCode",
    command: "opencode",
    // 没有代理能力，用与其余不支持者相同的上报端口。
    proxy_port: crate::cli_registry::NO_PROXY_PORT,
    file_root: FileRoot {
        // 数据根仍是文件系统上的目录，只是会话不在其下的子目录里。
        data_dir_segments: &[".local", "share", "opencode"],
        // 会话在 SQLite 库里，没有「会话子目录」这个概念。
        sessions_subdir: None,
    },
};

/// OpenCode 的用量统计能力与实现同处一文件：能力位由「有无实现」对账，不必另立清单。
///
/// token 用量是 `session` 表的列，不需要解析消息体。
static OPENCODE_USAGE: OpencodeUsageStats = OpencodeUsageStats;
struct OpencodeUsageStats;
impl crate::cli_registry::features::UsageStatsFeature for OpencodeUsageStats {
    fn usage_records(&self, key: &str, project: &str) -> Vec<UsageRecord> {
        // 读不到库或行不存在时给空表：用量面板缺一段数据，好过整页报错。
        usage_records_for(key, project).unwrap_or_default()
    }
}

/// 库文件路径（数据根下的 `opencode.db`）。进度上报与只读打开共用同一处拼接。
fn db_path() -> AppResult<PathBuf> {
    Ok(crate::cli::data_dir(CliKind::Opencode)?.join("opencode.db"))
}

/// 只读打开 OpenCode 库。
///
/// 每次调用现开一次连接，不做连接池：库很小、每次只跑一两条查询，池化带来的收益
/// 抵不过它长期持有文件句柄、并与本应用主库（app.db）的连接管理相互纠缠的代价。
/// `READ_ONLY` 是硬约束 —— 本应用只读别人的库，绝不能写。
fn open_db() -> AppResult<Connection> {
    Ok(Connection::open_with_flags(db_path()?, OpenFlags::SQLITE_OPEN_READ_ONLY)?)
}

/// 从序列化定位符取出裸会话键（`cli://opencode/<id>` → `<id>`）。
///
/// 复用 `SessionLocator` 的解码，而不是在此另剥一遍前缀：前缀拼法由 `to_key()` 单点产出，
/// 这里再写一份迟早与它分叉，而路由/解码判错不会编译报错。
fn bare_key(key: &str) -> String {
    match SessionLocator::decode(CliKind::Opencode, key) {
        SessionLocator::Virtual { key, .. } => key,
        SessionLocator::File { path, .. } => path.to_string_lossy().into_owned(),
    }
}

/// 库里的时间戳是毫秒整数；展示层一律用 RFC3339 字符串。
fn ms_to_rfc3339(ms: i64) -> Option<String> {
    chrono::DateTime::from_timestamp_millis(ms).map(|dt| dt.to_rfc3339())
}

/// 一行 `session` 记录 → 列表元数据。
///
/// 批量扫描（`scan`）与单行查询（`metadata_only`）共用这一份列映射：两处各写一遍迟早分叉，
/// 而分叉只表现为列表与详情不一致，不会编译报错。
fn session_list_metadata(
    session_id: String,
    title: Option<String>,
    project_path: Option<String>,
    time_created: i64,
    time_updated: i64,
) -> SessionListMetadata {
    SessionListMetadata {
        session_id,
        project_path,
        title,
        first_user_message: None,
        first_timestamp: ms_to_rfc3339(time_created),
        last_timestamp: ms_to_rfc3339(time_updated),
        git_branch: String::new(),
        // 库型会话不是文件，库里没有「文件大小」这个对应物。给 0 表示「不适用」，
        // 不编一个看起来合理的字节数 —— 那会让下游把假数据当真。
        file_size: 0,
    }
}

/// 毫秒时间戳 → 本地时区 `YYYY-MM-DD`。用量按「用户所在时区的一天」归档，
/// 与其余 CLI 的取日口径一致；用 UTC 会把跨零点的会话记到前一天。
fn ms_to_local_date(ms: i64) -> Option<String> {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|dt| dt.with_timezone(&chrono::Local).format("%Y-%m-%d").to_string())
}

/// 会话跨度（`time_updated - time_created`，毫秒）。
///
/// 负值只可能来自时钟回拨或脏数据；超过 30 天则基本是异常时间戳而非真实会话。
/// 两种情况都给 `None`，不把一个明显不合理的数字喂给用量面板。
fn duration_between(created: i64, updated: i64) -> Option<u64> {
    const MAX_PLAUSIBLE_MS: i64 = 30 * 24 * 60 * 60 * 1000;
    let diff = updated.checked_sub(created)?;
    if diff <= 0 || diff > MAX_PLAUSIBLE_MS {
        return None;
    }
    Some(diff as u64)
}

/// 首字母大写：库里的工具名是小写（`bash`/`read`），而展示层与共享摘要逻辑都按
/// 大写名工作（`Bash`/`Read`）。不改名而直接使用，摘要会一律落到 `[<小写名>]` 兜底，
/// 工具名也与其余源的 `Bash`/`Read` 对不上。
fn capitalize_first(name: &str) -> String {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// 摘要回退路径把入参交给共享的 `tool_use_summary`，后者按其余源的键名取值
/// （`Read`/`Write`/`Edit` 取 `file_path`）；库里这几个工具用的是驼峰 `filePath`。
/// 不归一化时回退摘要只会得到 `[Edit: unknown]`，故补一份共享逻辑认得的键。
fn summary_input(input: &Value) -> Value {
    let Some(object) = input.as_object() else {
        return input.clone();
    };
    if object.contains_key("file_path") || !object.contains_key("filePath") {
        return input.clone();
    }
    let mut normalized = object.clone();
    if let Some(path) = normalized.remove("filePath") {
        normalized.insert("file_path".to_string(), path);
    }
    Value::Object(normalized)
}

/// `message.data` 里的 token 用量；没有 `tokens` 字段时给 `None`。
///
/// 库里把推理 token 单列（`tokens.reasoning`），而 `TokenUsage` 没有对应字段。
/// 推理 token 同样是模型产出的输出 token，丢掉会少算，故并入 `output_tokens`。
fn token_usage_from_message(data: &Value) -> Option<TokenUsage> {
    let tokens = data.get("tokens")?;
    // 键存在不代表有内容：库里可能显式写 `tokens: null`。只看键存在会把 null
    // 当成「零用量」，凭空造出一条全 0 的记录，而不是如实表示「没有用量」。
    if !tokens.is_object() {
        return None;
    }
    Some(TokenUsage {
        input_tokens: tokens.get("input").and_then(Value::as_u64).unwrap_or(0),
        output_tokens: tokens.get("output").and_then(Value::as_u64).unwrap_or(0)
            + tokens.get("reasoning").and_then(Value::as_u64).unwrap_or(0),
        cache_creation_input_tokens: tokens
            .pointer("/cache/write")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        cache_read_input_tokens: tokens
            .pointer("/cache/read")
            .and_then(Value::as_u64)
            .unwrap_or(0),
    })
}

/// 把一行 `message`（时间戳 + `data`）与其已按序映射好的 parts 装配成 `ChatMessage`。
///
/// 单独拆出来是为了能脱离数据库直接对 JSON 形状写单测：映射写错了只会渲染异常，
/// 不会编译报错。
fn chat_message_from_row(
    time_created: i64,
    data: &Value,
    content_parts: Vec<ContentPart>,
    session_model: Option<&str>,
) -> ChatMessage {
    let role = data
        .get("role")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    // 助手行把模型放在顶层 `modelID`，用户行放在嵌套的 `model.modelID`；两处都读，
    // 都缺席时才回落到会话级 `model`。
    let model = data
        .get("modelID")
        .or_else(|| data.pointer("/model/modelID"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| session_model.map(str::to_string));
    ChatMessage {
        role,
        timestamp: ms_to_rfc3339(time_created).unwrap_or_default(),
        model,
        token_usage: token_usage_from_message(data),
        content_parts,
        is_meta: false,
        // 库型源不支持「从此处分叉」，uuid 只服务于分叉锚点，故不填。
        uuid: None,
    }
}

/// 非空文本块才产出 `Text` part：空白块没有展示价值，还会让搜索文档多出空行。
fn push_nonempty_text(text: Option<&str>, out: &mut Vec<ContentPart>) {
    if let Some(text) = text {
        if !text.trim().is_empty() {
            out.push(ContentPart::Text { text: text.to_string() });
        }
    }
}

/// 一条 `tool` part 同时携带调用与结果，这里展开成 `ToolUse`（+ 有输出或错误文本时
/// 再补 `ToolResult`），与其它 CLI「调用、结果各成一条 part」的展示形态对齐。
fn push_tool_part(data: &Value, out: &mut Vec<ContentPart>) {
    let tool_name = capitalize_first(
        data.get("tool")
            .and_then(Value::as_str)
            .unwrap_or("tool"),
    );
    let state = data.get("state");
    let input = state
        .and_then(|s| s.get("input"))
        .cloned()
        .unwrap_or(Value::Null);
    // 调用与结果同处一条 part，配对 id 取同一个 `callID`，前端才能把结果归到对应调用下。
    let call_id = data.get("callID").and_then(Value::as_str).map(str::to_string);
    // `state.title` 是库里已生成的人话摘要，优先用它；缺席时才退回按工具名+入参拼摘要。
    let summary = state
        .and_then(|s| s.get("title"))
        .and_then(Value::as_str)
        .filter(|t| !t.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| crate::session::tool_use_summary(&tool_name, &summary_input(&input)));
    out.push(ContentPart::ToolUse {
        summary,
        tool_name: tool_name.clone(),
        input: serde_json::to_string_pretty(&input).unwrap_or_default(),
        tool_use_id: call_id.clone(),
    });

    let is_error = state
        .and_then(|s| s.get("status"))
        .and_then(Value::as_str)
        .map(|status| status == "error")
        .unwrap_or(false);
    // 正常结束的正文在 `output`；出错时库不写 `output`，错误文本单独放在 `error`。
    // 只读 `output` 会把错误正文整段丢掉（用户只剩一个没有结果的调用），故出错时改取 `error`。
    let content = state
        .and_then(|s| s.get("output"))
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty())
        .or_else(|| {
            if !is_error {
                return None;
            }
            state
                .and_then(|s| s.get("error"))
                .and_then(Value::as_str)
                .filter(|text| !text.trim().is_empty())
        });
    let Some(content) = content else {
        return;
    };
    let mut content = content.trim().to_string();
    let mut truncated = false;
    // 与其它解析器共用同一个上限：50 KB 是用户可见的截断行为，各写一份会各自漂移。
    if content.len() > TOOL_RESULT_MAX_LEN {
        let mut cut = TOOL_RESULT_MAX_LEN;
        while cut > 0 && !content.is_char_boundary(cut) {
            cut -= 1;
        }
        content.truncate(cut);
        truncated = true;
    }
    let summary = if is_error {
        format!("[{tool_name} error]")
    } else {
        format!("[{tool_name} result]")
    };
    out.push(ContentPart::ToolResult {
        summary,
        content,
        is_error,
        tool_use_id: call_id,
        full_len: 0,
        truncated_preview: false,
        truncated,
        source_offset: 0,
    });
}

/// 把一条 `part.data` 映射为展示用的 `ContentPart`（一条 part 可能展开成多个）。
fn push_part_content(data: &Value, out: &mut Vec<ContentPart>) {
    match data.get("type").and_then(Value::as_str).unwrap_or("unknown") {
        "text" => push_nonempty_text(data.get("text").and_then(Value::as_str), out),
        "reasoning" => {
            if let Some(text) = data.get("text").and_then(Value::as_str) {
                if !text.trim().is_empty() {
                    out.push(ContentPart::Thinking { thinking: text.to_string() });
                }
            }
        }
        "tool" => push_tool_part(data, out),
        // 步骤边界与用量快照：message 层已承载 role/时间/用量，这两个 part 没有可展示内容。
        // 它们不是「被丢弃的内容」，故不产占位 —— 否则每条助手消息都会多出两行噪声。
        "step-start" | "step-finish" => {}
        "patch" => {
            // `patch` 只记录被改动的文件与快照哈希，`ContentPart` 没有对应类型。
            // 显式占位而不是丢弃：丢弃会让用户以为这段改动不存在；列出文件名便于定位。
            let files: Vec<&str> = data
                .get("files")
                .and_then(Value::as_array)
                .map(|arr| arr.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();
            let text = if files.is_empty() {
                "[patch]".to_string()
            } else {
                format!("[patch: {}]", files.join(", "))
            };
            out.push(ContentPart::Text { text });
        }
        other => {
            // 未来新增的 part 类型：给显式占位，绝不静默丢弃。
            out.push(ContentPart::Text { text: format!("[{other} part]") });
        }
    }
}

/// 读取一条会话的全部消息（按时间序），每条消息带上其 parts。
///
/// 顺序即展示顺序：`parse` 与 `search_docs` 都以本函数的次序为准，两处不会各排一遍，
/// 前端从搜索命中跳到消息才不会错位。
fn load_session_messages(conn: &Connection, session_id: &str) -> AppResult<Vec<ChatMessage>> {
    let session_model: Option<String> = conn
        .query_row("SELECT model FROM session WHERE id = ?1", params![session_id], |row| {
            row.get(0)
        })
        .optional()?
        .flatten();

    let mut message_stmt = conn.prepare(
        "SELECT id, time_created, data FROM message WHERE session_id = ?1 ORDER BY time_created, id",
    )?;
    let rows = message_stmt
        .query_map(params![session_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    // part 的 id 前缀按写入时间递增，`ORDER BY id` 即插入顺序，且走 (message_id, id) 索引。
    let mut part_stmt = conn.prepare("SELECT data FROM part WHERE message_id = ?1 ORDER BY id")?;
    let mut messages = Vec::with_capacity(rows.len());
    for (message_id, time_created, raw_message) in rows {
        let data: Value = serde_json::from_str(&raw_message).unwrap_or(Value::Null);
        let mut parts = Vec::new();
        for raw_part in part_stmt.query_map(params![message_id], |row| row.get::<_, String>(0))? {
            let part_data: Value = serde_json::from_str(&raw_part?).unwrap_or(Value::Null);
            push_part_content(&part_data, &mut parts);
        }
        messages.push(chat_message_from_row(
            time_created,
            &data,
            parts,
            session_model.as_deref(),
        ));
    }
    Ok(messages)
}

/// 从已解析消息构建检索文档。索引直接取消息在 `parse` 结果里的下标，
/// 而不是「已产出文档数」：跳过的空文本消息仍占一个下标，两处枚举才逐位对齐。
fn search_docs_from_messages(messages: &[ChatMessage]) -> Vec<SearchDocument> {
    let mut docs = Vec::new();
    for (message_index, message) in messages.iter().enumerate() {
        let texts: Vec<&str> = message
            .content_parts
            .iter()
            .filter_map(|part| match part {
                ContentPart::Text { text } => Some(text.as_str()),
                ContentPart::Thinking { thinking } => Some(thinking.as_str()),
                _ => None,
            })
            .collect();
        let search_text = texts.join("\n");
        if search_text.trim().is_empty() {
            continue;
        }
        docs.push(SearchDocument {
            message_index,
            role: message.role.clone(),
            search_text,
        });
    }
    docs
}

/// 从 `session` 行读 token 列装配一条用量记录。
///
/// 直接读列而不解析消息：列是会话级聚合值，逐条累加消息里的用量既慢又可能与会话行对不上。
fn usage_records_for(key: &str, project: &str) -> Option<Vec<UsageRecord>> {
    let id = bare_key(key);
    let conn = open_db().ok()?;
    let row = conn
        .query_row(
            "SELECT model, time_created, time_updated, tokens_input, tokens_output, \
             tokens_reasoning, tokens_cache_read, tokens_cache_write \
             FROM session WHERE id = ?1",
            params![id],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, i64>(7)?,
                ))
            },
        )
        .optional()
        .ok()??;
    let (model, created, updated, input, output, reasoning, cache_read, cache_write) = row;
    Some(vec![UsageRecord {
        // 与其余源一致：时间戳越界时给 `unknown`，而不是空串（空串会被当成「无日期」而丢行）。
        date: ms_to_local_date(created).unwrap_or_else(|| "unknown".to_string()),
        model: model.unwrap_or_default(),
        // 负的计数只可能来自脏数据；钳到 0，避免 u64 转换回绕成一个天文数字。
        input_tokens: input.max(0) as u64,
        // 推理 token 并入 output：见 `token_usage_from_message` 的同一条取舍。
        output_tokens: (output.max(0) + reasoning.max(0)) as u64,
        cache_creation_tokens: cache_write.max(0) as u64,
        cache_read_tokens: cache_read.max(0) as u64,
        duration_ms: duration_between(created, updated),
        project: project.to_string(),
    }])
}

impl CliSource for OpencodeSource {
    fn descriptor(&self) -> &'static CliDescriptor {
        &DESCRIPTOR
    }

    /// 虚拟键（`cli://opencode/<id>`）归属本 CLI。前缀由 `SessionLocator::to_key()` 产出，
    /// 不另拼字面量 —— 两处各写一遍迟早分叉，而路由判错不会编译报错。
    fn owns(&self, key: &str) -> bool {
        let prefix = SessionLocator::Virtual { cli_id: CliKind::Opencode, key: String::new() }
            .to_key();
        key.starts_with(&prefix)
    }

    /// 库型会话的存活判定：在 `session` 表里查这一行。
    ///
    /// **失败方向是承重的，不可反转。** 本方法被启动期孤儿清理（`db::session_exists`）调用，
    /// 答「不存在」意味着那一行的索引会被物理删除。故三种结果分开：
    ///
    /// - 查到行 → `true`；
    /// - 查明确实没有这一行 → `false`（会话真的没了，该清）；
    /// - 库打不开、查询报错 → `true`（读不到 ≠ 里面没有）。
    ///
    /// 最后一条与文件型源「读不到子树就保留既有索引行」同源：把「库暂时不可读」判成
    /// 「不存在」会让整库索引在下次启动时被清空，代价不可逆；宁可漏删一行陈旧索引。
    fn exists(&self, loc: &SessionLocator) -> bool {
        let key = match loc {
            SessionLocator::Virtual { key, .. } => key,
            SessionLocator::File { .. } => {
                debug_assert!(false, "library source received a file locator: {loc:?}");
                return true;
            }
        };
        let conn = match open_db() {
            Ok(conn) => conn,
            Err(err) => {
                tracing::warn!("OpenCode 库打开失败，存活判定按「存在」处理: err={}", err);
                return true;
            }
        };
        match conn.query_row("SELECT 1 FROM session WHERE id = ?1", params![key], |_| Ok(())) {
            Ok(()) => true,
            Err(rusqlite::Error::QueryReturnedNoRows) => false,
            Err(err) => {
                tracing::warn!("OpenCode 存活查询失败，按「存在」处理: err={}", err);
                true
            }
        }
    }

    /// 库型源：会话是 `session` 表里的行，没有可移入回收站的文件。
    ///
    /// 删除要改库（并处理子会话、消息等关联表），本源尚未提供，故为 `false`。
    /// 命令层据此把删除请求判为失败而不是静默跳过 —— 跳过会让 UI 报成功、
    /// 库里的行仍在，且被墓碑隐藏到保留期满。
    fn can_delete(&self) -> bool {
        false
    }

    /// 枚举全部会话：`session` 表的每一行是一个虚拟定位符。
    ///
    /// 查询失败整轮冒泡（`?`），不存在「读到一半」的子树，故没有需要标记为未验证的前缀。
    fn discover(&self) -> AppResult<Discovered> {
        let conn = open_db()?;
        let mut stmt = conn.prepare("SELECT id FROM session")?;
        let locators = stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .map(|key| SessionLocator::Virtual { cli_id: CliKind::Opencode, key })
            .collect();
        Ok(Discovered { locators, unverified_prefixes: Vec::new() })
    }

    /// 读索引装配列表：与其它源同一套骨架，逐记录投影按虚拟键语义给出。
    ///
    /// 索引缺席（首次运行、或索引被清）时返回 `None`，由列表路径回退到 `scan` 重建；
    /// 这正是本源与文件型源共用「快照优先、扫描兜底」两条路径的地方。
    fn snapshot(
        &self,
        custom_names: &HashMap<String, String>,
        page: usize,
        page_size: usize,
    ) -> AppResult<Option<PaginatedProjects>> {
        build_projects_snapshot(
            CliKind::Opencode,
            custom_names,
            page,
            page_size,
            || {
                Ok(|record: &app_db::SessionListIndexRecord| {
                    // 项目路径缺席时退回会话 id 作分组键，与 `scan` 的兜底口径一致 ——
                    // 两条装配路径若分组键不同，同一批会话会随「快照还是扫描」改变分组。
                    let original_path = record
                        .project_path
                        .clone()
                        .filter(|path| !path.trim().is_empty())
                        .unwrap_or_else(|| record.session_id.clone());
                    SnapshotProjection {
                        encoded_dir: Some(original_path.clone()),
                        original_path: Some(original_path),
                        title: record.title.clone(),
                        history_display: None,
                        // 库里的会话行不记录分支，列表不展示分支。
                        git_branch: String::new(),
                    }
                })
            },
        )
    }

    /// 扫描 `session` 表中的**顶层会话**并驱动共享索引机器。
    ///
    /// 一次查询取回全部行再逐条收：库很小，N+1 的逐行查询不会更快，反而多一轮
    /// 「枚举」与「取元数据」之间的竞态窗口。持久化键是虚拟键（`cli://opencode/<id>`），
    /// 与 `owns` / `exists` / `metadata_only` 认的是同一个串，索引因此能被后续快照与
    /// 检索找回。
    ///
    /// 子会话（`parent_id` 非空）在查询里就被滤掉：快照路径会经 `is_subagent` 把子会话
    /// 挡在列表外，扫描若把整表写进索引，两条装配路径就会随「索引是否存在」给出不同的
    /// 可见集合，`total_sessions` 与 `has_more` 也会把子会话算进去。
    fn scan(
        &self,
        custom_names: &HashMap<String, String>,
        force: bool,
    ) -> AppResult<Vec<RawSession>> {
        let conn = open_db()?;
        // 与 `is_subagent` 同一判据：`parent_id` 非空即子会话，不进列表索引。
        let mut stmt = conn.prepare(
            "SELECT id, title, directory, time_created, time_updated FROM session \
             WHERE parent_id IS NULL",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        drop(stmt);
        drop(conn);

        let mut acc = ScanAccumulator::new(CliKind::Opencode, custom_names, force)?;
        for (id, title, directory, time_created, time_updated) in rows {
            let key = SessionLocator::Virtual { cli_id: CliKind::Opencode, key: id.clone() }.to_key();
            // 墓碑拦截与文件型源同源：用户在本应用里删掉的会话，库里那一行仍在，
            // 不拦就会在下一轮扫描里被重新写回列表，删除被静默撤销。
            if acc.is_tombstoned(&id, &key) {
                continue;
            }
            // 空目录名不是「一个空路径的项目」，是没有这个值。缺席时退回会话 id 作分组键，
            // 与 `snapshot` 的兜底口径一致 —— 否则同一批会话会随「快照还是扫描」改变分组。
            let project_path = directory
                .filter(|path| !path.trim().is_empty())
                .unwrap_or_else(|| id.clone());
            let metadata = session_list_metadata(
                id,
                title.clone(),
                Some(project_path.clone()),
                time_created,
                time_updated,
            );
            acc.push_session(
                &key,
                metadata,
                ScanProjection {
                    project_path: Some(project_path.clone()),
                    encoded_dir: Some(project_path.clone()),
                    original_path: Some(project_path),
                    title,
                    history_display: None,
                    git_branch: String::new(),
                },
            );
        }
        // 库查询失败整轮冒泡，不存在「读到一半」的子树，故没有需要标记为未验证的前缀。
        acc.finish(&[])
    }

    /// 解析整条会话：`message` 行按时间序，每行带上其 `part` 依次映射。
    fn parse(&self, key: &str) -> AppResult<Vec<ChatMessage>> {
        let id = bare_key(key);
        let conn = open_db()?;
        load_session_messages(&conn, &id)
    }

    /// 首条用户文本：与其它源同一套清洗与截断（首句、30 字 + 省略号）。
    fn first_user_message(&self, key: &str) -> Option<String> {
        let id = bare_key(key);
        let conn = open_db().ok()?;
        let messages = load_session_messages(&conn, &id).ok()?;
        for message in &messages {
            if message.role != "user" {
                continue;
            }
            for part in &message.content_parts {
                if let ContentPart::Text { text } = part {
                    if let Some(cleaned) = clean_fallback_user_text(text) {
                        let truncated: String = cleaned.chars().take(30).collect();
                        return Some(if cleaned.chars().count() > 30 {
                            format!("{}...", truncated)
                        } else {
                            truncated
                        });
                    }
                }
            }
        }
        None
    }

    /// 库型会话的 id 就是行主键；顺带确认行存在，不给已删会话报一个假 id。
    fn session_id(&self, key: &str) -> Option<String> {
        let id = bare_key(key);
        let conn = open_db().ok()?;
        conn.query_row("SELECT id FROM session WHERE id = ?1", params![id], |row| {
            row.get::<_, String>(0)
        })
        .ok()
    }

    /// 会话所属项目目录就是 `session.directory`。
    fn project_path_raw(&self, key: &str) -> Option<String> {
        let id = bare_key(key);
        let conn = open_db().ok()?;
        conn.query_row("SELECT directory FROM session WHERE id = ?1", params![id], |row| {
            row.get::<_, String>(0)
        })
        .ok()
    }

    /// 单行查询 `session` 行装配列表元数据。
    ///
    /// `key` 是序列化定位符，需先取出裸 id 再查；查不到行或库不可读时返回 `None`，
    /// 由调用方决定回退（列表侧另有索引兜底）。
    fn metadata_only(&self, key: &str) -> Option<SessionListMetadata> {
        let id = bare_key(key);
        let conn = open_db().ok()?;
        conn.query_row(
            "SELECT title, directory, time_created, time_updated, parent_id FROM session WHERE id = ?1",
            params![id],
            |row| {
                let title: Option<String> = row.get(0)?;
                let directory: Option<String> = row.get(1)?;
                let time_created: i64 = row.get(2)?;
                let time_updated: i64 = row.get(3)?;
                // parent_id 读出来但本结构没有承载它的字段：子代理判定走 `is_subagent`，
                // 此处不把它硬塞进任何展示字段。
                let _parent_id: Option<String> = row.get(4)?;
                Ok(session_list_metadata(
                    id.clone(),
                    title,
                    directory.filter(|path| !path.trim().is_empty()),
                    time_created,
                    time_updated,
                ))
            },
        )
        .ok()
    }

    /// 子会话在库里以 `parent_id` 指向父会话；非空即为子代理。
    fn is_subagent(&self, key: &str) -> bool {
        let id = bare_key(key);
        let Some(conn) = open_db().ok() else { return false };
        conn.query_row(
            "SELECT parent_id FROM session WHERE id = ?1",
            params![id],
            |row| row.get::<_, Option<String>>(0),
        )
        .ok()
        .flatten()
        .is_some()
    }

    fn parse_streaming(
        &self,
        key: &str,
        _skip_sidechain: bool,
        on_batch: &mut dyn FnMut(Vec<ChatMessage>) -> bool,
    ) -> AppResult<(u64, HashMap<String, SubagentInfo>)> {
        let messages = self.parse(key)?;
        // 库型源没有字节偏移；offset 用消息条数，`parse_incremental` 按同口径续读。
        let offset = messages.len() as u64;
        // 转成 `&mut F`（F = `&mut dyn FnMut`）再交给分批器：它要求 F: Sized，
        // 直接传 `&mut dyn FnMut` 会因 dyn 不定长而编译失败。
        let mut on_batch = on_batch;
        let mut emitter = BatchEmitter::new();
        for message in messages {
            emitter.push(message);
            if !emitter.maybe_flush(&mut on_batch) {
                break;
            }
        }
        emitter.flush_remaining(&mut on_batch);
        // 子代理是独立会话行，不是本会话内的旁链消息，故没有可映射的 subagent_map。
        Ok((offset, HashMap::new()))
    }

    fn parse_incremental(
        &self,
        key: &str,
        offset: u64,
        _skip_sidechain: bool,
    ) -> AppResult<SessionLoadResult> {
        let messages = self.parse(key)?;
        let total = messages.len() as u64;
        // offset 是「已发出的消息条数」，与 `parse_streaming` 的返回口径一致。
        let start = offset.min(total) as usize;
        let messages = messages.into_iter().skip(start).collect();
        Ok(SessionLoadResult { messages, offset: total, subagent_map: HashMap::new() })
    }

    /// 库型源的会话没有「文件内容」，从内存内容解析对它没有意义，返回空。
    fn parse_from_content(&self, _content: &str) -> Vec<ChatMessage> {
        Vec::new()
    }

    /// 检索文档与 `parse` 同源同序：都走 `load_session_messages`，索引即消息下标。
    fn search_docs(
        &self,
        key: &str,
        on_progress: &mut dyn FnMut(SearchScanProgress),
    ) -> Option<Vec<SearchDocument>> {
        let id = bare_key(key);
        let conn = open_db().ok()?;
        let messages = load_session_messages(&conn, &id).ok()?;
        // 库型源没有「已读字节」，用整库大小表示「一次读完」，让进度条直接到顶。
        let size = db_path()
            .ok()
            .and_then(|path| std::fs::metadata(path).ok())
            .map(|meta| meta.len())
            .unwrap_or(0);
        on_progress(SearchScanProgress { bytes_read: size, file_size: size });
        let docs = search_docs_from_messages(&messages);
        if docs.is_empty() {
            None
        } else {
            Some(docs)
        }
    }

    fn search_docs_from_bytes(&self, _content: &[u8]) -> Option<Vec<SearchDocument>> {
        None
    }

    fn new_session(&self) -> Option<&'static dyn LaunchFeature> {
        Some(&OPENCODE_LAUNCH)
    }

    fn resume_session(&self) -> Option<&'static dyn LaunchFeature> {
        Some(&OPENCODE_LAUNCH)
    }

    fn config_file(&self) -> Option<&'static dyn crate::cli_registry::features::ConfigFileFeature> {
        None
    }

    fn api_proxy(&self) -> Option<&'static dyn crate::cli_registry::features::ApiProxyFeature> {
        None
    }

    fn api_profile(&self) -> Option<&'static dyn crate::cli_registry::features::ApiProfileFeature> {
        None
    }

    fn fork(&self) -> Option<&'static dyn crate::cli_registry::features::ForkFeature> {
        None
    }

    fn usage_stats(&self) -> Option<&'static dyn crate::cli_registry::features::UsageStatsFeature> {
        Some(&OPENCODE_USAGE)
    }

    /// 库型源的会话变化不来自文件系统，没有需要监听的路径事件。
    fn is_session_event_path(&self, _path: &Path, _sessions_dir: &Path) -> bool {
        false
    }

    fn locate_executable(&self) -> Option<String> {
        crate::cli::find_on_path(self.descriptor().command)
    }

    /// **临时占位**：库型会话没有文件路径，本期返回空串；后续任务按库内 id 写实。
    fn tombstone_fallback_id(&self, _session_path: &Path) -> String {
        String::new()
    }

    /// **临时占位**：库型会话的 id 在库里而不在路径上，仅凭路径取不到，返回 `None`。
    fn path_only_session_id(&self, _session_path: &str) -> Option<String> {
        None
    }

    fn custom_titles(&self) -> HashMap<String, String> {
        HashMap::new()
    }

    /// 标题在 `session` 表里、索引已存，没有另一份索引文件要读。
    fn index_titles(&self) -> Option<HashMap<String, String>> {
        None
    }

    fn build_search_results(
        &self,
        aggregated: Vec<AggregatedSessionSearch>,
        records: &HashMap<String, app_db::SessionListIndexRecord>,
        custom_names: &HashMap<String, String>,
    ) -> AppResult<Vec<SearchResult>> {
        // 与 Codex 式共用装配路径，但库型源没有 history 映射与自定义标题表，喂空表。
        Ok(build_codex_search_results(
            aggregated,
            records,
            custom_names,
            &HashMap::new(),
            &HashMap::new(),
            CliKind::Opencode,
        ))
    }

    /// 源不在文件系统上，归档/恢复本期不适用，原样透传。
    fn decode_archived_source(&self, bytes: Vec<u8>, _truncation: Truncation) -> Vec<u8> {
        bytes
    }

    /// 源不在文件系统上，归档/恢复本期不适用，原样透传。
    fn encode_for_restore(&self, content: Vec<u8>, _session_path: &str) -> AppResult<Vec<u8>> {
        Ok(content)
    }

    fn build_temp_settings(&self) -> AppResult<Option<String>> {
        Ok(None)
    }

    fn needs_hook_relay(&self) -> bool {
        false
    }

    fn transcript_format(&self) -> Option<transcript_store::extract::CliFormat> {
        None
    }
}

/// OpenCode 的新建与恢复共用一份启动计划：都是命令行，差异只在恢复多一个 `-s <id>`。
///
/// 新建即「在项目目录里启动」：`open_in_terminal` 已经把终端开在 `project_path`，
/// 所以新建**不需要任何参数**，也不必往库里插行 —— 会话由 OpenCode 自己创建。
///
/// `skip_permissions` 与 `settings_file` 在本实现里被有意忽略：OpenCode 没有等价的
/// 命令行开关，也没有本应用代管的配置文件；为它们编参数只会拼出 CLI 不认的调用。
struct OpencodeLaunch;

static OPENCODE_LAUNCH: OpencodeLaunch = OpencodeLaunch;

impl LaunchFeature for OpencodeLaunch {
    fn plan(&self, req: &LaunchRequest<'_>) -> AppResult<LaunchPlan> {
        let mut args = Vec::new();
        if let Some(id) = req.session_id {
            // 恢复走顶层 `-s/--session`。`opencode session` 子命令只有 list/delete，
            // 没有 resume —— 走子命令会拼出一个 CLI 不认识的调用。
            args.push("-s".to_string());
            args.push(id.to_string());
        }
        Ok(LaunchPlan::CommandLine { args })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 虚拟键必须路由回 OpenCode 自己。
    ///
    /// 认不出就会被 `kind_for_path` 静默判给 `FALLBACK`（Claude）—— 解析、搜索、快照
    /// 会全部走错源，而且**不会编译报错**。这是本期唯一一处「错了也全绿」的地方。
    #[test]
    fn virtual_opencode_key_routes_to_opencode() {
        let key = SessionLocator::Virtual {
            cli_id: CliKind::Opencode,
            key: "ses_x".to_string(),
        }
        .to_key();
        assert_eq!(
            crate::cli_registry::kind_for_path(&key),
            CliKind::Opencode,
            "虚拟键 {key} 没有路由回 OpenCode"
        );
    }

    /// 只读冒烟：库存在时，`discover` 的条数必须等于库里的会话行数。
    ///
    /// 库不存在就跳过 —— 不能假设 CI 上装了 OpenCode。全程 `READ_ONLY`，不碰用户数据。
    ///
    /// 与改 OpenCode 数据目录覆盖的清理测试共用一把锁：覆盖是进程级共享状态，若在
    /// 「读期望条数」与 `discover` 内部「重新解析目录」之间被翻转，本条就会拿一个目录的
    /// 条数去比另一个目录，静默通过或无故失败。
    #[test]
    fn discover_matches_real_db_when_present() {
        let _guard = crate::cli::OPENCODE_DATA_DIR_OVERRIDE_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        let Ok(dir) = crate::cli::data_dir(CliKind::Opencode) else { return };
        let db_path = dir.join("opencode.db");
        if !db_path.exists() {
            return;
        }
        let expected: i64 = {
            let conn = Connection::open_with_flags(&db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
                .expect("只读打开真实 OpenCode 库");
            conn.query_row("SELECT count(*) FROM session", [], |row| row.get(0))
                .expect("统计 session 行数")
        };
        let discovered = OpencodeSource.discover().expect("discover 查询库");
        assert_eq!(
            discovered.locators.len() as i64,
            expected,
            "discover 条数与库中会话行数不符"
        );
        for loc in &discovered.locators {
            assert_eq!(loc.cli_id(), CliKind::Opencode, "发现了他源定位符");
        }
    }

    fn message_data(role: &str, tokens: Value) -> Value {
        serde_json::json!({ "role": role, "tokens": tokens })
    }

    /// 文本与推理 part 分别映射为 `Text` / `Thinking`；步骤边界 part 不产展示内容。
    #[test]
    fn text_reasoning_and_step_parts_map_to_expected_content() {
        let mut parts = Vec::new();
        push_part_content(&serde_json::json!({"type": "step-start"}), &mut parts);
        push_part_content(&serde_json::json!({"type": "text", "text": "你好"}), &mut parts);
        push_part_content(
            &serde_json::json!({"type": "reasoning", "text": "想一下"}),
            &mut parts,
        );
        push_part_content(&serde_json::json!({"type": "step-finish"}), &mut parts);

        assert_eq!(parts.len(), 2, "步骤边界 part 不应产生展示内容");
        assert!(matches!(&parts[0], ContentPart::Text { text } if text == "你好"));
        assert!(matches!(&parts[1], ContentPart::Thinking { thinking } if thinking == "想一下"));
    }

    /// 一条 `tool` part 展开成调用与结果两条 part，结果携带工具名与错误标记。
    #[test]
    fn tool_part_expands_into_use_and_result() {
        let mut parts = Vec::new();
        push_part_content(
            &serde_json::json!({
                "type": "tool",
                "callID": "call_1",
                "tool": "bash",
                "state": {
                    "status": "completed",
                    "input": {"command": "ls"},
                    "output": "a\nb",
                    "title": "List files"
                }
            }),
            &mut parts,
        );

        assert_eq!(parts.len(), 2);
        assert!(matches!(
            &parts[0],
            ContentPart::ToolUse { summary, tool_name, tool_use_id, .. }
                if summary == "List files" && tool_name == "Bash"
                    && tool_use_id.as_deref() == Some("call_1")
        ));
        assert!(matches!(
            &parts[1],
            ContentPart::ToolResult { content, is_error, tool_use_id, .. }
                if content == "a\nb" && !is_error
                    && tool_use_id.as_deref() == Some("call_1")
        ));
    }

    /// 出错时库里不写 `output`，只有 `error` 文本；结果必须照样产出，否则错误正文整段消失。
    ///
    /// 形状取自真实库里的 `webfetch` 失败 part：`status: "error"`、无 `output` 键。
    #[test]
    fn error_tool_part_emits_error_result_from_error_text() {
        let mut parts = Vec::new();
        push_part_content(
            &serde_json::json!({
                "type": "tool",
                "tool": "webfetch",
                "callID": "call_err_1",
                "state": {
                    "status": "error",
                    "input": {"format": "markdown", "url": "https://example.com"},
                    "error": "Request failed with status code: 403"
                }
            }),
            &mut parts,
        );

        assert_eq!(parts.len(), 2, "错误 part 也必须同时给出调用与结果");
        assert!(matches!(
            &parts[0],
            ContentPart::ToolUse { tool_name, tool_use_id, .. }
                if tool_name == "Webfetch" && tool_use_id.as_deref() == Some("call_err_1")
        ));
        assert!(matches!(
            &parts[1],
            ContentPart::ToolResult { content, is_error, tool_use_id, .. }
                if content == "Request failed with status code: 403"
                    && *is_error
                    && tool_use_id.as_deref() == Some("call_err_1")
        ));
    }

    /// `state.title` 缺席时回退到共享摘要逻辑；库里的 `filePath` 要能被认成 `file_path`。
    #[test]
    fn tool_summary_fallback_reads_camel_case_file_path() {
        let mut parts = Vec::new();
        push_part_content(
            &serde_json::json!({
                "type": "tool",
                "tool": "edit",
                "callID": "call_edit_1",
                "state": {
                    "status": "completed",
                    "input": {"filePath": "/tmp/dir/target.rs"},
                    "output": "ok"
                }
            }),
            &mut parts,
        );

        assert!(matches!(
            &parts[0],
            ContentPart::ToolUse { summary, tool_name, .. }
                if summary == "[Edit: target.rs]" && tool_name == "Edit"
        ));
    }

    /// 映射不上的 part 类型必须留下显式占位，不能静默消失。
    #[test]
    fn unmappable_parts_leave_explicit_placeholders() {
        let mut parts = Vec::new();
        push_part_content(
            &serde_json::json!({"type": "patch", "files": ["/a/b.rs"]}),
            &mut parts,
        );
        push_part_content(&serde_json::json!({"type": "future-thing"}), &mut parts);

        assert!(matches!(
            &parts[0],
            ContentPart::Text { text } if text == "[patch: /a/b.rs]"
        ));
        assert!(matches!(
            &parts[1],
            ContentPart::Text { text } if text == "[future-thing part]"
        ));
    }

    /// 推理 token 并入输出；message 缺 `modelID` 时回落到会话级 model。
    #[test]
    fn token_usage_folds_reasoning_and_model_falls_back() {
        let data = message_data(
            "assistant",
            serde_json::json!({
                "input": 10, "output": 3, "reasoning": 7,
                "cache": {"read": 5, "write": 2}
            }),
        );
        let message = chat_message_from_row(1_700_000_000_000, &data, Vec::new(), Some("fallback-model"));
        let usage = message.token_usage.expect("有 tokens 就应有用量");
        assert_eq!(usage.input_tokens, 10);
        assert_eq!(usage.output_tokens, 10, "output(3) + reasoning(7)");
        assert_eq!(usage.cache_read_input_tokens, 5);
        assert_eq!(usage.cache_creation_input_tokens, 2);
        assert_eq!(message.model.as_deref(), Some("fallback-model"));

        let data = serde_json::json!({"role": "user", "modelID": "big-pickle"});
        let message = chat_message_from_row(1_700_000_000_000, &data, Vec::new(), Some("fallback-model"));
        assert_eq!(message.model.as_deref(), Some("big-pickle"));
        assert!(message.token_usage.is_none());

        // `tokens` 键存在但值为 `null`：没有用量，不能造一条全 0 的假记录。
        let data = message_data("assistant", Value::Null);
        let message = chat_message_from_row(1_700_000_000_000, &data, Vec::new(), None);
        assert!(message.token_usage.is_none(), "tokens: null 应给 None");

        // 用户行的模型嵌在 `model.modelID` 下，也要读到。
        let data = serde_json::json!({"role": "user", "model": {"modelID": "nested-model"}});
        let message = chat_message_from_row(1_700_000_000_000, &data, Vec::new(), Some("fallback-model"));
        assert_eq!(message.model.as_deref(), Some("nested-model"));
    }

    /// 负跨度与超过 30 天的异常跨度都给 `None`；正常跨度取毫秒差。
    #[test]
    fn duration_rejects_negative_and_absurd_values() {
        assert_eq!(duration_between(1_000, 3_000), Some(2_000));
        assert_eq!(duration_between(3_000, 1_000), None);
        assert_eq!(duration_between(1_000, 1_000), None);
        let thirty_one_days = 31 * 24 * 60 * 60 * 1000;
        assert_eq!(duration_between(0, thirty_one_days), None);
    }

    /// 检索文档的索引必须取消息下标：被跳过的空文本消息仍占一个下标。
    #[test]
    fn search_doc_indexes_stay_aligned_when_messages_are_skipped() {
        let user = chat_message_from_row(
            1,
            &message_data("user", Value::Null),
            vec![ContentPart::Text { text: "hello".into() }],
            None,
        );
        // 只有工具内容、没有可检索文本的消息。
        let tool_only = chat_message_from_row(
            2,
            &message_data("assistant", Value::Null),
            vec![ContentPart::ToolUse {
                summary: "[Bash]".into(),
                tool_name: "Bash".into(),
                input: "{}".into(),
                tool_use_id: None,
            }],
            None,
        );
        let assistant = chat_message_from_row(
            3,
            &message_data("assistant", Value::Null),
            vec![ContentPart::Text { text: "world".into() }],
            None,
        );

        let docs = search_docs_from_messages(&[user, tool_only, assistant]);
        assert_eq!(docs.len(), 2);
        assert_eq!(docs[0].message_index, 0);
        assert_eq!(docs[0].search_text, "hello");
        assert_eq!(docs[1].message_index, 2, "跳过的消息仍占一个下标");
        assert_eq!(docs[1].search_text, "world");
    }

    /// 真实库冒烟：解析条数、role、检索索引与用量列都要与库内容对得上。
    ///
    /// 库不存在就跳过 —— 不能假设 CI 上装了 OpenCode。全程 `READ_ONLY`，不碰用户数据。
    /// 与 `discover` 冒烟共用同一把覆盖锁，避免读到别的测试改过的数据目录。
    #[test]
    fn parse_and_usage_match_real_db_when_present() {
        let _guard = crate::cli::OPENCODE_DATA_DIR_OVERRIDE_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        let Ok(dir) = crate::cli::data_dir(CliKind::Opencode) else { return };
        let db_path = dir.join("opencode.db");
        if !db_path.exists() {
            return;
        }
        let conn = Connection::open_with_flags(&db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .expect("只读打开真实 OpenCode 库");
        // 挑一个确实带文本 part 的会话，保证检索文档非空。
        let session_id: String = conn
            .query_row(
                "SELECT session_id FROM part WHERE json_extract(data, '$.type') = 'text' \
                 GROUP BY session_id ORDER BY count(*) DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .expect("库里至少有一个带文本 part 的会话");
        let expected_messages: i64 = conn
            .query_row(
                "SELECT count(*) FROM message WHERE session_id = ?1",
                params![session_id],
                |row| row.get(0),
            )
            .expect("统计消息条数");
        let parent_id: Option<String> = conn
            .query_row(
                "SELECT parent_id FROM session WHERE id = ?1",
                params![session_id],
                |row| row.get(0),
            )
            .expect("读 parent_id");
        let directory: String = conn
            .query_row(
                "SELECT directory FROM session WHERE id = ?1",
                params![session_id],
                |row| row.get(0),
            )
            .expect("读 directory");
        drop(conn);

        let key = SessionLocator::Virtual { cli_id: CliKind::Opencode, key: session_id.clone() }
            .to_key();

        let parsed = OpencodeSource.parse(&key).expect("解析真实会话");
        assert_eq!(
            parsed.len() as i64,
            expected_messages,
            "解析出的消息条数应等于库里 message 行数"
        );
        assert!(!parsed.is_empty(), "选中的会话不该为空");
        for message in &parsed {
            assert!(!message.role.is_empty(), "每条消息都必须有 role");
            assert!(!message.timestamp.is_empty(), "每条消息都必须有时间戳");
        }

        // 检索索引必须落在 parse 结果的合法下标上，且与对应消息的 role 一致。
        let mut noop = |_: SearchScanProgress| {};
        let docs = OpencodeSource.search_docs(&key, &mut noop).expect("带文本的会话应有检索文档");
        assert!(!docs.is_empty());
        let mut previous = None;
        for doc in &docs {
            assert!(doc.message_index < parsed.len(), "检索索引越界");
            assert_eq!(
                doc.role, parsed[doc.message_index].role,
                "检索索引与 parse 次序错位"
            );
            if let Some(prev) = previous {
                assert!(doc.message_index > prev, "检索索引必须严格递增");
            }
            previous = Some(doc.message_index);
        }

        // 标识与路径直接来自 session 行；子代理判定与 parent_id 一致。
        assert_eq!(OpencodeSource.session_id(&key).as_deref(), Some(session_id.as_str()));
        assert_eq!(OpencodeSource.project_path_raw(&key).as_deref(), Some(directory.as_str()));
        assert_eq!(OpencodeSource.is_subagent(&key), parent_id.is_some());

        // 用量记录读列：会话行存在时恰好一条，日期非空。
        let records = usage_records_for(&key, "/proj").expect("会话行存在");
        assert_eq!(records.len(), 1);
        assert!(!records[0].date.is_empty(), "日期应由 time_created 生成");
        assert_eq!(records[0].project, "/proj");
    }

    /// 新建与恢复必须产出**不同**的命令行，且恢复必须带会话 id。
    ///
    /// 两者的差异只有 `-s <id>` 这一处 —— 写反了会让「恢复」变成一次全新的空会话，
    /// 而那是**能编译、能跑、只是接错会话**的错。
    #[test]
    fn launch_plan_distinguishes_new_from_resume() {
        use crate::cli::CliKind;
        use crate::cli_registry::features::{launch_plan_for, LaunchPlan};

        let new = launch_plan_for(CliKind::Opencode, None, false, None).expect("OpenCode 可新建");
        assert_eq!(new, LaunchPlan::CommandLine { args: vec![] }, "新建不应带参数");

        let resumed = launch_plan_for(CliKind::Opencode, Some("ses_abc"), false, None)
            .expect("OpenCode 可恢复");
        assert_eq!(
            resumed,
            LaunchPlan::CommandLine { args: vec!["-s".to_string(), "ses_abc".to_string()] },
            "恢复必须带 -s 与会话 id"
        );
    }
}
