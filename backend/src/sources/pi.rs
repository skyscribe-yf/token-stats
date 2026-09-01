use super::DataSource;
use crate::models::TokenRecord;
use serde::Deserialize;
use std::collections::HashSet;
use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// How far into `usage.jsonl` we have already parsed.
///
/// `ino` guards against log rotation: pi writes the next generation to a fresh
/// inode, and a byte offset carried over from the previous file would be
/// meaningless (or, worse, silently skip real records).
#[derive(Clone, Copy, Debug)]
struct PiCursor {
    ino: u64,
    offset: u64,
}

static PI_CURSOR: Mutex<Option<PiCursor>> = Mutex::new(None);

/// `(date, provider, model)` triples that `usage.jsonl` already covers,
/// accumulated across every pass.
///
/// `scan_taskplane_runtimes` uses these to suppress exit summaries that would
/// double-count usage already recorded per-call. Coverage must span all passes,
/// not just the latest tail — a triple read on an earlier pass still suppresses
/// summaries seen today.
static PI_COVERED: Mutex<Option<HashSet<(String, String, String)>>> = Mutex::new(None);

/// Pi token log source: reads `~/.pi/token-logs/usage.jsonl`
/// and taskplane lane-worker runtime exit summaries.
#[derive(Default)]
pub struct PiSource;

impl DataSource for PiSource {
    fn name(&self) -> &'static str {
        "pi"
    }

    fn load(&self) -> Vec<TokenRecord> {
        let mut records = Vec::new();

        // 1. Live session records from usage.jsonl (main session + workers
        //    that load pi-token-tracker via pi package mechanism)
        let log_path = Self::log_path();
        tracing::info!("Loading pi data from: {:?}", log_path);
        let (live_records, consumed) = Self::parse_log_from(&log_path, 0);
        tracing::info!("Loaded {} pi live records", live_records.len());

        // Build a coverage set: if usage.jsonl already has per-call records
        // for a given (UTC date, provider, model), exit summaries for
        // matching agents are redundant (would double-count).
        let covered = Self::merge_coverage(&live_records, true);
        // Seed the incremental cursor so the first refresh reads only what pi
        // appends after this point instead of re-reading the whole file.
        Self::set_cursor(&log_path, consumed);

        records.extend(live_records);

        // 2. Taskplane lane-worker runtime records from exit summaries.
        //    Only included for agents NOT already covered by per-call data
        //    (retroactive coverage for batches that ran before the
        //    pi-token-tracker extension was installed as a pi package).
        let runtime_records = Self::scan_taskplane_runtimes(&covered);
        if !runtime_records.is_empty() {
            tracing::info!(
                "Loaded {} pi taskplane runtime records (retroactive)",
                runtime_records.len()
            );
            records.extend(runtime_records);
        }

        records
    }

    /// Incremental: read only the bytes appended to usage.jsonl since the last
    /// pass. Taskplane runtime exit summaries are only scanned when the main
    /// log actually grew (new activity implies new batches may exist).
    fn load_incremental(&self) -> Vec<TokenRecord> {
        let log_path = Self::log_path();

        // `usage.jsonl` is append-only and grows past 100 MB, so the
        // (mtime, size) check every other source uses always reports "changed"
        // and the whole file gets re-read every 30 s. Track the parsed byte
        // offset instead and read only the tail.
        let (len, ino) = match std::fs::metadata(&log_path) {
            Ok(m) => (m.len(), m.ino()),
            Err(_) => return Vec::new(),
        };

        // Fall back to a full read whenever we cannot prove the file was only
        // appended to: first pass, rotation (different inode), or truncation.
        let start_offset = match *Self::cursor() {
            Some(c) if c.ino == ino && len >= c.offset => c.offset,
            _ => 0,
        };

        let (live_records, consumed) = Self::parse_log_from(&log_path, start_offset);

        // Nothing appended and nothing re-read — skip the taskplane scan,
        // matching the old "file unchanged" early-out.
        if live_records.is_empty() && consumed == start_offset {
            return Vec::new();
        }

        Self::set_cursor_at(ino, consumed);
        let covered = Self::merge_coverage(&live_records, start_offset == 0);

        let mut records = live_records;
        records.extend(Self::scan_taskplane_runtimes(&covered));
        records
    }

    fn data_files(&self) -> Vec<std::path::PathBuf> {
        vec![Self::log_path()]
    }

    fn is_available(&self) -> bool {
        Self::log_path().exists()
    }
}

