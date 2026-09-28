use crate::error::{AppError, AppResult};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use crate::pty_manager;

#[tauri::command]
pub fn create_pty_session(
    app: tauri::AppHandle,
    project_path: String,
    cli_kind: String,
    session_id: Option<String>,
    cols: Option<u16>,
    rows: Option<u16>,
    resume_session_id: Option<String>,
    skip_permissions: Option<bool>,
    profile_name: Option<String>,
    agent_status_mode: Option<String>,
) -> AppResult<pty_manager::PtySessionInfo> {
    let settings_file = resolve_settings_file_for_pty(
        &cli_kind,
        profile_name,
        |cli_kind, profile_name| {
            crate::commands::profile::build_profile_settings_file(
                Some(cli_kind.to_string()),
                profile_name,
                None,
            )
        },
        crate::commands::profile::build_claude_temp_settings_file,
    )?;

    let result = pty_manager::create_session(
        app,
        project_path,
        cli_kind,
        session_id,
        cols,
        rows,
        resume_session_id,
        skip_permissions.unwrap_or(false),
        settings_file.clone(),
        agent_status_mode,
    )
    .map_err(AppError::from);

    if result.is_err() {
        cleanup_settings_file_after_create_error(&settings_file);
    }

    result
}

fn resolve_settings_file_for_pty<BuildProfile, BuildClaudeTemp>(
    cli_kind: &str,
    profile_name: Option<String>,
    build_profile: BuildProfile,
    build_claude_temp: BuildClaudeTemp,
) -> AppResult<Option<String>>
where
    BuildProfile: FnOnce(&str, String) -> AppResult<String>,
    BuildClaudeTemp: FnOnce() -> AppResult<String>,
{
    match profile_name {
        Some(name) if !name.is_empty() => build_profile(cli_kind, name).map(Some),
        _ if cli_kind == "claude" => match build_claude_temp() {
            Ok(path) => Ok(Some(path)),
            Err(e) => {
                tracing::warn!(
                    "[pty_command] failed to create Claude temp settings file; falling back to frontend status detection: {}",
                    e.diagnostic()
                );
                Ok(None)
            }
        },
        _ => Ok(None),
    }
}

fn cleanup_settings_file_after_create_error(settings_file: &Option<String>) {
    if let Some(path) = settings_file {
        let _ = std::fs::remove_file(path);
    }
}

#[tauri::command]
pub fn write_pty_session(session_id: String, data: String) -> AppResult<()> {
    // Frontend sends base64-encoded data
    let bytes = STANDARD
        .decode(&data)
        .map_err(|e| AppError::coded("pty.input_decode_failed").with("detail", e.to_string()))?;
    pty_manager::write_session(&session_id, &bytes).map_err(AppError::from)
}

#[tauri::command]
pub fn resize_pty_session(session_id: String, cols: u16, rows: u16) -> AppResult<()> {
    pty_manager::resize_session(&session_id, cols, rows).map_err(AppError::from)
}

#[tauri::command]
pub fn close_pty_session(session_id: String) -> AppResult<()> {
    pty_manager::close_session(&session_id).map_err(AppError::from)
}

#[tauri::command]
pub fn list_pty_sessions() -> Vec<pty_manager::PtySessionInfo> {
    pty_manager::list_sessions()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_profile_claude_temp_settings_error_falls_back_to_none() {
        let settings_file = resolve_settings_file_for_pty(
            "claude",
            None,
            |_cli_kind, _profile_name| unreachable!("no profile should not build profile settings"),
            || Err(AppError::business("temp settings failed")),
        )
        .unwrap();

        assert_eq!(settings_file, None);
    }

    #[test]
    fn profile_temp_settings_errors_remain_fail_fast() {
        let err = resolve_settings_file_for_pty(
            "claude",
            Some("review".to_string()),
            |_cli_kind, _profile_name| Err(AppError::business("profile settings failed")),
            || unreachable!("profile should not build no-profile temp settings"),
        )
        .unwrap_err();

        assert!(err.to_string().contains("profile settings failed"));
    }

    #[test]
    fn cleanup_settings_file_after_create_error_removes_generated_file() {
        let path = std::env::temp_dir().join(format!(
            "seshbuddy-pty-cleanup-test-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::write(&path, "{}").unwrap();
        let settings_file = Some(path.to_string_lossy().to_string());

        cleanup_settings_file_after_create_error(&settings_file);

        assert!(!path.exists());
    }
}
