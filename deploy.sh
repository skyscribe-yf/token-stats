#!/usr/bin/env bash
set -euo pipefail

PROJECT_DIR="$HOME/srcs/token-stats"
BINARY_NAME="token-stats-backend"
PORT_A=3000
PORT_B=3001
# Cold start reloads every source and, on a fresh process, walks the whole
# DimAgent console history page by page (MAX_PAGES=400, ~0.25s each) before
# the HTTP listener is bound. Budget generously; /api/filters can also be slow
# while the first full aggregation runs.
HEALTH_TIMEOUT=240
NGINX_CONF_SRC="$PROJECT_DIR/nginx/token-stats.conf"
NGINX_CONF_DST="/etc/nginx/sites-available/token-stats"

# ── 0a. Load credentials into THIS shell ──────────────────────────────
# MUST happen before any inject_env_dropin call. clear_env_dropins() wipes the
# whole drop-in directory, so anything not present in this shell is *deleted*
# from the new instance — running deploy.sh from a shell without the creds
# exported silently drops every quota-card credential and the dim source.
#
# deploy.sh previously relied on the caller having sourced deploy-env.sh
# (scripts/deploy-dashboard.sh does so via `exec`). Direct `./deploy.sh` calls
# inherited whatever happened to be exported, or nothing. Loading it here makes
# this script self-sufficient either way; already-set values win so an explicit
# one-off override still works.
DEPLOY_ENV_FILE="${TOKEN_STATS_DEPLOY_ENV:-$HOME/.config/token-stats/deploy-env.sh}"

# ── 0a-pre. Refresh the CodeBuddy cookies from Chrome ──────────────────
# Those two cookies lapse every ~30 days of browser inactivity; once stale the
# card returns 401 and a deploy would faithfully re-inject the dead values.
# Extract fresh ones into $DEPLOY_ENV_FILE first, then let the source below pick
# them up. Best effort — Chrome/keyring may be unavailable (headless or remote
# deploy), which must not fail the deploy. SKIP_CODEBUDDY_COOKIE_REFRESH=1 to opt out.
if [ "${SKIP_CODEBUDDY_COOKIE_REFRESH:-}" = "1" ]; then
    echo "→ Skipping CodeBuddy cookie refresh (SKIP_CODEBUDDY_COOKIE_REFRESH=1)"
elif TOKEN_STATS_DEPLOY_ENV="$DEPLOY_ENV_FILE" \
        "$PROJECT_DIR/scripts/refresh-codebuddy-cookies.sh" --env-only; then
    echo "🔑 Refreshed CodeBuddy cookies from Chrome"
    # The refresher just wrote the browser's live cookies into $DEPLOY_ENV_FILE.
    # A copy already exported in this shell is by definition staler — ~/.bash_env
    # exports both, and scripts/deploy-dashboard.sh sources the file before
    # exec'ing us — but the "explicit override" re-application below would let it
    # win, silently re-injecting dead cookies. That is how the 2026-09-20 deploy
    # shipped a 2026-09-05-expired cookie into token-stats@3001 while
    # $DEPLOY_ENV_FILE held a valid one. Drop them so the file's fresh value wins.
    unset CODEBUDDY_SESSION_COOKIE CODEBUDDY_SESSION_COOKIE_2
else
    echo "⚠️  CodeBuddy cookie refresh failed (Chrome/keyring unavailable?) — using existing $DEPLOY_ENV_FILE values"
fi

# ── 0a-pre2. Refresh the StepFun console session from Chrome ────────────
# Oasis-Token's access JWT lapses in ~30 minutes. Re-injecting the pair stored
# in deploy-env.sh hides the Step Plan pool (401, plan=null) even though the
# credit-balance half of the card still works. Refresh rotates access+refresh
# and writes both deploy-env.sh and stepfun-auth.json. Best effort, same as
# CodeBuddy. SKIP_STEPFUN_TOKEN_REFRESH=1 to opt out.
if [ "${SKIP_STEPFUN_TOKEN_REFRESH:-}" = "1" ]; then
    echo "→ Skipping StepFun token refresh (SKIP_STEPFUN_TOKEN_REFRESH=1)"
