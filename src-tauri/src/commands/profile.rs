use crate::error::{AppError, AppResult};
use crate::app_db::{self, BookmarkRecord, FavoriteEntry};
use crate::cli::{self, CliKind};
use crate::cli_config::{
    ensure_parent_dir, read_json_file, read_toml_file, settings_path_for, strip_bom,
    unsupported_kind_error, write_atomically, write_json_file, write_toml_file,
};
use crate::commands::session::custom_session_name;
use crate::tray;
use chrono::Utc;
use serde_json::{Map, Value};
use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

const INTERNAL_PROFILE_META_KEY: &str = "__seshbuddyInternalProfileMeta";
const INTERNAL_PROFILE_CONTENT_KEY: &str = "__seshbuddyInternalProfileContent";
const INTERNAL_PROFILE_MANAGED_PATHS_KEY: &str = "managedPaths";
const CLAUDE_MANAGED_PROFILE_PATHS: &[&str] = &[
    "/env/ANTHROPIC_AUTH_TOKEN",
    "/env/ANTHROPIC_BASE_URL",
    "/env/ANTHROPIC_MODEL",
    "/effortLevel",
    "/env/CLAUDE_CODE_EFFORT_LEVEL",
    "/env/ANTHROPIC_DEFAULT_HAIKU_MODEL",
    "/env/ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME",
    "/env/ANTHROPIC_DEFAULT_SONNET_MODEL",
    "/env/ANTHROPIC_DEFAULT_SONNET_MODEL_NAME",
    "/env/ANTHROPIC_DEFAULT_OPUS_MODEL",
    "/env/ANTHROPIC_DEFAULT_OPUS_MODEL_NAME",
    "/env/API_TIMEOUT_MS",
    "/env/CLAUDE_CODE_MAX_OUTPUT_TOKENS",
    "/env/CLAUDE_CODE_DISABLE_EXPERIMENTAL_BETAS",
    "/env/CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC",
    "/attribution/commit",
    "/attribution/pr",
];
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

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CascadeApplyResult {
    pub success_count: usize,
    pub failed_dirs: Vec<String>,
    pub affected_scopes: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScopeBindingInfo {
    pub scope: String,
    /// 作用域显示名：项目作用域是用户数据（tab 名），全局作用域**没有**名字——
    /// 它的标签由前端按语言渲染（`api-profile.scope.global`），后端不回中文。
    pub name: Option<String>,
    pub is_global: bool,
    pub dirs: Vec<String>,
    pub active_profile: Option<String>,
}

#[derive(Debug, Clone)]
struct StoredProfile {
    visible: Value,
    managed_paths: Vec<String>,
}


fn codex_auth_path() -> AppResult<PathBuf> {
    Ok(cli::data_dir(CliKind::Codex)?.join("auth.json"))
}

fn validate_profile_name(name: String) -> AppResult<String> {
    let trimmed = name.trim().to_string();
    if trimmed.is_empty() {
        return Err(AppError::coded("profile.name_required"));
    }
    if trimmed.contains('/') || trimmed.contains('\\') {
        return Err(AppError::coded("profile.name_has_separator"));
    }
    Ok(trimmed)
}

/// 把内层错误压成一行说明文本，**保证它不会是裸 code**。
///
/// `Coded` 的 `Display` 就是 code 本身（见 `error.rs` 的 `#[error("{code}")]`），
/// 直接 `to_string()` 会让用户在「部分目录配置保存失败」的明细里看到
/// `cli_config.permission_denied` 这样的码。故 `Coded` 只取它的 `detail` 参数——
/// 那是底层 OS / 库文本，按 R3 不翻，本来就是要原样给用户看的那部分。
fn error_reason(err: &AppError) -> String {
    match err {
        AppError::Coded { params, .. } => params.get("detail").cloned().unwrap_or_default(),
        other => other.to_string(),
    }
}

/// 逐目录保存失败的两级组合。
///
/// 内层句子（哪个目录、为什么）**不能先拼成字符串再由外层本地化**——拼出来的那一刻
/// 文案就定死在 Rust 里了，而 `coded()` 产出的是 `{code, params}` 不是字符串，
/// 也塞不进 `Vec<String>`。故这里只把**不依赖语言的部分**（目录路径 + 底层原因）拼进
/// 一个 `{details}` 参数，句子骨架交给 `errors.profile.partial_dir_save_failed` 的四份语言包。
fn partial_dir_save_failed(failures: Vec<(String, String)>) -> AppError {
    let details = failures
        .iter()
        .map(|(dir, reason)| {
            if reason.is_empty() {
                dir.clone()
            } else {
                format!("{}: {}", dir, reason)
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    AppError::coded("profile.partial_dir_save_failed").with("details", details)
}

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

fn insert_json_string(target: &mut Map<String, Value>, key: &str, value: Option<&str>) {
    if let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) {
        target.insert(key.to_string(), Value::String(value.to_string()));
    }
}

const CODEX_OFFICIAL_PROFILE_NAME: &str = "Codex Official";

/// 检查 auth.json Value 是否包含 OAuth 登录态（而非仅 API key）
/// 参考 cc-switch: 只要有非 OPENAI_API_KEY、非 auth_mode 的有效字段即视为登录态
fn codex_auth_has_login_material(auth: &Value) -> bool {
    let Some(obj) = auth.as_object() else {
        return false;
    };
    obj.iter().any(|(k, v)| {
        if k == "auth_mode" || k == "OPENAI_API_KEY" {
            return false;
        }
        match v {
            Value::Null => false,
            Value::String(text) => !text.trim().is_empty(),
            Value::Array(items) => !items.is_empty(),
            Value::Object(map) => !map.is_empty(),
            _ => true,
        }
    })
}

fn value_has_content(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::String(text) => !text.trim().is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(map) => !map.is_empty(),
        _ => true,
    }
}

fn codex_settings_has_managed_provider_fields(settings: &Value) -> bool {
    let Some(codex) = settings.get("codex").and_then(Value::as_object) else {
        return false;
    };

    for key in [
        "base_url",
        "model",
        "model_reasoning_effort",
        "provider_name",
        "wire_api",
    ] {
        if codex.get(key).is_some_and(value_has_content) {
            return true;
        }
    }

    codex
        .get("model_provider")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|provider| {
            !provider.is_empty() && *provider != "openai-chat-completions"
        })
        .is_some()
}

fn codex_official_profile_content_from_live(settings: &Value) -> Value {
    let mut official = Map::new();
    if let Some(auth_obj) = settings
        .get("auth")
        .and_then(Value::as_object)
        .filter(|auth| !auth.is_empty())
    {
        let mut filtered = auth_obj.clone();
        filtered.remove("OPENAI_API_KEY");
        if !filtered.is_empty() {
            official.insert("auth".to_string(), Value::Object(filtered));
        }
    }
    Value::Object(official)
}

fn profile_value_matches_settings(profile: &Value, settings: &Value, pointer: &str) -> Option<bool> {
    let profile_value = profile.pointer(pointer)?;
    if !value_has_content(profile_value) {
        return None;
    }
    Some(settings.pointer(pointer) == Some(profile_value))
}

fn codex_profile_matches_live_settings(profile: &Value, settings: &Value) -> bool {
    let mut has_configured_field = false;

    // 比较 API Key：profile 可能是新版 /codex/experimental_bearer_token 或旧版 /auth/OPENAI_API_KEY
    let profile_token = profile
        .pointer("/codex/experimental_bearer_token")
        .or_else(|| profile.pointer("/auth/OPENAI_API_KEY"));
    let settings_token = settings
        .pointer("/codex/experimental_bearer_token")
        .or_else(|| settings.pointer("/auth/OPENAI_API_KEY"));

    if let (Some(p_tok), Some(s_tok)) = (profile_token, settings_token) {
        if value_has_content(p_tok) {
            has_configured_field = true;
            if p_tok != s_tok {
                return false;
            }
        }
    } else if let Some(p_tok) = profile_token {
        if value_has_content(p_tok) {
            return false;
        }
    }

    for pointer in [
        "/codex/base_url",
        "/codex/model",
        "/codex/model_reasoning_effort",
        "/codex/model_provider",
        "/codex/provider_name",
        "/codex/wire_api",
    ] {
        match profile_value_matches_settings(profile, settings, pointer) {
            Some(true) => has_configured_field = true,
            Some(false) => return false,
            None => {}
        }
    }

    has_configured_field
}

fn set_matching_codex_profile_active(settings: &Value) -> AppResult<bool> {
    for name in list_profiles_for(CliKind::Codex, None)? {
        if name == CODEX_OFFICIAL_PROFILE_NAME {
            continue;
        }
        let Ok(raw) = app_db::read_profile(CliKind::Codex, &name) else {
            continue;
        };
        let Ok(stored) = decode_stored_profile_for_name(CliKind::Codex, Some(&name), &raw) else {
            continue;
        };
        if codex_profile_matches_live_settings(&stored.visible, settings) {
            set_active_profile_for(CliKind::Codex, &name, None)?;
            return Ok(true);
        }
    }
    Ok(false)
}

fn read_codex_settings_value() -> AppResult<Value> {
    let config = read_toml_file(&settings_path_for(CliKind::Codex)?)?;
    let auth = read_json_file(&codex_auth_path()?, "{}")?;

    let mut root = Map::new();

    if let Some(auth_obj) = auth.as_object().filter(|obj| !obj.is_empty()) {
        root.insert("auth".to_string(), Value::Object(auth_obj.clone()));
    }

    let mut codex = Map::new();
    insert_json_string(
        &mut codex,
        "model",
        config.get("model").and_then(|v| v.as_str()),
    );
    insert_json_string(
        &mut codex,
        "model_reasoning_effort",
        config
            .get("model_reasoning_effort")
            .and_then(|v| v.as_str()),
    );

    let model_provider = config
        .get("model_provider")
        .and_then(|v| v.as_str())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("openai-chat-completions")
        .to_string();
    insert_json_string(&mut codex, "model_provider", Some(&model_provider));

    let provider_table = config
        .get("model_providers")
        .and_then(|v| v.as_table())
        .and_then(|providers| providers.get(&model_provider))
        .and_then(|v| v.as_table());

    insert_json_string(
        &mut codex,
        "provider_name",
        provider_table
            .and_then(|provider| provider.get("name"))
            .and_then(|v| v.as_str()),
    );
    insert_json_string(
        &mut codex,
        "base_url",
        provider_table
            .and_then(|provider| provider.get("base_url"))
            .and_then(|v| v.as_str()),
    );
    insert_json_string(
        &mut codex,
        "wire_api",
        provider_table
            .and_then(|provider| provider.get("wire_api"))
            .and_then(|v| v.as_str()),
    );
    insert_json_string(
        &mut codex,
        "experimental_bearer_token",
        provider_table
            .and_then(|provider| provider.get("experimental_bearer_token"))
            .or_else(|| config.get("experimental_bearer_token"))
            .and_then(|v| v.as_str()),
    );

    if !codex.is_empty() {
        root.insert("codex".to_string(), Value::Object(codex));
    }

    // 仅在 auth.json 包含真正的官方 OAuth 登录态时保留 auth 节点（供官方 profile 消费），
    // 且剔除其中的 OPENAI_API_KEY，确保第三方 profile 纯净无多余 auth 节点
    if codex_auth_has_login_material(&auth) {
        let mut auth_map = auth.as_object().cloned().unwrap_or_default();
        auth_map.remove("OPENAI_API_KEY");
        if !auth_map.is_empty() {
            root.insert("auth".to_string(), Value::Object(auth_map));
        }
    }

    Ok(Value::Object(root))
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

    // 参考 cc-switch 官方登录保护机制：
    // 第三方 API Key 已注入 config.toml 的 [model_providers.<id>].experimental_bearer_token 中。
    // auth.json 专用于保留 ChatGPT / Codex 官方 OAuth 登录态（远程操作和官方插件强依赖）。
    // 若现有 auth.json 已有官方 OAuth 登录态，切勿覆盖；
    // 仅在无官方登录态且当前 Profile 提供自定义 auth 时才写入。
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

fn escape_json_pointer_segment(segment: &str) -> String {
    segment.replace('~', "~0").replace('/', "~1")
}

fn unescape_json_pointer_segment(segment: &str) -> String {
    let mut unescaped = String::new();
    let mut chars = segment.chars();

    while let Some(ch) = chars.next() {
        if ch == '~' {
            match chars.next() {
                Some('0') => unescaped.push('~'),
                Some('1') => unescaped.push('/'),
                Some(other) => {
                    unescaped.push('~');
                    unescaped.push(other);
                }
                None => unescaped.push('~'),
            }
        } else {
            unescaped.push(ch);
        }
    }

    unescaped
}

fn collect_json_leaf_pointers(value: &Value, base: &str, output: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            if map.is_empty() {
                if !base.is_empty() {
                    output.push(base.to_string());
                }
                return;
            }

            let mut entries: Vec<_> = map.iter().collect();
            entries.sort_by(|(left_key, _), (right_key, _)| left_key.cmp(right_key));
            for (key, child) in entries {
                let next = if base.is_empty() {
                    format!("/{}", escape_json_pointer_segment(key))
                } else {
                    format!("{}/{}", base, escape_json_pointer_segment(key))
                };
                collect_json_leaf_pointers(child, &next, output);
            }
        }
        _ => {
            if !base.is_empty() {
                output.push(base.to_string());
            }
        }
    }
}

