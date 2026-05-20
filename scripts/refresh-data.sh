#!/bin/bash
# refresh-data.sh — 强制后端重新扫描所有数据源并增量合并新记录。
#
# 用法:
#   ./scripts/refresh-data.sh
#   ./scripts/refresh-data.sh http://localhost:3000

API_BASE="${1:-http://localhost:3000}"

echo "→ Refreshing data sources from ${API_BASE}/api/refresh ..."
curl -s -X POST "${API_BASE}/api/refresh" | jq .
