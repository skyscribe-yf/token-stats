use super::DataSource;
use crate::models::TokenRecord;
use std::collections::HashSet;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;

/// Qoder CLI source: reads `~/.qoder/projects/*/*.jsonl` and subagent JSONL files.
///
/// Qoder stores session transcripts under `~/.qoder/projects/<project-id>/` with
/// one JSONL per session (both main sessions and subagents). Each `type: "assistant"`
/// record contains a `usage` object with token counts following the OpenAI
/// convention (input_tokens INCLUDES cache_read_input_tokens), so we normalize
/// to Anthropic convention by subtracting cache_read from input.
#[derive(Default)]
pub struct QoderSource;

impl DataSource for QoderSource {
    fn name(&self) -> &'static str {
        "qoder"
    }

    fn load(&self) -> Vec<TokenRecord> {
        let base = Self::projects_path();
        tracing::info!("Loading Qoder data from: {:?}", base);
        let records = Self::parse(&base);
        tracing::info!("Loaded {} qoder records", records.len());
        records
    }

    fn is_available(&self) -> bool {
        Self::projects_path().exists()
    }
}

impl QoderSource {
    fn projects_path() -> PathBuf {
        let override_path = std::env::var("QODER_PROJECTS_PATH").ok();
        override_path.map_or_else(
            || super::home_dir().join(".qoder").join("projects"),
            PathBuf::from,
        )
    }

    fn parse(base_path: &std::path::Path) -> Vec<TokenRecord> {
        if !base_path.exists() {
            tracing::warn!("Qoder projects dir not found at {:?}, skipping", base_path);
            return Vec::new();
        }

        let mut records = Vec::new();
        let mut seen: HashSet<String> = HashSet::new(); // dedup by uuid

        let entries = match super::walkdir(base_path) {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!("Failed to walk Qoder projects dir: {}", e);
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
                    // Only process assistant messages with usage data
                    if obj.get("type").and_then(|t| t.as_str()) != Some("assistant") {
                        continue;
                    }

                    let uuid = obj
                        .get("uuid")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();

                    // Dedup by uuid
                    if uuid.is_empty() || seen.contains(&uuid) {
                        continue;
                    }

                    let msg = match obj.get("message") {
                        Some(m) => m,
                        None => continue,
                    };

                    let usage = match msg.get("usage") {
                        Some(u) => u,
                        None => continue,
                    };

                    let raw_input = usage
                        .get("input_tokens")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0);
                    let output_tokens = usage
                        .get("output_tokens")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0);
                    let cache_read_tokens = usage
                        .get("cache_read_input_tokens")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0);
                    let cache_write_tokens = usage
                        .get("cache_creation_input_tokens")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0);

                    // Filter out intermediate streaming snapshots with zero usage
                    if raw_input == 0
                        && output_tokens == 0
                        && cache_read_tokens == 0
                        && cache_write_tokens == 0
                    {
                        continue;
                    }

                    seen.insert(uuid);

                    // Qoder uses OpenAI convention: input_tokens INCLUDES cache_read.
                    // Normalize to Anthropic convention (input = non-cached only).
                    let input_tokens = (raw_input - cache_read_tokens).max(0);

                    let model = msg
                        .get("model")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown")
                        .to_string();

                    // Qoder uses dynamic model aliases (e.g. "qmodel_latest", "efficient").
                    // Use the model name as-is for provider resolution.
                    let provider = super::resolve_provider_from_model(&model);

                    let ts_str = obj.get("timestamp").and_then(|t| t.as_str()).unwrap_or("");
                    let (date, time) = super::parse_iso_timestamp(ts_str);

                    records.push(TokenRecord {
                        date,
                        time,
                        api_key_prefix: "N/A".to_string(),
                        provider,
                        original_provider: None,
                        model,
                        source: "qoder".to_string(),
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
}
