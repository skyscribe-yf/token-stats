import test from "node:test";
import assert from "node:assert/strict";

import {
  buildCsvFilterParam,
  formatAppliedRange,
  isEmptyAppliedSelection,
} from "./filterState.ts";

test("buildCsvFilterParam omits all-selected filters", () => {
  assert.equal(
    buildCsvFilterParam(new Set(["pi", "codex"]), ["codex", "pi"]),
    undefined
  );
});

test("buildCsvFilterParam sends selected subsets as stable CSV", () => {
  assert.equal(
    buildCsvFilterParam(new Set(["pi", "codex"]), ["claude-code", "codex", "pi"]),
    "codex,pi"
  );
});

test("empty selected sets are treated as intentionally empty after options load", () => {
  assert.equal(isEmptyAppliedSelection(new Set(), ["pi"]), true);
  assert.equal(isEmptyAppliedSelection(new Set(), []), false);
});

test("formatAppliedRange includes preset label and concrete bounds", () => {
  assert.equal(
    formatAppliedRange("最近6小时", {
      from: "2026-05-17T14:30",
      to: "2026-05-17T20:30",
    }),
    "最近6小时 · 2026-05-17 14:30 至 2026-05-17 20:30"
  );
});
