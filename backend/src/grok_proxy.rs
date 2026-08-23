use crate::models::TokenRecord;
use axum::{
    body::{to_bytes, Body},
    http::{header, HeaderName, Request, Response, StatusCode},
    response::IntoResponse,
    routing::post,
    Router,
};
use chrono::{DateTime, SecondsFormat, Utc};
use futures_util::{stream, StreamExt};
use serde::Deserialize;
use std::{io::Write, net::SocketAddr, path::PathBuf};

const GROK_SOURCE: &str = "grok-cli";

#[derive(Clone)]
pub struct ProxyConfig {
    yai_upstream_base_url: String,
    xai_upstream_base_url: String,
    xai_network_proxy: Option<String>,
    usage_log_path: PathBuf,
}

#[derive(Clone)]
struct ResolvedRoute {
    upstream_base_url: String,
    provider: &'static str,
    canonical_model: &'static str,
    network_proxy: Option<String>,
}

impl ProxyConfig {
    #[cfg(test)]
    fn new(upstream_base_url: String, usage_log_path: PathBuf) -> Self {
        Self {
            yai_upstream_base_url: upstream_base_url.trim_end_matches('/').to_string(),
            xai_upstream_base_url: "https://api.x.ai".to_string(),
            xai_network_proxy: None,
            usage_log_path,
        }
    }

    fn from_env() -> Self {
        let yai_upstream_base_url = std::env::var("GROK_YAI_UPSTREAM_BASE_URL")
            .or_else(|_| std::env::var("GROK_UPSTREAM_BASE_URL"))
            .unwrap_or_else(|_| "https://api.yairouter.com".to_string());
        let xai_upstream_base_url = std::env::var("GROK_XAI_UPSTREAM_BASE_URL")
            .unwrap_or_else(|_| "https://api.x.ai".to_string());
        let xai_network_proxy = std::env::var("GROK_XAI_NETWORK_PROXY")
            .ok()
            .filter(|value| !value.trim().is_empty());
        let usage_log_path = crate::sources::grok_usage_log_path();
        Self {
            yai_upstream_base_url: yai_upstream_base_url.trim_end_matches('/').to_string(),
            xai_upstream_base_url: xai_upstream_base_url.trim_end_matches('/').to_string(),
            xai_network_proxy,
            usage_log_path,
        }
    }

    fn resolve_route(&self, model: &str) -> Option<ResolvedRoute> {
        let (upstream_base_url, provider, canonical_model, network_proxy) = match model {
            "grok-4.5-yai" => (&self.yai_upstream_base_url, "yai-router", "grok-4.5", None),
            "grok-4.6-yai" => (&self.yai_upstream_base_url, "yai-router", "grok-4.6", None),
            "grok-4.5-xai" => (
                &self.xai_upstream_base_url,
                "xai-official",
                "grok-4.5",
                self.xai_network_proxy.clone(),
            ),
            "grok-4.6-xai" => (
                &self.xai_upstream_base_url,
                "xai-official",
                "grok-4.6",
                self.xai_network_proxy.clone(),
            ),
            _ => return None,
        };

        Some(ResolvedRoute {
            upstream_base_url: upstream_base_url.clone(),
            provider,
            canonical_model,
            network_proxy,
        })
    }
}

#[derive(Deserialize)]
struct ResponsePayload {
    #[serde(default)]
    response: Option<ResponseData>,
    #[serde(default)]
    usage: Option<Usage>,
}

#[derive(Deserialize)]
struct ResponseData {
    usage: Usage,
}

#[derive(Deserialize)]
struct Usage {
    input_tokens: i64,
    output_tokens: i64,
    #[serde(default)]
    input_tokens_details: InputTokenDetails,
}

#[derive(Default, Deserialize)]
struct InputTokenDetails {
    #[serde(default)]
    cached_tokens: i64,
    #[serde(default)]
    cache_write_tokens: i64,
}

fn parse_sse_usage_record(
    body: &[u8],
    recorded_at: DateTime<Utc>,
    provider: &str,
    canonical_model: &str,
) -> Option<TokenRecord> {
    let body = std::str::from_utf8(body).ok()?;
    for frame in body.split("\n\n") {
        if !frame
            .lines()
            .any(|line| line.trim() == "event: response.completed")
        {
            continue;
        }
        if let Some(data) = frame.lines().find_map(|line| line.strip_prefix("data: ")) {
            return parse_usage_record(data.as_bytes(), recorded_at, provider, canonical_model);
        }
    }
    None
}

