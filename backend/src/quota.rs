//! Quota/balance fetchers for external AI API providers.
//!
//! Supports:
//! - **Kimi Code**: Usage via `GET /usages` on the Kimi Code platform (OAuth access token)
//!   Auto-refreshes expired tokens using the stored refresh_token.
//! - **OpenCode-go**: Usage/quota via OpenAI-compatible billing endpoints
//!   When billing API returns 404, masks the HTML error and surfaces a workspace
//!   redirect URL constructed from the `OPENCODE_GO_WORKSPACE_ID` env var.
//!
//! Auth keys are read from environment variables or local config files.
//! For Kimi, the system reads the OAuth access token from
//! `~/.kimi/credentials/kimi-code.json` for the Kimi Code platform.

use chrono::Datelike;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

// ─── Error type ───────────────────────────────────────────────────────────────

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

// ─── Kimi Code (OAuth) ────────────────────────────────────────────────────────

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
    #[serde(deserialize_with = "deserialize_flexible_number", default)]
    pub limit: f64,
    #[serde(deserialize_with = "deserialize_flexible_number", default)]
    pub used: f64,
    #[serde(deserialize_with = "deserialize_flexible_number", default)]
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
    #[serde(deserialize_with = "deserialize_flexible_number", default)]
    pub limit: f64,
    #[serde(deserialize_with = "deserialize_flexible_number", default)]
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
    #[serde(deserialize_with = "deserialize_flexible_number", default)]
    pub limit: f64,
}

/// Deserializer that accepts both string and numeric values for number fields.
/// The Kimi Code API returns numbers as strings (e.g. "100" instead of 100).
fn deserialize_flexible_number<'de, D>(deserializer: D) -> Result<f64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de;

    struct FlexibleNumberVisitor;

    impl<'de> de::Visitor<'de> for FlexibleNumberVisitor {
        type Value = f64;

        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("a number or a string containing a number")
        }

        fn visit_f64<E>(self, v: f64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(v)
        }

        fn visit_i64<E>(self, v: i64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(v as f64)
        }

        fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(v as f64)
        }

        fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            v.parse::<f64>()
                .map_err(|_| de::Error::custom(format!("invalid number string: {}", v)))
        }

        fn visit_none<E>(self) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(0.0)
        }
    }

    deserializer.deserialize_any(FlexibleNumberVisitor)
}

/// Simplified Kimi Code quota info for the dashboard.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuotaKimiCode {
    pub provider: String,
    /// Weekly request limit
    pub weekly_limit: i64,
    /// Weekly requests used
    pub weekly_used: i64,
    /// Weekly requests remaining
    pub weekly_remaining: i64,
    /// Weekly reset time (ISO 8601)
    pub weekly_reset_time: Option<String>,
    /// 5-hour rolling window limit
    pub rp5h_limit: i64,
    /// 5-hour rolling window used
    pub rp5h_used: i64,
    /// 5-hour remaining
    pub rp5h_remaining: i64,
    /// 5-hour reset time
    pub rp5h_reset_time: Option<String>,
    /// Total quota limit
    pub total_limit: i64,
    /// Total quota remaining
    pub total_remaining: i64,
    /// Parallel request limit
    pub parallel_limit: i64,
    /// Membership level
    pub membership_level: Option<String>,
    /// Subscription type
    pub sub_type: Option<String>,
}

// ─── OpenCode-go ──────────────────────────────────────────────────────────────

/// Raw response from the OpenAI-compatible `/v1/dashboard/billing/subscription` endpoint.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OpenAiSubscriptionResponse {
    pub has_payment_method: Option<bool>,
    pub canceled: Option<bool>,
    pub canceled_at: Option<i64>,
    pub delinquent: Option<bool>,
    pub access_until: Option<i64>,
    pub soft_limit: Option<i64>,
    pub hard_limit: Option<i64>,
    pub system_hard_limit: Option<i64>,
    pub soft_limit_usd: Option<f64>,
    pub hard_limit_usd: Option<f64>,
    pub system_hard_limit_usd: Option<f64>,
    pub plan: Option<OpenAiPlan>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OpenAiPlan {
    pub title: Option<String>,
    pub id: Option<String>,
}

/// Raw response from `/v1/dashboard/billing/usage` endpoint.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OpenAiUsageResponse {
    pub object: Option<String>,
    pub total_usage: Option<f64>,
    pub total_tokens_used: Option<i64>,
}

/// Simplified OpenCode-go quota info for the dashboard.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuotaOpenCode {
    pub provider: String,
    pub plan_type: Option<String>,
    pub hard_limit_usd: Option<f64>,
    pub total_usage_usd: Option<f64>,
    pub usage_percent: Option<f64>,
    pub remaining_usd: Option<f64>,
    /// Link to the OpenCode-go workspace dashboard (when billing API returns 404)
    pub workspace_url: Option<String>,
}

// ─── Unified quota response ───────────────────────────────────────────────────

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

