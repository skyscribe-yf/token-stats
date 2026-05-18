//! OpenCode-go provider integration.
//!
//! Handles API key resolution, billing API calls to OpenCode-go's
//! OpenAI-compatible billing endpoints, and response parsing.

use super::types::*;
use chrono::Datelike;
use std::path::PathBuf;

// ─── Auth helpers ────────────────────────────────────────────────────────────

/// Path to the OpenCode auth.json file.
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

/// Read opencode-go API key from env var or local auth.json.
pub fn get_opencode_api_key() -> Option<String> {
    if let Ok(key) = std::env::var("OPENCODE_API_KEY") {
        if !key.is_empty() {
            return Some(key);
        }
    }

    let path = get_opencode_auth_path();
    if !path.exists() {
        return None;
    }

    let content = std::fs::read_to_string(path).ok()?;
    let auth: serde_json::Value = serde_json::from_str(&content).ok()?;

    auth.get("opencode-go")
        .and_then(|entry| entry.get("key"))
        .and_then(|k| k.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

/// OpenCode base URL for API calls.
pub fn get_opencode_base_url() -> String {
    std::env::var("OPENCODE_BASE_URL").unwrap_or_else(|_| "https://opencode.ai/zen/v1".to_string())
}

/// Build the OpenCode-go workspace dashboard URL from the workspace ID env var.
pub fn get_opencode_workspace_url() -> Option<String> {
    std::env::var("OPENCODE_GO_WORKSPACE_ID")
        .ok()
        .filter(|id| !id.is_empty())
        .map(|id| format!("https://opencode.ai/workspace/{}/go", id))
}

// ─── OpenCode Quota Fetching ─────────────────────────────────────────────────

/// Fetch OpenCode-go subscription and usage, then build the status DTO.
pub async fn fetch_opencode_quota(client: &reqwest::Client) -> OpenCodeQuotaStatus {
    let api_key = match get_opencode_api_key() {
        Some(k) => k,
        None => return opencode_no_auth_status(),
    };

    let base_url = get_opencode_base_url();
    let base = base_url.trim_end_matches('/');

    // Step 1: Fetch subscription info
    let sub_url = format!("{}/v1/dashboard/billing/subscription", base);
    let subscription = match fetch_subscription(client, &api_key, &sub_url).await {
        Ok(sub) => sub,
        Err(status) => return status,
    };

    let plan_type = subscription
        .plan
        .as_ref()
        .and_then(|p| p.title.clone())
        .or_else(|| subscription.plan.as_ref().and_then(|p| p.id.clone()));
    let hard_limit_usd = subscription.hard_limit_usd;

    // Step 2: Fetch current usage
    let (total_usage_usd, usage_percent) =
        fetch_opencode_usage(client, &api_key, base, hard_limit_usd).await;

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
            workspace_url: None,
        }),
        error: None,
    }
}

fn opencode_no_auth_status() -> OpenCodeQuotaStatus {
    if let Some(url) = get_opencode_workspace_url() {
        OpenCodeQuotaStatus {
            available: false,
            data: Some(QuotaOpenCode {
                provider: "opencode-go".to_string(),
                plan_type: None,
                hard_limit_usd: None,
                total_usage_usd: None,
                usage_percent: None,
                remaining_usd: None,
                workspace_url: Some(url),
            }),
            error: Some("API key not found. Visit workspace to check usage.".to_string()),
        }
    } else {
        OpenCodeQuotaStatus {
            available: false,
            data: None,
            error: Some(
                "opencode-go API key not found in auth.json or OPENCODE_API_KEY".to_string(),
            ),
        }
    }
}

