//! Grok / XAI subscription quota fetcher.
//!
//! Calls the same gRPC-Web endpoint used by the Grok usage page to retrieve
//! the paid SuperGrok weekly credit pool.  The endpoint returns protobuf
//! rather than HTML, so it works even when the page itself is protected by
//! Cloudflare.

use super::types::*;
use crate::models::TokenRecord;
use crate::pricing;
use chrono::{DateTime, SecondsFormat, Utc};
use reqwest::Client;
use serde::Deserialize;
use std::fs::File;
use std::path::PathBuf;
use tracing::{info, warn};

const XAI_API_BASE: &str = "https://api.x.ai";
const GROK_CREDITS_URL: &str = "https://grok.com/grok_api_v2.GrokBuildBilling/GetGrokCreditsConfig";
const XAI_TIMEOUT_SECS: u64 = 15;
const GROK_CREDITS_TIMEOUT_SECS: u64 = 15;
const EMPTY_GRPC_WEB_BODY: &[u8] = &[0, 0, 0, 0, 0];

/// Build a reqwest Client for api.x.ai calls.
///
/// api.x.ai is not reachable directly from this host (both IPv4 and IPv6
/// TCP connections time out). It can be reached through the local HTTP
/// proxy at 127.0.0.1:7800. We explicitly configure this proxy rather than
/// relying on env vars so the fetcher works under systemd (where env vars
/// differ from the interactive shell). Falls back to direct connection if
/// no proxy env vars are set.
fn build_xai_client() -> Client {
    // Honor explicit proxy env vars if set, otherwise configure the local proxy.
    let mut builder = reqwest::Client::builder();

    let has_proxy_env = std::env::var("http_proxy")
        .ok()
        .filter(|s| !s.is_empty())
        .is_some()
        || std::env::var("HTTP_PROXY")
            .ok()
            .filter(|s| !s.is_empty())
            .is_some()
        || std::env::var("https_proxy")
            .ok()
            .filter(|s| !s.is_empty())
            .is_some()
        || std::env::var("HTTPS_PROXY")
            .ok()
            .filter(|s| !s.is_empty())
            .is_some()
        || std::env::var("ALL_PROXY")
            .ok()
            .filter(|s| !s.is_empty())
            .is_some();

    if has_proxy_env {
        // Standard proxy env vars are set — use them (matching Client::new() behavior).
        return builder.build().unwrap_or_else(|_| Client::new());
    }

    // No proxy env vars: add the local HTTP proxy explicitly.
    // Use a local proxy URL that reqwest can handle without the socks feature.
    if let Ok(proxy) = reqwest::Proxy::https("http://127.0.0.1:7800") {
        builder = builder.proxy(proxy);
    }
    builder.build().unwrap_or_else(|_| Client::new())
}

// ─── Auth helpers ────────────────────────────────────────────────────────────

/// Resolve the xAI API key from environment or auth.json
pub fn get_api_key() -> Option<String> {
    // 1. GROK_XAI_API_KEY env var
    if let Ok(key) = std::env::var("GROK_XAI_API_KEY") {
        if !key.is_empty() {
            return Some(key);
        }
    }

    // 2. Fallback: read from ~/.grok/auth.json
    resolve_key_from_auth_json()
}

fn auth_json_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".grok/auth.json")
}

fn resolve_key_from_auth_json() -> Option<String> {
    let path = auth_json_path();
    let file = File::open(&path).ok()?;
    let data: serde_json::Value = serde_json::from_reader(file).ok()?;
    // auth.json is a map of issuer URL -> credentials
    for (_issuer, creds) in data.as_object()? {
        if let Some(key) = creds.get("key").and_then(|v| v.as_str()) {
            return Some(key.to_string());
        }
    }
    None
}

// ─── XAI API types ───────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct XaiMeResponse {
    user_id: String,
    team_id: String,
    #[serde(default)]
    zdr_status: String,
}

#[derive(Debug, Clone)]
struct GrokWeeklyCredits {
    usage_percent: f64,
    remaining_percent: f64,
    period_start: String,
    period_end: String,
    breakdown: Vec<GrokUsageBreakdown>,
}

#[derive(Debug, Clone)]
struct GrokUsageBreakdown {
    product: String,
    usage_percent: f64,
}

