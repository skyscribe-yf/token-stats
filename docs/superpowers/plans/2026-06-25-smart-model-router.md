# Smart Model Router Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a global Pi skill plus read-only router script that recommends safe, quota-aware model configurations for Pi subagents based on a user trigger.

**Architecture:** The router is a Node.js ESM CLI/module with pure scoring functions and small impure gather/apply adapters. It treats `pi --list-models` as the authoritative selectable-model registry, combines that with token-stats API quota/cost/status snapshots and current Pi settings, then emits recommendations and a settings merge patch. Apply mode is explicitly gated, backed up, hash-checked, and limited to `subagents.agentOverrides`.

**Tech Stack:** Node.js 24 ESM, built-in `node:test`, built-in `fetch`, Pi global skills, token-stats local HTTP API.

---

## Source References

- Token-stats routes live in `backend/src/app.rs:124` through `backend/src/app.rs:140` and expose `/api/stats`, `/api/quota`, `/api/xunfei`, `/api/pricing`, `/api/ainaiba-credit`, and settings endpoints.
- Current Pi subagent overrides live in `/home/skyscribe/.pi/agent/settings.json:22` under `subagents.agentOverrides` and are the current source of truth.
- Cost display logic lives in `backend/src/pricing.rs:611` and should inform, not be duplicated perfectly, in the first router implementation.
- Quota response types live in `backend/src/quota/types.rs:170`.
- Model normalization patterns live in `backend/src/sources/mod.rs:110`.

## File Map

- Create: `/home/skyscribe/.pi/agent/skills/smart-model-router/scripts/model-router.mjs`
  - CLI entry point and importable pure functions.
  - No external npm dependencies.
  - Defaults to dry-run and JSON-safe redacted output.
- Create: `/home/skyscribe/.pi/agent/skills/smart-model-router/tests/model-router.test.mjs`
  - Unit tests using `node:test` and fixture constants inline unless fixture bulk grows too large.
- Create: `/home/skyscribe/.pi/agent/skills/smart-model-router/SKILL.md`
  - Pi skill wrapping the script, trigger conditions, safety rules, and examples.
- Optional create: `/home/skyscribe/.pi/agent/skills/smart-model-router/fixtures/*.json`
  - Only if inline fixtures make the test file hard to read.
- Do not modify by default: `/home/skyscribe/.pi/agent/settings.json`
  - Real settings are only touched with `--apply` after explicit user approval.

## Router Contracts

### CLI Flags

```text
--trigger <plan|implement|review|research|scout|quick|long-context|image|cheap|premium|emergency>
--dry-run              Default behavior; do not write settings.
--json                 Emit machine-readable JSON only.
--apply                Write the computed patch to settings after safety checks.
--settings <path>      Override settings path; required for apply tests.
--models-json <path>   Override models.json path.
--token-stats-url <url>  Default: http://127.0.0.1:3000
--pi-bin <path|name>   Default: pi
--all-roles            Recommend for every known role instead of trigger-active roles only.
```

### Output Shape

```json
{
  "trigger": "review",
  "generatedAt": "2026-06-25T00:00:00.000Z",
  "warnings": [
    "Configured model commandcodego/moonshotai/Kimi-K2.6 is not selectable by pi --list-models"
  ],
  "activeRoles": ["reviewer", "spec-reviewer", "code-quality-reviewer", "reviewers.security-reviewer"],
  "recommendations": {
    "reviewer": {
      "model": "xunfei-ex/xopglm51",
      "thinking": "high",
      "fallbackModels": ["freemodel/gpt-5.4", "kimi-coding/kimi-for-coding"],
      "reasons": ["selectable", "quota headroom", "independent from worker"]
    }
  },
  "settingsPatch": {
    "subagents": {
      "agentOverrides": {}
    }
  }
}
```

### Safety Rules

- Validate every recommended `provider/model` against parsed `pi --list-models` rows.
- Never print secrets or PII from quota, auth, callback, cookie, token, email, or login URL fields.
- Treat missing token-stats API as degraded mode, not a crash, unless `--apply` is requested and no quota data exists.
- `--apply` writes only `subagents.agentOverrides`, creates a timestamped backup, preserves all unrelated settings, checks the original file hash before writing, and refuses invalid model IDs.
- Live validation uses dry-run only. Do not apply to real `/home/skyscribe/.pi/agent/settings.json` during implementation.

