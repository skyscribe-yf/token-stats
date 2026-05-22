# Agent Context: Token Stats Dashboard

This file provides essential context for AI coding agents working on this project.

## Project Overview

A web dashboard for monitoring AI token usage across multiple providers/tools. It aggregates data from pi (this coding agent), Claude Code, Codex, and Kimi CLI into a unified analytics view with charts, tables, and filtering.

**Tech Stack:** Rust (Axum) backend + React 19 + Tailwind CSS v4 + Recharts frontend. Served behind nginx reverse proxy at `/token-stats/`.

---

## Architecture

```
Browser → nginx:80 → Rust Axum API (:3000) + static files
                    ↑
              Reads from multiple data sources
```

### Backend (`backend/`)
- **Axum** web framework with CORS enabled
- In-memory data store (`Arc<AppState>` with `RwLock<Vec<TokenRecord>>`)
- Background refresh task every 30s (`REFRESH_INTERVAL_SECS`)
- Serves static files from `backend/static/` (built frontend)
- Parses data from 5 sources:
  1. **Pi**: `~/.pi/token-logs/usage.jsonl` (JSONL)
  2. **Codex**: `~/.codex/sessions/*/rollout-*.jsonl` (JSONL) — direct from Codex CLI
  3. **Claude Code**: `~/.claude/projects/*/*.jsonl` (JSONL) — direct from Claude Code CLI
  4. **OpenCode**: `~/.local/share/opencode/opencode.db` (SQLite) — direct from OpenCode CLI
  5. **Kimi CLI**: `~/.kimi/sessions/*/wire.jsonl` (JSONL)
  - **ccswitch fallback**: `~/.cc-switch/cc-switch.db` (SQLite) — only loaded if `USE_CC_SWITCH` env var is set

### Quota Data Sources
- **OpenCode-go subscription**: Fetched directly via HTTP to the workspace dashboard (`https://opencode.ai/workspace/{id}/go`) using `reqwest` + `scraper` for HTML parsing. Reads `OPENCODE_GO_WORKSPACE_ID` and `OPENCODE_GO_AUTH_COOKIE` from environment variables. Extracts Rolling/Weekly/Monthly usage percentages and reset timers from the `<div data-slot="usage">` element.

### Frontend (`frontend/`)
- Vite + React 19 + TypeScript
- Tailwind CSS v4 (via `@tailwindcss/vite` plugin)
- Recharts for charts
- Lucide React for icons
- Built output goes to `../backend/static`
- Base path: `/token-stats/`

---

## Key Files & Responsibilities

### Backend

| File | Purpose |
|------|---------|
| `src/main.rs` | Server init, background refresh, routing, CORS |
| `src/config.rs` | Vendor merge config loading and application |
| `src/models.rs` | All data structs: `TokenRecord`, `StatsResponse`, `AggregatedStats`, etc. |
| `src/parser.rs` | Parse all 3 data sources (JSONL + SQLite) |
| `src/aggregator.rs` | Filter, aggregate, sort, paginate records |
| `src/routes.rs` | Axum handlers: `/api/stats`, `/api/requests`, `/api/filters`, `/api/pricing` |
| `src/pricing.rs` | Real-time cost calculation: model prices, USD→CNY conversion, special rules (xunfei per-call, kimi per-token, opencode /6) |

### Frontend

| File | Purpose |
|------|---------|
| `src/App.tsx` | Single-page dashboard (no router), all UI components inline |
| `src/api.ts` | API client + TypeScript interfaces matching backend |
| `src/lib/utils.ts` | Formatting helpers, date utilities, source color mapping |

---

## API Endpoints

All endpoints accept `tz_offset` (minutes from UTC, e.g. `480` for UTC+8).

| Endpoint | Description |
|----------|-------------|
| `GET /api/stats?from=&to=&source=&provider=&tz_offset=` | Full aggregation: overall + by_vendor + by_date + by_model + by_source |
| `GET /api/requests?from=&to=&provider=&model=&source=&page=&limit=&tz_offset=` | Paginated raw requests, sorted desc by time |
| `GET /api/filters` | Available vendors, models, sources |
| `GET /api/pricing` | Current pricing config (models, exchange rate, special rules) |
| `POST /api/pricing/reload` | Reload `pricing.toml` without restarting |

### Time Bound Formats
`from`/`to` accept:
- Date: `2025-05-17`
- DateTime: `2025-05-17T14:30` or `2025-05-17T14:30:00`

### Filtering Behavior
- `source` and `provider` accept comma-separated values for multi-select
- Empty string or omitted = no filter
- Frontend sends empty string when "all" selected

---

## Data Model

