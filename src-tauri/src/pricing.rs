use crate::db::write_setting_json;
use crate::error::{AppError, AppResult};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

pub const PRICING_CATALOG_URL: &str = "https://models.dev/api.json";
const APP_SETTING_PRICING_CATALOG: &str = "pricing.catalog";
const APP_SETTING_PRICING_UPDATED_AT: &str = "pricing.updated_at";
const APP_SETTING_PRICING_MODEL_COUNT: &str = "pricing.model_count";

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct RemoteModelPricing {
    pub input_cost_per_token: Option<f64>,
    pub output_cost_per_token: Option<f64>,
    pub cache_read_input_token_cost: Option<f64>,
    pub cache_creation_input_token_cost: Option<f64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PricingCatalogResponse {
    pub catalog: HashMap<String, RemoteModelPricing>,
    pub updated_at: Option<String>,
    pub model_count: usize,
}

#[derive(Deserialize)]
struct ModelsDevCost {
    input: Option<f64>,
    output: Option<f64>,
    cache_read: Option<f64>,
    cache_write: Option<f64>,
    cache_creation: Option<f64>,
}

#[derive(Deserialize)]
struct ModelsDevModel {
    id: Option<String>,
    cost: Option<ModelsDevCost>,
    pricing: Option<ModelsDevCost>,
}

#[derive(Deserialize)]
struct ModelsDevProvider {
    id: Option<String>,
    models: Option<BTreeMap<String, ModelsDevModel>>,
}

const SKIP_KEYWORDS: &[&str] = &[
    "speech", "embedding", "image", "tts", "asr", "video", "ocr", "audio", "realtime", "vision",
];

const PREFERRED_ALIAS_PROVIDERS: &[&str] = &[
    "anthropic",
    "openai",
    "google",
    "deepseek",
    "moonshotai",
    "moonshotai-cn",
    "kimi-for-coding",
    "zhipuai",
    "zai",
    "alibaba",
    "alibaba-cn",
    "minimax",
    "stepfun",
    "xai",
    "meta",
    "mistral",
    "baichuan",
    "volcengine",
];

pub fn normalize_model_key(model: &str) -> String {
    model.trim().to_lowercase().replace("models/", "")
}

pub fn parse_models_dev(json_str: &str) -> AppResult<(HashMap<String, RemoteModelPricing>, usize)> {
    let providers: BTreeMap<String, ModelsDevProvider> = serde_json::from_str(json_str)
        .map_err(|e| AppError::coded("pricing.catalog_parse_failed").with("detail", e.to_string()))?;

    let mut catalog: HashMap<String, RemoteModelPricing> = HashMap::new();
    let mut unique_models: usize = 0;

    let mut ordered: Vec<_> = providers.iter().collect();
    ordered.sort_by_key(|(id, _)| {
        PREFERRED_ALIAS_PROVIDERS
            .iter()
            .position(|&candidate| candidate == id.as_str())
            .unwrap_or(usize::MAX)
    });

    for (provider_id, provider) in &ordered {
        let p_key = normalize_model_key(provider.id.as_deref().unwrap_or(provider_id));
        if !PREFERRED_ALIAS_PROVIDERS.iter().any(|&p| p_key == p || p_key.starts_with(p)) {
            continue;
        }
        let models = match &provider.models {
            Some(m) => m,
            None => continue,
        };

        for (model_id, model) in models {
            let m_key = normalize_model_key(model.id.as_deref().unwrap_or(model_id));
            if m_key.is_empty() {
                continue;
            }

            let cost = match model.cost.as_ref().or(model.pricing.as_ref()) {
                Some(c) => c,
                None => continue,
            };

            let input = cost.input.filter(|&v| v > 0.0);
            let output = cost.output.filter(|&v| v > 0.0);
            let cache_read = cost.cache_read.filter(|&v| v > 0.0);
            let cache_write = cost.cache_write.or(cost.cache_creation).filter(|&v| v > 0.0);

            if input.is_none() && output.is_none() && cache_read.is_none() && cache_write.is_none() {
                continue;
            }

            unique_models += 1;

            let pricing = RemoteModelPricing {
                input_cost_per_token: input.map(|v| v / 1_000_000.0),
                output_cost_per_token: output.map(|v| v / 1_000_000.0),
                cache_read_input_token_cost: cache_read.map(|v| v / 1_000_000.0),
                cache_creation_input_token_cost: cache_write.map(|v| v / 1_000_000.0),
            };

            let full_key = format!("{p_key}/{m_key}");
            catalog.insert(full_key, pricing.clone());

            // Short-name alias: first preferred provider wins
            if !catalog.contains_key(&m_key) && !SKIP_KEYWORDS.iter().any(|kw| m_key.contains(kw)) {
                catalog.insert(m_key, pricing);
            }
        }
    }

    Ok((catalog, unique_models))
}

#[tauri::command]
pub async fn refresh_pricing_catalog() -> AppResult<PricingCatalogResponse> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(12))
        .user_agent("SeshBuddy-App/1.0 (Macintosh; Apple Silicon)")
        .build()
        .map_err(|e| AppError::coded("pricing.http_client_failed").with("detail", e.to_string()))?;

    let resp = client
        .get(PRICING_CATALOG_URL)
        .send()
        .await
        .map_err(|e| AppError::coded("pricing.request_failed").with("detail", e.to_string()))?;

    if !resp.status().is_success() {
        return Err(AppError::coded("pricing.http_status").with("status", resp.status().to_string()));
    }

    let body = resp
        .text()
        .await
        .map_err(|e| AppError::coded("pricing.body_read_failed").with("detail", e.to_string()))?;

    let (catalog, model_count) = parse_models_dev(&body)?;
    let now = Utc::now().to_rfc3339();

    // Cache in SQLite
    write_setting_json(APP_SETTING_PRICING_CATALOG, &catalog)?;
    write_setting_json(APP_SETTING_PRICING_UPDATED_AT, &now)?;
    write_setting_json(APP_SETTING_PRICING_MODEL_COUNT, &model_count)?;

    Ok(PricingCatalogResponse {
        catalog,
        updated_at: Some(now),
        model_count,
    })
}

