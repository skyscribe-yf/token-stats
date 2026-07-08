//! Meituan LongCat quota fetcher.
//!
//! Fetches token resource pack balance and usage from the LongCat API platform.
//! API endpoints (all POST with JSON body):
//!   - /api/pay/commercial/entitlements/token-packs/list
//!   - /api/pay/quota/metering/token-usage/overview
//!
//! Authentication: `MEITUAN_AUTH_COOKIE` env var containing the `passport_token_key` value.

use super::types::*;
use reqwest::Client;
use serde::Deserialize;
use tracing::{info, warn};

const LONGCAT_BASE_URL: &str = "https://longcat.chat";
const MEITUAN_TIMEOUT_SECS: u64 = 15;

// ─── Auth ─────────────────────────────────────────────────────────────────────

/// Read `MEITUAN_AUTH_COOKIE` from environment.
/// Should contain the `passport_token_key` value for LongCat platform auth.
pub fn get_auth_cookie() -> Option<String> {
    std::env::var("MEITUAN_AUTH_COOKIE")
        .ok()
        .filter(|c| !c.is_empty())
}

// ─── API response types ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TokenPacksResponse {
    #[serde(default)]
    code: i64,
    #[serde(default)]
    msg: String,
    data: Option<TokenPacksData>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TokenPacksData {
    #[serde(default)]
    items: Vec<TokenPackItem>,
    #[serde(default)]
    active_count: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TokenPackItem {
    #[serde(default)]
    package_name: String,
    #[serde(default)]
    source_type_text: String,
    #[serde(default)]
    source_type_code: i64,
    #[serde(default)]
    status_text: String,
    #[serde(default)]
    status_code: i64,
    #[serde(default)]
    total_token_amount: i64,
    #[serde(default)]
    used_token_amount: i64,
    #[serde(default)]
    remain_token_amount: i64,
    #[serde(default)]
    usage_percent: i64,
    #[serde(default)]
    valid_start_time: String,
    #[serde(default)]
    valid_end_date_text: String,
    #[serde(default)]
    #[allow(dead_code)]
    valid_end_time: String,
    #[serde(default)]
    applicable_models: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UsageOverviewResponse {
    #[serde(default)]
    code: i64,
    #[serde(default)]
    msg: String,
    data: Option<UsageOverviewData>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UsageOverviewData {
    #[serde(default)]
    total_tokens: i64,
}

// ─── Fetch ───────────────────────────────────────────────────────────────────

/// Fetch Meituan LongCat quota info.
pub async fn fetch_meituan_quota(client: &Client) -> MeituanQuotaStatus {
    let cookie = match get_auth_cookie() {
        Some(c) => c,
        None => {
            warn!("MEITUAN_AUTH_COOKIE not set");
            return MeituanQuotaStatus {
                available: false,
                data: None,
                error: Some("MEITUAN_AUTH_COOKIE not set".to_string()),
            };
        }
    };

    let cookie_header = format!("passport_token_key={}", cookie);

    let (packs_result, overview_result) = tokio::join!(
        fetch_token_packs(client, &cookie_header),
        fetch_usage_overview(client, &cookie_header),
    );

    let packs_data = match packs_result {
        Ok(data) => data,
        Err(e) => {
            warn!("Meituan token-packs fetch failed: {e}");
            return MeituanQuotaStatus {
                available: false,
                data: None,
                error: Some(format!("Failed to fetch token packs: {e}")),
            };
        }
    };

    let recent_tokens = match overview_result {
        Ok(data) => data.total_tokens,
        Err(e) => {
            warn!("Meituan usage-overview fetch failed: {e}");
            0
        }
    };

    info!("Meituan quota fetched");

    let packs: Vec<MeituanTokenPack> = packs_data
        .items
        .into_iter()
        .map(|p| MeituanTokenPack {
            package_name: p.package_name,
            source_type_text: p.source_type_text,
            source_type_code: p.source_type_code,
            status_text: p.status_text,
            status_code: p.status_code,
            total_token_amount: p.total_token_amount,
            used_token_amount: p.used_token_amount,
            remain_token_amount: p.remain_token_amount,
            usage_percent: p.usage_percent,
            valid_start_time: p.valid_start_time,
            valid_end_date_text: p.valid_end_date_text,
            applicable_models: p.applicable_models,
        })
        .collect();

    MeituanQuotaStatus {
        available: true,
        data: Some(MeituanQuotaData {
            packs,
            active_count: packs_data.active_count,
            recent_7d_tokens: recent_tokens,
        }),
        error: None,
    }
}

async fn fetch_token_packs(
    client: &Client,
    cookie: &str,
) -> Result<TokenPacksData, String> {
    let url = format!("{LONGCAT_BASE_URL}/api/pay/commercial/entitlements/token-packs/list");

    let resp = client
        .post(&url)
        .header("Cookie", cookie)
        .header("Content-Type", "application/json")
        .header(
            "User-Agent",
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36",
        )
        .body("{}")
        .timeout(std::time::Duration::from_secs(MEITUAN_TIMEOUT_SECS))
        .send()
        .await
        .map_err(|e| format!("Network error: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }

    let body: TokenPacksResponse = resp
        .json()
        .await
        .map_err(|e| format!("Parse error: {e}"))?;

    if body.code != 0 {
        return Err(format!("API error: {} - {}", body.code, body.msg));
    }

    body.data.ok_or_else(|| "No data in response".to_string())
}

async fn fetch_usage_overview(
    client: &Client,
    cookie: &str,
) -> Result<UsageOverviewData, String> {
    let url = format!("{LONGCAT_BASE_URL}/api/pay/quota/metering/token-usage/overview");

    let resp = client
        .post(&url)
        .header("Cookie", cookie)
        .header("Content-Type", "application/json")
        .header(
            "User-Agent",
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36",
        )
        .body("{}")
        .timeout(std::time::Duration::from_secs(MEITUAN_TIMEOUT_SECS))
        .send()
        .await
        .map_err(|e| format!("Network error: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }

    let body: UsageOverviewResponse = resp
        .json()
        .await
        .map_err(|e| format!("Parse error: {e}"))?;

    if body.code != 0 {
        return Err(format!("API error: {} - {}", body.code, body.msg));
    }

    body.data.ok_or_else(|| "No data in response".to_string())
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_auth_cookie() {
        temp_env::with_var("MEITUAN_AUTH_COOKIE", Some("some_token_value"), || {
            assert_eq!(get_auth_cookie(), Some("some_token_value".to_string()));
        });
    }

    #[test]
    fn test_get_auth_cookie_unset() {
        temp_env::with_var("MEITUAN_AUTH_COOKIE", None::<&str>, || {
            assert_eq!(get_auth_cookie(), None);
        });
    }

    #[test]
    fn test_get_auth_cookie_empty() {
        temp_env::with_var("MEITUAN_AUTH_COOKIE", Some(""), || {
            assert_eq!(get_auth_cookie(), None);
        });
    }

    #[test]
    fn test_parse_token_packs_response() {
        let json = r#"{
            "code": 0,
            "msg": "success",
            "data": {
                "userId": 123,
                "activeCount": 1,
                "historyCount": 0,
                "total": 1,
                "pageNo": 1,
                "pageSize": 20,
                "totalPage": 1,
                "items": [{
                    "packageName": "实名奖励Token1000万资源包",
                    "sourceTypeText": "实名奖励",
                    "statusText": "使用中",
                    "totalTokenAmount": 10000000,
                    "usedTokenAmount": 765797,
                    "remainTokenAmount": 9234203,
                    "usagePercent": 7,
                    "validEndDateText": "2026-07-31",
                    "validEndTime": "2026-07-31T07:49:16.000+00:00"
                }]
            }
        }"#;

        let resp: TokenPacksResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.code, 0);
        let data = resp.data.unwrap();
        assert_eq!(data.active_count, 1);
        assert_eq!(data.items.len(), 1);
        assert_eq!(data.items[0].total_token_amount, 10_000_000);
        assert_eq!(data.items[0].remain_token_amount, 9_234_203);
        assert_eq!(data.items[0].usage_percent, 7);
    }
}
