//! 可托管的配置文件能力。

/// 该 CLI 是否有可托管的配置文件。
///
/// 这是「配置文件能力」的唯一事实来源：只有在本能力存在时，读、写、导入才有意义。
/// 其它模块要判断某 CLI 能否托管配置，应当问这里而不是另立清单 ——
/// 否则清单会与实现脱节，出现「列了某 CLI、但它没有配置文件实现」的矛盾。
pub(crate) trait ConfigFileFeature: Send + Sync {
    /// 配置文件名（`settings.json` / `config.toml` / …）。
    /// 路径由 `data_dir` 拼出，各 CLI 一致，故本能力只管文件名。
    fn settings_file_name(&self) -> &'static str;
}
