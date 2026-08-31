//! CodeBuddy source: `~/.codebuddy/projects/**/*.jsonl`.
//!
//! CodeBuddy stores request usage under `providerData.rawUsage`. The raw
//! `credit` value is kept in `TokenRecord.cost`; pricing converts it to CNY
//! using the configured CodeBuddy credit rate.

use super::DataSource;
use crate::models::TokenRecord;
use chrono::{DateTime, Utc};
use serde_json::Value;
use std::collections::HashSet;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

#[derive(Default)]
pub struct CodeBuddySource;

impl DataSource for CodeBuddySource {
    fn name(&self) -> &'static str {
        "codebuddy"
    }

    fn load(&self) -> Vec<TokenRecord> {
        let base = Self::projects_path();
        tracing::info!("Loading CodeBuddy data from: {:?}", base);
        let records = Self::parse(&base);
        tracing::info!("Loaded {} codebuddy records", records.len());
        records
    }

    fn load_incremental(&self) -> Vec<TokenRecord> {
        let base = Self::projects_path();
        let files = Self::jsonl_files(&base);
        let changed = self.changed_data_files();
        let records = if changed.is_empty() {
            Vec::new()
        } else {
            Self::parse_files(&files, &changed)
        };
        self.mark_files_parsed(&files);
        records
    }

    fn data_files(&self) -> Vec<PathBuf> {
        Self::jsonl_files(&Self::projects_path())
    }

    fn is_available(&self) -> bool {
        Self::projects_path().exists()
    }
}

impl CodeBuddySource {
    fn projects_path() -> PathBuf {
        std::env::var("CODEBUDDY_PROJECTS_PATH")
            .ok()
            .filter(|p| !p.trim().is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| super::home_dir().join(".codebuddy").join("projects"))
    }

    fn jsonl_files(base_path: &Path) -> Vec<PathBuf> {
        if !base_path.exists() {
            return Vec::new();
        }
        match super::walkdir(base_path) {
            Ok(entries) => entries
                .into_iter()
                .filter(|p| p.extension().and_then(|ext| ext.to_str()) == Some("jsonl"))
                .collect(),
            Err(_) => Vec::new(),
        }
    }

    fn parse(base_path: &Path) -> Vec<TokenRecord> {
        Self::parse_files(&Self::jsonl_files(base_path), &[])
    }

