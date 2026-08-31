# Token Stats 前端

React 19 + TypeScript + Vite + Tailwind CSS v4 + Recharts 单页仪表盘。
构建产物输出到 `../backend/static`，由后端直接静态服务，部署路径 `/token-stats/`。

## 常用命令

```bash
npm install          # 安装依赖
npm run dev          # 开发服务器（Vite，默认 5173）
npm run build        # 产物输出到 ../backend/static
npm test             # 单元测试（node:test）
npx tsc --noEmit     # 类型检查
npm run lint         # ESLint
```

## 结构约定

- `src/App.tsx`：页面编排。持有全局筛选/开关状态（供应商、工具、模型、时间范围、
  分辨率、指标勾选）、section 切换（用量 / 订阅 / 请求）、配额轮询（`useVisibleInterval`
  30s，页面不可见时暂停）、顶部告警。重图表组件用 `lazy()` 分包加载。
- `src/components/`：`TopBar`（标题 + section 切换）、`Sidebar`（筛选面板）、
  `GlanceBand`、`KpiStrip`、`QuotaChips`、`SettingsDrawer`、`TpsChart`、
  `sections/UsageSection`、`sections/QuotasSection`、`sections/RequestsSection`。
- `src/lib/`：与 UI 无关的纯逻辑 —— `utils.ts`（格式化/日期/来源颜色与标签）、
  `timeRange.ts`（预设区间）、`filterState.ts`（筛选状态持久化）、`pivotTable.ts`
  （透视汇总）、`quotaCards.ts`、`fennoQuota.ts`、`subscriptionCycle.ts`、
  `resizableColumns.ts`。均有伴生 `.test.ts`。
- `src/api.ts`：API 客户端。所有请求自动带 `/token-stats/` 前缀；时间筛选接口计算并传
  `tz_offset`（分钟）。
- 文案统一走 `ZH` 常量（中文）；来源颜色/标签在 `lib/utils.ts` 的
  `SOURCE_COLORS` / `SOURCE_LABELS`，新增数据源时同步扩展。

## 注意事项

- Vite `base: "/token-stats/"`；**不要**手动建 `../backend/static`，由构建生成。
- 所有本地状态（筛选、隐藏配额卡、告警忽略等）持久化在 localStorage，
  键前缀 `token-stats:`。
- 时区：`tzOffset = -new Date().getTimezoneOffset()`（UTC+8 → 480），随每次请求传给后端。
