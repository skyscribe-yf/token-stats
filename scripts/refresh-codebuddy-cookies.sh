#!/usr/bin/env bash
# Refresh the CodeBuddy (www.codebuddy.cn) session cookies used by the quota card.
#
# The `session` / `session_2` cookies expire roughly every 30 days (or as soon
# as the browser session is invalidated). When they expire the backend's
# /billing/meter/* calls return 401 and the CodeBuddy card shows the error
# instead of the package list.
#
# This script:
#   1. extracts fresh cookies from the Chrome profile (extract-codebuddy-cookies.sh)
#   2. rewrites the CODEBUDDY_* lines in ~/.config/token-stats/deploy-env.sh
#   3. injects them into every active token-stats@<port> systemd drop-in
#   4. restarts those instances so the new env takes effect
#
# Usage:
#   ./scripts/refresh-codebuddy-cookies.sh            # do everything
#   ./scripts/refresh-codebuddy-cookies.sh --dry-run  # extract + report only
#
# Requires sudo for steps 3-4. Cookie values are never printed (lengths only).
set -euo pipefail

DRY_RUN=false
if [ "${1:-}" = "--dry-run" ]; then
    DRY_RUN=true
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ENV_FILE="${TOKEN_STATS_DEPLOY_ENV:-$HOME/.config/token-stats/deploy-env.sh}"

TMP_RAW="$(mktemp)"
TMP_COOKIES="$(mktemp)"
chmod 600 "$TMP_RAW" "$TMP_COOKIES"
cleanup() { rm -f "$TMP_RAW" "$TMP_COOKIES"; }
trap cleanup EXIT

echo "→ Extracting CodeBuddy cookies from Chrome..."
if ! "$SCRIPT_DIR/extract-codebuddy-cookies.sh" > "$TMP_RAW"; then
    echo "❌ Cookie extraction failed (see message above)" >&2
    exit 1
fi

python3 - "$TMP_RAW" "$TMP_COOKIES" "$ENV_FILE" <<'PY'
import re
import sys
from pathlib import Path

raw_path, cookies_path, env_path = Path(sys.argv[1]), Path(sys.argv[2]), Path(sys.argv[3])
text = raw_path.read_text()

names = ("CODEBUDDY_SESSION_COOKIE", "CODEBUDDY_SESSION_COOKIE_2")
values = {}
for name in names:
    m = re.search(rf"export {name}='([^']*)'", text)
    if not m:
        sys.exit(f"ERROR: {name} missing from extraction output")
    values[name] = m.group(1)

print("→ Extracted:")
for name, value in values.items():
    print(f"   {name}: {len(value)} chars")
for line in text.splitlines():
    if line.startswith("#"):
        print(f"   {line}")

# Rewrite deploy-env.sh in place, preserving every other line.
if env_path.exists():
    env_text = env_path.read_text()
    for name, value in values.items():
        pattern = re.compile(rf"export {name}='[^']*'")
        if not pattern.search(env_text):
            sys.exit(f"ERROR: {name} not found in {env_path}")
        env_text = pattern.sub(f"export {name}='{value}'", env_text, count=1)
    env_path.write_text(env_text)
    print(f"→ Updated {env_path}")
else:
    print(f"⚠️  {env_path} not found — skipping in-place update", file=sys.stderr)

cookies_path.write_text(
    "".join(f"export {name}='{values[name]}'\n" for name in names)
)
PY

if [ "$DRY_RUN" = true ]; then
    echo "→ Dry run: skipping systemd injection/restart."
    exit 0
fi

# shellcheck disable=SC1090
source "$TMP_COOKIES"

inject_env_dropin() {
    local instance="$1" var_name="$2" var_value="$3"
    local dropin_dir="/etc/systemd/system/${instance}.service.d"
    local dropin_file="$dropin_dir/env.conf"
    # systemd treats % as a specifier inside Environment= lines.
    local escaped="${var_value//%/%%}"

    sudo mkdir -p "$dropin_dir"
    [ -f "$dropin_file" ] || echo "[Service]" | sudo tee "$dropin_file" >/dev/null
    sudo sed -i "/^Environment=\"$var_name=/d" "$dropin_file" 2>/dev/null || true
    echo "Environment=\"$var_name=$escaped\"" | sudo tee -a "$dropin_file" >/dev/null
}

mapfile -t INSTANCES < <(
    systemctl list-units --all --plain --no-legend 'token-stats@*.service' 2>/dev/null \
        | awk '{print $1}' | sed 's/\.service$//' | sort -u
)
if [ "${#INSTANCES[@]}" -eq 0 ]; then
    echo "⚠️  No token-stats@<port> instance found; cookies only updated in $ENV_FILE"
    exit 0
fi

for instance in "${INSTANCES[@]}"; do
    echo "→ Injecting into $instance..."
    inject_env_dropin "$instance" CODEBUDDY_SESSION_COOKIE "$CODEBUDDY_SESSION_COOKIE"
    inject_env_dropin "$instance" CODEBUDDY_SESSION_COOKIE_2 "$CODEBUDDY_SESSION_COOKIE_2"
done

sudo systemctl daemon-reload
for instance in "${INSTANCES[@]}"; do
    echo "→ Restarting $instance..."
    sudo systemctl restart "$instance"
done

echo "✅ CodeBuddy cookies refreshed. Verify with:"
echo "   curl -s http://127.0.0.1:<port>/api/quota | jq .codebuddy.available"
