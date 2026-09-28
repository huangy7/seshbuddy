//! 归档快照的 sidecar 元数据：与 `<hash>.jsonl.gz` 同目录的 `<hash>.meta.json`。
//!
//! retention_policy / pinned_at / archived_at / session_path 此前只存在于 app.db，
//! DB 一旦被删（或损坏）就无法恢复——pin 状态丢失、rebuild 只能靠 SHA256 反解
//! 猜测 session_path（codex/gemini 格式解不出来，全部静默跳过）。
//! sidecar 作为灾备副本随快照一起落盘：DB 健在时永不读取，rebuild 时优先采用。

use crate::error::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ArchiveSidecar {
    pub cli_id: String,
    pub session_path: String,
    pub retention_policy: String,
    pub pinned_at: Option<String>,
    pub archived_at: String,
    pub snapshot_modified_ms: i64,
}

/// 由 .gz 路径推导 sidecar 路径：`<hash>.jsonl.gz` -> `<hash>.meta.json`
pub(crate) fn sidecar_path_for_gz(gz_path: &Path) -> PathBuf {
    let file_name = gz_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    let stem = file_name
        .trim_end_matches(".jsonl.gz")
        .trim_end_matches(".gz");
    gz_path.with_file_name(format!("{}.meta.json", stem))
}

/// 写 sidecar。先写临时文件再 rename，避免崩溃留下半写损坏文件。
pub(crate) fn write_archive_sidecar(gz_path: &Path, meta: &ArchiveSidecar) -> AppResult<()> {
    let dest = sidecar_path_for_gz(gz_path);
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).map_err(AppError::from)?;
    }
    let tmp = dest.with_extension("tmp");
    fs::write(&tmp, serde_json::to_vec(meta)?).map_err(AppError::from)?;
    fs::rename(&tmp, &dest).map_err(AppError::from)?;
    Ok(())
}

/// 读 sidecar；缺失或损坏返回 None（调用方按无 sidecar 降级处理）。
pub(crate) fn read_archive_sidecar(gz_path: &Path) -> Option<ArchiveSidecar> {
    let data = fs::read(sidecar_path_for_gz(gz_path)).ok()?;
    serde_json::from_slice(&data).ok()
}

/// 删除 sidecar；文件不存在时静默忽略（与删 .gz 的语义一致）。
pub(crate) fn remove_archive_sidecar(gz_path: &Path) {
    let _ = fs::remove_file(sidecar_path_for_gz(gz_path));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_gz_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("seshbuddy-sidecar-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    #[test]
    fn sidecar_path_strips_jsonl_gz_suffix() {
        let gz = Path::new("/data/archives/claude/abc123.jsonl.gz");
        assert_eq!(
            sidecar_path_for_gz(gz),
            PathBuf::from("/data/archives/claude/abc123.meta.json")
        );
    }

    #[test]
    fn sidecar_path_strips_bare_gz_suffix() {
        let gz = Path::new("/data/archives/claude/abc123.gz");
        assert_eq!(
            sidecar_path_for_gz(gz),
            PathBuf::from("/data/archives/claude/abc123.meta.json")
        );
    }

    #[test]
    fn write_then_read_roundtrip() {
        let gz = temp_gz_path("roundtrip.jsonl.gz");
        fs::write(&gz, b"fake-gz").unwrap();
        let meta = ArchiveSidecar {
            cli_id: "claude".to_string(),
            session_path: "/Users/h/.claude/projects/-p/sid-1.jsonl".to_string(),
            retention_policy: "forever".to_string(),
            pinned_at: Some("2026-07-16T10:01:46Z".to_string()),
            archived_at: "2026-07-16T10:01:46Z".to_string(),
            snapshot_modified_ms: 1752,
        };
        write_archive_sidecar(&gz, &meta).unwrap();

        let read = read_archive_sidecar(&gz).expect("sidecar 应可读");
        assert_eq!(read.retention_policy, "forever");
        assert_eq!(read.session_path, meta.session_path);
        assert_eq!(read.pinned_at.as_deref(), Some("2026-07-16T10:01:46Z"));
        assert_eq!(read.snapshot_modified_ms, 1752);

        remove_archive_sidecar(&gz);
        assert!(read_archive_sidecar(&gz).is_none());
        let _ = fs::remove_file(&gz);
    }

    #[test]
    fn read_missing_or_corrupt_returns_none() {
        let gz = temp_gz_path("missing.jsonl.gz");
        assert!(read_archive_sidecar(&gz).is_none());

        // 损坏内容不得 panic，按无 sidecar 降级
        let gz2 = temp_gz_path("corrupt.jsonl.gz");
        fs::write(sidecar_path_for_gz(&gz2), b"{not json").unwrap();
        assert!(read_archive_sidecar(&gz2).is_none());
        let _ = fs::remove_file(sidecar_path_for_gz(&gz2));
    }
}
