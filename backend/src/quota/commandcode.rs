//! CommandCode provider integration.
//!
//! Fetches subscription/quota data from the CommandCode platform API
//! (https://api.commandcode.ai) for each locally authenticated account.
//!
//! Accounts are discovered from `~/.commandcode/auth*.json` (the same files
//! written by the CommandCode CLI on login). Each account's `apiKey` is sent
//! as a Bearer token on the `/alpha/*` routes:
//! - `/alpha/billing/subscriptions` — plan, status, renewal date
//! - `/alpha/billing/credits` — remaining monthly credits + window limits
//!   (rolling 5h and weekly caps with used amount and reset time)
//! - `/alpha/usage/summary` — consumed usage so far this billing period
//!
//! Fallback: when no `auth.json` exists, `COMMANDCODE_SESSION_TOKEN` is used
//! as the session cookie on the legacy `/internal/*` routes.

use super::types::*;
use reqwest::Client;
use serde::{de, Deserialize, Deserializer};
use std::fmt;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tracing::{info, warn};

// ─── Constants ───────────────────────────────────────────────────────────────

const COMMANDCODE_API_BASE: &str = "https://api.commandcode.ai";
const COMMANDCODE_TIMEOUT_SECS: u64 = 15;

// ─── Account discovery ───────────────────────────────────────────────────────

/// A CommandCode CLI account discovered from `~/.commandcode/auth*.json`.
#[derive(Debug, Clone)]
pub struct CommandCodeAccount {
    pub api_key: String,
    pub user_name: String,
    pub user_id: String,
}

fn commandcode_dir() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    std::path::PathBuf::from(home).join(".commandcode")
}

fn read_auth_file(path: &std::path::Path) -> Option<CommandCodeAccount> {
    let data = std::fs::read_to_string(path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&data).ok()?;
    let api_key = value.get("apiKey")?.as_str()?.to_string();
    if api_key.is_empty() {
        return None;
    }
    Some(CommandCodeAccount {
        api_key,
        user_name: value
            .get("userName")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        user_id: value
            .get("userId")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
    })
}

/// Read the primary account (`auth.json`) plus extra accounts
/// (`auth*.json`, e.g. `auth_frank.json`). Env-specific files
/// (`auth.local.json` / `auth.staging.json`) are skipped.
///
/// Results are cached for [`ACCOUNTS_CACHE_TTL`] keyed by the resolved
/// `~/.commandcode` directory, so the per-poll calls (primary + EX) and
/// repeated `/api/quota` polls don't re-read the auth files from disk every
/// time. The directory is part of the key so a `HOME` change (e.g. in tests)
/// always re-reads.
pub fn load_accounts() -> (Option<CommandCodeAccount>, Vec<CommandCodeAccount>) {
    let dir = commandcode_dir();
    let now = Instant::now();
    {
        let guard = ACCOUNTS_CACHE.lock().unwrap();
        if let Some(cache) = guard.as_ref() {
            if cache.dir.as_deref() == Some(dir.as_path())
                && now.duration_since(cache.fetched_at) < ACCOUNTS_CACHE_TTL
            {
                return cache.value.clone();
            }
        }
    }
    let value = load_accounts_inner();
    *ACCOUNTS_CACHE.lock().unwrap() = Some(AccountsCache {
        dir: Some(dir),
        fetched_at: now,
        value: value.clone(),
    });
    value
}

const ACCOUNTS_CACHE_TTL: Duration = Duration::from_secs(30);

struct AccountsCache {
    dir: Option<std::path::PathBuf>,
    fetched_at: Instant,
    value: (Option<CommandCodeAccount>, Vec<CommandCodeAccount>),
}

static ACCOUNTS_CACHE: Mutex<Option<AccountsCache>> = Mutex::new(None);

fn load_accounts_inner() -> (Option<CommandCodeAccount>, Vec<CommandCodeAccount>) {
    let dir = commandcode_dir();
    let primary = read_auth_file(&dir.join("auth.json"));

    let mut extras = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        let mut paths: Vec<std::path::PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.is_file()
                    && p.file_name()
                        .and_then(|n| n.to_str())
                        .map(|n| {
                            n.starts_with("auth")
                                && n.ends_with(".json")
                                && n != "auth.json"
                                && n != "auth.local.json"
                                && n != "auth.staging.json"
                        })
                        .unwrap_or(false)
            })
            .collect();
        paths.sort();
        for path in paths {
            if let Some(account) = read_auth_file(&path) {
                extras.push(account);
            }
        }
    }
    (primary, extras)
}

// ─── Auth helpers ────────────────────────────────────────────────────────────

