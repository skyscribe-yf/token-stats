import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}

export function formatNumber(n: number): string {
  return n.toLocaleString("en-US");
}

export function formatTokens(n: number): string {
  if (n >= 1_000_000) return (n / 1_000_000).toFixed(1) + "M";
  if (n >= 1_000) return (n / 1_000).toFixed(1) + "K";
  return n.toLocaleString("en-US");
}

export function formatCost(n: number): string {
  if (n >= 1) return "$" + n.toFixed(2);
  if (n >= 0.01) return "$" + n.toFixed(3);
  return "$" + n.toFixed(6);
}

export function formatPercent(n: number): string {
  return n.toFixed(1) + "%";
}

export function formatDate(dateStr: string): string {
  const d = new Date(dateStr);
  return d.toLocaleDateString("en-US", {
    month: "short",
    day: "numeric",
    year: "numeric",
  });
}

export function formatDateTime(dateStr: string): string {
  const d = new Date(dateStr);
  return d.toLocaleString("en-US", {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

const VENDOR_COLORS: Record<string, string> = {
  opencode: "#3b82f6",
  "opencode-go": "#3b82f6",
  deepseek: "#8b5cf6",
  kimi: "#f59e0b",
  "kimi-coding": "#f59e0b",
  xunfei: "#10b981",
  ainaiba: "#f43f5e",
  guancha: "#06b6d4",
  "xiaomi-mimo": "#f97316",
  default: "#64748b",
};

export function getVendorColor(vendor: string): string {
  return VENDOR_COLORS[vendor] || VENDOR_COLORS.default;
}
