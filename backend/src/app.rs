//! Application setup and lifecycle.
//!
//! Owns shared state, background data refresh, and router assembly.

use crate::models::TokenRecord;
use crate::quota::QuotaFetcher;
use crate::routes;
use crate::sources::load_all_sources;
use crate::store::TokenStore;
use axum::{
    routing::{get, post},
    Router,
};
use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::ServeDir;

/// Shared application state (thread-safe, arc-locked records).
#[derive(Clone)]
pub struct AppState {
    pub records: Arc<RwLock<Vec<TokenRecord>>>,
    /// Dedicated SQLite store: durable source of truth for token history.
    /// `records` is always rebuilt as a snapshot of this store.
    pub store: Arc<TokenStore>,
    pub quota_fetcher: Arc<QuotaFetcher>,
}

impl AppState {
    /// Create the application state with an initial data load.
    pub fn new() -> Self {
        let store = Arc::new(TokenStore::open_default());

        // Restore history from the durable store, then ingest whatever the
        // session logs contain that isn't persisted yet.
        let db_records = store.load_all();
        let seen: HashSet<_> = db_records.iter().map(TokenRecord::fingerprint).collect();
        let source_records = load_all_sources();
        let source_total = source_records.len();
        let new_from_sources: Vec<TokenRecord> = source_records
            .into_iter()
            .filter(|r| !seen.contains(&r.fingerprint()))
            .collect();
        let ingested = store.insert_batch(&new_from_sources);

        let records = store.load_all();
        tracing::info!(
            "Initial load: {} records restored from token store ({} new from sources, {} total scanned)",
            records.len(),
            ingested,
            source_total
        );
        Self {
            records: Arc::new(RwLock::new(records)),
            store,
            quota_fetcher: Arc::new(QuotaFetcher::new()),
        }
    }

    /// Incrementally refresh records from all data sources.
    ///
    /// Returns the number of new records persisted.
    ///
    /// Newly discovered source records are written into the durable store
    /// (idempotent, fingerprint-unique), then the in-memory snapshot is
    /// rebuilt from the store. Memory therefore always mirrors the DB, and
    /// any record that failed to persist is retried on the next refresh
    /// because it never entered memory.
    pub async fn refresh_records(&self) -> usize {
        let new_records = load_all_sources();

        // Phase 1: Filter records not yet in memory (which mirrors the DB).
        let records_to_add: Vec<TokenRecord> = {
            let guard = self.records.read().await;
            let seen: HashSet<_> = guard.iter().map(TokenRecord::fingerprint).collect();
            new_records
                .into_iter()
                .filter(|r| !seen.contains(&r.fingerprint()))
                .collect()
        };

        if records_to_add.is_empty() {
            tracing::debug!(
                "Refreshed data: {} records (unchanged)",
                self.records.read().await.len()
            );
            return 0;
        }

        // Phase 2: Persist new records outside any lock. A failed batch is
        // rolled back and retried next cycle, so the store never contains
        // partial data.
        let inserted = self.store.insert_batch(&records_to_add);

        // Phase 3: Rebuild the in-memory snapshot from the store.
        let reloaded = self.store.load_all();
        let mut guard = self.records.write().await;
        *guard = reloaded;
        tracing::info!(
            "Refreshed data: {} records ({} new, {} persisted)",
            guard.len(),
            records_to_add.len(),
            inserted
        );
        inserted
    }

    /// Spawn a background task that reloads data sources periodically.
    ///
    /// Uses **incremental** refresh: only adds new records, never removes
    /// existing ones. This preserves historical data even when a source
    /// directory (e.g. a project's runtime folder) is deleted.
    pub fn spawn_refresh_task(&self) {
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
        });
    }
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
pub async fn serve(router: Router) {
    let port = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or(3000);

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    tracing::info!("Token Stats server listening on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, router).await.unwrap();
}
