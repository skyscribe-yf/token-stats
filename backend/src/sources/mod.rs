//! Data source abstractions for loading token usage records.
//!
//! Each data source (Pi, Codex, Claude Code, Kimi CLI, OpenCode, ccswitch)
//! implements the `DataSource` trait. The `load_all_sources()` function
//! orchestrates loading all configured sources and applies vendor merging.

mod ccswitch;
mod claude_code;
mod codex;
mod commandcode;
mod dim;
mod dsh;
mod grok_cli;
mod kimi_cli;
mod kimi_code;
mod opencode;
mod pi;
mod qoder;
mod qoder_cn;
mod zcode;

use crate::config;
use crate::models::TokenRecord;
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

pub use ccswitch::CcSwitchSource;
pub use claude_code::ClaudeCodeSource;
pub use codex::CodexSource;
pub use commandcode::CommandCodeSource;
pub use dim::DimSource;
pub use dsh::DshSource;
pub(crate) use grok_cli::grok_usage_log_path;
pub use grok_cli::GrokCliSource;
pub use kimi_cli::KimiCliSource;
pub use kimi_code::KimiCodeSource;
pub use opencode::OpenCodeSource;
pub use pi::PiSource;
pub use qoder::QoderSource;
pub use qoder_cn::QoderCnSource;
pub use zcode::ZcodeSource;

/// Trait for a data source that produces `TokenRecord` batches.
pub trait DataSource: Send + Sync {
    /// Human-readable name for logging.
    fn name(&self) -> &'static str;

    /// Load all records from this source.
    fn load(&self) -> Vec<TokenRecord>;

    /// Load only the records from files that changed since the last call.
    ///
    /// Default implementation re-parses everything (same as `load`), so
    /// sources opt into incremental parsing by implementing
    /// [`Self::data_files`] and using [`Self::changed_data_files`].
    fn load_incremental(&self) -> Vec<TokenRecord> {
        self.load()
    }

    /// The files this source reads. Used by the incremental machinery to
    /// decide which files changed; sources that don't report files are
    /// always fully re-parsed.
    fn data_files(&self) -> Vec<std::path::PathBuf> {
        Vec::new()
    }

    /// Of [`Self::data_files`], the ones whose (mtime, size) changed since
    /// the last successful parse. After parsing, call
    /// [`Self::mark_files_parsed`] so the next refresh skips them.
    fn changed_data_files(&self) -> Vec<std::path::PathBuf> {
        changed_files(&self.data_files())
    }

    /// Record that `paths` have been parsed at their current (mtime, size).
    fn mark_files_parsed(&self, paths: &[std::path::PathBuf]) {
        for p in paths {
            if let Some(stamp) = stamp_of(p) {
                remember_parsed(p, stamp);
            }
        }
    }

    /// Whether this source's data path exists and should be loaded.
    /// Sources returning false are skipped entirely during refresh,
    /// avoiding unnecessary I/O and log spam.
    fn is_available(&self) -> bool {
        true
    }
}

// ─── Shared utilities ────────────────────────────────────────────────────────

/// Simple recursive directory walker to find all files.
pub(crate) fn walkdir(path: &Path) -> Result<Vec<std::path::PathBuf>, std::io::Error> {
    let mut result = Vec::new();
    walkdir_recursive(path, &mut result)?;
    Ok(result)
}

fn walkdir_recursive(
    path: &Path,
    result: &mut Vec<std::path::PathBuf>,
) -> Result<(), std::io::Error> {
    if path.is_dir() {
        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            let p = entry.path();
            if p.is_dir() {
                walkdir_recursive(&p, result)?;
            } else {
                result.push(p);
            }
        }
    }
    Ok(())
}

/// Home directory path.
pub(crate) fn home_dir() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    std::path::PathBuf::from(home)
}

