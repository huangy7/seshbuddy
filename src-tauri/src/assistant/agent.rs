use serde::Serialize;

/// 一轮的 token 用量（来自 stream-json result 行的 usage 字段；旧版 CLI 可能缺失）
#[derive(Debug, Clone, Copy, Serialize, Default, PartialEq)]
pub struct TurnUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_input_tokens: u64,
    pub cache_creation_input_tokens: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum AgentStreamEvent {
    Init { session_id: String, model: Option<String> },
    AssistantText { text: String },
    ToolUse { name: String, summary: String },
    Result {
        ok: bool,
        duration_ms: Option<u64>,
        cost_usd: Option<f64>,
        usage: Option<TurnUsage>,
        /// CLI 自己的失败说明（result 行的 `result` 键，缺失或为空白时回落 `errors[0]`），
        /// **不是** SeshBuddy 的文案。
        ///
        /// 它按 R3 包成 `assistant.cli_error` 的 `detail` 参数：文案本身是子进程的英文原文，
        /// **不翻译、不进语言包**，包一层只为了给出归属——否则用户看到一句没有出处的英文，
        /// 分不清是 CLI 拒绝了这次请求还是 SeshBuddy 自己坏了。类型是 `AppError` 而不是
        /// `String`，与另外两条错误通道共用一种线格式（`{code, params}`），前端
        /// `renderAppError` 两种形状都认。
        error: Option<AppError>,
    },
}

/// 解析单行 stream-json（claude -p --output-format stream-json 的输出）。
/// 非 JSON 行或不关心的类型返回 None。
pub fn parse_stream_line(line: &str) -> Option<AgentStreamEvent> {
    let parsed: serde_json::Value = serde_json::from_str(line).ok()?;
    let event_type = parsed.get("type")?.as_str()?;

    match event_type {
        "system" => {
            if parsed.get("subtype")?.as_str()? != "init" {
                return None;
            }
            let session_id = parsed.get("session_id")?.as_str()?.to_string();
            let model = parsed
                .get("model")
                .and_then(|m| m.as_str())
                .map(String::from);
            Some(AgentStreamEvent::Init { session_id, model })
        }
        "assistant" => {
            let content = parsed.get("message")?.get("content")?.as_array()?;
            for part in content {
                match part.get("type").and_then(|t| t.as_str()) {
                    Some("text") => {
                        if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                            if !text.trim().is_empty() {
                                return Some(AgentStreamEvent::AssistantText {
                                    text: text.to_string(),
                                });
                            }
                        }
                    }
                    Some("tool_use") => {
                        let Some(name) = part.get("name").and_then(|n| n.as_str()) else {
                            continue;
                        };
                        let name = name.to_string();
                        let summary = part
                            .get("input")
                            .and_then(|i| i.get("command"))
                            .and_then(|c| c.as_str())
                            .map(|c| c.chars().take(120).collect())
                            .unwrap_or_default();
                        return Some(AgentStreamEvent::ToolUse { name, summary });
                    }
                    _ => continue,
                }
            }
            None
        }
        "result" => {
            // 成功判据必须同时看 `is_error`：实测 CLI 2.1.246 在密钥无效 / 限流 / 模型不存在时
            // 退出码为 1，`subtype` 却仍是 `"success"`，只有 `is_error:true` 与
            // `terminal_reason:"api_error"` 标记了失败。只看 `subtype` 会把它判成成功，
            // 于是正文被丢掉、下游 `saw_result` 又跳过 stderr 兜底，用户这一轮静默结束。
            // 缺失 `is_error` 视为非错误，只有显式 `true` 才算失败。
            let ok = parsed.get("subtype").and_then(|s| s.as_str()) == Some("success")
                && parsed.get("is_error").and_then(|e| e.as_bool()) != Some(true);
            let usage = parsed.get("usage").map(|u| TurnUsage {
                input_tokens: u.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
                output_tokens: u.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
                cache_read_input_tokens: u.get("cache_read_input_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
                cache_creation_input_tokens: u.get("cache_creation_input_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
            });
            Some(AgentStreamEvent::Result {
                ok,
                duration_ms: parsed.get("duration_ms").and_then(|d| d.as_u64()),
                cost_usd: parsed.get("total_cost_usd").and_then(|c| c.as_f64()),
                usage,
                // 失败正文有两个可能的落点：`api_error` 一族（subtype 仍是 success）把正文放在
                // `result` 字符串里；`error_*` 一族（实测 4 种 subtype / 11 个构造点）则一律
                // **不带** `result` 键，正文只在 `errors:[…]` 里。两者都取，优先 `result`。
                // `result` 为空白串时视同没有：空串不是「CLI 的失败说明」，若就此短路，
                // 用户会看到一句只有前缀、真正文（`errors[0]`）被吞掉的提示——正是本次要修的
                // 「静默丢消息」同一类。故非空才认，空白则回落 `errors[0]`。
                error: if ok {
                    None
                } else {
                    parsed
                        .get("result")
                        .and_then(|r| r.as_str())
                        .filter(|s| !s.trim().is_empty())
                        .or_else(|| {
                            parsed
                                .get("errors")
                                .and_then(|e| e.as_array())
                                .and_then(|a| a.first())
                                .and_then(|v| v.as_str())
                        })
                        .map(|text| AppError::coded("assistant.cli_error").with("detail", text))
                },
            })
        }
        _ => None,
    }
}

use crate::error::{AppError, AppResult};
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, ExitStatus, Stdio};

