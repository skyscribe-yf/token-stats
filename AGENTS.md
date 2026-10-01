# 项目上下文：Token Stats Dashboard（供 AI 编码代理使用）

> 本文件是面向 AI 编码代理的项目上下文说明，是**索引与不变式**：架构、数据源清单（精简）、
> API、数据模型与约定留在此处，篇幅大的逐项参考拆到了 [`docs/agents/`](docs/agents/)。
> 修改代码时请先阅读本文件，再按下面的路由表展开对应参考文档；
> 内容若与实际代码不符，请优先以代码为准并同步更新文档。

## 参考文档路由（何时必读）

| 文档 | 内容 | 什么时候必须读 |
|------|------|---------------|
| [`docs/agents/data-sources.md`](docs/agents/data-sources.md) | 19 个源的完整行为说明 + 各 loopback 代理/CLIProxyAPI 插件（Grok、cc-proxy、workbuddy、ollama-usage、stepfun-usage）+ CPA 模型命名空间与 dim 侧踩坑史 | 改任何 `backend/src/sources/*.rs`、加数据源、动 CPA（`:8317`）配置或 dim provider/模型前缀 |
| [`docs/agents/pricing.md`](docs/agents/pricing.md) | `pricing.toml` 全部计费分支、分段汇率/折扣、实测费率与对账口径 | 改 `backend/src/pricing.rs`、`dim_entitlement.rs` 或编辑 `pricing.toml` |
| [`docs/agents/quota-cards.md`](docs/agents/quota-cards.md) | 每张配额卡的端点、认证细节、DimAgent console API 逆向结论 | 改 `backend/src/quota/*.rs` 或某张卡显示异常/凭据失效 |
| [`docs/agents/environment-variables.md`](docs/agents/environment-variables.md) | 全部环境变量（默认值 + 覆盖项 + 与插件共用的读取点） | 查某个变量的默认值、给 systemd 注入凭据 |
| [`docs/agents/pitfalls.md`](docs/agents/pitfalls.md) | 编号陷阱 1–25（共 24 条，历史上跳过 20；本文件末尾按主题给出该读哪几条） | 见下方「按主题的陷阱编号」 |
| [`docs/ecs-deployment.md`](docs/ecs-deployment.md) | 经 ECS + SSH 反向隧道暴露公网 | 只在做公网暴露相关改动时 |

---

## 项目概览

一个 Web 仪表盘，用于监控多个 AI 工具/提供商的 token 用量。聚合 **pi**（本编码代理）、
Claude Code、Codex、OpenCode、Kimi CLI、Kimi Code、Qoder、Grok CLI、Command Code、
CodeBuddy、ZCode、DSH、Dim 等数据源，提供图表、表格与筛选的统一分析视图。

**技术栈：** Rust（Axum）后端 + React 19 + Tailwind CSS v4 + Recharts 前端，经 nginx
反向代理部署在 `/token-stats/`。

---

## 架构

```
浏览器 → nginx:80 → Rust Axum API (:3000) + 静态文件
                     ↑
              读取多个数据源文件/SQLite
```

- 后端启动时从各数据源**全量读取**，之后每 30s 增量刷新（`REFRESH_INTERVAL_SECS`）。
- 所有解析后的记录写入专用 SQLite 存储（`TokenStore`）持久化；内存中持有
  `Arc<AppState>` + `RwLock<Vec<TokenRecord>>` 快照，内存是 DB 的超集（见"数据持久化"）。
- 后端同时提供静态文件服务（`backend/static/`，前端构建产物）。
- 后端二进制还兼任两个 loopback 用量代理（`grok_proxy.rs` / `cc_proxy.rs`），各以
  `--grok-proxy-only` / `--cc-proxy-only` 独立运行为 systemd 服务。

### 数据源清单（精简）

每个源的完整说明在 [`docs/agents/data-sources.md`](docs/agents/data-sources.md)；
下表是日常需要的「id → 位置 → 关键点」。

