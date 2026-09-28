//! 回填 JobManager：预热投影 + 驱动提示条数字。
//! 防重入 + 4 并发 + 显式取消；进度事件载荷保持旧契约。

use crate::error::AppResult;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::Emitter;
use transcript_store::extract::extract;
use transcript_store::store::{self, CacheStatus, ExtractPlan, SessionCacheRecord};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GapKind {
    Fresh,    // 已新鲜或终态，跳过
    Extract,  // 需要提取
    MarkGone, // 源文件已删，标记终态
}

/// 缺口分类（纯函数，可单测）。sig = (mtime_ms, size)，None = 源文件不存在。
pub fn classify(record: Option<&SessionCacheRecord>, sig: Option<(i64, i64)>) -> GapKind {
    match (record, sig) {
        (Some(r), _) if r.status != CacheStatus::Ok => GapKind::Fresh, // empty/gone 终态
        (_, None) => match record {
            Some(r) if r.status == CacheStatus::Gone => GapKind::Fresh,
            _ => GapKind::MarkGone,
        },
        (None, Some(_)) => GapKind::Extract,
        (Some(r), Some((mtime_ms, size))) => {
            match store::decide_plan(Some(r), mtime_ms, size) {
                ExtractPlan::Skip => GapKind::Fresh,
                _ => GapKind::Extract,
            }
        }
    }
}

pub struct GapItem {
    pub cli: String,
    pub session_id: String,
    pub session_path: String,
}

fn cache_db_path() -> AppResult<std::path::PathBuf> {
    let dir = crate::commands::assistant_dir()?;
    std::fs::create_dir_all(&dir).map_err(|e| crate::error::AppError::coded("assistant.dir_create_failed").with("detail", e.to_string()))?;
    Ok(dir.join("transcript_cache.db"))
}

/// 源文件签名状态：区分「已删」与「瞬时 IO 错误」。
/// gone 终态不可逆，只有确凿的 NotFound 才允许落 gone；瞬时错误本轮跳过、下轮重试。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SigState {
    Present(i64, i64),
    NotFound,
    IoError,
}

/// metadata 错误分类（纯函数，可单测）：只有 NotFound 才算"文件已删"。
pub fn classify_io_error(kind: std::io::ErrorKind) -> SigState {
    match kind {
        std::io::ErrorKind::NotFound => SigState::NotFound,
        _ => SigState::IoError,
    }
}

fn signature(path: &str) -> SigState {
    match std::fs::metadata(path) {
        Ok(m) => match m
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        {
            Some(d) => SigState::Present(d.as_millis() as i64, m.len() as i64),
            None => SigState::IoError,
        },
        Err(e) => classify_io_error(e.kind()),
    }
}

/// 收集全部 CLI 的提取缺口（gone 标记在收集阶段直接落库）。
pub fn collect_gaps() -> AppResult<Vec<GapItem>> {
    let mut conn = store::open(&cache_db_path()?)?;
    let records = store::all_records(&conn)?;
    let mut gaps = Vec::new();
    for kind in [crate::cli::CliKind::Claude, crate::cli::CliKind::Codex, crate::cli::CliKind::Gemini] {
        let index = crate::app_db::read_session_list_index(kind)?;
        for (path, item) in &index {
            let rec = records.get(&item.session_id);
            match signature(path) {
                SigState::IoError => {
                    // 瞬时 IO 错误（EACCES/挂载抖动等）：不落库，本轮跳过，下轮自然重试
                    tracing::warn!("签名探测 IO 错误，跳过本轮: path={}", path);
                }
                SigState::NotFound => {
                    if classify(rec, None) == GapKind::MarkGone {
                        if let Err(e) = store::mark_status(&mut conn, &item.session_id, kind.id(), CacheStatus::Gone, 0, 0) {
                            tracing::warn!("gone 标记失败: id={}, err={}", item.session_id, e);
                        }
                    }
                }
                SigState::Present(mtime_ms, size) => {
                    if classify(rec, Some((mtime_ms, size))) == GapKind::Extract {
                        gaps.push(GapItem {
                            cli: kind.id().to_string(),
                            session_id: item.session_id.clone(),
                            session_path: path.clone(),
                        });
                    }
                }
            }
        }
    }
    Ok(gaps)
}

