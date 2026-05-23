# Sidebar filters + output-ratio + DeepSeek-export classification Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix four UX/data issues at once: model multi-select with presets, select-all/clear-all on vendor + tool lists, output-ratio column in the pivot table, and reclassify 10 wrongly-labeled DeepSeek-export records.

**Architecture:**
- Backend: One pricing.rs fix adds a `deepseek+cost=0` → compute-from-tokens (CNY) path so reclassified records and existing session-recovery records show non-zero cost.
- Frontend: Sidebar gets shared multi-select model state (with `selectedPivotModels` Set already used by the pivot dropdown) and select-all/clear buttons on vendor/tool lists. Pivot + requests tables gain an `output_ratio = output / total` column.
- Data: A Python migration rewrites usage.jsonl in place (with `.bak.YYYYMMDD`), updating the 10 records to `provider=deepseek, source=deepseek-ai`. Recovery script's maps are fixed so future runs classify correctly.

**Tech Stack:** Rust (Axum, serde), React + TypeScript, Vite, Tailwind, Python 3, node:test for frontend tests, cargo test for backend.

**Branch:** `fix/sidebar-filters-output-ratio-deepseek-classification` (already checked out, design committed).

---

## File Structure

**Backend:**
- `backend/src/pricing.rs` — modify `display_cost()` to compute DeepSeek records from tokens when `cost == 0`; new tests.

**Frontend:**
- `frontend/src/lib/pivotTable.ts` — add `output_ratio` field + new SortColumn variant + `formatOutputRatio` helper. (Optional: keep helper inside pivotTable.ts since the table is the only consumer.)
- `frontend/src/lib/pivotTable.test.ts` — add a test for output_ratio computation and sort.
- `frontend/src/components/SidebarModelPicker.tsx` — rewrite as checkbox list with select-all / clear / advanced buttons.
- `frontend/src/components/SidebarVendorList.tsx` — add select-all/clear buttons to the regular vendors group header.
- `frontend/src/components/SidebarSourceList.tsx` — add select-all/clear buttons to its header.
- `frontend/src/components/Sidebar.tsx` — props update: drop single `selectedModel`/`onModelChange`, accept `selectedModels: ReadonlySet<string>` + handlers; add source-group + vendor-group toggle-all handlers.
- `frontend/src/App.tsx` — drop `selectedModel` state; route `selectedPivotModels` into sidebar + filter; add `handleVendorGroupToggle`, `handleSourceGroupToggle`.
- `frontend/src/components/sections/RequestsSection.tsx` — add 输出比 column (pivot + details tables); use shared `selectedPivotModels` Set from props.

**Scripts:**
- `scripts/recover_token_data.py` — change `API_KEY_MAP["opencode"]` to `"deepseek"` and `SOURCE_MAP["opencode"]` to `"deepseek-ai"`.
- `scripts/migrate_deepseek_export_classification.py` — new one-shot migration with `--dry-run`.

---

## Task 1: Backend — compute DeepSeek cost from tokens when stored cost is zero

**Files:**
- Modify: `backend/src/pricing.rs`

- [ ] **Step 1: Write the failing test**

Add this test at the bottom of `backend/src/pricing.rs` inside the existing `#[cfg(test)] mod tests { … }` block, just before the closing brace. The test follows the existing `freemodel_derived_cost_applies_divisor` pattern (writes a temp pricing.toml so `resolve_model_price` finds deepseek-v4-pro).

```rust
#[test]
fn deepseek_zero_cost_computes_from_tokens_in_cny() {
    use std::io::Write;
    let tmp = tempfile::NamedTempFile::new().unwrap();
    tmp.as_file()
        .write_all(
            br#"
usd_to_cny = 6.82
rate_date = "2026-05-20"

[special]
xunfei_per_call = 0.002211111111
kimi_per_token = 0.000000071071429
opencode_divisor = 6.0
ainaba_divisor = 40.0
freemodel_divisor = 68.2

[[model]]
name = "deepseek-v4-pro"
input = 0.5865
output = 2.346
cache_read = 0.05865
cache_write = 0.5865
"#,
        )
        .unwrap();

    let prev_env = std::env::var("PRICING_CONFIG").ok();
    std::env::set_var("PRICING_CONFIG", tmp.path().to_str().unwrap());
    reload();

    // DeepSeek record with cost=0 (the recovery-script case after reclassification).
    let mut record = make_record("deepseek-ai", "deepseek", "deepseek-v4-pro", 0, 0.0);
    record.input_tokens = 1_000_000;
    record.output_tokens = 100_000;
    record.cache_read_tokens = 500_000;
    record.cache_write_tokens = 0;
    record.total_tokens = 1_600_000;

    let cny = display_cost(&record);

    // pricing.toml lists deepseek-v4-pro rates in USD per million tokens
    // (input=0.5865, output=2.346, cache_read=0.05865). Convert to CNY via usd_to_cny=6.82,
    // no opencode/ainaba/freemodel divisor for source=deepseek-ai/provider=deepseek.
    let usd = 1_000_000.0 * 0.5865 / 1_000_000.0
        + 100_000.0 * 2.346 / 1_000_000.0
        + 500_000.0 * 0.05865 / 1_000_000.0;
    let expected = usd * 6.82;

    assert!(cny > 0.0, "deepseek zero-cost record should compute non-zero, got {}", cny);
    assert!(
        (cny - expected).abs() < 0.001,
        "deepseek cost mismatch: expected {}, got {}",
        expected,
        cny
    );

    // Restore env
    match prev_env {
        Some(v) => std::env::set_var("PRICING_CONFIG", v),
        None => std::env::remove_var("PRICING_CONFIG"),
    }
    reload();
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run from `backend/`:
```bash
cargo test --quiet pricing::tests::deepseek_zero_cost_computes_from_tokens_in_cny 2>&1 | tail -20
```
Expected: FAIL with assertion failure (`display_cost` returns 0).

- [ ] **Step 3: Implement the fix in `pricing.rs`**

In `backend/src/pricing.rs` find the `display_cost()` function. At the end of step 4 (the `if record.cost > 0.0 { … }` block), and BEFORE the existing step 5 (codex/claude-code derived), insert a new block:

```rust
    // 4d. DeepSeek records with cost=0 (e.g. from session recovery or DeepSeek
    //     platform CSV export). pricing.toml deepseek rates are listed as USD;
    //     multiply by usd_to_cny to display in CNY. No divisor — the user
    //     pays DeepSeek directly at official rates.
    let effective_provider = record
        .original_provider
        .as_deref()
        .unwrap_or(&record.provider);
    if effective_provider == "deepseek" && record.cost == 0.0 {
        if let Some(price) = resolve_model_price(&state, &record.model, &record.provider) {
            let usd = record.input_tokens as f64 * price.input / 1_000_000.0
                + record.output_tokens as f64 * price.output / 1_000_000.0
                + record.cache_read_tokens as f64 * price.cache_read / 1_000_000.0
                + record.cache_write_tokens as f64 * price.cache_write / 1_000_000.0;
            return usd * cfg.usd_to_cny;
        }
    }
