//! Ollama cloud subscription/quota fetcher.
//!
//! Primary path: the JSON API (`POST /api/me` + `GET /api/usage`) with
//! `OLLAMA_API_KEY` — the same key the CLIProxyAPI `ollama-cloud` upstream
//! uses, so the plan/limits always describe the account that serves the
//! metered traffic.
//!
//! The JSON API **used to** report session/weekly usage as fractions of a
//! dollar budget (`limits.session.usage = 0.119` → 11.9% of a $10 session
//! window, `limits.weekly.usage` → a $60 weekly window) but never carried
//! reset timestamps; since 2026-10-07 it returns per-day request buckets
//! instead, so the percentages come from the settings page too (API values win
//! whenever they are present).
//! The web UI (`/settings`) has both — one `.local-time[data-time]`
//! element per meter — and is the authoritative source: the weekly window is
//! calendar-aligned (observed: Monday 00:00 UTC), *not* a per-account 7-day
//! grid anchored on the first request, so deriving it from request history
//! drifts for days. Reset times are therefore resolved via
//! [`update_window_state`]:
//!
//! 1. scrape `/settings` with `OLLAMA_AUTH_COOKIE` (same account as the API
//!    key — page and API report identical per-model breakdowns) and take the
//!    displayed reset/resume times verbatim;
//! 2. re-anchor the persisted grid phase from them so predictions survive
//!    restarts (`OLLAMA_WINDOW_STATE_PATH`);
//! 3. fall back to the phase model only when the page is unavailable —
//!    bootstrap from the caller's metered `ollama-proxy` records (`/api/usage`
//!    reports how many requests the live window counted, and essentially all
//!    of them are in our log, so the window's first request is `count` records
//!    back), re-anchored whenever a usage sample drops sharply between two
//!    fresh polls (that drop *is* a window roll-over).
//!
//! The same records give the card its **actual** weekly token/cost totals
//! instead of the old percentage × empirical-quota estimate.
//!
//! Fallback path: scrape `/settings` + `/settings/billing` with
//! `OLLAMA_AUTH_COOKIE` (full Cookie header value) when no API key is set or
//! the API call fails.
//!
//! NOTE: when that cookie lapses ollama.com answers with `303 → /signin` and a
//! 200 login page, which the parsers used to accept as an empty-but-successful
//! fetch — see `auth_redirect_error()`. Re-login at ollama.com and update
//! `OLLAMA_AUTH_COOKIE` (currently maintained in `~/.bash_env`).

use super::types::*;
use crate::models::TokenRecord;
use crate::pricing;
use chrono::{DateTime, Datelike, Duration, SecondsFormat, Utc};
use reqwest::{Client, Url};
use scraper::Html;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::LazyLock;
use tracing::{info, warn};

// ─── Constants ───────────────────────────────────────────────────────────────

const OLLAMA_BASE_URL: &str = "https://ollama.com";
const OLLAMA_HOST: &str = "ollama.com";
const OLLAMA_TIMEOUT_SECS: u64 = 15;

// ─── Auth helpers ────────────────────────────────────────────────────────────

/// Read `OLLAMA_AUTH_COOKIE` from environment.
/// Should contain the full Cookie header value, e.g.
/// "aid=...; __Secure-session=..."
pub fn get_auth_cookie() -> Option<String> {
    std::env::var("OLLAMA_AUTH_COOKIE")
        .ok()
        .filter(|c| !c.is_empty())
}

// ─── Fetch functions ─────────────────────────────────────────────────────────

/// Fetch Ollama subscription and usage info.
///
/// Prefers the API-key JSON endpoints (see [`fetch_via_api`]) because they
/// report the plan/limits of the account whose key our traffic actually uses;
/// falls back to the cookie scrape when no key is set or the API call fails.
///
/// `records` is the caller's `source="ollama-proxy"` snapshot — the per-request
/// meter the CLIProxyAPI plugin writes. It is used to derive the session/weekly
/// window boundaries (which the API does not report) and the actual tokens
/// spent in the live weekly window.
pub async fn fetch_ollama_quota(client: &Client, records: &[TokenRecord]) -> OllamaQuotaStatus {
    if let Some(api_key) = get_api_key() {
        match fetch_via_api(client, &api_key, records).await {
            Ok(data) => {
                info!("Ollama quota fetched via api key (plan {})", data.plan_name);
                return OllamaQuotaStatus {
                    available: true,
                    data: Some(data),
                    error: None,
                };
            }
            Err(e) => {
                warn!("Ollama API-key fetch failed: {e}; falling back to cookie scrape")
            }
        }
    }
    fetch_via_cookie(client, records).await
}

/// Legacy path: scrape `/settings` + `/settings/billing` with `OLLAMA_AUTH_COOKIE`.
async fn fetch_via_cookie(client: &Client, records: &[TokenRecord]) -> OllamaQuotaStatus {
    let cookie = match get_auth_cookie() {
        Some(c) => c,
        None => {
            warn!("OLLAMA_AUTH_COOKIE not set");
            return OllamaQuotaStatus {
                available: false,
                data: None,
                error: Some("OLLAMA_AUTH_COOKIE not set".to_string()),
            };
        }
    };

    // Fetch billing and settings pages in parallel
    let (billing_result, settings_result) = tokio::join!(
        fetch_billing_page(client, &cookie),
        fetch_page(client, &cookie, "/settings"),
    );

    let billing_data = match billing_result {
        Ok(data) => data,
        Err(e) => {
            warn!("Ollama billing fetch failed: {e}");
            return OllamaQuotaStatus {
                available: false,
                data: None,
                error: Some(format!("Failed to fetch billing info: {e}")),
            };
        }
    };

    let settings_html = match settings_result {
        Ok(html) => html,
        Err(e) => {
            warn!("Ollama settings fetch failed: {e}");
            String::new() // at least show billing info
        }
    };

    let mut usage_entries = parse_usage_from_html(&settings_html);
    let web = parse_web_reset_times(&settings_html);

    // The HTML carries percentages and — when the UI renders them — the reset
    // timestamps; the phase model only fills entries the page left blank.
    let now = Utc::now();
    let windows = update_window_state(records, &[], now, &web);
    for entry in usage_entries.iter_mut() {
        if entry.reset_time.is_some() {
            continue;
        }
        entry.reset_time = match entry.usage_type.as_str() {
            "Session" => windows.session_reset,
            "Weekly" => windows.weekly_reset,
            _ => None,
        }
        .map(|t| t.to_rfc3339_opts(SecondsFormat::Secs, true));
    }

    let weekly_usage = windows
        .weekly_start
        .map(|start| weekly_actuals(records, start, now));

    let price = billing_data.price.clone();

    info!("Ollama quota fetched (cookie)");

    OllamaQuotaStatus {
        available: true,
        data: Some(OllamaQuotaData {
            plan_name: billing_data.plan_name,
            renews_on: billing_data.renews_on,
            price,
            usage_entries,
            has_annual_option: billing_data.has_annual_option,
            has_max_upgrade: billing_data.has_max_upgrade,
            weekly_tokens: weekly_usage.as_ref().map(|u| u.tokens),
            weekly_cost_cny: weekly_usage.as_ref().map(|u| u.cost_cny),
            weekly_calls: weekly_usage.as_ref().map(|u| u.calls),
        }),
        error: None,
    }
}

// ─── Billing page parser ─────────────────────────────────────────────────────

struct BillingData {
    plan_name: String,
    renews_on: Option<String>,
    price: Option<String>,
    has_annual_option: bool,
    has_max_upgrade: bool,
}

async fn fetch_billing_page(client: &Client, cookie: &str) -> Result<BillingData, String> {
    let html = fetch_page(client, cookie, "/settings/billing").await?;
    parse_billing_page(&html)
}

fn parse_billing_page(html: &str) -> Result<BillingData, String> {
    let document = Html::parse_document(html);

    // Extract plan name — look for "Current Plan: Pro" or "Current Plan: Max"
    let plan_name =
        extract_text_after(&document, "Current Plan:").unwrap_or_else(|| "Unknown".to_string());

    // Extract renewal date — "Your subscription renews on\nJuly 26, 2026."
    let renews_on = extract_text_after(&document, "renews on")
        .map(|s| s.trim().trim_end_matches('.').trim().to_string())
        .filter(|s| !s.is_empty());

    // Price: "Paid\n$20.00" in the invoices table
    let price = extract_invoice_price(&document);

    // Check for "Change to annual billing" link
    let has_annual = html.contains("Change to annual billing");

    // Check for "Upgrade to Max" link
    let has_max = html.contains("Upgrade to Max");

    Ok(BillingData {
        plan_name,
        renews_on,
        price,
        has_annual_option: has_annual,
        has_max_upgrade: has_max,
    })
}

