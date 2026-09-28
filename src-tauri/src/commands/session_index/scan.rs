//! 会话扫描与索引刷新：走盘读取、回写索引、并通过流式事件推进度。
//!
//! 与 `mod.rs` 的分工：`mod.rs` 负责索引的读取、清理与列表装配（数据源是库），
//! 本模块负责从文件系统重建这些数据（数据源是盘）。

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
    let snapshot = match kind {
        CliKind::Claude => {
            load_claude_projects_snapshot(custom_names, 0, SCAN_PROJECTS_SNAPSHOT_LIMIT)
        }
        CliKind::Codex => {
            load_codex_projects_snapshot(custom_names, 0, SCAN_PROJECTS_SNAPSHOT_LIMIT)
        }
        CliKind::Gemini => {
            load_gemini_projects_snapshot(custom_names, 0, SCAN_PROJECTS_SNAPSHOT_LIMIT)
        }
        CliKind::WorkBuddy => {
            load_workbuddy_projects_snapshot(custom_names, 0, SCAN_PROJECTS_SNAPSHOT_LIMIT)
        }
        CliKind::Dsh => {
            load_dsh_projects_snapshot(custom_names, 0, SCAN_PROJECTS_SNAPSHOT_LIMIT)
        }
        CliKind::Antigravity => {
            load_antigravity_projects_snapshot(custom_names, 0, SCAN_PROJECTS_SNAPSHOT_LIMIT)
        }
    };
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

    let mut all_sessions = match kind {
        CliKind::Claude => scan_claude_projects(custom_names, false)?,
        CliKind::Codex => scan_codex_projects(custom_names, false)?,
        CliKind::Gemini => scan_gemini_projects(custom_names, false)?,
        CliKind::WorkBuddy => scan_workbuddy_projects(custom_names, false)?,
        CliKind::Dsh => scan_dsh_projects(custom_names, false)?,
        CliKind::Antigravity => scan_antigravity_projects(custom_names, false)?,
    };
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
    let all_sessions = match kind {
        CliKind::Claude => scan_claude_projects(&custom_names, force)?,
        CliKind::Codex => scan_codex_projects(&custom_names, force)?,
        CliKind::Gemini => scan_gemini_projects(&custom_names, force)?,
        CliKind::WorkBuddy => scan_workbuddy_projects(&custom_names, force)?,
        CliKind::Dsh => scan_dsh_projects(&custom_names, force)?,
        CliKind::Antigravity => scan_antigravity_projects(&custom_names, force)?,
    };

    // 只有全量扫描真正跑完才落版本号：中途出错时保持落后，下一轮重试而不是固化半个索引
    if revision_outdated {
        mark_session_index_revision_current(kind);
    }

    Ok(paginate_sessions(all_sessions, page, page_size))
}

