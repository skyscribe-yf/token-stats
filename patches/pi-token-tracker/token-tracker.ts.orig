/**
 * pi-token-tracker extension core.
 *
 * Tracks per-call token usage across:
 *   1. Main pi session — via message_end hook → usage.jsonl
 *   2. Taskplane lane workers & merge agents — loaded as explicit -e
 *      extension by taskplane's loadPiSettingsPackages mechanism,
 *      records per-call data via the same message_end hook.
 *
 * The taskplane scanner in the token-report command also reads
 * cumulative exit summaries for offline/retroactive reporting.
 */
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import {
  appendFileSync,
  existsSync,
  mkdirSync,
  readFileSync,
  readdirSync,
} from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";

// ── Constants ────────────────────────────────────────────────────────

const LOG_DIR = join(homedir(), ".pi", "token-logs");
const LOG_FILE = join(LOG_DIR, "usage.jsonl");

function ensureLogDir() {
  if (!existsSync(LOG_DIR)) {
    mkdirSync(LOG_DIR, { recursive: true });
  }
}

// ── API Key Detection ────────────────────────────────────────────────

function getApiKeyPrefix(ctx: any): string {
  const model = ctx.model;
  if (model?.apiKey) {
    const key = model.apiKey;
    return key.length > 8 ? key.slice(0, 8) + "..." : key;
  }
  for (const env of [
    "ANTHROPIC_API_KEY",
    "OPENAI_API_KEY",
    "GEMINI_API_KEY",
    "DEEPSEEK_API_KEY",
    "XAI_API_KEY",
  ]) {
    const val = process.env[env];
    if (val) {
      return val.length > 8 ? val.slice(0, 8) + "..." : val;
    }
  }
  return "unknown";
}

// ── Extension Entry ──────────────────────────────────────────────────

export default function (pi: ExtensionAPI) {
  ensureLogDir();

  // Per-message timing state.  A single turn may contain multiple assistant
  // messages interleaved with tool calls.  We isolate timing to each
  // individual streaming response so that tool-execution time is never
  // included in TTFT or TPS.
  let providerResponseMs: number | undefined; // after_provider_response
  let firstTokenMs: number | undefined;       // first message_update

  // Per-message deduplication key.  Pi may emit message_end twice for
  // the same assistant message (e.g. once from streaming completion and
  // once from session persistence).  We skip the duplicate to avoid
  // double-counting tokens.
  let lastRecordFingerprint: string | undefined;

  pi.on("after_provider_response", async () => {
    providerResponseMs = Date.now();
  });

  pi.on("message_update", async (event) => {
    if (event.message.role !== "assistant") return;
    if (!firstTokenMs) {
      firstTokenMs = Date.now();
    }
  });

  // Per-call token recording — fires for EVERY assistant message,
  // including in taskplane worker/merge agent RPC sessions.
  pi.on("message_end", async (event, ctx) => {
    if (event.message.role !== "assistant") return;

    const usage = event.message.usage;
    if (!usage) return;

    const model = ctx.model;
    const now = new Date();
    const date = now.toISOString().slice(0, 10);
    const time = now.toISOString();
    const nowMs = Date.now();

    // Deduplicate: skip if this exact (model, input, output) combination
    // was already recorded within the same second.
    const fingerprint = `${model?.id || "?"}|${usage.input}|${usage.output}`;
    if (fingerprint === lastRecordFingerprint) return;
    lastRecordFingerprint = fingerprint;

    // TTFT: time from HTTP response headers to first streaming token.
    // Falls back to undefined when the provider does not expose response
    // timing or when streaming produced zero update events.
    const ttftMs =
      providerResponseMs && firstTokenMs
        ? firstTokenMs - providerResponseMs
        : undefined;

    // TPS: output tokens / streaming generation duration (seconds).
    // Computed only when we observed at least one streaming update.
    // Guard against division by zero (extremely fast responses).
    const elapsedSec = firstTokenMs ? (nowMs - firstTokenMs) / 1000 : 0;
    const tps =
      firstTokenMs && usage.output > 0 && elapsedSec > 0
        ? usage.output / elapsedSec
        : undefined;

    // Reset timing state so the next assistant message (or the next turn)
    // starts with a clean slate.
    providerResponseMs = undefined;
    firstTokenMs = undefined;

    const record = {
      date,
      time,
      apiKeyPrefix: getApiKeyPrefix(ctx),
      provider: model?.provider || "unknown",
      model: model?.id || "unknown",
      source: "pi",
      inputTokens: usage.input || 0,
      outputTokens: usage.output || 0,
      cacheReadTokens: usage.cacheRead || 0,
      cacheWriteTokens: usage.cacheWrite || 0,
      totalTokens:
        (usage.input || 0) +
        (usage.output || 0) +
        (usage.cacheRead || 0) +
        (usage.cacheWrite || 0),
      cost: usage.cost?.total || 0,
      ttftMs,
      tps,
    };

    try {
      appendFileSync(LOG_FILE, JSON.stringify(record) + "\n");
    } catch {
      // Silent fail — don't crash the agent on logging error
    }
  });

  // token-report command — available in both main session and workers
  pi.registerCommand("token-report", {
    description:
      "Show token usage statistics by vendor/model (includes taskplane lane workers)",
    handler: async (args: string, ctx) => {
      const days = parseInt(args) || 7;
      const report = generateReport(days);
      try {
        ctx.ui.notify(report, "info");
      } catch {
        // silent — RPC mode may not have UI
      }
    },
  });

  // Status indicator — safe no-op in RPC mode
  pi.on("session_start", async (_event, ctx) => {
    try {
      ctx.ui.setStatus("token-tracker", "● tracking");
    } catch {
      // RPC mode has no UI — ignore
    }
  });
}

