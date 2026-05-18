# Fix Plan: Taskplane Token Collection Gaps

> 调查日期：2026-05-19  
> 状态：计划阶段，待修复

---

## 问题概述

当前 token-stats dashboard 对 pi Taskplane 的 token 采集存在缺口，导致大量 lane worker 和 merge agent 的 token 消耗未显示在仪表盘中。

---

## 数据源架构回顾

```
pi Taskplane 的 token 数据有 3 个来源：

1. usage.jsonl（~/.pi/token-logs/）
   ─ 由 token-tracker 扩展的 message_end hook 写入
   ─ 覆盖：主 pi 会话（含 orchestrator/reviewer/supervisor 调用）
   ─ 不覆盖：lane worker / merge agent（它们用 --no-extensions 运行）

2. events-exit.json（<project>/.pi/runtime/<batchId>/agents/<agentId>/）
   ─ pi 进程退出时自动生成
   ─ 覆盖：lane worker 的累计 token 统计
   ─ 不覆盖：merge agent

3. exit-summary.json（同上目录）
   ─ 与 events-exit.json 结构完全相同，仅文件名不同
   ─ 覆盖：merge agent 的累计 token 统计
   ─ ⚠️ 当前完全未被采集
```

---

## Bug 1（🔴 P0）：Merge Agent 的 exit-summary.json 未被扫描

### 症状
17 个 merge agent 的 token 数据完全丢失，合计约 **3.6M tokens**（1.88M input + 1.71M cache read）。

### 根因
`events-exit.json` 和 `exit-summary.json` 的 JSON 结构完全一致（相同的顶层 key 和 tokens 子结构），仅文件名不同。但两处扫描代码都硬编码了 `events-exit.json`：

| 文件 | 行号 | 代码 |
|------|------|------|
| `backend/src/sources/pi.rs` | L120 | `let exit_path = agent_path.join("events-exit.json");` |
| `~/.pi/agent/extensions/token-tracker.ts` | L233 | `const exitPath = join(agentPath, "events-exit.json");` |

### 修复方案

**Rust 后端 (`backend/src/sources/pi.rs`)：扫描两种文件**

在 `scan_batches` 函数中，对每个 agent 目录依次尝试 `events-exit.json` 和 `exit-summary.json`：

```rust
// 伪代码
let exit_paths = [
    agent_path.join("events-exit.json"),
    agent_path.join("exit-summary.json"),
];
for exit_path in &exit_paths {
    if exit_path.exists() {
        // parse and create TokenRecord
        break;
    }
}
```

**Token-tracker 扩展 (`token-tracker.ts`)：同上**

```typescript
const exitPaths = ["events-exit.json", "exit-summary.json"];
for (const name of exitPaths) {
    const exitPath = join(agentPath, name);
    if (existsSync(exitPath)) {
        // parse and create record
        break;
    }
}
```

### 影响范围
- 17 个 merge agent 将被正确采集
- 数据结构兼容，无需修改 `ExitData`/`ExitTokens` 类型
- 不会产生重复——一个 agent 目录最多只有一个退出文件

---

## Bug 2（🟡 P1）：Mind-shield 批次的 Provider 识别错误

### 症状
API 中出现 5 条 `provider=taskplane-worker` 的记录（"taskplane-worker" 不是有效的 vendor 名），这些记录的 model 是 `kimi-for-coding`。

### 根因
不同批次的 `events.jsonl` 中 model 字段格式不一致：

| 批次 | model 字段 | 解析结果 |
|------|-----------|---------|
| mind-shield | `kimi-for-coding` | provider=`taskplane-worker`, model=`kimi-for-coding` |
| mini-program | `kimi/kimi-for-coding` | provider=`kimi`, model=`kimi-for-coding` |

`read_agent_provider_model()` 函数在没有 `/` 分隔符时回退到 `("taskplane-worker", model_ref)`。

### 修复方案

**方案 A（推荐）**：使用 `resolve_provider_from_model()` 回退

当 model 不含 `/` 时，不再使用硬编码的 `"taskplane-worker"`，而是通过 `resolve_provider_from_model()` 推断正确的 provider：

```rust
// pi.rs: read_agent_provider_model() 中
if let Some(slash_pos) = model_ref.find('/') {
    let provider = model_ref[..slash_pos].to_string();
    let model = model_ref[slash_pos + 1..].to_string();
    (provider, model)
} else {
    // 用 model 名推断 provider（如 kimi-for-coding → kimi）
    let provider = super::resolve_provider_from_model(&model_ref);
    (provider, model_ref)
}
```

**方案 B**：让 taskplane 统一 model 格式为 `provider/model`

在 taskplane 的 `engine.ts` 中生成 `agent_started` 事件时统一格式。但这是跨项目改动，影响面更大。

### 影响范围
- Token-tracker 扩展 (`token-tracker.ts`) 中也存在相同逻辑（L265-272），需同步修复
- 5 条记录将从 `taskplane-worker` 变成正确的 `kimi` provider

---

## Bug 3（🟡 P1）：Token-tracker 扩展的 scanTaskplaneRuntime 只扫描当前项目

### 症状
`token-report` 命令只能看到当前项目目录下的 runtime 数据。在 `mind-shield` 目录运行就看不到 `mini-program` 的 worker 记录。

### 根因
```typescript
// token-tracker.ts L210-214
function scanTaskplaneRuntime(projectPath: string | undefined, cutoffStr: string): any[] {
    if (!projectPath) return [];
    const runtimeRoot = join(projectPath, ".pi", "runtime");
```

