//! Fenno subscription quota integration.
//!
//! Fenno uses a short-lived dashboard access token plus a rotating refresh
//! token. Bootstrap credentials are read from the environment, then rotated
//! credentials are persisted in a locked state file so quota polling and
//! blue-green deploys can share the current pair.

use super::types::{FennoQuotaData, FennoQuotaStatus, FennoSubscription};
use fs2::FileExt;
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::{Mutex, RwLock};

const FENNO_API_BASE_URL: &str = "https://api.fenno.ai/api/v1";
const FENNO_TIMEOUT_SECS: u64 = 15;
const TOKEN_REFRESH_SKEW_SECS: i64 = 300;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct FennoCredentials {
    pub(crate) access_token: String,
    pub(crate) refresh_token: String,
    pub(crate) expires_at: i64,
}

impl FennoCredentials {
    fn is_valid_for(&self, now: i64) -> bool {
        !self.access_token.is_empty() && self.expires_at > now + TOKEN_REFRESH_SKEW_SECS
    }
}

#[derive(Clone)]
pub struct FennoAuthManager {
    client: Client,
    base_url: String,
    state_path: PathBuf,
    credentials: Arc<RwLock<Option<FennoCredentials>>>,
    refresh_lock: Arc<Mutex<()>>,
}

impl FennoAuthManager {
    pub fn new(client: Client) -> Self {
        Self::new_with_suffix(client, "")
    }

    pub fn new_ex(client: Client) -> Self {
        Self::new_with_suffix(client, "_EX")
    }

    fn new_with_suffix(client: Client, env_suffix: &str) -> Self {
        let state_path = get_state_path(env_suffix);
        let credentials = load_state(&state_path).or_else(|| load_bootstrap_credentials(env_suffix));
        Self::with_config(client, FENNO_API_BASE_URL, state_path, credentials)
    }

    fn with_config(
        client: Client,
        base_url: impl Into<String>,
        state_path: PathBuf,
        credentials: Option<FennoCredentials>,
    ) -> Self {
        Self {
            client,
            base_url: base_url.into().trim_end_matches('/').to_string(),
            state_path,
            credentials: Arc::new(RwLock::new(credentials)),
            refresh_lock: Arc::new(Mutex::new(())),
        }
    }

    #[cfg(test)]
    fn with_test_config(
        client: Client,
        base_url: impl Into<String>,
        state_path: PathBuf,
        bootstrap: Option<FennoCredentials>,
    ) -> Self {
        let credentials = load_state(&state_path).or(bootstrap);
        Self::with_config(client, base_url, state_path, credentials)
    }

    async fn access_token(&self) -> Result<String, String> {
        let now = unix_now();
        if let Some(credentials) = self.credentials.read().await.clone() {
            if credentials.is_valid_for(now) {
                return Ok(credentials.access_token);
            }
        }

        self.refresh(None).await
    }

