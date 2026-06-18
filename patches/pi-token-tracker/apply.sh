#!/usr/bin/env bash
# Apply the pi-token-tracker advisor logging patch.
#
# The pi-token-tracker package is installed at:
#   ~/.pi/agent/npm/node_modules/pi-token-tracker/
#
# This patch adds a `tool_execution_end` handler that captures
# advisor LLM call token usage and writes it to usage.jsonl.
#
# Usage:
#   ./apply.sh              # apply the patch
#   ./apply.sh --revert     # revert to original
#   ./apply.sh --diff       # show diff
#
# Note: Restart pi after applying for changes to take effect.

set -euo pipefail

DIR="$(cd "$(dirname "$0")" && pwd)"
TARGET="$HOME/.pi/agent/npm/node_modules/pi-token-tracker/token-tracker.ts"
ORIG="$DIR/token-tracker.ts.orig"
MODIFIED="$DIR/token-tracker.ts"

case "${1:-}" in
  --revert)
    if [[ -f "$ORIG" ]]; then
      cp "$ORIG" "$TARGET"
      echo "✓ Reverted to original token-tracker.ts"
      echo "  Restart pi for changes to take effect."
    else
      echo "✗ Original backup not found at $ORIG"
      exit 1
    fi
    ;;
  --diff)
    if [[ -f "$ORIG" && -f "$MODIFIED" ]]; then
      diff -u "$ORIG" "$MODIFIED" || true
    fi
    ;;
  *)
    # First ensure the original is backed up
    if [[ ! -f "$ORIG" ]]; then
      # Try downloading from npm
      echo "Downloading original from npm..."
      TMPDIR=$(mktemp -d)
      (cd "$TMPDIR" && npm pack pi-token-tracker > /dev/null 2>&1 && tar -xzf pi-token-tracker-*.tgz && cp package/token-tracker.ts "$ORIG")
      rm -rf "$TMPDIR"
    fi

    if [[ -f "$MODIFIED" ]]; then
      cp "$MODIFIED" "$TARGET"
      echo "✓ Applied advisor logging patch to token-tracker.ts"
      echo "  Restart pi for changes to take effect."
    else
      echo "✗ Patched file not found at $MODIFIED"
      exit 1
    fi
    ;;
esac
