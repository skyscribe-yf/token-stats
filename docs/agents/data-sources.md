# 数据源详解

> 从 [`AGENTS.md`](../../AGENTS.md) 拆出。**精简清单**（source id / 位置 / provider / 计费分支）
> 留在 AGENTS.md；本文件保存每个数据源的完整行为说明、代理架构与踩坑史。
> 改某个源的解析器前，先读它在本文里的小节；跨源规范化见 AGENTS.md 的「数据模型」。

## 完整清单

| # | 数据源（`source` 值） | 位置 | 格式/说明 |
|---|----------------------|------|-----------|
| 1 | `pi` | `~/.pi/token-logs/usage.jsonl` | JSONL；另扫描 Taskplane runtime `events-exit.json` / `exit-summary.json`（可用 `TASKPLANE_PROJECTS_DIR` 覆盖项目根） |
| 2 | `codex` | `~/.codex/sessions/*/rollout-*.jsonl` | JSONL，直接来自 Codex CLI |
| 3 | `claude-code` | `~/.claude/projects/*/*.jsonl` | JSONL，直接来自 Claude Code CLI |
| 4 | `opencode` | `~/.local/share/opencode/opencode.db` | SQLite，直接来自 OpenCode CLI。**1.x 与 2.x 双形态**（2026-09-26 起本机为 v2.0.18）：逐条消息 2.x 存在 **`session_message`**（配 `session_v2`），1.x 存在 `message`（配 `session`）；2.x 迁移会把 1.x 历史复制进新表，旧表**保留但不再写入**（本机停在 `2026-09-26T12:57:31Z`，194 行）。读法：`session_message WHERE type='assistant'` 为主 + `message WHERE id NOT IN (SELECT id FROM session_message)` 取残行（本机 10 行＝升级前最后一个 session）；整表都读会重复 184 行。**JSON 形状差异**：role 在 2.x 是行的 `type` 列（不在 JSON 内）、模型在 2.x 是 `data.model.{providerID,id,variant}`（1.x 是顶层 `modelID`/`providerID`）、2.x **没有 `tokens.total`**（= input+output+reasoning+cache.read+cache.write，1.x 实测恒等于该和）、2.x **不再记 cost**（原生行恒 0）。`providerID='opencode'` 归一成 `opencode-go`。**reasoning 是独立且按输出价计费的 token 类**，不是 output 的子集：1.x 的 `cost` 实测满足 `input×$0.14 + (output+reasoning)×$0.28 + cache_read×$0.0028` 每 1M（deepseek-v4-flash，逐行精确闭合），且有 46 行 `output < reasoning`；解析器把 reasoning 并入 `output_tokens`，早期少算的行由启动迁移 `migrate_opencode_reasoning_output` 在 `load_all()` **之前**删旧指纹行、全量重解析回灌（**带闸门**：仅当 `opencode_readable_row_count()` ≥ 已持久化行数才删，源库被清理时保留历史并在下次启动重试——与陷阱 11 的「store 只增不删」一致，不同于其它无条件 purge） |
| 5 | `kimi-cli` | `~/.kimi/sessions/*/wire.jsonl` | JSONL（`KIMI_SESSIONS_PATH` 可覆盖目录） |
| 6 | `kimi-code` | `~/.kimi-code*/sessions/*/*/agents/*/wire.jsonl` | JSONL（`KIMI_CODE_HOME` 可覆盖根目录） |
| 7 | `qoder-cli` | `~/.qoder/logs/sessions/<project-slug>/<session-id>/segments/*.jsonl` | JSONL，国际版 `qoder`/`qodercli` CLI（v1.0.14）；只取 `type=model.response.completed` 事件并按 `request_id` 去重；OpenAI 式 `input_tokens` **含**缓存命中，解析时减去归一为 Anthropic 语义；provider 取日志的 `data.provider`，缺失时回落模型别名表（`qfmodel`/`qmodel_latest`/`efficient`/`auto`/`qmodel_38max`→`qoder`）（`QODER_SESSIONS_PATH` 可覆盖）。旧实现读 `~/.qoder/projects/*/*.jsonl`，新版 CLI 已不往那里写 `usage` → 恒 0 条 |
| 8 | `qoder-desktop` | `~/.qoder-cn/logs/sessions/<project-slug>/<session-id>/segments/*.jsonl` | 同一解析器（`sources/qoder.rs`），数据由 **Qoder Desktop**（`/opt/Qoder CN`，Electron，内嵌 agent SDK 1.1.49 写 `~/.qoder-cn`）产出。**token 恒为 0**：CN 网关对免费 Qwen3.8-Flash 路由不向客户端回传 usage 块（2026-09-18 实测 686/686 条 `model.response.completed` 全零、缓存字段同样为 0，应用自己的上下文快照也写着 `tokenCountsAvailable: false`；客户端没有 `include_usage`/feature gate 可打开），所以这个源**只有调用次数有意义**。为此新增 `TokenRecord::counts_as_call_without_tokens()`（仅 `qoder-desktop`），让统计/筛选/RPM 三处跳过 `is_zero_token` 过滤，否则全部记录会被当作失败请求丢弃（`QODER_CN_SESSIONS_PATH` 可覆盖） |
| 9 | `grok-cli` | `~/.token-stats/grok-usage.jsonl` | JSONL，由内置 loopback Grok 代理写入（`GROK_USAGE_LOG_PATH` 可覆盖） |
| 10 | `commandcode` | `~/.commandcode/projects/<slug>/<session-id>.jsonl` | JSONL；`type=message` 行含 `usage`；跳过侧车 `*.checkpoints.jsonl`（`COMMANDCODE_PROJECTS_PATH` 可覆盖） |
| 11 | `zcode` | `~/.zcode/cli/db/db.sqlite` | SQLite `model_usage` 表（`ZCODE_DB_PATH` 可覆盖）。**provider 映射**：`provider_metadata_json` 的计费名（`OpenCodeGo`→`opencode-go`、`Tokenrouter`→`tokenrouter`）；适配器名键（`anthropic`/`openai`——bigmodel 编码套餐走 Anthropic 协议）被跳过，无计费元数据的 `builtin:bigmodel-start-plan`（Weekend Build 体验套餐/3 亿 token 赠量）标 `bigmodel-start`（2026-09-13 起，成本恒 0——不扣正式套餐积分；启动迁移 `purge_zcode_start_plan_bigmodel` 在 `load_all()` **之前**删 cutoff `2026-09-13T01:30Z` 起的旧 `bigmodel` 行、由全量重解析按新指纹回灌防双计），其余无计费元数据的 `bigmodel*` 通道一律标 `bigmodel`，再其余回落 `opencode-go`。**通道 ID 带命名空间前缀**（`builtin:` 客户端内置套餐、`account:` 登录账号绑定的套餐，如 `account:bigmodel-individual-coding-plan`＝个人编码套餐），映射时**先剥掉 `:` 前的命名空间**再匹配（`*start-plan` 结尾 → `bigmodel-start`，`bigmodel` 开头 → `bigmodel`）——曾因只认 `builtin:` 前缀把账号绑定套餐的 GLM 流量全标成 `opencode-go`（vendor 图表错位 + 走 OpenCode Go 计费分支而非 BigModel 积分公式），启动迁移 `purge_zcode_account_plan_opencode_go` 在 `load_all()` 之前删 cutoff `2026-09-19T14:00Z` 起 `source='zcode' AND provider='opencode-go' AND model LIKE 'GLM%'` 的错标行、由全量重解析按新指纹回灌。曾因 `{"anthropic":...}` 元数据把整个 provider 误标成 `anthropic`，启动迁移 `purge_zcode_anthropic` 一次性清理并按新指纹重灌。**代理计量通道排除**：`provider_metadata_json` 的键其实是 ZCode 通道显示名，名为 `commandcode` 的那个通道就是内置 cc-proxy（ZCode → `127.0.0.1:8787`），代理已逐请求写 `cc-proxy-usage.jsonl`，故该通道 `model_usage` 行整批丢弃（`PROXY_METERED_PROVIDERS`，且**仅当代理日志非空**——没有代理时 zcode 行是唯一点量）；启动迁移 `purge_zcode_commandcode` 删除已持久化的 `source='zcode' AND provider='commandcode'` 行。坑：在 ZCode 里给该通道改名会换掉元数据键 → 排除失效、重新双计 |
| 12 | `dsh` | `~/.dsh/sessions/*/session-*/session.jsonl.zstd` | zstd 压缩 JSONL，DeepSeek Harness；usage chunk 与 `finish` replayState 配对取 provider/model（`DSH_SESSIONS_PATH` 可覆盖） |
| 13 | `dim` | DimAgent 控制台 API `https://dimagent.cn/api/log/self` + 本地 SQLite 补充 | **HTTP 轮询**（每刷新周期一次，默认 30s）：逐请求明细（time/model/prompt/completion/cache/ttft/tps），即控制台 Activity 页数据；`p` 分页 + `page_size` 上限 100 + `type=2`。**本地补充**：只读 `~/.dimcode/v2/dimcode.sqlite` 的 `usage_run_stats` 中第三方通道记录（如 `custom-ollama-cloud-042036d3` → `ollama-cloud`，vendor merge 并入 `ollama` 组；`grok-build` → `xai-official`，与 grok-cli 的 xAI 官方用量合并计费），并排除所有已有逐请求计量的通道避免双计：`dimcode-api-oauth`（与 API 源双计）、`workbuddy`（与 `dim-agent` 源双计）、`grok-build-proxy`（与 grok 代理日志双计）、`ollama-cloud-proxy`（与 `ollama-proxy` 源双计）、`cc-proxy`（与 `cc-proxy` 源双计——displayName "Command Code" 曾被 slug 成 provider `command-code` 单独出现在 vendor 图表中，2026-09-12 修复 + store 一次性迁移清除）**外加一条按地址的通用规则**：凡 `providers.baseUrl` 指向 CPA 回环（`CPA_LOOPBACK_ADDRS`，默认 `127.0.0.1:8317`/`localhost:8317`/`[::1]:8317`，只比对 authority）的通道一律丢弃（`dim.rs::cpa_metered_providers`）——CPA 上每个上游都应有配套的 token-stats 用量插件逐请求计量，因此不必再为每个新通道改名字白名单（漏配就是双计）。当前只命中 `workbuddy`（已在名字白名单里），作用是防住未来 Dim 选 `step/step-5-preview` 等 CPA 通道时的双计（`DIM_LOCAL_DB_PATH` 可覆盖库路径；`DIM_DB_PATH` 已废弃）。旧 per-run 记录在首次成功同步后被一次性迁移清除。**冷启动 watermark**：store `sync_watermarks` 表的 `dim_console_last_id` 记录已见最大 id，`finish_sync` 在每次成功同步后持久化（2026-09-13 接线，此前该写入从未发生→每次冷启动全量翻页 ~279 页 ≈90s）；冷启动从 watermark 续读只拉增量。**注意**：legacy per-run 行的一次性 purge（`purge_dim_legacy`）门槛是 `full_backfill_done`（本进程做过**从零**全量回填）而非 `last_sync_completed`——从 watermark 续读得到的只是增量指纹集合，若当全量历史用会把全部历史 dim 行误删 |
| 14 | `ccswitch` | `~/.cc-switch/cc-switch.db` | 仅当设置了 `USE_CC_SWITCH` 环境变量才加载（`CCSWITCH_DB_PATH` 可覆盖） |
| 15 | `codebuddy` | `~/.codebuddy/projects/**/*.jsonl` | JSONL；事件的 `providerData.rawUsage` 含 credits 与 token 用量（`CODEBUDDY_PROJECTS_PATH` 可覆盖） |
| 16 | `cc-proxy` | `~/.token-stats/cc-proxy-usage.jsonl` | JSONL，由内置 loopback Command Code 代理写入（`CC_PROXY_USAGE_LOG_PATH` 可覆盖）；`provider=commandcode`（成本走 `cc:` 价格 ÷ `commandcode_divisor`），`model` 已剥 vendor 前缀。**该代理同时服务 DimAgent 与 ZCode 的 `commandcode` 通道**，两个客户端的逐请求用量都只在这里计量（ZCode 侧的 `model_usage` 重复行已被 zcode 源排除） |
| 17 | `dim-agent` | `~/.token-stats/workbuddy-usage.jsonl` | JSONL，由 workbuddy CLIProxyAPI 插件（`~/workbuddy-proxy`，systemd 服务 `token-stats-workbuddy.service`）写入——DimAgent 经 Tencent CodeBuddy Web API 的请求；`provider=codebuddy`（成本走 `codebuddy_cny_per_credit` 积分换算，与原生 codebuddy 源同一计费公式），`WORKBUDDY_USAGE_LOG_PATH` 可覆盖 |
| 18 | `ollama-proxy` | `~/.token-stats/ollama-usage.jsonl` | JSONL，由 `ollama-usage` CLIProxyAPI 插件（`~/workbuddy-proxy/ollama-usage-plugin`，与 workbuddy 同实例）写入——DimAgent 经 CPA `ollama-cloud` 上游（`ollama/` 前缀）的**逐请求**用量，含 TTFT/TPS；`provider=ollama-cloud`（vendor merge 并入 `ollama`，成本走经验费率），`OLLAMA_PROXY_USAGE_LOG_PATH` 可覆盖 |
| 19 | `stepfun-proxy` | `~/.token-stats/stepfun-usage.jsonl` | JSONL，由 `stepfun-usage` CLIProxyAPI 插件（`~/workbuddy-proxy/stepfun-usage-plugin`，与 workbuddy / ollama 同一个 CPA 实例 :8317）写入——DimAgent 经 CPA `stepfun` 上游（`step/` 前缀，模型 `step-5-preview`）的**逐请求**用量，含 TTFT/TPS；`provider=stepfun`，成本走 StepFun 列表价 ÷ `stepfun_plan_divisor`（见 [`pricing.md`](pricing.md)），`STEPFUN_PROXY_USAGE_LOG_PATH` 可覆盖 |
| 20 | `glm-acp` | `~/.token-stats/glm-acp-usage.jsonl` | JSONL，由内置 loopback GLM 代理写入（`GLM_ACP_USAGE_LOG_PATH` 可覆盖）——Paseo 的 `glm-acp-agent` 经 `ACP_GLM_BASE_URL` 指入代理（详见「GLM 代理」小节）；`provider=bigmodel`，与 zcode 同一积分公式但永远 100% 扣积分（无 zcode 专属优惠），模型名解析时归一为官方大小写（`GLM-5.3-Flash`），含 TTFT |

