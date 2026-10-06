use crate::app_db;
use crate::cli::CliKind;
use crate::cli_config::{
    ensure_parent_dir, read_json_file, read_toml_file, settings_path_for, strip_bom,
    write_atomically, write_json_file, write_toml_file,
};
use crate::commands::profile::{
    codex_auth_has_login_material, codex_auth_path, codex_official_profile_content_from_live,
    codex_settings_has_managed_provider_fields, get_active_profile_for, read_codex_settings_value,
    save_profile_for_internal, set_active_profile_for, set_matching_codex_profile_active,
    CODEX_OFFICIAL_PROFILE_NAME,
};
use crate::commands::session::{build_codex_search_results, AggregatedSessionSearch};
use crate::commands::session_index::{
    build_projects_scan, build_projects_snapshot, cached_session_list_metadata, file_modified_ms,
    RawSession, Resolution, ScanProjection, SnapshotProjection,
};
use crate::error::{AppError, AppResult};
use crate::history;
use crate::parser;
use crate::parser::codex;
use crate::parser::shared::{SearchDocument, SearchScanProgress};
use crate::session::{
    ChatMessage, PaginatedProjects, SearchResult, SessionListMetadata, SessionLoadResult,
    SubagentInfo, UsageRecord,
};
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

use super::super::descriptor::{CliDescriptor, FileRoot};
use super::super::features::{
    ApiProfileFeature, LaunchFeature, LaunchPlan, LaunchRequest, ProfileScopeCtx,
    ScopeApplyOutcome, StoredProfile,
};
use super::super::source::{
    collect_jsonl_files, file_backed_exists, under_data_root, CliSource, Truncation,
};
use super::super::{Discovered, SessionLocator};

pub(crate) struct CodexSource;

static DESCRIPTOR: CliDescriptor = CliDescriptor {
    name: "Codex",
    tray_label: "Codex",
    proxy_port: 18082,
    command: "codex",
    file_root: FileRoot { data_dir_segments: &[".codex"], sessions_subdir: Some("sessions") },
};

/// Codex 的配置文件名与实现同处一文件：能力位由「有无实现」对账，不必另立清单。
static CODEX_CONFIG_FILE: CodexConfigFile = CodexConfigFile;
struct CodexConfigFile;
impl crate::cli_registry::features::ConfigFileFeature for CodexConfigFile {
    fn settings_file_name(&self) -> &'static str {
        "config.toml"
    }
}

/// Codex 的 API 代理能力与实现同处一文件：能力位由「有无实现」对账，不必另立清单。
static CODEX_API_PROXY: CodexApiProxy = CodexApiProxy;
struct CodexApiProxy;

/// 未配置上游时的默认地址。
fn codex_default_base_url() -> String {
    "https://api.openai.com/v1".to_string()
}

impl crate::cli_registry::features::ApiProxyFeature for CodexApiProxy {
    fn default_base_url(&self) -> String {
        codex_default_base_url()
    }

    fn base_url(&self) -> AppResult<String> {
        let config =
            crate::cli_config::read_toml_file(&crate::cli_config::settings_path_for(CliKind::Codex)?)?;
        let provider_id = config
            .get("model_provider")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .unwrap_or("openai-chat-completions");
        Ok(config
            .get("model_providers")
            .and_then(|v| v.as_table())
            .and_then(|providers| providers.get(provider_id))
            .and_then(|v| v.as_table())
            .and_then(|provider| provider.get("base_url"))
            .and_then(|v| v.as_str())
            .unwrap_or("https://api.openai.com/v1")
            .to_string())
    }

    fn set_base_url(&self, url: &str) -> AppResult<()> {
        let path = crate::cli_config::settings_path_for(CliKind::Codex)?;
        let mut config = crate::cli_config::read_toml_file(&path)?;
        if !config.is_table() {
            config = toml::Value::Table(toml::map::Map::new());
        }
        let root = config
            .as_table_mut()
            .ok_or(crate::error::AppError::coded("proxy.codex_config_not_table"))?;
        let provider_id = root
            .get("model_provider")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .unwrap_or("openai-chat-completions")
            .to_string();
        root.insert(
            "model_provider".to_string(),
            toml::Value::String(provider_id.clone()),
        );

        let providers = root
            .entry("model_providers".to_string())
            .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
        if !providers.is_table() {
            *providers = toml::Value::Table(toml::map::Map::new());
        }
        let providers_table = providers
            .as_table_mut()
            .ok_or(crate::error::AppError::coded("proxy.model_providers_not_table"))?;
        let provider_entry = providers_table
            .entry(provider_id)
            .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
        if !provider_entry.is_table() {
            *provider_entry = toml::Value::Table(toml::map::Map::new());
        }
        let provider_table = provider_entry
            .as_table_mut()
            .ok_or(crate::error::AppError::coded("proxy.provider_config_not_table"))?;
        provider_table.insert("base_url".to_string(), toml::Value::String(url.to_string()));
        provider_table
            .entry("wire_api".to_string())
            .or_insert_with(|| toml::Value::String("responses".to_string()));
        crate::cli_config::write_toml_file(&path, &config)
    }

