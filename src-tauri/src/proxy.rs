use crate::cli::{self, CliKind};
use crate::cli_config::{
    read_json_file, read_toml_file, settings_path_for, write_atomically, write_json_file,
    write_toml_file,
};
use crate::error::{AppError, AppResult};
use crate::paths::app_data_dir;
use crate::{history, parser, session};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{LazyLock, Mutex};

// Track subprocess PIDs per CLI — used to kill proxy on disable/app exit
static SUBPROCESS_PIDS: LazyLock<Mutex<HashMap<String, u32>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

// ─── Types ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyState {
    pub enabled: bool,
    pub port: u16,
    pub ws_port: u16,
    pub original_base_url: String,
    pub target_url: String,
    #[serde(default = "default_cli_id")]
    pub cli_id: String,
}

fn default_cli_id() -> String {
    "claude".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyStatus {
    pub enabled: bool,
    pub running: bool,
    pub port: u16,
    pub ws_port: u16,
    pub target_url: String,
    pub cli_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrafficSummary {
    pub id: String,
    pub timestamp: String,
    pub method: String,
    pub path: String,
    pub req_size: i64,
    pub status: Option<i32>,
    pub res_size: i64,
    pub duration_ms: i64,
    /// 从 SSE 响应体解析的 token usage（非 SSE/解析失败为 None）
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub cache_read_tokens: Option<i64>,
    pub cache_creation_tokens: Option<i64>,
    /// 请求体携带的工具名列表（$.tools，Anthropic/OpenAI 双格式）
    pub tool_names: Vec<String>,
    /// 请求上下文中是否包含 Skill 工具的调用记录
    pub has_skill_call: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrafficDetail {
    pub id: String,
    pub timestamp: String,
    pub method: String,
    pub path: String,
    pub req_headers: Option<String>,
    pub req_body: Option<String>,
    pub req_size: i64,
    pub status: Option<i32>,
    pub res_headers: Option<String>,
    pub res_body: Option<String>,
    /// 上游强制压缩（且本进程未启用解压特性）时的 `content-encoding`；此时 `res_body` 为空。
    /// 提示句由前端按此字段渲染——后端不写中文标记进正文（否则语言被定死在库里）。
    pub compression: Option<String>,
    /// 我们自己的包装句所对应的**事实**（今天只有 `"upstream"`）。与 `compression` 同一条
    /// 原则：后端只回事实，提示句由前端按当前语言渲染；`res_body` 装的是第三方原文。
    pub error_kind: Option<String>,
    pub res_size: i64,
    pub duration_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrafficList {
    pub items: Vec<TrafficSummary>,
    pub total: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClearResult {
    pub deleted: u64,
    pub db_size: u64,
}

/// 会话在流量面板里的显示名。**不是文案**——前端据此渲染本地化标签，
/// 故这里只回结构化数据，不回拼好的句子（拼出来就把语言定死在返回值里）。
///
/// 会话 id 的截断由前端做：`Session` 只带原始 id，展示长度是前端的事，
/// 与流量面板里其它显示会话 id 的地方共用同一处截断长度。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TrafficSessionLabel {
    /// 会话列表索引解析出的展示名（用户重命名 / 原生标题 / 首条消息）：
    /// **是用户数据不是文案**，前端原样显示。
    Named { title: String },
    /// 索引与 history 都没有这个会话（解析不出标题）：前端渲染本地化标签 + 截断后的 id。
    Session { session_id: String },
    /// 没有归属会话的请求桶（`session_id IS NULL`）。
    Ungrouped,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionTrafficSummary {
    pub session_id: Option<String>,
    pub display_name: TrafficSessionLabel,
    /// `None` = 这个会话没有项目路径（不是空串）。前端据此隐藏该行，不渲染占位文案。
    pub project_path: Option<String>,
    pub request_count: u64,
    pub total_req_size: i64,
    pub total_res_size: i64,
    pub total_duration_ms: i64,
    pub first_timestamp: String,
    pub last_timestamp: String,
    pub ok_count: u64,
    pub error_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionTrafficList {
    pub items: Vec<SessionTrafficSummary>,
    pub total: u64,
}

// ─── Path helpers ────────────────────────────────────────────────────

/// DSH 代理本期及整个实施计划均不支持:所有 Dsh 代理分支均返回 Err。
/// 此端口仅是满足 match 穷尽性的占位值,实际不会被读取(永不生效)。
const DSH_PROXY_UNUSED_PORT: u16 = 18088;

pub fn default_port_for(kind: CliKind) -> u16 {
    match kind {
        CliKind::Claude => 18080,
        CliKind::Codex => 18082,
        CliKind::Gemini => 18084,
        CliKind::WorkBuddy => 18086,
        // DSH 代理暂不支持,占位端口,实际永不生效
        CliKind::Dsh => DSH_PROXY_UNUSED_PORT,
        CliKind::Antigravity => DSH_PROXY_UNUSED_PORT,
    }
}

fn default_ws_port_for(kind: CliKind) -> u16 {
    default_port_for(kind) + 1
}

/// 创建目录失败的统一形状：底层 OS 文本进 `params.detail`（R3），包装文案由语言包提供。
/// `create_dir_all` 对已存在的目录返回 `Ok`，故这里的失败都是真失败。
fn run_dir() -> AppResult<PathBuf> {
    let dir = app_data_dir()?.join("run");
    fs::create_dir_all(&dir)
        .map_err(|e| AppError::coded("proxy.run_dir_create_failed").with("detail", e.to_string()))?;
    Ok(dir)
}

fn proxy_state_path(kind: CliKind) -> AppResult<PathBuf> {
    Ok(run_dir()?.join(format!("proxy_state_{}.json", kind.id())))
}

fn traffic_db_path() -> AppResult<PathBuf> {
    let dir = app_data_dir()?.join("data");
    fs::create_dir_all(&dir).map_err(|e| {
        AppError::coded("proxy.data_dir_create_failed").with("detail", e.to_string())
    })?;
    Ok(dir.join("traffic.db"))
}

fn proxy_log_dir() -> AppResult<PathBuf> {
    let dir = app_data_dir()?.join("logs");
    fs::create_dir_all(&dir).map_err(|e| {
        AppError::coded("proxy.log_dir_create_failed").with("detail", e.to_string())
    })?;
    Ok(dir)
}

fn proxy_pid_path(kind: CliKind) -> AppResult<PathBuf> {
    Ok(run_dir()?.join(format!("proxy_{}.pid", kind.id())))
}

fn parse_pid_file_content(content: &str) -> Option<u32> {
    content.trim().parse::<u32>().ok().filter(|pid| *pid > 0)
}

fn read_proxy_pid_file(kind: CliKind) -> Option<u32> {
    let path = proxy_pid_path(kind).ok()?;
    let content = fs::read_to_string(path).ok()?;
    parse_pid_file_content(&content)
}

fn write_proxy_pid_file(kind: CliKind, pid: u32) {
    match proxy_pid_path(kind) {
        Ok(path) => {
            if let Err(err) = fs::write(&path, format!("{}\n", pid)) {
                tracing::warn!("Failed to write proxy pid file {}: {}", path.display(), err);
            }
        }
        Err(err) => tracing::warn!("Failed to resolve proxy pid file: {}", err.diagnostic()),
    }
}

fn remove_proxy_pid_file(kind: CliKind) {
    if let Ok(path) = proxy_pid_path(kind) {
        if let Err(err) = fs::remove_file(&path) {
            if err.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!("Failed to remove proxy pid file {}: {}", path.display(), err);
            }
        }
    }
}

fn command_name_looks_like_proxy(command: &str) -> bool {
    Path::new(command)
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| name == "seshbuddy-proxy" || name == "seshbuddy-proxy.exe")
        .unwrap_or(false)
}

#[cfg(target_os = "linux")]
fn process_command_name(pid: u32) -> Option<String> {
    fs::read_to_string(format!("/proc/{}/comm", pid))
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[cfg(all(unix, not(target_os = "linux")))]
fn process_command_name(pid: u32) -> Option<String> {
    Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "comm="])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|value| !value.is_empty())
}

#[cfg(windows)]
fn process_command_name(pid: u32) -> Option<String> {
    win_command("tasklist")
        .args(["/FI", &format!("PID eq {}", pid), "/FO", "CSV", "/NH"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|value| !value.is_empty())
}

#[cfg(unix)]
fn is_process_alive(pid: u32) -> bool {
    pid > 0 && unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
}

#[cfg(windows)]
fn is_process_alive(pid: u32) -> bool {
    pid > 0
        && win_command("tasklist")
            .args(["/FI", &format!("PID eq {}", pid)])
            .output()
            .map(|output| {
                output.status.success()
                    && String::from_utf8_lossy(&output.stdout).contains(&pid.to_string())
            })
            .unwrap_or(false)
}

fn is_proxy_process_alive(pid: u32) -> bool {
    if !is_process_alive(pid) {
        return false;
    }
    process_command_name(pid)
        .as_deref()
        .map(command_name_looks_like_proxy)
        .unwrap_or(false)
}

fn terminate_process_best_effort(pid: u32) {
    if !is_proxy_process_alive(pid) {
        return;
    }
    #[cfg(unix)]
    unsafe {
        libc::kill(pid as libc::pid_t, libc::SIGTERM);
    }
    #[cfg(windows)]
    {
        let _ = win_command("taskkill")
            .args(["/PID", &pid.to_string(), "/F"])
            .output();
    }
}

#[cfg(test)]
mod proxy_pid_tests {
    use super::*;

    #[test]
    fn parse_pid_file_content_accepts_positive_pid() {
        assert_eq!(parse_pid_file_content("12345\n"), Some(12345));
    }

    #[test]
    fn parse_pid_file_content_rejects_invalid_pid() {
        assert_eq!(parse_pid_file_content("not-a-pid"), None);
        assert_eq!(parse_pid_file_content("0"), None);
        assert_eq!(parse_pid_file_content("-1"), None);
    }

    #[test]
    fn command_name_match_only_accepts_seshbuddy_proxy_processes() {
        assert!(command_name_looks_like_proxy("/Applications/SeshBuddy.app/Contents/MacOS/seshbuddy-proxy"));
        assert!(command_name_looks_like_proxy("seshbuddy-proxy.exe"));
        assert!(!command_name_looks_like_proxy("/bin/sleep"));
        assert!(!command_name_looks_like_proxy("seshbuddy"));
    }

    #[test]
    fn extract_sse_usage_reads_anthropic_stream() {
        let body = "event: message_start\n\
                    data: {\"message\":{\"usage\":{\"input_tokens\":87,\"cache_read_input_tokens\":381568,\"cache_creation_input_tokens\":0,\"output_tokens\":0}}}\n\
                    event: content_block_delta\n\
                    data: {\"delta\":{\"text\":\"你\"}}\n\
                    event: message_delta\n\
                    data: {\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":79}}\n";
        let (i, o, cr, cc) = extract_sse_usage(body);
        assert_eq!(i, Some(87));
        assert_eq!(o, Some(79));
        assert_eq!(cr, Some(381568));
        assert_eq!(cc, Some(0));
    }

    #[test]
    fn extract_sse_usage_reads_openai_stream() {
        let body = "data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n\
                    data: {\"choices\":[],\"usage\":{\"prompt_tokens\":120,\"completion_tokens\":30}}\n";
        let (i, o, cr, cc) = extract_sse_usage(body);
        assert_eq!(i, Some(120));
        assert_eq!(o, Some(30));
        assert_eq!(cr, None);
        assert_eq!(cc, None);
    }

    #[test]
    fn extract_sse_usage_reads_codex_responses_stream() {
        let body = "data: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":500,\"output_tokens\":64,\"input_tokens_details\":{\"cached_tokens\":200}}}}\n";
        let (i, o, cr, _cc) = extract_sse_usage(body);
        assert_eq!(i, Some(500));
        assert_eq!(o, Some(64));
        assert_eq!(cr, Some(200));
    }

    #[test]
    fn extract_sse_usage_tolerates_non_sse_body() {
        assert_eq!(
            extract_sse_usage("{\"usage\": \"not-sse-json-per-line\"}"),
            (None, None, None, None)
        );
    }
}

/// 返回 `AppResult`：这条错误经 `assistant/commands.rs` 上屏，压成 `String` 会退化成裸 code。
fn proxy_binary_path() -> AppResult<PathBuf> {
    #[cfg(target_os = "windows")]
    let binary_name = "seshbuddy-proxy.exe";
    #[cfg(not(target_os = "windows"))]
    let binary_name = "seshbuddy-proxy";

    // Development: use cargo target directory
    let dev_path = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join(binary_name)));

    if let Some(ref path) = dev_path {
        if path.exists() {
            return Ok(path.clone());
        }
    }

    // Try workspace target/debug
    let workspace_debug = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(|p| p.join("target").join("debug").join(binary_name));

    if let Some(ref path) = workspace_debug {
        if path.exists() {
            return Ok(path.clone());
        }
    }

    Err(AppError::coded("proxy.binary_missing"))
}

