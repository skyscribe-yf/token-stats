//! Real-time cost calculation and pricing configuration.
//!
//! Stored `TokenRecord.cost` currency varies by source/provider:
//! - Pi provider `deepseek`: **CNY** (official DeepSeek API prices in yuan)
//! - Pi other providers (ainaiba, opencode-go, guancha, etc.): **USD**
//! - OpenCode DB records (source="opencode"): **USD**
//! - Codex/Claude-code: no stored cost, computed from tokens
//!
//! The `display_cost()` function converts everything to **CNY** on-the-fly
//! using the current `pricing.toml` configuration.

use crate::models::TokenRecord;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

// ── Configuration structs ────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpecialPricing {
    pub xunfei_per_call: f64,
    pub kimi_per_token: f64,
    pub opencode_divisor: f64,
    pub ainaba_divisor: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelPriceConfig {
    pub name: String,
    pub input: f64,
    pub output: f64,
    #[serde(default)]
    pub cache_read: f64,
    #[serde(default)]
    pub cache_write: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PricingConfig {
    pub usd_to_cny: f64,
    pub rate_date: String,
    pub special: SpecialPricing,
    #[serde(default)]
    pub model: Vec<ModelPriceConfig>,
}

impl Default for PricingConfig {
    fn default() -> Self {
        Self {
            usd_to_cny: 6.82,
            rate_date: "2026-05-20".to_string(),
            special: SpecialPricing {
                xunfei_per_call: 199.0 / 90_000.0,
                kimi_per_token: 199.0 / 1_500_000_000.0,
                opencode_divisor: 6.0,
                ainaba_divisor: 1.0,
            },
            model: Vec::new(),
        }
    }
}

impl PricingConfig {
    /// Build a fast lookup map from model names to prices.
    fn build_model_map(&self) -> HashMap<String, ModelPrice> {
        let mut map = HashMap::with_capacity(self.model.len());
        for m in &self.model {
            map.insert(
                m.name.clone(),
                ModelPrice {
                    input: m.input,
                    output: m.output,
                    cache_read: m.cache_read,
                    cache_write: m.cache_write,
                },
            );
        }
        map
    }
}

#[derive(Debug, Clone)]
struct ModelPrice {
    input: f64,
    output: f64,
    cache_read: f64,
    cache_write: f64,
}

/// Internal state that holds both the user config and the derived lookup map.
struct PricingState {
    config: PricingConfig,
    model_map: HashMap<String, ModelPrice>,
}

impl PricingState {
    fn new(config: PricingConfig) -> Self {
        let model_map = config.build_model_map();
        Self { config, model_map }
    }

    fn reload(&mut self, config: PricingConfig) {
        self.model_map = config.build_model_map();
        self.config = config;
    }
}

// ── Global state ─────────────────────────────────────────────────────────────

fn state_cell() -> &'static Mutex<PricingState> {
    static CELL: OnceLock<Mutex<PricingState>> = OnceLock::new();
    CELL.get_or_init(|| Mutex::new(PricingState::new(PricingConfig::default())))
}

// ── Config file loading ──────────────────────────────────────────────────────

pub fn config_path() -> std::path::PathBuf {
    if let Ok(p) = std::env::var("PRICING_CONFIG") {
        return std::path::PathBuf::from(p);
    }

    // Try next to the running binary
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let candidate = dir.join("pricing.toml");
            if candidate.exists() {
                return candidate;
            }
        }
    }

    // Fallback: current working directory
    std::path::PathBuf::from("pricing.toml")
}

fn load_config_from_file(path: &std::path::Path) -> PricingConfig {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(
                "pricing.toml not found at {:?}: {}. Using defaults.",
                path,
                e
            );
            return PricingConfig::default();
        }
    };
    match toml::from_str::<PricingConfig>(&content) {
        Ok(cfg) => {
            tracing::info!("Loaded pricing config from {:?}", path);
            cfg
        }
        Err(e) => {
            tracing::warn!("Failed to parse pricing.toml: {}. Using defaults.", e);
            PricingConfig::default()
        }
    }
}

/// Initialize global pricing state from the config file (called once at startup).
pub fn init() {
    let path = config_path();
    let config = load_config_from_file(&path);
    let mut state = state_cell().lock().unwrap();
    *state = PricingState::new(config);
}

