use crate::app_db;
use crate::cli::CliKind;
use crate::commands::session::{build_codex_search_results, AggregatedSessionSearch};
use crate::commands::session_index::{
    antigravity_transcript_naming_drift, build_projects_scan, build_projects_snapshot,
    cached_session_list_metadata, file_modified_ms, RawSession, Resolution, ScanProjection,
    SnapshotProjection, ANTIGRAVITY_TRANSCRIPT_FILE_NAME,
};
use crate::error::AppResult;
use crate::parser::antigravity;
use crate::parser::shared::{SearchDocument, SearchScanProgress};
use crate::session::{
    ChatMessage, PaginatedProjects, SearchResult, SessionListMetadata, SessionLoadResult,
    SubagentInfo, UsageRecord,
};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use super::super::descriptor::{CliDescriptor, FileRoot};
use super::super::features::{LaunchFeature, LaunchPlan, LaunchRequest};
use super::super::source::{
    collect_jsonl_files, file_backed_exists, under_data_root, CliSource, Truncation,
};
use super::super::{Discovered, SessionLocator};

pub(crate) struct AntigravitySource;

static DESCRIPTOR: CliDescriptor = CliDescriptor {
    name: "Antigravity",
    tray_label: "Antigravity",
    proxy_port: crate::cli_registry::NO_PROXY_PORT,
    // 展示名与可执行名刻意不同：`agy` 是命令名，展示名另有一份。
    command: "agy",
    // 数据目录嵌在 `.gemini/` 之下，是两级分段而非单级目录名。
    file_root: FileRoot {
        data_dir_segments: &[".gemini", "antigravity-cli"],
        sessions_subdir: Some("brain"),
    },
};

/// Antigravity 的用量统计能力与实现同处一文件：能力位由「有无实现」对账，不必另立清单。
static ANTIGRAVITY_USAGE_STATS: AntigravityUsageStats = AntigravityUsageStats;
struct AntigravityUsageStats;
impl crate::cli_registry::features::UsageStatsFeature for AntigravityUsageStats {
    fn usage_records(&self, key: &str, project: &str) -> Vec<UsageRecord> {
        antigravity::extract_usage_records(key, project)
    }
}

