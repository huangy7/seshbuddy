use crate::error::{AppError, AppResult};
use notify::{recommended_watcher, Event, RecursiveMode, Watcher};
use serde::Serialize;
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

// ─── Serialized response types ──────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct GitStatusEntry {
    pub path: String,
    /// "M" | "A" | "D" | "U" | "R" | "?" (untracked)
    pub status: String,
    pub staged: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct CommitInfo {
    pub hash: String,
    pub short_hash: String,
    pub message: String,
    pub author: String,
    pub email: String,
    pub timestamp: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct BranchInfo {
    pub name: String,
    pub is_current: bool,
    pub is_remote: bool,
    pub upstream: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiffFileContent {
    pub original: String,
    pub modified: String,
    pub too_large: bool,
}

// ─── Git watcher (Tauri managed state) ─────────────────────────────

pub struct GitWatcherEntry {
    pub stop_flag: Arc<AtomicBool>,
    #[allow(dead_code)]
    pub watcher: Box<dyn Watcher + Send>,
}

pub type GitWatcherState = Mutex<HashMap<String, GitWatcherEntry>>;

static GIT_WATCHER_STOP_FLAGS: LazyLock<Mutex<Vec<Arc<AtomicBool>>>> =
    LazyLock::new(|| Mutex::new(Vec::new()));

pub fn cleanup_watchers() {
    if let Ok(flags) = GIT_WATCHER_STOP_FLAGS.lock() {
        for flag in flags.iter() {
            flag.store(true, Ordering::SeqCst);
        }
    }
}

// ─── Helper: open repo in spawn_blocking ────────────────────────────

/// 打开仓库。底层 `git2` 文本是英文 / OS 文案，原样进 `detail`（R3）。
fn open_repo(path: &str) -> AppResult<git2::Repository> {
    git2::Repository::discover(path)
        .map_err(|e| AppError::coded("git.repo_open_failed").with("detail", e.to_string()))
}

fn status_char(s: git2::Status) -> &'static str {
    if s.contains(git2::Status::IGNORED) { return "I"; }
    if s.contains(git2::Status::INDEX_NEW) || s.contains(git2::Status::WT_NEW) { return "A"; }
    if s.contains(git2::Status::INDEX_DELETED) || s.contains(git2::Status::WT_DELETED) { return "D"; }
    if s.contains(git2::Status::INDEX_RENAMED) || s.contains(git2::Status::WT_RENAMED) { return "R"; }
    if s.contains(git2::Status::INDEX_MODIFIED) || s.contains(git2::Status::WT_MODIFIED) { return "M"; }
    "?"
}

// ─── Commands ───────────────────────────────────────────────────────

#[tauri::command]
pub async fn git_status(
    path: String,
    include_ignored: Option<bool>,
) -> AppResult<Vec<GitStatusEntry>> {
    tauri::async_runtime::spawn_blocking(move || {
        let repo = open_repo(&path)?;
        let mut opts = git2::StatusOptions::new();
        opts.include_untracked(true)
            .recurse_untracked_dirs(true);
        if include_ignored.unwrap_or(false) {
            opts.include_ignored(true);
        } else {
            opts.include_ignored(false);
        }
        let statuses = repo
            .statuses(Some(&mut opts))
            .map_err(|e| AppError::coded("git.status_failed").with("detail", e.to_string()))?;

        let mut entries = Vec::new();
        for entry in statuses.iter() {
            let file_path = entry.path().unwrap_or("").to_string();
            let s = entry.status();
            let staged = s.intersects(
                git2::Status::INDEX_NEW
                    | git2::Status::INDEX_MODIFIED
                    | git2::Status::INDEX_DELETED
                    | git2::Status::INDEX_RENAMED,
            );
            entries.push(GitStatusEntry {
                path: file_path,
                status: status_char(s).to_string(),
                staged,
            });
        }
        Ok(entries)
    })
    .await
    .map_err(|e| AppError::coded("git.spawn_blocking_failed").with("detail", e.to_string()))?
}

#[tauri::command]
pub async fn git_log(
    path: String,
    file: Option<String>,
    limit: Option<usize>,
) -> AppResult<Vec<CommitInfo>> {
    tauri::async_runtime::spawn_blocking(move || {
        let repo = open_repo(&path)?;
        let mut revwalk = repo
            .revwalk()
            .map_err(|e| AppError::coded("git.revwalk_failed").with("detail", e.to_string()))?;
        revwalk
            .push_head()
            .map_err(|e| AppError::coded("git.push_head_failed").with("detail", e.to_string()))?;
        revwalk
            .set_sorting(git2::Sort::TIME)
            .map_err(|e| AppError::coded("git.sort_failed").with("detail", e.to_string()))?;

        let limit = limit.unwrap_or(100);
        let mut commits = Vec::new();

        for oid_result in revwalk {
            if commits.len() >= limit { break; }
            let oid = oid_result
                .map_err(|e| AppError::coded("git.revwalk_next_failed").with("detail", e.to_string()))?;
            let commit = repo
                .find_commit(oid)
                .map_err(|e| AppError::coded("git.find_commit_failed").with("detail", e.to_string()))?;

            // If filtering by file, check if this commit touches it
            if let Some(ref file_path) = file {
                let dominated = commit_touches_file(&repo, &commit, file_path);
                if !dominated { continue; }
            }

            let author = commit.author();
            commits.push(CommitInfo {
                hash: oid.to_string(),
                short_hash: oid.to_string()[..7.min(oid.to_string().len())].to_string(),
                message: commit.message().unwrap_or("").to_string(),
                author: author.name().unwrap_or("").to_string(),
                email: author.email().unwrap_or("").to_string(),
                timestamp: author.when().seconds(),
            });
        }
        Ok(commits)
    })
    .await
    .map_err(|e| AppError::coded("git.spawn_blocking_failed").with("detail", e.to_string()))?
}

fn commit_touches_file(repo: &git2::Repository, commit: &git2::Commit, file_path: &str) -> bool {
    let tree = match commit.tree() {
        Ok(t) => t,
        Err(_) => return false,
    };
    let parent_tree = commit.parent(0).ok().and_then(|p| p.tree().ok());
    let diff = repo.diff_tree_to_tree(
        parent_tree.as_ref(),
        Some(&tree),
        Some(git2::DiffOptions::new().pathspec(file_path)),
    );
    match diff {
        Ok(d) => d.deltas().count() > 0,
        Err(_) => false,
    }
}

#[tauri::command]
pub async fn git_branches(path: String) -> AppResult<Vec<BranchInfo>> {
    tauri::async_runtime::spawn_blocking(move || {
        let repo = open_repo(&path)?;
        let branches = repo
            .branches(None)
            .map_err(|e| AppError::coded("git.branches_failed").with("detail", e.to_string()))?;
        let head = repo.head().ok();
        let head_name = head.as_ref().and_then(|h| h.shorthand().map(|s| s.to_string()));

        let mut result = Vec::new();
        for branch_result in branches {
            let (branch, branch_type) = branch_result
                .map_err(|e| AppError::coded("git.branches_iter_failed").with("detail", e.to_string()))?;
            let name = branch.name().ok().flatten().unwrap_or("").to_string();
            let is_remote = branch_type == git2::BranchType::Remote;
            let is_current = !is_remote && head_name.as_deref() == Some(&name);
            let upstream = branch.upstream().ok().and_then(|u| u.name().ok().flatten().map(|s| s.to_string()));
            result.push(BranchInfo { name, is_current, is_remote, upstream });
        }
        Ok(result)
    })
    .await
    .map_err(|e| AppError::coded("git.spawn_blocking_failed").with("detail", e.to_string()))?
}

#[tauri::command]
pub async fn git_is_repo(path: String) -> AppResult<bool> {
    tauri::async_runtime::spawn_blocking(move || {
        Ok(git2::Repository::discover(&path).is_ok())
    })
    .await
    .map_err(|e| AppError::coded("git.spawn_blocking_failed").with("detail", e.to_string()))?
}

// ─── Phase 2: Diff / Stage / Commit commands ──────────────────────────

const MAX_DIFF_SIZE: usize = 1024 * 1024; // 1MB

#[derive(Debug, Clone, Serialize)]
pub struct CommitDiffFile {
    pub path: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CommitDiffResult {
    pub info: CommitInfo,
    pub files: Vec<CommitDiffFile>,
}

#[tauri::command]
pub async fn git_diff_file_content(path: String, file: String) -> AppResult<DiffFileContent> {
    tauri::async_runtime::spawn_blocking(move || {
        let repo = open_repo(&path)?;
        let workdir = repo
            .workdir()
            .ok_or_else(|| AppError::coded("git.workdir_missing"))?;
        let abs_file = workdir.join(&file);

        // Read working tree version
        let modified = std::fs::read_to_string(&abs_file).map_err(|e| {
            AppError::coded("git.worktree_file_read_failed").with("detail", e.to_string())
        })?;
        if modified.len() > MAX_DIFF_SIZE {
            return Ok(DiffFileContent { original: String::new(), modified: String::new(), too_large: true });
        }

        // Read HEAD version
        let head = repo
            .head()
            .map_err(|e| AppError::coded("git.head_read_failed").with("detail", e.to_string()))?;
        let tree = head
            .peel_to_tree()
            .map_err(|e| AppError::coded("git.tree_parse_failed").with("detail", e.to_string()))?;
        let entry = tree.get_path(Path::new(&file));
        let original = match entry {
            Ok(e) => {
                let blob = repo.find_blob(e.id()).map_err(|e| {
                    AppError::coded("git.blob_read_failed").with("detail", e.to_string())
                })?;
                if blob.size() > MAX_DIFF_SIZE {
                    return Ok(DiffFileContent { original: String::new(), modified: String::new(), too_large: true });
                }
                String::from_utf8_lossy(blob.content()).to_string()
            }
            Err(_) => String::new(), // New file
        };

        Ok(DiffFileContent { original, modified, too_large: false })
    })
    .await
    .map_err(|e| AppError::coded("git.spawn_blocking_failed").with("detail", e.to_string()))?
}

#[tauri::command]
pub async fn git_commit_diff(path: String, hash: String) -> AppResult<CommitDiffResult> {
    tauri::async_runtime::spawn_blocking(move || {
        let repo = open_repo(&path)?;
        let oid = git2::Oid::from_str(&hash)
            .map_err(|e| AppError::coded("git.commit_hash_invalid").with("detail", e.to_string()))?;
        let commit = repo
            .find_commit(oid)
            .map_err(|e| AppError::coded("git.commit_not_found").with("detail", e.to_string()))?;
        let tree = commit
            .tree()
            .map_err(|e| AppError::coded("git.tree_parse_failed").with("detail", e.to_string()))?;
        let parent_tree = commit.parent(0).ok().and_then(|p| p.tree().ok());

        let diff = repo
            .diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), None)
            .map_err(|e| AppError::coded("git.diff_failed").with("detail", e.to_string()))?;

        let mut files = Vec::new();
        diff.foreach(
            &mut |delta, _| {
                let file_path = delta.new_file().path()
                    .or_else(|| delta.old_file().path())
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_default();
                let status = match delta.status() {
                    git2::Delta::Added => "A",
                    git2::Delta::Deleted => "D",
                    git2::Delta::Modified => "M",
                    git2::Delta::Renamed => "R",
                    _ => "?",
                };
                files.push(CommitDiffFile { path: file_path, status: status.to_string() });
                true
            },
            None, None, None,
        ).map_err(|e| {
            AppError::coded("git.diff_foreach_failed").with("detail", e.to_string())
        })?;

        let author = commit.author();
        let hash_str = oid.to_string();
        let info = CommitInfo {
            hash: hash_str.clone(),
            short_hash: hash_str[..7.min(hash_str.len())].to_string(),
            message: commit.message().unwrap_or("").to_string(),
            author: author.name().unwrap_or("").to_string(),
            email: author.email().unwrap_or("").to_string(),
            timestamp: author.when().seconds(),
        };

        Ok(CommitDiffResult { info, files })
    })
    .await
    .map_err(|e| AppError::coded("git.spawn_blocking_failed").with("detail", e.to_string()))?
}