```

The `effective_provider` extraction earlier in step 4a is inside the `if record.cost > 0.0` branch — for cost==0 records, the inner code can't reach it, so re-derive here. This insertion goes BETWEEN the closing brace of step 4's `if record.cost > 0.0` block and the `if record.source == "codex" || record.source == "claude-code"` line.

Also update the function's doc comment (just above `pub fn display_cost`):
- After the existing bullets ("OpenCode source", "DeepSeek official"), add: `/// - Records with provider=deepseek and cost=0: derived from pricing.toml deepseek rates (USD→CNY, no divisor). Covers session-recovery records and DeepSeek platform CSV export.`

- [ ] **Step 4: Run the test to verify it passes**

```bash
cargo test --quiet pricing::tests::deepseek_zero_cost_computes_from_tokens_in_cny 2>&1 | tail -5
```
Expected: `test result: ok. 1 passed`

- [ ] **Step 5: Run the full pricing test suite to verify no regression**

```bash
cargo test --quiet pricing:: 2>&1 | tail -5
```
Expected: all pricing tests pass (`test result: ok. N passed`).

- [ ] **Step 6: Commit**

```bash
git add backend/src/pricing.rs
git -c commit.gpgsign=false commit -m "fix: compute deepseek cost from tokens when stored cost is zero

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 2: Recovery script — fix DeepSeek-export classification maps

**Files:**
- Modify: `scripts/recover_token_data.py`

- [ ] **Step 1: Update API_KEY_MAP and SOURCE_MAP**

In `scripts/recover_token_data.py`, replace the existing API_KEY_MAP and SOURCE_MAP blocks (currently around lines 179-189):

```python
# Map DeepSeek export api_key_name → (provider, source).
# All DeepSeek export rows describe calls billed directly by DeepSeek's
# official platform — the api_key_name is just the channel that owned
# the key. Classify them all as provider=deepseek, source=deepseek-ai
# so the dashboard treats them uniformly and pricing.rs computes their
# cost from pricing.toml deepseek rates (CNY native, no OpenCode divisor).
API_KEY_MAP = {
    "opencode": "deepseek",
    "pi": "deepseek",
    "ai小北": "deepseek",
}

SOURCE_MAP = {
    "opencode": "deepseek-ai",
    "pi": "pi",
    "ai小北": "deepseek-ai",
}
```

- [ ] **Step 2: Smoke-test the script can still be parsed**

```bash
python3 -c "import ast; ast.parse(open('scripts/recover_token_data.py').read()); print('parse ok')"
```
Expected: `parse ok`

- [ ] **Step 3: Commit**

```bash
git add scripts/recover_token_data.py
git -c commit.gpgsign=false commit -m "fix: classify all DeepSeek-export records as provider=deepseek

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 3: One-shot migration script for the 10 already-wrong records

**Files:**
- Create: `scripts/migrate_deepseek_export_classification.py`

- [ ] **Step 1: Create the migration script**

```python
#!/usr/bin/env python3
"""
One-shot migration: reclassify legacy DeepSeek-export records.

Before this migration, scripts/recover_token_data.py mapped
api_key_name="opencode" to (provider=opencode-go, source=opencode).
Those records actually represent DeepSeek API usage (billed by
DeepSeek's platform with OpenCode's API key) and should be
(provider=deepseek, source=deepseek-ai), matching the other
DeepSeek-export records.

This script rewrites ~/.pi/token-logs/usage.jsonl in place:
  - finds records with apiKeyPrefix starting "deepseek-export:opencode"
  - sets provider="deepseek", source="deepseek-ai"
  - leaves cost=0 (display_cost computes from tokens via pricing.toml)
A backup is written to usage.jsonl.bak.YYYYMMDD before any changes.

Usage:
  python3 migrate_deepseek_export_classification.py [--dry-run]
"""

import json
import os
import shutil
import sys
from datetime import datetime

USAGE_JSONL = os.path.expanduser("~/.pi/token-logs/usage.jsonl")
TARGET_PREFIX = "deepseek-export:opencode"


def main():
    dry_run = "--dry-run" in sys.argv
    if not os.path.exists(USAGE_JSONL):
        print(f"ERROR: {USAGE_JSONL} not found")
        sys.exit(1)

    affected = []
    new_lines = []
    with open(USAGE_JSONL) as f:
        for line in f:
            stripped = line.strip()
            if not stripped:
                new_lines.append(line)
                continue
            try:
                obj = json.loads(stripped)
            except json.JSONDecodeError:
                new_lines.append(line)
                continue

            api_key_prefix = obj.get("apiKeyPrefix", "")
            if api_key_prefix.startswith(TARGET_PREFIX):
                affected.append({
                    "date": obj.get("date"),
                    "old_provider": obj.get("provider"),
                    "old_source": obj.get("source"),
                    "total_tokens": obj.get("totalTokens"),
                })
                obj["provider"] = "deepseek"
                obj["source"] = "deepseek-ai"
                new_lines.append(json.dumps(obj, ensure_ascii=False) + "\n")
            else:
                new_lines.append(line)

    print(f"Found {len(affected)} records to reclassify:")
    total_tokens = 0
    for r in affected:
        print(
            f"  {r['date']}: {r['old_source']}/{r['old_provider']} "
            f"-> deepseek-ai/deepseek ({r['total_tokens']:,} tokens)"
        )
        total_tokens += r["total_tokens"] or 0
    print(f"Total tokens to be reclassified: {total_tokens:,}")

    if not affected:
        print("Nothing to migrate.")
        return

    if dry_run:
        print("DRY RUN — no changes made. Run without --dry-run to apply.")
        return

    backup_path = f"{USAGE_JSONL}.bak.{datetime.now():%Y%m%d}"
    shutil.copyfile(USAGE_JSONL, backup_path)
    print(f"Backup written to {backup_path}")

    tmp_path = f"{USAGE_JSONL}.tmp"
    with open(tmp_path, "w") as f:
        f.writelines(new_lines)
    os.rename(tmp_path, USAGE_JSONL)
    print(f"Updated {USAGE_JSONL} in place.")


if __name__ == "__main__":
    main()
```

