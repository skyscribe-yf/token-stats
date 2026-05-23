import { ExternalLink } from "lucide-react";
import {
  formatCalls,
  formatNumber,
  formatResetTime,
} from "../../lib/utils";
import type {
  QuotaResponse,
  XunfeiMultiStatus,
  XunfeiAccountStatus,
  AinaibaCreditResponse,
  OpenCodeQuotaStatus,
  KimiQuotaStatus,
  XiaomiMiMoQuotaStatus,
  SubscriptionSettings,
} from "../../api";

interface QuotasSectionProps {
  quota: QuotaResponse | null;
  xunfei: XunfeiMultiStatus | null;
  ainaibaCredit: AinaibaCreditResponse | null;
  quotaLoading: boolean;
  xunfeiLoading: boolean;
  ainaibaCreditLoading: boolean;
  subscriptionSettings: SubscriptionSettings | null;
  highlightCardId: string | null;
}

function barColor(used: number, limit: number): string {
  const ratio = used / Math.max(limit, 1);
  if (ratio > 0.8) return "bg-rose-500";
  if (ratio > 0.5) return "bg-amber-500";
  return "bg-emerald-500";
}

function CardShell({
  id,
  available,
  highlight,
  children,
}: {
  id: string;
  available: boolean;
  highlight: boolean;
  children: React.ReactNode;
}) {
  return (
    <div
      id={id}
      className={`bg-white rounded-xl border p-3 shadow-sm transition-shadow ${
        available ? "border-emerald-200" : "border-slate-200"
      } ${highlight ? "outline outline-2 outline-primary-400" : ""}`}
    >
      {children}
    </div>
  );
}

function CardHeader({
  active,
  loading,
  name,
  href,
  suffix,
}: {
  active: boolean;
  loading: boolean;
  name: string;
  href?: string;
  suffix?: string;
}) {
  const dotClass = loading
    ? "bg-amber-400"
    : active
      ? "bg-emerald-500"
      : "bg-slate-300";
  return (
    <div className="flex items-center justify-between mb-1.5">
      <div className="flex items-center gap-1.5">
        <span className={`w-1.5 h-1.5 rounded-full ${dotClass}`} />
        <span className="text-xs font-semibold text-slate-700">{name}</span>
      </div>
      {href ? (
        <a
          href={href}
          target="_blank"
          rel="noreferrer"
          className="inline-flex items-center gap-0.5 text-[10px] text-slate-400 hover:text-slate-600"
        >
          {suffix ?? new URL(href).hostname}
          <ExternalLink className="w-2.5 h-2.5" />
        </a>
      ) : (
        suffix && <span className="text-[10px] text-slate-400">{suffix}</span>
      )}
    </div>
  );
}

function ProgressBar({
  label,
  used,
  limit,
  suffix,
}: {
  label: string;
  used: number;
  limit: number;
  suffix?: string;
}) {
  const pct = (used / Math.max(limit, 1)) * 100;
  return (
    <div>
      <div className="flex justify-between text-[10px] text-slate-500">
        <span>{label}</span>
        <span>
          {formatNumber(used)}/{formatNumber(limit)} ({pct.toFixed(0)}%)
        </span>
      </div>
      <div className="w-full h-1 bg-slate-100 rounded-full overflow-hidden">
        <div
          className={`h-full rounded-full transition-all ${barColor(used, limit)}`}
          style={{ width: `${Math.min(pct, 100)}%` }}
        />
      </div>
      {suffix && (
        <span className="text-[10px] text-slate-400">{suffix}</span>
      )}
    </div>
  );
}

function useHighlightFlash(highlightId: string | null, cardId: string): boolean {
  return highlightId === cardId;
}

