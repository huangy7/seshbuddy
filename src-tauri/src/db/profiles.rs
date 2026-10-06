use super::{conn, now_rfc3339, write_tx};
use crate::cli::CliKind;
use crate::error::{AppError, AppResult};
use nanoid::nanoid;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;

fn normalize_profile_name(name: &str) -> String {
    name.trim().to_lowercase()
}

fn normalize_profile_input(name: &str) -> AppResult<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(AppError::coded("profile.name_required"));
    }
    Ok(trimmed.to_string())
}

pub(crate) fn list_profiles(kind: CliKind) -> AppResult<Vec<String>> {
    let conn = conn()?;
    list_profiles_with(&conn, kind)
}

pub(crate) fn list_profiles_with(conn: &Connection, kind: CliKind) -> AppResult<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT name FROM profiles WHERE cli_id = ?1 AND scope = 'global' ORDER BY normalized_name ASC, name ASC",
    )?;

    let rows = stmt.query_map(params![kind.id()], |row| row.get::<_, String>(0))?;

    let mut names = Vec::new();
    for row in rows {
        names.push(row?);
    }
    Ok(names)
}

pub(crate) fn read_profile(kind: CliKind, name: &str) -> AppResult<String> {
    let conn = conn()?;
    read_profile_with(&conn, kind, name)
}

pub(crate) fn read_profile_with(conn: &Connection, kind: CliKind, name: &str) -> AppResult<String> {
    let normalized = normalize_profile_name(name);
    conn.query_row(
        "SELECT content_json FROM profiles WHERE cli_id = ?1 AND scope = 'global' AND normalized_name = ?2",
        params![kind.id(), normalized],
        |row| row.get(0),
    )
    .optional()
    .map_err(AppError::from)?
    .ok_or_else(|| AppError::coded("profile.not_found").with("name", name.trim()))
}

