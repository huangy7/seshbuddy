use crate::error::{AppError, AppResult};
use crate::paths::app_data_dir;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};
use tantivy::collector::TopDocs;
use tantivy::query::{BooleanQuery, Occur, TermQuery};
use tantivy::schema::*;
use tantivy::tokenizer::NgramTokenizer;
use tantivy::{DocAddress, Index, IndexReader, IndexWriter, ReloadPolicy, TantivyDocument, Term};

const NGRAM_MIN: usize = 2;
const NGRAM_MAX: usize = 4;
const NGRAM_TOKENIZER_NAME: &str = "ngram_2_4";
const SEARCH_HEAP_SIZE: usize = 50_000_000; // 50MB

/// Schema field accessors
pub(crate) struct SearchSchema {
    pub cli_id: Field,
    pub session_path: Field,
    pub message_index: Field,
    pub search_text: Field,
    pub modified_ms: Field,
}

/// Global index context — lazily initialized singleton
pub(crate) struct IndexContext {
    pub schema: SearchSchema,
    pub index: Index,
    pub reader: IndexReader,
    pub writer: Arc<Mutex<IndexWriter>>,
    /// Generation captured when this context was created. A borrowed writer
    /// whose generation no longer matches `INDEX_GENERATION` is stale (the
    /// physical index was reset underneath it) and must not commit.
    pub generation: u64,
}

static INDEX_CONTEXT: LazyLock<Mutex<Option<IndexContext>>> = LazyLock::new(|| Mutex::new(None));

/// Bumped by `reset_physical_index` while holding the writer mutex, so any
/// writer Arc borrowed before the reset detects the mismatch and aborts
/// instead of committing pre-reset docs into the recreated directory.
static INDEX_GENERATION: AtomicU64 = AtomicU64::new(0);

