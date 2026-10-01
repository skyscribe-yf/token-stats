//! Data types for quota/balance responses.
//!
//! Contains all serializable response structs, provider-specific types,
//! and the aggregated `QuotaResponse` for the dashboard.

use serde::{Deserialize, Serialize};

// ─── Error type (test-only) ──────────────────────────────────────────────────

#[cfg(test)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuotaError {
    pub provider: String,
    pub message: String,
}

#[cfg(test)]
impl QuotaError {
    pub fn new(provider: &str, message: &str) -> Self {
        Self {
            provider: provider.to_string(),
            message: message.to_string(),
        }
    }
}

#[cfg(test)]
impl std::fmt::Display for QuotaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.provider, self.message)
    }
}

// ─── Kimi Code types ─────────────────────────────────────────────────────────

/// Raw response from `GET /usages` on the Kimi Code platform API.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KimiCodeUsageResponse {
    #[serde(default)]
    pub usage: Option<KimiCodeUsageData>,
    #[serde(default)]
    pub limits: Vec<KimiCodeLimit>,
    #[serde(default)]
    pub total_quota: Option<KimiCodeTotalQuota>,
    #[serde(default)]
    pub user: Option<KimiCodeUser>,
    #[serde(default)]
    pub parallel: Option<KimiCodeParallel>,
    #[serde(default)]
    pub sub_type: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KimiCodeUsageData {
    #[serde(deserialize_with = "super::deserialize_flexible_number", default)]
    pub limit: f64,
    #[serde(deserialize_with = "super::deserialize_flexible_number", default)]
    pub used: f64,
    #[serde(deserialize_with = "super::deserialize_flexible_number", default)]
    pub remaining: f64,
    #[serde(default)]
    pub reset_time: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KimiCodeLimit {
    #[serde(default)]
    pub window: Option<KimiCodeWindow>,
    #[serde(default)]
    pub detail: Option<KimiCodeUsageData>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KimiCodeWindow {
    #[serde(default)]
    pub duration: Option<i64>,
    #[serde(default)]
    pub time_unit: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KimiCodeTotalQuota {
    #[serde(deserialize_with = "super::deserialize_flexible_number", default)]
    pub limit: f64,
    #[serde(deserialize_with = "super::deserialize_flexible_number", default)]
    pub remaining: f64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KimiCodeUser {
    #[serde(default)]
    pub user_id: Option<String>,
    #[serde(default)]
    pub membership: Option<KimiCodeMembership>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KimiCodeMembership {
    #[serde(default)]
    pub level: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct KimiCodeParallel {
    #[serde(deserialize_with = "super::deserialize_flexible_number", default)]
    pub limit: f64,
}

/// Kimi Code OAuth token refresh response.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct KimiCodeTokenRefreshResponse {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub token_type: Option<String>,
    pub expires_in: Option<f64>,
    pub scope: Option<String>,
}

// ─── OpenCode types ──────────────────────────────────────────────────────────

// ─── Dashboard DTOs ──────────────────────────────────────────────────────────

/// Simplified Kimi Code quota info for the dashboard.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuotaKimiCode {
    pub provider: String,
    pub weekly_limit: i64,
    pub weekly_used: i64,
    pub weekly_remaining: i64,
    pub weekly_reset_time: Option<String>,
    pub rp5h_limit: i64,
    pub rp5h_used: i64,
    pub rp5h_remaining: i64,
    pub rp5h_reset_time: Option<String>,
    pub total_limit: i64,
    pub total_remaining: i64,
    pub parallel_limit: i64,
    pub membership_level: Option<String>,
    pub sub_type: Option<String>,
}

/// Single usage entry from the OpenCode-go workspace dashboard.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuotaOpenCodeUsageEntry {
    pub usage_type: String,
    pub percentage: i32,
    pub resets_in: String,
    /// Computed absolute timestamp when the quota resets (ISO 8601 / RFC 3339).
    /// `None` if `resets_in` could not be parsed.
    pub reset_at: Option<String>,
}

/// Simplified OpenCode-go quota info for the dashboard.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuotaOpenCode {
    pub provider: String,
    pub entries: Vec<QuotaOpenCodeUsageEntry>,
    pub workspace_url: Option<String>,
}

/// Aggregated quota response for the dashboard.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuotaResponse {
    pub kimi: Option<KimiQuotaStatus>,
    pub kimi_ex: Option<KimiQuotaStatus>,
    pub opencode_go: Option<OpenCodeQuotaStatus>,
    pub opencode_go_ex: Option<OpenCodeQuotaStatus>,
    pub xiaomi_mimo: Option<XiaomiMiMoQuotaStatus>,
    pub commandcode: Option<CommandCodeQuotaStatus>,
    pub commandcode_ex: Option<CommandCodeQuotaStatus>,
    pub codebuddy: Option<CodeBuddyQuotaStatus>,
    pub ollama: Option<OllamaQuotaStatus>,
    pub meituan: Option<MeituanQuotaStatus>,
    pub fenno: Option<FennoQuotaStatus>,
    pub fenno_ex: Option<FennoQuotaStatus>,
    pub grok: Option<GrokQuotaStatus>,
    pub dimagent: Option<DimAgentQuotaStatus>,
    pub zcode: Option<ZcodeQuotaStatus>,
    pub zai: Option<ZaiQuotaStatus>,
    pub stepfun: Option<StepFunQuotaStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KimiQuotaStatus {
    pub available: bool,
    pub data: Option<QuotaKimiCode>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenCodeQuotaStatus {
    pub available: bool,
    pub data: Option<QuotaOpenCode>,
    pub error: Option<String>,
}

// ─── Fenno subscription types ───────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FennoSubscriptionGroup {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub platform: String,
    #[serde(default)]
    pub daily_limit_usd: Option<f64>,
    #[serde(default)]
    pub weekly_limit_usd: Option<f64>,
    #[serde(default)]
    pub monthly_limit_usd: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FennoSubscription {
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub expires_at: Option<String>,
    #[serde(default)]
    pub daily_usage_usd: f64,
    #[serde(default)]
    pub weekly_usage_usd: f64,
    #[serde(default)]
    pub monthly_usage_usd: f64,
    #[serde(default)]
    pub daily_window_start: Option<String>,
    #[serde(default)]
    pub weekly_window_start: Option<String>,
    #[serde(default)]
    pub monthly_window_start: Option<String>,
    #[serde(default)]
    pub group: FennoSubscriptionGroup,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FennoQuotaData {
    pub subscriptions: Vec<FennoSubscription>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FennoQuotaStatus {
    pub available: bool,
    pub data: Option<FennoQuotaData>,
    pub error: Option<String>,
}

// ─── Xiaomi MiMo TP types ────────────────────────────────────────────────────

/// Single usage entry from Xiaomi MiMo TP platform.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XiaomiMiMoUsageEntry {
    pub name: String,
    pub used: i64,
    pub limit: i64,
    pub percent: f64,
}

/// Xiaomi MiMo TP quota data for the dashboard.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XiaomiMiMoQuotaData {
    pub entries: Vec<XiaomiMiMoUsageEntry>,
    pub month_percent: f64,
    pub plan_name: String,
    pub plan_code: String,
    pub current_period_end: Option<String>,
    pub expired: bool,
    pub enable_auto_renew: bool,
}

/// Xiaomi MiMo TP quota status for the dashboard.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XiaomiMiMoQuotaStatus {
    pub available: bool,
    pub data: Option<XiaomiMiMoQuotaData>,
    pub error: Option<String>,
}

// ─── CommandCode types ───────────────────────────────────────────────────────

/// A rolling-window usage limit (5h / weekly) returned by the credits API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandCodeWindowLimit {
    pub used: f64,
    pub cap: f64,
    /// RFC 3339 timestamp when the window resets; `None` if unknown.
    pub reset_at: Option<String>,
}

/// CommandCode subscription/quota data for the dashboard.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandCodeQuotaData {
    pub plan_name: String,
    pub subscription_status: String,
    pub cancel_at_period_end: Option<bool>,
    /// CLI account name from the auth file (e.g. "yanchenxin0924gbst").
    #[serde(default)]
    pub user_name: String,
    /// CLI account user id from the auth file.
    #[serde(default)]
    pub user_id: String,
    pub monthly_credits_total: Option<f64>,
    pub monthly_credits_used: f64,
    pub monthly_credits_remaining: f64,
    pub purchased_credits: f64,
    pub premium_monthly_credits: f64,
    pub opensource_monthly_credits: f64,
    pub current_period_end: Option<String>,
    pub total_requests: i64,
    pub total_tokens: i64,
    pub total_tokens_in: i64,
    pub total_tokens_out: i64,
    /// Rolling 5-hour window limit (credit cap).
    #[serde(default)]
    pub five_hour: Option<CommandCodeWindowLimit>,
    /// Rolling weekly window limit (credit cap).
    #[serde(default)]
    pub weekly: Option<CommandCodeWindowLimit>,
}

