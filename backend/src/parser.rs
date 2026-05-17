use crate::models::TokenRecord;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

pub fn parse_jsonl_file<P: AsRef<Path>>(path: P) -> Vec<TokenRecord> {
    let file = match File::open(path) {
        Ok(f) => f,
        Err(_) => return Vec::new(),
    };

    let reader = BufReader::new(file);
    let mut records = Vec::new();

    for line in reader.lines() {
        if let Ok(line) = line {
            if line.trim().is_empty() {
                continue;
            }
            if let Ok(record) = serde_json::from_str::<TokenRecord>(&line) {
                records.push(record);
            }
        }
    }

    records
}

pub fn get_log_path() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    std::path::PathBuf::from(home).join(".pi").join("token-logs").join("usage.jsonl")
}
