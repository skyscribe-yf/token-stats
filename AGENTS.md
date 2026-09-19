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
| 7 | `qoder-cli` | `~/.qoder/logs/sessions/<project-slug>/<session-id>/segments/*.jsonl` | JSONL，国际版 `qoder`/`qodercli` CLI（v1.0.14）；只取 `type=model.response.completed` 事件并按 `request_id` 去重；OpenAI 式 `input_tokens` **含**缓存命中，解析时减去归一为 Anthropic 语义；provider 取日志的 `data.provider`，缺失时回落模型别名表（`qfmodel`/`qmodel_latest`/`efficient`/`auto`/`qmodel_38max`→`qoder`）（`QODER_SESSIONS_PATH` 可覆盖）。旧实现读 `~/.qoder/projects/*/*.jsonl`，新版 CLI 已不往那里写 `usage` → 恒 0 条 |
| 8 | `qoder-desktop` | `~/.qoder-cn/logs/sessions/<project-slug>/<session-id>/segments/*.jsonl` | 同一解析器（`sources/qoder.rs`），数据由 **Qoder Desktop**（`/opt/Qoder CN`，Electron，内嵌 agent SDK 1.1.49 写 `~/.qoder-cn`）产出。**token 恒为 0**：CN 网关对免费 Qwen3.8-Flash 路由不向客户端回传 usage 块（2026-09-18 实测 686/686 条 `model.response.completed` 全零、缓存字段同样为 0，应用自己的上下文快照也写着 `tokenCountsAvailable: false`；客户端没有 `include_usage`/feature gate 可打开），所以这个源**只有调用次数有意义**。为此新增 `TokenRecord::counts_as_call_without_tokens()`（仅 `qoder-desktop`），让统计/筛选/RPM 三处跳过 `is_zero_token` 过滤，否则全部记录会被当作失败请求丢弃（`QODER_CN_SESSIONS_PATH` 可覆盖） |
| 9 | `grok-cli` | `~/.token-stats/grok-usage.jsonl` | JSONL，由内置 loopback Grok 代理写入（`GROK_USAGE_LOG_PATH` 可覆盖） |
| 10 | `commandcode` | `~/.commandcode/projects/<slug>/<session-id>.jsonl` | JSONL；`type=message` 行含 `usage`；跳过侧车 `*.checkpoints.jsonl`（`COMMANDCODE_PROJECTS_PATH` 可覆盖） |
| 11 | `zcode` | `~/.zcode/cli/db/db.sqlite` | SQLite `model_usage` 表（`ZCODE_DB_PATH` 可覆盖）。**provider 映射**：`provider_metadata_json` 的计费名（`OpenCodeGo`→`opencode-go`、`Tokenrouter`→`tokenrouter`）；适配器名键（`anthropic`/`openai`——bigmodel 编码套餐走 Anthropic 协议）被跳过，无计费元数据的 `builtin:bigmodel-start-plan`（Weekend Build 体验套餐/3 亿 token 赠量）标 `bigmodel-start`（2026-09-13 起，成本恒 0——不扣正式套餐积分；启动迁移 `purge_zcode_start_plan_bigmodel` 在 `load_all()` **之前**删 cutoff `2026-09-13T01:30Z` 起的旧 `bigmodel` 行、由全量重解析按新指纹回灌防双计），其余无计费元数据的 `bigmodel*` 通道一律标 `bigmodel`，再其余回落 `opencode-go`。**通道 ID 带命名空间前缀**（`builtin:` 客户端内置套餐、`account:` 登录账号绑定的套餐，如 `account:bigmodel-individual-coding-plan`＝个人编码套餐），映射时**先剥掉 `:` 前的命名空间**再匹配（`*start-plan` 结尾 → `bigmodel-start`，`bigmodel` 开头 → `bigmodel`）——曾因只认 `builtin:` 前缀把账号绑定套餐的 GLM 流量全标成 `opencode-go`（vendor 图表错位 + 走 OpenCode Go 计费分支而非 BigModel 积分公式），启动迁移 `purge_zcode_account_plan_opencode_go` 在 `load_all()` 之前删 cutoff `2026-09-19T14:00Z` 起 `source='zcode' AND provider='opencode-go' AND model LIKE 'GLM%'` 的错标行、由全量重解析按新指纹回灌。曾因 `{"anthropic":...}` 元数据把整个 provider 误标成 `anthropic`，启动迁移 `purge_zcode_anthropic` 一次性清理并按新指纹重灌。**代理计量通道排除**：`provider_metadata_json` 的键其实是 ZCode 通道显示名，名为 `commandcode` 的那个通道就是内置 cc-proxy（ZCode → `127.0.0.1:8787`），代理已逐请求写 `cc-proxy-usage.jsonl`，故该通道 `model_usage` 行整批丢弃（`PROXY_METERED_PROVIDERS`，且**仅当代理日志非空**——没有代理时 zcode 行是唯一点量）；启动迁移 `purge_zcode_commandcode` 删除已持久化的 `source='zcode' AND provider='commandcode'` 行。坑：在 ZCode 里给该通道改名会换掉元数据键 → 排除失效、重新双计 |
| 12 | `dsh` | `~/.dsh/sessions/*/session-*/session.jsonl.zstd` | zstd 压缩 JSONL，DeepSeek Harness；usage chunk 与 `finish` replayState 配对取 provider/model（`DSH_SESSIONS_PATH` 可覆盖） |
| 13 | `dim` | DimAgent 控制台 API `https://dimagent.cn/api/log/self` + 本地 SQLite 补充 | **HTTP 轮询**（每刷新周期一次，默认 30s）：逐请求明细（time/model/prompt/completion/cache/ttft/tps），即控制台 Activity 页数据；`p` 分页 + `page_size` 上限 100 + `type=2`。**本地补充**：只读 `~/.dimcode/v2/dimcode.sqlite` 的 `usage_run_stats` 中第三方通道记录（如 `custom-ollama-cloud-042036d3` → `ollama-cloud`，vendor merge 并入 `ollama` 组；`grok-build` → `xai-official`，与 grok-cli 的 xAI 官方用量合并计费），并排除所有已有逐请求计量的通道避免双计：`dimcode-api-oauth`（与 API 源双计）、`workbuddy`（与 `dim-agent` 源双计）、`grok-build-proxy`（与 grok 代理日志双计）、`ollama-cloud-proxy`（与 `ollama-proxy` 源双计）、`cc-proxy`（与 `cc-proxy` 源双计——displayName "Command Code" 曾被 slug 成 provider `command-code` 单独出现在 vendor 图表中，2026-09-12 修复 + store 一次性迁移清除）（`DIM_LOCAL_DB_PATH` 可覆盖库路径；`DIM_DB_PATH` 已废弃）。旧 per-run 记录在首次成功同步后被一次性迁移清除。**冷启动 watermark**：store `sync_watermarks` 表的 `dim_console_last_id` 记录已见最大 id，`finish_sync` 在每次成功同步后持久化（2026-09-13 接线，此前该写入从未发生→每次冷启动全量翻页 ~279 页 ≈90s）；冷启动从 watermark 续读只拉增量。**注意**：legacy per-run 行的一次性 purge（`purge_dim_legacy`）门槛是 `full_backfill_done`（本进程做过**从零**全量回填）而非 `last_sync_completed`——从 watermark 续读得到的只是增量指纹集合，若当全量历史用会把全部历史 dim 行误删 |
| 14 | `ccswitch` | `~/.cc-switch/cc-switch.db` | 仅当设置了 `USE_CC_SWITCH` 环境变量才加载（`CCSWITCH_DB_PATH` 可覆盖） |
| 15 | `codebuddy` | `~/.codebuddy/projects/**/*.jsonl` | JSONL；事件的 `providerData.rawUsage` 含 credits 与 token 用量（`CODEBUDDY_PROJECTS_PATH` 可覆盖） |
| 16 | `cc-proxy` | `~/.token-stats/cc-proxy-usage.jsonl` | JSONL，由内置 loopback Command Code 代理写入（`CC_PROXY_USAGE_LOG_PATH` 可覆盖）；`provider=commandcode`（成本走 `cc:` 价格 ÷ `commandcode_divisor`），`model` 已剥 vendor 前缀。**该代理同时服务 DimAgent 与 ZCode 的 `commandcode` 通道**，两个客户端的逐请求用量都只在这里计量（ZCode 侧的 `model_usage` 重复行已被 zcode 源排除） |
| 17 | `dim-agent` | `~/.token-stats/workbuddy-usage.jsonl` | JSONL，由 workbuddy CLIProxyAPI 插件（`~/workbuddy-proxy`，systemd 服务 `token-stats-workbuddy.service`）写入——DimAgent 经 Tencent CodeBuddy Web API 的请求；`provider=codebuddy`（成本走 `codebuddy_cny_per_credit` 积分换算，与原生 codebuddy 源同一计费公式），`WORKBUDDY_USAGE_LOG_PATH` 可覆盖 |
| 18 | `ollama-proxy` | `~/.token-stats/ollama-usage.jsonl` | JSONL，由 `ollama-usage` CLIProxyAPI 插件（`~/workbuddy-proxy/ollama-usage-plugin`，与 workbuddy 同实例）写入——DimAgent 经 CPA `ollama-cloud` 上游（`ollama/` 前缀）的**逐请求**用量，含 TTFT/TPS；`provider=ollama-cloud`（vendor merge 并入 `ollama`，成本走经验费率），`OLLAMA_PROXY_USAGE_LOG_PATH` 可覆盖 |

