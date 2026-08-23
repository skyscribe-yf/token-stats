use super::DataSource;
use crate::models::TokenRecord;
use std::collections::HashMap;
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

    /// Incremental: only re-parse rollout files whose (mtime, size) changed
    /// since the last pass. The first pass parses everything.
    fn load_incremental(&self) -> Vec<TokenRecord> {
        let base = Self::sessions_path();
        let all = Self::rollout_files(&base);
        let changed = self.changed_data_files();
        let records = if changed.is_empty() {
            Vec::new()
        } else {
            Self::parse_files(&all, &changed)
        };
        self.mark_files_parsed(&all);
        if !changed.is_empty() {
            tracing::info!(
                "Codex: re-parsed {} changed rollout file(s), {} unchanged skipped",
                changed.len(),
                all.len() - changed.len()
            );
        } else {
            tracing::debug!(
                "Codex: all {} rollout file(s) unchanged, skipped",
                all.len()
            );
        }
        records
    }

    fn data_files(&self) -> Vec<std::path::PathBuf> {
        Self::rollout_files(&Self::sessions_path())
    }

    fn is_available(&self) -> bool {
        Self::sessions_path().exists()
    }
}

impl CodexSource {
    fn sessions_path() -> PathBuf {
        super::home_dir().join(".codex").join("sessions")
    }

    /// All rollout files under the sessions directory.
    fn rollout_files(base_path: &std::path::Path) -> Vec<std::path::PathBuf> {
        let Ok(entries) = super::walkdir(base_path) else {
            return Vec::new();
        };
        entries
            .into_iter()
            .filter(|path| {
                path.to_string_lossy().ends_with(".jsonl")
                    && path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .starts_with("rollout-")
            })
            .collect()
    }

    fn parse(base_path: &std::path::Path) -> Vec<TokenRecord> {
        Self::parse_files(&Self::rollout_files(base_path), &[])
    }