## 代理与插件说明

### Grok 代理

**Grok 代理说明**：`token-stats-grok-proxy.service` 用 `--grok-proxy-only` 启动后端二进制，
监听 `127.0.0.1:${GROK_PROXY_PORT:-3434}`，为 Grok CLI 提供 `/v1/responses` 转发
（YAI Router 与官方 xAI 双上游，别名 `grok-4.5-yai` / `grok-4.5-xai` / `grok-4.6-*` / `grok-4.7-*` 分别重写为对应裸模型名），
从响应中提取 usage 追加到 `~/.token-stats/grok-usage.jsonl`。代理透传上游状态/响应体，
不记录 prompt、完成文本、请求头与凭据。
**DimAgent grok-build 通道也走此代理**：dim 的 `grok-build` provider 因 `xai-grok-build`
driver 硬校验 OAuth 凭据只能发往 `https://*.x.ai`（`PROVIDER_TRANSPORT_CONFIG_ERROR`），
不能直接改 baseUrl 指向代理。改为自定义 provider `grok-build-proxy`
（`dim provider add grok-build-proxy --api-key placeholder --base-url http://127.0.0.1:3434/v1
--adapter openai-responses --model grok-4.7`），其 `openai-responses` driver 发
`{baseUrl}/responses` 命中代理；代理对裸模型名（`grok-4.5` / `grok-4.6` / `grok-4.7`）路由到官方
xAI 上游，并从 `~/.dimcode/v2/auth.json` 的 `xaiGrokBuild.access` 注入真实 OAuth token
（占位 key 被覆盖，token 由 `dim-grok-auth-refresh.py` 每 15 分钟自动刷新），usage 记录
到 `grok-usage.jsonl`（source=`grok-cli`，provider=`xai-official`）。dim 本地补充排除
`grok-build-proxy` 通道（`dim.rs` 的 `GROK_BUILD_PROXY_PROVIDER`）防双计；原生
`grok-build` 通道（直连 x.ai）仍按 run 粒度摄入（历史保留）。

