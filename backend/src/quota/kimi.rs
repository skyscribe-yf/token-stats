//! Kimi Code provider integration.
//!
//! Handles OAuth token management (read, refresh), API calls to the Kimi Code
//! usage endpoint, and response parsing into the dashboard DTO.

use super::types::*;
use std::path::PathBuf;

// ─── Auth helpers ────────────────────────────────────────────────────────────

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

/// Read Kimi Code OAuth access token from the credentials file.
/// Returns None if the file is missing, unreadable, or the token is expired.
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

/// Attempt to refresh the Kimi Code OAuth access token using the stored
/// refresh_token. On success, updates the credentials file and returns the new
/// access token.
pub async fn refresh_kimi_code_token(client: &reqwest::Client) -> Option<String> {
    let path = get_kimi_credentials_path();
    if !path.exists() {
        return None;
    }

    let content = std::fs::read_to_string(&path).ok()?;
    let creds: serde_json::Value = serde_json::from_str(&content).ok()?;

    let refresh_token = creds.get("refresh_token").and_then(|v| v.as_str())?;
    if refresh_token.is_empty() {
        return None;
    }

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

            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as f64;
            let expires_in = refresh_resp.expires_in.unwrap_or(900.0);
            let new_expires_at = now + expires_in;

            // Update the credentials file
            let mut new_creds = creds.clone();
            new_creds["access_token"] =
                serde_json::Value::String(refresh_resp.access_token.clone());
            new_creds["expires_at"] = serde_json::Value::Number(
                serde_json::Number::from_f64(new_expires_at).unwrap_or(serde_json::Number::from(0)),
            );
            if let Some(new_refresh) = &refresh_resp.refresh_token {
                new_creds["refresh_token"] = serde_json::Value::String(new_refresh.clone());
            }
            new_creds["expires_in"] = serde_json::Value::Number(
                serde_json::Number::from_f64(expires_in).unwrap_or(serde_json::Number::from(900)),
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
            tracing::info!(
                "Kimi Code token refreshed successfully, expires in {}s",
                expires_in as i64
            );
            Some(refresh_resp.access_token)
        }
        Ok(r) => {
            let status = r.status();
            let body = r.text().await.unwrap_or_default();
            tracing::warn!(
                "Kimi Code token refresh failed: {} {}",
                status,
                super::truncate_error_body(&body)
            );
            None
        }
        Err(e) => {
            tracing::warn!("Kimi Code token refresh request failed: {}", e);
            None
        }
    }
}

/// Kimi Auth API base URL for token refresh.
pub fn get_kimi_auth_base_url() -> String {
    std::env::var("KIMI_AUTH_BASE_URL").unwrap_or_else(|_| "https://auth.kimi.com".to_string())
}

/// Kimi Code API base URL for usage queries.
pub fn get_kimi_code_base_url() -> String {
    std::env::var("KIMI_CODE_BASE_URL")
        .unwrap_or_else(|_| "https://api.kimi.com/coding/v1".to_string())
}

// ─── Kimi Quota Fetching ─────────────────────────────────────────────────────

