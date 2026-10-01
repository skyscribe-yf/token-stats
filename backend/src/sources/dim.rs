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
//!
//! **Local SQLite supplement**: the console API only reports usage billed
//! through Dim's own OAuth channel (`token_name: oauth:DimAgent Public`).
//! Calls routed to third-party providers (e.g. a custom Ollama Cloud
//! endpoint) never appear there — they only exist in the local
//! `~/.dimcode/v2/dimcode.sqlite` `usage_run_stats` table (per-run
//! aggregates). We read that table read-only, **excluding** the
//! `dimcode-api-oauth` provider (already covered by the API, would
//! double-count), every channel named in the constants below (each has its
//! own per-request meter), and — generically, so new CPA upstreams are covered
//! without editing this file — any channel whose `baseUrl` points at the shared
//! CLIProxyAPI loopback (see [`cpa_loopback_addrs`]), whose upstreams all have
//! usage plugins. Remaining third-party provider ids are mapped to the
//! dashboard's canonical provider name (e.g. `custom-ollama-cloud-042036d3` →
//! `ollama-cloud`, which vendor_merge.toml merges into the `ollama` group
//! and pricing.rs bills with the empirical subscription rate).

use super::DataSource;
use super::OLLAMA_CLOUD_RUN_PROVIDER;
use crate::models::TokenRecord;
use chrono::TimeZone;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
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
/// Local dimcode SQLite DB (per-run aggregates incl. third-party providers).
const LOCAL_DB_DEFAULT: &str = ".dimcode/v2/dimcode.sqlite";
/// Provider id of Dim's own OAuth channel — already covered by the console
/// API; rows with this provider are excluded from the local supplement to
/// avoid double-counting.
const DIM_OAUTH_PROVIDER: &str = "dimcode-api-oauth";
/// Provider id of the workbuddy proxy channel — already covered by the
/// `dim-agent` source (workbuddy-usage.jsonl, written by the workbuddy
/// plugin); rows with this provider are excluded from the local supplement
/// to avoid double-counting the same requests.
const WORKBUDDY_PROVIDER: &str = "workbuddy";
/// Provider id of the custom grok-build proxy channel (openai-responses
/// adapter pointed at the loopback grok proxy) — already covered by the
/// grok proxy's per-request log (source `grok-cli`); rows with this provider
/// are excluded from the local supplement to avoid double-counting.
const GROK_BUILD_PROXY_PROVIDER: &str = "grok-build-proxy";
/// Provider id of the custom channel that routes Ollama Cloud through the
/// shared CLIProxyAPI instance (an `openai-compatible` upstream with the `oc/`
/// prefix). Rows with this provider are excluded from the local supplement:
/// those requests are now metered per-call by the `ollama-usage` CPA plugin and
/// ingested through the `ollama-proxy` source.
const OLLAMA_CLOUD_PROXY_PROVIDER: &str = "ollama-cloud-proxy";
/// Provider id of the built-in Command Code proxy channel — already metered
/// per-request by the loopback proxy itself (source `cc-proxy`,
/// `cc-proxy-usage.jsonl`). Rows with this provider are excluded from the
/// local supplement to avoid double-counting the same requests (the proxy IS
/// the transport, so every request it sees is logged there).
const CC_PROXY_PROVIDER: &str = "cc-proxy";

/// Loopback `host:port` of the shared CLIProxyAPI instance
/// (`~/workbuddy-proxy/config.yaml`, `CPA_LOOPBACK_ADDRS` overrides). Every
/// upstream served by that instance has a token-stats usage plugin
/// (`workbuddy` → `dim-agent`, `ollama-cloud` → `ollama-proxy`,
/// `stepfun` → `stepfun-proxy`), so a Dim channel pointed *at the proxy* has
/// its calls metered per-request there — keeping Dim's per-run row as well
/// would count them twice. Channels aimed anywhere else (e.g. Dim's own
/// `step-plan` / `stepfun` providers, which talk to StepFun directly) are kept,
/// because nothing else records those calls.
fn cpa_loopback_addrs() -> Vec<String> {
    std::env::var("CPA_LOOPBACK_ADDRS")
        .unwrap_or_else(|_| "127.0.0.1:8317,localhost:8317,[::1]:8317".to_string())
        .split(',')
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Provider ids whose configured `baseUrl` points at the CLIProxyAPI instance.
fn cpa_metered_providers(conn: &rusqlite::Connection) -> HashSet<String> {
    let addrs = cpa_loopback_addrs();
    let mut ids = HashSet::new();
    let Ok(mut stmt) = conn.prepare("SELECT providerId, baseUrl FROM providers WHERE baseUrl IS NOT NULL")
    else {
        return ids;
    };
    let Ok(rows) = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    }) else {
        return ids;
    };
    for row in rows.flatten() {
        if let Some(authority) = url_authority(&row.1) {
            if addrs.iter().any(|addr| addr == &authority) {
                ids.insert(row.0);
            }
        }
    }
    ids
}

