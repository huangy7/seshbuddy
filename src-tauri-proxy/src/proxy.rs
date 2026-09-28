use crate::capture::{self, TrafficRecord};
use crate::storage::Storage;
use crate::ws::{WsBroadcast, WsEvent};
use bytes::Bytes;
use http_body_util::{combinators::BoxBody, BodyExt, Full, StreamBody};
use hyper::body::{Frame, Incoming};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response};
use hyper_util::rt::TokioIo;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::Mutex;

type ResponseBody = BoxBody<Bytes, Infallible>;

/// Tracks the most recent session_id for associating count_tokens requests
struct LastSession {
    session_id: Option<String>,
    at: std::time::Instant,
}

pub async fn run_proxy(
    listen: &str,
    target: &str,
    cli_id: &str,
    storage: Storage,
    broadcast: WsBroadcast,
) -> anyhow::Result<()> {
    let addr: SocketAddr = listen.parse()?;
    let listener = TcpListener::bind(addr).await?;
    tracing::info!("Listening on http://{}", addr);

    let storage = Arc::new(storage);
    let client = reqwest::Client::builder().use_rustls_tls().build()?;
    let target: Arc<str> = target.trim_end_matches('/').into();
    let cli_id: Arc<str> = cli_id.into();
    let last_session = Arc::new(Mutex::new(LastSession {
        session_id: None,
        at: std::time::Instant::now(),
    }));

    let shutdown = Arc::new(tokio::sync::Notify::new());
    let shutdown_signal = shutdown.clone();
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.ok();
        tracing::info!("Shutting down...");
        shutdown_signal.notify_waiters();
    });

    loop {
        tokio::select! {
            result = listener.accept() => {
                let (stream, _) = result?;
                let io = TokioIo::new(stream);
                let storage = storage.clone();
                let client = client.clone();
                let target = target.clone();
                let broadcast = broadcast.clone();
                let cli_id_for_connection = cli_id.clone();
                let last_session = last_session.clone();

                tokio::spawn(async move {
                    let service = service_fn(move |req| {
                        let storage = storage.clone();
                        let client = client.clone();
                        let target = target.clone();
                        let broadcast = broadcast.clone();
                        let cli_id = cli_id_for_connection.clone();
                        let last_session = last_session.clone();
                        async move {
                            let resp = do_proxy(req, &target, &cli_id, &client, &storage, &broadcast, &last_session).await;
                            Ok::<_, Infallible>(resp)
                        }
                    });
                    if let Err(e) = http1::Builder::new()
                        .serve_connection(io, service)
                        .with_upgrades()
                        .await
                    {
                        tracing::error!("Connection error: {}", e);
                    }
                });
            }
            _ = shutdown.notified() => {
                tracing::info!("Proxy stopped");
                break;
            }
        }
    }

    Ok(())
}

