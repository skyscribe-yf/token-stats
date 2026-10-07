//! StepFun (platform.stepfun.com) account balance + Step Plan pool fetcher.
//!
//! The account-overview page's credit balance is exposed by the official
//! OpenAPI: `GET https://api.stepfun.com/v1/accounts` with
//! `Authorization: Bearer <STEPFUN_API_KEY>` — the same key the CPA `stepfun`
//! upstream uses. Returns `balance` (可用余额, CNY), plus the cash/voucher
//! split (`total_cash_balance` 总充值 / `total_voucher_balance` 总赠送).
//!
//! The Step Plan monthly pool is a separate console Connect-RPC. Its
//! `Oasis-Token` is a ~30-minute access JWT, not a year-long cookie: the
//! cookie `expires` date is just the browser's storage lifetime. The passport
//! `RefreshToken` RPC rotates `accessToken` + `refreshToken` (both short-lived).
//! Bootstrap values come from `STEPFUN_OASIS_TOKEN` (bare access JWT, or the
//! CodexBar `access...refresh` pair) plus `STEPFUN_OASIS_WEBID`; rotated pairs
//! are persisted so later polls and blue-green deploys keep a live session.

use super::types::*;
use fs2::FileExt;
use reqwest::Client;
use serde::Deserialize;
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::{Mutex, RwLock};
use tracing::warn;

const STEPFUN_API_BASE: &str = "https://api.stepfun.com";
const CONSOLE_RPC_PATH: &str = "/api/step.openapi.devcenter.Dashboard";
const REFRESH_PATH: &str = "/passport/proto.api.passport.v1.PassportService/RefreshToken";
const HTTP_TIMEOUT_SECS: u64 = 15;
/// Refresh before the access JWT lapses. Passport returns `duration: 1800`.
const TOKEN_REFRESH_SKEW_SECS: i64 = 120;

#[derive(Debug, Clone, PartialEq, Eq)]
struct ConsoleSession {
    access_token: String,
    refresh_token: String,
    webid: String,
    expires_at: i64,
}

impl ConsoleSession {
    fn is_valid_for(&self, now: i64) -> bool {
        !self.access_token.is_empty() && self.expires_at > now + TOKEN_REFRESH_SKEW_SECS
    }

    /// Header/cookie value the console actually accepts.
    ///
    /// A bare access JWT comes back from `RefreshToken` with `mode=2`, but
    /// `QueryStepPlanRateLimit` rejects it as "token is illegal". The browser
    /// cookie and a working probe both send `access...refresh`. A bare access
    /// token (no refresh half yet) is sent as-is.
    fn oasis_token(&self) -> String {
        if self.refresh_token.is_empty() {
            self.access_token.clone()
        } else {
            format!("{}...{}", self.access_token, self.refresh_token)
        }
    }

    fn cookie_header(&self) -> String {
        format!(
            "Oasis-Token={}; Oasis-Webid={}",
            self.oasis_token(),
            self.webid
        )
    }
}

#[derive(Clone)]
pub struct StepFunAuthManager {
    client: Client,
    /// Console origin (`https://platform.stepfun.com` in production). Tests
    /// point this at a local mock so RefreshToken and the plan RPCs share it.
    console_origin: String,
    state_path: PathBuf,
    session: Arc<RwLock<Option<ConsoleSession>>>,
    refresh_lock: Arc<Mutex<()>>,
}

impl StepFunAuthManager {
    pub fn new(client: Client) -> Self {
        let state_path = state_path_from_env();
        let session = load_state(&state_path).or_else(load_bootstrap_session);
        Self::with_config(client, CONSOLE_ORIGIN, state_path, session)
    }

    fn with_config(
        client: Client,
        console_origin: impl Into<String>,
        state_path: PathBuf,
        session: Option<ConsoleSession>,
    ) -> Self {
        Self {
            client,
            console_origin: console_origin.into().trim_end_matches('/').to_string(),
            state_path,
            session: Arc::new(RwLock::new(session)),
            refresh_lock: Arc::new(Mutex::new(())),
        }
    }

