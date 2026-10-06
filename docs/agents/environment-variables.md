# 环境变量

> 从 [`AGENTS.md`](../../AGENTS.md) 拆出。部署时这些值由 `deploy.sh` 写入 systemd drop-in，
> 来源是 `~/.config/token-stats/deploy-env.sh`（正确入口 `./scripts/deploy-dashboard.sh`，
> 见 [`pitfalls.md`](pitfalls.md) 第 17 条）。配额卡凭据的取法见
> [`quota-cards.md`](quota-cards.md)。

| 变量 | 默认 | 说明 |
|------|------|------|
| `PORT` | `3000` | 后端端口 |
| `RUST_LOG` | - | 日志级别（`info`、`debug`、`trace`） |
| `MALLOC_ARENA_MAX` | 单元里固定为 `2` | **不是本程序读取的变量**，glibc malloc 自己读的。Rust 分配已由 `#[global_allocator]` 交给 mimalloc，这条管的是仍走 libc 的部分（bundled SQLite 等）：默认按线程开到 8×nproc 个 arena 且只借不还，实测吃出 655 MB。删掉它会让常驻内存明显回涨，见 [`pitfalls.md`](pitfalls.md) 第 24 条 |
| `https_proxy` | 单元里设为 `http://127.0.0.1:7800`（配 `no_proxy=localhost,127.0.0.1,::1`） | **不是本程序读取的变量**，reqwest 自己读的（hyper-util env matcher）。`api.commandcode.ai` / `opencode.ai` 直连被 DNS 污染（HTTP 000 / 证书过期），配额抓取必须走本机 7800 出口；其他配额主机直连即可，设此变量后也一并经代理。删掉会让 CommandCode / OpenCode 配额卡整卡报「所有 API 请求失败」 |
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
| `GLM_PROXY_PORT` | `3435` | loopback GLM 代理端口（Paseo `glm-acp-agent` 接入；agent 侧 `ACP_GLM_BASE_URL` 指向 `http://127.0.0.1:3435/api/coding/paas/v4`，配置在 `~/.paseo/config.json` 的 provider env，非后端变量） |
| `GLM_PROXY_UPSTREAM_BASE_URL` | `https://api.z.ai` | GLM 代理上游基址（路径原样透传） |
| `GLM_ACP_USAGE_LOG_PATH` | `~/.token-stats/glm-acp-usage.jsonl` | GLM 代理用量日志覆盖 |
| `WORKBUDDY_USAGE_LOG_PATH` | `~/.token-stats/workbuddy-usage.jsonl` | WorkBuddy（CodeBuddy Web API）代理用量日志覆盖 |
| `OLLAMA_PROXY_USAGE_LOG_PATH` | `~/.token-stats/ollama-usage.jsonl` | CPA `ollama-usage` 插件（Ollama Cloud 逐请求）用量日志覆盖；插件与后端读取同一变量 |
| `STEPFUN_PROXY_USAGE_LOG_PATH` | `~/.token-stats/stepfun-usage.jsonl` | CPA `stepfun-usage` 插件（StepFun Step Plan 逐请求）用量日志覆盖；插件与后端读取同一变量 |
| `CPA_LOOPBACK_ADDRS` | `127.0.0.1:8317,localhost:8317,[::1]:8317` | `dim` 源本地补充据此判定「通道 baseUrl 指向 CPA」并丢弃其 per-run 行（CPA 侧已有逐请求插件计量）；换 CPA 端口时改这里 |
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
| `DIM_ENTITLEMENT_STATE_PATH` | `~/.config/token-stats/dim-entitlement.json` | Dim entitlement 折扣的分段历史（「rate 从何时起生效」的唯一记录来源） |
| `DIM_ENTITLEMENT_TTL_SECS` | `1800` | 后台重新读取 Dim entitlement rate 的最小间隔（每次都要 spawn `dim usage`，故放慢） |
| `TASKPLANE_PROJECTS_DIR` | `~/srcs` | Taskplane runtime 扫描根目录覆盖 |
| `OPENCODE_GO_WORKSPACE_ID(_EX)` | 未设置 | OpenCode Go 工作区 ID（配额卡必需） |
| `OPENCODE_GO_AUTH_COOKIE(_EX)` | 未设置 | OpenCode Go `auth` cookie（配额卡必需） |
| `XIAOMI_MIMO_SERVICE_TOKEN` / `XIAOMI_MIMO_USER_ID` | 未设置 | 小米 MiMo 配额卡凭据 |
| `OLLAMA_API_KEY` | 未设置 | Ollama Cloud 配额卡**主**凭据（与 CPA `ollama-cloud` 上游同一 key）；走 `POST /api/me` + `GET /api/usage` |
| `OLLAMA_AUTH_COOKIE` | 未设置 | Ollama Cloud 会话 cookie：主路径用它抓 `/settings` 网页端的重置时间；`OLLAMA_API_KEY` 缺失/失败时还是整个卡片的 HTML 回退凭据 |
| `OLLAMA_WINDOW_STATE_PATH` | `~/.config/token-stats/ollama-window.json` | Ollama session/weekly 用量窗口的网格相位（网页时间不可用时的重置时间兜底预测，可删，删后下一轮从本地计量重新 bootstrap） |
| `MEITUAN_AUTH_COOKIE` | 未设置 | 美团 LongCat `passport_token_key` |
| `FENNO_AUTH_TOKEN` | 未设置 | Fenno 初始访问 JWT（仅引导） |
| `FENNO_REFRESH_TOKEN` | 未设置 | Fenno 初始刷新 token（轮换后自动持久化） |
| `FENNO_AUTH_STATE_PATH` | `~/.config/token-stats/fenno-auth.json` | 轮换凭据状态文件 |
| `YAI_API_KEY` | 未设置 | Ainaiba/XAI 余额查询 Bearer token |
| `STEPFUN_API_KEY` | 未设置 | StepFun 配额卡（`/v1/accounts` credit 余额）查询 Bearer token；与 CPA `stepfun` 上游同一 key（`~/.bash_env`），deploy.sh 注入 systemd drop-in |
| `STEPFUN_OASIS_TOKEN` + `STEPFUN_OASIS_WEBID` | 未设置 | StepFun 配额卡的 **Step Plan 月池**引导登录态；**两者必须成对**——网关交叉校验，只给一个返回 "oasis-token is embezzled"。`./scripts/deploy-dashboard.sh` / `deploy.sh` 在 source 凭据文件前跑 `scripts/refresh-stepfun-token.sh --env-only`：从 Chrome 提取 `access...refresh`，调 `RefreshToken` 轮换，写回 `deploy-env.sh` 和 `stepfun-auth.json`。Chrome/keyring 不可用只告警、沿用旧值。`SKIP_STEPFUN_TOKEN_REFRESH=1` 跳过。cookie `expires` 约一年，JWT 本身约 30 分钟；运行中的后端继续用 `STEPFUN_AUTH_STATE_PATH` 自行续期 |
| `STEPFUN_AUTH_STATE_PATH` | `~/.config/token-stats/stepfun-auth.json` | Step Plan 轮换后的 access/refresh JWT（0600）。优先于环境变量里的引导 token |
| `ADVANCED_MODELS_CONFIG` | `~/.config/token-stats/advanced-models.json` | 高级模型编辑存储 |
| `SUBSCRIPTION_SETTINGS_CONFIG` | `~/.config/token-stats/subscription.json` | 订阅设置存储 |
| `DIMAGENT_SESSION_COOKIE` | 未设置 | DimAgent 会话 cookie（仅值，不含 `session=` 前缀）。**必需**：`dim` 数据源用它轮询 console API（每次刷新循环，默认 30s）；配额卡也用它作 CLI 回退 + 近 30 天统计增强。**约 30 天过期**，且失效只走优雅降级（见陷阱 22 的双源黑洞）。刷新：`./scripts/refresh-dimagent-cookie.sh`（从 Chrome 提取并实测 200），`deploy.sh` 已自动执行 |
| `DIM_USAGE_BIN` | 自动发现 | `dim usage --json` 二进制覆盖（仅配额卡主路径；默认扫描 `~/.dimcode/binaries/dimcode-linux-x64/*/bin/dimcode` 最新版 → PATH `dim`） |
