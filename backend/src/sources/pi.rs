use super::DataSource;
use crate::models::TokenRecord;
use serde::Deserialize;
use std::collections::HashSet;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

/// Pi token log source: reads `~/.pi/token-logs/usage.jsonl`
/// and taskplane lane-worker runtime exit summaries.
#[derive(Default)]
pub struct PiSource;

impl DataSource for PiSource {
    fn name(&self) -> &'static str {
        "pi"
    }

    fn load(&self) -> Vec<TokenRecord> {
        let mut records = Vec::new();

        // 1. Live session records from usage.jsonl (main session + workers
        //    that load pi-token-tracker via pi package mechanism)
        let log_path = Self::log_path();
        tracing::info!("Loading pi data from: {:?}", log_path);
        let live_records = Self::parse_log(&log_path);
        tracing::info!("Loaded {} pi live records", live_records.len());

        // Build a coverage set: if usage.jsonl already has per-call records
        // for a given (UTC date, provider, model), exit summaries for
        // matching agents are redundant (would double-count).
        let covered: HashSet<(String, String, String)> = live_records
            .iter()
            .map(|r| (r.date.clone(), r.provider.clone(), r.model.clone()))
            .collect();

        records.extend(live_records);

        // 2. Taskplane lane-worker runtime records from exit summaries.
        //    Only included for agents NOT already covered by per-call data
        //    (retroactive coverage for batches that ran before the
        //    pi-token-tracker extension was installed as a pi package).
        let runtime_records = Self::scan_taskplane_runtimes(&covered);
        if !runtime_records.is_empty() {
            tracing::info!(
                "Loaded {} pi taskplane runtime records (retroactive)",
                runtime_records.len()
            );
            records.extend(runtime_records);
        }

        records
    }
}

impl PiSource {
    fn log_path() -> PathBuf {
        super::home_dir()
            .join(".pi")
            .join("token-logs")
            .join("usage.jsonl")
    }

    fn parse_log(path: &Path) -> Vec<TokenRecord> {
        let file = match File::open(path) {
            Ok(f) => f,
            Err(_) => return Vec::new(),
        };

        let reader = BufReader::new(file);
        let mut records = Vec::new();

        for line in reader.lines().map_while(Result::ok) {
            if line.trim().is_empty() {
                continue;
            }
            if let Ok(mut record) = serde_json::from_str::<TokenRecord>(&line) {
                if record.source.is_empty() {
                    record.source = "pi".to_string();
                }
                records.push(record);
            }
        }

        records
    }

    // ── Taskplane Runtime Scanner ──────────────────────────────────────────
    //
    // Taskplane lane workers run in separate pi --mode rpc processes with
    // --no-extensions, so the token-tracker extension is NOT loaded. Their
    // token usage is recorded in exit summaries at:
    //   <project>/.pi/runtime/<batchId>/agents/<agentId>/events-exit.json
    //
    // Scans ~/srcs/*/.pi/runtime/ for these files and creates TokenRecords.