### Command Code 代理

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

### GLM 代理（Paseo glm-acp-agent）

**GLM 代理说明**：`token-stats-glm-proxy.service` 用 `--glm-proxy-only` 启动后端二进制，
监听 `127.0.0.1:${GLM_PROXY_PORT:-3435}`，为 Paseo 的 `glm-acp-agent`（npm 包，ACP agent）
提供纯透传转发到 `${GLM_PROXY_UPSTREAM_BASE_URL:-https://api.z.ai}`（路径原样透传，
Authorization 由调用方自带、代理不注入不改写），并从响应流里抄录 usage 追加到
`~/.token-stats/glm-acp-usage.jsonl`——即 `glm-acp` 数据源。代理透传上游状态/响应体，
不记录 prompt、完成文本与凭据。

**为什么必须由代理计量**：`glm-acp-agent` 直连 Z.AI OpenAI 兼容编码端点
（`api.z.ai/api/coding/paas/v4`，GLM Coding Plan PaaS），会话文件
（`~/.local/state/glm-acp-agent/sessions/*.json`）只存消息不存 usage——Paseo 驱动的
GLM 流量原本两个计量点都没有。该 agent 每次请求都带
`stream_options: { include_usage: true }` 且支持 `ACP_GLM_BASE_URL` 环境变量，因此
Paseo 侧 provider env 里把 base URL 指到代理即可，**npm 包零改动**；OpenAI 语义的
`prompt_tokens`（含 cached）在代理侧减去 `cached_tokens` 归一为 Anthropic 约定，
`reasoning_tokens` 是 completion 子集不另计。

