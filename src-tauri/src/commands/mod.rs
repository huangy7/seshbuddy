use crate::error::{AppError, AppResult};
use std::path::{Path, PathBuf};

pub mod locale;
pub mod onboarding;
pub mod model_list;
pub mod profile;
pub mod proxy;
pub mod pty;
pub mod session;
pub mod session_index;
pub mod settings;
pub mod system_ops;
pub mod updater;
pub mod workspace_fs;

pub use locale::*;
pub use onboarding::*;
pub use model_list::*;
pub use profile::*;
pub use proxy::*;
pub use session::*;
pub use session_index::*;
pub use settings::*;
pub use system_ops::*;
pub use updater::*;

/// assistant/ — Assistant workspace and cache directory
pub(crate) fn assistant_dir() -> AppResult<PathBuf> {
    Ok(crate::paths::app_data_dir()?.join("assistant"))
}

pub(crate) fn open_path(path: &Path) -> AppResult<()> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(path)
            .spawn()
            .map_err(|e| AppError::coded("system_ops.open_failed").with("detail", e.to_string()))?;
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(path)
            .spawn()
            .map_err(|e| AppError::coded("system_ops.open_failed").with("detail", e.to_string()))?;
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(path)
            .spawn()
            .map_err(|e| AppError::coded("system_ops.open_failed").with("detail", e.to_string()))?;
    }
    Ok(())
}

/// Format an ISO 8601 timestamp to local time "YYYY-MM-DD HH:MM".
pub(crate) fn format_timestamp(ts: &str) -> String {
    use chrono::{DateTime, Local, Utc};
    match ts.parse::<DateTime<Utc>>() {
        Ok(utc) => {
            let local: DateTime<Local> = utc.with_timezone(&Local);
            local.format("%Y-%m-%d %H:%M").to_string()
        }
        Err(_) => ts.to_string(),
    }
}