    fn traffic_path_filter(&self) -> &'static str {
        "(path LIKE '/v1/responses%' OR path LIKE '/v1/chat/completions%' OR path LIKE '/chat/completions%' OR path LIKE '/responses%')"
    }

    fn session_display_maps(
        &self,
    ) -> AppResult<(HashMap<String, String>, HashMap<String, Option<String>>)> {
        let history_path = crate::cli::history_path(CliKind::Codex)?;
        let history_map = history::parse_codex_history(history_path.to_str().unwrap_or(""));
        let mut project_map = crate::proxy::build_codex_session_project_map(
            &crate::cli::sessions_dir(CliKind::Codex)?,
        );

        // 会话列表索引库：自定义重命名 / 原生标题 / 清洗后首条用户消息 / 项目路径
        let records = crate::app_db::read_session_list_index(CliKind::Codex).unwrap_or_default();
        let custom_names = crate::app_db::load_session_names().unwrap_or_default();
        let mut display_map = HashMap::new();
        for record in records.values() {
            let sid = record.session_id.trim();
            if sid.is_empty() {
                continue;
            }
            let display = crate::title_resolver::resolve_display_name(
                crate::commands::session::custom_session_name(
                    &custom_names,
                    CliKind::Codex,
                    &record.session_path,
                ),
                record.title.as_deref(),
                record.first_user_message.as_deref(),
                history_map
                    .get(sid)
                    .map(|h| h.display.as_str())
                    .filter(|v| !v.is_empty()),
                sid,
            );
            display_map.insert(sid.to_string(), display);
            if let Some(p) = record.project_path.as_ref().filter(|p| !p.trim().is_empty()) {
                project_map.insert(sid.to_string(), Some(p.clone()));
            }
        }

        // 兜底：对仅在 history_map 中出现（索引库尚未收录）的 session，使用 history display 解析
        for (sid, hist) in &history_map {
            if !display_map.contains_key(sid) {
                let display = crate::title_resolver::resolve_display_name(
                    None,
                    None,
                    None,
                    Some(hist.display.as_str()).filter(|v| !v.is_empty()),
                    sid,
                );
                display_map.insert(sid.clone(), display);
            }
        }

        Ok((display_map, project_map))
    }
}

const CODEX_MANAGED_PROFILE_PATHS: &[&str] = &[
    "/codex/experimental_bearer_token",
    "/codex/base_url",
    "/codex/model",
    "/codex/model_reasoning_effort",
    "/codex/model_provider",
    "/codex/provider_name",
    "/codex/wire_api",
    "/auth/OPENAI_API_KEY",
];
const CODEX_OFFICIAL_MANAGED_PROFILE_PATHS: &[&str] = &["/auth"];

/// Codex 的用量统计能力与实现同处一文件：能力位由「有无实现」对账，不必另立清单。
static CODEX_USAGE_STATS: CodexUsageStats = CodexUsageStats;
struct CodexUsageStats;
impl crate::cli_registry::features::UsageStatsFeature for CodexUsageStats {
    fn usage_records(&self, key: &str, project: &str) -> Vec<UsageRecord> {
        codex::extract_usage_records(key, project)
    }
}

/// Codex 的配置档托管能力与实现同处一文件：能力位由「有无实现」对账，不必另立清单。
static CODEX_API_PROFILE: CodexApiProfile = CodexApiProfile;
struct CodexApiProfile;

fn deep_merge_toml(target: &mut toml::Value, source: &toml::Value) {
    if let (toml::Value::Table(target_table), toml::Value::Table(source_table)) = (target, source) {
        for (key, source_val) in source_table {
            match target_table.get_mut(key) {
                Some(target_val)
                    if matches!(target_val, toml::Value::Table(_))
                        && matches!(source_val, toml::Value::Table(_)) =>
                {
                    deep_merge_toml(target_val, source_val);
                }
                _ => {
                    target_table.insert(key.clone(), source_val.clone());
                }
            }
        }
    }
}

