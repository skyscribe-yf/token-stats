Now I have all the information needed. Let me compile the comprehensive review.

---

## Review

### Overview
The stashed changes on `fix/chart-metric-filter-and-model-dropdown` contain two groups of modifications: (1) the targeted `frontend/src/App.tsx` changes and (2) unrelated backend formatting/refactoring noise (whitespace, import reordering, deleted `useFilterInit.ts` hook — already unused in current code). This review focuses strictly on the `App.tsx` diff as instructed.

The frontend changes address all 5 review objectives: model dropdown fix, chart redesign (ComposedChart), metric filter dropdown, model reset logic, and relevant UX considerations.

---

### 1. Model dropdown fix — `filteredModels` derived from `stats.by_model`

**What changed:** In the detailed requests `<details>` section, the model `<select>` options changed from `filters.models.map(...)` to `filteredModels.map(...)` (line ~1545 in new version).

**New logic** (stash diff):
```ts
const filteredModels = useMemo(() => {
    if (!stats?.by_model) return filters.models;
    return [...new Set(stats.by_model.map((m) => m.model))].sort();
}, [stats, filters.models]);
```

- **Correct:** Verified in `backend/src/aggregator.rs` lines 65-81 that `by_model = compute_model_stats(&filtered)`, where `filtered` already applies source and provider filters. So `stats.by_model` only contains models present in the currently-filtered data. Deriving `filteredModels` from `stats.by_model` correctly respects active source/vendor filters.
- **Correct:** Deduplication via `new Set(...)` and `.sort()` ensures no duplicates and alphabetical ordering.
- **Correct:** Fallback to `filters.models` when `stats` is `null` (e.g., during initial load) keeps the dropdown populated.

- **Note (minor):** `filters.models` as a `useMemo` dependency is harmless but logically unnecessary in the primary path (only used when `stats?.by_model` is falsy). Its presence ensures the fallback updates if filters load before stats, which is fine.

**Verdict:** ✅ Correct.

---

### 2. Chart redesign — `ComposedChart` with stacked bars + lines

**What changed:** `LineChart` replaced with `ComposedChart`. Three `Bar` components with `stackId="tokens"` replace the three token `Line` components. Cache hit ratio `Line` components remain.

**Stacking order** (verified in stash diff, rendering order):
```
Bar dataKey="cache"    → stackId="tokens" → bottom of stack
Bar dataKey="input"    → stackId="tokens" → middle of stack
Bar dataKey="output"   → stackId="tokens" → top of stack
```
Recharts stacks bars in render order: first bar at bottom, last at top. This produces: **cache (bottom) → input (middle) → output (top)**. The total bar height = cache + input + output = `cache_read + cache_write + input + output` = `total_tokens`. Matches the intended composition.

**Colors:**
- Cache: `#8b5cf6` (purple/violet) — consistent with existing `cacheReadLabel` color
- Input: `#10b981` (emerald) — consistent with existing KPI row input color
- Output: `#f59e0b` (amber) — consistent with existing KPI row output color

**Cache hit ratio lines:**
- `cacheHitRatio`: `#f43f5e` (rose), dashed, right Y-axis — same as before
- `cacheHitRatioNoXunfei`: `#06b6d4` (cyan), dashed, right Y-axis — same as before
- Both conditionally rendered with `chartMetrics.has(key) && showRatioAxis`

**New `cache` data field** in `chartData`:
```ts
cache: d.cache_read_tokens + d.cache_write_tokens,
```
This aggregates cache reads and writes into a single bar. The old version showed `cacheRead` alone as a line; `cacheWrite` was available in the data but never displayed. Simplifying to a combined `cache` bar is a reasonable design choice for a stacked-bar view.

- **Note:** The chart title changed from `"Daily Token Usage & Cache Hit Ratio Trend"` to just `"Daily Token Usage"`. The cache hit ratio is now conveyed by the line overlays, which are togglable. This is fine UX since the metric filter button nearby indicates additional metrics are available.

