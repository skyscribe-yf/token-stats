import { memo, useState, useMemo, useCallback } from "react";
import {
  LineChart,
  Line,
  XAxis,
  YAxis,
  CartesianGrid,
  Tooltip,
  ResponsiveContainer,
  Legend,
} from "recharts";
import { X } from "lucide-react";
import { getVendorLabel, getVendorModelLabel } from "../lib/utils";
import type { TpsAnalysis } from "../api";

// Generate distinct colors for models
const MODEL_COLORS = [
  "#6366f1", // indigo
  "#f43f5e", // rose
  "#10b981", // emerald
  "#f59e0b", // amber
  "#3b82f6", // blue
  "#8b5cf6", // violet
  "#ec4899", // pink
  "#14b8a6", // teal
  "#f97316", // orange
  "#06b6d4", // cyan
  "#84cc16", // lime
  "#a855f7", // purple
];

function getModelColor(index: number): string {
  return MODEL_COLORS[index % MODEL_COLORS.length];
}

interface TpsChartProps {
  tpsData: TpsAnalysis | null;
  loading: boolean;
}

export const TpsChart = memo(function TpsChart({ tpsData, loading }: TpsChartProps) {
  const [selectedModels, setSelectedModels] = useState<Set<string>>(new Set());
  const [searchQuery, setSearchQuery] = useState("");

  // Initialize all models as selected when data loads
  const allModels = useMemo(() => {
    if (!tpsData) return [];
    return tpsData.available_models;
  }, [tpsData]);

  // If no selection, show all
  const effectiveSelection = useMemo(() => {
    if (selectedModels.size === 0) {
      return new Set(allModels);
    }
    return selectedModels;
  }, [selectedModels, allModels]);

  // Filter models by search
  const filteredModels = useMemo(() => {
    if (!searchQuery) return allModels;
    const q = searchQuery.toLowerCase();
    return allModels.filter((m) => m.toLowerCase().includes(q));
  }, [allModels, searchQuery]);

  // Build chart data: merge all model series into a unified time axis
  const { chartData, modelColorMap } = useMemo(() => {
    if (!tpsData || effectiveSelection.size === 0) {
      return { chartData: [], modelColorMap: new Map<string, string>() };
    }

    // Collect all time points and build a map
    const colorMap = new Map<string, string>();
    let colorIdx = 0;

    const selectedSeries = tpsData.models.filter((s) =>
      effectiveSelection.has(`${s.provider}/${s.model}`)
    );
    for (const series of selectedSeries) {
      const modelKey = `${series.provider}/${series.model}`;
      colorMap.set(modelKey, getModelColor(colorIdx++));
    }

    // Gather the sorted unique time axis across all selected series.
    const timesSet = new Set<string>();
    for (const series of selectedSeries) {
      for (const dp of series.data_points) timesSet.add(dp.time);
    }
    const sortedTimes = Array.from(timesSet).sort();

    // Cap the rendered time axis so huge ranges (e.g. "all") don't draw
    // thousands of points per model and freeze the UI. When the axis is
    // long we bucket consecutive times and average each model's TPS
    // within a bucket, keeping the chart meaningful at a glance.
    const MAX_POINTS = 600;
    let chartData: Record<string, unknown>[];

    if (sortedTimes.length <= MAX_POINTS) {
      const timeMap = new Map<string, Record<string, unknown>>();
      for (const series of selectedSeries) {
        const modelKey = `${series.provider}/${series.model}`;
        for (const dp of series.data_points) {
          if (!timeMap.has(dp.time)) timeMap.set(dp.time, { time: dp.time });
          timeMap.get(dp.time)![modelKey] = dp.tps;
        }
      }
      chartData = sortedTimes.map((t) => timeMap.get(t)!);
    } else {
      const binSize = Math.ceil(sortedTimes.length / MAX_POINTS);
      const binCount = Math.ceil(sortedTimes.length / binSize);
      const bins: Record<string, unknown>[] = Array.from(
        { length: binCount },
        (_, i) => ({ time: sortedTimes[i * binSize] })
      );
      const accums: Map<string, { sum: number; cnt: number }>[] =
        bins.map(() => new Map());
      const timeToBin = new Map<string, number>();
      sortedTimes.forEach((t, idx) =>
        timeToBin.set(t, Math.floor(idx / binSize))
      );
      for (const series of selectedSeries) {
        const modelKey = `${series.provider}/${series.model}`;
        for (const dp of series.data_points) {
          const b = timeToBin.get(dp.time)!;
          const a = accums[b].get(modelKey);
          if (a) {
            a.sum += dp.tps;
            a.cnt += 1;
          } else {
            accums[b].set(modelKey, { sum: dp.tps, cnt: 1 });
          }
        }
      }
      chartData = bins.map((row, b) => {
        const out: Record<string, unknown> = { time: row.time };
        for (const [modelKey, a] of accums[b]) {
          out[modelKey] = a.sum / a.cnt;
        }
        return out;
      });
    }

    return { chartData, modelColorMap: colorMap };
  }, [tpsData, effectiveSelection]);

  const toggleModel = useCallback(
    (model: string) => {
      setSelectedModels((prev) => {
        const next = new Set(prev);
        if (next.has(model)) {
          next.delete(model);
        } else {
          next.add(model);
        }
        // If all selected, clear to use default
        if (next.size === allModels.length) {
          return new Set();
        }
        return next;
      });
    },
    [allModels]
  );

  const selectAll = useCallback(() => {
    setSelectedModels(new Set());
  }, []);

  const clearAll = useCallback(() => {
    setSelectedModels(new Set(["__none__"]));
  }, []);

  if (loading) {
    return (
      <div className="bg-white rounded-xl border border-slate-200 p-5 shadow-sm">
        <div className="flex items-center justify-center h-64 text-slate-400 text-sm">
          加载 TPS 数据中...
        </div>
      </div>
    );
  }

  if (!tpsData || tpsData.models.length === 0) {
    return (
      <div className="bg-white rounded-xl border border-slate-200 p-5 shadow-sm">
        <h3 className="text-sm font-semibold text-slate-700 mb-2">
          TPS 趋势 (5分钟窗口活跃期)
        </h3>
        <div className="flex items-center justify-center h-48 text-slate-400 text-sm">
          暂无 TPS 数据
        </div>
      </div>
    );
  }

  return (
    <div className="bg-white rounded-xl border border-slate-200 p-5 shadow-sm">
      <div className="flex items-center justify-between mb-3">
        <h3 className="text-sm font-semibold text-slate-700">
          TPS 趋势 (5分钟窗口活跃期)
        </h3>
        <div className="text-[10px] text-slate-400">
          {chartData.length} 个数据点 · {modelColorMap.size} 个模型
        </div>
      </div>

      {/* Model filter chips */}
      <div className="mb-3">
        <div className="flex items-center gap-2 mb-2">
          <input
            type="text"
            placeholder="搜索模型..."
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
            className="px-2 py-1 text-[11px] border border-slate-200 rounded-md w-48 focus:outline-none focus:ring-1 focus:ring-primary-400"
          />
          <button
            onClick={selectAll}
            className="px-2 py-1 text-[10px] text-slate-500 hover:text-slate-700 hover:bg-slate-100 rounded transition-colors"
          >
            全选
          </button>
          <button
            onClick={clearAll}
            className="px-2 py-1 text-[10px] text-slate-500 hover:text-slate-700 hover:bg-slate-100 rounded transition-colors"
          >
            清除
          </button>
        </div>
        <div className="flex flex-wrap gap-1.5">
          {filteredModels.map((model) => {
            const isSelected = effectiveSelection.has(model);
            const color = modelColorMap.get(model) ?? "#94a3b8";
            return (
              <button
                key={model}
                onClick={() => toggleModel(model)}
                className={`inline-flex items-center gap-1 px-2 py-0.5 rounded-full text-[10px] font-medium transition-all ${
                  isSelected
                    ? "shadow-sm"
                    : "opacity-40 hover:opacity-70"
                }`}
                style={{
                  background: isSelected ? `${color}18` : "#f1f5f9",
                  color: isSelected ? color : "#64748b",
                  borderWidth: "1px",
                  borderColor: isSelected ? `${color}40` : "#e2e8f0",
                }}
              >
                <span
                  className="w-2 h-2 rounded-full"
                  style={{ background: isSelected ? color : "#94a3b8" }}
                />
                {getVendorModelLabel(model)}
                {isSelected && (
                  <X className="w-2.5 h-2.5 ml-0.5 opacity-60" />
                )}
              </button>
            );
          })}
        </div>
      </div>

      {/* Chart */}
      <ResponsiveContainer width="100%" height={320}>
        <LineChart data={chartData}>
          <CartesianGrid strokeDasharray="2 2" stroke="#f1f5f9" />
          <XAxis
            dataKey="time"
            tick={{ fontSize: 9, fill: "#64748b" }}
            angle={-45}
            textAnchor="end"
            height={60}
            interval={Math.max(0, Math.floor(chartData.length / 30) - 1)}
            tickFormatter={(v: string) => {
              // "2026-06-01 08:55" → "06-01 08:55"
              if (v.length >= 16) return v.substring(5);
              return v;
            }}
          />
          <YAxis
            tick={{ fontSize: 10, fill: "#64748b" }}
            width={45}
            label={{
              value: "TPS",
              angle: -90,
              position: "insideLeft",
              style: { fontSize: 10, fill: "#94a3b8" },
            }}
          />
          <Tooltip
            content={({ active, payload, label }) => {
              if (!active || !payload || payload.length === 0) return null;
              const filtered = payload.filter(
                (p) => !String(p.name).endsWith("__dotted")
              );
              if (filtered.length === 0) return null;
              const s = String(label);
              const displayLabel = s.length >= 16 ? s.substring(5) : s;
              return (
                <div
                  style={{
                    fontSize: 11,
                    borderRadius: 8,
                    border: "1px solid #e2e8f0",
                    boxShadow: "0 2px 8px rgba(0,0,0,0.08)",
                    background: "#fff",
                    padding: "8px 12px",
                  }}
                >
                  <div
                    style={{
                      fontWeight: 600,
                      marginBottom: 4,
                      color: "#334155",
                    }}
                  >
                    {displayLabel}
                  </div>
                  {filtered.map((entry, idx) => (
                    <div
                      key={idx}
                      style={{
                        display: "flex",
                        alignItems: "center",
                        gap: 6,
                        marginTop: 2,
                      }}
                    >
                      <span
                        style={{
                          width: 8,
                          height: 8,
                          borderRadius: "50%",
                          background: entry.color,
                          display: "inline-block",
                        }}
                      />
                      <span style={{ color: "#64748b" }}>
                        {(() => { const parts = String(entry.name).split("/"); return getVendorLabel(parts[0]) + " / " + parts.slice(1).join("/"); })()}:
                      </span>
                      <span style={{ fontWeight: 500, color: "#334155" }}>
                        {Number(entry.value).toFixed(1)} TPS
                      </span>
                    </div>
                  ))}
                </div>
              );
            }}
          />
          <Legend
            wrapperStyle={{ fontSize: 10 }}
            formatter={(value: string) => getVendorModelLabel(value)}
          />
          {Array.from(modelColorMap.entries()).map(([model, color]) => (
            <>
              {/* Dotted background line: connects across gaps */}
              <Line
                key={`${model}__dotted`}
                type="monotone"
                dataKey={model}
                name={`${model}__dotted`}
                stroke={color}
                strokeWidth={1.5}
                strokeDasharray="3 3"
                strokeOpacity={0.5}
                dot={false}
                connectNulls={true}
                legendType="none"
                isAnimationActive={false}
              />
              {/* Solid foreground line: actual data segments */}
              <Line
                key={model}
                type="monotone"
                dataKey={model}
                name={model}
                stroke={color}
                strokeWidth={1.5}
                dot={false}
                connectNulls={false}
                isAnimationActive={false}
              />
            </>
          ))}
        </LineChart>
      </ResponsiveContainer>
    </div>
  );
});
