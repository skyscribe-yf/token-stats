#!/bin/bash
# sync-ollama-models.sh — 把 Ollama Cloud 的模型目录同步进 CLIProxyAPI 配置。
#
# 用法:
#   ./scripts/sync-ollama-models.sh                 # 写入并原地更新
#   ./scripts/sync-ollama-models.sh --dry-run       # 只打印将要写入的模型列表
#   CPA_CONFIG=/path/to/config.yaml ./scripts/sync-ollama-models.sh
#
# 背景: CLIProxyAPI 的 openai-compatibility 条目必须显式列出 models，否则该 provider
# 不暴露任何模型（/v1/models 返回空、调用报 "unknown provider for model ..."）。
# Ollama 上新模型后跑一次本脚本，CPA 会自动热重载配置。
#
# 认证: 上游 api key 从 config.yaml 里已有的 ollama-cloud 条目读取（不打印、不改写）。
set -euo pipefail

CPA_CONFIG="${CPA_CONFIG:-$HOME/workbuddy-proxy/config.yaml}"
PROVIDER_NAME="${OLLAMA_PROXY_PROVIDER_NAME:-ollama-cloud}"
MODELS_URL="${OLLAMA_MODELS_URL:-https://ollama.com/v1/models}"
DRY_RUN=0

for arg in "$@"; do
  case "$arg" in
    --dry-run) DRY_RUN=1 ;;
    -h|--help) sed -n '2,14p' "$0"; exit 0 ;;
    *) echo "未知参数: $arg" >&2; exit 2 ;;
  esac
done

if [[ ! -f "$CPA_CONFIG" ]]; then
  echo "找不到 CLIProxyAPI 配置: $CPA_CONFIG" >&2
  exit 1
fi

python3 - "$CPA_CONFIG" "$PROVIDER_NAME" "$MODELS_URL" "$DRY_RUN" <<'PY'
import json
import re
import sys
import urllib.request

config_path, provider_name, models_url, dry_run = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4] == "1"
text = open(config_path, encoding="utf-8").read()
lines = text.split("\n")

# ── Locate the provider entry and its api key ────────────────────────────────
entry_re = re.compile(r"^(\s*)- name:\s*[\"']?%s[\"']?\s*$" % re.escape(provider_name))
start = next((i for i, l in enumerate(lines) if entry_re.match(l)), None)
if start is None:
    sys.exit("配置里没有名为 %r 的 openai-compatibility 条目" % provider_name)
indent = re.match(r"^(\s*)", lines[start]).group(1)

# Entry ends at the next list item at the same indent, or EOF.
end = len(lines)
item_re = re.compile(r"^%s- " % re.escape(indent))
for i in range(start + 1, len(lines)):
    if item_re.match(lines[i]):
        end = i
        break

entry = lines[start:end]
key_re = re.compile(r"^\s*- api-key:\s*[\"']?([^\"'\s]+)[\"']?\s*$")
api_key = next((m.group(1) for l in entry if (m := key_re.match(l))), None)
if not api_key:
    sys.exit("条目 %r 下没找到 api-key，无法拉取模型列表" % provider_name)

# ── Fetch the upstream catalog ──────────────────────────────────────────────
req = urllib.request.Request(models_url, headers={"Authorization": "Bearer " + api_key})
with urllib.request.urlopen(req, timeout=30) as resp:
    payload = json.load(resp)
models = sorted({m["id"] for m in payload.get("data", []) if m.get("id")})
if not models:
    sys.exit("上游未返回任何模型，拒绝写入空列表")
print("上游模型数: %d" % len(models))

# ── Compare with what is already configured ─────────────────────────────────
models_idx = next((i for i, l in enumerate(entry) if re.match(r"^\s*models:\s*$", l)), None)
existing = []
if models_idx is not None:
    for l in entry[models_idx + 1:]:
        m = re.match(r"^\s*- name:\s*[\"']?([^\"'\s]+)[\"']?\s*$", l)
        if m:
            existing.append(m.group(1))
        elif l.strip() and not l.strip().startswith("#"):
            break

if existing == models:
    print("已是最新，无需改动。")
    sys.exit(0)

added = [m for m in models if m not in existing]
removed = [m for m in existing if m not in models]
print("新增 %d 个: %s" % (len(added), ", ".join(added) or "-"))
print("移除 %d 个: %s" % (len(removed), ", ".join(removed) or "-"))

if dry_run:
    print("\n--dry-run，未写入。将要写入的模型列表:")
    for m in models:
        print("      - name: \"%s\"" % m)
    sys.exit(0)

# ── Rewrite the models block in place, preserving every other line ──────────
block = ["%s  models:" % indent] + ['%s    - name: "%s"' % (indent, m) for m in models]
if models_idx is None:
    new_entry = entry + block
else:
    tail = models_idx + 1 + len(existing)
    new_entry = entry[:models_idx] + block + entry[tail:]

out = "\n".join(lines[:start] + new_entry + lines[end:])
open(config_path, "w", encoding="utf-8").write(out)
print("已写入 %s（CLIProxyAPI 会自动热重载）" % config_path)
PY
