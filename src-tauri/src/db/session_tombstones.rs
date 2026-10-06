use crate::cli::CliKind;
use crate::error::AppResult;
use rusqlite::{params, Connection};

use std::collections::HashSet;

/// 内存级会话墓碑过滤器
///
/// 用于在全量扫描或批量构建全文索引前预加载指定 CLI 的全部墓碑，
/// 避免在成千上万个会话遍历循环中重复向连接池申请连接与执行 N+1 SQL 查询。
#[derive(Debug, Clone, Default)]
pub(crate) struct TombstoneFilter {
    pub ids: HashSet<String>,
    pub paths: HashSet<String>,
}

impl TombstoneFilter {
    #[inline]
    pub fn is_tombstoned(&self, session_id: &str, file_path: &str) -> bool {
        (!session_id.is_empty() && self.ids.contains(session_id))
            || (!file_path.is_empty() && self.paths.contains(file_path))
    }
}

/// 一次性加载指定 CLI 的全部有效墓碑集合
pub(crate) fn load_tombstone_filter_inner(
    conn: &Connection,
    cli_id: &str,
) -> AppResult<TombstoneFilter> {
    let mut stmt = conn.prepare(
        "SELECT session_id, file_path FROM session_tombstones WHERE cli_id = ?1",
    )?;
    let rows = stmt.query_map(params![cli_id], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;

    let mut ids = HashSet::new();
    let mut paths = HashSet::new();
    for row in rows {
        let (id, path) = row?;
        if !id.is_empty() {
            ids.insert(id);
        }
        if !path.is_empty() {
            paths.insert(path);
        }
    }
    Ok(TombstoneFilter { ids, paths })
}

/// 快速通过连接池预加载指定 CLI 的墓碑过滤器
pub(crate) fn load_tombstone_filter(kind: CliKind) -> AppResult<TombstoneFilter> {
    let conn = super::conn()?;
    load_tombstone_filter_inner(&conn, kind.id())
}

/// 检查会话是否已被标记为墓碑（双轨判定：逻辑键匹配 或 物理路径匹配）
///
/// 业务背景：
/// 在会话生命周期管理中，用户删除会话后，后台扫描器可能会因文件系统延迟、
/// 跨目录移动或并发扫描导致已删除会话重新进入索引。
/// 通过 (cli_id, session_id) 逻辑主键与 file_path 物理文件路径的双轨校验，
/// 确保即便会话文件发生异地更名或路径变动，均能可靠拦截，防止已删除会话复活。
pub(crate) fn is_tombstoned_inner(
    conn: &Connection,
    cli_id: &str,
    session_id: &str,
    file_path: &str,
) -> AppResult<bool> {
    if session_id.is_empty() && file_path.is_empty() {
        return Ok(false);
    }
    let exists: bool = conn.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM session_tombstones 
            WHERE cli_id = ?1 
              AND ((?2 != '' AND session_id = ?2) OR (?3 != '' AND file_path = ?3))
        )",
        params![cli_id, session_id, file_path],
        |r| r.get(0),
    )?;
    Ok(exists)
}

/// 快速通过连接池检查会话是否处于墓碑状态
pub(crate) fn is_session_tombstoned(
    kind: CliKind,
    session_id: &str,
    file_path: &str,
) -> AppResult<bool> {
    let conn = super::conn()?;
    is_tombstoned_inner(&conn, kind.id(), session_id, file_path)
}

/// 记录单条会话墓碑条目
///
/// 将已删除的会话逻辑标识与物理文件路径登记至墓碑持久层，
/// 使用 UTC ISO 8601 时间戳记录删除时间。
pub(crate) fn record_tombstone_inner(
    conn: &Connection,
    cli_id: &str,
    session_id: &str,
    file_path: &str,
) -> AppResult<()> {
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT OR REPLACE INTO session_tombstones (cli_id, session_id, file_path, deleted_at) VALUES (?1, ?2, ?3, ?4)",
        params![cli_id, session_id, file_path, now],
    )?;
    Ok(())
}

