//! ZAI Router (api.zairouter.com) balance/quota fetcher.
//!
//! ZAI is a Claude Code relay: you top up RMB and receive USD face value at
//! 1:1 (verified from the top-up order: `amount` 10000 分 → `credit_amount`
//! 100.0), then each model is charged at a multiple of the official Anthropic
//! list price (see `compute_zai_cost` in `pricing.rs`).
//!
//! Endpoints (all `Authorization: Bearer <ZAI_API_KEY>`, GET):
//!
//! - `GET /dashboard/info` — account + credit balance ("到账卡" list),
//!   lifetime `credit_used`, `requests`, rate limits.
//! - `GET /dashboard/live` — same snapshot plus `daily_usage` /
//!   `monthly_usage` (per-model `Requests`/`Prompt`/`Completion`/`CreditUsed`
//!   and `CachedDetails`).
//! - `GET /dashboard/status` — includes `suspended` and `balance`.
//!
//! The token-count fields use the OpenAI convention: `Prompt` **includes**
//! cache read and cache write. The card reports the raw buckets separately so
//! the UI can show the same shape as the platform console.
//!
//! Balance is presented in RMB. Because credit is bought 1:1 with RMB, the
//! credit figure *is* the RMB figure — that is what the platform console
//! displays to the user (`getCurrencySymbol()` yields "¥" when
//! `factor == 1`, "$" otherwise).

use super::types::*;
use reqwest::Client;
use tracing::warn;

const ZAI_API_BASE: &str = "https://api.zairouter.com";
const HTTP_TIMEOUT_SECS: u64 = 15;

/// Read `ZAI_API_KEY` from the environment.
pub fn get_api_key() -> Option<String> {
    std::env::var("ZAI_API_KEY").ok().filter(|k| !k.is_empty())
}

/// Fetch ZAI balance and usage.
pub async fn fetch_zai_quota(client: &Client) -> ZaiQuotaStatus {
    let api_key = match get_api_key() {
        Some(k) => k,
        None => {
            return ZaiQuotaStatus {
                available: false,
                data: None,
                error: Some("ZAI_API_KEY not set".to_string()),
            };
        }
    };

    let info_fut = fetch_json(client, &api_key, "/dashboard/info");
    let live_fut = fetch_json(client, &api_key, "/dashboard/live");
    let status_fut = fetch_json(client, &api_key, "/dashboard/status");

    let (info, live, status) = tokio::join!(info_fut, live_fut, status_fut);

    let info = match info {
        Ok(v) => v,
        Err(e) => {
            warn!("ZAI info fetch failed: {e}");
            return ZaiQuotaStatus {
                available: false,
                data: None,
                error: Some(format!("Failed to fetch ZAI info: {e}")),
            };
        }
    };

    // `/dashboard/live` carries the per-window usage breakdown and is the
    // richer source; fall back to `/dashboard/info` (same account fields, no
    // window usage) when it fails.
    let live = live.unwrap_or(serde_json::Value::Null);
    let status = status.ok();

    match parse_zai_data(&info, &live, status.as_ref()) {
        Ok(data) => ZaiQuotaStatus {
            available: true,
            data: Some(data),
            error: None,
        },
        Err(e) => {
            warn!("Failed to parse ZAI data: {e}");
            ZaiQuotaStatus {
                available: false,
                data: None,
                error: Some(format!("Data extraction error: {e}")),
            }
        }
    }
}

async fn fetch_json(
    client: &Client,
    api_key: &str,
    path: &str,
) -> Result<serde_json::Value, String> {
    let url = format!("{}{}", ZAI_API_BASE, path);
    let response = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", api_key))
        .timeout(std::time::Duration::from_secs(HTTP_TIMEOUT_SECS))
        .send()
        .await
        .map_err(|e| format!("Request failed: {e}"))?;

    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status()));
    }

    response
        .json::<serde_json::Value>()
        .await
        .map_err(|e| format!("Parse error: {e}"))
}

