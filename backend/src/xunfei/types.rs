//! Xunfei (iFlytek) Coding Plan types.
//!
//! Data transfer objects for the dashboard's Xunfei status API response.

use serde::{Deserialize, Serialize};

/// Aggregated Xunfei subscription status for the dashboard.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XunfeiStatus {
    pub available: bool,
    pub data: Option<XunfeiStatusData>,
    pub error: Option<String>,
}

/// A single account within the multi-account Xunfei response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XunfeiAccountStatus {
    pub label: String,
    pub available: bool,
    pub data: Option<XunfeiStatusData>,
    pub error: Option<String>,
}

/// Multi-account Xunfei response (primary + optional EX account).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XunfeiMultiStatus {
    pub accounts: Vec<XunfeiAccountStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XunfeiStatusData {
    pub plan_name: String,
    pub package_id: i64,
    pub status: String,
    pub expires_at: String,
    pub created_at: String,
    pub price: i64,
    pub usage: XunfeiUsage,
    pub balance: XunfeiBalance,
    pub app_id: String,
    pub api_key_masked: String,
    pub model_list: Vec<XunfeiModelInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XunfeiUsage {
    pub package_used: i64,
    pub package_limit: i64,
    pub package_left: i64,
    pub rp5h_used: i64,
    pub rp5h_limit: i64,
    pub rpw_used: i64,
    pub rpw_limit: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XunfeiBalance {
    pub cash: i64,
    pub virtual_balance: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XunfeiModelInfo {
    pub model_id: String,
    pub name: String,
    pub context_length: String,
    pub is_default: bool,
}
