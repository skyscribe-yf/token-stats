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
use chrono::{DateTime, Datelike, FixedOffset, NaiveDate, Timelike, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

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

/// Model-scoped, time-based OpenCode Go plan divisor.
///
/// Unmatched models and records before a segment's `effective_from` keep
/// [`SpecialPricing::opencode_divisor`]. When several segments match, the
/// latest qualifying `effective_from` wins (same rule as model prices).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpencodeModelSegment {
    /// Model-name substrings (case-insensitive) this segment applies to.
    pub models: Vec<String>,
    /// Inclusive start. `None` = baseline override for the listed models.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effective_from: Option<String>,
    pub divisor: f64,
}

/// Time-based CodeBuddy credit price (CNY per credit).
///
/// Plan upgrades change the amortized price of a credit (e.g. ¥70 / 4000
/// credits → ¥140 / 9000 credits on 2026-09-14). Records before the first
/// segment keep [`SpecialPricing::codebuddy_cny_per_credit`] (the historical
/// rate); when several segments qualify, the latest `effective_from` wins
/// (same rule as model prices / OpenCode segments).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodebuddyCreditSegment {
    /// Inclusive start: RFC3339 with offset, or `"YYYY-MM-DD"` = 00:00 UTC+8.
    /// `None` = baseline override applying to all records.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effective_from: Option<String>,
    /// CNY per credit from `effective_from` onwards.
    pub cny_per_credit: f64,
}

/// Kimi API list price in CNY per 1M tokens. Subscription estimates apply a
/// user-selected multiplier to this raw API equivalent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KimiApiModelPrice {
    pub name: String,
    pub input: f64,
    pub cache_read: f64,
    pub output: f64,
}

fn default_kimi_subscription_multiplier() -> f64 {
    20.0
}

fn default_kimi_api_models() -> Vec<KimiApiModelPrice> {
    vec![
        KimiApiModelPrice {
            name: "kimi-k3".to_string(),
            input: 20.0,
            cache_read: 2.0,
            output: 100.0,
        },
        KimiApiModelPrice {
            name: "kimi-k2.6".to_string(),
            input: 6.5,
            cache_read: 1.1,
            output: 27.0,
        },
        KimiApiModelPrice {
            name: "kimi-k2.7".to_string(),
            input: 6.5,
            cache_read: 1.3,
            output: 27.0,
        },
    ]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpecialPricing {
    pub xunfei_per_call: f64,
    /// CodeBuddy credits are priced at ¥70 per 4000 credits (CNY, no FX).
    /// Baseline for records that precede every `codebuddy_credit_segments`
    /// entry (kept at the historical plan price).
    #[serde(default = "default_codebuddy_cny_per_credit")]
    pub codebuddy_cny_per_credit: f64,
    /// Plan-upgrade segments for the CodeBuddy credit → CNY rate, e.g.
    /// `{ effective_from = "2026-09-14T13:00:00+08:00", cny_per_credit = 140.0 / 9000.0 }`.
    /// Empty = always use the flat `codebuddy_cny_per_credit`.
    #[serde(default)]
    pub codebuddy_credit_segments: Vec<CodebuddyCreditSegment>,
    /// Legacy flat Kimi rate, retained solely to parse existing pricing files.
    #[serde(default)]
    pub kimi_per_token: f64,
    /// Actual subscription cost = raw Kimi API equivalent / this multiplier.
    #[serde(default = "default_kimi_subscription_multiplier")]
    pub kimi_subscription_multiplier: f64,
    /// Kimi API prices in CNY per 1M tokens.
    #[serde(default = "default_kimi_api_models")]
    pub kimi_api_models: Vec<KimiApiModelPrice>,
    #[serde(default)]
    pub xiaomi_mimo_tp_per_token: f64,
    pub opencode_divisor: f64,
    /// Model-scoped OpenCode Go divisor overrides. Empty = always use
    /// `opencode_divisor`.
    #[serde(default)]
    pub opencode_model_segments: Vec<OpencodeModelSegment>,
    /// Legacy single-value divisor. Kept for backward compatibility.
    /// When `ainaba_segments` is non-empty, segments take precedence.
    #[serde(default)]
    pub ainaba_divisor: f64,
    /// Time-based rate segments (preferred). If empty, falls back to `ainaba_divisor`.
    #[serde(default)]
    pub ainaba_segments: Vec<AinabaSegment>,
    /// Ainaiba (AI奶爸/Yairouter) 平台结算汇率（元/USD），固定不变（7.0）。
    /// 充值 396 元 → 8000 元额度（倍率 8000/396 = 20.20202）；
    /// API 按美元面值计费，平台以该固定汇率折算人民币扣减。
    /// 实际成本 = USD × 7.0 / 20.20202 ≈ USD × 0.3465（元）。
    /// 倍率按时间段分段（见 ainaba_segments），平台汇率不分段。
    #[serde(default = "default_ainaba_platform_rate")]
    pub ainaba_platform_rate: f64,
    #[serde(default)]
    pub freemodel_divisor: f64,
    #[serde(default)]
    pub commandcode_divisor: f64,
    /// CommandCode platform rate: fixed USD→CNY rate used for cost calculation.
    /// CommandCode charges platform fees, so the effective rate differs from market.
    /// Default: 7.0 (same as Ainaiba/YAI Router)
    #[serde(default = "default_commandcode_platform_rate")]
    pub commandcode_platform_rate: f64,
    #[serde(default = "default_fenno_divisor")]
    pub fenno_divisor: f64,
    /// Meituan LongCat per-token cost in CNY (resource pack billing).
    /// Only non-cached input + output tokens are billed; cache hits are free.
    /// Default: 10 CNY / 50,000,000 tokens = 0.0000002
    #[serde(default = "default_meituan_per_token")]
    pub meituan_per_token: f64,
    /// Ollama Cloud empirical per-token cost in CNY.
    /// Derived from: $20/mo Pro × 6.7894 / (weekly_quota × 52/12)
    /// = ¥135.79 / 1,266,666,667 ≈ 0.0000001072
    #[serde(default)]
    pub ollama_cloud_empirical_per_token: f64,
    /// Ollama Cloud weekly quota in tokens (empirical).
    /// Derived from: 38M tokens / 13% ≈ 292,307,692
    #[serde(default)]
    pub ollama_cloud_empirical_weekly_quota: i64,
    /// Per-model multipliers relative to the Ollama Cloud baseline estimator.
    /// Models not listed here use the baseline multiplier of 1.0.
    #[serde(default = "default_ollama_cloud_model_multipliers")]
    pub ollama_cloud_model_multipliers: HashMap<String, f64>,
    /// Grok (XAI / Super Grok) subscription discount.
    /// 50 RMB → 3 months → ~$1,950 API value ($150/week × 13 weeks).
    /// Actual cost = API computed cost in CNY / this divisor.
    /// Default: $1,950 × 6.7894 / 50 = 264.79
    #[serde(default = "default_grok_divisor")]
    pub grok_divisor: f64,
    /// StepFun StepPlan subscription discount: actual cost = StepFun's CNY list
    /// price ÷ this divisor. ¥99 buys ¥1600 of API quota (deducted at list
    /// price), so 1600 / 99 ≈ 16.16.
    #[serde(default = "default_stepfun_plan_divisor")]
    pub stepfun_plan_divisor: f64,
    /// Off-peak (波谷) pricing configuration for xunfei/xunfei-ex.
    /// If `None`, no off-peak discount is applied (always full price).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub xunfei_off_peak: Option<XunfeiOffPeakConfig>,
    /// ZCode BigModel GLM Coding Plan: CNY price per 积分 (credit).
    /// 0 = disabled (zcode records fall through to model-price pricing).
    /// 实付分摊口径（2026-09-12）：3 个月实付 ¥188.04 = 13 周 × 10000 积分 =
    /// 130,000 积分 → 每积分 188.04 / 130000 ≈ ¥0.00144646。
    /// 成本 = 官方积分公式算出的积分 × 高峰因子 × 每积分单价。
    #[serde(default)]
    pub zcode_cny_per_credit: f64,
    /// ZCode 积分消耗系数（积分/万 token，lowercase key）：
    /// `[input, cache_read, output]`。即官方积分公式的抵扣系数，
    /// 2026-09-12 对 live quota API 验证（回算 3768.7 vs 实测 3774 积分，-0.14%）。
    /// GLM-5.3-Flash = [2.3, 0.56, 8.0]; GLM-5.3 = [6.9, 1.7, 24.0]。
    #[serde(default)]
    pub zcode_credit_rates: HashMap<String, [f64; 3]>,
    /// Peak (1×) hour ranges in UTC+8 for the ZCode plan. Official rule:
    /// 高峰 = 周一至周五 14:00–18:00 (UTC+8)；其余时段消耗按 50% 抵扣。
    #[serde(default = "default_zcode_peak_hours")]
    pub zcode_peak_hours_cst: Vec<[u32; 2]>,
    /// Off-peak credit discount factor (0.5 = half credits outside peak).
    #[serde(default = "default_zcode_off_peak_factor")]
    pub zcode_off_peak_factor: f64,
    /// 夜间畅用活动 free window (UTC+8 hours, [start,end)): during the promo
    /// (2026-09-03 ~ 09-20) GLM-5.3-Flash on ZCode consumes 0 credits
    /// between 23:00 and 09:00 daily. Empty = no free window.
    #[serde(default = "default_zcode_night_free_hours")]
    pub zcode_night_free_hours_cst: Vec<[u32; 2]>,
    /// Free window date bounds (inclusive, "YYYY-MM-DD", UTC+8). Both must be
    /// set for the free window to apply.
    #[serde(default)]
    pub zcode_night_free_from: Option<String>,
    #[serde(default)]
    pub zcode_night_free_until: Option<String>,
    /// 免费窗口的实际生效时刻（RFC3339 含时区，如 "2026-09-13T06:15:00+08:00"）。
    /// 旧版 ZCode 客户端不享受夜间免扣，服务端在该时刻前照常扣积分；此时刻之前
    /// 落在免费窗口内的记录按正常高峰/波谷因子计费。None = 窗口在 from..until
    /// 全程有效。
    #[serde(default)]
    pub zcode_night_free_effective_from: Option<String>,
    /// 夜间畅用适用的模型白名单（lowercase）。官方活动为 GLM-5.3-Flash 专属，
    /// GLM-5.3 夜间照常按高峰/波谷因子扣积分。缺省 = ["glm-5.3-flash"]。
    #[serde(default = "default_zcode_night_free_models")]
    pub zcode_night_free_models: Vec<String>,
    /// ZAI (api.zairouter.com) 充值汇率：¥1 = 1 美元额度（订单实测
    /// amount=10000 分 → credit_amount=100.0）。实际成本 = 平台实收美元费率 ×
    /// 本汇率。本机为 1.0；换套餐/换汇率时改这里。
    #[serde(default = "default_zai_rate_cny_per_usd")]
    pub zai_rate_cny_per_usd: f64,
    /// Qoder（qodercli / Qoder Desktop）当前套餐完全免费：模型目录里
    /// `isFree: true`、`priceFactor: 0`，账单不扣额度。为 true 时
    /// provider=qoder 的记录成本恒为 ¥0，而不是落到「无价目条目 → N/A」。
    /// 套餐转付费后请关掉本项并登记 `[[model]]` 价目。
    #[serde(default)]
    pub qoder_free: bool,
}

fn default_zai_rate_cny_per_usd() -> f64 {
    1.0
}

/// Baseline credit price: ¥70 / 4000 credits（国内版连续包月活动价）。
/// Serves as the fallback for records that predate every
/// `codebuddy_credit_segments` entry — the 2026-09-14 升级套餐
/// (¥140 / 9000 credits) is expressed as a segment, not by moving this value.
fn default_codebuddy_cny_per_credit() -> f64 {
    70.0 / 4000.0
}

fn default_grok_divisor() -> f64 {
    1950.0 * 6.7894 / 50.0
}

/// StepFun StepPlan：¥99 套餐给 ¥1600 的 API 额度（按官方列表价扣减），
/// 所以实付 = 列表价 ÷ (1600 / 99) ≈ 列表价 × 6.19%。
fn default_stepfun_plan_divisor() -> f64 {
    1600.0 / 99.0
}

fn default_fenno_divisor() -> f64 {
    150.0 * 6.7894 / 10.0
}

fn default_ainaba_platform_rate() -> f64 {
    7.0
}

fn default_commandcode_platform_rate() -> f64 {
    7.0
}

fn default_meituan_per_token() -> f64 {
    10.0 / 50_000_000.0 // 10 CNY / 50M tokens
}

fn default_ollama_cloud_model_multipliers() -> HashMap<String, f64> {
    HashMap::from([
        ("glm-5.2".to_string(), 1.0),
        ("deepseek-v4-flash".to_string(), 0.2),
        ("deepseek-v4-flash:0731".to_string(), 0.2),
        ("deepseek-v4-flash:0731-cloud".to_string(), 0.2),
        ("deepseek-v4.1-flash".to_string(), 0.2),
    ])
}

fn default_zcode_peak_hours() -> Vec<[u32; 2]> {
    vec![[14, 18]]
}

fn default_zcode_off_peak_factor() -> f64 {
    0.5
}

fn default_zcode_night_free_hours() -> Vec<[u32; 2]> {
    vec![[23, 24], [0, 9]]
}

fn default_zcode_night_free_models() -> Vec<String> {
    vec!["glm-5.3-flash".to_string()]
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
    #[serde(default)]
    pub input: f64,
    #[serde(default)]
    pub output: f64,
    #[serde(default)]
    pub cache_read: f64,
    #[serde(default)]
    pub cache_write: f64,
    /// UTC hour ranges (`[start, end)`) during which the optional `peak_*`
    /// rates apply.  Used by providers that publish recurring peak/off-peak
    /// pricing after a dated price segment becomes effective.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub peak_hours_utc: Vec<[u32; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peak_input: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peak_output: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peak_cache_read: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peak_cache_write: Option<f64>,
    /// Optional CNY-denominated rates (CNY per 1M tokens). When present, the
    /// model cost is computed directly in CNY without any USD→CNY conversion.
    /// Used for providers that publish CNY list prices (e.g. DeepSeek 官方,
    /// DimAgent 平台积分价).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_cny: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_cny: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read_cny: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write_cny: Option<f64>,
    /// Optional CNY peak-hour rates, paired with `peak_hours_utc`. When the
    /// record falls in a peak window and these are present, they override the
    /// base CNY rates (e.g. DimAgent vision-exp 高峰时段 2×).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peak_input_cny: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peak_output_cny: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peak_cache_read_cny: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peak_cache_write_cny: Option<f64>,
    /// Tier threshold in total input tokens (input + cache_read + cache_write).
    /// None = base tier (threshold 0). Some(128000) = applies when total_input >= 128K.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tier_threshold: Option<i64>,
    /// When true, the peak windows in `peak_hours_utc` apply on weekdays
    /// (UTC) only; weekends always use the base (off-peak) rates. Command
    /// Code's DeepSeek peak pricing is documented as "Peak runs Monday to
    /// Friday only, so it is never charged at the weekend"
    /// (https://commandcode.ai/models/deepseek-v4-1-flash), which the plain
    /// hour-range match would otherwise get wrong.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub peak_weekdays_only: bool,
    /// Optional RFC3339 timestamp marking when this price entry becomes effective.
    /// Entries without `effective_from` are the baseline (apply to all records).
    /// Entries with `effective_from` apply only to records whose time >= effective_from.
    /// Used for time-segmented pricing, e.g. official price reductions that take
    /// effect at a specific moment. When multiple time segments match a record,
    /// the one with the latest effective_from wins.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effective_from: Option<String>,
}

/// A time-bounded USD→CNY exchange rate segment.
///
/// `effective_from` semantics match model pricing segments:
/// - `None` = baseline/catch-all segment covering the earliest records;
/// - `"YYYY-MM-DD"` = effective at 00:00 China Standard Time (UTC+8) on that day;
/// - RFC3339 = effective at that exact instant.
///
/// For a given record, the latest segment whose `effective_from` <= record time
/// wins. When no segments are configured, `usd_to_cny` applies to everything
/// (legacy behavior).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsdToCnySegment {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effective_from: Option<String>,
    pub rate: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PricingConfig {
    pub usd_to_cny: f64,
    pub rate_date: String,
    /// Optional historical USD→CNY rate segments (see [`UsdToCnySegment`]).
    #[serde(default)]
    pub usd_to_cny_segments: Vec<UsdToCnySegment>,
    pub special: SpecialPricing,
    #[serde(default)]
    pub model: Vec<ModelPriceConfig>,
    /// Provider-scoped model prices for Yairouter/Ainaba. These overrides take
    /// effect only once their dated segment is active; earlier records retain
    /// the regular model schedule.
    #[serde(default)]
    pub yairouter_model: Vec<ModelPriceConfig>,
    /// DimAgent platform-scoped model prices (CNY, from the DimAgent console
    /// plans API converted at the subscription's credit price). Only records
    /// with source == "dim" use this table; other sources keep the regular
    /// model schedule. Example: deepseek-v4-flash-vision-exp with peak-hour
    /// (CST 09:00–12:00 / 14:00–18:00) double pricing.
    #[serde(default)]
    pub dim_model: Vec<ModelPriceConfig>,
    /// ZAI (api.zairouter.com) **billed** model rates (USD / 1M). ZAI publishes
    /// no price list and its charge is not a uniform multiple of the official
    /// Anthropic price, so the absolute billed rates live here — reverse
    /// engineered from the platform's own `credit_used` ledger. Only records
    /// with provider == "zai" use this table; models absent from it fall back
    /// to the official `[[model]]` price.
    #[serde(default)]
    pub zai_model: Vec<ModelPriceConfig>,
}

impl Default for PricingConfig {
    fn default() -> Self {
        Self {
            usd_to_cny: 6.7894,
            rate_date: "2026-07-31".to_string(),
            usd_to_cny_segments: Vec::new(),
            special: SpecialPricing {
                xunfei_per_call: 199.0 / 90_000.0,
                codebuddy_cny_per_credit: default_codebuddy_cny_per_credit(),
                codebuddy_credit_segments: Vec::new(),
                kimi_per_token: 199.0 / 2_800_000_000.0,
                kimi_subscription_multiplier: default_kimi_subscription_multiplier(),
                kimi_api_models: default_kimi_api_models(),
                // 99 CNY subscription, dashboard 672.26M tokens ≈ 84% usage
                // effective per-token = 99 * 0.84 / 672_260_000 ≈ 0.0000001237
                xiaomi_mimo_tp_per_token: 0.0000001237,
                opencode_divisor: 6.0,
                opencode_model_segments: Vec::new(),
                ainaba_divisor: 1.0,
                ainaba_segments: Vec::new(),
                ainaba_platform_rate: default_ainaba_platform_rate(),
                freemodel_divisor: 67.894,
                commandcode_divisor: 1.0,
                commandcode_platform_rate: default_commandcode_platform_rate(),
                fenno_divisor: default_fenno_divisor(),
                meituan_per_token: default_meituan_per_token(),
                ollama_cloud_empirical_per_token: 0.0000001072,
                ollama_cloud_empirical_weekly_quota: 292307692,
                ollama_cloud_model_multipliers: default_ollama_cloud_model_multipliers(),
                grok_divisor: default_grok_divisor(),
                stepfun_plan_divisor: default_stepfun_plan_divisor(),
                xunfei_off_peak: None,
                zcode_cny_per_credit: 0.0,
                zcode_credit_rates: HashMap::new(),
                zcode_peak_hours_cst: default_zcode_peak_hours(),
                zcode_off_peak_factor: default_zcode_off_peak_factor(),
                zcode_night_free_hours_cst: default_zcode_night_free_hours(),
                zcode_night_free_from: None,
                zcode_night_free_until: None,
                zcode_night_free_effective_from: None,
                zcode_night_free_models: default_zcode_night_free_models(),
                zai_rate_cny_per_usd: default_zai_rate_cny_per_usd(),
                qoder_free: false,
            },
            model: Vec::new(),
            yairouter_model: Vec::new(),
            dim_model: Vec::new(),
            zai_model: Vec::new(),
        }
    }
}

impl PricingConfig {
    /// Build a fast lookup map from model names to prices.
    fn build_model_map(&self) -> HashMap<String, ModelPrice> {
        Self::build_model_map_for(&self.model)
    }

    fn build_yairouter_model_map(&self) -> HashMap<String, ModelPrice> {
        Self::build_model_map_for(&self.yairouter_model)
    }

    fn build_dim_model_map(&self) -> HashMap<String, ModelPrice> {
        Self::build_model_map_for(&self.dim_model)
    }

    fn build_zai_model_map(&self) -> HashMap<String, ModelPrice> {
        Self::build_model_map_for(&self.zai_model)
    }

    fn build_model_map_for(models: &[ModelPriceConfig]) -> HashMap<String, ModelPrice> {
        // Group configs by model name
        let mut groups: HashMap<String, Vec<&ModelPriceConfig>> = HashMap::new();
        for m in models {
            groups.entry(m.name.clone()).or_default().push(m);
        }
        // Build ModelPrice from each group (time-segment + tier validation inside)
        groups
            .into_iter()
            .map(|(name, configs)| (name, ModelPrice::from_configs(&configs)))
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
    peak_input: Option<f64>,
    peak_output: Option<f64>,
    peak_cache_read: Option<f64>,
    peak_cache_write: Option<f64>,
    input_cny: Option<f64>,
    output_cny: Option<f64>,
    cache_read_cny: Option<f64>,
    cache_write_cny: Option<f64>,
    peak_input_cny: Option<f64>,
    peak_output_cny: Option<f64>,
    peak_cache_read_cny: Option<f64>,
    peak_cache_write_cny: Option<f64>,
}

/// A time segment of a model's pricing. Holds the token-count tiers that apply
/// to records whose time falls in this segment.
#[derive(Debug, Clone)]
struct TimeSegment {
    /// When this price segment takes effect. `None` = baseline (oldest rates)
    /// that applies to every record unless a later dated segment overrides it.
    effective_from: Option<DateTime<FixedOffset>>,
    /// Inclusive UTC hour ranges (`[start, end)`) using the optional peak
    /// rates on its tiers. An empty list means the base rates always apply.
    peak_hours_utc: Vec<[u32; 2]>,
    /// Restrict the peak windows above to weekdays (UTC). Providers that
    /// publish "peak runs Monday to Friday only" (Command Code DeepSeek)
    /// charge the off-peak rate all weekend; without this flag the hour-range
    /// match would bill weekend records at peak rates.
    peak_weekdays_only: bool,
    /// Token-count tiers sorted by threshold ascending (first = base, threshold 0).
    tiers: Vec<PriceTier>,
}

impl TimeSegment {
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

    fn is_peak_hour(&self, record_time: Option<&DateTime<FixedOffset>>) -> bool {
        let Some(record_time) = record_time else {
            return false;
        };
        let utc = record_time.with_timezone(&Utc);
        if self.peak_weekdays_only
            && matches!(utc.weekday(), chrono::Weekday::Sat | chrono::Weekday::Sun)
        {
            return false;
        }
        let hour = utc.hour();
        self.peak_hours_utc.iter().any(|[start, end]| {
            if start <= end {
                *start <= hour && hour < *end
            } else {
                hour >= *start || hour < *end
            }
        })
    }
}

#[derive(Debug, Clone)]
struct ModelPrice {
    /// Time segments sorted by effective_from ascending (None/baseline first).
    /// For a given record, the latest segment whose effective_from <= record
    /// time wins; the baseline (None) segment applies when no dated segment
    /// qualifies yet.
    segments: Vec<TimeSegment>,
}

impl ModelPrice {
    /// Build from a slice of ModelPriceConfig entries sharing the same name.
    /// Entries are grouped by `effective_from` into time segments; within each
    /// segment, tiers are sorted by token-count threshold.
    fn from_configs(configs: &[&ModelPriceConfig]) -> Self {
        // Group configs by their effective_from string (None = baseline).
        let mut groups: HashMap<Option<String>, Vec<&ModelPriceConfig>> = HashMap::new();
        for c in configs {
            groups.entry(c.effective_from.clone()).or_default().push(c);
        }

        let mut segments: Vec<TimeSegment> = groups
            .into_iter()
            .map(|(eff_from, cfgs)| {
                // Warn if a single time segment defines multiple base tiers.
                let base_count = cfgs.iter().filter(|c| c.tier_threshold.is_none()).count();
                if base_count > 1 {
                    tracing::warn!(
                        "Model time segment {:?} has {} base-tier entries, using last one",
                        eff_from,
                        base_count
                    );
                } else if base_count == 0 {
                    tracing::warn!(
                        "Model time segment {:?} has no base-tier entry; inputs below the \
                         lowest threshold will use that tier's rates",
                        eff_from
                    );
                }
                let mut tiers: Vec<PriceTier> = cfgs
                    .iter()
                    .map(|c| PriceTier {
                        threshold: c.tier_threshold.unwrap_or(0),
                        input: c.input,
                        output: c.output,
                        cache_read: c.cache_read,
                        cache_write: c.cache_write,
                        peak_input: c.peak_input,
                        peak_output: c.peak_output,
                        peak_cache_read: c.peak_cache_read,
                        peak_cache_write: c.peak_cache_write,
                        input_cny: c.input_cny,
                        output_cny: c.output_cny,
                        cache_read_cny: c.cache_read_cny,
                        cache_write_cny: c.cache_write_cny,
                        peak_input_cny: c.peak_input_cny,
                        peak_output_cny: c.peak_output_cny,
                        peak_cache_read_cny: c.peak_cache_read_cny,
                        peak_cache_write_cny: c.peak_cache_write_cny,
                    })
                    .collect();
                tiers.sort_by_key(|t| t.threshold);
                let effective_from = eff_from
                    .as_ref()
                    .and_then(|s| DateTime::parse_from_rfc3339(s).ok());
                let peak_hours_utc = cfgs
                    .iter()
                    .find(|c| !c.peak_hours_utc.is_empty())
                    .map(|c| c.peak_hours_utc.clone())
                    .unwrap_or_default();
                let peak_weekdays_only = cfgs.iter().any(|c| c.peak_weekdays_only);
                TimeSegment {
                    effective_from,
                    peak_hours_utc,
                    peak_weekdays_only,
                    tiers,
                }
            })
            .collect();
        // None (baseline) sorts first as -inf, then dated segments by time.
        segments.sort_by_key(|s| {
            s.effective_from
                .map(|dt| dt.timestamp())
                .unwrap_or(i64::MIN)
        });
        Self { segments }
    }

    /// Whether this provider-scoped price has started for the record time.
    /// Scoped configurations deliberately have no baseline: before their first
    /// dated segment, the normal model price must remain in effect.
    fn has_active_dated_segment(&self, record_time: &str) -> bool {
        let Ok(record_time) = DateTime::parse_from_rfc3339(record_time) else {
            return false;
        };
        self.segments.iter().any(|segment| {
            segment
                .effective_from
                .as_ref()
                .is_some_and(|effective_from| record_time >= *effective_from)
        })
    }

    /// Select the time segment for a record. `record_time` is the parsed RFC3339
    /// timestamp, or None if unparseable (in which case the baseline segment is
    /// used as a conservative default).
    fn select_segment(&self, record_time: Option<DateTime<FixedOffset>>) -> &TimeSegment {
        let mut chosen: Option<&TimeSegment> = None;
        for seg in &self.segments {
            let qualifies = match (&seg.effective_from, record_time) {
                (None, _) => true, // baseline always qualifies
                (Some(from), Some(rt)) => rt >= *from,
                (Some(_), None) => false, // can't compare without a record time
            };
            if qualifies {
                // Segments are sorted ascending, so the last qualifying one is
                // the latest applicable time segment.
                chosen = Some(seg);
            }
        }
        // If nothing qualified (record predates all dated segments and there
        // is no baseline), fall back to the earliest segment.
        chosen.unwrap_or_else(|| &self.segments[0])
    }

    /// Compute cost in USD for the given token counts at the record's time.
    /// `record_time` is the raw RFC3339 timestamp string from the record.
    fn compute_usd(
        &self,
        input_tokens: i64,
        output_tokens: i64,
        cache_read_tokens: i64,
        cache_write_tokens: i64,
        record_time: &str,
    ) -> f64 {
        let rt = DateTime::parse_from_rfc3339(record_time).ok();
        let seg = self.select_segment(rt.clone());
        let total_input = input_tokens + cache_read_tokens + cache_write_tokens;
        let tier = seg.select_tier(total_input);
        let (input, output, cache_read, cache_write) = if seg.is_peak_hour(rt.as_ref()) {
            (
                tier.peak_input.unwrap_or(tier.input),
                tier.peak_output.unwrap_or(tier.output),
                tier.peak_cache_read.unwrap_or(tier.cache_read),
                tier.peak_cache_write.unwrap_or(tier.cache_write),
            )
        } else {
            (tier.input, tier.output, tier.cache_read, tier.cache_write)
        };
        input_tokens as f64 * input / 1_000_000.0
            + output_tokens as f64 * output / 1_000_000.0
            + cache_read_tokens as f64 * cache_read / 1_000_000.0
            + cache_write_tokens as f64 * cache_write / 1_000_000.0
    }

    /// Whether this model is priced directly in CNY (any tier carries a CNY
    /// rate). CNY-priced models skip the USD→CNY conversion entirely.
    fn is_cny_priced(&self) -> bool {
        self.segments.iter().any(|seg| {
            seg.tiers.iter().any(|tier| {
                tier.input_cny.is_some()
                    || tier.output_cny.is_some()
                    || tier.cache_read_cny.is_some()
                    || tier.cache_write_cny.is_some()
            })
        })
    }

    /// Compute cost in CNY directly from CNY-denominated tier rates.
    /// Missing CNY fields are treated as 0.0 (e.g. DeepSeek has no cache_write).
    fn compute_cny(
        &self,
        input_tokens: i64,
        output_tokens: i64,
        cache_read_tokens: i64,
        cache_write_tokens: i64,
        record_time: &str,
    ) -> f64 {
        let rt = DateTime::parse_from_rfc3339(record_time).ok();
        let seg = self.select_segment(rt.clone());
        let total_input = input_tokens + cache_read_tokens + cache_write_tokens;
        let tier = seg.select_tier(total_input);
        let peak = seg.is_peak_hour(rt.as_ref());
        let (input, output, cache_read, cache_write) = if peak {
            (
                tier.peak_input_cny.or(tier.input_cny),
                tier.peak_output_cny.or(tier.output_cny),
                tier.peak_cache_read_cny.or(tier.cache_read_cny),
                tier.peak_cache_write_cny.or(tier.cache_write_cny),
            )
        } else {
            (
                tier.input_cny,
                tier.output_cny,
                tier.cache_read_cny,
                tier.cache_write_cny,
            )
        };
        input_tokens as f64 * input.unwrap_or(0.0) / 1_000_000.0
            + output_tokens as f64 * output.unwrap_or(0.0) / 1_000_000.0
            + cache_read_tokens as f64 * cache_read.unwrap_or(0.0) / 1_000_000.0
            + cache_write_tokens as f64 * cache_write.unwrap_or(0.0) / 1_000_000.0
    }
}

// ── Segmented USD→CNY exchange rate schedule ────────────────────────────────

/// Parse a segment `effective_from`: RFC3339 as-is, or `"YYYY-MM-DD"`
/// interpreted as 00:00 China Standard Time (UTC+8). Returns None on parse
/// failure (caller should warn and skip the segment).
fn parse_rate_effective_from(s: &str) -> Option<DateTime<FixedOffset>> {
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Some(dt);
    }
    let east8 = FixedOffset::east_opt(8 * 3600).unwrap();
    NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .ok()
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .map(|naive| naive.and_local_timezone(east8).single().unwrap())
}

/// One resolved rate segment with rate-derived divisor snapshots.
#[derive(Debug, Clone)]
struct RateSegment {
    effective_from: Option<DateTime<FixedOffset>>,
    rate: f64,
    /// rate-derived divisors, scaled by `rate / current_rate` so subscription
    /// invariants (FreeModel 0.1x, Fenno 10/150, Grok 50/1950, Ollama ¥/token)
    /// hold regardless of which segment a record falls into.
    freemodel_divisor: f64,
    fenno_divisor: f64,
    grok_divisor: f64,
    ollama_per_token: f64,
}

/// Resolved USD→CNY rate schedule built once at config load/reload.
#[derive(Debug, Clone)]
struct RateSchedule {
    /// Sorted: baseline (None) first, then by `effective_from` ascending.
    /// Always non-empty: when the config has no segments, a single implicit
    /// baseline segment carrying `usd_to_cny` and the configured divisors is
    /// used (exact legacy behavior).
    segments: Vec<RateSegment>,
    /// Rate in effect "now": latest segment rate, or `usd_to_cny` when no
    /// segments are configured. Used by quota cards / current-state displays.
    current_rate: f64,
}

impl RateSchedule {
    fn new(config: &PricingConfig) -> Self {
        let mut segments: Vec<RateSegment> = Vec::new();
        for seg in &config.usd_to_cny_segments {
            let effective_from = match seg.effective_from.as_deref() {
                Some(s) => match parse_rate_effective_from(s) {
                    Some(dt) => Some(dt),
                    None => {
                        tracing::warn!(
                            "Invalid usd_to_cny_segment effective_from '{}', skipping segment",
                            s
                        );
                        continue;
                    }
                },
                None => None,
            };
            segments.push(RateSegment {
                effective_from,
                rate: seg.rate,
                freemodel_divisor: 0.0,
                fenno_divisor: 0.0,
                grok_divisor: 0.0,
                ollama_per_token: 0.0,
            });
        }
        // None (baseline) sorts first as -inf, then dated segments by time.
        segments.sort_by_key(|s| {
            s.effective_from
                .map(|dt| dt.timestamp())
                .unwrap_or(i64::MIN)
        });

        let current_rate = segments.last().map(|s| s.rate).unwrap_or(config.usd_to_cny);

        if segments.is_empty() {
            // No segments configured → single implicit baseline (legacy behavior).
            segments.push(RateSegment {
                effective_from: None,
                rate: config.usd_to_cny,
                freemodel_divisor: config.special.freemodel_divisor,
                fenno_divisor: config.special.fenno_divisor,
                grok_divisor: config.special.grok_divisor,
                ollama_per_token: config.special.ollama_cloud_empirical_per_token,
            });
        } else {
            // Scale rate-derived divisors per segment, preserving invariants:
            //   rate_seg / divisor_seg == rate_current / divisor_base
            for seg in &mut segments {
                let scale = seg.rate / current_rate;
                seg.freemodel_divisor = config.special.freemodel_divisor * scale;
                seg.fenno_divisor = config.special.fenno_divisor * scale;
                seg.grok_divisor = config.special.grok_divisor * scale;
                seg.ollama_per_token = config.special.ollama_cloud_empirical_per_token * scale;
            }
        }

        Self {
            segments,
            current_rate,
        }
    }

    /// Select the rate segment for a record (same semantics as model pricing
    /// segments: latest qualifying `effective_from` wins, baseline always
    /// qualifies; unparseable record time uses the baseline).
    fn select_segment(&self, record_time: &str) -> &RateSegment {
        let rt = DateTime::parse_from_rfc3339(record_time).ok();
        let mut chosen: Option<&RateSegment> = None;
        for seg in &self.segments {
            let qualifies = match (&seg.effective_from, rt) {
                (None, _) => true, // baseline always qualifies
                (Some(from), Some(rt)) => rt >= *from,
                (Some(_), None) => false,
            };
            if qualifies {
                chosen = Some(seg);
            }
        }
        chosen.unwrap_or_else(|| &self.segments[0])
    }

    fn rate_for(&self, record_time: &str) -> f64 {
        self.select_segment(record_time).rate
    }

    fn freemodel_divisor_for(&self, record_time: &str) -> f64 {
        self.select_segment(record_time).freemodel_divisor
    }

    fn fenno_divisor_for(&self, record_time: &str) -> f64 {
        self.select_segment(record_time).fenno_divisor
    }

    fn grok_divisor_for(&self, record_time: &str) -> f64 {
        self.select_segment(record_time).grok_divisor
    }

    fn ollama_per_token_for(&self, record_time: &str) -> f64 {
        self.select_segment(record_time).ollama_per_token
    }
}

/// Internal state that holds both the user config and the derived lookup map.
pub(crate) struct PricingState {
    config: PricingConfig,
    model_map: HashMap<String, ModelPrice>,
    yairouter_model_map: HashMap<String, ModelPrice>,
    dim_model_map: HashMap<String, ModelPrice>,
    zai_model_map: HashMap<String, ModelPrice>,
    kimi_api_model_map: HashMap<String, KimiApiModelPrice>,
    rate_schedule: RateSchedule,
    /// DimAgent subscription billing rates learned from the account's own
    /// entitlement payload. `dim_model_map` holds the platform **list** prices;
    /// this applies the discount actually charged for each record's moment.
    dim_rates: crate::dim_entitlement::RateTable,
}

impl PricingState {
    fn new(config: PricingConfig) -> Self {
        let model_map = config.build_model_map();
        let yairouter_model_map = config.build_yairouter_model_map();
        let dim_model_map = config.build_dim_model_map();
        let zai_model_map = config.build_zai_model_map();
        let kimi_api_model_map = config
            .special
            .kimi_api_models
            .iter()
            .cloned()
            .map(|price| (price.name.clone(), price))
            .collect();
        let rate_schedule = RateSchedule::new(&config);
        Self {
            config,
            model_map,
            yairouter_model_map,
            dim_model_map,
            zai_model_map,
            kimi_api_model_map,
            rate_schedule,
            // Loaded by `init()`/`reload()` so unit tests never read the
            // machine's live entitlement state.
            dim_rates: HashMap::new(),
        }
    }

    fn reload(&mut self, config: PricingConfig) {
        self.model_map = config.build_model_map();
        self.yairouter_model_map = config.build_yairouter_model_map();
        self.dim_model_map = config.build_dim_model_map();
        self.zai_model_map = config.build_zai_model_map();
        self.kimi_api_model_map = config
            .special
            .kimi_api_models
            .iter()
            .cloned()
            .map(|price| (price.name.clone(), price))
            .collect();
        self.rate_schedule = RateSchedule::new(&config);
        self.dim_rates = crate::dim_entitlement::load_table();
        self.config = config;
    }
}

// ── Global state ─────────────────────────────────────────────────────────────

fn state_cell() -> &'static RwLock<PricingState> {
    static CELL: OnceLock<RwLock<PricingState>> = OnceLock::new();
    CELL.get_or_init(|| RwLock::new(PricingState::new(PricingConfig::default())))
}

