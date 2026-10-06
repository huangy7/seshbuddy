use crate::error::{AppError, AppResult};
use super::format_timestamp;
use crate::app_db;
use crate::cli::{self, CliKind};
use crate::cli_registry::{source_for, SessionLocator};
use crate::history;
use crate::parser;
use crate::session::{self, ChatMessage, ContentPart, SearchResult, UsageRecord};
use crate::streaming;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::Path;
use tauri::AppHandle;

#[tauri::command]
pub async fn load_session_incremental(
    file_path: String,
    offset: u64,
    skip_sidechain_filter: Option<bool>,
) -> AppResult<session::SessionLoadResult> {
    tauri::async_runtime::spawn_blocking(move || {
        let skip_sidechain = skip_sidechain_filter.unwrap_or(false);
        parser::parse_session_incremental(&file_path, offset, skip_sidechain)
    })
    .await
    .unwrap_or_else(|_| Err(AppError::coded("session.task_failed")))
}

/// 懒加载回取 tool_result 全文：按 `source_offset` seek 定位该行，
/// 反漂移校验（part 类型 + tool_use_id）通过才返回全文（50 KB 截断后）。
/// 校验失败返回明确错误，前端回退展示预览。
#[tauri::command]
pub async fn get_tool_result_full_content(
    file_path: String,
    source_offset: u64,
    tool_use_id: Option<String>,
) -> AppResult<String> {
    tauri::async_runtime::spawn_blocking(move || {
        crate::parser::claude::tool_result_full_content_at(
            &file_path,
            source_offset,
            tool_use_id.as_deref(),
        )
    })
    .await
    .unwrap_or_else(|_| Err(AppError::coded("session.task_failed")))
}

#[tauri::command]
pub fn resolve_session_id(cli_id: Option<String>, file_path: String) -> AppResult<Option<String>> {
    // cli_id 仅用于保持命令 ABI 兼容；subagents 歧义守卫必须与 CLI 类型无关，
    // 否则切换 CLI 后对旧 Tab 的重新解析会绕过守卫。
    let _ = &cli_id;

    let is_subagent_file = Path::new(&file_path)
        .parent()
        .and_then(|parent| parent.file_name())
        .and_then(|name| name.to_str())
        == Some("subagents")
        || parser::is_subagent_session(&file_path);
    if is_subagent_file {
        return Ok(None);
    }

    Ok(parser::read_session_id(&file_path).filter(|id| !id.trim().is_empty()))
}

const SESSION_STREAM_TOPIC: &str = "session";

#[derive(Debug, Clone, Serialize)]
struct SessionStreamDone {
    offset: u64,
    subagent_map: HashMap<String, session::SubagentInfo>,
}

/// 流式加载会话：通过 `session:<request_id>:chunk` / `:done` / `:error` 事件向前端推送。
/// 命令本身**立刻返回 Ok(())**——真正的工作在 `spawn_blocking` 里跑。
#[tauri::command]
pub async fn load_session_stream(
    app: AppHandle,
    request_id: String,
    file_path: String,
    cli_id: Option<String>,
    skip_sidechain_filter: Option<bool>,
) -> AppResult<()> {
    let skip_sidechain = skip_sidechain_filter.unwrap_or(false);

    streaming::spawn_streaming_task(app, SESSION_STREAM_TOPIC, request_id, move |app, topic, request_id| {
        // 会话归属的 CLI。解析不出时沿用旧行为落到 Claude —— 与下面的失败方向一致：
        // 认不出 CLI 意味着「不认识这个 CLI」，而不是「会话没了」，不能据此判会话不存在。
        let kind = CliKind::from_id(cli_id.as_deref()).unwrap_or(CliKind::Claude);
        // 会话不存在 -> 走归档兜底。判定经注册表：库型会话（`cli://…`）没有文件，
        // 裸 `Path::exists()` 会把它们一律当成「文件不存在」而误报会话丢失。
        if !source_for(kind).exists(&SessionLocator::decode(kind, &file_path)) {
            match app_db::read_archived_session_content(kind.id(), &file_path) {
                Ok(Some(data)) => {
                    let content = match String::from_utf8(data) {
                        Ok(s) => s,
                        Err(e) => {
                            // 归档内容是**第三方**字节（`std::str::Utf8Error` 的文案是英文），
                            // 按 R3 进 `params.detail`，包装文案由语言包给。
                            streaming::emit_error(
                                app,
                                topic,
                                request_id,
                                AppError::coded("session.archive_decode_failed")
                                    .with("detail", e.to_string()),
                            );
                            return;
                        }
                    };
                    let messages = parser::parse_session_content_by_kind(&file_path, &content);
                    if !messages.is_empty() {
                        streaming::emit_chunk(app, topic, request_id, &messages);
                    }
                    streaming::emit_done(app, topic, request_id, &SessionStreamDone {
                        offset: 0,
                        subagent_map: HashMap::new(),
                    });
                    return;
                }
                Ok(None) => {
                    streaming::emit_error(app, topic, request_id,
                        AppError::coded("session.file_missing").with("path", file_path.as_str()));
                    return;
                }
                Err(e) => {
                    // 不能写成 `e.to_string()`：`Coded` 的 `Display` 是裸 code，
                    // 压成字符串后前端 `renderAppError` 会原样上屏，用户看到的是码不是文案。
                    streaming::emit_error(app, topic, request_id, e);
                    return;
                }
            }
        }

        // 文件存在 -> 流式解析 + 分批 emit（批次策略统一在 parser::batch）
        let result = parser::parse_session_file_streaming(
            &file_path,
            skip_sidechain,
            |batch| {
                streaming::emit_chunk(app, topic, request_id, &batch);
                true
            },
        );

        match result {
            Ok((offset, subagent_map)) => {
                streaming::emit_done(app, topic, request_id, &SessionStreamDone {
                    offset,
                    subagent_map,
                });
            }
            Err(e) => streaming::emit_error(app, topic, request_id, e),
        }
    });

    Ok(())
}

/// 导出成功后的结果。**不是错误**——前端据此渲染本地化提示。
///
/// `count` 只在使用它的变体里出现：`Single` 的会话数恒为 1，给它一个 `count` 字段
/// 只会让前端拿到一个假参数，还会诱使渲染层去插值一个不该出现的数字。
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ExportOutcome {
    Single,
    Merged { count: usize },
    Separate { count: usize },
}

