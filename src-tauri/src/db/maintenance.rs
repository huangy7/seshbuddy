use super::{
    backfill_archive_sidecars, conn, purge_expired_archives, purge_tombstones,
    restore_missing_archived_index_rows, session_exists, snapshot_aging_sessions, write_tx,
    TOMBSTONE_RETENTION_DAYS,
};
use crate::cli::CliKind;
use crate::cli_registry::SessionLocator;
use crate::error::{AppError, AppResult};
use rusqlite::params;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

/// 归档索引恢复路径**已验证可用**的 CLI 范围。
///
/// 这不是「这些 CLI 支持归档恢复」的能力声明，而是「这些 CLI 的端到端恢复已实测通过」的
/// 历史数据修复范围：本路径救回的是「有归档快照但缺会话列表索引行」的旧缺陷遗留数据，
/// 其正确性只能靠逐 CLI 实测确认，无法从能力描述符推导，因此不取 `CliKind::ALL`。
/// 新增 CLI 若要纳入，先补上该 CLI 的归档恢复实测，再追加到本清单。
pub(crate) const ARCHIVE_RESTORE_VERIFIED_CLIS: &[CliKind] =
    &[CliKind::Claude, CliKind::Codex, CliKind::Gemini];

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

    // 删除键由定位符（会话身份）导出，而不是把行里的裸串原样透传：存在性判定与删除
    // 因此共用同一套身份 → 键的转换，不会各认一套键格式。`File` 定位符的 `to_key()`
    // 逐字节等于原路径串（见 `cli_registry::source` 的单测），故对文件型会话行为不变。
    // 认不出 cli_id 的行到不了这里（`session_exists` 已判其「存在」而滤掉），
    // 末尾的 `unwrap_or` 只是兜底，方向仍是「照旧删除」。
    let orphans: Vec<(String, String)> = pairs
        .into_iter()
        .filter(|(cli_id, path, archived_at)| !session_exists(cli_id, path) && archived_at.is_none())
        .map(|(cli_id, path, _)| {
            let key = CliKind::from_id(Some(cli_id.as_str()))
                .map(|kind| SessionLocator::decode(kind, &path).to_key())
                .unwrap_or(path);
            (cli_id, key)
        })
        .collect();

    if orphans.is_empty() {
        return Ok(0);
    }

    let mut conn = conn()?;
    let tx = write_tx(&mut conn)?;

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
    let tx = write_tx(&mut conn)?;

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
    for &kind in ARCHIVE_RESTORE_VERIFIED_CLIS {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// 归档恢复清单是「逐 CLI 实测」得出的范围，成员会随验证进度增减，因此不冻结精确成员。
    /// 但清单本身必须结构自洽：重复成员会让同一 CLI 被恢复两次，混入非 `CliKind` 成员则
    /// 说明有人把别的枚举塞了进来。本测试只守这两条不变量。
    #[test]
    fn archive_restore_verified_clis_are_unique_members_of_all() {
        let mut seen: Vec<&str> = Vec::new();
        for kind in ARCHIVE_RESTORE_VERIFIED_CLIS {
            assert!(
                CliKind::ALL.contains(kind),
                "归档恢复清单含非 CliKind::ALL 成员: {}",
                kind.id()
            );
            assert!(
                !seen.contains(&kind.id()),
                "归档恢复清单含重复成员: {}",
                kind.id()
            );
            seen.push(kind.id());
        }
    }

    /// 名单之外还有哪些 CLI，必须**显式**列出来。
    ///
    /// 原守卫只查「名单里的成员合法且不重复」，**不查完备** —— 新变体静默落在名单外，
    /// 它的旧归档数据不会被修复，而没有任何信号。这条把补集钉住：新 CLI 会落进补集，
    /// 测试红，逼作者去补它的归档恢复实测（或显式列进「尚未验证」）。
    #[test]
    fn clis_outside_the_verified_range_are_explicit() {
        let outside: Vec<&str> = CliKind::ALL
            .iter()
            .filter(|k| !ARCHIVE_RESTORE_VERIFIED_CLIS.contains(k))
            .map(|k| k.id())
            .collect();
        assert_eq!(
            outside,
vec!["workbuddy", "dsh", "antigravity", "opencode", "cursor", "pi", "aider"],
            "已验证范围之外的 CLI 集合变了：新 CLI 必须在此显式表态"
        );
    }

    /// 跨层守卫：库型会话（`cli://…`）的索引行必须能在启动期孤儿清理中存活。
    ///
    /// 这是本期最关键的一条回归。清理把「会话不存在」读作**删除依据**：一旦把库型会话
    /// 当成普通路径去 `Path::exists()`，它恒为 `false`，行会被物理删除、用户的会话在下次
    /// 启动时消失。
    ///
    /// 夹具用本构建**尚不认识的** CLI id（`legacy-cli-not-in-build`）：`db::session_exists`
    /// 对解析不成 `CliKind` 的 cli_id 走「不认识 → 视为存在」那条臂，这正是本测试要钉住的
    /// 取舍 —— 方向判错会物理删掉一个仍有数据的会话。已登记为 `CliKind` 的库型源另见
    /// `purge_keeps_present_library_session_and_drops_absent`。
    ///
    /// 反向的一半同样必要：文件确实缺失的行必须被清掉。否则一个「从不删除任何东西」的
    /// 清理也会让本测试通过，守卫等于没测。
    #[test]
    fn purge_keeps_virtual_session_and_drops_absent_file() {
        let virtual_cli = "legacy-cli-not-in-build";
        let virtual_path = "cli://legacy-cli-not-in-build/ses_guard_virtual";
        let absent_cli = CliKind::Claude.id();
        let absent_path = "/tmp/seshbuddy-guard-absent-should-be-purged.jsonl";

        let insert = |cli: &str, path: &str| {
            let conn = conn().unwrap();
            conn.execute(
                "INSERT OR REPLACE INTO session_list_index \
                 (cli_id, session_path, session_id, project_path, title, first_user_message, \
                  first_timestamp, last_timestamp, git_branch, file_size, modified_ms, indexed_at, archived_at) \
                 VALUES (?1, ?2, '', NULL, NULL, NULL, NULL, NULL, '', 0, 0, '', NULL)",
                params![cli, path],
            )
            .unwrap();
        };
        let in_index = |cli: &str, path: &str| -> bool {
            let conn = conn().unwrap();
            conn.query_row(
                "SELECT COUNT(*) FROM session_list_index WHERE cli_id = ?1 AND session_path = ?2",
                params![cli, path],
                |row| row.get::<_, i64>(0),
            )
            .unwrap()
                > 0
        };

        // 前置清理，保证幂等（套件共用一个落盘应用库）
        {
            let conn = conn().unwrap();
            let _ = conn.execute(
                "DELETE FROM session_list_index WHERE (cli_id = ?1 AND session_path = ?2) \
                 OR (cli_id = ?3 AND session_path = ?4)",
                params![virtual_cli, virtual_path, absent_cli, absent_path],
            );
        }
        assert!(!Path::new(absent_path).exists(), "夹具路径必须确实不存在");

        insert(virtual_cli, virtual_path);
        insert(absent_cli, absent_path);

        purge_orphan_session_index().unwrap();

        assert!(in_index(virtual_cli, virtual_path), "库型会话的索引行被孤儿清理误删");
        assert!(!in_index(absent_cli, absent_path), "文件确实缺失的索引行应被清理");

        // 后置清理
        let conn = conn().unwrap();
        let _ = conn.execute(
            "DELETE FROM session_list_index WHERE cli_id = ?1 AND session_path = ?2",
            params![virtual_cli, virtual_path],
        );
    }

    /// 已登记库型源的存活判定：库中命中的行必须活过孤儿清理，库中确实没有的行必须被清掉。
    ///
    /// 与上一条同源，但方向相反：`OpencodeSource::exists` 会真的打开 SQLite 库查询，
    /// 命中才答「存在」。这正是 OpenCode 用户依赖的行为 —— `exists` 一旦把命中的行误判成
    /// 「不存在」，用户的库型会话会在下次启动时被清掉，且不可恢复。
    ///
    /// 反向的一半同样必要：一个**恒答「存在」**的 `exists` 会让存活断言照样通过，等于没测。
    /// 因此夹具同时放入一条库里不存在的库型行并断言它被清理 —— 只有真的查了库、且把
    /// 「查不到」译成「不存在」，两个断言才会同时成立；库打不开而走了报错臂的 `true`，
    /// 也会被这条断言当场戳穿。
    ///
    /// 库建在临时目录里，经 `set_cli_data_dir_override` 指给 OpenCode；覆盖值测前快照、
    /// 测后恢复，索引行测后清理（套件共用一个落盘应用库）。覆盖是进程级共享状态，
    /// 与读真实库的冒烟测试共用一把锁串行化，避免两条测试互相看到对方的数据目录。
    #[test]
    fn purge_keeps_present_library_session_and_drops_absent() {
        // 与 `discover_matches_real_db_when_present` 共享：二者都要求 OpenCode 数据目录
        // 覆盖在各自临界区内取值稳定，并行会串台。
        let _guard = crate::cli::OPENCODE_DATA_DIR_OVERRIDE_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        let cli = CliKind::Opencode.id();
        let present_id = "ses_guard_opencode_present";
        let present_path = format!("cli://{}/{}", cli, present_id);
        let absent_id = "ses_guard_opencode_absent";
        let absent_path = format!("cli://{}/{}", cli, absent_id);

        // 临时库：只要本测试查询用到的列，schema 与真实库一致。
        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("opencode.db");
        {
            let db = rusqlite::Connection::open(&db_path).unwrap();
            db.execute_batch(
                "CREATE TABLE session (\
                   id TEXT PRIMARY KEY, project_id TEXT NOT NULL, parent_id TEXT,\
                   slug TEXT NOT NULL, directory TEXT NOT NULL, title TEXT NOT NULL,\
                   version TEXT NOT NULL, time_created INTEGER NOT NULL, time_updated INTEGER NOT NULL\
                 );",
            )
            .unwrap();
            db.execute(
                "INSERT INTO session \
                 (id, project_id, slug, directory, title, version, time_created, time_updated) \
                 VALUES (?1, 'proj', 'slug', '/tmp', 'title', '1', 0, 0)",
                params![present_id],
            )
            .unwrap();
        }

        // 先快照覆盖值、再改指向，保证任何一次并发清理看到的都是指向本临时库的覆盖。
        let _override_guard =
            crate::cli::CliDataDirOverrideGuard::set(CliKind::Opencode, temp.path());

        let insert = |path: &str| {
            let conn = conn().unwrap();
            conn.execute(
                "INSERT OR REPLACE INTO session_list_index \
                 (cli_id, session_path, session_id, project_path, title, first_user_message, \
                  first_timestamp, last_timestamp, git_branch, file_size, modified_ms, indexed_at, archived_at) \
                 VALUES (?1, ?2, '', NULL, NULL, NULL, NULL, NULL, '', 0, 0, '', NULL)",
                params![cli, path],
            )
            .unwrap();
        };
        let in_index = |path: &str| -> bool {
            let conn = conn().unwrap();
            conn.query_row(
                "SELECT COUNT(*) FROM session_list_index WHERE cli_id = ?1 AND session_path = ?2",
                params![cli, path],
                |row| row.get::<_, i64>(0),
            )
            .unwrap()
                > 0
        };

        // 前置清理，保证幂等（套件共用一个落盘应用库）。
        {
            let conn = conn().unwrap();
            let _ = conn.execute(
                "DELETE FROM session_list_index WHERE cli_id = ?1 AND session_path IN (?2, ?3)",
                params![cli, present_path, absent_path],
            );
        }
        insert(&present_path);
        insert(&absent_path);

        purge_orphan_session_index().unwrap();

        let present_survived = in_index(&present_path);
        let absent_purged = !in_index(&absent_path);

        // 后置清理：先删索引行
        {
            let conn = conn().unwrap();
            let _ = conn.execute(
                "DELETE FROM session_list_index WHERE cli_id = ?1 AND session_path IN (?2, ?3)",
                params![cli, present_path, absent_path],
            );
        }

        assert!(present_survived, "库中确实存在的库型会话索引行被孤儿清理误删");
        assert!(absent_purged, "库中不存在的库型会话索引行未被孤儿清理");
    }
}
