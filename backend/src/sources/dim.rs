//! DimAgent (dim) source: polls the DimAgent console API
//! (`https://dimagent.cn/api/log/self`), the same endpoint the console
//! "Activity" page ([https://dimagent.cn/console/activity]) uses.
//!
//! This replaces the previous SQLite collection
//! (`~/.dimcode/v2/dimcode.sqlite`, per-run aggregates in `usage_run_stats`):
//! the console API exposes *per-request* usage — timestamp, model, token
//! counts (input / output / cache hit), TTFT and TPS — which is the granular
//! shape the dashboard wants, and it requires no local DB at all.
//!
//! Endpoint (reverse-engineered from the console frontend bundle):
//!   GET /api/log/self?p={page}&page_size=100&type=2
//!   → `{"data": {items: [...], page, page_size, total, total_capped}}`
//!
//! - `p` is the page number (`page` is ignored by the server) and
//!   `page_size` is capped at 100.
//! - `type=2` is the usage log filter the Activity page uses.
//! - Items are ordered newest-first by `id`.
//! - Token convention is OpenAI-style: `prompt_tokens` **includes**
//!   `cache_tokens` (verified against `/api/user/daily-stats`, where
//!   `total_tokens = prompt_tokens + completion_tokens`). We subtract cache
//!   to normalize to the Anthropic convention used everywhere else (same as
//!   the old codex / zcode / qoder / dim parsers). The API has no cache-write
//!   field (daily-stats `cache_creation_tokens` is always 0), so
//!   `cache_write_tokens = 0`.
//!
//! Authentication: `DIMAGENT_SESSION_COOKIE` (browser `session` cookie value;
//! sent as `Cookie: session=<value>`), same credential as the quota card.
//! Without it the source is unavailable (graceful degradation).
//!
//! Polling: every refresh cycle (`REFRESH_INTERVAL_SECS`, default 30s) we
//! fetch page 1 and, only when new items appeared, the following pages up to
//! the last already-ingested id. Fingerprinting in the refresh path dedups
//! anything already persisted (e.g. after a page fetch fails mid-way and the
//! same pages are re-fetched on the next poll).

use super::DataSource;
use crate::models::TokenRecord;
use chrono::TimeZone;
use serde::Deserialize;
use std::sync::{Mutex, OnceLock};

#[derive(Default)]
pub struct DimSource;

/// Console API base URL (overridable for tests / future environments).
const API_BASE: &str = "https://dimagent.cn/api";
/// Server caps page_size at 100.
const PAGE_SIZE: u64 = 100;
const HTTP_TIMEOUT_SECS: u64 = 15;
/// Safety cap on the number of pages fetched in one backfill (~40k records).
const MAX_PAGES: u64 = 400;

// ─── JSON payload shapes ─────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct LogPage {
    items: Vec<LogItem>,
    #[serde(default)]
    total: i64,
}

#[derive(Debug, Clone, Deserialize)]
struct LogItem {
    id: i64,
    /// Unix timestamp (seconds).
    created_at: i64,
    /// `type` is a keyword; serde renames it.
    #[serde(rename = "type")]
    kind: i64,
    #[serde(default)]
    model_name: String,
    #[serde(default)]
    prompt_tokens: i64,
    #[serde(default)]
    completion_tokens: i64,
    #[serde(default)]
    cache_tokens: i64,
    #[serde(default)]
    ttft_ms: Option<f64>,
    #[serde(default)]
    tps: Option<f64>,
}

// ─── Polling state ───────────────────────────────────────────────────────────

/// Polling state for the console API.
struct DimPollState {
    /// Newest `id` already ingested; poll stops fetching pages once it
    /// crosses back to an id ≤ this value.
    last_seen_id: Option<i64>,
    /// Whether the most recent sync ran to completion (every page up to the
    /// watermark / end of history was fetched without error). Only complete
    /// syncs may advance `last_seen_id` and are safe to build the legacy-row
    /// migration purge on.
    last_sync_complete: bool,
}

static POLL_STATE: Mutex<DimPollState> = Mutex::new(DimPollState {
    last_seen_id: None,
    last_sync_complete: false,
});

fn http_client() -> &'static reqwest::blocking::Client {
    static CLIENT: OnceLock<reqwest::blocking::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(HTTP_TIMEOUT_SECS))
            .build()
            .expect("failed to build DimAgent HTTP client")
    })
}

impl DataSource for DimSource {
    fn name(&self) -> &'static str {
        "dim"
    }

    /// Full sync: fetch every available page (used at startup/restore).
    fn load(&self) -> Vec<TokenRecord> {
        let (items, complete) = match Self::sync(None) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("DimAgent API sync failed: {e}");
                return Vec::new();
            }
        };
        Self::commit(items, complete)
    }

    /// Incremental: fetch only pages that contain ids newer than the last
    /// ingested one (usually just one page → one HTTP request per poll).
    fn load_incremental(&self) -> Vec<TokenRecord> {
        let last_seen = POLL_STATE.lock().unwrap().last_seen_id;
        let (items, complete) = match Self::sync(last_seen) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("DimAgent API sync failed: {e}");
                return Vec::new();
            }
        };
        Self::commit(items, complete)
    }

    fn is_available(&self) -> bool {
        std::env::var("DIMAGENT_SESSION_COOKIE").is_ok()
    }
}

