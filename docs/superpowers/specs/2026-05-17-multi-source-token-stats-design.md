# Multi-Source Token Stats Design

## Goal

Extend token-stats to aggregate token usage from multiple AI coding tools (pi, Claude Code via ccswitch, Kimi CLI) with a source filter for easy comparison.

## Data Sources

| Source | Location | Format | Token Fields | Cost | Records |
|--------|----------|--------|-------------|------|---------|
| **pi** | `~/.pi/token-logs/usage.jsonl` | JSONL per request | inputTokens, outputTokens, cacheReadTokens, cacheWriteTokens | ✅ included | ~varies |
| **Claude Code** | `~/.cc-switch/cc-switch.db` → `proxy_request_logs` | SQLite | input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens | ✅ calculated by ccswitch | ~2981 (session_log) |
| **Codex** | `~/.cc-switch/cc-switch.db` → `proxy_request_logs` | SQLite (data_source=codex_session) | same as above | ✅ calculated by ccswitch | ~1259 |
| **Kimi CLI** | `~/.kimi/sessions/**/wire.jsonl` | JSONL (StatusUpdate messages) | input_other, output, input_cache_read, input_cache_creation | ❌ N/A | ~1764 |

## Architecture

### 1. Unified Data Model

Add `source` field to `TokenRecord`:

```rust
pub struct TokenRecord {
    pub date: String,           // YYYY-MM-DD
    pub time: String,           // ISO 8601 timestamp
    pub api_key_prefix: String, // "N/A" for non-pi sources
    pub provider: String,       // actual provider name
    pub model: String,          // actual model name
    pub source: String,         // "pi" | "claude-code" | "codex" | "kimi-cli"
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub total_tokens: i64,
    pub cost: f64,              // 0.0 for kimi-cli → frontend shows "N/A"
}
```

### 2. Backend: Multi-Source Parser

```
parser.rs
├── parse_pi_logs()           → existing JSONL parser (source="pi")
├── parse_ccswitch_db()       → SQLite reader for Claude Code + Codex (source="claude-code"|"codex")
├── parse_kimi_sessions()     → wire.jsonl parser for Kimi CLI (source="kimi-cli")
└── load_all_sources()        → merge + dedup all sources
```

#### 2.1 ccswitch DB Parser (`parse_ccswitch_db`)

- Open `~/.cc-switch/cc-switch.db` (read-only)
- Query `proxy_request_logs` table
- Map fields:
  - `model` → `provider` (use provider name from `providers` table via JOIN)
  - `request_model` → `model`
  - `input_tokens` → `input_tokens`
  - `output_tokens` → `output_tokens`
  - `cache_read_tokens` → `cache_read_tokens`
  - `cache_creation_tokens` → `cache_write_tokens`
  - `total_cost_usd` → `cost` (parse from TEXT to f64)
  - `created_at` → `date` + `time` (Unix timestamp → ISO 8601)
  - `data_source` → `source` ("session_log" → "claude-code", "codex_session" → "codex")
- JOIN with `providers` table to get human-readable provider name
- Dedup key: `request_id`

#### 2.2 Kimi CLI Parser (`parse_kimi_sessions`)

- Scan `~/.kimi/sessions/**/wire.jsonl`
- Extract messages where `message.type == "StatusUpdate"` and `payload.token_usage` exists
- Map fields:
  - `token_usage.input_other` → `input_tokens`
  - `token_usage.output` → `output_tokens`
  - `token_usage.input_cache_read` → `cache_read_tokens`
  - `token_usage.input_cache_creation` → `cache_write_tokens`
  - `cost` = 0.0 (no cost data available)
  - `provider` = "kimi" (from session config or hardcoded)
  - `model` = "kimi-for-coding" (from session config or nearest model hint)
  - `source` = "kimi-cli"
  - `timestamp` (Unix epoch float) → `date` + `time`
- Dedup key: `(file_path, timestamp)`

#### 2.3 Incremental Refresh

- **pi**: Check `mtime` of `usage.jsonl`, re-parse if changed (existing behavior)
- **ccswitch**: Check `mtime` of `cc-switch.db`, re-query if changed. Track `max(created_at)` from last load, only query new records
- **kimi-cli**: Check `mtime` of each `wire.jsonl`, only re-parse changed files. Track `(file_path, last_line_offset)` per file
- Refresh interval: **30 seconds** (configurable via env var `REFRESH_INTERVAL_SECS`)

