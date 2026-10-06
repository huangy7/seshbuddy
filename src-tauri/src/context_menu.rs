use crate::error::{AppError, AppResult};
use serde::Serialize;
#[cfg_attr(not(target_os = "macos"), allow(unused_imports))]
use std::fs;
#[cfg(target_os = "windows")]
use crate::cli::{self, CliKind};

/// 右键菜单注册/注销的结果。**不是错误**——前端据此渲染本地化提示。
///
/// 只按**动作**分两个变体，平台留给前端判断：macOS 与 Windows 的提示措辞不同
/// （Finder 的「快速操作」 vs 资源管理器的右键菜单），但平台不是后端的业务数据——
/// 前端读得到（`navigator.userAgent`，仓库里已有 `types/cli.ts` 的 `IS_WINDOWS` 等同一写法），
/// 把平台写进变体只会让这个枚举随支持平台的数量增长，而每加一个平台都要改后端。
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ContextMenuOutcome {
    Registered,
    Unregistered,
}

/// Register system context menu for "Open with Claude Code".
pub fn register(skip_permissions: bool, terminal_app: Option<&str>) -> AppResult<ContextMenuOutcome> {
    #[cfg(target_os = "macos")]
    {
        register_macos(skip_permissions, terminal_app)
    }

    #[cfg(target_os = "windows")]
    {
        let _ = terminal_app;
        register_windows(skip_permissions)
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = (skip_permissions, terminal_app);
        Err(AppError::coded("cli.unsupported_platform"))
    }
}

/// Unregister system context menu.
pub fn unregister() -> AppResult<ContextMenuOutcome> {
    #[cfg(target_os = "macos")]
    {
        unregister_macos()
    }

    #[cfg(target_os = "windows")]
    {
        unregister_windows()
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        Err(AppError::coded("cli.unsupported_platform"))
    }
}

/// Check if system context menu is registered.
pub fn is_registered() -> AppResult<bool> {
    #[cfg(target_os = "macos")]
    {
        is_registered_macos()
    }

    #[cfg(target_os = "windows")]
    {
        is_registered_windows()
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        Ok(false)
    }
}

