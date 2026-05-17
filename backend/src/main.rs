mod aggregator;
mod models;
mod parser;
mod routes;

use axum::{
    routing::get,
    Router,
};
use std::net::SocketAddr;
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::ServeDir;

use crate::parser::{get_log_path, parse_jsonl_file};
use crate::routes::{get_filters, get_requests, get_stats, AppState};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let log_path = get_log_path();
    tracing::info!("Loading token usage data from: {:?}", log_path);

    let records = parse_jsonl_file(&log_path);
    tracing::info!("Loaded {} records", records.len());

    let state = Arc::new(AppState { records });

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let api_routes = Router::new()
        .route("/api/stats", get(get_stats))
        .route("/api/requests", get(get_requests))
        .route("/api/filters", get(get_filters));

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
