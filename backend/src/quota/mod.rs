//! Quota/balance fetchers for external AI API providers.
//!
//! Supports:
//! - **Kimi Code**: Usage via `GET /usages` on the Kimi Code platform (OAuth access token)
//! - **OpenCode-go**: Usage via direct HTTP request to workspace dashboard (HTML parsing)

pub mod commandcode;
pub mod kimi;
pub mod opencode;
pub mod types;
pub mod xiaomi_mimo;

pub use types::{
    CommandCodeQuotaStatus, KimiQuotaStatus, OpenCodeQuotaStatus, QuotaResponse,
    XiaomiMiMoQuotaStatus,
};

use serde::de;

/// Deserializer that accepts both string and numeric values for number fields.
/// The Kimi Code API returns numbers as strings (e.g. "100" instead of 100).
pub(crate) fn deserialize_flexible_number<'de, D>(deserializer: D) -> Result<f64, D::Error>
where
    D: serde::Deserializer<'de>,
{
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

/// Truncate error response body for display. Long HTML responses are masked.
pub(crate) fn truncate_error_body(body: &str) -> String {
    const MAX_ERROR_BODY_LEN: usize = 200;
    if body.len() <= MAX_ERROR_BODY_LEN {
        return body.to_string();
    }
    if body.trim_start().starts_with("<!") || body.trim_start().starts_with("<html") {
        return "(HTML response, content omitted)".to_string();
    }
    format!("{}...", &body[..MAX_ERROR_BODY_LEN])
}

// ─── Fetcher ─────────────────────────────────────────────────────────────────

/// Quota fetcher with injectable HTTP client for testability.
pub struct QuotaFetcher {
    pub client: reqwest::Client,
}

impl QuotaFetcher {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }

    /// Fetch Kimi Code usage from the Kimi Code platform API.
    pub async fn fetch_kimi_quota(&self) -> KimiQuotaStatus {
        kimi::fetch_kimi_quota(&self.client).await
    }

    /// Fetch OpenCode-go subscription/quota info via HTTP + HTML scraping.
    pub async fn fetch_opencode_quota(&self) -> OpenCodeQuotaStatus {
        opencode::fetch_opencode_quota(&self.client).await
    }

    /// Fetch OpenCode-go **EX** workspace subscription/quota info.
    pub async fn fetch_opencode_quota_ex(&self) -> OpenCodeQuotaStatus {
        opencode::fetch_opencode_quota_ex(&self.client).await
    }

    /// Fetch Xiaomi MiMo TP token plan usage.
    pub async fn fetch_xiaomi_mimo_quota(&self) -> XiaomiMiMoQuotaStatus {
        xiaomi_mimo::fetch_xiaomi_mimo_quota(&self.client).await
    }

    /// Fetch CommandCode subscription/quota info.
    pub async fn fetch_commandcode_quota(&self) -> CommandCodeQuotaStatus {
        commandcode::fetch_commandcode_quota(&self.client).await
    }
}

impl Default for QuotaFetcher {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Tests (shared types) ────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::types::*;

    #[test]
    fn test_quota_error_display() {
        let err = QuotaError::new("kimi", "token expired");
        assert_eq!(format!("{}", err), "[kimi] token expired");
    }
}
