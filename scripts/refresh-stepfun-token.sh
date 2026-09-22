#!/usr/bin/env bash
# Refresh the StepFun console session used by the Step Plan quota half-card.
#
# Chrome's Oasis-Token cookie stores an `access...refresh` pair. The access JWT
# expires in ~30 minutes (the cookie `expires` date is only Chrome's storage
# lifetime). A deploy that re-injects a stale pair hides the monthly pool.
#
# This script:
#   1. extracts Oasis-Token / Oasis-Webid from the Chrome profile
#   2. calls PassportService/RefreshToken so the new instance starts with a live
#      access JWT plus a rotatable refresh JWT (the console only accepts the
#      combined `access...refresh` header value)
#   3. rewrites STEPFUN_OASIS_* in ~/.config/token-stats/deploy-env.sh
#   4. writes ~/.config/token-stats/stepfun-auth.json (0600) so the backend's
#      own refresher does not fall back to an older pair
#
# Usage:
#   ./scripts/refresh-stepfun-token.sh            # do everything
#   ./scripts/refresh-stepfun-token.sh --dry-run  # extract + refresh, do not write
#   ./scripts/refresh-stepfun-token.sh --env-only # steps 1-4, leave systemd alone
#     (deploy.sh injects the env file into the new instance itself)
#
# Cookie / token values are never printed (lengths only).
set -euo pipefail

SKIP_WRITE=false
case "${1:-}" in
    --dry-run)  SKIP_WRITE=true ;;
    --env-only) ;;
    "")         ;;
    *)          echo "Usage: $0 [--dry-run|--env-only]" >&2; exit 2 ;;
esac

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ENV_FILE="${TOKEN_STATS_DEPLOY_ENV:-$HOME/.config/token-stats/deploy-env.sh}"
STATE_FILE="${STEPFUN_AUTH_STATE_PATH:-$HOME/.config/token-stats/stepfun-auth.json}"

TMP_RAW="$(mktemp)"
chmod 600 "$TMP_RAW"
cleanup() { rm -f "$TMP_RAW"; }
trap cleanup EXIT

echo "→ Extracting StepFun console session from Chrome..."
if ! "$SCRIPT_DIR/extract-stepfun-token.sh" > "$TMP_RAW"; then
    echo "❌ StepFun cookie extraction failed (see message above)" >&2
    exit 1
fi

PY=""
for cand in python3 /usr/bin/python3 python3.12 /usr/bin/python3.12; do
    command -v "$cand" >/dev/null 2>&1 || continue
    PY="$cand"
    break
done
if [ -z "$PY" ]; then
    echo "ERROR: python3 not found" >&2
    exit 1
fi

"$PY" - "$TMP_RAW" "$ENV_FILE" "$STATE_FILE" "$SKIP_WRITE" <<'PY'
import json
import re
import sys
import time
import urllib.request
from pathlib import Path

raw_path, env_path, state_path, skip_write = (
    Path(sys.argv[1]),
    Path(sys.argv[2]),
    Path(sys.argv[3]),
    sys.argv[4] == "true",
)
text = raw_path.read_text()

def exported(name: str) -> str:
    match = re.search(rf"export {name}='([^']*)'", text)
    if not match or not match.group(1):
        sys.exit(f"ERROR: {name} missing from extraction output")
    return match.group(1)

token = exported("STEPFUN_OASIS_TOKEN")
webid = exported("STEPFUN_OASIS_WEBID")
print(f"→ Extracted Oasis-Token: {len(token)} chars, Oasis-Webid: {len(webid)} chars")
for line in text.splitlines():
    if line.startswith("#"):
        print(f"   {line}")

def post(url: str, oasis: str):
    req = urllib.request.Request(url, data=b"{}", method="POST")
    req.add_header("Content-Type", "application/json")
    req.add_header("Oasis-Token", oasis)
    req.add_header("Oasis-Webid", webid)
    req.add_header("Oasis-appID", "10300")
    req.add_header("Oasis-Platform", "web")
    req.add_header("Cookie", f"Oasis-Token={oasis}; Oasis-Webid={webid}")
    try:
        with urllib.request.urlopen(req, timeout=20) as resp:
            return resp.status, json.loads(resp.read().decode())
    except Exception as exc:
        body = exc.read().decode(errors="replace")[:180] if hasattr(exc, "read") else str(exc)
        return getattr(exc, "code", "ERR"), body

status, data = post(
    "https://platform.stepfun.com/passport/proto.api.passport.v1.PassportService/RefreshToken",
    token,
)
if not isinstance(data, dict):
    sys.exit(f"ERROR: StepFun RefreshToken failed ({status}): {data}")
access = (data.get("accessToken") or {}).get("raw") or ""
refresh = (data.get("refreshToken") or {}).get("raw") or ""
duration = int((data.get("accessToken") or {}).get("duration") or 0)
if not access or not refresh or duration <= 0:
    sys.exit("ERROR: StepFun RefreshToken returned an incomplete token pair")
combined = f"{access}...{refresh}"

plan_status, plan = post(
    "https://platform.stepfun.com/api/step.openapi.devcenter.Dashboard/QueryStepPlanRateLimit",
    combined,
)
left = None
if isinstance(plan, dict):
    left = (plan.get("plan_credit_rate_limit") or {}).get("subscription_credit_left_rate")
if plan_status != 200 or left is None:
    sys.exit(f"ERROR: refreshed StepFun token cannot read the plan pool ({plan_status})")
print(f"→ Refreshed Step Plan session ({duration}s access JWT, pool left {left:.1%})")

if skip_write:
    print("→ Dry run: not writing deploy-env.sh or stepfun-auth.json")
    raise SystemExit(0)

if not env_path.exists():
    sys.exit(f"ERROR: {env_path} not found (create it or set TOKEN_STATS_DEPLOY_ENV)")
env_text = env_path.read_text()
updates = {
    "STEPFUN_OASIS_TOKEN": combined,
    "STEPFUN_OASIS_WEBID": webid,
}
for name, value in updates.items():
    pattern = re.compile(rf"export {name}='[^']*'")
    if not pattern.search(env_text):
        sys.exit(f"ERROR: {name} not found in {env_path}")
    env_text = pattern.sub(f"export {name}='{value}'", env_text, count=1)
expiry = time.strftime("%Y-%m-%d %H:%M:%S", time.localtime(time.time() + duration))
env_text = re.sub(
    r"# Oasis-Token expires: .*",
    f"# Oasis-Token access JWT expires: {expiry} (refresh rotates it)",
    env_text,
    count=1,
)
env_path.write_text(env_text)
print(f"→ Updated {env_path}")

state_path.parent.mkdir(parents=True, exist_ok=True)
tmp = state_path.with_suffix(".json.tmp")
tmp.write_text(json.dumps({
    "access_token": access,
    "refresh_token": refresh,
    "webid": webid,
    "expires_at": int(time.time()) + duration,
}, indent=2) + "\n")
tmp.chmod(0o600)
tmp.replace(state_path)
state_path.chmod(0o600)
print(f"→ Wrote {state_path} (0600)")
PY