- [ ] **Step 2: Make it executable and parse-check**

```bash
chmod +x scripts/migrate_deepseek_export_classification.py
python3 -c "import ast; ast.parse(open('scripts/migrate_deepseek_export_classification.py').read()); print('parse ok')"
```
Expected: `parse ok`

- [ ] **Step 3: Dry-run to verify it finds the 10 records**

```bash
python3 scripts/migrate_deepseek_export_classification.py --dry-run
```
Expected: prints 10 records (2026-04-27 through 2026-05-09), total tokens 202,454,457, "DRY RUN — no changes made."

If the count is not 10 or total tokens differ significantly, stop and investigate before proceeding.

- [ ] **Step 4: Commit (do NOT run the live migration yet — separate step in Task 11)**

```bash
git add scripts/migrate_deepseek_export_classification.py
git -c commit.gpgsign=false commit -m "feat: add one-shot migration for legacy DeepSeek-export classification

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 4: Frontend pivotTable — add output_ratio field + test

**Files:**
- Modify: `frontend/src/lib/pivotTable.ts`
- Modify: `frontend/src/lib/pivotTable.test.ts`

- [ ] **Step 1: Write the failing test**

Append to `frontend/src/lib/pivotTable.test.ts`:

```typescript
test("buildPivotTree computes output_ratio from output_tokens / total_tokens", () => {
  const stats = [
    makeModelStats({
      provider: "deepseek",
      model: "deepseek-v4-pro",
      total_tokens: 1_000,
      output_tokens: 50,
      input_tokens: 950,
      source_details: [
        makeSourceDetail({
          source: "pi",
          total_tokens: 1_000,
          output_tokens: 50,
          input_tokens: 950,
        }),
      ],
    }),
  ];
  const tree = buildPivotTree(stats, "total_tokens", "desc", false);
  assert.equal(tree[0].models[0].summary.output_ratio, 5);
  assert.equal(tree[0].summary.output_ratio, 5);
});

test("getSortValue returns output_ratio in percent", () => {
  const summary = {
    calls: 1,
    input_tokens: 80,
    output_tokens: 20,
    cache_read_tokens: 0,
    cache_write_tokens: 0,
    total_tokens: 100,
    cost: 0,
    cache_hit_ratio: 0,
    output_ratio: 20,
  };
  assert.equal(getSortValue(summary, "output_ratio"), 20);
});

test("buildPivotTree handles zero total_tokens for output_ratio", () => {
  const stats = [
    makeModelStats({
      provider: "x",
      model: "m",
      total_tokens: 0,
      output_tokens: 0,
      input_tokens: 0,
      source_details: [
        makeSourceDetail({
          source: "pi",
          total_tokens: 0,
          output_tokens: 0,
          input_tokens: 0,
        }),
      ],
    }),
  ];
  const tree = buildPivotTree(stats, "total_tokens", "desc", false);
  assert.equal(tree[0].models[0].summary.output_ratio, 0);
});
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cd frontend && npx tsx --test src/lib/pivotTable.test.ts 2>&1 | tail -20
```
Expected: 3 new tests fail (output_ratio doesn't exist).

- [ ] **Step 3: Update PivotSummary type and SortColumn**

In `frontend/src/lib/pivotTable.ts`:

Change `export type SortColumn =` to include the new variant. Replace the existing block (lines 3-12):

```typescript
export type SortColumn =
  | "name"
  | "calls"
  | "input_tokens"
  | "output_tokens"
  | "cache"
  | "total_tokens"
  | "cache_hit_ratio"
  | "output_ratio"
  | "cost"
  | "avg_cost";
```

Replace the `PivotSummary` interface (lines 16-25):

```typescript
export interface PivotSummary {
  calls: number;
  input_tokens: number;
  output_tokens: number;
  cache_read_tokens: number;
  cache_write_tokens: number;
  total_tokens: number;
  cost: number;
  cache_hit_ratio: number;
  output_ratio: number;
}
```

Replace `emptySummary()`:

```typescript
function emptySummary(): PivotSummary {
  return {
    calls: 0,
    input_tokens: 0,
    output_tokens: 0,
    cache_read_tokens: 0,
    cache_write_tokens: 0,
    total_tokens: 0,
    cost: 0,
    cache_hit_ratio: 0,
    output_ratio: 0,
  };
}
```

Replace `finalizeSummary` (currently uses `computeCacheHitRatio`):

```typescript
function computeOutputRatio(summary: PivotSummary): number {
  return summary.total_tokens > 0
    ? (summary.output_tokens / summary.total_tokens) * 100
    : 0;
}

function finalizeSummary(summary: PivotSummary): PivotSummary {
  return {
    ...summary,
    cache_hit_ratio: computeCacheHitRatio(summary),
    output_ratio: computeOutputRatio(summary),
  };
}
```

Add a case to `getSortValue`'s switch (insert before the `cost` case):

```typescript
    case "output_ratio":
      return summary.output_ratio;
```

- [ ] **Step 4: Update sortSourceDetails for output_ratio**

In `sortSourceDetails`, the `aSummary`/`bSummary` literals must include `output_ratio` so the type checks. Compute it inline (replace the two existing literal objects):

```typescript
    const aSummary: PivotSummary = {
      calls: a.calls,
      input_tokens: a.input_tokens,
      output_tokens: a.output_tokens,
      cache_read_tokens: a.cache_read_tokens,
      cache_write_tokens: a.cache_write_tokens,
      total_tokens: a.total_tokens,
      cost: a.cost,
      cache_hit_ratio: a.cache_hit_ratio,
      output_ratio: a.total_tokens > 0 ? (a.output_tokens / a.total_tokens) * 100 : 0,
    };
    const bSummary: PivotSummary = {
      calls: b.calls,
      input_tokens: b.input_tokens,
      output_tokens: b.output_tokens,
      cache_read_tokens: b.cache_read_tokens,
      cache_write_tokens: b.cache_write_tokens,
      total_tokens: b.total_tokens,
      cost: b.cost,
      cache_hit_ratio: b.cache_hit_ratio,
      output_ratio: b.total_tokens > 0 ? (b.output_tokens / b.total_tokens) * 100 : 0,
    };