pub(crate) fn read_profile_record_with(
    conn: &Connection,
    kind: CliKind,
    name: &str,
) -> AppResult<Option<(String, String)>> {
    let normalized = normalize_profile_name(name);
    conn.query_row(
        "SELECT name, content_json FROM profiles WHERE cli_id = ?1 AND scope = 'global' AND normalized_name = ?2",
        params![kind.id(), normalized],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .optional()
    .map_err(AppError::from)
}

pub(crate) fn save_profile(kind: CliKind, name: &str, content: &str) -> AppResult<()> {
    let mut conn = conn()?;
    save_profile_with(&mut conn, kind, name, content)
}

pub(crate) fn save_profile_with(
    conn: &mut Connection,
    kind: CliKind,
    name: &str,
    content: &str,
) -> AppResult<()> {
    let name = normalize_profile_input(name)?;
    serde_json::from_str::<Value>(content)?;

    let normalized = normalize_profile_name(&name);
    let tx = write_tx(conn)?;

    let created_at: Option<String> = tx
        .query_row(
            "SELECT created_at FROM profiles WHERE cli_id = ?1 AND scope = 'global' AND normalized_name = ?2",
            params![kind.id(), normalized],
            |row| row.get(0),
        )
        .optional()?;

    let now = now_rfc3339();
    tx.execute(
        r#"
        INSERT INTO profiles (cli_id, scope, name, normalized_name, content_json, created_at, updated_at)
        VALUES (?1, 'global', ?2, ?3, ?4, ?5, ?6)
        ON CONFLICT(cli_id, scope, normalized_name) DO UPDATE SET
            name = excluded.name,
            content_json = excluded.content_json,
            updated_at = excluded.updated_at
        "#,
        params![
            kind.id(),
            name,
            normalized,
            content,
            created_at.unwrap_or_else(|| now.clone()),
            now,
        ],
    )?;

    // Synchronize active profile name casing across all scopes where this profile is active
    let active_rows: Vec<(String, String)> = {
        let mut active_stmt = tx.prepare(
            "SELECT scope, profile_name FROM active_profiles WHERE cli_id = ?1",
        )?;
        let rows = active_stmt
            .query_map(params![kind.id()], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };

    for (scope, prof_name) in active_rows {
        if normalize_profile_name(&prof_name) == normalized && prof_name != name {
            tx.execute(
                "UPDATE active_profiles SET profile_name = ?1, updated_at = ?2 WHERE cli_id = ?3 AND scope = ?4",
                params![name, now, kind.id(), scope],
            )?;
        }
    }

    tx.commit()?;
    Ok(())
}

pub(crate) fn delete_profile(kind: CliKind, name: &str) -> AppResult<()> {
    let mut conn = conn()?;
    delete_profile_with(&mut conn, kind, name)
}

pub(crate) fn delete_profile_with(conn: &mut Connection, kind: CliKind, name: &str) -> AppResult<()> {
    let normalized = normalize_profile_name(name);
    let tx = write_tx(conn)?;

    let deleted: bool = tx
        .query_row(
            "SELECT COUNT(*) > 0 FROM profiles WHERE cli_id = ?1 AND scope = 'global' AND normalized_name = ?2",
            params![kind.id(), normalized],
            |row| row.get(0),
        )?;

    if !deleted {
        return Ok(());
    }

    tx.execute(
        "DELETE FROM profiles WHERE cli_id = ?1 AND scope = 'global' AND normalized_name = ?2",
        params![kind.id(), normalized],
    )?;

    let active_rows: Vec<(String, String)> = {
        let mut active_stmt = tx.prepare(
            "SELECT scope, profile_name FROM active_profiles WHERE cli_id = ?1",
        )?;
        let rows = active_stmt
            .query_map(params![kind.id()], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };

    for (scope, prof_name) in active_rows {
        if normalize_profile_name(&prof_name) == normalized {
            tx.execute(
                "DELETE FROM active_profiles WHERE cli_id = ?1 AND scope = ?2",
                params![kind.id(), scope],
            )?;
        }
    }

    tx.commit()?;
    Ok(())
}

pub(crate) fn rename_profile(kind: CliKind, old_name: &str, new_name: &str) -> AppResult<()> {
    let mut conn = conn()?;
    rename_profile_with(&mut conn, kind, old_name, new_name)
}

pub(crate) fn rename_profile_with(
    conn: &mut Connection,
    kind: CliKind,
    old_name: &str,
    new_name: &str,
) -> AppResult<()> {
    let old_name = normalize_profile_input(old_name)?;
    let new_name = normalize_profile_input(new_name)?;
    let old_normalized = normalize_profile_name(&old_name);
    let new_normalized = normalize_profile_name(&new_name);

    let tx = write_tx(conn)?;

    let existing_old: Option<String> = tx
        .query_row(
            "SELECT name FROM profiles WHERE cli_id = ?1 AND scope = 'global' AND normalized_name = ?2",
            params![kind.id(), old_normalized],
            |row| row.get(0),
        )
        .optional()?;

    let existing_old = existing_old.ok_or_else(|| AppError::coded("profile.not_found").with("name", old_name))?;

    let now = now_rfc3339();
    if old_normalized == new_normalized {
        if existing_old != new_name {
            tx.execute(
                "UPDATE profiles SET name = ?3, updated_at = ?4 WHERE cli_id = ?1 AND scope = 'global' AND normalized_name = ?2",
                params![kind.id(), old_normalized, new_name, now],
            )?;
        }
    } else {
        let conflict: Option<String> = tx
            .query_row(
                "SELECT name FROM profiles WHERE cli_id = ?1 AND scope = 'global' AND normalized_name = ?2",
                params![kind.id(), new_normalized],
                |row| row.get(0),
            )
            .optional()?;

        if let Some(conflict_name) = conflict {
            return Err(AppError::coded("profile.name_taken").with("name", conflict_name));
        }

        tx.execute(
            "UPDATE profiles SET name = ?3, normalized_name = ?4, updated_at = ?5 WHERE cli_id = ?1 AND scope = 'global' AND normalized_name = ?2",
            params![kind.id(), old_normalized, new_name, new_normalized, now],
        )?;
    }

    let active_rows: Vec<(String, String)> = {
        let mut active_stmt = tx.prepare(
            "SELECT scope, profile_name FROM active_profiles WHERE cli_id = ?1",
        )?;
        let rows = active_stmt
            .query_map(params![kind.id()], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };

    for (scope, prof_name) in active_rows {
        if normalize_profile_name(&prof_name) == old_normalized {
            tx.execute(
                "UPDATE active_profiles SET profile_name = ?1, updated_at = ?2 WHERE cli_id = ?3 AND scope = ?4",
                params![new_name, now, kind.id(), scope],
            )?;
        }
    }

    tx.commit()?;
    Ok(())
}

pub(crate) fn get_active_scopes_for_profile_with(
    conn: &Connection,
    kind: CliKind,
    name: &str,
) -> AppResult<Vec<String>> {
    let normalized = normalize_profile_name(name);
    let mut stmt = conn.prepare(
        "SELECT scope, profile_name FROM active_profiles WHERE cli_id = ?1 ORDER BY scope ASC",
    )?;
    let rows = stmt.query_map(params![kind.id()], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;

    let mut scopes = Vec::new();
    for r in rows {
        let (scope, profile_name) = r?;
        if normalize_profile_name(&profile_name) == normalized {
            scopes.push(scope);
        }
    }
    Ok(scopes)
}

pub(crate) fn get_active_profile(kind: CliKind, scope: Option<&str>) -> AppResult<String> {
    let conn = conn()?;
    get_active_profile_with(&conn, kind, scope)
}

pub(crate) fn get_active_profile_with(
    conn: &Connection,
    kind: CliKind,
    scope: Option<&str>,
) -> AppResult<String> {
    let scope = scope.unwrap_or("global");
    Ok(conn
        .query_row(
            "SELECT profile_name FROM active_profiles WHERE cli_id = ?1 AND scope = ?2",
            params![kind.id(), scope],
            |row| row.get(0),
        )
        .optional()?
        .unwrap_or_default())
}

pub(crate) fn set_active_profile(kind: CliKind, name: &str, scope: Option<&str>) -> AppResult<()> {
    let conn = conn()?;
    set_active_profile_with(&conn, kind, name, scope)
}

pub(crate) fn set_active_profile_with(
    conn: &Connection,
    kind: CliKind,
    name: &str,
    scope: Option<&str>,
) -> AppResult<()> {
    let scope = scope.unwrap_or("global");
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(AppError::coded("profile.name_required"));
    }

    let (canonical_name, _) = read_profile_record_with(conn, kind, trimmed)?
        .ok_or_else(|| AppError::coded("profile.not_found").with("name", trimmed))?;

    conn.execute(
        r#"
        INSERT INTO active_profiles (cli_id, scope, profile_name, updated_at)
        VALUES (?1, ?2, ?3, ?4)
        ON CONFLICT(cli_id, scope) DO UPDATE SET
            profile_name = excluded.profile_name,
            updated_at = excluded.updated_at
        "#,
        params![kind.id(), scope, canonical_name, now_rfc3339()],
    )?;
    Ok(())
}

pub(crate) fn clear_active_profile(kind: CliKind, scope: Option<&str>) -> AppResult<()> {
    let conn = conn()?;
    clear_active_profile_with(&conn, kind, scope)
}

pub(crate) fn clear_active_profile_with(
    conn: &Connection,
    kind: CliKind,
    scope: Option<&str>,
) -> AppResult<()> {
    let scope = scope.unwrap_or("global");
    conn.execute(
        "DELETE FROM active_profiles WHERE cli_id = ?1 AND scope = ?2",
        params![kind.id(), scope],
    )?;
    Ok(())
}

#[derive(Debug, serde::Serialize, serde::Deserialize, Clone)]
pub struct TabInfo {
    pub id: String,
    pub cli_id: String,
    pub name: String,
    pub sort_order: i32,
    pub dirs: Vec<String>,
}

pub(crate) fn create_profile_tab(cli_id: &str, name: &str, dirs: &[String]) -> AppResult<TabInfo> {
    let mut conn = conn()?;
    create_profile_tab_with(&mut conn, cli_id, name, dirs)
}

pub(crate) fn create_profile_tab_with(
    conn: &mut Connection,
    cli_id: &str,
    name: &str,
    dirs: &[String],
) -> AppResult<TabInfo> {
    let name = name.trim();
    if name.is_empty() {
        return Err(AppError::coded("profile.tab_name_required"));
    }

    let id = nanoid!(10);
    let tx = write_tx(conn)?;

    let sort_order: i32 = tx.query_row(
        "SELECT COALESCE(MAX(sort_order), -1) + 1 FROM profile_tabs WHERE cli_id = ?1",
        params![cli_id],
        |row| row.get(0),
    )?;

    let now = now_rfc3339();
    tx.execute(
        "INSERT INTO profile_tabs (id, cli_id, name, sort_order, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![id, cli_id, name, sort_order, now, now],
    )?;

    for dir in dirs {
        tx.execute(
            "INSERT OR IGNORE INTO profile_tab_dirs (tab_id, dir_path) VALUES (?1, ?2)",
            params![id, dir],
        )?;
    }

    tx.commit()?;

    Ok(TabInfo {
        id,
        cli_id: cli_id.to_string(),
        name: name.to_string(),
        sort_order,
        dirs: dirs.to_vec(),
    })
}

pub(crate) fn update_profile_tab(
    tab_id: &str,
    name: Option<String>,
    dirs: Option<Vec<String>>,
) -> AppResult<TabInfo> {
    let mut conn = conn()?;
    update_profile_tab_with(&mut conn, tab_id, name, dirs)
}

pub(crate) fn update_profile_tab_with(
    conn: &mut Connection,
    tab_id: &str,
    name: Option<String>,
    dirs: Option<Vec<String>>,
) -> AppResult<TabInfo> {
    let tx = write_tx(conn)?;

    let existing = tx
        .query_row(
            "SELECT cli_id, name, sort_order FROM profile_tabs WHERE id = ?1",
            params![tab_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i32>(2)?,
                ))
            },
        )
        .optional()?;

    let (cli_id, current_name, sort_order) =
        existing.ok_or_else(|| AppError::coded("profile.tab_not_found"))?;

    if let Some(ref n) = name {
        if n.trim().is_empty() {
            return Err(AppError::coded("profile.tab_name_required"));
        }
    }
    let target_name = name
        .as_ref()
        .map(|n| n.trim().to_string())
        .unwrap_or_else(|| current_name.clone());

    let now = now_rfc3339();
    if name.is_some() {
        tx.execute(
            "UPDATE profile_tabs SET name = ?1, updated_at = ?2 WHERE id = ?3",
            params![&target_name, now, tab_id],
        )?;
    } else if dirs.is_some() {
        tx.execute(
            "UPDATE profile_tabs SET updated_at = ?1 WHERE id = ?2",
            params![now, tab_id],
        )?;
    }

    let final_dirs = if let Some(new_dirs) = dirs {
        tx.execute(
            "DELETE FROM profile_tab_dirs WHERE tab_id = ?1",
            params![tab_id],
        )?;
        for dir in &new_dirs {
            tx.execute(
                "INSERT OR IGNORE INTO profile_tab_dirs (tab_id, dir_path) VALUES (?1, ?2)",
                params![tab_id, dir],
            )?;
        }
        new_dirs
    } else {
        let mut current_dirs = Vec::new();
        {
            let mut dir_stmt = tx.prepare("SELECT dir_path FROM profile_tab_dirs WHERE tab_id = ?1")?;
            let rows = dir_stmt.query_map(params![tab_id], |row| row.get::<_, String>(0))?;
            for r in rows {
                current_dirs.push(r?);
            }
        }
        current_dirs
    };

    tx.commit()?;

    Ok(TabInfo {
        id: tab_id.to_string(),
        cli_id,
        name: target_name,
        sort_order,
        dirs: final_dirs,
    })
}

pub(crate) fn delete_profile_tab(tab_id: &str) -> AppResult<()> {
    let mut conn = conn()?;
    delete_profile_tab_with(&mut conn, tab_id)
}

pub(crate) fn delete_profile_tab_with(conn: &mut Connection, tab_id: &str) -> AppResult<()> {
    if tab_id == "global" {
        return Err(AppError::coded("profile.global_tab_undeletable"));
    }

    let tx = write_tx(conn)?;

    tx.execute(
        "DELETE FROM active_profiles WHERE scope = ?1",
        params![tab_id],
    )?;
    tx.execute(
        "DELETE FROM profile_tab_dirs WHERE tab_id = ?1",
        params![tab_id],
    )?;
    tx.execute("DELETE FROM profile_tabs WHERE id = ?1", params![tab_id])?;

    tx.commit()?;
    Ok(())
}

pub(crate) fn list_profile_tabs(cli_id: &str) -> AppResult<Vec<TabInfo>> {
    let conn = conn()?;
    list_profile_tabs_with(&conn, cli_id)
}

pub(crate) fn list_profile_tabs_with(conn: &Connection, cli_id: &str) -> AppResult<Vec<TabInfo>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, sort_order FROM profile_tabs WHERE cli_id = ?1 ORDER BY sort_order ASC",
    )?;

    let rows = stmt.query_map(params![cli_id], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i32>(2)?,
        ))
    })?;

    let mut tabs = Vec::new();
    for r in rows {
        let (id, name, sort_order) = r?;
        let dirs = get_profile_tab_dirs_with(conn, &id)?;
        tabs.push(TabInfo {
            id,
            cli_id: cli_id.to_string(),
            name,
            sort_order,
            dirs,
        });
    }

    Ok(tabs)
}

