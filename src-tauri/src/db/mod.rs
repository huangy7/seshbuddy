use crate::error::{AppError, AppResult};
use crate::paths::app_data_dir;
use chrono::Utc;
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::Connection;
use std::fs;
use std::path::PathBuf;
use std::sync::LazyLock;
use std::time::Duration;

mod profiles;
mod favorites;
mod bookmarks;
mod session_index;
pub(crate) mod session_archive;
mod archive_sidecar;
mod session_search;
mod maintenance;
mod settings;
mod blocked_folders;
mod session_tombstones;
pub(crate) mod tantivy_search;

pub(crate) use blocked_folders::*;
pub(crate) use bookmarks::*;
pub(crate) use favorites::*;
pub(crate) use profiles::*;
pub(crate) use session_archive::*;
pub(crate) use session_index::*;
pub(crate) use session_search::*;
pub(crate) use maintenance::*;
pub(crate) use settings::*;
pub(crate) use session_tombstones::*;

pub(crate) fn now_rfc3339() -> String {
    Utc::now().to_rfc3339()
}

static GLOBAL_POOL: LazyLock<Pool<SqliteConnectionManager>> = LazyLock::new(|| {
    create_pool().expect("Failed to initialize app database pool")
});

fn create_pool() -> AppResult<Pool<SqliteConnectionManager>> {
    let path = app_db_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| {
            AppError::coded("db.data_dir_create_failed").with("detail", e.to_string())
        })?;
    }

    let manager = SqliteConnectionManager::file(&path)
        .with_init(|conn| {
            conn.execute_batch(
                "PRAGMA journal_mode = WAL;
                 PRAGMA busy_timeout = 5000;
                 PRAGMA synchronous = NORMAL;
                 PRAGMA foreign_keys = ON;"
            )?;
            Ok(())
        });

    let pool = Pool::builder()
        .max_size(16)
        .min_idle(Some(2))
        .connection_timeout(Duration::from_secs(10))
        .build(manager)?;

    // Run schema init on a dedicated connection
    {
        let conn = pool.get()?;
        init_schema(&conn)?;
    }

    Ok(pool)
}

/// Get a connection from the pool. This never deadlocks because each call
/// gets its own connection — no nested locking.
pub(crate) fn conn() -> AppResult<r2d2::PooledConnection<SqliteConnectionManager>> {
    Ok(GLOBAL_POOL.get()?)
}

pub(crate) fn archives_dir() -> AppResult<PathBuf> {
    Ok(app_data_dir()?.join("archives"))
}

pub(crate) fn search_index_dir() -> AppResult<PathBuf> {
    Ok(app_data_dir()?.join("search_index"))
}

pub(crate) fn data_dir() -> AppResult<PathBuf> {
    Ok(app_data_dir()?.join("data"))
}

pub(crate) fn app_db_path() -> AppResult<PathBuf> {
    Ok(data_dir()?.join("app.db"))
}



fn init_schema(conn: &Connection) -> AppResult<()> {
    create_base_schema(conn)
}