    #[cfg(test)]
    fn with_test_config(
        client: Client,
        console_origin: impl Into<String>,
        state_path: PathBuf,
        session: Option<ConsoleSession>,
    ) -> Self {
        let session = load_state(&state_path).or(session);
        Self::with_config(client, console_origin, state_path, session)
    }

    async fn session(&self) -> Result<ConsoleSession, String> {
        let now = unix_now();
        if let Some(session) = self.session.read().await.clone()
            && session.is_valid_for(now)
        {
            return Ok(session);
        }
        self.refresh(None).await
    }

    async fn refresh(&self, failed_access_token: Option<&str>) -> Result<ConsoleSession, String> {
        let _refresh_guard = self.refresh_lock.lock().await;

        if let Some(failed_access_token) = failed_access_token {
            if let Some(session) = self.session.read().await.clone()
                && session.access_token != failed_access_token
                && session.is_valid_for(unix_now())
            {
                return Ok(session);
            }
        } else if let Some(session) = self.session.read().await.clone()
            && session.is_valid_for(unix_now())
        {
            return Ok(session);
        }

        let _state_lock = StateFileLock::acquire(&self.state_path)?;
        if let Some(session) = load_state(&self.state_path)
            && (failed_access_token.is_some_and(|failed| {
                session.access_token != failed && session.is_valid_for(unix_now())
            }) || (failed_access_token.is_none() && session.is_valid_for(unix_now())))
        {
            *self.session.write().await = Some(session.clone());
            return Ok(session);
        }

        let current = self
            .session
            .read()
            .await
            .clone()
            .or_else(|| load_state(&self.state_path))
            .or_else(load_bootstrap_session)
            .ok_or_else(|| "STEPFUN_OASIS_TOKEN/WEBID not set".to_string())?;
        if current.refresh_token.is_empty() {
            return Err(
                "Step Plan 登录态已过期，且没有 refresh token；请重新登录 platform.stepfun.com 后运行 ./scripts/extract-stepfun-token.sh"
                    .to_string(),
            );
        }

        let refreshed = self
            .refresh_remote(&current.oasis_token(), &current.webid)
            .await?;
        persist_state(&self.state_path, &refreshed)?;
        *self.session.write().await = Some(refreshed.clone());
        Ok(refreshed)
    }

    async fn refresh_remote(
        &self,
        oasis_token: &str,
        webid: &str,
    ) -> Result<ConsoleSession, String> {
        let response = self
            .client
            .post(format!("{}{}", self.console_origin, REFRESH_PATH))
            .json(&serde_json::json!({}))
            .header("Content-Type", "application/json")
            .header("Oasis-Token", oasis_token)
            .header("Oasis-Webid", webid)
            .header("Oasis-appID", "10300")
            .header("Oasis-Platform", "web")
            .header(
                "Cookie",
                format!("Oasis-Token={oasis_token}; Oasis-Webid={webid}"),
            )
            .timeout(std::time::Duration::from_secs(HTTP_TIMEOUT_SECS))
            .send()
            .await
            .map_err(|e| format!("StepFun RefreshToken request failed: {e}"))?;

        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|e| format!("StepFun RefreshToken body error: {e}"))?;
        if !status.is_success() {
            return Err(format!(
                "StepFun RefreshToken HTTP {}: {}",
                status,
                truncate_body(&body)
            ));
        }
        let parsed: RefreshResponse = serde_json::from_str(&body)
            .map_err(|e| format!("StepFun RefreshToken parse error: {e}"))?;
        let access_pair = parsed.access_token;
        let refresh_pair = parsed.refresh_token;
        let access = access_pair
            .as_ref()
            .and_then(|pair| pair.raw.clone())
            .filter(|raw| !raw.is_empty())
            .ok_or_else(|| "StepFun RefreshToken returned no access token".to_string())?;
        let refresh = refresh_pair
            .as_ref()
            .and_then(|pair| pair.raw.clone())
            .filter(|raw| !raw.is_empty())
            .or_else(|| {
                oasis_token
                    .split_once("...")
                    .map(|(_, refresh)| refresh.to_string())
            })
            .filter(|raw| !raw.is_empty())
            .ok_or_else(|| "StepFun RefreshToken returned no refresh token".to_string())?;
        let expires_at = unix_now()
            + access_pair
                .as_ref()
                .and_then(|pair| pair.duration)
                .filter(|duration| *duration > 0)
                .unwrap_or(1800);
        Ok(ConsoleSession {
            access_token: access,
            refresh_token: refresh,
            webid: webid.to_string(),
            expires_at,
        })
    }
}