pub(crate) fn get_profile_tab_dirs_with(conn: &Connection, tab_id: &str) -> AppResult<Vec<String>> {
    let mut stmt = conn.prepare("SELECT dir_path FROM profile_tab_dirs WHERE tab_id = ?1")?;
    let rows = stmt.query_map(params![tab_id], |row| row.get::<_, String>(0))?;
    let mut dirs = Vec::new();
    for r in rows {
        dirs.push(r?);
    }
    Ok(dirs)
}

#[derive(Debug, serde::Serialize, serde::Deserialize, Clone)]
pub struct RawProfileRecord {
    pub cli_id: String,
    pub name: String,
    pub content: String,
    pub is_active: bool,
}

pub(crate) fn export_all_profiles() -> AppResult<Vec<RawProfileRecord>> {
    let conn = conn()?;
    export_all_profiles_with(&conn)
}

pub(crate) fn export_all_profiles_with(conn: &Connection) -> AppResult<Vec<RawProfileRecord>> {
    let mut stmt = conn.prepare(
        "SELECT cli_id, name, content_json FROM profiles WHERE scope = 'global' ORDER BY cli_id, name",
    )?;

    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;

    let mut records = Vec::new();
    for r in rows {
        let (cli_id, name, content) = r?;
        let is_active = conn
            .query_row(
                "SELECT 1 FROM active_profiles WHERE cli_id = ?1 AND lower(profile_name) = ?2 AND scope = 'global'",
                params![&cli_id, normalize_profile_name(&name)],
                |_row| Ok(true),
            )
            .optional()?
            .unwrap_or(false);

        records.push(RawProfileRecord {
            cli_id,
            name,
            content,
            is_active,
        });
    }

    Ok(records)
}

