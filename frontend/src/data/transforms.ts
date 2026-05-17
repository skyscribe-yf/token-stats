import { formatDate } from "../lib/utils";
import type { StatsResponse, VendorStats, ModelStats } from "../api";

export interface ChartDataPoint {
  date: string;
  rawDate: string;
  calls: number;
  input: number;
  output: number;
  cacheRead: number;
  cacheWrite: number;
  total: number;
  cost: number;
  cacheHitRatio: number;
}

export interface VendorChartDatum {
  name: string;
  tokens: number;
  calls: number;
  cost: number;
  cacheHit: number;
}

export interface PieDatum {
  name: string;
  value: number;
}

export type MergedTableRow =
  | { type: "vendor"; data: VendorStats }
  | { type: "model"; data: ModelStats };

export function buildChartData(stats: StatsResponse | null): ChartDataPoint[] {
  if (!stats?.by_date) return [];
  return stats.by_date.map((d) => ({
    date: formatDate(d.date),
    rawDate: d.date,
    calls: d.calls,
    input: d.input_tokens,
    output: d.output_tokens,
    cacheRead: d.cache_read_tokens,
    cacheWrite: d.cache_write_tokens,
    total: d.total_tokens,
    cost: d.cost,
    cacheHitRatio: d.cache_hit_ratio,
  }));
}

export function buildVendorChartData(
  stats: StatsResponse | null
): VendorChartDatum[] {
  if (!stats?.by_vendor) return [];
  return stats.by_vendor.map((v) => ({
    name: v.provider,
    tokens: v.total_tokens,
    calls: v.calls,
    cost: v.cost,
    cacheHit: v.cache_hit_ratio,
  }));
}

export function buildPieData(stats: StatsResponse | null): PieDatum[] {
  if (!stats?.by_vendor) return [];
  return stats.by_vendor.map((v) => ({
    name: v.provider,
    value: v.total_tokens,
  }));
}

export function buildMergedTableData(
  stats: StatsResponse | null
): MergedTableRow[] {
  if (!stats?.by_vendor || !stats?.by_model) return [];
  const modelMap = new Map<string, ModelStats[]>();
  for (const m of stats.by_model) {
    const arr = modelMap.get(m.provider) || [];
    arr.push(m);
    modelMap.set(m.provider, arr);
  }
  const rows: MergedTableRow[] = [];
  for (const v of stats.by_vendor) {
    rows.push({ type: "vendor", data: v });
    const models = modelMap.get(v.provider) || [];
    for (const m of models) {
      rows.push({ type: "model", data: m });
    }
  }
  return rows;
}
