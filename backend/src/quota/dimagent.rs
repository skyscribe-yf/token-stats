//! DimAgent (dimcode) subscription quota integration.
//!
//! Primary path: the locally-installed Dim CLI — `dim usage --json`. The CLI
//! authenticates through the OAuth refresh token persisted in
//! `~/.dimcode/v2/auth.json` (auto-rotated by the CLI), so the card works
//! without any cookie or env secret: this is the "active read" path.
//!
//! Fallback path: the DimAgent console API `https://dimagent.cn/api/*` using
//! the browser `session` cookie injected via `DIMAGENT_SESSION_COOKIE`
//! (`Cookie: session=<value>`). When the cookie is set we also enrich the
//! card with last-30d call/token stats from `/api/user/daily-stats`.
//!
//! Cookies discovered during reverse engineering (see AGENTS.md):
//! - `session` — signed Flask-style session; the only credential needed for
//!   the console API (GET requests).
//! - `_c_WBKFRo` — NOT required for auth; a site analytics/anti-bot cookie.

use super::types::{
    DimAgentFeatureMeter, DimAgentQuotaData, DimAgentQuotaStatus, DimAgentRecent30d,
};
use reqwest::Client;
use serde::Deserialize;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tokio::process::Command;
use tokio::sync::Mutex;

const CONSOLE_API_BASE: &str = "https://dimagent.cn/api";
const CLI_TIMEOUT_SECS: u64 = 25;
const HTTP_TIMEOUT_SECS: u64 = 15;
/// CLI output is cached in-process so the 0.5s `dim usage` spawn is not
/// repeated on every 30s quota poll (visible tab).
const CLI_CACHE_TTL_SECS: u64 = 120;
/// The console API reports units in milli-units (×1000 vs the CLI).
const API_UNIT_SCALE: f64 = 1000.0;

// ─── JSON payload shapes (shared by CLI output and console API) ──────────────

#[derive(Debug, Clone, Deserialize)]
struct SubscriptionPayload {
    #[serde(default)]
    subscription: Option<InnerSubscription>,
    #[serde(default)]
    current_term: Option<CurrentTerm>,
    #[serde(default)]
    product: Option<Product>,
    #[serde(default)]
    price: Option<Price>,
}

#[derive(Debug, Clone, Deserialize)]
struct InnerSubscription {
    #[serde(default)]
    status: String,
    #[serde(default)]
    cancel_at_period_end: bool,
    #[serde(default)]
    current_term_id: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
struct CurrentTerm {
    #[serde(default)]
    start_at: String,
    #[serde(default)]
    end_at: String,
}

#[derive(Debug, Clone, Deserialize)]
struct Product {
    #[serde(default)]
    name: String,
    #[serde(default)]
    description: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct Price {
    #[serde(default)]
    billing_interval: String,
    /// Price amount in minor units (e.g. 990 = ¥9.90).
    #[serde(default)]
    amount: f64,
}

#[derive(Debug, Clone, Deserialize)]
struct CreditsPayload {
    /// Bucket list; pick by term id when the account has multiple subs.
    #[serde(default)]
    subscription_buckets: Vec<Bucket>,
    #[serde(default)]
    subscription_bucket: Option<Bucket>,
    #[serde(default)]
    total_units: f64,
    #[serde(default)]
    used_units: f64,
    #[serde(default)]
    remaining_units: f64,
}

#[derive(Debug, Clone, Deserialize)]
struct Bucket {
    #[serde(default)]
    term_id: Option<i64>,
    #[serde(default)]
    total_units: f64,
    #[serde(default)]
    used_units: f64,
    #[serde(default)]
    remaining_units: f64,
}

#[derive(Debug, Clone, Deserialize)]
struct FeatureMeter {
    #[serde(default)]
    feature_key: String,
    #[serde(default)]
    unit: String,
    #[serde(default)]
    unlimited: bool,
    #[serde(default)]
    total_used: f64,
    #[serde(default)]
    total_allowance: f64,
    #[serde(default)]
    total_remaining: f64,
    #[serde(default)]
    period_end: Option<String>,
}

/// `dim usage --json` output wrapper.
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)] // several fields are kept for shape documentation
struct CliUsageOutput {
    #[serde(default)]
    account_id: i64,
    #[serde(default)]
    subscription: Option<SubscriptionPayload>,
    #[serde(default)]
    credits: Option<CreditsPayload>,
    #[serde(default)]
    feature_meters: Vec<FeatureMeter>,
}

/// Generic `{"data": T}` console API envelope.
#[derive(Debug, Deserialize)]
struct ApiEnvelope<T> {
    data: T,
}

