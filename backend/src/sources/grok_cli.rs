use super::DataSource;
use crate::models::TokenRecord;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;

pub struct GrokCliSource;

pub(crate) fn grok_usage_log_path() -> PathBuf {
    std::env::var("GROK_USAGE_LOG_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| super::home_dir().join(".token-stats/grok-usage.jsonl"))
}

impl DataSource for GrokCliSource {
    fn name(&self) -> &'static str {
        "grok-cli"
    }

    fn load(&self) -> Vec<TokenRecord> {
        let path = grok_usage_log_path();
        let Ok(file) = File::open(&path) else {
            return Vec::new();
        };

        BufReader::new(file)
            .lines()
            .map_while(Result::ok)
            .filter(|line| !line.trim().is_empty())
            .filter_map(|line| match serde_json::from_str::<TokenRecord>(&line) {
                Ok(record) if record.source == "grok-cli" => Some(record),
                Ok(_) => None,
                Err(error) => {
                    tracing::warn!("Skipping invalid Grok usage record in {:?}: {error}", path);
                    None
                }
            })
            .collect()
    }

    fn is_available(&self) -> bool {
        grok_usage_log_path().exists()
    }
}

#[cfg(test)]
mod tests {
    use super::GrokCliSource;
    use crate::sources::DataSource;

    #[test]
    fn loads_grok_usage_jsonl() {
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("grok-usage.jsonl");
        std::fs::write(
            &log_path,
            r#"{"date":"2026-07-11","time":"2026-07-11T10:00:00Z","apiKeyPrefix":"","provider":"xai","model":"grok-4.5","source":"grok-cli","inputTokens":8,"outputTokens":2,"cacheReadTokens":0,"cacheWriteTokens":0,"totalTokens":10,"cost":0.0}"#,
        )
        .unwrap();

        temp_env::with_var("GROK_USAGE_LOG_PATH", Some(log_path), || {
            let records = GrokCliSource.load();
            assert_eq!(records.len(), 1);
            assert_eq!(records[0].source, "grok-cli");
        });
    }
}