fn parse_usage_record(
    body: &[u8],
    recorded_at: DateTime<Utc>,
    provider: &str,
    canonical_model: &str,
) -> Option<TokenRecord> {
    let ResponsePayload { response, usage } = serde_json::from_slice(body).ok()?;
    let response = response.or_else(|| Some(ResponseData { usage: usage? }))?;
    let cache_read_tokens = response.usage.input_tokens_details.cached_tokens;
    let cache_write_tokens = response.usage.input_tokens_details.cache_write_tokens;
    let input_tokens =
        (response.usage.input_tokens - cache_read_tokens - cache_write_tokens).max(0);

    Some(TokenRecord {
        date: recorded_at.format("%Y-%m-%d").to_string(),
        time: recorded_at.to_rfc3339_opts(SecondsFormat::Millis, true),
        api_key_prefix: String::new(),
        provider: provider.to_string(),
        original_provider: None,
        model: canonical_model.to_string(),
        source: GROK_SOURCE.to_string(),
        input_tokens,
        output_tokens: response.usage.output_tokens,
        cache_read_tokens,
        cache_write_tokens,
        total_tokens: input_tokens
            + response.usage.output_tokens
            + cache_read_tokens
            + cache_write_tokens,
        cost: 0.0,
        ttft_ms: None,
        tps: None,
    })
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
        .unwrap_or("/v1/responses");
    let body = match to_bytes(body, usize::MAX).await {
        Ok(body) => body,
        Err(error) => {
            tracing::warn!("Could not read Grok proxy request: {error}");
            return StatusCode::BAD_REQUEST.into_response();
        }
    };
    let mut request_body: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(body) => body,
        Err(error) => {
            tracing::warn!("Could not parse Grok proxy request JSON: {error}");
            return StatusCode::BAD_REQUEST.into_response();
        }
    };
    let Some(model) = request_body.get("model").and_then(|value| value.as_str()) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let Some(route) = config.resolve_route(model) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    request_body["model"] = serde_json::Value::String(route.canonical_model.to_string());
    let body = match serde_json::to_vec(&request_body) {
        Ok(body) => body,
        Err(error) => {
            tracing::warn!("Could not serialize Grok proxy request JSON: {error}");
            return StatusCode::BAD_REQUEST.into_response();
        }
    };
    let url = format!("{}{}", route.upstream_base_url, path_and_query);
    let mut headers = parts.headers;
    headers.remove(header::HOST);
    // The alias is replaced with the canonical upstream model above, which
    // changes the serialized body's size. Let reqwest calculate Content-Length
    // from the rewritten body rather than forwarding Grok CLI's stale value.
    headers.remove(header::CONTENT_LENGTH);
    // ponytail: per-request client mirrors prior pattern; gzip decompression is
    // required because YAI Router returns gzip-compressed SSE and bytes_stream()
    // would otherwise hand us compressed bytes the usage parser cannot read.
    let mut client_builder = reqwest::Client::builder().gzip(true);
    if let Some(proxy_url) = route.network_proxy.as_deref() {
        let proxy = match reqwest::Proxy::https(proxy_url) {
            Ok(proxy) => proxy,
            Err(error) => {
                tracing::warn!("Invalid xAI network proxy URL: {error}");
                return StatusCode::BAD_GATEWAY.into_response();
            }
        };
        client_builder = client_builder.proxy(proxy);
    }
    let client = match client_builder.build() {
        Ok(client) => client,
        Err(error) => {
            tracing::warn!("Could not create Grok proxy HTTP client: {error}");
            return StatusCode::BAD_GATEWAY.into_response();
        }
    };
    let upstream = match client
        .request(parts.method, url)
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
            tracing::warn!(?error_chain, "Grok proxy upstream request failed");
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
            config,
            is_sse,
            route.provider,
            route.canonical_model,
        ),
        |(mut upstream, mut captured, mut failed, config, is_sse, provider, canonical_model)| async move {
            match upstream.next().await {
                Some(Ok(chunk)) => {
                    captured.extend_from_slice(&chunk);
                    Some((
                        Ok::<_, reqwest::Error>(chunk),
                        (
                            upstream,
                            captured,
                            failed,
                            config,
                            is_sse,
                            provider,
                            canonical_model,
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
                            config,
                            is_sse,
                            provider,
                            canonical_model,
                        ),
                    ))
                }
                None => {
                    if failed {
                        tracing::info!(
                            "grok-proxy stream-end: FAILED flag set, {} bytes, is_sse={}",
                            captured.len(),
                            is_sse
                        );
                        None
                    } else {
                        let record = if is_sse {
                            parse_sse_usage_record(&captured, Utc::now(), provider, canonical_model)
                        } else {
                            parse_usage_record(&captured, Utc::now(), provider, canonical_model)
                        };
                        tracing::info!(
                            "grok-proxy stream-end: {} bytes, is_sse={}, parsed={}, content-type-headers-captured-bytes-first64={}",
                            captured.len(),
                            is_sse,
                            record.is_some(),
                            std::str::from_utf8(captured.get(..64).unwrap_or(&captured))
                                .unwrap_or("<non-utf8>")
                        );
                        if let Some(record) = record {
                            if let Err(error) = append_usage_record(&config.usage_log_path, &record)
                            {
                                tracing::warn!("Could not append Grok usage record: {error}");
                            }
                        }
                        None
                    }
                }
            }
        },
    );

    let mut response = Response::builder().status(status);
    for (name, value) in &headers {
        // Drop Content-Encoding too: reqwest already decompressed the body, so
        // forwarding it would make the client try to gunzip plaintext.
        if !is_hop_by_hop_header(name)
            && name != header::CONTENT_LENGTH
            && name != header::CONTENT_ENCODING
        {
            response = response.header(name, value);
            response = response.header(name, value);
        }
    }
    response
        .body(Body::from_stream(stream))
        .unwrap_or_else(|error| {
            tracing::warn!("Could not build Grok proxy response: {error}");
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
    Router::new()
        .route("/v1/responses", post(handle_proxy))
        .with_state(config)
}

pub async fn serve() -> std::io::Result<()> {
    let port = std::env::var("GROK_PROXY_PORT")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(3434);
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("Grok usage proxy listening on http://{addr}");
    axum::serve(listener, build_router(ProxyConfig::from_env())).await
}

#[cfg(test)]
mod tests {
    use crate::models::TokenRecord;
    use axum::{
        body::{to_bytes, Body},
        http::{Request, StatusCode},
    };
    use chrono::Utc;
    use tempfile::tempdir;
    use wiremock::{
        matchers::{method, path},
        Mock, MockServer, ResponseTemplate,
    };

    use super::{parse_usage_record, proxy_response, ProxyConfig};

    #[test]
    fn parses_terminal_response_usage_with_cached_input() {
        let record = parse_usage_record(
            br#"{"model":"grok-4.5","usage":{"input_tokens":120,"output_tokens":30,"input_tokens_details":{"cached_tokens":40}}}"#,
            Utc::now(),
            "yai-router",
            "grok-4.5",
        )
        .expect("usage record");

        assert_eq!(record.provider, "yai-router");
        assert_eq!(record.input_tokens, 80);
        assert_eq!(record.cache_read_tokens, 40);
        assert_eq!(record.output_tokens, 30);
        assert_eq!(record.total_tokens, 150);
    }

    #[test]
    fn parses_response_completed_sse_event() {
        let record = super::parse_sse_usage_record(
            b"event: response.completed\ndata: {\"response\":{\"model\":\"grok-4.5\",\"usage\":{\"input_tokens\":10,\"output_tokens\":2}}}\n\n",
            Utc::now(),
            "yai-router",
            "grok-4.5",
        )
        .expect("usage record");

        assert_eq!(record.total_tokens, 12);
    }

    #[tokio::test]
    async fn forwards_response_and_records_terminal_usage() {
        let upstream = MockServer::start().await;
        let response_json = r#"{"model":"grok-4.5-provider-version","usage":{"input_tokens":10,"output_tokens":2}}"#;
        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .respond_with(ResponseTemplate::new(200).set_body_string(response_json))
            .mount(&upstream)
            .await;
        let dir = tempdir().unwrap();
        let log_path = dir.path().join("grok-usage.jsonl");

        let response = proxy_response(
            ProxyConfig {
                yai_upstream_base_url: upstream.uri(),
                xai_upstream_base_url: upstream.uri(),
                xai_network_proxy: None,
                usage_log_path: log_path.clone(),
            },
            Request::post("/v1/responses")
                .body(Body::from(r#"{"model":"grok-4.5-xai"}"#))
                .unwrap(),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            to_bytes(response.into_body(), usize::MAX).await.unwrap(),
            response_json
        );
        assert_eq!(
            std::fs::read_to_string(&log_path).unwrap().lines().count(),
            1
        );
        let record: TokenRecord = serde_json::from_str(
            std::fs::read_to_string(&log_path)
                .unwrap()
                .lines()
                .next()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(record.provider, "xai-official");
        assert_eq!(record.model, "grok-4.5");
    }

    #[tokio::test]
    async fn routes_official_alias_to_xai_and_rewrites_the_model() {
        let yai = MockServer::start().await;
        let xai = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"model":"grok-4.5","usage":{"input_tokens":1,"output_tokens":1}}"#,
            ))
            .mount(&xai)
            .await;
        let dir = tempdir().unwrap();
        let response = proxy_response(
            ProxyConfig {
                yai_upstream_base_url: yai.uri(),
                xai_upstream_base_url: xai.uri(),
                xai_network_proxy: None,
                usage_log_path: dir.path().join("grok-usage.jsonl"),
            },
            Request::post("/v1/responses")
                .header("authorization", "Bearer official-key")
                .body(Body::from(r#"{"model":"grok-4.5-xai"}"#))
                .unwrap(),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        assert!(yai.received_requests().await.unwrap().is_empty());
        let requests = xai.received_requests().await.unwrap();
        assert_eq!(requests.len(), 1);
        let request_body: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(request_body["model"], "grok-4.5");
        assert_eq!(
            requests[0]
                .headers
                .get("authorization")
                .unwrap()
                .to_str()
                .unwrap(),
            "Bearer official-key"
        );
    }

    #[tokio::test]
    async fn routes_grok_4_6_official_alias_to_xai_and_rewrites_the_model() {
        let yai = MockServer::start().await;
        let xai = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"model":"grok-4.6","usage":{"input_tokens":1,"output_tokens":1}}"#,
            ))
            .mount(&xai)
            .await;
        let dir = tempdir().unwrap();
        let log_path = dir.path().join("grok-usage.jsonl");
        let response = proxy_response(
            ProxyConfig {
                yai_upstream_base_url: yai.uri(),
                xai_upstream_base_url: xai.uri(),
                xai_network_proxy: None,
                usage_log_path: log_path.clone(),
            },
            Request::post("/v1/responses")
                .header("authorization", "Bearer official-key")
                .body(Body::from(r#"{"model":"grok-4.6-xai"}"#))
                .unwrap(),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        assert!(yai.received_requests().await.unwrap().is_empty());
        let requests = xai.received_requests().await.unwrap();
        assert_eq!(requests.len(), 1);
        let request_body: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(request_body["model"], "grok-4.6");
        // Consume the stream so the terminal usage record is flushed.
        to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let record: TokenRecord = serde_json::from_str(
            std::fs::read_to_string(log_path)
                .unwrap()
                .lines()
                .next()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(record.provider, "xai-official");
        assert_eq!(record.model, "grok-4.6");
    }

    #[tokio::test]
    async fn routes_grok_4_6_yai_alias_to_yai_and_rewrites_the_model() {
        let yai = MockServer::start().await;
        let xai = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"model":"grok-4.6","usage":{"input_tokens":1,"output_tokens":1}}"#,
            ))
            .mount(&yai)
            .await;
        let dir = tempdir().unwrap();
        let log_path = dir.path().join("grok-usage.jsonl");
        let response = proxy_response(
            ProxyConfig {
                yai_upstream_base_url: yai.uri(),
                xai_upstream_base_url: xai.uri(),
                xai_network_proxy: None,
                usage_log_path: log_path.clone(),
            },
            Request::post("/v1/responses")
                .header("authorization", "Bearer yai-key")
                .header("content-length", r#"{"model":"grok-4.6-yai"}"#.len())
                .body(Body::from(r#"{"model":"grok-4.6-yai"}"#))
                .unwrap(),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        assert!(xai.received_requests().await.unwrap().is_empty());
        let requests = yai.received_requests().await.unwrap();
        assert_eq!(requests.len(), 1);
        let request_body: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(request_body["model"], "grok-4.6");
        assert_eq!(
            requests[0]
                .headers
                .get("authorization")
                .unwrap()
                .to_str()
                .unwrap(),
            "Bearer yai-key"
        );
        to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let record: TokenRecord = serde_json::from_str(
            std::fs::read_to_string(log_path)
                .unwrap()
                .lines()
                .next()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(record.provider, "yai-router");
        assert_eq!(record.model, "grok-4.6");
    }

    #[tokio::test]
    async fn routes_yai_alias_to_yai_and_records_its_provider() {
        let yai = MockServer::start().await;
        let xai = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"model":"grok-4.5-provider-version","usage":{"input_tokens":1,"output_tokens":1}}"#,
            ))
            .mount(&yai)
            .await;
        let dir = tempdir().unwrap();
        let log_path = dir.path().join("grok-usage.jsonl");
        let response = proxy_response(
            ProxyConfig {
                yai_upstream_base_url: yai.uri(),
                xai_upstream_base_url: xai.uri(),
                xai_network_proxy: None,
                usage_log_path: log_path.clone(),
            },
            Request::post("/v1/responses")
                .header("authorization", "Bearer yai-key")
                .header("content-length", r#"{"model":"grok-4.5-yai"}"#.len())
                .body(Body::from(r#"{"model":"grok-4.5-yai"}"#))
                .unwrap(),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert!(xai.received_requests().await.unwrap().is_empty());
        let requests = yai.received_requests().await.unwrap();
        assert_eq!(requests.len(), 1);
        let request_body: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(request_body["model"], "grok-4.5");
        assert_eq!(
            requests[0]
                .headers
                .get("authorization")
                .unwrap()
                .to_str()
                .unwrap(),
            "Bearer yai-key"
        );
        let record: TokenRecord = serde_json::from_str(
            std::fs::read_to_string(log_path)
                .unwrap()
                .lines()
                .next()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(record.provider, "yai-router");
        assert_eq!(record.model, "grok-4.5");
    }

    #[tokio::test]
    async fn rejects_unknown_model_alias_without_forwarding() {
        let upstream = MockServer::start().await;
        let dir = tempdir().unwrap();
        let response = proxy_response(
            ProxyConfig::new(upstream.uri(), dir.path().join("grok-usage.jsonl")),
            Request::post("/v1/responses")
                .body(Body::from(r#"{"model":"unknown-model"}"#))
                .unwrap(),
        )
        .await;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert!(upstream.received_requests().await.unwrap().is_empty());
    }

    // Regression: YAI Router returns gzip-compressed SSE. The proxy must
    // transparently decompress before parsing usage, otherwise bytes_stream()
    // hands the parser compressed bytes and nothing gets recorded.
    #[tokio::test]
    async fn records_usage_from_gzip_compressed_sse_upstream() {
        use flate2::write::GzEncoder;
        use flate2::Compression;
        use std::io::Write;

        let upstream = MockServer::start().await;
        let sse = "event: response.completed\n\
            data: {\"response\":{\"model\":\"grok-4.5\",\"usage\":{\"input_tokens\":10,\"output_tokens\":2}}}\n\n";
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(sse.as_bytes()).unwrap();
        let gzipped = encoder.finish().unwrap();

        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-encoding", "gzip")
                    .set_body_raw(gzipped, "text/event-stream"),
            )
            .mount(&upstream)
            .await;
        let dir = tempdir().unwrap();
        let log_path = dir.path().join("grok-usage.jsonl");

        let response = proxy_response(
            ProxyConfig::new(upstream.uri(), log_path.clone()),
            Request::post("/v1/responses")
                .body(Body::from(r#"{"model":"grok-4.5-yai"}"#))
                .unwrap(),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        // Client receives decompressed SSE plaintext (Content-Encoding stripped).
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert!(std::str::from_utf8(&body)
            .unwrap()
            .contains("response.completed"));
        // Proxy parsed the decompressed terminal event and recorded one line.
        assert_eq!(
            std::fs::read_to_string(log_path).unwrap().lines().count(),
            1
        );
    }
}
