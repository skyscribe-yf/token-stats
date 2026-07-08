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
use chrono::{Datelike, FixedOffset, NaiveDate, Timelike};
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
    #[serde(default = "default_fenno_divisor")]
    pub fenno_divisor: f64,
    /// Meituan LongCat per-token cost in CNY (resource pack billing).
    /// Only non-cached input + output tokens are billed; cache hits are free.
    /// Default: 10 CNY / 50,000,000 tokens = 0.0000002
    #[serde(default = "default_meituan_per_token")]
    pub meituan_per_token: f64,
    /// Ollama Cloud empirical per-token cost in CNY.
    /// Derived from: $20/mo Pro × 6.82 / (weekly_quota × 52/12)
    /// = ¥136.40 / 1,266,666,667 ≈ 0.0000001077
    #[serde(default)]
    pub ollama_cloud_empirical_per_token: f64,
    /// Ollama Cloud weekly quota in tokens (empirical).
    /// Derived from: 38M tokens / 13% ≈ 292,307,692
    #[serde(default)]
    pub ollama_cloud_empirical_weekly_quota: i64,
    /// Off-peak (波谷) pricing configuration for xunfei/xunfei-ex.
    /// If `None`, no off-peak discount is applied (always full price).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub xunfei_off_peak: Option<XunfeiOffPeakConfig>,
}

fn default_fenno_divisor() -> f64 {
    150.0 * 6.82 / 10.0
}

fn default_meituan_per_token() -> f64 {
    10.0 / 50_000_000.0 // 10 CNY / 50M tokens
}

