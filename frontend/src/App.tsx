import { useState, useEffect, useMemo, useRef, useCallback, Fragment } from "react";
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
import {
  ChevronLeft,
  ChevronRight,
  ChevronsLeft,
  ChevronsRight,
  ChevronDown,
  ChevronRight as ChevronRightIcon,
  Filter,
  Activity,
  X,
  SlidersHorizontal,
  Download,
  Upload,
  Receipt,
  Settings,
  Calendar,
} from "lucide-react";
import {
  fetchStats,
  fetchRequests,
  fetchFilters,
  fetchQuota,
  fetchXunfei,
  fetchAinaibaCredit,
  fetchRefresh,
  fetchPricing,
  fetchAdvancedModels,
  saveAdvancedModels,
  fetchSubscriptionSettings,
  saveSubscriptionSettings,
  type PricingConfig,
  exportBackup,
  restoreBackup,
  type StatsResponse,
  type PaginatedRequests,
  type FilterOptions,
  type QuotaResponse,
  type XunfeiMultiStatus,
  type AinaibaCreditResponse,
  type RestoreResponse,
  type SubscriptionSettings,
  type OpenCodeQuotaStatus,
} from "./api";
import {
  formatNumber,
  formatCalls,
  formatCost,
  formatPercent,
  formatDate,
  formatTime,
  getLocalToday,
  getLocalDateOffset,
  getLocalDatetimeOffsetHours,
  getSourceColor,
  getSourceLabel,
  getVendorColor,
  formatResetTime,
  formatAvgCost,
  computeNextBillingDate,
  isWithin24Hours,
} from "./lib/utils";
import {
  buildPivotTree,
  computePivotSummary,
  getDisplayModel,
  getOriginalModels,
  type SortColumn,
  type SortDirection,
} from "./lib/pivotTable";
import {
  buildCsvFilterParam,
  isEmptyAppliedSelection,
  type AppliedRange,
} from "./lib/filterState";

const ZH = {
  title: "Token 统计仪表盘",
  totalCalls: "总调用次数",
  totalTokens: "总 Token",
  inputTokens: "输入 Token",
  outputTokens: "输出 Token",
  cacheRead: "缓存读取",
  cacheHitRatio: "缓存命中率",
  weighted: "加权",
  totalCost: "总费用",
  dailyTokenUsage: "每日 Token 用量",
  hourlyRequests: "每小时请求数",
  vendorBreakdown: "供应商分布",
  tokenDistribution: "Token 分布",
  vendorAndModel: "供应商 & 模型表现",
  hideFreeModels: "过滤免费",
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
  avgCost: "平均成本",
  avgCostUnit: "元/百万Token",
  modelFilter: "模型筛选",
  selectAll: "全选",
  clearAll: "清除",
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
  cacheHitNoXunfei: "缓存命中率(无讯飞)",
  xunfeiNoCacheNote: "讯飞无缓存机制",
  subscription: "订阅",
  chartMetrics: "图表指标",
  cacheLabel: "缓存",
} as const;

const CHART_METRIC_OPTIONS = [
  { key: "cache", label: "缓存", color: "#c084fc" },
  { key: "input", label: "输入", color: "#38bdf8" },
  { key: "output", label: "输出", color: "#fb923c" },
  { key: "cacheHitRatio", label: "缓存命中率", color: "#f472b6" },
  { key: "cacheHitRatioNoXunfei", label: "缓存命中率(无讯飞)", color: "#22d3ee" },
] as const;

type ChartMetricKey = (typeof CHART_METRIC_OPTIONS)[number]["key"];

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
      {payload.map((p, i) => {
        const isRatio = p.name?.includes("命中率");
        return (
          <p key={i} className="text-slate-600">
            <span
              className="inline-block w-2 h-2 rounded-full mr-1.5"
              style={{ background: p.color }}
            />
            {p.name}: {isRatio ? formatPercent(Number(p.value ?? 0)) : formatNumber(Number(p.value ?? 0))}
          </p>
        );
      })}
    </div>
  );
}

interface ChartTooltipPayload {
  name?: string;
  value?: number | string;
  color?: string;
  percent?: number;
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
  const [activePreset, setActivePreset] = useState<TimePreset>("today");
  const [appliedRange, setAppliedRange] = useState<AppliedRange>(() => makeAppliedRange("today"));
  const [customFrom, setCustomFrom] = useState<string>("");
  const [customTo, setCustomTo] = useState<string>("");
  const [showCustomPanel, setShowCustomPanel] = useState(false);
  const customBtnRef = useRef<HTMLButtonElement>(null);
  const filtersInitializedRef = useRef(false);

  // ─── Alert Item Type ──────────────────────────────────────────────────────
  interface AlertItem {
    id: string;
    provider: string;
    type: "quota_low" | "expiring_soon";
    message: string;
    detail: string;
  }

  const [stats, setStats] = useState<StatsResponse | null>(null);
  const [requests, setRequests] = useState<PaginatedRequests | null>(null);
  const [quota, setQuota] = useState<QuotaResponse | null>(null);
  const [quotaLoading, setQuotaLoading] = useState(true);
  const [xunfei, setXunfei] = useState<XunfeiMultiStatus | null>(null);
  const [xunfeiLoading, setXunfeiLoading] = useState(true);
  const [ainaibaCredit, setAinaibaCredit] = useState<AinaibaCreditResponse | null>(null);
  const [ainaibaCreditLoading, setAinaibaCreditLoading] = useState(true);

  // Subscription settings & alerts
  const [subscriptionSettings, setSubscriptionSettings] = useState<SubscriptionSettings | null>(null);
  const [showSubscriptionSettings, setShowSubscriptionSettings] = useState(false);
  const [alertItems, setAlertItems] = useState<AlertItem[]>([]);
  const [showAlertModal, setShowAlertModal] = useState(false);
  const [dismissedAlerts, setDismissedAlerts] = useState<Set<string>>(new Set());
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

  // Pivot table expand/collapse state
  const [expandedVendors, setExpandedVendors] = useState<Set<string>>(new Set());
  const [expandedModels, setExpandedModels] = useState<Set<string>>(new Set());
  const pivotInitRef = useRef(false);

  // Pivot table model filter (multi-select dropdown)
  const [selectedPivotModels, setSelectedPivotModels] = useState<Set<string>>(new Set());
  const [pendingPivotModels, setPendingPivotModels] = useState<Set<string>>(new Set());
  const [showModelFilter, setShowModelFilter] = useState(false);

  // Chart metric filter
  const [chartMetrics, setChartMetrics] = useState<Set<ChartMetricKey>>(
    () => new Set(["cache", "input", "output", "cacheHitRatio"])
  );
  const [showChartFilter, setShowChartFilter] = useState(false);

  // Backup / restore
  const [showRestore, setShowRestore] = useState(false);
  const [restorePath, setRestorePath] = useState("");
  const [restoreResult, setRestoreResult] = useState<RestoreResponse | null>(null);
  const [restoreLoading, setRestoreLoading] = useState(false);
  const [restoreError, setRestoreError] = useState<string | null>(null);

  // Pricing logic modal
  const [showPricing, setShowPricing] = useState(false);
  const [pricingConfig, setPricingConfig] = useState<PricingConfig | null>(null);

  // Hourly stats (always 1h resolution)
  const [hourlyStats, setHourlyStats] = useState<StatsResponse | null>(null);

  // Pivot table sorting
  const [sortColumn, setSortColumn] = useState<SortColumn>("total_tokens");
  const [sortDirection, setSortDirection] = useState<SortDirection>("desc");

  // Free model filter
  const [hideFreeModels, setHideFreeModels] = useState(false);

  // Advanced models settings
  const [advancedModels, setAdvancedModels] = useState<string[]>([]);
  const [showAdvancedSettings, setShowAdvancedSettings] = useState(false);
  const [advancedModelsDraft, setAdvancedModelsDraft] = useState("");

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

  // Pivot table model filter — uses unfiltered model list from /api/filters
  // so dropdown always shows all models regardless of current filter state
  const pivotModelOptions = useMemo(() => {
    return [...filters.models].sort();
  }, [filters.models]);

  const modelFilter = useMemo(() => {
    if (selectedPivotModels.size === 0) return undefined;
    return [...selectedPivotModels].join(",");
  }, [selectedPivotModels]);

  const hasEmptySourceSelection = useMemo(
    () => isEmptyAppliedSelection(selectedSources, filters.sources),
    [selectedSources, filters.sources]
  );
  const hasEmptyVendorSelection = useMemo(
    () => isEmptyAppliedSelection(selectedVendors, filters.vendors),
    [selectedVendors, filters.vendors]
  );
  const hasEmptyRequiredSelection = hasEmptySourceSelection || hasEmptyVendorSelection;