fn search_index_dir() -> AppResult<PathBuf> {
    let dir = super::search_index_dir()?;
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

pub(crate) fn is_physical_index_ready() -> AppResult<bool> {
    let index_dir = search_index_dir()?;
    if !index_dir.join("meta.json").exists() {
        return Ok(false);
    }
    let (expected_schema, _) = build_schema();
    Ok(Index::open_in_dir(&index_dir)
        .map(|index| index.schema() == expected_schema)
        .unwrap_or(false))
}

/// Staging 临时构建目录守卫（RAII 机制）
///
/// 业务背景：
/// 在全文搜索索引全量构建或修复过程中，为了避免直接对线上 `search_index/` 目录进行破坏性
/// 清空导致查询服务中断（Zero-Downtime 目标），所有写操作首先在隔离的 staging 目录中进行。
/// 若构建过程中发生任何异常、错误或程序中断，该守卫离开作用域时通过 Drop 自动彻底清场，
/// 绝不留下孤儿垃圾碎片文件；当构建成功后调用 `mark_committed`，目录将被保留用于两阶段原子重命名切换。
pub(crate) struct StagingIndexGuard {
    pub dir: PathBuf,
    committed: bool,
}

/// 在途 Staging 守卫计数。
///
/// 大于 0 即表示有构建正在进行：此时它的 staging 目录可能正被写入，或正处在
/// `target → old` / `staging → target` 两次重命名的切换窗口内。全局清理必须让路 ——
/// 它无法分辨哪些临时目录属于在途构建。
static ACTIVE_STAGING_GUARDS: AtomicUsize = AtomicUsize::new(0);

impl StagingIndexGuard {
    pub fn new(dir: PathBuf) -> Self {
        ACTIVE_STAGING_GUARDS.fetch_add(1, Ordering::SeqCst);
        Self {
            dir,
            committed: false,
        }
    }

    pub fn mark_committed(&mut self) {
        self.committed = true;
    }
}

impl Drop for StagingIndexGuard {
    fn drop(&mut self) {
        ACTIVE_STAGING_GUARDS.fetch_sub(1, Ordering::SeqCst);
        if !self.committed && self.dir.exists() {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }
}

static STAGING_SEQ: AtomicU64 = AtomicU64::new(0);

/// 创建带唯一时间戳和原子序列号的独立 Staging 临时索引目录
pub(crate) fn create_staging_index_dir() -> AppResult<StagingIndexGuard> {
    let parent = app_data_dir()?;
    let timestamp = chrono::Utc::now().timestamp_millis();
    let seq = STAGING_SEQ.fetch_add(1, Ordering::Relaxed);
    let staging_path = parent.join(format!("search_index.staging-{}-{}", timestamp, seq));
    if staging_path.exists() {
        let _ = std::fs::remove_dir_all(&staging_path);
    }
    std::fs::create_dir_all(&staging_path)?;
    Ok(StagingIndexGuard::new(staging_path))
}

/// 执行两阶段目录原子重命名切换（底层支持自定义路径以便隔离测试）
pub(crate) fn swap_staging_dir_inner(
    target_dir: &std::path::Path,
    mut guard: StagingIndexGuard,
) -> AppResult<()> {
    let timestamp = chrono::Utc::now().timestamp_millis();
    let seq = STAGING_SEQ.fetch_add(1, Ordering::Relaxed);
    let old_dir = target_dir
        .parent()
        .unwrap_or(target_dir)
        .join(format!("search_index.old-{}-{}", timestamp, seq));

    if target_dir.exists() {
        std::fs::rename(target_dir, &old_dir)?;
    }

    if let Err(err) = std::fs::rename(&guard.dir, target_dir) {
        if old_dir.exists() && !target_dir.exists() {
            if let Err(rb_err) = std::fs::rename(&old_dir, target_dir) {
                tracing::error!("两阶段切换回滚失败，无法恢复旧索引目录: {:?}", rb_err);
            }
        }
        return Err(err.into());
    }

    guard.mark_committed();

    if old_dir.exists() {
        let _ = std::fs::remove_dir_all(&old_dir);
    }

    Ok(())
}

/// 将构建完成的 Staging 临时索引两阶段原子切换上线，并热重载全局 IndexContext
pub(crate) fn commit_staging_index(guard: StagingIndexGuard) -> AppResult<()> {
    let target_dir = search_index_dir()?;
    let mut ctx_guard = INDEX_CONTEXT
        .lock()
        .map_err(|e| AppError::coded("db.index_context_lock_poisoned").with("detail", e.to_string()))?;

    let writer = ctx_guard.as_ref().map(|ctx| Arc::clone(&ctx.writer));
    let writer_guard = match &writer {
        Some(w) => Some(
            w.lock()
                .map_err(|e| AppError::coded("db.writer_lock_poisoned").with("detail", e.to_string()))?,
        ),
        None => None,
    };

    INDEX_GENERATION.fetch_add(1, Ordering::SeqCst);
    *ctx_guard = None;
    drop(writer_guard);
    drop(writer); // 显式释放 Arc，彻底释放旧 IndexWriter 及其底层文件锁与句柄

    swap_staging_dir_inner(&target_dir, guard)?;

    // 内层错误**原样透出**，不套外层码：`init_index_context` 的每一个错误都是带 `{detail}`
    // 的本地化句子或整句文案，而套一层再 `.with("detail", e.to_string())` 装进去的是
    // `Coded` 的 `Display` = **裸 code** —— 本分支曾经如此，屏幕上就是
    // 「Staging 索引挂载失败: db.tantivy_index_open_failed」，而 tantivy 的原文被丢掉。
    // 形状由 `index_init_call_sites_surface_the_inner_error_unchanged` 钉住。
    let ctx = init_index_context(false)?;
    *ctx_guard = Some(ctx);

    Ok(())
}

/// 清理遗留的 staging / old 临时索引目录（内层实现，接收父目录以便测试）。
///
/// **有构建在途时一律不清理**：本函数是全局 `rm -rf`，无法分辨哪些临时目录属于正在进行的
/// 构建 —— 构建过程会经历 `target → old` 与 `staging → target` 两次重命名，此窗口内误删
/// 正在使用的 staging 会让切换的 rename 失败，而回滚又依赖 old 目录，两头落空之后该索引
/// 彻底不可用。守卫是按 CLI 的、清理是全局的，另一个 CLI 的调用或未受守卫的维护线程都
/// 可能在这个窗口里进来，所以判据必须是全局的「是否有构建在途」。
/// 陈旧目录留到没有构建在途时再清，代价可以忽略。
pub(crate) fn cleanup_stale_staging_dirs_inner(parent: &std::path::Path) -> AppResult<usize> {
    if ACTIVE_STAGING_GUARDS.load(Ordering::SeqCst) > 0 {
        tracing::debug!("索引构建进行中，跳过 staging 临时目录清理");
        return Ok(0);
    }

    let mut removed = 0usize;
    if let Ok(entries) = std::fs::read_dir(parent) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.starts_with("search_index.staging-")
                || name_str.starts_with("search_index.old-")
            {
                let path = entry.path();
                if path.is_dir() {
                    let _ = std::fs::remove_dir_all(&path);
                    removed += 1;
                }
            }
        }
    }
    Ok(removed)
}

pub(crate) fn cleanup_stale_staging_dirs() -> AppResult<()> {
    // staging 目录与切换目标同在 app_data_dir 下（见 create_staging_index_dir），
    // 扫描父目录必须与创建处一致，否则清不到任何残留
    cleanup_stale_staging_dirs_inner(&app_data_dir()?)?;
    Ok(())
}

pub(crate) fn reset_physical_index() -> AppResult<()> {
    let staging = init_staging_index()?;
    staging.commit()
}

