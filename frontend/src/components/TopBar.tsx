import { Activity, RefreshCw, Menu } from "lucide-react";
import { formatTime } from "../lib/utils";

export type SectionId = "usage" | "quotas" | "requests";

const SECTIONS: { id: SectionId; label: string }[] = [
  { id: "usage", label: "用量" },
  { id: "quotas", label: "订阅" },
  { id: "requests", label: "请求" },
];

interface TopBarProps {
  title: string;
  lastUpdatedAt: Date | null;
  loading: boolean;
  onRefresh: () => void;
  onToggleSidebar?: () => void;
  activeSection: SectionId;
  onSectionSelect: (id: SectionId) => void;
}

export function TopBar({
  title,
  lastUpdatedAt,
  loading,
  onRefresh,
  onToggleSidebar,
  activeSection,
  onSectionSelect,
}: TopBarProps) {
  return (
    <header className="h-11 bg-white border-b border-slate-200 sticky top-0 z-30 flex items-center px-4">
      {onToggleSidebar && (
        <button
          onClick={onToggleSidebar}
          className="lg:hidden mr-2 p-1.5 rounded hover:bg-slate-100 text-slate-500"
          aria-label="Toggle sidebar"
        >
          <Menu className="w-4 h-4" />
        </button>
      )}
      <div className="flex items-center gap-2 min-w-0">
        <div className="bg-primary-600 p-1.5 rounded-lg shrink-0">
          <Activity className="w-4 h-4 text-white" />
        </div>
        <h1 className="text-sm font-bold text-slate-800 leading-tight truncate">
          {title}
        </h1>
      </div>

      <nav
        aria-label="Section navigation"
        className="absolute left-1/2 -translate-x-1/2"
      >
        <div className="inline-flex items-center bg-slate-100 rounded-lg p-0.5 gap-0.5">
          {SECTIONS.map((s) => (
            <button
              key={s.id}
              onClick={() => onSectionSelect(s.id)}
              className={`px-3 py-1 text-xs font-medium rounded-md transition-colors ${
                activeSection === s.id
                  ? "bg-white text-primary-700 shadow-sm"
                  : "text-slate-500 hover:text-slate-700"
              }`}
            >
              {s.label}
            </button>
          ))}
        </div>
      </nav>

      <div className="ml-auto flex items-center gap-2 shrink-0">
        <span className="hidden sm:inline text-xs text-slate-400 tabular-nums">
          Updated {lastUpdatedAt ? formatTime(lastUpdatedAt.toISOString()).slice(11, 19) : "—"}
        </span>
        <button
          onClick={onRefresh}
          disabled={loading}
          className="p-1.5 rounded hover:bg-slate-100 text-slate-500 disabled:opacity-50 disabled:cursor-not-allowed"
          title="刷新"
          aria-label="Refresh"
        >
          <RefreshCw className={`w-3.5 h-3.5 ${loading ? "animate-spin" : ""}`} />
        </button>
      </div>
    </header>
  );
}
