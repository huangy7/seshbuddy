use crate::app_db;
use crate::cli::{self, CliKind};
use crate::error::{AppError, AppResult};
use crate::parser;
use crate::session::{PaginatedProjects, ProjectInfo, ProjectSessionChunkItem, SessionInfo};
use crate::streaming;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant, UNIX_EPOCH};
use tauri::{AppHandle, Emitter};

const SESSION_LIST_INDEX_UPDATED_EVENT: &str = "session-list-index-updated";
const SEARCH_INDEX_WRITE_BATCH_SIZE: usize = 20;
const SEARCH_INDEX_CONFIRM_SESSION_THRESHOLD: usize = 10;
const SEARCH_INDEX_CONFIRM_BYTES_THRESHOLD: u64 = 50 * 1024 * 1024;

static ONGOING_INDEX_BUILDS: LazyLock<Mutex<HashSet<CliKind>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

static ONGOING_LIST_INDEX_REFRESHES: LazyLock<Mutex<HashSet<CliKind>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

struct ListIndexRefreshGuard(CliKind);

impl Drop for ListIndexRefreshGuard {
    fn drop(&mut self) {
        if let Ok(mut ongoing) = ONGOING_LIST_INDEX_REFRESHES.lock() {
            ongoing.remove(&self.0);
        }
    }
}

/// 强制重建等待在飞刷新让出槽位的最长时间与轮询间隔。
const FORCE_REFRESH_SLOT_WAIT: Duration = Duration::from_secs(60);
const FORCE_REFRESH_SLOT_POLL: Duration = Duration::from_millis(100);

/// 尝试立即占住该 CLI 的列表索引刷新槽位；已被占用时返回 false。
fn try_acquire_list_index_refresh_slot(kind: CliKind) -> AppResult<bool> {
    let mut ongoing = ONGOING_LIST_INDEX_REFRESHES
        .lock()
        .map_err(|e| AppError::coded("session_index.refresh_lock_failed").with("detail", e.to_string()))?;
    if ongoing.contains(&kind) {
        return Ok(false);
    }
    ongoing.insert(kind);
    Ok(true)
}

