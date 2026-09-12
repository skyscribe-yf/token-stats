//! ZCode (BigModel GLM coding plan) quota fetcher.
//!
//! Queries the same BigModel monitor endpoints the ZCode desktop app uses
//! for its usage page (protocol reverse-engineered from the app bundle):
//!
//! - `GET https://open.bigmodel.cn/api/monitor/usage/quota/limit` — plan
//!   level plus a list of limit windows (`type`/`unit`/`number`/`usage`/
//!   `currentValue`/`remaining`/`percentage`/`nextResetTime` epoch ms/
//!   `usageDetails`). Auth header is the coding-plan API key, plain
//!   (`authorization: <key>`, no Bearer prefix) — verified against the live
//!   API.
//! - `GET https://open.bigmodel.cn/api/biz/subscription/list` — active
//!   subscription rows; we pick the `inCurrentPeriod && status=="VALID"`
//!   entry for plan name / renewal date / term end.
//!
//! The API key lives in plaintext in the ZCode desktop app config
//! (`~/.zcode/v2/config.json` → `provider["builtin:bigmodel-coding-plan"].
//! options.apiKey`; the app rotates it automatically). Env overrides mirror
//! the app's own: `ZCODE_BIGMODEL_USAGE_API_KEY` /
//! `ZCODE_BIGMODEL_USAGE_QUOTA_URL`.
//!
//! Local per-request usage comes from the `source="zcode"` record snapshot
//! (aggregated the same way the Grok card does it), so the card keeps
//! working even when the remote endpoint is unreachable.

use super::types::*;
use crate::models::TokenRecord;
use crate::pricing;
use chrono::TimeZone;
use reqwest::Client;
use serde::Deserialize;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use tracing::{info, warn};

const PROVIDER_ID: &str = "builtin:bigmodel-coding-plan";
const DEFAULT_QUOTA_URL: &str = "https://open.bigmodel.cn/api/monitor/usage/quota/limit";
const SUBSCRIPTION_URL: &str = "https://open.bigmodel.cn/api/biz/subscription/list";
const HTTP_TIMEOUT_SECS: u64 = 15;
/// The ZCode app polls its entitlement snapshot about every 2 minutes; the
/// dashboard refreshes quota every 30s, so a short cache keeps us polite
/// without staling the card.
const CACHE_TTL_SECS: u64 = 60;

/// Remote quota snapshot (plan level + limit windows + subscription).
#[derive(Debug, Clone)]
struct RemoteSnapshot {
    level: Option<String>,
    limits: Vec<ZcodeLimitEntry>,
    subscription: Option<ZcodeSubscription>,
}

static CACHE: OnceLock<Mutex<Option<(Instant, RemoteSnapshot)>>> = OnceLock::new();

fn cache() -> &'static Mutex<Option<(Instant, RemoteSnapshot)>> {
    CACHE.get_or_init(|| Mutex::new(None))
}

// ─── Raw API shapes (deserialization only) ───────────────────────────────────

#[derive(Debug, Deserialize)]
struct QuotaEnvelope {
    #[serde(default)]
    code: Option<i64>,
    #[serde(default)]
    success: Option<bool>,
    #[serde(default)]
    msg: Option<String>,
    data: Option<QuotaData>,
}

impl QuotaEnvelope {
    /// Mirror the ZCode app's envelope check:
    /// `success !== false && (code == null || code === 0 || code === 200)`.
    fn is_success(&self) -> bool {
        self.success != Some(false)
            && matches!(self.code, None | Some(0) | Some(200))
    }
}

#[derive(Debug, Deserialize)]
struct QuotaData {
    #[serde(default)]
    level: Option<String>,
    #[serde(default)]
    limits: Vec<RawLimit>,
}

#[derive(Debug, Deserialize)]
struct RawLimit {
    #[serde(rename = "type", default)]
    kind: Option<String>,
    #[serde(default)]
    unit: Option<f64>,
    #[serde(default)]
    number: Option<f64>,
    #[serde(default)]
    usage: Option<f64>,
    #[serde(default, rename = "currentValue")]
    current_value: Option<f64>,
    #[serde(default)]
    remaining: Option<f64>,
    #[serde(default)]
    percentage: Option<f64>,
    /// Epoch milliseconds (optional; converted to RFC 3339).
    #[serde(default, rename = "nextResetTime")]
    next_reset_time_ms: Option<f64>,
    #[serde(default, rename = "usageDetails")]
    usage_details: Vec<RawUsageDetail>,
}