    fn parse_files(paths: &[PathBuf], subset: &[PathBuf]) -> Vec<TokenRecord> {
        let mut records = Vec::new();
        let mut seen_ids = HashSet::new();

        for path in paths {
            if !subset.is_empty() && !subset.contains(path) {
                continue;
            }

            let file = match File::open(path) {
                Ok(file) => file,
                Err(_) => continue,
            };

            for line in BufReader::new(file).lines().map_while(Result::ok) {
                if line.trim().is_empty() {
                    continue;
                }
                let Ok(obj) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };

                let event_id = obj.get("id").and_then(Value::as_str).unwrap_or("");
                if !event_id.is_empty() && !seen_ids.insert(event_id.to_string()) {
                    continue;
                }

                let provider_data = match obj.get("providerData").and_then(Value::as_object) {
                    Some(data) => data,
                    None => continue,
                };
                let raw_usage = match provider_data.get("rawUsage").and_then(Value::as_object) {
                    Some(usage) => usage,
                    None => continue,
                };

                let credit = match raw_usage.get("credit").and_then(as_f64) {
                    Some(value) if value.is_finite() && value >= 0.0 => value,
                    _ => continue,
                };

                let (date, time) = match parse_timestamp(obj.get("timestamp")) {
                    Some(value) => value,
                    None => continue,
                };

                let prompt_tokens = first_i64(raw_usage, &["prompt_tokens", "input_tokens"])
                    .or_else(|| {
                        provider_data
                            .get("usage")
                            .and_then(Value::as_object)
                            .and_then(|usage| first_i64(usage, &["inputTokens", "input_tokens"]))
                    })
                    .unwrap_or(0)
                    .max(0);
                let output_tokens = first_i64(raw_usage, &["completion_tokens", "output_tokens"])
                    .or_else(|| {
                        provider_data
                            .get("usage")
                            .and_then(Value::as_object)
                            .and_then(|usage| first_i64(usage, &["outputTokens", "output_tokens"]))
                    })
                    .unwrap_or(0)
                    .max(0);

                let cache_read_tokens = first_positive([
                    first_i64(raw_usage, &["prompt_cache_hit_tokens"]),
                    first_i64(raw_usage, &["cache_read_input_tokens"]),
                    first_i64(raw_usage, &["cache_read_tokens"]),
                    detail_cached_tokens(raw_usage.get("prompt_tokens_details")),
                    detail_cached_tokens(raw_usage.get("input_tokens_details")),
                    first_i64(raw_usage, &["cached_tokens"]),
                ]);

                let direct_cache_write = first_positive([
                    first_i64(raw_usage, &["prompt_cache_write_tokens"]),
                    first_i64(raw_usage, &["cache_creation_input_tokens"]),
                    first_i64(raw_usage, &["cache_write_input_tokens"]),
                    first_i64(raw_usage, &["cache_write_tokens"]),
                    detail_cached_tokens(raw_usage.get("prompt_cache_write_details")),
                ]);
                let cache_write_tokens = if direct_cache_write > 0 {
                    direct_cache_write
                } else {
                    // `prompt_cache_miss_tokens` is uncached input, not cache
                    // write. It lets us derive the write portion when the
                    // gateway omits its explicit cache-write field.
                    let cache_miss = first_positive([
                        first_i64(raw_usage, &["prompt_cache_miss_tokens"]),
                        first_i64(raw_usage, &["prompt_cache_miss"]),
                    ]);
                    (prompt_tokens - cache_read_tokens - cache_miss).max(0)
                };
                let input_tokens = (prompt_tokens - cache_read_tokens - cache_write_tokens).max(0);
                let total_tokens =
                    input_tokens + output_tokens + cache_read_tokens + cache_write_tokens;

                if total_tokens == 0 && credit == 0.0 {
                    continue;
                }

                let model = provider_data
                    .get("model")
                    .and_then(Value::as_str)
                    .filter(|model| !model.is_empty())
                    .or_else(|| {
                        provider_data
                            .get("requestModelId")
                            .and_then(Value::as_str)
                            .filter(|model| !model.is_empty())
                    })
                    .or_else(|| {
                        provider_data
                            .get("requestModelName")
                            .and_then(Value::as_str)
                            .filter(|model| !model.is_empty())
                    })
                    .unwrap_or("unknown")
                    .to_string();

                records.push(TokenRecord {
                    date,
                    time,
                    api_key_prefix: "N/A".to_string(),
                    provider: "codebuddy".to_string(),
                    original_provider: None,
                    model,
                    source: "codebuddy".to_string(),
                    input_tokens,
                    output_tokens,
                    cache_read_tokens,
                    cache_write_tokens,
                    total_tokens,
                    cost: credit,
                    ttft_ms: None,
                    tps: None,
                });
            }
        }

        records
    }
}

fn as_f64(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_i64().map(|value| value as f64))
}

fn first_i64(object: &serde_json::Map<String, Value>, keys: &[&str]) -> Option<i64> {
    keys.iter().find_map(|key| {
        object.get(*key).and_then(|value| {
            value
                .as_i64()
                .or_else(|| value.as_f64().map(|value| value.round() as i64))
        })
    })
}

fn first_positive<const N: usize>(values: [Option<i64>; N]) -> i64 {
    values
        .into_iter()
        .flatten()
        .find(|value| *value > 0)
        .unwrap_or(0)
}

fn detail_cached_tokens(value: Option<&Value>) -> Option<i64> {
    match value {
        Some(Value::Object(object)) => first_i64(object, &["cached_tokens", "cache_read_tokens"]),
        Some(Value::Array(items)) => items.iter().find_map(|item| {
            item.as_object()
                .and_then(|object| first_i64(object, &["cached_tokens", "cache_read_tokens"]))
        }),
        _ => None,
    }
}

fn parse_timestamp(value: Option<&Value>) -> Option<(String, String)> {
    let timestamp = value?;
    let datetime = if let Some(milliseconds) = timestamp.as_i64() {
        DateTime::<Utc>::from_timestamp_millis(milliseconds)
    } else if let Some(milliseconds) = timestamp.as_f64() {
        DateTime::<Utc>::from_timestamp_millis(milliseconds.round() as i64)
    } else if let Some(text) = timestamp.as_str() {
        if let Ok(datetime) = DateTime::parse_from_rfc3339(text) {
            Some(datetime.with_timezone(&Utc))
        } else {
            text.parse::<i64>()
                .ok()
                .and_then(DateTime::<Utc>::from_timestamp_millis)
        }
    } else {
        None
    }?;

    Some((
        datetime.format("%Y-%m-%d").to_string(),
        datetime.to_rfc3339(),
    ))
}

#[cfg(test)]
mod tests {
    use super::CodeBuddySource;
    use crate::sources::DataSource;
    use std::fs;
    use tempfile::tempdir;

