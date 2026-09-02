# 项目上下文：Token Stats Dashboard（供 AI 编码代理使用）

> 本文件是面向 AI 编码代理的项目上下文说明，描述架构、数据源、API、数据模型与约定。
> 修改代码时请先阅读本文件；内容若与实际代码不符，请优先以代码为准并同步更新本文件。

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
- 后端还内置一个可选 loopback Grok 用量代理（`grok_proxy.rs`），单独以
  `--grok-proxy-only` 运行（systemd 服务 `token-stats-grok-proxy.service`）。

### 数据源清单

| # | 数据源（`source` 值） | 位置 | 格式/说明 |
|---|----------------------|------|-----------|
| 1 | `pi` | `~/.pi/token-logs/usage.jsonl` | JSONL；另扫描 Taskplane runtime `events-exit.json` / `exit-summary.json`（可用 `TASKPLANE_PROJECTS_DIR` 覆盖项目根） |
| 2 | `codex` | `~/.codex/sessions/*/rollout-*.jsonl` | JSONL，直接来自 Codex CLI |
| 3 | `claude-code` | `~/.claude/projects/*/*.jsonl` | JSONL，直接来自 Claude Code CLI |
| 4 | `opencode` | `~/.local/share/opencode/opencode.db` | SQLite，直接来自 OpenCode CLI |
| 5 | `kimi-cli` | `~/.kimi/sessions/*/wire.jsonl` | JSONL（`KIMI_SESSIONS_PATH` 可覆盖目录） |
| 6 | `kimi-code` | `~/.kimi-code*/sessions/*/*/agents/*/wire.jsonl` | JSONL（`KIMI_CODE_HOME` 可覆盖根目录） |
| 7 | `qoder` | `~/.qoder/projects/*/*.jsonl` | JSONL（`QODER_PROJECTS_PATH` 可覆盖） |
| 8 | `qoder-cn` | `~/.qoder-cn/logs/sessions/*/segments/*.jsonl` | JSONL（`QODER_CN_SESSIONS_PATH` 可覆盖） |
| 9 | `grok-cli` | `~/.token-stats/grok-usage.jsonl` | JSONL，由内置 loopback Grok 代理写入（`GROK_USAGE_LOG_PATH` 可覆盖） |
| 10 | `commandcode` | `~/.commandcode/projects/<slug>/<session-id>.jsonl` | JSONL；`type=message` 行含 `usage`；跳过侧车 `*.checkpoints.jsonl`（`COMMANDCODE_PROJECTS_PATH` 可覆盖） |
| 11 | `zcode` | `~/.zcode/cli/db/db.sqlite` | SQLite `model_usage` 表（`ZCODE_DB_PATH` 可覆盖） |
| 12 | `dsh` | `~/.dsh/sessions/*/session-*/session.jsonl.zstd` | zstd 压缩 JSONL，DeepSeek Harness；usage chunk 与 `finish` replayState 配对取 provider/model（`DSH_SESSIONS_PATH` 可覆盖） |
| 13 | `dim` | DimAgent 控制台 API `https://dimagent.cn/api/log/self` | **HTTP 轮询**（每刷新周期一次，默认 30s）：逐请求明细（time/model/prompt/completion/cache/ttft/tps），即控制台 Activity 页数据；`p` 分页 + `page_size` 上限 100 + `type=2`；不再读本地 SQLite（`DIM_DB_PATH` 已废弃）。旧 per-run 记录在首次成功同步后被一次性迁移清除 |
| 14 | `ccswitch` | `~/.cc-switch/cc-switch.db` | 仅当设置了 `USE_CC_SWITCH` 环境变量才加载（`CCSWITCH_DB_PATH` 可覆盖） |
| 15 | `codebuddy` | `~/.codebuddy/projects/**/*.jsonl` | JSONL；事件的 `providerData.rawUsage` 含 credits 与 token 用量（`CODEBUDDY_PROJECTS_PATH` 可覆盖） |

