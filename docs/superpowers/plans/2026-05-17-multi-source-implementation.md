# Implementation Plan: Multi-Source Token Stats

**Spec**: `docs/superpowers/specs/2026-05-17-multi-source-token-stats-design.md`

## Steps

### Step 1: Backend — Add `source` field to models + new structs
- Add `source: String` to `TokenRecord`
- Add `SourceStats` struct
- Add `by_source` to `StatsResponse`
- Add `sources` to `FilterOptions`
- **Verify**: `cargo check` passes

### Step 2: Backend — Implement `parse_ccswitch_db()`
- Add `rusqlite` dependency to Cargo.toml
- Implement `parse_ccswitch_db()` in parser.rs
- JOIN with providers table for human-readable provider name
- Map data_source → source ("session_log" → "claude-code", "codex_session" → "codex")
- **Verify**: unit test with real DB reads records correctly

### Step 3: Backend — Implement `parse_kimi_sessions()`
- Implement `parse_kimi_sessions()` in parser.rs
- Parse wire.jsonl StatusUpdate messages
- Resolve model from session state.json or StepBegin
- **Verify**: unit test reads records from real data

### Step 4: Backend — Wire up `load_all_sources()` + incremental refresh
- Replace single `parse_jsonl_file()` call with `load_all_sources()`
- Implement mtime-based incremental refresh (30s interval)
- Add source filter query param to routes
- Add `by_source` aggregation
- Update `FilterOptions` endpoint
- **Verify**: `cargo build` + manual API test

### Step 5: Frontend — Add source filter UI + source badges
- Add source toggle buttons in header
- Add source badge column to tables
- Handle "N/A" cost display
- Add source summary section
- **Verify**: `npm run build` + visual check

### Step 6: End-to-end verification
- Start backend + frontend
- Verify all sources appear in filter
- Verify data from pi, claude-code, codex, kimi-cli
- Verify source filtering works
- **Verify**: full manual test