    fn event(id: &str, timestamp: i64, raw_usage: &str, model: &str) -> String {
        format!(
            r#"{{"type":"function_call","id":"{id}","timestamp":{timestamp},"providerData":{{"model":"{model}","requestModelId":"fallback-model","rawUsage":{raw_usage}}}}}"#
        )
    }

    #[test]
    fn parses_credit_and_normalizes_cache_accounting() {
        let dir = tempdir().unwrap();
        let raw = r#"{"prompt_tokens":114667,"completion_tokens":189,"prompt_cache_hit_tokens":80532,"prompt_cache_miss_tokens":2,"prompt_cache_write_tokens":34133,"credit":1.04}"#;
        fs::write(
            dir.path().join("session.jsonl"),
            event("request-1", 1787978653879, raw, "gpt-5.6-luna"),
        )
        .unwrap();

        let records = CodeBuddySource::parse(dir.path());
        assert_eq!(records.len(), 1);
        let record = &records[0];
        assert_eq!(record.source, "codebuddy");
        assert_eq!(record.provider, "codebuddy");
        assert_eq!(record.model, "gpt-5.6-luna");
        assert_eq!(record.date, "2026-08-29");
        assert_eq!(record.input_tokens, 2);
        assert_eq!(record.output_tokens, 189);
        assert_eq!(record.cache_read_tokens, 80532);
        assert_eq!(record.cache_write_tokens, 34133);
        assert_eq!(record.total_tokens, 114856);
        assert!((record.cost - 1.04).abs() < f64::EPSILON);
    }

    #[test]
    fn derives_cache_write_without_mistaking_cache_miss_for_write() {
        let dir = tempdir().unwrap();
        let raw = r#"{"prompt_tokens":100,"completion_tokens":10,"prompt_cache_hit_tokens":60,"prompt_cache_miss_tokens":5,"credit":0.1}"#;
        fs::write(
            dir.path().join("session.jsonl"),
            event("request-2", 1787978653879, raw, "gpt-5.6-luna"),
        )
        .unwrap();

        let records = CodeBuddySource::parse(dir.path());
        assert_eq!(records[0].cache_write_tokens, 35);
        assert_eq!(records[0].input_tokens, 5);
        assert_eq!(records[0].total_tokens, 110);
    }

    #[test]
    fn falls_back_to_request_model_and_deduplicates_ids() {
        let dir = tempdir().unwrap();
        let raw = r#"{"prompt_tokens":10,"completion_tokens":5,"credit":0.01}"#;
        let first = event("duplicate", 1787978653879, raw, "");
        let second = event("duplicate", 1787978653879, raw, "other-model");
        fs::write(
            dir.path().join("session.jsonl"),
            format!("{first}\n{second}\n"),
        )
        .unwrap();

        let records = CodeBuddySource::parse(dir.path());
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].model, "fallback-model");
    }

    #[test]
    fn skips_invalid_timestamp_and_empty_zero_credit_event() {
        let dir = tempdir().unwrap();
        let invalid = r#"{"id":"bad","timestamp":"not-a-time","providerData":{"rawUsage":{"prompt_tokens":1,"credit":1}}}"#;
        let empty =
            r#"{"id":"empty","timestamp":1787978653879,"providerData":{"rawUsage":{"credit":0}}}"#;
        fs::write(
            dir.path().join("session.jsonl"),
            format!("{invalid}\n{empty}\n"),
        )
        .unwrap();

        assert!(CodeBuddySource::parse(dir.path()).is_empty());
    }

    #[test]
    fn keeps_zero_credit_request_with_tokens() {
        let dir = tempdir().unwrap();
        let raw = r#"{"prompt_tokens":10,"completion_tokens":5,"credit":0}"#;
        fs::write(
            dir.path().join("session.jsonl"),
            event("free-request", 1787978653879, raw, "gpt-5.6-luna"),
        )
        .unwrap();

        let records = CodeBuddySource::parse(dir.path());
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].cost, 0.0);
    }

    #[test]
    fn loads_via_env_override() {
        let dir = tempdir().unwrap();
        let raw = r#"{"prompt_tokens":10,"completion_tokens":5,"credit":0.01}"#;
        fs::write(
            dir.path().join("session.jsonl"),
            event("env-request", 1787978653879, raw, "gpt-5.6-luna"),
        )
        .unwrap();

        temp_env::with_var(
            "CODEBUDDY_PROJECTS_PATH",
            Some(dir.path().to_str().unwrap()),
            || assert_eq!(CodeBuddySource.load().len(), 1),
        );
    }
}