**Grok 代理说明**：`token-stats-grok-proxy.service` 用 `--grok-proxy-only` 启动后端二进制，
监听 `127.0.0.1:${GROK_PROXY_PORT:-3434}`，为 Grok CLI 提供 `/v1/responses` 转发
（YAI Router 与官方 xAI 双上游，别名 `grok-4.5-yai` / `grok-4.5-xai` 均重写为 `grok-4.5`），
从响应中提取 usage 追加到 `~/.token-stats/grok-usage.jsonl`。代理透传上游状态/响应体，
不记录 prompt、完成文本、请求头与凭据。

### 配额数据源（`GET /api/quota`）

| 卡 | 来源 | 配置 |
|----|------|------|
| Kimi / Kimi EX | `https://auth.kimi.com` 刷新 token 后查 `/usages` | `KIMI_CREDENTIALS_PATH` / `KIMI_CREDENTIALS_PATH_EX`；EX 默认指向 `~/.kimi-code-user2/credentials/kimi-code.json`；`KIMI_AUTH_BASE_URL` 可覆盖 |
| OpenCode Go / OpenCode Go EX | HTTP 抓取 `https://opencode.ai/workspace/{id}/go` 的 `<div data-slot="usage">`（`reqwest`+`scraper`） | `OPENCODE_GO_WORKSPACE_ID(_EX)` + `OPENCODE_GO_AUTH_COOKIE(_EX)` |
| Xiaomi MiMo | MiMo token 计划 API | `XIAOMI_MIMO_SERVICE_TOKEN` + `XIAOMI_MIMO_USER_ID` |
| Command Code | `https://api.commandcode.ai`（`/alpha/billing/subscriptions`、`/alpha/billing/credits`、`/alpha/usage/summary`）；主账号从 `~/.commandcode/auth.json` 的 `apiKey`（Bearer），第二账号（EX）从 `auth*.json`（如 `auth_frank.json`）；无 auth 文件时回退 `COMMANDCODE_SESSION_TOKEN` cookie（`/internal/*` 旧路由） | `COMMANDCODE_SESSION_TOKEN` 作为 `__Secure-commandcode_prod_.session_token` cookie（仅回退） |
| CodeBuddy 套餐 | `www.codebuddy.cn` billing meter API（`POST /billing/meter/get-user-resource-summary` 取各套餐包周期总量/剩余，`POST /billing/meter/get-user-resource` 取套餐名与周期；即 `/profile/plans-usage` 页同源接口）。**必需 `session` + `session_2` 两个 cookie**（单 `session` 返回 401）；边缘 WAF 拒绝过旧 Chrome UA（Chrome/126 被拦、152 可过）。cookie 从 Chrome 提取：`scripts/extract-codebuddy-cookies.sh`（约 30 天过期需重取） | `CODEBUDDY_SESSION_COOKIE` + `CODEBUDDY_SESSION_COOKIE_2`（仅 cookie 值） |
| Ollama Cloud | Ollama 云端 API | `OLLAMA_AUTH_COOKIE`（`__Secure-session=...`） |
| Meituan LongCat | 美团 API | `MEITUAN_AUTH_COOKIE`（`passport_token_key`） |
| Fenno / Fenno EX | `https://api.fenno.ai/api/v1/subscriptions/active` | `FENNO_AUTH_TOKEN` + `FENNO_REFRESH_TOKEN` 引导凭据管理器；轮换凭据持久化到 `FENNO_AUTH_STATE_PATH`（默认 `~/.config/token-stats/fenno-auth.json`）并自动刷新 |
| Grok | 基于 `grok-cli` 记录 + 订阅配额页面 | `grok_proxy.rs` 读取的用量记录；配额逻辑在 `quota/grok.rs` |
| Ainaiba 余额 | `api-xai.ainaibahub.com` | `YAI_API_KEY`（`/api/ainaiba-credit` 端点） |
| DimAgent | **主路径**：本地 `dim usage --json`（CLI 自动发现，见 `quota/dimagent.rs`；OAuth 凭据在 `~/.dimcode/v2/auth.json`，CLI 自动刷新，无需任何环境变量）。**回退**：console API `dimagent.cn/api`（`/me/subscription` + `/me/credits` + `/me/feature-meters` + `/user/quota-estimate`） | `DIMAGENT_SESSION_COOKIE`（浏览器 `session` cookie 值）仅用于回退和近 30 天统计增强；`DIM_USAGE_BIN` 可覆盖 CLI 二进制 |

