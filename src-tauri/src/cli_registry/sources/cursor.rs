//! Cursor CLI 会话源。
//!
//! Cursor 会话转录在物理磁盘上存放在 `~/.cursor/projects/<project-key>/agent-transcripts/` 下。
//! 终端 `agent` CLI 与编辑器内部 Composer 均会向该目录写转录，但**只有终端 CLI 启动的会话才会在
//! `~/.cursor/chats/<md5>/<sessionId>/store.db` 下生成 Sidecar 数据库**。
//!
//! 本模块以此建立严格白名单门禁：扫描时优先提取包含 `store.db` 的会话集合，
//! 仅收录白名单内的终端 CLI 会话，将 IDE 内部 Composer 会话坚决过滤。

use crate::app_db;
use crate::cli::CliKind;
use crate::commands::session::{build_codex_search_results, AggregatedSessionSearch};
use crate::commands::session_index::{
    build_projects_scan, build_projects_snapshot, cached_session_list_metadata, file_modified_ms,
    RawSession, Resolution, ScanProjection, SnapshotProjection,
};
use crate::error::AppResult;
use crate::parser::cursor;
use crate::parser::shared::{SearchDocument, SearchScanProgress};
use crate::session::{
    ChatMessage, PaginatedProjects, SearchResult, SessionListMetadata, SessionLoadResult,
    SubagentInfo,
};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use super::super::descriptor::{CliDescriptor, FileRoot};
use super::super::features::{LaunchFeature, LaunchPlan, LaunchRequest};
use super::super::source::{file_backed_exists, under_data_root, CliSource, Truncation};
use super::super::{Discovered, SessionLocator};

pub(crate) struct CursorSource;

static DESCRIPTOR: CliDescriptor = CliDescriptor {
    name: "Cursor Agent",
    tray_label: "Cursor",
    proxy_port: crate::cli_registry::NO_PROXY_PORT,
    command: "agent",
    file_root: FileRoot {
        data_dir_segments: &[".cursor"],
        sessions_subdir: Some("projects"),
    },
};

static CURSOR_LAUNCH: CursorLaunch = CursorLaunch;

struct CursorLaunch;

impl LaunchFeature for CursorLaunch {
    fn plan(&self, req: &LaunchRequest<'_>) -> AppResult<LaunchPlan> {
        let mut args = Vec::new();
        if let Some(id) = req.session_id {
            args.push(format!("--resume={id}"));
        }
        Ok(LaunchPlan::CommandLine { args })
    }
}

/// 扫描 `~/.cursor/chats/<md5>/<sessionId>/store.db` 收集所有 CLI 终端会话 ID 白名单。
pub(crate) fn collect_cli_session_whitelist(home_dir: &Path) -> HashSet<String> {
    let chats_dir = home_dir.join(".cursor").join("chats");
    let mut whitelist = HashSet::new();
    let Ok(buckets) = fs::read_dir(&chats_dir) else {
        return whitelist;
    };
    for bucket in buckets.flatten() {
        let bucket_dir = bucket.path();
        if !bucket_dir.is_dir() {
            continue;
        }
        let Ok(sessions) = fs::read_dir(&bucket_dir) else {
            continue;
        };
        for session in sessions.flatten() {
            let session_dir = session.path();
            if session_dir.is_dir() && session_dir.join("store.db").is_file() {
                if let Some(id) = session_dir.file_name().and_then(|n| n.to_str()) {
                    whitelist.insert(id.to_string());
                }
            }
        }
    }
    whitelist
}

