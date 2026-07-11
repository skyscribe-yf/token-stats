import test from "node:test";
import assert from "node:assert/strict";

import { getSourceLabel } from "./utils.ts";

test("labels recorded Grok usage", () => {
  assert.equal(getSourceLabel("grok-cli"), "Grok CLI");
});