    /// Parse `files` (optionally limited to a subset of `paths`).
    ///
    /// Files are streamed line-by-line — never read wholesale into memory —
    /// because individual codex rollouts can be hundreds of MB. The session
    /// provider/model pre-scan is done on the first pass only when the file
    /// hasn't been seen before (incremental append: model/provider never
    /// change mid-file, so a changed file re-uses the known values).
    fn parse_files(
        paths: &[std::path::PathBuf],
        subset: &[std::path::PathBuf],
    ) -> Vec<TokenRecord> {
        let mut records = Vec::new();
        let mut seen_usage: HashMap<_, PathBuf> = HashMap::new();

        for path in paths {
            if !subset.is_empty() && !subset.contains(path) {
                continue;
            }
            let file = match File::open(path) {
                Ok(f) => f,
                Err(_) => continue,
            };

            // Pre-scan to discover the model from turn_context events.
            // Forked subagent sessions replay parent token_count events before
            // any turn_context appears, so a naive sequential parse would assign
            // a wrong hardcoded default model. Skipped on incremental passes:
            // provider/model never change within a file.
            let mut session_provider = "openai".to_string();
            let mut session_model = "unknown".to_string();
            if !subset.contains(path) {
                let lines = BufReader::new(file);
                for line in lines.lines().map_while(Result::ok) {
                    if session_provider == "openai" && session_model != "unknown" {
                        break;
                    }
                    let Ok(obj) = serde_json::from_str::<serde_json::Value>(&line) else {
                        continue;
                    };
                    if obj.get("type").and_then(|t| t.as_str()) == Some("session_meta") {
                        if let Some(provider) = obj
                            .get("payload")
                            .and_then(|p| p.get("model_provider"))
                            .and_then(|provider| provider.as_str())
                            .filter(|provider| !provider.is_empty())
                        {
                            session_provider = provider.to_string();
                        }
                    } else if obj.get("type").and_then(|t| t.as_str()) == Some("turn_context") {
                        if let Some(model) = obj
                            .get("payload")
                            .and_then(|p| p.get("model"))
                            .and_then(|m| m.as_str())
                        {
                            session_model = model.to_string();
                        }
                    }
                }
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
                if let Ok(obj) = serde_json::from_str::<serde_json::Value>(&line) {
                    if obj.get("type").and_then(|t| t.as_str()) == Some("turn_context") {
                        if !subset.contains(path) {
                            if let Some(model) = obj
                                .get("payload")
                                .and_then(|p| p.get("model"))
                                .and_then(|m| m.as_str())
                            {
                                session_model = model.to_string();
                            }
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

                    // Codex re-emits a token_count event with a fresh timestamp but
                    // identical usage when rate-limit info refreshes (plan change,
                    // reset window rollover) or when a session history is replayed
                    // into a forked subagent. Within a single session cumulative
                    // totals are monotonic, so an identical usage_key (same
                    // last_token_usage AND same cumulative) is a re-emit of the same
                    // billed call, not a new one — drop it. Legitimate back-to-back
                    // calls that happen to share last_token_usage always differ in
                    // cumulative and are preserved by the key.
                    let usage_key = (
                        session_provider.clone(),
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
                    if seen_usage.insert(usage_key, path.clone()).is_some() {
                        continue;
                    }

                    // OpenAI convention: input_tokens includes cache; normalize
                    let effective_input = (input_tokens - cached_input_tokens).max(0);
                    // OpenAI convention: output_tokens already includes
                    // reasoning_output_tokens (it's a breakdown subset, not
                    // additive). Use it directly to avoid double-counting.
                    let total_output_tokens = output_tokens;

                    let ts_str = obj.get("timestamp").and_then(|t| t.as_str()).unwrap_or("");
                    let (date, time) = super::parse_iso_timestamp(ts_str);

                    records.push(TokenRecord {
                        date,
                        time,
                        api_key_prefix: "N/A".to_string(),
                        provider: session_provider.clone(),
                        original_provider: None,
                        model: session_model.clone(),
                        source: "codex".to_string(),
                        input_tokens: effective_input,
                        output_tokens: total_output_tokens,
                        cache_read_tokens: cached_input_tokens,
                        cache_write_tokens: 0,
                        total_tokens: effective_input + total_output_tokens + cached_input_tokens,
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
        // output_tokens (10) already includes reasoning (2); do not add them.
        assert_eq!(records[0].output_tokens, 10);
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
    fn assigns_provider_from_session_profile() {
        let dir = tempdir().unwrap();
        let session_meta = r#"{"type":"session_meta","payload":{"model_provider":"fenno"}}"#;
        let context = r#"{"type":"turn_context","payload":{"model":"gpt-5.6-terra"}}"#;
        let event = r#"{"timestamp":"2026-07-10T10:00:00Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":100,"cached_input_tokens":80,"output_tokens":10,"total_tokens":110}}}}"#;
        fs::write(
            dir.path().join("rollout-fenno.jsonl"),
            format!("{session_meta}\n{context}\n{event}\n"),
        )
        .unwrap();

        let records = CodexSource::parse(dir.path());

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].provider, "fenno");
    }

    #[test]
    fn deduplicates_rate_limit_refresh_reemit_in_same_file() {
        // Codex re-emits the last token_count event with a fresh timestamp but
        // identical last_token_usage AND identical cumulative when rate-limit
        // info refreshes (plan change / reset-window rollover). The re-emit is
        // not a new billed call and must be dropped.
        let dir = tempdir().unwrap();
        let context = r#"{"type":"turn_context","payload":{"model":"gpt-5.6-terra"}}"#;
        let usage = r#"{"input_tokens":100,"cached_input_tokens":80,"output_tokens":10,"reasoning_output_tokens":2,"total_tokens":110}"#;
        let cumulative = r#"{"input_tokens":100,"cached_input_tokens":80,"output_tokens":10,"reasoning_output_tokens":2,"total_tokens":110}"#;
        let mk = |ts: &str, plan: &str| {
            let event = serde_json::json!({
                "timestamp": ts,
                "type": "event_msg",
                "payload": {
                    "type": "token_count",
                    "info": {
                        "last_token_usage": serde_json::from_str::<serde_json::Value>(usage).unwrap(),
                        "total_token_usage": serde_json::from_str::<serde_json::Value>(cumulative).unwrap(),
                    }
                },
                "rate_limits": {"plan_type": plan}
            });
            event.to_string()
        };
        fs::write(
            dir.path().join("rollout-a.jsonl"),
            format!(
                "{context}\n{}\n{}\n",
                mk("2026-07-23T06:02:56.315Z", "pro"),
                mk("2026-07-23T06:03:29.189Z", "team"),
            ),
        )
        .unwrap();

        let records = CodexSource::parse(dir.path());
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].output_tokens, 10);
        assert_eq!(records[0].total_tokens, 110);
    }

    #[test]
    fn preserves_identical_usage_from_different_session_profiles() {
        let dir = tempdir().unwrap();
        let context = r#"{"type":"turn_context","payload":{"model":"gpt-5.6-terra"}}"#;
        let event = r#"{"timestamp":"2026-07-10T10:00:00Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":100,"cached_input_tokens":80,"output_tokens":10,"total_tokens":110}}}}"#;

        for provider in ["fenno", "xai"] {
            let session_meta =
                format!(r#"{{"type":"session_meta","payload":{{"model_provider":"{provider}"}}}}"#);
            fs::write(
                dir.path().join(format!("rollout-{provider}.jsonl")),
                format!("{session_meta}\n{context}\n{event}\n"),
            )
            .unwrap();
        }

        let mut providers: Vec<_> = CodexSource::parse(dir.path())
            .into_iter()
            .map(|record| record.provider)
            .collect();
        providers.sort();

        assert_eq!(providers, ["fenno", "xai"]);
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

    #[test]
    fn resolves_model_for_token_count_before_turn_context() {
        // Forked subagent sessions replay parent token_count events before
        // any turn_context appears. The pre-scan must find the model from
        // later turn_context events and not fall back to a hardcoded default.
        let dir = tempdir().unwrap();
        let usage = r#"{"input_tokens":100,"cached_input_tokens":80,"output_tokens":10,"reasoning_output_tokens":2,"total_tokens":110}"#;
        let mk_count = |ts: &str, cum_input: i64| {
            format!("{{\"timestamp\":\"{ts}\",\"type\":\"event_msg\",\"payload\":{{\"type\":\"token_count\",\"info\":{{\"last_token_usage\":{usage},\"total_token_usage\":{{\"input_tokens\":{cum_input},\"cached_input_tokens\":80,\"output_tokens\":10,\"reasoning_output_tokens\":2,\"total_tokens\":110}}}}}}}}")
        };
        // token_count events (with different cumulative totals) appear BEFORE turn_context
        let replayed1 = mk_count("2026-07-10T10:00:00Z", 100);
        let replayed2 = mk_count("2026-07-10T10:01:00Z", 200);
        let turn_ctx = r#"{"type":"turn_context","payload":{"model":"gpt-5.6-terra"}}"#.to_string();
        let own_count = mk_count("2026-07-10T10:02:00Z", 300);
        fs::write(
            dir.path().join("rollout-forked.jsonl"),
            format!("{replayed1}\n{replayed2}\n{turn_ctx}\n{own_count}\n"),
        )
        .unwrap();

        let records = CodexSource::parse(dir.path());
        assert_eq!(records.len(), 3);
        for record in &records {
            assert_eq!(
                record.model, "gpt-5.6-terra",
                "Records replayed before turn_context must still get the correct model"
            );
        }
    }

    #[test]
    fn falls_back_to_unknown_when_no_turn_context_exists() {
        let dir = tempdir().unwrap();
        let event = r#"{"timestamp":"2026-07-10T10:00:00Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":50,"cached_input_tokens":20,"output_tokens":5,"reasoning_output_tokens":1,"total_tokens":55}}}}"#;
        fs::write(dir.path().join("rollout-no-context.jsonl"), event).unwrap();

        let records = CodexSource::parse(dir.path());
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].model, "unknown");
    }

    #[test]
    fn incremental_only_reparses_changed_files() {
        let dir = tempdir().unwrap();
        let context = r#"{"type":"turn_context","payload":{"model":"gpt-5.6-terra"}}"#;
        let usage = r#"{"input_tokens":100,"cached_input_tokens":80,"output_tokens":10,"reasoning_output_tokens":2,"total_tokens":110}"#;
        let event = |ts: &str| {
            format!(
                r#"{{"timestamp":"{ts}","type":"event_msg","payload":{{"type":"token_count","info":{{"last_token_usage":{usage},"total_token_usage":{{"input_tokens":100}}}}}}}}"#
            )
        };

        let file_a = dir.path().join("rollout-a.jsonl");
        let file_b = dir.path().join("rollout-b.jsonl");
        fs::write(&file_a, format!("{context}\n{}\n", event("2026-07-10T10:00:00Z"))).unwrap();
        fs::write(
            &file_b,
            format!(
                "{context}\n{}\n",
                event("2026-07-10T11:00:00Z").replace(
                    r#""total_token_usage":{"input_tokens":100}"#,
                    r#""total_token_usage":{"input_tokens":200}"#
                )
            ),
        )
        .unwrap();

        let all = vec![file_a.clone(), file_b.clone()];

        // First pass: everything is new → both files parsed.
        let changed1 = crate::sources::changed_files(&all);
        assert_eq!(changed1.len(), 2);
        let records1 = CodexSource::parse_files(&all, &changed1);
        assert_eq!(records1.len(), 2);
        crate::sources::mark_files_parsed_for_test(&all);

        // Second pass: nothing changed → no files parsed.
        let changed2 = crate::sources::changed_files(&all);
        assert!(changed2.is_empty(), "unchanged files must be skipped");
        let records2 = if changed2.is_empty() {
            Vec::new()
        } else {
            CodexSource::parse_files(&all, &changed2)
        };
        assert!(records2.is_empty());

        // Append a NEW call (different cumulative) to file_a → only file_a
        // re-parsed. Re-parsing yields all records from the changed file;
        // the caller (refresh_records) dedups already-seen ones by
        // fingerprint against memory.
        fs::write(
            &file_a,
            format!(
                "{context}\n{}\n{}\n",
                event("2026-07-10T10:00:00Z"),
                event("2026-07-10T10:01:00Z")
                    .replace(
                        r#""total_token_usage":{"input_tokens":100}"#,
                        r#""total_token_usage":{"input_tokens":300}"#
                    )
            ),
        )
        .unwrap();
        let changed3 = crate::sources::changed_files(&all);
        assert_eq!(changed3, vec![file_a.clone()], "only the appended file changed");
        let records3 = CodexSource::parse_files(&all, &changed3);
        assert_eq!(
            records3.len(),
            2,
            "re-parsing the changed file returns all its records"
        );
        assert!(
            records3
                .iter()
                .any(|r| r.time == "2026-07-10T10:01:00+00:00"),
            "the appended call is included"
        );
    }
}
