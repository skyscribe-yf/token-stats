use super::DataSource;
use crate::models::TokenRecord;
use chrono::{TimeZone, Utc};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;

/// Kimi Code source: reads `~/.kimi-code*/sessions/*/*/agents/*/wire.jsonl`.
///
/// Auto-discovers all `~/.kimi-code*` directories (e.g. `~/.kimi-code`,
/// `~/.kimi-code-user2`) to support multi-account setups.
/// If `KIMI_CODE_HOME` is set, only that directory is used (backward compat).
///
/// Each session is stored under a working-directory bucket:
///   sessions/<workDirKey>/<sessionId>/agents/<agentId>/wire.jsonl
///
/// The wire format is a JSONL stream where the first line is a `metadata`
/// record and subsequent lines are agent event records. We look for
/// `usage.record` entries with `usageScope: "turn"` to capture per-turn
/// token consumption without double-counting.
#[derive(Default)]
pub struct KimiCodeSource;

impl DataSource for KimiCodeSource {
    fn name(&self) -> &'static str {
        "kimi-code"
    }

    fn load(&self) -> Vec<TokenRecord> {
        let dirs = Self::data_dirs();
        let mut all_records = Vec::new();
        for data_dir in &dirs {
            let base = data_dir.join("sessions");
            tracing::info!("Loading Kimi Code data from: {:?}", base);
            let records = Self::parse(&base);
            tracing::info!(
                "Loaded {} kimi-code records from {:?}",
                records.len(),
                data_dir
            );
            all_records.extend(records);
        }
        tracing::info!(
            "Loaded {} total kimi-code records across {} directories",
            all_records.len(),
            dirs.len()
        );
        all_records
    }

    fn is_available(&self) -> bool {
        Self::data_dirs().iter().any(|d| d.join("sessions").exists())
    }
}