/// CommandCode quota status for the dashboard.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandCodeQuotaStatus {
    pub available: bool,
    pub data: Option<CommandCodeQuotaData>,
    pub error: Option<String>,
}

// ─── CodeBuddy (codebuddy.cn) types ─────────────────────────────────────────

/// A single CodeBuddy capacity package (subscription plan or bonus pack).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeBuddyPackage {
    pub package_code: String,
    pub package_name: String,
    /// `true` for the package matching `SubscriptionPackageCode` (the paid plan).
    pub is_subscription: bool,
    /// Capacity unit (currently always "credits").
    pub unit: String,
    pub total: f64,
    pub used: f64,
    pub remain: f64,
    /// Local "YYYY-MM-DD HH:MM:SS" as returned by the API.
    pub cycle_start: Option<String>,
    pub cycle_end: Option<String>,
}

/// CodeBuddy subscription/quota data for the dashboard.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeBuddyQuotaData {
    pub is_paid_user: bool,
    pub packages: Vec<CodeBuddyPackage>,
}

/// CodeBuddy quota status for the dashboard.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeBuddyQuotaStatus {
    pub available: bool,
    pub data: Option<CodeBuddyQuotaData>,
    pub error: Option<String>,
}

// ─── ZAI (api.zairouter.com) types ───────────────────────────────────────────