---

### Task 1: Router Unit Tests

**Files:**
- Create: `/home/skyscribe/.pi/agent/skills/smart-model-router/tests/model-router.test.mjs`
- Test target: `/home/skyscribe/.pi/agent/skills/smart-model-router/scripts/model-router.mjs`

- [ ] **Step 1: Create the test directory**

Run:

```bash
mkdir -p /home/skyscribe/.pi/agent/skills/smart-model-router/tests
```

Expected: command exits 0.

- [ ] **Step 2: Write failing tests first**

Create `/home/skyscribe/.pi/agent/skills/smart-model-router/tests/model-router.test.mjs` with this content:

```javascript
import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { tmpdir } from 'node:os';

import {
  applySettingsPatch,
  buildRecommendations,
  buildSettingsPatch,
  evaluateProviderHealth,
  hashText,
  parseModelList,
  rankCandidatesForRole,
  redactSecrets,
} from '../scripts/model-router.mjs';

const MODEL_LIST = `provider        model                         context  max-out  thinking  images
ainaiba         gpt-5.4                       1.1M     128K     yes       yes
freemodel       gpt-5.4                       1.1M     128K     yes       yes
kimi-coding     kimi-for-coding               262.1K   32K      yes       yes
opencode-go     deepseek-v4-pro               1M       384K     yes       no
opencode-go     mimo-v2.5-pro                 1.0M     128K     yes       no
openmodel       deepseek-v4-flash             1.0M     32.8K    yes       no
xunfei-ex       xopglm51                      204.8K   32.8K    yes       no
`;

const CURRENT_SETTINGS = {
  defaultProvider: 'ainaiba',
  defaultModel: 'gpt-5.4',
  theme: 'dark',
  subagents: {
    agentOverrides: {
      worker: {
        model: 'opencode-go/mimo-v2.5-pro',
        thinking: 'high',
        fallbackModels: ['opencode-go/deepseek-v4-pro', 'kimi-coding/kimi-for-coding'],
      },
      reviewer: {
        model: 'opencode-go/mimo-v2.5-pro',
        thinking: 'high',
        fallbackModels: ['commandcodego/moonshotai/Kimi-K2.6'],
      },
    },
  },
};

const MODELS_JSON = {
  providers: {
    ainaiba: {
      models: [
        { id: 'gpt-5.4', reasoning: true, input: ['text', 'image'], contextWindow: 1050000, maxTokens: 128000, cost: { input: 2.5, output: 15, cacheRead: 0.25, cacheWrite: 0 } },
      ],
    },
    freemodel: {
      models: [
        { id: 'gpt-5.4', reasoning: true, input: ['text', 'image'], contextWindow: 1050000, maxTokens: 128000, cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 } },
      ],
    },
  },
};

const QUOTA = {
  kimi: { available: true, data: { weekly_limit: 100, weekly_remaining: 24, rp5h_limit: 100, rp5h_remaining: 100 }, error: null },
  opencode_go: { available: true, data: { entries: [
    { usage_type: 'Rolling', percentage: 0 },
    { usage_type: 'Weekly', percentage: 0 },
    { usage_type: 'Monthly', percentage: 100 },
  ] }, error: null },
  xiaomi_mimo: { available: false, data: null, error: 'HTTP 401 Unauthorized: loginUrl=https://example.invalid/login?token=secret' },
};

const XUNFEI = {
  accounts: [
    { label: 'primary', available: true, data: { status: 'expired', usage: { package_left: 0, package_limit: 90000, rp5h_used: 0, rp5h_limit: 6000, rpw_used: 0, rpw_limit: 45000 } }, error: null },
    { label: 'ex', available: true, data: { status: 'active', usage: { package_left: 13072, package_limit: 18000, rp5h_used: 0, rp5h_limit: 1200, rpw_used: 4452, rpw_limit: 9000 } }, error: null },
  ],
};

const AINAIBA = {
  available: true,
  data: { balance: 1279.5, hard_limit: 120000, daily_limit: 4600, daily_used: 54, email: 'person@example.com', expires_at: '2099-01-01T00:00:00+08:00' },
  error: null,
};

test('parseModelList parses selectable models and token units', () => {
  const models = parseModelList(MODEL_LIST);
  assert.equal(models.length, 7);
  assert.deepEqual(models[0], {
    provider: 'ainaiba',
    model: 'gpt-5.4',
    ref: 'ainaiba/gpt-5.4',
    contextTokens: 1100000,
    maxOutputTokens: 128000,
    thinking: true,
    images: true,
  });
  assert.equal(models.find((m) => m.ref === 'xunfei-ex/xopglm51').contextTokens, 204800);
  assert.equal(models.find((m) => m.ref === 'openmodel/deepseek-v4-flash').maxOutputTokens, 32800);
});

test('evaluateProviderHealth marks exhausted and unavailable quota sources', () => {
  const health = evaluateProviderHealth({ quota: QUOTA, xunfei: XUNFEI, ainaiba: AINAIBA });
  assert.equal(health.get('opencode-go').available, false);
  assert.match(health.get('opencode-go').reason, /Monthly quota exhausted/);
  assert.equal(health.get('xunfei-ex').available, true);
  assert.ok(health.get('xunfei-ex').headroom > 0.5);
  assert.equal(health.get('xunfei').available, false);
  assert.equal(health.get('xiaomi-mimo-tp').available, false);
  assert.equal(health.get('ainaiba').available, true);
});

test('rankCandidatesForRole excludes invalid and exhausted providers', () => {
  const models = parseModelList(MODEL_LIST);
  const health = evaluateProviderHealth({ quota: QUOTA, xunfei: XUNFEI, ainaiba: AINAIBA });
  const ranked = rankCandidatesForRole({
    role: 'worker',
    trigger: 'implement',
    models,
    providerHealth: health,
    modelsJson: MODELS_JSON,
    settings: CURRENT_SETTINGS,
  });
  assert.ok(ranked.length > 0);
  assert.notEqual(ranked[0].ref, 'opencode-go/mimo-v2.5-pro');
  assert.ok(!ranked.some((candidate) => candidate.ref.startsWith('opencode-go/')));
  assert.ok(!ranked.some((candidate) => candidate.ref === 'commandcodego/moonshotai/Kimi-K2.6'));
});

test('buildRecommendations warns about configured refs not selectable by pi', () => {
  const result = buildRecommendations({
    trigger: 'review',
    models: parseModelList(MODEL_LIST),
    quota: QUOTA,
    xunfei: XUNFEI,
    ainaiba: AINAIBA,
    modelsJson: MODELS_JSON,
    settings: CURRENT_SETTINGS,
    generatedAt: '2026-06-25T00:00:00.000Z',
  });
  assert.match(result.warnings.join('\n'), /commandcodego\/moonshotai\/Kimi-K2\.6/);
  assert.doesNotMatch(JSON.stringify(result.settingsPatch), /commandcodego/);
  assert.deepEqual(result.activeRoles, ['reviewer', 'spec-reviewer', 'code-quality-reviewer', 'reviewers.security-reviewer']);
});

test('review recommendations prefer provider diversity from worker primary', () => {
  const result = buildRecommendations({
    trigger: 'review',
    models: parseModelList(MODEL_LIST),
    quota: QUOTA,
    xunfei: XUNFEI,
    ainaiba: AINAIBA,
    modelsJson: MODELS_JSON,
    settings: CURRENT_SETTINGS,
    generatedAt: '2026-06-25T00:00:00.000Z',
  });
  const reviewer = result.recommendations.reviewer;
  assert.ok(reviewer.model);
  assert.notEqual(reviewer.model, CURRENT_SETTINGS.subagents.agentOverrides.worker.model);
  assert.ok(reviewer.reasons.some((reason) => /independent|divers/i.test(reason)));
});

test('buildSettingsPatch only targets subagents.agentOverrides', () => {
  const patch = buildSettingsPatch({
    reviewer: { model: 'xunfei-ex/xopglm51', thinking: 'high', fallbackModels: ['freemodel/gpt-5.4'], reasons: [] },
  });
  assert.deepEqual(Object.keys(patch), ['subagents']);
  assert.deepEqual(Object.keys(patch.subagents), ['agentOverrides']);
  assert.equal(patch.subagents.agentOverrides.reviewer.model, 'xunfei-ex/xopglm51');
});

test('redactSecrets removes emails, tokens, cookies, auth, and login URLs', () => {
  const redacted = redactSecrets({
    email: 'person@example.com',
    access_token: 'abc123',
    authCookie: 'cookie-value',
    loginUrl: 'https://example.invalid/login?token=secret',
    nested: { message: 'HTTP 401 Unauthorized: callback=https://example.invalid/cb?code=secret' },
  });
  const text = JSON.stringify(redacted);
  assert.doesNotMatch(text, /person@example\.com/);
  assert.doesNotMatch(text, /abc123/);
  assert.doesNotMatch(text, /cookie-value/);
  assert.doesNotMatch(text, /token=secret/);
  assert.match(text, /\[REDACTED/);
});

test('applySettingsPatch writes a backup, preserves unrelated keys, and checks hash', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'smart-model-router-'));
  try {
    const settingsPath = join(dir, 'settings.json');
    const original = JSON.stringify(CURRENT_SETTINGS, null, 2) + '\n';
    await writeFile(settingsPath, original);
    const patch = buildSettingsPatch({
      reviewer: { model: 'xunfei-ex/xopglm51', thinking: 'high', fallbackModels: ['freemodel/gpt-5.4'], reasons: [] },
    });
    const result = await applySettingsPatch({
      settingsPath,
      patch,
      expectedHash: hashText(original),
      now: new Date('2026-06-25T00:00:00.000Z'),
      validModelRefs: new Set(parseModelList(MODEL_LIST).map((model) => model.ref)),
    });
    assert.equal(result.wrote, true);
    assert.ok(result.backupPath.endsWith('settings.json.20260625T000000Z.bak'));
    const updated = JSON.parse(await readFile(settingsPath, 'utf8'));
    assert.equal(updated.theme, 'dark');
    assert.equal(updated.subagents.agentOverrides.worker.model, 'opencode-go/mimo-v2.5-pro');
    assert.equal(updated.subagents.agentOverrides.reviewer.model, 'xunfei-ex/xopglm51');

    await assert.rejects(
      () => applySettingsPatch({
        settingsPath,
        patch,
        expectedHash: 'not-the-current-hash',
        now: new Date('2026-06-25T00:00:00.000Z'),
        validModelRefs: new Set(parseModelList(MODEL_LIST).map((model) => model.ref)),
      }),
      /settings file changed/i,
    );
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
});
```

