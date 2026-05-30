use crate::models::*;
use crate::pricing;
use crate::time::TimeBound;
use chrono::{FixedOffset, Timelike, Utc};
use std::collections::{HashMap, HashSet};

// ── Shared accumulation helper ───────────────────────────────────────────────

/// Token-usage accumulator shared by all dimension-level `compute_*` functions.
#[derive(Default)]
struct StatAccum {
    calls: i64,
    input_tokens: i64,
    output_tokens: i64,
    cache_read_tokens: i64,
    cache_write_tokens: i64,
    total_tokens: i64,
    cost: f64,
}

impl StatAccum {
    fn accumulate(&mut self, r: &TokenRecord) {
        self.calls += 1;
        self.input_tokens += r.input_tokens;
        self.output_tokens += r.output_tokens;
        self.cache_read_tokens += r.cache_read_tokens;
        self.cache_write_tokens += r.cache_write_tokens;
        self.total_tokens += r.total_tokens;
        self.cost += pricing::display_cost(r);
    }

    fn cache_hit_ratio(&self) -> f64 {
        let denom = self.input_tokens + self.cache_read_tokens;
        if denom > 0 {
            self.cache_read_tokens as f64 / denom as f64 * 100.0
        } else {
            0.0
        }
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Get the local date string for a record, given an optional timezone offset.
fn local_date_for_record(record: &TokenRecord, tz: Option<&FixedOffset>) -> String {
    if let Some(tz) = tz {
        if let Some(utc_dt) = record.parsed_time() {
            let local_dt = utc_dt.with_timezone(tz);
            return local_dt.format("%Y-%m-%d").to_string();
        }
    }
    record.date.clone()
}

fn parse_csv_filter(s: Option<&str>) -> Vec<&str> {
    s.map(|v| v.split(',').filter(|x| !x.is_empty()).collect())
        .unwrap_or_default()
}

/// Shared filtering criteria for aggregation and record listing.
pub struct FilterCriteria<'a> {
    pub from: Option<&'a TimeBound>,
    pub to: Option<&'a TimeBound>,
    pub source: Option<&'a str>,
    pub provider: Option<&'a str>,
    pub model: Option<&'a str>,
    pub tz: Option<&'a FixedOffset>,
}

pub fn aggregate_records(
    records: &[TokenRecord],
    filters: &FilterCriteria,
    resolution: Resolution,
) -> StatsResponse {
    let sources = parse_csv_filter(filters.source);
    let providers = parse_csv_filter(filters.provider);
    let models = parse_csv_filter(filters.model);
    let filtered: Vec<&TokenRecord> = records
        .iter()
        .filter(|r| record_matches_bound(r, filters.from, filters.to, filters.tz))
        .filter(|r| sources.is_empty() || sources.contains(&r.source.as_str()))
        .filter(|r| providers.is_empty() || providers.contains(&r.provider.as_str()))
        .filter(|r| models.is_empty() || models.contains(&r.model.as_str()))
        .collect();

    let overall = compute_overall_stats(&filtered);
    let by_vendor = compute_vendor_stats(&filtered);
    let by_date = compute_date_stats(&filtered, filters.tz, resolution);
    let by_model = compute_model_stats(&filtered);
    let by_source = compute_source_stats(&filtered);

    StatsResponse {
        overall,
        by_vendor,
        by_date,
        by_model,
        by_source,
    }
}

pub fn filter_records<'a>(
    records: &'a [TokenRecord],
    filters: &FilterCriteria,
) -> Vec<&'a TokenRecord> {
    let sources = parse_csv_filter(filters.source);
    let providers = parse_csv_filter(filters.provider);
    let models = parse_csv_filter(filters.model);
    let mut filtered: Vec<&'a TokenRecord> = records
        .iter()
        .filter(|r| record_matches_bound(r, filters.from, filters.to, filters.tz))
        .filter(|r| {
            let provider_ok = providers.is_empty() || providers.contains(&r.provider.as_str());
            let model_ok = models.is_empty() || models.contains(&r.model.as_str());
            let source_ok = sources.is_empty() || sources.contains(&r.source.as_str());
            provider_ok && model_ok && source_ok
        })
        .collect();

    // Sort by time descending, then source asc, provider asc, model asc
    filtered.sort_by(|a, b| {
        let time_order = match (a.parsed_time(), b.parsed_time()) {
            (Some(at), Some(bt)) => bt.cmp(&at),
            _ => b.time.cmp(&a.time),
        };
        time_order
            .then_with(|| a.source.cmp(&b.source))
            .then_with(|| a.provider.cmp(&b.provider))
            .then_with(|| a.model.cmp(&b.model))
    });

    filtered
}