| # | `source` | 位置 | 关键点 |
|---|----------|------|--------|
| 1 | `pi` | `~/.pi/token-logs/usage.jsonl` | JSONL；另扫描 Taskplane runtime `events-exit.json` / `exit-summary.json` |
| 2 | `codex` | `~/.codex/sessions/*/rollout-*.jsonl` | 增量解析**必须**读 `session_meta`/`turn_context`，否则堆出 `model=unknown` 双份（陷阱 12） |
| 3 | `claude-code` | `~/.claude/projects/*/*.jsonl` | Anthropic 缓存语义，无需减法 |
| 4 | `opencode` | `~/.local/share/opencode/opencode.db` | SQLite 只读；**2.x 把消息搬到 `session_message`（旧 `message` 表冻结不再写入）**，两处都要读，且 JSON 形状不同（见详版） |
| 5 | `kimi-cli` | `~/.kimi/sessions/*/wire.jsonl` | JSONL |
| 6 | `kimi-code` | `~/.kimi-code*/sessions/*/*/agents/*/wire.jsonl` | 自动发现多个 home（`KIMI_CODE_HOME` 可锁定） |
| 7 | `qoder-cli` | `~/.qoder/logs/sessions/<slug>/<session>/segments/*.jsonl` | 国际版 CLI；只取 `type=model.response.completed` 并按 `request_id` 去重；OpenAI 式 input **含**缓存需减 |
| 8 | `qoder-desktop` | `~/.qoder-cn/logs/sessions/…`（同一解析器） | **token 恒为 0**（CN 网关不回传 usage）→ 只有调用次数有意义；靠 `counts_as_call_without_tokens()` 不被当作失败请求丢弃（陷阱 7） |
| 9 | `grok-cli` | `~/.token-stats/grok-usage.jsonl` | 内置 Grok 代理写入；**不出现在请求明细**（陷阱 5） |
| 10 | `commandcode` | `~/.commandcode/projects/<slug>/<session-id>.jsonl` | `type=message` 行含 `usage`；跳过侧车 `*.checkpoints.jsonl` |
| 11 | `zcode` | `~/.zcode/cli/db/db.sqlite`（`model_usage` 表） | provider 映射看 `provider_metadata_json`，**先剥 `builtin:`/`account:` 命名空间**；内置 cc-proxy 通道整批丢弃防双计（陷阱 19） |
| 12 | `dsh` | `~/.dsh/sessions/*/session-*/session.jsonl.zstd` | zstd JSONL；usage chunk 与 `finish` replayState 配对取 provider/model |
| 13 | `dim` | DimAgent console API `dimagent.cn/api/log/self` + 本地 `dimcode.sqlite` 补充 | HTTP 轮询逐请求明细（`p` 分页、`type=2`）；本地补充只取第三方通道并排除一切已逐请求计量/指向 CPA 回环的通道（陷阱 4） |
| 14 | `ccswitch` | `~/.cc-switch/cc-switch.db` | 仅当设置了 `USE_CC_SWITCH` 才加载 |
| 15 | `codebuddy` | `~/.codebuddy/projects/**/*.jsonl` | `providerData.rawUsage` 含 credits 与 token |
| 16 | `cc-proxy` | `~/.token-stats/cc-proxy-usage.jsonl` | 内置 CC 代理写入；**同时服务 DimAgent 与 ZCode 的 `commandcode` 通道** |
| 17 | `dim-agent` | `~/.token-stats/workbuddy-usage.jsonl` | CPA `workbuddy` 插件写入（DimAgent → 腾讯 CodeBuddy Web API）；`provider=codebuddy` |
| 18 | `ollama-proxy` | `~/.token-stats/ollama-usage.jsonl` | CPA `ollama-usage` 插件写入（`ollama/` 前缀上游），含 TTFT/TPS |
| 19 | `stepfun-proxy` | `~/.token-stats/stepfun-usage.jsonl` | CPA `stepfun-usage` 插件写入（`step/` 前缀上游），含 TTFT/TPS |

路径大多有 `*_PATH` / `*_LOG_PATH` 环境变量覆盖，逐条见
[`docs/agents/environment-variables.md`](docs/agents/environment-variables.md)。

### loopback 代理与 CPA 插件拓扑

| systemd 服务 | 监听 | 谁在调用 | 写入的 `source` |
|--------------|------|----------|----------------|
| `token-stats-grok-proxy.service`（`--grok-proxy-only`） | `127.0.0.1:3434` | Grok CLI；dim 的 `grok-build-proxy` provider | `grok-cli` |
| `token-stats-cc-proxy.service`（`--cc-proxy-only`） | `127.0.0.1:8787` | DimAgent、ZCode 的 `commandcode` 通道 | `cc-proxy` |
| `token-stats-workbuddy.service`（CLIProxyAPI + `workbuddy.so` / `ollama-usage.so` / `stepfun-usage.so`） | `127.0.0.1:8317` | DimAgent（模型前缀 `wb/`、`ollama/`、`step/`） | `dim-agent` / `ollama-proxy` / `stepfun-proxy` |

**CPA = CLIProxyAPI（`:8317`）**，下文简称 CPA。

### 必须始终遵守的不变式

这几条会在无人提示的情况下被改坏，故留在此处（详解见
[`docs/agents/data-sources.md`](docs/agents/data-sources.md)）：

1. **`source` 是传输通道 id，不是显示名。** 由写日志的一方固定，用于去重指纹、迁移谓词与
   增量解析。**不要**为了 UI 好看改它——历史和新记录会分属两个 source、指纹不同 → 双计。
   UI 文案只在 `frontend/src/lib/utils.ts` 的 `SOURCE_LABELS` / `SOURCE_COLORS` 里映射，
   新增来源必须补这两张表（否则原样显示 id）。
