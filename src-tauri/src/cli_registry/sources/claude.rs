use crate::app_db;
use crate::cli::CliKind;
use crate::cli_config::{
    ensure_parent_dir, read_json_file, settings_path_for, strip_bom, write_atomically,
    write_json_file,
};
use crate::commands::profile::{
    build_claude_temp_settings_file, clean_claude_settings_for_unbind, ensure_json_object,
    error_reason, normalized_json_leaf_pointers, partial_dir_save_failed,
    remove_json_pointer_value, set_json_pointer_value, CLAUDE_MANAGED_PROFILE_PATHS,
};
use crate::commands::session::{build_claude_search_results, AggregatedSessionSearch};
use crate::commands::session_index::{
    build_projects_scan, build_projects_snapshot, cached_session_list_metadata, file_modified_ms,
    RawSession, Resolution, ScanProjection, SnapshotProjection,
};
use crate::error::{AppError, AppResult};
use crate::history;
use crate::parser;
use crate::parser::claude;
use crate::parser::shared::{SearchDocument, SearchScanProgress};
use crate::session as session_mod;
use crate::session::{
    ChatMessage, PaginatedProjects, SearchResult, SessionListMetadata, SessionLoadResult,
    SubagentInfo, UsageRecord,
};
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use super::super::descriptor::{CliDescriptor, FileRoot};
use super::super::features::{
    ApiProfileFeature, LaunchFeature, LaunchPlan, LaunchRequest, ProfileScopeCtx,
    ScopeApplyOutcome, StoredProfile,
};
use super::super::source::{
    file_backed_exists, read_dir_or_record_unverified, under_data_root, CliSource, Truncation,
};
use super::super::{Discovered, SessionLocator};

pub(crate) struct ClaudeSource;

/// 元数据与归属判定、启动计划同处一个文件：改这个 CLI 不必再去别处同步一张表。
static DESCRIPTOR: CliDescriptor = CliDescriptor {
    name: "Claude Code",
    tray_label: "Claude",
    proxy_port: 18080,
    command: "claude",
    file_root: FileRoot { data_dir_segments: &[".claude"], sessions_subdir: Some("projects") },
};

/// Claude 的配置文件名与实现同处一文件：能力位由「有无实现」对账，不必另立清单。
static CLAUDE_CONFIG_FILE: ClaudeConfigFile = ClaudeConfigFile;
struct ClaudeConfigFile;
impl crate::cli_registry::features::ConfigFileFeature for ClaudeConfigFile {
    fn settings_file_name(&self) -> &'static str {
        "settings.json"
    }
}

/// Claude 的 API 代理能力与实现同处一文件：能力位由「有无实现」对账，不必另立清单。
static CLAUDE_API_PROXY: ClaudeApiProxy = ClaudeApiProxy;
struct ClaudeApiProxy;
impl crate::cli_registry::features::ApiProxyFeature for ClaudeApiProxy {
    fn default_base_url(&self) -> String {
        "https://api.anthropic.com".to_string()
    }

    fn base_url(&self) -> AppResult<String> {
        let settings = crate::cli_config::read_json_file(
            &crate::cli_config::settings_path_for(CliKind::Claude)?,
            "{}",
        )?;
        Ok(settings
            .get("env")
            .and_then(|env| env.get("ANTHROPIC_BASE_URL"))
            .and_then(|v| v.as_str())
            .unwrap_or("https://api.anthropic.com")
            .to_string())
    }

    fn set_base_url(&self, url: &str) -> AppResult<()> {
        let path = crate::cli_config::settings_path_for(CliKind::Claude)?;
        let mut settings = crate::cli_config::read_json_file(&path, "{}")?;
        let env = settings
            .as_object_mut()
            .ok_or(crate::error::AppError::coded("proxy.settings_not_object"))?
            .entry("env")
            .or_insert_with(|| serde_json::json!({}));
        env.as_object_mut()
            .ok_or(crate::error::AppError::coded("proxy.env_not_object"))?
            .insert(
                "ANTHROPIC_BASE_URL".to_string(),
                serde_json::Value::String(url.to_string()),
            );
        crate::cli_config::write_json_file(&path, &settings)
    }