#[tauri::command]
pub fn export_session(
    cli_id: String,
    file_path: String,
    save_path: String,
    format: String,
    selected_indexes: Option<Vec<usize>>,
) -> AppResult<ExportOutcome> {
    CliKind::from_id(Some(cli_id.as_str()))?;
    let mut messages = parser::parse_session_file(&file_path)
        .map_err(|_| AppError::coded("session.content_load_failed"))?;
    
    if let Some(indexes) = selected_indexes {
        let mut index_set = std::collections::HashSet::new();
        for i in indexes { index_set.insert(i); }
        let mut filtered = Vec::new();
        for (i, msg) in messages.into_iter().enumerate() {
            if index_set.contains(&i) {
                filtered.push(msg);
            }
        }
        messages = filtered;
    }

    let output = match format.as_str() {
        "markdown" => format_as_markdown(&messages),
        "json" => format_as_json(&messages),
        "jsonl" => {
            let mut s = String::new();
            for m in &messages {
                if let Ok(line) = serde_json::to_string(m) {
                    s.push_str(&line);
                    s.push('\n');
                }
            }
            s
        },
        _ => format_as_text(&messages),
    };

    // Ensure the save path has the correct file extension
    let ext = match format.as_str() {
        "markdown" => ".md",
        "json" => ".json",
        "jsonl" => ".jsonl",
        _ => ".txt",
    };
    let final_path = if save_path.ends_with(ext) {
        save_path
    } else {
        format!("{}{}", save_path, ext)
    };

    // Validate the parent directory exists
    let final_path_buf = std::path::Path::new(&final_path);
    if let Some(parent) = final_path_buf.parent() {
        if !parent.exists() {
            return Err(
                AppError::coded("session.dir_not_found").with("path", parent.display().to_string())
            );
        }
    }

    let mut file =
        fs::File::create(&final_path).map_err(|_| AppError::coded("session.file_create_failed"))?;
    file.write_all(output.as_bytes())
        .map_err(|_| AppError::coded("session.file_create_failed"))?;

    Ok(ExportOutcome::Single)
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchExportSessionIdentity {
    cli_id: String,
    file_path: String,
}

#[tauri::command]
pub fn batch_export_sessions(
    sessions: Vec<BatchExportSessionIdentity>,
    save_path: String,
    format: String,
    mode: String,
) -> AppResult<ExportOutcome> {
    let file_paths = sessions
        .into_iter()
        .map(|identity| {
            CliKind::from_id(Some(identity.cli_id.as_str()))?;
            if identity.file_path.trim().is_empty() {
                return Err(AppError::coded("session.path_required"));
            }
            Ok(identity.file_path)
        })
        .collect::<AppResult<Vec<_>>>()?;
    let ext = match format.as_str() {
        "markdown" => ".md",
        "json" => ".json",
        _ => ".txt",
    };

    if mode == "merged" {
        let mut all_messages: Vec<ChatMessage> = Vec::new();
        for fp in &file_paths {
            let msgs = parser::parse_session_file(fp)
                .map_err(|_| AppError::coded("session.load_failed").with("path", fp.as_str()))?;
            all_messages.extend(msgs);
        }

        let output = match format.as_str() {
            "markdown" => format_as_markdown(&all_messages),
            "json" => format_as_json(&all_messages),
            _ => format_as_text(&all_messages),
        };

        let final_path = if save_path.ends_with(ext) {
            save_path
        } else {
            format!("{}{}", save_path, ext)
        };

        let mut file = fs::File::create(&final_path)
            .map_err(|_| AppError::coded("session.file_create_failed"))?;
        file.write_all(output.as_bytes())
            .map_err(|_| AppError::coded("session.file_write_failed"))?;

        Ok(ExportOutcome::Merged { count: file_paths.len() })
    } else {
        let base = if save_path.ends_with(ext) {
            save_path[..save_path.len() - ext.len()].to_string()
        } else {
            save_path.clone()
        };

        for (i, fp) in file_paths.iter().enumerate() {
            let msgs = parser::parse_session_file(fp)
                .map_err(|_| AppError::coded("session.load_failed").with("path", fp.as_str()))?;

            let output = match format.as_str() {
                "markdown" => format_as_markdown(&msgs),
                "json" => format_as_json(&msgs),
                _ => format_as_text(&msgs),
            };

            let final_path = format!("{}_{}{}", base, i + 1, ext);
            let mut file = fs::File::create(&final_path)
                .map_err(|_| AppError::coded("session.file_create_failed"))?;
            file.write_all(output.as_bytes())
                .map_err(|_| AppError::coded("session.file_write_failed"))?;
        }

        Ok(ExportOutcome::Separate { count: file_paths.len() })
    }
}

/// Fork a session at a specific transcript entry (identified by its uuid).
/// Copies the original JSONL up to and including the line carrying `anchor_uuid`,
/// rewrites every kept line's `sessionId` to a fresh UUID, writes a new file next
/// to the original, and returns the new session ID.
/// 递归复制目录（file-history 快照用，目标已存在时跳过整体拷贝）
pub(crate) fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if from.is_dir() {
            copy_dir_recursive(&from, &to)?;
        } else {
            fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

/// Fork 核心（纯函数，便于单测）：保留到 anchor_uuid 所在行（含），此后全部丢弃；
/// 所有保留行的 sessionId 重写为 new_id。anchor 未出现时报错。
///
/// 返回 `AppResult`：锚点缺失的文案已迁成 `session.fork_anchor_missing`。
/// **退回 `Result<_, String>` 今天不再是静默降级，而是编译错误**：那条会在调用点把码
/// 经 `?` 退回 `Business` 的 `impl From<String> for AppError` 已由计划 13 的 T2 删除
/// （记录见 `error.rs`），故码不会被无声丢掉。
fn fork_truncate_lines(
    lines: impl IntoIterator<Item = String>,
    anchor_uuid: &str,
    new_id: &str,
) -> AppResult<Vec<String>> {
    let mut kept: Vec<String> = Vec::new();
    let mut found = false;

    for line in lines {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            kept.push(line);
            continue;
        }

        let entry: serde_json::Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(_) => {
                kept.push(line);
                continue;
            }
        };

        // 锚点匹配链必须与 parser/workbuddy.rs 的 uuid 取值保持一致：
        // WorkBuddy 的 function_call 行优先 callId（message 级 id 在多工具调用时重复），
        // 其余行维持 uuid → id → callId。
        let anchor_candidate = if entry.get("type").and_then(|v| v.as_str()) == Some("function_call")
        {
            entry
                .get("uuid")
                .or_else(|| entry.get("callId"))
                .or_else(|| entry.get("id"))
        } else {
            entry
                .get("uuid")
                .or_else(|| entry.get("id"))
                .or_else(|| entry.get("callId"))
        };
        let is_anchor = anchor_candidate.and_then(|u| u.as_str()) == Some(anchor_uuid);

        // Rewrite sessionId so the forked transcript owns a fresh session identity.
        let new_line = if entry.get("sessionId").is_some() {
            let mut obj = entry;
            obj["sessionId"] = serde_json::Value::String(new_id.to_string());
            serde_json::to_string(&obj).unwrap_or(line)
        } else {
            line
        };

        kept.push(new_line);
        if is_anchor {
            found = true;
            break;
        }
    }

    if !found {
        return Err(AppError::coded("session.fork_anchor_missing"));
    }
    Ok(kept)
}

pub(crate) fn sync_workbuddy_forked_session_to_db(
    db_path: &Path,
    old_id: &str,
    new_id: &str,
) -> Result<(), rusqlite::Error> {
    let conn = rusqlite::Connection::open(db_path)?;
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);

    let rows_affected = conn.execute(
        r#"
        INSERT INTO sessions (
            id, cwd, user_id, title, custom_title, status, created_at, updated_at, last_activity_at, deleted_at,
            is_playground, source_mode, is_background_automation, mode, model, expert_id, expert_locale,
            expert_runtime_identity, expert_marketplace, permission_mode, use_sandbox_cli, project_id,
            plugin_context_json, last_user_prompt_expert_selection
        )
        SELECT 
            ?1, cwd, user_id,
            CASE 
                WHEN title IS NOT NULL AND title != '' THEN title || ' (分支)' 
                ELSE '新建分支会话' 
            END,
            custom_title, 'completed',
            ?2, ?2, ?2, NULL,
            is_playground, source_mode, is_background_automation, mode, model, expert_id, expert_locale,
            expert_runtime_identity, expert_marketplace, permission_mode, use_sandbox_cli, project_id,
            plugin_context_json, last_user_prompt_expert_selection
        FROM sessions WHERE id = ?3;
        "#,
        rusqlite::params![new_id, now_ms, old_id],
    )?;

    if rows_affected == 0 {
        let _ = conn.execute(
            r#"
            INSERT INTO sessions (
                id, cwd, user_id, title, status, created_at, updated_at, last_activity_at, is_playground
            ) VALUES (?1, '', 'default', '新建分支会话', 'completed', ?2, ?2, ?2, 0);
            "#,
            rusqlite::params![new_id, now_ms],
        );
    }

    Ok(())
}

#[tauri::command]
pub fn fork_session(
    cli_id: Option<String>,
    file_path: String,
    anchor_uuid: String,
) -> AppResult<String> {
    use std::io::{BufRead, BufReader};
    use uuid::Uuid;

    let kind = CliKind::from_id(cli_id.as_deref())?;
    // 守卫：没有分叉能力的 CLI 明确报错，错误码与改动前逐字相同
    let Some(fork) = crate::cli_registry::source_for(kind).fork() else {
        return Err(AppError::coded("session.continue_unsupported").with("cli", kind.name()));
    };

    // Validate file_path is within the CLI's sessions directory
    let sessions_root = cli::sessions_dir(kind)?;
    let file_path_buf = Path::new(&file_path);
    if !file_path_buf.starts_with(&sessions_root) {
        return Err(AppError::coded("session.path_invalid"));
    }

    // 与 `parser/claude.rs` 的会话文件读取共用同一个码：同一句「打开会话文件失败」只建一个码，
    // 否则同一句英文在语言包里出现两遍，译文一改就要改两处。
    let file = fs::File::open(&file_path)
        .map_err(|e| AppError::coded("parser.file_open_failed").with("detail", e.to_string()))?;
    let reader = BufReader::new(file);

    let new_id = Uuid::new_v4().to_string();
    let kept_lines =
        fork_truncate_lines(reader.lines().map_while(Result::ok), &anchor_uuid, &new_id)?;

    // Write new file next to the original
    let original_path = Path::new(&file_path);
    let parent = original_path
        .parent()
        .ok_or(AppError::coded("session.parent_dir_missing"))?;
    let new_file_path = parent.join(format!("{}.jsonl", new_id));

    let mut writer =
        fs::File::create(&new_file_path).map_err(|e| AppError::coded("session.fork_file_create_failed").with("detail", e.to_string()))?;
    for line in &kept_lines {
        writeln!(writer, "{}", line)?;
    }

    // 后置动作：原来的 if kind == Claude { ... } else if kind == WorkBuddy { ... }
    // 换成一次调用。old_id 是原文件的词干。
    if let Some(old_id) = original_path.file_stem().and_then(|s| s.to_str()) {
        fork.after_fork(old_id, &new_id);
    }

    Ok(new_id)
}

#[derive(Debug, Clone)]
pub(crate) struct AggregatedSessionSearch {
    session_path: String,
    snippet: String,
    matched_field: Option<session::MatchedField>,
    match_count: usize,
    matched_doc_count: usize,
    first_match_message_index: Option<usize>,
    rank: f64,
}

fn count_case_insensitive_occurrences(text: &str, query_lower: &str) -> usize {
    if text.is_empty() || query_lower.is_empty() {
        return 0;
    }

    text.to_lowercase().matches(query_lower).count()
}

fn aggregate_session_searches(
    query: &str,
    candidates: Vec<app_db::SessionSearchCandidate>,
    matched_docs: Vec<app_db::SessionSearchMatchedDoc>,
) -> Vec<AggregatedSessionSearch> {
    let query_lower = query.to_lowercase();
    let mut docs_by_session: HashMap<String, Vec<app_db::SessionSearchMatchedDoc>> = HashMap::new();
    for doc in matched_docs {
        docs_by_session
            .entry(doc.session_path.clone())
            .or_default()
            .push(doc);
    }

    let mut aggregated = Vec::new();
    for candidate in candidates {
        let docs = docs_by_session.remove(&candidate.session_path).unwrap_or_default();
        if docs.is_empty() {
            continue;
        }

        let mut match_count = 0usize;
        let mut snippet = String::new();
        let mut matched_field = None;
        let mut first_match_message_index = candidate.first_match_message_index;

        for doc in docs {
            let current_count = count_case_insensitive_occurrences(&doc.search_text, &query_lower);
            if current_count == 0 {
                continue;
            }
            match_count += current_count;
            if snippet.is_empty() {
                snippet = session::extract_snippet(&doc.search_text, query, 100);
                matched_field = doc.matched_field;
                first_match_message_index = Some(doc.message_index);
            }
        }

        if match_count == 0 {
            continue;
        }

        aggregated.push(AggregatedSessionSearch {
            session_path: candidate.session_path,
            snippet,
            matched_field,
            match_count,
            matched_doc_count: candidate.matched_doc_count,
            first_match_message_index,
            rank: candidate.rank,
        });
    }

    aggregated.sort_by(|a, b| {
        let a_priority = if a.rank < 0.0 { a.rank } else { 0.0 };
        let b_priority = if b.rank < 0.0 { b.rank } else { 0.0 };
        a_priority
            .partial_cmp(&b_priority)
            .unwrap_or(Ordering::Equal)
            .then_with(|| b.match_count.cmp(&a.match_count))
            .then_with(|| b.matched_doc_count.cmp(&a.matched_doc_count))
            .then_with(|| a.rank.partial_cmp(&b.rank).unwrap_or(Ordering::Equal))
            .then_with(|| a.first_match_message_index.cmp(&b.first_match_message_index))
            .then_with(|| a.session_path.cmp(&b.session_path))
    });
    aggregated.truncate(50);
    aggregated
}

