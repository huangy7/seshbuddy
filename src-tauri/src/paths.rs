//! 应用数据目录的**单一来源**。
//!
//! 这个函数此前在 `db/mod.rs`、`commands/mod.rs` 与 `proxy.rs` 里各有一份**逐字相同**的实现
//! （另有 `context_menu.rs` 一处内联副本）。它们共用同一个错误码，但 `com.seshbuddy.app`
//! 这个路径字面量是各写各的——改数据目录、或改用 Tauri 解析的路径时，漏改一处就会让那个子系统
//! 静默读写另一个目录，而其余照常。
//!
//! 放在这里而不是 `commands/`：`db` 也要用它，而数据层依赖 IPC 层是反向的。

use crate::error::{AppError, AppResult};
use std::path::PathBuf;

/// `~/Library/Application Support/com.seshbuddy.app/`（各平台由 `dirs::data_dir()` 解析）。
///
/// 取不到时用 `cli_config.app_data_dir_missing`——这个码是刻意共用的：同一句
/// 「无法获取应用数据目录」若各建一个码，语言包里就会出现多遍同样的文案。
/// 返回 `AppResult` 而非 `Option`：这条错误要跨 IPC 给前端，压成 `String` 会退化成裸 code。
pub(crate) fn app_data_dir() -> AppResult<PathBuf> {
    let data = dirs::data_dir().ok_or_else(|| AppError::coded("cli_config.app_data_dir_missing"))?;
    Ok(data.join("com.seshbuddy.app"))
}
