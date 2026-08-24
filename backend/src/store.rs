//! Dedicated SQLite persistence for token usage records.
//!
//! This store is the durable source of truth for the dashboard: every
//! record discovered in a tool's session logs is written here, so the
//! data survives even when the original session files are cleaned up.
//!
//! Design:
//! - In-memory `records` in `AppState` is the live view the frontend
//!   reads; it is updated immediately on refresh, while disk writes are
//!   deferred through [`PendingBuffer`] and batched at most once per
//!   `FLUSH_DELAY` to avoid frequent SSD writes. Memory is therefore a
//!   superset of the DB (memory = DB + queued records).
//! - Inserts are idempotent (`INSERT OR IGNORE` on a fingerprint unique
//!   index): re-scanning sources never duplicates history.
//! - A failed batch is rolled back and re-queued for the next flush, since
//!   the source logs still contain the records.

use crate::models::TokenRecord;
use rusqlite::{params, Connection, OpenFlags};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Env var overriding the token-stats database path.
pub const DB_PATH_ENV: &str = "TOKEN_STATS_DB_PATH";

/// Default database location: `~/.config/token-stats/token-stats.db`
/// (same directory family as the persisted Fenno auth state).
pub fn token_store_path() -> PathBuf {
    if let Ok(p) = std::env::var(DB_PATH_ENV) {
        if !p.trim().is_empty() {
            return PathBuf::from(p);
        }
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home)
        .join(".config")
        .join("token-stats")
        .join("token-stats.db")
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS token_records (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    time TEXT NOT NULL,
    date TEXT NOT NULL,
    api_key_prefix TEXT NOT NULL DEFAULT '',
    provider TEXT NOT NULL,
    original_provider TEXT,
    model TEXT NOT NULL,
    source TEXT NOT NULL DEFAULT '',
    input_tokens INTEGER NOT NULL DEFAULT 0,
    output_tokens INTEGER NOT NULL DEFAULT 0,
    cache_read_tokens INTEGER NOT NULL DEFAULT 0,
    cache_write_tokens INTEGER NOT NULL DEFAULT 0,
    total_tokens INTEGER NOT NULL DEFAULT 0,
    cost REAL NOT NULL DEFAULT 0,
    ttft_ms REAL,
    tps REAL,
    ingested_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_token_records_fingerprint
    ON token_records(time, provider, model, source,
                     input_tokens, output_tokens, cache_read_tokens);

CREATE INDEX IF NOT EXISTS idx_token_records_time ON token_records(time);
CREATE INDEX IF NOT EXISTS idx_token_records_source ON token_records(source);
CREATE INDEX IF NOT EXISTS idx_token_records_provider ON token_records(provider);

PRAGMA user_version = 1;
"#;

const INSERT_SQL: &str = r#"
INSERT OR IGNORE INTO token_records
    (time, date, api_key_prefix, provider, original_provider, model, source,
     input_tokens, output_tokens, cache_read_tokens, cache_write_tokens,
     total_tokens, cost, ttft_ms, tps)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
"#;

const SELECT_SQL: &str = r#"
SELECT time, date, api_key_prefix, provider, original_provider, model, source,
       input_tokens, output_tokens, cache_read_tokens, cache_write_tokens,
       total_tokens, cost, ttft_ms, tps
FROM token_records
ORDER BY time, source, provider, model
"#;

/// Thread-safe wrapper around the SQLite connection.
pub struct TokenStore {
    path: PathBuf,
    conn: Mutex<Connection>,
}

impl TokenStore {
    /// Open (creating if needed) the default token store.
    pub fn open_default() -> Self {
        let path = token_store_path();
        Self::open(&path)
    }

    /// Open (creating if needed) the store at `path`.
    ///
    /// Panics on unrecoverable errors (unwritable directory, corrupt DB)
    /// because durability is the point of this store — failing loudly beats
    /// silently running without persistence.
    pub fn open(path: &Path) -> Self {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).unwrap_or_else(|e| {
                    panic!(
                        "Failed to create token store directory {:?}: {}. \
                         Set {} to use a different location.",
                        parent, e, DB_PATH_ENV
                    )
                });
            }
        }

        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_FULL_MUTEX,
        )
        .unwrap_or_else(|e| {
            panic!(
                "Failed to open token store at {:?}: {}. \
                 Set {} to use a different location.",
                path, e, DB_PATH_ENV
            )
        });

        conn.busy_timeout(Duration::from_secs(5)).ok();
        // WAL is preferred but optional (some filesystems disallow it);
        // the default journal still works.
        if let Err(e) = conn.pragma_update(None, "journal_mode", "WAL") {
            tracing::debug!("Failed to enable WAL on token store: {}", e);
        }
        let _ = conn.pragma_update(None, "synchronous", "NORMAL");

        conn.execute_batch(SCHEMA).unwrap_or_else(|e| {
            panic!(
                "Failed to initialize token store schema at {:?}: {}",
                path, e
            )
        });

        apply_store_patches(&conn);

        tracing::info!("Token store ready at {:?}", path);
        Self {
            path: path.to_path_buf(),
            conn: Mutex::new(conn),
        }
    }
}

