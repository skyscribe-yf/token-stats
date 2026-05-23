import { useMemo, useState } from "react";

interface SidebarModelPickerProps {
  models: string[];
  selectedModels: ReadonlySet<string>;
  onSelectedModelsChange: (next: Set<string>) => void;
  advancedModels: string[];
  hideFreeModels: boolean;
  onHideFreeModelsChange: (hide: boolean) => void;
}

export function SidebarModelPicker({
  models,
  selectedModels,
  onSelectedModelsChange,
  advancedModels,
  hideFreeModels,
  onHideFreeModelsChange,
}: SidebarModelPickerProps) {
  const [query, setQuery] = useState("");

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return models;
    return models.filter((m) => m.toLowerCase().includes(q));
  }, [models, query]);

  const allVisibleSelected =
    filtered.length > 0 && filtered.every((m) => selectedModels.has(m));

  const toggle = (model: string) => {
    const next = new Set(selectedModels);
    if (next.has(model)) next.delete(model);
    else next.add(model);
    onSelectedModelsChange(next);
  };

  const selectAllVisible = () => {
    const next = new Set(selectedModels);
    for (const m of filtered) next.add(m);
    onSelectedModelsChange(next);
  };

  const clearAll = () => {
    onSelectedModelsChange(new Set());
  };

  const applyAdvanced = () => {
    const available = new Set(models);
    const next = new Set(advancedModels.filter((m) => available.has(m)));
    onSelectedModelsChange(next);
  };

  return (
    <div className="py-3 px-3">
      <div className="px-2 mb-1.5 flex items-center justify-between">
        <p className="text-[10px] uppercase tracking-wider text-slate-400 font-semibold">
          模型
        </p>
        <div className="flex items-center gap-1.5">
          <button
            onClick={selectAllVisible}
            className="text-[10px] text-primary-600 hover:text-primary-700 font-medium"
            disabled={filtered.length === 0 || allVisibleSelected}
          >
            全选
          </button>
          <span className="text-slate-300 text-[10px]">·</span>
          <button
            onClick={applyAdvanced}
            className="text-[10px] text-primary-600 hover:text-primary-700 font-medium"
            disabled={advancedModels.length === 0}
            title="应用高级模型预设"
          >
            高级
          </button>
          <span className="text-slate-300 text-[10px]">·</span>
          <button
            onClick={clearAll}
            className="text-[10px] text-slate-500 hover:text-slate-700 font-medium"
            disabled={selectedModels.size === 0}
          >
            清除
          </button>
        </div>
      </div>

      <div className="px-2 mb-1.5">
        <input
          type="text"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="搜索模型..."
          className="w-full px-2 py-1 text-xs border border-slate-200 rounded outline-none focus:ring-1 focus:ring-primary-500 focus:border-primary-500"
        />
      </div>

      <div className="space-y-0.5 max-h-72 overflow-y-auto">
        {filtered.length === 0 ? (
          <p className="px-2 py-2 text-xs text-slate-400">无匹配</p>
        ) : (
          filtered.map((m) => (
            <label
              key={m}
              className="flex items-center gap-2 px-2 py-1.5 text-xs text-slate-700 hover:bg-slate-50 rounded cursor-pointer"
              title={m}
            >
              <input
                type="checkbox"
                checked={selectedModels.has(m)}
                onChange={() => toggle(m)}
                className="rounded border-slate-300 text-primary-600 focus:ring-primary-500"
              />
              <span className="truncate">{m}</span>
            </label>
          ))
        )}
      </div>

      <label className="mt-2 flex items-center gap-2 px-1 py-1 text-xs text-slate-700 hover:bg-slate-50 rounded cursor-pointer">
        <input
          type="checkbox"
          checked={hideFreeModels}
          onChange={(e) => onHideFreeModelsChange(e.target.checked)}
          className="rounded border-slate-300 text-primary-600 focus:ring-primary-500"
        />
        <span>过滤免费</span>
      </label>
    </div>
  );
}
