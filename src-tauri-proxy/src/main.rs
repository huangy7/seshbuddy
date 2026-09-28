#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod assistant;
mod capture;
mod emit_osc;
mod hook_relay;
mod lifecycle;
mod proxy;
mod storage;
mod ws;

use clap::{Parser, Subcommand};
use tracing_subscriber::fmt;

#[derive(Parser)]
#[command(
    name = "seshbuddy-proxy",
    about = "AI API reverse proxy with traffic logging"
)]
struct Args {
    #[command(subcommand)]
    command: Option<Command>,

    /// Listen address (e.g. 127.0.0.1:18080)
    #[arg(long, default_value = "127.0.0.1:18080")]
    listen: String,

    /// Target URL to forward requests to (e.g. https://api.anthropic.com)
    #[arg(long, default_value = "https://api.anthropic.com")]
    target: String,

    /// Current CLI id (e.g. claude / codex)
    #[arg(long, default_value = "claude")]
    cli_id: String,

    /// WebSocket port for real-time event push
    #[arg(long, default_value = "18081")]
    ws_port: u16,

    /// SQLite database path for traffic storage
    #[arg(long)]
    db: Option<String>,

    /// Log directory (proxy writes its own log file here)
    #[arg(long)]
    log_dir: Option<String>,
}

#[derive(Subcommand)]
enum Command {
    /// Forward one Claude hook payload to SeshBuddy.
    HookRelay {
        #[arg(long)]
        socket: std::path::PathBuf,

        #[arg(long)]
        session_id: String,

        #[arg(long)]
        event: String,
    },

    /// Read a Claude hook payload from stdin and emit an OSC terminal sequence.
    EmitOsc {
        #[arg(long)]
        event: String,
    },

    /// List session metadata from the app index (read-only).
    List {
        #[arg(long)]
        cli: Option<String>,
        #[arg(long)]
        days: Option<u32>,
        /// 起始日期（YYYY-MM-DD，含当日），可与 --days 叠加（取交集）
        #[arg(long)]
        since: Option<String>,
        /// 截止日期（YYYY-MM-DD，含当日）
        #[arg(long)]
        until: Option<String>,
        #[arg(long)]
        project: Option<String>,
        #[arg(long, default_value = "200")]
        limit: u32,
        #[arg(long)]
        json: bool,
    },

    /// Read a session transcript from the extraction cache.
    Show {
        /// Session id (as returned by `list`; 可只给前缀)
        id: String,
        /// 不传则全 CLI 查找（id 全索引唯一）
        #[arg(long)]
        cli: Option<String>,
        #[arg(long)]
        from: Option<u64>,
        #[arg(long)]
        to: Option<u64>,
        #[arg(long, default_value = "8000")]
        max_chars: usize,
        /// 输出格式：json（默认）/ text（纯 [#N role] 文本行）
        #[arg(long, default_value = "json")]
        format: String,
        /// 兼容 flag：输出本来就是 JSON，接受但忽略（防 agent 习惯性追加报错）
        #[arg(long)]
        json: bool,
    },

    /// Full-text search across the extraction cache.
    Grep {
        keyword: String,
        #[arg(long)]
        cli: Option<String>,
        #[arg(long)]
        days: Option<u32>,
        #[arg(long, default_value = "20")]
        max_hits: usize,
        /// 兼容 flag：输出本来就是 JSON，接受但忽略
        #[arg(long)]
        json: bool,
    },
}

