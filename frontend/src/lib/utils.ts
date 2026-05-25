export function formatNumber(n: number): string {
  if (n == null || Number.isNaN(n)) return "-";
  if (n >= 1_000_000_000) return (n / 1_000_000_000).toFixed(2) + "B";
  if (n >= 1_000_000) return (n / 1_000_000).toFixed(2) + "M";
  if (n >= 1_000) return (n / 1_000).toFixed(1) + "K";
  return n.toLocaleString();
}

/** Format a call count — always show the full number with locale separators (e.g. 2,620) */
export function formatCalls(n: number): string {
  if (n == null || Number.isNaN(n)) return "-";
  return n.toLocaleString();
}

export function formatCost(cost: number, source?: string): string {
  if (cost == null || Number.isNaN(cost)) return "-";
  // For non-pi sources with zero cost, show N/A
  if (cost === 0 && source && source !== "pi") return "N/A";
  if (cost === 0) return "¥0.00";
  if (cost < 0.01) return "<¥0.01";
  return "¥" + cost.toFixed(2);
}

/** Format average cost per million tokens (元/百万Token).
 *  Formula: costCny / (totalTokens / 1_000_000) */
export function formatAvgCost(costCny: number, totalTokens: number): string {
  if (costCny == null || Number.isNaN(costCny)) return "-";
  if (totalTokens == null || totalTokens <= 0) return "N/A";
  if (costCny === 0) return "¥0.00/百万";
  const avgCost = costCny / (totalTokens / 1_000_000);
  if (avgCost < 0.01) return "<¥0.01/百万";
  return "¥" + avgCost.toFixed(2) + "/百万";
}

export function formatPercent(pct: number): string {
  if (pct == null || Number.isNaN(pct)) return "-";
  return pct.toFixed(1) + "%";
}

export interface CycleCountdown {
  daysRemaining: number;
  isUrgent: boolean;
  text: string;
}

function startOfLocalDay(value: Date): number {
  return new Date(value.getFullYear(), value.getMonth(), value.getDate()).getTime();
}

/** Build Chinese countdown text for the next subscription cycle. */
export function buildCycleCountdown(
  targetDate: Date | string | null | undefined,
  now: Date = new Date()
): CycleCountdown | null {
  if (!targetDate) return null;
  const target = targetDate instanceof Date ? targetDate : new Date(targetDate);
  if (Number.isNaN(target.getTime()) || Number.isNaN(now.getTime())) return null;

  const dayMs = 24 * 60 * 60 * 1000;
  const daysRemaining = Math.max(
    0,
    Math.ceil((startOfLocalDay(target) - startOfLocalDay(now)) / dayMs)
  );

  return {
    daysRemaining,
    isUrgent: daysRemaining < 3,
    text: `距下周期 ${daysRemaining} 天`,
  };
}

export function cycleCountdownTextClass(isUrgent: boolean): string {
  return isUrgent
    ? "text-[10px] font-semibold text-rose-600"
    : "text-[10px] font-medium text-slate-500";
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
  pi: "#2563eb",          // blue-600
  "claude-code": "#fbbf24", // amber-400
  codex: "#34d399",       // emerald-400
  "kimi-cli": "#a78bfa",  // violet-400
  opencode: "#fb923c",    // orange-400
  "xiaomi-mimo-tp": "#f43f5e", // rose-500
};

/** Modern, diverse vendor color palette for charts and UI tags */
export const VENDOR_COLORS: Record<string, string> = {
  deepseek: "#0ea5e9",     // sky-500
  kimi: "#8b5cf6",         // violet-500
  "kimi-coding": "#a78bfa", // violet-400
  ainaba: "#10b981",       // emerald-500
  xunfei: "#f59e0b",       // amber-500
  guancha: "#ec4899",      // pink-500
  "opencode-go": "#f97316", // orange-500
  opencode: "#fb923c",     // orange-400
  "xiaomi-mimo": "#ef4444", // red-500
  anthropic: "#6366f1",    // indigo-500
  openai: "#06b6d4",       // cyan-500
  commandcode: "#f472b6",  // pink-400
};

export function getVendorColor(vendor: string): string {
  return VENDOR_COLORS[vendor] || "#94a3b8";
}

export const SOURCE_LABELS: Record<string, string> = {
  pi: "Pi",
  "claude-code": "Claude Code",
  codex: "Codex",
  "kimi-cli": "Kimi CLI",
  opencode: "OpenCode",
  "xiaomi-mimo-tp": "Xiaomi MiMo TP",
};

export function getSourceColor(source: string): string {
  return SOURCE_COLORS[source] || "#6b7280";
}

export function getSourceLabel(source: string): string {
  return SOURCE_LABELS[source] || source;
}

/** Format a reset time string (ISO 8601 / RFC 3339) into a relative time like "2h 15m 后重置" */
export function formatResetTime(resetTime: string | null | undefined): string | null {
  if (!resetTime) return null;
  try {
    const target = new Date(resetTime);
    if (isNaN(target.getTime())) return null;
    const now = new Date();
    const diffMs = target.getTime() - now.getTime();
    if (diffMs <= 0) return "即将重置";
    const diffMin = Math.floor(diffMs / 60000);
    const hours = Math.floor(diffMin / 60);
    const mins = diffMin % 60;
    if (hours > 24) {
      const days = Math.floor(hours / 24);
      const remainHours = hours % 24;
      return `${days}d ${remainHours}h 后重置`;
    }
    if (hours > 0) {
      return `${hours}h ${mins}m 后重置`;
    }
    return `${mins}m 后重置`;
  } catch {
    return null;
  }
}

/** Compute the next billing/reset date for a monthly subscription
 *  that starts on the given day of month (1-28).
 *  Returns the Date of the next occurrence (could be this month or next). */
export function computeNextBillingDate(
  startDay: number,
  now: Date = new Date()
): Date {
  const today = new Date(now.getFullYear(), now.getMonth(), now.getDate());
  const billingThisMonth = new Date(now.getFullYear(), now.getMonth(), startDay);
  if (billingThisMonth >= today) return billingThisMonth;
  return new Date(now.getFullYear(), now.getMonth() + 1, startDay);
}

/** Check if a date string (ISO 8601 or similar) is within 24 hours from now */
export function isWithin24Hours(dateStr: string): boolean {
  const target = new Date(dateStr);
  if (isNaN(target.getTime())) return false;
  const now = new Date();
  const diffMs = target.getTime() - now.getTime();
  return diffMs > 0 && diffMs <= 24 * 60 * 60 * 1000;
}