/// One top-up card ("到账卡") on the ZAI account. ZAI grants USD face value
/// 1:1 with the RMB paid, so `amount`/`balance` are USD-denominated.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZaiCreditCard {
    pub amount: f64,
    pub balance: f64,
    pub reference: String,
    pub granted_at: String,
    pub expires_at: String,
}

/// Per-model usage for one window returned by `dashboard/live`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZaiModelUsage {
    pub model: String,
    pub requests: i64,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    /// Credit (USD face value) charged for this model in the window.
    pub credit_used: f64,
}

/// ZAI (ZAI Router) account balance and usage.
///
/// ZAI settles in USD credit bought 1:1 with RMB: the top-up order records
/// `amount` (分) and `credit_amount` (美元额度) as the same number, e.g.
/// ¥100 → 100.0. The dashboard therefore presents the balance in RMB
/// (`= credit`), matching what the platform console shows the user.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZaiQuotaData {
    pub user_id: i64,
    pub name: String,
    pub alias: String,
    pub email: String,
    /// Remaining balance (RMB == USD credit, 1:1).
    pub balance: f64,
    /// Sum of all active top-up cards' granted amounts.
    pub credit_total: f64,
    /// Lifetime credit consumed.
    pub credit_used: f64,
    /// Earliest card expiry.
    pub expires_at: String,
    pub cards: Vec<ZaiCreditCard>,
    pub total_requests: i64,
    pub suspended: bool,
    /// Today's usage (platform-local day).
    pub daily_used: f64,
    pub daily_requests: i64,
    pub daily_input_tokens: i64,
    pub daily_output_tokens: i64,
    pub daily_cache_read_tokens: i64,
    pub daily_cache_write_tokens: i64,
    pub daily_models: Vec<ZaiModelUsage>,
    /// Current calendar month's usage.
    pub monthly_used: f64,
    pub monthly_requests: i64,
    pub monthly_input_tokens: i64,
    pub monthly_output_tokens: i64,
    pub monthly_cache_read_tokens: i64,
    pub monthly_cache_write_tokens: i64,
    pub monthly_models: Vec<ZaiModelUsage>,
}