/// 供 assistant 模块复用的 proxy 二进制路径解析（proxy_binary_path 的 pub(crate) 包装）
pub(crate) fn proxy_binary_path_pub() -> AppResult<PathBuf> {
    proxy_binary_path()
}

// ─── State management ────────────────────────────────────────────────

fn read_proxy_state(kind: CliKind) -> AppResult<Option<ProxyState>> {
    read_proxy_state_file(&proxy_state_path(kind)?)
}

fn read_proxy_state_file(path: &Path) -> AppResult<Option<ProxyState>> {
    if !path.exists() {
        return Ok(None);
    }
    let content = fs::read_to_string(path).map_err(|e| {
        AppError::coded("proxy.state_read_failed").with("detail", e.to_string())
    })?;
    let state: ProxyState = serde_json::from_str(&content).map_err(|e| {
        AppError::coded("proxy.state_parse_failed").with("detail", e.to_string())
    })?;
    Ok(Some(state))
}

/// 返回 `AppResult`：`write_atomically` 的失败是 `AppError::Coded`（`internal.io`），
/// 压成 `String` 会退化成裸 code 上屏（`Coded` 的 `Display` 就是 code）。
fn write_proxy_state(kind: CliKind, state: &ProxyState) -> AppResult<()> {
    let path = proxy_state_path(kind)?;
    let json = serde_json::to_string_pretty(state)
        .map_err(|e| AppError::coded("proxy.state_serialize_failed").with("detail", e.to_string()))?;
    // 同样先写临时文件再 rename：状态里存着原始 base_url，写到一半崩溃会让用户
    // 既开不了代理、也回不去原始端点
    write_atomically(&path, json.as_bytes())
}

fn activate_proxy(kind: CliKind, state: &ProxyState, proxy_url: &str) -> AppResult<()> {
    write_proxy_state(kind, state)?;
    if let Err(err) = set_base_url_for_cli(kind, proxy_url) {
        let _ = remove_proxy_state(kind);
        return Err(err);
    }
    Ok(())
}

fn remove_proxy_state(kind: CliKind) -> AppResult<()> {
    let path = proxy_state_path(kind)?;
    if path.exists() {
        fs::remove_file(&path).map_err(|e| {
            AppError::coded("proxy.state_remove_failed").with("detail", e.to_string())
        })?;
    }
    Ok(())
}

fn is_port_listening(port: u16) -> bool {
    std::net::TcpStream::connect_timeout(
        &format!("127.0.0.1:{}", port).parse().unwrap(),
        std::time::Duration::from_millis(100),
    )
    .is_ok()
}

#[cfg(target_os = "windows")]
fn win_command(program: &str) -> Command {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    let mut cmd = Command::new(program);
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}

// ─── Settings manipulation ───────────────────────────────────────────

/// Validate that a URL is safe to use in shell commands (no injection characters)
fn validate_url(url: &str) -> AppResult<()> {
    // Must start with http:// or https://
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err(AppError::coded("proxy.url_scheme_invalid").with("url", url));
    }
    // Reject shell metacharacters that could be used for command injection
    const FORBIDDEN: &[char] = &[
        '&', '|', ';', '`', '$', '(', ')', '{', '}', '<', '>', '!', '\n', '\r', '"', '\'', '\\',
    ];
    if let Some(c) = url.chars().find(|c| FORBIDDEN.contains(c)) {
        return Err(AppError::coded("proxy.url_char_invalid")
            .with("char", c.to_string())
            .with("url", url));
    }
    // Reject localhost URLs to prevent proxying internal services
    if let Ok(parsed) = url::Url::parse(url) {
        if let Some(host) = parsed.host() {
            let is_localhost = match host {
                url::Host::Domain("localhost") => true,
                url::Host::Ipv4(ip) => ip.is_loopback(),
                url::Host::Ipv6(ip) => ip.is_loopback(),
                _ => false,
            };
            if is_localhost {
                return Err(AppError::coded("proxy.url_localhost_forbidden").with("url", url));
            }
        }
    }
    Ok(())
}

fn codex_default_base_url() -> String {
    "https://api.openai.com/v1".to_string()
}

fn default_base_url_for(kind: CliKind) -> String {
    match kind {
        CliKind::Claude => "https://api.anthropic.com".to_string(),
        CliKind::Codex => codex_default_base_url(),
        CliKind::Gemini => String::new(),
        CliKind::WorkBuddy => String::new(),
        CliKind::Dsh => String::new(),
        CliKind::Antigravity => String::new(),
    }
}

fn get_base_url_for_cli(kind: CliKind) -> AppResult<String> {
    match kind {
        CliKind::Claude => {
            let settings = read_json_file(&settings_path_for(kind)?, "{}")?;
            Ok(settings
                .get("env")
                .and_then(|env| env.get("ANTHROPIC_BASE_URL"))
                .and_then(|v| v.as_str())
                .unwrap_or("https://api.anthropic.com")
                .to_string())
        }
        CliKind::Codex => {
            let config = read_toml_file(&settings_path_for(kind)?)?;
            let provider_id = config
                .get("model_provider")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .unwrap_or("openai-chat-completions");
            Ok(config
                .get("model_providers")
                .and_then(|v| v.as_table())
                .and_then(|providers| providers.get(provider_id))
                .and_then(|v| v.as_table())
                .and_then(|provider| provider.get("base_url"))
                .and_then(|v| v.as_str())
                .unwrap_or("https://api.openai.com/v1")
                .to_string())
        }
        // 四种 CLI 是**同一句文案**，共用 `proxy.unsupported_kind`，CLI 展示名进 `params.kind`
        // （`CliKind::name()` 是品牌名，四语同值，不是需要本地化的文本）；
        // 拆成四个码会让同一句话在语言包里出现四遍。
        CliKind::Gemini | CliKind::WorkBuddy | CliKind::Dsh | CliKind::Antigravity => {
            Err(AppError::coded("proxy.unsupported_kind").with("kind", kind.name()))
        }
    }
}

