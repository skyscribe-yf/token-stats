# Grok CLI Official xAI Routing Design

## Goal

Let Grok CLI select either the existing YAI Router route or the official xAI
route with `-m`, while retaining separate API keys and separate dashboard
usage attribution.

## Selected Approach

Keep one loopback usage proxy at `127.0.0.1:3434` and expose two client-facing
model aliases:

- `grok-4.5-yai` routes to `https://api.yairouter.com` and records provider
  `yai-router`.
- `grok-4.5-xai` routes to `https://api.x.ai` and records provider
  `xai-official`.

The proxy rewrites either alias to the upstream model ID `grok-4.5`. It keeps
the request's `Authorization` header intact, so the proxy neither receives
configuration for nor persists either API key.

## Grok CLI Configuration

`~/.grok/config.toml` continues to use one global
`models_base_url = "http://127.0.0.1:3434/v1"`. It defines one model entry per
alias, each with the appropriate API key and `api_backend = "responses"`.

The aliases are CLI-only routing names. Both upstream services receive
`model: "grok-4.5"`.

## Proxy Behaviour

The proxy reads the request JSON before forwarding it. A recognized alias
resolves to an upstream base URL, a canonical upstream model name, and a
provider label. It replaces the outbound `model` field with `grok-4.5` and
forwards the otherwise unchanged request and headers to the selected upstream.

Unknown aliases return a client error without forwarding. Malformed request
JSON continues to return a bad-request response. Upstream failures remain
transparent to Grok CLI as today.

For JSON and terminal SSE responses, usage records retain the canonical model
`grok-4.5` and use the resolved provider label. Records remain `source =
"grok-cli"`; prompts, completions, headers, and credentials are not logged.

## Dashboard Behaviour

New records appear as separate vendors (`yai-router` and `xai-official`) in
aggregate charts and filters. Existing historical `provider = "xai"` records
are not rewritten. The vendor merge configuration must not merge either new
provider label into another vendor.

Existing Grok CLI pricing for `grok-4.5` remains applicable because the logged
model name stays canonical. Detailed-request exclusion for `grok-cli` is
unchanged.

## Verification

- Unit-test alias resolution, outbound model rewriting, and authorization-header
  preservation for both routes.
- Unit-test unknown aliases are rejected and never sent upstream.
- Unit-test JSON and SSE usage records carry the route's provider label and
  canonical model.
- Validate the two Grok CLI model entries with a syntax check, without exposing
  either key.
- Run the relevant Rust test suite and confirm existing Grok usage loading and
  aggregation still pass.