#[tauri::command]
pub async fn git_commit_file_diff(
    path: String,
    hash: String,
    file: String,
) -> AppResult<DiffFileContent> {
    tauri::async_runtime::spawn_blocking(move || {
        let repo = open_repo(&path)?;
        let oid = git2::Oid::from_str(&hash)
            .map_err(|e| AppError::coded("git.hash_invalid").with("detail", e.to_string()))?;
        let commit = repo
            .find_commit(oid)
            .map_err(|e| AppError::coded("git.commit_not_found").with("detail", e.to_string()))?;
        let tree = commit
            .tree()
            .map_err(|e| AppError::coded("git.tree_read_failed").with("detail", e.to_string()))?;

        // New version (this commit)
        let modified = match tree.get_path(Path::new(&file)) {
            Ok(e) => {
                let blob = repo.find_blob(e.id()).map_err(|e| {
                    AppError::coded("git.blob_failed").with("detail", e.to_string())
                })?;
                if blob.size() > MAX_DIFF_SIZE {
                    return Ok(DiffFileContent { original: String::new(), modified: String::new(), too_large: true });
                }
                String::from_utf8_lossy(blob.content()).to_string()
            }
            Err(_) => String::new(), // File deleted in this commit
        };

        // Old version (parent commit)
        let original = match commit.parent(0) {
            Ok(parent) => {
                let parent_tree = parent.tree().map_err(|e| {
                    AppError::coded("git.parent_tree_failed").with("detail", e.to_string())
                })?;
                match parent_tree.get_path(Path::new(&file)) {
                    Ok(e) => {
                        let blob = repo.find_blob(e.id()).map_err(|e| {
                            AppError::coded("git.blob_failed").with("detail", e.to_string())
                        })?;
                        if blob.size() > MAX_DIFF_SIZE {
                            return Ok(DiffFileContent { original: String::new(), modified: String::new(), too_large: true });
                        }
                        String::from_utf8_lossy(blob.content()).to_string()
                    }
                    Err(_) => String::new(),
                }
            }
            Err(_) => String::new(), // Initial commit
        };

        Ok(DiffFileContent { original, modified, too_large: false })
    })
    .await
    .map_err(|e| AppError::coded("git.spawn_blocking_failed").with("detail", e.to_string()))?
}

