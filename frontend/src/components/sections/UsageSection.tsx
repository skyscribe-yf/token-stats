import { useMemo, useState, useEffect, useRef } from "react";
import {
  BarChart,
  Bar,
  XAxis,
  YAxis,
  CartesianGrid,
  Tooltip,
  Legend,
  ResponsiveContainer,
  ComposedChart,
  Line,
  Cell,
} from "recharts";
import { SlidersHorizontal } from "lucide-react";
import {
  formatCalls,
  formatNumber,
  formatCost,
  formatDate,
  formatPercent,
  getSourceColor,
  getSourceLabel,
  getVendorColor,
} from "../../lib/utils";
import type { StatsResponse } from "../../api";

const CHART_METRIC_OPTIONS = [
  { key: "cache", label: "缓存", color: "#c084fc" },
  { key: "input", label: "输入", color: "#38bdf8" },
  { key: "output", label: "输出", color: "#fb923c" },
  { key: "cacheHitRatio", label: "缓存命中率", color: "#f472b6" },
  { key: "cacheHitRatioNoXunfei", label: "缓存命中率(无讯飞)", color: "#22d3ee" },
] as const;

export type ChartMetricKey = (typeof CHART_METRIC_OPTIONS)[number]["key"];

export type VendorBreakdownMetric = "tokens" | "cost";

interface UsageSectionProps {
  stats: StatsResponse;
  hourlyStats: StatsResponse | null;
  chartMetrics: ReadonlySet<ChartMetricKey>;
  onChartMetricsChange: (metrics: Set<ChartMetricKey>) => void;
  vendorBreakdownMetric: VendorBreakdownMetric;
  onVendorBreakdownMetricChange: (m: VendorBreakdownMetric) => void;
}

function CustomTooltip({
  active,
  payload,
  label,
}: {
  active?: boolean;
  payload?: {
    name?: string;
    value?: number | string;
    color?: string;
  }[];
  label?: string;
}) {
  if (!active || !payload?.length) return null;
  return (
    <div className="bg-white border border-slate-200 rounded-lg shadow-lg p-3 text-sm">
      {label && <p className="font-semibold text-slate-700 mb-1">{label}</p>}
      {payload.map((p, i) => {
        const isRatio = p.name?.includes("命中率");
        return (
          <p key={i} className="text-slate-600">
            <span
              className="inline-block w-2 h-2 rounded-full mr-1.5"
              style={{ background: p.color }}
            />
            {p.name}:{" "}
            {isRatio
              ? formatPercent(Number(p.value ?? 0))
              : formatNumber(Number(p.value ?? 0))}
          </p>
        );
      })}
    </div>
  );
}

function VendorBreakdownTooltip({
  active,
  payload,
  metric,
}: {
  active?: boolean;
  payload?: {
    name?: string;
    value?: number | string;
    color?: string;
    payload?: { name?: string };
  }[];
  metric: VendorBreakdownMetric;
}) {
  if (!active || !payload?.length) return null;
  const p = payload[0];
  return (
    <div className="bg-white border border-slate-200 rounded-lg shadow-lg p-2 text-xs">
      <p className="font-semibold text-slate-700 mb-0.5">{p.payload?.name}</p>
      <p className="text-slate-600">
        {metric === "cost"
          ? formatCost(Number(p.value ?? 0))
          : formatNumber(Number(p.value ?? 0))}
      </p>
    </div>
  );
}