Expected: file is created.

- [ ] **Step 3: Verify tests fail for the expected reason**

Run:

```bash
node --test /home/skyscribe/.pi/agent/skills/smart-model-router/tests/model-router.test.mjs
```

Expected: FAIL with an `ERR_MODULE_NOT_FOUND` or missing export error for `../scripts/model-router.mjs`. If it passes, stop because production code already exists and the RED phase was not valid.

---

### Task 2: Router Pure Functions

**Files:**
- Create: `/home/skyscribe/.pi/agent/skills/smart-model-router/scripts/model-router.mjs`
- Test: `/home/skyscribe/.pi/agent/skills/smart-model-router/tests/model-router.test.mjs`

- [ ] **Step 1: Create the scripts directory**

Run:

```bash
mkdir -p /home/skyscribe/.pi/agent/skills/smart-model-router/scripts
```

Expected: command exits 0.

- [ ] **Step 2: Implement minimal pure functions and exports**

Create `/home/skyscribe/.pi/agent/skills/smart-model-router/scripts/model-router.mjs` with these public exports and behavior:

Required public exports:

```text
parseModelList(text)
evaluateProviderHealth({ quota, xunfei, ainaiba })
rankCandidatesForRole({ role, trigger, models, providerHealth, modelsJson, settings })
buildRecommendations({ trigger, models, quota, xunfei, ainaiba, modelsJson, settings, generatedAt, allRoles })
buildSettingsPatch(recommendations)
redactSecrets(value)
hashText(text)
applySettingsPatch({ settingsPath, patch, expectedHash, now, validModelRefs })
```

