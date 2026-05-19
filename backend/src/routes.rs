use crate::aggregator;
use crate::app::AppState;
use crate::models::*;
use crate::quota::{QuotaFetcher, QuotaResponse};
use crate::time::{parse_time_bound, tz_offset_to_fixed};
use crate::xunfei::XunfeiFetcher;
use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use std::collections::HashSet;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::sync::Arc;

// ─── Query parameter types ───────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct StatsQuery {
    pub from: Option<String>,
    pub to: Option<String>,
    pub source: Option<String>,
    pub provider: Option<String>,
    /// Timezone offset in minutes from UTC (e.g. 480 for UTC+8, -300 for UTC-5)
    pub tz_offset: Option<i32>,
    /// Aggregation resolution: "day" (default), "4h", "1h"
    pub resolution: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RequestsQuery {
    pub from: Option<String>,
    pub to: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub source: Option<String>,
    pub tz_offset: Option<i32>,
    #[serde(default = "default_page")]
    pub page: usize,
    #[serde(default = "default_limit")]
    pub limit: usize,
}

fn default_page() -> usize {
    1
}

fn default_limit() -> usize {
    50
}

const MAX_LIMIT: usize = 1000;
const MIN_PAGE: usize = 1;

/// Clamp pagination parameters to safe ranges.
fn validate_pagination(page: usize, limit: usize) -> (usize, usize) {
    let page = page.max(MIN_PAGE);
    let limit = limit.clamp(1, MAX_LIMIT);
    (page, limit)
}

// ─── Route handlers ──────────────────────────────────────────────────────────

pub async fn get_stats(
    State(state): State<Arc<AppState>>,
    Query(query): Query<StatsQuery>,
) -> impl IntoResponse {
    let records = state.records.read().await;
    let from = query.from.as_ref().and_then(|s| parse_time_bound(s));
    let to = query.to.as_ref().and_then(|s| parse_time_bound(s));
    let source = query.source.as_deref().filter(|s| !s.is_empty());
    let provider = query.provider.as_deref().filter(|s| !s.is_empty());
    let tz = query.tz_offset.map(tz_offset_to_fixed);
    let resolution = query
        .resolution
        .as_deref()
        .and_then(Resolution::from_str)
        .unwrap_or_default();

    let response = aggregator::aggregate_records(
        &records,
        from.as_ref(),
        to.as_ref(),
        source,
        provider,
        tz.as_ref(),
        resolution,
    );
    Json(response)
}

pub async fn get_requests(
    State(state): State<Arc<AppState>>,
    Query(query): Query<RequestsQuery>,
) -> impl IntoResponse {
    let records = state.records.read().await;
    let from = query.from.as_ref().and_then(|s| parse_time_bound(s));
    let to = query.to.as_ref().and_then(|s| parse_time_bound(s));
    let provider = query.provider.as_deref().filter(|s| !s.is_empty());
    let model = query.model.as_deref().filter(|s| !s.is_empty());
    let source = query.source.as_deref().filter(|s| !s.is_empty());
    let tz = query.tz_offset.map(tz_offset_to_fixed);

    let filtered = aggregator::filter_records(
        &records,
        from.as_ref(),
        to.as_ref(),
        provider,
        model,
        source,
        tz.as_ref(),
    );
    let (page, limit) = validate_pagination(query.page, query.limit);
    let paginated = aggregator::paginate_requests(filtered, page, limit, tz.as_ref());

    Json(paginated)
}

pub async fn get_filters(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let records = state.records.read().await;

    let mut vendors: Vec<String> = records.iter().map(|r| r.provider.clone()).collect();
    vendors.sort();
    vendors.dedup();

    let mut models: Vec<String> = records.iter().map(|r| r.model.clone()).collect();
    models.sort();
    models.dedup();

    let mut sources: Vec<String> = records.iter().map(|r| r.source.clone()).collect();
    sources.sort();
    sources.dedup();

    Json(FilterOptions {
        vendors,
        models,
        sources,
    })
}

pub async fn get_quota() -> impl IntoResponse {
    let fetcher = QuotaFetcher::new();

    let (kimi_result, opencode_result) =
        tokio::join!(fetcher.fetch_kimi_quota(), fetcher.fetch_opencode_quota());

    let response = QuotaResponse {
        kimi: Some(kimi_result),
        opencode_go: Some(opencode_result),
    };

    Json(response)
}

