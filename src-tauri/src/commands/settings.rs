use crate::app_db;
use crate::cli::{self, CliKind, CliPathConfig, CliStatus};
use crate::error::{AppError, AppResult};

#[tauri::command]
pub fn list_cli_statuses() -> Vec<CliStatus> {
    cli::list_cli_statuses()
}

#[tauri::command]
pub fn list_cli_path_configs() -> AppResult<Vec<CliPathConfig>> {
    cli::list_cli_path_configs().map_err(AppError::from)
}

#[tauri::command]
pub fn set_cli_data_dir(
    app: tauri::AppHandle,
    cli_id: String,
    data_dir: Option<String>,
) -> AppResult<()> {
    let kind = CliKind::from_id(Some(cli_id.as_str()))?;
    cli::set_cli_data_dir_override(kind, data_dir)?;
    // 数据源一换，旧源范围内的会话就不再属于当前列表：必须在设置生效后立刻作废这些索引行。
    // 否则它们既不会被新一轮扫描枚举到，也走不到常规的陈旧行清理，会永远滞留在列表里。
    crate::commands::session_index::purge_index_rows_outside_data_source(kind);
    crate::session_index_watcher::restart_session_index_watchers(&app)?;
    Ok(())
}

/// Read the dock_visible setting from app.db.
/// Returns true (show dock icon) by default.
pub fn read_dock_visible_setting() -> bool {
    app_db::read_dock_visible().unwrap_or(true)
}

fn write_dock_visible_setting(visible: bool) -> AppResult<()> {
    app_db::write_dock_visible(visible)
}

#[cfg(target_os = "macos")]
fn resolve_macos_icon_path(app: &tauri::AppHandle) -> AppResult<std::path::PathBuf> {
    use std::path::Path;
    use tauri::Manager;

    let mut candidates = Vec::new();

    if let Ok(resource_dir) = app.path().resource_dir() {
        candidates.push(resource_dir.join("icon.icns"));
        candidates.push(resource_dir.join("icons").join("icon.icns"));
    }

    candidates.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("icons/icon.icns"));

    candidates
        .into_iter()
        .find(|path| path.is_file())
        .ok_or_else(|| AppError::coded("settings.app_icon_missing"))
}

#[cfg(target_os = "macos")]
pub(crate) fn refresh_macos_app_icon(app: &tauri::AppHandle) -> AppResult<()> {
    use objc2::{AnyThread, MainThreadMarker};
    use objc2_app_kit::{NSApplication, NSImage};
    use objc2_foundation::NSData;

    let icon_path = resolve_macos_icon_path(app)?;
    let icon_bytes = std::fs::read(&icon_path).map_err(|e| {
        AppError::coded("settings.app_icon_read_failed")
            .with("path", icon_path.display().to_string())
            .with("detail", e.to_string())
    })?;

    app.run_on_main_thread(move || {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };

        let image_data = NSData::with_bytes(&icon_bytes);
        if let Some(icon) = NSImage::initWithData(NSImage::alloc(), &image_data) {
            let app = NSApplication::sharedApplication(mtm);
            unsafe {
                app.setApplicationIconImage(Some(&icon));
            }
        }
    })
    .map_err(|e| AppError::coded("internal.tauri").with("detail", e.to_string()))?;

    Ok(())
}

#[cfg(target_os = "macos")]
pub(crate) fn apply_dock_visibility(
    app: &tauri::AppHandle,
    visible: bool,
    focus_window: bool,
) -> AppResult<()> {
    use tauri::Manager;

    app.set_dock_visibility(visible)
        .map_err(|e| AppError::coded("internal.tauri").with("detail", e.to_string()))?;

    if focus_window {
        if let Some(window) = app.get_webview_window("main") {
            let _ = window.show();
            let _ = window.set_focus();
        }
    }

    if visible {
        let _ = refresh_macos_app_icon(app);
    }

    Ok(())
}

/// Read the effective dock visibility setting from backend config.
#[tauri::command]
pub fn get_dock_visible() -> bool {
    read_dock_visible_setting()
}

/// macOS only: show or hide the Dock icon.
/// Persists the setting to config/dock_visible.
/// On other platforms this only updates the persisted setting.
#[tauri::command]
pub fn set_dock_visible(_app: tauri::AppHandle, visible: bool) -> AppResult<()> {
    #[cfg(target_os = "macos")]
    apply_dock_visibility(&_app, visible, true)?;

    write_dock_visible_setting(visible)?;

    Ok(())
}
