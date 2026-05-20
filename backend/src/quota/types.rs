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
    pub opencode_go: Option<OpenCodeQuotaStatus>,
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