// ─── Auth helpers ─────────────────────────────────────────────────────────────

/// Path to the Kimi Code credentials file.
/// Can be overridden via the `KIMI_CREDENTIALS_PATH` env var.
pub fn get_kimi_credentials_path() -> PathBuf {
    if let Ok(path) = std::env::var("KIMI_CREDENTIALS_PATH") {
        return PathBuf::from(path);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home)
        .join(".kimi")
        .join("credentials")
        .join("kimi-code.json")
}

/// Kimi Code OAuth token refresh response from the auth API.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct KimiCodeTokenRefreshResponse {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub token_type: Option<String>,
    pub expires_in: Option<f64>,
    pub scope: Option<String>,
}

/// Read Kimi Code OAuth access token from `~/.kimi/credentials/kimi-code.json`.
/// If the token is expired, returns None (caller should attempt refresh).
pub fn get_kimi_code_access_token() -> Option<String> {
    let path = get_kimi_credentials_path();
    if !path.exists() {
        tracing::debug!("Kimi credentials file not found at {:?}", path);
        return None;
    }

    let content = std::fs::read_to_string(path).ok()?;
    let creds: serde_json::Value = serde_json::from_str(&content).ok()?;

    let token = creds.get("access_token").and_then(|v| v.as_str())?;
    if token.is_empty() {
        return None;
    }

    // Check if token is expired
    if let Some(expires_at) = creds.get("expires_at").and_then(|v| v.as_f64()) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as f64;
        if now > expires_at {
            tracing::warn!(
                "Kimi Code access token expired at {}, current time {}",
                expires_at,
                now
            );
            return None;
        }
    }

    Some(token.to_string())
}

/// Attempt to refresh the Kimi Code OAuth access token using the stored refresh_token.
/// On success, updates the credentials file and returns the new access token.
/// Returns None if refresh fails or no refresh_token is available.
pub async fn refresh_kimi_code_token(client: &reqwest::Client) -> Option<String> {
    let path = get_kimi_credentials_path();
    if !path.exists() {
        return None;
    }

    let content = std::fs::read_to_string(&path).ok()?;
    let creds: serde_json::Value = serde_json::from_str(&content).ok()?;

    let refresh_token = creds
        .get("refresh_token")
        .and_then(|v| v.as_str())?;
    if refresh_token.is_empty() {
        return None;
    }

    // Extract client_id from the credentials file, or use the default kimi-code client
    let client_id = creds
        .get("client_id")
        .and_then(|v| v.as_str())
        .unwrap_or("17e5f671-d194-4dfb-9706-5516cb48c098");

    let auth_base_url = get_kimi_auth_base_url();
    let url = format!("{}/api/oauth/token", auth_base_url.trim_end_matches('/'));

    tracing::info!("Refreshing Kimi Code access token via {:?}", url);

    let response = client
        .post(&url)
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", client_id),
        ])
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await;

    match response {
        Ok(r) if r.status().is_success() => {
            let refresh_resp: KimiCodeTokenRefreshResponse = match r.json().await {
                Ok(resp) => resp,
                Err(e) => {
                    tracing::warn!("Failed to parse token refresh response: {}", e);
                    return None;
                }
            };

            // Compute new expires_at
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as f64;
            let expires_in = refresh_resp.expires_in.unwrap_or(900.0);
            let new_expires_at = now + expires_in;

            // Update the credentials file
            let mut new_creds = creds.clone();
            new_creds["access_token"] = serde_json::Value::String(refresh_resp.access_token.clone());
            new_creds["expires_at"] = serde_json::Value::Number(
                serde_json::Number::from_f64(new_expires_at)
                    .unwrap_or(serde_json::Number::from(0)),
            );
            if let Some(new_refresh) = &refresh_resp.refresh_token {
                new_creds["refresh_token"] = serde_json::Value::String(new_refresh.clone());
            }
            new_creds["expires_in"] = serde_json::Value::Number(
                serde_json::Number::from_f64(expires_in)
                    .unwrap_or(serde_json::Number::from(900)),
            );
            if let Some(scope) = &refresh_resp.scope {
                new_creds["scope"] = serde_json::Value::String(scope.clone());
            }
            if let Some(token_type) = &refresh_resp.token_type {
                new_creds["token_type"] = serde_json::Value::String(token_type.clone());
            }

            let new_content = serde_json::to_string_pretty(&new_creds).unwrap_or_default();
            if std::fs::write(&path, new_content).is_err() {
                tracing::warn!("Failed to write refreshed token to credentials file");
            }
            tracing::info!("Kimi Code token refreshed successfully, expires in {}s", expires_in as i64);
            Some(refresh_resp.access_token)
        }
        Ok(r) => {
            let status = r.status();
            let body = r.text().await.unwrap_or_default();
            tracing::warn!("Kimi Code token refresh failed: {} {}", status, truncate_error_body(&body));
            None
        }
        Err(e) => {
            tracing::warn!("Kimi Code token refresh request failed: {}", e);
            None
        }
    }
}