fn normalized_json_leaf_pointers(value: &Value) -> Vec<String> {
    let mut pointers = Vec::new();
    collect_json_leaf_pointers(value, "", &mut pointers);
    let mut unique = BTreeSet::new();
    for pointer in pointers {
        let trimmed = pointer.trim();
        if !trimmed.is_empty() {
            unique.insert(trimmed.to_string());
        }
    }
    unique.into_iter().collect()
}

fn parse_json_pointer(pointer: &str) -> Vec<String> {
    if pointer.is_empty() {
        return Vec::new();
    }

    pointer
        .split('/')
        .skip(1)
        .map(unescape_json_pointer_segment)
        .collect()
}

fn extract_managed_paths(meta: &Value, visible: &Value) -> Vec<String> {
    let Some(meta_obj) = meta.as_object() else {
        return normalized_json_leaf_pointers(visible);
    };
    let Some(paths) = meta_obj
        .get(INTERNAL_PROFILE_MANAGED_PATHS_KEY)
        .and_then(|value| value.as_array())
    else {
        return normalized_json_leaf_pointers(visible);
    };

    let mut unique = BTreeSet::new();
    for path in paths {
        let Some(path_str) = path.as_str() else {
            continue;
        };
        let trimmed = path_str.trim();
        if !trimmed.is_empty() {
            unique.insert(trimmed.to_string());
        }
    }

    if unique.is_empty() {
        normalized_json_leaf_pointers(visible)
    } else {
        unique.into_iter().collect()
    }
}

fn allowed_profile_paths_for_name(kind: CliKind, name: Option<&str>) -> &'static [&'static str] {
    match kind {
        CliKind::Claude => CLAUDE_MANAGED_PROFILE_PATHS,
        CliKind::Codex if name == Some(CODEX_OFFICIAL_PROFILE_NAME) => {
            CODEX_OFFICIAL_MANAGED_PROFILE_PATHS
        }
        CliKind::Codex => CODEX_MANAGED_PROFILE_PATHS,
        CliKind::Gemini => &[],
        CliKind::WorkBuddy => &[],
        CliKind::Dsh => &[],
        CliKind::Antigravity => &[],
    }
}

fn sanitize_profile_value_for_name(kind: CliKind, name: Option<&str>, visible: &Value) -> Value {
    let mut sanitized = Value::Object(Map::new());
    for path in allowed_profile_paths_for_name(kind, name) {
        if let Some(value) = visible.pointer(path) {
            set_json_pointer_value(&mut sanitized, path, value.clone());
        }
    }
    sanitized
}

fn normalize_managed_paths_for_name(
    kind: CliKind,
    name: Option<&str>,
    managed_paths: &[String],
    visible: &Value,
) -> Vec<String> {
    let allowed = allowed_profile_paths_for_name(kind, name);
    let mut unique = BTreeSet::new();

    for path in managed_paths {
        let trimmed = path.trim();
        if !trimmed.is_empty() && allowed.contains(&trimmed) {
            unique.insert(trimmed.to_string());
        }
    }

    for path in normalized_json_leaf_pointers(visible) {
        if allowed.contains(&path.as_str()) {
            unique.insert(path);
        }
    }

    unique.into_iter().collect()
}

fn decode_stored_profile_for_name(
    kind: CliKind,
    name: Option<&str>,
    content: &str,
) -> AppResult<StoredProfile> {
    let value: Value =
        serde_json::from_str(content).map_err(|e| AppError::coded("profile.content_parse_failed").with("detail", e.to_string()))?;

    if let Some(root) = value.as_object() {
        let meta_and_visible = root
            .get(INTERNAL_PROFILE_META_KEY)
            .and_then(|m| root.get(INTERNAL_PROFILE_CONTENT_KEY).map(|v| (m, v)));

        if let Some((meta, visible)) = meta_and_visible {
            let sanitized_visible = sanitize_profile_value_for_name(kind, name, visible);
            return Ok(StoredProfile {
                managed_paths: normalize_managed_paths_for_name(
                    kind,
                    name,
                    &extract_managed_paths(meta, visible),
                    &sanitized_visible,
                ),
                visible: sanitized_visible,
            });
        }
    }

    let sanitized_visible = sanitize_profile_value_for_name(kind, name, &value);
    Ok(StoredProfile {
        managed_paths: normalize_managed_paths_for_name(
            kind,
            name,
            &normalized_json_leaf_pointers(&value),
            &sanitized_visible,
        ),
        visible: sanitized_visible,
    })
}

fn wrap_profile_content_for_name(
    kind: CliKind,
    name: Option<&str>,
    visible: &Value,
    previous_managed_paths: &[String],
) -> Value {
    let sanitized_visible = sanitize_profile_value_for_name(kind, name, visible);
    let managed_paths =
        normalize_managed_paths_for_name(kind, name, previous_managed_paths, &sanitized_visible);

    if managed_paths.is_empty() {
        return sanitized_visible;
    }

    let mut meta = Map::new();
    meta.insert(
        INTERNAL_PROFILE_MANAGED_PATHS_KEY.to_string(),
        Value::Array(managed_paths.into_iter().map(Value::String).collect()),
    );

    let mut root = Map::new();
    root.insert(INTERNAL_PROFILE_META_KEY.to_string(), Value::Object(meta));
    root.insert(INTERNAL_PROFILE_CONTENT_KEY.to_string(), sanitized_visible);
    Value::Object(root)
}

fn ensure_json_object(value: &mut Value) {
    if !value.is_object() {
        *value = Value::Object(Map::new());
    }
}

fn set_json_pointer_value(target: &mut Value, pointer: &str, next_value: Value) {
    let segments = parse_json_pointer(pointer);
    if segments.is_empty() {
        *target = next_value;
        return;
    }

    ensure_json_object(target);
    let mut current = target;
    for segment in &segments[..segments.len() - 1] {
        ensure_json_object(current);
        let map = current.as_object_mut().expect("对象已初始化");
        let entry = map
            .entry(segment.clone())
            .or_insert_with(|| Value::Object(Map::new()));
        if !entry.is_object() {
            *entry = Value::Object(Map::new());
        }
        current = entry;
    }

    ensure_json_object(current);
    current
        .as_object_mut()
        .expect("对象已初始化")
        .insert(segments.last().cloned().unwrap_or_default(), next_value);
}

fn remove_json_pointer_segments(target: &mut Value, segments: &[String]) -> bool {
    let Some(map) = target.as_object_mut() else {
        return false;
    };

    let key = &segments[0];
    if segments.len() == 1 {
        map.remove(key);
        return map.is_empty();
    }

    let should_remove_child = map
        .get_mut(key)
        .map(|child| remove_json_pointer_segments(child, &segments[1..]))
        .unwrap_or(false);
    if should_remove_child {
        map.remove(key);
    }

    map.is_empty()
}

fn remove_json_pointer_value(target: &mut Value, pointer: &str) {
    let segments = parse_json_pointer(pointer);
    if segments.is_empty() {
        *target = Value::Object(Map::new());
        return;
    }

    let _ = remove_json_pointer_segments(target, &segments);
}

fn apply_claude_settings_value_to_path(settings_path: &PathBuf, content: &Value, managed_paths: &[String]) -> AppResult<()> {
    ensure_parent_dir(settings_path)?;

    let mut existing = if settings_path.exists() {
        read_json_file(settings_path, "{}")?
    } else {
        Value::Object(Map::new())
    };
    ensure_json_object(&mut existing);

    let paths = if managed_paths.is_empty() {
        normalized_json_leaf_pointers(content)
    } else {
        managed_paths.to_vec()
    };

    for path in paths {
        match content.pointer(&path) {
            Some(value) => set_json_pointer_value(&mut existing, &path, value.clone()),
            None => remove_json_pointer_value(&mut existing, &path),
        }
    }

    write_json_file(settings_path, &existing)
}

fn read_settings_json_for(kind: CliKind) -> AppResult<String> {
    match kind {
        CliKind::Claude => {
            let path = settings_path_for(kind)?;
            if !path.exists() {
                return Ok("{}".to_string());
            }
            fs::read_to_string(&path)
                .map(strip_bom)
                .map_err(|e| AppError::coded("profile.settings_read_failed").with("detail", e.to_string()))
        }
        CliKind::Codex => {
            let value = read_codex_settings_value()?;
            serde_json::to_string_pretty(&value).map_err(|e| AppError::coded("profile.serialize_failed").with("detail", e.to_string()))
        }
        CliKind::Gemini => Err(unsupported_kind_error(kind)),
        CliKind::WorkBuddy => Err(unsupported_kind_error(kind)),
        CliKind::Dsh => Err(unsupported_kind_error(kind)),
        CliKind::Antigravity => Err(unsupported_kind_error(kind)),
    }
}

fn write_settings_json_for(kind: CliKind, content: String) -> AppResult<()> {
    let value: Value =
        serde_json::from_str(&content).map_err(|e| AppError::coded("profile.content_parse_failed").with("detail", e.to_string()))?;

    match kind {
        CliKind::Claude => {
            let path = settings_path_for(kind)?;
            ensure_parent_dir(&path)?;
            write_atomically(&path, content.as_bytes())?;
        }
        CliKind::Codex => {
            apply_codex_settings_value(&value)?;
        }
        CliKind::Gemini => return Err(unsupported_kind_error(kind)),
        CliKind::WorkBuddy => return Err(unsupported_kind_error(kind)),
        CliKind::Dsh => return Err(unsupported_kind_error(kind)),
        CliKind::Antigravity => return Err(unsupported_kind_error(kind)),
    }

    Ok(())
}

fn list_profiles_for(kind: CliKind, _scope: Option<&str>) -> AppResult<Vec<String>> {
    app_db::list_profiles(kind)
}

fn read_profile_for(kind: CliKind, name: String, _scope: Option<&str>) -> AppResult<String> {
    let name = validate_profile_name(name)?;
    let raw = app_db::read_profile(kind, &name)?;
    let stored = decode_stored_profile_for_name(kind, Some(&name), &raw)?;
    serde_json::to_string_pretty(&stored.visible).map_err(|e| AppError::coded("profile.serialize_failed").with("detail", e.to_string()))
}

fn save_profile_for_internal(
    kind: CliKind,
    name: String,
    content: String,
    allow_official_overwrite: bool,
    _scope: Option<&str>,
) -> AppResult<()> {
    let name = validate_profile_name(name)?;
    if matches!(kind, CliKind::Codex)
        && name == CODEX_OFFICIAL_PROFILE_NAME
        && !allow_official_overwrite
    {
        // 允许首次创建，但阻止覆盖更新
        if app_db::read_profile(kind, &name).is_ok() {
            return Err(AppError::coded("profile.builtin_cannot_overwrite"));
        }
    }
    let visible: Value =
        serde_json::from_str(&content).map_err(|e| AppError::coded("profile.content_parse_failed").with("detail", e.to_string()))?;

    let previous_managed_paths = app_db::read_profile(kind, &name)
        .ok()
        .and_then(|raw| {
            decode_stored_profile_for_name(kind, Some(&name), &raw)
                .ok()
                .map(|stored| stored.managed_paths)
        })
        .unwrap_or_default();
    let stored =
        wrap_profile_content_for_name(kind, Some(&name), &visible, &previous_managed_paths);
    let stored_content =
        serde_json::to_string_pretty(&stored).map_err(|e| AppError::coded("profile.serialize_failed").with("detail", e.to_string()))?;

    app_db::save_profile(kind, &name, &stored_content)
}