// ─── Fetch functions ─────────────────────────────────────────────────────────

/// Fetch Grok/XAI subscription and usage info.
pub async fn fetch_grok_quota(_client: &Client, grok_records: &[TokenRecord]) -> GrokQuotaStatus {
    let api_key = match get_api_key() {
        Some(k) => k,
        None => {
            warn!("GROK_XAI_API_KEY not set and ~/.grok/auth.json not found");
            return GrokQuotaStatus {
                available: false,
                data: None,
                error: Some("GROK_XAI_API_KEY not set".to_string()),
            };
        }
    };

    // Use IPv4-only client to avoid broken IPv6 on this host
    let client = build_xai_client();
    let me_result = fetch_me(&client, &api_key).await;

    let (user_id, team_id, zdr_status) = match me_result {
        Ok(me) => (me.user_id, me.team_id, me.zdr_status),
        Err(e) => {
            warn!("XAI /v1/me failed: {e}");
            return GrokQuotaStatus {
                available: false,
                data: None,
                error: Some(format!("API error: {e}")),
            };
        }
    };

    let weekly = match fetch_weekly_credits(&client, &api_key).await {
        Ok(quota) => quota,
        Err(e) => {
            warn!("Grok weekly SuperGrok quota failed: {e}");
            return GrokQuotaStatus {
                available: false,
                data: None,
                error: Some(format!("Weekly SuperGrok quota error: {e}")),
            };
        }
    };

    // Aggregate usage from grok-cli records for xai-official provider
    let xai_records: Vec<&TokenRecord> = grok_records
        .iter()
        .filter(|r| r.provider == "xai-official" || r.provider == "xai")
        .collect();

    let total_calls = xai_records.len() as i64;
    let total_input_tokens: i64 = xai_records.iter().map(|r| r.input_tokens).sum();
    let total_output_tokens: i64 = xai_records.iter().map(|r| r.output_tokens).sum();
    let total_cache_read_tokens: i64 = xai_records.iter().map(|r| r.cache_read_tokens).sum();
    let total_tokens: i64 = total_input_tokens + total_output_tokens + total_cache_read_tokens;

    // Keep local usage totals as diagnostics, but never use them as a proxy for
    // the subscription quota shown by grok.com.
    let pricing_cfg = pricing::get_config();
    // 配额卡显示“当前”成本，使用最新分段的汇率。
    let usd_to_cny = pricing::current_rate();
    // Super Grok pricing: $12.50/M input, $25.00/M output (per /v1/models)
    // 订阅折扣：50 元/3 月 ≈ $150/周 原始额度，divisor = 264.79
    let per_million_input = 12.50;
    let per_million_output = 25.00;
    let input_cost_usd = total_input_tokens as f64 / 1_000_000.0 * per_million_input;
    let output_cost_usd = total_output_tokens as f64 / 1_000_000.0 * per_million_output;
    let estimated_cost_cny =
        (input_cost_usd + output_cost_usd) * usd_to_cny / pricing_cfg.special.grok_divisor;

    info!(
        "Grok weekly quota fetched: user={user_id}, used={:.1}%, reset={}",
        weekly.usage_percent, weekly.period_end
    );

    GrokQuotaStatus {
        available: true,
        data: Some(GrokQuotaData {
            user_id,
            team_id,
            zdr_status,
            total_calls,
            total_input_tokens,
            total_output_tokens,
            total_cache_read_tokens,
            total_tokens,
            estimated_cost_cny,
            weekly_usage_percent: weekly.usage_percent,
            weekly_remaining_percent: weekly.remaining_percent,
            weekly_period_start: weekly.period_start,
            weekly_reset_at: Some(weekly.period_end),
            weekly_breakdown: weekly
                .breakdown
                .into_iter()
                .map(|entry| GrokQuotaBreakdown {
                    product: entry.product,
                    usage_percent: entry.usage_percent,
                })
                .collect(),
        }),
        error: None,
    }
}