#[derive(Debug, Deserialize)]
struct RawUsageDetail {
    #[serde(default, rename = "modelCode")]
    model_code: Option<String>,
    #[serde(default, rename = "displayName")]
    display_name: Option<String>,
    #[serde(default)]
    usage: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct SubscriptionEnvelope {
    #[serde(default)]
    code: Option<i64>,
    #[serde(default)]
    success: Option<bool>,
    #[serde(default)]
    data: Option<Vec<RawSubscription>>,
}

#[derive(Debug, Deserialize)]
struct RawSubscription {
    #[serde(default, rename = "productName")]
    product_name: Option<String>,
    #[serde(default, rename = "billingCycle")]
    billing_cycle: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default, rename = "inCurrentPeriod")]
    in_current_period: Option<bool>,
    /// Term range, e.g. `"2026-09-12 10:29:31-2026-12-12 10:00:00"`.
    #[serde(default)]
    valid: Option<String>,
    /// "YYYY-MM-DD" next renewal date.
    #[serde(default, rename = "nextRenewTime")]
    next_renew_time: Option<String>,
    /// Numeric boolean (`1`/`0`) in the live API.
    #[serde(default, rename = "autoRenew")]
    auto_renew: Option<i64>,
}

// ─── Entry point ─────────────────────────────────────────────────────────────

/// Fetch ZCode (BigModel GLM coding plan) quota plus local usage aggregation.
pub async fn fetch_zcode_quota(
    client: &Client,
    zcode_records: &[TokenRecord],
) -> ZcodeQuotaStatus {
    let local = LocalUsage::from_records(zcode_records);

    // 1. Credentials — without a key the remote half cannot work, but the
    //    local usage half still can (the dashboard reads the same install).
    let creds = resolve_credentials();
    let remote = match creds {
        Ok((key, quota_url)) => fetch_remote_cached(client, &key, &quota_url).await,
        Err(e) => Err(e),
    };

    match remote {
        Ok(snap) => ZcodeQuotaStatus {
            available: true,
            data: Some(build_data(Some(snap), &local, None)),
            error: None,
        },
        Err(e) => {
            warn!("ZCode quota fetch failed: {e}");
            if zcode_records.is_empty() {
                ZcodeQuotaStatus {
                    available: false,
                    data: None,
                    error: Some(e),
                }
            } else {
                // Degrade gracefully: keep the usage half visible and carry
                // the remote failure in `quota_error`.
                ZcodeQuotaStatus {
                    available: true,
                    data: Some(build_data(None, &local, Some(e))),
                    error: None,
                }
            }
        }
    }
}

fn build_data(
    remote: Option<RemoteSnapshot>,
    local: &LocalUsage,
    quota_error: Option<String>,
) -> ZcodeQuotaData {
    let (plan_level, limits, subscription) = match remote {
        Some(snap) => (snap.level, snap.limits, snap.subscription),
        None => (None, Vec::new(), None),
    };
    ZcodeQuotaData {
        plan_level,
        limits,
        subscription,
        today_calls: local.today_calls,
        today_input_tokens: local.today_input,
        today_output_tokens: local.today_output,
        today_cache_read_tokens: local.today_cache_read,
        today_cache_write_tokens: local.today_cache_write,
        today_total_tokens: local.today_total,
        today_cost_cny: local.today_cost_cny,
        total_calls: local.total_calls,
        total_input_tokens: local.total_input,
        total_output_tokens: local.total_output,
        total_cache_read_tokens: local.total_cache_read,
        total_cache_write_tokens: local.total_cache_write,
        total_tokens: local.total_total,
        total_cost_cny: local.total_cost_cny,
        quota_error,
    }
}

// ─── Remote fetching ─────────────────────────────────────────────────────────

