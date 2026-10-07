# 配额卡数据源（`GET /api/quota`）

> 从 [`AGENTS.md`](../../AGENTS.md) 拆出。AGENTS.md 只留「卡 → 凭据」的一行索引；
> 本文件是每张卡的端点、认证细节与逆向结论。改 `backend/src/quota/*.rs` 前先读对应行。
> 环境变量清单见 [`environment-variables.md`](environment-variables.md)，计费口径见
> [`pricing.md`](pricing.md)。

| 卡 | 来源 | 配置 |
|----|------|------|
| Kimi / Kimi EX | `https://auth.kimi.com` 刷新 token 后查 `/usages` | `KIMI_CREDENTIALS_PATH` / `KIMI_CREDENTIALS_PATH_EX`；EX 默认指向 `~/.kimi-code-user2/credentials/kimi-code.json`；`KIMI_AUTH_BASE_URL` 可覆盖 |
| OpenCode Go / OpenCode Go EX | HTTP 抓取 `https://opencode.ai/workspace/{id}/go` 的 `<div data-slot="usage">`（`reqwest`+`scraper`） | `OPENCODE_GO_WORKSPACE_ID(_EX)` + `OPENCODE_GO_AUTH_COOKIE(_EX)` |
| Xiaomi MiMo | MiMo token 计划 API | `XIAOMI_MIMO_SERVICE_TOKEN` + `XIAOMI_MIMO_USER_ID` |
| Command Code | `https://api.commandcode.ai`（`/alpha/billing/subscriptions`、`/alpha/billing/credits`、`/alpha/usage/summary`）；主账号从 `~/.commandcode/auth.json` 的 `apiKey`（Bearer），第二账号（EX）从 `auth*.json`（如 `auth_frank.json`）——与主账号 apiKey/userId 相同的重复 auth*.json 会被跳过（否则 EX 卡会显示主账号）；无 auth 文件时回退 `COMMANDCODE_SESSION_TOKEN` cookie（`/internal/*` 旧路由） | `COMMANDCODE_SESSION_TOKEN` 作为 `__Secure-commandcode_prod_.session_token` cookie（仅回退） |
| CodeBuddy 套餐 | `www.codebuddy.cn` billing meter API（`POST /billing/meter/get-user-resource-summary` 取各套餐包周期总量/剩余，`POST /billing/meter/get-user-resource` 取套餐名与周期；即 `/profile/plans-usage` 页同源接口）。**必需 `session` + `session_2` 两个 cookie**（单 `session` 返回 401）；边缘 WAF 拒绝过旧 Chrome UA（Chrome/126 被拦、152 可过）。cookie 从 Chrome 提取：`scripts/extract-codebuddy-cookies.sh`（约 30 天过期需重取） | `CODEBUDDY_SESSION_COOKIE` + `CODEBUDDY_SESSION_COOKIE_2`（仅 cookie 值） |
| Ollama Cloud | **主路径**：JSON API `POST https://ollama.com/api/me`（`Plan`、`CreatedAt`）+ `GET https://ollama.com/api/usage`（原返回 `limits.session.usage` / `limits.weekly.usage`，**分数**：`0.293` → 29.3%；**2026-10-07 起该端点改为 `range/totals/buckets` 的按日请求数，`limits` 字段消失** —— session/weekly 百分比因此改从 `/settings` 页面抓取（与重置时间同一次请求，API 值若回归仍优先）；响应内始终**无重置时间**）。鉴权用 `OLLAMA_API_KEY` —— **与 CPA `ollama-cloud` 上游同一 key**，因此卡片显示的必然是实际跑流量的那个账号。踩坑：本仓库 `reqwest` 没开 `http2` feature（HTTP/1.1），ollama 的 Go 服务对无 body 的 POST 返回 `411 Length Required`，必须显式带 `Content-Length: 0`。**回退**：`GET /settings/billing` + `/settings` 抓 HTML（`OLLAMA_AUTH_COOKIE`）——只认「Current Plan: X」和「Session usage / Weekly usage」标签，ollama 改版（`Included usage` + `free`/`pro` badge、单个 `Free usage` 表）后会退化为 `plan_name: "Unknown"` + 空用量；且 cookie 属于**哪个账号就显示哪个账号**（实测浏览器 cookie 指向免费号、API key 指向 Pro 号时会误导） | `OLLAMA_API_KEY`（主，`~/.bash_env`，deploy.sh 注入 drop-in）；`OLLAMA_AUTH_COOKIE`（仅回退） |
| Meituan LongCat | 美团 API | `MEITUAN_AUTH_COOKIE`（`passport_token_key`） |
| Fenno / Fenno EX | `https://api.fenno.ai/api/v1/subscriptions/active` | `FENNO_AUTH_TOKEN` + `FENNO_REFRESH_TOKEN` 引导凭据管理器；轮换凭据持久化到 `FENNO_AUTH_STATE_PATH`（默认 `~/.config/token-stats/fenno-auth.json`）并自动刷新 |
| Grok | `POST grok.com/grok_api_v2.GrokBuildBilling/GetGrokCreditsConfig`（gRPC-web protobuf）取 SuperGrok 周池百分比 + `grok-cli` 记录作诊断 | `GROK_XAI_API_KEY` / `~/.grok/auth.json`。config field 1 = 已用%；**proto3 省略默认 0.0**，周重置后缺该字段应视为 0% 而非错误（`quota/grok.rs`） |
| Ainaiba 余额 | `api-xai.ainaibahub.com` | `YAI_API_KEY`（`/api/ainaiba-credit` 端点） |
| ZAI | `api.zairouter.com` 的 `/dashboard/info` + `/dashboard/live` + `/dashboard/status`（Bearer `ZAI_API_KEY`）——账户、到账卡、逐模型日/月用量、`suspended`。**计费**：充值 **1 元 = 1 美元额度**（订单实测 `amount`=10000 分 → `credit_amount`=100.0），但按平台内部价目表扣费，**不对外公布且不是官方价的统一倍数**；实测费率登记在 `pricing.toml` 的 `[[zai_model]]`（见 [`pricing.md`](pricing.md)）。`claude-haiku-4-5` 在该订阅下不可用（平台返回 long-context beta 未开通）。余额符号取决于账号 `factor`（1→`¥`，否则 `$`），本机 `factor=1` 但 1:1 兑换使两者数值相同 | `ZAI_API_KEY`（浏览器控制台/`~/.bash_env`）。注意与 `YAI_API_KEY` 是**两个不同账号**（充值独立、倍率独立） |
| DimAgent | **主路径**：本地 `dim usage --json`（CLI 自动发现，见 `quota/dimagent.rs`；OAuth 凭据在 `~/.dimcode/v2/auth.json`，CLI 自动刷新，无需任何环境变量）。**回退**：console API `dimagent.cn/api`（`/me/subscription` + `/me/credits` + `/me/feature-meters` + `/user/quota-estimate`） | `DIMAGENT_SESSION_COOKIE`（浏览器 `session` cookie 值）仅用于回退和近 30 天统计增强；`DIM_USAGE_BIN` 可覆盖 CLI 二进制 |
| StepFun credit 余额 | 官方 OpenAPI `GET https://api.stepfun.com/v1/accounts`（Bearer `STEPFUN_API_KEY`，即控制台 account-overview 页的 credit 余额，`quota/stepfun.rs`）——`balance` 可用余额 + `total_cash_balance` 累计充值 + `total_voucher_balance` 累计赠送，CNY 计价；无需 cookie | `STEPFUN_API_KEY`（与 CPA `stepfun` 上游同一 key，`~/.bash_env`；deploy.sh 注入 drop-in）。注意这是**余额账户**，与 Step Plan 套餐额度（¥99/¥1600 折算，见 [`pricing.md`](pricing.md)）是两回事。**套餐月池半区**：`POST https://platform.stepfun.com/api/step.openapi.devcenter.Dashboard/QueryStepPlanRateLimit` + `GetStepPlanStatus`（Connect-RPC，从控制台前端 bundle 逆向；官方 OpenAPI 没有任何套餐接口，`/v1/step_plan/*` 全 404）。头必须同时带 `Oasis-Token` + `Oasis-Webid` + `Oasis-appID: 10300` + `Oasis-Platform: web`——token 与 webid 交叉校验，只给一个返回 "oasis-token is embezzled"。取 `plan_credit_rate_limit.subscription_credit_left_rate`（0–1 剩余比例）与 `credit_buckets[]`（type=1 为月池；`credit_total`/`credit_residual` 是 int64 **字符串**，1M credit = ¥1 列表价，月末清零）。凭据 `STEPFUN_OASIS_TOKEN` + `STEPFUN_OASIS_WEBID`（`./scripts/extract-stepfun-token.sh` 从 Chrome 提取）。**cookie 的 `expires` 约一年只是浏览器存储期限；JWT `exp` 只有约 30 分钟**（2026-09-22 实测 platform cookie JWT 当天即过期，接口 401 `token is expired`）。续期走 `POST /passport/proto.api.passport.v1.PassportService/RefreshToken`（body `{}`，头带当前 token + webid），返回轮换的 `accessToken.raw` / `refreshToken.raw`（`duration` 1800s）。后端在到期前 120s 自动续期并写入 `STEPFUN_AUTH_STATE_PATH`（默认 `~/.config/token-stats/stepfun-auth.json`，0600）。引导 token 可以是裸 access JWT，或 CodexBar 的 `access...refresh` 拼接；没有 refresh 半段时过期无法自愈，卡片余额半区仍在，月池半区改为显示 `plan_error` 而不是静默消失。缺配时同样只显余额 |
| ZCode | **主路径**：BigModel monitor API（逆向 ZCode 桌面应用 app.asar 得出，`quota/zcode.rs`）：`GET open.bigmodel.cn/api/monitor/usage/quota/limit`（套餐 level + 限额窗口；裸 apiKey 放 `authorization` 头，无 Bearer 前缀）+ `GET open.bigmodel.cn/api/biz/subscription/list`（套餐名/续订/到期）。apiKey 从 `~/.zcode/v2/config.json` 的 `provider["builtin:bigmodel-coding-plan"].options.apiKey` 读取（应用自动轮换）。**用量半区**：`source='zcode'` 内存记录聚合（今日/累计 调用、tokens、成本），其中 `provider='bigmodel-start'`（体验套餐）行被排除、单独进 `data.startPlan`（本地账本：grantTokens 总量默认 3 亿 / usedTokens / remainingTokens / calls；monitor API 拒收 start-plan key 返回 401，无远端半区）。**字段语义陷阱**：`usage`=窗口总量、`currentValue`=已用、`remaining`=剩余、`percentage`=已用百分比（currentValue/usage）——命名有误导，前端统一走 `zcodeWindowUsage()`。60s 缓存防 30s 轮询打满；远程失败但本地有记录时 `available:true` + `data.quota_error` 降级显示 | `ZCODE_BIGMODEL_USAGE_API_KEY` / `ZCODE_BIGMODEL_USAGE_QUOTA_URL`（与应用自身 env 名一致）、`ZCODE_CONFIG_PATH`（默认 `~/.zcode/v2/config.json`） |