/// Parse an ISO-8601 / RFC3339 timestamp into (date, time) strings.
pub(crate) fn parse_iso_timestamp(ts: &str) -> (String, String) {
    match DateTime::parse_from_rfc3339(ts) {
        Ok(dt) => {
            let utc = dt.with_timezone(&Utc);
            (utc.format("%Y-%m-%d").to_string(), utc.to_rfc3339())
        }
        Err(_) => ("unknown".to_string(), "unknown".to_string()),
    }
}

/// Resolve a human-readable provider name from a model name.
/// Used as a fallback when the provider field is missing or generic.
pub(crate) fn resolve_provider_from_model(model: &str) -> String {
    match model {
        "kimi-for-coding" | "kimi-k2" | "kimi-k2.6" | "kimi-k2.5" | "kimi-k2.7" => {
            "kimi".to_string()
        }
        "astron-code-latest" => "xunfei".to_string(),
        "mimo-v2.5-pro" | "mimo-v2-pro" | "mimo-v2.5" => "xiaomi-mimo".to_string(),
        "deepseek-v4-pro" | "deepseek-v4-flash" | "deepseek-v3.2" => "deepseek".to_string(),
        "gpt-5.5" | "gpt-5.4" | "gpt-5.4-mini" => "openai".to_string(),
        "glm-5" | "glm-5.1" | "glm-4.7-flash" => "opencode-go".to_string(),
        "sonnet" | "haiku" => "anthropic".to_string(),
        "qmodel_latest" | "efficient" | "auto" => "qoder".to_string(),
        "spark-x2" | "spark-x2-flash" => "xunfei".to_string(),
        "qwen3.6-35b" | "qwen3.5-35b" | "qwen3.5-397b" | "qwen3-coder-next" => "qwen".to_string(),
        "minimax-m2.5" => "minimax".to_string(),
        "LongCat-2.0" => "meituan".to_string(),
        _ if model.starts_with("claude-") => "anthropic".to_string(),
        _ => model.to_string(),
    }
}

// ─── Model name normalization ────────────────────────────────────────────────

/// Normalize model names across sources so the same model appears under one name.
///
/// Pi uses `claude-opus-4.7` (dot) while Claude Code uses `claude-opus-4-7` (hyphen).
/// Known thinking / reasoning level suffixes applied by providers (ainaiba, kimi, etc.).
/// These indicate the reasoning effort level and should not create separate model buckets.
const THINKING_LEVEL_SUFFIXES: &[&str] = &[":xhigh", ":high", ":low", ":medium"];

/// Strip a known thinking-level suffix from the model name, if present.
fn strip_thinking_level(model: &str) -> &str {
    for suffix in THINKING_LEVEL_SUFFIXES {
        if let Some(base) = model.strip_suffix(suffix) {
            return base;
        }
    }
    model
}

pub fn normalize_model_name(model: &str) -> String {
    // Strip thinking-level suffixes first so later normalizations work on the base name.
    let model = strip_thinking_level(model);

    // Normalize claude-opus-4.7 -> claude-opus-4-7
    if let Some(rest) = model.strip_prefix("claude-opus-") {
        return format!("claude-opus-{}", rest.replace('.', "-"));
    }
    // Xunfei MaaS model ID → human-readable name
    match model {
        // Legacy channel name → default model
        "astron-code-latest" => "glm-5.1".to_string(),
        // GLM family
        "xopglm5" => "glm-5".to_string(),
        "xopglm51" => "glm-5.1".to_string(),
        "xopglmv47flash" => "glm-4.7-flash".to_string(),
        // Kimi family — also normalizes short alias "k3" to full "kimi-k3"
        "k3" => "kimi-k3".to_string(),
        "xopkimik26" => "kimi-k2.6".to_string(),
        "xopkimik25" => "kimi-k2.5".to_string(),
        // DeepSeek family
        "xopdeepseekv4pro" => "deepseek-v4-pro".to_string(),
        "xopdeepseekv4flash" => "deepseek-v4-flash".to_string(),
        "xopdeepseekv32" => "deepseek-v3.2".to_string(),
        // Spark family
        "xsparkx2" => "spark-x2".to_string(),
        "xsparkx2flash" => "spark-x2-flash".to_string(),
        // Qwen family
        "xopqwen36v35b" => "qwen3.6-35b".to_string(),
        "xopqwen35v35b" => "qwen3.5-35b".to_string(),
        "xopqwen35397b" => "qwen3.5-397b".to_string(),
        "xop3qwencodernext" => "qwen3-coder-next".to_string(),
        // MiniMax family
        "xminimaxm25" => "minimax-m2.5".to_string(),
        // Grok: -build is a preview variant of the same base model
        "grok-4.5-build" => "grok-4.5".to_string(),
        _ => model.to_string(),
    }
}