fn cascade_apply_profile_for(kind: CliKind, profile_name: &str) -> AppResult<CascadeApplyResult> {
    let conn = app_db::conn()?;
    cascade_apply_profile_with(&conn, kind, profile_name, None)
}

pub(crate) fn cascade_apply_profile_with(
    conn: &rusqlite::Connection,
    kind: CliKind,
    profile_name: &str,
    global_override: Option<&PathBuf>,
) -> AppResult<CascadeApplyResult> {
    let raw = app_db::read_profile_with(conn, kind, profile_name)?;
    let stored = decode_stored_profile_for_name(kind, Some(profile_name), &raw)?;
    let active_scopes = app_db::get_active_scopes_for_profile_with(conn, kind, profile_name)?;

    let mut success_count = 0;
    let mut failed_dirs = Vec::new();
    let mut affected_scopes = Vec::new();

    for scope_str in active_scopes {
        affected_scopes.push(scope_str.clone());
        match kind {
            CliKind::Claude => {
                if scope_str == "global" {
                    let settings_path = match global_override {
                        Some(p) => p.clone(),
                        None => settings_path_for(CliKind::Claude)?,
                    };
                    match apply_claude_settings_value_to_path(&settings_path, &stored.visible, &stored.managed_paths) {
                        Ok(()) => success_count += 1,
                        Err(e) => {
                            tracing::warn!("Failed to cascade apply profile '{}' to global: {}", profile_name, e.diagnostic());
                            failed_dirs.push(settings_path.to_string_lossy().to_string());
                        }
                    }
                } else {
                    let dirs = app_db::get_profile_tab_dirs_with(conn, &scope_str)?;
                    for dir in dirs {
                        let settings_path = PathBuf::from(&dir).join(".claude").join("settings.json");
                        match apply_claude_settings_value_to_path(&settings_path, &stored.visible, &stored.managed_paths) {
                            Ok(()) => success_count += 1,
                            Err(e) => {
                                tracing::warn!("Failed to cascade apply profile '{}' to dir '{}': {}", profile_name, dir, e.diagnostic());
                                failed_dirs.push(dir);
                            }
                        }
                    }
                }
            }
            CliKind::Codex => {
                if scope_str == "global" {
                    let res = if profile_name == CODEX_OFFICIAL_PROFILE_NAME {
                        apply_codex_official_profile(&stored.visible)
                    } else {
                        apply_codex_settings_value(&stored.visible)
                    };
                    match res {
                        Ok(()) => success_count += 1,
                        Err(e) => {
                            tracing::warn!("Failed to cascade apply codex profile '{}': {}", profile_name, e.diagnostic());
                            if let Ok(p) = settings_path_for(CliKind::Codex) {
                                failed_dirs.push(p.to_string_lossy().to_string());
                            }
                        }
                    }
                }
            }
            CliKind::Gemini | CliKind::WorkBuddy | CliKind::Dsh | CliKind::Antigravity => {}
        }
    }

    Ok(CascadeApplyResult {
        success_count,
        failed_dirs,
        affected_scopes,
    })
}

fn save_profile_for(kind: CliKind, name: String, content: String) -> AppResult<CascadeApplyResult> {
    save_profile_for_internal(kind, name.clone(), content, false, None)?;
    cascade_apply_profile_for(kind, &name)
}

fn clean_claude_settings_for_unbind(settings_path: &PathBuf) -> AppResult<()> {
    if !settings_path.exists() {
        return Ok(());
    }
    let mut existing = match read_json_file(settings_path, "{}") {
        Ok(val) => val,
        Err(_) => return Ok(()),
    };
    ensure_json_object(&mut existing);

    for p in CLAUDE_MANAGED_PROFILE_PATHS {
        remove_json_pointer_value(&mut existing, p);
    }
    remove_json_pointer_value(&mut existing, "/env/ANTHROPIC_API_KEY");
    remove_json_pointer_value(&mut existing, "/env/OPENAI_API_KEY");
    remove_json_pointer_value(&mut existing, "/env/OPENAI_BASE_URL");
    remove_json_pointer_value(&mut existing, "/model");
    remove_json_pointer_value(&mut existing, "/customModels");

    if let Some(env_val) = existing.pointer("/env") {
        if let Some(map) = env_val.as_object() {
            if map.is_empty() {
                remove_json_pointer_value(&mut existing, "/env");
            }
        }
    }
    if let Some(attr_val) = existing.pointer("/attribution") {
        if let Some(map) = attr_val.as_object() {
            if map.is_empty() {
                remove_json_pointer_value(&mut existing, "/attribution");
            }
        }
    }

    if let Some(map) = existing.as_object() {
        if map.is_empty() {
            let _ = fs::remove_file(settings_path);
            return Ok(());
        }
    }

    write_json_file(settings_path, &existing)
}

pub(crate) fn set_scope_binding_with(
    conn: &rusqlite::Connection,
    kind: CliKind,
    scope: &str,
    profile_name: &str,
    global_override: Option<&PathBuf>,
) -> AppResult<()> {
    let trimmed = profile_name.trim();
    if trimmed.is_empty() {
        if scope == "global" {
            return Err(AppError::coded("profile.global_scope_needs_profile"));
        }
        app_db::clear_active_profile_with(conn, kind, Some(scope))?;
        if kind == CliKind::Claude {
            if let Ok(dirs) = app_db::get_profile_tab_dirs_with(conn, scope) {
                for dir in &dirs {
                    let settings_path = PathBuf::from(dir).join(".claude").join("settings.json");
                    if settings_path.exists() {
                        let _ = clean_claude_settings_for_unbind(&settings_path);
                    }
                }
            }
        }
        return Ok(());
    }

    let name = validate_profile_name(profile_name.to_string())?;
    let profile_content = app_db::read_profile_with(conn, kind, &name)?;
    let stored = decode_stored_profile_for_name(kind, Some(&name), &profile_content)?;

    match kind {
        CliKind::Claude => {
            if scope == "global" {
                let settings_path = match global_override {
                    Some(p) => p.clone(),
                    None => settings_path_for(CliKind::Claude)?,
                };
                apply_claude_settings_value_to_path(&settings_path, &stored.visible, &stored.managed_paths)?;
            } else {
                let dirs = app_db::get_profile_tab_dirs_with(conn, scope)?;
                if dirs.is_empty() {
                    return Err(AppError::coded("profile.project_tab_no_dirs"));
                }
                let mut failures = Vec::new();
                for dir in &dirs {
                    let settings_path = PathBuf::from(dir).join(".claude").join("settings.json");
                    if let Err(e) = apply_claude_settings_value_to_path(&settings_path, &stored.visible, &stored.managed_paths) {
                        failures.push((dir.clone(), error_reason(&e)));
                    }
                }
                if !failures.is_empty() {
                    return Err(partial_dir_save_failed(failures));
                }
            }
        }
        CliKind::Codex => {
            if scope != "global" {
                return Err(AppError::coded("profile.codex_no_project_binding"));
            }

            // Backfill: 将当前 live 状态写回旧的 active profile
            let active_name = app_db::get_active_profile_with(conn, CliKind::Codex, None).unwrap_or_default();
            if !active_name.trim().is_empty() && active_name != name {
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
                    let official_exists = app_db::list_profiles_with(conn, CliKind::Codex)
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

            if name == CODEX_OFFICIAL_PROFILE_NAME {
                apply_codex_official_profile(&stored.visible)?;
            } else {
                apply_codex_settings_value(&stored.visible)?;
            }
        }
        CliKind::Gemini => return Err(unsupported_kind_error(kind)),
        CliKind::WorkBuddy => return Err(unsupported_kind_error(kind)),
        CliKind::Dsh => return Err(unsupported_kind_error(kind)),
        CliKind::Antigravity => return Err(unsupported_kind_error(kind)),
    }

    app_db::set_active_profile_with(conn, kind, &name, Some(scope))?;
    Ok(())
}

fn set_scope_binding_for(kind: CliKind, scope: &str, profile_name: &str) -> AppResult<()> {
    let conn = app_db::conn()?;
    set_scope_binding_with(&conn, kind, scope, profile_name, None)
}

fn apply_profile_for(kind: CliKind, name: String, scope: Option<&str>) -> AppResult<()> {
    let scope_str = scope.unwrap_or("global");
    set_scope_binding_for(kind, scope_str, &name)
}

pub(crate) fn get_scope_bindings_with(
    conn: &rusqlite::Connection,
    kind: CliKind,
) -> AppResult<Vec<ScopeBindingInfo>> {
    let global_active_raw = app_db::get_active_profile_with(conn, kind, Some("global"))?;
    let global_active = if global_active_raw.trim().is_empty() {
        None
    } else {
        Some(global_active_raw)
    };

    let mut bindings = vec![ScopeBindingInfo {
        scope: "global".to_string(),
        name: None,
        is_global: true,
        dirs: vec![],
        active_profile: global_active,
    }];

    let tabs = app_db::list_profile_tabs_with(conn, kind.id())?;
    for tab in tabs {
        let tab_active_raw = app_db::get_active_profile_with(conn, kind, Some(&tab.id))?;
        let tab_active = if tab_active_raw.trim().is_empty() {
            None
        } else {
            Some(tab_active_raw)
        };
        bindings.push(ScopeBindingInfo {
            scope: tab.id,
            name: Some(tab.name),
            is_global: false,
            dirs: tab.dirs,
            active_profile: tab_active,
        });
    }

    Ok(bindings)
}

fn get_scope_bindings_for(kind: CliKind) -> AppResult<Vec<ScopeBindingInfo>> {
    let conn = app_db::conn()?;
    get_scope_bindings_with(&conn, kind)
}


fn delete_profile_for(kind: CliKind, name: String, _scope: Option<&str>) -> AppResult<()> {
    let name = validate_profile_name(name)?;
    app_db::delete_profile(kind, &name)
}

fn set_active_profile_for(kind: CliKind, name: &str, scope: Option<&str>) -> AppResult<()> {
    app_db::set_active_profile(kind, name, scope)
}

fn get_active_profile_for(kind: CliKind, scope: Option<&str>) -> AppResult<String> {
    app_db::get_active_profile(kind, scope)
}

fn sync_active_profile_from_cli_for(kind: CliKind) -> AppResult<bool> {
    if matches!(kind, CliKind::Codex) {
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
            return Ok(true);
        }

        let active_name = get_active_profile_for(CliKind::Codex, None)?;
        if active_name == CODEX_OFFICIAL_PROFILE_NAME {
            if !set_matching_codex_profile_active(&current_settings)? {
                app_db::clear_active_profile(CliKind::Codex, None)?;
            }
            return Ok(true);
        }
    }

    let active_name = get_active_profile_for(kind, None)?;
    if active_name.trim().is_empty() {
        return Ok(false);
    }

    let current_settings = read_settings_json_for(kind)?;
    let allow_official_overwrite =
        matches!(kind, CliKind::Codex) && active_name == CODEX_OFFICIAL_PROFILE_NAME;
    save_profile_for_internal(
        kind,
        active_name,
        current_settings,
        allow_official_overwrite,
        None,
    )?;
    Ok(true)
}

fn rename_profile_for(kind: CliKind, old_name: String, new_name: String, _scope: Option<&str>) -> AppResult<()> {
    let old_name = validate_profile_name(old_name)?;
    let new_name = validate_profile_name(new_name)?;
    if matches!(kind, CliKind::Codex)
        && (old_name == CODEX_OFFICIAL_PROFILE_NAME || new_name == CODEX_OFFICIAL_PROFILE_NAME)
    {
        return Err(AppError::coded("profile.builtin_cannot_rename"));
    }
    app_db::rename_profile(kind, &old_name, &new_name)
}