// ─── Settings page parser ────────────────────────────────────────────────────

/// One usage window's reset timestamp as displayed by the web UI.
#[derive(Debug, Clone, Copy)]
struct WebWindowReset {
    at: DateTime<Utc>,
    /// The text said "Sessions resume …" — the weekly budget is exhausted, so
    /// this timestamp dates the weekly roll-over, not the 5h session grid.
    is_resume: bool,
}

/// Reset timestamps scraped from `/settings`, per usage section.
#[derive(Debug, Clone, Copy, Default)]
struct WebResetTimes {
    session: Option<WebWindowReset>,
    weekly: Option<WebWindowReset>,
}

impl WebResetTimes {
    fn for_label(&self, usage_type: &str) -> Option<WebWindowReset> {
        match usage_type {
            "Session" => self.session,
            "Weekly" => self.weekly,
            _ => None,
        }
    }
}

/// Parse the reset timestamps the web UI renders next to each usage meter:
/// `<div class="… local-time" data-time="…">Resets in 2 days.</div>`.
///
/// The JSON API carries no reset info and the weekly window is
/// calendar-aligned (observed: Monday 00:00 UTC), so the page is the only
/// authoritative source. Sections are located by their "Session usage" /
/// "Weekly usage" labels and each runs to the next label; ASCII-lowercasing
/// keeps byte offsets stable for the raw-HTML slicing below.
fn parse_web_reset_times(html: &str) -> WebResetTimes {
    let lower = html.to_ascii_lowercase();
    let mut labels: Vec<(&str, usize)> = Vec::new();
    for (label, kind) in [("session usage", "Session"), ("weekly usage", "Weekly")] {
        if let Some(pos) = lower.find(label) {
            labels.push((kind, pos));
        }
    }
    labels.sort_by_key(|(_, pos)| *pos);

    let mut web = WebResetTimes::default();
    for (i, (kind, start)) in labels.iter().enumerate() {
        let start = *start;
        let end = labels.get(i + 1).map_or(lower.len(), |(_, pos)| *pos);
        let reset = parse_local_time_element(&lower[start..end]);
        match *kind {
            "Session" => web.session = reset,
            _ => web.weekly = reset,
        }
    }
    web
}

/// First `.local-time` element inside a section slice.
fn parse_local_time_element(slice: &str) -> Option<WebWindowReset> {
    let rest = &slice[slice.find("local-time")?..];
    let tag_end = rest.find('>')?;
    let tag = &rest[..tag_end];
    let time_attr = "data-time=\"";
    let value_start = tag.find(time_attr)? + time_attr.len();
    let value_end = value_start + tag[value_start..].find('"')?;
    let at = DateTime::parse_from_rfc3339(&tag[value_start..value_end])
        .ok()?
        .with_timezone(&Utc);
    let text_end = rest[tag_end..]
        .find("</div>")
        .map_or(rest.len() - tag_end, |pos| pos);
    let is_resume = rest[tag_end..tag_end + text_end].contains("resume");
    Some(WebWindowReset { at, is_resume })
}

/// Parse usage entries from the settings page.
/// Looks for "Session usage" and "Weekly usage" sections with percentage + reset time.
fn parse_usage_from_html(html: &str) -> Vec<OllamaUsageEntry> {
    let document = Html::parse_document(html);
    let web = parse_web_reset_times(html);
    let mut entries = Vec::new();

    let root = document.root_element();
    let text: String = root.text().collect();
    let text_lower = text.to_lowercase();

    for usage_type in ["Session", "Weekly"] {
        let label = format!("{} usage", usage_type);
        let label_lower = label.to_lowercase();

        if !text_lower.contains(&label_lower) {
            continue;
        }

        let Some(pct) = extract_usage_percentage(&document, usage_type) else {
            continue;
        };

        entries.push(OllamaUsageEntry {
            usage_type: usage_type.to_string(),
            percentage: pct,
            reset_time: web
                .for_label(usage_type)
                .map(|w| w.at.to_rfc3339_opts(SecondsFormat::Secs, true)),
        });
    }

    entries
}

// ─── Extraction helpers ─────────────────────────────────────────────────────

/// Extract text that appears after a label in the page. Uses a simple text search
/// through the flattened document text.
fn extract_text_after(document: &Html, label: &str) -> Option<String> {
    // Walk the document root and collect text nodes
    let root = document.root_element();
    let text = root.text().collect::<Vec<_>>().join("");

    // Find the label position
    let label_lower = label.to_lowercase();
    let text_lower = text.to_lowercase();
    let label_pos = text_lower.find(&label_lower)?;
    let after = &text[label_pos + label.len()..];

    // Take the first meaningful line or phrase after the label
    let result = after.trim().lines().next().unwrap_or("").trim().to_string();

    // Clean up: remove trailing dots and extra whitespace
    let result = result.trim_end_matches('.').trim().to_string();

    if result.is_empty() {
        None
    } else {
        Some(result)
    }
}

/// Extract the invoice price from the billing page.
/// Looks for a "$" price near "Paid" in the invoice table.
fn extract_invoice_price(document: &Html) -> Option<String> {
    let root = document.root_element();
    let text = root.text().collect::<Vec<_>>().join(" ");

    // Find "Paid" and look backwards for a $XX.XX pattern in the same vicinity
    let paid_pos = text.rfind("Paid")?;

    // Search backwards within ~100 chars before "Paid"
    let search_start = paid_pos.saturating_sub(100);
    let before = &text[search_start..paid_pos];

    // Find the last $XX.XX pattern before "Paid"
    let dollar_pos = before.rfind('$')?;
    let dollar_str = &before[dollar_pos..];
    let end = dollar_str[1..]
        .char_indices()
        .find(|(_, c)| !c.is_ascii_digit() && *c != '.')
        .map(|(i, _)| i + 1)
        .unwrap_or(dollar_str.len());

    let price_str = &dollar_str[..end];
    if price_str.len() > 1 {
        Some(price_str.to_string())
    } else {
        None
    }
}

/// Extract usage percentage for a given usage type (Session or Weekly).
/// Handles both integer ("100%") and float ("19.2%") formats.
fn extract_usage_percentage(document: &Html, usage_type: &str) -> Option<f64> {
    let root = document.root_element();
    let text = root.text().collect::<Vec<_>>().join("");

    // Find the usage type label
    let label = format!("{} usage", usage_type);
    let text_lower = text.to_lowercase();
    let label_pos = text_lower.find(&label.to_lowercase())?;

    // Look for "X% used" after the label — match digits, dots, then "%"
    let after = &text[label_pos + label.len()..];
    let pct_start = after.find(|c: char| c.is_ascii_digit())?;
    let pct_str = after[pct_start..]
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect::<String>();

    pct_str.parse::<f64>().ok()
}

// ─── Generic helpers ─────────────────────────────────────────────────────────

/// Classify a request that landed on the sign-in flow instead of the requested
/// settings page.
///
/// ollama.com answers an unauthenticated request with `303 → /signin`, which
/// hops to `signin.ollama.com` and serves a **200** login page. reqwest follows
/// that chain, so an expired cookie is indistinguishable from success by status
/// code alone — every selector then silently misses and the card renders
/// "Unknown" with zero usage.
fn auth_redirect_error(final_url: &Url) -> Option<String> {
    let host = final_url.host_str().unwrap_or_default();
    let path = final_url.path().trim_end_matches('/');

    if host != OLLAMA_HOST || path.eq_ignore_ascii_case("/signin") {
        return Some(format!(
            "Session cookie expired (redirected to {}{}); re-login at ollama.com and refresh OLLAMA_AUTH_COOKIE",
            host, path
        ));
    }

    None
}

/// Fetch a page from ollama.com with authentication cookies.
async fn fetch_page(client: &Client, cookie: &str, path: &str) -> Result<String, String> {
    let url = format!("{}{}", OLLAMA_BASE_URL, path);

    let response = client
        .get(&url)
        .header(
            "User-Agent",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
             (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
        )
        .header("Cookie", cookie)
        .timeout(std::time::Duration::from_secs(OLLAMA_TIMEOUT_SECS))
        .send()
        .await
        .map_err(|e| format!("Network error: {e}"))?;

    let final_url = response.url().clone();
    if let Some(e) = auth_redirect_error(&final_url) {
        return Err(e);
    }

    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status()));
    }

    response
        .text()
        .await
        .map_err(|e| format!("Read error: {e}"))
}