fn record_matches_bound(
    record: &TokenRecord,
    from: Option<&TimeBound>,
    to: Option<&TimeBound>,
    tz: Option<&FixedOffset>,
) -> bool {
    let record_dt = record.parsed_time().map(|dt| dt.naive_utc());
    // Use local date if tz is provided, otherwise use UTC date
    let record_date = if let Some(tz) = tz {
        record
            .parsed_time()
            .map(|dt| dt.with_timezone(tz).date_naive())
    } else {
        record.parsed_date()
    };

    // Frontend datetime-local inputs are in the user's local timezone.
    // Convert them to UTC using the tz_offset so we compare apples to apples.
    let local_naive_to_utc = |naive: &chrono::NaiveDateTime| -> chrono::NaiveDateTime {
        tz.and_then(|tz| {
            naive
                .and_local_timezone(*tz)
                .single()
                .map(|dt| dt.with_timezone(&Utc).naive_utc())
        })
        .unwrap_or(*naive)
    };

    let from_ok = match from {
        Some(TimeBound::DateTime(f)) => {
            let from_utc = local_naive_to_utc(f);
            record_dt.is_some_and(|rd| rd >= from_utc)
        }
        Some(TimeBound::Date(f)) => record_date.is_some_and(|rd| rd >= *f),
        None => true,
    };

    let to_ok = match to {
        Some(TimeBound::DateTime(t)) => {
            let to_utc = local_naive_to_utc(t);
            record_dt.is_some_and(|rd| rd <= to_utc)
        }
        Some(TimeBound::Date(t)) => {
            // For date-only upper bound, include the entire day
            record_date.is_some_and(|rd| rd <= *t)
        }
        None => true,
    };

    from_ok && to_ok
}

pub fn paginate_requests(
    records: Vec<&TokenRecord>,
    page: usize,
    limit: usize,
    tz: Option<&FixedOffset>,
) -> PaginatedRequests {
    let total = records.len();
    // Guard: avoid divide-by-zero and negative start index
    let page = page.max(1);
    let limit = limit.max(1);
    let total_pages = total.div_ceil(limit);
    let start = ((page - 1) * limit).min(total);
    let end = (start + limit).min(total);

    let data: Vec<DetailedRequest> = records[start..end]
        .iter()
        .map(|r| {
            let local_date = local_date_for_record(r, tz);
            DetailedRequest {
                date: local_date,
                time: r.time.clone(),
                provider: r.provider.clone(),
                model: r.model.clone(),
                source: r.source.clone(),
                input_tokens: r.input_tokens,
                output_tokens: r.output_tokens,
                cache_read_tokens: r.cache_read_tokens,
                cache_write_tokens: r.cache_write_tokens,
                total_tokens: r.total_tokens,
                cost: pricing::display_cost(r),
                cache_hit_ratio: r.cache_hit_ratio(),
            }
        })
        .collect();

    PaginatedRequests {
        data,
        total,
        page,
        limit,
        total_pages,
    }
}

// ── Aggregation functions ───────────────────────────────────────────────────

fn compute_overall_stats(records: &[&TokenRecord]) -> AggregatedStats {
    let mut acc = StatAccum::default();
    let mut total_cache_hit_ratio = 0.0;

    for r in records {
        acc.accumulate(r);
        total_cache_hit_ratio += r.cache_hit_ratio();
    }

    let avg_cache_hit_ratio = if !records.is_empty() {
        total_cache_hit_ratio / records.len() as f64
    } else {
        0.0
    };

    AggregatedStats {
        total_calls: acc.calls,
        total_input_tokens: acc.input_tokens,
        total_output_tokens: acc.output_tokens,
        total_cache_read_tokens: acc.cache_read_tokens,
        total_cache_write_tokens: acc.cache_write_tokens,
        total_tokens: acc.total_tokens,
        total_cost: acc.cost,
        avg_cache_hit_ratio,
        weighted_cache_hit_ratio: acc.cache_hit_ratio(),
    }
}