fn apply_store_patches(conn: &Connection) {
    // One-off fix: all dim-agent usage was miscategorized by model/prefix.
    // Every record with source='dim' belongs to vendor `dim`.
    let _ = conn.execute(
        "UPDATE OR IGNORE token_records SET provider = 'dim' WHERE source = 'dim' AND provider != 'dim'",
        [],
    );
    let _ = conn.execute(
        "DELETE FROM token_records WHERE source = 'dim' AND provider != 'dim'",
        [],
    );

    collapse_commandcode_inclusive_twins(conn);
    collapse_unknown_codex_twins(conn);
}

/// Drop native cmd rows that stored OpenAI-inclusive `inputTokens` when the
/// exclusive twin is also present. Safe to run on every open: exclusive
/// leftovers (cache hit ≤ 50%) are left alone.
fn collapse_commandcode_inclusive_twins(conn: &Connection) -> usize {
    let deleted = conn
        .execute(
            "DELETE FROM token_records
             WHERE id IN (
                 SELECT a.id
                 FROM token_records a
                 JOIN token_records b
                   ON a.source = b.source
                  AND a.time = b.time
                  AND a.provider = b.provider
                  AND a.model = b.model
                  AND a.output_tokens = b.output_tokens
                  AND a.cache_read_tokens = b.cache_read_tokens
                 WHERE a.source = 'commandcode'
                   AND a.cache_read_tokens > 0
                   AND a.input_tokens = b.input_tokens + a.cache_read_tokens
             )",
            [],
        )
        .unwrap_or(0);
    if deleted > 0 {
        tracing::info!(
            "Removed {} commandcode row(s) that double-counted cache in input",
            deleted
        );
    }
    deleted
}

/// Drop Codex rows that were stored as model=unknown when the same call
/// also exists with the real model. Incremental re-parses used to skip
/// session_meta / turn_context and emit openai/unknown twins.
fn collapse_unknown_codex_twins(conn: &Connection) -> usize {
    let deleted = conn
        .execute(
            "DELETE FROM token_records
             WHERE id IN (
                 SELECT a.id
                 FROM token_records a
                 JOIN token_records b
                   ON a.source = b.source
                  AND a.time = b.time
                  AND a.input_tokens = b.input_tokens
                  AND a.output_tokens = b.output_tokens
                  AND a.cache_read_tokens = b.cache_read_tokens
                 WHERE a.source = 'codex'
                   AND a.model = 'unknown'
                   AND b.model != 'unknown'
             )",
            [],
        )
        .unwrap_or(0);
    if deleted > 0 {
        tracing::info!(
            "Removed {} codex row(s) whose model was unknown but a named twin exists",
            deleted
        );
    }
    deleted
}