// ─── Cache ───────────────────────────────────────────────────────────────────

struct CliCache {
    fetched_at: Option<Instant>,
    result: Option<Result<DimAgentQuotaData, String>>,
}

static CLI_CACHE: Mutex<CliCache> = Mutex::const_new(CliCache {
    fetched_at: None,
    result: None,
});

// ─── Public hook ─────────────────────────────────────────────────────────────

/// Fetch the DimAgent subscription card.
pub async fn fetch_dimagent_quota(client: &Client) -> DimAgentQuotaStatus {
    let cli_result = {
        let mut cache = CLI_CACHE.lock().await;
        let fresh = cache
            .fetched_at
            .map(|t| t.elapsed() < Duration::from_secs(CLI_CACHE_TTL_SECS))
            .unwrap_or(false);
        if !fresh {
            let out = fetch_via_cli().await;
            cache.fetched_at = Some(Instant::now());
            cache.result = Some(out);
        }
        cache.result.clone()
    };

    match cli_result {
        Some(Ok(mut data)) => {
            if std::env::var("DIMAGENT_SESSION_COOKIE").is_ok() {
                data.recent_30d = fetch_recent_30d(client).await.ok();
            }
            DimAgentQuotaStatus {
                available: true,
                data: Some(data),
                error: None,
            }
        }
        Some(Err(cli_err)) => {
            // Fallback: console API with injected session cookie.
            match fetch_via_api(client).await {
                Ok(mut data) => {
                    data.recent_30d = fetch_recent_30d(client).await.ok();
                    DimAgentQuotaStatus {
                        available: true,
                        data: Some(data),
                        error: None,
                    }
                }
                Err(api_err) => DimAgentQuotaStatus {
                    available: false,
                    data: None,
                    error: Some(format!("{cli_err}; console api: {api_err}")),
                },
            }
        }
        None => DimAgentQuotaStatus {
            available: false,
            data: None,
            error: Some("dim usage unavailable".to_string()),
        },
    }
}

// ─── CLI path (active read via ~/.dimcode/v2/auth.json) ──────────────────────

/// Locate the dim CLI binary: `DIM_USAGE_BIN` override, else newest
/// `~/.dimcode/binaries/dimcode-linux-x64/<ver>/bin/dimcode`, else PATH.
fn discover_cli_bin() -> Result<PathBuf, String> {
    if let Ok(p) = std::env::var("DIM_USAGE_BIN") {
        let pb = PathBuf::from(p);
        if pb.is_file() {
            return Ok(pb);
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        let root = PathBuf::from(home)
            .join(".dimcode")
            .join("binaries")
            .join("dimcode-linux-x64");
        let mut best: Option<(Vec<u32>, PathBuf)> = None;
        if let Ok(entries) = std::fs::read_dir(&root) {
            for entry in entries.flatten() {
                let bin = entry.path().join("bin").join("dimcode");
                if !bin.is_file() {
                    continue;
                }
                let ver = entry.file_name().to_string_lossy().to_string();
                let parts: Vec<u32> =
                    ver.split('.').filter_map(|s| s.parse::<u32>().ok()).collect();
                if parts.len() == 3
                    && best.as_ref().is_none_or(|(v, _)| parts > *v)
                {
                    best = Some((parts, bin));
                }
            }
        }
        if let Some((_, bin)) = best {
            return Ok(bin);
        }
    }
    // PATH fallback — spawn failure is surfaced as an error by the caller.
    Ok(PathBuf::from("dim"))
}

async fn fetch_via_cli() -> Result<DimAgentQuotaData, String> {
    let bin = discover_cli_bin()?;
    let output = tokio::time::timeout(
        Duration::from_secs(CLI_TIMEOUT_SECS),
        Command::new(&bin)
            .arg("usage")
            .arg("--json")
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| format!("dim usage timed out after {CLI_TIMEOUT_SECS}s"))?
    .map_err(|e| format!("failed to spawn {bin:?}: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(format!(
            "dim usage exited with {:?}: {}",
            output.status.code(),
            if stderr.is_empty() { "(no stderr)".to_string() } else { stderr }
        ));
    }

    let parsed: CliUsageOutput = serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("failed to parse dim usage --json: {e}"))?;
    parsed_to_card(parsed)
}

// ─── Console API path (DIMAGENT_SESSION_COOKIE) ──────────────────────────────