async fn fetch_remote_cached(client: &Client, key: &str, quota_url: &str) -> Result<RemoteSnapshot, String> {
    if let Some((at, snap)) = cache().lock().ok().and_then(|guard| guard.clone()) {
        if at.elapsed() < Duration::from_secs(CACHE_TTL_SECS) {
            return Ok(snap);
        }
    }

    let snap = fetch_remote(client, key, quota_url).await?;
    if let Ok(mut guard) = cache().lock() {
        *guard = Some((Instant::now(), snap.clone()));
    }
    Ok(snap)
}

async fn fetch_remote(client: &Client, key: &str, quota_url: &str) -> Result<RemoteSnapshot, String> {
    let (level, limits) = fetch_quota_limits(client, key, quota_url).await?;
    // Subscription info is a nice-to-have; failure must not sink the card.
    let subscription = match fetch_subscription(client, key).await {
        Ok(sub) => sub,
        Err(e) => {
            info!("ZCode subscription fetch failed (non-fatal): {e}");
            None
        }
    };
    Ok(RemoteSnapshot {
        level,
        limits,
        subscription,
    })
}

/// GET the quota/limit endpoint. Returns `(plan_level, limits)`; a
/// "no coding plan" response maps to `(None, [])` rather than an error.
async fn fetch_quota_limits(
    client: &Client,
    key: &str,
    quota_url: &str,
) -> Result<(Option<String>, Vec<ZcodeLimitEntry>), String> {
    let resp = client
        .get(quota_url)
        .header("authorization", key)
        .timeout(Duration::from_secs(HTTP_TIMEOUT_SECS))
        .send()
        .await
        .map_err(|e| format!("network error: {e}"))?;

    let status = resp.status();
    let body = resp
        .text()
        .await
        .map_err(|e| format!("response read error: {e}"))?;
    if !status.is_success() {
        return Err(format!(
            "GET {quota_url}: HTTP {status}: {}",
            super::truncate_error_body(&body)
        ));
    }

    let envelope: QuotaEnvelope =
        serde_json::from_str(&body).map_err(|e| format!("parse error: {e}"))?;
    if !envelope.is_success() {
        let msg = envelope.msg.clone().unwrap_or_default();
        // "不存在coding plan" / "没有资格" — account without an entitlement.
        if msg.contains("不存在coding plan") || msg.contains("没有资格") {
            return Ok((None, Vec::new()));
        }
        return Err(format!(
            "quota API error (code {:?}): {}",
            envelope.code,
            super::truncate_error_body(&msg)
        ));
    }

    let data = envelope.data.unwrap_or(QuotaData {
        level: None,
        limits: Vec::new(),
    });
    let limits = data
        .limits
        .into_iter()
        .map(|raw| ZcodeLimitEntry {
            kind: raw.kind,
            unit: raw.unit,
            number: raw.number,
            usage: raw.usage,
            current_value: raw.current_value,
            remaining: raw.remaining,
            percentage: raw.percentage,
            next_reset_time: raw.next_reset_time_ms.and_then(epoch_ms_to_rfc3339),
            usage_details: raw
                .usage_details
                .into_iter()
                .map(|d| ZcodeUsageDetail {
                    model_code: d.model_code.unwrap_or_default(),
                    display_name: d.display_name,
                    usage: d.usage.unwrap_or(0.0),
                })
                .collect(),
        })
        .collect();

    Ok((data.level, limits))
}

/// GET the subscription list and pick the current-term entry.
async fn fetch_subscription(client: &Client, key: &str) -> Result<Option<ZcodeSubscription>, String> {
    let resp = client
        .get(SUBSCRIPTION_URL)
        .header("Authorization", key)
        .timeout(Duration::from_secs(HTTP_TIMEOUT_SECS))
        .send()
        .await
        .map_err(|e| format!("network error: {e}"))?;

    let status = resp.status();
    let body = resp
        .text()
        .await
        .map_err(|e| format!("response read error: {e}"))?;
    if !status.is_success() {
        return Err(format!("GET {SUBSCRIPTION_URL}: HTTP {status}"));
    }

    let envelope: SubscriptionEnvelope =
        serde_json::from_str(&body).map_err(|e| format!("parse error: {e}"))?;
    if envelope.success == Some(false) || !matches!(envelope.code, None | Some(0) | Some(200)) {
        return Err(format!("subscription API error (code {:?})", envelope.code));
    }

    let rows = envelope.data.unwrap_or_default();
    let picked = rows
        .iter()
        .find(|r| r.in_current_period == Some(true) && r.status.as_deref() == Some("VALID"))
        .or_else(|| rows.iter().find(|r| r.status.as_deref() == Some("VALID")))
        .or_else(|| rows.first());

    Ok(picked.map(|r| ZcodeSubscription {
        product_name: r.product_name.clone(),
        billing_cycle: r.billing_cycle.clone(),
        next_renew_time: r.next_renew_time.clone(),
        expire_time: r.valid.as_deref().and_then(valid_range_end),
        auto_renew: r.auto_renew.unwrap_or(0) > 0,
    }))
}