impl TokenStore {
    /// Path to the SQLite database file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Number of records currently persisted.
    pub fn count(&self) -> usize {
        let conn = match self.conn.lock() {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("Token store lock poisoned: {}", e);
                return 0;
            }
        };
        conn.query_row("SELECT COUNT(*) FROM token_records", [], |row| {
            row.get::<_, i64>(0)
        })
        .map(|n| n as usize)
        .unwrap_or(0)
    }

    /// Load every persisted record, ordered by time.
    pub fn load_all(&self) -> Vec<TokenRecord> {
        let conn = match self.conn.lock() {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("Token store lock poisoned: {}", e);
                return Vec::new();
            }
        };
        let mut stmt = match conn.prepare(SELECT_SQL) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("Failed to prepare token store query: {}", e);
                return Vec::new();
            }
        };
        let rows = stmt.query_map([], row_to_record);
        match rows {
            Ok(iter) => iter.flatten().collect(),
            Err(e) => {
                tracing::warn!("Failed to read token store: {}", e);
                Vec::new()
            }
        }
    }

    /// Remove native cmd rows that stored cache-inclusive input when the
    /// exclusive twin is also present. Idempotent.
    pub fn collapse_commandcode_inclusive_twins(&self) -> usize {
        let conn = match self.conn.lock() {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("Token store lock poisoned: {}", e);
                return 0;
            }
        };
        collapse_commandcode_inclusive_twins(&conn)
    }

    /// Remove Codex unknown-model rows when the same call is also stored
    /// with a real model. Idempotent.
    pub fn collapse_unknown_codex_twins(&self) -> usize {
        let conn = match self.conn.lock() {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("Token store lock poisoned: {}", e);
                return 0;
            }
        };
        collapse_unknown_codex_twins(&conn)
    }

    /// Insert records that are not already present (fingerprint-unique).
    ///
    /// Returns the number of rows newly inserted. Duplicates are ignored.
    /// The whole batch is rolled back if any insert fails, so callers can
    /// safely retry on the next refresh.
    pub fn insert_batch(&self, records: &[TokenRecord]) -> usize {
        if records.is_empty() {
            return 0;
        }
        let mut conn = match self.conn.lock() {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("Token store lock poisoned: {}", e);
                return 0;
            }
        };
        let tx = match conn.transaction() {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!("Failed to start token store transaction: {}", e);
                return 0;
            }
        };

        let mut inserted = 0usize;
        let mut failed = false;
        for r in records {
            match tx.execute(
                INSERT_SQL,
                params![
                    r.time,
                    r.date,
                    r.api_key_prefix,
                    r.provider,
                    r.original_provider,
                    r.model,
                    r.source,
                    r.input_tokens,
                    r.output_tokens,
                    r.cache_read_tokens,
                    r.cache_write_tokens,
                    r.total_tokens,
                    r.cost,
                    r.ttft_ms,
                    r.tps,
                ],
            ) {
                Ok(n) => inserted += n,
                Err(e) => {
                    tracing::warn!(
                        "Failed to persist record to token store: {} ({:?})",
                        e,
                        r.time
                    );
                    failed = true;
                    break;
                }
            }
        }

        if failed {
            if let Err(e) = tx.rollback() {
                tracing::warn!("Failed to roll back token store transaction: {}", e);
            }
            tracing::warn!(
                "Token store batch rolled back ({} of {} records inserted before failure); \
                 will retry on next refresh",
                inserted,
                records.len()
            );
            return 0;
        }

        match tx.commit() {
            Ok(()) => inserted,
            Err(e) => {
                tracing::warn!("Failed to commit token store transaction: {}", e);
                0
            }
        }
    }
}

/// Debounced disk-write buffer.
///
/// The refresh task queues records here (and publishes them to memory
/// immediately, so the frontend always sees the latest data). A background
/// flush task drains the buffer into SQLite in one batch once `delay` has
/// elapsed since the last queue *or* the last flush — at most one write per
/// `delay`, and every queued record is persisted within `delay` of arrival.
/// [`PendingBuffer::take_all`] is the shutdown path: it drains
/// unconditionally so the final write on exit is reliable.
///
/// `take_if_due` takes an explicit `now` so the debounce logic is testable
/// with fake time.
pub struct PendingBuffer {
    records: Mutex<Vec<TokenRecord>>,
    last_queued: Mutex<Option<Instant>>,
    last_flush: Mutex<Option<Instant>>,
}