/// Read `COMMANDCODE_SESSION_TOKEN` from environment (legacy fallback).
pub fn get_session_token() -> Option<String> {
    std::env::var("COMMANDCODE_SESSION_TOKEN")
        .ok()
        .filter(|t| !t.is_empty())
}

#[derive(Clone)]
enum Auth {
    /// CLI apiKey from an auth file — `/alpha/*` routes, Bearer auth.
    Bearer(String),
    /// Legacy session cookie — `/internal/*` routes.
    SessionCookie(String),
}

impl Auth {
    fn apply(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match self {
            Auth::Bearer(key) => builder.header("Authorization", format!("Bearer {}", key)),
            Auth::SessionCookie(token) => builder.header(
                "Cookie",
                format!("__Secure-commandcode_prod_.session_token={}", token),
            ),
        }
    }

    fn route(&self, path: &str) -> String {
        let prefix = match self {
            Auth::Bearer(_) => "/alpha",
            Auth::SessionCookie(_) => "/internal",
        };
        format!("{}{}", prefix, path)
    }
}

// ─── API response types ──────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreditsResponse {
    #[serde(default)]
    credits: Option<CreditsData>,
    #[serde(default)]
    window_limits: Option<WindowLimits>,
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
#[serde(rename_all = "camelCase")]
struct WindowLimits {
    #[serde(default)]
    five_hour: Option<WindowLimitEntry>,
    #[serde(default)]
    weekly: Option<WindowLimitEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WindowLimitEntry {
    #[serde(default)]
    used: f64,
    #[serde(default)]
    cap: f64,
    /// Epoch milliseconds at which the window resets (0 = none/unknown).
    #[serde(default)]
    reset_at: i64,
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

/// Deserialize a number that may be encoded as a JSON string.
/// CommandCode API returns some numeric fields (like token counts) as strings.
fn deserialize_number_or_string<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<i64, D::Error> {
    struct NumberOrString;
    impl<'de> de::Visitor<'de> for NumberOrString {
        type Value = i64;
        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("a number or string containing a number")
        }
        fn visit_u64<E: de::Error>(self, v: u64) -> Result<i64, E> {
            i64::try_from(v).map_err(de::Error::custom)
        }
        fn visit_i64<E: de::Error>(self, v: i64) -> Result<i64, E> {
            Ok(v)
        }
        fn visit_str<E: de::Error>(self, v: &str) -> Result<i64, E> {
            v.parse().map_err(de::Error::custom)
        }
    }
    deserializer.deserialize_any(NumberOrString)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UsageSummaryResponse {
    #[serde(default)]
    total_cost: f64,
    #[serde(default)]
    total_count: i64,
    #[serde(default, deserialize_with = "deserialize_number_or_string")]
    total_tokens: i64,
    #[serde(default, deserialize_with = "deserialize_number_or_string")]
    total_tokens_in: i64,
    #[serde(default, deserialize_with = "deserialize_number_or_string")]
    total_tokens_out: i64,
}

fn epoch_ms_to_rfc3339(ms: i64) -> Option<String> {
    if ms <= 0 {
        return None;
    }
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(ms)
        .map(|dt| dt.to_rfc3339())
}

// ─── Fetch ───────────────────────────────────────────────────────────────────

/// Fetch quota for the primary account (auth.json) using its apiKey.
/// Falls back to the legacy `COMMANDCODE_SESSION_TOKEN` cookie when no
/// auth file exists.
pub async fn fetch_commandcode_quota(client: &Client) -> CommandCodeQuotaStatus {
    let (primary, _extras) = load_accounts();
    if let Some(account) = primary {
        fetch_quota_with(
            client,
            Auth::Bearer(account.api_key),
            account.user_name,
            account.user_id,
        )
        .await
    } else if let Some(token) = get_session_token() {
        fetch_quota_with(client, Auth::SessionCookie(token), String::new(), String::new()).await
    } else {
        warn!("no CommandCode auth file and COMMANDCODE_SESSION_TOKEN not set");
        CommandCodeQuotaStatus {
            available: false,
            data: None,
            error: Some("未找到 CommandCode 账号（~/.commandcode/auth.json）".to_string()),
        }
    }
}

/// Fetch quota for the second account (first extra `auth*.json` file,
/// e.g. `auth_frank.json`).
pub async fn fetch_commandcode_quota_ex(client: &Client) -> CommandCodeQuotaStatus {
    let (_primary, extras) = load_accounts();
    match extras.into_iter().next() {
        Some(account) => {
            fetch_quota_with(
                client,
                Auth::Bearer(account.api_key),
                account.user_name,
                account.user_id,
            )
            .await
        }
        None => CommandCodeQuotaStatus {
            available: false,
            data: None,
            error: Some("未找到第二个 CommandCode 账号（~/.commandcode/auth*.json）".to_string()),
        },
    }
}

async fn fetch_quota_with(
    client: &Client,
    auth: Auth,
    user_name: String,
    user_id: String,
) -> CommandCodeQuotaStatus {
    // Run all three requests in parallel
    let (credits_result, subscription_result, usage_result) = tokio::join!(
        fetch_credits(client, &auth),
        fetch_subscription(client, &auth),
        fetch_usage_summary(client, &auth),
    );

    // If all failed, treat as unavailable
    if credits_result.is_none() && subscription_result.is_none() {
        return CommandCodeQuotaStatus {
            available: false,
            data: None,
            error: Some("所有 CommandCode API 请求失败".to_string()),
        };
    }

    let credits = credits_result;
    let sub = subscription_result;
    let usage = usage_result;

    // Compute monthly credits total: remaining + already used
    let monthly_used = usage.as_ref().map_or(0.0, |u| u.total_cost);
    let monthly_remaining = credits
        .as_ref()
        .and_then(|c| c.credits.as_ref())
        .map_or(0.0, |c| c.monthly_credits);
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

    let five_hour = credits
        .as_ref()
        .and_then(|c| c.window_limits.as_ref())
        .and_then(|w| w.five_hour.as_ref())
        .map(|w| CommandCodeWindowLimit {
            used: w.used,
            cap: w.cap,
            reset_at: epoch_ms_to_rfc3339(w.reset_at),
        });
    let weekly = credits
        .as_ref()
        .and_then(|c| c.window_limits.as_ref())
        .and_then(|w| w.weekly.as_ref())
        .map(|w| CommandCodeWindowLimit {
            used: w.used,
            cap: w.cap,
            reset_at: epoch_ms_to_rfc3339(w.reset_at),
        });

    let data = CommandCodeQuotaData {
        plan_name: plan_name.to_string(),
        subscription_status: sub
            .as_ref()
            .and_then(|s| s.status.as_deref())
            .map(|s| s.to_string())
            .unwrap_or_else(|| "unknown".to_string()),
        cancel_at_period_end: sub.as_ref().and_then(|s| s.cancel_at_period_end),
        user_name,
        user_id,
        monthly_credits_total: monthly_total,
        monthly_credits_used: monthly_used,
        monthly_credits_remaining: monthly_remaining,
        purchased_credits: credits
            .as_ref()
            .and_then(|c| c.credits.as_ref())
            .map_or(0.0, |c| c.purchased_credits),
        premium_monthly_credits: credits
            .as_ref()
            .and_then(|c| c.credits.as_ref())
            .map_or(0.0, |c| c.premium_monthly_credits),
        opensource_monthly_credits: credits
            .as_ref()
            .and_then(|c| c.credits.as_ref())
            .map_or(0.0, |c| c.opensource_monthly_credits),
        current_period_end: sub.as_ref().and_then(|s| s.current_period_end.clone()),
        total_requests: usage.as_ref().map_or(0, |u| u.total_count),
        total_tokens: usage.as_ref().map_or(0, |u| u.total_tokens),
        total_tokens_in: usage.as_ref().map_or(0, |u| u.total_tokens_in),
        total_tokens_out: usage.as_ref().map_or(0, |u| u.total_tokens_out),
        five_hour,
        weekly,
    };

    info!(
        "CommandCode quota fetched: user={}, plan={}, status={}, monthly used={:.4}/{:.4}",
        data.user_name,
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

async fn fetch_credits(client: &Client, auth: &Auth) -> Option<CreditsResponse> {
    let url = format!("{}{}", COMMANDCODE_API_BASE, auth.route("/billing/credits"));
    match auth
        .apply(client.get(&url))
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
                Ok(data) => Some(data),
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

async fn fetch_subscription(client: &Client, auth: &Auth) -> Option<SubscriptionData> {
    let url = format!(
        "{}{}",
        COMMANDCODE_API_BASE,
        auth.route("/billing/subscriptions")
    );
    match auth
        .apply(client.get(&url))
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

async fn fetch_usage_summary(client: &Client, auth: &Auth) -> Option<UsageSummaryResponse> {
    let url = format!("{}{}", COMMANDCODE_API_BASE, auth.route("/usage/summary"));
    match auth
        .apply(client.get(&url))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserialize_usage_summary_with_string_tokens() {
        let json = r#"{
            "totalCount": 851,
            "totalCost": 1.7591,
            "totalTokensIn": "64973237",
            "totalTokensOut": "585408",
            "totalTokens": "65558645"
        }"#;
        let parsed: UsageSummaryResponse = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.total_count, 851);
        assert_eq!(parsed.total_tokens, 65558645);
        assert_eq!(parsed.total_tokens_in, 64973237);
        assert_eq!(parsed.total_tokens_out, 585408);
    }

    #[test]
    fn deserialize_usage_summary_with_int_tokens() {
        let json = r#"{
            "totalCount": 10,
            "totalCost": 0.5,
            "totalTokensIn": 1000,
            "totalTokensOut": 500,
            "totalTokens": 1500
        }"#;
        let parsed: UsageSummaryResponse = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.total_count, 10);
        assert_eq!(parsed.total_tokens, 1500);
        assert_eq!(parsed.total_tokens_in, 1000);
        assert_eq!(parsed.total_tokens_out, 500);
    }

    #[test]
    fn deserialize_usage_summary_missing_fields() {
        let json = r#"{} "#;
        let parsed: UsageSummaryResponse = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.total_count, 0);
        assert_eq!(parsed.total_tokens, 0);
        assert_eq!(parsed.total_tokens_in, 0);
        assert_eq!(parsed.total_tokens_out, 0);
    }

    #[test]
    fn deserialize_credits_with_window_limits() {
        let json = r#"{
            "credits": {
                "belowThreshold": false,
                "creditThreshold": 0,
                "monthlyCredits": 9.921871092,
                "purchasedCredits": 0,
                "freeCredits": 0
            },
            "windowLimits": {
                "limited": true,
                "exceeded": null,
                "fiveHour": {"used": 0.078128908, "cap": 3, "exceeded": false, "resetAt": 1788090130325},
                "weekly": {"used": 0.078128908, "cap": 6, "exceeded": false, "resetAt": 1788676930325}
            }
        }"#;
        let parsed: CreditsResponse = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.credits.unwrap().monthly_credits, 9.921871092);
        let limits = parsed.window_limits.unwrap();
        let fh = limits.five_hour.unwrap();
        assert_eq!(fh.cap, 3.0);
        assert_eq!(fh.used, 0.078128908);
        assert!(fh.reset_at > 0);
        let weekly = limits.weekly.unwrap();
        assert_eq!(weekly.cap, 6.0);
        assert_eq!(
            epoch_ms_to_rfc3339(weekly.reset_at).unwrap(),
            "2026-09-06T06:42:10.325+00:00"
        );
    }

    #[test]
    fn epoch_ms_zero_means_no_reset() {
        assert!(epoch_ms_to_rfc3339(0).is_none());
        assert!(epoch_ms_to_rfc3339(-1).is_none());
    }

    #[test]
    fn discovers_accounts_from_auth_files() {
        let dir = tempfile::tempdir().unwrap();
        let cc_dir = dir.path().join(".commandcode");
        std::fs::create_dir_all(&cc_dir).unwrap();
        std::fs::write(
            cc_dir.join("auth.json"),
            r#"{"apiKey":"key-main","userId":"u1","userName":"alice","keyName":"cli-a","authenticatedAt":"2026-08-30T06:39:41.970Z"}"#,
        )
        .unwrap();
        std::fs::write(
            cc_dir.join("auth_frank.json"),
            r#"{"apiKey":"key-second","userId":"u2","userName":"bob","keyName":"cli-b","authenticatedAt":"2026-08-18T23:17:33.837Z"}"#,
        )
        .unwrap();
        // Env-specific files must be ignored.
        std::fs::write(cc_dir.join("auth.local.json"), r#"{"apiKey":"key-local"}"#).unwrap();

        temp_env::with_var("HOME", Some(dir.path().to_str().unwrap()), || {
            let (primary, extras) = load_accounts();
            let primary = primary.expect("primary account");
            assert_eq!(primary.user_name, "alice");
            assert_eq!(primary.api_key, "key-main");
            assert_eq!(extras.len(), 1, "only auth_frank.json should be extra");
            assert_eq!(extras[0].user_name, "bob");
            assert_eq!(extras[0].api_key, "key-second");
        });
    }

    #[test]
    fn auth_route_prefixes() {
        let bearer = Auth::Bearer("k".to_string());
        assert_eq!(bearer.route("/billing/credits"), "/alpha/billing/credits");
        let cookie = Auth::SessionCookie("t".to_string());
        assert_eq!(cookie.route("/billing/credits"), "/internal/billing/credits");
    }
}
