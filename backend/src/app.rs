//! Application setup and lifecycle.
//!
//! Owns shared state, background data refresh, and router assembly.

use crate::models::TokenRecord;
use crate::quota::QuotaFetcher;
use crate::routes;
use crate::sources::{load_all_sources, load_changed_sources, DimSource};
use crate::store::{PendingBuffer, TokenStore};
use axum::{
    routing::{get, post},
    Router,
};
use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::ServeDir;

/// Records are written to disk in one batch at most this often.
const FLUSH_DELAY: Duration = Duration::from_secs(120);
/// How often the flush task checks whether a write is due.
const FLUSH_CHECK_INTERVAL: Duration = Duration::from_secs(30);

/// In-memory records plus the fingerprint set used to dedup new arrivals,
/// behind a single lock so the two can never diverge.
///
/// `seen` is a *maintained* superset: a fingerprint is inserted when its
/// record is added and never removed. A dropped twin's fingerprint staying
/// in the set is harmless — it merely prevents re-adding a genuine
/// duplicate — so the set only grows, which removes the per-cycle HashSet
/// rebuild that the refresh fast path used to pay on every pass.
pub struct RecordTable {
    pub records: Vec<TokenRecord>,
    pub seen: HashSet<u64>,
}

impl RecordTable {
    pub fn new(records: Vec<TokenRecord>) -> Self {
        let seen = records.iter().map(TokenRecord::fingerprint).collect();
        Self { records, seen }
    }

    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn iter(&self) -> std::slice::Iter<'_, TokenRecord> {
        self.records.iter()
    }

    /// Insert a single record if its fingerprint is new. Returns true if it
    /// was added.
    pub fn push(&mut self, r: TokenRecord) -> bool {
        let fp = r.fingerprint();
        if self.seen.insert(fp) {
            self.records.push(r);
            true
        } else {
            false
        }
    }

    /// Insert many records, skipping any whose fingerprint is already present.
    pub fn extend(&mut self, new: impl IntoIterator<Item = TokenRecord>) {
        for r in new {
            self.push(r);
        }
    }

    /// Replace the entire table (store/backup restore). Rebuilds the
    /// fingerprint set from the new records.
    pub fn replace_all(&mut self, records: Vec<TokenRecord>) {
        self.seen = records.iter().map(TokenRecord::fingerprint).collect();
        self.records = records;
    }
}

impl std::ops::Deref for RecordTable {
    type Target = Vec<TokenRecord>;
    fn deref(&self) -> &Vec<TokenRecord> {
        &self.records
    }
}

impl std::ops::DerefMut for RecordTable {
    fn deref_mut(&mut self) -> &mut Vec<TokenRecord> {
        &mut self.records
    }
}

/// Shared application state (thread-safe, arc-locked records).
#[derive(Clone)]
pub struct AppState {
    /// Live in-memory records the frontend reads. Updated immediately on
    /// refresh; a superset of the store (memory = DB + queued records).
    pub records: Arc<RwLock<RecordTable>>,
    /// Dedicated SQLite store: durable source of truth for token history.
    /// Writes are deferred through `pending` and batched at most once per
    /// `FLUSH_DELAY` to avoid frequent SSD writes.
    pub store: Arc<TokenStore>,
    /// Debounced disk-write buffer: records queued by refresh, drained by
    /// the flush task (and unconditionally on shutdown).
    pub pending: Arc<PendingBuffer>,
    pub quota_fetcher: Arc<QuotaFetcher>,
}

