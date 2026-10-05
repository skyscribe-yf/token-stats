# 成本计算（pricing.toml 重点）

> 从 [`AGENTS.md`](../../AGENTS.md) 拆出。**改任何计费逻辑、或调整 `pricing.toml` 前先读
> 本文件。** 与计费强相关的两条陷阱（CodeBuddy 分段、Dim `week_days`）在
> [`pitfalls.md`](pitfalls.md) 的 18 / 21 条。

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
- **GPT-6.1 Sol（2026-09-30 发布）**：OpenAI 官方标准价 input $2 / cached input $0.10
  （输入价 5%）/ cache_write = input × 1.25 = $2.50 / output $10；长上下文档（>272K 输入）
  input 与 cache ×2、output ×1.5 → $4 / $0.20 / $5 / $15。走**通用** `[[model]]` +
  Yairouter 平台固定汇率 7.0 ÷ ainaba divisor（当前 21.538461538，等价于 官方 USD × 0.325）
  ——与 gpt-5.6-sol 同一条路径，**不要**加 `[[yairouter_model]]` 覆盖。2026-10-01 受控探针
  （`/v1/chat/completions`，逐次核对 `/dashboard/live` 的 `ModelUsage.gpt-6.1-sol` 增量）：
  base 档 (3514×$2 + 5×$10)/1M × 7 = 0.049546；cache 命中档（3840 cached ×$0.10 + 517
  未缓存 + 5 out）0.071624，均与账本分毫不差。
- **GPT-5.6 Sol 官方降价（2026-08-22，Yairouter/Fenno 同步）**：短上下文
  5/30/0.5/6.25 → **4/20/0.4/5**，长上下文 10/45/1/12.5 → **8/30/0.8/10**（IT之家
  08-22 12:01 CST 报道；官方页现标注为促销价 “available at least through
  November 21, 2026”）。pricing.toml 以 `effective_from = "2026-08-22T00:00:00+08:00"`
  加段、**基准段保留降价前价格**（08-22 全天无 Sol 流量，cutoff 取 00:00 CST 不触及任何
  记录）。同日账本增量核算与 4/0.4/20 × factor(7) 完全一致；促销到期后需再加分段。
- **Yairouter `gpt-5.6-terra` / `gpt-5.6-luna` 实测价与 `[[yairouter_model]]` 覆盖不符
  （2026-10-01 受控探针，尚未处理）**：两者实测都按 **input $2 / cached $0.2 / output $12**
  计费（luna：467 未缓存 + 3840 cached + 5 out → 0.024668 = (467×2 + 3840×0.2 + 5×12)/1M × 7；
  terra：(4307×2 + 5×12)/1M × 7 = 0.060718，两次独立探针一致；当日 luna 账本桶整体也精确
  落在 2/12）。而覆盖表里 terra = 2.5/15、luna = 1/6/0.1（08-17 起）。terra 与官方 07-31
  降价后的 2/12 相同；luna 则比官方 0.2/1.2 贵 10×，形似平台把 luna 按 terra 价计。
  **改前需确认生效时间**（无历史账本可回溯，只能确认 10-01 当天）；cache_write 与长上下文
  档位未实测。
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
  **OpenCode 2.x 起不再记录 per-message cost（恒为 0）**，这类记录改走「按 token ×
  `[[model]]` 价 ÷ divisor」的派生分支（`opencode` 已加入 pricing.rs 分支 6 的 source 列表）。
  因此 2.x 用到的免费模型（`space-bunny-free` / `ox-alpha-free` / `nemotron-3-ultra-free` /
  `mimo-v2.5-free`）必须显式登记 0 价，否则会掉进末尾的 `-1`（前端 N/A）而不是 ¥0。
  注意 `output_tokens` **已含 reasoning**（OpenCode 按输出价计 reasoning，见
  [`data-sources.md`](./data-sources.md) 的 opencode 条目），所以这里不能再额外加一次。