/// 导出作用域绑定：active_profiles 中非 global 的行 → (cli_id, scope, profile_name)
pub(crate) fn export_scope_bindings() -> AppResult<Vec<(String, String, String)>> {
    let conn = conn()?;
    export_scope_bindings_with(&conn)
}

pub(crate) fn export_scope_bindings_with(
    conn: &Connection,
) -> AppResult<Vec<(String, String, String)>> {
    let mut stmt = conn.prepare(
        "SELECT cli_id, scope, profile_name FROM active_profiles WHERE scope != 'global' ORDER BY cli_id, scope",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

/// 恢复作用域绑定：仅恢复配置确实被导入的绑定（重命名时映射到新名），其余跳过。
/// imported_names 键为 (cli_id, 归一化原始名)，值为导入后的目标名。
pub(crate) fn restore_scope_bindings(
    bindings: &[(String, String, String)],
    imported_names: &std::collections::HashMap<(String, String), String>,
) -> AppResult<()> {
    let conn = conn()?;
    restore_scope_bindings_with(&conn, bindings, imported_names)
}

pub(crate) fn restore_scope_bindings_with(
    conn: &Connection,
    bindings: &[(String, String, String)],
    imported_names: &std::collections::HashMap<(String, String), String>,
) -> AppResult<()> {
    for (cli_id, scope, profile_name) in bindings {
        let key = (cli_id.clone(), normalize_profile_name(profile_name));
        let Some(target) = imported_names.get(&key) else {
            continue;
        };
        let Ok(kind) = crate::cli::CliKind::from_id(Some(cli_id)) else {
            continue;
        };
        // 作用域 tab 不存在时跳过（用户在导入时跳过了该作用域），避免悬空绑定
        let tab_exists: bool = conn.query_row(
            "SELECT COUNT(*) > 0 FROM profile_tabs WHERE id = ?1",
            params![scope],
            |r| r.get(0),
        )?;
        if !tab_exists {
            continue;
        }
        set_active_profile_with(conn, kind, target, Some(scope))?;
    }
    Ok(())
}

pub(crate) fn export_all_profile_tabs() -> AppResult<Vec<TabInfo>> {
    let conn = conn()?;
    export_all_profile_tabs_with(&conn)
}

pub(crate) fn export_all_profile_tabs_with(conn: &Connection) -> AppResult<Vec<TabInfo>> {
    let mut stmt = conn.prepare(
        "SELECT id, cli_id, name, sort_order FROM profile_tabs ORDER BY cli_id, sort_order ASC",
    )?;

    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, i32>(3)?,
        ))
    })?;

    let mut tabs = Vec::new();
    for r in rows {
        let (id, cli_id, name, sort_order) = r?;
        let dirs = get_profile_tab_dirs_with(conn, &id)?;
        tabs.push(TabInfo {
            id,
            cli_id,
            name,
            sort_order,
            dirs,
        });
    }

    Ok(tabs)
}