pub(super) fn scan_claude_projects(
    custom_names: &HashMap<String, String>,
    force: bool,
) -> AppResult<Vec<RawSession>> {
    let projects_dir = cli::sessions_dir(CliKind::Claude)?;
    let history_path = cli::history_path(CliKind::Claude)?;
    let (session_map, project_map) = history::parse_history(history_path.to_str().unwrap_or(""));

    if !projects_dir.exists() {
        return Ok(Vec::new());
    }

    // force 模式下不使用缓存索引，所有文件重新解析
    let cached_index = if force {
        HashMap::new()
    } else {
        load_session_list_index(CliKind::Claude)
    };
    // 陈旧行判定始终以「库里已知的行」为基准：force 清空缓存只是不复用元数据，
    // 不能连带让清理失去比对基准 —— 否则强制刷新时陈旧行会永久滞留在列表里
    // （数据源切换、会话删除后都清不掉）。非 force 时两者同源，不额外读库。
    let forced_known_index = if force {
        load_session_list_index(CliKind::Claude)
    } else {
        HashMap::new()
    };
    let stale_base = if force { &forced_known_index } else { &cached_index };
    let mut index_updates = Vec::new();
    let mut seen_paths = HashSet::new();
    // 本轮读取失败（权限/IO）的子树：其下已有记录保留原样，不按"已删除"处理
    let mut unverified_prefixes: Vec<PathBuf> = Vec::new();
    let mut invalid_index_paths = HashSet::new();
    let mut all_sessions = Vec::new();
    let tombstone_filter = app_db::load_tombstone_filter(CliKind::Claude).unwrap_or_default();
    let entries = fs::read_dir(&projects_dir)?;

    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => continue,
        };
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let encoded_dir = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();
        let fallback_project_path =
            session_mod::resolve_project_path(&encoded_dir, None, Some(&project_map));

        let Some(session_entries) = read_dir_or_record_unverified(&path, &mut unverified_prefixes)
        else {
            continue;
        };

        for session_entry in session_entries {
            let session_entry = match session_entry {
                Ok(entry) => entry,
                Err(_) => continue,
            };
            let session_path = session_entry.path();
            let file_name = session_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("");
            if !file_name.ends_with(".jsonl") {
                continue;
            }

            // 双轨墓碑拦截：若会话已被标记删除，拦截重新索引，防止死灰复燃
            let session_id = session_path.file_stem().and_then(|n| n.to_str()).unwrap_or("");
            let path_str = session_path.to_string_lossy();
            if tombstone_filter.is_tombstoned(session_id, &path_str) {
                continue;
            }

            let file_metadata = match fs::metadata(&session_path) {
                Ok(metadata) if metadata.is_file() => metadata,
                _ => continue,
            };
            let file_path = session_path.to_str().unwrap_or("").to_string();
            seen_paths.insert(file_path.clone());
            let Some(mut metadata) = resolve_session_index_payload(
                &session_path,
                &file_path,
                &file_metadata,
                &cached_index,
                &mut index_updates,
                &mut invalid_index_paths,
            ) else {
                continue;
            };

            let resolved_project_path = parser::read_project_path(&file_path)
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .or_else(|| {
                    session_mod::find_project_path_in_map(&encoded_dir, &project_map)
                        .map(|value| value.to_string())
                })
                .or_else(|| {
                    // 索引里读回的旧值：空值表示这一列没有值，不能拿它顶替真路径。
                    metadata
                        .project_path
                        .as_deref()
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(|value| value.to_string())
                });

            if metadata.project_path != resolved_project_path {
                metadata.project_path = resolved_project_path.clone();
                index_updates.push(build_session_list_index_record(
                    &file_path,
                    file_modified_ms(&file_metadata),
                    &metadata,
                ));
            }

            let original_path = metadata
                .project_path
                .clone()
                .or_else(|| fallback_project_path.clone());
            let session_id = metadata.session_id;

            let display_name = crate::title_resolver::resolve_display_name(
                crate::commands::session::custom_session_name(custom_names, CliKind::Claude, &file_path),
                metadata.title.as_deref(),
                metadata.first_user_message.as_deref(),
                session_map
                    .get(&session_id)
                    .map(|h| h.display.as_str())
                    .filter(|d| !d.is_empty()),
                &session_id,
            );

            let timestamp = metadata
                .last_timestamp
                .or(metadata.first_timestamp)
                .unwrap_or_default();
            let file_size = metadata.file_size;
            let git_branch = metadata.git_branch;
            let cached_record = cached_index.get(&file_path);
            let has_archive_snapshot = cached_record
                .map(|record| record.has_archive_snapshot)
                .unwrap_or(false);
            let is_archived = cached_record
                .map(|record| record.is_archived)
                .unwrap_or(false);

            all_sessions.push(RawSession {
                session: SessionInfo {
                    session_id,
                    file_path,
                    display_name,
                    timestamp,
                    file_size,
                    git_branch,
                    has_archive_snapshot,
                    is_archived,
                    cli_id: CliKind::Claude.id().to_string(),
                },
                encoded_dir: Some(encoded_dir.clone()),
                original_path: original_path.clone(),
            });
        }
    }

    persist_session_list_index_changes(
        CliKind::Claude,
        stale_base,
        &seen_paths,
        index_updates,
        &invalid_index_paths,
        &unverified_prefixes,
    );

    Ok(all_sessions)
}

pub(super) fn scan_codex_projects(
    custom_names: &HashMap<String, String>,
    force: bool,
) -> AppResult<Vec<RawSession>> {
    let sessions_dir = cli::sessions_dir(CliKind::Codex)?;
    let history_path = cli::history_path(CliKind::Codex)?;
    let session_map = history::parse_codex_history(history_path.to_str().unwrap_or(""));

    if !sessions_dir.exists() {
        return Ok(Vec::new());
    }

    // force 模式下不使用缓存索引，所有文件重新解析
    // force 模式下不使用缓存索引，所有文件重新解析
    let cached_index = if force {
        HashMap::new()
    } else {
        load_session_list_index(CliKind::Codex)
    };
    // 陈旧行判定始终以「库里已知的行」为基准：force 清空缓存只是不复用元数据，
    // 不能连带让清理失去比对基准 —— 否则强制刷新时陈旧行会永久滞留在列表里
    // （数据源切换、会话删除后都清不掉）。非 force 时两者同源，不额外读库。
    let forced_known_index = if force {
        load_session_list_index(CliKind::Codex)
    } else {
        HashMap::new()
    };
    let stale_base = if force { &forced_known_index } else { &cached_index };
    let mut index_updates = Vec::new();
    let mut seen_paths = HashSet::new();
    let mut invalid_index_paths = HashSet::new();
    let mut session_files = Vec::new();
    collect_jsonl_files(&sessions_dir, &mut session_files)?;
    session_files.sort();

    let mut all_sessions = Vec::new();
    let tombstone_filter = app_db::load_tombstone_filter(CliKind::Codex).unwrap_or_default();
    // 单次读取 Codex 索引 thread_name Map（规范 §3.1），整批查询 O(1) 命中
    let codex_titles = parser::load_codex_index_titles();
    for session_path in session_files {
        // 双轨墓碑拦截：若会话已被标记删除，拦截重新索引，防止死灰复燃
        let session_id = session_path.file_stem().and_then(|n| n.to_str()).unwrap_or("");
        let path_str = session_path.to_string_lossy();
        if tombstone_filter.is_tombstoned(session_id, &path_str) {
            continue;
        }

        let file_metadata = match fs::metadata(&session_path) {
            Ok(metadata) if metadata.is_file() => metadata,
            _ => continue,
        };
        let file_path = session_path.to_str().unwrap_or("").to_string();
        seen_paths.insert(file_path.clone());
        let Some(metadata) = resolve_session_index_payload(
            &session_path,
            &file_path,
            &file_metadata,
            &cached_index,
            &mut index_updates,
            &mut invalid_index_paths,
        ) else {
            continue;
        };

        let session_id = metadata.session_id;
        // 没有项目路径就是没有：`None` 既是分组键也是展示值的「缺席」，兜底句由前端渲染。
        let original_path = metadata.project_path.filter(|path| !path.trim().is_empty());
        let encoded_dir = original_path.clone();
        let title = metadata
            .title
            .as_deref()
            .or_else(|| codex_titles.get(&session_id).map(String::as_str))
            .or_else(|| {
                session_path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .and_then(|stem| codex_titles.get(stem).map(String::as_str))
            });
        let display_name = crate::title_resolver::resolve_display_name(
            crate::commands::session::custom_session_name(custom_names, CliKind::Codex, &file_path),
            title,
            metadata.first_user_message.as_deref(),
            session_map
                .get(&session_id)
                .map(|h| h.display.as_str())
                .filter(|d| !d.is_empty()),
            &session_id,
        );
        let timestamp = metadata
            .last_timestamp
            .or(metadata.first_timestamp)
            .unwrap_or_default();
        let file_size = metadata.file_size;
        let git_branch = metadata.git_branch;
        let cached_record = cached_index.get(&file_path);
        let has_archive_snapshot = cached_record
            .map(|record| record.has_archive_snapshot)
            .unwrap_or(false);
        let is_archived = cached_record
            .map(|record| record.is_archived)
            .unwrap_or(false);

        all_sessions.push(RawSession {
            session: SessionInfo {
                session_id,
                file_path,
                display_name,
                timestamp,
                file_size,
                git_branch,
                has_archive_snapshot,
                is_archived,
                cli_id: CliKind::Codex.id().to_string(),
            },
            encoded_dir,
            original_path,
        });
    }

    persist_session_list_index_changes(
        CliKind::Codex,
        stale_base,
        &seen_paths,
        index_updates,
        &invalid_index_paths,
        // Codex 走 collect_jsonl_files 递归收集，读取失败会冒泡中止整轮扫描，无需逐目录记录
        &[],
    );

    Ok(all_sessions)
}