fn set_base_url_for_cli(kind: CliKind, url: &str) -> AppResult<()> {
    match kind {
        CliKind::Claude => {
            let path = settings_path_for(kind)?;
            let mut settings = read_json_file(&path, "{}")?;
            let env = settings
                .as_object_mut()
                .ok_or(AppError::coded("proxy.settings_not_object"))?
                .entry("env")
                .or_insert_with(|| serde_json::json!({}));
            env.as_object_mut().ok_or(AppError::coded("proxy.env_not_object"))?.insert(
                "ANTHROPIC_BASE_URL".to_string(),
                serde_json::Value::String(url.to_string()),
            );
            write_json_file(&path, &settings)
        }
        CliKind::Codex => {
            let path = settings_path_for(kind)?;
            let mut config = read_toml_file(&path)?;
            if !config.is_table() {
                config = toml::Value::Table(toml::map::Map::new());
            }
            let root = config.as_table_mut().ok_or(AppError::coded("proxy.codex_config_not_table"))?;
            let provider_id = root
                .get("model_provider")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .unwrap_or("openai-chat-completions")
                .to_string();
            root.insert(
                "model_provider".to_string(),
                toml::Value::String(provider_id.clone()),
            );

            let providers = root
                .entry("model_providers".to_string())
                .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
            if !providers.is_table() {
                *providers = toml::Value::Table(toml::map::Map::new());
            }
            let providers_table = providers
                .as_table_mut()
                .ok_or(AppError::coded("proxy.model_providers_not_table"))?;
            let provider_entry = providers_table
                .entry(provider_id)
                .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
            if !provider_entry.is_table() {
                *provider_entry = toml::Value::Table(toml::map::Map::new());
            }
            let provider_table = provider_entry
                .as_table_mut()
                .ok_or(AppError::coded("proxy.provider_config_not_table"))?;
            provider_table.insert("base_url".to_string(), toml::Value::String(url.to_string()));
            provider_table
                .entry("wire_api".to_string())
                .or_insert_with(|| toml::Value::String("responses".to_string()));
            write_toml_file(&path, &config)
        }
        // 四种 CLI 是**同一句文案**，共用 `proxy.unsupported_kind`，CLI 展示名进 `params.kind`
        // （`CliKind::name()` 是品牌名，四语同值，不是需要本地化的文本）；
        // 拆成四个码会让同一句话在语言包里出现四遍。
        CliKind::Gemini | CliKind::WorkBuddy | CliKind::Dsh | CliKind::Antigravity => {
            Err(AppError::coded("proxy.unsupported_kind").with("kind", kind.name()))
        }
    }
}

fn proxy_status_from_state(state: ProxyState, running: bool) -> ProxyStatus {
    ProxyStatus {
        enabled: state.enabled,
        running,
        port: state.port,
        ws_port: state.ws_port,
        target_url: state.target_url,
        cli_id: state.cli_id,
    }
}

fn default_proxy_status(kind: CliKind) -> ProxyStatus {
    ProxyStatus {
        enabled: false,
        running: false,
        port: default_port_for(kind),
        ws_port: default_ws_port_for(kind),
        target_url: String::new(),
        cli_id: kind.id().to_string(),
    }
}

fn is_proxy_running(state: &ProxyState) -> bool {
    is_port_listening(state.port)
}

/// 端口占用是**两条句子**（API 代理端口 / 调试 WebSocket 端口），故各是一个码。
///
/// 句子里唯一的可变部分是 CLI 展示名，它是品牌名（`CliKind::name()`，四语同值），
/// 因此进 `params.kind` 而不是进语言包；**可本地化的那半句（「API 代理」/「API 调试
/// WebSocket」）留在语言包里**——它是句子的一部分，由后端拼成参数就会把语言定死。
fn ensure_proxy_ports_available(kind: CliKind, port: u16, ws_port: u16) -> AppResult<()> {
    if is_port_listening(port) {
        return Err(AppError::coded("proxy.port_in_use")
            .with("port", port.to_string())
            .with("kind", kind.name()));
    }
    if is_port_listening(ws_port) {
        return Err(AppError::coded("proxy.port_in_use_ws")
            .with("port", ws_port.to_string())
            .with("kind", kind.name()));
    }
    Ok(())
}

pub fn enable(kind: CliKind, port: u16) -> AppResult<ProxyStatus> {
    tracing::info!("Enabling proxy for {} on port {}", kind.id(), port);

    if let Some(state) = read_proxy_state(kind)? {
        if state.enabled && is_proxy_running(&state) {
            return Ok(proxy_status_from_state(state, true));
        }
        cleanup_dead_proxy(kind, &state);
    }

    let ws_port = port + 1;
    ensure_proxy_ports_available(kind, port, ws_port)?;

    let original_base_url = get_base_url_for_cli(kind)?;
    let original_base_url = if original_base_url.starts_with("http://127.0.0.1:") {
        default_base_url_for(kind)
    } else {
        original_base_url
    };
    validate_url(&original_base_url)?;

    let binary = proxy_binary_path()?;
    let db_path = traffic_db_path()?;
    let db_path_arg = db_path
        .to_str()
        .ok_or(AppError::coded("proxy.db_path_not_utf8"))?
        .to_string();
    let listen_addr = format!("127.0.0.1:{}", port);
    let log_dir = proxy_log_dir()?;
    let log_dir_arg = log_dir
        .to_str()
        .ok_or(AppError::coded("proxy.log_dir_not_utf8"))?
        .to_string();

    let state = ProxyState {
        enabled: true,
        port,
        ws_port,
        original_base_url: original_base_url.clone(),
        target_url: original_base_url.clone(),
        cli_id: kind.id().to_string(),
    };

    let proxy_url = format!("http://127.0.0.1:{}", port);
    activate_proxy(kind, &state, &proxy_url)?;

    #[allow(unused_mut)]
    let mut cmd = Command::new(&binary);
    cmd.arg("--listen")
        .arg(&listen_addr)
        .arg("--target")
        .arg(&original_base_url)
        .arg("--ws-port")
        .arg(ws_port.to_string())
        .arg("--db")
        .arg(&db_path_arg)
        .arg("--log-dir")
        .arg(&log_dir_arg)
        .arg("--cli-id")
        .arg(kind.id())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = cmd.spawn().map_err(|e| {
        rollback_proxy(kind, &original_base_url);
        AppError::coded("proxy.subprocess_spawn_failed").with("detail", e.to_string())
    })?;
    let child_pid = child.id();

    if let Ok(mut pids) = SUBPROCESS_PIDS.lock() {
        pids.insert(kind.id().to_string(), child_pid);
    }
    write_proxy_pid_file(kind, child_pid);

    let mut ready = false;
    for _ in 0..100 {
        std::thread::sleep(std::time::Duration::from_millis(100));
        if is_port_listening(port) {
            ready = true;
            break;
        }
        if let Ok(Some(status)) = child.try_wait() {
            rollback_proxy(kind, &original_base_url);
            return Err(AppError::coded("proxy.subprocess_exited_early")
                .with("status", status.to_string()));
        }
    }

    if !ready {
        rollback_proxy(kind, &original_base_url);

        let log_dir = proxy_log_dir().unwrap_or_default();
        let stderr_log = log_dir.join("proxy.log");
        let log_msg = fs::read_to_string(&stderr_log)
            .ok()
            .map(|s| {
                let trimmed = if s.len() > 500 {
                    let start = s.len().saturating_sub(500);
                    // Find safe UTF-8 char boundary
                    let safe_start = s.floor_char_boundary(start);
                    &s[safe_start..]
                } else {
                    &s
                };
                trimmed.trim().to_string()
            })
            .unwrap_or_default();

        // 有日志与没有日志是**两条句子**（没有日志时不该留下一个空悬的「日志:」），
        // 故各是一个码；日志正文是子进程写的 OS/库文本，按 R3 进 `params.log`，不进语言包。
        return Err(if log_msg.is_empty() {
            AppError::coded("proxy.start_timeout")
        } else {
            AppError::coded("proxy.start_timeout_with_log").with("log", log_msg)
        });
    }

    tracing::info!(
        "Proxy started successfully on port {} for {}",
        port,
        kind.id()
    );
    Ok(proxy_status_from_state(state, true))
}

/// Stop proxy and rollback settings on failure
fn rollback_proxy(kind: CliKind, original_base_url: &str) {
    kill_subprocess(kind);
    if let Err(e) = set_base_url_for_cli(kind, original_base_url) {
        tracing::error!("Failed to rollback {} base_url: {}", kind.id(), e.diagnostic());
    }
    if let Err(e) = remove_proxy_state(kind) {
        tracing::error!("Failed to remove {} proxy state: {}", kind.id(), e.diagnostic());
    }
}

/// Kill the subprocess by CLI
fn kill_subprocess(kind: CliKind) {
    let mut pid_to_kill = None;
    if let Ok(mut pid_guard) = SUBPROCESS_PIDS.lock() {
        pid_to_kill = pid_guard.remove(kind.id());
    }

    if pid_to_kill.is_none() {
        pid_to_kill = read_proxy_pid_file(kind);
    }

    if let Some(pid) = pid_to_kill {
        if is_proxy_process_alive(pid) {
            terminate_process_best_effort(pid);
        } else {
            tracing::warn!(
                "Skip terminating pid {} for {} because it is not a seshbuddy-proxy process",
                pid,
                kind.id()
            );
        }
    }
    remove_proxy_pid_file(kind);
}

fn disable_kind(kind: CliKind) -> AppResult<bool> {
    let Some(state) = read_proxy_state(kind)? else {
        return Ok(false);
    };

    kill_subprocess(kind);
    set_base_url_for_cli(kind, &state.original_base_url)?;
    remove_proxy_state(kind)?;
    tracing::info!("Proxy disabled successfully for {}", kind.id());
    Ok(true)
}