- **Dim（console API 源）**：不存储原始 cost，`display_cost()` 走"衍生源"分支——按
  pricing.toml **`[[dim_model]]` 平台目录原价**（Lite 套餐 ¥70/11000 积分换算 CNY，如
  vision-exp input=0.653798 / output=3.922790 / cache_read=0.043587 元每 1M），
  vision-exp 高峰时段（CST 09:00–12:00 / 14:00–18:00 = UTC 01–04 / 06–10）按
  `peak_*_cny` 双倍（客服确认，2026-08-22~09-02 六天日账闭合验证）；v4-flash 无高峰
  双倍按接口价（0.871818 / 1.743636 / 0.017436）；deepseek-v4.1-flash-expires-on-0910
  （2026-09-08 上线的 v4.1 flash 临时命名）费率与 vision-exp 相同
  （0.653798 / 3.922790 / 0.043587，含高峰双倍）；**2026-09-10 平台把该模型改名为
  `deepseek-v4.1-flash` 并下调费率**，正式名按价目卡**目录原价**计费（0.890909 /
  3.500000 / 0.017182，忙时双倍 = 1.781818 / 7.000000 / 0.034364；当日日账闭合验证
  <0.01%）——两个名字都保留在 `[[dim_model]]` 中（旧名仅供历史记录，最后
  一条 2026-09-09T23:14Z），改名漏配会让该模型成本显示 0.00/N/A；seed-2.0-mini 按
  价目卡（0.174346 / 1.743458 / 0.034869，无高峰）；glm-5.3-flash 无折扣按接口价
  （0.477273 / 1.590909 / 0.095455）；**`mimo-v2.6-flash`（2026-09-28 上线）的结算口径被平台
  改过一次，因此 `[[dim_model]]` 里有两段，两段都别删**：
  - **2026-09-29 起**（`effective_from = "2026-09-29T00:00:00+08:00"`）= 价目卡口径
    input 140 / output 280 / cache_read 2.8 积分/M
    （0.890909 / 1.781818 / 0.017818 CNY/M，无忙时、无 cache 写入）；
  - **2026-09-28 及以前**（基线段）= input 140 / output **2.8** / cache_read **280**
    （0.890909 / 0.017818 / 1.781818）——当时账本实收就是这样，与价目卡字段顺序**相反**。
  - **`mimo-v2.6-pro`（2026-10-01 上线）** = 价目卡 6 个套餐一致写 input 435 / output 870 /
    cache_read 3.6 积分/M（**2.768182 / 5.536364 / 0.022909 CNY/M**），且字段顺序与结算
    口径一致（与 flash 那个反序的坑不同，照抄即可）。无忙时窗口、entitlement rate **1.0**
    （`model_access[]` 里该模型无 `rate`，`base == final` 印证）。对账见下方"整桶对账"。

  **`mimo-v2.6-pro` 的整桶对账**（`/api/user/daily-stats`，`interval=hourly`）：10-01 14:00
  CST 桶混了 deepseek-v4.1-flash，用它已验证的原价扣掉再解 mimo。前提是先在本地记录里
  找到与账本 token 口径**逐项精确相等**的前缀 = 前 311 条（prompt 594226 + cache 28232064
  = 账本 28826290、completion 206539、cache 28232064），其中 deepseek 306 条、mimo 5 条。
  按目录原价加总 286429.589 vs `base_amount_minor` 286432 → **0.999992x**；按 entitlement
  （deepseek ×0.65、mimo ×1.0）加总 193473.325 vs `final_amount_minor` 193477 → **0.999981x**。
  ⚠️ **别拿 `request_count`（当时 316）做前缀匹配**——它把尚未落账的在途请求也算进去了，
  照它算会对不上（`verify-dim-billing.py` 的前缀法在这个桶会误报）。以 token 口径为准。

  **`[[special.dim_offpeak_windows]]` = 平台全站闲时窗口**：节假日平台把**整个模型目录**
  按闲时计费，所以这不是某个模型的 `peak_hours_utc`，而是覆盖全 `dim` 源的开关——窗口内
  所有记录按基础价，`peak_*` 双倍一律不生效（`pricing.rs::in_offpeak_window` →
  `compute_cny(force_off_peak)`；`verify-dim-billing.py` 已同步）。
  当前一条：**2026 国庆** `from = "2026-09-30T11:21:00+08:00"` / `to = "2026-10-08"`。
  - 法定假期是 10-01~10-07（国办发明电〔2025〕7 号），但平台**提前到 09-30 中午**切换：
    09-30 11:00 CST 桶里 deepseek 的 228 条记录**前 62 条按忙时 2×、之后按闲时 1×**，
    逐条累加只有「11:20:54 之后 / 11:21:43 之前切换」这一个解（残差 1.9 毫积分 / 0.0007%，
    下一个候选解残差跳到 481）；12:00 起的桶 `base_amount_minor` 全部按闲时闭合，
    `final/base` 恒为 0.65。cutoff 取整到分钟。
  - 被改写的历史只有 **09-30 11:21–12:00** 这一段（12:00 以后本来就不忙）；09-30 之前
    不能提前——09-20 同时段账本确实按忙时收的。
  - **`to` 故意写死**（不留空）：留空会让窗口一直生效，之后每个忙时段少算一半且不报错。
    若平台延长假期就改这个 `to`，改完跑一次 `verify-dim-billing.py --interval hourly`；
    10-08 恢复后同样跑一次，确认桶回到 2×。**讯飞的节假日走
    `[special.xunfei_off_peak] holidays`（按日期），与这个无关。**

  两段各自对账（`/api/user/daily-stats`）：① 09-28 CST 整天 21 次 / input 66334 +
  output 18381 + cache 1008960 → 291.847 积分 = `final_amount_minor` 291847
  （`base == final` ⇒ entitlement rate 1.0，该模型 `model_access` 里无 rate/rate_windows），
  按②的费率算只有 17.259（0.059×）；② 09-29 CST 07:00 小时桶 52 次 / input 105415 +
  output 25986 + cache 3346432 → 31.404 积分 = 31403（1.0000x），按①的费率算是
  951.832（**30.3×**）——即平台当天换了口径。切换时刻只能定位到流量空档
  **09-28 20:32 CST → 09-29 07:27 CST** 之间（窗口内无任何请求），故 cutoff 取
  09-29 00:00 CST。**改这段前先跑 `./scripts/verify-dim-billing.py`**：它按
  `interval=daily|hourly` 拿账本逐桶核对，**当天即可验证**（不必等隔日结算），也能顺带
  发现 dim 源漏记录（账本次数 > 本地记录数）。复核要点：cookie 用
  `~/.config/token-stats/deploy-env.sh` 的 `DIMAGENT_SESSION_COOKIE`（shell 里那份可能已过期），
  且平台 `prompt_tokens` **含 cache**，对应我们已减去 cache 的 `input_tokens`。
  无价格模型的记录显示 N/A
  （`display_cost` 返回 -1，聚合时直接被剔除——**平台新增模型后忘登记就会这样静默漏计**）。
