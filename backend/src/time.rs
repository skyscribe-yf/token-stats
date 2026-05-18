//! Time-bound parsing and timezone utilities.
//!
//! `TimeBound` represents a query filter boundary — either an exact date-time
//! (from a datetime-local input) or a calendar date.  Used by both the
//! aggregator (business-logic layer) and the route handlers (web layer).

use chrono::{FixedOffset, NaiveDate};

/// A boundary for time-range filtering.
#[derive(Debug, Clone)]
pub enum TimeBound {
    DateTime(chrono::NaiveDateTime),
    Date(NaiveDate),
}

/// Parse a user-supplied time-bound string.
///
/// Accepted formats (tried in order):
/// - `YYYY-MM-DDTHH:MM:SS`       — full ISO with seconds
/// - `YYYY-MM-DDTHH:MM`          — HTML datetime-local default
/// - `YYYY-MM-DD`                — date only
pub fn parse_time_bound(s: &str) -> Option<TimeBound> {
    // Full ISO with optional fractional seconds
    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S") {
        return Some(TimeBound::DateTime(dt));
    }
    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f") {
        return Some(TimeBound::DateTime(dt));
    }
    // HTML datetime-local (no seconds)
    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M") {
        return Some(TimeBound::DateTime(dt));
    }
    // Date only
    if let Ok(d) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return Some(TimeBound::Date(d));
    }
    None
}

/// Convert a tz_offset (minutes from UTC) to a chrono `FixedOffset`.
///
/// Frontend sends e.g. `480` for UTC+8, `-300` for UTC-5.
pub fn tz_offset_to_fixed(offset_minutes: i32) -> FixedOffset {
    FixedOffset::east_opt(offset_minutes * 60).unwrap_or_else(|| FixedOffset::east_opt(0).unwrap())
}
