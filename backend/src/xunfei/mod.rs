//! Xunfei (iFlytek) Coding Plan subscription fetcher.
//!
//! Fetches the user's coding plan status, usage, and balance from the
//! Xunfei MaaS platform API using SSO session authentication.
//!
//! Auth: `XUNFEI_SSO_SESSION_ID` env var or `~/.config/token-stats/xunfei.json`

pub mod types;
pub use types::*;

// ─── Credential resolution ───────────────────────────────────────────────────

/// Resolve Xunfei SSO session ID from:
/// 1. `XUNFEI_SSO_SESSION_ID` environment variable
/// 2. `~/.config/token-stats/xunfei.json` with `ssoSessionId` field
pub fn resolve_xunfei_sso_session() -> Option<String> {
    // 1. Env var
    if let Ok(session) = std::env::var("XUNFEI_SSO_SESSION_ID") {
        let trimmed = session.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }

    // 2. Config file
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let config_path = std::path::PathBuf::from(&home)
        .join(".config")
        .join("token-stats")
        .join("xunfei.json");

    if let Ok(content) = std::fs::read_to_string(&config_path) {
        if let Ok(config) = serde_json::from_str::<serde_json::Value>(&content) {
            if let Some(session) = config.get("ssoSessionId").and_then(|v| v.as_str()) {
                let trimmed = session.trim();
                if !trimmed.is_empty() {
                    return Some(trimmed.to_string());
                }
            }
        }
    }

    None
}

// ─── Fetcher ─────────────────────────────────────────────────────────────────

/// Fetcher for the Xunfei MaaS platform coding plan API.
pub struct XunfeiFetcher {
    pub client: reqwest::Client,
    pub base_url: String,
}

impl XunfeiFetcher {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url: "https://maas.xfyun.cn".to_string(),
        }
    }

    /// Fetch the full Xunfei status (plan + usage + balance).
    pub async fn fetch_status(&self) -> XunfeiStatus {
        let sso_session = match resolve_xunfei_sso_session() {
            Some(s) => s,
            None => {
                return XunfeiStatus {
                    available: false,
                    data: None,
                    error: Some(
                        "Xunfei SSO session not found. Set XUNFEI_SSO_SESSION_ID \
                         or create ~/.config/token-stats/xunfei.json with ssoSessionId"
                            .to_string(),
                    ),
                };
            }
        };

        let plan_url = format!("{}/api/v1/gpt-finetune/coding-plan/list", self.base_url);
        let balance_url = format!("{}/api/v1/gpt-finetune/user/balance", self.base_url);

        let plan_future = self
            .client
            .get(&plan_url)
            .header("Cookie", format!("ssoSessionId={}", sso_session))
            .send();

        let balance_future = self
            .client
            .get(&balance_url)
            .header("Cookie", format!("ssoSessionId={}", sso_session))
            .send();

        let (plan_resp, balance_resp) = tokio::join!(plan_future, balance_future);

        // Parse plan list
        let plan_payload = match plan_resp {
            Ok(r) => {
                if !r.status().is_success() {
                    return XunfeiStatus {
                        available: false,
                        data: None,
                        error: Some(format!("Plan API returned HTTP {}", r.status())),
                    };
                }
                match r.json::<serde_json::Value>().await {
                    Ok(p) => p,
                    Err(e) => {
                        return XunfeiStatus {
                            available: false,
                            data: None,
                            error: Some(format!("Failed to parse plan response: {}", e)),
                        };
                    }
                }
            }
            Err(e) => {
                return XunfeiStatus {
                    available: false,
                    data: None,
                    error: Some(format!("Plan API request failed: {}", e)),
                };
            }
        };

        // Parse balance
        let balance_data = match balance_resp {
            Ok(r) => {
                if r.status().is_success() {
                    r.json::<serde_json::Value>().await.ok()
                } else {
                    None
                }
            }
            Err(_) => None,
        };

        parse_xunfei_status(&plan_payload, balance_data.as_ref())
    }
}

