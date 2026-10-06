use crate::app_db;
use crate::cli::CliKind;
use crate::commands::session::{build_codex_search_results, AggregatedSessionSearch};
use crate::commands::session_index::{
    build_projects_snapshot, cached_session_list_metadata, file_modified_ms,
    pick_first_valid_candidate, RawSession, ScanAccumulator, ScanProjection, SnapshotProjection,
};
use crate::error::{AppError, AppResult};
use crate::parser::dsh;
use crate::parser::shared::{SearchDocument, SearchScanProgress};
use crate::session as session_mod;
use crate::session::{
    ChatMessage, PaginatedProjects, SearchResult, SessionListMetadata, SessionLoadResult,
    SubagentInfo, UsageRecord,
};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use super::super::descriptor::{CliDescriptor, FileRoot};
use super::super::features::LaunchFeature;
use super::super::source::{
    file_backed_exists, read_dir_or_record_unverified, under_data_root, CliSource, Truncation,
};
use super::super::{Discovered, SessionLocator};

pub(crate) struct DshSource;

static DESCRIPTOR: CliDescriptor = CliDescriptor {
    name: "DSH",
    tray_label: "DSH",
    proxy_port: crate::cli_registry::NO_PROXY_PORT,
    command: "dsh",
    file_root: FileRoot { data_dir_segments: &[".dsh"], sessions_subdir: Some("sessions") },
};

/// DSH 的用量统计能力与实现同处一文件：能力位由「有无实现」对账，不必另立清单。
static DSH_USAGE_STATS: DshUsageStats = DshUsageStats;
struct DshUsageStats;
impl crate::cli_registry::features::UsageStatsFeature for DshUsageStats {
    fn usage_records(&self, key: &str, project: &str) -> Vec<UsageRecord> {
        dsh::extract_usage_records(key, project)
    }
}

