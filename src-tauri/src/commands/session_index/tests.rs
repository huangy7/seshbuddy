    use super::*;
    use crate::session as session_mod;
    use crate::session::SessionListMetadata;
    use serde_json::Value;

    #[test]
    fn requested_kinds_resolves_supplied_subset_and_deduplicates_ids() {
        let kinds = requested_kinds_with_data_dir(
            Some(&[
                "codex".to_string(),
                "claude".to_string(),
                "codex".to_string(),
            ]),
            None,
            |_| true,
        )
        .unwrap();

        assert_eq!(kinds, vec![CliKind::Codex, CliKind::Claude]);
    }

    #[test]
    fn requested_kinds_rejects_invalid_ids_before_filtering_absent_directories() {
        let result = requested_kinds_with_data_dir(
            Some(&["not-a-cli".to_string()]),
            None,
            |_| false,
        );

        assert!(result.is_err());
    }

    #[test]
    fn requested_kinds_moves_included_front_cli_first() {
        let kinds = requested_kinds_with_data_dir(
            Some(&[
                "claude".to_string(),
                "codex".to_string(),
                "gemini".to_string(),
            ]),
            Some("gemini"),
            |_| true,
        )
        .unwrap();

        assert_eq!(
            kinds,
            vec![CliKind::Gemini, CliKind::Claude, CliKind::Codex]
        );
    }

    #[test]
    fn requested_kinds_skips_absent_data_directories_and_allows_empty_subset() {
        let kinds = requested_kinds_with_data_dir(
            Some(&["claude".to_string(), "codex".to_string()]),
            Some("claude"),
            |kind| kind == CliKind::Codex,
        )
        .unwrap();
        let empty = requested_kinds_with_data_dir(Some(&[]), None, |_| true).unwrap();

        assert_eq!(kinds, vec![CliKind::Codex]);
        assert!(empty.is_empty());
    }

    #[test]
    fn explicit_cli_ids_take_precedence_over_legacy_cli_id() {
        let ids = effective_cli_ids(
            Some(vec!["codex".to_string(), "gemini".to_string()]),
            Some("claude".to_string()),
        );

        assert_eq!(ids, Some(vec!["codex".to_string(), "gemini".to_string()]));
    }

    #[test]
    fn legacy_cli_id_preserves_single_source_and_all_semantics() {
        assert_eq!(
            effective_cli_ids(None, Some("claude".to_string())),
            Some(vec!["claude".to_string()])
        );
        assert_eq!(effective_cli_ids(None, Some("all".to_string())), None);
        assert_eq!(effective_cli_ids(None, None), None);
    }

    #[test]
    fn cli_fanout_continues_after_one_cli_fails() {
        let mut outcomes = Vec::new();
        run_cli_fanout(
            &[CliKind::Claude, CliKind::Codex],
            |kind| {
                if kind == CliKind::Claude {
                    Err(AppError::business("broken source"))
                } else {
                    Ok(kind.id().to_string())
                }
            },
            |outcome| outcomes.push(outcome),
        );

        assert_eq!(outcomes.len(), 2);
        assert!(outcomes[0].result.is_err());
        assert_eq!(outcomes[1].result.as_deref().unwrap(), "codex");
    }

    fn list_record(path: &str, modified_ms: i64) -> app_db::SessionListIndexRecord {
        app_db::SessionListIndexRecord {
            session_path: path.to_string(),
            session_id: path.to_string(),
            project_path: None,
            title: None,
            first_user_message: None,
            first_timestamp: None,
            last_timestamp: None,
            git_branch: String::new(),
            file_size: 1,
            modified_ms,
            has_archive_snapshot: false,
            is_archived: false,
        }
    }

    fn search_record(
        path: &str,
        modified_ms: i64,
        doc_count: usize,
    ) -> app_db::SessionSearchIndexRecord {
        app_db::SessionSearchIndexRecord {
            session_path: path.to_string(),
            modified_ms,
            doc_count,
        }
    }

    /// 两个 DSH 扫描测试共享真实 app DB（Dsh 路径覆盖 + Dsh 索引），并行会互相覆盖，
    /// 串行化避免 flaky。
    static DSH_SCAN_TEST_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

    /// 防御性清理：真实 app DB 跨进程共享，先前中断/并发运行的冒烟可能残留 DSH 索引行
    /// （曾致快照断言看到 4 个项目而非 1 个）。每个 DSH 测试开始前清一次，
    /// 保证起始状态干净（结束后再清一次恢复原状）。
    fn clear_dsh_index_rows() {
        if let Ok(conn) = app_db::conn() {
            let _ = app_db::clear_session_index_inner(&conn, "dsh");
        }
    }

    /// Codex 的项目路径落库测试与 DSH 同理：共享真实 app DB 的 Codex 索引行与数据目录覆盖，
    /// 与 cli 模块中的守卫单元测试共用此锁串行化避免争用。
    use crate::cli::CODEX_DATA_DIR_OVERRIDE_TEST_LOCK as CODEX_SCAN_TEST_LOCK;

    fn clear_codex_index_rows() {
        if let Ok(conn) = app_db::conn() {
            let _ = app_db::clear_session_index_inner(&conn, "codex");
        }
    }

    /// Claude 的项目路径展示回归与 DSH / Codex 同理：共享真实 app DB 的 Claude 索引行，
    /// 串行化避免 flaky。
    static CLAUDE_SCAN_TEST_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

    fn clear_claude_index_rows() {
        if let Ok(conn) = app_db::conn() {
            let _ = app_db::clear_session_index_inner(&conn, "claude");
        }
    }

    /// OpenCode 端到端验收会写共享的真实 app DB；清掉本 CLI 的索引行，
    /// 既让起始状态干净，也在结束后恢复原状。
    fn clear_opencode_index_rows() {
        if let Ok(conn) = app_db::conn() {
            let _ = app_db::clear_session_index_inner(&conn, "opencode");
        }
    }

    #[test]
    fn collect_missing_search_paths_rebuilds_all_when_physical_index_is_not_ready() {
        let list_index = HashMap::from([
            ("a.jsonl".to_string(), list_record("a.jsonl", 10)),
            ("b.jsonl".to_string(), list_record("b.jsonl", 20)),
        ]);
        let search_index = HashMap::from([
            ("a.jsonl".to_string(), search_record("a.jsonl", 10, 3)),
            ("b.jsonl".to_string(), search_record("b.jsonl", 20, 2)),
        ]);

        let missing = collect_missing_search_paths(CliKind::Claude, &list_index, &search_index, false);

        assert_eq!(missing, vec!["a.jsonl".to_string(), "b.jsonl".to_string()]);
    }

    #[test]
    fn collect_missing_search_paths_ignores_txt_cache_gap() {
        let list_index = HashMap::from([
            ("a.jsonl".to_string(), list_record("a.jsonl", 10)),
            ("b.jsonl".to_string(), list_record("b.jsonl", 20)),
        ]);
        let search_index = HashMap::from([
            ("a.jsonl".to_string(), search_record("a.jsonl", 10, 3)),
            ("b.jsonl".to_string(), search_record("b.jsonl", 20, 2)),
        ]);

        // txt 缺失不再计入 missing：record 与 search_index 一致时为空
        let missing = collect_missing_search_paths(CliKind::Claude, &list_index, &search_index, true);

        assert!(missing.is_empty(), "txt 缺口不应再触发重扫");
    }

    #[test]
    fn collect_missing_search_paths_treats_zero_doc_count_with_matching_ts_as_indexed() {
        // 回归：tool_use/tool_result 内容从索引剔除后，纯工具调用会话的 doc_count == 0，
        // 但 modified_ms 匹配说明已经尝试过索引、只是无可搜索文本，不应再次触发索引构建。
        let list_index = HashMap::from([
            ("tool_only.jsonl".to_string(), list_record("tool_only.jsonl", 100)),
            ("has_text.jsonl".to_string(), list_record("has_text.jsonl", 200)),
            ("stale.jsonl".to_string(), list_record("stale.jsonl", 300)),
        ]);
        let search_index = HashMap::from([
            // doc_count == 0 但 modified_ms 匹配 → 已索引（无文本内容），不算 missing
            ("tool_only.jsonl".to_string(), search_record("tool_only.jsonl", 100, 0)),
            // doc_count > 0，modified_ms 匹配 → 正常已索引
            ("has_text.jsonl".to_string(), search_record("has_text.jsonl", 200, 5)),
            // modified_ms 不匹配（文件有更新）→ 需要重新索引
            ("stale.jsonl".to_string(), search_record("stale.jsonl", 999, 3)),
        ]);

        let missing = collect_missing_search_paths(CliKind::Claude, &list_index, &search_index, true);

        // 只有 stale.jsonl（ts 不匹配）是 missing，tool_only 和 has_text 均已索引
        assert_eq!(missing, vec!["stale.jsonl".to_string()]);
    }

    #[test]
    fn test_tombstone_blocks_session_scanning() {
        let conn = app_db::conn().unwrap();
        let dead_id = "mock-dead-session";
        let dead_path = "/tmp/mock-dead-session.jsonl";

        // 前置清理，保持测试幂等
        let _ = app_db::clear_session_tombstone_inner(&conn, "claude", dead_id);

        app_db::record_tombstone_inner(&conn, "claude", dead_id, dead_path).unwrap();

        // 验证无论传该文件路径还是异地路径，只要 ID 或路径命中即被判定为 tombstoned
        assert!(app_db::is_tombstoned_inner(&conn, "claude", dead_id, "/another/dir/mock-dead-session.jsonl").unwrap());
        assert!(app_db::is_tombstoned_inner(&conn, "claude", "alive-session", dead_path).unwrap());
        assert!(!app_db::is_tombstoned_inner(&conn, "claude", "alive-session", "/another/dir/alive-session.jsonl").unwrap());

        // 验证 collect_missing_search_paths 过滤墓碑会话
        let list_index = HashMap::from([
            (dead_path.to_string(), list_record(dead_path, 100)),
            ("/tmp/mock-alive-session.jsonl".to_string(), list_record("/tmp/mock-alive-session.jsonl", 100)),
        ]);
        let search_index = HashMap::new();
        let missing = collect_missing_search_paths(CliKind::Claude, &list_index, &search_index, false);
        assert_eq!(missing, vec!["/tmp/mock-alive-session.jsonl".to_string()]);

        // 后置清理
        let _ = app_db::clear_session_tombstone_inner(&conn, "claude", dead_id);
    }

    #[test]
    fn collect_stale_paths_keeps_sessions_with_archive_snapshot() {
        let mut archived_record = list_record("/archived.jsonl", 10);
        archived_record.has_archive_snapshot = true;
        let cached = HashMap::from([
            ("/archived.jsonl".to_string(), archived_record),
            ("/gone.jsonl".to_string(), list_record("/gone.jsonl", 20)),
        ]);
        let seen_paths = HashSet::new();
        let invalid_paths = HashSet::new();

        let stale = collect_stale_paths(&cached, &seen_paths, &invalid_paths, &[]);

        assert_eq!(stale, vec!["/gone.jsonl".to_string()]);
    }

    /// 读取失败的子树下，行的"没被枚举到"不能推断为已删除：该子树原样保留，其余照常判定。
    #[test]
    fn collect_stale_paths_keeps_rows_under_unverified_subtree() {
        let cached = HashMap::from([
            (
                "/proj/readable/a.jsonl".to_string(),
                list_record("/proj/readable/a.jsonl", 1),
            ),
            (
                "/proj/locked/b.jsonl".to_string(),
                list_record("/proj/locked/b.jsonl", 2),
            ),
            (
                "/proj/locked/nested/c.jsonl".to_string(),
                list_record("/proj/locked/nested/c.jsonl", 3),
            ),
        ]);
        let seen_paths = HashSet::new();
        let invalid_paths = HashSet::new();
        let unverified = vec![PathBuf::from("/proj/locked")];

        let stale = collect_stale_paths(&cached, &seen_paths, &invalid_paths, &unverified);

        assert_eq!(
            stale,
            vec!["/proj/readable/a.jsonl".to_string()],
            "读取失败子树（含其子目录）下的记录应原样保留"
        );
    }

    /// 大面积"未枚举到"被存活比例守卫拦下：一次瞬时故障不该把整批记录清光。
    /// 小集合与存活率过半时仍照常清理，否则日常删除会失效。
    #[test]
    fn collect_stale_paths_blocks_mass_deletion() {
        let cached: HashMap<String, app_db::SessionListIndexRecord> = (0..12)
            .map(|i| {
                let path = format!("/proj/s{i}.jsonl");
                (path.clone(), list_record(&path, i))
            })
            .collect();
        let seen = |n: i64| -> HashSet<String> {
            (0..n).map(|i| format!("/proj/s{i}.jsonl")).collect()
        };

        // 一条都没枚举到 → 疑似数据源不可用或读取故障，拒绝清理
        let stale = collect_stale_paths(&cached, &HashSet::new(), &HashSet::new(), &[]);
        assert!(stale.is_empty(), "已知 12 条全部缺失时应拒绝清理: {stale:?}");

        // 存活 4/12 ≈ 33%，未过半 → 同样拦下
        let stale = collect_stale_paths(&cached, &seen(4), &HashSet::new(), &[]);
        assert!(stale.is_empty(), "存活率未过半时应拒绝清理: {stale:?}");

        // 存活 9/12 = 75% → 正常清理缺失的 3 条
        let stale = collect_stale_paths(&cached, &seen(9), &HashSet::new(), &[]);
        assert_eq!(stale.len(), 3, "存活率正常时应照常清理: {stale:?}");

        // 小集合不做保护：3 条全缺失属日常删除，照常清理
        let small: HashMap<String, app_db::SessionListIndexRecord> = (0..3)
            .map(|i| {
                let path = format!("/small/s{i}.jsonl");
                (path.clone(), list_record(&path, i))
            })
            .collect();
        let stale = collect_stale_paths(&small, &HashSet::new(), &HashSet::new(), &[]);
        assert_eq!(stale.len(), 3, "小集合不应被守卫拦下: {stale:?}");
    }

    #[test]
    fn collect_stale_paths_keeps_invalid_paths_regardless_of_archive() {
        let mut archived_record = list_record("/archived.jsonl", 10);
        archived_record.has_archive_snapshot = true;
        let cached = HashMap::from([("/archived.jsonl".to_string(), archived_record)]);
        let seen_paths = HashSet::from(["/archived.jsonl".to_string()]);
        let invalid_paths = HashSet::from(["/broken.jsonl".to_string()]);

        let stale = collect_stale_paths(&cached, &seen_paths, &invalid_paths, &[]);

        assert_eq!(stale, vec!["/broken.jsonl".to_string()]);
    }

    /// 临时目录构造 zstd 会话夹具 → `scan_projects_inner_for_cli(Dsh,..)` 返回 1 项目/1 会话，
    /// `file_path` 指向 zstd、`original_path` 取 header cwd。
    #[test]
    fn dsh_scan_indexes_zstd_session_fixture() {
        let _guard = DSH_SCAN_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_dsh_index_rows();
        let root = tempfile::tempdir().unwrap();
        let session_file = root
            .path()
            .join("sessions")
            .join("--Users-x-proj--")
            .join("s1")
            .join("session.jsonl.zstd");
        fs::create_dir_all(session_file.parent().unwrap()).unwrap();
        let content = concat!(
            "{\"type\":\"session\",\"version\":0,\"id\":\"s1\",\"createdAt\":1700000000000,\"cwd\":\"/Users/x/proj\"}\n",
            "{\"type\":\"user/message\",\"seq\":1,\"time\":1700000000000,\"data\":{\"content\":[{\"type\":\"text\",\"text\":\"hi\"}]}}\n",
        );
        let compressed = zstd::encode_all(std::io::Cursor::new(content.as_bytes()), 3).unwrap();
        fs::write(&session_file, compressed).unwrap();

        let _override_guard = cli::CliDataDirOverrideGuard::set(CliKind::Dsh, root.path());

        let result = scan_projects_inner_for_cli(CliKind::Dsh, None, None, true).unwrap();
        clear_dsh_index_rows();

        assert_eq!(result.total_sessions, 1);
        assert_eq!(result.projects.len(), 1);
        let project = &result.projects[0];
        assert_eq!(project.original_path.as_deref(), Some("/Users/x/proj"));
        assert_eq!(project.encoded_dir.as_deref(), Some("/Users/x/proj"));
        assert_eq!(project.sessions.len(), 1);
        let session = &project.sessions[0];
        assert!(session.file_path.ends_with(".zstd"));
        assert_eq!(session.session_id, "s1");
        assert!(!session.timestamp.is_empty());
        // 统一标题回退规则：title 恒 None，展示名回退到清洗后的首条用户消息（不再是目录名）
        assert_eq!(session.display_name, "hi");
    }

    /// 日志换代回归：会话目录里只有带代次的新代文件时，会话必须照样被发现。
    /// 只认旧文件名的实现会整条漏掉这类会话（列表里直接消失）。
    #[test]
    fn dsh_scan_discovers_session_stored_in_newer_generation_file() {
        let _guard = DSH_SCAN_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_dsh_index_rows();
        let root = tempfile::tempdir().unwrap();
        let session_file = root
            .path()
            .join("sessions")
            .join("--Users-x-proj--")
            .join("s1")
            .join("session.v3.jsonl.zstd");
        fs::create_dir_all(session_file.parent().unwrap()).unwrap();
        let content = concat!(
            "{\"type\":\"session\",\"version\":3,\"id\":\"s1\",\"createdAt\":1700000000000,\"cwd\":\"/Users/x/proj\"}\n",
            "{\"type\":\"user/message\",\"seq\":1,\"time\":1700000000000,\"data\":{\"content\":[{\"type\":\"text\",\"text\":\"hi\"}]}}\n",
        );
        let compressed = zstd::encode_all(std::io::Cursor::new(content.as_bytes()), 3).unwrap();
        fs::write(&session_file, compressed).unwrap();

        let _override_guard = cli::CliDataDirOverrideGuard::set(CliKind::Dsh, root.path());

        let result = scan_projects_inner_for_cli(CliKind::Dsh, None, None, true).unwrap();
        clear_dsh_index_rows();

        assert_eq!(result.total_sessions, 1, "只有新代文件的会话必须被扫到");
        assert!(result.projects[0].sessions[0]
            .file_path
            .ends_with("session.v3.jsonl.zstd"));
    }

    /// 换代搬迁回归：旧代与当前代文件并存时，会话只出现一次、路径指向当前代，
    /// 且旧代路径上遗留的索引行被搬迁走，不会留下一条永远扫不到的重复记录。
    #[test]
    fn dsh_scan_rekeys_legacy_path_records_to_current_generation() {
        let _guard = DSH_SCAN_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_dsh_index_rows();
        let root = tempfile::tempdir().unwrap();
        let session_dir = root
            .path()
            .join("sessions")
            .join("--Users-x-proj--")
            .join("s1");
        fs::create_dir_all(&session_dir).unwrap();
        let legacy_file = session_dir.join("session.jsonl.zstd");
        let current_file = session_dir.join("session.v3.jsonl.zstd");

        let legacy_content = concat!(
            "{\"type\":\"session\",\"version\":0,\"id\":\"s1\",\"createdAt\":1700000000000,\"cwd\":\"/Users/x/proj\"}\n",
            "{\"type\":\"user/message\",\"seq\":1,\"time\":1700000000000,\"data\":{\"content\":[{\"type\":\"text\",\"text\":\"old\"}]}}\n",
        );
        let current_content = concat!(
            "{\"type\":\"session\",\"version\":3,\"id\":\"s1\",\"createdAt\":1700000000000,\"cwd\":\"/Users/x/proj\"}\n",
            "{\"type\":\"user/message\",\"seq\":1,\"time\":1700000000000,\"data\":{\"content\":[{\"type\":\"text\",\"text\":\"new\"}]}}\n",
            "{\"type\":\"assistant/message\",\"seq\":2,\"time\":1700000000001,\"data\":{\"content\":[{\"type\":\"text\",\"text\":\"newer\"}]}}\n",
        );
        fs::write(
            &legacy_file,
            zstd::encode_all(std::io::Cursor::new(legacy_content.as_bytes()), 3).unwrap(),
        )
        .unwrap();
        fs::write(
            &current_file,
            zstd::encode_all(std::io::Cursor::new(current_content.as_bytes()), 3).unwrap(),
        )
        .unwrap();

        let _override_guard = cli::CliDataDirOverrideGuard::set(CliKind::Dsh, root.path());

        // 模拟换代前的存量索引：旧代路径上已有一条记录
        let mut legacy_record = list_record(legacy_file.to_str().unwrap(), 1);
        legacy_record.session_id = "s1".to_string();
        app_db::upsert_session_list_index(CliKind::Dsh, &[legacy_record]).unwrap();

        // 先把索引规则版本钉到当前值：否则本轮会被升级重建逻辑强制成全量解析，
        // 缓存分支不生效，搬迁路径也就无从触发（测试结论会随是否跑过首轮而漂移）
        mark_session_index_revision_current(CliKind::Dsh);

        // force=false 才走缓存索引分支 —— 搬迁正是靠「旧路径上还有记录」来判定
        let result = scan_projects_inner_for_cli(CliKind::Dsh, None, None, false).unwrap();

        // 只取本次夹具目录下的索引行：测试与真实会话共用同一个 app.db，
        // 断言全局表状态会被机器上既有会话干扰
        let root_prefix = format!("{}/", root.path().to_string_lossy());
        let indexed_paths: Vec<String> = {
            let conn = app_db::conn().unwrap();
            let mut stmt = conn
                .prepare("SELECT session_path FROM session_list_index WHERE cli_id = 'dsh' ORDER BY session_path")
                .unwrap();
            stmt.query_map([], |row| row.get::<_, String>(0))
                .unwrap()
                .filter_map(Result::ok)
                .filter(|path| path.starts_with(&root_prefix))
                .collect()
        };
        clear_dsh_index_rows();

        assert_eq!(result.total_sessions, 1, "同一会话目录只能贡献一条记录");
        assert!(result.projects[0].sessions[0]
            .file_path
            .ends_with("session.v3.jsonl.zstd"));
        assert_eq!(
            indexed_paths.len(),
            1,
            "旧代路径的索引行应被搬迁而不是保留: {indexed_paths:?}"
        );
        assert!(
            indexed_paths[0].ends_with("session.v3.jsonl.zstd"),
            "索引行应指向当前代: {indexed_paths:?}"
        );
    }

    /// 首轮强制全量扫描下的搬迁回归：改过推导规则后第一次扫描必然是强制全量（元数据缓存
    /// 为空），此时搬迁判定必须仍能看见库里已入库的旧代路径，否则首轮会留下重复记录。
    #[test]
    fn dsh_scan_rekeys_legacy_path_on_forced_full_scan() {
        let _guard = DSH_SCAN_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_dsh_index_rows();
        let root = tempfile::tempdir().unwrap();
        let session_dir = root
            .path()
            .join("sessions")
            .join("--Users-x-proj--")
            .join("s1");
        fs::create_dir_all(&session_dir).unwrap();
        let legacy_file = session_dir.join("session.jsonl.zstd");
        let current_file = session_dir.join("session.v3.jsonl.zstd");
        let content = concat!(
            "{\"type\":\"session\",\"version\":3,\"id\":\"s1\",\"createdAt\":1700000000000,\"cwd\":\"/Users/x/proj\"}\n",
            "{\"type\":\"user/message\",\"seq\":1,\"time\":1700000000000,\"data\":{\"content\":[{\"type\":\"text\",\"text\":\"hi\"}]}}\n",
        );
        let compressed = zstd::encode_all(std::io::Cursor::new(content.as_bytes()), 3).unwrap();
        fs::write(&legacy_file, &compressed).unwrap();
        fs::write(&current_file, compressed).unwrap();

        let _override_guard = cli::CliDataDirOverrideGuard::set(CliKind::Dsh, root.path());

        let mut legacy_record = list_record(legacy_file.to_str().unwrap(), 1);
        legacy_record.session_id = "s1".to_string();
        app_db::upsert_session_list_index(CliKind::Dsh, &[legacy_record]).unwrap();

        // force=true 等价于规则变更后的首轮：缓存被清空，搬迁判定只能靠库里已入库的路径集合
        let result = scan_projects_inner_for_cli(CliKind::Dsh, None, None, true).unwrap();

        let root_prefix = format!("{}/", root.path().to_string_lossy());
        let indexed_paths: Vec<String> = {
            let conn = app_db::conn().unwrap();
            let mut stmt = conn
                .prepare("SELECT session_path FROM session_list_index WHERE cli_id = 'dsh' ORDER BY session_path")
                .unwrap();
            stmt.query_map([], |row| row.get::<_, String>(0))
                .unwrap()
                .filter_map(Result::ok)
                .filter(|path| path.starts_with(&root_prefix))
                .collect()
        };
        clear_dsh_index_rows();

        assert_eq!(result.total_sessions, 1);
        assert_eq!(
            indexed_paths.len(),
            1,
            "强制全量扫描同样要完成搬迁，不能留下旧代重复行: {indexed_paths:?}"
        );
        assert!(
            indexed_paths[0].ends_with("session.v3.jsonl.zstd"),
            "索引行应指向当前代: {indexed_paths:?}"
        );
    }

    /// 强制全量刷新必须照样清理陈旧行：force 只是不复用元数据缓存，
    /// 不能连带让清理失去比对基准 —— 否则陈旧行（数据源切换、会话删除后产生）
    /// 在强制刷新下永远清不掉。
    #[test]
    fn dsh_forced_scan_still_prunes_stale_index_rows() {
        let _guard = DSH_SCAN_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_dsh_index_rows();
        let root = tempfile::tempdir().unwrap();
        let session_file = root
            .path()
            .join("sessions")
            .join("--Users-x-proj--")
            .join("s1")
            .join("session.jsonl.zstd");
        fs::create_dir_all(session_file.parent().unwrap()).unwrap();
        let content = concat!(
            "{\"type\":\"session\",\"version\":0,\"id\":\"s1\",\"createdAt\":1700000000000,\"cwd\":\"/Users/x/proj\"}\n",
            "{\"type\":\"user/message\",\"seq\":1,\"time\":1700000000000,\"data\":{\"content\":[{\"type\":\"text\",\"text\":\"hi\"}]}}\n",
        );
        fs::write(
            &session_file,
            zstd::encode_all(std::io::Cursor::new(content.as_bytes()), 3).unwrap(),
        )
        .unwrap();

        let _override_guard = cli::CliDataDirOverrideGuard::set(CliKind::Dsh, root.path());

        // 陈旧行：会话目录已不存在，且没有归档快照保护
        let stale_file = root
            .path()
            .join("sessions")
            .join("--Users-x-proj--")
            .join("gone")
            .join("session.jsonl.zstd");
        let stale_record = list_record(stale_file.to_str().unwrap(), 1);
        app_db::upsert_session_list_index(CliKind::Dsh, &[stale_record]).unwrap();

        let result = scan_projects_inner_for_cli(CliKind::Dsh, None, None, true).unwrap();

        let root_prefix = format!("{}/", root.path().to_string_lossy());
        let indexed_paths: Vec<String> = {
            let conn = app_db::conn().unwrap();
            let mut stmt = conn
                .prepare("SELECT session_path FROM session_list_index WHERE cli_id = 'dsh'")
                .unwrap();
            stmt.query_map([], |row| row.get::<_, String>(0))
                .unwrap()
                .filter_map(Result::ok)
                .filter(|path| path.starts_with(&root_prefix))
                .collect()
        };

        clear_dsh_index_rows();

        assert_eq!(result.total_sessions, 1);
        assert!(
            !indexed_paths.iter().any(|path| path.contains("/gone/")),
            "强制刷新也应清理陈旧行: {indexed_paths:?}"
        );
    }

    /// 数据源切换后，范围外的索引行被作废、范围内的保留。
    /// 归属必须按**路径分量**判定：`<scope>-old` 与 `<scope>` 只是字符串前缀相同，不属范围内。
    #[test]
    fn purge_index_rows_outside_data_source_drops_only_out_of_scope_rows() {
        let _guard = DSH_SCAN_TEST_LOCK.lock().unwrap_or_else(|o| o.into_inner());
        clear_dsh_index_rows();
        let scope = cli::sessions_dir(CliKind::Dsh).expect("dsh 会话目录");
        let inside = scope
            .join("--fake-scope-test--")
            .join("s1")
            .join("session.jsonl.zstd");
        // 与 scope 字符串前缀相同但路径分量不同的兄弟目录
        let sibling = PathBuf::from(format!("{}-old", scope.to_string_lossy()))
            .join("--fake-scope-test--")
            .join("s2")
            .join("session.jsonl.zstd");
        let unrelated = PathBuf::from("/tmp/seshbuddy-scope-test")
            .join("s3")
            .join("session.jsonl.zstd");

        for (idx, path) in [&inside, &sibling, &unrelated].iter().enumerate() {
            let record = list_record(path.to_str().unwrap(), idx as i64 + 1);
            app_db::upsert_session_list_index(CliKind::Dsh, &[record]).unwrap();
        }

        purge_index_rows_outside_data_source(CliKind::Dsh);

        let seeded: HashSet<String> = [&inside, &sibling, &unrelated]
            .iter()
            .map(|path| path.to_string_lossy().to_string())
            .collect();
        let survivors: Vec<String> = {
            let conn = app_db::conn().unwrap();
            let mut stmt = conn
                .prepare("SELECT session_path FROM session_list_index WHERE cli_id = 'dsh'")
                .unwrap();
            stmt.query_map([], |row| row.get::<_, String>(0))
                .unwrap()
                .filter_map(Result::ok)
                .filter(|path| seeded.contains(path))
                .collect()
        };
        clear_dsh_index_rows();

        assert_eq!(
            survivors,
            vec![inside.to_string_lossy().to_string()],
            "只应保留落在当前数据源范围内的行"
        );
    }

    /// 子目录读取失败时，该子树下的会话不能被当成"已从磁盘删除"清掉：读不到 ≠ 里面没有。
    /// 权限变化、外置盘未挂载、I/O 压力都会命中这条路径。
    #[test]
    fn dsh_scan_keeps_rows_under_unreadable_project_dir() {
        let _guard = DSH_SCAN_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_dsh_index_rows();
        let root = tempfile::tempdir().unwrap();
        let content = concat!(
            "{\"type\":\"session\",\"version\":0,\"id\":\"s\",\"createdAt\":1700000000000,\"cwd\":\"/Users/x/proj\"}\n",
            "{\"type\":\"user/message\",\"seq\":1,\"time\":1700000000000,\"data\":{\"content\":[{\"type\":\"text\",\"text\":\"hi\"}]}}\n",
        );
        let compressed = zstd::encode_all(std::io::Cursor::new(content.as_bytes()), 3).unwrap();
        let mut files = Vec::new();
        for project in ["--Users-x-projA--", "--Users-x-projB--"] {
            let file = root
                .path()
                .join("sessions")
                .join(project)
                .join("s1")
                .join("session.jsonl.zstd");
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(&file, &compressed).unwrap();
            files.push(file);
        }

        let _override_guard = cli::CliDataDirOverrideGuard::set(CliKind::Dsh, root.path());
        for (idx, file) in files.iter().enumerate() {
            let record = list_record(file.to_str().unwrap(), idx as i64 + 1);
            app_db::upsert_session_list_index(CliKind::Dsh, &[record]).unwrap();
        }
        // 钉住派生规则版本，避免本轮被强制成全量、缓存分支不生效
        mark_session_index_revision_current(CliKind::Dsh);

        // 让其中一个项目目录不可读（以 root 运行测试时 chmod 不生效，用例退化为恒真，不会误报）
        let _locked = root.path().join("sessions").join("--Users-x-projB--");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&_locked, fs::Permissions::from_mode(0o000)).unwrap();
        }

        let scanned = scan_projects_inner_for_cli(CliKind::Dsh, None, None, false);

        // 先还原权限，保证临时目录能被清理
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&_locked, fs::Permissions::from_mode(0o755)).unwrap();
        }
        scanned.unwrap();

        let root_prefix = format!("{}/", root.path().to_string_lossy());
        let indexed_paths: Vec<String> = {
            let conn = app_db::conn().unwrap();
            let mut stmt = conn
                .prepare("SELECT session_path FROM session_list_index WHERE cli_id = 'dsh'")
                .unwrap();
            stmt.query_map([], |row| row.get::<_, String>(0))
                .unwrap()
                .filter_map(Result::ok)
                .filter(|path| path.starts_with(&root_prefix))
                .collect()
        };
        clear_dsh_index_rows();

        assert_eq!(
            indexed_paths.len(),
            2,
            "读取失败的子树下记录应保留，不能按已删除清掉: {indexed_paths:?}"
        );
    }

    /// Antigravity 命名漂移检测：只有兄弟文件、没有正本名时报告该目录；
    /// 正本存在时（哪怕兄弟文件更多）不应报告。
    #[test]
    fn antigravity_drift_detects_missing_canonical_transcript() {
        let normal = vec![
            PathBuf::from("/brain/s1/.system_generated/logs/transcript.jsonl"),
            PathBuf::from("/brain/s1/.system_generated/logs/transcript_full.jsonl"),
        ];
        assert!(
            antigravity_transcript_naming_drift(&normal).is_empty(),
            "正本名存在时不应报告漂移"
        );

        let drifted = vec![
            PathBuf::from("/brain/s2/.system_generated/logs/transcript.v2.jsonl"),
            PathBuf::from("/brain/s2/.system_generated/logs/transcript_full.jsonl"),
        ];
        let report = antigravity_transcript_naming_drift(&drifted);
        assert_eq!(report.len(), 1, "缺正本名的目录应被报告: {report:?}");
        assert_eq!(
            report[0].0,
            PathBuf::from("/brain/s2/.system_generated/logs")
        );
        assert_eq!(
            report[0].1,
            vec!["transcript.v2.jsonl", "transcript_full.jsonl"]
        );
    }

    /// 槽位占用语义：空闲时拿到、被占用时立即放弃，RAII 释放后可再次拿到。
    #[test]
    fn list_index_refresh_slot_acquire_and_release() {
        let kind = CliKind::Gemini;

        assert!(try_acquire_list_index_refresh_slot(kind).unwrap());
        assert!(
            !try_acquire_list_index_refresh_slot(kind).unwrap(),
            "已被占用时不得重复占位"
        );

        drop(ListIndexRefreshGuard(kind));
        assert!(
            try_acquire_list_index_refresh_slot(kind).unwrap(),
            "RAII 释放后应能重新占位"
        );
        drop(ListIndexRefreshGuard(kind));
    }

    /// 强制重建等待槽位：在飞刷新释放后应确实拿到槽位，而不是被静默跳过
    /// （静默跳过会让「已清空的索引没人重建」，这正是本次修复的场景）。
    #[test]
    fn wait_for_list_index_refresh_slot_waits_until_released() {
        let kind = CliKind::Claude;
        assert!(try_acquire_list_index_refresh_slot(kind).unwrap());
        let holder = ListIndexRefreshGuard(kind);

        let poll = FORCE_REFRESH_SLOT_POLL;
        let releaser = std::thread::spawn(move || {
            std::thread::sleep(poll * 3);
            drop(holder);
        });

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        let waited = rt.block_on(wait_for_list_index_refresh_slot(kind));
        releaser.join().unwrap();

        assert!(waited.is_ok(), "在飞刷新释放后应拿到槽位: {waited:?}");
        // 等待期间已占位，释放掉避免影响其它用例
        drop(ListIndexRefreshGuard(kind));
    }

    /// 索引规则版本闸门：落后时触发一次全量重解析，落版本号后不再重复触发。
    /// 没有这道闸门，按旧规则写下的元数据会被 (路径, 大小, mtime) 缓存永久固化。
    ///
    /// 版本必须按 CLI 独立：前端按 CLI 并发扇出，若共用一个全局键，先扫完的 CLI 会把
    /// 版本记成当前值，其余 CLI 随后读到「不落后」而跳过全量重建。
    #[test]
    fn session_index_revision_gates_full_rebuild_per_cli() {
        struct SettingCleanupGuard(String);
        impl Drop for SettingCleanupGuard {
            fn drop(&mut self) {
                if let Ok(conn) = app_db::conn() {
                    let _ = conn.execute(
                        "DELETE FROM app_settings WHERE key = ?1",
                        rusqlite::params![&self.0],
                    );
                }
            }
        }
        let _cleanup = SettingCleanupGuard(session_index_revision_key(CliKind::Dsh));

        mark_session_index_revision_current(CliKind::Dsh);
        mark_session_index_revision_current(CliKind::Claude);
        assert!(
            !session_index_revision_outdated(CliKind::Dsh),
            "落版本号后不应再强制全量重解析"
        );

        // 只把 DSH 的版本号回退成落后于当前规则的值
        app_db::write_setting_json(
            &session_index_revision_key(CliKind::Dsh),
            &(SESSION_INDEX_REVISION - 1),
        )
        .unwrap();
        assert!(
            session_index_revision_outdated(CliKind::Dsh),
            "版本落后时必须触发一次全量重解析以刷新存量元数据"
        );
        assert!(
            !session_index_revision_outdated(CliKind::Claude),
            "版本闸门必须按 CLI 独立，否则并发扇出时其余 CLI 拿不到全量重建"
        );

        mark_session_index_revision_current(CliKind::Dsh);
        assert!(!session_index_revision_outdated(CliKind::Dsh));
    }

    /// 空会话（只有 header + 策略事件、无 surface 消息）不进列表，
    /// 无 surfaced 消息时跳过该会话，避免空会话入库。
    #[test]
    fn dsh_scan_skips_session_without_surface_messages() {
        let _guard = DSH_SCAN_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_dsh_index_rows();
        let root = tempfile::tempdir().unwrap();
        let session_file = root
            .path()
            .join("sessions")
            .join("--Users-x-proj--")
            .join("s-empty")
            .join("session.jsonl.zstd");
        fs::create_dir_all(session_file.parent().unwrap()).unwrap();
        // 真实空会话形态（323B 案例）：header + permission/sandbox/approval，无聊天消息
        let content = concat!(
            "{\"type\":\"session\",\"version\":0,\"id\":\"s-empty\",\"createdAt\":1700000000000,\"cwd\":\"/Users/x/proj\"}\n",
            "{\"type\":\"permission/preset\",\"seq\":0,\"time\":1700000000001,\"data\":{\"preset\":\"danger-full-access\"}}\n",
            "{\"type\":\"sandbox/mode\",\"seq\":1,\"time\":1700000000002,\"data\":{\"mode\":\"danger-full-access\"}}\n",
            "{\"type\":\"approval/policy\",\"seq\":2,\"time\":1700000000003,\"data\":{\"policy\":\"never\"}}\n",
        );
        let compressed = zstd::encode_all(std::io::Cursor::new(content.as_bytes()), 3).unwrap();
        fs::write(&session_file, compressed).unwrap();

        let _override_guard = cli::CliDataDirOverrideGuard::set(CliKind::Dsh, root.path());

        let result = scan_projects_inner_for_cli(CliKind::Dsh, None, None, true).unwrap();
        clear_dsh_index_rows();

        assert_eq!(
            result.total_sessions, 0,
            "空会话（无 surface 消息）不应进列表"
        );
    }

    /// cwd-less header 回归：header 缺 `cwd` 时扫描按目录名解码兜底项目路径，
    /// 并回写持久化索引，快照加载（读索引）必须与实时扫描返回同一项目 key。
    #[test]
    fn dsh_scan_and_snapshot_agree_on_cwdless_header() {
        let _guard = DSH_SCAN_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_dsh_index_rows();
        let root = tempfile::tempdir().unwrap();
        let session_file = root
            .path()
            .join("sessions")
            .join("--Users-x-proj--")
            .join("s1")
            .join("session.jsonl.zstd");
        fs::create_dir_all(session_file.parent().unwrap()).unwrap();
        // 头部无 cwd：权威项目路径缺失，只能按目录名解码兜底
        let content = concat!(
            "{\"type\":\"session\",\"version\":0,\"id\":\"s1\",\"createdAt\":1700000000000}\n",
            "{\"type\":\"user/message\",\"seq\":1,\"time\":1700000000000,\"data\":{\"content\":[{\"type\":\"text\",\"text\":\"hi\"}]}}\n",
        );
        let compressed = zstd::encode_all(std::io::Cursor::new(content.as_bytes()), 3).unwrap();
        fs::write(&session_file, compressed).unwrap();

        let _override_guard = cli::CliDataDirOverrideGuard::set(CliKind::Dsh, root.path());

        let scan_result = scan_projects_inner_for_cli(CliKind::Dsh, None, None, true).unwrap();
        let snapshot_result = crate::cli_registry::source_for(CliKind::Dsh)
            .snapshot(&HashMap::new(), 0, 100)
            .unwrap()
            .expect("索引应有该行");
        clear_dsh_index_rows();

        // 目录名 --Users-x-proj-- 解码兜底（decode 保留 -- 产生的双斜杠）
        let expected_key = session_mod::decode_project_dir("--Users-x-proj--");
        assert_eq!(scan_result.projects.len(), 1);
        assert_eq!(scan_result.projects[0].encoded_dir.as_deref(), Some(expected_key.as_str()));
        assert_eq!(scan_result.projects[0].original_path.as_deref(), Some(expected_key.as_str()));

        assert_eq!(snapshot_result.projects.len(), 1);
        assert_eq!(
            snapshot_result.projects[0].encoded_dir.as_deref(), Some(expected_key.as_str()),
            "快照（读索引）必须回读到扫描兜底的项目 key"
        );
        assert_eq!(
            snapshot_result.projects[0].original_path.as_deref(), Some(expected_key.as_str()),
            "快照项目路径必须与扫描一致"
        );
    }

    /// `session_list_index.project_path` 的落库语义：这一列声明为 `TEXT`（允许 NULL），
    /// 所以「解析不出项目路径」落 `NULL` 而**不是任何占位串**。这条用例同时守住两端——
    /// 解析得出真实路径的会话必须仍带着那条真路径落库并被快照原样读回，解析不出的必须
    /// 落 `NULL` 且读回仍是 `None`。
    ///
    /// 判据取在**库列**上而不是界面文案上：占位串一旦落库，读取侧就再也分不出它和真路径，
    /// 前端会把它当成一个可以 `cd` 进去的目录（右键「在此新建会话」、屏蔽文件夹、
    /// 用量面板的项目名全都照它走）。
    #[test]
    fn codex_scan_and_snapshot_keep_absent_project_path_absent() {
        let _guard = CODEX_SCAN_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_codex_index_rows();
        let root = tempfile::tempdir().unwrap();
        // 路径里必须带 `/.codex/`：`cli_registry::kind_for_path` 按数据目录标记认 CLI，
        // 认不出来会落到兜底的 Claude 解析（夹具因此静默变空，用例退化成恒真）。
        let data_dir = root.path().join(".codex");
        let sessions_dir = data_dir.join("sessions");
        fs::create_dir_all(&sessions_dir).unwrap();

        // ① 头部带 cwd：权威项目路径
        fs::write(
            sessions_dir.join("rollout-2026-01-02T00-00-00-real.jsonl"),
            concat!(
                "{\"timestamp\":\"2026-01-02T00:00:00Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"s-real\",\"cwd\":\"/Users/x/real\"}}\n",
                "{\"timestamp\":\"2026-01-02T00:00:01Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"hi\"}]}}\n",
            ),
        )
        .unwrap();

        // ② 头部无 cwd、也没有 turn_context：项目路径解析不出来
        fs::write(
            sessions_dir.join("rollout-2026-01-02T00-00-01-absent.jsonl"),
            concat!(
                "{\"timestamp\":\"2026-01-02T00:00:00Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"s-absent\"}}\n",
                "{\"timestamp\":\"2026-01-02T00:00:01Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"hi\"}]}}\n",
            ),
        )
        .unwrap();

        // ③ 头部 cwd 是空白串：`session_meta` 分支与 `turn_context` 分支必须持同一条不变量，
        //    否则 `Some("")` 会绕过 `NULL` 的缺席语义落进这一列
        fs::write(
            sessions_dir.join("rollout-2026-01-02T00-00-02-blank.jsonl"),
            concat!(
                "{\"timestamp\":\"2026-01-02T00:00:00Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"s-blank\",\"cwd\":\"   \"}}\n",
                "{\"timestamp\":\"2026-01-02T00:00:01Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"hi\"}]}}\n",
            ),
        )
        .unwrap();

        let _override_guard = cli::CliDataDirOverrideGuard::set(CliKind::Codex, &data_dir);

        let scan_result = scan_projects_inner_for_cli(CliKind::Codex, None, None, true).unwrap();

        let stored = load_session_list_index(CliKind::Codex);
        let real = stored
            .values()
            .find(|record| record.session_id == "s-real")
            .expect("带 cwd 的会话应已落库");
        assert_eq!(
            real.project_path.as_deref(),
            Some("/Users/x/real"),
            "解析得出真实路径时必须原样落库"
        );
        let absent = stored
            .values()
            .find(|record| record.session_id == "s-absent")
            .expect("无 cwd 的会话也要落库，缺的只是项目路径");
        assert_eq!(
            absent.project_path, None,
            "解析不出项目路径时必须落 NULL，不能落占位串"
        );
        let blank = stored
            .values()
            .find(|record| record.session_id == "s-blank")
            .expect("空白 cwd 的会话也要落库");
        assert_eq!(
            blank.project_path, None,
            "空白 cwd 不是路径：必须落 NULL，不能落空串（空串是第三种缺席表达）"
        );

        let snapshot_result = crate::cli_registry::source_for(CliKind::Codex)
            .snapshot(&HashMap::new(), 0, 100)
            .unwrap()
            .expect("索引应有该行");

        clear_codex_index_rows();

        assert_eq!(scan_result.total_sessions, 3);
        // 实时扫描（前端首屏拿到的就是它）同样不许造占位值
        let mut scan_paths: Vec<Option<String>> = scan_result
            .projects
            .iter()
            .map(|project| project.original_path.clone())
            .collect();
        scan_paths.sort();
        assert_eq!(
            scan_paths,
            vec![None, Some("/Users/x/real".to_string())],
            "实时扫描必须与落库同形：真路径仍是真路径，缺席仍是 None"
        );
        // 两个「没有项目路径」的会话归入同一组（`None` 是分组键，与落库同形）
        let absent_group = scan_result
            .projects
            .iter()
            .find(|project| project.original_path.is_none())
            .expect("缺席组应存在");
        assert_eq!(absent_group.encoded_dir, None);
        assert_eq!(absent_group.sessions.len(), 2);

        let mut snapshot_paths: Vec<Option<String>> = snapshot_result
            .projects
            .iter()
            .map(|project| project.original_path.clone())
            .collect();
        snapshot_paths.sort();
        assert_eq!(
            snapshot_paths,
            vec![None, Some("/Users/x/real".to_string())],
            "快照（读索引）必须原样回读：真路径仍是真路径，缺席仍是 None"
        );
    }

    /// 真实 `~/.dsh/sessions` 冒烟：扫描不 panic、项目路径正确、可见会话数与磁盘一致。
    /// `--Users-...--` key 目录解码（去前后缀双斜杠）后的路径应能在结果 original_path 命中。
    /// 计数口径（与 DshSource::scan 同步）：header 可解析 && origin != "subagent"（子会话
    /// 不进侧边栏）&& 含 surface 消息（空会话过滤）。计数断言防止"静默丢会话测试全绿"。
    #[test]
    fn dsh_scan_smokes_against_real_sessions_dir() {
        let _guard = DSH_SCAN_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_dsh_index_rows();
        let sessions_dir = match cli::sessions_dir(CliKind::Dsh) {
            Ok(dir) if dir.exists() => dir,
            _ => return, // 机器无 ~/.dsh/sessions 时跳过
        };

        let result = scan_projects_inner_for_cli(CliKind::Dsh, None, None, true).unwrap();
        let session_count: usize = result
            .projects
            .iter()
            .map(|project| project.sessions.len())
            .sum();
        assert_eq!(result.total_sessions, session_count);

        for project in &result.projects {
            let original_path = project
                .original_path
                .as_deref()
                .expect("本 CLI 的项目路径来自目录名解码，不应缺席");
            assert!(
                Path::new(original_path).is_absolute(),
                "original_path 应为绝对路径: {}",
                original_path
            );
        }

        // 遍历磁盘会话文件，按扫描同口径算出期望可见集合（排除 subagent 与空会话）
        let mut expected_visible: HashMap<String, Vec<String>> = HashMap::new();
        let mut skipped_subagent = 0usize;
        let mut skipped_empty = 0usize;
        let mut skipped_bad_header = 0usize;
        for project_entry in fs::read_dir(&sessions_dir)
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
        {
            let project_dir = project_entry.path();
            if !project_dir.is_dir() {
                continue;
            }
            let key = project_entry
                .file_name()
                .to_str()
                .unwrap_or("")
                .to_string();
            for session_entry in fs::read_dir(&project_dir)
                .into_iter()
                .flatten()
                .filter_map(Result::ok)
            {
                let session_dir = session_entry.path();
                if !session_dir.is_dir() {
                    continue;
                }
                // 与扫描同口径取当前代文件：避免测试自己硬编码文件名，
                // 否则日志换代后测试会跟着扫描一起「看不见」新会话而失去交叉校验能力
                let Some(session_file) = crate::parser::dsh::session_dir_candidates(&session_dir)
                    .into_iter()
                    .next()
                else {
                    continue;
                };
                let Some(header) = crate::parser::dsh::read_header_line(&session_file) else {
                    skipped_bad_header += 1;
                    continue;
                };
                if header.get("origin").and_then(Value::as_str) == Some("subagent") {
                    skipped_subagent += 1;
                    continue;
                }
                if !crate::parser::dsh::has_surface_messages(&session_file) {
                    skipped_empty += 1;
                    continue;
                }
                expected_visible
                    .entry(key.clone())
                    .or_default()
                    .push(session_file.to_string_lossy().to_string());
            }
        }
        let expected_total: usize = expected_visible.values().map(Vec::len).sum();
        eprintln!(
            "[dsh smoke] visible={session_count} expected={expected_total} \
             skipped_subagent={skipped_subagent} skipped_empty={skipped_empty} \
             skipped_bad_header={skipped_bad_header}"
        );
        for project in &result.projects {
            eprintln!(
                "[dsh smoke]   {} -> {} 个会话",
                project.original_path.as_deref().unwrap_or("<缺席>"),
                project.sessions.len()
            );
        }
        assert_eq!(
            session_count, expected_total,
            "可见会话数必须等于磁盘上非 subagent 且含 surface 消息的会话数"
        );

        // 对照磁盘 key：含期望可见会话的 key 解码后应命中结果 original_path；
        // 只有 subagent/空会话的 key 可以不在结果中
        let found_paths: Vec<String> = result
            .projects
            .iter()
            .map(|project| {
                project
                    .original_path
                    .clone()
                    .unwrap_or_default()
                    .trim_matches('/')
                    .to_string()
            })
            .collect();
        for (key, files) in &expected_visible {
            if files.is_empty() {
                continue;
            }
            let normalized = session_mod::decode_project_dir(key)
                .trim_matches('/')
                .to_string();
            assert!(
                found_paths.contains(&normalized),
                "含可见会话的 key {key} 应解码命中 original_path {normalized}"
            );
        }

        // 清理冒烟扫描写入的 Dsh 索引行（Dsh 索引在接入前应为空）
        if let Ok(conn) = app_db::conn() {
            let _ = app_db::clear_session_index_inner(&conn, "dsh");
        }
    }

    #[test]
    fn test_candidate_fallback_on_corrupted_primary_file() {
        let dir = tempfile::tempdir().unwrap();
        let corrupt_path = dir.path().join("session-corrupt.jsonl");
        let fallback_path = dir.path().join("session-fallback.jsonl");

        // 构造损坏的主文件（0 字节）
        std::fs::write(&corrupt_path, b"").unwrap();
        // 构造正常的备选文件
        std::fs::write(&fallback_path, b"{\"type\":\"session\",\"version\":0,\"id\":\"fallback-id\",\"cwd\":\"/tmp\"}\n").unwrap();

        let candidates = vec![
            corrupt_path.clone(),
            fallback_path.clone(),
        ];

        // 验证降级读取首个有效文件
        let valid = pick_first_valid_candidate(&candidates, |p| {
            let meta = std::fs::metadata(p).ok()?;
            if meta.len() == 0 {
                return None;
            }
            Some(p.to_path_buf())
        });

        assert_eq!(valid, Some(fallback_path));
    }

    #[test]
    fn test_sort_session_candidates() {
        let mut candidates = vec![
            SessionCandidate {
                file_path: std::path::PathBuf::from("/backup/b.jsonl"),
                is_active_workspace: false,
                modified_ms: 100,
                file_size: 1000,
            },
            SessionCandidate {
                file_path: std::path::PathBuf::from("/active/a.jsonl"),
                is_active_workspace: true,
                modified_ms: 50,
                file_size: 500,
            },
            SessionCandidate {
                file_path: std::path::PathBuf::from("/backup/c.jsonl"),
                is_active_workspace: false,
                modified_ms: 200,
                file_size: 2000,
            },
        ];

        sort_session_candidates(&mut candidates);

        // 1. 活跃工作区优先
        assert_eq!(candidates[0].file_path, std::path::PathBuf::from("/active/a.jsonl"));
        // 2. 备份中修改时间较新（200 > 100）优先
        assert_eq!(candidates[1].file_path, std::path::PathBuf::from("/backup/c.jsonl"));
        assert_eq!(candidates[2].file_path, std::path::PathBuf::from("/backup/b.jsonl"));
    }

    /// 累加器契约测试共享真实 app DB 的 Gemini 索引行，串行化避免 flaky。
    static ACCUMULATOR_TEST_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

    fn clear_gemini_index_rows() {
        if let Ok(conn) = app_db::conn() {
            let _ = app_db::clear_session_index_inner(&conn, "gemini");
        }
    }

    /// `upsert_session_list_index` 刻意不写归档列（`has_archive_snapshot` 由 `archived_at`
    /// 派生，只归归档流程维护），因此测试要构造「库中已有归档行」，只能自己落 `archived_at`。
    fn mark_gemini_row_archived(key: &str) {
        if let Ok(conn) = app_db::conn() {
            let _ = conn.execute(
                "UPDATE session_list_index SET archived_at = ?1 \
                 WHERE cli_id = ?2 AND session_path = ?3",
                rusqlite::params!["2024-01-01T00:00:00Z", "gemini", key],
            );
        }
    }

    /// `force` 只清空**复用视图**，不清空**已知视图**。可观测的后果是展示状态的回落：
    /// 非 force 时从库中已有行取归档标记，force 时该行不再被复用，标记因此不回落。
    /// 两个视图合成一份的后果是双向的 —— 合成「恒复用」会让强制刷新拿不到新状态，
    /// 合成「恒不复用」会让归档标记在每次刷新后丢失。
    #[test]
    fn accumulator_force_stops_reusing_archived_state() {
        let _guard = ACCUMULATOR_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        clear_gemini_index_rows();
        let key = "/tmp/seshbuddy-accumulator-contract/session.jsonl";
        let record = list_record(key, 0);
        app_db::upsert_session_list_index(CliKind::Gemini, &[record]).unwrap();
        mark_gemini_row_archived(key);
        let names = HashMap::new();

        let metadata = SessionListMetadata {
            session_id: key.to_string(),
            project_path: None,
            title: None,
            first_user_message: None,
            first_timestamp: None,
            last_timestamp: None,
            git_branch: String::new(),
            file_size: 1,
        };
        let projection = || ScanProjection {
            project_path: None,
            encoded_dir: None,
            original_path: None,
            title: None,
            history_display: None,
            git_branch: String::new(),
        };

        let mut cached = ScanAccumulator::new(CliKind::Gemini, &names, false).unwrap();
        cached.push_session(key, metadata.clone(), projection());
        let reused = cached.finish(&[]).unwrap();
        assert_eq!(reused.len(), 1);
        assert!(
            reused[0].session.has_archive_snapshot,
            "非 force：应从库中已有行回落归档状态"
        );

        let mut forced = ScanAccumulator::new(CliKind::Gemini, &names, true).unwrap();
        forced.push_session(key, metadata, projection());
        let forced_sessions = forced.finish(&[]).unwrap();
        assert_eq!(forced_sessions.len(), 1);
        assert!(
            !forced_sessions[0].session.has_archive_snapshot,
            "force：不复用库中行，归档状态不回落"
        );

        clear_gemini_index_rows();
    }

    /// 展示路径必须取**纠正后**的项目路径。会话文件里的 cwd 与 history 映射指向同一个
    /// 编码目录、但字符串不同（下划线 vs 连字符）：纠正值来自 history 映射，展示路径也应
    /// 是它。若误用纠正前的元数据原值，实时扫描会显示 cwd 而快照（读索引）显示映射值，
    /// 同一会话出现两个路径。这是六个源里唯一「投影 project_path 与元数据原值不同」的分叉点。
    #[test]
    fn claude_scan_original_path_takes_corrected_project_path() {
        let _guard = CLAUDE_SCAN_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_claude_index_rows();
        let root = tempfile::tempdir().unwrap();
        let data_dir = root.path().join(".claude");
        let session_dir = data_dir.join("projects").join("-Users-x-my-proj");
        fs::create_dir_all(&session_dir).unwrap();
        // cwd 用下划线、history project 用连字符：两者编码到同一个目录名。
        fs::write(
            session_dir.join("s1.jsonl"),
            concat!(
                "{\"type\":\"user\",\"cwd\":\"/Users/x/my_proj\",\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"text\",\"text\":\"hi\"}]}}\n",
            ),
        )
        .unwrap();
        fs::write(
            data_dir.join("history.jsonl"),
            concat!("{\"sessionId\":\"s1\",\"display\":\"d\",\"project\":\"/Users/x/my-proj\"}\n"),
        )
        .unwrap();

        let _override_guard = cli::CliDataDirOverrideGuard::set(CliKind::Claude, &data_dir);

        let scan_result = scan_projects_inner_for_cli(CliKind::Claude, None, None, true).unwrap();
        let snapshot_result = crate::cli_registry::source_for(CliKind::Claude)
            .snapshot(&HashMap::new(), 0, 100)
            .unwrap()
            .expect("索引应有该行");

        clear_claude_index_rows();

        assert_eq!(scan_result.projects.len(), 1);
        assert_eq!(
            scan_result.projects[0].original_path.as_deref(),
            Some("/Users/x/my-proj"),
            "展示路径必须取 history 映射纠正后的值，而不是会话文件里的 cwd"
        );
        assert_eq!(
            snapshot_result.projects[0].original_path.as_deref(),
            Some("/Users/x/my-proj"),
            "快照（读索引）必须与实时扫描显示同一路径"
        );
    }

    /// 库型源（OpenCode）的会话身份是虚拟键，不是文件路径。索引构建的存在性判定必须
    /// 问源本身，若仍用 `Path::exists()` 则虚拟键恒判不存在，检索对库型源整体失效。
    ///
    /// 真实库不存在就跳过 —— 不能假设 CI 上装了 OpenCode。全程只读，不碰用户数据。
    /// 与读写 OpenCode 数据目录覆盖的测试共用同一把锁，避免读到被改过的数据目录。
    #[test]
    fn opencode_virtual_key_reaches_search_docs_scan() {
        let _guard = crate::cli::OPENCODE_DATA_DIR_OVERRIDE_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        let Ok(dir) = cli::data_dir(CliKind::Opencode) else { return };
        let db_path = dir.join("opencode.db");
        if !db_path.exists() {
            return;
        }

        // 挑一个确实带文本 part 的会话，保证检索文档非空。
        let conn = rusqlite::Connection::open_with_flags(
            &db_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .expect("只读打开真实 OpenCode 库");
        let session_id: String = conn
            .query_row(
                "SELECT session_id FROM part WHERE json_extract(data, '$.type') = 'text' \
                 GROUP BY session_id ORDER BY count(*) DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .expect("库里至少有一个带文本 part 的会话");
        drop(conn);

        let key = crate::cli_registry::SessionLocator::Virtual {
            cli_id: CliKind::Opencode,
            key: session_id,
        }
        .to_key();
        assert!(key.starts_with("cli://opencode/"), "库型会话应是虚拟键: {key}");

        let record = list_record(&key, 0);
        let docs = scan_session_search_docs_for_index(CliKind::Opencode, &key, &record, |_| {});

        let docs = docs.expect("虚拟键会话应经源的存在性判定后走到 search_docs");
        assert!(!docs.is_empty(), "带文本 part 的会话应产出至少一条检索文档");
    }

    /// 库型源（OpenCode）端到端验收：真实库存在时，**真实入口**必须能看到库里的会话。
    ///
    /// 与上一条的分工：上一条把合成索引记录直接喂给 `scan_session_search_docs_for_index`，
    /// 绕过了「索引怎么被填」与「CLI 怎么被判可见」这两段，所以 `scan` / `has_sessions`
    /// 还是占位时它也照样通过。本条只走真实入口：
    /// - 列表扫描 `scan_projects_inner_for_cli`；
    /// - 可见性判定 `cli::has_sessions`；
    /// - 检索候选 `resolve_search_index_status`（`get_search_index_status` 的实现）。
    ///
    /// 三处任一仍是占位，条数就与 `session` 表对不上。真实库不存在就跳过 ——
    /// 不能假设 CI 上装了 OpenCode。全程只读 OpenCode 库，只写本应用索引且前后各清一次。
    /// 与改 OpenCode 数据目录覆盖的测试共用同一把锁，避免读到被改过的数据目录。
    #[test]
    fn opencode_end_to_end_list_visibility_and_search_candidates() {
        let _guard = crate::cli::OPENCODE_DATA_DIR_OVERRIDE_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        let Ok(dir) = cli::data_dir(CliKind::Opencode) else { return };
        let db_path = dir.join("opencode.db");
        if !db_path.exists() {
            return;
        }

        let expected: i64 = {
            let conn = rusqlite::Connection::open_with_flags(
                &db_path,
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
            )
            .expect("只读打开真实 OpenCode 库");
            conn.query_row("SELECT count(*) FROM session", [], |row| row.get(0))
                .expect("统计 session 行数")
        };
        assert!(expected > 0, "库存在却没有任何会话，无法验证端到端路径");

        // 先清索引：让可见性必须由源自己判定（索引为空时才会走到 discover 分支），
        // 也让随后的扫描从干净状态重建。
        clear_opencode_index_rows();

        assert!(
            cli::has_sessions(CliKind::Opencode),
            "索引为空时，有会话的库型源必须被判为可见，否则前端会把它整块过滤掉"
        );

        let scanned = scan_projects_inner_for_cli(CliKind::Opencode, None, None, true)
            .expect("扫描真实 OpenCode 库");
        assert_eq!(
            scanned.total_sessions as i64, expected,
            "列表扫描条数应等于 session 表行数"
        );
        assert!(!scanned.projects.is_empty(), "扫描结果应至少有一个项目分组");

        // 检索候选：列表索引刚被扫描填好、检索索引尚空，候选数应等于会话数 ——
        // 即 `search_docs` 在生产路径上真的会被调用，而不是只有直接调用时才可达。
        let status = resolve_search_index_status(CliKind::Opencode).expect("读取检索索引状态");
        assert_eq!(
            status.total_sessions as i64, expected,
            "检索状态里的总会话数应与库一致"
        );
        assert_eq!(
            status.missing_sessions as i64, expected,
            "检索索引为空时全部会话都应是待索引候选，否则 search_docs 永远不会被触发"
        );

        clear_opencode_index_rows();
    }

    /// 子会话可见性回归：库型源的 `scan` 与快照两条装配路径必须给出同一批会话。
    ///
    /// 真实库当前不含任何子会话（`parent_id IS NOT NULL` 的行数为 0），在真库上两条
    /// 路径天然一致，分辨不出「扫描漏滤子会话」这个分歧；故此处显式造一个临时库，
    /// 放入一条顶层会话与一条 `parent_id` 指向它的子会话，断言才可能变红。
    ///
    /// 若 `scan` 不按 `parent_id` 过滤，子会话会经扫描写进列表索引：扫描路径能看到它，
    /// 而快照路径会经 `is_subagent` 把它挡在列表外，两条路径的可见集合与
    /// `total_sessions` 随之分叉 —— 正是本用例要钉住的行为。
    #[test]
    fn opencode_scan_and_snapshot_agree_on_sub_sessions() {
        let _guard = crate::cli::OPENCODE_DATA_DIR_OVERRIDE_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        // 临时库：只建 `scan` / `is_subagent` 用到的列，schema 与真实库一致。
        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("opencode.db");
        {
            let db = rusqlite::Connection::open(&db_path).unwrap();
            db.execute_batch(
                "CREATE TABLE session (\
                   id TEXT PRIMARY KEY, project_id TEXT NOT NULL, parent_id TEXT,\
                   slug TEXT NOT NULL, directory TEXT NOT NULL, title TEXT NOT NULL,\
                   version TEXT NOT NULL, time_created INTEGER NOT NULL, time_updated INTEGER NOT NULL\
                 );",
            )
            .unwrap();
            db.execute(
                "INSERT INTO session \
                 (id, project_id, slug, directory, title, version, time_created, time_updated) \
                 VALUES ('ses_regr_sub_parent', 'proj', 'slug', '/tmp/proj', 'parent', '1', 1000, 2000)",
                [],
            )
            .unwrap();
            db.execute(
                "INSERT INTO session \
                 (id, project_id, parent_id, slug, directory, title, version, time_created, time_updated) \
                 VALUES ('ses_regr_sub_child', 'proj', 'ses_regr_sub_parent', 'slug', '/tmp/proj', 'child', '1', 1000, 2000)",
                [],
            )
            .unwrap();
        }

        let _override_guard = cli::CliDataDirOverrideGuard::set(CliKind::Opencode, temp.path());
        clear_opencode_index_rows();

        // 真实入口：扫描先把索引写满，快照再读同一份索引 —— 两条装配路径。
        let scanned = scan_projects_inner_for_cli(CliKind::Opencode, None, None, true)
            .expect("扫描临时 OpenCode 库");
        let snapshot = crate::cli_registry::source_for(CliKind::Opencode)
            .snapshot(&HashMap::new(), 0, 100)
            .expect("读取列表索引")
            .expect("扫描已写入索引，快照不应缺席");

        clear_opencode_index_rows();

        let scan_ids: Vec<String> = scanned
            .projects
            .iter()
            .flat_map(|project| project.sessions.iter().map(|s| s.session_id.clone()))
            .collect();
        let snapshot_ids: Vec<String> = snapshot
            .projects
            .iter()
            .flat_map(|project| project.sessions.iter().map(|s| s.session_id.clone()))
            .collect();

        assert_eq!(
            scan_ids,
            vec!["ses_regr_sub_parent".to_string()],
            "扫描只应看到顶层会话，子会话不得写进列表索引"
        );
        assert_eq!(
            snapshot_ids, scan_ids,
            "快照与扫描的可见会话集合必须一致"
        );
        assert_eq!(scanned.total_sessions, 1, "总数不得把子会话算进去");
        assert_eq!(snapshot.total_sessions, 1, "快照总数同样不得含子会话");
    }