fn compute_vendor_stats(records: &[&TokenRecord]) -> Vec<VendorStats> {
    let mut map: HashMap<String, StatAccum> = HashMap::new();

    for r in records {
        map.entry(r.provider.clone()).or_default().accumulate(r);
    }

    let mut result: Vec<VendorStats> = map
        .into_iter()
        .map(|(provider, acc)| VendorStats {
            provider,
            calls: acc.calls,
            input_tokens: acc.input_tokens,
            output_tokens: acc.output_tokens,
            cache_read_tokens: acc.cache_read_tokens,
            cache_write_tokens: acc.cache_write_tokens,
            total_tokens: acc.total_tokens,
            cost: acc.cost,
            cache_hit_ratio: acc.cache_hit_ratio(),
        })
        .collect();

    result.sort_by_key(|v| std::cmp::Reverse(v.total_tokens));
    result
}

/// Check if a provider is xunfei (has no cache mechanism).
/// Xunfei uses astron-code models which have zero cache reads, skewing the ratio.
fn is_xunfei_provider(provider: &str) -> bool {
    provider == "xunfei"
}

/// Compute the bucket key for a record based on the resolution.
fn period_key_for_record(
    record: &TokenRecord,
    tz: Option<&FixedOffset>,
    resolution: Resolution,
) -> String {
    if resolution == Resolution::Day {
        return local_date_for_record(record, tz);
    }

    // For sub-day resolutions, we need the local datetime
    let local_dt = if let Some(tz) = tz {
        record
            .parsed_time()
            .map(|dt| dt.with_timezone(tz).naive_local())
    } else {
        record.parsed_time().map(|dt| dt.naive_utc())
    };

    let dt = match local_dt {
        Some(dt) => dt,
        None => {
            // Fallback: use date string + midnight
            let date = local_date_for_record(record, tz);
            return format!("{} 00:00", date);
        }
    };

    let hour = dt.hour();
    match resolution {
        Resolution::OneHour => format!("{} {:02}:00", dt.format("%Y-%m-%d"), hour),
        Resolution::TwoHours => {
            let bucket_start = (hour / 2) * 2;
            format!("{} {:02}:00", dt.format("%Y-%m-%d"), bucket_start)
        }
        Resolution::FourHours => {
            let bucket_start = (hour / 4) * 4;
            format!("{} {:02}:00", dt.format("%Y-%m-%d"), bucket_start)
        }
        Resolution::HalfDay => {
            // AM (00:00) or PM (12:00)
            let bucket_start = if hour < 12 { 0 } else { 12 };
            format!("{} {:02}:00", dt.format("%Y-%m-%d"), bucket_start)
        }
        Resolution::Day => unreachable!(),
    }
}

fn compute_date_stats(
    records: &[&TokenRecord],
    tz: Option<&FixedOffset>,
    resolution: Resolution,
) -> Vec<DateStats> {
    // Accumulator that excludes xunfei provider (which has no cache mechanism)
    #[derive(Default)]
    struct PeriodAccum {
        all: StatAccum,
        no_xunfei: StatAccum,
        has_xunfei: bool,
    }

    let mut map: HashMap<String, PeriodAccum> = HashMap::new();

    for r in records {
        let key = period_key_for_record(r, tz, resolution);
        let acc = map.entry(key).or_default();
        acc.all.accumulate(r);
        if is_xunfei_provider(&r.provider) {
            acc.has_xunfei = true;
        } else {
            acc.no_xunfei.accumulate(r);
        }
    }

    let mut result: Vec<DateStats> = map
        .into_iter()
        .map(|(date, acc)| {
            // Always return a value: when no xunfei data exists, the
            // overall ratio is already xunfei-free, so use it directly.
            let cache_hit_ratio_no_xunfei = if acc.has_xunfei {
                acc.no_xunfei.cache_hit_ratio()
            } else {
                acc.all.cache_hit_ratio()
            };
            DateStats {
                date,
                calls: acc.all.calls,
                input_tokens: acc.all.input_tokens,
                output_tokens: acc.all.output_tokens,
                cache_read_tokens: acc.all.cache_read_tokens,
                cache_write_tokens: acc.all.cache_write_tokens,
                total_tokens: acc.all.total_tokens,
                cost: acc.all.cost,
                cache_hit_ratio: acc.all.cache_hit_ratio(),
                cache_hit_ratio_no_xunfei,
            }
        })
        .collect();

    result.sort_by(|a, b| a.date.cmp(&b.date));
    result
}

