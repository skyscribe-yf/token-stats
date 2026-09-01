//! Command Code (`cmd`) source: `~/.commandcode/projects/<slug>/<session>.jsonl`.
//!
//! Each billable assistant turn is a `type=message` line with `usage` and
//! `model`. `inputTokens` includes `cacheReadTokens` (OpenAI convention);
//! the parser subtracts so stored `input_tokens` is uncached input only.
//! Sidecar `*.checkpoints.jsonl` files are skipped.

use super::DataSource;
use crate::models::TokenRecord;
use std::collections::HashSet;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;

#[derive(Default)]
pub struct CommandCodeSource;

impl DataSource for CommandCodeSource {
    fn name(&self) -> &'static str {
        "commandcode"
    }

    fn load(&self) -> Vec<TokenRecord> {
        let base = Self::projects_path();
        tracing::info!("Loading Command Code data from: {:?}", base);
        let records = Self::parse(&base);
        tracing::info!("Loaded {} commandcode records", records.len());
        records
    }

    /// Incremental: only re-parse session jsonl files whose (mtime, size) changed.
    fn load_incremental(&self) -> Vec<TokenRecord> {
        let base = Self::projects_path();
        let files = Self::jsonl_files(&base);
        // Reuse the list we just built: the trait default for
        // `changed_data_files` would walk the projects tree again.
        let changed = super::changed_files(&files);
        let records = if changed.is_empty() {
            Vec::new()
        } else {
            Self::parse_files(&files, &changed)
        };
        self.mark_files_parsed(&files);
        records
    }

    fn data_files(&self) -> Vec<std::path::PathBuf> {
        Self::jsonl_files(&Self::projects_path())
    }

    fn is_available(&self) -> bool {
        Self::projects_path().exists()
    }
}

impl CommandCodeSource {
    fn projects_path() -> PathBuf {
        if let Ok(p) = std::env::var("COMMANDCODE_PROJECTS_PATH") {
            return PathBuf::from(p);
        }
        super::home_dir().join(".commandcode").join("projects")
    }

    fn jsonl_files(base_path: &std::path::Path) -> Vec<std::path::PathBuf> {
        if !base_path.exists() {
            return Vec::new();
        }
        match super::walkdir(base_path) {
            Ok(entries) => entries
                .into_iter()
                .filter(|p| {
                    p.to_string_lossy().ends_with(".jsonl")
                        && !p
                            .file_name()
                            .and_then(|n| n.to_str())
                            .is_some_and(|n| n.ends_with(".checkpoints.jsonl"))
                })
                .collect(),
            Err(_) => Vec::new(),
        }
    }

    fn parse(base_path: &std::path::Path) -> Vec<TokenRecord> {
        Self::parse_files(&Self::jsonl_files(base_path), &[])
    }

    fn parse_files(
        paths: &[std::path::PathBuf],
        subset: &[std::path::PathBuf],
    ) -> Vec<TokenRecord> {
        let mut records = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();

        for path in paths {
            if !subset.is_empty() && !subset.contains(path) {
                continue;
            }

            let file = match File::open(path) {
                Ok(f) => f,
                Err(_) => continue,
            };

            let reader = BufReader::new(file);
            for line in reader.lines().map_while(Result::ok) {
                if line.trim().is_empty() {
                    continue;
                }
                let Ok(obj) = serde_json::from_str::<serde_json::Value>(&line) else {
                    continue;
                };
                if obj.get("type").and_then(|v| v.as_str()) != Some("message") {
                    continue;
                }
                let usage = match obj.get("usage") {
                    Some(u) if !u.is_null() => u,
                    _ => continue,
                };
                let model_raw = match obj.get("model").and_then(|v| v.as_str()) {
                    Some(m) if !m.is_empty() => m,
                    _ => continue,
                };

                let dedup_id = obj
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                if !dedup_id.is_empty() && !seen.insert(dedup_id.clone()) {
                    continue;
                }

                let raw_input_tokens = usage
                    .get("inputTokens")
                    .or_else(|| usage.get("input_tokens"))
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0);
                let output_tokens = usage
                    .get("outputTokens")
                    .or_else(|| usage.get("output_tokens"))
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0);
                let cache_read_tokens = usage
                    .get("cacheReadTokens")
                    .or_else(|| usage.get("cache_read_input_tokens"))
                    .or_else(|| usage.get("cache_read_tokens"))
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0);
                let cache_write_tokens = usage
                    .get("cacheWriteTokens")
                    .or_else(|| usage.get("cache_creation_input_tokens"))
                    .or_else(|| usage.get("cache_write_tokens"))
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0);

                if raw_input_tokens == 0
                    && output_tokens == 0
                    && cache_read_tokens == 0
                    && cache_write_tokens == 0
                {
                    continue;
                }

                // Native `cmd` JSONL uses the OpenAI convention: inputTokens
                // includes cacheReadTokens. Subtract so cache_hit_ratio =
                // cache / (uncached_input + cache) can exceed 50%.
                // Pi-via-Command-Code is normalized the same way in
                // load_all_sources().
                let input_tokens = (raw_input_tokens - cache_read_tokens).max(0);

                let model = if let Some(pos) = model_raw.find('/') {
                    model_raw[pos + 1..].to_string()
                } else {
                    model_raw.to_string()
                };
                let model = super::normalize_model_name(&model);
                let provider = "commandcode".to_string();

                let ts_str = obj.get("timestamp").and_then(|v| v.as_str()).unwrap_or("");
                let (date, time) = super::parse_iso_timestamp(ts_str);

                records.push(TokenRecord {
                    date,
                    time,
                    api_key_prefix: "N/A".to_string(),
                    provider,
                    original_provider: None,
                    model,
                    source: "commandcode".to_string(),
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

        records
    }
}

