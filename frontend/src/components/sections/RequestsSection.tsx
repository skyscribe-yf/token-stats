import { memo, useMemo, useState, useCallback, Fragment, useEffect } from "react";
import {
  ChevronDown,
  ChevronRight as ChevronRightIcon,
  ChevronLeft,
  ChevronRight,
  ChevronsLeft,
  ChevronsRight,
  Filter,
  Settings,
  X,
} from "lucide-react";
import {
  formatNumber,
  formatCalls,
  formatCost,
  formatPercent,
  formatAvgCost,
  formatRpm,
  formatPeakRpm,
  formatTtft,
  formatTps,
  formatTime,
  getSourceColor,
  getSourceLabel,
} from "../../lib/utils";
import {
  buildPivotTree,
  computePivotSummary,
  getAdvancedDisplayModelSelection,
  getDisplayModel,
  type SortColumn,
  type SortDirection,
} from "../../lib/pivotTable";
import { toggleInSet } from "../../lib/timeRange";
import { useResizableColumns } from "../../lib/resizableColumns";
import { saveAdvancedModels } from "../../api";
import type { StatsResponse, PaginatedRequests } from "../../api";

const PIVOT_COLUMN_KEYS = [
  "name",
  "calls",
  "input",
  "output",
  "cache",
  "total",
  "cacheHit",
  "outputRatio",
  "cost",
  "avgCost",
  "avgRpm",
  "peakRpm",
  "avgTtft",
  "avgTps",
] as const;

const PIVOT_DEFAULT_WIDTHS: Record<string, number> = {
  name: 220,
  calls: 72,
  input: 88,
  output: 88,
  cache: 88,
  total: 92,
  cacheHit: 76,
  outputRatio: 72,
  cost: 80,
  avgCost: 84,
  avgRpm: 80,
  peakRpm: 80,
  avgTtft: 80,
  avgTps: 80,
};

interface RequestsSectionProps {
  stats: StatsResponse;
  requests: PaginatedRequests | null;
  hideFreeModels: boolean;
  showZeroTokens: boolean;
  onShowZeroTokensChange: (v: boolean) => void;
  page: number;
  onPageChange: (p: number) => void;

  // Pivot table local model multi-select (separate from sidebar's single-select)
  pivotModelOptions: string[];
  selectedPivotModels: ReadonlySet<string>;
  onSelectedPivotModelsChange: (s: Set<string>) => void;
  advancedModels: string[];
  onAdvancedModelsChange: (models: string[]) => void;
}

