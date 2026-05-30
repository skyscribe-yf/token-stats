//! Token Stats Backend — AI token usage dashboard API.
//!
//! Serves aggregated analytics from multiple AI coding tool sources
//! (Pi, Codex, Claude Code, Kimi CLI, OpenCode) with charts, tables,
//! and filtering.

mod aggregator;
mod ainaiba;
mod app;
mod config;
mod models;
mod pricing;
mod quota;
mod routes;
mod settings;
mod sources;
mod time;
mod xunfei;

use clap::Parser;
use flexi_logger::{
    trc::{setup_tracing, FormatConfig},
    writers::FileLogWriter,
    Cleanup, Criterion, FileSpec, LogSpecification, Naming, WriteMode,
};

/// Token Stats Backend — AI token usage dashboard API.
#[derive(Parser, Debug)]
#[command(name = "token-stats-backend", version)]
struct Args {
    /// Log level (trace, debug, info, warn, error).  Also reads RUST_LOG env.
    #[arg(short = 'l', long = "log-level", default_value = "info")]
    log_level: String,
}

fn init_logging(log_level: &str) {
    let log_spec = LogSpecification::env_or_parse(log_level).expect("Failed to parse log level");

    let _log_handle = setup_tracing(
        log_spec,
        None,
        FileLogWriter::builder(
            FileSpec::default()
                .directory("logs")
                .basename("token-stats")
                .suffix("log"),
        )
        .rotate(
            Criterion::Size(10_000_000),
            Naming::Timestamps,
            Cleanup::KeepLogFiles(20),
        )
        .append()
        .write_mode(WriteMode::AsyncWith {
            pool_capa: 1 << 14,       // 16K message pool
            message_capa: 1 << 16,    // 64K message channel
            flush_interval: std::time::Duration::from_secs(2),
        }),
        &FormatConfig::default().with_file(true),
    )
    .expect("Failed to set up flexi_logger tracing");

    // Prevent the log handle from being dropped (which would stop the logger).
    // Leak is intentional: the logger must outlive the process.
    std::mem::forget(_log_handle);
}

#[tokio::main]
async fn main() {
    let args = Args::parse();
    init_logging(&args.log_level);
    tracing::info!(
        "Starting Token Stats Backend — log level: {}",
        args.log_level
    );

    pricing::init();
    let state = app::AppState::new();
    state.spawn_refresh_task();

    let router = app::build_router(state);
    app::serve(router).await;
}
