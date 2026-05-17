#!/usr/bin/env bash
# One-shot setup script — run this in your terminal with sudo access
set -e

PROJECT_DIR="$HOME/srcs/token-stats"

echo "🚀 Token Stats Dashboard — Nginx + Systemd Setup"
echo "================================================="
echo ""

# ── 1. Build backend (if needed) ──────────────────────────────────────
if [ ! -f "$PROJECT_DIR/backend/target/release/token-stats-backend" ]; then
    echo "🔧 Building Rust backend..."
    cd "$PROJECT_DIR/backend"
    cargo build --release
    echo "✅ Backend built"
else
    echo "✅ Backend already built"
fi

# ── 2. Build frontend (if needed) ────────────────────────────────────
if [ ! -f "$PROJECT_DIR/backend/static/index.html" ]; then
    echo "🔧 Building React frontend..."
    cd "$PROJECT_DIR/frontend"
    npm install
    npm run build
    echo "✅ Frontend built → $PROJECT_DIR/backend/static"
else
    echo "✅ Frontend already built"
fi

# ── 3. Install nginx config ──────────────────────────────────────────
echo ""
echo "📋 Installing nginx configuration..."
sudo cp "$PROJECT_DIR/nginx/token-stats.conf" /etc/nginx/sites-available/token-stats
sudo ln -sf /etc/nginx/sites-available/token-stats /etc/nginx/sites-enabled/token-stats
echo "✅ nginx config installed to /etc/nginx/sites-available/token-stats"

# ── 4. Test nginx ────────────────────────────────────────────────────
echo "🧪 Testing nginx configuration..."
sudo nginx -t
echo "✅ nginx config valid"

# ── 5. Install systemd service ───────────────────────────────────────
echo ""
echo "📋 Installing systemd service..."
sudo cp "$PROJECT_DIR/nginx/token-stats.service" /etc/systemd/system/token-stats.service
sudo systemctl daemon-reload
echo "✅ systemd service installed"

# ── 6. Start services ────────────────────────────────────────────────
echo ""
echo "🔄 Starting token-stats backend..."
sudo systemctl enable token-stats
sudo systemctl start token-stats
sleep 2
sudo systemctl status token-stats --no-pager -l | head -15

echo ""
echo "🔄 Reloading nginx..."
sudo nginx -s reload

# ── 7. Verify ────────────────────────────────────────────────────────
echo ""
echo "🧪 Verifying deployment..."
sleep 1

HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" http://localhost:8080/ 2>/dev/null || echo "000")
if [ "$HTTP_CODE" = "200" ]; then
    echo "✅ Dashboard is LIVE at http://localhost:8080"
else
    echo "⚠️  Got HTTP $HTTP_CODE — check that the backend is running:"
    echo "   sudo systemctl status token-stats"
fi

echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "📊  Dashboard:  http://localhost:8080"
echo "🔧  Backend:    http://localhost:3000 (direct)"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""
echo "Useful commands:"
echo "  sudo systemctl status token-stats   # check backend"
echo "  sudo systemctl restart token-stats  # restart backend"
echo "  sudo journalctl -u token-stats -f   # view logs"
echo "  sudo nginx -t                       # test nginx config"
echo "  sudo nginx -s reload                # reload nginx"