    fn scan_taskplane_runtimes(covered: &HashSet<(String, String, String)>) -> Vec<TokenRecord> {
        let projects_dir = std::env::var("TASKPLANE_PROJECTS_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| super::home_dir().join("srcs"));

        let project_dirs = match std::fs::read_dir(&projects_dir) {
            Ok(entries) => entries
                .filter_map(|e| e.ok())
                .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
                .map(|e| e.path())
                .collect::<Vec<_>>(),
            Err(_) => return Vec::new(),
        };

        let mut records = Vec::new();

        for project_dir in &project_dirs {
            let runtime_root = project_dir.join(".pi").join("runtime");
            if !runtime_root.exists() {
                continue;
            }
            let batch_records = Self::scan_batches(&runtime_root, covered);
            records.extend(batch_records);
        }

        records
    }

    fn scan_batches(
        runtime_root: &Path,
        covered: &HashSet<(String, String, String)>,
    ) -> Vec<TokenRecord> {
        let batch_dirs = match std::fs::read_dir(runtime_root) {
            Ok(entries) => entries
                .filter_map(|e| e.ok())
                .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
                .map(|e| e.path())
                .collect::<Vec<_>>(),
            Err(_) => return Vec::new(),
        };

        let mut records = Vec::new();

        for batch_path in &batch_dirs {
            let batch_name = match batch_path.file_name().and_then(|n| n.to_str()) {
                Some(name) => name.to_string(),
                None => continue,
            };

            let (batch_date, batch_time) = parse_batch_timestamp(&batch_name);

            let agents_dir = batch_path.join("agents");
            if !agents_dir.exists() {
                continue;
            }

            let agent_dirs = match std::fs::read_dir(&agents_dir) {
                Ok(entries) => entries
                    .filter_map(|e| e.ok())
                    .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
                    .map(|e| e.path())
                    .collect::<Vec<_>>(),
                Err(_) => continue,
            };

            for agent_path in &agent_dirs {
                let _agent_name = match agent_path.file_name().and_then(|n| n.to_str()) {
                    Some(name) => name.to_string(),
                    None => continue,
                };

                let exit_paths = [
                    agent_path.join("events-exit.json"),
                    agent_path.join("exit-summary.json"),
                ];

                let exit_data: ExitData = match exit_paths.iter().find_map(|p| {
                    if !p.exists() {
                        return None;
                    }
                    serde_json::from_reader(match File::open(p) {
                        Ok(f) => f,
                        Err(_) => return None,
                    })
                    .ok()
                }) {
                    Some(d) => d,
                    None => continue,
                };

                let tokens = match exit_data.tokens {
                    Some(t) => t,
                    None => continue,
                };

                let (provider, model) = read_agent_provider_model(agent_path);

                // Skip if usage.jsonl already has per-call records for this
                // (UTC date, provider, model). The batch_time is the UTC-
                // converted timestamp; its date portion matches usage.jsonl
                // records which also use UTC dates.
                let utc_date = if batch_time.len() >= 10 {
                    &batch_time[..10]
                } else {
                    &batch_date
                };
                let cover_key = (utc_date.to_string(), provider.clone(), model.clone());
                if covered.contains(&cover_key) {
                    continue;
                }

                let total_tokens = tokens.input
                    + tokens.output
                    + tokens.cache_read.unwrap_or(0)
                    + tokens.cache_write.unwrap_or(0);

                records.push(TokenRecord {
                    date: batch_date.clone(),
                    time: batch_time.clone(),
                    api_key_prefix: format!("runtime:{}", batch_name),
                    provider,
                    model,
                    source: "pi".to_string(),
                    input_tokens: tokens.input,
                    output_tokens: tokens.output,
                    cache_read_tokens: tokens.cache_read.unwrap_or(0),
                    cache_write_tokens: tokens.cache_write.unwrap_or(0),
                    total_tokens,
                    cost: exit_data.cost.unwrap_or(0.0),
                });
            }
        }

        records
    }
}

// ── Helper data types ────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct ExitTokens {
    input: i64,
    output: i64,
    #[serde(rename = "cacheRead")]
    cache_read: Option<i64>,
    #[serde(rename = "cacheWrite")]
    cache_write: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct ExitData {
    tokens: Option<ExitTokens>,
    cost: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct AgentStartedPayload {
    model: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AgentEvent {
    #[serde(rename = "type")]
    event_type: Option<String>,
    payload: Option<AgentStartedPayload>,
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Parse a batch directory name like "20260518T213033" into a date and time string.
/// Returns ("2026-05-18", "2026-05-18T13:30:33Z") assuming the timestamp is in
/// local time (Asia/Shanghai, UTC+8). Falls back to defaults on parse failure.
fn parse_batch_timestamp(batch_name: &str) -> (String, String) {
    // Expect format: YYYYMMDDTHHMMSS (e.g. "20260518T213033")
    if batch_name.len() < 15 {
        return ("unknown".to_string(), "unknown".to_string());
    }

    let year = &batch_name[0..4];
    let month = &batch_name[4..6];
    let day = &batch_name[6..8];
    let hour = &batch_name[9..11];
    let min = &batch_name[11..13];
    let sec = &batch_name[13..15];

    let date = format!("{}-{}-{}", year, month, day);

    // Assume Asia/Shanghai (UTC+8) for the batch timestamp
    // Convert to UTC by subtracting 8 hours
    let local_h: i32 = hour.parse().unwrap_or(0);
    let utc_h = (local_h - 8 + 24) % 24;
    let utc_date_adjust = if local_h < 8 { -1 } else { 0 };

    let time = if utc_date_adjust != 0 {
        // Previous day in UTC
        let prev_day: i32 = day.parse().unwrap_or(1);
        let utc_day = (prev_day + utc_date_adjust).max(1);
        format!(
            "{}-{}-{:02}T{:02}:{}:{}Z",
            year, month, utc_day, utc_h, min, sec
        )
    } else {
        format!("{}-{}-{}T{:02}:{}:{}Z", year, month, day, utc_h, min, sec)
    };

    (date, time)
}

/// Read provider and model from the first line of events.jsonl in an agent
/// directory. The first event is typically:
///   {"type":"agent_started","payload":{"model":"kimi-for-coding"}}
/// or:
///   {"type":"agent_started","payload":{"model":"xunfei/astron-code-latest"}}
///
/// If the model contains "/", it's split as "provider/model".
/// Falls back to ("taskplane-worker", "unknown").
fn read_agent_provider_model(agent_path: &Path) -> (String, String) {
    let events_path = agent_path.join("events.jsonl");
    if !events_path.exists() {
        return ("taskplane-worker".to_string(), "unknown".to_string());
    }

    let content = match std::fs::read_to_string(&events_path) {
        Ok(c) => c,
        Err(_) => return ("taskplane-worker".to_string(), "unknown".to_string()),
    };

    let first_line = match content.lines().find(|l| !l.trim().is_empty()) {
        Some(l) => l,
        None => return ("taskplane-worker".to_string(), "unknown".to_string()),
    };

    let event: AgentEvent = match serde_json::from_str(first_line) {
        Ok(e) => e,
        Err(_) => return ("taskplane-worker".to_string(), "unknown".to_string()),
    };

    // Only process agent_started events
    if event.event_type.as_deref() != Some("agent_started") {
        return ("taskplane-worker".to_string(), "unknown".to_string());
    }

    let model_ref = match event.payload.and_then(|p| p.model) {
        Some(m) => m,
        None => return ("taskplane-worker".to_string(), "unknown".to_string()),
    };

    // If model contains "/", split into provider/model
    if let Some(slash_pos) = model_ref.find('/') {
        let provider = model_ref[..slash_pos].to_string();
        let model = model_ref[slash_pos + 1..].to_string();
        (provider, model)
    } else {
        // Fallback: infer provider from model name (e.g. kimi-for-coding → kimi)
        let provider = super::resolve_provider_from_model(&model_ref);
        (provider, model_ref)
    }
}
