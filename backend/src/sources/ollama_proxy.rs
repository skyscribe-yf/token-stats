//! Ollama Cloud per-request usage source: `~/.token-stats/ollama-usage.jsonl`.
//!
//! Records are appended by the `ollama-usage` CLIProxyAPI plugin
//! (`~/workbuddy-proxy/ollama-usage-plugin`), which receives every completed
//! request routed through the shared CLIProxyAPI instance's `ollama-cloud`
//! openai-compatibility upstream (DimAgent's `ollama-cloud-proxy` channel).
//!
//! This replaces the per-run aggregates Dim's local SQLite used to expose for
//! that channel: each line is a single upstream call with its own TTFT and TPS,
//! so the dashboard's request table can show individual calls instead of one
//! summed row per DimAgent run.
//!
//! `provider` is stored as `ollama-cloud` so vendor_merge.toml merges the
//! records into the `ollama` group and `pricing.rs` applies the empirical
//! subscription rate; `source` is `ollama-proxy` to distinguish them from
//! dim-supplement rows and from other proxies' logs.

use super::DataSource;
use crate::models::TokenRecord;
use chrono::DateTime;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::sync::Mutex;

pub struct OllamaProxySource;

/// Provider id the Ollama Cloud channel used *before* it was routed through
/// CLIProxyAPI. Its `usage_run_stats` rows are per-DimAgent-run aggregates; the
/// ones at or after the proxy's first record are dropped so they do not double
/// count the same usage as the per-request `ollama-proxy` records. (Vendor
/// merge renames the *provider* to `ollama`, but keeps this raw id in
/// `original_provider`, which is what the predicates below match on.)
pub(crate) const OLLAMA_CLOUD_RUN_PROVIDER: &str = "custom-ollama-cloud-042036d3";

pub(crate) fn ollama_proxy_usage_log_path() -> PathBuf {
    std::env::var("OLLAMA_PROXY_USAGE_LOG_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| super::home_dir().join(".token-stats/ollama-usage.jsonl"))
}

/// Timestamp of the earliest proxied Ollama Cloud request, i.e. the moment the
/// channel switched from Dim's per-run aggregates to per-request capture.
///
/// Any `source=dim` / `original_provider=custom-ollama-cloud-…` row at or
/// after this instant covers usage that the proxy also recorded, so it must be
/// dropped to avoid double counting. Rows *before* it are older history the
/// proxy never saw and are kept.
///
/// The log is append-only, so its first line is the earliest record. The value
/// is cached per log path once found and re-attempted while absent, so a
/// dashboard started before the plugin has produced anything picks the cutoff
/// up on a later refresh instead of keeping every run row forever.
pub(crate) fn ollama_run_cutoff() -> Option<String> {
    static CUTOFF: Mutex<Option<(PathBuf, String)>> = Mutex::new(None);

    let path = ollama_proxy_usage_log_path();
    let mut cached = match CUTOFF.lock() {
        Ok(guard) => guard,
        // A poisoned lock only costs a re-read; never panic on it.
        Err(poisoned) => poisoned.into_inner(),
    };
    if let Some((cached_path, value)) = cached.as_ref()
        && *cached_path == path
    {
        return Some(value.clone());
    }
    let cutoff = first_record_time(&path)?;
    *cached = Some((path, cutoff.clone()));
    Some(cutoff)
}

/// Read the `time` field of the log's first line.
fn first_record_time(path: &std::path::Path) -> Option<String> {
    let file = File::open(path).ok()?;
    for line in BufReader::new(file).lines() {
        let line = line.ok()?;
        if line.trim().is_empty() {
            continue;
        }
        let value: serde_json::Value = serde_json::from_str(&line).ok()?;
        return value
            .get("time")
            .and_then(|t| t.as_str())
            .filter(|t| !t.is_empty())
            .map(|t| t.to_string());
    }
    None
}

/// Whether a timestamp belongs to the window the proxy now covers: at or after
/// the proxy's first recorded request. Unparseable inputs return `false` so the
/// row is kept — retaining history is safer than dropping it.
pub(crate) fn ollama_run_time_superseded(time: &str) -> bool {
    let Some(cutoff) = ollama_run_cutoff() else {
        return false;
    };
    match (
        DateTime::parse_from_rfc3339(time),
        DateTime::parse_from_rfc3339(&cutoff),
    ) {
        (Ok(record_time), Ok(cutoff_time)) => record_time >= cutoff_time,
        _ => false,
    }
}

/// Record-level form of [`ollama_run_time_superseded`], matched on the raw
/// provider id so it works both on freshly parsed dim rows (before vendor
/// merge) and on already-persisted ones (where `provider` is `ollama`).
pub(crate) fn ollama_run_record_superseded(record: &TokenRecord) -> bool {
    let raw_provider = record
        .original_provider
        .as_deref()
        .unwrap_or(record.provider.as_str());
    if raw_provider != OLLAMA_CLOUD_RUN_PROVIDER {
        return false;
    }
    ollama_run_time_superseded(&record.time)
}

impl DataSource for OllamaProxySource {
    fn name(&self) -> &'static str {
        "ollama-proxy"
    }

    fn load(&self) -> Vec<TokenRecord> {
        let path = ollama_proxy_usage_log_path();
        let Ok(file) = File::open(&path) else {
            return Vec::new();
        };

        BufReader::new(file)
            .lines()
            .map_while(Result::ok)
            .filter(|line| !line.trim().is_empty())
            .filter_map(|line| match serde_json::from_str::<TokenRecord>(&line) {
                Ok(record) if record.source == "ollama-proxy" => Some(record),
                Ok(_) => None,
                Err(error) => {
                    tracing::warn!(
                        "Skipping invalid ollama proxy usage record in {:?}: {error}",
                        path
                    );
                    None
                }
            })
            .collect()
    }

    /// Incremental: only re-read the usage log when its (mtime, size) changed.
    fn load_incremental(&self) -> Vec<TokenRecord> {
        let files = vec![ollama_proxy_usage_log_path()];
        if self.changed_data_files().is_empty() {
            return Vec::new();
        }
        let records = self.load();
        self.mark_files_parsed(&files);
        records
    }

    fn data_files(&self) -> Vec<PathBuf> {
        vec![ollama_proxy_usage_log_path()]
    }

    fn is_available(&self) -> bool {
        ollama_proxy_usage_log_path().exists()
    }
}

