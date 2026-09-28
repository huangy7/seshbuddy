//! transcript_cache.db：助手专用的会话文本投影（独立 SQLite，可整库删除重建）。

use crate::extract::ExtractedMessage;
use rusqlite::{params, Connection};
use std::collections::HashMap;
use std::path::Path;

// v2：提取器支持 codex/gemini 格式——v1 误判的 empty 终态记录需整库重建
pub const SCHEMA_VERSION: i64 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheStatus {
    Ok,
    Empty,
    Gone,
}

impl CacheStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            CacheStatus::Ok => "ok",
            CacheStatus::Empty => "empty",
            CacheStatus::Gone => "gone",
        }
    }
    fn from_str(s: &str) -> Self {
        match s {
            "empty" => CacheStatus::Empty,
            "gone" => CacheStatus::Gone,
            _ => CacheStatus::Ok,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SessionCacheRecord {
    pub session_id: String,
    pub cli: String,
    pub mtime_ms: i64,
    pub size: i64,
    pub raw_lines: i64,
    pub last_seq: i64,
    pub status: CacheStatus,
    pub message_count: i64,
    pub extracted_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ExtractPlan {
    Skip,
    Full,
    Append { skip_raw_lines: u64, base_seq: u64 },
}

/// 签名比对决定提取策略。仅在源文件存在时调用。
pub fn decide_plan(record: Option<&SessionCacheRecord>, mtime_ms: i64, size: i64) -> ExtractPlan {
    let Some(r) = record else { return ExtractPlan::Full };
    if r.status == CacheStatus::Gone {
        return ExtractPlan::Full; // 文件复活：全量重提
    }
    if r.mtime_ms == mtime_ms && r.size == size {
        return ExtractPlan::Skip;
    }
    if size > r.size {
        // jsonl append-only 前提：纯增长走增量
        return ExtractPlan::Append {
            skip_raw_lines: r.raw_lines as u64,
            base_seq: r.last_seq as u64,
        };
    }
    ExtractPlan::Full
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// 打开（必要时创建）投影库：WAL + busy_timeout，schema 版本不匹配即整库重建。
pub fn open(db_path: &Path) -> rusqlite::Result<Connection> {
    if let Some(dir) = db_path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let conn = Connection::open(db_path)?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "busy_timeout", 3000)?;
    migrate(&conn)?;
    Ok(conn)
}

fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    let version: Option<i64> = conn
        .query_row(
            "SELECT schema_version FROM cache_meta",
            [],
            |r| r.get(0),
        )
        .ok();
    if version == Some(SCHEMA_VERSION) {
        return Ok(());
    }
    // 不存在或版本不匹配：整库重建（投影可随时从源数据重建）
    conn.execute_batch(
        "DROP TABLE IF EXISTS session_message;
         DROP TABLE IF EXISTS session_cache;
         DROP TABLE IF EXISTS cache_meta;
         CREATE TABLE cache_meta (schema_version INTEGER NOT NULL);
         CREATE TABLE session_cache (
           session_id TEXT PRIMARY KEY,
           cli TEXT NOT NULL,
           mtime_ms INTEGER NOT NULL,
           size INTEGER NOT NULL,
           raw_lines INTEGER NOT NULL DEFAULT 0,
           last_seq INTEGER NOT NULL DEFAULT 0,
           status TEXT NOT NULL,
           message_count INTEGER NOT NULL DEFAULT 0,
           extracted_at INTEGER NOT NULL
         );
         CREATE TABLE session_message (
           session_id TEXT NOT NULL,
           seq INTEGER NOT NULL,
           role TEXT NOT NULL,
           text TEXT NOT NULL,
           PRIMARY KEY (session_id, seq)
         );",
    )?;
    conn.execute(
        "INSERT INTO cache_meta (schema_version) VALUES (?1)",
        params![SCHEMA_VERSION],
    )?;
    Ok(())
}

fn row_to_record(row: &rusqlite::Row) -> rusqlite::Result<SessionCacheRecord> {
    let status: String = row.get(6)?;
    Ok(SessionCacheRecord {
        session_id: row.get(0)?,
        cli: row.get(1)?,
        mtime_ms: row.get(2)?,
        size: row.get(3)?,
        raw_lines: row.get(4)?,
        last_seq: row.get(5)?,
        status: CacheStatus::from_str(&status),
        message_count: row.get(7)?,
        extracted_at: row.get(8)?,
    })
}

const RECORD_COLS: &str =
    "session_id, cli, mtime_ms, size, raw_lines, last_seq, status, message_count, extracted_at";

pub fn get_record(conn: &Connection, session_id: &str) -> rusqlite::Result<Option<SessionCacheRecord>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {} FROM session_cache WHERE session_id = ?1",
        RECORD_COLS
    ))?;
    let mut rows = stmt.query_map(params![session_id], row_to_record)?;
    Ok(rows.next().transpose()?)
}

