use super::DataSource;
use crate::models::TokenRecord;
use chrono::{TimeZone, Utc};
use std::collections::HashMap;
use std::path::PathBuf;

/// ZCode source: reads `~/.zcode/cli/db/db.sqlite` (SQLite) `model_usage` table.
///
/// ZCode records every completed model request in the `model_usage` table
/// (one row per attempt, keyed by a unique `id`). Token counts use the
/// OpenAI convention — `input_tokens` includes `cache_read_input_tokens`
/// and `cache_creation_input_tokens` — so the parser subtracts them to match
/// the Anthropic convention used everywhere else (same as the Codex parser).
///
/// Billing provider comes from `provider_metadata_json` (e.g.
/// `{"OpenCodeGo": {}}` / `{"Tokenrouter": {}}`), which names the provider
/// that billed the request. Records are tagged with that provider so
/// display_cost() applies the right formula (opencode-go → plan divisor,
/// tokenrouter → free/listed pricing).
#[derive(Default)]
pub struct ZcodeSource;

impl DataSource for ZcodeSource {
    fn name(&self) -> &'static str {
        "zcode"
    }

    fn load(&self) -> Vec<TokenRecord> {
        let path = Self::db_path();
        tracing::info!("Loading ZCode data from: {:?}", path);
        let records = Self::parse(&path);
        tracing::info!("Loaded {} zcode records", records.len());
        records
    }

    /// Incremental: skip entirely when the DB file (mtime, size) is unchanged.
    fn load_incremental(&self) -> Vec<TokenRecord> {
        let path = Self::db_path();
        let files = vec![path.clone()];
        if self.changed_data_files().is_empty() {
            return Vec::new();
        }
        let records = Self::parse(&path);
        self.mark_files_parsed(&files);
        records
    }

    fn data_files(&self) -> Vec<std::path::PathBuf> {
        Self::with_wal_sidecar(Self::db_path())
    }

    fn is_available(&self) -> bool {
        Self::db_path().exists()
    }
}

/// Map a billing-provider name from `provider_metadata_json` to the
/// canonical provider tag used by the dashboard.
fn normalize_billing_provider(name: &str) -> String {
    match name {
        "OpenCodeGo" => "opencode-go".to_string(),
        other => other.to_lowercase(),
    }
}

/// Build provider_id → billing provider from rows whose
/// `provider_metadata_json` names the billing provider (e.g.
/// `{"OpenCodeGo":{}}`, `{"Tokenrouter":{}}`). Most rows only carry
/// `{"rawFinishReason":...}` or empty metadata, so the map is built from
/// the few rows that name the provider.
fn billing_provider_map(conn: &rusqlite::Connection) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let sql = "SELECT provider_id, provider_metadata_json FROM model_usage
               WHERE provider_metadata_json IS NOT NULL AND provider_metadata_json != ''";
    if let Ok(mut stmt) = conn.prepare(sql) {
        if let Ok(rows) = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        }) {
            for row in rows.flatten() {
                let (pid, meta) = row;
                if let Ok(serde_json::Value::Object(fields)) =
                    serde_json::from_str::<serde_json::Value>(&meta)
                {
                    for key in fields.keys() {
                        if key != "rawFinishReason" {
                            map.entry(pid.clone())
                                .or_insert_with(|| normalize_billing_provider(key));
                        }
                    }
                }
            }
        }
    }
    map
}

