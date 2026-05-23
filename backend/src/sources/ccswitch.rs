use super::DataSource;
use crate::models::TokenRecord;
use chrono::{TimeZone, Utc};
use std::collections::HashMap;
use std::path::PathBuf;

/// Legacy ccswitch source: reads `~/.cc-switch/cc-switch.db` (SQLite).
///
/// Only loaded when `USE_CC_SWITCH` env var is set.
#[derive(Default)]
pub struct CcSwitchSource;

impl DataSource for CcSwitchSource {
    fn name(&self) -> &'static str {
        "ccswitch"
    }

    fn load(&self) -> Vec<TokenRecord> {
        let path = Self::db_path();
        tracing::info!("Loading ccswitch data from: {:?}", path);
        let records = Self::parse(&path);
        tracing::info!("Loaded {} ccswitch records", records.len());
        records
    }
}

impl CcSwitchSource {
    fn db_path() -> PathBuf {
        let override_path = std::env::var("CCSWITCH_DB_PATH").ok();
        override_path
            .map(PathBuf::from)
            .unwrap_or_else(|| super::home_dir().join(".cc-switch").join("cc-switch.db"))
    }

    /// Query the cc-switch DB for the currently active provider name
    /// for a given `app_type` (e.g. "claude", "codex").
    ///
    /// Returns `None` if the DB is missing, cannot be opened, or no
    /// active provider is found for the given app_type.
    pub fn get_active_provider(app_type: &str) -> Option<String> {
        let path = Self::db_path();
        if !path.exists() {
            return None;
        }

        let conn = rusqlite::Connection::open_with_flags(
            &path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .ok()?;

        let mut stmt = conn
            .prepare("SELECT name FROM providers WHERE app_type = ?1 AND is_current = 1")
            .ok()?;

        let name: String = stmt.query_row([app_type], |row| row.get(0)).ok()?;
        Some(name)
    }

    fn parse(path: &std::path::Path) -> Vec<TokenRecord> {
        if !path.exists() {
            tracing::warn!("ccswitch DB not found at {:?}, skipping", path);
            return Vec::new();
        }

        let conn = match rusqlite::Connection::open_with_flags(
            path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        ) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("Failed to open ccswitch DB: {}, skipping", e);
                return Vec::new();
            }
        };

