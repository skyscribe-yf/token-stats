use crate::aggregator;
use crate::models::*;
use axum::{
    extract::{Query, State},
    response::IntoResponse,
    Json,
};
use chrono::NaiveDate;
use serde::Deserialize;
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct AppState {
    pub records: RwLock<Vec<TokenRecord>>,
}

#[derive(Debug, Deserialize)]
pub struct StatsQuery {
    pub from: Option<String>,
    pub to: Option<String>,
    pub source: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RequestsQuery {
    pub from: Option<String>,
    pub to: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub source: Option<String>,
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

#[derive(Debug, Clone)]
pub enum TimeBound {
    DateTime(chrono::NaiveDateTime),
    Date(NaiveDate),
}

pub fn parse_time_bound(s: &str) -> Option<TimeBound> {
    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S") {
        return Some(TimeBound::DateTime(dt));
    }
    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f") {
        return Some(TimeBound::DateTime(dt));
    }
    if let Ok(d) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return Some(TimeBound::Date(d));
    }
    None
}

pub async fn get_stats(
    State(state): State<Arc<AppState>>,
    Query(query): Query<StatsQuery>,
) -> impl IntoResponse {
    let records = state.records.read().await;
    let from = query.from.as_ref().and_then(|s| parse_time_bound(s));
    let to = query.to.as_ref().and_then(|s| parse_time_bound(s));
    let source = query.source.as_deref().filter(|s| !s.is_empty());

    let response = aggregator::aggregate_records(&records, from.as_ref(), to.as_ref(), source);
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

    let filtered = aggregator::filter_records(&records, from.as_ref(), to.as_ref(), provider, model, source);
    let paginated = aggregator::paginate_requests(filtered, query.page, query.limit);

    Json(paginated)
}

pub async fn get_filters(
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
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