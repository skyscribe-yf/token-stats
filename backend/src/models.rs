use chrono::{DateTime, NaiveDate, Utc};
use compact_str::CompactString;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TokenRecord {
    /// The low-cardinality fields are `CompactString`: values up to 22 bytes
    /// live inline, so `date`/`api_key_prefix`/`provider`/`source` and most
    /// model names cost no heap allocation at all. A plain `String` costs one
    /// malloc per field, which at ~760k records is ~4M allocations of ~12
    /// bytes — mostly allocator overhead, and pointer-chasing every time
    /// `fingerprint()` hashes or `sort_by` compares them.
    pub date: CompactString,
    pub time: String,
    #[serde(rename = "apiKeyPrefix", default)]
    pub api_key_prefix: CompactString,
    pub provider: CompactString,
    /// The provider name before vendor merge was applied.
    /// Used by display_cost() to determine the correct cost formula
    /// (e.g. opencode-go records merged into deepseek still need USD→CNY conversion).
    ///
    /// `CompactString` like the other low-cardinality fields: 433k of the 842k
    /// persisted rows carry one, and every value is a short provider slug
    /// (`kimi-coding`, `deepseek-official`), so inlining removes a malloc per
    /// row — ~20 MB resident across the table. `as_deref()` keeps its use as
    /// `&str` at every call site.
    #[serde(default, skip_serializing)]
    pub original_provider: Option<CompactString>,
    pub model: CompactString,
    #[serde(default)]
    pub source: CompactString,
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
    #[serde(rename = "ttftMs", default)]
    pub ttft_ms: Option<f64>,
    #[serde(rename = "tps", default)]
    pub tps: Option<f64>,
    /// Memoized parse of [`Self::time`].
    ///
    /// `time` is written once at parse time and read from every aggregation
    /// pass (`filter_records`, `compute_*`, `minute_index_utc`, pricing's
    /// `select_segment`), so re-running the RFC3339 parse on each access cost
    /// ~5 parses over the whole table per full-history request. OnceLock keeps
    /// that at ≤1 parse per record for the life of the process, with no lock
    /// on the hot path and no per-entry heap allocation.
    ///
    /// Not serialized: `time` stays the single source of truth, so a
    /// JSON round-trip (or a restore from the store) rebuilds it lazily.
    ///
    /// Public only so the many struct-literal construction sites across the
    /// source parsers can initialize it; nothing outside `models.rs` should
    /// ever read it — use [`Self::parsed_time`] for that.
    #[serde(skip)]
    pub parsed_time: OnceLock<Option<DateTime<Utc>>>,
}

impl TokenRecord {
    /// Stable identity used for cross-source dedup.
    ///
    /// Two records with the same fingerprint are considered the same
    /// request (regardless of cost/ttft/tps refinements), matching the
    /// historical in-memory dedup semantics. DB-level uniqueness is
    /// enforced separately by the `idx_token_records_fingerprint` unique
    /// index on the raw columns, so this in-memory form can be a hash.
    ///
    /// ponytail: u64 hash instead of a tuple of cloned Strings — zero
    /// allocations; collision odds at ~1e5 records are ≈1e-10. Dedup is
    /// in-memory only, rehashed per process, so hasher stability across
    /// runs is not required.
    pub fn fingerprint(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.time.hash(&mut h);
        self.provider.hash(&mut h);
        self.model.hash(&mut h);
        self.source.hash(&mut h);
        self.input_tokens.hash(&mut h);
        self.output_tokens.hash(&mut h);
        self.cache_read_tokens.hash(&mut h);
        h.finish()
    }

    pub fn cache_hit_ratio(&self) -> f64 {
        let total_input = self.input_tokens + self.cache_read_tokens;
        if total_input > 0 {
            self.cache_read_tokens as f64 / total_input as f64 * 100.0
        } else {
            0.0
        }
    }

    /// Returns true if all token fields are zero (e.g. 429 error response).
    pub fn is_zero_token(&self) -> bool {
        self.input_tokens == 0
            && self.output_tokens == 0
            && self.cache_read_tokens == 0
            && self.cache_write_tokens == 0
    }

    /// Zero-token records normally mean a failed request (429 etc.) and are
    /// filtered out. Qoder Desktop is the exception: its gateway never returns
    /// usage to the client, so every request logs zeros while still being a
    /// real call. Keep those so call counts stay visible.
    pub fn counts_as_call_without_tokens(&self) -> bool {
        self.source == "qoder-desktop"
    }

    pub fn parsed_date(&self) -> Option<NaiveDate> {
        NaiveDate::parse_from_str(&self.date, "%Y-%m-%d").ok()
    }

    pub fn parsed_time(&self) -> Option<DateTime<Utc>> {
        // `get_or_init` is a single acquire load once populated, and the
        // closure runs at most once per record even under concurrent readers.
        *self.parsed_time.get_or_init(|| {
            DateTime::parse_from_rfc3339(&self.time)
                .ok()
                .map(|dt| dt.with_timezone(&Utc))
        })
    }