    fn traffic_path_filter(&self) -> &'static str {
        "(path LIKE '/v1/messages%' AND path NOT LIKE '/v1/messages/count_tokens%')"
    }

    fn session_display_maps(
        &self,
    ) -> AppResult<(HashMap<String, String>, HashMap<String, Option<String>>)> {
        let history_path = crate::cli::history_path(CliKind::Claude)?;
        // history.jsonl 只解析一次，同时拿到 display 兜底 map 与项目路径 map
        // （此前 Claude 分支解析了两遍，sessions 全量拉取时 CPU 翻倍）
        let (history_map, history_project_map) =
            history::parse_history(history_path.to_str().unwrap_or(""));
        let mut project_map = crate::proxy::build_claude_session_project_map(
            &crate::cli::sessions_dir(CliKind::Claude)?,
            &history_project_map,
        );

        // 会话列表索引库：自定义重命名 / 原生标题 / 清洗后首条用户消息 / 项目路径
        let records = crate::app_db::read_session_list_index(CliKind::Claude).unwrap_or_default();
        let custom_names = crate::app_db::load_session_names().unwrap_or_default();
        let mut display_map = HashMap::new();
        for record in records.values() {
            let sid = record.session_id.trim();
            if sid.is_empty() {
                continue;
            }
            let display = crate::title_resolver::resolve_display_name(
                crate::commands::session::custom_session_name(
                    &custom_names,
                    CliKind::Claude,
                    &record.session_path,
                ),
                record.title.as_deref(),
                record.first_user_message.as_deref(),
                history_map
                    .get(sid)
                    .map(|h| h.display.as_str())
                    .filter(|v| !v.is_empty()),
                sid,
            );
            display_map.insert(sid.to_string(), display);
            if let Some(p) = record.project_path.as_ref().filter(|p| !p.trim().is_empty()) {
                project_map.insert(sid.to_string(), Some(p.clone()));
            }
        }

        // 兜底：对仅在 history_map 中出现（索引库尚未收录）的 session，使用 history display 解析
        for (sid, hist) in &history_map {
            if !display_map.contains_key(sid) {
                let display = crate::title_resolver::resolve_display_name(
                    None,
                    None,
                    None,
                    Some(hist.display.as_str()).filter(|v| !v.is_empty()),
                    sid,
                );
                display_map.insert(sid.clone(), display);
            }
        }

        Ok((display_map, project_map))
    }
}

/// Claude 的分叉能力与实现同处一文件：能力位由「有无实现」对账，不必另立清单。
static CLAUDE_FORK: ClaudeFork = ClaudeFork;
struct ClaudeFork;
impl crate::cli_registry::features::ForkFeature for ClaudeFork {
    /// 把 /rewind 的代码回滚快照（`~/.claude/file-history/<旧id>/`）递归拷到新 id 下，
    /// 让分叉出的会话也能回滚代码。目录不存在或拷贝失败都不阻断分叉，只记日志。
    fn after_fork(&self, old_id: &str, new_id: &str) {
        // 取不到数据目录就什么都不做，与改动前 `if let (... Ok(data_dir))` 的语义一致。
        let Ok(data_dir) = crate::cli::data_dir(CliKind::Claude) else {
            return;
        };
        let src_history = data_dir.join("file-history").join(old_id);
        if src_history.is_dir() {
            let dst_history = data_dir.join("file-history").join(new_id);
            if let Err(e) =
                crate::commands::session::copy_dir_recursive(&src_history, &dst_history)
            {
                eprintln!("[fork_session] file-history copy failed: {}", e);
            }
        }
    }
}

/// Claude 的用量统计能力与实现同处一文件：能力位由「有无实现」对账，不必另立清单。
static CLAUDE_USAGE_STATS: ClaudeUsageStats = ClaudeUsageStats;
struct ClaudeUsageStats;
impl crate::cli_registry::features::UsageStatsFeature for ClaudeUsageStats {
    fn usage_records(&self, key: &str, project: &str) -> Vec<UsageRecord> {
        claude::extract_usage_records(key, project)
    }
}