fn find_custom_name_matches(
    kind: CliKind,
    custom_names: &HashMap<String, String>,
    query: &str,
) -> Vec<(String, String)> {
    let query_lower = query.to_lowercase();
    let mut matches = Vec::new();
    for (key, name) in custom_names {
        if !name.to_lowercase().contains(&query_lower) {
            continue;
        }
        // 键的解析必须走 app_db 的共享实现：读写两端一旦各认一种分隔符，
        // 复合键就会被整串（含 NUL 字节）当成会话路径喂给搜索结果
        if let Some(path) = app_db::session_name_path(kind, key) {
            matches.push((path.to_string(), name.clone()));
        }
    }
    matches
}

fn merge_title_and_id_search_matches(
    query: &str,
    mut candidates: Vec<app_db::SessionSearchCandidate>,
    matched_docs: Vec<app_db::SessionSearchMatchedDoc>,
    session_id_matches: Vec<app_db::SessionListIndexRecord>,
    title_matches: Vec<app_db::SessionListIndexRecord>,
    custom_name_matches: Vec<(String, String)>,
) -> (Vec<app_db::SessionSearchCandidate>, Vec<app_db::SessionSearchMatchedDoc>) {
    let query_lower = query.to_lowercase();
    let mut candidate_map: HashMap<String, usize> = candidates
        .iter()
        .enumerate()
        .map(|(idx, c)| (c.session_path.clone(), idx))
        .collect();
    let mut metadata_docs = Vec::new();

    // 1. 自定义重命名会话命中（用户主动自定义，赋予强置顶权重 rank -2.0）
    for (session_path, custom_name) in custom_name_matches {
        if let Some(&idx) = candidate_map.get(&session_path) {
            candidates[idx].rank = candidates[idx].rank.min(-2.0);
            candidates[idx].matched_doc_count += 1;
        } else {
            candidate_map.insert(session_path.clone(), candidates.len());
            candidates.push(app_db::SessionSearchCandidate {
                session_path: session_path.clone(),
                first_match_message_index: Some(0),
                matched_doc_count: 1,
                rank: -2.0,
            });
        }
        metadata_docs.push(app_db::SessionSearchMatchedDoc {
            session_path,
            message_index: 0,
            search_text: custom_name,
            matched_field: Some(session::MatchedField::Title),
        });
    }

    // 2. 数据库会话标题与首条消息命中（rank -2.0）
    for record in title_matches {
        let session_path = record.session_path;
        // 文档文本就是命中字段的**值本身**：前缀一旦进 `search_text`，就会被
        // `extract_snippet` 切进 `snippet` 并原样上屏（`会话标题: 优化会话加载性能`）。
        // 「命中哪个字段」改由 `matched_field` 结构化带回，前端渲染本地化标签。
        let (matched_text, matched_field) = if let Some(title) = record
            .title
            .as_ref()
            .filter(|t| t.to_lowercase().contains(&query_lower))
        {
            (title.clone(), Some(session::MatchedField::Title))
        } else if let Some(first_msg) = record
            .first_user_message
            .as_ref()
            .filter(|m| m.to_lowercase().contains(&query_lower))
        {
            (first_msg.clone(), Some(session::MatchedField::FirstMessage))
        } else {
            // 既非标题也非首条消息命中：SQL 命中的是 session_path / project_path，
            // **或者**（title 缺席时）session_id —— `search_session_titles` 的 LIKE 也覆盖
            // session_id，所以这一支**不保证文本不含查询词**：`title IS NULL` 且查询词落在
            // session_id 上时，文本就是 session_id、计数非零，并且因为本支的文档排在
            // 第 3 步的会话 ID 文档之前，它会**成为 snippet 的来源**。那种情况下命中的
            // 事实就是会话 ID，必须标成 `SessionId`，否则前端只能渲染一个裸 id。
            //
            // 保留本支是为了下面那次候选晋升（`rank.min(-2.0)` 与 `matched_doc_count += 1`）：
            // 「路径/ID 命中且同时有正文命中」的会话今天靠它排到最前，删掉会改变排序。
            match record.title.clone() {
                // 文本是标题且不含查询词 → 计数为 0，不会被选作 snippet 来源；
                // 这一支确实是「无从分类」，是 `None` 唯一还该出现的地方。
                Some(title) => (title, None),
                // title 缺席 → 文本是 session_id，命中事实可命名。
                None => (
                    record.session_id.clone(),
                    Some(session::MatchedField::SessionId),
                ),
            }
        };

        if let Some(&idx) = candidate_map.get(&session_path) {
            candidates[idx].rank = candidates[idx].rank.min(-2.0);
            candidates[idx].matched_doc_count += 1;
        } else {
            candidate_map.insert(session_path.clone(), candidates.len());
            candidates.push(app_db::SessionSearchCandidate {
                session_path: session_path.clone(),
                first_match_message_index: Some(0),
                matched_doc_count: 1,
                rank: -2.0,
            });
        }

        metadata_docs.push(app_db::SessionSearchMatchedDoc {
            session_path,
            message_index: 0,
            search_text: matched_text,
            matched_field,
        });
    }

    // 3. Session ID 命中（rank -1.0）
    for record in session_id_matches {
        let session_path = record.session_path;
        // 文档文本就是 session_id **本身**：`Session ID: ` 前缀一旦进 `search_text`，
        // 就会被 `extract_snippet` 切进 `snippet` 并原样上屏（与标题前缀同源）。
        // 命中事实改由 `matched_field` 结构化带回，前缀句由前端按语言渲染。
        let session_id_text = record.session_id;

        if let Some(&idx) = candidate_map.get(&session_path) {
            candidates[idx].rank = candidates[idx].rank.min(-1.0);
            candidates[idx].matched_doc_count += 1;
        } else {
            candidate_map.insert(session_path.clone(), candidates.len());
            candidates.push(app_db::SessionSearchCandidate {
                session_path: session_path.clone(),
                first_match_message_index: Some(0),
                matched_doc_count: 1,
                rank: -1.0,
            });
        }

        metadata_docs.push(app_db::SessionSearchMatchedDoc {
            session_path,
            message_index: 0,
            search_text: session_id_text,
            matched_field: Some(session::MatchedField::SessionId),
        });
    }

    // 优先将标题与 ID 匹配的元数据文档放在前面，保证摘要提取优先命中标题
    metadata_docs.extend(matched_docs);
    (candidates, metadata_docs)
}

#[cfg(test)]
fn merge_session_id_search_matches(
    query: &str,
    candidates: Vec<app_db::SessionSearchCandidate>,
    matched_docs: Vec<app_db::SessionSearchMatchedDoc>,
    session_id_matches: Vec<app_db::SessionListIndexRecord>,
) -> (Vec<app_db::SessionSearchCandidate>, Vec<app_db::SessionSearchMatchedDoc>) {
    merge_title_and_id_search_matches(
        query,
        candidates,
        matched_docs,
        session_id_matches,
        Vec::new(),
        Vec::new(),
    )
}

fn fallback_session_id_from_path(file_path: &str) -> String {
    Path::new(file_path)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("")
        .to_string()
}

pub(crate) fn build_claude_search_results(
    aggregated: Vec<AggregatedSessionSearch>,
    records: &HashMap<String, app_db::SessionListIndexRecord>,
    custom_names: &HashMap<String, String>,
    session_map: &HashMap<String, history::HistoryEntry>,
    project_map: &HashMap<String, String>,
) -> Vec<SearchResult> {
    aggregated
        .into_iter()
        .map(|entry| {
            let record = records.get(&entry.session_path);
            let session_id = record
                .map(|value| value.session_id.clone())
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| fallback_session_id_from_path(&entry.session_path));
            let encoded_dir = Path::new(&entry.session_path)
                .parent()
                .and_then(|parent| parent.file_name())
                .and_then(|name| name.to_str())
                .filter(|value| !value.is_empty())
                .unwrap_or(session::UNKNOWN_PROJECT_DIR)
                .to_string();
            let project_path = session::resolve_project_path(
                &encoded_dir,
                record.and_then(|value| value.project_path.as_deref()),
                Some(project_map),
            );
            let display_name = crate::title_resolver::resolve_display_name(
                custom_session_name(custom_names, CliKind::Claude, &entry.session_path),
                record.and_then(|value| value.title.as_deref()),
                record.and_then(|value| value.first_user_message.as_deref()),
                session_map
                    .get(&session_id)
                    .map(|history| history.display.as_str())
                    .filter(|value| !value.is_empty()),
                &session_id,
            );

            SearchResult {
                session_id,
                file_path: entry.session_path,
                display_name,
                project_path,
                snippet: entry.snippet,
                matched_field: entry.matched_field,
                match_count: entry.match_count,
                first_match_message_index: entry.first_match_message_index,
                cli_id: String::new(),
            }
        })
        .collect()
}