/// Xunfei off-peak (波谷) pricing configuration.
///
/// Since 2026-06-18, Xunfei introduced time-differentiated billing (分时段差异化计量)
/// with an off-peak discount coefficient. The official rules are:
///
/// | 时段         | 条件                          | 计费系数 |
/// |-------------|-------------------------------|---------|
/// | 高峰 (Peak) | 工作日 08:00–22:00 (UTC+8)    | 1.0     |
/// | 波谷 (Off)  | 夜间 22:00–次日08:00          | 0.8     |
/// | 波谷 (Off)  | 周末全天 (Sat/Sun)             | 0.8     |
/// | 波谷 (Off)  | 法定节假日全天                   | 0.8     |
///
/// The coefficient also affects rate-limit quotas (流控次数): at coefficient 0.8,
/// a 6000-call limit in 5 hours becomes effectively 7500 calls (6000 / 0.8).
/// All rate-limit dimensions follow the same conversion logic.
///
/// Records before `effective_from` always use coefficient 1.0 (no discount).
/// Chinese holidays must be updated annually when the State Council
/// announces the official schedule (typically in November/December).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XunfeiOffPeakConfig {
    /// Off-peak pricing coefficient (e.g. 0.8 = 80% of normal per-call price).
    /// Peak hours always use coefficient 1.0 (full price).
    pub coefficient: f64,

    /// Effective start date in "YYYY-MM-DD" format (China Standard Time, UTC+8).
    /// Records before this date always use coefficient 1.0 (no off-peak discount).
    /// Per the official announcement: "2026-06-18".
    pub effective_from: String,

    /// Peak hour range in UTC+8: [start_hour, end_hour), 24-hour format.
    /// Example: [8, 22] means peak is 08:00–22:00 (UTC+8).
    /// Hours outside this range on non-holiday weekdays are off-peak.
    pub peak_hours: [u8; 2],

    /// Chinese public holidays in "YYYY-MM-DD" format (China Standard Time).
    /// These dates are treated as off-peak regardless of day of week.
    /// Must be updated annually when the State Council announces the schedule
    /// (typically in November/December for the following year).
    #[serde(default)]
    pub holidays: Vec<String>,
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
                // 99 CNY subscription, dashboard 672.26M tokens ≈ 84% usage
                // effective per-token = 99 * 0.84 / 672_260_000 ≈ 0.0000001237
                xiaomi_mimo_tp_per_token: 0.0000001237,
                opencode_divisor: 6.0,
                ainaba_divisor: 1.0,
                ainaba_segments: Vec::new(),
                freemodel_divisor: 68.2,
                commandcode_divisor: 1.0,
                fenno_divisor: default_fenno_divisor(),
                meituan_per_token: default_meituan_per_token(),
                ollama_cloud_empirical_per_token: 0.0000001077,
                ollama_cloud_empirical_weekly_quota: 292307692,
                xunfei_off_peak: None,
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

    // Kimi family — kimi-k2.7, kimi-k2.6, kimi-k2.5, kimi-for-coding, etc.
    if model_lower.contains("kimi-k2.7") {
        return state.model_map.get("kimi-k2.7");
    }
    if model_lower.contains("kimi-k2.6") {
        return state.model_map.get("kimi-k2.6");
    }
    if model_lower.contains("kimi-k2.5") {
        return state.model_map.get("kimi-k2.5");
    }
    if model_lower.contains("kimi") || provider == "kimi" {
        // Fallback: try kimi-k2.7 as the default kimi pricing
        return state.model_map.get("kimi-k2.7");
    }

    // DeepSeek family
    if model_lower.contains("deepseek-v4-pro") {
        return state.model_map.get("deepseek-v4-pro");
    }
    if model_lower.contains("deepseek-v4-flash") {
        return state.model_map.get("deepseek-v4-flash");
    }
    if provider == "deepseek" {
        return state.model_map.get("deepseek-v4-pro");
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

// ── Off-peak (波谷) determination for Xunfei ──────────────────────────────────

/// Determine whether a xunfei/xunfei-ex record falls in the off-peak (波谷) period.
///
/// All times are interpreted in China Standard Time (UTC+8) because the
/// off-peak policy is defined in terms of Chinese business hours.
///
/// Decision logic (checked in order, first match wins):
///
/// 1. **Before effective date** → NOT off-peak (coefficient 1.0)
///    The policy started on 2026-06-18. Earlier records pay full price.
///
/// 2. **Chinese public holiday** → off-peak (coefficient 0.8)
///    Holidays are off-peak all day regardless of day-of-week or hour.
///    E.g. 端午节 (Dragon Boat) on a Friday at 10:00 → off-peak.
///    Holiday dates come from `config.holidays` and must be updated annually.
///
/// 3. **Weekend (Saturday or Sunday)** → off-peak (coefficient 0.8)
///    Weekends are off-peak all day (00:00–24:00).
///
/// 4. **Weekday night** (22:00–08:00) → off-peak (coefficient 0.8)
///    Night hours on a non-holiday weekday are off-peak.
///    Specifically: hour < peak_start (8) or hour >= peak_end (22).
///
/// 5. **Weekday peak hours** (08:00–22:00) → NOT off-peak (coefficient 1.0)
///    Regular business hours on a working day → full price.
///
/// Note: The record's `time` field is in RFC3339 UTC. We convert to UTC+8
/// before checking any time-based conditions.
fn is_xunfei_off_peak(record: &TokenRecord, config: &XunfeiOffPeakConfig) -> bool {
    // Step 1: Parse record time (RFC3339 UTC) and convert to China Standard Time (UTC+8).
    // All peak/off-peak decisions are based on China time, not the user's local timezone.
    let record_dt = match chrono::DateTime::parse_from_rfc3339(&record.time) {
        Ok(dt) => dt.with_timezone(&FixedOffset::east_opt(8 * 3600).unwrap()),
        Err(_) => {
            tracing::warn!(
                "Failed to parse xunfei record time '{}', assuming peak",
                record.time
            );
            return false; // Cannot determine → assume peak (safe default: no discount)
        }
    };

    // Step 2: Check if the record is before the off-peak policy effective date.
    // The policy started on 2026-06-18 (UTC+8). Records before this date
    // always use coefficient 1.0 (no off-peak discount).
    let effective_date = match NaiveDate::parse_from_str(&config.effective_from, "%Y-%m-%d") {
        Ok(d) => d,
        Err(_) => {
            tracing::warn!(
                "Invalid xunfei_off_peak effective_from '{}', assuming peak",
                config.effective_from
            );
            return false; // Bad config → assume peak (safe default)
        }
    };
    if record_dt.date_naive() < effective_date {
        return false; // Before policy start → peak (coefficient 1.0)
    }

    // Step 3: Check if the record's date (UTC+8) is a Chinese public holiday.
    // Holidays are off-peak regardless of day of week or hour.
    // E.g. 端午节 on a Wednesday at 14:00 is still off-peak (波谷).
    let date_str = record_dt.format("%Y-%m-%d").to_string();
    if config.holidays.contains(&date_str) {
        return true; // 法定节假日全天 → 波谷
    }

    // Step 4: Check if the record falls on a weekend (Saturday or Sunday).
    // Weekends are off-peak all day (00:00–24:00 UTC+8).
    let weekday = record_dt.weekday();
    if weekday == chrono::Weekday::Sat || weekday == chrono::Weekday::Sun {
        return true; // 周末全天 → 波谷
    }

    // Step 5: For weekdays (that are not holidays), check the hour.
    // Peak hours: [peak_start, peak_end) = [08:00, 22:00) in UTC+8.
    // Off-peak hours: [00:00, peak_start) ∪ [peak_end, 24:00) = night time.
    let hour = record_dt.hour() as u8;
    let peak_start = config.peak_hours[0]; // e.g. 8 (08:00)
    let peak_end = config.peak_hours[1]; // e.g. 22 (22:00)
    if hour < peak_start || hour >= peak_end {
        return true; // 工作日夜间 (22:00–次日08:00) → 波谷
    }

    // Step 6: Weekday during peak hours (08:00–22:00) → NOT off-peak.
    // Coefficient 1.0 (full price) applies.
    false
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

    // 1. 讯飞 (xunfei / xunfei-ex): flat per-call rate in CNY
    //    With off-peak (波谷) discount since 2026-06-18:
    //    - Peak (工作日 08:00–22:00, UTC+8): coefficient 1.0 → full per-call price
    //    - Off-peak (夜间/周末/节假日): coefficient 0.8 → 80% of per-call price
    //    - Before 2026-06-18: always full price (no off-peak policy)
    //
    //    Calculation: base_per_call × off_peak_coefficient
    //    Example: 199元/90000次 × 0.8 = 0.001769元/次 (off-peak)
    if record.provider == "xunfei" || record.provider == "xunfei-ex" {
        let base = cfg.special.xunfei_per_call;
        if let Some(ref off_peak) = cfg.special.xunfei_off_peak {
            if is_xunfei_off_peak(record, off_peak) {
                return base * off_peak.coefficient; // 波谷折扣价
            }
        }
        return base; // 高峰原价
    }

    // 1b. 讯飞API接口 (xunfei_api): cost is already in CNY from pi calculations.
    //     If stored cost is available, return it as-is. Otherwise fall back to
    //     flat per-call rate.
    if record.provider == "xunfei_api" {
        if record.cost > 0.0 {
            return record.cost;
        }
        return cfg.special.xunfei_per_call;
    }

    // 2. Kimi provider with zero stored cost: per-token estimate in CNY.
    //    Provider aliases such as "kimi-code" are merged to the canonical
    //    "kimi" vendor before pricing, so all zero-cost kimi records share
    //    the same subscription estimate regardless of source.
    if record.provider == "kimi" && record.cost == 0.0 {
        return record.total_tokens as f64 * cfg.special.kimi_per_token;
    }

    // 2b. Xiaomi MiMo provider with zero stored cost: per-token estimate in CNY
    //     Similar to Kimi subscription model: 99 元 / 110 亿 Token (platform tokenization)
    //     Covers both "xiaomi-mimo" (pi direct) and "xiaomi-mimo-tp" (token plan).
    if (record.provider == "xiaomi-mimo" || record.provider == "xiaomi-mimo-tp")
        && record.cost == 0.0
    {
        return record.total_tokens as f64 * cfg.special.xiaomi_mimo_tp_per_token;
    }

    // 2c. Meituan LongCat: resource-pack billing, only non-cached input + output count.
    //     Cache hits (cache_read) are free — not deducted from the resource pack.
    //     Formula: (input_tokens + output_tokens) × meituan_per_token (CNY)
    if record.provider == "meituan" {
        return (record.input_tokens + record.output_tokens) as f64 * cfg.special.meituan_per_token;
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

    // 4b. Ollama Cloud (subscription): empirical per-token estimate in CNY.
    //     Applies to both "ollama" and vendor-merged "ollama-cloud" records.
    //     Uses the same per-token rate as the quota card cost estimation.
    if record.provider == "ollama" && record.cost == 0.0 {
        return record.total_tokens as f64 * cfg.special.ollama_cloud_empirical_per_token;
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

        // Pi's stored Ainaba cost can lag the latest pricing table for long
        // contexts, so recompute from token counts when a known model exists.
        if record.provider == "ainaba" || effective_provider == "ainaiba" {
            if let Some(mp) = resolve_model_price(&state, &record.model, &record.provider) {
                let usd = mp.compute_usd(
                    record.input_tokens,
                    record.output_tokens,
                    record.cache_read_tokens,
                    record.cache_write_tokens,
                );
                return usd * cfg.usd_to_cny / get_ainaba_divisor(&cfg.special, &record.time);
            }
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

        // 4b2. kimi-coding Pi provider: subscription model, same as kimi-code.
        //     The stored cost is the API list price (USD), not the actual
        //     subscription cost. Use kimi_per_token estimate instead.
        //     (original_provider preserved by vendor merge from "kimi-coding" → "kimi")
        if effective_provider == "kimi-coding" {
            return record.total_tokens as f64 * cfg.special.kimi_per_token;
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

        // Fenno subscription discount: 10 CNY buys 150 USD face value.
        // After USD→CNY conversion, divide by the effective face-value ratio.
        if effective_provider == "fenno" {
            cny /= cfg.special.fenno_divisor;
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

    // 6. Derived sources without original cost: codex, claude-code, kimi-code, etc.
    //    Compute from per-model token rates. pricing.toml model prices are in USD.
    if record.source == "codex" || record.source == "claude-code" || record.source == "kimi-code" {
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
            // OpenCode Go plan discount: listed API cost / opencode_divisor
            // kimi-code records with provider="opencode-go" go through the
            // same OpenCode Go subscription as pi records with opencode-go.
            if record.provider == "opencode-go" {
                cny /= cfg.special.opencode_divisor;
            }
            if record.provider == "fenno" {
                cny /= cfg.special.fenno_divisor;
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
            ttft_ms: None,
            tps: None,
        }
    }

    #[test]
    fn project_pricing_toml_keeps_ainaba_segments_and_models() {
        let cfg: PricingConfig = toml::from_str(include_str!("../pricing.toml"))
            .expect("backend/pricing.toml should parse as PricingConfig");

        assert_eq!(cfg.special.commandcode_divisor, 10.0);
        assert_eq!(cfg.special.ainaba_segments.len(), 2);
        assert_eq!(cfg.special.ainaba_segments[1].divisor, 20.0);
        assert_eq!(
            cfg.special
                .xunfei_off_peak
                .as_ref()
                .map(|cfg| (&cfg.effective_from, cfg.coefficient)),
            Some((&"2026-06-18".to_string(), 0.8))
        );
        assert!(cfg.model.iter().any(|m| m.name == "gpt-5.4"));
        assert!(cfg.model.iter().any(|m| m.name == "gpt-5.5"));
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
    fn kimi_coding_subscription_uses_per_token_estimate() {
        let _guard = pricing_test_guard();
        // Records from kimi-coding provider (subscription) with cost>0 should
        // use kimi_per_token estimate, NOT the stored USD cost.
        // This matches kimi-code behavior (same subscription model).
        let mut record = make_record("pi", "kimi", "kimi-for-coding", 1_000_000, 0.05);
        record.original_provider = Some("kimi-coding".to_string());
        let cost = display_cost(&record);
        let expected = 1_000_000.0 * PricingConfig::default().special.kimi_per_token;
        // The subscription estimate should be significantly lower than USD*6.82
        let usd_cny = 0.05 * PricingConfig::default().usd_to_cny; // 0.341
        assert!(
            expected < usd_cny,
            "kimi_per_token estimate ({}) should be < USD*CNY ({})",
            expected,
            usd_cny
        );
        assert!(
            (cost - expected).abs() < 1e-9,
            "kimi-coding should use per-token estimate, expected {}, got {}",
            expected,
            cost
        );
    }

    #[test]
    fn kimi_with_stored_cost_uses_stored_cost() {
        let _guard = pricing_test_guard();
        // Records with provider="kimi" (no original_provider) and cost>0
        // should use the stored cost path (raw kimi API, not subscription)
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
    fn xunfei_ex_uses_same_per_call_rate() {
        let _guard = pricing_test_guard();
        // xunfei-ex provider should use the same flat per-call rate as xunfei
        let record = make_record("pi", "xunfei-ex", "astron-code-latest", 1_000_000, 0.0);
        let cost = display_cost(&record);
        let expected = PricingConfig::default().special.xunfei_per_call;
        assert!(
            (cost - expected).abs() < 1e-9,
            "xunfei-ex should use per-call rate, expected {}, got {}",
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
    fn fenno_stored_cost_applies_subscription_divisor() {
        let _guard = pricing_test_guard();
        // Fenno list price is stored in USD, but the actual subscription spend is
        // about 10 CNY for 150 USD face value.
        let record = make_record("pi", "fenno", "gpt-5.4", 1_000_000, 0.05);
        let cost = display_cost(&record);
        let fenno_divisor = 150.0 * PricingConfig::default().usd_to_cny / 10.0;
        let expected = 0.05 * PricingConfig::default().usd_to_cny / fenno_divisor;
        assert!(
            (cost - expected).abs() < 1e-9,
            "fenno cost should use subscription divisor, expected {}, got {}",
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
    fn meituan_per_token_only_bills_input_and_output() {
        let _guard = pricing_test_guard();
        // Meituan LongCat: resource-pack billing, only input + output tokens count.
        // Cache hits (cache_read) and cache writes are FREE.
        // Default rate: 10 CNY / 50M tokens = 0.0000002 CNY/token
        let mut record = make_record("pi", "meituan", "LongCat-2.0", 0, 0.0);
        record.input_tokens = 50_000_000;
        record.output_tokens = 0;
        record.cache_read_tokens = 0;
        record.cache_write_tokens = 0;
        record.total_tokens = 50_000_000;
        let cost = display_cost(&record);
        // 50M input tokens × 0.0000002 = 10.0 CNY
        assert!((cost - 10.0).abs() < 0.01, "expected 10.0, got {}", cost);

        // Cache reads should NOT add to cost
        record.cache_read_tokens = 100_000_000;
        record.total_tokens = 150_000_000;
        let cost_with_cache = display_cost(&record);
        assert!((cost_with_cache - 10.0).abs() < 0.01, "cache should be free, expected 10.0, got {}", cost_with_cache);

        // Input + output both count
        record.input_tokens = 25_000_000;
        record.output_tokens = 25_000_000;
        record.cache_read_tokens = 0;
        record.total_tokens = 50_000_000;
        let cost_mixed = display_cost(&record);
        assert!((cost_mixed - 10.0).abs() < 0.01, "input+output should = 10.0, got {}", cost_mixed);
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

    #[test]
    fn kimi_code_derived_cost_openai_model() {
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
name = "gpt-5.4"
input = 2.50
output = 15.00
cache_read = 0.25
cache_write = 2.50
"#,
        );

        // kimi-code record with gpt-5.4 from ainaiba provider, cost=0
        let mut record = make_record("kimi-code", "ainaba", "gpt-5.4", 0, 0.0);
        record.time = "2025-05-25T15:00:00Z".to_string(); // after cutoff → 25x
        record.input_tokens = 100_000;
        record.output_tokens = 10_000;
        record.cache_read_tokens = 0;
        record.cache_write_tokens = 0;
        record.total_tokens = 110_000;

        let cost = display_cost(&record);
        // usd = 100000*2.5/1M + 10000*15/1M = 0.25 + 0.15 = 0.40
        // cny = 0.40 * 6.82 / 25.0 = 0.10912
        let expected = 0.40 * 6.82 / 25.0;
        assert!(
            cost > 0.0,
            "kimi-code gpt-5.4 should have non-zero cost, got {}",
            cost
        );
        assert!(
            (cost - expected).abs() < 1e-9,
            "kimi-code gpt-5.4 cost: expected {}, got {}",
            expected,
            cost
        );

        restore_pricing_env(prev_env);
    }

    #[test]
    fn kimi_code_kimi_provider_uses_same_per_token_estimate() {
        let _guard = pricing_test_guard();
        // kimi-code records merged to provider="kimi" should follow the same
        // subscription estimate as other kimi zero-cost records.
        let record = make_record("kimi-code", "kimi", "kimi-k2.6", 170_000, 0.0);
        let cost = display_cost(&record);
        let expected = 170_000.0 * PricingConfig::default().special.kimi_per_token;
        assert!(
            cost > 0.0,
            "kimi-code/kimi-k2.6 should have non-zero cost, got {}",
            cost
        );
        assert!(
            (cost - expected).abs() < 1e-9,
            "kimi-code/kimi-k2.6 cost: expected {}, got {}",
            expected,
            cost
        );

        // kimi-code/kimi-for-coding should resolve to the same kimi estimate.
        let record2 = make_record("kimi-code", "kimi", "kimi-for-coding", 170_000, 0.0);
        let cost2 = display_cost(&record2);
        assert!(
            cost2 > 0.0,
            "kimi-code/kimi-for-coding should have non-zero cost, got {}",
            cost2
        );
        assert!(
            (cost2 - expected).abs() < 1e-9,
            "kimi-code/kimi-for-coding cost: expected {}, got {}",
            expected,
            cost2
        );
    }

    #[test]
    fn ainaba_pi_stored_cost_recomputes_high_tier_with_cached_tokens() {
        let _guard = pricing_test_guard();
        let prev_env = std::env::var("PRICING_CONFIG").ok();
        let _tmp = load_temp_config(
            br#"
usd_to_cny = 6.82
rate_date = "2026-06-24"

[special]
xunfei_per_call = 0.002211111111
kimi_per_token = 0.000000071071429
opencode_divisor = 6.0
ainaba_segments = [{ divisor = 20.0 }]
freemodel_divisor = 68.2
commandcode_divisor = 10.0

[[model]]
name = "gpt-5.4"
input = 2.50
output = 15.00
cache_read = 0.25
cache_write = 2.50

[[model]]
name = "gpt-5.4"
tier_threshold = 272000
input = 5.00
output = 22.50
cache_read = 0.50
cache_write = 5.00
"#,
        );

        let mut record = make_record("pi", "ainaba", "gpt-5.4", 0, 0.0);
        record.time = "2026-06-23T00:00:00Z".to_string();
        record.input_tokens = 150_000;
        record.output_tokens = 12_000;
        record.cache_read_tokens = 100_000;
        record.cache_write_tokens = 30_000;
        record.total_tokens = 292_000;
        // Simulate the tracker writing a base-tier USD cost even though the
        // total input crosses the tier-2 threshold.
        record.cost = 0.655;

        let cost = display_cost(&record);
        let expected_usd = 1.22;
        let expected = expected_usd * 6.82 / 20.0;
        assert!(
            (cost - expected).abs() < 1e-9,
            "ainaba high-tier pi cost: expected {}, got {}",
            expected,
            cost
        );

        restore_pricing_env(prev_env);
    }

    #[test]
    fn ainaba_pi_stored_cost_keeps_base_tier_below_threshold() {
        let _guard = pricing_test_guard();
        let prev_env = std::env::var("PRICING_CONFIG").ok();
        let _tmp = load_temp_config(
            br#"
usd_to_cny = 6.82
rate_date = "2026-06-24"

[special]
xunfei_per_call = 0.002211111111
kimi_per_token = 0.000000071071429
opencode_divisor = 6.0
ainaba_segments = [{ divisor = 20.0 }]
freemodel_divisor = 68.2
commandcode_divisor = 10.0

[[model]]
name = "gpt-5.4"
input = 2.50
output = 15.00
cache_read = 0.25
cache_write = 2.50

[[model]]
name = "gpt-5.4"
tier_threshold = 272000
input = 5.00
output = 22.50
cache_read = 0.50
cache_write = 5.00
"#,
        );

        let mut record = make_record("pi", "ainaba", "gpt-5.4", 0, 0.0);
        record.time = "2026-06-24T00:00:00Z".to_string();
        record.input_tokens = 100_000;
        record.output_tokens = 10_000;
        record.cache_read_tokens = 50_000;
        record.cache_write_tokens = 10_000;
        record.total_tokens = 170_000;
        record.cost = 0.4375;

        let cost = display_cost(&record);
        let expected = 0.4375 * 6.82 / 20.0;
        assert!(
            (cost - expected).abs() < 1e-9,
            "ainaba base-tier pi cost: expected {}, got {}",
            expected,
            cost
        );

        restore_pricing_env(prev_env);
    }

    // ─── Xunfei off-peak (波谷) pricing tests ─────────────────────────────

    /// Helper: build a temp config with xunfei off-peak enabled.
    fn xunfei_off_peak_config() -> Vec<u8> {
        "usd_to_cny = 6.82
rate_date = \"2026-05-20\"

[special]
xunfei_per_call = 0.002211111111
kimi_per_token = 0.000000071071429
opencode_divisor = 6.0
ainaba_divisor = 40.0
freemodel_divisor = 68.2

[special.xunfei_off_peak]
coefficient = 0.8
effective_from = \"2026-06-18\"
peak_hours = [8, 22]
holidays = [
    \"2026-06-19\",  # Dragon Boat Festival (Friday)
    \"2026-10-01\",  # National Day (Thursday)
    \"2026-10-02\",  # National Day
    \"2026-10-03\",  # National Day
]
"
        .as_bytes()
        .to_vec()
    }

    #[test]
    fn xunfei_off_peak_before_effective_date_uses_full_price() {
        // Records before 2026-06-18 should always use coefficient 1.0 (full price),
        // regardless of time of day or day of week.
        let _guard = pricing_test_guard();
        let prev_env = std::env::var("PRICING_CONFIG").ok();
        let _tmp = load_temp_config(&xunfei_off_peak_config());

        // 2026-06-17 23:00 CST (night before effective date) → still full price
        let mut record = make_record("pi", "xunfei", "astron-code-latest", 1_000_000, 0.0);
        record.time = "2026-06-17T15:00:00Z".to_string(); // 23:00 CST
        let cost = display_cost(&record);
        let expected = PricingConfig::default().special.xunfei_per_call; // full price
        assert!(
            (cost - expected).abs() < 1e-9,
            "before effective date should use full price: expected {}, got {}",
            expected,
            cost
        );

        restore_pricing_env(prev_env);
    }

    #[test]
    fn xunfei_off_peak_weekday_night_uses_discount() {
        // Weekday night (22:00–08:00 CST) → off-peak, coefficient 0.8
        let _guard = pricing_test_guard();
        let prev_env = std::env::var("PRICING_CONFIG").ok();
        let _tmp = load_temp_config(&xunfei_off_peak_config());

        // 2026-06-18 is a Thursday (weekday). 23:00 CST = 15:00 UTC
        let mut record = make_record("pi", "xunfei", "astron-code-latest", 1_000_000, 0.0);
        record.time = "2026-06-18T15:00:00Z".to_string(); // 23:00 CST, off-peak
        let cost = display_cost(&record);
        let expected = PricingConfig::default().special.xunfei_per_call * 0.8;
        assert!(
            (cost - expected).abs() < 1e-9,
            "weekday night should use 0.8 coefficient: expected {}, got {}",
            expected,
            cost
        );

        restore_pricing_env(prev_env);
    }

    #[test]
    fn xunfei_off_peak_weekday_early_morning_uses_discount() {
        // Weekday early morning (before 08:00 CST) → off-peak, coefficient 0.8
        let _guard = pricing_test_guard();
        let prev_env = std::env::var("PRICING_CONFIG").ok();
        let _tmp = load_temp_config(&xunfei_off_peak_config());

        // 2026-06-18 07:00 CST = 2026-06-17T23:00:00Z
        let mut record = make_record("pi", "xunfei", "astron-code-latest", 1_000_000, 0.0);
        record.time = "2026-06-17T23:00:00Z".to_string(); // 07:00 CST on Jun 18, off-peak
        let cost = display_cost(&record);
        let expected = PricingConfig::default().special.xunfei_per_call * 0.8;
        assert!(
            (cost - expected).abs() < 1e-9,
            "weekday early morning should use 0.8 coefficient: expected {}, got {}",
            expected,
            cost
        );

        restore_pricing_env(prev_env);
    }

    #[test]
    fn xunfei_peak_weekday_daytime_uses_full_price() {
        // Weekday daytime (08:00–22:00 CST) → peak, coefficient 1.0
        let _guard = pricing_test_guard();
        let prev_env = std::env::var("PRICING_CONFIG").ok();
        let _tmp = load_temp_config(&xunfei_off_peak_config());

        // 2026-06-18 10:00 CST = 02:00 UTC
        let mut record = make_record("pi", "xunfei", "astron-code-latest", 1_000_000, 0.0);
        record.time = "2026-06-18T02:00:00Z".to_string(); // 10:00 CST, peak
        let cost = display_cost(&record);
        let expected = PricingConfig::default().special.xunfei_per_call; // full price
        assert!(
            (cost - expected).abs() < 1e-9,
            "weekday daytime should use full price: expected {}, got {}",
            expected,
            cost
        );

        restore_pricing_env(prev_env);
    }

    #[test]
    fn xunfei_off_peak_weekend_uses_discount() {
        // Weekend (Saturday/Sunday) all day → off-peak, coefficient 0.8
        let _guard = pricing_test_guard();
        let prev_env = std::env::var("PRICING_CONFIG").ok();
        let _tmp = load_temp_config(&xunfei_off_peak_config());

        // 2026-06-20 is a Saturday. 10:00 CST = 02:00 UTC
        let mut record = make_record("pi", "xunfei", "astron-code-latest", 1_000_000, 0.0);
        record.time = "2026-06-20T02:00:00Z".to_string(); // 10:00 CST Saturday, off-peak
        let cost = display_cost(&record);
        let expected = PricingConfig::default().special.xunfei_per_call * 0.8;
        assert!(
            (cost - expected).abs() < 1e-9,
            "weekend should use 0.8 coefficient: expected {}, got {}",
            expected,
            cost
        );

        restore_pricing_env(prev_env);
    }

    #[test]
    fn xunfei_off_peak_holiday_uses_discount() {
        // Chinese public holiday → off-peak all day, coefficient 0.8
        // even if it falls on a weekday during business hours.
        let _guard = pricing_test_guard();
        let prev_env = std::env::var("PRICING_CONFIG").ok();
        let _tmp = load_temp_config(&xunfei_off_peak_config());

        // 2026-06-19 is a Friday (端午节, Dragon Boat Festival) at 10:00 CST
        let mut record = make_record("pi", "xunfei", "astron-code-latest", 1_000_000, 0.0);
        record.time = "2026-06-19T02:00:00Z".to_string(); // 10:00 CST Friday holiday, off-peak
        let cost = display_cost(&record);
        let expected = PricingConfig::default().special.xunfei_per_call * 0.8;
        assert!(
            (cost - expected).abs() < 1e-9,
            "holiday on weekday should use 0.8 coefficient: expected {}, got {}",
            expected,
            cost
        );

        restore_pricing_env(prev_env);
    }

    #[test]
    fn xunfei_off_peak_national_day_holiday_uses_discount() {
        // National Day holiday (国庆节) → off-peak all day
        let _guard = pricing_test_guard();
        let prev_env = std::env::var("PRICING_CONFIG").ok();
        let _tmp = load_temp_config(&xunfei_off_peak_config());

        // 2026-10-01 is a Thursday (国庆节) at 14:00 CST
        let mut record = make_record("pi", "xunfei", "astron-code-latest", 1_000_000, 0.0);
        record.time = "2026-10-01T06:00:00Z".to_string(); // 14:00 CST Thursday holiday, off-peak
        let cost = display_cost(&record);
        let expected = PricingConfig::default().special.xunfei_per_call * 0.8;
        assert!(
            (cost - expected).abs() < 1e-9,
            "national day holiday should use 0.8 coefficient: expected {}, got {}",
            expected,
            cost
        );

        restore_pricing_env(prev_env);
    }

    #[test]
    fn xunfei_ex_off_peak_same_as_xunfei() {
        // xunfei-ex provider should use the same off-peak logic as xunfei
        let _guard = pricing_test_guard();
        let prev_env = std::env::var("PRICING_CONFIG").ok();
        let _tmp = load_temp_config(&xunfei_off_peak_config());

        // 2026-06-18 23:00 CST (off-peak) with xunfei-ex provider
        let mut record = make_record("pi", "xunfei-ex", "astron-code-latest", 1_000_000, 0.0);
        record.time = "2026-06-18T15:00:00Z".to_string(); // 23:00 CST, off-peak
        let cost = display_cost(&record);
        let expected = PricingConfig::default().special.xunfei_per_call * 0.8;
        assert!(
            (cost - expected).abs() < 1e-9,
            "xunfei-ex off-peak should use 0.8 coefficient: expected {}, got {}",
            expected,
            cost
        );

        restore_pricing_env(prev_env);
    }

    #[test]
    fn xunfei_no_off_peak_config_uses_full_price() {
        // Without xunfei_off_peak config, all records should use full price
        let _guard = pricing_test_guard();
        // Default config has xunfei_off_peak = None
        let record = make_record("pi", "xunfei", "astron-code-latest", 1_000_000, 0.0);
        let cost = display_cost(&record);
        let expected = PricingConfig::default().special.xunfei_per_call;
        assert!(
            (cost - expected).abs() < 1e-9,
            "no off-peak config should use full price: expected {}, got {}",
            expected,
            cost
        );
    }

    #[test]
    fn xunfei_peak_boundary_at_08_00_is_peak() {
        // Exactly at 08:00 CST → peak starts (hour >= peak_start)
        let _guard = pricing_test_guard();
        let prev_env = std::env::var("PRICING_CONFIG").ok();
        let _tmp = load_temp_config(&xunfei_off_peak_config());

        // 2026-06-18 08:00 CST = 00:00 UTC
        let mut record = make_record("pi", "xunfei", "astron-code-latest", 1_000_000, 0.0);
        record.time = "2026-06-18T00:00:00Z".to_string(); // 08:00 CST, peak boundary
        let cost = display_cost(&record);
        let expected = PricingConfig::default().special.xunfei_per_call; // full price
        assert!(
            (cost - expected).abs() < 1e-9,
            "08:00 CST should be peak: expected {}, got {}",
            expected,
            cost
        );

        restore_pricing_env(prev_env);
    }

    #[test]
    fn xunfei_peak_boundary_at_22_00_is_off_peak() {
        // Exactly at 22:00 CST → off-peak starts (hour >= peak_end)
        let _guard = pricing_test_guard();
        let prev_env = std::env::var("PRICING_CONFIG").ok();
        let _tmp = load_temp_config(&xunfei_off_peak_config());

        // 2026-06-18 22:00 CST = 14:00 UTC
        let mut record = make_record("pi", "xunfei", "astron-code-latest", 1_000_000, 0.0);
        record.time = "2026-06-18T14:00:00Z".to_string(); // 22:00 CST, off-peak boundary
        let cost = display_cost(&record);
        let expected = PricingConfig::default().special.xunfei_per_call * 0.8;
        assert!(
            (cost - expected).abs() < 1e-9,
            "22:00 CST should be off-peak: expected {}, got {}",
            expected,
            cost
        );

        restore_pricing_env(prev_env);
    }
}
