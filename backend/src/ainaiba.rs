//! Ainaiba (XAI) credit balance fetcher.
//!
//! Queries the Ainaiba dashboard API at `api-xai.ainaibahub.com` using
//! the `XAI_API_KEY` environment variable.

use serde::Serialize;

const AINAIBA_API_BASE: &str = "https://api-xai.ainaibahub.com";

/// Response wrapper with availability flag and optional error.
#[derive(Debug, Serialize)]
pub struct AinaibaCreditResponse {
    pub available: bool,
    pub data: Option<AinaibaCreditData>,
    pub error: Option<String>,
}

/// Parsed credit balance and usage data from the Ainaiba dashboard.
#[derive(Debug, Serialize)]
pub struct AinaibaCreditData {
    pub user_id: i64,
    pub name: String,
    pub email: String,
    pub alias: String,
    pub balance: f64,
    pub credit_total: f64,
    pub credit_used: f64,
    pub expires_at: String,
    pub daily_used: f64,
    pub daily_requests: i64,
    pub monthly_used: f64,
    pub monthly_requests: i64,
    pub hard_limit: f64,
    pub daily_limit: f64,
}

/// Fetch Ainaiba credit info from the remote dashboard API.
pub async fn fetch_ainaiba_credit() -> AinaibaCreditResponse {
    let api_key = match std::env::var("XAI_API_KEY") {
        Ok(key) if !key.is_empty() => key,
        _ => {
            return AinaibaCreditResponse {
                available: false,
                data: None,
                error: Some("XAI_API_KEY not configured".to_string()),
            };
        }
    };

    let client = reqwest::Client::new();
    let url = format!("{}/dashboard/info", AINAIBA_API_BASE);

    let response = match client
        .get(&url)
        .header("Authorization", format!("Bearer {}", api_key))
        .send()
        .await
    {
        Ok(resp) => resp,
        Err(e) => {
            tracing::warn!("Ainaiba API request failed: {}", e);
            return AinaibaCreditResponse {
                available: false,
                data: None,
                error: Some(format!("Request failed: {}", e)),
            };
        }
    };

    if !response.status().is_success() {
        let status = response.status();
        tracing::warn!("Ainaiba API returned status {}", status);
        return AinaibaCreditResponse {
            available: false,
            data: None,
            error: Some(format!("HTTP {}", status)),
        };
    }

    let json: serde_json::Value = match response.json().await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("Failed to parse Ainaiba API response: {}", e);
            return AinaibaCreditResponse {
                available: false,
                data: None,
                error: Some(format!("Parse error: {}", e)),
            };
        }
    };

    let data = match parse_credit_data(&json) {
        Ok(d) => d,
        Err(e) => {
            tracing::warn!("Failed to extract credit data: {}", e);
            return AinaibaCreditResponse {
                available: false,
                data: None,
                error: Some(format!("Data extraction error: {}", e)),
            };
        }
    };

    AinaibaCreditResponse {
        available: true,
        data: Some(data),
        error: None,
    }
}

fn parse_credit_data(json: &serde_json::Value) -> Result<AinaibaCreditData, String> {
    let user_id = json["id"]
        .as_i64()
        .ok_or_else(|| "missing 'id'".to_string())?;
    let name = json["name"]
        .as_str()
        .unwrap_or("")
        .to_string();
    let email = json["email"]
        .as_str()
        .unwrap_or("")
        .to_string();
    let alias = json["alias"]
        .as_str()
        .unwrap_or("")
        .to_string();
    let balance = json["balance"]
        .as_f64()
        .unwrap_or(0.0);
    let credit_used = json["credit_used"]
        .as_f64()
        .unwrap_or(0.0);
    let hard_limit = json["hard_limit"]
        .as_f64()
        .unwrap_or(0.0);
    let daily_limit = json["daily_limit"]
        .as_f64()
        .unwrap_or(0.0);
    let monthly_requests = json["requests"]
        .as_i64()
        .unwrap_or(0);

    // Extract from credit_balance array
    let credit_balance = &json["credit_balance"];
    let credit_total = credit_balance
        .get(0)
        .and_then(|c| c["amount"].as_f64())
        .unwrap_or(0.0);
    let expires_at = credit_balance
        .get(0)
        .and_then(|c| c["expires_at"].as_str())
        .unwrap_or("")
        .to_string();

    // Future: could also call /dashboard/live for daily breakdown.
    // For now, use credit_used as monthly proxy and leave daily as 0.
    let daily_used = 0.0;
    let daily_requests = 0;
    let monthly_used = credit_used;

    Ok(AinaibaCreditData {
        user_id,
        name,
        email,
        alias,
        balance,
        credit_total,
        credit_used,
        expires_at,
        daily_used,
        daily_requests,
        monthly_used,
        monthly_requests,
        hard_limit,
        daily_limit,
    })
}