/// Get the Kimi Auth API base URL for token refresh.
pub fn get_kimi_auth_base_url() -> String {
    std::env::var("KIMI_AUTH_BASE_URL")
        .unwrap_or_else(|_| "https://auth.kimi.com".to_string())
}

/// Get the Kimi Code API base URL.
pub fn get_kimi_code_base_url() -> String {
    std::env::var("KIMI_CODE_BASE_URL")
        .unwrap_or_else(|_| "https://api.kimi.com/coding/v1".to_string())
}

/// Path to the OpenCode auth.json file.
/// Can be overridden via the `OPENCODE_AUTH_PATH` env var (useful for testing).
pub fn get_opencode_auth_path() -> PathBuf {
    if let Ok(path) = std::env::var("OPENCODE_AUTH_PATH") {
        return PathBuf::from(path);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home)
        .join(".local")
        .join("share")
        .join("opencode")
        .join("auth.json")
}

/// Read opencode-go API key from local auth.json or env var `OPENCODE_API_KEY`.
pub fn get_opencode_api_key() -> Option<String> {
    // Check env var first
    if let Ok(key) = std::env::var("OPENCODE_API_KEY") {
        if !key.is_empty() {
            return Some(key);
        }
    }

    // Fall back to reading ~/.local/share/opencode/auth.json
    let path = get_opencode_auth_path();
    if !path.exists() {
        return None;
    }

    let content = std::fs::read_to_string(path).ok()?;
    let auth: serde_json::Value = serde_json::from_str(&content).ok()?;

    // Look for "opencode-go" entry with "key" field
    if let Some(entry) = auth.get("opencode-go") {
        if let Some(key) = entry.get("key").and_then(|k| k.as_str()) {
            if !key.is_empty() {
                return Some(key.to_string());
            }
        }
    }

    None
}

/// Truncate error response body for display. Long HTML responses (like OpenCode 404s)
/// are masked to avoid flooding the error message with irrelevant content.
fn truncate_error_body(body: &str) -> String {
    const MAX_ERROR_BODY_LEN: usize = 200;
    if body.len() <= MAX_ERROR_BODY_LEN {
        return body.to_string();
    }
    // Check if it looks like HTML
    if body.trim_start().starts_with("<!") || body.trim_start().starts_with("<html") {
        return "(HTML response, content omitted)".to_string();
    }
    // Otherwise truncate with ellipsis
    format!("{}...", &body[..MAX_ERROR_BODY_LEN])
}

/// Get the OpenCode base URL for API calls.
pub fn get_opencode_base_url() -> String {
    std::env::var("OPENCODE_BASE_URL")
        .unwrap_or_else(|_| "https://opencode.ai/zen/v1".to_string())
}

/// Get the OpenCode-go workspace URL from env var.
/// Constructs a redirect URL using the actual workspace ID from OPENCODE_GO_WORKSPACE_ID.
pub fn get_opencode_workspace_url() -> Option<String> {
    std::env::var("OPENCODE_GO_WORKSPACE_ID")
        .ok()
        .filter(|id| !id.is_empty())
        .map(|id| format!("https://opencode.ai/workspace/{}/go", id))
}

// ─── Fetcher ──────────────────────────────────────────────────────────────────

/// Quota fetcher with injectable HTTP client for testability.
pub struct QuotaFetcher {
    pub client: reqwest::Client,
}

