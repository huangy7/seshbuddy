use crate::app_db;
use crate::cli::CliKind;
use crate::commands::session::{build_codex_search_results, AggregatedSessionSearch};
use crate::commands::session_index::{
    build_projects_scan, build_projects_snapshot, cached_session_list_metadata, file_modified_ms,
    RawSession, Resolution, ScanProjection, SnapshotProjection,
};
use crate::error::AppResult;
use crate::parser::grok;
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

pub(crate) struct GrokSource;

static DESCRIPTOR: CliDescriptor = CliDescriptor {
    name: "Grok",
    tray_label: "Grok",
    proxy_port: crate::cli_registry::NO_PROXY_PORT,
    command: "grok",
    file_root: FileRoot {
        data_dir_segments: &[".grok"],
        sessions_subdir: Some("sessions"),
    },
};

static GROK_USAGE_STATS: GrokUsageStats = GrokUsageStats;
struct GrokUsageStats;
impl crate::cli_registry::features::UsageStatsFeature for GrokUsageStats {
    fn usage_records(&self, key: &str, project: &str) -> Vec<UsageRecord> {
        grok::extract_usage_records(key, project)
    }
}

impl CliSource for GrokSource {
    fn descriptor(&self) -> &'static CliDescriptor {
        &DESCRIPTOR
    }

    fn owns(&self, key: &str) -> bool {
        under_data_root(key, CliKind::Grok)
    }

    fn exists(&self, loc: &SessionLocator) -> bool {
        file_backed_exists(loc)
    }

    fn can_delete(&self) -> bool {
        true
    }

    fn discover(&self) -> AppResult<Discovered> {
        let sessions_dir = crate::cli::sessions_dir(CliKind::Grok)?;
        if !sessions_dir.exists() {
            return Ok(Discovered {
                locators: Vec::new(),
                unverified_prefixes: Vec::new(),
            });
        }
        let mut locators = Vec::new();
        let mut unverified_prefixes = Vec::new();

        let cwd_entries = match fs::read_dir(&sessions_dir) {
            Ok(entries) => entries,
            Err(_) => {
                return Ok(Discovered {
                    locators,
                    unverified_prefixes,
                })
            }
        };

        for cwd_entry in cwd_entries.flatten() {
            let cwd_path = cwd_entry.path();
            if !cwd_path.is_dir() {
                continue;
            }

            let session_entries =
                match read_dir_or_record_unverified(&cwd_path, &mut unverified_prefixes) {
                    Some(entries) => entries,
                    None => continue,
                };

            for session_entry in session_entries.flatten() {
                let session_dir = session_entry.path();
                if !session_dir.is_dir() {
                    continue;
                }

                let history_path = session_dir.join("chat_history.jsonl");
                if history_path.exists() {
                    locators.push(SessionLocator::File {
                        cli_id: CliKind::Grok,
                        path: history_path,
                    });
                }
            }
        }

        Ok(Discovered {
            locators,
            unverified_prefixes,
        })
    }

    fn snapshot(
        &self,
        custom_names: &HashMap<String, String>,
        page: usize,
        page_size: usize,
    ) -> AppResult<Option<PaginatedProjects>> {
        build_projects_snapshot(
            CliKind::Grok,
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
        let sessions_dir = crate::cli::sessions_dir(CliKind::Grok)?;
        if !sessions_dir.exists() {
            return Ok(Vec::new());
        }

        let discovered = self.discover()?;

        build_projects_scan(
            CliKind::Grok,
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
                if file_name != "chat_history.jsonl"
                    || crate::parser::is_subagent_session(&session_path.to_string_lossy())
                {
                    return Ok(Resolution::Skip);
                }
                let file_path = loc.to_key();
                let session_id = crate::parser::read_session_id(&file_path).unwrap_or_default();
                if ctx.tombstone_filter.is_tombstoned(&session_id, &file_path) {
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
        grok::parse_session_file(key)
    }

    fn first_user_message(&self, key: &str) -> Option<String> {
        grok::read_first_user_message(key)
    }

    fn session_id(&self, key: &str) -> Option<String> {
        grok::read_session_id(key)
    }

    fn project_path_raw(&self, key: &str) -> Option<String> {
        grok::read_project_path(key)
    }

    fn metadata_only(&self, key: &str) -> Option<SessionListMetadata> {
        grok::scan_session_metadata_only(Path::new(key))
    }

    fn is_subagent(&self, key: &str) -> bool {
        grok::is_subagent_file(key)
    }

    fn parse_streaming(
        &self,
        key: &str,
        skip_sidechain: bool,
        on_batch: &mut dyn FnMut(Vec<ChatMessage>) -> bool,
    ) -> AppResult<(u64, HashMap<String, SubagentInfo>)> {
        grok::parse_session_file_streaming(key, skip_sidechain, on_batch)
    }

    fn parse_incremental(
        &self,
        key: &str,
        offset: u64,
        skip_sidechain: bool,
    ) -> AppResult<SessionLoadResult> {
        grok::parse_session_incremental(key, offset, skip_sidechain)
    }

    fn parse_from_content(&self, content: &str) -> Vec<ChatMessage> {
        grok::parse_from_content(content)
    }

    fn search_docs(
        &self,
        key: &str,
        on_progress: &mut dyn FnMut(SearchScanProgress),
    ) -> Option<Vec<SearchDocument>> {
        grok::scan_session_search_docs_with_progress(Path::new(key), on_progress)
    }

    fn search_docs_from_bytes(&self, content: &[u8]) -> Option<Vec<SearchDocument>> {
        grok::scan_session_search_docs_from_bytes(content)
    }

    fn new_session(&self) -> Option<&'static dyn LaunchFeature> {
        Some(&GROK_LAUNCH)
    }

    fn resume_session(&self) -> Option<&'static dyn LaunchFeature> {
        Some(&GROK_LAUNCH)
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
        Some(&GROK_USAGE_STATS)
    }

    fn is_session_event_path(&self, path: &Path, sessions_dir: &Path) -> bool {
        let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if file_name == "chat_history.jsonl"
            || file_name == "summary.json"
            || file_name == "updates.jsonl"
        {
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

    fn tombstone_fallback_id(&self, session_path: &Path) -> String {
        grok::read_session_id(&session_path.to_string_lossy()).unwrap_or_else(|| {
            session_path
                .parent()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string()
        })
    }

    fn path_only_session_id(&self, session_path: &str) -> Option<String> {
        Path::new(session_path)
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .map(|s| s.to_string())
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
            CliKind::Grok,
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

struct GrokLaunch;
static GROK_LAUNCH: GrokLaunch = GrokLaunch;

impl LaunchFeature for GrokLaunch {
    fn plan(&self, req: &LaunchRequest<'_>) -> AppResult<LaunchPlan> {
        let mut args = Vec::new();
        if let Some(id) = req.session_id {
            args.push("--resume".to_string());
            args.push(id.to_string());
        }
        Ok(LaunchPlan::CommandLine { args })
    }
}
