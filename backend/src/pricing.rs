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

/// Time-based rate segment for Ainaba (AI奶爸) pricing.
/// Segments should be ordered from earliest cutoff to latest.
/// The last segment should have no `before` (catch-all for the current rate).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AinabaSegment {
    /// Records whose time is before this timestamp use this segment's divisor.
    /// If `None`, this is the catch-all segment (applies to all remaining records).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<String>,
    pub divisor: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpecialPricing {
    pub xunfei_per_call: f64,
    pub kimi_per_token: f64,
    #[serde(default)]
    pub xiaomi_mimo_tp_per_token: f64,
    pub opencode_divisor: f64,
    /// Legacy single-value divisor. Kept for backward compatibility.
    /// When `ainaba_segments` is non-empty, segments take precedence.
    #[serde(default)]
    pub ainaba_divisor: f64,
    /// Time-based rate segments (preferred). If empty, falls back to `ainaba_divisor`.
    #[serde(default)]
    pub ainaba_segments: Vec<AinabaSegment>,
    #[serde(default)]
    pub freemodel_divisor: f64,
    #[serde(default)]
    pub commandcode_divisor: f64,
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
    /// Tier threshold in total input tokens (input + cache_read + cache_write).
    /// None = base tier (threshold 0). Some(128000) = applies when total_input >= 128K.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tier_threshold: Option<i64>,
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
                kimi_per_token: 199.0 / 2_800_000_000.0,
                // 99 CNY subscription, 268M platform tokens ≈ 16.36M dashboard tokens (~2.44% usage)
                // effective per-token = 99 * 0.0244 / 16_360_000 ≈ 0.0000001479
                xiaomi_mimo_tp_per_token: 0.0000001479,
                opencode_divisor: 6.0,
                ainaba_divisor: 1.0,
                ainaba_segments: Vec::new(),
                freemodel_divisor: 68.2,
                commandcode_divisor: 1.0,
            },
            model: Vec::new(),
        }
    }
}

impl PricingConfig {
    /// Build a fast lookup map from model names to prices.
    fn build_model_map(&self) -> HashMap<String, ModelPrice> {
        // Group configs by model name
        let mut groups: HashMap<String, Vec<&ModelPriceConfig>> = HashMap::new();
        for m in &self.model {
            groups.entry(m.name.clone()).or_default().push(m);
        }
        // Build ModelPrice from each group
        groups
            .into_iter()
            .map(|(name, configs)| {
                let base_count = configs
                    .iter()
                    .filter(|c| c.tier_threshold.is_none())
                    .count();
                if base_count > 1 {
                    tracing::warn!(
                        "Model '{}' has {} base-tier entries, using last one",
                        name,
                        base_count
                    );
                } else if base_count == 0 {
                    tracing::warn!(
                        "Model '{}' has no base-tier entry (all entries specify tier_threshold); \
                         inputs below the lowest threshold will use that tier's rates",
                        name
                    );
                }
                (name, ModelPrice::from_configs(&configs))
            })
            .collect()
    }
}

#[derive(Debug, Clone)]
struct PriceTier {
    threshold: i64,
    input: f64,
    output: f64,
    cache_read: f64,
    cache_write: f64,
}

#[derive(Debug, Clone)]
struct ModelPrice {
    /// Price tiers sorted by threshold ascending. First tier has threshold=0 (base).
    tiers: Vec<PriceTier>,
}

impl ModelPrice {
    /// Build from a slice of ModelPriceConfig entries sharing the same name.
    fn from_configs(configs: &[&ModelPriceConfig]) -> Self {
        let mut tiers: Vec<PriceTier> = configs
            .iter()
            .map(|c| PriceTier {
                threshold: c.tier_threshold.unwrap_or(0),
                input: c.input,
                output: c.output,
                cache_read: c.cache_read,
                cache_write: c.cache_write,
            })
            .collect();
        tiers.sort_by_key(|t| t.threshold);
        Self { tiers }
    }

    /// Select the appropriate tier based on total input tokens.
    /// Total input = input_tokens + cache_read_tokens + cache_write_tokens.
    /// Returns the last tier whose threshold <= total_input.
    fn select_tier(&self, total_input: i64) -> &PriceTier {
        let mut selected = &self.tiers[0];
        for tier in &self.tiers {
            if total_input >= tier.threshold {
                selected = tier;
            } else {
                break;
            }
        }
        selected
    }

    /// Compute cost in USD for the given token counts.
    fn compute_usd(
        &self,
        input_tokens: i64,
        output_tokens: i64,
        cache_read_tokens: i64,
        cache_write_tokens: i64,
    ) -> f64 {
        let total_input = input_tokens + cache_read_tokens + cache_write_tokens;
        let tier = self.select_tier(total_input);
        input_tokens as f64 * tier.input / 1_000_000.0
            + output_tokens as f64 * tier.output / 1_000_000.0
            + cache_read_tokens as f64 * tier.cache_read / 1_000_000.0
            + cache_write_tokens as f64 * tier.cache_write / 1_000_000.0
    }
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

/// Normalize a Command Code model name to the `cc:` prefix used in pricing.toml.
///
/// Command Code model names come in two forms:
/// - Plain: `claude-sonnet-4-6`, `gpt-5.5` (direct CC name)
/// - Provider-prefixed: `deepseek/deepseek-v4-flash`, `moonshotai/Kimi-K2.6` (pi convention)
///
/// Maps to `cc:` prefixed keys in the pricing model map.
fn resolve_commandcode_price<'a>(state: &'a PricingState, model: &str) -> Option<&'a ModelPrice> {
    let cc_key = normalize_commandcode_model(model);
    state.model_map.get(&cc_key)
}