// ─── Phase 3: Branch ops / Network CLI / Blame ────────────────────────

// ─── Phase 4: Git state watching ────────────────────────────────────

#[tauri::command]
pub fn watch_git_state(
    path: String,
    app: tauri::AppHandle,
    state: tauri::State<'_, GitWatcherState>,
) -> AppResult<()> {
    use tauri::Emitter;
    let mut guard = state
        .lock()
        .map_err(|_| AppError::coded("git.watcher_lock_failed"))?;

    // Remove old watcher
    if let Some(old) = guard.remove(&path) {
        old.stop_flag.store(true, Ordering::SeqCst);
    }

    // Find .git dir
    let repo = git2::Repository::discover(&path)
        .map_err(|e| AppError::coded("git.not_a_repo").with("detail", e.to_string()))?;
    let git_dir = repo.path().to_path_buf(); // .git/
    drop(repo);

    let head_path = git_dir.join("HEAD");
    let index_path = git_dir.join("index");

    let (tx, rx) = std::sync::mpsc::channel::<notify::Result<Event>>();
    let mut watcher = recommended_watcher(tx)
        .map_err(|e| AppError::coded("git.watcher_create_failed").with("detail", e.to_string()))?;

    // Watch .git/HEAD and .git/index
    if head_path.exists() {
        watcher.watch(&head_path, RecursiveMode::NonRecursive)
            .map_err(|e| AppError::coded("git.head_watch_failed").with("detail", e.to_string()))?;
    }
    if index_path.exists() {
        watcher.watch(&index_path, RecursiveMode::NonRecursive)
            .map_err(|e| AppError::coded("git.index_watch_failed").with("detail", e.to_string()))?;
    }

    let stop_flag = Arc::new(AtomicBool::new(false));
    let stop_clone = stop_flag.clone();
    let path_clone = path.clone();

    if let Ok(mut flags) = GIT_WATCHER_STOP_FLAGS.lock() {
        flags.push(stop_flag.clone());
    }

    std::thread::spawn(move || {
        let debounce = Duration::from_millis(500);
        let mut last_event: Option<Instant> = None;
        loop {
            if stop_clone.load(Ordering::SeqCst) { break; }
            match rx.recv_timeout(Duration::from_millis(50)) {
                Ok(Ok(_event)) => {
                    last_event = Some(Instant::now());
                }
                Ok(Err(_)) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            }
            if let Some(last) = last_event {
                if last.elapsed() >= debounce {
                    last_event = None;
                    let _ = app.emit("git-state-changed", &path_clone);
                }
            }
        }
    });

    guard.insert(path, GitWatcherEntry {
        stop_flag,
        watcher: Box::new(watcher),
    });

    Ok(())
}

