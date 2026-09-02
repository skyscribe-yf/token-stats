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
    /// Estimated tokens used this week based on usage percentage and empirical weekly quota.
    #[serde(default)]
    pub estimated_tokens_used: Option<i64>,
    /// Estimated cost in CNY for this week's usage.
    #[serde(default)]
    pub estimated_cost_cny: Option<f64>,
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
