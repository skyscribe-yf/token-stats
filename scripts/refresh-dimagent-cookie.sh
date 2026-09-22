#!/usr/bin/env bash
# Refresh the DimAgent console session cookie used by the `dim` data source.
#
# The `session` cookie for dimagent.cn lapses roughly 30 days after the last
# browser login. When it goes stale the console API returns 401 and the `dim`
# source stops advancing — silently, because 401 is a graceful-degradation path.
#
# Why this matters more than a broken quota card: Dim bills its own OAuth
# channel (`dimcode-api-oauth`) *through* the console API. The local dimcode
# SQLite supplement deliberately excludes that provider to avoid double-counting,
# so while the cookie is dead every Dim-OAuth call (e.g. deepseek-v4.1-flash)
# falls into a hole — it exists in neither source. History stays, new traffic
# disappears.
#
# This script:
#   1. extracts a fresh cookie from the Chrome profile (extract-dimagent-cookie.sh)
#   2. verifies it live against the console API, so a second stale cookie can
#      never overwrite a working entry
#   3. rewrites the DIMAGENT_SESSION_COOKIE line in ~/.config/token-stats/deploy-env.sh
#   4. injects it into every active token-stats@<port> systemd drop-in
#   5. restarts those instances so the new env takes effect
#
# Usage:
#   ./scripts/refresh-dimagent-cookie.sh            # do everything
#   ./scripts/refresh-dimagent-cookie.sh --dry-run  # extract + verify + report only
#   ./scripts/refresh-dimagent-cookie.sh --env-only # steps 1-3, leave systemd alone
#     (for deploys: deploy.sh injects the env file into the new instance itself)
#
# Requires sudo for steps 4-5. Cookie values are never printed (lengths only).
set -euo pipefail

SKIP_SYSTEMD=false
case "${1:-}" in
    --dry-run)  SKIP_SYSTEMD=true ;;
    --env-only) SKIP_SYSTEMD=true ;;
    "")         ;;
    *)          echo "Usage: $0 [--dry-run|--env-only]" >&2; exit 2 ;;
esac

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ENV_FILE="${TOKEN_STATS_DEPLOY_ENV:-$HOME/.config/token-stats/deploy-env.sh}"
API_URL="${DIMAGENT_COOKIE_PROBE_URL:-https://dimagent.cn/api/log/self?p=1&page_size=1&type=2}"

TMP_RAW="$(mktemp)"
TMP_COOKIE="$(mktemp)"
chmod 600 "$TMP_RAW" "$TMP_COOKIE"
cleanup() { rm -f "$TMP_RAW" "$TMP_COOKIE"; }
trap cleanup EXIT

echo "→ Extracting DimAgent cookie from Chrome..."
if ! "$SCRIPT_DIR/extract-dimagent-cookie.sh" > "$TMP_RAW"; then
    echo "❌ Cookie extraction failed (see message above)" >&2
    exit 1
fi

python3 - "$TMP_RAW" "$TMP_COOKIE" "$ENV_FILE" <<'PY'
import re
import sys
from pathlib import Path

raw_path, cookie_path, env_path = Path(sys.argv[1]), Path(sys.argv[2]), Path(sys.argv[3])
text = raw_path.read_text()

m = re.search(r"export DIMAGENT_SESSION_COOKIE='([^']*)'", text)
if not m:
    sys.exit("ERROR: DIMAGENT_SESSION_COOKIE missing from extraction output")
value = m.group(1)
if not value:
    # An empty cookie would blank a working entry and stop the dim source.
    sys.exit("ERROR: DIMAGENT_SESSION_COOKIE extracted empty")

expiry = re.search(r"^# (session expires: .*)$", text, re.M)

print("→ Extracted:")
print(f"   DIMAGENT_SESSION_COOKIE: {len(value)} chars")
if expiry:
    print(f"   # {expiry.group(1)}")

if not env_path.exists():
    # A missing file means nothing would be refreshed — fail loudly rather than
    # letting a deploy proceed with whatever env the caller happens to have.
    sys.exit(f"ERROR: {env_path} not found (create it or set TOKEN_STATS_DEPLOY_ENV)")

# The file has historically stored this value unquoted; normalise to single
# quotes so a future cookie that does contain shell metacharacters cannot be
# silently truncated (CodeBuddy's cookie bit us exactly that way).
env_text = env_path.read_text()
pattern = re.compile(r"^export DIMAGENT_SESSION_COOKIE=('([^']*)'|(\S*))", re.M)
if not pattern.search(env_text):
    sys.exit(f"ERROR: DIMAGENT_SESSION_COOKIE not found in {env_path}")
previous = pattern.search(env_text).group(2) or pattern.search(env_text).group(3)
env_text = pattern.sub(lambda _: f"export DIMAGENT_SESSION_COOKIE='{value}'", env_text, count=1)
env_path.write_text(env_text)
print(f"→ Updated {env_path}")
if previous == value:
    print("   (value unchanged — the stored cookie was already the browser's current one)")

cookie_path.write_text(f"export DIMAGENT_SESSION_COOKIE='{value}'\n")
PY

# shellcheck disable=SC1090
source "$TMP_COOKIE"

echo "→ Verifying the cookie against the DimAgent console API..."
probe="$(curl -s -m 20 -o /dev/null -w '%{http_code}' \
    -H "Cookie: session=${DIMAGENT_SESSION_COOKIE}" "$API_URL" || echo 000)"
if [ "$probe" != "200" ]; then
    echo "❌ Console API returned HTTP $probe for the freshly extracted cookie." >&2
    echo "   Chrome's own session is stale too — log in at" >&2
    echo "   https://dimagent.cn/console/activity in Chrome, then re-run." >&2
    echo "   $ENV_FILE was updated but the value is not working; nothing was restarted." >&2
    exit 1
fi
echo "✅ Cookie verified (HTTP 200)"

if [ "$SKIP_SYSTEMD" = true ]; then
    if [ "${1:-}" = "--env-only" ]; then
        echo "→ Deploy mode: new instance gets the cookie from $ENV_FILE via deploy.sh."
    else
        echo "→ Dry run: skipping systemd injection/restart."
    fi
    exit 0
fi

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
    echo "⚠️  No token-stats@<port> instance found; cookie only updated in $ENV_FILE"
    exit 0
fi

for instance in "${INSTANCES[@]}"; do
    echo "→ Injecting into $instance..."
    inject_env_dropin "$instance" DIMAGENT_SESSION_COOKIE "$DIMAGENT_SESSION_COOKIE"
done

sudo systemctl daemon-reload
for instance in "${INSTANCES[@]}"; do
    echo "→ Restarting $instance..."
    sudo systemctl restart "$instance"
done

echo "✅ DimAgent cookie refreshed. Verify with:"
echo "   curl -s 'http://127.0.0.1:<port>/api/requests?source=dim&page=1&limit=3&tz_offset=480' | jq '.data[0].time'"
