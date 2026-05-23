import { Settings, X } from "lucide-react";
import { SidebarTimeRange } from "./SidebarTimeRange";
import { SidebarSourceList } from "./SidebarSourceList";
import { SidebarVendorList } from "./SidebarVendorList";
import { SidebarModelPicker } from "./SidebarModelPicker";
import type { TimePreset } from "../lib/timeRange";
import type { AppliedRange } from "../lib/filterState";

interface SidebarProps {
  open: boolean;
  onClose: () => void;
  activePreset: TimePreset;
  onTimeRangeChange: (preset: TimePreset, range: AppliedRange) => void;

  sources: string[];
  selectedSources: ReadonlySet<string>;
  onSourceToggle: (source: string) => void;

  vendors: string[];
  selectedVendors: ReadonlySet<string>;
  onVendorToggle: (vendor: string) => void;
  onSubscriptionGroupToggle: (selectAll: boolean) => void;

  models: string[];
  selectedModel: string;
  onModelChange: (model: string) => void;

  hideFreeModels: boolean;
  onHideFreeModelsChange: (hide: boolean) => void;

  onOpenSettings: () => void;
}

export function Sidebar({
  open,
  onClose,
  activePreset,
  onTimeRangeChange,
  sources,
  selectedSources,
  onSourceToggle,
  vendors,
  selectedVendors,
  onVendorToggle,
  onSubscriptionGroupToggle,
  models,
  selectedModel,
  onModelChange,
  hideFreeModels,
  onHideFreeModelsChange,
  onOpenSettings,
}: SidebarProps) {
  return (
    <>
      {/* Mobile overlay */}
      {open && (
        <div
          className="lg:hidden fixed inset-0 z-30 bg-black/30"
          onClick={onClose}
          aria-hidden
        />
      )}

      <aside
        className={`w-52 shrink-0 bg-white border-r border-slate-200 flex flex-col h-[calc(100vh-2.75rem)] sticky top-11 z-30 transition-transform ${
          open ? "translate-x-0" : "-translate-x-full lg:translate-x-0"
        } lg:static lg:translate-x-0 max-lg:fixed max-lg:top-11 max-lg:left-0`}
      >
        <div className="lg:hidden flex items-center justify-end px-3 py-1 border-b border-slate-100">
          <button
            onClick={onClose}
            className="p-1 rounded hover:bg-slate-100 text-slate-400"
            aria-label="Close sidebar"
          >
            <X className="w-3.5 h-3.5" />
          </button>
        </div>

        <div className="flex-1 overflow-y-auto divide-y divide-slate-100">
          <SidebarTimeRange
            activePreset={activePreset}
            onChange={onTimeRangeChange}
          />
          <SidebarSourceList
            sources={sources}
            selectedSources={selectedSources}
            onToggle={onSourceToggle}
          />
          <SidebarVendorList
            vendors={vendors}
            selectedVendors={selectedVendors}
            onToggle={onVendorToggle}
            onToggleSubscriptionGroup={onSubscriptionGroupToggle}
          />
          <SidebarModelPicker
            models={models}
            selectedModel={selectedModel}
            onChange={onModelChange}
            hideFreeModels={hideFreeModels}
            onHideFreeModelsChange={onHideFreeModelsChange}
          />
        </div>

        <button
          onClick={onOpenSettings}
          className="border-t border-slate-200 px-4 py-2.5 flex items-center gap-2 text-xs text-slate-600 hover:bg-slate-50 transition-colors"
        >
          <Settings className="w-3.5 h-3.5" />
          <span className="font-medium">设置</span>
        </button>
      </aside>
    </>
  );
}