// ─── API-key path ────────────────────────────────────────────────────────────

/// Read `OLLAMA_API_KEY` from environment — the same key CPA's `ollama-cloud`
/// upstream uses, so the plan/limits reported belong to the account that
/// actually serves the metered traffic.
pub fn get_api_key() -> Option<String> {
    std::env::var("OLLAMA_API_KEY").ok().filter(|k| !k.is_empty())
}

#[derive(serde::Deserialize)]
struct ApiMe {
    #[serde(rename = "Plan", default)]
    plan: Option<String>,
    /// Account creation time (`CreatedAt`). Ollama's included usage resets
    /// monthly on the day-of-month the plan started, which for this account is
    /// the creation date — see [`next_monthly_renewal`].
    #[serde(rename = "CreatedAt", default)]
    created_at: Option<String>,
}

#[derive(serde::Deserialize)]
struct ApiUsage {
    #[serde(default)]
    limits: Option<ApiLimits>,
}

#[derive(serde::Deserialize)]
struct ApiLimits {
    #[serde(default)]
    session: Option<ApiLimit>,
    #[serde(default)]
    weekly: Option<ApiLimit>,
}

#[derive(serde::Deserialize)]
struct ApiLimit {
    /// Fraction of the limit consumed (`0.293` → 29.3%).
    #[serde(default)]
    usage: Option<f64>,
    /// Per-model breakdown of the requests counted in this window (includes
    /// non-model rows such as `web search` / `web fetch`).
    #[serde(default)]
    models: Vec<ApiLimitModel>,
}

#[derive(serde::Deserialize)]
struct ApiLimitModel {
    #[serde(default)]
    request_count: i64,
}

impl ApiLimit {
    fn total_requests(&self) -> i64 {
        self.models.iter().map(|m| m.request_count).sum()
    }
}

async fn fetch_via_api(
    client: &Client,
    api_key: &str,
    records: &[TokenRecord],
) -> Result<OllamaQuotaData, String> {
    let (me_result, usage_result, (web, web_usage)) = tokio::join!(
        fetch_api_json(client, api_key, reqwest::Method::POST, "/api/me"),
        fetch_api_json(client, api_key, reqwest::Method::GET, "/api/usage"),
        web_settings(client),
    );
    let me = me_result?;
    let usage = usage_result?;
    parse_api_quota(&me, &usage, records, &web, &web_usage, Utc::now())
}

/// Best-effort scrape of `/settings`: reset timestamps **and** the usage
/// percentages next to each meter. Requires `OLLAMA_AUTH_COOKIE`; an empty
/// result makes every consumer fall back to the local phase model.
async fn web_settings(client: &Client) -> (WebResetTimes, Vec<OllamaUsageEntry>) {
    let Some(cookie) = get_auth_cookie() else {
        return (WebResetTimes::default(), Vec::new());
    };
    match fetch_page(client, &cookie, "/settings").await {
        Ok(html) => (parse_web_reset_times(&html), parse_usage_from_html(&html)),
        Err(e) => {
            warn!(
                "Ollama /settings scrape failed ({e}); reset times and percentages fall back \
                 to the local phase"
            );
            (WebResetTimes::default(), Vec::new())
        }
    }
}

async fn fetch_api_json(
    client: &Client,
    api_key: &str,
    method: reqwest::Method,
    path: &str,
) -> Result<String, String> {
    let url = format!("{}{}", OLLAMA_BASE_URL, path);
    let response = client
        .request(method, &url)
        // reqwest here is HTTP/1.1-only (no `http2` feature) and ollama.com's
        // Go server answers a body-less POST on HTTP/1.1 with `411 Length
        // Required`, so the zero length has to be explicit.
        .header(reqwest::header::CONTENT_LENGTH, "0")
        .header("Authorization", format!("Bearer {api_key}"))
        .timeout(std::time::Duration::from_secs(OLLAMA_TIMEOUT_SECS))
        .send()
        .await
        .map_err(|e| format!("Network error: {e}"))?;

    if !response.status().is_success() {
        return Err(format!("{path} HTTP {}", response.status()));
    }

    response.text().await.map_err(|e| format!("Read error: {e}"))
}

/// Build the card payload from `/api/me` + `/api/usage` responses.
///
/// `web` carries the reset timestamps scraped from `/settings`; the phase
/// model only fills windows the page could not date. `web_usage` carries that
/// page's percentages, used when the API reports no `limits` (its shape as of
/// 2026-10-07).
fn parse_api_quota(
    me_json: &str,
    usage_json: &str,
    records: &[TokenRecord],
    web: &WebResetTimes,
    web_usage: &[OllamaUsageEntry],
    now: DateTime<Utc>,
) -> Result<OllamaQuotaData, String> {
    let me: ApiMe = serde_json::from_str(me_json).map_err(|e| format!("Bad /api/me: {e}"))?;
    let usage: ApiUsage =
        serde_json::from_str(usage_json).map_err(|e| format!("Bad /api/usage: {e}"))?;
    let limits = usage.limits.as_ref();
    let session = limits.and_then(|l| l.session.as_ref());
    let weekly = limits.and_then(|l| l.weekly.as_ref());

    let samples = [
        WindowSample {
            kind: WindowKind::Session,
            usage: session.and_then(|l| l.usage),
            counted_requests: session.map(ApiLimit::total_requests),
        },
        WindowSample {
            kind: WindowKind::Weekly,
            usage: weekly.and_then(|l| l.usage),
            counted_requests: weekly.map(ApiLimit::total_requests),
        },
    ];
    let windows = update_window_state(records, &samples, now, web);

    let mut usage_entries = Vec::new();
    for (label, limit, reset) in [
        (
            "Session",
            session,
            web.session.map(|w| w.at).or(windows.session_reset),
        ),
        (
            "Weekly",
            weekly,
            web.weekly.map(|w| w.at).or(windows.weekly_reset),
        ),
    ] {
        let pct = limit
            .and_then(|l| l.usage)
            .map(|u| u * 100.0)
            .or_else(|| web_percentage(web_usage, label));
        if let Some(pct) = pct {
            usage_entries.push(OllamaUsageEntry {
                usage_type: label.to_string(),
                percentage: pct,
                reset_time: reset.map(|t| t.to_rfc3339_opts(SecondsFormat::Secs, true)),
            });
        }
    }

    let weekly_usage = windows
        .weekly_start
        .map(|start| weekly_actuals(records, start, now));

    Ok(OllamaQuotaData {
        plan_name: me
            .plan
            .map(|p| capitalize(&p))
            .unwrap_or_else(|| "Unknown".to_string()),
        renews_on: next_monthly_renewal(me.created_at.as_deref(), now),
        price: None,
        usage_entries,
        has_annual_option: false,
        has_max_upgrade: false,
        weekly_tokens: weekly_usage.as_ref().map(|u| u.tokens),
        weekly_cost_cny: weekly_usage.as_ref().map(|u| u.cost_cny),
        weekly_calls: weekly_usage.as_ref().map(|u| u.calls),
    })
}

/// Percentage the settings page rendered for `usage_type`, when scraped.
fn web_percentage(entries: &[OllamaUsageEntry], usage_type: &str) -> Option<f64> {
    entries
        .iter()
        .find(|e| e.usage_type == usage_type)
        .map(|e| e.percentage)
}

// ─── Usage windows (session 5h / weekly 7d) ─────────────────────────────────

/// Ollama's session window length.
const SESSION_PERIOD_SECS: i64 = 5 * 3600;
/// Ollama's weekly window length.
const WEEKLY_PERIOD_SECS: i64 = 7 * 86_400;
/// A usage drop of at least this many fraction points between two samples
/// means the window rolled over in between.
const RESET_DROP_EPS: f64 = 0.02;
/// Re-anchor the grid on a detected roll-over only when the previous sample is
/// this fresh; otherwise the poll gap is too wide to date the reset.
const RESET_FRESH_SECS: i64 = 600;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WindowKind {
    Session,
    Weekly,
}

impl WindowKind {
    fn period_secs(self) -> i64 {
        match self {
            WindowKind::Session => SESSION_PERIOD_SECS,
            WindowKind::Weekly => WEEKLY_PERIOD_SECS,
        }
    }
}

