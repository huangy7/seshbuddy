//! Streaming IPC helpers.
//!
//! 所有需要分批推送大列表给前端的命令复用本模块的事件命名规则：
//! - `<topic>:<request_id>:chunk` — payload 是一批数据（具体类型由命令决定）
//! - `<topic>:<request_id>:done`  — payload 是完成元数据（如总偏移量）
//! - `<topic>:<request_id>:error` — payload 是 `AppError` 的 IPC 线格式

use crate::error::AppError;
use serde::Serialize;
use tauri::{AppHandle, Emitter};

/// 拼出 `<topic>:<request_id>:<event>` 形式的事件名。
pub fn event_name(topic: &str, request_id: &str, event: &str) -> String {
    format!("{}:{}:{}", topic, request_id, event)
}

/// emit chunk — 出错时记日志，返回 false 表示前端已断开，调用方可提前终止
pub fn emit_chunk<T: Serialize + Clone>(
    app: &AppHandle,
    topic: &str,
    request_id: &str,
    payload: &T,
) -> bool {
    let name = event_name(topic, request_id, "chunk");
    if let Err(e) = app.emit(&name, payload) {
        tracing::warn!("emit chunk failed: event={} err={}", name, e);
        return false;
    }
    true
}

pub fn emit_done<T: Serialize + Clone>(
    app: &AppHandle,
    topic: &str,
    request_id: &str,
    payload: &T,
) {
    let name = event_name(topic, request_id, "done");
    if let Err(e) = app.emit(&name, payload) {
        tracing::warn!("emit done failed: event={} err={}", name, e);
    }
}

/// 发 `<topic>:<request_id>:error`。
///
/// 载荷直接复用 `AppError` 的 IPC 线格式（`Coded` → `{code, params}`，其余 → 裸字符串），
/// 不另造第三种形状：前端 `renderAppError` 只认这两种，认不出的会经 `String(err)`
/// 退化成 `[object Object]` 上屏。
///
/// 实参收 `impl Into<AppError>` 而非 `String`：`AppError` 直接透传。**`String` 已经进不来**——
/// 那条把第三方文本静默造成 `Business` 的 `impl From<String> for AppError`（R1 的桥）
/// 已由计划 13 的 T2 删除，传 `String` 现在是 `E0277`，不是「原样发出」。
/// **本仓今天的全部调用点都已经是 `AppError`**（计划 11 的 T4 关掉了 `parser` / `proxy`
/// 那条 `Result<_, String>` 链），保留 `Into` 是为了不让下一次改造被迫改签名。
/// **不能在函数里先 `to_string()`**——`Coded` 的 `Display` 是裸 code，那样用户看到的
/// 就是 `session.file_missing` 而不是文案。
pub fn emit_error(app: &AppHandle, topic: &str, request_id: &str, err: impl Into<AppError>) {
    let name = event_name(topic, request_id, "error");
    let err = err.into();
    if let Err(e) = app.emit(&name, &err) {
        tracing::warn!("emit error failed: event={} err={}", name, e);
    }
}

/// 启动一个流式后端任务的统一入口：spawn_blocking + catch_unwind + panic 兜底 emit_error。
///
/// 用法：
/// ```ignore
/// streaming::spawn_streaming_task(app, "session", request_id, |app, topic, request_id| {
///     // 业务核心：parse 文件 / 查 DB / emit chunk / emit done
/// });
/// ```
///
/// `work` 闭包内若 panic，会被 `catch_unwind` 接住，外层自动 emit `<topic>:<request_id>:error`，
/// 避免前端流式 UI 永远停在 loading 状态。
pub fn spawn_streaming_task<F>(
    app: AppHandle,
    topic: &'static str,
    request_id: String,
    work: F,
)
where
    F: FnOnce(&AppHandle, &str, &str) + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(move || {
        let app_for_panic = app.clone();
        let request_id_for_panic = request_id.clone();
        let topic_for_panic = topic;

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            work(&app, topic, &request_id);
        }));

        if let Err(payload) = result {
            let panic_msg = payload.downcast_ref::<&str>().copied()
                .or_else(|| payload.downcast_ref::<String>().map(|s| s.as_str()))
                .unwrap_or("<non-string panic>");
            tracing::error!("streaming task '{}' panicked: {}", topic_for_panic, panic_msg);
            emit_error(
                &app_for_panic,
                topic_for_panic,
                &request_id_for_panic,
                AppError::coded("streaming.panic"),
            );
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 通道 2 的载荷线格式：`emit_error` 体内就是 `app.emit(&name, &err)`，中间不加工，
    /// 所以 `AppError` 的序列化结果就是用户收到的字节。
    ///
    /// 两种形状都要有：`Coded` 发对象、其余发裸字符串（R1）。第三种形状（例如回到
    /// 带 `message` 字段的结构）前端认不出，会退化成 `[object Object]`。
    #[test]
    fn error_payload_is_the_app_error_wire_shape() {
        let coded = AppError::coded("streaming.panic");
        assert_eq!(
            serde_json::to_value(&coded).unwrap(),
            serde_json::json!({"code": "streaming.panic", "params": {}})
        );

        let legacy = AppError::Business("会话文件不存在: /tmp/gone.jsonl".to_string());
        assert_eq!(
            serde_json::to_value(&legacy).unwrap(),
            serde_json::json!("会话文件不存在: /tmp/gone.jsonl")
        );
    }
}