static RUNNING: AtomicBool = AtomicBool::new(false);
static CANCEL: AtomicBool = AtomicBool::new(false);

/// 4 并发线程池回填；每会话独立 db 连接（WAL 下写串行化由 busy_timeout 兜底）。
pub fn run(app: tauri::AppHandle, gaps: Vec<GapItem>) {
    let total = gaps.len();
    let queue = Arc::new(Mutex::new(std::collections::VecDeque::from(gaps)));
    let counters = Arc::new(Mutex::new((0usize, 0usize, 0usize))); // (done, written, skipped)
    let mut handles = Vec::new();
    for _ in 0..4.min(total.max(1)) {
        let queue = Arc::clone(&queue);
        let counters = Arc::clone(&counters);
        let app = app.clone();
        handles.push(std::thread::spawn(move || loop {
            if CANCEL.load(Ordering::SeqCst) {
                return;
            }
            let item = queue.lock().unwrap_or_else(|e| e.into_inner()).pop_front();
            let Some(item) = item else { return };
            let result = extract_one(&item);
            let mut c = counters.lock().unwrap_or_else(|e| e.into_inner());
            c.0 += 1;
            match result {
                Ok(true) => c.1 += 1,
                Ok(false) => c.2 += 1,
                Err(e) => {
                    c.2 += 1;
                    // `path` 必须显式打：`e` 现在是 `AppError::Coded`，`Display` 就是裸 code，
                    // 那条错误里承载路径的 `params.path` 不会跟着打出来；而同一个会话 id
                    // 会在多轮里反复失败，路径才是能区分这次失败落在哪个文件上的字段。
                    tracing::warn!(
                        "回填提取失败: id={}, path={}, err={}",
                        item.session_id,
                        item.session_path,
                        e.diagnostic()
                    );
                }
            }
            let _ = app.emit(
                "assistant-backfill-progress",
                serde_json::json!({
                    "current": c.0, "total": total, "written": c.1, "skipped": c.2,
                }),
            );
        }));
    }
    for h in handles {
        let _ = h.join();
    }
    let c = counters.lock().unwrap_or_else(|e| e.into_inner());
    let _ = app.emit(
        "assistant-backfill-progress",
        serde_json::json!({
            "current": c.0, "total": total, "written": c.1, "skipped": c.2, "done": true,
        }),
    );
}

/// 提取单个会话。返回 Ok(false) = 无内容（empty 终态）。
fn extract_one(item: &GapItem) -> AppResult<bool> {
    let (mtime_ms, size) = match signature(&item.session_path) {
        SigState::Present(mtime_ms, size) => (mtime_ms, size),
        SigState::NotFound => {
            let mut conn = store::open(&cache_db_path()?)?;
            store::mark_status(&mut conn, &item.session_id, &item.cli, CacheStatus::Gone, 0, 0)?;
            return Ok(false);
        }
        SigState::IoError => {
            // 瞬时 IO 错误：不写任何状态（gone 终态不可逆），报错让本轮计 skipped，下轮重试
            return Err(crate::error::AppError::coded("assistant.signature_probe_io_error")
                .with("path", item.session_path.as_str()));
        }
    };
    let mut conn = store::open(&cache_db_path()?)?;
    let record = store::get_record(&conn, &item.session_id)?;
    let plan = store::decide_plan(record.as_ref(), mtime_ms, size);
    if plan == ExtractPlan::Skip {
        return Ok(true);
    }
    let (skip, base) = match plan {
        ExtractPlan::Append { skip_raw_lines, base_seq } => (skip_raw_lines, base_seq),
        _ => (0, 0),
    };
    let file = std::fs::File::open(&item.session_path)?;
    let format = transcript_store::extract::CliFormat::from_cli_id(&item.cli);
    let out = extract(std::io::BufReader::new(file), format, skip, base);
    let base_count = match plan {
        ExtractPlan::Append { .. } => record.map(|r| r.message_count).unwrap_or(0),
        _ => 0,
    };
    let total = base_count + out.messages.len() as i64;
    let status = if total == 0 { CacheStatus::Empty } else { CacheStatus::Ok };
    let rec = SessionCacheRecord {
        session_id: item.session_id.clone(),
        cli: item.cli.clone(),
        mtime_ms, size,
        raw_lines: out.raw_lines as i64,
        last_seq: out.message_lines as i64,
        status,
        message_count: total,
        extracted_at: 0,
    };
    match plan {
        ExtractPlan::Append { .. } if status == CacheStatus::Ok => {
            store::append_session(&mut conn, &rec, &out.messages)?;
        }
        _ => store::replace_session(&mut conn, &rec, &out.messages)?,
    }
    Ok(status == CacheStatus::Ok)
}