pub(super) fn scan_gemini_projects(
    custom_names: &HashMap<String, String>,
    force: bool,
) -> AppResult<Vec<RawSession>> {
    let sessions_dir = cli::sessions_dir(CliKind::Gemini)?;
    let data_dir = cli::data_dir(CliKind::Gemini)?;

    if !sessions_dir.exists() {
        return Ok(Vec::new());
    }

    let history_dir = data_dir.join("history");
    let mut project_paths: HashMap<String, String> = HashMap::new();
    if history_dir.exists() {
        if let Ok(entries) = fs::read_dir(&history_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                let project_name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .to_string();
                let root_file = path.join(".project_root");
                if let Ok(root) = fs::read_to_string(&root_file) {
                    let root = root.trim().to_string();
                    if !root.is_empty() {
                        project_paths.insert(project_name, root);
                    }
                }
            }
        }
    }

    let projects_json_path = data_dir.join("projects.json");
    if projects_json_path.exists() {
        if let Ok(content) = fs::read_to_string(&projects_json_path) {
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&content) {
                if let Some(projects) = parsed.get("projects").and_then(|v| v.as_object()) {
                    for (path, name) in projects {
                        if let Some(name_str) = name.as_str() {
                            project_paths
                                .entry(name_str.to_string())
                                .or_insert(path.clone());
                        }
                    }
                }
            }
        }
    }

    // force 模式下不使用缓存索引，所有文件重新解析
    // force 模式下不使用缓存索引，所有文件重新解析
    let cached_index = if force {
        HashMap::new()
    } else {
        load_session_list_index(CliKind::Gemini)
    };
    // 陈旧行判定始终以「库里已知的行」为基准：force 清空缓存只是不复用元数据，
    // 不能连带让清理失去比对基准 —— 否则强制刷新时陈旧行会永久滞留在列表里
    // （数据源切换、会话删除后都清不掉）。非 force 时两者同源，不额外读库。
    let forced_known_index = if force {
        load_session_list_index(CliKind::Gemini)
    } else {
        HashMap::new()
    };
    let stale_base = if force { &forced_known_index } else { &cached_index };
    let mut index_updates = Vec::new();
    let mut seen_paths = HashSet::new();
    // 本轮读取失败（权限/IO）的子树：其下已有记录保留原样，不按"已删除"处理
    let mut unverified_prefixes: Vec<PathBuf> = Vec::new();
    let mut invalid_index_paths = HashSet::new();
    let mut all_sessions = Vec::new();
    let tombstone_filter = app_db::load_tombstone_filter(CliKind::Gemini).unwrap_or_default();

    let entries = fs::read_dir(&sessions_dir)?;
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => continue,
        };
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let project_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();
        let chats_dir = path.join("chats");
        if !chats_dir.exists() {
            continue;
        }

        let original_path = project_paths
            .get(&project_name)
            .cloned()
            .unwrap_or_else(|| project_name.clone());
        let encoded_dir = original_path.clone();

        let Some(chat_entries) =
            read_dir_or_record_unverified(&chats_dir, &mut unverified_prefixes)
        else {
            continue;
        };

        for session_entry in chat_entries {
            let session_entry = match session_entry {
                Ok(entry) => entry,
                Err(_) => continue,
            };
            let session_path = session_entry.path();
            let file_name = session_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("");
            if !file_name.ends_with(".jsonl") {
                continue;
            }

            // 双轨墓碑拦截：若会话已被标记删除，拦截重新索引，防止死灰复燃
            let session_id = session_path.file_stem().and_then(|n| n.to_str()).unwrap_or("");
            let path_str = session_path.to_string_lossy();
            if tombstone_filter.is_tombstoned(session_id, &path_str) {
                continue;
            }

            let file_metadata = match fs::metadata(&session_path) {
                Ok(metadata) if metadata.is_file() => metadata,
                _ => continue,
            };
            let file_path = session_path.to_str().unwrap_or("").to_string();
            seen_paths.insert(file_path.clone());
            let Some(mut metadata) = resolve_session_index_payload(
                &session_path,
                &file_path,
                &file_metadata,
                &cached_index,
                &mut index_updates,
                &mut invalid_index_paths,
            ) else {
                continue;
            };

            let resolved_project_path = Some(original_path.clone());
            if metadata.project_path != resolved_project_path {
                metadata.project_path = resolved_project_path;
                index_updates.push(build_session_list_index_record(
                    &file_path,
                    file_modified_ms(&file_metadata),
                    &metadata,
                ));
            }

            let session_id = metadata.session_id;
            let display_name = crate::title_resolver::resolve_display_name(
                crate::commands::session::custom_session_name(custom_names, CliKind::Gemini, &file_path),
                metadata.title.as_deref(),
                metadata.first_user_message.as_deref(),
                None,
                &session_id,
            );
            let timestamp = metadata
                .last_timestamp
                .or(metadata.first_timestamp)
                .unwrap_or_default();
            let file_size = metadata.file_size;
            let cached_record = cached_index.get(&file_path);
            let has_archive_snapshot = cached_record
                .map(|record| record.has_archive_snapshot)
                .unwrap_or(false);
            let is_archived = cached_record
                .map(|record| record.is_archived)
                .unwrap_or(false);

            all_sessions.push(RawSession {
                session: SessionInfo {
                    session_id,
                    file_path,
                    display_name,
                    timestamp,
                    file_size,
                    git_branch: String::new(),
                    has_archive_snapshot,
                    is_archived,
                    cli_id: CliKind::Gemini.id().to_string(),
                },
                encoded_dir: Some(encoded_dir.clone()),
                original_path: Some(original_path.clone()),
            });
        }
    }

    persist_session_list_index_changes(
        CliKind::Gemini,
        stale_base,
        &seen_paths,
        index_updates,
        &invalid_index_paths,
        &unverified_prefixes,
    );

    Ok(all_sessions)
}