// ─── Credential resolution ───────────────────────────────────────────────────

/// Resolve `(api_key, quota_url)`. The key comes from the env override or the
/// ZCode desktop app config; the URL from the env override or the default
/// BigModel monitor endpoint.
fn resolve_credentials() -> Result<(String, String), String> {
    let key = match std::env::var("ZCODE_BIGMODEL_USAGE_API_KEY") {
        Ok(k) if !k.trim().is_empty() => k.trim().to_string(),
        _ => key_from_config()?,
    };
    let url = std::env::var("ZCODE_BIGMODEL_USAGE_QUOTA_URL")
        .ok()
        .filter(|u| !u.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_QUOTA_URL.to_string());
    Ok((key, url))
}

/// Read the coding-plan API key from the ZCode desktop app config
/// (`~/.zcode/v2/config.json`, `ZCODE_CONFIG_PATH` overrides).
fn key_from_config() -> Result<String, String> {
    let path = std::env::var("ZCODE_CONFIG_PATH")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            dirs_or_home().join(".zcode").join("v2").join("config.json")
        });
    let text = std::fs::read_to_string(&path)
        .map_err(|e| format!("ZCode 配置读取失败 ({}): {e}", path.display()))?;
    let cfg: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("ZCode 配置解析失败: {e}"))?;
    let key = cfg
        .get("provider")
        .and_then(|p| p.get(PROVIDER_ID))
        .and_then(|p| p.get("options"))
        .and_then(|o| o.get("apiKey"))
        .and_then(|k| k.as_str())
        .map(str::trim)
        .filter(|k| !k.is_empty())
        .ok_or_else(|| format!("ZCode 配置中未找到 {PROVIDER_ID} 的 apiKey（应用需登录过 BigModel 编码套餐）"))?;
    Ok(key.to_string())
}