impl AppState {
    /// Create the application state with an initial data load.
    pub fn new() -> Self {
        let store = Arc::new(TokenStore::open_default());

        // Restore history from the durable store, then ingest whatever the
        // session logs contain that isn't persisted yet.
        let db_records = store.load_all();
        // Filtering with a growing set also dedups internal duplicates that
        // INSERT OR IGNORE would previously have dropped before the second
        // full reload.
        let mut seen: HashSet<u64> = db_records.iter().map(TokenRecord::fingerprint).collect();
        let source_records = load_all_sources();
        let source_total = source_records.len();
        let has_cc = source_records.iter().any(|r| r.source == "commandcode");

        // ── Dim collection migration ────────────────────────────────────
        // The dim source now ingests per-request records from the DimAgent
        // console API. Legacy per-run rows (collected from the local SQLite
        // before the API source) would double-count the same usage at a
        // coarser granularity, so once a complete API backfill has delivered
        // records we drop the legacy rows (fingerprint-guarded: only rows
        // NOT matching an API record are removed). Until the API backfill
        // completes (or is unavailable), the legacy history is kept intact.
        let dim_keep: HashSet<u64> = source_records
            .iter()
            .filter(|r| r.source == "dim")
            .map(TokenRecord::fingerprint)
            .collect();
        let dim_migrated = !dim_keep.is_empty() && DimSource::last_sync_completed();
        if dim_migrated {
            store.purge_dim_legacy(&dim_keep);
        }

        let new_from_sources: Vec<TokenRecord> = source_records
            .into_iter()
            .filter(|r| seen.insert(r.fingerprint()))
            .collect();
        let ingested = store.insert_batch(&new_from_sources);
        // Newly ingested exclusive cmd rows can pair with older inclusive
        // twins that were already in the store. Only worth scanning for when
        // any cmd rows arrived (open() already collapses on startup).
        if has_cc {
            store.collapse_commandcode_inclusive_twins();
        }
        // Newly ingested named Codex rows pair with older unknown-model
        // twins left by the incremental pre-scan skip.
        store.collapse_unknown_codex_twins();

        // Build memory from the rows we already have instead of re-reading
        // the whole DB a second time; apply the same in-memory twin-drop the
        // refresh path uses and the load_all ORDER BY for identical shape.
        let mut records: Vec<TokenRecord> = db_records
            .into_iter()
            .filter(|r| {
                // Drop the legacy per-run dim rows the migration purge just
                // removed from the store so memory and DB stay in sync.
                !(dim_migrated
                    && r.source == "dim"
                    && !dim_keep.contains(&r.fingerprint()))
            })
            .chain(new_from_sources)
            .collect();
        if has_cc {
            drop_commandcode_inclusive_twins(&mut records);
        }
        drop_unknown_codex_twins(&mut records);
        records.sort_by(|a, b| {
            a.time
                .cmp(&b.time)
                .then_with(|| a.source.cmp(&b.source))
                .then_with(|| a.provider.cmp(&b.provider))
                .then_with(|| a.model.cmp(&b.model))
        });
        tracing::info!(
            "Initial load: {} records restored from token store ({} new from sources, {} total scanned)",
            records.len(),
            ingested,
            source_total
        );
        Self {
            records: Arc::new(RwLock::new(RecordTable::new(records))),
            store,
            pending: Arc::new(PendingBuffer::new()),
            quota_fetcher: Arc::new(QuotaFetcher::new()),
        }
    }

    /// Incrementally refresh records from all data sources.
    ///
    /// Uses **incremental** loading: only files whose (mtime, size) changed
    /// since the last pass are re-parsed, so the 2.4GB codex history (and
    /// other large sources) is not re-read every 30s.
    ///
    /// Returns the number of new records discovered.
    ///
    /// Newly discovered records are published to memory **immediately** so
    /// the frontend always reads the latest data, and queued for a deferred
    /// disk write (see [`Self::flush_pending_if_due`]). Memory and the
    /// pending queue are updated under the same lock, so a shutdown flush
    /// can never observe memory without the matching queued records.
    pub async fn refresh_records(&self) -> usize {
        let new_records = load_changed_sources();

        // Phase 1: Fast path — check against the maintained fingerprint set
        // under a read lock. Most refreshes find nothing new and never take
        // the write lock, and (unlike before) no per-cycle HashSet is built.
        let candidates: Vec<TokenRecord> = {
            let guard = self.records.read().await;
            new_records
                .into_iter()
                .filter(|r| !guard.seen.contains(&r.fingerprint()))
                .collect()
        };

        if candidates.is_empty() {
            tracing::debug!(
                "Refreshed data: {} records (unchanged)",
                self.records.read().await.len()
            );
            return 0;
        }

        // Phase 2: Re-filter under the write lock — a concurrent refresh may
        // have added some of these between Phase 1 and now. Publish to memory
        // and queue for the deferred write under the same lock (no await
        // inside, so the two can never diverge).
        let mut guard = self.records.write().await;
        let records_to_add: Vec<TokenRecord> = candidates
            .into_iter()
            .filter(|r| !guard.seen.contains(&r.fingerprint()))
            .collect();
        if records_to_add.is_empty() {
            return 0;
        }
        // New cmd rows only affect twin-collapsing when cmd rows actually
        // arrived; skip the full-vec and full-DB scans otherwise.
        let has_cc = records_to_add.iter().any(|r| r.source == "commandcode");
        let has_codex = records_to_add.iter().any(|r| r.source == "codex");
        guard.extend(records_to_add.iter().cloned());
        if has_cc {
            drop_commandcode_inclusive_twins(&mut guard.records);
        }
        if has_codex {
            drop_unknown_codex_twins(&mut guard.records);
        }
        let added = records_to_add.len();
        self.pending.queue(records_to_add);
        // DB-level twin collapse is deferred to startup (AppState::new already
        // collapses the whole store) and to the flush path. Scanning the full
        // DB here on every refresh where cc/codex rows arrive was a steady
        // SSD/CPU cost for no runtime benefit — the in-memory `seen` set and
        // the `drop_*_twins` above already keep the live view deduplicated.
        tracing::info!(
            "Refreshed data: {} records ({} new, queued for deferred write)",
            guard.len(),
            added
        );
        added
    }