pub(super) fn scan_workbuddy_projects(
    custom_names: &HashMap<String, String>,
    force: bool,
) -> AppResult<Vec<RawSession>> {
    let sessions_dir = cli::sessions_dir(CliKind::WorkBuddy)?;

    if !sessions_dir.exists() {
        return Ok(Vec::new());
    }

    // force 模式下不使用缓存索引，所有文件重新解析
    // force 模式下不使用缓存索引，所有文件重新解析
    let cached_index = if force {
        HashMap::new()
    } else {
        load_session_list_index(CliKind::WorkBuddy)
    };
    // 陈旧行判定始终以「库里已知的行」为基准：force 清空缓存只是不复用元数据，
    // 不能连带让清理失去比对基准 —— 否则强制刷新时陈旧行会永久滞留在列表里
    // （数据源切换、会话删除后都清不掉）。非 force 时两者同源，不额外读库。
    let forced_known_index = if force {
        load_session_list_index(CliKind::WorkBuddy)
    } else {
        HashMap::new()
    };
    let stale_base = if force { &forced_known_index } else { &cached_index };
    let mut index_updates = Vec::new();
    let mut seen_paths = HashSet::new();
    // 本轮读取失败（权限/IO）的子树：其下已有记录保留原样，不按"已删除"处理
    let mut unverified_prefixes: Vec<PathBuf> = Vec::new();
    let mut invalid_index_paths = HashSet::new();
    let mut all_sessions = Vec::new();
    let tombstone_filter = app_db::load_tombstone_filter(CliKind::WorkBuddy).unwrap_or_default();
    // 单次读取 WorkBuddy 用户重命名 Map（workbuddy.db 的 sessions.custom_title），整批 O(1) 命中
    let wb_titles = parser::load_workbuddy_custom_titles();

    let entries = fs::read_dir(&sessions_dir)?;
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => continue,
        };
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let project_slug = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();

        let Some(session_entries) = read_dir_or_record_unverified(&path, &mut unverified_prefixes)
        else {
            continue;
        };

        for session_entry in session_entries {
            let session_entry = match session_entry {
                Ok(entry) => entry,
                Err(_) => continue,
            };
            let session_path = session_entry.path();
            let file_name = session_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("");
            if !file_name.ends_with(".jsonl") {
                continue;
            }

            // 双轨墓碑拦截：若会话已被标记删除，拦截重新索引，防止死灰复燃
            let session_id = session_path.file_stem().and_then(|n| n.to_str()).unwrap_or("");
            let path_str = session_path.to_string_lossy();
            if tombstone_filter.is_tombstoned(session_id, &path_str) {
                continue;
            }

            let file_metadata = match fs::metadata(&session_path) {
                Ok(metadata) if metadata.is_file() => metadata,
                _ => continue,
            };
            let file_path = session_path.to_str().unwrap_or("").to_string();
            seen_paths.insert(file_path.clone());
            let Some(mut metadata) = resolve_session_index_payload(
                &session_path,
                &file_path,
                &file_metadata,
                &cached_index,
                &mut index_updates,
                &mut invalid_index_paths,
            ) else {
                continue;
            };

            // 权威项目路径来自每行 JSONL 的 cwd；兜底按 slug 解码
            let original_path = metadata
                .project_path
                .clone()
                .filter(|p| !p.trim().is_empty())
                .or_else(|| session_mod::resolve_project_path(&project_slug, None, None));
            let encoded_dir = original_path.clone();

            if metadata.project_path != original_path {
                metadata.project_path = original_path.clone();
                index_updates.push(build_session_list_index_record(
                    &file_path,
                    file_modified_ms(&file_metadata),
                    &metadata,
                ));
            }

            let session_id = metadata.session_id;
            let display_name = crate::title_resolver::resolve_display_name(
                crate::commands::session::custom_session_name(custom_names, CliKind::WorkBuddy, &file_path),
                wb_titles
                    .get(&session_id)
                    .map(String::as_str)
                    .or(metadata.title.as_deref()),
                metadata.first_user_message.as_deref(),
                None,
                &session_id,
            );
            let timestamp = metadata
                .last_timestamp
                .or(metadata.first_timestamp)
                .unwrap_or_default();
            let file_size = metadata.file_size;
            let cached_record = cached_index.get(&file_path);
            let has_archive_snapshot = cached_record
                .map(|record| record.has_archive_snapshot)
                .unwrap_or(false);
            let is_archived = cached_record
                .map(|record| record.is_archived)
                .unwrap_or(false);

            all_sessions.push(RawSession {
                session: SessionInfo {
                    session_id,
                    file_path,
                    display_name,
                    timestamp,
                    file_size,
                    git_branch: String::new(),
                    has_archive_snapshot,
                    is_archived,
                    cli_id: CliKind::WorkBuddy.id().to_string(),
                },
                encoded_dir: encoded_dir.clone(),
                original_path: original_path.clone(),
            });
        }
    }

    persist_session_list_index_changes(
        CliKind::WorkBuddy,
        stale_base,
        &seen_paths,
        index_updates,
        &invalid_index_paths,
        &unverified_prefixes,
    );

    Ok(all_sessions)
}

