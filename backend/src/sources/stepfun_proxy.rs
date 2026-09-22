//! StepFun StepPlan per-request usage source: `~/.token-stats/stepfun-usage.jsonl`.
//!
//! Records are appended by the `stepfun-usage` CLIProxyAPI plugin
//! (`~/workbuddy-proxy/stepfun-usage-plugin`), which receives every completed
//! request routed through the shared CLIProxyAPI instance's `stepfun`
//! openai-compatibility upstream (`https://api.stepfun.com/step_plan/v1`,
//! catalog id `step/step-5-preview`).
//!
//! `provider` is stored as `stepfun` and deliberately kept out of
//! `vendor_merge.toml`, so the vendor chart shows StepFun on its own and
//! `pricing.rs` can apply the StepPlan list-price/subscription divisor.
//!
//! DimAgent channels that talk to StepFun **directly** (`step-plan`,
//! `stepfun`) stay in the `dim` source: those requests never reach the proxy,
//! so dropping them would lose data. Channels whose baseUrl points at the
//! proxy are excluded on the `dim` side instead — see `dim.rs`.

use super::DataSource;
use crate::models::TokenRecord;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;

pub struct StepfunProxySource;

fn stepfun_proxy_usage_log_path() -> PathBuf {
    std::env::var("STEPFUN_PROXY_USAGE_LOG_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| super::home_dir().join(".token-stats/stepfun-usage.jsonl"))
}

impl DataSource for StepfunProxySource {
    fn name(&self) -> &'static str {
        "stepfun-proxy"
    }

    fn load(&self) -> Vec<TokenRecord> {
        let path = stepfun_proxy_usage_log_path();
        let Ok(file) = File::open(&path) else {
            return Vec::new();
        };

        BufReader::new(file)
            .lines()
            .map_while(Result::ok)
            .filter(|line| !line.trim().is_empty())
            .filter_map(|line| match serde_json::from_str::<TokenRecord>(&line) {
                Ok(record) if record.source == "stepfun-proxy" => Some(record),
                Ok(_) => None,
                Err(error) => {
                    tracing::warn!(
                        "Skipping invalid stepfun proxy usage record in {:?}: {error}",
                        path
                    );
                    None
                }
            })
            .collect()
    }

    /// Incremental: only re-read the usage log when its (mtime, size) changed.
    fn load_incremental(&self) -> Vec<TokenRecord> {
        let files = vec![stepfun_proxy_usage_log_path()];
        if self.changed_data_files().is_empty() {
            return Vec::new();
        }
        let records = self.load();
        self.mark_files_parsed(&files);
        records
    }

    fn data_files(&self) -> Vec<PathBuf> {
        vec![stepfun_proxy_usage_log_path()]
    }

    fn is_available(&self) -> bool {
        stepfun_proxy_usage_log_path().exists()
    }
}

#[cfg(test)]
mod tests {
    use super::StepfunProxySource;
    use crate::sources::DataSource;

    #[test]
    fn loads_stepfun_proxy_usage_jsonl() {
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("stepfun-usage.jsonl");
        std::fs::write(
            &log_path,
            concat!(
                r#"{"date":"2026-09-20","time":"2026-09-20T06:00:24.170Z","apiKeyPrefix":"N/A","provider":"stepfun","model":"step-5-preview","source":"stepfun-proxy","inputTokens":18,"outputTokens":64,"cacheReadTokens":0,"cacheWriteTokens":0,"totalTokens":82,"cost":0,"ttftMs":7446.024638}"#,
                "\n",
                // A record from another proxy log must never be picked up here.
                r#"{"date":"2026-09-20","time":"2026-09-20T06:10:00Z","apiKeyPrefix":"N/A","provider":"ollama-cloud","model":"glm-5.3","source":"ollama-proxy","inputTokens":1,"outputTokens":1,"cacheReadTokens":0,"cacheWriteTokens":0,"totalTokens":2,"cost":0}"#,
                "\n"
            ),
        )
        .unwrap();

        temp_env::with_var("STEPFUN_PROXY_USAGE_LOG_PATH", Some(&log_path), || {
            let records = StepfunProxySource.load();
            assert_eq!(records.len(), 1);
            assert_eq!(records[0].source, "stepfun-proxy");
            assert_eq!(records[0].provider, "stepfun");
            assert_eq!(records[0].model, "step-5-preview");
            assert_eq!(records[0].input_tokens, 18);
            assert_eq!(records[0].output_tokens, 64);
            assert_eq!(records[0].ttft_ms, Some(7446.024638));
        });
    }

    #[test]
    fn missing_log_degrades_to_empty() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("absent.jsonl");
        temp_env::with_var("STEPFUN_PROXY_USAGE_LOG_PATH", Some(&missing), || {
            assert!(StepfunProxySource.load().is_empty());
            assert!(!StepfunProxySource.is_available());
        });
    }
}
