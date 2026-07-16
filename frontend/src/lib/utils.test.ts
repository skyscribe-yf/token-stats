import test from "node:test";
import assert from "node:assert/strict";

import { getSourceLabel } from "./utils.ts";
import { remainingQuota } from "./fennoQuota.ts";

test("labels recorded Grok usage", () => {
  assert.equal(getSourceLabel("grok-cli"), "Grok CLI");
});

test("calculates non-negative Fenno quota remaining", () => {
  assert.equal(remainingQuota(38, 4.5), 33.5);
  assert.equal(remainingQuota(10, 12), 0);
  assert.equal(remainingQuota(null, 12), null);
});
