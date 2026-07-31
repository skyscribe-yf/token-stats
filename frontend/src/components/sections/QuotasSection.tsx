import { memo } from "react";
import { ExternalLink } from "lucide-react";
import {
  buildCycleCountdown,
  computeNextBillingDate,
  cycleCountdownTextClass,
  formatCalls,
  formatNumber,
  formatResetTime,
  type CycleCountdown,
} from "../../lib/utils";
import type {
  QuotaResponse,
  XunfeiMultiStatus,
  XunfeiAccountStatus,
  AinaibaCreditResponse,
  KimiQuotaStatus,
  CommandCodeQuotaStatus,
  OllamaQuotaStatus,
  MeituanQuotaStatus,
  FennoQuotaStatus,
  GrokQuotaStatus,
  SubscriptionSettings,
} from "../../api";
import { remainingQuota } from "../../lib/fennoQuota";

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
  cycleCountdown,
}: {
  active: boolean;
  loading: boolean;
  name: string;
  href?: string;
  suffix?: string;
  cycleCountdown?: CycleCountdown | null;
}) {
  const dotClass = loading
    ? "bg-amber-400"
    : active
      ? "bg-emerald-500"
      : "bg-slate-300";
  return (
    <div className="mb-1.5 space-y-0.5">
      <div className="flex items-center justify-between">
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
      {cycleCountdown && (
        <div className={cycleCountdownTextClass(cycleCountdown.isUrgent)}>
          {cycleCountdown.text}
        </div>
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

  // Only show active subscriptions; hide expired/inactive ones
  const activeSubs = account.data.filter((d) => d.status === "active");
  const allExpired = account.available && account.data.length > 0 && activeSubs.length === 0;
  // Display subscriptions: active ones, or if all expired just the first for context
  const displaySubs = activeSubs.length > 0 ? activeSubs : (allExpired ? [account.data[0]] : []);

  const hasActive = account.available && activeSubs.length > 0;
  // Use the earliest expiry across active subscriptions for the cycle countdown
  const earliestExpiry = activeSubs
    .map((d) => d.expires_at.includes("T") ? d.expires_at : d.expires_at.replace(" ", "T"))
    .sort()[0];
  const cycleCountdown = buildCycleCountdown(earliestExpiry ?? null);

  const suffix = account.label === "ex" ? " (EX)" : "";

  return (
    <CardShell id={cardId} available={!!hasActive} highlight={flash}>
      <CardHeader
        active={!!hasActive}
        loading={loading}
        name={`讯飞编程套餐${suffix}`}
        href="https://xinghuo.xfyun.cn"
        suffix="xfyun.cn"
        cycleCountdown={cycleCountdown}
      />
      {loading ? (
        <SkeletonBars />
      ) : account.available && displaySubs.length > 0 ? (
        <div className="space-y-2">
          {allExpired && (
            <div className="flex items-center gap-1.5 text-[11px] text-amber-600 bg-amber-50 rounded px-2 py-1 mb-1">
              <span className="font-medium">⚠ 所有订阅已失效</span>
              <span className="text-amber-500">· 请续费或更换套餐</span>
            </div>
          )}
          {displaySubs.map((sub, idx) => {
            const isActive = sub.status === "active";
            return (
              <div key={idx} className={idx > 0 ? "pt-2 border-t border-slate-100" : ""}>
                <div className="flex flex-wrap items-center gap-x-2 gap-y-0.5 text-[11px] mb-1.5">
                  <span className="font-bold text-slate-800">
                    {sub.plan_name}
                    {displaySubs.length > 1 && (
                      <span className="text-slate-400 font-normal ml-1">#{idx + 1}</span>
                    )}
                  </span>
                  <span
                    className={`px-1 py-0 rounded-full text-[10px] font-medium ${
                      isActive
                        ? "bg-emerald-100 text-emerald-700"
                        : "bg-slate-100 text-slate-600"
                    }`}
                  >
                    {isActive ? "有效" : sub.status}
                  </span>
                  <span className="text-slate-400">
                    ¥{(sub.price / 100).toFixed(2)}/月
                  </span>
                  {sub.api_key_masked && sub.api_key_masked !== "N/A" && (
                    <span className="text-slate-400 text-[10px]">🔑 {sub.api_key_masked}</span>
                  )}
                </div>
                {isActive && (
                  <div className="space-y-1">
                    {sub.usage.rp5h_limit > 0 && (
                      <ProgressBar
                        label="5h"
                        used={sub.usage.rp5h_used}
                        limit={sub.usage.rp5h_limit}
                      />
                    )}
                    {sub.usage.rpw_limit > 0 && (
                      <ProgressBar
                        label="周"
                        used={sub.usage.rpw_used}
                        limit={sub.usage.rpw_limit}
                      />
                    )}
                    <ProgressBar
                      label="月"
                      used={sub.usage.package_used}
                      limit={sub.usage.package_limit}
                    />
                  </div>
                )}
                <div className="flex flex-wrap items-center gap-x-2 gap-y-0.5 mt-1.5 text-[10px] text-slate-500">
                  {isActive && (
                    <>
                      <span>余额 ¥{(sub.balance.cash / 100).toFixed(2)}</span>
                      {sub.balance.virtual_balance > 0 && (
                        <span>
                          赠送 ¥{(sub.balance.virtual_balance / 100).toFixed(2)}
                        </span>
                      )}
                    </>
                  )}
                  <span>到期 {sub.expires_at.replace(" ", "T")}</span>
                </div>
              </div>
            );
          })}
        </div>
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
  const cycleCountdown = buildCycleCountdown(status?.data?.expires_at ?? null);
  return (
    <CardShell
      id={cardId}
      available={!!status?.available}
      highlight={flash}
    >
      <CardHeader
        active={!!status?.available}
        loading={loading}
        name="Yairouter"
        suffix="api.yairouter"
        cycleCountdown={cycleCountdown}
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
                ¥{status.data.balance.toFixed(2)}
              </span>{" "}
              剩余 / ¥{status.data.credit_total.toFixed(2)} 到账
            </div>
          </div>
          <div className="space-y-1.5">
            <ProgressBar
              label="已用"
              used={status.data.credit_total - status.data.balance}
              limit={status.data.credit_total}
            />
            <ProgressBar
              label="日限"
              used={status.data.daily_used}
              limit={status.data.daily_limit}
            />
          </div>
          {status.data.cards && status.data.cards.length > 1 && (
            <div className="mt-2 pt-2 border-t border-slate-100">
              <div className="text-[10px] text-slate-500 mb-1">到账卡明细</div>
              <div className="space-y-1">
                {status.data.cards.map((card, i) => (
                  <div key={i} className="flex items-center justify-between text-[10px]">
                    <span className="text-slate-500">
                      卡{i + 1}
                      {card.expires_at && (
                        <span className="ml-1 text-slate-400">
                          ({card.expires_at.slice(0, 10)})
                        </span>
                      )}
                    </span>
                    <span className="text-slate-700 font-medium tabular-nums">
                      剩¥{(card.balance ?? card.amount).toFixed(2)} / ¥{card.amount.toFixed(2)}
                    </span>
                  </div>
                ))}
              </div>
            </div>
          )}
          <details className="group mt-1.5">
            <summary className="cursor-pointer text-[10px] text-slate-500 hover:text-slate-700 transition-colors">
              详细用量
            </summary>
            <div className="mt-1.5 space-y-1.5">
              <ProgressBar
                label="已用"
                used={status.data.credit_used}
                limit={status.data.credit_used + status.data.balance}
              />
              <div className="grid grid-cols-3 gap-x-2 gap-y-0.5 text-[10px]">
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
  const monthlyStartDay = subscriptionSettings?.kimi_monthly_start_day ?? null;
  const cycleCountdown = monthlyStartDay
    ? buildCycleCountdown(
        computeNextBillingDate(monthlyStartDay)
      )
    : null;
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
        cycleCountdown={cycleCountdown}
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
          {monthlyStartDay && (
            <div className="mt-1 text-[10px] text-slate-400">
              月起始日: 每月 {monthlyStartDay} 号
            </div>
          )}
        </>
      ) : (
        <p className="text-[11px] text-slate-400 italic">获取失败</p>
      )}
    </CardShell>
  );
}

function CommandCodeCard({
  status,
  loading,
  highlightId,
}: {
  status: CommandCodeQuotaStatus | null;
  loading: boolean;
  highlightId: string | null;
}) {
  const cardId = "quota-commandcode";
  const flash = useHighlightFlash(highlightId, cardId);
  const cycleCountdown = buildCycleCountdown(
    status?.data?.current_period_end ?? null
  );
  return (
    <CardShell id={cardId} available={!!status?.available} highlight={flash}>
      <CardHeader
        active={!!status?.available}
        loading={loading}
        name="CommandCode"
        href="https://commandcode.ai"
        suffix="commandcode.ai"
        cycleCountdown={cycleCountdown}
      />
      {loading ? (
        <SkeletonBars />
      ) : status?.available && status.data ? (
        <>
          <div className="flex items-center gap-2 text-[11px] mb-1">
            <span className="font-medium text-slate-600">
              {status.data.plan_name}
            </span>
            <span
              className={`px-1 py-0 rounded-full text-[10px] font-medium ${
                status.data.subscription_status === "active"
                  ? "bg-emerald-100 text-emerald-700"
                  : "bg-slate-100 text-slate-600"
              }`}
            >
              {status.data.subscription_status === "active"
                ? "有效"
                : status.data.subscription_status}
            </span>
          </div>
          <div className="space-y-1">
            {status.data.monthly_credits_total != null &&
              status.data.monthly_credits_total > 0 && (
                <ProgressBar
                  label="月额度"
                  used={status.data.monthly_credits_used}
                  limit={status.data.monthly_credits_total}
                />
              )}
            {status.data.premium_monthly_credits > 0 && (
              <div className="flex justify-between text-[10px] text-slate-500">
                <span>高级月额</span>
                <span>${status.data.premium_monthly_credits.toFixed(2)} 剩余</span>
              </div>
            )}
            {status.data.opensource_monthly_credits > 0 && (
              <div className="flex justify-between text-[10px] text-slate-500">
                <span>开源月额</span>
                <span>
                  ${status.data.opensource_monthly_credits.toFixed(2)} 剩余
                </span>
              </div>
            )}
            {status.data.purchased_credits > 0 && (
              <div className="flex justify-between text-[10px] text-slate-500">
                <span>购买额度</span>
                <span>${status.data.purchased_credits.toFixed(2)}</span>
              </div>
            )}
          </div>
          <div className="mt-1.5 pt-1.5 border-t border-slate-100 flex flex-wrap gap-x-2 gap-y-0.5 text-[10px] text-slate-500">
            <span>
              请求 {status.data.total_requests.toLocaleString()}
            </span>
            <span>
              输入 {formatNumber(status.data.total_tokens_in)}
            </span>
            <span>
              输出 {formatNumber(status.data.total_tokens_out)}
            </span>
          </div>
          {status.data.current_period_end && (
            <div className="mt-1 text-[10px] text-slate-400">
              续订 {status.data.current_period_end.slice(0, 10)}
              {status.data.cancel_at_period_end && " · 取消续订"}
            </div>
          )}
        </>
      ) : (
        <p className="text-[11px] text-slate-400 italic">
          {status?.error || "获取失败"}
        </p>
      )}
    </CardShell>
  );
}

function FennoCard({
  status,
  loading,
  highlightId,
  suffix,
}: {
  status: FennoQuotaStatus | null;
  loading: boolean;
  highlightId: string | null;
  suffix?: string;
}) {
  const cardId = suffix === "EX" ? "quota-fenno-ex" : "quota-fenno";
  const label = `Fenno${suffix === "EX" ? " EX" : ""}`;
  const flash = useHighlightFlash(highlightId, cardId);
  const subscriptions = status?.data?.subscriptions ?? [];
  const activeSubscriptions = subscriptions.filter(
    (subscription) => subscription.status === "active",
  );
  const nearestExpiry = activeSubscriptions
    .map((subscription) => subscription.expires_at)
    .filter((expiresAt): expiresAt is string => expiresAt != null)
    .sort()[0];
  const cycleCountdown = buildCycleCountdown(nearestExpiry ?? null);

  return (
    <CardShell
      id={cardId}
      available={!!status?.available && activeSubscriptions.length > 0}
      highlight={flash}
    >
      <CardHeader
        active={!!status?.available && activeSubscriptions.length > 0}
        loading={loading}
        name={label}
        href="https://api.fenno.ai/subscriptions"
        suffix="fenno.ai"
        cycleCountdown={cycleCountdown}
      />
      {loading ? (
        <SkeletonBars />
      ) : status?.available && status.data ? (
        activeSubscriptions.length > 0 ? (
          <div className="space-y-2">
            {activeSubscriptions.map((subscription, index) => {
              const weeklyRemaining = remainingQuota(
                subscription.group.weekly_limit_usd,
                subscription.weekly_usage_usd,
              );
              const monthlyRemaining = remainingQuota(
                subscription.group.monthly_limit_usd,
                subscription.monthly_usage_usd,
              );

              return (
                <div
                  key={`${subscription.group.name}-${subscription.expires_at ?? index}`}
                  className={index > 0 ? "pt-2 border-t border-slate-100" : ""}
                >
                  <div className="flex flex-wrap items-center gap-x-2 gap-y-0.5 text-[11px] mb-1.5">
                    <span className="font-bold text-slate-800">
                      {subscription.group.name || "Fenno subscription"}
                    </span>
                    <span className="px-1 py-0 rounded-full text-[10px] font-medium bg-emerald-100 text-emerald-700">
                      有效
                    </span>
                    <span className="text-slate-400">
                      {subscription.group.platform}
                    </span>
                  </div>
                  <div className="space-y-1">
                    {subscription.group.weekly_limit_usd != null && (
                      <div>
                        <div className="flex justify-between text-[10px] text-slate-500">
                          <span>周额度</span>
                          <span>${weeklyRemaining?.toFixed(2)} 剩余</span>
                        </div>
                        <ProgressBar
                          label="已用"
                          used={subscription.weekly_usage_usd}
                          limit={subscription.group.weekly_limit_usd}
                        />
                      </div>
                    )}
                    {subscription.group.monthly_limit_usd != null ? (
                      <div>
                        <div className="flex justify-between text-[10px] text-slate-500">
                          <span>月额度</span>
                          <span>${monthlyRemaining?.toFixed(2)} 剩余</span>
                        </div>
                        <ProgressBar
                          label="已用"
                          used={subscription.monthly_usage_usd}
                          limit={subscription.group.monthly_limit_usd}
                        />
                      </div>
                    ) : (
                      <div className="text-[10px] text-slate-500">月额度不限</div>
                    )}
                  </div>
                  {subscription.expires_at && (
                    <div className="mt-1.5 text-[10px] text-slate-500">
                      到期 {subscription.expires_at.replace("T", " ").slice(0, 19)}
                    </div>
                  )}
                </div>
              );
            })}
          </div>
        ) : (
          <p className="text-[11px] text-slate-400 italic">暂无有效订阅</p>
        )
      ) : (
        <p className="text-[11px] text-slate-400 italic">
          {status?.error || "获取失败"}
        </p>
      )}
    </CardShell>
  );
}

function OllamaCard({
  status,
  loading,
  highlightId,
}: {
  status: OllamaQuotaStatus | null;
  loading: boolean;
  highlightId: string | null;
}) {
  const cardId = "quota-ollama";
  const flash = useHighlightFlash(highlightId, cardId);
  const data = status?.data;
  // Use the nearest reset time for the cycle countdown
  const sessionEntry = data?.usage_entries?.find(
    (e) => e.usage_type === "Session"
  );
  const weeklyEntry = data?.usage_entries?.find(
    (e) => e.usage_type === "Weekly"
  );
  const cycleCountdown = buildCycleCountdown(
    sessionEntry?.reset_time ?? weeklyEntry?.reset_time ?? null
  );

  return (
    <CardShell id={cardId} available={!!status?.available} highlight={flash}>
      <CardHeader
        active={!!status?.available}
        loading={loading}
        name="Ollama"
        href="https://ollama.com/settings/billing"
        suffix="ollama.com"
        cycleCountdown={cycleCountdown}
      />
      {loading ? (
        <SkeletonBars />
      ) : status?.available && data ? (
        <>
          <div className="flex items-center gap-2 text-[11px] mb-1.5">
            <span className="font-bold text-slate-800">
              {data.plan_name}
            </span>
            {data.price && (
              <span className="text-slate-400">{data.price}/月</span>
            )}
            {data.renews_on && (
              <span className="text-slate-400 text-[10px]">
                续费 {data.renews_on}
              </span>
            )}
          </div>
          <div className="flex items-center justify-between text-[10px] mb-1.5">
            <div className="flex items-center gap-1.5 text-slate-500">
              {data.has_max_upgrade && (
                <a
                  href="https://ollama.com/settings/billing"
                  target="_blank"
                  rel="noreferrer"
                  className="text-primary-500 hover:underline"
                >
                  升级至 Max
                </a>
              )}
              {data.has_max_upgrade && data.has_annual_option && " · "}
              {data.has_annual_option && (
                <a
                  href="https://ollama.com/settings/billing"
                  target="_blank"
                  rel="noreferrer"
                  className="text-primary-500 hover:underline"
                >
                  切换年付
                </a>
              )}
            </div>
            {data.estimated_tokens_used != null && data.estimated_cost_cny != null && (
              <div className="flex items-center gap-2">
                <span className="text-slate-400">
                  本周 {formatNumber(data.estimated_tokens_used)}
                </span>
                <span className="text-slate-700 font-medium">
                  ≈¥{data.estimated_cost_cny.toFixed(2)}
                </span>
              </div>
            )}
          </div>
          {data.usage_entries.length > 0 && (
            <div className="space-y-1.5">
              {data.usage_entries.map((entry) => {
                const scope =
                  entry.usage_type === "Session" ? "会话" : "周";
                const pctDisplay =
                  entry.percentage % 1 === 0
                    ? `${entry.percentage}`
                    : entry.percentage.toFixed(1);
                const resetDisplay = formatResetTime(entry.reset_time);
                return (
                  <div key={entry.usage_type}>
                    <div className="flex justify-between text-[10px] text-slate-500">
                      <span>{scope}</span>
                      <span>
                        {pctDisplay}%
                        {resetDisplay && ` · 剩余 ${resetDisplay}`}
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
                        style={{
                          width: `${Math.min(entry.percentage, 100)}%`,
                        }}
                      />
                    </div>
                  </div>
                );
              })}
            </div>
          )}


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

function MeituanCard({
  status,
  loading,
  highlightId,
}: {
  status: MeituanQuotaStatus | null;
  loading: boolean;
  highlightId: string | null;
}) {
  const cardId = "quota-meituan";
  const flash = useHighlightFlash(highlightId, cardId);
  const data = status?.data;

  // Only show packs with remaining credits; hide burned-out packs
  const remainingPacks = data?.packs.filter((p) => p.remain_token_amount > 0) ?? [];
  const activePacks = remainingPacks.filter((p) => p.status_code === 2);
  const hasActive = activePacks.length > 0;

  // Hide card entirely when all packs are burned out (not loading)
  if (!loading && remainingPacks.length === 0) return null;

  // Nearest expiry from active packs
  const nearestExpiry = activePacks
    .map((p) => p.valid_end_date_text)
    .sort()[0];

  // Sum across active packs for overall stats
  const totalRemain = activePacks.reduce((s, p) => s + p.remain_token_amount, 0);
  const totalAmount = activePacks.reduce((s, p) => s + p.total_token_amount, 0);
  const totalUsed = activePacks.reduce((s, p) => s + p.used_token_amount, 0);
  const overallPct = totalAmount > 0 ? Math.round((totalUsed / totalAmount) * 100) : 0;

  return (
    <CardShell id={cardId} available={!!status?.available && hasActive} highlight={flash}>
      <CardHeader
        active={!!status?.available && hasActive}
        loading={loading}
        name="美团 LongCat"
        href="https://longcat.chat/platform/usage"
        suffix="longcat.chat"
        cycleCountdown={nearestExpiry ? buildCycleCountdown(nearestExpiry) : null}
      />
      {loading ? (
        <SkeletonBars />
      ) : status?.available && data && data.packs.length > 0 ? (
        <div className="space-y-2">
          {/* Overall summary when multiple active packs */}
          {activePacks.length > 1 && (
            <div className="space-y-1 pb-2 border-b border-slate-100">
              <div className="flex justify-between text-[10px] text-slate-500">
                <span>合计 {formatNumber(totalRemain)} 剩余</span>
                <span>{overallPct}% 已消耗</span>
              </div>
              <div className="w-full h-1 bg-slate-100 rounded-full overflow-hidden">
                <div
                  className={`h-full rounded-full transition-all ${barColor(totalUsed, totalAmount)}`}
                  style={{ width: `${Math.min(overallPct, 100)}%` }}
                />
              </div>
            </div>
          )}

          {/* Per-pack details */}
          {remainingPacks.map((pack, i) => {
            const isActive = pack.status_code === 2;
            return (
              <div
                key={i}
                className={i > 0 ? "pt-2 border-t border-slate-100" : ""}
              >
                <div className="flex flex-wrap items-center gap-x-2 gap-y-0.5 text-[11px] mb-1.5">
                  <span className="font-bold text-slate-800 truncate max-w-[55%]" title={pack.package_name}>
                    {pack.source_type_text}
                  </span>
                  <span
                    className={`px-1 py-0 rounded-full text-[10px] font-medium ${
                      isActive
                        ? "bg-emerald-100 text-emerald-700"
                        : "bg-slate-100 text-slate-600"
                    }`}
                  >
                    {isActive ? "有效" : pack.status_text}
                  </span>
                  {pack.applicable_models.length > 0 && (
                    <span className="text-slate-400 text-[10px]">
                      {pack.applicable_models.join(", ")}
                    </span>
                  )}
                </div>
                {isActive && (pack.usage_percent > 0 || pack.used_token_amount > 0) && (
                  <div className="space-y-1">
                    <ProgressBar
                      label="用量"
                      used={pack.used_token_amount}
                      limit={pack.total_token_amount}
                    />
                  </div>
                )}
                <div className="flex flex-wrap items-center gap-x-2 gap-y-0.5 mt-1.5 text-[10px] text-slate-500">
                  {isActive && (
                    <span>
                      剩余 {formatNumber(pack.remain_token_amount)} / {formatNumber(pack.total_token_amount)}
                    </span>
                  )}
                  <span>到期 {pack.valid_end_date_text}</span>
                </div>
              </div>
            );
          })}

          {/* 7-day usage */}
          {data.recent_7d_tokens > 0 && (
            <div className="pt-2 border-t border-slate-100 text-[10px] text-slate-400">
              近7天消耗 {formatNumber(data.recent_7d_tokens)}
            </div>
          )}
        </div>
      ) : (
        <p className="text-[11px] text-slate-400 italic">
          {status?.error || "获取失败"}
        </p>
      )}
    </CardShell>
  );
}

function GrokCard({
  status,
  loading,
  highlightId,
}: {
  status: GrokQuotaStatus | null;
  loading: boolean;
  highlightId: string | null;
}) {
  const cardId = "quota-grok";
  const flash = useHighlightFlash(highlightId, cardId);
  const data = status?.data;
  const usedPercent =
    data?.weekly_usage_percent == null
      ? null
      : Math.min(Math.max(data.weekly_usage_percent, 0), 100);
  const remainingPercent =
    data?.weekly_remaining_percent == null
      ? null
      : Math.min(Math.max(data.weekly_remaining_percent, 0), 100);
  const resetDisplay = formatResetTime(data?.weekly_reset_at);
  const cycleCountdown = buildCycleCountdown(data?.weekly_reset_at ?? null);
  const breakdown = data?.weekly_breakdown ?? [];
  const productLabels: Record<string, string> = {
    third_party: "第三方",
    api: "API",
    build: "Build",
    plugins: "插件",
    chat: "对话",
    imagine: "Imagine",
    voice: "语音",
  };

  return (
    <CardShell id={cardId} available={!!status?.available} highlight={flash}>
      <CardHeader
        active={!!status?.available}
        loading={loading}
        name="Super Grok"
        href="https://grok.com/?_s=usage"
        suffix="grok.com"
      />
      {loading ? (
        <SkeletonBars />
      ) : status?.available && data && usedPercent !== null && remainingPercent !== null ? (
        <>
          <div className="flex items-center gap-2 text-[11px] mb-1.5">
            <span className="font-bold text-slate-800">Weekly SuperGrok Limit</span>
            <span className="px-1 py-0 rounded-full text-[10px] font-medium bg-emerald-100 text-emerald-700">
              实时
            </span>
          </div>
          <div className="flex items-center justify-between text-[10px] text-slate-500 mb-1">
            <span>
              已用 {usedPercent.toFixed(0)}% · 剩余 {remainingPercent.toFixed(0)}%
            </span>
            <span className="text-slate-400">
              {resetDisplay ?? "重置时间未知"}
            </span>
          </div>
          <div className="space-y-1">
            <div className="flex justify-between text-[10px] text-slate-500">
              <span>本周已用</span>
              <span className="text-slate-700 font-medium tabular-nums">
                {usedPercent.toFixed(0)}%
              </span>
            </div>
            <div className="w-full h-1 bg-slate-100 rounded-full overflow-hidden">
              <div
                className={`h-full rounded-full transition-all ${barColor(usedPercent, 100)}`}
                style={{
                  width: `${usedPercent}%`,
                }}
              />
            </div>
          </div>
          {cycleCountdown && (
            <div className={`mt-1.5 ${cycleCountdownTextClass(cycleCountdown.isUrgent)}`}>
              {cycleCountdown.text}
            </div>
          )}
          {breakdown.length > 0 && (
            <div className="mt-2 pt-1.5 border-t border-slate-100 space-y-1">
              {breakdown.map((entry) => (
                <div
                  key={entry.product}
                  className="flex items-center justify-between text-[10px] text-slate-500"
                >
                  <span>{productLabels[entry.product] ?? entry.product}</span>
                  <span className="tabular-nums">{entry.usage_percent.toFixed(0)}% 已用</span>
                </div>
              ))}
            </div>
          )}
          {data.zdr_status && data.zdr_status !== "no_zdr" && (
            <div className="mt-1 text-[10px] text-amber-500">
              ZDR: {data.zdr_status}
            </div>
          )}
        </>
      ) : (
        <p className="text-[11px] text-slate-400 italic">
          {status?.error || (status?.available ? "未返回实时周额度" : "获取失败")}
        </p>
      )}
    </CardShell>
  );
}

export const QuotasSection = memo(function QuotasSection({
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

        <CommandCodeCard
          status={quota?.commandcode ?? null}
          loading={quotaLoading}
          highlightId={highlightCardId}
        />
        <FennoCard
          status={quota?.fenno ?? null}
          loading={quotaLoading}
          highlightId={highlightCardId}
        />
        <FennoCard
          status={quota?.fenno_ex ?? null}
          loading={quotaLoading}
          highlightId={highlightCardId}
          suffix="EX"
        />
        <OllamaCard
          status={quota?.ollama ?? null}
          loading={quotaLoading}
          highlightId={highlightCardId}
        />
        <MeituanCard
          status={quota?.meituan ?? null}
          loading={quotaLoading}
          highlightId={highlightCardId}
        />
        <GrokCard
          status={quota?.grok ?? null}
          loading={quotaLoading}
          highlightId={highlightCardId}
        />
      </div>
    </section>
  );
});