impl PendingBuffer {
    pub fn new() -> Self {
        Self {
            records: Mutex::new(Vec::new()),
            last_queued: Mutex::new(None),
            last_flush: Mutex::new(None),
        }
    }

    /// Queue records for the next batched write.
    pub fn queue(&self, records: Vec<TokenRecord>) {
        self.queue_at(records, Instant::now());
    }

    /// Queue records, recording `now` as the queue time (testable with fake
    /// time).
    fn queue_at(&self, records: Vec<TokenRecord>, now: Instant) {
        if records.is_empty() {
            return;
        }
        let mut buf = self.records.lock().unwrap();
        buf.extend(records);
        *self.last_queued.lock().unwrap() = Some(now);
    }

    /// Number of records waiting to be written.
    pub fn len(&self) -> usize {
        self.records.lock().unwrap().len()
    }

    /// Drain the buffer if a write is due: at least `delay` since the last
    /// queue or the last flush. Returns the batch to write (empty if not
    /// due). A write that is drained here is considered a flush, so the
    /// next one cannot happen before `delay` again.
    pub fn take_if_due(&self, delay: Duration, now: Instant) -> Vec<TokenRecord> {
        let mut buf = self.records.lock().unwrap();
        if buf.is_empty() {
            return Vec::new();
        }
        let queued_due = self
            .last_queued
            .lock()
            .unwrap()
            .is_some_and(|t| now.duration_since(t) >= delay);
        let flush_due = self
            .last_flush
            .lock()
            .unwrap()
            .is_some_and(|t| now.duration_since(t) >= delay);
        if queued_due || flush_due {
            let batch = std::mem::take(&mut *buf);
            *self.last_flush.lock().unwrap() = Some(now);
            batch
        } else {
            Vec::new()
        }
    }

    /// Drain unconditionally (shutdown flush).
    pub fn take_all(&self) -> Vec<TokenRecord> {
        std::mem::take(&mut *self.records.lock().unwrap())
    }
}