// ── Per-model RPM from raw timestamps ───────────────────────────────────────

/// Compute (avg_rpm, peak_rpm) from a list of RFC3339 timestamp strings.
///
/// Algorithm:
/// 1. Parse timestamps → round to minute buckets → count per minute.
/// 2. Sort minute-buckets chronologically.
/// 3. Detect active windows: a gap of ≥5 min with zero requests = boundary.
/// 4. Fill zero-request minutes within each window.
/// 5. avg_rpm = total_requests / total_active_minutes; peak_rpm = max bucket count.
fn compute_rpm_from_times(times: &[String]) -> (f64, i64) {
    if times.is_empty() {
        return (0.0, 0);
    }
    const GAP_THRESHOLD: i64 = 5;

    // 1. Count per minute
    let mut minute_map: HashMap<String, i64> = HashMap::new();
    for t in times {
        if let Some(key) = parse_time_to_minute_key(t) {
            *minute_map.entry(key).or_default() += 1;
        }
    }

    if minute_map.is_empty() {
        return (0.0, 0);
    }

    // 2. Sort chronologically
    let mut sorted_keys: Vec<String> = minute_map.keys().cloned().collect();
    sorted_keys.sort();

    // 3. Detect active windows & count total active minutes + requests
    let mut total_active_minutes: i64 = 0;
    let mut total_requests: i64 = 0;
    let mut window_start_idx: usize = 0;

    for i in 1..sorted_keys.len() {
        let prev_dt = parse_minute_key(&sorted_keys[i - 1]);
        let curr_dt = parse_minute_key(&sorted_keys[i]);
        let gap: i64 = match (prev_dt, curr_dt) {
            (Some(p), Some(c)) => (c - p).num_minutes(),
            _ => 1,
        };
        if gap > GAP_THRESHOLD {
            // Close window: count minutes from sorted_keys[window_start_idx] to sorted_keys[i-1]
            let (mins, reqs) = count_window_minutes_and_requests(
                &sorted_keys[window_start_idx..i],
                &minute_map,
            );
            total_active_minutes += mins;
            total_requests += reqs;
            window_start_idx = i;
        }
    }
    // Close last window
    let (mins, reqs) = count_window_minutes_and_requests(
        &sorted_keys[window_start_idx..],
        &minute_map,
    );
    total_active_minutes += mins;
    total_requests += reqs;

    let avg_rpm = if total_active_minutes > 0 {
        total_requests as f64 / total_active_minutes as f64
    } else {
        0.0
    };
    let peak_rpm = minute_map.values().copied().max().unwrap_or(0);

    (avg_rpm, peak_rpm)
}

/// Parse an RFC3339 timestamp string into a minute key "YYYY-MM-DD HH:MM".
fn parse_time_to_minute_key(time_str: &str) -> Option<String> {
    let dt = chrono::DateTime::parse_from_rfc3339(time_str).ok()?;
    Some(format!(
        "{} {:02}:{:02}",
        dt.format("%Y-%m-%d"),
        dt.hour(),
        dt.minute()
    ))
}