- **Dim 折扣走 entitlement rate（`src/dim_entitlement.rs`）**：平台实收 = **目录原价 ×
  账号 entitlement 的 `rate`**，所以 `[[dim_model]]` 只登记**原价**——折扣不再写死
  （glm-5.3 曾因而已从 840/2940/182 的 7 折价改回原价，夜间再 5 折也改由窗口给出）。
  rate 取自 `dim usage --json` 的
  `subscription.current_term.entitlement_payload_json` → `model_access[].rate` /
  `rate_windows[]`（CLI 不可用时回落 `dimagent.cn/api/me/subscription`，与配额卡同一
  凭据链），由后台刷新循环每 `DIM_ENTITLEMENT_TTL_SECS`（默认 1800s）读一次。
  **payload 本身不带生效时间**，故每次观测都并入分段历史
  `~/.config/token-stats/dim-entitlement.json`：rate 变了才追加一段、生效时刻记为
  「首次观测到的时间」，因此折扣只影响其后的记录、历史不会被追溯改写（早于任何分段的
  记录按 1.0 原价）。已知首个切换点（v4.1-flash 0.65，2026-09-18 18:00 CST）是**对账
  daily-stats 实测后手工播种**进该文件的——新机器首次观测只能记为「当时」。
  `rate_windows`（如 glm-5.3 的 20:00–08:00 CST ×0.5 夜间优惠）与 base rate **相乘**，
  重叠时取最低；学到的分段可在 `GET /api/pricing` 的 `dim_entitlement_rates` 里看。
  **校验口径**：`/api/user/daily-stats` 同时返回 `base_amount_minor`（原价）与
  `final_amount_minor`（实扣，即 `quota_consumed`），毫积分（÷1000）——整天比对
  `仪表盘成本 ÷ (final_amount_minor/1000 × 70/11000)` 应恒为 1.0000（2026-09-15~19
  实测一致，含 09-18 的半天切换）。
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
- **StepFun Step Plan**（`source='stepfun-proxy'` 或 dim 通道 `stepfun`/`step-plan`）：
  套餐 **¥99 购买 ¥1600 的 API 额度** ⇒ `stepfun_plan_divisor = 1600/99 ≈ 16.1616`，
  实际成本 = 列表价 ÷ divisor。`pricing.toml` 记**列表价 CNY/1M**：`step-5-preview`
  input=7.0 / output=20.0 / cache_read=0.35 / cache_write=0.0。分支选中的是
  **provider**（`pricing.rs::is_stepfun_plan_billed`），不是模型价表——`[[model]]`
  条目只负责价格，将来 CPA 加 `step-3.7-flash` 之类时**忘记加分支也不会漏掉套餐价**。
  dim 侧的 StepFun 通道（控制台把价目 key 按渠道 slug 成 `step-plan`，库里
  `cost` 恒 0 + `quality:"missing_price"`）同样按列表价 ÷ divisor 计费，与代理记录口径
  一致；**`step-plan-intl`（`api.stepfun.ai`，USD 定价的海外端点）不适用国内套餐额度，
  故意排除**，落到不折算的通用路径。
  默认 divisor 也写在代码里（`default_stepfun_plan_divisor`），改 TOML 不是前提。