fn row_to_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<TokenRecord> {
    Ok(TokenRecord {
        time: row.get(0)?,
        date: row.get(1)?,
        api_key_prefix: row.get(2)?,
        provider: row.get(3)?,
        original_provider: row.get(4)?,
        model: row.get(5)?,
        source: row.get(6)?,
        input_tokens: row.get(7)?,
        output_tokens: row.get(8)?,
        cache_read_tokens: row.get(9)?,
        cache_write_tokens: row.get(10)?,
        total_tokens: row.get(11)?,
        cost: row.get(12)?,
        ttft_ms: row.get(13)?,
        tps: row.get(14)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(source: &str, provider: &str, model: &str, time: &str, tokens: i64) -> TokenRecord {
        TokenRecord {
            date: time[..10].to_string(),
            time: time.to_string(),
            api_key_prefix: "sk-test".to_string(),
            provider: provider.to_string(),
            original_provider: None,
            model: model.to_string(),
            source: source.to_string(),
            input_tokens: tokens / 2,
            output_tokens: tokens / 2,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            total_tokens: tokens,
            cost: 0.0,
            ttft_ms: Some(123.0),
            tps: Some(45.6),
        }
    }

    fn temp_store() -> TokenStore {
        let dir = tempfile::tempdir().expect("tempdir");
        TokenStore::open(&dir.path().join("token-stats.db"))
    }

    #[test]
    fn insert_and_load_roundtrip() {
        let store = temp_store();
        let records = vec![
            fixture(
                "pi",
                "deepseek",
                "deepseek-v4-pro",
                "2026-07-01T01:00:00Z",
                100,
            ),
            fixture("codex", "openai", "gpt-5.5", "2026-07-01T02:00:00Z", 200),
        ];
        assert_eq!(store.insert_batch(&records), 2);
        assert_eq!(store.count(), 2);

        let loaded = store.load_all();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].time, "2026-07-01T01:00:00Z");
        assert_eq!(loaded[1].model, "gpt-5.5");
        assert_eq!(loaded[1].ttft_ms, Some(123.0));
        assert_eq!(loaded[1].tps, Some(45.6));
        // Fields survive the round-trip exactly.
        assert_eq!(loaded[0], records[0]);
        assert_eq!(loaded[1], records[1]);
    }

    #[test]
    fn insert_ignores_duplicate_fingerprints() {
        let store = temp_store();
        let a = fixture(
            "pi",
            "deepseek",
            "deepseek-v4-pro",
            "2026-07-01T01:00:00Z",
            100,
        );
        let mut b = a.clone();
        // Same fingerprint but different cost — treated as the same record.
        b.cost = 9.99;
        b.ttft_ms = None;

        assert_eq!(store.insert_batch(&[a]), 1);
        assert_eq!(store.insert_batch(&[b]), 0);
        assert_eq!(store.count(), 1);

        let loaded = store.load_all();
        assert_eq!(loaded[0].cost, 0.0, "first-seen values are kept");
        assert_eq!(loaded[0].ttft_ms, Some(123.0));
    }

    #[test]
    fn load_all_sorts_by_time() {
        let store = temp_store();
        let records = vec![
            fixture(
                "pi",
                "deepseek",
                "deepseek-v4-pro",
                "2026-07-02T01:00:00Z",
                100,
            ),
            fixture(
                "pi",
                "deepseek",
                "deepseek-v4-pro",
                "2026-07-01T01:00:00Z",
                100,
            ),
            fixture(
                "pi",
                "deepseek",
                "deepseek-v4-pro",
                "2026-07-03T01:00:00Z",
                100,
            ),
        ];
        store.insert_batch(&records);
        let loaded = store.load_all();
        let times: Vec<&str> = loaded.iter().map(|r| r.time.as_str()).collect();
        assert_eq!(
            times,
            vec![
                "2026-07-01T01:00:00Z",
                "2026-07-02T01:00:00Z",
                "2026-07-03T01:00:00Z",
            ]
        );
    }

    #[test]
    fn empty_batch_is_noop() {
        let store = temp_store();
        assert_eq!(store.insert_batch(&[]), 0);
        assert_eq!(store.count(), 0);
    }

    #[test]
    fn optional_fields_roundtrip_as_null() {
        let store = temp_store();
        let mut r = fixture(
            "claude-code",
            "anthropic",
            "claude-opus-4-7",
            "2026-07-01T01:00:00Z",
            50,
        );
        r.ttft_ms = None;
        r.tps = None;
        r.original_provider = Some("opencode-go".to_string());
        assert_eq!(store.insert_batch(&[r]), 1);
        let loaded = store.load_all();
        assert_eq!(loaded[0].ttft_ms, None);
        assert_eq!(loaded[0].tps, None);
        assert_eq!(loaded[0].original_provider.as_deref(), Some("opencode-go"));
    }

    #[test]
    fn path_resolution_uses_env_override() {
        let dir = tempfile::tempdir().expect("tempdir");
        let custom = dir.path().join("custom.db");
        temp_env::with_var(DB_PATH_ENV, Some(custom.to_str().unwrap()), || {
            assert_eq!(token_store_path(), custom);
        });
    }

    #[test]
    fn path_resolution_defaults_to_config_dir() {
        temp_env::with_var(DB_PATH_ENV, None::<&str>, || {
            temp_env::with_var("HOME", Some("/tmp/fake-home"), || {
                assert_eq!(
                    token_store_path(),
                    PathBuf::from("/tmp/fake-home/.config/token-stats/token-stats.db")
                );
            });
        });
    }

    fn seed_legacy_commandcode(
        path: &Path,
        rows: &[( &str, i64, i64, i64)],
    ) {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        for (time, input, cache_read, output) in rows {
            conn.execute(
                INSERT_SQL,
                params![
                    time,
                    &time[..10],
                    "N/A",
                    "commandcode",
                    None::<String>,
                    "muse-spark-1.2-contributor",
                    "commandcode",
                    input,
                    output,
                    cache_read,
                    0i64,
                    input + output + cache_read,
                    0.0f64,
                    None::<f64>,
                    None::<f64>,
                ],
            )
            .unwrap();
        }
    }

    #[test]
    fn reopen_drops_inclusive_commandcode_twin() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("token-stats.db");
        seed_legacy_commandcode(
            &path,
            &[
                ("2026-08-18T23:37:11.318+00:00", 140505, 140209, 1197),
                ("2026-08-18T23:37:11.318+00:00", 296, 140209, 1197),
            ],
        );

        let store = TokenStore::open(&path);
        let loaded = store.load_all();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].input_tokens, 296);
        assert_eq!(loaded[0].cache_read_tokens, 140209);
        assert_eq!(loaded[0].total_tokens, 296 + 1197 + 140209);
    }

    #[test]
    fn reopen_keeps_exclusive_commandcode_when_no_twin() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("token-stats.db");
        // Already-normalized exclusive input can still be >= cache_read
        // (cache hit ≤ 50%). Do not subtract again.
        seed_legacy_commandcode(&path, &[("2026-08-23T12:32:48.057Z", 15600, 7424, 71)]);

        let store = TokenStore::open(&path);
        let loaded = store.load_all();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].input_tokens, 15600);
        assert_eq!(loaded[0].total_tokens, 15600 + 71 + 7424);
    }

    #[test]
    fn collapse_after_insert_drops_inclusive_twin() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("token-stats.db");
        seed_legacy_commandcode(&path, &[("2026-08-23T12:32:48.057Z", 23024, 7424, 71)]);

        let store = TokenStore::open(&path);
        assert_eq!(store.count(), 1);
        let exclusive = TokenRecord {
            date: "2026-08-23".to_string(),
            time: "2026-08-23T12:32:48.057Z".to_string(),
            api_key_prefix: "N/A".to_string(),
            provider: "commandcode".to_string(),
            original_provider: None,
            model: "muse-spark-1.2-contributor".to_string(),
            source: "commandcode".to_string(),
            input_tokens: 15600,
            output_tokens: 71,
            cache_read_tokens: 7424,
            cache_write_tokens: 0,
            total_tokens: 15600 + 71 + 7424,
            cost: 0.0,
            ttft_ms: None,
            tps: None,
        };
        assert_eq!(store.insert_batch(&[exclusive]), 1);
        assert_eq!(store.collapse_commandcode_inclusive_twins(), 1);
        let loaded = store.load_all();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].input_tokens, 15600);
    }

    #[test]
    fn reopen_drops_unknown_codex_twin() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("token-stats.db");
        let store = TokenStore::open(&path);
        let named = fixture(
            "codex",
            "ainaba",
            "gpt-5.6-terra",
            "2026-08-25T10:00:00Z",
            110,
        );
        let mut unknown = named.clone();
        unknown.model = "unknown".to_string();
        assert_eq!(store.insert_batch(&[named, unknown]), 2);

        let reopened = TokenStore::open(&path);
        let loaded = reopened.load_all();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].model, "gpt-5.6-terra");
    }

    #[test]
    fn reopen_keeps_unknown_codex_when_no_twin() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("token-stats.db");
        let store = TokenStore::open(&path);
        let unknown = fixture("codex", "ainaba", "unknown", "2026-08-25T10:00:00Z", 110);
        assert_eq!(store.insert_batch(&[unknown]), 1);

        let reopened = TokenStore::open(&path);
        let loaded = reopened.load_all();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].model, "unknown");
    }

    #[test]
    fn collapse_after_insert_drops_unknown_codex_twin() {
        let store = temp_store();
        let unknown = fixture("codex", "ainaba", "unknown", "2026-08-25T10:00:00Z", 110);
        let named = fixture(
            "codex",
            "ainaba",
            "gpt-5.6-terra",
            "2026-08-25T10:00:00Z",
            110,
        );
        assert_eq!(store.insert_batch(&[unknown]), 1);
        assert_eq!(store.insert_batch(&[named]), 1);
        assert_eq!(store.collapse_unknown_codex_twins(), 1);
        let loaded = store.load_all();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].model, "gpt-5.6-terra");
    }

    #[test]
    fn collapse_drops_unknown_codex_twin_with_wrong_provider() {
        // Incremental skip defaulted provider to openai→ainaba while the
        // real call was fenno/ollama/xai. Match on time+tokens, not provider.
        let store = temp_store();
        let unknown = fixture("codex", "ainaba", "unknown", "2026-08-25T10:00:00Z", 110);
        let named = fixture(
            "codex",
            "fenno",
            "gpt-5.6-terra",
            "2026-08-25T10:00:00Z",
            110,
        );
        assert_eq!(store.insert_batch(&[unknown, named]), 2);
        assert_eq!(store.collapse_unknown_codex_twins(), 1);
        let loaded = store.load_all();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].provider, "fenno");
        assert_eq!(loaded[0].model, "gpt-5.6-terra");
    }

    // ── PendingBuffer (deferred disk writes) ────────────────────────────────

    #[test]
    fn pending_buffer_waits_for_delay_after_queue() {
        let buf = PendingBuffer::new();
        let t0 = Instant::now();
        buf.queue_at(
            vec![fixture(
                "pi",
                "deepseek",
                "deepseek-v4-pro",
                "2026-07-01T01:00:00Z",
                100,
            )],
            t0,
        );

        // Not due yet: 1 min after the queue.
        assert!(buf
            .take_if_due(Duration::from_secs(120), t0 + Duration::from_secs(60))
            .is_empty());
        // Due: 2 min after the queue.
        let batch = buf.take_if_due(Duration::from_secs(120), t0 + Duration::from_secs(121));
        assert_eq!(batch.len(), 1);
        assert_eq!(buf.len(), 0);
    }

    #[test]
    fn pending_buffer_flushes_at_most_once_per_delay_under_continuous_load() {
        let buf = PendingBuffer::new();
        let t0 = Instant::now();

        // First batch: queued at t0, flushed 2 min later (due via last_queued).
        buf.queue_at(
            vec![fixture(
                "pi",
                "deepseek",
                "deepseek-v4-pro",
                "2026-07-01T01:00:00Z",
                100,
            )],
            t0,
        );
        let first = buf.take_if_due(Duration::from_secs(120), t0 + Duration::from_secs(121));
        assert_eq!(first.len(), 1);

        // Continuous load: new data every 30s after the flush, pushing
        // last_queued forward so the queued_due arm never fires.
        for i in 1..=3 {
            let at = t0 + Duration::from_secs(150 + 30 * (i - 1));
            buf.queue_at(
                vec![fixture(
                    "pi",
                    "deepseek",
                    "deepseek-v4-pro",
                    &format!("2026-07-01T01:0{}:00Z", i),
                    100,
                )],
                at,
            );
            assert!(buf.take_if_due(Duration::from_secs(120), at).is_empty());
        }

        // 2 min after the last flush (t0+121s) the flush_due arm fires and
        // drains everything in one batch.
        let batch = buf.take_if_due(Duration::from_secs(120), t0 + Duration::from_secs(241));
        assert_eq!(batch.len(), 3, "all queued records flush in one batch");
        assert_eq!(buf.len(), 0);
    }

    #[test]
    fn pending_buffer_take_all_drains_unconditionally() {
        let buf = PendingBuffer::new();
        buf.queue(vec![fixture(
            "pi",
            "deepseek",
            "deepseek-v4-pro",
            "2026-07-01T01:00:00Z",
            100,
        )]);
        assert_eq!(buf.take_all().len(), 1);
        assert_eq!(buf.len(), 0);
        assert!(buf.take_all().is_empty());
    }

    #[test]
    fn pending_buffer_empty_queue_is_noop() {
        let buf = PendingBuffer::new();
        buf.queue(Vec::new());
        assert_eq!(buf.len(), 0);
        assert!(buf
            .take_if_due(Duration::from_secs(120), Instant::now())
            .is_empty());
    }
}
