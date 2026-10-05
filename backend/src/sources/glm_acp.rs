//! GLM ACP usage source: `~/.token-stats/glm-acp-usage.jsonl`.
//!
//! Records written by the loopback GLM proxy (`glm_proxy.rs`) for the Paseo
//! `glm-acp-agent`, which calls Z.AI's OpenAI-compatible coding endpoint and
//! persists no usage of its own. `provider` is `bigmodel` (GLM Coding Plan
//! vendor) so the records group with the other GLM-plan traffic. Costs use the
//! same official credit formula, amortized per-credit price and time factors
//! as zcode (peak 1.0× / off-peak 0.5×); its 夜间畅用 window bills at 0.25×
//! instead of zcode's 0 (`compute_glm_acp_credit_cost`).
//!
//! The proxy records the wire model name verbatim (`glm-5.3-flash`); parse
//! time rewrites it to BigModel's official casing (`GLM-5.3-Flash`) so
//! glm-acp rows group with the ZCode source instead of splitting into a
//! duplicate lowercase model.

use super::DataSource;
use crate::models::TokenRecord;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;

pub struct GlmAcpSource;

pub(crate) fn glm_acp_usage_log_path() -> PathBuf {
    std::env::var("GLM_ACP_USAGE_LOG_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| super::home_dir().join(".token-stats/glm-acp-usage.jsonl"))
}

/// Rewrite the wire model name to BigModel's official casing so glm-acp rows
/// group with the ZCode source's models (`glm-5.3-flash` → `GLM-5.3-Flash`).
/// Non-GLM names pass through untouched.
fn official_glm_model_name(model: &str) -> String {
    let segments: Vec<String> = model.split('-').map(str::to_string).collect();
    if !segments
        .first()
        .is_some_and(|s| s.eq_ignore_ascii_case("glm"))
    {
        return model.to_string();
    }
    segments
        .into_iter()
        .enumerate()
        .map(|(i, segment)| match i {
            0 => "GLM".to_string(),
            _ if segment.eq_ignore_ascii_case("flash") => "Flash".to_string(),
            _ => segment,
        })
        .collect::<Vec<_>>()
        .join("-")
}

impl DataSource for GlmAcpSource {
    fn name(&self) -> &'static str {
        "glm-acp"
    }

    fn load(&self) -> Vec<TokenRecord> {
        let path = glm_acp_usage_log_path();
        let Ok(file) = File::open(&path) else {
            return Vec::new();
        };

        BufReader::new(file)
            .lines()
            .map_while(Result::ok)
            .filter(|line| !line.trim().is_empty())
            .filter_map(|line| match serde_json::from_str::<TokenRecord>(&line) {
                Ok(mut record) if record.source == "glm-acp" => {
                    record.model = official_glm_model_name(&record.model).into();
                    Some(record)
                }
                Ok(_) => None,
                Err(error) => {
                    tracing::warn!(
                        "Skipping invalid GLM ACP usage record in {:?}: {error}",
                        path
                    );
                    None
                }
            })
            .collect()
    }

    /// Incremental: only re-read the usage log when its (mtime, size) changed.
    fn load_incremental(&self) -> Vec<TokenRecord> {
        let path = glm_acp_usage_log_path();
        let files = vec![path.clone()];
        if self.changed_data_files().is_empty() {
            return Vec::new();
        }
        let records = self.load();
        self.mark_files_parsed(&files);
        records
    }

    fn data_files(&self) -> Vec<std::path::PathBuf> {
        vec![glm_acp_usage_log_path()]
    }

    fn is_available(&self) -> bool {
        glm_acp_usage_log_path().exists()
    }
}

#[cfg(test)]
mod tests {
    use super::{GlmAcpSource, official_glm_model_name};
    use crate::sources::DataSource;

    #[test]
    fn loads_glm_acp_usage_jsonl() {
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("glm-acp-usage.jsonl");
        std::fs::write(
            &log_path,
            r#"{"date":"2026-10-04","time":"2026-10-04T05:20:43.288Z","apiKeyPrefix":"","provider":"bigmodel","model":"glm-5.3-flash","source":"glm-acp","inputTokens":360,"outputTokens":120,"cacheReadTokens":640,"cacheWriteTokens":0,"totalTokens":1120,"cost":0.0,"ttftMs":812.0}"#,
        )
        .unwrap();

        temp_env::with_var("GLM_ACP_USAGE_LOG_PATH", Some(log_path), || {
            let records = GlmAcpSource.load();
            assert_eq!(records.len(), 1);
            assert_eq!(records[0].source, "glm-acp");
            assert_eq!(records[0].provider, "bigmodel");
            assert_eq!(records[0].model, "GLM-5.3-Flash");
            assert_eq!(records[0].ttft_ms, Some(812.0));
        });
    }

    #[test]
    fn normalizes_glm_wire_names_to_official_casing() {
        assert_eq!(official_glm_model_name("glm-5.3-flash"), "GLM-5.3-Flash");
        assert_eq!(official_glm_model_name("GLM-5.3-Flash"), "GLM-5.3-Flash");
        assert_eq!(official_glm_model_name("glm-5.3"), "GLM-5.3");
        // Non-GLM names pass through untouched.
        assert_eq!(
            official_glm_model_name("deepseek-v4-flash"),
            "deepseek-v4-flash"
        );
    }
}
