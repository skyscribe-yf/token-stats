import type { ModelStats, SourceDetailStats } from "../api";

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

export type SortDirection = "asc" | "desc";

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

export interface PivotModelNode {
  model: string;
  source_details: SourceDetailStats[];
  summary: PivotSummary;
}

export interface PivotTreeNode {
  provider: string;
  models: PivotModelNode[];
  summary: PivotSummary;
}

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

function accumulateSummary(acc: PivotSummary, sd: SourceDetailStats): void {
  acc.calls += sd.calls;
  acc.input_tokens += sd.input_tokens;
  acc.output_tokens += sd.output_tokens;
  acc.cache_read_tokens += sd.cache_read_tokens;
  acc.cache_write_tokens += sd.cache_write_tokens;
  acc.total_tokens += sd.total_tokens;
  acc.cost += sd.cost;
}

function computeCacheHitRatio(summary: PivotSummary): number {
  const denom = summary.input_tokens + summary.cache_read_tokens;
  return denom > 0 ? (summary.cache_read_tokens / denom) * 100 : 0;
}

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

function getAvgCost(summary: PivotSummary): number {
  if (summary.total_tokens <= 0) return 0;
  return (summary.cost / summary.total_tokens) * 1_000_000;
}

export function getSortValue(
  summary: PivotSummary,
  column: SortColumn
): number | string {
  switch (column) {
    case "name":
      return "";
    case "calls":
      return summary.calls;
    case "input_tokens":
      return summary.input_tokens;
    case "output_tokens":
      return summary.output_tokens;
    case "cache":
      return summary.cache_read_tokens + summary.cache_write_tokens;
    case "total_tokens":
      return summary.total_tokens;
    case "cache_hit_ratio":
      return summary.cache_hit_ratio;
    case "output_ratio":
      return summary.output_ratio;
    case "cost":
      return summary.cost;
    case "avg_cost":
      return getAvgCost(summary);
    default:
      return 0;
  }
}

function compareValues(
  a: number | string,
  b: number | string,
  direction: SortDirection
): number {
  let cmp: number;
  if (typeof a === "string" && typeof b === "string") {
    cmp = a.localeCompare(b);
  } else if (typeof a === "number" && typeof b === "number") {
    cmp = a - b;
  } else {
    cmp = String(a).localeCompare(String(b));
  }
  return direction === "desc" ? -cmp : cmp;
}

