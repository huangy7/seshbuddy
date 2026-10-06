//! 会话扫描与索引刷新：走盘读取、回写索引、并通过流式事件推进度。
//!
//! 与 `mod.rs` 的分工：`mod.rs` 负责索引的读取、清理与列表装配（数据源是库），
//! 本模块负责从文件系统重建这些数据（数据源是盘）。

use crate::cli_registry::SessionLocator;
use crate::session::SessionListMetadata;

use super::*;

const SCAN_PROJECTS_TOPIC: &str = "scan_projects";
const SCAN_PROJECTS_BATCH_SIZE: usize = 50;
const SCAN_PROJECTS_SNAPSHOT_LIMIT: usize = 100_000;

#[derive(Debug, Clone, serde::Serialize)]
struct ScanProjectsStreamDone {
    total_sessions: usize,
    cli_results: Vec<ScanProjectsCliResult>,
}

#[derive(Debug, Clone, serde::Serialize)]
struct ScanProjectsCliResult {
    cli_id: String,
    total_sessions: usize,
    /// 分项失败原样携带 `AppError`（`Coded` → `{code, params}`），不压成裸 code。
    /// 前端消费点：`useSessions.ts` 的 `cli_results[].error` → `scanCliErrors` →
    /// `HistoryPanel.vue` 的 `:title`。
    error: Option<AppError>,
}

fn load_projects_stream_items(
    kind: CliKind,
    custom_names: &HashMap<String, String>,
) -> AppResult<(Vec<ProjectSessionChunkItem>, usize)> {
    let snapshot = crate::cli_registry::source_for(kind).snapshot(
        custom_names,
        0,
        SCAN_PROJECTS_SNAPSHOT_LIMIT,
    );
    if let Ok(Some(paginated)) = snapshot {
        let mut items = Vec::new();
        for project in paginated.projects {
            for session in project.sessions {
                items.push(ProjectSessionChunkItem {
                    session,
                    encoded_dir: project.encoded_dir.clone(),
                    original_path: project.original_path.clone(),
                });
            }
        }
        return Ok((items, paginated.total_sessions));
    }

    let mut all_sessions = crate::cli_registry::source_for(kind).scan(custom_names, false)?;
    all_sessions.sort_by(|a, b| b.session.timestamp.cmp(&a.session.timestamp));
    let total = all_sessions.len();
    let items = all_sessions
        .into_iter()
        .map(|raw| ProjectSessionChunkItem {
            session: raw.session,
            encoded_dir: raw.encoded_dir,
            original_path: raw.original_path,
        })
        .collect();
    Ok((items, total))
}

#[tauri::command]
pub async fn scan_projects_stream(
    app: AppHandle,
    request_id: String,
    cli_ids: Option<Vec<String>>,
    cli_id: Option<String>,
) -> AppResult<()> {
    let effective_ids = effective_cli_ids(cli_ids, cli_id);
    let kinds = requested_kinds(effective_ids.as_deref(), None)?;

    streaming::spawn_streaming_task(
        app,
        SCAN_PROJECTS_TOPIC,
        request_id,
        move |app, topic, request_id| {
            let custom_names = crate::commands::session::load_session_names().unwrap_or_default();
            let mut total_sessions = 0usize;
            let mut cli_results = Vec::with_capacity(kinds.len());
            run_cli_fanout(
                &kinds,
                |kind| load_projects_stream_items(kind, &custom_names),
                |outcome| match outcome.result {
                    Ok((items, total)) => {
                        total_sessions += total;
                        for batch in items.chunks(SCAN_PROJECTS_BATCH_SIZE) {
                            streaming::emit_chunk(app, topic, request_id, &batch.to_vec());
                        }
                        cli_results.push(ScanProjectsCliResult {
                            cli_id: outcome.kind.id().to_string(),
                            total_sessions: total,
                            error: None,
                        });
                    }
                    Err(error) => cli_results.push(ScanProjectsCliResult {
                        cli_id: outcome.kind.id().to_string(),
                        total_sessions: 0,
                        error: Some(error),
                    }),
                },
            );

            streaming::emit_done(
                app,
                topic,
                request_id,
                &ScanProjectsStreamDone {
                    total_sessions,
                    cli_results,
                },
            );
        },
    );

    Ok(())
}

