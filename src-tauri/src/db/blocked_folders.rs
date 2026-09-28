use super::{conn, now_rfc3339};
use crate::error::AppResult;
use rusqlite::{params, Connection};
use std::collections::HashSet;

pub(crate) fn normalize_blocked_folders(paths: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut normalized = Vec::new();
    for path in paths {
        let trimmed = path.trim();
        if trimmed.is_empty() {
            continue;
        }
        if seen.insert(trimmed.to_string()) {
            normalized.push(trimmed.to_string());
        }
    }
    normalized
}

pub(crate) fn read_blocked_folders() -> AppResult<Vec<String>> {
    let conn = conn()?;
    read_blocked_folders_inner(&conn)
}

pub(crate) fn write_blocked_folders(paths: &[String]) -> AppResult<()> {
    let conn = conn()?;
    write_blocked_folders_inner(&conn, paths)
}

pub(super) fn read_blocked_folders_inner(conn: &Connection) -> AppResult<Vec<String>> {
    let mut stmt = conn.prepare("SELECT path FROM blocked_folders ORDER BY rowid ASC")?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
    let mut paths = Vec::new();
    for row in rows {
        paths.push(row?);
    }
    Ok(paths)
}

pub(super) fn write_blocked_folders_inner(conn: &Connection, paths: &[String]) -> AppResult<()> {
    let normalized = normalize_blocked_folders(paths);
    let tx = conn.unchecked_transaction()?;
    tx.execute("DELETE FROM blocked_folders", [])?;

    let now = now_rfc3339();
    for path in normalized {
        tx.execute(
            "INSERT INTO blocked_folders (path, created_at) VALUES (?1, ?2)",
            params![path, now],
        )?;
    }

    tx.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS blocked_folders (
                path TEXT PRIMARY KEY,
                created_at TEXT NOT NULL
            );
            "#,
        )
        .unwrap();
        conn
    }

    #[test]
    fn write_then_read_preserves_order_and_deduplicates() {
        let conn = test_conn();
        let input = vec![
            "/path/to/b".to_string(),
            "/path/to/a".to_string(),
            "  /path/to/b  ".to_string(),
            "".to_string(),
            "   ".to_string(),
            "/path/to/c".to_string(),
        ];
        write_blocked_folders_inner(&conn, &input).unwrap();

        let result = read_blocked_folders_inner(&conn).unwrap();
        assert_eq!(
            result,
            vec![
                "/path/to/b".to_string(),
                "/path/to/a".to_string(),
                "/path/to/c".to_string(),
            ]
        );
    }

    #[test]
    fn write_overwrites_previous_blocked_folders() {
        let conn = test_conn();
        write_blocked_folders_inner(&conn, &["/first".to_string()]).unwrap();
        assert_eq!(read_blocked_folders_inner(&conn).unwrap(), vec!["/first"]);

        write_blocked_folders_inner(&conn, &["/second".to_string(), "/third".to_string()]).unwrap();
        assert_eq!(
            read_blocked_folders_inner(&conn).unwrap(),
            vec!["/second", "/third"]
        );
    }
}