#[cfg(test)]
mod tests {
    use super::CommandCodeSource;
    use crate::sources::DataSource;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn parses_commandcode_usage() {
        let dir = tempdir().unwrap();
        let proj = dir.path().join("proj");
        fs::create_dir_all(&proj).unwrap();
        let line = r#"{"type":"message","id":"abc123","timestamp":"2026-08-18T23:22:28.444Z","model":"meta/muse-spark-1.2-contributor","usage":{"inputTokens":29710,"outputTokens":345,"cacheReadTokens":177,"cacheWriteTokens":0,"costUsd":0.003},"message":{"role":"assistant","content":[]}}"#;
        fs::write(proj.join("sess.jsonl"), format!("{line}\n")).unwrap();
        let records = CommandCodeSource::parse(dir.path());
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].source, "commandcode");
        assert_eq!(records[0].provider, "commandcode");
        assert_eq!(records[0].model, "muse-spark-1.2-contributor");
        // Native cmd reports OpenAI-inclusive inputTokens. Exclusive
        // input is 29710 - 177 = 29533.
        assert_eq!(records[0].input_tokens, 29533);
        assert_eq!(records[0].cache_read_tokens, 177);
        assert_eq!(records[0].total_tokens, 29533 + 345 + 177);
    }

    #[test]
    fn subtracts_inclusive_input_when_cache_dominates() {
        let dir = tempdir().unwrap();
        let proj = dir.path().join("proj");
        fs::create_dir_all(&proj).unwrap();
        // Real cmd line: inputTokens includes cache. Storing as-is makes
        // cache_hit = cache/(input+cache) cap at 50%.
        let line = r#"{"type":"message","id":"76da4e81","timestamp":"2026-08-23T12:32:48.057Z","model":"deepseek/deepseek-v4-flash","usage":{"inputTokens":140505,"outputTokens":1197,"cacheReadTokens":140209,"cacheWriteTokens":0,"costUsd":0.014570318},"message":{"role":"assistant","content":[]}}"#;
        fs::write(proj.join("sess.jsonl"), format!("{line}\n")).unwrap();
        let records = CommandCodeSource::parse(dir.path());
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].input_tokens, 296);
        assert_eq!(records[0].cache_read_tokens, 140209);
        assert_eq!(records[0].total_tokens, 296 + 1197 + 140209);
        let denom = records[0].input_tokens + records[0].cache_read_tokens;
        let ratio = records[0].cache_read_tokens as f64 / denom as f64 * 100.0;
        assert!(ratio > 99.0, "cache hit should reflect inclusive input, got {ratio}");
    }

    #[test]
    fn parses_local_home_projects_if_present() {
        let path = crate::sources::home_dir()
            .join(".commandcode")
            .join("projects");
        if !path.exists() {
            return;
        }
        let records = CommandCodeSource::parse(&path);
        if records.is_empty() {
            return;
        }
        assert!(records.iter().all(|r| r.source == "commandcode"));
        assert!(records.iter().all(|r| r.provider == "commandcode"));
        // Known first muse-spark turn from this machine's cmd log. Raw
        // inputTokens is 29710 inclusive of 177 cache reads.
        if let Some(r) = records.iter().find(|r| {
            r.model == "muse-spark-1.2-contributor"
                && r.output_tokens == 345
                && r.cache_read_tokens == 177
        }) {
            assert_eq!(r.input_tokens, 29533);
        }
    }

    #[test]
    fn deduplicates_by_id() {
        let dir = tempdir().unwrap();
        let line = r#"{"type":"message","id":"dup","timestamp":"2026-08-18T23:22:28.444Z","model":"meta/muse-spark-1.2-contributor","usage":{"inputTokens":100,"outputTokens":10,"cacheReadTokens":0,"cacheWriteTokens":0},"message":{"role":"assistant","content":[]}}"#;
        fs::write(dir.path().join("a.jsonl"), format!("{line}\n{line}\n")).unwrap();
        assert_eq!(CommandCodeSource::parse(dir.path()).len(), 1);
    }

    #[test]
    fn loads_via_env_override() {
        let dir = tempdir().unwrap();
        let proj = dir.path().join("proj");
        fs::create_dir_all(&proj).unwrap();
        let line = r#"{"type":"message","id":"x1","timestamp":"2026-08-18T23:22:28.444Z","model":"meta/muse-spark-1.2-contributor","usage":{"inputTokens":10,"outputTokens":5,"cacheReadTokens":0,"cacheWriteTokens":0},"message":{"role":"assistant","content":[]}}"#;
        fs::write(proj.join("sess.jsonl"), format!("{line}\n")).unwrap();
        temp_env::with_var(
            "COMMANDCODE_PROJECTS_PATH",
            Some(dir.path().to_str().unwrap()),
            || {
                assert_eq!(CommandCodeSource.load().len(), 1);
            },
        );
    }

    #[test]
    fn skips_checkpoints_file() {
        let dir = tempdir().unwrap();
        let line = r#"{"type":"message","id":"cp1","timestamp":"2026-08-18T23:22:28.444Z","model":"meta/muse-spark-1.2-contributor","usage":{"inputTokens":10,"outputTokens":5,"cacheReadTokens":0,"cacheWriteTokens":0},"message":{"role":"assistant","content":[]}}"#;
        fs::write(
            dir.path().join("sess.checkpoints.jsonl"),
            format!("{line}\n"),
        )
        .unwrap();
        assert_eq!(CommandCodeSource::parse(dir.path()).len(), 0);
    }
}