2. **每个 CPA 上游都要配套一个 token-stats 用量插件。** CPA 通道只有自己的 usage 插件能
   逐请求计量（dim 本地库对 CPA 通道只有按 run 的聚合行，且已被回环地址排除规则丢弃）。
   加新上游时四步一起做：① `config.yaml` 的 `openai-compatibility` 条目（`prefix` 命名空间
   + 显式 `models:`）② fork 一个 `*-usage-plugin`（写日志前 `TrimPrefix` 剥回裸模型名以对齐
   pricing 与 vendor_merge 的键）③ 后端新增 `sources/<name>_proxy.rs` 并注册 ④ dim 侧无需
   改动（排除按 baseUrl 地址判定）。
3. **同一次调用只能有一个计量点。** 已有逐请求源的通道必须从 run 粒度聚合里排除，漏配就是
   双计（tokens / 成本 / 调用次数全部翻倍）。`dim` 源同时按**名字白名单**与 **baseUrl 是否
   指向 CPA 回环**（`CPA_LOOPBACK_ADDRS`）两条规则排除；ZCode 侧按 `PROXY_METERED_PROVIDERS`
   排除内置 cc-proxy 通道。给通道改名会换掉匹配键 → 排除静默失效。
4. **切换计量点要留 cutoff。** 代理日志是 append-only，首行时间即切换点；早于它的 run 粒度
   行保留为历史，之后的丢弃 + 一次性 store 迁移。cutoff 未知时**不截断**，避免丢历史。
5. **源库一律只读。** ccswitch / opencode / zcode / dimcode 全部以 `SQLITE_OPEN_READ_ONLY`
   打开（陷阱 3）。

### 配额卡索引（`GET /api/quota`）

端点、认证细节与逆向结论见 [`docs/agents/quota-cards.md`](docs/agents/quota-cards.md)。

| 卡 | 凭据来源 | 代码 |
|----|----------|------|
| Kimi / Kimi EX | `KIMI_CREDENTIALS_PATH(_EX)` | `quota/kimi.rs` |
| OpenCode Go / EX | `OPENCODE_GO_WORKSPACE_ID(_EX)` + `OPENCODE_GO_AUTH_COOKIE(_EX)` | `quota/opencode.rs` |
| Xiaomi MiMo | `XIAOMI_MIMO_SERVICE_TOKEN` + `XIAOMI_MIMO_USER_ID` | `quota/xiaomi_mimo.rs` |
| Command Code | `~/.commandcode/auth.json` 的 `apiKey`（回退 `COMMANDCODE_SESSION_TOKEN`） | `quota/commandcode.rs` |
| CodeBuddy 套餐 | `CODEBUDDY_SESSION_COOKIE` + `_2`（**两者必需**，易过期） | `quota/codebuddy.rs` |
| Ollama Cloud | `OLLAMA_API_KEY`（主，同上上游 key）/ `OLLAMA_AUTH_COOKIE`（抓 `/settings` 网页端**重置时间** + 回退路径）；**「本周」用量由本地 `ollama-proxy` 计量，重置时间以网页端为准**（API 只给百分比；weekly 是日历对齐窗口，本地推算会漂移，见 [`docs/agents/quota-cards.md`](docs/agents/quota-cards.md)） | `quota/ollama.rs` |
| Meituan LongCat | `MEITUAN_AUTH_COOKIE` | `quota/meituan.rs` |
| Fenno / EX | `FENNO_AUTH_TOKEN` + `FENNO_REFRESH_TOKEN`（自动轮换持久化） | `quota/fenno.rs` |
| Grok | 由 `grok-cli` 记录推算 | `quota/grok.rs` |
| Ainaiba 余额 | `YAI_API_KEY`（`GET /api/ainaiba-credit`） | `ainaiba.rs` |
| ZAI | `ZAI_API_KEY` | `quota/zai.rs` |
| DimAgent | 主路径本地 `dim usage --json`（无需 env）；回退 `DIMAGENT_SESSION_COOKIE` | `quota/dimagent.rs` |
| StepFun credit 余额 + Step Plan 月池 | `STEPFUN_API_KEY`（官方 `/v1/accounts`）；套餐剩余% 另需 `STEPFUN_OASIS_TOKEN`+`_WEBID`（控制台 Connect-RPC，成对；JWT 约 30 分钟，`RefreshToken` 轮换后写入 `stepfun-auth.json`） | `quota/stepfun.rs` |
| ZCode | `~/.zcode/v2/config.json` 里的编码套餐 apiKey（可被 `ZCODE_BIGMODEL_USAGE_API_KEY` 覆盖） | `quota/zcode.rs` |

### 前端结构（`frontend/`）

