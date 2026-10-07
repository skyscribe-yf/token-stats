//! Loopback Command Code proxy for DimAgent.
//!
//! Exposes an OpenAI-compatible `POST /v1/chat/completions` + `GET /v1/models`
//! on 127.0.0.1:${CC_PROXY_PORT:-8787}. Incoming OpenAI-format requests are
//! converted to Command Code's private `/alpha/generate` protocol (the same
//! shape the pi-commandcode-provider uses), streamed back as OpenAI SSE, and
//! per-request usage is appended to `~/.token-stats/cc-proxy-usage.jsonl`
//! (`CC_PROXY_USAGE_LOG_PATH` overrides).
//!
//! Auth: reads the Command Code API key from `~/.commandcode/auth.json`
//! (or `COMMANDCODE_API_KEY` env). Models are fetched live from
//! `https://api.commandcode.ai/provider/v1/models` (`COMMANDCODE_MODELS_URL`
//! overrides), so new models appear without a rebuild.
//!
//! Run standalone with `--cc-proxy-only` (systemd:
//! `token-stats-cc-proxy.service`), then in DimAgent:
//!   dim provider add cc-proxy --api-key x --base-url http://127.0.0.1:8787

use axum::{
    Router,
    body::{Body, Bytes, to_bytes},
    extract::State,
    http::{Request, Response, StatusCode, header},
    response::{IntoResponse, Json},
    routing::{get, post},
};
use chrono::{DateTime, SecondsFormat, Utc};
use compact_str::CompactString;
use futures_util::StreamExt;
use serde_json::{Value, json};
use std::sync::OnceLock;
use std::{io::Write, net::SocketAddr, path::PathBuf};
use tokio::sync::mpsc;

const CC_SOURCE: &str = "cc-proxy";
const DEFAULT_API_BASE: &str = "https://api.commandcode.ai";
const DEFAULT_MODELS_URL: &str = "https://api.commandcode.ai/provider/v1/models";
const CC_CLI_VERSION: &str = "0.27.2";
const DEFAULT_MAX_TOKENS: u64 = 64_000;

#[derive(Clone)]
pub struct CcProxyConfig {
    api_base: String,
    models_url: String,
    usage_log_path: PathBuf,
}

impl CcProxyConfig {
    fn from_env() -> Self {
        let api_base =
            std::env::var("COMMANDCODE_API_BASE").unwrap_or_else(|_| DEFAULT_API_BASE.to_string());
        let models_url = std::env::var("COMMANDCODE_MODELS_URL")
            .unwrap_or_else(|_| DEFAULT_MODELS_URL.to_string());
        let usage_log_path = crate::sources::cc_proxy_usage_log_path();
        Self {
            api_base: api_base.trim_end_matches('/').to_string(),
            models_url,
            usage_log_path,
        }
    }
}

// ---------------------------------------------------------------------------
// Auth
// ---------------------------------------------------------------------------

