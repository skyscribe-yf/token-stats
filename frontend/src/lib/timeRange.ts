import {
  getLocalToday,
  getLocalDateOffset,
  getLocalDatetimeOffsetHours,
} from "./utils";
import type { AppliedRange } from "./filterState";

export type TimePreset =
  | "today"
  | "6h"
  | "12h"
  | "1d"
  | "3d"
  | "7d"
  | "14d"
  | "30d"
  | "all"
  | "custom";

export function getPresetRange(
  preset: Exclude<TimePreset, "custom">
): Pick<AppliedRange, "from" | "to"> {
  switch (preset) {
    case "today": {
      const today = getLocalToday();
      return { from: today, to: today };
    }
    case "6h":
      return {
        from: getLocalDatetimeOffsetHours(6),
        to: getLocalDatetimeOffsetHours(0),
      };
    case "12h":
      return {
        from: getLocalDatetimeOffsetHours(12),
        to: getLocalDatetimeOffsetHours(0),
      };
    case "1d":
      return {
        from: getLocalDatetimeOffsetHours(24),
        to: getLocalDatetimeOffsetHours(0),
      };
    case "3d":
      return { from: getLocalDateOffset(3), to: getLocalToday() };
    case "7d":
      return { from: getLocalDateOffset(7), to: getLocalToday() };
    case "14d":
      return { from: getLocalDateOffset(14), to: getLocalToday() };
    case "30d":
      return { from: getLocalDateOffset(30), to: getLocalToday() };
    case "all":
      return { from: getLocalDateOffset(365 * 10), to: getLocalToday() };
  }
}

export function makeAppliedRange(
  preset: Exclude<TimePreset, "custom">
): AppliedRange {
  return { ...getPresetRange(preset), appliedAt: Date.now() };
}

export function makeCustomAppliedRange(from: string, to: string): AppliedRange {
  return { from, to, appliedAt: Date.now() };
}

export function toggleInSet<T>(
  set: ReadonlySet<T>,
  value: T
): Set<T> {
  const next = new Set(set);
  if (next.has(value)) next.delete(value);
  else next.add(value);
  return next;
}