**provider 标签与成本口径**：记录 `provider=bigmodel`（GLM 编码套餐厂商，与 zcode 的
GLM 流量在 vendor 图同组），成本与 zcode **同一套官方积分公式、积分分摊单价与时间
因子**（高峰 1.0× / 非高峰 0.5×，实测 ACP 通道同样享受）；唯一差别是夜间畅用窗口内
**不归零**，按 0.25× 扣（zcode 通道专属的是"归零"；白名单/日期边界与 zcode 共用）。
见 `compute_glm_acp_credit_cost` 与 docs/agents/pricing.md。这把 key 若与
bigmodel.cn 编码套餐同账户可再对齐积分口径；若是 z.ai 国际版的窗口配额套餐
（5h/weekly prompt window、超量限流不扣费），更接近 `bigmodel-start` 的「套餐内
边际成本 0」口径，待确认后调整。

**模型名归一化**：代理按线上原样抄录模型名（`glm-5.3-flash`），`GlmAcpSource::load`
解析时把它改写成 BigModel 官方大小写（`glm-前缀→GLM`、`flash` 后缀→`Flash`，如
`GLM-5.3-Flash`），与 zcode 源的模型名同组显示；非 GLM 名原样透传。改大小写会换
fingerprint，因此 `TokenStore::purge_glm_acp_wire_casing` 在启动时一次性删除
`source='glm-acp'` 下小写 `glm-*` 旧行（GLOB 区分大小写；日志是 append-only 且含全部
历史，删后由全量重解析原样重建；日志文件不存在时不动）。

