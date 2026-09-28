use serde_json::{json, Value};
#[cfg(unix)]
use std::sync::atomic::AtomicBool;
#[cfg(unix)]
use std::sync::Arc;
use std::sync::LazyLock;
use tauri::Emitter;

use crate::error::{AppError, AppResult};
use crate::pty_manager::PtyStatus;

static VERSION_RE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"(\d+)\.(\d+)\.(\d+)").unwrap());

pub const MIN_TERMINAL_SEQUENCE_VERSION: (u32, u32, u32) = (2, 1, 139);

const HOOK_POLL_INTERVAL_MS: u64 = 50;

#[cfg(windows)]
trait HookTransport: Send {
    fn accept(&self) -> std::io::Result<Box<dyn HookStream>>;
}

#[cfg(windows)]
trait HookStream: Send {
    fn read_line(&mut self, buf: &mut String) -> std::io::Result<usize>;
}

#[cfg(unix)]
async fn unix_accept_loop<F>(
    listener: tokio::net::UnixListener,
    stop: Arc<AtomicBool>,
    handler: Arc<F>,
) -> std::io::Result<()>
where
    F: Fn(String) + Send + Sync + 'static,
{
    use std::sync::atomic::Ordering;
    use std::time::Duration;
    use tokio::io::AsyncBufReadExt;
    use tokio::io::BufReader;

    loop {
        if stop.load(Ordering::SeqCst) {
            return Ok(());
        }
        match tokio::time::timeout(
            Duration::from_millis(HOOK_POLL_INTERVAL_MS),
            listener.accept(),
        )
        .await
        {
            Ok(Ok((stream, _))) => {
                let stop = stop.clone();
                let handler = handler.clone();
                tokio::spawn(async move {
                    let mut reader = BufReader::new(stream);
                    let mut line = String::new();
                    loop {
                        if stop.load(Ordering::SeqCst) {
                            break;
                        }
                        match tokio::time::timeout(
                            Duration::from_millis(HOOK_POLL_INTERVAL_MS),
                            reader.read_line(&mut line),
                        )
                        .await
                        {
                            Ok(Ok(0)) => break,
                            Ok(Ok(_)) => {
                                if line.len() > 65536 {
                                    break;
                                }
                                handler(line.trim_end_matches(['\r', '\n']).to_string());
                                line.clear();
                            }
                            Ok(Err(e)) => {
                                if matches!(
                                    e.kind(),
                                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                                ) {
                                    continue;
                                }
                                tracing::warn!("[claude_hooks] stream read failed: {}", e);
                                break;
                            }
                            Err(_) => continue,
                        }
                    }
                });
            }
            Ok(Err(e)) => {
                tracing::warn!("[claude_hooks] transport accept failed: {}", e);
                tokio::time::sleep(Duration::from_millis(HOOK_POLL_INTERVAL_MS)).await;
            }
            Err(_) => continue,
        }
    }
}

#[cfg(windows)]
struct NamedPipeHookTransport {
    listener: interprocess::os::windows::named_pipe::PipeListener<
        interprocess::os::windows::named_pipe::pipe_mode::Bytes,
        interprocess::os::windows::named_pipe::pipe_mode::Bytes,
    >,
}

#[cfg(windows)]
impl NamedPipeHookTransport {
    fn bind(path: std::path::PathBuf) -> std::io::Result<Self> {
        use interprocess::os::windows::named_pipe::{PipeListenerOptions, PipeMode};
        let pipe_name = format!(r"\\.\pipe\{}", path.to_string_lossy().replace(['\\', '/'], "_"));
        let listener = PipeListenerOptions::new()
            .path(pipe_name)
            .mode(PipeMode::Bytes)
            .nonblocking(true)
            .create_duplex::<interprocess::os::windows::named_pipe::pipe_mode::Bytes>()?;
        Ok(Self { listener })
    }
}

#[cfg(windows)]
impl HookTransport for NamedPipeHookTransport {
    fn accept(&self) -> std::io::Result<Box<dyn HookStream>> {
        let stream = self.listener.accept()?;
        Ok(Box::new(NamedPipeHookStream {
            reader: std::io::BufReader::new(stream),
        }))
    }
}

#[cfg(windows)]
struct NamedPipeHookStream {
    reader: std::io::BufReader<
        interprocess::os::windows::named_pipe::DuplexPipeStream<
            interprocess::os::windows::named_pipe::pipe_mode::Bytes,
        >,
    >,
}

