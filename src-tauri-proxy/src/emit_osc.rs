use serde_json::{json, Value};
use std::time::Duration;
use tokio::io::AsyncReadExt;

const STDIN_READ_TIMEOUT_MS: u64 = 100;

pub async fn run(event: &str) -> anyhow::Result<()> {
    let mut stdin = Vec::new();
    let read_result = tokio::time::timeout(
        Duration::from_millis(STDIN_READ_TIMEOUT_MS),
        tokio::io::stdin().read_to_end(&mut stdin),
    )
    .await;

    if let Ok(Err(e)) = read_result {
        tracing::warn!("[emit_osc] failed to read stdin: {}", e);
    }

    let payload: Value = serde_json::from_slice(&stdin).unwrap_or_else(|_| json!({}));

    let Some(osc_event) = determine_osc_event(event, &payload) else {
        return Ok(());
    };

    let osc = format!("\x1b]777;notify;SeshBuddy;{}\x07", osc_event);
    let output = json!({ "terminalSequence": osc });
    println!("{}", output);
    Ok(())
}

fn determine_osc_event(event: &str, payload: &Value) -> Option<&'static str> {
    let tool_name = payload.get("tool_name").and_then(|v| v.as_str());

    match event {
        "Notification" => match payload.get("notification_type").and_then(|v| v.as_str()) {
            Some("permission_prompt") | Some("idle_prompt") => Some("attention"),
            _ => None,
        },
        "Elicitation" => Some("attention"),
        "UserPromptSubmit" | "ElicitationResult" => Some("working"),
        "PreToolUse" if tool_name == Some("AskUserQuestion") => Some("attention"),
        "PreToolUse" | "PostToolUse" => Some("working"),
        "Stop" => Some("finished"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_events_to_osc_status() {
        assert_eq!(
            determine_osc_event(
                "Notification",
                &json!({"notification_type": "permission_prompt"})
            ),
            Some("attention")
        );
        assert_eq!(
            determine_osc_event("Elicitation", &json!({})),
            Some("attention")
        );
        assert_eq!(
            determine_osc_event("UserPromptSubmit", &json!({})),
            Some("working")
        );
        assert_eq!(
            determine_osc_event(
                "PreToolUse",
                &json!({"tool_name": "AskUserQuestion"})
            ),
            Some("attention")
        );
        assert_eq!(
            determine_osc_event("PreToolUse", &json!({"tool_name": "Read"})),
            Some("working")
        );
        assert_eq!(
            determine_osc_event("PostToolUse", &json!({"tool_name": "Read"})),
            Some("working")
        );
        assert_eq!(determine_osc_event("Stop", &json!({})), Some("finished"));
        assert_eq!(determine_osc_event("Notification", &json!({})), None);
    }

}