/// Count the number of minutes (including zero-request fill) and total requests
/// in a window spanned by the given sorted minute keys.
fn count_window_minutes_and_requests(
    keys: &[String],
    minute_map: &HashMap<String, i64>,
) -> (i64, i64) {
    if keys.is_empty() {
        return (0, 0);
    }
    let start_dt = match parse_minute_key(&keys[0]) {
        Some(dt) => dt,
        None => return (keys.len() as i64, keys.iter().map(|k| minute_map.get(k).copied().unwrap_or(0)).sum()),
    };
    let end_dt = match parse_minute_key(&keys[keys.len() - 1]) {
        Some(dt) => dt,
        None => return (keys.len() as i64, keys.iter().map(|k| minute_map.get(k).copied().unwrap_or(0)).sum()),
    };
    let duration_minutes = (end_dt - start_dt).num_minutes() + 1;
    let total_requests: i64 = keys.iter().map(|k| minute_map.get(k).copied().unwrap_or(0)).sum();
    (duration_minutes, total_requests)
}

fn compute_model_stats(records: &[&TokenRecord]) -> Vec<ModelStats> {
    struct Agg {
        accum: StatAccum,
        source_set: HashSet<String>,
        source_aggs: HashMap<String, StatAccum>,
        /// Collect timestamps for RPM calculation (provider+model level)
        times: Vec<String>,
        /// Collect timestamps per source for RPM calculation
        source_times: HashMap<String, Vec<String>>,
    }

    let mut map: HashMap<(String, String), Agg> = HashMap::new();

    for r in records {
        let key = (r.provider.clone(), r.model.clone());
        let agg = map.entry(key).or_insert_with(|| Agg {
            accum: StatAccum::default(),
            source_set: HashSet::new(),
            source_aggs: HashMap::new(),
            times: Vec::new(),
            source_times: HashMap::new(),
        });
        agg.accum.accumulate(r);
        agg.source_set.insert(r.source.clone());
        agg.source_aggs
            .entry(r.source.clone())
            .or_default()
            .accumulate(r);
        agg.times.push(r.time.clone());
        agg.source_times
            .entry(r.source.clone())
            .or_default()
            .push(r.time.clone());
    }

    let mut result: Vec<ModelStats> = map
        .into_iter()
        .map(|((provider, model), agg)| {
            let mut sources: Vec<String> = agg.source_set.into_iter().collect();
            sources.sort();
            // Compute RPM for this provider+model from its timestamps
            let (avg_rpm, peak_rpm) = compute_rpm_from_times(&agg.times);
            let mut source_details: Vec<SourceDetailStats> = agg
                .source_aggs
                .into_iter()
                .map(|(source, acc)| {
                    let source_times = agg.source_times.get(&source).map(|v| v.as_slice()).unwrap_or(&[]);
                    let (source_avg_rpm, source_peak_rpm) = compute_rpm_from_times(source_times);
                    SourceDetailStats {
                        source,
                        calls: acc.calls,
                        input_tokens: acc.input_tokens,
                        output_tokens: acc.output_tokens,
                        cache_read_tokens: acc.cache_read_tokens,
                        cache_write_tokens: acc.cache_write_tokens,
                        total_tokens: acc.total_tokens,
                        cost: acc.cost,
                        cache_hit_ratio: acc.cache_hit_ratio(),
                        avg_rpm: source_avg_rpm,
                        peak_rpm: source_peak_rpm,
                    }
                })
                .collect();
            source_details.sort_by_key(|s| std::cmp::Reverse(s.total_tokens));
            ModelStats {
                model,
                provider,
                sources,
                calls: agg.accum.calls,
                input_tokens: agg.accum.input_tokens,
                output_tokens: agg.accum.output_tokens,
                cache_read_tokens: agg.accum.cache_read_tokens,
                cache_write_tokens: agg.accum.cache_write_tokens,
                total_tokens: agg.accum.total_tokens,
                cost: agg.accum.cost,
                cache_hit_ratio: agg.accum.cache_hit_ratio(),
                source_details,
                avg_rpm,
                peak_rpm,
            }
        })
        .collect();

    result.sort_by_key(|m| std::cmp::Reverse(m.total_tokens));
    result
}