impl CliSource for DshSource {
    fn descriptor(&self) -> &'static CliDescriptor {
        &DESCRIPTOR
    }

    fn owns(&self, key: &str) -> bool {
        under_data_root(key, CliKind::Dsh)
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

    /// 枚举本 CLI 的会话：逐层下降 `sessions/<项目目录>/<会话目录>`。
    ///
    /// 与别的文件型源不同，一个会话是一个**目录**（内含多代候选日志），故这里只收目录、
    /// 不按扩展名过滤；具体落到哪个候选文件由扫描侧按代次与可读性挑选。
    /// 项目目录读取失败时记入 `unverified_prefixes`，其下既有索引行本轮保留。
    fn discover(&self) -> AppResult<Discovered> {
        let sessions_dir = crate::cli::sessions_dir(CliKind::Dsh)?;
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
                if !session_path.is_dir() {
                    continue;
                }
                locators.push(SessionLocator::File {
                    cli_id: CliKind::Dsh,
                    path: session_path,
                });
            }
        }
        Ok(Discovered { locators, unverified_prefixes })
    }

    /// 项目路径缺席时退回会话 id 作分组键；索引不记录分支，列表按空分支展示。
    fn snapshot(
        &self,
        custom_names: &HashMap<String, String>,
        page: usize,
        page_size: usize,
    ) -> AppResult<Option<PaginatedProjects>> {
        build_projects_snapshot(
            CliKind::Dsh,
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
                        git_branch: String::new(),
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
        let sessions_dir = crate::cli::sessions_dir(CliKind::Dsh)?;
        // 会话根目录不存在与「存在但为空」对陈旧行判定意味着相反的事：根目录缺席说明
        // 数据源整体不可用（未安装、外置盘未挂载），此时不该按「都没扫到」清理索引；
        // 空目录才是「会话确实没了」。故缺席时直接早退，不进入索引机器。
        if !sessions_dir.exists() {
            return Ok(Vec::new());
        }

        let mut acc = ScanAccumulator::new(CliKind::Dsh, custom_names, force)?;
        // 待搬迁判据取自「库里已知的路径」，不是本轮是否复用元数据：改过推导规则后的
        // 首轮扫描被派生规则版本闸门强制成全量（复用视图为空），若只看复用视图就永远
        // 发现不了待搬迁的旧代路径 —— 首轮按新路径插一条新记录、旧路径的记录因为带
        // 归档快照被保留，直接表现为列表里同一会话出现两次。
        let indexed_paths: HashSet<String> = acc.known_index().keys().cloned().collect();
        // 本轮发生换代搬迁的记录，按新路径存放：搬迁过的记录已不在复用视图的旧键下，
        // 归档标记等展示状态要能从新路径查到。
        let mut rekeyed_records: HashMap<String, app_db::SessionListIndexRecord> = HashMap::new();

        let discovered = self.discover()?;
        // 本轮读取失败（权限/IO）的子树：其下已有记录保留原样，不按"已删除"处理
        let unverified_prefixes = discovered.unverified_prefixes;

        for locator in discovered.locators {
            let Some(session_path) = locator.as_file_path() else {
                continue;
            };
            if !session_path.is_dir() {
                continue;
            }
            // 枚举已拉平成路径列表，项目目录名改由会话目录的父目录名还原
            let encoded_dir_name = session_path
                .parent()
                .and_then(|parent| parent.file_name())
                .and_then(|name| name.to_str())
                .unwrap_or("")
                .to_string();
            let fallback_project_path =
                session_mod::resolve_project_path(&encoded_dir_name, None, None);

            let session_id = session_path.file_stem().and_then(|n| n.to_str()).unwrap_or("");
            let dir_str = session_path.to_string_lossy();

            // 候选顺位与 Fallback 容错机制：同一会话目录内可能并存多个「格式代」的日志副本，
            // 以及同代内的压缩/明文副本。按「代次降序 + 同代 zstd 优先」排序，首位即会话
            // 当前内容；当前代不可读时顺位回退到上一代，宁可展示历史内容也不让整条会话
            // 从列表里消失。
            let candidates = crate::parser::dsh::session_dir_candidates(session_path);
            if candidates.is_empty() {
                continue;
            }

            // 双轨墓碑拦截（会话级终态拦截）：若目录或任一物理候选副本已被标记为墓碑，
            // 立即跳过整个会话，严禁向其他候选 Fallback 导致已删会话意外复活。
            //
            // 必须先于路径归一：换代搬迁会改写索引行/归档元数据并物理移动归档快照，
            // 先改状态再判断「该不该改」，会把已删会话的记录搬到墓碑覆盖不到的路径上。
            if acc.is_tombstoned(session_id, &dir_str)
                || candidates
                    .iter()
                    .any(|c| acc.is_tombstoned(session_id, &c.to_string_lossy()))
            {
                continue;
            }

            // 路径归一：日志换代后当前内容落在新一代文件名上，而库里可能还挂着旧代路径的
            // 记录。不搬迁就会留下一条再也扫不到的陈旧记录 —— 列表里同一会话出现两次，
            // 且归档快照与活动条目失联。搬迁后一个会话目录只对应新路径上的唯一一条记录。
            rekeyed_records.extend(rekey_dsh_session_paths(
                acc.reusable_index(),
                &indexed_paths,
                session_id,
                &dir_str,
                &candidates,
            ));

            enum DshCandidateOutcome {
                Valid {
                    file_path: String,
                    metadata: SessionListMetadata,
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

                if let Some(m) =
                    cached_session_list_metadata(acc.reusable_index(), &file_path, file_size, modified_ms)
                {
                    return Some(DshCandidateOutcome::Valid { file_path, metadata: m });
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

                let m = build_dsh_session_metadata(&header, candidate, &meta)?;

                Some(DshCandidateOutcome::Valid {
                    file_path,
                    metadata: m,
                })
            });

            let (file_path, metadata) = match picked {
                Some(DshCandidateOutcome::Valid {
                    file_path,
                    metadata,
                }) => (file_path, metadata),
                Some(DshCandidateOutcome::SkipSession) => continue,
                None => {
                    // 候选链全部落空：把**每一个候选路径**记入失效集合，收尾时其索引行
                    // 与陈旧行一并清除（失效集合无条件并入陈旧集合）。**不记已见** ——
                    // 与改动前的口径一致，只记失效的源在大面积部分失败时不该被当成
                    // 「扫到了这么多会话」。
                    for c in &candidates {
                        if let Some(p) = c.to_str() {
                            acc.mark_invalid(p);
                        }
                    }
                    continue;
                }
            };

            // 权威项目路径来自 header cwd；兜底按项目目录名解码
            let original_path = metadata
                .project_path
                .clone()
                .filter(|value| !value.trim().is_empty())
                .or_else(|| fallback_project_path.clone());
            let title = metadata.title.clone();
            // 展示状态回落：换代搬迁后旧记录不在复用视图的新键下，故先查复用视图、
            // 再查本轮搬迁产出的副本。
            let cached = acc
                .reusable_index()
                .get(&file_path)
                .cloned()
                .or_else(|| rekeyed_records.get(&file_path).cloned());

            acc.push_session_with_cached(
                &file_path,
                metadata,
                ScanProjection {
                    project_path: original_path.clone(),
                    encoded_dir: original_path.clone(),
                    original_path,
                    title,
                    history_display: None,
                    // 扫描用空串：DSH 的会话文件不记录分支，列表不展示分支。
                    // 快照走 `record.git_branch` 是另一条装配路径，两者不可「统一」。
                    git_branch: String::new(),
                },
                cached,
            );
        }

        acc.finish(&unverified_prefixes)
    }

    fn parse(&self, key: &str) -> AppResult<Vec<ChatMessage>> {
        dsh::parse_dsh_session_file(key)
    }

    fn first_user_message(&self, key: &str) -> Option<String> {
        dsh::read_first_user_message(key)
    }

    fn session_id(&self, key: &str) -> Option<String> {
        dsh::read_session_id(key)
    }

    fn project_path_raw(&self, key: &str) -> Option<String> {
        dsh::read_project_path(key)
    }

    fn metadata_only(&self, key: &str) -> Option<SessionListMetadata> {
        dsh::scan_session_metadata_only(Path::new(key))
    }

    fn is_subagent(&self, key: &str) -> bool {
        dsh::is_subagent_file(key)
    }

    fn parse_streaming(
        &self,
        key: &str,
        _skip_sidechain: bool,
        on_batch: &mut dyn FnMut(Vec<ChatMessage>) -> bool,
    ) -> AppResult<(u64, HashMap<String, SubagentInfo>)> {
        dsh::parse_dsh_session_file_streaming(key, on_batch)
    }

    fn parse_incremental(
        &self,
        key: &str,
        offset: u64,
        _skip_sidechain: bool,
    ) -> AppResult<SessionLoadResult> {
        dsh::parse_dsh_session_incremental(key, offset)
    }

    fn parse_from_content(&self, content: &str) -> Vec<ChatMessage> {
        dsh::parse_dsh_session_from_string(content)
    }

    fn search_docs(
        &self,
        key: &str,
        on_progress: &mut dyn FnMut(SearchScanProgress),
    ) -> Option<Vec<SearchDocument>> {
        dsh::scan_search_docs_with_progress(Path::new(key), on_progress)
    }

    fn search_docs_from_bytes(&self, content: &[u8]) -> Option<Vec<SearchDocument>> {
        dsh::scan_search_docs_from_bytes(content)
    }

    /// 两个都没有：本 CLI 的会话由它自己的程序管理启动，命令行的新建与恢复
    /// 都还没有可用实现。能力位与之逐位对账（见 `features` 的契约测试）。
    fn new_session(&self) -> Option<&'static dyn LaunchFeature> {
        None
    }

    fn resume_session(&self) -> Option<&'static dyn LaunchFeature> {
        None
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
        Some(&DSH_USAGE_STATS)
    }

    fn is_session_event_path(&self, path: &Path, _sessions_dir: &Path) -> bool {
        // 会话日志按格式代命名（session.jsonl / session.v3.jsonl[.zstd] 等），
        // 新建会话或迁移到新一代都会以新文件名落盘。按规范代名判定，代际升级后
        // 的新会话才能触发重扫，否则新会话要等到下一次启动才出现在列表里。
        let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        crate::parser::dsh::parse_generation_log_filename(file_name).is_some()
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

    /// DSH 与 Codex 共用同一装配路径，喂空的 history 映射。
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
            CliKind::Dsh,
        ))
    }

    /// DSH 源文件本身 zstd 压缩，归档为 `gzip(zstd(jsonl))`，故内层要多解一层 zstd。
    /// 两个臂逐字保留原两处调用形态的判据，差异仅在「输入是否被截断」。
    fn decode_archived_source(&self, bytes: Vec<u8>, truncation: Truncation) -> Vec<u8> {
        use std::io::Read;

        let Ok(mut decoder) = zstd::stream::read::Decoder::new(&bytes[..]) else {
            return bytes;
        };
        let mut out = Vec::new();
        match truncation {
            // 完整读取：帧不完整说明数据损坏，原样返回上一层结果更安全。
            Truncation::Complete => {
                if decoder.read_to_end(&mut out).is_ok() {
                    out
                } else {
                    bytes
                }
            }
            // 头部读取：调用方按上限截断了流，帧必然不完整，能解多少要多少。
            Truncation::Truncated => {
                let _ = decoder.read_to_end(&mut out);
                if out.is_empty() {
                    bytes
                } else {
                    out
                }
            }
        }
    }

    /// DSH 源路径后缀为 `.zstd` / `.zst` 时，恢复写回须回压 zstd，
    /// 否则明文 jsonl 写到 `.zstd` 路径会让按后缀解码的解析器读不了。
    fn encode_for_restore(&self, content: Vec<u8>, session_path: &str) -> AppResult<Vec<u8>> {
        if dsh_source_path_is_zstd(session_path) {
            Ok(zstd::encode_all(std::io::Cursor::new(&content[..]), 3).map_err(AppError::from)?)
        } else {
            Ok(content)
        }
    }

    /// 启动 PTY 不需要临时配置。
    fn build_temp_settings(&self) -> AppResult<Option<String>> {
        Ok(None)
    }

    /// 本 CLI 的钩子事件无需转给本应用。
    fn needs_hook_relay(&self) -> bool {
        false
    }

    /// DSH 的转录暂无提取格式。
    fn transcript_format(&self) -> Option<transcript_store::extract::CliFormat> {
        None
    }
}

