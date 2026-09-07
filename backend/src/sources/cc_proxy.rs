//! CC proxy usage source: `~/.token-stats/cc-proxy-usage.jsonl`.
//!
//! Records written by the loopback Command Code proxy (`cc_proxy.rs`) for
//! DimAgent requests routed through Command Code. `provider` is stored as
//! `commandcode` so pricing's `cc:` model prices ÷ `commandcode_divisor`
//! apply automatically; `source` is `cc-proxy` to distinguish from native
//! Command Code CLI sessions.

use super::DataSource;
use crate::models::TokenRecord;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;

pub struct CcProxySource;

pub(crate) fn cc_proxy_usage_log_path() -> PathBuf {
    std::env::var("CC_PROXY_USAGE_LOG_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| super::home_dir().join(".token-stats/cc-proxy-usage.jsonl"))
}

impl DataSource for CcProxySource {
    fn name(&self) -> &'static str {
        "cc-proxy"
    }

    fn load(&self) -> Vec<TokenRecord> {
        let path = cc_proxy_usage_log_path();
        let Ok(file) = File::open(&path) else {
            return Vec::new();
        };

        BufReader::new(file)
            .lines()
            .map_while(Result::ok)
            .filter(|line| !line.trim().is_empty())
            .filter_map(|line| match serde_json::from_str::<TokenRecord>(&line) {
                Ok(record) if record.source == "cc-proxy" => Some(record),
                Ok(_) => None,
                Err(error) => {
                    tracing::warn!(
                        "Skipping invalid CC proxy usage record in {:?}: {error}",
                        path
                    );
                    None
                }
            })
            .collect()
    }

    /// Incremental: only re-read the usage log when its (mtime, size) changed.
    fn load_incremental(&self) -> Vec<TokenRecord> {
        let path = cc_proxy_usage_log_path();
        let files = vec![path.clone()];
        if self.changed_data_files().is_empty() {
            return Vec::new();
        }
        let records = self.load();
        self.mark_files_parsed(&files);
        records
    }

    fn data_files(&self) -> Vec<std::path::PathBuf> {
        vec![cc_proxy_usage_log_path()]
    }

    fn is_available(&self) -> bool {
        cc_proxy_usage_log_path().exists()
    }
}

#[cfg(test)]
mod tests {
    use super::CcProxySource;
    use crate::sources::DataSource;

    #[test]
    fn loads_cc_proxy_usage_jsonl() {
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("cc-proxy-usage.jsonl");
        std::fs::write(
            &log_path,
            r#"{"date":"2026-09-07","time":"2026-09-07T06:00:00Z","apiKeyPrefix":"","provider":"commandcode","model":"deepseek-v4-flash","source":"cc-proxy","inputTokens":96,"outputTokens":16,"cacheReadTokens":7552,"cacheWriteTokens":0,"totalTokens":7664,"cost":0.0}"#,
        )
        .unwrap();

        temp_env::with_var("CC_PROXY_USAGE_LOG_PATH", Some(log_path), || {
            let records = CcProxySource.load();
            assert_eq!(records.len(), 1);
            assert_eq!(records[0].source, "cc-proxy");
            assert_eq!(records[0].provider, "commandcode");
        });
    }
}
