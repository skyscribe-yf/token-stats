use crate::aggregator;
use crate::ainaiba::fetch_ainaiba_credit;
use crate::app::AppState;
use crate::models::*;
use crate::pricing;
use crate::quota::{QuotaFetcher, QuotaResponse};
use crate::settings;
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
use std::path::{Path, PathBuf};
use std::sync::Arc;

// ─── Query parameter types ───────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct StatsQuery {
    pub from: Option<String>,
    pub to: Option<String>,
    pub source: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
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
    /// When true, include records with zero tokens (e.g. 429 errors) in results.
    #[serde(default)]
    pub show_zero_tokens: Option<bool>,
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
    let model = query.model.as_deref().filter(|s| !s.is_empty());
    let tz = query.tz_offset.map(tz_offset_to_fixed);
    let resolution = query
        .resolution
        .as_deref()
        .and_then(Resolution::from_str)
        .unwrap_or_default();

    let filters = aggregator::FilterCriteria {
        from: from.as_ref(),
        to: to.as_ref(),
        source,
        provider,
        model,
        tz: tz.as_ref(),
        exclude_zero_tokens: true,
    };
    let response = aggregator::aggregate_records(&records, &filters, resolution);
    Json(response)
}

#[derive(Debug, Deserialize)]
pub struct RpmQuery {
    pub from: Option<String>,
    pub to: Option<String>,
    pub source: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub tz_offset: Option<i32>,
    /// Gap threshold in minutes for active-window boundary detection (default: 5)
    #[serde(default = "default_gap_threshold")]
    pub gap_threshold: i64,
}

fn default_gap_threshold() -> i64 {
    5
}

pub async fn get_rpm(
    State(state): State<Arc<AppState>>,
    Query(query): Query<RpmQuery>,
) -> impl IntoResponse {
    let records = state.records.read().await;
    let from = query.from.as_ref().and_then(|s| parse_time_bound(s));
    let to = query.to.as_ref().and_then(|s| parse_time_bound(s));
    let source = query.source.as_deref().filter(|s| !s.is_empty());
    let provider = query.provider.as_deref().filter(|s| !s.is_empty());
    let model = query.model.as_deref().filter(|s| !s.is_empty());
    let tz = query.tz_offset.map(tz_offset_to_fixed);
    let gap_threshold = query.gap_threshold.max(1);

    let filters = aggregator::FilterCriteria {
        from: from.as_ref(),
        to: to.as_ref(),
        source,
        provider,
        model,
        tz: tz.as_ref(),
        exclude_zero_tokens: true,
    };
    let response = aggregator::compute_rpm_analysis(&records, &filters, gap_threshold);
    Json(response)
}

#[derive(Debug, Deserialize)]
pub struct TpsQuery {
    pub from: Option<String>,
    pub to: Option<String>,
    pub source: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub tz_offset: Option<i32>,
    /// Comma-separated list of models to include (e.g. "astron-code-latest,deepseek-v4-flash")
    pub models: Option<String>,
}

pub async fn get_tps(
    State(state): State<Arc<AppState>>,
    Query(query): Query<TpsQuery>,
) -> impl IntoResponse {
    let records = state.records.read().await;
    let from = query.from.as_ref().and_then(|s| parse_time_bound(s));
    let to = query.to.as_ref().and_then(|s| parse_time_bound(s));
    let source = query.source.as_deref().filter(|s| !s.is_empty());
    let provider = query.provider.as_deref().filter(|s| !s.is_empty());
    let model = query.model.as_deref().filter(|s| !s.is_empty());
    let tz = query.tz_offset.map(tz_offset_to_fixed);

    let filters = aggregator::FilterCriteria {
        from: from.as_ref(),
        to: to.as_ref(),
        source,
        provider,
        model,
        tz: tz.as_ref(),
        exclude_zero_tokens: true,
    };
    let response = aggregator::compute_tps_analysis(&records, &filters, query.models.as_deref());
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
    let exclude_zero_tokens = !query.show_zero_tokens.unwrap_or(false);

    let filters = aggregator::FilterCriteria {
        from: from.as_ref(),
        to: to.as_ref(),
        source,
        provider,
        model,
        tz: tz.as_ref(),
        exclude_zero_tokens,
    };
    let filtered = aggregator::filter_records(&records, &filters);
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

    let (
        kimi_result,
        kimi_ex_result,
        opencode_result,
        opencode_ex_result,
        xiaomi_mimo_result,
        commandcode_result,
    ) = tokio::join!(
        fetcher.fetch_kimi_quota(),
        fetcher.fetch_kimi_quota_ex(),
        fetcher.fetch_opencode_quota(),
        fetcher.fetch_opencode_quota_ex(),
        fetcher.fetch_xiaomi_mimo_quota(),
        fetcher.fetch_commandcode_quota(),
    );

    let response = QuotaResponse {
        kimi: Some(kimi_result),
        kimi_ex: Some(kimi_ex_result),
        opencode_go: Some(opencode_result),
        opencode_go_ex: Some(opencode_ex_result),
        xiaomi_mimo: Some(xiaomi_mimo_result),
        commandcode: Some(commandcode_result),
    };

    Json(response)
}