export const RequestsSection = memo(function RequestsSection({
  stats,
  requests,
  hideFreeModels,
  showZeroTokens,
  onShowZeroTokensChange,
  page,
  onPageChange,
  pivotModelOptions,
  selectedPivotModels,
  onSelectedPivotModelsChange,
  advancedModels,
  onAdvancedModelsChange,
}: RequestsSectionProps) {
  const [sortColumn, setSortColumn] = useState<SortColumn>("total_tokens");
  const [sortDirection, setSortDirection] = useState<SortDirection>("desc");
  const [expandedVendors, setExpandedVendors] = useState<Set<string>>(new Set());
  const [expandedModels, setExpandedModels] = useState<Set<string>>(new Set());
  const [pivotInitialized, setPivotInitialized] = useState(false);

  const [showModelFilter, setShowModelFilter] = useState(false);
  const [pendingPivotModels, setPendingPivotModels] = useState<Set<string>>(
    new Set(selectedPivotModels)
  );
  const [showAdvancedSettings, setShowAdvancedSettings] = useState(false);
  const [advancedModelsDraft, setAdvancedModelsDraft] = useState("");

  const vendorModelTree = useMemo(() => {
    if (!stats?.by_model) return [];
    return buildPivotTree(stats.by_model, sortColumn, sortDirection, hideFreeModels);
  }, [stats, sortColumn, sortDirection, hideFreeModels]);

  const pivotSummary = useMemo(
    () => computePivotSummary(vendorModelTree),
    [vendorModelTree]
  );

  useEffect(() => {
    if (pivotInitialized) return;
    if (!stats?.by_model || stats.by_model.length === 0) return;
    const vendors = new Set(stats.by_model.map((m) => m.provider));
    const models = new Set(
      stats.by_model.map((m) => `${m.provider}|${getDisplayModel(m.model)}`)
    );
    // eslint-disable-next-line react-hooks/set-state-in-effect
    setExpandedVendors(vendors);
    setExpandedModels(models);
    setPivotInitialized(true);
  }, [stats, pivotInitialized]);

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

  const { widths: pivotWidths, getHandleProps: getPivotHandleProps } =
    useResizableColumns({
      storageKey: "token-stats:pivot-col-widths",
      defaultWidths: PIVOT_DEFAULT_WIDTHS,
    });

  const pivotH = useCallback(
    (
      key: (typeof PIVOT_COLUMN_KEYS)[number],
      label: string,
      align: "left" | "right" = "left",
      sortCol?: SortColumn
    ): React.ReactNode => {
      const alignCls = { left: "text-left", right: "text-right", center: "text-center" }[align];
      const handleProps = getPivotHandleProps(key);
      const w = pivotWidths[key];
      const style: React.CSSProperties = w
        ? { width: `${w}px`, minWidth: `${w}px` }
        : {};
      return (
        <th
          key={key}
          className={`px-3 py-2 font-medium cursor-pointer hover:bg-slate-100 transition-colors select-none relative group ${alignCls}`}
          style={style}
          onClick={sortCol ? () => handleSort(sortCol) : undefined}
        >
          <span className="inline-flex items-center gap-0.5 whitespace-nowrap">
            <span>{label}</span>
            {sortCol && (
              <span className="text-slate-400">{sortIndicator(sortCol)}</span>
            )}
          </span>
          <span
            aria-hidden
            onMouseDown={handleProps.onMouseDown}
            onClick={handleProps.onClick}
            className="absolute right-0 top-0 bottom-0 w-1.5 cursor-col-resize opacity-0 group-hover:opacity-100 hover:bg-primary-400/60 active:bg-primary-500 transition-opacity z-10"
          />
        </th>
      );
    },
    [pivotWidths, getPivotHandleProps, sortColumn, sortDirection]
  );

  return (
    <section id="section-requests" className="space-y-3 scroll-mt-32">
      <h2 className="text-base font-semibold text-slate-800">请求</h2>

      {/* Vendor & Model Pivot Table */}
      <div className="bg-white rounded-xl border border-slate-200 shadow-sm">
        <div className="px-4 py-2.5 border-b border-slate-100 flex items-center justify-between">
          <h3 className="text-sm font-semibold text-slate-700">
            供应商 &amp; 模型表现
          </h3>
          <div className="relative model-filter-dropdown">
            <button
              onClick={() => {
                setPendingPivotModels(new Set(selectedPivotModels));
                setShowModelFilter((v) => !v);
              }}
              className={`inline-flex items-center gap-1 px-2 py-1 text-[11px] rounded-md border transition-colors ${
                selectedPivotModels.size > 0
                  ? "bg-blue-50 border-blue-300 text-blue-700"
                  : "bg-white border-slate-200 text-slate-500 hover:bg-slate-50"
              }`}
            >
              <Filter className="w-3 h-3" />
              模型筛选
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
                      <span className="text-xs font-semibold text-slate-700">
                        高级模型设置
                      </span>
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
                        className="px-2 py-1 text-[10px] font-medium rounded text-slate-500 hover:bg-slate-100"
                      >
                        取消
                      </button>
                      <button
                        onClick={async () => {
                          const models = advancedModelsDraft
                            .split("\n")
                            .map((s) => s.trim())
                            .filter((s) => s.length > 0);
                          try {
                            await saveAdvancedModels(models);
                            onAdvancedModelsChange(models);
                          } catch {
                            /* ignore */
                          }
                          setShowAdvancedSettings(false);
                        }}
                        className="px-2 py-1 text-[10px] font-medium rounded bg-primary-600 text-white hover:bg-primary-700"
                      >
                        应用
                      </button>
                    </div>
                  </>
                ) : (
                  <>
                    <div className="shrink-0 bg-white border-b border-slate-100 px-2 py-1.5 flex items-center justify-between">
                      <div className="flex gap-2">
                        <button
                          onClick={() =>
                            setPendingPivotModels(new Set(pivotModelOptions))
                          }
                          className="text-[10px] text-blue-600 hover:text-blue-800"
                        >
                          全选
                        </button>
                        <span className="text-slate-300">|</span>
                        <button
                          onClick={() => setPendingPivotModels(new Set())}
                          className="text-[10px] text-slate-500 hover:text-slate-700"
                        >
                          清除
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
                          <span className="truncate" title={model}>
                            {model}
                          </span>
                        </label>
                      ))}
                    </div>
                    <div className="shrink-0 bg-white border-t border-slate-100 px-2 py-1.5 flex justify-between items-center">
                      <button
                        onClick={() => {
                          const next = getAdvancedDisplayModelSelection(
                            advancedModels,
                            pivotModelOptions
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
                          className="px-2 py-1 text-[10px] font-medium rounded text-slate-500 hover:bg-slate-100"
                        >
                          取消
                        </button>
                        <button
                          onClick={() => {
                            onSelectedPivotModelsChange(pendingPivotModels);
                            setShowModelFilter(false);
                          }}
                          className="px-2 py-1 text-[10px] font-medium rounded bg-primary-600 text-white hover:bg-primary-700"
                        >
                          应用
                        </button>
                      </div>
                    </div>
                  </>
                )}
              </div>
            )}
          </div>
        </div>
        <div className="overflow-x-auto">
          <table className="w-full text-xs" style={{ tableLayout: "auto" }}>
            <thead>
              <tr className="bg-slate-50 text-slate-500 text-[10px] uppercase tracking-wider">
                {pivotH("name", "供应商 / 模型 / 工具", "left", "name")}
                {pivotH("calls", "调用次数", "right", "calls")}
                {pivotH("input", "输入", "right", "input_tokens")}
                {pivotH("output", "输出", "right", "output_tokens")}
                {pivotH("cache", "缓存", "right", "cache")}
                {pivotH("total", "合计", "right", "total_tokens")}
                {pivotH("cacheHit", "缓存命中", "right", "cache_hit_ratio")}
                {pivotH("outputRatio", "输出比", "right", "output_ratio")}
                {pivotH("cost", "费用", "right", "cost")}
                {pivotH("avgCost", "平均成本(/百万)", "right", "avg_cost")}
                {pivotH("avgRpm", "平均 RPM", "right", "avg_rpm")}
                {pivotH("peakRpm", "峰值 RPM", "right", "peak_rpm")}
                {pivotH("avgTtft", "平均 TTFT", "right", "avg_ttft_ms")}
                {pivotH("avgTps", "平均 TPS", "right", "avg_tps")}
              </tr>
            </thead>
            <tbody className="divide-y divide-slate-100">
              {vendorModelTree.map(({ provider, models, summary: vendorSummary }) => {
                const vendorExpanded = expandedVendors.has(provider);
                const vendorCacheHit = vendorSummary.cache_hit_ratio;
                return (
                  <Fragment key={provider}>
                    <tr
                      className="bg-slate-50/80 transition-colors cursor-pointer"
                      onClick={() =>
                        setExpandedVendors((prev) => toggleInSet(prev, provider))
                      }
                    >
                      <td className="px-3 py-2">
                        <div className="flex items-center gap-1.5">
                          {vendorExpanded ? (
                            <ChevronDown className="w-3 h-3 text-slate-400" />
                          ) : (
                            <ChevronRightIcon className="w-3 h-3 text-slate-400" />
                          )}
                          <span
                            className="w-2 h-2 rounded-full"
                            style={{ background: getSourceColor(provider) }}
                          />
                          <span className="font-bold text-slate-800">
                            {provider}
                          </span>
                        </div>
                      </td>
                      <td className="px-3 py-2 text-right font-semibold text-slate-700">
                        {formatCalls(vendorSummary.calls)}
                      </td>
                      <td className="px-3 py-2 text-right text-slate-600">
                        {formatNumber(vendorSummary.input_tokens)}
                      </td>
                      <td className="px-3 py-2 text-right text-slate-600">
                        {formatNumber(vendorSummary.output_tokens)}
                      </td>
                      <td className="px-3 py-2 text-right text-slate-600">
                        {formatNumber(
                          vendorSummary.cache_read_tokens +
                            vendorSummary.cache_write_tokens
                        )}
                      </td>
                      <td className="px-3 py-2 text-right font-bold text-slate-800">
                        {formatNumber(vendorSummary.total_tokens)}
                      </td>
                      <td className="px-3 py-2 text-right">
                        <span
                          className={`px-1.5 py-0.5 rounded-full text-[10px] font-medium ${
                            vendorCacheHit > 50
                              ? "bg-emerald-100 text-emerald-700"
                              : vendorCacheHit > 10
                                ? "bg-amber-100 text-amber-700"
                                : "bg-slate-100 text-slate-600"
                          }`}
                        >
                          {formatPercent(vendorCacheHit)}
                        </span>
                      </td>
                      <td className="px-3 py-2 text-right">
                        <span
                          className={`px-1.5 py-0.5 rounded-full text-[10px] font-medium ${
                            vendorSummary.output_ratio > 20
                              ? "bg-amber-100 text-amber-700"
                              : vendorSummary.output_ratio < 5
                                ? "bg-slate-100 text-slate-500"
                                : "bg-slate-100 text-slate-600"
                          }`}
                        >
                          {formatPercent(vendorSummary.output_ratio)}
                        </span>
                      </td>
                      <td className="px-3 py-2 text-right font-semibold text-slate-700">
                        {formatCost(vendorSummary.cost)}
                      </td>
                      <td className="px-3 py-2 text-right text-slate-600">
                        {formatAvgCost(
                          vendorSummary.cost,
                          vendorSummary.total_tokens
                        )}
                      </td>
                      <td className="px-3 py-2 text-right">
                        <span className={`px-1.5 py-0.5 rounded-full text-[10px] font-medium ${
                          vendorSummary.avg_rpm > 10
                            ? "bg-indigo-100 text-indigo-700"
                            : vendorSummary.avg_rpm > 1
                              ? "bg-slate-100 text-slate-600"
                              : "bg-slate-50 text-slate-400"
                        }`}>
                          {formatRpm(vendorSummary.avg_rpm)}
                        </span>
                      </td>
                      <td className="px-3 py-2 text-right">
                        <span className="px-1.5 py-0.5 rounded-full text-[10px] font-medium bg-rose-50 text-rose-600">
                          {formatPeakRpm(vendorSummary.peak_rpm)}
                        </span>
                      </td>
                      <td className="px-3 py-2 text-right">
                        <span className={`px-1.5 py-0.5 rounded-full text-[10px] font-medium ${
                          vendorSummary.avg_ttft_ms > 0
                            ? vendorSummary.avg_ttft_ms > 1000
                              ? "bg-amber-100 text-amber-700"
                              : "bg-emerald-100 text-emerald-700"
                            : "bg-slate-50 text-slate-400"
                        }`}>
                          {formatTtft(vendorSummary.avg_ttft_ms)}
                        </span>
                      </td>
                      <td className="px-3 py-2 text-right">
                        <span className={`px-1.5 py-0.5 rounded-full text-[10px] font-medium ${
                          vendorSummary.avg_tps > 0
                            ? vendorSummary.avg_tps > 50
                              ? "bg-emerald-100 text-emerald-700"
                              : "bg-slate-100 text-slate-600"
                            : "bg-slate-50 text-slate-400"
                        }`}>
                          {formatTps(vendorSummary.avg_tps)}
                        </span>
                      </td>
                    </tr>

                    {vendorExpanded &&
                      models.map((model) => {
                        const modelKey = `${provider}|${model.model}`;
                        const modelExpanded = expandedModels.has(modelKey);
                        const singleSource = model.source_details.length <= 1;
                        const ms = model.summary;
                        return (
                          <Fragment key={modelKey}>
                            <tr
                              className={`hover:bg-slate-50 transition-colors ${
                                singleSource ? "" : "cursor-pointer"
                              }`}
                              onClick={() => {
                                if (!singleSource) {
                                  setExpandedModels((prev) =>
                                    toggleInSet(prev, modelKey)
                                  );
                                }
                              }}
                            >
                              <td className="px-3 py-2 pl-8">
                                <div className="flex items-center gap-1.5 flex-wrap">
                                  {!singleSource &&
                                    (modelExpanded ? (
                                      <ChevronDown className="w-3 h-3 text-slate-400 shrink-0" />
                                    ) : (
                                      <ChevronRightIcon className="w-3 h-3 text-slate-400 shrink-0" />
                                    ))}
                                  <span className="font-medium text-slate-700">
                                    {model.model}
                                  </span>
                                  {model.source_details.map((sd) => (
                                    <span
                                      key={sd.source}
                                      className="inline-flex items-center gap-1 px-1.5 py-0.5 rounded text-[9px] font-medium"
                                      style={{
                                        background: `${getSourceColor(sd.source)}15`,
                                        color: getSourceColor(sd.source),
                                      }}
                                    >
                                      {getSourceLabel(sd.source)}
                                    </span>
                                  ))}
                                </div>
                              </td>
                              <td className="px-3 py-2 text-right text-slate-600">
                                {formatCalls(ms.calls)}
                              </td>
                              <td className="px-3 py-2 text-right text-slate-600">
                                {formatNumber(ms.input_tokens)}
                              </td>
                              <td className="px-3 py-2 text-right text-slate-600">
                                {formatNumber(ms.output_tokens)}
                              </td>
                              <td className="px-3 py-2 text-right text-slate-600">
                                {formatNumber(
                                  ms.cache_read_tokens + ms.cache_write_tokens
                                )}
                              </td>
                              <td className="px-3 py-2 text-right font-semibold text-slate-700">
                                {formatNumber(ms.total_tokens)}
                              </td>
                              <td className="px-3 py-2 text-right">
                                <span
                                  className={`px-1.5 py-0.5 rounded-full text-[10px] font-medium ${
                                    ms.cache_hit_ratio > 50
                                      ? "bg-emerald-100 text-emerald-700"
                                      : ms.cache_hit_ratio > 10
                                        ? "bg-amber-100 text-amber-700"
                                        : "bg-slate-100 text-slate-600"
                                  }`}
                                >
                                  {formatPercent(ms.cache_hit_ratio)}
                                </span>
                              </td>
                              <td className="px-3 py-2 text-right">
                                <span
                                  className={`px-1.5 py-0.5 rounded-full text-[10px] font-medium ${
                                    ms.output_ratio > 20
                                      ? "bg-amber-100 text-amber-700"
                                      : ms.output_ratio < 5
                                        ? "bg-slate-100 text-slate-500"
                                        : "bg-slate-100 text-slate-600"
                                  }`}
                                >
                                  {formatPercent(ms.output_ratio)}
                                </span>
                              </td>
                              <td className="px-3 py-2 text-right text-slate-600">
                                {formatCost(ms.cost)}
                              </td>
                              <td className="px-3 py-2 text-right text-slate-600">
                                {formatAvgCost(ms.cost, ms.total_tokens)}
                              </td>
                              <td className="px-3 py-2 text-right">
                                <span className={`px-1.5 py-0.5 rounded-full text-[10px] font-medium ${
                                  ms.avg_rpm > 10
                                    ? "bg-indigo-100 text-indigo-700"
                                    : ms.avg_rpm > 1
                                      ? "bg-slate-100 text-slate-600"
                                      : "bg-slate-50 text-slate-400"
                                }`}>
                                  {formatRpm(ms.avg_rpm)}
                                </span>
                              </td>
                              <td className="px-3 py-2 text-right">
                                <span className="px-1.5 py-0.5 rounded-full text-[10px] font-medium bg-rose-50 text-rose-600">
                                  {formatPeakRpm(ms.peak_rpm)}
                                </span>
                              </td>
                              <td className="px-3 py-2 text-right">
                                <span className={`px-1.5 py-0.5 rounded-full text-[10px] font-medium ${
                                  ms.avg_ttft_ms > 0
                                    ? ms.avg_ttft_ms > 1000
                                      ? "bg-amber-100 text-amber-700"
                                      : "bg-emerald-100 text-emerald-700"
                                    : "bg-slate-50 text-slate-400"
                                }`}>
                                  {formatTtft(ms.avg_ttft_ms)}
                                </span>
                              </td>
                              <td className="px-3 py-2 text-right">
                                <span className={`px-1.5 py-0.5 rounded-full text-[10px] font-medium ${
                                  ms.avg_tps > 0
                                    ? ms.avg_tps > 50
                                      ? "bg-emerald-100 text-emerald-700"
                                      : "bg-slate-100 text-slate-600"
                                    : "bg-slate-50 text-slate-400"
                                }`}>
                                  {formatTps(ms.avg_tps)}
                                </span>
                              </td>
                            </tr>
                            {modelExpanded &&
                              !singleSource &&
                              model.source_details.map((source) => (
                                <tr
                                  key={`${modelKey}|${source.source}`}
                                  className="hover:bg-slate-50/60 transition-colors"
                                >
                                  <td className="px-3 py-2 pl-14">
                                    <span
                                      className="inline-flex items-center gap-1 px-1.5 py-0.5 rounded text-[9px] font-medium"
                                      style={{
                                        background: `${getSourceColor(source.source)}15`,
                                        color: getSourceColor(source.source),
                                      }}
                                    >
                                      {getSourceLabel(source.source)}
                                    </span>
                                  </td>
                                  <td className="px-3 py-2 text-right text-slate-600">
                                    {formatCalls(source.calls)}
                                  </td>
                                  <td className="px-3 py-2 text-right text-slate-600">
                                    {formatNumber(source.input_tokens)}
                                  </td>
                                  <td className="px-3 py-2 text-right text-slate-600">
                                    {formatNumber(source.output_tokens)}
                                  </td>
                                  <td className="px-3 py-2 text-right text-slate-600">
                                    {formatNumber(
                                      source.cache_read_tokens +
                                        source.cache_write_tokens
                                    )}
                                  </td>
                                  <td className="px-3 py-2 text-right font-medium text-slate-700">
                                    {formatNumber(source.total_tokens)}
                                  </td>
                                  <td className="px-3 py-2 text-right">
                                    <span
                                      className={`px-1.5 py-0.5 rounded-full text-[10px] font-medium ${
                                        source.cache_hit_ratio > 50
                                          ? "bg-emerald-100 text-emerald-700"
                                          : source.cache_hit_ratio > 10
                                            ? "bg-amber-100 text-amber-700"
                                            : "bg-slate-100 text-slate-600"
                                      }`}
                                    >
                                      {formatPercent(source.cache_hit_ratio)}
                                    </span>
                                  </td>
                                  <td className="px-3 py-2 text-right">
                                    {(() => {
                                      const ratio = source.total_tokens > 0
                                        ? (source.output_tokens / source.total_tokens) * 100
                                        : 0;
                                      return (
                                        <span
                                          className={`px-1.5 py-0.5 rounded-full text-[10px] font-medium ${
                                            ratio > 20
                                              ? "bg-amber-100 text-amber-700"
                                              : ratio < 5
                                                ? "bg-slate-100 text-slate-500"
                                                : "bg-slate-100 text-slate-600"
                                          }`}
                                        >
                                          {formatPercent(ratio)}
                                        </span>
                                      );
                                    })()}
                                  </td>
                                  <td className="px-3 py-2 text-right text-slate-600">
                                    {formatCost(source.cost)}
                                  </td>
                                  <td className="px-3 py-2 text-right text-slate-600">
                                    {formatAvgCost(source.cost, source.total_tokens)}
                                  </td>
                                  <td className="px-3 py-2 text-right">
                                    <span className={`px-1.5 py-0.5 rounded-full text-[10px] font-medium ${
                                      source.avg_rpm > 10
                                        ? "bg-indigo-100 text-indigo-700"
                                        : source.avg_rpm > 1
                                          ? "bg-slate-100 text-slate-600"
                                          : "bg-slate-50 text-slate-400"
                                    }`}>
                                      {formatRpm(source.avg_rpm)}
                                    </span>
                                  </td>
                                  <td className="px-3 py-2 text-right">
                                    <span className="px-1.5 py-0.5 rounded-full text-[10px] font-medium bg-rose-50 text-rose-600">
                                      {formatPeakRpm(source.peak_rpm)}
                                    </span>
                                  </td>
                                  <td className="px-3 py-2 text-right">
                                    <span className={`px-1.5 py-0.5 rounded-full text-[10px] font-medium ${
                                      source.avg_ttft_ms > 0
                                        ? source.avg_ttft_ms > 1000
                                          ? "bg-amber-100 text-amber-700"
                                          : "bg-emerald-100 text-emerald-700"
                                        : "bg-slate-50 text-slate-400"
                                    }`}>
                                      {formatTtft(source.avg_ttft_ms)}
                                    </span>
                                  </td>
                                  <td className="px-3 py-2 text-right">
                                    <span className={`px-1.5 py-0.5 rounded-full text-[10px] font-medium ${
                                      source.avg_tps > 0
                                        ? source.avg_tps > 50
                                          ? "bg-emerald-100 text-emerald-700"
                                          : "bg-slate-100 text-slate-600"
                                        : "bg-slate-50 text-slate-400"
                                    }`}>
                                      {formatTps(source.avg_tps)}
                                    </span>
                                  </td>
                                </tr>
                              ))}
                          </Fragment>
                        );
                      })}
                  </Fragment>
                );
              })}

              {pivotSummary && (
                <tr className="bg-slate-100/80 border-t-2 border-slate-200">
                  <td className="px-3 py-2 font-bold text-slate-800">
                    当前视图汇总
                  </td>
                  <td className="px-3 py-2 text-right font-bold text-slate-800">
                    {formatCalls(pivotSummary.calls)}
                  </td>
                  <td className="px-3 py-2 text-right font-bold text-slate-800">
                    {formatNumber(pivotSummary.input_tokens)}
                  </td>
                  <td className="px-3 py-2 text-right font-bold text-slate-800">
                    {formatNumber(pivotSummary.output_tokens)}
                  </td>
                  <td className="px-3 py-2 text-right font-bold text-slate-800">
                    {formatNumber(
                      pivotSummary.cache_read_tokens +
                        pivotSummary.cache_write_tokens
                    )}
                  </td>
                  <td className="px-3 py-2 text-right font-bold text-slate-800">
                    {formatNumber(pivotSummary.total_tokens)}
                  </td>
                  <td className="px-3 py-2 text-right">
                    <span
                      className={`px-1.5 py-0.5 rounded-full text-[10px] font-bold ${
                        pivotSummary.cache_hit_ratio > 50
                          ? "bg-emerald-100 text-emerald-700"
                          : pivotSummary.cache_hit_ratio > 10
                            ? "bg-amber-100 text-amber-700"
                            : "bg-slate-100 text-slate-600"
                      }`}
                    >
                      {formatPercent(pivotSummary.cache_hit_ratio)}
                    </span>
                  </td>
                  <td className="px-3 py-2 text-right">
                    <span
                      className={`px-1.5 py-0.5 rounded-full text-[10px] font-bold ${
                        pivotSummary.output_ratio > 20
                          ? "bg-amber-100 text-amber-700"
                          : pivotSummary.output_ratio < 5
                            ? "bg-slate-100 text-slate-500"
                            : "bg-slate-100 text-slate-600"
                      }`}
                    >
                      {formatPercent(pivotSummary.output_ratio)}
                    </span>
                  </td>
                  <td className="px-3 py-2 text-right font-bold text-slate-800">
                    {formatCost(pivotSummary.cost)}
                  </td>
                  <td className="px-3 py-2 text-right font-bold text-slate-800">
                    {formatAvgCost(pivotSummary.cost, pivotSummary.total_tokens)}
                  </td>
                  <td className="px-3 py-2 text-right text-slate-400 text-[10px]">
                    {formatRpm(pivotSummary.avg_rpm)}
                  </td>
                  <td className="px-3 py-2 text-right text-slate-400 text-[10px]">
                    {formatPeakRpm(pivotSummary.peak_rpm)}
                  </td>
                  <td className="px-3 py-2 text-right text-slate-400 text-[10px]">
                    {formatTtft(pivotSummary.avg_ttft_ms)}
                  </td>
                  <td className="px-3 py-2 text-right text-slate-400 text-[10px]">
                    {formatTps(pivotSummary.avg_tps)}
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        </div>
      </div>

      {/* Detailed Requests */}
      <details className="group">
        <summary className="bg-white rounded-xl border border-slate-200 shadow-sm px-4 py-2.5 cursor-pointer select-none flex items-center gap-2 text-sm font-semibold text-slate-700 hover:bg-slate-50 transition-colors list-none">
          <svg
            className="w-3.5 h-3.5 text-slate-400 transition-transform group-open:rotate-90"
            fill="none"
            stroke="currentColor"
            viewBox="0 0 24 24"
          >
            <path
              strokeLinecap="round"
              strokeLinejoin="round"
              strokeWidth={2}
              d="M9 5l7 7-7 7"
            />
          </svg>
          详细请求
          <span className="text-[11px] text-slate-400 font-normal ml-1">
            ({requests ? formatNumber(requests.total) + " 条" : "…"})
          </span>
          <div className="ml-auto flex items-center gap-1.5" onClick={(e) => e.preventDefault()}>
            <label className="flex items-center gap-1 text-[10px] text-slate-400 font-normal cursor-pointer">
              <input
                type="checkbox"
                checked={showZeroTokens}
                onChange={(e) => onShowZeroTokensChange(e.target.checked)}
                className="w-3 h-3 rounded border-slate-300 text-blue-600 focus:ring-blue-500"
              />
              显示空请求
            </label>
          </div>
        </summary>
        <div className="mt-1.5 bg-white rounded-xl border border-slate-200 shadow-sm overflow-x-auto">
          <table className="w-full text-xs">
            <thead>
              <tr className="bg-slate-50 text-slate-500 text-[10px] uppercase tracking-wider">
                <th className="px-3 py-2 text-left font-medium">日期</th>
                <th className="px-3 py-2 text-left font-medium">供应商</th>
                <th className="px-3 py-2 text-left font-medium">模型</th>
                <th className="px-3 py-2 text-left font-medium">工具</th>
                <th className="px-3 py-2 text-right font-medium">输入</th>
                <th className="px-3 py-2 text-right font-medium">输出</th>
                <th className="px-3 py-2 text-right font-medium">缓存</th>
                <th className="px-3 py-2 text-right font-medium">合计</th>
                <th className="px-3 py-2 text-right font-medium">缓存命中</th>
                <th className="px-3 py-2 text-right font-medium">输出比</th>
                <th className="px-3 py-2 text-right font-medium">费用</th>
                <th className="px-3 py-2 text-right font-medium">TTFT</th>
                <th className="px-3 py-2 text-right font-medium">TPS</th>
              </tr>
            </thead>
            <tbody className="divide-y divide-slate-100">
              {requests?.data.map((r, i) => (
                <tr key={i} className="hover:bg-slate-50 transition-colors">
                  <td className="px-3 py-2 text-slate-600 whitespace-nowrap">
                    {formatTime(r.time)}
                  </td>
                  <td className="px-3 py-2">
                    <span
                      className="text-xs font-medium"
                      style={{ color: getSourceColor(r.provider) }}
                    >
                      {r.provider}
                    </span>
                  </td>
                  <td className="px-3 py-2 text-slate-600">
                    {getDisplayModel(r.model)}
                  </td>
                  <td className="px-3 py-2">
                    <span
                      className="inline-flex items-center gap-1 px-1.5 py-0.5 rounded text-[9px] font-medium"
                      style={{
                        background: `${getSourceColor(r.source)}15`,
                        color: getSourceColor(r.source),
                      }}
                    >
                      {getSourceLabel(r.source)}
                    </span>
                  </td>
                  <td className="px-3 py-2 text-right text-slate-600">
                    {formatNumber(r.input_tokens)}
                  </td>
                  <td className="px-3 py-2 text-right text-slate-600">
                    {formatNumber(r.output_tokens)}
                  </td>
                  <td className="px-3 py-2 text-right text-slate-600">
                    {formatNumber(r.cache_read_tokens + r.cache_write_tokens)}
                  </td>
                  <td className="px-3 py-2 text-right font-semibold text-slate-700">
                    {formatNumber(r.total_tokens)}
                  </td>
                  <td className="px-3 py-2 text-right">
                    <span
                      className={`px-1.5 py-0.5 rounded-full text-[10px] font-medium ${
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
                  <td className="px-3 py-2 text-right">
                    {(() => {
                      const ratio = r.total_tokens > 0
                        ? (r.output_tokens / r.total_tokens) * 100
                        : 0;
                      return (
                        <span
                          className={`px-1.5 py-0.5 rounded-full text-[10px] font-medium ${
                            ratio > 20
                              ? "bg-amber-100 text-amber-700"
                              : ratio < 5
                                ? "bg-slate-100 text-slate-500"
                                : "bg-slate-100 text-slate-600"
                          }`}
                        >
                          {formatPercent(ratio)}
                        </span>
                      );
                    })()}
                  </td>
                  <td className="px-3 py-2 text-right text-slate-600">
                    {formatCost(r.cost, r.source)}
                  </td>
                  <td className="px-3 py-2 text-right text-slate-600">
                    {r.ttft_ms != null ? (
                      <span className="px-1.5 py-0.5 rounded-full text-[10px] font-medium bg-slate-100 text-slate-600">
                        {Math.round(r.ttft_ms)}ms
                      </span>
                    ) : (
                      <span className="text-slate-300">—</span>
                    )}
                  </td>
                  <td className="px-3 py-2 text-right text-slate-600">
                    {r.tps != null ? (
                      <span className="px-1.5 py-0.5 rounded-full text-[10px] font-medium bg-slate-100 text-slate-600">
                        {r.tps.toFixed(1)} t/s
                      </span>
                    ) : (
                      <span className="text-slate-300">—</span>
                    )}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>

          {requests && requests.total_pages > 1 && (
            <div className="px-3 py-2 border-t border-slate-100 flex items-center justify-between">
              <p className="text-xs text-slate-500">
                显示 {(requests.page - 1) * requests.limit + 1}-
                {Math.min(requests.page * requests.limit, requests.total)} /
                {" "}
                {formatNumber(requests.total)} 条
              </p>
              <div className="flex items-center gap-1">
                <button
                  onClick={() => onPageChange(1)}
                  disabled={requests.page <= 1}
                  className="p-1 rounded hover:bg-slate-100 disabled:opacity-40 disabled:cursor-not-allowed transition-colors"
                  title="第一页"
                >
                  <ChevronsLeft className="w-3.5 h-3.5" />
                </button>
                <button
                  onClick={() => onPageChange(Math.max(1, page - 10))}
                  disabled={requests.page <= 1}
                  className="p-1 rounded hover:bg-slate-100 disabled:opacity-40 disabled:cursor-not-allowed transition-colors"
                  title="前10页"
                >
                  <ChevronLeft className="w-3.5 h-3.5" />
                  <span className="text-[9px] -ml-0.5">10</span>
                </button>
                <button
                  onClick={() => onPageChange(Math.max(1, page - 1))}
                  disabled={requests.page <= 1}
                  className="p-1 rounded hover:bg-slate-100 disabled:opacity-40 disabled:cursor-not-allowed transition-colors"
                  title="上一页"
                >
                  <ChevronLeft className="w-3.5 h-3.5" />
                </button>
                {Array.from(
                  { length: Math.min(5, requests.total_pages) },
                  (_, i) => {
                    let pageNum: number;
                    if (requests.total_pages <= 5) pageNum = i + 1;
                    else if (requests.page <= 3) pageNum = i + 1;
                    else if (requests.page >= requests.total_pages - 2)
                      pageNum = requests.total_pages - 4 + i;
                    else pageNum = requests.page - 2 + i;
                    return (
                      <button
                        key={pageNum}
                        onClick={() => onPageChange(pageNum)}
                        className={`w-7 h-7 rounded text-xs font-medium transition-colors ${
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
                    onPageChange(Math.min(requests.total_pages, page + 1))
                  }
                  disabled={requests.page >= requests.total_pages}
                  className="p-1 rounded hover:bg-slate-100 disabled:opacity-40 disabled:cursor-not-allowed transition-colors"
                  title="下一页"
                >
                  <ChevronRight className="w-3.5 h-3.5" />
                </button>
                <button
                  onClick={() =>
                    onPageChange(Math.min(requests.total_pages, page + 10))
                  }
                  disabled={requests.page >= requests.total_pages}
                  className="p-1 rounded hover:bg-slate-100 disabled:opacity-40 disabled:cursor-not-allowed transition-colors"
                  title="后10页"
                >
                  <span className="text-[9px] -mr-0.5">10</span>
                  <ChevronRight className="w-3.5 h-3.5" />
                </button>
                <button
                  onClick={() => onPageChange(requests.total_pages)}
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
    </section>
  );
});