function XunfeiCard({
  account,
  loading,
  highlightId,
}: {
  account: XunfeiAccountStatus;
  loading: boolean;
  highlightId: string | null;
}) {
  const cardId = `quota-xunfei-${account.label}`;
  const flash = useHighlightFlash(highlightId, cardId);
  const active = account.available && account.data?.status === "active";

  return (
    <CardShell id={cardId} available={!!active} highlight={flash}>
      <CardHeader
        active={!!active}
        loading={loading}
        name={`讯飞编程套餐${account.label === "ex" ? " (EX)" : ""}`}
        href="https://xinghuo.xfyun.cn"
        suffix="xfyun.cn"
      />
      {loading ? (
        <SkeletonBars />
      ) : account.available && account.data ? (
        <>
          <div className="flex flex-wrap items-center gap-x-2 gap-y-0.5 text-[11px] mb-1.5">
            <span className="font-bold text-slate-800">
              {account.data.plan_name}
            </span>
            <span
              className={`px-1 py-0 rounded-full text-[10px] font-medium ${
                account.data.status === "active"
                  ? "bg-emerald-100 text-emerald-700"
                  : "bg-slate-100 text-slate-600"
              }`}
            >
              {account.data.status === "active" ? "有效" : account.data.status}
            </span>
            <span className="text-slate-400">
              ¥{(account.data.price / 100).toFixed(2)}/月
            </span>
          </div>
          <div className="space-y-1">
            {account.data.usage.rp5h_limit > 0 && (
              <ProgressBar
                label="5h"
                used={account.data.usage.rp5h_used}
                limit={account.data.usage.rp5h_limit}
              />
            )}
            {account.data.usage.rpw_limit > 0 && (
              <ProgressBar
                label="周"
                used={account.data.usage.rpw_used}
                limit={account.data.usage.rpw_limit}
              />
            )}
            <ProgressBar
              label="月"
              used={account.data.usage.package_used}
              limit={account.data.usage.package_limit}
            />
          </div>
          <div className="flex flex-wrap items-center gap-x-2 gap-y-0.5 mt-1.5 pt-1.5 border-t border-slate-100 text-[10px] text-slate-500">
            <span>余额 ¥{(account.data.balance.cash / 100).toFixed(2)}</span>
            {account.data.balance.virtual_balance > 0 && (
              <span>
                赠送 ¥{(account.data.balance.virtual_balance / 100).toFixed(2)}
              </span>
            )}
            <span>到期 {account.data.expires_at.replace(" ", "T")}</span>
          </div>
        </>
      ) : (
        <p className="text-[11px] text-slate-400 italic">
          {account.error || "获取失败"}
        </p>
      )}
    </CardShell>
  );
}

function AinaibaCard({
  status,
  loading,
  highlightId,
}: {
  status: AinaibaCreditResponse | null;
  loading: boolean;
  highlightId: string | null;
}) {
  const cardId = "quota-ainaiba";
  const flash = useHighlightFlash(highlightId, cardId);
  return (
    <CardShell
      id={cardId}
      available={!!status?.available}
      highlight={flash}
    >
      <CardHeader
        active={!!status?.available}
        loading={loading}
        name="Ainaiba"
        suffix="xai.ainaibahub"
      />
      {loading ? (
        <SkeletonBars />
      ) : status?.available && status.data ? (
        <>
          <div className="flex items-center justify-between text-[11px] mb-1.5">
            <span className="font-medium text-slate-600">
              {status.data.alias || status.data.name}
            </span>
            <span className="text-slate-400">#{status.data.user_id}</span>
          </div>
          <div className="flex items-center gap-3 mb-1.5">
            <div className="text-[10px] text-slate-500">
              <span className="text-slate-700 font-medium">
                {formatCalls(status.data.total_requests)}
              </span>{" "}
              总请求
            </div>
            <div className="text-[10px] text-slate-500">
              <span className="text-slate-700 font-medium">
                {status.data.credit_used.toFixed(2)}
              </span>{" "}
              / {status.data.credit_total.toFixed(2)} 额度
            </div>
          </div>
          <div className="space-y-1.5">
            <ProgressBar
              label="总额度"
              used={status.data.credit_used}
              limit={status.data.credit_total}
            />
            <ProgressBar
              label="日限"
              used={status.data.daily_used}
              limit={status.data.daily_limit}
            />
          </div>
          <details className="group mt-1.5">
            <summary className="cursor-pointer text-[10px] text-slate-500 hover:text-slate-700 transition-colors">
              详细用量
            </summary>
            <div className="mt-1 grid grid-cols-3 gap-x-2 gap-y-0.5 text-[10px]">
              <div className="text-slate-500">
                请求 <span className="text-slate-700">{formatCalls(status.data.daily_requests)}</span>
              </div>
              <div className="text-slate-500">
                输入 <span className="text-slate-700">{formatNumber(status.data.daily_input_tokens)}</span>
              </div>
              <div className="text-slate-500">
                输出 <span className="text-slate-700">{formatNumber(status.data.daily_output_tokens)}</span>
              </div>
              <div className="text-slate-500">
                推理 <span className="text-slate-700">{formatNumber(status.data.daily_reasoning_tokens)}</span>
              </div>
              <div className="text-slate-500">
                缓存 <span className="text-slate-700">{formatNumber(status.data.daily_cached_tokens)}</span>
              </div>
              <div className="text-slate-500">
                消耗 <span className="text-slate-700">{status.data.daily_used.toFixed(2)}</span>
              </div>
            </div>
          </details>
          <div className="pt-1.5 mt-1.5 border-t border-slate-100 flex flex-wrap items-center gap-x-2 gap-y-0.5 text-[10px] text-slate-500">
            <span>
              到期 {status.data.expires_at ? status.data.expires_at.slice(0, 10) : "-"}
            </span>
            <span>硬限 {status.data.hard_limit.toLocaleString()}</span>
            {status.data.rpm > 0 && (
              <span>
                限流 {status.data.rpm}/{status.data.rph}/{status.data.rpd}
              </span>
            )}
          </div>
        </>
      ) : (
        <p className="text-[11px] text-slate-400 italic">
          {status?.error || "获取失败"}
        </p>
      )}
    </CardShell>
  );
}

