# 陷阱与注意事项

> 从 [`AGENTS.md`](../../AGENTS.md) 拆出。编号沿用原文件（历史上跳过了 20，保持不变以免
> 与其他文档/提交信息里的编号引用失配）。AGENTS.md 里只保留「每次改代码都可能踩到」的
> 少数几条，其余在此。

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
    重启实例；`--dry-run` 只提取不改 systemd）。**部署会自动做这件事**：
    deploy.sh 在 source 凭据文件**之前**跑 `refresh-codebuddy-cookies.sh --env-only`
    （只刷新 `deploy-env.sh`，注入由 deploy.sh 对新实例的 drop-in 完成），
    Chrome/keyring 不可用时只告警并沿用旧值，`SKIP_CODEBUDDY_COOKIE_REFRESH=1`
    可整体跳过；注入前还会用 `warn_if_codebuddy_cookies_dead()` 实探一次接口
    （非 200 只告警不中止），把「悄悄注入死 cookie」变成部署时就可见。
    **坑（2026-09-21 查明真因）**：env 文件是新的不代表**注入的**是新的。
    deploy.sh 故意让「调用 shell 里已导出的值」覆盖凭据文件（为了支持一次性
    override），而 `~/.bash_env` 里正好导出着两个 CODEBUDDY cookie——旧的
    `1788591467`（2026-09-05 到期）——于是**刚提取的新 cookie 当场被旧值覆盖**。
    2026-09-20 的部署就是这样：`deploy-env.sh` 已是有效 cookie（实测直连 200），
    而 `token-stats@3001` 的 drop-in 拿到的是 `~/.bash_env` 里那份 9 月 5 日就
    过期的——刷新白做，卡片照旧 401。修复：refresh 成功后
    `unset CODEBUDDY_SESSION_COOKIE CODEBUDDY_SESSION_COOKIE_2`，让文件的新值生效
    （只有这两个变量；它们每次部署都从浏览器重取，shell 里的副本必然更旧）。
    StepFun 同款：`scripts/refresh-stepfun-token.sh` 在 source 前轮换
    `STEPFUN_OASIS_TOKEN`（access JWT 约 30 分钟，不是 cookie 上的一年），
    成功后同样 `unset STEPFUN_OASIS_TOKEN STEPFUN_OASIS_WEBID`。漏掉 unset
    会把 shell 里过期的 Oasis-Token 注回新实例，月池半区又变成 `plan=null`。
    自查手法：`bash -c` 里先 export 一份旧 cookie、再按 deploy.sh 的顺序 source
    凭据文件，看最终留下的是哪份；或直接比对 drop-in 与 `deploy-env.sh` 里的
    过期时间戳（`grep -o 'CODEBUDDY_SESSION_COOKIE=[^|]*|[0-9]*'`）。
    注意 `session`/`session_2` 是 Flask 签名会话，
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
21. **Dim 价目卡的 `week_days` 不参与计费，别照着它设 `peak_weekdays_only`**
    （2026-09-20 对账）——`/api/public/website/plans` 的
    `time_periods[].week_days = [1,2,3,4,5]` 写着「仅周一至周五」，但 daily-stats 的
    `base_amount_minor` 证明**周末同样按 2× 忙时扣**：2026-08-23（日）、09-12（六）、
    09-13（日）三天先把 `source='dim'` 的 token 合计与平台逐日总量对齐（误差 <0.01%），
    再比对价——只有「周末也翻倍」才闭合（差 -0.00%~-0.15%），按平日价则差 -29%~-32%。
    给 `[[dim_model]]` 高峰条目加 `peak_weekdays_only = true` 会把周末忙时用量少算一半。
    另注意：**当天的日账滞后于** `/api/log/self`（09-20 实测 store 比 daily-stats 多
    29% 的 prompt token），所以只有**完整的一天**能用于对账，别拿今天的数据调参。
