//! Dedicated SQLite persistence for token usage records.
//!
//! This store is the durable source of truth for the dashboard: every
//! record discovered in a tool's session logs is written here, so the
//! data survives even when the original session files are cleaned up.
//!
//! Design:
//! - In-memory `records` in `AppState` is always rebuilt as a snapshot of
//!   this DB (after startup, each refresh, and each restore), so the two
//!   never diverge.
//! - Inserts are idempotent (`INSERT OR IGNORE` on a fingerprint unique
//!   index): re-scanning sources never duplicates history.
//! - A failed batch is rolled back and simply retried on the next refresh,
//!   since the source logs still contain the records.

use crate::models::TokenRecord;
use rusqlite::{params, Connection, OpenFlags};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

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
            panic!("Failed to initialize token store schema at {:?}: {}", path, e)
        });

        tracing::info!("Token store ready at {:?}", path);
        Self {
            path: path.to_path_buf(),
            conn: Mutex::new(conn),
        }
    }

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
            fixture("pi", "deepseek", "deepseek-v4-pro", "2026-07-01T01:00:00Z", 100),
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
        let a = fixture("pi", "deepseek", "deepseek-v4-pro", "2026-07-01T01:00:00Z", 100);
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
            fixture("pi", "deepseek", "deepseek-v4-pro", "2026-07-02T01:00:00Z", 100),
            fixture("pi", "deepseek", "deepseek-v4-pro", "2026-07-01T01:00:00Z", 100),
            fixture("pi", "deepseek", "deepseek-v4-pro", "2026-07-03T01:00:00Z", 100),
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
        assert_eq!(
            loaded[0].original_provider.as_deref(),
            Some("opencode-go")
        );
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
}