fn build_codex_toml_patch(content: &Value) -> toml::Value {
    let mut root = toml::map::Map::new();

    let Some(codex) = content.get("codex").and_then(|value| value.as_object()) else {
        return toml::Value::Table(root);
    };

    if let Some(model) = codex
        .get("model")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        root.insert("model".to_string(), toml::Value::String(model.to_string()));
    }
    if let Some(effort) = codex
        .get("model_reasoning_effort")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        root.insert(
            "model_reasoning_effort".to_string(),
            toml::Value::String(effort.to_string()),
        );
    }

    let provider_id = codex
        .get("model_provider")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or("openai-chat-completions")
        .to_string();

    let mut provider_patch = toml::map::Map::new();
    if let Some(name) = codex
        .get("provider_name")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        provider_patch.insert("name".to_string(), toml::Value::String(name.to_string()));
    }
    if let Some(base_url) = codex
        .get("base_url")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        provider_patch.insert(
            "base_url".to_string(),
            toml::Value::String(base_url.to_string()),
        );
    }
    if let Some(wire_api) = codex
        .get("wire_api")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        provider_patch.insert(
            "wire_api".to_string(),
            toml::Value::String(wire_api.to_string()),
        );
    } else if !provider_patch.is_empty() {
        provider_patch.insert(
            "wire_api".to_string(),
            toml::Value::String("responses".to_string()),
        );
    }

    // 优先从 codex.experimental_bearer_token 提取 API Key，兼顾兼容历史配置中的 auth.OPENAI_API_KEY
    let api_key = codex
        .get("experimental_bearer_token")
        .or_else(|| content.pointer("/auth/OPENAI_API_KEY"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty());

    if let Some(key) = api_key {
        provider_patch.insert(
            "experimental_bearer_token".to_string(),
            toml::Value::String(key.to_string()),
        );
        provider_patch.insert(
            "requires_openai_auth".to_string(),
            toml::Value::Boolean(true),
        );
    }

    if !provider_patch.is_empty() {
        root.insert(
            "model_provider".to_string(),
            toml::Value::String(provider_id.clone()),
        );

        let mut providers = toml::map::Map::new();
        providers.insert(provider_id, toml::Value::Table(provider_patch));
        root.insert("model_providers".to_string(), toml::Value::Table(providers));
    }

    toml::Value::Table(root)
}

fn apply_codex_settings_value(content: &Value) -> AppResult<()> {
    let config_path = settings_path_for(CliKind::Codex)?;
    let mut existing_config = read_toml_file(&config_path)?;
    let patch = build_codex_toml_patch(content);
    deep_merge_toml(&mut existing_config, &patch);

    // 官方登录态保护：第三方 API Key 已注入 config.toml 的
    // [model_providers.<id>].experimental_bearer_token；auth.json 专用于保留
    // ChatGPT / Codex 官方 OAuth 登录态（远程操作与官方插件强依赖它）。
    // 现有 auth.json 已含官方登录态时不能被本次写入覆盖，
    // 仅在无官方登录态、且当前配置档提供了自定义 auth 时才写入。
    let auth_path = codex_auth_path()?;
    let existing_auth = read_json_file(&auth_path, "{}").unwrap_or(Value::Object(Map::new()));

    if !codex_auth_has_login_material(&existing_auth) {
        if let Some(auth_obj) = content.get("auth").and_then(|value| value.as_object()) {
            write_json_file(&auth_path, &Value::Object(auth_obj.clone()))?;
        }
    }

    write_toml_file(&config_path, &existing_config)?;

    Ok(())
}

/// 从 config.toml 中移除 `build_codex_toml_patch` 写入的管理字段，
/// 保留 Codex 自身的 projects、tui、plugins 等字段
fn clear_codex_managed_config_fields(config: &mut toml::Value) {
    let toml::Value::Table(table) = config else {
        return;
    };

    // 记录当前的 model_provider，用于清理对应的 [model_providers.<id>]
    let provider_id = table
        .get("model_provider")
        .and_then(|v| v.as_str())
        .map(str::to_string);

    table.remove("model");
    table.remove("model_reasoning_effort");
    table.remove("model_provider");
    table.remove("experimental_bearer_token");

    // 清理 [model_providers.<id>]
    if let Some(pid) = provider_id {
        if let Some(toml::Value::Table(providers)) = table.get_mut("model_providers") {
            providers.remove(&pid);
            if providers.is_empty() {
                table.remove("model_providers");
            }
        }
    }
}

/// 应用 Codex Official profile：恢复 auth.json 中的 OAuth token，清除 config.toml 管理字段
fn apply_codex_official_profile(content: &Value) -> AppResult<()> {
    // 1. 写入 auth.json（恢复 OAuth token）
    if let Some(auth_obj) = content.get("auth").and_then(|v| v.as_object()) {
        let auth_path = codex_auth_path()?;
        write_json_file(&auth_path, &Value::Object(auth_obj.clone()))?;
    }

    // 2. 清除 config.toml 中的管理字段
    let config_path = settings_path_for(CliKind::Codex)?;
    let mut config = read_toml_file(&config_path)?;
    clear_codex_managed_config_fields(&mut config);
    write_toml_file(&config_path, &config)?;

    Ok(())
}