/// `host:port` of a URL, lowercased and without the default-port-less forms
/// (`http://127.0.0.1:8317/v1` → `127.0.0.1:8317`). None when unparseable.
fn url_authority(url: &str) -> Option<String> {
    let rest = url.split_once("://").map(|(_, r)| r)?;
    let authority = rest.split(['/', '?', '#']).next()?;
    if authority.is_empty() {
        return None;
    }
    Some(authority.to_lowercase())
}

/// Map a local `usage_run_stats.providerId` to the dashboard's canonical
/// provider name. Unknown ids fall back to the raw id (lowercased) so new
/// third-party channels still show up instead of being silently dropped.
fn map_local_provider(provider_id: &str) -> String {
    match provider_id {
        OLLAMA_CLOUD_RUN_PROVIDER => "ollama-cloud".to_string(),
        // Dim's Grok Build channel routes to xAI's official API — the same
        // SuperGrok subscription the grok-cli proxy bills against. Map it to
        // the canonical provider so usage aggregates and the Grok quota card
        // merge with xAI-official records.
        "grok-build" => "xai-official".to_string(),
        other => other.to_lowercase(),
    }
}

/// Fallback for unknown provider ids: prefer a slugified display name
/// (e.g. "ollama cloud" → "ollama-cloud") over the raw id.
fn fallback_provider_name(provider_id: &str, provider_names: &HashMap<String, String>) -> String {
    if let Some(display) = provider_names.get(provider_id) {
        let slug: String = display
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() {
                    c.to_ascii_lowercase()
                } else {
                    '-'
                }
            })
            .collect();
        let slug = slug.trim_matches('-').to_string();
        if !slug.is_empty() {
            return slug;
        }
    }
    provider_id.to_lowercase()
}

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

/// Key under which the console-API watermark is persisted in the token
/// store's `sync_watermarks` table. A cold start resumes from it instead of
/// walking the entire remote history again (279+ page requests, ~90s).
const WATERMARK_KEY: &str = "dim_console_last_id";

/// Polling state for the console API.
struct DimPollState {
    /// Newest `id` already ingested; poll stops fetching pages once it
    /// crosses back to an id ≤ this value.
    last_seen_id: Option<i64>,
    /// Whether the most recent console-API sync ran to completion (every page
    /// up to the watermark / end of history was fetched without error).
    /// Session-scoped: used by the migration to decide whether the records
    /// just loaded cover the whole remote history.
    last_sync_complete: bool,
    /// Whether a full-history backfill has ever completed in this process.
    /// Set only by a sync that started with no persisted watermark (and thus
    /// covered the *whole* remote history — incremental polls only see page
    /// 1, so they cannot prove the historic per-run rows are superseded).
    /// Gates the startup legacy-row purge.
    backfill_done: bool,
}

static POLL_STATE: Mutex<DimPollState> = Mutex::new(DimPollState {
    last_seen_id: None,
    last_sync_complete: false,
    backfill_done: false,
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

    /// Sync the console API. Startup resumes from the persisted watermark
    /// (one page when nothing new happened); a store without one — fresh
    /// install, or a store predating the watermark table — pays a single
    /// full backfill and records it.
    fn load(&self) -> Vec<TokenRecord> {
        let watermark = Self::resume_watermark();
        let from_scratch = watermark.is_none();
        let (items, complete) = match Self::sync(watermark) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("DimAgent API sync failed: {e}");
                return Vec::new();
            }
        };
        let mut records = Self::finish_sync(items, complete, from_scratch);
        records.extend(Self::load_local_supplement());
        records
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
        let mut records = Self::finish_sync(items, complete, false);
        records.extend(Self::load_local_supplement());
        records
    }

    fn is_available(&self) -> bool {
        std::env::var("DIMAGENT_SESSION_COOKIE").is_ok()
    }
}

