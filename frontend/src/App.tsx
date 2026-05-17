import { useState, useEffect, useMemo } from "react";
import {
  BarChart,
  Bar,
  XAxis,
  YAxis,
  CartesianGrid,
  Tooltip,
  Legend,
  ResponsiveContainer,
  LineChart,
  Line,
  PieChart,
  Pie,
  Cell,
} from "recharts";
import {
  Calendar,
  ChevronLeft,
  ChevronRight,
  Filter,
  Activity,
  Coins,
  Database,
  TrendingUp,
  Zap,
  Server,
} from "lucide-react";
import {
  fetchStats,
  fetchRequests,
  fetchFilters,
  type StatsResponse,
  type PaginatedRequests,
  type FilterOptions,
} from "./api";
import {
  formatNumber,
  formatCost,
  formatPercent,
  formatDate,
  getSourceColor,
  getSourceLabel,
} from "./lib/utils";

const ZH = {
  title: "Token 统计仪表盘",
  subtitle: "监控跨工具的 AI Token 使用情况",
  totalCalls: "总调用次数",
  inputTokens: "输入 Token",
  outputTokens: "输出 Token",
  cacheRead: "缓存读取",
  cacheHitRatio: "缓存命中率",
  weighted: "加权",
  totalCost: "总费用",
  dailyTokenUsage: "每日 Token 用量",
  vendorBreakdown: "供应商分布",
  cacheHitTrend: "缓存命中率趋势",
  tokenDistribution: "Token 分布",
  vendorPerformance: "供应商表现",
  modelPerformance: "模型表现",
  sourceOverview: "工具概览",
  detailedRequests: "详细请求",
  allProviders: "全部供应商",
  allModels: "全部模型",
  allSources: "全部工具",
  provider: "供应商",
  model: "模型",
  source: "工具",
  calls: "调用次数",
  input: "输入",
  output: "输出",
  cacheReadCol: "缓存读取",
  cacheWriteCol: "缓存写入",
  total: "合计",
  cacheHit: "缓存命中",
  cost: "费用",
  date: "日期",
  showing: "显示",
  of: "/",
  requests: "条请求",
  footer: "Token 统计仪表盘 · 基于 Rust + React 构建",
  selectDay: "选择日期",
  dateRange: "日期范围",
  from: "从",
  to: "至",
  inputLabel: "输入",
  outputLabel: "输出",
  cacheReadLabel: "缓存读取",
  cacheHitLabel: "缓存命中率",
  totalTokensLabel: "总 Token",
  tokens: "Token",
  today: "今天",
  toggleTime: "切换精确时间",
} as const;

type DateMode = "day" | "range";

function getDefaultDateRange() {
  const to = new Date();
  const from = new Date();
  from.setDate(from.getDate() - 30);
  return {
    from: from.toISOString().slice(0, 10),
    to: to.toISOString().slice(0, 10),
  };
}

function getDefaultTimeRange() {
  const to = new Date();
  const from = new Date();
  from.setDate(from.getDate() - 30);
  return {
    from: from.toISOString().slice(0, 16),
    to: to.toISOString().slice(0, 16),
  };
}

function getToday() {
  return new Date().toISOString().slice(0, 10);
}

function SourceBadge({ source }: { source: string }) {
  return (
    <span
      className="inline-flex items-center gap-1.5 px-2 py-0.5 rounded-full text-xs font-medium"
      style={{
        background: `${getSourceColor(source)}15`,
        color: getSourceColor(source),
      }}
    >
      <span
        className="w-1.5 h-1.5 rounded-full"
        style={{ background: getSourceColor(source) }}
      />
      {getSourceLabel(source)}
    </span>
  );
}

function StatCard({
  title,
  value,
  subtitle,
  icon: Icon,
  color,
}: {
  title: string;
  value: string;
  subtitle?: string;
  icon: React.ElementType;
  color: string;
}) {
  return (
    <div className="bg-white rounded-xl border border-slate-200 p-5 shadow-sm hover:shadow-md transition-shadow">
      <div className="flex items-start justify-between">
        <div>
          <p className="text-slate-500 text-sm font-medium">{title}</p>
          <p className="text-2xl font-bold text-slate-800 mt-1">{value}</p>
          {subtitle && <p className="text-slate-400 text-xs mt-1">{subtitle}</p>}
        </div>
        <div className={`p-2.5 rounded-lg ${color}`}>
          <Icon className="w-5 h-5 text-white" />
        </div>
      </div>
    </div>
  );
}