async fn fetch_weekly_credits(client: &Client, api_key: &str) -> Result<GrokWeeklyCredits, String> {
    let response = client
        .post(GROK_CREDITS_URL)
        .header("Authorization", format!("Bearer {api_key}"))
        .header("Content-Type", "application/grpc-web+proto")
        .header("Accept", "*/*")
        .header("Origin", "https://grok.com")
        .header("Referer", "https://grok.com/?_s=usage")
        .header("x-grpc-web", "1")
        .header("x-user-agent", "connect-es/2.1.1")
        .header(
            "User-Agent",
            "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 Chrome/150 Safari/537.36",
        )
        .body(EMPTY_GRPC_WEB_BODY)
        .timeout(std::time::Duration::from_secs(GROK_CREDITS_TIMEOUT_SECS))
        .send()
        .await
        .map_err(|e| format!("network error: {e}"))?;

    let status = response.status();
    let body = response
        .bytes()
        .await
        .map_err(|e| format!("response read error: {e}"))?;
    if !status.is_success() {
        return Err(format!("HTTP {status}"));
    }

    parse_weekly_credits_response(&body)
}

/// Parse the gRPC-Web response returned by GetGrokCreditsConfig.
///
/// The response is a standard five-byte gRPC-Web data frame followed by a
/// protobuf message.  The outer message's field 1 contains the usage config;
/// the config stores the aggregate used percentage in field 1, period bounds
/// in fields 4/5, and product usage breakdowns in repeated field 7.
fn parse_weekly_credits_response(body: &[u8]) -> Result<GrokWeeklyCredits, String> {
    let payload = first_grpc_web_message(body)?;
    let config = first_length_delimited_field(payload, 1)?.unwrap_or(payload);

    let usage_percent = fixed32_field(config, 1)?
        .ok_or_else(|| "missing aggregate usage percentage".to_string())?;
    let period_start =
        timestamp_field(config, 4)?.ok_or_else(|| "missing quota period start".to_string())?;
    let period_end =
        timestamp_field(config, 5)?.ok_or_else(|| "missing quota period end".to_string())?;

    let usage_percent = clamp_percent(usage_percent);
    let breakdown = length_delimited_fields(config, 7)?
        .into_iter()
        .filter_map(|message| parse_breakdown(message).ok())
        .collect();

    Ok(GrokWeeklyCredits {
        usage_percent,
        remaining_percent: 100.0 - usage_percent,
        period_start,
        period_end,
        breakdown,
    })
}

fn first_grpc_web_message(body: &[u8]) -> Result<&[u8], String> {
    let mut offset = 0;
    while offset + 5 <= body.len() {
        let flags = body[offset];
        let length = u32::from_be_bytes([
            body[offset + 1],
            body[offset + 2],
            body[offset + 3],
            body[offset + 4],
        ]) as usize;
        offset += 5;
        let end = offset
            .checked_add(length)
            .ok_or_else(|| "invalid gRPC-Web frame length".to_string())?;
        if end > body.len() {
            return Err("truncated gRPC-Web frame".to_string());
        }
        let message = &body[offset..end];
        offset = end;
        // A trailer frame has the high bit set. Data frames are flag 0.
        if flags & 0x80 == 0 {
            return Ok(message);
        }
    }
    Err("missing gRPC-Web data frame".to_string())
}

fn read_varint(data: &[u8], offset: &mut usize) -> Result<u64, String> {
    let mut value = 0u64;
    for byte_index in 0..10 {
        let byte = *data
            .get(*offset)
            .ok_or_else(|| "truncated protobuf varint".to_string())?;
        *offset += 1;
        if byte_index == 9 && byte > 1 {
            return Err("protobuf varint overflow".to_string());
        }
        value |= u64::from(byte & 0x7f) << (byte_index * 7);
        if byte & 0x80 == 0 {
            return Ok(value);
        }
    }
    Err("unterminated protobuf varint".to_string())
}

fn skip_field(data: &[u8], offset: &mut usize, wire_type: u8) -> Result<(), String> {
    match wire_type {
        0 => {
            read_varint(data, offset)?;
        }
        1 => {
            *offset = offset
                .checked_add(8)
                .ok_or_else(|| "protobuf field overflow".to_string())?;
        }
        2 => {
            let length = read_varint(data, offset)? as usize;
            *offset = offset
                .checked_add(length)
                .ok_or_else(|| "protobuf field overflow".to_string())?;
        }
        5 => {
            *offset = offset
                .checked_add(4)
                .ok_or_else(|| "protobuf field overflow".to_string())?;
        }
        other => return Err(format!("unsupported protobuf wire type {other}")),
    }
    if *offset > data.len() {
        return Err("truncated protobuf field".to_string());
    }
    Ok(())
}

