import { describe, it, expect } from "vitest";
import {
  getPresetRange,
  makeAppliedRange,
  refreshAppliedRangeForPreset,
} from "./timeRange";
import type { AppliedRange } from "./filterState";

describe("getPresetRange", () => {
  it("returns today's date for 'today' preset", () => {
    const now = new Date(2026, 5, 17, 14, 30); // 2026-06-17 14:30 local
    const range = getPresetRange("today", now);
    expect(range.from).toBe("2026-06-17");
    expect(range.to).toBe("2026-06-17");
  });

  it("shifts 'today' after midnight", () => {
    const beforeMidnight = new Date(2026, 5, 17, 23, 59);
    const afterMidnight = new Date(2026, 5, 18, 0, 1);

    const rangeBefore = getPresetRange("today", beforeMidnight);
    const rangeAfter = getPresetRange("today", afterMidnight);

    expect(rangeBefore.from).toBe("2026-06-17");
    expect(rangeAfter.from).toBe("2026-06-18");
  });

  it("shifts '7d' upper bound after midnight", () => {
    const beforeMidnight = new Date(2026, 5, 17, 23, 59);
    const afterMidnight = new Date(2026, 5, 18, 0, 1);

    const rangeBefore = getPresetRange("7d", beforeMidnight);
    const rangeAfter = getPresetRange("7d", afterMidnight);

    expect(rangeBefore.to).toBe("2026-06-17");
    expect(rangeAfter.to).toBe("2026-06-18");
    expect(rangeAfter.from).toBe("2026-06-11");
  });

  it("shifts '6h' bounds after 6 hours", () => {
    const t1 = new Date(2026, 5, 17, 12, 0);
    const t2 = new Date(2026, 5, 17, 18, 0);

    const range1 = getPresetRange("6h", t1);
    const range2 = getPresetRange("6h", t2);

    expect(range1.from).toBe("2026-06-17T06:00");
    expect(range1.to).toBe("2026-06-17T12:00");
    expect(range2.from).toBe("2026-06-17T12:00");
    expect(range2.to).toBe("2026-06-17T18:00");
  });
});

describe("makeAppliedRange", () => {
  it("includes appliedAt timestamp", () => {
    const now = new Date(2026, 5, 17, 14, 30);
    const range = makeAppliedRange("today", now);
    expect(range.from).toBe("2026-06-17");
    expect(range.to).toBe("2026-06-17");
    expect(range.appliedAt).toBeTypeOf("number");
    expect(range.appliedAt).toBeGreaterThan(0);
  });
});

describe("refreshAppliedRangeForPreset", () => {
  it("returns same object when bounds are unchanged", () => {
    const now = new Date(2026, 5, 17, 14, 30);
    const current: AppliedRange = {
      from: "2026-06-17",
      to: "2026-06-17",
      appliedAt: 1000,
    };
    const result = refreshAppliedRangeForPreset("today", current, now);
    expect(result).toBe(current); // same reference
  });

  it("returns new range when bounds have changed (midnight rollover)", () => {
    const beforeMidnight = new Date(2026, 5, 17, 23, 59);
    const afterMidnight = new Date(2026, 5, 18, 0, 1);

    const current: AppliedRange = {
      from: "2026-06-17",
      to: "2026-06-17",
      appliedAt: 1000,
    };

    // Same day → no change
    const sameDay = refreshAppliedRangeForPreset("today", current, beforeMidnight);
    expect(sameDay).toBe(current);

    // New day → new range
    const newDay = refreshAppliedRangeForPreset("today", current, afterMidnight);
    expect(newDay).not.toBe(current);
    expect(newDay.from).toBe("2026-06-18");
    expect(newDay.to).toBe("2026-06-18");
    expect(newDay.appliedAt).toBeGreaterThan(1000);
  });

  it("returns current unchanged for 'custom' preset", () => {
    const now = new Date(2026, 5, 18, 0, 1);
    const current: AppliedRange = {
      from: "2026-06-10",
      to: "2026-06-15",
      appliedAt: 1000,
    };
    const result = refreshAppliedRangeForPreset("custom", current, now);
    expect(result).toBe(current);
  });

  it("updates '7d' range after midnight", () => {
    const afterMidnight = new Date(2026, 5, 18, 0, 1);
    const current: AppliedRange = {
      from: "2026-06-10",
      to: "2026-06-17",
      appliedAt: 1000,
    };
    const result = refreshAppliedRangeForPreset("7d", current, afterMidnight);
    expect(result).not.toBe(current);
    expect(result.to).toBe("2026-06-18");
    expect(result.from).toBe("2026-06-11");
  });

  it("updates 'all' range after midnight", () => {
    const afterMidnight = new Date(2026, 5, 18, 0, 1);
    const current: AppliedRange = {
      from: "2016-06-17",
      to: "2026-06-17",
      appliedAt: 1000,
    };
    const result = refreshAppliedRangeForPreset("all", current, afterMidnight);
    expect(result).not.toBe(current);
    expect(result.to).toBe("2026-06-18");
  });
});