pub fn disable(kind: Option<CliKind>) -> AppResult<()> {
    tracing::info!("Disabling proxy");

    match kind {
        Some(kind) => {
            if disable_kind(kind)? {
                Ok(())
            } else {
                Err(AppError::coded("proxy.not_enabled"))
            }
        }
        None => {
            let mut disabled_any = false;
            for cli_kind in CliKind::all() {
                disabled_any |= disable_kind(cli_kind)?;
            }
            if disabled_any {
                Ok(())
            } else {
                Err(AppError::coded("proxy.not_enabled"))
            }
        }
    }
}

fn status_for_kind(kind: CliKind) -> AppResult<ProxyStatus> {
    match read_proxy_state(kind)? {
        Some(state) => Ok(proxy_status_from_state(
            state.clone(),
            is_proxy_running(&state),
        )),
        None => Ok(default_proxy_status(kind)),
    }
}

pub fn status(kind: Option<CliKind>) -> AppResult<ProxyStatus> {
    if let Some(kind) = kind {
        return status_for_kind(kind);
    }

    for cli_kind in CliKind::all() {
        let status = status_for_kind(cli_kind)?;
        if status.enabled || status.running {
            return Ok(status);
        }
    }

    Ok(default_proxy_status(CliKind::Claude))
}

// ─── State consistency check ─────────────────────────────────────────

fn check_consistency_for_kind(kind: CliKind) {
    let state = match read_proxy_state(kind) {
        Ok(Some(s)) => s,
        Ok(None) => return,
        Err(e) => {
            tracing::error!(
                "Proxy consistency: failed to read state for {}: {}",
                kind.id(),
                e.diagnostic()
            );
            return;
        }
    };

    if !state.enabled {
        tracing::warn!(
            "Proxy consistency: disabled state file found for {}, cleaning up",
            kind.id()
        );
        cleanup_dead_proxy(kind, &state);
        return;
    }

    if !is_proxy_running(&state) {
        tracing::warn!(
            "Proxy consistency: proxy port not listening for {}, cleaning up",
            kind.id()
        );
        cleanup_dead_proxy(kind, &state);
    }
}

pub fn check_consistency() {
    tracing::info!("Checking proxy state consistency");
    for kind in CliKind::all() {
        check_consistency_for_kind(kind);
    }
}

pub fn cleanup_stale_proxy_processes() {
    for kind in CliKind::all() {
        let Some(pid) = read_proxy_pid_file(kind) else {
            continue;
        };

        if is_proxy_process_alive(pid) {
            tracing::warn!(
                "Found stale proxy process {} for {}, terminating before startup",
                pid,
                kind.id()
            );
            terminate_process_best_effort(pid);
        } else if is_process_alive(pid) {
            tracing::warn!(
                "Stale proxy pid file for {} points to non-proxy process {}, removing pid file only",
                kind.id(),
                pid
            );
        }
        remove_proxy_pid_file(kind);
    }
}

fn cleanup_dead_proxy(kind: CliKind, state: &ProxyState) {
    kill_subprocess(kind);
    if let Err(e) = set_base_url_for_cli(kind, &state.original_base_url) {
        tracing::error!("Failed to restore {} base_url in cleanup: {}", kind.id(), e.diagnostic());
    }
    if let Err(e) = remove_proxy_state(kind) {
        tracing::error!("Failed to remove {} proxy state in cleanup: {}", kind.id(), e.diagnostic());
    }
}

/// Called from lib.rs on app exit to clean up proxy subprocesses
pub fn cleanup_on_exit() {
    for kind in CliKind::all() {
        if let Ok(Some(state)) = read_proxy_state(kind) {
            if state.enabled {
                tracing::info!("App exiting, cleaning up proxy for {}", kind.id());
                let _ = disable(Some(kind));
            }
        }
    }
}

// ─── Traffic queries (read from SQLite) ──────────────────────────────

/// 打开流量数据库；文件尚未创建（代理还没跑过）时返回 `proxy.traffic_db_missing`。
///
/// 返回 `AppResult` 而不是 `Result<_, String>`：这条「还没跑过」是**跨进程协议状态**，
/// 前端要按 code 判（`useProxy.ts` 据此不弹错误）。压成 `String` 会让 `Coded` 退化成
/// 裸 code（`Display` 就是 code），前端再也拿不到结构化字段。
fn open_traffic_db() -> AppResult<Connection> {
    let path = traffic_db_path()?;
    if !path.exists() {
        return Err(AppError::coded("proxy.traffic_db_missing"));
    }
    let conn = Connection::open(&path).map_err(|e| {
        AppError::coded("proxy.db_open_failed").with("detail", e.to_string())
    })?;
    // Allow up to 5s for locks held by the proxy subprocess (important on Windows)
    conn.busy_timeout(std::time::Duration::from_secs(5))
        .map_err(|e| AppError::coded("proxy.db_busy_timeout_failed").with("detail", e.to_string()))?;
    // WAL mode allows concurrent reads while proxy is writing
    conn.execute_batch("PRAGMA journal_mode=WAL;")
        .map_err(|e| AppError::coded("proxy.db_wal_failed").with("detail", e.to_string()))?;
    // 建表归侧车（`src-tauri-proxy/src/storage.rs` 的 `Storage::new`），这里只补列：
    // 侧车还没以新版本跑过一次时，旧库没有 `compression` / `error_kind` 列，下面的 SELECT 会整条失败。
    // 幂等（列已存在即报 duplicate column，忽略），未发布版本无迁移包袱。
    let _ = conn.execute_batch("ALTER TABLE traffic ADD COLUMN compression TEXT;");
    // ⚠️ 这条补列会把侧车的列名改动**吞掉**：侧车若把该列改名，这里会成功补出一个全 NULL 的
    // `error_kind`，下面的 SELECT 照常返回 —— 不报错、不 panic，只是值没了（前端 `errorNotice`
    // 静默不出提示句）。改列名时侧车的 DDL/INSERT 与本文件的列清单必须一起改。
    let _ = conn.execute_batch("ALTER TABLE traffic ADD COLUMN error_kind TEXT;");
    Ok(conn)
}

/// `TrafficDetail` 从 `traffic` 表读出的列清单，**顺序即 `map_traffic_detail_row` 里 `row.get(N)` 的下标**。
///
/// 抽成常量是为了让两个 SELECT（`get_detail` 与 `find_traffic_by_timestamp`）与测试共用同一份列清单：
/// 抄副本的话三处会各自漂移。**这里列序与结构体字段序不同**（字段序是
/// `… res_body, compression, error_kind, res_size, duration_ms`），绑定全靠下标；两个同类型字段
/// 之间错位时编译器与运行时都不会响，只会静默换值。
const TRAFFIC_DETAIL_COLUMNS: &str = "id, timestamp, method, path, req_headers, req_body, \
     req_size, status, res_headers, res_body, res_size, duration_ms, compression, error_kind";

/// 按 `TRAFFIC_DETAIL_COLUMNS` 的列序把一行读成 `TrafficDetail`。
fn map_traffic_detail_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<TrafficDetail> {
    Ok(TrafficDetail {
        id: row.get(0)?,
        timestamp: row.get(1)?,
        method: row.get(2)?,
        path: row.get(3)?,
        req_headers: row.get(4)?,
        req_body: row.get(5)?,
        req_size: row.get(6)?,
        status: row.get(7)?,
        res_headers: row.get(8)?,
        res_body: row.get(9)?,
        res_size: row.get(10)?,
        duration_ms: row.get(11)?,
        compression: row.get(12)?,
        error_kind: row.get(13)?,
    })
}

pub fn get_detail(id: String) -> AppResult<TrafficDetail> {
    let conn = open_traffic_db()?;
    get_detail_from(&conn, &id)
}

/// `get_detail` 的查询体，连接由调用方给：测试用内存库跑**同一份 SQL 与同一个行映射**
/// （经 `TRAFFIC_DETAIL_COLUMNS` + `map_traffic_detail_row`），而不是在测试里抄一份列清单——
/// 抄的那份只证明副本自身对，证明不了生产这条。
fn get_detail_from(conn: &Connection, id: &str) -> AppResult<TrafficDetail> {
    let sql = format!("SELECT {TRAFFIC_DETAIL_COLUMNS} FROM traffic WHERE id = ?1");
    conn.query_row(&sql, params![id], map_traffic_detail_row)
        .map_err(|e| AppError::coded("proxy.detail_query_failed").with("detail", e.to_string()))
}

pub fn clear_traffic(
    before_days: Option<u32>,
    kind: Option<CliKind>,
) -> AppResult<ClearResult> {
    let conn = open_traffic_db()?;

    let deleted = match (before_days, kind) {
        (Some(days), Some(kind)) => {
            let cutoff = chrono::Utc::now() - chrono::Duration::days(days as i64);
            let cutoff_str = cutoff.to_rfc3339();
            conn.execute(
                "DELETE FROM traffic WHERE timestamp < ?1 AND cli_id = ?2",
                params![cutoff_str, kind.id()],
            )
            .map_err(|e| AppError::coded("proxy.clear_failed").with("detail", e.to_string()))?
        }
        (Some(days), None) => {
            let cutoff = chrono::Utc::now() - chrono::Duration::days(days as i64);
            let cutoff_str = cutoff.to_rfc3339();
            conn.execute(
                "DELETE FROM traffic WHERE timestamp < ?1",
                params![cutoff_str],
            )
            .map_err(|e| AppError::coded("proxy.clear_failed").with("detail", e.to_string()))?
        }
        (None, Some(kind)) => conn
            .execute("DELETE FROM traffic WHERE cli_id = ?1", params![kind.id()])
            .map_err(|e| AppError::coded("proxy.clear_failed").with("detail", e.to_string()))?,
        (None, None) => conn
            .execute("DELETE FROM traffic", [])
            .map_err(|e| AppError::coded("proxy.clear_failed").with("detail", e.to_string()))?,
    };

    conn.execute_batch("VACUUM")
        .map_err(|e| AppError::coded("proxy.vacuum_failed").with("detail", e.to_string()))?;

    let db_size = db_size_inner()?;

    Ok(ClearResult {
        deleted: deleted as u64,
        db_size,
    })
}