```

- [ ] **Step 5: Fix existing getSortValue test that uses an inline summary literal**

In `pivotTable.test.ts`, the existing `getSortValue returns correct values for all columns` test creates a summary literal without `output_ratio`. Add `output_ratio: 0` to that literal so it satisfies the new type.

- [ ] **Step 6: Run tests to verify all pass**

```bash
cd frontend && npx tsx --test src/lib/pivotTable.test.ts 2>&1 | tail -10
```
Expected: all tests pass.

- [ ] **Step 7: Commit**

```bash
git add frontend/src/lib/pivotTable.ts frontend/src/lib/pivotTable.test.ts
git -c commit.gpgsign=false commit -m "feat: add output_ratio to pivot table summary

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 5: RequestsSection — add 输出比 column to pivot + details tables

**Files:**
- Modify: `frontend/src/components/sections/RequestsSection.tsx`

- [ ] **Step 1: Add the column header in the pivot table**

In `RequestsSection.tsx`, find the `<thead>` of the first table (the pivot table). Insert a new `<th>` between the existing `缓存命中` (sort column `"cache_hit_ratio"`) and `费用` (sort column `"cost"`) blocks:

```tsx
                <th
                  className="px-3 py-2 text-right font-medium cursor-pointer hover:bg-slate-100 transition-colors select-none"
                  onClick={() => handleSort("output_ratio")}
                >
                  输出比{sortIndicator("output_ratio")}
                </th>
```

- [ ] **Step 2: Add the data cells for vendor rows, model rows, source rows, and pivot summary**

In each of the four `<tr>` blocks (vendor row, model row, source row, pivot summary footer), insert a new `<td>` between the cache-hit-ratio `<td>` and the cost `<td>`.

Vendor row (search for `vendorSummary.cache_hit_ratio` and follow to the cost cell — insert before that):
```tsx
                      <td className="px-3 py-2 text-right">
                        <span
                          className={`px-1.5 py-0.5 rounded-full text-[10px] font-medium ${
                            vendorSummary.output_ratio > 20
                              ? "bg-amber-100 text-amber-700"
                              : vendorSummary.output_ratio < 5
                                ? "bg-slate-100 text-slate-500"
                                : "bg-slate-100 text-slate-600"
                          }`}
                        >
                          {formatPercent(vendorSummary.output_ratio)}
                        </span>
                      </td>
```

Model row — same pattern but reading `ms.output_ratio`:
```tsx
                              <td className="px-3 py-2 text-right">
                                <span
                                  className={`px-1.5 py-0.5 rounded-full text-[10px] font-medium ${
                                    ms.output_ratio > 20
                                      ? "bg-amber-100 text-amber-700"
                                      : ms.output_ratio < 5
                                        ? "bg-slate-100 text-slate-500"
                                        : "bg-slate-100 text-slate-600"
                                  }`}
                                >
                                  {formatPercent(ms.output_ratio)}
                                </span>
                              </td>
```

Note: `ms` is shorthand for `model.summary` (already declared as `const ms = model.summary;` further up in the model row block). The `output_ratio` field will now exist on the summary because Task 4 added it to `PivotSummary` and `finalizeSummary` computes it.

Source row (uses `source` object which is a `SourceDetailStats`, which does NOT have `output_ratio` — compute inline):
```tsx
                                  <td className="px-3 py-2 text-right">
                                    {(() => {
                                      const ratio = source.total_tokens > 0
                                        ? (source.output_tokens / source.total_tokens) * 100
                                        : 0;
                                      return (
                                        <span
                                          className={`px-1.5 py-0.5 rounded-full text-[10px] font-medium ${
                                            ratio > 20
                                              ? "bg-amber-100 text-amber-700"
                                              : ratio < 5
                                                ? "bg-slate-100 text-slate-500"
                                                : "bg-slate-100 text-slate-600"
                                          }`}
                                        >
                                          {formatPercent(ratio)}
                                        </span>
                                      );
                                    })()}
                                  </td>
```

Pivot summary footer (uses `pivotSummary.output_ratio`):
```tsx
                  <td className="px-3 py-2 text-right">
                    <span
                      className={`px-1.5 py-0.5 rounded-full text-[10px] font-bold ${
                        pivotSummary.output_ratio > 20
                          ? "bg-amber-100 text-amber-700"
                          : pivotSummary.output_ratio < 5
                            ? "bg-slate-100 text-slate-500"
                            : "bg-slate-100 text-slate-600"
                      }`}
                    >
                      {formatPercent(pivotSummary.output_ratio)}
                    </span>
                  </td>
```

- [ ] **Step 3: Add the column to the detailed requests table**

The detailed-requests table is the second `<table>` block (under `<details>`). Add column header between 缓存命中 and 费用:

```tsx
                <th className="px-3 py-2 text-right font-medium">输出比</th>
```

And a body `<td>` (inside the `requests?.data.map((r, i) => …)` block) between cache_hit_ratio cell and cost cell:

```tsx
                  <td className="px-3 py-2 text-right">
                    {(() => {
                      const ratio = r.total_tokens > 0
                        ? (r.output_tokens / r.total_tokens) * 100
                        : 0;
                      return (
                        <span
                          className={`px-1.5 py-0.5 rounded-full text-[10px] font-medium ${
                            ratio > 20
                              ? "bg-amber-100 text-amber-700"
                              : ratio < 5
                                ? "bg-slate-100 text-slate-500"
                                : "bg-slate-100 text-slate-600"
                          }`}
                        >
                          {formatPercent(ratio)}
                        </span>
                      );
                    })()}
                  </td>
```

- [ ] **Step 4: Type-check the frontend**

```bash
cd frontend && npx tsc --noEmit 2>&1 | tail -20
```
Expected: no errors.

- [ ] **Step 5: Commit**