    async fn refresh(&self, failed_access_token: Option<&str>) -> Result<String, String> {
        let _refresh_guard = self.refresh_lock.lock().await;

        if let Some(failed_access_token) = failed_access_token {
            if let Some(credentials) = self.credentials.read().await.clone() {
                if credentials.access_token != failed_access_token
                    && credentials.is_valid_for(unix_now())
                {
                    return Ok(credentials.access_token);
                }
            }
        } else if let Some(credentials) = self.credentials.read().await.clone() {
            if credentials.is_valid_for(unix_now()) {
                return Ok(credentials.access_token);
            }
        }

        let _state_lock = StateFileLock::acquire(&self.state_path)?;

        // Another backend instance may have refreshed while this instance was
        // waiting for the process-wide file lock.
        if let Some(credentials) = load_state(&self.state_path) {
            if failed_access_token.is_some_and(|failed| {
                credentials.access_token != failed && credentials.is_valid_for(unix_now())
            })
                || failed_access_token.is_none() && credentials.is_valid_for(unix_now())
            {
                let access_token = credentials.access_token.clone();
                *self.credentials.write().await = Some(credentials);
                return Ok(access_token);
            }
        }

        let credentials = self.credentials.read().await.clone().ok_or_else(|| {
            "FENNO_AUTH_TOKEN and FENNO_REFRESH_TOKEN are not configured".to_string()
        })?;
        if credentials.refresh_token.is_empty() {
            return Err("FENNO_REFRESH_TOKEN is empty".to_string());
        }

        let response = self
            .client
            .post(format!("{}/auth/refresh", self.base_url))
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({ "refresh_token": credentials.refresh_token }))
            .timeout(std::time::Duration::from_secs(FENNO_TIMEOUT_SECS))
            .send()
            .await
            .map_err(|e| format!("Fenno token refresh network error: {e}"))?;

        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|e| format!("Fenno token refresh response error: {e}"))?;
        if !status.is_success() {
            return Err(format!(
                "Fenno token refresh returned HTTP {}: {}",
                status,
                super::truncate_error_body(&body)
            ));
        }

        let envelope: FennoRefreshEnvelope = serde_json::from_str(&body)
            .map_err(|e| format!("Fenno token refresh parse error: {e}"))?;
        let refreshed = envelope
            .data
            .ok_or_else(|| format!("Fenno token refresh failed: {}", envelope.message))?;
        if refreshed.access_token.is_empty() || refreshed.refresh_token.is_empty() {
            return Err("Fenno token refresh returned incomplete credentials".to_string());
        }

        let next = FennoCredentials {
            access_token: refreshed.access_token,
            refresh_token: refreshed.refresh_token,
            expires_at: unix_now() + refreshed.expires_in,
        };
        persist_state(&self.state_path, &next)?;
        let access_token = next.access_token.clone();
        *self.credentials.write().await = Some(next);
        Ok(access_token)
    }

    async fn get_json<T: DeserializeOwned>(&self, path: &str) -> Result<T, String> {
        let access_token = self.access_token().await?;
        let response = self.send_get(path, &access_token).await?;
        if response.status() == StatusCode::UNAUTHORIZED {
            let refreshed_access_token = self.refresh(Some(&access_token)).await?;
            let retry = self.send_get(path, &refreshed_access_token).await?;
            return parse_json_response(retry).await;
        }

        parse_json_response(response).await
    }

    async fn send_get(&self, path: &str, access_token: &str) -> Result<reqwest::Response, String> {
        self.client
            .get(format!("{}{}", self.base_url, path))
            .bearer_auth(access_token)
            .header("Accept", "application/json")
            .timeout(std::time::Duration::from_secs(FENNO_TIMEOUT_SECS))
            .send()
            .await
            .map_err(|e| format!("Fenno quota network error: {e}"))
    }
}

#[derive(Debug, Deserialize)]
struct FennoRefreshEnvelope {
    #[serde(default)]
    message: String,
    data: Option<FennoRefreshData>,
}

#[derive(Debug, Deserialize)]
struct FennoRefreshData {
    #[serde(default)]
    access_token: String,
    #[serde(default)]
    refresh_token: String,
    #[serde(default)]
    expires_in: i64,
}

#[derive(Debug, Deserialize)]
struct FennoSubscriptionsEnvelope {
    #[serde(default)]
    data: Vec<FennoSubscription>,
}

pub async fn fetch_fenno_quota(auth: &FennoAuthManager) -> FennoQuotaStatus {
    let result: Result<FennoSubscriptionsEnvelope, String> =
        auth.get_json("/subscriptions/active").await;

    match result {
        Ok(envelope) => FennoQuotaStatus {
            available: true,
            data: Some(FennoQuotaData {
                subscriptions: envelope.data,
            }),
            error: None,
        },
        Err(error) => {
            tracing::warn!("Fenno quota fetch failed: {error}");
            FennoQuotaStatus {
                available: false,
                data: None,
                error: Some(error),
            }
        }
    }
}

async fn parse_json_response<T: DeserializeOwned>(
    response: reqwest::Response,
) -> Result<T, String> {
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|e| format!("Fenno quota response error: {e}"))?;
    if !status.is_success() {
        return Err(format!(
            "Fenno quota returned HTTP {}: {}",
            status,
            super::truncate_error_body(&body)
        ));
    }
    serde_json::from_str(&body).map_err(|e| format!("Fenno quota parse error: {e}"))
}

fn load_bootstrap_credentials(suffix: &str) -> Option<FennoCredentials> {
    let access_token = std::env::var(format!("FENNO_AUTH_TOKEN{suffix}")).ok()?.trim().to_string();
    let refresh_token = std::env::var(format!("FENNO_REFRESH_TOKEN{suffix}"))
        .ok()?
        .trim()
        .to_string();
    if access_token.is_empty() || refresh_token.is_empty() {
        return None;
    }
    Some(FennoCredentials {
        expires_at: jwt_expiry(&access_token).unwrap_or(0),
        access_token,
        refresh_token,
    })
}