- Vite + React 19 + TypeScript，Tailwind CSS v4（`@tailwindcss/vite` 插件），Recharts，Lucide React。
- 构建产物输出到 `../backend/static`；Vite `base: "/token-stats/"`。
- `App.tsx`（约 1400 行）负责布局编排、全局状态、懒加载三个 section；重图表组件按需
  `lazy()` 分包。
- 主要组件：`TopBar`（含 section 切换）、`Sidebar`（筛选器）、`GlanceBand`、`KpiStrip`、
  `QuotaChips`、`SettingsDrawer`、`TpsChart`、`sections/UsageSection`、
  `sections/QuotasSection`、`sections/RequestsSection`。
- 工具库：`lib/utils.ts`（格式化、日期、来源颜色）、`lib/timeRange.ts`（预设区间）、
  `lib/filterState.ts`（筛选状态）、`lib/pivotTable.ts`（透视表）、`lib/quotaCards.ts`、
  `lib/fennoQuota.ts`、`lib/resizableColumns.ts`、`lib/subscriptionCycle.ts`。

---

## 后端关键文件

| 文件 | 职责 |
|------|------|
| `src/main.rs` | CLI 入口（`--grok-proxy-only`、`--cc-proxy-only`、`-l/--log-level`） |
| `src/app.rs` | `AppState`、`build_router()`、`serve()`（SIGINT/SIGTERM 优雅退出 + 落盘） |
| `src/models.rs` | `TokenRecord`、`StatsResponse`、`AggregatedStats` 等全部数据结构 |
| `src/sources/mod.rs` | `DataSource` trait、`load_all_sources()`/`load_changed_sources()`、跨源规范化（去重、模型名归一、vendor merge、Kimi 模型升级） |
| `src/sources/*.rs` | 各数据源解析器（说明见 [`docs/agents/data-sources.md`](docs/agents/data-sources.md)） |
| `src/aggregator.rs` | 过滤、聚合（overall/vendor/date/model/source）、RPM/TPS、排序、分页 |
| `src/routes.rs` | Axum 处理器与查询参数类型 |
| `src/store.rs` | 专用 SQLite 持久化：schema、指纹去重插入、整库恢复、一次性迁移 |
| `src/pricing.rs` | 实时成本计算：模型价格、USD→CNY、分段汇率、特殊规则 |
| `src/dim_entitlement.rs` | Dim 账号 entitlement 折扣（rate/rate_windows）的读取、分段历史与按时刻折算 |
| `src/config.rs` | vendor merge 配置加载与应用 |
| `src/settings.rs` | 高级模型 / 订阅设置持久化（JSON） |
| `src/ainaiba.rs` | Ainaiba 余额查询 |
| `src/grok_proxy.rs` | loopback Grok usage 代理（双上游路由） |
| `src/cc_proxy.rs` | loopback Command Code 代理（OpenAI ↔ CC 协议转换，供 DimAgent 使用） |
| `src/quota/*.rs` | 各类配额/订阅抓取（见上方配额卡索引） |
| `src/xunfei/` | 讯飞订阅查询 |
| `src/time.rs` | 时间边界解析与时区换算 |

### 前端关键文件

| 文件 | 职责 |
|------|------|
| `src/App.tsx` | 单页仪表盘编排：全局筛选状态、section 切换、懒加载、配额轮询 |
| `src/api.ts` | API 客户端 + 与后端匹配的 TypeScript 类型 |
| `src/lib/utils.ts` | 格式化助手、日期工具、来源颜色/名称映射（`SOURCE_COLORS`/`SOURCE_LABELS`） |
| `src/components/sections/*.tsx` | 用量 / 配额 / 请求三个区块 |

---

## API 端点

所有端点接受 `tz_offset`（距 UTC 的分钟数，如 UTC+8 → `480`）。

| 端点 | 说明 |
|------|------|
| `GET /api/stats?from=&to=&source=&provider=&model=&tz_offset=&resolution=` | 完整聚合：overall + by_vendor + by_date + by_model + by_source；`resolution` 支持 `day`（默认）/`4h`/`1h` |
| `GET /api/requests?from=&to=&provider=&model=&source=&page=&limit=&tz_offset=&show_zero_tokens=` | 分页原始请求，按时间倒序；默认排除零 token 记录（如 429），`show_zero_tokens=true` 包含 |
| `GET /api/filters` | 可用 vendors / models / sources |
| `GET /api/rpm?from=&to=&gap_threshold=` | 每分钟请求数分析（活跃窗口边界检测，阈值默认 5 分钟） |
| `GET /api/tps?from=&to=&models=` | 每秒 token 分析（`models` 为逗号分隔模型列表） |
| `GET /api/quota` | 全部配额卡（详见[`docs/agents/quota-cards.md`](docs/agents/quota-cards.md)） |
| `GET /api/xunfei` | 讯飞订阅用量 |
| `GET /api/pricing` | 当前定价配置（模型、汇率、特殊规则、`dim_entitlement_rates`） |
| `POST /api/pricing/reload` | 不重启热加载 `pricing.toml` |
| `GET /api/export` | 以 JSONL 导出全部记录 |
| `POST /api/refresh` | 手动触发后台刷新 |
| `POST /api/restore` | 从 JSONL 备份恢复（会并入 store） |
| `GET /api/store/info` | store 状态（含 `pending_records` 未落盘数） |
| `POST /api/store/restore` | 从 SQLite 整库恢复内存 |
| `GET /api/ainaiba-credit` | Ainaiba 余额 |
| `GET/POST /api/settings/advanced-models` | 高级模型编辑（JSON，`ADVANCED_MODELS_CONFIG` 可覆盖路径） |
| `GET/POST /api/settings/subscriptions` | 订阅设置（Kimi 倍率等，`SUBSCRIPTION_SETTINGS_CONFIG` 可覆盖路径） |

