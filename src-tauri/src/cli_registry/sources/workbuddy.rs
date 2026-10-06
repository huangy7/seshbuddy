use crate::app_db;
use crate::cli::CliKind;
use crate::commands::session::{build_codex_search_results, AggregatedSessionSearch};
use crate::commands::session_index::{
    build_projects_scan, build_projects_snapshot, cached_session_list_metadata, file_modified_ms,
    RawSession, Resolution, ScanProjection, SnapshotProjection,
};
use crate::error::{AppError, AppResult};
use crate::parser::shared::{SearchDocument, SearchScanProgress};
use crate::parser::workbuddy;
use crate::session::{
    ChatMessage, PaginatedProjects, SearchResult, SessionListMetadata, SessionLoadResult,
    SubagentInfo,
};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

use super::super::descriptor::{CliDescriptor, FileRoot};
use super::super::features::{LaunchFeature, LaunchPlan, LaunchRequest};
use super::super::source::{
    file_backed_exists, read_dir_or_record_unverified, under_data_root, CliSource, Truncation,
};
use super::super::{Discovered, SessionLocator};

/// 一次扫描的整批准备产物：会话 id → 用户重命名标题。
///
/// 整批读一次（`workbuddy.db` 的 `sessions.custom_title`），由闭包捕获、逐条 O(1) 命中；
/// 标题优先取映射，未重命名才回落到元数据里的标题。
struct WorkBuddyPrepared {
    wb_titles: HashMap<String, String>,
}

pub(crate) struct WorkBuddySource;

static DESCRIPTOR: CliDescriptor = CliDescriptor {
    name: "WorkBuddy",
    tray_label: "WorkBuddy",
    proxy_port: 18086,
    // 桌面应用而非 PATH 上的二进制；恢复走深链，从不 exec 该字符串。
    command: "workbuddy",
    file_root: FileRoot { data_dir_segments: &[".workbuddy"], sessions_subdir: Some("projects") },
};

