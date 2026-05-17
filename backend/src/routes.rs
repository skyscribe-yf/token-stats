use crate::aggregator::{aggregate_records, filter_records, paginate_requests};
use crate::models::{FilterOptions, PaginatedRequests, StatsResponse, TokenRecord};
use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::Json,
};
use chrono::{NaiveDate, NaiveDateTime};
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

pub enum TimeBound {
    Date(NaiveDate),
    DateTime(NaiveDateTime),
}

fn parse_time_bound(s: &str) -> Option<TimeBound> {
    // Try datetime-local format first: "2024-01-01T10:30"
    if let Ok(dt) = NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M") {
        return Some(TimeBound::DateTime(dt));
    }
    // Try full datetime with seconds: "2024-01-01T10:30:00"
    if let Ok(dt) = NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S") {
        return Some(TimeBound::DateTime(dt));
    }
    // Fallback to date-only: "2024-01-01"
    NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .ok()
        .map(TimeBound::Date)
}

fn record_matches_bound(record: &TokenRecord, from: Option<&TimeBound>, to: Option<&TimeBound>) -> bool {
    let record_dt = record.parsed_time().map(|dt| dt.naive_utc());
    let record_date = record.parsed_date();

    let from_ok = match from {
        Some(TimeBound::DateTime(f)) => {
            record_dt.map_or(false, |rd| rd >= *f)
        }
        Some(TimeBound::Date(f)) => {
            record_date.map_or(false, |rd| rd >= *f)
        }
        None => true,
    };

    let to_ok = match to {
        Some(TimeBound::DateTime(t)) => {
            record_dt.map_or(false, |rd| rd <= *t)
        }
        Some(TimeBound::Date(t)) => {
            // For date-only upper bound, include the entire day
            record_date.map_or(false, |rd| rd <= *t)
        }
        None => true,
    };

    from_ok && to_ok
}

pub async fn get_stats(
    State(state): State<Arc<AppState>>,
    Query(params): Query<DateRangeQuery>,
) -> Result<Json<StatsResponse>, StatusCode> {
    let from = params.from.as_deref().and_then(parse_time_bound);
    let to = params.to.as_deref().and_then(parse_time_bound);

    let stats = aggregate_records(&state.records, from.as_ref(), to.as_ref());
    Ok(Json(stats))
}

pub async fn get_requests(
    State(state): State<Arc<AppState>>,
    Query(params): Query<RequestsQuery>,
) -> Result<Json<PaginatedRequests>, StatusCode> {
    let from = params.from.as_deref().and_then(parse_time_bound);
    let to = params.to.as_deref().and_then(parse_time_bound);
    let provider = params.provider.as_deref();
    let model = params.model.as_deref();
    let page = params.page.max(1);
    let limit = params.limit.clamp(1, 500);

    let filtered = filter_records(&state.records, from.as_ref(), to.as_ref(), provider, model);
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
