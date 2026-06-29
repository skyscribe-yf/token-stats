use super::DataSource;
use crate::models::TokenRecord;
use std::collections::HashSet;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;

/// Qoder CLI CN source: reads `~/.qoder-cn/logs/sessions/*/segments/*.jsonl`.
///
/// Qoder CN (qoderclicn) stores session logs under `~/.qoder-cn/logs/sessions/`
/// with segment JSONL files containing structured log events. Each
/// `type: "model.response.completed"` event contains token usage data following
/// the OpenAI convention (input_tokens INCLUDES cache_read_input_tokens), so we
/// normalize to Anthropic convention by subtracting cache_read from input.
#[derive(Default)]
pub struct QoderCnSource;

impl DataSource for QoderCnSource {
    fn name(&self) -> &'static str {
        "qoder-cn"
    }

    fn load(&self) -> Vec<TokenRecord> {
        let base = Self::sessions_path();
        tracing::info!("Loading Qoder CN data from: {:?}", base);
        let records = Self::parse(&base);
        tracing::info!("Loaded {} qoder-cn records", records.len());
        records
    }

    fn is_available(&self) -> bool {
        Self::sessions_path().exists()
    }
}

impl QoderCnSource {
    fn sessions_path() -> PathBuf {
        let override_path = std::env::var("QODER_CN_SESSIONS_PATH").ok();
        override_path.map_or_else(
            || {
                super::home_dir()
                    .join(".qoder-cn")
                    .join("logs")
                    .join("sessions")
            },
            PathBuf::from,
        )
    }

    fn parse(base_path: &std::path::Path) -> Vec<TokenRecord> {
        if !base_path.exists() {
            tracing::warn!(
                "Qoder CN sessions dir not found at {:?}, skipping",
                base_path
            );
            return Vec::new();
        }

        let mut records = Vec::new();
        let mut seen: HashSet<String> = HashSet::new(); // dedup by request_id

        let entries = match super::walkdir(base_path) {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!("Failed to walk Qoder CN sessions dir: {}", e);
                return records;
            }
        };

        for path in entries {
            if !path.to_string_lossy().ends_with(".jsonl") {
                continue;
            }

            let file = match File::open(&path) {
                Ok(f) => f,
                Err(_) => continue,
            };

            let reader = BufReader::new(file);
            for line in reader.lines().map_while(Result::ok) {
                if line.trim().is_empty() {
                    continue;
                }
                if let Ok(obj) = serde_json::from_str::<serde_json::Value>(&line) {
                    // Only process model.response.completed events with usage data
                    if obj.get("type").and_then(|t| t.as_str()) != Some("model.response.completed")
                    {
                        continue;
                    }

                    let data = match obj.get("data") {
                        Some(d) => d,
                        None => continue,
                    };

                    let request_id = obj
                        .get("request_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();

                    // Dedup by request_id
                    if request_id.is_empty() || seen.contains(&request_id) {
                        continue;
                    }

                    let raw_input = data
                        .get("input_tokens")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0);
                    let output_tokens = data
                        .get("output_tokens")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0);
                    let cache_read_tokens = data
                        .get("cache_read_input_tokens")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0);
                    let cache_write_tokens = data
                        .get("cache_creation_input_tokens")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0);

                    // Filter out records with zero usage
                    if raw_input == 0
                        && output_tokens == 0
                        && cache_read_tokens == 0
                        && cache_write_tokens == 0
                    {
                        continue;
                    }

                    seen.insert(request_id);

                    // Qoder CN uses OpenAI convention: input_tokens INCLUDES cache_read.
                    // Normalize to Anthropic convention (input = non-cached only).
                    let input_tokens = (raw_input - cache_read_tokens).max(0);

                    let model = data
                        .get("model")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown")
                        .to_string();

                    // Qoder CN uses "auto" as a model alias. Resolve provider from model.
                    let provider = super::resolve_provider_from_model(&model);

                    // Parse timestamp from "ts" field (format: "2026-06-03T20:47:20.194+08:00")
                    let ts_str = obj.get("ts").and_then(|t| t.as_str()).unwrap_or("");
                    let (date, time) = Self::parse_timestamp(ts_str);

                    records.push(TokenRecord {
                        date,
                        time,
                        api_key_prefix: "N/A".to_string(),
                        provider,
                        original_provider: None,
                        model,
                        source: "qoder-cn".to_string(),
                        input_tokens,
                        output_tokens,
                        cache_read_tokens,
                        cache_write_tokens,
                        total_tokens: input_tokens
                            + output_tokens
                            + cache_read_tokens
                            + cache_write_tokens,
                        cost: 0.0,
                        ttft_ms: None,
                        tps: None,
                    });
                }
            }
        }

        records
    }

    /// Parse timestamp in format "2026-06-03T20:47:20.194+08:00" (local time with offset).
    fn parse_timestamp(ts: &str) -> (String, String) {
        // Try parsing as RFC3339 with timezone offset
        match chrono::DateTime::parse_from_rfc3339(ts) {
            Ok(dt) => {
                let utc = dt.with_timezone(&chrono::Utc);
                (utc.format("%Y-%m-%d").to_string(), utc.to_rfc3339())
            }
            Err(_) => ("unknown".to_string(), "unknown".to_string()),
        }
    }
}