#[tauri::command]
pub async fn refresh_session_list_index(
    app: tauri::AppHandle,
    cli_id: Option<String>,
    notify: Option<bool>,
    force: Option<bool>,
) -> AppResult<bool> {
    let kind = CliKind::from_id(cli_id.as_deref())?;
    let force = force.unwrap_or(false);
    // 强制请求排队等待槽位（被跳过会让索引停在「已清空但没人重建」的状态）；
    // 后台刷新沿用「占用即跳过」，避免定时与监听刷新堆叠
    if force {
        wait_for_list_index_refresh_slot(kind).await?;
    } else if !try_acquire_list_index_refresh_slot(kind)? {
        tracing::debug!("CLI {:?} 的会话列表索引刷新已在进行中，跳过并发调用", kind);
        return Ok(false);
    }
    let _guard = ListIndexRefreshGuard(kind);

    let app_clone = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        scan_projects_inner_for_cli(kind, None, None, force)?;
        Ok::<(), AppError>(())
    })
    .await
    .map_err(|e| AppError::coded("session_index.scan_task_failed").with("detail", e.to_string()))??;
    if notify.unwrap_or(false) {
        emit_session_list_index_updated(&app_clone, kind);
    }
    Ok(true)
}

/// 会话列表索引的派生规则版本号。
///
/// 每当「从会话文件推导出的元数据」其含义发生变化（修正某个字段的推导方式）就必须递增。
/// 索引缓存按 `(路径, 文件大小, mtime)` 命中，会话文件本身没动就不会重新解析，于是按旧规则
/// 写下的错误元数据会一直留在库里、且看起来「一切正常」。递增后本轮扫描强制全量重解析
/// 一次，把存量行刷新到新规则，然后才落版本号。
///
/// **版本 2**：`project_path` 解析不出时的取值由占位串改为「没有值」（SQL `NULL`）。
/// 旧规则把占位串写进了库，而读取侧再也认不出它（见 `session::resolve_project_path`），
/// 于是那些行必须按新规则重解析一次。
pub(super) const SESSION_INDEX_REVISION: u64 = 2;
const SESSION_INDEX_REVISION_KEY_PREFIX: &str = "session_index_revision";

/// 派生规则版本按 CLI 独立记录。
///
/// 索引本身是「每个 CLI 一份」，版本闸门也必须同粒度：用单个全局键时，前端按 CLI 并发
/// 扇出、谁先扫完谁就把版本记成当前值，其余 CLI 随后读到「不落后」而跳过全量重建 ——
/// 于是只有第一个 CLI 的逻辑真正生效，其余 CLI 按老规则写下的元数据被永久固化
/// （例如按日志文件名推导出的占位会话 ID）。
pub(super) fn session_index_revision_key(kind: CliKind) -> String {
    format!("{}:{}", SESSION_INDEX_REVISION_KEY_PREFIX, kind.id())
}

/// 存量索引是否落后于当前派生规则。
/// 版本号缺失按「落后」处理：改过推导规则后第一次扫描需要按新规则重建一次；
/// 而全新安装时索引本来就是空的，多走一次全量解析不产生额外开销。
/// 设置表读取失败则按「不落后」处理，避免设置表异常导致每轮扫描都全量重解析。
pub(super) fn session_index_revision_outdated(kind: CliKind) -> bool {
    match app_db::read_setting_json::<u64>(&session_index_revision_key(kind)) {
        Ok(Some(stored)) => stored < SESSION_INDEX_REVISION,
        Ok(None) => true,
        Err(err) => {
            tracing::warn!(
                "读取会话索引规则版本失败，本轮不做全量重建: cli={}, err={}",
                kind.id(),
                err.diagnostic()
            );
            false
        }
    }
}

