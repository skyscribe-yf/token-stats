use super::DataSource;
use crate::models::TokenRecord;
use chrono::{TimeZone, Utc};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

/// Kimi CLI session source: reads `~/.kimi/sessions/*/wire.jsonl`.
#[derive(Default)]
pub struct KimiCliSource;

impl DataSource for KimiCliSource {
    fn name(&self) -> &'static str {
        "kimi-cli"
    }

    fn load(&self) -> Vec<TokenRecord> {
        let base = Self::sessions_path();
        tracing::info!("Loading Kimi CLI data from: {:?}", base);
        let records = Self::parse(&base);
        tracing::info!("Loaded {} kimi-cli records", records.len());
        records
    }
}

impl KimiCliSource {
    fn sessions_path() -> PathBuf {
        let override_path = std::env::var("KIMI_SESSIONS_PATH").ok();
        override_path
            .map(PathBuf::from)
            .unwrap_or_else(|| super::home_dir().join(".kimi").join("sessions"))
    }

    fn parse(base_path: &std::path::Path) -> Vec<TokenRecord> {
        if !base_path.exists() {
            tracing::warn!("Kimi sessions dir not found at {:?}, skipping", base_path);
            return Vec::new();
        }

        let mut records = Vec::new();

        let entries = match super::walkdir(base_path) {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!("Failed to walk Kimi sessions dir: {}", e);
                return records;
            }
        };

        for wire_path in entries {
            if !wire_path.to_string_lossy().ends_with("wire.jsonl") {
                continue;
            }

            let session_dir = wire_path.parent().unwrap_or(base_path);
            let model = Self::read_session_model(session_dir);

            let file = match File::open(&wire_path) {
                Ok(f) => f,
                Err(_) => continue,
            };

            let reader = BufReader::new(file);
            for line in reader.lines().map_while(Result::ok) {
                if line.trim().is_empty() {
                    continue;
                }
                if let Ok(msg) = serde_json::from_str::<serde_json::Value>(&line) {
                    let message = msg.get("message");
                    if let Some(message) = message {
                        if message.get("type").and_then(|t| t.as_str()) == Some("StatusUpdate") {
                            let payload = message.get("payload");
                            if let Some(payload) = payload {
                                let token_usage = payload.get("token_usage");
                                if let Some(usage) = token_usage {
                                    let input_other = usage
                                        .get("input_other")
                                        .and_then(|v| v.as_i64())
                                        .unwrap_or(0);
                                    let output =
                                        usage.get("output").and_then(|v| v.as_i64()).unwrap_or(0);
                                    let cache_read = usage
                                        .get("input_cache_read")
                                        .and_then(|v| v.as_i64())
                                        .unwrap_or(0);
                                    let cache_creation = usage
                                        .get("input_cache_creation")
                                        .and_then(|v| v.as_i64())
                                        .unwrap_or(0);

                                    let timestamp = msg
                                        .get("timestamp")
                                        .and_then(|v| v.as_f64())
                                        .unwrap_or(0.0);
                                    let secs = timestamp as i64;
                                    let dt = Utc.timestamp_opt(secs, 0).single();
                                    let (date, time) = match dt {
                                        Some(dt) => {
                                            (dt.format("%Y-%m-%d").to_string(), dt.to_rfc3339())
                                        }
                                        None => ("unknown".to_string(), "unknown".to_string()),
                                    };

                                    let total = input_other + output + cache_read + cache_creation;

                                    records.push(TokenRecord {
                                        date,
                                        time,
                                        api_key_prefix: "N/A".to_string(),
                                        provider: "kimi".to_string(),
                                        model: model.clone(),
                                        source: "kimi-cli".to_string(),
                                        input_tokens: input_other,
                                        output_tokens: output,
                                        cache_read_tokens: cache_read,
                                        cache_write_tokens: cache_creation,
                                        total_tokens: total,
                                        cost: 0.0,
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }

        records
    }

    /// Read model name from a Kimi session's state.json.
    fn read_session_model(session_dir: &Path) -> String {
        let state_path = session_dir.join("state.json");
        if let Ok(file) = File::open(&state_path) {
            let reader = BufReader::new(file);
            if let Ok(state) = serde_json::from_reader::<_, serde_json::Value>(reader) {
                if let Some(model) = state.get("model").and_then(|m| m.as_str()) {
                    return model.to_string();
                }
                if let Some(config) = state.get("config") {
                    if let Some(model) = config.get("default_model").and_then(|m| m.as_str()) {
                        return model.to_string();
                    }
                }
            }
        }
        "kimi-for-coding".to_string()
    }
}