fn compute_source_stats(records: &[&TokenRecord]) -> Vec<SourceStats> {
    let mut map: HashMap<String, StatAccum> = HashMap::new();

    for r in records {
        map.entry(r.source.clone()).or_default().accumulate(r);
    }

    let mut result: Vec<SourceStats> = map
        .into_iter()
        .map(|(source, acc)| SourceStats {
            source,
            calls: acc.calls,
            input_tokens: acc.input_tokens,
            output_tokens: acc.output_tokens,
            cache_read_tokens: acc.cache_read_tokens,
            cache_write_tokens: acc.cache_write_tokens,
            total_tokens: acc.total_tokens,
            cost: acc.cost,
            cache_hit_ratio: acc.cache_hit_ratio(),
        })
        .collect();

    result.sort_by_key(|s| std::cmp::Reverse(s.total_tokens));
    result
}

// ── RPM Analysis ────────────────────────────────────────────────────────────

/// Compute the local minute-bucket key for a record ("YYYY-MM-DD HH:MM").
fn minute_key_for_record(record: &TokenRecord, tz: Option<&FixedOffset>) -> Option<String> {
    let local_dt = if let Some(tz) = tz {
        record.parsed_time().map(|dt| dt.with_timezone(tz).naive_local())
    } else {
        record.parsed_time().map(|dt| dt.naive_utc())
    };
    local_dt.map(|dt| format!("{} {:02}:{:02}", dt.format("%Y-%m-%d"), dt.hour(), dt.minute()))
}

/// Parse a minute key back to a chrono NaiveDateTime for arithmetic.
fn parse_minute_key(key: &str) -> Option<chrono::NaiveDateTime> {
    // Format: "YYYY-MM-DD HH:MM"
    chrono::NaiveDateTime::parse_from_str(key, "%Y-%m-%d %H:%M").ok()
}

/// Compute Requests-Per-Minute analysis with active-window boundary detection.
///
/// Algorithm:
/// 1. Group all filtered records into 1-minute buckets.
/// 2. Sort buckets chronologically.
/// 3. Detect "active windows" — consecutive periods where requests occur.
///    A gap of `gap_threshold_minutes` or more with zero requests marks a boundary.
///    We also fill in zero-request minutes within a window (between first and last
///    request) for a complete timeline.
/// 4. Compute RPM metrics per window and overall.
pub fn compute_rpm_analysis(
    records: &[TokenRecord],
    filters: &FilterCriteria,
    gap_threshold_minutes: i64,
) -> RpmAnalysis {
    let sources = parse_csv_filter(filters.source);
    let providers = parse_csv_filter(filters.provider);
    let models = parse_csv_filter(filters.model);
    let filtered: Vec<&TokenRecord> = records
        .iter()
        .filter(|r| record_matches_bound(r, filters.from, filters.to, filters.tz))
        .filter(|r| sources.is_empty() || sources.contains(&r.source.as_str()))
        .filter(|r| providers.is_empty() || providers.contains(&r.provider.as_str()))
        .filter(|r| models.is_empty() || models.contains(&r.model.as_str()))
        .collect();

    // 1. Count requests per minute
    let mut minute_map: HashMap<String, i64> = HashMap::new();
    for r in &filtered {
        if let Some(key) = minute_key_for_record(r, filters.tz) {
            *minute_map.entry(key).or_default() += 1;
        }
    }

    if minute_map.is_empty() {
        return RpmAnalysis {
            all_buckets: vec![],
            windows: vec![],
            overall_avg_rpm: 0.0,
            overall_peak_rpm: 0,
            total_active_minutes: 0,
            gap_threshold_minutes,
        };
    }

    // 2. Sort minute keys chronologically
    let mut sorted_keys: Vec<String> = minute_map.keys().cloned().collect();
    sorted_keys.sort();

    // 3. Detect active windows using gap threshold
    let mut windows: Vec<ActiveWindow> = Vec::new();
    let mut window_start_idx: usize = 0;

    for i in 1..sorted_keys.len() {
        let prev_dt = parse_minute_key(&sorted_keys[i - 1]);
        let curr_dt = parse_minute_key(&sorted_keys[i]);

        let gap: i64 = match (prev_dt, curr_dt) {
            (Some(p), Some(c)) => {
                let diff = c - p;
                diff.num_minutes()
            }
            _ => {
                // Fallback: if parsing fails, compare strings
                if sorted_keys[i] != sorted_keys[i - 1] { 1 } else { 0 }
            }
        };

        if gap > gap_threshold_minutes {
            // Close current window
            let window_keys = &sorted_keys[window_start_idx..i];
            if let Some(w) = build_window(window_keys, &minute_map) {
                windows.push(w);
            }
            window_start_idx = i;
        }
    }

    // Close last window
    let window_keys = &sorted_keys[window_start_idx..];
    if let Some(w) = build_window(window_keys, &minute_map) {
        windows.push(w);
    }

    // 4. Build the full all_buckets list (filling in zero-request minutes within windows)
    let mut all_buckets: Vec<MinuteBucket> = Vec::new();
    for w in &windows {
        let start_dt = match parse_minute_key(&w.start) {
            Some(dt) => dt,
            None => continue,
        };
        let end_dt = match parse_minute_key(&w.end) {
            Some(dt) => dt,
            None => continue,
        };
        let mut cursor = start_dt;
        loop {
            let key = format!("{} {:02}:{:02}", cursor.format("%Y-%m-%d"), cursor.hour(), cursor.minute());
            let requests = minute_map.get(&key).copied().unwrap_or(0);
            all_buckets.push(MinuteBucket {
                minute: key,
                requests,
            });
            if cursor >= end_dt {
                break;
            }
            cursor += chrono::Duration::minutes(1);
        }
    }

    // 5. Compute overall stats
    let total_requests: i64 = windows.iter().map(|w| w.total_requests).sum();
    let total_active_minutes: i64 = windows.iter().map(|w| w.duration_minutes).sum();
    let overall_avg_rpm = if total_active_minutes > 0 {
        total_requests as f64 / total_active_minutes as f64
    } else {
        0.0
    };
    let overall_peak_rpm = windows
        .iter()
        .map(|w| w.peak_rpm)
        .max()
        .unwrap_or(0);

    RpmAnalysis {
        all_buckets,
        windows,
        overall_avg_rpm,
        overall_peak_rpm,
        total_active_minutes,
        gap_threshold_minutes,
    }
}

