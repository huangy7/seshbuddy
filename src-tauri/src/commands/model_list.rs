use crate::error::{AppError, AppResult};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

#[derive(Deserialize)]
struct ModelsResponse {
    data: Vec<ModelEntry>,
}

#[derive(Deserialize)]
struct ModelEntry {
    id: String,
    display: Option<String>,
    billing_rules: Option<String>,
}

#[derive(Serialize)]
pub struct ModelInfo {
    id: String,
    display: Option<String>,
    billing_rules: Option<String>,
}

#[derive(Serialize)]
pub struct FetchResult {
    models: Vec<ModelInfo>,
    latency_ms: u128,
}

#[tauri::command]
pub async fn fetch_available_models(base_url: String, api_key: String) -> AppResult<FetchResult> {
    let trimmed = base_url.trim_end_matches('/');
    let url = if trimmed.ends_with("/v1") || trimmed.ends_with("/v2") {
        format!("{}/models", trimmed)
    } else {
        format!("{}/v1/models", trimmed)
    };

    // 与 `pricing.rs` 的目录刷新共用同一个码：同一句「创建 HTTP 客户端失败」只建一个码，
    // 否则同一句英文在语言包里出现两遍，译文一改就要改两处。
    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| AppError::coded("pricing.http_client_failed").with("detail", e.to_string()))?;

    // 发起一次无鉴权的热身请求，用于复用连接/绕过首包惩罚
    let _ = client.get(&url).send().await;

    // 第二次请求开始计时，获取真实的网络往返延迟
    let start = Instant::now();
    let resp = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| AppError::coded("model_list.request_failed").with("detail", e.to_string()))?;

    let latency_ms = start.elapsed().as_millis();

    if !resp.status().is_success() {
        return Err(AppError::coded("model_list.http_status").with("status", resp.status().to_string()));
    }

    let body: ModelsResponse = resp
        .json()
        .await
        .map_err(|e| AppError::coded("model_list.response_parse_failed").with("detail", e.to_string()))?;

    let models: Vec<ModelInfo> = body
        .data
        .into_iter()
        .map(|m| ModelInfo {
            id: m.id,
            display: m.display,
            billing_rules: m.billing_rules,
        })
        .collect();

    Ok(FetchResult { models, latency_ms })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_model_entry_with_optional_display() {
        let json = r#"{"data":[{"id":"kimi-k2.6","display":"Kimi K2.6"},{"id":"no-display"}]}"#;
        let resp: ModelsResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.data.len(), 2);
        assert_eq!(resp.data[0].id, "kimi-k2.6");
        assert_eq!(resp.data[0].display.as_deref(), Some("Kimi K2.6"));
        assert_eq!(resp.data[1].id, "no-display");
        assert_eq!(resp.data[1].display, None);
    }

    #[test]
    fn parses_model_entry_with_optional_billing_rules() {
        let json = r#"{"data":[{"id":"kimi-k2.6","billing_rules":"tier(\"base\", p * 5 + c * 25 + cr * 0.5)"},{"id":"no-rules"}]}"#;
        let resp: ModelsResponse = serde_json::from_str(json).unwrap();
        assert_eq!(
            resp.data[0].billing_rules.as_deref(),
            Some("tier(\"base\", p * 5 + c * 25 + cr * 0.5)")
        );
        assert_eq!(resp.data[1].billing_rules, None);
    }
}