impl KimiCodeSource {
    /// Discover all kimi-code home directories.
    ///
    /// If `KIMI_CODE_HOME` is set, uses only that path (backward compat).
    /// Otherwise, auto-discovers all `~/.kimi-code*` directories (e.g.
    /// `~/.kimi-code`, `~/.kimi-code-user2`) to support multi-account setups.
    fn data_dirs() -> Vec<PathBuf> {
        let home = super::home_dir();

        // Explicit override takes precedence — single directory, backward compatible.
        if let Ok(path) = std::env::var("KIMI_CODE_HOME") {
            return vec![PathBuf::from(path)];
        }

        // Auto-discover all ~/.kimi-code* directories.
        let mut dirs = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&home) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                if name_str.starts_with(".kimi-code") && entry.path().is_dir() {
                    dirs.push(entry.path());
                }
            }
        }

        // Ensure ~/.kimi-code is always included even if the glob missed it.
        let default = home.join(".kimi-code");
        if !dirs.iter().any(|d| *d == default) {
            dirs.push(default);
        }

        dirs.sort();
        dirs
    }

    fn parse(base_path: &std::path::Path) -> Vec<TokenRecord> {
        if !base_path.exists() {
            tracing::warn!(
                "Kimi Code sessions dir not found at {:?}, skipping",
                base_path
            );
            return Vec::new();
        }

        let mut records = Vec::new();

        let entries = match super::walkdir(base_path) {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!("Failed to walk Kimi Code sessions dir: {}", e);
                return records;
            }
        };

        for wire_path in entries {
            if !wire_path.to_string_lossy().ends_with("wire.jsonl") {
                continue;
            }

            let file = match File::open(&wire_path) {
                Ok(f) => f,
                Err(_) => continue,
            };

            let reader = BufReader::new(file);
            for line in reader.lines().map_while(Result::ok) {
                if line.trim().is_empty() {
                    continue;
                }
                let Ok(msg) = serde_json::from_str::<serde_json::Value>(&line) else {
                    continue;
                };

                let Some(record_type) = msg.get("type").and_then(|t| t.as_str()) else {
                    continue;
                };

                // Skip metadata header and all non-usage records
                if record_type != "usage.record" {
                    continue;
                }

                // Only count turn-level usage to avoid aggregating
                // session-level totals that would double-count.
                let usage_scope = msg.get("usageScope").and_then(|v| v.as_str());
                if usage_scope != Some("turn") {
                    continue;
                }

                let Some(usage) = msg.get("usage") else {
                    continue;
                };

                let input_other = usage
                    .get("inputOther")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0);
                let output = usage.get("output").and_then(|v| v.as_i64()).unwrap_or(0);
                let cache_read = usage
                    .get("inputCacheRead")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0);
                let cache_creation = usage
                    .get("inputCacheCreation")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0);

                // Split vendor prefix: "kimi-code/kimi-for-coding" -> provider="kimi-code", model="kimi-for-coding"
                let raw_model = msg
                    .get("model")
                    .and_then(|v| v.as_str())
                    .unwrap_or("kimi-for-coding");
                let (provider_from_prefix, model) = match raw_model.split_once('/') {
                    Some((vendor, model_name)) => (Some(vendor.to_string()), model_name),
                    None => (None, raw_model),
                };

                // Timestamps in kimi-code wire are milliseconds since epoch.
                let timestamp_ms = msg.get("time").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let secs = (timestamp_ms / 1000.0) as i64;
                let dt = Utc.timestamp_opt(secs, 0).single();
                let (date, time) = match dt {
                    Some(dt) => (dt.format("%Y-%m-%d").to_string(), dt.to_rfc3339()),
                    None => ("unknown".to_string(), "unknown".to_string()),
                };

                let total = input_other + output + cache_read + cache_creation;

                // Resolve model vendor first — kimi-code is a kimi subscription,
                // so all kimi-family models should bill as provider="kimi" regardless
                // of which API proxy routed them (e.g. "opencode-go/kimi-k2.6").
                // The vendor prefix is just API routing info; the model's actual
                // vendor determines billing.
                let resolved = Self::resolve_provider(model);
                let provider = if resolved == "kimi" {
                    "kimi".to_string()
                } else {
                    provider_from_prefix.unwrap_or(resolved)
                };

                records.push(TokenRecord {
                    date,
                    time,
                    api_key_prefix: "N/A".to_string(),
                    provider,
                    original_provider: None,
                    model: super::normalize_model_name(model),
                    source: "kimi-code".to_string(),
                    input_tokens: input_other,
                    output_tokens: output,
                    cache_read_tokens: cache_read,
                    cache_write_tokens: cache_creation,
                    total_tokens: total,
                    cost: 0.0,
                    ttft_ms: None,
                    tps: None,
                });
            }
        }

        records
    }

    /// Resolve provider from a kimi-code model name.
    ///
    /// kimi-code can use models from multiple providers (kimi, anthropic,
    /// openai, deepseek, etc.). We try to infer the provider from known
    /// model name patterns.
    fn resolve_provider(model: &str) -> String {
        match model {
            "kimi-for-coding" | "kimi-k2" | "kimi-k2.5" | "kimi-k2.6" | "kimi-k2.7" | "k3" => "kimi".to_string(),
            "astron-code-latest" => "xunfei".to_string(),
            "mimo-v2.5-pro" | "mimo-v2-pro" | "mimo-v2.5" => "xiaomi-mimo".to_string(),
            "deepseek-v4-pro" | "deepseek-v4-flash" => "deepseek".to_string(),
            "gpt-5.5" | "gpt-5.4" | "gpt-5.4-mini" => "openai".to_string(),
            "glm-5.1" => "opencode-go".to_string(),
            "sonnet" | "haiku" => "anthropic".to_string(),
            _ if model.starts_with("claude-") => "anthropic".to_string(),
            _ if model.starts_with("kimi-") => "kimi".to_string(),
            _ if model.starts_with("gpt-") => "openai".to_string(),
            _ if model.starts_with("deepseek-") => "deepseek".to_string(),
            _ => model.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_kimi_models() {
        assert_eq!(KimiCodeSource::resolve_provider("kimi-k2"), "kimi");
        assert_eq!(KimiCodeSource::resolve_provider("kimi-k2.5"), "kimi");
        assert_eq!(KimiCodeSource::resolve_provider("kimi-for-coding"), "kimi");
    }

    #[test]
    fn resolve_openai_models() {
        assert_eq!(KimiCodeSource::resolve_provider("gpt-5.5"), "openai");
        assert_eq!(KimiCodeSource::resolve_provider("gpt-4o"), "openai");
    }

    #[test]
    fn resolve_anthropic_models() {
        assert_eq!(
            KimiCodeSource::resolve_provider("claude-sonnet-4"),
            "anthropic"
        );
        assert_eq!(KimiCodeSource::resolve_provider("sonnet"), "anthropic");
    }

    #[test]
    fn resolve_unknown_model_returns_model_name() {
        assert_eq!(
            KimiCodeSource::resolve_provider("some-exotic-model"),
            "some-exotic-model"
        );
    }

    #[test]
    fn strip_vendor_prefix_from_model() {
        // Simulate what the parser does: split vendor/model
        let raw = "kimi-code/kimi-for-coding";
        let (prefix, model) = raw.split_once('/').unwrap();
        assert_eq!(prefix, "kimi-code");
        assert_eq!(model, "kimi-for-coding");

        let raw = "ainaiba/gpt-5.4";
        let (prefix, model) = raw.split_once('/').unwrap();
        assert_eq!(prefix, "ainaiba");
        assert_eq!(model, "gpt-5.4");

        let raw = "xunfei/astron-code-latest";
        let (prefix, model) = raw.split_once('/').unwrap();
        assert_eq!(prefix, "xunfei");
        assert_eq!(model, "astron-code-latest");

        // No prefix - uses resolve_provider fallback
        let raw = "kimi-for-coding";
        assert!(raw.split_once('/').is_none());
        assert_eq!(KimiCodeSource::resolve_provider(raw), "kimi");
    }

    #[test]
    fn kimi_models_always_resolve_to_kimi_provider() {
        // Regardless of the vendor prefix in the wire record, kimi-family models
        // should always get provider="kimi" for correct subscription billing.
        fn resolve_provider_for_raw(raw: &str) -> String {
            let (prefix, model) = match raw.split_once('/') {
                Some((p, m)) => (Some(p), m),
                None => (None, raw),
            };
            let resolved = KimiCodeSource::resolve_provider(model);
            if resolved == "kimi" {
                "kimi".to_string()
            } else {
                prefix.unwrap_or(&resolved).to_string()
            }
        }

        // kimi models with various prefixes → always "kimi"
        assert_eq!(resolve_provider_for_raw("opencode-go/kimi-k2.6"), "kimi");
        assert_eq!(
            resolve_provider_for_raw("kimi-code/kimi-for-coding"),
            "kimi"
        );
        assert_eq!(resolve_provider_for_raw("kimi-code/kimi-k2.5"), "kimi");
        assert_eq!(resolve_provider_for_raw("kimi-k2.6"), "kimi");

        // Non-kimi models keep their prefix-based provider
        assert_eq!(resolve_provider_for_raw("ainaiba/gpt-5.4"), "ainaiba");
        assert_eq!(
            resolve_provider_for_raw("xunfei/astron-code-latest"),
            "xunfei"
        );
        assert_eq!(
            resolve_provider_for_raw("deepseek/deepseek-v4-pro"),
            "deepseek"
        );
    }
}