/// Reload pricing configuration from disk without restarting the server.
pub fn reload() {
    let path = config_path();
    let config = load_config_from_file(&path);
    let mut state = state_cell().lock().unwrap();
    state.reload(config);
    tracing::info!("Pricing configuration reloaded from {:?}", path);
}

/// Return a clone of the current pricing configuration (for the API endpoint).
pub fn get_config() -> PricingConfig {
    state_cell().lock().unwrap().config.clone()
}

// ── Model price resolution ───────────────────────────────────────────────────

fn resolve_model_price<'a>(
    state: &'a PricingState,
    model: &str,
    provider: &str,
) -> Option<&'a ModelPrice> {
    // Exact match first
    if let Some(p) = state.model_map.get(model) {
        return Some(p);
    }

    let model_lower = model.to_lowercase();

    // OpenAI family — match by model name first, then fallback by provider
    if model_lower.contains("gpt-5.4-mini") {
        return state.model_map.get("gpt-5.4-mini");
    }
    if model_lower.contains("gpt-5.4") {
        return state.model_map.get("gpt-5.4");
    }
    if model_lower.contains("gpt-5.5") || provider == "openai" {
        return state.model_map.get("gpt-5.5");
    }

    // Anthropic family — match by model name first, then fallback by provider
    if model_lower.contains("opus") {
        return state.model_map.get("claude-opus-4-7");
    }
    if model_lower.contains("sonnet") {
        return state.model_map.get("claude-sonnet-4-6");
    }
    if model_lower.contains("haiku") || provider == "anthropic" {
        return state.model_map.get("claude-haiku-4-5");
    }

    None
}

// ── Cost calculation ─────────────────────────────────────────────────────────

