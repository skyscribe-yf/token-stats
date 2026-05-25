#!/usr/bin/env bash
set -euo pipefail

PROJECT_DIR="$HOME/srcs/token-stats"
BINARY_NAME="token-stats-backend"
PORT_A=3000
PORT_B=3001
HEALTH_TIMEOUT=30
NGINX_CONF_SRC="$PROJECT_DIR/nginx/token-stats.conf"
NGINX_CONF_DST="/etc/nginx/sites-available/token-stats"

# ── helpers ───────────────────────────────────────────────────────────

health_check() {
    local port=$1
    local i
    for i in $(seq 1 "$HEALTH_TIMEOUT"); do
        if curl -sf "http://127.0.0.1:$port/api/filters" >/dev/null 2>&1; then
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

    sudo mkdir -p "$dropin_dir"
    if [ ! -f "$dropin_file" ]; then
        echo "[Service]" | sudo tee "$dropin_file" >/dev/null
    fi
    # Remove existing line for this variable, then append
    sudo sed -i "/^Environment=\"$var_name=/d" "$dropin_file" 2>/dev/null || true
    echo "Environment=\"$var_name=$var_value\"" | sudo tee -a "$dropin_file" >/dev/null
}

clear_env_dropins() {
    local service_instance=$1
    sudo rm -rf "/etc/systemd/system/${service_instance}.service.d"
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

if [ ${#ACTIVE_PORTS[@]} -gt 0 ]; then
    CURRENT_PORT="${ACTIVE_PORTS[0]}"
    echo "✅ Active template instance(s): ${ACTIVE_PORTS[*]}"
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
echo "📋 Installing systemd template service..."
sudo cp "$PROJECT_DIR/nginx/token-stats@.service" /etc/systemd/system/token-stats@.service

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

if [ -n "${XAI_API_KEY:-}" ]; then
    inject_env_dropin "$NEW_INSTANCE" "XAI_API_KEY" "$XAI_API_KEY"
    echo "✅ Injected XAI_API_KEY"
else
    echo "⚠️  XAI_API_KEY not set"
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

sudo systemctl daemon-reload

# ── 6. Start new instance ─────────────────────────────────────────────
echo ""
echo "🟢 Starting $NEW_INSTANCE..."
sudo systemctl start "$NEW_INSTANCE"

# ── 7. Health check ───────────────────────────────────────────────────
echo "⏳ Health check on port $NEW_PORT (max ${HEALTH_TIMEOUT}s)..."
if ! health_check "$NEW_PORT"; then
    echo "❌ Health check failed — aborting and cleaning up"
    sudo systemctl stop "$NEW_INSTANCE" 2>/dev/null || true
    exit 1
fi
echo "✅ New instance is healthy"
echo ""

# ── 8. Update nginx to point to new port ──────────────────────────────
echo "🔄 Updating nginx upstream to port $NEW_PORT..."
sed "s|server 127.0.0.1:[0-9]*;|server 127.0.0.1:$NEW_PORT;|" "$NGINX_CONF_SRC" | sudo tee "$NGINX_CONF_DST" >/dev/null
sudo ln -sf "$NGINX_CONF_DST" /etc/nginx/sites-enabled/token-stats

echo "🧪 Testing nginx configuration..."
sudo nginx -t
echo "✅ nginx config valid"

echo "🔄 Reloading nginx gracefully..."
sudo nginx -s reload
echo "✅ nginx reloaded — traffic now routing to port $NEW_PORT"
echo ""

# ── 9. Drain and stop old instance(s) ─────────────────────────────────
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
    echo "✅ Old instance stopped"
done
echo ""

# ── 10. Enable new instance for boot ──────────────────────────────────
sudo systemctl enable "$NEW_INSTANCE"

# ── 11. Verify ────────────────────────────────────────────────────────
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
