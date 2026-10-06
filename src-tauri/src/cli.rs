use crate::app_db;
use crate::cli_registry::features::{launch_plan_for, LaunchPlan};
use crate::error::{AppError, AppResult};
use serde::Serialize;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
#[cfg(target_os = "macos")]
use std::sync::LazyLock;
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(target_os = "macos")]
static ANSI_REGEX: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"[\u001b\u009b][\[()#;?]*(?:[0-9]{1,4}(?:;[0-9]{0,4})*)?[0-9A-ORZcf-nqry=><]")
        .expect("Failed to compile ANSI regex")
});

#[cfg(target_os = "macos")]
fn strip_ansi_codes(input: &str) -> String {
    ANSI_REGEX.replace_all(input, "").to_string()
}

/// 变体清单的唯一定义处：枚举本体与 `ALL` 由同一条列表一次生成，
/// 「加了变体忘了放进清单」在构造上不可能发生。
///
/// 宏只负责「变体 ↔ 清单」这一条同步；`id()` 与 `source_for` 仍是手写穷尽 `match`，
/// 那两处的完整性由编译器兜底，交给宏生成反而会削弱报错信息与跳转定位。
macro_rules! cli_kinds {
    ($($variant:ident),+ $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum CliKind { $($variant),+ }

        impl CliKind {
            /// 全部变体，次序即声明次序，也是展示次序。
            pub const ALL: &'static [CliKind] = &[$(CliKind::$variant),+];
        }
    };
}

cli_kinds!(Claude, Codex, Gemini, WorkBuddy, Dsh, Antigravity);

#[derive(Debug, Clone, Serialize)]
pub struct CliStatus {
    pub id: String,
    pub has_sessions: bool,
    pub has_binary: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CliPathConfig {
    pub id: String,
    pub default_data_dir: String,
    pub effective_data_dir: String,
    pub sessions_dir: String,
    pub has_override: bool,
}

impl CliKind {
    pub fn id(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Gemini => "gemini",
            Self::WorkBuddy => "workbuddy",
            Self::Dsh => "dsh",
            Self::Antigravity => "antigravity",
        }
    }

    pub fn name(self) -> &'static str {
        crate::cli_registry::descriptor_for(self).name
    }

    /// 可执行名。仅作展示与回退，**不保证可直接执行** ——
    /// 桌面应用型 CLI 的恢复走深链，从不 exec 该字符串。
    pub fn command(self) -> &'static str {
        crate::cli_registry::descriptor_for(self).command
    }

    // 参数拼装已收敛到各 CLI 源文件里的启动计划（`cli_registry::features`）：
    // 命令行与深链两种启动形态在那里用类型表达，命令行参数只是其中一种。
    // 此处不再并列维护一份按变体分岔的参数表 —— 两张表迟早分叉，而分叉时两边都能编译。

    // 变体清单由 `cli_kinds!` 一次生成（枚举本体与 `ALL` 同源），
    // 故此处不再并列维护一份手写清单，也不再有需要人工同步的变体总数常量。

    pub fn from_id(value: Option<&str>) -> AppResult<Self> {
        // 反查走 `ALL` 而非再写一份字面量清单：`id()` 已是 id 串的唯一来源。
        match value.map(str::trim).filter(|value| !value.is_empty()) {
            Some(value) => CliKind::ALL
                .iter()
                .copied()
                .find(|kind| kind.id() == value)
                .ok_or_else(|| AppError::coded("cli.unsupported_kind").with("kind", value)),
            None => Err(AppError::coded("cli.id_required")),
        }
    }
}

pub fn list_cli_statuses() -> Vec<CliStatus> {
    CliKind::ALL
        .iter()
        .copied()
        .map(|kind| {
            let has_binary = detect_cli(kind);
            CliStatus {
                id: kind.id().to_string(),
                has_sessions: has_sessions(kind),
                has_binary,
            }
        })
        .collect()
}

/// 该 CLI 是否存在可恢复的会话。
/// 优先读会话索引计数;若索引暂不可用(如新接入的 Dsh 尚未建索引),
/// 回退到目录级探测:sessions_dir 存在且非空。
pub fn has_sessions(kind: CliKind) -> bool {
    if let Ok(count) = crate::app_db::count_session_list_index(kind) {
        if count > 0 {
            return true;
        }
    }
    let dir = match sessions_dir(kind) {
        Ok(d) => d,
        Err(_) => return false,
    };
    std::fs::read_dir(&dir)
        .map(|mut it| it.next().is_some())
        .unwrap_or(false)
}

/// 数据根取自描述符的分段表，故不再需要「嵌套目录」的特例分支。
pub fn default_data_dir(kind: CliKind) -> AppResult<PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| AppError::coded("cli.home_dir_missing"))?;
    let root = crate::cli_registry::descriptor_for(kind).file_root;
    // 逐级 join 而非拼字符串：前导点与路径分隔符交给 PathBuf 处理，
    // 从而消除此前对 Antigravity 两级目录的特例分支。
    Ok(root.data_dir_segments.iter().fold(home, |acc, seg| acc.join(seg)))
}

pub fn data_dir(kind: CliKind) -> AppResult<PathBuf> {
    let overrides = read_cli_path_overrides();
    if let Some(path) = overrides.get(kind.id()) {
        if let Ok(normalized) = normalize_cli_data_dir(path) {
            return Ok(PathBuf::from(normalized));
        }
    }
    default_data_dir(kind)
}

pub fn history_path(kind: CliKind) -> AppResult<PathBuf> {
    Ok(data_dir(kind)?.join("history.jsonl"))
}