### 3. API Changes

#### 3.1 New `source` query parameter

```
GET /api/stats?from=&to=&source=pi              # pi only
GET /api/stats?from=&to=&source=claude-code     # Claude Code only
GET /api/stats?from=&to=&source=                # all sources (default)
GET /api/requests?from=&to=&source=kimi-cli     # Kimi CLI requests
```

#### 3.2 Extended FilterOptions

```rust
pub struct FilterOptions {
    pub vendors: Vec<String>,
    pub models: Vec<String>,
    pub sources: Vec<String>,  // NEW: ["pi", "claude-code", "codex", "kimi-cli"]
}
```

#### 3.3 StatsResponse Enhancement

Add `by_source` aggregation:

```rust
pub struct StatsResponse {
    pub overall: AggregatedStats,
    pub by_vendor: Vec<VendorStats>,
    pub by_date: Vec<DateStats>,
    pub by_model: Vec<ModelStats>,
    pub by_source: Vec<SourceStats>,  // NEW
}

pub struct SourceStats {
    pub source: String,
    pub calls: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub total_tokens: i64,
    pub cost: f64,
    pub cache_hit_ratio: f64,
}
```

### 4. Frontend Changes

#### 4.1 Source Filter in Header

Add a source dropdown next to existing provider/model filters:

```
[全部工具 ▾]  [pi] [Claude Code] [Codex] [Kimi CLI]  ← pill-style toggle buttons
```

- Default: all sources selected
- Multi-select: can compare multiple sources
- When a source is toggled off, all API calls add `source=` filter

#### 4.2 Source Column in Tables

- Vendor table: add "Source" badge column (color-coded per source)
- Model table: add "Source" badge column
- Request table: add "Source" badge column

Source badge colors:
- pi: `#3b82f6` (blue)
- claude-code: `#f59e0b` (amber)
- codex: `#10b981` (emerald)
- kimi-cli: `#8b5cf6` (violet)

#### 4.3 Cost Display

- When `cost == 0.0` and `source != "pi"`: display "N/A" instead of "$0.00"
- When `cost > 0`: display normally

#### 4.4 Source Summary Cards

Add a "Source Overview" section with cards showing per-source totals:
- Total calls, tokens, cost per source
- Visual comparison via small bar chart

### 5. Deduplication Strategy

- **pi**: No dedup needed (append-only JSONL)
- **ccswitch**: Use `request_id` as unique key (already unique in DB)
- **kimi-cli**: Use `(wire.jsonl file path, message timestamp)` as composite key
- In-memory: `HashSet<String>` of seen keys per source

### 6. Error Handling

- If `cc-switch.db` doesn't exist: skip Claude Code/Codex source, log warning
- If `~/.kimi/sessions/` doesn't exist: skip Kimi CLI source, log warning
- If `usage.jsonl` doesn't exist: skip pi source (existing behavior)
- Partial failures don't block other sources

### 7. Configuration

Environment variables:

| Variable | Default | Description |
|----------|---------|-------------|
| `PORT` | `3000` | Backend server port |
| `REFRESH_INTERVAL_SECS` | `30` | Data refresh interval |
| `PI_LOG_PATH` | `~/.pi/token-logs/usage.jsonl` | pi data file path |
| `CCSWITCH_DB_PATH` | `~/.cc-switch/cc-switch.db` | ccswitch DB path |
| `KIMI_SESSIONS_PATH` | `~/.kimi/sessions` | Kimi CLI sessions dir |

### 8. Implementation Order

1. Add `source` field to `TokenRecord` + `SourceStats` to models
2. Implement `parse_ccswitch_db()` in parser.rs (add rusqlite dependency)
3. Implement `parse_kimi_sessions()` in parser.rs
4. Wire up `load_all_sources()` with incremental refresh in main.rs
5. Add `source` query param to routes + `by_source` aggregation
6. Update `FilterOptions` with `sources` field
7. Frontend: add source filter UI + source badges in tables
8. Frontend: handle "N/A" cost display
9. Frontend: source summary section