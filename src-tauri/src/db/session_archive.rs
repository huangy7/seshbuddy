use super::archive_sidecar::{
    read_archive_sidecar, remove_archive_sidecar, sidecar_path_for_gz, write_archive_sidecar,
    ArchiveSidecar,
};
use super::settings::{read_setting_json, write_setting_json};
use super::{conn, now_rfc3339};
use crate::cli::CliKind;
use crate::title_resolver::{extract_snapshot_metadata_enhanced, TitleResolverOptions};
use crate::error::{AppError, AppResult};
use chrono::{DateTime, Utc};
use rusqlite::{params, OptionalExtension};
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;
use sha2::{Digest, Sha256};

#[derive(Debug, serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveRebuildProgress {
    pub step: String,
    pub current: usize,
    pub total: usize,
}

pub(crate) fn archives_dir() -> AppResult<PathBuf> {
    super::archives_dir()
}

pub(crate) fn archive_file_path(cli_id: &str, session_path: &str) -> AppResult<PathBuf> {
    let mut hasher = Sha256::new();
    hasher.update(session_path.as_bytes());
    let hash = format!("{:x}", hasher.finalize());
    Ok(archives_dir()?
        .join(cli_id)
        .join(format!("{}.jsonl.gz", hash)))
}

const APP_SETTING_ARCHIVE_RETENTION_DAYS: &str = "archive.retention_days";
const DEFAULT_ARCHIVE_RETENTION_DAYS: i64 = 60;
const ARCHIVE_SNAPSHOT_THRESHOLD_DAYS: i64 = 28;
const ARCHIVE_RETENTION_POLICY_DEFAULT: &str = "default";
const ARCHIVE_RETENTION_POLICY_FOREVER: &str = "forever";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionArchiveStatus {
    pub pinned: bool,
    pub source_exists: bool,
    pub expired_if_unpinned: bool,
    pub archived_at: Option<String>,
    pub retention_days: i64,
}

pub(crate) fn get_archive_retention_days() -> AppResult<i64> {
    read_setting_json::<i64>(APP_SETTING_ARCHIVE_RETENTION_DAYS)
        .map(|v| v.unwrap_or(DEFAULT_ARCHIVE_RETENTION_DAYS))
}

pub(crate) fn set_archive_retention_days(days: i64) -> AppResult<()> {
    write_setting_json(APP_SETTING_ARCHIVE_RETENTION_DAYS, &days)
}

fn file_modified_ms(path: &Path) -> i64 {
    fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
        .unwrap_or_default()
}

fn gzip_file_content(path: &Path) -> AppResult<Vec<u8>> {
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use std::io::Write;

    let content = fs::read(path)?;
    let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
    encoder.write_all(&content)?;
    encoder.finish().map_err(AppError::from)
}

pub(crate) fn snapshot_aging_sessions() -> AppResult<usize> {
    let threshold = Utc::now() - chrono::Duration::days(ARCHIVE_SNAPSHOT_THRESHOLD_DAYS);
    let threshold_str = threshold.to_rfc3339();

    let candidates: Vec<(String, String, i64, String, Option<String>)> = {
        let conn = conn()?;
        let mut stmt = conn.prepare(
            r#"SELECT sli.cli_id, sli.session_path, sli.modified_ms,
                          COALESCE(ac.retention_policy, ?2),
                          ac.pinned_at
                   FROM session_list_index sli
                   LEFT JOIN archived_session_content ac
                     ON ac.cli_id = sli.cli_id AND ac.session_path = sli.session_path
                   WHERE sli.last_timestamp IS NOT NULL
                     AND (sli.last_timestamp < ?1 OR ac.retention_policy = 'forever')
                     AND (ac.snapshot_modified_ms IS NULL OR ac.snapshot_modified_ms < sli.modified_ms)"#,
        )?;
        let rows: Vec<(String, String, i64, String, Option<String>)> = stmt.query_map(params![&threshold_str, ARCHIVE_RETENTION_POLICY_DEFAULT], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?))
        })?
        .filter_map(|r| r.ok())
        .collect();
        rows
    };

    if candidates.is_empty() {
        return Ok(0);
    }

    let now = now_rfc3339();
    let mut prepared: Vec<(String, String, Vec<u8>, i64, String, Option<String>)> = Vec::new();
    for (cli_id, session_path, modified_ms, retention_policy, pinned_at) in &candidates {
        let path = Path::new(session_path);
        if !path.exists() {
            continue;
        }
        let compressed = match gzip_file_content(path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        prepared.push((
            cli_id.clone(),
            session_path.clone(),
            compressed,
            *modified_ms,
            retention_policy.clone(),
            pinned_at.clone(),
        ));
    }

    if prepared.is_empty() {
        return Ok(0);
    }

    let mut conn = conn()?;
    let tx = conn.transaction()?;

    for (cli_id, session_path, compressed, modified_ms, retention_policy, pinned_at) in &prepared {
        // Write to file system instead of DB
        let dest = archive_file_path(cli_id, session_path)?;
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).map_err(AppError::from)?;
        }
        fs::write(&dest, compressed).map_err(AppError::from)?;

        // sidecar 灾备副本（best-effort：失败由启动维护的回填自愈）
        let sidecar = ArchiveSidecar {
            cli_id: cli_id.clone(),
            session_path: session_path.clone(),
            retention_policy: retention_policy.clone(),
            pinned_at: pinned_at.clone(),
            archived_at: now.clone(),
            snapshot_modified_ms: *modified_ms,
        };
        if let Err(e) = write_archive_sidecar(&dest, &sidecar) {
            tracing::warn!("写归档 sidecar 失败: {}", e.diagnostic());
        }

        tx.execute(
            r#"INSERT OR REPLACE INTO archived_session_content
               (cli_id, session_path, snapshot_modified_ms, archived_at, retention_policy, pinned_at)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6)"#,
            params![cli_id, session_path, modified_ms, &now, retention_policy, pinned_at],
        )?;

        tx.execute(
            "UPDATE session_list_index SET archived_at = ?1 WHERE cli_id = ?2 AND session_path = ?3",
            params![&now, cli_id, session_path],
        )?;
    }

    tx.commit()?;
    Ok(prepared.len())
}

pub(crate) fn purge_expired_archives() -> AppResult<usize> {
    let retention_days = get_archive_retention_days()?;
    if retention_days < 0 {
        return Ok(0);
    }
    let expired: Vec<(String, String)> = {
        let conn = conn()?;
        if retention_days == 0 {
            let mut stmt = conn.prepare(
                r#"SELECT cli_id, session_path FROM archived_session_content
                       WHERE retention_policy != ?1"#,
            )?;
            let rows: Vec<(String, String)> = stmt.query_map(params![ARCHIVE_RETENTION_POLICY_FOREVER], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })?
            .filter_map(|r| r.ok())
            .collect();
            rows
        } else {
            let cutoff = Utc::now() - chrono::Duration::days(retention_days);
            let cutoff_str = cutoff.to_rfc3339();
            let mut stmt = conn.prepare(
                r#"SELECT cli_id, session_path FROM archived_session_content
                       WHERE archived_at < ?1 AND retention_policy != ?2"#,
            )?;
            let rows: Vec<(String, String)> = stmt.query_map(params![&cutoff_str, ARCHIVE_RETENTION_POLICY_FOREVER], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })?
            .filter_map(|r| r.ok())
            .collect();
            rows
        }
    };

    if expired.is_empty() {
        return Ok(0);
    }

    let mut conn = conn()?;
    let tx = conn.transaction()?;

    for (cli_id, session_path) in &expired {
        let file_exists = Path::new(session_path).exists();
        tx.execute(
            "DELETE FROM archived_session_content WHERE cli_id = ?1 AND session_path = ?2",
            params![cli_id, session_path],
        )?;

        if file_exists {
            tx.execute(
                "UPDATE session_list_index SET archived_at = NULL WHERE cli_id = ?1 AND session_path = ?2",
                params![cli_id, session_path],
            )?;
        } else {
            tx.execute(
                "DELETE FROM session_list_index WHERE cli_id = ?1 AND session_path = ?2",
                params![cli_id, session_path],
            )?;
            tx.execute(
                "DELETE FROM session_search_index_state WHERE cli_id = ?1 AND session_path = ?2",
                params![cli_id, session_path],
            )?;
        }

        // Also delete physical file
        if let Ok(path) = archive_file_path(cli_id, session_path) {
            let _ = fs::remove_file(&path);
            remove_archive_sidecar(&path);
        }
    }

    tx.commit()?;
    Ok(expired.len())
}

/// 以 DB 归档行为准刷新 sidecar（pin 状态变更后调用）。
/// best-effort：sidecar 只是灾备副本，失败由启动维护的回填自愈。
fn sync_sidecar_from_db_row(conn: &rusqlite::Connection, cli_id: &str, session_path: &str) {
    let row = conn
        .query_row(
            "SELECT retention_policy, pinned_at, archived_at, snapshot_modified_ms
             FROM archived_session_content WHERE cli_id = ?1 AND session_path = ?2",
            params![cli_id, session_path],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            },
        )
        .optional();
    let Ok(Some((policy, pinned_at, archived_at, snapshot_ms))) = row else {
        return;
    };
    let Ok(gz) = archive_file_path(cli_id, session_path) else {
        return;
    };
    let sidecar = ArchiveSidecar {
        cli_id: cli_id.to_string(),
        session_path: session_path.to_string(),
        retention_policy: policy,
        pinned_at,
        archived_at,
        snapshot_modified_ms: snapshot_ms,
    };
    if let Err(e) = write_archive_sidecar(&gz, &sidecar) {
        tracing::warn!("刷新归档 sidecar 失败: {}", e.diagnostic());
    }
}

pub(crate) fn is_session_archive_pinned(kind: CliKind, session_path: &str) -> AppResult<bool> {    let conn = conn()?;
    let policy: Option<String> = conn
        .query_row(
            "SELECT retention_policy FROM archived_session_content WHERE cli_id = ?1 AND session_path = ?2",
            params![kind.id(), session_path],
            |row| row.get(0),
        )
        .optional()?;
    Ok(policy.as_deref() == Some(ARCHIVE_RETENTION_POLICY_FOREVER))
}