async fn fetch_via_api(client: &Client) -> Result<DimAgentQuotaData, String> {
    let cookie = std::env::var("DIMAGENT_SESSION_COOKIE")
        .map_err(|_| "DIMAGENT_SESSION_COOKIE not set".to_string())?;

    let subscription: ApiEnvelope<SubscriptionPayload> = get_json(
        client,
        &format!("{CONSOLE_API_BASE}/me/subscription"),
        &cookie,
    )
    .await?;
    let credits: ApiEnvelope<CreditsPayload> =
        get_json(client, &format!("{CONSOLE_API_BASE}/me/credits"), &cookie).await?;
    let meters: ApiEnvelope<Vec<FeatureMeter>> =
        get_json(client, &format!("{CONSOLE_API_BASE}/me/feature-meters"), &cookie).await?;

    let term_match = subscription
        .data
        .subscription
        .as_ref()
        .and_then(|s| s.current_term_id);

    let bucket = credits
        .data
        .subscription_buckets
        .iter()
        .find(|b| term_match.map(|t| b.term_id == Some(t)).unwrap_or(false))
        .or(credits.data.subscription_bucket.as_ref());

    let (total, used, remaining) = match bucket {
        Some(b) => (
            b.total_units,
            b.used_units,
            b.remaining_units,
        ),
        None => (
            credits.data.total_units,
            credits.data.used_units,
            credits.data.remaining_units,
        ),
    };

    // quota-estimate (optional): average units/call → estimated calls left.
    let (estimated, request_count_total) = match get_json::<ApiEnvelope<QuotaEstimate>>(
        client,
        &format!("{CONSOLE_API_BASE}/user/quota-estimate"),
        &cookie,
    )
    .await
    {
        Ok(est) => (est.data.estimated_remaining_calls, est.data.request_count),
        Err(e) => {
            tracing::warn!("DimAgent quota-estimate unavailable: {e}");
            (None, 0)
        }
    };

    card_from_parts(
        subscription.data,
        total,
        used,
        remaining,
        meters.data,
        estimated,
        request_count_total,
    )
}

#[derive(Debug, Deserialize)]
struct QuotaEstimate {
    #[serde(default)]
    estimated_remaining_calls: Option<i64>,
    #[serde(default)]
    request_count: i64,
}

async fn get_json<T: serde::de::DeserializeOwned>(
    client: &Client,
    url: &str,
    cookie: &str,
) -> Result<T, String> {
    let resp = client
        .get(url)
        .header("Cookie", format!("session={cookie}"))
        .header("Accept", "application/json")
        .timeout(Duration::from_secs(HTTP_TIMEOUT_SECS))
        .send()
        .await
        .map_err(|e| format!("GET {url}: {e}"))?;

    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!("GET {url}: HTTP {status}: {}", super::truncate_error_body(&body)));
    }
    serde_json::from_str(&body).map_err(|e| format!("parse {url}: {e}"))
}

/// Last-30d call/token stats from `/api/user/daily-stats`.
async fn fetch_recent_30d(client: &Client) -> Result<DimAgentRecent30d, String> {
    let cookie = std::env::var("DIMAGENT_SESSION_COOKIE")
        .map_err(|_| "DIMAGENT_SESSION_COOKIE not set".to_string())?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let start = now.saturating_sub(30 * 86400);

    let stats: ApiEnvelope<Vec<DailyStat>> = get_json(
        client,
        &format!(
            "{CONSOLE_API_BASE}/user/daily-stats?start_time={start}&end_time={now}&interval=daily"
        ),
        &cookie,
    )
    .await?;

    let mut calls = 0i64;
    let mut total_tokens = 0i64;
    let mut prompt_tokens = 0i64;
    let mut completion_tokens = 0i64;
    let mut cache_tokens = 0i64;
    let mut quota_units = 0f64;
    for d in &stats.data {
        calls += d.request_count;
        total_tokens += d.total_tokens;
        prompt_tokens += d.prompt_tokens;
        completion_tokens += d.completion_tokens;
        cache_tokens += d.cache_tokens;
        quota_units += d.quota_consumed;
    }
    Ok(DimAgentRecent30d {
        calls,
        total_tokens,
        prompt_tokens,
        completion_tokens,
        cache_tokens,
        quota_units,
    })
}

#[derive(Debug, Deserialize)]
struct DailyStat {
    #[serde(default)]
    request_count: i64,
    #[serde(default)]
    prompt_tokens: i64,
    #[serde(default)]
    completion_tokens: i64,
    #[serde(default)]
    cache_tokens: i64,
    #[serde(default)]
    total_tokens: i64,
    #[serde(default)]
    quota_consumed: f64,
}

// ─── Mapping ─────────────────────────────────────────────────────────────────