// ── Token Report Generator ───────────────────────────────────────────

function generateReport(days: number): string {
  const allRecords: any[] = [];

  const cutoff = new Date();
  cutoff.setDate(cutoff.getDate() - days);
  const cutoffStr = cutoff.toISOString().slice(0, 10);

  // 1. Live records from usage.jsonl (main session + workers)
  const liveRecords: any[] = [];
  if (existsSync(LOG_FILE)) {
    const lines = readFileSync(LOG_FILE, "utf-8")
      .split("\n")
      .filter(Boolean)
      .map((l) => JSON.parse(l));
    for (const r of lines) {
      if (r.date >= cutoffStr) {
        allRecords.push(r);
        liveRecords.push(r);
      }
    }
  }

  // Build coverage set: if usage.jsonl already has per-call records
  // for a given (date, provider, model), exit summaries for matching
  // agents are redundant (would double-count).
  const covered = new Set<string>();
  for (const r of liveRecords) {
    covered.add(`${r.date}|${r.provider}|${r.model}`);
  }

  // 2. Taskplane runtime records (retroactive — only agents NOT already
  //    covered by per-call data from usage.jsonl)
  const runtimeRecords = scanTaskplaneRuntime(cutoffStr, covered);
  for (const r of runtimeRecords) {
    allRecords.push(r);
  }

  if (allRecords.length === 0) {
    return `No token usage data in the last ${days} days.`;
  }

  // Build hierarchy: provider -> model -> stats
  const tree = new Map<string, Map<string, any>>();
  let grandTotal = {
    calls: 0,
    inputTokens: 0,
    outputTokens: 0,
    cacheReadTokens: 0,
    cacheWriteTokens: 0,
    totalTokens: 0,
    cost: 0,
  };
  let runtimeCalls = 0;

  for (const r of allRecords) {
    if (!tree.has(r.provider)) {
      tree.set(r.provider, new Map());
    }
    const models = tree.get(r.provider)!;
    if (!models.has(r.model)) {
      models.set(r.model, {
        calls: 0,
        inputTokens: 0,
        outputTokens: 0,
        cacheReadTokens: 0,
        cacheWriteTokens: 0,
        totalTokens: 0,
        cost: 0,
      });
    }
    const m = models.get(r.model)!;
    m.calls += 1;
    m.inputTokens += r.inputTokens;
    m.outputTokens += r.outputTokens;
    m.cacheReadTokens += r.cacheReadTokens;
    m.cacheWriteTokens += r.cacheWriteTokens;
    m.totalTokens += r.totalTokens;
    m.cost += r.cost;

    grandTotal.calls += 1;
    grandTotal.inputTokens += r.inputTokens;
    grandTotal.outputTokens += r.outputTokens;
    grandTotal.cacheReadTokens += r.cacheReadTokens;
    grandTotal.cacheWriteTokens += r.cacheWriteTokens;
    grandTotal.totalTokens += r.totalTokens;
    grandTotal.cost += r.cost;

    if (r._source === "runtime") runtimeCalls++;
  }

  let out = `\n📊 Token Usage Report (last ${days} days)\n\n`;

  const liveCalls = allRecords.length - runtimeCalls;
  out += `Source breakdown:  ${liveCalls} live calls (main session)  +  ${runtimeCalls} lane-worker batch(es) from runtime\n\n`;

  for (const [provider, models] of tree) {
    let providerTotal = {
      calls: 0,
      inputTokens: 0,
      outputTokens: 0,
      cacheReadTokens: 0,
      cacheWriteTokens: 0,
      totalTokens: 0,
      cost: 0,
    };
    for (const [, m] of models) {
      providerTotal.calls += m.calls;
      providerTotal.inputTokens += m.inputTokens;
      providerTotal.outputTokens += m.outputTokens;
      providerTotal.cacheReadTokens += m.cacheReadTokens;
      providerTotal.cacheWriteTokens += m.cacheWriteTokens;
      providerTotal.totalTokens += m.totalTokens;
      providerTotal.cost += m.cost;
    }

    out += `▶ ${provider}\n`;
    out += `  Provider Total:  Calls=${providerTotal.calls}  Input=${fmt(providerTotal.inputTokens)}  Output=${fmt(providerTotal.outputTokens)}  CacheR=${fmt(providerTotal.cacheReadTokens)}  CacheW=${fmt(providerTotal.cacheWriteTokens)}  Total=${fmt(providerTotal.totalTokens)}  Cost=$${providerTotal.cost.toFixed(4)}\n`;
    out += `  ─────────────────────────────────────────────────────────────────────────\n`;

    const sortedModels = Array.from(models.entries()).sort(
      (a, b) => b[1].totalTokens - a[1].totalTokens,
    );
    for (const [modelId, m] of sortedModels) {
      out += `    ${modelId.padEnd(28)}  calls=${String(m.calls).padStart(4)}  in=${fmt(m.inputTokens).padStart(10)}  out=${fmt(m.outputTokens).padStart(10)}  cacheR=${fmt(m.cacheReadTokens).padStart(8)}  cacheW=${fmt(m.cacheWriteTokens).padStart(8)}  total=${fmt(m.totalTokens).padStart(11)}  cost=$${m.cost.toFixed(4)}\n`;
    }
    out += `\n`;
  }

  out += `━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n`;
  out += `GRAND TOTAL  calls=${grandTotal.calls}  in=${fmt(grandTotal.inputTokens)}  out=${fmt(grandTotal.outputTokens)}  cacheR=${fmt(grandTotal.cacheReadTokens)}  cacheW=${fmt(grandTotal.cacheWriteTokens)}  total=${fmt(grandTotal.totalTokens)}  cost=$${grandTotal.cost.toFixed(4)}\n`;
  out += `━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n`;

  // Per-API-key summary
  const keyAgg = new Map<string, any>();
  for (const r of allRecords) {
    if (!keyAgg.has(r.apiKeyPrefix)) {
      keyAgg.set(r.apiKeyPrefix, {
        calls: 0,
        inputTokens: 0,
        outputTokens: 0,
        cacheReadTokens: 0,
        cacheWriteTokens: 0,
        totalTokens: 0,
        cost: 0,
      });
    }
    const k = keyAgg.get(r.apiKeyPrefix)!;
    k.calls += 1;
    k.inputTokens += r.inputTokens;
    k.outputTokens += r.outputTokens;
    k.cacheReadTokens += r.cacheReadTokens;
    k.cacheWriteTokens += r.cacheWriteTokens;
    k.totalTokens += r.totalTokens;
    k.cost += r.cost;
  }

  out += `\n🔑 Per-API-Key Summary\n`;
  out += `─────────────────────────────────────────────────────────────────────────\n`;
  for (const [key, k] of keyAgg) {
    out += `  ${key}  calls=${k.calls}  in=${fmt(k.inputTokens)}  out=${fmt(k.outputTokens)}  cacheR=${fmt(k.cacheReadTokens)}  cacheW=${fmt(k.cacheWriteTokens)}  total=${fmt(k.totalTokens)}  cost=$${k.cost.toFixed(4)}\n`;
  }
  out += `─────────────────────────────────────────────────────────────────────────\n`;

  return out;
}

