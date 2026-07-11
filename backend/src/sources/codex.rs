use super::DataSource;
use crate::models::TokenRecord;
use std::collections::HashSet;
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

    fn is_available(&self) -> bool {
        Self::sessions_path().exists()
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
        let mut seen_usage = HashSet::new();

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
            let mut session_model = "gpt-5.5".to_string();
            let reader = BufReader::new(file);
            for line in reader.lines().map_while(Result::ok) {
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
                        }
                        continue;
                    }
                    if obj.get("type").and_then(|t| t.as_str()) != Some("event_msg") {
                        continue;
                    }
                    let payload = obj.get("payload");
                    if payload.is_none() {
                        continue;
                    }
                    let payload = payload.unwrap();
                    if payload.get("type").and_then(|t| t.as_str()) != Some("token_count") {
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

                    let input_tokens = usage
                        .get("input_tokens")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0);
                    let cached_input_tokens = usage
                        .get("cached_input_tokens")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0);
                    let output_tokens = usage
                        .get("output_tokens")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0);
                    let reasoning_output_tokens = usage
                        .get("reasoning_output_tokens")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0);
                    let reported_total_tokens = usage
                        .get("total_tokens")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0);
                    let cumulative = info.unwrap().get("total_token_usage");
                    let cumulative_token = |name| {
                        cumulative
                            .and_then(|v| v.get(name))
                            .and_then(|v| v.as_i64())
                            .unwrap_or(0)
                    };

                    // Codex replays token_count events with new timestamps when
                    // session histories are copied or forwarded to subagents.
                    let usage_key = (
                        session_model.clone(),
                        input_tokens,
                        cached_input_tokens,
                        output_tokens,
                        reasoning_output_tokens,
                        reported_total_tokens,
                        cumulative_token("input_tokens"),
                        cumulative_token("cached_input_tokens"),
                        cumulative_token("output_tokens"),
                        cumulative_token("reasoning_output_tokens"),
                        cumulative_token("total_tokens"),
                    );
                    if !seen_usage.insert(usage_key) {
                        continue;
                    }

                    // OpenAI convention: input_tokens includes cache; normalize
                    let effective_input = (input_tokens - cached_input_tokens).max(0);

                    let ts_str = obj.get("timestamp").and_then(|t| t.as_str()).unwrap_or("");
                    let (date, time) = super::parse_iso_timestamp(ts_str);

                    records.push(TokenRecord {
                        date,
                        time,
                        api_key_prefix: "N/A".to_string(),
                        provider: "openai".to_string(),
                        original_provider: None,
                        model: session_model.clone(),
                        source: "codex".to_string(),
                        input_tokens: effective_input,
                        output_tokens,
                        cache_read_tokens: cached_input_tokens,
                        cache_write_tokens: 0,
                        total_tokens: effective_input + output_tokens + cached_input_tokens,
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

#[cfg(test)]
mod tests {
    use super::CodexSource;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn deduplicates_replayed_token_count_events() {
        let dir = tempdir().unwrap();
        let context = r#"{"type":"turn_context","payload":{"model":"gpt-5.6-terra"}}"#;
        let usage = r#"{"input_tokens":100,"cached_input_tokens":80,"output_tokens":10,"reasoning_output_tokens":2,"total_tokens":110}"#;

        for (name, timestamp) in [
            ("rollout-a.jsonl", "2026-07-10T10:00:00Z"),
            ("rollout-b.jsonl", "2026-07-10T11:00:00Z"),
        ] {
            let event = format!(
                r#"{{"timestamp":"{timestamp}","type":"event_msg","payload":{{"type":"token_count","info":{{"last_token_usage":{usage}}}}}}}"#
            );
            fs::write(dir.path().join(name), format!("{context}\n{event}\n")).unwrap();
        }

        let records = CodexSource::parse(dir.path());

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].input_tokens, 20);
        assert_eq!(records[0].cache_read_tokens, 80);
        assert_eq!(records[0].total_tokens, 110);
    }

    #[test]
    fn preserves_identical_usage_from_different_requests() {
        let dir = tempdir().unwrap();
        let context = r#"{"type":"turn_context","payload":{"model":"gpt-5.6-terra"}}"#;
        let usage = r#"{"input_tokens":100,"cached_input_tokens":80,"output_tokens":10,"reasoning_output_tokens":2,"total_tokens":110}"#;
        let mut lines = vec![context.to_string()];

        for (timestamp, cumulative_input) in
            [("2026-07-10T10:00:00Z", 100), ("2026-07-10T11:00:00Z", 200)]
        {
            lines.push(format!(
                r#"{{"timestamp":"{timestamp}","type":"event_msg","payload":{{"type":"token_count","info":{{"last_token_usage":{usage},"total_token_usage":{{"input_tokens":{cumulative_input},"cached_input_tokens":80,"output_tokens":10,"reasoning_output_tokens":2,"total_tokens":110}}}}}}}}"#
            ));
        }
        fs::write(dir.path().join("rollout-a.jsonl"), lines.join("\n")).unwrap();

        assert_eq!(CodexSource::parse(dir.path()).len(), 2);
    }

    #[test]
    fn assigns_token_counts_to_the_latest_turn_context_model() {
        let dir = tempdir().unwrap();
        let terra_context = r#"{"type":"turn_context","payload":{"model":"gpt-5.6-terra"}}"#;
        let sol_context = r#"{"type":"turn_context","payload":{"model":"gpt-5.6-sol"}}"#;
        let terra_event = r#"{"timestamp":"2026-07-10T10:00:00Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":100,"cached_input_tokens":80,"output_tokens":10,"total_tokens":110}}}}"#;
        let sol_event = r#"{"timestamp":"2026-07-10T10:01:00Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":200,"cached_input_tokens":160,"output_tokens":20,"total_tokens":220}}}}"#;
        fs::write(
            dir.path().join("rollout-a.jsonl"),
            format!("{terra_context}\n{terra_event}\n{sol_context}\n{sol_event}\n"),
        )
        .unwrap();

        let records = CodexSource::parse(dir.path());
        let models: Vec<_> = records.iter().map(|record| record.model.as_str()).collect();

        assert_eq!(models, ["gpt-5.6-terra", "gpt-5.6-sol"]);
    }
}