### `TokenRecord` (core)
```rust
date: String,              // "2025-05-17"
time: String,              // RFC3339 UTC
api_key_prefix: String,
provider: String,          // e.g. "openai", "anthropic", "deepseek"
model: String,             // e.g. "gpt-5.5", "claude-sonnet-4"
source: String,            // "pi" | "claude-code" | "codex" | "kimi-cli"
input_tokens: i64,
output_tokens: i64,
cache_read_tokens: i64,
cache_write_tokens: i64,
total_tokens: i64,
cost: f64,
```

### Cache Hit Ratio Formula
```
cache_hit_ratio = cache_read_tokens / (input_tokens + cache_read_tokens) × 100%
```
- `input_tokens` = non-cached input ONLY (normalized)
- `total_tokens` = input + output + cache_read + cache_write

**Important normalization:**
- **Codex (OpenAI API)**: `input_tokens` INCLUDES `cache_read_tokens` in the raw data. The parser subtracts: `effective_input = input_tokens - cache_read_tokens`.
- **Claude Code (Anthropic API)**: `input_tokens` already excludes cache tokens. No normalization needed.
- **Kimi CLI**: `input_tokens` already excludes cache tokens. No normalization needed.
- This ensures consistent cache hit ratio calculation across all sources.

### Vendor Merging

Different data sources may use different provider names for the same vendor (e.g. "kimi" from Kimi CLI vs "kimi-coding" from ccswitch). The `vendor_merge.toml` config file defines merge rules to unify these into canonical names.

**Config file**: `backend/vendor_merge.toml` (auto-detected next to the binary, or override via `VENDOR_MERGE_CONFIG` env var)

```toml
[[vendor_group]]
name = "kimi"
providers = ["kimi", "kimi-coding"]

[[vendor_group]]
name = "ainaba"
providers = ["openai", "ainaiba"]
```

- Each `[[vendor_group]]` has a `name` (the canonical provider name) and `providers` (all original names that should map to it).
- Merging is applied in `load_all_sources()` after all data is loaded, before records are stored.
- If the config file is missing, no merging occurs (graceful degradation).
- The `config.rs` module handles loading and applying the merge map.

---

## Design Decisions & Conventions

### Backend
1. **No database** — All data is ephemeral, read from files/SQLite on startup and refreshed periodically.
2. **In-memory store** — `RwLock<Vec<TokenRecord>>` is simple and sufficient (datasets are small, thousands of records).
3. **Graceful degradation** — If a data source is missing, parsing continues with available sources (warnings logged).
4. **UTC everywhere internally** — Times stored as RFC3339 UTC. Local timezone only applied at aggregation/display time via `tz_offset`.
5. **No auth** — Local dashboard, no authentication.

### Frontend
1. **Single-file app** — `App.tsx` is large (~900 lines) by design; all components are inline closures.
2. **Chinese UI labels** — Dashboard uses Chinese text (`ZH` constant object). Keep new UI text in Chinese.
3. **Source-aware colors** — Each tool source has a fixed color in `SOURCE_COLORS`. Extend this when adding new sources.
4. **Cost display** — All costs are displayed in **CNY (¥)**. Backend `TokenRecord.cost` keeps the *original* unit. Currency varies by Pi provider config in `models.json`:
   - `deepseek` Pi provider: **CNY** (official DeepSeek API prices in yuan)
   - All other Pi providers (ainaiba, opencode-go, guancha, xiaomi-mimo, etc.): **USD**
   - OpenCode DB records (`source="opencode"`): **USD** (from OpenCode API)
   - Codex/Claude-code: no stored cost (`cost=0`), computed from tokens
   The `pricing::display_cost()` function converts everything to CNY on-the-fly using `pricing.toml`, applying provider-specific discounts. Non-pi sources with zero computed cost show "N/A".
5. **Preset time ranges** — Today, 6h, 12h, 1d, 3d, 7d, 14d, 30d, all, custom.

---

## Adding a New Data Source

To add a new tool's token data:

1. **`parser.rs`**: Add a new `parse_*` function + `get_*_path()` helper
   - Return `Vec<TokenRecord>`
   - Normalize cache semantics to match the Anthropic convention
   - Set `source` field to identify the tool
   - Handle missing files gracefully (return empty vec)

2. **`load_all_sources()`**: Call your new parser and `extend` into `all_records`

3. **`utils.ts`** (frontend): Add source to `SOURCE_COLORS` and `SOURCE_LABELS`

4. **Test** by checking the dashboard loads the new data

---

## Adding New API Endpoints

1. Define response model in `models.rs`
2. Add aggregation logic in `aggregator.rs` (if needed)
3. Add handler in `routes.rs` with `Query<YourQuery>` struct
4. Register route in `main.rs`
5. Add TypeScript interface + fetch function in `frontend/src/api.ts`
6. Use in `App.tsx`