22. **DimAgent cookie 一死，OAuth 通道的流量两个源都没有**（2026-09-22 排障）——
    `DIMAGENT_SESSION_COOKIE` 约 30 天（自浏览器最后登录）过期，且 401 走的是
    优雅降级路径（陷阱 4），日志里不留错误，所以症状是「某类流量悄悄消失」而不是报错。
    危害比普通配额卡 401 大：Dim 自有 OAuth 通道 `dimcode-api-oauth` **只由 console API
    逐请求计量**，本地 `usage_run_stats` 补充又按设计排除该 provider（防双计），于是
    cookie 失效期间该通道的每一次调用（实测 `deepseek-v4.1-flash`）既不在 API 也不在
    补充里 —— **历史还在，新数据全丢，且没有任何一侧报错**。
    排查口径：`/api/requests?source=dim` 看最新时间戳是否停滞，而
    `~/.dimcode/v2/dimcode.sqlite` 的 `usage_run_stats` 里同期有
    `providerId='dimcode-api-oauth'` 行 → 即 cookie 死了（`curl -H "Cookie: session=$CK"
    https://dimagent.cn/api/log/self?p=1&page_size=1&type=2` 返回 401 确认）。
    修复：`./scripts/refresh-dimagent-cookie.sh`（从 Chrome 提取 → **先实测 200 再写回**
    → deploy-env.sh → systemd drop-in → 重启），已挂在 `deploy.sh` 的 `0a-pre3`，
    两条部署入口都会自动刷新（`SKIP_DIMAGENT_COOKIE_REFRESH=1` 跳过）。
    回填安全：watermark `dim_console_last_id` 存在 store 的 `sync_watermarks` 表，
    重启后按 watermark 往回翻页补齐缺口（实测停滞 25h ≈ 13 页，远低于
    `MAX_PAGES=400`），指纹去重保证不双计。**不要**用 `sudo` 整体跑刷新脚本 ——
    Chrome 解密要走用户自己的 D-Bus/GNOME Keyring，提权后取不到密钥，让脚本内部的
    sudo 自己提示即可。
23. **OpenCode 2.x 换了表，读 `message` 的代码会静默停更**（2026-09-27 排障）——
    OpenCode 2.x 把逐条消息从 `message`（配 `session`）搬到 **`session_message`**
    （配 `session_v2`），旧表**保留但不再写入**，于是症状是「opencode 数据停在升级那天，
    不报错」，与陷阱 22 同类。2.x 的 `data` JSON 也同时改形：role 变成行上的 **`type` 列**
    （不在 JSON 里了）、模型藏在 **`data.model.{providerID,id}`**（不再是顶层
    `modelID`/`providerID`）、**没有 `tokens.total`**（要自己加），而且**不再记录
    per-message `cost`**（实测 2.x 原生行 cost 恒为 0）。所以只把 SQL 换成
    `session_message` 仍然全废：`role != "assistant"` 会把每一行判掉，model 变 unknown，
    total 变 0。
    双计口径：2.x 迁移会把 1.x 历史**复制**进 `session_message`（实测 194 行里 184 行
    两边同 id、token 一致但 JSON 文本不同），所以 `message` 只能取
    `WHERE id NOT IN (SELECT id FROM session_message)` 的残行（实测正是升级前最后一个
    session 的 10 行）；整表都读会靠指纹去重兜底，但别指望它。
    计费口径：**2.x 行 cost=0 会掉进 `display_cost()` 最后的 `-1`（N/A）分支**——
    `opencode` 原本不在「按 token 计算」那批 source 里，必须加进去；同时给免费模型
    （`space-bunny-free` 等）显式登记 0 价，否则显示 N/A 而不是 ¥0。
    见 `sources/opencode.rs` 顶部对照表与 `migrate_opencode_reasoning_output`。
24. **后端常驻内存靠 mimalloc + `CompactString` + `MALLOC_ARENA_MAX` 三样撑着，动其中任一样都会静默涨回去**（2026-09-28 优化）——
    症状不是泄漏：`VmRSS == VmHWM`，跨 30s 刷新周期完全平稳，但数值只有实际数据的 3.7 倍。
    旧实测 RSS **1135 MB**，而 762k 条记录真实需要 ~300 MB（`TokenRecord` 248 B × 762k =
    180 MB 缓冲 + 全表字符串实测 53.8 MB）。三个成因，缺一不可地叠在一起：
    ① glibc 按线程建 arena，实测 **44 个 64MB 对齐的 arena 堆共 655 MB**（nproc=16 →
    上限 8×16）；glibc 只能 trim **主** arena 顶部，非主 arena 必须整块全空才 `munmap`，
    而启动期的记录散落在各 arena 里，每个都剩几个活对象 → 谁都还不掉；
    ② 8 个 `String` 字段 = 每条记录 7 次独立 malloc（全表 ~5.3M 次），glibc 最小 chunk 32 B，
    12 字节的有效数据吃 32 字节，还让 `fingerprint()`/`sort_by` 每次比较都追指针；
    ③ 启动时 `store.load_all()` 的 762k Vec 与 `load_all_sources()` 的**全量重解析**同时存活，
    `load_sources_impl` 的 `extend` 又向 180 MB 逐级 doubling，每步 realloc 都弃置一份前缀副本。
    对策（三处都要留着）：`main.rs` 的 `#[global_allocator] mimalloc::MiMalloc`；
    `nginx/token-stats*.service` 的 `Environment="MALLOC_ARENA_MAX=2"`（管的是走 libc 的
    bundled SQLite 等，mimalloc 接管不了）；`TokenRecord` 的
    `date`/`api_key_prefix`/`provider`/`model`/`source` 用 `CompactString`（≤22 字节内联、零分配），
    只有 `time`（RFC3339 ≥24 字节）留 `String`。另外 `load_sources_impl` 先分源装入各自
    Vec 再按总数 `with_capacity` 一次摊平；`AppState::new` 末尾用 `retain` **原地**过滤，
    不要退回 `into_iter().filter().collect()`（那会在 `db_records` 还活着时再要一个 180 MB 缓冲）。
    实测效果：峰值 1135 → **671 MB**，稳态 1109 → **300 MB**，且临时分配消退后 RSS 会回落
    （mimalloc 会 decommit 整段，505 MB 用后自动掉回 300 MB）。
    改 `TokenRecord` 字段类型时 `store.rs` 有两处必须跟着改：`insert_batch` 的 `params![...]`
    要 `.as_str()`（`CompactString` 没实现 `ToSql`），`row_to_record` 走 `text_col()`
    （`ValueRef` 直读；用 `row.get::<String>()` 会在启动时白建 ~5M 个中间 String）。
    复测口径：`PORT=3999 TOKEN_STATS_DB_PATH=<库副本> ./target/release/token-stats-backend`，
    采样 `/proc/PID/status` 的 `VmRSS`/`VmHWM`；正确性用两个实例对比
    `/api/store/info` 的 `memory_records` 与 `/api/stats` 的 `by_vendor`/`by_model`/`by_source`
    （差异应只剩最新那一条实时记录）。