fn parsed_to_card(out: CliUsageOutput) -> Result<DimAgentQuotaData, String> {
    let sub = out.subscription.ok_or("no subscription in dim usage output")?;
    let credits = out.credits.unwrap_or(CreditsPayload {
        subscription_buckets: Vec::new(),
        subscription_bucket: None,
        total_units: 0.0,
        used_units: 0.0,
        remaining_units: 0.0,
    });
    // CLI reports whole units; scale is 1.
    card_from_parts(
        sub,
        credits.total_units / 1.0,
        credits.used_units / 1.0,
        credits.remaining_units / 1.0,
        out.feature_meters,
        None,
        0,
    )
}

#[allow(clippy::too_many_arguments)]
fn card_from_parts(
    sub: SubscriptionPayload,
    total_units_raw: f64,
    used_units_raw: f64,
    remaining_units_raw: f64,
    meters: Vec<FeatureMeter>,
    estimated_remaining_calls: Option<i64>,
    request_count_total: i64,
) -> Result<DimAgentQuotaData, String> {
    let term = sub
        .current_term
        .clone()
        .ok_or_else(|| "no current_term in subscription payload".to_string())?;
    // The console API reports units in milli-units; the CLI reports whole
    // units. Normalize both to whole units.
    let scale = if total_units_raw >= 1_000_000.0 { API_UNIT_SCALE } else { 1.0 };
    let total = (total_units_raw / scale).round() as i64;
    let used = (used_units_raw / scale).round() as i64;
    let remaining = (remaining_units_raw / scale).round() as i64;

    let product = sub.product.unwrap_or(Product { name: String::new(), description: None });
    let price = sub.price.unwrap_or(Price {
        billing_interval: String::new(),
        amount: 0.0,
    });

    Ok(DimAgentQuotaData {
        plan_name: product.name,
        plan_description: product.description.filter(|d| !d.is_empty()),
        price_cny: price.amount / 100.0,
        billing_interval: price.billing_interval,
        subscription_status: sub
            .subscription
            .as_ref()
            .map(|s| s.status.clone())
            .unwrap_or_default(),
        cancel_at_period_end: sub
            .subscription
            .as_ref()
            .map(|s| s.cancel_at_period_end)
            .unwrap_or(false),
        period_start: term.start_at,
        period_end: term.end_at,
        total_units: total,
        used_units: used,
        remaining_units: remaining,
        estimated_remaining_calls,
        request_count_total,
        feature_meters: meters
            .into_iter()
            .map(|m| DimAgentFeatureMeter {
                feature_key: m.feature_key,
                unit: m.unit,
                unlimited: m.unlimited,
                used: m.total_used as i64,
                allowance: m.total_allowance as i64,
                remaining: m.total_remaining as i64,
                period_end: m.period_end,
            })
            .collect(),
        recent_30d: None,
    })
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_sub() -> SubscriptionPayload {
        SubscriptionPayload {
            subscription: Some(InnerSubscription {
                status: "active".into(),
                cancel_at_period_end: true,
                current_term_id: Some(2830),
            }),
            current_term: Some(CurrentTerm {
                start_at: "2026-08-22T12:02:10.226Z".into(),
                end_at: "2026-09-22T12:02:10.226Z".into(),
            }),
            product: Some(Product {
                name: "Nano套餐".into(),
                description: Some("估约 300 次对话".into()),
            }),
            price: Some(Price {
                billing_interval: "month".into(),
                amount: 990.0,
            }),
        }
    }

    #[test]
    fn maps_cli_units() {
        let card = card_from_parts(sample_sub(), 1500.0, 110.0, 1390.0, vec![], None, 0)
            .unwrap();
        assert_eq!(card.plan_name, "Nano套餐");
        assert_eq!(card.price_cny, 9.9);
        assert_eq!(card.total_units, 1500);
        assert_eq!(card.used_units, 110);
        assert_eq!(card.remaining_units, 1390);
        assert_eq!(card.billing_interval, "month");
        assert!(card.cancel_at_period_end);
    }

    #[test]
    fn maps_api_milli_units() {
        let card = card_from_parts(
            sample_sub(),
            1_500_000.0,
            132_866.0,
            1_367_134.0,
            vec![],
            Some(3198),
            1193,
        )
        .unwrap();
        assert_eq!(card.total_units, 1500);
        assert_eq!(card.used_units, 133);
        assert_eq!(card.remaining_units, 1367);
        assert_eq!(card.estimated_remaining_calls, Some(3198));
    }

    #[test]
    fn missing_term_is_error() {
        let mut sub = sample_sub();
        sub.current_term = None;
        assert!(card_from_parts(sub, 1.0, 1.0, 1.0, vec![], None, 0).is_err());
    }
}