- **ZCode / BigModel GLM Coding Plan**（source=`zcode`、provider=`bigmodel`）：按**官方
  列表价**计费，不走模型价目表。官方积分公式（docs.bigmodel.cn 套餐概览）：
  `积分 = (输入×2.3 + 缓存命中×0.56 + 输出×8) / 10000`（GLM-5.3-Flash；GLM-5.3 系数
  6.9/1.7/24，配 `zcode_list_rates`）——积分系数即官方列表价（元/M tokens）。高峰
  （周一至周五 14:00–18:00 CST）按 1×，其余时段按 50% 抵扣（`zcode_off_peak_factor`）；
  夜间畅用活动期内（2026-09-03~10-07，官方公告把结束日从 09-20 延到 10-07 并写成
  9/3~10/7 连续；每日 23:00–09:00 CST）ZCode 端消耗为 0（`zcode_night_free_*`）。
  实测免扣在 **09-21 断档一天**（09-20 结束、09-22 恢复：该日 flash 请求 0 条），
  但因该日无记录、且公告口径为连续，**刻意只用单段 `from`/`until` 建模**，
  不必为这一天加空档窗口。
  活动**仅限 GLM-5.3-Flash**（`zcode_night_free_models`
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
- **Paseo glm-acp**（source=`glm-acp`、provider=`bigmodel`）：与 zcode **同一套官方
  积分公式、积分分摊单价（`zcode_credit_rates` / `zcode_cny_per_credit`）与同一套
  时间因子**——高峰 1.0×、非高峰 0.5×（实测 ACP 通道同样享受）。唯一差别：夜间畅用
  窗口内**不归零**，按 `GLM_ACP_NIGHT_CREDIT_FACTOR = 0.25×` 扣（= 非高峰 0.5 的再
  减半；zcode 通道专属的是"归零"）。窗口的日期边界、每日时段与模型白名单
  （仅 GLM-5.3-Flash；GLM-5.3 夜间照常按高峰/波谷因子）与 zcode 完全共用，见
  `compute_glm_acp_credit_cost`。`glm_acp_shares_zcode_time_factors_but_nights_are_not_free`
  测试钉死：同一时刻 flash 夜间 zcode=0 / glm-acp=0.25×，非高峰两边同为 0.5×，
  高峰两边同为 1.0×。积分单价未配置时兜底落到 `[[model]]` 的 GLM 列表价条目。
  模型名在 glm-acp 源解析时归一为官方大小写（`GLM-5.3-Flash`），与 zcode 行同组显示。
- **成本展示规则**（`display_cost()`）：`original_provider` 决定公式分支；无任何可用价格
  的非 pi 来源显示 "N/A"；pi 记录沿用其存储 cost（DeepSeek 为 CNY 原样，
  其余 USD 折算）。
