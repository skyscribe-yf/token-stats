use crate::config;
use crate::models::TokenRecord;
use chrono::{Utc, TimeZone, DateTime};
use std::collections::HashSet;
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

                    // ── Cache semantics differ by API ──
                    // OpenAI (Codex): input_tokens INCLUDES cache_read_tokens (cached is a subset)
                    // Anthropic (Claude Code): input_tokens does NOT include cache tokens (separate)
                    //
                    // We normalize to the Anthropic convention where input_tokens = non-cached input only,
                    // so that total_tokens = input + output + cache_read + cache_write is never double-counted.
                    let (effective_input, effective_cache_read) = if data_source == "codex_session" {
                        // OpenAI: input_tokens includes cached; subtract to get non-cached input
                        let non_cached = (input_tokens - cache_read_tokens).max(0);
                        (non_cached, cache_read_tokens)
                    } else {
                        // Anthropic / others: input_tokens already excludes cache
                        (input_tokens, cache_read_tokens)
                    };

                    let cost: f64 = total_cost_usd.parse().unwrap_or(0.0);

                    records.push(TokenRecord {
                        date,
                        time,
                        api_key_prefix: "N/A".to_string(),
                        provider,
                        model: request_model, // request_model is what was actually requested
                        source: source.to_string(),
                        input_tokens: effective_input,
                        output_tokens,
                        cache_read_tokens: effective_cache_read,
                        cache_write_tokens: cache_creation_tokens,
                        total_tokens: effective_input + output_tokens + effective_cache_read + cache_creation_tokens,
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
        "glm-5.1" => "opencode-go".to_string(),
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

// ─── Codex direct source ───

pub fn get_codex_sessions_path() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    std::path::PathBuf::from(home).join(".codex").join("sessions")
}

pub fn parse_codex_sessions<P: AsRef<Path>>(base_path: P) -> Vec<TokenRecord> {
    let base = base_path.as_ref();
    if !base.exists() {
        tracing::warn!("Codex sessions dir not found at {:?}, skipping", base);
        return Vec::new();
    }

    let mut records = Vec::new();

    if let Ok(entries) = walkdir(base) {
        for path in entries {
            if !path.to_string_lossy().ends_with(".jsonl") {
                continue;
            }
            if !path.file_name().unwrap_or_default().to_string_lossy().starts_with("rollout-") {
                continue;
            }

            let file = match File::open(&path) {
                Ok(f) => f,
                Err(_) => continue,
            };

            // First pass: find model from turn_context
            let mut session_model = "gpt-5.5".to_string();
            {
                let reader = BufReader::new(&file);
                for line in reader.lines() {
                    if let Ok(line) = line {
                        if line.trim().is_empty() { continue; }
                        if let Ok(obj) = serde_json::from_str::<serde_json::Value>(&line) {
                            if obj.get("type").and_then(|t| t.as_str()) == Some("turn_context") {
                                if let Some(model) = obj.get("payload").and_then(|p| p.get("model")).and_then(|m| m.as_str()) {
                                    session_model = model.to_string();
                                    break;
                                }
                            }
                        }
                    }
                }
            }

            // Second pass: collect token_count events
            let file = match File::open(&path) {
                Ok(f) => f,
                Err(_) => continue,
            };
            let reader = BufReader::new(file);
            for line in reader.lines() {
                if let Ok(line) = line {
                    if line.trim().is_empty() { continue; }
                    if let Ok(obj) = serde_json::from_str::<serde_json::Value>(&line) {
                        if obj.get("type").and_then(|t| t.as_str()) != Some("event_msg") {
                            continue;
                        }
                        let payload = obj.get("payload");
                        if payload.is_none() { continue; }
                        let payload = payload.unwrap();
                        if payload.get("type").and_then(|t| t.as_str()) != Some("token_count") {
                            continue;
                        }
                        let info = payload.get("info");
                        if info.is_none() || info.unwrap().is_null() { continue; }
                        let last_usage = info.unwrap().get("last_token_usage");
                        if last_usage.is_none() { continue; }
                        let usage = last_usage.unwrap();

                        let input_tokens = usage.get("input_tokens").and_then(|v| v.as_i64()).unwrap_or(0);
                        let cached_input_tokens = usage.get("cached_input_tokens").and_then(|v| v.as_i64()).unwrap_or(0);
                        let output_tokens = usage.get("output_tokens").and_then(|v| v.as_i64()).unwrap_or(0);
                        // reasoning_output_tokens is part of output_tokens, ignore for totals

                        // OpenAI convention: input_tokens includes cache; normalize
                        let effective_input = (input_tokens - cached_input_tokens).max(0);

                        let ts_str = obj.get("timestamp").and_then(|t| t.as_str()).unwrap_or("");
                        let (date, time) = parse_iso_timestamp(ts_str);

                        records.push(TokenRecord {
                            date,
                            time,
                            api_key_prefix: "N/A".to_string(),
                            provider: "openai".to_string(),
                            model: session_model.clone(),
                            source: "codex".to_string(),
                            input_tokens: effective_input,
                            output_tokens,
                            cache_read_tokens: cached_input_tokens,
                            cache_write_tokens: 0,
                            total_tokens: effective_input + output_tokens + cached_input_tokens,
                            cost: 0.0,
                        });
                    }
                }
            }
        }
    }

    tracing::info!("Loaded {} records from Codex sessions", records.len());
    records
}

fn parse_iso_timestamp(ts: &str) -> (String, String) {
    match DateTime::parse_from_rfc3339(ts) {
        Ok(dt) => {
            let utc = dt.with_timezone(&Utc);
            (utc.format("%Y-%m-%d").to_string(), utc.to_rfc3339())
        }
        Err(_) => ("unknown".to_string(), "unknown".to_string()),
    }
}

// ─── Claude Code direct source ───

pub fn get_claude_code_projects_path() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    std::path::PathBuf::from(home).join(".claude").join("projects")
}

