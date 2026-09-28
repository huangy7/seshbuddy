//! CLI 配置文件（`settings.json` / `config.toml`）的读写。
//!
//! 代理开关与 profile 应用写的是**同一批文件**，必须共用同一套实现：写入侧若不一致，
//! 一边原子替换、一边直接截断，并发下读端会读到写了一半的文件、两次写入也会互相覆盖；
//! 读取侧则会出现只有一边处理 UTF-8 BOM 与空文件。

use crate::cli::{self, CliKind};
use crate::error::{AppError, AppResult};
use std::fs;
use std::path::{Path, PathBuf};

/// 去掉 UTF-8 BOM。部分编辑器（尤其 Windows 记事本）会写入 BOM，
/// 带 BOM 的内容交给 serde_json / toml 解析会直接报错。
pub(crate) fn strip_bom(s: String) -> String {
    s.strip_prefix('\u{feff}').map(str::to_string).unwrap_or(s)
}

/// 该 CLI 没有可管理的配置文件时统一的错误。
///
/// CLI 展示名进 `params.kind`（`CliKind::name()`），不拼进码里：四种 CLI 是**同一句文案**，
/// 拆成四个码会让同一句话在语言包里出现四遍，译文一改就要改四处。
pub(crate) fn unsupported_kind_error(kind: CliKind) -> AppError {
    AppError::coded("profile.unsupported_kind").with("kind", kind.name())
}

/// 把 IO 错误转成用户能看懂的消息：权限不足是最常见的失败原因，
/// 原样抛出 "Permission denied" 对用户没有指导意义。
///
/// 底层 OS 文本按 R3 进 `params.detail`，不进语言包：它随平台与 locale 变，
/// 翻不了也不该翻。原文里那句硬编码的 "(Permission denied)" 因此改成 `{detail}` 插值。
pub(crate) fn map_io_error(e: std::io::Error, path: &Path) -> AppError {
    if e.kind() == std::io::ErrorKind::PermissionDenied {
        AppError::coded("cli_config.permission_denied")
            .with("detail", e.to_string())
            .with("path", path.display().to_string())
    } else {
        AppError::coded("cli_config.io_failed").with("detail", e.to_string())
    }
}

pub(crate) fn settings_path_for(kind: CliKind) -> AppResult<PathBuf> {
    match kind {
        CliKind::Claude => Ok(cli::data_dir(kind)?.join("settings.json")),
        CliKind::Codex => Ok(cli::data_dir(kind)?.join("config.toml")),
        CliKind::Gemini => Err(unsupported_kind_error(kind)),
        CliKind::WorkBuddy => Err(unsupported_kind_error(kind)),
        CliKind::Dsh => Err(unsupported_kind_error(kind)),
        CliKind::Antigravity => Err(unsupported_kind_error(kind)),
    }
}

pub(crate) fn ensure_parent_dir(path: &Path) -> AppResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| map_io_error(e, parent))?;
    }
    Ok(())
}

/// 先写同目录临时文件再 `rename`。`rename` 在同一文件系统上是原子的，
/// 因此读端要么看到旧内容、要么看到新内容，绝不会读到写了一半的文件。
pub(crate) fn write_atomically(path: &Path, bytes: &[u8]) -> AppResult<()> {
    let tmp_path = path.with_extension("tmp");
    fs::write(&tmp_path, bytes).map_err(|e| map_io_error(e, &tmp_path))?;
    fs::rename(&tmp_path, path).map_err(|e| map_io_error(e, path))
}

/// 读取 JSON 配置。文件缺失或为空时回退到 `default_content`（而非报错）——
/// CLI 首次运行时配置文件尚未创建，这是正常状态不是异常。
pub(crate) fn read_json_file(path: &Path, default_content: &str) -> AppResult<serde_json::Value> {
    let fallback = || {
        serde_json::from_str(default_content).map_err(|e| {
            AppError::coded("cli_config.default_json_invalid").with("detail", e.to_string())
        })
    };
    if !path.exists() {
        return fallback();
    }
    let raw = strip_bom(fs::read_to_string(path).map_err(|e| map_io_error(e, path))?);
    if raw.trim().is_empty() {
        return fallback();
    }
    serde_json::from_str(&raw).map_err(|e| AppError::coded("cli_config.json_parse_failed").with("detail", e.to_string()))
}

pub(crate) fn write_json_file(path: &Path, value: &serde_json::Value) -> AppResult<()> {
    ensure_parent_dir(path)?;
    let json =
        serde_json::to_string_pretty(value).map_err(|e| AppError::coded("cli_config.json_serialize_failed").with("detail", e.to_string()))?;
    write_atomically(path, json.as_bytes())
}

/// 读取 TOML 配置。文件缺失或为空时返回空表（同 [`read_json_file`] 的理由）。
pub(crate) fn read_toml_file(path: &Path) -> AppResult<toml::Value> {
    let empty = || Ok(toml::Value::Table(toml::map::Map::new()));
    if !path.exists() {
        return empty();
    }
    let raw = strip_bom(fs::read_to_string(path).map_err(|e| map_io_error(e, path))?);
    if raw.trim().is_empty() {
        return empty();
    }
    raw.parse::<toml::Value>()
        .map_err(|e| AppError::coded("cli_config.toml_parse_failed").with("detail", e.to_string()))
}

pub(crate) fn write_toml_file(path: &Path, value: &toml::Value) -> AppResult<()> {
    ensure_parent_dir(path)?;
    let content = toml::to_string_pretty(value).map_err(|e| AppError::coded("cli_config.toml_serialize_failed").with("detail", e.to_string()))?;
    write_atomically(path, content.as_bytes())
}