fn first_length_delimited_field<'a>(
    data: &'a [u8],
    wanted_field: u32,
) -> Result<Option<&'a [u8]>, String> {
    let mut offset = 0;
    while offset < data.len() {
        let tag = read_varint(data, &mut offset)?;
        let field_number = (tag >> 3) as u32;
        let wire_type = (tag & 0x07) as u8;
        if wire_type == 2 {
            let length = read_varint(data, &mut offset)? as usize;
            let end = offset
                .checked_add(length)
                .ok_or_else(|| "protobuf field overflow".to_string())?;
            if end > data.len() {
                return Err("truncated protobuf length-delimited field".to_string());
            }
            let value = &data[offset..end];
            offset = end;
            if field_number == wanted_field {
                return Ok(Some(value));
            }
        } else {
            skip_field(data, &mut offset, wire_type)?;
        }
    }
    Ok(None)
}

fn length_delimited_fields<'a>(data: &'a [u8], wanted_field: u32) -> Result<Vec<&'a [u8]>, String> {
    let mut fields = Vec::new();
    let mut offset = 0;
    while offset < data.len() {
        let tag = read_varint(data, &mut offset)?;
        let field_number = (tag >> 3) as u32;
        let wire_type = (tag & 0x07) as u8;
        if wire_type == 2 {
            let length = read_varint(data, &mut offset)? as usize;
            let end = offset
                .checked_add(length)
                .ok_or_else(|| "protobuf field overflow".to_string())?;
            if end > data.len() {
                return Err("truncated protobuf length-delimited field".to_string());
            }
            if field_number == wanted_field {
                fields.push(&data[offset..end]);
            }
            offset = end;
        } else {
            skip_field(data, &mut offset, wire_type)?;
        }
    }
    Ok(fields)
}

fn fixed32_field(data: &[u8], wanted_field: u32) -> Result<Option<f64>, String> {
    let mut offset = 0;
    while offset < data.len() {
        let tag = read_varint(data, &mut offset)?;
        let field_number = (tag >> 3) as u32;
        let wire_type = (tag & 0x07) as u8;
        if wire_type == 5 {
            let end = offset
                .checked_add(4)
                .ok_or_else(|| "protobuf field overflow".to_string())?;
            if end > data.len() {
                return Err("truncated protobuf fixed32 field".to_string());
            }
            let value = f32::from_le_bytes(data[offset..end].try_into().unwrap()) as f64;
            offset = end;
            if field_number == wanted_field {
                return Ok(Some(value));
            }
        } else {
            skip_field(data, &mut offset, wire_type)?;
        }
    }
    Ok(None)
}

fn varint_field(data: &[u8], wanted_field: u32) -> Result<Option<u64>, String> {
    let mut offset = 0;
    while offset < data.len() {
        let tag = read_varint(data, &mut offset)?;
        let field_number = (tag >> 3) as u32;
        let wire_type = (tag & 0x07) as u8;
        if wire_type == 0 {
            let value = read_varint(data, &mut offset)?;
            if field_number == wanted_field {
                return Ok(Some(value));
            }
        } else {
            skip_field(data, &mut offset, wire_type)?;
        }
    }
    Ok(None)
}

fn timestamp_field(data: &[u8], wanted_field: u32) -> Result<Option<String>, String> {
    let Some(timestamp) = first_length_delimited_field(data, wanted_field)? else {
        return Ok(None);
    };
    let seconds = varint_field(timestamp, 1)?.unwrap_or(0) as i64;
    let nanos = varint_field(timestamp, 2)?.unwrap_or(0);
    if nanos >= 1_000_000_000 {
        return Err("invalid protobuf timestamp nanos".to_string());
    }
    let Some(value) = DateTime::<Utc>::from_timestamp(seconds, nanos as u32) else {
        return Err("invalid protobuf timestamp".to_string());
    };
    Ok(Some(value.to_rfc3339_opts(SecondsFormat::Millis, true)))
}