/// Fetch Kimi Code usage and build the status DTO.
pub async fn fetch_kimi_quota(client: &reqwest::Client) -> KimiQuotaStatus {
    let access_token = match get_kimi_code_access_token() {
        Some(t) => t,
        None => {
            tracing::info!("Kimi Code access token expired or missing, attempting refresh");
            match refresh_kimi_code_token(client).await {
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

    let response = match client
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
            error: Some(format!(
                "API returned {}: {}",
                status,
                super::truncate_error_body(&body)
            )),
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

    let quota = parse_kimi_usage(&body);
    KimiQuotaStatus {
        available: true,
        data: Some(quota),
        error: None,
    }
}

/// Extract quota information from the parsed Kimi Code usage response.
fn parse_kimi_usage(body: &KimiCodeUsageResponse) -> QuotaKimiCode {
    let mut weekly_limit: i64 = 0;
    let mut weekly_used: i64 = 0;
    let mut weekly_remaining: i64 = 0;
    let mut weekly_reset_time: Option<String> = None;
    let mut rp5h_limit: i64 = 0;
    let mut rp5h_used: i64 = 0;
    let mut rp5h_remaining: i64 = 0;
    let mut rp5h_reset_time: Option<String> = None;

    if let Some(usage) = &body.usage {
        weekly_limit = usage.limit as i64;
        weekly_used = usage.used as i64;
        weekly_remaining = usage.remaining as i64;
        weekly_reset_time = usage.reset_time.clone();
    }

    for limit in &body.limits {
        if let Some(detail) = &limit.detail {
            let window_duration = limit.window.as_ref().and_then(|w| w.duration);
            let time_unit = limit.window.as_ref().and_then(|w| w.time_unit.as_deref());

            let is_5h = (window_duration == Some(5) && time_unit == Some("TIME_UNIT_HOUR"))
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
    let parallel_limit = body.parallel.as_ref().map(|p| p.limit as i64).unwrap_or(0);
    let membership_level = body
        .user
        .as_ref()
        .and_then(|u| u.membership.as_ref().and_then(|m| m.level.clone()));
    let sub_type = body.sub_type.clone();

    QuotaKimiCode {
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
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    static ENV_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn reset_env_var(name: &str, old: Option<String>) {
        match old {
            Some(v) => std::env::set_var(name, v),
            None => std::env::remove_var(name),
        }
    }

    #[test]
    fn test_get_kimi_code_base_url_default() {
        temp_env::with_var("KIMI_CODE_BASE_URL", None::<&str>, || {
            assert_eq!(get_kimi_code_base_url(), "https://api.kimi.com/coding/v1");
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
    fn test_parse_kimi_usage_success() {
        let json = r#"{
            "usage": {
                "limit": "100", "used": "8", "remaining": "92",
                "resetTime": "2026-05-23T14:00:42.150347Z"
            },
            "limits": [{
                "window": {"duration": 300, "timeUnit": "TIME_UNIT_MINUTE"},
                "detail": {
                    "limit": "100", "used": "4", "remaining": "96",
                    "resetTime": "2026-05-17T17:00:42.150347Z"
                }
            }],
            "totalQuota": {"limit": "100", "remaining": "99"},
            "user": {"userId": "abc123", "membership": {"level": "LEVEL_INTERMEDIATE"}},
            "parallel": {"limit": "20"},
            "subType": "TYPE_PURCHASE"
        }"#;

        let resp: KimiCodeUsageResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.usage.as_ref().map(|u| u.limit as i64), Some(100));
        assert_eq!(resp.limits.len(), 1);

        let quota = parse_kimi_usage(&resp);
        assert_eq!(quota.weekly_limit, 100);
        assert_eq!(quota.rp5h_limit, 100);
        assert_eq!(quota.total_limit, 100);
        assert_eq!(quota.parallel_limit, 20);
    }

    #[test]
    fn test_parse_kimi_usage_empty() {
        let json = "{}";
        let resp: KimiCodeUsageResponse = serde_json::from_str(json).unwrap();
        assert!(resp.usage.is_none());
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn test_fetch_kimi_quota_success() {
        let _lock = ENV_MUTEX.lock().unwrap();
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/usages"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "usage": {"limit": "100", "used": "8", "remaining": "92",
                          "resetTime": "2026-05-23T14:00:42Z"},
                "limits": [{
                    "window": {"duration": 300, "timeUnit": "TIME_UNIT_MINUTE"},
                    "detail": {"limit": "100", "used": "4", "remaining": "96"}
                }],
                "totalQuota": {"limit": "100", "remaining": "99"},
                "parallel": {"limit": "20"},
                "subType": "TYPE_PURCHASE"
            })))
            .mount(&mock_server)
            .await;

        let tmp_dir = tempfile::tempdir().unwrap();
        let cred_path = tmp_dir.path().join("kimi-code.json");
        std::fs::write(
            &cred_path,
            serde_json::json!({
                "access_token": "test-token-123",
                "expires_at": std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as f64 + 3600.0
            })
            .to_string(),
        )
        .unwrap();

        let old_path = std::env::var("KIMI_CREDENTIALS_PATH").ok();
        let old_url = std::env::var("KIMI_CODE_BASE_URL").ok();
        std::env::set_var("KIMI_CREDENTIALS_PATH", cred_path.to_str().unwrap());
        std::env::set_var("KIMI_CODE_BASE_URL", mock_server.uri());

        let client = reqwest::Client::new();
        let status = fetch_kimi_quota(&client).await;

        reset_env_var("KIMI_CREDENTIALS_PATH", old_path);
        reset_env_var("KIMI_CODE_BASE_URL", old_url);

        assert!(status.available);
        let data = status.data.unwrap();
        assert_eq!(data.weekly_limit, 100);
        assert_eq!(data.rp5h_limit, 100);
        assert_eq!(data.parallel_limit, 20);
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn test_fetch_kimi_quota_no_token() {
        let _lock = ENV_MUTEX.lock().unwrap();
        let old_path = std::env::var("KIMI_CREDENTIALS_PATH").ok();
        std::env::set_var(
            "KIMI_CREDENTIALS_PATH",
            "/tmp/nonexistent-kimi-creds-test.json",
        );

        let client = reqwest::Client::new();
        let status = fetch_kimi_quota(&client).await;

        reset_env_var("KIMI_CREDENTIALS_PATH", old_path);
        assert!(!status.available);
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn test_fetch_kimi_quota_api_error() {
        let _lock = ENV_MUTEX.lock().unwrap();
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/usages"))
            .respond_with(
                ResponseTemplate::new(401).set_body_string(r#"{"error":"invalid_token"}"#),
            )
            .mount(&mock_server)
            .await;

        let tmp_dir = tempfile::tempdir().unwrap();
        let cred_path = tmp_dir.path().join("kimi-code.json");
        std::fs::write(
            &cred_path,
            serde_json::json!({
                "access_token": "bad-token",
                "expires_at": std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as f64 + 3600.0
            })
            .to_string(),
        )
        .unwrap();

        let old_path = std::env::var("KIMI_CREDENTIALS_PATH").ok();
        let old_url = std::env::var("KIMI_CODE_BASE_URL").ok();
        std::env::set_var("KIMI_CREDENTIALS_PATH", cred_path.to_str().unwrap());
        std::env::set_var("KIMI_CODE_BASE_URL", mock_server.uri());

        let client = reqwest::Client::new();
        let status = fetch_kimi_quota(&client).await;

        reset_env_var("KIMI_CREDENTIALS_PATH", old_path);
        reset_env_var("KIMI_CODE_BASE_URL", old_url);

        assert!(!status.available);
    }
}