async fn fetch_subscription(
    client: &reqwest::Client,
    api_key: &str,
    url: &str,
) -> Result<OpenAiSubscriptionResponse, OpenCodeQuotaStatus> {
    let response = match client
        .get(url)
        .header("Authorization", format!("Bearer {}", api_key))
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            return Err(OpenCodeQuotaStatus {
                available: false,
                data: None,
                error: Some(format!("Subscription request failed: {}", e)),
            });
        }
    };

    if !response.status().is_success() {
        let status = response.status();
        if status.as_u16() == 404 {
            let workspace_url = get_opencode_workspace_url();
            return Err(OpenCodeQuotaStatus {
                available: false,
                data: Some(QuotaOpenCode {
                    provider: "opencode-go".to_string(),
                    plan_type: None,
                    hard_limit_usd: None,
                    total_usage_usd: None,
                    usage_percent: None,
                    remaining_usd: None,
                    workspace_url,
                }),
                error: Some("Billing API not available. Visit workspace.".to_string()),
            });
        }
        let body = response.text().await.unwrap_or_default();
        return Err(OpenCodeQuotaStatus {
            available: false,
            data: None,
            error: Some(format!(
                "Subscription API returned {}: {}",
                status,
                super::truncate_error_body(&body)
            )),
        });
    }

    response.json().await.map_err(|e| OpenCodeQuotaStatus {
        available: false,
        data: None,
        error: Some(format!("Failed to parse subscription response: {}", e)),
    })
}

