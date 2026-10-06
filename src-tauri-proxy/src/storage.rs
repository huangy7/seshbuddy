use crate::capture::TrafficRecord;
use rusqlite::{params, Connection};
use std::path::Path;
use std::sync::Mutex;

pub struct Storage {
    conn: Mutex<Connection>,
}

impl Storage {
    pub fn new(db_path: &str) -> anyhow::Result<Self> {
        if let Some(parent) = Path::new(db_path).parent() {
            std::fs::create_dir_all(parent)?;
        }

        let conn = Connection::open(db_path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL;")?;
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS traffic (
                id          TEXT PRIMARY KEY,
                timestamp   TEXT NOT NULL,
                method      TEXT NOT NULL,
                path        TEXT NOT NULL,
                req_headers TEXT,
                req_body    TEXT,
                req_size    INTEGER NOT NULL DEFAULT 0,
                status      INTEGER,
                res_headers TEXT,
                res_body    TEXT,
                compression TEXT,
                error_kind  TEXT,
                res_size    INTEGER NOT NULL DEFAULT 0,
                duration_ms INTEGER NOT NULL DEFAULT 0,
                session_id  TEXT,
                cli_id      TEXT DEFAULT 'claude'
            );
            CREATE INDEX IF NOT EXISTS idx_traffic_timestamp ON traffic(timestamp);
            CREATE INDEX IF NOT EXISTS idx_traffic_session_timestamp ON traffic(session_id, timestamp);
            CREATE INDEX IF NOT EXISTS idx_traffic_cli_timestamp ON traffic(cli_id, timestamp);
            ",
        )?;
        // `CREATE TABLE IF NOT EXISTS` 对**已存在**的旧库是空操作，补一次列：
        // 幂等（列已在则报 duplicate column，忽略）。未发布版本，不需要正式迁移。
        let _ = conn.execute_batch("ALTER TABLE traffic ADD COLUMN compression TEXT;");
        let _ = conn.execute_batch("ALTER TABLE traffic ADD COLUMN error_kind TEXT;");

        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn insert(&self, record: &TrafficRecord) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "INSERT INTO traffic (id, timestamp, method, path, req_headers, req_body, req_size, status, res_headers, res_body, compression, error_kind, res_size, duration_ms, session_id, cli_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
            params![
                record.id,
                record.timestamp,
                record.method,
                record.path,
                record.req_headers,
                record.req_body,
                record.req_size,
                record.status,
                record.res_headers,
                record.res_body,
                record.compression,
                record.error_kind,
                record.res_size,
                record.duration_ms,
                record.session_id,
                record.cli_id,
            ],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record() -> TrafficRecord {
        TrafficRecord {
            id: "r1".to_string(),
            timestamp: "2026-09-25T00:00:00Z".to_string(),
            method: "POST".to_string(),
            path: "/v1/messages".to_string(),
            req_headers: Some("{}".to_string()),
            req_body: Some("{}".to_string()),
            req_size: 10,
            status: Some(200),
            res_headers: Some("{}".to_string()),
            res_body: None,
            compression: Some("gzip".to_string()),
            error_kind: None,
            res_size: 1234,
            duration_ms: 30,
            session_id: Some("s1".to_string()),
            cli_id: "claude".to_string(),
        }
    }

    /// 压缩事实走独立列，正文留 `NULL`——Tauri 侧读同一个库并据此渲染提示句。
    #[test]
    fn compression_round_trips_and_body_stays_null() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("traffic.db");
        let storage = Storage::new(db.to_str().unwrap()).unwrap();
        storage.insert(&record()).unwrap();

        let conn = Connection::open(db.to_str().unwrap()).unwrap();
        let (body, compression): (Option<String>, Option<String>) = conn
            .query_row("SELECT res_body, compression FROM traffic", [], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .unwrap();
        assert_eq!(body, None);
        assert_eq!(compression.as_deref(), Some("gzip"));
    }

    /// 上游失败：`error_kind` 落库并能读回；`res_body` 装的是**第三方原文本身**。
    ///
    /// 这里刻意走**真实的代理链路**（`proxy::run_proxy` → 上游连接失败那条分支），
    /// 而不是手搓一条 `TrafficRecord` 塞进 `Storage`：手搓只能证明存储层存得下，
    /// 证明不了「我们自己的包装句没有进库」——而后者才是这条测试存在的理由。
    /// 进程级环境锁：本测试要临时改 `NO_PROXY`。
    /// 环境变量是进程全局的，与其它会构造 reqwest 客户端的测试必须互斥。
    static PROXY_ENV_LOCK: std::sync::LazyLock<Mutex<()>> =
        std::sync::LazyLock::new(|| Mutex::new(()));

    /// 临时把回环地址排除出上游代理，退出时（含 panic）还原。
    ///
    /// 为什么必须有：`reqwest::Client::builder()` 默认读取环境里的
    /// `HTTP_PROXY` / `NO_PROXY`。开发机若设了 `HTTP_PROXY` 而没设 `NO_PROXY`，
    /// 本测试「上游指向死端口 → 连接失败」的前提就不成立——请求会被交给环境代理，
    /// 环境代理够不到那个端口，回一个 502 空响应。于是 `send()` 返回 `Ok(502)`，
    /// 走的是**上游返回了 502** 那条正常分支（写 `status=502`、空正文、
    /// `error_kind` 为 `None`），而不是本测试要覆盖的**上游连接失败**分支。
    /// 表现是断言 `error_kind == Some("upstream")` 失败，且只在设了代理的机器上复现。
    struct NoProxyGuard {
        saved: Vec<(&'static str, Option<std::ffi::OsString>)>,
    }

    impl NoProxyGuard {
        fn bypass_loopback() -> Self {
            const KEYS: [&str; 2] = ["NO_PROXY", "no_proxy"];
            let saved = KEYS.iter().map(|k| (*k, std::env::var_os(k))).collect();
            for key in KEYS {
                std::env::set_var(key, "127.0.0.1,localhost,::1");
            }
            Self { saved }
        }
    }

    impl Drop for NoProxyGuard {
        fn drop(&mut self) {
            for (key, value) in &self.saved {
                match value {
                    Some(v) => std::env::set_var(key, v),
                    None => std::env::remove_var(key),
                }
            }
        }
    }

    #[tokio::test]
    async fn error_kind_round_trips_and_body_holds_third_party_text() {
        let _env_lock = PROXY_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _no_proxy = NoProxyGuard::bypass_loopback();

        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("traffic.db");
        let db_path = db.to_str().unwrap().to_string();

        // 上游指向一个刚释放的端口：连接必然失败，正是写 `res_body` 的那条分支。
        // 代理自身另监听一个刚释放的端口（`run_proxy` 只收地址字符串，收不了已绑定的 listener）。
        let upstream_port = free_port();
        let listen_port = free_port();
        // 两次 `free_port()` 之间端口可能被复用（前一个 listener 已释放）：一旦撞上，
        // 代理就把自己当上游、层层递归。下面的超时让这种情形变成失败，而不是挂死。
        assert_ne!(upstream_port, listen_port, "两个端口撞了：代理会把自己当上游");

        let storage = Storage::new(&db_path).unwrap();
        let broadcast = crate::ws::WsBroadcast::new(16);
        let target = format!("http://127.0.0.1:{}", upstream_port);
        let listen = format!("127.0.0.1:{}", listen_port);
        tokio::spawn(async move {
            let _ = crate::proxy::run_proxy(&listen, &target, "claude", storage, broadcast).await;
        });

        let proxy_addr = format!("127.0.0.1:{}", listen_port);
        assert!(
            wait_until_listening(&proxy_addr).await,
            "代理未在 5s 内监听 {proxy_addr}"
        );

        let resp = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .unwrap()
            .post(format!("http://{}/v1/messages", proxy_addr))
            .header("content-type", "application/json")
            .body(r#"{"messages":[]}"#)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status().as_u16(), 502);

        // 502 返回前 `store_record` 已在同一个任务里同步落库，这里直接读。
        let conn = Connection::open(&db_path).unwrap();
        let (body, kind): (Option<String>, Option<String>) = conn
            .query_row("SELECT res_body, error_kind FROM traffic", [], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .unwrap();

        // 断言一：事实落库、能读回——存储层真的接上了。
        assert_eq!(kind.as_deref(), Some("upstream"));
        // 断言二（本任务的核心）：正文里只有第三方原文，**没有**我们自己的包装句。
        // 少了它，「继续把包装句粘进 res_body、只额外加个字段」的实现同样能过。
        let body = body.expect("res_body 必须保留第三方原文");
        assert!(!body.trim().is_empty(), "res_body 不该是空的");
        assert!(
            !body.contains("Upstream error") && !body.contains("Proxy error"),
            "res_body 里混进了我们自己的包装句：{body}"
        );
        // 只钉英文前缀不够：把包装句换成**中文**同样是把我们的文案粘进正文（R3 禁止的
        // 那件事），只是换了个语言。这条正文只可能是 reqwest 对连接失败的英文诊断，
        // 因此出现中日韩表意文字即说明混进了我们的文案。这里刻意不绑定 reqwest 的
        // 具体措辞（那样会随版本脆断），只钉「我们的文案不在里面」。
        assert!(
            !contains_cjk(&body),
            "res_body 里混进了我们自己的中文文案：{body}"
        );
    }

    /// 是否含中日韩表意文字（U+4E00–U+9FFF）。
    fn contains_cjk(s: &str) -> bool {
        s.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c))
    }

    fn free_port() -> u16 {
        std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port()
    }

    async fn wait_until_listening(addr: &str) -> bool {
        for _ in 0..200 {
            if tokio::net::TcpStream::connect(addr).await.is_ok() {
                return true;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        false
    }

    /// 旧库（建表时还没有 `compression` 列）由 `Storage::new` 就地补列：
    /// 不补的话本文件的 INSERT 与 Tauri 侧的 SELECT 都会因缺列整条失败。
    #[test]
    fn existing_db_without_column_is_backfilled() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("traffic.db");
        let path = db.to_str().unwrap();
        {
            let conn = Connection::open(path).unwrap();
            conn.execute_batch(
                "CREATE TABLE traffic (
                    id          TEXT PRIMARY KEY,
                    timestamp   TEXT NOT NULL,
                    method      TEXT NOT NULL,
                    path        TEXT NOT NULL,
                    req_headers TEXT,
                    req_body    TEXT,
                    req_size    INTEGER NOT NULL DEFAULT 0,
                    status      INTEGER,
                    res_headers TEXT,
                    res_body    TEXT,
                    res_size    INTEGER NOT NULL DEFAULT 0,
                    duration_ms INTEGER NOT NULL DEFAULT 0,
                    session_id  TEXT,
                    cli_id      TEXT DEFAULT 'claude'
                );",
            )
            .unwrap();
        }

        let storage = Storage::new(path).unwrap();
        storage.insert(&record()).unwrap();
        // 再开一次：补列是幂等的，重复打开不该报错
        let storage = Storage::new(path).unwrap();
        let mut second = record();
        second.id = "r2".to_string();
        storage.insert(&second).unwrap();
    }
}