/// 从 DSH 会话头行构建列表索引元数据。
/// 权威项目路径 = header `cwd`（缺失时不视为无效，由调用侧兜底解码目录名）；
/// `title` 恒为 None（扫描期不读日志尾部的 `session/title` LLM 标题），展示名经
/// title_resolver 回退到清洗后的首条用户消息（与 `scan_session_metadata_only` 一致，
/// 快照加载与实时扫描共用同一解析链）。
fn build_dsh_session_metadata(
    header: &Value,
    session_path: &Path,
    file_metadata: &fs::Metadata,
) -> Option<parser::SessionListMetadata> {
    let cwd = crate::parser::dsh::parse_session_header_value(header)
        .map(|(cwd, _, _)| cwd)
        .filter(|cwd| !cwd.trim().is_empty());
    let session_id = header
        .get("id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            session_path
                .parent()
                .and_then(|parent| parent.file_name())
                .and_then(|name| name.to_str())
                .map(str::to_string)
        })?;
    let timestamp = header
        .get("createdAt")
        .and_then(Value::as_i64)
        .or_else(|| Some(file_modified_ms(file_metadata)))
        .and_then(chrono::DateTime::from_timestamp_millis)
        .map(|dt| dt.to_rfc3339());
    let first_user_message =
        crate::parser::dsh::read_first_user_message(&session_path.to_string_lossy())
            .and_then(|text| crate::title_resolver::clean_fallback_user_text(&text));
    Some(parser::SessionListMetadata {
        session_id,
        project_path: cwd,
        title: None,
        first_user_message,
        first_timestamp: timestamp.clone(),
        last_timestamp: timestamp,
        git_branch: String::new(),
        file_size: file_metadata.len(),
    })
}

