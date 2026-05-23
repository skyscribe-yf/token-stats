# Diagnosis: Claude Opus Provider & Pricing

## 问题

2025-06-18 发现 Claude Opus 4.7 的 token 使用数据存在两个问题：

### 1. Provider 不准确

通过 cc-switch 切换到 FreeModel 的 Claude Opus 4.7 会话，在 dashboard 中显示 provider 为 `anthropic` 而非实际的 `FreeModel`。

**根因：** `claude_code.rs` 的 `parse_claude_code()` 函数使用 `resolve_provider_from_model()` 将 `claude-*` 模型硬编码映射到 `"anthropic"`。该函数不感知 cc-switch 的当前活跃 provider。

**影响：**
- Dashboard 将 FreeModel 会话误归为 Anthropic 原生
- 无法区分自建 Claude API 和 cc-switch 代理
- 对应的折扣定价无法正确应用

### 2. 定价缺失

FreeModel 的实际计费方式为 **1 USD 面值 = 0.1 CNY 实际成本**（即 6.82x 折扣因子），但 `display_cost()` 中没有对应的处理逻辑。

**影响：** Dashboard 中 FreeModel 的成本显示严重偏高（约 68x 虚高）。

---

## 已实施的修复

### 修复 1: Claude Code Provider 解析

**文件：** `backend/src/sources/claude_code.rs`

**方案：** 对 claude-* 模型，优先从 cc-switch DB 查询当前活跃 provider；如果查询失败（DB 不可用），回退到 `resolve_provider_from_model()`。

```rust
let provider = {
    if model.starts_with("claude-") || model == "sonnet" || model == "haiku" {
        super::CcSwitchSource::get_active_provider("claude")
            .unwrap_or_else(|| super::resolve_provider_from_model(&model))
    } else {
        super::resolve_provider_from_model(&model)
    }
};
```

**新增函数：** `CcSwitchSource::get_active_provider(app_type: &str) -> Option<String>`
- 查询 cc-switch DB 中 `is_current = 1` 的 provider name
- 只读模式打开 DB，不写入
- DB 不存在或查询失败时返回 `None`（优雅降级）

### 修复 2: FreeModel 定价

**文件：** `backend/src/pricing.rs`, `backend/pricing.toml`

**方案：** 新增 `freemodel_divisor = 68.2`，在 `display_cost()` 中对 `provider == "FreeModel"` 的记录应用除法。

```rust
// pricing.toml
freemodel_divisor = 68.2

// pricing.rs
if record.provider == "FreeModel" {
    cny /= cfg.special.freemodel_divisor;
}
```

**计算公式：**
- FreeModel: 1 USD face value = 0.1 CNY actual
- divisor = usd_to_cny / 0.1 = 6.82 / 0.1 = 68.2
- 对有 stored cost 的 Pi 记录：`cost_cny = cost_usd × 6.82 / 68.2`
- 对无 stored cost 的 claude-code 记录：先按 model 价格计算 USD cost，再 `/ 68.2`

### Vendor Merge 决策

**不**将 FreeModel 合并到 anthropic vendor group。原因：
1. `display_cost()` 依赖 `record.provider` 判断定价规则，合并后折扣逻辑失效
2. Dashboard 中区分 FreeModel 和原生 Anthropic 更有意义
3. `original_provider` 未在 vendor merge 中保存，合并不可逆

---

## 测试覆盖

新增 2 个测试：

| 测试 | 验证 |
|------|------|
| `freemodel_stored_cost_applies_divisor` | Pi 记录（有 stored cost）应用 68.2 除数 |
| `freemodel_derived_cost_applies_divisor` | claude-code 记录（无 stored cost，按 tokens 计算）应用 68.2 除数 |

全部 77 个测试通过。
