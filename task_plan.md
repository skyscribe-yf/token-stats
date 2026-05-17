# Token Stats Dashboard - Task Plan

## Goal
Build a web dashboard for token usage analytics with Rust backend + React frontend,
deployed behind local nginx with port forwarding.

## Data Source
- `~/.pi/token-logs/usage.jsonl` — JSON Lines format
- Fields: date, time, apiKeyPrefix, provider, model, inputTokens, outputTokens, cacheReadTokens, cacheWriteTokens, totalTokens, cost

## Architecture
```
┌─────────────┐     ┌──────────────┐     ┌─────────────┐
│   Browser   │────▶│  nginx:8080  │────▶│ Rust Axum   │
│             │◄────│  (reverse    │◄────│ API :3000   │
│             │     │   proxy)     │     │             │
└─────────────┘     └──────────────┘     └─────────────┘
                           │
                           ▼
                    ┌──────────────┐
                    │  React SPA   │
                    │  (static)    │
                    └──────────────┘
```

## Phases

### Phase 1: Rust Backend (Axum)
- [ ] Create Cargo project with dependencies (axum, tokio, serde, chrono, tower-http)
- [ ] Define data models matching usage.jsonl schema
- [ ] Implement JSONL parser with streaming for large files
- [ ] Build aggregation engine (per vendor, per date, per model, per request)
- [ ] Create REST API endpoints:
  - `GET /api/stats?from=YYYY-MM-DD&to=YYYY-MM-DD` — aggregated stats
  - `GET /api/stats/by-vendor?from=&to=` — vendor breakdown
  - `GET /api/stats/by-date?from=&to=` — daily time series
  - `GET /api/requests?from=&to=&provider=&model=&page=&limit` — detailed requests
  - `GET /api/vendors` — list of vendors
  - `GET /api/models` — list of models
- [ ] Add CORS for local development
- [ ] Serve static frontend files

### Phase 2: React Frontend (Vite + Tailwind + shadcn)
- [ ] Initialize Vite + React + TypeScript project
- [ ] Configure Tailwind CSS
- [ ] Install charting library (Recharts)
- [ ] Install date picker and UI components
- [ ] Build dashboard layout with sidebar/nav
- [ ] Create summary cards (total tokens, cache hit ratio, cost, calls)
- [ ] Build vendor breakdown chart (bar/pie)
- [ ] Build daily trends chart (line chart)
- [ ] Build cache hit ratio visualization
- [ ] Build detailed requests table with pagination
- [ ] Implement date range selector
- [ ] Add provider/model filters
- [ ] Polish UI with good visual design

### Phase 3: Integration & Build
- [ ] Build frontend to static files
- [ ] Configure Rust backend to serve static files
- [ ] Create nginx config template
- [ ] Create systemd service template
- [ ] Write setup instructions

### Phase 4: Testing & Polish
- [ ] Test API endpoints
- [ ] Test frontend data fetching
- [ ] Test with real data
- [ ] Verify cache hit ratio calculations
- [ ] Final UI polish

## Cache Hit Ratio Formula
```
cache_hit_ratio = cacheReadTokens / (inputTokens + cacheReadTokens) * 100
```
(When inputTokens + cacheReadTokens > 0, else 0)

## Project Structure
```
token-stats/
├── backend/           # Rust Axum backend
│   ├── Cargo.toml
│   └── src/
│       ├── main.rs
│       ├── models.rs
│       ├── parser.rs
│       ├── aggregator.rs
│       └── routes.rs
├── frontend/          # React + Vite frontend
│   ├── package.json
│   ├── vite.config.ts
│   ├── index.html
│   └── src/
│       ├── main.tsx
│       ├── App.tsx
│       ├── components/
│       └── api.ts
├── nginx/             # Nginx configuration
│   └── token-stats.conf
└── README.md
```