impl PiSource {
    fn log_path() -> PathBuf {
        super::home_dir()
            .join(".pi")
            .join("token-logs")
            .join("usage.jsonl")
    }

    /// Parse complete lines of `path` starting at byte `offset`.
    ///
    /// Only lines terminated by `\n` are consumed: pi appends with a single
    /// `write`, so a pass can observe a half-written trailing line. Parsing a
    /// fragment would emit a bogus record and, worse, advance `offset` past
    /// bytes we never actually read. Such a partial line is left for the next
    /// pass.
    ///
    /// Returns the records parsed and the offset just past the last complete
    /// line read.
    fn parse_log_from(path: &Path, mut offset: u64) -> (Vec<TokenRecord>, u64) {
        let mut file = match File::open(path) {
            Ok(f) => f,
            Err(_) => return (Vec::new(), offset),
        };
        // A seek failure mid-stream means the file changed underneath us;
        // re-read from the start instead of skipping bytes we can't locate.
        if offset > 0 && file.seek(SeekFrom::Start(offset)).is_err() {
            offset = 0;
            let _ = file.seek(SeekFrom::Start(0));
        }

        let mut reader = BufReader::with_capacity(1 << 16, file);
        let mut records = Vec::new();
        let mut consumed = offset;
        let mut buf = Vec::new();

        loop {
            buf.clear();
            let n = match reader.read_until(b'\n', &mut buf) {
                Ok(n) => n,
                Err(e) => {
                    tracing::debug!("pi: read error at offset {}: {e}", consumed);
                    break;
                }
            };
            if n == 0 {
                break;
            }
            if buf.last() != Some(&b'\n') {
                // Partial trailing line (writer still mid-append): don't
                // consume it, don't advance past it.
                break;
            }
            consumed += n as u64;
            let line = match std::str::from_utf8(&buf) {
                Ok(s) => s,
                Err(_) => continue,
            };
            if line.trim().is_empty() {
                continue;
            }
            if let Ok(mut record) = serde_json::from_str::<TokenRecord>(line) {
                if record.source.is_empty() {
                    record.source = "pi".to_string();
                }
                records.push(record);
            }
        }

        (records, consumed)
    }

    /// Merge `records` into the persistent coverage set and return a snapshot
    /// for `scan_taskplane_runtimes`.
    ///
    /// Coverage spans every pass, not just the latest tail, so an exit summary
    /// whose triple was covered by `usage.jsonl` lines read on an earlier pass
    /// stays suppressed. On a full re-read (`reset`) the set is rebuilt, since
    /// the previous triples described a different file.
    fn merge_coverage(
        records: &[TokenRecord],
        reset: bool,
    ) -> HashSet<(String, String, String)> {
        let mut guard = PI_COVERED
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let set = guard.get_or_insert_with(HashSet::new);
        if reset {
            set.clear();
        }
        for r in records {
            set.insert((r.date.clone(), r.provider.clone(), r.model.clone()));
        }
        // ~800 distinct triples; cloning is cheaper than holding the lock
        // across the taskplane scan's file I/O.
        set.clone()
    }

    fn cursor() -> std::sync::MutexGuard<'static, Option<PiCursor>> {
        PI_CURSOR.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn set_cursor(path: &Path, offset: u64) {
        let ino = std::fs::metadata(path).map(|m| m.ino()).unwrap_or(0);
        let mut guard = Self::cursor();
        *guard = Some(PiCursor { ino, offset });
    }