#[derive(Debug, Deserialize)]
struct RefreshResponse {
    #[serde(rename = "accessToken", alias = "access_token")]
    access_token: Option<TokenPair>,
    #[serde(rename = "refreshToken", alias = "refresh_token")]
    refresh_token: Option<TokenPair>,
}

#[derive(Debug, Clone, Deserialize)]
struct TokenPair {
    raw: Option<String>,
    #[serde(default)]
    duration: Option<i64>,
}

/// Fetch the StepFun credit account balance and, when a console session is
/// configured, the Step Plan monthly pool.
pub async fn fetch_stepfun_quota(client: &Client, auth: &StepFunAuthManager) -> StepFunQuotaStatus {
    let api_key = match get_api_key() {
        Some(k) => k,
        None => {
            return StepFunQuotaStatus {
                available: false,
                data: None,
                error: Some("STEPFUN_API_KEY not set".to_string()),
            };
        }
    };

    let mut data = match fetch_account(client, &api_key).await {
        Ok(d) => d,
        Err(e) => {
            warn!("StepFun account fetch failed: {e}");
            return StepFunQuotaStatus {
                available: false,
                data: None,
                error: Some(format!("Failed to fetch StepFun account: {e}")),
            };
        }
    };

    match fetch_plan_quota(client, auth).await {
        Ok(plan) => data.plan = Some(plan),
        Err(error) => {
            warn!("StepFun Step Plan pool unavailable: {error}");
            data.plan_error = Some(error);
        }
    }

    StepFunQuotaStatus {
        available: true,
        data: Some(data),
        error: None,
    }
}

/// Read `STEPFUN_API_KEY` from the environment.
pub fn get_api_key() -> Option<String> {
    std::env::var("STEPFUN_API_KEY")
        .ok()
        .filter(|k| !k.is_empty())
}

async fn fetch_account(client: &Client, api_key: &str) -> Result<StepFunQuotaData, String> {
    let url = format!("{}/v1/accounts", STEPFUN_API_BASE);
    let response = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", api_key))
        .timeout(std::time::Duration::from_secs(HTTP_TIMEOUT_SECS))
        .send()
        .await
        .map_err(|e| format!("Request failed: {e}"))?;

    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status()));
    }

    let value = response
        .json::<serde_json::Value>()
        .await
        .map_err(|e| format!("Parse error: {e}"))?;

    if value["object"].as_str() != Some("account") {
        return Err("unexpected response shape".to_string());
    }

    Ok(StepFunQuotaData {
        account_type: value["type"].as_str().unwrap_or("").to_string(),
        balance: value["balance"].as_f64().unwrap_or(0.0),
        total_cash_balance: value["total_cash_balance"].as_f64().unwrap_or(0.0),
        total_voucher_balance: value["total_voucher_balance"].as_f64().unwrap_or(0.0),
        plan: None,
        plan_error: None,
    })
}

/// The console gateway serialises int64 fields as strings.
fn as_i64(value: &serde_json::Value) -> i64 {
    match value {
        serde_json::Value::String(s) => s.parse().unwrap_or(0),
        serde_json::Value::Number(n) => n.as_i64().unwrap_or(0),
        _ => 0,
    }
}

async fn console_rpc(
    client: &Client,
    origin: &str,
    session: &ConsoleSession,
    method: &str,
) -> Result<serde_json::Value, String> {
    let url = format!("{}{}/{}", origin, CONSOLE_RPC_PATH, method);
    let response = client
        .post(&url)
        .json(&serde_json::json!({}))
        .header("Oasis-Token", session.oasis_token())
        .header("Oasis-Webid", &session.webid)
        .header("Oasis-appID", "10300")
        .header("Oasis-Platform", "web")
        .header("Cookie", session.cookie_header())
        .timeout(std::time::Duration::from_secs(HTTP_TIMEOUT_SECS))
        .send()
        .await
        .map_err(|e| format!("Request failed: {e}"))?;

    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|e| format!("Parse error: {e}"))?;
    if status == reqwest::StatusCode::UNAUTHORIZED {
        return Err(format!("HTTP 401: {}", truncate_body(&body)));
    }
    if !status.is_success() {
        return Err(format!("HTTP {}: {}", status, truncate_body(&body)));
    }
    serde_json::from_str(&body).map_err(|e| format!("Parse error: {e}"))
}