fn build_schema() -> (Schema, SearchSchema) {
    let mut builder = Schema::builder();

    let cli_id = builder.add_text_field("cli_id", STRING | STORED);
    let session_path = builder.add_text_field("session_path", STRING | STORED);
    let message_index = builder.add_u64_field("message_index", INDEXED | STORED);

    // search_text uses ngram tokenizer for substring matching
    let text_options = TextOptions::default()
        .set_indexing_options(
            TextFieldIndexing::default()
                .set_tokenizer(NGRAM_TOKENIZER_NAME)
                .set_index_option(IndexRecordOption::WithFreqsAndPositions),
        )
        .set_stored();
    let search_text = builder.add_text_field("search_text", text_options);

    let modified_ms = builder.add_u64_field("modified_ms", STORED);

    let schema_fields = SearchSchema {
        cli_id,
        session_path,
        message_index,
        search_text,
        modified_ms,
    };

    (builder.build(), schema_fields)
}

/// 打开（`create_if_missing` 时创建）物理索引并组装 `IndexContext`。
///
/// **本函数产出的每一个错误都是用户可读的整句**（`errors.db.*`，四语各一份）：带 `{detail}`
/// 的那些装的是第三方原文（tantivy 的英文错误文本，R3 不翻译），不带 `{detail}` 的
/// （`db.tantivy_schema_mismatch` / `db.tantivy_index_missing`）本身就是完整句子。
/// 故**调用点必须原样透出它**：套一层 `.with("detail", e.to_string())` 会把 `Coded` 的
/// `Display`（**裸 code**）塞进 `detail`，屏幕上就成了「…: db.tantivy_index_open_failed」，
/// 而内层真正的 tantivy 原文被永久丢弃（终审在 `commit_staging_index` 与
/// `get_index_context_inner` 两处实测到这个形态，已删）。
fn init_index_context(create_if_missing: bool) -> AppResult<IndexContext> {
    init_index_context_inner(&search_index_dir()?, create_if_missing)
}

/// 同 [`init_index_context`]，只是索引目录由调用方给。
///
/// 开这个接缝的理由：`search_index_dir()` 落到真实 app data 目录，单测里驱动不了
/// （会碰用户的真实索引），故目录从外面传进来 —— 与同文件的 `swap_staging_dir_inner` /
/// `cleanup_stale_staging_dirs_inner` 同一惯例。
fn init_index_context_inner(
    index_dir: &std::path::Path,
    create_if_missing: bool,
) -> AppResult<IndexContext> {
    let (schema, schema_fields) = build_schema();

    // Open or create index. Existing broken indexes must be repaired by the
    // explicit rebuild path so SQLite sync state can be invalidated together.
    let index = if index_dir.join("meta.json").exists() {
        match Index::open_in_dir(index_dir) {
            Ok(idx) if idx.schema() == schema => idx,
            Ok(_) => return Err(AppError::coded("db.tantivy_schema_mismatch")),
            Err(e) => {
                return Err(
                    AppError::coded("db.tantivy_index_open_failed").with("detail", e.to_string())
                )
            }
        }
    } else if create_if_missing {
        Index::create_in_dir(index_dir, schema.clone())?
    } else {
        return Err(AppError::coded("db.tantivy_index_missing"));
    };

    // Register ngram tokenizer
    index.tokenizers().register(
        NGRAM_TOKENIZER_NAME,
        NgramTokenizer::new(NGRAM_MIN, NGRAM_MAX, false)
            .map_err(|e| AppError::coded("db.ngram_tokenizer_create_failed").with("detail", format!("{:?}", e)))?,
    );

    let reader = index
        .reader_builder()
        .reload_policy(ReloadPolicy::OnCommitWithDelay)
        .try_into()
        .map_err(|e| AppError::coded("db.index_reader_create_failed").with("detail", e.to_string()))?;

    let writer = index.writer(SEARCH_HEAP_SIZE)?;

    Ok(IndexContext {
        schema: schema_fields,
        index,
        reader,
        writer: Arc::new(Mutex::new(writer)),
        generation: INDEX_GENERATION.load(Ordering::SeqCst),
    })
}

/// Open the global index context without creating a missing physical index.
pub(crate) fn get_index_context() -> AppResult<MutexGuard<'static, Option<IndexContext>>> {
    get_index_context_inner(false)
}

fn get_or_create_index_context() -> AppResult<MutexGuard<'static, Option<IndexContext>>> {
    get_index_context_inner(true)
}

fn get_index_context_inner(
    create_if_missing: bool,
) -> AppResult<MutexGuard<'static, Option<IndexContext>>> {
    let mut guard = INDEX_CONTEXT
        .lock()
        .map_err(|e| AppError::coded("db.index_context_lock_poisoned").with("detail", e.to_string()))?;

    if guard.is_none() {
        // 同上：内层错误原样透出，不套外层码（理由见 `init_index_context` 的文档）。
        let ctx = init_index_context(create_if_missing)?;
        *guard = Some(ctx);
    }

    Ok(guard)
}

