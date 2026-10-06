//! 用量统计能力。

use crate::session::UsageRecord;

/// 用量统计能力。
///
/// 有本能力 = 该 CLI 能提取用量记录；没有 = 没有用量数据。
///
/// 这条把「不支持」从**返回空表**改成**根本没有这个能力**：改动前不支持的 CLI 由
/// `CliSource::usage_records` 返回空 `Vec`，调用方分不出「没有数据」与「不支持」。
pub(crate) trait UsageStatsFeature: Send + Sync {
    /// 从一条会话提取用量记录。
    fn usage_records(&self, key: &str, project: &str) -> Vec<UsageRecord>;
}

#[cfg(test)]
mod tests {
    use crate::cli::CliKind;

    /// 有能力的 CLI 必须真的有提取实现，没有的必须没有 —— 两者不能分叉。
    /// 改动前这个集合是「除 WorkBuddy 外都有」，散在一条 `if kind == WorkBuddy` 里。
    #[test]
    fn usage_stats_capability_is_declared_explicitly() {
        let capable: Vec<CliKind> = CliKind::ALL
            .iter()
            .copied()
            .filter(|k| crate::cli_registry::source_for(*k).usage_stats().is_some())
            .collect();
        assert_eq!(
            capable,
            vec![
                CliKind::Claude,
                CliKind::Codex,
                CliKind::Gemini,
                CliKind::Dsh,
                CliKind::Antigravity
            ],
            "有用量统计能力的 CLI 集合变了"
        );
    }
}