/// Claude 的配置档托管能力与实现同处一文件：能力位由「有无实现」对账，不必另立清单。
static CLAUDE_API_PROFILE: ClaudeApiProfile = ClaudeApiProfile;
struct ClaudeApiProfile;

/// 把一份配置写进某个 settings.json：只覆盖受管路径，其余字段原样保留。
///
/// 受管路径为空时退回「按内容叶子路径覆盖」——那是旧档没有记录 managedPaths 时的兼容行为。
fn apply_claude_settings_value_to_path(
    settings_path: &PathBuf,
    content: &Value,
    managed_paths: &[String],
) -> AppResult<()> {
    ensure_parent_dir(settings_path)?;

    let mut existing = if settings_path.exists() {
        read_json_file(settings_path, "{}")?
    } else {
        Value::Object(Map::new())
    };
    ensure_json_object(&mut existing);

    let paths = if managed_paths.is_empty() {
        normalized_json_leaf_pointers(content)
    } else {
        managed_paths.to_vec()
    };

    for path in paths {
        match content.pointer(&path) {
            Some(value) => set_json_pointer_value(&mut existing, &path, value.clone()),
            None => remove_json_pointer_value(&mut existing, &path),
        }
    }

    write_json_file(settings_path, &existing)
}

impl ApiProfileFeature for ClaudeApiProfile {
    fn allowed_paths(&self, _profile: Option<&str>) -> &'static [&'static str] {
        CLAUDE_MANAGED_PROFILE_PATHS
    }

    fn read_settings(&self) -> AppResult<String> {
        let path = settings_path_for(CliKind::Claude)?;
        if !path.exists() {
            return Ok("{}".to_string());
        }
        fs::read_to_string(&path)
            .map(strip_bom)
            .map_err(|e| AppError::coded("profile.settings_read_failed").with("detail", e.to_string()))
    }

    fn write_settings(&self, content: String) -> AppResult<()> {
        // 内容校验先行（与改动前一致）：非法 JSON 在写盘前失败；Claude 不消费解析值。
        let _: Value = serde_json::from_str(&content)
            .map_err(|e| AppError::coded("profile.content_parse_failed").with("detail", e.to_string()))?;
        let path = settings_path_for(CliKind::Claude)?;
        ensure_parent_dir(&path)?;
        write_atomically(&path, content.as_bytes())
    }

    /// 应用一份配置：全局写全局 settings.json，项目作用域逐目录展开。
    ///
    /// 单条路径失败**不冒泡**：写进 `failed_dirs` 计数返回，只有目录列表本身读不到才返回 `Err`。
    fn apply_to_scope(
        &self,
        ctx: &ProfileScopeCtx<'_>,
        stored: &StoredProfile,
    ) -> AppResult<ScopeApplyOutcome> {
        let mut outcome = ScopeApplyOutcome::default();
        if ctx.scope == "global" {
            let settings_path = match ctx.global_override {
                Some(p) => p.to_path_buf(),
                None => settings_path_for(CliKind::Claude)?,
            };
            match apply_claude_settings_value_to_path(&settings_path, &stored.visible, &stored.managed_paths) {
                Ok(()) => outcome.success_count += 1,
                Err(e) => {
                    tracing::warn!(
                        "Failed to cascade apply profile '{}' to global: {}",
                        ctx.profile_name,
                        e.diagnostic()
                    );
                    outcome.failed_dirs.push(settings_path.to_string_lossy().to_string());
                }
            }
        } else {
            let dirs = app_db::get_profile_tab_dirs_with(ctx.conn, ctx.scope)?;
            for dir in dirs {
                let settings_path = PathBuf::from(&dir).join(".claude").join("settings.json");
                match apply_claude_settings_value_to_path(&settings_path, &stored.visible, &stored.managed_paths) {
                    Ok(()) => outcome.success_count += 1,
                    Err(e) => {
                        tracing::warn!(
                            "Failed to cascade apply profile '{}' to dir '{}': {}",
                            ctx.profile_name,
                            dir,
                            e.diagnostic()
                        );
                        outcome.failed_dirs.push(dir);
                    }
                }
            }
        }
        Ok(outcome)
    }

    /// 解绑：清掉该作用域下各目录 settings.json 里的受管字段。尽力而为，错误只忽略。
    fn unbind_scope(&self, ctx: &ProfileScopeCtx<'_>) -> AppResult<()> {
        if let Ok(dirs) = app_db::get_profile_tab_dirs_with(ctx.conn, ctx.scope) {
            for dir in &dirs {
                let settings_path = PathBuf::from(dir).join(".claude").join("settings.json");
                if settings_path.exists() {
                    let _ = clean_claude_settings_for_unbind(&settings_path);
                }
            }
        }
        Ok(())
    }

    /// 绑定：把配置写到作用域。项目作用域要求至少一个目录，否则没有可写目标（错误码不变）。
    fn bind_scope(&self, ctx: &ProfileScopeCtx<'_>, stored: &StoredProfile) -> AppResult<()> {
        if ctx.scope == "global" {
            let settings_path = match ctx.global_override {
                Some(p) => p.to_path_buf(),
                None => settings_path_for(CliKind::Claude)?,
            };
            apply_claude_settings_value_to_path(&settings_path, &stored.visible, &stored.managed_paths)?;
        } else {
            let dirs = app_db::get_profile_tab_dirs_with(ctx.conn, ctx.scope)?;
            if dirs.is_empty() {
                return Err(AppError::coded("profile.project_tab_no_dirs"));
            }
            let mut failures = Vec::new();
            for dir in &dirs {
                let settings_path = PathBuf::from(dir).join(".claude").join("settings.json");
                if let Err(e) = apply_claude_settings_value_to_path(&settings_path, &stored.visible, &stored.managed_paths) {
                    failures.push((dir.clone(), error_reason(&e)));
                }
            }
            if !failures.is_empty() {
                return Err(partial_dir_save_failed(failures));
            }
        }
        Ok(())
    }

    /// Claude 没有由登录态自动生成的受保护档。
    fn protected_profile_name(&self) -> Option<&'static str> {
        None
    }

    /// Claude 的配置按目录级联，项目作用域对它有实际含义。
    fn supports_project_scope(&self) -> bool {
        true
    }

    /// Claude 没有「从 live 反查活动档」的行为，调用方走通用回写路径。
    fn sync_active_from_live(&self) -> AppResult<Option<bool>> {
        Ok(None)
    }

    /// 读作用域设置：全局读全局 settings.json，项目作用域取该 tab 目录下第一个存在的 settings.json。
    fn read_scope_settings(&self, scope: &str) -> AppResult<String> {
        if scope == "global" {
            return self.read_settings();
        }

        let tabs = app_db::list_profile_tabs(CliKind::Claude.id())?;
        let tab = tabs.iter().find(|t| t.id == scope);
        if let Some(t) = tab {
            for dir in &t.dirs {
                let path = PathBuf::from(dir).join(".claude").join("settings.json");
                if path.exists() {
                    if let Ok(content) = fs::read_to_string(&path).map(strip_bom) {
                        return Ok(content);
                    }
                }
            }
        }

        Ok("{}".to_string())
    }

    /// 写作用域设置：全局写全局 settings.json，项目作用域逐目录写并汇总失败目录。
    fn write_scope_settings(&self, scope: &str, content: String) -> AppResult<()> {
        if scope == "global" {
            return self.write_settings(content);
        }

        let value: Value =
            serde_json::from_str(&content).map_err(|e| AppError::coded("profile.content_parse_failed").with("detail", e.to_string()))?;

        let tabs = app_db::list_profile_tabs(CliKind::Claude.id())?;
        let tab = tabs.iter().find(|t| t.id == scope);
        if let Some(t) = tab {
            let mut failures = Vec::new();
            for dir in &t.dirs {
                let path = PathBuf::from(dir).join(".claude").join("settings.json");
                if let Err(e) = write_json_file(&path, &value) {
                    failures.push((dir.clone(), error_reason(&e)));
                }
            }
            if !failures.is_empty() {
                return Err(partial_dir_save_failed(failures));
            }
        }

        Ok(())
    }
}

