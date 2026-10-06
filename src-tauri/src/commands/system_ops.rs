use crate::app_db;
use crate::cli::{self, CliKind};
use crate::context_menu;
use crate::error::{AppError, AppResult};
use serde::Serialize;
use std::path::{Path, PathBuf};
use tauri::Emitter;

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchDeleteOutcome {
    /// `(path, reason)`。reason 是 `AppError` 而非 `String`：失败原因必须带着归属跨 IPC。
    /// ⚠️ 前端 `useSessions.ts` 的两个渲染点要过 `renderAppError`——`Coded` 的线格式是
    /// `{code, params}` 对象，原样插进 `{error}` 占位符会渲染成 `[object Object]`。
    pub failed: Vec<(String, AppError)>,
    pub succeeded: usize,
}

/// 文件已不存在 → 视为跳过（返回 Ok，计入成功）；存在则走系统回收站。
///
/// `trash::Error` 是**第三方**文本（R3：进 `params.detail`，不翻译）。**没有
/// `From<trash::Error> for AppError`**，故这里不能像同族的 OS 错误那样只留 `?`——
/// 少一层归属，用户看到的会是 `删除失败：<trash crate 的英文>`。
fn trash_or_skip(path: &str) -> AppResult<()> {
    if std::path::Path::new(path).exists() {
        trash::delete(path)
            .map_err(|e| AppError::coded("system_ops.trash_failed").with("detail", e.to_string()))
    } else {
        Ok(())
    }
}

/// 删除一个会话定位符：先问源能不能删，再走文件回收站。
///
/// **这道门是承重的。** 库型源的会话身份是 `cli://<id>/<key>` 虚拟键，不是文件路径；
/// `trash_or_skip` 的 `Path::exists()` 对它恒为假，于是静默返回 `Ok`、被
/// `run_batch_delete` 计为成功。随后数据库侧写墓碑、删索引行 —— 用户看到「删除成功」、
/// 列表里会话消失，而库里的行原封不动，且被墓碑隐藏到保留期满。这是「文件系统形状的
/// 假设撞上虚拟键」的同一类缺陷，只是这次毁的是用户数据的**可见性**却谎报成功。
///
/// 删除本就不在库型源的接入范围内（本期只做浏览、检索、用量、新建、恢复），所以这里
/// 诚实地判失败，而不是假装删掉。失败原因带 `cli` 归属，前端按 `{reasons}` 渲染。
fn trash_session(kind: CliKind, path: &str) -> AppResult<()> {
    if !crate::cli_registry::source_for(kind).can_delete() {
        return Err(AppError::coded("system_ops.delete_unsupported").with("cli", kind.name()));
    }
    trash_or_skip(path)
}