**Grok 代理说明**：`token-stats-grok-proxy.service` 用 `--grok-proxy-only` 启动后端二进制，
监听 `127.0.0.1:${GROK_PROXY_PORT:-3434}`，为 Grok CLI 提供 `/v1/responses` 转发
（YAI Router 与官方 xAI 双上游，别名 `grok-4.5-yai` / `grok-4.5-xai` 均重写为 `grok-4.5`），
从响应中提取 usage 追加到 `~/.token-stats/grok-usage.jsonl`。代理透传上游状态/响应体，
不记录 prompt、完成文本、请求头与凭据。
**DimAgent grok-build 通道也走此代理**：dim 的 `grok-build` provider 因 `xai-grok-build`
driver 硬校验 OAuth 凭据只能发往 `https://*.x.ai`（`PROVIDER_TRANSPORT_CONFIG_ERROR`），
不能直接改 baseUrl 指向代理。改为自定义 provider `grok-build-proxy`
（`dim provider add grok-build-proxy --api-key placeholder --base-url http://127.0.0.1:3434/v1
--adapter openai-responses --model grok-4.6`），其 `openai-responses` driver 发
`{baseUrl}/responses` 命中代理；代理对裸模型名（`grok-4.5` / `grok-4.6`）路由到官方
xAI 上游，并从 `~/.dimcode/v2/auth.json` 的 `xaiGrokBuild.access` 注入真实 OAuth token
（占位 key 被覆盖，token 由 `dim-grok-auth-refresh.py` 每 15 分钟自动刷新），usage 记录
到 `grok-usage.jsonl`（source=`grok-cli`，provider=`xai-official`）。dim 本地补充排除
`grok-build-proxy` 通道（`dim.rs` 的 `GROK_BUILD_PROXY_PROVIDER`）防双计；原生
`grok-build` 通道（直连 x.ai）仍按 run 粒度摄入（历史保留）。

**Command Code 代理说明**：`token-stats-cc-proxy.service` 用 `--cc-proxy-only` 启动后端二进制，
监听 `127.0.0.1:${CC_PROXY_PORT:-8787}`，为 DimAgent 提供 OpenAI 兼容的
`POST /v1/chat/completions`（流式+非流式）与 `GET /v1/models`（`/models` 同路由）。
请求转换为 Command Code 私有 `/alpha/generate` 协议（同 pi-commandcode-provider 形状：
`x-command-code-version` / `x-cli-environment` / `x-project-slug` 头 + UUID `threadId`），
从 `finish` 事件的 `totalUsage` 提取用量追加到 `~/.token-stats/cc-proxy-usage.jsonl`。
认证读 `~/.commandcode/auth.json` 的 `apiKey`（或 `COMMANDCODE_API_KEY`）；模型列表从
`provider/v1/models` 拉（`COMMANDCODE_MODELS_URL` 可覆盖）。DimAgent 接入：
`dim provider add cc-proxy --api-key x --base-url http://127.0.0.1:8787 --adapter openai-compatible`。
注意：CC 服务端**不会**为代理请求写本地 session jsonl，用量只能靠代理自记；`/alpha/generate`
要求 `stream:true` + 完整 CLI 头 + UUID threadId，否则返回 400/403。

