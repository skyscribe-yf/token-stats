#!/usr/bin/env bash
set -euo pipefail

# Apply nginx config for public access without disrupting the running backend.
# This script ONLY updates nginx configuration and reloads it — no backend restart,
# no port switch, no blue-green deployment.

PROJECT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
NGINX_CONF_SRC="$PROJECT_DIR/nginx/token-stats.conf"
NGINX_CONF_DST="/etc/nginx/sites-available/token-stats"

# Detect the currently active backend port
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
    echo "❌ No active token-stats backend found on port 3000 or 3001"
    exit 1
fi

echo "ℹ️  Active backend detected on port $ACTIVE_PORT"
echo "📋 Regenerating nginx config from template..."

# Generate config: replace upstream port while keeping everything else
sed "s|server 127.0.0.1:[0-9]*;|server 127.0.0.1:$ACTIVE_PORT;|" "$NGINX_CONF_SRC" | sudo tee "$NGINX_CONF_DST" >/dev/null

# Ensure token-stats is the default site (remove competing default)
if [ -L /etc/nginx/sites-enabled/default ]; then
    sudo rm -f /etc/nginx/sites-enabled/default
fi
sudo ln -sf "$NGINX_CONF_DST" /etc/nginx/sites-enabled/token-stats

echo "🧪 Testing nginx configuration..."
sudo nginx -t

echo "🔄 Reloading nginx gracefully..."
sudo nginx -s reload

echo "✅ Nginx config updated and reloaded."
echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "📊 Dashboard should now be accessible at:"
echo "   http://112.10.196.126/token-stats/"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""
echo "⚠️  If the site is still unreachable from the public internet,"
echo "   please verify that port 80 is allowed in your Alibaba Cloud"
echo "   security group (控制台 → 安全组 → 入方向规则 → 允许 80/tcp)."
