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
    pub total_requests: i64,
    pub daily_used: f64,
    pub daily_requests: i64,
    pub daily_input_tokens: i64,
    pub daily_output_tokens: i64,
    pub daily_reasoning_tokens: i64,
    pub daily_cached_tokens: i64,
    pub monthly_used: f64,
    pub monthly_requests: i64,
    pub monthly_input_tokens: i64,
    pub monthly_output_tokens: i64,
    pub monthly_reasoning_tokens: i64,
    pub monthly_cached_tokens: i64,
    pub hard_limit: f64,
    pub daily_limit: f64,
    pub rpm: i64,
    pub rph: i64,
    pub rpd: i64,
}

/// Fetch Ainaiba credit info from the remote dashboard API.
/// Calls both `/dashboard/info` (account + credit balance) and
/// `/dashboard/live` (daily/monthly usage) concurrently.
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

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .unwrap_or_default();

    let info_fut = fetch_dashboard(&client, &api_key, "/dashboard/info");
    let live_fut = fetch_dashboard(&client, &api_key, "/dashboard/live");

    let (info_json, live_json) = tokio::join!(info_fut, live_fut);

    let info_json = match info_json {
        Ok(v) => v,
        Err(e) => {
            return AinaibaCreditResponse {
                available: false,
                data: None,
                error: Some(format!("Info API failed: {}", e)),
            };
        }
    };

    // Live dashboard data is supplemental — daily/monthly usage enriches the card.
    // If it fails we still return credit balance from /dashboard/info,
    // just with daily_used=0 and monthly_used=credit_used as fallback.
    let live_json = match live_json {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("Ainaiba live API failed (optional, continuing): {}", e);
            serde_json::Value::Null
        }
    };

    match parse_credit_data(&info_json, &live_json) {
        Ok(data) => AinaibaCreditResponse {
            available: true,
            data: Some(data),
            error: None,
        },
        Err(e) => {
            tracing::warn!("Failed to extract Ainaiba credit data: {}", e);
            AinaibaCreditResponse {
                available: false,
                data: None,
                error: Some(format!("Data extraction error: {}", e)),
            }
        }
    }
}

async fn fetch_dashboard(
    client: &reqwest::Client,
    api_key: &str,
    path: &str,
) -> Result<serde_json::Value, String> {
    let url = format!("{}{}", AINAIBA_API_BASE, path);
    // Produces e.g. "https://api-xai.ainaibahub.com/dashboard/live".
    let response = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", api_key))
        .send()
        .await
        .map_err(|e| format!("Request failed: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status()));
    }

    response
        .json::<serde_json::Value>()
        .await
        .map_err(|e| format!("Parse error: {}", e))
}

fn parse_credit_data(
    info: &serde_json::Value,
    live: &serde_json::Value,
) -> Result<AinaibaCreditData, String> {
    let user_id = info["id"]
        .as_i64()
        .ok_or_else(|| "missing 'id'".to_string())?;
    let name = info["name"].as_str().unwrap_or("").to_string();
    let email = info["email"].as_str().unwrap_or("").to_string();
    let alias = info["alias"].as_str().unwrap_or("").to_string();
    let balance = info["balance"].as_f64().unwrap_or(0.0);
    let credit_used = info["credit_used"].as_f64().unwrap_or(0.0);
    let hard_limit = info["hard_limit"].as_f64().unwrap_or(0.0);
    let daily_limit = info["daily_limit"].as_f64().unwrap_or(0.0);
    let rpm = info["rpm"].as_i64().unwrap_or(0);
    let rph = info["rph"].as_i64().unwrap_or(0);
    let rpd = info["rpd"].as_i64().unwrap_or(0);
    let total_requests = info["requests"].as_i64().unwrap_or(0);

    // Extract from credit_balance array
    let credit_balance = &info["credit_balance"];
    let credit_total = credit_balance
        .get(0)
        .and_then(|c| c["amount"].as_f64())
        .unwrap_or(0.0);
    let expires_at = credit_balance
        .get(0)
        .and_then(|c| c["expires_at"].as_str())
        .unwrap_or("")
        .to_string();

    // Extract daily/monthly usage from live dashboard data
    let daily_used = live["daily_usage"]["CreditUsed"].as_f64().unwrap_or(0.0);
    let daily_requests = live["daily_usage"]["Requests"].as_i64().unwrap_or(0);
    let daily_input_tokens = live["daily_usage"]["Prompt"].as_i64().unwrap_or(0);
    let daily_output_tokens = live["daily_usage"]["Completion"].as_i64().unwrap_or(0);
    let daily_reasoning_tokens = live["daily_usage"]["Reasoning"].as_i64().unwrap_or(0);
    let daily_cached_tokens = live["daily_usage"]["PromptDetails"]["cached_tokens"]
        .as_i64()
        .unwrap_or(0);

    let monthly_used = live["monthly_usage"]["CreditUsed"]
        .as_f64()
        .unwrap_or(credit_used); // fallback to total if live data missing
    let monthly_requests = live["monthly_usage"]["Requests"].as_i64().unwrap_or(0);
    let monthly_input_tokens = live["monthly_usage"]["Prompt"].as_i64().unwrap_or(0);
    let monthly_output_tokens = live["monthly_usage"]["Completion"].as_i64().unwrap_or(0);
    let monthly_reasoning_tokens = live["monthly_usage"]["Reasoning"].as_i64().unwrap_or(0);
    let monthly_cached_tokens = live["monthly_usage"]["PromptDetails"]["cached_tokens"]
        .as_i64()
        .unwrap_or(0);

    Ok(AinaibaCreditData {
        user_id,
        name,
        email,
        alias,
        balance,
        credit_total,
        credit_used,
        expires_at,
        total_requests,
        daily_used,
        daily_requests,
        daily_input_tokens,
        daily_output_tokens,
        daily_reasoning_tokens,
        daily_cached_tokens,
        monthly_used,
        monthly_requests,
        monthly_input_tokens,
        monthly_output_tokens,
        monthly_reasoning_tokens,
        monthly_cached_tokens,
        hard_limit,
        daily_limit,
        rpm,
        rph,
        rpd,
    })
}
