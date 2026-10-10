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
  "quota-kimi",
  "quota-meituan",
];

test("default-hides xunfei, both OpenCode-go, both Fenno, Kimi and LongCat", () => {
  const hidden = seedHiddenQuotaCards(null, 0);
  for (const cardId of HIDDEN_CARD_IDS) {
    assert.equal(isQuotaCardHidden(hidden, cardId), true, cardId);
  }
  assert.equal(isQuotaCardHidden(hidden, "quota-ollama"), false);
  assert.equal(isQuotaCardHidden(hidden, "quota-grok"), false);
});

test("one-time migration unions defaults into an existing saved set", () => {
  // NaN stands in for a corrupt stored version.
  const hidden = seedHiddenQuotaCards(["ollama"], Number.NaN);
  assert.equal(hidden.has("ollama"), true);
  for (const cardId of HIDDEN_CARD_IDS) {
    assert.equal(isQuotaCardHidden(hidden, cardId), true, cardId);
  }
});

test("a browser already on the previous version picks up the new defaults", () => {
  const hidden = seedHiddenQuotaCards(["xunfei"], HIDDEN_QUOTA_CARDS_VERSION - 1);
  for (const cardId of HIDDEN_CARD_IDS) {
    assert.equal(isQuotaCardHidden(hidden, cardId), true, cardId);
  }
});

test("an explicit unhide survives after the migration version is current", () => {
  const hidden = seedHiddenQuotaCards(["xunfei"], HIDDEN_QUOTA_CARDS_VERSION);
  // One key covers both 讯飞 cards.
  assert.equal(isQuotaCardHidden(hidden, "quota-xunfei-primary"), true);
  assert.equal(isQuotaCardHidden(hidden, "quota-xunfei-ex"), true);
  for (const cardId of HIDDEN_CARD_IDS.filter((id) => !id.startsWith("quota-xunfei-"))) {
    assert.equal(isQuotaCardHidden(hidden, cardId), false, cardId);
  }
});
