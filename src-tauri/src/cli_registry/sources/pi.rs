//! Pi 会话数据源。
//!
//! Pi CLI 会话转录文件存放在 `~/.pi/agent/sessions/` 目录下（单会话对应一个 `.jsonl` 文件）。
//! 恢复会话通过命令 `pi --session <id>` 启动。

use crate::app_db;
use crate::cli::CliKind;
use crate::commands::session::{build_codex_search_results, AggregatedSessionSearch};
use crate::commands::session_index::{
    build_projects_scan, build_projects_snapshot, cached_session_list_metadata, file_modified_ms,
    RawSession, Resolution, ScanProjection, SnapshotProjection,
};
use crate::error::AppResult;
use crate::parser::pi;
use crate::parser::shared::{SearchDocument, SearchScanProgress};
use crate::session::{
    ChatMessage, PaginatedProjects, SearchResult, SessionListMetadata, SessionLoadResult,
    SubagentInfo,
};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use super::super::descriptor::{CliDescriptor, FileRoot};
use super::super::features::{LaunchFeature, LaunchPlan, LaunchRequest};
use super::super::source::{
    collect_jsonl_files, file_backed_exists, under_data_root, CliSource, Truncation,
};
use super::super::{Discovered, SessionLocator};

pub(crate) struct PiSource;

static DESCRIPTOR: CliDescriptor = CliDescriptor {
    name: "Pi",
    tray_label: "Pi",
    proxy_port: crate::cli_registry::NO_PROXY_PORT,
    command: "pi",
    file_root: FileRoot {
        data_dir_segments: &[".pi", "agent"],
        sessions_subdir: Some("sessions"),
    },
};

static PI_LAUNCH: PiLaunch = PiLaunch;

struct PiLaunch;

impl LaunchFeature for PiLaunch {
    fn plan(&self, req: &LaunchRequest<'_>) -> AppResult<LaunchPlan> {
        let mut args = Vec::new();
        if let Some(id) = req.session_id {
            args.push("--session".to_string());
            args.push(id.to_string());
        }
        Ok(LaunchPlan::CommandLine { args })
    }
}

/// 提取 Pi 会话文件的头部 session_id 与文件最后修改时间。
fn read_pi_identity(path: &Path) -> Option<(String, SystemTime)> {
    let id = pi::session_id(path)?;
    let modified = fs::metadata(path).ok()?.modified().ok()?;
    Some((id, modified))
}

