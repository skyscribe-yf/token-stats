use super::DataSource;
use crate::models::TokenRecord;
use std::collections::HashSet;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

/// Qoder CLI (`qodercli`) source: `~/.qoder/logs/sessions/<project>/<session>/segments/*.jsonl`.
#[derive(Default)]
pub struct QoderCliSource;

/// Qoder Desktop source: the same segment logs under `~/.qoder-cn/`, written by
/// the desktop app's embedded agent SDK (`/opt/Qoder CN/qoder-cn`).
///
/// The CN gateway does not report usage for the free model: every
/// `model.response.completed` event carries zeros (measured 2026-09-18 on
/// 463/463 events; the app's own context snapshot says
/// `tokenCountsAvailable: false`). The calls themselves are real, so the
/// records are kept and stay visible via
/// [`TokenRecord::counts_as_call_without_tokens`].
#[derive(Default)]
pub struct QoderDesktopSource;

/// Session segment logs, newest event per line. `model.response.completed`
/// carries the per-request usage with the OpenAI convention (`input_tokens`
/// INCLUDES `cache_read_input_tokens`), normalized here to the Anthropic
/// convention the rest of the store uses (input = non-cached only).
fn load_segments(base_path: &Path, source: &str) -> Vec<TokenRecord> {
    parse_segment_files(&jsonl_files(base_path), &[], source)
}

/// Incremental variant: re-parse only the segment files whose (mtime, size)
/// changed since the previous refresh.
fn load_segments_incremental(
    base_path: &Path,
    source: &str,
    mark: impl Fn(&[PathBuf]),
) -> Vec<TokenRecord> {
    let files = jsonl_files(base_path);
    let changed = super::changed_files(&files);
    let records = if changed.is_empty() {
        Vec::new()
    } else {
        parse_segment_files(&files, &changed, source)
    };
    mark(&files);
    records
}

fn jsonl_files(base_path: &Path) -> Vec<PathBuf> {
    if !base_path.exists() {
        return Vec::new();
    }
    match super::walkdir(base_path) {
        Ok(entries) => entries
            .into_iter()
            .filter(|p| p.to_string_lossy().ends_with(".jsonl"))
            .collect(),
        Err(_) => Vec::new(),
    }
}

fn parse_segment_files(
    paths: &[PathBuf],
    subset: &[PathBuf],
    source: &str,
) -> Vec<TokenRecord> {
    let mut records = Vec::new();
    let mut seen: HashSet<String> = HashSet::new(); // dedup by request_id

    for path in paths {
        if !subset.is_empty() && !subset.contains(path) {
            continue;
        }

        let file = match File::open(path) {
            Ok(f) => f,
            Err(_) => continue,
        };

        for line in BufReader::new(file).lines().map_while(Result::ok) {
            if line.trim().is_empty() {
                continue;
            }
            let Ok(obj) = serde_json::from_str::<serde_json::Value>(&line) else {
                continue;
            };
            if obj.get("type").and_then(|t| t.as_str()) != Some("model.response.completed") {
                continue;
            }

            let request_id = obj
                .get("request_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if request_id.is_empty() || !seen.insert(request_id) {
                continue;
            }

            let Some(data) = obj.get("data") else { continue };
            let read = |key: &str| data.get(key).and_then(|v| v.as_i64()).unwrap_or(0);
            let raw_input = read("input_tokens");
            let output_tokens = read("output_tokens");
            let cache_read_tokens = read("cache_read_input_tokens");
            let cache_write_tokens = read("cache_creation_input_tokens");

            let input_tokens = (raw_input - cache_read_tokens).max(0);

            let model = data
                .get("model")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();
            let provider = data
                .get("provider")
                .and_then(|v| v.as_str())
                .filter(|p| !p.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| super::resolve_provider_from_model(&model));

            let (date, time) = parse_timestamp(obj.get("ts").and_then(|t| t.as_str()).unwrap_or(""));

            records.push(TokenRecord {
                date,
                time,
                api_key_prefix: "N/A".to_string(),
                provider,
                original_provider: None,
                model,
                source: source.to_string(),
                input_tokens,
                output_tokens,
                cache_read_tokens,
                cache_write_tokens,
                total_tokens: input_tokens
                    + output_tokens
                    + cache_read_tokens
                    + cache_write_tokens,
                cost: 0.0,
                ttft_ms: None,
                tps: None,
            });
        }
    }

    records
}

/// Parse an event timestamp such as "2026-09-18T15:13:51.098+08:00" (local time
/// with offset) or a trailing-`Z` UTC value.
fn parse_timestamp(ts: &str) -> (String, String) {
    match chrono::DateTime::parse_from_rfc3339(ts) {
        Ok(dt) => {
            let utc = dt.with_timezone(&chrono::Utc);
            (utc.format("%Y-%m-%d").to_string(), utc.to_rfc3339())
        }
        Err(_) => ("unknown".to_string(), "unknown".to_string()),
    }
}