    /// Write a batch to the store. On total failure the batch is re-queued
    /// so the next flush retries it (records stay visible in memory either
    /// way). Returns the number of records persisted.
    async fn write_batch(&self, batch: Vec<TokenRecord>) -> usize {
        if batch.is_empty() {
            return 0;
        }
        let inserted = self.store.insert_batch(&batch);
        if batch.iter().any(|r| r.source == "commandcode") {
            self.store.collapse_commandcode_inclusive_twins();
        }
        if batch.iter().any(|r| r.source == "codex") {
            self.store.collapse_unknown_codex_twins();
        }
        if inserted == 0 {
            // Transaction rolled back — nothing was written. Re-queue so the
            // next flush retries.
            self.pending.queue(batch);
            tracing::warn!(
                "Token store write failed; {} record(s) re-queued for retry",
                self.pending.len()
            );
            return 0;
        }
        tracing::info!("Flushed {} queued record(s) to token store", inserted);
        inserted
    }

    /// Drain the pending buffer unconditionally (shutdown path).
    pub async fn flush_pending(&self) -> usize {
        self.write_batch(self.pending.take_all()).await
    }

    /// Drain the pending buffer if a write is due (at most once per
    /// `FLUSH_DELAY`).
    pub async fn flush_pending_if_due(&self) -> usize {
        self.write_batch(self.pending.take_if_due(FLUSH_DELAY, Instant::now()))
            .await
    }

    /// Spawn a background task that reloads data sources periodically.
    ///
    /// Uses **incremental** refresh: only adds new records, never removes
    /// existing ones. This preserves historical data even when a source
    /// directory (e.g. a project's runtime folder) is deleted.
    ///
    /// Returns the task handle so shutdown can stop it before the final
    /// flush.
    pub fn spawn_refresh_task(&self) -> tokio::task::JoinHandle<()> {
        let state = self.clone();
        let refresh_interval = std::env::var("REFRESH_INTERVAL_SECS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(30);

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(refresh_interval));
            loop {
                interval.tick().await;
                state.refresh_records().await;
            }
        })
    }

    /// Spawn the background task that drains the pending buffer into SQLite
    /// at most once per `FLUSH_DELAY`.
    pub fn spawn_flush_task(&self) -> tokio::task::JoinHandle<()> {
        let state = self.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(FLUSH_CHECK_INTERVAL);
            loop {
                interval.tick().await;
                state.flush_pending_if_due().await;
            }
        })
    }
}

/// Hash key for twin-pairing cmd rows (allocation-free; see fingerprint()).
fn cc_twin_key(r: &TokenRecord, exclusive_input: i64) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    r.time.hash(&mut h);
    r.provider.hash(&mut h);
    r.model.hash(&mut h);
    r.output_tokens.hash(&mut h);
    r.cache_read_tokens.hash(&mut h);
    exclusive_input.hash(&mut h);
    h.finish()
}

/// Keep the exclusive cmd row when an older cache-inclusive twin is present.
fn drop_commandcode_inclusive_twins(records: &mut Vec<TokenRecord>) {
    let exclusive: std::collections::HashSet<u64> = records
        .iter()
        .filter(|r| r.source == "commandcode" && r.cache_read_tokens > 0)
        .map(|r| cc_twin_key(r, r.input_tokens))
        .collect();
    records.retain(|r| {
        if r.source != "commandcode" || r.cache_read_tokens <= 0 {
            return true;
        }
        let exclusive_input = r.input_tokens - r.cache_read_tokens;
        if exclusive_input < 0 {
            return true;
        }
        !exclusive.contains(&cc_twin_key(r, exclusive_input))
    });
}

/// Hash key for pairing a Codex unknown-model row with its named twin.
/// Provider is excluded: incremental skip defaulted it to openai→ainaba.
fn codex_unknown_twin_key(r: &TokenRecord) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    r.time.hash(&mut h);
    r.input_tokens.hash(&mut h);
    r.output_tokens.hash(&mut h);
    r.cache_read_tokens.hash(&mut h);
    h.finish()
}