Implementation rules:

```text
parseModelList:
- split non-empty rows after header on two-or-more spaces
- support token units K and M, including decimals
- return provider, model, ref, contextTokens, maxOutputTokens, thinking, images

evaluateProviderHealth:
- opencode-go unavailable if any Rolling/Weekly/Monthly entry is 100 percent or higher
- xunfei maps primary account to xunfei and ex account to xunfei-ex
- xunfei account unavailable if available=false, status=expired, package_left<=0, or rp5h/rpw are exhausted
- kimi-coding maps to quota.kimi; unavailable if weekly or rp5h remaining <=0
- xiaomi-mimo and xiaomi-mimo-tp map to quota.xiaomi_mimo
- ainaiba maps to /api/ainaiba-credit data and is unavailable if balance<=0 or expired
- unknown providers default to available=true, headroom=0.5, reason='quota unknown'

rankCandidatesForRole:
- reject candidates whose provider health is unavailable
- reject image-less candidates when role/trigger needs image support
- reject small-context candidates for long-context/premium/planner/oracle/researcher needs
- score quota headroom, cost, context, thinking, image fit, and provider diversity
- prefer free/cheap providers for scout/delegate/cheap/quick triggers
- prefer stronger context/thinking for planner/oracle/reviewer/security/research roles

buildRecommendations:
- compute activeRoles from trigger
- warn for configured model/fallback refs absent from model list
- create recommendation entries with model, thinking, fallbackModels, reasons
- build settingsPatch from the recommended entries
- redact warnings and raw health details before output

applySettingsPatch:
- read current settings text and verify hash matches expectedHash
- validate model and fallback refs against validModelRefs
- copy settings to settings.json.YYYYMMDDTHHMMSSZ.bak
- merge only patch.subagents.agentOverrides into current settings
- preserve all unrelated keys and existing role fields not present in patch role value
- write pretty JSON with trailing newline
```