/// Add documents for a session (delete-then-insert pattern).
pub(crate) fn write_session_docs(
    cli_id: &str,
    session_path: &str,
    modified_ms: u64,
    docs: &[(usize, String)], // (message_index, search_text)
) -> AppResult<()> {
    let ctx_guard = get_or_create_index_context()?;
    let ctx = ctx_guard
        .as_ref()
        .ok_or_else(|| AppError::coded("db.index_context_unavailable"))?;
    let mut writer = ctx
        .writer
        .lock()
        .map_err(|e| AppError::coded("db.writer_lock_poisoned").with("detail", e.to_string()))?;

    // Delete existing docs for this session_path
    let path_term = Term::from_field_text(ctx.schema.session_path, session_path);
    writer.delete_term(path_term);

    // Add new docs. On failure, roll back the writer buffer so the queued
    // delete + partial adds are not flushed by a later unrelated commit
    // (this also discards earlier successfully-queued batches — safe because
    // re-indexing is delete-then-insert and therefore idempotent).
    for (msg_idx, text) in docs {
        let mut doc = TantivyDocument::default();
        doc.add_text(ctx.schema.cli_id, cli_id);
        doc.add_text(ctx.schema.session_path, session_path);
        doc.add_u64(ctx.schema.message_index, *msg_idx as u64);
        doc.add_text(ctx.schema.search_text, text);
        doc.add_u64(ctx.schema.modified_ms, modified_ms);
        if let Err(e) = writer.add_document(doc) {
            let _ = writer.rollback();
            return Err(e.into());
        }
    }

    Ok(())
}

/// Discard any queued (uncommitted) writes in the shared writer buffer.
///
/// Best-effort helper for error paths: callers that fail between queueing
/// docs and committing them invoke this so the leftover delete+partial adds
/// cannot be flushed by a later unrelated `commit_writes` /
/// `delete_session_docs`. Lock/rollback failures are ignored because the
/// caller is already propagating a primary error.
pub(crate) fn rollback_writes() {
    let writer = match INDEX_CONTEXT.lock() {
        Ok(guard) => guard.as_ref().map(|ctx| Arc::clone(&ctx.writer)),
        Err(_) => None,
    };
    if let Some(writer) = writer {
        if let Ok(mut writer) = writer.lock() {
            let _ = writer.rollback();
        }
    }
}

/// Commit pending writes to the index.
pub(crate) fn commit_writes() -> AppResult<()> {
    // Clone the writer Arc and reader, then DROP the global context guard so
    // concurrent searches are not blocked for the duration of writer.commit()
    // (which can take seconds on large rebuilds). The single-writer invariant
    // is preserved: the writer itself stays behind its own Mutex, and cloning
    // the Arc never creates a second IndexWriter. IndexReader is an Arc
    // wrapper, so reload() on the clone reloads the shared reader state.
    let (writer, reader, generation) = {
        let ctx_guard = get_or_create_index_context()?;
        let ctx = ctx_guard
            .as_ref()
            .ok_or_else(|| AppError::coded("db.index_context_unavailable"))?;
        (
            Arc::clone(&ctx.writer),
            ctx.reader.clone(),
            ctx.generation,
        )
    };
    let mut writer = writer
        .lock()
        .map_err(|e| AppError::coded("db.writer_lock_poisoned").with("detail", e.to_string()))?;
    let current_generation = INDEX_GENERATION.load(Ordering::SeqCst);
    if generation != current_generation {
        // The physical index was reset after this writer was borrowed:
        // discard the buffer instead of committing pre-reset docs into the
        // recreated directory. Returning an error (instead of a silent Ok)
        // prevents rebuild callers from persisting SQLite index-state rows
        // for documents that were never committed — which would cause
        // permanent silent search misses.
        tracing::warn!(
            "搜索索引: commit 检测到代际不匹配，丢弃缓冲写入 (borrowed_gen={}, current_gen={})",
            generation,
            current_generation
        );
        let _ = writer.rollback();
        return Err(AppError::coded("db.generation_mismatch")
            .with("from", generation.to_string())
            .with("to", current_generation.to_string()));
    }
    if let Err(e) = writer.commit() {
        // A failed commit can leave the buffer intact; discard it so partial
        // writes are not flushed by a later unrelated commit.
        let _ = writer.rollback();
        return Err(e.into());
    }
    drop(writer);
    reader.reload()?;
    Ok(())
}

