//! Quota/balance fetchers for external AI API providers.
//!
//! Supports:
//! - **Kimi (Moonshot)**: Balance via `GET /v1/users/me/balance`
//! - **OpenCode-go**: Usage/quota via OpenAI-compatible billing endpoints
//!
//! Auth keys are read from environment variables or local config files.

use chrono::Datelike;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

// ─── Error type ───────────────────────────────────────────────────────────────

#[cfg(test)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuotaError {
    pub provider: String,
    pub message: String,
}

#[cfg(test)]
impl QuotaError {
    pub fn new(provider: &str, message: &str) -> Self {
        Self {
            provider: provider.to_string(),
            message: message.to_string(),
        }
    }
}

#[cfg(test)]
impl std::fmt::Display for QuotaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.provider, self.message)
    }
}

// ─── Kimi (Moonshot) ─────────────────────────────────────────────────────────

/// Raw response from `GET /v1/users/me/balance` on the Moonshot API.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MoonshotBalanceResponse {
    pub code: i64,
    pub data: MoonshotBalanceData,
    pub scode: String,
    pub status: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MoonshotBalanceData {
    pub available_balance: f64,
    pub voucher_balance: f64,
    pub cash_balance: f64,
}

/// Simplified Kimi quota info for the dashboard.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuotaKimi {
    pub provider: String,
    pub available_balance: f64,
    pub voucher_balance: f64,
    pub cash_balance: f64,
}

// ─── OpenCode-go ──────────────────────────────────────────────────────────────

/// Raw response from the OpenAI-compatible `/v1/dashboard/billing/subscription` endpoint.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OpenAiSubscriptionResponse {
    pub has_payment_method: Option<bool>,
    pub canceled: Option<bool>,
    pub canceled_at: Option<i64>,
    pub delinquent: Option<bool>,
    pub access_until: Option<i64>,
    pub soft_limit: Option<i64>,
    pub hard_limit: Option<i64>,
    pub system_hard_limit: Option<i64>,
    pub soft_limit_usd: Option<f64>,
    pub hard_limit_usd: Option<f64>,
    pub system_hard_limit_usd: Option<f64>,
    pub plan: Option<OpenAiPlan>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OpenAiPlan {
    pub title: Option<String>,
    pub id: Option<String>,
}

/// Raw response from `/v1/dashboard/billing/usage` endpoint.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OpenAiUsageResponse {
    pub object: Option<String>,
    pub total_usage: Option<f64>,
    pub total_tokens_used: Option<i64>,
}

/// Simplified OpenCode-go quota info for the dashboard.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuotaOpenCode {
    pub provider: String,
    pub plan_type: Option<String>,
    pub hard_limit_usd: Option<f64>,
    pub total_usage_usd: Option<f64>,
    pub usage_percent: Option<f64>,
    pub remaining_usd: Option<f64>,
}

// ─── Unified quota response ───────────────────────────────────────────────────