/// 会话日志换代后的路径归一：把库里挂在旧代路径上的记录改写到当前代路径，并返回这些记录
/// 在新路径下的副本，让本轮扫描按新路径命中既有记录（从而保留归档标记等展示状态）。
///
/// 需要搬迁的判据取自数据库里已入库的路径集合，而不是本轮是否启用了元数据缓存：改过
/// 推导规则后的首轮扫描会被派生规则版本闸门强制成全量（缓存为空），若只看缓存就永远发现
/// 不了需要搬迁的旧路径 —— 首轮会按新路径插一条新记录、旧路径的记录因为带归档快照被保留，
/// 直接表现为列表里同一会话出现两次。
///
/// 搬迁失败时返回空表，本轮按当前代路径当作新会话落库 —— 会话可见性永远不依赖搬迁成功，
/// 失败的陈旧记录留待下一轮扫描重试。
fn rekey_dsh_session_paths(
    cached_index: &HashMap<String, app_db::SessionListIndexRecord>,
    indexed_paths: &HashSet<String>,
    session_id: &str,
    session_dir: &str,
    candidates: &[PathBuf],
) -> HashMap<String, app_db::SessionListIndexRecord> {
    let Some(current_path) = candidates.first().and_then(|path| path.to_str()) else {
        return HashMap::new();
    };
    let retired: Vec<String> = candidates
        .iter()
        .skip(1)
        .filter_map(|path| path.to_str())
        .filter(|path| indexed_paths.contains(*path))
        .map(str::to_string)
        .collect();
    if retired.is_empty() {
        return HashMap::new();
    }

    let moves: Vec<(String, String)> = retired
        .iter()
        .map(|old| (old.clone(), current_path.to_string()))
        .collect();
    match app_db::rekey_session_paths(CliKind::Dsh, &moves) {
        Ok(moved) => tracing::info!(
            "DSH 会话 {} 日志换代，搬迁 {} 条路径记录至当前代: dir={}, current={}",
            session_id,
            moved,
            session_dir,
            current_path
        ),
        Err(err) => {
            tracing::warn!(
                "DSH 会话路径搬迁失败，本轮按当前代单独落库: session={}, dir={}, err={}",
                session_id,
                session_dir,
                err.diagnostic()
            );
            return HashMap::new();
        }
    }

    // 返回搬迁后记录在新路径下的副本：搬迁过的记录其大小/修改时间仍属于旧代文件，元数据
    // 缓存必然落空并触发重解析（这正是我们要的），但归档标记这类展示状态要跟着走到新路径
    retired
        .iter()
        .filter_map(|old| {
            cached_index
                .get(old)
                .map(|record| (current_path.to_string(), record.clone()))
        })
        .collect()
}

