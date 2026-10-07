//! Token Stats Backend — AI token usage dashboard API.
//!
//! Serves aggregated analytics from multiple AI coding tool sources
//! (Pi, Codex, Claude Code, Kimi CLI, OpenCode) with charts, tables,
//! and filtering.

mod aggregator;
mod ainaiba;
mod app;
mod cc_proxy;
mod config;
mod dim_entitlement;
mod glm_proxy;
mod grok_proxy;
mod models;
mod pricing;
mod quota;
mod routes;
mod settings;
mod sources;
mod store;
mod time;
mod xunfei;

use clap::Parser;
use flexi_logger::{
    Cleanup, Criterion, FileSpec, LogSpecification, Naming, WriteMode,
    trc::{FormatConfig, setup_tracing},
    writers::FileLogWriter,
};

/// Route every heap allocation through mimalloc instead of glibc malloc.
///
/// The dashboard keeps ~760k `TokenRecord`s alive, which is millions of small
/// `String` allocations. glibc spreads those across one arena per thread
/// (44 arenas observed) and can only trim the top of the *main* arena, so the
/// startup high-water mark stays resident forever: RSS sat at 1.1 GB while the
/// live data is ~300 MB. mimalloc tracks pages per size class and decommits
/// whole segments once they drain, so the peak is handed back.
#[global_allocator]
static ALLOC: mimalloc::MiMalloc = mimalloc::MiMalloc;

/// Token Stats Backend — AI token usage dashboard API.
#[derive(Parser, Debug)]
#[command(name = "token-stats-backend", version)]
struct Args {
    /// Log level (trace, debug, info, warn, error).  Also reads RUST_LOG env.
    #[arg(short = 'l', long = "log-level", default_value = "info")]
    log_level: String,

    /// Run only the loopback Grok usage recorder.
    #[arg(long)]
    grok_proxy_only: bool,

    /// Run only the loopback Command Code proxy for DimAgent.
    #[arg(long)]
    cc_proxy_only: bool,

    /// Run only the loopback GLM usage proxy for Paseo's glm-acp-agent.
    #[arg(long)]
    glm_proxy_only: bool,
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
            pool_capa: 1 << 14,    // 16K message pool
            message_capa: 1 << 16, // 64K message channel
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
    if args.grok_proxy_only {
        grok_proxy::serve()
            .await
            .expect("Grok usage proxy stopped unexpectedly");
        return;
    }
    if args.cc_proxy_only {
        cc_proxy::serve()
            .await
            .expect("Command Code proxy stopped unexpectedly");
        return;
    }
    if args.glm_proxy_only {
        glm_proxy::serve()
            .await
            .expect("GLM usage proxy stopped unexpectedly");
        return;
    }

    let state = app::AppState::new();
    let refresh_task = state.spawn_refresh_task();
    let flush_task = state.spawn_flush_task();

    let router = app::build_router(state.clone());
    app::serve(router, state, refresh_task, flush_task).await;
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::Args;

    #[test]
    fn parses_grok_proxy_only_mode() {
        let args = Args::try_parse_from(["token-stats-backend", "--grok-proxy-only"])
            .expect("proxy-only flag should parse");

        assert!(args.grok_proxy_only);
    }

    #[test]
    fn parses_cc_proxy_only_mode() {
        let args = Args::try_parse_from(["token-stats-backend", "--cc-proxy-only"])
            .expect("cc-proxy-only flag should parse");

        assert!(args.cc_proxy_only);
    }

    #[test]
    fn parses_glm_proxy_only_mode() {
        let args = Args::try_parse_from(["token-stats-backend", "--glm-proxy-only"])
            .expect("glm-proxy-only flag should parse");

        assert!(args.glm_proxy_only);
    }
}
