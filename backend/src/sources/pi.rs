use super::DataSource;
use crate::models::TokenRecord;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;

/// Pi token log source: reads `~/.pi/token-logs/usage.jsonl`.
#[derive(Default)]
pub struct PiSource;

impl DataSource for PiSource {
    fn name(&self) -> &'static str {
        "pi"
    }

    fn load(&self) -> Vec<TokenRecord> {
        let path = Self::log_path();
        tracing::info!("Loading pi data from: {:?}", path);
        let records = Self::parse(&path);
        tracing::info!("Loaded {} pi records", records.len());
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

    fn parse(path: &std::path::Path) -> Vec<TokenRecord> {
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
}
