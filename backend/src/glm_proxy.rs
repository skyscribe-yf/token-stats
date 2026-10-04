//! Loopback GLM usage proxy for the Paseo `glm-acp-agent`.
//!
//! `glm-acp-agent` (npm) talks straight to Z.AI's OpenAI-compatible coding
//! endpoint and persists nothing — its session store carries messages only,
//! no usage — so Paseo-driven GLM traffic was invisible to the dashboard.
//! The agent honours `ACP_GLM_BASE_URL`, so Paseo points it here and this
//! proxy forwards every request verbatim to the upstream, tees the response
//! stream, and scrapes the usage object the agent already asks for
//! (`stream_options: { include_usage: true }` on every call).
//!
//! Pure passthrough: the caller's Authorization header and request body
//! (model included) are forwarded untouched; the proxy never rewrites or
//! injects credentials. Usage records append to the `glm-acp` source log
//! with `provider = bigmodel` (GLM Coding Plan vendor — billing detail via
//! the plan-credit branch is a zcode-source concern; these records price at
//! the GLM list price).

use crate::models::TokenRecord;
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{HeaderName, Request, Response, StatusCode, header},
    response::IntoResponse,
};
use chrono::{DateTime, SecondsFormat, Utc};
use compact_str::CompactString;
use futures_util::{StreamExt, stream};
use serde::Deserialize;
use std::{io::Write, net::SocketAddr, path::PathBuf, time::Instant};

const GLM_SOURCE: &str = "glm-acp";
const GLM_PROVIDER: &str = "bigmodel";

#[derive(Clone)]
pub struct ProxyConfig {
    upstream_base_url: String,
    usage_log_path: PathBuf,
}

impl ProxyConfig {
    #[cfg(test)]
    fn new(upstream_base_url: String, usage_log_path: PathBuf) -> Self {
        Self {
            upstream_base_url: upstream_base_url.trim_end_matches('/').to_string(),
            usage_log_path,
        }
    }

    fn from_env() -> Self {
        let upstream_base_url = std::env::var("GLM_PROXY_UPSTREAM_BASE_URL")
            .map(|value| value.trim_end_matches('/').to_string())
            .unwrap_or_else(|_| "https://api.z.ai".to_string());
        Self {
            upstream_base_url,
            usage_log_path: crate::sources::glm_acp_usage_log_path(),
        }
    }
}

#[derive(Deserialize)]
struct PromptTokensDetails {
    #[serde(default)]
    cached_tokens: i64,
}

#[derive(Deserialize)]
struct ChatUsage {
    #[serde(default)]
    prompt_tokens: i64,
    #[serde(default)]
    completion_tokens: i64,
    #[serde(default)]
    prompt_tokens_details: Option<PromptTokensDetails>,
}

/// Extract `(prompt, completion, cached)` from an OpenAI-style chat
/// completion `usage` object; `None` when the value is not an object.
fn extract_usage(value: &serde_json::Value) -> Option<(i64, i64, i64)> {
    let usage: ChatUsage = serde_json::from_value(value.get("usage")?.clone()).ok()?;
    let cached = usage.prompt_tokens_details.map_or(0, |d| d.cached_tokens);
    Some((usage.prompt_tokens, usage.completion_tokens, cached))
}

/// Scan a buffered SSE body for the final usage frame (OpenAI streaming with
/// `include_usage` ships one choices-empty chunk carrying `usage`; taking the
/// last also covers providers that attach usage to the final content chunk).
fn parse_sse_usage(body: &[u8]) -> Option<(i64, i64, i64)> {
    let body = std::str::from_utf8(body).ok()?;
    let mut last = None;
    for line in body.lines() {
        let Some(data) = line.strip_prefix("data:") else {
            continue;
        };
        let data = data.trim();
        if data.is_empty() || data == "[DONE]" {
            continue;
        }
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(data) {
            if let Some(usage) = extract_usage(&value) {
                last = Some(usage);
            }
        }
    }
    last
}

