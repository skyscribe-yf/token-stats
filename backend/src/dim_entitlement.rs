//! DimAgent entitlement billing rates.
//!
//! The Dim console bills a subscription model at **platform list price ×
//! `rate`**, where `rate` comes from the account's own entitlement payload
//! (`model_access[].rate`, plus `rate_windows[]` for time-of-day discounts).
//! Rates change without notice — the platform cut `deepseek-v4.1-flash` to
//! 0.65 on 2026-09-18 — and the payload carries **no effective timestamp**, so
//! reading it live would silently rewrite all history at the newest value.
//!
//! Each observation is therefore merged into a local segment history
//! (`~/.config/token-stats/dim-entitlement.json`) and a changed rate only takes
//! effect from the moment it was first seen. Records predating a model's first
//! segment are billed at list price (rate 1.0).

use crate::quota::QuotaFetcher;
use chrono::{DateTime, FixedOffset, Timelike, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// China Standard Time — the platform expresses every billing window in CST.
const CST: i32 = 8 * 3600;
/// Keep a bounded history per model; superseded transitions stay in the file's
/// earlier entries only as long as they still affect visible records.
const MAX_SEGMENTS_PER_MODEL: usize = 64;
/// How often to re-read the account's entitlement payload. Rates change rarely
/// and each read spawns `dim usage`, so this is deliberately slow.
const DEFAULT_TTL_SECS: u64 = 1800;

/// A time-of-day discount window from `rate_windows[]`, e.g. 夜间优惠
/// `20:00 → 08:00` at 0.5 on top of the model's base rate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RateWindow {
    /// `"HH:MM"` CST, inclusive.
    pub start: String,
    /// `"HH:MM"` CST, exclusive. Wraps midnight when `start > end`.
    pub end: String,
    pub rate: f64,
}

/// One observed rate regime for a model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RateSegment {
    /// RFC3339 with offset; inclusive start of this regime.
    pub effective_from: String,
    pub rate: f64,
    #[serde(default)]
    pub windows: Vec<RateWindow>,
}

/// `{"models": {model: [segments]}}`, the on-disk form.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct StoredRates {
    #[serde(default)]
    models: HashMap<String, Vec<RateSegment>>,
}

#[derive(Debug, Clone)]
struct PreparedWindow {
    start_min: u32,
    end_min: u32,
    wraps: bool,
    rate: f64,
}

/// A [`RateSegment`] with its timestamp and windows parsed once at load time,
/// so `rate_for` stays cheap across the ~10^5 records a full aggregation prices.
#[derive(Debug, Clone)]
pub struct PreparedSegment {
    effective_from: String,
    from: DateTime<FixedOffset>,
    rate: f64,
    windows: Vec<PreparedWindow>,
}

/// Model (lowercase) → segments sorted by `effective_from` ascending.
pub type RateTable = HashMap<String, Vec<PreparedSegment>>;

fn state_path() -> PathBuf {
    let override_path = std::env::var("DIM_ENTITLEMENT_STATE_PATH")
        .ok()
        .filter(|p| !p.trim().is_empty());
    if let Some(p) = override_path {
        return PathBuf::from(p);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home)
        .join(".config")
        .join("token-stats")
        .join("dim-entitlement.json")
}

fn parse_rfc3339(value: &str) -> Option<DateTime<FixedOffset>> {
    DateTime::parse_from_rfc3339(value).ok()
}

/// `"HH:MM"` → minutes since CST midnight.
fn parse_clock(value: &str) -> Option<u32> {
    let (h, m) = value.trim().split_once(':')?;
    let (h, m) = (h.parse::<u32>().ok()?, m.parse::<u32>().ok()?);
    if h < 24 && m < 60 {
        Some(h * 60 + m)
    } else {
        None
    }
}

fn format_clock(minutes: u32) -> String {
    format!("{:02}:{:02}", minutes / 60, minutes % 60)
}

fn prepare(segments: &[RateSegment]) -> Vec<PreparedSegment> {
    let mut out: Vec<PreparedSegment> = segments
        .iter()
        .filter_map(|seg| {
            let from = parse_rfc3339(&seg.effective_from)?;
            let windows = seg
                .windows
                .iter()
                .filter_map(|w| {
                    let start_min = parse_clock(&w.start)?;
                    let end_min = parse_clock(&w.end)?;
                    Some(PreparedWindow {
                        start_min,
                        end_min,
                        wraps: start_min > end_min,
                        rate: w.rate,
                    })
                })
                .collect();
            Some(PreparedSegment {
                effective_from: seg.effective_from.clone(),
                from,
                rate: seg.rate,
                windows,
            })
        })
        .collect();
    out.sort_by_key(|s| s.from);
    out.dedup_by_key(|s| s.from);
    out
}

