import { useState } from "react";
import {
  makeAppliedRange,
  makeCustomAppliedRange,
  type TimePreset,
} from "../lib/timeRange";
import type { AppliedRange } from "../lib/filterState";

interface SidebarTimeRangeProps {
  activePreset: TimePreset;
  onChange: (preset: TimePreset, range: AppliedRange) => void;
}

const CUSTOM_PRESETS: { key: Exclude<TimePreset, "custom">; label: string }[] = [
  { key: "6h", label: "6h" },
  { key: "12h", label: "12h" },
  { key: "1d", label: "1d" },
  { key: "3d", label: "3d" },
  { key: "14d", label: "14d" },
  { key: "30d", label: "30d" },
  { key: "all", label: "all" },
];

const PRIMARY_PRESETS: { key: Exclude<TimePreset, "custom">; label: string }[] = [
  { key: "today", label: "今日" },
  { key: "7d", label: "最近 7 天" },
  { key: "all", label: "所有" },
];

function presetButtonClass(active: boolean): string {
  return active
    ? "border-l-2 border-primary-600 bg-primary-50 text-primary-700 pl-2"
    : "border-l-2 border-transparent text-slate-600 hover:bg-slate-50 pl-2";
}

const SUB_DAY_PRESETS = new Set<TimePreset>(["6h", "12h", "1d", "3d", "14d", "30d", "custom"]);

export function SidebarTimeRange({
  activePreset,
  onChange,
}: SidebarTimeRangeProps) {
  const [showCustomPanel, setShowCustomPanel] = useState(false);
  const [customFrom, setCustomFrom] = useState("");
  const [customTo, setCustomTo] = useState("");

  const isCustomActive = SUB_DAY_PRESETS.has(activePreset);

  const applyPreset = (key: Exclude<TimePreset, "custom">) => {
    onChange(key, makeAppliedRange(key));
    setShowCustomPanel(false);
  };

  const applyCustom = () => {
    if (customFrom && customTo) {
      onChange("custom", makeCustomAppliedRange(customFrom, customTo));
      setShowCustomPanel(false);
    }
  };

  return (
    <div className="py-3 px-3">
      <p className="text-[10px] uppercase tracking-wider text-slate-400 font-semibold mb-1.5 px-2">
        时间范围
      </p>
      <div className="space-y-0.5">
        {PRIMARY_PRESETS.map((p) => (
          <button
            key={p.key}
            onClick={() => applyPreset(p.key)}
            className={`block w-full text-left py-1.5 text-xs font-medium rounded-r transition-colors ${presetButtonClass(
              activePreset === p.key
            )}`}
          >
            {p.label}
          </button>
        ))}
        <button
          onClick={() => setShowCustomPanel((v) => !v)}
          className={`block w-full text-left py-1.5 text-xs font-medium rounded-r transition-colors ${presetButtonClass(
            isCustomActive
          )}`}
        >
          自定义...
        </button>
      </div>

      {showCustomPanel && (
        <div className="mt-2 p-2 rounded-lg bg-slate-50 border border-slate-200 space-y-2">
          <div className="flex flex-wrap gap-1">
            {CUSTOM_PRESETS.map((p) => (
              <button
                key={p.key}
                onClick={() => applyPreset(p.key)}
                className={`px-1.5 py-0.5 text-[10px] font-medium rounded transition-colors ${
                  activePreset === p.key
                    ? "bg-primary-100 text-primary-700"
                    : "bg-white text-slate-500 hover:bg-slate-100 border border-slate-200"
                }`}
              >
                {p.label}
              </button>
            ))}
          </div>
          <div className="space-y-1">
            <div>
              <label className="block text-[10px] text-slate-400 mb-0.5">从</label>
              <input
                type="datetime-local"
                value={customFrom}
                onChange={(e) => setCustomFrom(e.target.value)}
                className="w-full px-1.5 py-1 text-[11px] border border-slate-200 rounded focus:ring-1 focus:ring-primary-500 focus:border-primary-500 outline-none bg-white"
              />
            </div>
            <div>
              <label className="block text-[10px] text-slate-400 mb-0.5">至</label>
              <input
                type="datetime-local"
                value={customTo}
                onChange={(e) => setCustomTo(e.target.value)}
                className="w-full px-1.5 py-1 text-[11px] border border-slate-200 rounded focus:ring-1 focus:ring-primary-500 focus:border-primary-500 outline-none bg-white"
              />
            </div>
          </div>
          <div className="flex justify-end gap-1">
            <button
              onClick={() => setShowCustomPanel(false)}
              className="px-2 py-0.5 text-[10px] font-medium rounded text-slate-500 hover:bg-white transition-colors"
            >
              取消
            </button>
            <button
              onClick={applyCustom}
              disabled={!customFrom || !customTo}
              className="px-2 py-0.5 text-[10px] font-medium rounded bg-primary-600 text-white hover:bg-primary-700 disabled:opacity-40 disabled:cursor-not-allowed transition-colors"
            >
              应用
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
