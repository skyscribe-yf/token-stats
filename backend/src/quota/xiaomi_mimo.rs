//! Xiaomi MiMo TP quota fetching.
//!
//! Fetches token plan usage from `https://platform.xiaomimimo.com/api/v1/tokenPlan/usage`
//! using cookies from environment variables.

use super::types::{XiaomiMiMoQuotaStatus, XiaomiMiMoUsageEntry};
use reqwest::Client;

/// Get the service token from environment variable.
fn get_service_token() -> Option<String> {
    std::env::var("XIAOMI_MIMO_SERVICE_TOKEN").ok()
}

/// Get the user ID from environment variable.
fn get_user_id() -> Option<String> {
    std::env::var("XIAOMI_MIMO_USER_ID").ok()
}

/// Fetch Xiaomi MiMo TP quota status.
pub async fn fetch_xiaomi_mimo_quota(client: &Client) -> XiaomiMiMoQuotaStatus {
    let service_token = match get_service_token() {
        Some(t) => t,
        None => {
            return XiaomiMiMoQuotaStatus {
                available: false,
                data: None,
                error: Some("XIAOMI_MIMO_SERVICE_TOKEN not set".to_string()),
            };
        }
    };

    let user_id = match get_user_id() {
        Some(u) => u,
        None => {
            return XiaomiMiMoQuotaStatus {
                available: false,
                data: None,
                error: Some("XIAOMI_MIMO_USER_ID not set".to_string()),
            };
        }
    };

    let cookie = format!(
        "api-platform_serviceToken={}; userId={}",
        service_token, user_id
    );

    let url = "https://platform.xiaomimimo.com/api/v1/tokenPlan/usage";

    let resp = match client
        .get(url)
        .header("Cookie", cookie)
        .header("User-Agent", "token-stats-dashboard/1.0")
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            return XiaomiMiMoQuotaStatus {
                available: false,
                data: None,
                error: Some(format!("Request failed: {}", e)),
            };
        }
    };

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return XiaomiMiMoQuotaStatus {
            available: false,
            data: None,
            error: Some(format!("HTTP {}: {}", status, body)),
        };
    }

    let body: serde_json::Value = match resp.json().await {
        Ok(b) => b,
        Err(e) => {
            return XiaomiMiMoQuotaStatus {
                available: false,
                data: None,
                error: Some(format!("Failed to parse response: {}", e)),
            };
        }
    };

    // Check for API error
    if let Some(code) = body.get("code").and_then(|c| c.as_i64()) {
        if code != 0 {
            let msg = body
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("Unknown error");
            return XiaomiMiMoQuotaStatus {
                available: false,
                data: None,
                error: Some(format!("API error {}: {}", code, msg)),
            };
        }
    }

    let data = match body.get("data") {
        Some(d) => d,
        None => {
            return XiaomiMiMoQuotaStatus {
                available: false,
                data: None,
                error: Some("No data in response".to_string()),
            };
        }
    };

    // Parse usage entries
    let usage = data.get("usage");
    let month_usage = data.get("monthUsage");

    let mut entries = Vec::new();

    // Parse overall usage
    if let Some(usage) = usage {
        if let Some(items) = usage.get("items").and_then(|i| i.as_array()) {
            for item in items {
                let name = item
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or("")
                    .to_string();
                let used = item
                    .get("used")
                    .and_then(|u| u.as_i64())
                    .unwrap_or(0);
                let limit = item
                    .get("limit")
                    .and_then(|l| l.as_i64())
                    .unwrap_or(0);
                let percent = item
                    .get("percent")
                    .and_then(|p| p.as_f64())
                    .unwrap_or(0.0);

                if limit > 0 {
                    entries.push(XiaomiMiMoUsageEntry {
                        name,
                        used,
                        limit,
                        percent,
                    });
                }
            }
        }
    }

    // Parse monthly usage
    let mut month_percent = 0.0;
    if let Some(month_usage) = month_usage {
        month_percent = month_usage
            .get("percent")
            .and_then(|p| p.as_f64())
            .unwrap_or(0.0);
    }

    // Get plan details (we'll fetch this separately)
    let plan_detail = fetch_plan_detail(client, &service_token, &user_id).await;

    XiaomiMiMoQuotaStatus {
        available: true,
        data: Some(super::types::XiaomiMiMoQuotaData {
            entries,
            month_percent,
            plan_name: plan_detail.as_ref().map(|p| p.0.clone()).unwrap_or_default(),
            plan_code: plan_detail.as_ref().map(|p| p.1.clone()).unwrap_or_default(),
            current_period_end: plan_detail.as_ref().and_then(|p| p.2.clone()),
            expired: plan_detail.as_ref().map(|p| p.3).unwrap_or(false),
            enable_auto_renew: plan_detail.as_ref().map(|p| p.4).unwrap_or(false),
        }),
        error: None,
    }
}

/// Fetch plan detail information.
async fn fetch_plan_detail(
    client: &Client,
    service_token: &str,
    user_id: &str,
) -> Option<(String, String, Option<String>, bool, bool)> {
    let cookie = format!(
        "api-platform_serviceToken={}; userId={}",
        service_token, user_id
    );

    let url = "https://platform.xiaomimimo.com/api/v1/tokenPlan/detail";

    let resp = client
        .get(url)
        .header("Cookie", cookie)
        .header("User-Agent", "token-stats-dashboard/1.0")
        .send()
        .await
        .ok()?;

    if !resp.status().is_success() {
        return None;
    }

    let body: serde_json::Value = resp.json().await.ok()?;

    if body.get("code").and_then(|c| c.as_i64()) != Some(0) {
        return None;
    }

    let data = body.get("data")?;

    let plan_name = data
        .get("planName")
        .and_then(|n| n.as_str())
        .unwrap_or("")
        .to_string();
    let plan_code = data
        .get("planCode")
        .and_then(|c| c.as_str())
        .unwrap_or("")
        .to_string();
    let current_period_end = data
        .get("currentPeriodEnd")
        .and_then(|e| e.as_str())
        .map(|s| s.to_string());
    let expired = data
        .get("expired")
        .and_then(|e| e.as_bool())
        .unwrap_or(false);
    let enable_auto_renew = data
        .get("enableAutoRenew")
        .and_then(|a| a.as_bool())
        .unwrap_or(false);

    Some((plan_name, plan_code, current_period_end, expired, enable_auto_renew))
}