pub(crate) fn get_session_archive_status(
    kind: CliKind,
    session_path: &str,
) -> AppResult<SessionArchiveStatus> {
    let retention_days = get_archive_retention_days()?;
    let source_exists = Path::new(session_path).exists();
    let conn = conn()?;
    let archive_row: Option<(String, String)> = conn
        .query_row(
            "SELECT retention_policy, archived_at FROM archived_session_content WHERE cli_id = ?1 AND session_path = ?2",
            params![kind.id(), session_path],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;

    let pinned = archive_row
        .as_ref()
        .map(|(policy, _)| policy == ARCHIVE_RETENTION_POLICY_FOREVER)
        .unwrap_or(false);
    let archived_at = archive_row.map(|(_, archived_at)| archived_at);
    let expired_if_unpinned = archived_at
        .as_deref()
        .map(|value| {
            if retention_days <= 0 {
                return true;
            }
            DateTime::parse_from_rfc3339(value)
                .map(|dt| dt.with_timezone(&Utc) < Utc::now() - chrono::Duration::days(retention_days))
                .unwrap_or(false)
        })
        .unwrap_or(false);

    Ok(SessionArchiveStatus {
        pinned,
        source_exists,
        expired_if_unpinned,
        archived_at,
        retention_days,
    })
}

pub(crate) fn set_session_archive_pinned(
    kind: CliKind,
    session_path: &str,
    pinned: bool,
) -> AppResult<bool> {
    let now = now_rfc3339();
    if pinned {
        let path = Path::new(session_path);
        if path.exists() {
            let modified_ms = file_modified_ms(path);
            let compressed = gzip_file_content(path)?;

            // Write to file system
            let dest = archive_file_path(kind.id(), session_path)?;
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent).map_err(AppError::from)?;
            }
            fs::write(&dest, compressed).map_err(AppError::from)?;

            let sidecar = ArchiveSidecar {
                cli_id: kind.id().to_string(),
                session_path: session_path.to_string(),
                retention_policy: ARCHIVE_RETENTION_POLICY_FOREVER.to_string(),
                pinned_at: Some(now.clone()),
                archived_at: now.clone(),
                snapshot_modified_ms: modified_ms,
            };
            if let Err(e) = write_archive_sidecar(&dest, &sidecar) {
                tracing::warn!("写归档 sidecar 失败: {}", e.diagnostic());
            }

            let conn = conn()?;
            conn.execute(
                r#"INSERT OR REPLACE INTO archived_session_content
                   (cli_id, session_path, snapshot_modified_ms, archived_at, retention_policy, pinned_at)
                   VALUES (?1, ?2, ?3, ?4, ?5, ?6)"#,
                params![
                    kind.id(),
                    session_path,
                    modified_ms,
                    &now,
                    ARCHIVE_RETENTION_POLICY_FOREVER,
                    &now,
                ],
            )?;
            conn.execute(
                "UPDATE session_list_index SET archived_at = COALESCE(archived_at, ?1) WHERE cli_id = ?2 AND session_path = ?3",
                params![&now, kind.id(), session_path],
            )?;
            return Ok(true);
        }

        let conn = conn()?;
        let updated = conn.execute(
            r#"UPDATE archived_session_content
                   SET retention_policy = ?1, pinned_at = COALESCE(pinned_at, ?2)
                   WHERE cli_id = ?3 AND session_path = ?4"#,
            params![ARCHIVE_RETENTION_POLICY_FOREVER, &now, kind.id(), session_path],
        )?;
        if updated == 0 {
            return Err(AppError::coded("db.archive_pin_source_missing"));
        }
        conn.execute(
            "UPDATE session_list_index SET archived_at = COALESCE(archived_at, ?1) WHERE cli_id = ?2 AND session_path = ?3",
            params![&now, kind.id(), session_path],
        )?;
        sync_sidecar_from_db_row(&conn, kind.id(), session_path);
        return Ok(true);
    }

    let path_exists = Path::new(session_path).exists();
    if path_exists {
        // 源文件健在时「取消保留」= 撤销 pin：彻底移除归档（行 + .gz + sidecar），
        // 会话回到普通状态、从归档列表消失。>28 天的会话下轮维护周期会被自动重新快照。
        {
            let conn = conn()?;
            delete_session_archive_inner(&conn, kind.id(), session_path, true)?;
        }
        if let Ok(gz) = archive_file_path(kind.id(), session_path) {
            let _ = fs::remove_file(&gz);
            remove_archive_sidecar(&gz);
        }
        return Ok(false);
    }

    // 源文件已缺失：归档快照是内容唯一副本，仅降级为普通保留，不可删除
    let conn = conn()?;
    conn.execute(
        r#"UPDATE archived_session_content
           SET retention_policy = ?1, pinned_at = NULL
           WHERE cli_id = ?2 AND session_path = ?3"#,
        params![ARCHIVE_RETENTION_POLICY_DEFAULT, kind.id(), session_path],
    )?;
    sync_sidecar_from_db_row(&conn, kind.id(), session_path);
    Ok(false)
}

pub(crate) fn read_archived_session_content(
    cli_id: &str,
    session_path: &str,
) -> AppResult<Option<Vec<u8>>> {
    let dest = archive_file_path(cli_id, session_path)?;
    if dest.exists() {
        let decompressed = read_archived_file_content(&dest, cli_id)?;
        Ok(Some(decompressed))
    } else {
        Ok(None)
    }
}

/// 读取归档物理文件并按 CLI 解压（DSH 双层：外层 gzip → 内层 zstd）。
fn read_archived_file_content(path: &Path, cli_id: &str) -> AppResult<Vec<u8>> {
    let data = fs::read(path)?;
    decompress_archived_bytes(cli_id, &data)
}

/// DSH 归档第二层 zstd 解压：输入为已 gzip 解压的字节（即 DSH 源文件内容）。
/// DSH 源文件本身 zstd 压缩，归档为 `gzip(zstd(jsonl))`；内层无法按 zstd 解出
/// （明文源文件快照或截断）时原样返回 gzip 层结果。非 DSH 不做任何处理。
fn decompress_dsh_zstd_layer(cli_id: &str, bytes: Vec<u8>) -> Vec<u8> {
    use std::io::Read;

    if cli_id != "dsh" {
        return bytes;
    }
    match zstd::stream::read::Decoder::new(&bytes[..]) {
        Ok(mut decoder) => {
            let mut out = Vec::new();
            if decoder.read_to_end(&mut out).is_ok() {
                out
            } else {
                bytes
            }
        }
        Err(_) => bytes,
    }
}

/// 归档快照解压：先解外层 gzip；`cli_id=="dsh"` 时 DSH 源文件本身为 zstd 压缩，
/// 归档是 `gzip(zstd(jsonl))`，再追加内层 zstd 解压，返回完整 jsonl。
fn decompress_archived_bytes(cli_id: &str, gz_data: &[u8]) -> AppResult<Vec<u8>> {
    use flate2::read::GzDecoder;
    use std::io::Read;

    let mut gz = GzDecoder::new(gz_data);
    let mut gz_out = Vec::new();
    gz.read_to_end(&mut gz_out)?;
    Ok(decompress_dsh_zstd_layer(cli_id, gz_out))
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchivedSessionEntry {
    pub session_path: String,
    pub session_id: String,
    pub project_path: Option<String>,
    pub display_name: Option<String>,
    pub first_user_message: Option<String>,
    pub archived_at: String,
    pub pinned: bool,
    pub source_exists: bool,
}

pub(crate) fn list_archived_sessions_inner(
    conn: &rusqlite::Connection,
    cli_id: &str,
) -> AppResult<Vec<ArchivedSessionEntry>> {
    let mut stmt = conn.prepare(
        r#"
        SELECT
            ac.session_path,
            COALESCE(sli.session_id, ''),
            sli.project_path,
            sn.display_name,
            sli.title,
            sli.first_user_message,
            ac.archived_at,
            ac.retention_policy
        FROM archived_session_content ac
        LEFT JOIN session_list_index sli
          ON sli.cli_id = ac.cli_id AND sli.session_path = ac.session_path
        LEFT JOIN session_names sn
          -- 自定义名以 `<cli_id>\0<路径>` 复合键存储（见 session_index::session_name_key）。
          -- 这里必须按同一形态拼接，直接等值匹配裸路径永远命中不了，归档列表会一直拿不到用户改名。
          ON sn.session_path = ac.cli_id || char(0) || ac.session_path
        WHERE ac.cli_id = ?1
        ORDER BY ac.archived_at DESC
        "#,
    )?;

    let rows = stmt.query_map(params![cli_id], |row| {
        let policy: String = row.get(7)?;
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, Option<String>>(3)?,
            row.get::<_, Option<String>>(4)?,
            row.get::<_, Option<String>>(5)?,
            row.get::<_, String>(6)?,
            policy,
        ))
    })?;

    let codex_titles = load_codex_titles_for(cli_id);
    let mut entries = Vec::new();
    for row in rows {
        let (session_path, session_id, project_path, display_name, title, first_user_message, archived_at, policy) =
            row?;
        let source_exists = Path::new(&session_path).exists();
        // 链：用户重命名 > 原生标题（展示名）；首条用户消息原样使用（扫描/归档时已清洗），
        // 缺失时才回源文件或快照兜底，不再二次清洗
        let first_user_message = first_user_message.or_else(|| {
            if source_exists {
                crate::parser::read_first_user_message(&session_path)
            } else {
                read_archived_snapshot_metadata(cli_id, &session_path, codex_titles.as_ref()).1
            }
        });

        entries.push(ArchivedSessionEntry {
            source_exists,
            pinned: policy == ARCHIVE_RETENTION_POLICY_FOREVER,
            session_path,
            session_id,
            project_path,
            display_name: display_name.or(title),
            first_user_message,
            archived_at,
        });
    }
    Ok(entries)
}

pub(crate) fn list_archived_sessions(kind: CliKind) -> AppResult<Vec<ArchivedSessionEntry>> {
    let conn = conn()?;
    list_archived_sessions_inner(&conn, kind.id())
}

/// 删除单个会话的归档：清归档记录，源文件在则仅清 archived_at 标记，
/// 源文件不在则连同列表索引和搜索状态一并删除（会话从 UI 消失）。
/// 仅做 DB 操作；.jsonl.gz 物理文件由外层 delete_session_archive 删除。
pub(crate) fn delete_session_archive_inner(
    conn: &rusqlite::Connection,
    cli_id: &str,
    session_path: &str,
    source_exists: bool,
) -> AppResult<()> {
    conn.execute(
        "DELETE FROM archived_session_content WHERE cli_id = ?1 AND session_path = ?2",
        params![cli_id, session_path],
    )?;

    if source_exists {
        conn.execute(
            "UPDATE session_list_index SET archived_at = NULL WHERE cli_id = ?1 AND session_path = ?2",
            params![cli_id, session_path],
        )?;
    } else {
        conn.execute(
            "DELETE FROM session_list_index WHERE cli_id = ?1 AND session_path = ?2",
            params![cli_id, session_path],
        )?;
        conn.execute(
            "DELETE FROM session_search_index_state WHERE cli_id = ?1 AND session_path = ?2",
            params![cli_id, session_path],
        )?;
    }
    Ok(())
}

pub(crate) fn delete_session_archive(kind: CliKind, session_path: &str) -> AppResult<()> {
    let source_exists = Path::new(session_path).exists();
    {
        let conn = conn()?;
        delete_session_archive_inner(&conn, kind.id(), session_path, source_exists)?;
    }
    // 删除 .jsonl.gz 物理文件（修复 delete_sessions_to_trash 路径的残留问题）
    if let Ok(path) = archive_file_path(kind.id(), session_path) {
        let _ = fs::remove_file(&path);
        remove_archive_sidecar(&path);
    }
    Ok(())
}