/// Aggregated quota response for the dashboard.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuotaResponse {
    pub kimi: Option<KimiQuotaStatus>,
    pub opencode_go: Option<OpenCodeQuotaStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KimiQuotaStatus {
    pub available: bool,
    pub data: Option<QuotaKimi>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenCodeQuotaStatus {
    pub available: bool,
    pub data: Option<QuotaOpenCode>,
    pub error: Option<String>,
}

// ─── Auth helpers ─────────────────────────────────────────────────────────────

/// Read Moonshot API key from environment variable.
pub fn get_moonshot_api_key() -> Option<String> {
    std::env::var("MOONSHOT_API_KEY")
        .ok()
        .filter(|k| !k.is_empty())
}

/// Path to the OpenCode auth.json file.
/// Can be overridden via the `OPENCODE_AUTH_PATH` env var (useful for testing).
pub fn get_opencode_auth_path() -> PathBuf {
    if let Ok(path) = std::env::var("OPENCODE_AUTH_PATH") {
        return PathBuf::from(path);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home)
        .join(".local")
        .join("share")
        .join("opencode")
        .join("auth.json")
}

/// Read opencode-go API key from local auth.json or env var `OPENCODE_API_KEY`.
pub fn get_opencode_api_key() -> Option<String> {
    // Check env var first
    if let Ok(key) = std::env::var("OPENCODE_API_KEY") {
        if !key.is_empty() {
            return Some(key);
        }
    }

    // Fall back to reading ~/.local/share/opencode/auth.json
    let path = get_opencode_auth_path();
    if !path.exists() {
        return None;
    }

    let content = std::fs::read_to_string(path).ok()?;
    let auth: serde_json::Value = serde_json::from_str(&content).ok()?;

    // Look for "opencode-go" entry with "key" field
    if let Some(entry) = auth.get("opencode-go") {
        if let Some(key) = entry.get("key").and_then(|k| k.as_str()) {
            if !key.is_empty() {
                return Some(key.to_string());
            }
        }
    }

    None
}

/// Get the OpenCode base URL for API calls.
pub fn get_opencode_base_url() -> String {
    std::env::var("OPENCODE_BASE_URL").unwrap_or_else(|_| "https://opencode.ai/zen/v1".to_string())
}

/// Get the Moonshot API base URL.
pub fn get_moonshot_base_url() -> String {
    std::env::var("MOONSHOT_BASE_URL").unwrap_or_else(|_| "https://api.moonshot.ai".to_string())
}

// ─── Fetcher ──────────────────────────────────────────────────────────────────

/// Quota fetcher with injectable HTTP client for testability.
pub struct QuotaFetcher {
    pub client: reqwest::Client,
}

impl QuotaFetcher {
    /// Create a new fetcher with the default reqwest client.
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }

    /// Fetch Kimi balance from the Moonshot API.
    pub async fn fetch_kimi_balance(&self) -> KimiQuotaStatus {
        let api_key = match get_moonshot_api_key() {
            Some(k) => k,
            None => {
                return KimiQuotaStatus {
                    available: false,
                    data: None,
                    error: Some("MOONSHOT_API_KEY not configured".to_string()),
                };
            }
        };

        let base_url = get_moonshot_base_url();
        let url = format!("{}/v1/users/me/balance", base_url.trim_end_matches('/'));

        let response = match self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", api_key))
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                return KimiQuotaStatus {
                    available: false,
                    data: None,
                    error: Some(format!("HTTP request failed: {}", e)),
                };
            }
        };

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return KimiQuotaStatus {
                available: false,
                data: None,
                error: Some(format!("API returned {}: {}", status, body)),
            };
        }

        let balance: MoonshotBalanceResponse = match response.json().await {
            Ok(b) => b,
            Err(e) => {
                return KimiQuotaStatus {
                    available: false,
                    data: None,
                    error: Some(format!("Failed to parse response: {}", e)),
                };
            }
        };

        if balance.code != 0 {
            return KimiQuotaStatus {
                available: false,
                data: None,
                error: Some(format!(
                    "API error code {}: {}",
                    balance.code, balance.scode
                )),
            };
        }

        KimiQuotaStatus {
            available: true,
            data: Some(QuotaKimi {
                provider: "kimi".to_string(),
                available_balance: balance.data.available_balance,
                voucher_balance: balance.data.voucher_balance,
                cash_balance: balance.data.cash_balance,
            }),
            error: None,
        }
    }

    /// Fetch OpenCode-go subscription/quota info.
    ///
    /// Calls the OpenAI-compatible billing endpoints at the configured base URL.
    /// First tries `/v1/dashboard/billing/subscription` for plan info and limits,
    /// then `/v1/dashboard/billing/usage` for current usage.
    pub async fn fetch_opencode_quota(&self) -> OpenCodeQuotaStatus {
        let api_key = match get_opencode_api_key() {
            Some(k) => k,
            None => {
                return OpenCodeQuotaStatus {
                    available: false,
                    data: None,
                    error: Some(
                        "opencode-go API key not found in auth.json or OPENCODE_API_KEY"
                            .to_string(),
                    ),
                };
            }
        };

        let base_url = get_opencode_base_url();
        let base = base_url.trim_end_matches('/');

        // Step 1: Fetch subscription info
        let sub_url = format!("{}/v1/dashboard/billing/subscription", base);
        let sub_response = self
            .client
            .get(&sub_url)
            .header("Authorization", format!("Bearer {}", api_key))
            .send()
            .await;

        let subscription: OpenAiSubscriptionResponse = match sub_response {
            Ok(r) if r.status().is_success() => match r.json().await {
                Ok(s) => s,
                Err(e) => {
                    return OpenCodeQuotaStatus {
                        available: false,
                        data: None,
                        error: Some(format!("Failed to parse subscription response: {}", e)),
                    };
                }
            },
            Ok(r) => {
                let status = r.status();
                let body = r.text().await.unwrap_or_default();
                return OpenCodeQuotaStatus {
                    available: false,
                    data: None,
                    error: Some(format!("Subscription API returned {}: {}", status, body)),
                };
            }
            Err(e) => {
                return OpenCodeQuotaStatus {
                    available: false,
                    data: None,
                    error: Some(format!("Subscription request failed: {}", e)),
                };
            }
        };

        let plan_type = subscription
            .plan
            .as_ref()
            .and_then(|p| p.title.clone())
            .or_else(|| subscription.plan.as_ref().and_then(|p| p.id.clone()));
        let hard_limit_usd = subscription.hard_limit_usd;

        // Step 2: Fetch current usage
        let now = chrono::Utc::now();
        let start_of_month = chrono::NaiveDate::from_ymd_opt(now.year(), now.month(), 1)
            .unwrap_or_else(|| now.date_naive());
        let end_date = now.format("%Y-%m-%d").to_string();
        let start_date = start_of_month.format("%Y-%m-%d").to_string();

        let usage_url = format!(
            "{}/v1/dashboard/billing/usage?start_date={}&end_date={}",
            base, start_date, end_date
        );

        let total_usage_usd;
        let usage_percent;

        match self
            .client
            .get(&usage_url)
            .header("Authorization", format!("Bearer {}", api_key))
            .send()
            .await
        {
            Ok(r) if r.status().is_success() => match r.json::<OpenAiUsageResponse>().await {
                Ok(usage) => {
                    total_usage_usd = usage.total_usage;
                    usage_percent = hard_limit_usd.and_then(|limit| {
                        if limit > 0.0 {
                            total_usage_usd.map(|used| (used / limit * 100.0).min(100.0))
                        } else {
                            None
                        }
                    });
                }
                Err(_) => {
                    total_usage_usd = None;
                    usage_percent = None;
                }
            },
            _ => {
                // Usage endpoint may not be available; that's OK
                total_usage_usd = None;
                usage_percent = None;
            }
        }

        let remaining_usd = match (hard_limit_usd, total_usage_usd) {
            (Some(limit), Some(used)) => Some((limit - used).max(0.0)),
            _ => None,
        };

        OpenCodeQuotaStatus {
            available: true,
            data: Some(QuotaOpenCode {
                provider: "opencode-go".to_string(),
                plan_type,
                hard_limit_usd,
                total_usage_usd,
                usage_percent,
                remaining_usd,
            }),
            error: None,
        }
    }
}