/// Build an ActiveWindow from a slice of sorted minute keys that belong together.
/// Computes window stats without storing per-minute buckets (those live in `all_buckets`).
fn build_window(window_keys: &[String], minute_map: &HashMap<String, i64>) -> Option<ActiveWindow> {
    if window_keys.is_empty() {
        return None;
    }

    let start = window_keys[0].clone();
    let end = window_keys[window_keys.len() - 1].clone();

    let start_dt = parse_minute_key(&start)?;
    let end_dt = parse_minute_key(&end)?;

    // Compute duration and total requests without materializing every minute bucket
    let duration_minutes = (end_dt - start_dt).num_minutes() + 1;
    let total_requests: i64 = window_keys.iter().map(|k| minute_map.get(k).copied().unwrap_or(0)).sum();
    let peak_rpm = window_keys.iter().map(|k| minute_map.get(k).copied().unwrap_or(0)).max().unwrap_or(0);
    let avg_rpm = if duration_minutes > 0 {
        total_requests as f64 / duration_minutes as f64
    } else {
        0.0
    };

    Some(ActiveWindow {
        start,
        end,
        duration_minutes,
        total_requests,
        avg_rpm,
        peak_rpm,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{NaiveDate, NaiveDateTime};

    fn record(
        source: &str,
        provider: &str,
        model: &str,
        time: &str,
        total_tokens: i64,
    ) -> TokenRecord {
        TokenRecord {
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

    #[test]
    fn aggregate_records_accepts_comma_separated_source_and_provider_filters() {
        let records = vec![
            record("pi", "openai", "gpt-5.5", "2026-05-17T10:00:00Z", 100),
            record("codex", "openai", "gpt-5.5", "2026-05-17T11:00:00Z", 200),
            record("kimi-cli", "kimi", "kimi-k2", "2026-05-17T12:00:00Z", 300),
            record(
                "claude-code",
                "anthropic",
                "claude-sonnet",
                "2026-05-17T13:00:00Z",
                400,
            ),
        ];

        let stats = aggregate_records(
            &records,
            &FilterCriteria {
                from: None,
                to: None,
                source: Some("pi,codex"),
                provider: Some("openai"),
                model: None,
                tz: None,
            },
            Resolution::Day,
        );

        assert_eq!(stats.overall.total_calls, 2);
        assert_eq!(stats.overall.total_tokens, 300);
        assert_eq!(stats.by_source.len(), 2);
        assert!(stats.by_source.iter().any(|s| s.source == "pi"));
        assert!(stats.by_source.iter().any(|s| s.source == "codex"));
    }

    #[test]
    fn model_filter_filters_by_single_model() {
        let records = vec![
            record("pi", "openai", "gpt-4", "2026-05-17T10:00:00Z", 100),
            record("pi", "openai", "claude-3", "2026-05-17T11:00:00Z", 200),
            record(
                "codex",
                "openai",
                "gpt-4-turbo",
                "2026-05-17T12:00:00Z",
                300,
            ),
        ];

        let stats = aggregate_records(
            &records,
            &FilterCriteria {
                from: None,
                to: None,
                source: None,
                provider: None,
                model: Some("gpt-4"),
                tz: None,
            },
            Resolution::Day,
        );

        assert_eq!(stats.overall.total_calls, 1);
        assert_eq!(stats.overall.total_tokens, 100);
        assert_eq!(stats.by_model.len(), 1);
        assert_eq!(stats.by_model[0].model, "gpt-4");
    }

    #[test]
    fn model_filter_filters_by_comma_separated_models() {
        let records = vec![
            record("pi", "openai", "gpt-4", "2026-05-17T10:00:00Z", 100),
            record("pi", "openai", "claude-3", "2026-05-17T11:00:00Z", 200),
            record(
                "codex",
                "openai",
                "gpt-4-turbo",
                "2026-05-17T12:00:00Z",
                300,
            ),
        ];

        let stats = aggregate_records(
            &records,
            &FilterCriteria {
                from: None,
                to: None,
                source: None,
                provider: None,
                model: Some("gpt-4,claude-3"),
                tz: None,
            },
            Resolution::Day,
        );

        assert_eq!(stats.overall.total_calls, 2);
        assert_eq!(stats.overall.total_tokens, 300);
        assert!(stats.by_model.iter().any(|m| m.model == "gpt-4"));
        assert!(stats.by_model.iter().any(|m| m.model == "claude-3"));
    }

    #[test]
    fn model_filter_none_passes_all_records() {
        let records = vec![
            record("pi", "openai", "gpt-4", "2026-05-17T10:00:00Z", 100),
            record("pi", "openai", "claude-3", "2026-05-17T11:00:00Z", 200),
            record(
                "codex",
                "openai",
                "gpt-4-turbo",
                "2026-05-17T12:00:00Z",
                300,
            ),
        ];

        let stats = aggregate_records(
            &records,
            &FilterCriteria {
                from: None,
                to: None,
                source: None,
                provider: None,
                model: None,
                tz: None,
            },
            Resolution::Day,
        );

        assert_eq!(stats.overall.total_calls, 3);
        assert_eq!(stats.overall.total_tokens, 600);
    }

    #[test]
    fn datetime_bounds_are_interpreted_in_requested_local_timezone() {
        let records = vec![
            record("pi", "openai", "before", "2026-05-17T11:30:00Z", 100),
            record("pi", "openai", "inside", "2026-05-17T12:30:00Z", 200),
            record("pi", "openai", "after", "2026-05-17T13:30:00Z", 300),
        ];
        let tz = FixedOffset::east_opt(8 * 60 * 60).unwrap();
        let from = TimeBound::DateTime(local_dt(2026, 5, 17, 20, 0, 0));
        let to = TimeBound::DateTime(local_dt(2026, 5, 17, 21, 0, 0));

        let filtered = filter_records(
            &records,
            &FilterCriteria {
                from: Some(&from),
                to: Some(&to),
                source: Some("pi"),
                provider: None,
                model: None,
                tz: Some(&tz),
            },
        );

        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].model, "inside");
    }

    fn local_dt(
        year: i32,
        month: u32,
        day: u32,
        hour: u32,
        minute: u32,
        second: u32,
    ) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(year, month, day)
            .unwrap()
            .and_hms_opt(hour, minute, second)
            .unwrap()
    }
}