// ─── Load all sources ────────────────────────────────────────────────────────

/// Cache which sources are available (path exists) so we don't re-stat
/// missing directories every 30s, and don't log "not found" warnings
/// on every refresh cycle. Re-checked on every call but only logged once.
static UNAVAILABLE_SOURCES: OnceLock<Vec<&'static str>> = OnceLock::new();

/// A lightweight fingerprint of a data file's contents.
///
/// Comparing `(modified, len)` catches both brand-new files and appended
/// lines (codex/kimi/claude append to the same rollout file as a session
/// progresses). It does not catch in-place rewrites of identical length,
/// which these append-only logs never do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct FileStamp {
    modified: std::time::SystemTime,
    len: u64,
}

/// Remembers which data files have already been parsed so refreshes only
/// re-read files that changed. Keyed by canonical path.
///
/// Stored on the heap via `OnceLock<Box<...>>` because `Mutex::new` is not
/// const-constructible in older Rust; the box is created once on first use.
static FILE_STAMPS: OnceLock<Box<Mutex<HashMap<std::path::PathBuf, FileStamp>>>> =
    OnceLock::new();

fn file_stamps() -> &'static Mutex<HashMap<std::path::PathBuf, FileStamp>> {
    FILE_STAMPS.get_or_init(|| Box::new(Mutex::new(HashMap::new())))
}

/// Return the current stamp of `path` (missing files are treated as
/// "no stamp" so a previously-seen file that vanishes is re-parsed if it
/// reappears with a different identity).
pub(crate) fn stamp_of(path: &Path) -> Option<FileStamp> {
    let meta = std::fs::metadata(path).ok()?;
    let modified = meta.modified().ok()?;
    Some(FileStamp {
        modified,
        len: meta.len(),
    })
}

/// Record that `path` has been fully parsed at the given stamp.
pub(crate) fn remember_parsed(path: &Path, stamp: FileStamp) {
    if let Ok(mut map) = file_stamps().lock() {
        map.insert(path.to_path_buf(), stamp);
    }
}

/// Filter `paths` down to those whose (mtime, size) differs from the last
/// time they were parsed. Paths never seen before are always included.
pub(crate) fn changed_files(paths: &[std::path::PathBuf]) -> Vec<std::path::PathBuf> {
    let map = match file_stamps().lock() {
        Ok(m) => m,
        Err(_) => return paths.to_vec(),
    };
    paths
        .iter()
        .filter(|p| {
            stamp_of(p).map_or(true, |stamp| map.get(*p) != Some(&stamp))
        })
        .cloned()
        .collect()
}

/// Test-only: mark `paths` as parsed at their current (mtime, size).
#[cfg(test)]
pub(crate) fn mark_files_parsed_for_test(paths: &[std::path::PathBuf]) {
    for p in paths {
        if let Some(stamp) = stamp_of(p) {
            remember_parsed(p, stamp);
        }
    }
}