**Verdict:** ✅ Correct stacking order, colors, and data mapping.

---

### 3. Chart metric filter dropdown

**What changed:** A button with a `SlidersHorizontal` icon opens a multi-select checkbox panel. Five options:
- 缓存 (cache, bar, purple)
- 输入 (input, bar, emerald)
- 输出 (output, bar, amber)
- 缓存命中率 (cacheHitRatio, line, rose)
- 缓存命中率(无讯飞) (cacheHitRatioNoXunfei, line, cyan)

**Default state:** cache, input, output, cacheHitRatio are checked; cacheHitRatioNoXunfei is unchecked.

**Click-outside handling** — new `useEffect` (stash diff):
```ts
useEffect(() => {
    if (!showChartFilter) return;
    const handler = (e: MouseEvent) => {
      const target = e.target as HTMLElement;
      if (!target.closest(".chart-metric-panel") && !target.closest(".chart-metric-btn")) {
        setShowChartFilter(false);
      }
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
}, [showChartFilter]);
```
- **Correct:** Follows the exact pattern of the existing `showCustomPanel` click-outside handler.
- **Correct:** Uses `.chart-metric-panel` and `.chart-metric-btn` classes as exclusion zones, preventing immediate re-close when clicking the toggle button.
- **Correct:** Cleanup removes the listener when the panel is closed or component unmounts.

**Conditional Y-axis:**
```ts
const showRatioAxis = useMemo(
    () => chartMetrics.has("cacheHitRatio") || chartMetrics.has("cacheHitRatioNoXunfei"),
    [chartMetrics]
);
```
When neither ratio metric is checked, the right Y-axis (`yAxisId="ratio"`) is removed entirely. This prevents an empty axis from rendering. Lines also have a redundant `&& showRatioAxis` guard.

**Interaction with chart rendering:**
- Each bar/line is wrapped in `{chartMetrics.has(key) && (<Bar .../>)}`
- Toggling a checkbox re-renders the chart with/without that metric
- All bars share `stackId="tokens"`, so removing one bar adjusts the stack height — e.g., removing "cache" shows only input+output stacked.

**Edge cases considered:**
- All unchecked → empty chart (no bars, no lines, no ratio axis). Only the left tokens Y-axis remains. Acceptable since user explicitly chose this.
- cacheHitRatioNoXunfei checked alone → ratio axis appears, only the cyan line renders. The left tokens axis still shows (always rendered).
- Good UX: the toggle button highlights when panel is open (`bg-primary-100 text-primary-700`).

**Verdict:** ✅ Correct implementation, good UX patterns.

---

### 4. `selectedModel` reset logic

**What changed** (in `loadData` callback, stash diff):
```ts
// Reset model selection if the currently selected model is no longer available
if (selectedModel) {
    const availableModels = new Set(s.by_model.map((m) => m.model));
    if (!availableModels.has(selectedModel)) {
        setSelectedModel("");
    }
}
```
And `selectedModel` was added to `loadData`'s dependency array.

- **Correct:** Uses `s.by_model` (the fresh stats response, not the stale state), so it correctly checks whether the currently-selected model still exists in the newly-filtered data.
- **Correct:** Only resets when `selectedModel` is non-empty (avoids unnecessary work).
- **Correct:** `setSelectedModel("")` resets to "all models" in the dropdown.

- **Note:** Adding `selectedModel` to `loadData`'s deps causes `loadData` to be re-created when the model changes. Since `loadData` is called from a `useEffect` keyed on `loadData`, this triggers an unnecessary stats re-fetch when the user changes the model dropdown (for request filtering). The stats fetch doesn't use `selectedModel` as a filter — it only uses `sourceFilter`, `vendorFilter`, `tzOffset`, and `resolution`. This is **wasteful but not harmful** — the re-fetched data will be identical.

    A cleaner approach would be a separate `useEffect`:
    ```ts
    useEffect(() => {
        if (stats && selectedModel) {
            const available = new Set(stats.by_model.map(m => m.model));
            if (!available.has(selectedModel)) setSelectedModel("");
        }
    }, [stats, selectedModel]);
    ```
    This avoids coupling the stats fetch to model selection changes. However, the current approach works correctly and the overhead of an extra stats fetch when changing the model filter is negligible for this dashboard's scale.