25. **Dim 对账别拿 `request_count` 当"账本计了前 N 条"**（2026-10-01 排障）——
    `verify-dim-billing.py` 的前缀匹配用 `by_key[key][:counted]`（`counted =
    request_count`），但账本的 `request_count` 把**尚未落账的在途请求**也算进去了，
    于是"前 N 条"里混进本地有、账本 token 还没计的记录。2026-10-01 14:00 CST 桶
    账本 316 次 / 本地 323 条，前缀法直接报 1.7845×；按**token 口径**找精确吻合的
    前缀（prompt 含 cache = 我们已减过的 `input_tokens` + `cache_read_tokens`）
    得到的却是前 **311** 条，用它对账 base/final 双双闭合到 0.99999。
    **调参（cutoff、费率、忙时窗口）一律以 token 口径的前缀为准**，别信 request_count。
    同一个桶还暴露了 `peak_hours_utc` 会因**平台全站活动**整体失效：2026 国庆平台把
    整个目录按闲时计费（见 `pricing.md` 的 `dim_offpeak_windows`），所以
    "某个桶不翻倍"**不能**直接推断 `peak_hours_utc` 配错了——09-30 11:00 桶是
    11:21 之前按 2×、之后按 1×（切换点在桶中间，逐条解出来的），而 09-20 同时段
    确实按忙时收。判据是**同一时段跨日期对比**，不是单桶。

26. **CPA 通道的客户端侧副本必须整条丢弃，不是过滤**（2026-10-07 排障）——
    CPA 的 usage 插件是唯一逐请求计量点，但 `pi` / `zcode` / `codex` 这些客户端源会把同一次
    调用**再记一遍**：`pi` 的 `provider='cpa'`（`wb/hy4-preview`、`step/step-5-preview`、
    `ollama/*`），`zcode` 的 model 带 `ollama/` 前缀（provider 存的是真实通道名
    `opencode-go`，所以前缀是唯一信号），`codex` 的 `turn_context` 指向 CPA 模型。
    症状有两条，见到任意一条就往这查：**cost 恒为 0**（前缀名匹配不到 `pricing.toml` 任何
    `[[model]]`），以及与插件记录**相隔几秒成对出现**（客户端记请求开始、插件记请求完成，
    实测 `pi`/`dim-agent` 0s、`zcode`/`ollama-proxy` -2s ~ -20s）。
    修法是解析期 `continue`（`sources/mod.rs` 的 `meters_cpa_channel()`，三个源各自调用并
    各自计数打 info 日志），**不能**只在前端/聚合层过滤——fingerprint 不同、UI 过滤遮不住
    双计的 token 与成本。已落库的历史行由
    `TokenStore::purge_cpa_metered_client_rows()` 在每次 store open 时幂等清除
    （`CLIENT_SOURCES = ["pi","zcode","codex"]`，**只删客户端源**：插件自己就是权威计量点
    且天生带前缀，`cc-proxy` 的 `deepseek/deepseek-v4-flash` 必须保留）。
    2026-10-07 实删 420 行；注意**加新客户端源时必须同步这张表**，否则它的 CPA 行永远
    清不掉。