fn normalize_commandcode_model(model: &str) -> String {
    // Handle provider/model format: strip the provider prefix
    let model_only = if let Some(slash_pos) = model.find('/') {
        &model[slash_pos + 1..]
    } else {
        model
    };

    let lower = model_only.to_lowercase();

    // Map known CC model names to pricing.toml cc: keys
    let key = match lower.as_str() {
        // Anthropic
        "claude-opus-4-7" | "claude-opus-4.7" => "cc:claude-opus-4-7",
        "claude-opus-4-6" | "claude-opus-4.6" => "cc:claude-opus-4-6",
        "claude-opus-4-5" | "claude-opus-4.5" => "cc:claude-opus-4-6",
        "claude-sonnet-4-6" | "claude-sonnet-4.6" => "cc:claude-sonnet-4-6",
        "claude-sonnet-4-5" | "claude-sonnet-4.5" => "cc:claude-sonnet-4-6",
        s if s.starts_with("claude-haiku-4-5") => "cc:claude-haiku-4-5",

        // OpenAI
        "gpt-5.5" => "cc:gpt-5.5",
        "gpt-5.4" => "cc:gpt-5.4",
        "gpt-5.4-mini" => "cc:gpt-5.4-mini",
        "gpt-5.3-codex" => "cc:gpt-5.3-codex",

        // Google
        "gemini-3.5-flash" => "cc:gemini-3.5-flash",

        // DeepSeek
        "deepseek-v4-pro" => "cc:deepseek-v4-pro",
        "deepseek-v4-flash" => "cc:deepseek-v4-flash",

        // Moonshot/Kimi
        "kimi-k2.6" => "cc:kimi-k2.6",
        "kimi-k2.5" => "cc:kimi-k2.5",

        // Zhipu/GLM
        "glm-5.1" => "cc:glm-5.1",
        "glm-5" => "cc:glm-5",

        // MiniMax
        "minimax-m2.7" => "cc:minimax-m2.7",
        "minimax-m2.5" => "cc:minimax-m2.5",

        // Qwen
        "qwen3.6-max-preview" => "cc:qwen3.6-max-preview",
        "qwen3.6-plus" => "cc:qwen3.6-plus",
        "qwen3.7-max" => "cc:qwen3.7-max",

        // Step
        "step-3.5-flash" => "cc:step-3.5-flash",

        // Fallback: try with cc: prefix
        other => return format!("cc:{}", other),
    };

    key.to_string()
}

// ── Cost calculation ─────────────────────────────────────────────────────────

/// Select the Ainaba divisor for a record based on its timestamp.
/// Checks `ainaba_segments` first (time-based), falls back to legacy `ainaba_divisor`.
fn get_ainaba_divisor(special: &SpecialPricing, record_time: &str) -> f64 {
    if !special.ainaba_segments.is_empty() {
        if let Ok(record_dt) = chrono::DateTime::parse_from_rfc3339(record_time) {
            for segment in &special.ainaba_segments {
                if let Some(ref before) = segment.before {
                    if let Ok(cutoff) = chrono::DateTime::parse_from_rfc3339(before) {
                        if record_dt < cutoff {
                            return segment.divisor;
                        }
                    }
                } else {
                    // Catch-all segment (no `before` field)
                    return segment.divisor;
                }
            }
        }
        // If no segment matched or time parsing failed, use first segment
        return special.ainaba_segments[0].divisor;
    }
    // Fallback to legacy single-value divisor
    special.ainaba_divisor
}