#[cfg(windows)]
impl HookStream for NamedPipeHookStream {
    fn read_line(&mut self, buf: &mut String) -> std::io::Result<usize> {
        use std::io::BufRead;
        self.reader.read_line(buf)
    }
}

pub struct ClaudeHookRuntime {
    #[allow(dead_code)]
    path: std::path::PathBuf,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    join: Option<std::thread::JoinHandle<()>>,
    handler_threads: std::sync::Arc<std::sync::Mutex<Vec<std::thread::JoinHandle<()>>>>,
}

#[cfg(unix)]
impl Drop for ClaudeHookRuntime {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::SeqCst);
        if let Ok(mut threads) = self.handler_threads.lock() {
            for handle in threads.drain(..) {
                let _ = handle.join();
            }
        }
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(windows)]
impl Drop for ClaudeHookRuntime {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::SeqCst);
        if let Ok(mut threads) = self.handler_threads.lock() {
            for handle in threads.drain(..) {
                let _ = handle.join();
            }
        }
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
        // Named pipe path is just a string identifier, no filesystem cleanup needed
    }
}

#[cfg(unix)]
pub fn start_runtime(
    app: tauri::AppHandle,
    session_id: String,
    project_path: String,
) -> std::io::Result<(ClaudeHookRuntime, std::path::PathBuf)> {
    let socket_path = std::env::temp_dir().join(format!("seshbuddy-agent-{}.sock", session_id));
    let runtime = start_runtime_with_handler(
        socket_path.clone(),
        make_hook_handler(app, session_id, project_path),
    )?;
    Ok((runtime, socket_path))
}

#[cfg(windows)]
pub fn start_runtime(
    app: tauri::AppHandle,
    session_id: String,
    project_path: String,
) -> std::io::Result<(ClaudeHookRuntime, std::path::PathBuf)> {
    let socket_path = std::path::PathBuf::from(format!("seshbuddy-agent-{}", session_id));
    let full_pipe_path =
        std::path::PathBuf::from(format!(r"\\.\pipe\seshbuddy-agent-{}", session_id));
    let runtime = start_runtime_with_handler(
        socket_path.clone(),
        make_hook_handler(app, session_id, project_path),
    )?;
    Ok((runtime, full_pipe_path))
}

fn make_hook_handler(
    app: tauri::AppHandle,
    session_id: String,
    project_path: String,
) -> impl Fn(String) + Send + Sync + 'static {
    move |raw| {
        tracing::info!(
            target: "claude_hook_relay",
            session_id = %session_id,
            raw = %raw,
            "received hook relay payload"
        );
        let effects = parse_hook_effects(&raw, &session_id, &project_path);
        tracing::info!(
            target: "claude_hook_relay",
            session_id = %session_id,
            status = ?effects.status,
            file_link = ?effects.file_link.is_some(),
            "parsed hook effects"
        );
        if let Some(status) = effects.status {
            crate::pty_manager::update_session_status(&app, &session_id, status);
        }
        if let Some(file_link) = effects.file_link {
            let _ = app.emit(&format!("pty-file-link-{}", session_id), file_link);
        }
    }
}

#[cfg(unix)]
fn start_runtime_with_handler<F>(
    path: std::path::PathBuf,
    handler: F,
) -> std::io::Result<ClaudeHookRuntime>
where
    F: Fn(String) + Send + Sync + 'static,
{
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::UnixListener as StdUnixListener;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    let _ = std::fs::remove_file(&path);
    let std_listener = StdUnixListener::bind(&path)?;
    std_listener.set_nonblocking(true)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;

    let stop = Arc::new(AtomicBool::new(false));
    let stop_thread = stop.clone();
    let handler = Arc::new(handler);

    let join = std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("failed to create tokio runtime for hook relay");
        runtime.block_on(async move {
            let listener = tokio::net::UnixListener::from_std(std_listener)
                .expect("failed to convert std unix listener to tokio");
            if let Err(e) = unix_accept_loop(listener, stop_thread, handler).await {
                tracing::warn!("[claude_hooks] unix hook runtime ended with error: {}", e);
            }
        });
    });

    Ok(ClaudeHookRuntime {
        path,
        stop,
        join: Some(join),
        handler_threads: Arc::new(std::sync::Mutex::new(Vec::new())),
    })
}

