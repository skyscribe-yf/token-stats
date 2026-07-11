# Grok Usage Recorder Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Record provider-reported Grok Responses API usage through a loopback proxy and include it in aggregate analytics without adding it to the detailed-request list.

**Architecture:** A loopback Axum router proxies `/v1/responses` to YAI Router, streams the upstream response without alteration, and records terminal usage JSON to a local JSONL file. The existing source registry loads the JSONL as `grok-cli`; the aggregate path needs no special handling, while the detailed-request handler drops that source.

**Tech Stack:** Rust 2021, Axum 0.8, Reqwest 0.12, Tokio, Serde, React 19, TypeScript.

---

### Task 1: Parse and Persist Terminal Usage

**Files:**
- Create: `backend/src/grok_proxy.rs`
- Modify: `backend/Cargo.toml`
- Modify: `backend/src/main.rs`
- Test: inline unit tests in `backend/src/grok_proxy.rs`

- [ ] **Step 1: Write the failing terminal-response parser test**

```rust
#[test]
fn parses_terminal_response_usage_with_cached_input() {
    let record = parse_usage_record(
        br#"{\"model\":\"grok-4.5\",\"usage\":{\"input_tokens\":120,\"output_tokens\":30,\"input_tokens_details\":{\"cached_tokens\":40}}}"#,
        Utc::now(),
    )
    .expect("usage record");

    assert_eq!(record.provider, "xai");
    assert_eq!(record.input_tokens, 80);
    assert_eq!(record.cache_read_tokens, 40);
    assert_eq!(record.output_tokens, 30);
    assert_eq!(record.total_tokens, 150);
}
```

- [ ] **Step 2: Run the parser test and verify it fails because `grok_proxy` is absent**

Run: `cd backend && cargo test grok_proxy::tests::parses_terminal_response_usage_with_cached_input`

Expected: compilation failure referencing the missing module or parser.

- [ ] **Step 3: Implement the minimal parser and append helper**

```rust
fn parse_usage_record(body: &[u8], recorded_at: DateTime<Utc>) -> Option<TokenRecord> {
    let payload: ResponsePayload = serde_json::from_slice(body).ok()?;
    let response = payload.response.unwrap_or(payload.into_response()?);
    let usage = response.usage?;
    let cache_read = usage.input_tokens_details.cached_tokens;
    let cache_write = usage.input_tokens_details.cache_write_tokens;
    let input = (usage.input_tokens - cache_read - cache_write).max(0);
    Some(TokenRecord { /* normalized grok-cli record */ })
}
```

Use `std::fs::create_dir_all` and `OpenOptions::append(true).create(true)` to append exactly one JSON line. Keep this helper synchronous and call it only after an upstream response ends.

- [ ] **Step 4: Run the parser test and verify it passes**

Run: `cd backend && cargo test grok_proxy::tests::parses_terminal_response_usage_with_cached_input`

Expected: one passing test.

- [ ] **Step 5: Write the failing SSE terminal-event parser test**

```rust
#[test]
fn parses_response_completed_sse_event() {
    let record = parse_sse_usage_record(
        b"event: response.completed\ndata: {\"response\":{\"model\":\"grok-4.5\",\"usage\":{\"input_tokens\":10,\"output_tokens\":2}}}\n\n",
        Utc::now(),
    )
    .expect("usage record");

    assert_eq!(record.total_tokens, 12);
}
```

- [ ] **Step 6: Implement terminal SSE extraction and verify it passes**

Split SSE frames on blank lines, select only `event: response.completed`, parse its `data:` line with the same usage parser, and ignore all other frames. Run:

`cd backend && cargo test grok_proxy::tests`

Expected: all Grok proxy parser tests pass.

- [ ] **Step 7: Commit the parser task**

```bash
git add backend/Cargo.toml backend/src/grok_proxy.rs backend/src/main.rs
git commit -m "feat: record Grok response usage"
```

### Task 2: Stream the Responses API Through a Loopback Listener

**Files:**
- Modify: `backend/src/grok_proxy.rs`
- Test: inline async tests in `backend/src/grok_proxy.rs`

- [ ] **Step 1: Write the failing proxy pass-through test**

```rust
#[tokio::test]
async fn proxy_forwards_status_body_and_records_terminal_usage() {
    let upstream = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(ResponseTemplate::new(200).set_body_string(response_json))
        .mount(&upstream)
        .await;

    let response = build_router(proxy_config(upstream.uri(), log_path))
        .oneshot(Request::post("/v1/responses").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(to_bytes(response.into_body(), usize::MAX).await.unwrap(), response_json);
    assert_eq!(load_usage_log(log_path).len(), 1);
}
```

- [ ] **Step 2: Run it and verify it fails because the route has not been implemented**

Run: `cd backend && cargo test grok_proxy::tests::proxy_forwards_status_body_and_records_terminal_usage`

Expected: test failure from a `404 Not Found` response or missing router function.

- [ ] **Step 3: Implement the narrow proxy route**