function KimiCard({
  status,
  loading,
  highlightId,
  subscriptionSettings,
}: {
  status: KimiQuotaStatus | null;
  loading: boolean;
  highlightId: string | null;
  subscriptionSettings: SubscriptionSettings | null;
}) {
  const cardId = "quota-kimi";
  const flash = useHighlightFlash(highlightId, cardId);
  return (
    <CardShell
      id={cardId}
      available={!!status?.available}
      highlight={flash}
    >
      <CardHeader
        active={!!status?.available}
        loading={loading}
        name="Kimi Code"
        href="https://kimi.com"
        suffix="kimi.com"
      />
      {loading ? (
        <SkeletonBars />
      ) : status?.available && status.data ? (
        <>
          <div className="flex items-center gap-2 text-[11px] mb-1">
            <span className="font-medium text-slate-600">
              {status.data.sub_type === "TYPE_PURCHASE"
                ? "付费版"
                : status.data.membership_level || "免费版"}
            </span>
            <span className="text-slate-400">
              并发 {status.data.parallel_limit}
            </span>
          </div>
          <div className="space-y-1">
            <ProgressBar
              label="周限额"
              used={status.data.weekly_used}
              limit={status.data.weekly_limit}
              suffix={formatResetTime(status.data.weekly_reset_time) ?? undefined}
            />
            {status.data.rp5h_limit > 0 && (
              <ProgressBar
                label="5小时"
                used={status.data.rp5h_used}
                limit={status.data.rp5h_limit}
                suffix={formatResetTime(status.data.rp5h_reset_time) ?? undefined}
              />
            )}
          </div>
          {status.data.total_limit > 0 && (
            <div className="mt-1 pt-1 border-t border-slate-100 text-[10px] text-slate-500">
              总配额 {status.data.total_remaining}/{status.data.total_limit}
            </div>
          )}
          {subscriptionSettings?.kimi_monthly_start_day && (
            <div className="mt-1 text-[10px] text-slate-400">
              月起始日: 每月 {subscriptionSettings.kimi_monthly_start_day} 号
            </div>
          )}
        </>
      ) : (
        <p className="text-[11px] text-slate-400 italic">获取失败</p>
      )}
    </CardShell>
  );
}

function OpenCodeCard({
  status,
  loading,
  highlightId,
  cardKey,
  suffix,
}: {
  status: OpenCodeQuotaStatus | null;
  loading: boolean;
  highlightId: string | null;
  cardKey: string;
  suffix?: string;
}) {
  const cardId = `quota-opencode-${cardKey}`;
  const flash = useHighlightFlash(highlightId, cardId);
  const exLabel = suffix === "ex" ? " (EX)" : "";
  return (
    <CardShell
      id={cardId}
      available={!!status?.available}
      highlight={flash}
    >
      <CardHeader
        active={!!status?.available}
        loading={loading}
        name={`OpenCode-go${exLabel}`}
        href="https://opencode.ai"
        suffix="opencode.ai"
      />
      {loading ? (
        <SkeletonBars />
      ) : status?.available && status.data && status.data.entries.length > 0 ? (
        <div className="space-y-1.5">
          {status.data.entries.map((entry) => {
            const scope =
              entry.usage_type === "Rolling"
                ? "滚动"
                : entry.usage_type === "Weekly"
                  ? "周"
                  : entry.usage_type === "Monthly"
                    ? "月"
                    : entry.usage_type;
            return (
              <div key={entry.usage_type}>
                <div className="flex justify-between text-[10px] text-slate-500">
                  <span>{scope}</span>
                  <span>
                    {entry.percentage}% · {entry.resets_in}
                  </span>
                </div>
                <div className="w-full h-1 bg-slate-100 rounded-full overflow-hidden">
                  <div
                    className={`h-full rounded-full transition-all ${
                      entry.percentage > 80
                        ? "bg-rose-500"
                        : entry.percentage > 50
                          ? "bg-amber-500"
                          : "bg-emerald-500"
                    }`}
                    style={{ width: `${Math.min(entry.percentage, 100)}%` }}
                  />
                </div>
              </div>
            );
          })}
        </div>
      ) : (
        <p className="text-[11px] text-slate-400 italic">
          {status?.error || "获取失败"}
        </p>
      )}
    </CardShell>
  );
}

