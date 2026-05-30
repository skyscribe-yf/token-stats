use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TokenRecord {
    pub date: String,
    pub time: String,
    #[serde(rename = "apiKeyPrefix", default)]
    pub api_key_prefix: String,
    pub provider: String,
    /// The provider name before vendor merge was applied.
    /// Used by display_cost() to determine the correct cost formula
    /// (e.g. opencode-go records merged into deepseek still need USD→CNY conversion).
    #[serde(default, skip_serializing)]
    pub original_provider: Option<String>,
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

/// Aggregation time resolution for date-based stats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Resolution {
    #[default]
    Day,
    HalfDay,
    FourHours,
    TwoHours,
    OneHour,
}

impl Resolution {
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "day" => Some(Self::Day),
            "12h" => Some(Self::HalfDay),
            "4h" => Some(Self::FourHours),
            "2h" => Some(Self::TwoHours),
            "1h" => Some(Self::OneHour),
            _ => None,
        }
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
    /// Cache hit ratio excluding xunfei provider records (xunfei has no cache mechanism)
    pub cache_hit_ratio_no_xunfei: f64,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct SourceDetailStats {
    pub source: String,
    pub calls: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub total_tokens: i64,
    pub cost: f64,
    pub cache_hit_ratio: f64,
    /// Average RPM for this source within the model.
    #[serde(default)]
    pub avg_rpm: f64,
    /// Peak RPM for this source within the model.
    #[serde(default)]
    pub peak_rpm: i64,
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
    pub source_details: Vec<SourceDetailStats>,
    /// Average requests per minute during active windows for this provider+model.
    /// Computed using consecutive-request window boundary detection.
    #[serde(default)]
    pub avg_rpm: f64,
    /// Peak requests per minute in any single minute bucket for this provider+model.
    #[serde(default)]
    pub peak_rpm: i64,
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

/// Minute-level request count for RPM analysis.
#[derive(Debug, Clone, Serialize)]
pub struct MinuteBucket {
    /// Minute timestamp in format "YYYY-MM-DD HH:MM"
    pub minute: String,
    /// Number of requests in this minute
    pub requests: i64,
}

/// An active request window - consecutive minutes with requests.
#[derive(Debug, Clone, Serialize)]
pub struct ActiveWindow {
    /// Start minute of the window
    pub start: String,
    /// End minute of the window
    pub end: String,
    /// Duration in minutes
    pub duration_minutes: i64,
    /// Total requests in this window
    pub total_requests: i64,
    /// Average requests per minute during the window
    pub avg_rpm: f64,
    /// Peak requests per minute in this window
    pub peak_rpm: i64,
    /// Minute-by-minute breakdown
    pub buckets: Vec<MinuteBucket>,
}

/// Response for RPM analysis endpoint.
#[derive(Debug, Clone, Serialize)]
pub struct RpmAnalysis {
    /// All minute buckets (including zero-request minutes within windows)
    pub all_buckets: Vec<MinuteBucket>,
    /// Detected active windows
    pub windows: Vec<ActiveWindow>,
    /// Overall average RPM (total requests / total active minutes)
    pub overall_avg_rpm: f64,
    /// Overall peak RPM (max requests in any single minute)
    pub overall_peak_rpm: i64,
    /// Total active minutes (minutes with at least 1 request)
    pub total_active_minutes: i64,
    /// Gap threshold used for boundary detection (minutes)
    pub gap_threshold_minutes: i64,
}

// ─── Test helpers ────────────────────────────────────────────────────────────

#[cfg(test)]
impl TokenRecord {
    /// Create a test fixture record.
    #[allow(dead_code)]
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
            original_provider: None,
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
