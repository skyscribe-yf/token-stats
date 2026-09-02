//! CodeBuddy (codebuddy.cn) subscription quota fetching.
//!
//! Mirrors the user-center "plans-usage" page (`https://www.codebuddy.cn/profile/plans-usage`):
//! - `POST /billing/meter/get-user-resource-summary` — per-package cycle totals
//! - `POST /billing/meter/get-user-resource` — package names / subscription cycles
//!
//! Auth requires both `session` and `session_2` cookies of `www.codebuddy.cn`,
//! provided via `CODEBUDDY_SESSION_COOKIE` and `CODEBUDDY_SESSION_COOKIE_2`
//! (cookie values only, without the `session=` / `session_2=` prefix).

use super::types::{CodeBuddyPackage, CodeBuddyQuotaData, CodeBuddyQuotaStatus};
use super::deserialize_flexible_number;
use reqwest::Client;
use serde::Deserialize;

const BASE_URL: &str = "https://www.codebuddy.cn";

fn get_session_cookie() -> Option<String> {
    std::env::var("CODEBUDDY_SESSION_COOKIE").ok().filter(|s| !s.is_empty())
}

fn get_session_cookie_2() -> Option<String> {
    std::env::var("CODEBUDDY_SESSION_COOKIE_2").ok().filter(|s| !s.is_empty())
}

fn unavailable(error: Option<String>) -> CodeBuddyQuotaStatus {
    CodeBuddyQuotaStatus {
        available: false,
        data: None,
        error,
    }
}

// ─── API response shapes (PascalCase fields) ────────────────────────────────

#[derive(Deserialize)]
struct MeterEnvelope<T> {
    #[serde(default)]
    code: i64,
    #[serde(default)]
    msg: Option<String>,
    data: Option<T>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct SummaryData {
    #[serde(default)]
    packages: Vec<SummaryPackage>,
    subscription_package_code: Option<String>,
    is_paid_user: Option<bool>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct SummaryPackage {
    package_code: String,
    #[serde(deserialize_with = "deserialize_flexible_number", default)]
    cycle_total_capacity: f64,
    #[serde(deserialize_with = "deserialize_flexible_number", default)]
    cycle_remain_capacity: f64,
    #[serde(deserialize_with = "deserialize_flexible_number", default)]
    cycle_used_capacity: f64,
    capacity_unit: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct ResourceData {
    response: Option<ResourceResponse>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct ResourceResponse {
    data: Option<ResourceInner>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct ResourceInner {
    #[serde(default)]
    accounts: Vec<ResourceAccount>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct ResourceAccount {
    package_code: String,
    package_name: Option<String>,
    cycle_start_time: Option<String>,
    cycle_end_time: Option<String>,
    #[serde(default)]
    deduction_end_time: Option<i64>,
}

// ─── Fetcher ────────────────────────────────────────────────────────────────

async fn post_meter<T: for<'de> Deserialize<'de>>(
    client: &Client,
    cookie_header: &str,
    path: &str,
    body: serde_json::Value,
) -> Result<T, String> {
    let resp = client
        .post(format!("{}{}", BASE_URL, path))
        .header("Cookie", cookie_header)
        .header("Content-Type", "application/json")
        .header("Origin", BASE_URL)
        .header("Referer", format!("{}/profile/plans-usage", BASE_URL))
        .header(
            "User-Agent",
            // NOTE: the codebuddy.cn edge WAF rejects outdated Chrome UA
            // versions (e.g. Chrome/126 → HTTP 401 HTML challenge); keep
            // this reasonably recent.
            "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/152.0.0.0 Safari/537.36",
        )
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Request failed: {}", e))?;

    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!("HTTP {}: {}", status, super::truncate_error_body(&text)));
    }

    let envelope: MeterEnvelope<T> =
        serde_json::from_str(&text).map_err(|e| format!("Failed to parse response: {}", e))?;
    if envelope.code != 0 {
        return Err(format!(
            "API error {}: {}",
            envelope.code,
            envelope.msg.unwrap_or_else(|| "Unknown error".to_string())
        ));
    }
    envelope.data.ok_or_else(|| "No data in response".to_string())
}

/// Fetch CodeBuddy (codebuddy.cn) subscription / package quota.
pub async fn fetch_codebuddy_quota(client: &Client) -> CodeBuddyQuotaStatus {
    let session = match get_session_cookie() {
        Some(v) => v,
        None => {
            return unavailable(Some("CODEBUDDY_SESSION_COOKIE not set".to_string()));
        }
    };
    let session_2 = match get_session_cookie_2() {
        Some(v) => v,
        None => {
            return unavailable(Some("CODEBUDDY_SESSION_COOKIE_2 not set".to_string()));
        }
    };
    let cookie_header = format!("session={}; session_2={}", session, session_2);

    let (summary_res, resource_res) = tokio::join!(
        post_meter::<SummaryData>(
            client,
            &cookie_header,
            "/billing/meter/get-user-resource-summary",
            serde_json::json!({}),
        ),
        post_meter::<ResourceData>(
            client,
            &cookie_header,
            "/billing/meter/get-user-resource",
            serde_json::json!({"Limit": 50, "Offset": 0}),
        ),
    );

    let summary = match summary_res {
        Ok(s) => s,
        Err(e) => return unavailable(Some(e)),
    };

    // Latest account entry per package (name / cycle info); resource is
    // best-effort — summary numbers still display when it fails.
    let accounts = match resource_res {
        Ok(r) => r.response.and_then(|x| x.data).map(|d| d.accounts).unwrap_or_default(),
        Err(_) => Vec::new(),
    };
    let latest_account = |package_code: &str| -> Option<&ResourceAccount> {
        accounts
            .iter()
            .filter(|a| a.package_code == package_code)
            .max_by_key(|a| a.deduction_end_time.unwrap_or(0))
    };

    let subscription_code = summary.subscription_package_code.clone().unwrap_or_default();
    let mut packages: Vec<CodeBuddyPackage> = summary
        .packages
        .into_iter()
        .map(|p| {
            let account = latest_account(&p.package_code);
            CodeBuddyPackage {
                package_code: p.package_code.clone(),
                package_name: account
                    .and_then(|a| a.package_name.clone())
                    .unwrap_or_else(|| p.package_code.clone()),
                is_subscription: p.package_code == subscription_code,
                unit: p.capacity_unit.unwrap_or_else(|| "credits".to_string()),
                total: p.cycle_total_capacity,
                used: p.cycle_used_capacity,
                remain: p.cycle_remain_capacity,
                cycle_start: account.and_then(|a| a.cycle_start_time.clone()),
                cycle_end: account.and_then(|a| a.cycle_end_time.clone()),
            }
        })
        .collect();

    // Subscription package first, then by remaining credits desc.
    packages.sort_by(|a, b| {
        b.is_subscription
            .cmp(&a.is_subscription)
            .then(b.remain.partial_cmp(&a.remain).unwrap_or(std::cmp::Ordering::Equal))
    });

    CodeBuddyQuotaStatus {
        available: true,
        data: Some(CodeBuddyQuotaData {
            is_paid_user: summary.is_paid_user.unwrap_or(false),
            packages,
        }),
        error: None,
    }
}