impl DataSource for QoderCliSource {
    fn name(&self) -> &'static str {
        "qoder-cli"
    }

    fn load(&self) -> Vec<TokenRecord> {
        let base = sessions_path("QODER_SESSIONS_PATH", ".qoder");
        let records = load_segments(&base, self.name());
        tracing::info!("Loaded {} qoder-cli records", records.len());
        records
    }

    fn load_incremental(&self) -> Vec<TokenRecord> {
        load_segments_incremental(
            &sessions_path("QODER_SESSIONS_PATH", ".qoder"),
            self.name(),
            |files| self.mark_files_parsed(files),
        )
    }

    fn data_files(&self) -> Vec<PathBuf> {
        jsonl_files(&sessions_path("QODER_SESSIONS_PATH", ".qoder"))
    }

    fn is_available(&self) -> bool {
        sessions_path("QODER_SESSIONS_PATH", ".qoder").exists()
    }
}

impl DataSource for QoderDesktopSource {
    fn name(&self) -> &'static str {
        "qoder-desktop"
    }

    fn load(&self) -> Vec<TokenRecord> {
        let base = sessions_path("QODER_CN_SESSIONS_PATH", ".qoder-cn");
        let records = load_segments(&base, self.name());
        tracing::info!("Loaded {} qoder-desktop records", records.len());
        records
    }

    fn load_incremental(&self) -> Vec<TokenRecord> {
        load_segments_incremental(
            &sessions_path("QODER_CN_SESSIONS_PATH", ".qoder-cn"),
            self.name(),
            |files| self.mark_files_parsed(files),
        )
    }

    fn data_files(&self) -> Vec<PathBuf> {
        jsonl_files(&sessions_path("QODER_CN_SESSIONS_PATH", ".qoder-cn"))
    }

    fn is_available(&self) -> bool {
        sessions_path("QODER_CN_SESSIONS_PATH", ".qoder-cn").exists()
    }
}

fn sessions_path(env_var: &str, home_dir_name: &str) -> PathBuf {
    std::env::var(env_var)
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| super::home_dir().join(home_dir_name).join("logs").join("sessions"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_segment(dir: &Path, content: &str) -> PathBuf {
        let session = dir.join("-proj").join("sess-1").join("segments");
        std::fs::create_dir_all(&session).unwrap();
        let path = session.join("run-1.jsonl");
        std::fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn reads_usage_and_subtracts_cache_read() {
        let dir = std::env::temp_dir().join(format!("qoder-cli-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        write_segment(
            &dir,
            r#"{"ts":"2026-09-18T15:13:59.658+08:00","type":"model.response.completed","request_id":"r1","data":{"model":"qfmodel","input_tokens":23052,"output_tokens":137,"cache_read_input_tokens":22755,"cache_creation_input_tokens":0}}"#,
        );

        let records = load_segments(&dir, "qoder-cli");
        assert_eq!(records.len(), 1);
        let r = &records[0];
        assert_eq!(r.input_tokens, 297);
        assert_eq!(r.output_tokens, 137);
        assert_eq!(r.cache_read_tokens, 22755);
        assert_eq!(r.total_tokens, 297 + 137 + 22755);
        assert_eq!(r.provider, "qoder");
        assert_eq!(r.model, "qfmodel");
        assert_eq!(r.source, "qoder-cli");
        assert_eq!(r.date, "2026-09-18");
        assert_eq!(r.time, "2026-09-18T07:13:59.658+00:00");
    }

    #[test]
    fn keeps_zero_token_calls_and_prefers_reported_provider() {
        let dir = std::env::temp_dir().join(format!("qoder-desktop-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        write_segment(
            &dir,
            &[
                r#"{"ts":"2026-09-18T13:30:52.704+08:00","type":"model.response.completed","request_id":"r1","data":{"provider":"qoder","model":"qfmodel","input_tokens":0,"output_tokens":0,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}"#,
                // Same request_id repeated in another segment file: deduped.
                r#"{"ts":"2026-09-18T13:30:52.704+08:00","type":"model.response.completed","request_id":"r1","data":{"provider":"qoder","model":"qfmodel","input_tokens":0,"output_tokens":0,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}"#,
                r#"{"ts":"2026-09-18T13:31:00.000+08:00","type":"turn.started","data":{"model":"qfmodel"}}"#,
            ]
            .join("\n"),
        );

        let records = load_segments(&dir, "qoder-desktop");
        assert_eq!(records.len(), 1);
        assert!(records[0].is_zero_token());
        assert_eq!(records[0].source, "qoder-desktop");
    }
}
