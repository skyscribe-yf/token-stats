import assert from "node:assert/strict";
import test from "node:test";

import {
  buildCycleCountdown,
  computeNextBillingDate,
  cycleCountdownTextClass,
} from "./utils.ts";

test("buildCycleCountdown returns Chinese calendar days-until-next-cycle text", () => {
  const now = new Date(2026, 4, 20, 10);
  const target = new Date(2026, 4, 25);

  const result = buildCycleCountdown(target, now);

  assert.deepEqual(result, {
    daysRemaining: 5,
    isUrgent: false,
    text: "距下周期 5 天",
  });
});

test("buildCycleCountdown marks fewer than three remaining days as urgent", () => {
  const now = new Date(2026, 4, 20, 10);
  const target = new Date(2026, 4, 22, 23, 59, 59);

  const result = buildCycleCountdown(target, now);

  assert.equal(result?.daysRemaining, 2);
  assert.equal(result?.isUrgent, true);
  assert.equal(result?.text, "距下周期 2 天");
});

test("buildCycleCountdown treats elapsed cycle dates as zero days and urgent", () => {
  const now = new Date(2026, 4, 20, 10);
  const target = new Date(2026, 4, 20, 9);

  const result = buildCycleCountdown(target, now);

  assert.deepEqual(result, {
    daysRemaining: 0,
    isUrgent: true,
    text: "距下周期 0 天",
  });
});

test("buildCycleCountdown returns null for missing or invalid dates", () => {
  assert.equal(buildCycleCountdown(null), null);
  assert.equal(buildCycleCountdown("not-a-date"), null);
});

test("computeNextBillingDate returns this month's cycle when upcoming", () => {
  const now = new Date(2026, 4, 10, 10);

  const result = computeNextBillingDate(15, now);

  assert.equal(result.getFullYear(), 2026);
  assert.equal(result.getMonth(), 4);
  assert.equal(result.getDate(), 15);
});

test("computeNextBillingDate returns today when the cycle day is today", () => {
  const now = new Date(2026, 4, 15, 23, 59);

  const result = computeNextBillingDate(15, now);

  assert.equal(result.getFullYear(), 2026);
  assert.equal(result.getMonth(), 4);
  assert.equal(result.getDate(), 15);
});

test("computeNextBillingDate rolls to next month after this month's cycle", () => {
  const now = new Date(2026, 4, 20, 10);

  const result = computeNextBillingDate(15, now);

  assert.equal(result.getFullYear(), 2026);
  assert.equal(result.getMonth(), 5);
  assert.equal(result.getDate(), 15);
});

test("computeNextBillingDate supports the configured 1 to 28 day range", () => {
  const now = new Date(2026, 0, 29, 10);

  const first = computeNextBillingDate(1, now);
  const twentyEighth = computeNextBillingDate(28, now);

  assert.equal(first.getMonth(), 1);
  assert.equal(first.getDate(), 1);
  assert.equal(twentyEighth.getMonth(), 1);
  assert.equal(twentyEighth.getDate(), 28);
});

test("cycleCountdownTextClass highlights urgent countdowns in red", () => {
  assert.match(cycleCountdownTextClass(true), /text-rose-600/);
  assert.doesNotMatch(cycleCountdownTextClass(false), /text-rose-600/);
});
