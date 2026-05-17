import { useState, useEffect, useMemo, useRef, useCallback } from "react";
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
  ChevronLeft,
  ChevronRight,
  Filter,
  Activity,
  Coins,
  Database,
  TrendingUp,
  Zap,
  Server,
  X,
  SlidersHorizontal,
} from "lucide-react";
import {
  fetchStats,
  fetchRequests,
  fetchFilters,
  fetchQuota,
  type StatsResponse,
  type PaginatedRequests,
  type FilterOptions,
  type QuotaResponse,
} from "./api";
import {
  formatNumber,
  formatCost,
  formatPercent,
  formatDate,
  formatTime,
  getLocalToday,
  getLocalDateOffset,
  getLocalDatetimeOffsetHours,
  getSourceColor,
  getSourceLabel,
} from "./lib/utils";
import {
  buildCsvFilterParam,
  formatAppliedRange,
  isEmptyAppliedSelection,
  type AppliedRange,
} from "./lib/filterState";

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
  last7Days: "最近7天",
  customTime: "自定义",
  last6h: "最近6小时",
  last12h: "最近12小时",
  last1d: "最近1天",
  last3d: "最近3天",
  last14d: "最近14天",
  last30d: "最近30天",
  allTime: "所有",
  vendorAndModel: "供应商 & 模型表现",
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
  from: "从",
  to: "至",
  inputLabel: "输入",
  outputLabel: "输出",
  cacheReadLabel: "缓存读取",
  cacheHitLabel: "缓存命中率",
  totalTokensLabel: "总 Token",
  tokens: "Token",
  today: "今天",
  apply: "应用",
  cancel: "取消",
  quickSelect: "快捷选择",
  currentFilters: "当前筛选",
  timeRange: "时间范围",
  updatedAt: "更新时间",
  updating: "更新中...",
  noneSelected: "未选择",
  requestModel: "明细模型",
} as const;

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

function QuotaCard({
  title,
  provider,
  status,
  children,
}: {
  title: string;
  provider: string;
  status: "available" | "unavailable" | "loading";
  children: React.ReactNode;
}) {
  const borderColor = status === "available" ? "border-emerald-200" : "border-slate-200";
  const indicatorColor = status === "available" ? "bg-emerald-500" : status === "loading" ? "bg-amber-400" : "bg-slate-300";
  return (
    <div className={`bg-white rounded-xl border ${borderColor} p-4 shadow-sm`}>
      <div className="flex items-center justify-between mb-3">
        <div className="flex items-center gap-2">
          <span className={`w-2 h-2 rounded-full ${indicatorColor}`} />
          <span className="text-xs font-semibold text-slate-700">{title}</span>
        </div>
        <span className="text-[10px] text-slate-400">{provider}</span>
      </div>
      {children}
    </div>
  );
}

function CustomTooltip({
  active,
  payload,
  label,
}: {
  active?: boolean;
  payload?: ChartTooltipPayload[];
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
          {p.name}: {formatNumber(Number(p.value ?? 0))}
        </p>
      ))}
    </div>
  );
}

interface ChartTooltipPayload {
  name?: string;
  value?: number | string;
  color?: string;
  percent?: number;
}

function PieTooltip({ active, payload }: { active?: boolean; payload?: ChartTooltipPayload[] }) {
  if (!active || !payload?.length) return null;
  const p = payload[0];
  return (
    <div className="bg-white border border-slate-200 rounded-lg shadow-lg p-3 text-sm">
      <p className="font-semibold text-slate-700">{p.name}</p>
      <p className="text-slate-600">{formatNumber(Number(p.value ?? 0))} {ZH.tokens}</p>
      <p className="text-slate-500">{p.percent?.toFixed(1)}%</p>
    </div>
  );
}

function toggleInSet<T>(set: Set<T>, setter: (s: Set<T>) => void, value: T) {
  const next = new Set(set);
  if (next.has(value)) next.delete(value);
  else next.add(value);
  setter(next);
}

