#!/usr/bin/env bash
# Extract the StepFun console session (Oasis-Token / Oasis-Webid cookies of
# platform.stepfun.com) from a running-or-closed Google Chrome profile and
# print `export` lines for STEPFUN_OASIS_TOKEN / STEPFUN_OASIS_WEBID.
#
# The dashboard's Step Plan quota card calls the console Connect-RPC
# (step.openapi.devcenter.Dashboard/QueryStepPlanRateLimit +
# GetStepPlanStatus), which requires BOTH headers — the gateway cross-checks
# the token against the webid ("oasis-token is embezzled" otherwise).
#
# Usage:
#   ./scripts/extract-stepfun-token.sh              # print export lines
#
# Requires: python3 with `dbus-python` (GNOME Keyring "Chrome Safe Storage"
# key) plus `pycryptodome`/`pycryptodomex` — same interpreter auto-detect as
# extract-codebuddy-cookies.sh. Cookie values are never printed.
#
# The cookie `expires` date is Chrome's storage lifetime (about a year). The
# Oasis-Token JWT itself expires in ~30 minutes (`exp` claim). The backend
# renews it via PassportService/RefreshToken and stores the rotated pair in
# ~/.config/token-stats/stepfun-auth.json — re-run this script when that
# refresh fails (the quota card then shows plan_error instead of the pool).
set -euo pipefail

CHROME_COOKIES="${CHROME_COOKIES:-$HOME/.config/google-chrome/Default/Cookies}"

PY=""
for cand in python3 /usr/bin/python3 python3.12 /usr/bin/python3.12 python3.11 /usr/bin/python3.11; do
    command -v "$cand" >/dev/null 2>&1 || continue
    "$cand" -c 'import dbus' >/dev/null 2>&1 || continue
    if "$cand" -c 'import Crypto.Cipher' >/dev/null 2>&1 ||
       "$cand" -c 'import Cryptodome.Cipher' >/dev/null 2>&1; then
        PY="$cand"
        break
    fi
done
if [ -z "$PY" ]; then
    echo "ERROR: no python3 with dbus-python + pycryptodome(x) found on PATH" >&2
    exit 1
fi

"$PY" - "$CHROME_COOKIES" << 'PYEOF'
import datetime
import hashlib
import shutil
import sqlite3
import sys
import tempfile

try:
    from Crypto.Cipher import AES
except ImportError:
    from Cryptodome.Cipher import AES

import dbus

HOST = b"platform.stepfun.com"
KEYRING_LABEL = "Chrome Safe Storage"
NAMES = {"Oasis-Token": "STEPFUN_OASIS_TOKEN", "Oasis-Webid": "STEPFUN_OASIS_WEBID"}

db_path = sys.argv[1]
tmp = tempfile.NamedTemporaryFile(suffix=".db", delete=False)
tmp.close()
shutil.copy(db_path, tmp.name)

conn = sqlite3.connect(tmp.name)
rows = conn.execute(
    "SELECT name, encrypted_value, expires_utc FROM cookies "
    "WHERE host_key=? AND name IN ({})".format(",".join("?" * len(NAMES))),
    (HOST.decode(), *NAMES),
).fetchall()

bus = dbus.SessionBus()
service_obj = bus.get_object("org.freedesktop.secrets", "/org/freedesktop/secrets")
service = dbus.Interface(service_obj, "org.freedesktop.Secret.Service")

unlocked, locked = service.SearchItems({})
key = None
for path in list(unlocked) + list(locked):
    props = dbus.Interface(
        bus.get_object("org.freedesktop.secrets", path),
        "org.freedesktop.DBus.Properties",
    )
    try:
        label = str(props.Get("org.freedesktop.Secret.Item", "Label"))
    except Exception:
        continue
    if label != KEYRING_LABEL:
        continue
    try:
        session_path = service.OpenSession("plain", dbus.String(""))[1]
        session = dbus.Interface(
            bus.get_object("org.freedesktop.secrets", str(session_path)),
            "org.freedesktop.Secret.Session",
        )
        secrets = service.GetSecrets([path], session)
        key = bytes(bytearray(secrets[path][2]))
    except Exception as exc:
        print(f"WARNING: could not read keyring item {path}: {exc}", file=sys.stderr)
        continue
    if key:
        break

if not key:
    sys.exit(f"ERROR: '{KEYRING_LABEL}' not found (or empty) in GNOME Keyring")

dec_key = hashlib.pbkdf2_hmac("sha1", key, b"saltysalt", 1, 16)

out = {}
for name, enc, expires in rows:
    cipher = AES.new(dec_key, AES.MODE_CBC, IV=b" " * 16)
    d = cipher.decrypt(bytes(enc)[3:])
    d = d[:-d[-1]]
    if d[:32] != hashlib.sha256(HOST).digest():
        sys.exit(f"ERROR: decryption failed for cookie {name}")
    expiry = datetime.datetime(1601, 1, 1) + datetime.timedelta(microseconds=expires)
    out[name] = (d[32:].decode("utf-8"), expiry)

missing = set(NAMES) - set(out)
if missing:
    sys.exit(
        f"ERROR: cookies not found in Chrome profile: {', '.join(sorted(missing))} "
        "(log in to platform.stepfun.com first)"
    )

for name, var in NAMES.items():
    value, expiry = out[name]
    print(f"export {var}='{value}'")
    if name == "Oasis-Token":
        print(f"# Oasis-Token expires: {expiry:%Y-%m-%d %H:%M:%S}")

conn.close()
PYEOF
