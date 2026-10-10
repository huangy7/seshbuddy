//! 多 CLI 会话解析。按 CLI 拆分到独立模块，本模块负责种类判定与薄分派。

pub(crate) mod antigravity;
pub(crate) mod batch;
pub(crate) mod claude;
pub(crate) mod claude_entry;
pub(crate) mod codex;
pub(crate) mod cursor;
pub(crate) mod dsh;
pub(crate) mod gemini;
#[cfg(test)]
mod parse_bench;
pub(crate) mod pi;
pub(crate) mod shared;
pub(crate) mod workbuddy;

use crate::cli_registry::{kind_for_path, source_for};
use crate::error::AppResult;
use crate::session::{self, ChatMessage, SessionLoadResult, UsageRecord};
use std::path::Path;

pub use crate::parser::shared::{SearchDocument, SearchScanProgress};
pub(crate) use codex::load_codex_index_titles;
pub(crate) use workbuddy::load_workbuddy_custom_titles;
pub use crate::session::SessionListMetadata;

pub(crate) fn parse_session_file(file_path: &str) -> AppResult<Vec<ChatMessage>> {
    source_for(kind_for_path(file_path)).parse(file_path)
}

pub(crate) fn parse_session_file_streaming<F>(
    file_path: &str,
    skip_sidechain: bool,
    on_batch: F,
) -> AppResult<(
    u64,
    std::collections::HashMap<String, session::SubagentInfo>,
)>
where
    F: FnMut(Vec<session::ChatMessage>) -> bool,
{
    let mut cb = on_batch;
    source_for(kind_for_path(file_path)).parse_streaming(file_path, skip_sidechain, &mut cb)
}

pub(crate) fn parse_session_incremental(
    file_path: &str,
    offset: u64,
    skip_sidechain: bool,
) -> AppResult<SessionLoadResult> {
    source_for(kind_for_path(file_path)).parse_incremental(file_path, offset, skip_sidechain)
}

/// 从内存内容按会话类型解析消息（归档兜底用）。
/// 与源文件路径解析保持一致：按 kind_for_path 分派对应 CLI 的解析器，
/// 避免归档兜底误用通用 Claude 解析器导致 Codex/Gemini/WorkBuddy/Antigravity 归档解析出 0 条。
pub(crate) fn parse_session_content_by_kind(
    file_path: &str,
    content: &str,
) -> Vec<session::ChatMessage> {
    source_for(kind_for_path(file_path)).parse_from_content(content)
}

pub(crate) fn read_first_user_message(file_path: &str) -> Option<String> {
    source_for(kind_for_path(file_path)).first_user_message(file_path)
}

pub(crate) fn read_session_id(file_path: &str) -> Option<String> {
    source_for(kind_for_path(file_path)).session_id(file_path)
}

/// 解析会话所属的项目路径。
///
/// 出口处收口到同一条不变量：**空白不是路径**（缺席是 `None`，不是空串）。
/// 逐分支各写一次过滤会让某一个分支漏掉 —— `codex` 的 `session_meta` 分支就这么漏过一次。
pub(crate) fn read_project_path(file_path: &str) -> Option<String> {
    source_for(kind_for_path(file_path))
        .project_path_raw(file_path)
        .filter(|value| !value.trim().is_empty())
}

pub(crate) fn extract_usage_records(file_path: &str, project: &str) -> Vec<UsageRecord> {
    let kind = kind_for_path(file_path);
    match source_for(kind).usage_stats() {
        Some(feature) => feature.usage_records(file_path, project),
        // 没有该能力 = 没有用量数据。改动前这里拿到的是空表，行为相同。
        None => Vec::new(),
    }
}

pub(crate) fn is_subagent_session(file_path: &str) -> bool {
    let path = Path::new(file_path);
    if path
        .parent()
        .and_then(|parent| parent.file_name())
        .and_then(|name| name.to_str())
        == Some("subagents")
    {
        return true;
    }
    source_for(kind_for_path(file_path)).is_subagent(file_path)
}