fn read_api_key() -> Option<String> {
    if let Ok(key) = std::env::var("COMMANDCODE_API_KEY") {
        let key = key.trim().to_string();
        if !key.is_empty() {
            return Some(key);
        }
    }
    let home = std::env::var("HOME").ok()?;
    for path in [
        PathBuf::from(&home).join(".commandcode").join("auth.json"),
        PathBuf::from(&home)
            .join(".pi")
            .join("agent")
            .join("auth.json"),
    ] {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(parsed) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        if let Some(key) = parsed
            .get("apiKey")
            .and_then(|v| v.as_str())
            .filter(|v| !v.is_empty())
        {
            return Some(key.to_string());
        }
        if let Some(key) = parsed
            .get("commandcode")
            .and_then(|v| v.as_str())
            .filter(|v| !v.is_empty())
        {
            return Some(key.to_string());
        }
        // pi stores OAuth credentials as {"commandcode": {"type":"oauth","access":"..."}}.
        for key in ["commandcode", "command-code"] {
            if let Some(cred) = parsed.get(key) {
                let access = cred.get("access").and_then(|v| v.as_str()).unwrap_or("");
                if !access.is_empty() {
                    return Some(access.to_string());
                }
                let k = cred.get("key").and_then(|v| v.as_str()).unwrap_or("");
                if !k.is_empty() {
                    return Some(k.to_string());
                }
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// OpenAI → Command Code request conversion
// ---------------------------------------------------------------------------

fn to_json_schema(schema: &Value) -> Value {
    let Some(obj) = schema.as_object() else {
        return json!({});
    };
    if let Some(enum_values) = obj.get("enum").and_then(|v| v.as_array()) {
        let ty = enum_values
            .first()
            .map(v_type)
            .unwrap_or_else(|| "string".to_string());
        return json!({ "type": ty, "enum": enum_values });
    }
    let kind = obj
        .get("kind")
        .and_then(|v| v.as_str())
        .or_else(|| obj.get("type").and_then(|v| v.as_str()))
        .unwrap_or("");
    match kind {
        "string" | "String" => json!({ "type": "string" }),
        "number" | "Number" => json!({ "type": "number" }),
        "boolean" | "Boolean" => json!({ "type": "boolean" }),
        "object" | "Object" => {
            let mut out = json!({ "type": "object" });
            if let Some(props) = obj.get("properties").and_then(|v| v.as_object()) {
                let mut converted = serde_json::Map::new();
                for (k, v) in props {
                    converted.insert(k.clone(), to_json_schema(v));
                }
                out["properties"] = Value::Object(converted);
            }
            let required: Vec<String> = obj
                .get("required")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            if !required.is_empty() {
                out["required"] = Value::Array(required.into_iter().map(Value::String).collect());
            }
            out
        }
        "array" | "Array" => json!({
            "type": "array",
            "items": to_json_schema(obj.get("items").or_else(|| obj.get("element")).unwrap_or(&json!({})))
        }),
        "union" | "Union" => {
            let variants = obj
                .get("variants")
                .or_else(|| obj.get("anyOf"))
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            for variant in &variants {
                let converted = to_json_schema(variant);
                if !converted.as_object().map(|o| o.is_empty()).unwrap_or(true) {
                    return converted;
                }
            }
            json!({})
        }
        "optional" | "Optional" => to_json_schema(
            obj.get("wrapped")
                .or_else(|| obj.get("inner"))
                .unwrap_or(&json!({})),
        ),
        _ => json!({}),
    }
}

fn v_type(value: &Value) -> String {
    match value {
        Value::String(_) => "string".to_string(),
        Value::Number(_) => "number".to_string(),
        Value::Bool(_) => "boolean".to_string(),
        _ => "string".to_string(),
    }
}

fn tools_to_cc(tools: &Value) -> Value {
    let Some(arr) = tools.as_array() else {
        return json!([]);
    };
    let mut out = Vec::new();
    for tool in arr {
        let Some(obj) = tool.as_object() else {
            continue;
        };
        let function = obj.get("function").and_then(|v| v.as_object());
        let name = function
            .and_then(|f| f.get("name").and_then(|v| v.as_str()))
            .or_else(|| obj.get("name").and_then(|v| v.as_str()))
            .unwrap_or("");
        if name.is_empty() {
            continue;
        }
        let description = function
            .and_then(|f| f.get("description").and_then(|v| v.as_str()))
            .or_else(|| obj.get("description").and_then(|v| v.as_str()))
            .unwrap_or("");
        let parameters = function
            .and_then(|f| f.get("parameters"))
            .or_else(|| obj.get("parameters"))
            .cloned()
            .unwrap_or_else(|| json!({}));
        out.push(json!({
            "type": "function",
            "name": name,
            "description": description,
            "input_schema": to_json_schema(&parameters),
        }));
    }
    Value::Array(out)
}

/// Collect tool-call ids that have a matching tool result (pi's
/// `completeToolCallIds` logic): only forward completed tool calls.
fn complete_tool_call_ids(messages: &[Value]) -> std::collections::HashSet<String> {
    let mut call_ids = std::collections::HashSet::new();
    let mut result_ids = std::collections::HashSet::new();
    for message in messages {
        let Some(role) = message.get("role").and_then(|v| v.as_str()) else {
            continue;
        };
        if role == "assistant" {
            if let Some(content) = message.get("content").and_then(|v| v.as_array()) {
                for part in content {
                    if part.get("type").and_then(|v| v.as_str()) == Some("tool_calls")
                        && let Some(calls) = part.get("tool_calls").and_then(|v| v.as_array())
                    {
                        for call in calls {
                            if let Some(id) = call.get("id").and_then(|v| v.as_str()) {
                                call_ids.insert(id.to_string());
                            }
                        }
                    }
                }
            }
            // OpenAI assistant message may carry tool_calls at top level.
            if let Some(calls) = message.get("tool_calls").and_then(|v| v.as_array()) {
                for call in calls {
                    if let Some(id) = call.get("id").and_then(|v| v.as_str()) {
                        call_ids.insert(id.to_string());
                    }
                }
            }
        } else if role == "tool"
            && let Some(id) = message.get("tool_call_id").and_then(|v| v.as_str())
        {
            result_ids.insert(id.to_string());
        }
    }
    call_ids
        .into_iter()
        .filter(|id| result_ids.contains(id))
        .collect()
}

fn messages_to_cc(messages: &Value) -> (String, Value) {
    let Some(arr) = messages.as_array() else {
        return (String::new(), json!([]));
    };
    let paired = complete_tool_call_ids(arr);
    let mut system_parts: Vec<String> = Vec::new();
    let mut out: Vec<Value> = Vec::new();

    for message in arr {
        let Some(role) = message.get("role").and_then(|v| v.as_str()) else {
            continue;
        };
        match role {
            "system" => {
                if let Some(text) = message.get("content").and_then(|v| v.as_str()) {
                    system_parts.push(text.to_string());
                }
            }
            "user" => {
                let content = message.get("content").cloned().unwrap_or_else(|| json!(""));
                out.push(json!({ "role": "user", "content": content }));
            }
            "assistant" => {
                let mut parts: Vec<Value> = Vec::new();
                if let Some(content) = message.get("content").and_then(|v| v.as_array()) {
                    for part in content {
                        match part.get("type").and_then(|v| v.as_str()) {
                            Some("text") => {
                                if let Some(text) = part.get("text").and_then(|v| v.as_str()) {
                                    parts.push(json!({ "type": "text", "text": text }));
                                }
                            }
                            Some("tool_calls") => {
                                if let Some(calls) =
                                    part.get("tool_calls").and_then(|v| v.as_array())
                                {
                                    for call in calls {
                                        let id =
                                            call.get("id").and_then(|v| v.as_str()).unwrap_or("");
                                        if !paired.contains(id) {
                                            continue;
                                        }
                                        let name = call
                                            .get("function")
                                            .and_then(|f| f.get("name").and_then(|v| v.as_str()))
                                            .unwrap_or("");
                                        let args = call
                                            .get("function")
                                            .and_then(|f| f.get("arguments"))
                                            .cloned()
                                            .unwrap_or_else(|| json!({}));
                                        let args = parse_arguments(&args);
                                        parts.push(json!({
                                            "type": "tool-call",
                                            "toolCallId": id,
                                            "toolName": name,
                                            "input": args,
                                        }));
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
                // Top-level tool_calls (OpenAI style).
                if let Some(calls) = message.get("tool_calls").and_then(|v| v.as_array()) {
                    for call in calls {
                        let id = call.get("id").and_then(|v| v.as_str()).unwrap_or("");
                        if !paired.contains(id) {
                            continue;
                        }
                        let name = call
                            .get("function")
                            .and_then(|f| f.get("name").and_then(|v| v.as_str()))
                            .unwrap_or("");
                        let args = call
                            .get("function")
                            .and_then(|f| f.get("arguments"))
                            .cloned()
                            .unwrap_or_else(|| json!({}));
                        parts.push(json!({
                            "type": "tool-call",
                            "toolCallId": id,
                            "toolName": name,
                            "input": parse_arguments(&args),
                        }));
                    }
                }
                if !parts.is_empty() {
                    out.push(json!({ "role": "assistant", "content": parts }));
                }
            }
            "tool" => {
                let id = message
                    .get("tool_call_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if !paired.contains(id) {
                    continue;
                }
                let name = message.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let content = message.get("content").cloned().unwrap_or_else(|| json!(""));
                let output = if message
                    .get("is_error")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
                {
                    json!({ "type": "error-text", "value": content })
                } else {
                    json!({ "type": "text", "value": content })
                };
                out.push(json!({
                    "role": "tool",
                    "content": [{
                        "type": "tool-result",
                        "toolCallId": id,
                        "toolName": name,
                        "output": output,
                    }],
                }));
            }
            _ => {}
        }
    }

    (system_parts.join("\n\n"), Value::Array(out))
}

fn parse_arguments(value: &Value) -> Value {
    if value.is_object() {
        return Value::Object(value.as_object().unwrap().clone());
    }
    if let Some(text) = value.as_str()
        && let Ok(parsed) = serde_json::from_str::<Value>(text)
    {
        return parsed;
    }
    json!({})
}

fn build_cc_body(openai: &Value) -> Result<Value, String> {
    let model = openai
        .get("model")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "missing model".to_string())?;
    // DimAgent sends provider-prefixed ids like "cc-proxy/deepseek-v4-flash";
    // Command Code expects "vendor/model" (e.g. "deepseek/deepseek-v4-flash")
    // or bare ids for its own models. Strip the proxy's own prefix.
    let model = model.strip_prefix("cc-proxy/").unwrap_or(model).to_string();
    let (system, messages) = messages_to_cc(openai.get("messages").unwrap_or(&json!([])));
    let tools = tools_to_cc(openai.get("tools").unwrap_or(&json!([])));
    let max_tokens = openai
        .get("max_tokens")
        .or_else(|| openai.get("max_completion_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(DEFAULT_MAX_TOKENS)
        .min(DEFAULT_MAX_TOKENS);
    let temperature = openai
        .get("temperature")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.3);

    let mut params = json!({
        "model": model,
        "messages": messages,
        "tools": tools,
        "system": system,
        "max_tokens": max_tokens,
        "temperature": temperature,
        "stream": true,
    });
    // CC's /alpha/generate expects tool_choice as an object (e.g.
    // {"type":"function","name":"..."}); OpenAI-style string values like
    // "auto"/"none"/"required" are rejected. Drop strings (default is auto),
    // forward objects only.
    if let Some(tool_choice) = openai.get("tool_choice").filter(|v| v.is_object()) {
        params["tool_choice"] = tool_choice.clone();
    }

    let working_dir = std::env::current_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| "/tmp".to_string());
    let thread_id = uuid_v4();

    Ok(json!({
        "config": {
            "workingDir": working_dir,
            "date": Utc::now().format("%Y-%m-%d").to_string(),
            "environment": format!("{}-{}, Rust proxy", std::env::consts::OS, std::env::consts::ARCH),
            "structure": [],
            "isGitRepo": false,
            "currentBranch": "",
            "mainBranch": "",
            "gitStatus": "",
            "recentCommits": [],
        },
        "memory": null,
        "taste": null,
        "skills": null,
        "params": params,
        "threadId": thread_id,
    }))
}

fn uuid_v4() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let now = Utc::now().timestamp_nanos_opt().unwrap_or(0) as u64;
    let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
    // Mix time and counter into 122 bits.
    let x = now ^ counter.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    let y = now.rotate_left(32) ^ counter.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    let a = (x >> 32) as u32;
    let b = ((x >> 16) & 0xFFFF) as u16;
    let c = ((x & 0x0FFF) | 0x4000) as u16; // version 4
    let d = (((y >> 48) & 0x3FFF) | 0x8000) as u16; // variant
    let e = y & 0xFFFF_FFFF_FFFF;
    format!("{:08x}-{:04x}-{:04x}-{:04x}-{:012x}", a, b, c, d, e)
}

// ---------------------------------------------------------------------------
// Command Code SSE → OpenAI conversion
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Accumulated {
    text: String,
    reasoning: String,
    tool_calls: Vec<Value>,
    usage: Option<Value>,
    finish_reason: Option<String>,
    error: Option<String>,
}

fn cc_finish_reason_to_openai(reason: &str) -> String {
    match reason {
        "tool-calls" => "tool_calls".to_string(),
        "length" | "max_tokens" | "max-tokens" | "max_output_tokens" => "length".to_string(),
        other => other.to_string(),
    }
}

/// Parse one CC event line and fold it into the accumulator, emitting OpenAI
/// SSE frames into `out`.
fn handle_cc_event(
    line: &str,
    acc: &mut Accumulated,
    out: &mut Vec<String>,
    model: &str,
    id: &str,
) {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with(':') || trimmed.starts_with("event:") {
        return;
    }
    let data = trimmed.strip_prefix("data:").unwrap_or(trimmed).trim();
    if data.is_empty() || data == "[DONE]" {
        return;
    }
    let Ok(event) = serde_json::from_str::<Value>(data) else {
        return;
    };
    let Some(etype) = event.get("type").and_then(|v| v.as_str()) else {
        return;
    };
    match etype {
        "text-delta" => {
            if let Some(delta) = event.get("text").and_then(|v| v.as_str()) {
                acc.text.push_str(delta);
                out.push(sse_frame(&json!({
                    "id": id,
                    "object": "chat.completion.chunk",
                    "created": Utc::now().timestamp(),
                    "model": model,
                    "choices": [{
                        "index": 0,
                        "delta": { "content": delta },
                        "finish_reason": null,
                    }],
                })));
            }
        }
        "reasoning-delta" => {
            if let Some(delta) = event.get("text").and_then(|v| v.as_str()) {
                acc.reasoning.push_str(delta);
                out.push(sse_frame(&json!({
                    "id": id,
                    "object": "chat.completion.chunk",
                    "created": Utc::now().timestamp(),
                    "model": model,
                    "choices": [{
                        "index": 0,
                        "delta": { "reasoning_content": delta },
                        "finish_reason": null,
                    }],
                })));
            }
        }
        "tool-call" => {
            let tool_call_id = event
                .get("toolCallId")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let name = event.get("toolName").and_then(|v| v.as_str()).unwrap_or("");
            let input = event.get("input").cloned().unwrap_or_else(|| json!({}));
            let arguments = if input.is_string() {
                input.as_str().unwrap_or("").to_string()
            } else {
                input.to_string()
            };
            let index = acc.tool_calls.len();
            acc.tool_calls.push(json!({
                "id": tool_call_id,
                "type": "function",
                "function": { "name": name, "arguments": arguments },
            }));
            out.push(sse_frame(&json!({
                "id": id,
                "object": "chat.completion.chunk",
                "created": Utc::now().timestamp(),
                "model": model,
                "choices": [{
                    "index": 0,
                    "delta": {
                        "tool_calls": [{
                            "index": index,
                            "id": tool_call_id,
                            "type": "function",
                            "function": { "name": name, "arguments": arguments },
                        }],
                    },
                    "finish_reason": null,
                }],
            })));
        }
        "finish" => {
            let reason = event
                .get("finishReason")
                .and_then(|v| v.as_str())
                .unwrap_or("stop");
            acc.finish_reason = Some(cc_finish_reason_to_openai(reason));
            acc.usage = event.get("totalUsage").cloned();
            let mut chunk = json!({
                "id": id,
                "object": "chat.completion.chunk",
                "created": Utc::now().timestamp(),
                "model": model,
                "choices": [{
                    "index": 0,
                    "delta": {},
                    "finish_reason": acc.finish_reason.clone().unwrap_or_else(|| "stop".to_string()),
                }],
            });
            if let Some(usage) = &acc.usage {
                chunk["usage"] = usage_to_openai(usage);
            }
            out.push(sse_frame(&chunk));
        }
        "error" => {
            let message = event
                .get("error")
                .and_then(|v| v.get("message"))
                .and_then(|v| v.as_str())
                .or_else(|| event.get("error").and_then(|v| v.as_str()))
                .unwrap_or("Command Code stream error")
                .to_string();
            acc.error = Some(message);
        }
        _ => {}
    }
}

fn sse_frame(value: &Value) -> String {
    format!("data: {}\n\n", value)
}

fn usage_to_openai(usage: &Value) -> Value {
    let input_tokens = usage
        .get("inputTokens")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let output_tokens = usage
        .get("outputTokens")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let cache_read = usage
        .get("inputTokenDetails")
        .and_then(|d| d.get("cacheReadTokens"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    json!({
        "prompt_tokens": input_tokens,
        "completion_tokens": output_tokens,
        "total_tokens": input_tokens + output_tokens,
        "prompt_tokens_details": { "cached_tokens": cache_read },
    })
}

// ---------------------------------------------------------------------------
// Usage recording
// ---------------------------------------------------------------------------

fn append_usage_record(path: &PathBuf, record: &crate::models::TokenRecord) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    serde_json::to_writer(&mut file, record)?;
    writeln!(file)
}

fn record_usage(
    usage: &Value,
    model: &str,
    recorded_at: DateTime<Utc>,
) -> Option<crate::models::TokenRecord> {
    let input_tokens = usage
        .get("inputTokens")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let output_tokens = usage
        .get("outputTokens")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let cache_read = usage
        .get("inputTokenDetails")
        .and_then(|d| d.get("cacheReadTokens"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let cache_write = usage
        .get("inputTokenDetails")
        .and_then(|d| d.get("cacheWriteTokens"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    if input_tokens == 0 && output_tokens == 0 && cache_read == 0 && cache_write == 0 {
        return None;
    }
    // OpenAI convention: inputTokens includes cacheReadTokens. Store uncached
    // input only (same normalization as the commandcode source).
    let uncached_input = (input_tokens - cache_read - cache_write).max(0);
    // CC model ids look like "deepseek/deepseek-v4-flash"; strip the vendor
    // prefix so pricing's cc: mapping and the dashboard match the
    // commandcode source.
    let model = model
        .split_once('/')
        .map(|(_, rest)| rest)
        .unwrap_or(model)
        .to_string();
    // Apply the same model-name normalization as the native commandcode source
    // (e.g. claude-opus-4.7 → claude-opus-4-7) so records merge cleanly in the
    // dashboard and pricing.rs's resolve_commandcode_price sees consistent keys.
    let model = crate::sources::normalize_model_name(&model);

    Some(crate::models::TokenRecord {
        date: compact_str::format_compact!("{}", recorded_at.format("%Y-%m-%d")),
        parsed_time: OnceLock::new(),
        time: recorded_at.to_rfc3339_opts(SecondsFormat::Millis, true),
        api_key_prefix: CompactString::default(),
        provider: "commandcode".into(),
        original_provider: None,
        model: model.into(),
        source: CC_SOURCE.into(),
        input_tokens: uncached_input,
        output_tokens,
        cache_read_tokens: cache_read,
        cache_write_tokens: cache_write,
        total_tokens: uncached_input + output_tokens + cache_read + cache_write,
        cost: 0.0,
        ttft_ms: None,
        tps: None,
    })
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn handle_models(State(config): State<CcProxyConfig>) -> Response<Body> {
    let Some(api_key) = read_api_key() else {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": { "message": "No Command Code API key. Configure ~/.commandcode/auth.json or COMMANDCODE_API_KEY." } })),
        )
            .into_response();
    };
    let client = match reqwest::Client::builder().build() {
        Ok(client) => client,
        Err(error) => {
            tracing::warn!("Could not create CC proxy HTTP client: {error}");
            return StatusCode::BAD_GATEWAY.into_response();
        }
    };
    match client
        .get(&config.models_url)
        .header("Authorization", format!("Bearer {api_key}"))
        .header("accept", "application/json")
        .send()
        .await
    {
        Ok(response) => {
            let status = response.status();
            let body = match response.bytes().await {
                Ok(body) => body,
                Err(error) => {
                    tracing::warn!("Could not read CC models response: {error}");
                    return StatusCode::BAD_GATEWAY.into_response();
                }
            };
            let mut builder = Response::builder().status(status);
            builder = builder.header(header::CONTENT_TYPE, "application/json");
            builder.body(Body::from(body)).unwrap_or_else(|error| {
                tracing::warn!("Could not build CC models response: {error}");
                StatusCode::BAD_GATEWAY.into_response()
            })
        }
        Err(error) => {
            tracing::warn!("CC models request failed: {error}");
            StatusCode::BAD_GATEWAY.into_response()
        }
    }
}

async fn handle_chat(
    axum::extract::State(config): axum::extract::State<CcProxyConfig>,
    request: Request<Body>,
) -> Response<Body> {
    let body = match to_bytes(request.into_body(), usize::MAX).await {
        Ok(body) => body,
        Err(error) => {
            tracing::warn!("Could not read CC proxy request: {error}");
            return StatusCode::BAD_REQUEST.into_response();
        }
    };
    let openai: Value = match serde_json::from_slice(&body) {
        Ok(body) => body,
        Err(error) => {
            tracing::warn!("Could not parse CC proxy request JSON: {error}");
            return StatusCode::BAD_REQUEST.into_response();
        }
    };
    let model = match openai.get("model").and_then(|v| v.as_str()) {
        Some(model) if !model.is_empty() => model.to_string(),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": { "message": "missing model" } })),
            )
                .into_response();
        }
    };
    // DimAgent sends provider-prefixed ids like "cc-proxy/deepseek/deepseek-v4-flash";
    // Command Code expects "vendor/model" (e.g. "deepseek/deepseek-v4-flash") or
    // bare ids for its own models. Strip the proxy's own prefix.
    let cc_model = model
        .strip_prefix("cc-proxy/")
        .unwrap_or(&model)
        .to_string();
    let stream = openai
        .get("stream")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let Some(api_key) = read_api_key() else {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": { "message": "No Command Code API key. Configure ~/.commandcode/auth.json or COMMANDCODE_API_KEY." } })),
        )
            .into_response();
    };
    let cc_body = match build_cc_body(&openai) {
        Ok(body) => body,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": { "message": error } })),
            )
                .into_response();
        }
    };

    let client = match reqwest::Client::builder().build() {
        Ok(client) => client,
        Err(error) => {
            tracing::warn!("Could not create CC proxy HTTP client: {error}");
            return StatusCode::BAD_GATEWAY.into_response();
        }
    };
    let upstream = match client
        .post(format!("{}/alpha/generate", config.api_base))
        .header("Authorization", format!("Bearer {api_key}"))
        .header("Content-Type", "application/json")
        .header("x-command-code-version", CC_CLI_VERSION)
        .header("x-cli-environment", "production")
        .header("x-project-slug", "dim-agent")
        .header("x-taste-learning", "true")
        .header("x-co-flag", "false")
        .json(&cc_body)
        .send()
        .await
    {
        Ok(response) => response,
        Err(error) => {
            let mut error_chain = vec![error.to_string()];
            let mut source = std::error::Error::source(&error);
            while let Some(cause) = source {
                error_chain.push(cause.to_string());
                source = cause.source();
            }
            tracing::warn!(?error_chain, "CC proxy upstream request failed");
            return StatusCode::BAD_GATEWAY.into_response();
        }
    };

    let status = upstream.status();
    if !status.is_success() {
        let err_body = upstream.text().await.unwrap_or_default();
        tracing::warn!(
            "CC upstream error {status}: {}",
            &err_body[..err_body.len().min(500)]
        );
        let axum_status = StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
        return (
            axum_status,
            Json(json!({ "error": { "message": format!("Command Code API error {status}: {}", &err_body[..err_body.len().min(500)]) } })),
        )
            .into_response();
    }

    let id = format!("chatcmpl-{}", uuid_v4().replace('-', ""));
    let id_for_stream = id.clone();
    let usage_log_path = config.usage_log_path.clone();
    let (tx, rx) = mpsc::channel::<Result<Bytes, std::io::Error>>(64);
    let model_for_stream = cc_model.clone();

    tokio::spawn(async move {
        let mut acc = Accumulated::default();
        let mut buffer = String::new();
        let mut stream = upstream.bytes_stream();
        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(bytes) => {
                    buffer.push_str(&String::from_utf8_lossy(&bytes));
                    let mut frames: Vec<String> = Vec::new();
                    let lines: Vec<&str> = buffer.split('\n').collect();
                    let mut keep = String::new();
                    for (i, line) in lines.iter().enumerate() {
                        if i == lines.len() - 1 {
                            keep = (*line).to_string();
                        } else {
                            handle_cc_event(
                                line,
                                &mut acc,
                                &mut frames,
                                &model_for_stream,
                                &id_for_stream,
                            );
                        }
                    }
                    buffer = keep;
                    for frame in frames {
                        if tx.send(Ok(Bytes::from(frame))).await.is_err() {
                            return;
                        }
                    }
                }
                Err(error) => {
                    tracing::warn!("CC upstream stream error: {error}");
                    let _ = tx
                        .send(Err(std::io::Error::other(format!(
                            "upstream stream error: {error}"
                        ))))
                        .await;
                    return;
                }
            }
        }
        // Flush any trailing partial line.
        if !buffer.trim().is_empty() {
            let mut frames: Vec<String> = Vec::new();
            handle_cc_event(
                &buffer,
                &mut acc,
                &mut frames,
                &model_for_stream,
                &id_for_stream,
            );
            for frame in frames {
                if tx.send(Ok(Bytes::from(frame))).await.is_err() {
                    return;
                }
            }
        }

        if let Some(error) = &acc.error {
            let _ = tx
                .send(Ok(Bytes::from(format!(
                    "data: {}\n\n",
                    json!({ "error": { "message": error } })
                ))))
                .await;
        } else if let Some(usage) = &acc.usage
            && let Some(record) = record_usage(usage, &model_for_stream, Utc::now())
            && let Err(error) = append_usage_record(&usage_log_path, &record)
        {
            tracing::warn!("Could not append CC proxy usage record: {error}");
        }
        let _ = tx.send(Ok(Bytes::from("data: [DONE]\n\n"))).await;
    });

    if stream {
        let builder = Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/event-stream")
            .header(header::CACHE_CONTROL, "no-cache")
            .header(header::CONNECTION, "keep-alive");
        builder
            .body(Body::from_stream(
                tokio_stream::wrappers::ReceiverStream::new(rx),
            ))
            .unwrap_or_else(|error| {
                tracing::warn!("Could not build CC proxy stream response: {error}");
                StatusCode::BAD_GATEWAY.into_response()
            })
    } else {
        // Non-streaming: collect all frames, parse the final accumulated state.
        // The spawned task already folded events; we re-run the conversion on
        // the accumulated text by reading frames is wasteful — instead we
        // buffer the whole body here.
        let mut collected = String::new();
        let mut rx = rx;
        while let Some(item) = rx.recv().await {
            match item {
                Ok(bytes) => collected.push_str(&String::from_utf8_lossy(&bytes)),
                Err(_) => break,
            }
        }
        // Rebuild a non-streaming response from the accumulated state is not
        // possible here (acc lives in the task). Simplest correct approach:
        // parse the SSE frames we collected and merge deltas.
        let mut text = String::new();
        let mut reasoning = String::new();
        let mut tool_calls: Vec<Value> = Vec::new();
        let mut finish_reason = "stop".to_string();
        let mut usage: Option<Value> = None;
        for frame in collected.split("\n\n") {
            let data = frame.strip_prefix("data: ").unwrap_or(frame).trim();
            if data.is_empty() || data == "[DONE]" {
                continue;
            }
            let Ok(chunk) = serde_json::from_str::<Value>(data) else {
                continue;
            };
            if let Some(error) = chunk.get("error") {
                return (StatusCode::BAD_GATEWAY, Json(json!({ "error": error }))).into_response();
            }
            if let Some(choices) = chunk.get("choices").and_then(|v| v.as_array()) {
                for choice in choices {
                    if let Some(delta) = choice.get("delta") {
                        if let Some(t) = delta.get("content").and_then(|v| v.as_str()) {
                            text.push_str(t);
                        }
                        if let Some(r) = delta.get("reasoning_content").and_then(|v| v.as_str()) {
                            reasoning.push_str(r);
                        }
                        if let Some(calls) = delta.get("tool_calls").and_then(|v| v.as_array()) {
                            for call in calls {
                                tool_calls.push(call.clone());
                            }
                        }
                    }
                    if let Some(fr) = choice
                        .get("finish_reason")
                        .and_then(|v| v.as_str())
                        .filter(|v| !v.is_empty())
                    {
                        finish_reason = fr.to_string();
                    }
                }
            }
            if let Some(u) = chunk.get("usage") {
                usage = Some(u.clone());
            }
        }
        let mut message = json!({ "role": "assistant", "content": text });
        if !reasoning.is_empty() {
            message["reasoning_content"] = Value::String(reasoning);
        }
        if !tool_calls.is_empty() {
            message["tool_calls"] = Value::Array(tool_calls);
        }
        let mut response = json!({
            "id": id,
            "object": "chat.completion",
            "created": Utc::now().timestamp(),
            "model": model,
            "choices": [{
                "index": 0,
                "message": message,
                "finish_reason": finish_reason,
            }],
        });
        if let Some(u) = usage {
            response["usage"] = u;
        }
        (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/json")],
            Json(response),
        )
            .into_response()
    }
}

fn build_router(config: CcProxyConfig) -> Router {
    Router::new()
        .route("/v1/chat/completions", post(handle_chat))
        .route("/chat/completions", post(handle_chat))
        .route("/v1/models", get(handle_models))
        .route("/models", get(handle_models))
        .with_state(config)
}

pub async fn serve() -> std::io::Result<()> {
    let port = std::env::var("CC_PROXY_PORT")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(8787);
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("Command Code proxy listening on http://{addr}");
    axum::serve(listener, build_router(CcProxyConfig::from_env())).await
}

#[cfg(test)]
mod tests {
    use super::{
        Accumulated, CcProxyConfig, build_cc_body, handle_cc_event, messages_to_cc, record_usage,
        tools_to_cc, usage_to_openai,
    };
    use chrono::Utc;
    use serde_json::{Value, json};
    use std::path::{Path, PathBuf};
    use tempfile::tempdir;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };

    /// Fold raw CC SSE text into an accumulator + emitted OpenAI frames,
    /// mirroring what `handle_chat`'s spawned task does.
    fn fold_sse(raw: &str) -> (Accumulated, Vec<String>) {
        let mut acc = Accumulated::default();
        let mut frames = Vec::new();
        for line in raw.lines() {
            handle_cc_event(line, &mut acc, &mut frames, "test-model", "chatcmpl-test");
        }
        (acc, frames)
    }

    /// Parse every `data:` frame emitted during folding back into JSON.
    fn frames_as_json(frames: &[String]) -> Vec<Value> {
        frames
            .iter()
            .filter_map(|f| {
                let data = f.strip_prefix("data: ").unwrap_or(f).trim();
                serde_json::from_str::<Value>(data).ok()
            })
            .collect()
    }

    // ─── Request conversion: OpenAI → Command Code ─────────────────────

    #[test]
    fn strips_cc_proxy_prefix_from_model_id() {
        // DimAgent sends "cc-proxy/deepseek/deepseek-v4-flash"; CC wants the
        // "vendor/model" form. Without stripping, CC answers 403
        // "Model/provider not recognized: anthropic:cc-proxy/...".
        let body = build_cc_body(&json!({
            "model": "cc-proxy/deepseek/deepseek-v4-flash",
            "messages": [{"role":"user","content":"hi"}],
        }))
        .expect("cc body");

        assert_eq!(body["params"]["model"], "deepseek/deepseek-v4-flash");
        // CC requires streaming for /alpha/generate.
        assert_eq!(body["params"]["stream"], true);
        // threadId must be a UUID or CC rejects the request (400/403).
        let thread_id = body["threadId"].as_str().expect("threadId");
        assert_eq!(
            thread_id.len(),
            36,
            "threadId should be a UUID: {thread_id}"
        );
        assert_eq!(thread_id.matches('-').count(), 4);
    }

    #[test]
    fn leaves_unprefixed_model_id_untouched() {
        let body = build_cc_body(&json!({
            "model": "deepseek/deepseek-v4-flash",
            "messages": [],
        }))
        .expect("cc body");

        assert_eq!(body["params"]["model"], "deepseek/deepseek-v4-flash");
    }

    #[test]
    fn drops_string_tool_choice_and_keeps_object() {
        // CC rejects string tool_choice ("auto"/"none") with
        // "expected object, received string at params.tool_choice".
        let body = build_cc_body(&json!({
            "model": "m",
            "messages": [],
            "tool_choice": "auto",
        }))
        .expect("cc body");
        assert!(
            body["params"].get("tool_choice").is_none(),
            "string tool_choice must be dropped, got {:?}",
            body["params"]["tool_choice"]
        );

        let object_choice = json!({ "type": "function", "name": "read" });
        let body = build_cc_body(&json!({
            "model": "m",
            "messages": [],
            "tool_choice": object_choice.clone(),
        }))
        .expect("cc body");
        assert_eq!(body["params"]["tool_choice"], object_choice);
    }

    #[test]
    fn clamps_max_tokens_to_default() {
        // CC caps at DEFAULT_MAX_TOKENS; larger client values must be clamped
        // rather than forwarded (which 400s).
        let body = build_cc_body(&json!({
            "model": "m",
            "messages": [],
            "max_tokens": 500_000,
        }))
        .expect("cc body");
        assert_eq!(body["params"]["max_tokens"], 64_000);
    }

    #[test]
    fn errors_when_model_missing() {
        assert!(build_cc_body(&json!({ "messages": [] })).is_err());
    }

    #[test]
    fn separates_system_messages_from_history() {
        let (system, messages) = messages_to_cc(&json!([
            {"role":"system","content":"be terse"},
            {"role":"system","content":"use rust"},
            {"role":"user","content":"hi"},
        ]));
        assert_eq!(system, "be terse\n\nuse rust");
        assert_eq!(messages.as_array().map(|a| a.len()), Some(1));
        assert_eq!(messages[0]["role"], "user");
    }

    #[test]
    fn drops_unpaired_tool_calls_and_results() {
        // An assistant tool call with no matching tool result must be dropped:
        // CC rejects histories that reference tool calls it cannot see.
        let (_system, messages) = messages_to_cc(&json!([
            {
                "role": "assistant",
                "tool_calls": [{"id":"call-1","function":{"name":"read","arguments":"{}"}}],
            },
            {"role":"tool","tool_call_id":"call-1","name":"read","content":"ok"},
            {
                "role": "assistant",
                "tool_calls": [{"id":"call-orphan","function":{"name":"read","arguments":"{}"}}],
            },
        ]));

        let arr = messages.as_array().expect("array");
        // assistant(call-1) + tool(call-1) survive; the orphan is dropped.
        assert_eq!(
            arr.len(),
            2,
            "orphan tool call should be dropped: {messages}"
        );
        assert_eq!(arr[0]["role"], "assistant");
        assert_eq!(arr[0]["content"][0]["toolCallId"], "call-1");
        assert_eq!(arr[1]["role"], "tool");
    }

    #[test]
    fn converts_tools_to_cc_input_schema() {
        let tools = tools_to_cc(&json!([{
            "type": "function",
            "function": {
                "name": "read",
                "description": "read a file",
                "parameters": {
                    "type": "object",
                    "properties": { "path": {"type":"string"} },
                    "required": ["path"],
                },
            },
        }]));

        let arr = tools.as_array().expect("array");
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["type"], "function");
        assert_eq!(arr[0]["name"], "read");
        assert_eq!(arr[0]["input_schema"]["type"], "object");
        assert_eq!(
            arr[0]["input_schema"]["properties"]["path"]["type"],
            "string"
        );
        assert_eq!(arr[0]["input_schema"]["required"][0], "path");
    }

    // ─── Usage extraction ──────────────────────────────────────────────

    #[test]
    fn usage_subtracts_cache_read_from_input() {
        // CC reports OpenAI-style `inputTokens` INCLUDING cache reads. Storing
        // it raw would double-count cache and pin hit ratio at <=50%.
        let usage = json!({
            "inputTokens": 130_000,
            "outputTokens": 2_000,
            "inputTokenDetails": { "cacheReadTokens": 100_000, "cacheWriteTokens": 5_000 },
        });
        let record =
            record_usage(&usage, "deepseek/deepseek-v4-flash", Utc::now()).expect("record");

        assert_eq!(record.input_tokens, 25_000); // 130k - 100k - 5k
        assert_eq!(record.cache_read_tokens, 100_000);
        assert_eq!(record.cache_write_tokens, 5_000);
        assert_eq!(record.output_tokens, 2_000);
        assert_eq!(record.total_tokens, 132_000);
        assert_eq!(record.provider, "commandcode");
        assert_eq!(record.source, "cc-proxy");
    }

    #[test]
    fn usage_strips_vendor_prefix_from_model() {
        // "deepseek/deepseek-v4-flash" must be stored as "deepseek-v4-flash"
        // so pricing's cc: mapping and the dashboard match the native
        // commandcode source.
        let usage = json!({ "inputTokens": 10, "outputTokens": 1 });
        let record =
            record_usage(&usage, "deepseek/deepseek-v4-flash", Utc::now()).expect("record");
        assert_eq!(record.model, "deepseek-v4-flash");
    }

    #[test]
    fn usage_skips_all_zero_records() {
        let usage = json!({ "inputTokens": 0, "outputTokens": 0 });
        assert!(record_usage(&usage, "m", Utc::now()).is_none());
    }

    #[test]
    fn usage_to_openai_maps_to_prompt_tokens() {
        let openai = usage_to_openai(&json!({
            "inputTokens": 120,
            "outputTokens": 30,
            "inputTokenDetails": { "cacheReadTokens": 40 },
        }));
        assert_eq!(openai["prompt_tokens"], 120);
        assert_eq!(openai["completion_tokens"], 30);
        assert_eq!(openai["total_tokens"], 150);
        assert_eq!(openai["prompt_tokens_details"]["cached_tokens"], 40);
    }

    // ─── CC SSE → OpenAI frame conversion ──────────────────────────────

    #[test]
    fn folds_text_reasoning_finish_into_frames() {
        let raw = concat!(
            r#"data: {"type":"text-delta","text":"hello"}"#,
            "\n",
            r#"data: {"type":"reasoning-delta","text":"hmm"}"#,
            "\n",
            r#"data: {"type":"finish","finishReason":"stop","totalUsage":{"inputTokens":100,"outputTokens":10}}"#,
        );
        let (acc, frames) = fold_sse(raw);

        assert_eq!(acc.text, "hello");
        assert_eq!(acc.reasoning, "hmm");
        assert_eq!(acc.finish_reason.as_deref(), Some("stop"));

        let chunks = frames_as_json(&frames);
        assert_eq!(chunks.len(), 3, "one frame per event: {chunks:?}");
        assert_eq!(chunks[0]["choices"][0]["delta"]["content"], "hello");
        assert_eq!(chunks[1]["choices"][0]["delta"]["reasoning_content"], "hmm");
        assert_eq!(chunks[2]["choices"][0]["finish_reason"], "stop");
        assert_eq!(chunks[2]["usage"]["prompt_tokens"], 100);
    }

    #[test]
    fn maps_cc_finish_reasons_to_openai() {
        // CC uses "tool-calls"; OpenAI uses "tool_calls".
        let raw = r#"data: {"type":"finish","finishReason":"tool-calls"}"#;
        let (acc, frames) = fold_sse(raw);
        assert_eq!(acc.finish_reason.as_deref(), Some("tool_calls"));
        let chunks = frames_as_json(&frames);
        assert_eq!(chunks[0]["choices"][0]["finish_reason"], "tool_calls");
    }

    #[test]
    fn captures_error_event() {
        let raw = r#"data: {"type":"error","error":{"message":"rate limited"}}"#;
        let (acc, frames) = fold_sse(raw);
        assert_eq!(acc.error.as_deref(), Some("rate limited"));
        assert!(frames.is_empty(), "error events emit no frame");
    }

    #[test]
    fn ignores_non_data_lines_and_malformed_json() {
        let raw = concat!(
            ": keepalive\n",
            "event: ping\n",
            "\n",
            "data: [DONE]\n",
            "data: {not json}\n",
            r#"data: {"type":"text-delta","text":"ok"}"#,
        );
        let (acc, frames) = fold_sse(raw);
        assert_eq!(acc.text, "ok");
        assert_eq!(
            frames.len(),
            1,
            "only the valid text-delta emits: {frames:?}"
        );
    }

    #[test]
    fn tool_call_frame_carries_index_and_arguments() {
        let raw = concat!(
            r#"data: {"type":"tool-call","toolCallId":"call-9","toolName":"read","input":{"path":"/tmp/x"}}"#,
            "\n",
            r#"data: {"type":"finish","finishReason":"tool-calls"}"#,
        );
        let (_acc, frames) = fold_sse(raw);
        let chunks = frames_as_json(&frames);
        let call = &chunks[0]["choices"][0]["delta"]["tool_calls"][0];
        assert_eq!(call["index"], 0);
        assert_eq!(call["id"], "call-9");
        assert_eq!(call["function"]["name"], "read");
        assert_eq!(call["function"]["arguments"], r#"{"path":"/tmp/x"}"#);
    }

    // ─── End-to-end through the router ─────────────────────────────────

    /// Start the proxy on an ephemeral loopback port pointing at `upstream`,
    /// and return its base URL. Avoids a `tower::ServiceExt` dependency while
    /// still exercising the real axum router and HTTP stack.
    async fn spawn_proxy(api_base: &str, models_url: &str, log_path: PathBuf) -> String {
        let config = CcProxyConfig {
            api_base: api_base.to_string(),
            models_url: models_url.to_string(),
            usage_log_path: log_path,
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, super::build_router(config))
                .await
                .unwrap();
        });
        format!("http://{addr}")
    }

    /// Wait for the async recorder task to append at least one usage line.
    async fn wait_for_usage_log(path: &Path) -> String {
        for _ in 0..100 {
            if let Ok(text) = std::fs::read_to_string(path)
                && !text.trim().is_empty()
            {
                return text;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        panic!("usage log never written to {path:?}");
    }

    #[tokio::test]
    async fn chat_streams_upstream_and_appends_usage_record() {
        let upstream = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/alpha/generate"))
            .respond_with(ResponseTemplate::new(200).set_body_string(concat!(
                r#"data: {"type":"text-delta","text":"hi"}"#,
                "\n\n",
                r#"data: {"type":"finish","finishReason":"stop","totalUsage":{"inputTokens":50000,"outputTokens":100,"inputTokenDetails":{"cacheReadTokens":40000}}}"#,
                "\n\n",
            )))
            .mount(&upstream)
            .await;

        let dir = tempdir().unwrap();
        let log_path = dir.path().join("cc-proxy-usage.jsonl");
        let base = spawn_proxy(&upstream.uri(), &upstream.uri(), log_path.clone()).await;

        // `read_api_key()` must succeed, otherwise the handler short-circuits
        // with 401 before reaching the upstream.
        temp_env::async_with_vars([("COMMANDCODE_API_KEY", Some("test-key"))], async {
            let client = reqwest::Client::new();
            let response = client
                .post(format!("{base}/v1/chat/completions"))
                .json(&json!({
                    "model": "cc-proxy/deepseek/deepseek-v4-flash",
                    "messages": [{"role":"user","content":"hi"}],
                    "stream": true,
                }))
                .send()
                .await
                .expect("request");

            assert_eq!(response.status(), reqwest::StatusCode::OK);
            let content_type = response
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .unwrap_or_default()
                .to_string();
            assert!(content_type.contains("text/event-stream"), "{content_type}");

            let text = response.text().await.unwrap();
            assert!(
                text.contains("\"content\":\"hi\""),
                "missing content: {text}"
            );
            assert!(text.ends_with("data: [DONE]\n\n"), "missing [DONE]: {text}");

            let logged = wait_for_usage_log(&log_path).await;
            let record: Value = serde_json::from_str(logged.lines().next().unwrap()).unwrap();
            assert_eq!(record["source"], "cc-proxy");
            assert_eq!(record["model"], "deepseek-v4-flash");
            // 50k input - 40k cache read = 10k non-cached input.
            assert_eq!(record["inputTokens"], 10_000);
            assert_eq!(record["cacheReadTokens"], 40_000);
            assert_eq!(record["outputTokens"], 100);
        })
        .await;
    }

    #[tokio::test]
    async fn chat_returns_401_without_api_key() {
        let dir = tempdir().unwrap();
        let upstream = MockServer::start().await;
        let base = spawn_proxy(
            &upstream.uri(),
            &upstream.uri(),
            dir.path().join("cc-proxy-usage.jsonl"),
        )
        .await;

        // Force `read_api_key()` to fail: empty env + a HOME with no auth.json.
        temp_env::async_with_vars(
            [
                ("COMMANDCODE_API_KEY", Some("")),
                ("HOME", Some(dir.path().to_str().unwrap())),
            ],
            async {
                let client = reqwest::Client::new();
                let response = client
                    .post(format!("{base}/v1/chat/completions"))
                    .json(&json!({ "model": "m", "messages": [] }))
                    .send()
                    .await
                    .expect("request");

                assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);
                let parsed: Value = response.json().await.unwrap();
                assert!(
                    parsed["error"]["message"]
                        .as_str()
                        .unwrap_or_default()
                        .contains("No Command Code API key"),
                    "unexpected body: {parsed}"
                );
            },
        )
        .await;
    }

    #[tokio::test]
    async fn chat_passes_upstream_error_through() {
        let upstream = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/alpha/generate"))
            .respond_with(ResponseTemplate::new(403).set_body_string(r#"{"success":false}"#))
            .mount(&upstream)
            .await;

        let dir = tempdir().unwrap();
        let log_path = dir.path().join("cc-proxy-usage.jsonl");
        let base = spawn_proxy(&upstream.uri(), &upstream.uri(), log_path.clone()).await;

        temp_env::async_with_vars([("COMMANDCODE_API_KEY", Some("test-key"))], async {
            let client = reqwest::Client::new();
            let response = client
                .post(format!("{base}/v1/chat/completions"))
                .json(&json!({ "model": "m", "messages": [] }))
                .send()
                .await
                .expect("request");

            assert_eq!(response.status(), reqwest::StatusCode::FORBIDDEN);
            // A failed upstream call must not produce a usage record.
            assert!(!log_path.exists(), "no usage record for failed call");
        })
        .await;
    }

    #[tokio::test]
    async fn models_proxies_upstream_list() {
        let upstream = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/provider/v1/models"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "data": [{"id": "deepseek/deepseek-v4-flash"}] })),
            )
            .mount(&upstream)
            .await;

        let dir = tempdir().unwrap();
        let base = spawn_proxy(
            &upstream.uri(),
            &format!("{}/provider/v1/models", upstream.uri()),
            dir.path().join("cc-proxy-usage.jsonl"),
        )
        .await;

        temp_env::async_with_vars([("COMMANDCODE_API_KEY", Some("test-key"))], async {
            let client = reqwest::Client::new();
            let response = client
                .get(format!("{base}/v1/models"))
                .send()
                .await
                .expect("request");

            assert_eq!(response.status(), reqwest::StatusCode::OK);
            let parsed: Value = response.json().await.unwrap();
            assert_eq!(parsed["data"][0]["id"], "deepseek/deepseek-v4-flash");
        })
        .await;
    }

    #[test]
    fn config_reads_env_overrides() {
        let dir = tempdir().unwrap();
        let log_path = dir.path().join("usage.jsonl");
        let log_str = log_path.to_str().unwrap().to_string();

        temp_env::with_vars(
            [
                ("COMMANDCODE_API_BASE", Some("https://example.test/")),
                (
                    "COMMANDCODE_MODELS_URL",
                    Some("https://example.test/models"),
                ),
                ("CC_PROXY_USAGE_LOG_PATH", Some(log_str.as_str())),
            ],
            || {
                let config = CcProxyConfig::from_env();
                // Trailing slash is trimmed so path joins don't double up.
                assert_eq!(config.api_base, "https://example.test");
                assert_eq!(config.models_url, "https://example.test/models");
                assert_eq!(config.usage_log_path, log_path);
            },
        );
    }
}
