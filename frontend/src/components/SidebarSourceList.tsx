import { getSourceColor, getSourceLabel } from "../lib/utils";

interface SidebarSourceListProps {
  sources: string[];
  selectedSources: ReadonlySet<string>;
  onToggle: (source: string) => void;
}

export function SidebarSourceList({
  sources,
  selectedSources,
  onToggle,
}: SidebarSourceListProps) {
  if (sources.length === 0) return null;
  return (
    <div className="py-3 px-3">
      <p className="text-[10px] uppercase tracking-wider text-slate-400 font-semibold mb-1.5 px-2">
        工具
      </p>
      <div className="space-y-0.5">
        {sources.map((s) => (
          <label
            key={s}
            className="flex items-center gap-2 px-2 py-1.5 text-xs text-slate-700 hover:bg-slate-50 rounded cursor-pointer"
          >
            <input
              type="checkbox"
              checked={selectedSources.has(s)}
              onChange={() => onToggle(s)}
              className="rounded border-slate-300 text-primary-600 focus:ring-primary-500"
            />
            <span
              className="w-2 h-2 rounded-full shrink-0"
              style={{ background: getSourceColor(s) }}
            />
            <span className="truncate">{getSourceLabel(s)}</span>
          </label>
        ))}
      </div>
    </div>
  );
}