pub(crate) fn scan_session_search_docs_with_progress<F>(
    file_path: &Path,
    on_progress: F,
) -> Option<Vec<SearchDocument>>
where
    F: FnMut(SearchScanProgress),
{
    let key = file_path.to_string_lossy();
    let mut cb = on_progress;
    source_for(kind_for_path(&key)).search_docs(&key, &mut cb)
}

pub(crate) fn scan_session_search_docs_from_bytes(
    file_path: &str,
    content: &[u8],
) -> Option<Vec<SearchDocument>> {
    source_for(kind_for_path(file_path)).search_docs_from_bytes(content)
}

#[cfg(test)]
mod result_string_class_error_shape {
    use super::*;
    use crate::error::AppError;

    /// 六个 CLI 各一个路径，**按 `kind_for_path` 的路由规则**落进对应分支。
    fn per_kind_paths(base: &str) -> Vec<(&'static str, String)> {
        vec![
            ("claude", format!("{base}/.claude/projects/p/gone.jsonl")),
            ("codex", format!("{base}/.codex/sessions/gone.jsonl")),
            ("dsh", format!("{base}/.dsh/sessions/gone.jsonl")),
            ("gemini", format!("{base}/.gemini/tmp/gone.jsonl")),
            ("workbuddy", format!("{base}/.workbuddy/projects/p/gone.jsonl")),
            (
                "antigravity",
                format!("{base}/.gemini/antigravity-cli/brain/s1/transcript.jsonl"),
            ),
        ]
    }

    /// 断言错误是「带非空 `detail` 的 `internal.io`」。
    /// 退回 `Result<_, String>` 时这里是 `Business("No such file or directory (os error 2)")`，直接红。
    fn assert_io_coded(err: &AppError, kind: &str, entry: &str) {
        match err {
            AppError::Coded { code, params } => {
                assert_eq!(*code, "internal.io", "{kind} 的 {entry} 码不对");
                assert!(
                    params.get("detail").is_some_and(|d| !d.is_empty()),
                    "{kind} 的 {entry} 必须带非空 params.detail"
                );
            }
            other => panic!("{kind} 的 {entry} 退回了非结构化错误：{other:?}"),
        }
    }