fn build_profile_settings_file_for(kind: CliKind, name: String, _scope: Option<&str>) -> AppResult<String> {
    let name = validate_profile_name(name)?;
    let profile_content = app_db::read_profile(kind, &name)?;
    let stored = decode_stored_profile_for_name(kind, Some(&name), &profile_content)?;

    let settings_path = settings_path_for(kind)?;
    let mut base = if settings_path.exists() {
        read_json_file(&settings_path, "{}")?
    } else {
        Value::Object(Map::new())
    };
    ensure_json_object(&mut base);

    let paths = if stored.managed_paths.is_empty() {
        normalized_json_leaf_pointers(&stored.visible)
    } else {
        stored.managed_paths.clone()
    };

    for path in &paths {
        match stored.visible.pointer(path) {
            Some(value) => set_json_pointer_value(&mut base, path, value.clone()),
            None => remove_json_pointer_value(&mut base, path),
        }
    }

    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or_default();
    let pid = std::process::id();
    let temp_dir = if cfg!(target_os = "macos") {
        PathBuf::from("/tmp")
    } else {
        std::env::temp_dir()
    };
    let file_path = temp_dir.join(format!("seshbuddy-settings-{}-{}.json", pid, timestamp));

    write_json_file(&file_path, &base)?;
    Ok(file_path.to_string_lossy().to_string())
}

pub fn build_claude_temp_settings_file() -> AppResult<String> {
    let settings_path = settings_path_for(CliKind::Claude)?;
    let base = if settings_path.exists() {
        read_json_file(&settings_path, "{}")?
    } else {
        Value::Object(Map::new())
    };

    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or_default();
    let pid = std::process::id();
    let temp_dir = if cfg!(target_os = "macos") {
        PathBuf::from("/tmp")
    } else {
        std::env::temp_dir()
    };
    let file_path = temp_dir.join(format!("seshbuddy-settings-{}-{}.json", pid, timestamp));

    write_json_file(&file_path, &base)?;
    Ok(file_path.to_string_lossy().to_string())
}

#[tauri::command]
pub fn list_profiles(cli_id: Option<String>, scope: Option<String>) -> AppResult<Vec<String>> {
    let kind = CliKind::from_id(cli_id.as_deref())?;
    list_profiles_for(kind, scope.as_deref())
}

#[tauri::command]
pub fn read_profile(cli_id: Option<String>, name: String, scope: Option<String>) -> AppResult<String> {
    let kind = CliKind::from_id(cli_id.as_deref())?;
    read_profile_for(kind, name, scope.as_deref())
}

#[tauri::command]
pub fn save_profile(
    cli_id: Option<String>,
    name: String,
    content: String,
) -> AppResult<CascadeApplyResult> {
    let kind = CliKind::from_id(cli_id.as_deref())?;
    save_profile_for(kind, name, content)
}

#[tauri::command]
pub fn delete_profile(cli_id: Option<String>, name: String, scope: Option<String>) -> AppResult<()> {
    let kind = CliKind::from_id(cli_id.as_deref())?;
    delete_profile_for(kind, name, scope.as_deref())
}

#[tauri::command]
pub fn get_scope_bindings(cli_id: Option<String>) -> AppResult<Vec<ScopeBindingInfo>> {
    let kind = CliKind::from_id(cli_id.as_deref())?;
    get_scope_bindings_for(kind)
}

#[tauri::command]
pub fn set_scope_binding(
    cli_id: Option<String>,
    scope: String,
    profile_name: String,
) -> AppResult<()> {
    let kind = CliKind::from_id(cli_id.as_deref())?;
    set_scope_binding_for(kind, &scope, &profile_name)
}

#[tauri::command]
pub fn apply_profile(cli_id: Option<String>, name: String, scope: Option<String>) -> AppResult<()> {
    let kind = CliKind::from_id(cli_id.as_deref())?;
    apply_profile_for(kind, name, scope.as_deref())
}

#[tauri::command]
pub fn rename_profile(
    cli_id: Option<String>,
    old_name: String,
    new_name: String,
    scope: Option<String>,
) -> AppResult<()> {
    let kind = CliKind::from_id(cli_id.as_deref())?;
    rename_profile_for(kind, old_name, new_name, scope.as_deref())
}

#[tauri::command]
pub fn get_active_profile(cli_id: Option<String>, scope: Option<String>) -> AppResult<String> {
    let kind = CliKind::from_id(cli_id.as_deref())?;
    get_active_profile_for(kind, scope.as_deref())
}

fn read_scope_settings_for(kind: CliKind, scope: Option<&str>) -> AppResult<String> {
    let scope_str = scope.unwrap_or("global");
    if matches!(kind, CliKind::Codex) {
        let path = settings_path_for(kind)?;
        if !path.exists() {
            return Ok(String::new());
        }
        return fs::read_to_string(&path)
            .map(strip_bom)
            .map_err(|e| AppError::coded("profile.settings_read_failed").with("detail", e.to_string()));
    }

    if scope_str == "global" || !matches!(kind, CliKind::Claude) {
        return read_settings_json_for(kind);
    }

    let tabs = app_db::list_profile_tabs(kind.id())?;
    let tab = tabs.iter().find(|t| t.id == scope_str);
    if let Some(t) = tab {
        for dir in &t.dirs {
            let path = std::path::PathBuf::from(dir).join(".claude").join("settings.json");
            if path.exists() {
                if let Ok(content) = fs::read_to_string(&path).map(strip_bom) {
                    return Ok(content);
                }
            }
        }
    }

    Ok("{}".to_string())
}

fn write_scope_settings_for(kind: CliKind, scope: Option<&str>, content: String) -> AppResult<()> {
    let scope_str = scope.unwrap_or("global");
    if matches!(kind, CliKind::Codex) {
        content
            .parse::<toml::Value>()
            .map_err(|e| AppError::coded("profile.toml_invalid").with("detail", e.to_string()))?;
        let path = settings_path_for(kind)?;
        ensure_parent_dir(&path)?;
        return write_atomically(&path, content.as_bytes());
    }

    if scope_str == "global" || !matches!(kind, CliKind::Claude) {
        return write_settings_json_for(kind, content);
    }

    let value: Value =
        serde_json::from_str(&content).map_err(|e| AppError::coded("profile.content_parse_failed").with("detail", e.to_string()))?;

    let tabs = app_db::list_profile_tabs(kind.id())?;
    let tab = tabs.iter().find(|t| t.id == scope_str);
    if let Some(t) = tab {
        let mut failures = Vec::new();
        for dir in &t.dirs {
            let path = std::path::PathBuf::from(dir).join(".claude").join("settings.json");
            if let Err(e) = write_json_file(&path, &value) {
                failures.push((dir.clone(), error_reason(&e)));
            }
        }
        if !failures.is_empty() {
            return Err(partial_dir_save_failed(failures));
        }
    }

    Ok(())
}

#[tauri::command]
pub fn read_scope_settings(cli_id: Option<String>, scope: Option<String>) -> AppResult<String> {
    let kind = CliKind::from_id(cli_id.as_deref())?;
    read_scope_settings_for(kind, scope.as_deref())
}

#[tauri::command]
pub fn write_scope_settings(cli_id: Option<String>, scope: Option<String>, content: String) -> AppResult<()> {
    let kind = CliKind::from_id(cli_id.as_deref())?;
    write_scope_settings_for(kind, scope.as_deref(), content)
}

// ─── 项目作用域「配置感知」：检测 / 导入 / 状态 ───

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DirConfigInfo {
    pub dir: String,
    pub has_config: bool,
    pub base_url: Option<String>,
    pub model: Option<String>,
    pub matched_profile: Option<String>,
    /// token 的非加密短哈希，仅用于前端比较各目录配置是否一致，不暴露原始 token
    pub token_hash: Option<String>,
}

/// 目录状态徽标的附加数据。**不是文案**——前端按 `kind` 分派、渲染本地化 tooltip，
/// 故这里只回结构化数据，不回拼好的句子（拼出来就把语言定死在返回值里）。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ScopeDirDetail {
    /// 磁盘配置与已绑定配置不一致：承载第一个不一致的目录路径（用户数据）。
    Diverged { dir: String },
    /// 目录里有连接配置、但该作用域当前未绑定 profile：承载该目录的 base_url。
    UnboundWithConfig { base_url: String },
    /// 该作用域下各目录的连接配置互不一致：承载参与比较的目录列表
    /// （即「有 base_url 或 token 的目录」，其长度就是原来的计数）。
    DirsInconsistent { dirs: Vec<String> },
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScopeDirStatus {
    pub scope: String,
    /// aligned | unboundWithConfig | diverged | dirsInconsistent | none
    pub status: String,
    /// 附加数据，按 `status` 取对应变体（见 `ScopeDirDetail`）；无附加数据时为 `None`。
    pub detail: Option<ScopeDirDetail>,
}

fn deep_merge_json(target: &mut Value, source: &Value) {
    if let (Some(target_map), Some(source_map)) = (target.as_object_mut(), source.as_object()) {
        for (key, source_val) in source_map {
            match target_map.get_mut(key) {
                Some(target_val) if target_val.is_object() && source_val.is_object() => {
                    deep_merge_json(target_val, source_val);
                }
                _ => {
                    target_map.insert(key.clone(), source_val.clone());
                }
            }
        }
    }
}

/// 读取项目目录的有效 settings：settings.json 与 settings.local.json 只读合并（local 优先）。
fn read_dir_effective_settings(dir: &str) -> Option<Value> {
    let base = PathBuf::from(dir).join(".claude");
    let mut merged: Option<Value> = None;
    for file_name in ["settings.json", "settings.local.json"] {
        let path = base.join(file_name);
        if !path.exists() {
            continue;
        }
        if let Ok(value) = read_json_file(&path, "{}") {
            merged = Some(match merged {
                None => value,
                Some(mut prev) => {
                    deep_merge_json(&mut prev, &value);
                    prev
                }
            });
        }
    }
    merged
}

/// 用于身份判别的模型字段：同一网关下多个 profile 常共用 base_url+token，仅靠模型字段区分
const IDENTITY_MODEL_KEYS: &[&str] = &[
    "ANTHROPIC_MODEL",
    "ANTHROPIC_DEFAULT_HAIKU_MODEL",
    "ANTHROPIC_DEFAULT_SONNET_MODEL",
    "ANTHROPIC_DEFAULT_OPUS_MODEL",
];

/// 连接身份：base_url + token + 模型字段（与 IDENTITY_MODEL_KEYS 对齐）
#[derive(Debug, Clone, PartialEq)]
struct ConnIdentity {
    base_url: Option<String>,
    token: Option<String>,
    models: Vec<Option<String>>,
}

/// 从 settings 中提取连接身份
fn extract_conn_identity(settings: &Value) -> ConnIdentity {
    let env = settings.pointer("/env");
    let env_str = |key: &str| {
        env.and_then(|e| e.get(key))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
    };
    ConnIdentity {
        base_url: env_str("ANTHROPIC_BASE_URL").or_else(|| env_str("OPENAI_BASE_URL")),
        token: env_str("ANTHROPIC_AUTH_TOKEN")
            .or_else(|| env_str("ANTHROPIC_API_KEY"))
            .or_else(|| env_str("OPENAI_API_KEY")),
        models: IDENTITY_MODEL_KEYS.iter().map(|key| env_str(key)).collect(),
    }
}

