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
  AreaChart,
  Area,
  ReferenceLine,
} from "recharts";
import { SlidersHorizontal, ChevronLeft, ChevronRight } from "lucide-react";
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
import type { StatsResponse, RpmAnalysis } from "../../api";

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
  rpmData: RpmAnalysis | null;
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

function RpmTooltip({
  active,
  payload,
  label,
}: {
  active?: boolean;
  payload?: { value?: number | string; name?: string }[];
  label?: string;
}) {
  if (!active || !payload?.length) return null;
  return (
    <div className="bg-white border border-slate-200 rounded-lg shadow-lg p-2 text-xs">
      <p className="font-semibold text-slate-700 mb-0.5">{label}</p>
      <p className="text-indigo-600">
        {Number(payload[0].value ?? 0)} 请求/分钟
      </p>
    </div>
  );
}

export function UsageSection({
  stats,
  hourlyStats,
  rpmData,
  chartMetrics,
  onChartMetricsChange,
  vendorBreakdownMetric,
  onVendorBreakdownMetricChange,
}: UsageSectionProps) {
  const [showChartFilter, setShowChartFilter] = useState(false);
  const [windowPage, setWindowPage] = useState(1);
  const WINDOW_PAGE_SIZE = 10;
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

  const rpmChartData = useMemo(() => {
    if (!rpmData?.all_buckets) return [];
    // Pre-compute window start set for O(1) lookup instead of O(n*m) .some()
    const windowStarts = new Set(rpmData.windows.map((w) => w.start));
    return rpmData.all_buckets.map((b) => {
      // Format: "2026-05-17 10:30" → "05-17 10:30"
      const label = b.minute.includes(" ")
        ? `${b.minute.substring(5)}`
        : b.minute;
      return {
        minute: label,
        requests: b.requests,
        // Mark window boundaries for visual separation
        isWindowStart: windowStarts.has(b.minute),
      };
    });
  }, [rpmData]);

  const rpmWindowSummaries = useMemo(() => {
    if (!rpmData?.windows) return [];
    return rpmData.windows.map((w, i) => ({
      id: i + 1,
      start: w.start.substring(5), // "05-17 10:30"
      end: w.end.substring(5),
      duration: w.duration_minutes,
      total: w.total_requests,
      avgRpm: w.avg_rpm,
      peakRpm: w.peak_rpm,
    }));
  }, [rpmData]);

  // Reset window page when data changes
  const totalWindowPages = Math.ceil(rpmWindowSummaries.length / WINDOW_PAGE_SIZE);
  useEffect(() => {
    setWindowPage(1);
  }, [rpmData]);
  useEffect(() => {
    if (windowPage > totalWindowPages && totalWindowPages > 0) {
      setWindowPage(totalWindowPages);
    }
  }, [windowPage, totalWindowPages]);

  const pagedWindowSummaries = useMemo(() => {
    const start = (windowPage - 1) * WINDOW_PAGE_SIZE;
    return rpmWindowSummaries.slice(start, start + WINDOW_PAGE_SIZE);
  }, [rpmWindowSummaries, windowPage]);

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

      {/* RPM Analysis */}
      {rpmData && rpmData.all_buckets.length > 0 && (
        <div className="bg-white rounded-xl border border-slate-200 p-4 shadow-sm">
          <div className="flex items-center justify-between mb-2">
            <h3 className="text-sm font-semibold text-slate-700">
              每分钟请求数 (RPM)
            </h3>
            <div className="flex items-center gap-3 text-[10px] text-slate-500">
              <span>平均 <b className="text-slate-700">{rpmData.overall_avg_rpm.toFixed(1)}</b> RPM</span>
              <span>峰值 <b className="text-rose-600">{rpmData.overall_peak_rpm}</b> RPM</span>
              <span>活跃 <b className="text-slate-700">{rpmData.total_active_minutes}</b> 分钟</span>
              <span>活跃窗口 <b className="text-slate-700">{rpmData.windows.length}</b></span>
            </div>
          </div>
          <ResponsiveContainer width="100%" height={300}>
            <AreaChart data={rpmChartData}>
              <CartesianGrid strokeDasharray="2 2" stroke="#f1f5f9" />
              <XAxis
                dataKey="minute"
                tick={{ fontSize: 9, fill: "#64748b" }}
                angle={-45}
                textAnchor="end"
                height={60}
                interval={Math.max(0, Math.floor(rpmChartData.length / 40) - 1)}
              />
              <YAxis
                tick={{ fontSize: 10, fill: "#64748b" }}
                width={35}
                allowDecimals={false}
              />
              <Tooltip content={<RpmTooltip />} />
              {rpmData.overall_avg_rpm > 0 && (
                <ReferenceLine
                  y={rpmData.overall_avg_rpm}
                  stroke="#f59e0b"
                  strokeDasharray="4 2"
                  strokeWidth={1}
                  label={{
                    value: `平均 ${rpmData.overall_avg_rpm.toFixed(1)}`,
                    position: "insideTopRight",
                    fill: "#f59e0b",
                    fontSize: 10,
                  }}
                />
              )}
              <Area
                type="stepAfter"
                dataKey="requests"
                name="请求数"
                stroke="#6366f1"
                fill="#6366f1"
                fillOpacity={0.15}
                strokeWidth={1.5}
              />
              {/* Draw window boundary lines */}
              {rpmData.windows.slice(1).map((w, i) => (
                <ReferenceLine
                  key={`window-boundary-${i}`}
                  x={w.start.substring(5)}
                  stroke="#94a3b8"
                  strokeDasharray="2 4"
                  strokeWidth={1}
                />
              ))}
            </AreaChart>
          </ResponsiveContainer>
          {rpmWindowSummaries.length > 1 && (
            <div className="mt-3">
              <div className="flex items-center justify-between mb-1.5">
                <p className="text-[10px] text-slate-400 font-medium">活跃窗口明细</p>
                {totalWindowPages > 1 && (
                  <div className="flex items-center gap-1.5 text-[10px] text-slate-500">
                    <span>
                      {(windowPage - 1) * WINDOW_PAGE_SIZE + 1}-
                      {Math.min(windowPage * WINDOW_PAGE_SIZE, rpmWindowSummaries.length)} / {rpmWindowSummaries.length}
                    </span>
                    <button
                      onClick={() => setWindowPage(Math.max(1, windowPage - 1))}
                      disabled={windowPage <= 1}
                      className="p-1 rounded hover:bg-slate-100 disabled:opacity-40 disabled:cursor-not-allowed transition-colors"
                      title="上一页"
                    >
                      <ChevronLeft className="w-3 h-3" />
                    </button>
                    <button
                      onClick={() => setWindowPage(Math.min(totalWindowPages, windowPage + 1))}
                      disabled={windowPage >= totalWindowPages}
                      className="p-1 rounded hover:bg-slate-100 disabled:opacity-40 disabled:cursor-not-allowed transition-colors"
                      title="下一页"
                    >
                      <ChevronRight className="w-3 h-3" />
                    </button>
                  </div>
                )}
              </div>
              <div className="overflow-x-auto">
                <table className="w-full text-[10px]">
                  <thead>
                    <tr className="text-slate-500 border-b border-slate-100">
                      <th className="text-left py-1 pr-3 font-medium">#</th>
                      <th className="text-left py-1 pr-3 font-medium">开始</th>
                      <th className="text-left py-1 pr-3 font-medium">结束</th>
                      <th className="text-right py-1 pr-3 font-medium">时长</th>
                      <th className="text-right py-1 pr-3 font-medium">请求数</th>
                      <th className="text-right py-1 pr-3 font-medium">平均 RPM</th>
                      <th className="text-right py-1 font-medium">峰值 RPM</th>
                    </tr>
                  </thead>
                  <tbody>
                    {pagedWindowSummaries.map((w) => (
                      <tr key={w.id} className="border-b border-slate-50">
                        <td className="py-1 pr-3 text-slate-400">{w.id}</td>
                        <td className="py-1 pr-3 text-slate-700 font-mono">{w.start}</td>
                        <td className="py-1 pr-3 text-slate-700 font-mono">{w.end}</td>
                        <td className="py-1 pr-3 text-right text-slate-600">{w.duration} 分钟</td>
                        <td className="py-1 pr-3 text-right text-slate-600">{w.total}</td>
                        <td className="py-1 pr-3 text-right text-indigo-600 font-semibold">{w.avgRpm.toFixed(1)}</td>
                        <td className="py-1 text-right text-rose-600 font-semibold">{w.peakRpm}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
              <p className="text-[9px] text-slate-400 mt-1">
                * 间隔 ≥ {rpmData.gap_threshold_minutes} 分钟无请求时视为窗口边界
              </p>
            </div>
          )}
        </div>
      )}

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
