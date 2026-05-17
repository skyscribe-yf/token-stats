use crate::aggregator::{aggregate_records, filter_records, paginate_requests};
use crate::models::{FilterOptions, PaginatedRequests, StatsResponse, TokenRecord};
use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::Json,
};
use chrono::NaiveDate;
use serde::Deserialize;
use std::sync::Arc;

pub struct AppState {
    pub records: Vec<TokenRecord>,
}

#[derive(Debug, Deserialize)]
pub struct DateRangeQuery {
    pub from: Option<String>,
    pub to: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RequestsQuery {
    pub from: Option<String>,
    pub to: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
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

fn parse_date(s: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()
}

pub async fn get_stats(
    State(state): State<Arc<AppState>>,
    Query(params): Query<DateRangeQuery>,
) -> Result<Json<StatsResponse>, StatusCode> {
    let from = params.from.as_deref().and_then(parse_date);
    let to = params.to.as_deref().and_then(parse_date);

    let stats = aggregate_records(&state.records, from, to);
    Ok(Json(stats))
}

pub async fn get_requests(
    State(state): State<Arc<AppState>>,
    Query(params): Query<RequestsQuery>,
) -> Result<Json<PaginatedRequests>, StatusCode> {
    let from = params.from.as_deref().and_then(parse_date);
    let to = params.to.as_deref().and_then(parse_date);
    let provider = params.provider.as_deref();
    let model = params.model.as_deref();
    let page = params.page.max(1);
    let limit = params.limit.clamp(1, 500);

    let filtered = filter_records(&state.records, from, to, provider, model);
    let paginated = paginate_requests(filtered, page, limit);

    Ok(Json(paginated))
}

pub async fn get_filters(
    State(state): State<Arc<AppState>>,
) -> Result<Json<FilterOptions>, StatusCode> {
    let mut vendors: Vec<String> = state
        .records
        .iter()
        .map(|r| r.provider.clone())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();

    let mut models: Vec<String> = state
        .records
        .iter()
        .map(|r| r.model.clone())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();

    vendors.sort();
    models.sort();

    Ok(Json(FilterOptions { vendors, models }))
}
