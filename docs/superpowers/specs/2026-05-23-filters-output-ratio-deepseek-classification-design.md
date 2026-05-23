# Sidebar filters, output-ratio column, and DeepSeek-export classification fix

**Date:** 2026-05-23
**Status:** Design

## Problem

Four issues spotted while reviewing the dashboard:

1. The sidebar's model picker is a single-select dropdown — the dropdown clips long names and there's no quick way to multi-select common groups (e.g. all advanced models).
2. The "供应商" (vendor) and "工具" (tool) sidebar lists have no select-all / clear-all shortcuts. Only the subscription sub-group has them.
3. `deepseek-v4-pro` average cost differs implausibly between sources:
   - `claude-code/deepseek-v4-pro`: 0.57 CNY/call (high)
   - `opencode/deepseek/deepseek-v4-pro`: 0.01 CNY/call (low)
   The user suspects this is partly because output-token ratio differs, but the dashboard has no way to see this.
4. 10 records in `usage.jsonl` show `source=opencode, provider=opencode-go, model=deepseek-v4-pro, cost=0` totalling 202.45M tokens, all `apiKeyPrefix=deepseek-export:opencode`. They came from `scripts/recover_token_data.py` (DeepSeek platform CSV export) and were misclassified — they should be `provider=deepseek, source=deepseek-ai` like other DeepSeek-export records.

## Goals

- One unified multi-select model picker in the sidebar with select-all / clear / advanced-preset shortcuts.
- Select-all / clear shortcuts on both vendor and tool sidebar lists.
- Make output-token share visible in the pivot table so users can spot why avg costs differ.
- Fix the recovery script's mapping AND migrate the 10 already-wrong records in place.

Non-goals: redesigning the dashboard layout, changing other filter semantics, touching cost calculation for non-DeepSeek-export records.

## Design

### 1. Multi-select model picker (sidebar)

**File:** `frontend/src/components/SidebarModelPicker.tsx`

Replace the dropdown UI with a scrollable checkbox list (visually mirrors `SidebarVendorList`):

```
模型              [全选] [高级] [清除]
☑ claude-opus-4-7
☑ claude-sonnet-4-6
☐ deepseek-v4-pro
☑ deepseek-v4-flash
…
☑ 过滤免费
```

**State unification:**
- Drop `selectedModel: string` from `App.tsx`. The existing multi-select state `selectedPivotModels: Set<string>` becomes the single source of truth, owned by `App.tsx` and passed into both `SidebarModelPicker` and `RequestsSection`.
- The pivot table's "模型筛选" dropdown stays — it keeps the advanced-models editor (a power-user feature) and reads/writes the same Set, so selecting in either place is reflected in both.
- The detailed-requests filter (`effectiveModel`) currently uses single-select. Switch it to use the Set: if `selectedPivotModels` is empty, no filter; otherwise pass a comma-separated list (mapped through `getOriginalModels`).

**Buttons:**
- 全选: `setSelectedPivotModels(new Set(filteredModels))` — the visible (filtered) model list.
- 清除: `setSelectedPivotModels(new Set())`.
- 高级: `setSelectedPivotModels(new Set(advancedModels.filter(m => availableModels.has(m))))`.