**接入步骤**（新机器）：安装 `nginx/token-stats-glm-proxy.service` 并启用；Paseo
`~/.paseo/config.json` 的 `glm-acp-agent` provider 加
`"env": {"ACP_GLM_BASE_URL": "http://127.0.0.1:3435/api/coding/paas/v4"}`；重启 Paseo
daemon 使 provider env 生效。代理挂掉时 agent 直连上游、流量不计量但功能不受损
（base URL 指向本地端口会连接失败——此时代理是硬依赖，恢复服务即可）。

### WorkBuddy 代理

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

### Ollama Cloud 逐请求

**Ollama Cloud 逐请求说明**：Ollama Cloud 也走**同一个** CPA 实例（8317）——配置里
新增 `openai-compatibility` 条目 `ollama-cloud`（`prefix: "ollama"`；`models:` 必须显式
列出，否则 `/v1/models` 为空，用 `scripts/sync-ollama-models.sh` 幂等同步）。另有
usage-only 插件 `ollama-usage.so`（源码 `~/workbuddy-proxy/ollama-usage-plugin/`，
Go 1.26 + `CGO_ENABLED=1`，`-buildmode=c-shared`）通过 CPA 的 `UsagePlugin` 回调拿到
**每次**上游调用的用量与 TTFT，归一化后 append 到 `~/.token-stats/ollama-usage.jsonl`
——即 `ollama-proxy` 数据源，使详细请求表能显示单次调用（TTFT/TPS），取代原先 dim
本地库「一次 run 一行」的聚合。