/// ZAI quota status for the dashboard.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZaiQuotaStatus {
    pub available: bool,
    pub data: Option<ZaiQuotaData>,
    pub error: Option<String>,
}

// ─── StepFun (platform.stepfun.com) types ────────────────────────────────────

/// Step Plan subscription monthly credit pool (console RPC
/// `QueryStepPlanRateLimit` + `GetStepPlanStatus`, Oasis-Token auth).
/// 1M credit = ¥1 of list-price usage; the pool resets monthly.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepFunPlanQuota {
    /// Plan tier name, e.g. "Plus".
    pub plan_name: String,
    /// Monthly pool remaining fraction, 0.0–1.0.
    pub credit_left_rate: f64,
    /// Credits granted to the pool this month (e.g. 1_600_000_000 for Plus).
    pub credit_total: i64,
    pub credit_residual: i64,
    /// Next monthly pool reset (unix seconds).
    pub next_reset_at: i64,
    /// Subscription expiry (unix seconds).
    pub expired_at: i64,
}

/// StepFun credit account balance (`GET /v1/accounts`), in CNY.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepFunQuotaData {
    /// "prepaid" or "postpaid".
    pub account_type: String,
    /// Current usable balance.
    pub balance: f64,
    /// Lifetime top-up total.
    pub total_cash_balance: f64,
    /// Lifetime granted (voucher) total.
    pub total_voucher_balance: f64,
    /// Step Plan pool; None when console credentials are missing/expired.
    #[serde(default)]
    pub plan: Option<StepFunPlanQuota>,
    /// Why the pool is missing. The balance half still renders; the card
    /// surfaces this instead of silently dropping the plan percentage.
    #[serde(default)]
    pub plan_error: Option<String>,
}

/// StepFun quota status for the dashboard.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepFunQuotaStatus {
    pub available: bool,
    pub data: Option<StepFunQuotaData>,
    pub error: Option<String>,
}

// ─── Ollama types ────────────────────────────────────────────────────────────

/// Single usage entry from Ollama cloud (session or weekly).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaUsageEntry {
    pub usage_type: String,
    pub percentage: f64,
    pub reset_time: Option<String>,
}

/// Ollama Pro subscription and usage data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaQuotaData {
    pub plan_name: String,
    pub renews_on: Option<String>,
    pub price: Option<String>,
    pub usage_entries: Vec<OllamaUsageEntry>,
    pub has_annual_option: bool,
    pub has_max_upgrade: bool,
    /// Actual tokens metered by the `ollama-proxy` source in the live weekly
    /// window (input + output + cache read + cache write).
    #[serde(default)]
    pub weekly_tokens: Option<i64>,
    /// Subscription cost (CNY) of those records, at the dashboard's empirical
    /// Ollama rate.
    #[serde(default)]
    pub weekly_cost_cny: Option<f64>,
    /// Requests metered in the live weekly window.
    #[serde(default)]
    pub weekly_calls: Option<i64>,
}

/// Ollama quota status for the dashboard.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaQuotaStatus {
    pub available: bool,
    pub data: Option<OllamaQuotaData>,
    pub error: Option<String>,
}

// ─── Meituan LongCat types ─────────────────────────────────────────────────────

/// Single Meituan token resource pack.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeituanTokenPack {
    pub package_name: String,
    pub source_type_text: String,
    pub source_type_code: i64,
    pub status_text: String,
    pub status_code: i64,
    pub total_token_amount: i64,
    pub used_token_amount: i64,
    pub remain_token_amount: i64,
    pub usage_percent: i64,
    pub valid_start_time: String,
    pub valid_end_date_text: String,
    pub applicable_models: Vec<String>,
}

/// Meituan LongCat quota data for the dashboard.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeituanQuotaData {
    pub packs: Vec<MeituanTokenPack>,
    pub active_count: i64,
    /// Total tokens consumed in the last 7 days.
    pub recent_7d_tokens: i64,
}

/// Meituan quota status for the dashboard.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeituanQuotaStatus {
    pub available: bool,
    pub data: Option<MeituanQuotaData>,
    pub error: Option<String>,
}

// ─── Grok / XAI types ────────────────────────────────────────────────────────