impl Default for QuotaFetcher {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn reset_env_var(name: &str, old: Option<String>) {
        match old {
            Some(v) => std::env::set_var(name, v),
            None => std::env::remove_var(name),
        }
    }

    // ── Auth helpers ────────────────────────────────────────────────────

    #[test]
    fn test_get_moonshot_api_key_from_env() {
        temp_env::with_var("MOONSHOT_API_KEY", Some("sk-test-key-123"), || {
            assert_eq!(get_moonshot_api_key(), Some("sk-test-key-123".to_string()));
        });
    }

    #[test]
    fn test_get_moonshot_api_key_empty() {
        temp_env::with_var("MOONSHOT_API_KEY", Some(""), || {
            assert_eq!(get_moonshot_api_key(), None);
        });
    }

    #[test]
    fn test_get_moonshot_api_key_unset() {
        temp_env::with_var("MOONSHOT_API_KEY", None::<&str>, || {
            assert_eq!(get_moonshot_api_key(), None);
        });
    }

    #[test]
    fn test_get_opencode_base_url_default() {
        temp_env::with_var("OPENCODE_BASE_URL", None::<&str>, || {
            assert_eq!(get_opencode_base_url(), "https://opencode.ai/zen/v1");
        });
    }

    #[test]
    fn test_get_opencode_base_url_custom() {
        temp_env::with_var("OPENCODE_BASE_URL", Some("https://custom.url/v1"), || {
            assert_eq!(get_opencode_base_url(), "https://custom.url/v1");
        });
    }

    // ── Kimi balance parsing ────────────────────────────────────────────

    #[test]
    fn test_parse_kimi_balance_success() {
        let json = r#"{
            "code": 0,
            "data": {
                "available_balance": 49.58894,
                "voucher_balance": 46.58893,
                "cash_balance": 3.00001
            },
            "scode": "0x0",
            "status": true
        }"#;