/// Compute the display cost (CNY) for a single record based on the current
/// pricing configuration.
///
/// Currency conventions by Pi provider (from models.json):
/// - `deepseek`: cost is in **CNY** (official DeepSeek API)
/// - `xiaomi-mimo` / `xiaomi-mimo-tp`: cost is in **CNY** (platform subscription)
/// - All other providers (ainaiba, opencode-go, guancha, etc.):
///   cost is in **USD**
/// - OpenCode DB records (source="opencode"): cost is in USD
/// - Codex/Claude-code: no stored cost, computed from tokens using pricing.toml (USD)
/// - Records with provider=deepseek and cost=0: derived from pricing.toml
///   deepseek rates (USD→CNY, no divisor). Covers session-recovery records
///   and DeepSeek platform CSV export.
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

    // 2b. Xiaomi MiMo provider with zero stored cost: per-token estimate in CNY
    //     Similar to Kimi subscription model: 99 元 / 110 亿 Token (platform tokenization)
    //     Covers both "xiaomi-mimo" (pi direct) and "xiaomi-mimo-tp" (token plan).
    if (record.provider == "xiaomi-mimo" || record.provider == "xiaomi-mimo-tp") && record.cost == 0.0 {
        return record.total_tokens as f64 * cfg.special.xiaomi_mimo_tp_per_token;
    }

    // 3. OpenCode source (direct from OpenCode DB): cost is in USD
    //    Apply OpenCode Go plan divisor + convert to CNY
    if record.source == "opencode" && record.cost > 0.0 {
        return record.cost / cfg.special.opencode_divisor * cfg.usd_to_cny;
    }

    // 4. CommandCode provider: always compute from normalized tokens using
    //    CC model prices from pricing.toml. We ignore the extension's stored
    //    cost because the pi extension currently computes cost from raw input
    //    tokens (which include cache_read per OpenAI convention), inflating
    //    the result ~10×.
    //
    //    CC model prices in pricing.toml are the listed API rate (USD / 1M).
    //    Apply commandcode_divisor (subscription discount: actual = list / divisor),
    //    then convert to CNY.
    if record.provider == "commandcode" {
        if let Some(mp) = resolve_commandcode_price(&state, &record.model) {
            let usd = mp.compute_usd(
                record.input_tokens,
                record.output_tokens,
                record.cache_read_tokens,
                record.cache_write_tokens,
            );
            return usd * cfg.usd_to_cny / cfg.special.commandcode_divisor;
        }
    }

    // 5. Records with stored cost (Pi source, or others that recorded cost)
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

        // 4a2. Xiaomi MiMo Pi provider: cost is in CNY (from platform), display as-is
        if effective_provider == "xiaomi-mimo" || effective_provider == "xiaomi-mimo-tp" {
            return record.cost;
        }

        // 4b. opencode-go Pi provider: cost is in USD from OpenCode API
        //     Apply OpenCode Go plan divisor + convert to CNY
        if effective_provider == "opencode-go" {
            return record.cost / cfg.special.opencode_divisor * cfg.usd_to_cny;
        }

        // 4c. Other Pi providers: cost is in USD, convert to CNY
        let mut cny = record.cost * cfg.usd_to_cny;

        // Ainaba time-based rate: divisor depends on record timestamp
        // (provider="ainaba" after vendor merge, covering both Pi and Codex)
        if record.provider == "ainaba" {
            cny /= get_ainaba_divisor(&cfg.special, &record.time);
        }

        // FreeModel discount: 1 USD face value = 0.1 CNY actual cost
        // divisor = usd_to_cny / 0.1 = 68.2
        if record.provider == "FreeModel" {
            cny /= cfg.special.freemodel_divisor;
        }

        return cny;
    }

    // 4d. DeepSeek records with cost=0 (e.g. from session recovery or DeepSeek
    //     platform CSV export). pricing.toml deepseek rates are listed as USD;
    //     multiply by usd_to_cny to display in CNY. No divisor - the user
    //     pays DeepSeek directly at official rates.
    let effective_provider = record
        .original_provider
        .as_deref()
        .unwrap_or(&record.provider);
    if effective_provider == "deepseek" && record.cost == 0.0 {
        if let Some(mp) = resolve_model_price(&state, &record.model, &record.provider) {
            let usd = mp.compute_usd(
                record.input_tokens,
                record.output_tokens,
                record.cache_read_tokens,
                record.cache_write_tokens,
            );
            return usd * cfg.usd_to_cny;
        }
    }

    // 6. Derived sources without original cost: codex, claude-code, etc.
    //    Compute from per-model token rates. pricing.toml model prices are in USD.
    if record.source == "codex" || record.source == "claude-code" {
        if let Some(mp) = resolve_model_price(&state, &record.model, &record.provider) {
            let usd = mp.compute_usd(
                record.input_tokens,
                record.output_tokens,
                record.cache_read_tokens,
                record.cache_write_tokens,
            );
            let mut cny = usd * cfg.usd_to_cny;
            // Ainaba time-based rate: divisor depends on record timestamp
            if record.provider == "ainaba" {
                cny /= get_ainaba_divisor(&cfg.special, &record.time);
            }
            // FreeModel discount: 1 USD face value = 0.1 CNY actual cost
            if record.provider == "FreeModel" {
                cny /= cfg.special.freemodel_divisor;
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
    use std::sync::{Mutex, MutexGuard, OnceLock};

    fn pricing_test_guard() -> MutexGuard<'static, ()> {
        static TEST_MUTEX: OnceLock<Mutex<()>> = OnceLock::new();
        let guard = match TEST_MUTEX.get_or_init(|| Mutex::new(())).lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        std::env::remove_var("PRICING_CONFIG");
        let mut state = state_cell().lock().unwrap();
        state.reload(PricingConfig::default());
        drop(state);
        guard
    }

    /// Load a temp pricing config from TOML bytes, saving/restoring PRICING_CONFIG env var.
    /// Returns the NamedTempFile (must be kept alive for the file to exist).
    fn load_temp_config(toml: &[u8]) -> tempfile::NamedTempFile {
        use std::io::Write;
        let tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.as_file().write_all(toml).unwrap();
        std::env::set_var("PRICING_CONFIG", tmp.path().to_str().unwrap());
        reload();
        tmp
    }

    /// Restore PRICING_CONFIG env var after a temp config test.
    fn restore_pricing_env(prev: Option<String>) {
        match prev {
            Some(v) => std::env::set_var("PRICING_CONFIG", v),
            None => std::env::remove_var("PRICING_CONFIG"),
        }
        reload();
    }

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
        let _guard = pricing_test_guard();
        // kimi-cli records have cost=0 and provider="kimi"
        let record = make_record("kimi-cli", "kimi", "kimi-k2.6", 1_000_000, 0.0);
        let cost = display_cost(&record);
        let expected = 1_000_000.0 * PricingConfig::default().special.kimi_per_token;
        assert!(
            cost > 0.0,
            "kimi-cli record should have non-zero cost, got {}",
            cost
        );
        assert!(
            (cost - expected).abs() < 1e-9,
            "expected {}, got {}",
            expected,
            cost
        );
    }

    #[test]
    fn pi_kimi_zero_cost_uses_per_token_estimate() {
        let _guard = pricing_test_guard();
        // Pi-sourced kimi records with cost=0 should use the same formula
        let record = make_record("pi", "kimi", "kimi-k2.6", 1_000_000, 0.0);
        let cost = display_cost(&record);
        let expected = 1_000_000.0 * PricingConfig::default().special.kimi_per_token;
        assert!(
            cost > 0.0,
            "pi kimi record should have non-zero cost, got {}",
            cost
        );
        assert!(
            (cost - expected).abs() < 1e-9,
            "expected {}, got {}",
            expected,
            cost
        );
    }

    #[test]
    fn kimi_with_stored_cost_uses_stored_cost() {
        let _guard = pricing_test_guard();
        // Records with provider="kimi" but cost>0 should use the stored cost path
        let record = make_record("pi", "kimi", "kimi-k2.6", 1_000_000, 0.05);
        let cost = display_cost(&record);
        // cost is in USD, so should be converted to CNY (0.05 * 6.82)
        let expected = 0.05 * PricingConfig::default().usd_to_cny;
        assert!(
            (cost - expected).abs() < 1e-9,
            "kimi record with stored cost should use USD→CNY, expected {}, got {}",
            expected,
            cost
        );
    }

    #[test]
    fn xunfei_takes_precedence_over_kimi() {
        let _guard = pricing_test_guard();
        // xunfei provider should use flat per-call rate, not kimi per-token
        let record = make_record("pi", "xunfei", "astron-code-latest", 1_000_000, 0.0);
        let cost = display_cost(&record);
        let expected = PricingConfig::default().special.xunfei_per_call;
        assert!(
            (cost - expected).abs() < 1e-9,
            "xunfei should use per-call rate, expected {}, got {}",
            expected,
            cost
        );
    }

    #[test]
    fn non_kimi_provider_zero_cost_returns_zero() {
        let _guard = pricing_test_guard();
        // Non-kimi records with cost=0 should still return 0 (fallback)
        let record = make_record("pi", "openai", "gpt-5.5", 1_000_000, 0.0);
        let cost = display_cost(&record);
        assert_eq!(cost, 0.0, "non-kimi zero-cost record should return 0");
    }

    #[test]
    fn freemodel_stored_cost_applies_divisor() {
        let _guard = pricing_test_guard();
        // FreeModel records with stored cost (USD) should apply the 68.2x divisor
        // before converting to CNY: cost_usd * usd_to_cny / freemodel_divisor
        let record = make_record("pi", "FreeModel", "claude-opus-4-7", 1_000_000, 0.166844);
        let cost = display_cost(&record);
        let expected = 0.166844 * PricingConfig::default().usd_to_cny
            / PricingConfig::default().special.freemodel_divisor;
        assert!(
            cost > 0.0,
            "FreeModel record should have non-zero cost, got {}",
            cost
        );
        assert!(
            (cost - expected).abs() < 1e-9,
            "FreeModel cost should use divisor, expected {}, got {}",
            expected,
            cost
        );
    }

    #[test]
    fn freemodel_derived_cost_applies_divisor() {
        let _guard = pricing_test_guard();
        // FreeModel claude-code records (no stored cost) should compute from tokens
        // and then apply the 68.2x divisor.
        // The default PricingConfig has an empty model list, so derived-cost
        // calculation cannot resolve model prices. We write a temp config with
        // model prices so resolve_model_price() can find claude-opus-4-7.
        use std::io::Write;
        let tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.as_file()
            .write_all(
                br#"
usd_to_cny = 6.82
rate_date = "2026-05-20"

[special]
xunfei_per_call = 0.002211111111
kimi_per_token = 0.000000071071429
opencode_divisor = 6.0
ainaba_divisor = 40.0
freemodel_divisor = 68.2

[[model]]
name = "claude-opus-4-7"
input = 5.00
output = 25.00
cache_read = 0.50
cache_write = 6.25
"#,
            )
            .unwrap();

        // Save current config, then override with temp config
        let prev_config = get_config();
        let prev_env = std::env::var("PRICING_CONFIG").ok();
        std::env::set_var("PRICING_CONFIG", tmp.path().to_str().unwrap());
        reload();

        let mut record = make_record("claude-code", "FreeModel", "claude-opus-4-7", 10_000, 0.0);
        record.input_tokens = 5_000;
        record.output_tokens = 5_000;
        record.cache_read_tokens = 0;
        record.cache_write_tokens = 0;
        let cost = display_cost(&record);
        // claude-opus-4-7: input=$5/M, output=$25/M
        // usd = 5000*5/1M + 5000*25/1M = 0.025 + 0.125 = 0.15
        // cny = 0.15 * 6.82 / 68.2 = 0.015
        let usd = 5_000.0 * 5.0 / 1_000_000.0 + 5_000.0 * 25.0 / 1_000_000.0;
        let expected = usd * 6.82 / 68.2;
        assert!(
            cost > 0.0,
            "FreeModel claude-code record should have non-zero cost, got {}",
            cost
        );
        assert!(
            (cost - expected).abs() < 0.001,
            "FreeModel claude-code cost should use divisor, expected {}, got {}",
            expected,
            cost
        );

        // Restore previous config by writing it to a temp file and reloading
        let restore_tmp = tempfile::NamedTempFile::new().unwrap();
        let restore_toml = toml::to_string(&prev_config).unwrap();
        restore_tmp
            .as_file()
            .write_all(restore_toml.as_bytes())
            .unwrap();
        std::env::set_var("PRICING_CONFIG", restore_tmp.path().to_str().unwrap());
        reload();

        // Restore env var
        match prev_env {
            Some(v) => std::env::set_var("PRICING_CONFIG", v),
            None => std::env::remove_var("PRICING_CONFIG"),
        }
    }

    #[test]
    fn deepseek_zero_cost_computes_from_tokens_in_cny() {
        let _guard = pricing_test_guard();
        use std::io::Write;
        let tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.as_file()
            .write_all(
                br#"
usd_to_cny = 6.82
rate_date = "2026-05-20"

[special]
xunfei_per_call = 0.002211111111
kimi_per_token = 0.000000071071429
opencode_divisor = 6.0
ainaba_divisor = 40.0
freemodel_divisor = 68.2

[[model]]
name = "deepseek-v4-pro"
input = 0.5865
output = 2.346
cache_read = 0.05865
cache_write = 0.5865
"#,
            )
            .unwrap();

        let prev_env = std::env::var("PRICING_CONFIG").ok();
        std::env::set_var("PRICING_CONFIG", tmp.path().to_str().unwrap());
        reload();

        let mut record = make_record("deepseek-ai", "deepseek", "deepseek-v4-pro", 0, 0.0);
        record.input_tokens = 1_000_000;
        record.output_tokens = 100_000;
        record.cache_read_tokens = 500_000;
        record.cache_write_tokens = 0;
        record.total_tokens = 1_600_000;

        let cny = display_cost(&record);

        let usd = 1_000_000.0 * 0.5865 / 1_000_000.0
            + 100_000.0 * 2.346 / 1_000_000.0
            + 500_000.0 * 0.05865 / 1_000_000.0;
        let expected = usd * 6.82;

        assert!(
            cny > 0.0,
            "deepseek zero-cost record should compute non-zero, got {}",
            cny
        );
        assert!(
            (cny - expected).abs() < 0.001,
            "deepseek cost mismatch: expected {}, got {}",
            expected,
            cny
        );

        match prev_env {
            Some(v) => std::env::set_var("PRICING_CONFIG", v),
            None => std::env::remove_var("PRICING_CONFIG"),
        }
        reload();
    }

    #[test]
    fn xiaomi_mimo_tp_zero_cost_uses_per_token_estimate() {
        let _guard = pricing_test_guard();
        // xiaomi-mimo-tp records with cost=0 and provider="xiaomi-mimo-tp"
        let record = make_record("pi", "xiaomi-mimo-tp", "mimo-v2.5-pro", 1_000_000, 0.0);
        let cost = display_cost(&record);
        let expected = 1_000_000.0 * PricingConfig::default().special.xiaomi_mimo_tp_per_token;
        assert!(
            cost > 0.0,
            "xiaomi-mimo-tp record should have non-zero cost, got {}",
            cost
        );
        assert!(
            (cost - expected).abs() < 1e-9,
            "expected {}, got {}",
            expected,
            cost
        );
    }

    #[test]
    fn xiaomi_mimo_zero_cost_uses_per_token_estimate() {
        let _guard = pricing_test_guard();
        // xiaomi-mimo records with cost=0 should also use the per-token estimate
        let record = make_record("pi", "xiaomi-mimo", "mimo-v2.5-pro", 1_000_000, 0.0);
        let cost = display_cost(&record);
        let expected = 1_000_000.0 * PricingConfig::default().special.xiaomi_mimo_tp_per_token;
        assert!(
            cost > 0.0,
            "xiaomi-mimo record should have non-zero cost, got {}",
            cost
        );
        assert!(
            (cost - expected).abs() < 1e-9,
            "expected {}, got {}",
            expected,
            cost
        );
    }

    #[test]
    fn xiaomi_mimo_tp_with_stored_cost_is_cny() {
        let _guard = pricing_test_guard();
        // Records with provider="xiaomi-mimo-tp" and cost>0: cost is already in CNY
        let record = make_record("pi", "xiaomi-mimo-tp", "mimo-v2.5-pro", 1_000_000, 0.05);
        let cost = display_cost(&record);
        assert!(
            (cost - 0.05).abs() < 1e-9,
            "xiaomi-mimo-tp stored cost is CNY, expected 0.05, got {}",
            cost
        );
    }

    #[test]
    fn xiaomi_mimo_with_stored_cost_is_cny() {
        let _guard = pricing_test_guard();
        // Records with provider="xiaomi-mimo" and cost>0: cost is already in CNY
        let record = make_record("pi", "xiaomi-mimo", "mimo-v2.5-pro", 38537, 0.039);
        let cost = display_cost(&record);
        assert!(
            (cost - 0.039).abs() < 1e-9,
            "xiaomi-mimo stored cost is CNY, expected 0.039, got {}",
            cost
        );
    }

    #[test]
    fn tiered_pricing_base_tier_for_short_context() {
        let _guard = pricing_test_guard();
        let prev_env = std::env::var("PRICING_CONFIG").ok();
        let _tmp = load_temp_config(
            br#"
usd_to_cny = 6.82
rate_date = "2026-05-20"

[special]
xunfei_per_call = 0.002211111111
kimi_per_token = 0.000000071071429
opencode_divisor = 6.0
ainaba_divisor = 40.0
freemodel_divisor = 68.2

[[model]]
name = "gpt-5.5"
input = 5.00
output = 30.00
cache_read = 0.50
cache_write = 5.00

[[model]]
name = "gpt-5.5"
tier_threshold = 272000
input = 10.00
output = 45.00
cache_read = 1.00
cache_write = 10.00
"#,
        );

        // Short context (50K input) → should use base tier
        let mut record = make_record("codex", "openai", "gpt-5.5", 0, 0.0);
        record.input_tokens = 50_000;
        record.output_tokens = 10_000;
        record.cache_read_tokens = 0;
        record.cache_write_tokens = 0;
        record.total_tokens = 60_000;

        let cny = display_cost(&record);
        // Base tier: input=$5/M, output=$30/M
        // usd = 50000*5/1M + 10000*30/1M = 0.25 + 0.30 = 0.55
        let expected = 0.55 * 6.82;
        assert!(
            (cny - expected).abs() < 0.001,
            "base tier: expected {}, got {}",
            expected,
            cny
        );

        restore_pricing_env(prev_env);
    }

    #[test]
    fn tiered_pricing_high_tier_for_long_context() {
        let _guard = pricing_test_guard();
        let prev_env = std::env::var("PRICING_CONFIG").ok();
        let _tmp = load_temp_config(
            br#"
usd_to_cny = 6.82
rate_date = "2026-05-20"

[special]
xunfei_per_call = 0.002211111111
kimi_per_token = 0.000000071071429
opencode_divisor = 6.0
ainaba_divisor = 40.0
freemodel_divisor = 68.2

[[model]]
name = "gpt-5.5"
input = 5.00
output = 30.00
cache_read = 0.50
cache_write = 5.00

[[model]]
name = "gpt-5.5"
tier_threshold = 272000
input = 10.00
output = 45.00
cache_read = 1.00
cache_write = 10.00
"#,
        );

        // Long context (300K total input) → should use high tier
        let mut record = make_record("codex", "openai", "gpt-5.5", 0, 0.0);
        record.input_tokens = 250_000;
        record.output_tokens = 10_000;
        record.cache_read_tokens = 50_000; // total_input = 300K > 272K
        record.cache_write_tokens = 0;
        record.total_tokens = 310_000;

        let cny = display_cost(&record);
        // High tier: input=$10/M, output=$45/M, cache_read=$1/M
        // usd = 250000*10/1M + 10000*45/1M + 50000*1/1M = 2.5 + 0.45 + 0.05 = 3.0
        let expected = 3.0 * 6.82;
        assert!(
            (cny - expected).abs() < 0.001,
            "high tier: expected {}, got {}",
            expected,
            cny
        );

        restore_pricing_env(prev_env);
    }

    #[test]
    fn tiered_pricing_exactly_at_threshold() {
        let _guard = pricing_test_guard();
        let prev_env = std::env::var("PRICING_CONFIG").ok();
        let _tmp = load_temp_config(
            br#"
usd_to_cny = 6.82
rate_date = "2026-05-20"

[special]
xunfei_per_call = 0.002211111111
kimi_per_token = 0.000000071071429
opencode_divisor = 6.0
ainaba_divisor = 40.0
freemodel_divisor = 68.2

[[model]]
name = "gpt-5.5"
input = 5.00
output = 30.00
cache_read = 0.50
cache_write = 5.00

[[model]]
name = "gpt-5.5"
tier_threshold = 272000
input = 10.00
output = 45.00
cache_read = 1.00
cache_write = 10.00
"#,
        );

        // Exactly at threshold (272K total input) → should use high tier (>= threshold)
        let mut record = make_record("codex", "openai", "gpt-5.5", 0, 0.0);
        record.input_tokens = 272_000;
        record.output_tokens = 5_000;
        record.cache_read_tokens = 0;
        record.cache_write_tokens = 0;
        record.total_tokens = 277_000;

        let cny = display_cost(&record);
        // High tier: input=$10/M, output=$45/M
        // usd = 272000*10/1M + 5000*45/1M = 2.72 + 0.225 = 2.945
        let expected = 2.945 * 6.82;
        assert!(
            (cny - expected).abs() < 0.001,
            "at threshold: expected {}, got {}",
            expected,
            cny
        );

        restore_pricing_env(prev_env);
    }

    #[test]
    fn flat_pricing_unchanged_with_tiered_config() {
        let _guard = pricing_test_guard();
        let prev_env = std::env::var("PRICING_CONFIG").ok();
        let _tmp = load_temp_config(
            br#"
usd_to_cny = 6.82
rate_date = "2026-05-20"

[special]
xunfei_per_call = 0.002211111111
kimi_per_token = 0.000000071071429
opencode_divisor = 6.0
ainaba_divisor = 40.0
freemodel_divisor = 68.2

[[model]]
name = "gpt-5.5"
input = 5.00
output = 30.00
cache_read = 0.50
cache_write = 5.00

[[model]]
name = "gpt-5.5"
tier_threshold = 272000
input = 10.00
output = 45.00
cache_read = 1.00
cache_write = 10.00

[[model]]
name = "claude-sonnet-4-6"
input = 3.00
output = 15.00
cache_read = 0.30
cache_write = 3.75
"#,
        );

        // Claude model (flat pricing, no tiers) should work exactly as before
        let mut record = make_record("claude-code", "anthropic", "claude-sonnet-4-6", 0, 0.0);
        record.input_tokens = 100_000;
        record.output_tokens = 10_000;
        record.cache_read_tokens = 50_000;
        record.cache_write_tokens = 0;
        record.total_tokens = 160_000;

        let cny = display_cost(&record);
        // Flat: input=$3/M, output=$15/M, cache_read=$0.30/M
        // usd = 100000*3/1M + 10000*15/1M + 50000*0.30/1M = 0.3 + 0.15 + 0.015 = 0.465
        let expected = 0.465 * 6.82;
        assert!(
            (cny - expected).abs() < 0.001,
            "flat pricing: expected {}, got {}",
            expected,
            cny
        );

        restore_pricing_env(prev_env);
    }

    #[test]
    fn commandcode_missing_pricing_config_returns_zero() {
        let _guard = pricing_test_guard();
        // Without cc: model prices in the config, commandcode records should return 0
        let mut record = make_record("pi", "commandcode", "deepseek/deepseek-v4-flash", 0, 0.0);
        // After normalization (done by load_all_sources), input is already
        // separated from cache. Simulate normalized values:
        record.input_tokens = 295; // new input after normalization
        record.output_tokens = 286;
        record.cache_read_tokens = 20864;
        record.cache_write_tokens = 0;
        record.total_tokens = 21445;
        let cost = display_cost(&record);
        // No cc:deepseek-v4-flash in config → returns 0 (fallback)
        assert_eq!(
            cost, 0.0,
            "commandcode without cc: model prices should return 0"
        );
    }

    #[test]
    fn commandcode_computes_from_cc_model_prices_with_divisor() {
        let _guard = pricing_test_guard();
        let prev_env = std::env::var("PRICING_CONFIG").ok();
        let _tmp = load_temp_config(
            br#"
usd_to_cny = 6.82
rate_date = "2026-05-20"

[special]
xunfei_per_call = 0.002211111111
kimi_per_token = 0.000000071071429
opencode_divisor = 6.0
ainaba_divisor = 40.0
freemodel_divisor = 68.2
commandcode_divisor = 10.0

[[model]]
name = "cc:deepseek-v4-flash"
input = 0.14
output = 0.28
cache_read = 0.01
cache_write = 0.0

[[model]]
name = "cc:kimi-k2.6"
input = 0.95
output = 4.00
cache_read = 0.16
cache_write = 0.0
"#,
        );

        // Test: deepseek-v4-flash from commandcode
        // After normalization: input=295 (new), cache_read=20864 (cached)
        // cc price: input=$0.14/M, output=$0.28/M, cache_read=$0.01/M
        // usd = 295*0.14/1M + 286*0.28/1M + 20864*0.01/1M
        //     = 0.0000413 + 0.00008008 + 0.00020864 = 0.00033002
        // cny = 0.00033002 * 6.82 / 10.0 = 0.000225074
        let mut record = make_record("pi", "commandcode", "deepseek-v4-flash", 0, 0.0);
        record.input_tokens = 295;
        record.output_tokens = 286;
        record.cache_read_tokens = 20864;
        record.cache_write_tokens = 0;
        record.total_tokens = 21445;

        let cny = display_cost(&record);
        let usd =
            295.0 * 0.14 / 1_000_000.0 + 286.0 * 0.28 / 1_000_000.0 + 20864.0 * 0.01 / 1_000_000.0;
        let expected = usd * 6.82 / 10.0;
        assert!(
            cny > 0.0,
            "commandcode record should compute non-zero cost, got {}",
            cny
        );
        assert!(
            (cny - expected).abs() < 1e-9,
            "commandcode cost: expected {}, got {} (usd={})",
            expected,
            cny,
            usd
        );

        // Test: model with provider prefix "moonshotai/Kimi-K2.6" → cc:kimi-k2.6
        let mut record2 = make_record("pi", "commandcode", "moonshotai/Kimi-K2.6", 0, 0.0);
        record2.input_tokens = 10_000;
        record2.output_tokens = 2_000;
        record2.cache_read_tokens = 5_000;
        record2.cache_write_tokens = 0;
        record2.total_tokens = 17_000;

        let cny2 = display_cost(&record2);
        let usd2 = 10_000.0 * 0.95 / 1_000_000.0
            + 2_000.0 * 4.00 / 1_000_000.0
            + 5_000.0 * 0.16 / 1_000_000.0;
        let expected2 = usd2 * 6.82 / 10.0;
        assert!(
            (cny2 - expected2).abs() < 1e-9,
            "commandcode kimi: expected {}, got {}",
            expected2,
            cny2
        );

        restore_pricing_env(prev_env);
    }

    // ─── Ainaba time-based segment tests ────────────────────────────────

    #[test]
    fn ainaba_segments_before_cutoff_uses_40x() {
        let _guard = pricing_test_guard();
        let prev_env = std::env::var("PRICING_CONFIG").ok();
        let _tmp = load_temp_config(
            br#"
usd_to_cny = 6.82
rate_date = "2026-05-20"

[special]
xunfei_per_call = 0.002211111111
kimi_per_token = 0.000000071071429
opencode_divisor = 6.0
ainaba_segments = [
    { before = "2025-05-25T22:30:00+08:00", divisor = 40.0 },
    { divisor = 25.0 },
]
freemodel_divisor = 68.2
commandcode_divisor = 10.0
"#,
        );

        // Record from May 25 10:00 UTC = May 25 18:00 CST, BEFORE the 22:30 CST cutoff
        let mut record = make_record("pi", "ainaba", "gpt-5.5", 0, 0.05);
        record.time = "2025-05-25T10:00:00Z".to_string();
        let cost = display_cost(&record);
        // cost=0.05 USD, usd_to_cny=6.82, divisor=40.0
        // cny = 0.05 * 6.82 / 40.0 = 0.008525
        let expected = 0.05 * 6.82 / 40.0;
        assert!(
            (cost - expected).abs() < 1e-9,
            "before cutoff should use 40x: expected {}, got {}",
            expected,
            cost
        );

        restore_pricing_env(prev_env);
    }

    #[test]
    fn ainaba_segments_after_cutoff_uses_25x() {
        let _guard = pricing_test_guard();
        let prev_env = std::env::var("PRICING_CONFIG").ok();
        let _tmp = load_temp_config(
            br#"
usd_to_cny = 6.82
rate_date = "2026-05-20"

[special]
xunfei_per_call = 0.002211111111
kimi_per_token = 0.000000071071429
opencode_divisor = 6.0
ainaba_segments = [
    { before = "2025-05-25T22:30:00+08:00", divisor = 40.0 },
    { divisor = 25.0 },
]
freemodel_divisor = 68.2
commandcode_divisor = 10.0
"#,
        );

        // Record from May 25 15:00 UTC = May 25 23:00 CST, AFTER the 22:30 CST cutoff
        let mut record = make_record("pi", "ainaba", "gpt-5.5", 0, 0.05);
        record.time = "2025-05-25T15:00:00Z".to_string();
        let cost = display_cost(&record);
        // cost=0.05 USD, usd_to_cny=6.82, divisor=25.0
        // cny = 0.05 * 6.82 / 25.0 = 0.01364
        let expected = 0.05 * 6.82 / 25.0;
        assert!(
            (cost - expected).abs() < 1e-9,
            "after cutoff should use 25x: expected {}, got {}",
            expected,
            cost
        );

        restore_pricing_env(prev_env);
    }

    #[test]
    fn ainaba_segments_exactly_at_cutoff_uses_25x() {
        let _guard = pricing_test_guard();
        let prev_env = std::env::var("PRICING_CONFIG").ok();
        let _tmp = load_temp_config(
            br#"
usd_to_cny = 6.82
rate_date = "2026-05-20"

[special]
xunfei_per_call = 0.002211111111
kimi_per_token = 0.000000071071429
opencode_divisor = 6.0
ainaba_segments = [
    { before = "2025-05-25T22:30:00+08:00", divisor = 40.0 },
    { divisor = 25.0 },
]
freemodel_divisor = 68.2
commandcode_divisor = 10.0
"#,
        );

        // Exactly at cutoff: 2025-05-25T14:30:00Z = 2025-05-25T22:30:00+08:00
        let mut record = make_record("pi", "ainaba", "gpt-5.5", 0, 0.05);
        record.time = "2025-05-25T14:30:00Z".to_string();
        let cost = display_cost(&record);
        // Not before (record.time < cutoff is false), so falls through to catch-all: 25x
        let expected = 0.05 * 6.82 / 25.0;
        assert!(
            (cost - expected).abs() < 1e-9,
            "exactly at cutoff should use 25x (not before): expected {}, got {}",
            expected,
            cost
        );

        restore_pricing_env(prev_env);
    }

    #[test]
    fn ainaba_derived_cost_segments_before_cutoff_uses_40x() {
        let _guard = pricing_test_guard();
        let prev_env = std::env::var("PRICING_CONFIG").ok();
        let _tmp = load_temp_config(
            br#"
usd_to_cny = 6.82
rate_date = "2026-05-20"

[special]
xunfei_per_call = 0.002211111111
kimi_per_token = 0.000000071071429
opencode_divisor = 6.0
ainaba_segments = [
    { before = "2025-05-25T22:30:00+08:00", divisor = 40.0 },
    { divisor = 25.0 },
]
freemodel_divisor = 68.2
commandcode_divisor = 10.0

[[model]]
name = "gpt-5.5"
input = 5.00
output = 30.00
cache_read = 0.50
cache_write = 5.00
"#,
        );

        // Derived-cost record (codex, claude-code) from before cutoff
        let mut record = make_record("codex", "ainaba", "gpt-5.5", 0, 0.0);
        record.time = "2025-05-25T10:00:00Z".to_string();
        record.input_tokens = 100_000;
        record.output_tokens = 10_000;
        record.cache_read_tokens = 0;
        record.cache_write_tokens = 0;
        record.total_tokens = 110_000;

        let cost = display_cost(&record);
        // usd = 100000*5/1M + 10000*30/1M = 0.5 + 0.3 = 0.8
        // cny = 0.8 * 6.82 / 40.0 = 0.1364
        let expected = 0.8 * 6.82 / 40.0;
        assert!(
            (cost - expected).abs() < 1e-9,
            "derived cost before cutoff should use 40x: expected {}, got {}",
            expected,
            cost
        );

        restore_pricing_env(prev_env);
    }

    #[test]
    fn ainaba_fallback_to_legacy_divisor_when_no_segments() {
        let _guard = pricing_test_guard();
        let prev_env = std::env::var("PRICING_CONFIG").ok();
        let _tmp = load_temp_config(
            br#"
usd_to_cny = 6.82
rate_date = "2026-05-20"

[special]
xunfei_per_call = 0.002211111111
kimi_per_token = 0.000000071071429
opencode_divisor = 6.0
ainaba_divisor = 40.0
freemodel_divisor = 68.2
commandcode_divisor = 10.0
"#,
        );

        // Without ainaba_segments, should fall back to ainaba_divisor
        let mut record = make_record("pi", "ainaba", "gpt-5.5", 0, 0.05);
        record.time = "2025-05-25T15:00:00Z".to_string();
        let cost = display_cost(&record);
        let expected = 0.05 * 6.82 / 40.0;
        assert!(
            (cost - expected).abs() < 1e-9,
            "fallback should use legacy ainaba_divisor: expected {}, got {}",
            expected,
            cost
        );

        restore_pricing_env(prev_env);
    }
}