### 时间边界格式

`from`/`to` 接受：
- 日期：`2025-05-17`（上界为**包含**整天）
- 日期时间：`2025-05-17T14:30` 或 `2025-05-17T14:30:00`（上界为排他式比较）

### 筛选行为

- `source` / `provider` / `model` 均接受逗号分隔多选。
- 空字符串或省略 = 不过滤；前端在"全部"时发送空字符串。

---

## 数据模型

### `TokenRecord`（核心）

```rust
pub struct TokenRecord {
    pub date: String,               // "2025-05-17"
    pub time: String,               // RFC3339 UTC
    pub api_key_prefix: String,     // JSON 字段名 apiKeyPrefix
    pub provider: String,           // 如 "openai"、"anthropic"、"deepseek"（vendor merge 后）
    pub original_provider: Option<String>, // merge 前的原始 provider（cost 计算依据，不序列化）
    pub model: String,              // 如 "gpt-5.5"、"claude-sonnet-4-6"
    pub source: String,             // 数据源标识，见数据源清单
    pub input_tokens: i64,          // JSON: inputTokens（"非缓存输入"语义，见归一化）
    pub output_tokens: i64,         // JSON: outputTokens
    pub cache_read_tokens: i64,     // JSON: cacheReadTokens
    pub cache_write_tokens: i64,    // JSON: cacheWriteTokens
    pub total_tokens: i64,          // JSON: totalTokens
    pub cost: f64,                  // 原始币种存放；展示时由 pricing::display_cost() 换算
    pub ttft_ms: Option<f64>,       // JSON: ttftMs
    pub tps: Option<f64>,           // JSON: tps
}
```

### 缓存命中率

```
cache_hit_ratio = cache_read_tokens / (input_tokens + cache_read_tokens) × 100%
```

- `input_tokens` = **仅非缓存输入**（归一化后）；`total_tokens` = input + output + cache_read + cache_write。

**缓存语义归一化（统一为 Anthropic 约定）**：

| 来源 | 原始约定 | 解析器处理 |
|------|---------|-----------|
| Codex / Qoder（`qoder-cli`、`qoder-desktop`） | OpenAI：`input_tokens` **包含** cache read | 减去：`effective_input = input_tokens - cache_read_tokens` |
| Dim（console API） | OpenAI/OpenCode 式：`prompt_tokens` **包含** cache（`cache_tokens`） | 减去：`effective_input = prompt_tokens - cache_tokens`；`cache_write = 0`（API 无 cache 写入字段） |
| Command Code `cmd` | OpenAI：`inputTokens` **包含** `cacheReadTokens` | 解析时减去（存原始 input 会双计缓存且把命中率封顶在 50%） |
| Pi-via-Command-Code | 同上 | 在 `load_all_sources()` 中减去 |
| Claude Code / Kimi CLI | Anthropic：已排除 | 无需处理 |

**跨源处理**（`load_all_sources()` 内，顺序敏感）：
1. 交叉去重：`deepseek-ai`（DeepSeek 平台导出日报）与 `opencode`（OpenCode DB）在同日同
   provider/model 且 token 总数差 <5% 时，移除 `deepseek-ai` 记录。
2. 模型名归一：`claude-opus-4.7` → `claude-opus-4-7`；`grok-4.5-build` → `grok-4.5`；
   讯飞 ID（`xopglm5`、`xopglm51`、`xopkimik26` 等）映射到公开模型名。
3. Command Code Pi 记录缓存减法。
4. vendor merge（见下）。
5. Kimi 模型升级：`provider=kimi` 且 `model=kimi-for-coding`、时间 ≥ 2026-06-12T10:00:00Z
   的记录改名为 `kimi-k2.7`（**必须在 vendor merge 之后**，因为 pi 记录先被合并为 `kimi`）。

### 供应商合并（vendor_merge.toml）