    /// ★ 行为级回归守卫：本批把 35 个 `Result<_, String>` 签名加宽成 `AppResult`
    /// （六个 CLI 的解析入口经三个分派器覆盖）。
    ///
    /// **为什么必须有**：**实测（计划 11 的 T5 之后）**——把任一签名改回 `Result<_, String>`，
    /// `cargo check` 已经会红（`?` 转不过去：`claude::parse_session_file` 改回去报
    /// 3 × `E0277` + 分派器 `parser/mod.rs:56` 的 `E0308`）。**但这条守卫钉的是另一件事**：
    /// **函数体**回退（保留 `AppResult` 签名、把错误重新压成裸字符串或 `Business`）编译器
    /// 看不见，闸门规则 2 只认 CJK 码点也看不见，而用户会重新拿到裸 OS 英文
    /// （`删除失败：No such file or directory (os error 2)` 那一类）。
    /// 钉**行为**而不是钉类型：签名不是调用方，fn 指针钉不住实现侧的回退，
    /// 但线格式变了这条测试就会红。
    ///
    /// **两个方向都测**：同一批路径先建出空文件断言 `Ok`（阳性对照——否则「恒返回 `Err`」
    /// 与「真的只有 I/O 失败才 `Err`」在输出上无法区分），再删掉文件断言 `Coded`。
    #[test]
    fn all_cli_entry_points_report_io_failures_as_coded() {
        let base = std::env::temp_dir().join(format!(
            "seshbuddy-absent-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let base = base.to_string_lossy().to_string();

        for (kind, path) in per_kind_paths(&base) {
            if let Some(parent) = std::path::Path::new(&path).parent() {
                std::fs::create_dir_all(parent).expect("建夹具目录");
            }
            std::fs::write(&path, "").expect("建空夹具文件");

            // 阳性对照：文件在时三个入口都成功。
            assert!(parse_session_file(&path).is_ok(), "{kind} 空文件应当解析成功");
            assert!(
                parse_session_incremental(&path, 0, false).is_ok(),
                "{kind} 空文件应当增量解析成功"
            );
            assert!(
                parse_session_file_streaming(&path, false, |_| true).is_ok(),
                "{kind} 空文件应当流式解析成功"
            );

            std::fs::remove_file(&path).expect("删夹具文件");

            assert_io_coded(
                &parse_session_file(&path).expect_err("文件已删，必须失败"),
                kind,
                "parse_session_file",
            );
            assert_io_coded(
                &parse_session_incremental(&path, 0, false).expect_err("文件已删，必须失败"),
                kind,
                "parse_session_incremental",
            );
            assert_io_coded(
                &parse_session_file_streaming(&path, false, |_| true)
                    .expect_err("文件已删，必须失败"),
                kind,
                "parse_session_file_streaming",
            );
        }

        let _ = std::fs::remove_dir_all(&base);
    }

    /// 类型级补齐：`parse_session_from_string` 的错误槽是**空置的**（函数体从不返回 `Err`），
    /// 所以没有「错误形状」可断言——但它的签名仍应留在 `AppResult` 上：
    /// 调用方 `parse_session_content_by_kind` 用 `.unwrap_or_default()` 消费它，
    /// 回退成 `Result<_, String>` **照常编译**。钉类型是这里唯一能做的事。
    #[test]
    fn from_string_entry_stays_app_result() {
        let _: fn(&str) -> AppResult<Vec<ChatMessage>> = claude::parse_session_from_string;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn search_doc_scan_reports_file_byte_progress() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("seshbuddy-search-progress-{unique}.jsonl"));
        let content = concat!(
            "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"first\"}}\n",
            "{\"type\":\"assistant\",\"message\":{\"role\":\"assistant\",\"content\":\"second\"}}\n"
        );
        fs::write(&path, content).unwrap();

        let mut progress = Vec::new();
        let docs = scan_session_search_docs_with_progress(&path, |p| progress.push(p)).unwrap();
        let _ = fs::remove_file(&path);

        assert_eq!(docs.len(), 2);
        assert!(!progress.is_empty());
        assert_eq!(progress.last().unwrap().bytes_read, content.len() as u64);
        assert_eq!(progress.last().unwrap().file_size, content.len() as u64);
    }

    #[test]
    fn search_document_carries_role_from_extractor() {
        // 用 scan_session_search_docs_from_bytes 走真实提取路径
        let content = concat!(
            "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"你好\"}}\n",
            "{\"type\":\"assistant\",\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"我在\"}]}}\n"
        );
        let docs = scan_session_search_docs_from_bytes(
            "/tmp/claude-session.jsonl",
            content.as_bytes(),
        )
        .unwrap();
        assert_eq!(docs.len(), 2);
        assert_eq!(docs[0].role, "user");
        assert_eq!(docs[1].role, "assistant");
    }

    #[test]
    fn search_doc_scan_from_bytes_indexes_archived_content() {
        let content = concat!(
            "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"archived needle\"}}\n",
            "{\"type\":\"assistant\",\"message\":{\"role\":\"assistant\",\"content\":\"archived answer\"}}\n"
        );

        let docs =
            scan_session_search_docs_from_bytes("/tmp/claude-session.jsonl", content.as_bytes())
                .expect("archived content should produce search docs");

        assert_eq!(docs.len(), 2);
        assert!(docs[0].search_text.contains("archived needle"));
        assert!(docs[1].search_text.contains("archived answer"));
    }
}

#[cfg(test)]
mod archive_dispatch_tests {
    use super::*;