pub(crate) fn import_profile_tab(tab: &TabInfo) -> AppResult<()> {
    let mut conn = conn()?;
    import_profile_tab_with(&mut conn, tab)
}

pub(crate) fn import_profile_tab_with(conn: &mut Connection, tab: &TabInfo) -> AppResult<()> {
    let tx = write_tx(conn)?;
    let now = now_rfc3339();

    tx.execute(
        "INSERT INTO profile_tabs (id, cli_id, name, sort_order, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(id) DO UPDATE SET name = excluded.name, sort_order = excluded.sort_order, updated_at = excluded.updated_at",
        params![&tab.id, &tab.cli_id, &tab.name, tab.sort_order, &now, &now],
    )?;

    tx.execute("DELETE FROM profile_tab_dirs WHERE tab_id = ?1", params![&tab.id])?;
    for dir in &tab.dirs {
        tx.execute(
            "INSERT OR IGNORE INTO profile_tab_dirs (tab_id, dir_path) VALUES (?1, ?2)",
            params![&tab.id, dir],
        )?;
    }

    tx.commit()?;
    Ok(())
}

pub(crate) fn ensure_profile_tab_exists(cli_id: &str, scope: &str) -> AppResult<()> {
    let conn = conn()?;
    ensure_profile_tab_exists_with(&conn, cli_id, scope)
}

