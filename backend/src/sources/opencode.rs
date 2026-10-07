use super::DataSource;
use crate::models::TokenRecord;
use chrono::{TimeZone, Utc};
use rusqlite::{Connection, OptionalExtension};
use std::path::PathBuf;
use std::sync::OnceLock;

/// OpenCode source: reads `~/.local/share/opencode/opencode.db` (SQLite).
///
/// OpenCode 2.x moved per-message records out of `message` (keyed to
/// `session`) into `session_message` (keyed to `session_v2`) and reshaped the
/// `data` JSON on the way:
///
/// | | 1.x (`message`) | 2.x (`session_message`) |
/// |---|---|---|
/// | role | `data.role` | the row's `type` column |
/// | model | `data.modelID` | `data.model.id` |
/// | provider | `data.providerID` | `data.model.providerID` |
/// | total tokens | `data.tokens.total` | absent (must be summed) |
/// | cost | API-reported | always 0 (no longer recorded) |
///
/// The 2.x upgrade copies the 1.x history into `session_message`, so that
/// table is canonical and carries the whole timeline. The legacy `message`
/// table is read **only** for rows the copy left behind (in practice the last
/// session before the upgrade), which keeps the overlap from re-arriving as
/// duplicate fingerprints.
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

        let Some(conn) = Self::open_read_only(path) else {
            return Vec::new();
        };

        let has_v2 = Self::table_exists(&conn, "session_message");
        let mut records = Vec::new();

        // OpenCode 2.x: canonical, including the migrated 1.x history.
        if has_v2 {
            Self::collect(
                &conn,
                "SELECT data FROM session_message WHERE type = 'assistant'",
                true,
                "session_message",
                &mut records,
            );
        }

        // OpenCode 1.x: only the rows 2.x did not copy. Reading the table
        // whole would re-add every migrated message under a different JSON
        // shape (same usage, so the fingerprints collapse — but only for the
        // rows whose migration completed).
        if Self::table_exists(&conn, "message") {
            let sql = if has_v2 {
                "SELECT data FROM message
                 WHERE id NOT IN (SELECT id FROM session_message)"
            } else {
                "SELECT data FROM message"
            };
            Self::collect(&conn, sql, false, "message", &mut records);
        }

        records
    }

    /// Open the OpenCode DB read-only.
    ///
    /// OpenCode keeps writing to it while we read, so a busy timeout is
    /// required: without one, a query landing on an in-flight checkpoint fails
    /// outright and the whole source silently yields nothing (seen in practice
    /// on a machine with an actively streaming session).
    fn open_read_only(path: &std::path::Path) -> Option<Connection> {
        let conn =
            match Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY) {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!("Failed to open OpenCode DB: {}, skipping", e);
                    return None;
                }
            };
        conn.busy_timeout(std::time::Duration::from_secs(5)).ok();
        Some(conn)
    }

    fn table_exists(conn: &Connection, name: &str) -> bool {
        conn.prepare("SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1")
            .and_then(|mut stmt| stmt.exists([name]))
            .unwrap_or(false)
    }

    fn collect(
        conn: &Connection,
        sql: &str,
        is_v2: bool,
        table: &str,
        records: &mut Vec<TokenRecord>,
    ) {
        let mut stmt = match conn.prepare(sql) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("Failed to prepare OpenCode {table} query: {}, skipping", e);
                return;
            }
        };

        let rows = stmt.query_map([], |row| row.get::<_, String>(0));
        match rows {
            Ok(r) => {
                for data in r.flatten() {
                    if let Some(record) = Self::parse_message(&data, is_v2) {
                        records.push(record);
                    }
                }
            }
            Err(e) => tracing::warn!("Failed to iterate OpenCode {table}: {}", e),
        }
    }

    /// Build a record from one `data` JSON blob. `is_v2` selects between the
    /// 2.x (`session_message`) and 1.x (`message`) shapes.
    fn parse_message(data: &str, is_v2: bool) -> Option<TokenRecord> {
        let obj: serde_json::Value = serde_json::from_str(data).ok()?;

        // 1.x stored the role inside the blob; 2.x keeps it as the row's
        // `type` column and filters it in SQL.
        if !is_v2 && obj.get("role").and_then(|v| v.as_str()) != Some("assistant") {
            return None;
        }

        // Only assistant messages carry token usage.
        let tokens = obj.get("tokens")?;
        if tokens.is_null() {
            return None;
        }

        let get_i64 = |key: &str| -> i64 { tokens.get(key).and_then(|v| v.as_i64()).unwrap_or(0) };
        let input_tokens = get_i64("input");
        let output_tokens = get_i64("output");
        let reasoning_tokens = get_i64("reasoning");
        let cache = tokens.get("cache");
        let cache_read_tokens = cache
            .and_then(|c| c.get("read"))
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let cache_write_tokens = cache
            .and_then(|c| c.get("write"))
            .and_then(|v| v.as_i64())
            .unwrap_or(0);

        // Filter out zero-usage records (intermediate streaming states)
        if input_tokens == 0
            && output_tokens == 0
            && reasoning_tokens == 0
            && cache_read_tokens == 0
            && cache_write_tokens == 0
        {
            return None;
        }

        // `reasoning` is a separate, billed token class — NOT a subset of
        // `output`. OpenCode bills it at the output rate, which the API's own
        // `cost` field proves: for deepseek-v4-flash
        // `cost = input*0.14 + (output + reasoning)*0.28 + cache_read*0.0028`
        // per 1M reproduces every recorded cost exactly (1.x data), and 46
        // 1.x rows have `output < reasoning`, impossible if output were
        // inclusive. Folding it in here keeps the token columns aligned with
        // what was actually billed.
        let effective_output = output_tokens + reasoning_tokens;

        // 2.x dropped `tokens.total`; 1.x's `total` is exactly this same sum.
        let total_tokens = tokens
            .get("total")
            .and_then(|v| v.as_i64())
            .filter(|t| *t > 0)
            .unwrap_or(input_tokens + effective_output + cache_read_tokens + cache_write_tokens);

        let mut provider = if is_v2 {
            obj.get("model")
                .and_then(|m| m.get("providerID"))
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string()
        } else {
            obj.get("providerID")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string()
        };

        let model = if is_v2 {
            obj.get("model")
                .and_then(|m| m.get("id"))
                .and_then(|v| v.as_str())
        } else {
            obj.get("modelID").and_then(|v| v.as_str())
        }
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
            date: date.into(),
            parsed_time: OnceLock::new(),
            time,
            api_key_prefix: "N/A".into(),
            provider: provider.into(),
            original_provider: None,
            model: model.into(),
            source: "opencode".into(),
            input_tokens,
            output_tokens: effective_output,
            cache_read_tokens,
            cache_write_tokens,
            total_tokens,
            cost,
            ttft_ms: None,
            tps: None,
        })
    }
}