**DimAgent console API 逆向结论**（`quota/dimagent.rs` / `sources/dim.rs` 验证过）：
- `GET /api/user/self`、`/api/log/self`（逐次调用明细：`prompt_tokens`/`completion_tokens`/`cache_tokens`/`use_time_ms`/`ttft_ms`/`tps`/`model_name`/`token_name`）、`/api/user/daily-stats`（按日汇总：各 token 字段 + `request_count` + `quota_consumed`）、`/api/me/subscription`、`/api/me/credits`、`/api/me/feature-meters`、`/api/user/quota-estimate` —— 全部只需 `session` cookie（GET）。
- **`/api/log/self` 分页参数是 `p`**（`page` 会被服务端忽略，总是返回第 1 页）；`page_size` 上限 100；`type=2` 为用量日志筛选（Activity 页同款）；响应按 id 倒序（新→旧），带 `total`/`total_capped`。
- token 约定为 OpenAI 式：`prompt_tokens` **包含** `cache_tokens`（用 `/api/user/daily-stats` 验证：`total_tokens = prompt_tokens + completion_tokens`）；`cache_tokens` 即缓存命中（读）；API 无 cache 写入字段（daily-stats `cache_creation_tokens` 恒为 0）。
- 两个 cookie 的作用：`session`（Flask/itsdangerous 签名会话，唯一认证凭据，必需）；`_c_WBKFRo`（站点统计/风控 cookie，**非认证必需**，可弃用）。
- 本地 dimcode 库（`usage_run_stats`）是**按 run 聚合**（含 input/output/cache/model/cost）；逐调用明细（TTFT/TPS/每次调用的缓存命中）只存在于 console API，`dim usage --json` 与本地库都没有。
- CLI 输出与 console API 的 units 单位不同：CLI 是整单位（如 1500），console API 是毫单位（×1000，如 1500000），`card_from_parts()` 按总量阈值自动归一化。

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
| `src/main.rs` | CLI 入口（`--grok-proxy-only`、`-l/--log-level`） |
| `src/app.rs` | `AppState`、`build_router()`、`serve()`（SIGINT/SIGTERM 优雅退出 + 落盘） |
| `src/models.rs` | `TokenRecord`、`StatsResponse`、`AggregatedStats` 等全部数据结构 |
| `src/sources/mod.rs` | `DataSource` trait、`load_all_sources()`/`load_changed_sources()`、跨源规范化（去重、模型名归一、vendor merge、Kimi 模型升级） |
| `src/sources/*.rs` | 各数据源解析器（见上表） |
| `src/aggregator.rs` | 过滤、聚合（overall/vendor/date/model/source）、RPM/TPS、排序、分页 |
| `src/routes.rs` | Axum 处理器与查询参数类型 |
| `src/store.rs` | 专用 SQLite 持久化：schema、指纹去重插入、整库恢复 |
| `src/pricing.rs` | 实时成本计算：模型价格、USD→CNY、分段汇率、特殊规则 |
| `src/config.rs` | vendor merge 配置加载与应用 |
| `src/settings.rs` | 高级模型 / 订阅设置持久化（JSON） |
| `src/ainaiba.rs` | Ainaiba 余额查询 |
| `src/grok_proxy.rs` | loopback Grok usage 代理（双上游路由） |
| `src/quota/*.rs` | 各类配额/订阅抓取（kimi、opencode、fenno、grok、ollama、meituan、commandcode、xiaomi_mimo、dimagent） |
| `src/xunfei/` | 讯飞订阅查询 |
| `src/time.rs` | 时间边界解析与时区换算 |