        // Build provider_id → name mapping
        let mut provider_names: HashMap<String, String> = HashMap::new();
        // Build app_type → active provider name mapping (is_current = 1)
        let mut active_provider_by_app: HashMap<String, String> = HashMap::new();
        {
            let mut stmt =
                match conn.prepare("SELECT id, name, app_type, is_current FROM providers") {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::warn!("Failed to query providers: {}", e);
                        return Vec::new();
                    }
                };
            let rows = stmt.query_map([], |row| {
                let id: String = row.get(0)?;
                let name: String = row.get(1)?;
                let app_type: String = row.get(2)?;
                let is_current: bool = row.get(3)?;
                Ok((id, name, app_type, is_current))
            });
            match rows {
                Ok(r) => {
                    for (id, name, app_type, is_current) in r.flatten() {
                        provider_names.insert(id.clone(), name.clone());
                        if is_current {
                            active_provider_by_app.insert(app_type, name);
                        }
                    }
                }
                Err(e) => tracing::warn!("Failed to iterate providers: {}", e),
            }
        }

        tracing::info!("cc-switch active providers: {:?}", active_provider_by_app);

        let mut records = Vec::new();
        let sql = "SELECT request_id, provider_id, model, request_model, \
                   input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens, \
                   total_cost_usd, created_at, data_source, session_id \
                   FROM proxy_request_logs WHERE status_code = 200";

        let mut stmt = match conn.prepare(sql) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("Failed to prepare query: {}", e);
                return records;
            }
        };

        let rows = stmt.query_map([], |row| {
            let request_id: String = row.get(0)?;
            let provider_id: String = row.get(1)?;
            let model: String = row.get(2)?;
            let request_model: String = row.get(3)?;
            let input_tokens: i64 = row.get(4)?;
            let output_tokens: i64 = row.get(5)?;
            let cache_read_tokens: i64 = row.get(6)?;
            let cache_creation_tokens: i64 = row.get(7)?;
            let total_cost_usd: String = row.get(8)?;
            let created_at: i64 = row.get(9)?;
            let data_source: String = row.get(10)?;
            let session_id: Option<String> = row.get(11)?;
            Ok((
                request_id,
                provider_id,
                model,
                request_model,
                input_tokens,
                output_tokens,
                cache_read_tokens,
                cache_creation_tokens,
                total_cost_usd,
                created_at,
                data_source,
                session_id,
            ))
        });

        match rows {
            Ok(r) => {
                for (
                    _request_id,
                    provider_id,
                    model,
                    request_model,
                    input_tokens,
                    output_tokens,
                    cache_read_tokens,
                    cache_creation_tokens,
                    total_cost_usd,
                    created_at,
                    data_source,
                    _session_id,
                ) in r.flatten()
                {
                    let source = match data_source.as_str() {
                        "session_log" => "claude-code",
                        "codex_session" => "codex",
                        other => other,
                    }
                    .to_string();

                    let app_type_for_lookup = match data_source.as_str() {
                        "session_log" => "claude",
                        "codex_session" => "codex",
                        _ => "claude",
                    };

                    let provider =
                        if provider_id.starts_with("_session") || provider_id == "_codex_session" {
                            // For session-based entries, the provider_id is generic ("_session").
                            // Try the currently active provider for this app_type first,
                            // falling back to model-based resolution.
                            active_provider_by_app
                                .get(app_type_for_lookup)
                                .cloned()
                                .unwrap_or_else(|| super::resolve_provider_from_model(&model))
                        } else {
                            provider_names
                                .get(&provider_id)
                                .cloned()
                                .unwrap_or(provider_id)
                        };

                    let dt = Utc.timestamp_opt(created_at, 0).single();
                    let (date, time) = match dt {
                        Some(dt) => (dt.format("%Y-%m-%d").to_string(), dt.to_rfc3339()),
                        None => ("unknown".to_string(), "unknown".to_string()),
                    };

                    // Normalize cache semantics
                    let (effective_input, effective_cache_read) = if data_source == "codex_session"
                    {
                        let non_cached = (input_tokens - cache_read_tokens).max(0);
                        (non_cached, cache_read_tokens)
                    } else {
                        (input_tokens, cache_read_tokens)
                    };

                    let cost: f64 = total_cost_usd.parse().unwrap_or(0.0);

                    records.push(TokenRecord {
                        date,
                        time,
                        api_key_prefix: "N/A".to_string(),
                        provider,
                        original_provider: None,
                        model: request_model,
                        source,
                        input_tokens: effective_input,
                        output_tokens,
                        cache_read_tokens: effective_cache_read,
                        cache_write_tokens: cache_creation_tokens,
                        total_tokens: effective_input
                            + output_tokens
                            + effective_cache_read
                            + cache_creation_tokens,
                        cost,
                    });
                }
            }
            Err(e) => tracing::warn!("Failed to iterate request logs: {}", e),
        }

        records
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_provider_for_session_with_active_provider() {
        // Simulate: active_provider_by_app = {"claude": "FreeModel"}
        let mut active_provider_by_app: HashMap<String, String> = HashMap::new();
        active_provider_by_app.insert("claude".to_string(), "FreeModel".to_string());

        let model = "claude-opus-4-7";
        let provider_id = "_session";
        let data_source = "session_log";

        let app_type_for_lookup = match data_source {
            "session_log" => "claude",
            "codex_session" => "codex",
            _ => "claude",
        };

        let provider = if provider_id.starts_with("_session") || provider_id == "_codex_session" {
            active_provider_by_app
                .get(app_type_for_lookup)
                .cloned()
                .unwrap_or_else(|| super::super::resolve_provider_from_model(model))
        } else {
            "fallback".to_string()
        };

        assert_eq!(provider, "FreeModel");
    }

    #[test]
    fn test_resolve_provider_for_session_without_active_provider() {
        // No active provider: fall back to resolve_provider_from_model
        let active_provider_by_app: HashMap<String, String> = HashMap::new();

        let model = "claude-opus-4-7";
        let provider_id = "_session";
        let data_source = "session_log";

        let app_type_for_lookup = match data_source {
            "session_log" => "claude",
            "codex_session" => "codex",
            _ => "claude",
        };

        let provider = if provider_id.starts_with("_session") || provider_id == "_codex_session" {
            active_provider_by_app
                .get(app_type_for_lookup)
                .cloned()
                .unwrap_or_else(|| super::super::resolve_provider_from_model(model))
        } else {
            "fallback".to_string()
        };

        assert_eq!(provider, "anthropic");
    }
}