/// 会话子目录取自描述符，故不再需要逐 CLI 的穷尽分支。
///
/// 基准走 `data_dir` 而非 `default_data_dir`：这里要跟着用户配置的数据目录走，
/// 描述符提供的只是没配置覆盖时的默认根。
pub fn sessions_dir(kind: CliKind) -> AppResult<PathBuf> {
    let root = crate::cli_registry::descriptor_for(kind).file_root;
    let subdir = root.sessions_subdir.ok_or_else(|| {
        // 库型源没有会话目录。调用方本就只该对文件型源调用本函数，
        // 报错让「误用」当场显形，而不是返回一个编造的路径。
        AppError::coded("cli.no_sessions_dir").with("kind", kind.id())
    })?;
    Ok(data_dir(kind)?.join(subdir))
}

pub fn list_cli_path_configs() -> AppResult<Vec<CliPathConfig>> {
    let overrides = read_cli_path_overrides();

    CliKind::ALL
        .iter()
        .copied()
        // 库型源没有会话子目录（会话在库里，不是文件），路径配置对它没有意义。
        // 跳过而不是让 `sessions_dir` 的报错经 `?` 短路整张表 —— 后者会让**所有**
        // CLI 的路径配置一起失败，而不只是库型源自己缺席。
        .filter(|kind| {
            crate::cli_registry::descriptor_for(*kind).file_root.sessions_subdir.is_some()
        })
        .map(|kind| {
            let default_data_dir = default_data_dir(kind)?;
            let effective_data_dir = data_dir(kind)?;
            let sessions_dir = sessions_dir(kind)?;

            Ok(CliPathConfig {
                id: kind.id().to_string(),
                default_data_dir: default_data_dir.to_string_lossy().to_string(),
                effective_data_dir: effective_data_dir.to_string_lossy().to_string(),
                sessions_dir: sessions_dir.to_string_lossy().to_string(),
                has_override: overrides.contains_key(kind.id()),
            })
        })
        .collect()
}

pub fn set_cli_data_dir_override(kind: CliKind, data_dir: Option<String>) -> AppResult<()> {
    let mut overrides = read_cli_path_overrides();

    match data_dir
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        Some(value) => {
            let normalized = normalize_cli_data_dir(&value)?;
            overrides.insert(kind.id().to_string(), normalized);
        }
        None => {
            overrides.remove(kind.id());
        }
    }

    write_cli_path_overrides(&overrides)
}

pub fn detect_cli(kind: CliKind) -> bool {
    find_cli_path(kind).is_some()
}

fn read_cli_path_overrides() -> HashMap<String, String> {
    app_db::read_cli_path_overrides().unwrap_or_default()
}

/// 写覆盖表。返回 `AppResult` 而非压平成 `String`：`AppError::Coded` 的 `Display` 是裸 code，
/// 压平会让 `set_cli_data_dir` 的调用者拿到 `internal.database` 而不是可本地化的错误。
fn write_cli_path_overrides(overrides: &HashMap<String, String>) -> AppResult<()> {
    app_db::write_cli_path_overrides(overrides)
}

fn normalize_cli_data_dir(value: &str) -> AppResult<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(AppError::coded("cli.path_required"));
    }

    let expanded = expand_home_path(trimmed)?;
    if !expanded.is_absolute() {
        return Err(AppError::coded("cli.path_not_absolute"));
    }

    Ok(expanded.to_string_lossy().to_string())
}

fn expand_home_path(value: &str) -> AppResult<PathBuf> {
    if value == "~" {
        return dirs::home_dir().ok_or_else(|| AppError::coded("cli.home_dir_missing"));
    }

    if let Some(stripped) = value.strip_prefix("~/") {
        let home = dirs::home_dir().ok_or_else(|| AppError::coded("cli.home_dir_missing"))?;
        return Ok(home.join(stripped));
    }

    Ok(PathBuf::from(value))
}

/// 渲染一条可粘贴到终端的命令串，供「复制命令」使用。
///
/// 与真实启动共用 `launch_plan_for` 的路由与报错：两个启动能力都不存在的 CLI 没有任何会话参数可拼，
/// 渲染出来只能是一条裸 CLI 启动命令，用户以为复制的是「恢复这条会话」，粘出去却开了一个
/// 全新会话，且全程不报错。与其静默把「恢复」变成「新开」，不如报错（前端显示为复制失败）。
pub fn build_launch_command_string(
    kind: CliKind,
    project_path: &str,
    session_id: Option<&str>,
    skip_permissions: bool,
    settings_file: Option<&str>,
    terminal_app: Option<&str>,
) -> AppResult<String> {
    let launch_args = match launch_plan_for(kind, session_id, skip_permissions, settings_file) {
        Ok(LaunchPlan::CommandLine { args }) => args,
        // 桌面应用型 CLI 的「命令」是深链，复制出来应是一条可直接粘贴执行的 open 命令。
        Ok(LaunchPlan::DeepLink { url }) => return Ok(format!("open \"{}\"", url)),
        // 深链的会话 ID 非法时给不出深链：只读场景回退到按应用名打开，与改动前一致。
        Err(AppError::Coded { code: "cli.invalid_workbuddy_session_id", .. }) => {
            return Ok(format!("open -a {}", kind.name()));
        }
        Err(e) => return Err(e),
    };
    let cli_path = find_cli_path(kind).unwrap_or_else(|| kind.command().to_string());
    Ok(build_command_line_string(&cli_path, &launch_args, project_path, settings_file, terminal_app))
}