fn get_state_path(suffix: &str) -> PathBuf {
    let env_key = format!("FENNO_AUTH_STATE_PATH{suffix}");
    if let Ok(path) = std::env::var(&env_key) {
        if !path.trim().is_empty() {
            return PathBuf::from(path);
        }
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let filename = if suffix.is_empty() {
        "fenno-auth.json".to_string()
    } else {
        format!("fenno-auth{}.json", suffix.to_lowercase().replace('_', "-"))
    };
    PathBuf::from(home)
        .join(".config")
        .join("token-stats")
        .join(filename)
}

fn load_state(path: &Path) -> Option<FennoCredentials> {
    let content = fs::read(path).ok()?;
    let credentials: FennoCredentials = serde_json::from_slice(&content).ok()?;
    if credentials.access_token.is_empty() || credentials.refresh_token.is_empty() {
        return None;
    }
    Some(credentials)
}

fn persist_state(path: &Path, credentials: &FennoCredentials) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("create Fenno auth directory: {e}"))?;
        set_private_directory_permissions(parent)?;
    }

    let temp_path = path.with_extension(format!("json.tmp-{}", std::process::id()));
    let content = serde_json::to_vec_pretty(credentials)
        .map_err(|e| format!("serialize Fenno auth state: {e}"))?;
    fs::write(&temp_path, content).map_err(|e| format!("write Fenno auth state: {e}"))?;
    set_private_file_permissions(&temp_path)?;
    fs::rename(&temp_path, path).map_err(|e| format!("replace Fenno auth state: {e}"))
}

fn set_private_directory_permissions(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|e| format!("protect Fenno auth directory: {e}"))?;
    }
    Ok(())
}

fn set_private_file_permissions(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .map_err(|e| format!("protect Fenno auth state: {e}"))?;
    }
    Ok(())
}

struct StateFileLock(File);

impl StateFileLock {
    fn acquire(state_path: &Path) -> Result<Self, String> {
        if let Some(parent) = state_path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("create Fenno auth directory: {e}"))?;
            set_private_directory_permissions(parent)?;
        }
        let lock_path = state_path.with_extension("lock");
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(lock_path)
            .map_err(|e| format!("open Fenno auth lock: {e}"))?;
        file.lock_exclusive()
            .map_err(|e| format!("lock Fenno auth state: {e}"))?;
        Ok(Self(file))
    }
}

impl Drop for StateFileLock {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn jwt_expiry(token: &str) -> Option<i64> {
    let payload = token.split('.').nth(1)?;
    let decoded = base64_url_decode(payload)?;
    serde_json::from_slice::<serde_json::Value>(&decoded)
        .ok()?
        .get("exp")?
        .as_i64()
}

fn base64_url_decode(value: &str) -> Option<Vec<u8>> {
    let mut encoded = value.replace('-', "+").replace('_', "/");
    while !encoded.len().is_multiple_of(4) {
        encoded.push('=');
    }
    decode_base64(&encoded)
}

fn decode_base64(value: &str) -> Option<Vec<u8>> {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes = value.as_bytes();
    let mut output = Vec::with_capacity(bytes.len() * 3 / 4);
    let mut buffer = 0u32;
    let mut bits = 0u8;
    for &byte in bytes {
        if byte == b'=' {
            break;
        }
        let value = ALPHABET.iter().position(|candidate| *candidate == byte)? as u32;
        buffer = (buffer << 6) | value;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            output.push((buffer >> bits) as u8);
            buffer &= (1 << bits) - 1;
        }
    }
    Some(output)
}

