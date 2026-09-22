# 文档目录说明（docs/）

本目录保存项目的**代理参考文档**（`agents/`）、部署文档与历史设计/计划档案。
`agents/` 与 `ecs-deployment.md` 是当前有效、随代码维护的；其余均为历史记录，不再维护，
仅作追溯用途，内容不代表当前代码状态。

## 当前有效

### `agents/` —— AGENTS.md 拆出的参考文档

根目录 [AGENTS.md](../AGENTS.md) 是**索引 + 不变式**（每次都会注入代理上下文），
篇幅大的逐项参考放在这里（按需阅读）。何时必读哪一份，以 AGENTS.md 开头的
「参考文档路由」表为准。

| 文件 | 内容 |
|------|------|
| `agents/data-sources.md` | 19 个数据源的完整行为说明、4 个 loopback 代理/CLIProxyAPI 插件架构、CPA 模型命名空间与 dim 侧踩坑史 |
| `agents/pricing.md` | `pricing.toml` 全部计费分支、分段汇率/折扣、实测费率与对账口径 |
| `agents/quota-cards.md` | 每张配额卡的端点与认证细节、DimAgent console API 逆向结论 |
| `agents/environment-variables.md` | 全部环境变量的默认值与覆盖项 |
| `agents/pitfalls.md` | 编号陷阱 1–21（共 20 条；编号被提交信息与代码注释引用，不要重排） |

> 新增数据源、计费分支或陷阱时，请同时更新 AGENTS.md 里的简版索引与本目录的详版。

### 其他

| 文件 | 用途 |
|------|------|
| `ecs-deployment.md` | 本地仪表盘经 ECS 服务器 + SSH 反向隧道暴露到公网的部署说明（含 sslh、nginx、autossh 配置与排障） |

## 历史档案

| 目录 | 内容 | 状态 |
|------|------|------|
| `fix-plan-taskplane-token-collection.md` | Taskplane token 采集缺口调查（2026-05-19）；**后端修复已落地**（文件头部有状态更新说明），仅 token-tracker 扩展侧改动待确认 | 历史 |
| `plans/` | 分段汇率设计（2026-08-01，已实现）；Claude Opus provider 定价诊断（已修复） | 历史 |
| `superpowers/specs/` | 2026-05 ~ 2026-07 的功能设计文档（多源、筛选器、Kimi2 配额卡、Grok 用量记录/路由、Kimi 订阅成本等） | 历史，均已实现 |
| `superpowers/plans/` | 上述设计对应的实施计划（含未勾选 checkbox，仅表示当时进度） | 历史，均已实现 |
| `superpowers/completions/` | Smart Model Router 完成记录（2026-06-25；部署产物在 `~/.pi/agent/skills/smart-model-router`，仓库外） | 历史 |

> 历史档案的作用是保留"当时为什么这么做"的上下文。若要了解当前行为，
> 请直接看代码与根目录的 [AGENTS.md](../AGENTS.md)；功能细节以代码为准。