pub(crate) fn ensure_profile_tab_exists_with(
    conn: &Connection,
    cli_id: &str,
    scope: &str,
) -> AppResult<()> {
    if scope == "global" {
        return Ok(());
    }
    let exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM profile_tabs WHERE id = ?1)",
        params![scope],
        |row| row.get(0),
    )?;
    if !exists {
        let now = now_rfc3339();
        let sort_order: i32 = conn
            .query_row(
                "SELECT COALESCE(MAX(sort_order), -1) + 1 FROM profile_tabs WHERE cli_id = ?1",
                params![cli_id],
                |row| row.get(0),
            )
            .unwrap_or(0);

        conn.execute(
            "INSERT OR IGNORE INTO profile_tabs (id, cli_id, name, sort_order, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![scope, cli_id, scope, sort_order, &now, &now],
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_test_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE profiles (
                cli_id TEXT NOT NULL,
                scope TEXT NOT NULL DEFAULT 'global',
                name TEXT NOT NULL,
                normalized_name TEXT NOT NULL,
                content_json TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                PRIMARY KEY (cli_id, scope, normalized_name)
            );

            CREATE TABLE active_profiles (
                cli_id TEXT NOT NULL,
                profile_name TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                scope TEXT NOT NULL DEFAULT 'global',
                PRIMARY KEY (cli_id, scope)
            );

            CREATE TABLE profile_tabs (
                id TEXT PRIMARY KEY,
                cli_id TEXT NOT NULL,
                name TEXT NOT NULL,
                sort_order INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE TABLE profile_tab_dirs (
                tab_id TEXT NOT NULL,
                dir_path TEXT NOT NULL,
                PRIMARY KEY (tab_id, dir_path),
                FOREIGN KEY (tab_id) REFERENCES profile_tabs(id) ON DELETE CASCADE
            );
            "#,
        )
        .unwrap();
        conn
    }

    #[test]
    fn test_profile_name_normalization() {
        assert_eq!(normalize_profile_name("  MyProfile  "), "myprofile");
        assert_eq!(normalize_profile_input("  abc  ").unwrap(), "abc");
        assert!(normalize_profile_input("   ").is_err());
    }

    #[test]
    fn test_profile_crud_operations() {
        let mut conn = setup_test_db();
        let kind = CliKind::Claude;

        // 1. Save profile
        save_profile_with(&mut conn, kind, "Work", "{\"baseUrl\":\"https://api.anthropic.com\"}").unwrap();
        assert_eq!(list_profiles_with(&conn, kind).unwrap(), vec!["Work"]);

        // 2. Read profile
        let content = read_profile_with(&conn, kind, "Work").unwrap();
        assert!(content.contains("https://api.anthropic.com"));

        let record = read_profile_record_with(&conn, kind, "work").unwrap();
        assert!(record.is_some());
        let (name, rec_content) = record.unwrap();
        assert_eq!(name, "Work");
        assert_eq!(rec_content, content);

        // 3. Rename profile
        rename_profile_with(&mut conn, kind, "Work", "WorkPrimary").unwrap();
        assert_eq!(list_profiles_with(&conn, kind).unwrap(), vec!["WorkPrimary"]);
        assert!(read_profile_with(&conn, kind, "Work").is_err());
        assert!(read_profile_with(&conn, kind, "WorkPrimary").is_ok());

        // 4. Rename casing only
        rename_profile_with(&mut conn, kind, "workprimary", "WORKPRIMARY").unwrap();
        let record = read_profile_record_with(&conn, kind, "workprimary").unwrap().unwrap();
        assert_eq!(record.0, "WORKPRIMARY");

        // 5. Delete profile
        delete_profile_with(&mut conn, kind, "WORKPRIMARY").unwrap();
        assert!(list_profiles_with(&conn, kind).unwrap().is_empty());
    }

    #[test]
    fn test_active_profile_and_scopes_sync() {
        let mut conn = setup_test_db();
        let kind = CliKind::Claude;

        save_profile_with(&mut conn, kind, "Default", "{\"model\":\"claude-3\"}").unwrap();

        // Activate on global and tab_work
        set_active_profile_with(&conn, kind, "Default", None).unwrap();
        set_active_profile_with(&conn, kind, "default", Some("tab_work")).unwrap();

        assert_eq!(get_active_profile_with(&conn, kind, None).unwrap(), "Default");
        assert_eq!(get_active_profile_with(&conn, kind, Some("tab_work")).unwrap(), "Default");

        let scopes = get_active_scopes_for_profile_with(&conn, kind, "Default").unwrap();
        assert_eq!(scopes, vec!["global", "tab_work"]);

        // Save profile with new casing -> active_profiles updated
        save_profile_with(&mut conn, kind, "DEFAULT", "{\"model\":\"claude-3-opus\"}").unwrap();
        assert_eq!(get_active_profile_with(&conn, kind, None).unwrap(), "DEFAULT");
        assert_eq!(get_active_profile_with(&conn, kind, Some("tab_work")).unwrap(), "DEFAULT");

        // Rename profile -> active_profiles updated
        rename_profile_with(&mut conn, kind, "DEFAULT", "ClaudeMain").unwrap();
        assert_eq!(get_active_profile_with(&conn, kind, None).unwrap(), "ClaudeMain");
        assert_eq!(get_active_profile_with(&conn, kind, Some("tab_work")).unwrap(), "ClaudeMain");

        // Delete profile -> active_profiles references cleared
        delete_profile_with(&mut conn, kind, "ClaudeMain").unwrap();
        assert_eq!(get_active_profile_with(&conn, kind, None).unwrap(), "");
        assert_eq!(get_active_profile_with(&conn, kind, Some("tab_work")).unwrap(), "");
        assert!(get_active_scopes_for_profile_with(&conn, kind, "ClaudeMain").unwrap().is_empty());
    }

    #[test]
    fn test_tab_decoupling_preserves_profiles() {
        let mut conn = setup_test_db();
        let kind = CliKind::Claude;

        save_profile_with(&mut conn, kind, "SharedProfile", "{\"baseUrl\":\"https://api.com\"}").unwrap();

        // Create tab
        let tab = create_profile_tab_with(&mut conn, "claude", "Tab A", &["/path/a".to_string()]).unwrap();
        set_active_profile_with(&conn, kind, "SharedProfile", Some(&tab.id)).unwrap();

        // Verify active
        assert_eq!(get_active_profile_with(&conn, kind, Some(&tab.id)).unwrap(), "SharedProfile");

        // Delete tab
        delete_profile_tab_with(&mut conn, &tab.id).unwrap();

        // Profile should still be in library!
        let profiles = list_profiles_with(&conn, kind).unwrap();
        assert_eq!(profiles, vec!["SharedProfile"]);
        assert!(read_profile_with(&conn, kind, "SharedProfile").is_ok());

        // Tab and active profile for tab are deleted
        assert!(list_profile_tabs_with(&conn, "claude").unwrap().is_empty());
        assert_eq!(get_active_profile_with(&conn, kind, Some(&tab.id)).unwrap(), "");
    }

    #[test]
    fn test_export_and_import_tabs() {
        let mut conn = setup_test_db();
        let kind = CliKind::Claude;

        save_profile_with(&mut conn, kind, "Profile1", "{\"v\":1}").unwrap();
        set_active_profile_with(&conn, kind, "Profile1", None).unwrap();

        let tab = create_profile_tab_with(&mut conn, "claude", "Tab 1", &["/dir/1".to_string()]).unwrap();

        let exported_profiles = export_all_profiles_with(&conn).unwrap();
        assert_eq!(exported_profiles.len(), 1);
        assert_eq!(exported_profiles[0].name, "Profile1");
        assert!(exported_profiles[0].is_active);

        let exported_tabs = export_all_profile_tabs_with(&conn).unwrap();
        assert_eq!(exported_tabs.len(), 1);
        assert_eq!(exported_tabs[0].id, tab.id);
        assert_eq!(exported_tabs[0].dirs, vec!["/dir/1"]);
    }

    #[test]
    fn test_export_scope_bindings_and_is_active_is_global_only() {
        let mut conn = setup_test_db();
        let kind = CliKind::Claude;

        save_profile_with(&mut conn, kind, "ScopedOnly", "{}").unwrap();
        save_profile_with(&mut conn, kind, "GlobalActive", "{}").unwrap();
        let tab = create_profile_tab_with(&mut conn, "claude", "Tab A", &["/dir/a".to_string()]).unwrap();
        set_active_profile_with(&conn, kind, "ScopedOnly", Some(&tab.id)).unwrap();
        set_active_profile_with(&conn, kind, "GlobalActive", None).unwrap();

        // is_active 只应反映全局激活；仅在作用域激活的配置不应误标
        let exported = export_all_profiles_with(&conn).unwrap();
        let scoped = exported.iter().find(|p| p.name == "ScopedOnly").unwrap();
        let global = exported.iter().find(|p| p.name == "GlobalActive").unwrap();
        assert!(!scoped.is_active, "仅作用域激活不应标记 is_active");
        assert!(global.is_active);

        // 作用域绑定应可导出
        let bindings = export_scope_bindings_with(&conn).unwrap();
        assert_eq!(bindings, vec![("claude".to_string(), tab.id.clone(), "ScopedOnly".to_string())]);
    }

    #[test]
    fn test_restore_scope_bindings_maps_renamed_and_skips_missing() {
        let mut conn = setup_test_db();
        let kind = CliKind::Claude;

        // 导入后的本地状态：kimi 被重命名为 kimi2 导入
        save_profile_with(&mut conn, kind, "kimi2", "{}").unwrap();
        let tab = create_profile_tab_with(&mut conn, "claude", "Tab A", &["/dir/a".to_string()]).unwrap();

        // 备份里的绑定：kimi（导入时被重命名为 kimi2）、glm（被跳过未导入）
        let imported_names: std::collections::HashMap<(String, String), String> = [
            (("claude".to_string(), "kimi".to_string()), "kimi2".to_string()),
        ]
        .into_iter()
        .collect();
        let bindings = vec![
            ("claude".to_string(), tab.id.clone(), "kimi".to_string()),
            ("claude".to_string(), tab.id.clone(), "glm".to_string()),
        ];
        restore_scope_bindings_with(&conn, &bindings, &imported_names).unwrap();

        assert_eq!(get_active_profile_with(&conn, kind, Some(&tab.id)).unwrap(), "kimi2");
    }

    #[test]
    fn test_restore_scope_bindings_skips_binding_for_missing_tab() {
        let mut conn = setup_test_db();
        let kind = CliKind::Claude;

        save_profile_with(&mut conn, kind, "kimi", "{}").unwrap();
        let imported_names: std::collections::HashMap<(String, String), String> = [
            (("claude".to_string(), "kimi".to_string()), "kimi".to_string()),
        ]
        .into_iter()
        .collect();
        // 绑定指向本地不存在的作用域 tab（用户跳过了该作用域的导入）
        let bindings = vec![("claude".to_string(), "no-such-tab".to_string(), "kimi".to_string())];
        restore_scope_bindings_with(&conn, &bindings, &imported_names).unwrap();

        // 不应产生悬空绑定
        assert_eq!(get_active_profile_with(&conn, kind, Some("no-such-tab")).unwrap(), "");
    }
}
