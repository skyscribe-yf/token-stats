#!/usr/bin/env bash
# Extract CodeBuddy (www.codebuddy.cn) `session` / `session_2` cookies from a
# running-or-closed Google Chrome profile and print `export` lines ready to be
# sourced by the token-stats env import file.
#
# Usage:
#   ./scripts/extract-codebuddy-cookies.sh              # print export lines
#   ./scripts/extract-codebuddy-cookies.sh >> ~/.config/token-stats/env.sh
#
# Requires: python3 with pycryptodome + secretstorage (both preinstalled on
# this machine). The cookie DB is copied before reading, so Chrome may keep
# running. NOTE: the cookies expire every ~30 days of last browser login —
# re-run this script and redeploy when the CodeBuddy card starts failing.
set -euo pipefail

CHROME_COOKIES="${CHROME_COOKIES:-$HOME/.config/google-chrome/Default/Cookies}"

python3 - "$CHROME_COOKIES" << 'PYEOF'
import datetime
import hashlib
import json
import shutil
import sqlite3
import sys
import tempfile

from Crypto.Cipher import AES
from Crypto.Protocol.KDF import PBKDF2

db_path = sys.argv[1]
tmp = tempfile.NamedTemporaryFile(suffix=".db", delete=False)
tmp.close()
shutil.copy(db_path, tmp.name)

conn = sqlite3.connect(tmp.name)
cur = conn.cursor()
rows = cur.execute(
    "SELECT name, encrypted_value, expires_utc FROM cookies "
    "WHERE host_key='www.codebuddy.cn' AND name IN ('session','session_2')"
).fetchall()

# Chrome Safe Storage key lives in the default secret collection (GNOME Keyring)
import secretstorage
bus = secretstorage.dbus_init()
collection = secretstorage.get_default_collection(bus)
if collection.is_locked():
    collection.unlock()
key = None
for item in collection.get_all_items():
    if item.get_label() == "Chrome Safe Storage":
        key = item.get_secret()
if key is None:
    sys.exit("ERROR: 'Chrome Safe Storage' not found in GNOME Keyring")

dec_key = PBKDF2(key, b"saltysalt", dkLen=16, count=1)

out = {}
for name, enc, expires in rows:
    cipher = AES.new(dec_key, AES.MODE_CBC, IV=b" " * 16)
    d = cipher.decrypt(enc[3:])
    d = d[:-d[-1]]
    # v11 format: SHA256(host) prefix + ciphertext
    if d[:32] != hashlib.sha256(b"www.codebuddy.cn").digest():
        sys.exit(f"ERROR: decryption failed for cookie {name}")
    expiry = datetime.datetime(1601, 1, 1) + datetime.timedelta(microseconds=expires)
    out[name] = (d[32:].decode("utf-8"), expiry)

missing = {"session", "session_2"} - set(out)
if missing:
    sys.exit(f"ERROR: cookies not found in Chrome profile: {', '.join(sorted(missing))}")

for name in ("session", "session_2"):
    value, expiry = out[name]
    var = "CODEBUDDY_SESSION_COOKIE" if name == "session" else "CODEBUDDY_SESSION_COOKIE_2"
    print(f'export {var}={value}')
    print(f'# {name} expires: {expiry:%Y-%m-%d %H:%M:%S}')

conn.close()
PYEOF