impl ApiProfileFeature for CodexApiProfile {
    fn allowed_paths(&self, profile: Option<&str>) -> &'static [&'static str] {
        if profile == Some(CODEX_OFFICIAL_PROFILE_NAME) {
            CODEX_OFFICIAL_MANAGED_PROFILE_PATHS
        } else {
            CODEX_MANAGED_PROFILE_PATHS
        }
    }

    fn read_settings(&self) -> AppResult<String> {
        let value = read_codex_settings_value()?;
        serde_json::to_string_pretty(&value)
            .map_err(|e| AppError::coded("profile.serialize_failed").with("detail", e.to_string()))
    }

    fn write_settings(&self, content: String) -> AppResult<()> {
        let value: Value = serde_json::from_str(&content)
            .map_err(|e| AppError::coded("profile.content_parse_failed").with("detail", e.to_string()))?;
        apply_codex_settings_value(&value)
    }

    /// 应用：Codex 只有全局一份配置，项目作用域没有可写的局部配置，静默跳过（与改动前一致）。
    /// 单条路径失败**不冒泡**：写进 `failed_dirs` 计数返回。
    fn apply_to_scope(
        &self,
        ctx: &ProfileScopeCtx<'_>,
        stored: &StoredProfile,
    ) -> AppResult<ScopeApplyOutcome> {
        let mut outcome = ScopeApplyOutcome::default();
        if ctx.scope == "global" {
            let res = if ctx.profile_name == CODEX_OFFICIAL_PROFILE_NAME {
                apply_codex_official_profile(&stored.visible)
            } else {
                apply_codex_settings_value(&stored.visible)
            };
            match res {
                Ok(()) => outcome.success_count += 1,
                Err(e) => {
                    tracing::warn!(
                        "Failed to cascade apply codex profile '{}': {}",
                        ctx.profile_name,
                        e.diagnostic()
                    );
                    if let Ok(p) = settings_path_for(CliKind::Codex) {
                        outcome.failed_dirs.push(p.to_string_lossy().to_string());
                    }
                }
            }
        }
        Ok(outcome)
    }

    /// Codex 没有目录级局部配置，解绑为空操作。
    fn unbind_scope(&self, _ctx: &ProfileScopeCtx<'_>) -> AppResult<()> {
        Ok(())
    }

    /// 绑定：全局才允许；先回填旧活动档，再按登录态确保官方档存在，最后写配置。
    fn bind_scope(&self, ctx: &ProfileScopeCtx<'_>, stored: &StoredProfile) -> AppResult<()> {
        if ctx.scope != "global" {
            return Err(AppError::coded("profile.codex_no_project_binding"));
        }

        // Backfill: 将当前 live 状态写回旧的 active profile
        let active_name = app_db::get_active_profile_with(ctx.conn, CliKind::Codex, None).unwrap_or_default();
        if !active_name.trim().is_empty() && active_name != ctx.profile_name {
            if let Ok(current_settings) = read_codex_settings_value() {
                let current_json =
                    serde_json::to_string_pretty(&current_settings).unwrap_or_default();
                let allow_official_overwrite = active_name == CODEX_OFFICIAL_PROFILE_NAME;
                if let Err(e) = save_profile_for_internal(
                    CliKind::Codex,
                    active_name.clone(),
                    current_json,
                    allow_official_overwrite,
                    None,
                ) {
                    tracing::warn!("Backfill failed for profile '{}': {}", active_name, e.diagnostic());
                }
            }
        }

        // 检测 OAuth 登录态，必要时自动创建 Codex Official profile
        let auth_path = codex_auth_path().unwrap_or_default();
        if let Ok(auth) = read_json_file(&auth_path, "{}") {
            if codex_auth_has_login_material(&auth) {
                let official_exists = app_db::list_profiles_with(ctx.conn, CliKind::Codex)
                    .unwrap_or_default()
                    .iter()
                    .any(|n| n == CODEX_OFFICIAL_PROFILE_NAME);
                if !official_exists {
                    let official_content =
                        codex_official_profile_content_from_live(&read_codex_settings_value().unwrap_or_default());
                    let stored_json =
                        serde_json::to_string_pretty(&official_content).unwrap_or_default();
                    if let Err(e) = save_profile_for_internal(
                        CliKind::Codex,
                        CODEX_OFFICIAL_PROFILE_NAME.to_string(),
                        stored_json,
                        true,
                        None,
                    ) {
                        tracing::warn!("Failed to create Codex Official profile: {}", e.diagnostic());
                    }
                }
            }
        }

        if ctx.profile_name == CODEX_OFFICIAL_PROFILE_NAME {
            apply_codex_official_profile(&stored.visible)?;
        } else {
            apply_codex_settings_value(&stored.visible)?;
        }
        Ok(())
    }

    /// 登录态生成的官方档由 Codex 自己维护，用户改坏它会让 Codex 直接不可用。
    fn protected_profile_name(&self) -> Option<&'static str> {
        Some(CODEX_OFFICIAL_PROFILE_NAME)
    }

    /// Codex 只有全局一份 config.toml，没有按目录的项目作用域。
    fn supports_project_scope(&self) -> bool {
        false
    }

    /// 从当前 live 配置反向同步活动档：有 OAuth 登录态时把 live 回填进官方档，
    /// 否则当活动档正是官方档时，改选与 live 匹配的自建档（没有匹配则清空活动档）。
    ///
    /// 返回 `None` 表示这两条都不成立，调用方走通用回写路径。
    fn sync_active_from_live(&self) -> AppResult<Option<bool>> {
        let current_settings = read_codex_settings_value()?;
        let has_oauth = current_settings
            .get("auth")
            .is_some_and(codex_auth_has_login_material);
        if has_oauth {
            let official_settings = codex_official_profile_content_from_live(&current_settings);
            let current_json = serde_json::to_string_pretty(&official_settings)
                .map_err(|e| AppError::coded("profile.serialize_failed").with("detail", e.to_string()))?;
            save_profile_for_internal(
                CliKind::Codex,
                CODEX_OFFICIAL_PROFILE_NAME.to_string(),
                current_json,
                true,
                None,
            )?;
            if !codex_settings_has_managed_provider_fields(&current_settings) {
                set_active_profile_for(CliKind::Codex, CODEX_OFFICIAL_PROFILE_NAME, None)?;
            }
            return Ok(Some(true));
        }

        let active_name = get_active_profile_for(CliKind::Codex, None)?;
        if active_name == CODEX_OFFICIAL_PROFILE_NAME {
            if !set_matching_codex_profile_active(&current_settings)? {
                app_db::clear_active_profile(CliKind::Codex, None)?;
            }
            return Ok(Some(true));
        }

        Ok(None)
    }

    /// 读原始 TOML 文本：前端要按原文编辑，不走 JSON 桥接。
    fn read_scope_settings(&self, _scope: &str) -> AppResult<String> {
        let path = settings_path_for(CliKind::Codex)?;
        if !path.exists() {
            return Ok(String::new());
        }
        fs::read_to_string(&path)
            .map(strip_bom)
            .map_err(|e| AppError::coded("profile.settings_read_failed").with("detail", e.to_string()))
    }

    /// 原子写原始 TOML 文本：先按 TOML 校验，畸形内容报 `profile.toml_invalid`。
    fn write_scope_settings(&self, _scope: &str, content: String) -> AppResult<()> {
        content
            .parse::<toml::Value>()
            .map_err(|e| AppError::coded("profile.toml_invalid").with("detail", e.to_string()))?;
        let path = settings_path_for(CliKind::Codex)?;
        ensure_parent_dir(&path)?;
        write_atomically(&path, content.as_bytes())
    }
}

