# xAI-Only Grok Network Proxy Design

## Goal

Allow the local Grok usage recorder to reach official xAI through the local
HAProxy listener without routing YAI Router traffic through that network proxy.

## Configuration

`token-stats-grok-proxy.service` receives one optional environment variable:

```text
GROK_XAI_NETWORK_PROXY=http://127.0.0.1:7800
```

HAProxy listens on `127.0.0.1:7800` and forwards to the local Xray listener on
`127.0.0.1:2081`. The service must not receive generic `HTTP_PROXY` or
`HTTPS_PROXY` variables, because those would apply to both upstreams.

## Routing Behaviour

- `grok-4.5-xai` uses a Reqwest client configured with
  `GROK_XAI_NETWORK_PROXY` and forwards to `https://api.x.ai`.
- `grok-4.5-yai` uses a direct Reqwest client and forwards to YAI Router.
- If the xAI-only variable is absent, xAI uses a direct client, preserving the
  prior default behaviour.
- Invalid proxy URLs fail safely at service start with a clear error rather
  than silently redirecting either route.

## Verification

- Unit-test that xAI routing selects the configured network proxy client.
- Unit-test that YAI routing never receives that proxy setting.
- Run the Grok proxy tests and the backend suite.
- After the user restarts the service, verify `grok-4.5-xai` can reach xAI and
  `grok-4.5-yai` remains direct.