---

## Build & Development

```bash
# Quick dev (backend only, frontend pre-built)
./start.sh

# Full setup (nginx + systemd)
./setup.sh

# Zero-downtime deploy (builds, then blue-green swaps ports)
./deploy.sh

# Manual build
(cd backend && cargo build --release)
(cd frontend && npm install && npm run build)  # outputs to ../backend/static

# Run backend directly
cd backend && RUST_LOG=info ./target/release/token-stats-backend
```

### Environment Variables
| Variable | Default | Description |
|----------|---------|-------------|
| `PORT` | `3000` | Backend port |
| `RUST_LOG` | - | Logging level (`info`, `debug`, `trace`) |
| `REFRESH_INTERVAL_SECS` | `30` | Data refresh interval |
| `CCSWITCH_DB_PATH` | `~/.cc-switch/cc-switch.db` | Override ccswitch DB location |
| `USE_CC_SWITCH` | unset | Set to any value to also load legacy cc-switch SQLite data |
| `KIMI_SESSIONS_PATH` | `~/.kimi/sessions` | Override Kimi sessions directory |
| `VENDOR_MERGE_CONFIG` | auto-detect | Override vendor merge config path (see below) |
| `OPENCODE_GO_WORKSPACE_ID` | unset | OpenCode-go workspace ID (required for quota display) |
| `OPENCODE_GO_AUTH_COOKIE` | unset | OpenCode-go `auth` cookie value (required for quota display) |
| `XAI_API_KEY` | unset | Ainaiba/XAI API key for credit balance display (Bearer token passed to `api-xai.ainaibahub.com`) |

---

## Common Tasks

### "Add a new chart"
- Compute data in `aggregator.rs` if backend aggregation needed
- Or transform existing `stats` data in `App.tsx` via `useMemo`
- Use Recharts components (`BarChart`, `LineChart`, `PieChart`, etc.)
- Wrap in `<ResponsiveContainer width="100%" height={...}>`
- Use `CustomTooltip` or create a new tooltip component

### "Add a new filter"
- Add query param to `StatsQuery` or `RequestsQuery` in `routes.rs`
- Apply filtering in `aggregator.rs` (look at existing `filter_records`)
- Add UI control in `App.tsx` header section
- Pass through `api.ts` fetch function

### "Fix timezone issues"
- Backend: `tz_offset` converts to `FixedOffset`, then `local_date_for_record()` converts UTC times to local dates
- Frontend: `getTimezoneOffset()` returns negative minutes from UTC (e.g. UTC+8 = `-480`, so `tzOffset = -new Date().getTimezoneOffset()` = `480`)
- Date-only bounds include the entire day (upper bound is inclusive for dates)

### "Style changes"
- Tailwind v4 classes available
- Custom `primary` color configured in `index.css` via `@theme`
- Card pattern: `bg-white rounded-xl border border-slate-200 p-5 shadow-sm`
- Badge colors: `bg-emerald-100 text-emerald-700`, `bg-amber-100 text-amber-700`, `bg-slate-100 text-slate-600`

---

## Gotchas

1. **Frontend builds into backend directory** — `vite.config.ts` sets `outDir: ../backend/static`. Don't manually create `backend/static/`.
2. **Base path** — Frontend runs at `/token-stats/`, not root. API calls use `/token-stats/api/*`. Vite `base: "/token-stats/"` handles this.
3. **nginx strips prefix** — `location /token-stats/` proxies to backend at `/` (trailing slash matters).
4. **SQLite read-only** — ccswitch DB opened with `SQLITE_OPEN_READ_ONLY`. Never write to it.
5. **Kimi CLI cost is estimated** — Kimi CLI doesn't report cost natively. Backend estimates it as `total_tokens * (199元 / 2.8B tokens)` based on the subscription price.

6. **Pricing configuration** — `backend/pricing.toml` controls model prices, USD→CNY exchange rate, and special billing rules. Run `./scripts/reload-pricing.sh` to apply changes without restart.
7. **Zero-downtime deployment** — `deploy.sh` uses a blue-green pattern: it builds while the old instance is still running, starts a new instance on the alternate port (3000 ↔ 3001), health-checks it, switches nginx upstream, then gracefully drains and stops the old instance. Legacy `token-stats.service` is automatically migrated to `token-stats@.service` on first deploy.
8. **Date vs DateTime bounds** — Date-only upper bound (`to=2025-05-17`) includes the entire day. DateTime upper bound is exclusive-ish (compares naive UTC datetime).
9. **Sort stability** — Requests sorted by time DESC, then source ASC, provider ASC, model ASC.