#[tauri::command]
pub fn unwatch_git_state(
    path: String,
    state: tauri::State<'_, GitWatcherState>,
) -> AppResult<()> {
    let mut guard = state
        .lock()
        .map_err(|_| AppError::coded("git.watcher_lock_failed"))?;
    if let Some(entry) = guard.remove(&path) {
        entry.stop_flag.store(true, Ordering::SeqCst);
    }
    Ok(())
}

#[cfg(test)]
mod app_error_signature_guard {
    use super::*;

    /// `async fn` 的签名没法用 fn 指针直接钉（返回类型是 `impl Future`，写不出来），
    /// 故按实参个数分三个辅助函数：约束落在 future 的 `Output` 上，与直接写
    /// `fn(…) -> AppResult<…>` 等价。`async fn` 本身是 `fn(…) -> impl Future`，
    /// 能强制成这里的 fn 指针。
    fn pin1<A, T, F>(_: fn(A) -> F)
    where
        F: std::future::Future<Output = AppResult<T>>,
    {
    }

    fn pin2<A, B, T, F>(_: fn(A, B) -> F)
    where
        F: std::future::Future<Output = AppResult<T>>,
    {
    }

    fn pin3<A, B, C, T, F>(_: fn(A, B, C) -> F)
    where
        F: std::future::Future<Output = AppResult<T>>,
    {
    }

