use std::collections::HashMap;
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension};

use crate::app_db;
use crate::cli::CliKind;
use crate::commands::session::{build_codex_search_results, AggregatedSessionSearch};
use crate::commands::session_index::{
    build_projects_snapshot, RawSession, ScanAccumulator, ScanProjection, SnapshotProjection,
};
use crate::error::AppResult;
use crate::parser::batch::BatchEmitter;
use crate::parser::shared::{SearchDocument, SearchScanProgress};
use crate::session::{
    ChatMessage, ContentPart, PaginatedProjects, SearchResult, SessionListMetadata,
    SessionLoadResult, SubagentInfo, UsageRecord,
};
use crate::title_resolver::clean_fallback_user_text;

use super::super::descriptor::{CliDescriptor, FileRoot};
use super::super::features::{LaunchFeature, LaunchPlan, LaunchRequest};
use super::super::source::{under_data_root, CliSource, SessionLocator, Truncation};
use super::super::Discovered;

pub(crate) struct GooseSource;

static DESCRIPTOR: CliDescriptor = CliDescriptor {
    name: "Goose",
    tray_label: "Goose",
    command: "goose",
    proxy_port: crate::cli_registry::NO_PROXY_PORT,
    file_root: FileRoot {
        data_dir_segments: &[".local", "share", "goose"],
        sessions_subdir: Some("sessions"),
    },
};

static GOOSE_USAGE: GooseUsageStats = GooseUsageStats;
struct GooseUsageStats;
impl crate::cli_registry::features::UsageStatsFeature for GooseUsageStats {
    fn usage_records(&self, key: &str, project: &str) -> Vec<UsageRecord> {
        usage_records_for(key, project).unwrap_or_default()
    }
}

pub(crate) fn db_path() -> AppResult<PathBuf> {
    let configured = crate::cli::sessions_dir(CliKind::Goose)?.join("sessions.db");
    if configured.exists() {
        return Ok(configured);
    }
    #[cfg(target_os = "macos")]
    {
        if let Some(home) = dirs::home_dir() {
            let macos_db = home.join("Library/Application Support/Block/goose/sessions/sessions.db");
            if macos_db.exists() {
                return Ok(macos_db);
            }
        }
    }
    #[cfg(target_os = "windows")]
    {
        if let Some(data) = dirs::data_dir() {
            let win_db = data.join("Block").join("goose").join("data").join("sessions").join("sessions.db");
            if win_db.exists() {
                return Ok(win_db);
            }
        }
    }
    Ok(configured)
}

fn open_db() -> AppResult<Connection> {
    crate::parser::goose::open_goose_db(&db_path()?)
}

fn bare_key(key: &str) -> String {
    match SessionLocator::decode(CliKind::Goose, key) {
        SessionLocator::Virtual { key, .. } => key,
        SessionLocator::File { path, .. } => path.to_string_lossy().into_owned(),
    }
}

fn usage_records_for(key: &str, project: &str) -> Option<Vec<UsageRecord>> {
    let id = bare_key(key);
    let conn = open_db().ok()?;
    let row = conn
        .query_row(
            "SELECT created_at, updated_at, input_tokens, output_tokens FROM sessions WHERE id = ?1",
            params![id],
            |row| {
                let created_at = crate::parser::goose::row_timestamp_to_rfc3339(row, 0);
                let updated_at = crate::parser::goose::row_timestamp_to_rfc3339(row, 1);
                let input_tokens: i64 = row.get(2).unwrap_or(0);
                let output_tokens: i64 = row.get(3).unwrap_or(0);
                Ok((created_at, updated_at, input_tokens, output_tokens))
            },
        )
        .optional()
        .ok()??;

    let (created_at, updated_at, input, output) = row;
    let date = created_at
        .as_deref()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&chrono::Local).format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| "unknown".to_string());

    let duration_ms = match (created_at.as_deref(), updated_at.as_deref()) {
        (Some(c), Some(u)) => {
            let c_dt = chrono::DateTime::parse_from_rfc3339(c).ok();
            let u_dt = chrono::DateTime::parse_from_rfc3339(u).ok();
            match (c_dt, u_dt) {
                (Some(c), Some(u)) => {
                    let diff = u.signed_duration_since(c).num_milliseconds();
                    if diff > 0 && diff <= 30 * 24 * 3600 * 1000 {
                        Some(diff as u64)
                    } else {
                        None
                    }
                }
                _ => None,
            }
        }
        _ => None,
    };

    Some(vec![UsageRecord {
        date,
        model: String::new(),
        input_tokens: input.max(0) as u64,
        output_tokens: output.max(0) as u64,
        cache_creation_tokens: 0,
        cache_read_tokens: 0,
        duration_ms,
        project: project.to_string(),
    }])
}