impl Default for XunfeiFetcher {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Response parsing ────────────────────────────────────────────────────────

fn parse_xunfei_status(
    plan_payload: &serde_json::Value,
    balance_payload: Option<&serde_json::Value>,
) -> XunfeiStatus {
    let data = match plan_payload.get("data") {
        Some(d) => d,
        None => {
            let code = plan_payload
                .get("code")
                .and_then(|c| c.as_i64())
                .unwrap_or(-1);
            let msg = plan_payload
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("Unknown error");
            return XunfeiStatus {
                available: false,
                data: None,
                error: Some(format!("API error (code {}): {}", code, msg)),
            };
        }
    };

    let rows = match data.get("rows").and_then(|r| r.as_array()) {
        Some(r) => r,
        None => {
            return XunfeiStatus {
                available: false,
                data: None,
                error: Some("No active coding plans found".to_string()),
            };
        }
    };

    if rows.is_empty() {
        return XunfeiStatus {
            available: true,
            data: None,
            error: Some("No active coding plans".to_string()),
        };
    }

    let plan = &rows[0];
    let status_data = extract_plan_status(plan, balance_payload);
    XunfeiStatus {
        available: true,
        data: Some(status_data),
        error: None,
    }
}

fn extract_plan_status(
    plan: &serde_json::Value,
    balance_payload: Option<&serde_json::Value>,
) -> XunfeiStatusData {
    let plan_name = plan
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("未知")
        .to_string();
    let package_id = plan.get("packageId").and_then(|v| v.as_i64()).unwrap_or(0);
    let status = match plan.get("status").and_then(|v| v.as_i64()).unwrap_or(0) {
        1 => "active".to_string(),
        0 => "inactive".to_string(),
        _ => "unknown".to_string(),
    };
    let expires_at = plan
        .get("expiresAt")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let created_at = plan
        .get("createTime")
        .or_else(|| plan.get("validFrom"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let price = plan.get("price").and_then(|v| v.as_i64()).unwrap_or(0);
    let app_id = plan
        .get("appId")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let api_key = plan
        .get("codingPlanAppCredentialDTO")
        .and_then(|v| v.get("apiKey"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let api_key_masked = mask_api_key(api_key);

    let usage = extract_usage(plan);
    let balance = extract_balance(balance_payload);
    let model_list = extract_model_list(plan);

    XunfeiStatusData {
        plan_name,
        package_id,
        status,
        expires_at,
        created_at,
        price,
        usage,
        balance,
        app_id,
        api_key_masked,
        model_list,
    }
}

fn mask_api_key(key: &str) -> String {
    if key.len() > 8 {
        format!("{}...", &key[..8])
    } else if key.is_empty() {
        "N/A".to_string()
    } else {
        key.to_string()
    }
}

fn extract_usage(plan: &serde_json::Value) -> XunfeiUsage {
    let usage_dto = plan.get("codingPlanUsageDTO");
    XunfeiUsage {
        package_used: field_i64(usage_dto, "packageUsage"),
        package_limit: field_i64(usage_dto, "packageLimit"),
        package_left: field_i64(usage_dto, "packageLeft"),
        rp5h_used: field_i64(usage_dto, "rp5hUsage"),
        rp5h_limit: field_i64(usage_dto, "rp5hLimit"),
        rpw_used: field_i64(usage_dto, "rpwUsage"),
        rpw_limit: field_i64(usage_dto, "rpwLimit"),
    }
}

fn extract_balance(balance_payload: Option<&serde_json::Value>) -> XunfeiBalance {
    match balance_payload {
        Some(b) => {
            let bdata = b.get("data");
            XunfeiBalance {
                cash: bdata
                    .and_then(|v| v.get("balance"))
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0),
                virtual_balance: bdata
                    .and_then(|v| v.get("virtualBalance"))
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0),
            }
        }
        None => XunfeiBalance {
            cash: 0,
            virtual_balance: 0,
        },
    }
}

fn extract_model_list(plan: &serde_json::Value) -> Vec<XunfeiModelInfo> {
    plan.get("modelInfo")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .map(|m| XunfeiModelInfo {
                    model_id: m
                        .get("modelId")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    name: m
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    context_length: m
                        .get("contextLength")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    is_default: m.get("default").and_then(|v| v.as_bool()).unwrap_or(false),
                })
                .collect()
        })
        .unwrap_or_default()
}

fn field_i64(val: Option<&serde_json::Value>, key: &str) -> i64 {
    val.and_then(|v| v.get(key))
        .and_then(|v| v.as_i64())
        .unwrap_or(0)
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_xunfei_status_success() {
        let plan_json = serde_json::json!({
            "code": 0,
            "data": {
                "page": 1,
                "rows": [{
                    "appId": "mc65e071",
                    "codingPlanAppCredentialDTO": {
                        "apiKey": "a96f1790a7656ed5f0fa29be01c2fc08:SECRET"
                    },
                    "codingPlanUsageDTO": {
                        "packageUsage": 6256, "packageLimit": 18000, "packageLeft": 11744,
                        "rp5hUsage": 197, "rp5hLimit": 1200,
                        "rpwUsage": 6256, "rpwLimit": 9000
                    },
                    "createTime": "2026-05-14 11:50:44",
                    "expiresAt": "2026-06-14 11:52:41",
                    "modelInfo": [
                        {"modelId": "xopglm5", "name": "GLM-5", "contextLength": "200k", "default": true}
                    ],
                    "name": "专业版", "packageId": 9198006, "price": 3900, "status": 1,
                    "validFrom": "2026-05-14 11:52:41"
                }]
            }
        });

        let balance_json = serde_json::json!({
            "code": 0,
            "data": {"balance": 0, "virtualBalance": 24190}
        });

        let status = parse_xunfei_status(&plan_json, Some(&balance_json));
        assert!(status.available);
        let data = status.data.unwrap();
        assert_eq!(data.plan_name, "专业版");
        assert_eq!(data.status, "active");
        assert_eq!(data.package_id, 9198006);
        assert_eq!(data.price, 3900);
        assert_eq!(data.usage.package_used, 6256);
        assert_eq!(data.usage.rp5h_limit, 1200);
        assert_eq!(data.balance.virtual_balance, 24190);
        assert!(data.api_key_masked.starts_with("a96f1790"));
    }

    #[test]
    fn test_parse_xunfei_status_no_rows() {
        let plan_json = serde_json::json!({
            "code": 0,
            "data": {"page": 1, "rows": [], "size": 10, "total": 0}
        });
        let status = parse_xunfei_status(&plan_json, None);
        assert!(status.available);
        assert!(status.data.is_none());
    }

    #[test]
    fn test_parse_xunfei_status_api_error() {
        let plan_json = serde_json::json!({"code": 4001, "message": "用户未登录"});
        let status = parse_xunfei_status(&plan_json, None);
        assert!(!status.available);
        assert!(status.error.unwrap().contains("用户未登录"));
    }

    #[test]
    fn test_resolve_xunfei_sso_session_from_env() {
        temp_env::with_var("XUNFEI_SSO_SESSION_ID", Some("test-session-123"), || {
            assert_eq!(
                resolve_xunfei_sso_session(),
                Some("test-session-123".to_string())
            );
        });
    }

    #[test]
    fn test_resolve_xunfei_sso_session_empty_env() {
        temp_env::with_var("XUNFEI_SSO_SESSION_ID", Some(""), || {
            assert_eq!(resolve_xunfei_sso_session(), None);
        });
    }
}
