//! Kimi 会话数据源。
//!
//! Kimi 会话日志存放在 `~/.kimi-code/sessions/` 或 `~/.kimi/sessions/` 目录下。
//! 会话结构为 `<session_dir>/agents/<agent_name>/wire.jsonl`。
//! 启动新建会话使用 `kimi`，恢复会话使用 `kimi --session <session_id>`。

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::app_db;
use crate::cli::CliKind;
use crate::commands::session::{build_codex_search_results, AggregatedSessionSearch};
use crate::commands::session_index::{
    build_projects_scan, build_projects_snapshot, cached_session_list_metadata, file_modified_ms,
    RawSession, Resolution, ScanProjection, SnapshotProjection,
};
use crate::error::AppResult;
use crate::parser::kimi;
use crate::parser::shared::{SearchDocument, SearchScanProgress};
use crate::session::{
    ChatMessage, PaginatedProjects, SearchResult, SessionListMetadata, SessionLoadResult,
    SubagentInfo,
};

use super::super::descriptor::{CliDescriptor, FileRoot};
use super::super::features::{LaunchFeature, LaunchPlan, LaunchRequest};
use super::super::source::{file_backed_exists, under_data_root, CliSource, Truncation};
use super::super::{Discovered, SessionLocator};

pub(crate) struct KimiSource;

static DESCRIPTOR: CliDescriptor = CliDescriptor {
    name: "Kimi",
    tray_label: "Kimi",
    proxy_port: crate::cli_registry::NO_PROXY_PORT,
    command: "kimi",
    file_root: FileRoot {
        data_dir_segments: &[".kimi-code"],
        sessions_subdir: Some("sessions"),
    },
};

static KIMI_NEW_LAUNCH: KimiNewLaunch = KimiNewLaunch;
struct KimiNewLaunch;

impl LaunchFeature for KimiNewLaunch {
    fn plan(&self, _req: &LaunchRequest<'_>) -> AppResult<LaunchPlan> {
        Ok(LaunchPlan::CommandLine { args: Vec::new() })
    }
}

static KIMI_RESUME_LAUNCH: KimiResumeLaunch = KimiResumeLaunch;
struct KimiResumeLaunch;

impl LaunchFeature for KimiResumeLaunch {
    fn plan(&self, req: &LaunchRequest<'_>) -> AppResult<LaunchPlan> {
        let Some(session_id) = req.session_id else {
            return Ok(LaunchPlan::CommandLine { args: Vec::new() });
        };

        // 对于子代理 id `<parent>:<agent>`，Kimi CLI 仅支持恢复父会话目录名
        let resume_target = match session_id.split_once(':') {
            Some((parent, _)) => parent,
            None => session_id,
        };

        Ok(LaunchPlan::CommandLine {
            args: vec!["--session".to_string(), resume_target.to_string()],
        })
    }
}

/// 递归查找 `sessions/` 目录下的所有 `wire.jsonl` 文件。
fn collect_kimi_files(dir: &Path, files: &mut Vec<PathBuf>) -> AppResult<()> {
    if !dir.is_dir() {
        return Ok(());
    }

    for entry in fs::read_dir(dir)? {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let path = entry.path();
        if path.is_dir() {
            let dir_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if dir_name == ".git" || dir_name == "node_modules" || dir_name == "caches" {
                continue;
            }
            collect_kimi_files(&path, files)?;
        } else if path.is_file() {
            let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if file_name == "wire.jsonl" || file_name.ends_with(".jsonl") {
                files.push(path);
            }
        }
    }

    Ok(())
}

