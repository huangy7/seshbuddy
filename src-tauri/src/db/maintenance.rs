use super::{
    backfill_archive_sidecars, conn, purge_expired_archives, purge_tombstones,
    restore_missing_archived_index_rows, snapshot_aging_sessions, TOMBSTONE_RETENTION_DAYS,
};
use crate::cli::CliKind;
use crate::error::{AppError, AppResult};
use rusqlite::params;
use std::path::Path;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

/// Remove session_list_index and session_search_docs rows whose session file no longer exists on disk.
/// Returns the number of orphan session paths removed.
pub(crate) fn purge_orphan_session_index() -> AppResult<usize> {
    let pairs: Vec<(String, String, Option<String>)> = {
        let conn = conn()?;
        let mut stmt = conn.prepare("SELECT cli_id, session_path, archived_at FROM session_list_index")?;
        let rows: Vec<(String, String, Option<String>)> = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
            .filter_map(|r| r.ok())
            .collect();
        rows
    };

    let orphans: Vec<(String, String)> = pairs
        .into_iter()
        .filter(|(_, path, archived_at)| !Path::new(path).exists() && archived_at.is_none())
        .map(|(cli_id, path, _)| (cli_id, path))
        .collect();

    if orphans.is_empty() {
        return Ok(0);
    }

    let mut conn = conn()?;
    let tx = conn.transaction()?;

    for (cli_id, session_path) in &orphans {
        tx.execute(
            "DELETE FROM session_list_index WHERE cli_id = ?1 AND session_path = ?2",
            params![cli_id, session_path],
        )?;
        tx.execute(
            "DELETE FROM session_search_index_state WHERE cli_id = ?1 AND session_path = ?2",
            params![cli_id, session_path],
        )?;
    }

    // Also delete from Tantivy index. Best-effort: the SQLite cleanup must
    // still commit, but log so stale search hits are diagnosable.
    let paths: Vec<String> = orphans.iter().map(|(_, p)| p.clone()).collect();
    if let Err(err) = super::tantivy_search::delete_session_docs(&paths) {
        tracing::warn!(
            "清理失效会话的 Tantivy 搜索文档失败: paths={:?}, err={}",
            paths,
            err.diagnostic()
        );
    }

    tx.commit()?;
    Ok(orphans.len())
}

static LAST_SUBAGENT_PURGE: LazyLock<Mutex<Option<Instant>>> = LazyLock::new(|| Mutex::new(None));

/// 清理数据库中误存的子代理记录（包括 session_list_index、archived_session_content 与搜索索引）。
pub(crate) fn purge_subagents_from_db() -> AppResult<usize> {
    {
        let mut last = LAST_SUBAGENT_PURGE
            .lock()
            .map_err(|e| AppError::coded("db.subagent_purge_lock_poisoned").with("detail", e.to_string()))?;
        if let Some(prev) = *last {
            if prev.elapsed() < Duration::from_secs(300) {
                return Ok(0);
            }
        }
        *last = Some(Instant::now());
    }

    let subagents: Vec<(String, String)> = {
        let conn = conn()?;
        let mut found = Vec::new();

        if let Ok(mut stmt) = conn.prepare("SELECT cli_id, session_path FROM session_list_index") {
            if let Ok(rows) = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?))) {
                for row in rows.flatten() {
                    let (cli_id, path): (String, String) = row;
                    if crate::parser::is_subagent_session(&path) {
                        found.push((cli_id, path));
                    }
                }
            }
        }

        if let Ok(mut stmt_arc) = conn.prepare("SELECT cli_id, session_path FROM archived_session_content") {
            if let Ok(rows_arc) = stmt_arc.query_map([], |row| Ok((row.get(0)?, row.get(1)?))) {
                for row in rows_arc.flatten() {
                    let (cli_id, path): (String, String) = row;
                    if crate::parser::is_subagent_session(&path) {
                        found.push((cli_id, path));
                    }
                }
            }
        }

        found.sort();
        found.dedup();
        found
    };

    if subagents.is_empty() {
        return Ok(0);
    }

    let mut conn = conn()?;
    let tx = conn.transaction()?;

    for (cli_id, session_path) in &subagents {
        let _ = tx.execute(
            "DELETE FROM session_list_index WHERE cli_id = ?1 AND session_path = ?2",
            params![cli_id, session_path],
        );
        let _ = tx.execute(
            "DELETE FROM archived_session_content WHERE cli_id = ?1 AND session_path = ?2",
            params![cli_id, session_path],
        );
        let _ = tx.execute(
            "DELETE FROM session_search_index_state WHERE cli_id = ?1 AND session_path = ?2",
            params![cli_id, session_path],
        );
    }

    let paths: Vec<String> = subagents.iter().map(|(_, p)| p.clone()).collect();
    if let Err(err) = super::tantivy_search::delete_session_docs(&paths) {
        tracing::warn!(
            "清理子代理会话的 Tantivy 搜索文档失败: paths={:?}, err={}",
            paths,
            err.diagnostic()
        );
    }

    tx.commit()?;
    Ok(subagents.len())
}

