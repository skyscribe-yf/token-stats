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
use compact_str::CompactString;
use rusqlite::{Connection, OpenFlags, params};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

/// Env var overriding the token-stats database path.
pub const DB_PATH_ENV: &str = "TOKEN_STATS_DB_PATH";

/// `count()` is queried by the store-info endpoint on every poll; cache it
/// briefly so we don't run a `SELECT COUNT(*)` against SQLite each time.
/// The DB is only written in batches every `FLUSH_DELAY` (120s), so a 60s
/// staleness window can't drift far from reality.
const COUNT_CACHE_TTL: Duration = Duration::from_secs(60);

static COUNT_CACHE: LazyLock<Mutex<HashMap<PathBuf, (Instant, usize)>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

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

-- Small key/value side table for source sync watermarks (e.g. the DimAgent
-- console API's newest ingested log id). Persisting them keeps a cold start
-- from re-walking the whole remote history page by page.
CREATE TABLE IF NOT EXISTS sync_watermarks (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

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
    // NOTE: the one-off "all dim usage belongs to vendor dim" migration
    // (UPDATE/DELETE source='dim' AND provider != 'dim') was removed: it ran
    // on every open and destroyed legitimate dim supplement rows (third-party
    // channels like ollama-cloud / xai-official) that carry their own provider.

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
        let path = self.path().to_path_buf();
        let now = Instant::now();
        {
            let guard = COUNT_CACHE.lock().unwrap();
            if let Some((fetched_at, n)) = guard.get(&path) {
                if now.duration_since(*fetched_at) < COUNT_CACHE_TTL {
                    return *n;
                }
            }
        }
        let n = self.count_uncached();
        COUNT_CACHE.lock().unwrap().insert(path, (now, n));
        n
    }

    /// Uncached `COUNT(*)` — used after every disk write so the cache stays
    /// accurate shortly after new records are persisted.
    fn count_uncached(&self) -> usize {
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

    /// Read a persisted sync watermark (e.g. a remote API's newest ingested
    /// id). Returns `None` when the key was never written or is unparsable.
    pub fn get_sync_watermark(&self, key: &str) -> Option<i64> {
        let conn = match self.conn.lock() {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("Token store lock poisoned: {}", e);
                return None;
            }
        };
        conn.query_row(
            "SELECT value FROM sync_watermarks WHERE key = ?1",
            [key],
            |row| row.get::<_, String>(0),
        )
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
    }

    /// Persist a sync watermark. Best-effort: a failure only costs a slower
    /// next cold start, never correctness.
    pub fn set_sync_watermark(&self, key: &str, value: i64) {
        let conn = match self.conn.lock() {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("Token store lock poisoned: {}", e);
                return;
            }
        };
        if let Err(e) = conn.execute(
            "INSERT INTO sync_watermarks (key, value, updated_at)
             VALUES (?1, ?2, strftime('%Y-%m-%dT%H:%M:%fZ','now'))
             ON CONFLICT(key) DO UPDATE SET
                 value = excluded.value,
                 updated_at = excluded.updated_at",
            params![key, value.to_string()],
        ) {
            tracing::warn!("Failed to persist sync watermark {key}: {e}");
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

    /// One-time migration helper: remove persisted legacy `source='dim'` rows
    /// (per-run aggregates collected from the local SQLite before the console
    /// API source existed) once per-request console-API records have taken
    /// over. `keep` holds the fingerprints of the dim records just loaded
    /// from the API (full backfill).
    ///
    /// A row is deleted only when its fingerprint is NOT in `keep`, so
    /// console-API rows are never removed even if they lack ttft/tps. Returns
    /// the number of rows removed; idempotent (no-op once legacy rows are
    /// gone).
    pub fn purge_dim_legacy(&self, keep: &std::collections::HashSet<u64>) -> usize {
        let mut conn = match self.conn.lock() {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("Token store lock poisoned: {}", e);
                return 0;
            }
        };
        let rows: Vec<i64> = {
            let mut stmt = match conn.prepare(
                "SELECT id, time, provider, model, source,
                        input_tokens, output_tokens, cache_read_tokens
                 FROM token_records WHERE source = 'dim'",
            ) {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!("Failed to prepare dim purge query: {}", e);
                    return 0;
                }
            };
            let iter = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, i64>(7)?,
                ))
            });
            let mut to_delete = Vec::new();
            match iter {
                Ok(rows) => {
                    for row in rows.flatten() {
                        let (id, time, provider, model, source, input, output, cache_read) = row;
                        let rec = TokenRecord {
                            time,
                            provider: provider.into(),
                            model: model.into(),
                            source: source.into(),
                            input_tokens: input,
                            output_tokens: output,
                            cache_read_tokens: cache_read,
                            // Remaining fields are irrelevant for fingerprint().
                            date: CompactString::default(),
                            api_key_prefix: CompactString::default(),
                            original_provider: None,
                            cache_write_tokens: 0,
                            total_tokens: 0,
                            cost: 0.0,
                            ttft_ms: None,
                            tps: None,
                        };
                        if !keep.contains(&rec.fingerprint()) {
                            to_delete.push(id);
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!("Failed to iterate dim rows for purge: {}", e);
                    return 0;
                }
            }
            to_delete
        };
        if rows.is_empty() {
            return 0;
        }
        let tx = match conn.transaction() {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!("Failed to start dim purge transaction: {}", e);
                return 0;
            }
        };
        let mut deleted = 0usize;
        for id in &rows {
            match tx.execute("DELETE FROM token_records WHERE id = ?1", params![id]) {
                Ok(n) => deleted += n,
                Err(e) => {
                    tracing::warn!("Failed to delete legacy dim row {id}: {e}");
                    break;
                }
            }
        }
        match tx.commit() {
            Ok(()) => {
                if deleted > 0 {
                    tracing::info!(
                        "Migrated dim collection: removed {deleted} legacy per-run \
                         record(s) replaced by console-API per-request records"
                    );
                }
                deleted
            }
            Err(e) => {
                tracing::warn!("Failed to commit dim purge: {e}");
                0
            }
        }
    }

    /// One-time migration helper: remove persisted `source='dim'` rows whose
    /// provider is the legacy `grok-build` name. The dim source now maps the
    /// Grok Build channel to `xai-official` (same SuperGrok subscription as
    /// grok-cli), so old rows would otherwise double-count the same usage
    /// under a stale provider. Idempotent (no-op once the rows are gone).
    pub fn purge_dim_grok_build(&self) -> usize {
        let conn = match self.conn.lock() {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("Token store lock poisoned: {}", e);
                return 0;
            }
        };
        let deleted = conn
            .execute(
                "DELETE FROM token_records WHERE source = 'dim' AND provider = 'grok-build'",
                [],
            )
            .unwrap_or_else(|e| {
                tracing::warn!("Failed to purge legacy dim grok-build rows: {e}");
                0
            });
        if deleted > 0 {
            tracing::info!(
                "Migrated dim collection: removed {deleted} legacy grok-build row(s) \
                 replaced by xai-official records"
            );
        }
        deleted
    }

    /// One-time migration helper: remove persisted `source='zcode'` rows
    /// mislabeled with provider `anthropic`. The bigmodel coding plan speaks
    /// the Anthropic protocol, so `provider_metadata_json` rows carrying the
    /// `anthropic` adapter key used to label every `builtin:bigmodel-*`
    /// record `anthropic`; the source now maps those to `bigmodel`, and the
    /// old rows would double-count the same requests under the stale name.
    /// Idempotent (no-op once the rows are gone).
    pub fn purge_zcode_anthropic(&self) -> usize {
        let conn = match self.conn.lock() {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("Token store lock poisoned: {}", e);
                return 0;
            }
        };
        let deleted = conn
            .execute(
                "DELETE FROM token_records WHERE source = 'zcode' AND provider = 'anthropic'",
                [],
            )
            .unwrap_or_else(|e| {
                tracing::warn!("Failed to purge mislabeled zcode anthropic rows: {e}");
                0
            });
        if deleted > 0 {
            tracing::info!(
                "Migrated zcode collection: removed {deleted} row(s) mislabeled \
                 provider='anthropic', re-ingested as provider='bigmodel'"
            );
        }
        deleted
    }

    /// One-time migration helper for the ZCode Weekend Build trial channel.
    ///
    /// The `builtin:bigmodel-start-plan` provider (3 亿 token 体验套餐) used to
    /// be ingested as `provider='bigmodel'`, identical to the paid coding
    /// plan. The source now maps it to `bigmodel-start`, so the persisted
    /// rows would double-count once re-parsed under the new fingerprint.
    /// Unlike the other purge helpers this runs BEFORE the startup
    /// `load_all()` (see `AppState::new`): the deleted window is then
    /// re-ingested from the zcode DB with the new mapping — coding-plan rows
    /// regenerate with identical fingerprints, start-plan rows land on
    /// `bigmodel-start`. Cutoff = first start-plan usage (2026-09-13
    /// 09:34:58 CST) with a safety margin; earlier history never had the
    /// trial channel. Idempotent (later runs delete nothing).
    pub fn purge_zcode_start_plan_bigmodel(&self) -> usize {
        const CUTOFF: &str = "2026-09-13T01:30:00+00:00";
        let conn = match self.conn.lock() {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("Token store lock poisoned: {}", e);
                return 0;
            }
        };
        let deleted = conn
            .execute(
                "DELETE FROM token_records
                 WHERE source = 'zcode' AND provider = 'bigmodel' AND time >= ?1",
                [CUTOFF],
            )
            .unwrap_or_else(|e| {
                tracing::warn!("Failed to purge zcode start-plan rows: {e}");
                0
            });
        if deleted > 0 {
            tracing::info!(
                "Migrated zcode collection: removed {deleted} row(s) at/after {CUTOFF}, \
                 re-ingesting with provider='bigmodel-start' for trial-plan traffic"
            );
        }
        deleted
    }

    /// One-time migration helper for ZCode channels in the `account:`
    /// namespace (a BigModel plan bound to the signed-in account, e.g.
    /// `account:bigmodel-individual-coding-plan`).
    ///
    /// Those rows carry no billing metadata and the fallback only recognised
    /// the `builtin:` namespace, so the plan's GLM traffic was persisted as
    /// `provider='opencode-go'` — wrong vendor in the charts and the wrong
    /// billing formula (OpenCode Go divisor instead of the BigModel credit
    /// formula). The source now maps any `bigmodel*` channel to `bigmodel`, so
    /// delete the mislabeled window and let the startup re-parse re-ingest it
    /// under the correct provider. Like `purge_zcode_start_plan_bigmodel` this
    /// runs BEFORE `load_all()` (see `AppState::new`); the zcode DB still
    /// holds every row, so the history comes back with identical fingerprints.
    /// Cutoff = first `account:`-plan usage (2026-09-19 22:18 CST) with a
    /// margin, and `GLM%` keeps any genuine OpenCode Go traffic out of the
    /// window. Idempotent (later runs delete nothing).
    pub fn purge_zcode_account_plan_opencode_go(&self) -> usize {
        const CUTOFF: &str = "2026-09-19T14:00:00+00:00";
        let conn = match self.conn.lock() {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("Token store lock poisoned: {}", e);
                return 0;
            }
        };
        let deleted = conn
            .execute(
                "DELETE FROM token_records
                 WHERE source = 'zcode' AND provider = 'opencode-go'
                   AND model LIKE 'GLM%' AND time >= ?1",
                [CUTOFF],
            )
            .unwrap_or_else(|e| {
                tracing::warn!("Failed to purge zcode account-plan rows: {e}");
                0
            });
        if deleted > 0 {
            tracing::info!(
                "Migrated zcode collection: removed {deleted} row(s) at/after {CUTOFF} \
                 mislabeled provider='opencode-go', re-ingesting account-plan GLM as 'bigmodel'"
            );
        }
        deleted
    }

    /// One-time migration: drop persisted `source='opencode'` rows so they can
    /// be re-ingested with `output_tokens` including reasoning tokens.
    ///
    /// OpenCode bills reasoning at the output rate (the API's own `cost` field
    /// proves it — see `sources/opencode.rs`), but the parser used to discard
    /// the `reasoning` field, so every persisted row under-reports output.
    /// Folding it in changes the fingerprint, so keeping the old rows would
    /// duplicate the whole OpenCode history under new keys.
    ///
    /// Unlike the other purge helpers this one is **guarded**: it deletes only
    /// when `readable_rows` — what the OpenCode DB can still reproduce —
    /// covers every row we are about to drop. The dashboard store is meant to
    /// outlive the source files, so if the original sessions have already been
    /// cleaned up we keep the (slightly under-reported) history and retry on
    /// the next start rather than trade real data for a cosmetic fix.
    ///
    /// Must run before `load_all()` — see the call site in `AppState::new`.
    pub fn migrate_opencode_reasoning_output(&self, readable_rows: Option<i64>) -> usize {
        const WATERMARK: &str = "opencode_reasoning_output_v1";
        if self.get_sync_watermark(WATERMARK) == Some(1) {
            return 0; // Already migrated.
        }

        let existing: i64 = {
            let conn = match self.conn.lock() {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!("Token store lock poisoned: {}", e);
                    return 0;
                }
            };
            conn.query_row(
                "SELECT count(*) FROM token_records WHERE source = 'opencode'",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0)
        };

        if existing == 0 {
            // Nothing persisted predates the fix; rows land correctly from here on.
            self.set_sync_watermark(WATERMARK, 1);
            return 0;
        }

        match readable_rows {
            None => {
                tracing::warn!(
                    "OpenCode reasoning migration deferred: source DB unreadable, \
                     keeping {existing} under-reported row(s)"
                );
                return 0;
            }
            Some(readable) if readable < existing => {
                tracing::warn!(
                    "OpenCode reasoning migration skipped: source DB reproduces only \
                     {readable} of {existing} persisted row(s) (original sessions pruned?) \
                     — keeping history rather than losing it; will retry next start"
                );
                return 0;
            }
            Some(_) => {}
        }

        let deleted = {
            let conn = match self.conn.lock() {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!("Token store lock poisoned: {}", e);
                    return 0;
                }
            };
            conn.execute(
                "DELETE FROM token_records WHERE source = 'opencode'",
                [],
            )
            .unwrap_or_else(|e| {
                tracing::warn!("Failed to purge pre-reasoning opencode rows: {e}");
                0
            })
        };
        if deleted > 0 {
            // After the guard above is released — `set_sync_watermark` locks too.
            self.set_sync_watermark(WATERMARK, 1);
            tracing::info!(
                "Migrated opencode collection: removed {deleted} row(s) whose \
                 output_tokens omitted reasoning tokens, re-ingesting from the OpenCode DB"
            );
        }
        deleted
    }

    /// One-time migration helper: remove persisted `source='dim'` rows whose
    /// provider is the legacy `workbuddy` name. The workbuddy channel is now
    /// covered by the `dim-agent` source (workbuddy-usage.jsonl, written by
    /// the workbuddy plugin), so old rows would otherwise double-count the
    /// same usage. Idempotent (no-op once the rows are gone).
    pub fn purge_dim_workbuddy(&self) -> usize {
        let conn = match self.conn.lock() {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("Token store lock poisoned: {}", e);
                return 0;
            }
        };
        let deleted = conn
            .execute(
                "DELETE FROM token_records WHERE source = 'dim' AND provider = 'workbuddy'",
                [],
            )
            .unwrap_or_else(|e| {
                tracing::warn!("Failed to purge legacy dim workbuddy rows: {e}");
                0
            });
        if deleted > 0 {
            tracing::info!(
                "Migrated dim collection: removed {deleted} legacy workbuddy row(s) \
                 replaced by dim-agent records"
            );
        }
        deleted
    }

    /// One-time migration helper for the built-in Command Code proxy channel.
    ///
    /// DimAgent requests routed through the loopback cc-proxy are already
    /// metered per-request by the `cc-proxy` source (`cc-proxy-usage.jsonl`,
    /// written by the proxy itself). The dim local supplement used to also
    /// ingest the same requests as per-run rows from the local dimcode SQLite
    /// (`original_provider='cc-proxy'`, provider slug of the channel's
    /// displayName, e.g. `command-code`), double-counting them at a coarser
    /// granularity. Keyed on `original_provider` (the raw dim providerId) so
    /// rows persisted under any displayName slug variant are caught.
    /// Idempotent (no-op once the rows are gone).
    pub fn purge_dim_cc_proxy(&self) -> usize {
        let conn = match self.conn.lock() {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("Token store lock poisoned: {}", e);
                return 0;
            }
        };
        let deleted = conn
            .execute(
                "DELETE FROM token_records
                 WHERE source = 'dim' AND original_provider = 'cc-proxy'",
                [],
            )
            .unwrap_or_else(|e| {
                tracing::warn!("Failed to purge dim cc-proxy run rows: {e}");
                0
            });
        if deleted > 0 {
            tracing::info!(
                "Migrated dim collection: removed {deleted} cc-proxy run row(s) \
                 replaced by cc-proxy source records"
            );
        }
        deleted
    }

    /// One-time migration helper for the ZCode → Command Code channel.
    ///
    /// ZCode's `commandcode` channel is the built-in loopback cc-proxy, which
    /// meters every request into `cc-proxy-usage.jsonl` (`source='cc-proxy'`).
    /// The `model_usage` row ZCode writes for the same call was persisted as a
    /// second record (`source='zcode'`, `provider='commandcode'`), double
    /// counting tokens, cost and call count. The zcode source no longer emits
    /// those rows (see `PROXY_METERED_PROVIDERS`), so the stored twins go.
    /// Idempotent (no-op once the rows are gone).
    pub fn purge_zcode_commandcode(&self) -> usize {
        let conn = match self.conn.lock() {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("Token store lock poisoned: {}", e);
                return 0;
            }
        };
        let deleted = conn
            .execute(
                "DELETE FROM token_records WHERE source = 'zcode' AND provider = 'commandcode'",
                [],
            )
            .unwrap_or_else(|e| {
                tracing::warn!("Failed to purge zcode commandcode rows: {e}");
                0
            });
        if deleted > 0 {
            tracing::info!(
                "Migrated zcode collection: removed {deleted} commandcode row(s) \
                 already metered by the cc-proxy source"
            );
        }
        deleted
    }

    /// One-time migration helper for the Ollama Cloud channel switch.
    ///
    /// Dim's local `usage_run_stats` used to expose that channel as one row per
    /// DimAgent run. Those requests are now metered individually by the
    /// CLIProxyAPI `ollama-usage` plugin (`source='ollama-proxy'`), so any
    /// per-run row at or after the proxy's first record would double count the
    /// same usage. Rows *before* that instant are older history the proxy never
    /// saw and are kept.
    ///
    /// Comparison is done in Rust rather than SQL because stored timestamps mix
    /// `Z` and `+00:00` suffixes, which do not order lexicographically.
    /// Idempotent: once the superseded rows are gone this deletes nothing.
    pub fn purge_superseded_ollama_run_rows(&self, raw_provider: &str) -> usize {
        let conn = match self.conn.lock() {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("Token store lock poisoned: {}", e);
                return 0;
            }
        };
        let to_delete: Vec<i64> = {
            let mut stmt = match conn.prepare(
                "SELECT id, time FROM token_records
                 WHERE original_provider = ?1",
            ) {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!("Failed to prepare ollama run purge query: {e}");
                    return 0;
                }
            };
            let iter = stmt.query_map([raw_provider], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            });
            match iter {
                Ok(rows) => rows
                    .flatten()
                    .filter(|(_, time)| crate::sources::ollama_run_time_superseded(time))
                    .map(|(id, _)| id)
                    .collect(),
                Err(e) => {
                    tracing::warn!("Failed to iterate ollama run rows for purge: {e}");
                    return 0;
                }
            }
        };
        if to_delete.is_empty() {
            return 0;
        }
        let mut deleted = 0usize;
        for id in &to_delete {
            match conn.execute("DELETE FROM token_records WHERE id = ?1", [id]) {
                Ok(n) => deleted += n,
                Err(e) => {
                    tracing::warn!("Failed to purge superseded ollama run row {id}: {e}");
                }
            }
        }
        if deleted > 0 {
            tracing::info!(
                "Migrated Ollama Cloud channel: removed {deleted} per-run row(s) \
                 superseded by ollama-proxy per-request records"
            );
        }
        deleted
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
                    r.date.as_str(),
                    r.api_key_prefix.as_str(),
                    r.provider.as_str(),
                    r.original_provider,
                    r.model.as_str(),
                    r.source.as_str(),
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
/// elapsed since the oldest queued record *or* since the last flush — at most
/// one write per `delay`, and every queued record is persisted within `delay`
/// of arrival. [`PendingBuffer::take_all`] is the shutdown path: it drains
/// unconditionally so the final write on exit is reliable.
///
/// `take_if_due` takes an explicit `now` so the debounce logic is testable
/// with fake time.
pub struct PendingBuffer {
    records: Mutex<Vec<TokenRecord>>,
    /// When the *current* backlog started, i.e. when records were first queued
    /// into an empty buffer. This is what bounds the persistence delay: with
    /// only a "last queued" timestamp, a source that queues something every few
    /// seconds (the common case — dim/cc-proxy/codex all refresh on the same 30s
    /// tick) would keep pushing the deadline forward and the buffer would never
    /// come due. See `pending_buffer_drains_even_under_continuous_queueing`.
    oldest_queued: Mutex<Option<Instant>>,
    last_flush: Mutex<Option<Instant>>,
}