/// 返回指定会话的所有流量记录时间戳（仅 API 请求，排除 count_tokens 等辅助路径）。
/// 前端用这些时间戳做 120s 近邻匹配，只在有对应流量的消息上显示 ⚡ 按钮。
pub fn session_traffic_timestamps(session_id: &str, kind: CliKind) -> Vec<String> {
    let conn = match open_traffic_db() {
        Ok(c) => c,
        Err(_) => return vec![],
    };
    let path_filter = match kind {
        CliKind::Claude => "(path LIKE '/v1/messages%' AND path NOT LIKE '/v1/messages/count_tokens%')",
        CliKind::Codex => "(path LIKE '/v1/responses%' OR path LIKE '/v1/chat/completions%' OR path LIKE '/chat/completions%' OR path LIKE '/responses%')",
        CliKind::Gemini | CliKind::WorkBuddy | CliKind::Dsh | CliKind::Antigravity => return vec![],
    };
    let sql = format!(
        "SELECT timestamp FROM traffic WHERE session_id = ?1 AND cli_id = ?2 AND {path_filter} ORDER BY timestamp"
    );
    let mut stmt = match conn.prepare(&sql) {
        Ok(s) => s,
        Err(_) => return vec![],
    };
    stmt.query_map(rusqlite::params![session_id, kind.id()], |row| row.get::<_, String>(0))
        .map(|rows| rows.filter_map(|r| r.ok()).collect())
        .unwrap_or_default()
}

pub fn db_size() -> AppResult<u64> {
    db_size_inner()
}

fn db_size_inner() -> AppResult<u64> {
    let path = traffic_db_path()?;
    if !path.exists() {
        return Ok(0);
    }
    let metadata = fs::metadata(&path)
        .map_err(|e| AppError::coded("proxy.db_size_failed").with("detail", e.to_string()))?;
    Ok(metadata.len())
}

// ─── Session-grouped queries ─────────────────────────────────────────

pub fn get_traffic_sessions(
    limit: u32,
    offset: u32,
    kind: Option<CliKind>,
) -> AppResult<SessionTrafficList> {
    let conn = open_traffic_db()?;
    // 三种「缺 cliId」各是一个码：句子里的可变部分是**说明这次查询做什么的中文标签**
    // （「按会话查看代理流量」），它是句子的一部分而不是数据，不能当参数传
    // （后端拼出来就把语言定死了），故折进语言包、一个操作一个码。
    let query_kind = kind.ok_or_else(|| AppError::coded("proxy.sessions_cli_missing"))?;

    let total: u64 = conn
        .query_row(
            "SELECT COUNT(DISTINCT session_id)
             FROM traffic
             WHERE session_id IS NOT NULL
               AND cli_id = ?1",
            params![query_kind.id()],
            |row| row.get(0),
        )
        .map_err(|e| AppError::coded("proxy.count_query_failed").with("detail", e.to_string()))?;

    let mut stmt = conn
        .prepare(
            "SELECT session_id,
                    COUNT(*) as request_count,
                    SUM(req_size) as total_req_size,
                    SUM(res_size) as total_res_size,
                    SUM(duration_ms) as total_duration_ms,
                    MIN(timestamp) as first_timestamp,
                    MAX(timestamp) as last_timestamp,
                    SUM(CASE WHEN status >= 200 AND status < 300 THEN 1 ELSE 0 END) as ok_count,
                    SUM(CASE WHEN status >= 400 THEN 1 ELSE 0 END) as error_count
             FROM traffic
             WHERE session_id IS NOT NULL
               AND cli_id = ?1
             GROUP BY session_id
             ORDER BY MAX(timestamp) DESC
             LIMIT ?2 OFFSET ?3",
        )
        .map_err(|e| AppError::coded("proxy.query_failed").with("detail", e.to_string()))?;

    let rows = stmt
        .query_map(params![query_kind.id(), limit, offset], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, u64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, u64>(7)?,
                row.get::<_, u64>(8)?,
            ))
        })
        .map_err(|e| AppError::coded("proxy.query_failed").with("detail", e.to_string()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| AppError::coded("proxy.row_read_failed").with("detail", e.to_string()))?;

    let (display_map, session_project_map) = load_session_metadata(query_kind)?;

    let mut items: Vec<SessionTrafficSummary> = rows
        .into_iter()
        .map(
            |(
                session_id,
                request_count,
                total_req_size,
                total_res_size,
                total_duration_ms,
                first_timestamp,
                last_timestamp,
                ok_count,
                error_count,
            )| {
                // 查询带 `WHERE session_id IS NOT NULL`（见上方 SQL），故这里必为实值：
                // 不再有「未分组」分支。真正的未关联桶在下方 `offset == 0` 处单独构造。
                let display_name = match display_map.get(&session_id) {
                    Some(title) => TrafficSessionLabel::Named {
                        title: title.clone(),
                    },
                    None => TrafficSessionLabel::Session {
                        session_id: session_id.clone(),
                    },
                };
                let project_path = session_project_map
                    .get(&session_id)
                    .cloned()
                    .flatten();

                SessionTrafficSummary {
                    session_id: Some(session_id),
                    display_name,
                    project_path,
                    request_count,
                    total_req_size,
                    total_res_size,
                    total_duration_ms,
                    first_timestamp,
                    last_timestamp,
                    ok_count,
                    error_count,
                }
            },
        )
        .collect();

    if offset == 0 {
        let null_count: u64 = conn
            .query_row(
                "SELECT COUNT(*)
                 FROM traffic
                 WHERE session_id IS NULL
                   AND cli_id = ?1",
                params![query_kind.id()],
                |row| row.get(0),
            )
            .unwrap_or(0);
        if null_count > 0 {
            let null_row: Result<(i64, i64, i64, i64, String, String, u64, u64), _> = conn
                .query_row(
                    "SELECT SUM(req_size), SUM(res_size), SUM(duration_ms), COUNT(*),
                        MIN(timestamp), MAX(timestamp),
                        SUM(CASE WHEN status >= 200 AND status < 300 THEN 1 ELSE 0 END),
                        SUM(CASE WHEN status >= 400 THEN 1 ELSE 0 END)
                 FROM traffic
                 WHERE session_id IS NULL
                   AND cli_id = ?1",
                    params![query_kind.id()],
                    |row| {
                        Ok((
                            row.get(0)?,
                            row.get(1)?,
                            row.get(2)?,
                            row.get(3)?,
                            row.get(4)?,
                            row.get(5)?,
                            row.get(6)?,
                            row.get(7)?,
                        ))
                    },
                );
            if let Ok((total_req, total_res, total_dur, cnt, first_ts, last_ts, ok_cnt, err_cnt)) =
                null_row
            {
                items.push(SessionTrafficSummary {
                    session_id: None,
                    display_name: TrafficSessionLabel::Ungrouped,
                    project_path: None,
                    request_count: cnt as u64,
                    total_req_size: total_req,
                    total_res_size: total_res,
                    total_duration_ms: total_dur,
                    first_timestamp: first_ts,
                    last_timestamp: last_ts,
                    ok_count: ok_cnt,
                    error_count: err_cnt,
                });
            }
        }
    }

    Ok(SessionTrafficList { items, total })
}

/// 加载会话展示元数据：sid → 展示名（与会话列表同一套 resolve_display_name 解析链）、sid → 项目路径。
fn load_session_metadata(
    kind: CliKind,
) -> AppResult<(HashMap<String, String>, HashMap<String, Option<String>>)> {
    match kind {
        CliKind::Claude | CliKind::Codex => {
            let history_path = cli::history_path(kind)?;
            // history.jsonl 只解析一次，同时拿到 display 兜底 map 与项目路径 map
            // （此前 Claude 分支解析了两遍，sessions 全量拉取时 CPU 翻倍）
            let (history_map, history_project_map) = match kind {
                CliKind::Claude => history::parse_history(history_path.to_str().unwrap_or("")),
                _ => (
                    history::parse_codex_history(history_path.to_str().unwrap_or("")),
                    HashMap::new(),
                ),
            };
            let mut project_map = match kind {
                CliKind::Claude => build_claude_session_project_map(
                    &cli::sessions_dir(kind)?,
                    &history_project_map,
                ),
                _ => build_codex_session_project_map(&cli::sessions_dir(kind)?),
            };

            // 会话列表索引库：自定义重命名 / 原生标题 / 清洗后首条用户消息 / 项目路径
            let records = crate::app_db::read_session_list_index(kind).unwrap_or_default();
            let custom_names = crate::app_db::load_session_names().unwrap_or_default();
            let mut display_map = HashMap::new();
            for record in records.values() {
                let sid = record.session_id.trim();
                if sid.is_empty() {
                    continue;
                }
                let display = crate::title_resolver::resolve_display_name(
                    crate::commands::session::custom_session_name(
                        &custom_names,
                        kind,
                        &record.session_path,
                    ),
                    record.title.as_deref(),
                    record.first_user_message.as_deref(),
                    history_map
                        .get(sid)
                        .map(|h| h.display.as_str())
                        .filter(|v| !v.is_empty()),
                    sid,
                );
                display_map.insert(sid.to_string(), display);
                if let Some(p) = record.project_path.as_ref().filter(|p| !p.trim().is_empty()) {
                    project_map.insert(sid.to_string(), Some(p.clone()));
                }
            }

            // 兜底：对仅在 history_map 中出现（索引库尚未收录）的 session，使用 history display 解析
            for (sid, hist) in &history_map {
                if !display_map.contains_key(sid) {
                    let display = crate::title_resolver::resolve_display_name(
                        None,
                        None,
                        None,
                        Some(hist.display.as_str()).filter(|v| !v.is_empty()),
                        sid,
                    );
                    display_map.insert(sid.clone(), display);
                }
            }

            Ok((display_map, project_map))
        }
        CliKind::Gemini => Ok((HashMap::new(), HashMap::new())),
        CliKind::WorkBuddy => Ok((HashMap::new(), HashMap::new())),
        CliKind::Dsh => Ok((HashMap::new(), HashMap::new())),
        CliKind::Antigravity => Ok((HashMap::new(), HashMap::new())),
    }
}

