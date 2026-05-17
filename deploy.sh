#!/usr/bin/env bash
set -e

PROJECT_DIR="$HOME/srcs/token-stats"

echo "🚀 Token Stats Dashboard — Deploy"
echo "=================================="
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

# ── 3. Deploy static files ───────────────────────────────────────────
echo ""
echo "📋 Deploying static files to /var/www/token-stats..."
sudo rm -rf /var/www/token-stats
sudo mkdir -p /var/www/token-stats
sudo cp -r "$PROJECT_DIR/backend/static/"* /var/www/token-stats/
sudo chmod -R 755 /var/www/token-stats
echo "✅ Static files deployed"

# ── 4. Install nginx config ──────────────────────────────────────────
echo ""
echo "📋 Installing nginx configuration..."
sudo cp "$PROJECT_DIR/nginx/token-stats.conf" /etc/nginx/sites-available/token-stats
sudo ln -sf /etc/nginx/sites-available/token-stats /etc/nginx/sites-enabled/token-stats
echo "✅ nginx config installed"

# ── 5. Test nginx ────────────────────────────────────────────────────
echo "🧪 Testing nginx configuration..."
sudo nginx -t
echo "✅ nginx config valid"

# ── 6. Install systemd service ───────────────────────────────────────
echo ""
echo "📋 Installing systemd service..."
sudo cp "$PROJECT_DIR/nginx/token-stats.service" /etc/systemd/system/token-stats.service
sudo systemctl daemon-reload
echo "✅ systemd service installed"

# ── 7. Start services ────────────────────────────────────────────────
echo ""
echo "🔄 Starting token-stats backend..."
sudo systemctl enable token-stats
sudo systemctl restart token-stats
sleep 2
sudo systemctl status token-stats --no-pager -l | head -15

echo ""
echo "🔄 Reloading nginx..."
sudo nginx -s reload

# ── 8. Verify ────────────────────────────────────────────────────────
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