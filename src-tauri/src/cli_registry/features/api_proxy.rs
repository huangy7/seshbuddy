//! API 代理能力。

use crate::error::AppResult;
use std::collections::HashMap;

/// API 代理能力。
///
/// **端口不在这里**：`ProxyStatus.port` 是必填字段，`default_proxy_status` 对**所有** CLI
/// 都会读它并返回给前端；而 Gemini / WorkBuddy 没有代理能力却各有自己的端口号。
/// 端口因此是「每个 CLI 都要上报的逐 CLI 数据」，归描述符（见 `CliDescriptor::proxy_port`）。
/// 把它收进本能力会让那两个 CLI 丢掉原有的端口值。
pub(crate) trait ApiProxyFeature: Send + Sync {
    /// 未配置时展示的默认上游地址。
    fn default_base_url(&self) -> String;

    /// 读出当前配置的上游地址。
    fn base_url(&self) -> AppResult<String>;

    /// 写回上游地址。写哪个文件、写到哪个字段，逐 CLI 不同。
    fn set_base_url(&self, url: &str) -> AppResult<()>;

    /// 抓包库中「哪些请求属于本 CLI」的 SQL 谓词。
    fn traffic_path_filter(&self) -> &'static str;

    /// 会话展示元数据：`sid → 展示名` 与 `sid → 项目路径`。
    fn session_display_maps(
        &self,
    ) -> AppResult<(HashMap<String, String>, HashMap<String, Option<String>>)>;
}

#[cfg(test)]
mod tests {
    use crate::cli::CliKind;

    /// 抓包过滤谓词必须真的能筛出东西：`1=0` 这类恒假谓词会让抓包页永远为空，
    /// 却看起来一切正常。有代理能力就必须给出非恒假的谓词。
    #[test]
    fn traffic_path_filter_is_not_vacuously_false() {
        for kind in CliKind::ALL {
            let Some(feature) = crate::cli_registry::source_for(*kind).api_proxy() else {
                continue;
            };
            let filter = feature.traffic_path_filter();
            assert!(!filter.trim().is_empty(), "{kind:?} 的过滤谓词为空");
            assert_ne!(filter.trim(), "1=0", "{kind:?} 的过滤谓词恒假");
        }
    }
}