### StepFun Step Plan 逐请求

**StepFun Step Plan 逐请求说明**：同一个 CPA 实例（8317）再加 `openai-compatibility` 条目
`stepfun`（`prefix: "step"`、`base-url: https://api.stepfun.com/step_plan/v1`、仅
`step-5-preview`，`max-context-length: 1000000`、`input-modalities: [text, image]`）——
dim 侧模型名因此是 `step/step-5-preview`。API key 必须写成**明文**放进 `config.yaml`
（0600）：CLIProxyAPI **不展开** `${VAR}`，直接透传字面量 → StepFun 返回
`API-key is invalid`(10501)；值从 `~/.bash_env` 的 `STEPFUN_API_KEY` 抄。配套的
usage-only 插件 `stepfun-usage.so`（源码 `~/workbuddy-proxy/stepfun-usage-plugin/`，
`ollama-usage-plugin` 的 fork）把用量 append 到 `~/.token-stats/stepfun-usage.jsonl`
——即 `stepfun-proxy` 数据源（`provider=stepfun`）。插件**写日志前剥掉 `step/` 前缀**
（`logUsage` 里的 `strings.TrimPrefix`），与 `pricing.toml` / vendor_merge 的裸模型名对齐
（`display_cost` 按裸名查价目，漏剥会查不到价显示 N/A）。

## 跨源约定

### 来源 id vs 显示名（约定）

**来源 id vs 显示名（约定）**：`TokenRecord.source` 是**传输通道 id**，由写日志的一方
固定（`ollama-proxy` = CPA 的 `ollama-usage` 插件；`stepfun-proxy` = CPA 的
`stepfun-usage` 插件；`dim-agent` = workbuddy 插件；
`cc-proxy` = 内置 CC 代理），用于去重指纹、迁移谓词（`store.rs` / `app.rs` 的
`source='ollama-proxy'` 等）与增量解析；**不要**为了 UI 好看改它——改了会让历史记录与
新记录分属两个 source，指纹不同 → 双计。UI 文案只在
`frontend/src/lib/utils.ts` 的 `SOURCE_LABELS` / `SOURCE_COLORS` 里映射
（当前 `ollama-proxy → "Dim→Ollama"`、`dim-agent → "Dim→CB"`、`cc-proxy → "Dim→CC"`、
`stepfun-proxy → "Dim→StepFun"`），
未登记的 source 会原样显示 id（如旧版曾显示的 "ollama-proxy"）→ 新增来源必须补这两个表。

