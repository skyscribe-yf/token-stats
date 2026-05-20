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

pub fn aggregate_records(
    records: &[TokenRecord],
    from: Option<&TimeBound>,
    to: Option<&TimeBound>,
    source: Option<&str>,
    provider: Option<&str>,
    tz: Option<&FixedOffset>,
    resolution: Resolution,
) -> StatsResponse {
    let sources = parse_csv_filter(source);
    let providers = parse_csv_filter(provider);
    let filtered: Vec<&TokenRecord> = records
        .iter()
        .filter(|r| record_matches_bound(r, from, to, tz))
        .filter(|r| sources.is_empty() || sources.contains(&r.source.as_str()))
        .filter(|r| providers.is_empty() || providers.contains(&r.provider.as_str()))
        .collect();

    let overall = compute_overall_stats(&filtered);
    let by_vendor = compute_vendor_stats(&filtered);
    let by_date = compute_date_stats(&filtered, tz, resolution);
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
    from: Option<&TimeBound>,
    to: Option<&TimeBound>,
    provider: Option<&str>,
    model: Option<&str>,
    source: Option<&str>,
    tz: Option<&FixedOffset>,
) -> Vec<&'a TokenRecord> {
    let sources = parse_csv_filter(source);
    let providers = parse_csv_filter(provider);
    let mut filtered: Vec<&'a TokenRecord> = records
        .iter()
        .filter(|r| record_matches_bound(r, from, to, tz))
        .filter(|r| {
            let provider_ok = providers.is_empty() || providers.contains(&r.provider.as_str());
            let model_ok = model.map(|m| r.model == m).unwrap_or(true);
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

fn compute_model_stats(records: &[&TokenRecord]) -> Vec<ModelStats> {
    struct Agg {
        accum: StatAccum,
        source_set: HashSet<String>,
    }

    let mut map: HashMap<(String, String), Agg> = HashMap::new();

    for r in records {
        let key = (r.provider.clone(), r.model.clone());
        let agg = map.entry(key).or_insert_with(|| Agg {
            accum: StatAccum::default(),
            source_set: HashSet::new(),
        });
        agg.accum.accumulate(r);
        agg.source_set.insert(r.source.clone());
    }

    let mut result: Vec<ModelStats> = map
        .into_iter()
        .map(|((provider, model), agg)| {
            let mut sources: Vec<String> = agg.source_set.into_iter().collect();
            sources.sort();
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
            None,
            None,
            Some("pi,codex"),
            Some("openai"),
            None,
            Resolution::Day,
        );

        assert_eq!(stats.overall.total_calls, 2);
        assert_eq!(stats.overall.total_tokens, 300);
        assert_eq!(stats.by_source.len(), 2);
        assert!(stats.by_source.iter().any(|s| s.source == "pi"));
        assert!(stats.by_source.iter().any(|s| s.source == "codex"));
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
            Some(&from),
            Some(&to),
            None,
            None,
            Some("pi"),
            Some(&tz),
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