- [ ] **Step 3: Run unit tests**

Run:

```bash
node --test /home/skyscribe/.pi/agent/skills/smart-model-router/tests/model-router.test.mjs
```

Expected: PASS for all tests added in Task 1.

- [ ] **Step 4: Refactor while green**

Refactor into small private helpers inside the same file only if tests stay green:

```javascript
function parseTokenCount(value) {}
function providerForQuota(provider) {}
function getModelConfig(modelsJson, provider, model) {}
function targetRolesForTrigger(trigger, allRoles = false) {}
function roleNeeds(role, trigger) {}
function mergeAgentOverrides(settings, patch) {}
```

Run the unit test command again after refactor. Expected: PASS.

---

### Task 3: CLI Gathering and Dry-Run Output

**Files:**
- Modify: `/home/skyscribe/.pi/agent/skills/smart-model-router/scripts/model-router.mjs`
- Test: `/home/skyscribe/.pi/agent/skills/smart-model-router/tests/model-router.test.mjs`

- [ ] **Step 1: Add CLI-focused tests**

Append tests that call an exported `runCli(argv, deps)` with fake dependencies:

```javascript
import { runCli } from '../scripts/model-router.mjs';

test('runCli defaults to dry-run and returns JSON-safe recommendations', async () => {
  const output = await runCli(['--trigger', 'plan', '--json'], {
    now: () => new Date('2026-06-25T00:00:00.000Z'),
    listModels: async () => MODEL_LIST,
    readText: async (path) => {
      if (path.endsWith('settings.json')) return JSON.stringify(CURRENT_SETTINGS);
      if (path.endsWith('models.json')) return JSON.stringify(MODELS_JSON);
      throw new Error(`unexpected read ${path}`);
    },
    fetchJson: async (url) => {
      if (url.endsWith('/api/quota')) return QUOTA;
      if (url.endsWith('/api/xunfei')) return XUNFEI;
      if (url.endsWith('/api/ainaiba-credit')) return AINAIBA;
      if (url.includes('/api/stats')) return { by_model: [] };
      if (url.endsWith('/api/pricing')) return { model: [] };
      throw new Error(`unexpected fetch ${url}`);
    },
    writeText: async () => { throw new Error('dry-run must not write'); },
  });
  const parsed = JSON.parse(output.stdout);
  assert.equal(parsed.trigger, 'plan');
  assert.equal(output.exitCode, 0);
  assert.equal(output.wrote, false);
});
```