impl CliSource for PiSource {
    fn descriptor(&self) -> &'static CliDescriptor {
        &DESCRIPTOR
    }

    fn owns(&self, key: &str) -> bool {
        under_data_root(key, CliKind::Pi)
    }

    fn exists(&self, loc: &SessionLocator) -> bool {
        file_backed_exists(loc)
    }

    fn can_delete(&self) -> bool {
        true
    }

    /// 枚举 `~/.pi/agent/sessions/` 下所有 `.jsonl` 文件。
    /// 若同一会话复制跨越不同子路径，则仅保留最后修改时间最新的文件。
    fn discover(&self) -> AppResult<Discovered> {
        let sessions_dir = crate::cli::sessions_dir(CliKind::Pi)?;
        if !sessions_dir.exists() {
            return Ok(Discovered {
                locators: Vec::new(),
                unverified_prefixes: Vec::new(),
            });
        }

        let mut all_files = Vec::new();
        collect_jsonl_files(&sessions_dir, &mut all_files)?;

        let mut newest_by_id: HashMap<String, (SystemTime, PathBuf)> = HashMap::new();
        let mut unidentifiable = Vec::new();

        for path in all_files {
            if let Some((id, mtime)) = read_pi_identity(&path) {
                match newest_by_id.get(&id) {
                    Some((existing_mtime, _)) if mtime <= *existing_mtime => {
                        continue;
                    }
                    _ => {
                        newest_by_id.insert(id, (mtime, path));
                    }
                }
            } else {
                unidentifiable.push(path);
            }
        }

        let mut locators: Vec<SessionLocator> = newest_by_id
            .into_values()
            .map(|(_, path)| SessionLocator::File {
                cli_id: CliKind::Pi,
                path,
            })
            .collect();

        for path in unidentifiable {
            locators.push(SessionLocator::File {
                cli_id: CliKind::Pi,
                path,
            });
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
            CliKind::Pi,
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
        let sessions_dir = crate::cli::sessions_dir(CliKind::Pi)?;
        if !sessions_dir.exists() {
            return Ok(Vec::new());
        }

        let discovered = self.discover()?;

        build_projects_scan(
            CliKind::Pi,
            custom_names,
            force,
            discovered,
            |loc, ctx| {
                let Some(session_path) = loc.as_file_path() else {
                    return Ok(Resolution::Skip);
                };
                let file_path = loc.to_key();
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
                    .filter(|path| !path.trim().is_empty());
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
        pi::parse_session_file(Path::new(key))
    }

    fn first_user_message(&self, key: &str) -> Option<String> {
        pi::first_user_message(Path::new(key))
    }

    fn session_id(&self, key: &str) -> Option<String> {
        pi::session_id(Path::new(key))
    }

    fn project_path_raw(&self, key: &str) -> Option<String> {
        pi::project_path_raw(Path::new(key))
    }

    fn metadata_only(&self, key: &str) -> Option<SessionListMetadata> {
        pi::metadata_only(Path::new(key))
    }

    fn is_subagent(&self, key: &str) -> bool {
        pi::is_subagent(Path::new(key))
    }

    fn parse_streaming(
        &self,
        key: &str,
        _skip_sidechain: bool,
        on_batch: &mut dyn FnMut(Vec<ChatMessage>) -> bool,
    ) -> AppResult<(u64, HashMap<String, SubagentInfo>)> {
        let messages = pi::parse_session_file(Path::new(key))?;
        let offset = fs::metadata(key).map(|m| m.len()).unwrap_or(0);
        on_batch(messages);
        Ok((offset, HashMap::new()))
    }

    fn parse_incremental(
        &self,
        key: &str,
        _offset: u64,
        _skip_sidechain: bool,
    ) -> AppResult<SessionLoadResult> {
        pi::load_messages(Path::new(key))
    }

    fn parse_from_content(&self, content: &str) -> Vec<ChatMessage> {
        pi::parse_from_content(content)
    }

    fn search_docs(
        &self,
        key: &str,
        on_progress: &mut dyn FnMut(SearchScanProgress),
    ) -> Option<Vec<SearchDocument>> {
        pi::scan_search_docs(Path::new(key), on_progress)
    }

    fn search_docs_from_bytes(&self, content: &[u8]) -> Option<Vec<SearchDocument>> {
        pi::search_docs_from_text(content)
    }

    fn new_session(&self) -> Option<&'static dyn LaunchFeature> {
        Some(&PI_LAUNCH)
    }

    fn resume_session(&self) -> Option<&'static dyn LaunchFeature> {
        Some(&PI_LAUNCH)
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
        path.extension().and_then(|e| e.to_str()) == Some("jsonl")
    }

    fn locate_executable(&self) -> Option<String> {
        crate::cli::find_on_path(self.descriptor().command)
    }

    fn tombstone_fallback_id(&self, session_path: &Path) -> String {
        self.session_id(&session_path.to_string_lossy()).unwrap_or_default()
    }

    fn path_only_session_id(&self, _session_path: &str) -> Option<String> {
        None
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
            CliKind::Pi,
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
    fn launch_plan_formats_session_argument() {
        let launch = PiLaunch;
        let plan = launch.plan(&LaunchRequest {
            session_id: Some("sess-123"),
            skip_permissions: false,
            settings_file: None,
        }).unwrap();

        match plan {
            LaunchPlan::CommandLine { args } => {
                assert_eq!(args, vec!["--session", "sess-123"]);
            }
            _ => panic!("expected command line plan"),
        }
    }
}
