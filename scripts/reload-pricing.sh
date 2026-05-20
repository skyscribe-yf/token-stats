#!/bin/bash
# reload-pricing.sh — 重新加载 pricing.toml 配置，无需重启后端。
#
# 用法:
#   ./scripts/reload-pricing.sh
#   ./scripts/reload-pricing.sh http://localhost:3000
#
# 修改 backend/pricing.toml 后运行此脚本，新的价格/汇率会立即生效。

API_BASE="${1:-http://localhost:3000}"

echo "→ Reloading pricing config from ${API_BASE}/api/pricing/reload ..."
curl -s -X POST "${API_BASE}/api/pricing/reload" | jq .

echo "→ Current pricing config:"
curl -s "${API_BASE}/api/pricing" | jq .