/// Export all records as downloadable JSONL.
pub async fn export_data(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let guard = state.records.read().await;
    let mut out = String::with_capacity(guard.len() * 256);
    for r in guard.iter() {
        if let Ok(line) = serde_json::to_string(r) {
            out.push_str(&line);
            out.push('\n');
        }
    }
    (
        [
            ("Content-Type", "application/x-ndjson"),
            (
                "Content-Disposition",
                "attachment; filename=token-stats-export.jsonl",
            ),
        ],
        out,
    )
}

pub async fn refresh_data(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let added = state.refresh_records().await;
    let total = state.records.read().await.len();
    Json(serde_json::json!({
        "success": true,
        "added": added,
        "total": total,
    }))
}

pub async fn get_xunfei() -> impl IntoResponse {
    let fetcher = XunfeiFetcher::new();
    let status = fetcher.fetch_status().await;
    Json(status)
}

// ─── Restore ─────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct RestoreBody {
    /// Path to a single JSONL backup file (e.g. api_requests.jsonl or usage.jsonl).
    pub backup_file: Option<String>,
    /// Path to a backup directory containing usage.jsonl and/or api_requests.jsonl.
    pub backup_dir: Option<String>,
}

#[derive(Debug, serde::Serialize)]
pub struct RestoreResponse {
    pub success: bool,
    pub before_count: usize,
    pub after_count: usize,
    pub added: usize,
    pub skipped: usize,
    pub errors: Vec<String>,
}

pub async fn restore_backup(
    State(state): State<Arc<AppState>>,
    Json(body): Json<RestoreBody>,
) -> Result<Json<RestoreResponse>, (StatusCode, String)> {
    let mut guard = state.records.write().await;
    let before_count = guard.len();

    // Build dedup fingerprint set from existing records
    let mut seen: HashSet<(String, String, String, String, i64, i64, i64)> =
        HashSet::with_capacity(guard.len());
    for r in guard.iter() {
        seen.insert((
            r.time.clone(),
            r.provider.clone(),
            r.model.clone(),
            r.source.clone(),
            r.input_tokens,
            r.output_tokens,
            r.cache_read_tokens,
        ));
    }

    let mut added = 0usize;
    let mut skipped = 0usize;
    let mut errors: Vec<String> = Vec::new();

    // Collect file paths to restore
    let mut files: Vec<PathBuf> = Vec::new();

    if let Some(ref dir) = body.backup_dir {
        let dir = PathBuf::from(dir);
        for name in &["api_requests.jsonl", "usage.jsonl"] {
            let path = dir.join(name);
            if path.exists() {
                files.push(path);
            }
        }
    }

    if let Some(ref file) = body.backup_file {
        files.push(PathBuf::from(file));
    }

    if files.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "No backup files found. Provide backup_file or backup_dir.".into(),
        ));
    }

    for file_path in &files {
        let file = match File::open(file_path) {
            Ok(f) => f,
            Err(e) => {
                errors.push(format!("Cannot open {:?}: {}", file_path, e));
                continue;
            }
        };

        let reader = BufReader::new(file);
        for line in reader.lines().map_while(Result::ok) {
            if line.trim().is_empty() {
                continue;
            }

            let record: TokenRecord = match serde_json::from_str(&line) {
                Ok(r) => r,
                Err(e) => {
                    errors.push(format!(
                        "Parse error in {:?}: {} — {}",
                        file_path,
                        e,
                        line.chars().take(80).collect::<String>()
                    ));
                    continue;
                }
            };

            let key = (
                record.time.clone(),
                record.provider.clone(),
                record.model.clone(),
                record.source.clone(),
                record.input_tokens,
                record.output_tokens,
                record.cache_read_tokens,
            );

            if seen.insert(key) {
                guard.push(record);
                added += 1;
            } else {
                skipped += 1;
            }
        }
    }

    let after_count = guard.len();

    tracing::info!(
        "Restored from backup: {} added, {} skipped, {} errors",
        added,
        skipped,
        errors.len()
    );

    Ok(Json(RestoreResponse {
        success: errors.is_empty(),
        before_count,
        after_count,
        added,
        skipped,
        errors,
    }))
}