/// One read guard for a whole request's worth of `display_cost_in` calls.
/// Reload swaps the whole state under the write lock, so holding a read
/// guard across an aggregation keeps pricing consistent for that request.
pub(crate) fn state_read() -> std::sync::RwLockReadGuard<'static, PricingState> {
    state_cell().read().unwrap()
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
    let mut config = load_config_from_file(&path);
    let ss = crate::settings::load_subscription_settings();
    config.special.kimi_subscription_multiplier = ss.kimi_subscription_multiplier;
    config.special.grok_divisor = ss.grok_divisor;
    let mut state = state_cell().write().unwrap();
    *state = PricingState::new(config);
    state.dim_rates = crate::dim_entitlement::load_table();
}

/// Reload pricing configuration from disk without restarting the server.
pub fn reload() {
    let path = config_path();
    let mut config = load_config_from_file(&path);
    let ss = crate::settings::load_subscription_settings();
    config.special.kimi_subscription_multiplier = ss.kimi_subscription_multiplier;
    config.special.grok_divisor = ss.grok_divisor;
    let mut state = state_cell().write().unwrap();
    state.reload(config);
    tracing::info!("Pricing configuration reloaded from {:?}", path);
}

/// Return a clone of the current pricing configuration (for the API endpoint).
pub fn get_config() -> PricingConfig {
    state_cell().read().unwrap().config.clone()
}

/// `GET /api/pricing` payload: the TOML config plus the Dim billing rates
/// learned from the account's entitlement payload, so a wrong-looking Dim cost
/// can be traced to either side without shell access.
pub fn config_view() -> serde_json::Value {
    let state = state_cell().read().unwrap();
    let mut view = serde_json::to_value(&state.config).unwrap_or_else(|_| serde_json::json!({}));
    if let Some(obj) = view.as_object_mut() {
        obj.insert(
            "dim_entitlement_rates".to_string(),
            crate::dim_entitlement::table_view(&state.dim_rates),
        );
    }
    view
}

/// Replace the learned Dim entitlement rates after a poll. TOML keeps the
/// platform list prices; this is the discount actually billed.
pub fn set_dim_entitlement_rates(table: crate::dim_entitlement::RateTable) {
    state_cell().write().unwrap().dim_rates = table;
}

/// Current USD→CNY rate (latest segment, or `usd_to_cny` when no segments).
/// Used for quota cards and other "current state" displays.
pub fn current_rate() -> f64 {
    state_cell().read().unwrap().rate_schedule.current_rate
}

/// Update the live Kimi subscription multiplier after persisted settings change.
pub fn set_kimi_subscription_multiplier(multiplier: f64) {
    state_cell()
        .write()
        .unwrap()
        .config
        .special
        .kimi_subscription_multiplier = multiplier;
}

pub fn set_grok_divisor(divisor: f64) {
    let mut state = state_cell().write().unwrap();
    state.config.special.grok_divisor = divisor;
    // Divisors are derived per rate segment, so rebuild the schedule.
    state.rate_schedule = RateSchedule::new(&state.config);
}

// ── Model price resolution ───────────────────────────────────────────────────

fn resolve_model_price<'a>(
    state: &'a PricingState,
    record: &TokenRecord,
) -> Option<&'a ModelPrice> {
    let model = &record.model;
    let provider = &record.provider;

    if is_yairouter_billed(record) {
        let model_lower = model.to_lowercase();
        let override_key = if model_lower.contains("gpt-5.6-terra") {
            Some("gpt-5.6-terra")
        } else if model_lower.contains("gpt-5.6-luna") {
            Some("gpt-5.6-luna")
        } else {
            None
        };

        if let Some(price) = override_key
            .and_then(|key| state.yairouter_model_map.get(key))
            .filter(|price| price.has_active_dated_segment(&record.time))
        {
            return Some(price);
        }
    }

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

fn resolve_kimi_api_price<'a>(
    state: &'a PricingState,
    model: &str,
) -> Option<&'a KimiApiModelPrice> {
    let model_lower = model.to_lowercase();
    if model_lower.contains("kimi-k3") || model_lower.contains("k3-256k") {
        return state.kimi_api_model_map.get("kimi-k3");
    }
    if model_lower.contains("kimi-k2.6") {
        return state.kimi_api_model_map.get("kimi-k2.6");
    }
    if model_lower.contains("kimi-k2.7") {
        return state.kimi_api_model_map.get("kimi-k2.7");
    }
    state.kimi_api_model_map.get("kimi-k2.7")
}

fn compute_kimi_subscription_cost(
    state: &PricingState,
    record: &TokenRecord,
    multiplier: f64,
) -> Option<f64> {
    let price = resolve_kimi_api_price(state, &record.model)?;
    let raw_api_cost = (record.input_tokens as f64 * price.input
        + record.cache_read_tokens as f64 * price.cache_read
        + record.output_tokens as f64 * price.output)
        / 1_000_000.0;
    Some(raw_api_cost / multiplier)
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
        s if s.starts_with("deepseek-v4-pro") => "cc:deepseek-v4-pro",
        s if s.starts_with("deepseek-v4-flash") => "cc:deepseek-v4-flash",

        // Moonshot/Kimi
        "kimi-k2.6" => "cc:kimi-k2.6",
        "kimi-k2.5" => "cc:kimi-k2.5",

        // Zhipu/GLM
        "glm-5.3" => "cc:glm-5.3",
        "glm-5.3-flash" => "cc:glm-5.3-flash",
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

/// Normalize a Crof model name to the `crof:` prefix used in pricing.toml.
///
/// Crof model names come as plain names like `deepseek-v4-flash-0731`.
/// Maps to `crof:` prefixed keys in the pricing model map.
fn resolve_crof_price<'a>(state: &'a PricingState, model: &str) -> Option<&'a ModelPrice> {
    let crof_key = normalize_crof_model(model);
    state.model_map.get(&crof_key)
}