### 前端关键文件

| 文件 | 职责 |
|------|------|
| `src/App.tsx` | 单页仪表盘编排：全局筛选状态、section 切换、懒加载、配额轮询 |
| `src/api.ts` | API 客户端 + 与后端匹配的 TypeScript 类型 |
| `src/lib/utils.ts` | 格式化助手、日期工具、来源颜色映射（`SOURCE_COLORS`/`SOURCE_LABELS`） |
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
| `GET /api/quota` | 全部配额卡（kimi / opencode / xiaomi / commandcode / ollama / meituan / fenno / grok 等） |
| `GET /api/xunfei` | 讯飞订阅用量 |
| `GET /api/pricing` | 当前定价配置（模型、汇率、特殊规则） |
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
| Codex / Qoder / Qoder CN | OpenAI：`input_tokens` **包含** cache read | 减去：`effective_input = input_tokens - cache_read_tokens` |
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

## 成本计算（pricing.toml 重点）

`backend/pricing.toml` 控制一切价格与折扣，改完执行 `./scripts/reload-pricing.sh` 热生效。
要点：

- **分段汇率**：`[[usd_to_cny_segments]]` 按记录时间选段（无 `effective_from` 的为兜底段）；
  `usd_to_cny` = 最新段。每 2 周由 `scripts/update-exchange-rate.sh` 追加
  （幂等：距最后一段 <14 天跳过），可用 `--date --rate` 手动补录。
- **模型价格**：`[[model]]`（USD/1M），`tier_threshold` 触发长上下文档位；`effective_from`
  支持按时间分段；DeepSeek 用 `input_cny/output_cny/cache_read_cny/cache_write_cny`
  （CNY 定价，**不经过汇率换算**）；`yairouter_model` 是 Yairouter 专属覆盖（如 GPT-5.6
  于 2026-08-17 恢复原价，仅作用于该 provider）。
- **Command Code**：`cc:` 前缀模型为 Command Code 列表价（部分模型带 `peak_hours_utc`
  峰谷价，DeepSeek 2026-08-16 起实施）；实际成本 = 列表价 / `commandcode_divisor` → CNY。
- **CodeBuddy**：记录的 `cost` 保存原始 credits；实际成本 = credits × `codebuddy_cny_per_credit`
  （国内版连续包月活动价 70 元 / 4000 credits = 0.0175 元/credit，直接人民币计价，不经过汇率）。
- **Kimi 订阅**：`kimi_api_models`（CNY/1M，cache write 免费）+ `kimi_subscription_multiplier`
  （默认 20，设置抽屉可调、持久化）：`成本 = (input×in + cache_read×cr + output×out) / 1M / 倍率`。
- **OpenCode**：原始 cost / `opencode_divisor`（6.0）；`opencode_model_segments` 可按模型+
  时间覆盖 divisor（如 deepseek-v4 2026-08-18 起 divisor=3）。
- **Dim（console API 源）**：不存储原始 cost，`display_cost()` 走"衍生源"分支——按
  pricing.toml 每模型 token 单价估算（DeepSeek 为 CNY 直接计价，如 v4-flash
  input=1 / output=2 / cache_read=0.02 元每 1M）；无价格模型的记录显示 N/A。
- **Ainaba**：`USD × ainaba_platform_rate(7.0) / ainaba_segments 分段 divisor`（平台固定汇率，
  不随市场波动）。
- **订阅类折扣**：`freemodel_divisor`（=汇率/0.1）、`fenno_divisor`、`grok_divisor`；
  Ollama 用经验 per-token 价 + 模型倍率；讯飞订阅按次计
  （`xunfei_per_call` × 波谷系数 0.8，`peak_hours=[8,22]` + 节假日表）；
  Xiaomi MiMo / Meituan 按话单 per-token。
