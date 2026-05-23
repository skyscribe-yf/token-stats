import test from "node:test";
import assert from "node:assert/strict";
import {
  buildPivotTree,
  expandDisplayModels,
  getSortValue,
  reconcileSelectedModels,
} from "./pivotTable.ts";
import type { ModelStats, SourceDetailStats } from "../api.ts";

function makeSourceDetail(overrides: Partial<SourceDetailStats> = {}): SourceDetailStats {
  return {
    source: overrides.source ?? "pi",
    calls: overrides.calls ?? 1,
    input_tokens: overrides.input_tokens ?? 10,
    output_tokens: overrides.output_tokens ?? 10,
    cache_read_tokens: overrides.cache_read_tokens ?? 0,
    cache_write_tokens: overrides.cache_write_tokens ?? 0,
    total_tokens: overrides.total_tokens ?? 20,
    cost: overrides.cost ?? 0,
    cache_hit_ratio: overrides.cache_hit_ratio ?? 0,
  };
}

function makeModelStats(overrides: Partial<ModelStats> & { provider?: string; model?: string } = {}): ModelStats {
  const totalTokens = overrides.total_tokens ?? 20;
  const cost = overrides.cost ?? 0;
  const calls = overrides.calls ?? 1;
  const inputTokens = overrides.input_tokens ?? 10;
  const outputTokens = overrides.output_tokens ?? 10;
  const cacheReadTokens = overrides.cache_read_tokens ?? 0;
  const cacheWriteTokens = overrides.cache_write_tokens ?? 0;

  // Build a default source detail that mirrors the model totals so summary
  // recomputation yields the expected values.
  const defaultSourceDetail = makeSourceDetail({
    source: overrides.sources?.[0] ?? "pi",
    total_tokens: totalTokens,
    cost,
    calls,
    input_tokens: inputTokens,
    output_tokens: outputTokens,
    cache_read_tokens: cacheReadTokens,
    cache_write_tokens: cacheWriteTokens,
  });

  return {
    model: overrides.model ?? "gpt-4",
    provider: overrides.provider ?? "openai",
    sources: overrides.sources ?? ["pi"],
    calls,
    input_tokens: inputTokens,
    output_tokens: outputTokens,
    cache_read_tokens: cacheReadTokens,
    cache_write_tokens: cacheWriteTokens,
    total_tokens: totalTokens,
    cost,
    cache_hit_ratio: overrides.cache_hit_ratio ?? 0,
    source_details: overrides.source_details ?? [defaultSourceDetail],
  };
}

test("buildPivotTree groups by vendor then model", () => {
  const stats = [
    makeModelStats({ provider: "openai", model: "gpt-4", total_tokens: 100 }),
    makeModelStats({ provider: "openai", model: "gpt-3", total_tokens: 50 }),
    makeModelStats({ provider: "anthropic", model: "claude", total_tokens: 80 }),
  ];
  const tree = buildPivotTree(stats, "total_tokens", "desc", false);
  assert.equal(tree.length, 2);
  assert.equal(tree[0].provider, "openai");
  assert.equal(tree[0].models.length, 2);
  assert.equal(tree[1].provider, "anthropic");
});

test("buildPivotTree sorts vendors by total_tokens desc by default", () => {
  const stats = [
    makeModelStats({ provider: "b", total_tokens: 50 }),
    makeModelStats({ provider: "a", total_tokens: 100 }),
  ];
  const tree = buildPivotTree(stats, "total_tokens", "desc", false);
  assert.equal(tree[0].provider, "a");
  assert.equal(tree[1].provider, "b");
});

test("buildPivotTree sorts vendors and models by calls asc when configured", () => {
  const stats = [
    makeModelStats({ provider: "a", model: "m1", calls: 10, total_tokens: 100 }),
    makeModelStats({ provider: "a", model: "m2", calls: 5, total_tokens: 200 }),
    makeModelStats({ provider: "b", model: "m1", calls: 20, total_tokens: 50 }),
  ];
  const tree = buildPivotTree(stats, "calls", "asc", false);
  // Vendors sorted by calls asc: a (15) then b (20)
  assert.equal(tree[0].provider, "a");
  assert.equal(tree[1].provider, "b");
  // Models within a sorted by calls asc: m2 (5) then m1 (10)
  assert.equal(tree[0].models[0].model, "m2");
  assert.equal(tree[0].models[1].model, "m1");
});

test("buildPivotTree sorts by name alphabetically", () => {
  const stats = [
    makeModelStats({ provider: "z", model: "m2" }),
    makeModelStats({ provider: "a", model: "m1" }),
    makeModelStats({ provider: "a", model: "m0" }),
  ];
  const tree = buildPivotTree(stats, "name", "asc", false);
  assert.equal(tree[0].provider, "a");
  assert.equal(tree[1].provider, "z");
  // Models within vendor "a" sorted asc: m0, m1
  assert.equal(tree[0].models[0].model, "m0");
  assert.equal(tree[0].models[1].model, "m1");
});

test("buildPivotTree hides free models at source level and recomputes summaries", () => {
  const stats = [
    makeModelStats({
      provider: "openai",
      model: "gpt-4",
      total_tokens: 100,
      cost: 1.0,
      source_details: [
        makeSourceDetail({ source: "pi", total_tokens: 60, cost: 0.6 }),
        makeSourceDetail({ source: "codex", total_tokens: 40, cost: 0 }),
      ],
    }),
  ];
  const tree = buildPivotTree(stats, "total_tokens", "desc", true);
  assert.equal(tree.length, 1);
  assert.equal(tree[0].models.length, 1);
  assert.equal(tree[0].models[0].source_details.length, 1);
  assert.equal(tree[0].models[0].source_details[0].source, "pi");
  // Summary should be recomputed
  assert.equal(tree[0].models[0].summary.total_tokens, 60);
  assert.equal(tree[0].models[0].summary.cost, 0.6);
});

