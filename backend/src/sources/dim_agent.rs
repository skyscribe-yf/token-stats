//! DimAgent via workbuddy proxy usage source: `~/.token-stats/workbuddy-usage.jsonl`.
//!
//! Records are appended by the workbuddy CLIProxyAPI plugin for DimAgent
//! requests routed through Tencent CodeBuddy (copilot.tencent.com web API).
//! `provider` is stored as `codebuddy` so pricing's credit → CNY conversion
//! (`codebuddy_cny_per_credit`) applies; `source` is `dim-agent` to mark these
//! as DimAgent-initiated work (as opposed to native CodeBuddy CLI sessions,
//! which come from `~/.codebuddy/projects/**` with source=codebuddy).

use super::DataSource;
use crate::models::TokenRecord;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;

pub struct WorkbuddySource;

pub(crate) fn workbuddy_usage_log_path() -> PathBuf {
    std::env::var("WORKBUDDY_USAGE_LOG_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| super::home_dir().join(".token-stats/workbuddy-usage.jsonl"))
}

impl DataSource for WorkbuddySource {
    fn name(&self) -> &'static str {
        "workbuddy"
    }

    fn load(&self) -> Vec<TokenRecord> {
        let path = workbuddy_usage_log_path();
        let Ok(file) = File::open(&path) else {
            return Vec::new();
        };

        BufReader::new(file)
            .lines()
            .map_while(Result::ok)
            .filter(|line| !line.trim().is_empty())
            .filter_map(|line| match serde_json::from_str::<TokenRecord>(&line) {
                Ok(record) if record.source == "dim-agent" => Some(record),
                Ok(_) => None,
                Err(error) => {
                    tracing::warn!(
                        "Skipping invalid workbuddy usage record in {:?}: {error}",
                        path
                    );
                    None
                }
            })
            .collect()
    }

    /// Incremental: only re-read the usage log when its (mtime, size) changed.
    fn load_incremental(&self) -> Vec<TokenRecord> {
        let path = workbuddy_usage_log_path();
        let files = vec![path.clone()];
        if self.changed_data_files().is_empty() {
            return Vec::new();
        }
        let records = self.load();
        self.mark_files_parsed(&files);
        records
    }

    fn data_files(&self) -> Vec<std::path::PathBuf> {
        vec![workbuddy_usage_log_path()]
    }

    fn is_available(&self) -> bool {
        workbuddy_usage_log_path().exists()
    }
}

#[cfg(test)]
mod tests {
    use super::WorkbuddySource;
    use crate::sources::DataSource;

    #[test]
    fn loads_workbuddy_usage_jsonl() {
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("workbuddy-usage.jsonl");
        std::fs::write(
            &log_path,
            r#"{"date":"2026-09-07","time":"2026-09-07T02:48:11Z","apiKeyPrefix":"N/A","provider":"codebuddy","model":"hy3","source":"dim-agent","inputTokens":13,"outputTokens":39,"cacheReadTokens":0,"cacheWriteTokens":0,"totalTokens":52,"cost":0.0}"#,
        )
        .unwrap();

        temp_env::with_var("WORKBUDDY_USAGE_LOG_PATH", Some(log_path), || {
            let records = WorkbuddySource.load();
            assert_eq!(records.len(), 1);
            assert_eq!(records[0].source, "dim-agent");
            assert_eq!(records[0].provider, "codebuddy");
            assert_eq!(records[0].model, "hy3");
            assert_eq!(records[0].input_tokens, 13);
        });
    }

    #[test]
    fn skips_non_dim_agent_lines() {
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("workbuddy-usage.jsonl");
        // A line that is a valid TokenRecord but not from this source must be ignored.
        std::fs::write(
            &log_path,
            r#"{"date":"2026-09-07","time":"2026-09-07T02:48:11Z","apiKeyPrefix":"","provider":"codebuddy","model":"glm-5.3-flash","source":"codebuddy","inputTokens":1,"outputTokens":1,"cacheReadTokens":0,"cacheWriteTokens":0,"totalTokens":2,"cost":0.39}"#,
        )
        .unwrap();

        temp_env::with_var("WORKBUDDY_USAGE_LOG_PATH", Some(log_path), || {
            assert!(WorkbuddySource.load().is_empty());
        });
    }
}
