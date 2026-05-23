import {
  useState,
  useEffect,
  useMemo,
  useRef,
  useCallback,
} from "react";
import { X } from "lucide-react";
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
  computeNextBillingDate,
  isWithin24Hours,
} from "./lib/utils";
import { getDisplayModel, getOriginalModels } from "./lib/pivotTable";
import {
  buildCsvFilterParam,
  isEmptyAppliedSelection,
  type AppliedRange,
} from "./lib/filterState";
import {
  makeAppliedRange,
  toggleInSet,
  type TimePreset,
} from "./lib/timeRange";

import { TopBar, type SectionId } from "./components/TopBar";
import { Sidebar } from "./components/Sidebar";
import { SettingsDrawer } from "./components/SettingsDrawer";
import { GlanceBand } from "./components/GlanceBand";
import {
  UsageSection,
  type ChartMetricKey,
  type VendorBreakdownMetric,
} from "./components/sections/UsageSection";
import { QuotasSection } from "./components/sections/QuotasSection";
import { RequestsSection } from "./components/sections/RequestsSection";

interface AlertItem {
  id: string;
  provider: string;
  type: "quota_low" | "expiring_soon";
  message: string;
  detail: string;
}

const LS_ACTIVE_SECTION = "token-stats:active-section";
const LS_VENDOR_METRIC = "token-stats:vendor-breakdown-metric";

function readLs<T extends string>(key: string, fallback: T): T {
  if (typeof window === "undefined") return fallback;
  try {
    const raw = window.localStorage.getItem(key);
    if (raw == null) return fallback;
    const parsed = JSON.parse(raw);
    return typeof parsed === "string" ? (parsed as T) : fallback;
  } catch {
    return fallback;
  }
}