pub(super) fn scan_dsh_projects(
    custom_names: &HashMap<String, String>,
    force: bool,
) -> AppResult<Vec<RawSession>> {
    let sessions_dir = cli::sessions_dir(CliKind::Dsh)?;

    if !sessions_dir.exists() {
        return Ok(Vec::new());
    }

    // 陈旧行判定始终以「库里已知的行」为基准：force 清空缓存只是不复用元数据，
    // 不能连带让清理失去比对基准 —— 否则强制刷新时陈旧行会永久滞留在列表里
    // （数据源切换、会话删除后都清不掉）。非 force 时两者同源，不额外读库。
    // 这份基准同时供路径归一使用：规则变更后的首轮扫描被派生规则版本闸门强制成全量，
    // 缓存是空的，若只看缓存首轮就发现不了待搬迁的旧代路径。
    let cached_index = if force {
        HashMap::new()
    } else {
        load_session_list_index(CliKind::Dsh)
    };
    let forced_known_index = if force {
        load_session_list_index(CliKind::Dsh)
    } else {
        HashMap::new()
    };
    let stale_base = if force { &forced_known_index } else { &cached_index };
    let indexed_paths: HashSet<String> = stale_base.keys().cloned().collect();
    // 本轮发生换代搬迁的记录，按新路径存放：搬迁过的记录已不在 cached_index 的旧键下，
    // 归档标记等展示状态要能从新路径查到
    let mut rekeyed_records: HashMap<String, app_db::SessionListIndexRecord> = HashMap::new();
    let tombstone_filter = app_db::load_tombstone_filter(CliKind::Dsh).unwrap_or_default();
    let mut index_updates = Vec::new();
    let mut seen_paths = HashSet::new();
    // 本轮读取失败（权限/IO）的子树：其下已有记录保留原样，不按"已删除"处理
    let mut unverified_prefixes: Vec<PathBuf> = Vec::new();
    let mut invalid_index_paths = HashSet::new();
    let mut all_sessions = Vec::new();

    let entries = fs::read_dir(&sessions_dir)?;
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => continue,
        };
        let project_path = entry.path();
        if !project_path.is_dir() {
            continue;
        }

        let encoded_dir_name = project_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("")
            .to_string();
        let fallback_project_path =
            session_mod::resolve_project_path(&encoded_dir_name, None, None);

        let Some(session_entries) =
            read_dir_or_record_unverified(&project_path, &mut unverified_prefixes)
        else {
            continue;
        };

        for session_entry in session_entries {
            let session_entry = match session_entry {
                Ok(entry) => entry,
                Err(_) => continue,
            };
            let session_path = session_entry.path();
            if !session_path.is_dir() {
                continue;
            }

            let session_id = session_path.file_stem().and_then(|n| n.to_str()).unwrap_or("");
            let dir_str = session_path.to_string_lossy();

            // 候选顺位与 Fallback 容错机制：
            // 同一会话目录内可能并存多个「格式代」的日志副本，以及同代内的压缩/明文副本。
            // 按「代次降序 + 同代 zstd 优先」排序，首位即会话当前内容；当前代不可读时顺位
            // 回退到上一代，宁可展示历史内容也不让整条会话从列表里消失。
            let candidates = crate::parser::dsh::session_dir_candidates(&session_path);
            if candidates.is_empty() {
                continue;
            }

            // 双轨墓碑拦截（会话级终态拦截）：若目录或任一物理候选副本已被标记为墓碑，
            // 立即跳过整个会话，严禁向其他候选 Fallback 导致已删会话意外复活。
            //
            // 必须先于路径归一：换代搬迁会改写索引行/归档元数据并物理移动归档快照，
            // 先改状态再判断「该不该改」，会把已删会话的记录搬到墓碑覆盖不到的路径上。
            let is_tombstoned = tombstone_filter.is_tombstoned(session_id, &dir_str)
                || candidates.iter().any(|c| tombstone_filter.is_tombstoned(session_id, &c.to_string_lossy()));
            if is_tombstoned {
                continue;
            }

            // 路径归一：日志换代后当前内容落在新一代文件名上，而库里可能还挂着旧代路径的
            // 记录。不搬迁就会留下一条再也扫不到的陈旧记录 —— 列表里同一会话出现两次，
            // 且归档快照与活动条目失联。搬迁后一个会话目录只对应新路径上的唯一一条记录。
            rekeyed_records.extend(rekey_dsh_session_paths(
                &cached_index,
                &indexed_paths,
                &session_id,
                &dir_str,
                &candidates,
            ));

            enum DshCandidateOutcome {
                Valid {
                    file_path: String,
                    modified_ms: i64,
                    metadata: parser::SessionListMetadata,
                    is_new_record: bool,
                },
                SkipSession,
            }

            let picked = pick_first_valid_candidate(&candidates, |candidate| {
                let meta = match fs::metadata(candidate) {
                    Ok(m) if m.is_file() && m.len() > 0 => m,
                    _ => return None,
                };
                let file_path = candidate.to_str()?.to_string();
                let file_size = meta.len();
                let modified_ms = file_modified_ms(&meta);

                if let Some(m) = cached_session_list_metadata(&cached_index, &file_path, file_size, modified_ms) {
                    return Some(DshCandidateOutcome::Valid {
                        file_path,
                        modified_ms,
                        metadata: m,
                        is_new_record: false,
                    });
                }

                let header = match crate::parser::dsh::read_header_line(candidate) {
                    Some(h) => h,
                    None => return None,
                };

                // 子代理会话与无 surface 消息的空会话属于会话级正常业务过滤，
                // 短路跳过，不判定为文件损坏，不输出误导性告警日志
                if header.get("origin").and_then(Value::as_str) == Some("subagent")
                    || !crate::parser::dsh::has_surface_messages(candidate)
                {
                    return Some(DshCandidateOutcome::SkipSession);
                }

                let m = match build_dsh_session_metadata(&header, candidate, &meta) {
                    Some(m) => m,
                    None => return None,
                };

                Some(DshCandidateOutcome::Valid {
                    file_path,
                    modified_ms,
                    metadata: m,
                    is_new_record: true,
                })
            });

            let (file_path, modified_ms, mut metadata, is_new_record) = match picked {
                Some(DshCandidateOutcome::Valid { file_path, modified_ms, metadata, is_new_record }) => {
                    (file_path, modified_ms, metadata, is_new_record)
                }
                Some(DshCandidateOutcome::SkipSession) => {
                    continue;
                }
                None => {
                    for c in &candidates {
                        if let Some(p) = c.to_str() {
                            invalid_index_paths.insert(p.to_string());
                        }
                    }
                    continue;
                }
            };

            seen_paths.insert(file_path.clone());

            // 权威项目路径来自 header cwd；兜底按项目目录名解码
            let original_path = metadata
                .project_path
                .clone()
                .filter(|value| !value.trim().is_empty())
                .or_else(|| fallback_project_path.clone());
            // header cwd 缺失（用目录名解码兜底）时，把解析出的项目路径回写进持久化索引，
            // 否则快照加载（读索引）会退回 session_id 作项目 key，与实时扫描分组分叉。
            let mut should_persist = is_new_record;
            if metadata.project_path != original_path {
                metadata.project_path = original_path.clone();
                should_persist = true;
            }
            if should_persist {
                index_updates.push(build_session_list_index_record(
                    &file_path,
                    modified_ms,
                    &metadata,
                ));
            }
            // 项目分组 key 用权威 cwd（与快照加载路径一致）
            let encoded_dir = original_path.clone();
            let session_id = metadata.session_id.clone();
            let display_name = crate::title_resolver::resolve_display_name(
                crate::commands::session::custom_session_name(custom_names, CliKind::Dsh, &file_path),
                metadata.title.as_deref(),
                metadata.first_user_message.as_deref(),
                None,
                &session_id,
            );
            let timestamp = metadata
                .last_timestamp
                .or(metadata.first_timestamp)
                .unwrap_or_default();
            let file_size = metadata.file_size;
            let cached_record = cached_index
                .get(&file_path)
                .or_else(|| rekeyed_records.get(&file_path));
            let has_archive_snapshot = cached_record
                .map(|record| record.has_archive_snapshot)
                .unwrap_or(false);
            let is_archived = cached_record
                .map(|record| record.is_archived)
                .unwrap_or(false);

            all_sessions.push(RawSession {
                session: SessionInfo {
                    session_id,
                    file_path,
                    display_name,
                    timestamp,
                    file_size,
                    git_branch: String::new(),
                    has_archive_snapshot,
                    is_archived,
                    cli_id: CliKind::Dsh.id().to_string(),
                },
                encoded_dir,
                original_path,
            });
        }
    }

    persist_session_list_index_changes(
        CliKind::Dsh,
        stale_base,
        &seen_paths,
        index_updates,
        &invalid_index_paths,
        &unverified_prefixes,
    );

    Ok(all_sessions)
}