fn collect_jsonl_files(dir: &Path, files: &mut Vec<PathBuf>) -> AppResult<()> {
    if !dir.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(dir)? {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => continue,
        };
        let path = entry.path();
        if path.is_dir() {
            collect_jsonl_files(&path, files)?;
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("jsonl") {
            files.push(path);
        }
    }
    Ok(())
}

fn build_claude_session_project_map(
    projects_dir: &Path,
    project_map: &HashMap<String, String>,
) -> HashMap<String, Option<String>> {
    let mut map = HashMap::new();
    if let Ok(entries) = fs::read_dir(projects_dir) {
        for entry in entries.flatten() {
            let dir_name = entry.file_name().to_string_lossy().to_string();
            let dir_path = entry.path();
            if !dir_path.is_dir() {
                continue;
            }
            let fallback_project_path =
                session::resolve_project_path(&dir_name, None, Some(project_map));
            if let Ok(files) = fs::read_dir(&dir_path) {
                for file in files.flatten() {
                    let fname = file.file_name().to_string_lossy().to_string();
                    if let Some(session_id) = fname.strip_suffix(".jsonl") {
                        let file_path = file.path();
                        let project_path = file_path
                            .to_str()
                            .and_then(parser::read_project_path)
                            .map(|value| value.trim().to_string())
                            .filter(|value| !value.is_empty())
                            .or_else(|| {
                                session::find_project_path_in_map(&dir_name, project_map)
                                    .map(|value| value.to_string())
                            })
                            .or_else(|| fallback_project_path.clone());
                        map.insert(session_id.to_string(), project_path);
                    }
                }
            }
        }
    }
    map
}

fn build_codex_session_project_map(sessions_dir: &Path) -> HashMap<String, Option<String>> {
    let mut map = HashMap::new();
    let mut files = Vec::new();
    if collect_jsonl_files(sessions_dir, &mut files).is_err() {
        return map;
    }

    for session_path in files {
        let file_path = match session_path.to_str() {
            Some(value) => value,
            None => continue,
        };
        let Some(session_id) = parser::read_session_id(file_path) else {
            continue;
        };
        let project_path = parser::read_project_path(file_path);
        map.insert(session_id, project_path);
    }

    map
}

/// 从 SSE 响应体提取 token usage。
/// 兼容 Anthropic（message_start / message_delta）、OpenAI chat/completions（usage）
/// 与 Codex responses API（response.completed → response.usage）。
fn extract_sse_usage(res_body: &str) -> (Option<i64>, Option<i64>, Option<i64>, Option<i64>) {
    let mut input = None;
    let mut output = None;
    let mut cache_read = None;
    let mut cache_creation = None;
    for line in res_body.lines() {
        let Some(json) = line.strip_prefix("data: ") else {
            continue;
        };
        if !json.contains("usage") {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else {
            continue;
        };
        // Anthropic message_start: .message.usage（输入侧 + cache）
        if let Some(u) = v.get("message").and_then(|m| m.get("usage")) {
            input = u.get("input_tokens").and_then(|x| x.as_i64()).or(input);
            cache_read = u
                .get("cache_read_input_tokens")
                .and_then(|x| x.as_i64())
                .or(cache_read);
            cache_creation = u
                .get("cache_creation_input_tokens")
                .and_then(|x| x.as_i64())
                .or(cache_creation);
        }
        // Anthropic message_delta: 顶层 .usage（output_tokens 累计，取最后一条）
        // OpenAI chat/completions: .usage（prompt/completion_tokens）
        if let Some(u) = v.get("usage") {
            if let Some(o) = u.get("output_tokens").and_then(|x| x.as_i64()) {
                output = Some(o);
            }
            if let Some(i) = u.get("prompt_tokens").and_then(|x| x.as_i64()) {
                input = Some(i);
            }
            if let Some(o) = u.get("completion_tokens").and_then(|x| x.as_i64()) {
                output = Some(o);
            }
        }
        // Codex responses API: response.completed → .response.usage
        if let Some(u) = v.get("response").and_then(|r| r.get("usage")) {
            input = u.get("input_tokens").and_then(|x| x.as_i64()).or(input);
            if let Some(o) = u.get("output_tokens").and_then(|x| x.as_i64()) {
                output = Some(o);
            }
            cache_read = u
                .pointer("/input_tokens_details/cached_tokens")
                .and_then(|x| x.as_i64())
                .or(cache_read);
        }
    }
    (input, output, cache_read, cache_creation)
}

/// 会话内流量列表的全文搜索条件（`:search` 为 NULL 时整条短路）。
///
/// `compression` 一并参与匹配：压缩响应的正文没有落库（`res_body` 是 NULL），
/// 唯一搜得到的东西就是编码名（gzip / br / zstd）。它是**语言无关**的值——
/// 改前那句标记把编码名嵌在中文句子里，搜 `%gzip%` 在任何语言下都命中，
/// 少了这一项就是一次与语言无关的能力回退。
///
/// 抽成常量是为了让测试能对**同一份文本**断言：抄一份副本进测试的话，
/// 两处会各自漂移，而这里正是「删掉一项没人发现」的地方。
const TRAFFIC_SEARCH_PREDICATE: &str = "(:search IS NULL \
     OR req_body LIKE :search ESCAPE '\\' \
     OR res_body LIKE :search ESCAPE '\\' \
     OR compression LIKE :search ESCAPE '\\')";

pub fn get_session_traffic(
    session_id: Option<String>,
    limit: u32,
    offset: u32,
    kind: Option<CliKind>,
    search: Option<String>,
    presets: Option<Vec<String>>,
    tool_names: Option<Vec<String>>,
) -> AppResult<TrafficList> {
    let conn = open_traffic_db()?;
    let query_kind = kind.ok_or_else(|| AppError::coded("proxy.session_cli_missing"))?;

    // LIKE 预转义（ESCAPE '\'），None 时绑定 NULL 使条件短路
    let search_pattern = search
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .map(|s| {
            let escaped = s
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_");
            format!("%{}%", escaped)
        });
    let preset_list = presets.unwrap_or_default();
    let has_tools = preset_list.iter().any(|p| p == "has_tools") as i32;
    let has_system = preset_list.iter().any(|p| p == "has_system") as i32;
    let has_thinking = preset_list.iter().any(|p| p == "has_thinking") as i32;
    let has_tool_result = preset_list.iter().any(|p| p == "has_tool_result") as i32;
    let has_image = preset_list.iter().any(|p| p == "has_image") as i32;
    let has_subagent = preset_list.iter().any(|p| p == "has_subagent") as i32;
    let has_session_ref = preset_list.iter().any(|p| p == "has_session_ref") as i32;

    // 工具名过滤（值来自库内提取，仍做单引号转义防御）；未选择时子句恒真
    let tool_names: Vec<String> = tool_names
        .unwrap_or_default()
        .into_iter()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect();
    let tool_clause = if tool_names.is_empty() {
        "1=1".to_string()
    } else {
        let quoted = tool_names
            .iter()
            .map(|t| format!("'{}'", t.replace('\'', "''")))
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "(json_valid(req_body) AND EXISTS (SELECT 1 FROM json_each(req_body, '$.tools') t \
             WHERE COALESCE(json_extract(t.value, '$.name'), json_extract(t.value, '$.function.name')) IN ({quoted})))"
        )
    };

    // :sid 为 NULL 时匹配未分组记录（session_id IS NULL）
    // json_valid 前置保护：空 body / 非 JSON body（如错误页）会让 JSON 函数抛 malformed JSON
    let where_sql = format!("FROM traffic
         WHERE ((:sid IS NULL AND session_id IS NULL) OR session_id = :sid)
           AND cli_id = :cli
           AND {TRAFFIC_SEARCH_PREDICATE}
           AND (:tools = 0 OR (json_valid(req_body) AND json_array_length(req_body, '$.tools') > 0))
           AND (:system = 0 OR (json_valid(req_body) AND (
                json_type(req_body, '$.system') IS NOT NULL
                OR json_type(req_body, '$.instructions') IS NOT NULL
                OR EXISTS (SELECT 1 FROM json_each(req_body, '$.messages') m
                           WHERE json_extract(m.value, '$.role') = 'system'))))
           AND (:thinking = 0 OR (json_valid(req_body) AND json_type(req_body, '$.thinking') IS NOT NULL))
           AND (:tool_result = 0 OR req_body LIKE '%\"tool_result\"%')
           AND (:image = 0 OR req_body LIKE '%\"type\":\"image\"%' OR req_body LIKE '%\"type\": \"image\"%' OR req_body LIKE '%image_url%')
           AND (:subagent = 0 OR (json_valid(req_body) AND EXISTS (
                SELECT 1 FROM json_each(req_body, '$.tools') t
                WHERE COALESCE(json_extract(t.value, '$.name'), json_extract(t.value, '$.function.name')) = 'Task')))
           AND (:session_ref = 0 OR req_body LIKE '%session_id%')
           AND {tool_clause}"
    );

    let count_sql = format!("SELECT COUNT(*) {where_sql}");
    // tool_names: 单条 json_each 子查询一次解析完成提取（带 json_valid 保护）；
    // has_skill_call: 纯 instr 文本检测，无需 JSON 解析
    // res_body 不整列读入：usage 信息只分布在流的头（Anthropic message_start）与
    // 尾（message_delta / OpenAI 末块 / Codex response.completed），用头 8KB + 尾 256KB
    // 窗口拼接喂给 extract_sse_usage，避免长 SSE 响应（可达数百 MB）整行物化进内存。
    // 头尾之间补换行防止拼出伪行；body 短于窗口时两段重复，重复解析同值幂等无害。
    let query_sql = format!(
        "SELECT id, timestamp, method, path, req_size, status, res_size, duration_ms,
                (substr(res_body, 1, 8192) || char(10) || substr(res_body, -262144)),
                CASE WHEN req_size < 300000 AND json_valid(req_body) AND json_type(req_body, '$.tools') = 'array'
                     THEN (SELECT json_group_array(name) FROM (
                             SELECT DISTINCT COALESCE(json_extract(t.value, '$.name'), json_extract(t.value, '$.function.name')) AS name
                             FROM json_each(req_body, '$.tools') t WHERE name IS NOT NULL))
                     ELSE '[]' END,
                (instr(req_body, '\"name\":\"Skill\"') > 0 OR instr(req_body, '\"name\": \"Skill\"') > 0)
         {where_sql}
         ORDER BY timestamp DESC LIMIT :limit OFFSET :offset"
    );

    let total: u64 = conn
        .query_row(
            &count_sql,
            rusqlite::named_params! {
                ":sid": session_id,
                ":cli": query_kind.id(),
                ":search": search_pattern,
                ":tools": has_tools,
                ":system": has_system,
                ":thinking": has_thinking,
                ":tool_result": has_tool_result,
                ":image": has_image,
                ":subagent": has_subagent,
                ":session_ref": has_session_ref,
            },
            |row| row.get(0),
        )
        .map_err(|e| AppError::coded("proxy.count_query_failed").with("detail", e.to_string()))?;

    let mut stmt = conn
        .prepare(&query_sql)
        .map_err(|e| AppError::coded("proxy.query_failed").with("detail", e.to_string()))?;
    let rows = stmt
        .query_map(
            rusqlite::named_params! {
                ":sid": session_id,
                ":cli": query_kind.id(),
                ":search": search_pattern,
                ":tools": has_tools,
                ":system": has_system,
                ":thinking": has_thinking,
                ":tool_result": has_tool_result,
                ":image": has_image,
                ":subagent": has_subagent,
                ":session_ref": has_session_ref,
                ":limit": limit,
                ":offset": offset,
            },
            |row| {
                let res_body: Option<String> = row.get(8)?;
                let (input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens) =
                    res_body
                        .as_deref()
                        .map(extract_sse_usage)
                        .unwrap_or((None, None, None, None));
                let tool_names_json: String = row.get(9)?;
                let tool_names: Vec<String> =
                    serde_json::from_str(&tool_names_json).unwrap_or_default();
                let has_skill_call: bool = row.get(10)?;
                Ok(TrafficSummary {
                    id: row.get(0)?,
                    timestamp: row.get(1)?,
                    method: row.get(2)?,
                    path: row.get(3)?,
                    req_size: row.get(4)?,
                    status: row.get(5)?,
                    res_size: row.get(6)?,
                    duration_ms: row.get(7)?,
                    input_tokens,
                    output_tokens,
                    cache_read_tokens,
                    cache_creation_tokens,
                    tool_names,
                    has_skill_call,
                })
            },
        )
        .map_err(|e| AppError::coded("proxy.query_failed").with("detail", e.to_string()))?;
    let items = rows
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| AppError::coded("proxy.row_read_failed").with("detail", e.to_string()))?;

    Ok(TrafficList { items, total })
}