#[cfg(test)]
mod tests {
    use super::FennoCredentials;
    use super::{FennoAuthManager, FennoQuotaData, fetch_fenno_quota};
    use reqwest::Client;
    use serde_json::json;
    use std::fs;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use std::time::{SystemTime, UNIX_EPOCH};
    use tempfile::tempdir;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn future_expiry() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after unix epoch")
            .as_secs() as i64
            + 3600
    }

    fn credentials(access_token: &str, refresh_token: &str, expires_at: i64) -> FennoCredentials {
        FennoCredentials {
            access_token: access_token.to_string(),
            refresh_token: refresh_token.to_string(),
            expires_at,
        }
    }

    #[tokio::test]
    async fn uses_persisted_credentials_before_bootstrap_credentials() {
        let dir = tempdir().expect("tempdir");
        let state_path = dir.path().join("fenno-auth.json");
        fs::write(
            &state_path,
            serde_json::to_vec(&credentials(
                "persisted-access",
                "persisted-refresh",
                future_expiry(),
            ))
            .expect("serialize credentials"),
        )
        .expect("write state");

        let manager = FennoAuthManager::with_test_config(
            Client::new(),
            "http://127.0.0.1:1",
            state_path,
            Some(credentials(
                "bootstrap-access",
                "bootstrap-refresh",
                future_expiry(),
            )),
        );

        assert_eq!(
            manager.access_token().await.expect("access token"),
            "persisted-access"
        );
    }

    #[tokio::test]
    async fn refreshes_rotated_credentials_and_persists_the_new_pair() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/auth/refresh"))
            .and(header("content-type", "application/json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "code": 0,
                "message": "success",
                "data": {
                    "access_token": "rotated-access",
                    "refresh_token": "rotated-refresh",
                    "expires_in": 86400,
                    "token_type": "Bearer"
                }
            })))
            .expect(1)
            .mount(&server)
            .await;

        let dir = tempdir().expect("tempdir");
        let state_path = dir.path().join("fenno-auth.json");
        let manager = FennoAuthManager::with_test_config(
            Client::new(),
            format!("{}/api/v1", server.uri()),
            state_path.clone(),
            Some(credentials("expired-access", "initial-refresh", 0)),
        );

        assert_eq!(
            manager.access_token().await.expect("refreshed token"),
            "rotated-access"
        );

        let persisted: FennoCredentials =
            serde_json::from_slice(&fs::read(state_path).expect("persisted state"))
                .expect("parse persisted state");
        assert_eq!(persisted.access_token, "rotated-access");
        assert_eq!(persisted.refresh_token, "rotated-refresh");
        assert!(persisted.expires_at > future_expiry() - 120);
    }

    #[tokio::test]
    async fn retries_quota_request_after_unauthorized_with_a_refreshed_token() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/auth/refresh"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "code": 0,
                "message": "success",
                "data": {
                    "access_token": "refreshed-access",
                    "refresh_token": "refreshed-refresh",
                    "expires_in": 86400,
                    "token_type": "Bearer"
                }
            })))
            .mount(&server)
            .await;
        let request_count = Arc::new(AtomicUsize::new(0));
        let request_count_for_responder = Arc::clone(&request_count);
        Mock::given(method("GET"))
            .and(path("/api/v1/subscriptions/active"))
            .respond_with(move |_: &wiremock::Request| {
                if request_count_for_responder.fetch_add(1, Ordering::SeqCst) == 0 {
                    ResponseTemplate::new(401)
                } else {
                    ResponseTemplate::new(200).set_body_json(json!({
                    "code": 0,
                    "message": "success",
                    "data": [{
                        "status": "active",
                        "expires_at": "2026-08-15T15:44:32+08:00",
                        "weekly_usage_usd": 4.5,
                        "monthly_usage_usd": 12.25,
                        "group": {
                            "name": "code-plan/Trial",
                            "platform": "openai",
                            "weekly_limit_usd": 38,
                            "monthly_limit_usd": 150
                        }
                    }]
                    }))
                }
            })
            .expect(2)
            .mount(&server)
            .await;

        let dir = tempdir().expect("tempdir");
        let manager = FennoAuthManager::with_test_config(
            Client::new(),
            format!("{}/api/v1", server.uri()),
            dir.path().join("fenno-auth.json"),
            Some(credentials(
                "expired-at-server",
                "initial-refresh",
                future_expiry(),
            )),
        );

        let quota = fetch_fenno_quota(&manager).await;
        assert!(quota.available, "quota error: {:?}", quota.error);
        let data: FennoQuotaData = quota.data.expect("quota data");
        assert_eq!(data.subscriptions.len(), 1);
        assert_eq!(data.subscriptions[0].weekly_usage_usd, 4.5);
        assert_eq!(data.subscriptions[0].monthly_usage_usd, 12.25);
        assert_eq!(data.subscriptions[0].group.weekly_limit_usd, Some(38.0));
        assert_eq!(data.subscriptions[0].group.monthly_limit_usd, Some(150.0));
    }
}