```bash
git add frontend/src/components/sections/RequestsSection.tsx
git -c commit.gpgsign=false commit -m "feat: add output ratio column to pivot + details tables

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 6: Sidebar tool list — add select-all / clear-all buttons

**Files:**
- Modify: `frontend/src/components/SidebarSourceList.tsx`

- [ ] **Step 1: Add the toggle-all prop and buttons**

Replace the entire content of `frontend/src/components/SidebarSourceList.tsx`:

```typescript
import { getSourceColor, getSourceLabel } from "../lib/utils";

interface SidebarSourceListProps {
  sources: string[];
  selectedSources: ReadonlySet<string>;
  onToggle: (source: string) => void;
  onToggleAll: (selectAll: boolean) => void;
}

export function SidebarSourceList({
  sources,
  selectedSources,
  onToggle,
  onToggleAll,
}: SidebarSourceListProps) {
  if (sources.length === 0) return null;
  const allSelected = sources.every((s) => selectedSources.has(s));

  return (
    <div className="py-3 px-3">
      <div className="px-2 mb-1.5 flex items-center justify-between">
        <p className="text-[10px] uppercase tracking-wider text-slate-400 font-semibold">
          工具
        </p>
        <button
          onClick={() => onToggleAll(!allSelected)}
          className="text-[10px] text-primary-600 hover:text-primary-700 font-medium"
        >
          {allSelected ? "取消全选" : "全选"}
        </button>
      </div>
      <div className="space-y-0.5">
        {sources.map((s) => (
          <label
            key={s}
            className="flex items-center gap-2 px-2 py-1.5 text-xs text-slate-700 hover:bg-slate-50 rounded cursor-pointer"
          >
            <input
              type="checkbox"
              checked={selectedSources.has(s)}
              onChange={() => onToggle(s)}
              className="rounded border-slate-300 text-primary-600 focus:ring-primary-500"
            />
            <span
              className="w-2 h-2 rounded-full shrink-0"
              style={{ background: getSourceColor(s) }}
            />
            <span className="truncate">{getSourceLabel(s)}</span>
          </label>
        ))}
      </div>
    </div>
  );
}
```

- [ ] **Step 2: Wire the new prop in Sidebar.tsx**

In `frontend/src/components/Sidebar.tsx`:

Add to the SidebarProps interface (next to existing `onSourceToggle`):
```typescript
  onSourceGroupToggle: (selectAll: boolean) => void;
```

Add to the destructured params:
```typescript
  onSourceGroupToggle,
```

And update the `<SidebarSourceList>` JSX usage to pass it:
```tsx
          <SidebarSourceList
            sources={sources}
            selectedSources={selectedSources}
            onToggle={onSourceToggle}
            onToggleAll={onSourceGroupToggle}
          />
```

- [ ] **Step 3: Wire the handler in App.tsx**

In `frontend/src/App.tsx`, add a new callback near the other handlers:

```typescript
  const handleSourceGroupToggle = useCallback(
    (selectAll: boolean) => {
      setSelectedSources(() => {
        if (selectAll) return new Set(filters.sources);
        return new Set();
      });
      setPage(1);
    },
    [filters.sources]
  );
```

Pass it down to `<Sidebar … onSourceGroupToggle={handleSourceGroupToggle} … />`.

- [ ] **Step 4: Type-check**

```bash
cd frontend && npx tsc --noEmit 2>&1 | tail -10
```
Expected: no errors.

- [ ] **Step 5: Commit**

```bash
git add frontend/src/components/SidebarSourceList.tsx frontend/src/components/Sidebar.tsx frontend/src/App.tsx
git -c commit.gpgsign=false commit -m "feat: add select-all/clear toggle to sidebar tool list

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 7: Sidebar vendor list — add select-all / clear-all to the regular group

**Files:**
- Modify: `frontend/src/components/SidebarVendorList.tsx`

- [ ] **Step 1: Add a separate handler prop for regular-vendor group + render its buttons**

Replace `frontend/src/components/SidebarVendorList.tsx`:

```typescript
import { getVendorColor } from "../lib/utils";

const SUBSCRIPTION_VENDORS = ["kimi", "xunfei", "opencode-go", "opencode"];

interface SidebarVendorListProps {
  vendors: string[];
  selectedVendors: ReadonlySet<string>;
  onToggle: (vendor: string) => void;
  onToggleSubscriptionGroup: (selectAll: boolean) => void;
  onToggleRegularGroup: (selectAll: boolean) => void;
}

export function SidebarVendorList({
  vendors,
  selectedVendors,
  onToggle,
  onToggleSubscriptionGroup,
  onToggleRegularGroup,
}: SidebarVendorListProps) {
  if (vendors.length === 0) return null;

  const regularVendors = vendors.filter((v) => !SUBSCRIPTION_VENDORS.includes(v));
  const subVendors = vendors.filter((v) => SUBSCRIPTION_VENDORS.includes(v));
  const allSubSelected =
    subVendors.length > 0 && subVendors.every((v) => selectedVendors.has(v));
  const allRegSelected =
    regularVendors.length > 0 && regularVendors.every((v) => selectedVendors.has(v));

  return (
    <div className="py-3 px-3">
      <div className="px-2 mb-1.5 flex items-center justify-between">
        <p className="text-[10px] uppercase tracking-wider text-slate-400 font-semibold">
          供应商
        </p>
        {regularVendors.length > 0 && (
          <button
            onClick={() => onToggleRegularGroup(!allRegSelected)}
            className="text-[10px] text-primary-600 hover:text-primary-700 font-medium"
          >
            {allRegSelected ? "取消全选" : "全选"}
          </button>
        )}
      </div>
      <div className="space-y-0.5">
        {regularVendors.map((v) => (
          <VendorRow
            key={v}
            vendor={v}
            selected={selectedVendors.has(v)}
            onToggle={onToggle}
          />
        ))}
      </div>

      {subVendors.length > 0 && (
        <>
          <div className="mt-3 px-2 mb-1 flex items-center justify-between">
            <p className="text-[10px] uppercase tracking-wider text-slate-400 font-semibold">
              订阅
            </p>
            <button
              onClick={() => onToggleSubscriptionGroup(!allSubSelected)}
              className="text-[10px] text-primary-600 hover:text-primary-700 font-medium"
            >
              {allSubSelected ? "取消全选" : "全选"}
            </button>
          </div>
          <div className="space-y-0.5">
            {subVendors.map((v) => (
              <VendorRow
                key={v}
                vendor={v}
                selected={selectedVendors.has(v)}
                onToggle={onToggle}
              />
            ))}
          </div>
        </>
      )}
    </div>
  );
}

function VendorRow({
  vendor,
  selected,
  onToggle,
}: {
  vendor: string;
  selected: boolean;
  onToggle: (v: string) => void;
}) {
  return (
    <label className="flex items-center gap-2 px-2 py-1.5 text-xs text-slate-700 hover:bg-slate-50 rounded cursor-pointer">
      <input
        type="checkbox"
        checked={selected}
        onChange={() => onToggle(vendor)}
        className="rounded border-slate-300 text-primary-600 focus:ring-primary-500"
      />
      <span
        className="w-2 h-2 rounded-full shrink-0"
        style={{ background: getVendorColor(vendor) }}
      />
      <span className="truncate">{vendor}</span>
    </label>
  );
}
```