fn load_stored(path: &Path) -> StoredRates {
    match std::fs::read_to_string(path) {
        Ok(content) => serde_json::from_str(&content).unwrap_or_else(|e| {
            tracing::warn!("Unreadable dim entitlement state {path:?}: {e}; starting fresh");
            StoredRates::default()
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => StoredRates::default(),
        Err(e) => {
            tracing::warn!("Failed to read dim entitlement state {path:?}: {e}");
            StoredRates::default()
        }
    }
}

/// Read the segment history into a lookup table (called on pricing init/reload).
pub fn load_table() -> RateTable {
    load_table_from(&state_path())
}

/// Build a table from segment history directly — the same normalization
/// [`load_table`] applies, so tests can install rates without a state file.
#[cfg(test)]
pub fn build_table(models: &[(&str, Vec<RateSegment>)]) -> RateTable {
    models
        .iter()
        .map(|(model, segments)| ((*model).to_ascii_lowercase(), prepare(segments)))
        .collect()
}

fn load_table_from(path: &Path) -> RateTable {
    load_stored(path)
        .models
        .into_iter()
        .map(|(model, segs)| (model.to_ascii_lowercase(), prepare(&segs)))
        .collect()
}

/// A model's rate regime as the platform reports it right now.
#[derive(Debug, Clone, PartialEq)]
pub struct ObservedRate {
    pub model: String,
    pub rate: f64,
    pub windows: Vec<RateWindow>,
}

/// Extract `model_access[]` billing rates from an `entitlement_payload_json`
/// string. Models without a `rate` come back as 1.0 so that a revoked discount
/// is recorded rather than silently persisting the old one.
pub fn parse_payload(payload_json: &str) -> Result<Vec<ObservedRate>, String> {
    #[derive(Deserialize)]
    struct Payload {
        #[serde(default)]
        model_access: Vec<Entry>,
    }
    #[derive(Deserialize)]
    struct Entry {
        model: String,
        #[serde(default)]
        rate: Option<f64>,
        #[serde(default)]
        rate_windows: Vec<RawWindow>,
    }
    #[derive(Deserialize)]
    struct RawWindow {
        #[serde(default)]
        start: String,
        #[serde(default)]
        end: String,
        #[serde(default)]
        rate: f64,
    }

    let payload: Payload = serde_json::from_str(payload_json)
        .map_err(|e| format!("failed to parse entitlement payload: {e}"))?;
    let mut out = Vec::with_capacity(payload.model_access.len());
    for entry in payload.model_access {
        if entry.model.trim().is_empty() {
            continue;
        }
        let mut windows = entry
            .rate_windows
            .into_iter()
            .filter(|w| parse_clock(&w.start).is_some() && parse_clock(&w.end).is_some())
            .map(|w| RateWindow {
                start: w.start,
                end: w.end,
                rate: w.rate,
            })
            .collect::<Vec<_>>();
        // Order-insensitive comparison later, so the platform reshuffling its
        // window list is not mistaken for a rate change.
        windows.sort_by(|a, b| {
            a.start
                .cmp(&b.start)
                .then(a.end.cmp(&b.end))
                .then(a.rate.total_cmp(&b.rate))
        });
        out.push(ObservedRate {
            model: entry.model.to_ascii_lowercase(),
            rate: entry.rate.unwrap_or(1.0),
            windows,
        });
    }
    Ok(out)
}

/// Merge a fresh observation into the segment file. Returns the rebuilt table
/// only when a regime actually changed — `None` means nothing to do, which is
/// the common case for a 30-minute poll.
pub fn observe(observed: &[ObservedRate], at: DateTime<Utc>) -> Option<RateTable> {
    apply_observation(&state_path(), observed, at)
}

fn apply_observation(
    path: &Path,
    observed: &[ObservedRate],
    at: DateTime<Utc>,
) -> Option<RateTable> {
    let mut stored = load_stored(path);
    let stamp = at.to_rfc3339();
    let mut changed = false;

    for entry in observed {
        let segments = stored.models.entry(entry.model.clone()).or_default();
        if segments
            .last()
            .is_some_and(|last| last.rate == entry.rate && last.windows == entry.windows)
        {
            continue;
        }
        // A model that has never been discounted needs no entry: rate 1.0 with
        // no windows is the implicit baseline.
        if segments.is_empty() && entry.rate == 1.0 && entry.windows.is_empty() {
            continue;
        }
        segments.push(RateSegment {
            effective_from: stamp.clone(),
            rate: entry.rate,
            windows: entry.windows.clone(),
        });
        if segments.len() > MAX_SEGMENTS_PER_MODEL {
            let excess = segments.len() - MAX_SEGMENTS_PER_MODEL;
            segments.drain(0..excess);
        }
        changed = true;
        let windows = entry
            .windows
            .iter()
            .map(|w| format!(" {}-{}→{}", w.start, w.end, w.rate))
            .collect::<Vec<_>>()
            .join("");
        tracing::info!(
            "Dim entitlement rate for {} is now {}{windows} (effective {stamp})",
            entry.model,
            entry.rate
        );
    }
    stored.models.retain(|_, segments| !segments.is_empty());

    if !changed {
        return None;
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let json = match serde_json::to_string_pretty(&stored) {
        Ok(json) => json,
        Err(e) => {
            tracing::warn!("Failed to serialize dim entitlement state: {e}");
            return None;
        }
    };
    let temp = path.with_extension("json.tmp");
    if let Err(e) = std::fs::write(&temp, &json).and_then(|_| std::fs::rename(&temp, path)) {
        tracing::warn!("Failed to persist dim entitlement state {path:?}: {e}");
        return None;
    }
    Some(load_table_from(path))
}

/// The billing multiplier for `model` at `record_time`: 1.0 unless the account
/// had a discount for it back then. When several windows match, the deepest
/// discount wins.
pub fn rate_for(table: &RateTable, model: &str, record_time: &str) -> f64 {
    let lower;
    let segments = match table.get(model) {
        Some(segments) => segments.as_slice(),
        None => {
            lower = model.to_ascii_lowercase();
            table.get(&lower).map(|s| s.as_slice()).unwrap_or(&[])
        }
    };
    let Some(record) = parse_rfc3339(record_time) else {
        return 1.0;
    };
    let Some(segment) = segments.iter().rev().find(|s| record >= s.from) else {
        return 1.0;
    };
    let cst = record.with_timezone(&FixedOffset::east_opt(CST).unwrap());
    let minute = cst.hour() * 60 + cst.minute();
    let window_rate = segment
        .windows
        .iter()
        .filter(|w| {
            if w.wraps {
                minute >= w.start_min || minute < w.end_min
            } else {
                minute >= w.start_min && minute < w.end_min
            }
        })
        .map(|w| w.rate)
        .min_by(|a, b| a.total_cmp(b))
        .unwrap_or(1.0);
    segment.rate * window_rate
}

/// Segment history as JSON, for `GET /api/pricing`.
pub fn table_view(table: &RateTable) -> serde_json::Value {
    let mut models: Vec<&String> = table.keys().collect();
    models.sort();
    serde_json::json!(models
        .into_iter()
        .map(|model| {
            let segments = table
                .get(model)
                .map(|segments| {
                    segments
                        .iter()
                        .map(|s| {
                            serde_json::json!({
                                "effective_from": s.effective_from,
                                "rate": s.rate,
                                "windows": s.windows.iter().map(|w| serde_json::json!({
                                    "start": format_clock(w.start_min),
                                    "end": format_clock(w.end_min),
                                    "rate": w.rate,
                                })).collect::<Vec<_>>(),
                            })
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            (model.clone(), segments)
        })
        .collect::<serde_json::Value>())
}

/// TTL-guarded refresh of the entitlement rates, driven from the background
/// data-refresh loop so costs stay right even if nobody opens the quota tab.
/// Returns the new "last attempt" stamp, or `None` to keep the previous one
/// (i.e. still within the TTL).
pub async fn refresh_if_due(fetcher: &QuotaFetcher, last: Option<Instant>) -> Option<Instant> {
    let ttl = std::env::var("DIM_ENTITLEMENT_TTL_SECS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(DEFAULT_TTL_SECS);
    if last.is_some_and(|at| at.elapsed() < Duration::from_secs(ttl)) {
        return None;
    }
    let payload = match fetcher.fetch_dimagent_entitlement().await {
        Ok(payload) => payload,
        Err(e) => {
            tracing::warn!("Dim entitlement rates unavailable: {e}");
            return Some(Instant::now());
        }
    };
    let observed = match parse_payload(&payload) {
        Ok(observed) => observed,
        Err(e) => {
            tracing::warn!("Dim entitlement payload rejected: {e}");
            return Some(Instant::now());
        }
    };
    match observe(&observed, Utc::now()) {
        Some(table) => crate::pricing::set_dim_entitlement_rates(table),
        None => tracing::debug!("Dim entitlement rates unchanged"),
    }
    Some(Instant::now())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    const SAMPLE: &str = r#"{"oauth_only": true, "model_access": [
        {"model": "deepseek-v4-flash", "route_pool": "open beta"},
        {"rate": 0.7, "model": "glm-5.3", "route_pool": "open beta",
         "rate_windows": [{"end": "08:00", "name": "夜间优惠", "rate": 0.5, "start": "20:00"}]},
        {"rate": 0.65, "model": "deepseek-v4.1-flash", "route_pool": "open beta"}
    ]}"#;

    #[test]
    fn parses_platform_payload() {
        let observed = parse_payload(SAMPLE).unwrap();
        assert_eq!(observed.len(), 3);
        let v41 = observed
            .iter()
            .find(|o| o.model == "deepseek-v4.1-flash")
            .unwrap();
        assert_eq!(v41.rate, 0.65);
        assert!(v41.windows.is_empty());
        let glm = observed.iter().find(|o| o.model == "glm-5.3").unwrap();
        assert_eq!(glm.windows.len(), 1);
        assert_eq!(glm.windows[0].start, "20:00");
        // Unrated models come back as 1.0 so a revoked discount is recorded.
        let flash = observed.iter().find(|o| o.model == "deepseek-v4-flash").unwrap();
        assert_eq!(flash.rate, 1.0);
    }

    #[test]
    fn rates_apply_forward_only_and_night_window_composes() {
        let path = temp_state_path("forward");
        let observed = parse_payload(SAMPLE).unwrap();

        // First sight is 2026-09-20: earlier records must keep list price.
        let table = apply_observation(&path, &observed, at("2026-09-20T04:00:00Z")).unwrap();
        assert_eq!(
            rate_for(&table, "deepseek-v4.1-flash", "2026-09-17T03:00:00Z"),
            1.0
        );
        assert_eq!(
            rate_for(&table, "deepseek-v4.1-flash", "2026-09-20T05:00:00Z"),
            0.65
        );
        // glm-5.3: 0.7 base, and 0.7 × 0.5 inside 20:00–08:00 CST.
        assert_eq!(rate_for(&table, "glm-5.3", "2026-09-21T03:00:00Z"), 0.7);
        assert_eq!(
            rate_for(&table, "glm-5.3", "2026-09-21T13:00:00Z"),
            0.35,
            "21:00 CST falls in the night window"
        );
        assert_eq!(rate_for(&table, "unlisted-model", "2026-09-21T13:00:00Z"), 1.0);

        // Re-observing identical rates changes nothing.
        assert!(apply_observation(&path, &observed, at("2026-09-21T04:00:00Z")).is_none());

        // A revoked discount becomes a new 1.0 segment instead of vanishing.
        let revoked = vec![ObservedRate {
            model: "deepseek-v4.1-flash".into(),
            rate: 1.0,
            windows: vec![],
        }];
        let table = apply_observation(&path, &revoked, at("2026-09-25T04:00:00Z")).unwrap();
        assert_eq!(
            rate_for(&table, "deepseek-v4.1-flash", "2026-09-24T04:00:00Z"),
            0.65
        );
        assert_eq!(
            rate_for(&table, "deepseek-v4.1-flash", "2026-09-26T04:00:00Z"),
            1.0
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn mixed_case_model_names_resolve() {
        let path = temp_state_path("case");
        let observed = parse_payload(SAMPLE).unwrap();
        let table = apply_observation(&path, &observed, at("2026-09-20T04:00:00Z")).unwrap();
        assert_eq!(
            rate_for(&table, "DeepSeek-V4.1-Flash", "2026-09-21T04:00:00Z"),
            0.65
        );
        let _ = std::fs::remove_file(&path);
    }

    fn temp_state_path(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "token-stats-dim-rates-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::create_dir_all(&dir);
        dir.join("dim-entitlement.json")
    }
}
