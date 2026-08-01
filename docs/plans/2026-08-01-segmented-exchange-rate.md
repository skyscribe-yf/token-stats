# 分段汇率（Biweekly Rate Segments）设计

日期：2026-08-01
状态：已实现（2026-08-01）

## 背景与目标

**现状：** `pricing.toml` 只有单一 `usd_to_cny`，所有历史记录统一按当前汇率折算。汇率更新后，旧记录的成本也随之整体漂移。

**目标：**

- 每 2 周记录一次新汇率，形成时间分段；
- 每条记录按自身时间**所在分段的汇率**计算成本；
- 订阅类折扣（Fenno / FreeModel / Grok / Ollama）在分段后仍保持真实 CNY 成本（汇率不变式不破）；
- 向后兼容：无分段配置时行为与现在完全一致，历史数据无需迁移。

---

## 一、配置格式（`pricing.toml`）

```toml
# 当前汇率（人读字段 + 兼容层；配额卡等“现在”场景使用）
usd_to_cny = 6.7894
rate_date = "2026-07-31"

# 分段汇率：按记录时间选择“最近生效”的一段（语义与模型价格 effective_from 一致）
[[usd_to_cny_segments]]
rate = 6.82          # 兜底段：早于所有分段的记录用它（原单一汇率，保持旧记录成本不变）

[[usd_to_cny_segments]]
effective_from = "2026-07-31"
rate = 6.7894
```

**规则：**

- `effective_from` 支持 `YYYY-MM-DD`（UTC+8 当日 00:00 生效）或 RFC3339（精确到秒，如 `2026-07-31T14:00:00+08:00`）。
- 省略 `effective_from` 的段为**兜底段**，覆盖早于第一段的所有记录。
- 选段：取 `effective_from <= 记录时间` 中最近的一段；都不满足则用兜底段；无兜底段则回退 `usd_to_cny`。
- 完全没有任何 `[[usd_to_cny_segment]]` 时，行为与现在完全一致（仅 `usd_to_cny` 生效）。

---

## 二、Rust 数据模型

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsdToCnySegment {
    /// None = 兜底段（最早记录）。"YYYY-MM-DD"（UTC+8）或 RFC3339。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effective_from: Option<String>,
    pub rate: f64,
}
```

`PricingConfig` 新增（`#[serde(default)]`，保证旧配置可解析）：

```rust
pub usd_to_cny_segments: Vec<UsdToCnySegment>,
```

`PricingState::new()/reload()` 时构建一次 `RateSchedule`：

```rust
struct RateSegment {
    effective_from: Option<DateTime<FixedOffset>>, // None = 兜底
    rate: f64,
    // 由该段汇率派生的除数快照，保持订阅类不变式：
    freemodel_divisor: f64, // rate / 0.1
    fenno_divisor: f64,     // 150 * rate / 10
    grok_divisor: f64,      // 1950 * rate / 50（用户未覆盖时）
    ollama_per_token: f64,  // 20 * rate / (weekly_quota * 52/12)
}

struct RateSchedule {
    segments: Vec<RateSegment>, // 排序：兜底(None) 在前，其余按 effective_from 升序
    current_rate: f64,          // 无段时 = usd_to_cny；有段时 = 最后一段.rate
}

impl RateSchedule {
    fn rate_for(&self, record_time: &str) -> f64;
    fn segment_for(&self, record_time: &str) -> &RateSegment; // 含派生除数
}
```

选段逻辑直接复用模型价格 `select_segment` 的语义（排序、None 兜底、最近生效者胜出），建议抽成通用小工具避免两份实现漂移。

---

## 三、`display_cost` 改造（核心）

把 `cfg.usd_to_cny` 按场景区分：

1. **历史记录成本路径**（opencode-go、commandcode、普通 pi USD 提供商、codex/claude-code/kimi 派生价）：
   `rate = schedule.rate_for(&record.time)`，`cny = usd * rate`。

   **例外：Ainaba（AI奶爸/Yairouter）**。该平台按固定结算汇率 7.0 计费（充值 396 元
   → 8000 元额度，倍率 20.20202 自 2025-05-25 起生效，之前为 40 倍促销段），平台汇率
   不随时间变化，因此 ainaiba 记录使用 `special.ainaba_platform_rate = 7.0` 常量，
   不走市场分段汇率；倍率仍按 `ainaba_segments` 时间分段。
2. **订阅类折扣使用同一分段的派生除数**，保证不变式：

   | 提供商 | 公式 | 分段后结果 |
   |---|---|---|
   | FreeModel | `usd * rate / freemodel_divisor` | 恒等于 `usd * 0.1` |
   | Fenno | `usd * rate / fenno_divisor` | 恒等于 `usd * 10/150` |
   | Grok | `usd * rate / grok_divisor` | 恒等于 `usd * 50/1950` |
   | Ollama | 按段重算 per-token CNY | 恒等于订阅真实成本 |

   用户通过 settings 显式覆盖的固定 `grok_divisor` 除外：覆盖后对所有记录使用该固定值（尊重用户手工校准）。