pub fn parse_claude_code_sessions<P: AsRef<Path>>(base_path: P) -> Vec<TokenRecord> {
    let base = base_path.as_ref();
    if !base.exists() {
        tracing::warn!("Claude Code projects dir not found at {:?}, skipping", base);
        return Vec::new();
    }

    let mut records = Vec::new();
    let mut seen: HashSet<(String, String)> = HashSet::new();

    if let Ok(entries) = walkdir(base) {
        for path in entries {
            if !path.to_string_lossy().ends_with(".jsonl") {
                continue;
            }

            let file = match File::open(&path) {
                Ok(f) => f,
                Err(_) => continue,
            };

            let reader = BufReader::new(file);
            for line in reader.lines() {
                if let Ok(line) = line {
                    if line.trim().is_empty() { continue; }
                    if let Ok(obj) = serde_json::from_str::<serde_json::Value>(&line) {
                        if obj.get("type").and_then(|t| t.as_str()) != Some("assistant") {
                            continue;
                        }
                        let session_id = obj.get("sessionId").and_then(|v| v.as_str()).unwrap_or("").to_string();
                        let msg = obj.get("message");
                        if msg.is_none() { continue; }
                        let msg = msg.unwrap();
                        let msg_id = msg.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();

                        let key = (session_id.clone(), msg_id);
                        if seen.contains(&key) {
                            continue;
                        }

                        let usage = msg.get("usage");
                        if usage.is_none() { continue; }
                        let usage = usage.unwrap();
                        let input_tokens = usage.get("input_tokens").and_then(|v| v.as_i64()).unwrap_or(0);
                        let output_tokens = usage.get("output_tokens").and_then(|v| v.as_i64()).unwrap_or(0);
                        let cache_read_tokens = usage.get("cache_read_input_tokens").and_then(|v| v.as_i64()).unwrap_or(0);
                        let cache_write_tokens = usage.get("cache_creation_input_tokens").and_then(|v| v.as_i64()).unwrap_or(0);

                        // Filter out intermediate streaming snapshots with zero usage
                        if input_tokens == 0 && output_tokens == 0 && cache_read_tokens == 0 && cache_write_tokens == 0 {
                            continue;
                        }

                        seen.insert(key);

                        let model = msg.get("model").and_then(|v| v.as_str()).unwrap_or("unknown").to_string();
                        let provider = resolve_provider_from_model(&model, &std::collections::HashMap::new());

                        let ts_str = obj.get("timestamp").and_then(|t| t.as_str()).unwrap_or("");
                        let (date, time) = parse_iso_timestamp(ts_str);

                        records.push(TokenRecord {
                            date,
                            time,
                            api_key_prefix: "N/A".to_string(),
                            provider,
                            model,
                            source: "claude-code".to_string(),
                            input_tokens,
                            output_tokens,
                            cache_read_tokens,
                            cache_write_tokens,
                            total_tokens: input_tokens + output_tokens + cache_read_tokens + cache_write_tokens,
                            cost: 0.0,
                        });
                    }
                }
            }
        }
    }

    tracing::info!("Loaded {} records from Claude Code sessions", records.len());
    records
}

// ─── OpenCode source ───

pub fn get_opencode_db_path() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    std::path::PathBuf::from(home).join(".local").join("share").join("opencode").join("opencode.db")
}