#[cfg(windows)]
fn start_runtime_with_handler<F>(
    path: std::path::PathBuf,
    handler: F,
) -> std::io::Result<ClaudeHookRuntime>
where
    F: Fn(String) + Send + Sync + 'static,
{
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    let transport: Box<dyn HookTransport> = Box::new(NamedPipeHookTransport::bind(path.clone())?);

    let stop = Arc::new(AtomicBool::new(false));
    let stop_thread = stop.clone();
    let handler = Arc::new(handler);
    let handler_threads: Arc<std::sync::Mutex<Vec<std::thread::JoinHandle<()>>>> =
        Arc::new(std::sync::Mutex::new(Vec::new()));
    let handler_threads_accept = handler_threads.clone();

    let join = std::thread::spawn(move || {
        while !stop_thread.load(Ordering::SeqCst) {
            match transport.accept() {
                Ok(stream) => {
                    let stop_stream = stop_thread.clone();
                    let handler = handler.clone();
                    let handle = std::thread::spawn(move || {
                        windows_handle_hook_stream(stream, stop_stream, handler);
                    });
                    if let Ok(mut threads) = handler_threads_accept.lock() {
                        threads.push(handle);
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(HOOK_POLL_INTERVAL_MS));
                }
                Err(e) => {
                    tracing::warn!("[claude_hooks] transport accept failed: {}", e);
                    std::thread::sleep(Duration::from_millis(HOOK_POLL_INTERVAL_MS));
                }
            }
        }
    });

    Ok(ClaudeHookRuntime {
        path,
        stop,
        join: Some(join),
        handler_threads,
    })
}

#[cfg(windows)]
fn windows_handle_hook_stream<F>(
    mut stream: Box<dyn HookStream>,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    handler: std::sync::Arc<F>,
)
where
    F: Fn(String) + Send + Sync + 'static,
{
    use std::sync::atomic::Ordering;

    while !stop.load(Ordering::SeqCst) {
        let mut line = String::new();
        match stream.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {
                if line.len() > 65536 {
                    tracing::warn!(
                        "[claude_hooks] line too large ({} bytes), closing connection",
                        line.len()
                    );
                    break;
                }
                handler(line.trim_end_matches(['\r', '\n']).to_string());
            }
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                continue;
            }
            Err(e) => {
                tracing::warn!("[claude_hooks] stream read failed: {}", e);
                break;
            }
        }
    }
}

pub fn resolve_hook_relay_command() -> AppResult<std::path::PathBuf> {
    #[cfg(target_os = "windows")]
    let binary_name = "seshbuddy-proxy.exe";
    #[cfg(not(target_os = "windows"))]
    let binary_name = "seshbuddy-proxy";

    let mut candidates = Vec::new();
    if let Ok(current_exe) = std::env::current_exe() {
        if let Some(dir) = current_exe.parent() {
            candidates.push(dir.join(binary_name));
        }
    }

    if let Some(workspace_dir) = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent() {
        candidates.push(workspace_dir.join("target").join("debug").join(binary_name));
        candidates.push(
            workspace_dir
                .join("target")
                .join("release")
                .join(binary_name),
        );
    }

    candidates
        .into_iter()
        .find(|path| path.exists())
        .ok_or_else(|| AppError::coded("claude_hooks.relay_binary_missing"))
}

pub fn supports_terminal_sequence(cli_path: &std::path::Path) -> bool {
    let output = match std::process::Command::new(cli_path)
        .arg("--version")
        .output()
    {
        Ok(o) => o,
        Err(e) => {
            tracing::warn!("[claude_hooks] failed to run claude --version: {}", e);
            return false;
        }
    };
    let text = String::from_utf8_lossy(&output.stdout);
    parse_version(&text)
        .map(|v| v >= MIN_TERMINAL_SEQUENCE_VERSION)
        .unwrap_or(false)
}