/// 命令行形态的命令串。可执行文件与参数由调用方给出，因此这里与 CLI 数量无关 ——
/// 「复制命令」和「真实启动」共用它，两处不会各写一套切目录与清理规则。
fn build_command_line_string(
    cli_path: &str,
    launch_args: &[String],
    project_path: &str,
    settings_file: Option<&str>,
    terminal_app: Option<&str>,
) -> String {
    #[cfg(target_os = "windows")]
    {
        // 跟随终端设置生成粘贴可执行的语法：
        // 只有显式选 CMD 才用 cmd 语法（cd /d + &&）；Windows Terminal 默认 profile
        // 通常是 PowerShell，与 PowerShell 选项一样生成 PS 语法（PS 5.1 不支持 &&）
        if crate::terminal::resolve_windows_choice(terminal_app)
            != crate::terminal::WINDOWS_CMD
        {
            return build_powershell_command_line(cli_path, launch_args, project_path, settings_file);
        }
        let cli_command = build_windows_command_line(cli_path, launch_args);
        let mut cmd = format!("cd /d \"{}\" && {}", project_path, cli_command);
        if let Some(file) = settings_file {
            if file.contains("seshbuddy-settings-") {
                cmd = format!("{} & del /f /q \"{}\"", cmd, file);
            }
        }
        cmd
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = terminal_app;
        let cli_command = crate::shell_launch::build_startup_command(cli_path, launch_args);
        let mut cmd = format!("cd {} && {}", crate::shell_launch::posix_quote(project_path), cli_command);
        if let Some(file) = settings_file {
            if file.contains("seshbuddy-settings-") {
                cmd = format!("{}; rm -f {}", cmd, crate::shell_launch::posix_quote(file));
            }
        }
        cmd
    }
}

/// 定位 WorkBuddy 桌面应用 bundle(macOS)。由 WorkBuddy 的源用于安装检测 ——
/// 桌面应用型 CLI 没有 PATH 上的二进制，安装与否只能看应用 bundle 在不在。
pub(crate) fn find_workbuddy_app() -> Option<String> {
    let home = std::env::var("HOME").unwrap_or_default();
    [
        "/Applications/WorkBuddy.app".to_string(),
        format!("{}/Applications/WorkBuddy.app", home),
    ]
    .into_iter()
    .find(|p| std::path::Path::new(p).exists())
}

/// 按启动计划启动会话。分支数恒为 2（命令行 / 深链），与 CLI 数量无关 ——
/// 新增 CLI 只在自己的源文件里声明形态，不会在这里长出一个 `if kind ==` 分支。
pub fn open_in_terminal(
    kind: CliKind,
    project_path: &str,
    session_id: Option<&str>,
    skip_permissions: bool,
    settings_file: Option<&str>,
    terminal_app: Option<&str>,
) -> AppResult<()> {
    // 路由与 PTY 启动共用同一份判据，避免两个启动点对「新会话该走哪个能力」给出不同答案。
    let plan = launch_plan_for(kind, session_id, skip_permissions, settings_file)?;

    match plan {
        LaunchPlan::DeepLink { url } => open_deep_link(&url),
        LaunchPlan::CommandLine { args } => {
            // 解析可执行文件的实际位置：登录 shell 的 PATH 未必包含它，
            // 用户自定义路径与 Windows 上的 `.cmd` 包装尤其如此。取不到时退回描述符的命令名。
            let resolved = find_cli_path(kind).unwrap_or_else(|| kind.command().to_string());
            launch_in_terminal(&resolved, &args, project_path, settings_file, terminal_app)
        }
    }
}

/// 深链启动。macOS 走 `open`；其余平台返回明确错误而不是静默成功 ——
/// 静默成功会让「点了恢复但什么都没发生」变成没有报错的空白。
fn open_deep_link(url: &str) -> AppResult<()> {
    #[cfg(target_os = "macos")]
    {
        Command::new("open").arg(url).spawn().map_err(|e| {
            AppError::coded("cli.workbuddy_launch_failed").with("detail", e.to_string())
        })?;
        return Ok(());
    }
    #[cfg(not(target_os = "macos"))]
    {
        // 该平台没有系统级深链打开器，`url` 在本分支无事可做。这里显式消费它，
        // 是因为错误载荷里放不进去：`cli.workbuddy_macos_only` 的文案没有占位符，
        // 给站点加 `url` 参数会让「语言包声明 ⇔ 站点提供」的对账失败（那需要改四份语言包，
        // 而本任务的改动范围不含前端语言包）。
        // 不用 `#[allow(unused_variables)]`：属性会连同该分支里将来真正漏用的参数一起静默。
        let _ = url;
        Err(AppError::coded("cli.workbuddy_macos_only"))
    }
}