/// Grok (XAI) account and usage data for the dashboard.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GrokQuotaData {
    pub user_id: String,
    pub team_id: String,
    pub zdr_status: String,
    /// Aggregated token usage from grok-cli data source.
    #[serde(default)]
    pub total_calls: i64,
    #[serde(default)]
    pub total_input_tokens: i64,
    #[serde(default)]
    pub total_output_tokens: i64,
    #[serde(default)]
    pub total_cache_read_tokens: i64,
    #[serde(default)]
    pub total_tokens: i64,
    /// Estimated subscription spend in CNY.
    #[serde(default)]
    pub estimated_cost_cny: f64,
    /// Percentage used by the weekly SuperGrok pool returned by grok.com.
    #[serde(default)]
    pub weekly_usage_percent: f64,
    /// Percentage remaining in the weekly SuperGrok pool returned by grok.com.
    #[serde(default)]
    pub weekly_remaining_percent: f64,
    /// Start of the current weekly quota window.
    #[serde(default)]
    pub weekly_period_start: String,
    /// End of the current weekly quota window.
    #[serde(default)]
    pub weekly_reset_at: Option<String>,
    /// Product-level usage rows returned by the Grok billing endpoint.
    #[serde(default)]
    pub weekly_breakdown: Vec<GrokQuotaBreakdown>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GrokQuotaBreakdown {
    pub product: String,
    pub usage_percent: f64,
}

/// Grok quota status for the dashboard.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GrokQuotaStatus {
    pub available: bool,
    pub data: Option<GrokQuotaData>,
    pub error: Option<String>,
}

// ─── DimAgent (dimcode) types ────────────────────────────────────────────────

/// A DimAgent per-call feature allowance (e.g. web_search 20 calls/term).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DimAgentFeatureMeter {
    pub feature_key: String,
    pub unit: String,
    pub unlimited: bool,
    pub used: i64,
    pub allowance: i64,
    pub remaining: i64,
    /// RFC 3339 timestamp when the meter resets; `None` if unknown.
    #[serde(default)]
    pub period_end: Option<String>,
}

/// Last-30d call/token summary from the DimAgent console activity API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DimAgentRecent30d {
    pub calls: i64,
    pub total_tokens: i64,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub cache_tokens: i64,
    /// Quota units consumed in the window (as reported by the console).
    #[serde(default)]
    pub quota_units: f64,
}

/// DimAgent subscription/quota data for the dashboard.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DimAgentQuotaData {
    pub plan_name: String,
    #[serde(default)]
    pub plan_description: Option<String>,
    /// Price in CNY per billing interval (e.g. 9.9 for ¥9.9/月).
    #[serde(default)]
    pub price_cny: f64,
    /// Billing interval from the price catalog (e.g. "month").
    #[serde(default)]
    pub billing_interval: String,
    pub subscription_status: String,
    #[serde(default)]
    pub cancel_at_period_end: bool,
    /// RFC 3339 start of the current term.
    pub period_start: String,
    /// RFC 3339 end of the current term (renewal/expiry).
    pub period_end: String,
    pub total_units: i64,
    pub used_units: i64,
    pub remaining_units: i64,
    /// Estimated remaining calls derived from average units/call (quota-estimate).
    #[serde(default)]
    pub estimated_remaining_calls: Option<i64>,
    /// Total request count on the account (quota-estimate).
    #[serde(default)]
    pub request_count_total: i64,
    #[serde(default)]
    pub feature_meters: Vec<DimAgentFeatureMeter>,
    /// Last-30d stats; only present when `DIMAGENT_SESSION_COOKIE` is set.
    #[serde(default)]
    pub recent_30d: Option<DimAgentRecent30d>,
}

/// DimAgent quota status for the dashboard.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DimAgentQuotaStatus {
    pub available: bool,
    pub data: Option<DimAgentQuotaData>,
    pub error: Option<String>,
}

// ─── ZCode (BigModel GLM coding plan) types ──────────────────────────────────

