#!/bin/bash
# update-exchange-rate.sh — 每 2 周获取最新 USD/CNY 汇率并追加为分段汇率。
#
# 用法:
#   ./scripts/update-exchange-rate.sh                           # 自动拉取最新汇率并追加
#   ./scripts/update-exchange-rate.sh --rate 6.78 --date 2026-08-15   # 手动补录
#   ./scripts/update-exchange-rate.sh --force                   # 忽略 14 天间隔检查
#   ./scripts/update-exchange-rate.sh --api http://localhost:3000
#
# 说明:
#   - 距上次 rate_date 不足 14 天时自动跳过（幂等，防重复追加）。
#   - 汇率来源：open.er-api.com（市场价，含时间戳）；可用 --rate/--date 手动补录
#     央行中间价等其它来源。
#   - 追加后派生除数（fenno/freemodel/grok/ollama）会自动按新分段缩放，
#     DeepSeek 为 CNY 定价无需重算，因此只改汇率即可。
#   - 成功后默认调用 /api/pricing/reload 热生效。
set -euo pipefail

# ── 定位 pricing.toml ────────────────────────────────────────────────────────
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CONFIG="${PRICING_CONFIG:-}"
if [[ -z "$CONFIG" ]]; then
  if [[ -f "${SCRIPT_DIR}/../backend/pricing.toml" ]]; then
    CONFIG="${SCRIPT_DIR}/../backend/pricing.toml"
  else
    CONFIG="./pricing.toml"
  fi
fi

# ── 参数解析 ─────────────────────────────────────────────────────────────────
RATE=""
DATE=""
FORCE=0
API_BASE="http://localhost:3000"
while [[ $# -gt 0 ]]; do
  case "$1" in
    --rate) RATE="$2"; shift 2 ;;
    --date) DATE="$2"; shift 2 ;;
    --force) FORCE=1; shift ;;
    --api) API_BASE="$2"; shift 2 ;;
    *) echo "未知参数: $1" >&2; exit 1 ;;
  esac
done

if [[ ! -f "$CONFIG" ]]; then
  echo "错误: 找不到 $CONFIG" >&2
  exit 1
fi

# ── 读取当前 rate_date，检查 14 天间隔 ─────────────────────────────────────
CURRENT_DATE="$(grep -m1 -E '^rate_date[[:space:]]*=' "$CONFIG" | sed -E 's/.*"([^"]+)".*/\1/' || true)"
if [[ -z "$CURRENT_DATE" ]]; then
  echo "错误: 无法从 $CONFIG 解析 rate_date" >&2
  exit 1
fi

if [[ -z "$DATE" ]]; then
  DATE="$(date +%F)"
fi

if [[ "$FORCE" -eq 0 && -z "$RATE" ]]; then
  LAST_TS="$(date -d "$CURRENT_DATE" +%s 2>/dev/null || echo 0)"
  NOW_TS="$(date +%s)"
  DAYS=$(( (NOW_TS - LAST_TS) / 86400 ))
  if (( DAYS < 14 )); then
    echo "距上次更新（$CURRENT_DATE）仅 ${DAYS} 天，不足 14 天，跳过（可用 --force 强制）。"
    exit 0
  fi
fi

# ── 获取汇率 ────────────────────────────────────────────────────────────────
if [[ -z "$RATE" ]]; then
  echo "→ 拉取最新 USD/CNY (open.er-api.com) ..."
  RATE="$(curl -s --max-time 15 "https://open.er-api.com/v6/latest/USD" \
    | python3 -c 'import json,sys; print(json.load(sys.stdin)["rates"]["CNY"])')"
  if [[ -z "$RATE" ]]; then
    echo "错误: 汇率拉取失败，请用 --rate/--date 手动补录。" >&2
    exit 1
  fi
fi

if [[ ! "$RATE" =~ ^[0-9]+(\.[0-9]+)?$ ]]; then
  echo "错误: 无效汇率 '$RATE'" >&2
  exit 1
fi

echo "→ 新分段: effective_from=$DATE, rate=$RATE"

# ── 更新 usd_to_cny / rate_date，并在 rate_date 后插入新分段 ──────────────
perl -pi -e "s/^usd_to_cny[[:space:]]*=.*/usd_to_cny = $RATE/; s/^rate_date[[:space:]]*=.*/rate_date = \"$DATE\"/" "$CONFIG"

awk -v date="$DATE" -v rate="$RATE" '
  /^rate_date[[:space:]]*=/ {
    print
    print ""
    print "[[usd_to_cny_segments]]"
    print "effective_from = \"" date "\""
    print "rate = " rate
    next
  }
  { print }
' "$CONFIG" > "$CONFIG.tmp" && mv "$CONFIG.tmp" "$CONFIG"

echo "→ 已更新 $CONFIG："
grep -nE '^(usd_to_cny|rate_date|\[\[usd_to_cny_segments\]\]|effective_from|rate) *=' "$CONFIG" | tail -12

# ── 热加载 ──────────────────────────────────────────────────────────────────
if curl -s --max-time 5 -X POST "${API_BASE}/api/pricing/reload" >/dev/null 2>&1; then
  echo "→ 已通过 ${API_BASE}/api/pricing/reload 热生效。"
else
  echo "→ 后端 reload 失败（可能未运行），请稍后执行 ./scripts/reload-pricing.sh。"
fi