/// 终端启动。命令串由 `program` + `args` 组成，形态差异已在计划层消化，
/// 这里只剩「把命令交给哪个终端执行」这一件事。
fn launch_in_terminal(
    program: &str,
    args: &[String],
    project_path: &str,
    settings_file: Option<&str>,
    terminal_app: Option<&str>,
) -> AppResult<()> {
    #[cfg(target_os = "macos")]
    {
        let tmp_script = create_temp_launch_script(
            "sh",
            &{
                let mut script = format!(
                    "#!/bin/bash -l\n{}\n",
                    build_command_line_string(program, args, project_path, settings_file, terminal_app)
                );
                if let Some(sf) = settings_file {
                    script.push_str(&format!("rm -f {}\n", crate::shell_launch::posix_quote(sf)));
                }
                script.push_str("rm -f \"$0\"\n");
                script
            },
        )?;

        Command::new("chmod")
            .args(["+x", tmp_script.to_str().unwrap_or("")])
            .output()
            .map_err(|e| AppError::coded("cli.chmod_failed").with("detail", e.to_string()))?;

        let script_path = tmp_script.to_str().unwrap_or("").to_string();

        let terminal_app = crate::terminal::macos_app_name(terminal_app);

        Command::new("open")
            .args(["-a", terminal_app, &script_path])
            .spawn()
            .map_err(|e| AppError::coded("cli.terminal_not_found").with("detail", e.to_string()))?;

        return Ok(());
    }

    #[cfg(target_os = "windows")]
    {
        if crate::terminal::resolve_windows_choice(terminal_app)
            == crate::terminal::WINDOWS_POWERSHELL
        {
            let script_content = build_powershell_launch_script(
                program,
                args,
                project_path,
                settings_file,
            );
            let tmp_script = create_temp_launch_script("ps1", &script_content)?;
            let script_path = tmp_script
                .to_str()
                .ok_or_else(|| AppError::coded("cli.script_path_not_utf8"))?
                .to_string();
            Command::new("cmd")
                .args([
                    "/d", "/c", "start", "", "powershell", "-NoExit", "-ExecutionPolicy",
                    "Bypass", "-File", &script_path,
                ])
                .spawn()
                .map_err(|e| AppError::coded("cli.terminal_not_found").with("detail", e.to_string()))?;
            return Ok(());
        }

        let script_content =
            build_windows_launch_script(program, args, project_path, settings_file);
        let tmp_script = create_temp_launch_script("cmd", &script_content)?;
        let script_path = tmp_script
            .to_str()
            .ok_or_else(|| AppError::coded("cli.script_path_not_utf8"))?
            .to_string();

        let use_wt = crate::terminal::resolve_windows_choice(terminal_app)
            == crate::terminal::WINDOWS_TERMINAL;
        if use_wt {
            let wt_result = Command::new("wt.exe")
                .args(["-d", project_path, "cmd", "/k", &script_path])
                .spawn();
            if wt_result.is_ok() {
                return Ok(());
            }
        }

        Command::new("cmd")
            .args(["/d", "/c", "start", "", "cmd", "/k", &script_path])
            .spawn()
            .map_err(|e| AppError::coded("cli.terminal_not_found").with("detail", e.to_string()))?;
        return Ok(());
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = (program, args, project_path, settings_file, terminal_app);
        Err(AppError::coded("cli.unsupported_platform"))
    }
}

fn create_temp_launch_script(extension: &str, content: &str) -> AppResult<PathBuf> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or_default();
    let pid = std::process::id();
    let temp_dir = if cfg!(target_os = "macos") {
        PathBuf::from("/tmp")
    } else {
        std::env::temp_dir()
    };
    let path = temp_dir.join(format!(
        "seshbuddy-launch-{}-{}.{}",
        pid, timestamp, extension
    ));
    fs::write(&path, content)
        .map_err(|e| AppError::coded("cli.temp_script_create_failed").with("detail", e.to_string()))?;
    Ok(path)
}

#[cfg(target_os = "windows")]
fn build_windows_launch_script(
    cli_path: &str,
    launch_args: &[String],
    project_path: &str,
    settings_file: Option<&str>,
) -> String {
    let cli_command = build_windows_command_line(cli_path, launch_args);
    let mut script = format!(
        "@echo off\r\nsetlocal\r\ncd /d \"{}\"\r\ncall {}\r\n",
        escape_batch_value(project_path),
        cli_command,
    );
    if let Some(sf) = settings_file {
        script.push_str(&format!("del /f /q \"{}\" >nul 2>&1\r\n", escape_batch_value(sf)));
    }
    script
}

// cfg(any(windows, test))：纯字符串构造，不含 Windows API，测试在 macOS 上也能跑
#[cfg(any(target_os = "windows", test))]
fn ps_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

/// 单行 PowerShell 命令：用于「复制命令」等直接粘贴执行的场景
#[cfg(any(target_os = "windows", test))]
fn build_powershell_command_line(
    cli_path: &str,
    launch_args: &[String],
    project_path: &str,
    settings_file: Option<&str>,
) -> String {
    let mut parts = vec![ps_single_quote(cli_path)];
    parts.extend(launch_args.iter().map(|arg| ps_single_quote(arg)));
    let mut cmd = format!(
        "Set-Location -LiteralPath {}; & {}",
        ps_single_quote(project_path),
        parts.join(" ")
    );
    if let Some(sf) = settings_file {
        if sf.contains("seshbuddy-settings-") {
            cmd = format!(
                "{}; Remove-Item -LiteralPath {} -Force -ErrorAction SilentlyContinue",
                cmd,
                ps_single_quote(sf)
            );
        }
    }
    cmd
}

#[cfg(any(target_os = "windows", test))]
fn build_powershell_launch_script(
    cli_path: &str,
    launch_args: &[String],
    project_path: &str,
    settings_file: Option<&str>,
) -> String {
    let mut parts = vec![ps_single_quote(cli_path)];
    parts.extend(launch_args.iter().map(|arg| ps_single_quote(arg)));
    let mut script = format!(
        "Set-Location -LiteralPath {}\r\n& {}\r\n",
        ps_single_quote(project_path),
        parts.join(" ")
    );
    if let Some(sf) = settings_file {
        script.push_str(&format!(
            "Remove-Item -LiteralPath {} -Force -ErrorAction SilentlyContinue\r\n",
            ps_single_quote(sf)
        ));
    }
    script
}