/// One quota window/credit pool from `GET /api/monitor/usage/quota/limit`.
/// Live-API semantics (verified 2026-09-12 against a Lite plan): `usage` is
/// the window TOTAL, `currentValue` the consumed amount, `remaining` the
/// remainder (usage ≈ currentValue + remaining), and `percentage` the used
/// percent (currentValue/usage) — the upstream field names are misleading,
/// so consumers should use `current_value` as "used".
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ZcodeLimitEntry {
    /// Limit kind, e.g. `CREDIT_LIMIT` / `TIME_LIMIT`.
    #[serde(default)]
    pub kind: Option<String>,
    /// Window length code returned by the API (3 = hour, 6 = week).
    #[serde(default)]
    pub unit: Option<f64>,
    /// Window length in `unit`s (e.g. 5-hour window → unit=3, number=5).
    #[serde(default)]
    pub number: Option<f64>,
    /// Window total size (NOT the used amount).
    #[serde(default)]
    pub usage: Option<f64>,
    /// Consumed amount in the current window.
    #[serde(default)]
    pub current_value: Option<f64>,
    /// Remaining amount in the current window.
    #[serde(default)]
    pub remaining: Option<f64>,
    /// Used percentage (current_value / usage).
    #[serde(default)]
    pub percentage: Option<f64>,
    /// RFC 3339 reset time converted from the API's epoch-ms value.
    #[serde(default)]
    pub next_reset_time: Option<String>,
    #[serde(default)]
    pub usage_details: Vec<ZcodeUsageDetail>,
}

/// Per-model usage inside one limit entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ZcodeUsageDetail {
    pub model_code: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub usage: f64,
}

/// Active subscription summary from `GET /api/biz/subscription/list`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ZcodeSubscription {
    pub product_name: Option<String>,
    #[serde(default)]
    pub billing_cycle: Option<String>,
    /// "YYYY-MM-DD" next renewal date.
    #[serde(default)]
    pub next_renew_time: Option<String>,
    /// RFC 3339 end of the current term (parsed from the `valid` range).
    #[serde(default)]
    pub expire_time: Option<String>,
    #[serde(default)]
    pub auto_renew: bool,
}

/// ZCode quota data: remote plan/limits plus local per-request usage.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ZcodeQuotaData {
    /// Plan level from the quota endpoint (e.g. "lite" → GLM Coding Lite).
    #[serde(default)]
    pub plan_level: Option<String>,
    #[serde(default)]
    pub limits: Vec<ZcodeLimitEntry>,
    #[serde(default)]
    pub subscription: Option<ZcodeSubscription>,
    // ── Local usage (source='zcode' records) ──
    pub today_calls: i64,
    pub today_input_tokens: i64,
    pub today_output_tokens: i64,
    pub today_cache_read_tokens: i64,
    pub today_cache_write_tokens: i64,
    pub today_total_tokens: i64,
    /// Estimated spend today in CNY (display_cost semantics).
    pub today_cost_cny: f64,
    pub total_calls: i64,
    pub total_input_tokens: i64,
    pub total_output_tokens: i64,
    pub total_cache_read_tokens: i64,
    pub total_cache_write_tokens: i64,
    pub total_tokens: i64,
    pub total_cost_cny: f64,
    /// Weekend Build 体验套餐 (trial grant) usage, accounted locally from
    /// `provider='bigmodel-start'` records. The BigModel monitor API rejects
    /// the start-plan key (401, verified 2026-09-13), so there is no remote
    /// half; the grant total comes from `ZCODE_START_PLAN_TOTAL_TOKENS`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_plan: Option<ZcodeStartPlanUsage>,
    /// Set when the remote quota fetch failed but local usage is still shown.
    #[serde(default)]
    pub quota_error: Option<String>,
}

/// Local accounting for the ZCode Weekend Build trial grant
/// (体验套餐, provider=`bigmodel-start`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ZcodeStartPlanUsage {
    /// Grant size in tokens (default 3 亿, env-overridable).
    pub grant_tokens: i64,
    /// Tokens consumed so far (sum of total_tokens, cache included).
    pub used_tokens: i64,
    /// grant_tokens - used_tokens, floored at 0.
    pub remaining_tokens: i64,
    /// Requests billed against the grant.
    pub calls: i64,
}

/// ZCode quota status for the dashboard.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZcodeQuotaStatus {
    pub available: bool,
    pub data: Option<ZcodeQuotaData>,
    pub error: Option<String>,
}