pub(crate) fn build_codex_search_results(
    aggregated: Vec<AggregatedSessionSearch>,
    records: &HashMap<String, app_db::SessionListIndexRecord>,
    custom_names: &HashMap<String, String>,
    session_map: &HashMap<String, history::HistoryEntry>,
    // 该 CLI 的自定义标题表（`session_id → 标题`），没有则传空表。
    titles: &HashMap<String, String>,
    kind: CliKind,
) -> Vec<SearchResult> {
    // 单次读取 Codex 索引 thread_name Map，整批搜索结果 O(1) 命中
    let codex_titles = parser::load_codex_index_titles();
    aggregated
        .into_iter()
        .map(|entry| {
            let record = records.get(&entry.session_path);
            let session_id = record
                .map(|value| value.session_id.clone())
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| {
                    Path::new(&entry.session_path)
                        .file_stem()
                        .and_then(|stem| stem.to_str())
                        .and_then(|stem| stem.rsplit('-').next())
                        .unwrap_or("")
                        .to_string()
                });
            let project_path = record
                .and_then(|value| value.project_path.clone())
                .filter(|value| !value.trim().is_empty());
            let title = record
                .and_then(|value| value.title.as_deref())
                .or_else(|| codex_titles.get(&session_id).map(String::as_str))
                .or_else(|| {
                    Path::new(&entry.session_path)
                        .file_stem()
                        .and_then(|stem| stem.to_str())
                        .and_then(|stem| codex_titles.get(stem).map(String::as_str))
                });
            let title = titles
                .get(&session_id)
                .or_else(|| {
                    Path::new(&entry.session_path)
                        .file_stem()
                        .and_then(|stem| stem.to_str())
                        .and_then(|stem| titles.get(stem))
                })
                .map(String::as_str)
                .or(title);
            let display_name = crate::title_resolver::resolve_display_name(
                custom_session_name(custom_names, kind, &entry.session_path),
                title,
                record.and_then(|value| value.first_user_message.as_deref()),
                session_map
                    .get(&session_id)
                    .map(|history| history.display.as_str())
                    .filter(|value| !value.is_empty()),
                &session_id,
            );

            SearchResult {
                session_id,
                file_path: entry.session_path,
                display_name,
                project_path,
                snippet: entry.snippet,
                matched_field: entry.matched_field,
                match_count: entry.match_count,
                first_match_message_index: entry.first_match_message_index,
                cli_id: String::new(),
            }
        })
        .collect()
}

/// 内部 helper：执行 search_sessions 的完整 pipeline。
/// 已假定 `trimmed` 非空、`kind` 已解析、search index 已 ready。
/// 抽出供 search_sessions（sync 命令）与 search_sessions_stream 共用。
fn run_search_sessions_pipeline(kind: CliKind, trimmed: &str) -> AppResult<Vec<SearchResult>> {
    let candidates = app_db::search_session_candidates(kind, trimmed, 200)?;
    let mut session_id_matches = app_db::search_session_ids(kind, trimmed, 50)?;
    let mut title_matches = app_db::search_session_titles(kind, trimmed, 50)?;
    let custom_names = load_session_names().unwrap_or_default();
    let mut custom_name_matches = find_custom_name_matches(kind, &custom_names, trimmed);

    // 双轨墓碑过滤：确保已删除的会话不会从任何渠道（内容、ID、标题、自定义重命名）复活
    session_id_matches.retain(|r| {
        !app_db::is_session_tombstoned(kind, &r.session_id, &r.session_path).unwrap_or(false)
    });
    title_matches.retain(|r| {
        !app_db::is_session_tombstoned(kind, &r.session_id, &r.session_path).unwrap_or(false)
    });
    custom_name_matches.retain(|(path, _)| {
        !app_db::is_session_tombstoned(kind, "", path).unwrap_or(false)
    });

    if candidates.is_empty()
        && session_id_matches.is_empty()
        && title_matches.is_empty()
        && custom_name_matches.is_empty()
    {
        return Ok(Vec::new());
    }

    let session_paths: Vec<String> = candidates
        .iter()
        .map(|candidate| candidate.session_path.clone())
        .collect();
    let matched_docs = app_db::read_session_search_docs_for_paths(kind, trimmed, &session_paths)?;
    let (candidates, matched_docs) = merge_title_and_id_search_matches(
        trimmed,
        candidates,
        matched_docs,
        session_id_matches,
        title_matches,
        custom_name_matches,
    );
    let aggregated = aggregate_session_searches(trimmed, candidates, matched_docs);
    if aggregated.is_empty() {
        return Ok(Vec::new());
    }

    let resolved_paths: Vec<String> = aggregated
        .iter()
        .map(|entry| entry.session_path.clone())
        .collect();
    let records = app_db::read_session_list_index_for_paths(kind, &resolved_paths)?;

    // 六臂分派归位到各源：用哪条装配路径、喂哪份 history 映射是各 CLI 的整形知识。
    let mut results = crate::cli_registry::source_for(kind)
        .build_search_results(aggregated, &records, &custom_names)?;
    stamp_results_cli_id(&mut results, kind);
    Ok(results)
}

/// 为搜索结果盖戳来源 CLI。抽成小函数以便单元测试直接断言盖戳行为
/// （完整 pipeline 依赖全局 DB 与 tantivy 物理索引,无测试替身,见 cross_cli_tests）。
fn stamp_results_cli_id(results: &mut [SearchResult], kind: CliKind) {
    for r in results.iter_mut() {
        r.cli_id = kind.id().to_string();
    }
}

const SEARCH_SESSIONS_TOPIC: &str = "search_sessions";
const SEARCH_SESSIONS_BATCH_SIZE: usize = 20;

#[derive(Debug, Clone, serde::Serialize)]
struct SearchSessionsStreamDone {
    total: usize,
    query: String,
    /// 所选范围内从未建立内容索引、本次未参与搜索的 CLI
    pending_cli_ids: Vec<String>,
    /// 所选范围内参与了搜索但索引有增量未同步的 CLI(结果可能缺少最新内容)
    #[serde(default)]
    stale_cli_ids: Vec<String>,
    #[serde(default)]
    cli_errors: Vec<SearchStreamCliError>,
}

#[derive(Debug, Clone, serde::Serialize)]
struct SearchStreamCliError {
    cli_id: String,
    /// 分项失败原样携带 `AppError`（`Coded` → `{code, params}`），不压成裸 code。
    /// 前端消费点：`useSessions.ts` 的 `cli_errors[].error` → `searchCliErrors` → `App.vue`。
    error: AppError,
}

/// 多 CLI fan-out 中单个 CLI 的处置决策(纯函数,便于测试)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AllSearchDisposition {
    /// 无数据目录/无会话:不参与,也不提示
    Skip,
    /// 从未建立过内容索引(或物理索引缺失):进 pending 引导
    Pending,
    /// 参与搜索;stale = 有会话更新尚未同步进索引(仍搜索已有内容)
    Search { stale: bool },
}

fn all_search_disposition(
    data_dir_exists: bool,
    physical_index_ready: bool,
    missing_sessions: usize,
    total_sessions: usize,
) -> AllSearchDisposition {
    if !data_dir_exists {
        return AllSearchDisposition::Skip;
    }
    if !physical_index_ready {
        // 物理索引缺失:有会话才值得引导重建
        return if total_sessions > 0 {
            AllSearchDisposition::Pending
        } else {
            AllSearchDisposition::Skip
        };
    }
    if total_sessions > 0 && missing_sessions >= total_sessions {
        // 一条都没索引过 → 尚未建立
        return AllSearchDisposition::Pending;
    }
    AllSearchDisposition::Search {
        stale: missing_sessions > 0,
    }
}

enum SearchKindStreamResult {
    Skip,
    Pending,
    Search {
        results: Vec<SearchResult>,
        stale: bool,
    },
}

fn search_kind_for_stream(kind: CliKind, trimmed: &str) -> AppResult<SearchKindStreamResult> {
    let status = super::session_index::resolve_search_index_status(kind)?;
    match all_search_disposition(
        true,
        status.physical_index_ready,
        status.missing_sessions,
        status.total_sessions,
    ) {
        AllSearchDisposition::Skip => Ok(SearchKindStreamResult::Skip),
        AllSearchDisposition::Pending => Ok(SearchKindStreamResult::Pending),
        AllSearchDisposition::Search { stale } => Ok(SearchKindStreamResult::Search {
            results: run_search_sessions_pipeline(kind, trimmed)?,
            stale,
        }),
    }
}