/// 逐路径删除，单个失败不中止；on_progress 每个路径处理完回调一次（done/total/current）。
/// 纯计数：`trash_one` 返回 Ok 计成功、Err 记失败；"跳过已不存在"由 `trash_or_skip` 决定。
pub fn run_batch_delete<F, G>(paths: &[String], mut trash_one: F, mut on_progress: G) -> BatchDeleteOutcome
where
    F: FnMut(&str) -> AppResult<()>,
    G: FnMut(usize, usize, &str),
{
    let total = paths.len();
    let mut out = BatchDeleteOutcome { failed: Vec::new(), succeeded: 0 };
    for (i, path) in paths.iter().enumerate() {
        match trash_one(path) {
            Ok(()) => out.succeeded += 1,
            Err(reason) => out.failed.push((path.clone(), reason)),
        }
        on_progress(i + 1, total, path);
    }
    out
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemDiagnostics {
    pub app_name: String,
    pub app_version: String,
    pub tauri_version: String,
    pub os: String,
    pub os_name: String,
    pub os_version: String,
    pub kernel_version: String,
    pub arch: String,
    pub cpu_model: String,
    pub cpu_cores: usize,
    pub memory_total: String,
    pub clis: Vec<CliDiagnosticInfo>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CliDiagnosticInfo {
    pub id: String,
    pub name: String,
    pub command: String,
    pub installed: bool,
    pub has_sessions: bool,
}

#[tauri::command]
pub fn get_system_diagnostics(app: tauri::AppHandle) -> AppResult<SystemDiagnostics> {
    let app_version = app.package_info().version.to_string();
    let tauri_version = tauri::VERSION.to_string();
    let os = std::env::consts::OS.to_string();
    let arch = std::env::consts::ARCH.to_string();
    let cpu_cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);

    #[cfg(target_os = "macos")]
    let (os_name, os_version, kernel_version, cpu_model, memory_total) = {
        let p_name = std::process::Command::new("sw_vers")
            .arg("-productName")
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| "macOS".to_string());
        let p_ver = std::process::Command::new("sw_vers")
            .arg("-productVersion")
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| "".to_string());
        let b_ver = std::process::Command::new("sw_vers")
            .arg("-buildVersion")
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| "".to_string());
        let os_name = if p_ver.is_empty() { p_name } else { format!("{} {}", p_name, p_ver) };
        let os_version = if b_ver.is_empty() { p_ver } else { format!("Build {}", b_ver) };

        let kernel_version = std::process::Command::new("uname")
            .args(["-s", "-r"])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| "Darwin".to_string());

        let cpu_model = std::process::Command::new("sysctl")
            .args(["-n", "machdep.cpu.brand_string"])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| format!("Apple Silicon ({})", arch));

        let memory_total = std::process::Command::new("sysctl")
            .args(["-n", "hw.memsize"])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .and_then(|s| s.trim().parse::<u64>().ok())
            .map(|bytes| format!("{:.2} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0)))
            .unwrap_or_else(|| "Unknown".to_string());

        (os_name, os_version, kernel_version, cpu_model, memory_total)
    };

    #[cfg(target_os = "windows")]
    let (os_name, os_version, kernel_version, cpu_model, memory_total) = {
        let ver = std::process::Command::new("cmd")
            .args(["/c", "ver"])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| "Microsoft Windows".to_string());
        let cpu = std::env::var("PROCESSOR_IDENTIFIER").unwrap_or_else(|_| "Unknown".to_string());
        ("Windows".to_string(), ver, "Windows NT".to_string(), cpu, "Unknown".to_string())
    };

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let (os_name, os_version, kernel_version, cpu_model, memory_total) = {
        let release = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
        let pretty = release
            .lines()
            .find(|line| line.starts_with("PRETTY_NAME="))
            .map(|line| line.trim_start_matches("PRETTY_NAME=").trim_matches('"').to_string())
            .unwrap_or_else(|| "Linux".to_string());
        let kernel = std::process::Command::new("uname")
            .arg("-r")
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| "Linux".to_string());
        (pretty, "".to_string(), kernel, "Unknown".to_string(), "Unknown".to_string())
    };

    let clis = crate::cli::CliKind::ALL
        .iter()
        .copied()
        .map(|kind| CliDiagnosticInfo {
            id: kind.id().to_string(),
            name: kind.name().to_string(),
            command: kind.command().to_string(),
            installed: cli::detect_cli(kind),
            has_sessions: cli::has_sessions(kind),
        })
        .collect();

    Ok(SystemDiagnostics {
        app_name: "SeshBuddy".to_string(),
        app_version,
        tauri_version,
        os,
        os_name,
        os_version,
        kernel_version,
        arch,
        cpu_model,
        cpu_cores,
        memory_total,
        clis,
    })
}

/// 当前系统用户名（Dashboard 问候语用）。
/// 优先读环境变量以免额外起进程；环境变量缺失时回退 `whoami`。两者都取不到则返回空串，
/// 由前端退化为不带用户名的问候语——问候语不是关键路径，不值得为它向上抛错。
#[tauri::command]
pub fn get_current_user_name() -> AppResult<String> {
    #[cfg(windows)]
    const ENV_KEYS: &[&str] = &["USERNAME"];
    #[cfg(not(windows))]
    const ENV_KEYS: &[&str] = &["USER", "LOGNAME"];

    for key in ENV_KEYS {
        if let Ok(value) = std::env::var(key) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Ok(trimmed.to_string());
            }
        }
    }

    let fallback = std::process::Command::new("whoami")
        .output()
        .ok()
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_default();
    Ok(fallback)
}