impl CliSource for ClaudeSource {
    fn descriptor(&self) -> &'static CliDescriptor {
        &DESCRIPTOR
    }

    fn owns(&self, key: &str) -> bool {
        under_data_root(key, CliKind::Claude)
    }

    /// 文件型源：会话身份是一个文件，存在性即文件是否在盘上；虚拟定位符不属于本类源，
    /// 由共享实现判「存在」，以免库型会话被孤儿清理误删。
    fn exists(&self, loc: &SessionLocator) -> bool {
        file_backed_exists(loc)
    }

    /// 文件型源：会话是一个文件，可以移入回收站。
    fn can_delete(&self) -> bool {
        true
    }

    /// 枚举本 CLI 的会话：逐层下降 `projects/<编码目录>/<会话>.jsonl`。
    /// 编码目录读取失败时记入 `unverified_prefixes`，其下既有索引行本轮保留。
    fn discover(&self) -> AppResult<Discovered> {
        let projects_dir = crate::cli::sessions_dir(CliKind::Claude)?;
        if !projects_dir.exists() {
            return Ok(Discovered { locators: Vec::new(), unverified_prefixes: Vec::new() });
        }
        let mut locators = Vec::new();
        let mut unverified_prefixes = Vec::new();
        for entry in fs::read_dir(&projects_dir)? {
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => continue,
            };
            let project_dir = entry.path();
            if !project_dir.is_dir() {
                continue;
            }
            let Some(session_entries) =
                read_dir_or_record_unverified(&project_dir, &mut unverified_prefixes)
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
                locators.push(SessionLocator::File {
                    cli_id: CliKind::Claude,
                    path: session_path,
                });
            }
        }
        Ok(Discovered { locators, unverified_prefixes })
    }

    /// Claude 的分组键取自会话文件的父目录名（索引目录名），项目路径再经 history 映射反解；
    /// 展示名末位的 history 兜底也来自同一份 history。
    fn snapshot(
        &self,
        custom_names: &HashMap<String, String>,
        page: usize,
        page_size: usize,
    ) -> AppResult<Option<PaginatedProjects>> {
        build_projects_snapshot(
            CliKind::Claude,
            custom_names,
            page,
            page_size,
            || {
                let history_path = crate::cli::history_path(CliKind::Claude)?;
                let (session_map, project_map) =
                    crate::history::parse_history(history_path.to_str().unwrap_or(""));
                Ok(move |record: &crate::app_db::SessionListIndexRecord| {
                    let encoded_dir = Path::new(&record.session_path)
                        .parent()
                        .and_then(|parent| parent.file_name())
                        .and_then(|name| name.to_str())
                        .filter(|value| !value.is_empty())
                        .unwrap_or("unknown")
                        .to_string();
                    let original_path = crate::session::resolve_project_path(
                        &encoded_dir,
                        record.project_path.as_deref(),
                        Some(&project_map),
                    );
                    SnapshotProjection {
                        encoded_dir: Some(encoded_dir),
                        original_path,
                        title: record.title.clone(),
                        history_display: session_map
                            .get(&record.session_id)
                            .map(|entry| entry.display.clone())
                            .filter(|display| !display.is_empty()),
                        git_branch: record.git_branch.clone(),
                    }
                })
            },
        )
    }

    fn scan(
        &self,
        custom_names: &HashMap<String, String>,
        force: bool,
    ) -> AppResult<Vec<RawSession>> {
        let projects_dir = crate::cli::sessions_dir(CliKind::Claude)?;
        // 会话根目录不存在与「存在但为空」对陈旧行判定意味着相反的事：根目录缺席说明
        // 数据源整体不可用，此时不该按「都没扫到」清理索引；空目录才是「会话确实没了」。
        if !projects_dir.exists() {
            return Ok(Vec::new());
        }

        let history_path = crate::cli::history_path(CliKind::Claude)?;
        let (session_map, project_map) = history::parse_history(history_path.to_str().unwrap_or(""));
        let discovered = self.discover()?;

        build_projects_scan(CliKind::Claude, custom_names, force, discovered, |loc, ctx| {
            let Some(session_path) = loc.as_file_path() else {
                return Ok(Resolution::Skip);
            };
            let file_name = session_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("");
            if !file_name.ends_with(".jsonl") {
                return Ok(Resolution::Skip);
            }
            let file_path = loc.to_key();
            // 双轨墓碑拦截：若会话已被标记删除，拦截重新索引，防止死灰复燃。
            // 必须先于元数据解析 —— 已删会话不该再触发一次读盘解析。
            let session_id = session_path.file_stem().and_then(|n| n.to_str()).unwrap_or("");
            if ctx.tombstone_filter.is_tombstoned(session_id, &file_path) {
                return Ok(Resolution::Skip);
            }
            let file_metadata = match fs::metadata(session_path) {
                Ok(metadata) if metadata.is_file() => metadata,
                _ => return Ok(Resolution::Skip),
            };
            let file_size = file_metadata.len();
            let modified_ms = file_modified_ms(&file_metadata);
            // 缓存命中（未变更且非 force）时不重解析元数据；未命中才读盘解析。
            let metadata = if ctx.force {
                None
            } else {
                cached_session_list_metadata(ctx.cached_index, &file_path, file_size, modified_ms)
            }
            .or_else(|| self.metadata_only(&file_path));
            let Some(metadata) = metadata else {
                return Ok(Resolution::Unparsed);
            };

            // 枚举已拉平成路径列表，编码目录名改由会话文件的父目录名还原
            let encoded_dir = session_path
                .parent()
                .and_then(|parent| parent.file_name())
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();
            let fallback_project_path =
                session_mod::resolve_project_path(&encoded_dir, None, Some(&project_map));

            // 项目路径三级来源：会话文件里的真实 cwd → history 映射 → 索引里读回的旧值。
            // 索引里读回的旧值若是空串，表示这一列没有值，不能拿它顶替真路径。
            let resolved_project_path = parser::read_project_path(&file_path)
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .or_else(|| {
                    session_mod::find_project_path_in_map(&encoded_dir, &project_map)
                        .map(|value| value.to_string())
                })
                .or_else(|| {
                    metadata
                        .project_path
                        .as_deref()
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(|value| value.to_string())
                });

            let session_id = metadata.session_id.clone();
            // 展示路径取**纠正后**的值：索引里读回的旧 project_path 可能过期，纠正值才是
            // 最终采用的项目路径。骨架的纠正发生在投影求值之后，故此处必须自己取纠正值，
            // 不能沿用尚未纠正的元数据原值 —— 否则实时扫描与快照（读索引）会显示成两个路径。
            // 六个源里只有本源的纠正值会与元数据原值不同，其余五个两者恒等。
            let original_path = resolved_project_path
                .clone()
                .or_else(|| fallback_project_path.clone());
            let title = metadata.title.clone();
            Ok(Resolution::Session {
                projection: ScanProjection {
                    // 投影给出的「最终采用」值：与元数据不同时由骨架写一条索引更新，
                    // 否则快照（读索引）会与实时扫描的项目分组分叉。
                    project_path: resolved_project_path,
                    encoded_dir: Some(encoded_dir),
                    original_path,
                    title,
                    history_display: session_map
                        .get(&session_id)
                        .map(|h| h.display.clone())
                        .filter(|d| !d.is_empty()),
                    git_branch: metadata.git_branch.clone(),
                },
                metadata,
            })
        })
    }

    fn parse(&self, key: &str) -> AppResult<Vec<ChatMessage>> {
        claude::parse_session_file(key)
    }

    fn first_user_message(&self, key: &str) -> Option<String> {
        claude::read_first_user_message(key)
    }

    /// 会话文件以会话 id 命名，文件名主干即标识。
    fn session_id(&self, key: &str) -> Option<String> {
        Path::new(key)
            .file_stem()
            .and_then(|stem| stem.to_str())
            .map(|stem| stem.to_string())
    }

    /// Claude 的会话文件不记录所属项目路径，项目由目录编码反解，不走本出口。
    fn project_path_raw(&self, _key: &str) -> Option<String> {
        None
    }

    fn metadata_only(&self, key: &str) -> Option<SessionListMetadata> {
        claude::scan_claude_metadata_only(Path::new(key))
    }

    /// Claude 的子代理会话由转发层的父目录判据覆盖，本 CLI 无额外形态。
    fn is_subagent(&self, _key: &str) -> bool {
        false
    }

    fn parse_streaming(
        &self,
        key: &str,
        skip_sidechain: bool,
        on_batch: &mut dyn FnMut(Vec<ChatMessage>) -> bool,
    ) -> AppResult<(u64, HashMap<String, SubagentInfo>)> {
        claude::parse_session_file_streaming(key, skip_sidechain, on_batch)
    }

    fn parse_incremental(
        &self,
        key: &str,
        offset: u64,
        skip_sidechain: bool,
    ) -> AppResult<SessionLoadResult> {
        claude::parse_session_incremental(key, offset, skip_sidechain)
    }

    fn parse_from_content(&self, content: &str) -> Vec<ChatMessage> {
        claude::parse_session_from_string(content).unwrap_or_default()
    }

    fn search_docs(
        &self,
        key: &str,
        on_progress: &mut dyn FnMut(SearchScanProgress),
    ) -> Option<Vec<SearchDocument>> {
        crate::parser::shared::scan_search_docs_with_extractor(
            Path::new(key),
            claude::extract_claude_role_text,
            on_progress,
        )
    }

    fn search_docs_from_bytes(&self, content: &[u8]) -> Option<Vec<SearchDocument>> {
        crate::parser::shared::scan_search_docs_from_text(content, claude::extract_claude_role_text)
    }

    fn new_session(&self) -> Option<&'static dyn LaunchFeature> {
        Some(&CLAUDE_LAUNCH)
    }

    fn resume_session(&self) -> Option<&'static dyn LaunchFeature> {
        Some(&CLAUDE_LAUNCH)
    }

    fn config_file(&self) -> Option<&'static dyn crate::cli_registry::features::ConfigFileFeature> {
        Some(&CLAUDE_CONFIG_FILE)
    }

    fn api_proxy(&self) -> Option<&'static dyn crate::cli_registry::features::ApiProxyFeature> {
        Some(&CLAUDE_API_PROXY)
    }

    fn api_profile(&self) -> Option<&'static dyn crate::cli_registry::features::ApiProfileFeature> {
        Some(&CLAUDE_API_PROFILE)
    }

    fn fork(&self) -> Option<&'static dyn crate::cli_registry::features::ForkFeature> {
        Some(&CLAUDE_FORK)
    }

    fn usage_stats(&self) -> Option<&'static dyn crate::cli_registry::features::UsageStatsFeature> {
        Some(&CLAUDE_USAGE_STATS)
    }

    fn is_session_event_path(&self, path: &Path, sessions_dir: &Path) -> bool {
        let path_str = path.to_string_lossy();
        if path_str.contains("/subagents/") || path_str.contains("\\subagents\\") {
            return false;
        }
        if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
            return true;
        }
        if let Some(parent) = path.parent() {
            if parent.parent() == Some(sessions_dir) || parent == sessions_dir {
                return true;
            }
        }
        false
    }

    fn locate_executable(&self) -> Option<String> {
        crate::cli::find_on_path(self.descriptor().command)
    }

    /// 会话文件名是 `session.jsonl*` 这类成批共用的固定名时，文件名主干不是唯一 id，
    /// 改取父目录名；其余取文件名主干。
    ///
    /// **`session.jsonl*` 这一支与索引端刻意不同**：索引端的 `path_only_session_id` 只看路径、
    /// 不读文件，对这类文件名给的是主干（随后被归一成空串、退回物理路径那一支）。
    /// 两处**不要统一** —— 统一会同时改掉墓碑判定与检索待索引列表的判据。
    fn tombstone_fallback_id(&self, session_path: &Path) -> String {
        let file_name = session_path.file_name().and_then(|s| s.to_str()).unwrap_or("");
        if file_name.starts_with("session.jsonl") {
            session_path
                .parent()
                .and_then(|parent| parent.file_name())
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string()
        } else {
            session_path.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string()
        }
    }

    fn path_only_session_id(&self, session_path: &str) -> Option<String> {
        Path::new(session_path).file_stem().and_then(|s| s.to_str()).map(|s| s.to_string())
    }

    /// 本 CLI 的自定义标题就在会话文件里、索引已存，没有另一个数据源要取。
    fn custom_titles(&self) -> HashMap<String, String> {
        HashMap::new()
    }

    /// 本 CLI 没有独立维护的索引标题文件，标题随会话文件读取。
    fn index_titles(&self) -> Option<HashMap<String, String>> {
        None
    }

    /// Claude 的检索结果用 history.jsonl 同时提供展示名兜底与项目路径反解，
    /// 装配路径与其余五家不同，故走自己的 builder。
    fn build_search_results(
        &self,
        aggregated: Vec<AggregatedSessionSearch>,
        records: &HashMap<String, app_db::SessionListIndexRecord>,
        custom_names: &HashMap<String, String>,
    ) -> AppResult<Vec<SearchResult>> {
        let history_path = crate::cli::history_path(CliKind::Claude)?;
        let (session_map, project_map) = history::parse_history(history_path.to_str().unwrap_or(""));
        Ok(build_claude_search_results(
            aggregated,
            records,
            custom_names,
            &session_map,
            &project_map,
        ))
    }

    /// 源文件是明文，归档即 `gzip(jsonl)`，内层无需再解。
    fn decode_archived_source(&self, bytes: Vec<u8>, _truncation: Truncation) -> Vec<u8> {
        bytes
    }

    /// 源文件是明文，恢复写回无需回压。
    fn encode_for_restore(&self, content: Vec<u8>, _session_path: &str) -> AppResult<Vec<u8>> {
        Ok(content)
    }

    /// 启动 PTY 前把当前 settings 复制到一份临时文件，供随后注入钩子中继配置；
    /// 路径交给启动流程作为 `--settings` 参数。
    ///
    /// 失败由调用方决定取舍：PTY 启动会退回前端状态探测，故此处不吞错。
    fn build_temp_settings(&self) -> AppResult<Option<String>> {
        build_claude_temp_settings_file().map(Some)
    }

    /// Claude 的钩子事件需要转给本应用（状态栏与通知依赖）。
    fn needs_hook_relay(&self) -> bool {
        true
    }

    /// Claude 的转录按 Claude 格式提取。
    fn transcript_format(&self) -> Option<transcript_store::extract::CliFormat> {
        Some(transcript_store::extract::CliFormat::Claude)
    }
}

/// Claude 的新建与恢复共用一份启动计划：两者都是命令行参数，
/// 差异只在恢复多一个 `--resume <id>`。
struct ClaudeLaunch;

static CLAUDE_LAUNCH: ClaudeLaunch = ClaudeLaunch;

impl LaunchFeature for ClaudeLaunch {
    fn plan(&self, req: &LaunchRequest<'_>) -> AppResult<LaunchPlan> {
        let mut args = Vec::new();
        // 参数顺序与改动前逐项相同：命令串会原样出现在终端的进程列表与「复制命令」里，
        // 重排会让用户已存的脚本与本次结果分叉，而本期是零行为变更的重构。
        if let Some(id) = req.session_id {
            args.push("--resume".to_string());
            args.push(id.to_string());
        }
        if req.skip_permissions {
            args.push("--dangerously-skip-permissions".to_string());
        }
        if let Some(path) = req.settings_file {
            args.push("--settings".to_string());
            args.push(path.to_string());
        }
        Ok(LaunchPlan::CommandLine { args })
    }
}