fn parse_breakdown(data: &[u8]) -> Result<GrokUsageBreakdown, String> {
    let product_code = varint_field(data, 1)?.unwrap_or(0);
    let usage_percent = clamp_percent(fixed32_field(data, 2)?.unwrap_or(0.0));
    let product = match product_code {
        0 => "third_party",
        1 => "api",
        2 => "build",
        3 => "plugins",
        4 => "chat",
        5 => "imagine",
        6 => "voice",
        _ => return Err(format!("unknown Grok product code {product_code}")),
    };
    Ok(GrokUsageBreakdown {
        product: product.to_string(),
        usage_percent,
    })
}

fn clamp_percent(value: f64) -> f64 {
    if value.is_finite() {
        value.clamp(0.0, 100.0)
    } else {
        0.0
    }
}

async fn fetch_me(client: &Client, api_key: &str) -> Result<XaiMeResponse, String> {
    let url = format!("{}/v1/me", XAI_API_BASE);

    tracing::info!("Grok fetch_me: sending request to {url}");

    let response = match client
        .get(&url)
        .header("Authorization", format!("Bearer {}", api_key))
        .timeout(std::time::Duration::from_secs(XAI_TIMEOUT_SECS))
        .send()
        .await
    {
        Ok(resp) => resp,
        Err(e) => {
            // Log extra detail to help diagnose connectivity issues
            tracing::error!(
                "Grok fetch_me failed: {e}. is_timeout={}, is_connect={}, is_request={}, is_decode={}",
                e.is_timeout(),
                e.is_connect(),
                e.is_request(),
                e.is_decode()
            );
            return Err(format!("Network error: {e}"));
        }
    };

    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status()));
    }

    response
        .json::<XaiMeResponse>()
        .await
        .map_err(|e| format!("Parse error: {e}"))
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_api_key_from_env() {
        temp_env::with_var("GROK_XAI_API_KEY", Some("test-key-from-env"), || {
            assert_eq!(get_api_key(), Some("test-key-from-env".to_string()));
        });
    }

    #[test]
    fn test_get_api_key_from_env_empty() {
        temp_env::with_var("GROK_XAI_API_KEY", Some(""), || {
            // Falls back to auth.json — test just checks it doesn't panic
            let key = get_api_key();
            // Either None (no auth.json) or Some (if auth.json exists)
            assert!(key.is_none() || key.unwrap().len() > 10);
        });
    }

    #[test]
    fn test_get_api_key_not_set() {
        temp_env::with_var("GROK_XAI_API_KEY", None::<&str>, || {
            temp_env::with_var("HOME", Some("/nonexistent"), || {
                assert_eq!(get_api_key(), None);
            });
        });
    }

    #[test]
    fn test_resolve_key_from_auth_json_real_file() {
        // Just verify the function doesn't panic when resolving from real file
        let key = resolve_key_from_auth_json();
        if let Some(k) = key {
            assert!(!k.is_empty(), "key should be non-empty if file exists");
        }
        // May be None if ~/.grok/auth.json doesn't exist on this machine
    }

    #[test]
    fn parses_live_grok_weekly_credits_shape() {
        let payload = hex_bytes(
            "0a580d0000d04112001a00220c089da9f7d20610c09489cf022a0c089d9e9cd30610c09489cf023a070801150000d0413a020804421e0802120c089da9f7d20610c09489cf021a0c089d9e9cd30610c09489cf02580162006801",
        );
        let mut response = vec![0, 0, 0, 0, payload.len() as u8];
        response.extend(payload);

        let quota = parse_weekly_credits_response(&response).expect("valid Grok response");
        assert_eq!(quota.usage_percent, 26.0);
        assert_eq!(quota.remaining_percent, 74.0);
        assert_eq!(quota.breakdown[0].product, "api");
        assert_eq!(quota.breakdown[0].usage_percent, 26.0);
        assert_eq!(quota.breakdown[1].product, "chat");
        assert_eq!(quota.breakdown[1].usage_percent, 0.0);
    }

    fn hex_bytes(value: &str) -> Vec<u8> {
        value
            .as_bytes()
            .chunks(2)
            .map(|pair| {
                let digit = |byte: u8| match byte {
                    b'0'..=b'9' => byte - b'0',
                    b'a'..=b'f' => byte - b'a' + 10,
                    b'A'..=b'F' => byte - b'A' + 10,
                    _ => panic!("invalid hex"),
                };
                digit(pair[0]) * 16 + digit(pair[1])
            })
            .collect()
    }
}
