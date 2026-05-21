#!/usr/bin/env bash
set -e

echo "🚀 Token Stats Dashboard Setup"
echo "=============================="

# Detect home directory
HOME_DIR="$HOME"
PROJECT_DIR="$HOME_DIR/srcs/token-stats"
BACKEND_DIR="$PROJECT_DIR/backend"
NGINX_DIR="$PROJECT_DIR/nginx"

echo ""
echo "📁 Project directory: $PROJECT_DIR"

# Check prerequisites
echo ""
echo "🔍 Checking prerequisites..."

if ! command -v rustc &> /dev/null; then
    echo "❌ Rust not found. Please install Rust: https://rustup.rs"
    exit 1
fi

if ! command -v node &> /dev/null; then
    echo "❌ Node.js not found. Please install Node.js"
    exit 1
fi

if ! command -v nginx &> /dev/null; then
    echo "❌ nginx not found. Please install nginx"
    exit 1
fi

echo "✅ All prerequisites met"

# Build backend
echo ""
echo "🔧 Building Rust backend..."
cd "$BACKEND_DIR"
cargo build --release

# Build frontend
echo ""
echo "🔧 Building React frontend..."
cd "$PROJECT_DIR/frontend"
npm install
npm run build

echo "✅ Frontend built to $BACKEND_DIR/static"

# Setup nginx
echo ""
echo "🔧 Setting up nginx..."

NGINX_CONF="$NGINX_DIR/token-stats.conf"
NGINX_SITES_DIR=""

# Detect nginx sites directory
if [ -d /etc/nginx/sites-available ]; then
    NGINX_SITES_DIR="/etc/nginx/sites-available"
elif [ -d /etc/nginx/conf.d ]; then
    NGINX_SITES_DIR="/etc/nginx/conf.d"
fi

if [ -n "$NGINX_SITES_DIR" ]; then
    echo "📋 Installing nginx config to $NGINX_SITES_DIR"
    
    # Update the root path in the config
    sed "s|/home/skyscribe/srcs/token-stats|$PROJECT_DIR|g" "$NGINX_CONF" > "/tmp/token-stats-nginx.conf"
    
    if [ -d /etc/nginx/sites-available ]; then
        sudo cp "/tmp/token-stats-nginx.conf" "$NGINX_SITES_DIR/token-stats"
        sudo ln -sf "$NGINX_SITES_DIR/token-stats" /etc/nginx/sites-enabled/token-stats 2>/dev/null || true
    else
        sudo cp "/tmp/token-stats-nginx.conf" "$NGINX_SITES_DIR/token-stats.conf"
    fi
    
    # Create log directories
    sudo mkdir -p /var/log/nginx
    
    # Test nginx config
    echo "🧪 Testing nginx configuration..."
    sudo nginx -t
    
    echo "🔄 Reloading nginx..."
    sudo nginx -s reload
else
    echo "⚠️ Could not detect nginx sites directory."
    echo "   Please manually copy $NGINX_CONF to your nginx configuration."
fi

# Setup systemd service
echo ""
echo "🔧 Setting up systemd service..."

SERVICE_FILE="$NGINX_DIR/token-stats@.service"
TMP_SERVICE="/tmp/token-stats@.service"

sed -e "s|/home/skyscribe/srcs/token-stats|$PROJECT_DIR|g" \
    -e "s|User=skyscribe|User=$(whoami)|g" \
    "$SERVICE_FILE" > "$TMP_SERVICE"

if [ -d /etc/systemd/system ]; then
    sudo cp "$TMP_SERVICE" /etc/systemd/system/token-stats@.service
    sudo systemctl daemon-reload
    echo "✅ Systemd template service installed"
    echo ""
    echo "   Start the service: sudo systemctl start token-stats@3000"
    echo "   Enable on boot:    sudo systemctl enable token-stats@3000"
else
    echo "⚠️ Could not install systemd service."
    echo "   You can run the backend manually:"
    echo "   cd $BACKEND_DIR && PORT=3000 ./target/release/token-stats-backend"
fi

echo ""
echo "🎉 Setup complete!"
echo ""
echo "📊 Access your dashboard at: http://localhost:8081"
echo ""
echo "Useful commands:"
echo "  sudo systemctl start token-stats   # Start backend"
echo "  sudo systemctl stop token-stats    # Stop backend"
echo "  sudo systemctl status token-stats  # Check status"
echo "  sudo nginx -t                      # Test nginx config"
echo "  sudo nginx -s reload               # Reload nginx"
echo ""