#[tauri::command]
pub fn detect_cli(cli_id: Option<String>) -> AppResult<bool> {
    let kind = CliKind::from_id(cli_id.as_deref())?;
    Ok(cli::detect_cli(kind))
}

#[tauri::command]
pub async fn delete_sessions_to_trash(
    app: tauri::AppHandle,
    cli_id: Option<String>,
    paths: Vec<String>,
) -> AppResult<BatchDeleteOutcome> {
    let kind = CliKind::from_id(cli_id.as_deref())?;
    let mut session_paths: Vec<String> = paths
        .into_iter()
        .map(|path| path.trim().to_string())
        .filter(|path| !path.is_empty())
        .collect();
    session_paths.sort();
    session_paths.dedup();
    let paths_for_db = session_paths.clone();

    let outcome = tauri::async_runtime::spawn_blocking(move || {
        let outcome = run_batch_delete(
            &session_paths,
            |path| trash_session(kind, path),
            |done, total, current| {
                let _ = app.emit(
                    "batch-delete-progress",
                    serde_json::json!({
                        "done": done,
                        "total": total,
                        "currentPath": current,
                    }),
                );
            },
        );

        // 仅对真正成功进入回收站（或已不存在）的路径执行数据库记录清理与墓碑写入，
        // 且全量数据库与 Tantivy 磁盘 I/O 均在工作线程中执行，彻底释放 Tokio 异步运行时。
        let failed_set: std::collections::HashSet<&str> = outcome
            .failed
            .iter()
            .map(|(path, _)| path.as_str())
            .collect();
        let succeeded_paths: Vec<String> = paths_for_db
            .into_iter()
            .filter(|path| !failed_set.contains(path.as_str()))
            .collect();

        if !succeeded_paths.is_empty() {
            app_db::delete_session_records(kind, &succeeded_paths)?;
        }

        Ok::<_, AppError>(outcome)
    })
    .await
    .map_err(|e| AppError::coded("system_ops.batch_delete_failed").with("detail", e.to_string()))??;

    Ok(outcome)
}

#[tauri::command]
pub fn get_launch_command(
    cli_id: Option<String>,
    project_path: String,
    session_id: Option<String>,
    skip_permissions: bool,
    profile_name: Option<String>,
    terminal_app: Option<String>,
) -> AppResult<String> {
    let kind = CliKind::from_id(cli_id.as_deref())?;
    let settings_file = match profile_name {
        Some(name) if !name.is_empty() => Some(crate::commands::profile::build_profile_settings_file(
            Some(kind.id().to_string()),
            name,
            None,
        )?),
        _ => None,
    };
    cli::build_launch_command_string(
        kind,
        &project_path,
        session_id.as_deref(),
        skip_permissions,
        settings_file.as_deref(),
        terminal_app.as_deref(),
    )
}

#[tauri::command]
pub fn clean_temp_configs() -> AppResult<usize> {
    let temp_dir = if cfg!(target_os = "macos") {
        std::path::PathBuf::from("/tmp")
    } else {
        std::env::temp_dir()
    };
    
    let mut deleted = 0;
    if let Ok(entries) = std::fs::read_dir(temp_dir) {
        for entry in entries.flatten() {
            if let Some(name) = entry.file_name().to_str() {
                if name.starts_with("seshbuddy-settings-") && name.ends_with(".json") {
                    if std::fs::remove_file(entry.path()).is_ok() {
                        deleted += 1;
                    }
                }
            }
        }
    }
    Ok(deleted)
}

#[tauri::command]
pub fn detect_terminal_apps() -> Vec<String> {
    crate::terminal::detect_installed()
        .into_iter()
        .map(str::to_string)
        .collect()
}