/// 流式全局搜索：通过 `search_sessions:<request_id>:chunk` / `:done` / `:error` 推送。
/// chunk payload = `Vec<SearchResult>`（每批 20 条），顺序与 search_sessions 同步命令一致。
/// 对 cli_ids 指定的 CLI fan-out，front_cli_id 在集合中时优先；索引未就绪的 CLI
/// 记入 done 事件的 pending_cli_ids，单来源失败则记入 cli_errors 并继续其他来源。
/// 命令本身立刻返回 Ok(())。
#[tauri::command]
pub async fn search_sessions_stream(
    app: AppHandle,
    request_id: String,
    cli_ids: Option<Vec<String>>,
    front_cli_id: Option<String>,
    query: String,
    cli_id: Option<String>,
) -> AppResult<()> {
    let trimmed = query.trim().to_string();
    let effective_ids = super::session_index::effective_cli_ids(cli_ids, cli_id);
    let kinds = super::session_index::requested_kinds(
        effective_ids.as_deref(),
        front_cli_id.as_deref(),
    )?;

    streaming::spawn_streaming_task(app, SEARCH_SESSIONS_TOPIC, request_id, move |app, topic, request_id| {
        if trimmed.is_empty() {
            // 空 query 快路径：不查 DB，直接 done(0)。前端 useStreamingCollection 会清空 items
            streaming::emit_done(
                app,
                topic,
                request_id,
                &SearchSessionsStreamDone {
                    total: 0,
                    query: trimmed.clone(),
                    pending_cli_ids: Vec::new(),
                    stale_cli_ids: Vec::new(),
                    cli_errors: Vec::new(),
                },
            );
            return;
        }

        let mut pending: Vec<String> = Vec::new();
        let mut stale: Vec<String> = Vec::new();
        let mut cli_errors = Vec::new();
        let mut total = 0usize;
        super::session_index::run_cli_fanout(
            &kinds,
            |kind| search_kind_for_stream(kind, &trimmed),
            |outcome| match outcome.result {
                Ok(SearchKindStreamResult::Skip) => {}
                Ok(SearchKindStreamResult::Pending) => {
                    pending.push(outcome.kind.id().to_string());
                }
                Ok(SearchKindStreamResult::Search { results, stale: is_stale }) => {
                    if is_stale {
                        stale.push(outcome.kind.id().to_string());
                    }
                    total += results.len();
                    for batch in results.chunks(SEARCH_SESSIONS_BATCH_SIZE) {
                        streaming::emit_chunk(app, topic, request_id, &batch.to_vec());
                    }
                }
                Err(error) => {
                    cli_errors.push(SearchStreamCliError {
                        cli_id: outcome.kind.id().to_string(),
                        error,
                    });
                }
            },
        );
        streaming::emit_done(
            app,
            topic,
            request_id,
            &SearchSessionsStreamDone {
                total,
                query: trimmed.clone(),
                pending_cli_ids: pending,
                stale_cli_ids: stale,
                cli_errors,
            },
        );
    });

    Ok(())
}

// ---- Session Stats ----

#[derive(Debug, Clone, serde::Serialize)]
pub struct SessionStats {
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_cache_creation_tokens: u64,
    pub total_cache_read_tokens: u64,
    pub total_duration_ms: u64,
    pub turn_count: usize,
}

#[tauri::command]
pub async fn get_session_stats(
    cli_id: Option<String>,
    file_path: String,
) -> AppResult<SessionStats> {
    let _kind = CliKind::from_id(cli_id.as_deref())?;
    tauri::async_runtime::spawn_blocking(move || {
        let records = parser::extract_usage_records(&file_path, "");
        let mut stats = SessionStats {
            total_input_tokens: 0,
            total_output_tokens: 0,
            total_cache_creation_tokens: 0,
            total_cache_read_tokens: 0,
            total_duration_ms: 0,
            turn_count: records.len(),
        };
        for r in &records {
            stats.total_input_tokens += r.input_tokens;
            stats.total_output_tokens += r.output_tokens;
            stats.total_cache_creation_tokens += r.cache_creation_tokens;
            stats.total_cache_read_tokens += r.cache_read_tokens;
            if let Some(dur) = r.duration_ms {
                stats.total_duration_ms += dur;
            }
        }
        Ok(stats)
    })
    .await
    .unwrap_or_else(|_| Err(AppError::coded("session.task_failed")))
}

#[tauri::command]
pub fn rename_session(cli_id: String, file_path: String, new_name: String) -> AppResult<()> {
    let kind = CliKind::from_id(Some(cli_id.as_str()))?;
    let mut names = load_session_names().unwrap_or_default();
    let key = app_db::session_name_key(kind, &file_path);
    if new_name.trim().is_empty() {
        names.remove(&key);
    } else {
        names.insert(key, new_name.trim().to_string());
    }
    app_db::write_session_names(&names)
}

#[tauri::command]
pub async fn get_usage_stats(cli_id: Option<String>) -> AppResult<Vec<UsageRecord>> {
    let kind = CliKind::from_id(cli_id.as_deref())?;
    tauri::async_runtime::spawn_blocking(move || get_usage_stats_sync(kind))
        .await
        .map_err(|e| AppError::coded("session.usage_stats_task_failed").with("detail", e.to_string()))?
}

fn get_usage_stats_sync(kind: CliKind) -> AppResult<Vec<UsageRecord>> {
    // 没有用量统计能力的 CLI 直接返回空，不再靠「返回空表」表达不支持。
    if crate::cli_registry::source_for(kind).usage_stats().is_none() {
        return Ok(Vec::new());
    }

    let projects = super::scan_projects_inner_for_cli(kind, None, None, false)?.projects;
    let mut all_records: Vec<UsageRecord> = Vec::new();

    for project in projects {
        // 项目名是**派生值**：原路径缺席时留空，用量面板的 `getProjectName` 自带本地化兜底句，
        // 不在这里造中文（否则那句中文会跟着用量记录跨 IPC 上屏）。
        let project_name = match project.original_path.as_deref() {
            Some(path) => {
                let normalized = path.replace('\\', "/");
                normalized
                    .split('/')
                    .filter(|s| !s.is_empty())
                    .last()
                    .unwrap_or(path)
                    .to_string()
            }
            None => String::new(),
        };

        for session_info in project.sessions {
            let records =
                parser::extract_usage_records(&session_info.file_path, &project_name);
            all_records.extend(records);
        }
    }

    Ok(all_records)
}

/// Load custom session names map
pub(crate) fn load_session_names() -> AppResult<HashMap<String, String>> {
    app_db::load_session_names()
}

/// 读取自定义会话名。键的构造与解析统一放在 `app_db`（与 session_names 表同源），
/// 避免读写两端各认一种格式。
pub(crate) fn custom_session_name<'a>(
    names: &'a HashMap<String, String>,
    kind: CliKind,
    file_path: &str,
) -> Option<&'a str> {
    names
        .get(&app_db::session_name_key(kind, file_path))
        .map(String::as_str)
}

// ---- Format helpers ----

fn format_as_text(messages: &[ChatMessage]) -> String {
    let mut output = String::new();
    output.push('\u{FEFF}');

    for msg in messages {
        let ts = format_timestamp(&msg.timestamp);
        match msg.role.as_str() {
            "user" => output.push_str(&format!("[user] {}\n", ts)),
            "assistant" => {
                if let Some(ref model) = msg.model {
                    output.push_str(&format!("[assistant] {} ({})\n", ts, model));
                } else {
                    output.push_str(&format!("[assistant] {}\n", ts));
                }
            }
            _ => output.push_str(&format!("[{}] {}\n", msg.role, ts)),
        }
        for part in &msg.content_parts {
            match part {
                ContentPart::Text { text } => {
                    output.push_str(text);
                    output.push('\n');
                }
                ContentPart::ToolUse { summary, .. } => {
                    output.push_str(summary);
                    output.push('\n');
                }
                ContentPart::ToolResult { summary, .. } => {
                    output.push_str(summary);
                    output.push('\n');
                }
                ContentPart::Thinking { .. } | ContentPart::Image { .. } | ContentPart::ImageRef { .. } | ContentPart::ImageMeta => {}
            }
        }
        output.push('\n');
    }
    output
}

fn format_as_markdown(messages: &[ChatMessage]) -> String {
    let mut output = String::new();

    for msg in messages {
        let ts = format_timestamp(&msg.timestamp);
        let role_label = match msg.role.as_str() {
            "user" => "User".to_string(),
            "assistant" => match &msg.model {
                Some(model) => format!("Assistant ({})", model),
                None => "Assistant".to_string(),
            },
            other => other.to_string(),
        };

        output.push_str(&format!("### {} — {}\n\n", role_label, ts));

        for part in &msg.content_parts {
            match part {
                ContentPart::Text { text } => {
                    output.push_str(text);
                    output.push_str("\n\n");
                }
                ContentPart::ToolUse {
                    tool_name, input, ..
                } => {
                    output.push_str(&format!(
                        "<details>\n<summary><code>{}</code></summary>\n\n```json\n{}\n```\n\n</details>\n\n",
                        tool_name, input
                    ));
                }
                ContentPart::ToolResult {
                    summary, content, ..
                } => {
                    output.push_str(&format!(
                        "<details>\n<summary>{}</summary>\n\n```\n{}\n```\n\n</details>\n\n",
                        summary, content
                    ));
                }
                ContentPart::Thinking { thinking } => {
                    output.push_str(&format!(
                        "<details>\n<summary><em>Thinking...</em></summary>\n\n{}\n\n</details>\n\n",
                        thinking
                    ));
                }
                ContentPart::Image { .. } | ContentPart::ImageRef { .. } => {
                    output.push_str("*[Image]*\n\n");
                }
                ContentPart::ImageMeta => {}
            }
        }

        output.push_str("---\n\n");
    }
    output
}

fn format_as_json(messages: &[ChatMessage]) -> String {
    serde_json::to_string_pretty(messages).unwrap_or_else(|_| "[]".to_string())
}

#[tauri::command]
pub fn get_archive_retention_days() -> AppResult<i64> {
    app_db::get_archive_retention_days()
}

#[tauri::command]
pub fn set_archive_retention_days(days: i64) -> AppResult<()> {
    app_db::set_archive_retention_days(days)
}

#[tauri::command]
pub fn get_session_archive_pinned(cli_id: Option<String>, file_path: String) -> AppResult<bool> {
    let kind = CliKind::from_id(cli_id.as_deref())?;
    app_db::is_session_archive_pinned(kind, &file_path)
}

#[tauri::command]
pub fn get_session_archive_status(
    cli_id: Option<String>,
    file_path: String,
) -> AppResult<app_db::SessionArchiveStatus> {
    let kind = CliKind::from_id(cli_id.as_deref())?;
    app_db::get_session_archive_status(kind, &file_path)
}

