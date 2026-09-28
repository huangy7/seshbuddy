use super::{conn, now_rfc3339};
use crate::cli::CliKind;
use crate::error::AppResult;
use rusqlite::{params, params_from_iter, types::Value as SqlValue, Connection};
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone)]
pub(crate) struct SessionListIndexRecord {
    pub session_path: String,
    pub session_id: String,
    pub project_path: Option<String>,
    /// Provider 原生标题（Claude custom-title/ai-title、Codex thread_name），原样存储不再二次清洗
    pub title: Option<String>,
    /// 清洗后的首条用户消息（标题兜底），扫描时已清洗，消费端原样使用
    pub first_user_message: Option<String>,
    pub first_timestamp: Option<String>,
    pub last_timestamp: Option<String>,
    pub git_branch: String,
    pub file_size: u64,
    pub modified_ms: i64,
    pub has_archive_snapshot: bool,
    pub is_archived: bool,
}

/// 会话自定义名在库中的键：`<cli_id>\u{0}<file_path>`。
///
/// 用 NUL 分隔而非 `:` —— 路径里本身可能含冒号（Windows 盘符、macOS 允许的目录名），
/// 只有选一个不可能出现在路径中的分隔符，读取端才能无歧义地还原出路径。
pub(crate) fn session_name_key(kind: CliKind, file_path: &str) -> String {
    format!("{}\u{0}{}", kind.id(), file_path)
}

/// 由库内键还原会话文件路径。库内键恒为 `session_name_key` 生成的复合形态，
/// 解析不出 `\0` 分隔符即视为非法键，返回 None。
///
/// 读写必须共用本函数：分隔符只在这一处定义，读端另立规则就会与写入端对不上，
/// 整串（含 NUL 字节）被当成会话路径推进搜索结果，用户点击必然失败。
pub(crate) fn session_name_path<'a>(kind: CliKind, key: &'a str) -> Option<&'a str> {
    let (cli_id, path) = key.split_once('\u{0}')?;
    (cli_id == kind.id()).then_some(path)
}

pub(crate) fn load_session_names() -> AppResult<HashMap<String, String>> {
    let conn = conn()?;
    let mut stmt =
        conn.prepare("SELECT session_path, display_name FROM session_names ORDER BY session_path ASC")?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;

    let mut names = HashMap::new();
    for row in rows {
        let (path, display_name) = row?;
        names.insert(path, display_name);
    }
    Ok(names)
}

pub(crate) fn write_session_names(names: &HashMap<String, String>) -> AppResult<()> {
    let mut conn = conn()?;
    let tx = conn.transaction()?;
    tx.execute("DELETE FROM session_names", [])?;

    let now = now_rfc3339();
    let mut entries: Vec<_> = names.iter().collect();
    entries.sort_by(|a, b| a.0.cmp(b.0));
    for (session_path, display_name) in entries {
        tx.execute(
            "INSERT INTO session_names (session_path, display_name, updated_at) VALUES (?1, ?2, ?3)",
            params![session_path, display_name, now],
        )?;
    }

    tx.commit()?;
    Ok(())
}

fn map_session_list_index_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<SessionListIndexRecord> {
    let session_path: String = row.get(0)?;
    let has_archive_snapshot: bool = row.get(10)?;
    Ok(SessionListIndexRecord {
        session_path: session_path.clone(),
        session_id: row.get(1)?,
        project_path: row.get(2)?,
        title: row.get(3)?,
        first_user_message: row.get(4)?,
        first_timestamp: row.get(5)?,
        last_timestamp: row.get(6)?,
        git_branch: row.get(7)?,
        file_size: row.get::<_, i64>(8)? as u64,
        modified_ms: row.get(9)?,
        has_archive_snapshot,
        is_archived: has_archive_snapshot && !Path::new(&session_path).exists(),
    })
}

pub(crate) fn count_session_list_index(kind: CliKind) -> AppResult<usize> {
    let conn = conn()?;
    let count = conn.query_row(
        "SELECT COUNT(*) FROM session_list_index WHERE cli_id = ?1",
        params![kind.id()],
        |row| row.get::<_, i64>(0),
    )?;
    Ok(count.max(0) as usize)
}