/// 展示用模型：ANTHROPIC_MODEL 或顶层 /model
fn display_model_of(settings: &Value) -> Option<String> {
    settings
        .pointer("/env/ANTHROPIC_MODEL")
        .or_else(|| settings.pointer("/model"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
}

/// 读取配置库中所有 profile 的连接身份，用于与目录检测结果的匹配。
fn list_profile_conn_identities(
    conn: &rusqlite::Connection,
    kind: CliKind,
) -> Vec<(String, ConnIdentity)> {
    let names = app_db::list_profiles_with(conn, kind).unwrap_or_default();
    let mut result = Vec::new();
    for name in names {
        if let Ok(raw) = app_db::read_profile_with(conn, kind, &name) {
            if let Ok(stored) = decode_stored_profile_for_name(kind, Some(&name), &raw) {
                result.push((name, extract_conn_identity(&stored.visible)));
            }
        }
    }
    result
}

/// 匹配规则：
/// 1. 候选须 base_url 与 token 双等；
/// 2. 模型字段双方都有值时必须相等（冲突即淘汰）——同一网关共用 token 时靠模型区分；
/// 3. 多个候选取模型一致数最多者，仍并列则取列表首个（确定性）。
fn match_profile_by_identity(
    profiles: &[(String, ConnIdentity)],
    disk: &ConnIdentity,
) -> Option<String> {
    let mut best: Option<(&str, usize)> = None;
    for (name, identity) in profiles {
        if identity.base_url != disk.base_url || identity.token != disk.token {
            continue;
        }
        let mut agree = 0usize;
        let mut conflict = false;
        for (profile_val, disk_val) in identity.models.iter().zip(disk.models.iter()) {
            if let (Some(pv), Some(dv)) = (profile_val, disk_val) {
                if pv == dv {
                    agree += 1;
                } else {
                    conflict = true;
                    break;
                }
            }
        }
        if conflict {
            continue;
        }
        if best.is_none() || agree > best.unwrap().1 {
            best = Some((name.as_str(), agree));
        }
    }
    best.map(|(name, _)| name.to_string())
}

fn token_short_hash(token: &Option<String>) -> Option<String> {
    use std::hash::{Hash, Hasher};
    token.as_ref().map(|t| {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        t.hash(&mut hasher);
        format!("{:016x}", hasher.finish())
    })
}

/// 精确匹配：给定 settings 内容，在配置库中找身份完全一致的 profile。
/// 供全局自动绑定等场景「精确优先、宽松兜底」的精确阶段使用。
#[tauri::command]
pub fn match_profile_by_settings(
    cli_id: Option<String>,
    settings_content: String,
) -> AppResult<Option<String>> {
    let kind = CliKind::from_id(cli_id.as_deref())?;
    if !matches!(kind, CliKind::Claude) {
        return Ok(None);
    }
    let settings: Value =
        serde_json::from_str(&settings_content).map_err(|e| AppError::coded("profile.content_parse_failed").with("detail", e.to_string()))?;
    let identity = extract_conn_identity(&settings);
    if identity.base_url.is_none() && identity.token.is_none() {
        return Ok(None);
    }
    let conn = app_db::conn()?;
    let profiles = list_profile_conn_identities(&conn, kind);
    Ok(match_profile_by_identity(&profiles, &identity))
}

#[tauri::command]
pub fn detect_scope_dirs_config(
    cli_id: Option<String>,
    dirs: Vec<String>,
) -> AppResult<Vec<DirConfigInfo>> {    let kind = CliKind::from_id(cli_id.as_deref())?;
    if !matches!(kind, CliKind::Claude) {
        return Ok(vec![]);
    }
    let conn = app_db::conn()?;
    let profiles = list_profile_conn_identities(&conn, kind);

    let mut results = Vec::new();
    for dir in dirs {
        let info = match read_dir_effective_settings(&dir) {
            Some(settings) => {
                let identity = extract_conn_identity(&settings);
                let model = display_model_of(&settings);
                let has_config = identity.base_url.is_some() || identity.token.is_some();
                let matched_profile = if has_config {
                    match_profile_by_identity(&profiles, &identity)
                } else {
                    None
                };
                DirConfigInfo {
                    dir,
                    has_config,
                    base_url: identity.base_url.clone(),
                    model,
                    matched_profile,
                    token_hash: token_short_hash(&identity.token),
                }
            }
            None => DirConfigInfo {
                dir,
                has_config: false,
                base_url: None,
                model: None,
                matched_profile: None,
                token_hash: None,
            },
        };
        results.push(info);
    }
    Ok(results)
}

#[tauri::command]
pub fn import_scope_dir_config(
    cli_id: Option<String>,
    dir: String,
    scope: String,
) -> AppResult<String> {
    let kind = CliKind::from_id(cli_id.as_deref())?;
    if !matches!(kind, CliKind::Claude) {
        return Err(AppError::coded("profile.import_claude_only"));
    }
    let settings = read_dir_effective_settings(&dir)
        .ok_or_else(|| AppError::coded("profile.import_no_config_file"))?;
    let identity = extract_conn_identity(&settings);
    if identity.base_url.is_none() && identity.token.is_none() {
        return Err(AppError::coded("profile.import_no_api_info"));
    }

    // 仅导入受管字段（env 连接/模型相关 + /model），避免带入项目其他私有设置
    let mut visible = Map::new();
    let mut env_out = Map::new();
    for path in CLAUDE_MANAGED_PROFILE_PATHS {
        if let Some(key) = path.strip_prefix("/env/") {
            if let Some(value) = settings.pointer(path) {
                env_out.insert(key.to_string(), value.clone());
            }
        }
    }
    if let Some(value) = settings.pointer("/env/ANTHROPIC_API_KEY") {
        env_out.insert("ANTHROPIC_API_KEY".to_string(), value.clone());
    }
    if !env_out.is_empty() {
        visible.insert("env".to_string(), Value::Object(env_out));
    }
    if let Some(value) = settings.pointer("/model") {
        visible.insert("model".to_string(), value.clone());
    }
    let content = serde_json::to_string_pretty(&Value::Object(visible))
        .map_err(|e| AppError::coded("profile.serialize_failed").with("detail", e.to_string()))?;

    // 以目录名生成配置名，冲突时追加序号
    let conn = app_db::conn()?;
    let existing = app_db::list_profiles_with(&conn, kind).unwrap_or_default();
    let base_name = PathBuf::from(&dir)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "imported".to_string());
    let mut name = base_name.clone();
    let mut seq = 2;
    while existing.contains(&name) {
        name = format!("{}-{}", base_name, seq);
        seq += 1;
    }

    save_profile_for_internal(kind, name.clone(), content, false, None)?;
    set_scope_binding_for(kind, &scope, &name)?;
    Ok(name)
}

#[tauri::command]
pub fn get_scope_dir_status(cli_id: Option<String>) -> AppResult<Vec<ScopeDirStatus>> {
    let kind = CliKind::from_id(cli_id.as_deref())?;
    if !matches!(kind, CliKind::Claude) {
        return Ok(vec![]);
    }
    let conn = app_db::conn()?;
    get_scope_dir_status_with(&conn, kind)
}

/// `get_scope_dir_status` 的实现体。`_with` 后缀是本文件统一的测试接缝：
/// 命令层取全局连接池，测试用内存库直接调这里。
pub(crate) fn get_scope_dir_status_with(
    conn: &rusqlite::Connection,
    kind: CliKind,
) -> AppResult<Vec<ScopeDirStatus>> {
    let tabs = app_db::list_profile_tabs_with(conn, kind.id())?;

    let mut results = Vec::new();
    for tab in tabs {
        if tab.dirs.is_empty() {
            continue;
        }
        let identities: Vec<Option<ConnIdentity>> = tab
            .dirs
            .iter()
            .map(|dir| read_dir_effective_settings(dir).map(|s| extract_conn_identity(&s)))
            .collect();

        let bound_raw = app_db::get_active_profile_with(conn, kind, Some(&tab.id))?;
        let bound = {
            let trimmed = bound_raw.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        };

        let status = match &bound {
            Some(profile_name) => {
                let expected = app_db::read_profile_with(&conn, kind, profile_name)
                    .ok()
                    .and_then(|raw| {
                        decode_stored_profile_for_name(kind, Some(profile_name), &raw).ok()
                    })
                    .map(|stored| extract_conn_identity(&stored.visible));
                match expected {
                    Some(expected) => {
                        let mismatch = tab.dirs.iter().zip(identities.iter()).find(|(_, id)| {
                            id.as_ref() != Some(&expected)
                        });
                        match mismatch {
                            Some((dir, _)) => ScopeDirStatus {
                                scope: tab.id.clone(),
                                status: "diverged".to_string(),
                                detail: Some(ScopeDirDetail::Diverged { dir: dir.clone() }),
                            },
                            None => ScopeDirStatus {
                                scope: tab.id.clone(),
                                status: "aligned".to_string(),
                                detail: None,
                            },
                        }
                    }
                    None => ScopeDirStatus {
                        scope: tab.id.clone(),
                        status: "none".to_string(),
                        detail: None,
                    },
                }
            }
            None => {
                // 带连接配置的目录（连同目录名，供 `DirsInconsistent` 回列表）
                let with_config: Vec<(&String, &ConnIdentity)> = tab
                    .dirs
                    .iter()
                    .zip(identities.iter())
                    .filter_map(|(dir, id)| id.as_ref().map(|id| (dir, id)))
                    .filter(|(_, id)| id.base_url.is_some() || id.token.is_some())
                    .collect();
                if with_config.is_empty() {
                    ScopeDirStatus {
                        scope: tab.id.clone(),
                        status: "none".to_string(),
                        detail: None,
                    }
                } else {
                    let first = with_config[0].1;
                    let inconsistent = with_config.iter().any(|(_, id)| *id != first);
                    if inconsistent && with_config.len() > 1 {
                        ScopeDirStatus {
                            scope: tab.id.clone(),
                            status: "dirsInconsistent".to_string(),
                            detail: Some(ScopeDirDetail::DirsInconsistent {
                                dirs: with_config.iter().map(|(dir, _)| (*dir).clone()).collect(),
                            }),
                        }
                    } else {
                        ScopeDirStatus {
                            scope: tab.id.clone(),
                            status: "unboundWithConfig".to_string(),
                            detail: first
                                .base_url
                                .clone()
                                .map(|base_url| ScopeDirDetail::UnboundWithConfig { base_url }),
                        }
                    }
                }
            }
        };
        results.push(status);
    }
    Ok(results)
}

#[tauri::command]
pub fn sync_active_profile_from_cli(cli_id: Option<String>, scope: Option<String>) -> AppResult<bool> {
    let kind = CliKind::from_id(cli_id.as_deref())?;
    let scope_str = scope.as_deref().unwrap_or("global");

    if scope_str == "global" || !matches!(kind, CliKind::Claude) {
        return sync_active_profile_from_cli_for(kind);
    }

    let active_name = get_active_profile_for(kind, Some(scope_str))?;
    if active_name.trim().is_empty() {
        return Ok(false);
    }

    let current_settings = read_scope_settings_for(kind, Some(scope_str))?;

    save_profile_for_internal(
        kind,
        active_name,
        current_settings,
        false,
        Some(scope_str),
    )?;

    Ok(true)
}

#[tauri::command]
pub fn build_profile_settings_file(
    cli_id: Option<String>,
    profile_name: String,
    scope: Option<String>,
) -> AppResult<String> {
    let kind = CliKind::from_id(cli_id.as_deref())?;
    build_profile_settings_file_for(kind, profile_name, scope.as_deref())
}

#[tauri::command]
pub fn create_profile_tab(cli_id: String, name: String, dirs: Vec<String>) -> AppResult<app_db::TabInfo> {
    app_db::create_profile_tab(&cli_id, &name, &dirs)
}

#[tauri::command]
pub fn update_profile_tab(
    tab_id: String,
    name: Option<String>,
    dirs: Option<Vec<String>>,
) -> AppResult<app_db::TabInfo> {
    app_db::update_profile_tab(&tab_id, name, dirs)
}

#[tauri::command]
pub fn delete_profile_tab(tab_id: String, clean_disk: Option<bool>) -> AppResult<()> {
    if clean_disk.unwrap_or(false) {
        if let Ok(conn) = app_db::conn() {
            if let Ok(dirs) = app_db::get_profile_tab_dirs_with(&conn, &tab_id) {
                for dir in &dirs {
                    let settings_path = PathBuf::from(dir).join(".claude").join("settings.json");
                    if settings_path.exists() {
                        let _ = clean_claude_settings_for_unbind(&settings_path);
                    }
                }
            }
        }
    }
    app_db::delete_profile_tab(&tab_id)
}

#[tauri::command]
pub fn list_profile_tabs(cli_id: String) -> AppResult<Vec<app_db::TabInfo>> {
    app_db::list_profile_tabs(&cli_id)
}

#[tauri::command]
pub fn refresh_tray_menu(
    app_handle: tauri::AppHandle,
    cli_id: Option<String>,
) -> AppResult<()> {
    if let Some(cli_id) = cli_id.as_deref() {
        CliKind::from_id(Some(cli_id))?;
    }

    // 收口在 `tray::attribute_rebuild_error` 里：先拆箱再归属，保住内层 `Coded` 的 code 与
    // params（`Box<dyn Error>` 的 `Display` 只渲染裸 code）。三个调用点共用它。
    tray::rebuild_tray_menu(&app_handle).map_err(tray::attribute_rebuild_error)
}

#[tauri::command]
pub fn read_favorite_entries() -> AppResult<Vec<FavoriteEntry>> {
    app_db::read_favorite_entries()
}

#[tauri::command]
pub fn write_favorite_entries(entries: Vec<FavoriteEntry>) -> AppResult<()> {
    app_db::write_favorite_entries(&entries)
}

#[tauri::command]
pub fn read_blocked_folders() -> AppResult<Vec<String>> {
    app_db::read_blocked_folders()
}

#[tauri::command]
pub fn write_blocked_folders(paths: Vec<String>) -> AppResult<()> {
    app_db::write_blocked_folders(&paths)
}

fn default_bookmark_created_at() -> String {
    Utc::now().to_rfc3339()
}

fn read_all_bookmarks() -> AppResult<Vec<BookmarkRecord>> {
    app_db::read_all_bookmarks()
}

fn write_all_bookmarks(bookmarks: &[BookmarkRecord]) -> AppResult<()> {
    app_db::write_all_bookmarks(bookmarks)
}

fn bookmarks_for_kind(kind: CliKind) -> AppResult<Vec<BookmarkRecord>> {
    Ok(read_all_bookmarks()?
        .into_iter()
        .filter(|bookmark| bookmark.cli_id == kind.id())
        .collect())
}

#[tauri::command]
pub fn read_bookmarks(cli_id: Option<String>) -> AppResult<String> {
    let kind = CliKind::from_id(cli_id.as_deref())?;
    let bookmarks = bookmarks_for_kind(kind)?;
    serde_json::to_string_pretty(&bookmarks).map_err(|e| AppError::coded("profile.bookmark_serialize_failed").with("detail", e.to_string()))
}

#[tauri::command]
pub fn toggle_bookmark(
    cli_id: Option<String>,
    session_id: String,
    message_index: usize,
    message_role: Option<String>,
    message_text: Option<String>,
    message_timestamp: Option<String>,
    session_display_name: Option<String>,
) -> AppResult<String> {
    let kind = CliKind::from_id(cli_id.as_deref())?;
    let cli_id = kind.id().to_string();
    let mut bookmarks = read_all_bookmarks()?;

    let existing_idx = bookmarks.iter().position(|bookmark| {
        bookmark.cli_id == cli_id
            && bookmark.session_id == session_id
            && bookmark.message_index == message_index
    });

    if let Some(idx) = existing_idx {
        bookmarks.remove(idx);
    } else {
        bookmarks.push(BookmarkRecord {
            cli_id: cli_id.clone(),
            session_id,
            message_index,
            note: None,
            created_at: default_bookmark_created_at(),
            message_role,
            message_text,
            message_timestamp,
            session_display_name,
        });
    }

    write_all_bookmarks(&bookmarks)?;

    let visible: Vec<BookmarkRecord> = bookmarks
        .into_iter()
        .filter(|bookmark| bookmark.cli_id == cli_id)
        .collect();
    serde_json::to_string_pretty(&visible).map_err(|e| AppError::coded("profile.bookmark_serialize_failed").with("detail", e.to_string()))
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BookmarkWithContext {
    pub cli_id: String,
    pub session_id: String,
    pub session_path: String,
    pub session_display_name: String,
    pub message_index: usize,
    pub message_preview: String,
    pub note: Option<String>,
    pub created_at: String,
    pub source_deleted: bool,
    pub message_role: Option<String>,
    pub message_text: Option<String>,
    pub message_timestamp: Option<String>,
}

#[tauri::command]
pub fn list_bookmarks_with_context(cli_id: Option<String>) -> AppResult<Vec<BookmarkWithContext>> {
    let kind = CliKind::from_id(cli_id.as_deref())?;
    let bookmarks = bookmarks_for_kind(kind)?;
    if bookmarks.is_empty() {
        return Ok(vec![]);
    }

    let session_index = app_db::read_session_list_index(kind)?;
    let session_names = app_db::load_session_names()?;
    // WorkBuddy 用户重命名在 workbuddy.db（sessions.custom_title），覆盖原生 AI 标题
    let wb_titles = if kind == CliKind::WorkBuddy {
        crate::parser::load_workbuddy_custom_titles()
    } else {
        std::collections::HashMap::new()
    };

    let id_to_path: std::collections::HashMap<&str, &str> = session_index
        .iter()
        .map(|(path, record)| (record.session_id.as_str(), path.as_str()))
        .collect();

    let mut results = Vec::new();
    for bm in &bookmarks {
        let session_path_opt = id_to_path.get(bm.session_id.as_str()).map(|p| p.to_string());
        let source_deleted = session_path_opt.is_none();

        let (session_path, display_name, preview) = if let Some(ref sp) = session_path_opt {
            let custom_name = custom_session_name(&session_names, kind, sp);
            let dn = if let Some(record) = session_index.get(sp.as_str()) {
                crate::title_resolver::resolve_display_name(
                    custom_name,
                    wb_titles
                        .get(bm.session_id.as_str())
                        .map(String::as_str)
                        .or(record.title.as_deref()),
                    record.first_user_message.as_deref(),
                    None,
                    &bm.session_id,
                )
            } else {
                custom_name
                    .map(str::to_string)
                    .or_else(|| bm.session_display_name.clone())
                    .unwrap_or_else(|| bm.session_id.clone())
            };

            let pv = match app_db::read_session_search_doc_text(kind, sp, bm.message_index) {
                Ok(Some(text)) => {
                    let first_line = text.lines().next().unwrap_or("");
                    if first_line.chars().count() > 60 {
                        format!("{}…", first_line.chars().take(60).collect::<String>())
                    } else {
                        first_line.to_string()
                    }
                }
                _ => snapshot_preview(&bm.message_text),
            };

            (sp.clone(), dn, pv)
        } else {
            // 源文件已不在磁盘，拿不到会话路径，也就无法构造 session_names 的复合键。
            // 书签自带创建时的显示名，缺失时退回 session_id。
            let dn = bm
                .session_display_name
                .clone()
                .unwrap_or_else(|| bm.session_id.clone());
            let pv = snapshot_preview(&bm.message_text);
            (String::new(), dn, pv)
        };

        results.push(BookmarkWithContext {
            cli_id: bm.cli_id.clone(),
            session_id: bm.session_id.clone(),
            session_path,
            session_display_name: display_name,
            message_index: bm.message_index,
            message_preview: preview,
            note: bm.note.clone(),
            created_at: bm.created_at.clone(),
            source_deleted,
            message_role: bm.message_role.clone(),
            message_text: bm.message_text.clone(),
            message_timestamp: bm.message_timestamp.clone(),
        });
    }

    Ok(results)
}

fn snapshot_preview(text: &Option<String>) -> String {
    match text {
        Some(t) => {
            let first_line = t.lines().next().unwrap_or("");
            if first_line.chars().count() > 60 {
                format!("{}…", first_line.chars().take(60).collect::<String>())
            } else {
                first_line.to_string()
            }
        }
        None => String::new(),
    }
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct BackupProfileItem {
    pub cli_id: String,
    pub scope: String,
    pub name: String,
    pub content: String,
    pub is_active: bool,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct ProfilesBackupData {
    pub version: u32,
    pub timestamp: String,
    pub profiles: Vec<BackupProfileItem>,
    pub tabs: Option<Vec<app_db::TabInfo>>,
    /// 作用域绑定（作用域 tab id → 激活配置名）；旧备份无此字段，默认空
    #[serde(default)]
    pub scope_bindings: Vec<ScopeBindingExport>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ScopeBindingExport {
    pub cli_id: String,
    pub scope: String,
    pub profile_name: String,
}

#[derive(Debug, serde::Deserialize)]
pub struct ProfileImportItem {
    pub cli_id: String,
    pub scope: String,
    pub name: String,
    pub content: String,
    pub is_active: bool,
    pub action: String,
    pub new_name: Option<String>,
}

#[tauri::command]
pub fn export_profiles_backup(save_path: Option<String>) -> AppResult<ProfilesBackupData> {
    let records = app_db::export_all_profiles()?;
    let profiles = records.into_iter().map(|r| BackupProfileItem {
        cli_id: r.cli_id,
        scope: "global".to_string(),
        name: r.name,
        content: r.content,
        is_active: r.is_active,
    }).collect();

    let tabs = app_db::export_all_profile_tabs().ok();

    let scope_bindings = app_db::export_scope_bindings()
        .unwrap_or_default()
        .into_iter()
        .map(|(cli_id, scope, profile_name)| ScopeBindingExport {
            cli_id,
            scope,
            profile_name,
        })
        .collect();

    let backup_data = ProfilesBackupData {
        version: 1,
        timestamp: chrono::Utc::now().to_rfc3339(),
        profiles,
        tabs,
        scope_bindings,
    };

    if let Some(path) = save_path {
        let json_str = serde_json::to_string_pretty(&backup_data)
            .map_err(|e| AppError::coded("profile.backup_serialize_failed").with("detail", e.to_string()))?;
        std::fs::write(&path, json_str)
            .map_err(|e| AppError::coded("profile.backup_write_failed").with("detail", e.to_string()))?;
    }

    Ok(backup_data)
}

fn parse_profiles_backup_content(content: &str) -> AppResult<ProfilesBackupData> {
    serde_json::from_str(content)
        .map_err(|e| AppError::coded("profile.backup_parse_failed").with("detail", e.to_string()))
}

#[tauri::command]
pub fn read_profiles_backup(file_path: String) -> AppResult<ProfilesBackupData> {
    let content = std::fs::read_to_string(&file_path)
        .map_err(|e| AppError::coded("profile.backup_read_failed").with("detail", e.to_string()))?;
    parse_profiles_backup_content(&content)
}

/// 从剪贴板文本解析备份（与 read_profiles_backup 共享解析逻辑）
#[tauri::command]
pub fn parse_profiles_backup(content: String) -> AppResult<ProfilesBackupData> {
    parse_profiles_backup_content(&content)
}

#[tauri::command]
pub fn import_profiles_backup(
    items: Vec<ProfileImportItem>,
    tabs: Option<Vec<app_db::TabInfo>>,
    scope_bindings: Option<Vec<ScopeBindingExport>>,
) -> AppResult<()> {
    if let Some(tab_list) = tabs {
        for t in tab_list {
            let _ = app_db::import_profile_tab(&t);
        }
    }

    // 记录实际导入的 (cli_id, 归一化原始名) → 目标名，供作用域绑定做重命名映射
    let mut imported_names: std::collections::HashMap<(String, String), String> =
        std::collections::HashMap::new();

    for item in &items {
        if item.action == "skip" {
            continue;
        }

        let kind = match item.cli_id.as_str() {
            "claude" => CliKind::Claude,
            "codex" => CliKind::Codex,
            "gemini" => CliKind::Gemini,
            _ => continue,
        };

        let target_name = if item.action == "rename" {
            item.new_name.clone().unwrap_or_else(|| item.name.clone())
        } else {
            item.name.clone()
        };

        let _ = app_db::ensure_profile_tab_exists(&item.cli_id, &item.scope);
        app_db::save_profile(kind, &target_name, &item.content)?;
        imported_names.insert(
            (item.cli_id.clone(), item.name.trim().to_lowercase()),
            target_name.clone(),
        );

        if item.is_active {
            app_db::set_active_profile(kind, &target_name, Some(&item.scope))?;
        }
    }

    if let Some(bindings) = scope_bindings {
        let tuples: Vec<(String, String, String)> = bindings
            .into_iter()
            .map(|b| (b.cli_id, b.scope, b.profile_name))
            .collect();
        app_db::restore_scope_bindings(&tuples, &imported_names)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 取 `Coded` 错误的 code。
    ///
    /// 断言 code 而不是断言 `err.to_string()` 里含某段中文：`Coded` 的 `Display` 就是 code
    /// （见 `error.rs` 的 `#[error("{code}")]`），文案已移到语言包、随语言变，
    /// 而跨 IPC 的协议字段就是 code——断它才是断在契约上。
    fn coded_code(err: &AppError) -> &str {
        match err {
            AppError::Coded { code, .. } => code,
            other => panic!("期望 Coded 错误，实际拿到 {other:?}"),
        }
    }

    #[test]
    fn parse_profiles_backup_content_valid_and_invalid() {
        let ok = parse_profiles_backup_content(
            r#"{"version":1,"timestamp":"t","profiles":[],"tabs":null}"#,
        );
        assert!(ok.is_ok());
        assert!(ok.unwrap().profiles.is_empty());

        let bad = parse_profiles_backup_content("not json");
        assert!(bad.is_err());

        let wrong_shape = parse_profiles_backup_content(r#"{"foo":1}"#);
        assert!(wrong_shape.is_err());
    }

    fn setup_test_db() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE profiles (
                cli_id TEXT NOT NULL,
                scope TEXT NOT NULL DEFAULT 'global',
                name TEXT NOT NULL,
                normalized_name TEXT NOT NULL,
                content_json TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                PRIMARY KEY (cli_id, scope, normalized_name)
            );

            CREATE TABLE active_profiles (
                cli_id TEXT NOT NULL,
                profile_name TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                scope TEXT NOT NULL DEFAULT 'global',
                PRIMARY KEY (cli_id, scope)
            );

            CREATE TABLE profile_tabs (
                id TEXT PRIMARY KEY,
                cli_id TEXT NOT NULL,
                name TEXT NOT NULL,
                sort_order INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE TABLE profile_tab_dirs (
                tab_id TEXT NOT NULL,
                dir_path TEXT NOT NULL,
                PRIMARY KEY (tab_id, dir_path),
                FOREIGN KEY (tab_id) REFERENCES profile_tabs(id) ON DELETE CASCADE
            );
            "#,
        )
        .unwrap();
        conn
    }

    #[test]
    fn codex_regular_profile_sanitization_keeps_only_api_key_auth_field() {
        let visible = json!({
            "auth": {
                "OPENAI_API_KEY": "sk-test",
                "access_token": "oauth-access",
                "refresh_token": "oauth-refresh"
            },
            "codex": {
                "base_url": "https://api.example.com/v1",
                "model": "gpt-5"
            }
        });

        let sanitized = sanitize_profile_value_for_name(CliKind::Codex, None, &visible);

        assert_eq!(
            sanitized.pointer("/auth/OPENAI_API_KEY"),
            Some(&json!("sk-test"))
        );
        assert!(sanitized.pointer("/auth/access_token").is_none());
        assert!(sanitized.pointer("/auth/refresh_token").is_none());
        assert_eq!(
            sanitized.pointer("/codex/base_url"),
            Some(&json!("https://api.example.com/v1"))
        );
    }

    #[test]
    fn codex_official_profile_sanitization_keeps_oauth_auth_fields() {
        let visible = json!({
            "auth": {
                "auth_mode": "chatgpt",
                "access_token": "oauth-access",
                "refresh_token": "oauth-refresh",
                "OPENAI_API_KEY": "sk-ignored-for-official"
            },
            "codex": {}
        });

        let sanitized = sanitize_profile_value_for_name(
            CliKind::Codex,
            Some(CODEX_OFFICIAL_PROFILE_NAME),
            &visible,
        );

        assert_eq!(sanitized.pointer("/auth/access_token"), Some(&json!("oauth-access")));
        assert_eq!(
            sanitized.pointer("/auth/refresh_token"),
            Some(&json!("oauth-refresh"))
        );
        assert!(sanitized.get("codex").is_none());
    }

    #[test]
    fn codex_official_profile_content_from_live_drops_provider_fields() {
        let live = json!({
            "auth": {
                "auth_mode": "chatgpt",
                "access_token": "oauth-access"
            },
            "codex": {
                "base_url": "https://api.example.com/v1",
                "model": "gpt-5",
                "model_reasoning_effort": "medium"
            }
        });

        let official = codex_official_profile_content_from_live(&live);

        assert_eq!(
            official.pointer("/auth/access_token"),
            Some(&json!("oauth-access"))
        );
        assert!(official.get("codex").is_none());
    }

    #[test]
    fn codex_managed_provider_fields_detect_custom_live_config() {
        assert!(codex_settings_has_managed_provider_fields(&json!({
            "auth": {
                "access_token": "oauth-access"
            },
            "codex": {
                "base_url": "https://api.example.com/v1"
            }
        })));
        assert!(!codex_settings_has_managed_provider_fields(&json!({
            "auth": {
                "access_token": "oauth-access"
            },
            "codex": {
                "model_provider": "openai-chat-completions"
            }
        })));
    }

    #[test]
    fn codex_regular_profile_matches_live_api_key_and_provider_fields() {
        let profile = json!({
            "codex": {
                "base_url": "https://api.example.com/v1",
                "model": "gpt-5",
                "experimental_bearer_token": "sk-test"
            }
        });
        let live = json!({
            "codex": {
                "base_url": "https://api.example.com/v1",
                "model": "gpt-5",
                "model_provider": "custom",
                "experimental_bearer_token": "sk-test"
            }
        });

        assert!(codex_profile_matches_live_settings(&profile, &live));

        // 兼容旧 profile（auth.OPENAI_API_KEY）与新 live（codex.experimental_bearer_token）匹配
        let legacy_profile = json!({
            "auth": {
                "OPENAI_API_KEY": "sk-test"
            },
            "codex": {
                "base_url": "https://api.example.com/v1",
                "model": "gpt-5"
            }
        });
        assert!(codex_profile_matches_live_settings(&legacy_profile, &live));
    }

    #[test]
    fn codex_oauth_detection_ignores_api_key_and_empty_login_fields() {
        assert!(!codex_auth_has_login_material(&json!({
            "OPENAI_API_KEY": "sk-test"
        })));
        assert!(!codex_auth_has_login_material(&json!({
            "auth_mode": "chatgpt",
            "access_token": ""
        })));
        assert!(codex_auth_has_login_material(&json!({
            "auth_mode": "chatgpt",
            "access_token": "oauth-access"
        })));
    }

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

    #[test]
    fn test_save_profile_cascade_applies_to_all_active_scopes() {
        let mut conn = setup_test_db();
        let kind = CliKind::Claude;
        let temp_dir = tempfile::tempdir().unwrap();
        let global_settings_path = temp_dir.path().join("global_settings.json");
        let project_dir_a = temp_dir.path().join("project_a");
        let project_dir_b = temp_dir.path().join("project_b");
        fs::create_dir_all(&project_dir_a).unwrap();
        fs::create_dir_all(&project_dir_b).unwrap();

        // 1. Create profile "WorkProfile"
        let initial_content = json!({
            "env": {
                "ANTHROPIC_BASE_URL": "https://init.anthropic.com"
            }
        }).to_string();
        app_db::save_profile_with(&mut conn, kind, "WorkProfile", &initial_content).unwrap();

        // 2. Create tabs
        let tab1 = app_db::create_profile_tab_with(
            &mut conn,
            "claude",
            "Tab 1",
            &[project_dir_a.to_str().unwrap().to_string(), project_dir_b.to_str().unwrap().to_string()],
        ).unwrap();

        let tab2 = app_db::create_profile_tab_with(
            &mut conn,
            "claude",
            "Tab 2 (Empty)",
            &[],
        ).unwrap();

        // 3. Bind "WorkProfile" to global, tab1, and tab2
        app_db::set_active_profile_with(&conn, kind, "WorkProfile", Some("global")).unwrap();
        app_db::set_active_profile_with(&conn, kind, "WorkProfile", Some(&tab1.id)).unwrap();
        app_db::set_active_profile_with(&conn, kind, "WorkProfile", Some(&tab2.id)).unwrap();

        // 4. Update profile with new content and trigger cascade apply
        let updated_content = json!({
            "env": {
                "ANTHROPIC_BASE_URL": "https://updated.anthropic.com"
            }
        }).to_string();
        app_db::save_profile_with(&mut conn, kind, "WorkProfile", &updated_content).unwrap();

        let cascade_result = cascade_apply_profile_with(
            &conn,
            kind,
            "WorkProfile",
            Some(&global_settings_path),
        ).unwrap();

        // Global (1) + Tab 1 dirs (2) + Tab 2 dirs (0) = 3 success writes
        assert_eq!(cascade_result.success_count, 3);
        assert!(cascade_result.failed_dirs.is_empty());
        assert!(cascade_result.affected_scopes.contains(&"global".to_string()));
        assert!(cascade_result.affected_scopes.contains(&tab1.id));
        assert!(cascade_result.affected_scopes.contains(&tab2.id));

        // 5. Verify disk files
        let global_json: Value = serde_json::from_str(&fs::read_to_string(&global_settings_path).unwrap()).unwrap();
        assert_eq!(global_json.pointer("/env/ANTHROPIC_BASE_URL"), Some(&json!("https://updated.anthropic.com")));

        let tab1_a_json: Value = serde_json::from_str(
            &fs::read_to_string(project_dir_a.join(".claude").join("settings.json")).unwrap()
        ).unwrap();
        assert_eq!(tab1_a_json.pointer("/env/ANTHROPIC_BASE_URL"), Some(&json!("https://updated.anthropic.com")));

        let tab1_b_json: Value = serde_json::from_str(
            &fs::read_to_string(project_dir_b.join(".claude").join("settings.json")).unwrap()
        ).unwrap();
        assert_eq!(tab1_b_json.pointer("/env/ANTHROPIC_BASE_URL"), Some(&json!("https://updated.anthropic.com")));
    }

    /// ★ 诊断站点的**行为钉**（`:993` 与 `:1004`，`cascade_apply_profile_with` 的两个失败分支）。
    ///
    /// 钉的是「日志里必须带 `params.detail`，不能只有裸 code」——`Coded` 的 `Display` 就是 code，
    /// 故把站点的 `e.diagnostic()` 改回 `e` **编译得过、闸门绿、全套测试绿**（实测），
    /// 只有这一条能抓住它。
    ///
    /// 驱动方式：真失败，不 mock —— 把父目录做成一个**普通文件**，
    /// `ensure_parent_dir` 的 `create_dir_all` 必然 `NotADirectory`；
    /// global 分支命中 `File exists`（`blocker` 本身挡住了同名目录的创建）。
    /// 两条路径各产生一条日志，故断言 `detail=` 出现 **2** 次。
    #[test]
    fn cascade_apply_failure_logs_detail_not_bare_code() {
        let mut conn = setup_test_db();
        let kind = CliKind::Claude;
        let temp_dir = tempfile::tempdir().unwrap();
        let blocker = temp_dir.path().join("blocker");
        fs::write(&blocker, b"x").unwrap();

        let content = json!({ "env": { "ANTHROPIC_BASE_URL": "https://log-probe.example" } }).to_string();
        app_db::save_profile_with(&mut conn, kind, "LogProbe", &content).unwrap();
        let tab = app_db::create_profile_tab_with(
            &mut conn,
            "claude",
            "Log Probe Tab",
            &[blocker.join("dir").to_string_lossy().to_string()],
        )
        .unwrap();
        app_db::set_active_profile_with(&conn, kind, "LogProbe", Some("global")).unwrap();
        app_db::set_active_profile_with(&conn, kind, "LogProbe", Some(&tab.id)).unwrap();

        let logs = crate::error::log_capture::capture(|| {
            let result =
                cascade_apply_profile_with(&conn, kind, "LogProbe", Some(&blocker.join("global_settings.json")))
                    .unwrap();
            assert_eq!(result.success_count, 0, "两条路径都必须失败");
            assert_eq!(result.failed_dirs.len(), 2, "global 与 dir 各记一条失败");
        });

        let mut with_detail = 0;
        for line in logs.lines() {
            if let Some(detail) = line.split("cli_config.io_failed detail=").nth(1) {
                assert!(!detail.trim().is_empty(), "detail 不能为空：{line}");
                with_detail += 1;
            }
        }
        assert_eq!(
            with_detail, 2,
            "global 与 dir 两条失败日志都要带 detail，实际日志：\n{logs}"
        );
    }

    #[test]
    fn test_save_profile_inactive_does_not_apply() {
        let mut conn = setup_test_db();
        let kind = CliKind::Claude;
        let temp_dir = tempfile::tempdir().unwrap();
        let global_settings_path = temp_dir.path().join("global_settings.json");

        let content = json!({
            "env": {
                "ANTHROPIC_BASE_URL": "https://inactive.anthropic.com"
            }
        }).to_string();
        app_db::save_profile_with(&mut conn, kind, "InactiveProfile", &content).unwrap();

        let cascade_result = cascade_apply_profile_with(
            &conn,
            kind,
            "InactiveProfile",
            Some(&global_settings_path),
        ).unwrap();

        assert_eq!(cascade_result.success_count, 0);
        assert!(cascade_result.failed_dirs.is_empty());
        assert!(cascade_result.affected_scopes.is_empty());
        assert!(!global_settings_path.exists());
    }

    /// 跨 IPC 契约：`ScopeDirStatus` 与 `ScopeDirDetail` 的 JSON 形状就是前端可辨识联合的判据
    /// （`ApiProfileScopeAllocation.vue` 的 `ScopeDirStatus` 读 `detail` 键、再按 `kind` 分派）。
    /// 改字段名、改键名或改 `rename_all` 都不会让任何一边编译失败，
    /// 只会让前端静默拿到 `undefined` 并把 tooltip 渲染成空，故在这里把两层都钉死：
    /// 外层是 `detail` 这个**键名**，内层是各变体的 `kind` 与字段名。
    #[test]
    fn scope_dir_status_wire_shape() {
        let wire = |status: ScopeDirStatus| serde_json::to_string(&status).unwrap();
        assert_eq!(
            wire(ScopeDirStatus {
                scope: "tab1".to_string(),
                status: "dirsInconsistent".to_string(),
                detail: Some(ScopeDirDetail::DirsInconsistent {
                    dirs: vec!["/a".to_string(), "/b".to_string()]
                }),
            }),
            r#"{"scope":"tab1","status":"dirsInconsistent","detail":{"kind":"dirs_inconsistent","dirs":["/a","/b"]}}"#
        );
        // `detail` 缺席时是 `null`（`Option` 的默认形态），前端按 falsy 兜底
        assert_eq!(
            wire(ScopeDirStatus {
                scope: "tab2".to_string(),
                status: "aligned".to_string(),
                detail: None,
            }),
            r#"{"scope":"tab2","status":"aligned","detail":null}"#
        );

        let detail = |detail: ScopeDirDetail| serde_json::to_string(&detail).unwrap();
        assert_eq!(
            detail(ScopeDirDetail::Diverged {
                dir: "/d".to_string()
            }),
            r#"{"kind":"diverged","dir":"/d"}"#
        );
        assert_eq!(
            detail(ScopeDirDetail::UnboundWithConfig {
                base_url: "https://a".to_string()
            }),
            r#"{"kind":"unbound_with_config","base_url":"https://a"}"#
        );
    }

    #[test]
    fn test_get_scope_bindings() {
        let mut conn = setup_test_db();
        let kind = CliKind::Claude;

        // 1. Initial bindings when no tabs created
        let bindings = get_scope_bindings_with(&conn, kind).unwrap();
        assert_eq!(bindings.len(), 1);
        assert_eq!(bindings[0].scope, "global");
        // 全局作用域没有后端造的名字：标签由前端按语言渲染（`api-profile.scope.global`）。
        assert_eq!(bindings[0].name, None);
        assert!(bindings[0].is_global);
        assert_eq!(bindings[0].active_profile, None);

        // 2. Add profiles and tabs
        app_db::save_profile_with(&mut conn, kind, "GlobalProf", "{}").unwrap();
        app_db::save_profile_with(&mut conn, kind, "TabProf", "{}").unwrap();

        let tab = app_db::create_profile_tab_with(
            &mut conn,
            "claude",
            "Project Beta",
            &["/path/beta".to_string()],
        ).unwrap();

        app_db::set_active_profile_with(&conn, kind, "GlobalProf", Some("global")).unwrap();
        app_db::set_active_profile_with(&conn, kind, "TabProf", Some(&tab.id)).unwrap();

        let bindings = get_scope_bindings_with(&conn, kind).unwrap();
        assert_eq!(bindings.len(), 2);
        assert_eq!(bindings[0].scope, "global");
        assert_eq!(bindings[0].active_profile, Some("GlobalProf".to_string()));

        assert_eq!(bindings[1].scope, tab.id);
        assert_eq!(bindings[1].name, Some("Project Beta".to_string()));
        assert!(!bindings[1].is_global);
        assert_eq!(bindings[1].dirs, vec!["/path/beta".to_string()]);
        assert_eq!(bindings[1].active_profile, Some("TabProf".to_string()));
    }

    /// 守卫：`dirsInconsistent` 只在「有连接配置的目录 ≥ 2 且彼此不同」时置出。
    ///
    /// 这条判据是前端 tooltip 计数 `count ≥ 2` 的**唯一来源**（见
    /// `scripts/i18n-plural-adjudication.json` 里 `statusDirsInconsistentTooltip` 的
    /// `reachable: false` 裁决），所以在这里钉住它：**单目录有配置必须不进
    /// `dirsInconsistent`** —— 任何「让 count=1 变得可达」的谓词改写都会先让本用例变红。
    #[test]
    fn test_scope_dir_status_inconsistent_guard() {
        let mut conn = setup_test_db();
        let kind = CliKind::Claude;
        let temp = tempfile::tempdir().unwrap();

        let write_dir = |name: &str, base_url: &str| -> String {
            let dir = temp.path().join(name);
            fs::create_dir_all(dir.join(".claude")).unwrap();
            fs::write(
                dir.join(".claude").join("settings.json"),
                json!({ "env": { "ANTHROPIC_BASE_URL": base_url } }).to_string(),
            )
            .unwrap();
            dir.to_str().unwrap().to_string()
        };

        let only = write_dir("only", "https://only.example.com");
        let a = write_dir("a", "https://a.example.com");
        let b = write_dir("b", "https://b.example.com");
        let a_again = write_dir("a_again", "https://a.example.com");

        let one = app_db::create_profile_tab_with(&mut conn, "claude", "One", &[only]).unwrap();
        let two_diff = app_db::create_profile_tab_with(
            &mut conn,
            "claude",
            "TwoDiff",
            &[a.clone(), b.clone()],
        )
        .unwrap();
        let two_same =
            app_db::create_profile_tab_with(&mut conn, "claude", "TwoSame", &[a.clone(), a_again])
                .unwrap();

        let statuses = get_scope_dir_status_with(&conn, kind).unwrap();
        let by_scope = |id: &str| {
            statuses
                .iter()
                .find(|s| s.scope == id)
                .unwrap_or_else(|| panic!("作用域 {id} 应有状态"))
        };

        // 1. 只有一个目录有配置 → 不得进 `dirsInconsistent`（count ≥ 2 正是从这里来的）
        let s = by_scope(&one.id);
        assert_eq!(s.status, "unboundWithConfig");
        assert!(matches!(
            s.detail,
            Some(ScopeDirDetail::UnboundWithConfig { .. })
        ));

        // 2. 两个目录配置不同 → `dirsInconsistent`，列表就是参与比较的那两个目录
        let s = by_scope(&two_diff.id);
        assert_eq!(s.status, "dirsInconsistent");
        match &s.detail {
            Some(ScopeDirDetail::DirsInconsistent { dirs }) => {
                assert_eq!(dirs, &vec![a, b]);
            }
            other => panic!("期望 DirsInconsistent，实际 {other:?}"),
        }

        // 3. 两个目录配置相同 → 也不算不一致
        let s = by_scope(&two_same.id);
        assert_eq!(s.status, "unboundWithConfig");
    }

    #[test]
    fn test_set_scope_binding_and_validation() {
        let mut conn = setup_test_db();
        let kind = CliKind::Claude;
        let temp_dir = tempfile::tempdir().unwrap();
        let global_settings_path = temp_dir.path().join("global_settings.json");
        let project_dir = temp_dir.path().join("my_project");
        fs::create_dir_all(&project_dir).unwrap();

        let prof_content = json!({
            "env": {
                "ANTHROPIC_BASE_URL": "https://api.myproject.com"
            }
        }).to_string();
        app_db::save_profile_with(&mut conn, kind, "DevProfile", &prof_content).unwrap();

        let tab = app_db::create_profile_tab_with(
            &mut conn,
            "claude",
            "My Project",
            &[project_dir.to_str().unwrap().to_string()],
        ).unwrap();

        let empty_tab = app_db::create_profile_tab_with(
            &mut conn,
            "claude",
            "Empty Tab",
            &[],
        ).unwrap();

        // 1. Set global binding
        set_scope_binding_with(&conn, kind, "global", "DevProfile", Some(&global_settings_path)).unwrap();
        assert_eq!(app_db::get_active_profile_with(&conn, kind, Some("global")).unwrap(), "DevProfile");
        assert!(global_settings_path.exists());

        // 2. Set tab binding
        set_scope_binding_with(&conn, kind, &tab.id, "DevProfile", None).unwrap();
        assert_eq!(app_db::get_active_profile_with(&conn, kind, Some(&tab.id)).unwrap(), "DevProfile");
        let project_settings = project_dir.join(".claude").join("settings.json");
        assert!(project_settings.exists());

        // 3. Error on non-existent profile
        let err = set_scope_binding_with(&conn, kind, &tab.id, "NonExistent", None).unwrap_err();
        assert_eq!(coded_code(&err), "profile.not_found");

        // 4. Error on tab with empty dirs
        let err = set_scope_binding_with(&conn, kind, &empty_tab.id, "DevProfile", None).unwrap_err();
        assert_eq!(coded_code(&err), "profile.project_tab_no_dirs");

        // 5. Unbind project scope (set to empty string)
        set_scope_binding_with(&conn, kind, &tab.id, "", None).unwrap();
        assert_eq!(app_db::get_active_profile_with(&conn, kind, Some(&tab.id)).unwrap(), "");
        // Cleaned up API settings file if empty
        assert!(!project_settings.exists() || fs::read_to_string(&project_settings).unwrap() == "{}");

        // 6. Global scope cannot be empty
        let err = set_scope_binding_with(&conn, kind, "global", "", None).unwrap_err();
        assert_eq!(coded_code(&err), "profile.global_scope_needs_profile");
    }

    fn identity(
        base_url: Option<&str>,
        token: Option<&str>,
        models: &[Option<&str>],
    ) -> ConnIdentity {
        ConnIdentity {
            base_url: base_url.map(String::from),
            token: token.map(String::from),
            models: models.iter().map(|m| m.map(String::from)).collect(),
        }
    }

    #[test]
    fn match_prefers_model_consistent_profile_when_gateway_token_shared() {
        // 真实案例：同一网关 5 个 profile 共用同一 token，仅靠模型字段区分。
        // 磁盘残留是 deepseek 的配置，不能按列表顺序误命中 claude。
        let profiles = vec![
            (
                "claude".to_string(),
                identity(
                    Some("https://gw"),
                    Some("sk-1"),
                    &[None, Some("ds-haiku"), Some("glm-5.2"), Some("claude-opus-4-8")],
                ),
            ),
            (
                "deepseek".to_string(),
                identity(
                    Some("https://gw"),
                    Some("sk-1"),
                    &[None, Some("ds-haiku"), Some("ds[1m]"), Some("ds[1m]")],
                ),
            ),
        ];
        let disk = identity(
            Some("https://gw"),
            Some("sk-1"),
            &[None, Some("ds-haiku"), Some("ds[1m]"), Some("ds[1m]")],
        );
        assert_eq!(
            match_profile_by_identity(&profiles, &disk),
            Some("deepseek".to_string())
        );
    }

    #[test]
    fn match_returns_none_when_all_candidates_have_model_conflict() {
        let profiles = vec![(
            "claude".to_string(),
            identity(
                Some("https://gw"),
                Some("sk-1"),
                &[None, None, Some("glm-5.2"), None],
            ),
        )];
        let disk = identity(
            Some("https://gw"),
            Some("sk-1"),
            &[None, None, Some("ds[1m]"), None],
        );
        assert_eq!(match_profile_by_identity(&profiles, &disk), None);
    }

    #[test]
    fn match_requires_base_url_and_token_equal() {
        let profiles = vec![(
            "a".to_string(),
            identity(Some("https://gw"), Some("sk-1"), &[None, None, None, None]),
        )];
        // token 不同
        let disk = identity(Some("https://gw"), Some("sk-2"), &[None, None, None, None]);
        assert_eq!(match_profile_by_identity(&profiles, &disk), None);
        // base_url 不同
        let disk = identity(Some("https://other"), Some("sk-1"), &[None, None, None, None]);
        assert_eq!(match_profile_by_identity(&profiles, &disk), None);
        // 完全相同
        let disk = identity(Some("https://gw"), Some("sk-1"), &[None, None, None, None]);
        assert_eq!(match_profile_by_identity(&profiles, &disk), Some("a".to_string()));
    }

    #[test]
    fn match_profile_without_model_fields_matches_disk_with_models() {
        // profile 未配模型字段（不参与比较），url+token 相同即可匹配
        let profiles = vec![(
            "plain".to_string(),
            identity(Some("https://gw"), Some("sk-1"), &[None, None, None, None]),
        )];
        let disk = identity(
            Some("https://gw"),
            Some("sk-1"),
            &[Some("any-model"), None, None, None],
        );
        assert_eq!(
            match_profile_by_identity(&profiles, &disk),
            Some("plain".to_string())
        );
    }
}