/// Parse one window (`daily_usage` / `monthly_usage`) into counters plus the
/// per-model breakdown.
fn parse_window(window: &serde_json::Value) -> (f64, i64, i64, i64, i64, i64, Vec<ZaiModelUsage>) {
    let used = window["CreditUsed"].as_f64().unwrap_or(0.0);
    let requests = window["Requests"].as_i64().unwrap_or(0);
    let prompt = window["Prompt"].as_i64().unwrap_or(0);
    let completion = window["Completion"].as_i64().unwrap_or(0);
    let cache_read = window["CachedDetails"]["cache_read_input_tokens"]
        .as_i64()
        .unwrap_or(0);
    let cache_write = window["CachedDetails"]["cache_creation_input_tokens"]
        .as_i64()
        .unwrap_or(0);

    let mut models = Vec::new();
    if let Some(map) = window["ModelUsage"].as_object() {
        for (model, entry) in map {
            let model_requests = entry["Requests"].as_i64().unwrap_or(0);
            if model_requests == 0 {
                continue;
            }
            models.push(ZaiModelUsage {
                model: model.clone(),
                requests: model_requests,
                prompt_tokens: entry["Prompt"].as_i64().unwrap_or(0),
                completion_tokens: entry["Completion"].as_i64().unwrap_or(0),
                cache_read_tokens: entry["CachedDetails"]["cache_read_input_tokens"]
                    .as_i64()
                    .unwrap_or(0),
                cache_write_tokens: entry["CachedDetails"]["cache_creation_input_tokens"]
                    .as_i64()
                    .unwrap_or(0),
                credit_used: entry["CreditUsed"].as_f64().unwrap_or(0.0),
            });
        }
    }
    models.sort_by(|a, b| {
        b.credit_used
            .partial_cmp(&a.credit_used)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    (
        used,
        requests,
        prompt,
        completion,
        cache_read,
        cache_write,
        models,
    )
}

fn parse_zai_data(
    info: &serde_json::Value,
    live: &serde_json::Value,
    status: Option<&serde_json::Value>,
) -> Result<ZaiQuotaData, String> {
    let user_id = info["id"]
        .as_i64()
        .ok_or_else(|| "missing 'id'".to_string())?;

    // Cards: `credit_balance` normally lists top-up cards. Once a card is
    // fully consumed the platform drops it and may insert a synthetic
    // "Debt carry" entry (negative amount/balance, Go zero `expires_at`) when
    // usage overshot the credit — that is not a card and must not be rendered
    // as one.
    let mut cards = Vec::new();
    let mut card_balance = 0.0_f64;
    let mut expires_at = String::new();
    if let Some(arr) = info["credit_balance"].as_array() {
        for entry in arr {
            let amount = entry["amount"].as_f64().unwrap_or(0.0);
            let balance = entry["balance"].as_f64().unwrap_or(0.0);
            let raw_expires = entry["expires_at"].as_str().unwrap_or("");
            // Go zero time ("0001-01-01...") means "no expiry".
            let expires = if raw_expires.starts_with("0001-") {
                String::new()
            } else {
                raw_expires.to_string()
            };
            card_balance += balance;
            if !expires.is_empty() && (expires_at.is_empty() || expires < expires_at) {
                expires_at = expires.clone();
            }
            // Only positive grants are real top-up cards.
            if amount > 0.0 {
                cards.push(ZaiCreditCard {
                    amount,
                    balance,
                    reference: entry["reference"].as_str().unwrap_or("").to_string(),
                    granted_at: entry["granted_at"].as_str().unwrap_or("").to_string(),
                    expires_at: expires,
                });
            }
        }
    }

    // Balance: prefer the card sum; fall back to the top-level field.
    let balance = if info["credit_balance"].is_array() {
        card_balance
    } else {
        info["balance"].as_f64().unwrap_or(0.0)
    };
    let credit_used = info["credit_used"].as_f64().unwrap_or(0.0);

    // Total ever granted. The platform's card list shrinks as cards are
    // consumed, so summing it understates the grant — the identity
    // `granted = used + remaining` holds and survives consumed cards.
    // (Cross-check against the order history: ¥100 + ¥10 paid = ¥110 granted.)
    let card_grant_total: f64 = cards.iter().map(|c| c.amount).sum();
    let credit_total = card_grant_total.max(credit_used + balance);

    let (
        daily_used,
        daily_requests,
        daily_input,
        daily_output,
        daily_cache_read,
        daily_cache_write,
        daily_models,
    ) = parse_window(&live["daily_usage"]);
    let (
        monthly_used,
        monthly_requests,
        monthly_input,
        monthly_output,
        monthly_cache_read,
        monthly_cache_write,
        monthly_models,
    ) = parse_window(&live["monthly_usage"]);

    Ok(ZaiQuotaData {
        user_id,
        name: info["name"].as_str().unwrap_or("").to_string(),
        alias: info["alias"].as_str().unwrap_or("").to_string(),
        email: info["email"].as_str().unwrap_or("").to_string(),
        balance,
        credit_total,
        credit_used,
        expires_at,
        cards,
        total_requests: info["requests"].as_i64().unwrap_or(0),
        suspended: status
            .and_then(|s| s["suspended"].as_bool())
            .or_else(|| info["suspended"].as_bool())
            .unwrap_or(false),
        daily_used,
        daily_requests,
        daily_input_tokens: daily_input,
        daily_output_tokens: daily_output,
        daily_cache_read_tokens: daily_cache_read,
        daily_cache_write_tokens: daily_cache_write,
        daily_models,
        monthly_used,
        monthly_requests,
        monthly_input_tokens: monthly_input,
        monthly_output_tokens: monthly_output,
        monthly_cache_read_tokens: monthly_cache_read,
        monthly_cache_write_tokens: monthly_cache_write,
        monthly_models,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info_with(cards: serde_json::Value, used: f64, balance: f64) -> serde_json::Value {
        serde_json::json!({
            "id": 33, "name": "u_x", "alias": "al", "email": "e@x",
            "credit_used": used, "credit_balance": cards, "requests": 5,
            "balance": balance,
        })
    }

    /// A normal top-up card.
    #[test]
    fn parses_active_card() {
        let info = info_with(
            serde_json::json!([{
                "amount": 100.0, "balance": 39.27,
                "reference": "Recharge", "granted_at": "2026-09-15T10:54:17+08:00",
                "expires_at": "2027-09-15T10:54:17+08:00"
            }]),
            60.73,
            39.27,
        );
        let d = parse_zai_data(&info, &serde_json::Value::Null, None).unwrap();
        assert_eq!(d.cards.len(), 1);
        assert_eq!(d.balance, 39.27);
        assert_eq!(d.credit_total, 100.0);
        assert_eq!(d.expires_at, "2027-09-15T10:54:17+08:00");
        assert!(!d.suspended);
    }

    /// Once usage overshoots, the platform replaces the consumed card with a
    /// synthetic negative "Debt carry" entry carrying a Go zero `expires_at`.
    /// It must not surface as a card, nor as an expiry date.
    #[test]
    fn debt_carry_entry_is_not_a_card() {
        let info = info_with(
            serde_json::json!([{
                "amount": -2.674, "balance": -2.674,
                "reference": "Debt carry",
                "granted_at": "2026-09-15T12:38:03+08:00",
                "expires_at": "0001-01-01T00:00:00Z"
            }]),
            113.674,
            -2.674,
        );
        let d = parse_zai_data(&info, &serde_json::Value::Null, None).unwrap();
        assert!(
            d.cards.is_empty(),
            "debt carry must not be listed as a card"
        );
        assert_eq!(d.balance, -2.674);
        assert_eq!(d.expires_at, "", "Go zero time must not become an expiry");
        // granted = used + remaining, recovering the consumed grants.
        assert!(
            (d.credit_total - 111.0).abs() < 1e-6,
            "got {}",
            d.credit_total
        );
    }

    /// A negative balance with `suspended` set is what drives the alert.
    #[test]
    fn suspended_flag_comes_from_status_endpoint() {
        let info = info_with(serde_json::json!([]), 50.0, -1.0);
        let status = serde_json::json!({"suspended": true, "balance": -1.0});
        let d = parse_zai_data(&info, &serde_json::Value::Null, Some(&status)).unwrap();
        assert!(d.suspended);
    }

    /// `Prompt` includes cache tokens per the OpenAI convention; the parser
    /// must expose the cache buckets separately rather than folding them in.
    #[test]
    fn parses_window_cache_buckets() {
        let live = serde_json::json!({
            "daily_usage": {
                "Requests": 3, "Prompt": 1200, "Completion": 40, "CreditUsed": 1.5,
                "CachedDetails": {
                    "cache_read_input_tokens": 900,
                    "cache_creation_input_tokens": 200,
                },
                "ModelUsage": {
                    "claude-fable-5-1": {
                        "Requests": 3, "Prompt": 1200, "Completion": 40, "CreditUsed": 1.5,
                        "CachedDetails": {
                            "cache_read_input_tokens": 900,
                            "cache_creation_input_tokens": 200,
                        },
                    },
                    "XAPI": {"Requests": 0, "CreditUsed": 0.0},
                },
            },
        });
        let info = info_with(serde_json::json!([]), 0.0, 0.0);
        let d = parse_zai_data(&info, &live, None).unwrap();
        assert_eq!(d.daily_requests, 3);
        assert_eq!(d.daily_input_tokens, 1200);
        assert_eq!(d.daily_cache_read_tokens, 900);
        assert_eq!(d.daily_cache_write_tokens, 200);
        // Zero-request model rows are dropped.
        assert_eq!(d.daily_models.len(), 1);
        assert_eq!(d.daily_models[0].model, "claude-fable-5-1");
    }
}