- **成本展示规则**（`display_cost()`）：`original_provider` 决定公式分支；无任何可用价格
  的非 pi 来源显示 "N/A"；pi 记录沿用其存储 cost（DeepSeek 为 CNY 原样，
  其余 USD 折算）。

---

## 新增数据源步骤

1. `backend/src/sources/` 新增模块，实现 `DataSource` trait：
   - 返回 `Vec<TokenRecord>`；设置正确的 `source` 标识；
   - 缓存语义归一化到"非缓存输入"约定（减法）；
   - 文件缺失返回空 vec（优雅降级）；实现 `data_files()` 以启用增量解析。
2. `sources/mod.rs`：声明模块、`pub use`、加入 `load_sources_impl()` 的 sources 列表。
3. 前端 `lib/utils.ts`：`SOURCE_COLORS` + `SOURCE_LABELS` 增加该来源。
4. 验证：启动仪表盘确认新数据出现；跨源去重/归一化如有需要同步加到
   `load_all_sources()`。

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

# 零停机部署（构建后蓝绿切换端口 3000 ↔ 3001）
./deploy.sh

# 手动构建
(cd backend && cargo build --release)
(cd frontend && npm install && npm run build)  # 输出到 ../backend/static

# 直接运行后端
cd backend && ./target/release/token-stats-backend

