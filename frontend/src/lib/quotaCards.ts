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
  { key: "fenno", label: "Fenno", matches: (id) => id === "quota-fenno" },
  { key: "fenno-ex", label: "Fenno EX", matches: (id) => id === "quota-fenno-ex" },
  { key: "ollama", label: "Ollama", matches: (id) => id === "quota-ollama" },
  { key: "meituan", label: "美团 LongCat", matches: (id) => id === "quota-meituan" },
  { key: "grok", label: "Super Grok", matches: (id) => id === "quota-grok" },
];

export function isQuotaCardHidden(
  hiddenCards: Set<string>,
  cardId: string
): boolean {
  return QUOTA_CARD_DEFS.some(
    (d) => d.matches(cardId) && hiddenCards.has(d.key)
  );
}