- [ ] **Step 2: Wire the prop in Sidebar.tsx**

In `frontend/src/components/Sidebar.tsx`:

Add to the SidebarProps interface:
```typescript
  onVendorGroupToggle: (selectAll: boolean) => void;
```

Destructure it and pass it down to `<SidebarVendorList … onToggleRegularGroup={onVendorGroupToggle} … />`.

- [ ] **Step 3: Wire the handler in App.tsx**

```typescript
  const handleVendorGroupToggle = useCallback(
    (selectAll: boolean) => {
      const regular = filters.vendors.filter(
        (v) => !["kimi", "xunfei", "opencode-go", "opencode"].includes(v)
      );
      setSelectedVendors((prev) => {
        const next = new Set(prev);
        for (const v of regular) {
          if (selectAll) next.add(v);
          else next.delete(v);
        }
        return next;
      });
      setPage(1);
    },
    [filters.vendors]
  );
```

Pass it down via `<Sidebar … onVendorGroupToggle={handleVendorGroupToggle} … />`.

- [ ] **Step 4: Type-check**

```bash
cd frontend && npx tsc --noEmit 2>&1 | tail -10
```
Expected: no errors.

- [ ] **Step 5: Commit**

```bash
git add frontend/src/components/SidebarVendorList.tsx frontend/src/components/Sidebar.tsx frontend/src/App.tsx
git -c commit.gpgsign=false commit -m "feat: add select-all/clear toggle to regular vendors group

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 8: Rewrite SidebarModelPicker as a multi-select checkbox list

**Files:**
- Modify: `frontend/src/components/SidebarModelPicker.tsx`

- [ ] **Step 1: Replace the file**

```typescript
import { useMemo, useState } from "react";

interface SidebarModelPickerProps {
  models: string[];
  selectedModels: ReadonlySet<string>;
  onSelectedModelsChange: (next: Set<string>) => void;
  advancedModels: string[];
  hideFreeModels: boolean;
  onHideFreeModelsChange: (hide: boolean) => void;
}

export function SidebarModelPicker({
  models,
  selectedModels,
  onSelectedModelsChange,
  advancedModels,
  hideFreeModels,
  onHideFreeModelsChange,
}: SidebarModelPickerProps) {
  const [query, setQuery] = useState("");

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return models;
    return models.filter((m) => m.toLowerCase().includes(q));
  }, [models, query]);

  const visibleSet = useMemo(() => new Set(filtered), [filtered]);
  const allVisibleSelected =
    filtered.length > 0 && filtered.every((m) => selectedModels.has(m));

  const toggle = (model: string) => {
    const next = new Set(selectedModels);
    if (next.has(model)) next.delete(model);
    else next.add(model);
    onSelectedModelsChange(next);
  };

  const selectAllVisible = () => {
    const next = new Set(selectedModels);
    for (const m of filtered) next.add(m);
    onSelectedModelsChange(next);
  };

  const clearAll = () => {
    onSelectedModelsChange(new Set());
  };

  const applyAdvanced = () => {
    const available = new Set(models);
    const next = new Set(advancedModels.filter((m) => available.has(m)));
    onSelectedModelsChange(next);
  };

  return (
    <div className="py-3 px-3">
      <div className="px-2 mb-1.5 flex items-center justify-between">
        <p className="text-[10px] uppercase tracking-wider text-slate-400 font-semibold">
          模型
        </p>
        <div className="flex items-center gap-1.5">
          <button
            onClick={selectAllVisible}
            className="text-[10px] text-primary-600 hover:text-primary-700 font-medium"
            disabled={filtered.length === 0 || allVisibleSelected}
          >
            全选
          </button>
          <span className="text-slate-300 text-[10px]">·</span>
          <button
            onClick={applyAdvanced}
            className="text-[10px] text-primary-600 hover:text-primary-700 font-medium"
            disabled={advancedModels.length === 0}
            title="应用高级模型预设"
          >
            高级
          </button>
          <span className="text-slate-300 text-[10px]">·</span>
          <button
            onClick={clearAll}
            className="text-[10px] text-slate-500 hover:text-slate-700 font-medium"
            disabled={selectedModels.size === 0}
          >
            清除
          </button>
        </div>
      </div>

      <div className="px-2 mb-1.5">
        <input
          type="text"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="搜索模型..."
          className="w-full px-2 py-1 text-xs border border-slate-200 rounded outline-none focus:ring-1 focus:ring-primary-500 focus:border-primary-500"
        />
      </div>

      <div className="space-y-0.5 max-h-72 overflow-y-auto">
        {filtered.length === 0 ? (
          <p className="px-2 py-2 text-xs text-slate-400">无匹配</p>
        ) : (
          filtered.map((m) => (
            <label
              key={m}
              className="flex items-center gap-2 px-2 py-1.5 text-xs text-slate-700 hover:bg-slate-50 rounded cursor-pointer"
              title={m}
            >
              <input
                type="checkbox"
                checked={selectedModels.has(m)}
                onChange={() => toggle(m)}
                className="rounded border-slate-300 text-primary-600 focus:ring-primary-500"
              />
              <span className="truncate">{m}</span>
            </label>
          ))
        )}
      </div>

      <label className="mt-2 flex items-center gap-2 px-1 py-1 text-xs text-slate-700 hover:bg-slate-50 rounded cursor-pointer">
        <input
          type="checkbox"
          checked={hideFreeModels}
          onChange={(e) => onHideFreeModelsChange(e.target.checked)}
          className="rounded border-slate-300 text-primary-600 focus:ring-primary-500"
        />
        <span>过滤免费</span>
      </label>

      {/* visibleSet keeps the lint happy about useMemo when only used in toggles */}
      {/* eslint-disable-next-line @typescript-eslint/no-unused-expressions */}
      {visibleSet.size === Infinity && null}
    </div>
  );
}
```

Note: the final `visibleSet` block is a defensive measure — if eslint flags `visibleSet` as unused, delete the `useMemo` and the dummy expression. (Most lint configs don't flag declared-and-typed memos, so likely no-op.)

- [ ] **Step 2: Type-check (will fail until Task 9 updates the call site)**

```bash
cd frontend && npx tsc --noEmit 2>&1 | tail -20
```
Expected: errors complaining that Sidebar.tsx passes the old props to SidebarModelPicker. These resolve in Task 9.

- [ ] **Step 3: Commit (component rewrite only, call sites updated next task)**

```bash
git add frontend/src/components/SidebarModelPicker.tsx
git -c commit.gpgsign=false commit -m "refactor: rewrite SidebarModelPicker as multi-select checkbox list

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 9: Unify model state in App.tsx — drop selectedModel, share selectedPivotModels