#[tauri::command]
pub async fn set_session_archive_pinned(
    cli_id: Option<String>,
    file_path: String,
    pinned: bool,
) -> AppResult<bool> {
    // spawn_blocking：pin 会对整个会话 jsonl 做 gzip，大会话同步执行会卡主线程
    tauri::async_runtime::spawn_blocking(move || {
        let kind = CliKind::from_id(cli_id.as_deref())?;
        app_db::set_session_archive_pinned(kind, &file_path, pinned)
    })
    .await
    .map_err(|e| AppError::coded("session.archive_pin_failed").with("detail", e.to_string()))?
}

#[tauri::command]
pub async fn list_archived_sessions(
    cli_id: Option<String>,
) -> AppResult<Vec<app_db::ArchivedSessionEntry>> {
    // spawn_blocking：归档列表会触发退化行快照解析（gunzip），不能阻塞主线程
    tauri::async_runtime::spawn_blocking(move || {
        let kind = CliKind::from_id(cli_id.as_deref())?;
        // 救回历史 bug 误删索引行的归档会话（失败不阻塞列表）
        let _ = app_db::restore_missing_archived_index_rows(kind);
        app_db::list_archived_sessions(kind)
    })
    .await
    .map_err(|e| AppError::coded("session.archive_list_failed").with("detail", e.to_string()))?
}

#[tauri::command]
pub fn delete_session_archive(cli_id: Option<String>, file_path: String) -> AppResult<bool> {
    let kind = CliKind::from_id(cli_id.as_deref())?;
    app_db::delete_session_archive(kind, &file_path)?;
    Ok(true)
}

#[tauri::command]
pub fn restore_session_to_disk(cli_id: Option<String>, file_path: String) -> AppResult<bool> {
    let kind = CliKind::from_id(cli_id.as_deref())?;
    app_db::restore_session_to_disk(kind, &file_path)?;
    Ok(true)
}