pub fn start(app: tauri::AppHandle) {
    if RUNNING.swap(true, Ordering::SeqCst) {
        return; // 防重入：进行中再次触发直接挂到同一轮（进度事件照流）
    }
    CANCEL.store(false, Ordering::SeqCst);
    std::thread::spawn(move || {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            match collect_gaps() {
                Ok(gaps) => run(app.clone(), gaps),
                Err(e) => {
                    // 与 `assistant-event` 同一条判据：`e` 是 `AppError`，不能 `to_string()`——
                    // `Coded` 的 `Display` 是裸 code，压成字符串后前端会原样上屏。
                    let _ = app.emit(
                        "assistant-backfill-progress",
                        serde_json::json!({ "done": true, "error": e }),
                    );
                }
            }
        }));
        RUNNING.store(false, Ordering::SeqCst);
        if let Err(e) = result {
            tracing::warn!("cache 回填任务 panic: {:?}", e);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use transcript_store::store::{CacheStatus, SessionCacheRecord};

    fn rec(session_id: &str, status: CacheStatus, mtime_ms: i64, size: i64) -> SessionCacheRecord {
        SessionCacheRecord {
            session_id: session_id.into(), cli: "claude".into(),
            mtime_ms, size, raw_lines: 0, last_seq: 0,
            status, message_count: 0, extracted_at: 0,
        }
    }

    #[test]
    fn classify_gap_matrix() {
        // 无记录 + 文件在 → Extract
        assert_eq!(classify(None, Some((100, 50))), GapKind::Extract);
        // 无记录 + 文件无 → MarkGone
        assert_eq!(classify(None, None), GapKind::MarkGone);
        // ok + 签名一致 → Fresh（跳过）
        assert_eq!(classify(Some(&rec("a", CacheStatus::Ok, 100, 50)), Some((100, 50))), GapKind::Fresh);
        // ok + 签名变 → Extract
        assert_eq!(classify(Some(&rec("a", CacheStatus::Ok, 100, 50)), Some((200, 60))), GapKind::Extract);
        // empty/gone 终态 → 永远跳过（即使签名变了也不重提——gone 语义是源已删）
        assert_eq!(classify(Some(&rec("a", CacheStatus::Empty, 100, 50)), Some((200, 60))), GapKind::Fresh);
        assert_eq!(classify(Some(&rec("a", CacheStatus::Gone, 0, 0)), None), GapKind::Fresh);
        // ok 记录但文件已删 → MarkGone
        assert_eq!(classify(Some(&rec("a", CacheStatus::Ok, 100, 50)), None), GapKind::MarkGone);
    }

    #[test]
    fn io_error_kind_only_notfound_means_deleted() {
        // 非 NotFound 的 IO 错误必须归为 IoError（不落 gone 终态，下轮重试）
        assert_eq!(classify_io_error(std::io::ErrorKind::PermissionDenied), SigState::IoError);
        assert_eq!(classify_io_error(std::io::ErrorKind::NotADirectory), SigState::IoError);
        assert_eq!(classify_io_error(std::io::ErrorKind::Other), SigState::IoError);
        assert_eq!(classify_io_error(std::io::ErrorKind::NotFound), SigState::NotFound);
    }

    #[test]
    fn signature_distinguishes_missing_present_and_io_error() {
        let dir = tempfile::tempdir().unwrap();
        // 已删 → NotFound（唯一允许落 gone 的情形）
        let missing = dir.path().join("nope.jsonl");
        assert_eq!(signature(missing.to_str().unwrap()), SigState::NotFound);
        // 存在 → Present
        let present = dir.path().join("a.jsonl");
        std::fs::write(&present, "{}").unwrap();
        assert!(matches!(signature(present.to_str().unwrap()), SigState::Present(_, _)));
        // 把文件当目录用（ENOTDIR）：非 NotFound → IoError，绝不误判为已删
        let not_dir = present.join("child");
        assert_eq!(signature(not_dir.to_str().unwrap()), SigState::IoError);
    }
}