// ── Taskplane Runtime Scanner ─────────────────────────────────────────

/**
 * Scan all ~/srcs/* /.pi/runtime/ project directories for taskplane
 * lane-worker and merge-agent token usage from exit summaries.
 *
 * These agents run in separate pi --mode rpc processes with
 * --no-extensions + explicit -e flags (which now include pi-token-tracker).
 * Their per-call data is recorded live via the message_end hook above.
 *
 * The exit-summary scan below provides retroactive coverage for batches
 * that ran before pi-token-tracker was installed, or as a safety net
 * if the extension was excluded from a particular agent's explicit -e list.
 */
function scanTaskplaneRuntime(
  cutoffStr: string,
  covered: Set<string>,
): any[] {
  const projectsDir = process.env.TASKPLANE_PROJECTS_DIR
    ?? join(homedir(), "srcs");
  const srcsDir = projectsDir;
  if (!existsSync(srcsDir)) return [];

  const records: any[] = [];

  let projectDirs: string[];
  try {
    projectDirs = readdirSync(srcsDir);
  } catch {
    return [];
  }

  for (const project of projectDirs) {
    const runtimeRoot = join(srcsDir, project, ".pi", "runtime");
    if (!existsSync(runtimeRoot)) continue;

    let batchDirs: string[];
    try {
      batchDirs = readdirSync(runtimeRoot);
    } catch {
      continue;
    }

    for (const batchId of batchDirs) {
      const batchPath = join(runtimeRoot, batchId);
      const batchDate = parseBatchDate(batchId);
      if (batchDate < cutoffStr) continue;

      const batchTime = parseBatchTime(batchId);

      const agentsDir = join(batchPath, "agents");
      if (!existsSync(agentsDir)) continue;

      let agentDirs: string[];
      try {
        agentDirs = readdirSync(agentsDir);
      } catch {
        continue;
      }

      for (const agentId of agentDirs) {
        const agentPath = join(agentsDir, agentId);

        // Try both exit file types (lane workers use events-exit.json,
        // merge agents use exit-summary.json — same structure)
        const exitPaths = ["events-exit.json", "exit-summary.json"];
        let exitData: any = null;
        for (const name of exitPaths) {
          const exitPath = join(agentPath, name);
          if (existsSync(exitPath)) {
            try {
              exitData = JSON.parse(readFileSync(exitPath, "utf-8"));
              break;
            } catch {
              /* try next file */
            }
          }
        }
        if (!exitData) continue;

        const tokens = exitData.tokens;
        if (!tokens || typeof tokens.input !== "number") continue;

        // Extract provider + model from events.jsonl's agent_started event
        const [provider, model] = readAgentProviderModel(agentPath);

        // Skip if usage.jsonl already has per-call records for this
        // (date, provider, model). batchTime is UTC-converted; its date
        // matches usage.jsonl date field (also UTC).
        const utcDate = batchTime.slice(0, 10);
        const coverKey = `${utcDate}|${provider}|${model}`;
        if (covered.has(coverKey)) continue;

        records.push({
          date: batchDate,
          time: batchTime,
          apiKeyPrefix: `runtime:${batchId}`,
          source: "pi",
          provider,
          model,
          inputTokens: tokens.input,
          outputTokens: tokens.output,
          cacheReadTokens: tokens.cacheRead ?? 0,
          cacheWriteTokens: tokens.cacheWrite ?? 0,
          totalTokens:
            tokens.input +
            tokens.output +
            (tokens.cacheRead ?? 0) +
            (tokens.cacheWrite ?? 0),
          cost: exitData.cost ?? 0,
          _source: "runtime", // marker for stats in the report
        });
      }
    }
  }

  return records;
}

