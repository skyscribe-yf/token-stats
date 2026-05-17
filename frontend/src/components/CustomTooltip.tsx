import { formatNumber } from "../lib/utils";
import type { ChartTooltipPayload } from "../types/charts";
import ZH from "../i18n/zh";

export function CustomTooltip({
  active,
  payload,
  label,
}: {
  active?: boolean;
  payload?: ChartTooltipPayload[];
  label?: string;
}) {
  if (!active || !payload?.length) return null;
  return (
    <div className="bg-white border border-slate-200 rounded-lg shadow-lg p-3 text-sm">
      {label && <p className="font-semibold text-slate-700 mb-1">{label}</p>}
      {payload.map((p, i) => (
        <p key={i} className="text-slate-600">
          <span
            className="inline-block w-2 h-2 rounded-full mr-1.5"
            style={{ background: p.color }}
          />
          {p.name}: {formatNumber(Number(p.value ?? 0))}
        </p>
      ))}
    </div>
  );
}

export function PieTooltip({
  active,
  payload,
}: {
  active?: boolean;
  payload?: ChartTooltipPayload[];
}) {
  if (!active || !payload?.length) return null;
  const p = payload[0];
  return (
    <div className="bg-white border border-slate-200 rounded-lg shadow-lg p-3 text-sm">
      <p className="font-semibold text-slate-700">{p.name}</p>
      <p className="text-slate-600">
        {formatNumber(Number(p.value ?? 0))} {ZH.tokens}
      </p>
      <p className="text-slate-500">{p.percent?.toFixed(1)}%</p>
    </div>
  );
}