/// 解除特定会话的墓碑状态
///
/// 当用户主动新建或显式拉起同 ID 会话时，解除其墓碑屏蔽，允许其重新被索引与展示。
#[cfg(test)]
pub(crate) fn clear_session_tombstone_inner(
    conn: &Connection,
    cli_id: &str,
    session_id: &str,
) -> AppResult<()> {
    conn.execute(
        "DELETE FROM session_tombstones WHERE cli_id = ?1 AND session_id = ?2",
        params![cli_id, session_id],
    )?;
    Ok(())
}

/// 清理超过指定保留天数（如 30 天）的历史墓碑记录
///
/// 返回成功清理的过期条目数。
pub(crate) fn purge_expired_tombstones_inner(
    conn: &Connection,
    retention_days: i64,
) -> AppResult<usize> {
    if retention_days <= 0 {
        return Ok(0);
    }
    let cutoff = (chrono::Utc::now() - chrono::Duration::days(retention_days)).to_rfc3339();
    let count = conn.execute(
        "DELETE FROM session_tombstones WHERE deleted_at < ?1",
        params![cutoff],
    )?;
    Ok(count)
}

/// 为一条会话路径推导墓碑逻辑键。
///
/// 优先用库里已有的 session_id；缺失时按 CLI 规则从路径推导（Antigravity 取 conversation_id，
/// DSH 会话文件名固定为 `session.jsonl*` 故取父目录名，其余取文件名去后缀）。
///
/// 通用名防卫：绝不让 `transcript` / `session` / `session.jsonl*` 这类名字成为墓碑键 ——
/// 它们会被成批会话共用，一旦入库就会连带拦掉大量正常会话；无法提取有效 ID 时改用
/// 基于物理路径的唯一合成键。
///
/// 删除端与归档恢复/重建端**必须共用本函数**：两侧推导规则一旦不一致，墓碑就拦不住
/// 本该拦住的复活路径（这与自定义名键读写不一致是同一类缺陷）。
pub(crate) fn tombstone_session_id(
    kind: CliKind,
    session_path: &str,
    db_session_id: Option<&str>,
) -> String {
    let fallback_id = crate::cli_registry::source_for(kind)
        .tombstone_fallback_id(std::path::Path::new(session_path));
    let session_id = db_session_id
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(&fallback_id);

    if session_id.is_empty()
        || session_id == "transcript"
        || session_id == "session"
        || session_id.starts_with("session.jsonl")
    {
        format!("path:{}", session_path)
    } else {
        session_id.to_string()
    }
}

/// 墓碑默认保留期（天）。超过这个期限且文件始终没回来的墓碑记录不再需要拦截。
pub(crate) const TOMBSTONE_RETENTION_DAYS: i64 = 30;

/// 清除「文件已回到原位」的墓碑（内层实现，接收连接以便测试）。
///
/// 判定必须落在墓碑记录的那个路径上：只有用户真的把文件放回原处才满足，
/// 会话在别处留有副本不会误触发 —— 这正是双轨墓碑要防的复活场景。
///
/// 「文件是否回来」经会话存在性入口判定而非直接 `Path::exists()`：墓碑同样覆盖库型会话，
/// 其身份是 `cli://…` 而非文件路径，对它们做文件系统判定会恒为「不存在」，
/// 于是放回的会话永远清不掉自己的墓碑。
pub(crate) fn purge_restored_tombstones_inner(conn: &Connection) -> AppResult<usize> {
    let rows: Vec<(String, String, String)> = {
        let mut stmt = conn.prepare(
            "SELECT cli_id, session_id, file_path FROM session_tombstones",
        )?;
        // 必须先把 query_map 的结果落到局部变量再收集：直接把它作为块尾表达式时，
        // 临时值会活到块结束、晚于 stmt 析构，借用检查不通过
        let mapped =
            stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
        mapped.filter_map(Result::ok).collect()
    };

    let restored: Vec<(String, String)> = rows
        .iter()
        .filter(|(cli_id, _, file_path)| super::session_exists(cli_id, file_path))
        .map(|(cli_id, session_id, _)| (cli_id.clone(), session_id.clone()))
        .collect();
    if restored.is_empty() {
        return Ok(0);
    }

    let tx = conn.unchecked_transaction()?;
    let mut cleared = 0usize;
    for (cli_id, session_id) in &restored {
        cleared += tx.execute(
            "DELETE FROM session_tombstones WHERE cli_id = ?1 AND session_id = ?2",
            params![cli_id, session_id],
        )?;
    }
    tx.commit()?;
    Ok(cleared)
}