  /** Determine the resolution based on the time range
   *  < 4h  → 1h buckets
   *  < 1d  → 2h buckets
   *  < 3d  → 12h (half-day) buckets
   *  >= 3d → day buckets (default)
   */
  const resolution = useMemo(() => {
    if (!effectiveRange.from || !effectiveRange.to) return undefined;
    const fromMs = new Date(effectiveRange.from).getTime();
    const toMs = new Date(effectiveRange.to).getTime();
    if (isNaN(fromMs) || isNaN(toMs)) return undefined;
    const rangeMs = toMs - fromMs;
    const oneDayMs = 24 * 60 * 60 * 1000;
    if (rangeMs < 4 * 60 * 60 * 1000) return "1h";
    if (rangeMs < oneDayMs) return "2h";
    if (rangeMs < 3 * oneDayMs) return "12h";
    return undefined; // default = day
  }, [effectiveRange.from, effectiveRange.to]);

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
          tzOffset,
          resolution,
          modelFilter
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
      // Initialize pivot expand state on first data load
      if (s.by_model && !pivotInitRef.current) {
        const vendors = new Set(s.by_model.map((m) => m.provider));
        const models = new Set(s.by_model.map((m) => `${m.provider}|${getDisplayModel(m.model)}`));
        setExpandedVendors(vendors);
        setExpandedModels(models);
        pivotInitRef.current = true;
      }
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
    resolution,
    modelFilter,
  ]);

  // Filtered models derived from stats (respects source/vendor filters, with merging)
  const filteredModels = useMemo(() => {
    const rawModels = stats?.by_model
      ? [...new Set(stats.by_model.map((m) => m.model))]
      : filters.models;
    return [...new Set(rawModels.map(getDisplayModel))].sort();
  }, [stats, filters.models]);

  // Effective model: if selectedModel is no longer in filtered set, treat as empty
  const effectiveModel = useMemo(() => {
    if (!selectedModel) return "";
    if (filteredModels.includes(selectedModel)) return selectedModel;
    return "";
  }, [selectedModel, filteredModels]);

  const loadRequests = useCallback(async () => {
    if (!effectiveRange.from || !effectiveRange.to) return;

    if (hasEmptyRequiredSelection) {
      setRequests(emptyRequests(1));
      return;
    }

    try {
      const modelParam = effectiveModel
        ? (getOriginalModels(effectiveModel)?.join(",") || effectiveModel)
        : undefined;
      const r = await fetchRequests(
        effectiveRange.from,
        effectiveRange.to,
        vendorFilter,
        modelParam,
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
    effectiveModel,
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

  // Auto-refresh data every 30 seconds
  useEffect(() => {
    const interval = setInterval(() => {
      void loadData();
      void loadRequests();
    }, 30000);
    return () => clearInterval(interval);
  }, [loadData, loadRequests]);

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
    const interval = setInterval(loadQuota, 30_000);
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
    const interval = setInterval(loadXunfei, 30_000);
    return () => clearInterval(interval);
  }, []);

  useEffect(() => {
    const loadAinaiba = async () => {
      try {
        const x = await fetchAinaibaCredit();
        setAinaibaCredit(x);
      } catch {
        /* ainaiba credit is optional */
      } finally {
        setAinaibaCreditLoading(false);
      }
    };
    loadAinaiba();
    const interval = setInterval(loadAinaiba, 30_000);
    return () => clearInterval(interval);
  }, []);

  // Force refresh backend data on page load
  useEffect(() => {
    const doRefresh = async () => {
      try {
        await fetchRefresh();
        void loadData();
        void loadRequests();
      } catch {
        /* refresh is best-effort */
      }
    };
    doRefresh();
  }, [loadData, loadRequests]);

  // Load pricing config once on mount
  useEffect(() => {
    const loadPricing = async () => {
      try {
        const cfg = await fetchPricing();
        setPricingConfig(cfg);
      } catch {
        /* pricing config is optional */
      }
    };
    loadPricing();
  }, []);

  // Load advanced models config once on mount
  useEffect(() => {
    fetchAdvancedModels().then(setAdvancedModels).catch(() => {});
  }, []);

  // Load subscription settings once on mount
  useEffect(() => {
    fetchSubscriptionSettings()
      .then(setSubscriptionSettings)
      .catch(() => {});
  }, []);

  // ─── Subscription Alert Checking ──────────────────────────────────────────
  useEffect(() => {
    if (!quota && !xunfei && !ainaibaCredit) return;

    const alerts: AlertItem[] = [];

    // Kimi quota low
    if (quota?.kimi?.available && quota.kimi.data) {
      const kd = quota.kimi.data;
      const ratio = kd.weekly_remaining / Math.max(kd.weekly_limit, 1);
      if (ratio <= 0.2) {
        alerts.push({
          id: "kimi_quota_low",
          provider: "Kimi",
          type: "quota_low",
          message: "Kimi 周限额余量不足",
          detail: `周限额已用 ${((1 - ratio) * 100).toFixed(0)}%，建议切换至其他模型以节省额度`,
        });
      }
    }

    // Kimi expiration
    if (subscriptionSettings?.kimi_monthly_start_day) {
      const nextBilling = computeNextBillingDate(subscriptionSettings.kimi_monthly_start_day);
      if (isWithin24Hours(nextBilling.toISOString())) {
        alerts.push({
          id: "kimi_expiring",
          provider: "Kimi",
          type: "expiring_soon",
          message: "Kimi 订阅即将到期",
          detail: `下次计费日: ${nextBilling.toLocaleDateString()}，请注意续费`,
        });
      }
    }

    // OpenCode-go (merged quota_low + expiring checks)
    const checkOpenCode = (status: OpenCodeQuotaStatus | null | undefined, label: string) => {
      if (!status?.available || !status.data) return;
      const suffix = label === "ex" ? " (EX)" : "";
      for (const entry of status.data.entries) {
        // Quota low check
        if (entry.percentage >= 80) {
          const typeLabel = entry.usage_type === "Rolling" ? "滚动" : entry.usage_type === "Weekly" ? "周" : entry.usage_type === "Monthly" ? "月" : entry.usage_type;
          alerts.push({
            id: `opencode_${label}_${entry.usage_type}_low`,
            provider: `OpenCode-go${suffix}`,
            type: "quota_low",
            message: `OpenCode-go${suffix} ${typeLabel}限额已用 ${entry.percentage}%`,
            detail: `重置于 ${entry.resets_in}`,
          });
        }
        // Expiration check (only for Monthly entries)
        if (entry.usage_type === "Monthly" && entry.reset_at && isWithin24Hours(entry.reset_at)) {
          alerts.push({
            id: `opencode_${label}_expiring`,
            provider: `OpenCode-go${suffix}`,
            type: "expiring_soon",
            message: `OpenCode-go${suffix} 月度配额即将重置`,
            detail: `重置于 ${new Date(entry.reset_at).toLocaleString()}`,
          });
        }
      }
    };
    checkOpenCode(quota?.opencode_go, "primary");
    checkOpenCode(quota?.opencode_go_ex, "ex");

    // Xunfei (merged quota_low + expiring checks)
    if (xunfei?.accounts) {
      for (const acc of xunfei.accounts) {
        const suffix = acc.label === "ex" ? " (EX)" : "";
        if (acc.available && acc.data) {
          // Quota low check
          const ratio = acc.data.usage.package_left / Math.max(acc.data.usage.package_limit, 1);
          if (ratio <= 0.2) {
            alerts.push({
              id: `xunfei_${acc.label}_quota_low`,
              provider: `讯飞${suffix}`,
              type: "quota_low",
              message: `讯飞编程套餐${suffix} 月度余量不足`,
              detail: `月度已用 ${((1 - ratio) * 100).toFixed(0)}%，建议切换至其他模型`,
            });
          }
          // Expiration check
          if (acc.data.expires_at) {
            const dateStr = acc.data.expires_at.includes("T")
              ? acc.data.expires_at
              : acc.data.expires_at.replace(" ", "T");
            if (isWithin24Hours(dateStr)) {
              alerts.push({
                id: `xunfei_${acc.label}_expiring`,
                provider: `讯飞${suffix}`,
                type: "expiring_soon",
                message: `讯飞编程套餐${suffix} 即将到期`,
                detail: `到期日: ${acc.data.expires_at}`,
              });
            }
          }
        }
      }
    }

    // Ainaiba quota low
    if (ainaibaCredit?.available && ainaibaCredit.data) {
      const abRatio = ainaibaCredit.data.balance / Math.max(ainaibaCredit.data.credit_total, 1);
      if (abRatio <= 0.2) {
        alerts.push({
          id: "ainaiba_quota_low",
          provider: "Ainaiba",
          type: "quota_low",
          message: "Ainaiba 额度余量不足",
          detail: `剩余 ${(abRatio * 100).toFixed(0)}%，建议切换至其他模型`,
        });
      }
    }

    // Ainaiba expiration
    if (ainaibaCredit?.available && ainaibaCredit.data?.expires_at) {
      if (isWithin24Hours(ainaibaCredit.data.expires_at)) {
        alerts.push({
          id: "ainaiba_expiring",
          provider: "Ainaiba",
          type: "expiring_soon",
          message: "Ainaiba 额度即将到期",
          detail: `到期日: ${ainaibaCredit.data.expires_at.slice(0, 10)}`,
        });
      }
    }

    // Filter out dismissed alerts
    const newAlerts = alerts.filter((a) => !dismissedAlerts.has(a.id));
    setAlertItems(newAlerts);
    if (newAlerts.length > 0) {
      setShowAlertModal(true);
    }
  }, [quota, xunfei, ainaibaCredit, subscriptionSettings, dismissedAlerts]);

  // Fetch hourly stats whenever main filters change
  useEffect(() => {
    const loadHourly = async () => {
      if (!effectiveRange.from || !effectiveRange.to) return;
      if (hasEmptyRequiredSelection) {
        setHourlyStats(null);
        return;
      }
      try {
        const s = await fetchStats(
          effectiveRange.from,
          effectiveRange.to,
          sourceFilter,
          vendorFilter,
          tzOffset,
          "1h",
          modelFilter
        );
        setHourlyStats(s);
      } catch {
        /* ignore */
      }
    };
    loadHourly();
  }, [
    effectiveRange.from,
    effectiveRange.to,
    sourceFilter,
    vendorFilter,
    tzOffset,
    hasEmptyRequiredSelection,
    modelFilter,
  ]);

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

  // Close chart filter panel on outside click
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

  // Close model filter dropdown on outside click
  useEffect(() => {
    if (!showModelFilter) return;
    const handler = (e: MouseEvent) => {
      const target = e.target as HTMLElement;
      if (!target.closest(".model-filter-dropdown")) {
        setShowModelFilter(false);
      }
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [showModelFilter]);

  const chartData = useMemo(() => {
    if (!stats?.by_date) return [];
    return stats.by_date.map((d) => {
      // For sub-day resolution, the date field contains "YYYY-MM-DD HH:00"
      // Display as "MM-DD HH:00" for sub-day, or just the date for daily
      let label: string;
      if (d.date.includes(" ")) {
        // Sub-day: "2025-05-17 08:00" -> "05-17 08:00"
        const parts = d.date.split(" ");
        const datePart = parts[0].substring(5); // "05-17"
        const timePart = parts[1].substring(0, 5); // "08:00"
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

  // Build vendor → models tree for pivot table (with model merging, sorting, and free-model filtering)
  const vendorModelTree = useMemo(() => {
    if (!stats?.by_model) return [];
    return buildPivotTree(stats.by_model, sortColumn, sortDirection, hideFreeModels);
  }, [stats, sortColumn, sortDirection, hideFreeModels]);

  // Summary for visible rows in the pivot table
  const pivotSummary = useMemo(() => {
    return computePivotSummary(vendorModelTree);
  }, [vendorModelTree]);

  const showRatioAxis = useMemo(
    () => chartMetrics.has("cacheHitRatio") || chartMetrics.has("cacheHitRatioNoXunfei"),
    [chartMetrics]
  );

  const applyCustom = () => {
    if (customFrom && customTo) {
      setActivePreset("custom");
      setAppliedRange(makeCustomAppliedRange(customFrom, customTo));
      setShowCustomPanel(false);
      setPage(1);
    }
  };

  const applyPreset = (key: Exclude<TimePreset, "custom">) => {
    setActivePreset(key);
    setAppliedRange(makeAppliedRange(key));
    setShowCustomPanel(false);
    setPage(1);
  };

  const quickSetCustom = (key: Exclude<TimePreset, "custom">) => {
    applyPreset(key);
  };

  const presetBtnClass = (key: string) =>
    `px-3 py-1 text-xs font-medium rounded-md transition-colors ${
      activePreset === key
        ? "bg-white text-primary-700 shadow-sm"
        : "text-slate-500 hover:text-slate-700"
    }`;

  const customQuickClass = (key: string) =>
    `px-2 py-1 text-[11px] font-medium rounded transition-colors ${
      activePreset === key
        ? "bg-primary-100 text-primary-700"
        : "bg-slate-50 text-slate-500 hover:bg-slate-100"
    }`;

  const handleSort = (column: SortColumn) => {
    if (sortColumn === column) {
      setSortDirection((d) => (d === "asc" ? "desc" : "asc"));
    } else {
      setSortColumn(column);
      setSortDirection("desc");
    }
  };

  const sortIndicator = (column: SortColumn) => {
    if (sortColumn !== column) return null;
    return sortDirection === "desc" ? " ▼" : " ▲";
  };

  return (
    <div className="min-h-screen bg-slate-50">
      {/* Header - compact, sticky */}
      <header className="bg-white border-b border-slate-200 sticky top-0 z-20">
        <div className="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8 py-2">
          {/* Top bar: branding + time + actions */}
          <div className="flex flex-wrap items-center justify-between gap-y-2">
            <div className="flex flex-wrap items-center gap-x-3 gap-y-1.5">
              {/* Logo + Title */}
              <div className="flex items-center gap-2 shrink-0">
                <div className="bg-primary-600 p-1.5 rounded-lg">
                  <Activity className="w-4 h-4 text-white" />
                </div>
                <h1 className="text-sm font-bold text-slate-800 leading-tight">
                  {ZH.title}
                </h1>
              </div>

              {/* Time presets - segmented control */}
              <div className="flex items-center bg-slate-100/80 rounded-lg p-0.5 gap-0.5">
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
                <button
                  onClick={() => applyPreset("all")}
                  className={presetBtnClass("all")}
                >
                  {ZH.allTime}
                </button>
                <div className="relative">
                  <button
                    ref={customBtnRef}
                    onClick={() => setShowCustomPanel((v) => !v)}
                    className={`custom-time-btn inline-flex items-center gap-1 px-3 py-1 text-xs font-medium rounded-md transition-colors ${
                      ["6h", "12h", "1d", "3d", "14d", "30d", "all", "custom"].includes(activePreset)
                        ? "bg-white text-primary-700 shadow-sm"
                        : "text-slate-500 hover:text-slate-700"
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
            </div>

            <div className="flex items-center gap-1.5">
              <button
                onClick={() => setShowPricing(true)}
                className="inline-flex items-center gap-1 px-2 py-1 text-[11px] font-medium rounded text-slate-500 hover:bg-slate-100 transition-colors"
                title="查看计价逻辑"
              >
                <Receipt className="w-3 h-3" />
                计价
              </button>

              <button
                onClick={() => setShowSubscriptionSettings(!showSubscriptionSettings)}
                className={`inline-flex items-center gap-1 px-2 py-1 text-[11px] font-medium rounded transition-colors ${showSubscriptionSettings ? "bg-blue-100 text-blue-700" : "text-slate-500 hover:bg-slate-100"}`}
                title="订阅设置"
              >
                <Calendar className="w-3 h-3" />
                订阅
              </button>

              <button
                onClick={async () => {
                  try {
                    const res = await exportBackup();
                    const blob = await res.blob();
                    const url = URL.createObjectURL(blob);
                    const a = document.createElement("a");
                    a.href = url;
                    a.download = `token-stats-export-${new Date().toISOString().slice(0, 10)}.jsonl`;
                    a.click();
                    URL.revokeObjectURL(url);
                  } catch {
                    /* silent */
                  }
                }}
                className="inline-flex items-center gap-1 px-2 py-1 text-[11px] font-medium rounded text-slate-500 hover:bg-slate-100 transition-colors"
                title="导出备份"
              >
                <Download className="w-3 h-3" />
                备份
              </button>

              <button
                onClick={() => {
                  setShowRestore((v) => !v);
                  setRestoreResult(null);
                  setRestoreError(null);
                }}
                className={`inline-flex items-center gap-1 px-2 py-1 text-[11px] font-medium rounded transition-colors ${
                  showRestore ? "bg-amber-100 text-amber-700" : "text-slate-500 hover:bg-slate-100"
                }`}
                title="导入备份"
              >
                <Upload className="w-3 h-3" />
                恢复
              </button>

              <span className="text-[11px] text-slate-400 shrink-0">
                {loading
                  ? ZH.updating
                  : `${ZH.updatedAt}: ${lastUpdatedAt ? formatTime(lastUpdatedAt.toISOString()) : "-"}`}
              </span>
            </div>
          </div>

          {/* Subscription Settings Panel */}
          {showSubscriptionSettings && (
            <div className="mt-2 p-3 bg-blue-50 border border-blue-200 rounded-lg">
              <div className="flex items-center justify-between mb-2">
                <span className="text-xs font-semibold text-slate-700">订阅设置</span>
                <button onClick={() => setShowSubscriptionSettings(false)} className="p-0.5 rounded hover:bg-blue-100 text-slate-400">
                  <X className="w-3 h-3" />
                </button>
              </div>
              <div className="flex items-center gap-2 text-xs">
                <label className="text-slate-600">Kimi 月起始日:</label>
                <input
                  type="number"
                  min={1}
                  max={28}
                  value={subscriptionSettings?.kimi_monthly_start_day ?? ""}
                  onChange={(e) => {
                    const v = e.target.value ? parseInt(e.target.value) : null;
                    setSubscriptionSettings(prev => ({ ...prev!, kimi_monthly_start_day: v }));
                  }}
                  className="w-16 px-2 py-1 border border-slate-200 rounded text-xs focus:ring-1 focus:ring-primary-500 outline-none"
                  placeholder="1-28"
                />
                <span className="text-[10px] text-slate-400">每月几号开始计费（1-28）</span>
                <button
                  onClick={async () => {
                    try {
                      await saveSubscriptionSettings(subscriptionSettings!);
                    } catch { /* ignore */ }
                  }}
                  className="px-2 py-1 text-[10px] font-medium rounded bg-primary-600 text-white hover:bg-primary-700 transition-colors"
                >
                  保存
                </button>
              </div>
            </div>
          )}

          {/* Filter bar: sources + vendors */}
          <div className="mt-2 flex flex-wrap items-center gap-x-5 gap-y-1.5">
            {/* Source filter tags */}
            <div className="flex items-center gap-1.5 flex-wrap">
              <span className="text-[10px] font-semibold text-slate-400 uppercase tracking-wider">工具</span>
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

            <div className="hidden sm:block w-px h-4 bg-slate-200" />

            {/* Vendor filter tags */}
            <div className="flex items-center gap-1.5 flex-wrap">
              <span className="text-[10px] font-semibold text-slate-400 uppercase tracking-wider">供应商</span>
              {filters.vendors.map((v) => (
                <button
                  key={v}
                  onClick={() => {
                    toggleInSet(selectedVendors, setSelectedVendors, v);
                    setPage(1);
                  }}
                  className={`inline-flex items-center gap-1 px-2 py-0.5 text-[11px] font-medium rounded-full transition-all border ${
                    selectedVendors.has(v)
                      ? "text-white border-transparent shadow-sm"
                      : "bg-white text-slate-500 border-slate-200 hover:border-slate-300"
                  }`}
                  style={
                    selectedVendors.has(v)
                      ? { background: getVendorColor(v) }
                      : undefined
                  }
                >
                  <span
                    className="w-1.5 h-1.5 rounded-full"
                    style={{
                      background: selectedVendors.has(v) ? "white" : getVendorColor(v),
                    }}
                  />
                  {v}
                </button>
              ))}
              {/* Subscription toggle: kimi, xunfei, opencode-go */}
              {(() => {
                const subVendors = filters.vendors.filter((v) =>
                  ["kimi", "xunfei", "opencode-go", "opencode"].includes(v)
                );
                if (subVendors.length === 0) return null;
                const allSelected = subVendors.every((v) => selectedVendors.has(v));
                return (
                  <button
                    onClick={() => {
                      const next = new Set(selectedVendors);
                      for (const v of subVendors) {
                        if (allSelected) next.delete(v);
                        else next.add(v);
                      }
                      setSelectedVendors(next);
                      setPage(1);
                    }}
                    className={`inline-flex items-center gap-1 px-2 py-0.5 text-[11px] font-medium rounded-full transition-all border ${
                      allSelected
                        ? "bg-amber-500 text-white border-transparent shadow-sm"
                        : "bg-white text-amber-600 border-amber-300 hover:border-amber-400"
                    }`}
                  >
                    <span className="w-1.5 h-1.5 rounded-full bg-current" />
                    {ZH.subscription}
                  </button>
                );
              })()}
            </div>
          </div>

          {/* Restore panel */}
          {showRestore && (
            <div className="mt-2 p-3 bg-amber-50 border border-amber-200 rounded-lg">
              <div className="flex items-center gap-2 mb-2">
                <Upload className="w-3.5 h-3.5 text-amber-600" />
                <span className="text-xs font-semibold text-amber-800">恢复备份数据</span>
              </div>
              <div className="flex items-center gap-2">
                <input
                  type="text"
                  value={restorePath}
                  onChange={(e) => setRestorePath(e.target.value)}
                  placeholder="备份目录路径，如 /home/skyscribe/srcs/token-stats/backups/20260519_054107"
                  className="flex-1 px-2 py-1 text-xs border border-amber-300 rounded focus:ring-1 focus:ring-amber-500 outline-none bg-white"
                />
                <button
                  onClick={async () => {
                    if (!restorePath.trim()) return;
                    setRestoreLoading(true);
                    setRestoreError(null);
                    setRestoreResult(null);
                    try {
                      const result = await restoreBackup(restorePath.trim());
                      setRestoreResult(result);
                    } catch (e: unknown) {
                      setRestoreError(e instanceof Error ? e.message : "恢复失败");
                    } finally {
                      setRestoreLoading(false);
                    }
                  }}
                  disabled={restoreLoading || !restorePath.trim()}
                  className="px-3 py-1 text-xs font-medium rounded bg-amber-600 text-white hover:bg-amber-700 disabled:opacity-50 transition-colors"
                >
                  {restoreLoading ? "恢复中..." : "执行恢复"}
                </button>
              </div>
              {restoreResult && (
                <div className="mt-2 text-xs text-emerald-700 bg-emerald-50 border border-emerald-200 rounded px-2 py-1">
                  已恢复 {restoreResult.added} 条记录（跳过 {restoreResult.skipped} 条重复），
                  总数 {restoreResult.before_count} → {restoreResult.after_count}
                  {restoreResult.errors.length > 0 && (
                    <span className="text-rose-600 ml-1">({restoreResult.errors.length} 错误)</span>
                  )}
                </div>
              )}
              {restoreError && (
                <div className="mt-2 text-xs text-rose-700">{restoreError}</div>
              )}
            </div>
          )}

          {/* Pricing logic modal */}
          {showPricing && (
            <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-4">
              <div className="bg-white rounded-xl border border-slate-200 shadow-xl max-w-lg w-full max-h-[80vh] overflow-y-auto p-5">
                <div className="flex items-center justify-between mb-4">
                  <h2 className="text-sm font-bold text-slate-800">计价逻辑</h2>
                  <button
                    onClick={() => setShowPricing(false)}
                    className="p-1 rounded hover:bg-slate-100 text-slate-400"
                  >
                    <X className="w-4 h-4" />
                  </button>
                </div>
                {pricingConfig ? (
                  <div className="space-y-4 text-xs text-slate-600">
                    <div className="bg-slate-50 rounded-lg p-3 border border-slate-100">
                      <p className="font-semibold text-slate-700 mb-1">汇率</p>
                      <p>1 USD = {pricingConfig.usd_to_cny} CNY（汇率日期: {pricingConfig.rate_date}）</p>
                    </div>

                    <div className="bg-slate-50 rounded-lg p-3 border border-slate-100">
                      <p className="font-semibold text-slate-700 mb-1">特殊计费规则</p>
                      <ul className="list-disc list-inside space-y-0.5">
                        <li>讯飞 (xunfei): 按调用次数计费，每次 ¥{pricingConfig.special.xunfei_per_call.toFixed(6)}（199元 / 90000次）</li>
                        <li>Kimi CLI: 按 Token 估算，每 Token ¥{pricingConfig.special.kimi_per_token.toExponential(3)}（199元 / 28亿 Token）</li>
                        <li>OpenCode: 原始 cost ÷ {pricingConfig.special.opencode_divisor} 后再按汇率换算（10美金可用60美金额度）</li>
                        <li>pi / ccswitch: 原始 USD cost 直接按汇率换算为 CNY</li>
                        <li>codex / claude-code: 无原始 cost，按下方模型价格表计算后换算</li>
                      </ul>
                    </div>

                    <div className="bg-slate-50 rounded-lg p-3 border border-slate-100">
                      <p className="font-semibold text-slate-700 mb-1">模型价格表（USD / 1M tokens）</p>
                      <table className="w-full text-left">
                        <thead>
                          <tr className="text-slate-400 border-b border-slate-200">
                            <th className="pb-1 font-medium">模型</th>
                            <th className="pb-1 font-medium">Input</th>
                            <th className="pb-1 font-medium">Output</th>
                            <th className="pb-1 font-medium">Cache</th>
                          </tr>
                        </thead>
                        <tbody>
                          {pricingConfig.model.map((m) => (
                            <tr key={m.name} className="border-b border-slate-100 last:border-0">
                              <td className="py-1 font-medium text-slate-700">{m.name}</td>
                              <td className="py-1">${m.input}</td>
                              <td className="py-1">${m.output}</td>
                              <td className="py-1">${m.cache_read}</td>
                            </tr>
                          ))}
                        </tbody>
                      </table>
                    </div>

                    <p className="text-[10px] text-slate-400">
                      修改 backend/pricing.toml 后运行 ./scripts/reload-pricing.sh 即可生效，无需重启服务。
                    </p>
                  </div>
                ) : (
                  <p className="text-xs text-slate-400">加载中...</p>
                )}
              </div>
            </div>
          )}
        </div>
      </header>

      {/* Subscription Alert Modal */}
      {showAlertModal && alertItems.length > 0 && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-4">
          <div className="bg-white rounded-xl border border-slate-200 shadow-xl max-w-lg w-full max-h-[80vh] overflow-y-auto p-5">
            <div className="flex items-center justify-between mb-4">
              <h2 className="text-sm font-bold text-slate-800">⚠️ 订阅提醒</h2>
              <button onClick={() => {
                setShowAlertModal(false);
                setDismissedAlerts(prev => {
                  const next = new Set(prev);
                  alertItems.forEach(a => next.add(a.id));
                  return next;
                });
              }} className="p-0.5 rounded hover:bg-slate-100 text-slate-400">
                <X className="w-4 h-4" />
              </button>
            </div>
            <div className="space-y-3">
              {alertItems.map(item => (
                <div key={item.id} className={"rounded-lg border p-3 " + (item.type === "quota_low" ? "border-amber-200 bg-amber-50" : "border-rose-200 bg-rose-50")}>
                  <div className="flex items-center gap-1.5 mb-1">
                    <span className={"text-[11px] font-semibold " + (item.type === "quota_low" ? "text-amber-700" : "text-rose-700")}>
                      {item.provider}
                    </span>
                    <span className={"px-1.5 py-0 rounded-full text-[9px] font-medium " + (item.type === "quota_low" ? "bg-amber-100 text-amber-700" : "bg-rose-100 text-rose-700")}>
                      {item.type === "quota_low" ? "余量不足" : "即将到期"}
                    </span>
                  </div>
                  <p className="text-[11px] text-slate-700">{item.message}</p>
                  <p className="text-[10px] text-slate-500 mt-0.5">{item.detail}</p>
                </div>
              ))}
            </div>
            <div className="mt-4 flex justify-end gap-2">
              <button onClick={() => {
                setShowAlertModal(false);
                setDismissedAlerts(prev => {
                  const next = new Set(prev);
                  alertItems.forEach(a => next.add(a.id));
                  return next;
                });
              }} className="px-3 py-1.5 text-xs font-medium rounded-md bg-slate-100 text-slate-600 hover:bg-slate-200 transition-colors">
                知道了
              </button>
            </div>
          </div>
        </div>
      )}

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
                  <p className="text-lg font-bold text-slate-800 leading-tight">{formatCalls(stats.overall.total_calls)}</p>
                </div>
                <div className="px-4 min-w-0">
                  <p className="text-[11px] text-slate-400 font-medium">{ZH.totalTokens}</p>
                  <p className="text-lg font-bold text-sky-600 leading-tight">{formatNumber(stats.overall.total_tokens)}</p>
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
                      <span className="text-slate-400">{formatCalls(s.calls)}次</span>
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
                  {quotaLoading || xunfeiLoading || ainaibaCreditLoading
                    ? "加载中..."
                    : [
                          ...(xunfei?.accounts?.map((acc) =>
                          acc.available && acc.data
                            ? `讯飞${acc.label === "ex" ? " EX" : ""}: ` + (acc.data.usage.rp5h_limit > 0 ? "5h " + (acc.data.usage.rp5h_used / Math.max(acc.data.usage.rp5h_limit, 1) * 100).toFixed(0) + "%, " : "") + (acc.data.usage.rpw_limit > 0 ? "周 " + (acc.data.usage.rpw_used / Math.max(acc.data.usage.rpw_limit, 1) * 100).toFixed(0) + "%, " : "") + "月 " + (acc.data.usage.package_used / Math.max(acc.data.usage.package_limit, 1) * 100).toFixed(0) + "%"
                            : null
                        ) ?? []),
                        xunfei && !xunfeiLoading && (!xunfei.accounts || xunfei.accounts.every(a => !a.available))
                          ? "讯飞: 获取失败"
                          : null,
                        quota?.kimi?.available && quota.kimi.data
                          ? "Kimi: " + (quota.kimi.data.rp5h_limit > 0 ? "5h " + (quota.kimi.data.rp5h_used / Math.max(quota.kimi.data.rp5h_limit, 1) * 100).toFixed(0) + "%" + (formatResetTime(quota.kimi.data.rp5h_reset_time) ? "(" + formatResetTime(quota.kimi.data.rp5h_reset_time)!.replace("后重置", "") + "), " : ", ") : "") + "周 " + (quota.kimi.data.weekly_used / Math.max(quota.kimi.data.weekly_limit, 1) * 100).toFixed(0) + "%" + (formatResetTime(quota.kimi.data.weekly_reset_time) ? "(" + formatResetTime(quota.kimi.data.weekly_reset_time)!.replace("后重置", "") + ")" : "")
                          : quota?.kimi && !quotaLoading ? "Kimi: 获取失败" : null,
                        quota?.opencode_go?.available && quota.opencode_go.data
                          ? "OpenCode: " + (quota.opencode_go.data.entries.find(e => e.usage_type === "Monthly")?.percentage?.toFixed(0) ?? "?") + "%月已用"
                          : quota?.opencode_go?.error
                            ? "OpenCode: " + quota.opencode_go.error
                            : quota?.opencode_go && !quotaLoading ? "OpenCode: 获取失败" : null,
                        quota?.opencode_go_ex?.available && quota.opencode_go_ex.data
                          ? "OpenCode EX: " + (quota.opencode_go_ex.data.entries.find(e => e.usage_type === "Monthly")?.percentage?.toFixed(0) ?? "?") + "%月已用"
                          : quota?.opencode_go_ex?.error
                            ? "OpenCode EX: " + quota.opencode_go_ex.error
                            : quota?.opencode_go_ex && !quotaLoading ? "OpenCode EX: 获取失败" : null,
                        ainaibaCredit?.available && ainaibaCredit.data
                          ? `Ainaiba: 剩余 ${(ainaibaCredit.data.balance / Math.max(ainaibaCredit.data.credit_total, 1) * 100).toFixed(0)}%`
                          : ainaibaCredit?.error
                            ? "Ainaiba: " + ainaibaCredit.error
                            : ainaibaCredit && !ainaibaCreditLoading ? "Ainaiba: 获取失败" : null,
                      ]
                        .filter(Boolean)
                        .join(" · ") || "无可用订阅"}
                </span>
              </summary>
              <div className="mt-1.5 grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-2">
                {/* Xunfei accounts */}
                {xunfei?.accounts?.map((account) => (
                  <div key={account.label} className={"bg-white rounded-xl border " + (account.available && account.data?.status === "active" ? "border-emerald-200" : "border-slate-200") + " p-3 shadow-sm"}>
                    <div className="flex items-center justify-between mb-1.5">
                      <div className="flex items-center gap-1.5">
                        <span className={"w-1.5 h-1.5 rounded-full " + (xunfeiLoading ? "bg-amber-400" : account.available && account.data?.status === "active" ? "bg-emerald-500" : "bg-slate-300")} />
                        <span className="text-[11px] font-semibold text-slate-700">讯飞编程套餐{account.label === "ex" ? " (EX)" : ""}</span>
                      </div>
                      <span className="text-[10px] text-slate-400">xfyun.cn</span>
                    </div>
                    {xunfeiLoading ? (
                      <div className="h-8 flex items-center justify-center">
                        <div className="animate-spin rounded-full h-3 w-3 border-b-2 border-primary-600" />
                      </div>
                    ) : account.available && account.data ? (
                      <>
                        <div className="flex flex-wrap items-center gap-x-2 gap-y-0.5 text-[11px] mb-1.5">
                          <span className="font-bold text-slate-800">{account.data.plan_name}</span>
                          <span className={"px-1 py-0 rounded-full text-[10px] font-medium " + (account.data.status === "active" ? "bg-emerald-100 text-emerald-700" : "bg-slate-100 text-slate-600")}>
                            {account.data.status === "active" ? "有效" : account.data.status}
                          </span>
                          <span className="text-slate-400">¥{(account.data.price / 100).toFixed(2)}/月</span>
                        </div>
                        <div className="space-y-1">
                          <div>
                            <div className="flex justify-between text-[10px] text-slate-500">
                              <span>月度</span>
                              <span>{formatNumber(account.data.usage.package_used)}/{formatNumber(account.data.usage.package_limit)} ({(account.data.usage.package_used / Math.max(account.data.usage.package_limit, 1) * 100).toFixed(0)}%)</span>
                            </div>
                            <div className="w-full h-1 bg-slate-100 rounded-full overflow-hidden">
                              <div className={"h-full rounded-full transition-all " + barColor(account.data.usage.package_used, account.data.usage.package_limit)}
                                style={{ width: (Math.min(account.data.usage.package_used / Math.max(account.data.usage.package_limit, 1) * 100, 100)) + "%" }}
                              />
                            </div>
                          </div>
                          {account.data.usage.rpw_limit > 0 && (
                            <div>
                              <div className="flex justify-between text-[10px] text-slate-500">
                                <span>周限额</span>
                                <span>{formatNumber(account.data.usage.rpw_used)}/{formatNumber(account.data.usage.rpw_limit)} ({(account.data.usage.rpw_used / Math.max(account.data.usage.rpw_limit, 1) * 100).toFixed(0)}%)</span>
                              </div>
                              <div className="w-full h-1 bg-slate-100 rounded-full overflow-hidden">
                                <div className={"h-full rounded-full transition-all " + barColor(account.data.usage.rpw_used, account.data.usage.rpw_limit)}
                                  style={{ width: (Math.min(account.data.usage.rpw_used / Math.max(account.data.usage.rpw_limit, 1) * 100, 100)) + "%" }}
                                />
                              </div>
                            </div>
                          )}
                          {account.data.usage.rp5h_limit > 0 && (
                            <div>
                              <div className="flex justify-between text-[10px] text-slate-500">
                                <span>5小时</span>
                                <span>{formatNumber(account.data.usage.rp5h_used)}/{formatNumber(account.data.usage.rp5h_limit)} ({(account.data.usage.rp5h_used / Math.max(account.data.usage.rp5h_limit, 1) * 100).toFixed(0)}%)</span>
                              </div>
                              <div className="w-full h-1 bg-slate-100 rounded-full overflow-hidden">
                                <div className={"h-full rounded-full transition-all " + barColor(account.data.usage.rp5h_used, account.data.usage.rp5h_limit)}
                                  style={{ width: (Math.min(account.data.usage.rp5h_used / Math.max(account.data.usage.rp5h_limit, 1) * 100, 100)) + "%" }}
                                />
                              </div>
                            </div>
                          )}
                        </div>
                        <div className="flex flex-wrap items-center gap-x-2 gap-y-0.5 mt-1 pt-1 border-t border-slate-100 text-[10px] text-slate-500">
                          <span>余额 ¥{(account.data.balance.cash / 100).toFixed(2)}</span>
                          {account.data.balance.virtual_balance > 0 && <span>赠送 ¥{(account.data.balance.virtual_balance / 100).toFixed(2)}</span>}
                          <span>到期 {account.data.expires_at.replace(" ", "T")}</span>
                        </div>
                      </>
                    ) : (
                      <p className="text-[11px] text-slate-400 italic">{account.error || "获取失败"}</p>
                    )}
                  </div>
                ))}

                {/* Ainaiba */}
                <div className={"bg-white rounded-xl border " + (ainaibaCredit?.available ? "border-emerald-200" : "border-slate-200") + " p-3 shadow-sm"}>
                  <div className="flex items-center justify-between mb-1.5">
                    <div className="flex items-center gap-1.5">
                      <span className={"w-1.5 h-1.5 rounded-full " + (ainaibaCreditLoading ? "bg-amber-400" : ainaibaCredit?.available ? "bg-emerald-500" : "bg-slate-300")} />
                      <span className="text-[11px] font-semibold text-slate-700">Ainaiba</span>
                    </div>
                    <span className="text-[10px] text-slate-400">xai.ainaibahub</span>
                  </div>
                  {ainaibaCreditLoading ? (
                    <div className="h-8 flex items-center justify-center">
                      <div className="animate-spin rounded-full h-3 w-3 border-b-2 border-primary-600" />
                    </div>
                  ) : ainaibaCredit?.available && ainaibaCredit.data ? (
                    <>
                      <div className="flex items-center justify-between text-[11px] mb-1">
                        <span className="font-medium text-slate-600">{ainaibaCredit.data.alias || ainaibaCredit.data.name}</span>
                        <span className="text-slate-400">#{ainaibaCredit.data.user_id}</span>
                      </div>

                      {/* Total requests */}
                      <div className="flex items-center gap-3 mb-1.5">
                        <div className="text-[10px] text-slate-500">
                          <span className="text-slate-700 font-medium">{formatCalls(ainaibaCredit.data.total_requests)}</span> 总请求
                        </div>
                        <div className="text-[10px] text-slate-500">
                          <span className="text-slate-700 font-medium">{ainaibaCredit.data.credit_used.toFixed(2)}</span> / {ainaibaCredit.data.credit_total.toFixed(2)} 额度
                        </div>
                      </div>

                      {/* Total credit bar */}
                      <div className="space-y-1 mb-1.5">
                        <div className="flex justify-between text-[10px] text-slate-500">
                          <span>总额度已用 {((ainaibaCredit.data.credit_used / Math.max(ainaibaCredit.data.credit_total, 1)) * 100).toFixed(1)}%</span>
                          <span>剩余 {ainaibaCredit.data.balance.toFixed(2)}</span>
                        </div>
                        <div className="w-full h-1 bg-slate-100 rounded-full overflow-hidden">
                          <div className={"h-full rounded-full transition-all " + barColor(ainaibaCredit.data.credit_used, ainaibaCredit.data.credit_total)}
                            style={{ width: (Math.min((ainaibaCredit.data.credit_used / Math.max(ainaibaCredit.data.credit_total, 1)) * 100, 100)) + "%" }}
                          />
                        </div>
                      </div>

                      {/* Daily limit bar */}
                      <div className="space-y-1 mb-1.5">
                        <div className="flex justify-between text-[10px] text-slate-500">
                          <span>日限已用 {((ainaibaCredit.data.daily_used / Math.max(ainaibaCredit.data.daily_limit, 1)) * 100).toFixed(1)}%</span>
                          <span>{ainaibaCredit.data.daily_used.toFixed(2)} / {ainaibaCredit.data.daily_limit.toLocaleString()}</span>
                        </div>
                        <div className="w-full h-1 bg-slate-100 rounded-full overflow-hidden">
                          <div className={"h-full rounded-full transition-all " + barColor(ainaibaCredit.data.daily_used, ainaibaCredit.data.daily_limit)}
                            style={{ width: (Math.min((ainaibaCredit.data.daily_used / Math.max(ainaibaCredit.data.daily_limit, 1)) * 100, 100)) + "%" }}
                          />
                        </div>
                      </div>

                      {/* Expandable details */}
                      <details className="group">
                        <summary className="cursor-pointer select-none text-[10px] text-slate-500 hover:text-slate-700 transition-colors flex items-center gap-1 mb-1 list-none">
                          <svg className="w-3 h-3 text-slate-400 transition-transform group-open:rotate-90" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 5l7 7-7 7" /></svg>
                          详细用量
                        </summary>
                        {/* Today stats */}
                        <div className="mb-2 mt-1">
                          <div className="text-[10px] font-semibold text-slate-600 mb-0.5">今日</div>
                          <div className="grid grid-cols-3 gap-x-2 gap-y-0.5 text-[10px]">
                            <div className="text-slate-500">请求 <span className="text-slate-700">{formatCalls(ainaibaCredit.data.daily_requests)}</span></div>
                            <div className="text-slate-500">输入 <span className="text-slate-700">{formatNumber(ainaibaCredit.data.daily_input_tokens)}</span></div>
                            <div className="text-slate-500">输出 <span className="text-slate-700">{formatNumber(ainaibaCredit.data.daily_output_tokens)}</span></div>
                            <div className="text-slate-500">推理 <span className="text-slate-700">{formatNumber(ainaibaCredit.data.daily_reasoning_tokens)}</span></div>
                            <div className="text-slate-500">缓存 <span className="text-slate-700">{formatNumber(ainaibaCredit.data.daily_cached_tokens)}</span></div>
                            <div className="text-slate-500">消耗 <span className="text-slate-700">{ainaibaCredit.data.daily_used.toFixed(2)}</span></div>
                          </div>
                        </div>

                        {/* Monthly stats */}
                        <div className="mb-1">
                          <div className="text-[10px] font-semibold text-slate-600 mb-0.5">本月</div>
                          <div className="grid grid-cols-3 gap-x-2 gap-y-0.5 text-[10px]">
                            <div className="text-slate-500">请求 <span className="text-slate-700">{formatCalls(ainaibaCredit.data.monthly_requests)}</span></div>
                            <div className="text-slate-500">输入 <span className="text-slate-700">{formatNumber(ainaibaCredit.data.monthly_input_tokens)}</span></div>
                            <div className="text-slate-500">输出 <span className="text-slate-700">{formatNumber(ainaibaCredit.data.monthly_output_tokens)}</span></div>
                            <div className="text-slate-500">推理 <span className="text-slate-700">{formatNumber(ainaibaCredit.data.monthly_reasoning_tokens)}</span></div>
                            <div className="text-slate-500">缓存 <span className="text-slate-700">{formatNumber(ainaibaCredit.data.monthly_cached_tokens)}</span></div>
                            <div className="text-slate-500">消耗 <span className="text-slate-700">{ainaibaCredit.data.monthly_used.toFixed(2)}</span></div>
                          </div>
                        </div>
                      </details>

                      {/* Limits & expiry */}
                      <div className="pt-1.5 border-t border-slate-100 flex flex-wrap items-center gap-x-2 gap-y-0.5 text-[10px] text-slate-500">
                        <span>到期 {ainaibaCredit.data.expires_at ? ainaibaCredit.data.expires_at.slice(0, 10) : "-"}</span>
                        <span>硬限 {ainaibaCredit.data.hard_limit.toLocaleString()}</span>
                        {ainaibaCredit.data.rpm > 0 && (
                          <span>限流 {ainaibaCredit.data.rpm}/{ainaibaCredit.data.rph}/{ainaibaCredit.data.rpd}</span>
                        )}
                      </div>
                    </>
                  ) : (
                    <>
                      {ainaibaCredit?.error ? (
                        <p className="text-[11px] text-slate-400 italic">{ainaibaCredit.error}</p>
                      ) : (
                        <p className="text-[11px] text-slate-400 italic">获取失败</p>
                      )}
                    </>
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
                          {formatResetTime(quota.kimi.data.weekly_reset_time) && (
                            <span className="text-[10px] text-slate-400">{formatResetTime(quota.kimi.data.weekly_reset_time)}</span>
                          )}
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
                            {formatResetTime(quota.kimi.data.rp5h_reset_time) && (
                              <span className="text-[10px] text-slate-400">{formatResetTime(quota.kimi.data.rp5h_reset_time)}</span>
                            )}
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
                  ) : quota?.opencode_go?.available && quota.opencode_go.data && quota.opencode_go.data.entries.length > 0 ? (
                    <div className="space-y-1.5">
                      {quota.opencode_go.data.entries.map((entry) => (
                        <div key={entry.usage_type}>
                          <div className="flex justify-between text-[10px] text-slate-500">
                            <span>{entry.usage_type === "Rolling" ? "滚动" : entry.usage_type === "Weekly" ? "周" : entry.usage_type === "Monthly" ? "月" : entry.usage_type}</span>
                            <span>{entry.percentage}% · {entry.resets_in}</span>
                          </div>
                          <div className="w-full h-1 bg-slate-100 rounded-full overflow-hidden">
                            <div className={"h-full rounded-full transition-all " + (entry.percentage > 80 ? "bg-rose-500" : entry.percentage > 50 ? "bg-amber-500" : "bg-emerald-500")}
                              style={{ width: (Math.min(entry.percentage, 100)) + "%" }}
                            />
                          </div>
                        </div>
                      ))}
                    </div>
                  ) : (
                    <>
                      {quota?.opencode_go?.error ? (
                        <p className="text-[11px] text-slate-400 italic">{quota.opencode_go.error}</p>
                      ) : (
                        <p className="text-[11px] text-slate-400 italic">获取失败</p>
                      )}
                    </>
                  )}
                </div>

                {/* OpenCode-go EX */}
                <div className={"bg-white rounded-xl border " + (quota?.opencode_go_ex?.available ? "border-emerald-200" : "border-slate-200") + " p-3 shadow-sm"}>
                  <div className="flex items-center justify-between mb-1.5">
                    <div className="flex items-center gap-1.5">
                      <span className={"w-1.5 h-1.5 rounded-full " + (quotaLoading ? "bg-amber-400" : quota?.opencode_go_ex?.available ? "bg-emerald-500" : "bg-slate-300")} />
                      <span className="text-[11px] font-semibold text-slate-700">OpenCode-go (EX)</span>
                    </div>
                    <span className="text-[10px] text-slate-400">opencode.ai</span>
                  </div>
                  {quotaLoading ? (
                    <div className="h-8 flex items-center justify-center">
                      <div className="animate-spin rounded-full h-3 w-3 border-b-2 border-primary-600" />
                    </div>
                  ) : quota?.opencode_go_ex?.available && quota.opencode_go_ex.data && quota.opencode_go_ex.data.entries.length > 0 ? (
                    <div className="space-y-1.5">
                      {quota.opencode_go_ex.data.entries.map((entry) => (
                        <div key={entry.usage_type}>
                          <div className="flex justify-between text-[10px] text-slate-500">
                            <span>{entry.usage_type === "Rolling" ? "滚动" : entry.usage_type === "Weekly" ? "周" : entry.usage_type === "Monthly" ? "月" : entry.usage_type}</span>
                            <span>{entry.percentage}% · {entry.resets_in}</span>
                          </div>
                          <div className="w-full h-1 bg-slate-100 rounded-full overflow-hidden">
                            <div className={"h-full rounded-full transition-all " + (entry.percentage > 80 ? "bg-rose-500" : entry.percentage > 50 ? "bg-amber-500" : "bg-emerald-500")}
                              style={{ width: (Math.min(entry.percentage, 100)) + "%" }}
                            />
                          </div>
                        </div>
                      ))}
                    </div>
                  ) : (
                    <>
                      {quota?.opencode_go_ex?.error ? (
                        <p className="text-[11px] text-slate-400 italic">{quota.opencode_go_ex.error}</p>
                      ) : (
                        <p className="text-[11px] text-slate-400 italic">获取失败</p>
                      )}
                    </>
                  )}
                </div>
              </div>
            </details>

            {/* Charts Row */}
            <div className="flex flex-col gap-3 mb-3">
              {/* Daily Token Usage - Stacked Bar + Cache Hit Ratio Line */}
              <div className="bg-white rounded-xl border border-slate-200 p-4 shadow-sm">
                <div className="flex items-center justify-between mb-2">
                  <h3 className="text-xs font-semibold text-slate-700">
                    {ZH.dailyTokenUsage}
                  </h3>
                  <div className="relative">
                    <button
                      onClick={() => setShowChartFilter((v) => !v)}
                      className={`chart-metric-btn inline-flex items-center gap-1 px-2 py-1 text-[11px] font-medium rounded-md transition-colors ${
                        showChartFilter
                          ? "bg-primary-100 text-primary-700"
                          : "bg-slate-50 text-slate-500 hover:bg-slate-100"
                      }`}
                    >
                      <SlidersHorizontal className="w-3 h-3" />
                      {ZH.chartMetrics}
                    </button>
                    {showChartFilter && (
                      <div className="chart-metric-panel absolute right-0 top-full mt-1 bg-white border border-slate-200 rounded-lg shadow-xl p-1.5 min-w-[160px] z-30">
                        {CHART_METRIC_OPTIONS.map((opt) => (
                          <label
                            key={opt.key}
                            className="flex items-center gap-2 px-2 py-1.5 rounded hover:bg-slate-50 cursor-pointer"
                          >
                            <input
                              type="checkbox"
                              checked={chartMetrics.has(opt.key)}
                              onChange={() => toggleInSet(chartMetrics, setChartMetrics, opt.key)}
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
                <ResponsiveContainer width="100%" height={260}>
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
                        name={ZH.cacheLabel}
                        stackId="tokens"
                        fill="#c084fc"
                      />
                    )}
                    {chartMetrics.has("input") && (
                      <Bar
                        yAxisId="tokens"
                        dataKey="input"
                        name={ZH.inputLabel}
                        stackId="tokens"
                        fill="#38bdf8"
                      />
                    )}
                    {chartMetrics.has("output") && (
                      <Bar
                        yAxisId="tokens"
                        dataKey="output"
                        name={ZH.outputLabel}
                        stackId="tokens"
                        fill="#fb923c"
                      />
                    )}
                    {chartMetrics.has("cacheHitRatio") && showRatioAxis && (
                      <Line
                        yAxisId="ratio"
                        type="monotone"
                        dataKey="cacheHitRatio"
                        name={ZH.cacheHitLabel}
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
                        name={ZH.cacheHitNoXunfei}
                        stroke="#22d3ee"
                        strokeWidth={1.5}
                        strokeDasharray="4 2"
                        dot={{ r: 1.5 }}
                      />
                    )}
                  </ComposedChart>
                </ResponsiveContainer>
                <p className="text-[10px] text-slate-400 mt-1">* {ZH.xunfeiNoCacheNote}</p>
              </div>

              <div className="grid grid-cols-1 lg:grid-cols-2 gap-3">
                {/* Hourly Requests */}
                <div className="bg-white rounded-xl border border-slate-200 p-4 shadow-sm">
                  <h3 className="text-xs font-semibold text-slate-700 mb-2">
                    {ZH.hourlyRequests}
                  </h3>
                  <ResponsiveContainer width="100%" height={240}>
                    <BarChart
                      data={hourlyStats?.by_date?.map((d) => {
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
                      }) ?? []}
                    >
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
                        name={ZH.calls}
                        fill="#2dd4bf"
                        radius={[4, 4, 0, 0]}
                      />
                    </BarChart>
                  </ResponsiveContainer>
                </div>

                {/* Vendor Breakdown */}
                <div className="bg-white rounded-xl border border-slate-200 p-4 shadow-sm">
                  <h3 className="text-xs font-semibold text-slate-700 mb-2">
                    {ZH.vendorBreakdown}
                  </h3>
                  <ResponsiveContainer width="100%" height={240}>
                    <BarChart data={vendorChartData} layout="vertical">
                      <CartesianGrid strokeDasharray="2 2" stroke="#f1f5f9" />
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
                        {vendorChartData.map((entry, index) => (
                          <Cell key={`cell-${index}`} fill={getVendorColor(entry.name)} />
                        ))}
                      </Bar>
                    </BarChart>
                  </ResponsiveContainer>
                </div>
              </div>
            </div>

            {/* Vendor & Model Performance - pivot table with expand/collapse */}
            <div className="bg-white rounded-xl border border-slate-200 shadow-sm mb-3">
              <div className="px-4 py-2.5 border-b border-slate-100 flex items-center justify-between">
                <h3 className="text-xs font-semibold text-slate-700">
                  {ZH.vendorAndModel}
                </h3>
                <div className="flex items-center gap-2">
                  {/* Free model filter toggle */}
                  <button
                    onClick={() => setHideFreeModels((v) => !v)}
                    className={`inline-flex items-center gap-1 px-2 py-1 text-[11px] rounded-md border transition-colors ${
                      hideFreeModels
                        ? "bg-emerald-50 border-emerald-300 text-emerald-700"
                        : "bg-white border-slate-200 text-slate-500 hover:bg-slate-50"
                    }`}
                    title={ZH.hideFreeModels}
                  >
                    <Receipt className="w-3 h-3" />
                    {ZH.hideFreeModels}
                  </button>
                  {/* Model Filter Dropdown */}
                  <div className="relative model-filter-dropdown">
                    <button
                      onClick={() => {
                        setPendingPivotModels(new Set(selectedPivotModels));
                        setShowModelFilter(!showModelFilter);
                      }}
                      className={`inline-flex items-center gap-1 px-2 py-1 text-[11px] rounded-md border transition-colors ${
                        selectedPivotModels.size > 0
                          ? "bg-blue-50 border-blue-300 text-blue-700"
                          : "bg-white border-slate-200 text-slate-500 hover:bg-slate-50"
                      }`}
                    >
                      <Filter className="w-3 h-3" />
                      {ZH.modelFilter}
                      {selectedPivotModels.size > 0 && (
                        <span className="ml-0.5 px-1 py-0.5 text-[9px] bg-blue-100 text-blue-700 rounded-full">
                          {selectedPivotModels.size}
                        </span>
                      )}
                      <ChevronDown className="w-3 h-3" />
                    </button>
                    {showModelFilter && (
                      <div className="absolute right-0 top-full mt-1 w-56 max-h-80 bg-white border border-slate-200 rounded-lg shadow-lg z-50 flex flex-col">
                        {showAdvancedSettings ? (
                          <>
                            <div className="shrink-0 bg-white border-b border-slate-100 px-2 py-1.5 flex items-center justify-between">
                              <span className="text-xs font-semibold text-slate-700">高级模型设置</span>
                              <button
                                onClick={() => setShowAdvancedSettings(false)}
                                className="p-0.5 rounded hover:bg-slate-100 text-slate-400"
                              >
                                <X className="w-3 h-3" />
                              </button>
                            </div>
                            <div className="flex-1 p-2">
                              <textarea
                                value={advancedModelsDraft}
                                onChange={(e) => setAdvancedModelsDraft(e.target.value)}
                                className="w-full h-40 text-xs border border-slate-200 rounded p-2 focus:ring-1 focus:ring-primary-500 outline-none resize-none"
                                placeholder="每行一个模型名称"
                              />
                            </div>
                            <div className="shrink-0 bg-white border-t border-slate-100 px-2 py-1.5 flex justify-end gap-2">
                              <button
                                onClick={() => setShowAdvancedSettings(false)}
                                className="px-2 py-1 text-[10px] font-medium rounded text-slate-500 hover:bg-slate-100 transition-colors"
                              >
                                {ZH.cancel}
                              </button>
                              <button
                                onClick={async () => {
                                  const models = advancedModelsDraft
                                    .split("\n")
                                    .map((s) => s.trim())
                                    .filter((s) => s.length > 0);
                                  try {
                                    await saveAdvancedModels(models);
                                    setAdvancedModels(models);
                                  } catch {
                                    /* ignore */
                                  }
                                  setShowAdvancedSettings(false);
                                }}
                                className="px-2 py-1 text-[10px] font-medium rounded bg-primary-600 text-white hover:bg-primary-700 transition-colors"
                              >
                                {ZH.apply}
                              </button>
                            </div>
                          </>
                        ) : (
                          <>
                            <div className="shrink-0 bg-white border-b border-slate-100 px-2 py-1.5 flex items-center justify-between">
                              <div className="flex gap-2">
                                <button
                                  onClick={() => setPendingPivotModels(new Set(pivotModelOptions))}
                                  className="text-[10px] text-blue-600 hover:text-blue-800"
                                >
                                  {ZH.selectAll}
                                </button>
                                <span className="text-slate-300">|</span>
                                <button
                                  onClick={() => setPendingPivotModels(new Set())}
                                  className="text-[10px] text-slate-500 hover:text-slate-700"
                                >
                                  {ZH.clearAll}
                                </button>
                              </div>
                              <button
                                onClick={() => {
                                  setAdvancedModelsDraft(advancedModels.join("\n"));
                                  setShowAdvancedSettings(true);
                                }}
                                className="p-0.5 rounded hover:bg-slate-100 text-slate-400"
                                title="高级模型设置"
                              >
                                <Settings className="w-3 h-3" />
                              </button>
                            </div>
                            <div className="flex-1 overflow-y-auto p-1">
                              {pivotModelOptions.map((model) => (
                                <label
                                  key={model}
                                  className="flex items-center gap-2 px-2 py-1 text-xs text-slate-700 hover:bg-slate-50 rounded cursor-pointer"
                                >
                                  <input
                                    type="checkbox"
                                    checked={pendingPivotModels.has(model)}
                                    onChange={() => {
                                      const next = new Set(pendingPivotModels);
                                      if (next.has(model)) next.delete(model);
                                      else next.add(model);
                                      setPendingPivotModels(next);
                                    }}
                                    className="rounded border-slate-300 text-blue-600 focus:ring-blue-500"
                                  />
                                  <span className="truncate" title={model}>{model}</span>
                                </label>
                              ))}
                            </div>
                            <div className="shrink-0 bg-white border-t border-slate-100 px-2 py-1.5 flex justify-between items-center">
                              <button
                                onClick={() => {
                                  const available = new Set(pivotModelOptions);
                                  const next = new Set(
                                    advancedModels.filter((m) => available.has(m))
                                  );
                                  setPendingPivotModels(next);
                                }}
                                className="text-[10px] text-blue-600 hover:text-blue-800"
                              >
                                选择高级模型
                              </button>
                              <div className="flex gap-2">
                                <button
                                  onClick={() => setShowModelFilter(false)}
                                  className="px-2 py-1 text-[10px] font-medium rounded text-slate-500 hover:bg-slate-100 transition-colors"
                                >
                                  {ZH.cancel}
                                </button>
                                <button
                                  onClick={() => {
                                    setSelectedPivotModels(pendingPivotModels);
                                    setShowModelFilter(false);
                                  }}
                                  className="px-2 py-1 text-[10px] font-medium rounded bg-primary-600 text-white hover:bg-primary-700 transition-colors"
                                >
                                  {ZH.apply}
                                </button>
                              </div>
                            </div>
                          </>
                        )}
                      </div>
                    )}
                  </div>
                </div>
              </div>
              <div className="overflow-x-auto">
                <table className="w-full text-xs">
                  <thead>
                    <tr className="bg-slate-50 text-slate-500 text-[10px] uppercase tracking-wider">
                      <th className="px-3 py-2 text-left font-medium cursor-pointer hover:bg-slate-100 transition-colors select-none" onClick={() => handleSort("name")}>{ZH.provider} / {ZH.model} / {ZH.source}{sortIndicator("name")}</th>
                      <th className="px-3 py-2 text-right font-medium cursor-pointer hover:bg-slate-100 transition-colors select-none" onClick={() => handleSort("calls")}>{ZH.calls}{sortIndicator("calls")}</th>
                      <th className="px-3 py-2 text-right font-medium cursor-pointer hover:bg-slate-100 transition-colors select-none" onClick={() => handleSort("input_tokens")}>{ZH.input}{sortIndicator("input_tokens")}</th>
                      <th className="px-3 py-2 text-right font-medium cursor-pointer hover:bg-slate-100 transition-colors select-none" onClick={() => handleSort("output_tokens")}>{ZH.output}{sortIndicator("output_tokens")}</th>
                      <th className="px-3 py-2 text-right font-medium cursor-pointer hover:bg-slate-100 transition-colors select-none" onClick={() => handleSort("cache")}>缓存{sortIndicator("cache")}</th>
                      <th className="px-3 py-2 text-right font-medium cursor-pointer hover:bg-slate-100 transition-colors select-none" onClick={() => handleSort("total_tokens")}>{ZH.total}{sortIndicator("total_tokens")}</th>
                      <th className="px-3 py-2 text-right font-medium cursor-pointer hover:bg-slate-100 transition-colors select-none" onClick={() => handleSort("cache_hit_ratio")}>{ZH.cacheHit}{sortIndicator("cache_hit_ratio")}</th>
                      <th className="px-3 py-2 text-right font-medium cursor-pointer hover:bg-slate-100 transition-colors select-none" onClick={() => handleSort("cost")}>{ZH.cost}{sortIndicator("cost")}</th>
                      <th className="px-3 py-2 text-right font-medium cursor-pointer hover:bg-slate-100 transition-colors select-none" onClick={() => handleSort("avg_cost")}>{ZH.avgCost}{sortIndicator("avg_cost")}</th>
                    </tr>
                  </thead>
                  <tbody className="divide-y divide-slate-100">
                    {vendorModelTree.map(({ provider, models, summary: vendorSummary }) => {
                      const vendorExpanded = expandedVendors.has(provider);
                      const vendorCacheHit = vendorSummary.cache_hit_ratio;

                      return (
                        <Fragment key={provider}>
                          {/* Vendor row */}
                          <tr
                            className="bg-slate-50/80 transition-colors cursor-pointer"
                            onClick={() => toggleInSet(expandedVendors, setExpandedVendors, provider)}
                          >
                            <td className="px-3 py-2">
                              <div className="flex items-center gap-1.5">
                                {vendorExpanded ? (
                                  <ChevronDown className="w-3 h-3 text-slate-400" />
                                ) : (
                                  <ChevronRightIcon className="w-3 h-3 text-slate-400" />
                                )}
                                <span className="w-2 h-2 rounded-full" style={{ background: getSourceColor(provider) }} />
                                <span className="font-bold text-slate-800">{provider}</span>
                              </div>
                            </td>
                            <td className="px-3 py-2 text-right font-semibold text-slate-700">{formatCalls(vendorSummary.calls)}</td>
                            <td className="px-3 py-2 text-right text-slate-600">{formatNumber(vendorSummary.input_tokens)}</td>
                            <td className="px-3 py-2 text-right text-slate-600">{formatNumber(vendorSummary.output_tokens)}</td>
                            <td className="px-3 py-2 text-right text-slate-600">{formatNumber(vendorSummary.cache_read_tokens + vendorSummary.cache_write_tokens)}</td>
                            <td className="px-3 py-2 text-right font-bold text-slate-800">{formatNumber(vendorSummary.total_tokens)}</td>
                            <td className="px-3 py-2 text-right">
                              <span className={`px-1.5 py-0.5 rounded-full text-[10px] font-medium ${
                                vendorCacheHit > 50 ? "bg-emerald-100 text-emerald-700"
                                : vendorCacheHit > 10 ? "bg-amber-100 text-amber-700"
                                : "bg-slate-100 text-slate-600"
                              }`}>{formatPercent(vendorCacheHit)}</span>
                            </td>
                            <td className="px-3 py-2 text-right font-semibold text-slate-700">{formatCost(vendorSummary.cost)}</td>
                            <td className="px-3 py-2 text-right text-slate-600">{formatAvgCost(vendorSummary.cost, vendorSummary.total_tokens)}</td>
                          </tr>

                          {vendorExpanded &&
                            models.map((model) => {
                              const modelKey = `${provider}|${model.model}`;
                              const modelExpanded = expandedModels.has(modelKey);
                              const singleSource = model.source_details.length <= 1;
                              const ms = model.summary;
                              return (
                                <Fragment key={modelKey}>
                                  {/* Model row */}
                                  <tr
                                    className={`hover:bg-slate-50 transition-colors ${singleSource ? "" : "cursor-pointer"}`}
                                    onClick={() => {
                                      if (!singleSource) {
                                        toggleInSet(expandedModels, setExpandedModels, modelKey);
                                      }
                                    }}
                                  >
                                    <td className="px-3 py-2 pl-8">
                                      <div className="flex items-center gap-1.5 flex-wrap">
                                        {!singleSource && (
                                          modelExpanded ? (
                                            <ChevronDown className="w-3 h-3 text-slate-400 shrink-0" />
                                          ) : (
                                            <ChevronRightIcon className="w-3 h-3 text-slate-400 shrink-0" />
                                          )
                                        )}
                                        <span className="font-medium text-slate-700">{model.model}</span>
                                        {model.source_details.map((sd) => (
                                          <span
                                            key={sd.source}
                                            className="inline-flex items-center gap-1 px-1.5 py-0.5 rounded text-[9px] font-medium"
                                            style={{ background: `${getSourceColor(sd.source)}15`, color: getSourceColor(sd.source) }}
                                          >
                                            {getSourceLabel(sd.source)}
                                          </span>
                                        ))}
                                      </div>
                                    </td>
                                    <td className="px-3 py-2 text-right text-slate-600">{formatCalls(ms.calls)}</td>
                                    <td className="px-3 py-2 text-right text-slate-600">{formatNumber(ms.input_tokens)}</td>
                                    <td className="px-3 py-2 text-right text-slate-600">{formatNumber(ms.output_tokens)}</td>
                                    <td className="px-3 py-2 text-right text-slate-600">{formatNumber(ms.cache_read_tokens + ms.cache_write_tokens)}</td>
                                    <td className="px-3 py-2 text-right font-semibold text-slate-700">{formatNumber(ms.total_tokens)}</td>
                                    <td className="px-3 py-2 text-right">
                                      <span className={`px-1.5 py-0.5 rounded-full text-[10px] font-medium ${
                                        ms.cache_hit_ratio > 50 ? "bg-emerald-100 text-emerald-700"
                                        : ms.cache_hit_ratio > 10 ? "bg-amber-100 text-amber-700"
                                        : "bg-slate-100 text-slate-600"
                                      }`}>{formatPercent(ms.cache_hit_ratio)}</span>
                                    </td>
                                    <td className="px-3 py-2 text-right text-slate-600">{formatCost(ms.cost)}</td>
                                    <td className="px-3 py-2 text-right text-slate-600">{formatAvgCost(ms.cost, ms.total_tokens)}</td>
                                  </tr>

                                  {modelExpanded && !singleSource &&
                                    model.source_details.map((source) => (
                                      <tr key={`${modelKey}|${source.source}`} className="hover:bg-slate-50/60 transition-colors">
                                        <td className="px-3 py-2 pl-14">
                                          <span
                                            className="inline-flex items-center gap-1 px-1.5 py-0.5 rounded text-[9px] font-medium"
                                            style={{ background: `${getSourceColor(source.source)}15`, color: getSourceColor(source.source) }}
                                          >
                                            {getSourceLabel(source.source)}
                                          </span>
                                        </td>
                                        <td className="px-3 py-2 text-right text-slate-600">{formatCalls(source.calls)}</td>
                                        <td className="px-3 py-2 text-right text-slate-600">{formatNumber(source.input_tokens)}</td>
                                        <td className="px-3 py-2 text-right text-slate-600">{formatNumber(source.output_tokens)}</td>
                                        <td className="px-3 py-2 text-right text-slate-600">{formatNumber(source.cache_read_tokens + source.cache_write_tokens)}</td>
                                        <td className="px-3 py-2 text-right font-medium text-slate-700">{formatNumber(source.total_tokens)}</td>
                                        <td className="px-3 py-2 text-right">
                                          <span className={`px-1.5 py-0.5 rounded-full text-[10px] font-medium ${
                                            source.cache_hit_ratio > 50 ? "bg-emerald-100 text-emerald-700"
                                            : source.cache_hit_ratio > 10 ? "bg-amber-100 text-amber-700"
                                            : "bg-slate-100 text-slate-600"
                                          }`}>{formatPercent(source.cache_hit_ratio)}</span>
                                        </td>
                                        <td className="px-3 py-2 text-right text-slate-600">{formatCost(source.cost)}</td>
                                        <td className="px-3 py-2 text-right text-slate-600">{formatAvgCost(source.cost, source.total_tokens)}</td>
                                      </tr>
                                    ))}
                                </Fragment>
                              );
                            })}
                        </Fragment>
                      );
                    })}

                    {/* Summary row */}
                    {pivotSummary && (
                      <tr className="bg-slate-100/80 border-t-2 border-slate-200">
                        <td className="px-3 py-2 font-bold text-slate-800">当前视图汇总</td>
                        <td className="px-3 py-2 text-right font-bold text-slate-800">{formatCalls(pivotSummary.calls)}</td>
                        <td className="px-3 py-2 text-right font-bold text-slate-800">{formatNumber(pivotSummary.input_tokens)}</td>
                        <td className="px-3 py-2 text-right font-bold text-slate-800">{formatNumber(pivotSummary.output_tokens)}</td>
                        <td className="px-3 py-2 text-right font-bold text-slate-800">{formatNumber(pivotSummary.cache_read_tokens + pivotSummary.cache_write_tokens)}</td>
                        <td className="px-3 py-2 text-right font-bold text-slate-800">{formatNumber(pivotSummary.total_tokens)}</td>
                        <td className="px-3 py-2 text-right">
                          <span className={`px-1.5 py-0.5 rounded-full text-[10px] font-bold ${
                            pivotSummary.cache_hit_ratio > 50 ? "bg-emerald-100 text-emerald-700"
                            : pivotSummary.cache_hit_ratio > 10 ? "bg-amber-100 text-amber-700"
                            : "bg-slate-100 text-slate-600"
                          }`}>{formatPercent(pivotSummary.cache_hit_ratio)}</span>
                        </td>
                        <td className="px-3 py-2 text-right font-bold text-slate-800">{formatCost(pivotSummary.cost)}</td>
                        <td className="px-3 py-2 text-right font-bold text-slate-800">{formatAvgCost(pivotSummary.cost, pivotSummary.total_tokens)}</td>
                      </tr>
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
                    {filteredModels.map((m) => (
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
                        <td className="px-3 py-2 text-slate-600">{getDisplayModel(r.model)}</td>
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
                        onClick={() => setPage(1)}
                        disabled={requests.page <= 1}
                        className="p-1 rounded hover:bg-slate-100 disabled:opacity-40 disabled:cursor-not-allowed transition-colors"
                        title="第一页"
                      >
                        <ChevronsLeft className="w-3.5 h-3.5" />
                      </button>
                      <button
                        onClick={() => setPage((p) => Math.max(1, p - 10))}
                        disabled={requests.page <= 1}
                        className="p-1 rounded hover:bg-slate-100 disabled:opacity-40 disabled:cursor-not-allowed transition-colors"
                        title="前10页"
                      >
                        <ChevronLeft className="w-3.5 h-3.5" />
                        <span className="text-[9px] -ml-0.5">10</span>
                      </button>
                      <button
                        onClick={() => setPage((p) => Math.max(1, p - 1))}
                        disabled={requests.page <= 1}
                        className="p-1 rounded hover:bg-slate-100 disabled:opacity-40 disabled:cursor-not-allowed transition-colors"
                        title="上一页"
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
                        title="下一页"
                      >
                        <ChevronRight className="w-3.5 h-3.5" />
                      </button>
                      <button
                        onClick={() => setPage((p) => Math.min(requests.total_pages, p + 10))}
                        disabled={requests.page >= requests.total_pages}
                        className="p-1 rounded hover:bg-slate-100 disabled:opacity-40 disabled:cursor-not-allowed transition-colors"
                        title="后10页"
                      >
                        <span className="text-[9px] -mr-0.5">10</span>
                        <ChevronRight className="w-3.5 h-3.5" />
                      </button>
                      <button
                        onClick={() => setPage(requests.total_pages)}
                        disabled={requests.page >= requests.total_pages}
                        className="p-1 rounded hover:bg-slate-100 disabled:opacity-40 disabled:cursor-not-allowed transition-colors"
                        title="最后一页"
                      >
                        <ChevronsRight className="w-3.5 h-3.5" />
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