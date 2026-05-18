mod aggregator;
mod config;
mod models;
mod quota;
mod routes;
mod sources;
mod xunfei;

use axum::{routing::get, Router};
use clap::Parser;
use flexi_logger::{
    trc::{setup_tracing, FormatConfig},
    writers::FileLogWriter,
    Cleanup, Criterion, FileSpec, LogSpecification, Naming, WriteMode,
};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::ServeDir;

use crate::routes::AppState;
use crate::sources::load_all_sources;

/// Token Stats Backend — AI token usage dashboard API.
#[derive(Parser, Debug)]
#[command(name = "token-stats-backend", version)]
struct Args {
    /// Log level (trace, debug, info, warn, error)
    /// Also reads from RUST_LOG env var.
    #[arg(short = 'l', long = "log-level", default_value = "info")]
    log_level: String,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    // flexi_logger with file rotation:
    //   - writes to logs/token-stats*.log
    //   - rotates at 10 MB per file (Criterion::Size)
    //   - keeps at most 20 rotated files (Cleanup::KeepLogFiles)
    //   - bridges tracing events via flexi_logger::trc::setup_tracing
    //   - respects RUST_LOG env var (takes precedence via LogSpecification::env_or_parse)
    let log_spec =
        LogSpecification::env_or_parse(&args.log_level).expect("Failed to parse log level");

    let _log_handle = setup_tracing(
        log_spec,
        None, // no specfile for on-the-fly changes
        FileLogWriter::builder(
            FileSpec::default()
                .directory("logs")
                .basename("token-stats")
                .suffix("log"),
        )
        .rotate(
            Criterion::Size(10_000_000), // 10 MB
            Naming::Timestamps,
            Cleanup::KeepLogFiles(20),
        )
        .append()
        .write_mode(WriteMode::Async),
        &FormatConfig::default().with_file(true),
    )
    .expect("Failed to set up flexi_logger tracing");

    tracing::info!(
        "Starting Token Stats Backend — log level: {}",
        args.log_level
    );

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