#[cfg(target_os = "windows")]
pub fn build_windows_context_menu_script(
    kind: CliKind,
    cli_path: &str,
    skip_permissions: bool,
) -> String {
    // 右键菜单是「在这个目录新建会话」：只取参数，目录由脚本运行期从 `%~1` 取。
    let launch_args = match launch_plan_for(kind, None, skip_permissions, None) {
        Ok(LaunchPlan::CommandLine { args }) => args,
        // 无命令行形态的 CLI 在右键脚本里没有可拼装的参数。
        _ => Vec::new(),
    };
    let cli_command = build_windows_command_line(cli_path, &launch_args);
    format!(
        "@echo off\r\nsetlocal\r\nif \"%~1\"==\"\" (\r\n  echo 未提供目录路径。\r\n  exit /b 1\r\n)\r\ncd /d \"%~1\"\r\ncall {}\r\n",
        cli_command,
    )
}

#[cfg(target_os = "windows")]
fn build_windows_command_line(cli_path: &str, launch_args: &[String]) -> String {
    let mut parts = vec![quote_cmd_argument(cli_path)];
    parts.extend(launch_args.iter().map(|arg| quote_cmd_argument(arg)));
    parts.join(" ")
}

#[cfg(target_os = "windows")]
fn quote_cmd_argument(value: &str) -> String {
    if value.is_empty() {
        return "\"\"".to_string();
    }

    let needs_quotes = value.chars().any(|ch| {
        ch.is_whitespace() || matches!(ch, '"' | '&' | '|' | '<' | '>' | '^' | '(' | ')' | '%')
    });

    if !needs_quotes {
        return value.to_string();
    }

    let mut escaped = String::new();
    let mut backslashes = 0usize;

    for ch in value.chars() {
        match ch {
            '\\' => backslashes += 1,
            '"' => {
                escaped.push_str(&"\\".repeat(backslashes * 2 + 1));
                escaped.push('"');
                backslashes = 0;
            }
            '%' => {
                if backslashes > 0 {
                    escaped.push_str(&"\\".repeat(backslashes));
                    backslashes = 0;
                }
                escaped.push_str("%%");
            }
            _ => {
                if backslashes > 0 {
                    escaped.push_str(&"\\".repeat(backslashes));
                    backslashes = 0;
                }
                escaped.push(ch);
            }
        }
    }

    if backslashes > 0 {
        escaped.push_str(&"\\".repeat(backslashes * 2));
    }

    format!("\"{}\"", escaped)
}

#[cfg(target_os = "windows")]
fn escape_batch_value(value: &str) -> String {
    value.replace('%', "%%")
}

/// 在 PATH 上查找可执行文件。平台差异收在这里，各 CLI 的源直接复用，不必各自写一遍 cfg。
pub(crate) fn find_on_path(command: &str) -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        return find_cli_path_macos(command);
    }

    #[cfg(target_os = "windows")]
    {
        return find_cli_path_windows(command);
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = command;
        None
    }
}

pub fn find_cli_path(kind: CliKind) -> Option<String> {
    crate::cli_registry::source_for(kind).locate_executable()
}

#[cfg(target_os = "macos")]
fn find_cli_path_macos(command_name: &str) -> Option<String> {
    for shell in ["/bin/zsh", "/bin/bash"] {
        if let Ok(output) = Command::new(shell)
            .args(["-l", "-c", &format!("which {}", command_name)])
            .output()
        {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                for line in stdout.lines() {
                    let cleaned = strip_ansi_codes(line).trim().to_string();
                    if !cleaned.is_empty()
                        && cleaned.starts_with('/')
                        && std::path::Path::new(&cleaned).exists()
                    {
                        return Some(cleaned);
                    }
                }
            }
        }
    }

    let home = std::env::var("HOME").unwrap_or_default();
    let candidates = [
        format!("{}/.local/bin/{}", home, command_name),
        format!("{}/.npm-global/bin/{}", home, command_name),
        format!("/opt/homebrew/bin/{}", command_name),
        format!("/usr/local/bin/{}", command_name),
        format!("/usr/bin/{}", command_name),
    ];

    candidates
        .into_iter()
        .find(|path| std::path::Path::new(path).exists())
}

#[cfg(target_os = "windows")]
pub(crate) fn windows_hidden_command(program: &str) -> Command {
    use std::os::windows::process::CommandExt;

    const CREATE_NO_WINDOW: u32 = 0x08000000;
    let mut cmd = Command::new(program);
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}

