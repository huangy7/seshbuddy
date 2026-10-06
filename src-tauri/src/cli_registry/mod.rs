//! CLI 描述符、注册表与会话源。
//!
//! 描述符是**零 I/O 的静态元数据**——源未安装也能读，列表、图标、展示顺序都靠它。
//! 真正碰盘/碰库的会话源由 `CliSource` 承担。

// 本模块不挂模块级 `dead_code` 放行：路由与启动计划落地后，`kind_for_path` 与 `source_for`
// 都已各有本模块之外的引用点。模块级放行会把模块内任何新出现的未使用项一并静默。
mod catalog;
mod descriptor;
pub(crate) mod features;
mod source;
mod sources;

pub(crate) use catalog::{descriptor_for, kind_for_path, source_for};
pub(crate) use descriptor::NO_PROXY_PORT;
pub(crate) use source::{Discovered, SessionLocator, Truncation};
