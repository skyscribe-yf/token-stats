use crate::models::TokenRecord;
use chrono::{Utc, TimeZone};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

// ─── Pi source ───

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
            // Pi JSONL already has the right field names via serde rename,
            // but lacks "source". We inject it here.
            if let Ok(mut record) = serde_json::from_str::<TokenRecord>(&line) {
                if record.source.is_empty() {
                    record.source = "pi".to_string();
                }
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

// ─── ccswitch source ───

pub fn get_ccswitch_db_path() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let override_path = std::env::var("CCSWITCH_DB_PATH").ok();
    override_path.map(std::path::PathBuf::from).unwrap_or_else(|| {
        std::path::PathBuf::from(home).join(".cc-switch").join("cc-switch.db")
    })
}

pub fn parse_ccswitch_db<P: AsRef<Path>>(path: P) -> Vec<TokenRecord> {
    let path = path.as_ref();
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

    // Build a provider_id → name mapping from the providers table
    let mut provider_names: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    {
        let mut stmt = match conn.prepare("SELECT id, name FROM providers WHERE app_type = 'claude'") {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("Failed to query providers: {}", e);
                return Vec::new();
            }
        };
        let rows = stmt.query_map([], |row| {
            let id: String = row.get(0)?;
            let name: String = row.get(1)?;
            Ok((id, name))
        });
        match rows {
            Ok(r) => {
                for item in r {
                    if let Ok((id, name)) = item {
                        provider_names.insert(id, name);
                    }
                }
            }
            Err(e) => tracing::warn!("Failed to iterate providers: {}", e),
        }
    }

    // Query proxy_request_logs with JOIN to get provider name
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
        Ok((request_id, provider_id, model, request_model, input_tokens, output_tokens,
            cache_read_tokens, cache_creation_tokens, total_cost_usd, created_at,
            data_source, session_id))
    });

    match rows {
        Ok(r) => {
            for item in r {
                if let Ok((_request_id, provider_id, model, request_model,
                           input_tokens, output_tokens, cache_read_tokens,
                           cache_creation_tokens, total_cost_usd, created_at,
                           data_source, _session_id)) = item {

                    // Determine source from data_source
                    let source = match data_source.as_str() {
                        "session_log" => "claude-code",
                        "codex_session" => "codex",
                        other => other, // "proxy" or unknown
                    }.to_string();

                    // Determine provider name
                    // For _session / _codex_session provider_ids, use the model as provider hint
                    let provider = if provider_id.starts_with("_session") || provider_id == "_codex_session" {
                        // These are session-log entries; the provider_id is synthetic.
                        // Use the model name as provider hint (e.g., "deepseek-v4-pro" → "DeepSeek V4 Pro")
                        // Or look up from providers table by matching model to provider settings
                        resolve_provider_from_model(&model, &provider_names)
                    } else {
                        provider_names.get(&provider_id).cloned().unwrap_or(provider_id)
                    };

                    // Parse timestamp
                    let dt = Utc.timestamp_opt(created_at, 0).single();
                    let (date, time) = match dt {
                        Some(dt) => {
                            (dt.format("%Y-%m-%d").to_string(), dt.to_rfc3339())
                        }
                        None => {
                            ("unknown".to_string(), "unknown".to_string())
                        }
                    };

                    let cost: f64 = total_cost_usd.parse().unwrap_or(0.0);

                    records.push(TokenRecord {
                        date,
                        time,
                        api_key_prefix: "N/A".to_string(),
                        provider,
                        model: request_model, // request_model is what was actually requested
                        source: source.to_string(),
                        input_tokens,
                        output_tokens,
                        cache_read_tokens,
                        cache_write_tokens: cache_creation_tokens,
                        total_tokens: input_tokens + output_tokens + cache_read_tokens + cache_creation_tokens,
                        cost,
                    });
                }
            }
        }
        Err(e) => tracing::warn!("Failed to iterate request logs: {}", e),
    }

    tracing::info!("Loaded {} records from ccswitch DB", records.len());
    records
}

/// Try to resolve a human-readable provider name from the model name
/// by checking provider settings_config for ANTHROPIC_MODEL mappings
fn resolve_provider_from_model(
    model: &str,
    _provider_names: &std::collections::HashMap<String, String>,
) -> String {
    // Known model → provider mappings based on ccswitch provider configs
    match model {
        "kimi-for-coding" | "kimi-k2.6" | "kimi-k2.5" => "kimi".to_string(),
        "astron-code-latest" => "xunfei".to_string(),
        "mimo-v2.5-pro" | "mimo-v2-pro" | "mimo-v2.5" => "xiaomi-mimo".to_string(),
        "deepseek-v4-pro" | "deepseek-v4-flash" => "deepseek".to_string(),
        "gpt-5.5" | "gpt-5.4" | "gpt-5.4-mini" => "openai".to_string(),
        _ => model.to_string(), // fallback: use model name as provider
    }
}

// ─── Kimi CLI source ───

pub fn get_kimi_sessions_path() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let override_path = std::env::var("KIMI_SESSIONS_PATH").ok();
    override_path.map(std::path::PathBuf::from).unwrap_or_else(|| {
        std::path::PathBuf::from(home).join(".kimi").join("sessions")
    })
}