/// 等待并占住槽位，供强制重建使用。
///
/// 强制请求（用户点「重建索引」）**不能**像后台刷新那样被跳过：调用方会先清空索引、
/// 再发起强制重建、最后补建检索索引 —— 一旦这里直接跳过，索引就停在清空后的空状态，
/// 后续补建也失去依据，用户看到的是「点了重建，会话列表空了」。
/// 因此改为等待在飞刷新让位后再执行；等待超时则明确报错，让调用方能向用户反馈，
/// 而不是静默返回一个被忽略的 false。后台刷新仍走「立即尝试、占用即跳过」，
/// 避免定时刷新与监听刷新堆叠排队。
async fn wait_for_list_index_refresh_slot(kind: CliKind) -> AppResult<()> {
    let deadline = Instant::now() + FORCE_REFRESH_SLOT_WAIT;
    loop {
        if try_acquire_list_index_refresh_slot(kind)? {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(AppError::coded("session_index.refreshing").with("cli", kind.name()));
        }
        tokio::time::sleep(FORCE_REFRESH_SLOT_POLL).await;
    }
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct SessionListIndexUpdatedPayload {
    cli_id: String,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchIndexStatus {
    pub(crate) ready: bool,
    requires_confirmation: bool,
    pub(crate) missing_sessions: usize,
    pub(crate) total_sessions: usize,
    missing_bytes: u64,
    total_bytes: u64,
    pub(crate) physical_index_ready: bool,
}

/// 扫描阶段的中间产物：尚未投影为展示项的一条会话。
///
/// `pub(crate)` 而非私有：各 CLI 源的 `scan` 出口以它为返回类型，可见性至少要覆盖
/// `cli_registry`；它仍是索引层的内部类型，字段不对外暴露，不随定位符一起挪走。
pub(crate) struct RawSession {
    session: SessionInfo,
    /// 分组键；`None` = 会话解析不出项目路径（见 `ProjectInfo::encoded_dir`）。
    encoded_dir: Option<String>,
    /// 项目路径；`None` 的语义与 `ProjectInfo::original_path` 相同。
    original_path: Option<String>,
}

pub(crate) fn file_modified_ms(metadata: &fs::Metadata) -> i64 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
        .unwrap_or_default()
}

pub(crate) fn cached_session_list_metadata(
    cached: &HashMap<String, app_db::SessionListIndexRecord>,
    file_path: &str,
    file_size: u64,
    modified_ms: i64,
) -> Option<parser::SessionListMetadata> {
    if parser::is_subagent_session(file_path) {
        return None;
    }
    let record = cached.get(file_path)?;
    if record.file_size != file_size || record.modified_ms != modified_ms {
        return None;
    }
    if record.session_id.trim().is_empty() {
        return None;
    }

    Some(parser::SessionListMetadata {
        session_id: record.session_id.clone(),
        project_path: record.project_path.clone(),
        title: record.title.clone(),
        first_user_message: record.first_user_message.clone(),
        first_timestamp: record.first_timestamp.clone(),
        last_timestamp: record.last_timestamp.clone(),
        git_branch: record.git_branch.clone(),
        file_size: record.file_size,
    })
}

fn build_session_list_index_record(
    file_path: &str,
    modified_ms: i64,
    metadata: &parser::SessionListMetadata,
) -> app_db::SessionListIndexRecord {
    app_db::SessionListIndexRecord {
        session_path: file_path.to_string(),
        session_id: metadata.session_id.clone(),
        project_path: metadata.project_path.clone(),
        title: metadata.title.clone(),
        first_user_message: metadata.first_user_message.clone(),
        first_timestamp: metadata.first_timestamp.clone(),
        last_timestamp: metadata.last_timestamp.clone(),
        git_branch: metadata.git_branch.clone(),
        file_size: metadata.file_size,
        modified_ms,
        has_archive_snapshot: false,
        is_archived: false,
    }
}

fn build_session_search_doc_batch(
    file_path: &str,
    modified_ms: i64,
    docs: &[parser::SearchDocument],
) -> app_db::SessionSearchDocBatch {
    app_db::SessionSearchDocBatch {
        session_path: file_path.to_string(),
        modified_ms,
        docs: docs
            .iter()
            .map(|doc| app_db::SessionSearchDocRecord {
                message_index: doc.message_index,
                search_text: doc.search_text.clone(),
            })
            .collect(),
    }
}

fn scan_session_search_docs_for_index<F>(
    kind: CliKind,
    path: &str,
    record: &app_db::SessionListIndexRecord,
    on_progress: F,
) -> Option<Vec<parser::SearchDocument>>
where
    F: FnMut(parser::SearchScanProgress),
{
    // 存在性判定交给源自己：库型会话的身份是虚拟键（`cli://opencode/...`），不是文件
    // 路径，`Path::exists()` 对它恒为 false，会让检索对库型源整体失效。文件型源的
    // `exists` 落回 `path.exists()`，判定与原先逐字节相同。
    let loc = crate::cli_registry::SessionLocator::decode(kind, path);
    if crate::cli_registry::source_for(kind).exists(&loc) {
        return parser::scan_session_search_docs_with_progress(Path::new(path), on_progress);
    }

    match app_db::read_archived_session_content(kind.id(), path) {
        Ok(Some(content)) => {
            let docs = parser::scan_session_search_docs_from_bytes(path, &content);
            if docs.is_some() {
                tracing::info!(
                    "搜索索引: 从归档内容构建索引, cli={}, path={}",
                    kind.id(),
                    path
                );
            }
            docs
        }
        Ok(None) => {
            tracing::info!(
                "搜索索引: 会话源文件和归档内容均不存在, cli={}, path={}, archived={}",
                kind.id(),
                path,
                record.is_archived
            );
            None
        }
        Err(err) => {
            tracing::warn!(
                "搜索索引: 读取归档内容失败, cli={}, path={}, err={}",
                kind.id(),
                path,
                err.diagnostic()
            );
            None
        }
    }
}

fn emit_search_index_progress(
    app: &AppHandle,
    kind: CliKind,
    phase: &str,
    current: usize,
    total: usize,
    processed_bytes: u64,
    total_bytes: u64,
    current_path: Option<&str>,
    current_file_bytes: u64,
    current_file_size: u64,
) {
    let _ = app.emit(
        "search-index-progress",
        serde_json::json!({
            "cliId": kind.id(),
            "phase": phase,
            "current": current,
            "total": total,
            "processedBytes": processed_bytes,
            "totalBytes": total_bytes,
            "currentPath": current_path,
            "currentFileBytes": current_file_bytes,
            "currentFileSize": current_file_size,
        }),
    );
}

/// 循环级索引进度事件节流门：同一相位 250ms 内最多放行一次，
/// 但相位切换与最后一个会话（终态帧）始终放行，保证前端不丢
/// "committed"、最终计数等关键状态。"written" 归入 "writing" 相位组，
/// 避免每个会话 scanning→writing→written 的微相位循环令节流失效。
struct IndexProgressEmitGate {
    last_emit: Instant,
    last_phase: Option<String>,
}

impl IndexProgressEmitGate {
    fn new() -> Self {
        Self {
            last_emit: Instant::now()
                .checked_sub(Duration::from_millis(250))
                .unwrap_or_else(Instant::now),
            last_phase: None,
        }
    }

    fn gate_phase(phase: &str) -> &str {
        match phase {
            "scanning" | "writing" | "written" => "processing",
            other => other,
        }
    }

    fn should_emit(&mut self, phase: &str, force: bool) -> bool {
        let now = Instant::now();
        let gated = Self::gate_phase(phase);
        let phase_changed = self.last_phase.as_deref() != Some(gated);
        if force
            || phase_changed
            || now.duration_since(self.last_emit) >= Duration::from_millis(250)
        {
            self.last_emit = now;
            self.last_phase = Some(gated.to_string());
            true
        } else {
            false
        }
    }
}

fn load_session_list_index(kind: CliKind) -> HashMap<String, app_db::SessionListIndexRecord> {
    let _ = app_db::restore_missing_archived_index_rows(kind);
    match app_db::read_session_list_index(kind) {
        Ok(index) => index,
        Err(err) => {
            tracing::warn!(
                "读取会话列表索引失败，回退实时扫描: cli={}, err={}",
                kind.id(),
                err.diagnostic()
            );
            HashMap::new()
        }
    }
}

fn load_session_list_index_page(
    kind: CliKind,
    page: usize,
    page_size: usize,
) -> AppResult<Option<(Vec<app_db::SessionListIndexRecord>, usize, bool)>> {
    let _ = app_db::restore_missing_archived_index_rows(kind);
    let total_sessions = app_db::count_session_list_index(kind)?;
    if total_sessions == 0 {
        return Ok(None);
    }

    let start = page.saturating_mul(page_size);
    let records = app_db::read_session_list_index_page(kind, start, page_size)?;
    let has_more = start + records.len() < total_sessions;
    Ok(Some((records, total_sessions, has_more)))
}

fn build_paginated_projects_from_sessions(
    selected_sessions: Vec<RawSession>,
    total_sessions: usize,
    has_more: bool,
) -> PaginatedProjects {
    let mut project_map_ordered: Vec<(Option<String>, Option<String>, Vec<SessionInfo>)> = Vec::new();
    let mut project_index: HashMap<Option<String>, usize> = HashMap::new();

    for raw in selected_sessions {
        if let Some(&idx) = project_index.get(&raw.encoded_dir) {
            project_map_ordered[idx].2.push(raw.session);
        } else {
            let idx = project_map_ordered.len();
            project_index.insert(raw.encoded_dir.clone(), idx);
            project_map_ordered.push((raw.encoded_dir, raw.original_path, vec![raw.session]));
        }
    }

    let projects = project_map_ordered
        .into_iter()
        .map(|(encoded_dir, original_path, sessions)| ProjectInfo {
            encoded_dir,
            original_path,
            sessions,
        })
        .collect();

    PaginatedProjects {
        projects,
        has_more,
        total_sessions,
    }
}

/// 一条索引记录投影成列表项所需的 CLI 专属字段。
///
/// 六种 CLI 的列表装配只在「逐记录投影」上分叉：循环骨架、分页、分组与展示名解析链
/// 完全一致。把分叉点收进这个结构，投影由各源自己给出，公共骨架因此只写一次 ——
/// 新增一个 CLI 不必再往扫描/快照层补一份近似的循环。
pub(crate) struct SnapshotProjection {
    /// 分组键；`None` = 会话解析不出项目路径（见 `ProjectInfo::encoded_dir`）。
    pub(crate) encoded_dir: Option<String>,
    /// 项目路径；`None` 的语义与 `ProjectInfo::original_path` 相同。
    pub(crate) original_path: Option<String>,
    /// `resolve_display_name` 的原生标题实参。索引外还有标题来源的 CLI 在这里补上，
    /// 其余 CLI 直接透传索引里的 `title`。
    pub(crate) title: Option<String>,
    /// `resolve_display_name` 的 history 兜底实参；没有 history 来源的 CLI 给 `None`。
    pub(crate) history_display: Option<String>,
    /// 会话所在分支；索引不记录分支的 CLI 给空串。
    pub(crate) git_branch: String,
}

/// 装配分页项目快照的公共骨架。
///
/// `setup` 在确认本页确有记录之后才执行：索引为空时直接返回 `Ok(None)`，不做任何
/// CLI 专属准备（例如解析 history 文件）—— 与逐条 CLI 早退的次序一致，避免
/// 「空索引 + 准备步骤报错」从 `Ok(None)` 变成 `Err`。
/// `project` 携带该 CLI 的逐记录差异；由 `setup` 返回，好让准备步骤里读出的映射
/// 被闭包按值捕获。
pub(crate) fn build_projects_snapshot<S, F>(
    kind: CliKind,
    custom_names: &HashMap<String, String>,
    page: usize,
    page_size: usize,
    setup: S,
) -> AppResult<Option<PaginatedProjects>>
where
    S: FnOnce() -> AppResult<F>,
    F: Fn(&app_db::SessionListIndexRecord) -> SnapshotProjection,
{
    let Some((records, total_sessions, has_more)) =
        load_session_list_index_page(kind, page, page_size)?
    else {
        return Ok(None);
    };

    let project = setup()?;
    let mut selected_sessions = Vec::with_capacity(records.len());

    for record in records {
        if parser::is_subagent_session(&record.session_path) {
            continue;
        }
        let projection = project(&record);
        let file_path = record.session_path;
        let session_id = record.session_id;
        let display_name = crate::title_resolver::resolve_display_name(
            super::session::custom_session_name(custom_names, kind, &file_path),
            projection.title.as_deref(),
            record.first_user_message.as_deref(),
            projection.history_display.as_deref(),
            &session_id,
        );
        let timestamp = record
            .last_timestamp
            .or(record.first_timestamp)
            .unwrap_or_default();

        selected_sessions.push(RawSession {
            session: SessionInfo {
                session_id,
                file_path,
                display_name,
                timestamp,
                file_size: record.file_size,
                git_branch: projection.git_branch,
                has_archive_snapshot: record.has_archive_snapshot,
                is_archived: record.is_archived,
                cli_id: kind.id().to_string(),
            },
            encoded_dir: projection.encoded_dir,
            original_path: projection.original_path,
        });
    }

    Ok(Some(build_paginated_projects_from_sessions(
        selected_sessions,
        total_sessions,
        has_more,
    )))
}

pub(crate) fn emit_session_list_index_updated(app: &tauri::AppHandle, kind: CliKind) {
    if let Err(err) = app.emit(
        SESSION_LIST_INDEX_UPDATED_EVENT,
        SessionListIndexUpdatedPayload {
            cli_id: kind.id().to_string(),
        },
    ) {
        tracing::warn!("发送会话索引刷新事件失败: cli={}, err={}", kind.id(), err);
    }
}

fn collect_missing_search_paths(
    kind: CliKind,
    list_index: &HashMap<String, app_db::SessionListIndexRecord>,
    search_index: &HashMap<String, app_db::SessionSearchIndexRecord>,
    physical_index_ready: bool,
) -> Vec<String> {
    let tombstone_filter = match app_db::conn() {
        Ok(c) => app_db::load_tombstone_filter_inner(&c, kind.id()).unwrap_or_default(),
        Err(_) => app_db::TombstoneFilter::default(),
    };
    let mut paths: Vec<String> = list_index
        .iter()
        .filter(|(path, record)| {
            // 双轨墓碑拦截：已删除会话不进入全文搜索待索引列表
            let session_id = if !record.session_id.is_empty() {
                record.session_id.clone()
            } else {
                crate::cli_registry::source_for(kind)
                    .path_only_session_id(path)
                    .unwrap_or_default()
            };
            let safe_session_id = if session_id == "transcript"
                || session_id == "session"
                || session_id.starts_with("session.jsonl")
            {
                ""
            } else {
                session_id.as_str()
            };
            if tombstone_filter.is_tombstoned(safe_session_id, path) {
                return false;
            }

            if !physical_index_ready {
                return true;
            }
            match search_index.get(*path) {
                // modified_ms 匹配时无论 doc_count 是否为 0 均视为已索引：
                // doc_count == 0 表示该会话无可搜索的文本内容（如纯工具调用会话），
                // 不应重复触发索引构建。
                Some(sr) => sr.modified_ms != record.modified_ms,
                None => true,
            }
        })
        .map(|(path, _)| path.clone())
        .collect();
    paths.sort();
    paths
}

pub(crate) fn resolve_search_index_status(kind: CliKind) -> AppResult<SearchIndexStatus> {
    let list_index = app_db::read_session_list_index(kind)?;
    let total_sessions = list_index.len();
    let total_bytes: u64 = list_index.values().map(|record| record.file_size).sum();
    let physical_index_ready = app_db::tantivy_search::is_physical_index_ready()?;
    let search_index = if physical_index_ready {
        app_db::read_session_search_index(kind)?
    } else {
        HashMap::new()
    };
    let missing_paths = collect_missing_search_paths(
        kind,
        &list_index,
        &search_index,
        physical_index_ready,
    );
    let missing_bytes: u64 = missing_paths
        .iter()
        .filter_map(|path| list_index.get(path).map(|record| record.file_size))
        .sum();
    let missing_sessions = missing_paths.len();
    let ready = missing_sessions == 0;
    let requires_confirmation = !ready
        && (!physical_index_ready
            || missing_sessions >= SEARCH_INDEX_CONFIRM_SESSION_THRESHOLD
            || missing_bytes >= SEARCH_INDEX_CONFIRM_BYTES_THRESHOLD);

    Ok(SearchIndexStatus {
        ready,
        requires_confirmation,
        missing_sessions,
        total_sessions,
        missing_bytes,
        total_bytes,
        physical_index_ready,
    })
}

/// 已知行中落在「本轮没能读取」的子树前缀下时，不该按"没扫到"推断删除：读不到 ≠ 里面没有。
fn is_under_unverified_prefix(path: &str, prefixes: &[PathBuf]) -> bool {
    prefixes
        .iter()
        .any(|prefix| Path::new(path).starts_with(prefix))
}

/// 小集合不做存活比例保护：删几条会话是日常操作，加守卫只会让正常清理失效。
const MASS_DELETION_GUARD_MIN_KNOWN: usize = 10;

/// Antigravity 会话正本的固定文件名。
pub(crate) const ANTIGRAVITY_TRANSCRIPT_FILE_NAME: &str = "transcript.jsonl";

/// 找出「日志目录里有 jsonl、却没有会话正本名」的目录，返回 (目录, 实际文件名)。
///
/// Antigravity 的会话转录目前固定叫 `transcript.jsonl`，同一日志目录下还并存
/// `transcript_full.jsonl` 这类兄弟文件：两者行数、step 区间、事件类型分布几乎完全一致，
/// **无法从内容判断谁才是会话正本**。因此不能把匹配放宽成"目录下任意 jsonl" —— 那会把每个
/// 会话重复索引一遍。既然猜不出正本，就把它变成显式信号：上游一旦改命名，这里当天就会告警，
/// 而不是表现为"会话静默从列表里消失"（这正是 dsh 换代时踩过的坑）。
pub(crate) fn antigravity_transcript_naming_drift(
    jsonl_files: &[PathBuf],
) -> Vec<(PathBuf, Vec<String>)> {
    let mut by_dir: HashMap<&Path, Vec<String>> = HashMap::new();
    for path in jsonl_files {
        let (Some(parent), Some(name)) = (
            path.parent(),
            path.file_name().and_then(|n| n.to_str()),
        ) else {
            continue;
        };
        by_dir.entry(parent).or_default().push(name.to_string());
    }

    let mut drifted: Vec<(PathBuf, Vec<String>)> = by_dir
        .into_iter()
        .filter(|(_, names)| {
            !names
                .iter()
                .any(|name| name == ANTIGRAVITY_TRANSCRIPT_FILE_NAME)
        })
        .map(|(dir, mut names)| {
            names.sort();
            (dir.to_path_buf(), names)
        })
        .collect();
    drifted.sort();
    drifted
}

fn collect_stale_paths(
    cached: &HashMap<String, app_db::SessionListIndexRecord>,
    seen_paths: &HashSet<String>,
    invalid_paths: &HashSet<String>,
    unverified_prefixes: &[PathBuf],
) -> Vec<String> {
    let candidates: Vec<String> = cached
        .iter()
        .filter(|(path, record)| {
            // 有归档快照的会话即使源文件删除也保留索引行（与 purge_orphan_session_index 行为一致）
            !seen_paths.contains(*path)
                && !record.has_archive_snapshot
                // 读取失败的子树下，行的"没被枚举到"是读取失败造成的，不能推断为已删除
                && !is_under_unverified_prefix(path, unverified_prefixes)
        })
        .map(|(path, _)| path.clone())
        .collect();

    // 存活比例守卫：一次瞬时故障（数据源误配、磁盘未挂载、权限变化）会让本轮枚举结果大面积
    // 缺失，此时按"没扫到"推断删除会把整批记录清光。被拦下的陈旧行不会永久滞留 —— 启动维护
    // 的 purge_orphan_session_index 会按"文件确实不存在"逐条清掉，代价只是延迟一次重启。
    // 只作用于「压根没枚举到」这一类：解析失败的行已计入 seen_paths（文件确实在），走
    // invalid_paths 通道，不受这里影响。
    let known = cached.len();
    let alive = cached.keys().filter(|path| seen_paths.contains(*path)).count();
    let blocked = known > MASS_DELETION_GUARD_MIN_KNOWN && (alive == 0 || alive * 2 < known);
    if blocked {
        tracing::warn!(
            "会话陈旧行清理被存活比例守卫拦下：已知 {} 条、本轮存活 {} 条，疑似数据源不可用或读取故障；保留原样待下一轮判定",
            known,
            alive
        );
    }

    let mut stale_paths = if blocked { Vec::new() } else { candidates };
    stale_paths.extend(invalid_paths.iter().cloned());
    stale_paths.sort();
    stale_paths.dedup();
    stale_paths
}

fn persist_session_list_index_changes(
    kind: CliKind,
    cached: &HashMap<String, app_db::SessionListIndexRecord>,
    seen_paths: &HashSet<String>,
    updates: Vec<app_db::SessionListIndexRecord>,
    invalid_paths: &HashSet<String>,
    unverified_prefixes: &[PathBuf],
) {
    if let Err(err) = app_db::upsert_session_list_index(kind, &updates) {
        tracing::warn!("写入会话列表索引失败: cli={}, err={}", kind.id(), err.diagnostic());
    }

    let stale_paths = collect_stale_paths(cached, seen_paths, invalid_paths, unverified_prefixes);
    if !stale_paths.is_empty() {
        if let Err(err) = app_db::delete_session_list_index_paths(kind, &stale_paths) {
            tracing::warn!("清理失效会话列表索引失败: cli={}, err={}", kind.id(), err.diagnostic());
        }
        if let Err(err) = app_db::delete_session_search_paths(kind, &stale_paths) {
            tracing::warn!("清理失效会话全文索引失败: cli={}, err={}", kind.id(), err.diagnostic());
        }
    }
}

/// 数据源切换后作废「落在新数据源范围之外」的会话索引行。
///
/// 会话列表只呈现当前数据源范围内的会话；范围外的记录不再可见，但**数据本体一概保留**：
/// 归档快照（`archived_session_content` 与磁盘上的 .gz）、收藏、自定义名称、书签都不动，
/// 数据源改回去时靠重新扫描恢复可见性。
///
/// 这一步不能省：范围外的行既不会被扫描枚举到（扫的是新目录），也不会走常规的陈旧行清理
/// —— 那份清理只在扫描成功跑完后、且以「本轮枚举结果 vs 库里已知行」为基准才生效。少了它，
/// 旧数据源的会话会永远挂在列表里。
pub(crate) fn purge_index_rows_outside_data_source(kind: CliKind) {
    let Ok(scope_root) = cli::sessions_dir(kind) else {
        tracing::warn!(
            "数据源切换后作废索引行跳过：无法解析会话目录 cli={}",
            kind.id()
        );
        return;
    };
    let stale = match app_db::session_index_paths_outside_scope(kind, &scope_root) {
        Ok(paths) => paths,
        Err(err) => {
            tracing::warn!("读取范围外会话索引行失败: cli={}, err={}", kind.id(), err.diagnostic());
            return;
        }
    };
    if stale.is_empty() {
        return;
    }
    if let Err(err) = app_db::delete_session_list_index_paths(kind, &stale) {
        tracing::warn!("作废范围外会话索引行失败: cli={}, err={}", kind.id(), err.diagnostic());
    }
    // 检索是派生物：旧路径的检索状态一并作废，避免切换后命中已不在列表里的会话
    if let Err(err) = app_db::delete_session_search_paths(kind, &stale) {
        tracing::warn!("作废范围外会话检索状态失败: cli={}, err={}", kind.id(), err.diagnostic());
    }
    tracing::info!(
        "数据源切换：作废 {} 条范围外的会话索引行 cli={} scope={:?}",
        stale.len(),
        kind.id(),
        scope_root
    );
}

#[tauri::command]
pub async fn ensure_search_index_ready(
    app: tauri::AppHandle,
    cli_id: Option<String>,
) -> AppResult<()> {
    let kind = CliKind::from_id(cli_id.as_deref())?;

    {
        let ongoing = ONGOING_INDEX_BUILDS
            .lock()
            .map_err(|e| AppError::coded("session_index.build_lock_failed").with("detail", e.to_string()))?;
        if ongoing.contains(&kind) {
            return Ok(());
        }
    }

    // 先清理纯搜索孤儿（物理索引里有、源文件与归档都没有的路径），
    // 避免重建后仍残留"可搜不可开"的陈旧命中。
    if let Err(err) = app_db::purge_search_orphans(Some(kind.id())) {
        tracing::warn!("重建索引前清理搜索孤儿失败: err={}", err.diagnostic());
    }

    let list_index = app_db::read_session_list_index(kind)?;
    let physical_index_ready = app_db::tantivy_search::is_physical_index_ready()?;
    if !physical_index_ready {
        tracing::warn!(
            "搜索索引: 物理索引缺失或不可打开，将清空全部状态并重建, cli={}",
            kind.id()
        );
        app_db::clear_all_session_search_index_state()?;
        app_db::tantivy_search::reset_physical_index()?;
    }
    let search_index = if physical_index_ready {
        app_db::read_session_search_index(kind)?
    } else {
        HashMap::new()
    };

    let missing_paths = collect_missing_search_paths(
        kind,
        &list_index,
        &search_index,
        physical_index_ready,
    );

    if missing_paths.is_empty() {
        return Ok(());
    }

    {
        let mut ongoing = ONGOING_INDEX_BUILDS
            .lock()
            .map_err(|e| AppError::coded("session_index.build_lock_failed").with("detail", e.to_string()))?;
        ongoing.insert(kind);
    }

    let app_clone = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let total = missing_paths.len();
        let total_bytes: u64 = missing_paths
            .iter()
            .filter_map(|path| list_index.get(path).map(|record| record.file_size))
            .sum();
        tracing::info!(
            "搜索索引: 开始构建 {} 个会话的搜索索引, cli={}",
            total,
            kind.id()
        );

        emit_search_index_progress(
            &app_clone,
            kind,
            "scanning",
            0,
            total,
            0,
            total_bytes,
            None,
            0,
            0,
        );

        let single_commit_rebuild = total > SEARCH_INDEX_WRITE_BATCH_SIZE;
        let mut batches = Vec::new();
        let mut state_updates = Vec::new();
        let mut processed_complete_bytes = 0u64;
        let mut loop_emit_gate = IndexProgressEmitGate::new();
        for (i, path) in missing_paths.iter().enumerate() {
            let Some(record) = list_index.get(path) else {
                continue;
            };
            let current_file_size = record.file_size;
            let mut last_emit = Instant::now()
                .checked_sub(Duration::from_millis(250))
                .unwrap_or_else(Instant::now);
            let docs = scan_session_search_docs_for_index(kind, path, record, |progress| {
                let now = Instant::now();
                let is_complete = progress.bytes_read >= progress.file_size;
                if !is_complete && now.duration_since(last_emit) < Duration::from_millis(250) {
                    return;
                }
                last_emit = now;
                emit_search_index_progress(
                    &app_clone,
                    kind,
                    "scanning",
                    i,
                    total,
                    processed_complete_bytes.saturating_add(progress.bytes_read),
                    total_bytes,
                    Some(path),
                    progress.bytes_read,
                    progress.file_size,
                );
            });

            processed_complete_bytes = processed_complete_bytes.saturating_add(current_file_size);

            if let Some(docs) = docs {
                let batch = build_session_search_doc_batch(path, record.modified_ms, &docs);
                if single_commit_rebuild {
                    if loop_emit_gate.should_emit("writing", i == total - 1) {
                        emit_search_index_progress(
                            &app_clone,
                            kind,
                            "writing",
                            i + 1,
                            total,
                            processed_complete_bytes,
                            total_bytes,
                            Some(path),
                            0,
                            0,
                        );
                    }
                    let update = match app_db::write_session_search_docs_uncommitted(kind, &batch) {
                        Ok(update) => update,
                        Err(err) => {
                            // Discard any docs queued by earlier iterations so
                            // they are not flushed by a later unrelated commit.
                            app_db::tantivy_search::rollback_writes();
                            return Err(err);
                        }
                    };
                    state_updates.push(update);
                    if loop_emit_gate.should_emit("written", i == total - 1) {
                        emit_search_index_progress(
                            &app_clone,
                            kind,
                            "written",
                            i + 1,
                            total,
                            processed_complete_bytes,
                            total_bytes,
                            Some(path),
                            0,
                            0,
                        );
                    }
                } else {
                    batches.push(batch);
                }
            } else {
                // 文件存在但无可搜索文本（如纯工具调用会话）：
                // 写入 doc_count=0 的状态记录，标记该路径已处理完毕，
                // 避免 get_search_index_status 因找不到记录而反复计为 missing。
                let empty_update = app_db::SessionSearchIndexStateUpdate {
                    session_path: path.clone(),
                    modified_ms: record.modified_ms,
                    doc_count: 0,
                };
                if single_commit_rebuild {
                    state_updates.push(empty_update);
                } else {
                    // 小批量路径：用空 batch 触发 replace_session_search_docs_with_progress
                    // 实现同样效果 —— 直接将 empty_update 加入 state_updates 统一在结尾写入
                    state_updates.push(empty_update);
                }
            }

            if !single_commit_rebuild && batches.len() >= SEARCH_INDEX_WRITE_BATCH_SIZE {
                if loop_emit_gate.should_emit("writing", i == total - 1) {
                    emit_search_index_progress(
                        &app_clone,
                        kind,
                        "writing",
                        i + 1,
                        total,
                        processed_complete_bytes,
                        total_bytes,
                        Some(path),
                        current_file_size,
                        current_file_size,
                    );
                }
                let batch_base = i + 1 - batches.len();
                app_db::replace_session_search_docs_with_progress(kind, &batches, |progress| {
                    let current = batch_base + progress.written_sessions;
                    if loop_emit_gate.should_emit(progress.phase, current + 1 >= total) {
                        emit_search_index_progress(
                            &app_clone,
                            kind,
                            progress.phase,
                            current,
                            total,
                            processed_complete_bytes,
                            total_bytes,
                            progress.current_path.as_deref(),
                            0,
                            0,
                        );
                    }
                })?;
                batches.clear();
            }

            if loop_emit_gate.should_emit("scanning", i == total - 1) {
                emit_search_index_progress(
                    &app_clone,
                    kind,
                    "scanning",
                    i + 1,
                    total,
                    processed_complete_bytes,
                    total_bytes,
                    Some(path),
                    current_file_size,
                    current_file_size,
                );
            }
        }

        if single_commit_rebuild {
            if !state_updates.is_empty() {
                emit_search_index_progress(
                    &app_clone,
                    kind,
                    "committing",
                    total,
                    total,
                    processed_complete_bytes,
                    total_bytes,
                    None,
                    0,
                    0,
                );
                if let Err(err) = app_db::commit_session_search_writes() {
                    app_db::tantivy_search::rollback_writes();
                    return Err(err);
                }
                emit_search_index_progress(
                    &app_clone,
                    kind,
                    "committed",
                    total,
                    total,
                    processed_complete_bytes,
                    total_bytes,
                    None,
                    0,
                    0,
                );
                app_db::replace_session_search_index_state(kind, &state_updates)?;
            }
        } else {
            if !batches.is_empty() {
                emit_search_index_progress(
                    &app_clone,
                    kind,
                    "writing",
                    total,
                    total,
                    processed_complete_bytes,
                    total_bytes,
                    None,
                    0,
                    0,
                );
                let batch_base = total - batches.len();
                app_db::replace_session_search_docs_with_progress(kind, &batches, |progress| {
                    let current = batch_base + progress.written_sessions;
                    if loop_emit_gate.should_emit(progress.phase, current + 1 >= total) {
                        emit_search_index_progress(
                            &app_clone,
                            kind,
                            progress.phase,
                            current,
                            total,
                            processed_complete_bytes,
                            total_bytes,
                            progress.current_path.as_deref(),
                            0,
                            0,
                        );
                    }
                })?;
            }
            // 小批量路径：为无可搜索文本的会话单独写入 doc_count=0 的状态记录，
            // 防止 get_search_index_status 因找不到该路径记录而反复报告 missing。
            if !state_updates.is_empty() {
                app_db::replace_session_search_index_state(kind, &state_updates)?;
            }
        }

        tracing::info!("搜索索引: 构建完成, cli={}", kind.id());
        emit_search_index_progress(
            &app_clone,
            kind,
            "done",
            total,
            total,
            total_bytes,
            total_bytes,
            None,
            0,
            0,
        );
        Ok::<(), AppError>(())
    })
    .await;

    {
        let mut ongoing = ONGOING_INDEX_BUILDS
            .lock()
            .map_err(|e| AppError::coded("session_index.build_lock_failed").with("detail", e.to_string()))?;
        ongoing.remove(&kind);
    }

    // On ANY build failure (including a panicked blocking task), emit a
    // terminal "error" frame before propagating so the frontend clears its
    // progress indicator instead of being stuck (progress stays non-null and
    // blocks the search debounce). Single emit point: all error returns from
    // the build closure funnel through here.
    let build_result = match result {
        Ok(inner) => inner,
        Err(join_err) => Err(AppError::coded("session_index.build_task_failed").with("detail", join_err.to_string())),
    };
    if let Err(ref err) = build_result {
        tracing::warn!("搜索索引: 构建失败, cli={}, err={}", kind.id(), err.diagnostic());
        emit_search_index_progress(&app, kind, "error", 0, 0, 0, 0, None, 0, 0);
    }
    build_result
}

