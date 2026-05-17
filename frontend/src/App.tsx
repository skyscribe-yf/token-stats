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
  formatTokens,
  formatCost,
  formatPercent,
  formatDate,
  formatDateTime,
  getVendorColor,
  cn,
} from "./lib/utils";

function getDefaultDateRange() {
  const to = new Date();
  const from = new Date();
  from.setDate(from.getDate() - 30);
  return {
    from: from.toISOString().slice(0, 10),
    to: to.toISOString().slice(0, 10),
  };
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
        <div
          className={cn(
            "p-2.5 rounded-lg",
            color
          )}
        >
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
      <p className="text-slate-600">{formatNumber(p.value)} tokens</p>
      <p className="text-slate-500">{p.percent?.toFixed(1)}%</p>
    </div>
  );
}

export default function App() {
  const [dateRange, setDateRange] = useState(getDefaultDateRange);
  const [stats, setStats] = useState<StatsResponse | null>(null);
  const [requests, setRequests] = useState<PaginatedRequests | null>(null);
  const [filters, setFilters] = useState<FilterOptions>({ vendors: [], models: [] });
  const [selectedVendor, setSelectedVendor] = useState<string>("");
  const [selectedModel, setSelectedModel] = useState<string>("");
  const [page, setPage] = useState(1);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const loadData = async () => {
    setLoading(true);
    setError(null);
    try {
      const [s, f] = await Promise.all([
        fetchStats(dateRange.from, dateRange.to),
        fetchFilters(),
      ]);
      setStats(s);
      setFilters(f);
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
        dateRange.from,
        dateRange.to,
        selectedVendor || undefined,
        selectedModel || undefined,
        page,
        50
      );
      setRequests(r);
    } catch (e) {
      console.error("Failed to load requests", e);
    }
  };

  useEffect(() => {
    loadData();
  }, [dateRange.from, dateRange.to]);

  useEffect(() => {
    loadRequests();
  }, [dateRange.from, dateRange.to, selectedVendor, selectedModel, page]);

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
    { label: "7D", days: 7 },
    { label: "30D", days: 30 },
    { label: "90D", days: 90 },
    { label: "All", days: 365 * 10 },
  ];

  const applyPreset = (days: number) => {
    const to = new Date();
    const from = new Date();
    from.setDate(from.getDate() - days);
    setDateRange({
      from: from.toISOString().slice(0, 10),
      to: to.toISOString().slice(0, 10),
    });
  };

  return (
    <div className="min-h-screen bg-slate-50">
      {/* Header */}
      <header className="bg-white border-b border-slate-200 sticky top-0 z-10">
        <div className="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8 py-4">
          <div className="flex flex-col sm:flex-row sm:items-center sm:justify-between gap-4">
            <div className="flex items-center gap-3">
              <div className="bg-primary-600 p-2 rounded-lg">
                <Activity className="w-6 h-6 text-white" />
              </div>
              <div>
                <h1 className="text-xl font-bold text-slate-800">Token Stats Dashboard</h1>
                <p className="text-sm text-slate-500">Monitor your AI token usage across providers</p>
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
              <div className="flex items-center gap-1.5 ml-2">
                <Calendar className="w-4 h-4 text-slate-400" />
                <input
                  type="date"
                  value={dateRange.from}
                  onChange={(e) =>
                    setDateRange((prev) => ({ ...prev, from: e.target.value }))
                  }
                  className="px-2 py-1.5 text-sm border border-slate-200 rounded-md focus:ring-2 focus:ring-primary-500 focus:border-primary-500 outline-none"
                />
                <span className="text-slate-400">-</span>
                <input
                  type="date"
                  value={dateRange.to}
                  onChange={(e) =>
                    setDateRange((prev) => ({ ...prev, to: e.target.value }))
                  }
                  className="px-2 py-1.5 text-sm border border-slate-200 rounded-md focus:ring-2 focus:ring-primary-500 focus:border-primary-500 outline-none"
                />
              </div>
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
                title="Total Calls"
                value={formatNumber(stats.overall.total_calls)}
                icon={Server}
                color="bg-primary-500"
              />
              <StatCard
                title="Input Tokens"
                value={formatTokens(stats.overall.total_input_tokens)}
                icon={Database}
                color="bg-emerald-500"
              />
              <StatCard
                title="Output Tokens"
                value={formatTokens(stats.overall.total_output_tokens)}
                icon={Zap}
                color="bg-amber-500"
              />
              <StatCard
                title="Cache Read"
                value={formatTokens(stats.overall.total_cache_read_tokens)}
                icon={TrendingUp}
                color="bg-violet-500"
              />
              <StatCard
                title="Cache Hit Ratio"
                value={formatPercent(stats.overall.weighted_cache_hit_ratio)}
                subtitle="weighted"
                icon={Activity}
                color="bg-rose-500"
              />
              <StatCard
                title="Total Cost"
                value={formatCost(stats.overall.total_cost)}
                icon={Coins}
                color="bg-slate-700"
              />
            </div>

            {/* Charts Row */}
            <div className="grid grid-cols-1 lg:grid-cols-2 gap-6 mb-6">
              {/* Daily Trends */}
              <div className="bg-white rounded-xl border border-slate-200 p-5 shadow-sm">
                <h3 className="text-sm font-semibold text-slate-700 mb-4">
                  Daily Token Usage
                </h3>
                <ResponsiveContainer width="100%" height={280}>
                  <LineChart data={chartData}>
                    <CartesianGrid strokeDasharray="3 3" stroke="#e2e8f0" />
                    <XAxis
                      dataKey="date"
                      tick={{ fontSize: 11, fill: "#64748b" }}
                      angle={-30}
                      textAnchor="end"
                      height={60}
                    />
                    <YAxis tick={{ fontSize: 11, fill: "#64748b" }} />
                    <Tooltip content={<CustomTooltip />} />
                    <Legend />
                    <Line
                      type="monotone"
                      dataKey="input"
                      name="Input"
                      stroke="#10b981"
                      strokeWidth={2}
                      dot={false}
                    />
                    <Line
                      type="monotone"
                      dataKey="output"
                      name="Output"
                      stroke="#f59e0b"
                      strokeWidth={2}
                      dot={false}
                    />
                    <Line
                      type="monotone"
                      dataKey="cacheRead"
                      name="Cache Read"
                      stroke="#8b5cf6"
                      strokeWidth={2}
                      dot={false}
                    />
                  </LineChart>
                </ResponsiveContainer>
              </div>

              {/* Vendor Breakdown */}
              <div className="bg-white rounded-xl border border-slate-200 p-5 shadow-sm">
                <h3 className="text-sm font-semibold text-slate-700 mb-4">
                  Vendor Breakdown
                </h3>
                <ResponsiveContainer width="100%" height={280}>
                  <BarChart data={vendorChartData} layout="vertical">
                    <CartesianGrid strokeDasharray="3 3" stroke="#e2e8f0" />
                    <XAxis
                      type="number"
                      tick={{ fontSize: 11, fill: "#64748b" }}
                    />
                    <YAxis
                      type="category"
                      dataKey="name"
                      tick={{ fontSize: 11, fill: "#64748b" }}
                      width={100}
                    />
                    <Tooltip content={<CustomTooltip />} />
                    <Bar dataKey="tokens" name="Total Tokens" radius={[0, 4, 4, 0]}>
                      {vendorChartData.map((_, i) => (
                        <Cell
                          key={i}
                          fill={getVendorColor(vendorChartData[i]?.name || "")}
                        />
                      ))}
                    </Bar>
                  </BarChart>
                </ResponsiveContainer>
              </div>
            </div>

            {/* Second Row Charts */}
            <div className="grid grid-cols-1 lg:grid-cols-2 gap-6 mb-6">
              {/* Cache Hit Ratio by Date */}
              <div className="bg-white rounded-xl border border-slate-200 p-5 shadow-sm">
                <h3 className="text-sm font-semibold text-slate-700 mb-4">
                  Cache Hit Ratio Trend
                </h3>
                <ResponsiveContainer width="100%" height={250}>
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
                      tick={{ fontSize: 11, fill: "#64748b" }}
                      domain={[0, 100]}
                      unit="%"
                    />
                    <Tooltip content={<CustomTooltip />} />
                    <Line
                      type="monotone"
                      dataKey="cacheHitRatio"
                      name="Cache Hit %"
                      stroke="#f43f5e"
                      strokeWidth={2}
                      dot={{ r: 3 }}
                    />
                  </LineChart>
                </ResponsiveContainer>
              </div>

              {/* Token Distribution Pie */}
              <div className="bg-white rounded-xl border border-slate-200 p-5 shadow-sm">
                <h3 className="text-sm font-semibold text-slate-700 mb-4">
                  Token Distribution
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
                      {pieData.map((entry, i) => (
                        <Cell
                          key={i}
                          fill={getVendorColor(entry.name)}
                        />
                      ))}
                    </Pie>
                    <Tooltip content={<PieTooltip />} />
                    <Legend />
                  </PieChart>
                </ResponsiveContainer>
              </div>
            </div>

            {/* Vendor Detail Table */}
            <div className="bg-white rounded-xl border border-slate-200 shadow-sm mb-6">
              <div className="p-5 border-b border-slate-100">
                <h3 className="text-sm font-semibold text-slate-700">
                  Vendor Performance
                </h3>
              </div>
              <div className="overflow-x-auto">
                <table className="w-full text-sm">
                  <thead>
                    <tr className="bg-slate-50 text-slate-500 text-xs uppercase tracking-wider">
                      <th className="px-4 py-3 text-left font-medium">Provider</th>
                      <th className="px-4 py-3 text-right font-medium">Calls</th>
                      <th className="px-4 py-3 text-right font-medium">Input</th>
                      <th className="px-4 py-3 text-right font-medium">Output</th>
                      <th className="px-4 py-3 text-right font-medium">Cache Read</th>
                      <th className="px-4 py-3 text-right font-medium">Cache Write</th>
                      <th className="px-4 py-3 text-right font-medium">Total</th>
                      <th className="px-4 py-3 text-right font-medium">Cache Hit</th>
                      <th className="px-4 py-3 text-right font-medium">Cost</th>
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
                              style={{ background: getVendorColor(v.provider) }}
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
                          {formatTokens(v.input_tokens)}
                        </td>
                        <td className="px-4 py-3 text-right text-slate-600">
                          {formatTokens(v.output_tokens)}
                        </td>
                        <td className="px-4 py-3 text-right text-slate-600">
                          {formatTokens(v.cache_read_tokens)}
                        </td>
                        <td className="px-4 py-3 text-right text-slate-600">
                          {formatTokens(v.cache_write_tokens)}
                        </td>
                        <td className="px-4 py-3 text-right font-semibold text-slate-700">
                          {formatTokens(v.total_tokens)}
                        </td>
                        <td className="px-4 py-3 text-right">
                          <span
                            className={cn(
                              "px-2 py-0.5 rounded-full text-xs font-medium",
                              v.cache_hit_ratio > 50
                                ? "bg-emerald-100 text-emerald-700"
                                : v.cache_hit_ratio > 10
                                ? "bg-amber-100 text-amber-700"
                                : "bg-slate-100 text-slate-600"
                            )}
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
                  Model Performance
                </h3>
              </div>
              <div className="overflow-x-auto">
                <table className="w-full text-sm">
                  <thead>
                    <tr className="bg-slate-50 text-slate-500 text-xs uppercase tracking-wider">
                      <th className="px-4 py-3 text-left font-medium">Model</th>
                      <th className="px-4 py-3 text-left font-medium">Provider</th>
                      <th className="px-4 py-3 text-right font-medium">Calls</th>
                      <th className="px-4 py-3 text-right font-medium">Input</th>
                      <th className="px-4 py-3 text-right font-medium">Output</th>
                      <th className="px-4 py-3 text-right font-medium">Cache Read</th>
                      <th className="px-4 py-3 text-right font-medium">Total</th>
                      <th className="px-4 py-3 text-right font-medium">Cache Hit</th>
                      <th className="px-4 py-3 text-right font-medium">Cost</th>
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
                          <span
                            className="inline-flex items-center gap-1.5 px-2 py-0.5 rounded-full text-xs"
                            style={{
                              background: `${getVendorColor(m.provider)}15`,
                              color: getVendorColor(m.provider),
                            }}
                          >
                            <span
                              className="w-1.5 h-1.5 rounded-full"
                              style={{ background: getVendorColor(m.provider) }}
                            />
                            {m.provider}
                          </span>
                        </td>
                        <td className="px-4 py-3 text-right text-slate-600">
                          {formatNumber(m.calls)}
                        </td>
                        <td className="px-4 py-3 text-right text-slate-600">
                          {formatTokens(m.input_tokens)}
                        </td>
                        <td className="px-4 py-3 text-right text-slate-600">
                          {formatTokens(m.output_tokens)}
                        </td>
                        <td className="px-4 py-3 text-right text-slate-600">
                          {formatTokens(m.cache_read_tokens)}
                        </td>
                        <td className="px-4 py-3 text-right font-semibold text-slate-700">
                          {formatTokens(m.total_tokens)}
                        </td>
                        <td className="px-4 py-3 text-right">
                          <span
                            className={cn(
                              "px-2 py-0.5 rounded-full text-xs font-medium",
                              m.cache_hit_ratio > 50
                                ? "bg-emerald-100 text-emerald-700"
                                : m.cache_hit_ratio > 10
                                ? "bg-amber-100 text-amber-700"
                                : "bg-slate-100 text-slate-600"
                            )}
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
                  Detailed Requests
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
                    <option value="">All Providers</option>
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
                    <option value="">All Models</option>
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
                      <th className="px-4 py-3 text-left font-medium">Date</th>
                      <th className="px-4 py-3 text-left font-medium">Provider</th>
                      <th className="px-4 py-3 text-left font-medium">Model</th>
                      <th className="px-4 py-3 text-right font-medium">Input</th>
                      <th className="px-4 py-3 text-right font-medium">Output</th>
                      <th className="px-4 py-3 text-right font-medium">Cache Read</th>
                      <th className="px-4 py-3 text-right font-medium">Cache Write</th>
                      <th className="px-4 py-3 text-right font-medium">Total</th>
                      <th className="px-4 py-3 text-right font-medium">Cache Hit</th>
                      <th className="px-4 py-3 text-right font-medium">Cost</th>
                    </tr>
                  </thead>
                  <tbody className="divide-y divide-slate-100">
                    {requests?.data.map((r, i) => (
                      <tr key={i} className="hover:bg-slate-50 transition-colors">
                        <td className="px-4 py-3 text-slate-600 whitespace-nowrap">
                          {formatDateTime(r.time)}
                        </td>
                        <td className="px-4 py-3">
                          <span
                            className="inline-flex items-center gap-1.5 px-2 py-0.5 rounded-full text-xs font-medium"
                            style={{
                              background: `${getVendorColor(r.provider)}15`,
                              color: getVendorColor(r.provider),
                            }}
                          >
                            <span
                              className="w-1.5 h-1.5 rounded-full"
                              style={{ background: getVendorColor(r.provider) }}
                            />
                            {r.provider}
                          </span>
                        </td>
                        <td className="px-4 py-3 text-slate-600">{r.model}</td>
                        <td className="px-4 py-3 text-right text-slate-600">
                          {formatTokens(r.input_tokens)}
                        </td>
                        <td className="px-4 py-3 text-right text-slate-600">
                          {formatTokens(r.output_tokens)}
                        </td>
                        <td className="px-4 py-3 text-right text-slate-600">
                          {formatTokens(r.cache_read_tokens)}
                        </td>
                        <td className="px-4 py-3 text-right text-slate-600">
                          {formatTokens(r.cache_write_tokens)}
                        </td>
                        <td className="px-4 py-3 text-right font-semibold text-slate-700">
                          {formatTokens(r.total_tokens)}
                        </td>
                        <td className="px-4 py-3 text-right">
                          <span
                            className={cn(
                              "px-2 py-0.5 rounded-full text-xs font-medium",
                              r.cache_hit_ratio > 50
                                ? "bg-emerald-100 text-emerald-700"
                                : r.cache_hit_ratio > 10
                                ? "bg-amber-100 text-amber-700"
                                : "bg-slate-100 text-slate-600"
                            )}
                          >
                            {formatPercent(r.cache_hit_ratio)}
                          </span>
                        </td>
                        <td className="px-4 py-3 text-right text-slate-600">
                          {formatCost(r.cost)}
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
                    Showing {(requests.page - 1) * requests.limit + 1}-
                    {Math.min(
                      requests.page * requests.limit,
                      requests.total
                    )}{" "}
                    of {formatNumber(requests.total)} requests
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
                        } else if (requests.page >= requests.total_pages - 2) {
                          pageNum = requests.total_pages - 4 + i;
                        } else {
                          pageNum = requests.page - 2 + i;
                        }
                        return (
                          <button
                            key={pageNum}
                            onClick={() => setPage(pageNum)}
                            className={cn(
                              "w-8 h-8 rounded-md text-sm font-medium transition-colors",
                              pageNum === requests.page
                                ? "bg-primary-600 text-white"
                                : "hover:bg-slate-100 text-slate-600"
                            )}
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
              Token Stats Dashboard · Built with Rust + React
            </footer>
          </>
        )}
      </main>
    </div>
  );
}
