use super::conn;
use crate::cli::CliKind;
use crate::error::AppResult;
use rusqlite::params;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub(crate) struct SessionSearchIndexRecord {
    pub session_path: String,
    pub modified_ms: i64,
    #[allow(dead_code)]
    pub doc_count: usize,
}

#[derive(Debug, Clone)]
pub(crate) struct SessionSearchDocRecord {
    pub message_index: usize,
    pub search_text: String,
}

#[derive(Debug, Clone)]
pub(crate) struct SessionSearchDocBatch {
    pub session_path: String,
    pub modified_ms: i64,
    pub docs: Vec<SessionSearchDocRecord>,
}

#[derive(Debug, Clone)]
pub(crate) struct SessionSearchWriteProgress {
    pub phase: &'static str,
    pub written_sessions: usize,
    pub current_path: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct SessionSearchIndexStateUpdate {
    pub session_path: String,
    pub modified_ms: i64,
    pub doc_count: usize,
}

#[derive(Debug, Clone)]
pub(crate) struct SessionSearchCandidate {
    pub session_path: String,
    pub first_match_message_index: Option<usize>,
    pub matched_doc_count: usize,
    pub rank: f64,
}

#[derive(Debug, Clone)]
pub(crate) struct SessionSearchMatchedDoc {
    pub session_path: String,
    pub message_index: usize,
    /// 参与匹配与切片展示的文本。**元数据文档里放的是字段值本身**，
    /// 不再带 `会话标题: ` 一类前缀——前缀会经 `extract_snippet` 上屏。
    pub search_text: String,
    /// 该文档命中的字段；内容文档为 `Some(Content)`（不是 `None`：`None` 只留给无从分类的文档）。
    pub matched_field: Option<crate::session::MatchedField>,
}

fn map_session_search_index_record(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<SessionSearchIndexRecord> {
    Ok(SessionSearchIndexRecord {
        session_path: row.get(0)?,
        modified_ms: row.get(1)?,
        doc_count: row.get::<_, i64>(2)?.max(0) as usize,
    })
}

pub(crate) fn count_session_search_indexed_docs(kind: CliKind) -> AppResult<usize> {
    let conn = conn()?;
    let count: i64 = conn.query_row(
        "SELECT COALESCE(SUM(doc_count), 0) FROM session_search_index_state WHERE cli_id = ?1",
        params![kind.id()],
        |row| row.get(0),
    )?;
    Ok(count.max(0) as usize)
}

pub(crate) fn read_session_search_index(
    kind: CliKind,
) -> AppResult<HashMap<String, SessionSearchIndexRecord>> {
    let conn = conn()?;
    let mut stmt = conn.prepare(
        r#"
            SELECT session_path, modified_ms, doc_count
            FROM session_search_index_state
            WHERE cli_id = ?1
            "#,
    )?;
    let rows = stmt.query_map(params![kind.id()], map_session_search_index_record)?;

    let mut result = HashMap::new();
    for row in rows {
        let record = row?;
        result.insert(record.session_path.clone(), record);
    }
    Ok(result)
}

pub(crate) fn clear_all_session_search_index_state() -> AppResult<()> {
    let conn = conn()?;
    conn.execute("DELETE FROM session_search_index_state", [])?;
    Ok(())
}

pub(crate) fn replace_session_search_docs_with_progress<F>(
    kind: CliKind,
    batches: &[SessionSearchDocBatch],
    mut on_progress: F,
) -> AppResult<()>
where
    F: FnMut(SessionSearchWriteProgress),
{
    if batches.is_empty() {
        return Ok(());
    }

    let total_sessions = batches.len();

    // Phase 1: Tantivy writes + commit WITHOUT any SQLite transaction open.
    // ngram(2-4) tokenization and writer.commit() dominate the wall-clock time;
    // keeping them outside a SQLite transaction shrinks the database write-lock
    // window from seconds to milliseconds.
    //
    // On ANY error before the commit lands, roll back the writer buffer so the
    // queued delete+partial adds are not flushed by a later unrelated commit.
    let tantivy_stage = |on_progress: &mut F| -> AppResult<()> {
        for (idx, batch) in batches.iter().enumerate() {
            // Write to Tantivy index
            let tantivy_docs: Vec<(usize, String)> = batch
                .docs
                .iter()
                .map(|d| (d.message_index, d.search_text.clone()))
                .collect();
            super::tantivy_search::write_session_docs(
                kind.id(),
                &batch.session_path,
                batch.modified_ms as u64,
                &tantivy_docs,
            )?;

            on_progress(SessionSearchWriteProgress {
                phase: "writing",
                written_sessions: idx + 1,
                current_path: Some(batch.session_path.clone()),
            });
        }

        on_progress(SessionSearchWriteProgress {
            phase: "committing",
            written_sessions: total_sessions,
            current_path: None,
        });

        // Commit Tantivy changes FIRST. This ordering makes the dangerous
        // failure direction (SQLite claims indexed but Tantivy lacks the docs)
        // impossible: we only record state for docs that are already committed.
        super::tantivy_search::commit_writes()?;
        Ok(())
    };
    if let Err(err) = tantivy_stage(&mut on_progress) {
        super::tantivy_search::rollback_writes();
        return Err(err);
    }

    // Phase 2: bookkeeping rows in a short SQLite transaction AFTER the
    // Tantivy commit succeeded. If this fails, the sessions simply get
    // re-indexed on the next run (write_session_docs is delete-then-insert,
    // so re-indexing is idempotent) — a safe failure direction.
    let mut conn = conn()?;
    let tx = conn.transaction()?;
    let now = super::now_rfc3339();
    for batch in batches {
        tx.execute(
            r#"
            INSERT OR REPLACE INTO session_search_index_state (
                cli_id,
                session_path,
                modified_ms,
                doc_count,
                indexed_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5)
            "#,
            params![
                kind.id(),
                &batch.session_path,
                batch.modified_ms,
                batch.docs.len() as i64,
                &now,
            ],
        )?;
    }
    tx.commit()?;
    let now_ms = chrono::Utc::now().timestamp_millis();
    let _ = super::write_index_last_updated(&conn, kind.id(), now_ms);

    on_progress(SessionSearchWriteProgress {
        phase: "committed",
        written_sessions: total_sessions,
        current_path: None,
    });
    Ok(())
}

pub(crate) fn write_session_search_docs_uncommitted(
    kind: CliKind,
    batch: &SessionSearchDocBatch,
) -> AppResult<SessionSearchIndexStateUpdate> {
    let tantivy_docs: Vec<(usize, String)> = batch
        .docs
        .iter()
        .map(|d| (d.message_index, d.search_text.clone()))
        .collect();
    super::tantivy_search::write_session_docs(
        kind.id(),
        &batch.session_path,
        batch.modified_ms as u64,
        &tantivy_docs,
    )?;

    Ok(SessionSearchIndexStateUpdate {
        session_path: batch.session_path.clone(),
        modified_ms: batch.modified_ms,
        doc_count: batch.docs.len(),
    })
}

pub(crate) fn commit_session_search_writes() -> AppResult<()> {
    super::tantivy_search::commit_writes()
}

/// Number of `session_search_index_state` rows written per SQLite transaction.
/// Chunking keeps the write-lock window small during large rebuilds (a 10k
/// session rebuild no longer holds the lock in one giant transaction). On
/// failure, earlier chunks stay committed — acceptable because unrecorded
/// sessions are simply re-indexed on the next run (Tantivy writes are
/// delete-then-insert, hence idempotent).
const STATE_WRITE_CHUNK_SIZE: usize = 500;

pub(crate) fn replace_session_search_index_state(
    kind: CliKind,
    updates: &[SessionSearchIndexStateUpdate],
) -> AppResult<()> {
    if updates.is_empty() {
        return Ok(());
    }

    let mut conn = conn()?;
    let now = super::now_rfc3339();

    for chunk in updates.chunks(STATE_WRITE_CHUNK_SIZE) {
        let tx = conn.transaction()?;
        for update in chunk {
            tx.execute(
                r#"
            INSERT OR REPLACE INTO session_search_index_state (
                cli_id,
                session_path,
                modified_ms,
                doc_count,
                indexed_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5)
            "#,
                params![
                    kind.id(),
                    &update.session_path,
                    update.modified_ms,
                    update.doc_count as i64,
                    &now,
                ],
            )?;
        }
        tx.commit()?;
    }

    Ok(())
}

pub(crate) fn delete_session_search_paths(
    kind: CliKind,
    session_paths: &[String],
) -> AppResult<()> {
    if session_paths.is_empty() {
        return Ok(());
    }

    // Delete from Tantivy
    super::tantivy_search::delete_session_docs(session_paths)?;

    // Delete from SQLite
    let mut conn = conn()?;
    let tx = conn.transaction()?;

    for session_path in session_paths {
        tx.execute(
            "DELETE FROM session_search_index_state WHERE cli_id = ?1 AND session_path = ?2",
            params![kind.id(), session_path],
        )?;
    }

    tx.commit()?;

    Ok(())
}

pub(crate) fn delete_session_records(kind: CliKind, session_paths: &[String]) -> AppResult<()> {
    if session_paths.is_empty() {
        return Ok(());
    }

    // Delete from Tantivy search index first. Best-effort: record deletion
    // must still proceed on failure, but log so stale search hits are
    // diagnosable instead of silently lingering.
    if let Err(err) = super::tantivy_search::delete_session_docs(session_paths) {
        tracing::warn!(
            "删除会话搜索索引文档失败（记录仍将被删除）: cli={}, paths={:?}, err={}",
            kind.id(),
            session_paths,
            err.diagnostic()
        );
    }

    let mut conn = conn()?;
    let tx = conn.transaction()?;

    for session_path in session_paths {
        // 双轨墓碑防御：将待删除会话登记至 session_tombstones 墓碑表，
        // 绑定 (cli_id, session_id) 逻辑主键与物理文件路径，防止全量扫描或并发延迟导致已删会话复活
        let db_session_id: Option<String> = tx
            .query_row(
                "SELECT session_id FROM session_list_index WHERE cli_id = ?1 AND session_path = ?2",
                params![kind.id(), session_path],
                |r| r.get(0),
            )
            .ok();
        // 推导规则与归档恢复/重建端共用同一实现，避免两侧不一致导致墓碑拦不住复活
        let safe_session_id =
            super::session_tombstones::tombstone_session_id(kind, session_path, db_session_id.as_deref());
        super::session_tombstones::record_tombstone_inner(&tx, kind.id(), &safe_session_id, session_path)?;

        tx.execute(
            "DELETE FROM archived_session_content WHERE cli_id = ?1 AND session_path = ?2",
            params![kind.id(), session_path],
        )?;
        tx.execute(
            "DELETE FROM session_list_index WHERE cli_id = ?1 AND session_path = ?2",
            params![kind.id(), session_path],
        )?;
        tx.execute(
            "DELETE FROM session_search_index_state WHERE cli_id = ?1 AND session_path = ?2",
            params![kind.id(), session_path],
        )?;
        tx.execute(
            // 自定义名按 `<cli>\u{0}<路径>` 复合键存，漏删会让同一路径日后被重新索引
            //（从废纸篓恢复、上游重建同名文件）时，陈旧的旧名字悄悄重新贴到会话上。
            "DELETE FROM session_names WHERE session_path = ?1",
            params![super::session_index::session_name_key(kind, session_path)],
        )?;
    }

    tx.commit()?;

    Ok(())
}

/// 清理纯搜索孤儿：物理搜索索引中存在、但源文件与归档内容都不存在的会话路径。
///
/// 这类条目会被全局搜索命中，但打开必失败（load_session_stream 报"会话文件不存在"），
/// 且增量重建（collect_missing_search_paths 基于会话列表索引）无法发现它们，
/// 需要主动遍历物理索引逐一核对。`cli_id` 为 None 时清理全部 CLI，否则仅清理指定 CLI。
/// 返回清理的路径数。
pub(crate) fn purge_search_orphans(cli_id: Option<&str>) -> AppResult<usize> {
    let _ = super::tantivy_search::cleanup_stale_staging_dirs();
    let indexed = super::tantivy_search::all_indexed_session_paths()?;
    let mut orphans: Vec<String> = Vec::new();
    let mut pairs: Vec<(String, String)> = Vec::new();
    for (cli, path) in indexed {
        if let Some(filter) = cli_id {
            if cli != filter {
                continue;
            }
        }
        if std::path::Path::new(&path).exists() {
            continue;
        }
        match super::read_archived_session_content(&cli, &path) {
            // 归档内容可用 → 可从归档打开，保留
            Ok(Some(_)) => continue,
            // 归档不可用 → 孤儿；读取失败保守保留，避免误删
            Ok(None) => {}
            Err(_) => continue,
        }
        orphans.push(path.clone());
        pairs.push((cli, path));
    }
    if orphans.is_empty() {
        return Ok(0);
    }
    // tantivy 物理文档删除（best-effort，失败不阻断 SQLite 状态清理）
    if let Err(err) = super::tantivy_search::delete_session_docs(&orphans) {
        tracing::warn!("清理搜索孤儿文档失败: err={}", err.diagnostic());
    }
    // SQLite 搜索状态同步删除
    let mut conn = conn()?;
    let tx = conn.transaction()?;
    for (cli, path) in &pairs {
        tx.execute(
            "DELETE FROM session_search_index_state WHERE cli_id = ?1 AND session_path = ?2",
            params![cli, path],
        )?;
    }
    tx.commit()?;
    tracing::info!("搜索索引: 清理纯搜索孤儿 {} 个", orphans.len());
    Ok(orphans.len())
}

pub(crate) fn search_session_candidates(
    kind: CliKind,
    query: &str,
    limit: usize,
) -> AppResult<Vec<SessionSearchCandidate>> {
    let hits = super::tantivy_search::search_sessions(kind.id(), query, limit * 10)?;

    // Group and aggregate hits by session_path, since tantivy returns document-level hits
    let mut grouping: HashMap<String, (usize, usize, f64)> = HashMap::new(); // (first_match_msg_idx, matched_doc_count, max_score)

    for hit in hits {
        let entry = grouping
            .entry(hit.session_path)
            .or_insert((usize::MAX, 0, 0.0));
        if (hit.message_index as usize) < entry.0 {
            entry.0 = hit.message_index as usize;
        }
        entry.1 += 1;
        if (hit.score as f64) > entry.2 {
            entry.2 = hit.score as f64;
        }
    }

    let mut candidates: Vec<SessionSearchCandidate> = grouping
        .into_iter()
        .map(|(path, (first_msg, count, score))| SessionSearchCandidate {
            session_path: path,
            first_match_message_index: Some(first_msg),
            matched_doc_count: count,
            rank: score,
        })
        .collect();

    // Sort by matched_doc_count DESC, first_match_message_index ASC
    candidates.sort_by(|a, b| {
        b.matched_doc_count.cmp(&a.matched_doc_count).then_with(|| {
            a.first_match_message_index
                .cmp(&b.first_match_message_index)
        })
    });

    candidates.truncate(limit);
    Ok(candidates)
}

pub(crate) fn read_session_search_docs_for_paths(
    kind: CliKind,
    query: &str,
    session_paths: &[String],
) -> AppResult<Vec<SessionSearchMatchedDoc>> {
    let hits = super::tantivy_search::search_docs_for_paths(kind.id(), query, session_paths)?;

    let mut result: Vec<SessionSearchMatchedDoc> = hits
        .into_iter()
        .map(|hit| SessionSearchMatchedDoc {
            session_path: hit.session_path,
            message_index: hit.message_index as usize,
            search_text: hit.search_text,
            matched_field: Some(crate::session::MatchedField::Content),
        })
        .collect();

    // Sort by session_path ASC, message_index ASC
    result.sort_by(|a, b| {
        a.session_path
            .cmp(&b.session_path)
            .then_with(|| a.message_index.cmp(&b.message_index))
    });

    Ok(result)
}

pub(crate) fn read_session_search_doc_text(
    kind: CliKind,
    session_path: &str,
    message_index: usize,
) -> AppResult<Option<String>> {
    // Look up doc directly from SQLite using the list index fallback if needed
    // Actually, in the FTS implementation this was queried directly from FTS tables.
    // Let's get it from the Tantivy index via a specific term query on session_path and msg_index.
    let ctx_guard = super::tantivy_search::get_index_context()?;
    let ctx = ctx_guard
        .as_ref()
        .ok_or_else(|| crate::error::AppError::coded("db.index_context_unavailable"))?;
    let searcher = ctx.reader.searcher();

    use tantivy::query::{BooleanQuery, Occur, TermQuery};
    use tantivy::schema::{IndexRecordOption, Value};
    use tantivy::Term;

    let c_term = Term::from_field_text(ctx.schema.cli_id, kind.id());
    let c_query = Box::new(TermQuery::new(c_term, IndexRecordOption::Basic));

    let p_term = Term::from_field_text(ctx.schema.session_path, session_path);
    let p_query = Box::new(TermQuery::new(p_term, IndexRecordOption::Basic));

    let m_term = Term::from_field_u64(ctx.schema.message_index, message_index as u64);
    let m_query = Box::new(TermQuery::new(m_term, IndexRecordOption::Basic));

    let combined = BooleanQuery::new(vec![
        (Occur::Must, c_query),
        (Occur::Must, p_query),
        (Occur::Must, m_query),
    ]);

    let top_docs = searcher.search(&combined, &tantivy::collector::TopDocs::with_limit(1))?;

    if let Some((_, doc_addr)) = top_docs.first() {
        let doc: tantivy::TantivyDocument = searcher.doc(*doc_addr)?;
        if let Some(val) = doc
            .get_first(ctx.schema.search_text)
            .and_then(|v| v.as_str())
        {
            return Ok(Some(val.to_string()));
        }
    }

    Ok(None)
}

#[cfg(test)]
mod orphan_tests {
    use super::*;

    #[test]
    fn test_purge_removes_orphan_docs() {
        let fake = "/tmp/seshbuddy-nonexistent-orphan-12345.jsonl".to_string();
        // 应用运行时 tantivy 索引被独占（LockBusy），跳过而非误报
        if crate::db::tantivy_search::write_session_docs("claude", &fake, 1, &[(0, "hello world orphan".to_string())]).is_err() {
            println!("搜索索引被应用占用，跳过孤儿清理测试");
            return;
        }
        if crate::db::tantivy_search::commit_writes().is_err() {
            println!("搜索索引被应用占用，跳过孤儿清理测试");
            return;
        }

        let before: Vec<(String, String)> = crate::db::tantivy_search::all_indexed_session_paths().unwrap()
            .into_iter().filter(|(_, p)| *p == fake).collect();
        assert_eq!(before.len(), 1, "孤儿应已写入索引");

        purge_search_orphans(Some("claude")).unwrap();

        let after: Vec<(String, String)> = crate::db::tantivy_search::all_indexed_session_paths().unwrap()
            .into_iter().filter(|(_, p)| *p == fake).collect();
        assert_eq!(after.len(), 0, "孤儿应已被清理");
    }

    #[test]
    fn test_delete_session_records_creates_tombstone() {
        let fake_path = "/tmp/test-delete-records-tombstone.jsonl".to_string();
        let fake_id = "test-delete-records-tombstone";
        let agy_path = "/tmp/brain/agy-conv-uuid-1234/transcript.jsonl".to_string();
        let dsh_path = "/tmp/dsh/proj/dsh-sess-uuid-5678/session.jsonl.zstd".to_string();
        let generic_path_1 = "/tmp/unknown/session.jsonl".to_string();
        let generic_path_2 = "/tmp/unknown/transcript.jsonl".to_string();

        let conn = conn().unwrap();
        // 前置清理，确保测试幂等
        let _ = crate::db::clear_session_tombstone_inner(&conn, "claude", fake_id);
        let _ = crate::db::clear_session_tombstone_inner(&conn, "antigravity", "agy-conv-uuid-1234");
        let _ = crate::db::clear_session_tombstone_inner(&conn, "dsh", "dsh-sess-uuid-5678");

        delete_session_records(CliKind::Claude, &[fake_path.clone()]).unwrap();
        delete_session_records(CliKind::Antigravity, &[agy_path.clone()]).unwrap();
        delete_session_records(CliKind::Dsh, &[dsh_path.clone()]).unwrap();
        delete_session_records(CliKind::Claude, &[generic_path_1.clone(), generic_path_2.clone()]).unwrap();

        // 1. 常规会话验证
        assert!(crate::db::is_tombstoned_inner(&conn, "claude", fake_id, &fake_path).unwrap());
        // 2. Antigravity: 从路径自动提取真实 conversation_id，绝非 "transcript"
        assert!(crate::db::is_tombstoned_inner(&conn, "antigravity", "agy-conv-uuid-1234", &agy_path).unwrap());
        assert!(!crate::db::is_tombstoned_inner(&conn, "antigravity", "transcript", "/other/transcript.jsonl").unwrap());
        // 3. Dsh: 从父目录自动提取会话 ID，绝非 "session" 或 "session.jsonl"
        assert!(crate::db::is_tombstoned_inner(&conn, "dsh", "dsh-sess-uuid-5678", &dsh_path).unwrap());
        assert!(!crate::db::is_tombstoned_inner(&conn, "dsh", "session", "/other/session.jsonl").unwrap());
        // 4. 无法提取 ID 的通用路径会话：合成键防冲突，两条记录共存绝不相互覆盖
        assert!(crate::db::is_tombstoned_inner(&conn, "claude", "", &generic_path_1).unwrap());
        assert!(crate::db::is_tombstoned_inner(&conn, "claude", "", &generic_path_2).unwrap());

        // 后置清理
        let _ = crate::db::clear_session_tombstone_inner(&conn, "claude", fake_id);
        let _ = crate::db::clear_session_tombstone_inner(&conn, "antigravity", "agy-conv-uuid-1234");
        let _ = crate::db::clear_session_tombstone_inner(&conn, "dsh", "dsh-sess-uuid-5678");
    }

    /// 删除会话必须清掉其自定义名，且不得误删其它会话。
    /// 漏删会让同一路径日后被重新索引时，陈旧的旧名字重新贴回会话上。
    #[test]
    fn test_delete_session_records_clears_composite_session_name() {
        let path = "/tmp/test-delete-composite-name/sess.jsonl".to_string();
        let composite = crate::app_db::session_name_key(CliKind::Claude, &path);
        let survivor = "/tmp/test-delete-composite-name/other.jsonl".to_string();
        let survivor_composite = crate::app_db::session_name_key(CliKind::Claude, &survivor);

        {
            let conn = conn().unwrap();
            for (key, name) in [
                (composite.clone(), "待删会话名"),
                (survivor_composite.clone(), "无关会话名"),
            ] {
                conn.execute(
                    "INSERT OR REPLACE INTO session_names (session_path, display_name, updated_at) VALUES (?1, ?2, '')",
                    params![key, name],
                )
                .unwrap();
            }
        }

        delete_session_records(CliKind::Claude, &[path.clone()]).unwrap();

        // 复合键含 NUL，而 SQLite 的 LIKE 遇到 NUL 会截断（列值对 LIKE 只可见到分隔符前），
        // 因此不能在 SQL 里按 LIKE 过滤，取回后在 Rust 侧筛选
        let remaining: Vec<String> = {
            let conn = conn().unwrap();
            let mut stmt = conn.prepare("SELECT session_path FROM session_names").unwrap();
            stmt.query_map([], |row| row.get::<_, String>(0))
                .unwrap()
                .filter_map(Result::ok)
                .filter(|key| key.contains("test-delete-composite-name"))
                .collect()
        };
        {
            let conn = conn().unwrap();
            let _ = conn.execute(
                "DELETE FROM session_names WHERE session_path = ?1 OR session_path = ?2",
                params![composite, survivor_composite],
            );
        }

        assert_eq!(
            remaining,
            vec![survivor_composite],
            "应清掉待删会话的自定义名，且不得误删其它会话"
        );
    }
}
