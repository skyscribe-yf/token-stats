import { formatNumber, formatCost, formatPercent, getSourceColor, getSourceLabel } from "../lib/utils";
import type { AggregatedStats, SourceStats } from "../api";
import ZH from "../i18n/zh";

export function KpiStrip({
  overall,
  bySource,
}: {
  overall: AggregatedStats;
  bySource: SourceStats[];
}) {
  return (
    <div className="bg-white rounded-xl border border-slate-200 shadow-sm px-4 py-3 mb-3">
      <div className="flex flex-wrap items-center divide-x divide-slate-200 gap-y-1">
        <div className="pr-4 min-w-0">
          <p className="text-[11px] text-slate-400 font-medium">
            {ZH.totalCalls}
          </p>
          <p className="text-lg font-bold text-slate-800 leading-tight">
            {formatNumber(overall.total_calls)}
          </p>
        </div>
        <div className="px-4 min-w-0">
          <p className="text-[11px] text-slate-400 font-medium">
            {ZH.inputTokens}
          </p>
          <p className="text-lg font-bold text-emerald-600 leading-tight">
            {formatNumber(overall.total_input_tokens)}
          </p>
        </div>
        <div className="px-4 min-w-0">
          <p className="text-[11px] text-slate-400 font-medium">
            {ZH.outputTokens}
          </p>
          <p className="text-lg font-bold text-amber-600 leading-tight">
            {formatNumber(overall.total_output_tokens)}
          </p>
        </div>
        <div className="px-4 min-w-0">
          <p className="text-[11px] text-slate-400 font-medium">
            {ZH.cacheRead}
          </p>
          <p className="text-lg font-bold text-violet-600 leading-tight">
            {formatNumber(overall.total_cache_read_tokens)}
          </p>
        </div>
        <div className="px-4 min-w-0">
          <p className="text-[11px] text-slate-400 font-medium">
            {ZH.cacheHitRatio}
          </p>
          <p className="text-lg font-bold text-rose-600 leading-tight">
            {formatPercent(overall.weighted_cache_hit_ratio)}
          </p>
        </div>
        <div className="pl-4 min-w-0">
          <p className="text-[11px] text-slate-400 font-medium">
            {ZH.totalCost}
          </p>
          <p className="text-lg font-bold text-slate-800 leading-tight">
            {formatCost(overall.total_cost)}
          </p>
        </div>
      </div>

      {/* Source Overview - compact inline */}
      {bySource.length > 1 && (
        <div className="flex flex-wrap items-center gap-x-4 gap-y-1 mt-2 pt-2 border-t border-slate-100 text-xs text-slate-500">
          {bySource.map((s) => (
            <span key={s.source} className="inline-flex items-center gap-1.5">
              <span
                className="w-1.5 h-1.5 rounded-full"
                style={{ background: getSourceColor(s.source) }}
              />
              <span
                className="font-medium"
                style={{ color: getSourceColor(s.source) }}
              >
                {getSourceLabel(s.source)}
              </span>
              <span className="text-slate-400">{formatNumber(s.calls)}次</span>
              <span className="text-slate-400">·</span>
              <span className="text-slate-400">
                {formatNumber(s.total_tokens)}tok
              </span>
              <span className="text-slate-400">·</span>
              <span className="text-slate-400">
                {formatCost(s.cost, s.source)}
              </span>
              <span className="text-slate-400">·</span>
              <span className="text-slate-400">
                {formatPercent(s.cache_hit_ratio)}
              </span>
            </span>
          ))}
        </div>
      )}
    </div>
  );
}
