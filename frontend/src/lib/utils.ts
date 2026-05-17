export function formatNumber(n: number): string {
  if (n >= 1_000_000_000) return (n / 1_000_000_000).toFixed(2) + "B";
  if (n >= 1_000_000) return (n / 1_000_000).toFixed(2) + "M";
  if (n >= 1_000) return (n / 1_000).toFixed(1) + "K";
  return n.toLocaleString();
}

export function formatCost(cost: number, source?: string): string {
  // For non-pi sources with zero cost, show N/A
  if (cost === 0 && source && source !== "pi") return "N/A";
  if (cost === 0) return "$0.00";
  if (cost < 0.01) return "<$0.01";
  return "$" + cost.toFixed(2);
}

export function formatPercent(pct: number): string {
  return pct.toFixed(1) + "%";
}

export function formatDate(dateStr: string): string {
  return dateStr;
}

export const SOURCE_COLORS: Record<string, string> = {
  pi: "#3b82f6",          // blue
  "claude-code": "#f59e0b", // amber
  codex: "#10b981",       // emerald
  "kimi-cli": "#8b5cf6",  // violet
};

export const SOURCE_LABELS: Record<string, string> = {
  pi: "Pi",
  "claude-code": "Claude Code",
  codex: "Codex",
  "kimi-cli": "Kimi CLI",
};

export function getSourceColor(source: string): string {
  return SOURCE_COLORS[source] || "#6b7280";
}

export function getSourceLabel(source: string): string {
  return SOURCE_LABELS[source] || source;
}