# Token Stats Dashboard

智能体 token 用量监控仪表盘。聚合 **pi、Claude Code、Codex、OpenCode、Kimi CLI、
Kimi Code、Qoder、Grok CLI、Command Code、ZCode、DSH、Dim** 等多个 AI 工具/提供商的
token 用量，提供图表、表格、筛选与配额/订阅卡片的一站式视图。

**技术栈：** Rust（Axum）后端 + React 19 + Tailwind CSS v4 + Recharts 前端，
部署于 nginx 反向代理，路径 `/token-stats/`。

更多面向代理的工程细节见 [AGENTS.md](AGENTS.md)。

## 功能

- 📊 **实时用量分析**：按供应商、日期、模型、数据源聚合；支持 1h / 4h / 日 分辨率
- 📈 **交互式图表**：每日趋势（堆叠柱 + 缓存命中率线，指标可勾选）、供应商分布、TPS 图
- 💰 **成本追踪**：全站统一按 CNY（¥）展示；模型级定价 + 订阅折扣 + 分段汇率实时换算
- 🎯 **缓存命中率**：加权与单请求计算，跨来源归一化（OpenAI ↔ Anthropic 语义差异）
- 🔍 **请求明细表**：分页、多列筛选、透视汇总、列宽可拖拽
- 🛰️ **配额/订阅卡片**：Kimi、OpenCode Go、Xiaomi MiMo、Command Code、Ollama、Meituan、
  Fenno、Grok 等余额/额度与重置倒计时，临近到期告警
- 🗄️ **持久化历史**：专用 SQLite 存储，清理原始会话文件后历史不丢失
- 🎨 **中文界面**，固定侧边栏 + 置顶导航栏，2880×1800 单用户快查优化

## 架构

```
浏览器 → nginx:80 → Rust Axum API (:3000) + 静态文件
                     ↑
        读取多个数据源（JSONL / SQLite / zstd）+ HTTP 配额接口
```

- 后端启动时全量读取各数据源，之后每 30s 增量刷新；全部记录写入专用 SQLite
  （`~/.config/token-stats/token-stats.db`）持久化，内存快照即时对外服务。
- 内置 loopback Grok 用量代理（`--grok-proxy-only`，systemd 服务
  `token-stats-grok-proxy.service`），记录 Grok CLI 请求用量但不暴露请求内容。
- 蓝绿部署：`deploy.sh` 在 3000 ↔ 3001 端口间切换，nginx upstream 原子切换，零停机。

## 快速开始

### 先决条件

- Rust（2024 edition toolchain）
- Node.js 18+
- nginx（本地）

### 自动安装

```bash
./setup.sh
```

脚本会：构建后端、构建前端、安装 nginx 配置、安装 systemd 服务。随后：

```bash
sudo systemctl start token-stats@3000
sudo nginx -s reload
```

访问 **http://localhost/token-stats/**

### 手动构建

```bash
# 1. 后端
cd backend && cargo build --release

# 2. 前端（产物输出到 ../backend/static）
cd frontend && npm install && npm run build

# 3. 运行
cd backend && ./target/release/token-stats-backend
# http://localhost:3000
```

### 快速开发（仅后端，前端用已构建产物）

```bash
./start.sh
```

## 数据源

| 数据源 | 位置 |
|--------|------|
| pi | `~/.pi/token-logs/usage.jsonl` + Taskplane runtime 退出文件 |
| Codex | `~/.codex/sessions/*/rollout-*.jsonl` |
| Claude Code | `~/.claude/projects/*/*.jsonl` |
| OpenCode | `~/.local/share/opencode/opencode.db` |
| Kimi CLI | `~/.kimi/sessions/*/wire.jsonl` |
| Kimi Code | `~/.kimi-code*/sessions/*/*/agents/*/wire.jsonl` |
| Qoder / Qoder CN | `~/.qoder/projects/*/*.jsonl` / `~/.qoder-cn/logs/sessions/` |
| Grok CLI | `~/.token-stats/grok-usage.jsonl`（代理写入） |
| Command Code | `~/.commandcode/projects/<slug>/<session-id>.jsonl` |
| ZCode | `~/.zcode/cli/db/db.sqlite` |
| DSH | `~/.dsh/sessions/*/session-*/session.jsonl.zstd` |
| Dim | `~/.dimcode/v2/dimcode.sqlite` |
| ccswitch（可选） | `~/.cc-switch/cc-switch.db`（设置 `USE_CC_SWITCH` 后加载） |

