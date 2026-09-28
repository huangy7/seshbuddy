use serde::{Deserialize, Serialize};

use crate::pty_manager::PtyStatus;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PtyStatusSource {
    Hook,
    FrontendFallback,
}

pub fn next_status(current: PtyStatus, requested: PtyStatus) -> Option<PtyStatus> {
    if current == PtyStatus::Exited && requested != PtyStatus::Exited {
        return None;
    }
    if current == requested {
        return None;
    }
    Some(requested)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_source_serializes_as_camel_payload_value() {
        let value = serde_json::to_value(PtyStatusSource::FrontendFallback).unwrap();
        assert_eq!(value, serde_json::json!("frontend_fallback"));
    }

    #[test]
    fn exited_status_rejects_later_non_exit_updates() {
        assert_eq!(next_status(PtyStatus::Exited, PtyStatus::Active), None);
        assert_eq!(
            next_status(PtyStatus::Active, PtyStatus::WaitingInput),
            Some(PtyStatus::WaitingInput)
        );
    }
}