#[tauri::command]
pub async fn rebuild_archive_index_from_disk(
    app: AppHandle,
    cli_id: Option<String>,
) -> AppResult<app_db::RebuildArchiveResult> {
    // spawn_blocking：全量 gunzip + 元数据解析为重 IO/CPU 操作，同步命令会冻结主线程
    tauri::async_runtime::spawn_blocking(move || {
        app_db::rebuild_archive_index_from_disk(&app, cli_id.as_deref())
    })
    .await
    .map_err(|e| AppError::coded("session.archive_rebuild_failed").with("detail", e.to_string()))?
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 类型级守卫：fan-out 的分项错误必须**原样**（不压平）跨 IPC。
    ///
    /// `run_cli_fanout` 曾把 `AppResult` 压成 `String`；`Coded` 的 `Display` 就是裸 code，
    /// 于是用户看到 `internal.database` 而不是文案。改回 `Result<T, String>` 能编译通过、
    /// 运行时也没有任何症状，故在此把类型与线上形状一并钉住。
    #[test]
    fn fanout_error_reaches_ipc_as_app_error() {
        fn assert_error_type(_: &AppError) {}

        let payload = SearchStreamCliError {
            cli_id: "claude".to_string(),
            error: AppError::coded("internal.database").with("detail", "boom"),
        };
        assert_error_type(&payload.error);

        let json = serde_json::to_value(&payload).unwrap();
        assert_eq!(json["error"]["code"], "internal.database");
        assert_eq!(json["error"]["params"]["detail"], "boom");
    }

    /// 导出结果的线上形状：前端按 `kind` 取语言包键，`count` 只在 `merged`/`separate` 上。
    ///
    /// 钉的是**形状**不是措辞——措辞已经全部搬进语言包。前端 `EXPORT_OUTCOME_KEY` 是
    /// `Record<ExportOutcome["kind"], string>`，`kind` 的取值集合一旦漂移，那份映射就会
    /// 在类型上落空；`Single` 多出个 `count` 则会让渲染层插一个恒为 1 的假数字。
    #[test]
    fn export_outcome_wire_shape_is_tagged_by_kind() {
        let single = serde_json::to_value(ExportOutcome::Single).unwrap();
        assert_eq!(single, serde_json::json!({ "kind": "single" }));

        let merged = serde_json::to_value(ExportOutcome::Merged { count: 3 }).unwrap();
        assert_eq!(merged, serde_json::json!({ "kind": "merged", "count": 3 }));

        let separate = serde_json::to_value(ExportOutcome::Separate { count: 3 }).unwrap();
        assert_eq!(separate, serde_json::json!({ "kind": "separate", "count": 3 }));
    }

    #[test]
    fn custom_names_are_scoped_by_cli_for_same_path() {
        let path = "/shared/session.jsonl";
        let mut names = HashMap::new();
        names.insert(app_db::session_name_key(CliKind::Claude, path), "Claude 对话".to_string());
        names.insert(app_db::session_name_key(CliKind::Codex, path), "Codex 对话".to_string());

        assert_eq!(custom_session_name(&names, CliKind::Claude, path), Some("Claude 对话"));
        assert_eq!(custom_session_name(&names, CliKind::Codex, path), Some("Codex 对话"));
    }

    #[test]
    fn resolve_session_id_rejects_ambiguous_claude_subagent() {
        let resolved = resolve_session_id(
            Some("claude".to_string()),
            "/tmp/parent-session/subagents/agent-123.jsonl".to_string(),
        )
        .unwrap();

        assert_eq!(resolved, None);
    }

    #[test]
    fn fork_truncate_keeps_up_to_anchor_inclusive_and_rewrites_session_id() {
        let lines = vec![
            r#"{"type":"summary","summary":"s"}"#.to_string(),
            r#"{"type":"user","uuid":"u1","sessionId":"old","message":{"content":"hi"}}"#.to_string(),
            r#"{"type":"assistant","uuid":"u2","sessionId":"old","message":{"content":[]}}"#.to_string(),
            r#"{"type":"user","uuid":"u3","sessionId":"old","message":{"content":"later"}}"#.to_string(),
        ];

        let kept = fork_truncate_lines(lines, "u2", "new-id").unwrap();

        assert_eq!(kept.len(), 3);
        for line in &kept {
            let v: serde_json::Value = serde_json::from_str(line).unwrap();
            if v.get("sessionId").is_some() {
                assert_eq!(v["sessionId"], "new-id");
            }
        }
        let last: serde_json::Value = serde_json::from_str(&kept[2]).unwrap();
        assert_eq!(last["uuid"], "u2");
    }

    #[test]
    fn fork_truncate_errors_when_anchor_missing() {
        let lines = vec![
            r#"{"type":"user","uuid":"u1","sessionId":"old"}"#.to_string(),
        ];
        assert!(fork_truncate_lines(lines, "nope", "new-id").is_err());
    }

    #[test]
    fn fork_truncate_ignores_uuid_in_dropped_tail() {
        let lines = vec![
            r#"{"type":"user","uuid":"u1","sessionId":"old"}"#.to_string(),
            r#"{"type":"user","uuid":"u1","sessionId":"old"}"#.to_string(),
        ];
        // 第一条 u1 即截断，重复的 uuid 也不会被越过
        let kept = fork_truncate_lines(lines, "u1", "new-id").unwrap();
        assert_eq!(kept.len(), 1);
    }

    #[test]
    fn fork_truncate_workbuddy_function_call_anchor_uses_call_id() {
        // WorkBuddy：一条 assistant message 并发两个工具调用，两行 function_call 共享
        // message 级 id（"msg-1"），各自 callId 唯一。锚点必须是 callId 才不会截断过早。
        let lines = vec![
            r#"{"id":"u0","type":"message","role":"user","content":[{"type":"input_text","text":"hi"}],"sessionId":"old"}"#.to_string(),
            r#"{"id":"msg-1","type":"function_call","name":"Read","arguments":"{}","callId":"call-A","sessionId":"old"}"#.to_string(),
            r#"{"id":"msg-1","type":"function_call","name":"Bash","arguments":"{}","callId":"call-B","sessionId":"old"}"#.to_string(),
            r#"{"id":"res-1","type":"function_call_result","name":"Bash","callId":"call-B","output":{"type":"text","text":"ok"},"sessionId":"old"}"#.to_string(),
        ];

        // 锚点 = 第二个 function_call 的 callId：必须截到第 3 行（含），而不是
        // 第 2 行 —— 否则 fork 出的会话会丢掉 call-B 的调用与结果。
        let kept = fork_truncate_lines(lines.clone(), "call-B", "new-id").unwrap();
        assert_eq!(kept.len(), 3);
        let last: serde_json::Value = serde_json::from_str(&kept[2]).unwrap();
        assert_eq!(last["callId"], "call-B");

        // 共享的 message 级 id 不再是 function_call 的锚（旧链路会在第 2 行误截断）
        let kept_old = fork_truncate_lines(lines.clone(), "msg-1", "new-id");
        assert!(kept_old.is_err(), "msg-1 不应再被 function_call 行匹配");

        // function_call_result 仍按 id 锚定（其 id 行级唯一）
        let kept_res = fork_truncate_lines(lines, "res-1", "new-id").unwrap();
        assert_eq!(kept_res.len(), 4);
    }

    #[test]
    fn resolve_session_id_rejects_subagent_regardless_of_cli() {
        let resolved = resolve_session_id(
            Some("codex".to_string()),
            "/tmp/parent-session/subagents/agent-123.jsonl".to_string(),
        )
        .unwrap();

        assert_eq!(resolved, None);
    }

    #[test]
    fn session_id_matches_are_added_as_search_candidates() {
        let candidates = Vec::new();
        let docs = Vec::new();
        let session_id_matches = vec![app_db::SessionListIndexRecord {
            session_path: "/tmp/session-a.jsonl".to_string(),
            session_id: "abc123-session".to_string(),
            project_path: Some("/tmp/project".to_string()),
            title: None,
            first_user_message: Some("first prompt".to_string()),
            first_timestamp: None,
            last_timestamp: None,
            git_branch: String::new(),
            file_size: 0,
            modified_ms: 0,
            has_archive_snapshot: false,
            is_archived: false,
        }];

        let (merged_candidates, merged_docs) =
            merge_session_id_search_matches("ABC123", candidates, docs, session_id_matches);

        assert_eq!(merged_candidates.len(), 1);
        assert_eq!(merged_candidates[0].session_path, "/tmp/session-a.jsonl");
        assert_eq!(merged_candidates[0].first_match_message_index, Some(0));
        assert_eq!(merged_docs.len(), 1);
        // 文档文本是 id 本身、前缀句交给前端：带前缀的文本会被 `extract_snippet`
        // 切进 snippet 原样上屏（同 `snippet_has_no_field_prefix_...`）
        assert_eq!(merged_docs[0].search_text, "abc123-session");
        assert_eq!(
            merged_docs[0].matched_field,
            Some(session::MatchedField::SessionId)
        );
    }

    #[test]
    fn session_id_match_doc_is_prioritized_over_content_docs() {
        let candidates = vec![app_db::SessionSearchCandidate {
            session_path: "/tmp/session-a.jsonl".to_string(),
            first_match_message_index: Some(8),
            matched_doc_count: 1,
            rank: 0.0,
        }];
        let docs = vec![app_db::SessionSearchMatchedDoc {
            session_path: "/tmp/session-a.jsonl".to_string(),
            message_index: 8,
            search_text: "content also mentions abc123".to_string(),
            matched_field: Some(session::MatchedField::Content),
        }];
        let session_id_matches = vec![app_db::SessionListIndexRecord {
            session_path: "/tmp/session-a.jsonl".to_string(),
            session_id: "abc123-session".to_string(),
            project_path: Some("/tmp/project".to_string()),
            title: None,
            first_user_message: Some("first prompt".to_string()),
            first_timestamp: None,
            last_timestamp: None,
            git_branch: String::new(),
            file_size: 0,
            modified_ms: 0,
            has_archive_snapshot: false,
            is_archived: false,
        }];

        let (merged_candidates, merged_docs) =
            merge_session_id_search_matches("abc123", candidates, docs, session_id_matches);

        assert_eq!(merged_candidates.len(), 1);
        assert_eq!(merged_docs[0].search_text, "abc123-session");
        assert_eq!(merged_docs[1].search_text, "content also mentions abc123");
    }

    #[test]
    fn title_matches_are_added_with_top_priority() {
        let candidates = Vec::new();
        let docs = Vec::new();
        let title_matches = vec![app_db::SessionListIndexRecord {
            session_path: "/tmp/session-title.jsonl".to_string(),
            session_id: "title-session".to_string(),
            project_path: Some("/tmp/project".to_string()),
            title: Some("优化会话加载性能".to_string()),
            first_user_message: None,
            first_timestamp: None,
            last_timestamp: None,
            git_branch: String::new(),
            file_size: 0,
            modified_ms: 0,
            has_archive_snapshot: false,
            is_archived: false,
        }];

        let (merged_candidates, merged_docs) = merge_title_and_id_search_matches(
            "优化会话",
            candidates,
            docs,
            Vec::new(),
            title_matches,
            Vec::new(),
        );

        assert_eq!(merged_candidates.len(), 1);
        assert_eq!(merged_candidates[0].session_path, "/tmp/session-title.jsonl");
        assert_eq!(merged_candidates[0].rank, -2.0);
        assert_eq!(merged_docs.len(), 1);
        // 元数据文档存的是标题**本身**：带 `会话标题: ` 前缀的文本会被
        // `extract_snippet` 切进 snippet 原样上屏（见 `snippet_has_no_field_prefix_...`）
        assert_eq!(merged_docs[0].search_text, "优化会话加载性能");
        assert_eq!(
            merged_docs[0].matched_field,
            Some(session::MatchedField::Title)
        );
    }

    /// 今天没有任何断言盯 `snippet` 的内容，`会话标题: ` 前缀才能一路活到界面上。
    /// 这里把「前缀不上屏」与「切片确实切到了查询词」一起钉住：只断言前者的话，
    /// 把 `search_text` 换成空串也能绿。
    #[test]
    fn snippet_has_no_field_prefix_and_contains_the_query() {
        let title_matches = vec![app_db::SessionListIndexRecord {
            session_path: "/tmp/session-title.jsonl".to_string(),
            session_id: "title-session".to_string(),
            project_path: Some("/tmp/project".to_string()),
            title: Some("优化会话加载性能".to_string()),
            first_user_message: Some("帮我看看登录接口".to_string()),
            first_timestamp: None,
            last_timestamp: None,
            git_branch: String::new(),
            file_size: 0,
            modified_ms: 0,
            has_archive_snapshot: false,
            is_archived: false,
        }];

        // 标题命中
        let (candidates, docs) = merge_title_and_id_search_matches(
            "优化会话",
            Vec::new(),
            Vec::new(),
            Vec::new(),
            title_matches.clone(),
            Vec::new(),
        );
        let aggregated = aggregate_session_searches("优化会话", candidates, docs);
        assert_eq!(aggregated.len(), 1);
        assert_eq!(aggregated[0].snippet, "优化会话加载性能");
        assert!(
            !aggregated[0].snippet.contains("会话标题"),
            "前缀不得进 snippet: {:?}",
            aggregated[0].snippet
        );
        assert!(
            aggregated[0].snippet.contains("优化会话"),
            "snippet 必须含查询词，否则切片坏了: {:?}",
            aggregated[0].snippet
        );
        assert_eq!(
            aggregated[0].matched_field,
            Some(session::MatchedField::Title)
        );

        // 首条消息命中：同一份记录，查询词落在 first_user_message 上
        let (candidates, docs) = merge_title_and_id_search_matches(
            "登录接口",
            Vec::new(),
            Vec::new(),
            Vec::new(),
            title_matches,
            Vec::new(),
        );
        let aggregated = aggregate_session_searches("登录接口", candidates, docs);
        assert_eq!(aggregated.len(), 1);
        assert_eq!(aggregated[0].snippet, "帮我看看登录接口");
        assert!(
            !aggregated[0].snippet.contains("首条消息"),
            "前缀不得进 snippet: {:?}",
            aggregated[0].snippet
        );
        assert!(
            aggregated[0].snippet.contains("登录接口"),
            "snippet 必须含查询词，否则切片坏了: {:?}",
            aggregated[0].snippet
        );
        assert_eq!(
            aggregated[0].matched_field,
            Some(session::MatchedField::FirstMessage)
        );
    }

    /// `title IS NULL` 且查询词落在 session_id 上时，第 2 步的路径/ID 分支会成为 snippet
    /// 来源（它排在第 3 步的会话 ID 文档之前）。那种命中的事实就是会话 ID，必须标成
    /// `SessionId` —— 标成 `None` 前端就只能渲染一个没有说明的裸 id，
    /// 正是 `MatchedField` 的注释里记下的那条合流。
    #[test]
    fn null_title_path_hit_that_surfaces_a_session_id_is_labelled_as_session_id() {
        let title_matches = vec![app_db::SessionListIndexRecord {
            session_path: "/tmp/session-idonly.jsonl".to_string(),
            session_id: "abc123-session".to_string(),
            project_path: Some("/tmp/project".to_string()),
            title: None,
            first_user_message: Some("first prompt".to_string()),
            first_timestamp: None,
            last_timestamp: None,
            git_branch: String::new(),
            file_size: 0,
            modified_ms: 0,
            has_archive_snapshot: false,
            is_archived: false,
        }];

        let (candidates, docs) = merge_title_and_id_search_matches(
            "abc123",
            Vec::new(),
            Vec::new(),
            Vec::new(),
            title_matches,
            Vec::new(),
        );
        let aggregated = aggregate_session_searches("abc123", candidates, docs);

        assert_eq!(aggregated.len(), 1);
        // 文本是 id 本身，没有 `Session ID: ` 前缀
        assert_eq!(aggregated[0].snippet, "abc123-session");
        assert_eq!(
            aggregated[0].matched_field,
            Some(session::MatchedField::SessionId)
        );
    }

    /// 自定义重命名命中同样不许把 `会话标题: ` 前缀带上屏。
    #[test]
    fn custom_name_snippet_has_no_field_prefix() {
        let mut custom_names = HashMap::new();
        custom_names.insert(
            app_db::session_name_key(CliKind::Claude, "/tmp/c1.jsonl"),
            "重命名会议记录".to_string(),
        );
        let custom_name_matches = find_custom_name_matches(CliKind::Claude, &custom_names, "会议");
        assert_eq!(custom_name_matches.len(), 1);

        let (candidates, docs) = merge_title_and_id_search_matches(
            "会议",
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            custom_name_matches,
        );
        let aggregated = aggregate_session_searches("会议", candidates, docs);
        assert_eq!(aggregated.len(), 1);
        assert_eq!(aggregated[0].snippet, "重命名会议记录");
        assert!(
            !aggregated[0].snippet.contains("会话标题"),
            "前缀不得进 snippet: {:?}",
            aggregated[0].snippet
        );
        assert!(aggregated[0].snippet.contains("会议"));
        assert_eq!(
            aggregated[0].matched_field,
            Some(session::MatchedField::Title)
        );
    }

    #[test]
    fn title_match_sorts_ahead_of_content_in_aggregate_search() {
        let content_candidate = app_db::SessionSearchCandidate {
            session_path: "/tmp/content.jsonl".to_string(),
            first_match_message_index: Some(5),
            matched_doc_count: 5,
            rank: 10.0,
        };
        let title_candidate = app_db::SessionSearchCandidate {
            session_path: "/tmp/title.jsonl".to_string(),
            first_match_message_index: Some(0),
            matched_doc_count: 1,
            rank: -2.0,
        };

        let content_doc = app_db::SessionSearchMatchedDoc {
            session_path: "/tmp/content.jsonl".to_string(),
            message_index: 5,
            search_text: "搜索关键词 搜索关键词 搜索关键词 搜索关键词 搜索关键词".to_string(),
            matched_field: Some(session::MatchedField::Content),
        };
        let title_doc = app_db::SessionSearchMatchedDoc {
            session_path: "/tmp/title.jsonl".to_string(),
            message_index: 0,
            search_text: "包含搜索关键词的标题".to_string(),
            matched_field: Some(session::MatchedField::Title),
        };

        let aggregated = aggregate_session_searches(
            "搜索关键词",
            vec![content_candidate, title_candidate],
            vec![content_doc, title_doc],
        );

        assert_eq!(aggregated.len(), 2);
        // 标题匹配优先排序在首位
        assert_eq!(aggregated[0].session_path, "/tmp/title.jsonl");
        assert_eq!(aggregated[1].session_path, "/tmp/content.jsonl");
    }

    #[test]
    fn custom_name_matches_are_found_and_merged() {
        let mut custom_names = HashMap::new();
        // 必须用生产真实写入的键形态（app_db::session_name_key 产出的复合键）：
        // 种一个生产从不写入的格式，会让读写两端分隔符不一致的缺陷被测试掩盖
        custom_names.insert(
            app_db::session_name_key(CliKind::Claude, "/path/c1.jsonl"),
            "重命名会议记录".to_string(),
        );
        custom_names.insert(
            app_db::session_name_key(CliKind::Codex, "/path/c2.jsonl"),
            "其他记录".to_string(),
        );
        let matches = find_custom_name_matches(CliKind::Claude, &custom_names, "会议记录");
        // 只有 Claude 的复合键能命中：Codex 的键 cli_id 不匹配，
        // 且不存在任何非复合形态的键
        assert_eq!(matches.len(), 1, "只有本 CLI 的复合键应命中: {matches:?}");
        assert_eq!(
            matches,
            vec![("/path/c1.jsonl".to_string(), "重命名会议记录".to_string())]
        );

        // 复合键不得被整串（含 NUL）当成路径：还原出的路径必须是干净的会话路径
        assert!(
            !matches.iter().any(|(path, _)| path.contains('\u{0}')),
            "还原出的路径不得残留分隔符: {matches:?}"
        );
    }

    /// 其它 CLI 的复合键不该被当前 CLI 命中，避免跨来源串味。
    #[test]
    fn custom_name_matches_skip_other_cli_composite_keys() {
        let mut custom_names = HashMap::new();
        custom_names.insert(
            app_db::session_name_key(CliKind::Codex, "/path/c2.jsonl"),
            "会议记录".to_string(),
        );
        // 历史脏键（裸 CLI 名，不是路径）不应被当成会话路径呈现
        custom_names.insert("claude".to_string(), "会议记录脏键".to_string());

        let matches = find_custom_name_matches(CliKind::Claude, &custom_names, "会议记录");
        assert!(matches.is_empty(), "跨 CLI 复合键与裸 CLI 名都不应命中: {matches:?}");
    }
}

