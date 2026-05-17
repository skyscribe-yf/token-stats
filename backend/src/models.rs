use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TokenRecord {
    pub date: String,
    pub time: String,
    #[serde(rename = "apiKeyPrefix")]
    pub api_key_prefix: String,
    pub provider: String,
    pub model: String,
    #[serde(default)]
    pub source: String,
    #[serde(rename = "inputTokens")]
    pub input_tokens: i64,
    #[serde(rename = "outputTokens")]
    pub output_tokens: i64,
    #[serde(rename = "cacheReadTokens")]
    pub cache_read_tokens: i64,
    #[serde(rename = "cacheWriteTokens")]
    pub cache_write_tokens: i64,
    #[serde(rename = "totalTokens")]
    pub total_tokens: i64,
    pub cost: f64,
}

impl TokenRecord {
    pub fn cache_hit_ratio(&self) -> f64 {
        let total_input = self.input_tokens + self.cache_read_tokens;
        if total_input > 0 {
            self.cache_read_tokens as f64 / total_input as f64 * 100.0
        } else {
            0.0
        }
    }

    pub fn parsed_date(&self) -> Option<NaiveDate> {
        NaiveDate::parse_from_str(&self.date, "%Y-%m-%d").ok()
    }

    pub fn parsed_time(&self) -> Option<DateTime<Utc>> {
        DateTime::parse_from_rfc3339(&self.time)
            .ok()
            .map(|dt| dt.with_timezone(&Utc))
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct AggregatedStats {
    pub total_calls: i64,
    pub total_input_tokens: i64,
    pub total_output_tokens: i64,
    pub total_cache_read_tokens: i64,
    pub total_cache_write_tokens: i64,
    pub total_tokens: i64,
    pub total_cost: f64,
    pub avg_cache_hit_ratio: f64,
    pub weighted_cache_hit_ratio: f64,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct VendorStats {
    pub provider: String,
    pub calls: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub total_tokens: i64,
    pub cost: f64,
    pub cache_hit_ratio: f64,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct DateStats {
    pub date: String,
    pub calls: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub total_tokens: i64,
    pub cost: f64,
    pub cache_hit_ratio: f64,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct ModelStats {
    pub model: String,
    pub provider: String,
    pub sources: Vec<String>,
    pub calls: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub total_tokens: i64,
    pub cost: f64,
    pub cache_hit_ratio: f64,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct SourceStats {
    pub source: String,
    pub calls: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub total_tokens: i64,
    pub cost: f64,
    pub cache_hit_ratio: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct DetailedRequest {
    pub date: String,
    pub time: String,
    pub provider: String,
    pub model: String,
    pub source: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub total_tokens: i64,
    pub cost: f64,
    pub cache_hit_ratio: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct PaginatedRequests {
    pub data: Vec<DetailedRequest>,
    pub total: usize,
    pub page: usize,
    pub limit: usize,
    pub total_pages: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct StatsResponse {
    pub overall: AggregatedStats,
    pub by_vendor: Vec<VendorStats>,
    pub by_date: Vec<DateStats>,
    pub by_model: Vec<ModelStats>,
    pub by_source: Vec<SourceStats>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FilterOptions {
    pub vendors: Vec<String>,
    pub models: Vec<String>,
    pub sources: Vec<String>,
}

// ─── DimensionStats trait for generic aggregation ────────────────────────────

/// Trait for dimension-level stats structs that can be accumulated from
/// `TokenRecord` and finalized (cache hit ratio computation).
pub trait DimensionStats: Default + Clone {
    /// Accumulate token usage from a single record.
    fn accumulate(&mut self, r: &TokenRecord) {
        *self.calls_mut() += 1;
        *self.input_tokens_mut() += r.input_tokens;
        *self.output_tokens_mut() += r.output_tokens;
        *self.cache_read_tokens_mut() += r.cache_read_tokens;
        *self.cache_write_tokens_mut() += r.cache_write_tokens;
        *self.total_tokens_mut() += r.total_tokens;
        *self.cost_mut() += r.cost;
    }

    /// Compute derived fields (cache hit ratio) after all records are accumulated.
    fn finalize(&mut self) {
        let denom = self.input_tokens() + self.cache_read_tokens();
        if denom > 0 {
            *self.cache_hit_ratio_mut() =
                self.cache_read_tokens() as f64 / denom as f64 * 100.0;
        }
    }

    fn calls(&self) -> i64;
    fn calls_mut(&mut self) -> &mut i64;
    fn input_tokens(&self) -> i64;
    fn input_tokens_mut(&mut self) -> &mut i64;
    fn output_tokens(&self) -> i64;
    fn output_tokens_mut(&mut self) -> &mut i64;
    fn cache_read_tokens(&self) -> i64;
    fn cache_read_tokens_mut(&mut self) -> &mut i64;
    fn cache_write_tokens(&self) -> i64;
    fn cache_write_tokens_mut(&mut self) -> &mut i64;
    fn total_tokens(&self) -> i64;
    fn total_tokens_mut(&mut self) -> &mut i64;
    fn cost(&self) -> f64;
    fn cost_mut(&mut self) -> &mut f64;
    fn cache_hit_ratio_mut(&mut self) -> &mut f64;
}

macro_rules! impl_dimension_stats_base {
    ($t:ty) => {
        impl DimensionStats for $t {
            fn calls(&self) -> i64 { self.calls }
            fn calls_mut(&mut self) -> &mut i64 { &mut self.calls }
            fn input_tokens(&self) -> i64 { self.input_tokens }
            fn input_tokens_mut(&mut self) -> &mut i64 { &mut self.input_tokens }
            fn output_tokens(&self) -> i64 { self.output_tokens }
            fn output_tokens_mut(&mut self) -> &mut i64 { &mut self.output_tokens }
            fn cache_read_tokens(&self) -> i64 { self.cache_read_tokens }
            fn cache_read_tokens_mut(&mut self) -> &mut i64 { &mut self.cache_read_tokens }
            fn cache_write_tokens(&self) -> i64 { self.cache_write_tokens }
            fn cache_write_tokens_mut(&mut self) -> &mut i64 { &mut self.cache_write_tokens }
            fn total_tokens(&self) -> i64 { self.total_tokens }
            fn total_tokens_mut(&mut self) -> &mut i64 { &mut self.total_tokens }
            fn cost(&self) -> f64 { self.cost }
            fn cost_mut(&mut self) -> &mut f64 { &mut self.cost }
            fn cache_hit_ratio_mut(&mut self) -> &mut f64 { &mut self.cache_hit_ratio }
        }
    };
}

impl_dimension_stats_base!(VendorStats);
impl_dimension_stats_base!(DateStats);
impl_dimension_stats_base!(SourceStats);

impl ModelStats {
    pub fn add_source(&mut self, source: &str) {
        if !self.sources.contains(&source.to_string()) {
            self.sources.push(source.to_string());
        }
    }
}

impl DimensionStats for ModelStats {
    fn accumulate(&mut self, r: &TokenRecord) {
        *self.calls_mut() += 1;
        *self.input_tokens_mut() += r.input_tokens;
        *self.output_tokens_mut() += r.output_tokens;
        *self.cache_read_tokens_mut() += r.cache_read_tokens;
        *self.cache_write_tokens_mut() += r.cache_write_tokens;
        *self.total_tokens_mut() += r.total_tokens;
        *self.cost_mut() += r.cost;
        self.add_source(&r.source);
    }
    fn calls(&self) -> i64 { self.calls }
    fn calls_mut(&mut self) -> &mut i64 { &mut self.calls }
    fn input_tokens(&self) -> i64 { self.input_tokens }
    fn input_tokens_mut(&mut self) -> &mut i64 { &mut self.input_tokens }
    fn output_tokens(&self) -> i64 { self.output_tokens }
    fn output_tokens_mut(&mut self) -> &mut i64 { &mut self.output_tokens }
    fn cache_read_tokens(&self) -> i64 { self.cache_read_tokens }
    fn cache_read_tokens_mut(&mut self) -> &mut i64 { &mut self.cache_read_tokens }
    fn cache_write_tokens(&self) -> i64 { self.cache_write_tokens }
    fn cache_write_tokens_mut(&mut self) -> &mut i64 { &mut self.cache_write_tokens }
    fn total_tokens(&self) -> i64 { self.total_tokens }
    fn total_tokens_mut(&mut self) -> &mut i64 { &mut self.total_tokens }
    fn cost(&self) -> f64 { self.cost }
    fn cost_mut(&mut self) -> &mut f64 { &mut self.cost }
    fn cache_hit_ratio_mut(&mut self) -> &mut f64 { &mut self.cache_hit_ratio }
}

// ─── Test helpers ────────────────────────────────────────────────────────────

#[cfg(test)]
impl TokenRecord {
    /// Create a test fixture record.
    pub fn fixture(
        source: &str,
        provider: &str,
        model: &str,
        time: &str,
        total_tokens: i64,
    ) -> Self {
        Self {
            date: time[..10].to_string(),
            time: time.to_string(),
            api_key_prefix: "test".to_string(),
            provider: provider.to_string(),
            model: model.to_string(),
            source: source.to_string(),
            input_tokens: total_tokens / 2,
            output_tokens: total_tokens / 2,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            total_tokens,
            cost: 0.0,
        }
    }
}