elif TOKEN_STATS_DEPLOY_ENV="$DEPLOY_ENV_FILE" \
        "$PROJECT_DIR/scripts/refresh-stepfun-token.sh" --env-only; then
    echo "🔑 Refreshed StepFun Oasis session from Chrome"
    # Same override trap as CodeBuddy: a shell-exported copy is staler than the
    # file just rewritten above, and the re-application below would put it back.
    unset STEPFUN_OASIS_TOKEN STEPFUN_OASIS_WEBID
else
    echo "⚠️  StepFun token refresh failed (Chrome/keyring unavailable?) — using existing $DEPLOY_ENV_FILE values"
fi

# ── 0a-pre3. Refresh the DimAgent console session from Chrome ───────────
# The dimagent.cn `session` cookie lapses ~30 days after the last browser login.
# A dead cookie is worse than a broken card: Dim bills its own OAuth channel
# through the console API, and the local dimcode supplement excludes that
# provider on purpose, so every Dim-OAuth call (deepseek-v4.1-flash and friends)
# falls into a hole — present in neither source, with only a graceful-degradation
# 401 to show for it. Refresh + live-verify before the source below picks it up.
# Best effort, as above. SKIP_DIMAGENT_COOKIE_REFRESH=1 to opt out.
if [ "${SKIP_DIMAGENT_COOKIE_REFRESH:-}" = "1" ]; then
    echo "→ Skipping DimAgent cookie refresh (SKIP_DIMAGENT_COOKIE_REFRESH=1)"
elif TOKEN_STATS_DEPLOY_ENV="$DEPLOY_ENV_FILE" \
        "$PROJECT_DIR/scripts/refresh-dimagent-cookie.sh" --env-only; then
    echo "🔑 Refreshed DimAgent console cookie from Chrome"
    # Same override trap as CodeBuddy: a shell-exported copy is staler than the
    # file just rewritten above, and the re-application below would put it back.
    unset DIMAGENT_SESSION_COOKIE
else
    echo "⚠️  DimAgent cookie refresh failed (Chrome/keyring unavailable?) — using existing $DEPLOY_ENV_FILE values"
fi

if [ -f "$DEPLOY_ENV_FILE" ]; then
    _env_overrides="$(env | grep -E '^(CODEBUDDY_|COMMANDCODE_|DIMAGENT_|OPENCODE_GO_|OLLAMA_AUTH_|MEITUAN_|FENNO_|KIMI_|YAI_|ZAI_|STEPFUN_|XIAOMI_MIMO_|XUNFEI_|GROK_|DIM_|CCSWITCH_|USE_CC_SWITCH)' || true)"
    set -a
    # shellcheck disable=SC1090
    . "$DEPLOY_ENV_FILE"
    set +a
    if [ -n "$_env_overrides" ]; then
        # Re-apply values that were already exported before the source.
        while IFS= read -r _line; do
            [ -z "$_line" ] && continue
            export "$_line"
        done <<< "$_env_overrides"
    fi
    unset _env_overrides _line
    echo "🔑 Loaded credentials from $DEPLOY_ENV_FILE"
else
    echo "⚠️  $DEPLOY_ENV_FILE not found — relying on the current shell's environment"
fi

