#!/usr/bin/env bash
# deploy-dashboard.sh — 蓝绿部署包装脚本（给 deploy.sh 补两个手动部署时的坑）。
#
# 1. 备用端口上若已有旧的 systemd 实例在跑，deploy.sh 里的 `systemctl start`
#    对已 active 的 unit 是 no-op，健康检查照样通过 → nginx 切过去后仍在跑
#    旧二进制（新代码 / 新 pricing.toml 不生效）。这里先停掉非活跃端口的残留实例。
# 2. deploy.sh 依赖当前 shell 里的凭据环境变量：变量缺失时会先 clear_env_dropins
#    再注入空值，新实例直接丢光所有凭据（配额卡、dim 源全挂）。这里自动 source
#    deploy-env.sh（可用 DEPLOY_ENV_FILE 覆盖路径）。
#
# 用法：cd ~/srcs/token-stats && ./scripts/deploy-dashboard.sh
# 需要 sudo（会提示输入密码）。
set -euo pipefail

PROJECT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ENV_FILE="${DEPLOY_ENV_FILE:-$HOME/.config/token-stats/deploy-env.sh}"
NGINX_CONF="/etc/nginx/sites-available/token-stats"
PORTS=(3000 3001)

LIVE_PORT="$(sed -n 's/.*server 127\.0\.0\.1:\([0-9]\+\).*/\1/p' "$NGINX_CONF" 2>/dev/null | head -n1)"
if [ -z "$LIVE_PORT" ]; then
    echo "❌ 无法从 $NGINX_CONF 解析当前 upstream 端口，中止（避免误停所有实例）"
    exit 1
fi
echo "→ nginx 当前指向端口：$LIVE_PORT"

for p in "${PORTS[@]}"; do
    [ "$p" = "$LIVE_PORT" ] && continue
    if systemctl is-active --quiet "token-stats@$p"; then
        echo "→ 停止备用端口残留实例 token-stats@$p（否则 deploy 不会重启它）"
        sudo systemctl stop "token-stats@$p"
    fi
done

if [ -f "$ENV_FILE" ]; then
    echo "→ 载入凭据：$ENV_FILE"
    set -a
    # shellcheck disable=SC1090
    . "$ENV_FILE"
    set +a
else
    echo "❌ 未找到 $ENV_FILE —— 中止，避免新实例丢凭据"
    exit 1
fi

exec "$PROJECT_DIR/deploy.sh"