/// 记下该 CLI 已按当前规则完成全量重建；写入失败则保持版本落后，下一轮扫描重试。
pub(super) fn mark_session_index_revision_current(kind: CliKind) {
    if let Err(err) = app_db::write_setting_json(
        &session_index_revision_key(kind),
        &SESSION_INDEX_REVISION,
    ) {
        tracing::warn!(
            "记录会话索引规则版本失败，下轮扫描会再次全量重建: cli={}, err={}",
            kind.id(),
            err.diagnostic()
        );
    }
}

pub(crate) fn scan_projects_inner_for_cli(
    kind: CliKind,
    page: Option<usize>,
    page_size: Option<usize>,
    force: bool,
) -> AppResult<PaginatedProjects> {
    // 派生规则版本落后时，本轮强制全量重解析，刷新按旧规则固化的存量元数据
    let revision_outdated = session_index_revision_outdated(kind);
    let force = force || revision_outdated;

    let custom_names = crate::commands::session::load_session_names().unwrap_or_default();
    let all_sessions = crate::cli_registry::source_for(kind).scan(&custom_names, force)?;

    // 只有全量扫描真正跑完才落版本号：中途出错时保持落后，下一轮重试而不是固化半个索引
    if revision_outdated {
        mark_session_index_revision_current(kind);
    }

    Ok(paginate_sessions(all_sessions, page, page_size))
}

/// 一条会话的逐 CLI 投影。后五个字段与 `SnapshotProjection` 同义 ——
/// 扫描与快照是同一份投影的两种装配路径。
pub(crate) struct ScanProjection {
    /// **最终采用**的项目路径。投影不是纯增量的：元数据解析出的项目路径常需被目录结构
    /// 或外部映射纠正。实测 6 个源里 5 个都做这件事（只有 codex 不改），差异只在
    /// 「纠正值从哪来」。骨架负责比较它与元数据里的值、不同则写一条索引更新 ——
    /// 逐源各写一遍这段比较与记录是纯粹的重复。
    pub(crate) project_path: Option<String>,
    pub(crate) encoded_dir: Option<String>,
    pub(crate) original_path: Option<String>,
    pub(crate) title: Option<String>,
    pub(crate) history_display: Option<String>,
    pub(crate) git_branch: String,
}

/// 一个定位符的解析结果。
///
/// **为什么不直接返回元数据**：`dsh` 的会话是**目录**，其下有多代候选文件
/// （`session.jsonl` / `session.jsonl.zstd` / …），候选链可能全部落空，也可能命中一个
/// 子代理或空表面 —— 这些都不是「会话」，装不进「一个定位符 = 一份元数据」。
/// 库型源更极端（一个会话 = 库里 N+1 行）。用 `Skip` 显式表达「这条定位符不是会话」，
/// 模型才装得下它们。
pub(crate) enum Resolution {
    Session { metadata: SessionListMetadata, projection: ScanProjection },
    /// 是会话但解析失败：计入 invalid_index_paths，不写索引、不进列表。
    ///
    /// 必须与 `Skip` 分开：`Skip` 只表达「这条定位符不是会话」，而解析失败另有归宿 ——
    /// 失效集合是收尾清理里独立于「已见 / 没被枚举到」的通道，两者混同会让解析失败的行
    /// 改走陈旧判定，与迁移前的逐 CLI 扫描行为分叉。
    Unparsed,
    /// **不是会话**（候选链耗尽、子代理、空表面……）：跳过，不计入任何集合。
    Skip,
}

