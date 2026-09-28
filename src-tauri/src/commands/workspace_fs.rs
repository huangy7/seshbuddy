use crate::error::{AppError, AppResult};
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

// ─── helpers ─────────────────────────────────────────────────────────────────

/// 确保 path 在 project_root 内，防止路径穿越
fn validate_within(path: &Path, project_root: &Path) -> AppResult<()> {
    let canonical_path = path
        .canonicalize()
        .map_err(|e| AppError::coded("fs.path_resolve_failed").with("detail", e.to_string()))?;
    let canonical_root = project_root
        .canonicalize()
        .map_err(|e| AppError::coded("fs.path_resolve_failed").with("detail", e.to_string()))?;
    if !canonical_path.starts_with(&canonical_root) {
        return Err(AppError::coded("fs.path_outside_root")
            .with("path", canonical_path.display().to_string())
            .with("root", canonical_root.display().to_string()));
    }
    Ok(())
}

fn is_ignored(name: &str) -> bool {
    matches!(
        name.to_lowercase().as_str(),
        "node_modules" | ".git" | "target" | ".ds_store" | "dist" | ".next" | "__pycache__"
    )
}

// ─── types ────────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct FileEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub children: Option<Vec<FileEntry>>,
}

// ─── commands ─────────────────────────────────────────────────────────────────

/// 列出项目目录文件树（递归，忽略常见噪音目录）
#[tauri::command]
pub async fn list_project_files(project_path: String, project_root: String, filter: Option<String>) -> AppResult<Vec<FileEntry>> {
    let root = PathBuf::from(&project_path);
    let root_boundary = PathBuf::from(&project_root);
    validate_within(&root, &root_boundary)?;
    if !root.is_dir() {
        // 与 `commands/session.rs` 的导出路径共用同一个码：同一句中文只建一个码，
        // 否则另一处会静默留在中文里而闸门不报（该句已有产出方）。
        return Err(AppError::coded("session.dir_not_found").with("path", project_path));
    }
    let filter_lower = filter.as_deref().map(|s| s.to_lowercase());
    Ok(scan_dir(&root, filter_lower.as_deref())?)
}

#[tauri::command]
pub async fn read_directory(path: String, project_root: String) -> AppResult<Vec<FileEntry>> {
    let p = PathBuf::from(&path);
    let root = PathBuf::from(&project_root);
    validate_within(&p, &root)?;
    if !p.is_dir() {
        return Err(AppError::coded("fs.not_a_directory").with("path", path));
    }
    let mut entries: Vec<FileEntry> = Vec::new();
    let read = fs::read_dir(&p).map_err(|e| AppError::coded("fs.read_dir_failed").with("detail", e.to_string()))?;
    for entry in read.filter_map(|e| e.ok()) {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') || is_ignored(&name) {
            continue;
        }
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        entries.push(FileEntry {
            name,
            path: entry.path().to_string_lossy().to_string(),
            is_dir,
            children: None,
        });
    }
    entries.sort_by_key(|e| (!e.is_dir, e.name.clone()));
    Ok(entries)
}

const MAX_SCAN_DEPTH: usize = 32;

fn scan_dir(dir: &Path, filter: Option<&str>) -> AppResult<Vec<FileEntry>> {
    scan_dir_inner(dir, filter, 0)
}

fn scan_dir_inner(dir: &Path, filter: Option<&str>, depth: usize) -> AppResult<Vec<FileEntry>> {
    if depth > MAX_SCAN_DEPTH {
        return Err(AppError::coded("fs.depth_exceeded"));
    }
    let mut entries: Vec<FileEntry> = Vec::new();
    let read = fs::read_dir(dir).map_err(|e| AppError::coded("fs.read_dir_failed").with("detail", e.to_string()))?;

    let mut items: Vec<_> = read
        .filter_map(|e| e.ok())
        .collect();
    items.sort_by_key(|e| {
        let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
        (!is_dir, e.file_name())
    });

    for entry in items {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        let ft = entry.file_type()?;
        let path_str = entry.path().to_string_lossy().to_string();

        if ft.is_dir() {
            if is_ignored(&name) {
                continue;
            }
            let children = scan_dir_inner(&entry.path(), filter, depth + 1)?;
            entries.push(FileEntry {
                name,
                path: path_str,
                is_dir: true,
                children: Some(children),
            });
        } else {
            if let Some(f) = filter {
                if !name.to_lowercase().contains(f) {
                    continue;
                }
            }
            entries.push(FileEntry {
                name,
                path: path_str,
                is_dir: false,
                children: None,
            });
        }
    }
    Ok(entries)
}