impl CliSource for AntigravitySource {
    fn descriptor(&self) -> &'static CliDescriptor {
        &DESCRIPTOR
    }

    fn owns(&self, key: &str) -> bool {
        under_data_root(key, CliKind::Antigravity)
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

    /// 枚举本 CLI 的会话：递归收集 `brain/` 下的全部 `.jsonl`（含非正本的兄弟文件，
    /// 命名漂移检测要看到它们才能判断正本是否改名）。
    ///
    /// 不排序：递归收集的次序即列表次序。读取失败会冒泡中止整轮扫描
    /// （递归收集无法精确记出失败子树），故 `unverified_prefixes` 给空。
    fn discover(&self) -> AppResult<Discovered> {
        let brain_dir = crate::cli::sessions_dir(CliKind::Antigravity)?;
        if !brain_dir.exists() {
            return Ok(Discovered { locators: Vec::new(), unverified_prefixes: Vec::new() });
        }
        let mut jsonl_files = Vec::new();
        collect_jsonl_files(&brain_dir, &mut jsonl_files)?;
        Ok(Discovered {
            locators: jsonl_files
                .into_iter()
                .map(|path| SessionLocator::File { cli_id: CliKind::Antigravity, path })
                .collect(),
            unverified_prefixes: Vec::new(),
        })
    }

    /// 项目路径解析不出时分组键同为 `None`（与实时扫描一致）；无 history 来源、无分支概念。
    fn snapshot(
        &self,
        custom_names: &HashMap<String, String>,
        page: usize,
        page_size: usize,
    ) -> AppResult<Option<PaginatedProjects>> {
        build_projects_snapshot(
            CliKind::Antigravity,
            custom_names,
            page,
            page_size,
            || {
                Ok(|record: &crate::app_db::SessionListIndexRecord| {
                    let original_path = record
                        .project_path
                        .clone()
                        .filter(|path| !path.trim().is_empty());
                    SnapshotProjection {
                        encoded_dir: original_path.clone(),
                        original_path,
                        title: record.title.clone(),
                        history_display: None,
                        git_branch: String::new(),
                    }
                })
            },
        )
    }

    /// 准备本 CLI 的定位符，再把逐条解析交给公共骨架。
    ///
    /// 命名漂移检测要在过滤前看到全部 jsonl，故这里先对完整定位符集告警，再把完整集交给
    /// 骨架 —— 过滤（只留正本、排除子代理）由闭包承担，两种形态的差异都收在源里。
    ///
    /// 闭包逐条解析：只收录会话正本且排除子代理，墓碑拦截先于元数据解析，缓存命中时不重解析。
    ///
    /// 枚举递归收集 `brain/` 下全部 `.jsonl`，因为命名漂移检测要看到兄弟文件才能判断
    /// 正本是否改名；但真正进入索引的只有名为 `transcript.jsonl` 的正本 —— 同一日志目录下
    /// 的兄弟文件与正本行数、事件区间几乎一致，一并收录会把每个会话重复索引一遍。
    /// 子代理会话同样在此剔除：其判据要看文件内容，属于本 CLI 的形态知识，
    /// 且改动前在元数据解析之前就跳过它们。
    fn scan(
        &self,
        custom_names: &HashMap<String, String>,
        force: bool,
    ) -> AppResult<Vec<RawSession>> {
        let brain_dir = crate::cli::sessions_dir(CliKind::Antigravity)?;
        // 会话根目录不存在与「存在但为空」对陈旧行判定意味着相反的事：根目录缺席说明
        // 数据源整体不可用（未安装、外置盘未挂载），此时不该按「都没扫到」清理索引；
        // 空目录才是「会话确实没了」。故缺席时直接早退，不进入索引机器。
        if !brain_dir.exists() {
            return Ok(Vec::new());
        }

        let discovered = self.discover()?;

        // 命名漂移检测：只告警、不放宽匹配。日志目录里存在 jsonl 却没有会话正本名，说明上游
        // 很可能改了命名 —— 把「会话静默消失」变成日志里当天可见的信号，而不是等用户来问。
        let jsonl_files: Vec<PathBuf> = discovered
            .locators
            .iter()
            .filter_map(|locator| locator.as_file_path().map(Path::to_path_buf))
            .collect();
        for (dir, actual_names) in antigravity_transcript_naming_drift(&jsonl_files) {
            tracing::warn!(
                "Antigravity 日志目录中未找到会话正本 {}，实际文件: {:?}；上游可能改了命名，该目录下的会话本轮不会收录: dir={:?}",
                ANTIGRAVITY_TRANSCRIPT_FILE_NAME,
                actual_names,
                dir
            );
        }

        build_projects_scan(
            CliKind::Antigravity,
            custom_names,
            force,
            discovered,
            |loc, ctx| {
                let Some(session_path) = loc.as_file_path() else {
                    return Ok(Resolution::Skip);
                };
                let file_name = session_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("");
                if file_name != ANTIGRAVITY_TRANSCRIPT_FILE_NAME
                    || crate::parser::is_subagent_session(&session_path.to_string_lossy())
                {
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
                    cached_session_list_metadata(
                        ctx.cached_index,
                        &file_path,
                        file_size,
                        modified_ms,
                    )
                }
                .or_else(|| self.metadata_only(&file_path));
                let Some(metadata) = metadata else {
                    return Ok(Resolution::Unparsed);
                };

                // 项目路径解析不出就是 `None`：不退回会话 id，分组键与展示值同走「缺席」。
                let original_path = metadata
                    .project_path
                    .clone()
                    .filter(|path| !path.trim().is_empty());
                let encoded_dir = original_path.clone();
                Ok(Resolution::Session {
                    projection: ScanProjection {
                        project_path: original_path.clone(),
                        encoded_dir,
                        original_path,
                        title: metadata.title.clone(),
                        history_display: None,
                        // 扫描用空串：会话文件不记录分支，列表不展示分支。
                        // 快照走 `record.git_branch` 是另一条装配路径，两者不可「统一」。
                        git_branch: String::new(),
                    },
                    metadata,
                })
            },
        )
    }

    fn parse(&self, key: &str) -> AppResult<Vec<ChatMessage>> {
        antigravity::parse_session_file(key)
    }

    fn first_user_message(&self, key: &str) -> Option<String> {
        antigravity::read_first_user_message(key)
    }

    fn session_id(&self, key: &str) -> Option<String> {
        antigravity::read_session_id(key)
    }

    fn project_path_raw(&self, key: &str) -> Option<String> {
        antigravity::read_project_path(key)
    }

    fn metadata_only(&self, key: &str) -> Option<SessionListMetadata> {
        antigravity::scan_session_metadata_only(Path::new(key))
    }

    fn is_subagent(&self, key: &str) -> bool {
        antigravity::is_subagent_session(key)
    }

    fn parse_streaming(
        &self,
        key: &str,
        skip_sidechain: bool,
        on_batch: &mut dyn FnMut(Vec<ChatMessage>) -> bool,
    ) -> AppResult<(u64, HashMap<String, SubagentInfo>)> {
        antigravity::parse_session_file_streaming(key, skip_sidechain, on_batch)
    }

    fn parse_incremental(
        &self,
        key: &str,
        offset: u64,
        _skip_sidechain: bool,
    ) -> AppResult<SessionLoadResult> {
        antigravity::parse_session_incremental(key, offset)
    }

    fn parse_from_content(&self, content: &str) -> Vec<ChatMessage> {
        antigravity::parse_antigravity_session_from_string(content)
    }

    fn search_docs(
        &self,
        key: &str,
        on_progress: &mut dyn FnMut(SearchScanProgress),
    ) -> Option<Vec<SearchDocument>> {
        crate::parser::shared::scan_search_docs_with_extractor(
            Path::new(key),
            antigravity::extract_antigravity_role_text,
            on_progress,
        )
    }

    fn search_docs_from_bytes(&self, content: &[u8]) -> Option<Vec<SearchDocument>> {
        crate::parser::shared::scan_search_docs_from_text(
            content,
            antigravity::extract_antigravity_role_text,
        )
    }

    fn new_session(&self) -> Option<&'static dyn LaunchFeature> {
        Some(&ANTIGRAVITY_LAUNCH)
    }

    fn resume_session(&self) -> Option<&'static dyn LaunchFeature> {
        Some(&ANTIGRAVITY_LAUNCH)
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

    /// 本 CLI 不支持分叉会话。
    fn fork(&self) -> Option<&'static dyn crate::cli_registry::features::ForkFeature> {
        None
    }

    fn usage_stats(&self) -> Option<&'static dyn crate::cli_registry::features::UsageStatsFeature> {
        Some(&ANTIGRAVITY_USAGE_STATS)
    }

    fn is_session_event_path(&self, path: &Path, sessions_dir: &Path) -> bool {
        if path.file_name().and_then(|n| n.to_str()) == Some("transcript.jsonl") {
            return true;
        }
        if let Some(parent) = path.parent() {
            if parent == sessions_dir {
                return true;
            }
        }
        false
    }

    fn locate_executable(&self) -> Option<String> {
        crate::cli::find_on_path(self.descriptor().command)
    }

    /// 本 CLI 的逻辑 id 藏在转录内容里，路径推不出来，只能读文件。
    /// 读失败时给空串，交给调用方的通用名防卫改用物理路径合成键。
    fn tombstone_fallback_id(&self, session_path: &Path) -> String {
        antigravity::extract_conversation_id(session_path).unwrap_or_default()
    }

    /// 本 CLI 的 id 必须读文件才能得到，而本方法的调用方在索引全表过滤里逐个调用它，
    /// 读文件会让它变成 O(全表 × 读盘)。故这里如实回答「仅凭路径推不出来」。
    fn path_only_session_id(&self, _session_path: &str) -> Option<String> {
        None
    }

    /// 本 CLI 的自定义标题就在会话文件里、索引已存，没有另一个数据源要取。
    fn custom_titles(&self) -> HashMap<String, String> {
        HashMap::new()
    }

    /// 本 CLI 没有独立维护的索引标题文件，标题随会话文件读取。
    fn index_titles(&self) -> Option<HashMap<String, String>> {
        None
    }

    /// Antigravity 与 Codex 共用同一装配路径，喂空的 history 映射。
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
            &HashMap::new(),
            CliKind::Antigravity,
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

    /// Antigravity 的转录暂无提取格式。
    fn transcript_format(&self) -> Option<transcript_store::extract::CliFormat> {
        None
    }
}

/// Antigravity 恢复用 `--conversation <id>`（会话在它自己的术语里叫对话），
/// 权限开关排在恢复参数之后 —— 与 Claude 的组合相同、开关名不同。
struct AntigravityLaunch;

static ANTIGRAVITY_LAUNCH: AntigravityLaunch = AntigravityLaunch;

impl LaunchFeature for AntigravityLaunch {
    fn plan(&self, req: &LaunchRequest<'_>) -> AppResult<LaunchPlan> {
        let mut args = Vec::new();
        if let Some(id) = req.session_id {
            args.push("--conversation".to_string());
            args.push(id.to_string());
        }
        if req.skip_permissions {
            args.push("--dangerously-skip-permissions".to_string());
        }
        // Antigravity 没有设置文件参数：忽略而不是报错，与改动前一致。
        Ok(LaunchPlan::CommandLine { args })
    }
}