/// 将归档快照写回原始 JSONL 路径，让 CLI 可以继续使用该会话。
/// 恢复后归档快照仍保留（作为备份），仅清除 archived_at 标记。
pub(crate) fn restore_session_to_disk(kind: CliKind, session_path: &str) -> AppResult<()> {
    // 源文件已存在则不需要恢复
    if Path::new(session_path).exists() {
        return Err(AppError::coded("db.restore_target_exists"));
    }

    // 读取归档内容（DSH 已双层解压为 jsonl）
    let content = read_archived_session_content(kind.id(), session_path)?
        .ok_or_else(|| AppError::coded("db.archive_content_missing"))?;

    // DSH 源文件本身 zstd 压缩：恢复写回时按源路径后缀回压 zstd，
    // 避免明文 jsonl 写到 .zstd/.zst 路径导致 DSH 解析器（按后缀走 zstd 解码）读不了。
    let restore_bytes = if kind.id() == "dsh" && dsh_source_path_is_zstd(session_path) {
        zstd::encode_all(std::io::Cursor::new(&content[..]), 3).map_err(AppError::from)?
    } else {
        content
    };

    // 确保父目录存在
    let path = Path::new(session_path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(AppError::from)?;
    }

    // 写回原始路径
    fs::write(path, &restore_bytes).map_err(AppError::from)?;

    // 清除 archived_at 标记（is_archived 会变为 false，因为源文件已恢复）
    let conn = conn()?;
    conn.execute(
        "UPDATE session_list_index SET archived_at = NULL WHERE cli_id = ?1 AND session_path = ?2",
        params![kind.id(), session_path],
    )?;

    Ok(())
}

/// DSH 源路径后缀判定：`.zstd` / `.zst` 视为 zstd 压缩源文件（与 dsh 解析器同判）。
fn dsh_source_path_is_zstd(session_path: &str) -> bool {
    let name = Path::new(session_path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    name.ends_with(".zstd") || name.ends_with(".zst")
}

/// 归档快照的物理文件随会话路径一起搬迁。
///
/// 快照按 `<sha256(session_path)>.jsonl.gz` 命名，且读取归档内容时**优先走物理文件**
/// （app.db 只存保留策略等元数据），因此会话路径一变、文件名哈希就对不上，归档内容
/// 会被判为缺失。搬迁需要同时处理三件事：搬 .gz、把 sidecar 改名、并改写 sidecar 内部
/// 记录的 session_path —— 后者是 DB 丢失后重建索引时唯一的路径来源，不改会让重建把
/// 快照挂回一个已经不存在的旧路径。
///
/// 全程 best-effort：文件系统失败只告警，不回滚已经提交的数据库搬迁。宁可留下一个
/// 可由启动期回填补齐的 sidecar，也不能因为搬文件失败而让会话记录卡在旧路径上。
pub(crate) fn move_archive_artifacts(cli_id: &str, old_path: &str, new_path: &str) {
    if old_path == new_path {
        return;
    }
    let (Ok(old_gz), Ok(new_gz)) = (
        archive_file_path(cli_id, old_path),
        archive_file_path(cli_id, new_path),
    ) else {
        tracing::warn!("归档路径搬迁失败：无法解析归档文件路径 cli={}", cli_id);
        return;
    };
    move_archive_files(&old_gz, &new_gz, new_path);
}

/// 搬迁动作本体：只依赖传入的两个归档路径，与归档目录的解析方式解耦（便于用临时目录验证）。
fn move_archive_files(old_gz: &Path, new_gz: &Path, new_path: &str) {
    if !old_gz.exists() {
        return;
    }
    if new_gz.exists() {
        // 新路径已有快照（更可能是较新的一次归档）：保留新快照，只清掉旧哈希文件，
        // 避免同一会话在新旧两个哈希名下各留一份内容。
        if let Err(err) = fs::remove_file(old_gz) {
            tracing::warn!("清理旧路径归档快照失败: path={:?}, err={}", old_gz, err);
        }
        remove_archive_sidecar(old_gz);
        return;
    }
    if let Some(parent) = new_gz.parent() {
        if let Err(err) = fs::create_dir_all(parent) {
            tracing::warn!("创建归档目录失败: path={:?}, err={}", parent, err);
            return;
        }
    }
    if let Err(err) = fs::rename(old_gz, new_gz) {
        tracing::warn!("归档快照文件搬迁失败: {:?} -> {:?}, err={}", old_gz, new_gz, err);
        return;
    }

    let old_sidecar = sidecar_path_for_gz(old_gz);
    match read_archive_sidecar(old_gz) {
        Some(mut meta) => {
            meta.session_path = new_path.to_string();
            match write_archive_sidecar(new_gz, &meta) {
                Ok(()) => {
                    let _ = fs::remove_file(&old_sidecar);
                }
                Err(err) => tracing::warn!("重写归档 sidecar 失败: {:?}, err={}", new_gz, err.diagnostic()),
            }
        }
        None => {
            // sidecar 缺失或损坏：直接改名保留，内容里的路径留给启动期回填按新 gz 修正
            if old_sidecar.exists() {
                let _ = fs::rename(&old_sidecar, sidecar_path_for_gz(new_gz));
            }
        }
    }
}

/// 从归档快照内容中提取展示元数据：(原生标题, 清洗后首条用户消息, cwd)。
/// 兼容三种 CLI 的 JSONL 格式（Claude / Codex / Gemini），使用 title_resolver 增强解析。
fn extract_snapshot_metadata(
    content: &[u8],
    cli_id: &str,
    session_path: &str,
    options: &TitleResolverOptions,
) -> (Option<String>, Option<String>, Option<String>) {
    extract_snapshot_metadata_enhanced(content, cli_id, session_path, options)
}

/// 读取归档快照并提取 (原生标题, 清洗后首条用户消息, cwd)；快照缺失或解析失败时返回 (None, None, None)。
fn read_archived_snapshot_metadata(
    cli_id: &str,
    session_path: &str,
    codex_titles: Option<&std::collections::HashMap<String, String>>,
) -> (Option<String>, Option<String>, Option<String>) {
    let options = TitleResolverOptions {
        codex_index_titles: codex_titles,
    };
    match read_archived_session_content(cli_id, session_path) {
        Ok(Some(content)) => extract_snapshot_metadata(&content, cli_id, session_path, &options),
        _ => (None, None, None),
    }
}

/// Codex 场景加载一次 session_index.jsonl 内存 Map（单次读取，供整批归档解析复用）
fn load_codex_titles_for(cli_id: &str) -> Option<std::collections::HashMap<String, String>> {
    if cli_id != "codex" {
        return None;
    }
    let map = crate::parser::load_codex_index_titles();
    if map.is_empty() {
        None
    } else {
        Some(map)
    }
}

fn load_history_project_map(kind: CliKind) -> Option<std::collections::HashMap<String, String>> {
    let path = crate::cli::history_path(kind).ok()?;
    let (_, project_map) = crate::history::parse_history(path.to_str().unwrap_or(""));
    if project_map.is_empty() {
        None
    } else {
        Some(project_map)
    }
}

/// 为有归档记录但缺 session_list_index 行的会话补建索引行，并修复此前补建的
/// 退化行（缺 title/first_user_message、project_path 仅为编码目录名）。
/// 标题与项目路径优先从 .gz 快照内容解析（原生标题 / 首条用户消息 / cwd），
/// 其次查 history 项目映射，最后兜底为父目录名。
/// snapshot_meta: (session_path) -> (原生标题, 清洗后首条用户消息, cwd)，便于测试注入。
pub(crate) fn restore_missing_archived_index_rows_inner(
    conn: &rusqlite::Connection,
    cli_id: &str,
    scope_root: Option<&Path>,
    tombstones: &super::TombstoneFilter,
    project_map: Option<&std::collections::HashMap<String, String>>,
    snapshot_meta: &dyn Fn(&str) -> (Option<String>, Option<String>, Option<String>),
) -> AppResult<usize> {
    let now = now_rfc3339();
    let mut restored = 0;

    // 阶段一：完全没有索引行的归档会话，补建行
    let kind_for_cli_id = CliKind::from_id(Some(cli_id)).ok();
    let mut stmt = conn.prepare(
        r#"
        SELECT ac.session_path, ac.archived_at, ac.snapshot_modified_ms
        FROM archived_session_content ac
        LEFT JOIN session_list_index sli
          ON sli.cli_id = ac.cli_id AND sli.session_path = ac.session_path
        WHERE ac.cli_id = ?1 AND sli.session_path IS NULL
        "#,
    )?;
    let missing: Vec<(String, String, i64)> = stmt
        .query_map(params![cli_id], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?
        .filter_map(|r| r.ok())
        .collect();

    for (session_path, archived_at, snapshot_modified_ms) in &missing {
        if crate::parser::is_subagent_session(session_path) {
            continue;
        }
        // 范围外的归档会话不补建索引行。归属用路径分量判定（而非字符串前缀），
        // 避免 /a/sessions-old 被 /a/sessions 认领
        if let Some(root) = scope_root {
            if !Path::new(session_path).starts_with(root) {
                continue;
            }
        }
        // 已删会话的残存快照不得补建索引行：删除只清库记录、保留磁盘快照，
        // 少了这道拦截，每次列表加载都会把已删会话的行补回来（复活）
        if let Some(kind) = kind_for_cli_id {
            let tombstone_id =
                super::session_tombstones::tombstone_session_id(kind, session_path, None);
            if tombstones.is_tombstoned(&tombstone_id, session_path) {
                continue;
            }
        }
        // Claude/Codex 会话文件名即 session_id（去 .jsonl 后缀）
        let session_id = Path::new(session_path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        let encoded_dir = Path::new(session_path)
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .map(|s| s.to_string());
        let (title, first_user_message, cwd) = snapshot_meta(session_path);
        let project_path = cwd
            .or_else(|| {
                encoded_dir.as_deref().and_then(|dir| {
                    project_map
                        .and_then(|map| crate::session::find_project_path_in_map(dir, map))
                        .map(|s| s.to_string())
                })
            })
            .or(encoded_dir);

        let last_ts = if *snapshot_modified_ms > 0 {
            chrono::DateTime::from_timestamp_millis(*snapshot_modified_ms).map(|dt| dt.to_rfc3339())
        } else {
            Some(archived_at.clone())
        };
        let file_size = if Path::new(session_path).exists() {
            fs::metadata(session_path).map(|m| m.len()).unwrap_or(0)
        } else {
            archive_file_path(cli_id, session_path)
                .ok()
                .and_then(|p| fs::metadata(p).ok())
                .map(|m| m.len())
                .unwrap_or(0)
        };

        conn.execute(
            r#"
            INSERT OR IGNORE INTO session_list_index
                (cli_id, session_path, session_id, project_path, title, first_user_message,
                 first_timestamp, last_timestamp, git_branch, file_size, modified_ms,
                 indexed_at, archived_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7, '', ?8, ?9, ?10, ?11)
            "#,
            params![
                cli_id,
                session_path,
                session_id,
                project_path,
                title,
                first_user_message,
                last_ts,
                file_size,
                snapshot_modified_ms,
                &now,
                archived_at
            ],
        )?;
        restored += 1;
    }

    // 阶段二：修复退化或信息缺失行（title 与 first_user_message 同时为 NULL、last_timestamp 为 NULL，或 file_size 为 0）。
    // 注意：单独的 title IS NULL 或 first_user_message IS NULL 都是正常状态
    // （多数会话无原生标题；少数会话只有标题没有可取的用户文本），不能作为触发条件，
    // 否则每次列表加载都会对相应归档会话重复 gunzip+解析快照，缓存永不收敛。
    // title 只在阶段一补建行时或本阶段因其他原因解析快照时顺带 COALESCE 填充。
    let mut stmt = conn.prepare(
        r#"
        SELECT sli.session_path, sli.project_path, ac.snapshot_modified_ms, ac.archived_at, sli.file_size, sli.last_timestamp, sli.first_user_message, sli.title
        FROM session_list_index sli
        JOIN archived_session_content ac
          ON ac.cli_id = sli.cli_id AND ac.session_path = sli.session_path
        WHERE sli.cli_id = ?1 AND ((sli.first_user_message IS NULL AND sli.title IS NULL) OR sli.last_timestamp IS NULL OR sli.file_size = 0)
        "#,
    )?;
    let degraded: Vec<(String, Option<String>, i64, String, u64, Option<String>, Option<String>, Option<String>)> = stmt
        .query_map(params![cli_id], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
                row.get(7)?,
            ))
        })?
        .filter_map(|r| r.ok())
        .collect();

    for (session_path, old_project, snapshot_modified_ms, archived_at, current_size, current_last_ts, current_first_msg, current_title) in degraded {
        // 仅在真正需要快照内容时才解析（标题与首条用户消息同时缺失）；
        // last_timestamp/file_size 修复不需要 gunzip 快照
        let (extracted_title, extracted_first_msg, cwd) = if current_first_msg.is_none() && current_title.is_none() {
            snapshot_meta(&session_path)
        } else {
            (None, None, None)
        };
        let first_msg = current_first_msg.clone().or(extracted_first_msg);
        let title = current_title.clone().or(extracted_title);

        let parent_name = Path::new(&session_path)
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str());
        let new_project = match (&cwd, old_project.as_deref(), parent_name) {
            (Some(c), old, parent) if old.is_none() || old == parent => Some(c.clone()),
            _ => None,
        };

        let last_ts = current_last_ts.clone().or_else(|| {
            if snapshot_modified_ms > 0 {
                chrono::DateTime::from_timestamp_millis(snapshot_modified_ms).map(|dt| dt.to_rfc3339())
            } else {
                Some(archived_at.clone())
            }
        });

        let file_size = if current_size > 0 {
            current_size
        } else if Path::new(&session_path).exists() {
            fs::metadata(&session_path).map(|m| m.len()).unwrap_or(0)
        } else {
            archive_file_path(cli_id, &session_path)
                .ok()
                .and_then(|p| fs::metadata(p).ok())
                .map(|m| m.len())
                .unwrap_or(0)
        };

        let first_msg_changed = first_msg.is_some() && first_msg != current_first_msg;
        let title_changed = title.is_some() && title != current_title;
        let project_changed = new_project.is_some();
        let timestamp_changed = current_last_ts.is_none() && last_ts.is_some();
        let size_changed = current_size == 0 && file_size > 0;

        if !first_msg_changed && !title_changed && !project_changed && !timestamp_changed && !size_changed {
            continue;
        }

        conn.execute(
            r#"
            UPDATE session_list_index
               SET title = COALESCE(?3, title),
                   first_user_message = COALESCE(?4, first_user_message),
                   project_path = COALESCE(?5, project_path),
                   last_timestamp = COALESCE(last_timestamp, ?6),
                   first_timestamp = COALESCE(first_timestamp, ?6),
                   file_size = CASE WHEN file_size = 0 THEN ?7 ELSE file_size END
             WHERE cli_id = ?1 AND session_path = ?2
            "#,
            params![cli_id, session_path, title, first_msg, new_project, last_ts, file_size],
        )?;
        restored += 1;
    }

    Ok(restored)
}