The advanced preset already exists (`fetchAdvancedModels`, configurable in the pivot dropdown's advanced settings editor).

**Why:** Matches the user's preferred checkbox-list mockup. Sharing state avoids two filter widgets diverging.

### 2. Select-all / clear-all on vendor and tool groups

**Files:** `frontend/src/components/SidebarVendorList.tsx`, `frontend/src/components/SidebarSourceList.tsx`

Add a header row matching the existing subscription-group pattern:

```
供应商              [全选] [清除]
```

For the vendor list, the existing subscription-group buttons stay; the new buttons cover the "regular" group (and toggle behavior just operates on its members).

For the tool list, identical pattern — single group, two buttons.

Each button updates the relevant `Set<string>` in `App.tsx` via existing callbacks. The handlers stay in `App.tsx` to keep the components dumb.

### 3. Output-ratio column

**Files:** `frontend/src/lib/pivotTable.ts`, `frontend/src/components/sections/RequestsSection.tsx`, `frontend/src/lib/utils.ts`

**Formula:** `output_tokens / total_tokens × 100%` (per user's choice).

**Computation:** Add `output_ratio` to `PivotSummary` and to source-detail sort values. `getSortValue` learns a new `output_ratio` `SortColumn` variant.

**UI:**
- New column in the pivot table titled "输出比" between the existing "缓存命中" and "费用" columns.
- Same column in the detailed-requests table.
- Format via a new `formatPercent` variant (or reuse the existing one).
- Color thresholds: ≥ 20% amber, ≤ 5% slate, otherwise neutral. Same shape as the cache-hit-ratio styling so the table reads consistently.

**Tests:** Extend `pivotTable.test.ts` with a case verifying `output_ratio` is computed from the summed `output_tokens` and `total_tokens`.

### 4. Reclassify the 10 wrong records

**File 1 — Recovery script fix:** `scripts/recover_token_data.py`

```python
API_KEY_MAP = {
    "opencode": "deepseek",       # was "opencode-go"
    "pi": "deepseek",
    "ai小北": "deepseek",
}
SOURCE_MAP = {
    "opencode": "deepseek-ai",    # was "opencode"
    "pi": "pi",
    "ai小北": "deepseek-ai",
}
```

Add a brief comment explaining the rationale (these come from DeepSeek's CSV export — the API key name labels the *channel* but the calls are billed by DeepSeek directly; classification should reflect that).

**File 2 — One-shot migration:** `scripts/migrate_deepseek_export_classification.py`

```
1. Read $HOME/.pi/token-logs/usage.jsonl line-by-line
2. For each record with apiKeyPrefix starting "deepseek-export:opencode":
     - Update provider: "opencode-go" → "deepseek"
     - Update source: "opencode" → "deepseek-ai"
     - (cost stays 0; pricing.rs computes via deepseek provider path)
3. Write to usage.jsonl.tmp, then atomically rename (usage.jsonl → usage.jsonl.bak.YYYYMMDD).
4. Print count summary.
```

Supports `--dry-run` to preview affected records without writing.

**Cost computation after migration:** The existing `display_cost()` already handles `effective_provider == "deepseek"` and returns `record.cost` as-is (CNY). Since cost stays at 0 for these export-aggregate records, the displayed cost will continue to be 0. **Note for the user:** the DeepSeek export doesn't include per-token cost data in `amount-*.csv`; for accurate cost we'd need to either compute from the deepseek-v4-pro pricing in `pricing.toml` (CNY per million tokens, already configured) or join with `cost-*.csv` from the export. Computing from `pricing.toml` is preferred because we don't need a second data join.

**Addendum to pricing.rs:** Add a fallback in `display_cost()` so DeepSeek records with `cost == 0` compute from `pricing.toml` rates (CNY, no divisor). Specifically: after step 4a fails (cost is 0), if `effective_provider == "deepseek"` and the model is in the price map, compute `input × in_rate + output × out_rate + cache_read × cr_rate + cache_write × cw_rate`. This keeps deepseek pricing in CNY (which is how `pricing.toml` lists it). Update the doc comment at the top of `pricing.rs` to reflect this.

This change ALSO benefits the existing 14 `pi/deepseek/deepseek-v4-pro` session-recovery records with cost=0.

## Files touched

```
backend/src/pricing.rs                                 — add deepseek-from-tokens path
backend/src/pricing.rs (tests)                         — add coverage
frontend/src/App.tsx                                   — drop selectedModel, unify
frontend/src/components/Sidebar.tsx                    — props update
frontend/src/components/SidebarModelPicker.tsx         — checkbox list rewrite
frontend/src/components/SidebarVendorList.tsx          — add buttons
frontend/src/components/SidebarSourceList.tsx          — add buttons
frontend/src/components/sections/RequestsSection.tsx   — output-ratio column
frontend/src/lib/pivotTable.ts                         — output_ratio in PivotSummary
frontend/src/lib/pivotTable.test.ts                    — output_ratio test
scripts/recover_token_data.py                          — fix maps
scripts/migrate_deepseek_export_classification.py      — new one-shot
```

## Testing

- `cargo test -p token-stats-backend` (pricing tests).
- `cd frontend && npm test` (pivotTable + filterState).
- `cd frontend && npx tsc --noEmit && npx eslint src` (type + lint).
- Manual: run migration with `--dry-run`, confirm 10 records flagged; run live, confirm `.bak` created; restart backend, confirm dashboard shows deepseek/deepseek-v4-pro records from deepseek-ai source with non-zero cost.
- Manual: open dashboard, exercise new sidebar multi-select, select-all/clear-all on vendors and tools, output-ratio sort.

## Rollout

- Single PR against `main`.
- Migration script runs once locally; not part of build.
- No DB schema changes; usage.jsonl format unchanged.

## Risks

- **Migration overwrites usage.jsonl.** Mitigated by `.bak.YYYYMMDD` copy + `--dry-run`.
- **Shared model state between sidebar + pivot dropdown** could surprise users (selecting in one updates the other). Mitigated by removing duplicate dropdown content — both reference the same Set, no separate "pending" state in the pivot dropdown when sidebar is already up-to-date.
- **Adding deepseek-from-tokens cost path** affects existing pi/deepseek records with cost=0. Verified there are 14 such records; their cost will go from 0 to a positive CNY value, which is a more accurate reflection (they used DeepSeek's API).
