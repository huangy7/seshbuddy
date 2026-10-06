//! 可选能力对象。
//!
//! 每个对象都**真的能干活**：能力位是它的存在性的投影，因此
//! 「声称支持某项能力」与「存在可用实现」不可能分叉。
//!
//! 访问器一律**不给默认实现**。默认返回 `None` 会让「我忘了实现」
//! 变成合法的「我不支持」—— 这正是此前散落各处的 `_ => false` 兜底所犯的病。

use crate::cli::CliKind;
use crate::error::{AppError, AppResult};

#[derive(Debug, Clone)]
pub struct LaunchRequest<'a> {
    /// `None` 即新建会话；对只能被恢复的深链型 CLI 则是「打开应用但不定会话」。
    pub session_id: Option<&'a str>,
    pub skip_permissions: bool,
    pub settings_file: Option<&'a str>,
}

/// 启动计划。把「命令行参数」「深链」两种启动形态收进一个有限枚举，
/// 使执行侧的分支数与 CLI 数量无关 —— 此前 `Vec<String>` 装不下深链，
/// 差异被挤成 `if kind == CliKind::WorkBuddy` 特判与前端三处对应分支。
///
/// 命令行形态不携带可执行名：它由描述符的 `command` 唯一给出，执行侧再经 `find_cli_path` 解析。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchPlan {
    CommandLine { args: Vec<String> },
    DeepLink { url: String },
}

/// 新建与恢复共用一个能力形状：现有 CLI 的新建计划都恰好等于「不带会话 id 的恢复计划」，
/// 两者的差别只在源是否把它挂到 `new_session` / `resume_session` 访问器上。
pub(crate) trait LaunchFeature: Send + Sync {
    fn plan(&self, req: &LaunchRequest<'_>) -> AppResult<LaunchPlan>;
}

/// 依据是否带会话 id 选择新建还是恢复能力对象。
///
/// 这个「选哪个能力」的判断必须只有一处：终端启动（`cli.rs::open_in_terminal`）、
/// PTY 启动（`pty_manager::create_session`）与复制命令共用本函数，
/// 才能保证「同一个 CLI + 同一组输入」在各处得到同一个启动计划。
pub(crate) fn launch_plan_for(
    kind: CliKind,
    session_id: Option<&str>,
    skip_permissions: bool,
    settings_file: Option<&str>,
) -> AppResult<LaunchPlan> {
    let source = crate::cli_registry::source_for(kind);
    let feature = match session_id {
        Some(_) => source.resume_session(),
        // 没有新建能力时回落到恢复能力：对「只能被恢复」的 CLI 来说缺 id 是合法输入，
        // 深链能力据此落到应用根。
        None => source.new_session().or_else(|| source.resume_session()),
    };
    // 两个能力都没有时明确报错，而不是拼出空参数的命令行 ——
    // 空参数会让恢复静默变成一次不带 `--resume` 的裸启动，用户以为接上了原会话。
    let feature = feature.ok_or_else(|| unsupported_kind(kind))?;
    feature.plan(&LaunchRequest { session_id, skip_permissions, settings_file })
}

/// 「本 CLI 没有可用的启动形态」的统一错误。带 code 而非裸串：
/// 调用链上任何一次压成 `String` 都会让前端只能显示一句无法本地化的兜底文案。
pub(crate) fn unsupported_kind(kind: CliKind) -> AppError {
    AppError::coded("cli.unsupported_kind").with("kind", kind.id())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli_registry::source_for;

    const SESSION_ID: &str = "11111111-2222-3333-4444-555555555555";

    fn request(session_id: Option<&str>) -> LaunchRequest<'_> {
        LaunchRequest { session_id, skip_permissions: false, settings_file: None }
    }

    /// 恢复计划的分支形态必须与 CLI 的实际启动方式一致 ——
    /// WorkBuddy 是桌面应用，只能走深链；写错形态会让恢复静默打开一个终端窗口。
    #[test]
    fn resume_plans_use_the_right_launch_shape() {
        for kind in CliKind::ALL {
            let Some(feature) = source_for(*kind).resume_session() else { continue };
            let plan = feature.plan(&request(Some(SESSION_ID))).expect("恢复计划应可构造");
            match kind {
                CliKind::WorkBuddy => assert!(
                    matches!(plan, LaunchPlan::DeepLink { .. }),
                    "WorkBuddy 的恢复必须是深链，实际 {plan:?}"
                ),
                _ => assert!(
                    matches!(plan, LaunchPlan::CommandLine { .. }),
                    "{kind:?} 的恢复应当是命令行，实际 {plan:?}"
                ),
            }
        }
    }