`token-report` handler 传入的 `projectPath` 是 `ctx.cwd`（当前工作目录），而 Rust 后端扫描的是 `~/srcs/*/` 下所有项目。

### 修复方案

**方案 A（推荐）**：扫描所有项目目录

与 Rust 后端保持一致，遍历 `~/srcs/*/`：

```typescript
function scanTaskplaneRuntime(cutoffStr: string): any[] {
    const srcsDir = join(homedir(), "srcs");
    if (!existsSync(srcsDir)) return [];
    
    const records: any[] = [];
    let projectDirs;
    try { projectDirs = readdirSync(srcsDir); } catch { return []; }
    
    for (const project of projectDirs) {
        const runtimeRoot = join(srcsDir, project, ".pi", "runtime");
        if (!existsSync(runtimeRoot)) continue;
        // scan batches...
    }
    return records;
}
```

**方案 B**：保留 `projectPath` 参数作为可选的过滤项，默认扫描全部

### 影响范围
- `token-report` 命令调用处需要去 `projectPath` 参数（L97）
- `generateReport` 函数签名相应调整

---

## Bug 4（🟢 P2）：scanTaskplaneRuntime 中 time 字段不正确

### 症状
Runtime 记录的 `time` 字段使用了 `new Date().toISOString()`（报告生成时的当前时间），而非 agent 实际退出的时间。

### 根因
```typescript
// token-tracker.ts L291
time: new Date().toISOString(),
```

### 修复方案

从 batch 目录名（`YYYYMMDDTHHMMSS`）解析出近似时间。这只是 `token-report` 命令的问题，Rust 后端的 `parse_batch_timestamp()` 已正确处理。

---

## Bug 5（🟢 P3）：usage.jsonl 缺少 source 字段

### 症状
Token-tracker 扩展写入的 `usage.jsonl` 记录没有 `source` 字段。

### 根因
```typescript
// token-tracker.ts L50-62
const record = {
    date, time, apiKeyPrefix, provider, model,
    inputTokens, outputTokens, cacheReadTokens, cacheWriteTokens,
    totalTokens, cost,
    // 缺少 source: "pi"
};
```

### 修复方案
添加 `source: "pi"` 到 record 对象。当前 Rust 后端通过 `if record.source.is_empty() { record.source = "pi" }` 兜底，暂不影响功能。

---

## 非 Bug：运行中 Worker 无实时数据

### 说明
Lane worker 进程使用 `--no-extensions` 启动，token-tracker 扩展不加载。worker 的 token 数据只能在进程退出后通过 `events-exit.json` 采集。这是架构设计决定的，不是 bug。

**可能的改进方向**（非必需）：
- Taskplane engine-worker 定期轮询子进程的 token 使用情况并写入中间状态文件
- 或在 worker 的 `events.jsonl` 中按调用记录 token（当前 `events.jsonl` 有 `context_usage` 事件但仅记录 context window 使用率）

---

## 修复优先级

| 优先级 | Bug | 影响 | 修复难度 |
|:---:|-----|------|:---:|
| 🔴 P0 | Merge agent exit-summary.json 未扫描 | 17 agent / 3.6M tokens 丢失 | 低（改两处文件名检测） |
| 🟡 P1 | Provider 识别错误 | 5 条记录显示为伪 vendor | 低（改用回退函数） |
| 🟡 P1 | Extension 只扫描当前项目 | token-report 遗漏其他项目数据 | 低（改遍历逻辑） |
| 🟢 P2 | time 字段不正确 | token-report 显示时间不准 | 低 |
| 🟢 P3 | usage.jsonl 缺 source | 依赖 Rust 兜底 | 极低 |

---

## 需要修改的文件

### Rust 后端
| 文件 | 修改内容 |
|------|---------|
| `backend/src/sources/pi.rs` | ① `scan_batches` 中同时检查 `events-exit.json` 和 `exit-summary.json` |
| | ② `read_agent_provider_model` 用 `resolve_provider_from_model` 回退 |

### Token-tracker 扩展
| 文件 | 修改内容 |
|------|---------|
| `~/.pi/agent/extensions/token-tracker.ts` | ① `scanTaskplaneRuntime` 同时检查两种文件名 |
| | ② `read_agent_provider_model` 同步修复 provider 回退 |
| | ③ `scanTaskplaneRuntime` 改为扫描所有 `~/srcs/*/` 项目 |
| | ④ 修复 `time` 字段（从 batch 名解析） |
| | ⑤ 添加 `source: "pi"` 字段 |

---

## 验证方法

修复后验证：

1. **检查 API 返回数据量**：对比修复前后 `/api/stats` 中 pi source 的记录数
2. **检查 merge agent 数据**：`/api/requests?provider=kimi&model=kimi-for-coding` 应包含 merge agent 的记录
3. **检查 provider 名**：不应再出现 `provider=taskplane-worker`
4. **运行 token-report**：在不同项目目录下运行 `token-report` 应得到一致的 runtime 数据
5. **检查 Rust 端日志**：`Loading pi data` / `Loaded N pi taskplane runtime records` 应包含 merge agent 数量

---

## 附录：Agent 类型与退出文件对照表

| Agent ID 模式 | 角色 | 退出文件 | 当前已采集？ |
|--------------|------|---------|:---:|
| `*-lane-N-worker` | Lane worker | `events-exit.json` | ✅ |
| `*-merge-N` | Merge agent | `exit-summary.json` | ❌ |

> **注意**：reviewer / supervisor 不是独立的 runtime agent，它们由主 pi 会话（orchestrator）执行，token 已通过 `usage.jsonl` 正常采集。
