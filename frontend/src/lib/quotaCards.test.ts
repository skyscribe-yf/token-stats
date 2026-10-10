import test from "node:test";
import assert from "node:assert/strict";

import {
  HIDDEN_QUOTA_CARDS_VERSION,
  isQuotaCardHidden,
  seedHiddenQuotaCards,
} from "./quotaCards.ts";

const HIDDEN_CARD_IDS = [
  "quota-xunfei-primary",
  "quota-xunfei-ex",
  "quota-opencode-primary",
  "quota-opencode-ex",
  "quota-fenno",
  "quota-fenno-ex",
];

test("default-hides xunfei, both OpenCode-go, and both Fenno cards", () => {
  const hidden = seedHiddenQuotaCards(null, 0);
  for (const cardId of HIDDEN_CARD_IDS) {
    assert.equal(isQuotaCardHidden(hidden, cardId), true, cardId);
  }
  assert.equal(isQuotaCardHidden(hidden, "quota-kimi"), false);
  assert.equal(isQuotaCardHidden(hidden, "quota-ollama"), false);
});

test("one-time migration unions defaults into an existing saved set", () => {
  // NaN stands in for a corrupt stored version.
  const hidden = seedHiddenQuotaCards(["kimi"], Number.NaN);
  assert.equal(hidden.has("kimi"), true);
  assert.equal(hidden.has("fenno"), true);
  assert.equal(hidden.has("fenno-ex"), true);
  assert.equal(hidden.has("opencode"), true);
  assert.equal(hidden.has("opencode-ex"), true);
  assert.equal(hidden.has("xunfei"), true);
});

test("an explicit unhide survives after the migration version is current", () => {
  const hidden = seedHiddenQuotaCards(["xunfei"], HIDDEN_QUOTA_CARDS_VERSION);
  assert.equal(hidden.has("xunfei"), true);
  assert.equal(hidden.has("fenno"), false);
  assert.equal(hidden.has("fenno-ex"), false);
  assert.equal(hidden.has("opencode"), false);
  assert.equal(hidden.has("opencode-ex"), false);
});