    /// 路由判据：有会话 id 走恢复能力，没有走新建能力，缺新建能力时回落到恢复能力。
    ///
    /// `Some` 与 `None` 两侧都要断言：只有一侧会漏掉「恢复被误判成新建」或
    /// 「新会话被误路由到恢复能力」——后者正是两个启动点各写一份路由时出现过的分叉形态。
    #[test]
    fn launch_plan_for_routes_by_session_id() {
        let resumed = launch_plan_for(CliKind::Claude, Some(SESSION_ID), true, None)
            .expect("Claude 有恢复能力");
        assert_eq!(
            resumed,
            LaunchPlan::CommandLine {
                args: vec![
                    "--resume".to_string(),
                    SESSION_ID.to_string(),
                    "--dangerously-skip-permissions".to_string(),
                ],
            }
        );

        let fresh = launch_plan_for(CliKind::Claude, None, true, None).expect("Claude 有新建能力");
        assert_eq!(
            fresh,
            LaunchPlan::CommandLine { args: vec!["--dangerously-skip-permissions".to_string()] }
        );

        // 无会话 id 且无新建能力：回落到恢复能力，由它在缺 id 时给出「打开应用」的形态。
        assert!(matches!(
            launch_plan_for(CliKind::WorkBuddy, None, false, None)
                .expect("无新建能力时应回落到恢复能力"),
            LaunchPlan::DeepLink { .. }
        ));

        // 两个能力都没有：明确报错。若这里回落成「无参数命令行」，
        // 调用方会在终端里裸启动一个 CLI，把「新会话」变成一次没有上下文的启动。
        assert!(launch_plan_for(CliKind::Dsh, None, false, None).is_err());
    }

    /// Dsh 没有可用的启动形态：新建与恢复都**必须**明确报 `cli.unsupported_kind`。
    ///
    /// 这是**有意删除**的结果，不是遗漏：Dsh 的会话由它自己的程序管理启动，命令行启动
    /// 从未有过可用实现 —— 改动前那段 `--profile tui --resume <id>` 与能力位互相矛盾。
    ///
    /// 钉错误**码**而非只断言 `is_err`：调用链上任何一次把错误压成裸字符串，前端就只剩
    /// 一句无法本地化的兜底文案，用户看到的是码而不是话。两侧都要断言 —— 只测一侧会漏掉
    /// 「恢复路径悄悄回落成不带 `--resume` 的裸启动」这一形态。
    #[test]
    fn dsh_launch_is_explicitly_unsupported_on_both_paths() {
        for session_id in [Some(SESSION_ID), None] {
            let err = launch_plan_for(CliKind::Dsh, session_id, false, None)
                .expect_err("Dsh 没有任何启动能力，必须报错而不是给出空参数命令行");
            match err {
                AppError::Coded { code, params } => {
                    assert_eq!(code, "cli.unsupported_kind", "Dsh 的错误码不对: {session_id:?}");
                    assert_eq!(
                        params.get("kind").map(String::as_str),
                        Some("dsh"),
                        "错误参数里的 CLI 标识不对"
                    );
                }
                other => panic!("Dsh 应报结构化错误，实际 {other:?}"),
            }
        }
    }

    /// 逐个 CLI 钉住：无会话 id 时的路由结果必须等于新建能力（缺失时为恢复能力）自己给出的计划。
    #[test]
    fn launch_plan_for_none_matches_new_session_plan_for_every_cli() {
        let req = LaunchRequest {
            session_id: None,
            skip_permissions: true,
            settings_file: Some("/tmp/settings.json"),
        };
        for kind in CliKind::ALL {
            let source = source_for(*kind);
            let routed = launch_plan_for(*kind, None, req.skip_permissions, req.settings_file);
            match source.new_session().or_else(|| source.resume_session()) {
                Some(feature) => assert_eq!(
                    routed.expect("有能力就必须给出计划"),
                    feature.plan(&req).expect("计划应可构造"),
                    "{kind:?} 的无会话 id 路由没有走新建能力"
                ),
                None => assert!(routed.is_err(), "{kind:?} 两个能力都没有，必须报错"),
            }
        }
    }

    /// 深链里的会话 ID 必须经过字符白名单校验：拼进 URL 的 ID 一旦含 `/`、`?`、`#`
    /// 就会把深链指向另一条路径或另一组查询参数 —— 那是「打开别人的会话」这类越权跳转，
    /// 不是渲染问题。非法 ID 一律报错，不静默丢弃。
    #[test]
    fn workbuddy_resume_plan_rejects_unexpected_characters() {
        let feature = source_for(CliKind::WorkBuddy).resume_session().expect("WorkBuddy 可恢复");
        for id in ["a/b", "a?b", "a#b", "a b", "会话"] {
            assert!(feature.plan(&request(Some(id))).is_err(), "会话 ID {id:?} 不该拼进深链");
        }
    }

    /// 没有会话 ID 时要落到应用根深链，而不是报错：这是「打开 WorkBuddy 但不定会话」，
    /// 改动前同样得到 `workbuddy://chat`。
    #[test]
    fn workbuddy_resume_plan_without_session_id_opens_the_app_root() {
        let feature = source_for(CliKind::WorkBuddy).resume_session().expect("WorkBuddy 可恢复");
        assert_eq!(
            feature.plan(&request(None)).expect("无会话 ID 也必须给出计划"),
            LaunchPlan::DeepLink { url: "workbuddy://chat".to_string() }
        );
    }
}
