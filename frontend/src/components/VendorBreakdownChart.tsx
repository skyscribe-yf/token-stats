import {
  BarChart,
  Bar,
  XAxis,
  YAxis,
  CartesianGrid,
  Tooltip,
  ResponsiveContainer,
  PieChart,
  Pie,
  Cell,
} from "recharts";
import { formatNumber } from "../lib/utils";
import { CustomTooltip, PieTooltip } from "./CustomTooltip";
import type { VendorChartDatum, PieDatum } from "../data/transforms";
import ZH from "../i18n/zh";

export function VendorBreakdownChart({
  vendorData,
  pieData,
}: {
  vendorData: VendorChartDatum[];
  pieData: PieDatum[];
}) {
  return (
    <div className="bg-white rounded-xl border border-slate-200 p-4 shadow-sm">
      <h3 className="text-xs font-semibold text-slate-700 mb-2">
        {ZH.vendorBreakdown} & {ZH.tokenDistribution}
      </h3>
      <div className="flex items-center gap-2">
        <div className="flex-1">
          <ResponsiveContainer width="100%" height={220}>
            <BarChart data={vendorData} layout="vertical">
              <CartesianGrid strokeDasharray="3 3" stroke="#e2e8f0" />
              <XAxis
                type="number"
                tick={{ fontSize: 10, fill: "#64748b" }}
                tickFormatter={(v: number) => formatNumber(v)}
              />
              <YAxis
                type="category"
                dataKey="name"
                tick={{ fontSize: 10, fill: "#64748b" }}
                width={80}
              />
              <Tooltip content={<CustomTooltip />} />
              <Bar
                dataKey="tokens"
                name={ZH.totalTokensLabel}
                radius={[0, 4, 4, 0]}
              >
                {vendorData.map((_, i) => (
                  <Cell key={i} />
                ))}
              </Bar>
            </BarChart>
          </ResponsiveContainer>
        </div>
        <div className="w-[140px] shrink-0">
          <ResponsiveContainer width="100%" height={220}>
            <PieChart>
              <Pie
                data={pieData}
                cx="50%"
                cy="50%"
                innerRadius={40}
                outerRadius={65}
                paddingAngle={3}
                dataKey="value"
              >
                {pieData.map((_, i) => (
                  <Cell key={i} />
                ))}
              </Pie>
              <Tooltip content={<PieTooltip />} />
            </PieChart>
          </ResponsiveContainer>
        </div>
      </div>
    </div>
  );
}