/// 截断字符串尾部，保留最多 max 字节（UTF-8 边界安全）。
fn truncate_tail(s: &mut String, max: usize) {
    if s.len() > max {
        let keep_from = (s.len() - max..=s.len())
            .find(|&i| s.is_char_boundary(i))
            .unwrap_or(s.len());
        *s = s.split_off(keep_from);
    }
}

/// pump_stream 的返回结果。
pub struct PumpOutcome {
    pub stderr_tail: String,
    pub status: ExitStatus,
}

/// proxy 二进制名（unix 无扩展名，Windows 带 .exe）。allow 规则与提示词用裸名，
/// 避免 Windows 路径反斜杠导致 Bash 规则匹配失败（claude-code 权限 glob 把 `\` 当转义符）。
/// 按 `/` 和 `\` 都切分，保证跨平台取到最后一个路径段。
fn proxy_binary_name(proxy_abs: &str) -> String {
    proxy_abs
        .rsplit(['/', '\\'])
        .next()
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| proxy_abs.to_string())
}

/// 助手系统提示词。
///
/// **刻意保持英文，不是待翻译的文案**：模型对英文指令的遵循更好，而这份提示词只给模型看、
/// 用户看不到（与 proxy 的 `--json-help` 同理）。多语言由最后一条「用用户的语言作答」覆盖，
/// 因此不需要维护四份提示词，它也不进语言包。
///
/// 改写文案时**不得改动命令名与参数名**（`--json-help`、list/grep/show、`--from`/`--to`/
/// `--max-chars`/`--format`）：那些是模型要执行的命令，改一个字符工具就调不通。
/// 来源标记 `⟦N:…⟧` 与追问标记 `«FOLLOWUPS: …»` 的符号同样由前端解析，也不得改动。
pub fn build_system_prompt(proxy_abs: &str) -> String {
    let name = proxy_binary_name(proxy_abs);
    [
        "You are the SeshBuddy assistant: you help users query and understand their AI coding session history.",
        &format!("Your only tool is seshbuddy-proxy (already allowlisted): run `{} --json-help` first to learn the full command set.", name),
        "Common flow: list to filter sessions by time/project -> grep to locate content -> show to read the body (use --from/--to for a slice instead of reading a whole long session at once).",
        "For show, pass only the first 8 characters of the id (never restate the full UUID, it invites mistakes); add --format text when you only need the body.",
        "grep results carry a coverage field; when it is partial, say that only part of the sessions were searched.",
        "show extracts session content on demand; an empty text field means that session has nothing extractable (or its source file was cleaned up): skip it and note that, do not retry repeatedly.",
        "Do not pipe or redirect proxy commands (e.g. 2>/dev/null, | head, | python3): to read part of a body, use show's own --from/--to/--max-chars flags.",
        "[SOURCE ATTRIBUTION REQUIRED] Whenever your answer mentions the content, conclusions or details of a session, the sentence must end with a source marker, format `⟦N:first 8 characters of the session id⟧` (double brackets; N starts at 1 and increments; reuse the same N for the same source).",
        "Those first 8 characters must be copied verbatim from the id field you saw in this turn's show/grep output, not one character changed, and never invented.",
        "Example: if you showed the session with id 90259911-..., write \"that session split the cache layer ⟦1:90259911⟧\"; if you also showed 3f8a9c2d-..., write \"another spot optimized startup ⟦2:3f8a9c2d⟧\".",
        "Mark sparingly: when several consecutive sentences in a paragraph come from the same source, mark it once at the end of the last sentence using that source, do not repeat the marker on every sentence.",
        "[DYNAMIC FOLLOW-UP SUGGESTIONS] At the very end of every answer, on a new line, append 2-3 closely related follow-up questions or next-step suggestions chosen from the current conversation context and your answer, strictly in the format `«FOLLOWUPS: suggestion one | suggestion two | suggestion three»` (each suggestion short and clear, about 10 characters or the equivalent few words in other languages, generated by you from the context; if no follow-up is needed, omit this tag entirely).",
        "Answer in the user's language, i.e. the language of the user's latest message. Be concise and direct; state what any numbers you give are counting; if you cannot find something, say so, never make it up.",
    ].join("\n")
}

