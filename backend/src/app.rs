//! Application setup and lifecycle.
//!
//! Owns shared state, background data refresh, and router assembly.

use crate::models::TokenRecord;
use crate::routes;
use crate::sources::load_all_sources;
use axum::{routing::get, Router};
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
}

impl AppState {
    /// Create the application state with an initial data load.
    pub fn new() -> Self {
        let records = load_all_sources();
        tracing::info!("Initial load: {} records", records.len());
        Self {
            records: Arc::new(RwLock::new(records)),
        }
    }

    /// Spawn a background task that reloads data sources periodically.
    pub fn spawn_refresh_task(&self) {
        let records = self.records.clone();
        let refresh_interval = std::env::var("REFRESH_INTERVAL_SECS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(30);

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(refresh_interval));
            loop {
                interval.tick().await;
                let new_records = load_all_sources();
                let mut guard = records.write().await;
                let old_len = guard.len();
                *guard = new_records;
                if guard.len() != old_len {
                    tracing::info!("Refreshed data: {} records (was {})", guard.len(), old_len);
                } else {
                    tracing::debug!("Refreshed data: {} records (unchanged)", guard.len());
                }
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
        .route("/api/requests", get(routes::get_requests))
        .route("/api/filters", get(routes::get_filters))
        .route("/api/quota", get(routes::get_quota))
        .route("/api/xunfei", get(routes::get_xunfei));

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