    fn set_cursor_at(ino: u64, offset: u64) {
        let mut guard = Self::cursor();
        *guard = Some(PiCursor { ino, offset });
    }

    // ── Taskplane Runtime Scanner ──────────────────────────────────────────
    //
    // Taskplane lane workers run in separate pi --mode rpc processes with
    // --no-extensions, so the token-tracker extension is NOT loaded. Their
    // token usage is recorded in exit summaries at:
    //   <project>/.pi/runtime/<batchId>/agents/<agentId>/events-exit.json
    //
    // Scans ~/srcs/*/.pi/runtime/ for these files and creates TokenRecords.

    fn scan_taskplane_runtimes(covered: &HashSet<(String, String, String)>) -> Vec<TokenRecord> {
        let projects_dir = std::env::var("TASKPLANE_PROJECTS_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| super::home_dir().join("srcs"));

        let project_dirs = match std::fs::read_dir(&projects_dir) {
            Ok(entries) => entries
                .filter_map(|e| e.ok())
                .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
                .map(|e| e.path())
                .collect::<Vec<_>>(),
            Err(_) => return Vec::new(),
        };

        let mut records = Vec::new();

        for project_dir in &project_dirs {
            let runtime_root = project_dir.join(".pi").join("runtime");
            if !runtime_root.exists() {
                continue;
            }
            let batch_records = Self::scan_batches(&runtime_root, covered);
            records.extend(batch_records);
        }

        records
    }

    fn scan_batches(
        runtime_root: &Path,
        covered: &HashSet<(String, String, String)>,
    ) -> Vec<TokenRecord> {
        let batch_dirs = match std::fs::read_dir(runtime_root) {
            Ok(entries) => entries
                .filter_map(|e| e.ok())
                .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
                .map(|e| e.path())
                .collect::<Vec<_>>(),
            Err(_) => return Vec::new(),
        };

        let mut records = Vec::new();

        for batch_path in &batch_dirs {
            let batch_name = match batch_path.file_name().and_then(|n| n.to_str()) {
                Some(name) => name.to_string(),
                None => continue,
            };

            let (batch_date, batch_time) = parse_batch_timestamp(&batch_name);

            let agents_dir = batch_path.join("agents");
            if !agents_dir.exists() {
                continue;
            }

            let agent_dirs = match std::fs::read_dir(&agents_dir) {
                Ok(entries) => entries
                    .filter_map(|e| e.ok())
                    .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
                    .map(|e| e.path())
                    .collect::<Vec<_>>(),
                Err(_) => continue,
            };

            for agent_path in &agent_dirs {
                let _agent_name = match agent_path.file_name().and_then(|n| n.to_str()) {
                    Some(name) => name.to_string(),
                    None => continue,
                };

                let exit_paths = [
                    agent_path.join("events-exit.json"),
                    agent_path.join("exit-summary.json"),
                ];

                let exit_data: ExitData = match exit_paths.iter().find_map(|p| {
                    if !p.exists() {
                        return None;
                    }
                    serde_json::from_reader(match File::open(p) {
                        Ok(f) => f,
                        Err(_) => return None,
                    })
                    .ok()
                }) {
                    Some(d) => d,
                    None => continue,
                };

                let tokens = match exit_data.tokens {
                    Some(t) => t,
                    None => continue,
                };

                let (provider, model) = read_agent_provider_model(agent_path);

                // Skip if usage.jsonl already has per-call records for this
                // (UTC date, provider, model). The batch_time is the UTC-
                // converted timestamp; its date portion matches usage.jsonl
                // records which also use UTC dates.
                let utc_date = if batch_time.len() >= 10 {
                    &batch_time[..10]
                } else {
                    &batch_date
                };
                let cover_key = (utc_date.to_string(), provider.clone(), model.clone());
                if covered.contains(&cover_key) {
                    continue;
                }

                let total_tokens = tokens.input
                    + tokens.output
                    + tokens.cache_read.unwrap_or(0)
                    + tokens.cache_write.unwrap_or(0);

                records.push(TokenRecord {
                    date: batch_date.clone(),
                    time: batch_time.clone(),
                    api_key_prefix: format!("runtime:{}", batch_name),
                    provider,
                    original_provider: None,
                    model,
                    source: "pi".to_string(),
                    input_tokens: tokens.input,
                    output_tokens: tokens.output,
                    cache_read_tokens: tokens.cache_read.unwrap_or(0),
                    cache_write_tokens: tokens.cache_write.unwrap_or(0),
                    total_tokens,
                    cost: exit_data.cost.unwrap_or(0.0),
                    ttft_ms: None,
                    tps: None,
                });
            }
        }

        records
    }
}

// ── Helper data types ────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct ExitTokens {
    input: i64,
    output: i64,
    #[serde(rename = "cacheRead")]
    cache_read: Option<i64>,
    #[serde(rename = "cacheWrite")]
    cache_write: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct ExitData {
    tokens: Option<ExitTokens>,
    cost: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct AgentStartedPayload {
    model: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AgentEvent {
    #[serde(rename = "type")]
    event_type: Option<String>,
    payload: Option<AgentStartedPayload>,
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Parse a batch directory name like "20260518T213033" into a date and time string.
/// Returns ("2026-05-18", "2026-05-18T13:30:33Z") assuming the timestamp is in
/// local time (Asia/Shanghai, UTC+8). Falls back to defaults on parse failure.
fn parse_batch_timestamp(batch_name: &str) -> (String, String) {
    // Expect format: YYYYMMDDTHHMMSS (e.g. "20260518T213033")
    if batch_name.len() < 15 {
        return ("unknown".to_string(), "unknown".to_string());
    }

    let year = &batch_name[0..4];
    let month = &batch_name[4..6];
    let day = &batch_name[6..8];
    let hour = &batch_name[9..11];
    let min = &batch_name[11..13];
    let sec = &batch_name[13..15];

    let date = format!("{}-{}-{}", year, month, day);

    // Assume Asia/Shanghai (UTC+8) for the batch timestamp
    // Convert to UTC by subtracting 8 hours
    let local_h: i32 = hour.parse().unwrap_or(0);
    let utc_h = (local_h - 8 + 24) % 24;
    let utc_date_adjust = if local_h < 8 { -1 } else { 0 };

    let time = if utc_date_adjust != 0 {
        // Previous day in UTC
        let prev_day: i32 = day.parse().unwrap_or(1);
        let utc_day = (prev_day + utc_date_adjust).max(1);
        format!(
            "{}-{}-{:02}T{:02}:{}:{}Z",
            year, month, utc_day, utc_h, min, sec
        )
    } else {
        format!("{}-{}-{}T{:02}:{}:{}Z", year, month, day, utc_h, min, sec)
    };

    (date, time)
}

/// Read provider and model from the first line of events.jsonl in an agent
/// directory. The first event is typically:
///   {"type":"agent_started","payload":{"model":"kimi-for-coding"}}
/// or:
///   {"type":"agent_started","payload":{"model":"xunfei/astron-code-latest"}}
///
/// If the model contains "/", it's split as "provider/model".
/// Falls back to ("taskplane-worker", "unknown").
fn read_agent_provider_model(agent_path: &Path) -> (String, String) {
    let events_path = agent_path.join("events.jsonl");
    if !events_path.exists() {
        return ("taskplane-worker".to_string(), "unknown".to_string());
    }

    let content = match std::fs::read_to_string(&events_path) {
        Ok(c) => c,
        Err(_) => return ("taskplane-worker".to_string(), "unknown".to_string()),
    };

    let first_line = match content.lines().find(|l| !l.trim().is_empty()) {
        Some(l) => l,
        None => return ("taskplane-worker".to_string(), "unknown".to_string()),
    };

    let event: AgentEvent = match serde_json::from_str(first_line) {
        Ok(e) => e,
        Err(_) => return ("taskplane-worker".to_string(), "unknown".to_string()),
    };

    // Only process agent_started events
    if event.event_type.as_deref() != Some("agent_started") {
        return ("taskplane-worker".to_string(), "unknown".to_string());
    }

    let model_ref = match event.payload.and_then(|p| p.model) {
        Some(m) => m,
        None => return ("taskplane-worker".to_string(), "unknown".to_string()),
    };

    // If model contains "/", split into provider/model
    if let Some(slash_pos) = model_ref.find('/') {
        let provider = model_ref[..slash_pos].to_string();
        let model = model_ref[slash_pos + 1..].to_string();
        (provider, model)
    } else {
        // Fallback: infer provider from model name (e.g. kimi-for-coding → kimi)
        let provider = super::resolve_provider_from_model(&model_ref);
        (provider, model_ref)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    // `PiSource` state lives in process-global statics; serialize the tests
    // that touch it (and reset between scenarios) so they can't clobber each
    // other when Cargo runs tests in parallel.
    static PI_TEST_LOCK: Mutex<()> = Mutex::new(());

    fn reset_state() {
        *PI_CURSOR.lock().unwrap() = None;
        *PI_COVERED.lock().unwrap() = None;
    }

    fn in_temp_home<F: FnOnce()>(home: &Path, f: F) {
        let _lk = PI_TEST_LOCK.lock().unwrap();
        reset_state();
        // SAFETY: tests are serialized on PI_TEST_LOCK, so no other thread
        // reads HOME while we mutate it.
        unsafe { std::env::set_var("HOME", home.to_str().unwrap()) };
        f();
    }

    fn write_log(home: &Path, content: &str) {
        let p = home.join(".pi").join("token-logs").join("usage.jsonl");
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        // Real usage.jsonl is always newline-terminated per line; ensure the
        // content ends with one so a single trailing record parses as complete.
        let mut buf = content.to_string();
        if !buf.ends_with('\n') {
            buf.push('\n');
        }
        std::fs::write(&p, buf).unwrap();
    }

    fn log_path(home: &Path) -> PathBuf {
        home.join(".pi").join("token-logs").join("usage.jsonl")
    }

    fn rec(time: &str) -> String {
        format!(
            r#"{{"date":"2026-08-30","time":"{time}","apiKeyPrefix":"N/A","provider":"deepseek","model":"deepseek-v4-pro","source":"pi","inputTokens":10,"outputTokens":5,"cacheReadTokens":0,"cacheWriteTokens":0,"totalTokens":15,"cost":0.0,"ttftMs":null,"tps":null}}"#
        )
    }

    #[test]
    fn incremental_reads_only_appended_lines() {
        let home = tempfile::tempdir().unwrap();
        in_temp_home(home.path(), || {
            write_log(
                home.path(),
                &format!("{}\n{}\n{}\n", rec("2026-08-30T10:00:00Z"), rec("2026-08-30T10:01:00Z"), rec("2026-08-30T10:02:00Z")),
            );
            assert_eq!(PiSource.load_incremental().len(), 3, "first pass reads all 3");

            let p = log_path(home.path());
            let mut f = std::fs::OpenOptions::new().append(true).open(&p).unwrap();
            write!(f, "{}\n{}\n", rec("2026-08-30T10:03:00Z"), rec("2026-08-30T10:04:00Z")).unwrap();

            assert_eq!(PiSource.load_incremental().len(), 2, "second pass reads only the 2 appended lines");
        });
    }

    #[test]
    fn partial_trailing_line_is_not_consumed() {
        let home = tempfile::tempdir().unwrap();
        in_temp_home(home.path(), || {
            // A properly newline-terminated first line.
            write_log(home.path(), &format!("{}\n", rec("2026-08-30T10:00:00Z")));
            let p = log_path(home.path());
            // Append a partial, non-newline-terminated line.
            let mut f = std::fs::OpenOptions::new().append(true).open(&p).unwrap();
            let full = rec("2026-08-30T10:01:00Z");
            write!(f, "{}", &full[..50]).unwrap();

            assert_eq!(PiSource.load_incremental().len(), 1, "only the complete line is parsed");

            // Complete that same line.
            let mut f = std::fs::OpenOptions::new().append(true).open(&p).unwrap();
            write!(f, "{}\n", &full[50..]).unwrap();
            assert_eq!(PiSource.load_incremental().len(), 1, "completed line parsed exactly once");
        });
    }

    #[test]
    fn truncation_triggers_full_reread() {
        let home = tempfile::tempdir().unwrap();
        in_temp_home(home.path(), || {
            write_log(
                home.path(),
                &format!("{}\n{}\n", rec("2026-08-30T10:00:00Z"), rec("2026-08-30T10:01:00Z")),
            );
            assert_eq!(PiSource.load_incremental().len(), 2);
            // Truncate to a single line.
            write_log(home.path(), &rec("2026-08-30T10:00:00Z"));
            assert_eq!(PiSource.load_incremental().len(), 1, "re-reads the single remaining line");
        });
    }

    #[test]
    fn taskplane_summary_suppressed_when_covered_by_usage_jsonl() {
        let home = tempfile::tempdir().unwrap();
        in_temp_home(home.path(), || {
            // usage.jsonl covers (date, deepseek, deepseek-v4-pro); this seeds
            // the persistent coverage set.
            write_log(home.path(), &rec("2026-08-30T10:00:00Z"));

            // A taskplane exit summary with the same triple as the per-call
            // record must be suppressed on the taskplane scan.
            let rt = home
                .path()
                .join("srcs")
                .join("proj")
                .join(".pi")
                .join("runtime")
                .join("20260830T100000")
                .join("agents")
                .join("agent1");
            std::fs::create_dir_all(&rt).unwrap();
            std::fs::write(
                rt.join("exit-summary.json"),
                r#"{"tokens":{"input":100,"output":5,"cacheRead":0,"cacheWrite":0}}"#,
            )
            .unwrap();
            std::fs::write(
                rt.join("events.jsonl"),
                r#"{"type":"agent_started","payload":{"model":"deepseek/deepseek-v4-pro"}}"#,
            )
            .unwrap();

            // First pass: full read of usage.jsonl + taskplane scan.
            let got = PiSource.load_incremental();
            assert_eq!(got.len(), 1, "usage line kept, covered summary suppressed");

            // Grow usage.jsonl so the next pass re-scans taskplane (mirrors a
            // new per-call record arriving — the only trigger for a re-scan,
            // exactly as the original code behaved).
            let p = log_path(home.path());
            let mut f = std::fs::OpenOptions::new().append(true).open(&p).unwrap();
            write!(f, "{}\n", rec("2026-08-30T10:05:00Z")).unwrap();

            // An exit summary with an UNCOVERED triple is picked up on re-scan.
            let rt2 = rt.parent().unwrap().join("agent2");
            std::fs::create_dir_all(&rt2).unwrap();
            std::fs::write(
                rt2.join("exit-summary.json"),
                r#"{"tokens":{"input":50,"output":3,"cacheRead":0,"cacheWrite":0}}"#,
            )
            .unwrap();
            std::fs::write(
                rt2.join("events.jsonl"),
                r#"{"type":"agent_started","payload":{"model":"kimi/kimi-k2.7"}}"#,
            )
            .unwrap();

            let got = PiSource.load_incremental();
            assert_eq!(got.len(), 2, "uncovered exit summary read on re-scan");
            assert!(
                got.iter().any(|r| r.model == "kimi-k2.7"),
                "uncovered agent2 picked up"
            );
        });
    }
}