impl DimSource {
    /// Whether a complete backfill covering the *whole* remote history has
    /// finished in this process. Only then do the freshly loaded dim records
    /// demonstrably supersede the legacy per-run rows, which the
    /// fingerprint-guarded purge relies on.
    pub fn full_backfill_done() -> bool {
        POLL_STATE.lock().unwrap().backfill_done
    }

    /// Console-API watermark to resume a startup sync from:
    /// - the in-process state when this process already synced, else
    /// - the value persisted in the token store, else
    /// - `None` → one full backfill (its completion is persisted).
    fn resume_watermark() -> Option<i64> {
        if let Some(id) = POLL_STATE.lock().unwrap().last_seen_id {
            return Some(id);
        }
        match crate::store::TokenStore::open_default().get_sync_watermark(WATERMARK_KEY) {
            Some(id) => {
                {
                    let mut state = POLL_STATE.lock().unwrap();
                    state.last_seen_id = Some(id);
                }
                tracing::info!(
                    "DimAgent API: resuming from persisted watermark id={id} \
                     (skipping the full history walk)"
                );
                Some(id)
            }
            None => None,
        }
    }

    /// Read third-party provider usage from the local dimcode SQLite
    /// (`usage_run_stats`), excluding Dim's own OAuth channel (covered by
    /// the console API), the named channels with their own per-request meter
    /// (workbuddy → `dim-agent`, grok-build-proxy → grok proxy log,
    /// ollama-cloud-proxy → `ollama-proxy`, cc-proxy → `cc-proxy`), and any
    /// channel aimed at the CPA loopback — see [`cpa_loopback_addrs`], which
    /// covers every future CPA upstream without touching this list. Returns an
    /// empty vec when the DB is missing or unreadable (graceful degradation).
    fn load_local_supplement() -> Vec<TokenRecord> {
        let path = Self::local_db_path();
        if !path.exists() {
            return Vec::new();
        }
        let conn = match rusqlite::Connection::open_with_flags(
            &path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        ) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("Failed to open dim local DB {:?}: {e}", path);
                return Vec::new();
            }
        };
        let provider_names = local_provider_names(&conn);
        let cpa_channels = cpa_metered_providers(&conn);
        let sql = "SELECT providerId, modelId, startedAt, endedAt, createdAt,
                          inputTokens, outputTokens,
                          cacheReadTokens, cacheWriteTokens, cost
                   FROM usage_run_stats
                   WHERE status = 'completed'
                     AND providerId != ?1
                     AND providerId != ?2
                     AND providerId != ?3
                     AND providerId != ?4
                     AND providerId != ?5
                     AND (inputTokens > 0 OR outputTokens > 0
                          OR cacheReadTokens > 0 OR cacheWriteTokens > 0)";
        let mut stmt = match conn.prepare(sql) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("Failed to prepare dim local DB query: {e}");
                return Vec::new();
            }
        };
        let rows = stmt.query_map(
            [
                DIM_OAUTH_PROVIDER,
                WORKBUDDY_PROVIDER,
                GROK_BUILD_PROXY_PROVIDER,
                OLLAMA_CLOUD_PROXY_PROVIDER,
                CC_PROXY_PROVIDER,
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, Option<i64>>(7)?,
                    row.get::<_, Option<i64>>(8)?,
                    row.get::<_, String>(9)?,
                ))
            },
        );
        let mut records = Vec::new();
        match rows {
            Ok(iter) => {
                for row in iter.flatten() {
                    let (
                        provider_id,
                        model_id,
                        started_at,
                        ended_at,
                        created_at,
                        input_tokens,
                        output_tokens,
                        cache_read_tokens,
                        cache_write_tokens,
                        cost_json,
                    ) = row;
                    // Calls that went *through* CLIProxyAPI are metered
                    // per-request by its usage plugins, so this channel's
                    // per-run row would count them a second time.
                    if cpa_channels.contains(&provider_id) {
                        continue;
                    }
                    if let Some(rec) = local_row_to_record(
                        &provider_id,
                        &model_id,
                        started_at.as_deref(),
                        ended_at.as_deref(),
                        &created_at,
                        input_tokens,
                        output_tokens,
                        cache_read_tokens,
                        cache_write_tokens,
                        &cost_json,
                        &provider_names,
                    ) {
                        if super::ollama_run_record_superseded(&rec) {
                            continue;
                        }
                        records.push(rec);
                    }
                }
            }
            Err(e) => tracing::warn!("Failed to iterate dim local DB rows: {e}"),
        }
        if !records.is_empty() {
            tracing::info!("Loaded {} dim local-supplement records", records.len());
        }
        records
    }

    fn local_db_path() -> PathBuf {
        std::env::var("DIM_LOCAL_DB_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|_| super::home_dir().join(LOCAL_DB_DEFAULT))
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

    fn sync_inner(cookie: &str, last_seen: Option<i64>) -> Result<(Vec<LogItem>, bool), String> {
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
                tracing::warn!("DimAgent API backfill hit MAX_PAGES ({MAX_PAGES}); stopping");
                return Ok((items, false));
            }
        }
    }

    /// Convert fetched items to records and advance the in-process watermark
    /// when the sync completed. Items with zero total tokens (e.g. failed
    /// calls) are dropped, matching the dashboard's zero-token convention.
    fn commit(items: Vec<LogItem>, complete: bool) -> Vec<TokenRecord> {
        let records: Vec<TokenRecord> = items.iter().filter_map(item_to_record).collect();
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

    /// Convert fetched items to records, advance the watermark to the store
    /// when the sync completed, and mark full-history backfills.
    ///
    /// The persisted watermark lets the next cold start resume from it
    /// instead of re-walking the entire remote history (~279 pages, ~90s).
    /// `from_scratch` marks a sync that started with no watermark at all and
    /// therefore covered the *whole* remote history — only such a sync may
    /// arm the legacy per-run row purge (`full_backfill_done`), because a
    /// watermark-resumed sync only sees records newer than the watermark and
    /// its fingerprint set must never be treated as "the whole history".
    fn finish_sync(items: Vec<LogItem>, complete: bool, from_scratch: bool) -> Vec<TokenRecord> {
        let records = Self::commit(items, complete);
        if complete {
            let last_seen = POLL_STATE.lock().unwrap().last_seen_id;
            if let Some(id) = last_seen {
                crate::store::TokenStore::open_default()
                    .set_sync_watermark(WATERMARK_KEY, id);
            }
            if from_scratch {
                POLL_STATE.lock().unwrap().backfill_done = true;
            }
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

/// Build providerId → displayName from the local `providers` table, so
/// unknown provider ids can be labeled with their human-readable name.
fn local_provider_names(conn: &rusqlite::Connection) -> HashMap<String, String> {
    let mut map = HashMap::new();
    if let Ok(mut stmt) = conn.prepare("SELECT providerId, displayName FROM providers") {
        if let Ok(rows) = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        }) {
            for row in rows.flatten() {
                map.insert(row.0, row.1);
            }
        }
    }
    map
}

/// Map one local `usage_run_stats` row (third-party provider) to a
/// [`TokenRecord`]. OpenAI cache convention: `inputTokens` includes
/// `cacheReadTokens` → subtract to get the non-cached input.
fn local_row_to_record(
    provider_id: &str,
    model_id: &str,
    started_at: Option<&str>,
    ended_at: Option<&str>,
    created_at: &str,
    input_tokens: i64,
    output_tokens: i64,
    cache_read_tokens: Option<i64>,
    cache_write_tokens: Option<i64>,
    cost_json: &str,
    provider_names: &HashMap<String, String>,
) -> Option<TokenRecord> {
    let cache_read = cache_read_tokens.unwrap_or(0).max(0);
    let cache_write = cache_write_tokens.unwrap_or(0).max(0);
    let effective_input = (input_tokens - cache_read).max(0);
    let total = effective_input + output_tokens + cache_read + cache_write;
    if total == 0 {
        return None;
    }

    // Prefer completion time, fall back to start, then creation.
    let ts = ended_at.or(started_at).unwrap_or(created_at);
    let (date, time) = super::parse_iso_timestamp(ts);

    let provider = map_local_provider(provider_id);
    // Unknown provider ids (mapped to their lowercased raw id) get a
    // readable slug from the display name instead.
    let provider = if provider == provider_id.to_lowercase() {
        fallback_provider_name(provider_id, provider_names)
    } else {
        provider
    };
    // Keep the raw provider id as original_provider so display_cost() can
    // distinguish this channel (e.g. ollama-cloud subscription billing)
    // from records merged into the same vendor by vendor_merge.toml.
    let original_provider = Some(provider_id.to_string());

    // Exact catalog-computed USD cost (Dim's provider catalog). Stored so
    // display_cost() can fall back to it; ollama-cloud rows are billed with
    // the empirical subscription rate regardless (see pricing.rs).
    let cost = serde_json::from_str::<serde_json::Value>(cost_json)
        .ok()
        .and_then(|v| v.get("totalCostUsd").and_then(|c| c.as_f64()))
        .unwrap_or(0.0);

    Some(TokenRecord {
        date: date.into(),
        time,
        api_key_prefix: "N/A".into(),
        provider: provider.into(),
        original_provider,
        model: model_id.into(),
        source: "dim".into(),
        input_tokens: effective_input,
        output_tokens,
        cache_read_tokens: cache_read,
        cache_write_tokens: cache_write,
        total_tokens: total,
        cost,
        ttft_ms: None,
        tps: None,
    })
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
        date: date.into(),
        time,
        api_key_prefix: "N/A".into(),
        provider: "dim".into(),
        original_provider: Some("dim".to_string()),
        model: item.model_name.as_str().into(),
        source: "dim".into(),
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

    #[test]
    fn cpa_channel_detection_matches_loopback_authority_only() {
        // The shared CLIProxyAPI instance (per-request metered by its plugins).
        assert_eq!(
            url_authority("http://127.0.0.1:8317/v1").as_deref(),
            Some("127.0.0.1:8317")
        );
        // StepFun reached directly by Dim — must stay in the supplement.
        assert_eq!(
            url_authority("https://api.stepfun.com/step_plan/v1").as_deref(),
            Some("api.stepfun.com")
        );
        assert_eq!(url_authority(""), None);
        assert_eq!(url_authority("localhost:8317"), None);
    }

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

    // ─── Local SQLite supplement ─────────────────────────────────────────

    fn sample_provider_names() -> HashMap<String, String> {
        HashMap::from([
            (
                "custom-ollama-cloud-042036d3".to_string(),
                "ollama cloud".to_string(),
            ),
            (
                "dimcode-api-oauth".to_string(),
                "DimAgent OAuth".to_string(),
            ),
        ])
    }

    fn sample_local_row() -> (
        String,
        String,
        Option<String>,
        Option<String>,
        String,
        i64,
        i64,
        Option<i64>,
        Option<i64>,
        String,
    ) {
        (
            "custom-ollama-cloud-042036d3".to_string(),
            "deepseek-v4-flash:0731".to_string(),
            Some("2026-09-06T05:35:21.551Z".to_string()),
            Some("2026-09-06T05:37:26.777Z".to_string()),
            "2026-09-06T05:37:26.789Z".to_string(),
            1_193_902,
            12_362,
            Some(0),
            Some(0),
            r#"{"totalCostUsd":0.17304489199999998}"#.to_string(),
        )
    }

    #[test]
    fn maps_local_ollama_cloud_row() {
        let (pid, mid, s, e, c, i, o, cr, cw, cost) = sample_local_row();
        let r = local_row_to_record(
            &pid,
            &mid,
            s.as_deref(),
            e.as_deref(),
            &c,
            i,
            o,
            cr,
            cw,
            &cost,
            &sample_provider_names(),
        )
        .unwrap();
        assert_eq!(r.provider, "ollama-cloud");
        assert_eq!(
            r.original_provider.as_deref(),
            Some("custom-ollama-cloud-042036d3")
        );
        assert_eq!(r.source, "dim");
        assert_eq!(r.model, "deepseek-v4-flash:0731");
        // inputTokens includes cacheReadTokens → subtract (0 here).
        assert_eq!(r.input_tokens, 1_193_902);
        assert_eq!(r.output_tokens, 12_362);
        assert_eq!(r.total_tokens, 1_193_902 + 12_362);
        assert_eq!(r.cost, 0.17304489199999998);
        // Completion time preferred over start/creation.
        assert!(r.time.starts_with("2026-09-06T05:37:26"));
        assert_eq!(r.date, "2026-09-06");
        assert_eq!(r.ttft_ms, None);
        assert_eq!(r.tps, None);
    }

    #[test]
    fn maps_local_grok_build_row_to_xai_official() {
        let (pid, mid, s, e, c, i, o, cr, cw, cost) = (
            "grok-build".to_string(),
            "grok-4.6".to_string(),
            Some("2026-09-06T14:04:46.707Z".to_string()),
            Some("2026-09-06T14:15:21.333Z".to_string()),
            "2026-09-06T14:15:21.340Z".to_string(),
            2_382_437,
            12_671,
            Some(2_085_504),
            None,
            r#"{"totalCostUsd":1.712644}"#.to_string(),
        );
        let r = local_row_to_record(
            &pid,
            &mid,
            s.as_deref(),
            e.as_deref(),
            &c,
            i,
            o,
            cr,
            cw,
            &cost,
            &sample_provider_names(),
        )
        .unwrap();
        // Grok Build is billed through xAI's official API → same SuperGrok
        // subscription as grok-cli xai-official records.
        assert_eq!(r.provider, "xai-official");
        assert_eq!(r.original_provider.as_deref(), Some("grok-build"));
        assert_eq!(r.model, "grok-4.6");
        // inputTokens includes cacheReadTokens → subtract.
        assert_eq!(r.input_tokens, 2_382_437 - 2_085_504);
        assert_eq!(r.cache_read_tokens, 2_085_504);
        assert_eq!(r.cost, 1.712644);
    }

    #[test]
    fn local_row_subtracts_cache_from_input() {
        let (pid, mid, s, e, c, _i, _o, _cr, _cw, cost) = sample_local_row();
        let r = local_row_to_record(
            &pid,
            &mid,
            s.as_deref(),
            e.as_deref(),
            &c,
            500_000,
            1_000,
            Some(400_000),
            Some(0),
            &cost,
            &sample_provider_names(),
        )
        .unwrap();
        assert_eq!(r.input_tokens, 100_000);
        assert_eq!(r.cache_read_tokens, 400_000);
        assert_eq!(r.total_tokens, 100_000 + 1_000 + 400_000);
    }

    #[test]
    fn local_row_drops_zero_total() {
        let (pid, mid, s, e, c, _i, _o, _cr, _cw, cost) = sample_local_row();
        assert!(
            local_row_to_record(
                &pid,
                &mid,
                s.as_deref(),
                e.as_deref(),
                &c,
                0,
                0,
                Some(0),
                Some(0),
                &cost,
                &sample_provider_names(),
            )
            .is_none()
        );
    }

    #[test]
    fn unknown_local_provider_uses_display_name_slug() {
        let names = HashMap::from([("custom-foo-bar".to_string(), "Foo Bar Cloud".to_string())]);
        let r = local_row_to_record(
            "custom-foo-bar",
            "some-model",
            Some("2026-09-06T05:00:00Z"),
            None,
            "2026-09-06T05:00:00Z",
            10,
            10,
            Some(0),
            Some(0),
            "{}",
            &names,
        )
        .unwrap();
        assert_eq!(r.provider, "foo-bar-cloud");
        assert_eq!(r.original_provider.as_deref(), Some("custom-foo-bar"));
    }

    #[test]
    fn local_db_path_defaults_to_home() {
        let p = DimSource::local_db_path();
        assert!(p.ends_with(".dimcode/v2/dimcode.sqlite"));
    }
}