**配置文件**：`backend/vendor_merge.toml`（二进制旁自动探测，或 `VENDOR_MERGE_CONFIG` 覆盖）。

```toml
[[vendor_group]]
name = "kimi"
providers = ["kimi", "kimi-coding", "kimi-code"]

[[vendor_group]]
name = "ainaba"
providers = ["openai", "ainaiba", "xai"]
```

当前合并组：`kimi`、`ainaba`、`ollama`、`fenno`、`FreeModel`、`deepseek`。
- 每个 `[[vendor_group]]`：`name` 为规范名，`providers` 为被合并的原始名。
- 合并发生在 `load_all_sources()` 末尾，落库之前；缺失配置时优雅降级（不合并）。
- 合并不可逆：`original_provider` 保留 merge 前名称，供 `display_cost()` 选择计费公式。

---

## 设计决策与约定

### 后端

1. **UTC 内部统一** — 时间存 RFC3339 UTC；本地时区只在聚合/展示时通过 `tz_offset` 应用。
2. **优雅降级** — 某数据源缺失只记 warning，其余源照常解析。
3. **无鉴权** — 本地仪表盘，不设认证；仅绑定 `0.0.0.0` 由 nginx 暴露。
4. **增量解析** — `DataSource::data_files()` 报告源文件，mtime+size 未变则跳过；一次性跨源
   规范化仍每次执行（只作用于新记录，开销小）。
5. **单一定价入口** — `pricing::display_cost()` 统一输出 CNY；`cost` 字段保留原始币种。
6. **内存画像依赖三样东西**（详见 [`pitfalls.md`](docs/agents/pitfalls.md) 第 24 条）—
   `#[global_allocator] mimalloc`（`main.rs`）、单元里的 `MALLOC_ARENA_MAX=2`、以及
   `TokenRecord` 的 `date`/`api_key_prefix`/`provider`/`model`/`source` 用 `CompactString`
   （≤22 字节内联，只有 `time` 留 `String`）。三者共同把 762k 条记录的常驻从 1109 MB 压到
   300 MB；把字段改回 `String` 或去掉分配器覆盖都会**静默**涨 3 倍以上（不是泄漏，
   是 glibc arena 只借不还）。改 `TokenRecord` 字段类型必须同步 `store.rs` 的
   `params![...]`（`.as_str()`）与 `row_to_record`（走 `text_col` 的 `ValueRef` 直读）。

### 数据持久化（SQLite）

- **位置**：`~/.config/token-stats/token-stats.db`（`TOKEN_STATS_DB_PATH` 覆盖）。
  让历史在清理原始会话文件后依然存在。
- **生命周期**：`AppState::new()` 从 store 恢复记录、摄入尚未持久化的源记录（启动时一次
  写），随后从 DB 载入内存。`refresh_records()` 把新发现的记录**立即发布到内存**（前端永远
  读到最新），同时排队到 `PendingBuffer` 延迟写盘：后台 flush 任务每 2 分钟
  （`FLUSH_DELAY`，`app.rs`）批量落盘一次；SIGINT/SIGTERM 时 `serve()` 停后台任务并把队列
  一次性写完。因此 **内存 = DB + 未落盘队列**；`GET /api/store/info` 的 `pending_records`
  报告差距。
- **指纹去重**：`TokenRecord::fingerprint()`（time、provider、model、source、
  input_tokens、output_tokens、cache_read_tokens 的哈希）为内存与 DB 共用的去重键。
- **失败处理**：插入批次失败回滚并重新排队（记录仍在内存可见；源日志作为兜底）。
- **恢复**：启动自动；另提供 `POST /api/store/restore`（整库重读回内存）与
  `GET /api/store/info`（状态）。`POST /api/restore` 恢复的 JSONL 备份也会并入 store。
- **注意**：存储的是**最终归一化后**形态（vendor merge、模型归一已应用）。修改
  `vendor_merge.toml` 只影响之后摄入的记录，不追溯已持久化历史（汇率分段则相反——
  成本按记录时间实时换算，改 `pricing.toml` 会作用于全部历史显示）。

### 前端

1. **中文 UI** — 文案统一走 `ZH` 常量对象；新增文案保持中文。
2. **来源配色** — 每工具来源有固定色（`lib/utils.ts` 的 `SOURCE_COLORS`），新增来源时扩展。
3. **成本展示** — 全部显示为 CNY（¥）；后端 `cost` 保留原始币种，由 `display_cost()` 换算。
4. **预设时间范围** — 今天 / 6h / 12h / 1d / 3d / 7d / 14d / 30d / 全部 / 自定义。
5. **状态记忆** — 筛选状态、当前 section、隐藏配额卡、告警忽略等都持久化到 localStorage。
6. **配额告警** — 额度低 / 24h 内到期时出告警条，可忽略 24h。

---

## 成本计算