impl CliSource for CursorSource {
    fn descriptor(&self) -> &'static CliDescriptor {
        &DESCRIPTOR
    }

    fn owns(&self, key: &str) -> bool {
        under_data_root(key, CliKind::Cursor)
    }

    fn exists(&self, loc: &SessionLocator) -> bool {
        file_backed_exists(loc)
    }

    fn can_delete(&self) -> bool {
        true
    }

    /// 枚举转录文件，并应用 `store.db` 白名单严格过滤。
    fn discover(&self) -> AppResult<Discovered> {
        let projects_dir = crate::cli::sessions_dir(CliKind::Cursor)?;
        if !projects_dir.exists() {
            return Ok(Discovered {
                locators: Vec::new(),
                unverified_prefixes: Vec::new(),
            });
        }

        let home_dir = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        let cli_whitelist = collect_cli_session_whitelist(&home_dir);

        let mut locators = Vec::new();
        let Ok(project_entries) = fs::read_dir(&projects_dir) else {
            return Ok(Discovered {
                locators,
                unverified_prefixes: Vec::new(),
            });
        };

        for project in project_entries.flatten() {
            let transcripts_dir = project.path().join("agent-transcripts");
            if !transcripts_dir.is_dir() {
                continue;
            }
            let Ok(session_entries) = fs::read_dir(&transcripts_dir) else {
                continue;
            };
            for session in session_entries.flatten() {
                let session_dir = session.path();
                if !session_dir.is_dir() {
                    continue;
                }
                let sid = session_dir
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("");
                // 核心门禁：仅保留存在 store.db 的 CLI 会话，跳过无 store.db 的 IDE 会话
                if !cli_whitelist.contains(sid) {
                    continue;
                }

                let main_file = session_dir.join(format!("{sid}.jsonl"));
                if main_file.is_file() {
                    locators.push(SessionLocator::File {
                        cli_id: CliKind::Cursor,
                        path: main_file,
                    });
                }

                // 子代理文件
                for sub_path in cursor::subagent_paths_under(&session_dir) {
                    locators.push(SessionLocator::File {
                        cli_id: CliKind::Cursor,
                        path: sub_path,
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
            CliKind::Cursor,
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
        let projects_dir = crate::cli::sessions_dir(CliKind::Cursor)?;
        if !projects_dir.exists() {
            return Ok(Vec::new());
        }

        let discovered = self.discover()?;

        build_projects_scan(
            CliKind::Cursor,
            custom_names,
            force,
            discovered,
            |loc, ctx| {
                let Some(session_path) = loc.as_file_path() else {
                    return Ok(Resolution::Skip);
                };

                // 子代理不作为顶层会话独立上屏
                if cursor::is_subagent_session(&session_path.to_string_lossy()) {
                    return Ok(Resolution::Skip);
                }

                let file_path = loc.to_key();
                let session_id = session_path
                    .file_stem()
                    .and_then(|n| n.to_str())
                    .unwrap_or("");
                if ctx.tombstone_filter.is_tombstoned(session_id, &file_path) {
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
                .or_else(|| cursor::scan_session_metadata_only(session_path));

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
        cursor::parse_session_file(key)
    }

    fn first_user_message(&self, key: &str) -> Option<String> {
        cursor::read_first_user_message(key)
    }

    fn session_id(&self, key: &str) -> Option<String> {
        cursor::read_session_id(key)
    }

    fn project_path_raw(&self, key: &str) -> Option<String> {
        cursor::read_project_path(key)
    }

    fn metadata_only(&self, key: &str) -> Option<SessionListMetadata> {
        cursor::scan_session_metadata_only(Path::new(key))
    }

    fn is_subagent(&self, key: &str) -> bool {
        cursor::is_subagent_session(key)
    }

    fn parse_streaming(
        &self,
        key: &str,
        skip_sidechain: bool,
        on_batch: &mut dyn FnMut(Vec<ChatMessage>) -> bool,
    ) -> AppResult<(u64, HashMap<String, SubagentInfo>)> {
        cursor::parse_session_streaming(key, skip_sidechain, on_batch)
    }

    fn parse_incremental(
        &self,
        key: &str,
        offset: u64,
        skip_sidechain: bool,
    ) -> AppResult<SessionLoadResult> {
        cursor::parse_session_incremental(key, offset, skip_sidechain)
    }

    fn parse_from_content(&self, content: &str) -> Vec<ChatMessage> {
        cursor::parse_from_content(content)
    }

    fn search_docs(
        &self,
        key: &str,
        on_progress: &mut dyn FnMut(SearchScanProgress),
    ) -> Option<Vec<SearchDocument>> {
        cursor::scan_search_docs(Path::new(key), on_progress)
    }

    fn search_docs_from_bytes(&self, content: &[u8]) -> Option<Vec<SearchDocument>> {
        cursor::search_docs_from_text(content)
    }

    fn new_session(&self) -> Option<&'static dyn LaunchFeature> {
        Some(&CURSOR_LAUNCH)
    }

    fn resume_session(&self) -> Option<&'static dyn LaunchFeature> {
        Some(&CURSOR_LAUNCH)
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

    fn is_session_event_path(&self, path: &Path, sessions_dir: &Path) -> bool {
        if !path.starts_with(sessions_dir) {
            return false;
        }
        path.extension().and_then(|ext| ext.to_str()) == Some("jsonl")
            && path
                .components()
                .any(|c| c.as_os_str() == "agent-transcripts")
    }

    fn locate_executable(&self) -> Option<String> {
        // 优先检查 PATH 中的 `agent` 二进制
        if let Ok(output) = std::process::Command::new("which").arg("agent").output() {
            if output.status.success() {
                let bin = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !bin.is_empty() {
                    return Some(bin);
                }
            }
        }
        // 检查常见安装位置 ~/.local/bin/agent
        if let Some(home) = dirs::home_dir() {
            let local_bin = home.join(".local").join("bin").join("agent");
            if local_bin.is_file() {
                return Some(local_bin.to_string_lossy().to_string());
            }
        }
        None
    }

    fn tombstone_fallback_id(&self, session_path: &Path) -> String {
        session_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string()
    }

    fn path_only_session_id(&self, session_path: &str) -> Option<String> {
        Path::new(session_path)
            .file_stem()
            .and_then(|s| s.to_str())
            .map(ToString::to_string)
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
            CliKind::Cursor,
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
    fn launch_plan_builds_resume_flag() {
        let plan = CURSOR_LAUNCH
            .plan(&LaunchRequest {
                session_id: Some("sess-123"),
                skip_permissions: false,
                settings_file: None,
            })
            .unwrap();

        match plan {
            LaunchPlan::CommandLine { args } => {
                assert_eq!(args, vec!["--resume=sess-123".to_string()]);
            }
            _ => panic!("expected command line plan"),
        }
    }

    #[test]
    fn discover_skips_ide_session_without_store_db() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();

        // 构造 projects/ProjA/agent-transcripts/ide-ses/ide-ses.jsonl (无 store.db)
        let ide_dir = home
            .join(".cursor")
            .join("projects")
            .join("ProjA")
            .join("agent-transcripts")
            .join("ide-ses");
        fs::create_dir_all(&ide_dir).unwrap();
        fs::write(ide_dir.join("ide-ses.jsonl"), r#"{"role":"user","message":{"content":[{"type":"text","text":"hi"}]}}"#).unwrap();

        // 构造 projects/ProjB/agent-transcripts/cli-ses/cli-ses.jsonl 并在 chats/ 建立对应 store.db
        let cli_dir = home
            .join(".cursor")
            .join("projects")
            .join("ProjB")
            .join("agent-transcripts")
            .join("cli-ses");
        fs::create_dir_all(&cli_dir).unwrap();
        fs::write(cli_dir.join("cli-ses.jsonl"), r#"{"role":"user","message":{"content":[{"type":"text","text":"hello cli"}]}}"#).unwrap();

        let chat_db_dir = home.join(".cursor").join("chats").join("bucket1").join("cli-ses");
        fs::create_dir_all(&chat_db_dir).unwrap();
        fs::write(chat_db_dir.join("store.db"), b"fake sqlite").unwrap();

        let whitelist = collect_cli_session_whitelist(home);
        assert!(whitelist.contains("cli-ses"));
        assert!(!whitelist.contains("ide-ses"));
    }
}