        let resp: MoonshotBalanceResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.code, 0);
        assert!((resp.data.available_balance - 49.58894).abs() < 0.001);
        assert!((resp.data.voucher_balance - 46.58893).abs() < 0.001);
        assert!((resp.data.cash_balance - 3.00001).abs() < 0.001);
        assert_eq!(resp.scode, "0x0");
        assert!(resp.status);
    }

    #[test]
    fn test_parse_kimi_balance_zero() {
        let json = r#"{
            "code": 0,
            "data": {
                "available_balance": 0.0,
                "voucher_balance": 0.0,
                "cash_balance": 0.0
            },
            "scode": "0x0",
            "status": true
        }"#;

        let resp: MoonshotBalanceResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.code, 0);
        assert!((resp.data.available_balance - 0.0).abs() < 0.001);
    }

    #[test]
    fn test_parse_kimi_balance_error_code() {
        let json = r#"{
            "code": 401,
            "data": {
                "available_balance": 0.0,
                "voucher_balance": 0.0,
                "cash_balance": 0.0
            },
            "scode": "0x1",
            "status": false
        }"#;

        let resp: MoonshotBalanceResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.code, 401);
        assert!(!resp.status);
    }

    // ── OpenAI subscription parsing ─────────────────────────────────────

    #[test]
    fn test_parse_openai_subscription_with_plan() {
        let json = r#"{
            "has_payment_method": true,
            "canceled": false,
            "canceled_at": null,
            "delinquent": null,
            "access_until": 1777777777,
            "soft_limit": 0,
            "hard_limit": 0,
            "system_hard_limit": 0,
            "soft_limit_usd": 0.0,
            "hard_limit_usd": 100.0,
            "system_hard_limit_usd": 200.0,
            "plan": {
                "title": "Go Plan",
                "id": "opencode-go-monthly"
            }
        }"#;

        let resp: OpenAiSubscriptionResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.has_payment_method, Some(true));
        assert_eq!(resp.hard_limit_usd, Some(100.0));
        assert_eq!(
            resp.plan.as_ref().and_then(|p| p.title.as_deref()),
            Some("Go Plan")
        );
        assert_eq!(
            resp.plan.as_ref().and_then(|p| p.id.as_deref()),
            Some("opencode-go-monthly")
        );
    }

    #[test]
    fn test_parse_openai_subscription_no_plan() {
        let json = r#"{
            "hard_limit_usd": 50.0
        }"#;

        let resp: OpenAiSubscriptionResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.hard_limit_usd, Some(50.0));
        assert!(resp.plan.is_none());
    }

    #[test]
    fn test_parse_openai_subscription_empty() {
        let json = "{}";
        let resp: OpenAiSubscriptionResponse = serde_json::from_str(json).unwrap();
        assert!(resp.hard_limit_usd.is_none());
        assert!(resp.plan.is_none());
    }

    // ── OpenAI usage parsing ────────────────────────────────────────────

    #[test]
    fn test_parse_openai_usage() {
        let json = r#"{
            "object": "list",
            "total_usage": 25.50,
            "total_tokens_used": 150000
        }"#;

        let resp: OpenAiUsageResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.object, Some("list".to_string()));
        assert!((resp.total_usage.unwrap() - 25.50).abs() < 0.01);
        assert_eq!(resp.total_tokens_used, Some(150000));
    }

    #[test]
    fn test_parse_openai_usage_empty() {
        let json = "{}";
        let resp: OpenAiUsageResponse = serde_json::from_str(json).unwrap();
        assert!(resp.total_usage.is_none());
        assert!(resp.total_tokens_used.is_none());
    }

    // ── QuotaError ──────────────────────────────────────────────────────

    #[test]
    fn test_quota_error_display() {
        let err = QuotaError::new("kimi", "API key missing");
        assert_eq!(format!("{}", err), "[kimi] API key missing");
    }

    // ── Integration: Kimi balance ───────────────────────────────────────

    #[tokio::test]
    async fn test_fetch_kimi_balance_success() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/v1/users/me/balance"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "code": 0,
                "data": {
                    "available_balance": 100.0,
                    "voucher_balance": 50.0,
                    "cash_balance": 50.0
                },
                "scode": "0x0",
                "status": true
            })))
            .mount(&mock_server)
            .await;

        let old_key = std::env::var("MOONSHOT_API_KEY").ok();
        let old_url = std::env::var("MOONSHOT_BASE_URL").ok();
        std::env::set_var("MOONSHOT_API_KEY", "sk-test");
        std::env::set_var("MOONSHOT_BASE_URL", mock_server.uri());

        let fetcher = QuotaFetcher::new();
        let status = fetcher.fetch_kimi_balance().await;

        reset_env_var("MOONSHOT_API_KEY", old_key);
        reset_env_var("MOONSHOT_BASE_URL", old_url);

        assert!(status.available);
        let data = status.data.unwrap();
        assert!((data.available_balance - 100.0).abs() < 0.001);
        assert_eq!(data.provider, "kimi");
    }

    #[tokio::test]
    async fn test_fetch_kimi_balance_no_api_key() {
        let old_key = std::env::var("MOONSHOT_API_KEY").ok();
        std::env::remove_var("MOONSHOT_API_KEY");

        let fetcher = QuotaFetcher::new();
        let status = fetcher.fetch_kimi_balance().await;

        reset_env_var("MOONSHOT_API_KEY", old_key);

        assert!(!status.available);
        assert!(status
            .error
            .unwrap()
            .contains("MOONSHOT_API_KEY not configured"));
    }

    #[tokio::test]
    async fn test_fetch_kimi_balance_api_error() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/v1/users/me/balance"))
            .respond_with(ResponseTemplate::new(401).set_body_string(
                r#"{"error":{"message":"Invalid API key","type":"invalid_request_error"}}"#,
            ))
            .mount(&mock_server)
            .await;

        let old_key = std::env::var("MOONSHOT_API_KEY").ok();
        let old_url = std::env::var("MOONSHOT_BASE_URL").ok();
        std::env::set_var("MOONSHOT_API_KEY", "sk-bad");
        std::env::set_var("MOONSHOT_BASE_URL", mock_server.uri());

        let fetcher = QuotaFetcher::new();
        let status = fetcher.fetch_kimi_balance().await;

        reset_env_var("MOONSHOT_API_KEY", old_key);
        reset_env_var("MOONSHOT_BASE_URL", old_url);

        assert!(!status.available);
        let err = status.error.unwrap();
        assert!(err.contains("401"));
    }

    #[tokio::test]
    async fn test_fetch_kimi_balance_business_error() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/v1/users/me/balance"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "code": 401,
                "data": {
                    "available_balance": 0.0,
                    "voucher_balance": 0.0,
                    "cash_balance": 0.0
                },
                "scode": "0x1",
                "status": false
            })))
            .mount(&mock_server)
            .await;

        let old_key = std::env::var("MOONSHOT_API_KEY").ok();
        let old_url = std::env::var("MOONSHOT_BASE_URL").ok();
        std::env::set_var("MOONSHOT_API_KEY", "sk-test");
        std::env::set_var("MOONSHOT_BASE_URL", mock_server.uri());

        let fetcher = QuotaFetcher::new();
        let status = fetcher.fetch_kimi_balance().await;

        reset_env_var("MOONSHOT_API_KEY", old_key);
        reset_env_var("MOONSHOT_BASE_URL", old_url);

        assert!(!status.available);
        let err = status.error.unwrap();
        assert!(err.contains("API error code 401"));
    }

    // ── Integration: OpenCode-go quota ──────────────────────────────────

    #[tokio::test]
    async fn test_fetch_opencode_quota_success() {
        let mock_server = MockServer::start().await;

        // Mock subscription endpoint
        Mock::given(method("GET"))
            .and(path("/v1/dashboard/billing/subscription"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "has_payment_method": true,
                "hard_limit_usd": 100.0,
                "plan": {
                    "title": "Go Plan",
                    "id": "opencode-go-monthly"
                }
            })))
            .mount(&mock_server)
            .await;

        // Mock usage endpoint
        Mock::given(method("GET"))
            .and(path("/v1/dashboard/billing/usage"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "object": "list",
                "total_usage": 25.50
            })))
            .mount(&mock_server)
            .await;

        // Set env vars directly since temp_env::with_var doesn't work in async context
        let old_key = std::env::var("OPENCODE_API_KEY").ok();
        let old_url = std::env::var("OPENCODE_BASE_URL").ok();

        std::env::set_var("OPENCODE_API_KEY", "sk-test-opencode");
        std::env::set_var("OPENCODE_BASE_URL", mock_server.uri());

        let fetcher = QuotaFetcher::new();
        let status = fetcher.fetch_opencode_quota().await;

        match old_key {
            Some(k) => std::env::set_var("OPENCODE_API_KEY", k),
            None => std::env::remove_var("OPENCODE_API_KEY"),
        }
        match old_url {
            Some(u) => std::env::set_var("OPENCODE_BASE_URL", u),
            None => std::env::remove_var("OPENCODE_BASE_URL"),
        }

        assert!(status.available);
        let data = status.data.unwrap();
        assert_eq!(data.provider, "opencode-go");
        assert_eq!(data.plan_type, Some("Go Plan".to_string()));
        assert_eq!(data.hard_limit_usd, Some(100.0));
        assert!((data.total_usage_usd.unwrap() - 25.50).abs() < 0.01);
        assert!((data.usage_percent.unwrap() - 25.5).abs() < 0.01);
        assert!((data.remaining_usd.unwrap() - 74.5).abs() < 0.01);
    }

    #[tokio::test]
    async fn test_fetch_opencode_quota_no_api_key() {
        let old_key = std::env::var("OPENCODE_API_KEY").ok();
        let old_auth = std::env::var("OPENCODE_AUTH_PATH").ok();
        std::env::remove_var("OPENCODE_API_KEY");
        std::env::set_var("OPENCODE_AUTH_PATH", "/tmp/nonexistent-opencode-auth.json");

        let fetcher = QuotaFetcher::new();
        let status = fetcher.fetch_opencode_quota().await;

        reset_env_var("OPENCODE_API_KEY", old_key);
        reset_env_var("OPENCODE_AUTH_PATH", old_auth);

        assert!(!status.available);
        assert!(status
            .error
            .unwrap()
            .contains("opencode-go API key not found"));
    }

    #[tokio::test]
    async fn test_fetch_opencode_quota_subscription_error() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/v1/dashboard/billing/subscription"))
            .respond_with(
                ResponseTemplate::new(401)
                    .set_body_string(r#"{"error":{"message":"Invalid API key"}}"#),
            )
            .mount(&mock_server)
            .await;

        let old_key = std::env::var("OPENCODE_API_KEY").ok();
        let old_url = std::env::var("OPENCODE_BASE_URL").ok();

        std::env::set_var("OPENCODE_API_KEY", "sk-bad");
        std::env::set_var("OPENCODE_BASE_URL", mock_server.uri());

        let fetcher = QuotaFetcher::new();
        let status = fetcher.fetch_opencode_quota().await;

        match old_key {
            Some(k) => std::env::set_var("OPENCODE_API_KEY", k),
            None => std::env::remove_var("OPENCODE_API_KEY"),
        }
        match old_url {
            Some(u) => std::env::set_var("OPENCODE_BASE_URL", u),
            None => std::env::remove_var("OPENCODE_BASE_URL"),
        }

        assert!(!status.available);
        assert!(status.error.unwrap().contains("401"));
    }

    #[tokio::test]
    async fn test_fetch_opencode_quota_usage_unavailable() {
        let mock_server = MockServer::start().await;

        // Subscription endpoint works
        Mock::given(method("GET"))
            .and(path("/v1/dashboard/billing/subscription"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "hard_limit_usd": 100.0,
                "plan": { "title": "Go Plan" }
            })))
            .mount(&mock_server)
            .await;

        // Usage endpoint returns error (some providers don't expose it)
        Mock::given(method("GET"))
            .and(path("/v1/dashboard/billing/usage"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&mock_server)
            .await;

        let old_key = std::env::var("OPENCODE_API_KEY").ok();
        let old_url = std::env::var("OPENCODE_BASE_URL").ok();

        std::env::set_var("OPENCODE_API_KEY", "sk-test");
        std::env::set_var("OPENCODE_BASE_URL", mock_server.uri());

        let fetcher = QuotaFetcher::new();
        let status = fetcher.fetch_opencode_quota().await;

        match old_key {
            Some(k) => std::env::set_var("OPENCODE_API_KEY", k),
            None => std::env::remove_var("OPENCODE_API_KEY"),
        }
        match old_url {
            Some(u) => std::env::set_var("OPENCODE_BASE_URL", u),
            None => std::env::remove_var("OPENCODE_BASE_URL"),
        }

        // Should still succeed with subscription info even if usage endpoint fails
        assert!(status.available);
        let data = status.data.unwrap();
        assert_eq!(data.plan_type, Some("Go Plan".to_string()));
        assert_eq!(data.hard_limit_usd, Some(100.0));
        assert!(data.total_usage_usd.is_none()); // usage endpoint failed
    }
}