/// 会话路径是否落在该 CLI 当前数据源的范围内。
///
/// 会话列表只呈现当前数据源范围内的会话；范围外的记录不呈现（归档本体与用户数据保留），
/// 数据源切回来时靠重新扫描恢复。归属用**路径分量**判定（`Path::starts_with`）而不是
/// 字符串前缀：`/a/sessions-old` 不能被 `/a/sessions` 认领，否则会整批误判。
///
/// CLI 无法识别或目录解析失败时按「在范围内」处理 —— 宁可多呈现，也不静默吞掉会话。
fn path_in_current_data_source(cli_id: &str, session_path: &str) -> bool {
    let Ok(kind) = CliKind::from_id(Some(cli_id)) else {
        return true;
    };
    match crate::cli::sessions_dir(kind) {
        Ok(root) => Path::new(session_path).starts_with(root),
        Err(_) => true,
    }
}

pub(crate) fn restore_missing_archived_index_rows(kind: CliKind) -> AppResult<usize> {
    let conn = conn()?;
    let project_map = load_history_project_map(kind);
    let cli_id = kind.id().to_string();
    let codex_titles = load_codex_titles_for(&cli_id);
    // 只救回落在当前数据源范围内的归档会话：数据源切换后旧源范围内的归档同样不该回到
    // 列表里，否则"改数据源"这件事会被这一步当场撤销（归档行是列表加载时无条件补建的）
    let scope_root = crate::cli::sessions_dir(kind).ok();
    let tombstones = super::load_tombstone_filter(kind).unwrap_or_default();
    restore_missing_archived_index_rows_inner(
        &conn,
        &cli_id,
        scope_root.as_deref(),
        &tombstones,
        project_map.as_ref(),
        &|path| read_archived_snapshot_metadata(&cli_id, path, codex_titles.as_ref()),
    )
}

fn parse_archived_jsonl_info(content: &[u8]) -> (Option<String>, Option<String>, Option<String>) {
    let text = match std::str::from_utf8(content) {
        Ok(t) => t,
        Err(_) => return (None, None, None),
    };

    let mut session_id = None;
    let mut session_path = None;
    let mut cwd = None;

    for line in text.lines().take(60) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(trimmed) {
            if session_id.is_none() {
                if let Some(id) = val
                    .get("session_id")
                    .or_else(|| val.get("sessionId"))
                    .or_else(|| val.get("id"))
                    .and_then(|v| v.as_str())
                {
                    if !id.is_empty() {
                        session_id = Some(id.to_string());
                    }
                }
            }
            if session_path.is_none() {
                if let Some(p) = val
                    .get("file_path")
                    .or_else(|| val.get("session_path"))
                    .or_else(|| val.get("filePath"))
                    .and_then(|v| v.as_str())
                {
                    if !p.is_empty() {
                        session_path = Some(p.to_string());
                    }
                }
            }
            if cwd.is_none() {
                if let Some(c) = val
                    .get("cwd")
                    .or_else(|| val.get("project_path"))
                    .or_else(|| val.get("projectPath"))
                    .and_then(|v| v.as_str())
                {
                    if !c.is_empty() {
                        cwd = Some(c.to_string());
                    }
                }
            }
        }
    }

    (session_id, session_path, cwd)
}

