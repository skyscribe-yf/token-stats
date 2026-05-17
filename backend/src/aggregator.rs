use crate::models::*;
use crate::routes::TimeBound;
use chrono::{FixedOffset, Utc};
use std::collections::{HashMap, HashSet};

/// Get the local date string for a record, given an optional timezone offset
fn local_date_for_record(record: &TokenRecord, tz: Option<&FixedOffset>) -> String {
    if let Some(tz) = tz {
        // Convert UTC time to local timezone and extract the date
        if let Some(utc_dt) = record.parsed_time() {
            let local_dt = utc_dt.with_timezone(tz);
            return local_dt.format("%Y-%m-%d").to_string();
        }
    }
    // Fallback: use the record's own date field (UTC-based)
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
    let by_date = compute_date_stats(&filtered, tz);
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
    let total_pages = total.div_ceil(limit);
    let start = (page - 1) * limit;
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
                cost: r.cost,
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

fn compute_overall_stats(records: &[&TokenRecord]) -> AggregatedStats {
    let mut total_calls = 0i64;
    let mut total_input = 0i64;
    let mut total_output = 0i64;
    let mut total_cache_read = 0i64;
    let mut total_cache_write = 0i64;
    let mut total_tokens = 0i64;
    let mut total_cost = 0.0;
    let mut total_cache_hit_ratio = 0.0;

    for r in records {
        total_calls += 1;
        total_input += r.input_tokens;
        total_output += r.output_tokens;
        total_cache_read += r.cache_read_tokens;
        total_cache_write += r.cache_write_tokens;
        total_tokens += r.total_tokens;
        total_cost += r.cost;
        total_cache_hit_ratio += r.cache_hit_ratio();
    }

    let avg_cache_hit_ratio = if !records.is_empty() {
        total_cache_hit_ratio / records.len() as f64
    } else {
        0.0
    };

    let weighted_cache_hit_ratio = if total_input + total_cache_read > 0 {
        total_cache_read as f64 / (total_input + total_cache_read) as f64 * 100.0
    } else {
        0.0
    };

    AggregatedStats {
        total_calls,
        total_input_tokens: total_input,
        total_output_tokens: total_output,
        total_cache_read_tokens: total_cache_read,
        total_cache_write_tokens: total_cache_write,
        total_tokens,
        total_cost,
        avg_cache_hit_ratio,
        weighted_cache_hit_ratio,
    }
}

fn compute_vendor_stats(records: &[&TokenRecord]) -> Vec<VendorStats> {
    let mut map: HashMap<String, VendorStats> = HashMap::new();

    for r in records {
        let entry = map
            .entry(r.provider.clone())
            .or_insert_with(|| VendorStats {
                provider: r.provider.clone(),
                calls: 0,
                input_tokens: 0,
                output_tokens: 0,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
                total_tokens: 0,
                cost: 0.0,
                cache_hit_ratio: 0.0,
            });

        entry.calls += 1;
        entry.input_tokens += r.input_tokens;
        entry.output_tokens += r.output_tokens;
        entry.cache_read_tokens += r.cache_read_tokens;
        entry.cache_write_tokens += r.cache_write_tokens;
        entry.total_tokens += r.total_tokens;
        entry.cost += r.cost;
    }

    let mut result: Vec<VendorStats> = map.into_values().collect();
    for v in &mut result {
        let total_input = v.input_tokens + v.cache_read_tokens;
        if total_input > 0 {
            v.cache_hit_ratio = v.cache_read_tokens as f64 / total_input as f64 * 100.0;
        }
    }

    result.sort_by_key(|v| std::cmp::Reverse(v.total_tokens));
    result
}

fn compute_date_stats(records: &[&TokenRecord], tz: Option<&FixedOffset>) -> Vec<DateStats> {
    let mut map: HashMap<String, DateStats> = HashMap::new();

    for r in records {
        let local_date = local_date_for_record(r, tz);
        let entry = map.entry(local_date.clone()).or_insert_with(|| DateStats {
            date: local_date,
            calls: 0,
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            total_tokens: 0,
            cost: 0.0,
            cache_hit_ratio: 0.0,
        });

        entry.calls += 1;
        entry.input_tokens += r.input_tokens;
        entry.output_tokens += r.output_tokens;
        entry.cache_read_tokens += r.cache_read_tokens;
        entry.cache_write_tokens += r.cache_write_tokens;
        entry.total_tokens += r.total_tokens;
        entry.cost += r.cost;
    }

    let mut result: Vec<DateStats> = map.into_values().collect();
    for d in &mut result {
        let total_input = d.input_tokens + d.cache_read_tokens;
        if total_input > 0 {
            d.cache_hit_ratio = d.cache_read_tokens as f64 / total_input as f64 * 100.0;
        }
    }

    result.sort_by(|a, b| a.date.cmp(&b.date));
    result
}

fn compute_model_stats(records: &[&TokenRecord]) -> Vec<ModelStats> {
    struct Agg {
        stats: ModelStats,
        source_set: HashSet<String>,
    }

    let mut map: HashMap<(String, String), Agg> = HashMap::new();

    for r in records {
        let key = (r.provider.clone(), r.model.clone());
        let agg = map.entry(key).or_insert_with(|| Agg {
            stats: ModelStats {
                model: r.model.clone(),
                provider: r.provider.clone(),
                sources: Vec::new(),
                calls: 0,
                input_tokens: 0,
                output_tokens: 0,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
                total_tokens: 0,
                cost: 0.0,
                cache_hit_ratio: 0.0,
            },
            source_set: HashSet::new(),
        });

        agg.stats.calls += 1;
        agg.stats.input_tokens += r.input_tokens;
        agg.stats.output_tokens += r.output_tokens;
        agg.stats.cache_read_tokens += r.cache_read_tokens;
        agg.stats.cache_write_tokens += r.cache_write_tokens;
        agg.stats.total_tokens += r.total_tokens;
        agg.stats.cost += r.cost;
        agg.source_set.insert(r.source.clone());
    }

    let mut result: Vec<ModelStats> = map
        .into_values()
        .map(|mut agg| {
            let mut sources: Vec<String> = agg.source_set.into_iter().collect();
            sources.sort();
            agg.stats.sources = sources;
            agg.stats
        })
        .collect();

    for m in &mut result {
        let total_input = m.input_tokens + m.cache_read_tokens;
        if total_input > 0 {
            m.cache_hit_ratio = m.cache_read_tokens as f64 / total_input as f64 * 100.0;
        }
    }

    result.sort_by_key(|m| std::cmp::Reverse(m.total_tokens));
    result
}

fn compute_source_stats(records: &[&TokenRecord]) -> Vec<SourceStats> {
    let mut map: HashMap<String, SourceStats> = HashMap::new();

    for r in records {
        let entry = map.entry(r.source.clone()).or_insert_with(|| SourceStats {
            source: r.source.clone(),
            calls: 0,
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            total_tokens: 0,
            cost: 0.0,
            cache_hit_ratio: 0.0,
        });

        entry.calls += 1;
        entry.input_tokens += r.input_tokens;
        entry.output_tokens += r.output_tokens;
        entry.cache_read_tokens += r.cache_read_tokens;
        entry.cache_write_tokens += r.cache_write_tokens;
        entry.total_tokens += r.total_tokens;
        entry.cost += r.cost;
    }

    let mut result: Vec<SourceStats> = map.into_values().collect();
    for s in &mut result {
        let total_input = s.input_tokens + s.cache_read_tokens;
        if total_input > 0 {
            s.cache_hit_ratio = s.cache_read_tokens as f64 / total_input as f64 * 100.0;
        }
    }

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

        let stats = aggregate_records(&records, None, None, Some("pi,codex"), Some("openai"), None);

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
