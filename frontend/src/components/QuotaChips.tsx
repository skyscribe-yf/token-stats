import type {
  QuotaResponse,
  XunfeiMultiStatus,
  AinaibaCreditResponse,
} from "../api";

interface QuotaChip {
  id: string;
  cardId: string;
  vendor: string;
  scope: string;
  display: string;
  pct: number | null;
}

interface QuotaChipsProps {
  quota: QuotaResponse | null;
  xunfei: XunfeiMultiStatus | null;
  ainaibaCredit: AinaibaCreditResponse | null;
  loading: boolean;
  onChipClick: (cardId: string) => void;
}

function colorClass(pct: number | null): string {
  if (pct == null) {
    return "bg-slate-50 text-slate-700 border-slate-200";
  }
  if (pct > 90) return "bg-rose-50 text-rose-700 border-rose-200";
  if (pct >= 75) return "bg-amber-50 text-amber-700 border-amber-200";
  return "bg-slate-50 text-slate-700 border-slate-200";
}

function barColorClass(pct: number | null): string {
  if (pct == null) return "bg-slate-300";
  if (pct > 90) return "bg-rose-500";
  if (pct >= 75) return "bg-amber-500";
  return "bg-slate-400";
}

function buildQuotaChips(
  quota: QuotaResponse | null,
  xunfei: XunfeiMultiStatus | null,
  ainaibaCredit: AinaibaCreditResponse | null
): QuotaChip[] {
  const chips: QuotaChip[] = [];

  // Xunfei accounts
  if (xunfei?.accounts) {
    for (const acc of xunfei.accounts) {
      if (!acc.available || !acc.data) continue;
      const suffix = acc.label === "ex" ? " EX" : "";
      const usage = acc.data.usage;
      if (usage.rp5h_limit > 0) {
        const pct = (usage.rp5h_used / usage.rp5h_limit) * 100;
        chips.push({
          id: `xunfei-${acc.label}-5h`,
          cardId: `quota-xunfei-${acc.label}`,
          vendor: `讯飞${suffix}`,
          scope: "5h",
          display: `${pct.toFixed(0)}%`,
          pct,
        });
      }
      if (usage.rpw_limit > 0) {
        const pct = (usage.rpw_used / usage.rpw_limit) * 100;
        chips.push({
          id: `xunfei-${acc.label}-w`,
          cardId: `quota-xunfei-${acc.label}`,
          vendor: `讯飞${suffix}`,
          scope: "周",
          display: `${pct.toFixed(0)}%`,
          pct,
        });
      }
      if (usage.package_limit > 0) {
        const pct = (usage.package_used / usage.package_limit) * 100;
        chips.push({
          id: `xunfei-${acc.label}-m`,
          cardId: `quota-xunfei-${acc.label}`,
          vendor: `讯飞${suffix}`,
          scope: "月",
          display: `${pct.toFixed(0)}%`,
          pct,
        });
      }
    }
  }

  // Kimi
  if (quota?.kimi?.available && quota.kimi.data) {
    const k = quota.kimi.data;
    if (k.rp5h_limit > 0) {
      const pct = (k.rp5h_used / k.rp5h_limit) * 100;
      chips.push({
        id: "kimi-5h",
        cardId: "quota-kimi",
        vendor: "Kimi",
        scope: "5h",
        display: `${pct.toFixed(0)}%`,
        pct,
      });
    }
    if (k.weekly_limit > 0) {
      const pct = (k.weekly_used / k.weekly_limit) * 100;
      chips.push({
        id: "kimi-w",
        cardId: "quota-kimi",
        vendor: "Kimi",
        scope: "周",
        display: `${pct.toFixed(0)}%`,
        pct,
      });
    }
  }

  // OpenCode-go variants
  const ocPairs: { status: QuotaResponse["opencode_go"]; suffix: string; key: string }[] = quota
    ? [
        { status: quota.opencode_go, suffix: "", key: "primary" },
        { status: quota.opencode_go_ex, suffix: " EX", key: "ex" },
      ]
    : [];
  for (const { status, suffix, key } of ocPairs) {
    if (!status?.available || !status.data) continue;
    for (const entry of status.data.entries) {
      const scope =
        entry.usage_type === "Rolling"
          ? "滚动"
          : entry.usage_type === "Weekly"
            ? "周"
            : entry.usage_type === "Monthly"
              ? "月"
              : entry.usage_type;
      chips.push({
        id: `opencode-${key}-${entry.usage_type}`,
        cardId: `quota-opencode-${key}`,
        vendor: `OpenCode${suffix}`,
        scope,
        display: `${entry.percentage}%`,
        pct: entry.percentage,
      });
    }
  }

  // Ainaiba credit balance — show individual cards when multiple exist
  if (ainaibaCredit?.available && ainaibaCredit.data) {
    const a = ainaibaCredit.data;
    if (a.cards && a.cards.length > 1) {
      // Multiple cards: show each one individually
      for (let i = 0; i < a.cards.length; i++) {
        const card = a.cards[i];
        chips.push({
          id: `ainaiba-balance-${i}`,
          cardId: "quota-ainaiba",
          vendor: "Ainaiba",
          scope: `卡${i + 1}`,
          display: `¥${card.amount.toFixed(2)}`,
          pct: null,
        });
      }
    } else {
      // Single card or legacy: show total balance
      chips.push({
        id: "ainaiba-balance",
        cardId: "quota-ainaiba",
        vendor: "Ainaiba",
        scope: "余",
        display: `¥${a.balance.toFixed(2)}`,
        pct: null,
      });
    }
  }

  // CommandCode
  if (quota?.commandcode?.available && quota.commandcode.data) {
    const cc = quota.commandcode.data;
    if (cc.monthly_credits_total != null && cc.monthly_credits_total > 0) {
      const pct = (cc.monthly_credits_used / cc.monthly_credits_total) * 100;
      chips.push({
        id: "commandcode-monthly",
        cardId: "quota-commandcode",
        vendor: "CmdCode",
        scope: "月",
        display: `${pct.toFixed(0)}%`,
        pct,
      });
    }
    if (cc.purchased_credits > 0) {
      chips.push({
        id: "commandcode-purchased",
        cardId: "quota-commandcode",
        vendor: "CmdCode",
        scope: "余",
        display: `$${cc.purchased_credits.toFixed(2)}`,
        pct: null,
      });
    }
  }

  return chips;
}