function writeLs(key: string, value: string) {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.setItem(key, JSON.stringify(value));
  } catch {
    /* ignore */
  }
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
  // ─── Time range / filters ──────────────────────────────────────────────
  const [activePreset, setActivePreset] = useState<TimePreset>("today");
  const [appliedRange, setAppliedRange] = useState<AppliedRange>(() =>
    makeAppliedRange("today")
  );
  const filtersInitializedRef = useRef(false);

  const [filters, setFilters] = useState<FilterOptions>({
    vendors: [],
    models: [],
    sources: [],
  });
  const [selectedVendors, setSelectedVendors] = useState<Set<string>>(new Set());
  const [selectedSources, setSelectedSources] = useState<Set<string>>(new Set());
  const [selectedModel, setSelectedModel] = useState<string>("");
  const [hideFreeModels, setHideFreeModels] = useState(false);
  const [page, setPage] = useState(1);

  // ─── Data ──────────────────────────────────────────────────────────────
  const [stats, setStats] = useState<StatsResponse | null>(null);
  const [hourlyStats, setHourlyStats] = useState<StatsResponse | null>(null);
  const [requests, setRequests] = useState<PaginatedRequests | null>(null);
  const [quota, setQuota] = useState<QuotaResponse | null>(null);
  const [quotaLoading, setQuotaLoading] = useState(true);
  const [xunfei, setXunfei] = useState<XunfeiMultiStatus | null>(null);
  const [xunfeiLoading, setXunfeiLoading] = useState(true);
  const [ainaibaCredit, setAinaibaCredit] =
    useState<AinaibaCreditResponse | null>(null);
  const [ainaibaCreditLoading, setAinaibaCreditLoading] = useState(true);

  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [lastUpdatedAt, setLastUpdatedAt] = useState<Date | null>(null);

  // ─── UI state ─────────────────────────────────────────────────────────
  const [showSettings, setShowSettings] = useState(false);
  const [sidebarOpen, setSidebarOpen] = useState(false);
  const [activeSection, setActiveSection] = useState<SectionId>(() =>
    readLs<SectionId>(LS_ACTIVE_SECTION, "usage")
  );
  const [highlightCardId, setHighlightCardId] = useState<string | null>(null);
  const initialSectionRestoredRef = useRef(false);

  const [chartMetrics, setChartMetrics] = useState<Set<ChartMetricKey>>(
    () => new Set(["cache", "input", "output", "cacheHitRatio"])
  );
  const [vendorBreakdownMetric, setVendorBreakdownMetric] =
    useState<VendorBreakdownMetric>(() =>
      readLs<VendorBreakdownMetric>(LS_VENDOR_METRIC, "cost")
    );

  const [pricingConfig, setPricingConfig] = useState<PricingConfig | null>(null);
  const [subscriptionSettings, setSubscriptionSettings] =
    useState<SubscriptionSettings | null>(null);
  const [showAlertModal, setShowAlertModal] = useState(false);
  const [dismissedAlerts, setDismissedAlerts] = useState<Set<string>>(new Set());

  const [restoreLoading, setRestoreLoading] = useState(false);
  const [restoreResult, setRestoreResult] = useState<RestoreResponse | null>(
    null
  );
  const [restoreError, setRestoreError] = useState<string | null>(null);

  const [advancedModels, setAdvancedModels] = useState<string[]>([]);
  const [selectedPivotModels, setSelectedPivotModels] = useState<Set<string>>(
    new Set()
  );

  // ─── Derived ──────────────────────────────────────────────────────────
  const tzOffset = useMemo(() => -new Date().getTimezoneOffset(), []);

  const sourceFilter = useMemo(
    () => buildCsvFilterParam(selectedSources, filters.sources),
    [selectedSources, filters.sources]
  );

  const vendorFilter = useMemo(
    () => buildCsvFilterParam(selectedVendors, filters.vendors),
    [selectedVendors, filters.vendors]
  );

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
  const hasEmptyRequiredSelection =
    hasEmptySourceSelection || hasEmptyVendorSelection;

  const resolution = useMemo(() => {
    if (!appliedRange.from || !appliedRange.to) return undefined;
    const fromMs = new Date(appliedRange.from).getTime();
    const toMs = new Date(appliedRange.to).getTime();
    if (isNaN(fromMs) || isNaN(toMs)) return undefined;
    const rangeMs = toMs - fromMs;
    const oneDayMs = 24 * 60 * 60 * 1000;
    if (rangeMs < 4 * 60 * 60 * 1000) return "1h";
    if (rangeMs < oneDayMs) return "2h";
    if (rangeMs < 3 * oneDayMs) return "12h";
    return undefined;
  }, [appliedRange.from, appliedRange.to]);

  const filteredModels = useMemo(() => {
    const rawModels = stats?.by_model
      ? [...new Set(stats.by_model.map((m) => m.model))]
      : filters.models;
    return [...new Set(rawModels.map(getDisplayModel))].sort();
  }, [stats, filters.models]);

  const effectiveModel = useMemo(() => {
    if (!selectedModel) return "";
    if (filteredModels.includes(selectedModel)) return selectedModel;
    return "";
  }, [selectedModel, filteredModels]);

  const pivotModelOptions = useMemo(
    () => [...filters.models].sort(),
    [filters.models]
  );

  // ─── Data loading ─────────────────────────────────────────────────────
  const loadData = useCallback(async () => {
    if (!appliedRange.from || !appliedRange.to) return;
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
          appliedRange.from,
          appliedRange.to,
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
    } catch (e) {
      setError(e instanceof Error ? e.message : "加载数据失败");
    } finally {
      setLoading(false);
    }
  }, [
    appliedRange.from,
    appliedRange.to,
    sourceFilter,
    vendorFilter,
    tzOffset,
    resolution,
    modelFilter,
    hasEmptyRequiredSelection,
  ]);

  const loadRequests = useCallback(async () => {
    if (!appliedRange.from || !appliedRange.to) return;
    if (hasEmptyRequiredSelection) {
      setRequests(emptyRequests(1));
      return;
    }
    try {
      const modelParam = effectiveModel
        ? getOriginalModels(effectiveModel)?.join(",") || effectiveModel
        : undefined;
      const r = await fetchRequests(
        appliedRange.from,
        appliedRange.to,
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
    appliedRange.from,
    appliedRange.to,
    vendorFilter,
    effectiveModel,
    sourceFilter,
    page,
    tzOffset,
    hasEmptyRequiredSelection,
  ]);

  useEffect(() => {
    // eslint-disable-next-line react-hooks/set-state-in-effect
    void loadData();
  }, [loadData]);

  useEffect(() => {
    // eslint-disable-next-line react-hooks/set-state-in-effect
    void loadRequests();
  }, [loadRequests]);

  useEffect(() => {
    const interval = setInterval(() => {
      void loadData();
      void loadRequests();
    }, 30000);
    return () => clearInterval(interval);
  }, [loadData, loadRequests]);

  useEffect(() => {
    const load = async () => {
      try {
        const q = await fetchQuota();
        setQuota(q);
      } catch {
        /* optional */
      } finally {
        setQuotaLoading(false);
      }
    };
    load();
    const interval = setInterval(load, 30_000);
    return () => clearInterval(interval);
  }, []);

  useEffect(() => {
    const load = async () => {
      try {
        const x = await fetchXunfei();
        setXunfei(x);
      } catch {
        /* optional */
      } finally {
        setXunfeiLoading(false);
      }
    };
    load();
    const interval = setInterval(load, 30_000);
    return () => clearInterval(interval);
  }, []);

  useEffect(() => {
    const load = async () => {
      try {
        const x = await fetchAinaibaCredit();
        setAinaibaCredit(x);
      } catch {
        /* optional */
      } finally {
        setAinaibaCreditLoading(false);
      }
    };
    load();
    const interval = setInterval(load, 30_000);
    return () => clearInterval(interval);
  }, []);

  useEffect(() => {
    const doRefresh = async () => {
      try {
        await fetchRefresh();
        void loadData();
        void loadRequests();
      } catch {
        /* best effort */
      }
    };
    doRefresh();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    fetchPricing()
      .then(setPricingConfig)
      .catch(() => {});
  }, []);

  useEffect(() => {
    fetchAdvancedModels()
      .then(setAdvancedModels)
      .catch(() => {});
  }, []);

  useEffect(() => {
    fetchSubscriptionSettings()
      .then(setSubscriptionSettings)
      .catch(() => {});
  }, []);

  useEffect(() => {
    const load = async () => {
      if (!appliedRange.from || !appliedRange.to) return;
      if (hasEmptyRequiredSelection) {
        setHourlyStats(null);
        return;
      }
      try {
        const s = await fetchStats(
          appliedRange.from,
          appliedRange.to,
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
    load();
  }, [
    appliedRange.from,
    appliedRange.to,
    sourceFilter,
    vendorFilter,
    tzOffset,
    hasEmptyRequiredSelection,
    modelFilter,
  ]);

  // ─── Alerts ────────────────────────────────────────────────────────────
  const computedAlerts = useMemo<AlertItem[]>(() => {
    if (!quota && !xunfei && !ainaibaCredit) return [];
    const alerts: AlertItem[] = [];

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

    if (subscriptionSettings?.kimi_monthly_start_day) {
      const nextBilling = computeNextBillingDate(
        subscriptionSettings.kimi_monthly_start_day
      );
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

    const checkOpenCode = (
      status: OpenCodeQuotaStatus | null | undefined,
      label: string
    ) => {
      if (!status?.available || !status.data) return;
      const suffix = label === "ex" ? " (EX)" : "";
      for (const entry of status.data.entries) {
        if (entry.percentage >= 80) {
          const typeLabel =
            entry.usage_type === "Rolling"
              ? "滚动"
              : entry.usage_type === "Weekly"
                ? "周"
                : entry.usage_type === "Monthly"
                  ? "月"
                  : entry.usage_type;
          alerts.push({
            id: `opencode_${label}_${entry.usage_type}_low`,
            provider: `OpenCode-go${suffix}`,
            type: "quota_low",
            message: `OpenCode-go${suffix} ${typeLabel}限额已用 ${entry.percentage}%`,
            detail: `重置于 ${entry.resets_in}`,
          });
        }
        if (
          entry.usage_type === "Monthly" &&
          entry.reset_at &&
          isWithin24Hours(entry.reset_at)
        ) {
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

    if (xunfei?.accounts) {
      for (const acc of xunfei.accounts) {
        const suffix = acc.label === "ex" ? " (EX)" : "";
        if (acc.available && acc.data) {
          const ratio =
            acc.data.usage.package_left /
            Math.max(acc.data.usage.package_limit, 1);
          if (ratio <= 0.2) {
            alerts.push({
              id: `xunfei_${acc.label}_quota_low`,
              provider: `讯飞${suffix}`,
              type: "quota_low",
              message: `讯飞编程套餐${suffix} 月度余量不足`,
              detail: `月度已用 ${((1 - ratio) * 100).toFixed(0)}%，建议切换至其他模型`,
            });
          }
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

    if (ainaibaCredit?.available && ainaibaCredit.data) {
      const abRatio =
        ainaibaCredit.data.balance /
        Math.max(ainaibaCredit.data.credit_total, 1);
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

    return alerts.filter((a) => !dismissedAlerts.has(a.id));
  }, [quota, xunfei, ainaibaCredit, subscriptionSettings, dismissedAlerts]);

  const alertItems = computedAlerts;

  useEffect(() => {
    if (computedAlerts.length > 0) {
      // eslint-disable-next-line react-hooks/set-state-in-effect
      setShowAlertModal(true);
    }
  }, [computedAlerts]);

  // ─── Section nav (IntersectionObserver + restore on load) ─────────────
  useEffect(() => {
    if (!stats) return;
    if (initialSectionRestoredRef.current) return;
    initialSectionRestoredRef.current = true;
    const saved = readLs<SectionId>(LS_ACTIVE_SECTION, "usage");
    if (saved !== "usage") {
      const el = document.getElementById(`section-${saved}`);
      if (el) {
        el.scrollIntoView({ behavior: "auto", block: "start" });
      }
    }
  }, [stats]);

  useEffect(() => {
    if (!stats) return;
    const ids: SectionId[] = ["usage", "quotas", "requests"];
    const observer = new IntersectionObserver(
      (entries) => {
        const visible = entries
          .filter((e) => e.isIntersecting)
          .map((e) => e.target.id.replace("section-", "") as SectionId);
        if (visible.length > 0) {
          setActiveSection(visible[0]);
        }
      },
      {
        rootMargin: "-92px 0px -60% 0px",
        threshold: 0,
      }
    );
    for (const id of ids) {
      const el = document.getElementById(`section-${id}`);
      if (el) observer.observe(el);
    }
    return () => observer.disconnect();
  }, [stats]);

  useEffect(() => {
    writeLs(LS_ACTIVE_SECTION, activeSection);
  }, [activeSection]);

  useEffect(() => {
    writeLs(LS_VENDOR_METRIC, vendorBreakdownMetric);
  }, [vendorBreakdownMetric]);

  // ─── Event handlers ───────────────────────────────────────────────────
  const handleTimeRangeChange = useCallback(
    (preset: TimePreset, range: AppliedRange) => {
      setActivePreset(preset);
      setAppliedRange(range);
      setPage(1);
    },
    []
  );

  const handleSourceToggle = useCallback((source: string) => {
    setSelectedSources((prev) => toggleInSet(prev, source));
    setPage(1);
  }, []);

  const handleVendorToggle = useCallback((vendor: string) => {
    setSelectedVendors((prev) => toggleInSet(prev, vendor));
    setPage(1);
  }, []);

  const handleSubscriptionGroupToggle = useCallback(
    (selectAll: boolean) => {
      const subVendors = filters.vendors.filter((v) =>
        ["kimi", "xunfei", "opencode-go", "opencode"].includes(v)
      );
      setSelectedVendors((prev) => {
        const next = new Set(prev);
        for (const v of subVendors) {
          if (selectAll) next.add(v);
          else next.delete(v);
        }
        return next;
      });
      setPage(1);
    },
    [filters.vendors]
  );

  const handleModelChange = useCallback((model: string) => {
    setSelectedModel(model);
    setPage(1);
  }, []);

  const handleSectionSelect = useCallback((id: SectionId) => {
    setActiveSection(id);
    const el = document.getElementById(`section-${id}`);
    if (el) {
      el.scrollIntoView({ behavior: "smooth", block: "start" });
    }
  }, []);

  const handleQuotaChipClick = useCallback((cardId: string) => {
    setActiveSection("quotas");
    const card = document.getElementById(cardId);
    if (card) {
      card.scrollIntoView({ behavior: "smooth", block: "start" });
      setHighlightCardId(cardId);
      window.setTimeout(() => setHighlightCardId(null), 1300);
    } else {
      const section = document.getElementById("section-quotas");
      if (section) {
        section.scrollIntoView({ behavior: "smooth", block: "start" });
      }
    }
  }, []);

  const handleManualRefresh = useCallback(() => {
    void loadData();
    void loadRequests();
  }, [loadData, loadRequests]);

  const handleSaveSubscriptionSettings = useCallback(async () => {
    if (!subscriptionSettings) return;
    await saveSubscriptionSettings(subscriptionSettings);
  }, [subscriptionSettings]);

  const handleExport = useCallback(async () => {
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
  }, []);

  const handleRestore = useCallback(async (path: string) => {
    setRestoreLoading(true);
    setRestoreError(null);
    setRestoreResult(null);
    try {
      const result = await restoreBackup(path);
      setRestoreResult(result);
    } catch (e: unknown) {
      setRestoreError(e instanceof Error ? e.message : "恢复失败");
    } finally {
      setRestoreLoading(false);
    }
  }, []);

  const dismissAllAlerts = () => {
    setShowAlertModal(false);
    setDismissedAlerts((prev) => {
      const next = new Set(prev);
      alertItems.forEach((a) => next.add(a.id));
      return next;
    });
  };

  return (
    <div className="min-h-screen bg-slate-50">
      <TopBar
        title="Token 统计仪表盘"
        lastUpdatedAt={lastUpdatedAt}
        loading={loading}
        onRefresh={handleManualRefresh}
        onToggleSidebar={() => setSidebarOpen((v) => !v)}
        activeSection={activeSection}
        onSectionSelect={handleSectionSelect}
      />

      <div className="flex items-start">
        {showSettings ? (
          <SettingsDrawer
            open={showSettings}
            onClose={() => setShowSettings(false)}
            subscriptionSettings={subscriptionSettings}
            onSubscriptionSettingsChange={setSubscriptionSettings}
            onSaveSubscriptionSettings={handleSaveSubscriptionSettings}
            pricingConfig={pricingConfig}
            onExport={handleExport}
            onRestore={handleRestore}
            restoreLoading={restoreLoading}
            restoreResult={restoreResult}
            restoreError={restoreError}
          />
        ) : (
          <Sidebar
            open={sidebarOpen}
            onClose={() => setSidebarOpen(false)}
            activePreset={activePreset}
            onTimeRangeChange={handleTimeRangeChange}
            sources={filters.sources}
            selectedSources={selectedSources}
            onSourceToggle={handleSourceToggle}
            vendors={filters.vendors}
            selectedVendors={selectedVendors}
            onVendorToggle={handleVendorToggle}
            onSubscriptionGroupToggle={handleSubscriptionGroupToggle}
            models={filteredModels}
            selectedModel={selectedModel}
            onModelChange={handleModelChange}
            hideFreeModels={hideFreeModels}
            onHideFreeModelsChange={setHideFreeModels}
            onOpenSettings={() => setShowSettings(true)}
          />
        )}

        <main className="flex-1 min-w-0 px-4 py-3 space-y-4">
          {error && (
            <div className="p-3 bg-rose-50 border border-rose-200 rounded-lg text-rose-700 text-xs">
              {error}
            </div>
          )}

          {loading && !stats ? (
            <div className="flex items-center justify-center h-64">
              <div className="animate-spin rounded-full h-8 w-8 border-b-2 border-primary-600" />
            </div>
          ) : stats ? (
            <>
              <GlanceBand
                overall={stats.overall}
                quota={quota}
                xunfei={xunfei}
                ainaibaCredit={ainaibaCredit}
                quotaLoading={
                  quotaLoading || xunfeiLoading || ainaibaCreditLoading
                }
                onChipClick={handleQuotaChipClick}
              />

              <UsageSection
                stats={stats}
                hourlyStats={hourlyStats}
                chartMetrics={chartMetrics}
                onChartMetricsChange={setChartMetrics}
                vendorBreakdownMetric={vendorBreakdownMetric}
                onVendorBreakdownMetricChange={setVendorBreakdownMetric}
              />

              <QuotasSection
                quota={quota}
                xunfei={xunfei}
                ainaibaCredit={ainaibaCredit}
                quotaLoading={quotaLoading}
                xunfeiLoading={xunfeiLoading}
                ainaibaCreditLoading={ainaibaCreditLoading}
                subscriptionSettings={subscriptionSettings}
                highlightCardId={highlightCardId}
              />

              <RequestsSection
                stats={stats}
                requests={requests}
                hideFreeModels={hideFreeModels}
                page={page}
                onPageChange={setPage}
                pivotModelOptions={pivotModelOptions}
                selectedPivotModels={selectedPivotModels}
                onSelectedPivotModelsChange={setSelectedPivotModels}
                advancedModels={advancedModels}
                onAdvancedModelsChange={setAdvancedModels}
              />

              <footer className="text-center text-xs text-slate-400 pb-4">
                Token 统计仪表盘 · 基于 Rust + React 构建
              </footer>
            </>
          ) : null}
        </main>
      </div>

      {showAlertModal && alertItems.length > 0 && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-4">
          <div className="bg-white rounded-xl border border-slate-200 shadow-xl max-w-lg w-full max-h-[80vh] overflow-y-auto p-5">
            <div className="flex items-center justify-between mb-4">
              <h2 className="text-sm font-bold text-slate-800">⚠️ 订阅提醒</h2>
              <button
                onClick={dismissAllAlerts}
                className="p-0.5 rounded hover:bg-slate-100 text-slate-400"
              >
                <X className="w-4 h-4" />
              </button>
            </div>
            <div className="space-y-3">
              {alertItems.map((item) => (
                <div
                  key={item.id}
                  className={
                    "rounded-lg border p-3 " +
                    (item.type === "quota_low"
                      ? "border-amber-200 bg-amber-50"
                      : "border-rose-200 bg-rose-50")
                  }
                >
                  <div className="flex items-center gap-1.5 mb-1">
                    <span
                      className={
                        "text-[11px] font-semibold " +
                        (item.type === "quota_low"
                          ? "text-amber-700"
                          : "text-rose-700")
                      }
                    >
                      {item.provider}
                    </span>
                    <span
                      className={
                        "px-1.5 py-0 rounded-full text-[9px] font-medium " +
                        (item.type === "quota_low"
                          ? "bg-amber-100 text-amber-700"
                          : "bg-rose-100 text-rose-700")
                      }
                    >
                      {item.type === "quota_low" ? "余量不足" : "即将到期"}
                    </span>
                  </div>
                  <p className="text-[11px] text-slate-700">{item.message}</p>
                  <p className="text-[10px] text-slate-500 mt-0.5">
                    {item.detail}
                  </p>
                </div>
              ))}
            </div>
            <div className="mt-4 flex justify-end gap-2">
              <button
                onClick={dismissAllAlerts}
                className="px-3 py-1.5 text-xs font-medium rounded-md bg-slate-100 text-slate-600 hover:bg-slate-200"
              >
                知道了
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