impl CliSource for KimiSource {
    fn descriptor(&self) -> &'static CliDescriptor {
        &DESCRIPTOR
    }

    fn owns(&self, key: &str) -> bool {
        under_data_root(key, CliKind::Kimi)
            || key.contains("/.kimi-code/")
            || key.contains("/.kimi/")
            || key.contains("\\.kimi-code\\")
            || key.contains("\\.kimi\\")
    }

    fn exists(&self, loc: &SessionLocator) -> bool {
        file_backed_exists(loc)
    }

    fn can_delete(&self) -> bool {
        true
    }

    fn discover(&self) -> AppResult<Discovered> {
        let mut locators = Vec::new();

        // 优先检查默认 sessions 目录 (~/.kimi-code/sessions)
        if let Ok(sessions_dir) = crate::cli::sessions_dir(CliKind::Kimi) {
            if sessions_dir.exists() {
                let mut files = Vec::new();
                let _ = collect_kimi_files(&sessions_dir, &mut files);
                for path in files {
                    locators.push(SessionLocator::File {
                        cli_id: CliKind::Kimi,
                        path,
                    });
                }
            }
        }

        // 兼容检查备选目录 ~/.kimi/sessions
        if let Some(home) = dirs::home_dir() {
            let alt_sessions = home.join(".kimi").join("sessions");
            if alt_sessions.exists() && alt_sessions != crate::cli::sessions_dir(CliKind::Kimi).unwrap_or_default() {
                let mut files = Vec::new();
                let _ = collect_kimi_files(&alt_sessions, &mut files);
                for path in files {
                    locators.push(SessionLocator::File {
                        cli_id: CliKind::Kimi,
                        path,
                    });
                }
            }
        }

        Ok(Discovered {
            locators,
            unverified_prefixes: Vec::new(),
        })
    }

    fn snapshot(
        &self,
        custom_names: &HashMap<String, String>,
        page: usize,
        page_size: usize,
    ) -> AppResult<Option<PaginatedProjects>> {
        build_projects_snapshot(
            CliKind::Kimi,
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

    fn scan(
        &self,
        custom_names: &HashMap<String, String>,
        force: bool,
    ) -> AppResult<Vec<RawSession>> {
        let sessions_dir = crate::cli::sessions_dir(CliKind::Kimi)?;
        let alt_sessions = dirs::home_dir().map(|h| h.join(".kimi").join("sessions"));
        let alt_exists = alt_sessions.as_ref().map(|p| p.exists()).unwrap_or(false);
        if !sessions_dir.exists() && !alt_exists {
            return Ok(Vec::new());
        }

        let discovered = self.discover()?;

        build_projects_scan(
            CliKind::Kimi,
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
                if file_name != "wire.jsonl" && !file_name.ends_with(".jsonl") {
                    return Ok(Resolution::Skip);
                }
                // 子代理会话不单独作为顶层会话列出（列表只收录主会话）
                if kimi::is_subagent(session_path) {
                    return Ok(Resolution::Skip);
                }
                let file_path = loc.to_key();
                let session_id = kimi::session_id_from_path(session_path);
                if ctx.tombstone_filter.is_tombstoned(&session_id, &file_path) {
                    return Ok(Resolution::Skip);
                }
                let file_metadata = match fs::metadata(session_path) {
                    Ok(m) if m.is_file() => m,
                    _ => return Ok(Resolution::Skip),
                };
                let file_size = file_metadata.len();
                let modified_ms = file_modified_ms(&file_metadata);
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
                let original_path = metadata
                    .project_path
                    .clone()
                    .filter(|p| !p.trim().is_empty());
                let encoded_dir = original_path.clone();
                Ok(Resolution::Session {
                    projection: ScanProjection {
                        project_path: original_path.clone(),
                        encoded_dir,
                        original_path,
                        title: metadata.title.clone(),
                        history_display: None,
                        git_branch: String::new(),
                    },
                    metadata,
                })
            },
        )
    }

    fn parse(&self, key: &str) -> AppResult<Vec<ChatMessage>> {
        kimi::parse_session_file(Path::new(key))
    }

    fn first_user_message(&self, key: &str) -> Option<String> {
        kimi::first_user_message(Path::new(key))
    }

    fn session_id(&self, key: &str) -> Option<String> {
        kimi::session_id(Path::new(key))
    }

    fn project_path_raw(&self, key: &str) -> Option<String> {
        kimi::project_path_raw(Path::new(key))
    }

    fn metadata_only(&self, key: &str) -> Option<SessionListMetadata> {
        kimi::metadata_only(Path::new(key))
    }

    fn is_subagent(&self, key: &str) -> bool {
        kimi::is_subagent(Path::new(key))
    }

    fn parse_streaming(
        &self,
        key: &str,
        _skip_sidechain: bool,
        on_batch: &mut dyn FnMut(Vec<ChatMessage>) -> bool,
    ) -> AppResult<(u64, HashMap<String, SubagentInfo>)> {
        let messages = self.parse(key)?;
        let offset = messages.len() as u64;
        let mut on_batch = on_batch;
        let mut emitter = crate::parser::batch::BatchEmitter::new();
        for message in messages {
            emitter.push(message);
            if !emitter.maybe_flush(&mut on_batch) {
                break;
            }
        }
        emitter.flush_remaining(&mut on_batch);
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
        let start = offset.min(total) as usize;
        let messages = messages.into_iter().skip(start).collect();
        Ok(SessionLoadResult {
            messages,
            offset: total,
            subagent_map: HashMap::new(),
        })
    }

    fn parse_from_content(&self, content: &str) -> Vec<ChatMessage> {
        kimi::parse_from_content(content)
    }

    fn search_docs(
        &self,
        key: &str,
        on_progress: &mut dyn FnMut(SearchScanProgress),
    ) -> Option<Vec<SearchDocument>> {
        kimi::scan_search_docs(Path::new(key), on_progress)
    }

    fn search_docs_from_bytes(&self, content: &[u8]) -> Option<Vec<SearchDocument>> {
        kimi::search_docs_from_text(content)
    }

    fn new_session(&self) -> Option<&'static dyn LaunchFeature> {
        Some(&KIMI_NEW_LAUNCH)
    }

    fn resume_session(&self) -> Option<&'static dyn LaunchFeature> {
        Some(&KIMI_RESUME_LAUNCH)
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
        None
    }

    fn is_session_event_path(&self, path: &Path, _sessions_dir: &Path) -> bool {
        path.file_name()
            .and_then(|n| n.to_str())
            .map(|name| name == "wire.jsonl" || name.ends_with(".jsonl"))
            .unwrap_or(false)
    }

    fn locate_executable(&self) -> Option<String> {
        crate::cli::find_on_path(self.descriptor().command)
    }

    fn tombstone_fallback_id(&self, session_path: &Path) -> String {
        kimi::session_id_from_path(session_path)
    }

    fn path_only_session_id(&self, session_path: &str) -> Option<String> {
        Some(kimi::session_id_from_path(Path::new(session_path)))
    }

    fn custom_titles(&self) -> HashMap<String, String> {
        HashMap::new()
    }

    fn index_titles(&self) -> Option<HashMap<String, String>> {
        None
    }

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
            CliKind::Kimi,
        ))
    }

    fn decode_archived_source(&self, bytes: Vec<u8>, _truncation: Truncation) -> Vec<u8> {
        bytes
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_plan_formats_resume_argument() {
        let plan = KIMI_RESUME_LAUNCH
            .plan(&LaunchRequest {
                session_id: Some("ses_abc123"),
                skip_permissions: false,
                settings_file: None,
            })
            .expect("生成启动计划");

        match plan {
            LaunchPlan::CommandLine { args } => {
                assert_eq!(args, vec!["--session".to_string(), "ses_abc123".to_string()]);
            }
            _ => panic!("Expected CommandLine plan"),
        }
    }

    #[test]
    fn launch_plan_subagent_resumes_parent() {
        let plan = KIMI_RESUME_LAUNCH
            .plan(&LaunchRequest {
                session_id: Some("ses_abc123:agent-0"),
                skip_permissions: false,
                settings_file: None,
            })
            .expect("生成启动计划");

        match plan {
            LaunchPlan::CommandLine { args } => {
                assert_eq!(args, vec!["--session".to_string(), "ses_abc123".to_string()]);
            }
            _ => panic!("Expected CommandLine plan"),
        }
    }

    #[test]
    fn launch_plan_new_session_is_empty_args() {
        let plan = KIMI_NEW_LAUNCH
            .plan(&LaunchRequest {
                session_id: None,
                skip_permissions: false,
                settings_file: None,
            })
            .expect("生成新建计划");

        match plan {
            LaunchPlan::CommandLine { args } => {
                assert!(args.is_empty());
            }
            _ => panic!("Expected CommandLine plan"),
        }
    }
}