/// 把读文件失败映射成码：**内容不是合法 UTF-8**（二进制文件）单独立一个码。
///
/// 立这个码的理由：前端要据此走「不支持预览二进制」那条分支。没有它，前端只能去嗅探
/// 渲染后的文案里的 `UTF-8` 子串——而渲染结果随语言变，且那正是 `invokeApp` 明文禁止的
/// 「按后端文案子串判协议状态」。
///
/// ⚠️ 两个码都写成**字面量**：闸门规则 10 靠 `coded("…")` 的字面量做双向闭合，
/// 先把码存进变量再传进去会让它看不见，等于把这两个码从机械校验里摘出去。
///
/// 抽成独立函数是为了**能同步测**（`read_file_content` 是 `async` 且要过路径校验）。
fn read_file_error(e: std::io::Error) -> AppError {
    if e.kind() == std::io::ErrorKind::InvalidData {
        AppError::coded("fs.not_text_file").with("detail", e.to_string())
    } else {
        AppError::coded("fs.read_file_failed").with("detail", e.to_string())
    }
}

#[tauri::command]
pub async fn read_file_content(path: String, project_root: String) -> AppResult<String> {
    let p = PathBuf::from(&path);
    let root = PathBuf::from(&project_root);
    validate_within(&p, &root)?;
    fs::read_to_string(&p).map_err(read_file_error)
}

#[tauri::command]
pub async fn validate_workspace_file(path: String, project_root: String) -> AppResult<String> {
    let p = PathBuf::from(&path);
    let root = PathBuf::from(&project_root);
    validate_within(&p, &root)?;
    if !p.is_file() {
        return Err(AppError::coded("fs.file_missing").with("path", path));
    }
    p.canonicalize()
        .map(|path| path.to_string_lossy().to_string())
        .map_err(|e| AppError::coded("fs.path_resolve_failed").with("detail", e.to_string()))
}

#[tauri::command]
pub async fn save_file(path: String, content: String, project_root: String) -> AppResult<()> {
    let p = PathBuf::from(&path);
    let root = PathBuf::from(&project_root);
    // Validate parent (file may not exist yet for new files)
    let parent = p
        .parent()
        .ok_or_else(|| AppError::coded("fs.parent_dir_missing"))?;
    validate_within(parent, &root)?;
    if let Some(par) = p.parent() {
        fs::create_dir_all(par).map_err(|e| AppError::coded("fs.create_dir_failed").with("detail", e.to_string()))?;
    }
    fs::write(&p, content).map_err(|e| AppError::coded("fs.write_file_failed").with("detail", e.to_string()))
}

#[tauri::command]
pub async fn delete_file(path: String, project_root: String) -> AppResult<()> {
    let p = PathBuf::from(&path);
    let root = PathBuf::from(&project_root);
    validate_within(&p, &root)?;
    trash::delete(&p).map_err(|e| AppError::coded("fs.delete_file_failed").with("detail", e.to_string()))
}

// ─── File watcher state ────────────────────────────────────────────────────