    #[test]
    fn test_archive_content_parses_by_kind() {
        // Codex 归档：通用 Claude 解析器会解析出 0 条，分派后应解析出 1 条
        let codex = r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"hello"}]}}"#;
        let n = parse_session_content_by_kind("/Users/x/.codex/sessions/a.jsonl", codex);
        assert_eq!(n.len(), 1, "codex 归档应解析出 1 条");
        assert_eq!(n[0].role, "user");

        // WorkBuddy 归档
        let wb = r#"{"type":"message","role":"user","content":[{"type":"input_text","text":"hello"}]}"#;
        let n = parse_session_content_by_kind("/Users/x/.workbuddy/projects/p/a.jsonl", wb);
        assert_eq!(n.len(), 1, "workbuddy 归档应解析出 1 条");

        // Claude 归档保持原有行为
        let cl = r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"hi"}]}}"#;
        let n = parse_session_content_by_kind("/Users/x/.claude/projects/p/a.jsonl", cl);
        assert_eq!(n.len(), 1, "claude 归档应解析出 1 条");
    }
}

#[cfg(test)]
mod dispatch_tests {
    use super::*;

    fn claude_line() -> &'static str {
        r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"你好"}]}}"#
    }
    fn codex_line() -> &'static str {
        r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"hi"}]}}"#
    }
    fn gemini_line() -> &'static str {
        r#"{"type":"user","content":[{"text":"hello","role":"user"}]}"#
    }
    fn workbuddy_line() -> &'static str {
        r#"{"type":"message","role":"user","content":[{"type":"input_text","text":"hello"}]}"#
    }
    fn antigravity_line() -> &'static str {
        r#"{"step_index":1,"source":"USER_EXPLICIT","type":"USER_INPUT","status":"DONE","created_at":"2026-08-30T12:00:00Z","content":"<USER_REQUEST>hello agy</USER_REQUEST>"}"#
    }

    #[test]
    fn parse_content_by_kind_parses_each_format() {
        assert_eq!(parse_session_content_by_kind("/Users/x/.claude/projects/p/a.jsonl", claude_line()).len(), 1);
        assert_eq!(parse_session_content_by_kind("/Users/x/.codex/sessions/a.jsonl", codex_line()).len(), 1);
        assert_eq!(parse_session_content_by_kind("/Users/x/.gemini/tmp/a.jsonl", gemini_line()).len(), 1);
        assert_eq!(parse_session_content_by_kind("/Users/x/.workbuddy/projects/p/a.jsonl", workbuddy_line()).len(), 1);
        assert_eq!(parse_session_content_by_kind("/Users/x/.gemini/antigravity-cli/brain/s1/transcript.jsonl", antigravity_line()).len(), 1);
    }

    #[test]
    fn dsh_dispatch_arms_route_to_real_implementations() {
        let dir = tempfile::tempdir().unwrap();
        let session_dir = dir
            .path()
            .join(".dsh")
            .join("sessions")
            .join("--Users-x-proj--")
            .join("sess-1");
        std::fs::create_dir_all(&session_dir).unwrap();
        let path = session_dir.join("session.jsonl");
        let content = concat!(
            "{\"type\":\"session\",\"version\":0,\"id\":\"sess-1\",\"createdAt\":1700000000000,\"cwd\":\"/Users/x/proj\"}\n",
            "{\"type\":\"user/message\",\"seq\":1,\"time\":1700000000000,\"data\":{\"content\":[{\"type\":\"text\",\"text\":\"hello dsh\"}]}}\n",
            "{\"type\":\"assistant/message\",\"seq\":2,\"time\":1700000000001,\"data\":{\"content\":[{\"type\":\"text\",\"text\":\"hi there\"}]}}\n",
        );
        std::fs::write(&path, content).unwrap();
        let file_path = path.to_str().unwrap();

        assert_eq!(read_session_id(file_path).as_deref(), Some("sess-1"));
        assert_eq!(read_project_path(file_path).as_deref(), Some("/Users/x/proj"));
        assert_eq!(read_first_user_message(file_path).as_deref(), Some("hello dsh"));

        let docs = scan_session_search_docs_from_bytes(file_path, content.as_bytes()).unwrap();
        assert_eq!(docs.len(), 2);
        assert!(docs[0].search_text.contains("hello dsh"));

        let metadata =
            source_for(kind_for_path(file_path)).metadata_only(file_path).expect("metadata");
        assert_eq!(metadata.session_id, "sess-1");
        assert_eq!(metadata.project_path.as_deref(), Some("/Users/x/proj"));
    }

    #[test]
    fn antigravity_dispatch_arms_route_to_real_implementations() {
        let dir = tempfile::tempdir().unwrap();
        let session_dir = dir
            .path()
            .join(".gemini")
            .join("antigravity-cli")
            .join("brain")
            .join("agy-sess-1")
            .join(".system_generated")
            .join("logs");
        std::fs::create_dir_all(&session_dir).unwrap();
        let path = session_dir.join("transcript.jsonl");
        let content = concat!(
            "{\"step_index\":1,\"source\":\"USER_EXPLICIT\",\"type\":\"USER_INPUT\",\"status\":\"DONE\",\"created_at\":\"2026-08-30T12:00:00Z\",\"content\":\"<USER_REQUEST>hello antigravity</USER_REQUEST>\"}\n",
            "{\"step_index\":2,\"source\":\"MODEL\",\"type\":\"PLANNER_RESPONSE\",\"status\":\"DONE\",\"created_at\":\"2026-08-30T12:00:01Z\",\"content\":\"hi from agy\"}\n",
        );
        std::fs::write(&path, content).unwrap();
        let file_path = path.to_str().unwrap();

        assert_eq!(read_session_id(file_path).as_deref(), Some("agy-sess-1"));
        assert_eq!(read_first_user_message(file_path).as_deref(), Some("hello antigravity"));

        let docs = scan_session_search_docs_from_bytes(file_path, content.as_bytes()).unwrap();
        assert_eq!(docs.len(), 2);
        assert!(docs[0].search_text.contains("hello antigravity"));

        let metadata =
            source_for(kind_for_path(file_path)).metadata_only(file_path).expect("metadata");
        assert_eq!(metadata.session_id, "agy-sess-1");
    }

    /// 项目路径的出口不变量：**空白不是路径**。
    ///
    /// 缺席必须回 `None`，不能回空串或全空白串 —— `session_list_index.project_path` 的缺席
    /// 语义是 `NULL`，落库侧与读取侧都只按「非空」判缺席，空串会装成「有值」混进真路径。
    /// 三个分支（`session_meta.cwd` / `turn_context.cwd` / 各 CLI 自己的解析）都在出口收口。
    #[test]
    fn read_project_path_treats_blank_cwd_as_absent() {
        let dir = tempfile::tempdir().unwrap();
        let sessions = dir.path().join(".codex").join("sessions");
        std::fs::create_dir_all(&sessions).unwrap();

        let blank = sessions.join("rollout-blank.jsonl");
        std::fs::write(
            &blank,
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"s-blank\",\"cwd\":\"   \"}}\n",
        )
        .unwrap();
        assert_eq!(read_project_path(blank.to_str().unwrap()), None);

        let absent = sessions.join("rollout-absent.jsonl");
        std::fs::write(
            &absent,
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"s-absent\"}}\n",
        )
        .unwrap();
        assert_eq!(read_project_path(absent.to_str().unwrap()), None);

        // 阳性对照：同一形态的夹具，cwd 非空时必须真的解析出来 ——
        // 否则上面两条 `None` 与「函数恒返回 None」在输出上无法区分。
        let present = sessions.join("rollout-present.jsonl");
        std::fs::write(
            &present,
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"s-present\",\"cwd\":\"/Users/x/proj\"}}\n",
        )
        .unwrap();
        assert_eq!(
            read_project_path(present.to_str().unwrap()).as_deref(),
            Some("/Users/x/proj")
        );
    }
}