pub(crate) fn escape_claude_project_dir(cwd: &str) -> String {
    cwd.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RebuildArchiveResult {
    pub restored: usize,
    pub skipped: usize,
}

/// 重建时的归档元数据决策：有 sidecar 用 sidecar（pin 状态/原始归档时间灾备副本），
/// 否则 policy 落 default，archived_at 用 .gz 文件 mtime（快照真实创建时间），
/// 比「重建时刻」更接近真相，也避免 purge 倒计时被整体重置。
struct RebuildMeta {
    retention_policy: String,
    pinned_at: Option<String>,
    archived_at: String,
    snapshot_modified_ms: i64,
}

fn rebuild_meta_from(
    sidecar: Option<&ArchiveSidecar>,
    gz_modified_ms: i64,
    now: &str,
) -> RebuildMeta {
    if let Some(s) = sidecar {
        return RebuildMeta {
            retention_policy: s.retention_policy.clone(),
            pinned_at: s.pinned_at.clone(),
            archived_at: s.archived_at.clone(),
            snapshot_modified_ms: s.snapshot_modified_ms,
        };
    }
    let archived_at = if gz_modified_ms > 0 {
        chrono::DateTime::from_timestamp_millis(gz_modified_ms)
            .map(|dt| dt.to_rfc3339())
            .unwrap_or_else(|| now.to_string())
    } else {
        now.to_string()
    };
    RebuildMeta {
        retention_policy: ARCHIVE_RETENTION_POLICY_DEFAULT.to_string(),
        pinned_at: None,
        archived_at,
        snapshot_modified_ms: gz_modified_ms,
    }
}

/// sli 的 archived_at 标记：源文件缺失必须标（会话靠快照存活）；源文件健在时
/// 只有永久保留的会话才标（与 pin 时的语义一致），普通快照的活跃会话不误标。
fn sli_archived_at_for(source_exists: bool, retention_policy: &str, archived_at: &str) -> Option<String> {
    if source_exists && retention_policy != ARCHIVE_RETENTION_POLICY_FOREVER {
        None
    } else {
        Some(archived_at.to_string())
    }
}

/// 流式 gunzip 头部内容（用于身份识别，避免对注定跳过的文件全量解压）。
/// 截断/损坏容忍：能解出多少算多少，完全解不出返回 None。
/// DSH 归档为 `gzip(zstd(jsonl))`，会话头在 zstd 内层：对 dsh 再尽力 zstd 解出
/// （`take(limit)` 截断帧时保留已解出的头部），zstd 帧不存在时回退 gzip 层结果。
fn gunzip_head(path: &Path, cli_id: &str, limit: u64) -> Option<Vec<u8>> {
    use flate2::read::GzDecoder;
    use std::io::Read;

    let data = fs::read(path).ok()?;
    let mut buf = Vec::new();
    let _ = GzDecoder::new(&data[..]).take(limit).read_to_end(&mut buf);
    if buf.is_empty() {
        return None;
    }
    if cli_id == "dsh" {
        if let Ok(mut decoder) = zstd::stream::read::Decoder::new(&buf[..]) {
            let mut out = Vec::new();
            let _ = decoder.read_to_end(&mut out);
            if !out.is_empty() {
                return Some(out);
            }
        }
    }
    Some(buf)
}

pub(crate) fn rebuild_archive_index_from_disk(
    app: &tauri::AppHandle,
    cli_id_param: Option<&str>,
) -> AppResult<RebuildArchiveResult> {
    use flate2::read::GzDecoder;
    use sha2::{Digest, Sha256};
    use std::io::Read;
    use tauri::Emitter;

    let archives_root = archives_dir()?;
    if !archives_root.exists() {
        return Ok(RebuildArchiveResult { restored: 0, skipped: 0 });
    }

    // Collect all known project directories for Claude CLI to construct candidate paths
    let claude_projects_root = dirs::home_dir().map(|h| h.join(".claude").join("projects"));
    let mut project_dirs: Vec<String> = Vec::new();
    if let Some(ref root) = claude_projects_root {
        if root.is_dir() {
            if let Ok(entries) = fs::read_dir(root) {
                for entry in entries.flatten() {
                    if entry.path().is_dir() {
                        if let Some(name) = entry.file_name().to_str() {
                            project_dirs.push(name.to_string());
                        }
                    }
                }
            }
        }
    }

    // 先收集全部待处理 .gz 文件（同时得到进度总数）
    let mut targets: Vec<(String, std::path::PathBuf)> = Vec::new();
    let cli_dirs = match fs::read_dir(&archives_root) {
        Ok(d) => d,
        Err(_) => return Ok(RebuildArchiveResult { restored: 0, skipped: 0 }),
    };
    for cli_entry in cli_dirs.flatten() {
        let cli_path = cli_entry.path();
        if !cli_path.is_dir() {
            continue;
        }
        let current_cli_id = match cli_path.file_name().and_then(|s| s.to_str()) {
            Some(name) => name.to_string(),
            None => continue,
        };
        if let Some(target_cli) = cli_id_param {
            if !target_cli.is_empty() && target_cli != current_cli_id {
                continue;
            }
        }
        if let Ok(files) = fs::read_dir(&cli_path) {
            for file_entry in files.flatten() {
                let path = file_entry.path();
                if path.extension().and_then(|s| s.to_str()) == Some("gz") {
                    targets.push((current_cli_id.clone(), path));
                }
            }
        }
    }

    let total = targets.len();
    let compute_hash = |s: &str| -> String {
        let mut hasher = Sha256::new();
        hasher.update(s.as_bytes());
        format!("{:x}", hasher.finalize())
    };

    let mut conn = conn()?;
    let tx = conn.transaction()?;
    let mut restored = 0usize;
    let mut skipped = 0usize;
    let now = now_rfc3339();
    // 各 CLI 的墓碑集合在循环外取一次：下面逐条判定，拦掉用户已明确删除的会话
    let rebuild_tombstones: std::collections::HashMap<
        String,
        super::TombstoneFilter,
    > = ["claude", "codex", "gemini", "workbuddy", "dsh", "antigravity"]
        .into_iter()
        .filter_map(|cli_id| {
            let kind = CliKind::from_id(Some(cli_id)).ok()?;
            Some((cli_id.to_string(), super::load_tombstone_filter(kind).unwrap_or_default()))
        })
        .collect();

    for (index, (current_cli_id, path)) in targets.iter().enumerate() {
        let _ = app.emit(
            "archive-rebuild-progress",
            ArchiveRebuildProgress {
                step: "scan".to_string(),
                current: index + 1,
                total,
            },
        );

        // Extract the expected hash from filename (e.g. "abc123.jsonl.gz" -> "abc123")
        let expected_hash = match path.file_name().and_then(|s| s.to_str()) {
            Some(name) => name
                .trim_end_matches(".jsonl.gz")
                .trim_end_matches(".gz")
                .to_string(),
            None => {
                skipped += 1;
                continue;
            }
        };

        // sidecar 优先：灾备副本直接给出 session_path/policy/pinned_at/archived_at，
        // 无需靠内容反解（codex/gemini 的嵌套格式反解不出来）
        let sidecar = super::archive_sidecar::read_archive_sidecar(path).filter(|meta| {
            meta.cli_id == *current_cli_id && compute_hash(&meta.session_path) == expected_hash
        });

        // 头部解压用于身份识别（无 sidecar 时）；注定跳过的文件不做全量解压
        let head = if sidecar.is_some() {
            None
        } else {
            match gunzip_head(path, &current_cli_id, 256 * 1024) {
                Some(h) => Some(h),
                None => {
                    skipped += 1;
                    continue;
                }
            }
        };
        let (parsed_id, parsed_path, parsed_cwd) = head
            .as_deref()
            .map(parse_archived_jsonl_info)
            .unwrap_or((None, None, None));

        // Try to find the correct session_path by SHA256 verification
        let session_path: Option<String> = {
            let mut found = sidecar.as_ref().map(|meta| meta.session_path.clone());

            // 1. If parsed_path is available, check it directly
            if found.is_none() {
                if let Some(ref p) = parsed_path {
                    if !p.is_empty() && compute_hash(p) == expected_hash {
                        found = Some(p.clone());
                    }
                }
            }

            // 2. Try constructing Claude-style path directly from escaped cwd + sessionId
            if found.is_none() {
                if let (Some(ref sid), Some(ref dir)) = (&parsed_id, &parsed_cwd) {
                    let escaped_cwd = escape_claude_project_dir(dir);
                    if let Some(ref root) = claude_projects_root {
                        let candidate = root
                            .join(&escaped_cwd)
                            .join(format!("{}.jsonl", sid))
                            .to_string_lossy()
                            .to_string();
                        if compute_hash(&candidate) == expected_hash {
                            found = Some(candidate);
                        }
                    }
                }
            }

            // 3. Try project_dirs matching in ~/.claude/projects/
            if found.is_none() {
                if let Some(ref sid) = parsed_id {
                    if let Some(ref root) = claude_projects_root {
                        for proj_dir in &project_dirs {
                            let candidate = root
                                .join(proj_dir)
                                .join(format!("{}.jsonl", sid))
                                .to_string_lossy()
                                .to_string();
                            if compute_hash(&candidate) == expected_hash {
                                found = Some(candidate);
                                break;
                            }
                        }
                    }
                }
            }

            // 4. Try direct cwd-based path if we have cwd and sessionId
            if found.is_none() {
                if let (Some(ref sid), Some(ref dir)) = (&parsed_id, &parsed_cwd) {
                    let candidate = Path::new(dir)
                        .join(format!("{}.jsonl", sid))
                        .to_string_lossy()
                        .to_string();
                    if compute_hash(&candidate) == expected_hash {
                        found = Some(candidate);
                    }
                }
            }

            found
        };

        // If we couldn't verify the session_path via SHA256, skip this file
        let session_path = match session_path {
            Some(p) => p,
            None => {
                tracing::warn!(
                    "rebuild_archive: skipping {} — could not verify session_path (cli={}, sessionId={:?}, cwd={:?})",
                    path.display(),
                    current_cli_id,
                    parsed_id,
                    parsed_cwd
                );
                skipped += 1;
                continue;
            }
        };

        // 已确认身份，全量解压提取展示元数据（标题/首条用户消息/cwd）
        let decompressed = {
            let file_data = match fs::read(path) {
                Ok(d) => d,
                Err(_) => {
                    skipped += 1;
                    continue;
                }
            };
            let mut buf = Vec::new();
            if GzDecoder::new(&file_data[..]).read_to_end(&mut buf).is_err() {
                skipped += 1;
                continue;
            }
            decompress_dsh_zstd_layer(&current_cli_id, buf)
        };

        let codex_titles = load_codex_titles_for(current_cli_id);
        let options = TitleResolverOptions {
            codex_index_titles: codex_titles.as_ref(),
        };
        let (title, first_msg, meta_cwd) =
            extract_snapshot_metadata(&decompressed, current_cli_id, "", &options);
        let cwd = meta_cwd.or(parsed_cwd);

        let session_id = match &parsed_id {
            Some(id) if !id.is_empty() => id.clone(),
            _ => Path::new(&session_path)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .to_string(),
        };

        let gz_modified_ms = file_modified_ms(path);
        let meta = rebuild_meta_from(sidecar.as_ref(), gz_modified_ms, &now);
        let snapshot_ms = if meta.snapshot_modified_ms > 0 {
            meta.snapshot_modified_ms
        } else {
            gz_modified_ms
        };
        let last_ts = if snapshot_ms > 0 {
            chrono::DateTime::from_timestamp_millis(snapshot_ms).map(|dt| dt.to_rfc3339())
        } else {
            Some(now.clone())
        };
        let file_size = decompressed.len() as u64;

        // jsonl_content 由快照压缩包承载，行内只留空占位；schema 建表时该列恒定存在
        tx.execute(
            r#"INSERT INTO archived_session_content
               (cli_id, session_path, jsonl_content, snapshot_modified_ms, archived_at, retention_policy, pinned_at)
               VALUES (?1, ?2, x'', ?3, ?4, ?5, ?6)
               ON CONFLICT(cli_id, session_path) DO UPDATE SET
                 snapshot_modified_ms = CASE WHEN snapshot_modified_ms = 0 THEN ?3 ELSE snapshot_modified_ms END"#,
            params![
                current_cli_id,
                session_path,
                snapshot_ms,
                &meta.archived_at,
                &meta.retention_policy,
                &meta.pinned_at
            ],
        )?;

        // 范围外的归档会话不进列表索引：归档本体的元数据已在上方写入（那是快照本体的
        // 组成部分，数据源切回来时还要靠它），但会话列表只呈现当前数据源范围内的会话。
        // 少了这道判断，"重建归档索引"会把已切换掉的数据源里的会话整批拉回列表
        if !path_in_current_data_source(current_cli_id, &session_path) {
            skipped += 1;
            continue;
        }
        // 已删会话的残存快照同样不得重建索引行 —— 否则"重建归档索引"会把用户
        // 明确删除过的会话整批复活，正是双轨墓碑要防的场景
        if let Ok(current_kind) = CliKind::from_id(Some(current_cli_id)) {
            let tombstone_id =
                super::session_tombstones::tombstone_session_id(current_kind, &session_path, None);
            let is_tombstoned = rebuild_tombstones
                .get(current_cli_id.as_str())
                .map(|filter| filter.is_tombstoned(&tombstone_id, &session_path))
                .unwrap_or(false);
            if is_tombstoned {
                skipped += 1;
                continue;
            }
        }

        let source_exists = Path::new(&session_path).exists();
        let db_archived_at = sli_archived_at_for(source_exists, &meta.retention_policy, &meta.archived_at);

        tx.execute(
            r#"INSERT INTO session_list_index
               (cli_id, session_path, session_id, title, first_user_message, project_path,
                last_timestamp, first_timestamp, git_branch, file_size, modified_ms, indexed_at, archived_at)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7, '', ?8, ?9, ?10, ?11)
               ON CONFLICT(cli_id, session_path) DO UPDATE SET
                 archived_at = CASE WHEN ?11 IS NOT NULL THEN COALESCE(session_list_index.archived_at, ?11) ELSE session_list_index.archived_at END,
                 title = COALESCE(session_list_index.title, excluded.title),
                 first_user_message = COALESCE(session_list_index.first_user_message, excluded.first_user_message),
                 project_path = COALESCE(session_list_index.project_path, excluded.project_path)"#,
            params![
                current_cli_id,
                session_path,
                session_id,
                title,
                first_msg,
                cwd,
                last_ts,
                file_size,
                snapshot_ms,
                &now,
                db_archived_at
            ],
        )?;

        restored += 1;
    }

    tx.commit()?;
    Ok(RebuildArchiveResult { restored, skipped })
}

