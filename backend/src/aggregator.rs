use crate::models::*;
use crate::routes::TimeBound;
use std::collections::HashMap;

pub fn aggregate_records(
    records: &[TokenRecord],
    from: Option<&TimeBound>,
    to: Option<&TimeBound>,
    source: Option<&str>,
) -> StatsResponse {
    let filtered: Vec<&TokenRecord> = records
        .iter()
        .filter(|r| record_matches_bound(r, from, to))
        .filter(|r| source.map(|s| r.source == s).unwrap_or(true))
        .collect();

    let overall = compute_overall_stats(&filtered);
    let by_vendor = compute_vendor_stats(&filtered);
    let by_date = compute_date_stats(&filtered);
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
) -> Vec<&'a TokenRecord> {
    records
        .iter()
        .filter(|r| record_matches_bound(r, from, to))
        .filter(|r| {
            let provider_ok = provider.map(|p| r.provider == p).unwrap_or(true);
            let model_ok = model.map(|m| r.model == m).unwrap_or(true);
            let source_ok = source.map(|s| r.source == s).unwrap_or(true);
            provider_ok && model_ok && source_ok
        })
        .collect()
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

pub fn paginate_requests(
    records: Vec<&TokenRecord>,
    page: usize,
    limit: usize,
) -> PaginatedRequests {
    let total = records.len();
    let total_pages = (total + limit - 1) / limit;
    let start = (page - 1) * limit;
    let end = (start + limit).min(total);

    let data: Vec<DetailedRequest> = records[start..end]
        .iter()
        .map(|r| DetailedRequest {
            date: r.date.clone(),
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
        let entry = map.entry(r.provider.clone()).or_insert_with(|| VendorStats {
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

    result.sort_by(|a, b| b.total_tokens.cmp(&a.total_tokens));
    result
}

fn compute_date_stats(records: &[&TokenRecord]) -> Vec<DateStats> {
    let mut map: HashMap<String, DateStats> = HashMap::new();

    for r in records {
        let entry = map.entry(r.date.clone()).or_insert_with(|| DateStats {
            date: r.date.clone(),
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
    let mut map: HashMap<(String, String), ModelStats> = HashMap::new();

    for r in records {
        let key = (r.provider.clone(), r.model.clone());
        let entry = map.entry(key).or_insert_with(|| ModelStats {
            model: r.model.clone(),
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

    let mut result: Vec<ModelStats> = map.into_values().collect();
    for m in &mut result {
        let total_input = m.input_tokens + m.cache_read_tokens;
        if total_input > 0 {
            m.cache_hit_ratio = m.cache_read_tokens as f64 / total_input as f64 * 100.0;
        }
    }

    result.sort_by(|a, b| b.total_tokens.cmp(&a.total_tokens));
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

    result.sort_by(|a, b| b.total_tokens.cmp(&a.total_tokens));
    result
}