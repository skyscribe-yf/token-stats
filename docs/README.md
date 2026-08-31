# 文档目录说明（docs/）

本目录保存项目的部署文档与历史设计/计划档案。**当前有效的文档**只有
`ecs-deployment.md`（ECS 公网暴露）；其余均为历史记录，不再维护，
仅作追溯用途，内容不代表当前代码状态。

## 当前有效

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
