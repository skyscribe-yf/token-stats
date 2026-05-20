# Review: fix/chart-metric-filter-and-model-dropdown vs main

> **Note:** The branch `fix/chart-metric-filter-and-model-dropdown` was created from `main` at commit `823dc04` and has no commits of its own. All intended changes were in a stash (`stash@{0}`) which has been applied to the working tree for review. The diff covers 2 files: `frontend/src/App.tsx` and `deploy.sh`.

---

## Build Verification
- ✅ `npx tsc --noEmit` — TypeScript passes with `noUnusedLocals: true`, `noUnusedParameters: true`
- ✅ `npm run build` — Vite builds successfully (627 KB JS bundle, no errors)

---

## Correct ✅

### 1. `LineChart` → `ComposedChart` import swap
- **Evidence:** `frontend/src/App.tsx:11` now imports `ComposedChart` instead of `LineChart`
- `grep -n "LineChart"` returns zero matches — fully removed from both imports and JSX
- `ComposedChart` supports both `Bar` and `Line` children, which is exactly what the new chart needs

### 2. TypeScript type safety
- `ChartMetricKey` derived from `CHART_METRIC_OPTIONS` via `typeof ... [number]["key"]` (`frontend/src/App.tsx:132`)
- `chartMetrics` typed as `Set<ChartMetricKey>` (`frontend/src/App.tsx:286`)
- `CHART_METRIC_OPTIONS` is `as const` for literal type narrowing
- `CustomTooltip` uses `p.name?.includes(...)` with optional chaining — handles `undefined` name gracefully
- `formatPercent` and `formatNumber` both accept `Number()` coercion, consistent with existing patterns

### 3. State management correctness
- **`chartMetrics`**: Initialized to `Set(["cache", "input", "output", "cacheHitRatio"])` — defaults match the 4 most useful metrics, with `cacheHitRatioNoXunfei` opt-in. Initialized with lazy initializer `() => new Set(...)` for stability.
- **`showChartFilter`**: Simple boolean toggle, mirrored exactly by existing `showCustomPanel`/`showSourceFilter`/`showVendorFilter` patterns.
- **`filteredModels`**: Falls back to `filters.models` when stats haven't loaded yet; otherwise derives from `stats.by_model` to show only models present in the current time range. Correct dependency array `[stats, filters.models]`.
- **`showRatioAxis`**: Memo on `chartMetrics` — only renders the right-side percentage Y-axis when at least one ratio metric is selected. Consistent with the guard on `Line` children.

### 4. Consistent Chinese UI labels
- `chartMetrics: "图表指标"` — consistent with existing `ZH` convention
- `cacheLabel: "缓存"` — new label for combined cache bar (formerly `cacheReadLabel: "缓存读取"`)
- `CHART_METRIC_OPTIONS` labels match `ZH`: `"缓存"`, `"输入"`, `"输出"`, `"缓存命中率"`, `"缓存命中率(无讯飞)"`

### 5. CustomTooltip improvement (ratio formatting)
- **Lines 148–154:** Now checks `p.name?.includes("命中率")` to decide `formatPercent` vs `formatNumber`
- Cache hit ratio values displayed as percentages (e.g. `42.3%`) instead of raw numbers — correct
- Works for both `ZH.cacheHitLabel` ("缓存命中率") and `ZH.cacheHitNoXunfei` ("缓存命中率(无讯飞)")

### 6. Outside-click handler for chart filter panel
- **Lines 521–532:** Mirrors the existing pattern at lines ~492–507 for `showCustomPanel`
- Proper cleanup: `document.removeEventListener("mousedown", handler)` in return function
- Early return `if (!showChartFilter) return;` prevents unnecessary event binding

### 7. `deploy.sh` changes are well-structured
- Port wait loop with timeout prevents silent failures from port conflicts
- Graceful kill chain: systemd stop → TERM → KILL
- `set -e` compatibility: explicit `exit 1` on timeout

---

## Issues Found

### Blocker: None

### Note 1: Dead ZH constants (`cacheReadLabel`, `cacheHitTrend`)
- **File:** `frontend/src/App.tsx:75, 107`
- **Problem:** `cacheReadLabel: "缓存读取"` and `cacheHitTrend: "缓存命中率趋势"` are defined in `ZH` but never referenced after the change.
- **Why:** `cacheReadLabel` was used in the old `Line` with `name={ZH.cacheReadLabel}` (removed). `cacheHitTrend` was used in the old chart title `{ZH.dailyTokenUsage} & {ZH.cacheHitTrend}` (removed).
- **Risk:** Low — just dead code. Not caught by `noUnusedLocals` because they're object properties, not variables.
- **Recommendation:** Remove both entries from `ZH` to keep the object clean.

### Note 2: `selectedModel` in `loadStats` deps causes unnecessary refetches
- **File:** `frontend/src/App.tsx:400`
- **Problem:** Adding `selectedModel` to the `loadStats` `useCallback` dependency array means every time the model dropdown changes, `fetchStats()` + `fetchFilters()` are called — even though `/api/stats` does not accept a `model` parameter.
- **Why it was added:** So the model availability check (lines 381–385) inside `loadStats` can reference the current `selectedModel`.
- **Impact:** Each model dropdown change triggers 2 unnecessary API calls (stats + filters refresh). In practice these are cached/in-memory on the backend, so impact is minimal.
- **Alternative:** The model availability check could use a ref (`selectedModelRef`) instead of adding `selectedModel` to deps, avoiding the refetch.

### Note 3: `deploy.sh` changes are unrelated to chart/filter feature
- **File:** `deploy.sh:7-44`
- **Nature:** Infrastructure hardening (process kill, port wait loop) — entirely separate from the chart metric filter and model dropdown feature.
- **Recommendation:** Should be in a separate commit/PR for cleaner history, but doesn't cause regressions.

### Note 4: `filteredModels` has no `selectedModel` in deps (correct, but worth noting)
- **File:** `frontend/src/App.tsx:607-610`
- `filteredModels` depends on `[stats, filters.models]` — correct. The model dropdown's `<option>` list is derived from stats data, not from the selected model. This is intentional: the dropdown should show all available models regardless of current selection.

---

## Summary

| Check | Result |
|-------|--------|
| Unintended changes / regressions | None found (2 minor notes above) |
| Build correctness | ✅ TypeScript + Vite pass |
| TypeScript type safety | ✅ Strict types, no `any` used |
| Chinese UI labels (ZH) | ✅ Consistent, 2 dead entries noted |
| Unused imports (LineChart) | ✅ Fully removed |
| State management | ✅ Correct, 1 perf note on deps |
