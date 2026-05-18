use super::DataSource;
use crate::models::TokenRecord;
use chrono::{TimeZone, Utc};
use std::path::PathBuf;

/// OpenCode source: reads `~/.local/share/opencode/opencode.db` (SQLite).
#[derive(Default)]
pub struct OpenCodeSource;

impl DataSource for OpenCodeSource {
    fn name(&self) -> &'static str {
        "opencode"
    }

    fn load(&self) -> Vec<TokenRecord> {
        let path = Self::db_path();
        tracing::info!("Loading OpenCode data from: {:?}", path);
        let records = Self::parse(&path);
        tracing::info!("Loaded {} opencode records", records.len());
        records
    }
}

impl OpenCodeSource {
    fn db_path() -> PathBuf {
        super::home_dir()
            .join(".local")
            .join("share")
            .join("opencode")
            .join("opencode.db")
    }

    fn parse(path: &std::path::Path) -> Vec<TokenRecord> {
        if !path.exists() {
            tracing::warn!("OpenCode DB not found at {:?}, skipping", path);
            return Vec::new();
        }

        let conn = match rusqlite::Connection::open_with_flags(
            path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        ) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("Failed to open OpenCode DB: {}, skipping", e);
                return Vec::new();
            }
        };

        let mut records = Vec::new();
        let sql = "SELECT data FROM message";

        let mut stmt = match conn.prepare(sql) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("Failed to prepare OpenCode query: {}, skipping", e);
                return records;
            }
        };

        let rows = stmt.query_map([], |row| {
            let data: String = row.get(0)?;
            Ok(data)
        });

        match rows {
            Ok(r) => {
                for data in r.flatten() {
                    if let Some(record) = Self::parse_message(&data) {
                        records.push(record);
                    }
                }
            }
            Err(e) => tracing::warn!("Failed to iterate OpenCode messages: {}", e),
        }

        records
    }

    fn parse_message(data: &str) -> Option<TokenRecord> {
        let obj: serde_json::Value = serde_json::from_str(data).ok()?;

        // Only assistant messages with token usage
        if obj.get("role").and_then(|v| v.as_str()) != Some("assistant") {
            return None;
        }

        let tokens = obj.get("tokens")?;
        if tokens.is_null() {
            return None;
        }

        let input_tokens = tokens.get("input").and_then(|v| v.as_i64()).unwrap_or(0);
        let output_tokens = tokens.get("output").and_then(|v| v.as_i64()).unwrap_or(0);
        let reasoning_tokens = tokens
            .get("reasoning")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let cache = tokens.get("cache");
        let cache_read_tokens = cache
            .and_then(|c| c.get("read"))
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let cache_write_tokens = cache
            .and_then(|c| c.get("write"))
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let total_tokens = tokens.get("total").and_then(|v| v.as_i64()).unwrap_or(0);

        // Filter out zero-usage records (intermediate streaming states)
        if input_tokens == 0
            && output_tokens == 0
            && cache_read_tokens == 0
            && cache_write_tokens == 0
        {
            return None;
        }

        let effective_output = output_tokens + reasoning_tokens;

        let model = obj
            .get("modelID")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();

        let mut provider = obj
            .get("providerID")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();

        // Normalize opencode → opencode-go for consistency
        if provider == "opencode" {
            provider = "opencode-go".to_string();
        }
        // Fallback to model-based resolution
        if provider == "unknown" || provider.is_empty() {
            provider = super::resolve_provider_from_model(&model);
        }

        let cost = obj.get("cost").and_then(|v| v.as_f64()).unwrap_or(0.0);

        let time_obj = obj.get("time");
        let ts_ms = time_obj
            .and_then(|t| t.get("completed"))
            .and_then(|v| v.as_i64())
            .or_else(|| {
                time_obj
                    .and_then(|t| t.get("created"))
                    .and_then(|v| v.as_i64())
            })
            .unwrap_or(0);

        let (date, time) = if ts_ms > 0 {
            let secs = ts_ms / 1000;
            let dt = Utc.timestamp_opt(secs, 0).single();
            match dt {
                Some(dt) => (dt.format("%Y-%m-%d").to_string(), dt.to_rfc3339()),
                None => ("unknown".to_string(), "unknown".to_string()),
            }
        } else {
            ("unknown".to_string(), "unknown".to_string())
        };

        Some(TokenRecord {
            date,
            time,
            api_key_prefix: "N/A".to_string(),
            provider,
            model,
            source: "opencode".to_string(),
            input_tokens,
            output_tokens: effective_output,
            cache_read_tokens,
            cache_write_tokens,
            total_tokens,
            cost,
        })
    }
}