/// Keep the named Codex row when an unknown-model twin is present.
fn drop_unknown_codex_twins(records: &mut Vec<TokenRecord>) {
    let named: std::collections::HashSet<u64> = records
        .iter()
        .filter(|r| r.source == "codex" && r.model != "unknown")
        .map(codex_unknown_twin_key)
        .collect();
    if named.is_empty() {
        return;
    }
    records.retain(|r| {
        if r.source != "codex" || r.model != "unknown" {
            return true;
        }
        !named.contains(&codex_unknown_twin_key(r))
    });
}

/// Build the Axum router with all API routes, CORS, and static file serving.
pub fn build_router(state: AppState) -> Router {
    let state = Arc::new(state);

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let api_routes = Router::new()
        .route("/api/stats", get(routes::get_stats))
        .route("/api/rpm", get(routes::get_rpm))
        .route("/api/tps", get(routes::get_tps))
        .route("/api/requests", get(routes::get_requests))
        .route("/api/filters", get(routes::get_filters))
        .route("/api/quota", get(routes::get_quota))
        .route("/api/xunfei", get(routes::get_xunfei))
        .route("/api/pricing", get(routes::get_pricing))
        .route("/api/pricing/reload", post(routes::reload_pricing))
        .route("/api/export", get(routes::export_data))
        .route("/api/refresh", post(routes::refresh_data))
        .route("/api/restore", post(routes::restore_backup))
        .route("/api/store/info", get(routes::get_store_info))
        .route("/api/store/restore", post(routes::restore_store))
        .route("/api/ainaiba-credit", get(routes::get_ainaiba_credit))
        .route(
            "/api/settings/advanced-models",
            get(routes::get_advanced_models).post(routes::update_advanced_models),
        )
        .route(
            "/api/settings/subscriptions",
            get(routes::get_subscription_settings).post(routes::update_subscription_settings),
        );

    Router::new()
        .merge(api_routes)
        .fallback_service(ServeDir::new("static").append_index_html_on_directories(true))
        .layer(cors)
        .with_state(state)
}

/// Start the HTTP server on the configured port.
///
/// On SIGINT/SIGTERM the server drains in-flight requests, stops the
/// background tasks, and writes everything still queued to disk exactly
/// once before returning.
pub async fn serve(
    router: Router,
    state: AppState,
    refresh_task: tokio::task::JoinHandle<()>,
    flush_task: tokio::task::JoinHandle<()>,
) {
    let port = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or(3000);

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    tracing::info!("Token Stats server listening on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .unwrap();

    // Stop background tasks first so nothing can queue new records, then
    // write everything still pending exactly once. Aborting a task only
    // takes effect at its next await point, so awaiting the handles after
    // abort guarantees no refresh is mid-queue when we flush.
    refresh_task.abort();
    flush_task.abort();
    let _ = refresh_task.await;
    let _ = flush_task.await;
    let flushed = state.flush_pending().await;
    tracing::info!(
        "Shutdown complete; flushed {} pending record(s) to token store",
        flushed
    );
}

/// Resolve when the process receives SIGINT (Ctrl+C) or SIGTERM.
async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}

#[cfg(test)]
mod tests {
    use super::drop_unknown_codex_twins;
    use crate::models::TokenRecord;

    fn rec(source: &str, provider: &str, model: &str, time: &str) -> TokenRecord {
        TokenRecord {
            date: time[..10].to_string(),
            time: time.to_string(),
            api_key_prefix: "N/A".to_string(),
            provider: provider.to_string(),
            original_provider: None,
            model: model.to_string(),
            source: source.to_string(),
            input_tokens: 20,
            output_tokens: 10,
            cache_read_tokens: 80,
            cache_write_tokens: 0,
            total_tokens: 110,
            cost: 0.0,
            ttft_ms: None,
            tps: None,
        }
    }

    #[test]
    fn drops_unknown_codex_when_named_twin_exists() {
        let mut records = vec![
            rec("codex", "ainaba", "unknown", "2026-08-25T10:00:00Z"),
            rec("codex", "fenno", "gpt-5.6-terra", "2026-08-25T10:00:00Z"),
            rec("pi", "ainaba", "unknown", "2026-08-25T10:00:00Z"),
        ];
        drop_unknown_codex_twins(&mut records);
        let models: Vec<_> = records
            .iter()
            .map(|r| (r.source.as_str(), r.provider.as_str(), r.model.as_str()))
            .collect();
        assert_eq!(
            models,
            [
                ("codex", "fenno", "gpt-5.6-terra"),
                ("pi", "ainaba", "unknown"),
            ]
        );
    }

    #[test]
    fn keeps_unknown_codex_when_no_twin() {
        let mut records = vec![rec(
            "codex",
            "ainaba",
            "unknown",
            "2026-08-25T10:00:00Z",
        )];
        drop_unknown_codex_twins(&mut records);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].model, "unknown");
    }
}