/// DSH 源路径后缀判定：`.zstd` / `.zst` 视为 zstd 压缩源文件（与 dsh 解析器同判）。
fn dsh_source_path_is_zstd(session_path: &str) -> bool {
    let name = Path::new(session_path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    name.ends_with(".zstd") || name.ends_with(".zst")
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
) -> Option<SessionListMetadata> {
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
    Some(SessionListMetadata {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 内层解码的两条路径判据不同，且差异是刻意的：
    /// 完整读取（`Complete`）遇不完整帧视为数据损坏，原样返回；
    /// 截断读取（`Truncated`）遇不完整帧要保留已解出的明文头部。
    ///
    /// 用「完整帧 + 半个帧」拼出被截断的流：第一个帧必定完整解出，
    /// 第二个帧解到一半即遇 EOF，从而稳定复现两臂的分歧。
    #[test]
    fn decode_archived_source_keeps_head_only_when_truncated() {
        let part1 = "{\"type\":\"user/message\",\"seq\":1}\n".repeat(500);
        let part2 = "{\"type\":\"assistant/message\",\"seq\":2}\n".repeat(500);
        let frame1 = zstd::encode_all(std::io::Cursor::new(part1.as_bytes()), 3).unwrap();
        let frame2 = zstd::encode_all(std::io::Cursor::new(part2.as_bytes()), 3).unwrap();
        let mut truncated = frame1;
        truncated.extend_from_slice(&frame2[..frame2.len() / 2]);

        let source = DshSource;
        assert_eq!(
            source.decode_archived_source(truncated.clone(), Truncation::Complete),
            truncated,
            "完整路径下不完整帧应原样返回"
        );
        let head = source.decode_archived_source(truncated, Truncation::Truncated);
        assert!(head.starts_with(part1.as_bytes()), "截断路径应保留已完整解出的明文前缀");
    }
}
