use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::app_db;
use crate::error::AppResult;

const COMPLETED_KEY: &str = "onboarding.completed";
#[derive(Clone)]
struct InstallState {
    database_existed: bool,
    pending_marker: PathBuf,
}

static INSTALL_STATE: OnceLock<AppResult<InstallState>> = OnceLock::new();

fn capture_install_state_at(database_path: &Path) -> AppResult<InstallState> {
    let pending_marker = database_path.with_file_name("onboarding.pending");
    let database_existed = database_path.exists();
    if !database_existed && !pending_marker.exists() {
        if let Some(parent) = pending_marker.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::File::create(&pending_marker)?;
    }
    Ok(InstallState {
        database_existed,
        pending_marker,
    })
}

fn install_state() -> AppResult<InstallState> {
    INSTALL_STATE
        .get_or_init(|| app_db::app_db_path().and_then(|path| capture_install_state_at(&path)))
        .clone()
}

pub(crate) fn capture_install_state() {
    let state = install_state();
    if let Ok(state) = state.as_ref() {
        if state.pending_marker.exists() {
            if let Err(error) = app_db::write_setting_json(COMPLETED_KEY, &false) {
                tracing::warn!(
                    "failed to initialize onboarding state: {}",
                    error.diagnostic()
                );
            }
        }
    }
}

fn marker_to_initialize(
    completed: Option<bool>,
    database_existed: bool,
    pending: bool,
) -> Option<bool> {
    if completed.is_some() {
        None
    } else {
        Some(database_existed && !pending)
    }
}

fn needs_onboarding(completed: Option<bool>, database_existed: bool, pending: bool) -> bool {
    !completed.unwrap_or(database_existed && !pending)
}

#[tauri::command]
pub fn get_onboarding_status() -> AppResult<bool> {
    let state = install_state()?;
    let pending = state.pending_marker.exists();
    let completed = app_db::read_setting_json::<bool>(COMPLETED_KEY)?;
    if let Some(value) = marker_to_initialize(completed, state.database_existed, pending) {
        app_db::write_setting_json(COMPLETED_KEY, &value)?;
    }
    Ok(needs_onboarding(completed, state.database_existed, pending))
}

#[tauri::command]
pub fn complete_onboarding() -> AppResult<()> {
    app_db::write_setting_json(COMPLETED_KEY, &true)?;
    let state = install_state()?;
    if state.pending_marker.exists() {
        std::fs::remove_file(state.pending_marker)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{capture_install_state_at, marker_to_initialize, needs_onboarding};

    #[test]
    fn onboarding_is_only_required_for_an_unfinished_fresh_install() {
        assert!(needs_onboarding(None, false, true));
        assert!(!needs_onboarding(None, true, false));
        assert!(needs_onboarding(None, true, true));
        assert!(!needs_onboarding(Some(true), false, true));
        assert!(!needs_onboarding(Some(true), true, true));
        assert!(needs_onboarding(Some(false), false, true));
        assert!(needs_onboarding(Some(false), true, false));
    }

    #[test]
    fn fresh_install_records_unfinished_state_before_the_first_window_opens() {
        assert_eq!(marker_to_initialize(None, false, true), Some(false));
        assert_eq!(marker_to_initialize(None, true, false), Some(true));
        assert_eq!(marker_to_initialize(None, true, true), Some(false));
        assert_eq!(marker_to_initialize(Some(false), true, true), None);
    }

    #[test]
    fn pending_marker_survives_a_database_write_failure_and_restart() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("app.db");
        let first = capture_install_state_at(&db_path).unwrap();
        assert!(!first.database_existed);
        assert!(first.pending_marker.exists());

        std::fs::write(&db_path, b"").unwrap();
        let resumed = capture_install_state_at(&db_path).unwrap();
        assert!(resumed.database_existed);
        assert!(needs_onboarding(
            None,
            resumed.database_existed,
            resumed.pending_marker.exists()
        ));
    }
}