# 仅运行 Grok 用量代理（由 token-stats-grok-proxy.service 使用）
cd backend && ./target/release/token-stats-backend --grok-proxy-only
```

### 环境变量

| 变量 | 默认 | 说明 |
|------|------|------|
| `PORT` | `3000` | 后端端口 |
| `RUST_LOG` | - | 日志级别（`info`、`debug`、`trace`） |
| `REFRESH_INTERVAL_SECS` | `30` | 数据刷新间隔 |
| `TOKEN_STATS_DB_PATH` | `~/.config/token-stats/token-stats.db` | 专用 SQLite 持久化库 |
| `PRICING_CONFIG` | 二进制旁 `pricing.toml` | 定价配置路径 |
| `VENDOR_MERGE_CONFIG` | 二进制旁 `vendor_merge.toml` | 供应商合并配置路径 |
| `USE_CC_SWITCH` | 未设置 | 设置任意值即额外加载 cc-switch 数据 |
| `CCSWITCH_DB_PATH` | `~/.cc-switch/cc-switch.db` | cc-switch 库位置覆盖 |
| `KIMI_SESSIONS_PATH` | `~/.kimi/sessions` | Kimi CLI 会话目录覆盖 |
| `KIMI_CODE_HOME` | 未设置时自动发现全部 `~/.kimi-code*` 目录（如 `~/.kimi-code`、`~/.kimi-code-user2`） | Kimi Code 根目录覆盖（显式设置则只用该目录，兼容旧行为） |
| `KIMI_CREDENTIALS_PATH` | `~/.kimi-code/credentials/kimi-code.json`（优先，存在时）；回退 `~/.kimi/credentials/kimi-code.json` | 主账号凭据 |
| `KIMI_CREDENTIALS_PATH_EX` | `~/.kimi-code-user2/credentials/kimi-code.json` | EX（kimi2）账号凭据 |
| `KIMI_AUTH_BASE_URL` | `https://auth.kimi.com` | Kimi 认证基址 |
| `QODER_PROJECTS_PATH` | `~/.qoder/projects` | Qoder 会话目录覆盖 |
| `QODER_CN_SESSIONS_PATH` | `~/.qoder-cn/logs/sessions` | Qoder CN 会话目录覆盖 |
| `GROK_USAGE_LOG_PATH` | `~/.token-stats/grok-usage.jsonl` | Grok 用量日志覆盖 |
| `GROK_PROXY_PORT` | `3434` | loopback Grok 代理端口 |
| `GROK_YAI_UPSTREAM_BASE_URL` / `GROK_UPSTREAM_BASE_URL` | `https://api.yairouter.com` | Grok YAI 上游（兼容旧名 `GROK_UPSTREAM_BASE_URL`） |
| `GROK_XAI_UPSTREAM_BASE_URL` | `https://api.x.ai` | Grok xAI 上游 |
| `GROK_XAI_NETWORK_PROXY` | 未设置 | xAI-only 网络代理（如 `http://127.0.0.1:7800`），**不得**用通用 `HTTP_PROXY`（会同时影响双上游） |
| `COMMANDCODE_PROJECTS_PATH` | `~/.commandcode/projects` | Command Code 会话目录覆盖 |
| `CODEBUDDY_PROJECTS_PATH` | `~/.codebuddy/projects` | CodeBuddy 会话目录覆盖 |
| `COMMANDCODE_SESSION_TOKEN` | 未设置 | Command Code 配额卡 cookie 值（仅当无 `~/.commandcode/auth.json` 时作为回退） |
| `CODEBUDDY_SESSION_COOKIE` / `CODEBUDDY_SESSION_COOKIE_2` | 未设置 | CodeBuddy 套餐配额卡的 `session` / `session_2` cookie 值（两者必需；`scripts/extract-codebuddy-cookies.sh` 可从 Chrome 自动提取并输出 export 行） |
| `ZCODE_DB_PATH` | `~/.zcode/cli/db/db.sqlite` | ZCode 库位置覆盖 |
| `DSH_SESSIONS_PATH` | `~/.dsh/sessions` | DSH 会话目录覆盖 |
| `DIM_DB_PATH` | 已废弃 | 旧版 Dim 本地 SQLite 库路径，console API 源不再使用 |
| `TASKPLANE_PROJECTS_DIR` | `~/srcs` | Taskplane runtime 扫描根目录覆盖 |
| `OPENCODE_GO_WORKSPACE_ID(_EX)` | 未设置 | OpenCode Go 工作区 ID（配额卡必需） |
| `OPENCODE_GO_AUTH_COOKIE(_EX)` | 未设置 | OpenCode Go `auth` cookie（配额卡必需） |
| `XIAOMI_MIMO_SERVICE_TOKEN` / `XIAOMI_MIMO_USER_ID` | 未设置 | 小米 MiMo 配额卡凭据 |
| `OLLAMA_AUTH_COOKIE` | 未设置 | Ollama cloud 会话 cookie |
| `MEITUAN_AUTH_COOKIE` | 未设置 | 美团 LongCat `passport_token_key` |
| `FENNO_AUTH_TOKEN` | 未设置 | Fenno 初始访问 JWT（仅引导） |
| `FENNO_REFRESH_TOKEN` | 未设置 | Fenno 初始刷新 token（轮换后自动持久化） |
| `FENNO_AUTH_STATE_PATH` | `~/.config/token-stats/fenno-auth.json` | 轮换凭据状态文件 |
| `YAI_API_KEY` | 未设置 | Ainaiba/XAI 余额查询 Bearer token |
| `ADVANCED_MODELS_CONFIG` | `~/.config/token-stats/advanced-models.json` | 高级模型编辑存储 |
| `SUBSCRIPTION_SETTINGS_CONFIG` | `~/.config/token-stats/subscription.json` | 订阅设置存储 |
| `DIMAGENT_SESSION_COOKIE` | 未设置 | DimAgent 会话 cookie（仅值，不含 `session=` 前缀）。**必需**：`dim` 数据源用它轮询 console API（每次刷新循环，默认 30s）；配额卡也用它作 CLI 回退 + 近 30 天统计增强 |
| `DIM_USAGE_BIN` | 自动发现 | `dim usage --json` 二进制覆盖（仅配额卡主路径；默认扫描 `~/.dimcode/binaries/dimcode-linux-x64/*/bin/dimcode` 最新版 → PATH `dim`） |

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
- 编辑 `backend/pricing.toml`，然后 `./scripts/reload-pricing.sh`（等价
  `curl -X POST /token-stats/api/pricing/reload`）。