/// Fetch the Step Plan monthly credit pool. A 401 retries once after
/// `RefreshToken`; any remaining failure is returned so the card can say why
/// the pool is missing instead of silently showing balance only.
async fn fetch_plan_quota(
    client: &Client,
    auth: &StepFunAuthManager,
) -> Result<StepFunPlanQuota, String> {
    let mut session = auth.session().await?;
    let mut rate = console_rpc(
        client,
        &auth.console_origin,
        &session,
        "QueryStepPlanRateLimit",
    )
    .await;
    if rate.as_ref().is_err_and(|error| error.contains("HTTP 401")) {
        session = auth.refresh(Some(&session.access_token)).await?;
        rate = console_rpc(
            client,
            &auth.console_origin,
            &session,
            "QueryStepPlanRateLimit",
        )
        .await;
    }
    let rate = rate?;
    if as_i64(&rate["status"]) != 1 {
        return Err(format!(
            "QueryStepPlanRateLimit status={}: {}",
            rate["status"], rate["desc"]
        ));
    }
    let pool = &rate["plan_credit_rate_limit"];
    // type 1 = subscription monthly pool; top-up packs (加油包) are separate
    // buckets we don't need — subscription_credit_left_rate covers the pool.
    let bucket = pool["credit_buckets"]
        .as_array()
        .and_then(|buckets| {
            buckets
                .iter()
                .find(|b| as_i64(&b["type"]) == 1)
                .or_else(|| buckets.first())
        })
        .ok_or_else(|| "Step Plan response has no credit bucket".to_string())?;

    let mut status = console_rpc(client, &auth.console_origin, &session, "GetStepPlanStatus").await;
    if status
        .as_ref()
        .is_err_and(|error| error.contains("HTTP 401"))
    {
        session = auth.refresh(Some(&session.access_token)).await?;
        status = console_rpc(client, &auth.console_origin, &session, "GetStepPlanStatus").await;
    }
    let status = status?;
    if as_i64(&status["status"]) != 1 || status["subscription"].is_null() {
        return Err("GetStepPlanStatus: no active subscription".to_string());
    }
    let sub = &status["subscription"];

    Ok(StepFunPlanQuota {
        plan_name: sub["name"].as_str().unwrap_or("").to_string(),
        credit_left_rate: pool["subscription_credit_left_rate"]
            .as_f64()
            .unwrap_or(0.0),
        credit_total: as_i64(&bucket["credit_total"]),
        credit_residual: as_i64(&bucket["credit_residual"]),
        next_reset_at: as_i64(&bucket["next_reset_at"]),
        expired_at: as_i64(&sub["expired_at"]),
    })
}

fn state_path_from_env() -> PathBuf {
    if let Ok(path) = std::env::var("STEPFUN_AUTH_STATE_PATH")
        && !path.trim().is_empty()
    {
        return PathBuf::from(path);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home)
        .join(".config")
        .join("token-stats")
        .join("stepfun-auth.json")
}

fn load_bootstrap_session() -> Option<ConsoleSession> {
    let token = std::env::var("STEPFUN_OASIS_TOKEN")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())?;
    let webid = std::env::var("STEPFUN_OASIS_WEBID")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())?;
    parse_bootstrap_token(&token, &webid)
}

