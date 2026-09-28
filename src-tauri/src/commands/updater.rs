use crate::error::{AppError, AppResult};
use crate::app_db;
use crate::updater;

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdaterSettingsInput {
    pub auto_check: bool,
}

#[tauri::command]
pub fn get_updater_state(app: tauri::AppHandle) -> AppResult<updater::UpdaterState> {
    updater::get_updater_state(&app).map_err(AppError::from)
}

#[tauri::command]
pub fn set_updater_settings(
    app: tauri::AppHandle,
    input: UpdaterSettingsInput,
) -> AppResult<updater::UpdaterState> {
    let mut settings = app_db::read_updater_settings()?;
    settings.auto_check = input.auto_check;
    app_db::write_updater_settings(&settings)?;
    updater::get_updater_state(&app).map_err(AppError::from)
}

#[tauri::command]
pub async fn check_tauri_update(
    app: tauri::AppHandle,
) -> AppResult<Option<updater::TauriUpdateInfo>> {
    let result = updater::check_tauri_update(&app).await.map_err(AppError::from)?;
    if let Some(info) = &result {
        updater::publish_manual_update_available(&app, info).map_err(AppError::from)?;
    }
    Ok(result)
}

#[tauri::command]
pub async fn install_tauri_update(app: tauri::AppHandle) -> AppResult<()> {
    updater::install_tauri_update(&app).await.map_err(AppError::from)
}

#[tauri::command]
pub async fn check_tauri_update_bg(app: tauri::AppHandle) -> AppResult<()> {
    updater::check_and_notify_update(&app).await.map_err(AppError::from)
}

#[tauri::command]
pub fn ignore_update_version(
    app: tauri::AppHandle,
    version: String,
) -> AppResult<updater::UpdaterState> {
    let mut settings = app_db::read_updater_settings()?;
    settings.ignored_update_version = Some(version);
    app_db::write_updater_settings(&settings)?;
    // 收口在 `tray::attribute_rebuild_error` 里：先拆箱再归属，保住内层 `Coded` 的 code 与
    // params（`Box<dyn Error>` 的 `Display` 只渲染裸 code）。三个调用点共用它。
    crate::tray::rebuild_tray_menu_with_update(&app, None)
        .map_err(crate::tray::attribute_rebuild_error)?;
    updater::emit_update_cleared(&app);
    updater::get_updater_state(&app).map_err(AppError::from)
}
