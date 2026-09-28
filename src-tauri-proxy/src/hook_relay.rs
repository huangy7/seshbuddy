use std::io::Read;
use std::path::Path;

pub fn run(socket: &Path, session_id: &str, event: &str) -> anyhow::Result<()> {
    let mut stdin = Vec::new();
    std::io::stdin().read_to_end(&mut stdin)?;
    forward_hook_payload(socket, session_id, event, &stdin)
}

#[cfg(unix)]
pub fn forward_hook_payload(
    socket: &Path,
    session_id: &str,
    event: &str,
    input: &[u8],
) -> anyhow::Result<()> {
    use std::io::Write;
    use std::os::unix::net::UnixStream;

    let mut payload: serde_json::Value =
        serde_json::from_slice(input).unwrap_or_else(|_| serde_json::json!({}));
    if !payload.is_object() {
        payload = serde_json::json!({});
    }

    let object = payload.as_object_mut().expect("payload is object");
    object.insert(
        "seshbuddy_session_id".to_string(),
        serde_json::Value::String(session_id.to_string()),
    );
    object.insert(
        "seshbuddy_hook_event".to_string(),
        serde_json::Value::String(event.to_string()),
    );

    let mut stream = UnixStream::connect(socket)?;
    writeln!(stream, "{}", serde_json::to_string(&payload)?)?;
    Ok(())
}

#[cfg(not(unix))]
pub fn forward_hook_payload(
    _socket: &Path,
    _session_id: &str,
    _event: &str,
    _input: &[u8],
) -> anyhow::Result<()> {
    Ok(())
}