pub fn all_records(conn: &Connection) -> rusqlite::Result<HashMap<String, SessionCacheRecord>> {
    let mut stmt = conn.prepare(&format!("SELECT {} FROM session_cache", RECORD_COLS))?;
    let rows = stmt.query_map([], row_to_record)?;
    let mut map = HashMap::new();
    for r in rows {
        let r = r?;
        map.insert(r.session_id.clone(), r);
    }
    Ok(map)
}

pub fn load_messages(conn: &Connection, session_id: &str) -> rusqlite::Result<Vec<ExtractedMessage>> {
    let mut stmt = conn.prepare(
        "SELECT seq, role, text FROM session_message WHERE session_id = ?1 ORDER BY seq",
    )?;
    let rows = stmt.query_map(params![session_id], |row| {
        Ok(ExtractedMessage {
            seq: row.get::<_, i64>(0)? as u64,
            role: row.get(1)?,
            text: row.get(2)?,
        })
    })?;
    rows.collect()
}

fn upsert_record(conn: &Connection, rec: &SessionCacheRecord) -> rusqlite::Result<()> {
    conn.execute(
        &format!(
            "INSERT OR REPLACE INTO session_cache ({}) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            RECORD_COLS
        ),
        params![
            rec.session_id, rec.cli, rec.mtime_ms, rec.size, rec.raw_lines,
            rec.last_seq, rec.status.as_str(), rec.message_count,
            if rec.extracted_at == 0 { now_ms() } else { rec.extracted_at },
        ],
    )?;
    Ok(())
}

/// 全量重写一个会话：删旧消息 + 写新消息 + upsert 记录（单事务）。
pub fn replace_session(
    conn: &Connection,
    rec: &SessionCacheRecord,
    msgs: &[ExtractedMessage],
) -> rusqlite::Result<()> {
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "DELETE FROM session_message WHERE session_id = ?1",
        params![rec.session_id],
    )?;
    {
        let mut stmt = tx.prepare(
            "INSERT INTO session_message (session_id, seq, role, text) VALUES (?1,?2,?3,?4)",
        )?;
        for m in msgs {
            stmt.execute(params![rec.session_id, m.seq as i64, m.role, m.text])?;
        }
    }
    upsert_record(&tx, rec)?;
    tx.commit()
}

/// 追加式写入：只插新消息 + upsert 记录（单事务）。
pub fn append_session(
    conn: &Connection,
    rec: &SessionCacheRecord,
    new_msgs: &[ExtractedMessage],
) -> rusqlite::Result<()> {
    let tx = conn.unchecked_transaction()?;
    {
        let mut stmt = tx.prepare(
            "INSERT OR REPLACE INTO session_message (session_id, seq, role, text) VALUES (?1,?2,?3,?4)",
        )?;
        for m in new_msgs {
            stmt.execute(params![rec.session_id, m.seq as i64, m.role, m.text])?;
        }
    }
    upsert_record(&tx, rec)?;
    tx.commit()
}

/// 写终态记录（gone/empty），无消息写入。
pub fn mark_status(
    conn: &Connection,
    session_id: &str,
    cli: &str,
    status: CacheStatus,
    mtime_ms: i64,
    size: i64,
) -> rusqlite::Result<()> {
    let rec = SessionCacheRecord {
        session_id: session_id.to_string(),
        cli: cli.to_string(),
        mtime_ms,
        size,
        raw_lines: 0,
        last_seq: 0,
        status,
        message_count: 0,
        extracted_at: now_ms(),
    };
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "DELETE FROM session_message WHERE session_id = ?1",
        params![session_id],
    )?;
    upsert_record(&tx, &rec)?;
    tx.commit()
}

