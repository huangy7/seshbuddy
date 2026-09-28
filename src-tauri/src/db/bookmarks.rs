use super::{conn, now_rfc3339};
use crate::cli::CliKind;
use crate::error::AppResult;
use rusqlite::params;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BookmarkRecord {
    #[serde(default = "default_bookmark_cli_id")]
    pub cli_id: String,
    pub session_id: String,
    pub message_index: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default = "default_bookmark_created_at")]
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_timestamp: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_display_name: Option<String>,
}

fn default_bookmark_cli_id() -> String {
    CliKind::Claude.id().to_string()
}

fn default_bookmark_created_at() -> String {
    now_rfc3339()
}

pub(crate) fn normalize_bookmarks(mut bookmarks: Vec<BookmarkRecord>) -> Vec<BookmarkRecord> {
    for bookmark in &mut bookmarks {
        if bookmark.cli_id.trim().is_empty() {
            bookmark.cli_id = default_bookmark_cli_id();
        }
        if bookmark.created_at.trim().is_empty() {
            bookmark.created_at = default_bookmark_created_at();
        }
    }
    bookmarks
}

pub(crate) fn read_all_bookmarks() -> AppResult<Vec<BookmarkRecord>> {
    let conn = conn()?;
    let mut stmt = conn.prepare(
        "SELECT cli_id, session_id, message_index, note, created_at, message_role, message_text, message_timestamp, session_display_name FROM bookmarks ORDER BY created_at ASC, message_index ASC",
    )?;

    let rows = stmt.query_map([], |row| {
        Ok(BookmarkRecord {
            cli_id: row.get(0)?,
            session_id: row.get(1)?,
            message_index: row.get::<_, i64>(2)? as usize,
            note: row.get(3)?,
            created_at: row.get(4)?,
            message_role: row.get(5)?,
            message_text: row.get(6)?,
            message_timestamp: row.get(7)?,
            session_display_name: row.get(8)?,
        })
    })?;

    let mut bookmarks = Vec::new();
    for row in rows {
        bookmarks.push(row?);
    }
    Ok(normalize_bookmarks(bookmarks))
}

pub(crate) fn write_all_bookmarks(bookmarks: &[BookmarkRecord]) -> AppResult<()> {
    let normalized = normalize_bookmarks(bookmarks.to_vec());
    let mut conn = conn()?;
    let tx = conn.transaction()?;
    tx.execute("DELETE FROM bookmarks", [])?;

    for bookmark in &normalized {
        tx.execute(
            "INSERT INTO bookmarks (cli_id, session_id, message_index, note, created_at, message_role, message_text, message_timestamp, session_display_name) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                bookmark.cli_id,
                bookmark.session_id,
                bookmark.message_index as i64,
                bookmark.note,
                bookmark.created_at,
                bookmark.message_role,
                bookmark.message_text,
                bookmark.message_timestamp,
                bookmark.session_display_name,
            ],
        )?;
    }

    tx.commit()?;
    Ok(())
}
