// Repair dim session selections broken by the model-prefix rename.
//
// Context: the workbuddy plugin now namespaces its CodeBuddy models (`wb/...`)
// and the ollama upstream uses `ollama/...` (both served by the same CPA). The
// old direct ollama provider (`custom-ollama-cloud-042036d3`) and its CPA twin
// (`ollama-cloud-proxy`) were removed. Sessions created before the rename still
// pin bare ids (`hy4-preview`) or ids of removed providers, so dim resolves
// nothing and reports:
//
//   PROVIDER_CREDENTIAL_MISSING / "Credential missing for provider: ..."
//   (renderer shows 缺少凭据 / 凭据已失效)
//
// even though the credentials are fine. This script re-points those rows at the
// merged `workbuddy` provider and the correctly prefixed model id.
//
// Idempotent: rows that already resolve are left untouched.
//
// Usage:
//   bun scripts/dim-repair-session-models.mjs --dry-run   # report only
//   bun scripts/dim-repair-session-models.mjs             # apply
//
// Requires dim to be closed or idle (writes take a short BEGIN IMMEDIATE).
// Back up `~/.dimcode/v2/dimcode.sqlite` first if you want a rollback point.
import { Database } from "bun:sqlite";

const DB = "/home/skyscribe/.dimcode/v2/dimcode.sqlite";
const DRY = process.argv.includes("--dry-run");
const db = new Database(DB);

const providers = new Map();
for (const row of db.query("SELECT providerId, models FROM providers").all()) {
  let ids = [];
  try { ids = (JSON.parse(row.models) ?? []).map((m) => m.modelId); } catch {}
  providers.set(row.providerId, new Set(ids));
}

const PROVIDER_RENAME = new Map([
  ["custom-ollama-cloud-042036d3", "workbuddy"], // old direct ollama.com channel
  ["ollama-cloud-proxy", "workbuddy"],           // removed CPA twin
]);

// Namespace each origin prefers, so an ambiguous base name (one that exists on
// both upstreams, e.g. deepseek-v4.1-flash) lands on the right one.
// Renamed/delisted model ids: map to the current catalog id on the same provider.
const MODEL_RENAME = new Map([
  // DimAgent renamed the temporary v4.1 flash id (2026-09-10) and retired the
  // vision-exp alias; both now bill/serve as the plain v4.1 flash.
  ["deepseek-v4-flash-vision-exp", "deepseek-v4.1-flash"],
  ["deepseek-v4.1-flash-expires-on-0910", "deepseek-v4.1-flash"],
]);

const PREFERRED_PREFIX = new Map([
  ["custom-ollama-cloud-042036d3", "ollama/"],
  ["ollama-cloud-proxy", "ollama/"],
  ["workbuddy", "wb/"],
]);

// base name -> candidate ids (ids may be bare or prefixed).
const byBase = new Map();
for (const [pid, ids] of providers) {
  for (const id of ids) {
    const base = id.includes("/") ? id.slice(id.lastIndexOf("/") + 1) : id;
    if (!byBase.has(base)) byBase.set(base, []);
    byBase.get(base).push({ pid, id });
  }
}

function resolve(originPid, modelId) {
  if (!modelId) return null;
  const targetPid = PROVIDER_RENAME.get(originPid) ?? originPid;
  const target = providers.get(targetPid);
  if (!target) return null;
  if (target.has(modelId)) return { pid: targetPid, id: modelId }; // already valid
  const base = modelId.includes("/") ? modelId.slice(modelId.lastIndexOf("/") + 1) : modelId;
  const renamed = MODEL_RENAME.get(base);
  if (renamed && target.has(renamed)) return { pid: targetPid, id: renamed };
  const wantPrefix = PREFERRED_PREFIX.get(originPid);
  const candidates = (byBase.get(base) ?? []).filter((c) => c.pid === targetPid);
  const preferred = wantPrefix && candidates.find((c) => c.id === wantPrefix + base);
  const chosen = preferred ?? candidates[0];
  if (!chosen) return null;
  return { pid: chosen.pid, id: chosen.id };
}

const rows = db
  .query("SELECT sessionId, selectedProviderId, selectedModelId FROM session_states")
  .all();

const updates = [];
const unresolved = new Map();
for (const r of rows) {
  if (!r.selectedProviderId) continue;
  const got = resolve(r.selectedProviderId, r.selectedModelId);
  if (!got) {
    const key = `${r.selectedProviderId}/${r.selectedModelId}`;
    unresolved.set(key, (unresolved.get(key) ?? 0) + 1);
    continue;
  }
  if (got.pid === r.selectedProviderId && got.id === r.selectedModelId) continue;
  updates.push({ sessionId: r.sessionId, fromPid: r.selectedProviderId, fromMid: r.selectedModelId, ...got });
}

console.log(`session_states: ${rows.length} row(s); ${updates.length} need repair${DRY ? " (dry run)" : ""}`);
const summary = new Map();
for (const u of updates) {
  const k = `${u.fromPid}/${u.fromMid}  ->  ${u.pid}/${u.id}`;
  summary.set(k, (summary.get(k) ?? 0) + 1);
}
for (const [k, n] of [...summary].sort((a, b) => b[1] - a[1])) console.log(`  ${String(n).padStart(4)}  ${k}`);
if (unresolved.size) {
  console.log("left unresolved (no catalog match):");
  for (const [k, n] of [...unresolved].sort((a, b) => b[1] - a[1])) console.log(`  ${String(n).padStart(4)}  ${k}`);
}

if (!DRY && updates.length) {
  const stmt = db.prepare(
    "UPDATE session_states SET selectedProviderId = ?, selectedModelId = ? WHERE sessionId = ? AND selectedProviderId = ? AND selectedModelId IS ?"
  );
  db.exec("BEGIN IMMEDIATE");
  let n = 0;
  for (const u of updates) n += stmt.run(u.pid, u.id, u.sessionId, u.fromPid, u.fromMid).changes;
  db.exec("COMMIT");
  console.log(`updated ${n} session_states row(s)`);
}
db.close();