/// Like `load_all_sources`, but only re-parses files whose (mtime, size)
/// changed since the previous call. The first call behaves exactly like
/// `load_all_sources` (nothing is known yet).
///
/// `post_process` receives every record loaded on *this* call (both changed
/// and untouched sources) so normalizations that span sources (vendor merge,
/// deepseek-ai dedup) still see the full current data. Sources whose files
/// are all unchanged return zero records but still get post-processed —
/// cheap because it only touches the (small) set of new records.
pub fn load_changed_sources() -> Vec<TokenRecord> {
    load_sources_impl(true)
}

/// Full reload — parse every available source file, ignoring the cache.
pub fn load_all_sources() -> Vec<TokenRecord> {
    load_sources_impl(false)
}

fn load_sources_impl(incremental: bool) -> Vec<TokenRecord> {
    let mut all_records = Vec::new();

    let sources: Vec<Box<dyn DataSource>> = {
        let mut v: Vec<Box<dyn DataSource>> = vec![
            Box::new(PiSource),
            Box::new(CodexSource),
            Box::new(ClaudeCodeSource),
            Box::new(OpenCodeSource),
            Box::new(KimiCliSource),
            Box::new(KimiCodeSource),
            Box::new(QoderSource),
            Box::new(QoderCnSource),
            Box::new(GrokCliSource),
            Box::new(CommandCodeSource),
            Box::new(ZcodeSource),
            Box::new(DshSource),
            Box::new(DimSource),
        ];
        if std::env::var("USE_CC_SWITCH").is_ok() {
            v.push(Box::new(CcSwitchSource));
        }
        v
    };

    // Determine which sources are unavailable (only log warnings once)
    let unavailable: Vec<&'static str> = sources
        .iter()
        .filter(|src| !src.is_available())
        .map(|src| src.name())
        .collect();

    // Only log "not found" warnings once, not every 30s
    let already_warned = UNAVAILABLE_SOURCES.get().map_or(false, |prev| {
        prev.len() == unavailable.len() && prev.iter().zip(unavailable.iter()).all(|(a, b)| a == b)
    });
    if !already_warned {
        for name in &unavailable {
            tracing::warn!("Source '{}' data path not found, skipping", name);
        }
        let _ = UNAVAILABLE_SOURCES.set(unavailable.clone());
    }

    for src in &sources {
        if !src.is_available() {
            continue;
        }
        let records = if incremental {
            src.load_incremental()
        } else {
            src.load()
        };
        if !records.is_empty() {
            tracing::info!("Loaded {} records from {}", records.len(), src.name());
        }
        all_records.extend(records);
    }

    tracing::info!("Total records loaded this pass: {}", all_records.len());

    // ── Cross-source dedup: deepseek-ai vs opencode ────────────────────
    // deepseek-ai records imported from DeepSeek official platform exports
    // (daily aggregates) may duplicate individual records from the OpenCode
    // SQLite DB (source=opencode). Remove deepseek-ai records whose
    // (date, provider, model, total_tokens) closely matches an opencode
    // record (within 5% token count tolerance).
    let opencode_totals: std::collections::HashMap<_, i64> = all_records
        .iter()
        .filter(|r| r.source == "opencode")
        .fold(std::collections::HashMap::new(), |mut map, r| {
            let key = (r.date.clone(), r.provider.clone(), r.model.clone());
            *map.entry(key).or_insert(0) += r.total_tokens;
            map
        });

    all_records.retain(|r| {
        if r.source != "deepseek-ai" {
            return true;
        }
        let key = (r.date.clone(), r.provider.clone(), r.model.clone());
        match opencode_totals.get(&key) {
            Some(&oc_total) if oc_total > 0 => {
                let diff_pct = (r.total_tokens - oc_total).unsigned_abs() as f64 / oc_total as f64 * 100.0;
                if diff_pct < 5.0 {
                    tracing::debug!(
                        "Removing duplicate deepseek-ai record: {} {} {} ({} tokens vs opencode {})",
                        r.date, r.provider, r.model, r.total_tokens, oc_total
                    );
                    false
                } else {
                    true
                }
            }
            _ => true,
        }
    });

    // Normalize model names across sources (e.g. claude-opus-4.7 -> claude-opus-4-7)
    for record in all_records.iter_mut() {
        record.model = normalize_model_name(&record.model);
    }

    // ── Command Code normalization ─────────────────────────────────────
    // Command Code API via pi uses OpenAI convention: input_tokens includes
    // cache_read_tokens. Subtract to normalize (matching Codex / native cmd).
    // Native `cmd` logs (source="commandcode") are subtracted in the parser.
    for record in all_records.iter_mut() {
        if record.provider == "commandcode" && record.source == "pi" {
            let effective_input = (record.input_tokens - record.cache_read_tokens).max(0);
            record.input_tokens = effective_input;
            record.total_tokens = effective_input
                + record.output_tokens
                + record.cache_read_tokens
                + record.cache_write_tokens;
        }
    }

    // Apply vendor merging from config
    let merge_config_path = config::get_vendor_merge_config_path();
    if let Some(merge_map) = config::load_vendor_merge_map(&merge_config_path) {
        config::apply_vendor_merge(&mut all_records, &merge_map);
    }

    // ── Kimi model upgrade: kimi-for-coding → kimi-k2.7 ──────────────────────
    // On 2026-06-12 18:00 Beijing time (UTC+8), Kimi released kimi-k2.7 as the
    // successor to kimi-for-coding. The kimi-code tool's backend was updated
    // automatically, but the pi tool and kimi-cli still report the old model
    // name "kimi-for-coding" in their logs. We normalize post-cutover records
    // so that all tools show the correct model name consistently.
    //
    // IMPORTANT: This must run AFTER vendor merge, because pi records arrive
    // with provider="kimi-coding" which gets merged to "kimi" by the step
    // above. Without the merge, the provider check would miss these records.
    //
    // Only records with provider="kimi" are affected; records from other
    // vendors (e.g. opencode-go routing kimi models) are left untouched.
    let kimi_k27_cutoff = chrono::DateTime::parse_from_rfc3339("2026-06-12T10:00:00Z")
        .expect("hardcoded cutoff is valid RFC3339")
        .with_timezone(&chrono::Utc);
    let mut kimi_renamed = 0usize;
    for record in all_records.iter_mut() {
        if record.provider == "kimi" && record.model == "kimi-for-coding" {
            if let Ok(record_time) = chrono::DateTime::parse_from_rfc3339(&record.time) {
                if record_time.with_timezone(&chrono::Utc) >= kimi_k27_cutoff {
                    record.model = "kimi-k2.7".to_string();
                    kimi_renamed += 1;
                }
            }
        }
    }
    if kimi_renamed > 0 {
        tracing::info!(
            "Kimi model upgrade: renamed {} kimi-for-coding record(s) to kimi-k2.7 (cutoff: 2026-06-12 18:00 UTC+8)",
            kimi_renamed
        );
    }

    all_records
}
mod tests {
    #[allow(unused_imports)]
    use super::*;

