use super::{conn, now_rfc3339};
use crate::error::{AppError, AppResult};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::collections::HashMap;

pub(super) const APP_SETTING_DOCK_VISIBLE: &str = "dock_visible";
const APP_SETTING_CLI_PATH_OVERRIDES: &str = "cli_path_overrides";
const APP_SETTING_UPDATER_SETTINGS: &str = "updater.settings";
const APP_SETTING_INDEX_LAST_UPDATED_PREFIX: &str = "index.last_updated_ms";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdaterSettings {
    pub auto_check: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ignored_update_version: Option<String>,
}

impl Default for UpdaterSettings {
    fn default() -> Self {
        Self {
            auto_check: true,
            ignored_update_version: None,
        }
    }
}

pub(super) fn remove_setting(key: &str) -> AppResult<()> {
    let conn = conn()?;
    conn.execute("DELETE FROM app_settings WHERE key = ?1", params![key])?;
    Ok(())
}

pub(crate) fn read_setting_json<T: DeserializeOwned>(key: &str) -> AppResult<Option<T>> {
    let conn = conn()?;
    read_setting_json_inner(&conn, key)
}

pub(super) fn read_setting_json_inner<T: DeserializeOwned>(
    conn: &Connection,
    key: &str,
) -> AppResult<Option<T>> {
    let raw: Option<String> = conn
        .query_row(
            "SELECT value_json FROM app_settings WHERE key = ?1",
            params![key],
            |row| row.get(0),
        )
        .optional()?;

    raw.map(|content| serde_json::from_str(&content).map_err(AppError::from))
        .transpose()
}

pub(crate) fn write_setting_json<T: Serialize>(key: &str, value: &T) -> AppResult<()> {
    let conn = conn()?;
    write_setting_json_inner(&conn, key, value)
}

pub(super) fn write_setting_json_inner<T: Serialize>(
    conn: &Connection,
    key: &str,
    value: &T,
) -> AppResult<()> {
    let now = now_rfc3339();
    let json = serde_json::to_string(value)?;
    conn.execute(
        r#"
        INSERT INTO app_settings (key, value_json, updated_at)
        VALUES (?1, ?2, ?3)
        ON CONFLICT(key) DO UPDATE SET
            value_json = excluded.value_json,
            updated_at = excluded.updated_at
        "#,
        params![key, json, now],
    )?;
    Ok(())
}

pub(crate) fn read_updater_settings() -> AppResult<UpdaterSettings> {
    Ok(read_setting_json::<UpdaterSettings>(APP_SETTING_UPDATER_SETTINGS)?.unwrap_or_default())
}

pub(crate) fn write_updater_settings(settings: &UpdaterSettings) -> AppResult<UpdaterSettings> {
    write_setting_json(APP_SETTING_UPDATER_SETTINGS, settings)?;
    Ok(settings.clone())
}




pub(crate) fn read_dock_visible() -> AppResult<bool> {
    Ok(read_setting_json::<bool>(APP_SETTING_DOCK_VISIBLE)?.unwrap_or(true))
}

pub(crate) fn write_dock_visible(visible: bool) -> AppResult<()> {
    write_setting_json(APP_SETTING_DOCK_VISIBLE, &visible)
}

fn index_last_updated_key(cli_id: &str) -> String {
    format!("{}.{}", APP_SETTING_INDEX_LAST_UPDATED_PREFIX, cli_id)
}

pub(crate) fn write_index_last_updated(
    conn: &Connection,
    cli_id: &str,
    ms: i64,
) -> AppResult<()> {
    write_setting_json_inner(conn, &index_last_updated_key(cli_id), &ms)
}

fn read_index_last_updated_inner(conn: &Connection, cli_id: &str) -> AppResult<Option<i64>> {
    read_setting_json_inner(conn, &index_last_updated_key(cli_id))
}

pub(crate) fn read_index_last_updated(cli_id: &str) -> AppResult<Option<i64>> {
    let conn = conn()?;
    if let Some(ms) = read_index_last_updated_inner(&conn, cli_id)? {
        return Ok(Some(ms));
    }
    let max_ms: Option<i64> = conn
        .query_row(
            "SELECT MAX(modified_ms) FROM session_search_index_state WHERE cli_id = ?1",
            params![cli_id],
            |row| row.get(0),
        )
        .optional()?
        .flatten();
    Ok(max_ms)
}

pub(crate) fn read_cli_path_overrides() -> AppResult<HashMap<String, String>> {
    Ok(read_setting_json::<HashMap<String, String>>(APP_SETTING_CLI_PATH_OVERRIDES)?
        .unwrap_or_default())
}

pub(crate) fn write_cli_path_overrides(overrides: &HashMap<String, String>) -> AppResult<()> {
    if overrides.is_empty() {
        return remove_setting(APP_SETTING_CLI_PATH_OVERRIDES);
    }
    write_setting_json(APP_SETTING_CLI_PATH_OVERRIDES, overrides)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_last_updated_roundtrip() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE app_settings (
                key TEXT PRIMARY KEY,
                value_json TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );",
        )
        .unwrap();

        assert!(read_index_last_updated_inner(&conn, "claude").unwrap().is_none());

        write_index_last_updated(&conn, "claude", 1720000000000).unwrap();
        assert_eq!(
            read_index_last_updated_inner(&conn, "claude").unwrap(),
            Some(1720000000000)
        );

        // 覆盖写
        write_index_last_updated(&conn, "claude", 1720000001000).unwrap();
        assert_eq!(
            read_index_last_updated_inner(&conn, "claude").unwrap(),
            Some(1720000001000)
        );

        // 不同 cli 互不影响
        assert!(read_index_last_updated_inner(&conn, "codex").unwrap().is_none());
    }
}