/// `access...refresh` is the CodexBar combined form. A bare JWT is an access
/// token only — usable until `exp`, but not renewable.
fn parse_bootstrap_token(token: &str, webid: &str) -> Option<ConsoleSession> {
    let (access_token, refresh_token) = token
        .split_once("...")
        .map(|(access, refresh)| (access.to_string(), refresh.to_string()))
        .unwrap_or_else(|| (token.to_string(), String::new()));
    if access_token.is_empty() {
        return None;
    }
    Some(ConsoleSession {
        expires_at: jwt_expiry(&access_token).unwrap_or(0),
        access_token,
        refresh_token,
        webid: webid.to_string(),
    })
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct PersistedSession {
    access_token: String,
    refresh_token: String,
    webid: String,
    expires_at: i64,
}

fn load_state(path: &Path) -> Option<ConsoleSession> {
    let content = fs::read(path).ok()?;
    let persisted: PersistedSession = serde_json::from_slice(&content).ok()?;
    if persisted.access_token.is_empty() || persisted.webid.is_empty() {
        return None;
    }
    Some(ConsoleSession {
        access_token: persisted.access_token,
        refresh_token: persisted.refresh_token,
        webid: persisted.webid,
        expires_at: persisted.expires_at,
    })
}

fn persist_state(path: &Path, session: &ConsoleSession) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("create StepFun auth directory: {e}"))?;
        set_private_directory_permissions(parent)?;
    }
    let persisted = PersistedSession {
        access_token: session.access_token.clone(),
        refresh_token: session.refresh_token.clone(),
        webid: session.webid.clone(),
        expires_at: session.expires_at,
    };
    let temp_path = path.with_extension(format!("json.tmp-{}", std::process::id()));
    let content = serde_json::to_vec_pretty(&persisted)
        .map_err(|e| format!("serialize StepFun auth: {e}"))?;
    fs::write(&temp_path, content).map_err(|e| format!("write StepFun auth state: {e}"))?;
    set_private_file_permissions(&temp_path)?;
    fs::rename(&temp_path, path).map_err(|e| format!("replace StepFun auth state: {e}"))
}

fn set_private_directory_permissions(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|e| format!("protect StepFun auth directory: {e}"))?;
    }
    let _ = path;
    Ok(())
}

fn set_private_file_permissions(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .map_err(|e| format!("protect StepFun auth state: {e}"))?;
    }
    let _ = path;
    Ok(())
}

struct StateFileLock(std::fs::File);

impl StateFileLock {
    fn acquire(state_path: &Path) -> Result<Self, String> {
        if let Some(parent) = state_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("create StepFun auth directory: {e}"))?;
            set_private_directory_permissions(parent)?;
        }
        let lock_path = state_path.with_extension("lock");
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(lock_path)
            .map_err(|e| format!("open StepFun auth lock: {e}"))?;
        file.lock_exclusive()
            .map_err(|e| format!("lock StepFun auth state: {e}"))?;
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

fn truncate_body(body: &str) -> String {
    let trimmed = body.trim();
    if trimmed.chars().count() <= 180 {
        trimmed.to_string()
    } else {
        format!("{}…", trimmed.chars().take(180).collect::<String>())
    }
}