    /// Returns the UTC minute index (minutes since Unix epoch) for RPM calculations.
    /// Avoids the repeated RFC3339 parsing + string formatting in hot loops.
    pub fn minute_index_utc(&self) -> Option<i64> {
        self.parsed_time().map(|dt| dt.timestamp() / 60)
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
    /// Average time-to-first-token in milliseconds.
    #[serde(default)]
    pub avg_ttft_ms: f64,
    /// Average tokens per second.
    #[serde(default)]
    pub avg_tps: f64,
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
    /// Average time-to-first-token in milliseconds for this source.
    #[serde(default)]
    pub avg_ttft_ms: f64,
    /// Average tokens per second for this source.
    #[serde(default)]
    pub avg_tps: f64,
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
    /// Average time-to-first-token in milliseconds.
    #[serde(default)]
    pub avg_ttft_ms: f64,
    /// Average tokens per second.
    #[serde(default)]
    pub avg_tps: f64,
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
    pub ttft_ms: Option<f64>,
    pub tps: Option<f64>,
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

/// A single TPS data point for time-series chart.
#[derive(Debug, Clone, Serialize)]
pub struct TpsDataPoint {
    /// Timestamp in format "YYYY-MM-DD HH:MM"
    pub time: String,
    /// Active-period TPS within a 5-minute rolling window
    pub tps: f64,
}

/// TPS time-series for a single model.
#[derive(Debug, Clone, Serialize)]
pub struct TpsModelSeries {
    pub model: String,
    pub provider: String,
    pub data_points: Vec<TpsDataPoint>,
}

/// Response for TPS analysis endpoint.
#[derive(Debug, Clone, Serialize)]
pub struct TpsAnalysis {
    pub models: Vec<TpsModelSeries>,
    /// All available model names for filtering UI
    pub available_models: Vec<String>,
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
            date: time[..10].into(),
            time: time.to_string(),
            api_key_prefix: "test".into(),
            provider: provider.into(),
            original_provider: None,
            model: model.into(),
            source: source.into(),
            input_tokens: total_tokens / 2,
            output_tokens: total_tokens / 2,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            total_tokens,
            cost: 0.0,
            ttft_ms: None,
            tps: None,
            parsed_time: OnceLock::new(),
        }
    }
}

// ── Manual `PartialEq`: identity never depends on memo state ───────────────
//
// `OnceLock`'s derived `PartialEq` treats an uninitialized and an initialized
// cell as unequal even when the inner values match, so the auto-derive would
// make two records for the same request compare differently depending on
// whether one of them had been through an aggregation pass. Nothing in the
// pipeline compares whole records today (dedup is fingerprint-based), but the
// type now carries a lazily-populated field, so make the equality contract
// explicit rather than accidental.

impl PartialEq for TokenRecord {
    fn eq(&self, other: &Self) -> bool {
        self.time == other.time
            && self.date == other.date
            && self.api_key_prefix == other.api_key_prefix
            && self.provider == other.provider
            && self.original_provider == other.original_provider
            && self.model == other.model
            && self.source == other.source
            && self.input_tokens == other.input_tokens
            && self.output_tokens == other.output_tokens
            && self.cache_read_tokens == other.cache_read_tokens
            && self.cache_write_tokens == other.cache_write_tokens
            && self.total_tokens == other.total_tokens
            && self.cost == other.cost
            && self.ttft_ms == other.ttft_ms
            && self.tps == other.tps
    }
}

#[cfg(test)]
mod parsed_time_tests {
    use super::*;

    fn rec(time: &str) -> TokenRecord {
        TokenRecord::fixture("pi", "ainaba", "gpt-5.5", time, 100)
    }

    #[test]
    fn fingerprint_is_stable_across_memoization() {
        // The memo must not influence identity: a record whose parsed_time has
        // been materialized has to hash exactly like a freshly parsed twin,
        // otherwise dedup would silently split history.
        let fresh = rec("2026-10-06T23:03:47.173Z");
        let warm = rec("2026-10-06T23:03:47.173Z");
        assert_eq!(warm.parsed_time(), fresh.parsed_time());
        assert!(warm.parsed_time().is_some(), "the string must parse");

        assert_eq!(
            fresh.fingerprint(),
            warm.fingerprint(),
            "memoizing the timestamp changed the record's identity"
        );
    }

    #[test]
    fn memo_is_idempotent_and_parses_every_stored_format() {
        // Formats actually present in the store: Z-suffixed, +00:00-offset,
        // second precision and sub-second precision.
        for t in [
            "2026-10-06T23:03:15Z",
            "2026-10-06T23:03:47.173Z",
            "2026-10-06T23:03:47.173+00:00",
            "2026-10-06T23:04:02.143Z",
        ] {
            let r = rec(t);
            let first = r.parsed_time().expect(t);
            let second = r.parsed_time().expect(t);
            assert_eq!(first, second, "not idempotent for {t}");
            assert_eq!(
                first,
                DateTime::parse_from_rfc3339(t).unwrap().with_timezone(&Utc),
                "wrong instant for {t}"
            );
        }
    }

    #[test]
    fn malformed_time_memoizes_failure_rather_than_reparsing() {
        let r = rec("not-a-timestamp");
        assert!(r.parsed_time().is_none());
        assert!(
            r.parsed_time().is_none(),
            "failure must be cached, not retried"
        );
    }

    #[test]
    fn serde_round_trip_rebuilds_the_memo() {
        let r = rec("2026-10-06T23:03:47.173Z");
        let _ = r.parsed_time();
        let json = serde_json::to_string(&r).expect("serialize");
        assert!(
            !json.contains("parsed_time"),
            "memo leaked into the wire format"
        );
        let back: TokenRecord = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.parsed_time(), r.parsed_time());
        assert_eq!(back.fingerprint(), r.fingerprint());
    }

    #[test]
    fn equality_ignores_memo_state() {
        // Records built by different paths (parser vs. store restore) must
        // compare equal regardless of whether either side has parsed yet.
        let unparsed = rec("2026-10-06T23:03:47.173Z");
        let parsed = rec("2026-10-06T23:03:47.173Z");
        assert!(parsed.parsed_time().is_some());
        assert_eq!(unparsed, parsed);
    }
}
