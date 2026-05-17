import { formatNumber, formatCost, formatPercent, getSourceColor, getSourceLabel } from "../lib/utils";
import type { MergedTableRow } from "../data/transforms";
import ZH from "../i18n/zh";

export function VendorModelTable({ data }: { data: MergedTableRow[] }) {
  return (
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
            {data.map((row, idx) =>
              row.type === "vendor" ? (
                <tr
                  key={`vendor-${row.data.provider}`}
                  className="bg-slate-50/80 transition-colors"
                >
                  <td className="px-3 py-2">
                    <div className="flex items-center gap-1.5">
                      <span
                        className="w-2 h-2 rounded-full"
                        style={{
                          background: getSourceColor(row.data.provider),
                        }}
                      />
                      <span className="font-bold text-slate-800">
                        {row.data.provider}
                      </span>
                    </div>
                  </td>
                  <td className="px-3 py-2 text-[10px] text-slate-400 italic">
                    汇总
                  </td>
                  <td className="px-3 py-2"></td>
                  <td className="px-3 py-2 text-right font-semibold text-slate-700">
                    {formatNumber(row.data.calls)}
                  </td>
                  <td className="px-3 py-2 text-right text-slate-600">
                    {formatNumber(row.data.input_tokens)}
                  </td>
                  <td className="px-3 py-2 text-right text-slate-600">
                    {formatNumber(row.data.output_tokens)}
                  </td>
                  <td className="px-3 py-2 text-right text-slate-600">
                    {formatNumber(
                      row.data.cache_read_tokens + row.data.cache_write_tokens
                    )}
                  </td>
                  <td className="px-3 py-2 text-right font-bold text-slate-800">
                    {formatNumber(row.data.total_tokens)}
                  </td>
                  <td className="px-3 py-2 text-right">
                    <span
                      className={`px-1.5 py-0.5 rounded-full text-[10px] font-medium ${
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
                  <td className="px-3 py-2 text-right font-semibold text-slate-700">
                    {formatCost(row.data.cost)}
                  </td>
                </tr>
              ) : (
                <tr
                  key={`model-${row.data.provider}-${row.data.model}-${idx}`}
                  className="hover:bg-slate-50 transition-colors"
                >
                  <td className="px-3 py-2"></td>
                  <td className="px-3 py-2 font-medium text-slate-700">
                    {row.data.model}
                  </td>
                  <td className="px-3 py-2">
                    <div className="flex flex-wrap gap-0.5">
                      {(row.data.sources || []).map((s) => (
                        <span
                          key={s}
                          className="inline-flex items-center gap-0.5 px-1 py-0.5 rounded text-[9px] font-medium"
                          style={{
                            background: `${getSourceColor(s)}15`,
                            color: getSourceColor(s),
                          }}
                        >
                          {getSourceLabel(s)}
                        </span>
                      ))}
                    </div>
                  </td>
                  <td className="px-3 py-2 text-right text-slate-600">
                    {formatNumber(row.data.calls)}
                  </td>
                  <td className="px-3 py-2 text-right text-slate-600">
                    {formatNumber(row.data.input_tokens)}
                  </td>
                  <td className="px-3 py-2 text-right text-slate-600">
                    {formatNumber(row.data.output_tokens)}
                  </td>
                  <td className="px-3 py-2 text-right text-slate-600">
                    {formatNumber(
                      row.data.cache_read_tokens + row.data.cache_write_tokens
                    )}
                  </td>
                  <td className="px-3 py-2 text-right font-semibold text-slate-700">
                    {formatNumber(row.data.total_tokens)}
                  </td>
                  <td className="px-3 py-2 text-right">
                    <span
                      className={`px-1.5 py-0.5 rounded-full text-[10px] font-medium ${
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
                  <td className="px-3 py-2 text-right text-slate-600">
                    {formatCost(row.data.cost)}
                  </td>
                </tr>
              )
            )}
          </tbody>
        </table>
      </div>
    </div>
  );
}