#[cfg(test)]
mod tests {
    use super::OllamaProxySource;
    use crate::sources::DataSource;
    use std::sync::OnceLock;

    #[test]
    fn loads_ollama_proxy_usage_jsonl() {
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("ollama-usage.jsonl");
        std::fs::write(
            &log_path,
            r#"{"date":"2026-09-11","time":"2026-09-11T14:01:03.121Z","apiKeyPrefix":"N/A","provider":"ollama-cloud","model":"deepseek-v4.1-flash","source":"ollama-proxy","inputTokens":134,"outputTokens":296,"cacheReadTokens":2900,"cacheWriteTokens":0,"totalTokens":3330,"cost":0.0,"ttftMs":592.2,"tps":41.3}"#,
        )
        .unwrap();

        temp_env::with_var("OLLAMA_PROXY_USAGE_LOG_PATH", Some(&log_path), || {
            let records = OllamaProxySource.load();
            assert_eq!(records.len(), 1);
            assert_eq!(records[0].source, "ollama-proxy");
            assert_eq!(records[0].provider, "ollama-cloud");
            assert_eq!(records[0].model, "deepseek-v4.1-flash");
            assert_eq!(records[0].input_tokens, 134);
            assert_eq!(records[0].cache_read_tokens, 2900);
            assert_eq!(records[0].ttft_ms, Some(592.2));
            assert_eq!(records[0].tps, Some(41.3));
        });
    }

    #[test]
    fn skips_other_sources_and_reports_cutoff() {
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("ollama-usage.jsonl");
        // First line carries the cutoff; the second is a valid TokenRecord from
        // another source and must be ignored.
        std::fs::write(
            &log_path,
            concat!(
                r#"{"date":"2026-09-11","time":"2026-09-11T14:01:03.121Z","apiKeyPrefix":"N/A","provider":"ollama-cloud","model":"glm-5.3","source":"ollama-proxy","inputTokens":1,"outputTokens":1,"cacheReadTokens":0,"cacheWriteTokens":0,"totalTokens":2,"cost":0.0}"#,
                "\n",
                r#"{"date":"2026-09-11","time":"2026-09-11T15:00:00Z","apiKeyPrefix":"","provider":"codebuddy","model":"hy3","source":"dim-agent","inputTokens":1,"outputTokens":1,"cacheReadTokens":0,"cacheWriteTokens":0,"totalTokens":2,"cost":0.0}"#,
                "\n"
            ),
        )
        .unwrap();

        temp_env::with_var("OLLAMA_PROXY_USAGE_LOG_PATH", Some(&log_path), || {
            let records = OllamaProxySource.load();
            assert_eq!(records.len(), 1);
            assert_eq!(records[0].model, "glm-5.3");
        });
    }

