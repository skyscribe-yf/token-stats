//! DeepSeek Harness (dsh) data source.
//!
//! dsh stores each session as a zstd-compressed JSONL event log at
//! `~/.dsh/sessions/<workspace>/session-<uuid>/session.jsonl.zstd`.
//! Token usage appears as `assistant/chunk` events with `chunk.type == "usage"`
//! (inputTokens/outputTokens/cacheReadTokens, Anthropic convention — input
//! excludes cache). The `finish` chunk of the same (turn, step) carries a
//! `replayState` with the provider and model; `request/header` provides a
//! session-level fallback.

use super::DataSource;
use crate::models::TokenRecord;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

pub struct DshSource;

pub(crate) fn dsh_sessions_path() -> PathBuf {
    std::env::var("DSH_SESSIONS_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| super::home_dir().join(".dsh/sessions"))
}

impl DataSource for DshSource {
    fn name(&self) -> &'static str {
        "dsh"
    }

    fn load(&self) -> Vec<TokenRecord> {
        let root = dsh_sessions_path();
        let Ok(files) = super::walkdir(&root) else {
            return Vec::new();
        };

        let mut records = Vec::new();
        for path in files {
            if path.file_name().and_then(|n| n.to_str()) != Some("session.jsonl.zstd") {
                continue;
            }
            match parse_session_file(&path) {
                Ok(mut recs) => records.append(&mut recs),
                Err(error) => {
                    tracing::warn!("Skipping unreadable dsh session {:?}: {error}", path);
                }
            }
        }
        records
    }

    /// Incremental: only re-parse session files whose (mtime, size) changed.
    fn load_incremental(&self) -> Vec<TokenRecord> {
        let root = dsh_sessions_path();
        let files = Self::session_files(&root);
        let changed = self.changed_data_files();
        if changed.is_empty() {
            return Vec::new();
        }
        let mut records = Vec::new();
        for path in &changed {
            if let Ok(mut recs) = parse_session_file(path) {
                records.append(&mut recs);
            }
        }
        self.mark_files_parsed(&files);
        records
    }

    fn data_files(&self) -> Vec<std::path::PathBuf> {
        Self::session_files(&dsh_sessions_path())
    }

    fn is_available(&self) -> bool {
        dsh_sessions_path().exists()
    }
}

impl DshSource {
    fn session_files(root: &std::path::Path) -> Vec<std::path::PathBuf> {
        match super::walkdir(root) {
            Ok(files) => files
                .into_iter()
                .filter(|p| p.file_name().and_then(|n| n.to_str()) == Some("session.jsonl.zstd"))
                .collect(),
            Err(_) => Vec::new(),
        }
    }
}

