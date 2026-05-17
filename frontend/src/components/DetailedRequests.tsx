import {
  ChevronLeft,
  ChevronRight,
  Filter,
} from "lucide-react";
import {
  formatNumber,
  formatCost,
  formatPercent,
  formatTime,
  getSourceColor,
  getSourceLabel,
} from "../lib/utils";
import type { PaginatedRequests } from "../api";
import ZH from "../i18n/zh";

interface DetailedRequestsProps {
  requests: PaginatedRequests | null;
  models: string[];
  selectedModel: string;
  onModelChange: (model: string) => void;
  page: number;
  onPageChange: (page: number) => void;
}

export function DetailedRequests({
  requests,
  models,
  selectedModel,
  onModelChange,
  page,
  onPageChange,
}: DetailedRequestsProps) {
  const handleModelSelect = (e: React.ChangeEvent<HTMLSelectElement>) => {
    onModelChange(e.target.value);
  };

  return (
    <details className="group">
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
        {ZH.detailedRequests}
        <span className="text-[11px] text-slate-400 font-normal ml-1">
          {requests ? `${formatNumber(requests.total)} 条` : ""}
        </span>
        <div
          className="ml-auto flex items-center gap-2"
          onClick={(e) => e.stopPropagation()}
        >
          <Filter className="w-3.5 h-3.5 text-slate-400" />
          <select
            value={selectedModel}
            onChange={handleModelSelect}
            className="px-2 py-1 text-xs border border-slate-200 rounded focus:ring-1 focus:ring-primary-500 outline-none bg-white"
          >
            <option value="">{ZH.allModels}</option>
            {models.map((m) => (
              <option key={m} value={m}>
                {m}
              </option>
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
                <td className="px-3 py-2 text-slate-600">{r.model}</td>
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
                <td className="px-3 py-2 text-right text-slate-600">
                  {formatCost(r.cost, r.source)}
                </td>
              </tr>
            ))}
          </tbody>
        </table>

        {/* Pagination */}
        {requests && requests.total_pages > 1 && (
          <div className="px-3 py-2 border-t border-slate-100 flex items-center justify-between">
            <p className="text-xs text-slate-500">
              {ZH.showing} {(requests.page - 1) * requests.limit + 1}-
              {Math.min(
                requests.page * requests.limit,
                requests.total
              )}{" "}
              {ZH.of} {formatNumber(requests.total)} {ZH.requests}
            </p>
            <div className="flex items-center gap-1.5">
              <button
                onClick={() => onPageChange(Math.max(1, page - 1))}
                disabled={page <= 1}
                className="p-1 rounded hover:bg-slate-100 disabled:opacity-40 disabled:cursor-not-allowed transition-colors"
              >
                <ChevronLeft className="w-3.5 h-3.5" />
              </button>
              <span className="text-[11px] text-slate-500 mx-1">
                {page} / {requests.total_pages}
              </span>
              <button
                onClick={() =>
                  onPageChange(
                    Math.min(requests.total_pages, page + 1)
                  )
                }
                disabled={page >= requests.total_pages}
                className="p-1 rounded hover:bg-slate-100 disabled:opacity-40 disabled:cursor-not-allowed transition-colors"
              >
                <ChevronRight className="w-3.5 h-3.5" />
              </button>
            </div>
          </div>
        )}
      </div>
    </details>
  );
}