**全部分支、实测费率与对账口径 → [`docs/agents/pricing.md`](docs/agents/pricing.md)。**
改 `backend/pricing.toml` 后执行 `./scripts/reload-pricing.sh` 热生效。

`display_cost()` 按 `original_provider` 选择计费公式，当前分支速查：

| 计费分支 | 触发（provider / source） | 公式要点 |
|----------|--------------------------|----------|
| 通用 `[[model]]` | 大部分（USD/1M × 分段汇率） | `tier_threshold` 长上下文档、`effective_from` 时间分段 |
| CNY 直价 | DeepSeek | `input_cny/output_cny/cache_read_cny/cache_write_cny`，**不**经过汇率 |
| 列表价 ÷ divisor | Command Code（`cc:` 前缀） | `commandcode_divisor`；部分模型有峰谷价 + `peak_weekdays_only` |
| credits × 单价 | `codebuddy`、`dim-agent` | `codebuddy_cny_per_credit` + `codebuddy_credit_segments` 分段 |
| API 原价 ÷ 倍率 | `kimi` | `kimi_subscription_multiplier`（设置抽屉可调） |
| cost ÷ divisor | `opencode` | `opencode_divisor` + `opencode_model_segments`；**2.x 记录 cost=0 → 改走 token 价**，免费模型须登记 0 价 |
| 目录原价 × entitlement rate | `dim` | `[[dim_model]]` 只登记**原价**，折扣走 `dim_entitlement.rs` 的 rate/rate_windows |
| 平台倍率 ÷ 分段 divisor | Ainaba | `ainaba_platform_rate` / `ainaba_segments` |
| 免费 | `qoder` | `[special] qoder_free` → ¥0（不是 N/A） |
| 平台实收费率 | `zai` | `[[zai_model]]`，1 元 = 1 美元额度 |
| 列表价 ÷ 套餐 divisor | `stepfun` | `stepfun_plan_divisor`（¥99 买 ¥1600 额度） |
| 积分公式 | `bigmodel`（source `zcode`） | BigModel 官方积分公式 × `zcode_cny_per_credit`，含峰谷/夜间免扣窗口 |
| 按次 / 话单 | 讯飞、Xiaomi MiMo、Meituan、FreeModel、Fenno、Grok、Ollama | 各自 `*_divisor` / `xunfei_per_call` / 经验费率 |

---

## 新增数据源步骤

1. `backend/src/sources/` 新增模块，实现 `DataSource` trait：
   - 返回 `Vec<TokenRecord>`；设置正确的 `source` 标识；
   - 缓存语义归一化到"非缓存输入"约定（减法）；
   - 文件缺失返回空 vec（优雅降级）；实现 `data_files()` 以启用增量解析。
2. `sources/mod.rs`：声明模块、`pub use`、加入 `load_sources_impl()` 的 sources 列表。
3. 前端 `lib/utils.ts`：`SOURCE_COLORS` + `SOURCE_LABELS` 增加该来源。
4. 若走 CPA（`:8317`）：还要配 `config.yaml` 条目 + usage 插件 + dim 侧排除，见
   上方「必须始终遵守的不变式」第 2 条。
5. 验证：启动仪表盘确认新数据出现；跨源去重/归一化如有需要同步加到
   `load_all_sources()`。
6. 回写文档：本文件的精简清单 + [`docs/agents/data-sources.md`](docs/agents/data-sources.md)
   的完整表格各加一行。

## 新增 API 端点步骤

1. `models.rs` 定义响应模型。
2. `aggregator.rs` 添加聚合逻辑（如需）。
3. `routes.rs` 添加处理器 + `Query<YourQuery>` 结构。
4. `app.rs` 的 `api_routes` 注册路由。
5. `frontend/src/api.ts` 添加 TypeScript 接口 + fetch 函数。
6. `App.tsx` 或对应 section 消费。

---

## 构建与开发

```bash
# 快速开发（后端运行，前端用预构建产物）
./start.sh

# 完整安装（nginx + systemd）
./setup.sh

# 零停机部署 —— 用这个，不要直接跑 ./deploy.sh（见陷阱 17）
./scripts/deploy-dashboard.sh

# 手动构建
(cd backend && cargo build --release)
(cd frontend && npm install && npm run build)  # 输出到 ../backend/static

# 直接运行后端
cd backend && ./target/release/token-stats-backend

# 仅运行 Grok 用量代理（由 token-stats-grok-proxy.service 使用）
cd backend && ./target/release/token-stats-backend --grok-proxy-only
```

环境变量：核心几个列在下面，**完整清单（含全部凭据/路径覆盖）见
[`docs/agents/environment-variables.md`](docs/agents/environment-variables.md)**。