- 注意分段语义：`effective_from` 按记录时间生效；实时展示（配额卡）永远用最新段。

### "样式"
- Tailwind v4；自定义主题色在 `index.css` 的 `@theme` 中（`--color-primary-*`）。
- 卡片模式：`bg-white rounded-xl border border-slate-200 p-5 shadow-sm`。
- 徽章：`bg-emerald-100 text-emerald-700`、`bg-amber-100 text-amber-700`、
  `bg-slate-100 text-slate-600`。

---

## 陷阱与注意事项

1. **前端构建进后端目录** — `vite.config.ts` 的 `outDir: ../backend/static`；不要手工建
   `backend/static`。
2. **Base path** — 前端运行于 `/token-stats/`，API 调用走 `/token-stats/api/*`（Vite
   `base` 已处理）；nginx `location /token-stats/` 反代时**去掉前缀**转发到后端 `/`
   （`proxy_pass http://upstream/;` 尾斜杠重要）。
3. **SQLite 只读** — ccswitch / opencode / zcode 库均以 `SQLITE_OPEN_READ_ONLY`
   打开；**切勿写入**这些源库。dim 源已改为 console API 轮询，不再读任何本地
   SQLite（`~/.dimcode/v2/dimcode.sqlite` 只被 dim CLI 自身使用）。
4. **Dim console API 轮询** — `sources/dim.rs` 每次刷新循环（默认 30s）先拉第 1 页
   （`p=1&page_size=100&type=2`，`page` 参数会被服务端忽略），有新记录才继续翻页直
   到已见过的 id；页内 id 倒序。cookie 失效（401）时优雅降级为空并保留历史。
   启动时若完整回填成功，会对 store 做**一次性迁移**：删除旧的按 run 聚合的
   `source='dim'` 行（指纹不在 API 记录集合中的），避免与逐请求记录双重计数。
5. **Grok 记录不出现在请求明细** — 聚合包含 `grok-cli`，但 `paginate_requests` 明确排除
   该 source；detail 表永远不会显示 grok 单条记录。
6. **Kimi 成本是估算** — Kimi CLI/Code 不报原生 cost；按
   `kimi_api_models` API 原价 ÷ `kimi_subscription_multiplier` 估算。
7. **零 token 记录** — 默认从聚合与明细中排除（429 等）；`show_zero_tokens=true` 仅影响
   明细。`exclude_zero_tokens` 是 `FilterCriteria` 的统一开关。
8. **排序稳定性** — 请求按 time DESC，再 source ASC、provider ASC、model ASC。
9. **部署** — `deploy.sh` 蓝绿：构建 → 备用端口起新实例 → 健康检查 → 切 nginx upstream →
   排空旧实例；首次部署会把旧 `token-stats.service` 迁移为 `token-stats@.service`。
   Grok 代理是独立服务，**不随仪表盘蓝绿切换**（始终独占 3434）。
10. **vendor_merge 与历史** — 改合并组只影响新摄入记录；已持久化记录是合并后形态。
11. **设置类接口双路径** — 高级模型/订阅设置存 JSON（可被 `*_CONFIG` 环境变量重定向），
    与 `pricing.toml`（只读载入、`POST /reload` 热更）是两套机制，别混用。
12. **Codex 增量解析必须读 session_meta / turn_context** — `parse_files` 的 `subset`
    只表示“这次要重读哪些文件”，不是“跳过模型预扫”。跳过预扫会把每条用量写成
    `model=unknown`（provider 回落到 `openai` → vendor merge 成 `ainaba`），指纹不同
    于正确记录，会在 store 里堆出双份。启动时 `collapse_unknown_codex_twins` 按
    同时间+token 删除 unknown 行（不要求 provider 相同，因为增量路径曾把
    provider 错写成 openai→ainaba）；无孪生的 unknown 靠全量重解析再摄入正确模型后清除。