type TimePreset = "today" | "6h" | "12h" | "1d" | "3d" | "7d" | "14d" | "30d" | "all" | "custom";

function getPresetLabel(preset: TimePreset): string {
  switch (preset) {
    case "today":
      return ZH.today;
    case "6h":
      return ZH.last6h;
    case "12h":
      return ZH.last12h;
    case "1d":
      return ZH.last1d;
    case "3d":
      return ZH.last3d;
    case "7d":
      return ZH.last7Days;
    case "14d":
      return ZH.last14d;
    case "30d":
      return ZH.last30d;
    case "all":
      return ZH.allTime;
    case "custom":
      return ZH.customTime;
  }
}

function getPresetRange(preset: Exclude<TimePreset, "custom">): Pick<AppliedRange, "from" | "to"> {
  switch (preset) {
    case "today": {
      const today = getLocalToday();
      return { from: today, to: today };
    }
    case "6h":
      return { from: getLocalDatetimeOffsetHours(6), to: getLocalDatetimeOffsetHours(0) };
    case "12h":
      return { from: getLocalDatetimeOffsetHours(12), to: getLocalDatetimeOffsetHours(0) };
    case "1d":
      return { from: getLocalDatetimeOffsetHours(24), to: getLocalDatetimeOffsetHours(0) };
    case "3d":
      return { from: getLocalDateOffset(3), to: getLocalToday() };
    case "7d":
      return { from: getLocalDateOffset(7), to: getLocalToday() };
    case "14d":
      return { from: getLocalDateOffset(14), to: getLocalToday() };
    case "30d":
      return { from: getLocalDateOffset(30), to: getLocalToday() };
    case "all":
      return { from: getLocalDateOffset(365 * 10), to: getLocalToday() };
  }
}

function makeAppliedRange(preset: Exclude<TimePreset, "custom">): AppliedRange {
  return {
    ...getPresetRange(preset),
    appliedAt: Date.now(),
  };
}

function makeCustomAppliedRange(from: string, to: string): AppliedRange {
  return { from, to, appliedAt: Date.now() };
}

function emptyStatsResponse(): StatsResponse {
  return {
    overall: {
      total_calls: 0,
      total_input_tokens: 0,
      total_output_tokens: 0,
      total_cache_read_tokens: 0,
      total_cache_write_tokens: 0,
      total_tokens: 0,
      total_cost: 0,
      avg_cache_hit_ratio: 0,
      weighted_cache_hit_ratio: 0,
    },
    by_vendor: [],
    by_date: [],
    by_model: [],
    by_source: [],
  };
}

function emptyRequests(page: number): PaginatedRequests {
  return {
    data: [],
    total: 0,
    page,
    limit: 50,
    total_pages: 0,
  };
}

function selectionLabel(
  selected: ReadonlySet<string>,
  options: readonly string[],
  format: (value: string) => string = (value) => value
): string {
  if (options.length === 0) return ZH.noneSelected;
  const selectedInOptionOrder = options.filter((option) => selected.has(option));
  if (selectedInOptionOrder.length === 0) return ZH.noneSelected;
  if (selectedInOptionOrder.length === options.length) return "全部";
  return selectedInOptionOrder.map(format).join(", ");
}

