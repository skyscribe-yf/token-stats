import { useState, useMemo, useCallback } from "react";
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

export function TpsChart({ tpsData, loading }: TpsChartProps) {
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
    const timeMap = new Map<string, Record<string, unknown>>();
    const colorMap = new Map<string, string>();
    let colorIdx = 0;

    for (const series of tpsData.models) {
      const modelKey = `${series.provider}/${series.model}`;
      if (!effectiveSelection.has(modelKey)) continue;

      colorMap.set(modelKey, getModelColor(colorIdx++));

      for (const dp of series.data_points) {
        if (!timeMap.has(dp.time)) {
          timeMap.set(dp.time, { time: dp.time });
        }
        timeMap.get(dp.time)![modelKey] = dp.tps;
      }
    }

    // Sort by time and fill gaps with null
    const sorted = Array.from(timeMap.values()).sort((a, b) => {
      const at = typeof a.time === 'string' ? a.time : '';
      const bt = typeof b.time === 'string' ? b.time : '';
      return at.localeCompare(bt);
    });

    return { chartData: sorted, modelColorMap: colorMap };
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
  }, [selectedModels, allModels]);

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
          TPS 趋势 (5分钟滑动平均)
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
          TPS 趋势 (5分钟滑动平均)
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
                {model.split("/").pop()}
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
            contentStyle={{
              fontSize: 11,
              borderRadius: 8,
              border: "1px solid #e2e8f0",
              boxShadow: "0 2px 8px rgba(0,0,0,0.08)",
            }}
            labelFormatter={(v) => {
              const s = String(v);
              if (s.length >= 16) return s.substring(5);
              return s;
            }}
            formatter={(value, name) => [
              `${Number(value).toFixed(1)} TPS`,
              String(name).split("/").pop(),
            ]}
          />
          <Legend
            wrapperStyle={{ fontSize: 10 }}
            formatter={(value: string) => value.split("/").pop()}
          />
          {Array.from(modelColorMap.entries()).map(([model, color]) => (
            <Line
              key={model}
              type="monotone"
              dataKey={model}
              name={model}
              stroke={color}
              strokeWidth={1.5}
              dot={false}
              connectNulls={false}
              animationDuration={300}
            />
          ))}
        </LineChart>
      </ResponsiveContainer>
    </div>
  );
}