impl CliSource for CodexSource {
    fn descriptor(&self) -> &'static CliDescriptor {
        &DESCRIPTOR
    }

    fn owns(&self, key: &str) -> bool {
        under_data_root(key, CliKind::Codex)
    }

    /// 文件型源：会话身份是一个文件，存在性即文件是否在盘上；虚拟定位符不属于本类源，
    /// 由共享实现判「存在」，以免库型会话被孤儿清理误删。
    fn exists(&self, loc: &SessionLocator) -> bool {
        file_backed_exists(loc)
    }

    /// 文件型源：会话是一个文件，可以移入回收站。
    fn can_delete(&self) -> bool {
        true
    }

    /// 枚举本 CLI 的会话：递归收集 `sessions/` 下的全部 `.jsonl`。
    ///
    /// 排序必须保留：它决定索引写入次序，进而影响列表次序。读取失败会冒泡中止整轮扫描
    /// （递归收集无法精确记出失败子树），故 `unverified_prefixes` 给空。
    fn discover(&self) -> AppResult<Discovered> {
        let sessions_dir = crate::cli::sessions_dir(CliKind::Codex)?;
        if !sessions_dir.exists() {
            return Ok(Discovered { locators: Vec::new(), unverified_prefixes: Vec::new() });
        }
        let mut session_files = Vec::new();
        collect_jsonl_files(&sessions_dir, &mut session_files)?;
        session_files.sort();
        Ok(Discovered {
            locators: session_files
                .into_iter()
                .map(|path| SessionLocator::File { cli_id: CliKind::Codex, path })
                .collect(),
            unverified_prefixes: Vec::new(),
        })
    }

    /// 项目路径缺席时分组键同为 `None`（不造占位串）；标题另有索引 thread_name Map 兜底。
    fn snapshot(
        &self,
        custom_names: &HashMap<String, String>,
        page: usize,
        page_size: usize,
    ) -> AppResult<Option<PaginatedProjects>> {
        build_projects_snapshot(
            CliKind::Codex,
            custom_names,
            page,
            page_size,
            || {
                let history_path = crate::cli::history_path(CliKind::Codex)?;
                let session_map =
                    crate::history::parse_codex_history(history_path.to_str().unwrap_or(""));
                // 单次读取 Codex 索引 thread_name Map，整页查询 O(1) 命中
                let codex_titles = crate::parser::load_codex_index_titles();
                Ok(move |record: &crate::app_db::SessionListIndexRecord| {
                    // 快照与实时扫描必须给出同一个分组键：索引里没有项目路径时两边都是 `None`。
                    let original_path = record
                        .project_path
                        .clone()
                        .filter(|path| !path.trim().is_empty());
                    let encoded_dir = original_path.clone();
                    let stem = Path::new(&record.session_path)
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("");
                    let title = record
                        .title
                        .clone()
                        .or_else(|| codex_titles.get(stem).cloned())
                        .or_else(|| codex_titles.get(&record.session_id).cloned());
                    SnapshotProjection {
                        encoded_dir,
                        original_path,
                        title,
                        history_display: session_map
                            .get(&record.session_id)
                            .map(|entry| entry.display.clone())
                            .filter(|display| !display.is_empty()),
                        git_branch: record.git_branch.clone(),
                    }
                })
            },
        )
    }

    fn scan(
        &self,
        custom_names: &HashMap<String, String>,
        force: bool,
    ) -> AppResult<Vec<RawSession>> {
        let sessions_dir = crate::cli::sessions_dir(CliKind::Codex)?;
        // 会话根目录不存在与「存在但为空」对陈旧行判定意味着相反的事：根目录缺席说明
        // 数据源整体不可用，此时不该按「都没扫到」清理索引；空目录才是「会话确实没了」。
        if !sessions_dir.exists() {
            return Ok(Vec::new());
        }

        let history_path = crate::cli::history_path(CliKind::Codex)?;
        let session_map = history::parse_codex_history(history_path.to_str().unwrap_or(""));
        // 单次读取 Codex 索引 thread_name Map，整批查询 O(1) 命中
        let codex_titles = parser::load_codex_index_titles();
        let discovered = self.discover()?;

        build_projects_scan(CliKind::Codex, custom_names, force, discovered, |loc, ctx| {
            let Some(session_path) = loc.as_file_path() else {
                return Ok(Resolution::Skip);
            };
            let file_path = loc.to_key();
            // 双轨墓碑拦截：若会话已被标记删除，拦截重新索引，防止死灰复燃。
            // 必须先于元数据解析 —— 已删会话不该再触发一次读盘解析。
            let session_id = session_path.file_stem().and_then(|n| n.to_str()).unwrap_or("");
            if ctx.tombstone_filter.is_tombstoned(session_id, &file_path) {
                return Ok(Resolution::Skip);
            }
            let file_metadata = match fs::metadata(session_path) {
                Ok(metadata) if metadata.is_file() => metadata,
                _ => return Ok(Resolution::Skip),
            };
            let file_size = file_metadata.len();
            let modified_ms = file_modified_ms(&file_metadata);
            // 缓存命中（未变更且非 force）时不重解析元数据；未命中才读盘解析。
            // 子代理形态的路径在此恒不命中、每轮重解析，但其索引行在内容与 mtime 未变时不再
            // 重写（`indexed_at` 不刷新）；`indexed_at` 不参与逻辑与展示，行内容逐字相同。
            let metadata = if ctx.force {
                None
            } else {
                cached_session_list_metadata(ctx.cached_index, &file_path, file_size, modified_ms)
            }
            .or_else(|| self.metadata_only(&file_path));
            let Some(metadata) = metadata else {
                return Ok(Resolution::Unparsed);
            };

            let session_id = metadata.session_id.clone();
            // 没有项目路径就是没有：`None` 既是分组键也是展示值的「缺席」，兜底句由前端渲染。
            // Codex 不做项目路径纠正 —— 投影给出的值与元数据一致，骨架因此不写索引更新。
            let original_path = metadata.project_path.clone().filter(|path| !path.trim().is_empty());
            let encoded_dir = original_path.clone();
            let title = metadata
                .title
                .clone()
                .or_else(|| codex_titles.get(&session_id).cloned())
                .or_else(|| {
                    session_path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .and_then(|stem| codex_titles.get(stem).cloned())
                });
            Ok(Resolution::Session {
                projection: ScanProjection {
                    project_path: metadata.project_path.clone(),
                    encoded_dir,
                    original_path,
                    title,
                    history_display: session_map
                        .get(&session_id)
                        .map(|h| h.display.clone())
                        .filter(|d| !d.is_empty()),
                    git_branch: metadata.git_branch.clone(),
                },
                metadata,
            })
        })
    }

    fn parse(&self, key: &str) -> AppResult<Vec<ChatMessage>> {
        codex::parse_codex_session_file(key).map(|result| result.messages)
    }

    fn first_user_message(&self, key: &str) -> Option<String> {
        codex::read_codex_first_user_message(key)
    }

    /// `session_meta.id` 是权威标识；旧文件缺该字段时退回文件名主干，
    /// 文件名形如 `rollout-<时间戳>-<id>`，末段才是 id。
    fn session_id(&self, key: &str) -> Option<String> {
        codex::read_codex_session_meta_field(key, |payload| {
            payload
                .get("id")
                .and_then(|v| v.as_str())
                .map(|v| v.to_string())
        })
        .or_else(|| {
            Path::new(key)
                .file_stem()
                .and_then(|stem| stem.to_str())
                .and_then(|stem| stem.rsplit('-').next())
                .map(|stem| stem.to_string())
        })
    }

    /// `session_meta.cwd` 是首选；它缺席的旧文件改从 `turn_context.cwd` 取。
    fn project_path_raw(&self, key: &str) -> Option<String> {
        codex::read_codex_session_meta_field(key, |payload| {
            payload
                .get("cwd")
                .and_then(|v| v.as_str())
                .map(|v| v.to_string())
        })
        .or_else(|| codex::read_codex_turn_context_cwd(key))
    }

    fn metadata_only(&self, key: &str) -> Option<SessionListMetadata> {
        codex::scan_codex_metadata_only(Path::new(key))
    }

    fn is_subagent(&self, key: &str) -> bool {
        codex::is_codex_subagent_file(key)
    }

    fn parse_streaming(
        &self,
        key: &str,
        _skip_sidechain: bool,
        on_batch: &mut dyn FnMut(Vec<ChatMessage>) -> bool,
    ) -> AppResult<(u64, HashMap<String, SubagentInfo>)> {
        codex::parse_codex_session_file_streaming(key, on_batch)
    }

    fn parse_incremental(
        &self,
        key: &str,
        offset: u64,
        _skip_sidechain: bool,
    ) -> AppResult<SessionLoadResult> {
        codex::parse_codex_session_incremental(key, offset)
    }

    fn parse_from_content(&self, content: &str) -> Vec<ChatMessage> {
        codex::parse_codex_session_from_string(content)
    }

    fn search_docs(
        &self,
        key: &str,
        on_progress: &mut dyn FnMut(SearchScanProgress),
    ) -> Option<Vec<SearchDocument>> {
        crate::parser::shared::scan_search_docs_with_extractor(
            Path::new(key),
            codex::extract_codex_role_text,
            on_progress,
        )
    }

    fn search_docs_from_bytes(&self, content: &[u8]) -> Option<Vec<SearchDocument>> {
        crate::parser::shared::scan_search_docs_from_text(content, codex::extract_codex_role_text)
    }

    fn new_session(&self) -> Option<&'static dyn LaunchFeature> {
        Some(&CODEX_LAUNCH)
    }

    fn resume_session(&self) -> Option<&'static dyn LaunchFeature> {
        Some(&CODEX_LAUNCH)
    }

    fn config_file(&self) -> Option<&'static dyn crate::cli_registry::features::ConfigFileFeature> {
        Some(&CODEX_CONFIG_FILE)
    }

    fn api_proxy(&self) -> Option<&'static dyn crate::cli_registry::features::ApiProxyFeature> {
        Some(&CODEX_API_PROXY)
    }

    fn api_profile(&self) -> Option<&'static dyn crate::cli_registry::features::ApiProfileFeature> {
        Some(&CODEX_API_PROFILE)
    }

    /// 本 CLI 不支持分叉会话。
    fn fork(&self) -> Option<&'static dyn crate::cli_registry::features::ForkFeature> {
        None
    }

    fn usage_stats(&self) -> Option<&'static dyn crate::cli_registry::features::UsageStatsFeature> {
        Some(&CODEX_USAGE_STATS)
    }

    fn is_session_event_path(&self, path: &Path, _sessions_dir: &Path) -> bool {
        // Codex 会话文件为 sessions/YYYY/MM/DD/rollout-*.jsonl（扫描端 collect_jsonl_files 仅收 .jsonl）
        path.extension().and_then(|e| e.to_str()) == Some("jsonl")
    }

    fn locate_executable(&self) -> Option<String> {
        crate::cli::find_on_path(self.descriptor().command)
    }

    /// 会话文件名是 `session.jsonl*` 这类成批共用的固定名时，文件名主干不是唯一 id，
    /// 改取父目录名；其余取文件名主干。
    ///
    /// **`session.jsonl*` 这一支与索引端刻意不同**：索引端的 `path_only_session_id` 只看路径、
    /// 不读文件，对这类文件名给的是主干（随后被归一成空串、退回物理路径那一支）。
    /// 两处**不要统一** —— 统一会同时改掉墓碑判定与检索待索引列表的判据。
    fn tombstone_fallback_id(&self, session_path: &Path) -> String {
        let file_name = session_path.file_name().and_then(|s| s.to_str()).unwrap_or("");
        if file_name.starts_with("session.jsonl") {
            session_path
                .parent()
                .and_then(|parent| parent.file_name())
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string()
        } else {
            session_path.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string()
        }
    }

    fn path_only_session_id(&self, session_path: &str) -> Option<String> {
        Path::new(session_path).file_stem().and_then(|s| s.to_str()).map(|s| s.to_string())
    }

    /// 本 CLI 的自定义标题就在会话文件里、索引已存，没有另一个数据源要取。
    fn custom_titles(&self) -> HashMap<String, String> {
        HashMap::new()
    }

    /// Codex 的索引标题来自自己维护的 `session_index.jsonl`（会话名 → 标题）。
    ///
    /// 索引文件缺席与索引存在但为空，都归为「没有索引」返回 `None`：空表查不出任何
    /// 标题，与没有索引等价，调用方无须为「索引存在但为空」另设分支。
    fn index_titles(&self) -> Option<HashMap<String, String>> {
        let map = crate::parser::load_codex_index_titles();
        if map.is_empty() {
            None
        } else {
            Some(map)
        }
    }

    /// Codex 的检索结果用 history.jsonl 提供展示名兜底，装配路径与 Claude 不同，
    /// 其余四家共用本路径并喂空映射。
    fn build_search_results(
        &self,
        aggregated: Vec<AggregatedSessionSearch>,
        records: &HashMap<String, app_db::SessionListIndexRecord>,
        custom_names: &HashMap<String, String>,
    ) -> AppResult<Vec<SearchResult>> {
        let history_path = crate::cli::history_path(CliKind::Codex)?;
        let session_map = history::parse_codex_history(history_path.to_str().unwrap_or(""));
        Ok(build_codex_search_results(
            aggregated,
            records,
            custom_names,
            &session_map,
            &HashMap::new(),
            CliKind::Codex,
        ))
    }

    /// 源文件是明文，归档即 `gzip(jsonl)`，内层无需再解。
    fn decode_archived_source(&self, bytes: Vec<u8>, _truncation: Truncation) -> Vec<u8> {
        bytes
    }

    /// 源文件是明文，恢复写回无需回压。
    fn encode_for_restore(&self, content: Vec<u8>, _session_path: &str) -> AppResult<Vec<u8>> {
        Ok(content)
    }

    /// 启动 PTY 不需要临时配置。
    fn build_temp_settings(&self) -> AppResult<Option<String>> {
        Ok(None)
    }

    /// 本 CLI 的钩子事件无需转给本应用。
    fn needs_hook_relay(&self) -> bool {
        false
    }

    /// Codex 的转录按 Codex 格式提取。
    fn transcript_format(&self) -> Option<transcript_store::extract::CliFormat> {
        Some(transcript_store::extract::CliFormat::Codex)
    }
}