**Files:**
- Modify: `frontend/src/App.tsx`
- Modify: `frontend/src/components/Sidebar.tsx`

- [ ] **Step 1: Update Sidebar.tsx props**

In `frontend/src/components/Sidebar.tsx`:

Replace the model-related props in the interface:
```typescript
  // remove: selectedModel, onModelChange
  // add:
  selectedModels: ReadonlySet<string>;
  onSelectedModelsChange: (next: Set<string>) => void;
  advancedModels: string[];
```

Update the destructured params block accordingly.

Update the `<SidebarModelPicker>` usage:
```tsx
          <SidebarModelPicker
            models={models}
            selectedModels={selectedModels}
            onSelectedModelsChange={onSelectedModelsChange}
            advancedModels={advancedModels}
            hideFreeModels={hideFreeModels}
            onHideFreeModelsChange={onHideFreeModelsChange}
          />
```

- [ ] **Step 2: Update App.tsx to remove single-select model state**

In `frontend/src/App.tsx`:

Remove these lines:
```typescript
  const [selectedModel, setSelectedModel] = useState<string>("");
```

Remove the now-dead `effectiveModel`, `handleModelChange`, and the `getOriginalModels` import (it stays only if it's used elsewhere; in `loadRequests` we will use `selectedPivotModels` directly).

Replace `loadRequests`:
```typescript
  const loadRequests = useCallback(async () => {
    if (!appliedRange.from || !appliedRange.to) return;
    if (hasEmptyRequiredSelection) {
      setRequests(emptyRequests(1));
      return;
    }
    try {
      const modelParam = modelFilter; // same CSV used for stats
      const r = await fetchRequests(
        appliedRange.from,
        appliedRange.to,
        vendorFilter,
        modelParam,
        sourceFilter,
        page,
        50,
        tzOffset
      );
      setRequests(r);
    } catch (e) {
      console.error("Failed to load requests", e);
    }
  }, [
    appliedRange.from,
    appliedRange.to,
    vendorFilter,
    modelFilter,
    sourceFilter,
    page,
    tzOffset,
    hasEmptyRequiredSelection,
  ]);
```

Update the `<Sidebar … />` JSX to pass the new props:
```tsx
          <Sidebar
            open={sidebarOpen}
            onClose={() => setSidebarOpen(false)}
            activePreset={activePreset}
            onTimeRangeChange={handleTimeRangeChange}
            sources={filters.sources}
            selectedSources={selectedSources}
            onSourceToggle={handleSourceToggle}
            onSourceGroupToggle={handleSourceGroupToggle}
            vendors={filters.vendors}
            selectedVendors={selectedVendors}
            onVendorToggle={handleVendorToggle}
            onSubscriptionGroupToggle={handleSubscriptionGroupToggle}
            onVendorGroupToggle={handleVendorGroupToggle}
            models={filteredModels}
            selectedModels={selectedPivotModels}
            onSelectedModelsChange={setSelectedPivotModels}
            advancedModels={advancedModels}
            hideFreeModels={hideFreeModels}
            onHideFreeModelsChange={setHideFreeModels}
            onOpenSettings={() => setShowSettings(true)}
          />
```

`filteredModels` currently filters via `getDisplayModel` — keep that, since the sidebar's model list and the pivot table use the same display-model normalization.

- [ ] **Step 3: Update pivotModelOptions to use display models too (consistency)**

`pivotModelOptions` is currently `[...filters.models].sort()` — the un-normalized list. To keep the same set of model names everywhere, change it to:
```typescript
  const pivotModelOptions = useMemo(
    () => [...new Set(filters.models.map(getDisplayModel))].sort(),
    [filters.models]
  );
```
And keep the `getDisplayModel` import.

- [ ] **Step 4: Type-check the full frontend**

```bash
cd frontend && npx tsc --noEmit 2>&1 | tail -20
```
Expected: no errors.

- [ ] **Step 5: Run the frontend pivot tests**

```bash
cd frontend && npx tsx --test src/lib/pivotTable.test.ts src/lib/filterState.test.ts 2>&1 | tail -10
```
Expected: all tests pass.

- [ ] **Step 6: Lint**

```bash
cd frontend && npx eslint src 2>&1 | tail -20
```
Expected: no errors. (If `visibleSet` lint warning appears in SidebarModelPicker.tsx, remove the unused `visibleSet` useMemo block.)

- [ ] **Step 7: Commit**

```bash
git add frontend/src/App.tsx frontend/src/components/Sidebar.tsx
git -c commit.gpgsign=false commit -m "feat: unify sidebar + pivot model selection (multi-select with presets)

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 10: Full verification — backend tests, frontend tests, type check, lint

**Files:** (none — verification only)

- [ ] **Step 1: Backend tests**

From repo root:
```bash
cd backend && cargo test --quiet 2>&1 | tail -10
```
Expected: all tests pass.

- [ ] **Step 2: Backend clippy + fmt**

```bash
cd backend && cargo clippy --quiet --all-targets -- -D warnings 2>&1 | tail -20
cd backend && cargo fmt --check 2>&1 | tail -5
```
Expected: no clippy warnings, no fmt diffs. If `cargo fmt --check` fails, run `cargo fmt` and commit the formatting fix.

- [ ] **Step 3: Frontend tests**

```bash
cd frontend && npx tsx --test src/lib/*.test.ts 2>&1 | tail -10
```
Expected: all tests pass.

- [ ] **Step 4: Frontend type check + lint**

```bash
cd frontend && npx tsc --noEmit 2>&1 | tail -10
cd frontend && npx eslint src 2>&1 | tail -10
```
Expected: no errors from either.

- [ ] **Step 5: Frontend build**

```bash
cd frontend && npm run build 2>&1 | tail -20
```
Expected: build completes without errors. Output lands in `backend/static`.

- [ ] **Step 6: Commit any fmt/lint-driven changes**

If steps 2/4 produced fixes:
```bash
git add -A
git -c commit.gpgsign=false commit -m "chore: apply formatter / linter

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```
Otherwise skip.

---

## Task 11: Manual smoke test + apply the data migration

**Files:** (none — operational steps)

- [ ] **Step 1: Apply the DeepSeek-export migration (live)**

```bash
python3 scripts/migrate_deepseek_export_classification.py --dry-run
```
Sanity: 10 records, 202,454,457 total tokens. Then:
```bash
python3 scripts/migrate_deepseek_export_classification.py
```
Verify a `.bak.YYYYMMDD` exists alongside `~/.pi/token-logs/usage.jsonl`.

- [ ] **Step 2: Restart the backend**

```bash
sudo systemctl restart token-stats@3000 2>/dev/null || (cd backend && cargo run --release &)
```
(Use whichever fits your environment.)

- [ ] **Step 3: Open the dashboard and verify (browser)**

1. Open http://localhost/token-stats/ (or whichever URL is configured).
2. Confirm the sidebar:
   - Tool list has a 全选 / 取消全选 button.
   - 供应商 has 全选 / 取消全选 over the regular group AND 订阅 group separately.
   - 模型 panel renders as a checkbox list with 全选 · 高级 · 清除 buttons + search input.
3. Click 高级 in the model picker — only models from the configured advanced-models list become checked.
4. Open 请求 → 供应商 & 模型表现:
   - Verify the new 输出比 column appears between 缓存命中 and 费用.
   - Click the 输出比 header — sort reverses.
   - Confirm deepseek/deepseek-v4-pro rows now show non-zero 费用 and a non-zero 输出比.
5. Expand 详细请求 and confirm the 输出比 column shows up there too.
6. Filter sidebar to vendor=deepseek and model=deepseek-v4-pro. Confirm the pivot summary's 费用 reflects the migrated records (compare to the count of records you'd expect — should include the previously zero-cost 10 records + existing pi/deepseek records).

- [ ] **Step 4: Push the branch and open the PR**

```bash
git push -u origin fix/sidebar-filters-output-ratio-deepseek-classification
gh pr create --title "Fix: sidebar filters, output-ratio column, DeepSeek-export classification" --body "$(cat <<'EOF'
## Summary
- Sidebar gets a multi-select model picker (checkbox list with 全选/高级/清除 + search)
- Sidebar vendor list + tool list grow 全选/取消全选 toggles for each group
- Pivot table + detailed-requests table both gain a 输出比 column with color thresholds
- 10 legacy DeepSeek-export records reclassified from \`opencode-go/opencode\` → \`deepseek/deepseek-ai\`; \`recover_token_data.py\` map updated so future runs do this automatically; \`pricing.rs\` learns to compute DeepSeek-record cost from pricing.toml when stored \`cost == 0\`

Design: docs/superpowers/specs/2026-05-23-filters-output-ratio-deepseek-classification-design.md

## Test plan
- [ ] cargo test passes (new \`deepseek_zero_cost_computes_from_tokens_in_cny\` covers the pricing path)
- [ ] npx tsx --test pivotTable.test.ts passes (new output_ratio tests)
- [ ] tsc --noEmit, eslint, vite build all clean
- [ ] Dry-run migration finds 10 records, ~202M tokens
- [ ] Live migration creates .bak and dashboard now shows non-zero cost for deepseek-v4-pro reclassified records
- [ ] Sidebar UX: multi-select model picker works, advanced preset checks the configured set, search filters, 全选/清除 toggle vendor & tool groups
- [ ] Pivot 输出比 column sorts correctly and shows expected percentages (deepseek-v4-pro ~0.3–0.9% — matches investigation)

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

---

## Self-Review

**Spec coverage:**
- Bug #1 (multi-select model picker + advanced preset) — Task 8 + Task 9 ✓
- Bug #2 (select-all/clear on vendor + tool lists) — Task 6 + Task 7 ✓
- Bug #3 (output-ratio column + diagnose deepseek-v4-pro cost spread) — Task 4 + Task 5; root cause documented in spec; output ratio surfaces in UI for ongoing investigation ✓
- Bug #4 (reclassify 10 records + fix recovery script + pricing.rs deepseek path) — Task 1 + Task 2 + Task 3 + Task 11 ✓
- "Share state, keep both" architecture for model picker — Task 9 wires `selectedPivotModels` as the shared state, pivot dropdown stays ✓

**Placeholder scan:** None found. All steps include exact paths, complete code, and exact commands.

**Type consistency:** `selectedModels` / `onSelectedModelsChange` / `advancedModels` props match between `Sidebar.tsx` and `SidebarModelPicker.tsx`. `output_ratio` field appears in `PivotSummary` and is computed by `finalizeSummary` — `RequestsSection.tsx` reads it as `summary.output_ratio` consistently. `onSourceGroupToggle` / `onVendorGroupToggle` handler names match between App.tsx, Sidebar.tsx, and the sidebar children.

**Notes / risks acknowledged in plan:**
- Output-ratio column won't show a dramatic difference for deepseek-v4-pro between sources (verified in spec) — UI still useful for general transparency.
- Migration appends `.bak.YYYYMMDD` rather than touching usage.jsonl in destructive ways.
- `cost==0` for DeepSeek records was previously displayed as 0; after Task 1's pricing path, they'll display a computed CNY value — this is a behavior change that's intentional (more accurate, matches design).