fn init_logging(log_dir: &str) -> Option<tracing_appender::non_blocking::WorkerGuard> {
    std::fs::create_dir_all(log_dir).ok()?;

    let log_path = std::path::Path::new(log_dir).join("proxy.log");
    let file_appender = tracing_rolling_file::RollingFileAppenderBase::builder()
        .filename(log_path.to_string_lossy().into_owned())
        .max_filecount(2)
        .condition_max_file_size(10 * 1024 * 1024)
        .build()
        .ok()?;

    let (non_blocking, guard) = file_appender.get_non_blocking_appender();
    fmt::Subscriber::builder()
        .with_writer(non_blocking)
        .with_ansi(false)
        .init();

    Some(guard)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // `seshbuddy-proxy --json-help`：机器可读命令参考（agent 入口）
    // 必须在 Args::parse() 之前拦截——clap 会拒绝未知 flag
    if std::env::args().any(|a| a == "--json-help") {
        println!("{}", serde_json::to_string_pretty(&assistant::agent_help_json())?);
        return Ok(());
    }

    let args = Args::parse();

    if let Some(Command::HookRelay {
        socket,
        session_id,
        event,
    }) = &args.command
    {
        let _ = hook_relay::run(socket, session_id, event);
        return Ok(());
    }

    if let Some(Command::EmitOsc { event }) = &args.command {
        let _ = emit_osc::run(event).await;
        return Ok(());
    }

    if let Some(Command::List {
        cli,
        days,
        since,
        until,
        project,
        limit,
        json,
    }) = &args.command
    {
        let rows = assistant::run_list(
            cli.as_deref(),
            *days,
            since.as_deref(),
            until.as_deref(),
            project.as_deref(),
            *limit,
        )?;
        if *json {
            let out: Vec<serde_json::Value> = rows
                .iter()
                .map(|r| {
                    serde_json::json!({
                        "id": r.id, "title": r.title, "project": r.project,
                        "cli": r.cli, "updatedAt": r.updated_at,
                    })
                })
                .collect();
            println!("{}", serde_json::to_string(&out)?);
        } else {
            for r in &rows {
                println!("{}\t{}\t{}\t{}", r.id, r.cli, r.updated_at, r.title);
            }
        }
        return Ok(());
    }

    if let Some(Command::Show {
        id,
        cli,
        from,
        to,
        max_chars,
        format,
        json: _,
    }) = &args.command
    {
        assistant::run_show(cli.as_deref(), id, *from, *to, *max_chars, format)?;
        return Ok(());
    }

    if let Some(Command::Grep { keyword, cli, days, max_hits, json: _ }) = &args.command {
        assistant::run_grep(keyword, cli.as_deref(), *days, *max_hits)?;
        return Ok(());
    }

    let _log_guard = args.log_dir.as_deref().and_then(init_logging);
    lifecycle::install_process_guards();

    tracing::info!(
        "seshbuddy-proxy starting: {} -> {} [{}]",
        args.listen,
        args.target,
        args.cli_id
    );

    let db = args
        .db
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("--db is required when starting proxy mode"))?;
    let storage = storage::Storage::new(db)?;
    let broadcast = ws::WsBroadcast::new(256);

    let ws_addr = format!("127.0.0.1:{}", args.ws_port);
    let ws_broadcast = broadcast.clone();
    tokio::spawn(async move {
        if let Err(e) = ws::run_ws_server(&ws_addr, ws_broadcast).await {
            tracing::error!("WebSocket server error: {}", e);
        }
    });

    proxy::run_proxy(&args.listen, &args.target, &args.cli_id, storage, broadcast).await
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    #[cfg(unix)]
    #[test]
    fn hook_relay_forwards_stdin_json_to_unix_socket() {
        use std::io::BufRead;
        use std::os::unix::net::UnixListener;

        let socket_path = std::env::temp_dir().join(format!(
            "seshbuddy-proxy-hook-relay-test-{}.sock",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&socket_path);
        let listener = UnixListener::bind(&socket_path).unwrap();

        let socket_for_thread = socket_path.clone();
        let handle = std::thread::spawn(move || {
            super::hook_relay::forward_hook_payload(
                &socket_for_thread,
                "session-1",
                "Notification",
                br#"{"notification_type":"permission_prompt"}"#,
            )
            .unwrap();
        });

        let (stream, _) = listener.accept().unwrap();
        let mut line = String::new();
        std::io::BufReader::new(stream)
            .read_line(&mut line)
            .unwrap();
        handle.join().unwrap();
        let _ = std::fs::remove_file(&socket_path);

        let payload: serde_json::Value = serde_json::from_str(line.trim_end()).unwrap();
        assert_eq!(payload["notification_type"], "permission_prompt");
        assert_eq!(payload["seshbuddy_session_id"], "session-1");
        assert_eq!(payload["seshbuddy_hook_event"], "Notification");
    }

    #[test]
    fn hook_relay_command_does_not_require_proxy_db_argument() {
        let args = super::Args::try_parse_from([
            "seshbuddy-proxy",
            "hook-relay",
            "--socket",
            "/tmp/seshbuddy-agent.sock",
            "--session-id",
            "session-1",
            "--event",
            "Stop",
        ])
        .unwrap();

        assert!(matches!(
            args.command,
            Some(super::Command::HookRelay { .. })
        ));
    }

    #[test]
    fn emit_osc_command_does_not_require_proxy_db_argument() {
        let args = super::Args::try_parse_from([
            "seshbuddy-proxy",
            "emit-osc",
            "--event",
            "PreToolUse",
        ])
        .unwrap();

        assert!(matches!(
            args.command,
            Some(super::Command::EmitOsc { .. })
        ));
    }
}
