//! 助手快捷用语库：名称 + prompt 模板（支持 {{变量}}，前端填入时展开）。
//! 短语全部由用户创建与维护，不预置内置内容。

use crate::error::{AppError, AppResult};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuickPhrase {
    pub id: String,
    pub name: String,
    pub content: String,
    pub sort: i64,
    pub created_at: String,
    pub updated_at: String,
}

fn row_to_phrase(row: &rusqlite::Row<'_>) -> rusqlite::Result<QuickPhrase> {
    Ok(QuickPhrase {
        id: row.get(0)?,
        name: row.get(1)?,
        content: row.get(2)?,
        sort: row.get(3)?,
        created_at: row.get(4)?,
        updated_at: row.get(5)?,
    })
}

const COLS: &str = "id, name, content, sort, created_at, updated_at";

pub fn list_quick_phrases_with(conn: &Connection) -> AppResult<Vec<QuickPhrase>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {} FROM assistant_quick_phrases ORDER BY sort, created_at",
        COLS
    ))?;
    let rows = stmt.query_map([], row_to_phrase)?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// 新建（id=None）或更新（id=Some）快捷用语；返回 id
pub fn save_quick_phrase_with(
    conn: &Connection,
    id: Option<&str>,
    name: &str,
    content: &str,
) -> AppResult<String> {
    let name = name.trim();
    let content = content.trim();
    if name.is_empty() || content.is_empty() {
        return Err(AppError::coded("assistant.quick_phrase_empty"));
    }
    let now = crate::db::now_rfc3339();
    match id {
        Some(id) => {
            let affected = conn.execute(
                "UPDATE assistant_quick_phrases SET name = ?1, content = ?2, updated_at = ?3 WHERE id = ?4",
                params![name, content, now, id],
            )?;
            if affected == 0 {
                return Err(AppError::coded("assistant.quick_phrase_missing"));
            }
            Ok(id.to_string())
        }
        None => {
            let new_id = uuid::Uuid::new_v4().to_string();
            let sort: i64 = conn
                .query_row("SELECT COALESCE(MAX(sort), -1) + 1 FROM assistant_quick_phrases", [], |r| {
                    r.get(0)
                })?;
            conn.execute(
                "INSERT INTO assistant_quick_phrases (id, name, content, sort, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![new_id, name, content, sort, now, now],
            )?;
            Ok(new_id)
        }
    }
}

pub fn delete_quick_phrase_with(conn: &Connection, id: &str) -> AppResult<()> {
    let affected = conn.execute("DELETE FROM assistant_quick_phrases WHERE id = ?1", params![id])?;
    if affected == 0 {
        return Err(AppError::coded("assistant.quick_phrase_missing"));
    }
    Ok(())
}

// ── 连接池包装 ──
pub fn list_quick_phrases() -> AppResult<Vec<QuickPhrase>> {
    let conn = crate::db::conn()?;
    list_quick_phrases_with(&conn)
}

pub fn save_quick_phrase(id: Option<&str>, name: &str, content: &str) -> AppResult<String> {
    let conn = crate::db::conn()?;
    save_quick_phrase_with(&conn, id, name, content)
}

pub fn delete_quick_phrase(id: &str) -> AppResult<()> {
    let conn = crate::db::conn()?;
    delete_quick_phrase_with(&conn, id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::create_base_schema(&conn).unwrap();
        conn
    }

    #[test]
    fn save_creates_and_updates() {
        let conn = test_conn();
        let id = save_quick_phrase_with(&conn, None, " 复盘项目 ", " 看看 {{昨天}} 干了啥 ")
            .unwrap();
        let list = list_quick_phrases_with(&conn).unwrap();
        let created = list.iter().find(|p| p.id == id).unwrap();
        assert_eq!(created.name, "复盘项目"); // trim 生效
        assert_eq!(created.sort, 0); // 首条短语从 0 起编号

        save_quick_phrase_with(&conn, Some(&id), "改名", "新内容 {{今天}}").unwrap();
        let updated = list_quick_phrases_with(&conn)
            .unwrap()
            .into_iter()
            .find(|p| p.id == id)
            .unwrap();
        assert_eq!(updated.name, "改名");
        assert_eq!(updated.content, "新内容 {{今天}}");
    }

    #[test]
    fn save_rejects_empty_fields() {
        let conn = test_conn();
        assert!(save_quick_phrase_with(&conn, None, "", "内容").is_err());
        assert!(save_quick_phrase_with(&conn, None, "名字", "  ").is_err());
        assert!(save_quick_phrase_with(&conn, Some("不存在"), "名", "内容").is_err());
    }

    #[test]
    fn delete_missing_errors() {
        let conn = test_conn();
        assert!(delete_quick_phrase_with(&conn, "不存在").is_err());
    }
}
