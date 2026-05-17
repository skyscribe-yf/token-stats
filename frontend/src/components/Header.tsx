import { useState, useRef, useEffect } from "react";
import { Activity, X, SlidersHorizontal } from "lucide-react";
import {
  formatTime,
  getLocalToday,
  getLocalDateOffset,
  getLocalDatetimeOffsetHours,
  getSourceColor,
  getSourceLabel,
} from "../lib/utils";
import type { AppliedRange } from "../lib/filterState";
import ZH from "../i18n/zh";

type TimePreset =
  | "today"
  | "6h"
  | "12h"
  | "1d"
  | "3d"
  | "7d"
  | "14d"
  | "30d"
  | "all"
  | "custom";

function getPresetRange(
  preset: Exclude<TimePreset, "custom">
): Pick<AppliedRange, "from" | "to"> {
  switch (preset) {
    case "today": {
      const today = getLocalToday();
      return { from: today, to: today };
    }
    case "6h":
      return {
        from: getLocalDatetimeOffsetHours(6),
        to: getLocalDatetimeOffsetHours(0),
      };
    case "12h":
      return {
        from: getLocalDatetimeOffsetHours(12),
        to: getLocalDatetimeOffsetHours(0),
      };
    case "1d":
      return {
        from: getLocalDatetimeOffsetHours(24),
        to: getLocalDatetimeOffsetHours(0),
      };
    case "3d":
      return { from: getLocalDateOffset(3), to: getLocalToday() };
    case "7d":
      return { from: getLocalDateOffset(7), to: getLocalToday() };
    case "14d":
      return { from: getLocalDateOffset(14), to: getLocalToday() };
    case "30d":
      return { from: getLocalDateOffset(30), to: getLocalToday() };
    case "all":
      return { from: getLocalDateOffset(365 * 10), to: getLocalToday() };
  }
}

function makeAppliedRange(
  preset: Exclude<TimePreset, "custom">
): AppliedRange {
  return { ...getPresetRange(preset), appliedAt: Date.now() };
}

function toggleInSet<T>(
  set: Set<T>,
  setter: (s: Set<T>) => void,
  value: T
) {
  const next = new Set(set);
  if (next.has(value)) next.delete(value);
  else next.add(value);
  setter(next);
}

interface HeaderProps {
  activePreset: TimePreset;
  onPresetChange: (preset: TimePreset, range: AppliedRange) => void;
  sources: string[];
  selectedSources: Set<string>;
  onSourcesChange: (sources: Set<string>) => void;
  vendors: string[];
  selectedVendors: Set<string>;
  onVendorsChange: (vendors: Set<string>) => void;
  onFilterChange: () => void;
  loading: boolean;
  lastUpdatedAt: Date | null;
}