#[tauri::command]
pub fn open_in_terminal(
    cli_id: Option<String>,
    project_path: String,
    session_id: Option<String>,
    skip_permissions: bool,
    profile_name: Option<String>,
    terminal_app: Option<String>,
) -> AppResult<()> {
    let kind = CliKind::from_id(cli_id.as_deref())?;
    let settings_file = match profile_name {
        Some(name) if !name.is_empty() => Some(crate::commands::profile::build_profile_settings_file(
            Some(kind.id().to_string()),
            name,
            None,
        )?),
        _ => None,
    };
    cli::open_in_terminal(
        kind,
        &project_path,
        session_id.as_deref(),
        skip_permissions,
        settings_file.as_deref(),
        terminal_app.as_deref(),
    )
    .map_err(AppError::from)
}

#[tauri::command]
pub fn register_context_menu(
    skip_permissions: bool,
    terminal_app: Option<String>,
) -> AppResult<context_menu::ContextMenuOutcome> {
    context_menu::register(skip_permissions, terminal_app.as_deref()).map_err(AppError::from)
}

#[tauri::command]
pub fn unregister_context_menu() -> AppResult<context_menu::ContextMenuOutcome> {
    context_menu::unregister().map_err(AppError::from)
}

#[tauri::command]
pub fn is_context_menu_registered() -> AppResult<bool> {
    context_menu::is_registered().map_err(AppError::from)
}

pub(crate) fn resolve_path(path: &str) -> PathBuf {
    let trimmed = path.trim().trim_matches('"').trim_matches('\'');
    if trimmed == "~" {
        if let Some(home) = dirs::home_dir() {
            return home;
        }
    } else if let Some(stripped) = trimmed.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(stripped);
        }
    } else if let Some(stripped) = trimmed.strip_prefix("~\\") {
        if let Some(home) = dirs::home_dir() {
            return home.join(stripped);
        }
    }

    if let Some(stripped) = trimmed
        .strip_prefix("%USERPROFILE%\\")
        .or_else(|| trimmed.strip_prefix("%USERPROFILE%/"))
    {
        if let Some(home) = dirs::home_dir() {
            return home.join(stripped.replace('\\', "/"));
        }
    }

    #[cfg(target_os = "windows")]
    {
        if trimmed.starts_with('%') {
            if let Some(end_idx) = trimmed[1..].find('%') {
                let var_name = &trimmed[1..=end_idx];
                let rest = &trimmed[end_idx + 2..];
                if let Ok(val) = std::env::var(var_name) {
                    let rest_clean = rest.trim_start_matches(|c| c == '/' || c == '\\');
                    return PathBuf::from(val).join(rest_clean);
                }
            }
        }
    }

    PathBuf::from(trimmed)
}

#[tauri::command]
pub fn check_paths_exist(paths: Vec<String>) -> std::collections::HashMap<String, bool> {
    let mut map = std::collections::HashMap::new();
    for p in paths {
        let exists = resolve_path(&p).exists();
        map.insert(p, exists);
    }
    map
}

#[tauri::command]
pub fn open_path_in_file_manager(path: String) -> AppResult<()> {
    let target = resolve_path(&path);
    if !target.exists() {
        if let Some(parent) = target.parent() {
            if parent.exists() {
                return super::open_path(parent);
            }
        }
        // 前端据此给出「配置尚未创建」的专门提示：判据是错误码，不是文案
        // （译文随语言变，按文案子串匹配时分支会静默失效）。
        return Err(AppError::coded("system_ops.path_missing").with("path", path));
    }

    if target.is_file() {
        reveal_file_in_manager(&target)
    } else {
        super::open_path(&target)
    }
}

