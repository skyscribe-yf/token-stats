import {
  LineChart,
  Line,
  XAxis,
  YAxis,
  CartesianGrid,
  Tooltip,
  Legend,
  ResponsiveContainer,
} from "recharts";
import { formatNumber } from "../lib/utils";
import { CustomTooltip } from "./CustomTooltip";
import type { ChartDataPoint } from "../data/transforms";
import ZH from "../i18n/zh";

export function DailyTrendChart({ data }: { data: ChartDataPoint[] }) {
  return (
    <div className="bg-white rounded-xl border border-slate-200 p-4 shadow-sm">
      <h3 className="text-xs font-semibold text-slate-700 mb-2">
        {ZH.dailyTokenUsage} & {ZH.cacheHitTrend}
      </h3>
      <ResponsiveContainer width="100%" height={240}>
        <LineChart data={data}>
          <CartesianGrid strokeDasharray="3 3" stroke="#e2e8f0" />
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
          <YAxis
            yAxisId="ratio"
            orientation="right"
            tick={{ fontSize: 10, fill: "#f43f5e" }}
            domain={[0, 100]}
            unit="%"
            width={40}
          />
          <Tooltip content={<CustomTooltip />} />
          <Legend wrapperStyle={{ fontSize: 11 }} />
          <Line
            yAxisId="tokens"
            type="monotone"
            dataKey="input"
            name={ZH.inputLabel}
            stroke="#10b981"
            strokeWidth={2}
            dot={false}
          />
          <Line
            yAxisId="tokens"
            type="monotone"
            dataKey="output"
            name={ZH.outputLabel}
            stroke="#f59e0b"
            strokeWidth={2}
            dot={false}
          />
          <Line
            yAxisId="tokens"
            type="monotone"
            dataKey="cacheRead"
            name={ZH.cacheReadLabel}
            stroke="#8b5cf6"
            strokeWidth={2}
            dot={false}
          />
          <Line
            yAxisId="ratio"
            type="monotone"
            dataKey="cacheHitRatio"
            name={ZH.cacheHitLabel}
            stroke="#f43f5e"
            strokeWidth={2}
            strokeDasharray="6 3"
            dot={{ r: 2 }}
          />
        </LineChart>
      </ResponsiveContainer>
    </div>
  );
}
