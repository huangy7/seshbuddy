use crate::error::{AppError, AppResult};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantConversation {
    pub id: String,
    pub claude_session_id: String,
    pub profile: String,
    pub title: String,
    pub created_at: String,
    pub updated_at: String,
}

/// 从首条 prompt 生成会话标题：trim 后按字符截取前 50 字
pub fn title_from_prompt(prompt: &str) -> String {
    prompt.trim().chars().take(50).collect()
}

fn now() -> String {
    crate::db::now_rfc3339()
}

fn row_to_conversation(row: &rusqlite::Row<'_>) -> rusqlite::Result<AssistantConversation> {
    Ok(AssistantConversation {
        id: row.get(0)?,
        claude_session_id: row.get(1)?,
        profile: row.get(2)?,
        title: row.get(3)?,
        created_at: row.get(4)?,
        updated_at: row.get(5)?,
    })
}

pub fn create_conversation_with(
    conn: &Connection,
    claude_session_id: &str,
    profile: &str,
    title: &str,
) -> AppResult<String> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = now();
    conn.execute(
        "INSERT INTO assistant_conversations (id, claude_session_id, profile, title, created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![id, claude_session_id, profile, title, now, now],
    )?;
    Ok(id)
}

pub fn touch_conversation_with(
    conn: &Connection,
    id: &str,
    claude_session_id: &str,
) -> AppResult<()> {
    let affected = conn.execute(
        "UPDATE assistant_conversations SET updated_at = ?1, claude_session_id = ?2 WHERE id = ?3",
        params![now(), claude_session_id, id],
    )?;
    if affected == 0 {
        return Err(AppError::coded("assistant.conversation_missing"));
    }
    Ok(())
}