/// 当前完整 schema。本仓库尚无发布版本，不存在需要升级的旧库，
/// 因此初始化即建全表，不保留历史迁移链。
pub(crate) fn create_base_schema(conn: &Connection) -> AppResult<()> {
    conn.execute_batch(
        r#"
        PRAGMA foreign_keys = ON;

        CREATE TABLE IF NOT EXISTS profiles (
            cli_id TEXT NOT NULL,
            scope TEXT NOT NULL DEFAULT 'global',
            name TEXT NOT NULL,
            normalized_name TEXT NOT NULL,
            content_json TEXT NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            PRIMARY KEY (cli_id, scope, normalized_name)
        );

        CREATE TABLE IF NOT EXISTS active_profiles (
            cli_id TEXT NOT NULL,
            profile_name TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            scope TEXT NOT NULL DEFAULT 'global',
            PRIMARY KEY (cli_id, scope)
        );

        CREATE TABLE IF NOT EXISTS profile_tabs (
            id TEXT PRIMARY KEY,
            cli_id TEXT NOT NULL,
            name TEXT NOT NULL,
            sort_order INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS profile_tab_dirs (
            tab_id TEXT NOT NULL,
            dir_path TEXT NOT NULL,
            PRIMARY KEY (tab_id, dir_path),
            FOREIGN KEY (tab_id) REFERENCES profile_tabs(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS favorites (
            cli_id TEXT NOT NULL DEFAULT 'claude',
            path TEXT NOT NULL,
            position INTEGER NOT NULL,
            created_at TEXT NOT NULL,
            PRIMARY KEY (cli_id, path)
        );

        CREATE TABLE IF NOT EXISTS bookmarks (
            cli_id TEXT NOT NULL,
            session_id TEXT NOT NULL,
            message_index INTEGER NOT NULL,
            note TEXT,
            created_at TEXT NOT NULL,
            message_role TEXT,
            message_text TEXT,
            message_timestamp TEXT,
            session_display_name TEXT,
            PRIMARY KEY (cli_id, session_id, message_index)
        );

        CREATE TABLE IF NOT EXISTS session_names (
            session_path TEXT PRIMARY KEY,
            display_name TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS session_list_index (
            cli_id TEXT NOT NULL,
            session_path TEXT NOT NULL,
            session_id TEXT NOT NULL,
            project_path TEXT,
            title TEXT,
            first_user_message TEXT,
            first_timestamp TEXT,
            last_timestamp TEXT,
            git_branch TEXT NOT NULL,
            file_size INTEGER NOT NULL,
            modified_ms INTEGER NOT NULL,
            indexed_at TEXT NOT NULL,
            archived_at TEXT,
            PRIMARY KEY (cli_id, session_path)
        );

        CREATE TABLE IF NOT EXISTS app_settings (
            key TEXT PRIMARY KEY,
            value_json TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS blocked_folders (
            path TEXT PRIMARY KEY,
            created_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS session_tombstones (
            cli_id      TEXT NOT NULL,
            session_id  TEXT NOT NULL,
            file_path   TEXT NOT NULL,
            deleted_at  TEXT NOT NULL,
            PRIMARY KEY (cli_id, session_id, file_path)
        );

        CREATE TABLE IF NOT EXISTS archived_session_content (
            cli_id TEXT NOT NULL,
            session_path TEXT NOT NULL,
            jsonl_content BLOB NOT NULL,
            snapshot_modified_ms INTEGER NOT NULL,
            archived_at TEXT NOT NULL,
            retention_policy TEXT NOT NULL DEFAULT 'default',
            pinned_at TEXT,
            PRIMARY KEY (cli_id, session_path)
        );

        CREATE TABLE IF NOT EXISTS session_search_docs (
            cli_id TEXT NOT NULL,
            session_path TEXT NOT NULL,
            modified_ms INTEGER NOT NULL,
            message_index INTEGER NOT NULL,
            search_text TEXT NOT NULL,
            indexed_at TEXT NOT NULL,
            PRIMARY KEY (cli_id, session_path, message_index)
        );

        CREATE VIRTUAL TABLE IF NOT EXISTS session_search_docs_fts USING fts5(
            search_text,
            cli_id UNINDEXED,
            session_path UNINDEXED,
            message_index UNINDEXED,
            tokenize='trigram'
        );

        CREATE TABLE IF NOT EXISTS session_search_index_state (
            cli_id TEXT NOT NULL,
            session_path TEXT NOT NULL,
            modified_ms INTEGER NOT NULL,
            doc_count INTEGER NOT NULL,
            indexed_at TEXT NOT NULL,
            PRIMARY KEY (cli_id, session_path)
        );

        CREATE TABLE IF NOT EXISTS assistant_conversations (
            id TEXT PRIMARY KEY,
            claude_session_id TEXT NOT NULL,
            profile TEXT NOT NULL DEFAULT '',
            title TEXT NOT NULL DEFAULT '',
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS assistant_quick_phrases (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            content TEXT NOT NULL,
            sort INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_profiles_cli_scope_name ON profiles (cli_id, scope, name);
        CREATE INDEX IF NOT EXISTS idx_favorites_position ON favorites (position);
        CREATE INDEX IF NOT EXISTS idx_bookmarks_cli_created ON bookmarks (cli_id, created_at);
        CREATE INDEX IF NOT EXISTS idx_session_list_index_cli_last_timestamp
            ON session_list_index (cli_id, last_timestamp DESC, first_timestamp DESC, session_path ASC);
        CREATE INDEX IF NOT EXISTS idx_session_search_docs_cli_modified
            ON session_search_docs (cli_id, modified_ms DESC, session_path ASC);
        CREATE INDEX IF NOT EXISTS idx_session_search_docs_cli_session
            ON session_search_docs (cli_id, session_path, message_index ASC);
        CREATE INDEX IF NOT EXISTS idx_session_tombstones_path ON session_tombstones(file_path);
        "#,
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 初始化即建全表：schema 由 create_base_schema 单点定义，不再有迁移链
    #[test]
    fn base_schema_creates_all_tables() {
        let conn = Connection::open_in_memory().unwrap();
        create_base_schema(&conn).unwrap();

        let expected = [
            "active_profiles",
            "app_settings",
            "archived_session_content",
            "assistant_conversations",
            "assistant_quick_phrases",
            "blocked_folders",
            "bookmarks",
            "favorites",
            "profile_tab_dirs",
            "profile_tabs",
            "profiles",
            "session_list_index",
            "session_names",
            "session_search_docs",
            "session_search_index_state",
            "session_tombstones",
        ];
        for table in expected {
            let exists: bool = conn
                .query_row(
                    "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type = 'table' AND name = ?1",
                    [table],
                    |row| row.get(0),
                )
                .unwrap();
            assert!(exists, "{} 表应被创建", table);
        }

        // 幂等：重复执行不报错
        create_base_schema(&conn).unwrap();
    }
}