export function buildPivotTree(
  modelStats: ModelStats[],
  sortColumn: SortColumn,
  sortDirection: SortDirection,
  hideFreeModels: boolean
): PivotTreeNode[] {
  // Step 1: group by provider, merge display models
  type SD = SourceDetailStats;
  const map = new Map<string, Map<string, ModelStats>>();

  for (const m of modelStats) {
    const displayModel = getDisplayModel(m.model);
    const providerMap = map.get(m.provider) || new Map<string, ModelStats>();
    const existing = providerMap.get(displayModel);
    if (existing) {
      existing.calls += m.calls;
      existing.input_tokens += m.input_tokens;
      existing.output_tokens += m.output_tokens;
      existing.cache_read_tokens += m.cache_read_tokens;
      existing.cache_write_tokens += m.cache_write_tokens;
      existing.total_tokens += m.total_tokens;
      existing.cost += m.cost;
      // Merge source details
      const sourceMap = new Map<string, SD>();
      for (const sd of existing.source_details) {
        sourceMap.set(sd.source, { ...sd });
      }
      for (const sd of m.source_details) {
        const esd = sourceMap.get(sd.source);
        if (esd) {
          esd.calls += sd.calls;
          esd.input_tokens += sd.input_tokens;
          esd.output_tokens += sd.output_tokens;
          esd.cache_read_tokens += sd.cache_read_tokens;
          esd.cache_write_tokens += sd.cache_write_tokens;
          esd.total_tokens += sd.total_tokens;
          esd.cost += sd.cost;
        } else {
          sourceMap.set(sd.source, { ...sd });
        }
      }
      existing.source_details = Array.from(sourceMap.values());
      existing.sources = existing.source_details.map((sd) => sd.source).sort();
      const denom = existing.input_tokens + existing.cache_read_tokens;
      existing.cache_hit_ratio = denom > 0 ? (existing.cache_read_tokens / denom) * 100 : 0;
    } else {
      providerMap.set(displayModel, { ...m, model: displayModel });
    }
    map.set(m.provider, providerMap);
  }

  // Step 2: Build tree with filtering
  const tree: PivotTreeNode[] = [];

  for (const [provider, modelsMap] of map.entries()) {
    const models: PivotModelNode[] = [];

    for (const [modelName, ms] of modelsMap.entries()) {
      let sourceDetails = ms.source_details;

      if (hideFreeModels) {
        sourceDetails = sourceDetails.filter((sd) => sd.cost > 0);
        if (sourceDetails.length === 0) continue;
      }

      // Recompute model summary from (filtered) source details
      const summary = emptySummary();
      for (const sd of sourceDetails) {
        accumulateSummary(summary, sd);
      }

      if (hideFreeModels && summary.cost <= 0) continue;

      // Sort source details
      sourceDetails = sortSourceDetails(sourceDetails, sortColumn, sortDirection);

      models.push({
        model: modelName,
        source_details: sourceDetails,
        summary: finalizeSummary(summary),
      });
    }

    if (models.length === 0) continue;

    // Recompute vendor summary from models
    const vendorSummary = emptySummary();
    for (const m of models) {
      vendorSummary.calls += m.summary.calls;
      vendorSummary.input_tokens += m.summary.input_tokens;
      vendorSummary.output_tokens += m.summary.output_tokens;
      vendorSummary.cache_read_tokens += m.summary.cache_read_tokens;
      vendorSummary.cache_write_tokens += m.summary.cache_write_tokens;
      vendorSummary.total_tokens += m.summary.total_tokens;
      vendorSummary.cost += m.summary.cost;
    }

    // Sort models
    models.sort((a, b) => {
      if (sortColumn === "name") {
        return compareValues(a.model, b.model, sortDirection);
      }
      return compareValues(
        getSortValue(a.summary, sortColumn),
        getSortValue(b.summary, sortColumn),
        sortDirection
      );
    });

    tree.push({
      provider,
      models,
      summary: finalizeSummary(vendorSummary),
    });
  }

  // Sort vendors
  tree.sort((a, b) => {
    if (sortColumn === "name") {
      return compareValues(a.provider, b.provider, sortDirection);
    }
    return compareValues(
      getSortValue(a.summary, sortColumn),
      getSortValue(b.summary, sortColumn),
      sortDirection
    );
  });

  return tree;
}

function sortSourceDetails(
  details: SourceDetailStats[],
  sortColumn: SortColumn,
  sortDirection: SortDirection
): SourceDetailStats[] {
  const sorted = [...details];
  sorted.sort((a, b) => {
    if (sortColumn === "name") {
      return compareValues(a.source, b.source, sortDirection);
    }
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
    return compareValues(
      getSortValue(aSummary, sortColumn),
      getSortValue(bSummary, sortColumn),
      sortDirection
    );
  });
  return sorted;
}

// Re-export getDisplayModel so consumers don't need to import it separately
const MODEL_MERGE_GROUPS: { display: string; originals: string[] }[] = [
  { display: "kimi-k2.6", originals: ["kimi-k2.6", "kimi-k2.6:high", "kimi-for-coding"] },
];

const originalToDisplayModel = new Map<string, string>();
for (const group of MODEL_MERGE_GROUPS) {
  for (const orig of group.originals) {
    originalToDisplayModel.set(orig, group.display);
  }
}

export function getDisplayModel(original: string): string {
  return originalToDisplayModel.get(original) || original;
}

export function getOriginalModels(display: string): string[] | null {
  const group = MODEL_MERGE_GROUPS.find((g) => g.display === display);
  return group ? group.originals : null;
}

export function computePivotSummary(tree: PivotTreeNode[]): PivotSummary | null {
  if (tree.length === 0) return null;
  const summary = emptySummary();
  for (const vendor of tree) {
    summary.calls += vendor.summary.calls;
    summary.input_tokens += vendor.summary.input_tokens;
    summary.output_tokens += vendor.summary.output_tokens;
    summary.cache_read_tokens += vendor.summary.cache_read_tokens;
    summary.cache_write_tokens += vendor.summary.cache_write_tokens;
    summary.total_tokens += vendor.summary.total_tokens;
    summary.cost += vendor.summary.cost;
  }
  return finalizeSummary(summary);
}