### 每个 CPA 上游都要配一个 token-stats 用量插件（不变式）

**每个 CPA 上游都要配一个 token-stats 用量插件（不变式）**：CPA（`:8317`）上的通道只有
自己写的 usage 插件能逐请求计量（dim 本地库对 CPA 通道**只有 `usage_run_stats` 的按 run
聚合行**，已被 `dim` 源的回环地址排除规则丢掉）。加新上游时四步一起做：① `config.yaml`
的 `openai-compatibility` 条目（`prefix` 命名空间 + 显式 `models:`）② fork 一个
`*-usage-plugin`（改 `providerMatch`/`modelPrefix`/`sourceName`/`providerName`/env 名，
写日志前 `TrimPrefix` 剥回裸模型名以对齐 pricing 与 vendor_merge 的键）③ 后端新增
`sources/<name>_proxy.rs` 并注册 ④ dim 侧**无需改动**——排除按 baseUrl 地址判定，
不在名字白名单里。

### 「保留 agent 名」目前做不到

**「保留 agent 名」目前做不到**：CPA 的 `UsageRecord.Source` 是 `resolveUsageSource()`
从上游凭据推导的（OAuth 账号邮箱、api-key 明文），**不是**下游客户端身份；
`api-keys:` 里的 `cb-local-key` 只进 `userApiKey` gin context，`usageAdapter.HandleUsage`
不会把它透给插件。所以插件拿不到「谁发起的」——同一实例上 workbuddy 与 ollama 的流量
只能靠 `Provider`（`openai-compatible-<name>`）区分，日志里的 `"apiKeyPrefix":"N/A"` 即
此原因。若将来要按调用方（DimAgent / 其他客户端）拆分用量，可行路径是给每个客户端
**分配独立 api-key**，厂商自行维护 `api-key → agent` 映射（token-stats 侧只读日志，
无法还原）。

### 模型命名空间（重要）

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

### dim 侧只保留一个 provider

**dim 侧只保留一个 provider**：`ollama-cloud-proxy` 与 `workbuddy` 指向同一个
CPA（`http://127.0.0.1:8317/v1`）且拉到同一份 37 个模型目录，属于重复配置——
2026-09-12 起只保留 `workbuddy`（`dim provider update workbuddy --name "CPA(ollama+wb)"`），
用 `ollama/*` 与 `wb/*` 前缀区分通道；`ollama-cloud-proxy` 与旧的直连
`custom-ollama-cloud-042036d3`（`https://ollama.com/v1`）均已 `dim provider remove`。
新增/切换模型后必须 `dim model refresh workbuddy`。

**平台新增计费模型 → 必须同步登记 `pricing.toml` 的 `[[dim_model]]`**：dim 源能逐请求
拿到平台上的**任意**新模型（`item.model_name` 原样写入记录，`provider` 恒为 `"dim"`），
但 `display_cost()` 只认 `[[dim_model]]` 里的名字。漏登记时该模型的成本返回 `-1`
（前端 N/A、聚合直接剔除），**token 与调用次数还在、费用却凭空消失**——不报错，
只会让当日账单对不上。2026-09-28 的 `mimo-v2.6-flash` 就是这么漏了 9 次调用。
登记时**先**照抄价目卡 `GET https://dimagent.cn/api/public/website/plans?line=dimcode`
的 Lite 套餐（8102）`models[]` 三个字段，× 70/11000 换算 CNY，然后**必须**用
`/api/user/daily-stats` 的 `base_amount_minor`（原价）/`final_amount_minor`（实扣）
当天对账（见 [`pricing.md`](./pricing.md)）。⚠️ **价目卡的字段顺序不保证等于结算口径**：
`mimo-v2.6-flash` 的价目卡 6 个套餐一致地写 `input 140 / output 280 / cache_read 2.8`，
但账本实收是 `140 / 2.8 / 280`（output 与 cache_read 摆反）。所以新模型要按**整天**
对账（不是半天快照）再定稿；对不上时以账本为准，并把结论写进 `pricing.toml` 注释。

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