Run tests. Expected before implementation: FAIL with `runCli` missing.

- [ ] **Step 2: Implement CLI adapters**

Add:

```javascript
export async function runCli(argv = process.argv.slice(2), deps = defaultDeps) {}
```

`runCli` must:

```text
- parse flags listed in Router Contracts
- gather model list by running pi --list-models through deps.listModels
- read settings and models JSON through deps.readText
- fetch /api/quota, /api/xunfei, /api/ainaiba-credit, /api/stats, /api/pricing through deps.fetchJson
- continue with warnings if token-stats endpoints are unavailable in dry-run
- print redacted human output by default
- print pure JSON when --json is set
- refuse --apply unless settings hash, valid model refs, and recommendations are valid
```

`defaultDeps` must use Node built-ins:

```javascript
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { readFile, writeFile, copyFile } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { homedir } from 'node:os';
```

- [ ] **Step 3: Wire main guard**

Add this at the bottom:

```javascript
if (import.meta.url === `file://${process.argv[1]}`) {
  runCli().then((result) => {
    if (result.stdout) process.stdout.write(result.stdout);
    if (result.stderr) process.stderr.write(result.stderr);
    process.exitCode = result.exitCode;
  }).catch((error) => {
    console.error(redactSecrets(error?.stack || error?.message || String(error)));
    process.exitCode = 1;
  });
}
```

- [ ] **Step 4: Run tests**

Run:

```bash
node --test /home/skyscribe/.pi/agent/skills/smart-model-router/tests/model-router.test.mjs
```

Expected: PASS.

---

### Task 4: Skill Authoring TDD Baseline

**Files:**
- Create later: `/home/skyscribe/.pi/agent/skills/smart-model-router/SKILL.md`

- [ ] **Step 1: Run a baseline pressure scenario before writing the skill**

Use a fresh-context read-only delegate/reviewer subagent. The prompt should not mention the future skill by name:

```text
You are preparing to launch several Pi subagents for planning, implementation, and review. The user has many providers with different quotas/costs. Describe exactly what you would do before choosing subagent models. Do not modify files.
```

Expected baseline failure condition: the agent does not reliably run the router script because the skill does not exist yet. Record the observed gap in notes for the skill.

- [ ] **Step 2: Do not write `SKILL.md` until the baseline result is recorded**

Expected: a short note is available in the implementation log or final summary:

```text
Baseline gap: agent recommended manual/static model choice and did not invoke a deterministic quota/cost scanner.
```

---

### Task 5: Smart Model Router Skill

**Files:**
- Create: `/home/skyscribe/.pi/agent/skills/smart-model-router/SKILL.md`
- Existing: `/home/skyscribe/.pi/agent/skills/smart-model-router/scripts/model-router.mjs`

- [ ] **Step 1: Write the skill after baseline failure**

Create this skill content:

```markdown
---
name: smart-model-router
description: Use when choosing Pi models for subagents, changing subagent model overrides, launching multi-agent workflows, balancing model quotas/costs, or responding to triggers like plan, implement, review, research, scout, quick, image, cheap, premium, emergency, or long-context.
---

# Smart Model Router

## Overview

Run the router before model-sensitive subagent work. It checks live selectable Pi models, token-stats quota/cost/status data, and current subagent overrides, then recommends valid model/fallback settings.

## When to Use

Use before:
- launching planner, worker, reviewer, scout, researcher, oracle, implementer, fix-worker, or merger subagents
- editing `subagents.agentOverrides`
- choosing models under quota, cost, image, or long-context constraints
- recovering from model quota, 429, auth, or stale model-id failures

Do not use for one-off normal chat responses with no subagent routing decision.

## Required Workflow

1. Pick the closest trigger: `plan`, `implement`, `review`, `research`, `scout`, `quick`, `long-context`, `image`, `cheap`, `premium`, or `emergency`.
2. Run dry-run first:

```bash
node ~/.pi/agent/skills/smart-model-router/scripts/model-router.mjs --trigger <trigger>
```

3. Read the warnings and recommendations.
4. Validate that recommended models are present in `pi --list-models`.
5. Use explicit per-run subagent `model` overrides or ask the user before applying persistent settings.

## Persistent Apply Rule

