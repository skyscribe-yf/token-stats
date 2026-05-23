import { useMemo } from "react";
import {
  formatCalls,
  formatNumber,
  formatPercent,
  formatCost,
} from "../lib/utils";
import type { AggregatedStats } from "../api";

interface KpiStripProps {
  overall: AggregatedStats;
}

export function KpiStrip({ overall }: KpiStripProps) {
  const tokenSegments = useMemo(() => {
    const total = overall.total_tokens || 1;
    return [
      {
        key: "input",
        label: "输入",
        value: overall.total_input_tokens,
        pct: (overall.total_input_tokens / total) * 100,
        color: "#0ea5e9",
      },
      {
        key: "output",
        label: "输出",
        value: overall.total_output_tokens,
        pct: (overall.total_output_tokens / total) * 100,
        color: "#94a3b8",
      },
      {
        key: "cache",
        label: "缓存",
        value:
          overall.total_cache_read_tokens + overall.total_cache_write_tokens,
        pct:
          ((overall.total_cache_read_tokens +
            overall.total_cache_write_tokens) /
            total) *
          100,
        color: "#c084fc",
      },
    ];
  }, [overall]);

  const cacheHit = overall.weighted_cache_hit_ratio;

  return (
    <div className="grid grid-cols-2 sm:grid-cols-4 gap-2">
      <KpiCard label="调用" value={formatCalls(overall.total_calls)} />
      <KpiCard
        label="Token"
        value={formatNumber(overall.total_tokens)}
        accent
        composition={
          <div className="mt-1.5">
            <div className="flex h-1 w-full rounded-full overflow-hidden bg-slate-100">
              {tokenSegments.map((seg) => (
                <div
                  key={seg.key}
                  className="h-full"
                  style={{ width: `${seg.pct}%`, background: seg.color }}
                  title={`${seg.label}: ${formatNumber(seg.value)} (${seg.pct.toFixed(1)}%)`}
                />
              ))}
            </div>
            <div className="mt-0.5 flex justify-between text-[9px] text-slate-400">
              {tokenSegments.map((seg) => (
                <span key={seg.key} className="inline-flex items-center gap-0.5">
                  <span
                    className="w-1.5 h-1.5 rounded-full inline-block"
                    style={{ background: seg.color }}
                  />
                  {seg.label}
                </span>
              ))}
            </div>
          </div>
        }
      />
      <KpiCard
        label="命中率"
        value={formatPercent(cacheHit)}
        badge={cacheHit > 50 ? "cached" : undefined}
      />
      <KpiCard label="费用" value={formatCost(overall.total_cost)} />
    </div>
  );
}

function KpiCard({
  label,
  value,
  accent,
  composition,
  badge,
}: {
  label: string;
  value: string;
  accent?: boolean;
  composition?: React.ReactNode;
  badge?: string;
}) {
  return (
    <div className="bg-white px-4 py-3 rounded-lg border border-slate-200 min-w-0">
      <p className="text-[11px] font-medium text-slate-400">{label}</p>
      <div className="flex items-baseline gap-1.5">
        <p
          className={`text-lg font-bold leading-tight tabular-nums truncate ${
            accent ? "text-primary-600" : "text-slate-800"
          }`}
        >
          {value}
        </p>
        {badge && (
          <span className="px-1.5 py-0 rounded-full text-[9px] font-medium bg-emerald-100 text-emerald-700">
            {badge}
          </span>
        )}
      </div>
      {composition}
    </div>
  );
}