use notify::{recommended_watcher, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashMap;
use std::sync::{mpsc, Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

struct WatcherEntry {
    _watcher: RecommendedWatcher,
    stop_flag: Arc<AtomicBool>,
}

pub struct FileWatcherState {
    watchers: HashMap<String, WatcherEntry>,
}

impl Default for FileWatcherState {
    fn default() -> Self {
        Self {
            watchers: HashMap::new(),
        }
    }
}

pub type FileWatcherStateType = Mutex<FileWatcherState>;

/// 开始监听指定文件，变更事件经 300ms 防抖后推送 `file-changed` 事件。
/// 重复调用同一路径会先关闭旧 watcher 再创建新的。
#[tauri::command]
pub async fn watch_file(
    path: String,
    app: tauri::AppHandle,
    state: tauri::State<'_, FileWatcherStateType>,
) -> AppResult<()> {
    use tauri::Emitter;
    let mut guard = state
        .lock()
        .map_err(|_| AppError::coded("fs.watch_lock_failed"))?;
    // Canonicalize path to prevent duplicate watchers for different representations
    let key = std::fs::canonicalize(&path)
        .unwrap_or_else(|_| PathBuf::from(&path))
        .to_string_lossy()
        .to_string();
    // Remove old watcher for same path (sets stop_flag)
    if let Some(old) = guard.watchers.remove(&key) {
        old.stop_flag.store(true, Ordering::SeqCst);
    }

    let (tx, rx) = mpsc::channel::<notify::Result<Event>>();
    let mut watcher =
        recommended_watcher(tx).map_err(|e| AppError::coded("fs.watch_create_failed").with("detail", e.to_string()))?;
    watcher
        .watch(Path::new(&path), RecursiveMode::NonRecursive)
        .map_err(|e| AppError::coded("fs.watch_dir_failed").with("detail", e.to_string()))?;

    let stop_flag = Arc::new(AtomicBool::new(false));
    let stop_flag_clone = stop_flag.clone();
    let path_clone = path.clone();
    let app_clone = app.clone();

    std::thread::spawn(move || {
        let debounce = Duration::from_millis(300);
        let mut last_event: Option<Instant> = None;
        loop {
            if stop_flag_clone.load(Ordering::SeqCst) {
                break;
            }
            match rx.recv_timeout(Duration::from_millis(50)) {
                Ok(Ok(event)) => {
                    if matches!(
                        event.kind,
                        EventKind::Modify(_) | EventKind::Create(_) | EventKind::Remove(_)
                    ) {
                        last_event = Some(Instant::now());
                    }
                }
                Ok(Err(_)) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
            if let Some(t) = last_event {
                if t.elapsed() >= debounce {
                    let _ = app_clone.emit("file-changed", &path_clone);
                    last_event = None;
                }
            }
        }
    });

    guard.watchers.insert(key, WatcherEntry { _watcher: watcher, stop_flag });
    Ok(())
}

/// 停止监听指定文件（关闭对应 watcher，停止 debounce 线程）
#[tauri::command]
pub async fn unwatch_file(
    path: String,
    state: tauri::State<'_, FileWatcherStateType>,
) -> AppResult<()> {
    let mut guard = state
        .lock()
        .map_err(|_| AppError::coded("fs.watch_lock_failed"))?;
    let key = std::fs::canonicalize(&path)
        .unwrap_or_else(|_| PathBuf::from(&path))
        .to_string_lossy()
        .to_string();
    if let Some(entry) = guard.watchers.remove(&key) {
        entry.stop_flag.store(true, Ordering::SeqCst);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code_of(e: AppError) -> &'static str {
        match e {
            AppError::Coded { code, .. } => code,
            other => panic!("期望 Coded，实际是 {other:?}"),
        }
    }

    /// 非 UTF-8 的内容必须报 `fs.not_text_file`。
    ///
    /// 判据是**码**不是「有错误」：前端靠这个码走「不支持预览二进制」那条分支，
    /// 它今天若退化成 `fs.read_file_failed`，用户看到的是一句泛泛的「读取文件失败」，
    /// 而没有任何测试会红——除非钉在这里。
    #[test]
    fn invalid_utf8_reports_not_text_file() {
        // `read_to_string` 对非法 UTF-8 的报法正是 `InvalidData`
        let e = std::io::Error::new(std::io::ErrorKind::InvalidData, "stream did not contain valid UTF-8");
        assert_eq!(code_of(read_file_error(e)), "fs.not_text_file");
    }

    /// 其余 IO 失败仍报 `fs.read_file_failed`（阴性对照：不能把所有失败都归成二进制）。
    #[test]
    fn other_io_errors_keep_read_file_failed() {
        for kind in [std::io::ErrorKind::NotFound, std::io::ErrorKind::PermissionDenied] {
            let e = std::io::Error::new(kind, "boom");
            assert_eq!(code_of(read_file_error(e)), "fs.read_file_failed");
        }
    }
}
