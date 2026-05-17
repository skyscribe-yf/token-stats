use super::DataSource;
use crate::models::TokenRecord;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;

/// Codex session source: reads `~/.codex/sessions/*/rollout-*.jsonl`.
#[derive(Default)]
pub struct CodexSource;

impl DataSource for CodexSource {
    fn name(&self) -> &'static str {
        "codex"
    }

    fn load(&self) -> Vec<TokenRecord> {
        let base = Self::sessions_path();
        tracing::info!("Loading Codex data from: {:?}", base);
        let records = Self::parse(&base);
        tracing::info!("Loaded {} codex records", records.len());
        records
    }
}

impl CodexSource {
    fn sessions_path() -> PathBuf {
        super::home_dir().join(".codex").join("sessions")
    }

    fn parse(base_path: &std::path::Path) -> Vec<TokenRecord> {
        if !base_path.exists() {
            tracing::warn!("Codex sessions dir not found at {:?}, skipping", base_path);
            return Vec::new();
        }

        let mut records = Vec::new();

        let entries = match super::walkdir(base_path) {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!("Failed to walk Codex sessions dir: {}", e);
                return records;
            }
        };

        for path in entries {
            if !path.to_string_lossy().ends_with(".jsonl") {
                continue;
            }
            if !path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .starts_with("rollout-")
            {
                continue;
            }

            let file = match File::open(&path) {
                Ok(f) => f,
                Err(_) => continue,
            };

            // First pass: find model from turn_context
            let mut session_model = "gpt-5.5".to_string();
            {
                let reader = BufReader::new(&file);
                for line in reader.lines() {
                    if let Ok(line) = line {
                        if line.trim().is_empty() {
                            continue;
                        }
                        if let Ok(obj) = serde_json::from_str::<serde_json::Value>(&line) {
                            if obj.get("type").and_then(|t| t.as_str()) == Some("turn_context") {
                                if let Some(model) = obj
                                    .get("payload")
                                    .and_then(|p| p.get("model"))
                                    .and_then(|m| m.as_str())
                                {
                                    session_model = model.to_string();
                                    break;
                                }
                            }
                        }
                    }
                }
            }

            // Second pass: collect token_count events
            let file = match File::open(&path) {
                Ok(f) => f,
                Err(_) => continue,
            };
            let reader = BufReader::new(file);
            for line in reader.lines() {
                if let Ok(line) = line {
                    if line.trim().is_empty() {
                        continue;
                    }
                    if let Ok(obj) = serde_json::from_str::<serde_json::Value>(&line) {
                        if obj.get("type").and_then(|t| t.as_str()) != Some("event_msg") {
                            continue;
                        }
                        let payload = obj.get("payload");
                        if payload.is_none() {
                            continue;
                        }
                        let payload = payload.unwrap();
                        if payload.get("type").and_then(|t| t.as_str())
                            != Some("token_count")
                        {
                            continue;
                        }
                        let info = payload.get("info");
                        if info.is_none() || info.unwrap().is_null() {
                            continue;
                        }
                        let last_usage = info.unwrap().get("last_token_usage");
                        if last_usage.is_none() {
                            continue;
                        }
                        let usage = last_usage.unwrap();

                        let input_tokens =
                            usage.get("input_tokens").and_then(|v| v.as_i64()).unwrap_or(0);
                        let cached_input_tokens = usage
                            .get("cached_input_tokens")
                            .and_then(|v| v.as_i64())
                            .unwrap_or(0);
                        let output_tokens =
                            usage.get("output_tokens").and_then(|v| v.as_i64()).unwrap_or(0);

                        // OpenAI convention: input_tokens includes cache; normalize
                        let effective_input = (input_tokens - cached_input_tokens).max(0);

                        let ts_str = obj
                            .get("timestamp")
                            .and_then(|t| t.as_str())
                            .unwrap_or("");
                        let (date, time) = super::parse_iso_timestamp(ts_str);

                        records.push(TokenRecord {
                            date,
                            time,
                            api_key_prefix: "N/A".to_string(),
                            provider: "openai".to_string(),
                            model: session_model.clone(),
                            source: "codex".to_string(),
                            input_tokens: effective_input,
                            output_tokens,
                            cache_read_tokens: cached_input_tokens,
                            cache_write_tokens: 0,
                            total_tokens: effective_input + output_tokens + cached_input_tokens,
                            cost: 0.0,
                        });
                    }
                }
            }
        }

        records
    }
}
