use super::DataSource;
use crate::models::TokenRecord;
use std::collections::HashSet;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;

/// Claude Code project source: reads `~/.claude/projects/*/*.jsonl`.
#[derive(Default)]
pub struct ClaudeCodeSource;

impl DataSource for ClaudeCodeSource {
    fn name(&self) -> &'static str {
        "claude-code"
    }

    fn load(&self) -> Vec<TokenRecord> {
        let base = Self::projects_path();
        tracing::info!("Loading Claude Code data from: {:?}", base);
        let records = Self::parse(&base);
        tracing::info!("Loaded {} claude-code records", records.len());
        records
    }
}

impl ClaudeCodeSource {
    fn projects_path() -> PathBuf {
        super::home_dir().join(".claude").join("projects")
    }

    fn parse(base_path: &std::path::Path) -> Vec<TokenRecord> {
        if !base_path.exists() {
            tracing::warn!(
                "Claude Code projects dir not found at {:?}, skipping",
                base_path
            );
            return Vec::new();
        }

        let mut records = Vec::new();
        let mut seen: HashSet<(String, String)> = HashSet::new();

        let entries = match super::walkdir(base_path) {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!("Failed to walk Claude Code projects dir: {}", e);
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
                    if obj.get("type").and_then(|t| t.as_str()) != Some("assistant") {
                        continue;
                    }
                    let session_id = obj
                        .get("sessionId")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let msg = obj.get("message");
                    if msg.is_none() {
                        continue;
                    }
                    let msg = msg.unwrap();
                    let msg_id = msg
                        .get("id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();

                    let key = (session_id, msg_id);
                    if seen.contains(&key) {
                        continue;
                    }

                    let usage = msg.get("usage");
                    if usage.is_none() {
                        continue;
                    }
                    let usage = usage.unwrap();
                    let input_tokens = usage
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
                    if input_tokens == 0
                        && output_tokens == 0
                        && cache_read_tokens == 0
                        && cache_write_tokens == 0
                    {
                        continue;
                    }

                    seen.insert(key);

                    let model = msg
                        .get("model")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown")
                        .to_string();
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
                        source: "claude-code".to_string(),
                        input_tokens,
                        output_tokens,
                        cache_read_tokens,
                        cache_write_tokens,
                        total_tokens: input_tokens
                            + output_tokens
                            + cache_read_tokens
                            + cache_write_tokens,
                        cost: 0.0,
                    });
                }
            }
        }

        records
    }
}