## Ollama 用量窗口（session 5h / weekly 7d）逆向结论

`quota/ollama.rs` 的卡片数据分三块：

1. **百分比** —— 2026-10-07 前由 `/api/usage` 直接给（`limits.session.usage` /
   `limits.weekly.usage`）；该端点改版后 `limits` 消失（改为按日请求数桶），改抓 `/settings`
   页面渲染的「Session usage X% used / Weekly usage Y% used」（与重置时间同一次抓取；API 值
   一旦恢复仍优先）。
2. **重置时间** —— API 不给，**以网页端 `/settings` 为准**：每个用量区块各渲染一个
   `.local-time[data-time]`（如 `Resets in 2 days.` / 周预算耗尽时 Session 区块显示
   `Sessions resume in 2 days.`），代码按「Session usage / Weekly usage」标签切分区块、
   取各自区块内第一个 `.local-time`（`parse_web_reset_times`）。抓取需要
   `OLLAMA_AUTH_COOKIE` 且指向与 API key 相同的账号（旧 API 的 `limits.*.models`
   明细与页面模型列表一致，可用作核对；该明细已随 2026-10-07 改版消失，按 `request_count`
   反推窗口起点的 bootstrap 也随之失效，只剩网页回锚与观测翻转两条路径）。**为什么不能本地推算**：weekly 窗口是**日历对齐**的
   （2026-09-25 实测页面给出 `2026-09-28T00:00:00Z` = 周一 00:00 UTC），不是「首个请求锚定
   的 7d 网格」——旧实现从计量记录回数 `request_count` 反推相位，bootstrap 偏早（非 CPA
   流量计入 API 计数）+ 本周用量顶格 100% 使「用量骤降」翻转检测永不触发，错误相位持续
   被沿用，卡片显示的重置时间晚了 4 天多。网页时间每次轮询都会把持久化相位**回锚**到
   `reset − period`（`OLLAMA_WINDOW_STATE_PATH`，默认 `~/.config/token-stats/ollama-window.json`）；
   仅当页面不可用（无 cookie / 改版）时才退回相位模型（bootstrap 回数 + 观测翻转 + 网格预测）。
   「Sessions resume」时间只用于展示，不会写进 session 相位（它不是 5h 网格点）。