impl ZcodeSource {
    fn db_path() -> PathBuf {
        std::env::var("ZCODE_DB_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                super::home_dir()
                    .join(".zcode")
                    .join("cli")
                    .join("db")
                    .join("db.sqlite")
            })
    }

    fn parse(path: &std::path::Path) -> Vec<TokenRecord> {
        if !path.exists() {
            tracing::warn!("ZCode DB not found at {:?}, skipping", path);
            return Vec::new();
        }

        let conn = match rusqlite::Connection::open_with_flags(
            path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        ) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("Failed to open ZCode DB: {}, skipping", e);
                return Vec::new();
            }
        };

        // One row per completed model request (retries are separate rows with
        // their own attempt). Skip non-completed statuses (running / error /
        // cancelled) and zero-usage rows (intermediate streaming states).
        let billing = billing_provider_map(&conn);
        let sql = "SELECT provider_id, model_id, started_at, completed_at,
                          input_tokens, output_tokens,
                          cache_creation_input_tokens, cache_read_input_tokens,
                          time_to_first_token_ms
                   FROM model_usage
                   WHERE status = 'completed'
                     AND (input_tokens > 0 OR output_tokens > 0
                          OR cache_creation_input_tokens > 0 OR cache_read_input_tokens > 0)";

        let mut stmt = match conn.prepare(sql) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("Failed to prepare ZCode query: {}, skipping", e);
                return Vec::new();
            }
        };

        let mut records = Vec::new();
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, Option<i64>>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, i64>(6)?,
                row.get::<_, i64>(7)?,
                row.get::<_, Option<i64>>(8)?,
            ))
        });

        match rows {
            Ok(r) => {
                for row in r.flatten() {
                    let (
                        provider_id,
                        model_id,
                        started_at,
                        completed_at,
                        input_tokens,
                        output_tokens,
                        cache_creation,
                        cache_read,
                        ttft_ms,
                    ) = row;

                    // OpenAI convention: input_tokens includes cache reads and
                    // cache creations; subtract to match the Anthropic convention.
                    let effective_input = (input_tokens - cache_read - cache_creation).max(0);
                    // OpenAI convention: output_tokens already includes
                    // reasoning tokens (a subset breakdown, not additive).
                    let effective_output = output_tokens;

                    // Prefer completion time; fall back to start time.
                    let ts_ms = completed_at.unwrap_or(started_at);
                    let (date, time) = if ts_ms > 0 {
                        let secs = ts_ms / 1000;
                        let nanos = ((ts_ms % 1000) as u32) * 1_000_000;
                        match Utc.timestamp_opt(secs, nanos).single() {
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
                        // Billing provider from provider_metadata_json;
                        // unknown provider_ids keep the historical default.
                        provider: billing
                            .get(&provider_id)
                            .cloned()
                            .unwrap_or_else(|| "opencode-go".to_string()),
                        original_provider: None,
                        model: model_id,
                        source: "zcode".to_string(),
                        input_tokens: effective_input,
                        output_tokens: effective_output,
                        cache_read_tokens: cache_read,
                        cache_write_tokens: cache_creation,
                        total_tokens: effective_input
                            + effective_output
                            + cache_read
                            + cache_creation,
                        cost: 0.0,
                        ttft_ms: ttft_ms.map(|ms| ms as f64),
                        tps: None,
                    });
                }
            }
            Err(e) => tracing::warn!("Failed to iterate ZCode model_usage rows: {}", e),
        }

        records
    }
}

#[cfg(test)]
mod tests {
    use super::ZcodeSource;
    use tempfile::tempdir;