test("buildPivotTree hides free models at model level when all sources are free", () => {
  const stats = [
    makeModelStats({ provider: "openai", model: "gpt-4", cost: 0 }),
    makeModelStats({ provider: "openai", model: "gpt-3", cost: 1.0 }),
  ];
  const tree = buildPivotTree(stats, "total_tokens", "desc", true);
  assert.equal(tree.length, 1);
  assert.equal(tree[0].models.length, 1);
  assert.equal(tree[0].models[0].model, "gpt-3");
});

test("buildPivotTree hides free models at vendor level when all models are free", () => {
  const stats = [
    makeModelStats({ provider: "openai", cost: 0 }),
    makeModelStats({ provider: "anthropic", cost: 1.0 }),
  ];
  const tree = buildPivotTree(stats, "total_tokens", "desc", true);
  assert.equal(tree.length, 1);
  assert.equal(tree[0].provider, "anthropic");
});

test("avg_cost sort uses cost per million tokens", () => {
  const stats = [
    makeModelStats({ provider: "a", model: "m1", total_tokens: 1_000_000, cost: 2.0 }),
    makeModelStats({ provider: "a", model: "m2", total_tokens: 1_000_000, cost: 1.0 }),
  ];
  const tree = buildPivotTree(stats, "avg_cost", "asc", false);
  assert.equal(tree[0].models[0].model, "m2");
  assert.equal(tree[0].models[1].model, "m1");
});

test("getSortValue returns correct values for all columns", () => {
  const summary = {
    calls: 10,
    input_tokens: 100,
    output_tokens: 50,
    cache_read_tokens: 20,
    cache_write_tokens: 5,
    total_tokens: 175,
    cost: 2.5,
    cache_hit_ratio: 16.666,
    output_ratio: 0,
  };
  assert.equal(getSortValue(summary, "calls"), 10);
  assert.equal(getSortValue(summary, "input_tokens"), 100);
  assert.equal(getSortValue(summary, "output_tokens"), 50);
  assert.equal(getSortValue(summary, "cache"), 25);
  assert.equal(getSortValue(summary, "total_tokens"), 175);
  assert.equal(getSortValue(summary, "cost"), 2.5);
  // avg_cost = 2.5 / 175 * 1_000_000
  assert.ok(Math.abs(getSortValue(summary, "avg_cost") as number - (2.5 / 175 * 1_000_000)) < 0.01);
});

test("buildPivotTree sorts source details by the same column", () => {
  const stats = [
    makeModelStats({
      provider: "openai",
      model: "gpt-4",
      source_details: [
        makeSourceDetail({ source: "pi", total_tokens: 30 }),
        makeSourceDetail({ source: "codex", total_tokens: 70 }),
      ],
    }),
  ];
  const tree = buildPivotTree(stats, "total_tokens", "asc", false);
  assert.equal(tree[0].models[0].source_details[0].source, "pi");
  assert.equal(tree[0].models[0].source_details[1].source, "codex");
});

test("buildPivotTree computes output_ratio from output_tokens / total_tokens", () => {
  const stats = [
    makeModelStats({
      provider: "deepseek",
      model: "deepseek-v4-pro",
      total_tokens: 1_000,
      output_tokens: 50,
      input_tokens: 950,
      source_details: [
        makeSourceDetail({
          source: "pi",
          total_tokens: 1_000,
          output_tokens: 50,
          input_tokens: 950,
        }),
      ],
    }),
  ];
  const tree = buildPivotTree(stats, "total_tokens", "desc", false);
  assert.equal(tree[0].models[0].summary.output_ratio, 5);
  assert.equal(tree[0].summary.output_ratio, 5);
});

test("getSortValue returns output_ratio in percent", () => {
  const summary = {
    calls: 1,
    input_tokens: 80,
    output_tokens: 20,
    cache_read_tokens: 0,
    cache_write_tokens: 0,
    total_tokens: 100,
    cost: 0,
    cache_hit_ratio: 0,
    output_ratio: 20,
  };
  assert.equal(getSortValue(summary, "output_ratio"), 20);
});

test("buildPivotTree handles zero total_tokens for output_ratio", () => {
  const stats = [
    makeModelStats({
      provider: "x",
      model: "m",
      total_tokens: 0,
      output_tokens: 0,
      input_tokens: 0,
      source_details: [
        makeSourceDetail({
          source: "pi",
          total_tokens: 0,
          output_tokens: 0,
          input_tokens: 0,
        }),
      ],
    }),
  ];
  const tree = buildPivotTree(stats, "total_tokens", "desc", false);
  assert.equal(tree[0].models[0].summary.output_ratio, 0);
});

test("expandDisplayModels expands merged display models to raw API names", () => {
  const expanded = expandDisplayModels(new Set(["kimi-k2.6", "gpt-4.1"]));
  assert.deepEqual(expanded, [
    "kimi-k2.6",
    "kimi-k2.6:high",
    "kimi-for-coding",
    "gpt-4.1",
  ]);
});

test("reconcileSelectedModels drops models absent from the current visible slice even if globally known", () => {
  const reconciled = reconcileSelectedModels(
    new Set(["kimi-k2.6", "gpt-4.1"]),
    ["gpt-4.1", "claude-sonnet-4"]
  );
  assert.deepEqual([...reconciled], ["gpt-4.1"]);
});
