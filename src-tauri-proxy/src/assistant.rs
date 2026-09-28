// 子命令在后续 Task 2-5 消费这些函数
use rusqlite::Connection;
use std::path::PathBuf;
use transcript_store::extract::{extract, ExtractedMessage};
use transcript_store::store::{
    self, CacheStatus, ExtractPlan,
};

/// 与 src-tauri db/mod.rs 的 app_data_dir 同一套推导：
/// ~/Library/Application Support/com.seshbuddy.app/
pub fn app_data_dir() -> anyhow::Result<PathBuf> {
    let data = dirs::data_dir().ok_or_else(|| anyhow::anyhow!("无法获取应用数据目录"))?;
    Ok(data.join("com.seshbuddy.app"))
}

/// app.db 在 com.seshbuddy.app/data/ 目录下
pub fn app_db_path() -> anyhow::Result<PathBuf> {
    Ok(app_data_dir()?.join("data").join("app.db"))
}

/// transcript_cache.db 在 assistant/ 子目录下
pub fn cache_db_path() -> anyhow::Result<PathBuf> {
    Ok(app_data_dir()?.join("assistant").join("transcript_cache.db"))
}

/// 从 app.db 反查会话源路径与 cli（session_id 全索引唯一）。
/// 仅「查无此行」返回 Ok(None)（对应 exit 3）；SQL 错误（库损坏/schema 不匹配）向上抛。
fn find_session(
    conn: &Connection,
    session_id: &str,
    cli: Option<&str>,
) -> anyhow::Result<Option<(String, String)>> {
    let (sql, params) = match cli {
        Some(c) => (
            "SELECT session_path, cli_id FROM session_list_index WHERE session_id = ?1 AND cli_id = ?2",
            vec![session_id.to_string(), c.to_string()],
        ),
        None => (
            "SELECT session_path, cli_id FROM session_list_index WHERE session_id = ?1",
            vec![session_id.to_string()],
        ),
    };
    match conn.query_row(&sql, rusqlite::params_from_iter(params.iter()), |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    }) {
        Ok(v) => Ok(Some(v)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// 会话 ID 的解析结果：唯一命中、命中多个（歧义）或未命中。
///
/// 调用方是 LLM：它常只给出 ID 前几位，也可能拼错甚至编造。因此解析按「精确 → 前缀 → 分段」
/// 逐级放宽，并把歧义原样回报，让模型自己收窄或改问，而不是替它猜一个。
pub enum Resolution {
    Unique(String),
    Ambiguous(Vec<String>),
    NotFound,
}

fn like_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")
}

fn query_ids(conn: &Connection, where_clause: &str, params: Vec<String>) -> anyhow::Result<Vec<String>> {
    let sql = format!("SELECT session_id FROM session_list_index WHERE {}", where_clause);
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(params.iter()), |r| r.get(0))?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// 解析用户/agent 给的 session id：精确 → 唯一前缀 → 首段（防 LLM 复述长 UUID 时脑补后半段）。
/// 多候选返回 Ambiguous（调用方 exit 2 并列候选）。
pub fn resolve_session_id(conn: &Connection, input: &str, cli: Option<&str>) -> anyhow::Result<Resolution> {
    let cli_filter = cli.map(|c| format!(" AND cli_id = '{}'", c.replace('\'', "''"))).unwrap_or_default();

    // 1. 精确匹配
    let exact = query_ids(
        conn,
        &format!("session_id = ?1{}", cli_filter),
        vec![input.to_string()],
    )?;
    if exact.len() == 1 {
        return Ok(Resolution::Unique(exact.into_iter().next().unwrap()));
    }

    // 2. 前缀匹配（agent 给前 8 位的常见情形）
    let prefix = query_ids(
        conn,
        &format!("session_id LIKE ?1 ESCAPE '\\'{}", cli_filter),
        vec![format!("{}%", like_escape(input))],
    )?;
    match prefix.len() {
        1 => return Ok(Resolution::Unique(prefix.into_iter().next().unwrap())),
        n if n > 1 => return Ok(Resolution::Ambiguous(prefix)),
        _ => {}
    }

    // 3. 首段匹配（agent 记住首段但脑补后半段 UUID 的幻觉情形）：
    //    "922d7ecb-6780-..." 的首段 "922d7ecb" 命中 "922d7ecb-6fb6-..."
    if input.contains('-') {
        let seg = input.split('-').next().unwrap_or("");
        if seg.len() >= 4 {
            let seg_matches = query_ids(
                conn,
                &format!("session_id LIKE ?1 ESCAPE '\\'{}", cli_filter),
                vec![format!("{}-%", like_escape(seg))],
            )?;
            match seg_matches.len() {
                1 => return Ok(Resolution::Unique(seg_matches.into_iter().next().unwrap())),
                n if n > 1 => return Ok(Resolution::Ambiguous(seg_matches)),
                _ => {}
            }
        }
    }

    Ok(Resolution::NotFound)
}

/// 源文件签名：Ok(None) = 文件确已删（NotFound，唯一允许落 gone 终态的情形）；
/// 其他 IO 错误（EACCES/挂载抖动等）向上抛——不写任何状态，下次重试。
fn file_signature(path: &str) -> anyhow::Result<Option<(i64, i64)>> {
    match std::fs::metadata(path) {
        Ok(m) => {
            let mtime_ms = m
                .modified()?
                .duration_since(std::time::UNIX_EPOCH)?
                .as_millis() as i64;
            Ok(Some((mtime_ms, m.len() as i64)))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// 同步按需提取：确保 session 缓存新鲜。返回 false = 会话不在索引（exit 3）。
/// 源文件已删 → gone 终态（不追溯归档）。IO 错误向上抛（不写状态，下次重试）。
pub fn ensure_fresh(session_id: &str, cli: Option<&str>) -> anyhow::Result<bool> {
    let app_conn = Connection::open_with_flags(
        app_db_path()?,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let Some((session_path, cli_id)) = find_session(&app_conn, session_id, cli)? else {
        return Ok(false);
    };
    let cache_conn = store::open(&cache_db_path()?)?;
    let record = store::get_record(&cache_conn, session_id)?;
    let Some((mtime_ms, size)) = file_signature(&session_path)? else {
        // 源已删：gone 终态（幂等）
        if record.as_ref().map(|r| r.status) != Some(CacheStatus::Gone) {
            let mut conn = cache_conn;
            store::mark_status(&mut conn, session_id, &cli_id, CacheStatus::Gone, 0, 0)?;
        }
        return Ok(true);
    };
    match store::decide_plan(record.as_ref(), mtime_ms, size) {
        ExtractPlan::Skip => Ok(true),
        plan => {
            let file = std::fs::File::open(&session_path)?;
            let reader = std::io::BufReader::new(file);
            let (skip, base) = match plan {
                ExtractPlan::Append { skip_raw_lines, base_seq } => (skip_raw_lines, base_seq),
                _ => (0, 0),
            };
            let out = extract(reader, transcript_store::extract::CliFormat::from_cli_id(&cli_id), skip, base);
            write_extraction(cache_conn, session_id, &cli_id, mtime_ms, size, plan, out)?;
            Ok(true)
        }
    }
}

fn write_extraction(
    mut conn: Connection,
    session_id: &str,
    cli: &str,
    mtime_ms: i64,
    size: i64,
    plan: ExtractPlan,
    out: transcript_store::extract::Extraction,
) -> rusqlite::Result<()> {
    let base_count = match plan {
        ExtractPlan::Append { .. } => store::get_record(&conn, session_id)?
            .map(|r| r.message_count)
            .unwrap_or(0),
        _ => 0,
    };
    let total = base_count + out.messages.len() as i64;
    let status = if total == 0 { CacheStatus::Empty } else { CacheStatus::Ok };
    let rec = store::SessionCacheRecord {
        session_id: session_id.to_string(),
        cli: cli.to_string(),
        mtime_ms,
        size,
        raw_lines: out.raw_lines as i64,
        last_seq: out.message_lines as i64,
        status,
        message_count: total,
        extracted_at: 0,
    };
    match plan {
        ExtractPlan::Append { .. } if status == CacheStatus::Ok => {
            store::append_session(&mut conn, &rec, &out.messages)
        }
        _ => store::replace_session(&mut conn, &rec, &out.messages),
    }
}

pub struct ListRow {
    pub id: String,
    pub title: String,
    pub project: String,
    pub cli: String,
    pub updated_at: String,
}

/// 返回 (sql, params)；params 顺序与 SQL 占位符一致
pub fn build_list_query(
    cli: Option<&str>,
    days: Option<u32>,
    since: Option<&str>,
    until: Option<&str>,
    project: Option<&str>,
    limit: u32,
) -> (String, Vec<String>) {
    let mut conds = Vec::new();
    let mut params: Vec<String> = Vec::new();
    if let Some(c) = cli {
        conds.push("cli_id = ?");
        params.push(c.to_string());
    }
    if let Some(d) = days {
        conds.push("datetime(last_timestamp) >= datetime('now', ?)");
        params.push(format!("-{} days", d));
    }
    // 显式日期范围（YYYY-MM-DD，闭区间），与 --days 叠加时取交集
    if let Some(s) = since {
        conds.push("date(last_timestamp) >= date(?)");
        params.push(s.to_string());
    }
    if let Some(u) = until {
        conds.push("date(last_timestamp) <= date(?)");
        params.push(u.to_string());
    }
    if let Some(p) = project {
        conds.push("project_path LIKE ?");
        params.push(format!("%{}%", p));
    }
    let where_clause = if conds.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", conds.join(" AND "))
    };
    let sql = format!(
        "SELECT session_id, COALESCE(NULLIF(title,''), first_user_message, ''), \
         COALESCE(project_path,''), cli_id, COALESCE(last_timestamp,'') \
         FROM session_list_index {} ORDER BY last_timestamp DESC LIMIT {}",
        where_clause, limit
    );
    (sql, params)
}

pub fn run_list(
    cli: Option<&str>,
    days: Option<u32>,
    since: Option<&str>,
    until: Option<&str>,
    project: Option<&str>,
    limit: u32,
) -> anyhow::Result<Vec<ListRow>> {
    let db = app_db_path()?;
    if !db.exists() {
        eprintln!("app-db-not-found: {}", db.display());
        std::process::exit(3);
    }
    let conn =
        Connection::open_with_flags(&db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let (sql, params) = build_list_query(cli, days, since, until, project, limit);
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(params.iter()), |row| {
            Ok(ListRow {
                id: row.get(0)?,
                title: row.get(1)?,
                project: row.get(2)?,
                cli: row.get(3)?,
                updated_at: row.get(4)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// 按 [from, to] 闭区间选消息（None = 不限制），再按 max_chars 截断。
/// 返回 (选中消息, 是否被 max_chars 截断)
pub fn select_range(
    msgs: &[ExtractedMessage],
    from: Option<u64>,
    to: Option<u64>,
    max_chars: usize,
) -> (Vec<&ExtractedMessage>, bool) {
    let mut chars = 0usize;
    let mut out = Vec::new();
    let mut truncated = false;
    for m in msgs {
        if let Some(f) = from {
            if m.seq < f {
                continue;
            }
        }
        if let Some(t) = to {
            if m.seq > t {
                continue;
            }
        }
        let line_len = m.text.len() + 16; // 加行头开销
        if chars + line_len > max_chars && !out.is_empty() {
            truncated = true;
            break;
        }
        chars += line_len;
        out.push(m);
    }
    (out, truncated)
}

/// format: "json"（默认，单 JSON 对象）或 "text"（纯 [#N role] text 行，免后处理）
pub fn run_show(
    cli: Option<&str>,
    session_id: &str,
    from: Option<u64>,
    to: Option<u64>,
    max_chars: usize,
    format: &str,
) -> anyhow::Result<()> {
    // id 解析：前缀/首段都可；歧义 exit 2 列候选，不存在 exit 3
    let app_conn = Connection::open_with_flags(
        app_db_path()?,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let resolved = match resolve_session_id(&app_conn, session_id, cli)? {
        Resolution::Unique(full) => full,
        Resolution::Ambiguous(candidates) => {
            eprintln!("session-ambiguous:{} 候选: {}", session_id, candidates.join(", "));
            std::process::exit(2);
        }
        Resolution::NotFound => {
            eprintln!("session-not-found:{}", session_id);
            std::process::exit(3);
        }
    };
    drop(app_conn);
    if !ensure_fresh(&resolved, cli)? {
        eprintln!("session-not-found:{}", session_id);
        std::process::exit(3);
    }
    let conn = store::open(&cache_db_path()?)?;
    let msgs = store::load_messages(&conn, &resolved)?;
    let (sel, truncated) = select_range(&msgs, from, to, max_chars);
    let text = sel
        .iter()
        .map(|m| format!("[#{} {}] {}", m.seq, m.role, m.text))
        .collect::<Vec<_>>()
        .join("\n");
    if format == "text" {
        println!("{}", text);
        return Ok(());
    }
    let out = serde_json::json!({
        "session": resolved,
        "from": sel.first().map(|m| m.seq),
        "to": sel.last().map(|m| m.seq),
        "truncated": truncated,
        "text": text,
    });
    println!("{}", serde_json::to_string(&out)?);
    Ok(())
}

#[derive(Debug)]
pub struct GrepHit {
    pub session: String,
    pub title: String,
    pub message_index: u64,
    pub role: String,
    pub snippet: String,
}

/// 在单会话消息中搜关键词（子串匹配，大小写不敏感），返回带上下文的命中
pub fn grep_messages(
    session: &str,
    title: &str,
    msgs: &[ExtractedMessage],
    keyword: &str,
    snippet_radius: usize,
) -> Vec<GrepHit> {
    let kw = keyword.to_lowercase();
    let mut hits = Vec::new();
    for m in msgs {
        // 把原文每个 char 起始处的 lowercase 串收集起来，在「lowercase 世界」做匹配。
        // 这样即使 lowercase 改变字符数（如 'İ' -> "i\u{307}"），命中仍能映射回原文下标。
        let orig_chars: Vec<char> = m.text.chars().collect();
        let orig_char_count = orig_chars.len();
        let mut char_lower_starts: Vec<usize> = Vec::with_capacity(orig_char_count + 1);
        let mut lower_full = String::new();
        for ch in &orig_chars {
            char_lower_starts.push(lower_full.len());
            lower_full.extend(ch.to_lowercase());
        }
        char_lower_starts.push(lower_full.len());

        let Some(l_pos) = lower_full.find(&kw) else { continue };
        let l_end = l_pos + kw.len();
        // 首个 lower_start > l_pos 的前一个位置 = 命中起点所在的原文 char
        let start_char = char_lower_starts
            .iter()
            .position(|&s| s > l_pos)
            .unwrap_or(orig_char_count)
            .saturating_sub(1);
        // 首个 lower_start >= l_end 的位置 = 命中终点（exclusive）对应的原文 char
        let end_char = char_lower_starts
            .iter()
            .position(|&s| s >= l_end)
            .unwrap_or(orig_char_count);
        let start = start_char.saturating_sub(snippet_radius);
        let end = (end_char + snippet_radius).min(orig_char_count).max(start);
        let snippet: String = orig_chars[start..end].iter().collect();
        hits.push(GrepHit {
            session: session.to_string(),
            title: title.to_string(),
            message_index: m.seq,
            role: m.role.clone(),
            snippet,
        });
    }
    hits
}

pub fn coverage_label(indexed: usize, total: usize) -> &'static str {
    if total == 0 || indexed >= total { "full" } else { "partial" }
}

/// 从 app.db 统计某 cli 的会话总数（coverage 分母）；库不可用时返回 None
fn count_sessions_in_index(cli: Option<&str>) -> Option<usize> {
    let db = app_db_path().ok()?;
    let conn = Connection::open_with_flags(&db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).ok()?;
    let (sql, params) = match cli {
        Some(c) => ("SELECT COUNT(*) FROM session_list_index WHERE cli_id = ?".to_string(), vec![c.to_string()]),
        None => ("SELECT COUNT(*) FROM session_list_index".to_string(), vec![]),
    };
    conn.query_row(&sql, rusqlite::params_from_iter(params.iter()), |r| r.get(0)).ok()
}

pub fn run_grep(
    keyword: &str,
    cli: Option<&str>,
    days: Option<u32>,
    max_hits: usize,
) -> anyhow::Result<()> {
    let app_conn = Connection::open_with_flags(
        app_db_path()?,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    // days 过滤改用索引的 last_timestamp（比旧版 txt mtime 语义更准）
    let (sql, params) = match (cli, days) {
        (Some(c), Some(d)) => (
            "SELECT session_id, COALESCE(NULLIF(title,''), first_user_message, ''), cli_id \
             FROM session_list_index WHERE cli_id = ?1 \
             AND datetime(last_timestamp) >= datetime('now', ?2)",
            vec![c.to_string(), format!("-{} days", d)],
        ),
        (Some(c), None) => (
            "SELECT session_id, COALESCE(NULLIF(title,''), first_user_message, ''), cli_id \
             FROM session_list_index WHERE cli_id = ?1",
            vec![c.to_string()],
        ),
        (None, Some(d)) => (
            "SELECT session_id, COALESCE(NULLIF(title,''), first_user_message, ''), cli_id \
             FROM session_list_index WHERE datetime(last_timestamp) >= datetime('now', ?1)",
            vec![format!("-{} days", d)],
        ),
        (None, None) => (
            "SELECT session_id, COALESCE(NULLIF(title,''), first_user_message, ''), cli_id \
             FROM session_list_index",
            vec![],
        ),
    };
    let mut stmt = app_conn.prepare(sql)?;
    let sessions: Vec<(String, String, String)> = stmt
        .query_map(rusqlite::params_from_iter(params.iter()), |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(stmt);

    let cache_conn = store::open(&cache_db_path()?)?;
    let cached = store::all_records(&cache_conn)?;
    let mut hits: Vec<GrepHit> = Vec::new();
    let mut indexed = 0usize;
    for (session_id, title, _cli_id) in &sessions {
        let Some(rec) = cached.get(session_id) else { continue };
        if rec.status != CacheStatus::Ok { continue }
        // stale 的现场重提（追加游标命中时代价极低）
        ensure_fresh(session_id, cli)?;
        let msgs = store::load_messages(&cache_conn, session_id)?;
        indexed += 1;
        hits.extend(grep_messages(session_id, title, &msgs, keyword, 100));
        if hits.len() >= max_hits.saturating_mul(4) { break }
    }
    drop(cache_conn);
    hits.truncate(max_hits);
    let total = count_sessions_in_index(cli).unwrap_or(indexed);
    let out = serde_json::json!({
        "hits": hits.iter().map(|h| serde_json::json!({
            "session": h.session, "title": h.title,
            "messageIndex": h.message_index, "role": h.role, "snippet": h.snippet,
        })).collect::<Vec<_>>(),
        "coverage": coverage_label(indexed, total),
        "indexed": indexed,
        "total": total,
    });
    println!("{}", serde_json::to_string(&out)?);
    Ok(())
}

/// 机器可读命令参考：agent 先跑 `--json-help` 拿到全部命令、参数与示例，再据此规划调用，
/// 避免把命令清单硬编码进提示词后与实现漂移
pub fn agent_help_json() -> serde_json::Value {
    serde_json::json!({
        "name": "seshbuddy-proxy",
        "summary": "SeshBuddy 会话助手的会话查询工具",
        "commands": [
            { "usage": "list [--cli claude|codex|gemini] [--days N] [--since YYYY-MM-DD] [--until YYYY-MM-DD] [--project X] [--limit N] [--json]",
              "summary": "列出会话元数据（标题/项目/时间），用于按时间和项目筛选；--since/--until 为闭区间日期过滤",
              "output": "JSON 数组（--json）或 TSV",
              "examples": ["seshbuddy-proxy list --days 7 --json", "seshbuddy-proxy list --since 2026-07-27 --until 2026-07-31 --json"] },
            { "usage": "grep <关键词> [--cli X] [--days N] [--max-hits N]",
              "summary": "全文搜索会话内容，返回命中消息编号和片段",
              "output": "JSON 对象（含 coverage: partial 时表示只搜了部分会话）",
              "examples": ["seshbuddy-proxy grep \"认证失败\" --days 30"] },
            { "usage": "show <id> [--cli X] [--from N] [--to M] [--max-chars N] [--format json|text]",
              "summary": "读取单个会话正文，用 --from/--to 取消息段避免一次读太多；id 给前 8 位即可",
              "output": "JSON 对象；--format text 输出纯 [#N role] 文本行",
              "examples": ["seshbuddy-proxy show abc-123 --from 1 --to 30", "seshbuddy-proxy show 3f8e5341 --format text"] }
        ],
        "exitCodes": {
            "0": "成功",
            "1": "执行错误",
            "2": "参数用法错误或 id 歧义（stderr 会列出候选）",
            "3": "目标不存在（会话不在索引中）"
        },
        "notes": [
            "所有查询命令输出均为单个合法 JSON 值（stdout），日志走 stderr（list 默认输出 TSV，加 --json 才是 JSON）",
            "show 的 id 来自 list 输出的 id 字段；只需给前 8 位前缀即可（全 UUID 反而容易出错），歧义时会返回候选列表",
            "只需要正文时用 show --format text，不要接管道/重定向做后处理",
            "grep 命中后请用 show --from/--to 读取上下文，不要不带范围读整个长会话"
        ]
    })
}

#[cfg(test)]
mod tests {
    fn ids_db() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE session_list_index (session_path TEXT, session_id TEXT, cli_id TEXT);
             INSERT INTO session_list_index VALUES
               ('/p/a','922d7ecb-6fb6-4ebf-bfd5-415b4d5b9469','claude'),
               ('/p/b','3f8e5341-31e2-41cd-a178-1cc7cf7b2522','claude'),
               ('/p/c','aaaaaaaa-1111-0000-0000-000000000001','codex'),
               ('/p/d','aaaaaaaa-2222-0000-0000-000000000002','codex');",
        )
        .unwrap();
        conn
    }

    #[test]
    fn resolve_exact_and_prefix() {
        let conn = ids_db();
        match super::resolve_session_id(&conn, "3f8e5341-31e2-41cd-a178-1cc7cf7b2522", None).unwrap() {
            super::Resolution::Unique(id) => assert!(id.starts_with("3f8e5341")),
            _ => panic!("精确匹配应唯一命中"),
        }
        match super::resolve_session_id(&conn, "922d7ecb", None).unwrap() {
            super::Resolution::Unique(id) => assert_eq!(id, "922d7ecb-6fb6-4ebf-bfd5-415b4d5b9469"),
            _ => panic!("前缀应唯一命中"),
        }
    }

    #[test]
    fn resolve_hallucinated_uuid_via_first_segment() {
        let conn = ids_db();
        // agent 记住首段 922d7ecb 但脑补了后半段 → 首段匹配救回
        match super::resolve_session_id(&conn, "922d7ecb-6780-4d7b-beee-018bc87d59cc", None).unwrap() {
            super::Resolution::Unique(id) => assert_eq!(id, "922d7ecb-6fb6-4ebf-bfd5-415b4d5b9469"),
            _ => panic!("幻觉 UUID 应通过首段命中"),
        }
    }

    #[test]
    fn resolve_ambiguous_lists_candidates() {
        let conn = ids_db();
        match super::resolve_session_id(&conn, "aaaaaaaa", None).unwrap() {
            super::Resolution::Ambiguous(c) => assert_eq!(c.len(), 2),
            _ => panic!("同首段两条应歧义"),
        }
        // cli 过滤可消解歧义（此处两条同 cli，仍歧义）；不存在则 NotFound
        assert!(matches!(
            super::resolve_session_id(&conn, "aaaaaaaa", Some("claude")).unwrap(),
            super::Resolution::NotFound
        ));
        assert!(matches!(
            super::resolve_session_id(&conn, "deadbeef", None).unwrap(),
            super::Resolution::NotFound
        ));
    }

    #[test]
    fn app_db_path_lives_under_data_subdir() {
        let p = super::app_db_path().unwrap();
        assert!(
            p.ends_with("com.seshbuddy.app/data/app.db"),
            "app.db 应位于 com.seshbuddy.app/data/ 下，实际: {:?}",
            p
        );
    }

    #[test]
    fn cache_db_path_lives_under_data_subdir() {
        let p = super::cache_db_path().unwrap();
        assert!(
            p.ends_with("com.seshbuddy.app/assistant/transcript_cache.db"),
            "transcript_cache.db 应位于 com.seshbuddy.app/assistant/ 下，实际: {:?}",
            p
        );
    }

    #[test]
    fn ensure_fresh_extracts_and_show_reads_back() {
        // 用临时 HOME 不可行（app_data_dir 写死），改为拆纯函数测试：
        // write_extraction + load_messages 的往返已在 transcript-store 覆盖，
        // 这里只验证 find_session 的 SQL 形态
        use rusqlite::Connection;
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("app.db");
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch(
            "CREATE TABLE session_list_index (
               session_path TEXT, session_id TEXT, cli_id TEXT,
               title TEXT, first_user_message TEXT, last_timestamp TEXT
             );
             INSERT INTO session_list_index VALUES ('/p/a.jsonl', 'sess-1', 'claude', 't', '', '2026-08-01');",
        )
        .unwrap();
        let found = super::find_session(&conn, "sess-1", Some("claude")).unwrap();
        assert_eq!(found, Some(("/p/a.jsonl".to_string(), "claude".to_string())));
        assert!(super::find_session(&conn, "nope", None).unwrap().is_none());
    }

    #[test]
    fn find_session_propagates_sql_errors_instead_of_swallowing() {
        // 缺 session_list_index 表的库（损坏/schema 不匹配）：必须返回 Err 向上抛，
        // 而不是吞成 None 让调用方误报 exit 3 session-not-found
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let r = super::find_session(&conn, "any", None);
        assert!(r.is_err(), "SQL 错误应向上抛，实际: {:?}", r);
    }

    #[test]
    fn file_signature_distinguishes_notfound_from_transient_io_errors() {
        let dir = tempfile::tempdir().unwrap();
        // 已删 → Ok(None)：唯一允许走 gone 终态的情形
        let missing = dir.path().join("nope.jsonl");
        assert_eq!(super::file_signature(missing.to_str().unwrap()).unwrap(), None);
        // 存在 → Ok(Some)
        let present = dir.path().join("a.jsonl");
        std::fs::write(&present, "{}").unwrap();
        assert!(super::file_signature(present.to_str().unwrap()).unwrap().is_some());
        // 把文件当目录用（ENOTDIR）：非 NotFound 的 IO 错误必须 Err 向上抛，不能吞成 gone
        let not_dir = present.join("child");
        let r = super::file_signature(not_dir.to_str().unwrap());
        assert!(r.is_err(), "非 NotFound IO 错误必须向上抛，实际: {:?}", r);
    }

    #[test]
    fn build_list_sql_filters_by_cli_and_days() {
        let (sql, params) = super::build_list_query(Some("claude"), Some(7), None, None, None, 100);
        assert!(sql.contains("cli_id = ?"));
        assert!(sql.contains("datetime(last_timestamp)"));
        assert!(params.iter().any(|p| p == "claude"));
    }

    #[test]
    fn build_list_sql_filters_by_date_range() {
        let (sql, params) =
            super::build_list_query(None, None, Some("2026-07-27"), Some("2026-07-31"), None, 100);
        assert!(sql.contains("date(last_timestamp) >= date(?)"));
        assert!(sql.contains("date(last_timestamp) <= date(?)"));
        assert!(params.iter().any(|p| p == "2026-07-27"));
        assert!(params.iter().any(|p| p == "2026-07-31"));
    }

    #[test]
    fn build_list_sql_without_filters() {
        let (sql, params) = super::build_list_query(None, None, None, None, None, 100);
        assert!(!sql.contains("cli_id = ?"));
        assert!(params.is_empty());
    }

    #[test]
    fn select_range_reads_from_to_inclusive() {
        let msgs = (1..=10)
            .map(|i| transcript_store::extract::ExtractedMessage {
                seq: i,
                role: "user".into(),
                text: format!("msg{}", i),
            })
            .collect::<Vec<_>>();
        let (sel, truncated) = super::select_range(&msgs, Some(3), Some(5), 8000);
        assert_eq!(sel.len(), 3);
        assert_eq!(sel[0].seq, 3);
        assert_eq!(sel[2].seq, 5);
        assert!(!truncated);
    }

    #[test]
    fn select_range_truncates_by_max_chars() {
        let msgs = (1..=5)
            .map(|i| transcript_store::extract::ExtractedMessage {
                seq: i,
                role: "user".into(),
                text: "x".repeat(100),
            })
            .collect::<Vec<_>>();
        let (sel, truncated) = super::select_range(&msgs, None, None, 250);
        assert!(sel.len() < 5);
        assert!(truncated);
    }

    #[test]
    fn select_range_defaults_to_full() {
        let msgs = (1..=3)
            .map(|i| transcript_store::extract::ExtractedMessage {
                seq: i,
                role: "user".into(),
                text: format!("m{}", i),
            })
            .collect::<Vec<_>>();
        let (sel, truncated) = super::select_range(&msgs, None, None, 8000);
        assert_eq!(sel.len(), 3);
        assert!(!truncated);
    }

    #[test]
    fn grep_hits_include_snippet_around_keyword() {
        let msgs = vec![
            transcript_store::extract::ExtractedMessage { seq: 1, role: "user".into(), text: "前面一些文字关键词后面一些文字".repeat(10) },
        ];
        let hits = super::grep_messages("sess1", "标题", &msgs, "关键词", 100);
        assert_eq!(hits.len(), 1);
        assert!(hits[0].snippet.contains("关键词"));
        assert!(hits[0].snippet.len() < msgs[0].text.len());
        assert_eq!(hits[0].message_index, 1);
    }

    #[test]
    fn coverage_partial_when_cache_smaller_than_index() {
        assert_eq!(super::coverage_label(45, 1200), "partial");
        assert_eq!(super::coverage_label(0, 0), "full");
        assert_eq!(super::coverage_label(50, 50), "full");
    }

    #[test]
    fn grep_does_not_underflow_when_lowercase_expands_chars() {
        // 'İ' lowercase 后变成两字符（i + 组合点），lower 文本比原文长
        // 修复前 end - start 下溢：debug panic / release 空 snippet
        let msgs = vec![transcript_store::extract::ExtractedMessage {
            seq: 1,
            role: "user".into(),
            text: "İ".repeat(150) + "key",
        }];
        let hits = super::grep_messages("s", "", &msgs, "key", 100);
        assert_eq!(hits.len(), 1);
        assert!(hits[0].snippet.contains("key"));
    }

    #[test]
    fn grep_matches_case_insensitively() {
        let msgs = vec![transcript_store::extract::ExtractedMessage {
            seq: 1,
            role: "user".into(),
            text: "Hello KEY world".to_string(),
        }];
        let hits = super::grep_messages("s", "", &msgs, "key", 100);
        assert_eq!(hits.len(), 1);
        assert!(hits[0].snippet.contains("KEY"));
    }

    #[test]
    fn help_json_covers_all_agent_commands() {
        let help = super::agent_help_json();
        let commands = help["commands"].as_array().unwrap();
        let usages: Vec<&str> = commands.iter()
            .filter_map(|c| c["usage"].as_str()).collect();
        assert!(usages.iter().any(|u| u.starts_with("list")));
        assert!(usages.iter().any(|u| u.starts_with("grep")));
        assert!(usages.iter().any(|u| u.starts_with("show")));
        // 退出码契约必须包含 0/1/2/3
        let codes = help["exitCodes"].as_object().unwrap();
        for k in ["0", "1", "2", "3"] { assert!(codes.contains_key(k)); }
    }
}
