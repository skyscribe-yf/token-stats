#!/usr/bin/env python3
"""
One-shot migration: reclassify legacy DeepSeek-export records.

Before this migration, scripts/recover_token_data.py mapped
api_key_name="opencode" to (provider=opencode-go, source=opencode).
Those records actually represent DeepSeek API usage (billed by
DeepSeek's platform with OpenCode's API key) and should be
(provider=deepseek, source=deepseek-ai), matching the other
DeepSeek-export records.

This script rewrites ~/.pi/token-logs/usage.jsonl in place:
  - finds records with apiKeyPrefix starting "deepseek-export:opencode"
  - sets provider="deepseek", source="deepseek-ai"
  - leaves cost=0 (display_cost computes from tokens via pricing.toml)
A backup is written to usage.jsonl.bak.YYYYMMDD before any changes.

Usage:
  python3 migrate_deepseek_export_classification.py [--dry-run]
"""

import json
import os
import shutil
import sys
from datetime import datetime

USAGE_JSONL = os.path.expanduser("~/.pi/token-logs/usage.jsonl")
TARGET_PREFIX = "deepseek-export:opencode"


def main():
    dry_run = "--dry-run" in sys.argv
    if not os.path.exists(USAGE_JSONL):
        print(f"ERROR: {USAGE_JSONL} not found")
        sys.exit(1)

    affected = []
    new_lines = []
    with open(USAGE_JSONL) as f:
        for line in f:
            stripped = line.strip()
            if not stripped:
                new_lines.append(line)
                continue
            try:
                obj = json.loads(stripped)
            except json.JSONDecodeError:
                new_lines.append(line)
                continue

            api_key_prefix = obj.get("apiKeyPrefix", "")
            if api_key_prefix.startswith(TARGET_PREFIX):
                affected.append(
                    {
                        "date": obj.get("date"),
                        "old_provider": obj.get("provider"),
                        "old_source": obj.get("source"),
                        "total_tokens": obj.get("totalTokens"),
                    }
                )
                obj["provider"] = "deepseek"
                obj["source"] = "deepseek-ai"
                new_lines.append(json.dumps(obj, ensure_ascii=False) + "\n")
            else:
                new_lines.append(line)

    print(f"Found {len(affected)} records to reclassify:")
    total_tokens = 0
    for record in affected:
        print(
            f"  {record['date']}: {record['old_source']}/{record['old_provider']} "
            f"-> deepseek-ai/deepseek ({record['total_tokens']:,} tokens)"
        )
        total_tokens += record["total_tokens"] or 0
    print(f"Total tokens to be reclassified: {total_tokens:,}")

    if not affected:
        print("Nothing to migrate.")
        return

    if dry_run:
        print("DRY RUN — no changes made. Run without --dry-run to apply.")
        return

    backup_path = f"{USAGE_JSONL}.bak.{datetime.now():%Y%m%d}"
    shutil.copyfile(USAGE_JSONL, backup_path)
    print(f"Backup written to {backup_path}")

    tmp_path = f"{USAGE_JSONL}.tmp"
    with open(tmp_path, "w") as f:
        f.writelines(new_lines)
    os.rename(tmp_path, USAGE_JSONL)
    print(f"Updated {USAGE_JSONL} in place.")


if __name__ == "__main__":
    main()