impl QuotaFetcher {
    /// Create a new fetcher with the default reqwest client.
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }

    /// Fetch Kimi Code usage from the Kimi Code platform API.
    /// If the access token is expired, attempts to refresh it automatically.
    pub async fn fetch_kimi_quota(&self) -> KimiQuotaStatus {
        let access_token = match get_kimi_code_access_token() {
            Some(t) => t,
            None => {
                // Token expired or missing — try refreshing
                tracing::info!("Kimi Code access token expired or missing, attempting refresh");
                match refresh_kimi_code_token(&self.client).await {
                    Some(t) => t,
                    None => {
                        return KimiQuotaStatus {
                            available: false,
                            data: None,
                            error: Some(
                                "Kimi Code access token not found and refresh failed. \
                                 Log in with `kimi` CLI or set KIMI_CREDENTIALS_PATH"
                                    .to_string(),
                            ),
                        };
                    }
                }
            }
        };

        let base_url = get_kimi_code_base_url();
        let url = format!("{}/usages", base_url.trim_end_matches('/'));

        let response = match self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", access_token))
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                return KimiQuotaStatus {
                    available: false,
                    data: None,
                    error: Some(format!("HTTP request failed: {}", e)),
                };
            }
        };

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return KimiQuotaStatus {
                available: false,
                data: None,
                error: Some(format!("API returned {}: {}", status, truncate_error_body(&body))),
            };
        }

        let body: KimiCodeUsageResponse = match response.json().await {
            Ok(b) => b,
            Err(e) => {
                return KimiQuotaStatus {
                    available: false,
                    data: None,
                    error: Some(format!("Failed to parse response: {}", e)),
                };
            }
        };

        // Extract weekly and 5-hour limits from the response
        let mut weekly_limit: i64 = 0;
        let mut weekly_used: i64 = 0;
        let mut weekly_remaining: i64 = 0;
        let mut weekly_reset_time: Option<String> = None;
        let mut rp5h_limit: i64 = 0;
        let mut rp5h_used: i64 = 0;
        let mut rp5h_remaining: i64 = 0;
        let mut rp5h_reset_time: Option<String> = None;

        // Top-level usage is the primary (weekly) rate limit
        if let Some(usage) = &body.usage {
            weekly_limit = usage.limit as i64;
            weekly_used = usage.used as i64;
            weekly_remaining = usage.remaining as i64;
            weekly_reset_time = usage.reset_time.clone();
        }

        // Scan limits array for the 5-hour rolling window
        for limit in &body.limits {
            if let Some(detail) = &limit.detail {
                let window_duration = limit.window.as_ref().and_then(|w| w.duration);
                let time_unit = limit
                    .window
                    .as_ref()
                    .and_then(|w| w.time_unit.as_deref());

                // Identify the 5-hour rolling window
                let is_5h = (window_duration == Some(5)
                    && time_unit == Some("TIME_UNIT_HOUR"))
                    || (window_duration == Some(300) && time_unit == Some("TIME_UNIT_MINUTE"));

                if is_5h {
                    rp5h_limit = detail.limit as i64;
                    rp5h_used = detail.used as i64;
                    rp5h_remaining = detail.remaining as i64;
                    rp5h_reset_time = detail.reset_time.clone();
                }
            }
        }

        let total_limit = body
            .total_quota
            .as_ref()
            .map(|q| q.limit as i64)
            .unwrap_or(0);
        let total_remaining = body
            .total_quota
            .as_ref()
            .map(|q| q.remaining as i64)
            .unwrap_or(0);
        let parallel_limit = body
            .parallel
            .as_ref()
            .map(|p| p.limit as i64)
            .unwrap_or(0);
        let membership_level = body
            .user
            .as_ref()
            .and_then(|u| u.membership.as_ref().and_then(|m| m.level.clone()));
        let sub_type = body.sub_type.clone();

        KimiQuotaStatus {
            available: true,
            data: Some(QuotaKimiCode {
                provider: "kimi-code".to_string(),
                weekly_limit,
                weekly_used,
                weekly_remaining,
                weekly_reset_time,
                rp5h_limit,
                rp5h_used,
                rp5h_remaining,
                rp5h_reset_time,
                total_limit,
                total_remaining,
                parallel_limit,
                membership_level,
                sub_type,
            }),
            error: None,
        }
    }

    /// Fetch OpenCode-go subscription/quota info.
    ///
    /// Calls the OpenAI-compatible billing endpoints at the configured base URL.
    /// First tries `/v1/dashboard/billing/subscription` for plan info and limits,
    /// then `/v1/dashboard/billing/usage` for current usage.
    ///
    /// If the subscription endpoint returns 404, provides a workspace link
    /// (from `OPENCODE_GO_WORKSPACE_ID` env var) for the user to check manually.
    pub async fn fetch_opencode_quota(&self) -> OpenCodeQuotaStatus {
        let api_key = match get_opencode_api_key() {
            Some(k) => k,
            None => {
                // No API key — try providing workspace link if available
                if let Some(url) = get_opencode_workspace_url() {
                    return OpenCodeQuotaStatus {
                        available: false,
                        data: Some(QuotaOpenCode {
                            provider: "opencode-go".to_string(),
                            plan_type: None,
                            hard_limit_usd: None,
                            total_usage_usd: None,
                            usage_percent: None,
                            remaining_usd: None,
                            workspace_url: Some(url),
                        }),
                        error: Some(
                            "API key not found. Visit workspace to check usage.".to_string(),
                        ),
                    };
                }
                return OpenCodeQuotaStatus {
                    available: false,
                    data: None,
                    error: Some(
                        "opencode-go API key not found in auth.json or OPENCODE_API_KEY"
                            .to_string(),
                    ),
                };
            }
        };

        let base_url = get_opencode_base_url();
        let base = base_url.trim_end_matches('/');

        // Step 1: Fetch subscription info
        let sub_url = format!("{}/v1/dashboard/billing/subscription", base);
        let sub_response = self
            .client
            .get(&sub_url)
            .header("Authorization", format!("Bearer {}", api_key))
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await;

        let subscription: OpenAiSubscriptionResponse = match sub_response {
            Ok(r) if r.status().is_success() => match r.json().await {
                Ok(s) => s,
                Err(e) => {
                    return OpenCodeQuotaStatus {
                        available: false,
                        data: None,
                        error: Some(format!(
                            "Failed to parse subscription response: {}",
                            e
                        )),
                    };
                }
            },
            Ok(r) => {
                let status = r.status();
                // For 404, provide workspace link gracefully
                if status.as_u16() == 404 {
                    let workspace_url = get_opencode_workspace_url();
                    return OpenCodeQuotaStatus {
                        available: false,
                        data: Some(QuotaOpenCode {
                            provider: "opencode-go".to_string(),
                            plan_type: None,
                            hard_limit_usd: None,
                            total_usage_usd: None,
                            usage_percent: None,
                            remaining_usd: None,
                            workspace_url,
                        }),
                        error: Some(
                            "Billing API not available. Visit workspace to check usage."
                                .to_string(),
                        ),
                    };
                }
                let body = r.text().await.unwrap_or_default();
                return OpenCodeQuotaStatus {
                    available: false,
                    data: None,
                    error: Some(format!("Subscription API returned {}: {}", status, truncate_error_body(&body))),
                };
            }
            Err(e) => {
                return OpenCodeQuotaStatus {
                    available: false,
                    data: None,
                    error: Some(format!("Subscription request failed: {}", e)),
                };
            }
        };

        let plan_type = subscription
            .plan
            .as_ref()
            .and_then(|p| p.title.clone())
            .or_else(|| subscription.plan.as_ref().and_then(|p| p.id.clone()));
        let hard_limit_usd = subscription.hard_limit_usd;

        // Step 2: Fetch current usage
        let now = chrono::Utc::now();
        let start_of_month = chrono::NaiveDate::from_ymd_opt(now.year(), now.month(), 1)
            .unwrap_or_else(|| now.date_naive());
        let end_date = now.format("%Y-%m-%d").to_string();
        let start_date = start_of_month.format("%Y-%m-%d").to_string();

        let usage_url = format!(
            "{}/v1/dashboard/billing/usage?start_date={}&end_date={}",
            base, start_date, end_date
        );

        let total_usage_usd;
        let usage_percent;

        match self
            .client
            .get(&usage_url)
            .header("Authorization", format!("Bearer {}", api_key))
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await
        {
            Ok(r) if r.status().is_success() => match r.json::<OpenAiUsageResponse>().await {
                Ok(usage) => {
                    total_usage_usd = usage.total_usage;
                    usage_percent = hard_limit_usd.and_then(|limit| {
                        if limit > 0.0 {
                            total_usage_usd.map(|used| (used / limit * 100.0).min(100.0))
                        } else {
                            None
                        }
                    });
                }
                Err(_) => {
                    total_usage_usd = None;
                    usage_percent = None;
                }
            },
            _ => {
                // Usage endpoint may not be available; that's OK
                total_usage_usd = None;
                usage_percent = None;
            }
        }

        let remaining_usd = match (hard_limit_usd, total_usage_usd) {
            (Some(limit), Some(used)) => Some((limit - used).max(0.0)),
            _ => None,
        };

        OpenCodeQuotaStatus {
            available: true,
            data: Some(QuotaOpenCode {
                provider: "opencode-go".to_string(),
                plan_type,
                hard_limit_usd,
                total_usage_usd,
                usage_percent,
                remaining_usd,
                workspace_url: None,
            }),
            error: None,
        }
    }
}