impl PendingBuffer {
    pub fn new() -> Self {
        Self {
            records: Mutex::new(Vec::new()),
            oldest_queued: Mutex::new(None),
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
        let was_empty = buf.is_empty();
        buf.extend(records);
        if was_empty {
            // New backlog: remember when it started so it cannot be starved by
            // records that keep arriving.
            *self.oldest_queued.lock().unwrap() = Some(now);
        }
    }

    /// Number of records waiting to be written.
    pub fn len(&self) -> usize {
        self.records.lock().unwrap().len()
    }

    /// Drain the buffer if a write is due: at least `delay` since the oldest
    /// queued record, or since the last flush. Returns the batch to write
    /// (empty if not due). A write that is drained here is considered a flush,
    /// so the next one cannot happen before `delay` again.
    ///
    /// The `oldest_queued` arm makes the debounce a **maximum** latency rather
    /// than a best-effort one: `Flushed 1 queued record(s)` every 30 seconds
    /// (each refresh tick queues something) is normal, but with only
    /// `last_queued` the deadline moved forward on every tick and a busy day
    /// could leave records in memory indefinitely — the observed 7h gap on
    /// 2026-09-11 left 1000+ records unpublished to SQLite.
    pub fn take_if_due(&self, delay: Duration, now: Instant) -> Vec<TokenRecord> {
        let mut buf = self.records.lock().unwrap();
        if buf.is_empty() {
            return Vec::new();
        }
        let oldest_due = self
            .oldest_queued
            .lock()
            .unwrap()
            .is_some_and(|t| now.duration_since(t) >= delay);
        let flush_due = self
            .last_flush
            .lock()
            .unwrap()
            .is_some_and(|t| now.duration_since(t) >= delay);
        if oldest_due || flush_due {
            let batch = std::mem::take(&mut *buf);
            *self.last_flush.lock().unwrap() = Some(now);
            *self.oldest_queued.lock().unwrap() = None;
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

/// Read a TEXT column into a `CompactString`.
///
/// Goes through `ValueRef` rather than `row.get::<String>()` so the ~5M text
/// columns `load_all` reads per startup don't each pay for a `String` that is
/// immediately dropped. The columns are all `NOT NULL`, so a non-text value is
/// a genuine schema violation and propagates exactly as `row.get` did.
fn text_col(row: &rusqlite::Row<'_>, idx: usize) -> rusqlite::Result<CompactString> {
    Ok(row.get_ref(idx)?.as_str()?.into())
}

fn row_to_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<TokenRecord> {
    Ok(TokenRecord {
        time: row.get(0)?,
        date: text_col(row, 1)?,
        api_key_prefix: text_col(row, 2)?,
        provider: text_col(row, 3)?,
        original_provider: row.get(4)?,
        model: text_col(row, 5)?,
        source: text_col(row, 6)?,
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
            date: time[..10].into(),
            time: time.to_string(),
            api_key_prefix: "sk-test".into(),
            provider: provider.into(),
            original_provider: None,
            model: model.into(),
            source: source.into(),
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
    fn codebuddy_raw_credit_survives_roundtrip() {
        let store = temp_store();
        let mut record = fixture(
            "codebuddy",
            "codebuddy",
            "gpt-5.6-luna",
            "2026-08-29T04:44:13.879Z",
            100,
        );
        record.cost = 1.04;

        assert_eq!(store.insert_batch(&[record.clone()]), 1);
        let loaded = store.load_all();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0], record);
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

    fn seed_legacy_commandcode(path: &Path, rows: &[(&str, i64, i64, i64)]) {
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
    fn purge_dim_grok_build_removes_only_legacy_rows() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("token-stats.db");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        let insert = |time: &str, provider: &str, source: &str, input: i64, cache: i64| {
            conn.execute(
                INSERT_SQL,
                params![
                    time,
                    &time[..10],
                    "N/A",
                    provider,
                    None::<String>,
                    "grok-4.6",
                    source,
                    input,
                    100i64,
                    cache,
                    0i64,
                    input + 100 + cache,
                    0.0f64,
                    None::<f64>,
                    None::<f64>,
                ],
            )
            .unwrap();
        };
        insert(
            "2026-09-06T14:15:21.333Z",
            "grok-build",
            "dim",
            296_933,
            2_085_504,
        );
        insert(
            "2026-09-06T14:15:21.333Z",
            "xai-official",
            "dim",
            296_933,
            2_085_504,
        );
        insert(
            "2026-09-06T14:15:21.333Z",
            "xai-official",
            "grok-cli",
            100,
            0,
        );
        drop(conn);

        let store = TokenStore::open(&path);
        assert_eq!(store.count(), 3);
        let deleted = store.purge_dim_grok_build();
        assert_eq!(deleted, 1);
        let loaded = store.load_all();
        assert_eq!(loaded.len(), 2);
        assert!(
            loaded.iter().all(|r| r.provider != "grok-build"),
            "legacy grok-build row should be purged"
        );
        // Idempotent: second call removes nothing.
        assert_eq!(store.purge_dim_grok_build(), 0);
    }

    #[test]
    fn purge_zcode_commandcode_removes_only_proxy_twins() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("token-stats.db");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        let insert = |source: &str, provider: &str| {
            conn.execute(
                INSERT_SQL,
                params![
                    "2026-09-19T13:10:49.307+00:00",
                    "2026-09-19",
                    "N/A",
                    provider,
                    None::<String>,
                    "deepseek-v4.1-flash",
                    source,
                    625i64,
                    2013i64,
                    92800i64,
                    0i64,
                    95438i64,
                    0.0f64,
                    None::<f64>,
                    None::<f64>,
                ],
            )
            .unwrap();
        };
        // The same call, once per the two sources.
        insert("zcode", "commandcode");
        insert("cc-proxy", "commandcode");
        insert("zcode", "bigmodel");
        drop(conn);

        let store = TokenStore::open(&path);
        assert_eq!(store.count(), 3);
        assert_eq!(store.purge_zcode_commandcode(), 1);
        let loaded = store.load_all();
        assert_eq!(loaded.len(), 2);
        assert!(
            loaded.iter().all(|r| !(r.source == "zcode" && r.provider == "commandcode")),
            "proxy-metered zcode row should be purged"
        );
        // Idempotent: second call removes nothing.
        assert_eq!(store.purge_zcode_commandcode(), 0);
    }

    #[test]
    fn purge_zcode_account_plan_opencode_go_relabels_only_the_plan_window() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("token-stats.db");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        let insert = |time: &str, provider: &str, model: &str| {
            conn.execute(
                INSERT_SQL,
                params![
                    time,
                    &time[..10],
                    "N/A",
                    provider,
                    None::<String>,
                    model,
                    "zcode",
                    100i64,
                    20i64,
                    0i64,
                    0i64,
                    120i64,
                    0.0f64,
                    None::<f64>,
                    None::<f64>,
                ],
            )
            .unwrap();
        };
        // Account-bound BigModel plan mislabeled as OpenCode Go.
        insert(
            "2026-09-19T14:19:10.655+00:00",
            "opencode-go",
            "GLM-5.3-Flash",
        );
        // Genuine OpenCode Go traffic and pre-cutoff history must survive, and
        // so must the CPA channel whose model name carries a `wb/` prefix.
        insert(
            "2026-09-19T15:19:10.655+00:00",
            "opencode-go",
            "deepseek-v4-flash",
        );
        insert(
            "2026-09-19T15:20:10.655+00:00",
            "opencode-go",
            "wb/glm-5.3-flash",
        );
        insert(
            "2026-09-18T14:19:10.655+00:00",
            "opencode-go",
            "GLM-5.3-Flash",
        );
        insert("2026-09-19T15:21:10.655+00:00", "bigmodel", "GLM-5.3-Flash");
        drop(conn);

        let store = TokenStore::open(&path);
        assert_eq!(store.count(), 5);
        assert_eq!(store.purge_zcode_account_plan_opencode_go(), 1);
        let loaded = store.load_all();
        assert_eq!(loaded.len(), 4);
        assert!(
            loaded
                .iter()
                .all(|r| !(r.provider == "opencode-go" && r.model == "GLM-5.3-Flash"
                    && r.time == "2026-09-19T14:19:10.655+00:00")),
            "mislabeled account-plan row should be gone: {loaded:?}"
        );
        // Idempotent: second call removes nothing.
        assert_eq!(store.purge_zcode_account_plan_opencode_go(), 0);
    }

    #[test]
    fn purge_superseded_ollama_run_rows_keeps_history_before_cutoff() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("token-stats.db");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        let insert = |time: &str, original: &str, source: &str, model: &str| {
            conn.execute(
                INSERT_SQL,
                params![
                    time,
                    &time[..10],
                    "N/A",
                    // Vendor merge renamed the provider before persistence.
                    if source == "dim" {
                        "ollama"
                    } else {
                        "ollama-cloud"
                    },
                    original,
                    model,
                    source,
                    1_000i64,
                    100i64,
                    5_000i64,
                    0i64,
                    6_100i64,
                    0.0f64,
                    None::<f64>,
                    None::<f64>,
                ],
            )
            .unwrap();
        };
        insert(
            "2026-09-05T23:24:12.948+00:00",
            crate::sources::OLLAMA_CLOUD_RUN_PROVIDER,
            "dim",
            "deepseek-v4-flash:0731",
        );
        insert(
            "2026-09-11T14:30:00+00:00",
            crate::sources::OLLAMA_CLOUD_RUN_PROVIDER,
            "dim",
            "deepseek-v4.1-flash",
        );
        // A per-request record from the new source is never touched.
        insert(
            "2026-09-11T14:30:00+00:00",
            "ollama-cloud",
            "ollama-proxy",
            "deepseek-v4.1-flash",
        );
        drop(conn);

        let log_path = dir.path().join("ollama-usage.jsonl");
        std::fs::write(
            &log_path,
            concat!(
                r#"{"date":"2026-09-11","time":"2026-09-11T14:00:00Z","apiKeyPrefix":"N/A","provider":"ollama-cloud","model":"deepseek-v4.1-flash","source":"ollama-proxy","inputTokens":1,"outputTokens":1,"cacheReadTokens":0,"cacheWriteTokens":0,"totalTokens":2,"cost":0.0}"#,
                "\n"
            ),
        )
        .unwrap();

        temp_env::with_var("OLLAMA_PROXY_USAGE_LOG_PATH", Some(&log_path), || {
            let store = TokenStore::open(&path);
            assert_eq!(store.count(), 3);
            assert_eq!(
                store.purge_superseded_ollama_run_rows(crate::sources::OLLAMA_CLOUD_RUN_PROVIDER),
                1
            );
            let loaded = store.load_all();
            assert_eq!(loaded.len(), 2, "only the post-cutoff run row is dropped");
            assert!(
                loaded.iter().any(|r| r.source == "ollama-proxy"),
                "per-request records must survive"
            );
            assert!(
                loaded
                    .iter()
                    .any(|r| r.source == "dim" && r.time.starts_with("2026-09-05")),
                "pre-cutoff run history must be kept"
            );
            // Idempotent.
            assert_eq!(
                store.purge_superseded_ollama_run_rows(crate::sources::OLLAMA_CLOUD_RUN_PROVIDER),
                0
            );
        });
    }

    #[test]
    fn collapse_after_insert_drops_inclusive_twin() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("token-stats.db");
        seed_legacy_commandcode(&path, &[("2026-08-23T12:32:48.057Z", 23024, 7424, 71)]);

        let store = TokenStore::open(&path);
        assert_eq!(store.count(), 1);
        let exclusive = TokenRecord {
            date: "2026-08-23".into(),
            time: "2026-08-23T12:32:48.057Z".to_string(),
            api_key_prefix: "N/A".into(),
            provider: "commandcode".into(),
            original_provider: None,
            model: "muse-spark-1.2-contributor".into(),
            source: "commandcode".into(),
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
        unknown.model = "unknown".into();
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
        assert!(
            buf.take_if_due(Duration::from_secs(120), t0 + Duration::from_secs(60))
                .is_empty()
        );
        // Due: 2 min after the queue.
        let batch = buf.take_if_due(Duration::from_secs(120), t0 + Duration::from_secs(121));
        assert_eq!(batch.len(), 1);
        assert_eq!(buf.len(), 0);
    }

    #[test]
    fn pending_buffer_flushes_at_most_once_per_delay_under_continuous_load() {
        let buf = PendingBuffer::new();
        let t0 = Instant::now();

        // First batch: queued at t0, flushed 2 min later (due via oldest_queued).
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
        // last_queued forward. The oldest_queued arm still bounds the delay.
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
            // Early: the backlog is younger than the delay.
            assert!(buf.take_if_due(Duration::from_secs(120), at).is_empty());
        }

        // 2 min after the backlog started (t0+150s) the oldest_queued arm fires
        // and drains everything in one batch — a steady queue every 30s cannot
        // postpone the write past the delay.
        let batch = buf.take_if_due(Duration::from_secs(120), t0 + Duration::from_secs(271));
        assert_eq!(batch.len(), 3, "all queued records flush in one batch");
        assert_eq!(buf.len(), 0);
    }

    /// Regression: a source queueing something on every refresh tick (30s)
    /// must not be able to starve the buffer. Before the `oldest_queued` arm
    /// existed, `last_queued` was pushed forward on every tick and the buffer
    /// could stay unflushed indefinitely — 2026-09-11 saw a 7h gap and 1000+
    /// records published to memory but not to SQLite.
    #[test]
    fn pending_buffer_drains_even_under_continuous_queueing() {
        let buf = PendingBuffer::new();
        let t0 = Instant::now();
        let delay = Duration::from_secs(120);
        let mut flushed_at = None;
        for i in 0..40 {
            let at = t0 + Duration::from_secs(30 * i);
            buf.queue_at(
                vec![fixture(
                    "dim",
                    "ollama",
                    "deepseek-v4.1-flash",
                    &format!("2026-07-01T01:{:02}:00Z", i),
                    100,
                )],
                at,
            );
            if !buf.take_if_due(delay, at).is_empty() {
                flushed_at = Some(at);
                break;
            }
        }
        let flushed_at = flushed_at.expect("a steady queue must still drain");
        assert_eq!(
            flushed_at,
            t0 + Duration::from_secs(120),
            "drain at the first tick where the backlog has aged past the delay"
        );
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
        assert!(
            buf.take_if_due(Duration::from_secs(120), Instant::now())
                .is_empty()
        );
    }

    fn opencode_row_count(store: &TokenStore) -> usize {
        store
            .load_all()
            .iter()
            .filter(|r| r.source == "opencode")
            .count()
    }

    fn seed_opencode_rows(store: &TokenStore, n: usize) {
        let records: Vec<TokenRecord> = (0..n)
            .map(|i| fixture("opencode", "opencode-go", "space-bunny-free", &format!("2026-09-2{}T01:00:00Z", i % 9), 100 + i as i64))
            .collect();
        store.insert_batch(&records);
    }

    #[test]
    fn opencode_reasoning_migration_purges_when_source_can_reproduce() {
        let store = temp_store();
        seed_opencode_rows(&store, 3);
        store.insert_batch(&[fixture("codex", "openai", "gpt-5.5", "2026-09-20T01:00:00Z", 300)]);

        // The OpenCode DB reproduces every row → safe to drop and re-ingest.
        assert_eq!(store.migrate_opencode_reasoning_output(Some(3)), 3);
        assert_eq!(opencode_row_count(&store), 0, "pre-fix rows are gone");
        assert_eq!(store.count(), 1, "other sources are untouched");

        // Re-ingesting the same usage under the new fingerprints lands normally.
        let mut regenerated = fixture(
            "opencode",
            "opencode-go",
            "space-bunny-free",
            "2026-09-20T01:00:00Z",
            140,
        );
        regenerated.output_tokens = 90; // output + reasoning
        assert_eq!(store.insert_batch(&[regenerated]), 1);
        assert_eq!(opencode_row_count(&store), 1);
    }

    #[test]
    fn opencode_reasoning_migration_is_one_shot() {
        let store = temp_store();
        seed_opencode_rows(&store, 2);
        assert_eq!(store.migrate_opencode_reasoning_output(Some(99)), 2);
        assert_eq!(opencode_row_count(&store), 0);

        seed_opencode_rows(&store, 2);
        assert_eq!(
            store.migrate_opencode_reasoning_output(Some(99)),
            0,
            "watermarked: never purges again"
        );
        assert_eq!(opencode_row_count(&store), 2);
    }

    #[test]
    fn opencode_reasoning_migration_keeps_history_the_source_cannot_reproduce() {
        let store = temp_store();
        seed_opencode_rows(&store, 3);

        // Sessions were pruned since: the DB covers fewer rows than we hold,
        // so deleting would permanently lose usage.
        assert_eq!(store.migrate_opencode_reasoning_output(Some(2)), 0);
        assert_eq!(opencode_row_count(&store), 3, "history survives");

        assert_eq!(store.migrate_opencode_reasoning_output(None), 0);
        assert_eq!(opencode_row_count(&store), 3, "unreadable DB is not a licence to delete");

        // The guard is not sticky — a later start with an intact DB migrates.
        assert_eq!(store.migrate_opencode_reasoning_output(Some(3)), 3);
        assert_eq!(opencode_row_count(&store), 0);
    }

    #[test]
    fn opencode_reasoning_migration_marks_empty_store_without_touching_it() {
        let store = temp_store();
        store.insert_batch(&[fixture("codex", "openai", "gpt-5.5", "2026-09-20T01:00:00Z", 300)]);

        assert_eq!(store.migrate_opencode_reasoning_output(None), 0);
        assert_eq!(store.count(), 1);

        // Fresh install: rows arrive with correct numbers, so nothing to redo.
        store.insert_batch(&[fixture(
            "opencode",
            "opencode-go",
            "space-bunny-free",
            "2026-09-21T01:00:00Z",
            100,
        )]);
        assert_eq!(store.migrate_opencode_reasoning_output(Some(1)), 0);
        assert_eq!(opencode_row_count(&store), 1);
    }
}