    #[test]
    fn resolve_claude_model_to_anthropic() {
        assert_eq!(resolve_provider_from_model("claude-opus-4-7"), "anthropic");
        assert_eq!(
            resolve_provider_from_model("claude-sonnet-4-6"),
            "anthropic"
        );
        assert_eq!(resolve_provider_from_model("claude-haiku-4-5"), "anthropic");
        assert_eq!(
            resolve_provider_from_model("claude-3-5-sonnet-20241022"),
            "anthropic"
        );
    }

    #[test]
    fn resolve_shorthand_claude_models() {
        assert_eq!(resolve_provider_from_model("sonnet"), "anthropic");
        assert_eq!(resolve_provider_from_model("haiku"), "anthropic");
    }

    #[test]
    fn resolve_existing_providers_unchanged() {
        assert_eq!(resolve_provider_from_model("kimi-for-coding"), "kimi");
        assert_eq!(resolve_provider_from_model("astron-code-latest"), "xunfei");
        assert_eq!(resolve_provider_from_model("mimo-v2.5-pro"), "xiaomi-mimo");
        assert_eq!(resolve_provider_from_model("deepseek-v4-pro"), "deepseek");
        assert_eq!(resolve_provider_from_model("gpt-5.5"), "openai");
        assert_eq!(resolve_provider_from_model("glm-5.1"), "opencode-go");
    }