/// Delete all documents for given session paths.
pub(crate) fn delete_session_docs(session_paths: &[String]) -> AppResult<()> {
    if !is_physical_index_ready()? {
        return Ok(());
    }

    // Same pattern as commit_writes: release the global context guard before
    // the (potentially slow) writer.commit() so searches stay responsive.
    let (writer, reader, session_path_field, generation) = {
        let ctx_guard = get_index_context()?;
        let ctx = ctx_guard
            .as_ref()
            .ok_or_else(|| AppError::coded("db.index_context_unavailable"))?;
        (
            Arc::clone(&ctx.writer),
            ctx.reader.clone(),
            ctx.schema.session_path,
            ctx.generation,
        )
    };
    let mut writer = writer
        .lock()
        .map_err(|e| AppError::coded("db.writer_lock_poisoned").with("detail", e.to_string()))?;
    let current_generation = INDEX_GENERATION.load(Ordering::SeqCst);
    if generation != current_generation {
        // Stale writer (index was reset after borrowing): nothing to delete
        // in the recreated directory. Keeping Ok here is the safe direction
        // (pre-reset docs are gone with the reset), but log it for
        // observability.
        tracing::warn!(
            "搜索索引: delete 检测到代际不匹配，跳过删除 (borrowed_gen={}, current_gen={})",
            generation,
            current_generation
        );
        let _ = writer.rollback();
        return Ok(());
    }

    for path in session_paths {
        let term = Term::from_field_text(session_path_field, path);
        writer.delete_term(term);
    }
    if let Err(e) = writer.commit() {
        let _ = writer.rollback();
        return Err(e.into());
    }
    drop(writer);
    reader.reload()?;
    Ok(())
}

/// 返回物理索引中所有已索引的 (cli_id, session_path) 去重集合。
/// 供孤儿清理使用：逐一核对每个路径的源文件与归档内容是否仍然存在。
pub(crate) fn all_indexed_session_paths() -> AppResult<Vec<(String, String)>> {
    if !is_physical_index_ready()? {
        return Ok(Vec::new());
    }
    let ctx_guard = get_index_context()?;
    let ctx = ctx_guard
        .as_ref()
        .ok_or_else(|| AppError::coded("db.index_context_unavailable"))?;
    let searcher = ctx.reader.searcher();
    let schema = &ctx.schema;
    let mut seen: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();
    for (ord, segment_reader) in searcher.segment_readers().iter().enumerate() {
        for doc_id in segment_reader.doc_ids_alive() {
            let addr = DocAddress::new(ord as u32, doc_id);
            if let Ok(doc) = searcher.doc::<TantivyDocument>(addr) {
                let cli = doc
                    .get_first(schema.cli_id)
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let path = doc
                    .get_first(schema.session_path)
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                if !cli.is_empty() && !path.is_empty() {
                    seen.insert((cli, path));
                }
            }
        }
    }
    Ok(seen.into_iter().collect())
}

/// Search result from Tantivy
#[derive(Debug, Clone)]
pub(crate) struct TantivySearchHit {
    pub session_path: String,
    pub message_index: u64,
    pub search_text: String,
    pub score: f32,
}

/// Search for sessions matching the query.
/// Uses PhraseQuery via quoted string to correctly match ngram substrings.
pub(crate) fn search_sessions(
    cli_id: &str,
    query: &str,
    limit: usize,
) -> AppResult<Vec<TantivySearchHit>> {
    let ctx_guard = get_index_context()?;
    let ctx = ctx_guard
        .as_ref()
        .ok_or_else(|| AppError::coded("db.index_context_unavailable"))?;
    let searcher = ctx.reader.searcher();

    // Build query: cli_id filter AND phrase query on search_text
    let cli_term = Term::from_field_text(ctx.schema.cli_id, cli_id);
    let cli_query = TermQuery::new(cli_term, IndexRecordOption::Basic);

    // For search_text: wrap in quotes to create a PhraseQuery
    // This ensures ngram tokens are matched in order (position-aware)
    // Escape backslashes to prevent breaking the quoted phrase; preserve user's double quotes
    let safe_query = query.replace('\\', "\\\\").replace('"', "\\\"");
    let phrase_str = format!("\"{}\"", safe_query);

    let query_parser =
        tantivy::query::QueryParser::for_index(&ctx.index, vec![ctx.schema.search_text]);
    let text_query = query_parser
        .parse_query(&phrase_str)
        .map_err(|e| AppError::coded("db.query_parse_failed").with("detail", e.to_string()))?;

    let combined = BooleanQuery::new(vec![
        (Occur::Must, Box::new(cli_query)),
        (Occur::Must, text_query),
    ]);

    let top_docs = searcher.search(&combined, &TopDocs::with_limit(limit * 10))?;

    let mut hits = Vec::new();
    for (score, doc_addr) in top_docs {
        let doc: TantivyDocument = searcher.doc(doc_addr)?;

        let session_path = doc
            .get_first(ctx.schema.session_path)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let message_index = doc
            .get_first(ctx.schema.message_index)
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let search_text = doc
            .get_first(ctx.schema.search_text)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        hits.push(TantivySearchHit {
            session_path,
            message_index,
            search_text,
            score,
        });
    }

    Ok(hits)
}

