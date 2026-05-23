import { useState } from "react";
import { X, Download, Upload, Calendar, Receipt } from "lucide-react";
import type { PricingConfig, RestoreResponse, SubscriptionSettings } from "../api";

interface SettingsDrawerProps {
  open: boolean;
  onClose: () => void;

  subscriptionSettings: SubscriptionSettings | null;
  onSubscriptionSettingsChange: (s: SubscriptionSettings) => void;
  onSaveSubscriptionSettings: () => Promise<void>;

  pricingConfig: PricingConfig | null;

  onExport: () => Promise<void>;
  onRestore: (path: string) => Promise<void>;
  restoreLoading: boolean;
  restoreResult: RestoreResponse | null;
  restoreError: string | null;
}

export function SettingsDrawer({
  open,
  onClose,
  subscriptionSettings,
  onSubscriptionSettingsChange,
  onSaveSubscriptionSettings,
  pricingConfig,
  onExport,
  onRestore,
  restoreLoading,
  restoreResult,
  restoreError,
}: SettingsDrawerProps) {
  const [restorePath, setRestorePath] = useState("");
  const [subSavedAt, setSubSavedAt] = useState<number | null>(null);

  if (!open) return null;

  return (
    <aside className="w-52 shrink-0 bg-white border-r border-slate-200 flex flex-col h-[calc(100vh-2.75rem)] sticky top-11 z-30">
      <div className="flex items-center justify-between px-3 py-2 border-b border-slate-100">
        <h2 className="text-xs font-semibold text-slate-800 flex items-center gap-1.5">
          <span>设置</span>
        </h2>
        <button
          onClick={onClose}
          className="p-1 rounded hover:bg-slate-100 text-slate-400"
          aria-label="Close settings"
        >
          <X className="w-3.5 h-3.5" />
        </button>
      </div>

      <div className="flex-1 overflow-y-auto p-3 space-y-3">
        {/* 订阅设置 */}
        <section className="rounded-lg border border-slate-200 bg-white p-3">
          <h3 className="text-[11px] font-semibold text-slate-700 uppercase tracking-wider mb-2 flex items-center gap-1.5">
            <Calendar className="w-3 h-3 text-slate-400" />
            订阅设置
          </h3>
          <label className="block text-[10px] text-slate-500 mb-1">
            Kimi 月起始日 (1–28)
          </label>
          <div className="flex items-center gap-1.5">
            <input
              type="number"
              min={1}
              max={28}
              value={subscriptionSettings?.kimi_monthly_start_day ?? ""}
              onChange={(e) => {
                const v = e.target.value ? parseInt(e.target.value) : null;
                onSubscriptionSettingsChange({
                  ...(subscriptionSettings ?? { kimi_monthly_start_day: null }),
                  kimi_monthly_start_day: v,
                });
              }}
              className="flex-1 px-2 py-1 border border-slate-200 rounded text-xs focus:ring-1 focus:ring-primary-500 outline-none"
              placeholder="1-28"
            />
            <button
              onClick={async () => {
                await onSaveSubscriptionSettings();
                setSubSavedAt(Date.now());
              }}
              className="px-2 py-1 text-[11px] font-medium rounded bg-primary-600 text-white hover:bg-primary-700"
            >
              保存
            </button>
          </div>
          {subSavedAt && (
            <p className="mt-1 text-[10px] text-emerald-600">已保存</p>
          )}
        </section>

        {/* 计价逻辑 */}
        <section className="rounded-lg border border-slate-200 bg-white p-3">
          <h3 className="text-[11px] font-semibold text-slate-700 uppercase tracking-wider mb-2 flex items-center gap-1.5">
            <Receipt className="w-3 h-3 text-slate-400" />
            计价逻辑
          </h3>
          {pricingConfig ? (
            <div className="space-y-2 text-[11px] text-slate-600">
              <p>
                汇率 1 USD = {pricingConfig.usd_to_cny} CNY
                <span className="text-slate-400 ml-1">
                  ({pricingConfig.rate_date})
                </span>
              </p>
              <details className="rounded bg-slate-50 border border-slate-100 p-2">
                <summary className="cursor-pointer text-[10px] font-medium text-slate-600">
                  特殊计费规则
                </summary>
                <ul className="mt-1.5 list-disc list-inside space-y-0.5 text-[10px]">
                  <li>
                    讯飞: 每次 ¥{pricingConfig.special.xunfei_per_call.toFixed(6)}
                  </li>
                  <li>
                    Kimi CLI: 每 Token ¥
                    {pricingConfig.special.kimi_per_token.toExponential(3)}
                  </li>
                  <li>
                    OpenCode: cost ÷ {pricingConfig.special.opencode_divisor}
                  </li>
                  <li>pi / ccswitch: USD → CNY</li>
                  <li>codex / claude-code: 按模型价格表换算</li>
                </ul>
              </details>
              <details className="rounded bg-slate-50 border border-slate-100 p-2">
                <summary className="cursor-pointer text-[10px] font-medium text-slate-600">
                  模型价格表 (USD / 1M tokens)
                </summary>
                <table className="w-full text-left mt-1 text-[10px]">
                  <thead>
                    <tr className="text-slate-400 border-b border-slate-200">
                      <th className="pb-1 font-medium">模型</th>
                      <th className="pb-1 font-medium">In</th>
                      <th className="pb-1 font-medium">Out</th>
                      <th className="pb-1 font-medium">Cache</th>
                    </tr>
                  </thead>
                  <tbody>
                    {pricingConfig.model.map((m) => (
                      <tr
                        key={m.name}
                        className="border-b border-slate-100 last:border-0"
                      >
                        <td className="py-0.5 font-medium text-slate-700 truncate max-w-[80px]">
                          {m.name}
                        </td>
                        <td className="py-0.5">${m.input}</td>
                        <td className="py-0.5">${m.output}</td>
                        <td className="py-0.5">${m.cache_read}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </details>
            </div>
          ) : (
            <p className="text-[11px] text-slate-400">加载中...</p>
          )}
        </section>

        {/* 数据备份 */}
        <section className="rounded-lg border border-slate-200 bg-white p-3">
          <h3 className="text-[11px] font-semibold text-slate-700 uppercase tracking-wider mb-2 flex items-center gap-1.5">
            <Download className="w-3 h-3 text-slate-400" />
            数据备份
          </h3>
          <button
            onClick={() => {
              void onExport();
            }}
            className="w-full inline-flex items-center justify-center gap-1 px-2 py-1.5 text-[11px] font-medium rounded bg-slate-100 text-slate-700 hover:bg-slate-200 transition-colors"
          >
            <Download className="w-3 h-3" />
            导出备份
          </button>
        </section>

        {/* 数据恢复 */}
        <section className="rounded-lg border border-slate-200 bg-white p-3">
          <h3 className="text-[11px] font-semibold text-slate-700 uppercase tracking-wider mb-2 flex items-center gap-1.5">
            <Upload className="w-3 h-3 text-slate-400" />
            数据恢复
          </h3>
          <input
            type="text"
            value={restorePath}
            onChange={(e) => setRestorePath(e.target.value)}
            placeholder="备份目录路径"
            className="w-full px-2 py-1 text-[11px] border border-slate-200 rounded focus:ring-1 focus:ring-primary-500 outline-none bg-white"
          />
          <button
            onClick={() => {
              if (restorePath.trim()) {
                void onRestore(restorePath.trim());
              }
            }}
            disabled={restoreLoading || !restorePath.trim()}
            className="mt-1.5 w-full inline-flex items-center justify-center gap-1 px-2 py-1.5 text-[11px] font-medium rounded bg-amber-600 text-white hover:bg-amber-700 disabled:opacity-50 transition-colors"
          >
            <Upload className="w-3 h-3" />
            {restoreLoading ? "恢复中..." : "执行恢复"}
          </button>
          {restoreResult && (
            <p className="mt-1 text-[10px] text-emerald-700">
              已恢复 {restoreResult.added} / 跳过 {restoreResult.skipped}
              {restoreResult.errors.length > 0 &&
                ` · ${restoreResult.errors.length} 错误`}
            </p>
          )}
          {restoreError && (
            <p className="mt-1 text-[10px] text-rose-700">{restoreError}</p>
          )}
        </section>
      </div>
    </aside>
  );
}