/// 清空指定 CLI 的索引数据（列表索引、搜索文档、搜索状态），
/// 保留 favorites / bookmarks / session_names 等用户数据。
/// 返回被清除的 session_path 列表（供 tantivy 物理索引逐条删除）。
pub(crate) fn clear_session_index_inner(
    conn: &Connection,
    cli_id: &str,
) -> AppResult<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT session_path FROM session_list_index WHERE cli_id = ?1",
    )?;
    let paths: Vec<String> = stmt
        .query_map(params![cli_id], |row| row.get(0))?
        .filter_map(|r| r.ok())
        .collect();

    conn.execute(
        "DELETE FROM session_list_index WHERE cli_id = ?1",
        params![cli_id],
    )?;
    conn.execute(
        "DELETE FROM session_search_docs WHERE cli_id = ?1",
        params![cli_id],
    )?;
    conn.execute(
        "DELETE FROM session_search_index_state WHERE cli_id = ?1",
        params![cli_id],
    )?;
    Ok(paths)
}

pub(crate) fn read_session_list_index_page(
    kind: CliKind,
    offset: usize,
    limit: usize,
) -> AppResult<Vec<SessionListIndexRecord>> {
    if limit == 0 {
        return Ok(Vec::new());
    }

    let conn = conn()?;
    let mut stmt = conn.prepare(
        r#"
            SELECT
                session_path,
                session_id,
                project_path,
                title,
                first_user_message,
                first_timestamp,
                last_timestamp,
                git_branch,
                file_size,
                modified_ms,
                archived_at IS NOT NULL as has_archive_snapshot
            FROM session_list_index
            WHERE cli_id = ?1
            ORDER BY COALESCE(last_timestamp, first_timestamp, '') DESC, session_path ASC
            LIMIT ?2 OFFSET ?3
            "#,
    )?;
    let rows = stmt.query_map(
        params![kind.id(), limit as i64, offset as i64],
        map_session_list_index_record,
    )?;

    let mut result = Vec::new();
    for row in rows {
        result.push(row?);
    }
    Ok(result)
}

pub(crate) fn read_session_list_index(
    kind: CliKind,
) -> AppResult<HashMap<String, SessionListIndexRecord>> {
    let conn = conn()?;
    let mut stmt = conn.prepare(
        r#"
            SELECT
                session_path,
                session_id,
                project_path,
                title,
                first_user_message,
                first_timestamp,
                last_timestamp,
                git_branch,
                file_size,
                modified_ms,
                archived_at IS NOT NULL as has_archive_snapshot
            FROM session_list_index
            WHERE cli_id = ?1
            ORDER BY session_path ASC
            "#,
    )?;
    let rows = stmt.query_map(params![kind.id()], map_session_list_index_record)?;

    let mut result = HashMap::new();
    for row in rows {
        let record = row?;
        result.insert(record.session_path.clone(), record);
    }
    Ok(result)
}

pub(crate) fn search_session_ids(
    kind: CliKind,
    query: &str,
    limit: usize,
) -> AppResult<Vec<SessionListIndexRecord>> {
    let trimmed = query.trim().to_lowercase();
    if trimmed.is_empty() || limit == 0 {
        return Ok(Vec::new());
    }

    let conn = conn()?;
    let mut stmt = conn.prepare(
        r#"
            SELECT
                session_path,
                session_id,
                project_path,
                title,
                first_user_message,
                first_timestamp,
                last_timestamp,
                git_branch,
                file_size,
                modified_ms,
                archived_at IS NOT NULL as has_archive_snapshot
            FROM session_list_index
            WHERE cli_id = ?1
              AND INSTR(lower(session_id), ?2) > 0
            ORDER BY
                CASE WHEN lower(session_id) = ?2 THEN 0 ELSE 1 END,
                COALESCE(last_timestamp, first_timestamp, '') DESC,
                session_path ASC
            LIMIT ?3
            "#,
    )?;
    let rows = stmt.query_map(
        params![kind.id(), trimmed, limit as i64],
        map_session_list_index_record,
    )?;

    let mut result = Vec::new();
    for row in rows {
        result.push(row?);
    }
    Ok(result)
}

/// 跨 CLI 标题级搜索:对指定 CLI 的 session_list_index 做 LIKE 匹配。
/// 匹配字段:title / first_user_message / session_id / session_path / project_path。
pub(crate) fn search_session_titles(
    kind: CliKind,
    query: &str,
    limit: usize,
) -> AppResult<Vec<SessionListIndexRecord>> {
    let conn = conn()?;
    search_session_titles_inner(&conn, kind.id(), query, limit)
}

