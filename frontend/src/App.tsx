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
  X,
  SlidersHorizontal,
} from "lucide-react";
import {
  fetchStats,
  fetchRequests,
  fetchFilters,
  fetchQuota,
  fetchXunfei,
  type StatsResponse,
  type PaginatedRequests,
  type FilterOptions,
  type QuotaResponse,
  type XunfeiStatus,
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
  isEmptyAppliedSelection,
  type AppliedRange,
} from "./lib/filterState";

const ZH = {
  title: "Token 统计仪表盘",
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
  vendorAndModel: "供应商 & 模型表现",
  detailedRequests: "详细请求",
  allModels: "全部模型",
  last7Days: "最近7天",
  customTime: "自定义",
  last6h: "最近6小时",
  last12h: "最近12小时",
  last1d: "最近1天",
  last3d: "最近3天",
  last14d: "最近14天",
  last30d: "最近30天",
  allTime: "所有",
  provider: "供应商",
  model: "模型",
  source: "工具",
  calls: "调用次数",
  input: "输入",
  output: "输出",
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
  updatedAt: "更新时间",
  updating: "更新中...",
} as const;

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
  const [xunfei, setXunfei] = useState<XunfeiStatus | null>(null);
  const [xunfeiLoading, setXunfeiLoading] = useState(true);
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

  // Helper for progress bar colors based on usage ratio
  const barColor = (used: number, limit: number) => {
    const ratio = used / Math.max(limit, 1);
    if (ratio > 0.8) return "bg-rose-500";
    if (ratio > 0.5) return "bg-amber-500";
    return "bg-emerald-500";
  };

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
        /* quota is optional */
      } finally {
        setQuotaLoading(false);
      }
    };
    loadQuota();
    const interval = setInterval(loadQuota, 60_000);
    return () => clearInterval(interval);
  }, []);

  useEffect(() => {
    const loadXunfei = async () => {
      try {
        const x = await fetchXunfei();
        setXunfei(x);
      } catch {
        /* xunfei is optional */
      } finally {
        setXunfeiLoading(false);
      }
    };
    loadXunfei();
    const interval = setInterval(loadXunfei, 60_000);
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

  return (
    <div className="min-h-screen bg-slate-50">
      {/* Header - compact, sticky */}
      <header className="bg-white border-b border-slate-200 sticky top-0 z-20">
        <div className="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8 py-2">
          <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
            {/* Logo + Title */}
            <div className="flex items-center gap-2 shrink-0">
              <div className="bg-primary-600 p-1.5 rounded-lg">
                <Activity className="w-4 h-4 text-white" />
              </div>
              <h1 className="text-sm font-bold text-slate-800 leading-tight">
                {ZH.title}
              </h1>
            </div>

            {/* Divider */}
            <div className="hidden sm:block w-px h-5 bg-slate-200" />

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

            {/* Spacer + Updated at */}
            <div className="flex-1" />
            <span className="text-[11px] text-slate-400 shrink-0">
              {loading
                ? ZH.updating
                : `${ZH.updatedAt}: ${lastUpdatedAt ? formatTime(lastUpdatedAt.toISOString()) : "-"}`}
            </span>
          </div>
        </div>
      </header>

      <main className="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8 py-4">
        {error && (
          <div className="mb-3 p-3 bg-rose-50 border border-rose-200 rounded-lg text-rose-700 text-xs">
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
            {/* KPI Strip - compact horizontal metrics */}
            <div className="bg-white rounded-xl border border-slate-200 shadow-sm px-4 py-3 mb-3">
              <div className="flex flex-wrap items-center divide-x divide-slate-200 gap-y-1">
                <div className="pr-4 min-w-0">
                  <p className="text-[11px] text-slate-400 font-medium">{ZH.totalCalls}</p>
                  <p className="text-lg font-bold text-slate-800 leading-tight">{formatNumber(stats.overall.total_calls)}</p>
                </div>
                <div className="px-4 min-w-0">
                  <p className="text-[11px] text-slate-400 font-medium">{ZH.inputTokens}</p>
                  <p className="text-lg font-bold text-emerald-600 leading-tight">{formatNumber(stats.overall.total_input_tokens)}</p>
                </div>
                <div className="px-4 min-w-0">
                  <p className="text-[11px] text-slate-400 font-medium">{ZH.outputTokens}</p>
                  <p className="text-lg font-bold text-amber-600 leading-tight">{formatNumber(stats.overall.total_output_tokens)}</p>
                </div>
                <div className="px-4 min-w-0">
                  <p className="text-[11px] text-slate-400 font-medium">{ZH.cacheRead}</p>
                  <p className="text-lg font-bold text-violet-600 leading-tight">{formatNumber(stats.overall.total_cache_read_tokens)}</p>
                </div>
                <div className="px-4 min-w-0">
                  <p className="text-[11px] text-slate-400 font-medium">{ZH.cacheHitRatio}</p>
                  <p className="text-lg font-bold text-rose-600 leading-tight">{formatPercent(stats.overall.weighted_cache_hit_ratio)}</p>
                </div>
                <div className="pl-4 min-w-0">
                  <p className="text-[11px] text-slate-400 font-medium">{ZH.totalCost}</p>
                  <p className="text-lg font-bold text-slate-800 leading-tight">{formatCost(stats.overall.total_cost)}</p>
                </div>
              </div>

              {/* Source Overview - compact inline row */}
              {stats.by_source.length > 1 && (
                <div className="flex flex-wrap items-center gap-x-4 gap-y-1 mt-2 pt-2 border-t border-slate-100 text-xs text-slate-500">
                  {stats.by_source.map((s) => (
                    <span key={s.source} className="inline-flex items-center gap-1.5">
                      <span className="w-1.5 h-1.5 rounded-full" style={{ background: getSourceColor(s.source) }} />
                      <span className="font-medium" style={{ color: getSourceColor(s.source) }}>{getSourceLabel(s.source)}</span>
                      <span className="text-slate-400">{formatNumber(s.calls)}次</span>
                      <span className="text-slate-400">·</span>
                      <span className="text-slate-400">{formatNumber(s.total_tokens)}tok</span>
                      <span className="text-slate-400">·</span>
                      <span className="text-slate-400">{formatCost(s.cost, s.source)}</span>
                      <span className="text-slate-400">·</span>
                      <span className="text-slate-400">{formatPercent(s.cache_hit_ratio)}</span>
                    </span>
                  ))}
                </div>
              )}
            </div>


            {/* Vendor Subscriptions - collapsible (collapsed by default) */}
            <details className="mb-3 group">
              <summary className="bg-white rounded-xl border border-slate-200 shadow-sm px-4 py-2.5 cursor-pointer select-none flex items-center gap-2 text-xs font-semibold text-slate-700 hover:bg-slate-50 transition-colors list-none">
                <svg className="w-3.5 h-3.5 text-slate-400 transition-transform group-open:rotate-90" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 5l7 7-7 7" /></svg>
                供应商订阅
                <span className="text-[11px] text-slate-400 font-normal ml-1">
                  {quotaLoading || xunfeiLoading
                    ? "加载中..."
                    : [
                        xunfei?.available && xunfei.data
                          ? "讯飞: " + (xunfei.data.usage.rp5h_limit > 0 ? "5h " + (xunfei.data.usage.rp5h_used / Math.max(xunfei.data.usage.rp5h_limit, 1) * 100).toFixed(0) + "%, " : "") + "月 " + (xunfei.data.usage.package_used / Math.max(xunfei.data.usage.package_limit, 1) * 100).toFixed(0) + "%"
                          : xunfei && !xunfeiLoading ? "讯飞: 获取失败" : null,
                        quota?.kimi?.available && quota.kimi.data
                          ? "Kimi: " + (quota.kimi.data.rp5h_limit > 0 ? "5h " + (quota.kimi.data.rp5h_used / Math.max(quota.kimi.data.rp5h_limit, 1) * 100).toFixed(0) + "%, " : "") + "周 " + (quota.kimi.data.weekly_used / Math.max(quota.kimi.data.weekly_limit, 1) * 100).toFixed(0) + "%"
                          : quota?.kimi && !quotaLoading ? "Kimi: 获取失败" : null,
                        quota?.opencode_go?.available && quota.opencode_go.data
                          ? "OpenCode: " + (quota.opencode_go.data.usage_percent?.toFixed(0) ?? "?") + "%已用"
                          : quota?.opencode_go?.data?.workspace_url
                            ? "OpenCode: →工作区"
                            : quota?.opencode_go && !quotaLoading ? "OpenCode: 获取失败" : null,
                      ]
                        .filter(Boolean)
                        .join(" · ") || "无可用订阅"}
                </span>
              </summary>
              <div className="mt-1.5 grid grid-cols-1 sm:grid-cols-3 gap-2">
                {/* Xunfei */}
                <div className={"bg-white rounded-xl border " + (xunfei?.available && xunfei.data?.status === "active" ? "border-emerald-200" : "border-slate-200") + " p-3 shadow-sm"}>
                  <div className="flex items-center justify-between mb-1.5">
                    <div className="flex items-center gap-1.5">
                      <span className={"w-1.5 h-1.5 rounded-full " + (xunfeiLoading ? "bg-amber-400" : xunfei?.available && xunfei.data?.status === "active" ? "bg-emerald-500" : "bg-slate-300")} />
                      <span className="text-[11px] font-semibold text-slate-700">讯飞编程套餐</span>
                    </div>
                    <span className="text-[10px] text-slate-400">xfyun.cn</span>
                  </div>
                  {xunfeiLoading ? (
                    <div className="h-8 flex items-center justify-center">
                      <div className="animate-spin rounded-full h-3 w-3 border-b-2 border-primary-600" />
                    </div>
                  ) : xunfei?.available && xunfei.data ? (
                    <>
                      <div className="flex flex-wrap items-center gap-x-2 gap-y-0.5 text-[11px] mb-1.5">
                        <span className="font-bold text-slate-800">{xunfei.data.plan_name}</span>
                        <span className={"px-1 py-0 rounded-full text-[10px] font-medium " + (xunfei.data.status === "active" ? "bg-emerald-100 text-emerald-700" : "bg-slate-100 text-slate-600")}>
                          {xunfei.data.status === "active" ? "有效" : xunfei.data.status}
                        </span>
                        <span className="text-slate-400">¥{(xunfei.data.price / 100).toFixed(2)}/月</span>
                      </div>
                      <div className="space-y-1">
                        <div>
                          <div className="flex justify-between text-[10px] text-slate-500">
                            <span>月度</span>
                            <span>{(xunfei.data.usage.package_used / Math.max(xunfei.data.usage.package_limit, 1) * 100).toFixed(0)}%</span>
                          </div>
                          <div className="w-full h-1 bg-slate-100 rounded-full overflow-hidden">
                            <div className={"h-full rounded-full transition-all " + barColor(xunfei.data.usage.package_used, xunfei.data.usage.package_limit)}
                              style={{ width: (Math.min(xunfei.data.usage.package_used / Math.max(xunfei.data.usage.package_limit, 1) * 100, 100)) + "%" }}
                            />
                          </div>
                        </div>
                        {xunfei.data.usage.rp5h_limit > 0 && (
                          <div>
                            <div className="flex justify-between text-[10px] text-slate-500">
                              <span>5小时</span>
                              <span>{(xunfei.data.usage.rp5h_used / Math.max(xunfei.data.usage.rp5h_limit, 1) * 100).toFixed(0)}%</span>
                            </div>
                            <div className="w-full h-1 bg-slate-100 rounded-full overflow-hidden">
                              <div className={"h-full rounded-full transition-all " + barColor(xunfei.data.usage.rp5h_used, xunfei.data.usage.rp5h_limit)}
                                style={{ width: (Math.min(xunfei.data.usage.rp5h_used / Math.max(xunfei.data.usage.rp5h_limit, 1) * 100, 100)) + "%" }}
                              />
                            </div>
                          </div>
                        )}
                      </div>
                      <div className="flex flex-wrap items-center gap-x-2 gap-y-0.5 mt-1 pt-1 border-t border-slate-100 text-[10px] text-slate-500">
                        <span>余额 ¥{(xunfei.data.balance.cash / 100).toFixed(2)}</span>
                        {xunfei.data.balance.virtual_balance > 0 && <span>赠送 ¥{(xunfei.data.balance.virtual_balance / 100).toFixed(2)}</span>}
                        <span>到期 {xunfei.data.expires_at.replace(" ", "T")}</span>
                      </div>
                    </>
                  ) : (
                    <p className="text-[11px] text-slate-400 italic">获取失败</p>
                  )}
                </div>

                {/* Kimi Code */}
                <div className={"bg-white rounded-xl border " + (quota?.kimi?.available ? "border-emerald-200" : "border-slate-200") + " p-3 shadow-sm"}>
                  <div className="flex items-center justify-between mb-1.5">
                    <div className="flex items-center gap-1.5">
                      <span className={"w-1.5 h-1.5 rounded-full " + (quotaLoading ? "bg-amber-400" : quota?.kimi?.available ? "bg-emerald-500" : "bg-slate-300")} />
                      <span className="text-[11px] font-semibold text-slate-700">Kimi Code</span>
                    </div>
                    <span className="text-[10px] text-slate-400">kimi.com</span>
                  </div>
                  {quotaLoading ? (
                    <div className="h-8 flex items-center justify-center">
                      <div className="animate-spin rounded-full h-3 w-3 border-b-2 border-primary-600" />
                    </div>
                  ) : quota?.kimi?.available && quota.kimi.data ? (
                    <>
                      <div className="flex items-center gap-2 text-[11px] mb-1">
                        <span className="font-medium text-slate-600">
                          {quota.kimi.data.sub_type === "TYPE_PURCHASE" ? "付费版" : quota.kimi.data.membership_level || "免费版"}
                        </span>
                        <span className="text-slate-400">并发 {quota.kimi.data.parallel_limit}</span>
                      </div>
                      <div className="space-y-1">
                        <div>
                          <div className="flex justify-between text-[10px] text-slate-500">
                            <span>周限额</span>
                            <span>{(quota.kimi.data.weekly_used / Math.max(quota.kimi.data.weekly_limit, 1) * 100).toFixed(0)}%</span>
                          </div>
                          <div className="w-full h-1 bg-slate-100 rounded-full overflow-hidden">
                            <div className={"h-full rounded-full transition-all " + barColor(quota.kimi.data.weekly_used, quota.kimi.data.weekly_limit)}
                              style={{ width: (Math.min(quota.kimi.data.weekly_used / Math.max(quota.kimi.data.weekly_limit, 1) * 100, 100)) + "%" }}
                            />
                          </div>
                        </div>
                        {quota.kimi.data.rp5h_limit > 0 && (
                          <div>
                            <div className="flex justify-between text-[10px] text-slate-500">
                              <span>5小时</span>
                              <span>{(quota.kimi.data.rp5h_used / Math.max(quota.kimi.data.rp5h_limit, 1) * 100).toFixed(0)}%</span>
                            </div>
                            <div className="w-full h-1 bg-slate-100 rounded-full overflow-hidden">
                              <div className={"h-full rounded-full transition-all " + barColor(quota.kimi.data.rp5h_used, quota.kimi.data.rp5h_limit)}
                                style={{ width: (Math.min(quota.kimi.data.rp5h_used / Math.max(quota.kimi.data.rp5h_limit, 1) * 100, 100)) + "%" }}
                              />
                            </div>
                          </div>
                        )}
                      </div>
                      {quota.kimi.data.total_limit > 0 && (
                        <div className="mt-1 pt-1 border-t border-slate-100 text-[10px] text-slate-500">
                          总配额 {quota.kimi.data.total_remaining}/{quota.kimi.data.total_limit}
                        </div>
                      )}
                    </>
                  ) : (
                    <p className="text-[11px] text-slate-400 italic">获取失败</p>
                  )}
                </div>

                {/* OpenCode-go */}
                <div className={"bg-white rounded-xl border " + (quota?.opencode_go?.available ? "border-emerald-200" : "border-slate-200") + " p-3 shadow-sm"}>
                  <div className="flex items-center justify-between mb-1.5">
                    <div className="flex items-center gap-1.5">
                      <span className={"w-1.5 h-1.5 rounded-full " + (quotaLoading ? "bg-amber-400" : quota?.opencode_go?.available ? "bg-emerald-500" : "bg-slate-300")} />
                      <span className="text-[11px] font-semibold text-slate-700">OpenCode-go</span>
                    </div>
                    <span className="text-[10px] text-slate-400">opencode.ai</span>
                  </div>
                  {quotaLoading ? (
                    <div className="h-8 flex items-center justify-center">
                      <div className="animate-spin rounded-full h-3 w-3 border-b-2 border-primary-600" />
                    </div>
                  ) : quota?.opencode_go?.available && quota.opencode_go.data ? (
                    <>
                      <div className="flex items-center gap-2 text-[11px] mb-1">
                        <span className="font-medium text-slate-600">
                          {quota.opencode_go.data.plan_type || "No Plan"}
                        </span>
                        {quota.opencode_go.data.hard_limit_usd != null && (
                          <span className="text-slate-400">上限 ${quota.opencode_go.data.hard_limit_usd.toFixed(0)}</span>
                        )}
                      </div>
                      {quota.opencode_go.data.usage_percent != null ? (
                        <div>
                          <div className="flex justify-between text-[10px] text-slate-500">
                            <span>已用 ${quota.opencode_go.data.total_usage_usd?.toFixed(2) ?? "?"}</span>
                            <span>{quota.opencode_go.data.usage_percent.toFixed(0)}%</span>
                          </div>
                          <div className="w-full h-1 bg-slate-100 rounded-full overflow-hidden">
                            <div className={"h-full rounded-full transition-all " + (quota.opencode_go.data.usage_percent > 80 ? "bg-rose-500" : quota.opencode_go.data.usage_percent > 50 ? "bg-amber-500" : "bg-emerald-500")}
                              style={{ width: (Math.min(quota.opencode_go.data.usage_percent, 100)) + "%" }}
                            />
                          </div>
                          {quota.opencode_go.data.remaining_usd != null && (
                            <span className="text-[10px] text-slate-500">剩余 ${quota.opencode_go.data.remaining_usd.toFixed(2)}</span>
                          )}
                        </div>
                      ) : quota.opencode_go.data.workspace_url ? (
                        <a
                          href={quota.opencode_go.data.workspace_url}
                          target="_blank"
                          rel="noopener noreferrer"
                          className="inline-flex items-center gap-1 text-[11px] text-primary-600 hover:text-primary-700"
                        >
                          查看工作区 →
                        </a>
                      ) : null}
                    </>
                  ) : (
                    <>
                      {quota?.opencode_go?.data?.workspace_url ? (
                        <a
                          href={quota.opencode_go.data.workspace_url}
                          target="_blank"
                          rel="noopener noreferrer"
                          className="inline-flex items-center gap-1 text-[11px] text-primary-600 hover:text-primary-700"
                        >
                          查看工作区 →
                        </a>
                      ) : null}
                      <p className="text-[11px] text-slate-400 italic">获取失败</p>
                    </>
                  )}
                </div>
              </div>
            </details>

            {/* Charts Row - reduced height, pie merged into vendor card */}

            <div className="grid grid-cols-1 lg:grid-cols-2 gap-3 mb-3">
              {/* Daily Trends + Cache Hit Ratio */}
              <div className="bg-white rounded-xl border border-slate-200 p-4 shadow-sm">
                <h3 className="text-xs font-semibold text-slate-700 mb-2">
                  {ZH.dailyTokenUsage} & {ZH.cacheHitTrend}
                </h3>
                <ResponsiveContainer width="100%" height={240}>
                  <LineChart data={chartData}>
                    <CartesianGrid strokeDasharray="3 3" stroke="#e2e8f0" />
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
                    <YAxis
                      yAxisId="ratio"
                      orientation="right"
                      tick={{ fontSize: 10, fill: "#f43f5e" }}
                      domain={[0, 100]}
                      unit="%"
                      width={40}
                    />
                    <Tooltip content={<CustomTooltip />} />
                    <Legend wrapperStyle={{ fontSize: 11 }} />
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

              {/* Vendor Breakdown + Distribution (pie merged in) */}
              <div className="bg-white rounded-xl border border-slate-200 p-4 shadow-sm">
                <h3 className="text-xs font-semibold text-slate-700 mb-2">
                  {ZH.vendorBreakdown} & {ZH.tokenDistribution}
                </h3>
                <div className="flex items-center gap-2">
                  <div className="flex-1">
                    <ResponsiveContainer width="100%" height={220}>
                      <BarChart data={vendorChartData} layout="vertical">
                        <CartesianGrid strokeDasharray="3 3" stroke="#e2e8f0" />
                        <XAxis
                          type="number"
                          tick={{ fontSize: 10, fill: "#64748b" }}
                          tickFormatter={(v: number) => formatNumber(v)}
                        />
                        <YAxis
                          type="category"
                          dataKey="name"
                          tick={{ fontSize: 10, fill: "#64748b" }}
                          width={80}
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
                  <div className="w-[140px] shrink-0">
                    <ResponsiveContainer width="100%" height={220}>
                      <PieChart>
                        <Pie
                          data={pieData}
                          cx="50%"
                          cy="50%"
                          innerRadius={40}
                          outerRadius={65}
                          paddingAngle={3}
                          dataKey="value"
                        >
                          {pieData.map((_entry, i) => (
                            <Cell key={i} />
                          ))}
                        </Pie>
                        <Tooltip content={<PieTooltip />} />
                      </PieChart>
                    </ResponsiveContainer>
                  </div>
                </div>
              </div>
            </div>

            {/* Vendor & Model Performance - compact table, merged cache columns */}
            <div className="bg-white rounded-xl border border-slate-200 shadow-sm mb-3">
              <div className="px-4 py-2.5 border-b border-slate-100">
                <h3 className="text-xs font-semibold text-slate-700">
                  {ZH.vendorAndModel}
                </h3>
              </div>
              <div className="overflow-x-auto">
                <table className="w-full text-xs">
                  <thead>
                    <tr className="bg-slate-50 text-slate-500 text-[10px] uppercase tracking-wider">
                      <th className="px-3 py-2 text-left font-medium">{ZH.provider}</th>
                      <th className="px-3 py-2 text-left font-medium">{ZH.model}</th>
                      <th className="px-3 py-2 text-left font-medium">{ZH.source}</th>
                      <th className="px-3 py-2 text-right font-medium">{ZH.calls}</th>
                      <th className="px-3 py-2 text-right font-medium">{ZH.input}</th>
                      <th className="px-3 py-2 text-right font-medium">{ZH.output}</th>
                      <th className="px-3 py-2 text-right font-medium">缓存</th>
                      <th className="px-3 py-2 text-right font-medium">{ZH.total}</th>
                      <th className="px-3 py-2 text-right font-medium">{ZH.cacheHit}</th>
                      <th className="px-3 py-2 text-right font-medium">{ZH.cost}</th>
                    </tr>
                  </thead>
                  <tbody className="divide-y divide-slate-100">
                    {mergedTableData.map((row, idx) =>
                      row.type === "vendor" ? (
                        <tr key={`vendor-${row.data.provider}`} className="bg-slate-50/80 transition-colors">
                          <td className="px-3 py-2">
                            <div className="flex items-center gap-1.5">
                              <span className="w-2 h-2 rounded-full" style={{ background: getSourceColor(row.data.provider) }} />
                              <span className="font-bold text-slate-800">{row.data.provider}</span>
                            </div>
                          </td>
                          <td className="px-3 py-2 text-[10px] text-slate-400 italic">汇总</td>
                          <td className="px-3 py-2"></td>
                          <td className="px-3 py-2 text-right font-semibold text-slate-700">{formatNumber(row.data.calls)}</td>
                          <td className="px-3 py-2 text-right text-slate-600">{formatNumber(row.data.input_tokens)}</td>
                          <td className="px-3 py-2 text-right text-slate-600">{formatNumber(row.data.output_tokens)}</td>
                          <td className="px-3 py-2 text-right text-slate-600">{formatNumber(row.data.cache_read_tokens + row.data.cache_write_tokens)}</td>
                          <td className="px-3 py-2 text-right font-bold text-slate-800">{formatNumber(row.data.total_tokens)}</td>
                          <td className="px-3 py-2 text-right">
                            <span className={`px-1.5 py-0.5 rounded-full text-[10px] font-medium ${
                              row.data.cache_hit_ratio > 50 ? "bg-emerald-100 text-emerald-700"
                              : row.data.cache_hit_ratio > 10 ? "bg-amber-100 text-amber-700"
                              : "bg-slate-100 text-slate-600"
                            }`}>{formatPercent(row.data.cache_hit_ratio)}</span>
                          </td>
                          <td className="px-3 py-2 text-right font-semibold text-slate-700">{formatCost(row.data.cost)}</td>
                        </tr>
                      ) : (
                        <tr key={`model-${row.data.provider}-${row.data.model}-${idx}`} className="hover:bg-slate-50 transition-colors">
                          <td className="px-3 py-2"></td>
                          <td className="px-3 py-2 font-medium text-slate-700">{row.data.model}</td>
                          <td className="px-3 py-2">
                            <div className="flex flex-wrap gap-0.5">
                              {(row.data.sources || []).map((s) => (
                                <span key={s} className="inline-flex items-center gap-0.5 px-1 py-0.5 rounded text-[9px] font-medium"
                                  style={{ background: `${getSourceColor(s)}15`, color: getSourceColor(s) }}>
                                  {getSourceLabel(s)}
                                </span>
                              ))}
                            </div>
                          </td>
                          <td className="px-3 py-2 text-right text-slate-600">{formatNumber(row.data.calls)}</td>
                          <td className="px-3 py-2 text-right text-slate-600">{formatNumber(row.data.input_tokens)}</td>
                          <td className="px-3 py-2 text-right text-slate-600">{formatNumber(row.data.output_tokens)}</td>
                          <td className="px-3 py-2 text-right text-slate-600">{formatNumber(row.data.cache_read_tokens + row.data.cache_write_tokens)}</td>
                          <td className="px-3 py-2 text-right font-semibold text-slate-700">{formatNumber(row.data.total_tokens)}</td>
                          <td className="px-3 py-2 text-right">
                            <span className={`px-1.5 py-0.5 rounded-full text-[10px] font-medium ${
                              row.data.cache_hit_ratio > 50 ? "bg-emerald-100 text-emerald-700"
                              : row.data.cache_hit_ratio > 10 ? "bg-amber-100 text-amber-700"
                              : "bg-slate-100 text-slate-600"
                            }`}>{formatPercent(row.data.cache_hit_ratio)}</span>
                          </td>
                          <td className="px-3 py-2 text-right text-slate-600">{formatCost(row.data.cost)}</td>
                        </tr>
                      )
                    )}
                  </tbody>
                </table>
              </div>
            </div>

            {/* Detailed Requests - collapsible (collapsed by default) */}
            <details className="group">
              <summary className="bg-white rounded-xl border border-slate-200 shadow-sm px-4 py-2.5 cursor-pointer select-none flex items-center gap-2 text-xs font-semibold text-slate-700 hover:bg-slate-50 transition-colors list-none">
                <svg className="w-3.5 h-3.5 text-slate-400 transition-transform group-open:rotate-90" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 5l7 7-7 7" /></svg>
                {ZH.detailedRequests}
                <span className="text-[11px] text-slate-400 font-normal ml-1">
                  {requests ? `${formatNumber(requests.total)} 条` : ""}
                </span>
                <div className="ml-auto flex items-center gap-2" onClick={(e) => e.stopPropagation()}>
                  <Filter className="w-3.5 h-3.5 text-slate-400" />
                  <select
                    value={selectedModel}
                    onChange={(e) => {
                      setSelectedModel(e.target.value);
                      setPage(1);
                    }}
                    className="px-2 py-1 text-xs border border-slate-200 rounded focus:ring-1 focus:ring-primary-500 outline-none bg-white"
                  >
                    <option value="">{ZH.allModels}</option>
                    {filters.models.map((m) => (
                      <option key={m} value={m}>{m}</option>
                    ))}
                  </select>
                </div>
              </summary>
              <div className="mt-1.5 bg-white rounded-xl border border-slate-200 shadow-sm overflow-x-auto">
                <table className="w-full text-xs">
                  <thead>
                    <tr className="bg-slate-50 text-slate-500 text-[10px] uppercase tracking-wider">
                      <th className="px-3 py-2 text-left font-medium">{ZH.date}</th>
                      <th className="px-3 py-2 text-left font-medium">{ZH.provider}</th>
                      <th className="px-3 py-2 text-left font-medium">{ZH.model}</th>
                      <th className="px-3 py-2 text-left font-medium">{ZH.source}</th>
                      <th className="px-3 py-2 text-right font-medium">{ZH.input}</th>
                      <th className="px-3 py-2 text-right font-medium">{ZH.output}</th>
                      <th className="px-3 py-2 text-right font-medium">缓存</th>
                      <th className="px-3 py-2 text-right font-medium">{ZH.total}</th>
                      <th className="px-3 py-2 text-right font-medium">{ZH.cacheHit}</th>
                      <th className="px-3 py-2 text-right font-medium">{ZH.cost}</th>
                    </tr>
                  </thead>
                  <tbody className="divide-y divide-slate-100">
                    {requests?.data.map((r, i) => (
                      <tr key={i} className="hover:bg-slate-50 transition-colors">
                        <td className="px-3 py-2 text-slate-600 whitespace-nowrap">{formatTime(r.time)}</td>
                        <td className="px-3 py-2">
                          <span className="text-xs font-medium" style={{ color: getSourceColor(r.provider) }}>{r.provider}</span>
                        </td>
                        <td className="px-3 py-2 text-slate-600">{r.model}</td>
                        <td className="px-3 py-2">
                          <span className="inline-flex items-center gap-1 px-1.5 py-0.5 rounded text-[9px] font-medium"
                            style={{ background: `${getSourceColor(r.source)}15`, color: getSourceColor(r.source) }}>
                            {getSourceLabel(r.source)}
                          </span>
                        </td>
                        <td className="px-3 py-2 text-right text-slate-600">{formatNumber(r.input_tokens)}</td>
                        <td className="px-3 py-2 text-right text-slate-600">{formatNumber(r.output_tokens)}</td>
                        <td className="px-3 py-2 text-right text-slate-600">{formatNumber(r.cache_read_tokens + r.cache_write_tokens)}</td>
                        <td className="px-3 py-2 text-right font-semibold text-slate-700">{formatNumber(r.total_tokens)}</td>
                        <td className="px-3 py-2 text-right">
                          <span className={`px-1.5 py-0.5 rounded-full text-[10px] font-medium ${
                            r.cache_hit_ratio > 50 ? "bg-emerald-100 text-emerald-700"
                            : r.cache_hit_ratio > 10 ? "bg-amber-100 text-amber-700"
                            : "bg-slate-100 text-slate-600"
                          }`}>{formatPercent(r.cache_hit_ratio)}</span>
                        </td>
                        <td className="px-3 py-2 text-right text-slate-600">{formatCost(r.cost, r.source)}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>

                {/* Pagination */}
                {requests && requests.total_pages > 1 && (
                  <div className="px-3 py-2 border-t border-slate-100 flex items-center justify-between">
                    <p className="text-xs text-slate-500">
                      {ZH.showing} {(requests.page - 1) * requests.limit + 1}-
                      {Math.min(requests.page * requests.limit, requests.total)} {ZH.of} {formatNumber(requests.total)} {ZH.requests}
                    </p>
                    <div className="flex items-center gap-1">
                      <button
                        onClick={() => setPage((p) => Math.max(1, p - 1))}
                        disabled={requests.page <= 1}
                        className="p-1 rounded hover:bg-slate-100 disabled:opacity-40 disabled:cursor-not-allowed transition-colors"
                      >
                        <ChevronLeft className="w-3.5 h-3.5" />
                      </button>
                      {Array.from({ length: Math.min(5, requests.total_pages) }, (_, i) => {
                        let pageNum: number;
                        if (requests.total_pages <= 5) pageNum = i + 1;
                        else if (requests.page <= 3) pageNum = i + 1;
                        else if (requests.page >= requests.total_pages - 2) pageNum = requests.total_pages - 4 + i;
                        else pageNum = requests.page - 2 + i;
                        return (
                          <button key={pageNum} onClick={() => setPage(pageNum)}
                            className={`w-7 h-7 rounded text-xs font-medium transition-colors ${
                              pageNum === requests.page ? "bg-primary-600 text-white" : "hover:bg-slate-100 text-slate-600"
                            }`}>
                            {pageNum}
                          </button>
                        );
                      })}
                      <button
                        onClick={() => setPage((p) => Math.min(requests.total_pages, p + 1))}
                        disabled={requests.page >= requests.total_pages}
                        className="p-1 rounded hover:bg-slate-100 disabled:opacity-40 disabled:cursor-not-allowed transition-colors"
                      >
                        <ChevronRight className="w-3.5 h-3.5" />
                      </button>
                    </div>
                  </div>
                )}
              </div>
            </details>

            <footer className="mt-4 text-center text-xs text-slate-400 pb-4">
              {ZH.footer}
            </footer>
          </>
        )}
      </main>
    </div>
  );
}