    #[test]
    fn resolve_normalized_xunfei_models() {
        assert_eq!(resolve_provider_from_model("glm-5"), "opencode-go");
        assert_eq!(resolve_provider_from_model("glm-5.1"), "opencode-go");
        assert_eq!(resolve_provider_from_model("glm-4.7-flash"), "opencode-go");
        assert_eq!(resolve_provider_from_model("spark-x2"), "xunfei");
        assert_eq!(resolve_provider_from_model("spark-x2-flash"), "xunfei");
        assert_eq!(resolve_provider_from_model("deepseek-v3.2"), "deepseek");
        assert_eq!(resolve_provider_from_model("qwen3.6-35b"), "qwen");
        assert_eq!(resolve_provider_from_model("qwen3.5-35b"), "qwen");
        assert_eq!(resolve_provider_from_model("minimax-m2.5"), "minimax");
    }

    #[test]
    fn resolve_unknown_model_returns_model_name() {
        assert_eq!(
            resolve_provider_from_model("some-unknown-model"),
            "some-unknown-model"
        );
    }

    #[test]
    fn normalize_model_name_converts_dot_to_hyphen() {
        // claude-opus-4.7 (from Pi) should normalize to claude-opus-4-7 (from Claude Code)
        assert_eq!(normalize_model_name("claude-opus-4.7"), "claude-opus-4-7");
    }

    #[test]
    fn normalize_model_name_preserves_others() {
        assert_eq!(normalize_model_name("gpt-5.5"), "gpt-5.5");
        assert_eq!(normalize_model_name("deepseek-v4-pro"), "deepseek-v4-pro");
        assert_eq!(normalize_model_name("kimi-for-coding"), "kimi-for-coding");
    }

    #[test]
    fn normalize_model_name_merges_grok_build_variant() {
        assert_eq!(normalize_model_name("grok-4.5-build"), "grok-4.5");
        // Base model is unchanged
        assert_eq!(normalize_model_name("grok-4.5"), "grok-4.5");
    }

    #[test]
    fn normalize_xunfei_model_ids() {
        assert_eq!(normalize_model_name("astron-code-latest"), "glm-5.1");
        assert_eq!(normalize_model_name("xopglm5"), "glm-5");
        assert_eq!(normalize_model_name("xopglm51"), "glm-5.1");
        assert_eq!(normalize_model_name("xopkimik26"), "kimi-k2.6");
        assert_eq!(normalize_model_name("xopkimik25"), "kimi-k2.5");
        assert_eq!(normalize_model_name("xopdeepseekv4pro"), "deepseek-v4-pro");
        assert_eq!(
            normalize_model_name("xopdeepseekv4flash"),
            "deepseek-v4-flash"
        );
        assert_eq!(normalize_model_name("xopdeepseekv32"), "deepseek-v3.2");
        assert_eq!(normalize_model_name("xsparkx2"), "spark-x2");
        assert_eq!(normalize_model_name("xsparkx2flash"), "spark-x2-flash");
        assert_eq!(normalize_model_name("xopqwen36v35b"), "qwen3.6-35b");
        assert_eq!(normalize_model_name("xopqwen35v35b"), "qwen3.5-35b");
        assert_eq!(normalize_model_name("xopqwen35397b"), "qwen3.5-397b");
        assert_eq!(
            normalize_model_name("xop3qwencodernext"),
            "qwen3-coder-next"
        );
        assert_eq!(normalize_model_name("xopglmv47flash"), "glm-4.7-flash");
        assert_eq!(normalize_model_name("xminimaxm25"), "minimax-m2.5");
    }

