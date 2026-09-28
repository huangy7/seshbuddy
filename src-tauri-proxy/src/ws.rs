use serde::Serialize;
use std::sync::Arc;
use tokio::sync::broadcast;

/// WebSocket event sent to all connected clients
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub enum WsEvent {
    /// Request started (in-flight)
    #[serde(rename = "pending")]
    Pending {
        id: String,
        timestamp: String,
        method: String,
        path: String,
        session_id: Option<String>,
    },
    /// Request completed
    #[serde(rename = "complete")]
    Complete {
        id: String,
        status: i32,
        duration_ms: i64,
        req_size: i64,
        res_size: i64,
        session_id: Option<String>,
    },
}

/// Broadcast hub: proxy sends events, WS clients receive them
#[derive(Clone)]
pub struct WsBroadcast {
    tx: broadcast::Sender<Arc<str>>,
}

impl WsBroadcast {
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self { tx }
    }

    /// Send an event to all connected WebSocket clients
    pub fn send(&self, event: &WsEvent) {
        if let Ok(json) = serde_json::to_string(event) {
            // Ignore error (no receivers connected)
            let _ = self.tx.send(json.into());
        }
    }

    /// Subscribe to receive events (called by each WS connection)
    pub fn subscribe(&self) -> broadcast::Receiver<Arc<str>> {
        self.tx.subscribe()
    }
}

/// Run the WebSocket server on the given address
pub async fn run_ws_server(listen: &str, broadcast: WsBroadcast) -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind(listen).await?;
    tracing::info!("WebSocket listening on ws://{}", listen);

    loop {
        let (stream, addr) = listener.accept().await?;
        let broadcast = broadcast.clone();

        tokio::spawn(async move {
            match tokio_tungstenite::accept_async(stream).await {
                Ok(ws_stream) => {
                    tracing::info!("WS client connected: {}", addr);
                    handle_ws_client(ws_stream, broadcast).await;
                    tracing::info!("WS client disconnected: {}", addr);
                }
                Err(e) => {
                    tracing::error!("WS handshake error from {}: {}", addr, e);
                }
            }
        });
    }
}

async fn handle_ws_client(
    ws_stream: tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
    broadcast: WsBroadcast,
) {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;

    let (mut ws_sink, mut ws_source) = ws_stream.split();
    let mut rx = broadcast.subscribe();

    loop {
        tokio::select! {
            // Forward broadcast events to this WS client
            result = rx.recv() => {
                match result {
                    Ok(json) => {
                        if ws_sink.send(Message::Text(json.to_string())).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!("WS client lagged, skipped {} messages", n);
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
            // Handle incoming messages from client (ping/pong, close)
            msg = ws_source.next() => {
                match msg {
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(Message::Ping(data))) => {
                        if ws_sink.send(Message::Pong(data)).await.is_err() {
                            break;
                        }
                    }
                    _ => {} // ignore text/binary from client
                }
            }
        }
    }
}
