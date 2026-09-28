use crate::error::{AppError, AppResult};
use crate::cli::CliKind;
use crate::proxy;
use crate::streaming;
use tauri::AppHandle;

fn require_cli_kind(cli_id: Option<String>) -> AppResult<CliKind> {
    match cli_id {
        Some(value) => CliKind::from_id(Some(value.as_str())).map_err(AppError::from),
        None => Err(AppError::coded("proxy.cli_id_required")),
    }
}

#[tauri::command]
pub fn proxy_enable(cli_id: Option<String>, port: Option<u16>) -> AppResult<proxy::ProxyStatus> {
    let kind = require_cli_kind(cli_id)?;
    proxy::enable(kind, port.unwrap_or(proxy::default_port_for(kind))).map_err(AppError::from)
}

#[tauri::command]
pub fn proxy_disable(cli_id: Option<String>) -> AppResult<()> {
    let kind = require_cli_kind(cli_id)?;
    proxy::disable(Some(kind)).map_err(AppError::from)
}

#[tauri::command]
pub fn proxy_status(cli_id: Option<String>) -> AppResult<proxy::ProxyStatus> {
    let kind = require_cli_kind(cli_id)?;
    proxy::status(Some(kind)).map_err(AppError::from)
}

#[tauri::command]
pub fn proxy_get_detail(id: String) -> AppResult<proxy::TrafficDetail> {
    proxy::get_detail(id).map_err(AppError::from)
}

#[tauri::command]
pub fn proxy_clear_traffic(
    cli_id: Option<String>,
    before_days: Option<u32>,
) -> AppResult<proxy::ClearResult> {
    let kind = require_cli_kind(cli_id)?;
    proxy::clear_traffic(before_days, Some(kind)).map_err(AppError::from)
}

#[tauri::command]
pub fn proxy_db_size() -> AppResult<u64> {
    proxy::db_size().map_err(AppError::from)
}

#[tauri::command]
pub fn proxy_session_traffic_timestamps(cli_id: Option<String>, session_id: String) -> AppResult<Vec<String>> {
    let kind = require_cli_kind(cli_id)?;
    Ok(proxy::session_traffic_timestamps(&session_id, kind))
}

#[tauri::command]
pub fn proxy_get_session_traffic(
    cli_id: Option<String>,
    session_id: Option<String>,
    limit: Option<u32>,
    offset: Option<u32>,
    search: Option<String>,
    presets: Option<Vec<String>>,
    tool_names: Option<Vec<String>>,
) -> AppResult<proxy::TrafficList> {
    let kind = require_cli_kind(cli_id)?;
    proxy::get_session_traffic(
        session_id,
        limit.unwrap_or(50),
        offset.unwrap_or(0),
        Some(kind),
        search,
        presets,
        tool_names,
    )
    .map_err(AppError::from)
}

#[tauri::command]
pub fn proxy_find_by_timestamp(
    cli_id: Option<String>,
    session_id: String,
    timestamp: String,
) -> AppResult<Option<proxy::TrafficDetail>> {
    let kind = require_cli_kind(cli_id)?;
    proxy::find_traffic_by_timestamp(&session_id, &timestamp, Some(kind)).map_err(AppError::from)
}

const PROXY_BATCH_SIZE: usize = 50;
// 与 SCAN_PROJECTS_SNAPSHOT_LIMIT 同思路：u32::MAX 让 SQLite LIMIT 形同无 LIMIT，
// 避免新增"无 limit"路径重复 SQL 逻辑
const PROXY_FETCH_ALL: u32 = u32::MAX;


const PROXY_SESSIONS_TOPIC: &str = "proxy_sessions";

#[derive(Debug, Clone, serde::Serialize)]
struct ProxySessionsStreamDone {
    total: u64,
}

/// 流式拉取代理流量按 session 分组的汇总。事件 topic `proxy_sessions`，结构与 traffic 流相同。
#[tauri::command]
pub async fn proxy_get_sessions_stream(
    app: AppHandle,
    request_id: String,
    cli_id: Option<String>,
) -> AppResult<()> {
    let kind = require_cli_kind(cli_id)?;

    streaming::spawn_streaming_task(app, PROXY_SESSIONS_TOPIC, request_id, move |app, topic, request_id| {
        let list = match proxy::get_traffic_sessions(PROXY_FETCH_ALL, 0, Some(kind)) {
            Ok(v) => v,
            Err(e) => {
                streaming::emit_error(app, topic, request_id, e);
                return;
            }
        };

        let total = list.total;
        for batch in list.items.chunks(PROXY_BATCH_SIZE) {
            let items: Vec<proxy::SessionTrafficSummary> = batch.to_vec();
            streaming::emit_chunk(app, topic, request_id, &items);
        }

        streaming::emit_done(
            app,
            topic,
            request_id,
            &ProxySessionsStreamDone { total },
        );
    });

    Ok(())
}