export function Header({
  activePreset,
  onPresetChange,
  sources,
  selectedSources,
  onSourcesChange,
  vendors,
  selectedVendors,
  onVendorsChange,
  onFilterChange,
  loading,
  lastUpdatedAt,
}: HeaderProps) {
  const [showCustomPanel, setShowCustomPanel] = useState(false);
  const [customFrom, setCustomFrom] = useState("");
  const [customTo, setCustomTo] = useState("");
  const customBtnRef = useRef<HTMLButtonElement>(null);

  // Close custom panel on outside click
  useEffect(() => {
    if (!showCustomPanel) return;
    const handler = (e: MouseEvent) => {
      const target = e.target as HTMLElement;
      if (
        !target.closest(".custom-time-panel") &&
        !target.closest(".custom-time-btn")
      ) {
        setShowCustomPanel(false);
      }
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [showCustomPanel]);

  const applyPreset = (key: Exclude<TimePreset, "custom">) => {
    onPresetChange(key, makeAppliedRange(key));
    setShowCustomPanel(false);
  };

  const applyCustom = () => {
    if (customFrom && customTo) {
      onPresetChange("custom", {
        from: customFrom,
        to: customTo,
        appliedAt: Date.now(),
      });
      setShowCustomPanel(false);
    }
  };

  const handleSourceToggle = (s: string) => {
    toggleInSet(selectedSources, onSourcesChange, s);
    onFilterChange();
  };

  const handleVendorToggle = (v: string) => {
    toggleInSet(selectedVendors, onVendorsChange, v);
    onFilterChange();
  };

  const quickSetCustom = (key: Exclude<TimePreset, "custom">) => {
    applyPreset(key);
  };

  const presetBtnClass = (key: string) =>
    `px-3 py-1.5 text-xs font-medium rounded-md transition-colors ${
      activePreset === key
        ? "bg-primary-600 text-white"
        : "bg-slate-100 text-slate-600 hover:bg-slate-200"
    }`;

  const customQuickClass = (key: string) =>
    `px-2 py-1 text-[11px] font-medium rounded transition-colors ${
      activePreset === key
        ? "bg-primary-100 text-primary-700"
        : "bg-slate-50 text-slate-500 hover:bg-slate-100"
    }`;

  const isCustomActive = [
    "6h",
    "12h",
    "1d",
    "3d",
    "14d",
    "30d",
    "all",
    "custom",
  ].includes(activePreset);

  return (
    <header className="bg-white border-b border-slate-200 sticky top-0 z-20">
      <div className="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8 py-2">
        <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
          {/* Logo + Title */}
          <div className="flex items-center gap-2 shrink-0">
            <div className="bg-primary-600 p-1.5 rounded-lg">
              <Activity className="w-4 h-4 text-white" />
            </div>
            <div>
              <h1 className="text-sm font-bold text-slate-800 leading-tight">
                {ZH.title}
              </h1>
            </div>
          </div>

          {/* Divider */}
          <div className="hidden sm:block w-px h-5 bg-slate-200" />

          {/* Time presets */}
          <div className="flex items-center gap-1.5">
            <button
              onClick={() => applyPreset("today")}
              className={presetBtnClass("today")}
            >
              {ZH.today}
            </button>
            <button
              onClick={() => applyPreset("7d")}
              className={presetBtnClass("7d")}
            >
              {ZH.last7Days}
            </button>
            <div className="relative">
              <button
                ref={customBtnRef}
                onClick={() => setShowCustomPanel((v) => !v)}
                className={`custom-time-btn inline-flex items-center gap-1 px-3 py-1.5 text-xs font-medium rounded-md transition-colors ${
                  isCustomActive
                    ? "bg-primary-600 text-white"
                    : "bg-slate-100 text-slate-600 hover:bg-slate-200"
                }`}
              >
                <SlidersHorizontal className="w-3 h-3" />
                {ZH.customTime}
              </button>

              {/* Custom time panel */}
              {showCustomPanel && (
                <div className="custom-time-panel absolute left-0 top-full mt-1.5 bg-white border border-slate-200 rounded-lg shadow-xl p-3 min-w-[320px] z-30">
                  <div className="flex items-center justify-between mb-2">
                    <span className="text-xs font-semibold text-slate-700">
                      {ZH.quickSelect}
                    </span>
                    <button
                      onClick={() => setShowCustomPanel(false)}
                      className="p-0.5 rounded hover:bg-slate-100 text-slate-400"
                    >
                      <X className="w-3.5 h-3.5" />
                    </button>
                  </div>
                  <div className="flex flex-wrap gap-1 mb-3">
                    {(
                      [
                        { k: "6h", l: ZH.last6h },
                        { k: "12h", l: ZH.last12h },
                        { k: "1d", l: ZH.last1d },
                        { k: "3d", l: ZH.last3d },
                        { k: "14d", l: ZH.last14d },
                        { k: "30d", l: ZH.last30d },
                        { k: "all", l: ZH.allTime },
                      ] as const
                    ).map((q) => (
                      <button
                        key={q.k}
                        onClick={() =>
                          quickSetCustom(
                            q.k as Exclude<TimePreset, "custom">
                          )
                        }
                        className={customQuickClass(q.k)}
                      >
                        {q.l}
                      </button>
                    ))}
                  </div>
                  <div className="flex items-center gap-2 mb-3">
                    <div className="flex-1">
                      <label className="block text-[10px] text-slate-400 mb-0.5">
                        {ZH.from}
                      </label>
                      <input
                        type="datetime-local"
                        value={customFrom}
                        onChange={(e) => setCustomFrom(e.target.value)}
                        className="w-full px-2 py-1 text-xs border border-slate-200 rounded focus:ring-1 focus:ring-primary-500 focus:border-primary-500 outline-none"
                      />
                    </div>
                    <span className="text-slate-300 mt-4">-</span>
                    <div className="flex-1">
                      <label className="block text-[10px] text-slate-400 mb-0.5">
                        {ZH.to}
                      </label>
                      <input
                        type="datetime-local"
                        value={customTo}
                        onChange={(e) => setCustomTo(e.target.value)}
                        className="w-full px-2 py-1 text-xs border border-slate-200 rounded focus:ring-1 focus:ring-primary-500 focus:border-primary-500 outline-none"
                      />
                    </div>
                  </div>
                  <div className="flex justify-end gap-1.5">
                    <button
                      onClick={() => setShowCustomPanel(false)}
                      className="px-2.5 py-1 text-[11px] font-medium rounded text-slate-500 hover:bg-slate-100 transition-colors"
                    >
                      {ZH.cancel}
                    </button>
                    <button
                      onClick={applyCustom}
                      disabled={!customFrom || !customTo}
                      className="px-2.5 py-1 text-[11px] font-medium rounded bg-primary-600 text-white hover:bg-primary-700 disabled:opacity-40 disabled:cursor-not-allowed transition-colors"
                    >
                      {ZH.apply}
                    </button>
                  </div>
                </div>
              )}
            </div>
          </div>

          {/* Source filter tags */}
          <div className="flex items-center gap-1 flex-wrap">
            {sources.map((s) => (
              <button
                key={s}
                onClick={() => handleSourceToggle(s)}
                className={`inline-flex items-center gap-1 px-2 py-0.5 text-[11px] font-medium rounded-full transition-all border ${
                  selectedSources.has(s)
                    ? "text-white border-transparent shadow-sm"
                    : "bg-white text-slate-500 border-slate-200 hover:border-slate-300"
                }`}
                style={
                  selectedSources.has(s)
                    ? { background: getSourceColor(s) }
                    : undefined
                }
              >
                <span
                  className="w-1.5 h-1.5 rounded-full"
                  style={{
                    background: selectedSources.has(s)
                      ? "white"
                      : getSourceColor(s),
                  }}
                />
                {getSourceLabel(s)}
              </button>
            ))}
          </div>

          {/* Vendor filter tags */}
          <div className="flex items-center gap-1 flex-wrap">
            {vendors.map((v) => (
              <button
                key={v}
                onClick={() => handleVendorToggle(v)}
                className={`inline-flex items-center px-2 py-0.5 text-[11px] font-medium rounded-full transition-all border ${
                  selectedVendors.has(v)
                    ? "bg-primary-600 text-white border-transparent shadow-sm"
                    : "bg-white text-slate-500 border-slate-200 hover:border-slate-300"
                }`}
              >
                {v}
              </button>
            ))}
          </div>

          {/* Spacer + Updated at */}
          <div className="flex-1" />
          <span className="text-[11px] text-slate-400 shrink-0">
            {loading
              ? ZH.updating
              : `${ZH.updatedAt}: ${lastUpdatedAt ? formatTime(lastUpdatedAt.toISOString()) : "-"}`}
          </span>
        </div>
      </div>
    </header>
  );
}
