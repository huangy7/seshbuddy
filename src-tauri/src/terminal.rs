//! 终端应用探测与选择。
//!
//! 前端设置值（terminal_app）：
//! - macOS: `"iterm2"` / `"terminal"`
//! - Windows: `"windows-terminal"` / `"powershell"` / `"cmd"`
//!
//! 统一约定：用户选择无效或未安装时回退到自动探测结果，保证永不因设置失效而启动失败。

use std::path::Path;

#[cfg(target_os = "macos")]
pub const MACOS_ITERM2: &str = "iterm2";
#[cfg(target_os = "macos")]
pub const MACOS_TERMINAL: &str = "terminal";
// cfg(any(windows, test))：纯决策逻辑不依赖 Windows API，放开让测试在 macOS 上也能跑
#[cfg(any(target_os = "windows", test))]
pub const WINDOWS_TERMINAL: &str = "windows-terminal";
#[cfg(any(target_os = "windows", test))]
pub const WINDOWS_POWERSHELL: &str = "powershell";
#[cfg(any(target_os = "windows", test))]
pub const WINDOWS_CMD: &str = "cmd";

/// 当前平台已安装的终端应用 id 列表（按推荐顺序）。
pub fn detect_installed() -> Vec<&'static str> {
    #[cfg(target_os = "macos")]
    {
        let mut apps = Vec::with_capacity(2);
        if iterm_installed() {
            apps.push(MACOS_ITERM2);
        }
        apps.push(MACOS_TERMINAL);
        return apps;
    }
    #[cfg(target_os = "windows")]
    {
        let mut apps = Vec::with_capacity(3);
        if windows_terminal_installed() {
            apps.push(WINDOWS_TERMINAL);
        }
        apps.push(WINDOWS_POWERSHELL);
        apps.push(WINDOWS_CMD);
        return apps;
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        Vec::new()
    }
}

#[cfg(target_os = "macos")]
pub fn iterm_installed() -> bool {
    Path::new("/Applications/iTerm.app").exists()
        || std::env::var("HOME")
            .map(|home| Path::new(&home).join("Applications/iTerm.app").exists())
            .unwrap_or(false)
}

/// macOS：`open -a` 使用的应用名。用户选择优先，未安装/未选择时回退自动探测。
#[cfg(target_os = "macos")]
pub fn macos_app_name(choice: Option<&str>) -> &'static str {
    resolve_macos_choice(choice, iterm_installed())
}

/// 纯函数，便于测试。
#[cfg(target_os = "macos")]
fn resolve_macos_choice(choice: Option<&str>, iterm_available: bool) -> &'static str {
    match choice {
        Some(MACOS_ITERM2) if iterm_available => "iTerm",
        Some(MACOS_TERMINAL) => "Terminal",
        // 未选择或选择已失效：保持原有自动探测行为
        _ => {
            if iterm_available {
                "iTerm"
            } else {
                "Terminal"
            }
        }
    }
}

#[cfg(target_os = "windows")]
pub fn windows_terminal_installed() -> bool {
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        if Path::new(&local)
            .join("Microsoft")
            .join("WindowsApps")
            .join("wt.exe")
            .exists()
        {
            return true;
        }
    }
    crate::cli::windows_hidden_command("where")
        .arg("wt.exe")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Windows：解析用户选择为可用的终端 id。未选择/已失效时回退 wt → cmd（保持原有行为）。
#[cfg(target_os = "windows")]
pub fn resolve_windows_choice(choice: Option<&str>) -> &'static str {
    resolve_windows_choice_with(choice, windows_terminal_installed())
}

/// 纯函数，便于测试。
#[cfg(any(target_os = "windows", test))]
fn resolve_windows_choice_with(choice: Option<&str>, wt_available: bool) -> &'static str {
    match choice {
        Some(WINDOWS_TERMINAL) if wt_available => WINDOWS_TERMINAL,
        Some(WINDOWS_POWERSHELL) => WINDOWS_POWERSHELL,
        Some(WINDOWS_CMD) => WINDOWS_CMD,
        _ => {
            if wt_available {
                WINDOWS_TERMINAL
            } else {
                WINDOWS_CMD
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "macos")]
    mod macos {
        use super::*;

        #[test]
        fn explicit_iterm_used_when_installed() {
            assert_eq!(resolve_macos_choice(Some(MACOS_ITERM2), true), "iTerm");
        }

        #[test]
        fn explicit_iterm_falls_back_to_terminal_when_missing() {
            assert_eq!(resolve_macos_choice(Some(MACOS_ITERM2), false), "Terminal");
        }

        #[test]
        fn explicit_terminal_always_honored() {
            assert_eq!(resolve_macos_choice(Some(MACOS_TERMINAL), true), "Terminal");
            assert_eq!(resolve_macos_choice(Some(MACOS_TERMINAL), false), "Terminal");
        }

        #[test]
        fn no_choice_keeps_auto_detect() {
            assert_eq!(resolve_macos_choice(None, true), "iTerm");
            assert_eq!(resolve_macos_choice(None, false), "Terminal");
        }

        #[test]
        fn unknown_choice_falls_back_to_auto_detect() {
            assert_eq!(resolve_macos_choice(Some("ghostty"), true), "iTerm");
            assert_eq!(resolve_macos_choice(Some("ghostty"), false), "Terminal");
        }

        #[test]
        fn detect_installed_always_contains_terminal() {
            let apps = detect_installed();
            assert!(apps.contains(&MACOS_TERMINAL));
            assert_eq!(apps.contains(&MACOS_ITERM2), iterm_installed());
        }
    }

    #[cfg(any(target_os = "windows", test))]
    mod windows {
        use super::*;

        #[test]
        fn explicit_wt_used_when_installed() {
            assert_eq!(
                resolve_windows_choice_with(Some(WINDOWS_TERMINAL), true),
                WINDOWS_TERMINAL
            );
        }

        #[test]
        fn explicit_wt_falls_back_to_cmd_when_missing() {
            assert_eq!(
                resolve_windows_choice_with(Some(WINDOWS_TERMINAL), false),
                WINDOWS_CMD
            );
        }

        #[test]
        fn explicit_powershell_and_cmd_always_honored() {
            assert_eq!(
                resolve_windows_choice_with(Some(WINDOWS_POWERSHELL), false),
                WINDOWS_POWERSHELL
            );
            assert_eq!(
                resolve_windows_choice_with(Some(WINDOWS_CMD), true),
                WINDOWS_CMD
            );
        }

        #[test]
        fn no_choice_keeps_wt_then_cmd_fallback() {
            assert_eq!(resolve_windows_choice_with(None, true), WINDOWS_TERMINAL);
            assert_eq!(resolve_windows_choice_with(None, false), WINDOWS_CMD);
        }

        // 「复制命令」语法约定：只有 CMD 用 cmd 语法（cd /d + &&），
        // WT（默认 profile 通常是 PowerShell）与 PowerShell 都用 PS 语法。
        #[test]
        fn only_cmd_choice_uses_cmd_copy_syntax() {
            assert_eq!(resolve_windows_choice_with(Some(WINDOWS_CMD), true), WINDOWS_CMD);
            assert_ne!(resolve_windows_choice_with(Some(WINDOWS_POWERSHELL), true), WINDOWS_CMD);
            assert_ne!(resolve_windows_choice_with(Some(WINDOWS_TERMINAL), true), WINDOWS_CMD);
            assert_ne!(resolve_windows_choice_with(None, true), WINDOWS_CMD);
        }
    }
}