pub fn build_spawn_args(
    proxy_abs: &str,
    prompt: &str,
    settings_file: &str,
    system_prompt: &str,
    resume_session_id: Option<&str>,
    model: Option<&str>,
) -> Vec<String> {
    let mut args = vec![
        "-p".to_string(),
        prompt.to_string(),
        "--output-format".to_string(),
        "stream-json".to_string(),
        "--verbose".to_string(),
        "--allowedTools".to_string(),
        format!("Bash({}:*)", proxy_binary_name(proxy_abs)),
        "--settings".to_string(),
        settings_file.to_string(),
        "--append-system-prompt".to_string(),
        system_prompt.to_string(),
    ];
    if let Some(id) = resume_session_id {
        args.push("--resume".to_string());
        args.push(id.to_string());
    }
    if let Some(m) = model.filter(|m| !m.is_empty()) {
        args.push("--model".to_string());
        args.push(m.to_string());
    }
    args
}

pub struct RunningTurn {
    pub child: Child,
}

/// spawn 一轮 agent。主进程 PATH 已在启动时注入完整 shell 环境（pty_manager::
/// inject_shell_env_into_process），Command 直接继承，无需额外处理。
pub fn spawn_turn(
    claude_bin: &str,
    proxy_abs: &str,
    prompt: &str,
    settings_file: &str,
    resume_session_id: Option<&str>,
    model: Option<&str>,
    workspace: &std::path::Path,
) -> AppResult<RunningTurn> {
    let system_prompt = build_system_prompt(proxy_abs);
    let args = build_spawn_args(proxy_abs, prompt, settings_file, &system_prompt, resume_session_id, model);
    let mut cmd = Command::new(claude_bin);
    cmd.args(&args)
        .current_dir(workspace)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // claude 是控制台子程序：Windows 上必须加 CREATE_NO_WINDOW，否则 GUI 主进程
    // 拉起 claude 时会额外弹一个控制台黑框（与 cli.rs / proxy.rs 保持相同的 CREATE_NO_WINDOW 窗口标志处理）。
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    // allow 规则是裸名 Bash(seshbuddy-proxy:*)，agent 按名调用才能命中：
    // 把 proxy 所在目录 prepend 进子进程 PATH（Windows 分隔符 ';'，unix 是 ':'）。
    if let Some(dir) = std::path::Path::new(proxy_abs).parent() {
        let sep = if cfg!(windows) { ";" } else { ":" };
        let existing = std::env::var("PATH").unwrap_or_default();
        let merged = if existing.is_empty() {
            dir.display().to_string()
        } else {
            format!("{}{}{}", dir.display(), sep, existing)
        };
        cmd.env("PATH", merged);
    }
    let child = cmd
        .spawn()
        .map_err(|e| AppError::coded("assistant.agent_start_failed").with("detail", e.to_string()))?;
    Ok(RunningTurn { child })
}