async fn fetch_opencode_usage(
    client: &reqwest::Client,
    api_key: &str,
    base: &str,
    hard_limit_usd: Option<f64>,
) -> (Option<f64>, Option<f64>) {
    let now = chrono::Utc::now();
    let start_of_month = chrono::NaiveDate::from_ymd_opt(now.year(), now.month(), 1)
        .unwrap_or_else(|| now.date_naive());
    let usage_url = format!(
        "{}/v1/dashboard/billing/usage?start_date={}&end_date={}",
        base,
        start_of_month.format("%Y-%m-%d"),
        now.format("%Y-%m-%d")
    );

    let response = match client
        .get(&usage_url)
        .header("Authorization", format!("Bearer {}", api_key))
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => r,
        _ => return (None, None),
    };

    let usage: OpenAiUsageResponse = match response.json().await {
        Ok(u) => u,
        Err(_) => return (None, None),
    };

    let usage_percent = hard_limit_usd.and_then(|limit| {
        if limit > 0.0 {
            usage
                .total_usage
                .map(|used| (used / limit * 100.0).min(100.0))
        } else {
            None
        }
    });

    (usage.total_usage, usage_percent)
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    static ENV_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn reset_env_var(name: &str, old: Option<String>) {
        match old {
            Some(v) => std::env::set_var(name, v),
            None => std::env::remove_var(name),
        }
    }

    #[test]
    fn test_get_opencode_base_url_default() {
        temp_env::with_var("OPENCODE_BASE_URL", None::<&str>, || {
            assert_eq!(get_opencode_base_url(), "https://opencode.ai/zen/v1");
        });
    }

    #[test]
    fn test_get_opencode_workspace_url() {
        temp_env::with_var("OPENCODE_GO_WORKSPACE_ID", Some("wrk_TEST123"), || {
            assert_eq!(
                get_opencode_workspace_url(),
                Some("https://opencode.ai/workspace/wrk_TEST123/go".to_string())
            );
        });
    }

    #[test]
    fn test_get_opencode_workspace_url_unset() {
        temp_env::with_var("OPENCODE_GO_WORKSPACE_ID", None::<&str>, || {
            assert_eq!(get_opencode_workspace_url(), None);
        });
    }

    #[test]
    fn test_parse_openai_subscription() {
        let json = r#"{
            "has_payment_method": true,
            "hard_limit_usd": 100.0,
            "plan": {"title": "Go Plan", "id": "opencode-go-monthly"}
        }"#;
        let resp: OpenAiSubscriptionResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.has_payment_method, Some(true));
        assert_eq!(resp.hard_limit_usd, Some(100.0));
    }

    #[test]
    fn test_parse_openai_subscription_empty() {
        let json = "{}";
        let resp: OpenAiSubscriptionResponse = serde_json::from_str(json).unwrap();
        assert!(resp.hard_limit_usd.is_none());
    }

    #[test]
    fn test_parse_openai_usage() {
        let json = r#"{"object": "list", "total_usage": 25.50}"#;
        let resp: OpenAiUsageResponse = serde_json::from_str(json).unwrap();
        assert!((resp.total_usage.unwrap() - 25.50).abs() < 0.01);
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn test_fetch_opencode_quota_success() {
        let _lock = ENV_MUTEX.lock().unwrap();
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/v1/dashboard/billing/subscription"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "hard_limit_usd": 100.0,
                "plan": {"title": "Go Plan"}
            })))
            .mount(&mock_server)
            .await;

        Mock::given(method("GET"))
            .and(path("/v1/dashboard/billing/usage"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "total_usage": 25.50
            })))
            .mount(&mock_server)
            .await;

        let old_key = std::env::var("OPENCODE_API_KEY").ok();
        let old_url = std::env::var("OPENCODE_BASE_URL").ok();
        std::env::set_var("OPENCODE_API_KEY", "sk-test-opencode");
        std::env::set_var("OPENCODE_BASE_URL", mock_server.uri());

        let client = reqwest::Client::new();
        let status = fetch_opencode_quota(&client).await;

        reset_env_var("OPENCODE_API_KEY", old_key);
        reset_env_var("OPENCODE_BASE_URL", old_url);

        assert!(status.available);
        let data = status.data.unwrap();
        assert_eq!(data.plan_type, Some("Go Plan".to_string()));
        assert_eq!(data.hard_limit_usd, Some(100.0));
        assert!((data.total_usage_usd.unwrap() - 25.50).abs() < 0.01);
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn test_fetch_opencode_quota_404() {
        let _lock = ENV_MUTEX.lock().unwrap();
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/v1/dashboard/billing/subscription"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&mock_server)
            .await;

        let old_key = std::env::var("OPENCODE_API_KEY").ok();
        let old_url = std::env::var("OPENCODE_BASE_URL").ok();
        let old_ws = std::env::var("OPENCODE_GO_WORKSPACE_ID").ok();
        std::env::set_var("OPENCODE_API_KEY", "sk-test");
        std::env::set_var("OPENCODE_BASE_URL", mock_server.uri());
        std::env::set_var("OPENCODE_GO_WORKSPACE_ID", "wrk_TEST404");

        let client = reqwest::Client::new();
        let status = fetch_opencode_quota(&client).await;

        reset_env_var("OPENCODE_API_KEY", old_key);
        reset_env_var("OPENCODE_BASE_URL", old_url);
        reset_env_var("OPENCODE_GO_WORKSPACE_ID", old_ws);

        assert!(!status.available);
        let data = status.data.unwrap();
        assert_eq!(
            data.workspace_url,
            Some("https://opencode.ai/workspace/wrk_TEST404/go".to_string())
        );
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn test_fetch_opencode_quota_no_key() {
        let _lock = ENV_MUTEX.lock().unwrap();
        let old_key = std::env::var("OPENCODE_API_KEY").ok();
        let old_auth = std::env::var("OPENCODE_AUTH_PATH").ok();
        let old_ws = std::env::var("OPENCODE_GO_WORKSPACE_ID").ok();
        std::env::remove_var("OPENCODE_API_KEY");
        std::env::set_var("OPENCODE_AUTH_PATH", "/tmp/nonexistent-auth.json");
        std::env::remove_var("OPENCODE_GO_WORKSPACE_ID");

        let client = reqwest::Client::new();
        let status = fetch_opencode_quota(&client).await;

        reset_env_var("OPENCODE_API_KEY", old_key);
        reset_env_var("OPENCODE_AUTH_PATH", old_auth);
        reset_env_var("OPENCODE_GO_WORKSPACE_ID", old_ws);

        assert!(!status.available);
        assert!(status.data.is_none());
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn test_fetch_opencode_quota_usage_unavailable() {
        let _lock = ENV_MUTEX.lock().unwrap();
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/v1/dashboard/billing/subscription"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "hard_limit_usd": 100.0,
                "plan": {"title": "Go Plan"}
            })))
            .mount(&mock_server)
            .await;

        Mock::given(method("GET"))
            .and(path("/v1/dashboard/billing/usage"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&mock_server)
            .await;

        let old_key = std::env::var("OPENCODE_API_KEY").ok();
        let old_url = std::env::var("OPENCODE_BASE_URL").ok();
        std::env::set_var("OPENCODE_API_KEY", "sk-test");
        std::env::set_var("OPENCODE_BASE_URL", mock_server.uri());

        let client = reqwest::Client::new();
        let status = fetch_opencode_quota(&client).await;

        reset_env_var("OPENCODE_API_KEY", old_key);
        reset_env_var("OPENCODE_BASE_URL", old_url);

        assert!(status.available);
        let data = status.data.unwrap();
        assert_eq!(data.plan_type, Some("Go Plan".to_string()));
        assert!(data.total_usage_usd.is_none());
    }
}
