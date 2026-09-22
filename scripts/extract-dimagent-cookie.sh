#!/usr/bin/env bash
# Extract the DimAgent console (dimagent.cn) `session` cookie from a
# running-or-closed Google Chrome profile and print an `export` line ready to be
# sourced by the token-stats env import file.
#
# Usage:
#   ./scripts/extract-dimagent-cookie.sh              # print export line
#   ./scripts/extract-dimagent-cookie.sh >> ~/.config/token-stats/env.sh
#
# Requires: python3 with `dbus-python` for the GNOME Keyring "Chrome Safe
# Storage" key plus either `pycryptodome` (Crypto) or `pycryptodomex`
# (Cryptodome) for AES-CBC. The interpreter is auto-detected because the
# default `python3` on PATH may be a venv that lacks them.
#
# The cookie DB is copied before reading, so Chrome may keep running.
# NOTE: the cookie expires roughly 30 days after the last browser login — re-run
# scripts/refresh-dimagent-cookie.sh when the `dim` source stops advancing
# (its console-API polling is the only per-request meter for Dim's OAuth channel).
set -euo pipefail

CHROME_COOKIES="${CHROME_COOKIES:-$HOME/.config/google-chrome/Default/Cookies}"

# Pick the first interpreter that has dbus-python and an AES implementation.
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

HOST = b"dimagent.cn"
KEYRING_LABEL = "Chrome Safe Storage"

db_path = sys.argv[1]
tmp = tempfile.NamedTemporaryFile(suffix=".db", delete=False)
tmp.close()
shutil.copy(db_path, tmp.name)

conn = sqlite3.connect(tmp.name)
cur = conn.cursor()
rows = cur.execute(
    "SELECT name, encrypted_value, expires_utc FROM cookies "
    "WHERE host_key=? AND name='session'",
    (HOST.decode(),),
).fetchall()

# ── Chrome Safe Storage key from the default keyring collection ──────────
# secretstorage is not installed here, so talk to the Secret Service directly.
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
        # Secret struct: (session, parameters, value, content_type)
        key = bytes(bytearray(secrets[path][2]))
    except Exception as exc:
        print(f"WARNING: could not read keyring item {path}: {exc}", file=sys.stderr)
        continue
    if key:
        break

if not key:
    sys.exit(f"ERROR: '{KEYRING_LABEL}' not found (or empty) in GNOME Keyring")

# Same KDF Chrome uses: PBKDF2-HMAC-SHA1(password, "saltysalt", 1, 16 bytes)
dec_key = hashlib.pbkdf2_hmac("sha1", key, b"saltysalt", 1, 16)

out = {}
for name, enc, expires in rows:
    cipher = AES.new(dec_key, AES.MODE_CBC, IV=b" " * 16)
    d = cipher.decrypt(bytes(enc)[3:])
    d = d[:-d[-1]]
    # v11 format: SHA256(host) prefix + ciphertext
    if d[:32] != hashlib.sha256(HOST).digest():
        sys.exit(f"ERROR: decryption failed for cookie {name}")
    expiry = datetime.datetime(1601, 1, 1) + datetime.timedelta(microseconds=expires)
    out[name] = (d[32:].decode("utf-8"), expiry)

if "session" not in out:
    sys.exit("ERROR: cookie 'session' not found for dimagent.cn in the Chrome profile "
             f"({db_path}) — log in at https://dimagent.cn/console/activity first")

value, expiry = out["session"]
if not value:
    sys.exit("ERROR: extracted an empty session cookie")

# NOTE: the cookie value embeds `|` separators (Flask signed-session format:
# payload|timestamp|signature), so it MUST be quoted — an unquoted export line
# makes bash treat `|` as a pipe operator and the value silently truncates at
# the first separator.
print(f"export DIMAGENT_SESSION_COOKIE='{value}'")
print(f"# session expires: {expiry:%Y-%m-%d %H:%M:%S}")

conn.close()
PYEOF