3. **本周 token / 成本** —— 窗口内 `source="ollama-proxy"` 记录求和（`total_tokens` 实际值 +
   `pricing::display_cost` 的经验订阅费率），取代旧的「百分比 × 经验周配额」估算；窗口起点
   优先用网页回锚后的相位（= 页面重置时间 − 7d）。

**用量口径（校验额度用）**：ollama 的百分比 = 窗口内**目录价美元**消耗 ÷ 窗口额度，**按 peak 价计（不打 off-peak 折扣）**。用 `deepseek-v4.1-flash` 目录价（in $0.30 / cached $0.006 / out $1.20 每 M）对本机记录求和，实测 weekly 额度 $60（三个快照推得 $59.78–$59.99，偏差 <0.3%）、session 额度约 $10（噪声 ±20%，只适合做量级校验）。`/api/me` 的 `CreatedAt` 日号 = 月度订阅周期（免费号实测每月同日重置；本机账号 2026-06-26 创建 → 续费日 26 号），卡片「续费」日期即由此推出。


## DimAgent console API 逆向结论

**DimAgent console API 逆向结论**（`quota/dimagent.rs` / `sources/dim.rs` 验证过）：
- `GET /api/user/self`、`/api/log/self`（逐次调用明细：`prompt_tokens`/`completion_tokens`/`cache_tokens`/`use_time_ms`/`ttft_ms`/`tps`/`model_name`/`token_name`）、`/api/user/daily-stats`（按日汇总：各 token 字段 + `request_count` + `quota_consumed`）、`/api/me/subscription`、`/api/me/credits`、`/api/me/feature-meters`、`/api/user/quota-estimate` —— 全部只需 `session` cookie（GET）。
- **`/api/log/self` 分页参数是 `p`**（`page` 会被服务端忽略，总是返回第 1 页）；`page_size` 上限 100；`type=2` 为用量日志筛选（Activity 页同款）；响应按 id 倒序（新→旧），带 `total`/`total_capped`。
- token 约定为 OpenAI 式：`prompt_tokens` **包含** `cache_tokens`（用 `/api/user/daily-stats` 验证：`total_tokens = prompt_tokens + completion_tokens`）；`cache_tokens` 即缓存命中（读）；API 无 cache 写入字段（daily-stats `cache_creation_tokens` 恒为 0）。
- 两个 cookie 的作用：`session`（Flask/itsdangerous 签名会话，唯一认证凭据，必需）；`_c_WBKFRo`（站点统计/风控 cookie，**非认证必需**，可弃用）。
- 本地 dimcode 库（`usage_run_stats`）是**按 run 聚合**（含 input/output/cache/model/cost）；逐调用明细（TTFT/TPS/每次调用的缓存命中）只存在于 console API，`dim usage --json` 与本地库都没有。
- CLI 输出与 console API 的 units 单位不同：CLI 是整单位（如 1500），console API 是毫单位（×1000，如 1500000），`card_from_parts()` 按总量阈值自动归一化。
