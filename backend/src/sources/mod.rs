//! Data source abstractions for loading token usage records.
//!
//! Each data source (Pi, Codex, Claude Code, Kimi CLI, OpenCode, ccswitch)
//! implements the `DataSource` trait. The `load_all_sources()` function
//! orchestrates loading all configured sources and applies vendor merging.

mod ccswitch;
mod claude_code;
mod codex;
mod kimi_cli;
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
        "kimi-for-coding" | "kimi-k2.6" | "kimi-k2.5" => "kimi".to_string(),
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
}