#[cfg(target_os = "macos")]
fn register_macos(skip_permissions: bool, terminal_app: Option<&str>) -> AppResult<ContextMenuOutcome> {
    let home = dirs::home_dir().ok_or_else(|| AppError::coded("cli.home_dir_missing"))?;
    let services_dir = home.join("Library").join("Services");
    let workflow_dir = services_dir.join("Claude Code.workflow");
    let contents_dir = workflow_dir.join("Contents");

    // Clean up old install if exists
    if workflow_dir.exists() {
        let _ = fs::remove_dir_all(&workflow_dir);
    }

    fs::create_dir_all(&contents_dir)
        .map_err(|e| AppError::coded("context_menu.register_permission_denied").with("detail", e.to_string()))?;

    // Info.plist — register as a service that receives folders
    //
    // ⚠️ `NSMenuItem.default` 与 `CFBundleName` 的 `Claude Code` **刻意不本地化**（品牌名豁免）：
    // 它是产品名，四语渲染逐字相同；同一个串同时是**工作流目录名**（`Claude Code.workflow`，
    // 见本函数开头与 `is_registered_macos` / `unregister_macos` 的存在性判据），
    // 按语言改写就要把「身份」与「显示名」拆开，收益为零。
    // 对照：Windows 一侧的菜单项文案含可译的动词短语，**不是**品牌名，已本地化
    // （见 `windows_context_menu_label`）。
    let info_plist = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleIdentifier</key>
    <string>com.seshbuddy.claude-code-service</string>
    <key>CFBundleName</key>
    <string>Claude Code</string>
    <key>CFBundleShortVersionString</key>
    <string>1.0</string>
    <key>NSServices</key>
    <array>
        <dict>
            <key>NSMenuItem</key>
            <dict>
                <key>default</key>
                <string>Claude Code</string>
            </dict>
            <key>NSMessage</key>
            <string>runWorkflowAsService</string>
            <key>NSSendFileTypes</key>
            <array>
                <string>public.folder</string>
            </array>
        </dict>
    </array>
</dict>
</plist>"#;
    fs::write(contents_dir.join("Info.plist"), info_plist)
        .map_err(|e| AppError::coded("context_menu.register_permission_denied").with("detail", e.to_string()))?;

    // Determine preferred terminal: 用户设置优先，未安装/未设置时回退自动探测
    let terminal_app = crate::terminal::macos_app_name(terminal_app);

    // 右键菜单动作属于「用 Claude 在此目录新建会话」这一条功能，不是逐 CLI 分派：
    // 菜单本身按 CLI 划分，新 CLI 本就不该出现在这里。故此处硬编码 "claude" 是功能范围的一部分，
    // 不是等待按 CLI 归位的遗漏——后续按能力收敛时不要把它一并收走。
    // skip_permissions 决定是否附加跳过权限确认的旗标。
    let claude_cmd = if skip_permissions {
        "claude --dangerously-skip-permissions"
    } else {
        "claude"
    };

    // document.wflow — Automator Quick Action that runs a shell script
    let document_wflow = format!(r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>AMApplicationBuild</key>
	<string>523</string>
	<key>AMApplicationVersion</key>
	<string>2.10</string>
	<key>AMDocumentVersion</key>
	<string>2</string>
	<key>actions</key>
	<array>
		<dict>
			<key>action</key>
			<dict>
				<key>AMAccepts</key>
				<dict>
					<key>Container</key>
					<string>List</string>
					<key>Optional</key>
					<true/>
					<key>Types</key>
					<array>
						<string>com.apple.cocoa.string</string>
					</array>
				</dict>
				<key>AMActionVersion</key>
				<string>2.0.3</string>
				<key>AMApplication</key>
				<array>
					<string>Automator</string>
				</array>
				<key>AMCategory</key>
				<string>AMCategoryUtilities</string>
				<key>AMProvides</key>
				<dict>
					<key>Container</key>
					<string>List</string>
					<key>Types</key>
					<array>
						<string>com.apple.cocoa.string</string>
					</array>
				</dict>
				<key>ActionBundlePath</key>
				<string>/System/Library/Automator/Run Shell Script.action</string>
				<key>ActionName</key>
				<string>Run Shell Script</string>
				<key>ActionParameters</key>
				<dict>
					<key>COMMAND_STRING</key>
					<string>for f in "$@"; do
    if [ -d "$f" ]; then
        SCRIPT=$(mktemp /tmp/seshbuddy_ctx_XXXXXX.sh)
        echo '#!/bin/bash -l' &gt; "$SCRIPT"
        echo "cd '$f' &amp;&amp; {claude_cmd}" &gt;&gt; "$SCRIPT"
        chmod +x "$SCRIPT"
        open -a "{terminal_app}" "$SCRIPT"
        break
    fi
done</string>
					<key>CheckedForUserDefaultShell</key>
					<true/>
					<key>inputMethod</key>
					<integer>1</integer>
					<key>shell</key>
					<string>/bin/bash</string>
					<key>source</key>
					<string></string>
				</dict>
				<key>BundleIdentifier</key>
				<string>com.apple.RunShellScript</string>
				<key>CFBundleVersion</key>
				<string>2.0.3</string>
				<key>CanShowSelectedItemsWhenRun</key>
				<false/>
				<key>CanShowWhenRun</key>
				<true/>
				<key>Category</key>
				<array>
					<string>AMCategoryUtilities</string>
				</array>
				<key>Class Name</key>
				<string>RunShellScriptAction</string>
				<key>InputUUID</key>
				<string>A12BFEE3-6E2E-4D8E-AD72-E0F3C2D7A1B0</string>
				<key>Keywords</key>
				<array>
					<string>Shell</string>
					<string>Script</string>
					<string>Command</string>
					<string>Run</string>
				</array>
				<key>OutputUUID</key>
				<string>B23CFFE4-7F3F-5E9F-BE83-F1A4D3E8B2C1</string>
				<key>UUID</key>
				<string>C34D00F5-8A40-6FA0-CF94-02B5E4F9C3D2</string>
				<key>UnlocalizedApplications</key>
				<array>
					<string>Automator</string>
				</array>
			</dict>
		</dict>
	</array>
	<key>connectors</key>
	<dict/>
	<key>workflowMetaData</key>
	<dict>
		<key>serviceInputTypeIdentifier</key>
		<string>com.apple.Automator.fileSystemObject.folder</string>
		<key>serviceOutputTypeIdentifier</key>
		<string>com.apple.Automator.nothing</string>
		<key>serviceProcessesInput</key>
		<integer>0</integer>
		<key>workflowTypeIdentifier</key>
		<string>com.apple.Automator.servicesMenu</string>
	</dict>
</dict>
</plist>"#, terminal_app = terminal_app, claude_cmd = claude_cmd);

    fs::write(contents_dir.join("document.wflow"), document_wflow)
        .map_err(|e| AppError::coded("context_menu.register_permission_denied").with("detail", e.to_string()))?;

    // Refresh services cache
    std::process::Command::new("/System/Library/CoreServices/pbs")
        .arg("-flush")
        .output()
        .ok();

    Ok(ContextMenuOutcome::Registered)
}

#[cfg(target_os = "macos")]
fn unregister_macos() -> AppResult<ContextMenuOutcome> {
    let home = dirs::home_dir().ok_or_else(|| AppError::coded("cli.home_dir_missing"))?;
    let workflow_dir = home
        .join("Library")
        .join("Services")
        .join("Claude Code.workflow");

    if workflow_dir.exists() {
        fs::remove_dir_all(&workflow_dir)
            .map_err(|e| AppError::coded("context_menu.unregister_failed").with("detail", e.to_string()))?;
    }

    // Refresh services cache
    std::process::Command::new("/System/Library/CoreServices/pbs")
        .arg("-flush")
        .output()
        .ok();

    Ok(ContextMenuOutcome::Unregistered)
}

#[cfg(target_os = "macos")]
fn is_registered_macos() -> AppResult<bool> {
    let home = dirs::home_dir().ok_or_else(|| AppError::coded("cli.home_dir_missing"))?;
    let workflow_dir = home
        .join("Library")
        .join("Services")
        .join("Claude Code.workflow");
    Ok(workflow_dir.exists())
}

#[cfg(target_os = "windows")]
fn register_windows(skip_permissions: bool) -> AppResult<ContextMenuOutcome> {
    use winreg::enums::*;
    use winreg::RegKey;

    let cli_path = cli::find_cli_path(CliKind::Claude)
        .ok_or_else(|| AppError::coded("context_menu.cli_not_found"))?;
    let helper_script = write_windows_context_helper(skip_permissions, &cli_path)?;
    let helper_script_str = helper_script
        .to_str()
        .ok_or_else(|| AppError::coded("context_menu.script_path_not_utf8"))?;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    write_windows_context_entry(
        &hkcu,
        r"Software\Classes\Directory\Background\shell\ClaudeCode",
        &windows_context_menu_label(),
        &cli_path,
        &windows_registry_command(helper_script_str, "%V"),
    )?;
    write_windows_context_entry(
        &hkcu,
        r"Software\Classes\Directory\shell\ClaudeCode",
        &windows_context_menu_label(),
        &cli_path,
        &windows_registry_command(helper_script_str, "%1"),
    )?;

    Ok(ContextMenuOutcome::Registered)
}

#[cfg(target_os = "windows")]
fn unregister_windows() -> AppResult<ContextMenuOutcome> {
    use winreg::enums::*;
    use winreg::RegKey;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let _ = hkcu.delete_subkey_all(r"Software\Classes\Directory\Background\shell\ClaudeCode");
    let _ = hkcu.delete_subkey_all(r"Software\Classes\Directory\shell\ClaudeCode");

    if let Ok(script_path) = windows_context_helper_path() {
        if let Err(e) = fs::remove_file(&script_path) {
            eprintln!("Failed to remove context menu helper script {:?}: {}", script_path, e);
        }
    }

    Ok(ContextMenuOutcome::Unregistered)
}

#[cfg(target_os = "windows")]
fn is_registered_windows() -> AppResult<bool> {
    use winreg::enums::*;
    use winreg::RegKey;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    // If the key can be opened, it means it's registered
    let key_exists = hkcu.open_subkey(r"Software\Classes\Directory\Background\shell\ClaudeCode").is_ok();
    Ok(key_exists)
}

/// 资源管理器右键菜单项的显示名。
///
/// 这一格与托盘菜单同类：字符串由**系统**（资源管理器）渲染，不经前端，故走 `native_text`
/// 读语言包，而不是 `errors.*`；`Claude Code` 是产品名，四语同值（与 `native.json` 里
/// 品牌名保留逐字的做法一致）。
///
/// **刻意不用 `#[cfg(target_os = "windows")]` 收窄**：收窄后这段调用在 macOS 上根本不参与
/// 编译，函数名/键名拼错没有任何编译期症状——而 Windows 目标在本仓开发机上编不出来
/// （`ring` 的 C 部分要 Windows SDK），等于两处都测不到。放在无 cfg 的函数里，macOS 上
/// 照常编译，`native_text` 的键则由 `native_text.rs` 的键集测试覆盖。
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn windows_context_menu_label() -> String {
    crate::native_text::native_text("native", "contextMenuOpenWith")
}

#[cfg(target_os = "windows")]
fn write_windows_context_entry(
    hkcu: &winreg::RegKey,
    path: &str,
    title: &str,
    icon: &str,
    command: &str,
) -> AppResult<()> {
    let (key, _) = hkcu
        .create_subkey(path)
        .map_err(|e| AppError::coded("context_menu.register_permission_denied").with("detail", e.to_string()))?;
    key.set_value("", &title)
        .map_err(|e| AppError::coded("context_menu.register_failed").with("detail", e.to_string()))?;
    key.set_value("Icon", &icon)
        .map_err(|e| AppError::coded("context_menu.register_failed").with("detail", e.to_string()))?;

    let (cmd_key, _) = hkcu
        .create_subkey(&format!("{}\\command", path))
        .map_err(|e| AppError::coded("context_menu.register_permission_denied").with("detail", e.to_string()))?;
    cmd_key
        .set_value("", &command)
        .map_err(|e| AppError::coded("context_menu.register_failed").with("detail", e.to_string()))?;
    Ok(())
}

#[cfg(target_os = "windows")]
fn windows_context_helper_path() -> AppResult<std::path::PathBuf> {
    let dir = crate::paths::app_data_dir()?.join("config");
    fs::create_dir_all(&dir)
        .map_err(|e| AppError::coded("context_menu.helper_dir_create_failed").with("detail", e.to_string()))?;
    Ok(dir.join("context-menu-claude.cmd"))
}

#[cfg(target_os = "windows")]
fn write_windows_context_helper(skip_permissions: bool, cli_path: &str) -> AppResult<std::path::PathBuf> {
    let path = windows_context_helper_path()?;
    let script = cli::build_windows_context_menu_script(CliKind::Claude, cli_path, skip_permissions);
    fs::write(&path, script)
        .map_err(|e| AppError::coded("context_menu.helper_write_failed").with("detail", e.to_string()))?;
    Ok(path)
}

#[cfg(target_os = "windows")]
fn windows_registry_command(script_path: &str, target_placeholder: &str) -> String {
    let escaped_script = script_path.replace('"', "\"\"");
    format!(
        r#"cmd.exe /d /c start "" "%COMSPEC%" /k "\"{}\" \"{}\"""#,
        escaped_script,
        target_placeholder,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 注册结果的线上形状：前端按 `kind` 取语言包键，平台措辞由前端自己选。
    ///
    /// 钉的是**形状**不是措辞——措辞已经全部搬进语言包。尤其要钉住「平台不进变体」：
    /// 若把 `Registered` 拆成 macos/windows 两个变体，前端的
    /// `Record<ContextMenuOutcome["kind"], string>` 会在类型上落空。
    #[test]
    fn context_menu_outcome_wire_shape_is_tagged_by_kind() {
        let registered = serde_json::to_value(ContextMenuOutcome::Registered).unwrap();
        assert_eq!(registered, serde_json::json!({ "kind": "registered" }));

        let unregistered = serde_json::to_value(ContextMenuOutcome::Unregistered).unwrap();
        assert_eq!(unregistered, serde_json::json!({ "kind": "unregistered" }));
    }
}
