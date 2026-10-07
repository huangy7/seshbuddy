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

#[cfg(any(target_os = "linux", test))]
pub const LINUX_XDG_TERMINAL_EXEC: &str = "xdg-terminal-exec";
#[cfg(any(target_os = "linux", test))]
pub const LINUX_GNOME_TERMINAL: &str = "gnome-terminal";
#[cfg(any(target_os = "linux", test))]
pub const LINUX_KONSOLE: &str = "konsole";
#[cfg(any(target_os = "linux", test))]
pub const LINUX_XFCE4_TERMINAL: &str = "xfce4-terminal";
#[cfg(any(target_os = "linux", test))]
pub const LINUX_KITTY: &str = "kitty";
#[cfg(any(target_os = "linux", test))]
pub const LINUX_ALACRITTY: &str = "alacritty";
#[cfg(any(target_os = "linux", test))]
pub const LINUX_WEZTERM: &str = "wezterm";
#[cfg(any(target_os = "linux", test))]
pub const LINUX_FOOT: &str = "foot";
#[cfg(any(target_os = "linux", test))]
pub const LINUX_X_TERMINAL_EMULATOR: &str = "x-terminal-emulator";
#[cfg(any(target_os = "linux", test))]
pub const LINUX_XTERM: &str = "xterm";

#[cfg(any(target_os = "linux", test))]
pub const LINUX_CANDIDATES: &[&str] = &[
    LINUX_XDG_TERMINAL_EXEC,
    LINUX_GNOME_TERMINAL,
    LINUX_KONSOLE,
    LINUX_XFCE4_TERMINAL,
    LINUX_KITTY,
    LINUX_ALACRITTY,
    LINUX_WEZTERM,
    LINUX_FOOT,
    LINUX_X_TERMINAL_EMULATOR,
    LINUX_XTERM,
];

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
    #[cfg(target_os = "linux")]
    {
        return detect_installed_linux();
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
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

/// Linux：当前平台已安装的终端模拟器列表探测。
#[cfg(target_os = "linux")]
pub fn detect_installed_linux() -> Vec<&'static str> {
    let mut apps = Vec::new();
    for &candidate in LINUX_CANDIDATES {
        if is_executable_available(candidate) {
            apps.push(candidate);
        }
    }
    apps
}

#[cfg(target_os = "linux")]
fn is_executable_available(binary_name: &str) -> bool {
    if let Ok(path_var) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let candidate = dir.join(binary_name);
            if candidate.is_file() {
                return true;
            }
        }
    }
    let standard_dirs = ["/usr/bin", "/bin", "/usr/local/bin", "/snap/bin"];
    for dir in standard_dirs {
        if Path::new(dir).join(binary_name).is_file() {
            return true;
        }
    }
    false
}

/// Linux 终端命令行参数组装。
/// 不同终端模拟器在执行外部脚本时具有不同的参数标准（如 `--`、`-e`、`--command` 等）。
#[cfg(any(target_os = "linux", test))]
pub fn format_linux_terminal_command<'a>(app: &'a str, script_path: &str) -> (&'a str, Vec<String>) {
    match app {
        LINUX_GNOME_TERMINAL => (app, vec!["--".to_string(), script_path.to_string()]),
        LINUX_KONSOLE => (app, vec!["-e".to_string(), script_path.to_string()]),
        LINUX_XFCE4_TERMINAL => (app, vec!["--command".to_string(), script_path.to_string()]),
        LINUX_KITTY => (app, vec![script_path.to_string()]),
        LINUX_ALACRITTY => (app, vec!["-e".to_string(), script_path.to_string()]),
        LINUX_WEZTERM => (app, vec!["start".to_string(), "--".to_string(), script_path.to_string()]),
        LINUX_FOOT => (app, vec![script_path.to_string()]),
        LINUX_X_TERMINAL_EMULATOR | LINUX_XTERM => (app, vec!["-e".to_string(), script_path.to_string()]),
        LINUX_XDG_TERMINAL_EXEC => (app, vec![script_path.to_string()]),
        _ => (app, vec!["-e".to_string(), script_path.to_string()]),
    }
}

