import { QuotaCard } from "./QuotaCard";
import type { QuotaResponse } from "../api";

export function QuotaOverview({
  quota,
  quotaLoading,
}: {
  quota: QuotaResponse | null;
  quotaLoading: boolean;
}) {
  if (!quota && !quotaLoading) return null;

  return (
    <details className="mb-3 group">
      <summary className="bg-white rounded-xl border border-slate-200 shadow-sm px-4 py-2.5 cursor-pointer select-none flex items-center gap-2 text-xs font-semibold text-slate-700 hover:bg-slate-50 transition-colors list-none">
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
        配额概览
        <span className="text-[11px] text-slate-400 font-normal ml-1">
          {!quotaLoading && quota?.kimi?.available && quota.kimi.data
            ? `Kimi: ¥${quota.kimi.data.available_balance.toFixed(2)}`
            : !quotaLoading && quota?.opencode_go?.available && quota.opencode_go.data
              ? `OpenCode: ${quota.opencode_go.data.usage_percent?.toFixed(0) ?? "?"}%已用`
              : quotaLoading
                ? "加载中..."
                : ""}
        </span>
      </summary>
      <div className="mt-1.5 grid grid-cols-1 sm:grid-cols-2 gap-3">
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
                      已用 $
                      {quota.opencode_go.data.total_usage_usd?.toFixed(2) ||
                        "?"}
                    </span>
                    <span>
                      {quota.opencode_go.data.usage_percent.toFixed(1)}%
                    </span>
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
                      style={{
                        width: `${Math.min(quota.opencode_go.data.usage_percent, 100)}%`,
                      }}
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
    </details>
  );
}