/// Compute the display cost (CNY) for a single record based on the current
/// pricing configuration.
///
/// Currency conventions by Pi provider (from models.json):
/// - `deepseek`: cost is in **CNY** (official DeepSeek API)
/// - All other providers (ainaiba, opencode-go, guancha, xiaomi-mimo, etc.):
///   cost is in **USD**
/// - OpenCode DB records (source="opencode"): cost is in USD
/// - Codex/Claude-code: no stored cost, computed from tokens using pricing.toml (USD)
pub fn display_cost(record: &TokenRecord) -> f64 {
    let state = state_cell().lock().unwrap();
    let cfg = &state.config;

    // 1. 讯飞 (xunfei): flat per-call rate in CNY
    if record.provider == "xunfei" {
        return cfg.special.xunfei_per_call;
    }

    // 2. Kimi provider with zero stored cost: per-token estimate in CNY
    //    Covers pi-sourced and kimi-cli records where vendor merge mapped
    //    provider to "kimi" and no cost was recorded by the upstream tool.
    if record.provider == "kimi" && record.cost == 0.0 {
        return record.total_tokens as f64 * cfg.special.kimi_per_token;
    }

    // 3. OpenCode source (direct from OpenCode DB): cost is in USD
    //    Apply OpenCode Go plan divisor + convert to CNY
    if record.source == "opencode" && record.cost > 0.0 {
        return record.cost / cfg.special.opencode_divisor * cfg.usd_to_cny;
    }

    // 4. Records with stored cost (Pi source, or others that recorded cost)
    if record.cost > 0.0 {
        // 4a. DeepSeek official Pi provider: cost is in CNY, display as-is
        //     Use original_provider to distinguish from opencode-go records
        //     that were merged into deepseek vendor.
        let effective_provider = record
            .original_provider
            .as_deref()
            .unwrap_or(&record.provider);
        if effective_provider == "deepseek" {
            return record.cost;
        }

        // 4b. opencode-go Pi provider: cost is in USD from OpenCode API
        //     Apply OpenCode Go plan divisor + convert to CNY
        if effective_provider == "opencode-go" {
            return record.cost / cfg.special.opencode_divisor * cfg.usd_to_cny;
        }

        // 4c. Other Pi providers: cost is in USD, convert to CNY
        let mut cny = record.cost * cfg.usd_to_cny;

        // Ainaba 40x discount: all records going through ainaibahub.com
        // (provider="ainaba" after vendor merge, covering both Pi and Codex)
        if record.provider == "ainaba" {
            cny /= cfg.special.ainaba_divisor;
        }

        return cny;
    }

    // 5. Derived sources without original cost: codex, claude-code, etc.
    //    Compute from per-model token rates. pricing.toml model prices are in USD.
    if record.source == "codex" || record.source == "claude-code" {
        if let Some(price) = resolve_model_price(&state, &record.model, &record.provider) {
            let input_cost = record.input_tokens as f64 * price.input / 1_000_000.0;
            let cache_read_cost = record.cache_read_tokens as f64 * price.cache_read / 1_000_000.0;
            let output_cost = record.output_tokens as f64 * price.output / 1_000_000.0;
            let cache_write_cost =
                record.cache_write_tokens as f64 * price.cache_write / 1_000_000.0;
            let usd = input_cost + cache_read_cost + output_cost + cache_write_cost;
            let mut cny = usd * cfg.usd_to_cny;
            // Ainaba 40x discount: all records going through ainaibahub.com
            if record.provider == "ainaba" {
                cny /= cfg.special.ainaba_divisor;
            }
            return cny;
        }
    }

    // Fallback: keep as-is (likely 0)
    record.cost
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_record(
        source: &str,
        provider: &str,
        model: &str,
        total_tokens: i64,
        cost: f64,
    ) -> TokenRecord {
        TokenRecord {
            date: "2026-05-22".to_string(),
            time: "2026-05-22T00:00:00Z".to_string(),
            api_key_prefix: "test".to_string(),
            provider: provider.to_string(),
            original_provider: None,
            model: model.to_string(),
            source: source.to_string(),
            input_tokens: total_tokens / 2,
            output_tokens: total_tokens / 2,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            total_tokens,
            cost,
        }
    }

    #[test]
    fn kimi_cli_zero_cost_uses_per_token_estimate() {
        // kimi-cli records have cost=0 and provider="kimi"
        let record = make_record("kimi-cli", "kimi", "kimi-k2.6", 1_000_000, 0.0);
        let cost = display_cost(&record);
        let expected = 1_000_000.0 * PricingConfig::default().special.kimi_per_token;
        assert!(cost > 0.0, "kimi-cli record should have non-zero cost, got {}", cost);
        assert!((cost - expected).abs() < 1e-10, "expected {}, got {}", expected, cost);
    }

    #[test]
    fn pi_kimi_zero_cost_uses_per_token_estimate() {
        // Pi-sourced kimi records with cost=0 should use the same formula
        let record = make_record("pi", "kimi", "kimi-k2.6", 1_000_000, 0.0);
        let cost = display_cost(&record);
        let expected = 1_000_000.0 * PricingConfig::default().special.kimi_per_token;
        assert!(cost > 0.0, "pi kimi record should have non-zero cost, got {}", cost);
        assert!((cost - expected).abs() < 1e-10, "expected {}, got {}", expected, cost);
    }

    #[test]
    fn kimi_with_stored_cost_uses_stored_cost() {
        // Records with provider="kimi" but cost>0 should use the stored cost path
        let record = make_record("pi", "kimi", "kimi-k2.6", 1_000_000, 0.05);
        let cost = display_cost(&record);
        // cost is in USD, so should be converted to CNY (0.05 * 6.82)
        let expected = 0.05 * PricingConfig::default().usd_to_cny;
        assert!((cost - expected).abs() < 1e-10,
            "kimi record with stored cost should use USD→CNY, expected {}, got {}", expected, cost);
    }

    #[test]
    fn xunfei_takes_precedence_over_kimi() {
        // xunfei provider should use flat per-call rate, not kimi per-token
        let record = make_record("pi", "xunfei", "astron-code-latest", 1_000_000, 0.0);
        let cost = display_cost(&record);
        let expected = PricingConfig::default().special.xunfei_per_call;
        assert!((cost - expected).abs() < 1e-10,
            "xunfei should use per-call rate, expected {}, got {}", expected, cost);
    }

    #[test]
    fn non_kimi_provider_zero_cost_returns_zero() {
        // Non-kimi records with cost=0 should still return 0 (fallback)
        let record = make_record("pi", "openai", "gpt-5.5", 1_000_000, 0.0);
        let cost = display_cost(&record);
        assert_eq!(cost, 0.0, "non-kimi zero-cost record should return 0");
    }
}