/// 为存量归档行补写 sidecar 灾备副本（幂等：只写缺失的）。
/// DB 健在时把 pin/归档时间落盘，DB 丢失后 rebuild 才能恢复这些状态。
pub(crate) fn backfill_archive_sidecars() -> AppResult<usize> {
    let rows: Vec<(String, String, String, Option<String>, String, i64)> = {
        let conn = conn()?;
        let mut stmt = conn.prepare(
            "SELECT cli_id, session_path, retention_policy, pinned_at, archived_at, snapshot_modified_ms
             FROM archived_session_content",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            })?
            .filter_map(|r| r.ok())
            .collect();
        rows
    };

    let mut written = 0;
    for (cli_id, session_path, policy, pinned_at, archived_at, snapshot_ms) in rows {
        let Ok(gz) = archive_file_path(&cli_id, &session_path) else {
            continue;
        };
        if !gz.exists() || sidecar_path_for_gz(&gz).exists() {
            continue;
        }
        let sidecar = ArchiveSidecar {
            cli_id,
            session_path,
            retention_policy: policy,
            pinned_at,
            archived_at,
            snapshot_modified_ms: snapshot_ms,
        };
        match write_archive_sidecar(&gz, &sidecar) {
            Ok(()) => written += 1,
            Err(e) => tracing::warn!("回填归档 sidecar 失败: {}", e.diagnostic()),
        }
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    /// 空墓碑集合：多数归档测试不关心墓碑，用它与真实调用点保持同形
    fn no_tombstones() -> super::super::TombstoneFilter {
        Default::default()
    }

    fn create_archive_test_schema(conn: &Connection) {
        conn.execute_batch(
            r#"
            CREATE TABLE archived_session_content (
                cli_id TEXT NOT NULL,
                session_path TEXT NOT NULL,
                -- 生产建表时该列无默认值（快照本体写 x''）；测试夹具多数不关心此列，故给默认值
                jsonl_content BLOB NOT NULL DEFAULT x'',
                snapshot_modified_ms INTEGER NOT NULL DEFAULT 0,
                archived_at TEXT NOT NULL,
                retention_policy TEXT NOT NULL DEFAULT 'default',
                pinned_at TEXT,
                PRIMARY KEY (cli_id, session_path)
            );
            CREATE TABLE session_list_index (
                cli_id TEXT NOT NULL,
                session_path TEXT NOT NULL,
                session_id TEXT NOT NULL,
                project_path TEXT,
                title TEXT,
                first_user_message TEXT,
                first_timestamp TEXT,
                last_timestamp TEXT,
                git_branch TEXT NOT NULL DEFAULT '',
                file_size INTEGER NOT NULL DEFAULT 0,
                modified_ms INTEGER NOT NULL DEFAULT 0,
                indexed_at TEXT NOT NULL DEFAULT '',
                archived_at TEXT,
                PRIMARY KEY (cli_id, session_path)
            );
            CREATE TABLE session_names (
                session_path TEXT PRIMARY KEY,
                display_name TEXT NOT NULL
            );
            "#,
        )
        .unwrap();
    }

    #[test]
    fn rebuild_meta_prefers_sidecar_values() {
        let sidecar = ArchiveSidecar {
            cli_id: "claude".to_string(),
            session_path: "/p/s.jsonl".to_string(),
            retention_policy: "forever".to_string(),
            pinned_at: Some("2026-07-16T10:01:46Z".to_string()),
            archived_at: "2026-07-16T10:01:46Z".to_string(),
            snapshot_modified_ms: 1752696106847,
        };
        let meta = rebuild_meta_from(Some(&sidecar), 999, "2026-07-28T00:00:00Z");
        assert_eq!(meta.retention_policy, "forever");
        assert_eq!(meta.pinned_at.as_deref(), Some("2026-07-16T10:01:46Z"));
        assert_eq!(meta.archived_at, "2026-07-16T10:01:46Z");
        assert_eq!(meta.snapshot_modified_ms, 1752696106847);
    }

    #[test]
    fn rebuild_meta_without_sidecar_falls_back_to_gz_mtime() {
        // 无 sidecar：policy 落 default，archived_at 用 .gz mtime 而非重建时刻
        let meta = rebuild_meta_from(None, 1784196106847, "2026-07-28T00:00:00Z");
        assert_eq!(meta.retention_policy, "default");
        assert_eq!(meta.pinned_at, None);
        assert_eq!(meta.archived_at, "2026-07-16T10:01:46.847+00:00");
        assert_eq!(meta.snapshot_modified_ms, 1784196106847);

        // gz mtime 无效时兜底为重建时刻
        let meta = rebuild_meta_from(None, 0, "2026-07-28T00:00:00Z");
        assert_eq!(meta.archived_at, "2026-07-28T00:00:00Z");
    }

    #[test]
    fn sli_archived_at_marks_missing_source_and_forever_only() {
        let ts = "2026-07-16T10:01:46Z";
        // 源文件缺失：必须标记（会话靠快照存活）
        assert_eq!(
            sli_archived_at_for(false, "default", ts).as_deref(),
            Some(ts)
        );
        // 源文件健在 + 永久保留：标记（与 pin 时语义一致）
        assert_eq!(
            sli_archived_at_for(true, "forever", ts).as_deref(),
            Some(ts)
        );
        // 源文件健在 + 普通快照：不误标活跃会话
        assert_eq!(sli_archived_at_for(true, "default", ts), None);
    }

    #[test]
    fn list_archived_sessions_returns_entries_sorted_desc() {
        let conn = Connection::open_in_memory().unwrap();
        create_archive_test_schema(&conn);

        conn.execute_batch(
            r#"
            INSERT INTO session_list_index (cli_id, session_path, session_id, project_path, first_user_message) VALUES
                ('claude', '/a.jsonl', 'sid-a', '/proj/a', '你好'),
                ('claude', '/b.jsonl', 'sid-b', '/proj/b', '第二条');
            -- 必须用生产真实写入的复合键形态，否则 JOIN 命中不了，测试会掩盖归档列表拿不到自定义名的缺陷
            INSERT INTO session_names (session_path, display_name) VALUES ('claude' || char(0) || '/b.jsonl', '自定义名');
            INSERT INTO archived_session_content
                (cli_id, session_path, archived_at, retention_policy, pinned_at) VALUES
                ('claude', '/a.jsonl', '2026-07-01T10:00:00Z', 'default', NULL),
                ('claude', '/b.jsonl', '2026-07-20T10:00:00Z', 'forever', '2026-07-20T10:00:00Z'),
                ('codex',  '/c.jsonl', '2026-07-21T10:00:00Z', 'default', NULL);
            "#,
        )
        .unwrap();

        let entries = list_archived_sessions_inner(&conn, "claude").unwrap();
        assert_eq!(entries.len(), 2);

        // 按 archived_at 倒序：/b.jsonl 在前
        assert_eq!(entries[0].session_path, "/b.jsonl");
        assert!(entries[0].pinned);
        assert_eq!(entries[0].display_name.as_deref(), Some("自定义名"));
        assert_eq!(entries[0].session_id, "sid-b");

        assert_eq!(entries[1].session_path, "/a.jsonl");
        assert!(!entries[1].pinned);
        assert_eq!(entries[1].display_name, None);
        assert_eq!(entries[1].first_user_message.as_deref(), Some("你好"));

        // codex 的不混入
        let codex_entries = list_archived_sessions_inner(&conn, "codex").unwrap();
        assert_eq!(codex_entries.len(), 1);
        // sli 中无 /c.jsonl 行时，session_id 为空字符串（LEFT JOIN 兜底）
        assert_eq!(codex_entries[0].session_id, "");
    }

    #[test]
    fn delete_session_archive_source_exists_clears_flag_keeps_session() {
        let conn = Connection::open_in_memory().unwrap();
        create_archive_test_schema(&conn);
        conn.execute_batch(
            r#"
            INSERT INTO session_list_index (cli_id, session_path, session_id, archived_at) VALUES
                ('claude', '/a.jsonl', 'sid-a', '2026-07-01T10:00:00Z');
            INSERT INTO archived_session_content (cli_id, session_path, archived_at) VALUES
                ('claude', '/a.jsonl', '2026-07-01T10:00:00Z');
            "#,
        )
        .unwrap();

        delete_session_archive_inner(&conn, "claude", "/a.jsonl", true).unwrap();

        // 归档记录删除
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM archived_session_content", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
        // 会话保留，archived_at 清空
        let archived_at: Option<String> = conn
            .query_row(
                "SELECT archived_at FROM session_list_index WHERE session_path = '/a.jsonl'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(archived_at, None);
    }

    #[test]
    fn delete_session_archive_source_missing_removes_session_row() {
        let conn = Connection::open_in_memory().unwrap();
        create_archive_test_schema(&conn);
        conn.execute_batch(
            r#"
            CREATE TABLE session_search_index_state (
                cli_id TEXT NOT NULL,
                session_path TEXT NOT NULL,
                modified_ms INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (cli_id, session_path)
            );
            INSERT INTO session_list_index (cli_id, session_path, session_id, archived_at) VALUES
                ('claude', '/gone.jsonl', 'sid-g', '2026-07-01T10:00:00Z');
            INSERT INTO archived_session_content (cli_id, session_path, archived_at) VALUES
                ('claude', '/gone.jsonl', '2026-07-01T10:00:00Z');
            INSERT INTO session_search_index_state (cli_id, session_path) VALUES
                ('claude', '/gone.jsonl');
            "#,
        )
        .unwrap();

        delete_session_archive_inner(&conn, "claude", "/gone.jsonl", false).unwrap();

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM archived_session_content", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM session_list_index", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM session_search_index_state", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    /// 已删会话的残存快照不得补建索引行：删除只清库记录、保留磁盘快照，
    /// 少了这道拦截，每次列表加载都会把用户明确删掉的会话补回来（复活）。
    #[test]
    fn restore_missing_archived_index_rows_skips_tombstoned_sessions() {
        let conn = Connection::open_in_memory().unwrap();
        create_archive_test_schema(&conn);
        conn.execute_batch(
            r#"
            INSERT INTO archived_session_content
                (cli_id, session_path, snapshot_modified_ms, archived_at) VALUES
                ('claude', '/proj-a/live.jsonl', 111, '2026-07-01T10:00:00Z'),
                ('claude', '/proj-b/deleted.jsonl', 222, '2026-07-10T10:00:00Z');
            "#,
        )
        .unwrap();

        let mut tombstones = super::super::TombstoneFilter::default();
        // 删除时记录了该会话的物理路径，扫描侧按路径即可拦下
        tombstones.paths.insert("/proj-b/deleted.jsonl".to_string());

        let no_meta = &|_: &str| (None, None, None);
        let restored = restore_missing_archived_index_rows_inner(
            &conn,
            "claude",
            None,
            &tombstones,
            None,
            no_meta,
        )
        .unwrap();

        assert_eq!(restored, 1, "只应恢复未被删除的归档会话");
        let restored_path: String = conn
            .query_row("SELECT session_path FROM session_list_index", [], |r| r.get(0))
            .unwrap();
        assert_eq!(restored_path, "/proj-a/live.jsonl");
    }

    #[test]
    fn restore_missing_archived_index_rows_skips_archives_outside_scope() {
        let conn = Connection::open_in_memory().unwrap();
        create_archive_test_schema(&conn);
        conn.execute_batch(
            r#"
            INSERT INTO archived_session_content
                (cli_id, session_path, snapshot_modified_ms, archived_at) VALUES
                ('claude', '/current-source/proj-a/sid-a.jsonl', 111, '2026-07-01T10:00:00Z'),
                ('claude', '/old-source/proj-b/sid-b.jsonl', 222, '2026-07-10T10:00:00Z'),
                ('claude', '/current-source-old/proj-c/sid-c.jsonl', 333, '2026-07-11T10:00:00Z');
            "#,
        )
        .unwrap();

        let no_meta = &|_: &str| (None, None, None);
        let restored = restore_missing_archived_index_rows_inner(
            &conn,
            "claude",
            Some(Path::new("/current-source")),
            &no_tombstones(),
            None,
            no_meta,
        )
        .unwrap();

        // 数据源切换后，旧源的归档不该被这一步当场救回列表里；
        // `/current-source-old` 与 `/current-source` 只是字符串前缀相同，路径分量上并不在内
        assert_eq!(restored, 1, "只应恢复当前数据源范围内的归档会话");
        let restored_path: String = conn
            .query_row("SELECT session_path FROM session_list_index", [], |r| r.get(0))
            .unwrap();
        assert_eq!(restored_path, "/current-source/proj-a/sid-a.jsonl");
    }

    /// 数据源范围判定：按路径分量而非字符串前缀；未知 CLI 不做裁剪。
    #[test]
    fn path_in_current_data_source_uses_path_components() {
        let scope = crate::cli::sessions_dir(CliKind::Dsh).expect("dsh 会话目录");
        let inside = scope.join("--proj--").join("s1").join("session.jsonl.zstd");
        let sibling = std::path::PathBuf::from(format!("{}-old", scope.to_string_lossy()))
            .join("--proj--")
            .join("s1")
            .join("session.jsonl.zstd");

        assert!(path_in_current_data_source("dsh", inside.to_str().unwrap()));
        assert!(
            !path_in_current_data_source("dsh", sibling.to_str().unwrap()),
            "字符串前缀相同但路径分量不同，不属于同一数据源"
        );
        assert!(!path_in_current_data_source("dsh", "/tmp/other/session.jsonl.zstd"));
        // 未知 CLI 不做范围裁剪：宁可多呈现，也不静默吞掉会话
        assert!(path_in_current_data_source("nope", "/tmp/other/session.jsonl.zstd"));
    }

    #[test]
    fn restore_missing_archived_index_rows_backfills_and_is_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        create_archive_test_schema(&conn);
        conn.execute_batch(
            r#"
            INSERT INTO session_list_index
                (cli_id, session_path, session_id, project_path, first_user_message, last_timestamp, file_size, modified_ms, archived_at) VALUES
                ('claude', '/proj-a/sid-a.jsonl', 'sid-a', '/proj-a', '已有行', '2026-07-01T10:00:00Z', 100, 111, '2026-07-01T10:00:00Z');
            INSERT INTO archived_session_content
                (cli_id, session_path, snapshot_modified_ms, archived_at) VALUES
                ('claude', '/proj-a/sid-a.jsonl', 111, '2026-07-01T10:00:00Z'),
                ('claude', '/proj-b/sid-b.jsonl', 222, '2026-07-10T10:00:00Z');
            "#,
        )
        .unwrap();

        let no_meta = &|_: &str| (None, None, None);
        let restored =
            restore_missing_archived_index_rows_inner(&conn, "claude", None, &no_tombstones(), None, no_meta).unwrap();
        assert_eq!(restored, 1);

        // 缺行的归档会话被补建：session_id 取文件名去后缀，archived_at 对齐归档记录
        let (session_id, project_path, modified_ms, archived_at): (String, Option<String>, i64, Option<String>) = conn
            .query_row(
                "SELECT session_id, project_path, modified_ms, archived_at FROM session_list_index WHERE cli_id = 'claude' AND session_path = '/proj-b/sid-b.jsonl'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        assert_eq!(session_id, "sid-b");
        assert_eq!(project_path.as_deref(), Some("proj-b"));
        assert_eq!(modified_ms, 222);
        assert_eq!(archived_at.as_deref(), Some("2026-07-10T10:00:00Z"));

        // 已有 sli 行的记录不被改动（INSERT OR IGNORE）
        let first_user_message: Option<String> = conn
            .query_row(
                "SELECT first_user_message FROM session_list_index WHERE session_path = '/proj-a/sid-a.jsonl'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(first_user_message.as_deref(), Some("已有行"));

        // 幂等：再次调用返回 0（补建行无快照元数据可修，不重复计数）
        let restored =
            restore_missing_archived_index_rows_inner(&conn, "claude", None, &no_tombstones(), None, no_meta).unwrap();
        assert_eq!(restored, 0);
    }

    #[test]
    fn restore_enriches_title_and_project_from_snapshot() {
        let conn = Connection::open_in_memory().unwrap();
        create_archive_test_schema(&conn);
        conn.execute_batch(
            r#"
            INSERT INTO archived_session_content
                (cli_id, session_path, snapshot_modified_ms, archived_at) VALUES
                ('claude', '/Users/h/.claude/projects/-Users-h-codes-SeshBuddy/sid-x.jsonl', 333, '2026-07-10T10:00:00Z');
            "#,
        )
        .unwrap();

        let meta = &|_: &str| {
            (
                None,
                Some("你是什么模型?".to_string()),
                Some("/Users/h/codes/SeshBuddy".to_string()),
            )
        };
        let restored =
            restore_missing_archived_index_rows_inner(&conn, "claude", None, &no_tombstones(), None, meta).unwrap();
        assert_eq!(restored, 1);

        let (title, project_path): (Option<String>, Option<String>) = conn
            .query_row(
                "SELECT first_user_message, project_path FROM session_list_index WHERE session_path = '/Users/h/.claude/projects/-Users-h-codes-SeshBuddy/sid-x.jsonl'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(title.as_deref(), Some("你是什么模型?"));
        // cwd 优先于编码目录名
        assert_eq!(project_path.as_deref(), Some("/Users/h/codes/SeshBuddy"));
    }

    #[test]
    fn restore_uses_history_project_map_when_no_snapshot_cwd() {
        let conn = Connection::open_in_memory().unwrap();
        create_archive_test_schema(&conn);
        conn.execute_batch(
            r#"
            INSERT INTO archived_session_content
                (cli_id, session_path, snapshot_modified_ms, archived_at) VALUES
                ('claude', '/Users/h/.claude/projects/-Users-h-codes-SeshBuddy/sid-y.jsonl', 333, '2026-07-10T10:00:00Z');
            "#,
        )
        .unwrap();

        let mut project_map = std::collections::HashMap::new();
        project_map.insert(
            "-Users-h-codes-SeshBuddy".to_string(),
            "/Users/h/codes/SeshBuddy".to_string(),
        );
        // 快照只有标题、没有 cwd
        let meta = &|_: &str| (None, Some("只有标题".to_string()), None);
        let restored = restore_missing_archived_index_rows_inner(
            &conn,
            "claude",
            None,
            &no_tombstones(),
            Some(&project_map),
            meta,
        )
        .unwrap();
        assert_eq!(restored, 1);

        let project_path: Option<String> = conn
            .query_row(
                "SELECT project_path FROM session_list_index WHERE session_path = '/Users/h/.claude/projects/-Users-h-codes-SeshBuddy/sid-y.jsonl'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(project_path.as_deref(), Some("/Users/h/codes/SeshBuddy"));
    }

    #[test]
    fn restore_heals_degraded_row_from_snapshot() {
        let conn = Connection::open_in_memory().unwrap();
        create_archive_test_schema(&conn);
        // 模拟退化行：无标题，project_path 是编码目录名
        conn.execute_batch(
            r#"
            INSERT INTO session_list_index
                (cli_id, session_path, session_id, project_path, first_user_message, archived_at) VALUES
                ('claude', '/Users/h/.claude/projects/-Users-h-codes-projX/sid-z.jsonl', 'sid-z', '-Users-h-codes-projX', NULL, '2026-07-10T10:00:00Z');
            INSERT INTO archived_session_content
                (cli_id, session_path, snapshot_modified_ms, archived_at) VALUES
                ('claude', '/Users/h/.claude/projects/-Users-h-codes-projX/sid-z.jsonl', 333, '2026-07-10T10:00:00Z');
            "#,
        )
        .unwrap();

        let meta = &|_: &str| {
            (
                None,
                Some("帮我修个 bug".to_string()),
                Some("/Users/h/codes/projX".to_string()),
            )
        };
        let restored =
            restore_missing_archived_index_rows_inner(&conn, "claude", None, &no_tombstones(), None, meta).unwrap();
        assert_eq!(restored, 1);

        let (title, project_path): (Option<String>, Option<String>) = conn
            .query_row(
                "SELECT first_user_message, project_path FROM session_list_index WHERE session_path = '/Users/h/.claude/projects/-Users-h-codes-projX/sid-z.jsonl'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(title.as_deref(), Some("帮我修个 bug"));
        // 编码目录名被真实 cwd 覆盖
        assert_eq!(project_path.as_deref(), Some("/Users/h/codes/projX"));

        // 幂等：标题已补齐，再次调用返回 0
        let restored =
            restore_missing_archived_index_rows_inner(&conn, "claude", None, &no_tombstones(), None, meta).unwrap();
        assert_eq!(restored, 0);
    }

    #[test]
    fn restore_does_not_reparse_rows_with_null_title_only() {
        // 回归：title 为 NULL 是正常状态（多数会话无原生标题），不得触发快照解析，
        // 否则每次列表加载都会 gunzip 全部归档快照（缓存永不收敛）
        let conn = Connection::open_in_memory().unwrap();
        create_archive_test_schema(&conn);
        conn.execute_batch(
            r#"
            INSERT INTO session_list_index
                (cli_id, session_path, session_id, project_path, title, first_user_message, last_timestamp, file_size, modified_ms, archived_at) VALUES
                ('claude', '/proj-a/sid-a.jsonl', 'sid-a', '/proj-a', NULL, '已有首条消息', '2026-07-01T10:00:00Z', 100, 111, '2026-07-01T10:00:00Z'),
                ('claude', '/proj-b/sid-b.jsonl', 'sid-b', '/proj-b', '原生标题', NULL, '2026-07-01T10:00:00Z', 100, 111, '2026-07-01T10:00:00Z');
            INSERT INTO archived_session_content
                (cli_id, session_path, snapshot_modified_ms, archived_at) VALUES
                ('claude', '/proj-a/sid-a.jsonl', 111, '2026-07-01T10:00:00Z'),
                ('claude', '/proj-b/sid-b.jsonl', 111, '2026-07-01T10:00:00Z');
            "#,
        )
        .unwrap();

        let call_count = std::cell::Cell::new(0);
        let meta = &|_: &str| {
            call_count.set(call_count.get() + 1);
            (None, None, None)
        };
        let restored =
            restore_missing_archived_index_rows_inner(&conn, "claude", None, &no_tombstones(), None, meta).unwrap();

        assert_eq!(restored, 0);
        assert_eq!(
            call_count.get(),
            0,
            "title NULL 或 first_user_message NULL 单独存在都不应触发快照解析"
        );
    }

    #[test]
    fn extract_snapshot_metadata_claude_format() {
        let content = concat!(
            "{\"type\":\"summary\",\"summary\":\"test\"}\n",
            "{\"type\":\"user\",\"cwd\":\"/Users/h/codes/SeshBuddy\",\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"text\",\"text\":\"你是什么模型?\"}]}}\n",
            "{\"type\":\"assistant\",\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"我是...\"}]}}\n",
        );
        let options = TitleResolverOptions {
            codex_index_titles: None,
        };
        let (title, user, cwd) = extract_snapshot_metadata(
            content.as_bytes(),
            "claude",
            "/Users/h/codes/SeshBuddy/s.jsonl",
            &options,
        );
        assert_eq!(title, None);
        assert_eq!(user.as_deref(), Some("你是什么模型?"));
        assert_eq!(cwd.as_deref(), Some("/Users/h/codes/SeshBuddy"));
    }

    #[test]
    fn extract_snapshot_metadata_codex_format() {
        let content = concat!(
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"abc\",\"cwd\":\"/private/tmp/projY\"}}\n",
            "{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"看一下日志\"}]}}\n",
        );
        let options = TitleResolverOptions {
            codex_index_titles: None,
        };
        let (title, user, cwd) = extract_snapshot_metadata(
            content.as_bytes(),
            "codex",
            "/private/tmp/projY/s.jsonl",
            &options,
        );
        assert_eq!(title, None);
        assert_eq!(user.as_deref(), Some("看一下日志"));
        assert_eq!(cwd.as_deref(), Some("/private/tmp/projY"));
    }

    #[test]
    fn extract_snapshot_metadata_skips_tool_results_and_truncates() {
        let long_text = "这是一个非常非常长的用户消息用来验证标题会被正确截断到三十个字符以内哦";
        let content = format!(
            "{{\"type\":\"user\",\"message\":{{\"role\":\"user\",\"content\":[{{\"type\":\"tool_result\",\"content\":\"x\"}}]}}}}\n{{\"type\":\"user\",\"cwd\":\"/p\",\"message\":{{\"role\":\"user\",\"content\":\"{}\"}}}}\n",
            long_text
        );
        let options = TitleResolverOptions {
            codex_index_titles: None,
        };
        let (title, user, cwd) = extract_snapshot_metadata(
            content.as_bytes(),
            "claude",
            "/p/s.jsonl",
            &options,
        );
        assert_eq!(title, None);
        let user = user.expect("应有兜底用户消息");
        assert_eq!(user, long_text);
        assert_eq!(cwd.as_deref(), Some("/p"));
    }

    fn gzip_bytes(data: &[u8]) -> Vec<u8> {
        use flate2::write::GzEncoder;
        use flate2::Compression;
        use std::io::Write;
        let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
        encoder.write_all(data).unwrap();
        encoder.finish().unwrap()
    }

    fn dsh_two_message_jsonl() -> &'static str {
        concat!(
            "{\"type\":\"session\",\"version\":0,\"id\":\"sess-1\",\"createdAt\":1700000000000,\"cwd\":\"/proj/a\"}\n",
            "{\"type\":\"user/message\",\"seq\":1,\"time\":1700000000000,\"data\":{\"content\":[{\"type\":\"text\",\"text\":\"hello dsh\"}]}}\n",
            "{\"type\":\"assistant/message\",\"seq\":2,\"time\":1700000000001,\"data\":{\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"hi there\"}]}}}\n",
        )
    }

    #[test]
    fn read_archived_dsh_double_layer_decompresses_and_parses() {
        // DSH 源文件本身 zstd 压缩，归档快照为 gzip(zstd(jsonl))：读档须双层解压。
        let dsh_jsonl = dsh_two_message_jsonl();
        let zstd_bytes = zstd::encode_all(std::io::Cursor::new(dsh_jsonl.as_bytes()), 3).unwrap();
        let gz_bytes = gzip_bytes(&zstd_bytes);

        // 走归档读档解压路径（外层 gzip + DSH 内层 zstd）
        let decompressed = decompress_archived_bytes("dsh", &gz_bytes).unwrap();
        let content = String::from_utf8(decompressed).expect("双层解压后应为合法 utf8 jsonl");
        let messages = crate::parser::parse_session_content_by_kind(
            "/Users/x/.dsh/sessions/--p--/sess-1/session.jsonl.zstd",
            &content,
        );
        assert_eq!(messages.len(), 2, "DSH 归档双层解压后应解析出 2 条消息");
        assert_eq!(messages[0].role, "user");
        assert_eq!(messages[1].role, "assistant");
    }

    #[test]
    fn read_archived_dsh_double_layer_via_archive_file_read() {
        // 完整归档物理文件读档：写入 gzip(zstd(jsonl)) 到临时 .gz 文件，走 read_archived_file_content
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sess-1.jsonl.gz");
        let zstd_bytes = zstd::encode_all(
            std::io::Cursor::new(dsh_two_message_jsonl().as_bytes()),
            3,
        )
        .unwrap();
        std::fs::write(&path, gzip_bytes(&zstd_bytes)).unwrap();

        let content = read_archived_file_content(&path, "dsh").unwrap();
        let content = String::from_utf8(content).expect("双层解压后应为合法 utf8 jsonl");
        let messages = crate::parser::parse_session_content_by_kind(
            "/Users/x/.dsh/sessions/--p--/sess-1/session.jsonl.zstd",
            &content,
        );
        assert_eq!(messages.len(), 2, "归档物理文件读档应双层解压并解析出 2 条消息");
    }

    #[test]
    fn read_archived_claude_single_layer_unchanged() {
        // 回归：非 DSH 归档仍只解外层 gzip，行为不变
        let claude_jsonl = concat!(
            "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"text\",\"text\":\"hi\"}]}}\n",
            "{\"type\":\"assistant\",\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"hello\"}]}}\n",
        );
        let gz_bytes = gzip_bytes(claude_jsonl.as_bytes());

        let decompressed = decompress_archived_bytes("claude", &gz_bytes).unwrap();
        assert_eq!(decompressed, claude_jsonl.as_bytes(), "claude 归档应原样返回 jsonl");
        let content = String::from_utf8(decompressed).unwrap();
        let messages = crate::parser::parse_session_content_by_kind(
            "/Users/x/.claude/projects/p/a.jsonl",
            &content,
        );
        assert_eq!(messages.len(), 2, "claude 归档单层 gzip 保持原行为");
    }

    #[test]
    fn read_archived_dsh_plain_inner_falls_back_to_gzip_layer() {
        // 回归：DSH 明文源文件（非 zstd）快照 = gzip(plain jsonl)，zstd 层解不出时回退 gzip 层
        let dsh_jsonl = concat!(
            "{\"type\":\"session\",\"version\":0,\"id\":\"s1\",\"cwd\":\"/proj\"}\n",
            "{\"type\":\"user/message\",\"seq\":1,\"time\":1700000000000,\"data\":{\"content\":[{\"type\":\"text\",\"text\":\"hi\"}]}}\n",
        );
        let gz_bytes = gzip_bytes(dsh_jsonl.as_bytes());

        let decompressed = decompress_archived_bytes("dsh", &gz_bytes).unwrap();
        let content = String::from_utf8(decompressed).unwrap();
        let messages = crate::parser::parse_session_content_by_kind(
            "/Users/x/.dsh/sessions/p/s1/session.jsonl",
            &content,
        );
        assert_eq!(messages.len(), 1, "DSH 明文快照 zstd 层解不出时应回退 gzip 层");
    }

    fn sidecar_for(cli_id: &str, session_path: &str) -> ArchiveSidecar {
        ArchiveSidecar {
            cli_id: cli_id.to_string(),
            session_path: session_path.to_string(),
            retention_policy: "forever".to_string(),
            pinned_at: Some("2026-07-16T10:01:46Z".to_string()),
            archived_at: "2026-07-16T10:01:46Z".to_string(),
            snapshot_modified_ms: 1752696106847,
        }
    }

    #[test]
    fn move_archive_files_relocates_snapshot_and_rewrites_sidecar_path() {
        let dir = tempfile::tempdir().unwrap();
        let old_gz = dir.path().join("old.jsonl.gz");
        let new_gz = dir.path().join("new.jsonl.gz");
        fs::write(&old_gz, b"snapshot-bytes").unwrap();
        write_archive_sidecar(&old_gz, &sidecar_for("dsh", "/old/path/session.jsonl")).unwrap();

        move_archive_files(&old_gz, &new_gz, "/new/path/session.v3.jsonl.zstd");

        assert!(!old_gz.exists(), "旧哈希快照应被移走");
        assert_eq!(fs::read(&new_gz).unwrap(), b"snapshot-bytes");
        // 路径必须写进 sidecar：DB 丢失后靠它重建，写旧路径会把快照挂回不存在的路径
        let meta = read_archive_sidecar(&new_gz).expect("sidecar 应随快照迁移");
        assert_eq!(meta.session_path, "/new/path/session.v3.jsonl.zstd");
        assert_eq!(meta.retention_policy, "forever");
        assert_eq!(meta.pinned_at.as_deref(), Some("2026-07-16T10:01:46Z"));
        assert!(!sidecar_path_for_gz(&old_gz).exists(), "旧 sidecar 应被清理");
    }

    #[test]
    fn move_archive_files_keeps_existing_snapshot_and_drops_stale_hash() {
        let dir = tempfile::tempdir().unwrap();
        let old_gz = dir.path().join("old.jsonl.gz");
        let new_gz = dir.path().join("new.jsonl.gz");
        fs::write(&old_gz, b"stale").unwrap();
        write_archive_sidecar(&old_gz, &sidecar_for("dsh", "/old/path/session.jsonl")).unwrap();
        fs::write(&new_gz, b"fresher").unwrap();
        write_archive_sidecar(&new_gz, &sidecar_for("dsh", "/new/path/session.jsonl")).unwrap();

        move_archive_files(&old_gz, &new_gz, "/new/path/session.jsonl");

        // 目标已有快照（更可能是较新的一次归档）：保留它，清掉旧哈希，避免同一会话两份内容
        assert_eq!(fs::read(&new_gz).unwrap(), b"fresher");
        assert!(!old_gz.exists());
        assert!(!sidecar_path_for_gz(&old_gz).exists());
        let meta = read_archive_sidecar(&new_gz).expect("新快照的 sidecar 应保留");
        assert_eq!(meta.session_path, "/new/path/session.jsonl");
    }

    #[test]
    fn move_archive_files_is_noop_when_source_missing() {
        let dir = tempfile::tempdir().unwrap();
        let old_gz = dir.path().join("missing.jsonl.gz");
        let new_gz = dir.path().join("new.jsonl.gz");

        move_archive_files(&old_gz, &new_gz, "/new/path/session.jsonl");

        assert!(!new_gz.exists(), "源快照缺失时不应凭空创建目标文件");
        assert!(!sidecar_path_for_gz(&new_gz).exists());
    }
}