**Verdict:** ✅ Functionally correct. Minor performance nit as noted.

---

### 5. Potential bugs, edge cases, UX issues

#### ✅ Good

1. **`CustomTooltip` enhancement:** Ratio tooltip values now use `formatPercent` instead of `formatNumber`, detected by checking if the payload name contains "命中率". This is a solid improvement — previously a cache hit ratio of 75.5 would show as "75" (misleading), now shows "75.5%".

2. **`ZH.cacheLabel` and `ZH.chartMetrics`:** New Chinese labels are consistent with existing naming conventions.

3. **`ComposedChart` import:** The diff correctly replaces `LineChart` with `ComposedChart` in the import. `Line` and `Bar` were already imported (Bar was used for the vendor breakdown chart). No missing imports.

4. **`cacheWrite` now visible:** Previously only `cacheRead` was shown as a separate line; `cacheWrite` was invisible. The combined `cache` bar makes both visible. This is a net improvement in data transparency.

5. **No regression in vendor breakdown chart:** The `BarChart` in the vendor breakdown section uses `Bar` from the same import, which is unaffected.

#### ⚠️ Observations / Minor concerns

1. **Label truncation on X-axis:** With sub-day resolution (1h/2h/12h buckets), X-axis labels like "05-17 08:00" are already angled at -30°. Stacked bars make individual segment heights harder to read at a glance compared to overlaid lines. This is a tradeoff — bars are better for composition, lines are better for trend comparison. No action required, but worth noting.

2. **Legend order mismatch with visual stacking:** The Legend renders items in the order they appear in the JSX: cache, input, output, cacheHitRatio, cacheHitRatioNoXunfei. The visual stacking order is cache (bottom) → input → output (top). The legend matches the JSX order, which is also bottom-to-top. Consistent.

3. **`filteredModels` stale during resolution change:** When `resolution` changes (e.g., switching from "today" to "7d"), `loadData` re-fetches with new parameters. During the fetch, `stats` temporarily reflects the old data (React hasn't re-rendered with new stats yet). The `filteredModels` memo uses the old `stats.by_model` during this brief window. However, since the stats fetch replaces `stats` atomically and React batches the update, there's no user-visible flash. The models dropdown simply updates when new data arrives.

4. **Title change removed "Cache Hit Ratio Trend":** The chart title now says only "每日 Token 用量" (Daily Token Usage). The cache hit ratio is now an optional overlay controlled by the metric dropdown. This is semantically cleaner but users who don't notice the metric filter button may miss the ratio lines entirely. The default includes both ratio lines except cacheHitRatioNoXunfei, so it's discoverable.

5. **No "select all / deselect all" in metric filter:** The checkbox panel has 5 options with no bulk toggle. This is acceptable for a compact panel with few options.

---

### Summary

| Aspect | Verdict |
|--------|---------|
| Model dropdown filter correctness | ✅ Correct |
| Stacked bar order | ✅ Correct (cache→input→output) |
| Chart metric filter UI | ✅ Correct |
| Click-outside handling | ✅ Correct |
| Conditional Y-axis | ✅ Correct |
| Model reset on filter change | ✅ Correct (minor perf note) |
| Tooltip format for ratios | ✅ Improvement |
| No regressions | ✅ Verified |

**Blocker:** None. The changes are correct and ready to apply.

**Note (non-blocking):** Consider extracting the model-reset logic into a separate `useEffect` to avoid coupling `selectedModel` to the `loadData` dependency array (and the resulting unnecessary stats re-fetch). This is a minor optimization, not a correctness issue.