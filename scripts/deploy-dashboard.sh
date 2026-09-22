#!/usr/bin/env bash
# deploy-dashboard.sh — 蓝绿部署包装脚本（给 deploy.sh 补几个手动部署时的坑）。
#
# 1. 备用端口上若已有旧的 systemd 实例在跑，deploy.sh 里的 `systemctl start`
#    对已 active 的 unit 是 no-op，健康检查照样通过 → nginx 切过去后仍在跑
#    旧二进制（新代码 / 新 pricing.toml 不生效）。这里先停掉非活跃端口的残留实例。
# 2. deploy.sh 依赖当前 shell 里的凭据环境变量：变量缺失时会先 clear_env_dropins
#    再注入空值，新实例直接丢光所有凭据（配额卡、dim 源全挂）。这里自动 source
#    deploy-env.sh（可用 DEPLOY_ENV_FILE 覆盖路径）。
# 3. CodeBuddy 的 session/session_2 cookie 约 30 天过期（过期后配额卡 401）。
#    刷新在 deploy.sh 里（source 凭据文件前用 --env-only 从 Chrome 重新提取并写回
#    deploy-env.sh，注入由 deploy.sh 对新实例的 drop-in 完成），两条入口都只提取一次。
#    Chrome/keyring 不可用时只告警并沿用旧值；设 SKIP_CODEBUDDY_COOKIE_REFRESH=1
#    可整体跳过（这里 export 后 exec 的 deploy.sh 会继承）。
#    StepFun 的 Oasis-Token access JWT 约 30 分钟过期，同样在 deploy.sh 里从
#    Chrome 重取并 RefreshToken（SKIP_STEPFUN_TOKEN_REFRESH=1 可跳过）。
#    DimAgent 的 dimagent.cn session cookie 也约 30 天过期，同样在 deploy.sh 里
#    从 Chrome 重取（SKIP_DIMAGENT_COOKIE_REFRESH=1 可跳过）。它挂掉的后果比配额卡
#    401 更隐蔽：Dim 自有 OAuth 通道只由 console API 逐请求计量，本地 dimcode 补充
#    又故意排除该通道防双计 → cookie 一死这类流量（deepseek-v4.1-flash 等）两个源
#    都没有，历史还在但新数据全丢，且只走优雅降级不留错误。刷新脚本会先对
#    console API 实测 200 才写回，避免用第二个失效 cookie 覆盖可用值。
#    注意：这里 source 凭据文件会把当时的 CODEBUDDY / STEPFUN / DIMAGENT 会话留在
#    shell 环境里，
#    而 deploy.sh 之后才去提取新值，所以 deploy.sh 在刷新成功后会
#    unset 掉这些变量让文件里的新值生效（详见 deploy.sh 0a-pre 的注释）。
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

if [ "${SKIP_CODEBUDDY_COOKIE_REFRESH:-}" = "1" ]; then
    echo "→ 跳过 CodeBuddy cookie 刷新（SKIP_CODEBUDDY_COOKIE_REFRESH=1）"
    export SKIP_CODEBUDDY_COOKIE_REFRESH
fi

if [ "${SKIP_STEPFUN_TOKEN_REFRESH:-}" = "1" ]; then
    echo "→ 跳过 StepFun token 刷新（SKIP_STEPFUN_TOKEN_REFRESH=1）"
    export SKIP_STEPFUN_TOKEN_REFRESH
fi

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