/// 读 stdout 逐行解析，回调事件；同时收集 stderr 尾部（错误诊断）。
/// 结束后 wait 回收子进程，避免僵尸。
pub fn pump_stream<F: FnMut(AgentStreamEvent)>(
    turn: &mut RunningTurn,
    mut on_event: F,
) -> AppResult<PumpOutcome> {
    let stdout = turn.child.stdout.take()
        .ok_or_else(|| AppError::coded("assistant.agent_stdout_missing"))?;
    let stderr = turn.child.stderr.take()
        .ok_or_else(|| AppError::coded("assistant.agent_stderr_missing"))?;

    // stderr 独立线程收集（尾部 400 字节用于诊断）
    let stderr_handle = std::thread::spawn(move || {
        let mut tail = String::new();
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            tail.push_str(&line);
            tail.push('\n');
            truncate_tail(&mut tail, 400);
        }
        tail
    });

    for line in BufReader::new(stdout).lines().map_while(Result::ok) {
        if let Some(event) = parse_stream_line(&line) {
            on_event(event);
        }
    }

    let tail = match stderr_handle.join() {
        Ok(v) => v,
        Err(_) => {
            tracing::warn!("agent stderr 收集线程异常");
            String::new()
        }
    };

    let status = turn
        .child
        .wait()
        .map_err(|e| AppError::coded("assistant.agent_wait_failed").with("detail", e.to_string()))?;

    Ok(PumpOutcome {
        stderr_tail: tail,
        status,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_init_event() {
        let line = r#"{"type":"system","subtype":"init","session_id":"sess-123","model":"claude-sonnet-4-6"}"#;
        let event = parse_stream_line(line).unwrap();
        match event {
            AgentStreamEvent::Init { session_id, model } => {
                assert_eq!(session_id, "sess-123");
                assert_eq!(model.as_deref(), Some("claude-sonnet-4-6"));
            }
            _ => panic!("expected init"),
        }
    }

    #[test]
    fn parses_assistant_text() {
        let line = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"你好"}]}}"#;
        let event = parse_stream_line(line).unwrap();
        match event {
            AgentStreamEvent::AssistantText { text } => assert_eq!(text, "你好"),
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn parses_tool_use_with_command_summary() {
        let line = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash","input":{"command":"seshbuddy-proxy list --days 7"}}]}}"#;
        let event = parse_stream_line(line).unwrap();
        match event {
            AgentStreamEvent::ToolUse { name, summary } => {
                assert_eq!(name, "Bash");
                assert!(summary.contains("seshbuddy-proxy list"));
            }
            _ => panic!("expected tool_use"),
        }
    }

    #[test]
    fn parses_result_success_with_cost() {
        let line = r#"{"type":"result","subtype":"success","duration_ms":1234,"total_cost_usd":0.05}"#;
        let event = parse_stream_line(line).unwrap();
        match event {
            AgentStreamEvent::Result {
                ok,
                duration_ms,
                cost_usd,
                ..
            } => {
                assert!(ok);
                assert_eq!(duration_ms, Some(1234));
                assert_eq!(cost_usd, Some(0.05));
            }
            _ => panic!("expected result"),
        }
    }

    #[test]
    fn mixed_content_skips_malformed_text_part() {
        let line = r#"{"type":"assistant","message":{"content":[{"type":"text"},{"type":"tool_use","name":"Bash","input":{"command":"ls"}}]}}"#;
        assert!(matches!(parse_stream_line(line), Some(AgentStreamEvent::ToolUse { .. })));
    }

    /// 通道 1 的契约守卫：CLI 自己的失败说明以 `assistant.cli_error` 的 `detail` 过线。
    ///
    /// **为什么必须有**：字段类型是 `AppError` 而不是 `String`，而改回 `Option<String>` 一样
    /// 编译得过（`.map(String::from)` 同样成立），线格式却从 `{code, params}` 退回裸字符串——
    /// 前端 `renderAppError` 便不再知道那句话来自 CLI，用户看到的又成了一句无出处的英文。
    /// 故这里把类型与线上形状一并钉住：类型改回去，`assert_error_type` 先编译不过。
    #[test]
    fn result_error_is_coded_with_cli_text_as_detail() {
        fn assert_error_type(_: &Option<AppError>) {}

        // 合成形状：`subtype` 非 success 且**带** `result` 键。实测 CLI 2.1.246 的 `error_*` 一族
        // 一律不带 `result`（正文只在 `errors:[…]`，见下面 `error_max_turns` 用例），故此形状
        // 本身并非线上实测。保留它是为了单独钉住 `result` 路径的取值与线格式，
        // 与下面两个真实形状的用例互补——它不再是本函数唯一能填出 error 的形状。
        // **整行皆为构造值**：`subtype`、正文文本都没有逐字捕获。
        let line = r#"{"type":"result","subtype":"error_during_execution","result":"API Error: 400 bad model"}"#;
        let event = parse_stream_line(line).unwrap();
        let AgentStreamEvent::Result { ok, error, .. } = event else {
            panic!("expected result");
        };
        assert!(!ok);
        assert_error_type(&error);

        // CLI 原文逐字进 `detail`：它是子进程的输出，按 R3 不翻译、不进语言包。
        let json = serde_json::to_value(error.unwrap()).unwrap();
        assert_eq!(json["code"], "assistant.cli_error");
        assert_eq!(json["params"]["detail"], "API Error: 400 bad model");
    }

    /// 实测形状一（CLI 2.1.246，`--model <不存在>`）：退出码 1，`subtype` 却仍是 `"success"`，
    /// 失败只由 `is_error:true` + `terminal_reason:"api_error"` 标记，正文在 `result` 键里。
    /// 只看 `subtype` 的判据会把它判成成功，正文虽在线上却被丢掉，而下游 `saw_result` 又跳过
    /// stderr 兜底——用户这一轮**静默结束**。本用例钉住修复后的行为：判失败，且原文逐字进 `detail`。
    ///
    /// **键名与取值类型实测**（`subtype:"success"` + `is_error:true` + `terminal_reason:"api_error"`
    /// + 正文在 `result`）；**`result` 内的具体报错文本与 `duration_ms:812` 是构造值**，
    /// 未逐字捕获——钉的是判据与取数路径，不是那句正文的拼写。
    #[test]
    fn api_error_line_is_failure_with_result_text_as_detail() {
        const DETAIL: &str = r#"API Error: 400 {"type":"error","error":{"type":"invalid_request_error","message":"model: claude-nonexistent"}}"#;
        let line = r#"{"type":"result","subtype":"success","is_error":true,"terminal_reason":"api_error","duration_ms":812,"result":"API Error: 400 {\"type\":\"error\",\"error\":{\"type\":\"invalid_request_error\",\"message\":\"model: claude-nonexistent\"}}"}"#;

        let AgentStreamEvent::Result { ok, error, .. } = parse_stream_line(line).unwrap() else {
            panic!("expected result");
        };
        assert!(!ok);
        let json = serde_json::to_value(error.unwrap()).unwrap();
        assert_eq!(json["code"], "assistant.cli_error");
        assert_eq!(json["params"]["detail"], DETAIL);
    }

    /// 实测形状二（CLI 2.1.246，`--max-turns 1` + 多轮提示词）：`subtype:"error_max_turns"`、
    /// `is_error:true`，正文只在 `errors:[…]` 里，且**没有** `result` 键。修复前只读 `result`，
    /// `errors[0]` 的正文拿不到，用户只剩前端兜底文案；现在取它作 `detail`。
    ///
    /// **整行逐字捕获**：`subtype`、`is_error`、`errors[0]` 的文本都来自真实二进制输出。
    #[test]
    fn error_max_turns_line_is_failure_with_errors_text_as_detail() {
        let line = r#"{"type":"result","subtype":"error_max_turns","is_error":true,"errors":["Reached maximum number of turns (1)"]}"#;

        let AgentStreamEvent::Result { ok, error, .. } = parse_stream_line(line).unwrap() else {
            panic!("expected result");
        };
        assert!(!ok);
        let json = serde_json::to_value(error.unwrap()).unwrap();
        assert_eq!(json["code"], "assistant.cli_error");
        assert_eq!(json["params"]["detail"], "Reached maximum number of turns (1)");
    }

    /// 空白 `result` 不得短路 `errors[0]`：若认空串为正文，用户只会看到一句只有前缀、
    /// 真正文被吞掉的提示——正是本次修复要消灭的「静默丢消息」同类。空白 `result` 必须视同缺失。
    #[test]
    fn blank_result_falls_back_to_errors_text() {
        for line in [
            r#"{"type":"result","subtype":"error_during_execution","is_error":true,"result":"","errors":["E"]}"#,
            r#"{"type":"result","subtype":"error_during_execution","is_error":true,"result":"   ","errors":["E"]}"#,
        ] {
            let AgentStreamEvent::Result { ok, error, .. } = parse_stream_line(line).unwrap() else {
                panic!("expected result");
            };
            assert!(!ok);
            let json = serde_json::to_value(error.unwrap()).unwrap();
            assert_eq!(json["params"]["detail"], "E", "{line}");
        }
    }

    /// 反向控制：修复不能把一切轮次都判成失败。
    ///
    /// 两个成功形状必须保持 `ok=true` / `error=None`，否则前端会拿一句 CLI 的英文顶掉正常回答：
    /// - 纯成功行（无 `is_error`）—— 缺失视为非错误，只有显式 `true` 才算失败；
    /// - 成功且**带** `result` 字符串 —— 正文是回答不是错误，不得因存在 `result` 就被当失败
    ///   （这是最容易回归的一例：判据若写成「有 result 就算错」便在此翻车）。
    #[test]
    fn result_error_stays_none_for_success_shapes() {
        let plain_success = r#"{"type":"result","subtype":"success"}"#;
        let success_with_result = r#"{"type":"result","subtype":"success","result":"一切正常"}"#;

        for line in [plain_success, success_with_result] {
            match parse_stream_line(line).unwrap() {
                AgentStreamEvent::Result { ok, error, .. } => {
                    assert!(ok, "{line}");
                    assert!(error.is_none(), "{line}");
                }
                _ => panic!("expected result"),
            }
        }
    }

    /// 反向控制：`error_*` 行拿不到任何可上屏的正文时，`error` 必须是 `None`，
    /// 由前端 `assistant.error.engineFailed` 兜底；`ok` 必须为 false，否则失败又被判成成功。
    ///
    /// 三种「无正文」形状都要覆盖，缺一个就可能让空话顶掉兜底文案：
    /// - 既无 `result` 也无 `errors`；
    /// - `errors` 是空数组（首元素取不到）；
    /// - `errors[0]` 不是字符串（`as_str()` 取不到）。
    #[test]
    fn result_error_stays_none_when_failure_carries_no_text() {
        for line in [
            r#"{"type":"result","subtype":"error_during_execution","is_error":true}"#,
            r#"{"type":"result","subtype":"error_during_execution","is_error":true,"errors":[]}"#,
            r#"{"type":"result","subtype":"error_during_execution","is_error":true,"errors":[42]}"#,
        ] {
            match parse_stream_line(line).unwrap() {
                AgentStreamEvent::Result { ok, error, .. } => {
                    assert!(!ok, "{line}");
                    assert!(error.is_none(), "{line}");
                }
                _ => panic!("expected result"),
            }
        }
    }

    #[test]
    fn non_json_lines_are_ignored() {
        assert!(parse_stream_line("not json at all").is_none());
        assert!(parse_stream_line("").is_none());
    }

    #[test]
    fn build_args_include_bare_proxy_allow_rule() {
        let args = super::build_spawn_args(
            "/abs/seshbuddy-proxy", "提示词", "/tmp/settings.json",
            "系统提示", None, None,
        );
        let joined = args.join(" ");
        assert!(joined.contains("Bash(seshbuddy-proxy:*)"));
        assert!(joined.contains("--output-format stream-json"));
        assert!(joined.contains("--settings /tmp/settings.json"));
        assert!(!joined.contains("--resume"));
    }

    #[test]
    fn build_args_with_resume_session() {
        let args = super::build_spawn_args(
            "/abs/seshbuddy-proxy", "追问", "/tmp/s.json", "系统", Some("sess-1"), None,
        );
        let pos = args.iter().position(|a| a == "--resume").unwrap();
        assert_eq!(args[pos + 1], "sess-1");
    }

    #[test]
    fn system_prompt_contains_proxy_bare_name_and_contract() {
        let prompt = super::build_system_prompt("/abs/seshbuddy-proxy");
        assert!(prompt.contains("seshbuddy-proxy"));
        assert!(prompt.contains("--json-help"));
        assert!(prompt.contains("coverage"));
        // 提示词本身是英文（模型对英文指令遵循更好），多语言靠这一条覆盖：
        // 它若被删掉，非中文用户会拿到中文回答，而闸门看不见提示词内部。
        assert!(prompt.contains("user's language"));
    }

    #[test]
    fn proxy_binary_name_keeps_windows_exe_extension() {
        assert_eq!(super::proxy_binary_name("/abs/seshbuddy-proxy"), "seshbuddy-proxy");
        assert_eq!(
            super::proxy_binary_name("D:\\Codes\\SeshBuddy\\target\\debug\\seshbuddy-proxy.exe"),
            "seshbuddy-proxy.exe"
        );
    }

    #[test]
    fn truncate_tail_handles_utf8_boundary() {
        // 134 个"中" = 402 字节，keep_from=2 非边界，旧代码会 panic
        let mut s = "中".repeat(134);
        truncate_tail(&mut s, 400);
        assert!(s.len() <= 400);
        assert!(s.ends_with("中"));
    }

    #[test]
    fn pump_stream_reaps_child_and_returns_status() {
        let child = Command::new("/bin/echo")
            .arg("hello")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut turn = RunningTurn { child };
        let outcome = pump_stream(&mut turn, |_| {}).unwrap();
        assert!(outcome.status.success());
        assert_eq!(outcome.stderr_tail, "");
    }

    #[test]
    fn parses_result_usage() {
        let line = r#"{"type":"result","subtype":"success","duration_ms":1234,"total_cost_usd":0.01,"usage":{"input_tokens":1200,"output_tokens":380,"cache_read_input_tokens":5000,"cache_creation_input_tokens":600}}"#;
        let event = parse_stream_line(line).unwrap();
        match event {
            AgentStreamEvent::Result { usage, .. } => {
                let u = usage.expect("usage 应被解析");
                assert_eq!(u.input_tokens, 1200);
                assert_eq!(u.output_tokens, 380);
                assert_eq!(u.cache_read_input_tokens, 5000);
                assert_eq!(u.cache_creation_input_tokens, 600);
            }
            _ => panic!("expected result"),
        }
    }

    #[test]
    fn result_without_usage_degrades_to_none() {
        let line = r#"{"type":"result","subtype":"success","duration_ms":100}"#;
        match parse_stream_line(line).unwrap() {
            AgentStreamEvent::Result { usage, .. } => assert!(usage.is_none()),
            _ => panic!("expected result"),
        }
    }

    #[test]
    fn result_usage_missing_cache_fields_defaults_zero() {
        let line = r#"{"type":"result","subtype":"success","usage":{"input_tokens":10,"output_tokens":5}}"#;
        match parse_stream_line(line).unwrap() {
            AgentStreamEvent::Result { usage, .. } => {
                let u = usage.unwrap();
                assert_eq!(u.input_tokens, 10);
                assert_eq!(u.output_tokens, 5);
                assert_eq!(u.cache_read_input_tokens, 0);
                assert_eq!(u.cache_creation_input_tokens, 0);
            }
            _ => panic!("expected result"),
        }
    }
}