const CONSOLE_ORIGIN: &str = "https://platform.stepfun.com";

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use tempfile::tempdir;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn future_expiry() -> i64 {
        unix_now() + 3600
    }

    fn session(access: &str, refresh: &str, expires_at: i64) -> ConsoleSession {
        ConsoleSession {
            access_token: access.to_string(),
            refresh_token: refresh.to_string(),
            webid: "webid-1".to_string(),
            expires_at,
        }
    }

    fn plan_rate_body() -> serde_json::Value {
        json!({
            "status": 1,
            "plan_credit_rate_limit": {
                "subscription_credit_left_rate": 0.85,
                "credit_buckets": [{
                    "type": 1,
                    "credit_total": "1600000000",
                    "credit_residual": "1360000000",
                    "next_reset_at": "1791000000"
                }]
            }
        })
    }

    fn plan_status_body() -> serde_json::Value {
        json!({
            "status": 1,
            "subscription": {
                "name": "Plus",
                "expired_at": "1792000000"
            }
        })
    }

    #[test]
    fn splits_codexbar_combined_bootstrap_token() {
        let parsed = parse_bootstrap_token("access.jwt.sig...refresh.jwt.sig", "web-1")
            .expect("combined token");
        assert_eq!(parsed.access_token, "access.jwt.sig");
        assert_eq!(parsed.refresh_token, "refresh.jwt.sig");
        assert_eq!(parsed.webid, "web-1");
    }

    #[tokio::test]
    async fn uses_persisted_session_before_bootstrap() {
        let dir = tempdir().expect("tempdir");
        let state_path = dir.path().join("stepfun-auth.json");
        persist_state(
            &state_path,
            &session("persisted-access", "persisted-refresh", future_expiry()),
        )
        .expect("persist");

        let manager = StepFunAuthManager::with_test_config(
            Client::new(),
            "http://127.0.0.1:1",
            state_path,
            Some(session(
                "bootstrap-access",
                "bootstrap-refresh",
                future_expiry(),
            )),
        );

        assert_eq!(
            manager.session().await.expect("session").access_token,
            "persisted-access"
        );
    }

    #[tokio::test]
    async fn refreshes_expired_session_and_persists_the_rotated_pair() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(REFRESH_PATH))
            .and(header("Oasis-Token", "expired-access...initial-refresh"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "accessToken": {"raw": "rotated-access", "duration": 1800, "mode": 2},
                "refreshToken": {"raw": "rotated-refresh", "duration": 1800, "mode": 1}
            })))
            .expect(1)
            .mount(&server)
            .await;

        let dir = tempdir().expect("tempdir");
        let state_path = dir.path().join("stepfun-auth.json");
        let manager = StepFunAuthManager::with_test_config(
            Client::new(),
            server.uri(),
            state_path.clone(),
            Some(session("expired-access", "initial-refresh", 0)),
        );

        let refreshed = manager.session().await.expect("refreshed session");
        assert_eq!(refreshed.access_token, "rotated-access");
        assert_eq!(refreshed.refresh_token, "rotated-refresh");

        let persisted = load_state(&state_path).expect("persisted state");
        assert_eq!(persisted.access_token, "rotated-access");
        assert!(persisted.expires_at > unix_now() + 1200);
    }

    #[tokio::test]
    async fn retries_plan_query_once_after_unauthorized() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(REFRESH_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "accessToken": {"raw": "refreshed-access", "duration": 1800},
                "refreshToken": {"raw": "refreshed-refresh", "duration": 1800}
            })))
            .mount(&server)
            .await;

        let rate_hits = Arc::new(AtomicUsize::new(0));
        let rate_hits_for_responder = Arc::clone(&rate_hits);
        Mock::given(method("POST"))
            .and(path(format!("{CONSOLE_RPC_PATH}/QueryStepPlanRateLimit")))
            .respond_with(move |request: &wiremock::Request| {
                let token = request
                    .headers
                    .get("Oasis-Token")
                    .and_then(|value| value.to_str().ok())
                    .unwrap_or("");
                if rate_hits_for_responder.fetch_add(1, Ordering::SeqCst) == 0 {
                    assert_eq!(token, "stale-access...live-refresh");
                    ResponseTemplate::new(401).set_body_string("token is expired")
                } else {
                    assert_eq!(token, "refreshed-access...refreshed-refresh");
                    ResponseTemplate::new(200).set_body_json(plan_rate_body())
                }
            })
            .expect(2)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path(format!("{CONSOLE_RPC_PATH}/GetStepPlanStatus")))
            .respond_with(ResponseTemplate::new(200).set_body_json(plan_status_body()))
            .expect(1)
            .mount(&server)
            .await;

        let dir = tempdir().expect("tempdir");
        let auth = StepFunAuthManager::with_test_config(
            Client::new(),
            server.uri(),
            dir.path().join("stepfun-auth.json"),
            Some(session("stale-access", "live-refresh", future_expiry())),
        );

        let plan = fetch_plan_quota(&Client::new(), &auth)
            .await
            .expect("plan after refresh");
        assert_eq!(plan.plan_name, "Plus");
        assert!((plan.credit_left_rate - 0.85).abs() < f64::EPSILON);
        assert_eq!(plan.credit_total, 1_600_000_000);
        assert_eq!(plan.credit_residual, 1_360_000_000);
        assert_eq!(CONSOLE_ORIGIN, "https://platform.stepfun.com");
    }
}