async fn do_proxy(
    req: Request<Incoming>,
    target: &str,
    cli_id: &str,
    client: &reqwest::Client,
    storage: &Arc<Storage>,
    broadcast: &WsBroadcast,
    last_session: &Arc<Mutex<LastSession>>,
) -> Response<ResponseBody> {
    match proxy_inner(req, target, cli_id, client, storage, broadcast, last_session).await {
        Ok(resp) => resp,
        Err(e) => {
            tracing::error!("Proxy error: {}", e);
            // 这句英文是**故意的**，不要本地化：
            // 1) 消费方不是本应用的界面，而是被代理 CLI 的终端 —— 这段字符串进的是
            //    HTTP 502 响应体，由第三方程序原样打印在用户终端里（与例外表里的
            //    `.bat` 脚本同类：文案由 cmd.exe 打印，不由我们的 UI 渲染）。
            // 2) proxy 进程不感知语言：它不读语言设置，也不接收任何语言参数。本地化它
            //    需要一条跨进程的语言通道，并让 proxy 在编译期嵌入语言包（等于把
            //    `native_text` 那套机制跨进程复制一份），还要处理 proxy 长期存活期间
            //    「语言被改」的陈旧问题。
            // 3) 这条 502 **没有应用侧的一半**：本臂只被 `proxy_inner` 的 `?` 传播命中
            //    （读客户端正文的 `req.collect()`、上游中途断流的 `upstream_resp.bytes()`、
            //    组装响应的 `body(...)`）。前两项触发时 `store_record` 都还没跑过，**不写库** ——
            //    上游中途断流属于真正的上游失败，却只在应用里留下一条永不完成的 Pending 事件。
            //    第三项的 `body(...)` 有两处，失效形态都与前两项不同：流式分支那处的
            //    `store_record` 在后台任务里，与主任务**并发**（谁先谁后是竞态），但那任务
            //    **无条件**落一行；非流式分支那处**紧跟在 `store_record` 之后**。两处最终
            //    都是同一形态：「库里有一行（`error_kind` 为 `None`）、终端
            //    拿到 502，应用内却看不到这个 502」。
            //    有应用侧一半的是上游**连接**失败那一类：`proxy_inner` 里的 `store_record` 会写
            //    `res_body` + `error_kind`，由前端按当前语言渲染提示句。
            error_response(502, &format!("Proxy error: {}", e))
        }
    }
}