3. **配额卡等“当前状态”场景**（`quota/grok.rs` 等）：继续用 `current_rate`（最新一段），语义是“现在”，不随记录时间变化。

---

## 四、DeepSeek 派生模型价（已定：CNY 定价）

DeepSeek 官方按人民币定价，`pricing.toml` 直接存 CNY 值（`input_cny` / `output_cny` /
`cache_read_cny` / `cache_write_cny`），运行时**直接产出 CNY、不经过汇率换算**。
OpenAI / Anthropic / CommandCode 等原生 USD 定价模型不受影响（它们 × `rate_seg` 即正确）。

---

## 五、每 2 周获取新汇率（工作流）

### 方案 A（推荐）：脚本 + cron

已新增 `scripts/update-exchange-rate.sh`：

1. 拉取最新 USD/CNY：
   - 主源：`https://open.er-api.com/v6/latest/USD`（已验证可用，2026-08-01 返回 `CNY = 6.764897`）；
   - 备选：央行中间价等其它来源用 `--rate X --date Y` 手动补录。
2. 读取 `pricing.toml`，若距最后一段不足 14 天则跳过（幂等，防重复追加）。
3. 追加 `[[usd_to_cny_segments]]`（`effective_from = 今天`），同步更新 `usd_to_cny` / `rate_date`。
4. 调用 `POST /api/pricing/reload` 热生效（不重启）。
5. 支持 `--date 2026-08-01 --rate 6.7894` 手动补录历史段。

cron 示例（每两周一次，隔周周一 09:00）：

```cron
0 9 * * 1 [ $(( ($(date +\%s) / 86400 / 14) % 2 )) -eq 0 ] && /path/scripts/update-exchange-rate.sh >> /path/logs/rate.log 2>&1
```

或更直观：每月 1 日与 15 日各执行一次。

### 方案 B（可选增强）：后端自动拉取

在现有后台任务旁加一个低频任务：每 6h 检查最新段距今是否 ≥14 天，是则用 reqwest 拉取 → **原子写** `pricing.toml` → reload。零维护，但服务进程会写仓库配置文件、依赖外网，需要失败退避与日志。

**推荐 A**：可审查、可回滚，不引入后端联网写文件；B 作为后续可选增强。

---

## 六、API 与前端

- `GET /api/pricing`：新增 `usd_to_cny_segments: [{ effective_from, rate }]`；`usd_to_cny` 仍表示当前汇率（= 最新段，由后端归一化保证一致）。
- `frontend/src/api.ts`：扩展 `PricingConfig` 类型。
- `SettingsDrawer` 的“计价逻辑”区：增加“历史分段汇率”折叠列表（每段：生效时间 → 汇率），当前段高亮。

---

## 七、边界与兼容

- **旧配置**（无 segments）：行为不变，保留全部现有回归测试。
- **已入库记录**：`cost` 保持原始币种，display 实时换算 → 分段汇率自动作用于全部历史，**无需迁移数据库**（与 vendor_merge 不同，改汇率不影响已持久化数据）。
- `record.time` 无法解析：回退 `current_rate` 并 `warn`。
- `effective_from` 非法：跳过该段并 `warn`（不 panic）。
- `effective_from` 重复：后写者覆盖（与模型段一致）。
- 兜底段缺失且记录早于第一段：用 `usd_to_cny`（文档提示：加兜底段可精确还原最早历史）。

---

## 八、测试计划

1. 无分段回归：所有记录用 `usd_to_cny`，现有 211 个测试全部保留。
2. 分段选择：兜底 / 7-31 前后分别命中正确汇率；边界当天 UTC+8 00:00 生效。
3. 订阅类跨段不变式：fenno / freemodel / grok 在 6.82 与 6.7894 两段成本相等（按不变式预期值断言）。
4. 无兜底段 + 超早记录 → 回退当前汇率。
5. 无效时间 / 无效 `effective_from` → 不 panic，回退并 warn。
6. `/api/pricing` 返回 segments；前端类型检查通过。
7. 脚本幂等：14 天内重复执行不追加；`--date/--rate` 补录正确；追加后 reload 生效。

---

## 九、改动文件清单

| 文件 | 改动 |
|---|---|
| `backend/pricing.toml` | 加入分段（兜底 6.82 / 7-31=6.7894）；DeepSeek 改 CNY 定价；ainaba 平台汇率 7.0 + 倍率 20.20202 |
| `backend/src/pricing.rs` | `UsdToCnySegment`、`RateSchedule`、`rate_for()`、`display_cost` 改造、测试 |
| `backend/src/quota/grok.rs` | 配额卡改用 `current_rate`（注明语义） |
| `backend/src/routes.rs` | 无需改动（`get_config()` 直接序列化 segments） |
| `frontend/src/api.ts` | `PricingConfig` 类型扩展 |
| `frontend/src/components/SettingsDrawer.tsx` | 分段汇率展示 |
| `scripts/update-exchange-rate.sh` | 新增：拉取 + 追加 + reload |
| 文档 | 本设计 + cron 示例 |