pub fn parse_kimi_sessions<P: AsRef<Path>>(base_path: P) -> Vec<TokenRecord> {
    let base = base_path.as_ref();
    if !base.exists() {
        tracing::warn!("Kimi sessions dir not found at {:?}, skipping", base);
        return Vec::new();
    }

    let mut records = Vec::new();

    // Walk all wire.jsonl files under the sessions directory
    if let Ok(entries) = walkdir(base) {
        for wire_path in entries {
            if !wire_path.to_string_lossy().ends_with("wire.jsonl") {
                continue;
            }

            let session_dir = wire_path.parent().unwrap_or(base);
            // Try to read model from state.json in the same directory
            let model = read_kimi_session_model(session_dir);

            let file = match File::open(&wire_path) {
                Ok(f) => f,
                Err(_) => continue,
            };

            let reader = BufReader::new(file);
            for line in reader.lines() {
                if let Ok(line) = line {
                    if line.trim().is_empty() {
                        continue;
                    }
                    if let Ok(msg) = serde_json::from_str::<serde_json::Value>(&line) {
                        let message = msg.get("message");
                        if let Some(message) = message {
                            if message.get("type").and_then(|t| t.as_str()) == Some("StatusUpdate") {
                                let payload = message.get("payload");
                                if let Some(payload) = payload {
                                    let token_usage = payload.get("token_usage");
                                    if let Some(usage) = token_usage {
                                        let input_other = usage.get("input_other").and_then(|v| v.as_i64()).unwrap_or(0);
                                        let output = usage.get("output").and_then(|v| v.as_i64()).unwrap_or(0);
                                        let cache_read = usage.get("input_cache_read").and_then(|v| v.as_i64()).unwrap_or(0);
                                        let cache_creation = usage.get("input_cache_creation").and_then(|v| v.as_i64()).unwrap_or(0);

                                        let timestamp = msg.get("timestamp").and_then(|v| v.as_f64()).unwrap_or(0.0);
                                        let secs = timestamp as i64;
                                        let dt = Utc.timestamp_opt(secs, 0).single();
                                        let (date, time) = match dt {
                                            Some(dt) => (dt.format("%Y-%m-%d").to_string(), dt.to_rfc3339()),
                                            None => ("unknown".to_string(), "unknown".to_string()),
                                        };

                                        let total = input_other + output + cache_read + cache_creation;

                                        records.push(TokenRecord {
                                            date,
                                            time,
                                            api_key_prefix: "N/A".to_string(),
                                            provider: "kimi".to_string(),
                                            model: model.clone(),
                                            source: "kimi-cli".to_string(),
                                            input_tokens: input_other,
                                            output_tokens: output,
                                            cache_read_tokens: cache_read,
                                            cache_write_tokens: cache_creation,
                                            total_tokens: total,
                                            cost: 0.0, // Kimi CLI doesn't provide cost data
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    tracing::info!("Loaded {} records from Kimi CLI sessions", records.len());
    records
}

/// Read model name from a Kimi session's state.json
fn read_kimi_session_model(session_dir: &Path) -> String {
    let state_path = session_dir.join("state.json");
    if let Ok(file) = File::open(&state_path) {
        let reader = BufReader::new(file);
        if let Ok(state) = serde_json::from_reader::<_, serde_json::Value>(reader) {
            // Try to get model from state
            if let Some(model) = state.get("model").and_then(|m| m.as_str()) {
                return model.to_string();
            }
            // Try config.model
            if let Some(config) = state.get("config") {
                if let Some(model) = config.get("default_model").and_then(|m| m.as_str()) {
                    return model.to_string();
                }
            }
        }
    }
    "kimi-for-coding".to_string() // fallback
}

/// Simple recursive directory walker to find all files
fn walkdir(path: &Path) -> Result<Vec<std::path::PathBuf>, std::io::Error> {
    let mut result = Vec::new();
    walkdir_recursive(path, &mut result)?;
    Ok(result)
}

fn walkdir_recursive(path: &Path, result: &mut Vec<std::path::PathBuf>) -> Result<(), std::io::Error> {
    if path.is_dir() {
        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                walkdir_recursive(&path, result)?;
            } else {
                result.push(path);
            }
        }
    }
    Ok(())
}

// ─── Load all sources ───

pub fn load_all_sources() -> Vec<TokenRecord> {
    let mut all_records = Vec::new();

    // Pi
    let pi_path = get_log_path();
    tracing::info!("Loading pi data from: {:?}", pi_path);
    let pi_records = parse_jsonl_file(&pi_path);
    tracing::info!("Loaded {} pi records", pi_records.len());
    all_records.extend(pi_records);

    // ccswitch (Claude Code + Codex)
    let ccswitch_path = get_ccswitch_db_path();
    tracing::info!("Loading ccswitch data from: {:?}", ccswitch_path);
    let ccswitch_records = parse_ccswitch_db(&ccswitch_path);
    all_records.extend(ccswitch_records);

    // Kimi CLI
    let kimi_path = get_kimi_sessions_path();
    tracing::info!("Loading Kimi CLI data from: {:?}", kimi_path);
    let kimi_records = parse_kimi_sessions(&kimi_path);
    all_records.extend(kimi_records);

    tracing::info!("Total records across all sources: {}", all_records.len());
    all_records
}