impl CliSource for WorkBuddySource {
    fn descriptor(&self) -> &'static CliDescriptor {
        &DESCRIPTOR
    }

    fn owns(&self, key: &str) -> bool {
        under_data_root(key, CliKind::WorkBuddy)
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

    /// 枚举本 CLI 的会话：逐层下降 `projects/<slug>/<会话>.jsonl`。
    /// 项目目录读取失败时记入 `unverified_prefixes`，其下既有索引行本轮保留。
    fn discover(&self) -> AppResult<Discovered> {
        let sessions_dir = crate::cli::sessions_dir(CliKind::WorkBuddy)?;
        if !sessions_dir.exists() {
            return Ok(Discovered { locators: Vec::new(), unverified_prefixes: Vec::new() });
        }
        let mut locators = Vec::new();
        let mut unverified_prefixes = Vec::new();
        for entry in fs::read_dir(&sessions_dir)? {
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
                    cli_id: CliKind::WorkBuddy,
                    path: session_path,
                });
            }
        }
        Ok(Discovered { locators, unverified_prefixes })
    }

    /// 项目路径缺席时退回会话 id 作分组键；标题另有用户重命名 Map 兜底。
    fn snapshot(
        &self,
        custom_names: &HashMap<String, String>,
        page: usize,
        page_size: usize,
    ) -> AppResult<Option<PaginatedProjects>> {
        build_projects_snapshot(
            CliKind::WorkBuddy,
            custom_names,
            page,
            page_size,
            || {
                // 单次读取 WorkBuddy 用户重命名 Map，整页 O(1) 命中
                let wb_titles = crate::parser::load_workbuddy_custom_titles();
                Ok(move |record: &crate::app_db::SessionListIndexRecord| {
                    let original_path = record
                        .project_path
                        .clone()
                        .filter(|path| !path.trim().is_empty())
                        .unwrap_or_else(|| record.session_id.clone());
                    SnapshotProjection {
                        encoded_dir: Some(original_path.clone()),
                        original_path: Some(original_path),
                        title: wb_titles
                            .get(&record.session_id)
                            .cloned()
                            .or_else(|| record.title.clone()),
                        history_display: None,
                        git_branch: record.git_branch.clone(),
                    }
                })
            },
        )
    }

    /// 准备本 CLI 的用户重命名映射，再把逐条解析交给公共骨架。
    ///
    /// 整批读取用户重命名 Map（workbuddy.db 的 sessions.custom_title），由闭包捕获、
    /// 逐条 O(1) 命中。
    ///
    /// 闭包逐条解析：墓碑拦截先于元数据解析，缓存命中时不重解析。
    ///
    /// 投影：项目 slug 由会话文件的父目录名还原，权威项目路径来自每行 JSONL 的 `cwd`，
    /// 缺席时按 slug 解码兜底；标题优先取用户重命名映射，缺失才回落元数据里的标题。
    fn scan(
        &self,
        custom_names: &HashMap<String, String>,
        force: bool,
    ) -> AppResult<Vec<RawSession>> {
        let sessions_dir = crate::cli::sessions_dir(CliKind::WorkBuddy)?;
        // 会话根目录不存在与「存在但为空」对陈旧行判定意味着相反的事：根目录缺席说明
        // 数据源整体不可用（未安装、外置盘未挂载），此时不该按「都没扫到」清理索引；
        // 空目录才是「会话确实没了」。故缺席时直接早退，不进入索引机器。
        if !sessions_dir.exists() {
            return Ok(Vec::new());
        }

        let discovered = self.discover()?;
        let prepared = WorkBuddyPrepared {
            wb_titles: crate::parser::load_workbuddy_custom_titles(),
        };
        build_projects_scan(CliKind::WorkBuddy, custom_names, force, discovered, |loc, ctx| {
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

            // 枚举已拉平成路径列表，项目 slug 由会话文件的父目录名还原
            let project_slug = session_path
                .parent()
                .and_then(|path| path.file_name())
                .and_then(|name| name.to_str())
                .unwrap_or("")
                .to_string();
            // 权威项目路径来自每行 JSONL 的 cwd；兜底按 slug 解码
            let original_path = metadata
                .project_path
                .clone()
                .filter(|path| !path.trim().is_empty())
                .or_else(|| crate::session::resolve_project_path(&project_slug, None, None));
            let encoded_dir = original_path.clone();
            let title = prepared
                .wb_titles
                .get(&metadata.session_id)
                .cloned()
                .or_else(|| metadata.title.clone());
            Ok(Resolution::Session {
                projection: ScanProjection {
                    project_path: original_path.clone(),
                    encoded_dir,
                    original_path,
                    title,
                    history_display: None,
                    // 扫描用空串：会话文件不记录分支，列表不展示分支。
                    // 快照走 `record.git_branch` 是另一条装配路径，两者不可「统一」。
                    git_branch: String::new(),
                },
                metadata,
            })
        })
    }

    fn parse(&self, key: &str) -> AppResult<Vec<ChatMessage>> {
        workbuddy::parse_workbuddy_session_file(key).map(|result| result.messages)
    }

    fn first_user_message(&self, key: &str) -> Option<String> {
        workbuddy::read_workbuddy_first_user_message(key)
    }

    /// 头部 `sessionId` 是权威标识；旧文件缺该字段时退回文件名主干。
    fn session_id(&self, key: &str) -> Option<String> {
        workbuddy::read_workbuddy_header_field(key, "sessionId").or_else(|| {
            Path::new(key)
                .file_stem()
                .and_then(|stem| stem.to_str())
                .map(|stem| stem.to_string())
        })
    }

    fn project_path_raw(&self, key: &str) -> Option<String> {
        workbuddy::read_workbuddy_header_field(key, "cwd")
    }

    fn metadata_only(&self, key: &str) -> Option<SessionListMetadata> {
        workbuddy::scan_workbuddy_metadata_only(Path::new(key))
    }

    /// WorkBuddy 的子代理会话由转发层的父目录判据覆盖，本 CLI 无额外形态。
    fn is_subagent(&self, _key: &str) -> bool {
        false
    }

    fn parse_streaming(
        &self,
        key: &str,
        _skip_sidechain: bool,
        on_batch: &mut dyn FnMut(Vec<ChatMessage>) -> bool,
    ) -> AppResult<(u64, HashMap<String, SubagentInfo>)> {
        workbuddy::parse_workbuddy_session_file_streaming(key, on_batch)
    }

    fn parse_incremental(
        &self,
        key: &str,
        offset: u64,
        _skip_sidechain: bool,
    ) -> AppResult<SessionLoadResult> {
        workbuddy::parse_workbuddy_session_incremental(key, offset)
    }

    fn parse_from_content(&self, content: &str) -> Vec<ChatMessage> {
        workbuddy::parse_workbuddy_session_from_string(content)
    }

    fn search_docs(
        &self,
        key: &str,
        on_progress: &mut dyn FnMut(SearchScanProgress),
    ) -> Option<Vec<SearchDocument>> {
        crate::parser::shared::scan_search_docs_with_extractor(
            Path::new(key),
            workbuddy::extract_workbuddy_role_text,
            on_progress,
        )
    }

    fn search_docs_from_bytes(&self, content: &[u8]) -> Option<Vec<SearchDocument>> {
        crate::parser::shared::scan_search_docs_from_text(
            content,
            workbuddy::extract_workbuddy_role_text,
        )
    }

    /// 没有新建会话能力：新开会话只能在应用内完成，命令行形态无从表达。
    fn new_session(&self) -> Option<&'static dyn LaunchFeature> {
        None
    }

    fn resume_session(&self) -> Option<&'static dyn LaunchFeature> {
        Some(&WORKBUDDY_LAUNCH)
    }

    /// 本 CLI 没有可托管的配置文件。
    fn config_file(&self) -> Option<&'static dyn crate::cli_registry::features::ConfigFileFeature> {
        None
    }

    fn api_proxy(&self) -> Option<&'static dyn crate::cli_registry::features::ApiProxyFeature> {
        None
    }

    /// 本 CLI 不支持配置档托管。
    fn api_profile(&self) -> Option<&'static dyn crate::cli_registry::features::ApiProfileFeature> {
        None
    }

    fn fork(&self) -> Option<&'static dyn crate::cli_registry::features::ForkFeature> {
        Some(&WORKBUDDY_FORK)
    }

    /// 没有用量统计能力：没有可读的用量来源，不实现而非伪造空表。
    fn usage_stats(&self) -> Option<&'static dyn crate::cli_registry::features::UsageStatsFeature> {
        None
    }

    fn is_session_event_path(&self, path: &Path, _sessions_dir: &Path) -> bool {
        path.extension().and_then(|e| e.to_str()) == Some("jsonl")
    }

    /// 桌面应用型 CLI：没有 PATH 上的二进制，安装与否只能看应用 bundle 在不在。
    fn locate_executable(&self) -> Option<String> {
        crate::cli::find_workbuddy_app()
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

    /// 用户重命名存在 workbuddy.db 的 sessions.custom_title 里，**覆盖**原生 AI 标题。
    fn custom_titles(&self) -> HashMap<String, String> {
        crate::parser::load_workbuddy_custom_titles()
    }

    /// 本 CLI 没有独立维护的索引标题文件，标题随会话文件读取。
    fn index_titles(&self) -> Option<HashMap<String, String>> {
        None
    }

    /// WorkBuddy 与 Codex 共用同一装配路径，喂空的 history 映射。
    fn build_search_results(
        &self,
        aggregated: Vec<AggregatedSessionSearch>,
        records: &HashMap<String, app_db::SessionListIndexRecord>,
        custom_names: &HashMap<String, String>,
    ) -> AppResult<Vec<SearchResult>> {
        Ok(build_codex_search_results(
            aggregated,
            records,
            custom_names,
            &HashMap::new(),
            // 用户重命名存在 workbuddy.db 的 sessions.custom_title 里，**覆盖**原生 AI 标题：
            // 用户自己起的名字比模型生成的更该被看到。
            &self.custom_titles(),
            CliKind::WorkBuddy,
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

    /// 启动 PTY 不需要临时配置。
    fn build_temp_settings(&self) -> AppResult<Option<String>> {
        Ok(None)
    }

    /// 本 CLI 的钩子事件无需转给本应用。
    fn needs_hook_relay(&self) -> bool {
        false
    }

    /// WorkBuddy 的转录按 WorkBuddy 格式提取。
    fn transcript_format(&self) -> Option<transcript_store::extract::CliFormat> {
        Some(transcript_store::extract::CliFormat::WorkBuddy)
    }
}

/// WorkBuddy 的分叉能力与实现同处一文件：能力位由「有无实现」对账，不必另立清单。
static WORKBUDDY_FORK: WorkBuddyFork = WorkBuddyFork;
struct WorkBuddyFork;
impl crate::cli_registry::features::ForkFeature for WorkBuddyFork {
    /// 把新会话注册进 WorkBuddy 本地 SQLite 库（`~/.workbuddy/workbuddy.db`），
    /// 否则分叉出的会话在应用里不出现。库文件不存在或注册失败都不阻断分叉，只记日志。
    fn after_fork(&self, old_id: &str, new_id: &str) {
        // 取不到数据目录就什么都不做，与改动前 `if let (... Ok(data_dir))` 的语义一致。
        let Ok(data_dir) = crate::cli::data_dir(CliKind::WorkBuddy) else {
            return;
        };
        let db_path = data_dir.join("workbuddy.db");
        if db_path.is_file() {
            if let Err(e) = crate::commands::session::sync_workbuddy_forked_session_to_db(
                &db_path, old_id, new_id,
            ) {
                eprintln!("[fork_session] WorkBuddy SQLite 注册失败: {}", e);
            }
        }
    }
}

/// 桌面应用型 CLI 的启动形态：`workbuddy://` 深链交给应用自己解析。
/// 恢复能力在 `session_id = None` 时落到应用根深链，因此「打开应用」不需要第二个能力对象。
struct WorkBuddyLaunch;

static WORKBUDDY_LAUNCH: WorkBuddyLaunch = WorkBuddyLaunch;

impl LaunchFeature for WorkBuddyLaunch {
    fn plan(&self, req: &LaunchRequest<'_>) -> AppResult<LaunchPlan> {
        let url = match req.session_id {
            Some(id)
                if !id.is_empty()
                    && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') =>
            {
                format!("workbuddy://chat/{id}")
            }
            // 非法字符不接受：改动前这里同样报 `cli.invalid_workbuddy_session_id`，
            // 而不是把校验跳过后落到应用根深链 —— 那会让用户以为打开了会话，实际没有。
            Some(_) => {
                return Err(AppError::coded("cli.invalid_workbuddy_session_id")
                    .with("session_id", format!("{:?}", req.session_id)))
            }
            None => "workbuddy://chat".to_string(),
        };
        Ok(LaunchPlan::DeepLink { url })
    }
}