    /// 类型级回归守卫：本文件 9 个 `#[tauri::command]` 的 `Err` 必须是 `AppError`。
    ///
    /// **为什么必须有**：这 9 个命令直接跨 IPC，而 `impl From<AppError> for String` 会把
    /// `Coded` 退化成裸码（`Display` 就是 code），`params` 全丢，前端 `renderAppError`
    /// 认不出是码、原样上屏。**该 impl 已由计划 11 的 T5 删除**，故「只改签名、函数体不动」
    /// 这一种回退现在由编译器拦下（**实测**：把 `git_branches` 改回
    /// `Result<Vec<BranchInfo>, String>`，`cargo check` 报 4 × `E0277`（`?` 转不过去，
    /// `git.rs:197/200/207/217`）+ 1 × `E0271`（本守卫的 `pin1`）——**不再是 0 错**）。
    /// **但连签名带函数体一起回退**（`?` 的接收方也一并变回 `String`）不碰任何 `AppError`，
    /// 编译器看不见，闸门规则 2 也只认 CJK 码点。故这里把 9 个签名全部钉死在类型上——
    /// 改回去就**编译不过**，而不是静默放行。
    ///
    /// 只钉类型，不跑逻辑：这些命令会真的去开仓库、起 watcher，不适合当单测。
    #[test]
    fn git_command_errors_reach_ipc_as_app_error() {
        // 同步命令：签名能直接写成 fn 指针类型。
        let _: fn(String, tauri::State<'_, GitWatcherState>) -> AppResult<()> = unwatch_git_state;
        let _: fn(String, tauri::AppHandle, tauri::State<'_, GitWatcherState>) -> AppResult<()> =
            watch_git_state;
        // 异步命令：按实参个数走对应的 pin。
        pin1(git_branches);
        pin1(git_is_repo);
        pin2(git_status);
        pin2(git_commit_diff);
        pin2(git_diff_file_content);
        pin3(git_log);
        pin3(git_commit_file_diff);
    }
}
