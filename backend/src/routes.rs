use crate::aggregator;
use crate::app::AppState;
use crate::models::*;
use crate::quota::{QuotaFetcher, QuotaResponse};
use crate::time::{parse_time_bound, tz_offset_to_fixed};
use crate::xunfei::XunfeiFetcher;
use axum::{
    extract::{Query, State},
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
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

    let response = aggregator::aggregate_records(
        &records,
        from.as_ref(),
        to.as_ref(),
        source,
        provider,
        tz.as_ref(),
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

pub async fn get_xunfei() -> impl IntoResponse {
    let fetcher = XunfeiFetcher::new();
    let status = fetcher.fetch_status().await;
    Json(status)
}