function XiaomiMiMoCard({
  status,
  loading,
  highlightId,
}: {
  status: XiaomiMiMoQuotaStatus | null;
  loading: boolean;
  highlightId: string | null;
}) {
  const cardId = "quota-xiaomi-mimo";
  const flash = useHighlightFlash(highlightId, cardId);
  return (
    <CardShell id={cardId} available={!!status?.available} highlight={flash}>
      <CardHeader
        active={!!status?.available}
        loading={loading}
        name="Xiaomi MiMo TP"
        suffix="xiaomi.com"
      />
      {loading ? (
        <SkeletonBars />
      ) : status?.available && status.data ? (
        <>
          <div className="flex items-center gap-2 text-[11px] mb-1">
            <span className="font-medium text-slate-600">
              {status.data.plan_name || status.data.plan_code}
            </span>
            {status.data.enable_auto_renew && (
              <span className="px-1 py-0 rounded-full text-[10px] font-medium bg-emerald-100 text-emerald-700">
                自动续费
              </span>
            )}
          </div>
          <div className="space-y-1">
            {status.data.entries.map((entry) => (
              <ProgressBar
                key={entry.name}
                label={entry.name === "plan_total_token" ? "总配额" : entry.name}
                used={entry.used}
                limit={entry.limit}
              />
            ))}
          </div>
          {status.data.current_period_end && (
            <div className="mt-1 pt-1 border-t border-slate-100 text-[10px] text-slate-500">
              到期 {status.data.current_period_end.split(" ")[0]}
            </div>
          )}
          <div className="mt-2 p-2 bg-amber-50 border border-amber-200 rounded-lg">
            <p className="text-[10px] text-amber-700">
              ⚠️ 此价格为估算值，需根据实际使用情况调整
            </p>
          </div>
        </>
      ) : (
        <p className="text-[11px] text-slate-400 italic">
          {status?.error || "获取失败"}
        </p>
      )}
    </CardShell>
  );
}

function SkeletonBars() {
  return (
    <div className="space-y-1.5">
      <div className="h-3 w-full bg-slate-100 rounded animate-pulse" />
      <div className="h-1 w-full bg-slate-100 rounded animate-pulse" />
      <div className="h-3 w-2/3 bg-slate-100 rounded animate-pulse" />
      <div className="h-1 w-full bg-slate-100 rounded animate-pulse" />
    </div>
  );
}

export function QuotasSection({
  quota,
  xunfei,
  ainaibaCredit,
  quotaLoading,
  xunfeiLoading,
  ainaibaCreditLoading,
  subscriptionSettings,
  highlightCardId,
}: QuotasSectionProps) {
  return (
    <section id="section-quotas" className="space-y-3 scroll-mt-32">
      <h2 className="text-base font-semibold text-slate-800">订阅</h2>
      <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-2">
        {xunfei?.accounts?.map((acc) => (
          <XunfeiCard
            key={acc.label}
            account={acc}
            loading={xunfeiLoading}
            highlightId={highlightCardId}
          />
        ))}
        <AinaibaCard
          status={ainaibaCredit}
          loading={ainaibaCreditLoading}
          highlightId={highlightCardId}
        />
        <KimiCard
          status={quota?.kimi ?? null}
          loading={quotaLoading}
          highlightId={highlightCardId}
          subscriptionSettings={subscriptionSettings}
        />
        <XiaomiMiMoCard
          status={quota?.xiaomi_mimo ?? null}
          loading={quotaLoading}
          highlightId={highlightCardId}
        />
        <OpenCodeCard
          status={quota?.opencode_go ?? null}
          loading={quotaLoading}
          highlightId={highlightCardId}
          cardKey="primary"
        />
        <OpenCodeCard
          status={quota?.opencode_go_ex ?? null}
          loading={quotaLoading}
          highlightId={highlightCardId}
          cardKey="ex"
          suffix="ex"
        />
      </div>
    </section>
  );
}
