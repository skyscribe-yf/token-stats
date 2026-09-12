import test from "node:test";
import assert from "node:assert/strict";

import { getSourceLabel } from "./utils.ts";
import { remainingQuota } from "./fennoQuota.ts";

test("labels recorded Grok usage", () => {
  assert.equal(getSourceLabel("grok-cli"), "Grok CLI");
});

test("labels the CPA Ollama channel by upstream, not by proxy binary", () => {
  // The source id names the transport; the label names what it carries. An
  // unlabelled source would render its raw id ("ollama-proxy") in the UI.
  assert.equal(getSourceLabel("ollama-proxy"), "Dim→Ollama");
});

test("calculates non-negative Fenno quota remaining", () => {
  assert.equal(remainingQuota(38, 4.5), 33.5);
  assert.equal(remainingQuota(10, 12), 0);
  assert.equal(remainingQuota(null, 12), null);
});

test("does not auto-hide a configured quota card when the fetch fails", async () => {
  const { hideUnavailableQuotaCard } = await import("./quotaCards.ts");
  // Expired cookies / 401 must leave the card visible with an error,
  // otherwise the subscription looks like it vanished.
  assert.equal(hideUnavailableQuotaCard(false), false);
  assert.equal(hideUnavailableQuotaCard(undefined), false);
  assert.equal(hideUnavailableQuotaCard(true), true);
});