impl Default for QuotaFetcher {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// Mutex to serialize env-var-modifying tests (prevents race conditions).
    static ENV_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn reset_env_var(name: &str, old: Option<String>) {
        match old {
            Some(v) => std::env::set_var(name, v),
            None => std::env::remove_var(name),
        }
    }

    // ── Auth helpers ────────────────────────────────────────────────────

    #[test]
    fn test_get_kimi_code_base_url_default() {
        temp_env::with_var("KIMI_CODE_BASE_URL", None::<&str>, || {
            assert_eq!(
                get_kimi_code_base_url(),
                "https://api.kimi.com/coding/v1"
            );
        });
    }

    #[test]
    fn test_get_kimi_code_base_url_custom() {
        temp_env::with_var(
            "KIMI_CODE_BASE_URL",
            Some("https://custom.kimi.api/v1"),
            || {
                assert_eq!(get_kimi_code_base_url(), "https://custom.kimi.api/v1");
            },
        );
    }

    #[test]
    fn test_get_opencode_base_url_default() {
        temp_env::with_var("OPENCODE_BASE_URL", None::<&str>, || {
            assert_eq!(get_opencode_base_url(), "https://opencode.ai/zen/v1");
        });
    }

    #[test]
    fn test_get_opencode_workspace_url() {
        temp_env::with_var(
            "OPENCODE_GO_WORKSPACE_ID",
            Some("wrk_01KP0TEMM33PV08MR37JCC2F9G"),
            || {
                assert_eq!(
                    get_opencode_workspace_url(),
                    Some(
                        "https://opencode.ai/workspace/wrk_01KP0TEMM33PV08MR37JCC2F9G/go"
                            .to_string()
                    )
                );
            },
        );
    }

    #[test]
    fn test_get_opencode_workspace_url_unset() {
        temp_env::with_var("OPENCODE_GO_WORKSPACE_ID", None::<&str>, || {
            assert_eq!(get_opencode_workspace_url(), None);
        });
    }

