use super::DataSource;
use crate::models::TokenRecord;
use std::path::PathBuf;

/// Dim (dimcode) source: reads `~/.dimcode/v2/dimcode.sqlite` (SQLite).
///
/// Dim is an OpenCode fork and persists per-run usage into the
/// `usage_run_stats` table (one row per completed run). Each row carries the
/// final token counts plus an exact USD cost computed from the provider
/// catalog (Dim's claimed "deepseek July pre-increase" API prices).
///
/// Token convention is OpenAI-style: `inputTokens` INCLUDES `cacheReadTokens`.
/// We subtract to normalize to the Anthropic convention used everywhere else
/// (same as the Codex / ZCode / Qoder parsers).
#[derive(Default)]
pub struct DimSource;

impl DataSource for DimSource {
    fn name(&self) -> &'static str {
        "dim"
    }

    fn load(&self) -> Vec<TokenRecord> {
        let path = Self::db_path();
        tracing::info!("Loading Dim data from: {:?}", path);
        let records = Self::parse(&path);
        tracing::info!("Loaded {} dim records", records.len());
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
        vec![Self::db_path()]
    }

    fn is_available(&self) -> bool {
        Self::db_path().exists()
    }
}

impl DimSource {
    fn db_path() -> PathBuf {
        std::env::var("DIM_DB_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                super::home_dir()
                    .join(".dimcode")
                    .join("v2")
                    .join("dimcode.sqlite")
            })
    }

    fn parse(path: &std::path::Path) -> Vec<TokenRecord> {
        if !path.exists() {
            tracing::warn!("Dim DB not found at {:?}, skipping", path);
            return Vec::new();
        }

        let conn = match rusqlite::Connection::open_with_flags(
            path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        ) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("Failed to open Dim DB: {}, skipping", e);
                return Vec::new();
            }
        };

        // One row per completed run. Skip incomplete runs and zero-usage rows
        // (the aggregated ledger row for a still-active session or an empty run).
        let sql = "SELECT sessionId, providerId, modelId,
                          startedAt, endedAt, createdAt,
                          inputTokens, outputTokens,
                          cacheReadTokens, cacheWriteTokens, cost
                   FROM usage_run_stats
                   WHERE status = 'completed'
                     AND (inputTokens > 0 OR outputTokens > 0
                          OR cacheReadTokens > 0 OR cacheWriteTokens > 0)";

        let mut stmt = match conn.prepare(sql) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("Failed to prepare Dim query: {}, skipping", e);
                return Vec::new();
            }
        };

        let mut records = Vec::new();
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, i64>(6)?,
                row.get::<_, i64>(7)?,
                row.get::<_, Option<i64>>(8)?,
                row.get::<_, Option<i64>>(9)?,
                row.get::<_, String>(10)?,
            ))
        });

        match rows {
            Ok(r) => {
                for row in r.flatten() {
                    let (
                        _session_id,
                        provider_id,
                        model_id,
                        started_at,
                        ended_at,
                        created_at,
                        input_tokens,
                        output_tokens,
                        cache_read_tokens,
                        cache_write_tokens,
                        cost_json,
                    ) = row;
                    if let Some(record) = Self::to_record(
                        &provider_id,
                        &model_id,
                        started_at.as_deref(),
                        ended_at.as_deref(),
                        &created_at,
                        input_tokens,
                        output_tokens,
                        cache_read_tokens,
                        cache_write_tokens,
                        &cost_json,
                    ) {
                        records.push(record);
                    }
                }
            }
            Err(e) => tracing::warn!("Failed to iterate Dim usage_run_stats rows: {}", e),
        }

        records
    }

    fn to_record(
        _provider_id: &str,
        model_id: &str,
        started_at: Option<&str>,
        ended_at: Option<&str>,
        created_at: &str,
        input_tokens: i64,
        output_tokens: i64,
        cache_read_tokens: Option<i64>,
        cache_write_tokens: Option<i64>,
        cost_json: &str,
    ) -> Option<TokenRecord> {
        let cache_read_tokens = cache_read_tokens.unwrap_or(0);
        let cache_write_tokens = cache_write_tokens.unwrap_or(0);
        // OpenAI convention: inputTokens includes cacheReadTokens → subtract.
        let effective_input = (input_tokens - cache_read_tokens).max(0);
        let total = effective_input + output_tokens + cache_read_tokens + cache_write_tokens;

        // Prefer completion time, fall back to start, then creation.
        let ts = ended_at.or(started_at).unwrap_or(created_at);
        let (date, time) = super::parse_iso_timestamp(ts);

        // All usage from the dim agent belongs to the `dim` vendor.
        let provider = "dim".to_string();

        // Exact catalog-computed USD cost (Dim's provider catalog = July
        // deepseek API prices). Parse totalCostUsd from the cost JSON.
        let cost = serde_json::from_str::<serde_json::Value>(cost_json)
            .ok()
            .and_then(|v| v.get("totalCostUsd").and_then(|c| c.as_f64()))
            .unwrap_or(0.0);

        // Dim bills in USD. Mark original_provider so the provider=="deepseek"
        // CNY-as-is special case in display_cost() is bypassed; this routes the
        // stored USD cost through the standard USD→CNY conversion instead.
        let original_provider = Some("dim".to_string());

        Some(TokenRecord {
            date,
            time,
            api_key_prefix: "N/A".to_string(),
            provider,
            original_provider,
            model: model_id.to_string(),
            source: "dim".to_string(),
            input_tokens: effective_input,
            output_tokens,
            cache_read_tokens: cache_read_tokens,
            cache_write_tokens: cache_write_tokens,
            total_tokens: total,
            cost,
            ttft_ms: None,
            tps: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    /// Build a temp dim DB with one completed row per entry.
    /// Each entry is `model|createdAt|endedAt|input|output|cacheRead|cacheWrite|totalCostUsd`.
    fn make_db(entries: &[&str]) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("dimcode.sqlite");
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE usage_run_stats (
                runId text primary key,
                sessionId text not null,
                providerId text not null,
                modelId text not null,
                status text not null,
                startedAt text,
                endedAt text,
                inputTokens integer not null,
                outputTokens integer not null,
                totalTokens integer not null,
                cacheReadTokens integer,
                cacheWriteTokens integer,
                cost text not null,
                pricing text not null,
                createdAt text not null,
                updatedAt text not null
            );",
        )
        .unwrap();
        for (i, entry) in entries.iter().enumerate() {
            let f: Vec<&str> = entry.split('|').collect();
            let input: i64 = f[3].parse().unwrap();
            let output: i64 = f[4].parse().unwrap();
            let cache_read: i64 = f[5].parse().unwrap();
            let cache_write: i64 = f[6].parse().unwrap();
            let cost: f64 = f[7].parse().unwrap();
            conn.execute(
                "INSERT INTO usage_run_stats (runId, sessionId, providerId,
                    modelId, status, startedAt, endedAt, inputTokens,
                    outputTokens, totalTokens, cacheReadTokens, cacheWriteTokens,
                    cost, pricing, createdAt, updatedAt)
                 VALUES (?1, 'sess-1', 'dimcode-api-oauth', ?2, 'completed',
                         ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, '{}', '2026-08-22T00:00:00Z',
                         '2026-08-22T00:00:00Z')",
                rusqlite::params![
                    format!("run-{i}"),
                    f[0],
                    f[1],
                    f[2],
                    input,
                    output,
                    input + output,
                    cache_read,
                    cache_write,
                    format!(
                        "{{\"inputCostUsd\":0,\"outputCostUsd\":0,\"cacheReadCostUsd\":0,\"totalCostUsd\":{cost},\"quality\":\"exact\"}}"
                    ),
                ],
            )
            .unwrap();
        }
        drop(conn);
        (dir, db_path)
    }

    #[test]
    fn normalizes_openai_cache_convention() {
        // Real dim sample: input=5321963 includes cacheRead=5246464.
        let (_dir, db_path) = make_db(&[
            "deepseek-v4-flash-vision-exp|2026-08-22T12:11:31Z|2026-08-22T12:22:26Z|5321963|46865|5246464|0|0.0383821592",
        ]);
        let records = DimSource::parse(&db_path);

        assert_eq!(records.len(), 1);
        let r = &records[0];
        assert_eq!(r.provider, "dim");
        assert_eq!(r.model, "deepseek-v4-flash-vision-exp");
        assert_eq!(r.source, "dim");
        assert_eq!(r.input_tokens, 75499); // 5321963 - 5246464
        assert_eq!(r.output_tokens, 46865);
        assert_eq!(r.cache_read_tokens, 5246464);
        assert_eq!(r.cache_write_tokens, 0);
        // effective_input + output + cache_read + cache_write = 5368828
        assert_eq!(r.total_tokens, 75499 + 46865 + 5246464 + 0);
        assert!((r.cost - 0.0383821592).abs() < 1e-9);
        assert_eq!(r.original_provider.as_deref(), Some("dim"));
        assert_eq!(r.date, "2026-08-22");
    }

    #[test]
    fn handles_null_cache_columns() {
        // Real dim DB has NULL cacheWriteTokens (and possibly cacheReadTokens).
        // These must not cause the row to be dropped (flake via row.get::<i64>).
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("dimcode.sqlite");
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE usage_run_stats (
                runId text primary key, sessionId text not null,
                providerId text not null, modelId text not null, status text not null,
                startedAt text, endedAt text, inputTokens integer not null,
                outputTokens integer not null, totalTokens integer not null,
                cacheReadTokens integer, cacheWriteTokens integer,
                cost text not null, pricing text not null,
                createdAt text not null, updatedAt text not null
            );",
        )
        .unwrap();
        // cacheReadTokens and cacheWriteTokens left NULL.
        conn.execute(
            "INSERT INTO usage_run_stats (runId, sessionId, providerId,
                modelId, status, startedAt, endedAt, inputTokens,
                outputTokens, totalTokens, cacheReadTokens, cacheWriteTokens,
                cost, pricing, createdAt, updatedAt)
             VALUES ('run-1', 'sess', 'dimcode-api-oauth', 'deepseek-v4-flash',
                     'completed', '2026-08-22T00:00:00Z', '2026-08-22T00:01:00Z',
                     1000, 50, 1050, NULL, NULL, '{}', '{}',
                     '2026-08-22T00:00:00Z', '2026-08-22T00:00:00Z')",
            [],
        )
        .unwrap();
        drop(conn);

        let records = DimSource::parse(&db_path);
        assert_eq!(records.len(), 1, "NULL cache columns must not drop the row");
        let r = &records[0];
        assert_eq!(r.input_tokens, 1000);
        assert_eq!(r.cache_read_tokens, 0);
        assert_eq!(r.cache_write_tokens, 0);
        assert_eq!(r.total_tokens, 1050);
    }

    #[test]
    fn maps_non_deepseek_model_to_resolved_provider_or_raw_id() {
        // All dim records are attributed to the `dim` vendor regardless of model/providerId.
        let (_dir, db_path) = make_db(&[
            "glm-5.2|2026-08-22T12:11:31Z|2026-08-22T12:22:26Z|1000|200|300|0|0.001",
        ]);
        let records = DimSource::parse(&db_path);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].provider, "dim");
    }

    #[test]
    fn skips_zero_usage_and_non_completed_rows() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("dimcode.sqlite");
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE usage_run_stats (
                runId text primary key, sessionId text not null,
                providerId text not null, modelId text not null, status text not null,
                startedAt text, endedAt text, inputTokens integer not null,
                outputTokens integer not null, totalTokens integer not null,
                cacheReadTokens integer, cacheWriteTokens integer,
                cost text not null, pricing text not null,
                createdAt text not null, updatedAt text not null
            );",
        )
        .unwrap();
        for (id, status, input, output) in [
            ("ok", "completed", 100, 20),
            ("err", "error", 500, 0),
            ("zero", "completed", 0, 0),
        ] {
            conn.execute(
                "INSERT INTO usage_run_stats (runId, sessionId, providerId,
                    modelId, status, startedAt, endedAt, inputTokens,
                    outputTokens, totalTokens, cacheReadTokens, cacheWriteTokens,
                    cost, pricing, createdAt, updatedAt)
                 VALUES (?1, 'sess', 'p', 'deepseek-v4-flash', ?2,
                         '2026-08-22T00:00:00Z', '2026-08-22T00:01:00Z', ?3, ?4,
                         ?5, 0, 0, '{}', '{}', '2026-08-22T00:00:00Z',
                         '2026-08-22T00:00:00Z')",
                rusqlite::params![id, status, input, output, input + output],
            )
            .unwrap();
        }
        drop(conn);

        let records = DimSource::parse(&db_path);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].input_tokens, 100);
        assert_eq!(records[0].output_tokens, 20);
        assert_eq!(records[0].total_tokens, 120);
    }

    #[test]
    fn missing_db_returns_empty() {
        let dir = tempdir().unwrap();
        let records = DimSource::parse(&dir.path().join("nope.sqlite"));
        assert!(records.is_empty());
    }
}