Add a `POST /v1/responses` router. Buffer the loopback request body, forward its method, headers except `Host`, URI query, and body with `reqwest`. Convert the upstream byte stream to an Axum body with `futures_util::stream::unfold`; yield every chunk unchanged while accumulating bytes. After a clean upstream end, parse JSON or SSE terminal usage and append the normalized record. Copy status and end-to-end response headers while excluding hop-by-hop headers.

Start the listener from `main` on `127.0.0.1:${GROK_PROXY_PORT:-3434}`. Binding failure must log an error and leave the dashboard listener running.

- [ ] **Step 4: Run the proxy tests and verify they pass**

Run: `cd backend && cargo test grok_proxy::tests`

Expected: parser and pass-through tests pass.

- [ ] **Step 5: Commit the proxy task**

```bash
git add backend/Cargo.toml backend/src/grok_proxy.rs backend/src/main.rs
git commit -m "feat: proxy Grok Responses API locally"
```

### Task 3: Load Grok Records into Aggregates Only

**Files:**
- Create: `backend/src/sources/grok_cli.rs`
- Modify: `backend/src/sources/mod.rs`
- Modify: `backend/src/aggregator.rs`
- Test: inline tests in `backend/src/sources/grok_cli.rs` and `backend/src/aggregator.rs`

- [ ] **Step 1: Write the failing source-loader test**

```rust
#[test]
fn loads_grok_usage_jsonl() {
    temp_env::with_var("GROK_USAGE_LOG_PATH", Some(log_path), || {
        std::fs::write(log_path, grok_record_json_line()).unwrap();
        let records = GrokCliSource.load();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].source, "grok-cli");
    });
}
```

- [ ] **Step 2: Run it and verify it fails because `GrokCliSource` is absent**

Run: `cd backend && cargo test sources::grok_cli::tests::loads_grok_usage_jsonl`

Expected: compilation failure mentioning the missing source.

- [ ] **Step 3: Implement the JSONL source**

Add `GrokCliSource` with a `GROK_USAGE_LOG_PATH` override and default `~/.token-stats/grok-usage.jsonl`. Read non-empty lines, deserialize `TokenRecord`, keep only `source == "grok-cli"`, skip malformed lines with a warning, and register it in `load_all_sources()`.

- [ ] **Step 4: Write the failing detailed-list exclusion test**

```rust
#[test]
fn grok_records_are_excluded_from_paginated_requests() {
    let grok = make_record("grok-cli", "grok-4.5");
    let pi = make_record("pi", "gpt-5.5");
    let page = paginate_requests(vec![&grok, &pi], 1, 50, None);
    assert_eq!(page.total, 1);
    assert_eq!(page.data[0].source, "pi");
}
```

- [ ] **Step 5: Implement the minimal exclusion at the shared pagination boundary**

Filter `records` in `paginate_requests` with `record.source != "grok-cli"` before calculating `total`, page bounds, and output rows. This preserves the existing aggregate and filter logic for Grok but makes detailed requests consistently exclude it.

- [ ] **Step 6: Run source and aggregation tests**

Run: `cd backend && cargo test grok_cli && cargo test grok_records_are_excluded_from_paginated_requests`

Expected: both commands pass.

- [ ] **Step 7: Commit the source task**

```bash
git add backend/src/sources/grok_cli.rs backend/src/sources/mod.rs backend/src/aggregator.rs
git commit -m "feat: aggregate recorded Grok usage"
```

### Task 4: Present Grok CLI as a Source

**Files:**
- Modify: `frontend/src/lib/utils.ts`
- Test: `frontend/src/lib/utils.test.ts`

- [ ] **Step 1: Write the failing source label test**

```ts
import { getSourceLabel } from "./utils";

test("labels recorded Grok usage", () => {
  expect(getSourceLabel("grok-cli")).toBe("Grok CLI");
});
```

- [ ] **Step 2: Run it and verify it fails because the label is missing**

Run: `cd frontend && npm test -- utils.test.ts`

Expected: assertion failure returning `grok-cli`.

- [ ] **Step 3: Add the source label and distinct color**

```ts
"grok-cli": "#e11d48",
```

```ts
"grok-cli": "Grok CLI",
```

- [ ] **Step 4: Run frontend checks**

Run: `cd frontend && npm test -- utils.test.ts && npm run build`

Expected: the utility test and production build pass.

- [ ] **Step 5: Commit the frontend task**

```bash
git add frontend/src/lib/utils.ts frontend/src/lib/utils.test.ts
git commit -m "feat: label Grok CLI usage"
```

### Task 5: Full Verification

**Files:**
- Verify only

- [ ] **Step 1: Run backend tests**

Run: `cd backend && cargo test`

Expected: all backend tests pass.

- [ ] **Step 2: Run the frontend test suite and build**

Run: `cd frontend && npm test -- --run && npm run build`

Expected: all tests pass and Vite writes static assets to `backend/static`.

- [ ] **Step 3: Inspect the final diff**

Run: `git diff 6c198f2 --check && git status --short`

Expected: no whitespace errors; only intended source, dependency, frontend, and plan changes are present.