Never run `--apply` silently. Only run it after the user explicitly approves the generated patch.

Safe apply command:

```bash
node ~/.pi/agent/skills/smart-model-router/scripts/model-router.mjs --trigger <trigger> --apply
```

The script backs up settings, checks file hash, and only changes `subagents.agentOverrides`.

## Common Commands

```bash
node ~/.pi/agent/skills/smart-model-router/scripts/model-router.mjs --trigger plan
node ~/.pi/agent/skills/smart-model-router/scripts/model-router.mjs --trigger implement
node ~/.pi/agent/skills/smart-model-router/scripts/model-router.mjs --trigger review --json
node ~/.pi/agent/skills/smart-model-router/scripts/model-router.mjs --trigger image
node ~/.pi/agent/skills/smart-model-router/scripts/model-router.mjs --trigger cheap
```

## Red Flags

- A configured model is not in `pi --list-models`.
- A provider quota is exhausted, unauthorized, or expired.
- Reviewer and worker use the same primary provider when alternatives exist.
- Image work is routed to a text-only model.
- Long-context work is routed to a small-context model.
- The output contains secrets or personal data; stop and fix redaction.
```

- [ ] **Step 2: Verify skill frontmatter loads cleanly**

Run:

```bash
python3 - <<'PY'
from pathlib import Path
p = Path('/home/skyscribe/.pi/agent/skills/smart-model-router/SKILL.md')
text = p.read_text()
assert text.startswith('---\n')
assert 'name: smart-model-router' in text
assert 'description: Use when' in text
assert len(text.split('---', 2)[1]) < 1024
print('skill frontmatter ok')
PY
```

Expected: `skill frontmatter ok`.

- [ ] **Step 3: Run a verification pressure scenario with the skill available**

Use a fresh-context subagent and explicitly mention the skill is available or pass it as a skill if the harness supports it:

```text
You are preparing to launch several Pi subagents for planning, implementation, and review. The user has many providers with different quotas/costs. Use the smart-model-router skill if applicable. Describe exactly what you would do before choosing subagent models. Do not modify files.
```

Expected pass condition: the agent says to run the dry-run router before selecting or applying model overrides.

---

### Task 6: Live Smoke Validation

**Files:**
- Existing: `/home/skyscribe/.pi/agent/skills/smart-model-router/scripts/model-router.mjs`
- Existing: `/home/skyscribe/.pi/agent/skills/smart-model-router/SKILL.md`
- Do not modify: `/home/skyscribe/.pi/agent/settings.json`

- [ ] **Step 1: Run unit tests**

Run:

```bash
node --test /home/skyscribe/.pi/agent/skills/smart-model-router/tests/model-router.test.mjs
```

Expected: PASS.

- [ ] **Step 2: Run live dry-run for planning**

Run:

```bash
node /home/skyscribe/.pi/agent/skills/smart-model-router/scripts/model-router.mjs --trigger plan --json
```

Expected:

```text
- exit code 0
- JSON parses
- settingsPatch contains only subagents.agentOverrides
- warnings are redacted
- every model and fallback appears in pi --list-models
```

- [ ] **Step 3: Run live dry-run for review**

Run:

```bash
node /home/skyscribe/.pi/agent/skills/smart-model-router/scripts/model-router.mjs --trigger review
```

Expected:

```text
- exit code 0
- human-readable recommendations
- warns about stale invalid configured model refs if present
- does not print email, auth cookies, callback URLs, API keys, access tokens, or login URLs
```

- [ ] **Step 4: Confirm real settings were not modified**

Run:

```bash
python3 - <<'PY'
from pathlib import Path
p = Path('/home/skyscribe/.pi/agent/settings.json')
print(p.stat().st_mtime_ns)
PY
```

Expected: compare manually with the timestamp captured before live smoke if needed; dry-run must not write the file.

---

## Self-Review Checklist

- Spec coverage: The plan covers global skill, script, live data gathering, scoring, warnings, patch output, dry-run default, and gated apply.
- Placeholder scan: No deferred placeholder instructions remain; every task has commands and expected outcomes.
- Type consistency: Public exports named in tests match the exports required from `model-router.mjs`.
- Safety: Real settings writes occur only in `--apply`, and live validation uses dry-run.