/// Usage from a non-streaming chat completion JSON body.
fn parse_json_usage(body: &[u8]) -> Option<(i64, i64, i64)> {
    serde_json::from_slice::<serde_json::Value>(body)
        .ok()
        .and_then(|value| extract_usage(&value))
}

fn build_record(
    model: &str,
    (prompt, completion, cached): (i64, i64, i64),
    recorded_at: DateTime<Utc>,
    ttft_ms: Option<f64>,
) -> TokenRecord {
    // OpenAI convention: prompt_tokens includes cached tokens; normalize to
    // the Anthropic convention used across the dashboard. Reasoning tokens
    // are a subset of completion_tokens, not additive.
    let input = (prompt - cached).max(0);
    TokenRecord {
        date: compact_str::format_compact!("{}", recorded_at.format("%Y-%m-%d")),
        time: recorded_at.to_rfc3339_opts(SecondsFormat::Millis, true),
        api_key_prefix: CompactString::default(),
        provider: GLM_PROVIDER.into(),
        original_provider: None,
        model: model.into(),
        source: GLM_SOURCE.into(),
        input_tokens: input,
        output_tokens: completion,
        cache_read_tokens: cached,
        cache_write_tokens: 0,
        total_tokens: input + completion + cached,
        cost: 0.0,
        ttft_ms,
        tps: None,
    }
}

fn append_usage_record(path: &PathBuf, record: &TokenRecord) -> std::io::Result<()> {
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

fn is_hop_by_hop_header(name: &HeaderName) -> bool {
    matches!(
        name.as_str(),
        "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
    )
}

async fn proxy_response(config: ProxyConfig, request: Request<Body>) -> Response<Body> {
    let (parts, body) = request.into_parts();
    let path_and_query = parts
        .uri
        .path_and_query()
        .map(|value| value.as_str())
        .unwrap_or("/");
    let body = match to_bytes(body, usize::MAX).await {
        Ok(body) => body,
        Err(error) => {
            tracing::warn!("Could not read GLM proxy request: {error}");
            return StatusCode::BAD_REQUEST.into_response();
        }
    };
    // The body is forwarded verbatim; the model is only read for the usage
    // record. Requests without a parseable JSON body still pass through —
    // they just cannot be metered.
    let model = serde_json::from_slice::<serde_json::Value>(&body)
        .ok()
        .and_then(|value| {
            value
                .get("model")
                .and_then(|model| model.as_str())
                .map(str::to_string)
        });

    let mut headers = parts.headers;
    headers.remove(header::HOST);
    headers.remove(header::CONTENT_LENGTH);

    let started = Instant::now();
    let client = match reqwest::Client::builder().gzip(true).build() {
        Ok(client) => client,
        Err(error) => {
            tracing::warn!("Could not create GLM proxy HTTP client: {error}");
            return StatusCode::BAD_GATEWAY.into_response();
        }
    };
    let upstream = match client
        .request(parts.method, format!("{}{}", config.upstream_base_url, path_and_query))
        .headers(headers)
        .body(body)
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
            tracing::warn!(?error_chain, "GLM proxy upstream request failed");
            return StatusCode::BAD_GATEWAY.into_response();
        }
    };

    let status = upstream.status();
    let headers = upstream.headers().clone();
    let is_sse = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("text/event-stream"));
    let stream = stream::unfold(
        (
            upstream.bytes_stream(),
            Vec::new(),
            false,
            None::<Instant>,
            started,
            is_sse,
            model,
            config,
        ),
        |(mut upstream, mut captured, mut failed, mut first_byte, started, is_sse, model, config)| async move {
            match upstream.next().await {
                Some(Ok(chunk)) => {
                    if first_byte.is_none() {
                        first_byte = Some(Instant::now());
                    }
                    captured.extend_from_slice(&chunk);
                    Some((
                        Ok::<_, reqwest::Error>(chunk),
                        (
                            upstream,
                            captured,
                            failed,
                            first_byte,
                            started,
                            is_sse,
                            model,
                            config,
                        ),
                    ))
                }
                Some(Err(error)) => {
                    failed = true;
                    Some((
                        Err(error),
                        (
                            upstream,
                            captured,
                            failed,
                            first_byte,
                            started,
                            is_sse,
                            model,
                            config,
                        ),
                    ))
                }
                None => {
                    // Failed upstream streams and requests without a model in
                    // the body produce no record; the bytes were still passed
                    // through to the client unchanged.
                    if !failed {
                        let usage = if is_sse {
                            parse_sse_usage(&captured)
                        } else {
                            parse_json_usage(&captured)
                        };
                        if let (Some(usage), Some(model)) = (usage, model.as_deref()) {
                            let ttft_ms =
                                first_byte.map(|t| t.saturating_duration_since(started).as_millis() as f64);
                            let record = build_record(model, usage, Utc::now(), ttft_ms);
                            if let Err(error) =
                                append_usage_record(&config.usage_log_path, &record)
                            {
                                tracing::warn!("Could not append GLM usage record: {error}");
                            }
                        }
                    }
                    None
                }
            }
        },
    );

    let mut response = Response::builder().status(status);
    for (name, value) in &headers {
        // Content-Encoding is dropped: reqwest already decompressed the body
        // (gzip(true)) and forwarding the header would make the client try to
        // gunzip plaintext.
        if !is_hop_by_hop_header(name)
            && name != header::CONTENT_LENGTH
            && name != header::CONTENT_ENCODING
        {
            response = response.header(name, value);
        }
    }
    response
        .body(Body::from_stream(stream))
        .unwrap_or_else(|error| {
            tracing::warn!("Could not build GLM proxy response: {error}");
            StatusCode::BAD_GATEWAY.into_response()
        })
}

