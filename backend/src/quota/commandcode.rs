//! CommandCode provider integration.
//!
//! Fetches subscription/quota data from the CommandCode platform API
//! (https://api.commandcode.ai) using the session token cookie for auth.
//!
//! Three endpoints are called:
//! - `/internal/billing/subscriptions` — plan, status, renewal date
//! - `/internal/billing/credits` — remaining monthly credits, purchased credits
//! - `/internal/usage/summary` — consumed usage so far this month

use super::types::*;
use reqwest::Client;
use serde::Deserialize;
use tracing::{info, warn};

// ─── Constants ───────────────────────────────────────────────────────────────

const COMMANDCODE_API_BASE: &str = "https://api.commandcode.ai";
const COMMANDCODE_TIMEOUT_SECS: u64 = 15;

// ─── Auth helpers ────────────────────────────────────────────────────────────

/// Read `COMMANDCODE_SESSION_TOKEN` from environment.
pub fn get_session_token() -> Option<String> {
    std::env::var("COMMANDCODE_SESSION_TOKEN")
        .ok()
        .filter(|t| !t.is_empty())
}

// ─── API response types ──────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct CreditsResponse {
    credits: Option<CreditsData>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreditsData {
    #[serde(default)]
    monthly_credits: f64,
    #[serde(default)]
    purchased_credits: f64,
    #[serde(default)]
    premium_monthly_credits: f64,
    #[serde(default)]
    opensource_monthly_credits: f64,
}