/// Linux：解析用户选择为可用的终端 id。未选择或选择已失效时按候选列表回退。
#[cfg(any(target_os = "linux", test))]
pub fn resolve_linux_choice_with<'a>(
    choice: Option<&'a str>,
    installed: &[&'a str],
) -> Option<&'a str> {
    if let Some(chosen) = choice {
        if installed.contains(&chosen) {
            return Some(chosen);
        }
    }
    installed.first().copied()
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

    #[cfg(any(target_os = "linux", test))]
    mod linux {
        use super::*;

        #[test]
        fn test_resolve_linux_choice_with_valid_selection() {
            let installed = vec![LINUX_GNOME_TERMINAL, LINUX_KITTY];
            assert_eq!(
                resolve_linux_choice_with(Some(LINUX_KITTY), &installed),
                Some(LINUX_KITTY)
            );
        }

        #[test]
        fn test_resolve_linux_choice_with_fallback_to_first() {
            let installed = vec![LINUX_GNOME_TERMINAL, LINUX_KITTY];
            assert_eq!(
                resolve_linux_choice_with(Some("non-existent"), &installed),
                Some(LINUX_GNOME_TERMINAL)
            );
            assert_eq!(
                resolve_linux_choice_with(None, &installed),
                Some(LINUX_GNOME_TERMINAL)
            );
        }

        #[test]
        fn test_resolve_linux_choice_with_empty_installed() {
            let installed: Vec<&str> = vec![];
            assert_eq!(resolve_linux_choice_with(None, &installed), None);
            assert_eq!(
                resolve_linux_choice_with(Some(LINUX_KITTY), &installed),
                None
            );
        }

        #[test]
        fn test_format_linux_terminal_command_arguments() {
            let script = "/tmp/seshbuddy-test.sh";
            assert_eq!(
                format_linux_terminal_command(LINUX_GNOME_TERMINAL, script),
                (LINUX_GNOME_TERMINAL, vec!["--".to_string(), script.to_string()])
            );
            assert_eq!(
                format_linux_terminal_command(LINUX_KONSOLE, script),
                (LINUX_KONSOLE, vec!["-e".to_string(), script.to_string()])
            );
            assert_eq!(
                format_linux_terminal_command(LINUX_XFCE4_TERMINAL, script),
                (LINUX_XFCE4_TERMINAL, vec!["--command".to_string(), script.to_string()])
            );
            assert_eq!(
                format_linux_terminal_command(LINUX_KITTY, script),
                (LINUX_KITTY, vec![script.to_string()])
            );
            assert_eq!(
                format_linux_terminal_command(LINUX_ALACRITTY, script),
                (LINUX_ALACRITTY, vec!["-e".to_string(), script.to_string()])
            );
            assert_eq!(
                format_linux_terminal_command(LINUX_WEZTERM, script),
                (LINUX_WEZTERM, vec!["start".to_string(), "--".to_string(), script.to_string()])
            );
            assert_eq!(
                format_linux_terminal_command(LINUX_FOOT, script),
                (LINUX_FOOT, vec![script.to_string()])
            );
            assert_eq!(
                format_linux_terminal_command(LINUX_X_TERMINAL_EMULATOR, script),
                (LINUX_X_TERMINAL_EMULATOR, vec!["-e".to_string(), script.to_string()])
            );
            assert_eq!(
                format_linux_terminal_command(LINUX_XTERM, script),
                (LINUX_XTERM, vec!["-e".to_string(), script.to_string()])
            );
            assert_eq!(
                format_linux_terminal_command(LINUX_XDG_TERMINAL_EXEC, script),
                (LINUX_XDG_TERMINAL_EXEC, vec![script.to_string()])
            );
        }

        #[test]
        fn test_linux_candidates_integrity() {
            assert!(!LINUX_CANDIDATES.is_empty());
            assert_eq!(LINUX_CANDIDATES[0], LINUX_XDG_TERMINAL_EXEC);
        }
    }
}