/// One window's usage sample from `/api/usage`.
#[derive(Debug, Clone, Copy)]
struct WindowSample {
    kind: WindowKind,
    /// Fraction of the window budget consumed, as reported by the API.
    usage: Option<f64>,
    /// Requests the API counted in the window (summed over the model
    /// breakdown, including non-model rows such as `web search`).
    counted_requests: Option<i64>,
}

/// Window boundaries resolved for this poll.
#[derive(Debug, Clone, Copy, Default)]
struct ResolvedWindows {
    /// When the live session window ends.
    session_reset: Option<DateTime<Utc>>,
    /// When the live weekly window started / ends.
    weekly_start: Option<DateTime<Utc>>,
    weekly_reset: Option<DateTime<Utc>>,
}

/// Persisted grid phase + last sample, so reset predictions survive restarts
/// and can be re-anchored when a roll-over is observed.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct OllamaWindowState {
    #[serde(default)]
    session_anchor: Option<String>,
    #[serde(default)]
    weekly_anchor: Option<String>,
    /// Last observed usage fraction per window (for roll-over detection).
    #[serde(default)]
    session_usage: Option<f64>,
    #[serde(default)]
    weekly_usage: Option<f64>,
    /// When those samples were taken.
    #[serde(default)]
    observed_at: Option<String>,
}

impl OllamaWindowState {
    fn anchor(&self, kind: WindowKind) -> Option<DateTime<Utc>> {
        let raw = match kind {
            WindowKind::Session => self.session_anchor.as_deref(),
            WindowKind::Weekly => self.weekly_anchor.as_deref(),
        }?;
        DateTime::parse_from_rfc3339(raw)
            .ok()
            .map(|t| t.with_timezone(&Utc))
    }

    fn set_anchor(&mut self, kind: WindowKind, anchor: DateTime<Utc>) {
        let value = anchor.to_rfc3339_opts(SecondsFormat::Secs, true);
        match kind {
            WindowKind::Session => self.session_anchor = Some(value),
            WindowKind::Weekly => self.weekly_anchor = Some(value),
        }
    }

    fn usage(&self, kind: WindowKind) -> Option<f64> {
        match kind {
            WindowKind::Session => self.session_usage,
            WindowKind::Weekly => self.weekly_usage,
        }
    }

    fn set_usage(&mut self, kind: WindowKind, usage: Option<f64>) {
        match kind {
            WindowKind::Session => self.session_usage = usage,
            WindowKind::Weekly => self.weekly_usage = usage,
        }
    }
}

/// Serializes the load → update → save cycle so two concurrent `/api/quota`
/// requests cannot interleave a stale read with a fresh write.
static WINDOW_STATE_LOCK: LazyLock<std::sync::Mutex<()>> =
    LazyLock::new(|| std::sync::Mutex::new(()));