impl DimSource {
    /// Whether the most recent console-API sync completed successfully (no
    /// page fetch failed / no safety-cap stop). Used by the startup migration
    /// to decide it is safe to drop the legacy per-run rows.
    pub fn last_sync_completed() -> bool {
        POLL_STATE.lock().unwrap().last_sync_complete
    }

    /// Fetch pages from the console API, newest first, until either:
    /// - a page contains an item with `id <= last_seen` (incremental mode —
    ///   everything newer has been collected), or
    /// - the last page is reached (full backfill: `last_seen == None`).
    ///
    /// Returns (items, complete): `complete` means the full new range was
    /// fetched without error — only then may `last_seen_id` advance. A page
    /// fetch failure stops the scan with `complete = false`: the caller still
    /// ingests the pages collected so far (fingerprints dedup them on the
    /// next poll) but keeps the old watermark.
    ///
    /// Runs on a dedicated std thread: `reqwest::blocking` must not be
    /// created or used inside a tokio runtime context (it would panic when
    /// its internal runtime is dropped). Calls here happen from
    /// `#[tokio::main]` startup and from the refresh task.
    fn sync(last_seen: Option<i64>) -> Result<(Vec<LogItem>, bool), String> {
        let cookie = cookie()?;
        std::thread::scope(|scope| {
            let handle = scope.spawn(move || Self::sync_inner(&cookie, last_seen));
            match handle.join() {
                Ok(result) => result,
                Err(_) => {
                    tracing::warn!("DimAgent API sync thread panicked");
                    Ok((Vec::new(), false))
                }
            }
        })
    }

    fn sync_inner(
        cookie: &str,
        last_seen: Option<i64>,
    ) -> Result<(Vec<LogItem>, bool), String> {
        let client = http_client();
        let mut items = Vec::new();
        let mut page = 1u64;

        loop {
            let data = match fetch_page(client, cookie, page) {
                Ok(d) => d,
                Err(e) => {
                    tracing::warn!(
                        "DimAgent API: page {page} failed after {} item(s): {e}; \
                         stopping, will retry next refresh",
                        items.len()
                    );
                    return Ok((items, false));
                }
            };
            let done = match last_seen {
                Some(ls) => data.items.iter().any(|it| it.id <= ls),
                None => data.items.len() < PAGE_SIZE as usize,
            };
            let n = data.items.len();
            items.extend(data.items);
            if done {
                tracing::info!(
                    "DimAgent API: fetched {} item(s) across {page} page(s) (total={})",
                    items.len(),
                    data.total
                );
                return Ok((items, true));
            }
            if n < PAGE_SIZE as usize {
                // Server returned fewer than requested even though the page
                // was not "done" — treat as end of history.
                return Ok((items, true));
            }
            page += 1;
            if page > MAX_PAGES {
                tracing::warn!(
                    "DimAgent API backfill hit MAX_PAGES ({MAX_PAGES}); stopping"
                );
                return Ok((items, false));
            }
        }
    }

    /// Convert fetched items to records and advance the watermark when the
    /// sync completed. Items with zero total tokens (e.g. failed calls) are
    /// dropped, matching the dashboard's zero-token convention.
    fn commit(items: Vec<LogItem>, complete: bool) -> Vec<TokenRecord> {
        let records: Vec<TokenRecord> =
            items.iter().filter_map(item_to_record).collect();
        {
            let mut state = POLL_STATE.lock().unwrap();
            state.last_sync_complete = complete;
            if complete {
                if let Some(max_id) = items.iter().map(|it| it.id).max() {
                    state.last_seen_id = Some(max_id);
                }
            }
        }
        if !records.is_empty() {
            tracing::info!(
                "Loaded {} dim records{}",
                records.len(),
                if complete { "" } else { " (partial sync)" }
            );
        }
        records
    }
}

fn cookie() -> Result<String, String> {
    std::env::var("DIMAGENT_SESSION_COOKIE")
        .map_err(|_| "DIMAGENT_SESSION_COOKIE not set".to_string())
}