// ── Agent Provider/Model Resolution ──────────────────────────────────

/**
 * Read provider and model from the events.jsonl in an agent directory.
 * Falls back to model-based provider resolution when the model string
 * does not contain a "/" separator (e.g. "kimi-for-coding" → kimi).
 */
function readAgentProviderModel(agentPath: string): [string, string] {
  const eventsPath = join(agentPath, "events.jsonl");
  if (!existsSync(eventsPath)) return ["taskplane-worker", "unknown"];

  try {
    const firstLine = readFileSync(eventsPath, "utf-8")
      .split("\n")
      .filter(Boolean)[0];
    if (!firstLine) return ["taskplane-worker", "unknown"];

    const ev = JSON.parse(firstLine);
    if (ev.type !== "agent_started" || !ev.payload?.model) {
      return ["taskplane-worker", "unknown"];
    }

    const modelRef: string = String(ev.payload.model);
    const slashIdx = modelRef.indexOf("/");
    if (slashIdx !== -1) {
      return [modelRef.slice(0, slashIdx), modelRef.slice(slashIdx + 1)];
    }

    // No "/" — infer provider from model name
    const provider = resolveProviderFromModel(modelRef);
    return [provider, modelRef];
  } catch {
    return ["taskplane-worker", "unknown"];
  }
}