/// `OLLAMA_WINDOW_STATE_PATH` overrides; defaults next to the other
/// token-stats state files.
fn window_state_path() -> PathBuf {
    if let Ok(p) = std::env::var("OLLAMA_WINDOW_STATE_PATH") {
        if !p.trim().is_empty() {
            return PathBuf::from(p);
        }
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(home).join(".config/token-stats/ollama-window.json")
}

fn load_window_state(path: &PathBuf) -> OllamaWindowState {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

fn save_window_state(path: &PathBuf, state: &OllamaWindowState) {
    let Some(dir) = path.parent() else { return };
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    let Ok(json) = serde_json::to_string_pretty(state) else {
        return;
    };
    // Write-then-rename so a crash mid-write cannot truncate the phase.
    let tmp = path.with_extension("json.tmp");
    if std::fs::write(&tmp, json).is_ok() {
        let _ = std::fs::rename(&tmp, path);
    }
}

/// Grid point at or before `now` (`anchor + k·period`).
fn grid_start(anchor: DateTime<Utc>, period_secs: i64, now: DateTime<Utc>) -> DateTime<Utc> {
    if now <= anchor {
        return anchor;
    }
    let elapsed = (now - anchor).num_seconds();
    anchor + Duration::seconds(elapsed - elapsed % period_secs)
}

/// Next grid point strictly after `now`.
fn next_reset(anchor: DateTime<Utc>, period_secs: i64, now: DateTime<Utc>) -> DateTime<Utc> {
    grid_start(anchor, period_secs, now) + Duration::seconds(period_secs)
}

/// Parse a `TokenRecord.time` (RFC3339 UTC).
fn record_time(record: &TokenRecord) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(&record.time)
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

/// Start of the usage window containing `now`, located from the caller's own
/// metered records: `/api/usage` reports how many requests the live window
/// counted and (almost) every one of them went through our proxy, so the
/// window's first request sits `counted_requests` records back.
///
/// Returns `None` when the count is unknown, zero, or larger than the records
/// we hold (the account has traffic we do not meter) — the caller then keeps
/// predicting from the persisted phase instead.
fn infer_window_start(
    records: &[TokenRecord],
    counted_requests: Option<i64>,
    period_secs: i64,
    now: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    let count = counted_requests.filter(|c| *c > 0)? as usize;
    let floor = now - Duration::seconds(period_secs);
    let mut times: Vec<DateTime<Utc>> = records
        .iter()
        .filter_map(record_time)
        .filter(|t| *t > floor && *t <= now)
        .collect();
    if count > times.len() {
        return None;
    }
    times.sort_unstable();
    Some(times[times.len() - count])
}

/// Resolve both windows for this poll and persist the updated phase.
///
/// Phase rules, in priority order:
/// 0. a reset timestamp scraped from the web UI dates the window directly and
///    re-anchors the grid — authoritative, since the weekly window is
///    calendar-aligned rather than anchored to our first request;
/// 1. an observed roll-over (usage halved between two fresh samples) re-anchors
///    the grid to the observation time — the only signal that dates a reset
///    independently of our own traffic;
/// 2. otherwise the persisted phase predicts the boundaries;
/// 3. a phase is bootstrapped from [`infer_window_start`] when none is stored.
fn update_window_state(
    records: &[TokenRecord],
    samples: &[WindowSample],
    now: DateTime<Utc>,
    web: &WebResetTimes,
) -> ResolvedWindows {
    let _guard = WINDOW_STATE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let path = window_state_path();
    let mut state = load_window_state(&path);

    let prev_observed = state
        .observed_at
        .as_deref()
        .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
        .map(|t| t.with_timezone(&Utc));

    let mut resolved = ResolvedWindows::default();

    // 0. The web UI's own timestamps win. A "Sessions resume …" notice dates
    //    the weekly roll-over (the budget is exhausted), not the 5h session
    //    grid — display it, but don't corrupt the session phase with it.
    for (kind, web_reset) in [
        (WindowKind::Session, web.session),
        (WindowKind::Weekly, web.weekly),
    ] {
        let Some(w) = web_reset else { continue };
        if !(w.is_resume && kind == WindowKind::Session) {
            state.set_anchor(kind, w.at - Duration::seconds(kind.period_secs()));
        }
        match kind {
            WindowKind::Session => resolved.session_reset = Some(w.at),
            WindowKind::Weekly => {
                resolved.weekly_start = state
                    .anchor(kind)
                    .map(|a| grid_start(a, kind.period_secs(), now));
                resolved.weekly_reset = Some(w.at);
            }
        }
    }

    for sample in samples {
        let kind = sample.kind;
        let period = kind.period_secs();

        // The web already dated this window; only track its usage so the
        // phase-model bookkeeping stays fresh for cookie-less polls.
        let resolved_by_web = match kind {
            WindowKind::Session => resolved.session_reset.is_some(),
            WindowKind::Weekly => resolved.weekly_reset.is_some(),
        };
        if resolved_by_web {
            state.set_usage(kind, sample.usage);
            continue;
        }

        let mut anchor = state.anchor(kind);

        // 1. Roll-over observed: usage dropped hard since the last sample.
        let rolled_over = matches!(
            (state.usage(kind), sample.usage),
            (Some(prev), Some(now_usage))
                if prev - now_usage >= RESET_DROP_EPS && now_usage <= prev * 0.5
        );
        if rolled_over {
            // With a fresh previous sample the reset happened within the last
            // poll interval, which pins the new phase. A stale sample (backend
            // was down) cannot date it, so the known phase is kept.
            if let Some(prev_at) = prev_observed {
                if now - prev_at <= Duration::seconds(RESET_FRESH_SECS) {
                    // Keep the established phase when one of its grid points
                    // falls inside the poll gap (it *is* the reset); otherwise
                    // the phase moved and the observation re-anchors it.
                    let grid_point =
                        anchor.map(|a| grid_start(a, period, prev_at) + Duration::seconds(period));
                    anchor = Some(match grid_point {
                        Some(point) if point <= now => point,
                        _ => now,
                    });
                }
            }
        }

        // 2./3. Predict from the phase, bootstrapping from our records when
        // there is none yet.
        let window_start = match anchor {
            Some(a) => grid_start(a, period, now),
            None => match infer_window_start(records, sample.counted_requests, period, now) {
                Some(start) => {
                    state.set_anchor(kind, start);
                    start
                }
                None => {
                    state.set_usage(kind, sample.usage);
                    continue;
                }
            },
        };

        let reset = next_reset(window_start, period, now);
        match kind {
            WindowKind::Session => resolved.session_reset = Some(reset),
            WindowKind::Weekly => {
                resolved.weekly_start = Some(window_start);
                resolved.weekly_reset = Some(reset);
            }
        }
        state.set_usage(kind, sample.usage);
    }

    state.observed_at = Some(now.to_rfc3339_opts(SecondsFormat::Secs, true));
    save_window_state(&path, &state);
    resolved
}

/// Tokens, calls and subscription cost metered in one weekly window.
struct WeeklyUsage {
    calls: i64,
    tokens: i64,
    cost_cny: f64,
}

/// Aggregate the caller's `ollama-proxy` records inside `[start, now]`.
///
/// Costs use the same empirical subscription rate as the rest of the dashboard
/// ([`pricing::display_cost`]), so the card's ¥ figure matches the usage tables
/// instead of the old percentage × empirical-quota guess.
fn weekly_actuals(
    records: &[TokenRecord],
    start: DateTime<Utc>,
    now: DateTime<Utc>,
) -> WeeklyUsage {
    let ps = pricing::state_read();
    let mut usage = WeeklyUsage {
        calls: 0,
        tokens: 0,
        cost_cny: 0.0,
    };
    for record in records {
        let Some(t) = record_time(record) else { continue };
        if t < start || t > now {
            continue;
        }
        usage.calls += 1;
        usage.tokens += record.total_tokens;
        usage.cost_cny += pricing::display_cost_in(&ps, record);
    }
    usage
}

/// Next monthly renewal date (`YYYY-MM-DD`).
///
/// Ollama resets included usage monthly on the day-of-month the plan started
/// (annual plans included); the API-key path cannot read the billing page, so
/// this uses `/api/me`'s `CreatedAt`. Validated against a captured billing page
/// for this account: created 2026-06-26 → "renews on July 26, 2026".
fn next_monthly_renewal(created_at: Option<&str>, now: DateTime<Utc>) -> Option<String> {
    let created = DateTime::parse_from_rfc3339(created_at?)
        .ok()?
        .with_timezone(&Utc);
    let day = created.day();
    // Step forward month by month from the current month until the
    // anniversary is still ahead of `now`.
    let mut year = now.year();
    let mut month = now.month();
    for _ in 0..24 {
        let candidate = clamp_day(year, month, day);
        if candidate >= now.date_naive() {
            return Some(candidate.format("%Y-%m-%d").to_string());
        }
        if month == 12 {
            year += 1;
            month = 1;
        } else {
            month += 1;
        }
    }
    None
}

/// `year-month-day`, clamped to the month's last day (e.g. the 31st → 28/30).
fn clamp_day(year: i32, month: u32, day: u32) -> chrono::NaiveDate {
    let last = chrono::NaiveDate::from_ymd_opt(
        if month == 12 { year + 1 } else { year },
        if month == 12 { 1 } else { month + 1 },
        1,
    )
    .and_then(|d| d.pred_opt())
    .unwrap_or_else(|| chrono::NaiveDate::from_ymd_opt(year, month, 28).unwrap());
    chrono::NaiveDate::from_ymd_opt(year, month, day.min(last.day())).unwrap_or(last)
}

fn capitalize(value: &str) -> String {
    let mut chars = value.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_auth_cookie() {
        temp_env::with_var(
            "OLLAMA_AUTH_COOKIE",
            Some("aid=abc; __Secure-session=xyz"),
            || {
                assert_eq!(
                    get_auth_cookie(),
                    Some("aid=abc; __Secure-session=xyz".to_string())
                );
            },
        );
    }

    #[test]
    fn test_get_auth_cookie_unset() {
        temp_env::with_var("OLLAMA_AUTH_COOKIE", None::<&str>, || {
            assert_eq!(get_auth_cookie(), None);
        });
    }

    #[test]
    fn test_get_auth_cookie_empty() {
        temp_env::with_var("OLLAMA_AUTH_COOKIE", Some(""), || {
            assert_eq!(get_auth_cookie(), None);
        });
    }

    #[test]
    fn test_parse_billing_page() {
        let html = r#"<html><body>
            <div>Current Plan: Pro</div>
            <div>Your subscription renews on July 26, 2026.</div>
            <a href="/upgrade">Upgrade to Max</a>
            <a href="/billing/annual">Change to annual billing</a>
            <table>
                <tr><td>June 26, 2026</td><td>-</td><td>$20.00</td><td>Paid</td></tr>
            </table>
        </body></html>"#;

        let result = parse_billing_page(html).unwrap();
        assert_eq!(result.plan_name, "Pro");
        assert_eq!(result.renews_on, Some("July 26, 2026".to_string()));
        assert_eq!(result.price, Some("$20.00".to_string()));
        assert!(result.has_annual_option);
        assert!(result.has_max_upgrade);
    }

    #[test]
    fn test_parse_billing_page_minimal() {
        let html = r#"<html><body>
            <div>Current Plan: Pro</div>
        </body></html>"#;

        let result = parse_billing_page(html).unwrap();
        assert_eq!(result.plan_name, "Pro");
        assert!(result.renews_on.is_none());
        assert!(result.price.is_none());
    }

    #[test]
    fn test_parse_usage_from_html() {
        let html = r#"<html><body>
            <div>
                <span>Session usage</span>
                <span>0% used</span>
                <div class="local-time" data-time="2026-06-26T05:00:00Z">Resets in 3 hours.</div>
            </div>
            <div>
                <span>Weekly usage</span>
                <span>10% used</span>
                <div class="local-time" data-time="2026-06-29T00:00:00Z">Resets in 3 days.</div>
            </div>
        </body></html>"#;

        let entries = parse_usage_from_html(html);
        assert_eq!(entries.len(), 2);

        assert_eq!(entries[0].usage_type, "Session");
        assert_eq!(entries[0].percentage, 0.0);
        assert_eq!(
            entries[0].reset_time.as_deref(),
            Some("2026-06-26T05:00:00Z")
        );

        assert_eq!(entries[1].usage_type, "Weekly");
        assert_eq!(entries[1].percentage, 10.0);
        assert_eq!(
            entries[1].reset_time.as_deref(),
            Some("2026-06-29T00:00:00Z")
        );
    }

    #[test]
    fn test_parse_usage_from_html_no_data() {
        let html = r#"<html><body><p>No usage here</p></body></html>"#;
        let entries = parse_usage_from_html(html);
        assert!(entries.is_empty());
    }

    #[test]
    fn test_extract_usage_percentage() {
        let html = r#"<html><body>
            <span>Session usage</span>
            <span>42% used</span>
        </body></html>"#;
        let doc = Html::parse_document(html);
        let pct = extract_usage_percentage(&doc, "Session");
        assert_eq!(pct, Some(42.0));
    }

    #[test]
    fn test_extract_usage_percentage_not_found() {
        let html = r#"<html><body><p>Nothing</p></body></html>"#;
        let doc = Html::parse_document(html);
        let pct = extract_usage_percentage(&doc, "Session");
        assert!(pct.is_none());
    }

    #[test]
    fn test_extract_invoice_price() {
        let html = r#"<html><body>
            <table><tr><td>June 26, 2026</td><td>-</td><td>$20.00</td><td>Paid</td></tr></table>
        </body></html>"#;
        let doc = Html::parse_document(html);
        let price = extract_invoice_price(&doc);
        assert_eq!(price, Some("$20.00".to_string()));
    }

    #[test]
    fn test_auth_redirect_error_detects_expired_cookie() {
        // What an expired `__Secure-session` actually lands on.
        let signin_host =
            Url::parse("https://signin.ollama.com/?client_id=client_01JX&authorization_session_id=01M38")
                .unwrap();
        assert!(auth_redirect_error(&signin_host)
            .unwrap()
            .contains("Session cookie expired"));

        let signin_path = Url::parse("https://ollama.com/signin").unwrap();
        assert!(auth_redirect_error(&signin_path).is_some());
    }

    #[test]
    fn test_auth_redirect_error_accepts_signed_in_pages() {
        for ok in [
            "https://ollama.com/settings/billing",
            "https://ollama.com/settings",
        ] {
            assert!(
                auth_redirect_error(&Url::parse(ok).unwrap()).is_none(),
                "{ok} should not be treated as an auth redirect"
            );
        }
    }

    #[test]
    fn test_parse_api_quota_maps_fractions_to_percentages() {
        with_state_path("api-parse", || {
            let me = r#"{"Email":"fanghm@gmail.com","Plan":"pro","CreatedAt":"2026-06-26T00:06:33.377399Z"}"#;
            let usage = r#"{
                "activity":{"cost":"0.00000","models":[]},
                "limits":{
                    "session":{"usage":0.293,"models":[{"name":"deepseek-v4.1-flash","request_count":552}]},
                    "weekly":{"usage":0.486,"models":[{"name":"web search","request_count":11}]}
                }
            }"#;
            let now = ts("2026-09-24T10:30:00Z");

            // The counts (552 / 11) exceed the records we hold, so no window
            // can be dated — the card must still render plan + percentages.
            let records = vec![record_at("2026-09-24T09:00:00Z", 10)];
            let data = parse_quota(me, usage, &records, now).unwrap();
            assert_eq!(data.plan_name, "Pro");
            assert_eq!(data.usage_entries.len(), 2);
            assert_eq!(data.usage_entries[0].usage_type, "Session");
            assert!((data.usage_entries[0].percentage - 29.3).abs() < 0.001);
            assert!(data.usage_entries[0].reset_time.is_none());
            assert_eq!(data.usage_entries[1].usage_type, "Weekly");
            assert!((data.usage_entries[1].percentage - 48.6).abs() < 0.001);
            // Renewal date comes from CreatedAt's day-of-month.
            assert_eq!(data.renews_on.as_deref(), Some("2026-09-26"));
            // No window start → no weekly totals rather than a wrong number.
            assert_eq!(data.weekly_tokens, None);
        });
    }

    #[test]
    fn test_parse_api_quota_tolerates_free_plan_and_missing_limits() {
        with_state_path("api-free", || {
            // Free accounts have no session/weekly windows; plan must still render.
            let data = parse_quota(
                r#"{"Plan":"free"}"#,
                r#"{}"#,
                &[],
                ts("2026-09-24T10:30:00Z"),
            )
            .unwrap();
            assert_eq!(data.plan_name, "Free");
            assert!(data.usage_entries.is_empty());
            assert_eq!(data.weekly_tokens, None);

            // Unparseable payload is an error so the caller can fall back.
            assert!(parse_quota("not json", "{}", &[], ts("2026-09-24T10:30:00Z")).is_err());
        });
    }

    #[test]
    fn test_parse_api_quota_falls_back_to_settings_page_percentages() {
        with_state_path("api-web-fallback", || {
            // 2026-10-07: `/api/usage` stopped returning `limits` (now per-day
            // request buckets). The settings page still renders the meters, so
            // the card must use those percentages instead of dropping them.
            let usage =
                r#"{"range":"7d","scope":"self","totals":{"request_count":14671},"buckets":[]}"#;
            let web = WebResetTimes {
                session: Some(WebWindowReset {
                    at: ts("2026-10-07T03:00:00Z"),
                    is_resume: false,
                }),
                weekly: Some(WebWindowReset {
                    at: ts("2026-10-12T00:00:00Z"),
                    is_resume: false,
                }),
            };
            let web_usage = vec![
                OllamaUsageEntry {
                    usage_type: "Session".into(),
                    percentage: 3.1,
                    reset_time: None,
                },
                OllamaUsageEntry {
                    usage_type: "Weekly".into(),
                    percentage: 18.8,
                    reset_time: None,
                },
            ];

            let data = parse_api_quota(
                r#"{"Plan":"pro","CreatedAt":"2026-06-26T00:06:33.377399Z"}"#,
                usage,
                &[],
                &web,
                &web_usage,
                ts("2026-10-07T02:40:00Z"),
            )
            .unwrap();

            assert_eq!(data.usage_entries.len(), 2);
            assert_eq!(data.usage_entries[0].usage_type, "Session");
            assert!((data.usage_entries[0].percentage - 3.1).abs() < 0.001);
            assert_eq!(
                data.usage_entries[0].reset_time.as_deref(),
                Some("2026-10-07T03:00:00Z")
            );
            assert_eq!(data.usage_entries[1].usage_type, "Weekly");
            assert!((data.usage_entries[1].percentage - 18.8).abs() < 0.001);
        });
    }

    #[test]
    fn test_parse_api_quota_prefers_api_percentages() {
        with_state_path("api-web-priority", || {
            // When `limits` does come back, its fractions win over the page.
            let web_usage = vec![OllamaUsageEntry {
                usage_type: "Session".into(),
                percentage: 3.1,
                reset_time: None,
            }];
            let data = parse_api_quota(
                r#"{"Plan":"pro"}"#,
                r#"{"limits":{"session":{"usage":0.293}}}"#,
                &[],
                &WebResetTimes::default(),
                &web_usage,
                ts("2026-10-07T02:40:00Z"),
            )
            .unwrap();
            assert_eq!(data.usage_entries.len(), 1);
            assert!((data.usage_entries[0].percentage - 29.3).abs() < 0.001);
        });
    }

    #[test]
    fn test_extract_text_after() {
        let html = r#"<html><body>
            <div>Current Plan: Pro</div>
            <div>Your subscription renews on July 26, 2026.</div>
        </body></html>"#;
        let doc = Html::parse_document(html);
        let plan = extract_text_after(&doc, "Current Plan:");
        assert_eq!(plan, Some("Pro".to_string()));
        let renew = extract_text_after(&doc, "renews on");
        assert_eq!(renew, Some("July 26, 2026".to_string()));
    }

    // ── Realistic integration-style tests ──────────────────────────────────────

    #[test]
    fn test_parse_usage_from_html_realistic() {
        // Simulates the actual ollama.com/settings HTML structure
        let html = r#"<html>
            <head><title>Usage · Settings</title></head>
            <body>
                <h2><span>Cloud usage</span><span>pro</span></h2>
                <p>Cloud models and capabilities such as web search contribute to session and weekly limits.</p>
                <div>
                    <div><span>Session usage</span><span>0% used</span></div>
                    <div>
                        <div class="local-time" data-time="2026-06-26T05:00:00Z">Resets in 3 hours.</div>
                    </div>
                </div>
                <div>
                    <div><span>Weekly usage</span><span>0% used</span></div>
                    <div>
                        <div class="local-time" data-time="2026-06-29T00:00:00Z">Resets in 3 days.</div>
                    </div>
                </div>
            </body>
        </html>"#;

        let entries = parse_usage_from_html(html);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].usage_type, "Session");
        assert_eq!(entries[1].usage_type, "Weekly");
    }

    #[test]
    fn test_parse_usage_from_html_float_percentage() {
        // Test float percentage parsing (e.g. "19.2% used")
        let html = r#"<html>
            <head><title>Usage · Settings</title></head>
            <body>
                <h2><span>Cloud usage</span><span>pro</span></h2>
                <div>
                    <div><span>Session usage</span><span>100% used</span></div>
                    <div>
                        <div class="local-time" data-time="2026-06-26T10:00:00Z">Resets in 6 minutes.</div>
                    </div>
                </div>
                <div>
                    <div><span>Weekly usage</span><span>19.2% used</span></div>
                    <div>
                        <div class="local-time" data-time="2026-06-29T00:00:00Z">Resets in 2 days.</div>
                    </div>
                </div>
            </body>
        </html>"#;

        let entries = parse_usage_from_html(html);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].usage_type, "Session");
        assert_eq!(entries[0].percentage, 100.0);
        assert_eq!(entries[1].usage_type, "Weekly");
        assert!((entries[1].percentage - 19.2).abs() < 0.01);
    }

    // ── Usage-window derivation ───────────────────────────────────────────────

    fn record_at(time: &str, tokens: i64) -> TokenRecord {
        TokenRecord {
            date: time[..10].into(),
            time: time.to_string(),
            api_key_prefix: "N/A".into(),
            provider: "ollama-cloud".into(),
            original_provider: None,
            model: "deepseek-v4.1-flash".into(),
            source: "ollama-proxy".into(),
            input_tokens: tokens,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            total_tokens: tokens,
            cost: 0.0,
            ttft_ms: None,
            tps: None,
        }
    }

    fn ts(value: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(value)
            .unwrap()
            .with_timezone(&Utc)
    }

    /// Point the state file at a temp path so tests never touch the real
    /// `~/.config/token-stats/ollama-window.json`.
    fn with_state_path<T>(name: &str, f: impl FnOnce() -> T) -> T {
        let path = std::env::temp_dir().join(format!(
            "ollama-window-test-{}-{name}.json",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let result =
            temp_env::with_var("OLLAMA_WINDOW_STATE_PATH", Some(path.to_str().unwrap()), f);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("json.tmp"));
        result
    }

    /// Phase-model-only variants: no web UI timestamps available.
    fn update_windows(
        records: &[TokenRecord],
        samples: &[WindowSample],
        now: DateTime<Utc>,
    ) -> ResolvedWindows {
        update_window_state(records, samples, now, &WebResetTimes::default())
    }

    fn parse_quota(
        me_json: &str,
        usage_json: &str,
        records: &[TokenRecord],
        now: DateTime<Utc>,
    ) -> Result<OllamaQuotaData, String> {
        parse_api_quota(
            me_json,
            usage_json,
            records,
            &WebResetTimes::default(),
            &[],
            now,
        )
    }

    #[test]
    fn test_infer_window_start_walks_back_counted_requests() {
        let now = ts("2026-09-24T10:30:00Z");
        let records = vec![
            record_at("2026-09-24T05:00:00Z", 1), // previous session window
            record_at("2026-09-24T10:08:25Z", 1), // first request of the live one
            record_at("2026-09-24T10:20:00Z", 1),
            record_at("2026-09-24T10:29:00Z", 1),
        ];
        assert_eq!(
            infer_window_start(&records, Some(3), SESSION_PERIOD_SECS, now),
            Some(ts("2026-09-24T10:08:25Z"))
        );
        // Unknown / zero counts and unmetered traffic cannot be dated.
        assert_eq!(
            infer_window_start(&records, None, SESSION_PERIOD_SECS, now),
            None
        );
        assert_eq!(
            infer_window_start(&records, Some(0), SESSION_PERIOD_SECS, now),
            None
        );
        assert_eq!(
            infer_window_start(&records, Some(9), SESSION_PERIOD_SECS, now),
            None
        );
        // The 5h-old record is inside a 7d window.
        assert_eq!(
            infer_window_start(&records, Some(4), WEEKLY_PERIOD_SECS, now),
            Some(ts("2026-09-24T05:00:00Z"))
        );
    }

    #[test]
    fn test_grid_prediction_walks_forward_by_period() {
        let anchor = ts("2026-09-24T10:08:25Z");
        assert_eq!(
            grid_start(anchor, SESSION_PERIOD_SECS, ts("2026-09-24T11:00:00Z")),
            anchor
        );
        assert_eq!(
            next_reset(anchor, SESSION_PERIOD_SECS, ts("2026-09-24T11:00:00Z")),
            ts("2026-09-24T15:08:25Z")
        );
        // Several windows later the grid still lands on the phase.
        assert_eq!(
            next_reset(anchor, SESSION_PERIOD_SECS, ts("2026-09-25T02:00:00Z")),
            ts("2026-09-25T06:08:25Z")
        );
        // Exactly on a boundary means the window just started.
        assert_eq!(
            next_reset(anchor, SESSION_PERIOD_SECS, ts("2026-09-24T15:08:25Z")),
            ts("2026-09-24T20:08:25Z")
        );
    }

    #[test]
    fn test_update_window_state_bootstraps_phase_from_counts() {
        with_state_path("bootstrap", || {
            let now = ts("2026-09-24T10:30:00Z");
            let records = vec![
                record_at("2026-09-24T10:08:25Z", 1),
                record_at("2026-09-24T10:20:00Z", 1),
            ];
            let samples = [
                WindowSample {
                    kind: WindowKind::Session,
                    usage: Some(0.12),
                    counted_requests: Some(2),
                },
                WindowSample {
                    kind: WindowKind::Weekly,
                    usage: Some(0.54),
                    counted_requests: Some(2),
                },
            ];
            let resolved = update_windows(&records, &samples, now);
            assert_eq!(resolved.session_reset, Some(ts("2026-09-24T15:08:25Z")));
            assert_eq!(resolved.weekly_start, Some(ts("2026-09-24T10:08:25Z")));
            assert_eq!(resolved.weekly_reset, Some(ts("2026-10-01T10:08:25Z")));

            // A later poll keeps predicting from the persisted phase.
            let later = update_windows(&records, &samples, ts("2026-09-24T14:00:00Z"));
            assert_eq!(later.session_reset, Some(ts("2026-09-24T15:08:25Z")));
            assert_eq!(later.weekly_reset, Some(ts("2026-10-01T10:08:25Z")));
        });
    }

    #[test]
    fn test_update_window_state_reanchors_on_observed_rollover() {
        with_state_path("rollover", || {
            let records = vec![record_at("2026-09-24T15:08:30Z", 1)];
            let samples = |usage: f64| {
                [WindowSample {
                    kind: WindowKind::Session,
                    usage: Some(usage),
                    counted_requests: Some(1),
                }]
            };

            // Bootstrap the phase from the first request of the window.
            let first = update_windows(&records, &samples(0.4), ts("2026-09-24T15:20:00Z"));
            assert_eq!(first.session_reset, Some(ts("2026-09-24T20:08:30Z")));

            // Usage halved between two fresh samples → the window rolled over
            // in that poll gap, and the known phase dates it exactly.
            update_windows(&records, &samples(0.4), ts("2026-09-24T20:08:00Z"));
            let second = update_windows(&records, &samples(0.01), ts("2026-09-24T20:08:30Z"));
            assert_eq!(second.session_reset, Some(ts("2026-09-25T01:08:30Z")));
        });
    }

    #[test]
    fn test_update_window_state_moves_phase_when_grid_point_misses_rollover() {
        with_state_path("phase-move", || {
            // Phase bootstrapped early (14:50) while the window really rolls at
            // 20:30: the roll-over observation is what re-anchors it.
            let records = vec![record_at("2026-09-24T14:50:00Z", 1)];
            let samples = |usage: f64| {
                [WindowSample {
                    kind: WindowKind::Session,
                    usage: Some(usage),
                    counted_requests: Some(1),
                }]
            };
            update_windows(&records, &samples(0.4), ts("2026-09-24T15:00:00Z"));
            update_windows(&records, &samples(0.4), ts("2026-09-24T20:25:00Z"));
            let resolved = update_windows(&records, &samples(0.05), ts("2026-09-24T20:30:00Z"));
            assert_eq!(resolved.session_reset, Some(ts("2026-09-25T01:30:00Z")));
        });
    }

    #[test]
    fn test_update_window_state_ignores_stale_rollover_sample() {
        with_state_path("stale", || {
            let records = vec![record_at("2026-09-24T15:08:30Z", 1)];
            let samples = [WindowSample {
                kind: WindowKind::Session,
                usage: Some(0.4),
                counted_requests: Some(1),
            }];
            update_windows(&records, &samples, ts("2026-09-24T15:20:00Z"));

            // Backend was down for hours: the drop cannot date the reset, so
            // the known phase is kept.
            let dropped = [WindowSample {
                kind: WindowKind::Session,
                usage: Some(0.01),
                counted_requests: Some(1),
            }];
            let resolved = update_windows(&records, &dropped, ts("2026-09-24T18:00:00Z"));
            assert_eq!(resolved.session_reset, Some(ts("2026-09-24T20:08:30Z")));
        });
    }

    #[test]
    fn test_update_window_state_without_counts_or_phase_reports_nothing() {
        with_state_path("empty", || {
            let samples = [WindowSample {
                kind: WindowKind::Weekly,
                usage: Some(0.1),
                counted_requests: None,
            }];
            let resolved = update_windows(&[], &samples, ts("2026-09-24T10:30:00Z"));
            assert_eq!(resolved.weekly_reset, None);
            assert_eq!(resolved.weekly_start, None);
        });
    }

    #[test]
    fn test_parse_api_quota_dates_windows_and_totals_week() {
        with_state_path("api-full", || {
            let me = r#"{"Plan":"pro","CreatedAt":"2026-06-26T00:06:33Z"}"#;
            let usage = r#"{
                "limits":{
                    "session":{"usage":0.5,"models":[{"name":"deepseek-v4.1-flash","request_count":2}]},
                    "weekly":{"usage":0.25,"models":[{"name":"deepseek-v4.1-flash","request_count":2}]}
                }
            }"#;
            let now = ts("2026-09-24T10:30:00Z");
            let records = vec![
                record_at("2026-09-24T10:08:25Z", 1_000),
                record_at("2026-09-24T10:20:00Z", 2_000),
            ];

            let data = parse_quota(me, usage, &records, now).unwrap();
            assert_eq!(
                data.usage_entries[0].reset_time.as_deref(),
                Some("2026-09-24T15:08:25Z")
            );
            assert_eq!(
                data.usage_entries[1].reset_time.as_deref(),
                Some("2026-10-01T10:08:25Z")
            );
            // Actual metered totals, not a percentage-derived estimate.
            assert_eq!(data.weekly_tokens, Some(3_000));
            assert_eq!(data.weekly_calls, Some(2));
            assert!(data.weekly_cost_cny.unwrap() >= 0.0);
        });
    }

    #[test]
    fn test_weekly_actuals_totals_only_the_live_window() {
        let start = ts("2026-09-18T03:44:46Z");
        let now = ts("2026-09-24T10:30:00Z");
        let records = vec![
            record_at("2026-09-17T00:00:00Z", 1_000), // previous window
            record_at("2026-09-18T03:44:47Z", 100),
            record_at("2026-09-24T10:00:00Z", 250),
            record_at("2026-09-24T11:00:00Z", 1_000), // future record
        ];
        let usage = weekly_actuals(&records, start, now);
        assert_eq!(usage.calls, 2);
        assert_eq!(usage.tokens, 350);
        // The CNY figure comes from the loaded pricing config; only its
        // presence is asserted here (other tests may swap the config).
        assert!(usage.cost_cny >= 0.0);
    }

    #[test]
    fn test_next_monthly_renewal_follows_creation_day() {
        // Account created 2026-06-26 → renewal on the 26th of each month.
        let now = ts("2026-09-24T10:30:00Z");
        assert_eq!(
            next_monthly_renewal(Some("2026-06-26T00:06:33.377399Z"), now),
            Some("2026-09-26".to_string())
        );
        // On the renewal day itself the date is still today's.
        assert_eq!(
            next_monthly_renewal(Some("2026-06-26T00:06:33Z"), ts("2026-09-26T00:00:00Z")),
            Some("2026-09-26".to_string())
        );
        // Past that day it rolls into the next month.
        assert_eq!(
            next_monthly_renewal(Some("2026-06-26T00:06:33Z"), ts("2026-09-27T00:00:00Z")),
            Some("2026-10-26".to_string())
        );
        // Short months clamp (31st → 28th).
        assert_eq!(
            next_monthly_renewal(Some("2026-01-31T00:00:00Z"), ts("2026-02-01T00:00:00Z")),
            Some("2026-02-28".to_string())
        );
        assert_eq!(next_monthly_renewal(None, now), None);
        assert_eq!(next_monthly_renewal(Some("nonsense"), now), None);
    }

    // ── Web-sourced reset times ─────────────────────────────────────────────

    /// Mirrors the real /settings markup: per-section meter with an
    /// aria-label, then a `.local-time` div carrying the timestamp, and a
    /// trailing script that also mentions `.local-time`.
    const SETTINGS_HTML: &str = r#"<html><body>
        <h2><span>Cloud usage</span><span>pro</span></h2>
        <p>Cloud models and capabilities such as web search consume usage.</p>
        <div>
          <div class="flex justify-between mb-2">
            <span class="text-sm">Session usage</span>
            <span class="text-sm">0% used</span>
          </div>
          <div class="relative h-3 overflow-hidden rounded-full bg-neutral-200"
               data-usage-track aria-label="Session usage 0% used"></div>
          <div class="text-xs text-neutral-500 mt-1 local-time"
               data-time="2026-09-28T00:00:00Z">Sessions resume in 2 days.</div>
        </div>
        <div>
          <div class="flex justify-between mb-2">
            <span class="text-sm">Weekly usage</span>
            <span class="text-sm text-red-500">100% used</span>
          </div>
          <div class="relative h-3 overflow-hidden rounded-full bg-neutral-200"
               data-usage-track aria-label="Weekly usage 100% used"></div>
          <div class="text-xs text-neutral-500 mt-1 local-time"
               data-time="2026-09-28T00:00:00Z">Resets in 2 days.</div>
          <div id="weekly-usage-models" class="mt-3 space-y-1.5"></div>
        </div>
        <script>
          document.querySelectorAll(".local-time").forEach(function (el) {
            var iso = el.dataset.time;
          });
        </script>
    </body></html>"#;

    #[test]
    fn test_parse_web_reset_times_reads_each_section() {
        let web = parse_web_reset_times(SETTINGS_HTML);
        let session = web.session.unwrap();
        let weekly = web.weekly.unwrap();
        assert_eq!(session.at, ts("2026-09-28T00:00:00Z"));
        assert!(session.is_resume);
        assert_eq!(weekly.at, ts("2026-09-28T00:00:00Z"));
        assert!(!weekly.is_resume);

        // Entries built from the same page carry the page's own timestamps.
        let entries = parse_usage_from_html(SETTINGS_HTML);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].usage_type, "Session");
        assert_eq!(entries[0].percentage, 0.0);
        assert_eq!(
            entries[0].reset_time.as_deref(),
            Some("2026-09-28T00:00:00Z")
        );
        assert_eq!(entries[1].usage_type, "Weekly");
        assert!((entries[1].percentage - 100.0).abs() < 0.001);
        assert_eq!(
            entries[1].reset_time.as_deref(),
            Some("2026-09-28T00:00:00Z")
        );
    }

    #[test]
    fn test_update_window_state_anchors_phase_from_web_reset() {
        with_state_path("web-anchor", || {
            // Weekly saturated at 100% until the calendar-aligned Monday
            // 00:00 UTC reset the page shows; the request-count bootstrap had
            // anchored a Friday phase and predicted a reset 4 days late.
            let samples = [WindowSample {
                kind: WindowKind::Weekly,
                usage: Some(1.0),
                counted_requests: Some(10_384),
            }];
            let web = WebResetTimes {
                session: Some(WebWindowReset {
                    at: ts("2026-09-28T00:00:00Z"),
                    is_resume: true,
                }),
                weekly: Some(WebWindowReset {
                    at: ts("2026-09-28T00:00:00Z"),
                    is_resume: false,
                }),
            };
            let resolved =
                update_window_state(&[], &samples, ts("2026-09-25T06:35:00Z"), &web);
            assert_eq!(resolved.weekly_reset, Some(ts("2026-09-28T00:00:00Z")));
            assert_eq!(resolved.weekly_start, Some(ts("2026-09-21T00:00:00Z")));
            assert_eq!(resolved.session_reset, Some(ts("2026-09-28T00:00:00Z")));

            // The phase re-anchored to reset − period, so later cookie-less
            // polls keep predicting the Monday schedule; the "Sessions
            // resume" time must not become the session anchor.
            let state = load_window_state(&window_state_path());
            assert_eq!(
                state.anchor(WindowKind::Weekly),
                Some(ts("2026-09-21T00:00:00Z"))
            );
            assert_eq!(state.anchor(WindowKind::Session), None);
        });
    }

    #[test]
    fn test_parse_api_quota_prefers_web_reset_over_phase() {
        with_state_path("api-web", || {
            let me = r#"{"Plan":"pro","CreatedAt":"2026-06-26T00:06:33Z"}"#;
            let usage = r#"{
                "limits":{
                    "session":{"usage":0.0,"models":[]},
                    "weekly":{"usage":1.0,"models":[{"name":"deepseek-v4.1-flash","request_count":10370}]}
                }
            }"#;
            let web = WebResetTimes {
                weekly: Some(WebWindowReset {
                    at: ts("2026-09-28T00:00:00Z"),
                    is_resume: false,
                }),
                ..WebResetTimes::default()
            };
            let data = parse_api_quota(
                me,
                usage,
                &[record_at("2026-09-21T05:00:00Z", 10)],
                &web,
                &[],
                ts("2026-09-25T06:35:00Z"),
            )
            .unwrap();
            assert_eq!(
                data.usage_entries[1].reset_time.as_deref(),
                Some("2026-09-28T00:00:00Z")
            );
            // Weekly actuals now span the true calendar-aligned window.
            assert_eq!(data.weekly_calls, Some(1));
        });
    }
}
