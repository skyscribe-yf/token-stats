#!/usr/bin/env bash
set -e

PROJECT_DIR="$HOME/srcs/token-stats"

echo "🚀 Token Stats Dashboard — Deploy"
echo "=================================="
echo ""

# ── 1. Stop old backend & clean up port ──────────────────────────────
echo "🛑 Stopping old backend..."

BINARY_NAME="token-stats-backend"
PORT="3000"

# 1a. Stop systemd service
sudo systemctl stop token-stats 2>/dev/null || true

# 1b. Kill any stray processes matching the binary name (outside systemd)
PIDS=$(pgrep -f "$BINARY_NAME" || true)
if [ -n "$PIDS" ]; then
    echo "⚠️  Found stray $BINARY_NAME processes: $PIDS"
    echo "$PIDS" | xargs -r kill -TERM 2>/dev/null || true
    sleep 2
    # Force kill if still alive
    PIDS_STILL=$(pgrep -f "$BINARY_NAME" || true)
    if [ -n "$PIDS_STILL" ]; then
        echo "$PIDS_STILL" | xargs -r kill -KILL 2>/dev/null || true
        sleep 1
    fi
fi

# 1c. Wait until the TCP port is fully released
WAIT_MAX=15
WAIT_COUNT=0
while ss -tlnp 2>/dev/null | grep -q ":$PORT "; do
    WAIT_COUNT=$((WAIT_COUNT + 1))
    if [ "$WAIT_COUNT" -ge "$WAIT_MAX" ]; then
        echo "❌ Port $PORT is still in use after ${WAIT_MAX}s — aborting"
        ss -tlnp | grep ":$PORT "
        exit 1
    fi
    echo "⏳ Waiting for port $PORT to be released... ($WAIT_COUNT/$WAIT_MAX)"
    sleep 1
done

echo "✅ Port $PORT is clean"

# ── 2. Build backend ─────────────────────────────────────────────────
echo "🔧 Building Rust backend..."
cd "$PROJECT_DIR/backend"
cargo build --release
echo "✅ Backend built"

# ── 3. Build frontend ────────────────────────────────────────────────
echo "🔧 Building React frontend..."
cd "$PROJECT_DIR/frontend"
npm install
npm run build
echo "✅ Frontend built → $PROJECT_DIR/backend/static"

# ── 4. Deploy static files ───────────────────────────────────────────
echo ""
echo "📋 Deploying static files to /var/www/token-stats..."
sudo rm -rf /var/www/token-stats
sudo mkdir -p /var/www/token-stats
sudo cp -r "$PROJECT_DIR/backend/static/"* /var/www/token-stats/
sudo chmod -R 755 /var/www/token-stats
echo "✅ Static files deployed"

# ── 5. Install nginx config ──────────────────────────────────────────
echo ""
echo "📋 Installing nginx configuration..."
sudo cp "$PROJECT_DIR/nginx/token-stats.conf" /etc/nginx/sites-available/token-stats
sudo ln -sf /etc/nginx/sites-available/token-stats /etc/nginx/sites-enabled/token-stats
echo "✅ nginx config installed"

# ── 6. Test nginx ────────────────────────────────────────────────────
echo "🧪 Testing nginx configuration..."
sudo nginx -t
echo "✅ nginx config valid"

# ── 7. Install systemd service ───────────────────────────────────────
echo ""
echo "📋 Installing systemd service..."
sudo cp "$PROJECT_DIR/nginx/token-stats.service" /etc/systemd/system/token-stats.service

# Inject XUNFEI_SSO_SESSION_ID from current shell into systemd env drop-in
if [ -n "${XUNFEI_SSO_SESSION_ID:-}" ]; then
    sudo mkdir -p /etc/systemd/system/token-stats.service.d
    echo "[Service]" | sudo tee /etc/systemd/system/token-stats.service.d/xunfei.conf > /dev/null
    echo "Environment=\"XUNFEI_SSO_SESSION_ID=$XUNFEI_SSO_SESSION_ID\"" | sudo tee -a /etc/systemd/system/token-stats.service.d/xunfei.conf > /dev/null
    echo "✅ Injected XUNFEI_SSO_SESSION_ID into systemd env"
else
    echo "⚠️  XUNFEI_SSO_SESSION_ID not set in current shell — xunfei data will be unavailable"
fi

sudo systemctl daemon-reload
echo "✅ systemd service installed"

# ── 8. Start services ────────────────────────────────────────────────
echo ""
echo "🔄 Starting token-stats backend..."
sudo systemctl enable token-stats
sudo systemctl restart token-stats
sleep 2
sudo systemctl status token-stats --no-pager -l | head -15

echo ""
echo "🔄 Reloading nginx..."
sudo nginx -s reload

# ── 9. Verify ────────────────────────────────────────────────────────
echo ""
echo "🧪 Verifying deployment..."
sleep 1

HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" http://localhost/token-stats/ 2>/dev/null || echo "000")
if [ "$HTTP_CODE" = "200" ]; then
    echo "✅ Dashboard is LIVE at http://localhost:8081/token-stats/"
else
    echo "⚠️  Got HTTP $HTTP_CODE — checking backend..."
    curl -s http://localhost:3000/api/filters | head -3
fi

echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "📊  Dashboard:  http://localhost/token-stats/"
echo "🔧  Backend:    http://localhost:3000 (direct)"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""
echo "Useful commands:"
echo "  sudo systemctl status token-stats   # check backend"
echo "  sudo systemctl restart token-stats  # restart backend"
echo "  sudo journalctl -u token-stats -f   # view logs"
echo "  sudo nginx -t && sudo nginx -s reload  # reload nginx"