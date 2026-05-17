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

/** Format a date string (e.g. "2025-05-17") for display – keeps as-is since it's just a date */
export function formatDate(dateStr: string): string {
  return dateStr;
}

/** Format a UTC RFC3339 timestamp to local time string for display */
export function formatTime(utcTimeStr: string): string {
  if (!utcTimeStr || utcTimeStr === "unknown") return utcTimeStr;
  try {
    const d = new Date(utcTimeStr);
    if (isNaN(d.getTime())) return utcTimeStr;
    // Format as local datetime: "2025-05-17 16:30:00"
    const pad = (n: number) => String(n).padStart(2, "0");
    return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
  } catch {
    return utcTimeStr;
  }
}

/** Get today's date in local timezone as YYYY-MM-DD */
export function getLocalToday(): string {
  const d = new Date();
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

/** Get a Date offset by `days` from now, formatted as YYYY-MM-DD in local timezone */
export function getLocalDateOffset(days: number): string {
  const d = new Date();
  d.setDate(d.getDate() - days);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

/** Get a datetime-local value (YYYY-MM-DDTHH:mm) offset by `days` from now in local timezone */
export function getLocalDatetimeOffset(days: number): string {
  const d = new Date();
  d.setDate(d.getDate() - days);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

/** Get a datetime-local value (YYYY-MM-DDTHH:mm) offset by `hours` from now in local timezone */
export function getLocalDatetimeOffsetHours(hours: number): string {
  const d = new Date();
  d.setHours(d.getHours() - hours);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

export const SOURCE_COLORS: Record<string, string> = {
  pi: "#3b82f6",          // blue
  "claude-code": "#f59e0b", // amber
  codex: "#10b981",       // emerald
  "kimi-cli": "#8b5cf6",  // violet
  opencode: "#f97316",    // orange
};

export const SOURCE_LABELS: Record<string, string> = {
  pi: "Pi",
  "claude-code": "Claude Code",
  codex: "Codex",
  "kimi-cli": "Kimi CLI",
  opencode: "OpenCode",
};

export function getSourceColor(source: string): string {
  return SOURCE_COLORS[source] || "#6b7280";
}

export function getSourceLabel(source: string): string {
  return SOURCE_LABELS[source] || source;
}