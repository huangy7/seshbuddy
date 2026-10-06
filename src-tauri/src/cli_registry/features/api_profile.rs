//! 配置档（profile）托管能力。

use crate::error::AppResult;
use serde_json::Value;
use std::path::Path;

/// 配置档的托管能力。
///
/// 两个实现形态差别很大：一个把配置原样读写为 JSON 文本、按目录级联；另一个在 TOML 与 JSON
/// 之间桥接、且绑定前还要回填与按登录态建官方档。这些都是「这个 CLI 的配置怎么用」的知识，
/// 不是共用逻辑。
pub(crate) trait ApiProfileFeature: Send + Sync {
    /// 该 profile 允许被托管的配置路径白名单。可依 profile 名而不同
    /// （同一 CLI 的官方档与自建档白名单不同）。
    fn allowed_paths(&self, profile: Option<&str>) -> &'static [&'static str];

    /// 读出当前配置，统一表示为 JSON 文本（TOML 型 CLI 在此桥接）。
    fn read_settings(&self) -> AppResult<String>;

    /// 用 JSON 文本写回配置。只覆盖白名单内的路径，其余保持原样。
    fn write_settings(&self, content: String) -> AppResult<()>;

    /// 把一个作用域的配置应用下去。
    ///
    /// **逐目录**返回结果而非一个 `Ok(())`：非全局作用域会展开成多个目录，旧实现按目录
    /// 累加成功数、收集失败目录，这两个数字要返回给前端。
    /// 单条路径的失败**吞进** `failed_dirs`（与旧实现一致），只有解析路径本身失败才返回 `Err`。
    fn apply_to_scope(
        &self,
        ctx: &ProfileScopeCtx<'_>,
        stored: &StoredProfile,
    ) -> AppResult<ScopeApplyOutcome>;

    /// 解绑某个作用域：清掉本 CLI 在该作用域下写入的托管路径。
    ///
    /// **尽力而为**：旧实现在这里吞掉全部错误（目录读不到、清理失败都只忽略），
    /// 实现须保持同一语义。
    fn unbind_scope(&self, ctx: &ProfileScopeCtx<'_>) -> AppResult<()>;

    /// 绑定某个作用域：应用配置，外加该 CLI 特有的前置步骤。
    ///
    /// Codex 的前置步骤最重：先把当前 live 状态**回填**进旧的活动档，再检测 OAuth 登录态、
    /// 必要时自动创建官方档。这些只在绑定路径上发生，故不能塞进 `apply_to_scope`。
    fn bind_scope(&self, ctx: &ProfileScopeCtx<'_>, stored: &StoredProfile) -> AppResult<()>;

    /// 该 CLI 的**受保护内置档**名；没有则 `None`。
    ///
    /// 受保护的档不允许被覆盖、也不允许改名 —— 它由 CLI 自己维护（例如登录态生成的档），
    /// 用户改坏它会让 CLI 直接不可用。
    fn protected_profile_name(&self) -> Option<&'static str>;

    /// 该 CLI 是否支持**按目录的项目作用域**：同一份配置能按项目目录分别生效。
    ///
    /// 不支持的 CLI 只有全局一份配置，作用域参数对它无意义。
    fn supports_project_scope(&self) -> bool;

    /// 从 CLI 的**当前实际配置**反向同步到活动档。
    ///
    /// 返回 `Some(是否发生了同步)`；`None` 表示该 CLI 没有这种同步行为，
    /// 调用方走通用的「把当前配置写回活动档」路径。
    fn sync_active_from_live(&self) -> AppResult<Option<bool>>;

    /// 按作用域读设置。`scope == "global"` 时读全局配置。
    fn read_scope_settings(&self, scope: &str) -> AppResult<String>;

    /// 按作用域写设置。
    fn write_scope_settings(&self, scope: &str, content: String) -> AppResult<()>;
}

/// 一次作用域操作需要的全部上下文。
///
/// 收成一个结构体而不是六个参数：参数一多，调用点就再也读不出哪个实参对应哪个形参。
pub(crate) struct ProfileScopeCtx<'a> {
    pub conn: &'a rusqlite::Connection,
    /// `"global"` 或某个标签页作用域标识。
    pub scope: &'a str,
    /// 绑定路径上是被绑定的档名；解绑路径上为空串。
    pub profile_name: &'a str,
    /// 测试注入的全局配置路径覆盖。
    pub global_override: Option<&'a Path>,
}

/// 一个作用域的应用结果。
#[derive(Debug, Default)]
pub(crate) struct ScopeApplyOutcome {
    pub success_count: usize,
    /// 失败目录的展示路径，原样进返回给前端的 `failed_dirs`。
    pub failed_dirs: Vec<String>,
}

/// 一份已存储的配置档。
///
/// 从 `commands/profile.rs` 搬来（它出现在本 trait 的签名里，不能再是命令层的私有类型）。
/// 两个字段的含义不变。
#[derive(Debug, Clone)]
pub(crate) struct StoredProfile {
    pub visible: Value,
    pub managed_paths: Vec<String>,
}

#[cfg(test)]
mod tests {
    use crate::cli::CliKind;

    /// 白名单不能是空的：有能力却什么都不托管，等于「支持」是假的 ——
    /// 用户会看到 profile 存下来了、应用却什么都没变。
    #[test]
    fn capable_clis_manage_at_least_one_path() {
        for kind in CliKind::ALL {
            let Some(feature) = crate::cli_registry::source_for(*kind).api_profile() else {
                continue;
            };
            assert!(
                !feature.allowed_paths(None).is_empty(),
                "{kind:?} 声称支持配置档，白名单却是空的"
            );
        }
    }

    /// 受保护档与项目作用域都是**逐 CLI 的事实**，改动前藏在 `matches!(kind, …)` 里。
    /// 钉住当前取值，任何一处翻转都要有人来解释。
    #[test]
    fn profile_domain_facts_are_declared_explicitly() {
        use crate::cli::CliKind;

        let protected: Vec<(CliKind, &'static str)> = CliKind::ALL
            .iter()
            .copied()
            .filter_map(|k| {
                let f = crate::cli_registry::source_for(k).api_profile()?;
                Some((k, f.protected_profile_name()?))
            })
            .collect();
        assert_eq!(
            protected,
            vec![(CliKind::Codex, "Codex Official")],
            "有受保护内置档的 CLI 集合变了"
        );

        let scoped: Vec<CliKind> = CliKind::ALL
            .iter()
            .copied()
            .filter(|k| {
                crate::cli_registry::source_for(*k)
                    .api_profile()
                    .is_some_and(|f| f.supports_project_scope())
            })
            .collect();
        assert_eq!(scoped, vec![CliKind::Claude], "支持项目作用域的 CLI 集合变了");
    }
}