fn reveal_file_in_manager(path: &Path) -> AppResult<()> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg("-R")
            .arg(path)
            .spawn()
            .map_err(|e| AppError::coded("system_ops.reveal_failed").with("detail", e.to_string()))?;
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(format!("/select,{}", path.display()))
            .spawn()
            .map_err(|e| AppError::coded("system_ops.reveal_failed").with("detail", e.to_string()))?;
    }
    #[cfg(target_os = "linux")]
    {
        if let Some(parent) = path.parent() {
            std::process::Command::new("xdg-open")
                .arg(parent)
                .spawn()
                .map_err(|e| AppError::coded("system_ops.reveal_failed").with("detail", e.to_string()))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ★ 类型级回归守卫：`trash_or_skip` 的失败原因**带着归属**跨 IPC。
    ///
    /// **为什么必须有**：它的返回值经 `BatchDeleteOutcome.failed` 序列化后**被渲染**
    /// （`useSessions.ts` 的三个渲染点：单会话的 `deleteFailedMessage`、
    /// 项目删除与批量删除的 `deletePartial`）。
    /// **实测（T5 之后）**：退回 `Result<(), String>` 编译不过——函数体尾表达式的
    /// `.map_err(|e| AppError::coded(…))` 报 `E0308`（`system_ops.rs:26`），
    /// 本测试的 `F: FnMut(&str) -> AppResult<()>` 约束报 `E0271`。**但连函数体一起回退**
    /// 编译器看不见，用户看到的就是 `删除失败：<trash crate 的英文>`；闸门规则 2 只认
    /// CJK 码点，也看不见。故把签名钉在类型上：改回去本测试**编译不过**。
    ///
    /// ⚠️ **本测试只钉签名，钉不住函数体**——见下面 `trash_or_skip_reports_failure_as_coded`。
    ///
    /// `run_batch_delete` 的 `F` 约束也要一起钉——它决定 `failed` 里装什么。
    #[test]
    fn trash_failure_reaches_ipc_as_app_error() {
        let _: fn(&str) -> AppResult<()> = trash_or_skip;
        fn pin_batch_delete<F>(_: fn(&[String], F, fn(usize, usize, &str)) -> BatchDeleteOutcome)
        where
            F: FnMut(&str) -> AppResult<()>,
        {
        }
        pin_batch_delete::<fn(&str) -> AppResult<()>>(run_batch_delete);
    }

    /// ★ 行为证明（上一条钉不到的**函数体**）：`trash_or_skip` 的真实失败必须是
    /// **带非空 `detail` 的 `Coded("system_ops.trash_failed")`**，而不是裸英文的 `Business`。
    ///
    /// **为什么这条是必需的**：这是整个 `Result<_, String>` 类里**唯一会渲染到用户屏幕上**的
    /// 错误（`BatchDeleteOutcome.failed` → `useSessions.ts` 的 `{error}` / `{reasons}`）。
    /// 把函数体换成 `AppError::business(e.to_string())` 而保留 `AppResult<()>`：
    /// **编译得过、465 个测试全过**（前端测试 mock 的是载荷，看不见 Rust 侧的形状），
    /// 而用户看到的归属句会退化成 `trash crate` 的英文——正是本任务要消灭的形态。
    ///
    /// **怎么逼出一次真实的 trash 失败**：`trash::delete` 在 `canonicalize_paths` 阶段就对
    /// **没有父目录的路径**（`/`）返回 `Error::TargetedRoot`，且这一步**在动文件系统之前**
    /// （`trash-5.2.5/src/lib.rs` 的 `delete_all` 先 `canonicalize_paths(paths)?`）。
    /// 所以走的是**生产路径、无 mock**，既确定又无副作用——下面还断言了 `/` 仍然存在。
    ///
    /// ⚠️ **`#[cfg(unix)]` 是判据的一部分，不是图省事**：`/` 只在 Unix 上是「没有父目录的
    /// 路径」。Windows 上 `/` 会 canonicalize 成当前盘符根（`C:\` 之类），`trash` 走的是
    /// **另一条错误路径**，本测试的前提在那里不成立；而「让一条测试真的去 trash 一个盘符根」
    /// 本身就不该在跨平台仓里发生。故这条测试**只在 Unix 上跑**，Windows 上无等价覆盖
    /// （要覆盖得用另一个平台相关的根路径，那是另一条测试）。
    #[cfg(unix)]
    #[test]
    fn trash_or_skip_reports_failure_as_coded() {
        let err = trash_or_skip("/").expect_err("根目录必须被 trash 拒绝");
        assert!(
            std::path::Path::new("/").exists(),
            "这条测试不得真的动文件系统"
        );
        match err {
            AppError::Coded { code, params } => {
                assert_eq!(code, "system_ops.trash_failed");
                assert!(
                    params.get("detail").is_some_and(|d| !d.is_empty()),
                    "必须带非空 params.detail（第三方原文）"
                );
            }
            other => panic!("退回了非结构化错误：{other:?}"),
        }
    }

    fn make_paths(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("/tmp/ghost-{}.jsonl", i)).collect()
    }

    #[test]
    fn run_batch_delete_all_success_counts_and_progress() {
        let paths = make_paths(3);
        let mut progress = Vec::new();
        let out = run_batch_delete(&paths, |_| Ok(()), |done, total, cur| {
            progress.push((done, total, cur.to_string()));
        });
        assert!(out.failed.is_empty());
        assert_eq!(out.succeeded, 3);
        assert_eq!(progress, vec![
            (1, 3, "/tmp/ghost-0.jsonl".to_string()),
            (2, 3, "/tmp/ghost-1.jsonl".to_string()),
            (3, 3, "/tmp/ghost-2.jsonl".to_string()),
        ]);
    }

    #[test]
    fn run_batch_delete_failure_does_not_abort() {
        let paths = make_paths(3);
        let out = run_batch_delete(
            &paths,
            |p| if p.ends_with("ghost-1.jsonl") { Err(AppError::coded("system_ops.trash_failed")) } else { Ok(()) },
            |_, _, _| {},
        );
        assert_eq!(out.succeeded, 2);
        assert_eq!(out.failed.len(), 1);
        assert_eq!(out.failed[0].0, "/tmp/ghost-1.jsonl");
        assert_eq!(out.failed[0].1.to_string(), "system_ops.trash_failed");
    }

    #[test]
    fn trash_or_skip_returns_ok_for_missing_file() {
        assert!(trash_or_skip("/definitely/not/here.jsonl").is_ok());
    }

    #[test]
    fn run_batch_delete_with_skip_counts_missing_as_success() {
        // 不依赖 /tmp/ghost-* 恰好不存在：用 temp_dir + 随机后缀生成必然不冲突的路径，
        // 验证 trash_or_skip 对缺失文件返回 Ok、缺失文件计入成功。
        let uniq = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let paths: Vec<String> = (0..2)
            .map(|i| {
                std::env::temp_dir()
                    .join(format!("seshbuddy-ghost-{}-{}.jsonl", uniq, i))
                    .to_string_lossy()
                    .to_string()
            })
            .collect();
        let out = run_batch_delete(&paths, trash_or_skip, |_, _, _| {});
        assert_eq!(out.succeeded, 2);
        assert!(out.failed.is_empty());
    }

    /// 上面那道门不得误伤六个文件型源：缺失文件仍按「跳过已不存在」计成功。
    #[test]
    fn file_backed_sources_still_delete() {
        for kind in CliKind::ALL.iter().copied() {
            assert!(
                trash_session(kind, "/definitely/not/here.jsonl").is_ok(),
                "{kind:?} 的删除被误判为失败"
            );
        }
        let paths = vec!["/definitely/not/here.jsonl".to_string()];
        let out = run_batch_delete(
            &paths,
            |path| trash_session(CliKind::Claude, path),
            |_, _, _| {},
        );
        assert_eq!(out.succeeded, 1);
        assert!(out.failed.is_empty());
    }

    #[test]
    fn resolve_path_expands_tilde_and_quotes() {
        let home = dirs::home_dir().expect("must have home dir");
        assert_eq!(resolve_path("~"), home);
        assert_eq!(resolve_path("~/foo/bar"), home.join("foo/bar"));
        assert_eq!(resolve_path("~\\foo\\bar"), home.join("foo\\bar"));
        assert_eq!(resolve_path("\"~/foo/bar\""), home.join("foo/bar"));
    }

    #[test]
    fn resolve_path_expands_userprofile() {
        let home = dirs::home_dir().expect("must have home dir");
        assert_eq!(resolve_path("%USERPROFILE%\\.claude"), home.join(".claude"));
        assert_eq!(resolve_path("%USERPROFILE%/.claude"), home.join(".claude"));
    }

    #[test]
    fn resolve_path_leaves_normal_paths_intact() {
        assert_eq!(resolve_path("/Users/alice/projects"), PathBuf::from("/Users/alice/projects"));
    }
}