    #[test]
    fn cutoff_is_none_when_log_missing() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("absent.jsonl");
        temp_env::with_var("OLLAMA_PROXY_USAGE_LOG_PATH", Some(&missing), || {
            assert!(super::first_record_time(&missing).is_none());
        });
    }

    #[test]
    fn cutoff_gates_run_rows_at_or_after_first_record() {
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("ollama-usage.jsonl");
        std::fs::write(
            &log_path,
            concat!(
                r#"{"date":"2026-09-11","time":"2026-09-11T14:00:00Z","apiKeyPrefix":"N/A","provider":"ollama-cloud","model":"glm-5.3","source":"ollama-proxy","inputTokens":1,"outputTokens":1,"cacheReadTokens":0,"cacheWriteTokens":0,"totalTokens":2,"cost":0.0}"#,
                "\n"
            ),
        )
        .unwrap();

        temp_env::with_var("OLLAMA_PROXY_USAGE_LOG_PATH", Some(&log_path), || {
            // Rows on/after the cutoff belong to the proxy's window.
            assert!(super::ollama_run_time_superseded("2026-09-11T14:00:00Z"));
            assert!(super::ollama_run_time_superseded(
                "2026-09-11T14:00:00.500Z"
            ));
            assert!(super::ollama_run_time_superseded(
                "2026-09-11T15:30:00+00:00"
            ));
            // Earlier rows are history the proxy never saw.
            assert!(!super::ollama_run_time_superseded("2026-09-11T13:59:59Z"));
            assert!(!super::ollama_run_time_superseded(
                "2026-09-05T23:24:12.948+00:00"
            ));
            // Unparseable timestamps keep the row.
            assert!(!super::ollama_run_time_superseded("not-a-timestamp"));
        });
    }

    #[test]
    fn record_predicate_matches_raw_provider_only() {
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("ollama-usage.jsonl");
        std::fs::write(
            &log_path,
            concat!(
                r#"{"date":"2026-09-11","time":"2026-09-11T14:00:00Z","apiKeyPrefix":"N/A","provider":"ollama-cloud","model":"glm-5.3","source":"ollama-proxy","inputTokens":1,"outputTokens":1,"cacheReadTokens":0,"cacheWriteTokens":0,"totalTokens":2,"cost":0.0}"#,
                "\n"
            ),
        )
        .unwrap();

        temp_env::with_var("OLLAMA_PROXY_USAGE_LOG_PATH", Some(&log_path), || {
            let mut run_row = crate::models::TokenRecord {
                date: "2026-09-11".into(),
                parsed_time: OnceLock::new(),
                time: "2026-09-11T14:30:00+00:00".into(),
                api_key_prefix: "N/A".into(),
                // Vendor merge renames the provider but keeps the raw id.
                provider: "ollama".into(),
                original_provider: Some(super::OLLAMA_CLOUD_RUN_PROVIDER.into()),
                model: "deepseek-v4.1-flash".into(),
                source: "dim".into(),
                input_tokens: 1,
                output_tokens: 1,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
                total_tokens: 2,
                cost: 0.0,
                ttft_ms: None,
                tps: None,
            };
            assert!(super::ollama_run_record_superseded(&run_row));

            // A per-request record from the proxy is never "superseded".
            run_row.original_provider = Some("ollama-cloud".into());
            assert!(!super::ollama_run_record_superseded(&run_row));

            // Same raw provider but before the cutoff stays.
            run_row.original_provider = Some(super::OLLAMA_CLOUD_RUN_PROVIDER.into());
            run_row.time = "2026-09-10T10:00:00+00:00".into();
            assert!(!super::ollama_run_record_superseded(&run_row));
        });
    }
}
