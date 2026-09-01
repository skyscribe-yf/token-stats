#!/usr/bin/env bash
# 安装 quota-widget@token-stats 到当前用户扩展目录并启用。
# Wayland 下首次安装需注销重新登录一次才能加载；之后可反复运行本脚本更新。
set -euo pipefail

UUID="quota-widget@token-stats"
SRC_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DEST="${XDG_DATA_HOME:-$HOME/.local/share}/gnome-shell/extensions/$UUID"

mkdir -p "$DEST"
cp -r "$SRC_DIR/metadata.json" "$SRC_DIR/extension.js" "$SRC_DIR/prefs.js" "$SRC_DIR/stylesheet.css" "$SRC_DIR/schemas" "$DEST/"
glib-compile-schemas "$DEST/schemas/"

echo "已安装到 $DEST"

if command -v gnome-extensions >/dev/null 2>&1; then
    if gnome-extensions info "$UUID" >/dev/null 2>&1; then
        gnome-extensions enable "$UUID" 2>/dev/null || true
        if gnome-extensions info "$UUID" 2>/dev/null | grep -q "State: ACTIVE"; then
            echo "扩展已启用。"
        else
            echo "扩展已安装，请用「扩展管理器」启用（或注销重登后再试）。"
        fi
    else
        echo "GNOME Shell 尚未识别该扩展（Wayland 首次安装需注销重新登录），登录后运行："
        echo "  gnome-extensions enable $UUID"
    fi
fi