| 变量 | 默认 | 说明 |
|------|------|------|
| `PORT` | `3000` | 后端端口（蓝绿切换为 3000 ↔ 3001） |
| `RUST_LOG` | - | 日志级别（`info`、`debug`、`trace`） |
| `REFRESH_INTERVAL_SECS` | `30` | 数据刷新间隔 |
| `TOKEN_STATS_DB_PATH` | `~/.config/token-stats/token-stats.db` | 专用 SQLite 持久化库 |
| `PRICING_CONFIG` | 二进制旁 `pricing.toml` | 定价配置路径 |
| `VENDOR_MERGE_CONFIG` | 二进制旁 `vendor_merge.toml` | 供应商合并配置路径 |

---

## 常见任务

### "加一张图表"
- 聚合逻辑在 `aggregator.rs`（后端）或 `App.tsx`/section 内 `useMemo` 变换前端数据。
- Recharts 组件（`BarChart`、`LineChart`、`ComposedChart`、`PieChart`、`AreaChart` 等）
  包在 `<ResponsiveContainer width="100%" height={...}>` 中；tooltip 风格复用
  `CustomTooltip`/各 section 自有 tooltip。
- 图表重 → 放独立 `components/` 里用 `lazy()` 分包。

### "加一个筛选器"
- 后端：`routes.rs` 的 `StatsQuery`/`RequestsQuery` 加参数 → `aggregator.rs` 的
  `FilterCriteria`/`filter_records` 扩展 → `App.tsx` 加 UI 控件 → `api.ts` 透传。
- 多选参数逗号分隔；空串表示"全部"。

### "修时区问题"
- 后端：`tz_offset` → `FixedOffset`，`local_date_for_record()` 把 UTC 转换为本地日期；
  仅日期边界含整天（上界含）。
- 前端：`getTimezoneOffset()` 返回负分钟（UTC+8 = `-480`），
  `tzOffset = -new Date().getTimezoneOffset()` = `480`。

### "调定价"
- 先读 [`docs/agents/pricing.md`](docs/agents/pricing.md)；编辑 `backend/pricing.toml`，
  然后 `./scripts/reload-pricing.sh`（等价 `curl -X POST /token-stats/api/pricing/reload`）。
- 注意分段语义：`effective_from` 按记录时间生效；实时展示（配额卡）永远用最新段。
  **改基准值会追溯改写全部历史**，换套餐务必加分段。
- **dim 源的费率必须用平台账本复核**（它不存原始 cost，写错不报错、只静默偏）：
  `./scripts/verify-dim-billing.py [--model X] [--interval hourly|--both]`。
  账本支持 `interval=hourly`，所以**当天就能验**，不必等隔日结算。
  **公开价目卡（`/api/public/website/plans?line=dimcode`）的字段顺序不保证等于结算口径**——
  `mimo-v2.6-flash` 09-28 前是反的、09-29 起才与价目卡一致（见 pricing.md）。

### "样式"
- Tailwind v4；自定义主题色在 `index.css` 的 `@theme` 中（`--color-primary-*`）。
- 卡片模式：`bg-white rounded-xl border border-slate-200 p-5 shadow-sm`。
- 徽章：`bg-emerald-100 text-emerald-700`、`bg-amber-100 text-amber-700`、
  `bg-slate-100 text-slate-600`。

---

## 陷阱与注意事项

全部编号陷阱（1–25，共 24 条；编号被提交信息和代码注释引用，故保持不重排）见
[`docs/agents/pitfalls.md`](docs/agents/pitfalls.md)。按主题该先读哪几条：

| 你在动 | 必读陷阱 |
|--------|----------|
| 前端构建 / nginx / base path | 1、2 |
| 任何源库读取（SQLite） | 3、23 |
| `sources/dim.rs`、双计防护 | 4、15、19、22 |
| CPA 插件 / `.so` / dim 凭据 | 13、14 |
| 聚合 / 请求明细 / 零 token 记录 | 5、7、8 |
| 定价 / `pricing.toml` | 6、11、18、21、25（+ [`pricing.md`](docs/agents/pricing.md)） |
| 部署 / systemd / 凭据注入 | 9、16、17、22 |
| `vendor_merge` 与历史数据 | 10、12 |
| `TokenRecord` 字段类型 / 分配器 / 常驻内存 | 24 |

---

## 文档维护约定

- 本文件只保留**索引 + 不变式**（当前约 480 行 / 30KB；显著超过就该再拆一轮）。
  细节写进 `docs/agents/` 对应文件。
- 新增内容时先问一句：「改代码时需要立刻知道，还是查的时候再看？」——前者进本文件，
  后者进 `docs/agents/`。
- 判断标准是**会不会静默出错**：会导致双计、丢历史、计费错算的约束必须留在本文件的
  「必须始终遵守的不变式」里，不能只放在按需展开的文档中。
- 两处都写的细节，以本文件为简版、`docs/agents/` 为详版；改时记得同步，避免漂移。