#[tauri::command]
pub fn get_search_index_status(cli_id: Option<String>) -> AppResult<SearchIndexStatus> {
    let kind = CliKind::from_id(cli_id.as_deref())?;
    resolve_search_index_status(kind)
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexStats {
    pub session_count: usize,
    pub db_size_bytes: u64,
    pub search_doc_count: usize,
    pub search_index_bytes: u64,
    pub last_updated_ms: Option<i64>,
}

#[tauri::command]
pub fn get_index_stats(cli_id: Option<String>) -> AppResult<IndexStats> {
    let kind = CliKind::from_id(cli_id.as_deref())?;
    let session_count = app_db::count_session_list_index(kind)?;

    // DB 大小 = 主库 + WAL + SHM（WAL 模式下数据可能还在 wal 文件里）
    let mut db_size_bytes = 0u64;
    if let Ok(db_path) = app_db::app_db_path() {
        for suffix in ["", "-wal", "-shm"] {
            let path = if suffix.is_empty() {
                db_path.clone()
            } else {
                std::path::PathBuf::from(format!("{}{}", db_path.display(), suffix))
            };
            if let Ok(meta) = std::fs::metadata(&path) {
                db_size_bytes += meta.len();
            }
        }
    }

    let search_doc_count = app_db::count_session_search_indexed_docs(kind).unwrap_or(0);

    let search_index_bytes = {
        let mut total = 0u64;
        if let Ok(search_dir) = app_db::search_index_dir() {
            if let Ok(entries) = std::fs::read_dir(&search_dir) {
                for entry in entries.flatten() {
                    if let Ok(meta) = entry.metadata() {
                        if meta.is_file() {
                            total += meta.len();
                        }
                    }
                }
            }
        }
        total
    };

    let last_updated_ms = app_db::read_index_last_updated(kind.id())?;

    Ok(IndexStats {
        session_count,
        db_size_bytes,
        search_doc_count,
        search_index_bytes,
        last_updated_ms,
    })
}

#[tauri::command]
pub async fn clear_session_index(
    app: tauri::AppHandle,
    cli_id: Option<String>,
) -> AppResult<bool> {
    let kind = CliKind::from_id(cli_id.as_deref())?;
    let app_clone = app.clone();
    tauri::async_runtime::spawn_blocking(move || -> AppResult<()> {
        let removed_paths = {
            let conn = app_db::conn()?;
            app_db::clear_session_index_inner(&conn, kind.id())?
        };
        // tantivy 物理索引同步删除（失败不阻塞）
        if !removed_paths.is_empty() {
            let _ = app_db::tantivy_search::delete_session_docs(&removed_paths);
        }
        let now_ms = chrono::Utc::now().timestamp_millis();
        if let Ok(conn) = app_db::conn() {
            let _ = app_db::write_index_last_updated(&conn, kind.id(), now_ms);
        }
        Ok(())
    })
    .await
    .map_err(|e| AppError::coded("session_index.clear_task_failed").with("detail", e.to_string()))??;

    emit_session_list_index_updated(&app_clone, kind);
    Ok(true)
}

fn requested_kinds_with_data_dir<F>(
    cli_ids: Option<&[String]>,
    front_cli_id: Option<&str>,
    mut data_dir_exists: F,
) -> AppResult<Vec<CliKind>>
where
    F: FnMut(CliKind) -> bool,
{
    let mut seen = HashSet::new();
    let mut kinds = match cli_ids {
        Some(ids) => {
            let mut parsed = Vec::with_capacity(ids.len());
            for id in ids {
                let kind = CliKind::from_id(Some(id.as_str()))?;
                if seen.insert(kind) {
                    parsed.push(kind);
                }
            }
            parsed
        }
        None => crate::cli::CliKind::ALL.to_vec(),
    };

    let front = front_cli_id
        .map(|id| CliKind::from_id(Some(id)))
        .transpose()?;
    kinds.retain(|kind| data_dir_exists(*kind));

    if let Some(front) = front {
        if let Some(index) = kinds.iter().position(|kind| *kind == front) {
            kinds.remove(index);
            kinds.insert(0, front);
        }
    }

    Ok(kinds)
}

pub(crate) fn requested_kinds(
    cli_ids: Option<&[String]>,
    front_cli_id: Option<&str>,
) -> AppResult<Vec<CliKind>> {
    requested_kinds_with_data_dir(cli_ids, front_cli_id, |kind| {
        cli::data_dir(kind).map(|dir| dir.exists()).unwrap_or(false)
    })
}

pub(crate) fn effective_cli_ids(
    cli_ids: Option<Vec<String>>,
    legacy_cli_id: Option<String>,
) -> Option<Vec<String>> {
    cli_ids.or_else(|| match legacy_cli_id {
        Some(id) if id.trim() == "all" => None,
        Some(id) => Some(vec![id]),
        None => None,
    })
}

pub(crate) struct CliFanoutOutcome<T> {
    pub(crate) kind: CliKind,
    /// 保持 `AppResult` 不压平：`operation` 产出的可能是 `AppError::Coded`，
    /// 而 `Coded` 的 `Display` 就是裸 code——压成 `String` 后前端 `renderAppError`
    /// 只会把 code 原样上屏（用户看到 `internal.database` 而不是文案）。
    /// 两条消费通道（`SearchStreamCliError.error` / `ScanProjectsCliResult.error`）
    /// 因此都携带 `AppError` 的 IPC 形状：`Coded` → `{code, params}`，`Business` → 字符串。
    pub(crate) result: AppResult<T>,
}

pub(crate) fn run_cli_fanout<T, F, H>(
    kinds: &[CliKind],
    mut operation: F,
    mut handle_outcome: H,
)
where
    F: FnMut(CliKind) -> AppResult<T>,
    H: FnMut(CliFanoutOutcome<T>),
{
    for kind in kinds.iter().copied() {
        handle_outcome(CliFanoutOutcome {
            kind,
            result: operation(kind),
        });
    }
}

/// 会话候选副本描述结构
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SessionCandidate {
    pub file_path: std::path::PathBuf,
    pub is_active_workspace: bool,
    pub modified_ms: i64,
    pub file_size: u64,
}

impl AsRef<std::path::Path> for SessionCandidate {
    fn as_ref(&self) -> &std::path::Path {
        &self.file_path
    }
}

/// 对多候选会话副本按优先级排序：
/// 1. 活跃工作区目录优先
/// 2. 修改时间最新优先
/// 3. 文件大小大者优先
/// 4. 路径字典序升序（保证稳定排序）
#[allow(dead_code)]
pub(crate) fn sort_session_candidates(candidates: &mut [SessionCandidate]) {
    candidates.sort_by(|a, b| {
        b.is_active_workspace
            .cmp(&a.is_active_workspace)
            .then_with(|| b.modified_ms.cmp(&a.modified_ms))
            .then_with(|| b.file_size.cmp(&a.file_size))
            .then_with(|| a.file_path.cmp(&b.file_path))
    });
}

/// 从有序候选副本列表中顺位尝试提取首个有效解析结果。
/// 若前序候选遇到 0 字节、文件损坏或解析错误，平滑降级到后续副本。
pub(crate) fn pick_first_valid_candidate<P, T, F>(
    candidates: &[P],
    mut parser: F,
) -> Option<T>
where
    P: AsRef<std::path::Path>,
    F: FnMut(&std::path::Path) -> Option<T>,
{
    for (idx, candidate) in candidates.iter().enumerate() {
        let path = candidate.as_ref();
        if let Some(res) = parser(path) {
            return Some(res);
        }
        if idx + 1 < candidates.len() {
            tracing::warn!("会话候选文件解析失败，顺位尝试下一个副本: path={:?}", path);
        }
    }
    None
}

#[cfg(test)]
mod tests;

mod scan;
pub(crate) use scan::*;

mod scan_accumulator;
pub(crate) use scan_accumulator::*;