fn normalize_crof_model(model: &str) -> String {
    let lower = model.to_lowercase();

    let key = match lower.as_str() {
        "deepseek-v4-flash-0731" => "crof:deepseek-v4-flash-0731",
        "deepseek-v4-flash" => "crof:deepseek-v4-flash",
        "deepseek-v4-pro-0813" => "crof:deepseek-v4-pro-0813",
        "deepseek-v4-pro" => "crof:deepseek-v4-pro",
        // Fallback: try with crof: prefix
        other => return format!("crof:{}", other),
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

/// Whether this record was billed by ZAI (ZAI Router, `api.zairouter.com`).
///
/// ZAI is a Chinese relay for Claude Code: you top up RMB and get USD face
/// value 1:1 (order probe: ¥100 → credit_amount 100.0), but each model is
/// charged at a multiple of the official Anthropic list price. Measured
/// 2026-09-15 against the platform's own `credit_used` ledger:
///
/// | model                              | in | out | cache_read | cache_write | vs official |
/// |------------------------------------|----|-----|------------|-------------|-------------|
/// | claude-fable-5-1 / mythos-5-1      | 25 | 100 |    0.63    |     50      |  ×2.5 (cw ×4) |
/// | claude-fable-5 / mythos-5          | 25 | 100 |    0.078   |     50      |  ×2.5 (cw ×4) |
/// | claude-opus-5 / 4-8 / 4-7 / 4-6    |  5 |  25 |    0.50    |     10      |  1× (cw = 1h tier) |
/// | claude-sonnet-5 / 4-6 / 4-5        |  3 |  15 |    0.30    |      6      |  1× (cw = 1h tier) |
///
/// `claude-haiku-4-5` is not usable on this subscription (the platform
/// rejects it with "The long context beta is not yet available").
///
/// The multipliers live in `pricing.toml` (`zai_model_multipliers`) so the
/// table can be corrected without a rebuild.
fn is_zai_billed(record: &TokenRecord) -> bool {
    let effective = record
        .original_provider
        .as_deref()
        .unwrap_or(&record.provider);
    record.provider == "zai" || effective == "zai"
}

/// Whether a record is StepFun traffic billed against the StepPlan coding
/// subscription (¥99 for ¥1600 of list-price quota).
///
/// Two transports produce these:
/// - `source == "stepfun-proxy"` — the CLIProxyAPI `stepfun` upstream, metered
///   per request by the `stepfun-usage` plugin;
/// - DimAgent channels whose raw provider id is `stepfun` or `step-plan`
///   (`api.stepfun.com[/step_plan]/v1`). Dim stores no usable cost for them
///   (`quality: "missing_price"`), so the rate has to come from `pricing.toml`.
///
/// Matched on `original_provider` because vendor merge could rename the
/// displayed provider. `step-plan-intl` (api.stepfun.ai) is priced in USD and
/// deliberately excluded.
fn is_stepfun_plan_billed(record: &TokenRecord) -> bool {
    if record.source == "stepfun-proxy" {
        return true;
    }
    matches!(
        record
            .original_provider
            .as_deref()
            .unwrap_or(record.provider.as_str()),
        "stepfun" | "step-plan"
    )
}

/// Cost of a ZAI record in CNY.
///
/// ZAI publishes no public price list, so `pricing.toml` carries the platform's
/// actual **billed** rates per model (USD / 1M) in `[[zai_model]]`, reverse
/// engineered from the platform's own `credit_used` ledger. Rates are not a
/// uniform multiple of the official Anthropic price: Fable 5.1 bills input at
/// 2.5× official but output at 2×, and its cache terms differ again — so the
/// absolute rates are stored rather than a multiplier.
///
/// Models without a `[[zai_model]]` entry fall back to the official list price
/// (`[[model]]`), which under-bills but keeps the card useful for a model the
/// table hasn't been calibrated for yet.
///
/// Model names are matched with any `vendor/` prefix stripped, because the
/// platform's own mapper accepts both `claude-fable-5-1` and
/// `anthropic/claude-fable`.
fn compute_zai_cost(state: &PricingState, record: &TokenRecord) -> Option<f64> {
    let special = &state.config.special;

    let bare = record
        .model
        .rsplit_once('/')
        .map(|(_, m)| m)
        .unwrap_or(&record.model)
        .to_lowercase();

    let usd = if let Some(billed) = state.zai_model_map.get(&bare) {
        // Calibrated billed rates: flat, no long-context tier and no peak
        // windows (verified up to 273K input tokens).
        billed.compute_usd(
            record.input_tokens,
            record.output_tokens,
            record.cache_read_tokens,
            record.cache_write_tokens,
            &record.time,
        )
    } else {
        let normalized = TokenRecord {
            model: bare.clone(),
            ..record.clone()
        };
        resolve_model_price(state, &normalized)?
            .compute_usd(
                record.input_tokens,
                record.output_tokens,
                record.cache_read_tokens,
                record.cache_write_tokens,
                &record.time,
            )
    };

    Some(usd * special.zai_rate_cny_per_usd)
}

/// Yairouter / Ainaba billed providers share the same settlement:
/// official USD list price × fixed platform rate / subscription divisor.
fn is_yairouter_billed(record: &TokenRecord) -> bool {
    let effective = record
        .original_provider
        .as_deref()
        .unwrap_or(&record.provider);
    matches!(
        record.provider.as_str(),
        "ainaba" | "ainaiba" | "yai-router"
    ) || matches!(effective, "ainaba" | "ainaiba" | "yai-router")
}

/// Select the OpenCode Go divisor for a record.
///
/// Model-scoped segments override [`SpecialPricing::opencode_divisor`] when
/// the model name contains a listed substring and the record time is on or
/// after that segment's `effective_from`. The latest qualifying segment wins.
/// Select the CodeBuddy credit → CNY rate for a record.
///
/// `codebuddy_credit_segments` (plan upgrades) take precedence; records older
/// than every segment keep the flat `codebuddy_cny_per_credit` baseline, which
/// stays at the historical plan price (¥70 / 4000 credits).
fn codebuddy_cny_per_credit_for(special: &SpecialPricing, record_time: &str) -> f64 {
    if special.codebuddy_credit_segments.is_empty() {
        return special.codebuddy_cny_per_credit;
    }

    let record_dt = DateTime::parse_from_rfc3339(record_time).ok();
    let mut chosen: Option<(i64, f64)> = None;

    for segment in &special.codebuddy_credit_segments {
        let effective_from = segment
            .effective_from
            .as_deref()
            .and_then(parse_rate_effective_from);
        let qualifies = match (effective_from, record_dt) {
            (None, _) => true,
            (Some(from), Some(rt)) => rt >= from,
            (Some(_), None) => false,
        };
        if !qualifies {
            continue;
        }
        let ts = effective_from.map(|dt| dt.timestamp()).unwrap_or(i64::MIN);
        if chosen.is_none_or(|(prev, _)| ts >= prev) {
            chosen = Some((ts, segment.cny_per_credit));
        }
    }

    chosen
        .map(|(_, rate)| rate)
        .unwrap_or(special.codebuddy_cny_per_credit)
}

fn get_opencode_divisor(special: &SpecialPricing, model: &str, record_time: &str) -> f64 {
    if special.opencode_model_segments.is_empty() {
        return special.opencode_divisor;
    }

    let model_lower = model.to_lowercase();
    let record_dt = DateTime::parse_from_rfc3339(record_time).ok();
    let mut chosen: Option<(i64, f64)> = None;

    for segment in &special.opencode_model_segments {
        if !segment
            .models
            .iter()
            .any(|name| model_lower.contains(&name.to_lowercase()))
        {
            continue;
        }
        let effective_from = segment
            .effective_from
            .as_deref()
            .and_then(parse_rate_effective_from);
        let qualifies = match (effective_from, record_dt) {
            (None, _) => true,
            (Some(from), Some(rt)) => rt >= from,
            (Some(_), None) => false,
        };
        if !qualifies {
            continue;
        }
        let ts = effective_from.map(|dt| dt.timestamp()).unwrap_or(i64::MIN);
        if chosen.is_none_or(|(prev, _)| ts >= prev) {
            chosen = Some((ts, segment.divisor));
        }
    }

    chosen
        .map(|(_, divisor)| divisor)
        .unwrap_or(special.opencode_divisor)
}

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

/// Whether a model name belongs to the OpenAI GPT family (gpt-5.5, gpt-5.4,
/// gpt-5.6-sol/terra/luna, etc.). Used to scope provider-specific recomputation
/// (e.g. Fenno) to GPT models, where official price reductions apply.
fn is_gpt_family(model: &str) -> bool {
    model.to_lowercase().contains("gpt")
}

fn ollama_cloud_model_multiplier(special: &SpecialPricing, model: &str) -> f64 {
    special
        .ollama_cloud_model_multipliers
        .get(model)
        .copied()
        .unwrap_or(1.0)
}

/// Compute the actual CNY cost of a ZCode (BigModel GLM Coding Plan) record.
///
/// Official credit formula (docs.bigmodel.cn/cn/coding-plan/overview):
///   credits = (input×2.3 + cache_read×0.56 + output×8) / 10000  (GLM-5.3-Flash)
///   credits = (input×6.9 + cache_read×1.7 + output×24) / 10000  (GLM-5.3)
/// Peak hours (Mon–Fri 14:00–18:00 UTC+8) consume 1×; all other times 0.5×.
/// During the Flash×ZCode night promo (2026-09-03..09-20, 23:00–09:00 CST,
/// ZCode client) credits are 0 — but only for the models whitelisted in
/// `zcode_night_free_models` (the official promo is GLM-5.3-Flash-only;
/// GLM-5.3 bills normally at night), and only from
/// `zcode_night_free_effective_from` onwards: the old ZCode client build did
/// not enjoy the promo server-side, so records in the window before that
/// instant (2026-09-13 06:15 CST) bill at the normal peak/off-peak factor.
/// Actual cost = credits × factor ×
/// zcode_cny_per_credit（实付分摊：3 个月 ¥188.04 / 13 周×10000 积分 =
/// ¥0.00144646/积分）。
///
/// The record's `input_tokens` already excludes cache reads/writes (the zcode
/// parser subtracts them, Anthropic convention) — matches the "输入 Token" term
/// in the official formula.
fn compute_zcode_credit_cost(special: &SpecialPricing, record: &TokenRecord) -> Option<f64> {
    // Weekend Build 体验套餐（provider=bigmodel-start，赠送的 3 亿 token 额度）
    // 不消耗正式套餐积分 → 实际成本 0。
    if record.provider == "bigmodel-start" {
        return Some(0.0);
    }
    if special.zcode_cny_per_credit <= 0.0 {
        return None;
    }
    let model_key = record.model.to_lowercase();
    let rates = special.zcode_credit_rates.get(&model_key)?;
    let rt = DateTime::parse_from_rfc3339(&record.time).ok()?;
    let cst = rt.with_timezone(&FixedOffset::east_opt(8 * 3600)?);
    let hour = cst.hour();

    // 夜间畅用活动：活动日期内每日 23:00–09:00 CST，ZCode 端积分消耗为 0。
    // 活动仅限白名单模型（zcode_night_free_models，官方为 GLM-5.3-Flash 专属，
    // GLM-5.3 夜间照常扣积分）；且对旧版客户端不生效（zcode_night_free_effective_from
    // 之前服务端照常扣积分），窗口内的更早记录跳过免费分支，按正常高峰/波谷因子计费。
    if let (Some(from), Some(until)) = (
        special.zcode_night_free_from.as_deref(),
        special.zcode_night_free_until.as_deref(),
    ) {
        let promo_live = special
            .zcode_night_free_effective_from
            .as_deref()
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|effective| cst >= effective)
            .unwrap_or(true);
        let model_eligible = special
            .zcode_night_free_models
            .iter()
            .any(|m| *m == model_key);
        let date = cst.format("%Y-%m-%d").to_string();
        if promo_live
            && model_eligible
            && date.as_str() >= from
            && date.as_str() <= until
        {
            let in_free = special.zcode_night_free_hours_cst.iter().any(|[s, e]| {
                if s <= e {
                    *s <= hour && hour < *e
                } else {
                    hour >= *s || hour < *e
                }
            });
            if in_free {
                return Some(0.0);
            }
        }
    }

    // 高峰（周一至周五 14:00–18:00 CST）按 1×，其余时段按 0.5× 抵扣。
    let weekday = cst.weekday();
    let is_weekday = matches!(
        weekday,
        chrono::Weekday::Mon
            | chrono::Weekday::Tue
            | chrono::Weekday::Wed
            | chrono::Weekday::Thu
            | chrono::Weekday::Fri
    );
    let peak = is_weekday
        && special
            .zcode_peak_hours_cst
            .iter()
            .any(|[s, e]| *s <= hour && hour < *e);
    let factor = if peak { 1.0 } else { special.zcode_off_peak_factor };

    // 官方积分公式：积分 = (输入×系数 + 缓存×系数 + 输出×系数) / 10000
    let credits = (record.input_tokens as f64 * rates[0]
        + record.cache_read_tokens as f64 * rates[1]
        + record.output_tokens as f64 * rates[2])
        / 10_000.0;
    Some(credits * factor * special.zcode_cny_per_credit)
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
#[cfg(test)]
pub fn display_cost(record: &TokenRecord) -> f64 {
    display_cost_in(&state_cell().read().unwrap(), record)
}

/// Same as [`display_cost`] but against an already-held pricing-state guard,
/// so hot loops (aggregation over N records) pay one lock, not N.
pub(crate) fn display_cost_in(state: &PricingState, record: &TokenRecord) -> f64 {
    let cfg = &state.config;
    let schedule = &state.rate_schedule;

    // CodeBuddy stores the raw credit charge in TokenRecord.cost. Convert
    // credits to CNY at the plan's amortized credit price: baseline
    // ¥70 / 4000 credits, superseded by `codebuddy_credit_segments` after the
    // 2026-09-14 plan upgrade (¥140 / 9000 credits).
    // `dim-agent` records (DimAgent via the workbuddy proxy) carry the same
    // credit cost from the CodeBuddy web API, so they share this formula.
    if record.source == "codebuddy" || record.source == "dim-agent" {
        return record.cost * codebuddy_cny_per_credit_for(&cfg.special, &record.time);
    }

    // 1a. Qoder (qodercli / Qoder Desktop) — the current plan is free: the
    //     catalog marks every model `isFree: true` with `priceFactor: 0`, so
    //     price it at ¥0 rather than falling through to "no entry → N/A".
    if record.provider == "qoder" && cfg.special.qoder_free {
        return 0.0;
    }

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

    // 2. Kimi provider with zero stored cost: model-aware API-equivalent CNY
    //    cost divided by the subscription multiplier. Cache writes are free.
    if record.provider == "kimi" && record.cost == 0.0 {
        if let Some(cost) =
            compute_kimi_subscription_cost(&state, record, cfg.special.kimi_subscription_multiplier)
        {
            return cost;
        }
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
        return record.cost / get_opencode_divisor(&cfg.special, &record.model, &record.time)
            * schedule.rate_for(&record.time);
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
                &record.time,
            );
            return usd * schedule.rate_for(&record.time) / cfg.special.commandcode_divisor;
        }
    }

    // 4a. ZAI (api.zairouter.com): always recompute from token counts, because
    //     claude-code stores cost=0 for this source. The platform charges the
    //     official Anthropic list price scaled by a per-model multiplier and
    //     settles in USD credit bought 1:1 with RMB, so the result is already
    //     in CNY (zai_rate_cny_per_usd = 1.0).
    if is_zai_billed(record) {
        if let Some(cost) = compute_zai_cost(&state, record) {
            return cost;
        }
    }

    // 4a2. Crof provider: always compute from normalized tokens using
    //     crof model prices from pricing.toml. We ignore the extension's stored
    //     cost because it was calculated with incorrect pricing.
    //
    //     Crof model prices in pricing.toml are the listed API rate (USD / 1M).
    //     Convert to CNY using market rate.
    if record.provider == "crof" {
        if let Some(mp) = resolve_crof_price(&state, &record.model) {
            let usd = mp.compute_usd(
                record.input_tokens,
                record.output_tokens,
                record.cache_read_tokens,
                record.cache_write_tokens,
                &record.time,
            );
            return usd * schedule.rate_for(&record.time);
        }
    }

    // 4b. Ollama Cloud (subscription): empirical per-token estimate in CNY.
    //     The baseline rate is calibrated from glm-5.2. Model-specific
    //     multipliers then adjust that baseline (e.g. DeepSeek flash = 20%).
    //     Always uses the empirical subscription rate regardless of stored cost,
    //     because Ollama is a flat $20/mo subscription, not pay-per-token.
    if record.provider == "ollama" || record.provider == "ollama-cloud" {
        let multiplier = ollama_cloud_model_multiplier(&cfg.special, &record.model);
        return record.total_tokens as f64
            * schedule.ollama_per_token_for(&record.time)
            * multiplier;
    }

    // 4b2. StepFun StepPlan coding subscription (¥99 for ¥1600 of list-price
    //      quota): bill at StepFun's published CNY list rates and amortize the
    //      plan over them. Covers both the CLIProxyAPI per-request log
    //      (`source=stepfun-proxy`) and DimAgent's direct StepPlan channels,
    //      whose stored cost is `missing_price` (0). `step-plan-intl` is a
    //      USD-priced endpoint and intentionally not included.
    if is_stepfun_plan_billed(record) {
        if let Some(mp) = resolve_model_price(&state, record) {
            let cny = if mp.is_cny_priced() {
                mp.compute_cny(
                    record.input_tokens,
                    record.output_tokens,
                    record.cache_read_tokens,
                    record.cache_write_tokens,
                    &record.time,
                )
            } else {
                mp.compute_usd(
                    record.input_tokens,
                    record.output_tokens,
                    record.cache_read_tokens,
                    record.cache_write_tokens,
                    &record.time,
                ) * schedule.rate_for(&record.time)
            };
            let divisor = cfg.special.stepfun_plan_divisor;
            if divisor > 0.0 {
                return cny / divisor;
            }
            return cny;
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

        // Pi's stored Ainaba cost can lag the latest pricing table for long
        // contexts, so recompute from token counts when a known model exists.
        if is_yairouter_billed(record) {
            if let Some(mp) = resolve_model_price(&state, record) {
                let usd = mp.compute_usd(
                    record.input_tokens,
                    record.output_tokens,
                    record.cache_read_tokens,
                    record.cache_write_tokens,
                    &record.time,
                );
                // Ainaiba 按平台固定结算汇率折算（充值 396→8000 额度），不走市场分段汇率。
                return usd * cfg.special.ainaba_platform_rate
                    / get_ainaba_divisor(&cfg.special, &record.time);
            }
        }

        // Fenno subscription: like Ainaba, recompute GPT models from token
        // counts so time-segmented official prices (e.g. the 2026-07-31
        // GPT-5.6 Terra/Luna reduction) are applied. Non-GPT Fenno models and
        // models without a pricing.toml entry fall through to the stored-cost
        // path below. Actual cost = official list price (USD) × usd_to_cny ÷
        // fenno_divisor (10 CNY buys 150 USD face value).
        if (effective_provider == "fenno" || effective_provider == "fenno-ex")
            && is_gpt_family(&record.model)
        {
            if let Some(mp) = resolve_model_price(&state, record) {
                let usd = mp.compute_usd(
                    record.input_tokens,
                    record.output_tokens,
                    record.cache_read_tokens,
                    record.cache_write_tokens,
                    &record.time,
                );
                return usd * schedule.rate_for(&record.time)
                    / schedule.fenno_divisor_for(&record.time);
            }
        }

        // 4a2. Xiaomi MiMo Pi provider: cost is in CNY (from platform), display as-is
        if effective_provider == "xiaomi-mimo" || effective_provider == "xiaomi-mimo-tp" {
            return record.cost;
        }

        // 4b. opencode-go Pi provider: cost is in USD from OpenCode API
        //     Apply OpenCode Go plan divisor + convert to CNY
        if effective_provider == "opencode-go" {
            return record.cost / get_opencode_divisor(&cfg.special, &record.model, &record.time)
                * schedule.rate_for(&record.time);
        }

        // 4b2. kimi-coding Pi provider: subscription model, same as kimi-code.
        //     The stored cost is an API list price, not the actual subscription
        //     cost. Recompute from token counts and apply the multiplier.
        //     (original_provider preserved by vendor merge from "kimi-coding" → "kimi")
        if effective_provider == "kimi-coding" {
            if let Some(cost) = compute_kimi_subscription_cost(
                &state,
                record,
                cfg.special.kimi_subscription_multiplier,
            ) {
                return cost;
            }
        }

        // 4b3. Dim Grok Build channel: routed to xAI's official API and billed
        //     against the same SuperGrok subscription as grok-cli xai-official
        //     records. The stored catalog cost is the API list price, not the
        //     actual subscription cost — recompute from token counts and apply
        //     the grok divisor. Covers both the new mapping (provider =
        //     "xai-official") and pre-migration rows (provider = "grok-build").
        if record.source == "dim"
            && (record.provider == "xai-official" || effective_provider == "grok-build")
        {
            if let Some(mp) = resolve_model_price(&state, record) {
                let usd = mp.compute_usd(
                    record.input_tokens,
                    record.output_tokens,
                    record.cache_read_tokens,
                    record.cache_write_tokens,
                    &record.time,
                );
                return usd * schedule.rate_for(&record.time)
                    / schedule.grok_divisor_for(&record.time);
            }
        }

        // 4c. Other Pi providers: cost is in USD, convert to CNY.
        //     Ainaiba 使用平台固定结算汇率 7.0（充值 396 元 → 8000 元额度），
        //     其余提供商按记录时间所在的市场汇率分段折算。
        let base_rate = if is_yairouter_billed(record) {
            cfg.special.ainaba_platform_rate
        } else {
            schedule.rate_for(&record.time)
        };
        let mut cny = record.cost * base_rate;

        // Yairouter / Ainaba time-based rate: divisor depends on record timestamp
        // (provider="ainaba" after vendor merge, covering both Pi and Codex;
        //  grok-cli records keep provider="yai-router")
        if is_yairouter_billed(record) {
            cny /= get_ainaba_divisor(&cfg.special, &record.time);
        }

        // FreeModel discount: 1 USD face value = 0.1 CNY actual cost
        if record.provider == "FreeModel" {
            cny /= schedule.freemodel_divisor_for(&record.time);
        }

        // Fenno subscription discount: 10 CNY buys 150 USD face value.
        // After USD→CNY conversion, divide by the effective face-value ratio.
        if effective_provider == "fenno" || effective_provider == "fenno-ex" {
            cny /= schedule.fenno_divisor_for(&record.time);
        }

        return cny;
    }

    // 4d. DeepSeek records with cost=0 (e.g. from session recovery or DeepSeek
    //     platform CSV export). DeepSeek publishes CNY prices, so the cost is
    //     computed directly in CNY (no USD→CNY conversion). No divisor - the
    //     user pays DeepSeek directly at official rates.
    let effective_provider = record
        .original_provider
        .as_deref()
        .unwrap_or(&record.provider);
    if effective_provider == "deepseek" && record.cost == 0.0 {
        if let Some(mp) = resolve_model_price(&state, record) {
            if mp.is_cny_priced() {
                return mp.compute_cny(
                    record.input_tokens,
                    record.output_tokens,
                    record.cache_read_tokens,
                    record.cache_write_tokens,
                    &record.time,
                );
            }
            let usd = mp.compute_usd(
                record.input_tokens,
                record.output_tokens,
                record.cache_read_tokens,
                record.cache_write_tokens,
                &record.time,
            );
            return usd * schedule.rate_for(&record.time);
        }
    }

    // 6. Derived sources without original cost: codex, claude-code, kimi-code,
    //    Grok CLI, zcode, dsh, etc. (commandcode native is handled above via
    //    provider == "commandcode")
    //    Compute from per-model token rates. pricing.toml model prices are in
    //    USD by default; CNY-priced models (e.g. DeepSeek 官方) skip conversion.
    //    DimAgent (source == "dim") uses the platform's own credit-based table
    //    (`dim_model`) converted to CNY, with peak-hour double pricing.
    if record.source == "codex"
        || record.source == "claude-code"
        || record.source == "kimi-code"
        || record.source == "grok-cli"
        || record.source == "zcode"
        || record.source == "dsh"
        || record.source == "dim"
    {
        // ZCode BigModel GLM Coding Plan records (provider=bigmodel) are billed
        // by subscription credits (积分), not model list prices. When the plan
        // credit price is configured, prefer the credit-based cost; other
        // zcode records (opencode-go / tokenrouter channels) fall through.
        if record.source == "zcode" {
            if let Some(cost) = compute_zcode_credit_cost(&cfg.special, record) {
                return cost;
            }
        }

        // DimAgent console-API records are billed at the platform's credit
        // rates (`dim_model`), not the upstream API list price that
        // `model`/`vendor_merge` would resolve to.
        let dim_price = if record.source == "dim" {
            state.dim_model_map.get(&record.model)
        } else {
            None
        };
        let dim_list_priced = dim_price.is_some();
        if let Some(mp) = dim_price.or_else(|| resolve_model_price(&state, record)) {
            let base_rate = if is_yairouter_billed(record) {
                cfg.special.ainaba_platform_rate
            } else {
                schedule.rate_for(&record.time)
            };
            let mut cny = if mp.is_cny_priced() {
                mp.compute_cny(
                    record.input_tokens,
                    record.output_tokens,
                    record.cache_read_tokens,
                    record.cache_write_tokens,
                    &record.time,
                )
            } else {
                mp.compute_usd(
                    record.input_tokens,
                    record.output_tokens,
                    record.cache_read_tokens,
                    record.cache_write_tokens,
                    &record.time,
                ) * base_rate
            };
            // Yairouter / Ainaba time-based rate: divisor depends on record timestamp
            if is_yairouter_billed(record) {
                cny /= get_ainaba_divisor(&cfg.special, &record.time);
            }
            // FreeModel discount: 1 USD face value = 0.1 CNY actual cost
            if record.provider == "FreeModel" {
                cny /= schedule.freemodel_divisor_for(&record.time);
            }
            // OpenCode Go plan discount: listed API cost / opencode_divisor
            // kimi-code records with provider="opencode-go" go through the
            // same OpenCode Go subscription as pi records with opencode-go.
            if record.provider == "opencode-go" {
                cny /= get_opencode_divisor(&cfg.special, &record.model, &record.time);
            }
            if record.provider == "fenno" {
                cny /= schedule.fenno_divisor_for(&record.time);
            }
            if record.provider == "xai-official" || record.provider == "xai" {
                cny /= schedule.grok_divisor_for(&record.time);
            }
            // DimAgent subscription discount: the platform bills a subscription
            // model at list price × the `rate` in the account's own entitlement
            // payload (deepseek-v4.1-flash has been 0.65 since 2026-09-18;
            // glm-5.3 is 0.7, and 0.35 inside its 20:00–08:00 夜间优惠 window).
            // `dim_model` holds list prices only — the rates are learned and
            // time-segmented in `dim_entitlement`, so history keeps whatever
            // was in force when the record happened.
            if record.source == "dim" && dim_list_priced {
                cny *= crate::dim_entitlement::rate_for(
                    &state.dim_rates,
                    &record.model,
                    &record.time,
                );
            }
            return cny;
        }
    }

    // Fallback: no pricing entry for this model. Pi records always carry a
    // real stored cost, so 0 there means genuinely free. Other sources:
    // return -1 to signal "unknown cost" — the frontend renders N/A and the
    // aggregator skips it in sums (totals stay 0, same as before).
    if record.source == "pi" {
        record.cost
    } else {
        -1.0
    }
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
        unsafe { std::env::remove_var("PRICING_CONFIG") };
        let mut state = state_cell().write().unwrap();
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
        unsafe { std::env::set_var("PRICING_CONFIG", tmp.path().to_str().unwrap()) };
        reload();
        tmp
    }

    /// Restore PRICING_CONFIG env var after a temp config test.
    fn restore_pricing_env(prev: Option<String>) {
        match prev {
            Some(v) => unsafe { std::env::set_var("PRICING_CONFIG", v) },
            None => unsafe { std::env::remove_var("PRICING_CONFIG") },
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

    fn stepfun_config() -> Vec<u8> {
        br#"
usd_to_cny = 6.7894
rate_date = "2026-09-20"

[special]
xunfei_per_call = 0.002211111111
kimi_per_token = 0.000000071071429
opencode_divisor = 6.0
ainaba_divisor = 1.0
freemodel_divisor = 67.894
stepfun_plan_divisor = 16.16161616161616

[[model]]
name = "step-5-preview"
input_cny = 7.0
output_cny = 20.0
cache_read_cny = 0.35
cache_write_cny = 0.0
"#
        .to_vec()
    }

    fn stepfun_record(source: &str, provider: &str) -> TokenRecord {
        let mut record = make_record(source, provider, "step-5-preview", 0, 0.0);
        record.original_provider = Some(provider.to_string());
        record.input_tokens = 1_000;
        record.output_tokens = 500;
        record.cache_read_tokens = 2_000;
        record.total_tokens = 3_500;
        record
    }

    /// ¥99 for ¥1600 of list-price quota ⇒ actual = list × 99/1600.
    fn stepfun_expected_cny() -> f64 {
        let list =
            1_000.0 / 1e6 * 7.0 + 500.0 / 1e6 * 20.0 + 2_000.0 / 1e6 * 0.35;
        list * 99.0 / 1600.0
    }

    #[test]
    fn stepfun_proxy_records_bill_at_plan_amortized_rate() {
        let _guard = pricing_test_guard();
        let prev_env = std::env::var("PRICING_CONFIG").ok();
        let _tmp = load_temp_config(&stepfun_config());

        let cost = display_cost(&stepfun_record("stepfun-proxy", "stepfun"));
        let expected = stepfun_expected_cny();
        assert!(
            (cost - expected).abs() < 1e-9,
            "stepfun-proxy should bill list ÷ (1600/99): expected {expected}, got {cost}"
        );

        restore_pricing_env(prev_env);
    }

    #[test]
    fn dim_stepfun_channels_bill_at_plan_amortized_rate() {
        let _guard = pricing_test_guard();
        let prev_env = std::env::var("PRICING_CONFIG").ok();
        let _tmp = load_temp_config(&stepfun_config());

        // Dim's own StepPlan channel stores cost 0 ("missing_price"), so the
        // rate has to come from pricing.toml like the proxy's records.
        let expected = stepfun_expected_cny();
        for provider in ["step-plan", "stepfun"] {
            let cost = display_cost(&stepfun_record("dim", provider));
            assert!(
                (cost - expected).abs() < 1e-9,
                "dim channel {provider} should bill list ÷ (1600/99): expected {expected}, got {cost}"
            );
        }

        // The USD-priced international endpoint is deliberately excluded and
        // falls through to the plain list price.
        let intl = display_cost(&stepfun_record("dim", "step-plan-intl"));
        let list = 1_000.0 / 1e6 * 7.0 + 500.0 / 1e6 * 20.0 + 2_000.0 / 1e6 * 0.35;
        assert!(
            (intl - list).abs() < 1e-9,
            "step-plan-intl should keep the unamortized list price: expected {list}, got {intl}"
        );

        restore_pricing_env(prev_env);
    }

    #[test]
    fn project_pricing_toml_keeps_ainaba_segments_and_models() {
        let cfg: PricingConfig = toml::from_str(include_str!("../pricing.toml"))
            .expect("backend/pricing.toml should parse as PricingConfig");

        assert_eq!(cfg.special.commandcode_divisor, 9.95);
        assert!((cfg.special.codebuddy_cny_per_credit - 70.0 / 4000.0).abs() < 1e-15);
        assert_eq!(cfg.special.ainaba_segments.len(), 3);
        assert_eq!(cfg.special.ainaba_segments[1].divisor, 20.20202);
        assert_eq!(cfg.special.ainaba_segments[2].divisor, 21.538461538);
        assert_eq!(cfg.special.ainaba_platform_rate, 7.0);
        assert_eq!(
            cfg.special
                .xunfei_off_peak
                .as_ref()
                .map(|cfg| (&cfg.effective_from, cfg.coefficient)),
            Some((&"2026-06-18".to_string(), 0.8))
        );
        assert!(cfg.model.iter().any(|m| m.name == "gpt-5.4"));
        assert!(cfg.model.iter().any(|m| m.name == "gpt-5.5"));
        assert!(
            (cfg.special.stepfun_plan_divisor - 1600.0 / 99.0).abs() < 1e-9,
            "pricing.toml should amortize the ¥99 / ¥1600 StepPlan"
        );
        let stepfun_model = cfg
            .model
            .iter()
            .find(|m| m.name == "step-5-preview")
            .expect("pricing.toml should price step-5-preview");
        assert_eq!(stepfun_model.input_cny, Some(7.0));
        assert_eq!(stepfun_model.cache_read_cny, Some(0.35));
        assert_eq!(stepfun_model.output_cny, Some(20.0));
        assert_eq!(
            cfg.special
                .ollama_cloud_model_multipliers
                .get("glm-5.2")
                .copied(),
            Some(1.0)
        );
        assert_eq!(
            cfg.special
                .ollama_cloud_model_multipliers
                .get("deepseek-v4-flash")
                .copied(),
            Some(0.2)
        );
        assert_eq!(
            cfg.special
                .ollama_cloud_model_multipliers
                .get("deepseek-v4-flash:0731")
                .copied(),
            Some(0.2)
        );
        assert_eq!(
            cfg.special
                .ollama_cloud_model_multipliers
                .get("deepseek-v4-flash:0731-cloud")
                .copied(),
            Some(0.2)
        );
        assert_eq!(
            cfg.special
                .ollama_cloud_model_multipliers
                .get("deepseek-v4.1-flash")
                .copied(),
            Some(0.2)
        );
        assert_eq!(cfg.special.opencode_divisor, 6.0);
        assert_eq!(cfg.special.opencode_model_segments.len(), 1);
        assert_eq!(
            cfg.special.opencode_model_segments[0].models,
            vec!["deepseek-v4-flash", "deepseek-v4-pro"]
        );
        assert_eq!(
            cfg.special.opencode_model_segments[0]
                .effective_from
                .as_deref(),
            Some("2026-08-18T00:00:00+08:00")
        );
        assert_eq!(cfg.special.opencode_model_segments[0].divisor, 3.0);
    }

    /// ZAI charges the official Anthropic list price scaled per model, settled
    /// at ¥1 = $1 credit. Assert the table matches the measured ledger.
    #[test]
    fn project_pricing_toml_carries_measured_zai_rates() {
        let cfg: PricingConfig = toml::from_str(include_str!("../pricing.toml"))
            .expect("backend/pricing.toml should parse as PricingConfig");

        assert_eq!(cfg.special.zai_rate_cny_per_usd, 1.0);

        let rate = |name: &str| {
            cfg.zai_model
                .iter()
                .find(|m| m.name == name)
                .unwrap_or_else(|| panic!("missing [[zai_model]] {name}"))
        };

        // Fable 5.1: official 10/50/0.25/12.5 → billed 25/100/0.63/50.
        let fable = rate("claude-fable-5-1");
        assert_eq!(fable.input, 25.0);
        assert_eq!(fable.output, 100.0);
        assert_eq!(fable.cache_read, 0.63);
        assert_eq!(fable.cache_write, 50.0);

        // Opus/Sonnet bill official input/output but the 1h cache-write rate.
        let opus = rate("claude-opus-5");
        assert_eq!(opus.input, 5.0);
        assert_eq!(opus.cache_write, 10.0);
        let sonnet = rate("claude-sonnet-5");
        assert_eq!(sonnet.cache_write, 6.0);

        // The official list prices these were derived from must stay present,
        // since models missing from [[zai_model]] fall back to them.
        for name in ["claude-fable-5-1", "claude-opus-5", "claude-sonnet-5"] {
            assert!(
                cfg.model.iter().any(|m| m.name == name),
                "pricing.toml must define official list price for {name}"
            );
        }
    }

    /// One real `claude-fable-5-1` request from the platform ledger:
    /// 108 requests today charged 105.24 credit; the first single request
    /// (54 in / 26922 out / 1790155 cr / 155184 cw) charged ~9.80.
    #[test]
    fn zai_fable_5_1_matches_measured_charge() {
        let _guard = pricing_test_guard();
        let cfg: PricingConfig = toml::from_str(include_str!("../pricing.toml")).unwrap();
        let state = PricingState::new(cfg);

        let mut record = make_record("claude-code", "zai", "claude-fable-5-1", 0, 0.0);
        record.time = "2026-09-15T03:10:39Z".to_string();
        record.input_tokens = 54;
        record.output_tokens = 26922;
        record.cache_read_tokens = 1790155;
        record.cache_write_tokens = 155184;

        let cost = compute_zai_cost(&state, &record).expect("fable 5.1 must be priced");
        // 54×25 + 26922×100 + 1790155×0.63 + 155184×50, per 1M tokens.
        let expected = (54.0 * 25.0
            + 26922.0 * 100.0
            + 1790155.0 * 0.63
            + 155184.0 * 50.0)
            / 1_000_000.0;
        assert!(
            (cost - expected).abs() < 1e-9,
            "got {cost}, expected {expected}"
        );
    }

    /// The full-day `claude-fable-5-1` aggregate is the real independent check:
    /// the platform reports the charge (105.2439 credit for 108 requests),
    /// while we derive it from the token counts (1960979 in / 30238 out /
    /// 2225481 cr / 1095437 cw). It should close closely.
    #[test]
    fn zai_fable_5_1_day_aggregate_matches_platform_ledger() {
        let _guard = pricing_test_guard();
        let cfg: PricingConfig = toml::from_str(include_str!("../pricing.toml")).unwrap();
        let state = PricingState::new(cfg);

        let mut record = make_record("claude-code", "zai", "claude-fable-5-1", 0, 0.0);
        record.time = "2026-09-15T03:10:39Z".to_string();
        record.input_tokens = 1_960_979;
        record.output_tokens = 30_238;
        record.cache_read_tokens = 2_225_481;
        record.cache_write_tokens = 1_095_437;

        let cost = compute_zai_cost(&state, &record).unwrap();
        let ledger = 105.2439;
        let drift = (cost - ledger).abs() / ledger;
        assert!(
            drift < 0.05,
            "day aggregate {cost} vs ledger {ledger} (drift {:.1}%)",
            drift * 100.0
        );
    }

    /// 1x models bill the official price, except cache write (1h tier = 2×).
    #[test]
    fn zai_one_x_models_use_official_price_but_1h_cache_write() {
        let _guard = pricing_test_guard();
        let cfg: PricingConfig = toml::from_str(include_str!("../pricing.toml")).unwrap();
        let state = PricingState::new(cfg);

        let mut record = make_record("claude-code", "zai", "claude-opus-5", 0, 0.0);
        record.time = "2026-09-15T03:00:00Z".to_string();
        record.input_tokens = 100_000;
        record.output_tokens = 1_000;
        record.cache_read_tokens = 10_000;
        record.cache_write_tokens = 10_000;

        let cost = compute_zai_cost(&state, &record).unwrap();
        // opus-5 billed: 5/25/0.50/10.
        let expected = (100_000.0 * 5.0
            + 1_000.0 * 25.0
            + 10_000.0 * 0.50
            + 10_000.0 * 10.0)
            / 1_000_000.0;
        assert!((cost - expected).abs() < 1e-9, "got {cost}, expected {expected}");
    }

    /// The provider prefix the platform's model mapper accepts must not break
    /// rate lookup (`anthropic/claude-fable` is a real model id there).
    #[test]
    fn zai_strips_provider_prefix_when_matching_multipliers() {
        let _guard = pricing_test_guard();
        let cfg: PricingConfig = toml::from_str(include_str!("../pricing.toml")).unwrap();
        let state = PricingState::new(cfg);

        let mut bare = make_record("claude-code", "zai", "claude-fable-5-1", 0, 0.0);
        bare.time = "2026-09-15T03:00:00Z".to_string();
        bare.output_tokens = 1000;
        let mut prefixed = bare.clone();
        prefixed.model = "anthropic/claude-fable-5-1".to_string();

        let a = compute_zai_cost(&state, &bare).unwrap();
        let b = compute_zai_cost(&state, &prefixed).unwrap();
        assert!((a - b).abs() < 1e-12, "{a} vs {b}");
        // 1000 output × $100/1M = $0.10
        assert!((a - 0.10).abs() < 1e-9, "got {a}");
    }

    /// A model with no `[[zai_model]]` entry falls back to the official price
    /// rather than reporting N/A.
    #[test]
    fn zai_unknown_model_falls_back_to_official_price() {
        let _guard = pricing_test_guard();
        let cfg: PricingConfig = toml::from_str(include_str!("../pricing.toml")).unwrap();
        let state = PricingState::new(cfg);

        let mut record = make_record("claude-code", "zai", "claude-haiku-4-5", 0, 0.0);
        record.time = "2026-09-15T03:00:00Z".to_string();
        record.input_tokens = 1_000_000;
        // Official haiku-4-5 input = $1.00/1M.
        assert!((compute_zai_cost(&state, &record).unwrap() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn codebuddy_credits_convert_to_cny_at_flat_rate() {
        let _guard = pricing_test_guard();
        let mut config = PricingConfig::default();
        config.usd_to_cny = 6.0;
        config.usd_to_cny_segments = vec![
            UsdToCnySegment {
                effective_from: None,
                rate: 5.0,
            },
            UsdToCnySegment {
                effective_from: Some("2026-08-01".to_string()),
                rate: 7.0,
            },
        ];
        state_cell().write().unwrap().reload(config);

        let mut record = make_record("codebuddy", "codebuddy", "gpt-5.6-luna", 100, 3000.0);
        record.date = "2026-08-29".to_string();
        record.time = "2026-08-29T04:44:13.879Z".to_string();
        // 3000 credits × ¥0.0175 (pre-upgrade baseline, no segments configured)
        // = ¥52.5, regardless of FX segments.
        assert!((display_cost(&record) - 52.5).abs() < 1e-12);
    }

    #[test]
    fn codebuddy_default_price_is_seventy_yuan_per_four_thousand_credits() {
        let config = PricingConfig::default();
        assert!((config.special.codebuddy_cny_per_credit - 70.0 / 4000.0).abs() < 1e-15);
        assert!(config.special.codebuddy_credit_segments.is_empty());
    }

    #[test]
    fn codebuddy_credit_segments_switch_rate_at_plan_upgrade() {
        let _guard = pricing_test_guard();
        let mut config = PricingConfig::default();
        config.special.codebuddy_credit_segments = vec![CodebuddyCreditSegment {
            effective_from: Some("2026-09-14T13:00:00+08:00".to_string()),
            cny_per_credit: 140.0 / 9000.0,
        }];
        state_cell().write().unwrap().reload(config);

        // 1000 credits just before the upgrade (12:59:59 CST) → ¥70/4000 rate.
        let mut before = make_record("dim-agent", "codebuddy", "glm-5.3-flash", 100, 1000.0);
        before.time = "2026-09-14T04:59:59Z".to_string();
        assert!((display_cost(&before) - 1000.0 * (70.0 / 4000.0)).abs() < 1e-12);

        // At the cutover instant (13:00:00 CST) → ¥140/9000 rate.
        let mut at = make_record("dim-agent", "codebuddy", "glm-5.3-flash", 100, 1000.0);
        at.time = "2026-09-14T05:00:00Z".to_string();
        assert!((display_cost(&at) - 1000.0 * (140.0 / 9000.0)).abs() < 1e-12);

        // The native `codebuddy` source shares the same schedule.
        let mut native = make_record("codebuddy", "codebuddy", "glm-5.3-flash", 100, 1000.0);
        native.time = "2026-09-14T06:00:00Z".to_string();
        assert!((display_cost(&native) - 1000.0 * (140.0 / 9000.0)).abs() < 1e-12);

        // Earlier history (pre-upgrade days) keeps the old plan price.
        let mut historic = make_record("codebuddy", "codebuddy", "glm-5.3-flash", 100, 1000.0);
        historic.time = "2026-08-29T04:44:13Z".to_string();
        assert!((display_cost(&historic) - 1000.0 * (70.0 / 4000.0)).abs() < 1e-12);
    }

    #[test]
    fn project_pricing_toml_has_codebuddy_upgrade_segment() {
        let cfg: PricingConfig = toml::from_str(include_str!("../pricing.toml"))
            .expect("backend/pricing.toml should parse as PricingConfig");

        // Baseline stays at the historical plan price.
        assert!((cfg.special.codebuddy_cny_per_credit - 70.0 / 4000.0).abs() < 1e-15);
        assert_eq!(cfg.special.codebuddy_credit_segments.len(), 1);
        let seg = &cfg.special.codebuddy_credit_segments[0];
        assert_eq!(
            seg.effective_from.as_deref(),
            Some("2026-09-14T13:00:00+08:00")
        );
        assert!((seg.cny_per_credit - 140.0 / 9000.0).abs() < 1e-15);
    }

    #[test]
    fn ollama_cloud_model_costs_use_glm_baseline_multiplier() {
        let _guard = pricing_test_guard();
        let baseline = PricingConfig::default()
            .special
            .ollama_cloud_empirical_per_token;
        let baseline_cost = 1_000_000.0 * baseline;

        let glm = make_record("pi", "ollama", "glm-5.2", 1_000_000, 0.0);
        let deepseek_flash = make_record("pi", "ollama", "deepseek-v4-flash", 1_000_000, 0.0);
        let deepseek_0731 = make_record("pi", "ollama", "deepseek-v4-flash:0731", 1_000_000, 0.0);
        let deepseek_cloud = make_record(
            "pi",
            "ollama-cloud",
            "deepseek-v4-flash:0731-cloud",
            1_000_000,
            0.0,
        );
        let deepseek_v41 = make_record("pi", "ollama", "deepseek-v4.1-flash", 1_000_000, 0.0);
        let deepseek_v41_proxy = make_record(
            "ollama-proxy",
            "ollama-cloud",
            "deepseek-v4.1-flash",
            1_000_000,
            0.0,
        );

        assert!((display_cost(&glm) - baseline_cost).abs() < 1e-12);
        assert!((display_cost(&deepseek_flash) - baseline_cost * 0.2).abs() < 1e-12);
        assert!((display_cost(&deepseek_0731) - baseline_cost * 0.2).abs() < 1e-12);
        assert!((display_cost(&deepseek_cloud) - baseline_cost * 0.2).abs() < 1e-12);
        assert!(
            (display_cost(&deepseek_v41) - baseline_cost * 0.2).abs() < 1e-12,
            "deepseek-v4.1-flash should bill at the same 0.2× rate as deepseek-v4-flash"
        );
        assert!(
            (display_cost(&deepseek_v41_proxy) - display_cost(&deepseek_flash)).abs() < 1e-12,
            "ollama-proxy deepseek-v4.1-flash should match v4-flash"
        );
    }

    #[test]
    fn project_pricing_toml_has_gpt_5_6_base_and_long_context_tiers() {
        let cfg: PricingConfig = toml::from_str(include_str!("../pricing.toml"))
            .expect("backend/pricing.toml should parse as PricingConfig");

        let expected = [
            ("gpt-5.6-sol", None, 5.0, 30.0, 0.5, 6.25),
            ("gpt-5.6-sol", Some(272_000), 10.0, 45.0, 1.0, 12.5),
            ("gpt-5.6-terra", None, 2.5, 15.0, 0.25, 3.125),
            ("gpt-5.6-terra", Some(272_000), 5.0, 22.5, 0.5, 6.25),
            ("gpt-5.6-luna", None, 1.0, 6.0, 0.1, 1.25),
            ("gpt-5.6-luna", Some(272_000), 2.0, 9.0, 0.2, 2.5),
        ];

        for (name, tier_threshold, input, output, cache_read, cache_write) in expected {
            assert!(
                cfg.model.iter().any(|model| {
                    model.name == name
                        && model.tier_threshold == tier_threshold
                        && (model.input - input).abs() < f64::EPSILON
                        && (model.output - output).abs() < f64::EPSILON
                        && (model.cache_read - cache_read).abs() < f64::EPSILON
                        && (model.cache_write - cache_write).abs() < f64::EPSILON
                }),
                "missing or incorrect {name} tier {tier_threshold:?}"
            );
        }
    }

    #[test]
    fn project_pricing_toml_has_gpt_6_astra_base_and_long_context_tiers() {
        let cfg: PricingConfig = toml::from_str(include_str!("../pricing.toml"))
            .expect("backend/pricing.toml should parse as PricingConfig");

        // OpenAI official standard rates (developers.openai.com/api/docs/pricing):
        // base: input $10 / output $50 / cache_read $1 / cache_write (1.25× input) $12.50
        // >272K total input: input & cache ×2, output ×1.5
        let expected = [
            ("gpt-6-astra", None, 10.0, 50.0, 1.0, 12.5),
            ("gpt-6-astra", Some(272_000), 20.0, 75.0, 2.0, 25.0),
        ];

        for (name, tier_threshold, input, output, cache_read, cache_write) in expected {
            assert!(
                cfg.model.iter().any(|model| {
                    model.name == name
                        && model.tier_threshold == tier_threshold
                        && (model.input - input).abs() < f64::EPSILON
                        && (model.output - output).abs() < f64::EPSILON
                        && (model.cache_read - cache_read).abs() < f64::EPSILON
                        && (model.cache_write - cache_write).abs() < f64::EPSILON
                }),
                "missing or incorrect {name} tier {tier_threshold:?}"
            );
        }
    }

    #[test]
    fn project_pricing_toml_has_current_deepseek_cny_rates() {
        let cfg: PricingConfig = toml::from_str(include_str!("../pricing.toml"))
            .expect("backend/pricing.toml should parse as PricingConfig");

        // DeepSeek publishes CNY/M rates and is priced directly in CNY;
        // pricing.toml stores the CNY values verbatim (no USD conversion).
        let expected = [
            ("deepseek-v4-pro", 3.0, 6.0, 0.025, 0.0),
            ("deepseek-v4-flash", 1.0, 2.0, 0.02, 0.0),
        ];

        for (name, input_cny, output_cny, cache_read_cny, cache_write_cny) in expected {
            assert!(
                cfg.model.iter().any(|model| {
                    model.name == name
                        && model.input_cny == Some(input_cny)
                        && model.output_cny == Some(output_cny)
                        && model.cache_read_cny == Some(cache_read_cny)
                        && model.cache_write_cny == Some(cache_write_cny)
                }),
                "missing or incorrect current DeepSeek CNY rates for {name}"
            );
        }
    }

    #[test]
    fn project_pricing_toml_has_commandcode_deepseek_flash_peak_schedule() {
        let cfg: PricingConfig = toml::from_str(include_str!("../pricing.toml"))
            .expect("backend/pricing.toml should parse as PricingConfig");
        let flash_segments: Vec<_> = cfg
            .model
            .iter()
            .filter(|model| model.name == "cc:deepseek-v4-flash")
            .collect();

        assert_eq!(
            flash_segments.len(),
            2,
            "expected baseline and dated Flash rates"
        );
        assert!(flash_segments.iter().any(|model| {
            model.effective_from.is_none()
                && model.input == 0.14
                && model.output == 0.28
                && model.cache_read == 0.0028
        }));

        let current = flash_segments
            .iter()
            .find(|model| model.effective_from.as_deref() == Some("2026-08-16T16:00:00Z"))
            .expect("missing current Command Code Flash segment");
        assert_eq!(current.peak_hours_utc, vec![[1, 4], [6, 10]]);
        assert_eq!(current.input, 0.22);
        assert_eq!(current.output, 0.66);
        assert_eq!(current.cache_read, 0.007);
        assert_eq!(current.peak_input, Some(0.44));
        assert_eq!(current.peak_output, Some(1.32));
        assert_eq!(current.peak_cache_read, Some(0.014));
    }

    /// DeepSeek V4.1 Flash (Command Code, added 2026-09-09) must carry the Go
    /// plan rates — without this entry `cc-proxy` rows fall through to the
    /// "no price" branch and render as N/A, dropping out of cost totals. Its
    /// peak windows are weekday-only per the official model page.
    #[test]
    fn project_pricing_toml_has_commandcode_deepseek_v41_flash() {
        let cfg: PricingConfig = toml::from_str(include_str!("../pricing.toml"))
            .expect("backend/pricing.toml should parse as PricingConfig");

        let entry = cfg
            .model
            .iter()
            .find(|model| model.name == "cc:deepseek-v4.1-flash")
            .expect("missing cc:deepseek-v4.1-flash price entry");

        assert_eq!(entry.input, 0.15);
        assert_eq!(entry.output, 0.60);
        assert_eq!(entry.cache_read, 0.003);
        assert_eq!(entry.peak_hours_utc, vec![[1, 4], [6, 10]]);
        assert!(entry.peak_weekdays_only);
        assert_eq!(entry.peak_input, Some(0.30));
        assert_eq!(entry.peak_output, Some(1.20));
        assert_eq!(entry.peak_cache_read, Some(0.006));
    }

    /// End-to-end: a `cc-proxy` row for `deepseek-v4.1-flash` must produce a
    /// positive CNY cost (not the -1 "unknown" sentinel the dashboard renders
    /// as N/A). Uses the real project pricing.toml.
    #[test]
    fn cc_proxy_deepseek_v41_flash_has_a_resolved_cost() {
        let _guard = pricing_test_guard();
        let prev_env = std::env::var("PRICING_CONFIG").ok();
        let _tmp = load_temp_config(include_bytes!("../pricing.toml"));

        let mut record = make_record("cc-proxy", "commandcode", "deepseek-v4.1-flash", 0, 0.0);
        record.input_tokens = 1_000;
        record.output_tokens = 1_000;
        record.cache_read_tokens = 50_000;
        record.cache_write_tokens = 0;
        record.total_tokens = 52_000;
        record.time = "2026-09-12T00:30:00Z".to_string();

        let cost = display_cost(&record);
        assert!(cost > 0.0, "expected a resolved cost, got {cost}");

        restore_pricing_env(prev_env);
    }

    /// `deepseek-v4.1-flash` must NOT be swallowed by the v4-flash prefix rule
    /// (it used to fall through to the `cc:` fallback and lose its price).
    #[test]
    fn commandcode_v41_flash_resolves_to_its_own_key() {
        assert_eq!(
            normalize_commandcode_model("deepseek-v4.1-flash"),
            "cc:deepseek-v4.1-flash"
        );
        assert_ne!(
            normalize_commandcode_model("deepseek-v4.1-flash"),
            "cc:deepseek-v4-flash"
        );
    }

    #[test]
    fn project_pricing_toml_has_historical_rate_segments() {
        let cfg: PricingConfig = toml::from_str(include_str!("../pricing.toml"))
            .expect("backend/pricing.toml should parse as PricingConfig");

        // 兜底段 + 2026-07-31 段；最新段汇率与当前 usd_to_cny 一致。
        assert_eq!(cfg.usd_to_cny_segments.len(), 2);
        assert_eq!(cfg.usd_to_cny_segments[0].effective_from, None);
        assert_eq!(cfg.usd_to_cny_segments[0].rate, 6.82);
        assert_eq!(
            cfg.usd_to_cny_segments[1].effective_from.as_deref(),
            Some("2026-07-31")
        );
        assert!((cfg.usd_to_cny_segments[1].rate - cfg.usd_to_cny).abs() < 1e-12);
    }

    #[test]
    fn grok_cli_zero_cost_uses_configured_high_tier_price() {
        let _guard = pricing_test_guard();
        let prev_env = std::env::var("PRICING_CONFIG").ok();
        let _tmp = load_temp_config(include_bytes!("../pricing.toml"));

        // 500K uncached input reaches the 200K Grok tier. The canonical
        // ainaba provider applies its current 20x subscription divisor.
        let mut record = make_record("grok-cli", "ainaba", "grok-4.5", 1_000_000, 0.0);
        // 使用最新分段之后的记录时间，命中当前汇率（= PricingConfig 默认）。
        record.time = "2026-08-01T00:00:00Z".to_string();
        let cost = display_cost(&record);
        // high tier: input = 4.105571848 USD/M, output = 12.316715543 USD/M
        let usd = 500_000.0 / 1_000_000.0 * 4.105571848 + 500_000.0 / 1_000_000.0 * 12.316715543;
        let expected = usd * PricingConfig::default().special.ainaba_platform_rate / 20.20202;

        assert!(
            (cost - expected).abs() < 1e-9,
            "grok-cli high-tier cost: expected {}, got {}",
            expected,
            cost
        );

        restore_pricing_env(prev_env);
    }

    #[test]
    fn project_pricing_toml_has_grok_4_6_official_usd_rates() {
        let cfg: PricingConfig = toml::from_str(include_str!("../pricing.toml"))
            .expect("backend/pricing.toml should parse as PricingConfig");

        let base = cfg
            .model
            .iter()
            .find(|m| m.name == "grok-4.6" && m.tier_threshold.is_none())
            .expect("grok-4.6 base tier");
        assert_eq!(base.input, 2.00);
        assert_eq!(base.output, 6.00);
        assert_eq!(base.cache_read, 0.50);
        assert_eq!(base.cache_write, 0.0);

        let high = cfg
            .model
            .iter()
            .find(|m| m.name == "grok-4.6" && m.tier_threshold == Some(200000))
            .expect("grok-4.6 200K tier");
        assert_eq!(high.input, 4.00);
        assert_eq!(high.output, 12.00);
        assert_eq!(high.cache_read, 1.00);
        assert_eq!(high.cache_write, 0.0);
    }

    #[test]
    fn project_pricing_toml_has_grok_4_7_official_usd_rates() {
        let cfg: PricingConfig = toml::from_str(include_str!("../pricing.toml"))
            .expect("backend/pricing.toml should parse as PricingConfig");

        let base = cfg
            .model
            .iter()
            .find(|m| m.name == "grok-4.7" && m.tier_threshold.is_none())
            .expect("grok-4.7 base tier");
        assert_eq!(base.input, 2.00);
        assert_eq!(base.output, 6.00);
        assert_eq!(base.cache_read, 0.50);
        assert_eq!(base.cache_write, 0.0);

        let high = cfg
            .model
            .iter()
            .find(|m| m.name == "grok-4.7" && m.tier_threshold == Some(200000))
            .expect("grok-4.7 200K tier");
        assert_eq!(high.input, 4.00);
        assert_eq!(high.output, 12.00);
        assert_eq!(high.cache_read, 1.00);
        assert_eq!(high.cache_write, 0.0);
    }

    #[test]
    fn grok_46_yai_router_uses_official_usd_fixed_rate_and_ainaba_divisor() {
        let _guard = pricing_test_guard();
        let prev_env = std::env::var("PRICING_CONFIG").ok();
        let _tmp = load_temp_config(include_bytes!("../pricing.toml"));

        // Official list: $2 / $6 / $0.50. Yairouter converts at the fixed
        // platform rate (7.0) and then applies the current ainaba divisor
        // (21.538461538 from 2026-08-02 01:00 +08:00).
        let mut record = make_record("grok-cli", "yai-router", "grok-4.6", 0, 0.0);
        record.input_tokens = 100_000;
        record.output_tokens = 20_000;
        record.cache_read_tokens = 40_000;
        record.cache_write_tokens = 0;
        record.total_tokens = 160_000;
        record.time = "2026-08-13T00:00:00Z".to_string();

        let cost = display_cost(&record);
        let usd = 100_000.0 * 2.00 / 1_000_000.0
            + 20_000.0 * 6.00 / 1_000_000.0
            + 40_000.0 * 0.50 / 1_000_000.0;
        let expected = usd * 7.0 / 21.538461538;
        assert!(
            (cost - expected).abs() < 1e-9,
            "yai-router grok-4.6 base-tier: expected {}, got {}",
            expected,
            cost
        );

        restore_pricing_env(prev_env);
    }

    #[test]
    fn grok_46_yai_router_high_tier_uses_official_long_context_rates() {
        let _guard = pricing_test_guard();
        let prev_env = std::env::var("PRICING_CONFIG").ok();
        let _tmp = load_temp_config(include_bytes!("../pricing.toml"));

        // 200K+ total input (uncached + cache) selects the official long-context
        // tier: $4 / $12 / $1.00. Same fixed-rate / divisor conversion.
        let mut record = make_record("grok-cli", "yai-router", "grok-4.6", 0, 0.0);
        record.input_tokens = 180_000;
        record.output_tokens = 10_000;
        record.cache_read_tokens = 40_000;
        record.cache_write_tokens = 0;
        record.total_tokens = 230_000;
        record.time = "2026-08-13T00:00:00Z".to_string();

        let cost = display_cost(&record);
        let usd = 180_000.0 * 4.00 / 1_000_000.0
            + 10_000.0 * 12.00 / 1_000_000.0
            + 40_000.0 * 1.00 / 1_000_000.0;
        let expected = usd * 7.0 / 21.538461538;
        assert!(
            (cost - expected).abs() < 1e-9,
            "yai-router grok-4.6 high-tier: expected {}, got {}",
            expected,
            cost
        );

        restore_pricing_env(prev_env);
    }

    #[test]
    fn dim_grok_build_uses_grok_subscription_divisor() {
        let _guard = pricing_test_guard();
        let prev_env = std::env::var("PRICING_CONFIG").ok();
        let _tmp = load_temp_config(include_bytes!("../pricing.toml"));

        // Dim's Grok Build channel is billed against the SuperGrok
        // subscription: official USD list price × rate ÷ grok_divisor.
        // The stored catalog cost (API list price) must NOT be used.
        let mut record = make_record("dim", "grok-build", "grok-4.6", 0, 1.712644);
        record.input_tokens = 296_933;
        record.output_tokens = 12_671;
        record.cache_read_tokens = 2_085_504;
        record.cache_write_tokens = 0;
        record.total_tokens = 2_395_108;
        record.time = "2026-09-06T14:15:21.333Z".to_string();

        let cost = display_cost(&record);
        // 2.4M total input → high tier ($4 / $12 / $1.00).
        let usd = 296_933.0 * 4.00 / 1_000_000.0
            + 12_671.0 * 12.00 / 1_000_000.0
            + 2_085_504.0 * 1.00 / 1_000_000.0;
        // The grok divisor may be overridden by subscription_settings.json
        // (settings drawer); use the live configured value.
        let divisor = crate::pricing::get_config().special.grok_divisor;
        let expected = usd * 6.7894 / divisor;
        assert!(
            (cost - expected).abs() < 1e-9,
            "dim grok-build: expected {}, got {}",
            expected,
            cost
        );

        // The same record after the provider mapping (grok-build → xai-official)
        // must produce the identical cost.
        let mut mapped = record.clone();
        mapped.provider = "xai-official".to_string();
        let cost_mapped = display_cost(&mapped);
        assert!(
            (cost_mapped - expected).abs() < 1e-9,
            "dim xai-official: expected {}, got {}",
            expected,
            cost_mapped
        );

        restore_pricing_env(prev_env);
    }

    #[test]
    fn zcode_zero_cost_uses_model_price_with_opencode_divisor() {
        let _guard = pricing_test_guard();
        let prev_env = std::env::var("PRICING_CONFIG").ok();
        let _tmp = load_temp_config(include_bytes!("../pricing.toml"));

        // ZCode records are billed through the OpenCode Go subscription
        // (provider_metadata_json = {"OpenCodeGo": {}}), so the official
        // CNY price (deepseek) is divided by the opencode plan divisor.
        let mut record = make_record("zcode", "opencode-go", "deepseek-v4-flash", 1_000_000, 0.0);
        record.input_tokens = 15276;
        record.output_tokens = 1048;
        record.cache_read_tokens = 17664;
        record.cache_write_tokens = 0;
        record.time = "2026-08-01T00:00:00Z".to_string();
        let cost = display_cost(&record);
        let expected = (15276.0 / 1_000_000.0 * 1.0 // input_cny
            + 1048.0 / 1_000_000.0 * 2.0 // output_cny
            + 17664.0 / 1_000_000.0 * 0.02) // cache_read_cny
            / 6.0; // opencode_divisor
        assert!(
            (cost - expected).abs() < 1e-9,
            "zcode cost: expected {}, got {}",
            expected,
            cost
        );

        restore_pricing_env(prev_env);
    }

    #[test]
    fn zcode_bigmodel_credit_cost_follows_plan_formula() {
        let _guard = pricing_test_guard();
        let prev_env = std::env::var("PRICING_CONFIG").ok();
        let _tmp = load_temp_config(include_bytes!("../pricing.toml"));

        // BigModel GLM Coding Plan records (provider=bigmodel) are billed by
        // plan credits: 积分 = (输入×2.3 + 缓存×0.56 + 输出×8)/10000 × 高峰因子，
        // 每积分 ¥0.00144646（实付分摊：¥188.04 / 13周×10000 积分）。
        // 2026-09-12 对 live quota API 验证：3768.7 vs 3774 积分（-0.14%）。
        let mut record = make_record("zcode", "bigmodel", "GLM-5.3-Flash", 1_000_000, 0.0);
        record.input_tokens = 2_251_621; // non-cache input (verified convention)
        record.output_tokens = 635_525;
        record.cache_read_tokens = 113_710_464;
        record.cache_write_tokens = 0;
        // Saturday 2026-09-12 20:00 CST = 12:00 UTC → off-peak (0.5×)
        record.time = "2026-09-12T12:00:00Z".to_string();
        let cost = display_cost(&record);
        let credits = (2_251_621.0 * 2.3 + 113_710_464.0 * 0.56 + 635_525.0 * 8.0) / 10_000.0;
        let expected = credits * 0.5 * 0.0014464615384615384;
        assert!(
            (cost - expected).abs() < 1e-9,
            "zcode bigmodel off-peak: expected {}, got {}",
            expected,
            cost
        );

        // Weekday peak: Monday 2026-09-14 15:00 CST = 07:00 UTC → 1×
        let mut peak = record.clone();
        peak.time = "2026-09-14T07:00:00Z".to_string();
        let peak_cost = display_cost(&peak);
        let expected_peak = credits * 1.0 * 0.0014464615384615384;
        assert!(
            (peak_cost - expected_peak).abs() < 1e-9,
            "zcode bigmodel peak: expected {}, got {}",
            expected_peak,
            peak_cost
        );

        // 夜间畅用活动对旧版客户端不生效：cutoff（2026-09-13 06:15 CST）之前
        // 落在窗口内的记录按正常波谷 0.5× 计费，而不是 0。
        // 2026-09-12 23:30 CST = 15:30 UTC → 0.5×（非高峰）
        let mut pre_promo = record.clone();
        pre_promo.time = "2026-09-12T15:30:00Z".to_string();
        let pre_promo_cost = display_cost(&pre_promo);
        let expected_pre = credits * 0.5 * 0.0014464615384615384;
        assert!(
            (pre_promo_cost - expected_pre).abs() < 1e-9,
            "zcode bigmodel night before promo effective: expected {}, got {}",
            expected_pre,
            pre_promo_cost
        );

        // 活动生效后（≥ 2026-09-13 06:15 CST）窗口内消耗为 0：
        // 2026-09-13 07:30 CST = 2026-09-12T23:30Z
        let mut night = record.clone();
        night.time = "2026-09-12T23:30:00Z".to_string();
        let night_cost = display_cost(&night);
        assert!(
            (night_cost - 0.0).abs() < 1e-9,
            "zcode bigmodel night free: expected 0, got {}",
            night_cost
        );

        // 边界：恰好 2026-09-13 06:15:00 CST 起享受免扣
        let mut boundary = record.clone();
        boundary.time = "2026-09-12T22:15:00Z".to_string();
        assert!(
            (display_cost(&boundary) - 0.0).abs() < 1e-9,
            "zcode bigmodel promo boundary 06:15 CST should be free"
        );

        // 活动结束后（2026-09-25）夜间不再免费：23:30 CST 属于非高峰 → 0.5×
        let mut after = record.clone();
        after.time = "2026-09-25T15:30:00Z".to_string();
        let after_cost = display_cost(&after);
        let expected_after = credits * 0.5 * 0.0014464615384615384;
        assert!(
            (after_cost - expected_after).abs() < 1e-9,
            "zcode bigmodel after promo: expected {}, got {}",
            expected_after,
            after_cost
        );

        // 夜间畅用仅限 GLM-5.3-Flash：GLM-5.3 在同一免扣窗口内（活动生效后）
        // 照常按波谷 0.5× 扣积分，系数 6.9/1.7/24。
        let mut nonflash = make_record("zcode", "bigmodel", "GLM-5.3", 0, 0.0);
        nonflash.input_tokens = 1_000_000;
        nonflash.output_tokens = 500_000;
        nonflash.cache_read_tokens = 2_000_000;
        nonflash.cache_write_tokens = 0;
        nonflash.time = "2026-09-12T23:30:00Z".to_string(); // 2026-09-13 07:30 CST，窗口内
        let nonflash_cost = display_cost(&nonflash);
        let nf_credits = (1_000_000.0 * 6.9 + 2_000_000.0 * 1.7 + 500_000.0 * 24.0) / 10_000.0;
        let expected_nf = nf_credits * 0.5 * 0.0014464615384615384;
        assert!(
            (nonflash_cost - expected_nf).abs() < 1e-9,
            "zcode bigmodel glm-5.3 night should NOT be free: expected {}, got {}",
            expected_nf,
            nonflash_cost
        );

        // Unconfigured models (e.g. the free tokenrouter model) still fall
        // through to model-price pricing (0 here), not the credit formula.
        let mut free = make_record("zcode", "tokenrouter", "z-ai/glm-5.3-free", 0, 0.0);
        free.input_tokens = 15276;
        free.output_tokens = 1048;
        free.cache_read_tokens = 17664;
        free.time = "2026-09-12T12:00:00Z".to_string();
        let free_cost = display_cost(&free);
        assert!(
            (free_cost - 0.0).abs() < 1e-9,
            "zcode free model: expected 0, got {}",
            free_cost
        );

        restore_pricing_env(prev_env);
    }

    #[test]
    fn dim_console_api_cost_estimated_from_cny_model_price() {
        let _guard = pricing_test_guard();
        let prev_env = std::env::var("PRICING_CONFIG").ok();
        let _tmp = load_temp_config(include_bytes!("../pricing.toml"));

        // Console-API dim records carry no stored cost; the derived-source
        // branch prices them from pricing.toml dim_model table (DimAgent
        // 平台积分价换算 CNY — vision-exp 非高峰费率).
        let mut record =
            make_record("dim", "dim", "deepseek-v4-flash-vision-exp", 1_000_000, 0.0);
        record.input_tokens = 15276;
        record.output_tokens = 1048;
        record.cache_read_tokens = 17664;
        record.cache_write_tokens = 0;
        record.time = "2026-08-01T00:00:00Z".to_string();
        let cost = display_cost(&record);
        let expected = 15276.0 / 1_000_000.0 * 0.653798 // input_cny
            + 1048.0 / 1_000_000.0 * 3.922790 // output_cny
            + 17664.0 / 1_000_000.0 * 0.043587; // cache_read_cny
        assert!(
            (cost - expected).abs() < 1e-9,
            "dim cost: expected {}, got {}",
            expected,
            cost
        );

        // Peak hours (CST 09:00–12:00 / 14:00–18:00 = UTC 01:00–04:00 /
        // 06:00–10:00) bill vision-exp at double the base rates.
        let mut peak_record = record.clone();
        peak_record.time = "2026-09-02T02:00:00Z".to_string(); // UTC 02:00 = CST 10:00 高峰
        let peak_cost = display_cost(&peak_record);
        let expected_peak = 15276.0 / 1_000_000.0 * 1.307597 // peak_input_cny
            + 1048.0 / 1_000_000.0 * 7.845579 // peak_output_cny
            + 17664.0 / 1_000_000.0 * 0.087173; // peak_cache_read_cny
        assert!(
            (peak_cost - expected_peak).abs() < 1e-9,
            "dim peak cost: expected {}, got {}",
            expected_peak,
            peak_cost
        );
        // Off-peak same model at UTC 12:00 (CST 20:00) keeps base rates.
        let mut offpeak_record = record.clone();
        offpeak_record.time = "2026-09-02T12:00:00Z".to_string();
        let offpeak_cost = display_cost(&offpeak_record);
        assert!(
            (offpeak_cost - expected).abs() < 1e-9,
            "dim off-peak cost: expected {}, got {}",
            expected,
            offpeak_cost
        );

        restore_pricing_env(prev_env);
    }

    #[test]
    fn dim_cost_multiplies_list_price_by_entitlement_rate() {
        let _guard = pricing_test_guard();
        let prev_env = std::env::var("PRICING_CONFIG").ok();
        let _tmp = load_temp_config(include_bytes!("../pricing.toml"));
        // pricing.toml now carries platform *list* prices only; these are the
        // rates the account's entitlement payload reports, recorded from the
        // moment each was first seen.
        use crate::dim_entitlement::{RateSegment, build_table};
        let segment = |effective_from: &str, rate: f64, windows: Vec<_>| RateSegment {
            effective_from: effective_from.to_string(),
            rate,
            windows,
        };
        set_dim_entitlement_rates(build_table(&[
            (
                "deepseek-v4.1-flash",
                vec![segment("2026-09-18T18:00:00+08:00", 0.65, vec![])],
            ),
            (
                "glm-5.3",
                vec![segment(
                    "2026-01-01T00:00:00+08:00",
                    0.7,
                    vec![crate::dim_entitlement::RateWindow {
                        start: "20:00".to_string(),
                        end: "08:00".to_string(),
                        rate: 0.5,
                    }],
                )],
            ),
        ]));

        let dim_record = |model: &str, time: &str| {
            let mut record = make_record("dim", "dim", model, 0, 0.0);
            record.input_tokens = 1_000_000;
            record.output_tokens = 0;
            record.cache_read_tokens = 0;
            record.cache_write_tokens = 0;
            record.total_tokens = 1_000_000;
            record.time = time.to_string();
            record
        };

        // Before the platform's 0.65 cut: DeepSeek list price, nothing applied.
        let before = display_cost(&dim_record("deepseek-v4.1-flash", "2026-09-17T12:00:00Z"));
        assert!(
            (before - 0.890909).abs() < 1e-9,
            "pre-discount dim cost: expected list price, got {before}"
        );
        // After it: 1M input at 0.890909 × 0.65 (Monday, CST 20:00 = off peak).
        let after = display_cost(&dim_record("deepseek-v4.1-flash", "2026-09-21T12:00:00Z"));
        assert!(
            (after - 0.890909 * 0.65).abs() < 1e-9,
            "discounted dim cost: expected {}, got {after}",
            0.890909 * 0.65
        );
        // The price card claims 忙时 runs Mon–Fri, but the platform doubles it on
        // weekends too (daily-stats base_amount_minor for 2026-08-23 / 09-05 /
        // 09-12 / 09-13 only closes that way), so a Sunday morning in the window
        // bills peak rates with the discount on top — weekday and weekend agree.
        let weekend_peak_hour =
            display_cost(&dim_record("deepseek-v4.1-flash", "2026-09-20T02:00:00Z"));
        let weekday_peak_hour =
            display_cost(&dim_record("deepseek-v4.1-flash", "2026-09-21T02:00:00Z"));
        assert!(
            (weekday_peak_hour - 1.781818 * 0.65).abs() < 1e-9,
            "忙时 cost: expected {}, got {weekday_peak_hour}",
            1.781818 * 0.65
        );
        assert!(
            (weekend_peak_hour - weekday_peak_hour).abs() < 1e-12,
            "weekend must bill 忙时 like weekdays: {weekend_peak_hour} vs {weekday_peak_hour}"
        );
        // glm-5.3: list 5.345455 × 0.7 by day, and × 0.5 more inside the
        // 20:00–08:00 CST 夜间优惠 window — the same totals the previously
        // hard-coded discounted rates produced.
        let glm_day = display_cost(&dim_record("glm-5.3", "2026-09-19T03:00:00Z"));
        assert!(
            (glm_day - 3.741818).abs() < 1e-6,
            "glm-5.3 daytime: expected 3.741818, got {glm_day}"
        );
        let glm_night = display_cost(&dim_record("glm-5.3", "2026-09-19T13:00:00Z"));
        assert!(
            (glm_night - 1.870909).abs() < 1e-6,
            "glm-5.3 21:00 CST: expected 1.870909, got {glm_night}"
        );

        set_dim_entitlement_rates(std::collections::HashMap::new());
        restore_pricing_env(prev_env);
    }

    #[test]
    fn zcode_free_model_costs_zero() {
        let _guard = pricing_test_guard();
        let prev_env = std::env::var("PRICING_CONFIG").ok();
        let _tmp = load_temp_config(include_bytes!("../pricing.toml"));

        // z-ai/glm-5.3-free is billed via Tokenrouter and priced at 0 in
        // pricing.toml → display cost is exactly 0 (free), not N/A.
        let mut record = make_record("zcode", "tokenrouter", "z-ai/glm-5.3-free", 0, 0.0);
        record.input_tokens = 15276;
        record.output_tokens = 1048;
        record.cache_read_tokens = 17664;
        record.cache_write_tokens = 0;
        record.total_tokens = 33988;
        record.time = "2026-08-01T00:00:00Z".to_string();
        assert_eq!(display_cost(&record), 0.0);

        restore_pricing_env(prev_env);
    }

    #[test]
    fn unknown_model_cost_is_negative_sentinel() {
        let _guard = pricing_test_guard();
        let prev_env = std::env::var("PRICING_CONFIG").ok();
        let _tmp = load_temp_config(include_bytes!("../pricing.toml"));

        // No pricing.toml entry → -1 signals "unknown cost" (frontend N/A).
        let mut record = make_record("zcode", "tokenrouter", "some-unknown-model", 0, 0.0);
        record.input_tokens = 100;
        record.output_tokens = 20;
        record.total_tokens = 120;
        record.time = "2026-08-01T00:00:00Z".to_string();
        assert_eq!(display_cost(&record), -1.0);

        // Pi records keep 0 (stored cost is real; 0 = free).
        let mut pi = make_record("pi", "openai", "some-unknown-model", 0, 0.0);
        pi.input_tokens = 100;
        pi.output_tokens = 20;
        pi.total_tokens = 120;
        pi.time = "2026-08-01T00:00:00Z".to_string();
        assert_eq!(display_cost(&pi), 0.0);

        restore_pricing_env(prev_env);
    }

    #[test]
    fn dsh_deepseek_zero_cost_computes_in_cny() {
        let _guard = pricing_test_guard();
        let prev_env = std::env::var("PRICING_CONFIG").ok();
        let _tmp = load_temp_config(include_bytes!("../pricing.toml"));

        // DSH records with provider "deepseek-official" are merged to
        // "deepseek" by vendor_merge.toml (original_provider preserved).
        // They carry no stored cost, so display_cost must derive the official
        // DeepSeek CNY price from tokens (no USD→CNY conversion, no divisor).
        let mut record = make_record("dsh", "deepseek", "deepseek-v4-pro", 0, 0.0);
        record.original_provider = Some("deepseek-official".to_string());
        record.input_tokens = 653;
        record.output_tokens = 17287;
        record.cache_read_tokens = 50432;
        record.cache_write_tokens = 0;
        record.total_tokens = 68372;
        record.time = "2026-08-13T23:11:54.627+00:00".to_string();
        let cost = display_cost(&record);
        let expected = 653.0 / 1_000_000.0 * 3.0 // input_cny
            + 17287.0 / 1_000_000.0 * 6.0 // output_cny
            + 50432.0 / 1_000_000.0 * 0.025; // cache_read_cny
        assert!(
            cost > 0.0,
            "dsh deepseek record should have non-zero cost, got {}",
            cost
        );
        assert!(
            (cost - expected).abs() < 1e-9,
            "dsh deepseek cost: expected {}, got {}",
            expected,
            cost
        );

        restore_pricing_env(prev_env);
    }

    #[test]
    fn dsh_opencode_go_uses_opencode_divisor() {
        let _guard = pricing_test_guard();
        let prev_env = std::env::var("PRICING_CONFIG").ok();
        let _tmp = load_temp_config(include_bytes!("../pricing.toml"));

        // DSH records routed through the OpenCode Go provider are billed via
        // the OpenCode Go subscription, so the official CNY price is divided
        // by the opencode plan divisor (same as zcode records).
        let mut record = make_record("dsh", "opencode-go", "deepseek-v4-flash", 0, 0.0);
        record.input_tokens = 15276;
        record.output_tokens = 1048;
        record.cache_read_tokens = 17664;
        record.cache_write_tokens = 0;
        record.total_tokens = 33988;
        record.time = "2026-08-01T00:00:00Z".to_string();
        let cost = display_cost(&record);
        let expected = (15276.0 / 1_000_000.0 * 1.0 // input_cny
            + 1048.0 / 1_000_000.0 * 2.0 // output_cny
            + 17664.0 / 1_000_000.0 * 0.02) // cache_read_cny
            / 6.0; // opencode_divisor
        assert!(
            (cost - expected).abs() < 1e-9,
            "dsh opencode-go cost: expected {}, got {}",
            expected,
            cost
        );

        restore_pricing_env(prev_env);
    }

    fn opencode_deepseek_segment_config() -> Vec<u8> {
        br#"
usd_to_cny = 6.7894
rate_date = "2026-07-31"

[special]
xunfei_per_call = 0.002211111111
kimi_per_token = 0.000000071071429
opencode_divisor = 6.0
ainaba_divisor = 1.0
freemodel_divisor = 67.894
opencode_model_segments = [
    { models = ["deepseek-v4-flash", "deepseek-v4-pro"], effective_from = "2026-08-18T00:00:00+08:00", divisor = 3.0 },
]

[[model]]
name = "deepseek-v4-flash"
input_cny = 1.0
output_cny = 2.0
cache_read_cny = 0.02
cache_write_cny = 0.0

[[model]]
name = "deepseek-v4-pro"
input_cny = 3.0
output_cny = 6.0
cache_read_cny = 0.025
cache_write_cny = 0.0
"#
        .to_vec()
    }

    fn opencode_stored_cost(record_time: &str, model: &str, usd_cost: f64) -> TokenRecord {
        let mut record = make_record("pi", "opencode-go", model, 1_000, usd_cost);
        record.time = record_time.to_string();
        record
    }

    #[test]
    fn opencode_deepseek_keeps_old_divisor_before_quota_cut() {
        let _guard = pricing_test_guard();
        let prev_env = std::env::var("PRICING_CONFIG").ok();
        let _tmp = load_temp_config(&opencode_deepseek_segment_config());

        let record = opencode_stored_cost("2026-08-17T23:59:59+08:00", "deepseek-v4-flash", 1.0);
        let expected = 1.0 / 6.0 * 6.7894;
        let cost = display_cost(&record);
        assert!(
            (cost - expected).abs() < 1e-9,
            "DeepSeek OpenCode cost before 2026-08-18 00:00 CST should use /6, expected {}, got {}",
            expected,
            cost
        );

        restore_pricing_env(prev_env);
    }

    #[test]
    fn opencode_deepseek_uses_halved_quota_from_beijing_cutoff() {
        let _guard = pricing_test_guard();
        let prev_env = std::env::var("PRICING_CONFIG").ok();
        let _tmp = load_temp_config(&opencode_deepseek_segment_config());

        for (time, model) in [
            ("2026-08-18T00:00:00+08:00", "deepseek-v4-flash"),
            ("2026-08-18T00:00:00+08:00", "deepseek-v4-pro"),
            ("2026-08-17T16:00:00Z", "deepseek-v4-flash"),
            ("2026-08-20T12:00:00+08:00", "opencode-go/deepseek-v4-pro"),
        ] {
            let record = opencode_stored_cost(time, model, 1.0);
            let expected = 1.0 / 3.0 * 6.7894;
            let cost = display_cost(&record);
            assert!(
                (cost - expected).abs() < 1e-9,
                "DeepSeek OpenCode cost at {} ({}) should use /3 after $30 quota, expected {}, got {}",
                time,
                model,
                expected,
                cost
            );
        }

        restore_pricing_env(prev_env);
    }

    #[test]
    fn opencode_non_deepseek_keeps_plan_divisor_after_cutoff() {
        let _guard = pricing_test_guard();
        let prev_env = std::env::var("PRICING_CONFIG").ok();
        let _tmp = load_temp_config(&opencode_deepseek_segment_config());

        let record = opencode_stored_cost("2026-08-18T00:00:00+08:00", "kimi-k2.6", 1.0);
        let expected = 1.0 / 6.0 * 6.7894;
        let cost = display_cost(&record);
        assert!(
            (cost - expected).abs() < 1e-9,
            "non-DeepSeek OpenCode models should keep /6 after the DeepSeek quota cut, expected {}, got {}",
            expected,
            cost
        );

        restore_pricing_env(prev_env);
    }

    #[test]
    fn opencode_source_and_derived_paths_use_dated_deepseek_divisor() {
        let _guard = pricing_test_guard();
        let prev_env = std::env::var("PRICING_CONFIG").ok();
        let _tmp = load_temp_config(&opencode_deepseek_segment_config());

        let mut opencode_db =
            make_record("opencode", "opencode-go", "deepseek-v4-flash", 1_000, 1.2);
        opencode_db.time = "2026-08-18T01:00:00+08:00".to_string();
        let opencode_expected = 1.2 / 3.0 * 6.7894;
        let opencode_cost = display_cost(&opencode_db);
        assert!(
            (opencode_cost - opencode_expected).abs() < 1e-9,
            "source=opencode DeepSeek after cutoff should use /3, expected {}, got {}",
            opencode_expected,
            opencode_cost
        );

        let mut zcode = make_record("zcode", "opencode-go", "deepseek-v4-flash", 0, 0.0);
        zcode.input_tokens = 1_000_000;
        zcode.output_tokens = 0;
        zcode.cache_read_tokens = 0;
        zcode.cache_write_tokens = 0;
        zcode.total_tokens = 1_000_000;
        zcode.time = "2026-08-18T00:00:00+08:00".to_string();
        // CNY list: 1.0 yuan / 1M input, then /3 after the quota cut.
        let zcode_expected = 1.0 / 3.0;
        let zcode_cost = display_cost(&zcode);
        assert!(
            (zcode_cost - zcode_expected).abs() < 1e-9,
            "zcode DeepSeek after cutoff should use /3, expected {}, got {}",
            zcode_expected,
            zcode_cost
        );

        restore_pricing_env(prev_env);
    }

    #[test]
    fn kimi_cli_zero_cost_uses_model_aware_subscription_estimate() {
        let _guard = pricing_test_guard();
        // kimi-cli records have cost=0 and provider="kimi"
        let record = make_record("kimi-cli", "kimi", "kimi-k2.6", 1_000_000, 0.0);
        let cost = display_cost(&record);
        let expected = (500_000.0 * 6.5 + 500_000.0 * 27.0) / 1_000_000.0 / 20.0;
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
    fn pi_kimi_zero_cost_uses_model_aware_subscription_estimate() {
        let _guard = pricing_test_guard();
        // Pi-sourced kimi records with cost=0 should use the same formula.
        let record = make_record("pi", "kimi", "kimi-k2.6", 1_000_000, 0.0);
        let cost = display_cost(&record);
        let expected = (500_000.0 * 6.5 + 500_000.0 * 27.0) / 1_000_000.0 / 20.0;
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
    fn kimi_coding_subscription_uses_model_aware_subscription_estimate() {
        let _guard = pricing_test_guard();
        // Records from kimi-coding provider (subscription) with cost>0 should
        // use the model-aware subscription estimate, NOT the stored API cost.
        // This matches kimi-code behavior (same subscription model).
        let mut record = make_record("pi", "kimi", "kimi-for-coding", 1_000_000, 0.05);
        record.original_provider = Some("kimi-coding".to_string());
        let cost = display_cost(&record);
        let expected = (500_000.0 * 6.5 + 500_000.0 * 27.0) / 1_000_000.0 / 20.0;
        assert!(
            (cost - expected).abs() < 1e-9,
            "kimi-coding should use model-aware estimate, expected {}, got {}",
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
        // cost is in USD, so should be converted to CNY (0.05 * 6.7894)
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
    fn grok_cli_zero_cost_uses_grok_model_pricing() {
        let _guard = pricing_test_guard();
        let previous_config = std::env::var("PRICING_CONFIG").ok();
        let _tmp = load_temp_config(
            br#"
usd_to_cny = 6.7894
rate_date = "2026-07-31"

[special]
xunfei_per_call = 0.002211111111
kimi_per_token = 0.000000071071429
opencode_divisor = 6.0
ainaba_divisor = 40.0
freemodel_divisor = 67.894

[[model]]
name = "grok-4.5"
input = 2.0
output = 6.0
cache_read = 0.5
cache_write = 0.0
"#,
        );
        let record = make_record("grok-cli", "xai-official", "grok-4.5", 1_000_000, 0.0);

        assert!(
            display_cost(&record) > 0.0,
            "Grok CLI records should derive cost from pricing.toml"
        );
        restore_pricing_env(previous_config);
    }

    #[test]
    fn freemodel_stored_cost_applies_divisor() {
        let _guard = pricing_test_guard();
        // FreeModel records with stored cost (USD) should apply the 67.894x divisor
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
        // and then apply the 67.894x divisor.
        // The default PricingConfig has an empty model list, so derived-cost
        // calculation cannot resolve model prices. We write a temp config with
        // model prices so resolve_model_price() can find claude-opus-4-7.
        use std::io::Write;
        let tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.as_file()
            .write_all(
                br#"
usd_to_cny = 6.7894
rate_date = "2026-07-31"

[special]
xunfei_per_call = 0.002211111111
kimi_per_token = 0.000000071071429
opencode_divisor = 6.0
ainaba_divisor = 40.0
freemodel_divisor = 67.894

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
        unsafe { std::env::set_var("PRICING_CONFIG", tmp.path().to_str().unwrap()) };
        reload();

        let mut record = make_record("claude-code", "FreeModel", "claude-opus-4-7", 10_000, 0.0);
        record.input_tokens = 5_000;
        record.output_tokens = 5_000;
        record.cache_read_tokens = 0;
        record.cache_write_tokens = 0;
        let cost = display_cost(&record);
        // claude-opus-4-7: input=$5/M, output=$25/M
        // usd = 5000*5/1M + 5000*25/1M = 0.025 + 0.125 = 0.15
        // cny = 0.15 * 6.7894 / 67.894 = 0.015
        let usd = 5_000.0 * 5.0 / 1_000_000.0 + 5_000.0 * 25.0 / 1_000_000.0;
        let expected = usd * 6.7894 / 67.894;
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
        unsafe { std::env::set_var("PRICING_CONFIG", restore_tmp.path().to_str().unwrap()) };
        reload();

        // Restore env var
        match prev_env {
            Some(v) => unsafe { std::env::set_var("PRICING_CONFIG", v) },
            None => unsafe { std::env::remove_var("PRICING_CONFIG") },
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
usd_to_cny = 6.7894
rate_date = "2026-07-31"

[special]
xunfei_per_call = 0.002211111111
kimi_per_token = 0.000000071071429
opencode_divisor = 6.0
ainaba_divisor = 40.0
freemodel_divisor = 67.894

[[model]]
name = "deepseek-v4-pro"
input_cny = 3.0
output_cny = 6.0
cache_read_cny = 0.025
cache_write_cny = 0.0
"#,
            )
            .unwrap();

        let prev_env = std::env::var("PRICING_CONFIG").ok();
        unsafe { std::env::set_var("PRICING_CONFIG", tmp.path().to_str().unwrap()) };
        reload();

        let mut record = make_record("deepseek-ai", "deepseek", "deepseek-v4-pro", 0, 0.0);
        record.input_tokens = 1_000_000;
        record.output_tokens = 100_000;
        record.cache_read_tokens = 500_000;
        record.cache_write_tokens = 0;
        record.total_tokens = 1_600_000;

        let cny = display_cost(&record);

        // DeepSeek 官方 CNY 定价，直接产出人民币：
        // input=3/M, output=6/M, cache_read=0.025/M
        let expected = 1_000_000.0 * 3.0 / 1_000_000.0
            + 100_000.0 * 6.0 / 1_000_000.0
            + 500_000.0 * 0.025 / 1_000_000.0;

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
            Some(v) => unsafe { std::env::set_var("PRICING_CONFIG", v) },
            None => unsafe { std::env::remove_var("PRICING_CONFIG") },
        }
        reload();
    }

    // ─── Segmented USD→CNY rate tests ────────────────────────────────────

    /// Temp config with two rate segments (baseline 6.82 + 2026-07-31 6.7894).
    fn segmented_rate_config() -> Vec<u8> {
        br#"
usd_to_cny = 6.7894
rate_date = "2026-07-31"

[[usd_to_cny_segments]]
rate = 6.82

[[usd_to_cny_segments]]
effective_from = "2026-07-31"
rate = 6.7894

[special]
xunfei_per_call = 0.002211111111
kimi_per_token = 0.000000071071429
opencode_divisor = 6.0
ainaba_divisor = 1.0
freemodel_divisor = 67.894
fenno_divisor = 101.841
"#
        .to_vec()
    }

    #[test]
    fn segmented_rate_selects_by_record_time() {
        let _guard = pricing_test_guard();
        let prev_env = std::env::var("PRICING_CONFIG").ok();
        let _tmp = load_temp_config(&segmented_rate_config());

        // Unknown Pi provider with stored USD cost → convert at the record's
        // segment rate.
        let mut early = make_record("pi", "unknown-vendor", "some-model", 1_000, 1.0);
        early.time = "2026-07-01T00:00:00Z".to_string();
        let early_cost = display_cost(&early);
        assert!(
            (early_cost - 6.82).abs() < 1e-9,
            "records before 2026-07-31 should use baseline 6.82, got {}",
            early_cost
        );

        let mut latest = make_record("pi", "unknown-vendor", "some-model", 1_000, 1.0);
        latest.time = "2026-08-01T00:00:00Z".to_string();
        let latest_cost = display_cost(&latest);
        assert!(
            (latest_cost - 6.7894).abs() < 1e-9,
            "records after 2026-07-31 should use 6.7894, got {}",
            latest_cost
        );

        // current_rate() 返回最新分段汇率。
        assert!((current_rate() - 6.7894).abs() < 1e-12);

        restore_pricing_env(prev_env);
    }

    #[test]
    fn segmented_rate_keeps_freemodel_invariant() {
        let _guard = pricing_test_guard();
        let prev_env = std::env::var("PRICING_CONFIG").ok();
        let _tmp = load_temp_config(&segmented_rate_config());

        // FreeModel: 1 USD 面值 = 0.1 CNY，跨分段成本应保持不变（0.5 × 0.1）。
        let mut early = make_record("pi", "FreeModel", "claude-opus-4-7", 1_000, 0.5);
        early.time = "2026-07-01T00:00:00Z".to_string();
        let mut latest = make_record("pi", "FreeModel", "claude-opus-4-7", 1_000, 0.5);
        latest.time = "2026-08-01T00:00:00Z".to_string();

        let early_cost = display_cost(&early);
        let latest_cost = display_cost(&latest);
        assert!(
            (early_cost - 0.05).abs() < 1e-9,
            "FreeModel early segment should stay 0.05, got {}",
            early_cost
        );
        assert!(
            (latest_cost - 0.05).abs() < 1e-9,
            "FreeModel latest segment should stay 0.05, got {}",
            latest_cost
        );

        restore_pricing_env(prev_env);
    }

    #[test]
    fn segmented_rate_deepseek_cny_ignores_rate() {
        let _guard = pricing_test_guard();
        let prev_env = std::env::var("PRICING_CONFIG").ok();
        let mut cfg = segmented_rate_config();
        cfg.extend_from_slice(
            br#"
[[model]]
name = "deepseek-v4-pro"
input_cny = 3.0
output_cny = 6.0
cache_read_cny = 0.025
cache_write_cny = 0.0
"#,
        );
        let _tmp = load_temp_config(&cfg);

        // DeepSeek 官方 CNY 定价：1M input 在任何分段都直接产出 ¥3，与汇率无关。
        // make_record 将 total 对半分：500K input × 3/M + 500K output × 6/M = ¥4.5
        let mut early = make_record("deepseek-ai", "deepseek", "deepseek-v4-pro", 1_000_000, 0.0);
        early.time = "2026-07-01T00:00:00Z".to_string();
        let mut latest = make_record("deepseek-ai", "deepseek", "deepseek-v4-pro", 1_000_000, 0.0);
        latest.time = "2026-08-01T00:00:00Z".to_string();

        let early_cost = display_cost(&early);
        let latest_cost = display_cost(&latest);
        assert!(
            (early_cost - 4.5).abs() < 1e-9,
            "deepseek CNY early segment should be ¥4.5, got {}",
            early_cost
        );
        assert!(
            (latest_cost - 4.5).abs() < 1e-9,
            "deepseek CNY latest segment should be ¥4.5, got {}",
            latest_cost
        );

        restore_pricing_env(prev_env);
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
        assert!(
            (cost_with_cache - 10.0).abs() < 0.01,
            "cache should be free, expected 10.0, got {}",
            cost_with_cache
        );

        // Input + output both count
        record.input_tokens = 25_000_000;
        record.output_tokens = 25_000_000;
        record.cache_read_tokens = 0;
        record.total_tokens = 50_000_000;
        let cost_mixed = display_cost(&record);
        assert!(
            (cost_mixed - 10.0).abs() < 0.01,
            "input+output should = 10.0, got {}",
            cost_mixed
        );
    }

    #[test]
    fn tiered_pricing_base_tier_for_short_context() {
        let _guard = pricing_test_guard();
        let prev_env = std::env::var("PRICING_CONFIG").ok();
        let _tmp = load_temp_config(
            br#"
usd_to_cny = 6.7894
rate_date = "2026-07-31"

[special]
xunfei_per_call = 0.002211111111
kimi_per_token = 0.000000071071429
opencode_divisor = 6.0
ainaba_divisor = 40.0
freemodel_divisor = 67.894

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
        let expected = 0.55 * 6.7894;
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
usd_to_cny = 6.7894
rate_date = "2026-07-31"

[special]
xunfei_per_call = 0.002211111111
kimi_per_token = 0.000000071071429
opencode_divisor = 6.0
ainaba_divisor = 40.0
freemodel_divisor = 67.894

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
        let expected = 3.0 * 6.7894;
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
usd_to_cny = 6.7894
rate_date = "2026-07-31"

[special]
xunfei_per_call = 0.002211111111
kimi_per_token = 0.000000071071429
opencode_divisor = 6.0
ainaba_divisor = 40.0
freemodel_divisor = 67.894

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
        let expected = 2.945 * 6.7894;
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
usd_to_cny = 6.7894
rate_date = "2026-07-31"

[special]
xunfei_per_call = 0.002211111111
kimi_per_token = 0.000000071071429
opencode_divisor = 6.0
ainaba_divisor = 40.0
freemodel_divisor = 67.894

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
        let expected = 0.465 * 6.7894;
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
usd_to_cny = 6.7894
rate_date = "2026-07-31"

[special]
xunfei_per_call = 0.002211111111
kimi_per_token = 0.000000071071429
opencode_divisor = 6.0
ainaba_divisor = 40.0
freemodel_divisor = 67.894
commandcode_divisor = 10.0

[[model]]
name = "cc:deepseek-v4-flash"
input = 0.14
output = 0.28
cache_read = 0.0028
cache_write = 0.0

[[model]]
name = "cc:kimi-k2.6"
input = 0.95
output = 4.00
cache_read = 0.16
cache_write = 0.0

[[model]]
name = "cc:kimi-k3"
input = 3.00
output = 15.00
cache_read = 0.30
cache_write = 0.0
"#,
        );

        // Test: deepseek-v4-flash from commandcode
        // After normalization: input=295 (new), cache_read=20864 (cached)
        // cc price: input=$0.14/M, output=$0.28/M, cache_read=$0.0028/M
        // usd = 295*0.14/1M + 286*0.28/1M + 20864*0.0028/1M
        //     = 0.0000413 + 0.00008008 + 0.0000584192 = 0.0001797992
        // cny = 0.0001797992 * 6.7894 / 10.0 = 0.000122078
        let mut record = make_record("pi", "commandcode", "deepseek-v4-flash", 0, 0.0);
        record.input_tokens = 295;
        record.output_tokens = 286;
        record.cache_read_tokens = 20864;
        record.cache_write_tokens = 0;
        record.total_tokens = 21445;

        let cny = display_cost(&record);
        let usd = 295.0 * 0.14 / 1_000_000.0
            + 286.0 * 0.28 / 1_000_000.0
            + 20864.0 * 0.0028 / 1_000_000.0;
        let expected = usd * 6.7894 / 10.0;
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
        let expected2 = usd2 * 6.7894 / 10.0;
        assert!(
            (cny2 - expected2).abs() < 1e-9,
            "commandcode kimi: expected {}, got {}",
            expected2,
            cny2
        );

        // Test: model with provider prefix "moonshotai/Kimi-K3" → cc:kimi-k3
        // cc price: input=$3.00/M, output=$15.00/M, cache_read=$0.30/M
        let mut record3 = make_record("pi", "commandcode", "moonshotai/Kimi-K3", 0, 0.0);
        record3.input_tokens = 100_000;
        record3.output_tokens = 10_000;
        record3.cache_read_tokens = 50_000;
        record3.cache_write_tokens = 0;
        record3.total_tokens = 160_000;

        let cny3 = display_cost(&record3);
        let usd3 = 100_000.0 * 3.00 / 1_000_000.0
            + 10_000.0 * 15.00 / 1_000_000.0
            + 50_000.0 * 0.30 / 1_000_000.0;
        let expected3 = usd3 * 6.7894 / 10.0;
        assert!(
            cny3 > 0.0,
            "commandcode kimi-k3 should compute non-zero cost, got {}",
            cny3
        );
        assert!(
            (cny3 - expected3).abs() < 1e-9,
            "commandcode kimi-k3: expected {}, got {}",
            expected3,
            cny3
        );

        restore_pricing_env(prev_env);
    }

    #[test]
    fn commandcode_deepseek_flash_applies_dated_peak_schedule_to_display_cost() {
        let _guard = pricing_test_guard();
        let prev_env = std::env::var("PRICING_CONFIG").ok();
        let _tmp = load_temp_config(
            br#"
usd_to_cny = 6.7894
rate_date = "2026-07-31"

[special]
xunfei_per_call = 0.002211111111
kimi_per_token = 0.000000071071429
opencode_divisor = 6.0
ainaba_divisor = 40.0
freemodel_divisor = 67.894
commandcode_divisor = 10.0

[[model]]
name = "cc:deepseek-v4-flash"
input = 0.14
output = 0.28
cache_read = 0.0028
cache_write = 0.0

[[model]]
name = "cc:deepseek-v4-flash"
effective_from = "2026-08-16T16:00:00Z"
peak_hours_utc = [[1, 4], [6, 10]]
peak_weekdays_only = true
input = 0.22
output = 0.66
cache_read = 0.007
cache_write = 0.0
peak_input = 0.44
peak_output = 1.32
peak_cache_read = 0.014
peak_cache_write = 0.0
"#,
        );

        let mut record = make_record("pi", "commandcode", "deepseek-v4-flash", 1_000_000, 0.0);
        record.input_tokens = 25_000;
        record.cache_read_tokens = 975_000;
        record.output_tokens = 5_000;
        record.cache_write_tokens = 0;
        record.total_tokens = 1_000_000;

        let expected = |input: f64, cache_read: f64, output: f64| {
            (25_000.0 * input + 975_000.0 * cache_read + 5_000.0 * output) / 1_000_000.0 * 6.7894
                / 10.0
        };

        record.time = "2026-08-16T15:59:59Z".to_string();
        assert!((display_cost(&record) - expected(0.14, 0.0028, 0.28)).abs() < 1e-12);

        // DeepSeek may expose the dated API revision in the model name.
        record.model = "deepseek-v4-flash-0731".to_string();
        record.time = "2026-08-16T16:00:00Z".to_string();
        assert!((display_cost(&record) - expected(0.22, 0.007, 0.66)).abs() < 1e-12);

        record.time = "2026-08-17T01:00:00Z".to_string();
        assert!((display_cost(&record) - expected(0.44, 0.014, 1.32)).abs() < 1e-12);

        // Peak ranges are half-open: 04:00 UTC is already off-peak.
        record.time = "2026-08-17T04:00:00Z".to_string();
        assert!((display_cost(&record) - expected(0.22, 0.007, 0.66)).abs() < 1e-12);

        // Peak is weekday-only: Sat 2026-08-22 01:00 UTC is inside the peak
        // hour ranges but must bill at the off-peak rate.
        record.time = "2026-08-22T01:00:00Z".to_string();
        assert!((display_cost(&record) - expected(0.22, 0.007, 0.66)).abs() < 1e-12);

        // Same hour on the following Monday is peak.
        record.time = "2026-08-17T01:00:00Z".to_string();
        assert!((display_cost(&record) - expected(0.44, 0.014, 1.32)).abs() < 1e-12);

        restore_pricing_env(prev_env);
    }

    /// `peak_weekdays_only` must reject weekend peak windows while leaving the
    /// base (off-peak) rates in force — this is what Command Code's DeepSeek
    /// schedule documents ("Peak runs Monday to Friday only").
    #[test]
    fn peak_weekdays_only_skips_weekend_peak_windows() {
        let _guard = pricing_test_guard();
        let prev_env = std::env::var("PRICING_CONFIG").ok();
        let _tmp = load_temp_config(
            br#"
usd_to_cny = 6.7894
rate_date = "2026-07-31"

[special]
xunfei_per_call = 0.002211111111
kimi_per_token = 0.000000071071429
opencode_divisor = 6.0
ainaba_divisor = 40.0
freemodel_divisor = 67.894
commandcode_divisor = 1.0

[[model]]
name = "cc:weekend-probe"
input = 0.15
output = 0.60
cache_read = 0.003
cache_write = 0.0
peak_hours_utc = [[1, 4], [6, 10]]
peak_weekdays_only = true
peak_input = 0.30
peak_output = 1.20
peak_cache_read = 0.006
peak_cache_write = 0.0
"#,
        );

        let mut record = make_record("pi", "commandcode", "weekend-probe", 1_000_000, 0.0);
        record.input_tokens = 1_000_000;
        record.output_tokens = 0;
        record.cache_read_tokens = 0;
        record.cache_write_tokens = 0;
        record.total_tokens = 1_000_000;

        // Sat 2026-08-15 02:00 UTC — inside the window, weekday-only → off-peak.
        record.time = "2026-08-15T02:00:00Z".to_string();
        let weekend = display_cost(&record) / 6.7894;
        assert!((weekend - 0.15).abs() < 1e-12, "weekend got {weekend}");

        // Mon 2026-08-17 02:00 UTC — same hour, weekday → peak.
        record.time = "2026-08-17T02:00:00Z".to_string();
        let weekday = display_cost(&record) / 6.7894;
        assert!((weekday - 0.30).abs() < 1e-12, "weekday got {weekday}");

        restore_pricing_env(prev_env);
    }

    // ─── Ainaba time-based segment tests ────────────────────────────────

    #[test]
    fn ainaba_segments_before_cutoff_uses_40x() {
        let _guard = pricing_test_guard();
        let prev_env = std::env::var("PRICING_CONFIG").ok();
        let _tmp = load_temp_config(
            br#"
usd_to_cny = 6.7894
rate_date = "2026-07-31"

[special]
xunfei_per_call = 0.002211111111
kimi_per_token = 0.000000071071429
opencode_divisor = 6.0
ainaba_segments = [
    { before = "2025-05-25T22:30:00+08:00", divisor = 40.0 },
    { divisor = 25.0 },
]
freemodel_divisor = 67.894
commandcode_divisor = 10.0
"#,
        );

        // Record from May 25 10:00 UTC = May 25 18:00 CST, BEFORE the 22:30 CST cutoff
        let mut record = make_record("pi", "ainaba", "gpt-5.5", 0, 0.05);
        record.time = "2025-05-25T10:00:00Z".to_string();
        let cost = display_cost(&record);
        // cost=0.05 USD, 平台汇率=7.0, divisor=40.0
        // cny = 0.05 * 7.0 / 40.0 = 0.00875
        let expected = 0.05 * PricingConfig::default().special.ainaba_platform_rate / 40.0;
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
usd_to_cny = 6.7894
rate_date = "2026-07-31"

[special]
xunfei_per_call = 0.002211111111
kimi_per_token = 0.000000071071429
opencode_divisor = 6.0
ainaba_segments = [
    { before = "2025-05-25T22:30:00+08:00", divisor = 40.0 },
    { divisor = 25.0 },
]
freemodel_divisor = 67.894
commandcode_divisor = 10.0
"#,
        );

        // Record from May 25 15:00 UTC = May 25 23:00 CST, AFTER the 22:30 CST cutoff
        let mut record = make_record("pi", "ainaba", "gpt-5.5", 0, 0.05);
        record.time = "2025-05-25T15:00:00Z".to_string();
        let cost = display_cost(&record);
        // cost=0.05 USD, 平台汇率=7.0, divisor=25.0
        // cny = 0.05 * 7.0 / 25.0 = 0.014
        let expected = 0.05 * PricingConfig::default().special.ainaba_platform_rate / 25.0;
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
usd_to_cny = 6.7894
rate_date = "2026-07-31"

[special]
xunfei_per_call = 0.002211111111
kimi_per_token = 0.000000071071429
opencode_divisor = 6.0
ainaba_segments = [
    { before = "2025-05-25T22:30:00+08:00", divisor = 40.0 },
    { divisor = 25.0 },
]
freemodel_divisor = 67.894
commandcode_divisor = 10.0
"#,
        );

        // Exactly at cutoff: 2025-05-25T14:30:00Z = 2025-05-25T22:30:00+08:00
        let mut record = make_record("pi", "ainaba", "gpt-5.5", 0, 0.05);
        record.time = "2025-05-25T14:30:00Z".to_string();
        let cost = display_cost(&record);
        // Not before (record.time < cutoff is false), so falls through to catch-all: 25x
        let expected = 0.05 * PricingConfig::default().special.ainaba_platform_rate / 25.0;
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
usd_to_cny = 6.7894
rate_date = "2026-07-31"

[special]
xunfei_per_call = 0.002211111111
kimi_per_token = 0.000000071071429
opencode_divisor = 6.0
ainaba_segments = [
    { before = "2025-05-25T22:30:00+08:00", divisor = 40.0 },
    { divisor = 25.0 },
]
freemodel_divisor = 67.894
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
        // cny = 0.8 * 7.0 / 40.0 = 0.14
        let expected = 0.8 * PricingConfig::default().special.ainaba_platform_rate / 40.0;
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
usd_to_cny = 6.7894
rate_date = "2026-07-31"

[special]
xunfei_per_call = 0.002211111111
kimi_per_token = 0.000000071071429
opencode_divisor = 6.0
ainaba_divisor = 40.0
freemodel_divisor = 67.894
commandcode_divisor = 10.0
"#,
        );

        // Without ainaba_segments, should fall back to ainaba_divisor
        let mut record = make_record("pi", "ainaba", "gpt-5.5", 0, 0.05);
        record.time = "2025-05-25T15:00:00Z".to_string();
        let cost = display_cost(&record);
        let expected = 0.05 * PricingConfig::default().special.ainaba_platform_rate / 40.0;
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
usd_to_cny = 6.7894
rate_date = "2026-07-31"

[special]
xunfei_per_call = 0.002211111111
kimi_per_token = 0.000000071071429
opencode_divisor = 6.0
ainaba_segments = [
    { before = "2025-05-25T22:30:00+08:00", divisor = 40.0 },
    { divisor = 25.0 },
]
freemodel_divisor = 67.894
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
        // cny = 0.40 * 7.0 / 25.0 = 0.112
        let expected = 0.40 * PricingConfig::default().special.ainaba_platform_rate / 25.0;
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
    fn kimi_code_kimi_provider_uses_model_aware_subscription_rates() {
        let _guard = pricing_test_guard();
        // kimi-code records merged to provider="kimi" use their model's raw
        // API equivalent, then apply the subscription multiplier.
        let record = make_record("kimi-code", "kimi", "kimi-k2.6", 170_000, 0.0);
        let cost = display_cost(&record);
        let expected = (85_000.0 * 6.5 + 85_000.0 * 27.0) / 1_000_000.0 / 20.0;
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

        // kimi-for-coding is an alias without a dedicated rate and falls back
        // to K2.7 Code, including its ¥1.30/M cache-hit rate.
        let mut record2 = make_record("kimi-code", "kimi", "kimi-for-coding", 1_170_000, 0.0);
        record2.input_tokens = 85_000;
        record2.output_tokens = 85_000;
        record2.cache_read_tokens = 1_000_000;
        let cost2 = display_cost(&record2);
        let expected2 = (85_000.0 * 6.5 + 1_000_000.0 * 1.3 + 85_000.0 * 27.0) / 1_000_000.0 / 20.0;
        assert!(
            cost2 > 0.0,
            "kimi-code/kimi-for-coding should have non-zero cost, got {}",
            cost2
        );
        assert!(
            (cost2 - expected2).abs() < 1e-9,
            "kimi-code/kimi-for-coding cost: expected {}, got {}",
            expected2,
            cost2
        );
    }

    #[test]
    fn kimi_subscription_cost_uses_k2_6_api_rates_and_excludes_cache_writes() {
        let _guard = pricing_test_guard();
        let mut record = make_record("kimi-code", "kimi", "kimi-k2.6", 10_000_000, 0.0);
        record.input_tokens = 1_000_000;
        record.cache_read_tokens = 2_000_000;
        record.output_tokens = 3_000_000;
        record.cache_write_tokens = 4_000_000;

        let cost = display_cost(&record);
        // Raw API equivalent: ¥6.50 input + ¥2.20 cache-hit + ¥81 output.
        // Subscription estimate: raw API equivalent ÷ 20. Cache writes are free.
        let expected = (6.5 + 2.0 * 1.1 + 3.0 * 27.0) / 20.0;
        assert!(
            (cost - expected).abs() < 1e-9,
            "K2.6 subscription cost: expected {}, got {}",
            expected,
            cost
        );
    }

    #[test]
    fn kimi_subscription_cost_uses_k3_api_rates() {
        let _guard = pricing_test_guard();
        let mut record = make_record("kimi-cli", "kimi", "kimi-k3", 3_000_000, 0.0);
        record.input_tokens = 1_000_000;
        record.cache_read_tokens = 1_000_000;
        record.output_tokens = 1_000_000;

        let cost = display_cost(&record);
        // Raw API equivalent: ¥20 input + ¥2 cache-hit + ¥100 output, then ÷ 20.
        let expected = (20.0 + 2.0 + 100.0) / 20.0;
        assert!(
            (cost - expected).abs() < 1e-9,
            "K3 subscription cost: expected {}, got {}",
            expected,
            cost
        );
    }

    #[test]
    fn kimi_k3_256k_variant_uses_k3_api_rates() {
        let _guard = pricing_test_guard();
        let mut record = make_record("kimi-cli", "kimi", "k3-256k", 3_000_000, 0.0);
        record.input_tokens = 1_000_000;
        record.cache_read_tokens = 1_000_000;
        record.output_tokens = 1_000_000;

        let cost = display_cost(&record);
        let expected = (20.0 + 2.0 + 100.0) / 20.0;
        assert!(
            (cost - expected).abs() < 1e-9,
            "k3-256k subscription cost: expected {}, got {}",
            expected,
            cost
        );
    }

    #[test]
    fn ainaba_pi_stored_cost_recomputes_high_tier_with_cached_tokens() {
        let _guard = pricing_test_guard();
        let prev_env = std::env::var("PRICING_CONFIG").ok();
        let _tmp = load_temp_config(
            br#"
usd_to_cny = 6.7894
rate_date = "2026-07-31"

[special]
xunfei_per_call = 0.002211111111
kimi_per_token = 0.000000071071429
opencode_divisor = 6.0
ainaba_segments = [{ divisor = 20.0 }]
freemodel_divisor = 67.894
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
        let expected = expected_usd * PricingConfig::default().special.ainaba_platform_rate / 20.0;
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
usd_to_cny = 6.7894
rate_date = "2026-07-31"

[special]
xunfei_per_call = 0.002211111111
kimi_per_token = 0.000000071071429
opencode_divisor = 6.0
ainaba_segments = [{ divisor = 20.0 }]
freemodel_divisor = 67.894
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
        let expected = 0.4375 * PricingConfig::default().special.ainaba_platform_rate / 20.0;
        assert!(
            (cost - expected).abs() < 1e-9,
            "ainaba base-tier pi cost: expected {}, got {}",
            expected,
            cost
        );

        restore_pricing_env(prev_env);
    }

    // ── GPT-5.6 Terra/Luna time-segmented pricing tests ───────────────
    //
    // OpenAI announced a price reduction on 2026-07-30: GPT-5.6 Terra -20%,
    // GPT-5.6 Luna -80%, effective for reseller billing at 2026-07-31 14:00
    // CST (UTC+8). pricing.toml keeps the old prices as the baseline and adds
    // new entries with `effective_from = "2026-07-31T14:00:00+08:00"`.

    /// Build a record with explicit token counts and timestamp.
    fn make_timed_record(
        source: &str,
        provider: &str,
        model: &str,
        time: &str,
        input: i64,
        output: i64,
        cache_read: i64,
        cache_write: i64,
        cost: f64,
    ) -> TokenRecord {
        TokenRecord {
            date: time[..10].to_string(),
            time: time.to_string(),
            api_key_prefix: "test".to_string(),
            provider: provider.to_string(),
            original_provider: None,
            model: model.to_string(),
            source: source.to_string(),
            input_tokens: input,
            output_tokens: output,
            cache_read_tokens: cache_read,
            cache_write_tokens: cache_write,
            total_tokens: input + output + cache_read + cache_write,
            cost,
            ttft_ms: None,
            tps: None,
        }
    }

    #[test]
    fn gpt_5_6_terra_ainaba_uses_old_price_before_cutoff() {
        let _guard = pricing_test_guard();
        let prev_env = std::env::var("PRICING_CONFIG").ok();
        let _tmp = load_temp_config(include_bytes!("../pricing.toml"));

        // 13:59:59 CST = 05:59:59 UTC, just before the 14:00 CST cutoff.
        let record = make_timed_record(
            "pi",
            "ainaba",
            "gpt-5.6-terra",
            "2026-07-31T05:59:59Z",
            100_000,
            10_000,
            0,
            0,
            0.05,
        );
        let cost = display_cost(&record);
        // Old base tier: input=$2.50/M, output=$15/M. ainaba divisor = 20.
        // usd = 100000*2.5/1M + 10000*15/1M = 0.25 + 0.15 = 0.40
        // cny = 0.40 * 7.0 / 20.0 = 0.14
        let expected = 0.40 * PricingConfig::default().special.ainaba_platform_rate / 20.20202;
        assert!(
            (cost - expected).abs() < 1e-9,
            "terra before cutoff should use old price: expected {}, got {}",
            expected,
            cost
        );

        restore_pricing_env(prev_env);
    }

    #[test]
    fn gpt_5_6_terra_ainaba_uses_new_price_after_cutoff() {
        let _guard = pricing_test_guard();
        let prev_env = std::env::var("PRICING_CONFIG").ok();
        let _tmp = load_temp_config(include_bytes!("../pricing.toml"));

        // 14:00:00 CST = 06:00:00 UTC, exactly at the cutoff (>= applies new).
        let record = make_timed_record(
            "pi",
            "ainaba",
            "gpt-5.6-terra",
            "2026-07-31T06:00:00Z",
            100_000,
            10_000,
            0,
            0,
            0.05,
        );
        let cost = display_cost(&record);
        // New base tier: input=$2.00/M, output=$12/M. ainaba divisor = 20.
        // usd = 100000*2.0/1M + 10000*12/1M = 0.20 + 0.12 = 0.32
        // cny = 0.32 * 7.0 / 20.0 = 0.112
        let expected = 0.32 * PricingConfig::default().special.ainaba_platform_rate / 20.20202;
        assert!(
            (cost - expected).abs() < 1e-9,
            "terra after cutoff should use new reduced price: expected {}, got {}",
            expected,
            cost
        );

        restore_pricing_env(prev_env);
    }

    #[test]
    fn gpt_5_6_terra_long_context_after_cutoff_uses_new_high_tier() {
        let _guard = pricing_test_guard();
        let prev_env = std::env::var("PRICING_CONFIG").ok();
        let _tmp = load_temp_config(include_bytes!("../pricing.toml"));

        // total_input = 250000 + 50000 = 300000 > 272000 → high tier.
        let record = make_timed_record(
            "pi",
            "ainaba",
            "gpt-5.6-terra",
            "2026-07-31T06:00:00Z",
            250_000,
            10_000,
            50_000,
            0,
            0.05,
        );
        let cost = display_cost(&record);
        // New high tier: input=$4/M, output=$18/M, cache_read=$0.40/M.
        // usd = 250000*4/1M + 50000*0.40/1M + 10000*18/1M = 1.0 + 0.02 + 0.18 = 1.20
        // cny = 1.20 * 7.0 / 20.0 = 0.42
        let expected = 1.20 * PricingConfig::default().special.ainaba_platform_rate / 20.20202;
        assert!(
            (cost - expected).abs() < 1e-9,
            "terra long-context after cutoff should use new high tier: expected {}, got {}",
            expected,
            cost
        );

        restore_pricing_env(prev_env);
    }

    #[test]
    fn gpt_5_6_luna_ainaba_uses_new_price_after_cutoff() {
        let _guard = pricing_test_guard();
        let prev_env = std::env::var("PRICING_CONFIG").ok();
        let _tmp = load_temp_config(include_bytes!("../pricing.toml"));

        let record = make_timed_record(
            "pi",
            "ainaba",
            "gpt-5.6-luna",
            "2026-07-31T06:00:00Z",
            100_000,
            10_000,
            0,
            0,
            0.05,
        );
        let cost = display_cost(&record);
        // New base tier: input=$0.20/M, output=$1.20/M. ainaba divisor = 20.
        // usd = 100000*0.20/1M + 10000*1.20/1M = 0.02 + 0.012 = 0.032
        // cny = 0.032 * 7.0 / 20.0 = 0.0112
        let expected = 0.032 * PricingConfig::default().special.ainaba_platform_rate / 20.20202;
        assert!(
            (cost - expected).abs() < 1e-9,
            "luna after cutoff should use new reduced price: expected {}, got {}",
            expected,
            cost
        );

        restore_pricing_env(prev_env);
    }

    #[test]
    fn gpt_5_6_sol_unchanged_across_cutoff() {
        let _guard = pricing_test_guard();
        let prev_env = std::env::var("PRICING_CONFIG").ok();
        let _tmp = load_temp_config(include_bytes!("../pricing.toml"));

        // Sol pricing was not reduced; cost must be identical before & after.
        let before = make_timed_record(
            "pi",
            "ainaba",
            "gpt-5.6-sol",
            "2026-07-31T05:59:59Z",
            100_000,
            10_000,
            0,
            0,
            0.05,
        );
        let after = make_timed_record(
            "pi",
            "ainaba",
            "gpt-5.6-sol",
            "2026-07-31T06:00:00Z",
            100_000,
            10_000,
            0,
            0,
            0.05,
        );
        let cost_before = display_cost(&before);
        let cost_after = display_cost(&after);
        // Sol base: input=$5/M, output=$30/M. usd = 0.5 + 0.3 = 0.8.
        let expected = 0.8 * PricingConfig::default().special.ainaba_platform_rate / 20.20202;
        assert!(
            (cost_before - expected).abs() < 1e-9,
            "sol before cutoff: expected {}, got {}",
            expected,
            cost_before
        );
        assert!(
            (cost_after - expected).abs() < 1e-9,
            "sol after cutoff should be unchanged: expected {}, got {}",
            expected,
            cost_after
        );

        restore_pricing_env(prev_env);
    }

    #[test]
    fn fenno_gpt_5_6_terra_recomputes_with_time_segmented_pricing() {
        let _guard = pricing_test_guard();
        let prev_env = std::env::var("PRICING_CONFIG").ok();
        let _tmp = load_temp_config(include_bytes!("../pricing.toml"));

        let fenno_divisor = 150.0 * 6.7894 / 10.0;

        // Before cutoff → old terra prices.
        let before = make_timed_record(
            "pi",
            "fenno",
            "gpt-5.6-terra",
            "2026-07-31T05:59:59Z",
            100_000,
            10_000,
            0,
            0,
            0.05,
        );
        let cost_before = display_cost(&before);
        let expected_before = 0.40 * 6.7894 / fenno_divisor;
        assert!(
            (cost_before - expected_before).abs() < 1e-9,
            "fenno terra before cutoff should recompute at old price: expected {}, got {}",
            expected_before,
            cost_before
        );

        // After cutoff → new reduced terra prices.
        let after = make_timed_record(
            "pi",
            "fenno",
            "gpt-5.6-terra",
            "2026-07-31T06:00:00Z",
            100_000,
            10_000,
            0,
            0,
            0.05,
        );
        let cost_after = display_cost(&after);
        let expected_after = 0.32 * 6.7894 / fenno_divisor;
        assert!(
            (cost_after - expected_after).abs() < 1e-9,
            "fenno terra after cutoff should recompute at new price: expected {}, got {}",
            expected_after,
            cost_after
        );
        // New price must be lower than old.
        assert!(
            cost_after < cost_before,
            "fenno terra cost should drop after cutoff: before={}, after={}",
            cost_before,
            cost_after
        );

        restore_pricing_env(prev_env);
    }

    #[test]
    fn fenno_non_gpt_model_keeps_stored_cost_path() {
        let _guard = pricing_test_guard();
        let prev_env = std::env::var("PRICING_CONFIG").ok();
        let _tmp = load_temp_config(include_bytes!("../pricing.toml"));

        // A non-GPT Fenno model (claude-sonnet-4-6) must NOT be recomputed;
        // it falls through to the stored-cost path with the Fenno divisor.
        let record = make_timed_record(
            "pi",
            "fenno",
            "claude-sonnet-4-6",
            "2026-07-31T06:00:00Z",
            100_000,
            10_000,
            0,
            0,
            0.05,
        );
        let cost = display_cost(&record);
        let fenno_divisor = 150.0 * 6.7894 / 10.0;
        let expected = 0.05 * 6.7894 / fenno_divisor;
        assert!(
            (cost - expected).abs() < 1e-9,
            "fenno non-GPT should use stored cost + divisor: expected {}, got {}",
            expected,
            cost
        );

        restore_pricing_env(prev_env);
    }

    // ─── Xunfei off-peak (波谷) pricing tests ─────────────────────────────

    /// Helper: build a temp config with xunfei off-peak enabled.
    fn xunfei_off_peak_config() -> Vec<u8> {
        "usd_to_cny = 6.7894
rate_date = \"2026-07-31\"

[special]
xunfei_per_call = 0.002211111111
kimi_per_token = 0.000000071071429
opencode_divisor = 6.0
ainaba_divisor = 40.0
freemodel_divisor = 67.894

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

    #[test]
    fn model_price_applies_recurring_utc_peak_hours_after_cutover() {
        let base_tier = PriceTier {
            threshold: 0,
            input: 0.435,
            output: 0.87,
            cache_read: 0.003625,
            cache_write: 0.0,
            peak_input: None,
            peak_output: None,
            peak_cache_read: None,
            peak_cache_write: None,
            input_cny: None,
            output_cny: None,
            cache_read_cny: None,
            cache_write_cny: None,
            peak_input_cny: None,
            peak_output_cny: None,
            peak_cache_read_cny: None,
            peak_cache_write_cny: None,
        };
        let off_peak_tier = PriceTier {
            threshold: 0,
            input: 0.66,
            output: 1.98,
            cache_read: 0.022,
            cache_write: 0.0,
            peak_input: Some(1.32),
            peak_output: Some(3.96),
            peak_cache_read: Some(0.044),
            peak_cache_write: Some(0.0),
            input_cny: None,
            output_cny: None,
            cache_read_cny: None,
            cache_write_cny: None,
            peak_input_cny: None,
            peak_output_cny: None,
            peak_cache_read_cny: None,
            peak_cache_write_cny: None,
        };
        let price = ModelPrice {
            segments: vec![
                TimeSegment {
                    effective_from: None,
                    peak_hours_utc: Vec::new(),
                    peak_weekdays_only: false,
                    tiers: vec![base_tier],
                },
                TimeSegment {
                    effective_from: Some(
                        DateTime::parse_from_rfc3339("2026-08-16T16:00:00Z").unwrap(),
                    ),
                    peak_hours_utc: vec![[1, 4], [6, 10]],
                    peak_weekdays_only: false,
                    tiers: vec![off_peak_tier],
                },
            ],
        };

        assert_eq!(
            price.compute_usd(1_000_000, 0, 0, 0, "2026-08-16T15:59:59Z"),
            0.435
        );
        assert_eq!(
            price.compute_usd(1_000_000, 0, 0, 0, "2026-08-16T17:00:00Z"),
            0.66
        );
        assert_eq!(
            price.compute_usd(1_000_000, 0, 0, 0, "2026-08-17T01:00:00Z"),
            1.32
        );
    }

    #[test]
    fn yairouter_gpt_5_6_reverts_to_pre_july_rates_at_beijing_midnight() {
        let _guard = pricing_test_guard();
        let prev_env = std::env::var("PRICING_CONFIG").ok();
        let _tmp = load_temp_config(include_bytes!("../pricing.toml"));
        // 2026-08-17 00:00 CST is 2026-08-16 16:00 UTC. The rate change is
        // inclusive at that instant and applies only to Yairouter/Ainaba.
        let before = make_timed_record(
            "pi",
            "ainaba",
            "gpt-5.6-terra",
            "2026-08-16T15:59:59Z",
            100_000,
            10_000,
            0,
            0,
            0.01,
        );
        let after = make_timed_record(
            "pi",
            "ainaba",
            "gpt-5.6-terra",
            "2026-08-16T16:00:00Z",
            100_000,
            10_000,
            0,
            0,
            0.01,
        );

        // With the helper's 10% output split, Terra's discounted price is
        // $0.32/M equivalent; the reinstated pre-July price is $0.40/M.
        let divisor = 21.538461538;
        let before_expected = 0.32 * 7.0 / divisor;
        let after_expected = 0.40 * 7.0 / divisor;
        let before_cost = display_cost(&before);
        let after_cost = display_cost(&after);
        assert!(
            (before_cost - before_expected).abs() < 1e-9,
            "before cutoff: expected {before_expected}, got {before_cost}"
        );
        assert!(
            (after_cost - after_expected).abs() < 1e-9,
            "at cutoff: expected {after_expected}, got {after_cost}"
        );

        let luna = make_timed_record(
            "pi",
            "ainaba",
            "gpt-5.6-luna",
            "2026-08-16T16:00:00Z",
            100_000,
            10_000,
            0,
            0,
            0.01,
        );
        // Luna likewise changes from $0.032/M back to $0.160/M equivalent.
        let luna_expected = 0.160 * 7.0 / divisor;
        let luna_cost = display_cost(&luna);
        assert!(
            (luna_cost - luna_expected).abs() < 1e-9,
            "Luna at cutoff: expected {luna_expected}, got {luna_cost}"
        );

        restore_pricing_env(prev_env);
    }
}