pub fn list_conversations_with(conn: &Connection) -> AppResult<Vec<AssistantConversation>> {
    let mut stmt = conn.prepare(
        "SELECT id, claude_session_id, profile, title, created_at, updated_at \
         FROM assistant_conversations ORDER BY updated_at DESC",
    )?;
    let rows = stmt.query_map([], row_to_conversation)?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub fn get_conversation_with(conn: &Connection, id: &str) -> AppResult<Option<AssistantConversation>> {
    use rusqlite::OptionalExtension;
    let row = conn
        .query_row(
            "SELECT id, claude_session_id, profile, title, created_at, updated_at \
             FROM assistant_conversations WHERE id = ?1",
            params![id],
            row_to_conversation,
        )
        .optional()?;
    Ok(row)
}

/// 删除对话记录（仅删元数据；claude 会话 jsonl 保留在磁盘）
pub fn delete_conversation_with(conn: &Connection, id: &str) -> AppResult<()> {
    let affected = conn.execute("DELETE FROM assistant_conversations WHERE id = ?1", params![id])?;
    if affected == 0 {
        return Err(AppError::coded("assistant.conversation_missing"));
    }
    Ok(())
}

// ── 连接池包装 ──
pub fn create_conversation(claude_session_id: &str, profile: &str, title: &str) -> AppResult<String> {
    let conn = crate::db::conn()?;
    create_conversation_with(&conn, claude_session_id, profile, title)
}

pub fn touch_conversation(id: &str, claude_session_id: &str) -> AppResult<()> {
    let conn = crate::db::conn()?;
    touch_conversation_with(&conn, id, claude_session_id)
}

pub fn list_conversations() -> AppResult<Vec<AssistantConversation>> {
    let conn = crate::db::conn()?;
    list_conversations_with(&conn)
}

pub fn get_conversation(id: &str) -> AppResult<Option<AssistantConversation>> {
    let conn = crate::db::conn()?;
    get_conversation_with(&conn, id)
}

pub fn delete_conversation(id: &str) -> AppResult<()> {
    let conn = crate::db::conn()?;
    delete_conversation_with(&conn, id)
}

/// 清理引用孤儿：底层 jsonl 不存在或 claude_session_id 为空 → 删除该引用行。
/// 返回清理行数。session_path_exists 接收完整 jsonl 路径串，供测试注入。
/// 单条路径计算/删除失败只记 warn 并跳过，不拖垮整个加载；连接级错误仍向上传播。
pub fn purge_orphan_conversations_with<F>(conn: &Connection, session_path_exists: F) -> AppResult<usize>
where
    F: Fn(&str) -> bool,
{
    let all = list_conversations_with(conn)?;
    let mut removed = 0;
    for conv in &all {
        let orphan = match assistant_session_path(&conv.claude_session_id) {
            Ok(Some(p)) => !session_path_exists(&p.to_string_lossy()),
            Ok(None) => true,
            Err(e) => {
                tracing::warn!("assistant_session_path 计算失败，跳过会话 {}: {}", conv.id, e.diagnostic());
                continue;
            }
        };
        if orphan {
            if let Err(e) = delete_conversation_with(conn, &conv.id) {
                tracing::warn!("删除孤儿会话引用失败，跳过会话 {}: {}", conv.id, e.diagnostic());
            } else {
                removed += 1;
            }
        }
    }
    Ok(removed)
}

pub fn purge_orphan_conversations() -> AppResult<usize> {
    let conn = crate::db::conn()?;
    purge_orphan_conversations_with(&conn, |p| std::path::Path::new(p).exists())
}

/// 助手会话 jsonl 路径：~/.claude/projects/<workspace-slug>/<claude_session_id>.jsonl
/// （slug 规则与 claude 一致：非 ASCII 字母数字全部转 '-'）。claude_session_id 为空返回 None。
pub fn assistant_session_path(claude_session_id: &str) -> AppResult<Option<std::path::PathBuf>> {
    if claude_session_id.is_empty() {
        return Ok(None);
    }
    let workspace = crate::commands::assistant_dir()?.join("workspace");
    let slug = crate::db::session_archive::escape_claude_project_dir(&workspace.to_string_lossy());
    let home = dirs::home_dir().ok_or_else(|| AppError::coded("cli.home_dir_missing"))?;
    Ok(Some(
        home.join(".claude")
            .join("projects")
            .join(slug)
            .join(format!("{}.jsonl", claude_session_id)),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn test_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::create_base_schema(&conn).unwrap();
        conn
    }

    /// ★ 诊断站点的**行为钉**（`:144`，`delete_conversation_with` 的失败分支）。
    ///
    /// 它钉的是「日志里必须带 `params.detail`，不能只有裸 code」——
    /// `Coded` 的 `Display` 就是 code，故把站点的 `e.diagnostic()` 改回 `e`
    /// **编译得过、闸门绿、全套测试绿**（实测），只有这一条能抓住它。
    ///
    /// 驱动方式：真失败，不 mock —— `BEFORE DELETE` 触发器让 `DELETE` 报
    /// `probe blocked`，该 rusqlite 错误经 `From<rusqlite::Error>` 变成
    /// `internal.database` + `detail`。
    #[test]
    fn orphan_purge_failure_logs_detail_not_bare_code() {
        let conn = test_conn();
        create_conversation_with(&conn, "claude-sess-log-probe", "work", "日志探针对话").unwrap();
        conn.execute_batch(
            "CREATE TRIGGER log_probe_abort BEFORE DELETE ON assistant_conversations \
             BEGIN SELECT RAISE(ABORT, 'probe blocked'); END;",
        )
        .unwrap();

        let logs = crate::error::log_capture::capture(|| {
            let removed = purge_orphan_conversations_with(&conn, |_| false).unwrap();
            assert_eq!(removed, 0, "删除被触发器挡下，故一条都不该算成功");
        });

        let line = logs
            .lines()
            .find(|l| l.contains("删除孤儿会话引用失败"))
            .unwrap_or_else(|| panic!("没捕获到该站点，实际日志：\n{logs}"));
        let detail = line
            .split("internal.database detail=")
            .nth(1)
            .unwrap_or_else(|| panic!("日志里只有裸 code、没有 detail：{line}"));
        assert!(!detail.trim().is_empty(), "detail 不能为空：{line}");
    }

    #[test]
    fn touch_with_nonexistent_id_errors() {
        let conn = test_conn();
        let result = touch_conversation_with(&conn, "不存在的id", "claude-sess-x");
        assert!(result.is_err(), "touch 不存在的对话应报错");
    }

    #[test]
    fn get_returns_none_for_missing_id() {
        let conn = test_conn();
        let result = get_conversation_with(&conn, "不存在的id").unwrap();
        assert!(result.is_none(), "查询不存在的对话应返回 None");
    }

    #[test]
    fn create_then_list_orders_by_updated_desc() {
        let conn = test_conn();
        let a = create_conversation_with(&conn, "claude-sess-a", "work", "第一个对话").unwrap();
        let _b = create_conversation_with(&conn, "claude-sess-b", "work", "第二个对话").unwrap();
        let list = list_conversations_with(&conn).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].title, "第二个对话");
        // now_rfc3339 秒级精度，touch 前 sleep 确保时间戳变化
        std::thread::sleep(std::time::Duration::from_millis(1100));
        touch_conversation_with(&conn, &a, "claude-sess-a").unwrap();
        let list = list_conversations_with(&conn).unwrap();
        assert_eq!(list[0].title, "第一个对话");
    }

    #[test]
    fn delete_removes_row_and_errors_on_missing() {
        let conn = test_conn();
        let id = create_conversation_with(&conn, "claude-sess-a", "work", "待删对话").unwrap();
        assert_eq!(list_conversations_with(&conn).unwrap().len(), 1);
        delete_conversation_with(&conn, &id).unwrap();
        assert!(list_conversations_with(&conn).unwrap().is_empty());
        assert!(delete_conversation_with(&conn, &id).is_err(), "重复删除应报错");
    }

    #[test]
    fn title_from_first_prompt_truncates_at_50_chars() {
        let long = "字".repeat(80);
        assert_eq!(super::title_from_prompt(&long).chars().count(), 50);
        assert_eq!(super::title_from_prompt("短标题"), "短标题");
    }

    #[test]
    fn assistant_session_path_empty_id_returns_none() {
        assert!(super::assistant_session_path("").unwrap().is_none());
    }

    #[test]
    fn assistant_session_path_builds_projects_path() {
        // 不依赖真实文件：只验证路径结构形如 .../.claude/projects/<slug>/<id>.jsonl
        let p = super::assistant_session_path("sess-abc123").unwrap().expect("应返回 Some");
        let s = p.to_string_lossy();
        assert!(s.contains(".claude/projects/"));
        assert!(s.ends_with("sess-abc123.jsonl"));
    }

    #[test]
    fn purge_removes_orphans_and_keeps_existing() {
        let conn = test_conn();
        let keep = super::create_conversation_with(&conn, "sess-keep", "default", "在").unwrap();
        let orphan = super::create_conversation_with(&conn, "sess-gone", "default", "没").unwrap();
        let empty = super::create_conversation_with(&conn, "", "default", "空").unwrap();
        // 谓词接的是完整 jsonl 路径串，用 ends_with 模拟"sess-keep 的文件存在"
        let removed = super::purge_orphan_conversations_with(&conn, |p| p.ends_with("sess-keep.jsonl"));
        assert_eq!(removed.unwrap(), 2);
        let left = super::list_conversations_with(&conn).unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].id, keep);
        assert_ne!(left[0].id, orphan);
        assert_ne!(left[0].id, empty);
    }
}