fn dirs_or_home() -> std::path::PathBuf {
    std::env::var("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn epoch_ms_to_rfc3339(ms: f64) -> Option<String> {
    let secs = (ms / 1000.0).floor() as i64;
    let nanos = ((ms - secs as f64 * 1000.0).round() as u32) * 1_000_000;
    chrono::Utc
        .timestamp_opt(secs, nanos)
        .single()
        .map(|dt| dt.to_rfc3339())
}

/// Extract the term end from the `valid` range
/// `"2026-09-12 10:29:31-2026-12-12 10:00:00"` → `2026-12-12 10:00:00`.
/// The separator is the `-` that splits the range into two halves; times use
/// colons, so splitting on `-` and re-joining each half works.
fn valid_range_end(valid: &str) -> Option<String> {
    let parts: Vec<&str> = valid.split('-').collect();
    if parts.len() < 2 || parts.len() % 2 != 0 {
        return None;
    }
    let half = parts.len() / 2;
    Some(parts[half..].join("-"))
}

// ─── Local usage aggregation ─────────────────────────────────────────────────

struct LocalUsage {
    today_calls: i64,
    today_input: i64,
    today_output: i64,
    today_cache_read: i64,
    today_cache_write: i64,
    today_total: i64,
    today_cost_cny: f64,
    total_calls: i64,
    total_input: i64,
    total_output: i64,
    total_cache_read: i64,
    total_cache_write: i64,
    total_total: i64,
    total_cost_cny: f64,
}

impl LocalUsage {
    /// Aggregate the `source="zcode"` snapshot; "today" matches the record's
    /// UTC date (TokenRecord.date is a UTC `YYYY-MM-DD` string). Costs use
    /// the pricing state guard once for the whole loop; unknown-cost (-1)
    /// records are skipped in the sums, same as the aggregator.
    fn from_records(records: &[TokenRecord]) -> Self {
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let ps = pricing::state_read();
        let mut u = Self {
            today_calls: 0,
            today_input: 0,
            today_output: 0,
            today_cache_read: 0,
            today_cache_write: 0,
            today_total: 0,
            today_cost_cny: 0.0,
            total_calls: 0,
            total_input: 0,
            total_output: 0,
            total_cache_read: 0,
            total_cache_write: 0,
            total_total: 0,
            total_cost_cny: 0.0,
        };
        for r in records {
            u.total_calls += 1;
            u.total_input += r.input_tokens;
            u.total_output += r.output_tokens;
            u.total_cache_read += r.cache_read_tokens;
            u.total_cache_write += r.cache_write_tokens;
            u.total_total += r.total_tokens;
            let cost = pricing::display_cost_in(&ps, r);
            if cost > 0.0 {
                u.total_cost_cny += cost;
            }
            if r.date == today {
                u.today_calls += 1;
                u.today_input += r.input_tokens;
                u.today_output += r.output_tokens;
                u.today_cache_read += r.cache_read_tokens;
                u.today_cache_write += r.cache_write_tokens;
                u.today_total += r.total_tokens;
                if cost > 0.0 {
                    u.today_cost_cny += cost;
                }
            }
        }
        u
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_live_quota_envelope() {
        // Shape captured from the live endpoint (2026-09-12).
        let body = r#"{"code":200,"msg":"操作成功","data":{"limits":[
            {"type":"CREDIT_LIMIT","unit":3,"number":5,"usage":2000,"currentValue":161,
             "remaining":1838,"percentage":8,"nextResetTime":1789198447629},
            {"type":"CREDIT_LIMIT","unit":6,"number":1,"usage":10000,"currentValue":161,
             "remaining":9838,"percentage":1,"nextResetTime":1789784970997}],
            "level":"lite"},"success":true}"#;
        let envelope: QuotaEnvelope = serde_json::from_str(body).unwrap();
        assert!(envelope.is_success());
        let data = envelope.data.unwrap();
        assert_eq!(data.level.as_deref(), Some("lite"));
        assert_eq!(data.limits.len(), 2);
        let first = &data.limits[0];
        assert_eq!(first.kind.as_deref(), Some("CREDIT_LIMIT"));
        assert_eq!(first.usage, Some(2000.0));
        assert_eq!(first.remaining, Some(1838.0));
        assert_eq!(first.number, Some(5.0));
        // 1789198447629 ms → 2026-09-12T07:34:07.629Z (UTC); the plan was
        // purchased 2026-09-12 10:30 CST, so this reset is purchase+5h —
        // the classic 5-hour coding-plan window.
        let reset = epoch_ms_to_rfc3339(first.next_reset_time_ms.unwrap()).unwrap();
        assert!(reset.starts_with("2026-09-12T07:34:07"), "got {reset}");
    }

    #[test]
    fn envelope_failure_shapes() {
        let no_plan: QuotaEnvelope =
            serde_json::from_str(r#"{"code":500,"msg":"当前用户不存在coding plan","success":false}"#)
                .unwrap();
        assert!(!no_plan.is_success());
        let msg = no_plan.msg.unwrap();
        assert!(msg.contains("不存在coding plan"));

        let zero_ok: QuotaEnvelope =
            serde_json::from_str(r#"{"code":0,"msg":"ok","data":null}"#).unwrap();
        assert!(zero_ok.is_success());
    }

    #[test]
    fn picks_current_subscription_and_term_end() {
        let body = r#"{"code":200,"msg":"操作成功","data":[
            {"id":"1","productId":"product-e90ff2","productName":"GLM Coding Lite",
             "status":"VALID","purchaseTime":"2026-09-12 10:30:21",
             "valid":"2026-09-12 10:29:31-2026-12-12 10:00:00","autoRenew":1,
             "nextRenewTime":"2026-12-12","billingCycle":"quarterly",
             "inCurrentPeriod":true},
            {"id":"2","productName":"GLM Coding Lite (old)","status":"EXPIRED",
             "inCurrentPeriod":false,"autoRenew":0,"valid":"2026-06-12 00:00:00-2026-09-12 00:00:00"}
        ],"success":true}"#;
        let envelope: SubscriptionEnvelope = serde_json::from_str(body).unwrap();
        let rows = envelope.data.unwrap();
        let picked = rows
            .iter()
            .find(|r| r.in_current_period == Some(true) && r.status.as_deref() == Some("VALID"))
            .unwrap();
        assert_eq!(picked.product_name.as_deref(), Some("GLM Coding Lite"));
        assert_eq!(picked.auto_renew, Some(1));
        let sub = ZcodeSubscription {
            product_name: picked.product_name.clone(),
            billing_cycle: picked.billing_cycle.clone(),
            next_renew_time: picked.next_renew_time.clone(),
            expire_time: picked.valid.as_deref().and_then(valid_range_end),
            auto_renew: picked.auto_renew.unwrap_or(0) > 0,
        };
        assert_eq!(sub.expire_time.as_deref(), Some("2026-12-12 10:00:00"));
        assert!(sub.auto_renew);
        assert_eq!(sub.next_renew_time.as_deref(), Some("2026-12-12"));
    }

    #[test]
    fn valid_range_end_handles_plain_dates() {
        assert_eq!(
            valid_range_end("2026-09-12-2026-12-12").as_deref(),
            Some("2026-12-12")
        );
        assert_eq!(valid_range_end("garbage"), None);
    }

    #[test]
    fn local_usage_aggregates_today_and_totals() {
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let mk = |date: &str, input: i64, output: i64| TokenRecord {
            date: date.to_string(),
            time: format!("{date}T00:00:00Z"),
            api_key_prefix: "N/A".to_string(),
            provider: "bigmodel".to_string(),
            original_provider: None,
            model: "GLM-5.3-Flash".to_string(),
            source: "zcode".to_string(),
            input_tokens: input,
            output_tokens: output,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            total_tokens: input + output,
            cost: 0.0,
            ttft_ms: None,
            tps: None,
        };
        let records = vec![
            mk(&today, 100, 20),
            mk(&today, 50, 10),
            mk("2020-01-01", 7, 3),
        ];
        let usage = LocalUsage::from_records(&records);
        assert_eq!(usage.today_calls, 2);
        assert_eq!(usage.today_input, 150);
        assert_eq!(usage.today_output, 30);
        assert_eq!(usage.total_calls, 3);
        assert_eq!(usage.total_input, 157);
    }

    #[test]
    fn resolve_credentials_prefers_env_key() {
        temp_env::with_var("ZCODE_BIGMODEL_USAGE_API_KEY", Some("env-key"), || {
            temp_env::with_var("ZCODE_BIGMODEL_USAGE_QUOTA_URL", None::<&str>, || {
                let (key, url) = resolve_credentials().unwrap();
                assert_eq!(key, "env-key");
                assert_eq!(url, DEFAULT_QUOTA_URL);
            });
        });
    }

    #[test]
    fn key_from_config_reads_provider_options() {
        let dir = tempfile::tempdir().unwrap();
        let cfg_path = dir.path().join("config.json");
        std::fs::write(
            &cfg_path,
            format!(
                r#"{{"provider":{{"{PROVIDER_ID}":{{"kind":"anthropic","options":{{"baseURL":"https://open.bigmodel.cn/api/anthropic","apiKey":"cfg-key-123"}}}}}}}}"#
            ),
        )
        .unwrap();
        temp_env::with_var("ZCODE_CONFIG_PATH", Some(cfg_path.to_str().unwrap()), || {
            temp_env::with_var("ZCODE_BIGMODEL_USAGE_API_KEY", None::<&str>, || {
                let (key, _) = resolve_credentials().unwrap();
                assert_eq!(key, "cfg-key-123");
            });
        });
    }

    #[test]
    fn key_from_config_missing_key_errors() {
        let dir = tempfile::tempdir().unwrap();
        let cfg_path = dir.path().join("config.json");
        std::fs::write(&cfg_path, r#"{"provider":{}}"#).unwrap();
        temp_env::with_var("ZCODE_CONFIG_PATH", Some(cfg_path.to_str().unwrap()), || {
            temp_env::with_var("ZCODE_BIGMODEL_USAGE_API_KEY", None::<&str>, || {
                assert!(resolve_credentials().is_err());
            });
        });
    }
}