async fn proxy_inner(
    req: Request<Incoming>,
    target: &str,
    cli_id: &str,
    client: &reqwest::Client,
    storage: &Arc<Storage>,
    broadcast: &WsBroadcast,
    last_session: &Arc<Mutex<LastSession>>,
) -> anyhow::Result<Response<ResponseBody>> {
    let start = std::time::Instant::now();
    let record_id = uuid::Uuid::new_v4().to_string();
    let timestamp = chrono::Utc::now().to_rfc3339();

    let method = req.method().clone();
    let path = req
        .uri()
        .path_and_query()
        .map(|pq| pq.to_string())
        .unwrap_or_else(|| "/".to_string());

    // Capture request headers
    let req_headers_map = capture::hyper_headers_to_map(req.headers());
    let req_headers_json = capture::sanitize_headers(&req_headers_map);

    // Read full request body
    let req_body_bytes = req.collect().await?.to_bytes();
    let req_body = String::from_utf8_lossy(&req_body_bytes).to_string();
    let req_size = req_body_bytes.len() as i64;

    // Extract session_id from request body metadata
    let session_id = match capture::extract_session_id(&req_body) {
        Some(sid) => {
            // Update last_session for count_tokens association
            let mut ls = last_session.lock().await;
            ls.session_id = Some(sid.clone());
            ls.at = std::time::Instant::now();
            Some(sid)
        }
        None if path.contains("count_tokens") => {
            // Inherit session_id from most recent request within 5s
            let ls = last_session.lock().await;
            if ls.at.elapsed().as_secs() < 5 {
                ls.session_id.clone()
            } else {
                None
            }
        }
        None => None,
    };

    // Notify WS clients: request started
    broadcast.send(&WsEvent::Pending {
        id: record_id.clone(),
        timestamp: timestamp.clone(),
        method: method.to_string(),
        path: path.clone(),
        session_id: session_id.clone(),
    });

    // Build upstream request
    let upstream_url = format!("{}{}", target, path);
    let mut upstream_req = client.request(method.clone(), &upstream_url);

    for (key, value) in &req_headers_map {
        // accept-encoding 不透传：要求上游返回未压缩的 identity 响应，
        // 否则抓包存下的是 gzip 字节流（且经 UTF-8 lossy 转换后不可还原）
        if !key.eq_ignore_ascii_case("host")
            && !key.eq_ignore_ascii_case("transfer-encoding")
            && !key.eq_ignore_ascii_case("accept-encoding")
        {
            upstream_req = upstream_req.header(key.as_str(), value.as_str());
        }
    }
    if !req_body_bytes.is_empty() {
        upstream_req = upstream_req.body(req_body_bytes);
    }

    // Send upstream request
    let upstream_resp = match upstream_req.send().await {
        Ok(resp) => resp,
        Err(e) => {
            store_record(
                storage,
                broadcast,
                record_id,
                timestamp,
                method.to_string(),
                path,
                req_headers_json,
                req_body,
                req_size,
                None,
                None,
                Some(e.to_string()),                // 第三方原文，前端原样展示（R3）
                None,
                Some("upstream".to_string()),       // 事实：上游请求失败
                0,
                start.elapsed().as_millis() as i64,
                session_id,
                cli_id,
            );
            // 这句英文是**故意的**，不要本地化。同一次失败在应用侧的那一半由上面
            // `res_body` + `error_kind` 覆盖，这里的 502 体则交给被代理 CLI 的终端。
            // 三条理由：
            // 1) 消费方不是本应用的界面，而是被代理 CLI 的终端 —— 这段字符串进的是
            //    HTTP 502 响应体，由第三方程序原样打印在用户终端里（与例外表里的
            //    `.bat` 脚本同类：文案由 cmd.exe 打印，不由我们的 UI 渲染）。
            // 2) proxy 进程不感知语言：它不读语言设置，也不接收任何语言参数。要本地化
            //    它，就得新增一条跨进程的语言通道，并让 proxy 在编译期嵌入语言包 ——
            //    等于把 `native_text` 那套机制跨进程复制一份；何况 proxy 是长期存活的，
            //    还得处理「运行中语言被改」的陈旧问题。
            // 3) 应用界面那一半已被覆盖：同一次失败经上面 `res_body`（第三方原文）与
            //    `error_kind`（事实）落进流量库，由前端按当前语言渲染提示句。
            return Ok(error_response(502, &format!("Proxy error: {}", e)));
        }
    };

    let status = upstream_resp.status().as_u16() as i32;
    let res_headers_map = capture::headers_to_map(upstream_resp.headers());
    let res_headers_json = capture::sanitize_headers(&res_headers_map);

    let is_sse = upstream_resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ct| ct.contains("text/event-stream"));

    // 上游无视我们剥离 accept-encoding 而强制压缩时的兜底：reqwest 未启用
    // gzip/brotli 特性，拿到的是原始压缩字节，from_utf8_lossy 会把它们存成
    // 不可还原的乱码。这里显式检测并记录标记文本（转发给客户端的字节不受影响）。
    let compressed_encoding = res_headers_map
        .get("content-encoding")
        .map(|v| v.to_ascii_lowercase())
        .filter(|enc| enc != "identity" && !enc.is_empty());

    // Copy response headers
    let mut resp_builder = Response::builder().status(upstream_resp.status());
    for (key, value) in upstream_resp.headers() {
        if key != "transfer-encoding" {
            resp_builder = resp_builder.header(key, value);
        }
    }

    if is_sse {
        // SSE streaming: use channel-based tee.
        // Spawn a reader task that forwards chunks to client via channel
        // and collects the full body. After stream ends, store to SQLite.
        use futures_util::StreamExt;

        let mut byte_stream = upstream_resp.bytes_stream();
        let (tx, rx) = tokio::sync::mpsc::channel::<Result<Frame<Bytes>, Infallible>>(64);

        let storage = storage.clone();
        let broadcast = broadcast.clone();
        let session_id_clone = session_id.clone();
        let cli_id = cli_id.to_string();
        tokio::spawn(async move {
            let mut collected = Vec::new();
            while let Some(chunk_result) = byte_stream.next().await {
                match chunk_result {
                    Ok(chunk) => {
                        collected.extend_from_slice(&chunk);
                        if tx.send(Ok(Frame::data(chunk))).await.is_err() {
                            break; // client disconnected
                        }
                    }
                    Err(e) => {
                        tracing::error!("Upstream stream error: {}", e);
                        break;
                    }
                }
            }
            // Stream complete — store record
            // 压缩响应只记事实（`compression` 列），不往正文里写中文标记：
            // 正文会被流量检视界面原样渲染，写进去就把提示句的语言定死在了库里。
            let (res_body, compression) = match &compressed_encoding {
                Some(enc) => {
                    tracing::warn!(
                        "上游返回压缩响应 (content-encoding={})，仅记录元数据: {} {}",
                        enc,
                        method,
                        path
                    );
                    (None, Some(enc.clone()))
                }
                None => (
                    Some(String::from_utf8_lossy(&collected).to_string()),
                    None,
                ),
            };
            let res_size = collected.len() as i64;
            let duration_ms = start.elapsed().as_millis() as i64;
            store_record(
                &storage,
                &broadcast,
                record_id,
                timestamp,
                method.to_string(),
                path,
                req_headers_json,
                req_body,
                req_size,
                Some(status),
                Some(res_headers_json),
                res_body,
                compression,
                None,
                res_size,
                duration_ms,
                session_id_clone,
                &cli_id,
            );
        });

        let body_stream = tokio_stream::wrappers::ReceiverStream::new(rx);
        let stream_body = StreamBody::new(body_stream);
        let boxed: ResponseBody = BodyExt::boxed(stream_body);
        Ok(resp_builder.body(boxed)?)
    } else {
        // Non-streaming: read full body, store, return
        let res_body_bytes = upstream_resp.bytes().await?;
        // 同流式分支：压缩事实走 `compression` 列，正文留空（标记句由前端渲染）
        let (res_body, compression) = match &compressed_encoding {
            Some(enc) => {
                tracing::warn!(
                    "上游返回压缩响应 (content-encoding={})，仅记录元数据: {} {}",
                    enc,
                    method,
                    path
                );
                (None, Some(enc.clone()))
            }
            None => (
                Some(String::from_utf8_lossy(&res_body_bytes).to_string()),
                None,
            ),
        };
        let res_size = res_body_bytes.len() as i64;
        let duration_ms = start.elapsed().as_millis() as i64;

        store_record(
            storage,
            broadcast,
            record_id,
            timestamp,
            method.to_string(),
            path,
            req_headers_json,
            req_body,
            req_size,
            Some(status),
            Some(res_headers_json),
            res_body,
            compression,
            None,
            res_size,
            duration_ms,
            session_id,
            cli_id,
        );

        Ok(resp_builder.body(full_body(res_body_bytes))?)
    }
}