**WorkBuddy 代理说明**：`token-stats-workbuddy.service`（user 级 systemd）运行
`~/workbuddy-proxy/cli-proxy-api`（CLIProxyAPI v7.2.x，监听 `127.0.0.1:8317`）+ `workbuddy.so`
插件（[libukai/workbuddy-cliproxy](https://github.com/libukai/workbuddy-cliproxy)，本机 Go 1.26
编译，仓库在 `~/workbuddy-proxy/workbuddy-cliproxy`），把 Tencent CodeBuddy Web API
（`copilot.tencent.com/v2/chat/completions`）封装为 OpenAI 兼容接口供 DimAgent 使用
（`dim provider add workbuddy --api-key <key> --base-url http://127.0.0.1:8317/v1`）。
扫码登录凭据持久化于 `~/.cli-proxy-api/workbuddy-*.json`（0600，自动刷新）。
插件在每次请求完成时把归一化后的用量（含 `credit` 积分）追加到
`~/.token-stats/workbuddy-usage.jsonl`（`WORKBUDDY_USAGE_LOG_PATH` 可覆盖）——即
`dim-agent` 数据源；缓存语义按 Anthropic 约定归一（`inputTokens` 不含 cache），
缓存命中从 `prompt_tokens_details.cached_tokens`（腾讯 Web API 的 OpenAI 式字段）
提取。管理 API
密钥在 `~/workbuddy-proxy/config.yaml`（仅绑 127.0.0.1）。注意：hy3 系列免费
（credit=0），hy4-preview / glm-5.x / kimi-k* 等消耗积分，按
`codebuddy_cny_per_credit` 换算 CNY。
**模型目录**：插件给 dim 的模型列表来自编译内嵌 `models.yaml`（2026-08-30 验证版），
已改用外部 manifest `~/workbuddy-proxy/workbuddy-models.yaml`（config.yaml 的
`plugins.configs.workbuddy.model_manifest`，整体替换内嵌目录，CLIProxyAPI 配置重载时
重新读取）以追加平台新模型（如 `deepseek-v4.1-flash`，2026-09-11 上线）——CodeBuddy CLI
的模型列表跟平台走，插件清单不会自动跟上，新模型需手动加进 manifest 并重启
`token-stats-workbuddy.service`，然后在 dim 侧执行 `dim model refresh workbuddy`
（模型缓存在 `~/.dimcode/v2/dimcode.sqlite` 的 `providers.models`，`dim provider list`
显示的数量不会自动更新）。
**模型前缀**：同一 manifest 下新增 `plugins.configs.workbuddy.model_prefix: "wb"`
（插件已 fork 改造：`config.go` 读取 + `main.go` `wbModels()` 加前缀、
`stripModelPrefix()` 执行前剥回裸名），使 CPA 共享目录里 workbuddy 模型一律
`wb/<model>`，与 ollama 的 `ollama/<model>` 不重名。改前缀后（本机由裸名 → `wb/`）
必须重启 `token-stats-workbuddy.service` + `dim model refresh workbuddy`。
插件的 `logUsage` 会把前缀剥掉再写 JSONL，所以 token-stats 侧的模型名仍是裸名
（与 pricing / vendor_merge 的键一致）。

**Ollama Cloud 逐请求说明**：Ollama Cloud 也走**同一个** CPA 实例（8317）——配置里
新增 `openai-compatibility` 条目 `ollama-cloud`（`prefix: "ollama"`；`models:` 必须显式
列出，否则 `/v1/models` 为空，用 `scripts/sync-ollama-models.sh` 幂等同步）。另有
usage-only 插件 `ollama-usage.so`（源码 `~/workbuddy-proxy/ollama-usage-plugin/`，
Go 1.26 + `CGO_ENABLED=1`，`-buildmode=c-shared`）通过 CPA 的 `UsagePlugin` 回调拿到
**每次**上游调用的用量与 TTFT，归一化后 append 到 `~/.token-stats/ollama-usage.jsonl`
——即 `ollama-proxy` 数据源，使详细请求表能显示单次调用（TTFT/TPS），取代原先 dim
本地库「一次 run 一行」的聚合。

**来源 id vs 显示名（约定）**：`TokenRecord.source` 是**传输通道 id**，由写日志的一方
固定（`ollama-proxy` = CPA 的 `ollama-usage` 插件；`dim-agent` = workbuddy 插件；
`cc-proxy` = 内置 CC 代理），用于去重指纹、迁移谓词（`store.rs` / `app.rs` 的
`source='ollama-proxy'` 等）与增量解析；**不要**为了 UI 好看改它——改了会让历史记录与
新记录分属两个 source，指纹不同 → 双计。UI 文案只在
`frontend/src/lib/utils.ts` 的 `SOURCE_LABELS` / `SOURCE_COLORS` 里映射
（当前 `ollama-proxy → "Dim→Ollama"`、`dim-agent → "Dim→CB"`、`cc-proxy → "Dim→CC"`），
未登记的 source 会原样显示 id（如旧版曾显示的 "ollama-proxy"）→ 新增来源必须补这两个表。

**「保留 agent 名」目前做不到**：CPA 的 `UsageRecord.Source` 是 `resolveUsageSource()`
从上游凭据推导的（OAuth 账号邮箱、api-key 明文），**不是**下游客户端身份；
`api-keys:` 里的 `cb-local-key` 只进 `userApiKey` gin context，`usageAdapter.HandleUsage`
不会把它透给插件。所以插件拿不到「谁发起的」——同一实例上 workbuddy 与 ollama 的流量
只能靠 `Provider`（`openai-compatible-<name>`）区分，日志里的 `"apiKeyPrefix":"N/A"` 即
此原因。若将来要按调用方（DimAgent / 其他客户端）拆分用量，可行路径是给每个客户端
**分配独立 api-key**，厂商自行维护 `api-key → agent` 映射（token-stats 侧只读日志，
无法还原）。

**模型命名空间（重要）**：CPA 的 `/v1/models` 返回的是**所有通道的并集**，而 dim 的
每个 provider 都会把整个并集当成自己的目录 → 同一个模型 ID 会出现在多个 provider
下且无法区分谁是谁。因此：

- CPA 侧 `force-model-prefix: true`（config.yaml）——**必须开**，否则 workbuddy 插件
  的模型会既带前缀又以裸名各出现一次（插件模型走 host 的 `applyModelPrefixes`，
  与该开关相干）；
- `openai-compatibility` 的 ollama 条目 `prefix: "ollama"` → `ollama/<model>`；
- workbuddy 插件新增配置 `plugins.configs.workbuddy.model_prefix`（本机 `"wb"`，
  实现在 fork 的 `config.go` `configuredModelPrefix()` + `main.go` `wbModels()` 前缀
  加前缀、`stripModelPrefix()` 在执行前剥回裸名）→ `wb/<model>`。

结果：`/v1/models` 里 ollama 模型一律 `ollama/*`、workbuddy 一律 `wb/*`，**无裸名、无
重名**。两边的请求体仍以裸模型名发往上游（host 与插件各自负责剥前缀）。

**dim 侧只保留一个 provider**：`ollama-cloud-proxy` 与 `workbuddy` 指向同一个
CPA（`http://127.0.0.1:8317/v1`）且拉到同一份 37 个模型目录，属于重复配置——
2026-09-12 起只保留 `workbuddy`（`dim provider update workbuddy --name "CPA(ollama+wb)"`），
用 `ollama/*` 与 `wb/*` 前缀区分通道；`ollama-cloud-proxy` 与旧的直连
`custom-ollama-cloud-042036d3`（`https://ollama.com/v1`）均已 `dim provider remove`。
新增/切换模型后必须 `dim model refresh workbuddy`。

**踩坑：直连 ollama provider 会被「重新加回来」**（2026-09-12 复现）。上一条记录后
`custom-ollama-cloud-042036d3` 又被 `dim provider add` 重建了一次
（`createdAt` = 2026-09-11T23:13Z，`baseUrl` = `https://ollama.com/v1`），于是模型
选择器里多出 20 个**裸名**模型（`deepseek-v4.1-flash`、`glm-5.3-flash`…）——它们与
CPA 的 `ollama/*` 是同一批模型、同一个上游，只是绕开了 CPA（因此没有 `ollama-usage`
插件计量、没有 `ollama/*` 前缀）。它和 `workbuddy` 的 `ollama/*` 是**完全相同的
集合**（脚本比对：`set(direct) == set(cpa_ollama)` = True），属于纯重复。
排查手法：`dim provider list` 里出现多于 `workbuddy` 的 connected provider，或
`dim model list | grep -v /` 出现裸名。修复：`dim provider remove <id>`，然后
确认 `sqlite3 ~/.dimcode/v2/dimcode.sqlite "select providerId from providers where enabled=1"`
只剩预期条目。注意 `remove` **不删历史**——`usage_run_stats` 里该 provider 的
390 行会保留（token-stats 的 `dim` 源仍会读它们作为历史，见下方 cutoff 逻辑）。

**踩坑（已复现并修复）**：`dim model refresh` 会把 `metadata.enabledModelIds` 收敛为
「新目录里仍然存在的旧 ID」。若旧目录是 `oc/*`（前缀改名前的历史），refresh 后该字段
变成 `[]`；随后 `dim exec` 的 `resolveProviderDefaultModel`/`isProviderModelSelectable`
判定无任何可选模型，报 `Error: Failed to execute prompt`（`activeModelId` 落在不在
目录里的旧 ID 也会同样报错）。修复：删掉 `metadata.enabledModelIds`（`undefined` =
不限制，而非空数组 = 全禁），并把 `activeModelId` 指向目录内的新 ID（如 `wb/hy3`）。

**改前缀/删 provider 必须同步修历史会话**：`session_states` 为每个会话存了
`selectedProviderId + selectedModelId`。改名或 `dim provider remove` 之后，旧会话
指针失效，dim 解析不到凭据 → 桌面应用报「缺少凭据 / 凭据已失效」
（`PROVIDER_CREDENTIAL_MISSING: Credential missing for provider: <old-id>`），
**凭据本身没问题**（`dim provider test`、`dim exec` 直跑都正常）。用
`scripts/dim-repair-session-models.mjs`（幂等，先 `--dry-run`）把会话重新指向
合并后的 provider + 正确前缀的模型 ID；脚本同时处理已改名模型
（`deepseek-v4-flash-vision-exp` → `deepseek-v4.1-flash` 等）。2026-09-12 的
前缀改动一次性影响了 452 行里的 283 行。**注意**：只修 DB 不够——桌面应用进程内缓存
着启动时的 provider 目录，点「新建会话」会把旧选择原样写回（Electron LocalStorage 的
`dimcode:model-recent:v1` 也留着旧条目）。修完必须**重启 DimAgent 桌面应用**，并在模型
选择器里重选一次 `ollama/<model>` 或 `wb/<model>`，否则会反复复发。

DimAgent 侧：新会话直接选 `workbuddy` provider 下的 `ollama/<model>`（Ollama Cloud）
或 `wb/<model>`（CodeBuddy）；`dim model refresh workbuddy` 更新目录。
**两个坑**：① `pluginapi.Metadata` / `UsageRecord` / `UsageDetail` 都**没有 json tag**，
线格式是 PascalCase（只有 `Capabilities` 是 `usage_plugin` 小写）——写错会让注册被拒
（日志刷 `invalid metadata` 并反复重试）或用量静默全零；② CPA 上报的 `InputTokens`
**含**缓存，插件按 `input = InputTokens - cacheRead - cacheWrite` 做减法（`cacheRead` 取
`max64(CacheReadTokens, CachedTokens)`，CPA 两个字段都填）。插件目录**仅启动时扫描**，
新增/重建 `.so` 需重启 `token-stats-workbuddy.service`。切换时点由
`ollama-usage.jsonl` 首行时间确定（append-only）：`dim.rs` 丢弃该时点**及之后**的
ollama-cloud 按 run 行，之前的保留为历史；`app.rs` 在 cutoff 已知后做一次性 store
迁移删除（`store.purge_superseded_ollama_run_rows`），启动早于插件首条记录时会在后续
刷新重试。dim 本地补充同时排除 `ollama-cloud-proxy` 通道防双计（该 provider 已于
2026-09-12 移除；常量仍在 `dim.rs`，以防重新加入）。

### 配额数据源（`GET /api/quota`）

| 卡 | 来源 | 配置 |
|----|------|------|
| Kimi / Kimi EX | `https://auth.kimi.com` 刷新 token 后查 `/usages` | `KIMI_CREDENTIALS_PATH` / `KIMI_CREDENTIALS_PATH_EX`；EX 默认指向 `~/.kimi-code-user2/credentials/kimi-code.json`；`KIMI_AUTH_BASE_URL` 可覆盖 |
| OpenCode Go / OpenCode Go EX | HTTP 抓取 `https://opencode.ai/workspace/{id}/go` 的 `<div data-slot="usage">`（`reqwest`+`scraper`） | `OPENCODE_GO_WORKSPACE_ID(_EX)` + `OPENCODE_GO_AUTH_COOKIE(_EX)` |
| Xiaomi MiMo | MiMo token 计划 API | `XIAOMI_MIMO_SERVICE_TOKEN` + `XIAOMI_MIMO_USER_ID` |
| Command Code | `https://api.commandcode.ai`（`/alpha/billing/subscriptions`、`/alpha/billing/credits`、`/alpha/usage/summary`）；主账号从 `~/.commandcode/auth.json` 的 `apiKey`（Bearer），第二账号（EX）从 `auth*.json`（如 `auth_frank.json`）——与主账号 apiKey/userId 相同的重复 auth*.json 会被跳过（否则 EX 卡会显示主账号）；无 auth 文件时回退 `COMMANDCODE_SESSION_TOKEN` cookie（`/internal/*` 旧路由） | `COMMANDCODE_SESSION_TOKEN` 作为 `__Secure-commandcode_prod_.session_token` cookie（仅回退） |
| CodeBuddy 套餐 | `www.codebuddy.cn` billing meter API（`POST /billing/meter/get-user-resource-summary` 取各套餐包周期总量/剩余，`POST /billing/meter/get-user-resource` 取套餐名与周期；即 `/profile/plans-usage` 页同源接口）。**必需 `session` + `session_2` 两个 cookie**（单 `session` 返回 401）；边缘 WAF 拒绝过旧 Chrome UA（Chrome/126 被拦、152 可过）。cookie 从 Chrome 提取：`scripts/extract-codebuddy-cookies.sh`（约 30 天过期需重取） | `CODEBUDDY_SESSION_COOKIE` + `CODEBUDDY_SESSION_COOKIE_2`（仅 cookie 值） |
| Ollama Cloud | Ollama 云端 API | `OLLAMA_AUTH_COOKIE`（`__Secure-session=...`） |
| Meituan LongCat | 美团 API | `MEITUAN_AUTH_COOKIE`（`passport_token_key`） |
| Fenno / Fenno EX | `https://api.fenno.ai/api/v1/subscriptions/active` | `FENNO_AUTH_TOKEN` + `FENNO_REFRESH_TOKEN` 引导凭据管理器；轮换凭据持久化到 `FENNO_AUTH_STATE_PATH`（默认 `~/.config/token-stats/fenno-auth.json`）并自动刷新 |
| Grok | 基于 `grok-cli` 记录 + 订阅配额页面 | `grok_proxy.rs` 读取的用量记录；配额逻辑在 `quota/grok.rs` |
| Ainaiba 余额 | `api-xai.ainaibahub.com` | `YAI_API_KEY`（`/api/ainaiba-credit` 端点） |
| ZAI | `api.zairouter.com` 的 `/dashboard/info` + `/dashboard/live` + `/dashboard/status`（Bearer `ZAI_API_KEY`）——账户、到账卡、逐模型日/月用量、`suspended`。**计费**：充值 **1 元 = 1 美元额度**（订单实测 `amount`=10000 分 → `credit_amount`=100.0），但按平台内部价目表扣费，**不对外公布且不是官方价的统一倍数**；实测费率登记在 `pricing.toml` 的 `[[zai_model]]`（见"成本计算"）。`claude-haiku-4-5` 在该订阅下不可用（平台返回 long-context beta 未开通）。余额符号取决于账号 `factor`（1→`¥`，否则 `$`），本机 `factor=1` 但 1:1 兑换使两者数值相同 | `ZAI_API_KEY`（浏览器控制台/`~/.bash_env`）。注意与 `YAI_API_KEY` 是**两个不同账号**（充值独立、倍率独立） |
| DimAgent | **主路径**：本地 `dim usage --json`（CLI 自动发现，见 `quota/dimagent.rs`；OAuth 凭据在 `~/.dimcode/v2/auth.json`，CLI 自动刷新，无需任何环境变量）。**回退**：console API `dimagent.cn/api`（`/me/subscription` + `/me/credits` + `/me/feature-meters` + `/user/quota-estimate`） | `DIMAGENT_SESSION_COOKIE`（浏览器 `session` cookie 值）仅用于回退和近 30 天统计增强；`DIM_USAGE_BIN` 可覆盖 CLI 二进制 |
| ZCode | **主路径**：BigModel monitor API（逆向 ZCode 桌面应用 app.asar 得出，`quota/zcode.rs`）：`GET open.bigmodel.cn/api/monitor/usage/quota/limit`（套餐 level + 限额窗口；裸 apiKey 放 `authorization` 头，无 Bearer 前缀）+ `GET open.bigmodel.cn/api/biz/subscription/list`（套餐名/续订/到期）。apiKey 从 `~/.zcode/v2/config.json` 的 `provider["builtin:bigmodel-coding-plan"].options.apiKey` 读取（应用自动轮换）。**用量半区**：`source='zcode'` 内存记录聚合（今日/累计 调用、tokens、成本），其中 `provider='bigmodel-start'`（体验套餐）行被排除、单独进 `data.startPlan`（本地账本：grantTokens 总量默认 3 亿 / usedTokens / remainingTokens / calls；monitor API 拒收 start-plan key 返回 401，无远端半区）。**字段语义陷阱**：`usage`=窗口总量、`currentValue`=已用、`remaining`=剩余、`percentage`=已用百分比（currentValue/usage）——命名有误导，前端统一走 `zcodeWindowUsage()`。60s 缓存防 30s 轮询打满；远程失败但本地有记录时 `available:true` + `data.quota_error` 降级显示 | `ZCODE_BIGMODEL_USAGE_API_KEY` / `ZCODE_BIGMODEL_USAGE_QUOTA_URL`（与应用自身 env 名一致）、`ZCODE_CONFIG_PATH`（默认 `~/.zcode/v2/config.json`） |

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
| `src/main.rs` | CLI 入口（`--grok-proxy-only`、`--cc-proxy-only`、`-l/--log-level`） |
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
| `src/cc_proxy.rs` | loopback Command Code 代理（OpenAI ↔ CC 协议转换，供 DimAgent 使用） |
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
  **Yairouter `gpt-6-astra` 实际服务模型不符（2026-09-18 受控探针验证）**：向
  `api.yairouter.com/responses` 请求 `gpt-6-astra`（带不带 codex 的
  `x-codex-routing-hint` 头都一样），响应 `response.model` 均为 **`gpt-5.6-luna`**；
  sol/luna/terra 则请求什么服务什么。但**计费按请求模型 `gpt-6-astra` 的列表价**
  走（`/dashboard/live` 的 `ModelUsage.gpt-6-astra` 增量与
  `(input×$10 + output×$50)/1M × factor(7)` 分毫不差，luna 条目不动）——即
  花 astra 价买到的是 luna。因此 token-stats 按 `turn_context` 请求模型计费
  **与实际扣费一致，无需修改**；codex 本地数据（rollout / logs_2.sqlite）只记
  请求模型，**拿不到**实际服务模型，无法在解析器侧检测此调包。修复只能在
  codex 配置层（停用/绕开 astra）或向平台反馈。探针脚本思路：POST
  `/responses` + `stream:true`，比对 `response.created`/`response.completed`
  里的 `model` 字段与请求模型；计费口径用 `/dashboard/live` 的
  `daily_usage.ModelUsage.<model>.CreditUsed` 前后差值验证。
- **Command Code**：`cc:` 前缀模型为 Command Code 列表价（部分模型带 `peak_hours_utc`
  峰谷价，DeepSeek 2026-08-16 起实施）；实际成本 = 列表价 / `commandcode_divisor` → CNY。
  **2026-09-09 新增 `deepseek-v4.1-flash`**（CLI v1.53.0，v4-flash 同价，Go 计划页确认）：
  `cc:deepseek-v4.1-flash` 低谷 0.15/0.60/0.003，高峰（UTC 01–04 & 06–10，**仅周一–周五**）
  2× = 0.30/1.20/0.006。注意模型名以 `deepseek-v4.1-` 开头、**不**匹配
  `normalize_commandcode_model` 的 `starts_with("deepseek-v4-flash")` 分支（走 `cc:` fallback），
  漏配该键会让 cc-proxy/cmd 记录显示 **N/A** 并从费用合计中消失。周末高峰语义由
  `ModelPriceConfig.peak_weekdays_only`（+ `TimeSegment.peak_weekdays_only`）实现，已同时
  用于 cc 的 v4-pro / v4-flash / v4.1-flash；官方免费模型（`cc:laguna-s-2.1-free`）显式记 0，
  以便计入调用次数而不被当作「无价格」剔除。
- **CodeBuddy**：记录的 `cost` 保存原始 credits；实际成本 = credits × 每 credit 单价（直接人民币
  计价，不经过汇率）。单价由 `codebuddy_cny_per_credit`（历史基准：连续包月活动价 70 元 /
  4000 credits = 0.0175）与 `codebuddy_credit_segments`（套餐升级分段）共同决定：取最近一个
  已生效的 `effective_from`，早于所有分段的记录沿用基准价。**2026-09-14 13:00 CST** 起升级为
  140 元 / 9000 credits = 0.0155556 元/credit（比原价低约 11%）。`codebuddy` 与 `dim-agent`
  两个 source 共用该公式，均按记录时间选段（改价只影响新记录，历史波动不会被追溯改写）。
- **Kimi 订阅**：`kimi_api_models`（CNY/1M，cache write 免费）+ `kimi_subscription_multiplier`
  （默认 20，设置抽屉可调、持久化）：`成本 = (input×in + cache_read×cr + output×out) / 1M / 倍率`。
- **OpenCode**：原始 cost / `opencode_divisor`（6.0）；`opencode_model_segments` 可按模型+
  时间覆盖 divisor（如 deepseek-v4 2026-08-18 起 divisor=3）。
- **Dim（console API 源）**：不存储原始 cost，`display_cost()` 走"衍生源"分支——按
  pricing.toml **`[[dim_model]]` 平台积分价**（Lite 套餐 ¥70/11000 积分换算 CNY，如
  vision-exp input=0.653798 / output=3.922790 / cache_read=0.043587 元每 1M），
  vision-exp 高峰时段（CST 09:00–12:00 / 14:00–18:00 = UTC 01–04 / 06–10）按
  `peak_*_cny` 双倍（客服确认，2026-08-22~09-02 六天日账闭合验证）；v4-flash 无高峰
  双倍按接口价（0.871818 / 1.743636 / 0.017436）；deepseek-v4.1-flash-expires-on-0910
  （2026-09-08 上线的 v4.1 flash 临时命名）费率与 vision-exp 相同
  （0.653798 / 3.922790 / 0.043587，含高峰双倍）；**2026-09-10 平台把该模型改名为
  `deepseek-v4.1-flash` 并下调费率**，正式名按价目卡计费（0.890909 / 3.500000 /
  0.017182，高峰 UTC 01–04 / 06–10 双倍 = 1.781818 / 7.000000 / 0.034364；当日日账
  闭合验证 <0.01%）——两个名字都保留在 `[[dim_model]]` 中（旧名仅供历史记录，最后
  一条 2026-09-09T23:14Z），改名漏配会让该模型成本显示 0.00/N/A；seed-2.0-mini 按
  价目卡（0.174346 / 1.743458 / 0.034869，无高峰）；glm-5.3 全时段 7 折（基础价已含），
  夜间 20:00–08:00 CST（= UTC 12–24）再 5 折（peak 价）；glm-5.3-flash 无折扣按接口价
  （0.477273 / 1.590909 / 0.095455）；无价格模型的记录显示 N/A。
- **Ainaba**：`USD × ainaba_platform_rate(7.0) / ainaba_segments 分段 divisor`（平台固定汇率，
  不随市场波动）。
- **Qoder**（provider=`qoder`，`qoder-cli` / `qoder-desktop` 源）：当前套餐**免费**（模型目录
  `isFree: true` + `priceFactor: 0`），`pricing.toml` 的 `[special] qoder_free = true` 让
  `display_cost()` 直接返回 ¥0，而不是落到「无价目条目 → N/A」从而被聚合统计剔除。套餐转付费后
  关掉该开关并登记 `[[model]]` 价目即可回到通用路径。
- **ZAI**（provider=`zai`，claude-code 源）：**不公布价目表**，实收费率也**不是官方价的统一
  倍数**，因此 `pricing.toml` 直接登记**平台实收费率**（USD/1M，`[[zai_model]]`），
  `成本 = 实收费率 × zai_rate_cny_per_usd(1.0)`。充值 **1 元 = 1 美元额度**（订单实测
  `amount`=10000 分 → `credit_amount`=100.0），故该值恒为 1 元/美元。实测费率
  （2026-09-15，受控探针 + 当天日账交叉验证）：
  `claude-fable-5-1`/`mythos-5-1` 25/100/0.63/50（官方 10/50/0.25/12.5）；
  `claude-opus-5`/`4-8`/`4-7`/`4-6` 5/25/0.50/10（官方缓存写 6.25，实收走 1h 档）；
  `claude-sonnet-5`/`4-6`/`4-5` 3/15/0.30/6。已确认无长上下文分档（273K 输入同费率）。
  未登记模型回落官方 `[[model]]` 价（不会显示 N/A）。`claude-haiku-4-5` 在该订阅不可用。
- **订阅类折扣**：`freemodel_divisor`（=汇率/0.1）、`fenno_divisor`、`grok_divisor`；
  Ollama 用经验 per-token 价 + 模型倍率（`deepseek-v4-flash` / `deepseek-v4.1-flash` 均为 0.2，未登记模型回落 1.0）；讯飞订阅按次计
  （`xunfei_per_call` × 波谷系数 0.8，`peak_hours=[8,22]` + 节假日表）；
  Xiaomi MiMo / Meituan 按话单 per-token。
- **ZCode / BigModel GLM Coding Plan**（source=`zcode`、provider=`bigmodel`）：按**官方
  列表价**计费，不走模型价目表。官方积分公式（docs.bigmodel.cn 套餐概览）：
  `积分 = (输入×2.3 + 缓存命中×0.56 + 输出×8) / 10000`（GLM-5.3-Flash；GLM-5.3 系数
  6.9/1.7/24，配 `zcode_list_rates`）——积分系数即官方列表价（元/M tokens）。高峰
  （周一至周五 14:00–18:00 CST）按 1×，其余时段按 50% 抵扣（`zcode_off_peak_factor`）；
  夜间畅用活动期内（2026-09-03~09-20，每日 23:00–09:00 CST）ZCode 端消耗为 0
  （`zcode_night_free_*`），**仅限 GLM-5.3-Flash**（`zcode_night_free_models`
  白名单，2026-09-14 修正；GLM-5.3 夜间照常按高峰/波谷因子扣积分）；但活动自
  **2026-09-13 06:15 CST** 起才实际生效
  （此前用的旧版 ZCode 客户端不享受免扣，服务端照常扣积分）——该时刻之前落在
  窗口内的记录按正常波谷 0.5× 计费，由
  `zcode_night_free_effective_from`（RFC3339 含时区）控制，None = 窗口全程免费。
  实际成本 = 积分 × `zcode_cny_per_credit`。**实付分摊
  口径（2026-09-12）**：实付 ¥188.04 买 3 个月 ≈ 13 周 × 10000 积分 = 130,000
  积分 → 每积分 ¥0.00144646；一周用满 10000 积分 = ¥14.46，一个月用满 ≈ ¥62.68
  = 月均实付。注意：官方列表价×0.5312 口径高估约 3.7 倍（套餐额度价值远超实付），
  已废弃。实测验证：积分公式逐请求回算 3768.7 vs 配额
  API 的 3774（-0.14%），滚动 5h 窗口与分钟级增量同样吻合。公式要求
  `input_tokens` 为**非缓存输入**（zcode 解析器已减）。
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
| `QODER_SESSIONS_PATH` | `~/.qoder/logs/sessions` | qoder-cli（国际版 CLI）段日志目录覆盖（旧版 `QODER_PROJECTS_PATH` 指向的 `~/.qoder/projects` 已无 usage 数据，随解析器重写废弃） |
| `QODER_CN_SESSIONS_PATH` | `~/.qoder-cn/logs/sessions` | qoder-desktop（Qoder Desktop 内嵌 SDK）段日志目录覆盖 |
| `GROK_USAGE_LOG_PATH` | `~/.token-stats/grok-usage.jsonl` | Grok 用量日志覆盖 |
| `GROK_PROXY_PORT` | `3434` | loopback Grok 代理端口 |
| `CC_PROXY_PORT` | `8787` | loopback Command Code 代理端口（DimAgent 接入） |
| `CC_PROXY_USAGE_LOG_PATH` | `~/.token-stats/cc-proxy-usage.jsonl` | Command Code 代理用量日志覆盖 |
| `WORKBUDDY_USAGE_LOG_PATH` | `~/.token-stats/workbuddy-usage.jsonl` | WorkBuddy（CodeBuddy Web API）代理用量日志覆盖 |
| `OLLAMA_PROXY_USAGE_LOG_PATH` | `~/.token-stats/ollama-usage.jsonl` | CPA `ollama-usage` 插件（Ollama Cloud 逐请求）用量日志覆盖；插件与后端读取同一变量 |
| `COMMANDCODE_API_BASE` | `https://api.commandcode.ai` | Command Code 代理 API 基址 |
| `COMMANDCODE_MODELS_URL` | `https://api.commandcode.ai/provider/v1/models` | Command Code 代理模型列表 URL |
| `GROK_YAI_UPSTREAM_BASE_URL` / `GROK_UPSTREAM_BASE_URL` | `https://api.yairouter.com` | Grok YAI 上游（兼容旧名 `GROK_UPSTREAM_BASE_URL`） |
| `GROK_XAI_UPSTREAM_BASE_URL` | `https://api.x.ai` | Grok xAI 上游 |
| `GROK_XAI_NETWORK_PROXY` | 未设置 | xAI-only 网络代理（如 `http://127.0.0.1:7800`），**不得**用通用 `HTTP_PROXY`（会同时影响双上游） |
| `COMMANDCODE_PROJECTS_PATH` | `~/.commandcode/projects` | Command Code 会话目录覆盖 |
| `CODEBUDDY_PROJECTS_PATH` | `~/.codebuddy/projects` | CodeBuddy 会话目录覆盖 |
| `COMMANDCODE_SESSION_TOKEN` | 未设置 | Command Code 配额卡 cookie 值（仅当无 `~/.commandcode/auth.json` 时作为回退） |
| `CODEBUDDY_SESSION_COOKIE` / `CODEBUDDY_SESSION_COOKIE_2` | 未设置 | CodeBuddy 套餐配额卡的 `session` / `session_2` cookie 值（两者必需；`scripts/extract-codebuddy-cookies.sh` 可从 Chrome 自动提取并输出 export 行） |
| `ZCODE_DB_PATH` | `~/.zcode/cli/db/db.sqlite` | ZCode 库位置覆盖 |
| `ZCODE_CONFIG_PATH` | `~/.zcode/v2/config.json` | ZCode 桌面应用配置路径覆盖（ZCode 配额卡读其中编码套餐 apiKey） |
| `ZCODE_BIGMODEL_USAGE_API_KEY` | 未设置 | ZCode 配额卡 BigModel apiKey 覆盖（与 ZCode 应用自身 env 名一致；未设置时读 config.json） |
| `ZCODE_BIGMODEL_USAGE_QUOTA_URL` | `https://open.bigmodel.cn/api/monitor/usage/quota/limit` | ZCode 配额卡 quota 端点覆盖（与 ZCode 应用自身 env 名一致） |
| `ZCODE_START_PLAN_TOTAL_TOKENS` | `300000000` | ZCode 体验套餐（Weekend Build 赠量）总额度 token 数覆盖；配额卡 `start_plan` 统计的分母 |
| `DSH_SESSIONS_PATH` | `~/.dsh/sessions` | DSH 会话目录覆盖 |
| `DIM_DB_PATH` | 已废弃 | 旧版 Dim 本地 SQLite 库路径，console API 源不再使用 |
| `DIM_LOCAL_DB_PATH` | `~/.dimcode/v2/dimcode.sqlite` | dim 源本地补充库路径（第三方通道 per-run 记录，如 ollama cloud） |
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
   打开；**切勿写入**这些源库。dim 源对 `~/.dimcode/v2/dimcode.sqlite` 的补充读取
   同样只读（该库本身只被 dim CLI 自身写入）。
4. **Dim console API 轮询** — `sources/dim.rs` 每次刷新循环（默认 30s）先拉第 1 页
   （`p=1&page_size=100&type=2`，`page` 参数会被服务端忽略），有新记录才继续翻页直
   到已见过的 id；页内 id 倒序。cookie 失效（401）时优雅降级为空并保留历史。
   启动时若完整回填成功，会对 store 做**一次性迁移**：删除旧的按 run 聚合的
   `source='dim'` 行（指纹不在 API 记录集合中的），避免与逐请求记录双重计数。
   每次刷新同时读本地 `usage_run_stats` 的第三方通道行（排除所有已有逐请求计量的
   通道：`dimcode-api-oauth`、`workbuddy`、`grok-build-proxy`、`ollama-cloud-proxy`、
   `cc-proxy`——漏排的通道会和对应的逐请求源双计，`cc-proxy` 就曾因此以 provider
   `command-code`（displayName "Command Code" 的 slug）重复出现，2026-09-12 修复并
   一次性迁移清除 `source='dim' AND original_provider='cc-proxy'` 行），
   与 API 记录指纹不同不会双计；`original_provider`
   保留原始 providerId 供 `display_cost()` 区分计费公式（如 ollama-cloud 订阅价）。
   `grok-build` 通道映射为 `xai-official`（与 grok-cli 的 xAI 官方用量合并计费）：
   成本按官方 USD 列表价 × 汇率 ÷ `grok_divisor`（SuperGrok 订阅），忽略存储的
   catalog 价；启动时对 store 做一次性迁移删除旧的 `provider='grok-build'` 行（幂等）。
5. **Grok 记录不出现在请求明细** — 聚合包含 `grok-cli`（以及 dim 的 `xai-official` 通道），但 `paginate_requests` 明确排除
   该 source；detail 表永远不会显示 grok 单条记录。
6. **Kimi 成本是估算** — Kimi CLI/Code 不报原生 cost；按
   `kimi_api_models` API 原价 ÷ `kimi_subscription_multiplier` 估算。
7. **零 token 记录** — 默认从聚合与明细中排除（429 等）；`show_zero_tokens=true` 仅影响
   明细。`exclude_zero_tokens` 是 `FilterCriteria` 的统一开关。**例外**：
   `aggregator::counts_in_stats()` 在 `TokenRecord::counts_as_call_without_tokens()`
   为真时保留零 token 记录——目前只有 `qoder-desktop`（服务端不回传 usage，token 恒 0
   但调用真实存在），统计/筛选/RPM 与「请求明细」四处都走这个判断（`paginate_requests`
   复用 `filter_records`），所以 desktop 的逐次调用默认可见，无需勾选显示零 token；
   其余零 token 记录（429 等）仍按 `show_zero_tokens` 控制。
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
13. **DimAgent 的 `grok-build` provider 不会自刷新 x.ai OAuth** — 该 provider
    （`driverKind=xai-grok-build`，上游 `https://api.x.ai/v1`；其用量进 dim 源的
    `xai-official` 通道）的凭据存在 `~/.dimcode/v2/auth.json` 的 `xaiGrokBuild`
    条目，access token 寿命 6h。**dim 在过期时不会刷新它**：`dim auth refresh`
    只动 `nextApiOauth`（DimAgent 自身账号），`dim auth status` 也只看那个账号，
    所以状态始终显示 "Authenticated"。症状是所有 grok-build 请求 403
    `unauthenticated:bad-credentials` → dim 报
    `登录状态已失效或凭据无效（PROVIDER_AUTHENTICATION_ERROR）`。
    排错要点：看 `auth.json` 里 `xaiGrokBuild.expires`（或 access token 的 JWT
    `exp`）是否早已过期；过期就用 `xaiGrokBuild.refresh` 向
    `https://auth.x.ai/oauth2/token` 换新的（refresh token 会轮换，新值必须写回）。
    已自动化：`~/.local/bin/dim-grok-auth-refresh.py`（剩余寿命 <30min 才刷新；
    flock 防并发、原子写回、保留 0600 权限）+ user 级
    `dim-grok-auth-refresh.service` / `.timer`（每 15min，`Persistent=true`）。
    **坑**：user 级 unit **不继承 shell 的代理变量**，而 `auth.x.ai` 直连必然超时，
    所以 service 里显式设了 `https_proxy=http://127.0.0.1:7800`（haproxy 转发到
    sing-box/xray）与 `TimeoutStartSec=60`——否则 unit 会静默挂死不退出。
    若日志出现 "refresh token is rejected too"，说明 refresh token 也失效，只能
    `dim provider disconnect grok-build` 后重新登录。
14. **Ollama Cloud 代理插件的两个静默失败模式** — `pluginapi.Metadata` /
    `UsageRecord` / `UsageDetail` 无 json tag → 线格式 PascalCase（`Capabilities`
    例外，是 `usage_plugin`）。写错 key 时注册会被拒且 host **反复重试刷屏**
    （`invalid metadata or no capabilities`），或 `HandleUsage` 照常被调用但字段全零
    而**静默丢数据**。另外 `pluginhost` 只在启动时扫描插件目录：重建 `.so` 后必须
    重启 `token-stats-workbuddy.service` 才生效。插件内 panic 会被 host fuse，
    JSONL 写入是 best-effort（失败只丢记录，不阻塞请求），因此验证时要拿**同一次
    响应**的 `usage.cached_tokens` 与 JSONL 行比对，不能只看行数。
15. **Ollama Cloud 切换的双计防护** — `ollama-usage.jsonl` 首行时间即 cutoff。
    `sources/dim.rs` 在读时丢弃 cutoff 及之后的 `custom-ollama-cloud-042036d3`
    按 run 行（`ollama_run_record_superseded`），`app.rs` 另做一次性 store 迁移；
    cutoff 未知（插件尚未产出）时**不截断**，避免切换期丢历史。迁移只在 cutoff
    已知后 latch，所以先启动仪表盘、后产生首条代理记录的场景会在后续刷新补上。
16. **CodeBuddy 卡「消失」其实是 cookie 过期（401）** — 卡片在
    `!available && error 含 "not set"` 时才整卡隐藏（未配置凭据）；cookie 过期时
    后端返回 `HTTP 401 Unauthorized: (HTML response)`（边缘 WAF 挑战），此时**保留**
    卡片并显示错误文案（2026-09-12 修复前是一起隐藏，看起来像订阅没了）。排查：
    `curl -s localhost:3001/api/quota | jq .codebuddy`。修复：跑
    `./scripts/refresh-codebuddy-cookies.sh`（= 提取 Chrome cookie → 改写
    `~/.config/token-stats/deploy-env.sh` → 注入各 `token-stats@<port>` drop-in →
    重启实例；`--dry-run` 只提取不改）。注意 `session`/`session_2` 是 Flask 签名会话，
    值里含 `|`，写入 systemd `Environment=` 时 `|` 是合法的（不能带引号嵌套），
    但 `%` 必须转义为 `%%`（deploy.sh 的 `inject_env_dropin` 已处理）。
17. **部署不能直接跑 `./deploy.sh`：备用端口残留实例 + 凭据环境**（2026-09-14 发现）—
    deploy.sh 对**已 active** 的 `systemctl start token-stats@<备端口>` 是 no-op，
    健康检查照样通过 → nginx 切过去后实际仍在跑**旧二进制**（新代码/新 pricing.toml
    静默不生效）。另外 deploy.sh 依赖**当前 shell** 里的凭据环境变量：变量缺失时它会先
    `clear_env_dropins` 再注入空值，新实例直接丢光所有凭据（配额卡、dim 源全挂）。
    正确入口：`./scripts/deploy-dashboard.sh` —— 自动读 nginx upstream 找出备用端口
    并停掉残留实例、`source ~/.config/token-stats/deploy-env.sh` 后再 exec deploy.sh。
    注意仍需 sudo（写 `/etc/systemd/system`、`/var/www`、nginx 配置并 reload）。
18. **CodeBuddy 积分单价按套餐分段** — `codebuddy_cny_per_credit` 是**历史基准**
    （70 元 / 4000 credits = 0.0175），套餐升级走 `codebuddy_credit_segments`
    （2026-09-14 13:00 CST 起 140 元 / 9000 credits ≈ 0.0155556，低约 11%）。
    改基准值会**追溯改写全部历史**（含 9/1 之前），所以换套餐务必加分段而不是改基准；
    `source` 为 `codebuddy` 与 `dim-agent` 的记录共用该公式，都按 `record.time` 选段。
19. **ZCode 的 `commandcode` 通道 = 内置 cc-proxy，会逐请求双计**（2026-09-19 修复）—
    ZCode 用 `127.0.0.1:8787` 当自定义 provider，因此同一次调用既有代理写的
    `cc-proxy` 记录、又有 ZCode `model_usage` 行（时间只差 ~30ms、模型名带 `deepseek/`
    前缀），请求明细里成对出现，tokens/成本/调用次数全部翻倍，ZCode 配额卡的
    今日/累计用量也被带偏（它聚合 `source='zcode'`）。修复：`sources/zcode.rs` 用
    `PROXY_METERED_PROVIDERS` 在解析期丢弃该通道（代理日志为空时**不**丢，此时
    zcode 行是唯一点量）+ 启动迁移 `store.purge_zcode_commandcode()` 清历史行。
    排查手法：`select time,source,model,input_tokens,output_tokens,cache_read_tokens
    from token_records where provider='commandcode' order by time desc` 看是否成对。
    **注意**：同一实例上 ZCode 还把 CPA（`wb/`、`ollama/` 前缀模型）与 Grok 通道
    当 provider 用，这些行没有计费元数据、回落成 `opencode-go`，目前各只有 1 行
    （2026-09-15 试验期），若将来流量变大需按同样办法排除（与 `dim-agent` /
    `ollama-proxy` / `grok-cli` 源双计）。