fn parse_version(text: &str) -> Option<(u32, u32, u32)> {
    let caps = VERSION_RE.captures(text)?;
    Some((
        caps[1].parse().ok()?,
        caps[2].parse().ok()?,
        caps[3].parse().ok()?,
    ))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HookMode {
    Osc,
    HookRelay,
}

pub fn merge_seshbuddy_hooks(
    settings: &mut Value,
    session_id: &str,
    socket_path: &str,
    relay_command: &str,
    mode: HookMode,
) {
    match mode {
        HookMode::Osc => merge_osc_hooks(settings, relay_command),
        HookMode::HookRelay => merge_relay_hooks(settings, session_id, socket_path, relay_command),
    }
}

fn merge_osc_hooks(settings: &mut Value, relay_command: &str) {
    ensure_object(settings);
    let hooks = settings
        .as_object_mut()
        .unwrap()
        .entry("hooks")
        .or_insert_with(|| json!({}));
    ensure_object(hooks);

    for event_name in [
        "Notification",
        "Elicitation",
        "ElicitationResult",
        "Stop",
        "UserPromptSubmit",
        "PreToolUse",
        "PostToolUse",
    ] {
        let entry = build_osc_hook_entry(event_name, relay_command);
        push_hook_entry(hooks, event_name, entry);
    }
}

fn build_osc_hook_entry(event_name: &str, relay_command: &str) -> Value {
    json!({
        "matcher": "",
        "hooks": [{
            "type": "command",
            "command": relay_command,
            "args": [
                "emit-osc",
                "--event", event_name
            ],
            "timeout": 5
        }]
    })
}

fn merge_relay_hooks(
    settings: &mut Value,
    session_id: &str,
    socket_path: &str,
    relay_command: &str,
) {
    ensure_object(settings);
    let hooks = settings
        .as_object_mut()
        .unwrap()
        .entry("hooks")
        .or_insert_with(|| json!({}));
    ensure_object(hooks);

    for event_name in [
        "Notification",
        "Elicitation",
        "ElicitationResult",
        "Stop",
        "UserPromptSubmit",
        "PreToolUse",
        "PostToolUse",
    ] {
        let entry = build_hook_entry(event_name, session_id, socket_path, relay_command);
        push_hook_entry(hooks, event_name, entry);
    }
}

fn push_hook_entry(hooks: &mut Value, event_name: &str, entry: Value) {
    let hooks = hooks.as_object_mut().unwrap();
    let event_entries = hooks.entry(event_name).or_insert_with(|| json!([]));
    if !event_entries.is_array() {
        *event_entries = json!([]);
    }
    event_entries.as_array_mut().unwrap().push(entry);
}

fn ensure_object(value: &mut Value) {
    if !value.is_object() {
        *value = json!({});
    }
}

fn build_hook_entry(
    event_name: &str,
    session_id: &str,
    socket_path: &str,
    relay_command: &str,
) -> Value {
    json!({
        "matcher": "",
        "hooks": [{
            "type": "command",
            "command": relay_command,
            "args": [
                "hook-relay",
                "--socket", socket_path,
                "--session-id", session_id,
                "--event", event_name
            ],
            "timeout": 5
        }]
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PtyFileAction {
    Read,
    Write,
    Edit,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PtyFileLinkPayload {
    pub session_id: String,
    pub path: String,
    pub display_path: String,
    pub action: PtyFileAction,
}

pub fn parse_file_link_payload(
    raw: &str,
    session_id: &str,
    project_path: &str,
) -> Option<PtyFileLinkPayload> {
    let value: Value = serde_json::from_str(raw).ok()?;
    let event = value
        .get("hook_event_name")
        .or_else(|| value.get("seshbuddy_hook_event"))
        .and_then(|v| v.as_str())?;
    if !matches!(event, "PreToolUse" | "PostToolUse") {
        return None;
    }

    let tool_name = value.get("tool_name").and_then(|v| v.as_str())?;
    let tool_input = value.get("tool_input")?;
    let (action, raw_path) = match tool_name {
        "Read" => (PtyFileAction::Read, tool_input.get("file_path")?.as_str()?),
        "Write" => (PtyFileAction::Write, tool_input.get("file_path")?.as_str()?),
        "Edit" | "MultiEdit" => (PtyFileAction::Edit, tool_input.get("file_path")?.as_str()?),
        "NotebookEdit" => (
            PtyFileAction::Edit,
            tool_input.get("notebook_path")?.as_str()?,
        ),
        _ => return None,
    };

    normalize_project_file_path(raw_path, project_path).map(|(path, display_path)| {
        PtyFileLinkPayload {
            session_id: session_id.to_string(),
            path,
            display_path,
            action,
        }
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaudeHookEffects {
    pub status: Option<PtyStatus>,
    pub file_link: Option<PtyFileLinkPayload>,
}

pub fn parse_hook_effects(
    raw: &str,
    session_id: &str,
    project_path: &str,
) -> ClaudeHookEffects {
    ClaudeHookEffects {
        status: parse_hook_payload(raw),
        file_link: parse_file_link_payload(raw, session_id, project_path),
    }
}

fn normalize_project_file_path(raw_path: &str, project_path: &str) -> Option<(String, String)> {
    if raw_path.trim().is_empty() {
        return None;
    }

    let project_root = std::path::Path::new(project_path).canonicalize().ok()?;
    let candidate = std::path::Path::new(raw_path);

    if candidate
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return None;
    }

    let lexical_candidate = if candidate.is_absolute() {
        normalize_lexically(candidate)
    } else {
        normalize_lexically(&project_root.join(candidate))
    };
    let absolute = normalize_with_existing_ancestor(&lexical_candidate)?;

    if !absolute.starts_with(&project_root) {
        return None;
    }

    let display_path = absolute
        .strip_prefix(&project_root)
        .ok()
        .and_then(|p| p.to_str())
        .map(|p| p.trim_start_matches(std::path::MAIN_SEPARATOR).replace('\\', "/"))
        .filter(|p| !p.is_empty())?;

    Some((absolute.to_string_lossy().to_string(), display_path))
}

fn normalize_with_existing_ancestor(path: &std::path::Path) -> Option<std::path::PathBuf> {
    let mut current = path.to_path_buf();
    let mut missing = Vec::new();

    loop {
        if let Ok(mut resolved) = current.canonicalize() {
            for component in missing.iter().rev() {
                resolved.push(component);
            }
            return Some(resolved);
        }

        missing.push(current.file_name()?.to_os_string());
        current = current.parent()?.to_path_buf();
    }
}

fn normalize_lexically(path: &std::path::Path) -> std::path::PathBuf {
    let mut result = std::path::PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                result.pop();
            }
            _ => result.push(component.as_os_str()),
        }
    }
    result
}

pub fn parse_hook_payload(raw: &str) -> Option<PtyStatus> {
    let value: Value = serde_json::from_str(raw).ok()?;
    let event = value
        .get("hook_event_name")
        .or_else(|| value.get("seshbuddy_hook_event"))
        .and_then(|v| v.as_str())?;
    let tool_name = value.get("tool_name").and_then(|v| v.as_str());

    match event {
        "Notification" => match value.get("notification_type").and_then(|v| v.as_str()) {
            Some("permission_prompt") | Some("idle_prompt") => Some(PtyStatus::WaitingInput),
            _ => None,
        },
        "Elicitation" => Some(PtyStatus::WaitingInput),
        "UserPromptSubmit" | "ElicitationResult" => Some(PtyStatus::Active),
        "PreToolUse" if tool_name == Some("AskUserQuestion") => Some(PtyStatus::WaitingInput),
        "PreToolUse" | "PostToolUse" => Some(PtyStatus::Active),
        "Stop" => Some(PtyStatus::Idle),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempProject {
        base: std::path::PathBuf,
        root: std::path::PathBuf,
    }

    impl TempProject {
        fn new(test_name: &str) -> Self {
            let unique = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let base = std::env::temp_dir().join(format!(
                "seshbuddy-{}-{}-{}",
                test_name,
                std::process::id(),
                unique
            ));
            let root = base.join("project");
            std::fs::create_dir_all(root.join("src")).unwrap();
            std::fs::create_dir_all(root.join("notebooks")).unwrap();
            Self { base, root }
        }

        fn project_path(&self) -> &str {
            self.root.to_str().unwrap()
        }

        fn path_string(&self, relative_path: &str) -> String {
            self.root
                .canonicalize()
                .unwrap()
                .join(relative_path)
                .to_string_lossy()
                .to_string()
        }
    }

    impl Drop for TempProject {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.base);
        }
    }

    #[test]
    fn merge_hooks_appends_without_replacing_user_entries() {
        let mut settings = serde_json::json!({
            "hooks": {
                "Notification": [
                    { "matcher": "", "hooks": [{ "type": "command", "command": "echo user" }] }
                ]
            }
        });

        merge_seshbuddy_hooks(
            &mut settings,
            "abc123",
            "/tmp/seshbuddy-agent.sock",
            "/Applications/SeshBuddy.app/Contents/MacOS/seshbuddy-proxy",
            HookMode::HookRelay,
        );

        let entries = settings["hooks"]["Notification"].as_array().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0]["hooks"][0]["command"], "echo user");
        assert_eq!(entries[1]["hooks"][0]["args"][2], "/tmp/seshbuddy-agent.sock");
        assert_eq!(entries[1]["hooks"][0]["args"][4], "abc123");
    }

    #[test]
    fn merge_osc_hooks_generates_emit_osc_command() {
        let mut settings = json!({});
        merge_seshbuddy_hooks(
            &mut settings,
            "abc123",
            "/tmp/seshbuddy-agent.sock",
            "/Applications/SeshBuddy.app/Contents/MacOS/seshbuddy-proxy",
            HookMode::Osc,
        );

        let notification = &settings["hooks"]["Notification"][0];
        let hook = &notification["hooks"][0];

        assert_eq!(hook["command"], "/Applications/SeshBuddy.app/Contents/MacOS/seshbuddy-proxy");
        let args = hook["args"].as_array().unwrap();
        assert_eq!(args[0], "emit-osc");
        assert_eq!(args[1], "--event");
        assert_eq!(args[2], "Notification");
    }

    #[test]
    fn merge_osc_hooks_generates_emit_osc_command_on_windows() {
        let mut settings = json!({});
        merge_seshbuddy_hooks(
            &mut settings,
            "abc123",
            r"\\.\pipe\seshbuddy-agent-abc123",
            r"C:\Program Files\SeshBuddy\seshbuddy-proxy.exe",
            HookMode::Osc,
        );

        let notification = &settings["hooks"]["Notification"][0];
        let hook = &notification["hooks"][0];

        assert_eq!(hook["command"], r"C:\Program Files\SeshBuddy\seshbuddy-proxy.exe");
        let args = hook["args"].as_array().unwrap();
        assert_eq!(args[0], "emit-osc");
        assert_eq!(args[1], "--event");
        assert_eq!(args[2], "Notification");
    }

    #[test]
    fn parse_hook_payload_maps_claude_events_to_status() {
        assert_eq!(
            parse_hook_payload(
                r#"{"hook_event_name":"Notification","notification_type":"permission_prompt"}"#
            ),
            Some(PtyStatus::WaitingInput)
        );
        assert_eq!(
            parse_hook_payload(r#"{"hook_event_name":"Elicitation"}"#),
            Some(PtyStatus::WaitingInput)
        );
        assert_eq!(
            parse_hook_payload(r#"{"hook_event_name":"UserPromptSubmit"}"#),
            Some(PtyStatus::Active)
        );
        assert_eq!(
            parse_hook_payload(r#"{"hook_event_name":"ElicitationResult"}"#),
            Some(PtyStatus::Active)
        );
        assert_eq!(
            parse_hook_payload(r#"{"hook_event_name":"Stop"}"#),
            Some(PtyStatus::Idle)
        );
        assert_eq!(
            parse_hook_payload(r#"{"hook_event_name":"PreToolUse","tool_name":"AskUserQuestion"}"#),
            Some(PtyStatus::WaitingInput)
        );
        assert_eq!(
            parse_hook_payload(r#"{"hook_event_name":"PostToolUse","tool_name":"AskUserQuestion"}"#),
            Some(PtyStatus::Active)
        );
        assert_eq!(parse_hook_payload("not json"), None);
    }

    #[test]
    fn merge_osc_hooks_includes_all_status_events() {
        let mut settings = json!({});
        merge_seshbuddy_hooks(
            &mut settings,
            "abc123",
            "/tmp/seshbuddy-agent.sock",
            "/Applications/SeshBuddy.app/Contents/MacOS/seshbuddy-proxy",
            HookMode::Osc,
        );

        let elicitation = &settings["hooks"]["Elicitation"][0];
        assert_eq!(elicitation["hooks"][0]["args"][2], "Elicitation");

        let elicitation_result = &settings["hooks"]["ElicitationResult"][0];
        assert_eq!(elicitation_result["hooks"][0]["args"][2], "ElicitationResult");

        assert!(settings["hooks"]["PreToolUse"].is_array());
        assert_eq!(
            settings["hooks"]["PreToolUse"][0]["hooks"][0]["args"][2],
            "PreToolUse"
        );
        assert!(settings["hooks"]["PostToolUse"].is_array());
        assert_eq!(
            settings["hooks"]["PostToolUse"][0]["hooks"][0]["args"][2],
            "PostToolUse"
        );
    }

    #[test]
    fn merge_relay_hooks_includes_elicitation_events() {
        let mut settings = json!({});
        merge_seshbuddy_hooks(
            &mut settings,
            "abc123",
            "/tmp/seshbuddy-agent.sock",
            "/Applications/SeshBuddy.app/Contents/MacOS/seshbuddy-proxy",
            HookMode::HookRelay,
        );

        assert!(settings["hooks"]["Elicitation"].is_array());
        assert!(settings["hooks"]["ElicitationResult"].is_array());
    }

    #[test]
    fn parse_file_link_payload_reads_project_file_from_post_tool_use() {
        let project = TempProject::new("read-file-link");
        let payload = r#"{
            "hook_event_name": "PostToolUse",
            "tool_name": "Read",
            "tool_input": { "file_path": "src/main.rs" }
        }"#;

        let event = parse_file_link_payload(payload, "session-1", project.project_path()).unwrap();

        assert_eq!(event.session_id, "session-1");
        assert_eq!(event.path, project.path_string("src/main.rs"));
        assert_eq!(event.display_path, "src/main.rs");
        assert_eq!(event.action, PtyFileAction::Read);
    }

    #[test]
    fn parse_file_link_payload_reads_project_file_from_pre_tool_use() {
        let project = TempProject::new("pre-read-file-link");
        let payload = r#"{
            "hook_event_name": "PreToolUse",
            "tool_name": "Read",
            "tool_input": { "file_path": "README.md" }
        }"#;

        let event = parse_file_link_payload(payload, "session-1", project.project_path()).unwrap();

        assert_eq!(event.session_id, "session-1");
        assert_eq!(event.path, project.path_string("README.md"));
        assert_eq!(event.display_path, "README.md");
        assert_eq!(event.action, PtyFileAction::Read);
    }

    #[test]
    fn parse_file_link_payload_maps_write_and_edit_tools() {
        let project = TempProject::new("write-edit-file-link");
        let write_path = project.path_string("src/new.rs");
        let write_payload = serde_json::json!({
            "hook_event_name": "PostToolUse",
            "tool_name": "Write",
            "tool_input": { "file_path": write_path }
        })
        .to_string();
        let write = parse_file_link_payload(
            &write_payload,
            "session-1",
            project.project_path(),
        )
        .unwrap();
        assert_eq!(write.action, PtyFileAction::Write);
        assert_eq!(write.display_path, "src/new.rs");

        for tool_name in ["Edit", "MultiEdit"] {
            let raw = format!(
                r#"{{"hook_event_name":"PostToolUse","tool_name":"{}","tool_input":{{"file_path":"src/lib.rs"}}}}"#,
                tool_name
            );
            let event =
                parse_file_link_payload(&raw, "session-1", project.project_path()).unwrap();
            assert_eq!(event.action, PtyFileAction::Edit);
            assert_eq!(event.path, project.path_string("src/lib.rs"));
        }
    }

    #[test]
    fn parse_file_link_payload_supports_notebook_edit_path() {
        let project = TempProject::new("notebook-file-link");
        let payload = r#"{
            "hook_event_name": "PostToolUse",
            "tool_name": "NotebookEdit",
            "tool_input": { "notebook_path": "notebooks/demo.ipynb" }
        }"#;

        let event = parse_file_link_payload(payload, "session-1", project.project_path()).unwrap();

        assert_eq!(event.action, PtyFileAction::Edit);
        assert_eq!(event.path, project.path_string("notebooks/demo.ipynb"));
        assert_eq!(event.display_path, "notebooks/demo.ipynb");
    }

    #[test]
    fn parse_file_link_payload_rejects_external_or_unsupported_paths() {
        let project = TempProject::new("reject-file-link");
        assert_eq!(
            parse_file_link_payload(
                r#"{"hook_event_name":"PostToolUse","tool_name":"Read","tool_input":{"file_path":"../outside.rs"}}"#,
                "session-1",
                project.project_path(),
            ),
            None
        );
        assert_eq!(
            parse_file_link_payload(
                r#"{"hook_event_name":"Notification","tool_name":"Read","tool_input":{"file_path":"src/main.rs"}}"#,
                "session-1",
                project.project_path(),
            ),
            None
        );
        assert_eq!(
            parse_file_link_payload(
                r#"{"hook_event_name":"PostToolUse","tool_name":"Bash","tool_input":{"command":"cat src/main.rs"}}"#,
                "session-1",
                project.project_path(),
            ),
            None
        );
    }

    #[cfg(unix)]
    #[test]
    fn parse_file_link_payload_rejects_symlink_ancestor_escape_for_missing_file() {
        let project = TempProject::new("symlink-file-link");
        let outside = project.base.join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, project.root.join("out")).unwrap();

        assert_eq!(
            parse_file_link_payload(
                r#"{"hook_event_name":"PostToolUse","tool_name":"Read","tool_input":{"file_path":"out/new.rs"}}"#,
                "session-1",
                project.project_path(),
            ),
            None
        );
    }

    #[test]
    fn parse_hook_effects_returns_status_and_file_link() {
        let project = TempProject::new("hook-effects");
        let effects = parse_hook_effects(
            r#"{"hook_event_name":"PostToolUse","tool_name":"Read","tool_input":{"file_path":"src/main.rs"}}"#,
            "session-1",
            project.project_path(),
        );

        assert_eq!(effects.status, Some(PtyStatus::Active));
        assert_eq!(
            effects.file_link.unwrap().path,
            project.path_string("src/main.rs")
        );

        let stop = parse_hook_effects(
            r#"{"hook_event_name":"Stop"}"#,
            "session-1",
            project.project_path(),
        );
        assert_eq!(stop.status, Some(PtyStatus::Idle));
        assert!(stop.file_link.is_none());
    }

    #[test]
    fn hook_command_uses_bundled_proxy_relay_without_shell_or_python() {
        let entry = build_hook_entry(
            "Notification",
            "abc123",
            "/tmp/seshbuddy's.sock",
            "/Applications/SeshBuddy.app/Contents/MacOS/seshbuddy-proxy",
        );
        let hook = &entry["hooks"][0];

        assert_eq!(
            hook["command"],
            "/Applications/SeshBuddy.app/Contents/MacOS/seshbuddy-proxy"
        );
        assert_eq!(hook["args"][0], "hook-relay");
        assert_eq!(hook["args"][2], "/tmp/seshbuddy's.sock");
        assert_eq!(hook["args"][4], "abc123");
        assert_eq!(hook["args"][6], "Notification");
        assert!(!serde_json::to_string(hook).unwrap().contains("|| true"));
        assert!(!serde_json::to_string(hook).unwrap().contains("python"));
    }

    #[cfg(unix)]
    #[test]
    fn dropping_runtime_without_connections_returns_and_removes_socket() {
        let session_id = format!("drop-test-{}", std::process::id());
        let socket_path = std::env::temp_dir().join(format!("seshbuddy-agent-{}.sock", session_id));
        let runtime = start_runtime_with_handler(socket_path.clone(), |_| {}).unwrap();

        assert!(socket_path.exists());

        let started = std::time::Instant::now();
        drop(runtime);

        assert!(
            started.elapsed() < std::time::Duration::from_secs(1),
            "runtime drop should not block waiting for socket activity"
        );
        assert!(!socket_path.exists());
    }

    #[cfg(unix)]
    #[test]
    fn runtime_reads_newline_delimited_payloads_from_socket() {
        let session_id = format!("payload-test-{}", std::process::id());
        let socket_path = std::env::temp_dir().join(format!("seshbuddy-agent-{}.sock", session_id));
        let (tx, rx) = std::sync::mpsc::channel();
        let runtime = start_runtime_with_handler(socket_path.clone(), move |raw| {
            tx.send(raw).unwrap();
        })
        .unwrap();

        let mut stream = std::os::unix::net::UnixStream::connect(&socket_path).unwrap();
        use std::io::Write as _;
        writeln!(stream, r#"{{"hook_event_name":"Stop"}}"#).unwrap();

        assert_eq!(
            rx.recv_timeout(std::time::Duration::from_secs(1)).unwrap(),
            r#"{"hook_event_name":"Stop"}"#
        );

        let started = std::time::Instant::now();
        drop(runtime);

        assert!(
            started.elapsed() < std::time::Duration::from_secs(1),
            "runtime drop should not block after receiving socket payloads"
        );
        assert!(!socket_path.exists());
    }

    #[test]
    fn parse_version_extracts_semver_from_claude_code_output() {
        assert_eq!(parse_version("Claude Code 2.1.139"), Some((2, 1, 139)));
        assert_eq!(
            parse_version("claude version 2.1.140\n"),
            Some((2, 1, 140))
        );
        assert_eq!(parse_version("2.1.138"), Some((2, 1, 138)));
        assert_eq!(parse_version("no version here"), None);
        assert_eq!(parse_version("1.2"), None);
    }
}