export function QuotaChips({
  quota,
  xunfei,
  ainaibaCredit,
  loading,
  onChipClick,
}: QuotaChipsProps) {
  const chips = buildQuotaChips(quota, xunfei, ainaibaCredit);

  if (loading && chips.length === 0) {
    return (
      <div className="flex flex-wrap gap-1.5">
        {Array.from({ length: 4 }).map((_, i) => (
          <div
            key={i}
            className="h-7 w-32 bg-slate-100 rounded-full animate-pulse"
          />
        ))}
      </div>
    );
  }

  if (chips.length === 0) return null;

  return (
    <div className="flex flex-wrap gap-1.5">
      {chips.map((c) => (
        <button
          key={c.id}
          onClick={() => onChipClick(c.cardId)}
          className={`inline-flex items-center gap-1.5 px-2.5 py-1 text-[11px] font-medium rounded-full border transition-colors ${colorClass(c.pct)} hover:shadow-sm`}
        >
          <span className="text-slate-500 font-medium">{c.vendor}</span>
          <span className="text-[10px] uppercase tracking-wider text-slate-400">
            {c.scope}
          </span>
          <span className="tabular-nums font-semibold">{c.display}</span>
          {c.pct != null && (
            <span className="inline-flex h-1 w-10 rounded-full bg-slate-200 overflow-hidden">
              <span
                className={`h-full ${barColorClass(c.pct)}`}
                style={{ width: `${Math.min(c.pct, 100)}%` }}
              />
            </span>
          )}
        </button>
      ))}
    </div>
  );
}
