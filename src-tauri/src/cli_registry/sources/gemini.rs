use crate::app_db;
use crate::cli::CliKind;
use crate::commands::session::{build_codex_search_results, AggregatedSessionSearch};
use crate::commands::session_index::{
    build_projects_scan, build_projects_snapshot, cached_session_list_metadata, file_modified_ms,
    RawSession, Resolution, ScanProjection, SnapshotProjection,
};
use crate::error::AppResult;
use crate::parser::gemini;
use crate::parser::shared::{SearchDocument, SearchScanProgress};
use crate::session::{
    ChatMessage, PaginatedProjects, SearchResult, SessionListMetadata, SessionLoadResult,
    SubagentInfo, UsageRecord,
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

/// 一次扫描的整批准备产物：项目名 → 真实项目路径。
///
/// 两个来源、后者不覆盖前者：`history/<项目名>/.project_root` 是显式登记，
/// `projects.json` 只在名字尚未登记时补位 —— 前者更贴近用户改过的目录。
/// 在 `scan` 里整批读一次、由闭包捕获，逐条定位符 O(1) 命中。
struct GeminiPrepared {
    project_paths: HashMap<String, String>,
}

pub(crate) struct GeminiSource;

static DESCRIPTOR: CliDescriptor = CliDescriptor {
    name: "Gemini",
    tray_label: "Gemini",
    proxy_port: 18084,
    command: "gemini",
    file_root: FileRoot { data_dir_segments: &[".gemini"], sessions_subdir: Some("tmp") },
};

/// Gemini 的用量统计能力与实现同处一文件：能力位由「有无实现」对账，不必另立清单。
static GEMINI_USAGE_STATS: GeminiUsageStats = GeminiUsageStats;
struct GeminiUsageStats;
impl crate::cli_registry::features::UsageStatsFeature for GeminiUsageStats {
    fn usage_records(&self, key: &str, project: &str) -> Vec<UsageRecord> {
        gemini::extract_gemini_usage_records(key, project)
    }
}

impl CliSource for GeminiSource {
    fn descriptor(&self) -> &'static CliDescriptor {
        &DESCRIPTOR
    }

    /// Antigravity 的数据根嵌在 `.gemini/` 之下，故显式排除它：互斥性写在判据里，
    /// 而不是写在注册次序里。两侧用同一个判据，否决范围与对方的认领范围逐字对应。
    fn owns(&self, key: &str) -> bool {
        under_data_root(key, CliKind::Gemini) && !under_data_root(key, CliKind::Antigravity)
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

    /// 枚举本 CLI 的会话：逐层下降 `tmp/<项目名>/chats/<会话>.jsonl`，比别的源多一层。
    /// 没有 `chats/` 子目录的项目目录直接跳过（不记为未验证）；`chats/` 读取失败才记入
    /// `unverified_prefixes`，其下既有索引行本轮保留。
    fn discover(&self) -> AppResult<Discovered> {
        let sessions_dir = crate::cli::sessions_dir(CliKind::Gemini)?;
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
            let chats_dir = project_dir.join("chats");
            if !chats_dir.exists() {
                continue;
            }
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
                locators.push(SessionLocator::File {
                    cli_id: CliKind::Gemini,
                    path: session_path,
                });
            }
        }
        Ok(Discovered { locators, unverified_prefixes })
    }

    /// 项目路径缺席时退回会话 id 作分组键；无 history 来源，无分支概念。
    fn snapshot(
        &self,
        custom_names: &HashMap<String, String>,
        page: usize,
        page_size: usize,
    ) -> AppResult<Option<PaginatedProjects>> {
        build_projects_snapshot(
            CliKind::Gemini,
            custom_names,
            page,
            page_size,
            || {
                Ok(|record: &crate::app_db::SessionListIndexRecord| {
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
                        git_branch: record.git_branch.clone(),
                    }
                })
            },
        )
    }

    /// 准备「项目名 → 真实项目路径」映射，再交给公共骨架。
    ///
    /// 映射整批读一次、由闭包捕获，逐条定位符 O(1) 命中；根目录缺席时不会走到这里
    /// （`scan` 已早退），与改动前「准备在早退之后」的次序一致。
    ///
    /// 闭包逐条解析：墓碑拦截先于元数据解析，缓存命中时不重解析。
    ///
    /// 投影：项目名 = 会话路径的祖父目录名（`tmp/<项目名>/chats/<会话>.jsonl`），
    /// 再经准备映射换成真实项目路径；映射缺席时退回项目名本身。项目路径恒有值
    /// （`Some`），故 `encoded_dir` / `original_path` 都不会是 `None`。
    fn scan(
        &self,
        custom_names: &HashMap<String, String>,
        force: bool,
    ) -> AppResult<Vec<RawSession>> {
        let sessions_dir = crate::cli::sessions_dir(CliKind::Gemini)?;
        // 会话根目录不存在与「存在但为空」对陈旧行判定意味着相反的事：根目录缺席说明
        // 数据源整体不可用（未安装、外置盘未挂载），此时不该按「都没扫到」清理索引；
        // 空目录才是「会话确实没了」。故缺席时直接早退，不进入索引机器。
        if !sessions_dir.exists() {
            return Ok(Vec::new());
        }

        let discovered = self.discover()?;

        let data_dir = crate::cli::data_dir(CliKind::Gemini)?;
        let mut project_paths: HashMap<String, String> = HashMap::new();
        let history_dir = data_dir.join("history");
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
        let prepared = GeminiPrepared { project_paths };

        build_projects_scan(CliKind::Gemini, custom_names, force, discovered, |loc, ctx| {
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

            let project_name = session_path
                .parent()
                .and_then(|p| p.parent())
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();
            let original_path = prepared
                .project_paths
                .get(&project_name)
                .cloned()
                .unwrap_or_else(|| project_name.clone());
            Ok(Resolution::Session {
                projection: ScanProjection {
                    project_path: Some(original_path.clone()),
                    encoded_dir: Some(original_path.clone()),
                    original_path: Some(original_path),
                    title: metadata.title.clone(),
                    history_display: None,
                    // 扫描用空串：gemini 的会话文件不记录分支，列表不展示分支。
                    // 快照走 `record.git_branch` 是另一条装配路径，两者不可「统一」。
                    git_branch: String::new(),
                },
                metadata,
            })
        })
    }

    fn parse(&self, key: &str) -> AppResult<Vec<ChatMessage>> {
        gemini::parse_gemini_session_file(key).map(|result| result.messages)
    }

    fn first_user_message(&self, key: &str) -> Option<String> {
        gemini::read_gemini_first_user_message(key)
    }

    fn session_id(&self, key: &str) -> Option<String> {
        gemini::read_gemini_header_field(key, "sessionId")
    }

    /// Gemini 的会话文件不记录所属项目路径，项目由目录编码反解，不走本出口。
    fn project_path_raw(&self, _key: &str) -> Option<String> {
        None
    }

    fn metadata_only(&self, key: &str) -> Option<SessionListMetadata> {
        gemini::scan_gemini_metadata_only(Path::new(key))
    }

    /// Gemini 的子代理会话由转发层的父目录判据覆盖，本 CLI 无额外形态。
    fn is_subagent(&self, _key: &str) -> bool {
        false
    }

    fn parse_streaming(
        &self,
        key: &str,
        _skip_sidechain: bool,
        on_batch: &mut dyn FnMut(Vec<ChatMessage>) -> bool,
    ) -> AppResult<(u64, HashMap<String, SubagentInfo>)> {
        gemini::parse_gemini_session_file_streaming(key, on_batch)
    }

    fn parse_incremental(
        &self,
        key: &str,
        offset: u64,
        _skip_sidechain: bool,
    ) -> AppResult<SessionLoadResult> {
        gemini::parse_gemini_session_incremental(key, offset)
    }

    fn parse_from_content(&self, content: &str) -> Vec<ChatMessage> {
        gemini::parse_gemini_session_from_string(content)
    }

    fn search_docs(
        &self,
        key: &str,
        on_progress: &mut dyn FnMut(SearchScanProgress),
    ) -> Option<Vec<SearchDocument>> {
        crate::parser::shared::scan_search_docs_with_extractor(
            Path::new(key),
            gemini::extract_gemini_role_text,
            on_progress,
        )
    }

    fn search_docs_from_bytes(&self, content: &[u8]) -> Option<Vec<SearchDocument>> {
        crate::parser::shared::scan_search_docs_from_text(content, gemini::extract_gemini_role_text)
    }

    fn new_session(&self) -> Option<&'static dyn LaunchFeature> {
        Some(&GEMINI_LAUNCH)
    }

    fn resume_session(&self) -> Option<&'static dyn LaunchFeature> {
        Some(&GEMINI_LAUNCH)
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
        Some(&GEMINI_USAGE_STATS)
    }

    fn is_session_event_path(&self, path: &Path, _sessions_dir: &Path) -> bool {
        // Gemini 会话文件为 tmp/<hash>/chats/session-*.jsonl
        path.extension().and_then(|e| e.to_str()) == Some("jsonl")
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

    /// Gemini 与 Codex 共用同一装配路径，喂空的 history 映射。
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
            CliKind::Gemini,
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

    /// Gemini 的转录按 Gemini 格式提取。
    fn transcript_format(&self) -> Option<transcript_store::extract::CliFormat> {
        Some(transcript_store::extract::CliFormat::Gemini)
    }
}

/// Gemini 的免确认开关是短开关 `-y`，且排在 `--resume` 之前 ——
/// 恢复是开关而非子命令，这一点与 Claude 相同、与 Codex 不同。
struct GeminiLaunch;

static GEMINI_LAUNCH: GeminiLaunch = GeminiLaunch;

impl LaunchFeature for GeminiLaunch {
    fn plan(&self, req: &LaunchRequest<'_>) -> AppResult<LaunchPlan> {
        let mut args = Vec::new();
        if req.skip_permissions {
            args.push("-y".to_string());
        }
        if let Some(id) = req.session_id {
            args.push("--resume".to_string());
            args.push(id.to_string());
        }
        // Gemini 没有设置文件参数：忽略而不是报错，与改动前一致。
        Ok(LaunchPlan::CommandLine { args })
    }
}