    /// Create a temp zcode-style DB with one completed row per entry.
    /// Each entry is `model|started_at|completed_at|ttft|input|output|cache_creation|cache_read`.
    fn make_db(entries: &[&str]) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("db.sqlite");
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE model_usage (
                id text primary key,
                logical_request_id text not null,
                session_id text not null,
                query_source text not null,
                provider_id text not null,
                model_id text not null,
                status text not null,
                started_at integer not null,
                completed_at integer,
                time_to_first_token_ms integer,
                input_tokens integer not null default 0,
                output_tokens integer not null default 0,
                cache_creation_input_tokens integer not null default 0,
                cache_read_input_tokens integer not null default 0,
                provider_metadata_json text
            );",
        )
        .unwrap();
        for (i, entry) in entries.iter().enumerate() {
            let f: Vec<&str> = entry.split('|').collect();
            conn.execute(
                "INSERT INTO model_usage (id, logical_request_id, session_id,
                    query_source, provider_id, model_id, status, started_at,
                    completed_at, time_to_first_token_ms, input_tokens,
                    output_tokens, cache_creation_input_tokens,
                    cache_read_input_tokens)
                 VALUES (?1, 'req-1', 'sess-1', 'main_turn', 'provider-1',
                         ?2, 'completed', ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                rusqlite::params![
                    format!("id-{i}"),
                    f[0],
                    f[1].parse::<i64>().unwrap(),
                    f[2].parse::<i64>().unwrap(),
                    f[3].parse::<i64>().unwrap(),
                    f[4].parse::<i64>().unwrap(),
                    f[5].parse::<i64>().unwrap(),
                    f[6].parse::<i64>().unwrap(),
                    f[7].parse::<i64>().unwrap(),
                ],
            )
            .unwrap();
        }
        drop(conn);
        (dir, db_path)
    }

    #[test]
    fn parses_completed_requests_and_normalizes_tokens() {
        // Real sample: input=32940 includes cache_read=17664 → effective input 15276
        let (_dir, db_path) =
            make_db(&["deepseek-v4-flash|1786539129536|1786539138916|2479|32940|1048|0|17664"]);
        let records = ZcodeSource::parse(&db_path);

        assert_eq!(records.len(), 1);
        let r = &records[0];
        assert_eq!(r.provider, "opencode-go");
        assert_eq!(r.model, "deepseek-v4-flash");
        assert_eq!(r.source, "zcode");
        assert_eq!(r.input_tokens, 15276);
        assert_eq!(r.output_tokens, 1048);
        assert_eq!(r.cache_read_tokens, 17664);
        assert_eq!(r.cache_write_tokens, 0);
        assert_eq!(r.total_tokens, 33988);
        assert_eq!(r.ttft_ms, Some(2479.0));
        assert_eq!(r.date, "2026-08-12");
    }

    #[test]
    fn subtracts_cache_creation_from_input() {
        // input=10000 includes cache_creation=2000 and cache_read=3000
        let (_dir, db_path) =
            make_db(&["deepseek-v4-flash|1786539125000|1786539129000|100|10000|500|2000|3000"]);
        let records = ZcodeSource::parse(&db_path);

        assert_eq!(records.len(), 1);
        let r = &records[0];
        assert_eq!(r.input_tokens, 5000);
        assert_eq!(r.cache_write_tokens, 2000);
        assert_eq!(r.cache_read_tokens, 3000);
        assert_eq!(r.total_tokens, 5000 + 500 + 2000 + 3000);
    }

    #[test]
    fn skips_non_completed_and_zero_usage_rows() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("db.sqlite");
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE model_usage (
                id text primary key, logical_request_id text not null,
                session_id text not null, query_source text not null,
                provider_id text not null, model_id text not null,
                status text not null, started_at integer not null,
                completed_at integer,
                time_to_first_token_ms integer,
                input_tokens integer not null default 0,
                output_tokens integer not null default 0,
                cache_creation_input_tokens integer not null default 0,
                cache_read_input_tokens integer not null default 0
            );",
        )
        .unwrap();
        for (id, status, input, output) in [
            ("ok", "completed", 100, 20),
            ("err", "error", 500, 0),
            ("zero", "completed", 0, 0),
        ] {
            conn.execute(
                "INSERT INTO model_usage (id, logical_request_id, session_id,
                    query_source, provider_id, model_id, status, started_at,
                    completed_at, time_to_first_token_ms, input_tokens,
                    output_tokens, cache_creation_input_tokens,
                    cache_read_input_tokens)
                 VALUES (?1, 'r', 's', 'main_turn', 'p', 'deepseek-v4-flash',
                         ?2, 1786539125181, 1786539126181, 50, ?3, ?4, 0, 0)",
                rusqlite::params![id, status, input, output],
            )
            .unwrap();
        }
        drop(conn);

        let records = ZcodeSource::parse(&db_path);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].input_tokens, 100);
        assert_eq!(records[0].output_tokens, 20);
        assert_eq!(records[0].total_tokens, 120);
    }

    #[test]
    fn tags_provider_from_metadata() {
        // provider-1 is billed via OpenCodeGo, provider-2 via Tokenrouter.
        // Only a few rows carry the billing metadata; the rest have
        // rawFinishReason or empty metadata.
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("db.sqlite");
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE model_usage (
                id text primary key, logical_request_id text not null,
                session_id text not null, query_source text not null,
                provider_id text not null, model_id text not null,
                status text not null, started_at integer not null,
                completed_at integer,
                time_to_first_token_ms integer,
                input_tokens integer not null default 0,
                output_tokens integer not null default 0,
                cache_creation_input_tokens integer not null default 0,
                cache_read_input_tokens integer not null default 0,
                provider_metadata_json text
            );",
        )
        .unwrap();
        for (id, pid, model, meta) in [
            ("a", "p-1", "deepseek-v4-flash", "{\"OpenCodeGo\":{}}"),
            ("b", "p-1", "deepseek-v4-flash", "{\"rawFinishReason\":\"tool_calls\"}"),
            ("c", "p-2", "z-ai/glm-5.3-free", "{\"Tokenrouter\":{}}"),
            ("d", "p-2", "z-ai/glm-5.3-free", ""),
            ("e", "p-3", "some-model", "{\"rawFinishReason\":\"stop\"}"),
        ] {
            conn.execute(
                "INSERT INTO model_usage (id, logical_request_id, session_id,
                    query_source, provider_id, model_id, status, started_at,
                    completed_at, time_to_first_token_ms, input_tokens,
                    output_tokens, cache_creation_input_tokens,
                    cache_read_input_tokens, provider_metadata_json)
                 VALUES (?1, 'r', 's', 'main_turn', ?2, ?3, 'completed',
                         1786539125181, 1786539126181, 50, 100, 20, 0, 0, ?4)",
                rusqlite::params![id, pid, model, meta],
            )
            .unwrap();
        }
        drop(conn);

        let records = ZcodeSource::parse(&db_path);
        let by_id: std::collections::HashMap<_, _> =
            records.iter().map(|r| (r.model.clone(), r.provider.clone())).collect();
        assert_eq!(by_id["deepseek-v4-flash"], "opencode-go");
        assert_eq!(by_id["z-ai/glm-5.3-free"], "tokenrouter");
        // Provider with no billing metadata keeps the historical default.
        assert_eq!(by_id["some-model"], "opencode-go");
    }

    #[test]
    fn missing_db_returns_empty() {
        let dir = tempdir().unwrap();
        let records = ZcodeSource::parse(&dir.path().join("nope.sqlite"));
        assert!(records.is_empty());
    }
}
