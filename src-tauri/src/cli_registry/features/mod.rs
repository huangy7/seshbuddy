//! 可选能力对象。
//!
//! 每个对象都**真的能干活**：能力位是它的存在性的投影，因此
//! 「声称支持某项能力」与「存在可用实现」不可能分叉。
//!
//! 访问器一律**不给默认实现**。默认返回 `None` 会让「我忘了实现」
//! 变成合法的「我不支持」—— 这正是此前散落各处的 `_ => false` 兜底所犯的病。

mod api_profile;
mod api_proxy;
mod config_file;
mod fork;
mod launch;
mod usage_stats;

pub(crate) use api_profile::{
    ApiProfileFeature, ProfileScopeCtx, ScopeApplyOutcome, StoredProfile,
};
pub(crate) use api_proxy::ApiProxyFeature;
pub(crate) use config_file::ConfigFileFeature;
pub(crate) use fork::ForkFeature;
pub(crate) use launch::{launch_plan_for, unsupported_kind, LaunchFeature, LaunchPlan, LaunchRequest};
pub(crate) use usage_stats::UsageStatsFeature;