/**
 * Resolve a human-readable provider name from a model name.
 * Synchronized with the Rust backend's resolve_provider_from_model().
 */
function resolveProviderFromModel(model: string): string {
  switch (model) {
    case "kimi-for-coding":
    case "kimi-k2.6":
    case "kimi-k2.5":
      return "kimi";
    case "astron-code-latest":
      return "xunfei";
    case "mimo-v2.5-pro":
    case "mimo-v2-pro":
    case "mimo-v2.5":
      return "xiaomi-mimo";
    case "deepseek-v4-pro":
    case "deepseek-v4-flash":
      return "deepseek";
    case "gpt-5.5":
    case "gpt-5.4":
    case "gpt-5.4-mini":
      return "openai";
    case "glm-5.1":
      return "opencode-go";
    default:
      return model;
  }
}

// ── Batch Timestamp Parsing ──────────────────────────────────────────

/**
 * Parse a taskplane batch directory name like "20260508T094931" into
 * a date string ("2026-05-08") for cutoff comparison.
 */
function parseBatchDate(batchId: string): string {
  if (batchId.length < 8) return "";
  try {
    const year = batchId.slice(0, 4);
    const month = batchId.slice(4, 6);
    const day = batchId.slice(6, 8);
    return `${year}-${month}-${day}`;
  } catch {
    return "";
  }
}

/**
 * Parse a taskplane batch directory name into an ISO 8601 timestamp.
 * Format: "20260508T094931" (local time, UTC+8) → "2026-05-08T01:49:31Z"
 */
function parseBatchTime(batchId: string): string {
  if (batchId.length < 15) return new Date().toISOString();
  try {
    const year = batchId.slice(0, 4);
    const month = batchId.slice(4, 6);
    const day = parseInt(batchId.slice(6, 8), 10) || 1;
    const hour = parseInt(batchId.slice(9, 11), 10) || 0;
    const min = batchId.slice(11, 13);
    const sec = batchId.slice(13, 15);
    // Convert from UTC+8 to UTC
    let utcH = (hour - 8 + 24) % 24;
    let utcDay = day;
    if (hour < 8) {
      utcDay = Math.max(1, day - 1);
    }
    return `${year}-${month}-${String(utcDay).padStart(2, "0")}T${String(utcH).padStart(2, "0")}:${min}:${sec}Z`;
  } catch {
    return new Date().toISOString();
  }
}

// ── Formatting ───────────────────────────────────────────────────────

function fmt(n: number): string {
  return n.toLocaleString("en-US");
}