#[derive(Debug, Deserialize)]
struct SubscriptionResponse {
    data: Option<SubscriptionData>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SubscriptionData {
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    plan_id: Option<String>,
    #[serde(default)]
    current_period_end: Option<String>,
    #[serde(default)]
    cancel_at_period_end: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UsageSummaryResponse {
    #[serde(default)]
    total_cost: f64,
    #[serde(default)]
    total_count: i64,
    #[serde(default)]
    total_tokens: i64,
    #[serde(default)]
    total_tokens_in: i64,
    #[serde(default)]
    total_tokens_out: i64,
}

// ─── Fetch ───────────────────────────────────────────────────────────────────

pub async fn fetch_commandcode_quota(client: &Client) -> CommandCodeQuotaStatus {
    let token = match get_session_token() {
        Some(t) => t,
        None => {
            warn!("COMMANDCODE_SESSION_TOKEN not set");
            return CommandCodeQuotaStatus {
                available: false,
                data: None,
                error: Some("COMMANDCODE_SESSION_TOKEN not set".to_string()),
            };
        }
    };

    let cookie = format!(
        "__Secure-commandcode_prod_.session_token={}",
        token
    );

    // Run all three requests in parallel
    let (credits_result, subscription_result, usage_result) = tokio::join!(
        fetch_credits(client, &cookie),
        fetch_subscription(client, &cookie),
        fetch_usage_summary(client, &cookie),
    );

    // If all failed, treat as unavailable
    if credits_result.is_none() && subscription_result.is_none() {
        return CommandCodeQuotaStatus {
            available: false,
            data: None,
            error: Some("All CommandCode API requests failed".to_string()),
        };
    }

    let credits = credits_result;
    let sub = subscription_result;
    let usage = usage_result;

    // Compute monthly credits total: remaining + already used
    let monthly_used = usage.as_ref().map_or(0.0, |u| u.total_cost);
    let monthly_remaining = credits.as_ref().map_or(0.0, |c| c.monthly_credits);
    let monthly_total = if monthly_remaining > 0.0 || monthly_used > 0.0 {
        Some(monthly_remaining + monthly_used)
    } else {
        None
    };

    let plan_name = sub
        .as_ref()
        .and_then(|s| s.plan_id.as_deref())
        .map(plan_id_to_label)
        .unwrap_or("N/A");

    let data = CommandCodeQuotaData {
        plan_name: plan_name.to_string(),
        subscription_status: sub
            .as_ref()
            .and_then(|s| s.status.as_deref())
            .map(|s| s.to_string())
            .unwrap_or_else(|| "unknown".to_string()),
        cancel_at_period_end: sub.as_ref().and_then(|s| s.cancel_at_period_end),
        monthly_credits_total: monthly_total,
        monthly_credits_used: monthly_used,
        monthly_credits_remaining: monthly_remaining,
        purchased_credits: credits.as_ref().map_or(0.0, |c| c.purchased_credits),
        premium_monthly_credits: credits
            .as_ref()
            .map_or(0.0, |c| c.premium_monthly_credits),
        opensource_monthly_credits: credits
            .as_ref()
            .map_or(0.0, |c| c.opensource_monthly_credits),
        current_period_end: sub
            .as_ref()
            .and_then(|s| s.current_period_end.clone()),
        total_requests: usage.as_ref().map_or(0, |u| u.total_count),
        total_tokens: usage.as_ref().map_or(0, |u| u.total_tokens),
        total_tokens_in: usage.as_ref().map_or(0, |u| u.total_tokens_in),
        total_tokens_out: usage.as_ref().map_or(0, |u| u.total_tokens_out),
    };

    info!(
        "CommandCode quota fetched: plan={}, status={}, monthly used={:.4}/{:.4}",
        data.plan_name,
        data.subscription_status,
        data.monthly_credits_used,
        data.monthly_credits_total.unwrap_or(0.0),
    );

    CommandCodeQuotaStatus {
        available: true,
        data: Some(data),
        error: None,
    }
}

async fn fetch_credits(client: &Client, cookie: &str) -> Option<CreditsData> {
    let url = format!("{}/internal/billing/credits", COMMANDCODE_API_BASE);
    match client
        .get(&url)
        .header("Cookie", cookie)
        .timeout(std::time::Duration::from_secs(COMMANDCODE_TIMEOUT_SECS))
        .send()
        .await
    {
        Ok(resp) => {
            if !resp.status().is_success() {
                warn!("CommandCode credits API returned {}", resp.status());
                return None;
            }
            match resp.json::<CreditsResponse>().await {
                Ok(data) => data.credits,
                Err(e) => {
                    warn!("Failed to parse CommandCode credits: {}", e);
                    None
                }
            }
        }
        Err(e) => {
            warn!("CommandCode credits fetch failed: {}", e);
            None
        }
    }
}

async fn fetch_subscription(client: &Client, cookie: &str) -> Option<SubscriptionData> {
    let url = format!("{}/internal/billing/subscriptions", COMMANDCODE_API_BASE);
    match client
        .get(&url)
        .header("Cookie", cookie)
        .timeout(std::time::Duration::from_secs(COMMANDCODE_TIMEOUT_SECS))
        .send()
        .await
    {
        Ok(resp) => {
            if !resp.status().is_success() {
                warn!("CommandCode subscription API returned {}", resp.status());
                return None;
            }
            match resp.json::<SubscriptionResponse>().await {
                Ok(data) => data.data,
                Err(e) => {
                    warn!("Failed to parse CommandCode subscription: {}", e);
                    None
                }
            }
        }
        Err(e) => {
            warn!("CommandCode subscription fetch failed: {}", e);
            None
        }
    }
}

async fn fetch_usage_summary(client: &Client, cookie: &str) -> Option<UsageSummaryResponse> {
    let url = format!("{}/internal/usage/summary", COMMANDCODE_API_BASE);
    match client
        .get(&url)
        .header("Cookie", cookie)
        .timeout(std::time::Duration::from_secs(COMMANDCODE_TIMEOUT_SECS))
        .send()
        .await
    {
        Ok(resp) => {
            if !resp.status().is_success() {
                warn!("CommandCode usage summary API returned {}", resp.status());
                return None;
            }
            match resp.json::<UsageSummaryResponse>().await {
                Ok(data) => Some(data),
                Err(e) => {
                    warn!("Failed to parse CommandCode usage summary: {}", e);
                    None
                }
            }
        }
        Err(e) => {
            warn!("CommandCode usage summary fetch failed: {}", e);
            None
        }
    }
}

fn plan_id_to_label(plan_id: &str) -> &str {
    match plan_id {
        "individual-go" => "Individual Go",
        "individual-pro" => "Individual Pro",
        "org-go" => "Organization Go",
        "org-pro" => "Organization Pro",
        "free" => "Free",
        _ => plan_id,
    }
}
