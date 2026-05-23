import { useState, useRef, useEffect, useMemo } from "react";
import { ChevronDown, X } from "lucide-react";

interface SidebarModelPickerProps {
  models: string[];
  selectedModel: string;
  onChange: (model: string) => void;
  hideFreeModels: boolean;
  onHideFreeModelsChange: (hide: boolean) => void;
}

export function SidebarModelPicker({
  models,
  selectedModel,
  onChange,
  hideFreeModels,
  onHideFreeModelsChange,
}: SidebarModelPickerProps) {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const wrapperRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const handler = (e: MouseEvent) => {
      const target = e.target as Node;
      if (wrapperRef.current && !wrapperRef.current.contains(target)) {
        setOpen(false);
      }
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [open]);

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return models;
    return models.filter((m) => m.toLowerCase().includes(q));
  }, [models, query]);

  const display = selectedModel || "全部模型";

  return (
    <div className="py-3 px-3">
      <p className="text-[10px] uppercase tracking-wider text-slate-400 font-semibold mb-1.5 px-2">
        模型
      </p>
      <div className="px-2 space-y-1.5">
        <div ref={wrapperRef} className="relative">
          <button
            onClick={() => {
              setOpen((v) => !v);
              setQuery("");
            }}
            className="w-full inline-flex items-center justify-between px-2 py-1.5 text-xs border border-slate-200 rounded bg-white hover:border-slate-300 focus:ring-1 focus:ring-primary-500 focus:border-primary-500 outline-none"
          >
            <span className="truncate text-left flex-1">{display}</span>
            {selectedModel && (
              <span
                role="button"
                tabIndex={0}
                onClick={(e) => {
                  e.stopPropagation();
                  onChange("");
                }}
                onKeyDown={(e) => {
                  if (e.key === "Enter" || e.key === " ") {
                    e.preventDefault();
                    e.stopPropagation();
                    onChange("");
                  }
                }}
                className="ml-1 p-0.5 rounded hover:bg-slate-100 text-slate-400 cursor-pointer"
                aria-label="清除模型筛选"
              >
                <X className="w-3 h-3" />
              </span>
            )}
            <ChevronDown className="w-3 h-3 text-slate-400 ml-1" />
          </button>
          {open && (
            <div className="absolute left-0 top-full mt-1 w-full bg-white border border-slate-200 rounded-lg shadow-lg z-30 flex flex-col max-h-72">
              <input
                type="text"
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                placeholder="搜索模型..."
                className="px-2 py-1.5 text-xs border-b border-slate-100 outline-none"
                autoFocus
              />
              <div className="flex-1 overflow-y-auto py-1">
                <button
                  onClick={() => {
                    onChange("");
                    setOpen(false);
                  }}
                  className={`block w-full text-left px-3 py-1.5 text-xs ${
                    selectedModel === ""
                      ? "bg-primary-50 text-primary-700 font-medium"
                      : "text-slate-700 hover:bg-slate-50"
                  }`}
                >
                  全部模型
                </button>
                {filtered.map((m) => (
                  <button
                    key={m}
                    onClick={() => {
                      onChange(m);
                      setOpen(false);
                    }}
                    className={`block w-full text-left px-3 py-1.5 text-xs truncate ${
                      m === selectedModel
                        ? "bg-primary-50 text-primary-700 font-medium"
                        : "text-slate-700 hover:bg-slate-50"
                    }`}
                    title={m}
                  >
                    {m}
                  </button>
                ))}
                {filtered.length === 0 && (
                  <p className="px-3 py-2 text-xs text-slate-400">无匹配</p>
                )}
              </div>
            </div>
          )}
        </div>

        <label className="flex items-center gap-2 px-1 py-1 text-xs text-slate-700 hover:bg-slate-50 rounded cursor-pointer">
          <input
            type="checkbox"
            checked={hideFreeModels}
            onChange={(e) => onHideFreeModelsChange(e.target.checked)}
            className="rounded border-slate-300 text-primary-600 focus:ring-primary-500"
          />
          <span>过滤免费</span>
        </label>
      </div>
    </div>
  );
}
