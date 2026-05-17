#!/usr/bin/env bash
# Quick start script for development/testing

set -e

PROJECT_DIR="$(cd "$(dirname "$0")" && pwd)"
BACKEND_DIR="$PROJECT_DIR/backend"

echo "🚀 Starting Token Stats Dashboard"
echo "================================="

# Check if backend binary exists
if [ ! -f "$BACKEND_DIR/target/release/token-stats-backend" ]; then
    echo "❌ Backend not built. Building now..."
    cd "$BACKEND_DIR"
    cargo build --release
fi

# Check if static files exist
if [ ! -f "$BACKEND_DIR/static/index.html" ]; then
    echo "❌ Frontend not built. Building now..."
    cd "$PROJECT_DIR/frontend"
    npm install
    npm run build
fi

echo ""
echo "📊 Starting server..."
cd "$BACKEND_DIR"
RUST_LOG=info ./target/release/token-stats-backend
