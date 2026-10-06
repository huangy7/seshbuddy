//! 会话分叉能力。

/// 分叉会话的能力。
///
/// 有本能力 = 这个 CLI 的会话可以被分叉；没有 = 不支持，调用方报
/// `session.continue_unsupported`（错误码与改动前逐字相同）。
pub(crate) trait ForkFeature: Send + Sync {
    /// 分叉落盘之后的该 CLI 特有动作。
    ///
    /// 两种形态差别很大：一种要同步复制代码回滚快照，另一种要往自己的本地库注册新会话。
    /// 两者都是**尽力而为**：改动前失败只打日志、不阻断分叉，实现须保持同一语义
    /// —— 因此本方法**不返回 `AppResult`**，失败在实现里自己记日志。
    fn after_fork(&self, old_id: &str, new_id: &str);
}

#[cfg(test)]
mod tests {
    use crate::cli::CliKind;

    /// 声称能分叉的 CLI 必须真的做得到：分叉路径要能把新会话落盘，
    /// 所以「有能力」与「有实现」必须同源。
    #[test]
    fn fork_capability_is_declared_explicitly() {
        let capable: Vec<CliKind> = CliKind::ALL
            .iter()
            .copied()
            .filter(|k| crate::cli_registry::source_for(*k).fork().is_some())
            .collect();
        assert_eq!(
            capable,
            vec![CliKind::Claude, CliKind::WorkBuddy],
            "能分叉的 CLI 集合变了"
        );
    }
}
