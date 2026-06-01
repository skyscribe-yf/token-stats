#!/usr/bin/env bash
set -euo pipefail

# Apply nginx config for qiantangai.com domain + WSL2 forwarding.
# This ONLY updates nginx configuration and reloads it — no backend restart,
# no port switch, no blue-green deployment.

PROJECT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
NGINX_CONF_SRC="$PROJECT_DIR/nginx/token-stats.conf"
NGINX_CONF_DST="/etc/nginx/sites-available/token-stats"

# Detect the currently active local backend port
ACTIVE_PORT=""
for p in 3000 3001; do
    if systemctl is-active --quiet "token-stats@$p" 2>/dev/null; then
        ACTIVE_PORT="$p"
        break
    fi
done

# Fallback: check which port is actually listening
if [ -z "$ACTIVE_PORT" ]; then
    for p in 3000 3001; do
        if ss -tlnp 2>/dev/null | grep -q ":$p\b"; then
            ACTIVE_PORT="$p"
            break
        fi
    done
fi

if [ -z "$ACTIVE_PORT" ]; then
    echo "⚠️  No active local token-stats backend found; defaulting to 3000"
    ACTIVE_PORT=3000
fi

echo "ℹ️  Active local backend detected on port $ACTIVE_PORT"
echo "📋 Regenerating nginx config from template..."

# Generate config: replace upstream port while keeping everything else
sed "s|server 127.0.0.1:3000;|server 127.0.0.1:$ACTIVE_PORT;|" "$NGINX_CONF_SRC" | sudo tee "$NGINX_CONF_DST" >/dev/null

echo "🧪 Testing nginx configuration..."
sudo nginx -t

echo "🔄 Reloading nginx gracefully..."
sudo nginx -s reload

echo "✅ Nginx config updated and reloaded."
echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "📊 Local dashboard:  http://localhost/token-stats/"
echo "🌐 Domain dashboard: http://qiantangai.com/token-stats/"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""
echo "⚠️  Make sure the SSH tunnel from WSL2 is active:"
echo "   ssh -N -R 3002:localhost:80 skyscribe@47.96.139.133"
echo ""
echo "   Or use autossh for automatic reconnection:"
echo "   autossh -M 0 -N -R 3002:localhost:80 skyscribe@47.96.139.133"
echo ""
echo "   (Port 80 is the local nginx which handles blue-green upstream switching.")"
echo "   (DO NOT tunnel directly to backend port 3000 — that breaks blue-green.)"
