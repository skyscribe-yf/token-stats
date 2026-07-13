//! Meituan LongCat quota fetcher.
//!
//! Fetches token resource pack balance and usage from the LongCat API platform.
//! API endpoints (all POST with JSON body):
//!   - /api/pay/commercial/entitlements/token-packs/list
//!   - /api/pay/commercial/orders/list
//!   - /api/pay/quota/metering/token-usage/overview
//!
//! Purchased token packs (orders) are merged with entitlements because the
//! entitlements API may not return fully-consumed or recently-purchased packs.
//!
//! Authentication: `MEITUAN_AUTH_COOKIE` env var containing the `passport_token_key` value.

use super::types::*;
use reqwest::Client;
use serde::Deserialize;
use std::collections::HashSet;
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
    #[serde(default)]
    sku_code: String,
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

// ─── Orders response types ────────────────────────────────────────────────────

/// Synthetic pack entry built from order data.
#[derive(Debug, Clone)]
struct OrderPack {
    package_name: String,
    source_type_text: String,
    source_type_code: i64,
    status_text: String,
    status_code: i64,
    total_token_amount: i64,
    used_token_amount: i64,
    remain_token_amount: i64,
    usage_percent: i64,
    valid_start_time: String,
    valid_end_date_text: String,
    applicable_models: Vec<String>,
    sku_code: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OrdersResponse {
    #[serde(default)]
    code: i64,
    #[serde(default)]
    msg: String,
    data: Option<OrdersData>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OrdersData {
    #[serde(default)]
    orders: Vec<OrderItem>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OrderItem {
    #[serde(default)]
    order_type: String,
    #[serde(default)]
    order_status: String,
    #[serde(default)]
    order_status_text: String,
    #[serde(default)]
    entitlement_grant_status: String,
    #[serde(default)]
    token_amount: i64,
    #[serde(default)]
    valid_start_time: String,
    #[serde(default)]
    valid_end_time: String,
    #[serde(default)]
    purchase_type_text: String,
    #[serde(default)]
    sku_code: String,
    #[serde(default)]
    sku_snapshot: String,
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

    let (packs_result, orders_result, overview_result) = tokio::join!(
        fetch_token_packs(client, &cookie_header),
        fetch_token_pack_orders(client, &cookie_header),
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

    // Build set of known sku_codes from the entitlement list
    let known_sku_ids: HashSet<String> = packs_data
        .items
        .iter()
        .map(|p| p.sku_code.clone())
        .collect();

    let mut packs: Vec<MeituanTokenPack> = packs_data
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

    // Merge purchased packs from orders that aren't already in the entitlement list
    for op in orders_result {
        if known_sku_ids.contains(&op.sku_code) {
            continue;
        }
        packs.push(MeituanTokenPack {
            package_name: op.package_name,
            source_type_text: op.source_type_text,
            source_type_code: op.source_type_code,
            status_text: op.status_text,
            status_code: op.status_code,
            total_token_amount: op.total_token_amount,
            used_token_amount: op.used_token_amount,
            remain_token_amount: op.remain_token_amount,
            usage_percent: op.usage_percent,
            valid_start_time: op.valid_start_time,
            valid_end_date_text: op.valid_end_date_text,
            applicable_models: op.applicable_models,
        });
    }

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

// ─── Orders fetch ─────────────────────────────────────────────────────────────

/// Fetch purchased TOKEN_PACK orders and convert to synthetic pack entries.
async fn fetch_token_pack_orders(client: &Client, cookie: &str) -> Vec<OrderPack> {
    let url = format!("{LONGCAT_BASE_URL}/api/pay/commercial/orders/list");

    let resp = match client
        .post(&url)
        .header("Cookie", cookie)
        .header("Content-Type", "application/json")
        .header(
            "User-Agent",
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36",
        )
        .body("{\"pageNo\":1,\"pageSize\":50}")
        .timeout(std::time::Duration::from_secs(MEITUAN_TIMEOUT_SECS))
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            warn!("Meituan orders fetch failed: {e}");
            return vec![];
        }
    };

    if !resp.status().is_success() {
        warn!("Meituan orders API returned {}", resp.status());
        return vec![];
    }

    let body: OrdersResponse = match resp.json().await {
        Ok(b) => b,
        Err(e) => {
            warn!("Failed to parse Meituan orders: {e}");
            return vec![];
        }
    };

    if body.code != 0 {
        warn!("Meituan orders API error: {} - {}", body.code, body.msg);
        return vec![];
    }

    let data = match body.data {
        Some(d) => d,
        None => return vec![],
    };

    data.orders
        .into_iter()
        .filter(|o| {
            o.order_type == "TOKEN_PACK"
                && o.order_status == "PAID"
                && o.entitlement_grant_status == "GRANTED"
                && o.token_amount > 0
        })
        .map(|o| {
            let (sku_name, applicable_models) = parse_sku_snapshot(&o.sku_snapshot);
            let expiry_date = o
                .valid_end_time
                .split('T')
                .next()
                .unwrap_or("")
                .to_string();

            let source_type = if o.purchase_type_text.is_empty() {
                "购买".to_string()
            } else {
                o.purchase_type_text.clone()
            };

            // ponytail: per-pack usage is unknown from order data — the
            // entitlements API is the canonical source. Orders-only packs
            // show total amount as remaining; could subtract known-pack
            // usage from usage-overview total but the overview is 7-day
            // not lifetime, so the estimate would be misleading.
            OrderPack {
                package_name: sku_name,
                source_type_text: source_type,
                source_type_code: 1,
                status_text: o.order_status_text,
                status_code: 2,
                total_token_amount: o.token_amount,
                used_token_amount: 0,
                remain_token_amount: o.token_amount,
                usage_percent: 0,
                valid_start_time: o.valid_start_time,
                valid_end_date_text: expiry_date,
                applicable_models,
                sku_code: o.sku_code,
            }
        })
        .collect()
}

/// Extract sku name and applicable models from the skuSnapshot JSON.
fn parse_sku_snapshot(snapshot: &str) -> (String, Vec<String>) {
    if snapshot.is_empty() {
        return (String::new(), Vec::new());
    }

    // skuSnapshot is valid JSON — parse it
    let v: serde_json::Value = match serde_json::from_str(snapshot) {
        Ok(v) => v,
        Err(_) => return (String::new(), Vec::new()),
    };

    let sku_name = v
        .get("skuName")
        .and_then(|v| v.as_str())
        .unwrap_or("Token 资源包")
        .to_string();

    // scopeConfig is a JSON-encoded string within the snapshot
    let applicable_models: Vec<String> = v
        .get("scopeConfig")
        .and_then(|v| v.as_str())
        .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
        .and_then(|cfg| cfg.get("models").cloned())
        .and_then(|models| {
            models.as_array().map(|arr| {
                arr.iter()
                    .filter_map(|m| m.as_str())
                    .filter(|m| *m != "ALL")
                    .map(|m| m.to_string())
                    .collect()
            })
        })
        .unwrap_or_default();

    (sku_name, applicable_models)
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

    #[test]
    fn test_parse_sku_snapshot() {
        // scopeConfig is a JSON-encoded string within the snapshot JSON.
        // This matches the actual API response format.
        let snap = r#"{"skuName":"5000万 新手特惠 Token 资源包","scopeConfig":"{\"models\":[\"ALL\"],\"tools\":[\"ALL\"]}"}"#;
        let (name, models) = parse_sku_snapshot(snap);
        assert_eq!(name, "5000万 新手特惠 Token 资源包");
        assert!(models.is_empty(), "ALL should be filtered out");

        let snap2 = r#"{"skuName":"Test Pack","scopeConfig":"{\"models\":[\"LongCat-2.0\",\"deepseek-v4\"],\"tools\":[\"ALL\"]}"}"#;
        let (name2, models2) = parse_sku_snapshot(snap2);
        assert_eq!(name2, "Test Pack");
        assert_eq!(models2, vec!["LongCat-2.0", "deepseek-v4"]);
    }

    #[test]
    fn test_parse_sku_snapshot_empty() {
        let (name, models) = parse_sku_snapshot("");
        assert!(name.is_empty());
        assert!(models.is_empty());
    }

    #[test]
    fn test_parse_orders_response() {
        let json = r#"{
            "code": 0,
            "msg": "success",
            "data": {
                "total": 1,
                "orders": [{
                    "orderType": "TOKEN_PACK",
                    "orderStatus": "PAID",
                    "orderStatusText": "生效中",
                    "entitlementGrantStatus": "GRANTED",
                    "tokenAmount": 50000000,
                    "validStartTime": "2026-07-02T06:33:43.000+00:00",
                    "validEndTime": "2026-08-01T06:33:43.000+00:00",
                    "purchaseTypeText": "首购",
                    "skuCode": "2d879942-20dc-452a-9e9c-3da40428d16e",
                    "skuSnapshot": "{\"skuName\":\"50M Pack\"}"
                }]
            }
        }"#;

        let resp: OrdersResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.code, 0);
        let data = resp.data.unwrap();
        assert_eq!(data.orders.len(), 1);
        assert_eq!(data.orders[0].token_amount, 50_000_000);
    }
}