pub(super) fn scan_antigravity_projects(
    custom_names: &HashMap<String, String>,
    force: bool,
) -> AppResult<Vec<RawSession>> {
    let brain_dir = cli::sessions_dir(CliKind::Antigravity)?;

    if !brain_dir.exists() {
        return Ok(Vec::new());
    }

    // force 模式下不使用缓存索引，所有文件重新解析
    let cached_index = if force {
        HashMap::new()
    } else {
        load_session_list_index(CliKind::Antigravity)
    };
    // 陈旧行判定始终以「库里已知的行」为基准：force 清空缓存只是不复用元数据，
    // 不能连带让清理失去比对基准 —— 否则强制刷新时陈旧行会永久滞留在列表里
    // （数据源切换、会话删除后都清不掉）。非 force 时两者同源，不额外读库。
    let forced_known_index = if force {
        load_session_list_index(CliKind::Antigravity)
    } else {
        HashMap::new()
    };
    let stale_base = if force { &forced_known_index } else { &cached_index };
    let tombstone_filter = app_db::load_tombstone_filter(CliKind::Antigravity).unwrap_or_default();
    let mut index_updates = Vec::new();
    let mut seen_paths = HashSet::new();
    let mut invalid_index_paths = HashSet::new();
    let mut all_sessions = Vec::new();

    let mut jsonl_files = Vec::new();
    collect_jsonl_files(&brain_dir, &mut jsonl_files)?;

    // 命名漂移检测：只告警、不放宽匹配。日志目录里存在 jsonl 却没有会话正本名，说明上游
    // 很可能改了命名 —— 把"会话静默消失"变成日志里当天可见的信号，而不是等用户来问
    for (dir, actual_names) in antigravity_transcript_naming_drift(&jsonl_files) {
        tracing::warn!(
            "Antigravity 日志目录中未找到会话正本 {}，实际文件: {:?}；上游可能改了命名，该目录下的会话本轮不会收录: dir={:?}",
            ANTIGRAVITY_TRANSCRIPT_FILE_NAME,
            actual_names,
            dir
        );
    }

    for session_path in jsonl_files {
        let file_name = session_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");
        if file_name != ANTIGRAVITY_TRANSCRIPT_FILE_NAME {
            continue;
        }

        // 双轨墓碑拦截：若会话已被标记删除，拦截重新索引，防止死灰复燃。
        // 注意：Antigravity 文件的文件名恒为 transcript.jsonl，严禁将其 stem ("transcript")
        // 作为 session_id 查询，必须提取真实 conv_id 或直接匹配文件路径。
        let conv_id = crate::parser::antigravity::extract_conversation_id(&session_path);
        let path_str = session_path.to_string_lossy();
        let conv_id_str = conv_id.as_deref().unwrap_or("");
        if tombstone_filter.is_tombstoned(conv_id_str, &path_str) {
            continue;
        }

        let file_metadata = match fs::metadata(&session_path) {
            Ok(metadata) if metadata.is_file() => metadata,
            _ => continue,
        };
        let file_path = session_path.to_str().unwrap_or("").to_string();

        if parser::is_subagent_session(&file_path) {
            continue;
        }

        seen_paths.insert(file_path.clone());

        let Some(mut metadata) = resolve_session_index_payload(
            &session_path,
            &file_path,
            &file_metadata,
            &cached_index,
            &mut index_updates,
            &mut invalid_index_paths,
        ) else {
            continue;
        };

        let original_path = metadata
            .project_path
            .clone()
            .filter(|p| !p.trim().is_empty());
        let encoded_dir = original_path.clone();

        if metadata.project_path != original_path {
            metadata.project_path = original_path.clone();
            index_updates.push(build_session_list_index_record(
                &file_path,
                file_modified_ms(&file_metadata),
                &metadata,
            ));
        }

        let session_id = metadata.session_id;
        let display_name = crate::title_resolver::resolve_display_name(
            crate::commands::session::custom_session_name(custom_names, CliKind::Antigravity, &file_path),
            metadata.title.as_deref(),
            metadata.first_user_message.as_deref(),
            None,
            &session_id,
        );
        let timestamp = metadata
            .last_timestamp
            .or(metadata.first_timestamp)
            .unwrap_or_default();
        let file_size = metadata.file_size;
        let cached_record = cached_index.get(&file_path);
        let has_archive_snapshot = cached_record
            .map(|record| record.has_archive_snapshot)
            .unwrap_or(false);
        let is_archived = cached_record
            .map(|record| record.is_archived)
            .unwrap_or(false);

        all_sessions.push(RawSession {
            session: SessionInfo {
                session_id,
                file_path,
                display_name,
                timestamp,
                file_size,
                git_branch: String::new(),
                has_archive_snapshot,
                is_archived,
                cli_id: CliKind::Antigravity.id().to_string(),
            },
            encoded_dir,
            original_path,
        });
    }

    persist_session_list_index_changes(
        CliKind::Antigravity,
        stale_base,
        &seen_paths,
        index_updates,
        &invalid_index_paths,
        // 同 Codex：走 collect_jsonl_files 递归收集，读取失败会冒泡中止整轮扫描
        &[],
    );

    Ok(all_sessions)
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

fn collect_jsonl_files(dir: &Path, files: &mut Vec<PathBuf>) -> AppResult<()> {
    let entries = fs::read_dir(dir)?;
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => continue,
        };
        let path = entry.path();
        if path.is_dir() {
            collect_jsonl_files(&path, files)?;
            continue;
        }
        if path.extension().and_then(|ext| ext.to_str()) == Some("jsonl") {
            files.push(path);
        }
    }
    Ok(())
}