# Refuse to deploy when credentials that are *required* for a card/source are
# missing. This turns a silent "card broke after deploy" into a loud abort.
# Set TOKEN_STATS_ALLOW_MISSING_CREDS=1 to bypass (e.g. intentionally deploying
# without a given provider).
if [ "${TOKEN_STATS_ALLOW_MISSING_CREDS:-}" != "1" ]; then
    missing=()
    for pair in \
        "CODEBUDDY_SESSION_COOKIE:CodeBuddy card" \
        "CODEBUDDY_SESSION_COOKIE_2:CodeBuddy card (session_2)" \
        "DIMAGENT_SESSION_COOKIE:dim data source + DimAgent card" \
        "YAI_API_KEY:Ainaiba/XAI balance" \
        "ZAI_API_KEY:ZAI card" \
        "OLLAMA_AUTH_COOKIE:Ollama Cloud" \
        "FENNO_AUTH_TOKEN:Fenno card" \
        "MEITUAN_AUTH_COOKIE:Meituan LongCat" \
        "XIAOMI_MIMO_SERVICE_TOKEN:Xiaomi MiMo"
    do
        var="${pair%%:*}"
        label="${pair#*:}"
        if [ -z "${!var:-}" ]; then
            missing+=("$var ($label)")
        fi
    done
    if [ ${#missing[@]} -gt 0 ]; then
        echo ""
        echo "❌ Aborting deploy — required credentials missing from $DEPLOY_ENV_FILE and the shell:"
        printf '   • %s\n' "${missing[@]}"
        echo ""
        echo "   These would be DELETED from the new instance by clear_env_dropins,"
        echo "   leaving the corresponding cards/sources dead."
        echo "   Fix: populate $DEPLOY_ENV_FILE, or set"
        echo "        TOKEN_STATS_ALLOW_MISSING_CREDS=1 to deploy anyway."
        echo ""
        exit 1
    fi
    echo "✅ All required credentials present"
fi

# ── helpers ───────────────────────────────────────────────────────────

health_check() {
    local port=$1
    local i
    for i in $(seq 1 "$HEALTH_TIMEOUT"); do
        if curl -sf -m 10 "http://127.0.0.1:$port/api/filters" >/dev/null 2>&1; then
            return 0
        fi
        sleep 1
    done
    return 1
}

inject_env_dropin() {
    local service_instance=$1
    local var_name=$2
    local var_value=$3
    local dropin_dir="/etc/systemd/system/${service_instance}.service.d"
    local dropin_file="$dropin_dir/env.conf"

    # Systemd interprets % as specifiers in Environment= lines.
    # Escape literal % as %% so values like %2B / %2F / %3D are preserved.
    local escaped_value="${var_value//%/%%}"

    sudo mkdir -p "$dropin_dir"
    if [ ! -f "$dropin_file" ]; then
        echo "[Service]" | sudo tee "$dropin_file" >/dev/null
    fi
    # Remove existing line for this variable, then append
    sudo sed -i "/^Environment=\"$var_name=/d" "$dropin_file" 2>/dev/null || true
    echo "Environment=\"$var_name=$escaped_value\"" | sudo tee -a "$dropin_file" >/dev/null
}

clear_env_dropins() {
    local service_instance=$1
    sudo rm -rf "/etc/systemd/system/${service_instance}.service.d"
}

# Warn — never fail — when the CodeBuddy cookies about to be shipped do not
# authenticate. Turns a silent "card 401s weeks later" into a deploy-time
# message; a dead cookie must not block a deploy.
warn_if_codebuddy_cookies_dead() {
    local code
    code=$(curl -s -o /dev/null -w '%{http_code}' -m 20 \
        -X POST "https://www.codebuddy.cn/billing/meter/get-user-resource-summary" \
        -H "Cookie: session=$CODEBUDDY_SESSION_COOKIE; session_2=$CODEBUDDY_SESSION_COOKIE_2" \
        -H 'Content-Type: application/json' \
        -H 'Origin: https://www.codebuddy.cn' \
        -H 'Referer: https://www.codebuddy.cn/profile/plans-usage' \
        -H 'User-Agent: Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/152.0.0.0 Safari/537.36' \
        -d '{"resourceType":1}' 2>/dev/null) || code="000"
    if [ "$code" != "200" ]; then
        echo "⚠️  CodeBuddy cookies return HTTP $code (expected 200) — the card will 401."
        echo "    Re-login at https://www.codebuddy.cn/ in Chrome, or unset the stale"
        echo "    CODEBUDDY_* exports in ~/.bash_env, then redeploy."
    fi
}

# Find a non-systemd token-stats-backend process listening on a given port.
# Returns the PID on stdout, or exits 1 if none found.
get_rogue_backend_pid() {
    local port=$1
    local pid

    # Try ss first, then fall back to lsof
    pid=$(ss -tlnp 2>/dev/null | grep -E ":$port\b" | grep -oP 'pid=\K[0-9]+' | head -n1)
    if [ -z "$pid" ] && command -v lsof >/dev/null 2>&1; then
        pid=$(lsof -t -i TCP:"$port" 2>/dev/null | head -n1)
    fi
    [ -z "$pid" ] && return 1

    # Verify it's actually our binary
    local exe
    exe=$(readlink -f /proc/"$pid"/exe 2>/dev/null || true)
    [[ "$exe" == *token-stats-backend* ]] || return 1

    # If systemd actively manages this port and this PID is the service's MainPID,
    # it's not rogue — it's legitimate.
    if systemctl is-active --quiet "token-stats@$port" 2>/dev/null; then
        local main_pid
        main_pid=$(systemctl show --property=MainPID --value "token-stats@$port" 2>/dev/null || true)
        if [ "$pid" = "$main_pid" ]; then
            return 1
        fi
    fi

    echo "$pid"
}

# Kill a rogue backend process gracefully, then forcefully if needed.
kill_rogue_backend() {
    local port=$1
    local pid
    pid=$(get_rogue_backend_pid "$port" 2>/dev/null || true)
    [ -z "$pid" ] && return 0

    echo "🛑 Killing rogue backend on port $port (PID $pid)..."
    kill "$pid" 2>/dev/null || true
    sleep 1
    if kill -0 "$pid" 2>/dev/null; then
        echo "   Force-killing PID $pid..."
        kill -9 "$pid" 2>/dev/null || true
        sleep 1
    fi
    if kill -0 "$pid" 2>/dev/null; then
        echo "❌ Unable to kill PID $pid — aborting"
        return 1
    fi
    echo "✅ Rogue backend on port $port removed"
}

# ── 0. Detect active port ─────────────────────────────────────────────
echo "🚀 Token Stats Dashboard — Zero-Downtime Deploy"
echo "================================================"
echo ""

LEGACY_ACTIVE=false
CURRENT_PORT=""
ACTIVE_PORTS=()

# Check legacy token-stats.service
if systemctl is-active --quiet token-stats 2>/dev/null; then
    LEGACY_ACTIVE=true
    CURRENT_PORT=3000
    echo "⚠️  Legacy token-stats.service is active (port 3000)"
fi

# Check template instances
for p in "$PORT_A" "$PORT_B"; do
    if systemctl is-active --quiet "token-stats@$p" 2>/dev/null; then
        ACTIVE_PORTS+=("$p")
    fi
done

# Detect rogue (non-systemd) backend processes that may hold ports
for p in "$PORT_A" "$PORT_B"; do
    rogue_pid=$(get_rogue_backend_pid "$p" 2>/dev/null || true)
    if [ -n "$rogue_pid" ]; then
        echo "⚠️  Rogue backend detected on port $p (PID $rogue_pid, not managed by systemd)"
        if [[ " ${ACTIVE_PORTS[*]} " != *" $p "* ]]; then
            ACTIVE_PORTS+=("$p")
        fi
    fi
done

if [ ${#ACTIVE_PORTS[@]} -gt 0 ]; then
    CURRENT_PORT="${ACTIVE_PORTS[0]}"
    echo "✅ Active instance(s): ${ACTIVE_PORTS[*]}"
fi

if [ -z "$CURRENT_PORT" ]; then
    echo "ℹ️  No active backend found"
fi

# Pick new port
if [ "$CURRENT_PORT" = "$PORT_A" ]; then
    NEW_PORT="$PORT_B"
elif [ "$CURRENT_PORT" = "$PORT_B" ]; then
    NEW_PORT="$PORT_A"
else
    NEW_PORT="$PORT_A"
fi

echo "🎯 New deployment will use port $NEW_PORT"
echo ""

# ── 1. Build backend (old service still running) ──────────────────────
echo "🔧 Building Rust backend..."
cd "$PROJECT_DIR/backend"
cargo build --release
echo "✅ Backend built"
echo ""

# ── 2. Build frontend (old service still running) ─────────────────────
echo "🔧 Building React frontend..."
cd "$PROJECT_DIR/frontend"
npm install
npm run build
echo "✅ Frontend built"
echo ""

# ── 3. Deploy static files atomically ─────────────────────────────────
echo "📋 Deploying static files..."
STATIC_TMP="/var/www/token-stats-deploy-$$"
sudo mkdir -p "$STATIC_TMP"
if [ -d "$PROJECT_DIR/backend/static" ]; then
    sudo cp -r "$PROJECT_DIR/backend/static/"* "$STATIC_TMP/"
    sudo chmod -R 755 "$STATIC_TMP"
    # Atomic swap
    sudo rm -rf /var/www/token-stats-prev 2>/dev/null || true
    sudo mv -T /var/www/token-stats /var/www/token-stats-prev 2>/dev/null || true
    sudo mv -T "$STATIC_TMP" /var/www/token-stats
    sudo rm -rf /var/www/token-stats-prev
fi
echo "✅ Static files deployed"
echo ""

# ── 4. Install template service file ──────────────────────────────────
echo "📋 Installing systemd services..."
sudo cp "$PROJECT_DIR/nginx/token-stats@.service" /etc/systemd/system/token-stats@.service
sudo cp "$PROJECT_DIR/nginx/token-stats-grok-proxy.service" /etc/systemd/system/token-stats-grok-proxy.service

# ── 5. Inject environment variables for new instance ──────────────────
NEW_INSTANCE="token-stats@$NEW_PORT"

# Clear stale drop-ins for this port, then inject current env
clear_env_dropins "$NEW_INSTANCE"

if [ -n "${XUNFEI_SSO_SESSION_ID:-}" ]; then
    inject_env_dropin "$NEW_INSTANCE" "XUNFEI_SSO_SESSION_ID" "$XUNFEI_SSO_SESSION_ID"
    echo "✅ Injected XUNFEI_SSO_SESSION_ID"
else
    echo "⚠️  XUNFEI_SSO_SESSION_ID not set"
fi

if [ -n "${XUNFEI_SSO_SESSION_ID_EX:-}" ]; then
    inject_env_dropin "$NEW_INSTANCE" "XUNFEI_SSO_SESSION_ID_EX" "$XUNFEI_SSO_SESSION_ID_EX"
    echo "✅ Injected XUNFEI_SSO_SESSION_ID_EX"
else
    echo "⚠️  XUNFEI_SSO_SESSION_ID_EX not set"
fi

if [ -n "${OPENCODE_GO_WORKSPACE_ID:-}" ] && [ -n "${OPENCODE_GO_AUTH_COOKIE:-}" ]; then
    inject_env_dropin "$NEW_INSTANCE" "OPENCODE_GO_WORKSPACE_ID" "$OPENCODE_GO_WORKSPACE_ID"
    inject_env_dropin "$NEW_INSTANCE" "OPENCODE_GO_AUTH_COOKIE" "$OPENCODE_GO_AUTH_COOKIE"
    echo "✅ Injected OpenCode-go credentials"
else
    echo "⚠️  OpenCode-go credentials not set"
fi

if [ -n "${OPENCODE_GO_WORKSPACE_ID_EX:-}" ] && [ -n "${OPENCODE_GO_AUTH_COOKIE_EX:-}" ]; then
    inject_env_dropin "$NEW_INSTANCE" "OPENCODE_GO_WORKSPACE_ID_EX" "$OPENCODE_GO_WORKSPACE_ID_EX"
    inject_env_dropin "$NEW_INSTANCE" "OPENCODE_GO_AUTH_COOKIE_EX" "$OPENCODE_GO_AUTH_COOKIE_EX"
    echo "✅ Injected OpenCode-go EX credentials"
else
    echo "⚠️  OpenCode-go EX credentials not set"
fi

if [ -n "${YAI_API_KEY:-}" ]; then
    inject_env_dropin "$NEW_INSTANCE" "YAI_API_KEY" "$YAI_API_KEY"
    echo "✅ Injected YAI_API_KEY"
else
    echo "⚠️  YAI_API_KEY not set"
fi

# ZAI (ZAI Router, api.zairouter.com) — balance/usage card on /api/quota.
# Note this is a *different* account from YAI/Ainaba: separate key, separate
# balance, and its own billing formula (official Anthropic price × per-model
# rate, see [[zai_model]] in pricing.toml).
if [ -n "${ZAI_API_KEY:-}" ]; then
    inject_env_dropin "$NEW_INSTANCE" "ZAI_API_KEY" "$ZAI_API_KEY"
    echo "✅ Injected ZAI_API_KEY"
else
    echo "⚠️  ZAI_API_KEY not set"
fi

# StepFun (platform.stepfun.com) — credit balance card on /api/quota.
# Same key the CPA `stepfun` upstream uses (~/.bash_env).
if [ -n "${STEPFUN_API_KEY:-}" ]; then
    inject_env_dropin "$NEW_INSTANCE" "STEPFUN_API_KEY" "$STEPFUN_API_KEY"
    echo "✅ Injected STEPFUN_API_KEY"
else
    echo "⚠️  STEPFUN_API_KEY not set"
fi

# StepFun Step Plan pool (console RPC). Both values come from a logged-in
# platform.stepfun.com Chrome session: ./scripts/extract-stepfun-token.sh
# The JWT lasts ~30 minutes; the running backend renews it into
# ~/.config/token-stats/stepfun-auth.json, which outranks these env values.
if [ -n "${STEPFUN_OASIS_TOKEN:-}" ] && [ -n "${STEPFUN_OASIS_WEBID:-}" ]; then
    inject_env_dropin "$NEW_INSTANCE" "STEPFUN_OASIS_TOKEN" "$STEPFUN_OASIS_TOKEN"
    inject_env_dropin "$NEW_INSTANCE" "STEPFUN_OASIS_WEBID" "$STEPFUN_OASIS_WEBID"
    echo "✅ Injected STEPFUN_OASIS_TOKEN/WEBID"
else
    echo "⚠️  STEPFUN_OASIS_TOKEN/WEBID not set (Step Plan pool hidden)"
fi

if [ -n "${XIAOMI_MIMO_SERVICE_TOKEN:-}" ]; then
    inject_env_dropin "$NEW_INSTANCE" "XIAOMI_MIMO_SERVICE_TOKEN" "$XIAOMI_MIMO_SERVICE_TOKEN"
    echo "✅ Injected XIAOMI_MIMO_SERVICE_TOKEN"
else
    echo "⚠️  XIAOMI_MIMO_SERVICE_TOKEN not set"
fi

if [ -n "${XIAOMI_MIMO_USER_ID:-}" ]; then
    inject_env_dropin "$NEW_INSTANCE" "XIAOMI_MIMO_USER_ID" "$XIAOMI_MIMO_USER_ID"
    echo "✅ Injected XIAOMI_MIMO_USER_ID"
else
    echo "⚠️  XIAOMI_MIMO_USER_ID not set"
fi

if [ -n "${COMMANDCODE_SESSION_TOKEN:-}" ]; then
    inject_env_dropin "$NEW_INSTANCE" "COMMANDCODE_SESSION_TOKEN" "$COMMANDCODE_SESSION_TOKEN"
    echo "✅ Injected COMMANDCODE_SESSION_TOKEN"
else
    echo "⚠️  COMMANDCODE_SESSION_TOKEN not set"
fi

if [ -n "${CODEBUDDY_SESSION_COOKIE:-}" ] && [ -n "${CODEBUDDY_SESSION_COOKIE_2:-}" ]; then
    warn_if_codebuddy_cookies_dead
    inject_env_dropin "$NEW_INSTANCE" "CODEBUDDY_SESSION_COOKIE" "$CODEBUDDY_SESSION_COOKIE"
    inject_env_dropin "$NEW_INSTANCE" "CODEBUDDY_SESSION_COOKIE_2" "$CODEBUDDY_SESSION_COOKIE_2"
    echo "✅ Injected CodeBuddy cookies"
else
    echo "⚠️  CodeBuddy cookies not set (run scripts/extract-codebuddy-cookies.sh)"
fi

if [ -n "${OLLAMA_AUTH_COOKIE:-}" ]; then
    inject_env_dropin "$NEW_INSTANCE" "OLLAMA_AUTH_COOKIE" "$OLLAMA_AUTH_COOKIE"
    echo "✅ Injected OLLAMA_AUTH_COOKIE"
else
    echo "⚠️  OLLAMA_AUTH_COOKIE not set"
fi

if [ -n "${MEITUAN_AUTH_COOKIE:-}" ]; then
    inject_env_dropin "$NEW_INSTANCE" "MEITUAN_AUTH_COOKIE" "$MEITUAN_AUTH_COOKIE"
    echo "✅ Injected MEITUAN_AUTH_COOKIE"
else
    echo "⚠️  MEITUAN_AUTH_COOKIE not set"
fi

if [ -n "${FENNO_AUTH_TOKEN:-}" ]; then
    inject_env_dropin "$NEW_INSTANCE" "FENNO_AUTH_TOKEN" "$FENNO_AUTH_TOKEN"
    echo "✅ Injected FENNO_AUTH_TOKEN"
else
    echo "⚠️  FENNO_AUTH_TOKEN not set (using persisted Fenno auth state if available)"
fi

if [ -n "${FENNO_REFRESH_TOKEN:-}" ]; then
    inject_env_dropin "$NEW_INSTANCE" "FENNO_REFRESH_TOKEN" "$FENNO_REFRESH_TOKEN"
    echo "✅ Injected FENNO_REFRESH_TOKEN"
else
    echo "⚠️  FENNO_REFRESH_TOKEN not set (using persisted Fenno auth state if available)"
fi

if [ -n "${FENNO_AUTH_STATE_PATH:-}" ]; then
    inject_env_dropin "$NEW_INSTANCE" "FENNO_AUTH_STATE_PATH" "$FENNO_AUTH_STATE_PATH"
    echo "✅ Injected FENNO_AUTH_STATE_PATH"
fi

if [ -n "${FENNO_AUTH_TOKEN_EX:-}" ]; then
    inject_env_dropin "$NEW_INSTANCE" "FENNO_AUTH_TOKEN_EX" "$FENNO_AUTH_TOKEN_EX"
    echo "✅ Injected FENNO_AUTH_TOKEN_EX"
else
    echo "⚠️  FENNO_AUTH_TOKEN_EX not set"
fi

if [ -n "${FENNO_REFRESH_TOKEN_EX:-}" ]; then
    inject_env_dropin "$NEW_INSTANCE" "FENNO_REFRESH_TOKEN_EX" "$FENNO_REFRESH_TOKEN_EX"
    echo "✅ Injected FENNO_REFRESH_TOKEN_EX"
else
    echo "⚠️  FENNO_REFRESH_TOKEN_EX not set"
fi

if [ -n "${FENNO_AUTH_STATE_PATH_EX:-}" ]; then
    inject_env_dropin "$NEW_INSTANCE" "FENNO_AUTH_STATE_PATH_EX" "$FENNO_AUTH_STATE_PATH_EX"
    echo "✅ Injected FENNO_AUTH_STATE_PATH_EX"
fi

if [ -n "${DIMAGENT_SESSION_COOKIE:-}" ]; then
    inject_env_dropin "$NEW_INSTANCE" "DIMAGENT_SESSION_COOKIE" "$DIMAGENT_SESSION_COOKIE"
    echo "✅ Injected DIMAGENT_SESSION_COOKIE (dim console API source)"
else
    echo "⚠️  DIMAGENT_SESSION_COOKIE not set (dim source will be skipped)"
fi

if [ -n "${GROK_XAI_API_KEY:-}" ]; then
    inject_env_dropin "$NEW_INSTANCE" "GROK_XAI_API_KEY" "$GROK_XAI_API_KEY"
    echo "✅ Injected GROK_XAI_API_KEY"
else
    echo "⚠️  GROK_XAI_API_KEY not set (Grok quota card will use ~/.grok/auth.json fallback)"
fi

if [ -n "${DIMAGENT_SESSION_COOKIE:-}" ]; then
    inject_env_dropin "$NEW_INSTANCE" "DIMAGENT_SESSION_COOKIE" "$DIMAGENT_SESSION_COOKIE"
    echo "✅ Injected DIMAGENT_SESSION_COOKIE (DimAgent 30d stats)"
else
    echo "⚠️  DIMAGENT_SESSION_COOKIE not set (DimAgent card uses dim CLI only, no 30d stats)"
fi

if [ -n "${DIM_USAGE_BIN:-}" ]; then
    inject_env_dropin "$NEW_INSTANCE" "DIM_USAGE_BIN" "$DIM_USAGE_BIN"
    echo "✅ Injected DIM_USAGE_BIN"
fi

sudo systemctl daemon-reload

# ── 6. Free new port from rogue processes ─────────────────────────────
echo ""
echo "🔍 Checking for rogue backends on port $NEW_PORT..."
kill_rogue_backend "$NEW_PORT"

# ── 7. Start new instance ─────────────────────────────────────────────
echo ""
echo "🟢 Starting $NEW_INSTANCE..."
sudo systemctl start "$NEW_INSTANCE"

# ── 8. Health check ───────────────────────────────────────────────────
echo "⏳ Health check on port $NEW_PORT (max ${HEALTH_TIMEOUT}s)..."
if ! health_check "$NEW_PORT"; then
    echo "❌ Health check failed — aborting and cleaning up"
    sudo systemctl stop "$NEW_INSTANCE" 2>/dev/null || true
    exit 1
fi
echo "✅ New instance is healthy"
echo ""

# ── 9. Update nginx to point to new port ──────────────────────────────
echo "🔄 Updating nginx upstream to port $NEW_PORT..."
sed "s|server 127.0.0.1:[0-9]*;|server 127.0.0.1:$NEW_PORT;|" "$NGINX_CONF_SRC" | sudo tee "$NGINX_CONF_DST" >/dev/null
sudo ln -sf "$NGINX_CONF_DST" /etc/nginx/sites-enabled/token-stats

# Ensure token-stats is the default site (remove competing default)
if [ -L /etc/nginx/sites-enabled/default ]; then
    sudo rm -f /etc/nginx/sites-enabled/default
fi

echo "🧪 Testing nginx configuration..."
sudo nginx -t
echo "✅ nginx config valid"

echo "🔄 Reloading nginx gracefully..."
sudo nginx -s reload
echo "✅ nginx reloaded — traffic now routing to port $NEW_PORT"
echo ""

# ── 10. Drain and stop old instance(s) ────────────────────────────────
if [ "$LEGACY_ACTIVE" = true ]; then
    echo "⏳ Draining legacy connections (5s)..."
    sleep 5
    echo "🛑 Stopping legacy token-stats.service..."
    sudo systemctl stop token-stats 2>/dev/null || true
    sudo systemctl disable token-stats 2>/dev/null || true
    sudo rm -f /etc/systemd/system/token-stats.service
    sudo rm -rf /etc/systemd/system/token-stats.service.d
    sudo systemctl daemon-reload
    echo "✅ Legacy service removed"
fi

# Stop any template instances on the old port(s)
for p in "${ACTIVE_PORTS[@]}"; do
    if [ "$p" = "$NEW_PORT" ]; then
        continue
    fi
    OLD_INSTANCE="token-stats@$p"
    echo "⏳ Draining old connections on port $p (5s)..."
    sleep 5
    echo "🛑 Stopping $OLD_INSTANCE..."
    sudo systemctl stop "$OLD_INSTANCE" 2>/dev/null || true
    sudo systemctl disable "$OLD_INSTANCE" 2>/dev/null || true
    clear_env_dropins "$OLD_INSTANCE"
    echo "✅ Old systemd instance stopped"

    # Also clean up any rogue backend that might still be holding the port
    kill_rogue_backend "$p"
done
echo ""

# ── 11. Start stable Grok proxy after old dashboard releases port 3434 ─
echo "🟢 Restarting stable Grok usage proxy..."
sudo systemctl restart token-stats-grok-proxy.service
sudo systemctl enable token-stats-grok-proxy.service

# ── 12. Enable new instance for boot ──────────────────────────────────
sudo systemctl enable "$NEW_INSTANCE"

# ── 13. Verify ────────────────────────────────────────────────────────
echo "🧪 Verifying deployment..."
sleep 1
HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" http://localhost/token-stats/ 2>/dev/null || echo "000")
if [ "$HTTP_CODE" = "200" ]; then
    echo "✅ Dashboard is LIVE at http://localhost/token-stats/"
else
    echo "⚠️  Got HTTP $HTTP_CODE from nginx"
    echo "   Checking backend directly on port $NEW_PORT..."
    curl -sf "http://127.0.0.1:$NEW_PORT/api/filters" | head -3 || echo "   Backend not responding"
fi

echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "📊  Dashboard:  http://localhost/token-stats/"
echo "🔧  Backend:    http://localhost:$NEW_PORT (direct)"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""
echo "Useful commands:"
echo "  sudo systemctl status $NEW_INSTANCE     # check backend"
echo "  sudo systemctl restart $NEW_INSTANCE    # restart backend"
echo "  sudo journalctl -u $NEW_INSTANCE -f     # view logs"
echo "  sudo nginx -t && sudo nginx -s reload   # reload nginx"