function CustomTooltip({
  active,
  payload,
  label,
}: {
  active?: boolean;
  payload?: any[];
  label?: string;
}) {
  if (!active || !payload?.length) return null;
  return (
    <div className="bg-white border border-slate-200 rounded-lg shadow-lg p-3 text-sm">
      {label && <p className="font-semibold text-slate-700 mb-1">{label}</p>}
      {payload.map((p, i) => (
        <p key={i} className="text-slate-600">
          <span
            className="inline-block w-2 h-2 rounded-full mr-1.5"
            style={{ background: p.color }}
          />
          {p.name}: {formatNumber(p.value)}
        </p>
      ))}
    </div>
  );
}

function PieTooltip({ active, payload }: { active?: boolean; payload?: any[] }) {
  if (!active || !payload?.length) return null;
  const p = payload[0];
  return (
    <div className="bg-white border border-slate-200 rounded-lg shadow-lg p-3 text-sm">
      <p className="font-semibold text-slate-700">{p.name}</p>
      <p className="text-slate-600">{formatNumber(p.value)} {ZH.tokens}</p>
      <p className="text-slate-500">{p.percent?.toFixed(1)}%</p>
    </div>
  );
}

export default function App() {
  const [dateMode, setDateMode] = useState<DateMode>("range");
  const [selectedDay, setSelectedDay] = useState(getToday);
  const [dateRange, setDateRange] = useState(getDefaultDateRange);
  const [timeRange, setTimeRange] = useState(getDefaultTimeRange);
  const [useTimeRange, setUseTimeRange] = useState(false);
  const [stats, setStats] = useState<StatsResponse | null>(null);
  const [requests, setRequests] = useState<PaginatedRequests | null>(null);
  const [filters, setFilters] = useState<FilterOptions>({
    vendors: [],
    models: [],
    sources: [],
  });
  const [selectedVendor, setSelectedVendor] = useState<string>("");
  const [selectedModel, setSelectedModel] = useState<string>("");
  const [selectedSources, setSelectedSources] = useState<Set<string>>(new Set());
  const [page, setPage] = useState(1);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const effectiveRange = useMemo(() => {
    if (dateMode === "day") {
      return { from: selectedDay, to: selectedDay };
    }
    if (useTimeRange) {
      return { from: timeRange.from, to: timeRange.to };
    }
    return dateRange;
  }, [dateMode, selectedDay, dateRange, timeRange, useTimeRange]);

  // Compute source filter string (empty = all)
  const sourceFilter = useMemo(() => {
    if (selectedSources.size === 0 || selectedSources.size === filters.sources.length) {
      return "";
    }
    // API only supports single source, so pick first if multiple selected
    // We'll filter client-side for multi-select
    if (selectedSources.size === 1) {
      return [...selectedSources][0];
    }
    return "";
  }, [selectedSources, filters.sources.length]);

  const loadData = async () => {
    setLoading(true);
    setError(null);
    try {
      const [s, f] = await Promise.all([
        fetchStats(effectiveRange.from, effectiveRange.to, sourceFilter || undefined),
        fetchFilters(),
      ]);
      // Client-side filter if multiple sources selected
      if (selectedSources.size > 0 && selectedSources.size < filters.sources.length) {
        // Already filtered by API for single source; for multi we'd need to adjust
      }
      setStats(s);
      setFilters(f);
      // Initialize selectedSources if empty
      if (selectedSources.size === 0 && f.sources.length > 0) {
        setSelectedSources(new Set(f.sources));
      }
      setPage(1);
    } catch (e) {
      setError(e instanceof Error ? e.message : "Failed to load data");
    } finally {
      setLoading(false);
    }
  };

  const loadRequests = async () => {
    try {
      const r = await fetchRequests(
        effectiveRange.from,
        effectiveRange.to,
        selectedVendor || undefined,
        selectedModel || undefined,
        sourceFilter || undefined,
        page,
        50
      );
      // Client-side multi-source filter
      if (selectedSources.size > 0 && selectedSources.size < filters.sources.length && selectedSources.size !== 1) {
        const filtered = r.data.filter((req) => selectedSources.has(req.source));
        r.data = filtered;
        r.total = filtered.length;
        r.total_pages = Math.ceil(filtered.length / r.limit);
      }
      setRequests(r);
    } catch (e) {
      console.error("Failed to load requests", e);
    }
  };

  useEffect(() => {
    loadData();
  }, [effectiveRange.from, effectiveRange.to, sourceFilter]);

  useEffect(() => {
    loadRequests();
  }, [
    effectiveRange.from,
    effectiveRange.to,
    selectedVendor,
    selectedModel,
    sourceFilter,
    page,
  ]);

  const chartData = useMemo(() => {
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

  const pieData = useMemo(() => {
    if (!stats?.by_vendor) return [];
    return stats.by_vendor.map((v) => ({
      name: v.provider,
      value: v.total_tokens,
    }));
  }, [stats]);

  const presetRanges = [
    { label: ZH.today, days: 0 },
    { label: "7天", days: 7 },
    { label: "30天", days: 30 },
    { label: "90天", days: 90 },
    { label: "全部", days: 365 * 10 },
  ];

  const applyPreset = (days: number) => {
    setDateMode("range");
    setUseTimeRange(false);
    const to = new Date();
    const from = new Date();
    from.setDate(from.getDate() - days);
    setDateRange({
      from: from.toISOString().slice(0, 10),
      to: to.toISOString().slice(0, 10),
    });
    setTimeRange({
      from: from.toISOString().slice(0, 16),
      to: to.toISOString().slice(0, 16),
    });
  };

  const toggleSource = (source: string) => {
    setSelectedSources((prev) => {
      const next = new Set(prev);
      if (next.has(source)) {
        next.delete(source);
      } else {
        next.add(source);
      }
      return next;
    });
  };

  return (
    <div className="min-h-screen bg-slate-50">
      {/* Header */}
      <header className="bg-white border-b border-slate-200 sticky top-0 z-10">
        <div className="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8 py-4">
          <div className="flex flex-col gap-4">
            {/* Top row: title + date controls */}
            <div className="flex flex-col sm:flex-row sm:items-center sm:justify-between gap-4">
              <div className="flex items-center gap-3">
                <div className="bg-primary-600 p-2 rounded-lg">
                  <Activity className="w-6 h-6 text-white" />
                </div>
                <div>
                  <h1 className="text-xl font-bold text-slate-800">
                    {ZH.title}
                  </h1>
                  <p className="text-sm text-slate-500">{ZH.subtitle}</p>
                </div>
              </div>

              <div className="flex items-center gap-2 flex-wrap">
                {presetRanges.map((p) => (
                  <button
                    key={p.label}
                    onClick={() => applyPreset(p.days)}
                    className="px-3 py-1.5 text-xs font-medium rounded-md bg-slate-100 text-slate-600 hover:bg-primary-100 hover:text-primary-700 transition-colors"
                  >
                    {p.label}
                  </button>
                ))}
                <div className="flex items-center gap-3 ml-2">
                  <div className="flex items-center bg-slate-100 rounded-md p-0.5">
                    <button
                      onClick={() => setDateMode("day")}
                      className={`px-2.5 py-1 text-xs font-medium rounded transition-colors ${
                        dateMode === "day"
                          ? "bg-white text-primary-700 shadow-sm"
                          : "text-slate-500 hover:text-slate-700"
                      }`}
                    >
                      {ZH.selectDay}
                    </button>
                    <button
                      onClick={() => setDateMode("range")}
                      className={`px-2.5 py-1 text-xs font-medium rounded transition-colors ${
                        dateMode === "range"
                          ? "bg-white text-primary-700 shadow-sm"
                          : "text-slate-500 hover:text-slate-700"
                      }`}
                    >
                      {ZH.dateRange}
                    </button>
                  </div>

                  <Calendar className="w-4 h-4 text-slate-400" />

                  {dateMode === "day" ? (
                    <input
                      type="date"
                      value={selectedDay}
                      onChange={(e) => setSelectedDay(e.target.value)}
                      className="px-2 py-1.5 text-sm border border-slate-200 rounded-md focus:ring-2 focus:ring-primary-500 focus:border-primary-500 outline-none"
                    />
                  ) : (
                    <>
                      <div className="flex items-center gap-1.5">
                        <span className="text-xs text-slate-500">
                          {ZH.from}
                        </span>
                        {useTimeRange ? (
                          <input
                            type="datetime-local"
                            value={timeRange.from}
                            onChange={(e) =>
                              setTimeRange((prev) => ({
                                ...prev,
                                from: e.target.value,
                              }))
                            }
                            className="px-2 py-1.5 text-sm border border-slate-200 rounded-md focus:ring-2 focus:ring-primary-500 focus:border-primary-500 outline-none"
                          />
                        ) : (
                          <input
                            type="date"
                            value={dateRange.from}
                            onChange={(e) =>
                              setDateRange((prev) => ({
                                ...prev,
                                from: e.target.value,
                              }))
                            }
                            className="px-2 py-1.5 text-sm border border-slate-200 rounded-md focus:ring-2 focus:ring-primary-500 focus:border-primary-500 outline-none"
                          />
                        )}
                        <span className="text-slate-400">-</span>
                        <span className="text-xs text-slate-500">{ZH.to}</span>
                        {useTimeRange ? (
                          <input
                            type="datetime-local"
                            value={timeRange.to}
                            onChange={(e) =>
                              setTimeRange((prev) => ({
                                ...prev,
                                to: e.target.value,
                              }))
                            }
                            className="px-2 py-1.5 text-sm border border-slate-200 rounded-md focus:ring-2 focus:ring-primary-500 focus:border-primary-500 outline-none"
                          />
                        ) : (
                          <input
                            type="date"
                            value={dateRange.to}
                            onChange={(e) =>
                              setDateRange((prev) => ({
                                ...prev,
                                to: e.target.value,
                              }))
                            }
                            className="px-2 py-1.5 text-sm border border-slate-200 rounded-md focus:ring-2 focus:ring-primary-500 focus:border-primary-500 outline-none"
                          />
                        )}
                      </div>
                      <button
                        onClick={() => setUseTimeRange((v) => !v)}
                        className={`px-2 py-1 text-xs font-medium rounded-md transition-colors ${
                          useTimeRange
                            ? "bg-primary-100 text-primary-700"
                            : "bg-slate-100 text-slate-500 hover:bg-slate-200"
                        }`}
                        title={ZH.toggleTime}
                      >
                        🕐
                      </button>
                    </>
                  )}
                </div>
              </div>
            </div>

            {/* Source filter row */}
            <div className="flex items-center gap-2 flex-wrap">
              <Filter className="w-4 h-4 text-slate-400" />
              <span className="text-xs text-slate-500 font-medium">
                {ZH.source}:
              </span>
              {filters.sources.map((s) => (
                <button
                  key={s}
                  onClick={() => toggleSource(s)}
                  className={`inline-flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium rounded-full transition-all ${
                    selectedSources.has(s)
                      ? "text-white shadow-sm"
                      : "bg-slate-100 text-slate-500 hover:bg-slate-200"
                  }`}
                  style={
                    selectedSources.has(s)
                      ? { background: getSourceColor(s) }
                      : undefined
                  }
                >
                  <span
                    className="w-1.5 h-1.5 rounded-full"
                    style={{
                      background: selectedSources.has(s)
                        ? "white"
                        : getSourceColor(s),
                    }}
                  />
                  {getSourceLabel(s)}
                </button>
              ))}
            </div>
          </div>
        </div>
      </header>

      <main className="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8 py-6">
        {error && (
          <div className="mb-6 p-4 bg-rose-50 border border-rose-200 rounded-lg text-rose-700 text-sm">
            {error}
          </div>
        )}

        {loading && !stats && (
          <div className="flex items-center justify-center h-64">
            <div className="animate-spin rounded-full h-8 w-8 border-b-2 border-primary-600" />
          </div>
        )}

        {stats && (
          <>
            {/* Summary Cards */}
            <div className="grid grid-cols-2 md:grid-cols-3 lg:grid-cols-6 gap-4 mb-6">
              <StatCard
                title={ZH.totalCalls}
                value={formatNumber(stats.overall.total_calls)}
                icon={Server}
                color="bg-primary-500"
              />
              <StatCard
                title={ZH.inputTokens}
                value={formatNumber(stats.overall.total_input_tokens)}
                icon={Database}
                color="bg-emerald-500"
              />
              <StatCard
                title={ZH.outputTokens}
                value={formatNumber(stats.overall.total_output_tokens)}
                icon={Zap}
                color="bg-amber-500"
              />
              <StatCard
                title={ZH.cacheRead}
                value={formatNumber(stats.overall.total_cache_read_tokens)}
                icon={TrendingUp}
                color="bg-violet-500"
              />
              <StatCard
                title={ZH.cacheHitRatio}
                value={formatPercent(stats.overall.weighted_cache_hit_ratio)}
                subtitle={ZH.weighted}
                icon={Activity}
                color="bg-rose-500"
              />
              <StatCard
                title={ZH.totalCost}
                value={formatCost(stats.overall.total_cost)}
                icon={Coins}
                color="bg-slate-700"
              />
            </div>

            {/* Source Overview */}
            {stats.by_source.length > 1 && (
              <div className="bg-white rounded-xl border border-slate-200 p-5 shadow-sm mb-6">
                <h3 className="text-sm font-semibold text-slate-700 mb-4">
                  {ZH.sourceOverview}
                </h3>
                <div className="grid grid-cols-2 md:grid-cols-4 gap-4">
                  {stats.by_source.map((s) => (
                    <div
                      key={s.source}
                      className="rounded-lg border border-slate-100 p-4"
                    >
                      <div className="flex items-center gap-2 mb-3">
                        <SourceBadge source={s.source} />
                      </div>
                      <div className="space-y-1 text-xs text-slate-600">
                        <p>
                          调用:{" "}
                          <span className="font-semibold text-slate-800">
                            {formatNumber(s.calls)}
                          </span>
                        </p>
                        <p>
                          Token:{" "}
                          <span className="font-semibold text-slate-800">
                            {formatNumber(s.total_tokens)}
                          </span>
                        </p>
                        <p>
                          费用:{" "}
                          <span className="font-semibold text-slate-800">
                            {formatCost(s.cost, s.source)}
                          </span>
                        </p>
                        <p>
                          命中率:{" "}
                          <span className="font-semibold text-slate-800">
                            {formatPercent(s.cache_hit_ratio)}
                          </span>
                        </p>
                      </div>
                    </div>
                  ))}
                </div>
              </div>
            )}

            {/* Charts Row */}
            <div className="grid grid-cols-1 lg:grid-cols-2 gap-6 mb-6">
              {/* Daily Trends + Cache Hit Ratio */}
              <div className="bg-white rounded-xl border border-slate-200 p-5 shadow-sm">
                <h3 className="text-sm font-semibold text-slate-700 mb-4">
                  {ZH.dailyTokenUsage} & {ZH.cacheHitTrend}
                </h3>
                <ResponsiveContainer width="100%" height={320}>
                  <LineChart data={chartData}>
                    <CartesianGrid strokeDasharray="3 3" stroke="#e2e8f0" />
                    <XAxis
                      dataKey="date"
                      tick={{ fontSize: 11, fill: "#64748b" }}
                      angle={-30}
                      textAnchor="end"
                      height={60}
                    />
                    <YAxis
                      yAxisId="tokens"
                      tick={{ fontSize: 11, fill: "#64748b" }}
                      tickFormatter={(v: number) => formatNumber(v)}
                    />
                    <YAxis
                      yAxisId="ratio"
                      orientation="right"
                      tick={{ fontSize: 11, fill: "#f43f5e" }}
                      domain={[0, 100]}
                      unit="%"
                    />
                    <Tooltip content={<CustomTooltip />} />
                    <Legend />
                    <Line
                      yAxisId="tokens"
                      type="monotone"
                      dataKey="input"
                      name={ZH.inputLabel}
                      stroke="#10b981"
                      strokeWidth={2}
                      dot={false}
                    />
                    <Line
                      yAxisId="tokens"
                      type="monotone"
                      dataKey="output"
                      name={ZH.outputLabel}
                      stroke="#f59e0b"
                      strokeWidth={2}
                      dot={false}
                    />
                    <Line
                      yAxisId="tokens"
                      type="monotone"
                      dataKey="cacheRead"
                      name={ZH.cacheReadLabel}
                      stroke="#8b5cf6"
                      strokeWidth={2}
                      dot={false}
                    />
                    <Line
                      yAxisId="ratio"
                      type="monotone"
                      dataKey="cacheHitRatio"
                      name={ZH.cacheHitLabel}
                      stroke="#f43f5e"
                      strokeWidth={2}
                      strokeDasharray="6 3"
                      dot={{ r: 2 }}
                    />
                  </LineChart>
                </ResponsiveContainer>
              </div>

              {/* Vendor Breakdown */}
              <div className="bg-white rounded-xl border border-slate-200 p-5 shadow-sm">
                <h3 className="text-sm font-semibold text-slate-700 mb-4">
                  {ZH.vendorBreakdown}
                </h3>
                <ResponsiveContainer width="100%" height={320}>
                  <BarChart data={vendorChartData} layout="vertical">
                    <CartesianGrid
                      strokeDasharray="3 3"
                      stroke="#e2e8f0"
                    />
                    <XAxis
                      type="number"
                      tick={{ fontSize: 11, fill: "#64748b" }}
                      tickFormatter={(v: number) => formatNumber(v)}
                    />
                    <YAxis
                      type="category"
                      dataKey="name"
                      tick={{ fontSize: 11, fill: "#64748b" }}
                      width={100}
                    />
                    <Tooltip content={<CustomTooltip />} />
                    <Bar
                      dataKey="tokens"
                      name={ZH.totalTokensLabel}
                      radius={[0, 4, 4, 0]}
                    >
                      {vendorChartData.map((_, i) => (
                        <Cell key={i} />
                      ))}
                    </Bar>
                  </BarChart>
                </ResponsiveContainer>
              </div>
            </div>

            {/* Token Distribution Pie */}
            <div className="bg-white rounded-xl border border-slate-200 p-5 shadow-sm mb-6">
              <h3 className="text-sm font-semibold text-slate-700 mb-4">
                {ZH.tokenDistribution}
              </h3>
              <ResponsiveContainer width="100%" height={250}>
                <PieChart>
                  <Pie
                    data={pieData}
                    cx="50%"
                    cy="50%"
                    innerRadius={60}
                    outerRadius={90}
                    paddingAngle={3}
                    dataKey="value"
                  >
                    {pieData.map((_entry, i) => (
                      <Cell key={i} />
                    ))}
                  </Pie>
                  <Tooltip content={<PieTooltip />} />
                  <Legend />
                </PieChart>
              </ResponsiveContainer>
            </div>

            {/* Vendor Detail Table */}
            <div className="bg-white rounded-xl border border-slate-200 shadow-sm mb-6">
              <div className="p-5 border-b border-slate-100">
                <h3 className="text-sm font-semibold text-slate-700">
                  {ZH.vendorPerformance}
                </h3>
              </div>
              <div className="overflow-x-auto">
                <table className="w-full text-sm">
                  <thead>
                    <tr className="bg-slate-50 text-slate-500 text-xs uppercase tracking-wider">
                      <th className="px-4 py-3 text-left font-medium">
                        {ZH.provider}
                      </th>
                      <th className="px-4 py-3 text-right font-medium">
                        {ZH.calls}
                      </th>
                      <th className="px-4 py-3 text-right font-medium">
                        {ZH.input}
                      </th>
                      <th className="px-4 py-3 text-right font-medium">
                        {ZH.output}
                      </th>
                      <th className="px-4 py-3 text-right font-medium">
                        {ZH.cacheReadCol}
                      </th>
                      <th className="px-4 py-3 text-right font-medium">
                        {ZH.cacheWriteCol}
                      </th>
                      <th className="px-4 py-3 text-right font-medium">
                        {ZH.total}
                      </th>
                      <th className="px-4 py-3 text-right font-medium">
                        {ZH.cacheHit}
                      </th>
                      <th className="px-4 py-3 text-right font-medium">
                        {ZH.cost}
                      </th>
                    </tr>
                  </thead>
                  <tbody className="divide-y divide-slate-100">
                    {stats.by_vendor.map((v) => (
                      <tr
                        key={v.provider}
                        className="hover:bg-slate-50 transition-colors"
                      >
                        <td className="px-4 py-3">
                          <div className="flex items-center gap-2">
                            <span
                              className="w-2.5 h-2.5 rounded-full"
                              style={{
                                background: getSourceColor(v.provider),
                              }}
                            />
                            <span className="font-medium text-slate-700">
                              {v.provider}
                            </span>
                          </div>
                        </td>
                        <td className="px-4 py-3 text-right text-slate-600">
                          {formatNumber(v.calls)}
                        </td>
                        <td className="px-4 py-3 text-right text-slate-600">
                          {formatNumber(v.input_tokens)}
                        </td>
                        <td className="px-4 py-3 text-right text-slate-600">
                          {formatNumber(v.output_tokens)}
                        </td>
                        <td className="px-4 py-3 text-right text-slate-600">
                          {formatNumber(v.cache_read_tokens)}
                        </td>
                        <td className="px-4 py-3 text-right text-slate-600">
                          {formatNumber(v.cache_write_tokens)}
                        </td>
                        <td className="px-4 py-3 text-right font-semibold text-slate-700">
                          {formatNumber(v.total_tokens)}
                        </td>
                        <td className="px-4 py-3 text-right">
                          <span
                            className={`px-2 py-0.5 rounded-full text-xs font-medium ${
                              v.cache_hit_ratio > 50
                                ? "bg-emerald-100 text-emerald-700"
                                : v.cache_hit_ratio > 10
                                ? "bg-amber-100 text-amber-700"
                                : "bg-slate-100 text-slate-600"
                            }`}
                          >
                            {formatPercent(v.cache_hit_ratio)}
                          </span>
                        </td>
                        <td className="px-4 py-3 text-right text-slate-600">
                          {formatCost(v.cost)}
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            </div>

            {/* Model Performance */}
            <div className="bg-white rounded-xl border border-slate-200 shadow-sm mb-6">
              <div className="p-5 border-b border-slate-100">
                <h3 className="text-sm font-semibold text-slate-700">
                  {ZH.modelPerformance}
                </h3>
              </div>
              <div className="overflow-x-auto">
                <table className="w-full text-sm">
                  <thead>
                    <tr className="bg-slate-50 text-slate-500 text-xs uppercase tracking-wider">
                      <th className="px-4 py-3 text-left font-medium">
                        {ZH.model}
                      </th>
                      <th className="px-4 py-3 text-left font-medium">
                        {ZH.provider}
                      </th>
                      <th className="px-4 py-3 text-right font-medium">
                        {ZH.calls}
                      </th>
                      <th className="px-4 py-3 text-right font-medium">
                        {ZH.input}
                      </th>
                      <th className="px-4 py-3 text-right font-medium">
                        {ZH.output}
                      </th>
                      <th className="px-4 py-3 text-right font-medium">
                        {ZH.cacheReadCol}
                      </th>
                      <th className="px-4 py-3 text-right font-medium">
                        {ZH.total}
                      </th>
                      <th className="px-4 py-3 text-right font-medium">
                        {ZH.cacheHit}
                      </th>
                      <th className="px-4 py-3 text-right font-medium">
                        {ZH.cost}
                      </th>
                    </tr>
                  </thead>
                  <tbody className="divide-y divide-slate-100">
                    {stats.by_model.map((m) => (
                      <tr
                        key={`${m.provider}-${m.model}`}
                        className="hover:bg-slate-50 transition-colors"
                      >
                        <td className="px-4 py-3 font-medium text-slate-700">
                          {m.model}
                        </td>
                        <td className="px-4 py-3 text-slate-500">
                          {m.provider}
                        </td>
                        <td className="px-4 py-3 text-right text-slate-600">
                          {formatNumber(m.calls)}
                        </td>
                        <td className="px-4 py-3 text-right text-slate-600">
                          {formatNumber(m.input_tokens)}
                        </td>
                        <td className="px-4 py-3 text-right text-slate-600">
                          {formatNumber(m.output_tokens)}
                        </td>
                        <td className="px-4 py-3 text-right text-slate-600">
                          {formatNumber(m.cache_read_tokens)}
                        </td>
                        <td className="px-4 py-3 text-right font-semibold text-slate-700">
                          {formatNumber(m.total_tokens)}
                        </td>
                        <td className="px-4 py-3 text-right">
                          <span
                            className={`px-2 py-0.5 rounded-full text-xs font-medium ${
                              m.cache_hit_ratio > 50
                                ? "bg-emerald-100 text-emerald-700"
                                : m.cache_hit_ratio > 10
                                ? "bg-amber-100 text-amber-700"
                                : "bg-slate-100 text-slate-600"
                            }`}
                          >
                            {formatPercent(m.cache_hit_ratio)}
                          </span>
                        </td>
                        <td className="px-4 py-3 text-right text-slate-600">
                          {formatCost(m.cost)}
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            </div>

            {/* Detailed Requests */}
            <div className="bg-white rounded-xl border border-slate-200 shadow-sm">
              <div className="p-5 border-b border-slate-100 flex flex-col sm:flex-row sm:items-center sm:justify-between gap-4">
                <h3 className="text-sm font-semibold text-slate-700">
                  {ZH.detailedRequests}
                </h3>
                <div className="flex items-center gap-2">
                  <Filter className="w-4 h-4 text-slate-400" />
                  <select
                    value={selectedVendor}
                    onChange={(e) => {
                      setSelectedVendor(e.target.value);
                      setPage(1);
                    }}
                    className="px-3 py-1.5 text-sm border border-slate-200 rounded-md focus:ring-2 focus:ring-primary-500 outline-none bg-white"
                  >
                    <option value="">{ZH.allProviders}</option>
                    {filters.vendors.map((v) => (
                      <option key={v} value={v}>
                        {v}
                      </option>
                    ))}
                  </select>
                  <select
                    value={selectedModel}
                    onChange={(e) => {
                      setSelectedModel(e.target.value);
                      setPage(1);
                    }}
                    className="px-3 py-1.5 text-sm border border-slate-200 rounded-md focus:ring-2 focus:ring-primary-500 outline-none bg-white"
                  >
                    <option value="">{ZH.allModels}</option>
                    {filters.models.map((m) => (
                      <option key={m} value={m}>
                        {m}
                      </option>
                    ))}
                  </select>
                </div>
              </div>

              <div className="overflow-x-auto">
                <table className="w-full text-sm">
                  <thead>
                    <tr className="bg-slate-50 text-slate-500 text-xs uppercase tracking-wider">
                      <th className="px-4 py-3 text-left font-medium">
                        {ZH.date}
                      </th>
                      <th className="px-4 py-3 text-left font-medium">
                        {ZH.provider}
                      </th>
                      <th className="px-4 py-3 text-left font-medium">
                        {ZH.model}
                      </th>
                      <th className="px-4 py-3 text-left font-medium">
                        {ZH.source}
                      </th>
                      <th className="px-4 py-3 text-right font-medium">
                        {ZH.input}
                      </th>
                      <th className="px-4 py-3 text-right font-medium">
                        {ZH.output}
                      </th>
                      <th className="px-4 py-3 text-right font-medium">
                        {ZH.cacheReadCol}
                      </th>
                      <th className="px-4 py-3 text-right font-medium">
                        {ZH.cacheWriteCol}
                      </th>
                      <th className="px-4 py-3 text-right font-medium">
                        {ZH.total}
                      </th>
                      <th className="px-4 py-3 text-right font-medium">
                        {ZH.cacheHit}
                      </th>
                      <th className="px-4 py-3 text-right font-medium">
                        {ZH.cost}
                      </th>
                    </tr>
                  </thead>
                  <tbody className="divide-y divide-slate-100">
                    {requests?.data.map((r, i) => (
                      <tr
                        key={i}
                        className="hover:bg-slate-50 transition-colors"
                      >
                        <td className="px-4 py-3 text-slate-600 whitespace-nowrap">
                          {r.time}
                        </td>
                        <td className="px-4 py-3">
                          <span
                            className="inline-flex items-center gap-1.5 px-2 py-0.5 rounded-full text-xs font-medium"
                            style={{
                              background: `${getSourceColor(r.provider)}15`,
                              color: getSourceColor(r.provider),
                            }}
                          >
                            <span
                              className="w-1.5 h-1.5 rounded-full"
                              style={{
                                background: getSourceColor(r.provider),
                              }}
                            />
                            {r.provider}
                          </span>
                        </td>
                        <td className="px-4 py-3 text-slate-600">
                          {r.model}
                        </td>
                        <td className="px-4 py-3">
                          <SourceBadge source={r.source} />
                        </td>
                        <td className="px-4 py-3 text-right text-slate-600">
                          {formatNumber(r.input_tokens)}
                        </td>
                        <td className="px-4 py-3 text-right text-slate-600">
                          {formatNumber(r.output_tokens)}
                        </td>
                        <td className="px-4 py-3 text-right text-slate-600">
                          {formatNumber(r.cache_read_tokens)}
                        </td>
                        <td className="px-4 py-3 text-right text-slate-600">
                          {formatNumber(r.cache_write_tokens)}
                        </td>
                        <td className="px-4 py-3 text-right font-semibold text-slate-700">
                          {formatNumber(r.total_tokens)}
                        </td>
                        <td className="px-4 py-3 text-right">
                          <span
                            className={`px-2 py-0.5 rounded-full text-xs font-medium ${
                              r.cache_hit_ratio > 50
                                ? "bg-emerald-100 text-emerald-700"
                                : r.cache_hit_ratio > 10
                                ? "bg-amber-100 text-amber-700"
                                : "bg-slate-100 text-slate-600"
                            }`}
                          >
                            {formatPercent(r.cache_hit_ratio)}
                          </span>
                        </td>
                        <td className="px-4 py-3 text-right text-slate-600">
                          {formatCost(r.cost, r.source)}
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>

              {/* Pagination */}
              {requests && requests.total_pages > 1 && (
                <div className="p-4 border-t border-slate-100 flex items-center justify-between">
                  <p className="text-sm text-slate-500">
                    {ZH.showing}{" "}
                    {(requests.page - 1) * requests.limit + 1}-
                    {Math.min(
                      requests.page * requests.limit,
                      requests.total
                    )}{" "}
                    {ZH.of} {formatNumber(requests.total)} {ZH.requests}
                  </p>
                  <div className="flex items-center gap-1">
                    <button
                      onClick={() => setPage((p) => Math.max(1, p - 1))}
                      disabled={requests.page <= 1}
                      className="p-1.5 rounded-md hover:bg-slate-100 disabled:opacity-40 disabled:cursor-not-allowed transition-colors"
                    >
                      <ChevronLeft className="w-4 h-4" />
                    </button>
                    {Array.from(
                      { length: Math.min(5, requests.total_pages) },
                      (_, i) => {
                        let pageNum: number;
                        if (requests.total_pages <= 5) {
                          pageNum = i + 1;
                        } else if (requests.page <= 3) {
                          pageNum = i + 1;
                        } else if (
                          requests.page >=
                          requests.total_pages - 2
                        ) {
                          pageNum = requests.total_pages - 4 + i;
                        } else {
                          pageNum = requests.page - 2 + i;
                        }
                        return (
                          <button
                            key={pageNum}
                            onClick={() => setPage(pageNum)}
                            className={`w-8 h-8 rounded-md text-sm font-medium transition-colors ${
                              pageNum === requests.page
                                ? "bg-primary-600 text-white"
                                : "hover:bg-slate-100 text-slate-600"
                            }`}
                          >
                            {pageNum}
                          </button>
                        );
                      }
                    )}
                    <button
                      onClick={() =>
                        setPage((p) =>
                          Math.min(requests.total_pages, p + 1)
                        )
                      }
                      disabled={requests.page >= requests.total_pages}
                      className="p-1.5 rounded-md hover:bg-slate-100 disabled:opacity-40 disabled:cursor-not-allowed transition-colors"
                    >
                      <ChevronRight className="w-4 h-4" />
                    </button>
                  </div>
                </div>
              )}
            </div>

            <footer className="mt-8 text-center text-sm text-slate-400 pb-6">
              {ZH.footer}
            </footer>
          </>
        )}
      </main>
    </div>
  );
}