export function UsageSection({
  stats,
  hourlyStats,
  chartMetrics,
  onChartMetricsChange,
  vendorBreakdownMetric,
  onVendorBreakdownMetricChange,
}: UsageSectionProps) {
  const [showChartFilter, setShowChartFilter] = useState(false);
  const wrapperRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!showChartFilter) return;
    const handler = (e: MouseEvent) => {
      const target = e.target as Node;
      if (wrapperRef.current && !wrapperRef.current.contains(target)) {
        setShowChartFilter(false);
      }
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [showChartFilter]);

  const chartData = useMemo(() => {
    if (!stats?.by_date) return [];
    return stats.by_date.map((d) => {
      let label: string;
      if (d.date.includes(" ")) {
        const parts = d.date.split(" ");
        const datePart = parts[0].substring(5);
        const timePart = parts[1].substring(0, 5);
        label = `${datePart} ${timePart}`;
      } else {
        label = formatDate(d.date);
      }
      return {
        date: label,
        rawDate: d.date,
        calls: d.calls,
        input: d.input_tokens,
        output: d.output_tokens,
        cacheRead: d.cache_read_tokens,
        cacheWrite: d.cache_write_tokens,
        cache: d.cache_read_tokens + d.cache_write_tokens,
        total: d.total_tokens,
        cost: d.cost,
        cacheHitRatio: d.cache_hit_ratio,
        cacheHitRatioNoXunfei: d.cache_hit_ratio_no_xunfei,
      };
    });
  }, [stats]);

  const vendorChartData = useMemo(() => {
    if (!stats?.by_vendor) return [];
    return stats.by_vendor.map((v) => ({
      name: v.provider,
      tokens: v.total_tokens,
      calls: v.calls,
      cost: v.cost,
      cacheHit: v.cache_hit_ratio,
    }));
  }, [stats]);

  const hourlyData = useMemo(() => {
    if (!hourlyStats?.by_date) return [];
    return hourlyStats.by_date.map((d) => {
      let label: string;
      if (d.date.includes(" ")) {
        const parts = d.date.split(" ");
        const datePart = parts[0].substring(5);
        const timePart = parts[1].substring(0, 5);
        label = `${datePart} ${timePart}`;
      } else {
        label = formatDate(d.date);
      }
      return { date: label, calls: d.calls };
    });
  }, [hourlyStats]);

  const showRatioAxis =
    chartMetrics.has("cacheHitRatio") || chartMetrics.has("cacheHitRatioNoXunfei");

  const toggleMetric = (key: ChartMetricKey) => {
    const next = new Set(chartMetrics);
    if (next.has(key)) next.delete(key);
    else next.add(key);
    onChartMetricsChange(next);
  };

  return (
    <section id="section-usage" className="space-y-3 scroll-mt-32">
      <h2 className="text-base font-semibold text-slate-800">用量</h2>

      {/* Daily Token Usage */}
      <div className="bg-white rounded-xl border border-slate-200 p-4 shadow-sm">
        <div className="flex items-center justify-between mb-2">
          <h3 className="text-sm font-semibold text-slate-700">每日 Token 用量</h3>
          <div ref={wrapperRef} className="relative">
            <button
              onClick={() => setShowChartFilter((v) => !v)}
              className={`inline-flex items-center gap-1 px-2 py-1 text-[11px] font-medium rounded-md transition-colors ${
                showChartFilter
                  ? "bg-primary-100 text-primary-700"
                  : "bg-slate-50 text-slate-500 hover:bg-slate-100"
              }`}
            >
              <SlidersHorizontal className="w-3 h-3" />
              图表指标
            </button>
            {showChartFilter && (
              <div className="absolute right-0 top-full mt-1 bg-white border border-slate-200 rounded-lg shadow-xl p-1.5 min-w-[180px] z-30">
                {CHART_METRIC_OPTIONS.map((opt) => (
                  <label
                    key={opt.key}
                    className="flex items-center gap-2 px-2 py-1.5 rounded hover:bg-slate-50 cursor-pointer"
                  >
                    <input
                      type="checkbox"
                      checked={chartMetrics.has(opt.key)}
                      onChange={() => toggleMetric(opt.key)}
                      className="rounded border-slate-300 text-primary-600 focus:ring-primary-500"
                    />
                    <span
                      className="w-2 h-2 rounded-full shrink-0"
                      style={{ background: opt.color }}
                    />
                    <span className="text-xs text-slate-700">{opt.label}</span>
                  </label>
                ))}
              </div>
            )}
          </div>
        </div>
        <ResponsiveContainer width="100%" height={340}>
          <ComposedChart data={chartData}>
            <CartesianGrid strokeDasharray="2 2" stroke="#f1f5f9" />
            <XAxis
              dataKey="date"
              tick={{ fontSize: 10, fill: "#64748b" }}
              angle={-30}
              textAnchor="end"
              height={50}
            />
            <YAxis
              yAxisId="tokens"
              tick={{ fontSize: 10, fill: "#64748b" }}
              tickFormatter={(v: number) => formatNumber(v)}
              width={50}
            />
            {showRatioAxis && (
              <YAxis
                yAxisId="ratio"
                orientation="right"
                tick={{ fontSize: 10, fill: "#f472b6" }}
                domain={[0, 100]}
                unit="%"
                width={40}
              />
            )}
            <Tooltip content={<CustomTooltip />} />
            <Legend wrapperStyle={{ fontSize: 11 }} />
            {chartMetrics.has("cache") && (
              <Bar
                yAxisId="tokens"
                dataKey="cache"
                name="缓存"
                stackId="tokens"
                fill="#c084fc"
              />
            )}
            {chartMetrics.has("input") && (
              <Bar
                yAxisId="tokens"
                dataKey="input"
                name="输入"
                stackId="tokens"
                fill="#38bdf8"
              />
            )}
            {chartMetrics.has("output") && (
              <Bar
                yAxisId="tokens"
                dataKey="output"
                name="输出"
                stackId="tokens"
                fill="#fb923c"
              />
            )}
            {chartMetrics.has("cacheHitRatio") && showRatioAxis && (
              <Line
                yAxisId="ratio"
                type="monotone"
                dataKey="cacheHitRatio"
                name="缓存命中率"
                stroke="#f472b6"
                strokeWidth={2}
                strokeDasharray="6 3"
                dot={{ r: 2 }}
              />
            )}
            {chartMetrics.has("cacheHitRatioNoXunfei") && showRatioAxis && (
              <Line
                yAxisId="ratio"
                type="monotone"
                dataKey="cacheHitRatioNoXunfei"
                name="缓存命中率(无讯飞)"
                stroke="#22d3ee"
                strokeWidth={1.5}
                strokeDasharray="4 2"
                dot={{ r: 1.5 }}
              />
            )}
          </ComposedChart>
        </ResponsiveContainer>
        <p className="text-[10px] text-slate-400 mt-1">* 讯飞无缓存机制</p>
      </div>

      {/* Hourly + Vendor Breakdown */}
      <div className="grid grid-cols-1 lg:grid-cols-2 gap-3">
        <div className="bg-white rounded-xl border border-slate-200 p-4 shadow-sm">
          <h3 className="text-sm font-semibold text-slate-700 mb-2">
            每小时请求数
          </h3>
          <ResponsiveContainer width="100%" height={240}>
            <BarChart data={hourlyData}>
              <CartesianGrid strokeDasharray="2 2" stroke="#f1f5f9" />
              <XAxis
                dataKey="date"
                tick={{ fontSize: 10, fill: "#64748b" }}
                angle={-30}
                textAnchor="end"
                height={50}
              />
              <YAxis
                tick={{ fontSize: 10, fill: "#64748b" }}
                tickFormatter={(v: number) => formatNumber(v)}
                width={40}
              />
              <Tooltip content={<CustomTooltip />} />
              <Bar
                dataKey="calls"
                name="调用次数"
                fill="#2dd4bf"
                radius={[4, 4, 0, 0]}
              />
            </BarChart>
          </ResponsiveContainer>
        </div>

        <div className="bg-white rounded-xl border border-slate-200 p-4 shadow-sm">
          <div className="flex items-center justify-between mb-2">
            <h3 className="text-sm font-semibold text-slate-700">
              供应商分布
            </h3>
            <div className="inline-flex items-center bg-slate-100 rounded-md p-0.5 gap-0.5">
              <button
                onClick={() => onVendorBreakdownMetricChange("tokens")}
                className={`px-2 py-0.5 text-[10px] font-medium rounded transition-colors ${
                  vendorBreakdownMetric === "tokens"
                    ? "bg-white text-primary-700 shadow-sm"
                    : "text-slate-500"
                }`}
              >
                Token
              </button>
              <button
                onClick={() => onVendorBreakdownMetricChange("cost")}
                className={`px-2 py-0.5 text-[10px] font-medium rounded transition-colors ${
                  vendorBreakdownMetric === "cost"
                    ? "bg-white text-primary-700 shadow-sm"
                    : "text-slate-500"
                }`}
              >
                费用
              </button>
            </div>
          </div>
          <ResponsiveContainer width="100%" height={240}>
            <BarChart data={vendorChartData} layout="vertical">
              <CartesianGrid strokeDasharray="2 2" stroke="#f1f5f9" />
              <XAxis
                type="number"
                tick={{ fontSize: 10, fill: "#64748b" }}
                tickFormatter={(v: number) =>
                  vendorBreakdownMetric === "cost"
                    ? formatCost(v)
                    : formatNumber(v)
                }
              />
              <YAxis
                type="category"
                dataKey="name"
                tick={{ fontSize: 10, fill: "#64748b" }}
                width={80}
              />
              <Tooltip
                content={<VendorBreakdownTooltip metric={vendorBreakdownMetric} />}
              />
              <Bar
                dataKey={vendorBreakdownMetric}
                name={vendorBreakdownMetric === "cost" ? "费用" : "总 Token"}
                radius={[0, 4, 4, 0]}
              >
                {vendorChartData.map((entry, index) => (
                  <Cell
                    key={`cell-${index}`}
                    fill={getVendorColor(entry.name)}
                  />
                ))}
              </Bar>
            </BarChart>
          </ResponsiveContainer>
        </div>
      </div>

      {/* Per-source grid */}
      {stats.by_source.length > 0 && (
        <div>
          <h3 className="text-sm font-semibold text-slate-700 mb-2">
            分工具明细
          </h3>
          <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-2">
            {stats.by_source.map((s) => (
              <div
                key={s.source}
                className="bg-white rounded-lg border border-slate-200 p-3"
              >
                <div className="flex items-center gap-1.5 mb-2">
                  <span
                    className="w-2 h-2 rounded-full"
                    style={{ background: getSourceColor(s.source) }}
                  />
                  <span
                    className="text-xs font-semibold"
                    style={{ color: getSourceColor(s.source) }}
                  >
                    {getSourceLabel(s.source)}
                  </span>
                </div>
                <div className="grid grid-cols-2 gap-2">
                  <Metric label="调用" value={formatCalls(s.calls)} />
                  <Metric
                    label="Token"
                    value={formatNumber(s.total_tokens)}
                  />
                  <Metric label="费用" value={formatCost(s.cost, s.source)} />
                  <Metric
                    label="命中率"
                    value={formatPercent(s.cache_hit_ratio)}
                  />
                </div>
              </div>
            ))}
          </div>
        </div>
      )}
    </section>
  );
}

function Metric({ label, value }: { label: string; value: string }) {
  return (
    <div className="min-w-0">
      <p className="text-[10px] text-slate-400 font-medium">{label}</p>
      <p className="text-sm font-bold text-slate-800 tabular-nums truncate">
        {value}
      </p>
    </div>
  );
}