fn store_record(
    storage: &Storage,
    broadcast: &WsBroadcast,
    id: String,
    timestamp: String,
    method: String,
    path: String,
    req_headers: String,
    req_body: String,
    req_size: i64,
    status: Option<i32>,
    res_headers: Option<String>,
    res_body: Option<String>,
    compression: Option<String>,
    error_kind: Option<String>,
    res_size: i64,
    duration_ms: i64,
    session_id: Option<String>,
    cli_id: &str,
) {
    let record = TrafficRecord {
        id: id.clone(),
        timestamp,
        method,
        path,
        req_headers: Some(req_headers),
        req_body: Some(req_body),
        req_size,
        status,
        res_headers,
        res_body,
        compression,
        error_kind,
        res_size,
        duration_ms,
        session_id: session_id.clone(),
        cli_id: cli_id.to_string(),
    };
    if let Err(e) = storage.insert(&record) {
        tracing::error!("Failed to store traffic: {}", e);
    }

    // Notify WS clients: request completed
    broadcast.send(&WsEvent::Complete {
        id,
        status: record.status.unwrap_or(0),
        duration_ms,
        req_size,
        res_size,
        session_id,
    });
}

fn error_response(status: u16, msg: &str) -> Response<ResponseBody> {
    Response::builder()
        .status(status)
        .body(full_body(Bytes::from(msg.to_string())))
        .unwrap()
}

fn full_body(data: Bytes) -> ResponseBody {
    Full::new(data)
        .map_err(|_| -> Infallible { unreachable!() })
        .boxed()
}