export default function App() {
  const [activePreset, setActivePreset] = useState<TimePreset>("7d");
  const [appliedRange, setAppliedRange] = useState<AppliedRange>(() => makeAppliedRange("7d"));
  const [customFrom, setCustomFrom] = useState<string>("");
  const [customTo, setCustomTo] = useState<string>("");
  const [showCustomPanel, setShowCustomPanel] = useState(false);
  const customBtnRef = useRef<HTMLButtonElement>(null);
  const filtersInitializedRef = useRef(false);

  const [stats, setStats] = useState<StatsResponse | null>(null);
  const [requests, setRequests] = useState<PaginatedRequests | null>(null);
  const [quota, setQuota] = useState<QuotaResponse | null>(null);
  const [quotaLoading, setQuotaLoading] = useState(true);
  const [filters, setFilters] = useState<FilterOptions>({
    vendors: [],
    models: [],
    sources: [],
  });
  const [selectedVendors, setSelectedVendors] = useState<Set<string>>(new Set());
  const [selectedSources, setSelectedSources] = useState<Set<string>>(new Set());
  const [selectedModel, setSelectedModel] = useState<string>("");
  const [page, setPage] = useState(1);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [lastUpdatedAt, setLastUpdatedAt] = useState<Date | null>(null);

  const tzOffset = useMemo(() => -new Date().getTimezoneOffset(), []);

  const effectiveRange = appliedRange;

  const sourceFilter = useMemo(() => {
    return buildCsvFilterParam(selectedSources, filters.sources);
  }, [selectedSources, filters.sources]);

  const vendorFilter = useMemo(() => {
    return buildCsvFilterParam(selectedVendors, filters.vendors);
  }, [selectedVendors, filters.vendors]);

  const hasEmptySourceSelection = useMemo(
    () => isEmptyAppliedSelection(selectedSources, filters.sources),
    [selectedSources, filters.sources]
  );
  const hasEmptyVendorSelection = useMemo(
    () => isEmptyAppliedSelection(selectedVendors, filters.vendors),
    [selectedVendors, filters.vendors]
  );
  const hasEmptyRequiredSelection = hasEmptySourceSelection || hasEmptyVendorSelection;

  const loadData = useCallback(async () => {
    if (!effectiveRange.from || !effectiveRange.to) return;

    if (hasEmptyRequiredSelection) {
      setStats(emptyStatsResponse());
      setRequests(emptyRequests(1));
      setPage(1);
      setLastUpdatedAt(new Date());
      return;
    }

    setLoading(true);
    setError(null);
    try {
      const [s, f] = await Promise.all([
        fetchStats(
          effectiveRange.from,
          effectiveRange.to,
          sourceFilter,
          vendorFilter,
          tzOffset
        ),
        fetchFilters(),
      ]);
      setStats(s);
      setFilters(f);
      setLastUpdatedAt(new Date());
      if (!filtersInitializedRef.current) {
        setSelectedSources(new Set(f.sources));
        setSelectedVendors(new Set(f.vendors));
        filtersInitializedRef.current = true;
      }
      setPage(1);
    } catch (e) {
      setError(e instanceof Error ? e.message : "加载数据失败");
    } finally {
      setLoading(false);
    }
  }, [
    effectiveRange.from,
    effectiveRange.to,
    sourceFilter,
    vendorFilter,
    tzOffset,
    hasEmptyRequiredSelection,
  ]);

  const loadRequests = useCallback(async () => {
    if (!effectiveRange.from || !effectiveRange.to) return;

    if (hasEmptyRequiredSelection) {
      setRequests(emptyRequests(1));
      return;
    }

    try {
      const r = await fetchRequests(
        effectiveRange.from,
        effectiveRange.to,
        vendorFilter,
        selectedModel || undefined,
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
    effectiveRange.from,
    effectiveRange.to,
    vendorFilter,
    selectedModel,
    sourceFilter,
    page,
    tzOffset,
    hasEmptyRequiredSelection,
  ]);

  useEffect(() => {
    const appliedAt = effectiveRange.appliedAt;
    queueMicrotask(() => {
      void appliedAt;
      void loadData();
    });
  }, [loadData, effectiveRange.appliedAt]);

  useEffect(() => {
    const appliedAt = effectiveRange.appliedAt;
    queueMicrotask(() => {
      void appliedAt;
      void loadRequests();
    });
  }, [loadRequests, effectiveRange.appliedAt]);

  useEffect(() => {
    const loadQuota = async () => {
      try {
        const q = await fetchQuota();
        setQuota(q);
      } catch {
        /* quota is optional — don't set error state */
      } finally {
        setQuotaLoading(false);
      }
    };
    loadQuota();
    const interval = setInterval(loadQuota, 60_000);
    return () => clearInterval(interval);
  }, []);

  // Close custom panel on outside click
  useEffect(() => {
    if (!showCustomPanel) return;
    const handler = (e: MouseEvent) => {
      const target = e.target as HTMLElement;
      if (!target.closest(".custom-time-panel") && !target.closest(".custom-time-btn")) {
        setShowCustomPanel(false);
      }
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [showCustomPanel]);

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

  const mergedTableData = useMemo(() => {
    if (!stats?.by_vendor || !stats?.by_model) return [];
    const modelMap = new Map<string, typeof stats.by_model>();
    for (const m of stats.by_model) {
      const arr = modelMap.get(m.provider) || [];
      arr.push(m);
      modelMap.set(m.provider, arr);
    }
    const rows: Array<
      | { type: "vendor"; data: (typeof stats.by_vendor)[0] }
      | { type: "model"; data: (typeof stats.by_model)[0] }
    > = [];
    for (const v of stats.by_vendor) {
      rows.push({ type: "vendor", data: v });
      const models = modelMap.get(v.provider) || [];
      for (const m of models) {
        rows.push({ type: "model", data: m });
      }
    }
    return rows;
  }, [stats]);

  const applyCustom = () => {
    if (customFrom && customTo) {
      setActivePreset("custom");
      setAppliedRange(makeCustomAppliedRange(customFrom, customTo));
      setShowCustomPanel(false);
    }
  };

  const applyPreset = (key: Exclude<TimePreset, "custom">) => {
    setActivePreset(key);
    setAppliedRange(makeAppliedRange(key));
    setShowCustomPanel(false);
  };

  const quickSetCustom = (key: Exclude<TimePreset, "custom">) => {
    applyPreset(key);
  };

  const presetBtnClass = (key: string) =>
    `px-3 py-1.5 text-xs font-medium rounded-md transition-colors ${
      activePreset === key
        ? "bg-primary-600 text-white"
        : "bg-slate-100 text-slate-600 hover:bg-slate-200"
    }`;

  const customQuickClass = (key: string) =>
    `px-2 py-1 text-[11px] font-medium rounded transition-colors ${
      activePreset === key
        ? "bg-primary-100 text-primary-700"
        : "bg-slate-50 text-slate-500 hover:bg-slate-100"
    }`;

  const appliedRangeLabel = useMemo(
    () => formatAppliedRange(getPresetLabel(activePreset), effectiveRange),
    [activePreset, effectiveRange]
  );
  const selectedSourceLabel = useMemo(
    () => selectionLabel(selectedSources, filters.sources, getSourceLabel),
    [selectedSources, filters.sources]
  );
  const selectedVendorLabel = useMemo(
    () => selectionLabel(selectedVendors, filters.vendors),
    [selectedVendors, filters.vendors]
  );

  return (
    <div className="min-h-screen bg-slate-50">
      {/* Header */}
      <header className="bg-white border-b border-slate-200 sticky top-0 z-20">
        <div className="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8 py-2.5">
          <div className="flex flex-wrap items-center gap-x-4 gap-y-1.5">
            {/* Logo + Title */}
            <div className="flex items-center gap-2 shrink-0">
              <div className="bg-primary-600 p-1.5 rounded-lg">
                <Activity className="w-5 h-5 text-white" />
              </div>
              <div>
                <h1 className="text-base font-bold text-slate-800 leading-tight">
                  {ZH.title}
                </h1>
                <p className="text-[11px] text-slate-500 leading-tight">{ZH.subtitle}</p>
              </div>
            </div>

            {/* Divider */}
            <div className="hidden sm:block w-px h-6 bg-slate-200" />

            {/* Time presets */}
            <div className="flex items-center gap-1.5">
              <button
                onClick={() => applyPreset("today")}
                className={presetBtnClass("today")}
              >
                {ZH.today}
              </button>
              <button
                onClick={() => applyPreset("7d")}
                className={presetBtnClass("7d")}
              >
                {ZH.last7Days}
              </button>
              <div className="relative">
                <button
                  ref={customBtnRef}
                  onClick={() => setShowCustomPanel((v) => !v)}
                  className={`custom-time-btn inline-flex items-center gap-1 px-3 py-1.5 text-xs font-medium rounded-md transition-colors ${
                    ["6h", "12h", "1d", "3d", "14d", "30d", "all", "custom"].includes(activePreset)
                      ? "bg-primary-600 text-white"
                      : "bg-slate-100 text-slate-600 hover:bg-slate-200"
                  }`}
                >
                  <SlidersHorizontal className="w-3 h-3" />
                  {ZH.customTime}
                </button>

                {/* Custom time panel */}
                {showCustomPanel && (
                  <div className="custom-time-panel absolute left-0 top-full mt-1.5 bg-white border border-slate-200 rounded-lg shadow-xl p-3 min-w-[320px] z-30">
                    <div className="flex items-center justify-between mb-2">
                      <span className="text-xs font-semibold text-slate-700">{ZH.quickSelect}</span>
                      <button
                        onClick={() => setShowCustomPanel(false)}
                        className="p-0.5 rounded hover:bg-slate-100 text-slate-400"
                      >
                        <X className="w-3.5 h-3.5" />
                      </button>
                    </div>
                    <div className="flex flex-wrap gap-1 mb-3">
                      {[
                        { k: "6h", l: ZH.last6h },
                        { k: "12h", l: ZH.last12h },
                        { k: "1d", l: ZH.last1d },
                        { k: "3d", l: ZH.last3d },
                        { k: "14d", l: ZH.last14d },
                        { k: "30d", l: ZH.last30d },
                        { k: "all", l: ZH.allTime },
                      ].map((q) => (
                        <button
                          key={q.k}
                          onClick={() => quickSetCustom(q.k as Exclude<TimePreset, "custom">)}
                          className={customQuickClass(q.k)}
                        >
                          {q.l}
                        </button>
                      ))}
                    </div>
                    <div className="flex items-center gap-2 mb-3">
                      <div className="flex-1">
                        <label className="block text-[10px] text-slate-400 mb-0.5">{ZH.from}</label>
                        <input
                          type="datetime-local"
                          value={customFrom}
                          onChange={(e) => setCustomFrom(e.target.value)}
                          className="w-full px-2 py-1 text-xs border border-slate-200 rounded focus:ring-1 focus:ring-primary-500 focus:border-primary-500 outline-none"
                        />
                      </div>
                      <span className="text-slate-300 mt-4">-</span>
                      <div className="flex-1">
                        <label className="block text-[10px] text-slate-400 mb-0.5">{ZH.to}</label>
                        <input
                          type="datetime-local"
                          value={customTo}
                          onChange={(e) => setCustomTo(e.target.value)}
                          className="w-full px-2 py-1 text-xs border border-slate-200 rounded focus:ring-1 focus:ring-primary-500 focus:border-primary-500 outline-none"
                        />
                      </div>
                    </div>
                    <div className="flex justify-end gap-1.5">
                      <button
                        onClick={() => setShowCustomPanel(false)}
                        className="px-2.5 py-1 text-[11px] font-medium rounded text-slate-500 hover:bg-slate-100 transition-colors"
                      >
                        {ZH.cancel}
                      </button>
                      <button
                        onClick={applyCustom}
                        disabled={!customFrom || !customTo}
                        className="px-2.5 py-1 text-[11px] font-medium rounded bg-primary-600 text-white hover:bg-primary-700 disabled:opacity-40 disabled:cursor-not-allowed transition-colors"
                      >
                        {ZH.apply}
                      </button>
                    </div>
                  </div>
                )}
              </div>
            </div>

            {/* Spacer */}
            <div className="flex-1" />

            {/* Source filter tags */}
            <div className="flex items-center gap-1 flex-wrap">
              {filters.sources.map((s) => (
                <button
                  key={s}
                  onClick={() => {
                    toggleInSet(selectedSources, setSelectedSources, s);
                    setPage(1);
                  }}
                  className={`inline-flex items-center gap-1 px-2 py-0.5 text-[11px] font-medium rounded-full transition-all border ${
                    selectedSources.has(s)
                      ? "text-white border-transparent shadow-sm"
                      : "bg-white text-slate-500 border-slate-200 hover:border-slate-300"
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
                      background: selectedSources.has(s) ? "white" : getSourceColor(s),
                    }}
                  />
                  {getSourceLabel(s)}
                </button>
              ))}
            </div>

            {/* Vendor filter tags */}
            <div className="flex items-center gap-1 flex-wrap">
              {filters.vendors.map((v) => (
                <button
                  key={v}
                  onClick={() => {
                    toggleInSet(selectedVendors, setSelectedVendors, v);
                    setPage(1);
                  }}
                  className={`inline-flex items-center px-2 py-0.5 text-[11px] font-medium rounded-full transition-all border ${
                    selectedVendors.has(v)
                      ? "bg-primary-600 text-white border-transparent shadow-sm"
                      : "bg-white text-slate-500 border-slate-200 hover:border-slate-300"
                  }`}
                >
                  {v}
                </button>
              ))}
            </div>
          </div>
        </div>
      </header>

      <main className="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8 py-6">
        <div className="mb-4 rounded-lg border border-slate-200 bg-white px-4 py-3 shadow-sm">
          <div className="flex flex-wrap items-center gap-x-4 gap-y-2 text-xs text-slate-600">
            <div className="inline-flex items-center gap-1.5 font-semibold text-slate-700">
              <Filter className="h-3.5 w-3.5 text-slate-400" />
              {ZH.currentFilters}
            </div>
            <span>
              {ZH.timeRange}: <span className="font-medium text-slate-800">{appliedRangeLabel}</span>
            </span>
            <span>
              {ZH.source}: <span className="font-medium text-slate-800">{selectedSourceLabel}</span>
            </span>
            <span>
              {ZH.provider}: <span className="font-medium text-slate-800">{selectedVendorLabel}</span>
            </span>
            {selectedModel && (
              <span>
                {ZH.requestModel}: <span className="font-medium text-slate-800">{selectedModel}</span>
              </span>
            )}
            <span className="ml-auto text-slate-500">
              {loading
                ? ZH.updating
                : `${ZH.updatedAt}: ${lastUpdatedAt ? formatTime(lastUpdatedAt.toISOString()) : "-"}`}
            </span>
          </div>
        </div>

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

            {/* Quota Overview */}
            {(quota || quotaLoading) && (
              <div className="mb-6">
                <div className="grid grid-cols-1 sm:grid-cols-2 gap-4">
                  {/* Kimi balance */}
                  <QuotaCard
                    title="Kimi 余额"
                    provider="kimi"
                    status={
                      quotaLoading
                        ? "loading"
                        : quota?.kimi?.available
                        ? "available"
                        : "unavailable"
                    }
                  >
                    {quotaLoading ? (
                      <div className="h-12 flex items-center justify-center">
                        <div className="animate-spin rounded-full h-4 w-4 border-b-2 border-primary-600" />
                      </div>
                    ) : quota?.kimi?.available && quota.kimi.data ? (
                      <div>
                        <p className="text-xl font-bold text-slate-800">
                          ¥{quota.kimi.data.available_balance.toFixed(2)}
                        </p>
                        <div className="flex gap-3 mt-1 text-xs text-slate-500">
                          <span>现金: ¥{quota.kimi.data.cash_balance.toFixed(2)}</span>
                          <span>赠送: ¥{quota.kimi.data.voucher_balance.toFixed(2)}</span>
                        </div>
                      </div>
                    ) : (
                      <p className="text-xs text-slate-400 italic">
                        {quota?.kimi?.error || "不可用"}
                      </p>
                    )}
                  </QuotaCard>

                  {/* OpenCode-go quota */}
                  <QuotaCard
                    title="OpenCode-go 配额"
                    provider="opencode-go"
                    status={
                      quotaLoading
                        ? "loading"
                        : quota?.opencode_go?.available
                        ? "available"
                        : "unavailable"
                    }
                  >
                    {quotaLoading ? (
                      <div className="h-12 flex items-center justify-center">
                        <div className="animate-spin rounded-full h-4 w-4 border-b-2 border-primary-600" />
                      </div>
                    ) : quota?.opencode_go?.available && quota.opencode_go.data ? (
                      <div>
                        <div className="flex items-center gap-2 mb-1">
                          <span className="text-xs font-medium text-slate-600">
                            {quota.opencode_go.data.plan_type || "No Plan"}
                          </span>
                          {quota.opencode_go.data.hard_limit_usd != null && (
                            <span className="text-[11px] text-slate-400">
                              上限 ${quota.opencode_go.data.hard_limit_usd.toFixed(0)}
                            </span>
                          )}
                        </div>
                        {quota.opencode_go.data.usage_percent != null && (
                          <div className="mt-1">
                            <div className="flex justify-between text-xs text-slate-500 mb-0.5">
                              <span>
                                已用 ${quota.opencode_go.data.total_usage_usd?.toFixed(2) || "?"}
                              </span>
                              <span>{quota.opencode_go.data.usage_percent.toFixed(1)}%</span>
                            </div>
                            <div className="w-full h-1.5 bg-slate-100 rounded-full overflow-hidden">
                              <div
                                className={`h-full rounded-full transition-all ${
                                  quota.opencode_go.data.usage_percent > 80
                                    ? "bg-rose-500"
                                    : quota.opencode_go.data.usage_percent > 50
                                    ? "bg-amber-500"
                                    : "bg-emerald-500"
                                }`}
                                style={{ width: `${Math.min(quota.opencode_go.data.usage_percent, 100)}%` }}
                              />
                            </div>
                          </div>
                        )}
                        {quota.opencode_go.data.remaining_usd != null && (
                          <p className="text-xs text-slate-500 mt-1">
                            剩余: ${quota.opencode_go.data.remaining_usd.toFixed(2)}
                          </p>
                        )}
                      </div>
                    ) : (
                      <p className="text-xs text-slate-400 italic">
                        {quota?.opencode_go?.error || "不可用"}
                      </p>
                    )}
                  </QuotaCard>
                </div>
              </div>
            )}

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

            {/* Vendor & Model Performance */}
            <div className="bg-white rounded-xl border border-slate-200 shadow-sm mb-6">
              <div className="p-5 border-b border-slate-100">
                <h3 className="text-sm font-semibold text-slate-700">
                  {ZH.vendorAndModel}
                </h3>
              </div>
              <div className="overflow-x-auto">
                <table className="w-full text-sm">
                  <thead>
                    <tr className="bg-slate-50 text-slate-500 text-xs uppercase tracking-wider">
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
                    {mergedTableData.map((row, idx) =>
                      row.type === "vendor" ? (
                        <tr
                          key={`vendor-${row.data.provider}`}
                          className="bg-slate-50/80 transition-colors"
                        >
                          <td className="px-4 py-3">
                            <div className="flex items-center gap-2">
                              <span
                                className="w-2.5 h-2.5 rounded-full"
                                style={{
                                  background: getSourceColor(row.data.provider),
                                }}
                              />
                              <span className="font-bold text-slate-800">
                                {row.data.provider}
                              </span>
                            </div>
                          </td>
                          <td className="px-4 py-3 text-xs text-slate-400 italic">
                            汇总
                          </td>
                          <td className="px-4 py-3"></td>
                          <td className="px-4 py-3 text-right font-semibold text-slate-700">
                            {formatNumber(row.data.calls)}
                          </td>
                          <td className="px-4 py-3 text-right text-slate-600">
                            {formatNumber(row.data.input_tokens)}
                          </td>
                          <td className="px-4 py-3 text-right text-slate-600">
                            {formatNumber(row.data.output_tokens)}
                          </td>
                          <td className="px-4 py-3 text-right text-slate-600">
                            {formatNumber(row.data.cache_read_tokens)}
                          </td>
                          <td className="px-4 py-3 text-right text-slate-600">
                            {formatNumber(row.data.cache_write_tokens)}
                          </td>
                          <td className="px-4 py-3 text-right font-bold text-slate-800">
                            {formatNumber(row.data.total_tokens)}
                          </td>
                          <td className="px-4 py-3 text-right">
                            <span
                              className={`px-2 py-0.5 rounded-full text-xs font-medium ${
                                row.data.cache_hit_ratio > 50
                                  ? "bg-emerald-100 text-emerald-700"
                                  : row.data.cache_hit_ratio > 10
                                  ? "bg-amber-100 text-amber-700"
                                  : "bg-slate-100 text-slate-600"
                              }`}
                            >
                              {formatPercent(row.data.cache_hit_ratio)}
                            </span>
                          </td>
                          <td className="px-4 py-3 text-right font-semibold text-slate-700">
                            {formatCost(row.data.cost)}
                          </td>
                        </tr>
                      ) : (
                        <tr
                          key={`model-${row.data.provider}-${row.data.model}-${idx}`}
                          className="hover:bg-slate-50 transition-colors"
                        >
                          <td className="px-4 py-3"></td>
                          <td className="px-4 py-3 font-medium text-slate-700">
                            {row.data.model}
                          </td>
                          <td className="px-4 py-3">
                            <div className="flex flex-wrap gap-1">
                              {(row.data.sources || []).map((s) => (
                                <span
                                  key={s}
                                  className="inline-flex items-center gap-1 px-1.5 py-0.5 rounded text-[10px] font-medium"
                                  style={{
                                    background: `${getSourceColor(s)}15`,
                                    color: getSourceColor(s),
                                  }}
                                >
                                  <span
                                    className="w-1 h-1 rounded-full"
                                    style={{ background: getSourceColor(s) }}
                                  />
                                  {getSourceLabel(s)}
                                </span>
                              ))}
                            </div>
                          </td>
                          <td className="px-4 py-3 text-right text-slate-600">
                            {formatNumber(row.data.calls)}
                          </td>
                          <td className="px-4 py-3 text-right text-slate-600">
                            {formatNumber(row.data.input_tokens)}
                          </td>
                          <td className="px-4 py-3 text-right text-slate-600">
                            {formatNumber(row.data.output_tokens)}
                          </td>
                          <td className="px-4 py-3 text-right text-slate-600">
                            {formatNumber(row.data.cache_read_tokens)}
                          </td>
                          <td className="px-4 py-3 text-right text-slate-600">
                            {formatNumber(row.data.cache_write_tokens)}
                          </td>
                          <td className="px-4 py-3 text-right font-semibold text-slate-700">
                            {formatNumber(row.data.total_tokens)}
                          </td>
                          <td className="px-4 py-3 text-right">
                            <span
                              className={`px-2 py-0.5 rounded-full text-xs font-medium ${
                                row.data.cache_hit_ratio > 50
                                  ? "bg-emerald-100 text-emerald-700"
                                  : row.data.cache_hit_ratio > 10
                                  ? "bg-amber-100 text-amber-700"
                                  : "bg-slate-100 text-slate-600"
                              }`}
                            >
                              {formatPercent(row.data.cache_hit_ratio)}
                            </span>
                          </td>
                          <td className="px-4 py-3 text-right text-slate-600">
                            {formatCost(row.data.cost)}
                          </td>
                        </tr>
                      )
                    )}
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
                          {formatTime(r.time)}
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