/// Codex 的恢复是 `resume <id>` **子命令**而非开关，且权限开关要排在子命令之前 ——
/// 与 Claude 的参数顺序不同，故两份计划各自成文，而不是共用一个模板再打补丁。
struct CodexLaunch;

static CODEX_LAUNCH: CodexLaunch = CodexLaunch;

impl LaunchFeature for CodexLaunch {
    fn plan(&self, req: &LaunchRequest<'_>) -> AppResult<LaunchPlan> {
        let mut args = Vec::new();
        if req.skip_permissions {
            args.push("--dangerously-bypass-approvals-and-sandbox".to_string());
        }
        if let Some(id) = req.session_id {
            args.push("resume".to_string());
            args.push(id.to_string());
        }
        // Codex 没有设置文件参数：多拼一个 `--settings` 会被当成未知参数，
        // 结果是每次启动都失败。忽略而不是报错，与改动前一致。
        Ok(LaunchPlan::CommandLine { args })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn codex_build_toml_patch_injects_experimental_bearer_token_and_requires_auth() {
        // 1. 新版标准结构：experimental_bearer_token 在 codex 节点下
        let content = json!({
            "codex": {
                "model_provider": "deepseek",
                "provider_name": "DeepSeek",
                "base_url": "https://api.deepseek.com",
                "model": "deepseek-chat",
                "experimental_bearer_token": "sk-custom-secret"
            }
        });

        let patch = build_codex_toml_patch(&content);
        let patch_table = patch.as_table().expect("patch should be a toml table");
        assert_eq!(
            patch_table.get("model_provider").and_then(|v| v.as_str()),
            Some("deepseek")
        );
        assert_eq!(
            patch_table.get("model").and_then(|v| v.as_str()),
            Some("deepseek-chat")
        );

        let provider_table = patch_table
            .get("model_providers")
            .and_then(|v| v.as_table())
            .and_then(|v| v.get("deepseek"))
            .and_then(|v| v.as_table())
            .expect("should have [model_providers.deepseek] table");

        assert_eq!(
            provider_table.get("experimental_bearer_token").and_then(|v| v.as_str()),
            Some("sk-custom-secret")
        );
        assert_eq!(
            provider_table.get("requires_openai_auth").and_then(|v| v.as_bool()),
            Some(true)
        );

        // 2. 兼容旧版结构：OPENAI_API_KEY 在 auth 节点下
        let legacy_content = json!({
            "auth": {
                "OPENAI_API_KEY": "sk-legacy-secret"
            },
            "codex": {
                "model_provider": "deepseek",
                "provider_name": "DeepSeek",
                "base_url": "https://api.deepseek.com",
                "model": "deepseek-chat"
            }
        });
        let legacy_patch = build_codex_toml_patch(&legacy_content);
        let legacy_provider_table = legacy_patch
            .get("model_providers")
            .and_then(|v| v.get("deepseek"))
            .expect("legacy patch should have [model_providers.deepseek]");
        assert_eq!(
            legacy_provider_table.get("experimental_bearer_token").and_then(|v| v.as_str()),
            Some("sk-legacy-secret")
        );
    }

    #[test]
    fn codex_clear_managed_config_fields_removes_provider_table_and_bearer_token() {
        let raw = r#"
model_provider = "custom"
model = "deepseek-chat"
experimental_bearer_token = "stale-key"

[model_providers.custom]
name = "DeepSeek"
base_url = "https://api.deepseek.com"
experimental_bearer_token = "sk-custom-secret"
requires_openai_auth = true

[other_section]
keep_me = true
"#;
        let mut config: toml::Value = raw.parse().unwrap();
        clear_codex_managed_config_fields(&mut config);

        let table = config.as_table().unwrap();
        assert!(table.get("model").is_none());
        assert!(table.get("model_provider").is_none());
        assert!(table.get("experimental_bearer_token").is_none());
        assert!(table.get("model_providers").is_none());
        assert!(table.get("other_section").is_some());
    }
}