/// Search and return matched docs for specific session paths.
pub(crate) fn search_docs_for_paths(
    cli_id: &str,
    query: &str,
    session_paths: &[String],
) -> AppResult<Vec<TantivySearchHit>> {
    if session_paths.is_empty() || query.trim().is_empty() {
        return Ok(Vec::new());
    }

    let ctx_guard = get_index_context()?;
    let ctx = ctx_guard
        .as_ref()
        .ok_or_else(|| AppError::coded("db.index_context_unavailable"))?;
    let searcher = ctx.reader.searcher();

    // Build cli_id filter
    let cli_term = Term::from_field_text(ctx.schema.cli_id, cli_id);
    let cli_query = TermQuery::new(cli_term, IndexRecordOption::Basic);

    // Build session_path OR filter
    let path_queries: Vec<(Occur, Box<dyn tantivy::query::Query>)> = session_paths
        .iter()
        .map(|p| {
            let term = Term::from_field_text(ctx.schema.session_path, p);
            (
                Occur::Should,
                Box::new(TermQuery::new(term, IndexRecordOption::Basic))
                    as Box<dyn tantivy::query::Query>,
            )
        })
        .collect();
    let paths_query = BooleanQuery::new(path_queries);

    // Build text phrase query
    let safe_query = query.replace('\\', "\\\\").replace('"', "\\\"");
    let phrase_str = format!("\"{}\"", safe_query);
    let query_parser =
        tantivy::query::QueryParser::for_index(&ctx.index, vec![ctx.schema.search_text]);
    let text_query = query_parser
        .parse_query(&phrase_str)
        .map_err(|e| AppError::coded("db.query_parse_failed").with("detail", e.to_string()))?;

    let combined = BooleanQuery::new(vec![
        (Occur::Must, Box::new(cli_query)),
        (Occur::Must, Box::new(paths_query)),
        (Occur::Must, text_query),
    ]);

    let top_docs = searcher.search(&combined, &TopDocs::with_limit(10_000))?;

    let mut hits = Vec::new();
    for (score, doc_addr) in top_docs {
        let doc: TantivyDocument = searcher.doc(doc_addr)?;

        let session_path = doc
            .get_first(ctx.schema.session_path)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let message_index = doc
            .get_first(ctx.schema.message_index)
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let search_text = doc
            .get_first(ctx.schema.search_text)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        hits.push(TantivySearchHit {
            session_path,
            message_index,
            search_text,
            score,
        });
    }

    Ok(hits)
}

/// 独立 Staging 索引构建器
///
/// 用于在独立的临时 staging 目录中批量构建全新的 Tantivy 索引，
/// 构建期间不占用全局写入器，不影响线上只读查询；
/// 完成后调用 `commit()` 自动完成底层提交、刷盘与两阶段原子目录切换。
#[allow(dead_code)]
pub(crate) struct StagingIndexWriter {
    pub writer: Option<IndexWriter>,
    pub index: Index,
    pub schema: SearchSchema,
    pub guard: Option<StagingIndexGuard>,
}

impl Drop for StagingIndexWriter {
    fn drop(&mut self) {
        // 先释放 writer 及其底层持有的文件锁与工作线程，
        // 随后释放 guard 执行未提交清理。
        drop(self.writer.take());
        drop(self.guard.take());
    }
}

#[allow(dead_code)]
impl StagingIndexWriter {
    pub fn write_session_docs(
        &mut self,
        cli_id: &str,
        session_path: &str,
        modified_ms: u64,
        docs: &[(usize, String)],
    ) -> AppResult<()> {
        let writer = self
            .writer
            .as_mut()
            .ok_or_else(|| AppError::coded("db.staging_writer_taken"))?;
        let path_term = Term::from_field_text(self.schema.session_path, session_path);
        writer.delete_term(path_term);

        for (msg_idx, text) in docs {
            let mut doc = TantivyDocument::default();
            doc.add_text(self.schema.cli_id, cli_id);
            doc.add_text(self.schema.session_path, session_path);
            doc.add_u64(self.schema.message_index, *msg_idx as u64);
            doc.add_text(self.schema.search_text, text);
            doc.add_u64(self.schema.modified_ms, modified_ms);
            writer.add_document(doc)?;
        }
        Ok(())
    }

    pub fn commit(mut self) -> AppResult<()> {
        if let Some(mut writer) = self.writer.take() {
            writer.commit()?;
            drop(writer);
        }
        let guard = self
            .guard
            .take()
            .ok_or_else(|| AppError::coded("db.staging_guard_taken"))?;
        commit_staging_index(guard)
    }
}