/// `build_projects_scan` 这层糖在调用 `resolve` 前备好的上下文 —— 让闭包不必自己去读库。
///
/// 这是**糖**的上下文类型。直接用累加器的源不走这里：它按需调 `acc` 上对应的方法，
/// 不必先被塞进一个固定字段的上下文。
pub(crate) struct ScanContext<'a> {
    /// `force` 为真时源应忽略 `cached_index` 里的缓存元数据、强制重解析。
    pub force: bool,
    /// 库中已知的索引行，键是持久化的定位符串。恒为库里已知的行（不受 `force` 清空影响），
    /// 由源据 `force` 决定是否使用。
    pub cached_index: &'a HashMap<String, app_db::SessionListIndexRecord>,
    pub tombstone_filter: &'a app_db::TombstoneFilter,
}

/// 文件型源的通用扫描：一个定位符一个文件，持久化键就是定位符串。
///
/// 装在 `ScanAccumulator` 之上的**可选糖** —— 形态吻合的源用它，不吻合的源（会话是目录、
/// 一个会话对应多个候选文件或库中多行）直接驱动累加器。
///
/// **骨架保留的次序**（是行为，不可重排）：
/// 1. `force` 时清空元数据复用视图，但陈旧行判定始终以库中已知行为基准 ——
///    否则强制刷新时陈旧行永久滞留；
/// 2. 收尾持久化带上 `unverified_prefixes`。
///
/// 逐条解析由调用方的闭包给出：它捕获该 CLI 的准备数据（history 解析、标题映射等），
/// 返回 `Resolution`。**墓碑拦截与缓存命中的判断也归闭包** —— 各 CLI 的判据不同
/// （DSH 的墓碑要覆盖会话目录下的全部候选文件），收在骨架里会削窄它。
///
/// 闭包而非 trait 方法：`resolve` 要用到 `scan` 里算好的准备数据，而源是 `&'static`
/// 无状态的，trait 方法签名里带不了每个源各自不同的准备类型。闭包天然捕获准备数据，
/// 类型由编译器保证，不必靠运行时向下转型把它递进来。
pub(crate) fn build_projects_scan<F>(
    kind: CliKind,
    custom_names: &HashMap<String, String>,
    force: bool,
    discovered: crate::cli_registry::Discovered,
    resolve: F,
) -> AppResult<Vec<RawSession>>
where
    F: Fn(&SessionLocator, &ScanContext) -> AppResult<Resolution>,
{
    let mut acc = ScanAccumulator::new(kind, custom_names, force)?;
    let unverified_prefixes = discovered.unverified_prefixes;
    for locator in discovered.locators {
        let key = locator.to_key();
        // 借用先结束再改动：`context()` 借 `acc`，`resolve` 返回的是自有值。
        let resolution = {
            let ctx = acc.context();
            resolve(&locator, &ctx)?
        };
        match resolution {
            Resolution::Session {
                metadata,
                projection,
            } => acc.push_session(&key, metadata, projection),
            // 是会话但解析失败：不写索引、不进列表，只记入失效集合，收尾时与陈旧行一并清理。
            // 与 Skip 分开，否则会与「没被枚举到」的陈旧判定混为一谈。
            Resolution::Unparsed => acc.mark_unparsed(&key),
            Resolution::Skip => {}
        }
    }
    acc.finish(&unverified_prefixes)
}

pub(super) fn paginate_sessions(
    mut all_sessions: Vec<RawSession>,
    page: Option<usize>,
    page_size: Option<usize>,
) -> PaginatedProjects {
    let total_sessions = all_sessions.len();
    all_sessions.sort_by(|a, b| b.session.timestamp.cmp(&a.session.timestamp));

    let selected_sessions: Vec<RawSession> = match (page, page_size) {
        (Some(p), Some(ps)) => {
            let start = p * ps;
            if start >= all_sessions.len() {
                Vec::new()
            } else {
                let end = std::cmp::min(start + ps, all_sessions.len());
                all_sessions
                    .into_iter()
                    .skip(start)
                    .take(end - start)
                    .collect()
            }
        }
        _ => all_sessions,
    };

    let has_more = match (page, page_size) {
        (Some(p), Some(ps)) => (p + 1) * ps < total_sessions,
        _ => false,
    };

    build_paginated_projects_from_sessions(selected_sessions, total_sessions, has_more)
}