#[cfg(target_os = "windows")]
fn find_cli_path_windows(command_name: &str) -> Option<String> {
    let mut candidates = Vec::new();
    if let Ok(appdata) = std::env::var("APPDATA") {
        candidates.push(
            PathBuf::from(&appdata)
                .join("npm")
                .join(format!("{}.cmd", command_name)),
        );
        candidates.push(
            PathBuf::from(&appdata)
                .join("npm")
                .join(format!("{}.exe", command_name)),
        );
    }
    if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
        candidates.push(
            PathBuf::from(&local_app_data)
                .join("Microsoft")
                .join("WindowsApps")
                .join(format!("{}.exe", command_name)),
        );
    }

    if let Some(path) = candidates
        .into_iter()
        .find(|path| path.exists())
        .map(|path| path.to_string_lossy().to_string())
    {
        return Some(path);
    }

    let path_var = std::env::var_os("PATH")?;
    let path_exts = std::env::var("PATHEXT")
        .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string())
        .split(';')
        .filter_map(|ext| {
            let trimmed = ext.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        })
        .collect::<Vec<_>>();

    for dir in std::env::split_paths(&path_var) {
        let direct = dir.join(command_name);
        if direct.exists() {
            return Some(direct.to_string_lossy().to_string());
        }

        for ext in &path_exts {
            let candidate = dir.join(format!("{}{}", command_name, ext));
            if candidate.exists() {
                return Some(candidate.to_string_lossy().to_string());
            }
        }
    }

    let output = windows_hidden_command("where")
        .arg(command_name)
        .output()
        .ok()?;
    if output.status.success() {
        let first = String::from_utf8_lossy(&output.stdout)
            .lines()
            .next()
            .unwrap_or("")
            .trim()
            .to_string();
        if !first.is_empty() {
            return Some(first);
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 每个 CLI 都必须对「怎么找到自己」给出答案，且**找错了路径形态**要能失败。
    ///
    /// 环境里没装某个 CLI 时结果是 `None`，所以只能做条件断言 —— 但正是这条条件断言能挡住
    /// 「桌面应用型 CLI 被误按 PATH 查找」与「PATH 型 CLI 被误按应用包查找」这两种错配：
    /// 它们都能编译、都能跑，只是永远找不到。
    #[test]
    fn every_cli_answers_how_to_locate_itself() {
        for kind in CliKind::ALL {
            let Some(found) = crate::cli_registry::source_for(*kind).locate_executable() else {
                // 本机没装：无从断言，跳过。
                continue;
            };
            if *kind == CliKind::WorkBuddy {
                assert!(
                    found.ends_with(".app") || found.contains("WorkBuddy"),
                    "WorkBuddy 是桌面应用，定位结果不该是 PATH 上的可执行文件：{found}"
                );
            } else {
                assert!(
                    found.contains(kind.command()),
                    "{kind:?} 的定位结果应当来自它自己的可执行名 {}，实际 {found}",
                    kind.command()
                );
            }
        }
    }

    #[test]
    fn powershell_command_line_uses_set_location_and_call_operator() {
        let args = vec!["--resume".to_string(), "abc-123".to_string()];
        let cmd = build_powershell_command_line(
            "C:\\Users\\y\\npm\\claude.cmd",
            &args,
            "D:\\codes\\proj",
            None,
        );
        assert_eq!(
            cmd,
            "Set-Location -LiteralPath 'D:\\codes\\proj'; & 'C:\\Users\\y\\npm\\claude.cmd' '--resume' 'abc-123'"
        );
    }

    #[test]
    fn powershell_command_line_escapes_single_quotes() {
        let cmd = build_powershell_command_line("claude", &[], "D:\\it's here", None);
        assert!(cmd.contains("'D:\\it''s here'"));
    }

    #[test]
    fn powershell_command_line_appends_settings_cleanup_only_for_temp_settings() {
        let args = vec!["--settings".to_string(), "C:\\tmp\\seshbuddy-settings-1.json".to_string()];
        let cmd = build_powershell_command_line(
            "claude",
            &args,
            "D:\\proj",
            Some("C:\\tmp\\seshbuddy-settings-1.json"),
        );
        assert!(cmd.contains(
            "; Remove-Item -LiteralPath 'C:\\tmp\\seshbuddy-settings-1.json' -Force -ErrorAction SilentlyContinue"
        ));

        let keep = build_powershell_command_line("claude", &[], "D:\\proj", Some("C:\\tmp\\my-settings.json"));
        assert!(!keep.contains("Remove-Item"));
    }

    #[test]
    fn powershell_launch_script_contains_set_location_and_invocation() {
        let args = vec!["--resume".to_string(), "abc".to_string()];
        let script = build_powershell_launch_script("claude", &args, "D:\\proj", None);
        assert!(script.starts_with("Set-Location -LiteralPath 'D:\\proj'\r\n"));
        assert!(script.contains("& 'claude' '--resume' 'abc'\r\n"));
    }

    #[test]
    fn dsh_default_data_dir_is_home_dsh() {
        let dir = default_data_dir(CliKind::Dsh).unwrap();
        assert!(dir.ends_with(".dsh"));
    }
    #[test]
    fn dsh_sessions_dir_is_sessions_subdir() {
        let dir = sessions_dir(CliKind::Dsh).unwrap();
        assert_eq!(dir.file_name().unwrap().to_str(), Some("sessions"));
    }
    #[test]
    fn antigravity_default_data_dir_is_home_gemini_antigravity() {
        let dir = default_data_dir(CliKind::Antigravity).unwrap();
        assert!(dir.ends_with(".gemini/antigravity-cli") || dir.ends_with(".gemini\\antigravity-cli"));
    }
    #[test]
    fn antigravity_sessions_dir_is_brain_subdir() {
        let dir = sessions_dir(CliKind::Antigravity).unwrap();
        assert_eq!(dir.file_name().unwrap().to_str(), Some("brain"));
    }
    #[test]
    fn cli_status_carries_antigravity() {
        let statuses = list_cli_statuses();
        let agy = statuses.iter().find(|s| s.id == "antigravity").expect("antigravity 在列表中");
        assert_eq!(
            agy.has_sessions && agy.has_binary,
            agy.has_binary && agy.has_sessions
        );
    }

    /// 类型级回归守卫：本批把 `cli.rs` / `context_menu.rs` 的 `Result<_, String>` 签名改成
    /// `AppResult`，它们的 `Coded` 错误必须一路以 `AppError` 到 IPC，中途一次都不许压成
    /// `String`。
    ///
    /// `impl From<AppError> for String` 会把 `Coded` 退化成裸 code（`Display` 就是 code），
    /// 而这条退化路径**不报错、不 panic**。**该 impl 已由计划 11 的 T5 删除**，故「只改签名、
    /// 函数体不动」这一种回退现在由编译器拦下（**实测**：把 `set_cli_data_dir` 改回
    /// `Result<(), String>`，`cargo check` 报 3 × `E0277`（`settings.rs:21/22/26`）——
    /// **不再是「都编译得过」**）。**但连签名带函数体一起回退**不碰任何 `AppError`，
    /// 编译器看不见函数体；调用点那些 `.map_err(AppError::from)`
    /// （`commands/system_ops.rs` 的 `open_in_terminal` / `register_context_menu` /
    /// `unregister_context_menu`）曾靠反向的 `From<String> for AppError`
    /// 对两种签名都成立，**那条 impl 已由计划 13 的 T2 删除**。所以这里把签名钉死在
    /// 类型上——把任一环改回 `Result<_, String>`，本测试**编译不过**，而不是静默放行。
    ///
    /// 只钉类型，不跑逻辑。
    ///
    /// ⚠️ **三个 `fn(&str)` 形态的私有函数也要钉**（`normalize_cli_data_dir` / `expand_home_path` /
    /// `create_temp_launch_script`）：它们的 `?` 消费点都在 `AppResult` 函数里。
    /// 改回 `Result<_, String>` 现在编译不过（**实测**：`normalize_cli_data_dir` 改回去报
    /// `E0277` + 3 × `E0308`）；**但连函数体一起回退**编译器看不见，而它们承载的
    /// `cli.path_required` / `cli.path_not_absolute` / `cli.home_dir_missing` /
    /// `cli.temp_script_create_failed` 会就此退化成裸 code。守卫漏钉这三个 = 漏掉同一类缺陷。
    #[test]
    fn coded_errors_reach_ipc_as_app_error() {
        let _: fn(&HashMap<String, String>) -> AppResult<()> = write_cli_path_overrides;
        let _: fn(CliKind, Option<String>) -> AppResult<()> = set_cli_data_dir_override;
        let _: fn(&str) -> AppResult<String> = normalize_cli_data_dir;
        let _: fn(&str) -> AppResult<PathBuf> = expand_home_path;
        let _: fn(&str, &str) -> AppResult<PathBuf> = create_temp_launch_script;
        let _: fn(Option<&str>) -> AppResult<CliKind> = CliKind::from_id;
        let _: fn(CliKind) -> AppResult<PathBuf> = default_data_dir;
        let _: fn(CliKind) -> AppResult<PathBuf> = data_dir;
        let _: fn(CliKind) -> AppResult<PathBuf> = sessions_dir;
        let _: fn(CliKind) -> AppResult<PathBuf> = history_path;
        let _: fn() -> AppResult<Vec<CliPathConfig>> = list_cli_path_configs;
        let _: fn(CliKind, &str, Option<&str>, bool, Option<&str>, Option<&str>) -> AppResult<()> =
            open_in_terminal;
        let _: fn(bool, Option<&str>) -> AppResult<crate::context_menu::ContextMenuOutcome> =
            crate::context_menu::register;
        let _: fn() -> AppResult<crate::context_menu::ContextMenuOutcome> =
            crate::context_menu::unregister;
        let _: fn() -> AppResult<bool> = crate::context_menu::is_registered;
    }

    /// 启动参数与改动前逐项一致 —— 这是本期「零行为变更」的判据。
    ///
    /// 期望值是改动前 `CliKind::launch_arguments` 的逐字输出，含参数顺序：
    /// Claude 的 `--resume <id>` 在最前、Codex 的 `resume <id>` 排在权限开关之后、
    /// Gemini 用短开关 `-y`、Antigravity 用 `--conversation <id>` 且权限开关在后。
    ///
    /// 不覆盖 `dsh`：它没有任何启动能力，`launch_plan_for` 对它一律返回
    /// `cli.unsupported_kind`。那是**有意删除**而非遗漏 —— 改动前那段
    /// `--profile tui --resume <id>` 与能力位互相矛盾，命令行启动从未有过可用实现。
    /// 该契约由 `features` 的 `dsh_launch_is_explicitly_unsupported_on_both_paths` 钉住。
    #[test]
    fn launch_args_match_the_previous_implementation() {
        let cases: [(CliKind, Option<&str>, bool, Option<&str>, &[&str]); 9] = [
            (CliKind::Claude, Some("sid"), false, None, &["--resume", "sid"]),
            (
                CliKind::Claude,
                Some("sid"),
                true,
                Some("/tmp/s.json"),
                &["--resume", "sid", "--dangerously-skip-permissions", "--settings", "/tmp/s.json"],
            ),
            (CliKind::Claude, None, true, None, &["--dangerously-skip-permissions"]),
            (
                CliKind::Codex,
                Some("sid"),
                true,
                None,
                &["--dangerously-bypass-approvals-and-sandbox", "resume", "sid"],
            ),
            // Codex 没有设置文件参数：传了也必须逐字丢弃，否则启动会因为未知参数失败。
            (CliKind::Codex, None, false, Some("/tmp/s.json"), &[]),
            (CliKind::Gemini, Some("sid"), true, None, &["-y", "--resume", "sid"]),
            (CliKind::Gemini, None, true, Some("/tmp/s.json"), &["-y"]),
            (
                CliKind::Antigravity,
                Some("sid"),
                true,
                None,
                &["--conversation", "sid", "--dangerously-skip-permissions"],
            ),
            (CliKind::Antigravity, None, true, None, &["--dangerously-skip-permissions"]),
        ];

        for (kind, session_id, skip_permissions, settings_file, expected) in cases {
            let plan = launch_plan_for(kind, session_id, skip_permissions, settings_file)
                .unwrap_or_else(|e| panic!("{kind:?} 应当给出启动计划: {e:?}"));
            match plan {
                LaunchPlan::CommandLine { args } => {
                    let actual: Vec<&str> = args.iter().map(String::as_str).collect();
                    assert_eq!(actual.as_slice(), expected, "{kind:?} 的参数与改动前不一致");
                }
                other => panic!("{kind:?} 应当是命令行形态，实际 {other:?}"),
            }
        }
    }

    /// 复制命令对桌面应用型 CLI 必须给出可直接粘贴执行的深链命令；
    /// 会话 ID 非法时回退到按应用名打开，而不是把应用二进制当成终端命令拼出来。
    #[test]
    fn copy_command_renders_deep_link_for_desktop_apps() {
        let resumed = build_launch_command_string(
            CliKind::WorkBuddy,
            "/tmp",
            Some("abc-123"),
            false,
            None,
            None,
        )
        .expect("深链计划应当渲染出命令");
        assert_eq!(resumed, "open \"workbuddy://chat/abc-123\"");

        let app_root = build_launch_command_string(CliKind::WorkBuddy, "/tmp", None, false, None, None)
            .expect("缺会话 ID 时落到应用根深链");
        assert_eq!(app_root, "open \"workbuddy://chat\"");

        let invalid = build_launch_command_string(
            CliKind::WorkBuddy,
            "/tmp",
            Some("a/b"),
            false,
            None,
            None,
        )
        .expect("有恢复能力就必须回退渲染，而不是报错");
        assert_eq!(invalid, "open -a WorkBuddy");
    }

    /// 两个启动能力都没有的 CLI 必须报错，而不是渲染出空参数的裸命令行 ——
    /// 那会把「复制恢复命令」变成「复制一条新开会话的命令」，且全程不报错。
    #[test]
    fn copy_command_rejects_clis_without_launch_capability() {
        for session_id in [None, Some("11111111-2222-3333-4444-555555555555")] {
            let result =
                build_launch_command_string(CliKind::Dsh, "/tmp", session_id, false, None, None);
            let error = result.expect_err("无启动能力时不该渲染出命令");
            // 必须是带 code 的形态：压成裸串会让前端只能显示无法本地化的兜底文案。
            match error {
                AppError::Coded { code, .. } => assert_eq!(code, "cli.unsupported_kind"),
                other => panic!("应当是带 code 的错误，实际 {other:?}"),
            }
        }
    }

    /// 变体清单无重复，且 `id()` 两两不同。
    ///
    /// `ALL` 是列表与路由遍历的唯一入口，`id()` 是反查的唯一键：任一重复都会让某个
    /// CLI 在列表或反查里被另一个顶掉，而全程不报错。变体数很小，直接两两比较即可。
    #[test]
    fn kind_list_and_ids_are_unique() {
        let mut ids: Vec<&str> = CliKind::ALL.iter().map(|kind| kind.id()).collect();
        let before = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), before, "id() 有重复");

        for (i, a) in CliKind::ALL.iter().enumerate() {
            for b in &CliKind::ALL[i + 1..] {
                assert_ne!(a, b, "CliKind::ALL 有重复变体");
            }
        }
    }

    /// 目录分段必须与旧实现的硬编码结果逐一相等。
    ///
    /// 期望值由完整路径拼接而成，逐段钉死为字面量：只比末级分量会让
    /// 「Antigravity 少了一级 `.gemini`」这类错误照绿通过。
    ///
    /// `sessions_dir` 的基准是用户配置的数据目录（`data_dir`），而另有测试会临时写入
    /// `dsh` / `codex` 的覆盖并在结束时还原。故断言分两段：**基准那一级可能与主目录不同**，
    /// 此时仍逐段校验会话子目录；其余四个 CLI 无覆盖写入，直接与主目录拼接的期望值全等比对。
    #[test]
    fn derived_dirs_match_expected_segments() {
        let home = dirs::home_dir().expect("测试环境必须有主目录");

        assert_eq!(
            default_data_dir(CliKind::Antigravity).unwrap(),
            home.join(".gemini").join("antigravity-cli")
        );

        let cases: [(CliKind, PathBuf, &str); 6] = [
            (CliKind::Claude, home.join(".claude"), "projects"),
            (CliKind::Codex, home.join(".codex"), "sessions"),
            (CliKind::Gemini, home.join(".gemini"), "tmp"),
            (CliKind::WorkBuddy, home.join(".workbuddy"), "projects"),
            (CliKind::Dsh, home.join(".dsh"), "sessions"),
            (
                CliKind::Antigravity,
                home.join(".gemini").join("antigravity-cli"),
                "brain",
            ),
        ];

        for (kind, expected_default_dir, sessions_subdir) in cases {
            assert_eq!(default_data_dir(kind).unwrap(), expected_default_dir, "{kind:?} 的数据根");

            let sessions = sessions_dir(kind).unwrap();
            assert_eq!(
                sessions.file_name().and_then(|name| name.to_str()),
                Some(sessions_subdir),
                "{kind:?} 的会话子目录"
            );
            if kind != CliKind::Dsh && kind != CliKind::Codex {
                assert_eq!(sessions, expected_default_dir.join(sessions_subdir), "{kind:?} 的会话目录");
            }
        }
    }
}