配额/订阅数据（Kimi、OpenCode Go、Fenno、Grok、Xiaomi MiMo、Command Code、Ollama、
Meituan）通过各自官方接口抓取，需要相应环境变量凭据，未配置时对应卡片显示不可用。

## 主要 API

| 端点 | 说明 |
|------|------|
| `GET /api/stats?from=&to=&source=&provider=&model=&tz_offset=&resolution=` | 完整聚合 |
| `GET /api/requests?from=&to=&provider=&model=&source=&page=&limit=&tz_offset=` | 分页原始请求 |
| `GET /api/quota` | 全部配额/订阅卡 |
| `GET /api/rpm` / `GET /api/tps` | 每分钟请求 / 每秒 token 分析 |
| `GET /api/filters` | 可用供应商、模型、数据源 |
| `GET /api/pricing` / `POST /api/pricing/reload` | 定价配置与热加载 |
| `GET /api/store/info` / `POST /api/store/restore` | 持久化状态 / 整库恢复 |
| `POST /api/restore` / `GET /api/export` | JSONL 备份恢复 / 导出 |

## 项目结构

```
token-stats/
├── backend/               # Rust Axum 后端
│   ├── src/
│   │   ├── main.rs        # CLI 入口（--grok-proxy-only）
│   │   ├── app.rs         # 路由构建、后台刷新/flush、优雅退出
│   │   ├── models.rs      # 数据结构
│   │   ├── sources/       # 各数据源解析器
│   │   ├── aggregator.rs  # 过滤/聚合/RPM/TPS/分页
│   │   ├── routes.rs      # API 处理器
│   │   ├── pricing.rs     # 成本计算（模型价、汇率、折扣）
│   │   ├── store.rs       # SQLite 持久化
│   │   ├── quota/         # 配额/订阅抓取
│   │   ├── grok_proxy.rs  # Grok loopback 代理
│   │   └── ...
│   ├── pricing.toml       # 定价配置（热重载）
│   ├── vendor_merge.toml  # 供应商合并规则
│   └── static/            # 前端构建产物（勿手建）
├── frontend/              # React + Vite 前端
│   └── src/               # App.tsx + components/ + lib/ + sections/
├── nginx/                 # nginx 配置、systemd 模板、Grok 代理服务
├── scripts/               # 部署/刷新/汇率更新/数据修复脚本
├── docs/                  # 部署与设计文档
├── setup.sh               # 自动安装
├── deploy.sh              # 蓝绿零停机部署
└── start.sh               # 快速启动（后端）
```

## 成本模型要点

- 所有成本统一展示为 **CNY**；后端 `cost` 字段保留原始币种，前端按需换算。
- `pricing.toml` 集中管理：模型单价（USD/1M，DeepSeek 为 CNY 直报价）、
  USD→CNY **分段汇率**（每 2 周更新）、各订阅折扣（OpenCode /6、Command Code /10、
  FreeModel 面值 0.1、Kimi API 原价 ÷ 倍率、Fenno / Grok / Ainaba 平台等）。
- 修改定价：编辑 `backend/pricing.toml` → `./scripts/reload-pricing.sh`（或
  `POST /api/pricing/reload`），无需重启。
- 汇率更新：`./scripts/update-exchange-rate.sh`（建议 cron 每两周一次，14 天幂等节流）。

## 许可证

MIT
