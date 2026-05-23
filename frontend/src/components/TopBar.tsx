import { Activity, RefreshCw, Menu } from "lucide-react";
import { formatTime } from "../lib/utils";

interface TopBarProps {
  title: string;
  lastUpdatedAt: Date | null;
  loading: boolean;
  onRefresh: () => void;
  onToggleSidebar?: () => void;
}

export function TopBar({
  title,
  lastUpdatedAt,
  loading,
  onRefresh,
  onToggleSidebar,
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
      <div className="flex items-center gap-2">
        <div className="bg-primary-600 p-1.5 rounded-lg">
          <Activity className="w-4 h-4 text-white" />
        </div>
        <h1 className="text-sm font-bold text-slate-800 leading-tight">
          {title}
        </h1>
      </div>
      <div className="ml-auto flex items-center gap-2">
        <span className="text-xs text-slate-400 tabular-nums">
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