/// Export all records as downloadable JSONL.
pub async fn export_data(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let guard = state.records.read().await;
    let mut out = String::with_capacity(guard.len() * 256);
    for r in guard.iter() {
        match serde_json::to_string(r) {
            Ok(line) => {
                out.push_str(&line);
                out.push('\n');
            }
            Err(e) => {
                tracing::warn!("Failed to serialize record during export: {}", e);
            }
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
    let status = fetcher.fetch_all_statuses().await;
    Json(status)
}

pub async fn get_ainaiba_credit() -> impl IntoResponse {
    Json(fetch_ainaiba_credit().await)
}

pub async fn get_advanced_models() -> impl IntoResponse {
    Json(settings::load_advanced_models())
}

pub async fn update_advanced_models(Json(body): Json<Vec<String>>) -> impl IntoResponse {
    match settings::save_advanced_models(&body) {
        Ok(()) => Json(serde_json::json!({ "success": true })),
        Err(e) => {
            tracing::warn!("Failed to save advanced models: {}", e);
            Json(serde_json::json!({ "success": false, "error": "Failed to save settings" }))
        }
    }
}

pub async fn get_subscription_settings() -> impl IntoResponse {
    Json(settings::load_subscription_settings())
}

pub async fn update_subscription_settings(
    Json(body): Json<settings::SubscriptionSettings>,
) -> impl IntoResponse {
    // Validate kimi_monthly_start_day: must be None or 1..=28
    if let Some(day) = body.kimi_monthly_start_day {
        if !(1..=28).contains(&day) {
            return Json(serde_json::json!({
                "success": false,
                "error": "kimi_monthly_start_day must be between 1 and 28"
            }));
        }
    }
    match settings::save_subscription_settings(&body) {
        Ok(()) => Json(serde_json::json!({ "success": true })),
        Err(e) => {
            tracing::warn!("Failed to save subscription settings: {}", e);
            Json(serde_json::json!({ "success": false, "error": "Failed to save settings" }))
        }
    }
}

pub async fn get_pricing() -> impl IntoResponse {
    Json(pricing::get_config())
}

pub async fn reload_pricing() -> impl IntoResponse {
    pricing::reload();
    Json(serde_json::json!({ "success": true }))
}

// ─── Lenient parse helpers ──────────────────────────────────────────────────

/// Attempt to parse a JSONL line as a TokenRecord, trying camelCase first
/// then falling back to snake_case field names (for api_requests.jsonl exports).
fn parse_record_lenient(line: &str) -> Option<TokenRecord> {
    // First try canonical camelCase (TokenRecord's serde rename)
    if let Ok(r) = serde_json::from_str::<TokenRecord>(line) {
        return Some(r);
    }

    // Fallback: manually translate snake_case keys to camelCase
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    let obj = v.as_object()?;

    // Build a translated JSON object with camelCase keys
    let mut camel = serde_json::Map::new();
    for (k, val) in obj {
        match k.as_str() {
            "input_tokens" => {
                camel.insert("inputTokens".into(), val.clone());
            }
            "output_tokens" => {
                camel.insert("outputTokens".into(), val.clone());
            }
            "cache_read_tokens" => {
                camel.insert("cacheReadTokens".into(), val.clone());
            }
            "cache_write_tokens" => {
                camel.insert("cacheWriteTokens".into(), val.clone());
            }
            "total_tokens" => {
                camel.insert("totalTokens".into(), val.clone());
            }
            "api_key_prefix" => {
                camel.insert("apiKeyPrefix".into(), val.clone());
            }
            "ttft_ms" => {
                camel.insert("ttftMs".into(), val.clone());
            }
            "cache_hit_ratio" => { /* skip, not a TokenRecord field */ }
            _ => {
                camel.insert(k.clone(), val.clone());
            }
        }
    }
    serde_json::from_value(serde_json::Value::Object(camel)).ok()
}

/// Infer source from a backup filename when the record itself is missing `source`.
fn infer_source_from_filename(path: &Path) -> String {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    match name {
        "usage.jsonl" => "pi".to_string(),
        n if n.starts_with("token-stats-export-") && n.ends_with(".jsonl") => "pi".to_string(),
        _ => "pi".to_string(), // default backup source
    }
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
        // Look for standard backup filenames and exported files
        for name in &["api_requests.jsonl", "usage.jsonl"] {
            let path = dir.join(name);
            if path.exists() {
                files.push(path);
            }
        }
        // Also accept token-stats-export-*.jsonl files produced by the export button
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.filter_map(Result::ok) {
                let path = entry.path();
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if name.starts_with("token-stats-export-") && name.ends_with(".jsonl") {
                        files.push(path);
                    }
                }
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

            let record: TokenRecord = match parse_record_lenient(&line) {
                None => {
                    errors.push(format!(
                        "Parse error in {:?}: — {}",
                        file_path,
                        line.chars().take(80).collect::<String>()
                    ));
                    continue;
                }
                Some(r) => r,
            };

            // Infer source from filename when missing
            let record = if record.source.is_empty() {
                let inferred = infer_source_from_filename(file_path);
                TokenRecord {
                    source: inferred,
                    ..record
                }
            } else {
                record
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