async fn handle_proxy(
    axum::extract::State(config): axum::extract::State<ProxyConfig>,
    request: Request<Body>,
) -> Response<Body> {
    proxy_response(config, request).await
}

fn build_router(config: ProxyConfig) -> Router {
    // Fallback (not a fixed route): the agent addresses
    // `{base}/chat/completions` under whatever path Paseo configured as
    // ACP_GLM_BASE_URL, and any other endpoint (model lists etc.) must pass
    // through untouched as well.
    Router::new().fallback(handle_proxy).with_state(config)
}

pub async fn serve() -> std::io::Result<()> {
    let port = std::env::var("GLM_PROXY_PORT")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(3435);
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("GLM usage proxy listening on http://{addr}");
    axum::serve(listener, build_router(ProxyConfig::from_env())).await
}

#[cfg(test)]
mod tests {
    use super::{
        ProxyConfig, build_record, parse_json_usage, parse_sse_usage, proxy_response,
    };
    use crate::models::TokenRecord;
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode},
    };
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };

    #[test]
    fn parses_streaming_usage_chunk_with_cached_tokens() {
        let sse = b"data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n\n\
                    data: {\"choices\":[],\"usage\":{\"prompt_tokens\":1000,\"completion_tokens\":120,\"total_tokens\":1120,\"prompt_tokens_details\":{\"cached_tokens\":640}}}\
                    \n\ndata: [DONE]\n\n";
        let usage = parse_sse_usage(sse).expect("usage from final chunk");

        // prompt_tokens includes the cached reads; normalize like zcode.
        assert_eq!(usage, (1000, 120, 640));
        let record = build_record("glm-5.3-flash", usage, chrono::Utc::now(), None);
        assert_eq!(record.input_tokens, 360);
        assert_eq!(record.output_tokens, 120);
        assert_eq!(record.cache_read_tokens, 640);
        assert_eq!(record.total_tokens, 1120);
        assert_eq!(record.source, "glm-acp");
        assert_eq!(record.provider, "bigmodel");
    }

    #[test]
    fn last_usage_frame_wins() {
        let sse = b"data: {\"choices\":[],\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":5}}\n\n\
                    data: {\"choices\":[],\"usage\":{\"prompt_tokens\":20,\"completion_tokens\":7,\"prompt_tokens_details\":{\"cached_tokens\":4}}}\n\n";
        assert_eq!(parse_sse_usage(sse), Some((20, 7, 4)));
    }

    #[test]
    fn parses_non_streaming_usage() {
        let body = br#"{"id":"x","model":"glm-5.3-flash","choices":[{"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}],"usage":{"prompt_tokens":30,"completion_tokens":4,"total_tokens":34}}"#;
        assert_eq!(parse_json_usage(body), Some((30, 4, 0)));
    }

    #[test]
    fn sse_without_usage_yields_nothing() {
        assert_eq!(parse_sse_usage(b"data: {\"choices\":[{\"delta\":{\"content\":\"x\"}}]}\n\n"), None);
        assert_eq!(parse_sse_usage(b"data: [DONE]\n\n"), None);
    }

    #[tokio::test]
    async fn forwards_verbatim_and_records_usage() {
        let upstream = MockServer::start().await;
        let response_json = r#"{"model":"glm-5.3-flash","choices":[{"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}],"usage":{"prompt_tokens":30,"completion_tokens":4,"total_tokens":34}}"#;
        Mock::given(method("POST"))
            .and(path("/api/coding/paas/v4/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_string(response_json))
            .mount(&upstream)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("glm-acp-usage.jsonl");

        let response = proxy_response(
            ProxyConfig::new(upstream.uri(), log_path.clone()),
            Request::post("/api/coding/paas/v4/chat/completions")
                .header("authorization", "Bearer caller-key")
                .body(Body::from(
                    r#"{"model":"glm-5.3-flash","stream":false,"messages":[{"role":"user","content":"hi"}]}"#,
                ))
                .unwrap(),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            to_bytes(response.into_body(), usize::MAX).await.unwrap(),
            response_json
        );
        let record: TokenRecord = serde_json::from_str(
            std::fs::read_to_string(&log_path)
                .unwrap()
                .lines()
                .next()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(record.source, "glm-acp");
        assert_eq!(record.provider, "bigmodel");
        assert_eq!(record.model, "glm-5.3-flash");
        assert_eq!(record.input_tokens, 30);
        assert_eq!(record.output_tokens, 4);
    }

    #[tokio::test]
    async fn records_streaming_sse_and_passes_it_through() {
        let upstream = MockServer::start().await;
        let sse = "data: {\"choices\":[{\"delta\":{\"content\":\"hello\"}}]}\n\n\
                   data: {\"choices\":[],\"usage\":{\"prompt_tokens\":50,\"completion_tokens\":9,\"prompt_tokens_details\":{\"cached_tokens\":10}}}\n\n\
                   data: [DONE]\n\n";
        Mock::given(method("POST"))
            .and(path("/api/coding/paas/v4/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(sse, "text/event-stream"))
            .mount(&upstream)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("glm-acp-usage.jsonl");

        let response = proxy_response(
            ProxyConfig::new(upstream.uri(), log_path.clone()),
            Request::post("/api/coding/paas/v4/chat/completions")
                .body(Body::from(r#"{"model":"glm-5.3-flash","stream":true}"#))
                .unwrap(),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            to_bytes(response.into_body(), usize::MAX).await.unwrap(),
            sse
        );
        let record: TokenRecord = serde_json::from_str(
            std::fs::read_to_string(&log_path)
                .unwrap()
                .lines()
                .next()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(record.input_tokens, 40);
        assert_eq!(record.cache_read_tokens, 10);
        assert_eq!(record.total_tokens, 59);
        assert!(record.ttft_ms.is_some());
    }

    #[tokio::test]
    async fn upstream_errors_forward_without_a_record() {
        let upstream = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(401).set_body_string(r#"{"error":{"message":"bad key"}}"#))
            .mount(&upstream)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("glm-acp-usage.jsonl");

        let response = proxy_response(
            ProxyConfig::new(upstream.uri(), log_path.clone()),
            Request::post("/api/coding/paas/v4/chat/completions")
                .body(Body::from(r#"{"model":"glm-5.3-flash"}"#))
                .unwrap(),
        )
        .await;

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert!(!log_path.exists());
    }
}
