mod aggregator;
mod config;
mod models;
mod sources;
mod quota;
mod routes;
mod xunfei;

use axum::{routing::get, Router};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::ServeDir;

use crate::sources::load_all_sources;
use crate::routes::AppState;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let records = load_all_sources();
    tracing::info!("Initial load: {} records", records.len());

    let state = Arc::new(AppState {
        records: RwLock::new(records),
    });

    // Background refresh task
    let refresh_state = state.clone();
    let refresh_interval = std::env::var("REFRESH_INTERVAL_SECS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(30);

    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(refresh_interval));
        loop {
            interval.tick().await;
            let new_records = load_all_sources();
            let mut records = refresh_state.records.write().await;
            let old_len = records.len();
            *records = new_records;
            if records.len() != old_len {
                tracing::info!(
                    "Refreshed data: {} records (was {})",
                    records.len(),
                    old_len
                );
            } else {
                tracing::debug!("Refreshed data: {} records (unchanged)", records.len());
            }
        }
    });

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

    let app = Router::new()
        .merge(api_routes)
        .fallback_service(ServeDir::new("static").append_index_html_on_directories(true))
        .layer(cors)
        .with_state(state);

    let port = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or(3000);

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    tracing::info!("Token Stats server listening on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