pub fn count_cached(conn: &Connection) -> rusqlite::Result<usize> {
    conn.query_row(
        "SELECT COUNT(*) FROM session_cache WHERE status = 'ok'",
        [],
        |r| r.get::<_, i64>(0).map(|n| n as usize),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extract::ExtractedMessage;

    fn msg(seq: u64, text: &str) -> ExtractedMessage {
        ExtractedMessage { seq, role: "user".into(), text: text.into() }
    }

    fn rec(session_id: &str) -> SessionCacheRecord {
        SessionCacheRecord {
            session_id: session_id.into(), cli: "claude".into(),
            mtime_ms: 100, size: 50, raw_lines: 10, last_seq: 3,
            status: CacheStatus::Ok, message_count: 2, extracted_at: 0,
        }
    }

    #[test]
    fn open_creates_schema_and_reopens() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("cache.db");
        let conn = open(&db).unwrap();
        replace_session(&conn, &rec("s1"), &[msg(1, "你好"), msg(3, "在")]).unwrap();
        drop(conn);
        let conn = open(&db).unwrap();
        let msgs = load_messages(&conn, "s1").unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[1].seq, 3);
        let r = get_record(&conn, "s1").unwrap().unwrap();
        assert_eq!(r.status, CacheStatus::Ok);
        assert_eq!(r.raw_lines, 10);
    }

    #[test]
    fn decide_plan_matrix() {
        // 无记录 → Full
        assert!(matches!(decide_plan(None, 100, 50), ExtractPlan::Full));
        let r = rec("s1");
        // 签名一致 → Skip
        assert!(matches!(decide_plan(Some(&r), 100, 50), ExtractPlan::Skip));
        // 纯增长 → Append（游标来自记录）
        match decide_plan(Some(&r), 200, 80) {
            ExtractPlan::Append { skip_raw_lines, base_seq } => {
                assert_eq!(skip_raw_lines, 10);
                assert_eq!(base_seq, 3);
            }
            other => panic!("expected Append, got {:?}", other),
        }
        // 文件缩小/改写 → Full
        assert!(matches!(decide_plan(Some(&r), 200, 40), ExtractPlan::Full));
        // gone 记录但文件又存在 → Full（复活场景）
        let mut g = rec("s1");
        g.status = CacheStatus::Gone;
        assert!(matches!(decide_plan(Some(&g), 200, 80), ExtractPlan::Full));
    }

    #[test]
    fn append_session_only_inserts_new_messages() {
        let dir = tempfile::tempdir().unwrap();
        let conn = open(&dir.path().join("c.db")).unwrap();
        replace_session(&conn, &rec("s1"), &[msg(1, "a"), msg(3, "b")]).unwrap();
        let mut r2 = rec("s1");
        r2.mtime_ms = 200; r2.size = 80; r2.raw_lines = 12; r2.last_seq = 4; r2.message_count = 3;
        append_session(&conn, &r2, &[msg(4, "c")]).unwrap();
        let msgs = load_messages(&conn, "s1").unwrap();
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[2].text, "c");
        assert_eq!(get_record(&conn, "s1").unwrap().unwrap().last_seq, 4);
    }

    #[test]
    fn mark_status_writes_terminal_without_messages() {
        let dir = tempfile::tempdir().unwrap();
        let conn = open(&dir.path().join("c.db")).unwrap();
        mark_status(&conn, "s9", "claude", CacheStatus::Gone, 0, 0).unwrap();
        let r = get_record(&conn, "s9").unwrap().unwrap();
        assert_eq!(r.status, CacheStatus::Gone);
        assert!(load_messages(&conn, "s9").unwrap().is_empty());
        assert_eq!(count_cached(&conn).unwrap(), 0);
    }

    #[test]
    fn schema_version_mismatch_rebuilds() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("c.db");
        let conn = open(&db).unwrap();
        replace_session(&conn, &rec("s1"), &[msg(1, "a")]).unwrap();
        conn.execute("UPDATE cache_meta SET schema_version = 999", []).unwrap();
        drop(conn);
        let conn = open(&db).unwrap(); // 版本不匹配 → 整库重建
        assert!(get_record(&conn, "s1").unwrap().is_none());
    }
}
