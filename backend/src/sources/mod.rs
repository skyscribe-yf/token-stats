//! Data source abstractions for loading token usage records.
//!
//! Each data source (Pi, Codex, Claude Code, Kimi CLI, OpenCode, ccswitch)
//! implements the `DataSource` trait. The `load_all_sources()` function
//! orchestrates loading all configured sources and applies vendor merging.

mod ccswitch;
mod claude_code;
mod codex;
mod kimi_cli;
mod kimi_code;
mod opencode;
mod pi;

use crate::config;
use crate::models::TokenRecord;
use chrono::{DateTime, Utc};
use std::path::Path;

pub use ccswitch::CcSwitchSource;
pub use claude_code::ClaudeCodeSource;
pub use codex::CodexSource;
pub use kimi_cli::KimiCliSource;
pub use kimi_code::KimiCodeSource;
pub use opencode::OpenCodeSource;
pub use pi::PiSource;

/// Trait for a data source that produces `TokenRecord` batches.
pub trait DataSource: Send + Sync {
    /// Human-readable name for logging.
    fn name(&self) -> &'static str;

    /// Load all records from this source.
    fn load(&self) -> Vec<TokenRecord>;
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
        "kimi-for-coding" | "kimi-k2" | "kimi-k2.6" | "kimi-k2.5" => "kimi".to_string(),
        "astron-code-latest" => "xunfei".to_string(),
        "mimo-v2.5-pro" | "mimo-v2-pro" | "mimo-v2.5" => "xiaomi-mimo".to_string(),
        "deepseek-v4-pro" | "deepseek-v4-flash" => "deepseek".to_string(),
        "gpt-5.5" | "gpt-5.4" | "gpt-5.4-mini" => "openai".to_string(),
        "glm-5.1" => "opencode-go".to_string(),
        "sonnet" | "haiku" => "anthropic".to_string(),
        _ if model.starts_with("claude-") => "anthropic".to_string(),
        _ => model.to_string(),
    }
}

// ─── Model name normalization ────────────────────────────────────────────────

/// Normalize model names across sources so the same model appears under one name.
///
/// Pi uses `claude-opus-4.7` (dot) while Claude Code uses `claude-opus-4-7` (hyphen).
pub fn normalize_model_name(model: &str) -> String {
    // Normalize claude-opus-4.7 -> claude-opus-4-7
    if let Some(rest) = model.strip_prefix("claude-opus-") {
        return format!("claude-opus-{}", rest.replace('.', "-"));
    }
    model.to_string()
}

// ─── Load all sources ────────────────────────────────────────────────────────

pub fn load_all_sources() -> Vec<TokenRecord> {
    let mut all_records = Vec::new();

    let sources: Vec<Box<dyn DataSource>> = {
        let mut v: Vec<Box<dyn DataSource>> = vec![
            Box::new(PiSource),
            Box::new(CodexSource),
            Box::new(ClaudeCodeSource),
            Box::new(OpenCodeSource),
            Box::new(KimiCliSource),
            Box::new(KimiCodeSource),
        ];
        if std::env::var("USE_CC_SWITCH").is_ok() {
            v.push(Box::new(CcSwitchSource));
        }
        v
    };

    for src in &sources {
        let records = src.load();
        tracing::info!("Loaded {} records from {}", records.len(), src.name());
        all_records.extend(records);
    }

    tracing::info!("Total records across all sources: {}", all_records.len());

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
    // Command Code API uses OpenAI convention: input_tokens includes
    // cache_read_tokens. Subtract to normalize (matching Codex parser).
    // This ensures correct pricing and cache hit ratio calculations.
    for record in all_records.iter_mut() {
        if record.provider == "commandcode" {
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

    all_records
}

#[cfg(test)]
mod tests {
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
    fn commandcode_input_normalization() {
        // Simulate what load_all_sources does: normalize commandcode
        // input_tokens from OpenAI convention to Anthropic convention
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
        };

        // Apply normalization (as load_all_sources does)
        let effective_input = (record.input_tokens - record.cache_read_tokens).max(0);
        record.input_tokens = effective_input;
        record.total_tokens = effective_input
            + record.output_tokens
            + record.cache_read_tokens
            + record.cache_write_tokens;

        // input should be 21159 - 20864 = 295 (only new uncached input)
        assert_eq!(record.input_tokens, 295);
        assert_eq!(record.total_tokens, 295 + 286 + 20864);
        // No change for non-commandcode records
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
        };
        assert_eq!(normal.input_tokens, 10000);
        assert_eq!(normal.total_tokens, 17000);
    }
}
