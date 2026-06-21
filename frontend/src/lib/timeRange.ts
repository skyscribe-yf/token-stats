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
  preset: Exclude<TimePreset, "custom">,
  now: Date = new Date()
): Pick<AppliedRange, "from" | "to"> {
  switch (preset) {
    case "today": {
      const today = getLocalToday(now);
      return { from: today, to: today };
    }
    case "6h":
      return {
        from: getLocalDatetimeOffsetHours(6, now),
        to: getLocalDatetimeOffsetHours(0, now),
      };
    case "12h":
      return {
        from: getLocalDatetimeOffsetHours(12, now),
        to: getLocalDatetimeOffsetHours(0, now),
      };
    case "1d":
      return {
        from: getLocalDatetimeOffsetHours(24, now),
        to: getLocalDatetimeOffsetHours(0, now),
      };
    case "3d":
      return { from: getLocalDateOffset(3, now), to: getLocalToday(now) };
    case "7d":
      return { from: getLocalDateOffset(7, now), to: getLocalToday(now) };
    case "14d":
      return { from: getLocalDateOffset(14, now), to: getLocalToday(now) };
    case "30d":
      return { from: getLocalDateOffset(30, now), to: getLocalToday(now) };
    case "all":
      return { from: getLocalDateOffset(365 * 10, now), to: getLocalToday(now) };
  }
}

export function makeAppliedRange(
  preset: Exclude<TimePreset, "custom">,
  now: Date = new Date()
): AppliedRange {
  return { ...getPresetRange(preset, now), appliedAt: Date.now() };
}

export function makeCustomAppliedRange(from: string, to: string): AppliedRange {
  return { from, to, appliedAt: Date.now() };
}

/**
 * Refresh an applied range for a preset if the computed bounds have changed
 * (e.g. after midnight, "today" should shift to the new date).
 * Returns the same object if bounds are unchanged to avoid unnecessary re-renders.
 * For "custom" preset, always returns the current range unchanged.
 */
export function refreshAppliedRangeForPreset(
  preset: TimePreset,
  current: AppliedRange,
  now: Date = new Date()
): AppliedRange {
  if (preset === "custom") return current;

  const fresh = getPresetRange(preset, now);
  if (fresh.from === current.from && fresh.to === current.to) return current;

  return { ...fresh, appliedAt: Date.now() };
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