fn search_session_titles_inner(
    conn: &Connection,
    cli_id: &str,
    query: &str,
    limit: usize,
) -> AppResult<Vec<SessionListIndexRecord>> {
    let escaped = query
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    let pattern = format!("%{}%", escaped.to_lowercase());
    let mut stmt = conn.prepare(
        r#"
            SELECT
                session_path,
                session_id,
                project_path,
                title,
                first_user_message,
                first_timestamp,
                last_timestamp,
                git_branch,
                file_size,
                modified_ms,
                archived_at IS NOT NULL as has_archive_snapshot
            FROM session_list_index
            WHERE cli_id = ?1 AND (
                lower(COALESCE(title, '')) LIKE ?2 ESCAPE '\' OR
                lower(COALESCE(first_user_message, '')) LIKE ?2 ESCAPE '\' OR
                lower(session_id) LIKE ?2 ESCAPE '\' OR
                lower(session_path) LIKE ?2 ESCAPE '\' OR
                lower(COALESCE(project_path, '')) LIKE ?2 ESCAPE '\'
            )
            ORDER BY COALESCE(last_timestamp, first_timestamp) DESC
            LIMIT ?3
            "#,
    )?;
    let rows = stmt.query_map(
        params![cli_id, pattern, limit as i64],
        map_session_list_index_record,
    )?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

pub(crate) fn upsert_session_list_index(
    kind: CliKind,
    records: &[SessionListIndexRecord],
) -> AppResult<()> {
    if records.is_empty() {
        return Ok(());
    }

    let mut conn = conn()?;
    let tx = conn.transaction()?;
    let now = now_rfc3339();

    for record in records {
        tx.execute(
            r#"
            INSERT INTO session_list_index (
                cli_id,
                session_path,
                session_id,
                project_path,
                title,
                first_user_message,
                first_timestamp,
                last_timestamp,
                git_branch,
                file_size,
                modified_ms,
                indexed_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
            ON CONFLICT(cli_id, session_path) DO UPDATE SET
                session_id = excluded.session_id,
                project_path = excluded.project_path,
                title = excluded.title,
                first_user_message = excluded.first_user_message,
                first_timestamp = excluded.first_timestamp,
                last_timestamp = excluded.last_timestamp,
                git_branch = excluded.git_branch,
                file_size = excluded.file_size,
                modified_ms = excluded.modified_ms,
                indexed_at = excluded.indexed_at
            "#,
            params![
                kind.id(),
                &record.session_path,
                &record.session_id,
                &record.project_path,
                &record.title,
                &record.first_user_message,
                &record.first_timestamp,
                &record.last_timestamp,
                &record.git_branch,
                record.file_size as i64,
                record.modified_ms,
                &now,
            ],
        )?;
    }

    tx.commit()?;
    Ok(())
}

pub(crate) fn delete_session_list_index_paths(
    kind: CliKind,
    session_paths: &[String],
) -> AppResult<()> {
    if session_paths.is_empty() {
        return Ok(());
    }

    let mut conn = conn()?;
    let tx = conn.transaction()?;

    for session_path in session_paths {
        tx.execute(
            "DELETE FROM session_list_index WHERE cli_id = ?1 AND session_path = ?2",
            params![kind.id(), session_path],
        )?;
    }

    tx.commit()?;
    Ok(())
}

/// 会话日志「换代号」时的路径搬迁：把旧代路径上的持久化记录整体改写到新代路径。
///
/// 会话日志按格式代命名，迁移到新一代后同一会话目录里历史代文件仍然保留、而当前内容
/// 落在新一代文件名上。扫描层解析出当前代路径后调用本函数，否则旧路径会留下一条再也
/// 扫不到的索引行：同一会话在列表里出现两次，且归档快照与活动条目失联。
///
/// 搬迁范围是「必须跟着会话走」的持久化状态：列表索引、归档元数据、自定义名称。
/// 归档快照的物理文件由 `move_archive_artifacts` 在事务提交后另行搬迁；全文检索属于
/// 可重建的派生物，旧路径的检索状态直接作废、由新路径重新索引（Tantivy 文档按路径
/// 键控，改写路径键会与既有 message_index 冲突），失败只告警，不影响会话可见性。
pub(crate) fn rekey_session_paths(kind: CliKind, moves: &[(String, String)]) -> AppResult<usize> {
    if moves.is_empty() {
        return Ok(0);
    }

    let mut moved = 0usize;
    {
        let mut conn = conn()?;
        let tx = conn.transaction()?;
        for (old_path, new_path) in moves {
            if old_path == new_path {
                continue;
            }
            // 新路径上可能已存在同会话记录（例如搬迁中途中断后重跑），
            // OR REPLACE 让同一次搬迁收敛到新路径上的唯一一条记录
            moved += tx.execute(
                "UPDATE OR REPLACE session_list_index SET session_path = ?1 WHERE cli_id = ?2 AND session_path = ?3",
                params![new_path, kind.id(), old_path],
            )?;
            tx.execute(
                "UPDATE OR REPLACE archived_session_content SET session_path = ?1 WHERE cli_id = ?2 AND session_path = ?3",
                params![new_path, kind.id(), old_path],
            )?;
            // 自定义名与其它两张表同用复合键，随路径一起搬迁
            tx.execute(
                "UPDATE OR REPLACE session_names SET session_path = ?1 WHERE session_path = ?2",
                params![
                    format!("{}\u{0}{}", kind.id(), new_path),
                    format!("{}\u{0}{}", kind.id(), old_path)
                ],
            )?;
        }
        tx.commit()?;
    }

    for (old_path, new_path) in moves {
        super::session_archive::move_archive_artifacts(kind.id(), old_path, new_path);
    }

    let retired: Vec<String> = moves
        .iter()
        .filter(|(old_path, new_path)| old_path != new_path)
        .map(|(old_path, _)| old_path.clone())
        .collect();
    if let Err(err) = super::session_search::delete_session_search_paths(kind, &retired) {
        tracing::warn!(
            "作废旧路径的检索状态失败（不影响会话可见性，下轮扫描会重建）: cli={}, err={}",
            kind.id(),
            err.diagnostic()
        );
    }

    Ok(moved)
}

/// 列出该 CLI 下路径**不落在** `scope_root` 内的索引行路径。
///
/// 归属判定用**路径分量**比较（`Path::starts_with`）而不是字符串前缀：`/a/sessions-old`
/// 不能被 `/a/sessions` 认领。用字符串前缀会让切换数据源时误删（或误留）整批会话，
/// 而这类错误在真实数据上表现为"某天起会话列表莫名少了一整个目录"。
pub(crate) fn session_index_paths_outside_scope(
    kind: CliKind,
    scope_root: &Path,
) -> AppResult<Vec<String>> {
    let conn = conn()?;
    let mut stmt = conn.prepare("SELECT session_path FROM session_list_index WHERE cli_id = ?1")?;
    let rows = stmt.query_map(params![kind.id()], |row| row.get::<_, String>(0))?;
    Ok(rows
        .filter_map(|r| r.ok())
        .filter(|session_path| !Path::new(session_path).starts_with(scope_root))
        .collect())
}

pub(crate) fn read_session_list_index_for_paths(
    kind: CliKind,
    session_paths: &[String],
) -> AppResult<HashMap<String, SessionListIndexRecord>> {
    if session_paths.is_empty() {
        return Ok(HashMap::new());
    }

    let conn = conn()?;
    let placeholders = vec!["?"; session_paths.len()].join(", ");
    let sql = format!(
        r#"
        SELECT
            session_path,
            session_id,
            project_path,
            title,
            first_user_message,
            first_timestamp,
            last_timestamp,
            git_branch,
            file_size,
            modified_ms,
            archived_at IS NOT NULL as has_archive_snapshot
        FROM session_list_index
        WHERE cli_id = ?1 AND session_path IN ({})
        ORDER BY session_path ASC
        "#,
        placeholders
    );
    let mut stmt = conn.prepare(&sql)?;

    let mut params_vec = Vec::with_capacity(session_paths.len() + 1);
    params_vec.push(SqlValue::Text(kind.id().to_string()));
    params_vec.extend(session_paths.iter().cloned().map(SqlValue::Text));

    let rows = stmt.query_map(params_from_iter(params_vec), map_session_list_index_record)?;

    let mut result = HashMap::new();
    for row in rows {
        let record = row?;
        result.insert(record.session_path.clone(), record);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_index_test_schema(conn: &Connection) {
        conn.execute_batch(
            r#"
            CREATE TABLE session_list_index (
                cli_id TEXT NOT NULL,
                session_path TEXT NOT NULL,
                session_id TEXT NOT NULL,
                project_path TEXT,
                title TEXT,
                first_user_message TEXT,
                first_timestamp TEXT,
                last_timestamp TEXT,
                git_branch TEXT NOT NULL DEFAULT '',
                file_size INTEGER NOT NULL DEFAULT 0,
                modified_ms INTEGER NOT NULL DEFAULT 0,
                indexed_at TEXT NOT NULL DEFAULT '',
                archived_at TEXT,
                PRIMARY KEY (cli_id, session_path)
            );
            CREATE TABLE session_search_docs (
                cli_id TEXT NOT NULL,
                session_path TEXT NOT NULL,
                modified_ms INTEGER NOT NULL,
                message_index INTEGER NOT NULL,
                search_text TEXT NOT NULL,
                indexed_at TEXT NOT NULL DEFAULT '',
                PRIMARY KEY (cli_id, session_path, message_index)
            );
            CREATE TABLE session_search_index_state (
                cli_id TEXT NOT NULL,
                session_path TEXT NOT NULL,
                modified_ms INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (cli_id, session_path)
            );
            CREATE TABLE favorites (
                cli_id TEXT NOT NULL,
                session_path TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT '',
                PRIMARY KEY (cli_id, session_path)
            );
            CREATE TABLE session_names (
                session_path TEXT PRIMARY KEY,
                display_name TEXT NOT NULL
            );
            "#,
        )
        .unwrap();
    }

    #[test]
    fn clear_session_index_clears_only_index_tables_for_cli() {
        let conn = Connection::open_in_memory().unwrap();
        create_index_test_schema(&conn);

        conn.execute_batch(
            r#"
            INSERT INTO session_list_index (cli_id, session_path, session_id) VALUES
                ('claude', '/a.jsonl', 'a'), ('claude', '/b.jsonl', 'b'), ('codex', '/c.jsonl', 'c');
            INSERT INTO session_search_docs (cli_id, session_path, modified_ms, message_index, search_text) VALUES
                ('claude', '/a.jsonl', 1, 0, 'hello'), ('codex', '/c.jsonl', 1, 0, 'world');
            INSERT INTO session_search_index_state (cli_id, session_path) VALUES
                ('claude', '/a.jsonl'), ('codex', '/c.jsonl');
            INSERT INTO favorites (cli_id, session_path) VALUES ('claude', '/a.jsonl');
            INSERT INTO session_names (session_path, display_name) VALUES ('claude' || char(0) || '/a.jsonl', '我的会话');
            "#,
        )
        .unwrap();

        let removed = clear_session_index_inner(&conn, "claude").unwrap();
        assert_eq!(removed.len(), 2);

        // 目标 cli 的索引被清空
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM session_list_index WHERE cli_id = 'claude'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM session_search_docs WHERE cli_id = 'claude'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM session_search_index_state WHERE cli_id = 'claude'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);

        // 其他 cli 的索引保留
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM session_list_index WHERE cli_id = 'codex'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);

        // 用户数据保留
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM favorites", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM session_names", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn search_session_titles_matches_and_escapes_like_wildcards() {
        let conn = Connection::open_in_memory().unwrap();
        create_index_test_schema(&conn);

        conn.execute_batch(
            r#"
            INSERT INTO session_list_index (cli_id, session_path, session_id, title, first_user_message, project_path, last_timestamp) VALUES
                ('codex', '/reports/a.jsonl', 'sid-a', '季度报表汇总', NULL, '/proj/a', '2026-08-01T00:00:00Z'),
                ('codex', '/other/b.jsonl', 'sid-b', '100 个优化点', NULL, '/proj/b', '2026-08-02T00:00:00Z'),
                ('codex', '/other/c.jsonl', 'sid-c', '覆盖率 100% 达标', NULL, '/proj/c', '2026-08-03T00:00:00Z'),
                ('claude', '/claude/d.jsonl', 'sid-d', '报表 claude 侧', NULL, '/proj/d', '2026-08-04T00:00:00Z');
            "#,
        )
        .unwrap();

        // 中文标题命中
        let hits = search_session_titles_inner(&conn, "codex", "报表", 20).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].session_id, "sid-a");

        // % 被转义:搜 "100%" 只命中字面含 "100%" 的记录,不命中只含 "100" 的
        let hits = search_session_titles_inner(&conn, "codex", "100%", 20).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].session_id, "sid-c");

        // cli_id 过滤:claude 侧的记录不会出现在 codex 搜索里
        let hits = search_session_titles_inner(&conn, "claude", "报表", 20).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].session_id, "sid-d");

        // limit 语义:"1" 命中 sid-b/sid-c 两条,limit 1 截断,按 last_timestamp DESC 新的在前
        let hits = search_session_titles_inner(&conn, "codex", "1", 1).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].session_id, "sid-c");

        // project_path / session_path 也参与匹配
        let hits = search_session_titles_inner(&conn, "codex", "/proj/a", 20).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].session_id, "sid-a");
        let hits = search_session_titles_inner(&conn, "codex", "/reports/a.jsonl", 20).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].session_id, "sid-a");
    }
}