/// Run startup maintenance tasks in the background:
/// - Purge orphan session index rows (session file deleted from disk)
pub(crate) fn run_startup_maintenance() {
    match purge_subagents_from_db() {
        Ok(n) if n > 0 => tracing::info!("Startup maintenance: removed {} subagent database records", n),
        Ok(_) => {}
        Err(e) => tracing::warn!("Startup maintenance: subagent purge failed: {}", e.diagnostic()),
    }
    match purge_orphan_session_index() {
        Ok(n) if n > 0 => tracing::info!("Startup maintenance: removed {} orphan session index rows", n),
        Ok(_) => {}
        Err(e) => tracing::warn!("Startup maintenance: orphan purge failed: {}", e.diagnostic()),
    }
    // 墓碑维护：清除「文件已放回原位」与「已过期」的墓碑。删除是移到废纸篓、可逆，
    // 但墓碑一旦写入就没有其它清除路径 —— 不跑这一步，用户恢复文件后会话将永久不可见
    match purge_tombstones(TOMBSTONE_RETENTION_DAYS) {
        Ok((cleared, expired)) if cleared > 0 || expired > 0 => tracing::info!(
            "Startup maintenance: cleared {} restored + {} expired session tombstones",
            cleared,
            expired
        ),
        Ok(_) => {}
        Err(e) => tracing::warn!("Startup maintenance: tombstone purge failed: {}", e.diagnostic()),
    }
    // 清理纯搜索孤儿（物理索引里有、源文件与归档都没有的路径），
    // 这类条目在会话列表索引里不存在，purge_orphan_session_index 覆盖不到。
    match super::session_search::purge_search_orphans(None) {
        Ok(n) if n > 0 => tracing::info!("Startup maintenance: removed {} pure search orphans", n),
        Ok(_) => {}
        Err(e) => tracing::warn!("Startup maintenance: pure search orphan purge failed: {}", e.diagnostic()),
    }
    match snapshot_aging_sessions() {
        Ok(n) if n > 0 => tracing::info!("Startup maintenance: archived {} aging sessions", n),
        Ok(_) => {}
        Err(e) => tracing::warn!("Startup maintenance: session archive failed: {}", e.diagnostic()),
    }
    match purge_expired_archives() {
        Ok(n) if n > 0 => tracing::info!("Startup maintenance: purged {} expired archives", n),
        Ok(_) => {}
        Err(e) => tracing::warn!("Startup maintenance: archive purge failed: {}", e.diagnostic()),
    }
    // 救回历史 bug 误删索引行的归档会话（有快照但缺 session_list_index 行）
    for kind in [CliKind::Claude, CliKind::Codex, CliKind::Gemini] {
        match restore_missing_archived_index_rows(kind) {
            Ok(n) if n > 0 => tracing::info!(
                "Startup maintenance: restored {} archived session index rows, cli={}",
                n,
                kind.id()
            ),
            Ok(_) => {}
            Err(e) => tracing::warn!(
                "Startup maintenance: archived index row restore failed, cli={}: {}",
                kind.id(),
                e.diagnostic()
            ),
        }
    }

    // 为存量归档补写 sidecar 灾备副本（幂等，仅写缺失；DB 丢失后 rebuild 靠它恢复 pin/归档时间）
    match backfill_archive_sidecars() {
        Ok(n) if n > 0 => tracing::info!("Startup maintenance: backfilled {} archive sidecars", n),
        Ok(_) => {}
        Err(e) => tracing::warn!("Startup maintenance: sidecar backfill failed: {}", e.diagnostic()),
    }
}