impl CliSource for GooseSource {
    fn descriptor(&self) -> &'static CliDescriptor {
        &DESCRIPTOR
    }

    fn owns(&self, key: &str) -> bool {
        let prefix = SessionLocator::Virtual { cli_id: CliKind::Goose, key: String::new() }.to_key();
        if key.starts_with(&prefix) {
            return true;
        }
        under_data_root(key, CliKind::Goose)
    }

    fn exists(&self, loc: &SessionLocator) -> bool {
        let key = match loc {
            SessionLocator::Virtual { key, .. } => key,
            SessionLocator::File { path, .. } => {
                return path.exists();
            }
        };
        let conn = match open_db() {
            Ok(conn) => conn,
            Err(err) => {
                tracing::warn!("Goose 库打开失败，存活判定按「存在」处理: err={}", err);
                return true;
            }
        };
        match conn.query_row("SELECT 1 FROM sessions WHERE id = ?1", params![key], |_| Ok(())) {
            Ok(()) => true,
            Err(rusqlite::Error::QueryReturnedNoRows) => false,
            Err(err) => {
                tracing::warn!("Goose 存活查询失败，按「存在」处理: err={}", err);
                true
            }
        }
    }

    fn can_delete(&self) -> bool {
        false
    }

    fn discover(&self) -> AppResult<Discovered> {
        let conn = open_db()?;
        let mut stmt = conn.prepare(
            "SELECT id FROM sessions \
             WHERE (parent_session_id IS NULL OR parent_session_id = '') \
               AND (session_type IS NULL OR session_type != 'sub_agent')",
        )?;
        let locators = stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .map(|key| SessionLocator::Virtual { cli_id: CliKind::Goose, key })
            .collect();
        Ok(Discovered { locators, unverified_prefixes: Vec::new() })
    }

    fn snapshot(
        &self,
        custom_names: &HashMap<String, String>,
        page: usize,
        page_size: usize,
    ) -> AppResult<Option<PaginatedProjects>> {
        build_projects_snapshot(
            CliKind::Goose,
            custom_names,
            page,
            page_size,
            || {
                Ok(|record: &app_db::SessionListIndexRecord| {
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
        let conn = open_db()?;
        let mut stmt = conn.prepare(
            "SELECT id, name, description, working_dir, created_at, updated_at \
             FROM sessions \
             WHERE (parent_session_id IS NULL OR parent_session_id = '') \
               AND (session_type IS NULL OR session_type != 'sub_agent')",
        )?;

        let rows = stmt
            .query_map([], |row| {
                let id = row.get::<_, String>(0)?;
                let name = row.get::<_, Option<String>>(1)?;
                let description = row.get::<_, Option<String>>(2)?;
                let working_dir = row.get::<_, Option<String>>(3)?;
                let created_at = crate::parser::goose::row_timestamp_to_rfc3339(row, 4);
                let updated_at = crate::parser::goose::row_timestamp_to_rfc3339(row, 5);
                Ok((id, name, description, working_dir, created_at, updated_at))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        drop(stmt);
        drop(conn);

        let mut acc = ScanAccumulator::new(CliKind::Goose, custom_names, force)?;
        for (id, name, description, working_dir, created_at, updated_at) in rows {
            let key = SessionLocator::Virtual { cli_id: CliKind::Goose, key: id.clone() }.to_key();
            if acc.is_tombstoned(&id, &key) {
                continue;
            }
            let project_path = working_dir
                .filter(|path| !path.trim().is_empty())
                .unwrap_or_else(|| id.clone());
            let title = name.or(description);
            let metadata = crate::parser::goose::session_list_metadata(
                id,
                title.clone(),
                Some(project_path.clone()),
                created_at,
                updated_at,
            );
            acc.push_session(
                &key,
                metadata,
                ScanProjection {
                    project_path: Some(project_path.clone()),
                    encoded_dir: Some(project_path.clone()),
                    original_path: Some(project_path),
                    title,
                    history_display: None,
                    git_branch: String::new(),
                },
            );
        }
        acc.finish(&[])
    }

    fn parse(&self, key: &str) -> AppResult<Vec<ChatMessage>> {
        let id = bare_key(key);
        let conn = open_db()?;
        crate::parser::goose::load_session_messages(&conn, &id)
    }

    fn first_user_message(&self, key: &str) -> Option<String> {
        let id = bare_key(key);
        let conn = open_db().ok()?;
        let messages = crate::parser::goose::load_session_messages(&conn, &id).ok()?;
        for message in &messages {
            if message.role != "user" {
                continue;
            }
            for part in &message.content_parts {
                if let ContentPart::Text { text } = part {
                    if let Some(cleaned) = clean_fallback_user_text(text) {
                        let truncated: String = cleaned.chars().take(30).collect();
                        return Some(if cleaned.chars().count() > 30 {
                            format!("{}...", truncated)
                        } else {
                            truncated
                        });
                    }
                }
            }
        }
        None
    }

    fn session_id(&self, key: &str) -> Option<String> {
        let id = bare_key(key);
        let conn = open_db().ok()?;
        conn.query_row("SELECT id FROM sessions WHERE id = ?1", params![id], |row| {
            row.get::<_, String>(0)
        })
        .ok()
    }

    fn project_path_raw(&self, key: &str) -> Option<String> {
        let id = bare_key(key);
        let conn = open_db().ok()?;
        conn.query_row("SELECT working_dir FROM sessions WHERE id = ?1", params![id], |row| {
            row.get::<_, Option<String>>(0)
        })
        .ok()
        .flatten()
    }

    fn metadata_only(&self, key: &str) -> Option<SessionListMetadata> {
        let id = bare_key(key);
        let conn = open_db().ok()?;
        conn.query_row(
            "SELECT name, description, working_dir, created_at, updated_at FROM sessions WHERE id = ?1",
            params![id],
            |row| {
                let name: Option<String> = row.get(0)?;
                let description: Option<String> = row.get(1)?;
                let working_dir: Option<String> = row.get(2)?;
                let created_at = crate::parser::goose::row_timestamp_to_rfc3339(row, 3);
                let updated_at = crate::parser::goose::row_timestamp_to_rfc3339(row, 4);
                let title = name.or(description);
                Ok(crate::parser::goose::session_list_metadata(
                    id.clone(),
                    title,
                    working_dir.filter(|p| !p.trim().is_empty()),
                    created_at,
                    updated_at,
                ))
            },
        )
        .ok()
    }

    fn is_subagent(&self, key: &str) -> bool {
        let id = bare_key(key);
        let Ok(conn) = open_db() else { return false };
        conn.query_row(
            "SELECT parent_session_id, session_type FROM sessions WHERE id = ?1",
            params![id],
            |row| {
                let parent: Option<String> = row.get(0)?;
                let session_type: Option<String> = row.get(1)?;
                Ok(parent.filter(|p| !p.trim().is_empty()).is_some()
                    || session_type.map(|t| t == "sub_agent").unwrap_or(false))
            },
        )
        .unwrap_or(false)
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
        let mut emitter = BatchEmitter::new();
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
        Ok(SessionLoadResult { messages, offset: total, subagent_map: HashMap::new() })
    }

    fn parse_from_content(&self, _content: &str) -> Vec<ChatMessage> {
        Vec::new()
    }

    fn search_docs(
        &self,
        key: &str,
        on_progress: &mut dyn FnMut(SearchScanProgress),
    ) -> Option<Vec<SearchDocument>> {
        let id = bare_key(key);
        let conn = open_db().ok()?;
        let messages = crate::parser::goose::load_session_messages(&conn, &id).ok()?;
        let size = db_path()
            .ok()
            .and_then(|path| std::fs::metadata(path).ok())
            .map(|meta| meta.len())
            .unwrap_or(0);
        on_progress(SearchScanProgress { bytes_read: size, file_size: size });
        let docs = crate::parser::goose::search_docs_from_messages(&messages);
        if docs.is_empty() {
            None
        } else {
            Some(docs)
        }
    }

    fn search_docs_from_bytes(&self, _content: &[u8]) -> Option<Vec<SearchDocument>> {
        None
    }

    fn new_session(&self) -> Option<&'static dyn LaunchFeature> {
        Some(&GOOSE_LAUNCH)
    }

    fn resume_session(&self) -> Option<&'static dyn LaunchFeature> {
        Some(&GOOSE_LAUNCH)
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
        Some(&GOOSE_USAGE)
    }

    fn is_session_event_path(&self, path: &Path, _sessions_dir: &Path) -> bool {
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else { return false };
        name == "sessions.db" || name == "sessions.db-wal" || name.ends_with(".jsonl")
    }

    fn locate_executable(&self) -> Option<String> {
        crate::cli::find_on_path(self.descriptor().command)
    }

    fn tombstone_fallback_id(&self, _session_path: &Path) -> String {
        String::new()
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
            CliKind::Goose,
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

struct GooseLaunch;
static GOOSE_LAUNCH: GooseLaunch = GooseLaunch;

impl LaunchFeature for GooseLaunch {
    fn plan(&self, req: &LaunchRequest<'_>) -> AppResult<LaunchPlan> {
        let mut args = vec!["session".to_string()];
        if let Some(id) = req.session_id {
            args.push("--resume".to_string());
            args.push("--session-id".to_string());
            args.push(id.to_string());
        }
        Ok(LaunchPlan::CommandLine { args })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn virtual_goose_key_routes_to_goose() {
        let key = SessionLocator::Virtual {
            cli_id: CliKind::Goose,
            key: "ses_goose_123".to_string(),
        }
        .to_key();
        assert!(GooseSource.owns(&key));
    }

    #[test]
    fn goose_launch_plan_new_and_resume() {
        let new_plan = GOOSE_LAUNCH
            .plan(&LaunchRequest {
                session_id: None,
                skip_permissions: false,
                settings_file: None,
            })
            .unwrap();
        match new_plan {
            LaunchPlan::CommandLine { args } => {
                assert_eq!(args, vec!["session"]);
            }
            _ => panic!("expected CommandLine"),
        }

        let resume_plan = GOOSE_LAUNCH
            .plan(&LaunchRequest {
                session_id: Some("session-abc-123"),
                skip_permissions: false,
                settings_file: None,
            })
            .unwrap();
        match resume_plan {
            LaunchPlan::CommandLine { args } => {
                assert_eq!(args, vec!["session", "--resume", "--session-id", "session-abc-123"]);
            }
            _ => panic!("expected CommandLine"),
        }
    }
}