    #[test]
    fn normalize_strips_thinking_level_suffixes() {
        assert_eq!(normalize_model_name("gpt-5.5:xhigh"), "gpt-5.5");
        assert_eq!(normalize_model_name("gpt-5.5:high"), "gpt-5.5");
        assert_eq!(
            normalize_model_name("deepseek-v4-pro:xhigh"),
            "deepseek-v4-pro"
        );
        assert_eq!(normalize_model_name("kimi-k2.6:high"), "kimi-k2.6");
        assert_eq!(normalize_model_name("kimi-k2.6:xhigh"), "kimi-k2.6");
    }

    #[test]
    fn normalize_preserves_non_thinking_colons() {
        // Model names with colons that aren't thinking levels (e.g. qoder models)
        // should not be stripped
        let unknown = normalize_model_name("some-model:unknown");
        assert_eq!(
            unknown, "some-model:unknown",
            "unknown colon suffix preserved"
        );
    }

    #[test]
    fn normalize_thinking_level_with_xunfei_models() {
        // Thinking level should be stripped before xunfei normalization
        assert_eq!(normalize_model_name("astron-code-latest:xhigh"), "glm-5.1");
    }

    #[test]
    fn resolve_qoder_models() {
        assert_eq!(resolve_provider_from_model("qmodel_latest"), "qoder");
        assert_eq!(resolve_provider_from_model("efficient"), "qoder");
        assert_eq!(resolve_provider_from_model("auto"), "qoder");
    }

    #[test]
    fn commandcode_input_normalization() {
        // Simulate what load_all_sources does: normalize commandcode
        // input_tokens from OpenAI convention to Anthropic convention.
        // Only pi-origin records are normalized here; native cmd is
        // subtracted in the commandcode parser.
        let mut record = TokenRecord {
            date: "2026-05-25".to_string(),
            time: "2026-05-25T12:46:55Z".to_string(),
            api_key_prefix: "sk-test".to_string(),
            provider: "commandcode".to_string(),
            original_provider: None,
            model: "deepseek/deepseek-v4-flash".to_string(),
            source: "pi".to_string(),
            input_tokens: 21159, // includes cache
            output_tokens: 286,
            cache_read_tokens: 20864, // 20864 cached
            cache_write_tokens: 0,
            total_tokens: 42309, // 21159 + 286 + 20864
            cost: 0.0,
            ttft_ms: None,
            tps: None,
        };

        // Apply normalization (as load_all_sources does) — only source=="pi"
        if record.provider == "commandcode" && record.source == "pi" {
            let effective_input = (record.input_tokens - record.cache_read_tokens).max(0);
            record.input_tokens = effective_input;
            record.total_tokens = effective_input
                + record.output_tokens
                + record.cache_read_tokens
                + record.cache_write_tokens;
        }

        // input should be 21159 - 20864 = 295 (only new uncached input)
        assert_eq!(record.input_tokens, 295);
        assert_eq!(record.total_tokens, 295 + 286 + 20864);
        // Native cmd source is NOT normalized (already exclusive)
        let native = TokenRecord {
            date: "2026-08-18".to_string(),
            time: "2026-08-18T23:22:28.444Z".to_string(),
            api_key_prefix: "N/A".to_string(),
            provider: "commandcode".to_string(),
            original_provider: None,
            model: "muse-spark-1.2-contributor".to_string(),
            source: "commandcode".to_string(),
            input_tokens: 29710,
            output_tokens: 345,
            cache_read_tokens: 177,
            cache_write_tokens: 0,
            total_tokens: 30232,
            cost: 0.0,
            ttft_ms: None,
            tps: None,
        };
        // Guard: native cmd records must NOT be normalized (exclusive already)
        let mut native_clone = native.clone();
        if native_clone.provider == "commandcode" && native_clone.source == "pi" {
            let eff = (native_clone.input_tokens - native_clone.cache_read_tokens).max(0);
            native_clone.input_tokens = eff;
        }
        assert_eq!(native_clone.input_tokens, 29710, "native cmd should stay unmodified");
        // Non-commandcode records unchanged either way
        let normal = TokenRecord {
            date: "2026-05-25".to_string(),
            time: "2026-05-25T12:00:00Z".to_string(),
            api_key_prefix: "sk-test".to_string(),
            provider: "openai".to_string(),
            original_provider: None,
            model: "gpt-5.5".to_string(),
            source: "codex".to_string(),
            input_tokens: 10000,
            output_tokens: 5000,
            cache_read_tokens: 2000,
            cache_write_tokens: 0,
            total_tokens: 17000,
            cost: 0.0,
            ttft_ms: None,
            tps: None,
        };
        assert_eq!(normal.input_tokens, 10000);
        assert_eq!(normal.total_tokens, 17000);
    }