/// 墓碑的完整生命周期维护：清除「文件已回到原位」与「已过期」的墓碑。
///
/// 删除动作是「移到废纸篓」，本身可逆；而墓碑按 `session_id OR file_path` 拦截扫描，
/// 一旦写入就再没有任何清除路径 —— 用户从废纸篓恢复文件后，该会话在重启、重建索引后
/// 依然不可见，只能删库才能找回。保留期此前也只是写在注释里，从未被强制执行。
pub(crate) fn purge_tombstones(retention_days: i64) -> AppResult<(usize, usize)> {
    let conn = super::conn()?;
    let cleared = purge_restored_tombstones_inner(&conn)?;
    let expired = purge_expired_tombstones_inner(&conn, retention_days)?;
    Ok((cleared, expired))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn create_test_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE session_tombstones (
                cli_id      TEXT NOT NULL,
                session_id  TEXT NOT NULL,
                file_path   TEXT NOT NULL,
                deleted_at  TEXT NOT NULL,
                PRIMARY KEY (cli_id, session_id, file_path)
            );
            CREATE INDEX idx_session_tombstones_path ON session_tombstones(file_path);
            "#,
        )
        .unwrap();
        conn
    }

    #[test]
    fn test_tombstone_dual_track_query() {
        let conn = create_test_db();
        record_tombstone_inner(&conn, "claude", "sess-1", "/path/to/sess-1.jsonl").unwrap();

        // 1. 逻辑会话 ID 命中（即便路径不同，如异地备份恢复或目录移动）
        assert!(is_tombstoned_inner(&conn, "claude", "sess-1", "/different/path/moved.jsonl").unwrap());

        // 2. 物理文件路径命中（即便逻辑 ID 变更）
        assert!(is_tombstoned_inner(&conn, "claude", "different-id", "/path/to/sess-1.jsonl").unwrap());

        // 3. 逻辑 ID 与路径均不匹配
        assert!(!is_tombstoned_inner(&conn, "claude", "sess-2", "/path/to/sess-2.jsonl").unwrap());

        // 4. 跨 CLI 隔离：其他 CLI 的同名会话或路径不应互相误杀
        assert!(!is_tombstoned_inner(&conn, "codex", "sess-1", "/path/to/sess-1.jsonl").unwrap());

        // 5. 空串防卫：全空入参不匹配任何条目
        assert!(!is_tombstoned_inner(&conn, "claude", "", "").unwrap());
    }

    #[test]
    fn test_tombstone_filter_in_memory() {
        let mut filter = TombstoneFilter::default();
        filter.ids.insert("tombstone-id".to_string());
        filter.paths.insert("/path/to/dead.jsonl".to_string());

        // 1. ID 命中
        assert!(filter.is_tombstoned("tombstone-id", "/any/other/path.jsonl"));
        // 2. 路径命中
        assert!(filter.is_tombstoned("other-id", "/path/to/dead.jsonl"));
        // 3. 均未命中
        assert!(!filter.is_tombstoned("alive-id", "/path/to/alive.jsonl"));
        // 4. 空入参防卫：不会误伤空 ID 或空路径
        assert!(!filter.is_tombstoned("", ""));
        assert!(!filter.is_tombstoned("", "/path/to/alive.jsonl"));
        assert!(!filter.is_tombstoned("alive-id", ""));
    }

    #[test]
    fn test_tombstone_expiry_cleanup() {
        let conn = create_test_db();
        // 插入一条 31 天前的记录
        let old_time = (chrono::Utc::now() - chrono::Duration::days(31)).to_rfc3339();
        conn.execute(
            "INSERT INTO session_tombstones (cli_id, session_id, file_path, deleted_at) VALUES ('claude', 'old-sess', '/old.jsonl', ?1)",
            rusqlite::params![old_time],
        ).unwrap();

        // 插入一条刚删除的记录
        record_tombstone_inner(&conn, "claude", "new-sess", "/new.jsonl").unwrap();

        let purged = purge_expired_tombstones_inner(&conn, 30).unwrap();
        assert_eq!(purged, 1);
        assert!(!is_tombstoned_inner(&conn, "claude", "old-sess", "/old.jsonl").unwrap());
        assert!(is_tombstoned_inner(&conn, "claude", "new-sess", "/new.jsonl").unwrap());
    }

    #[test]
    fn test_clear_session_tombstone() {
        let conn = create_test_db();
        record_tombstone_inner(&conn, "claude", "sess-123", "/path/to/a.jsonl").unwrap();
        assert!(is_tombstoned_inner(&conn, "claude", "sess-123", "/path/to/a.jsonl").unwrap());

        clear_session_tombstone_inner(&conn, "claude", "sess-123").unwrap();
        assert!(!is_tombstoned_inner(&conn, "claude", "sess-123", "/path/to/a.jsonl").unwrap());

        // 对不存在的记录执行解除保持幂等
        assert!(clear_session_tombstone_inner(&conn, "claude", "non-existent").is_ok());
    }

    #[test]
    fn test_purge_expired_tombstones_zero_or_negative_days() {
        let conn = create_test_db();
        let old_time = (chrono::Utc::now() - chrono::Duration::days(100)).to_rfc3339();
        conn.execute(
            "INSERT INTO session_tombstones (cli_id, session_id, file_path, deleted_at) VALUES ('claude', 'old-sess', '/old.jsonl', ?1)",
            rusqlite::params![old_time],
        ).unwrap();

        // 负数与零天数防卫，不执行任何清空
        assert_eq!(purge_expired_tombstones_inner(&conn, 0).unwrap(), 0);
        assert_eq!(purge_expired_tombstones_inner(&conn, -1).unwrap(), 0);
        assert!(is_tombstoned_inner(&conn, "claude", "old-sess", "/old.jsonl").unwrap());
    }

    #[test]
    fn test_insert_or_replace_idempotency_and_multi_path() {
        let conn = create_test_db();
        // 同一会话的多份副本路径共存，不相互覆盖
        record_tombstone_inner(&conn, "claude", "sess-1", "/path/1.jsonl").unwrap();
        record_tombstone_inner(&conn, "claude", "sess-1", "/path/2.jsonl").unwrap();

        assert!(is_tombstoned_inner(&conn, "claude", "other", "/path/1.jsonl").unwrap());
        assert!(is_tombstoned_inner(&conn, "claude", "other", "/path/2.jsonl").unwrap());

        // 同一条目重复插入幂等
        record_tombstone_inner(&conn, "claude", "sess-1", "/path/1.jsonl").unwrap();
        assert!(is_tombstoned_inner(&conn, "claude", "sess-1", "/path/1.jsonl").unwrap());
    }

    /// 文件被放回墓碑记录的那个路径时，墓碑必须被清除 —— 否则从废纸篓恢复的会话
    /// 会永久不可见（重启、重建索引都无效）。文件仍缺失的墓碑不受影响。
    #[test]
    fn purge_restored_tombstones_clears_only_returned_files() {
        let conn = create_test_db();
        let dir = tempfile::tempdir().unwrap();
        let restored_path = dir.path().join("restored.jsonl");
        std::fs::write(&restored_path, b"{}").unwrap();
        let still_gone = dir.path().join("still-gone.jsonl");

        record_tombstone_inner(&conn, "claude", "restored", restored_path.to_str().unwrap()).unwrap();
        record_tombstone_inner(&conn, "claude", "gone", still_gone.to_str().unwrap()).unwrap();

        let cleared = purge_restored_tombstones_inner(&conn).unwrap();

        assert_eq!(cleared, 1, "只应清除文件已回到原位的墓碑");
        assert!(!is_tombstoned_inner(&conn, "claude", "restored", restored_path.to_str().unwrap()).unwrap());
        assert!(
            is_tombstoned_inner(&conn, "claude", "gone", still_gone.to_str().unwrap()).unwrap(),
            "文件仍未回来的墓碑必须保留，否则已删会话会复活"
        );
    }
}