/// Find the closest traffic record matching a session_id and timestamp.
/// Used to link a ChatView assistant message to its corresponding API request.
pub fn find_traffic_by_timestamp(
    session_id: &str,
    timestamp: &str,
    kind: Option<CliKind>,
) -> AppResult<Option<TrafficDetail>> {
    let conn = match open_traffic_db() {
        Ok(c) => c,
        Err(_) => return Ok(None),
    };

    let query_kind = kind.ok_or_else(|| AppError::coded("proxy.find_by_timestamp_cli_missing"))?;
    let path_filter = match query_kind {
        CliKind::Claude => "(path LIKE '/v1/messages%' AND path NOT LIKE '/v1/messages/count_tokens%')",
        CliKind::Codex => "(path LIKE '/v1/responses%' OR path LIKE '/v1/chat/completions%' OR path LIKE '/chat/completions%' OR path LIKE '/responses%')",
        CliKind::Gemini => "1=0",
        CliKind::WorkBuddy => "1=0",
        CliKind::Dsh => "1=0",
        CliKind::Antigravity => "1=0",
    };
    let sql = format!(
        "SELECT {TRAFFIC_DETAIL_COLUMNS}
         FROM traffic
         WHERE session_id = ?1
           AND cli_id = ?2
           AND {path_filter}
           AND ABS(julianday(timestamp) - julianday(?3)) < (120.0 / 86400.0)
         ORDER BY ABS(julianday(timestamp) - julianday(?3))
         LIMIT 1"
    );

    let result = conn.query_row(
        &sql,
        params![session_id, query_kind.id(), timestamp],
        map_traffic_detail_row,
    );

    match result {
        Ok(detail) => Ok(Some(detail)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(AppError::coded("proxy.query_failed").with("detail", e.to_string())),
    }
}

#[cfg(test)]
mod load_session_metadata_smoke {
    use super::*;

    #[test]
    fn load_session_metadata_smoke() {
        // 冒烟：真实环境缺 GLOBAL_POOL 时应走容错路径而非 panic
        let r = load_session_metadata(CliKind::Claude);
        assert!(r.is_ok());
    }

    /// 类型级回归守卫：`cli_config.rs` 的 helper 返回 `AppResult`，它们的 `Coded` 错误
    /// 必须一路以 `AppError` 传到 IPC，中途**一次都不许压成 `String`**。
    ///
    /// `impl From<AppError> for String`（**已由计划 11 的 T5 删除**）曾会把 `Coded` 退化成
    /// 裸 code（`Display` 就是 code），而那条退化路径**不报错、不 panic**。T5 之后，单把签名
    /// 改回 `Result<_, String>` 已经会因函数体里的 `?` 而编译不过；但**连函数体一起回退**
    /// 不碰任何 `AppError`，编译器看不见。另外 `commands/proxy.rs` 的
    /// `.map_err(AppError::from)` 曾靠反向的 `From<String> for AppError` 对
    /// `Result<_, String>` 与 `AppResult` 都编译得过，**那条 impl 已由计划 13 的 T2 删除**；
    /// 一旦回退漏过编译器，前端就会从 `{code, params}` 变成用户直接看到
    /// `internal.io`。所以这里把签名钉死在类型上——把任一环改回 `Result<_, String>`，
    /// 本测试**编译不过**，而不是静默放行。
    ///
    /// 只钉类型，不跑逻辑：`enable` 会真的拉起子进程，不适合当单测。
    #[test]
    fn cli_config_errors_reach_ipc_as_app_error() {
        let _: fn(CliKind, &ProxyState) -> AppResult<()> = write_proxy_state;
        let _: fn(CliKind, &ProxyState, &str) -> AppResult<()> = activate_proxy;
        let _: fn(CliKind) -> AppResult<String> = get_base_url_for_cli;
        let _: fn(CliKind, &str) -> AppResult<()> = set_base_url_for_cli;
        let _: fn(CliKind, u16) -> AppResult<ProxyStatus> = enable;
        let _: fn(Option<CliKind>) -> AppResult<()> = disable;
    }

    /// 上一条守卫的续集：B3 把本文件剩下的 54 个错误站点也迁成 `coded()`，**连带**把 17 个
    /// `Result<_, String>` 函数加宽成 `AppResult`——不加宽就没有一处装得下 `Coded`，
    /// `?` 会在链上换一处继续压平，而那条路径静默无测试（同上一条的说明）。
    ///
    /// 这里把加宽过的签名全部钉死。它们大多是**私有**的，编译器只在文件内看得见调用点。
    /// T5 删掉 `From<AppError> for String` 之后，「只改签名、函数体不动」已经会因函数体里的
    /// `?` 而编译不过（**实测**：把 `app_data_dir` 改回 `Result<PathBuf, String>`，报
    /// `E0277` + 本测试的 `E0308`）；**但连签名带函数体一起回退**编译器看不见，
    /// 用户看到的仍是裸 code。本测试让那种改动**编译不过**。
    #[test]
    fn proxy_error_sites_reach_ipc_as_app_error() {
        let _: fn() -> AppResult<PathBuf> = app_data_dir;
        let _: fn() -> AppResult<PathBuf> = run_dir;
        let _: fn() -> AppResult<PathBuf> = traffic_db_path;
        let _: fn() -> AppResult<PathBuf> = proxy_log_dir;
        let _: fn(CliKind) -> AppResult<PathBuf> = proxy_state_path;
        let _: fn(CliKind) -> AppResult<PathBuf> = proxy_pid_path;
        let _: fn() -> AppResult<PathBuf> = proxy_binary_path;
        let _: fn() -> AppResult<PathBuf> = proxy_binary_path_pub;
        let _: fn(CliKind) -> AppResult<Option<ProxyState>> = read_proxy_state;
        let _: fn(&Path) -> AppResult<Option<ProxyState>> = read_proxy_state_file;
        let _: fn(CliKind) -> AppResult<()> = remove_proxy_state;
        let _: fn(&str) -> AppResult<()> = validate_url;
        let _: fn(CliKind, u16, u16) -> AppResult<()> = ensure_proxy_ports_available;
        let _: fn(CliKind) -> AppResult<ProxyStatus> = status_for_kind;
        let _: fn(Option<CliKind>) -> AppResult<ProxyStatus> = status;
        let _: fn() -> AppResult<u64> = db_size;
        let _: fn() -> AppResult<u64> = db_size_inner;
        // 本批加宽的私有助手：错误被调用方的 `.is_err()` 丢掉。退回 `Result<_, String>`
        // 现在编译不过（**实测**：报 1 × `E0277` + 本守卫的 `E0308`）；**连函数体一起回退**
        // 编译器看不见，故仍钉在类型上。
        let _: fn(&Path, &mut Vec<PathBuf>) -> AppResult<()> = collect_jsonl_files;
    }

    /// 跨进程契约：`TrafficSessionLabel` 的 JSON 形状就是前端可辨识联合的判据
    /// （`useProxy.ts` 的 `TrafficSessionLabel` 按 `kind` 分派，`SessionTrafficSummary`
    /// 的 `display_name` 直接喂给它）。改字段名或 `rename_all` 不会让任何一边编译失败，
    /// 只会让前端静默拿到 `undefined` 并渲染出空标签，故在这里钉死。
    #[test]
    fn traffic_session_label_wire_shape() {
        let wire = |label: TrafficSessionLabel| serde_json::to_string(&label).unwrap();
        assert_eq!(
            wire(TrafficSessionLabel::Named {
                title: "T".to_string()
            }),
            r#"{"kind":"named","title":"T"}"#
        );
        assert_eq!(
            wire(TrafficSessionLabel::Session {
                session_id: "s1".to_string()
            }),
            r#"{"kind":"session","session_id":"s1"}"#
        );
        assert_eq!(
            wire(TrafficSessionLabel::Ungrouped),
            r#"{"kind":"ungrouped"}"#
        );
    }

    /// 压缩响应在会话内搜索里**搜得到**（按编码名）。
    ///
    /// 压缩响应的正文没有落库，`compression` 是它唯一可搜的值，而那个值（`gzip`）
    /// 在改前嵌在中文标记里、与语言无关——所以这不是「中文特有的能力」，
    /// 漏掉它就是回退。用内存库跑**真 SQL**，条件文本取自
    /// `TRAFFIC_SEARCH_PREDICATE`（与生产同一个常量，不是抄一份）。
    ///
    /// 只拼最外层那两条与 `:search` 无关的守卫（会话归属 + cli）；被测的就是搜索条件本身。
    #[test]
    fn search_matches_compression_encoding() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE traffic (
                id TEXT PRIMARY KEY, session_id TEXT, cli_id TEXT,
                req_body TEXT, res_body TEXT, compression TEXT
             );",
        )
        .unwrap();
        conn.execute_batch(
            "INSERT INTO traffic VALUES
                ('compressed', 's1', 'claude', '{\"model\":\"m\"}', NULL, 'gzip'),
                ('plain',      's1', 'claude', '{\"model\":\"m\"}', '{\"text\":\"hello\"}', NULL),
                ('brotli',     's1', 'claude', '{\"model\":\"m\"}', NULL, 'br');",
        )
        .unwrap();

        let count = |pattern: Option<&str>| -> i64 {
            let sql = format!(
                "SELECT COUNT(*) FROM traffic
                 WHERE ((:sid IS NULL AND session_id IS NULL) OR session_id = :sid)
                   AND cli_id = :cli
                   AND {TRAFFIC_SEARCH_PREDICATE}"
            );
            conn.query_row(
                &sql,
                rusqlite::named_params! { ":sid": "s1", ":cli": "claude", ":search": pattern },
                |row| row.get(0),
            )
            .unwrap()
        };

        // 压缩行按编码名命中——改前 `res_body LIKE '%gzip%'` 在这里必然为 0
        assert_eq!(count(Some("%gzip%")), 1);
        assert_eq!(count(Some("%br%")), 1);
        // 阳性对照：正文里的词照旧搜得到（另两个分支没被这一项挤掉）
        assert_eq!(count(Some("%hello%")), 1);
        // 反向对照：没写过的东西不命中，证明上面的 1 不是「条件恒真」
        assert_eq!(count(Some("%zstd%")), 0);
        // :search 为 NULL 时整条短路，三行都在
        assert_eq!(count(None), 3);
    }

    /// 建一张与侧车建表同形的内存 `traffic` 表。
    ///
    /// 列名与类型照 `src-tauri-proxy/src/storage.rs` 的 `Storage::new` 抄，**不从
    /// `TRAFFIC_DETAIL_COLUMNS` 生成**：从常量生成的话，常量里删掉一列、建表跟着删，
    /// 下面的测试就跟着一起瞎了。
    fn traffic_fixture() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE traffic (
                id TEXT PRIMARY KEY, timestamp TEXT NOT NULL, method TEXT NOT NULL, path TEXT NOT NULL,
                req_headers TEXT, req_body TEXT, req_size INTEGER NOT NULL DEFAULT 0, status INTEGER,
                res_headers TEXT, res_body TEXT, compression TEXT, error_kind TEXT,
                res_size INTEGER NOT NULL DEFAULT 0, duration_ms INTEGER NOT NULL DEFAULT 0,
                session_id TEXT, cli_id TEXT DEFAULT 'claude'
             );",
        )
        .unwrap();
        conn
    }

    /// 插一行；`compression` / `error_kind` / `res_size` / `duration_ms` 由调用方给，
    /// 其余列填固定值（供【下标对位】测试逐个断言）。
    fn insert_traffic_row(
        conn: &Connection,
        id: &str,
        compression: Option<&str>,
        error_kind: Option<&str>,
        res_size: i64,
        duration_ms: i64,
    ) {
        conn.execute(
            "INSERT INTO traffic (id, timestamp, method, path, req_headers, req_body, req_size,
                                  status, res_headers, res_body, compression, error_kind, res_size,
                                  duration_ms, session_id, cli_id)
             VALUES (?1, '2026-09-25T00:00:00Z', 'POST', '/v1/messages', 'req-headers', 'req-body',
                     11, 502, 'res-headers', 'third-party text', ?2, ?3, ?4, ?5, 'sess-1', 'claude')",
            params![id, compression, error_kind, res_size, duration_ms],
        )
        .unwrap();
    }

    /// 应用侧转发 `error_kind` 的证据：库里的 `error_kind` 要**经 `get_detail` 的查询体读回**。
    ///
    /// 判据是**值**不是「字段存在」——只断言 `is_some()` 的话，把 `row.get(13)` 写成
    /// `row.get(12)`（`compression` 也是 `Option<String>`）照样通过。
    /// 这一行的 `compression` 为 NULL、`error_kind` 为 `"upstream"`：错位到 12 号槽位会读成
    /// `None`，故阳性值 + 非 `None` 一起断言。
    #[test]
    fn detail_reads_error_kind_value_from_its_own_column() {
        let conn = traffic_fixture();
        insert_traffic_row(&conn, "row-1", None, Some("upstream"), 4096, 777);

        let detail = get_detail_from(&conn, "row-1").unwrap();
        assert_eq!(detail.error_kind.as_deref(), Some("upstream"));
        assert!(detail.error_kind.is_some(), "error_kind 必须到，不能是 NULL");
        // 反向对照：正文仍是第三方原文本身，没被这一列挤掉
        assert_eq!(detail.res_body.as_deref(), Some("third-party text"));
        assert_eq!(detail.compression, None);
    }

    /// 【下标对位】同一行给每个字段填一个可区分的值，断言读回的十四个值各自正确。
    ///
    /// 专盯**同类型**字段：这张表里任何两个同类型列互换都会静默换值 —— `String` 四列
    /// （`id`/`timestamp`/`method`/`path`）、`Option<String>` 六列（`req_headers`/`req_body`/
    /// `res_headers`/`res_body`/`compression`/`error_kind`）、`i64` 三列（`req_size`/`res_size`/
    /// `duration_ms`）；`status` 是 `Option<i32>`，同型只有它一个。错位时编译器不报错、`row.get`
    /// 也不报错，只静默换值——只有逐个断言值才能发现。列序（`… res_size, duration_ms, compression,
    /// error_kind`）与结构体字段序（`… compression, error_kind, res_size, duration_ms`）不同，
    /// 绑定全靠下标。
    #[test]
    fn detail_row_mapping_keeps_same_typed_columns_apart() {
        let conn = traffic_fixture();
        insert_traffic_row(&conn, "row-1", Some("gzip"), Some("upstream"), 4096, 777);

        let detail = get_detail_from(&conn, "row-1").unwrap();
        assert_eq!(detail.id, "row-1");
        assert_eq!(detail.timestamp, "2026-09-25T00:00:00Z");
        assert_eq!(detail.method, "POST");
        assert_eq!(detail.path, "/v1/messages");
        assert_eq!(detail.req_headers.as_deref(), Some("req-headers"));
        assert_eq!(detail.req_body.as_deref(), Some("req-body"));
        assert_eq!(detail.req_size, 11);
        assert_eq!(detail.status, Some(502));
        assert_eq!(detail.res_headers.as_deref(), Some("res-headers"));
        assert_eq!(detail.res_body.as_deref(), Some("third-party text"));
        assert_eq!(detail.compression.as_deref(), Some("gzip"));
        assert_eq!(detail.error_kind.as_deref(), Some("upstream"));
        assert_eq!(detail.res_size, 4096);
        assert_eq!(detail.duration_ms, 777);
    }
}