#[cfg(test)]
mod cross_cli_tests {
    use crate::cli::CliKind;

    fn empty_result() -> crate::session::SearchResult {
        crate::session::SearchResult {
            session_id: "s1".into(),
            file_path: "/tmp/a.jsonl".into(),
            display_name: "t".into(),
            project_path: Some("/p".into()),
            snippet: String::new(),
            matched_field: None,
            match_count: 1,
            first_match_message_index: None,
            cli_id: String::new(),
        }
    }

    // 注:不对 run_search_sessions_pipeline 做端到端断言 —— 它依赖全局
    // LazyLock DB 连接池(指向真实用户库,无测试替身)与 tantivy 物理索引,
    // 播种成本过高且有污染真实数据的风险。盖戳逻辑抽为 stamp_results_cli_id，
    // CLI 子集解析与故障隔离在 session_index 的纯函数测试中覆盖。

    #[test]
    fn stamp_results_cli_id_stamps_kind_id_on_every_result() {
        let mut results = vec![empty_result(), empty_result()];
        super::stamp_results_cli_id(&mut results, CliKind::WorkBuddy);
        assert!(results.iter().all(|r| r.cli_id == "workbuddy"));

        super::stamp_results_cli_id(&mut results, CliKind::Claude);
        assert!(results.iter().all(|r| r.cli_id == "claude"));
    }

    #[test]
    fn all_search_disposition_three_states() {
        use super::{all_search_disposition, AllSearchDisposition};
        // 无数据目录 → Skip
        assert_eq!(
            all_search_disposition(false, true, 0, 10),
            AllSearchDisposition::Skip
        );
        // 物理索引缺失:有会话 → Pending;无会话 → Skip
        assert_eq!(
            all_search_disposition(true, false, 0, 10),
            AllSearchDisposition::Pending
        );
        assert_eq!(
            all_search_disposition(true, false, 0, 0),
            AllSearchDisposition::Skip
        );
        // 从未索引过任何会话(missing == total)→ Pending(尚未建立)
        assert_eq!(
            all_search_disposition(true, true, 26, 26),
            AllSearchDisposition::Pending
        );
        // 已建索引但有增量(missing < total)→ 照常搜索 + stale
        assert_eq!(
            all_search_disposition(true, true, 3, 381),
            AllSearchDisposition::Search { stale: true }
        );
        // 完全新鲜 → Search 且不 stale
        assert_eq!(
            all_search_disposition(true, true, 0, 381),
            AllSearchDisposition::Search { stale: false }
        );
        // 没有任何会话的 CLI:不为它提示构建
        assert_eq!(
            all_search_disposition(true, true, 0, 0),
            AllSearchDisposition::Search { stale: false }
        );
    }

    #[test]
    fn test_fork_truncate_lines_claude_uuid() {
        let lines = vec![
            r#"{"uuid":"u1","sessionId":"old-s","text":"hello"}"#.to_string(),
            r#"{"uuid":"u2","sessionId":"old-s","text":"target"}"#.to_string(),
            r#"{"uuid":"u3","sessionId":"old-s","text":"after"}"#.to_string(),
        ];
        let res = super::fork_truncate_lines(lines, "u2", "new-s").unwrap();
        assert_eq!(res.len(), 2);
        assert!(res[0].contains(r#""sessionId":"new-s""#));
        assert!(res[1].contains(r#""sessionId":"new-s""#));
        assert!(res[1].contains(r#""uuid":"u2""#));
    }

    #[test]
    fn test_fork_truncate_lines_workbuddy_id() {
        let lines = vec![
            r#"{"id":"wb-1","sessionId":"old-wb","type":"message","role":"user"}"#.to_string(),
            r#"{"id":"wb-2","sessionId":"old-wb","type":"message","role":"assistant"}"#.to_string(),
            r#"{"id":"wb-3","sessionId":"old-wb","type":"message","role":"user"}"#.to_string(),
        ];
        let res = super::fork_truncate_lines(lines, "wb-2", "new-wb").unwrap();
        assert_eq!(res.len(), 2);
        assert!(res[0].contains(r#""sessionId":"new-wb""#));
        assert!(res[1].contains(r#""sessionId":"new-wb""#));
        assert!(res[1].contains(r#""id":"wb-2""#));
    }

    #[test]
    fn test_sync_workbuddy_forked_session_to_db() {
        use rusqlite::Connection;
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let db_path = std::env::temp_dir().join(format!("seshbuddy-wb-fork-{unique}.db"));
        let conn = Connection::open(&db_path).unwrap();
        conn.execute(
            r#"
            CREATE TABLE sessions (
                id TEXT PRIMARY KEY,
                cwd TEXT NOT NULL,
                user_id TEXT NOT NULL,
                title TEXT,
                custom_title TEXT,
                status TEXT NOT NULL DEFAULT 'Pending',
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                last_activity_at INTEGER,
                deleted_at INTEGER,
                is_playground INTEGER NOT NULL DEFAULT 0,
                source_mode TEXT,
                is_background_automation INTEGER,
                mode TEXT,
                model TEXT,
                expert_id TEXT,
                expert_locale TEXT,
                expert_runtime_identity TEXT,
                expert_marketplace TEXT,
                permission_mode TEXT,
                use_sandbox_cli INTEGER,
                project_id TEXT,
                plugin_context_json TEXT,
                last_user_prompt_expert_selection TEXT
            );
            "#,
            [],
        ).unwrap();

        conn.execute(
            r#"
            INSERT INTO sessions (id, cwd, user_id, title, status, created_at, updated_at)
            VALUES ('orig-wb', '/proj', 'user-1', '测试会话', 'completed', 1000, 1000);
            "#,
            [],
        ).unwrap();
        drop(conn);

        super::sync_workbuddy_forked_session_to_db(&db_path, "orig-wb", "forked-wb").unwrap();

        let conn = Connection::open(&db_path).unwrap();
        let mut stmt = conn.prepare("SELECT id, cwd, user_id, title, status FROM sessions WHERE id = 'forked-wb'").unwrap();
        let mut rows = stmt.query([]).unwrap();
        let row = rows.next().unwrap().unwrap();
        let id: String = row.get(0).unwrap();
        let cwd: String = row.get(1).unwrap();
        let user_id: String = row.get(2).unwrap();
        let title: String = row.get(3).unwrap();
        let status: String = row.get(4).unwrap();

        assert_eq!(id, "forked-wb");
        assert_eq!(cwd, "/proj");
        assert_eq!(user_id, "user-1");
        assert_eq!(title, "测试会话 (分支)");
        assert_eq!(status, "completed");

        let _ = std::fs::remove_file(&db_path);
    }
}
