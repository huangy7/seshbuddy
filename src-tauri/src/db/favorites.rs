use super::{conn, now_rfc3339};
use crate::error::AppResult;
use rusqlite::{params, Connection};
use std::collections::HashSet;

/// 复合身份条目；字段名与前端 SessionIdentity 的映射由 TS 侧处理
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct FavoriteEntry {
    pub cli_id: String,
    pub path: String,
}

pub(crate) fn normalize_favorite_entries(entries: &[FavoriteEntry]) -> Vec<FavoriteEntry> {
    let mut seen = HashSet::new();
    let mut normalized = Vec::new();
    for entry in entries {
        let cli_id = entry.cli_id.trim();
        let path = entry.path.trim();
        if cli_id.is_empty() || path.is_empty() {
            continue;
        }
        let key = (cli_id.to_string(), path.to_string());
        if seen.insert(key.clone()) {
            normalized.push(FavoriteEntry {
                cli_id: key.0,
                path: key.1,
            });
        }
    }
    normalized
}

pub(crate) fn read_favorite_entries() -> AppResult<Vec<FavoriteEntry>> {
    let conn = conn()?;
    read_favorite_entries_inner(&conn)
}

pub(crate) fn write_favorite_entries(entries: &[FavoriteEntry]) -> AppResult<()> {
    let conn = conn()?;
    write_favorite_entries_inner(&conn, entries)
}

pub(super) fn read_favorite_entries_inner(conn: &Connection) -> AppResult<Vec<FavoriteEntry>> {
    let mut stmt =
        conn.prepare("SELECT cli_id, path FROM favorites ORDER BY position ASC, cli_id ASC, path ASC")?;
    let rows = stmt.query_map([], |row| {
        Ok(FavoriteEntry {
            cli_id: row.get::<_, String>(0)?,
            path: row.get::<_, String>(1)?,
        })
    })?;

    let mut entries = Vec::new();
    for row in rows {
        entries.push(row?);
    }
    Ok(entries)
}

pub(super) fn write_favorite_entries_inner(
    conn: &Connection,
    entries: &[FavoriteEntry],
) -> AppResult<()> {
    let normalized = normalize_favorite_entries(entries);
    let tx = conn.unchecked_transaction()?;
    tx.execute("DELETE FROM favorites", [])?;

    let now = now_rfc3339();
    for (index, entry) in normalized.iter().enumerate() {
        tx.execute(
            "INSERT INTO favorites (cli_id, path, position, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![entry.cli_id, entry.path, index as i64, now],
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
            CREATE TABLE favorites (
                cli_id TEXT NOT NULL DEFAULT 'claude',
                path TEXT NOT NULL,
                position INTEGER NOT NULL,
                created_at TEXT NOT NULL,
                PRIMARY KEY (cli_id, path)
            );
            "#,
        )
        .unwrap();
        conn
    }

    #[test]
    fn same_path_different_cli_coexists() {
        let conn = test_conn();
        write_favorite_entries_inner(
            &conn,
            &[
                FavoriteEntry { cli_id: "claude".into(), path: "/a.jsonl".into() },
                FavoriteEntry { cli_id: "codex".into(), path: "/a.jsonl".into() },
            ],
        )
        .unwrap();
        assert_eq!(read_favorite_entries_inner(&conn).unwrap().len(), 2);
    }

    #[test]
    fn write_then_read_preserves_position_order() {
        let conn = test_conn();
        write_favorite_entries_inner(
            &conn,
            &[
                FavoriteEntry { cli_id: "codex".into(), path: "/z.jsonl".into() },
                FavoriteEntry { cli_id: "claude".into(), path: "/a.jsonl".into() },
                FavoriteEntry { cli_id: "claude".into(), path: "/m.jsonl".into() },
            ],
        )
        .unwrap();
        let entries = read_favorite_entries_inner(&conn).unwrap();
        assert_eq!(
            entries,
            vec![
                FavoriteEntry { cli_id: "codex".into(), path: "/z.jsonl".into() },
                FavoriteEntry { cli_id: "claude".into(), path: "/a.jsonl".into() },
                FavoriteEntry { cli_id: "claude".into(), path: "/m.jsonl".into() },
            ],
            "读出顺序应与写入 position 一致"
        );

        // 全量覆盖写：再写一次只剩新条目
        write_favorite_entries_inner(
            &conn,
            &[FavoriteEntry { cli_id: "claude".into(), path: "/only.jsonl".into() }],
        )
        .unwrap();
        assert_eq!(read_favorite_entries_inner(&conn).unwrap().len(), 1);
    }

    #[test]
    fn normalize_dedupes_composite_key_and_drops_empty() {
        let normalized = normalize_favorite_entries(&[
            FavoriteEntry { cli_id: "claude".into(), path: "/a.jsonl".into() },
            FavoriteEntry { cli_id: "claude".into(), path: " /a.jsonl ".into() },
            FavoriteEntry { cli_id: "codex".into(), path: "/a.jsonl".into() },
            FavoriteEntry { cli_id: "claude".into(), path: "   ".into() },
            FavoriteEntry { cli_id: " ".into(), path: "/b.jsonl".into() },
        ]);
        assert_eq!(
            normalized,
            vec![
                FavoriteEntry { cli_id: "claude".into(), path: "/a.jsonl".into() },
                FavoriteEntry { cli_id: "codex".into(), path: "/a.jsonl".into() },
            ],
            "复合键去重、去空白、去空 cli_id/path"
        );
    }
}