/// SQL mirror of [`OpenCodeSource::parse_message`]'s zero-usage filter, so a
/// row counted here is a row the parser actually emits.
const USAGE_SQL: &str = "(
        ifnull(json_extract(data, '$.tokens.input'), 0) > 0
     OR ifnull(json_extract(data, '$.tokens.output'), 0) > 0
     OR ifnull(json_extract(data, '$.tokens.reasoning'), 0) > 0
     OR ifnull(json_extract(data, '$.tokens.cache.read'), 0) > 0
     OR ifnull(json_extract(data, '$.tokens.cache.write'), 0) > 0
    )";

/// How many records the OpenCode DB can currently reproduce — exactly what
/// [`OpenCodeSource::parse`] would return for it.
///
/// Lets [`crate::store::TokenStore::migrate_opencode_reasoning_output`] tell
/// "history is intact, safe to re-ingest" apart from "the original sessions
/// have been cleaned up". `None` means the DB is unreadable.
pub fn opencode_readable_row_count() -> Option<i64> {
    let conn = OpenCodeSource::open_read_only(&OpenCodeSource::db_path())?;
    let sql = if OpenCodeSource::table_exists(&conn, "session_message") {
        format!(
            "SELECT (SELECT count(*) FROM session_message \
                    WHERE type = 'assistant' AND {USAGE_SQL}) \
                  + (SELECT count(*) FROM message m \
                     WHERE m.id NOT IN (SELECT id FROM session_message) AND {USAGE_SQL})"
        )
    } else {
        format!("SELECT count(*) FROM message WHERE {USAGE_SQL}")
    };
    conn.query_row(&sql, [], |row| row.get::<_, i64>(0))
        .optional()
        .ok()
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_row_for_test(data: &str, is_v2: bool) -> Option<TokenRecord> {
        OpenCodeSource::parse_message(data, is_v2)
    }

    fn v1_json(model: &str, input: i64, output: i64, reasoning: i64, cache_read: i64) -> String {
        serde_json::json!({
            "role": "assistant",
            "modelID": model,
            "providerID": "opencode",
            "tokens": {
                "input": input, "output": output, "reasoning": reasoning,
                "cache": {"read": cache_read, "write": 0},
                "total": input + output + reasoning + cache_read
            },
            "cost": 0.0020916,
            "time": {"created": 1789600000000i64, "completed": 1789600001000i64}
        })
        .to_string()
    }

    /// The same usage in 2.x shape: no role, no total, model nested.
    fn v2_json(provider: &str, model: &str, input: i64, output: i64, reasoning: i64) -> String {
        serde_json::json!({
            "agent": "build",
            "model": {"providerID": provider, "id": model, "variant": "max"},
            "content": [],
            "cost": 0,
            "tokens": {
                "input": input, "output": output, "reasoning": reasoning,
                "cache": {"read": 0, "write": 0}
            },
            "time": {"created": 1789600000000i64, "completed": 1789600001000i64}
        })
        .to_string()
    }

    #[test]
    fn parses_v1_shape() {
        let rec = parse_row_for_test(&v1_json("deepseek-v4-flash", 14866, 10, 27, 0), false)
            .expect("v1 assistant row");
        assert_eq!(rec.model, "deepseek-v4-flash");
        assert_eq!(rec.provider, "opencode-go", "opencode normalizes");
        assert_eq!(rec.input_tokens, 14866);
        assert_eq!(rec.output_tokens, 37, "reasoning folds into output");
        assert_eq!(rec.total_tokens, 14903, "1.x total is kept as-is");
        assert_eq!(rec.source, "opencode", "source id is stable");
        assert_eq!(rec.time, "2026-09-16T23:06:41+00:00", "completed wins");
    }

    #[test]
    fn parses_v2_shape_with_the_same_numbers() {
        let v1 = parse_row_for_test(&v1_json("deepseek-v4-flash", 14866, 10, 27, 0), false)
            .expect("v1 row");
        let v2 = parse_row_for_test(
            &v2_json("opencode-go", "deepseek-v4-flash", 14866, 10, 27),
            true,
        )
        .expect("v2 assistant row");
        assert_eq!(v2.model, v1.model);
        assert_eq!(v2.provider, v1.provider);
        assert_eq!(v2.input_tokens, v1.input_tokens);
        assert_eq!(
            v2.output_tokens, v1.output_tokens,
            "reasoning must fold identically across shapes"
        );
        assert_eq!(
            v2.total_tokens, v1.total_tokens,
            "computed total must match 1.x's stored total"
        );
        assert_eq!(v2.time, v1.time);
    }

    #[test]
    fn v2_nested_model_and_provider_are_read() {
        let rec = parse_row_for_test(&v2_json("ollama-cloud", "glm-5.3", 10, 5, 0), true).unwrap();
        assert_eq!(rec.model, "glm-5.3");
        assert_eq!(rec.provider, "ollama-cloud", "only `opencode` is renamed");
    }

    #[test]
    fn skips_non_assistant_and_zero_usage_rows() {
        let user = serde_json::json!({"role": "user", "text": "hi"}).to_string();
        assert!(parse_row_for_test(&user, false).is_none(), "1.x user row");

        let zero = v2_json("opencode", "space-bunny-free", 0, 0, 0);
        assert!(
            parse_row_for_test(&zero, true).is_none(),
            "streaming placeholder with no usage"
        );

        // Reasoning alone still counts as usage.
        let reasoning_only = v2_json("opencode", "space-bunny-free", 0, 0, 42);
        let rec = parse_row_for_test(&reasoning_only, true).expect("reasoning-only row");
        assert_eq!(rec.output_tokens, 42);
    }

    /// The billing claim the reasoning fold-in rests on, checked against a
    /// real 1.x row whose cost the API reported.
    #[test]
    fn reasoning_is_billed_as_output() {
        let (input, output, reasoning, cache_read, cost) =
            (14866_i64, 10_i64, 27_i64, 0_i64, 0.0020916_f64);
        let billed = input as f64 * 0.14e-6
            + (output + reasoning) as f64 * 0.28e-6
            + cache_read as f64 * 0.0028e-6;
        assert!(
            (billed - cost).abs() < 1e-9,
            "output-only billing would give {}, reasoning-inclusive {}",
            input as f64 * 0.14e-6 + output as f64 * 0.28e-6,
            billed
        );
    }

    /// Invariants over the machine's real OpenCode DB, which mixes 1.x-era
    /// sessions, their 2.x-migrated copies, and native 2.x rows. Skips
    /// silently where OpenCode is not installed.
    #[test]
    fn parses_local_db_if_present() {
        let path = OpenCodeSource::db_path();
        if !path.exists() {
            return;
        }
        // Sampled first: the DB is live and only appends, so the parse below
        // can only see at least as many rows.
        let readable = opencode_readable_row_count();
        let records = OpenCodeSource::parse(&path);
        assert!(!records.is_empty(), "{} has usage rows", path.display());

        for record in &records {
            assert_eq!(record.source, "opencode");
            assert_ne!(
                record.model, "unknown",
                "model must resolve: {:?}",
                record.time
            );
            assert_ne!(
                record.provider, "unknown",
                "provider must resolve: {:?}",
                record.time
            );
            assert_ne!(
                record.time, "unknown",
                "timestamp must resolve for {:?}",
                record.model
            );
            assert_eq!(
                record.total_tokens,
                record.input_tokens
                    + record.output_tokens
                    + record.cache_read_tokens
                    + record.cache_write_tokens,
                "total must equal the sum of the token columns for {:?}",
                record.time
            );
        }

        // Reading the two tables must not hand back the same usage twice: the
        // 2.x copy is preferred and the 1.x table only contributes leftovers.
        let mut fingerprints: Vec<u64> = records.iter().map(TokenRecord::fingerprint).collect();
        let unique = fingerprints.len();
        fingerprints.sort_unstable();
        fingerprints.dedup();
        assert_eq!(
            fingerprints.len(),
            unique,
            "the 1.x/2.x tables must not double-report a message"
        );

        // The migration gate purges when this count covers the persisted rows,
        // so it must never overestimate what the parser emits. `readable` was
        // sampled first and the source only appends, so `>=` is the direction
        // that proves `USAGE_SQL` mirrors `parse_message`'s zero-usage filter.
        // `None` means the DB was busy — the gate already fails safe there.
        if let Some(readable) = readable {
            assert!(
                records.len() as i64 >= readable,
                "USAGE_SQL counted {readable} rows but the parser emitted only {}; \
                 the gate would purge rows it cannot re-ingest",
                records.len()
            );
        }
    }
}