fn parse_session_file(path: &Path) -> Result<Vec<TokenRecord>, Box<dyn std::error::Error>> {
    let file = File::open(path)?;
    let mut decoded = Vec::new();
    zstd::stream::read::Decoder::new(file)?.read_to_end(&mut decoded)?;
    let text = String::from_utf8_lossy(&decoded);

    // Session-level fallback provider/model from request/header events.
    let mut default_provider: Option<String> = None;
    let mut default_model: Option<String> = None;
    // Per (turn, step): accumulated usage + provider/model from finish chunk.
    let mut steps: HashMap<(u64, u64), StepUsage> = HashMap::new();

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(event) = serde_json::from_str::<DshEvent>(line) else {
            continue;
        };
        let Some(data) = event.data else { continue };
        match event.event_type.as_str() {
            "request/header" => {
                if let Some(config) = data.header.and_then(|h| h.config) {
                    if default_provider.is_none() {
                        default_provider = config.provider;
                    }
                    if default_model.is_none() {
                        default_model = config.model;
                    }
                }
            }
            "assistant/chunk" => {
                let Some(chunk) = data.chunk else { continue };
                let Some((turn, step)) = data.turn.zip(data.step) else {
                    continue;
                };
                let entry = steps.entry((turn, step)).or_default();
                match chunk.chunk_type.as_str() {
                    "usage" => {
                        if let Some(usage) = chunk.usage {
                            entry.input_tokens += usage.input_tokens;
                            entry.output_tokens += usage.output_tokens;
                            entry.cache_read_tokens += usage.cache_read_tokens.unwrap_or(0);
                            entry.cache_write_tokens += usage.cache_write_tokens.unwrap_or(0);
                            if entry.time_ms.is_none() {
                                entry.time_ms = event.time;
                            }
                        }
                    }
                    "finish" => {
                        if let Some(rs) = chunk.replay_state {
                            if entry.provider.is_none() {
                                entry.provider = rs.provider;
                            }
                            if entry.model.is_none() {
                                entry.model = rs.model;
                            }
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    let session_id = path
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("dsh")
        .to_string();

    let mut records = Vec::new();
    for ((_turn, _step), usage) in steps {
        if usage.input_tokens == 0 && usage.output_tokens == 0 {
            continue;
        }
        let Some(time_ms) = usage.time_ms else {
            continue;
        };
        let Some(dt) = DateTime::from_timestamp_millis(time_ms) else {
            continue;
        };
        let utc: DateTime<Utc> = dt.with_timezone(&Utc);
        let provider = usage
            .provider
            .clone()
            .or_else(|| default_provider.clone())
            .unwrap_or_else(|| "unknown".to_string());
        let model = usage
            .model
            .clone()
            .or_else(|| default_model.clone())
            .unwrap_or_else(|| "unknown".to_string());
        let total_tokens = usage.input_tokens
            + usage.output_tokens
            + usage.cache_read_tokens
            + usage.cache_write_tokens;
        records.push(TokenRecord {
            date: utc.format("%Y-%m-%d").to_string(),
            time: utc.to_rfc3339(),
            api_key_prefix: session_id.clone(),
            provider,
            original_provider: None,
            model,
            source: "dsh".to_string(),
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            cache_read_tokens: usage.cache_read_tokens,
            cache_write_tokens: usage.cache_write_tokens,
            total_tokens,
            cost: 0.0,
            ttft_ms: None,
            tps: None,
        });
    }
    Ok(records)
}

#[derive(Default)]
struct StepUsage {
    input_tokens: i64,
    output_tokens: i64,
    cache_read_tokens: i64,
    cache_write_tokens: i64,
    time_ms: Option<i64>,
    provider: Option<String>,
    model: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DshEvent {
    #[serde(rename = "type")]
    event_type: String,
    time: Option<i64>,
    data: Option<DshEventData>,
}

#[derive(Debug, Deserialize)]
struct DshEventData {
    turn: Option<u64>,
    step: Option<u64>,
    chunk: Option<DshChunk>,
    header: Option<DshHeader>,
}

#[derive(Debug, Deserialize)]
struct DshChunk {
    #[serde(rename = "type")]
    chunk_type: String,
    usage: Option<DshUsage>,
    #[serde(rename = "replayState")]
    replay_state: Option<DshReplayState>,
}

#[derive(Debug, Deserialize)]
struct DshUsage {
    #[serde(rename = "inputTokens")]
    input_tokens: i64,
    #[serde(rename = "outputTokens")]
    output_tokens: i64,
    #[serde(rename = "cacheReadTokens")]
    cache_read_tokens: Option<i64>,
    #[serde(rename = "cacheWriteTokens")]
    cache_write_tokens: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct DshReplayState {
    provider: Option<String>,
    model: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DshHeader {
    config: Option<DshConfig>,
}

#[derive(Debug, Deserialize)]
struct DshConfig {
    provider: Option<String>,
    model: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::DataSource;

    /// Build a minimal zstd session log with one usage + finish pair.
    fn write_session(dir: &std::path::Path, lines: &[&str]) -> std::path::PathBuf {
        let session_dir = dir.join("ws").join("session-abc123");
        std::fs::create_dir_all(&session_dir).unwrap();
        let path = session_dir.join("session.jsonl.zstd");
        let file = File::create(&path).unwrap();
        let mut encoder = zstd::stream::write::Encoder::new(file, 0).unwrap();
        for line in lines {
            use std::io::Write;
            writeln!(encoder, "{line}").unwrap();
        }
        encoder.finish().unwrap();
        path
    }

    #[test]
    fn parses_usage_and_finish_pair() {
        let dir = tempfile::tempdir().unwrap();
        write_session(
            dir.path(),
            &[
                r#"{"type":"request/header","seq":0,"time":1786628997362,"data":{"header":{"config":{"provider":"opencode-go","model":"deepseek-v4-flash"}}}}"#,
                r#"{"type":"assistant/chunk","seq":1,"time":1786629000039,"data":{"turn":1,"step":1,"chunk":{"type":"usage","usage":{"inputTokens":12104,"outputTokens":100}}}}"#,
                r#"{"type":"assistant/chunk","seq":2,"time":1786629000040,"data":{"turn":1,"step":1,"chunk":{"type":"finish","reason":{"kind":"tool-calls"},"replayState":{"kind":"pi-ai","version":1,"api":"openai-completions","provider":"opencode-go","model":"deepseek-v4-flash","responseId":"abc"}}}}"#,
            ],
        );

        temp_env::with_var("DSH_SESSIONS_PATH", Some(dir.path()), || {
            let records = DshSource.load();
            assert_eq!(records.len(), 1);
            let r = &records[0];
            assert_eq!(r.source, "dsh");
            assert_eq!(r.provider, "opencode-go");
            assert_eq!(r.model, "deepseek-v4-flash");
            assert_eq!(r.input_tokens, 12104);
            assert_eq!(r.output_tokens, 100);
            assert_eq!(r.cache_read_tokens, 0);
            assert_eq!(r.total_tokens, 12204);
            assert_eq!(r.time, "2026-08-13T13:50:00.039+00:00");
        });
    }

    #[test]
    fn sums_multiple_usage_chunks_per_step() {
        let dir = tempfile::tempdir().unwrap();
        write_session(
            dir.path(),
            &[
                r#"{"type":"assistant/chunk","seq":1,"time":1786629000039,"data":{"turn":1,"step":1,"chunk":{"type":"usage","usage":{"inputTokens":100,"outputTokens":10}}}}"#,
                r#"{"type":"assistant/chunk","seq":2,"time":1786629000040,"data":{"turn":1,"step":1,"chunk":{"type":"usage","usage":{"inputTokens":50,"outputTokens":5,"cacheReadTokens":200}}}}"#,
                r#"{"type":"assistant/chunk","seq":3,"time":1786629000041,"data":{"turn":1,"step":1,"chunk":{"type":"finish","reason":{"kind":"stop"},"replayState":{"kind":"pi-ai","version":1,"api":"openai-completions","provider":"opencode-go","model":"deepseek-v4-flash","responseId":"abc"}}}}"#,
            ],
        );

        temp_env::with_var("DSH_SESSIONS_PATH", Some(dir.path()), || {
            let records = DshSource.load();
            assert_eq!(records.len(), 1);
            let r = &records[0];
            assert_eq!(r.input_tokens, 150);
            assert_eq!(r.output_tokens, 15);
            assert_eq!(r.cache_read_tokens, 200);
            assert_eq!(r.total_tokens, 365);
        });
    }

    #[test]
    fn falls_back_to_request_header_config() {
        let dir = tempfile::tempdir().unwrap();
        write_session(
            dir.path(),
            &[
                r#"{"type":"request/header","seq":0,"time":1786628997362,"data":{"header":{"config":{"provider":"opencode-go","model":"deepseek-v4-flash"}}}}"#,
                r#"{"type":"assistant/chunk","seq":1,"time":1786629000039,"data":{"turn":1,"step":1,"chunk":{"type":"usage","usage":{"inputTokens":10,"outputTokens":2}}}}"#,
            ],
        );

        temp_env::with_var("DSH_SESSIONS_PATH", Some(dir.path()), || {
            let records = DshSource.load();
            assert_eq!(records.len(), 1);
            assert_eq!(records[0].provider, "opencode-go");
            assert_eq!(records[0].model, "deepseek-v4-flash");
        });
    }

    #[test]
    fn skips_missing_directory() {
        let dir = tempfile::tempdir().unwrap();
        temp_env::with_var("DSH_SESSIONS_PATH", Some(dir.path().join("nope")), || {
            assert!(DshSource.load().is_empty());
        });
    }
}