#[allow(dead_code)]
pub(crate) fn init_staging_index() -> AppResult<StagingIndexWriter> {
    let guard = create_staging_index_dir()?;
    let (schema, schema_fields) = build_schema();
    let index = Index::create_in_dir(&guard.dir, schema.clone())?;
    index.tokenizers().register(
        NGRAM_TOKENIZER_NAME,
        NgramTokenizer::new(NGRAM_MIN, NGRAM_MAX, false)
            .map_err(|e| AppError::coded("db.ngram_tokenizer_create_failed").with("detail", format!("{:?}", e)))?,
    );
    let writer = index.writer(SEARCH_HEAP_SIZE)?;

    Ok(StagingIndexWriter {
        writer: Some(writer),
        index,
        schema: schema_fields,
        guard: Some(guard),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use regex::Regex;

    /// ★ 搜索索引初始化的失败必须是**用户能读的整句**：不是裸 code，也不会渲染成「…: 」。
    ///
    /// 判据的来路：这些错误会跨 IPC 上屏 —— 用户点「重建索引」时，
    /// `ensure_search_index_ready` → `reset_physical_index` → `commit_staging_index` 这条链
    /// 走到的就是它们，前端渲染 `errors.db.*` 的句子。曾经两个调用点写成
    /// `.map_err(|e| AppError::coded("db.index_init_failed").with("detail", e.to_string()))`，
    /// 而 `e` 是 `AppError`、`to_string()` 是它的 `Display` = **裸 code**，
    /// 于是屏幕上出现「…: db.tantivy_index_open_failed」，内层 tantivy 的原文被永久丢弃
    /// （终审实测）。外层包装已删，故这里钉住内层错误本身的形状。
    ///
    /// ⚠️ 这条钉的是**错误本身的形状**，钉不到调用点会不会再套一层 ——
    /// 那一半由 `index_init_call_sites_surface_the_inner_error_unchanged` 钉。
    #[test]
    fn index_init_errors_are_readable_sentences() {
        let dir = tempfile::tempdir().unwrap();
        let index_dir = dir.path().join("search_index");
        std::fs::create_dir_all(&index_dir).unwrap();

        // ① 没有 meta.json 且不创建：整句文案，没有 `{detail}` 位可留空。
        // `.err().expect(...)` 而非 `.expect_err(...)`：`IndexContext` 没实现 `Debug`。
        let missing = init_index_context_inner(&index_dir, false)
            .err()
            .expect("缺索引必须报错");
        let value = serde_json::to_value(&missing).unwrap();
        assert_eq!(value["code"], "db.tantivy_index_missing");
        assert_eq!(
            value["params"],
            serde_json::json!({}),
            "该码没有 {{detail}} 位：外层若拿它拼「…: {{detail}}」就会只剩一个冒号"
        );

        // ② meta.json 在但不是索引：detail 是 tantivy 原文，非空，且不是我们自己的 code。
        std::fs::write(index_dir.join("meta.json"), b"not a tantivy index").unwrap();
        let open_failed = init_index_context_inner(&index_dir, false)
            .err()
            .expect("坏索引必须报错");
        let value = serde_json::to_value(&open_failed).unwrap();
        assert_eq!(value["code"], "db.tantivy_index_open_failed");
        let detail = value["params"]["detail"].as_str().expect("必须带 detail");
        assert!(!detail.is_empty(), "detail 不得为空");
        assert!(
            !detail.starts_with("db.") && !detail.starts_with("internal."),
            "detail 必须是第三方原文，不能是我们自己的 code：{detail}"
        );
    }

    /// 调用点的形状：`init_index_context` 的结果必须**原样透出**（只跟一个 `?`）。
    ///
    /// 上一条钉的是内层错误本身的形状，**钉不到调用点**：把 `commit_staging_index` 或
    /// `get_index_context_inner` 改回 `.map_err(|e| AppError::coded("db.index_init_failed")…`
    /// ——编译得过、上一条也绿（它只驱动内层函数），而用户屏幕上又会出现
    /// 「…: db.tantivy_index_open_failed」。闸门只挡「码没有语言包条目」那一半，
    /// 把键加回去就绿了。故形状本身也要钉 —— 判据与阳性对照的做法同
    /// `tray::tests::tray_rebuild_call_sites_route_through_the_shared_attribution`。
    ///
    /// ⚠️ **看不到的**：真实路径（`ensure_search_index_ready` → `reset_physical_index` →
    /// `commit_staging_index`）要真实 app data 目录下的索引，单测里不驱动；
    /// 判据对形状敏感（调用被 `;` 拆行、或中间插了别的调用会误报）；
    /// 只剥行首注释（行尾注释与块注释里的同形串仍会命中）。
    #[test]
    fn index_init_call_sites_surface_the_inner_error_unchanged() {
        let routed = Regex::new(r"init_index_context\([^;)]*\)\?").expect("正则");
        let wrapped = Regex::new(r"init_index_context\([^;)]*\)\.map_err").expect("正则");

        // 阳性对照：被变异过的形状必须报得出，合法形状报不出。
        assert!(wrapped.is_match(
            r#"init_index_context(create_if_missing).map_err(|e| AppError::coded("db.index_init_failed").with("detail", e.to_string()))?"#
        ));
        assert!(!wrapped.is_match("init_index_context(false)?"));
        assert!(routed.is_match("init_index_context(false)?"));

        // 只扫生产段：上面的阳性对照字面量本身就在测试段里，扫全文件会自我命中。
        let production: String = include_str!("tantivy_search.rs")
            .lines()
            .take_while(|line| !line.starts_with("#[cfg(test)]"))
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<String>()
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();

        assert_eq!(
            wrapped.find_iter(&production).count(),
            0,
            "init_index_context 的调用点又套了一层 map_err：内层 Coded 会被压成裸 code 塞进 detail"
        );
        assert_eq!(
            routed.find_iter(&production).count(),
            2,
            "两个调用点都必须原样透出 init_index_context 的错误"
        );
    }

    #[test]
    fn test_staging_guard_cleans_on_drop_when_uncommitted() {
        let dir = tempfile::tempdir().unwrap();
        let staging_path = dir.path().join("search_index.build-test");
        std::fs::create_dir_all(&staging_path).unwrap();

        {
            let _guard = StagingIndexGuard::new(staging_path.clone());
            assert!(staging_path.exists());
        }

        assert!(!staging_path.exists(), "未提交的 staging 目录在离开作用域时必须被自动清理");
    }

    /// 有构建在途时，全局 staging 清理必须让路：清除正在使用的临时目录会让两阶段切换的
    /// rename 失败，而回滚又依赖同时被清掉的 old 目录，两头落空后索引彻底不可用。
    #[test]
    fn test_cleanup_stale_staging_dirs_skips_while_build_in_flight() {
        let dir = tempfile::tempdir().unwrap();
        let staging_path = dir.path().join("search_index.staging-stale");
        let old_path = dir.path().join("search_index.old-stale");
        let unrelated = dir.path().join("search_index");
        for path in [&staging_path, &old_path, &unrelated] {
            std::fs::create_dir_all(path).unwrap();
        }

        {
            let _live = StagingIndexGuard::new(dir.path().join("search_index.staging-live"));
            let removed = cleanup_stale_staging_dirs_inner(dir.path()).unwrap();
            assert_eq!(removed, 0, "构建在途时必须跳过清理");
            assert!(staging_path.exists(), "在途构建期间不得删除 staging 目录");
            assert!(old_path.exists(), "在途构建期间不得删除 old 目录（回滚依赖它）");
        }
    }

    #[test]
    fn test_staging_guard_preserves_dir_when_committed() {
        let dir = tempfile::tempdir().unwrap();
        let staging_path = dir.path().join("search_index.build-test-keep");
        std::fs::create_dir_all(&staging_path).unwrap();

        {
            let mut guard = StagingIndexGuard::new(staging_path.clone());
            assert!(staging_path.exists());
            guard.mark_committed();
        }

        assert!(staging_path.exists(), "已提交的 staging 目录必须被保留");
    }

    #[test]
    fn test_staging_index_two_phase_swap() {
        let dir = tempfile::tempdir().unwrap();
        let target_dir = dir.path().join("search_index");
        std::fs::create_dir_all(&target_dir).unwrap();
        std::fs::write(target_dir.join("old.txt"), b"old content").unwrap();

        let staging_dir = dir.path().join("search_index.staging-12345");
        std::fs::create_dir_all(&staging_dir).unwrap();
        std::fs::write(staging_dir.join("new.txt"), b"new content").unwrap();

        let guard = StagingIndexGuard::new(staging_dir.clone());
        swap_staging_dir_inner(&target_dir, guard).unwrap();

        assert!(target_dir.join("new.txt").exists(), "新内容必须出现在目标目录");
        assert!(!target_dir.join("old.txt").exists(), "旧内容必须已被替换");
        assert!(!staging_dir.exists(), "staging 目录已重命名，不应存在");
    }

    #[test]
    fn test_staging_index_writer_construction_and_drop() {
        let dir = tempfile::tempdir().unwrap();
        let staging_path = dir.path().join("search_index.staging-test-writer");
        std::fs::create_dir_all(&staging_path).unwrap();

        let guard = StagingIndexGuard::new(staging_path.clone());
        let (schema, schema_fields) = build_schema();
        let index = Index::create_in_dir(&guard.dir, schema).unwrap();
        index.tokenizers().register(
            NGRAM_TOKENIZER_NAME,
            NgramTokenizer::new(NGRAM_MIN, NGRAM_MAX, false).unwrap(),
        );
        let writer = index.writer(SEARCH_HEAP_SIZE).unwrap();

        let mut staging_writer = StagingIndexWriter {
            writer: Some(writer),
            index,
            schema: schema_fields,
            guard: Some(guard),
        };

        // 写入文档
        staging_writer
            .write_session_docs(
                "claude",
                "/path/to/test.jsonl",
                1000,
                &[(0, "测试文本内容 hello world".to_string())],
            )
            .unwrap();

        // 故意不 commit，直接 drop 验证 RAII 清场
        drop(staging_writer);
        assert!(!staging_path.exists(), "未提交的 staging 索引目录应在 drop 后完全清理");
    }
}