pub fn parse_opencode_db<P: AsRef<Path>>(path: P) -> Vec<TokenRecord> {
    let path = path.as_ref();
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
            for item in r {
                if let Ok(data) = item {
                    if let Ok(obj) = serde_json::from_str::<serde_json::Value>(&data) {
                        // Only assistant messages with token usage
                        if obj.get("role").and_then(|v| v.as_str()) != Some("assistant") {
                            continue;
                        }
                        let tokens = obj.get("tokens");
                        if tokens.is_none() || tokens.unwrap().is_null() {
                            continue;
                        }
                        let tokens = tokens.unwrap();
                        let input_tokens = tokens.get("input").and_then(|v| v.as_i64()).unwrap_or(0);
                        let output_tokens = tokens.get("output").and_then(|v| v.as_i64()).unwrap_or(0);
                        let reasoning_tokens = tokens.get("reasoning").and_then(|v| v.as_i64()).unwrap_or(0);
                        let cache = tokens.get("cache");
                        let cache_read_tokens = cache.and_then(|c| c.get("read")).and_then(|v| v.as_i64()).unwrap_or(0);
                        let cache_write_tokens = cache.and_then(|c| c.get("write")).and_then(|v| v.as_i64()).unwrap_or(0);
                        let total_tokens = tokens.get("total").and_then(|v| v.as_i64()).unwrap_or(0);

                        // Filter out zero-usage records (intermediate streaming states)
                        if input_tokens == 0 && output_tokens == 0 && cache_read_tokens == 0 && cache_write_tokens == 0 {
                            continue;
                        }

                        // reasoning is part of output
                        let effective_output = output_tokens + reasoning_tokens;

                        let model = obj.get("modelID").and_then(|v| v.as_str()).unwrap_or("unknown").to_string();
                        let mut provider = obj.get("providerID").and_then(|v| v.as_str()).unwrap_or("unknown").to_string();
                        // Normalize opencode → opencode-go for consistency with existing data
                        if provider == "opencode" {
                            provider = "opencode-go".to_string();
                        }
                        // Fallback to model-based resolution if provider is missing/generic
                        if provider == "unknown" || provider.is_empty() {
                            provider = resolve_provider_from_model(&model, &std::collections::HashMap::new());
                        }

                        let cost = obj.get("cost").and_then(|v| v.as_f64()).unwrap_or(0.0);

                        let time_obj = obj.get("time");
                        let ts_ms = time_obj
                            .and_then(|t| t.get("completed"))
                            .and_then(|v| v.as_i64())
                            .or_else(|| time_obj.and_then(|t| t.get("created")).and_then(|v| v.as_i64()))
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

                        records.push(TokenRecord {
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
                        });
                    }
                }
            }
        }
        Err(e) => tracing::warn!("Failed to iterate OpenCode messages: {}", e),
    }

    tracing::info!("Loaded {} records from OpenCode DB", records.len());
    records
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

    // Codex (direct from session JSONL)
    let codex_path = get_codex_sessions_path();
    tracing::info!("Loading Codex data from: {:?}", codex_path);
    let codex_records = parse_codex_sessions(&codex_path);
    tracing::info!("Loaded {} codex records", codex_records.len());
    all_records.extend(codex_records);

    // Claude Code (direct from project JSONL)
    let claude_path = get_claude_code_projects_path();
    tracing::info!("Loading Claude Code data from: {:?}", claude_path);
    let claude_records = parse_claude_code_sessions(&claude_path);
    tracing::info!("Loaded {} claude-code records", claude_records.len());
    all_records.extend(claude_records);

    // OpenCode
    let opencode_path = get_opencode_db_path();
    tracing::info!("Loading OpenCode data from: {:?}", opencode_path);
    let opencode_records = parse_opencode_db(&opencode_path);
    tracing::info!("Loaded {} opencode records", opencode_records.len());
    all_records.extend(opencode_records);

    // ccswitch (fallback, only if env var explicitly set)
    if std::env::var("USE_CC_SWITCH").is_ok() {
        let ccswitch_path = get_ccswitch_db_path();
        tracing::info!("Loading ccswitch data from: {:?}", ccswitch_path);
        let ccswitch_records = parse_ccswitch_db(&ccswitch_path);
        all_records.extend(ccswitch_records);
    }

    // Kimi CLI
    let kimi_path = get_kimi_sessions_path();
    tracing::info!("Loading Kimi CLI data from: {:?}", kimi_path);
    let kimi_records = parse_kimi_sessions(&kimi_path);
    all_records.extend(kimi_records);

    tracing::info!("Total records across all sources: {}", all_records.len());

    // Apply vendor merging from config
    let merge_config_path = config::get_vendor_merge_config_path();
    if let Some(merge_map) = config::load_vendor_merge_map(&merge_config_path) {
        config::apply_vendor_merge(&mut all_records, &merge_map);
    }

    all_records
}