    #[test]
    fn kimi_for_coding_renamed_to_k2_7_after_cutoff() {
        // After 2026-06-12T10:00:00Z (18:00 Beijing time), kimi-for-coding
        // records with provider="kimi" should be renamed to kimi-k2.7.
        //
        // In the real pipeline, vendor merge runs first: "kimi-coding" → "kimi".
        // So by the time this normalization runs, all kimi-family providers
        // already have provider="kimi".
        let cutoff = chrono::DateTime::parse_from_rfc3339("2026-06-12T10:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);

        fn make_record(provider: &str, model: &str, time: &str) -> TokenRecord {
            TokenRecord {
                date: time[..10].to_string(),
                time: time.to_string(),
                api_key_prefix: "test".to_string(),
                provider: provider.to_string(),
                original_provider: None,
                model: model.to_string(),
                source: "test".to_string(),
                input_tokens: 100,
                output_tokens: 50,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
                total_tokens: 150,
                cost: 0.0,
                ttft_ms: None,
                tps: None,
            }
        }

        let mut records = vec![
            // Before cutoff — should NOT be renamed
            make_record("kimi", "kimi-for-coding", "2026-06-11T22:00:00Z"),
            // Exactly at cutoff — should be renamed
            make_record("kimi", "kimi-for-coding", "2026-06-12T10:00:00Z"),
            // After cutoff — should be renamed (e.g. pi with kimi-coding already merged)
            make_record("kimi", "kimi-for-coding", "2026-06-15T08:30:00Z"),
            // Different provider — should NOT be renamed
            make_record("opencode-go", "kimi-for-coding", "2026-06-15T08:30:00Z"),
            // Already kimi-k2.7 — should NOT be changed
            make_record("kimi", "kimi-k2.7", "2026-06-15T08:30:00Z"),
        ];

        // Apply the same logic as load_all_sources() (post vendor-merge)
        let mut renamed = 0usize;
        for record in records.iter_mut() {
            if record.provider == "kimi" && record.model == "kimi-for-coding" {
                if let Ok(record_time) = chrono::DateTime::parse_from_rfc3339(&record.time) {
                    if record_time.with_timezone(&chrono::Utc) >= cutoff {
                        record.model = "kimi-k2.7".to_string();
                        renamed += 1;
                    }
                }
            }
        }

        assert_eq!(renamed, 2, "exactly 2 records should be renamed");
        assert_eq!(
            records[0].model, "kimi-for-coding",
            "before cutoff: unchanged"
        );
        assert_eq!(records[1].model, "kimi-k2.7", "at cutoff: renamed");
        assert_eq!(records[2].model, "kimi-k2.7", "after cutoff: renamed");
        assert_eq!(
            records[3].model, "kimi-for-coding",
            "non-kimi provider: unchanged"
        );
        assert_eq!(records[4].model, "kimi-k2.7", "already k2.7: unchanged");
    }
}