fn fetch_page(
    client: &reqwest::blocking::Client,
    cookie: &str,
    page: u64,
) -> Result<LogPage, String> {
    let url = format!("{API_BASE}/log/self");
    let resp = client
        .get(&url)
        .header("Cookie", format!("session={cookie}"))
        .header("Accept", "application/json")
        .query(&[
            ("p", page.to_string()),
            ("page_size", PAGE_SIZE.to_string()),
            ("type", "2".to_string()),
        ])
        .send()
        .map_err(|e| format!("GET {url}: {e}"))?;

    let status = resp.status();
    let body = resp.text().unwrap_or_default();
    if !status.is_success() {
        const MAX_BODY: usize = 200;
        let snippet = if body.chars().count() > MAX_BODY {
            let short: String = body.chars().take(MAX_BODY).collect();
            format!("{short}...")
        } else {
            body.clone()
        };
        return Err(format!("GET {url}: HTTP {status}: {snippet}"));
    }

    // `{"data": {...}}` envelope.
    #[derive(Deserialize)]
    struct Envelope {
        data: LogPage,
    }
    serde_json::from_str::<Envelope>(&body)
        .map(|e| e.data)
        .map_err(|e| format!("parse {url}: {e}"))
}

/// Map one console-API log item to a [`TokenRecord`].
///
/// OpenAI cache convention: `prompt_tokens` includes `cache_tokens` →
/// subtract to get the non-cached input. `cache_tokens` is a cache *read*
/// (the API has no cache-write metric; daily reports show 0).
fn item_to_record(item: &LogItem) -> Option<TokenRecord> {
    if item.kind != 2 {
        // Only usage entries (the Activity page's `type=2` filter).
        return None;
    }
    let cache_read = item.cache_tokens.max(0);
    let effective_input = (item.prompt_tokens - cache_read).max(0);
    let output = item.completion_tokens.max(0);
    let total = effective_input + output + cache_read;
    if total == 0 {
        // Zero-token row (e.g. failed/429 call) — skip, see dashboard norms.
        return None;
    }

    let dt = chrono::Utc.timestamp_opt(item.created_at, 0).single()?;
    let (date, time) = super::parse_iso_timestamp(&dt.to_rfc3339());

    Some(TokenRecord {
        date,
        time,
        api_key_prefix: "N/A".to_string(),
        provider: "dim".to_string(),
        original_provider: Some("dim".to_string()),
        model: item.model_name.clone(),
        source: "dim".to_string(),
        input_tokens: effective_input,
        output_tokens: output,
        cache_read_tokens: cache_read,
        cache_write_tokens: 0,
        total_tokens: total,
        // Estimated at display time from pricing.toml (see display_cost:
        // source "dim" → per-model token rates, CNY-priced for DeepSeek).
        cost: 0.0,
        ttft_ms: item.ttft_ms,
        tps: item.tps,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_item() -> LogItem {
        serde_json::from_str(
            r#"{"id":10339681,"created_at":1788298903,"type":2,
                "token_name":"oauth:DimAgent Public",
                "model_name":"deepseek-v4-flash-vision-exp",
                "prompt_tokens":49182,"completion_tokens":635,
                "cache_tokens":48896,"use_time":6,"use_time_ms":6365,
                "ttft_ms":275,"tps":104.26929392446634,"is_stream":true}"#,
        )
        .unwrap()
    }

    #[test]
    fn maps_item_to_record_with_openai_cache_subtraction() {
        let r = item_to_record(&sample_item()).unwrap();
        assert_eq!(r.provider, "dim");
        assert_eq!(r.original_provider.as_deref(), Some("dim"));
        assert_eq!(r.source, "dim");
        assert_eq!(r.model, "deepseek-v4-flash-vision-exp");
        // prompt 49182 includes cache 48896 → non-cached input is 286.
        assert_eq!(r.input_tokens, 286);
        assert_eq!(r.output_tokens, 635);
        assert_eq!(r.cache_read_tokens, 48896);
        assert_eq!(r.cache_write_tokens, 0);
        assert_eq!(r.total_tokens, 286 + 635 + 48896);
        assert_eq!(r.cost, 0.0);
        assert_eq!(r.ttft_ms, Some(275.0));
        assert!(r.tps.is_some());
        assert_eq!(r.date, "2026-09-01");
    }

    #[test]
    fn cache_ratio_uses_normalized_input() {
        let r = item_to_record(&sample_item()).unwrap();
        // 48896 / (286 + 48896) ≈ 99.4% — the UI formula.
        assert!(r.cache_hit_ratio() > 99.0);
    }

    #[test]
    fn drops_zero_token_and_non_usage_items() {
        let mut zero = sample_item();
        zero.prompt_tokens = 0;
        zero.completion_tokens = 0;
        zero.cache_tokens = 0;
        assert!(item_to_record(&zero).is_none());

        let mut other = sample_item();
        other.kind = 1;
        assert!(item_to_record(&other).is_none());
    }

    #[test]
    fn clock_roundtrip_is_rfc3339_utc() {
        let r = item_to_record(&sample_item()).unwrap();
        assert!(r.time.starts_with("2026-09-01T21:41:43"));
        assert!(r.time.ends_with("+00:00"));
    }
}
