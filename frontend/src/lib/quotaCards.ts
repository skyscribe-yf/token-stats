/** Stable keys for hiding subscription cards (persisted in localStorage). */
export interface QuotaCardDef {
  key: string;
  label: string;
  matches: (cardId: string) => boolean;
}

export const QUOTA_CARD_DEFS: QuotaCardDef[] = [
  { key: "xunfei", label: "讯飞编程套餐", matches: (id) => id.startsWith("quota-xunfei-") },
  { key: "ainaiba", label: "Yairouter", matches: (id) => id === "quota-ainaiba" },
  { key: "kimi", label: "Kimi Code", matches: (id) => id === "quota-kimi" },
  { key: "opencode", label: "OpenCode-go", matches: (id) => id === "quota-opencode-primary" },
  { key: "opencode-ex", label: "OpenCode-go EX", matches: (id) => id === "quota-opencode-ex" },
  { key: "commandcode", label: "CommandCode", matches: (id) => id === "quota-commandcode" },
  { key: "commandcode-ex", label: "CommandCode EX", matches: (id) => id === "quota-commandcode-ex" },
  { key: "codebuddy", label: "CodeBuddy 套餐", matches: (id) => id === "quota-codebuddy" },
  { key: "fenno", label: "Fenno", matches: (id) => id === "quota-fenno" },
  { key: "fenno-ex", label: "Fenno EX", matches: (id) => id === "quota-fenno-ex" },
  { key: "ollama", label: "Ollama", matches: (id) => id === "quota-ollama" },
  { key: "meituan", label: "美团 LongCat", matches: (id) => id === "quota-meituan" },
  { key: "grok", label: "Super Grok", matches: (id) => id === "quota-grok" },
  { key: "dimagent", label: "DimAgent", matches: (id) => id === "quota-dimagent" },
  { key: "zcode", label: "ZCode", matches: (id) => id === "quota-zcode" },
  { key: "zai", label: "ZAI", matches: (id) => id === "quota-zai" },
];

export function isQuotaCardHidden(
  hiddenCards: Set<string>,
  cardId: string
): boolean {
  return QUOTA_CARD_DEFS.some(
    (d) => d.matches(cardId) && hiddenCards.has(d.key)
  );
}

/** Whether a configured quota card should be omitted when its fetch fails.
 *  Cookie expiry / 401 must keep the card visible with an error — otherwise
 *  a live subscription looks like it vanished. Only cards that are truly
 *  unconfigured (no credentials at all) may hide. */
export function hideUnavailableQuotaCard(
  unconfigured?: boolean
): boolean {
  return unconfigured === true;
}

/** ZCode limit-window usage math.
 *  Live-API semantics (verified 2026-09-12): `usage` is the window TOTAL,
 *  `currentValue` is consumed, `remaining` is what's left, and `percentage`
 *  is the used percent (currentValue/usage). Falls back to
 *  currentValue+remaining when `usage` is missing. Returns null when the
 *  window has no size at all. */
export function zcodeWindowUsage(entry: {
  usage?: number | null;
  currentValue?: number | null;
  remaining?: number | null;
}): { used: number; limit: number; usedPct: number } | null {
  const total =
    entry.usage != null && entry.usage > 0
      ? entry.usage
      : (entry.currentValue ?? 0) + (entry.remaining ?? 0);
  if (!(total > 0)) return null;
  const used =
    entry.currentValue != null
      ? entry.currentValue
      : Math.max(total - (entry.remaining ?? 0), 0);
  const usedPct = Math.min(Math.max((used / total) * 100, 0), 100);
  return { used, limit: total, usedPct };
}