    // ── Kimi Code usage parsing ────────────────────────────────────────

    #[test]
    fn test_parse_kimi_code_usage_success() {
        let json = r#"{
            "usage": {
                "limit": "100",
                "used": "8",
                "remaining": "92",
                "resetTime": "2026-05-23T14:00:42.150347Z"
            },
            "limits": [{
                "window": {"duration": 300, "timeUnit": "TIME_UNIT_MINUTE"},
                "detail": {
                    "limit": "100",
                    "used": "4",
                    "remaining": "96",
                    "resetTime": "2026-05-17T17:00:42.150347Z"
                }
            }],
            "totalQuota": {"limit": "100", "remaining": "99"},
            "user": {"userId": "abc123", "membership": {"level": "LEVEL_INTERMEDIATE"}},
            "parallel": {"limit": "20"},
            "subType": "TYPE_PURCHASE"
        }"#;

        let resp: KimiCodeUsageResponse = serde_json::from_str(json).unwrap();
        let usage = resp.usage.unwrap();
        assert_eq!(usage.limit as i64, 100);
        assert_eq!(usage.used as i64, 8);
        assert_eq!(usage.remaining as i64, 92);
        assert_eq!(resp.limits.len(), 1);
        let limit = &resp.limits[0];
        let detail = limit.detail.as_ref().unwrap();
        assert_eq!(detail.limit as i64, 100);
        assert_eq!(detail.used as i64, 4);
        assert_eq!(detail.remaining as i64, 96);
        let tq = resp.total_quota.unwrap();
        assert_eq!(tq.limit as i64, 100);
        assert_eq!(tq.remaining as i64, 99);
    }

    #[test]
    fn test_parse_kimi_code_usage_empty() {
        let json = "{}";
        let resp: KimiCodeUsageResponse = serde_json::from_str(json).unwrap();
        assert!(resp.usage.is_none());
        assert!(resp.limits.is_empty());
    }

    // ── OpenAI subscription parsing ─────────────────────────────────────

    #[test]
    fn test_parse_openai_subscription_with_plan() {
        let json = r#"{
            "has_payment_method": true,
            "hard_limit_usd": 100.0,
            "plan": {
                "title": "Go Plan",
                "id": "opencode-go-monthly"
            }
        }"#;

        let resp: OpenAiSubscriptionResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.has_payment_method, Some(true));
        assert_eq!(resp.hard_limit_usd, Some(100.0));
        assert_eq!(
            resp.plan.as_ref().and_then(|p| p.title.as_deref()),
            Some("Go Plan")
        );
    }

    #[test]
    fn test_parse_openai_subscription_empty() {
        let json = "{}";
        let resp: OpenAiSubscriptionResponse = serde_json::from_str(json).unwrap();
        assert!(resp.hard_limit_usd.is_none());
        assert!(resp.plan.is_none());
    }

    // ── OpenAI usage parsing ────────────────────────────────────────────

    #[test]
    fn test_parse_openai_usage() {
        let json = r#"{
            "object": "list",
            "total_usage": 25.50
        }"#;

        let resp: OpenAiUsageResponse = serde_json::from_str(json).unwrap();
        assert!((resp.total_usage.unwrap() - 25.50).abs() < 0.01);
    }

    // ── QuotaError ──────────────────────────────────────────────────────

    #[test]
    fn test_quota_error_display() {
        let err = QuotaError::new("kimi", "token expired");
        assert_eq!(format!("{}", err), "[kimi] token expired");
    }

    // ── Integration: Kimi Code quota ────────────────────────────────────

    #[tokio::test]
    async fn test_fetch_kimi_quota_success() {
        let _lock = ENV_MUTEX.lock().unwrap();
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/usages"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "usage": {
                    "limit": "100",
                    "used": "8",
                    "remaining": "92",
                    "resetTime": "2026-05-23T14:00:42Z"
                },
                "limits": [{
                    "window": {"duration": 300, "timeUnit": "TIME_UNIT_MINUTE"},
                    "detail": {
                        "limit": "100",
                        "used": "4",
                        "remaining": "96",
                        "resetTime": "2026-05-17T17:00:42Z"
                    }
                }],
                "totalQuota": {"limit": "100", "remaining": "99"},
                "parallel": {"limit": "20"},
                "subType": "TYPE_PURCHASE"
            })))
            .mount(&mock_server)
            .await;

        // Write a temp credentials file
        let tmp_dir = tempfile::tempdir().unwrap();
        let cred_path = tmp_dir.path().join("kimi-code.json");
        std::fs::write(
            &cred_path,
            serde_json::json!({
                "access_token": "test-token-123",
                "expires_at": std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs() as f64 + 3600.0
            })
            .to_string(),
        )
        .unwrap();

        let old_path = std::env::var("KIMI_CREDENTIALS_PATH").ok();
        let old_url = std::env::var("KIMI_CODE_BASE_URL").ok();
        std::env::set_var("KIMI_CREDENTIALS_PATH", cred_path.to_str().unwrap());
        std::env::set_var("KIMI_CODE_BASE_URL", mock_server.uri());

        let fetcher = QuotaFetcher::new();
        let status = fetcher.fetch_kimi_quota().await;

        reset_env_var("KIMI_CREDENTIALS_PATH", old_path);
        reset_env_var("KIMI_CODE_BASE_URL", old_url);

        assert!(status.available);
        let data = status.data.unwrap();
        assert_eq!(data.provider, "kimi-code");
        assert_eq!(data.weekly_limit, 100);
        assert_eq!(data.weekly_used, 8);
        assert_eq!(data.weekly_remaining, 92);
        assert_eq!(data.rp5h_limit, 100);
        assert_eq!(data.rp5h_used, 4);
        assert_eq!(data.rp5h_remaining, 96);
        assert_eq!(data.total_limit, 100);
        assert_eq!(data.total_remaining, 99);
        assert_eq!(data.parallel_limit, 20);
        assert_eq!(data.sub_type, Some("TYPE_PURCHASE".to_string()));
    }

    #[tokio::test]
    async fn test_fetch_kimi_quota_no_token() {
        let _lock = ENV_MUTEX.lock().unwrap();
        let old_path = std::env::var("KIMI_CREDENTIALS_PATH").ok();
        std::env::set_var(
            "KIMI_CREDENTIALS_PATH",
            "/tmp/nonexistent-kimi-creds-test.json",
        );

        let fetcher = QuotaFetcher::new();
        let status = fetcher.fetch_kimi_quota().await;

        reset_env_var("KIMI_CREDENTIALS_PATH", old_path);

        assert!(!status.available);
        assert!(status.error.unwrap().contains("access token not found"));
    }

    #[tokio::test]
    async fn test_fetch_kimi_quota_api_error() {
        let _lock = ENV_MUTEX.lock().unwrap();
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/usages"))
            .respond_with(ResponseTemplate::new(401).set_body_string(
                r#"{"error":{"message":"Invalid token","type":"invalid_authentication_error"}}"#,
            ))
            .mount(&mock_server)
            .await;

        // Write a temp credentials file
        let tmp_dir = tempfile::tempdir().unwrap();
        let cred_path = tmp_dir.path().join("kimi-code.json");
        std::fs::write(
            &cred_path,
            serde_json::json!({
                "access_token": "bad-token",
                "expires_at": std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs() as f64 + 3600.0
            })
            .to_string(),
        )
        .unwrap();

        let old_path = std::env::var("KIMI_CREDENTIALS_PATH").ok();
        let old_url = std::env::var("KIMI_CODE_BASE_URL").ok();
        std::env::set_var("KIMI_CREDENTIALS_PATH", cred_path.to_str().unwrap());
        std::env::set_var("KIMI_CODE_BASE_URL", mock_server.uri());

        let fetcher = QuotaFetcher::new();
        let status = fetcher.fetch_kimi_quota().await;

        reset_env_var("KIMI_CREDENTIALS_PATH", old_path);
        reset_env_var("KIMI_CODE_BASE_URL", old_url);

        assert!(!status.available);
        assert!(status.error.unwrap().contains("401"));
    }

    // ── Integration: OpenCode-go quota ──────────────────────────────────

    #[tokio::test]
    async fn test_fetch_opencode_quota_success() {
        let _lock = ENV_MUTEX.lock().unwrap();
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/v1/dashboard/billing/subscription"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "hard_limit_usd": 100.0,
                "plan": { "title": "Go Plan" }
            })))
            .mount(&mock_server)
            .await;

        Mock::given(method("GET"))
            .and(path("/v1/dashboard/billing/usage"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "total_usage": 25.50
            })))
            .mount(&mock_server)
            .await;

        let old_key = std::env::var("OPENCODE_API_KEY").ok();
        let old_url = std::env::var("OPENCODE_BASE_URL").ok();
        std::env::set_var("OPENCODE_API_KEY", "sk-test-opencode");
        std::env::set_var("OPENCODE_BASE_URL", mock_server.uri());

        let fetcher = QuotaFetcher::new();
        let status = fetcher.fetch_opencode_quota().await;

        reset_env_var("OPENCODE_API_KEY", old_key);
        reset_env_var("OPENCODE_BASE_URL", old_url);

        assert!(status.available);
        let data = status.data.unwrap();
        assert_eq!(data.provider, "opencode-go");
        assert_eq!(data.plan_type, Some("Go Plan".to_string()));
        assert_eq!(data.hard_limit_usd, Some(100.0));
        assert!((data.total_usage_usd.unwrap() - 25.50).abs() < 0.01);
        assert!((data.usage_percent.unwrap() - 25.5).abs() < 0.01);
        assert!((data.remaining_usd.unwrap() - 74.5).abs() < 0.01);
        assert!(data.workspace_url.is_none());
    }

    #[tokio::test]
    async fn test_fetch_opencode_quota_404_with_workspace() {
        let _lock = ENV_MUTEX.lock().unwrap();
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/v1/dashboard/billing/subscription"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&mock_server)
            .await;

        let old_key = std::env::var("OPENCODE_API_KEY").ok();
        let old_url = std::env::var("OPENCODE_BASE_URL").ok();
        let old_ws = std::env::var("OPENCODE_GO_WORKSPACE_ID").ok();
        std::env::set_var("OPENCODE_API_KEY", "sk-test");
        std::env::set_var("OPENCODE_BASE_URL", mock_server.uri());
        std::env::set_var("OPENCODE_GO_WORKSPACE_ID", "wrk_TEST123");

        let fetcher = QuotaFetcher::new();
        let status = fetcher.fetch_opencode_quota().await;

        reset_env_var("OPENCODE_API_KEY", old_key);
        reset_env_var("OPENCODE_BASE_URL", old_url);
        reset_env_var("OPENCODE_GO_WORKSPACE_ID", old_ws);

        assert!(!status.available);
        // Should have data with workspace_url
        let data = status.data.unwrap();
        assert_eq!(
            data.workspace_url,
            Some("https://opencode.ai/workspace/wrk_TEST123/go".to_string())
        );
        assert!(status.error.unwrap().contains("Billing API not available"));
    }

    #[tokio::test]
    async fn test_fetch_opencode_quota_no_api_key_with_workspace() {
        let _lock = ENV_MUTEX.lock().unwrap();
        let old_key = std::env::var("OPENCODE_API_KEY").ok();
        let old_auth = std::env::var("OPENCODE_AUTH_PATH").ok();
        let old_ws = std::env::var("OPENCODE_GO_WORKSPACE_ID").ok();
        std::env::remove_var("OPENCODE_API_KEY");
        std::env::set_var("OPENCODE_AUTH_PATH", "/tmp/nonexistent-opencode-auth.json");
        std::env::set_var("OPENCODE_GO_WORKSPACE_ID", "wrk_NOKEY");

        let fetcher = QuotaFetcher::new();
        let status = fetcher.fetch_opencode_quota().await;

        reset_env_var("OPENCODE_API_KEY", old_key);
        reset_env_var("OPENCODE_AUTH_PATH", old_auth);
        reset_env_var("OPENCODE_GO_WORKSPACE_ID", old_ws);

        assert!(!status.available);
        let data = status.data.unwrap();
        assert_eq!(
            data.workspace_url,
            Some("https://opencode.ai/workspace/wrk_NOKEY/go".to_string())
        );
    }

    #[tokio::test]
    async fn test_fetch_opencode_quota_no_api_key_no_workspace() {
        let _lock = ENV_MUTEX.lock().unwrap();
        let old_key = std::env::var("OPENCODE_API_KEY").ok();
        let old_auth = std::env::var("OPENCODE_AUTH_PATH").ok();
        let old_ws = std::env::var("OPENCODE_GO_WORKSPACE_ID").ok();
        std::env::remove_var("OPENCODE_API_KEY");
        std::env::set_var("OPENCODE_AUTH_PATH", "/tmp/nonexistent-opencode-auth.json");
        std::env::remove_var("OPENCODE_GO_WORKSPACE_ID");

        let fetcher = QuotaFetcher::new();
        let status = fetcher.fetch_opencode_quota().await;

        reset_env_var("OPENCODE_API_KEY", old_key);
        reset_env_var("OPENCODE_AUTH_PATH", old_auth);
        reset_env_var("OPENCODE_GO_WORKSPACE_ID", old_ws);

        assert!(!status.available);
        assert!(status.data.is_none());
        assert!(status.error.unwrap().contains("API key not found"));
    }

    #[tokio::test]
    async fn test_fetch_opencode_quota_usage_unavailable() {
        let _lock = ENV_MUTEX.lock().unwrap();
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/v1/dashboard/billing/subscription"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "hard_limit_usd": 100.0,
                "plan": { "title": "Go Plan" }
            })))
            .mount(&mock_server)
            .await;

        Mock::given(method("GET"))
            .and(path("/v1/dashboard/billing/usage"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&mock_server)
            .await;

        let old_key = std::env::var("OPENCODE_API_KEY").ok();
        let old_url = std::env::var("OPENCODE_BASE_URL").ok();
        std::env::set_var("OPENCODE_API_KEY", "sk-test");
        std::env::set_var("OPENCODE_BASE_URL", mock_server.uri());

        let fetcher = QuotaFetcher::new();
        let status = fetcher.fetch_opencode_quota().await;

        reset_env_var("OPENCODE_API_KEY", old_key);
        reset_env_var("OPENCODE_BASE_URL", old_url);

        assert!(status.available);
        let data = status.data.unwrap();
        assert_eq!(data.plan_type, Some("Go Plan".to_string()));
        assert!(data.total_usage_usd.is_none());
    }
}