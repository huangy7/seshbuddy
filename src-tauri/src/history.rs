use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct HistoryEntry {
    pub display: String,
}

/// Parse history.jsonl and return two maps:
/// 1. session_id -> first HistoryEntry (earliest entry per session for display name)
/// 2. encoded_dir -> original_path (project path mapping)
pub fn parse_history(
    history_path: &str,
) -> (HashMap<String, HistoryEntry>, HashMap<String, String>) {
    let mut session_map: HashMap<String, HistoryEntry> = HashMap::new();
    let mut project_map: HashMap<String, String> = HashMap::new();

    let content = match std::fs::read_to_string(history_path) {
        Ok(c) => c,
        Err(_) => return (session_map, project_map),
    };

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let entry: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let session_id = match entry.get("sessionId").and_then(|v| v.as_str()) {
            Some(s) => s.to_string(),
            None => continue,
        };

        let display = entry
            .get("display")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let project = entry
            .get("project")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        // Use first (earliest) entry per session_id
        if !session_map.contains_key(&session_id) {
            session_map.insert(
                session_id.clone(),
                HistoryEntry {
                    display,
                },
            );
        }

        // Build project path mapping: encode the project path and map to original
        if !project.is_empty() {
            let encoded = encode_project_path(&project);
            project_map.entry(encoded).or_insert(project);
        }
    }

    (session_map, project_map)
}

pub fn parse_codex_history(history_path: &str) -> HashMap<String, HistoryEntry> {
    let mut session_map: HashMap<String, HistoryEntry> = HashMap::new();

    let content = match std::fs::read_to_string(history_path) {
        Ok(c) => c,
        Err(_) => return session_map,
    };

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let entry: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let session_id = match entry.get("session_id").and_then(|v| v.as_str()) {
            Some(s) => s.to_string(),
            None => continue,
        };

        let display = entry
            .get("text")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        session_map.entry(session_id).or_insert(HistoryEntry { display });
    }

    session_map
}

/// Encode a project path the same way Claude Code does.
/// Non-ASCII characters (e.g. Chinese) and special chars are each replaced with a single `-`.
fn encode_project_path(path: &str) -> String {
    path.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect()
}
