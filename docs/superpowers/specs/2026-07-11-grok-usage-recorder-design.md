# Grok Usage Recorder Design

## Goal

Capture exact Grok CLI request usage while retaining the dashboard's existing aggregate views. Grok-derived records must not appear in the detailed request list.

## Context

Grok CLI session files under `~/.grok/sessions` contain cumulative context counters but no per-request input, output, cache, or cost breakdown. The configured YAI Router dashboard exposes aggregate usage, but cannot attribute requests to Grok when an API key is shared.

## Selected Approach

A dedicated `token-stats-grok-proxy.service` starts the dashboard binary with `--grok-proxy-only` and owns `127.0.0.1:${GROK_PROXY_PORT:-3434}`. Dashboard instances do not start proxy listeners, so the stable Grok endpoint survives blue-green dashboard swaps. Grok is configured to use this loopback URL as its model base URL. The proxy forwards requests to `GROK_UPSTREAM_BASE_URL`, defaulting to `https://api.yairouter.com`.

The proxy preserves the client's authorization header and does not store API keys. It streams upstream responses through unchanged. For a non-stream response or a terminal SSE `response.completed` event, it extracts only the model, completion timestamp, and API usage fields and appends one JSONL record to `~/.token-stats/grok-usage.jsonl`.

## Recorded Fields

Each record contains:

- UTC timestamp
- Model and resolved provider
- Non-cached input tokens
- Output tokens, including reasoning when the provider reports them together
- Cache read and cache write tokens when provided
- Total tokens
- Optional provider-reported cost

Prompt content, completion text, headers, request bodies, and credentials are never written to the usage log.

## Dashboard Integration

`GrokProxySource` reads the JSONL file through the existing `DataSource` path with `source = "grok-cli"`. The normal refresh cycle loads it with other sources.

Aggregate routes include Grok data in overall, vendor, model, date, and source statistics. The proxy keeps one record per request for accurate aggregation, but detailed-request routes exclude records with `source = "grok-cli"`.

The frontend adds a Grok CLI label and source color. Existing filters and vendor/model source details discover the new source automatically.

## Configuration

- `GROK_PROXY_PORT`: loopback listener port, default `3434`
- `GROK_UPSTREAM_BASE_URL`: upstream API base URL, default `https://api.yairouter.com`
- `GROK_USAGE_LOG_PATH`: usage JSONL path, default `~/.token-stats/grok-usage.jsonl`

`setup.sh` installs and starts the service. `deploy.sh` restarts it after the prior dashboard instance releases port `3434`.

Grok's `models_base_url` changes to `http://127.0.0.1:3434/v1`; its existing API key remains in Grok configuration and is forwarded upstream.

## Error Handling

The proxy remains transparent to Grok. Upstream transport and status failures are returned unchanged. Usage-log parsing or append failures are logged but do not alter the upstream response. Malformed or usage-free responses produce no record.

## Verification

- Unit-test terminal JSON and SSE usage parsing.
- Unit-test normalized cache semantics and JSONL loading.
- Verify a proxy fixture yields aggregate Grok model/source statistics but zero detailed requests.
- Verify the proxy listener only binds loopback and passes upstream response bytes unchanged.
