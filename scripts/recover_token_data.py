#!/usr/bin/env python3
"""
Recover missing token usage data from two sources:
  1. Pi session JSONL files (~/.pi/agent/sessions/**/*.jsonl)
  2. DeepSeek official platform export ZIPs

Outputs recovered records to usage.jsonl with dedup guarantees:
  - Pi sessions: Counter-based dedup using (date, provider, model, totalTokens, inputTokens, outputTokens)
    This handles both real-timestamp and midnight-timestamp providers correctly.
  - DeepSeek export: dedup by comparing daily 4-tuple
    (input_cache_miss, input_cache_hit, output_tokens, request_count)
    against existing daily totals for the same (date, model)

Usage:
  python3 recover_token_data.py [--dry-run] [--pi] [--deepseek]
"""

import json
import csv
import glob
import os
import sys
import zipfile
import tempfile
import shutil
from collections import defaultdict, Counter
from datetime import datetime, timezone

USAGE_JSONL = os.path.expanduser("~/.pi/token-logs/usage.jsonl")
PI_SESSIONS_DIR = os.path.expanduser("~/.pi/agent/sessions")
DEEPSEEK_ZIPS = sorted(glob.glob("/mnt/d/Downloads/usage_data_2026_*.zip"))


def load_existing():
    """Load existing records from usage.jsonl for dedup.
    
    Returns:
      record_keys: Counter of (date, provider, model, total, input, output) for per-request dedup
      daily_totals: dict (date, model) -> {input, output, cache_read, cache_write, count, cost}
                    for DeepSeek export daily aggregate dedup
    """
    record_keys = Counter()
    daily_totals = defaultdict(lambda: {
        "count": 0, "input": 0, "output": 0,
        "cache_read": 0, "cache_write": 0, "cost": 0.0
    })

    if not os.path.exists(USAGE_JSONL):
        return record_keys, daily_totals

    with open(USAGE_JSONL) as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            try:
                obj = json.loads(line)
            except json.JSONDecodeError:
                continue

            provider = obj.get("provider", "")
            model = obj.get("model", "")
            total = obj.get("totalTokens", 0)
            date = obj.get("date", "")
            input_t = obj.get("inputTokens", 0)
            output_t = obj.get("outputTokens", 0)

            # Per-request dedup key
            key = (date, provider, model, total, input_t, output_t)
            record_keys[key] += 1

            # Daily aggregate dedup for DeepSeek export
            if "deepseek" in provider.lower() or "deepseek" in model.lower():
                dm_key = (date, model)
                r = daily_totals[dm_key]
                r["count"] += 1
                r["input"] += input_t
                r["output"] += output_t
                r["cache_read"] += obj.get("cacheReadTokens", 0)
                r["cache_write"] += obj.get("cacheWriteTokens", 0)
                r["cost"] += obj.get("cost", 0.0)

    return record_keys, daily_totals


def recover_pi_sessions(existing_keys, dry_run=False):
    """Extract token usage from Pi session JSONL files.
    
    Dedup: Counter-based matching using (date, provider, model, total, input, output).
    For each key, we track how many existing records have that key and how many
    session records we've already matched. A session record is considered duplicate
    only if we haven't exceeded the count of existing records with that key.
    """
    session_files = glob.glob(os.path.join(PI_SESSIONS_DIR, "**", "*.jsonl"), recursive=True)
    new_records = []
    skipped = 0
    matched = Counter()  # Track how many of each key we've matched against existing

    for f in session_files:
        with open(f) as fh:
            for line in fh:
                line = line.strip()
                if not line:
                    continue
                try:
                    obj = json.loads(line)
                except json.JSONDecodeError:
                    continue

                if obj.get("type") != "message":
                    continue
                msg = obj.get("message", {})
                if msg.get("role") != "assistant":
                    continue

                usage = msg.get("usage", {})
                if not usage:
                    continue

                input_t = usage.get("input", 0)
                output_t = usage.get("output", 0)
                cache_read = usage.get("cacheRead", 0)
                cache_write = usage.get("cacheWrite", 0)
                total_t = usage.get("totalTokens", 0)

                # Skip zero-usage records
                if input_t == 0 and output_t == 0 and cache_read == 0 and cache_write == 0:
                    continue

                provider = msg.get("provider", "unknown")
                model = msg.get("model", "unknown")
                timestamp = msg.get("timestamp", 0)

                if timestamp > 0:
                    dt = datetime.fromtimestamp(timestamp / 1000, tz=timezone.utc)
                    date_str = dt.strftime("%Y-%m-%d")
                    # Preserve sub-second precision in time field
                    time_str = dt.strftime("%Y-%m-%dT%H:%M:%S.") + f"{timestamp % 1000:03d}Z"
                else:
                    basename = os.path.basename(f)
                    date_str = basename[:10] if len(basename) >= 10 else "unknown"
                    time_str = "unknown"

                # Dedup check: Counter-based matching
                key = (date_str, provider, model, total_t, input_t, output_t)
                if matched[key] < existing_keys.get(key, 0):
                    matched[key] += 1
                    skipped += 1
                    continue

                cost_obj = usage.get("cost", {})
                cost = cost_obj.get("total", 0.0) if isinstance(cost_obj, dict) else 0.0

                # Build usage.jsonl record
                record = {
                    "date": date_str,
                    "time": time_str,
                    "apiKeyPrefix": "session-recovery",
                    "provider": provider,
                    "model": model,
                    "inputTokens": input_t,
                    "outputTokens": output_t,
                    "cacheReadTokens": cache_read,
                    "cacheWriteTokens": cache_write,
                    "totalTokens": total_t,
                    "cost": cost,
                    "source": "pi",
                }

                new_records.append(record)
                # Track the new record in our matched counter
                matched[key] += 1

    return new_records, skipped


# ── Source 2: DeepSeek platform export ───────────────────────────────────────

# Map DeepSeek export api_key_name -> (provider, source).
# All DeepSeek export rows describe calls billed directly by DeepSeek's
# official platform - the api_key_name is just the channel that owned
# the key. Classify them all as provider=deepseek, source=deepseek-ai
# so the dashboard treats them uniformly and pricing.rs computes their
# cost from pricing.toml deepseek rates (CNY native, no OpenCode divisor).
API_KEY_MAP = {
    "opencode": "deepseek",
    "pi": "deepseek",
    "ai小北": "deepseek",
}

SOURCE_MAP = {
    "opencode": "deepseek-ai",
    "pi": "pi",
    "ai小北": "deepseek-ai",
}


def load_deepseek_export():
    """Load DeepSeek export ZIPs and return daily aggregate records."""
    records = {}
    cost_data = {}

    tmpdir = tempfile.mkdtemp(prefix="deepseek-export-")
    try:
        for zip_path in DEEPSEEK_ZIPS:
            with zipfile.ZipFile(zip_path) as zf:
                zf.extractall(tmpdir)

        for fname in glob.glob(os.path.join(tmpdir, "amount-*.csv")):
            with open(fname, newline="", encoding="utf-8-sig") as f:
                reader = csv.reader(f)
                next(reader)
                for row in reader:
                    if len(row) < 8:
                        continue
                    utc_date = row[1]
                    model = row[2]
                    api_key_name = row[3]
                    type_ = row[5]
                    amount = int(row[7])

                    key = (utc_date, model, api_key_name)
                    if key not in records:
                        records[key] = {
                            "input_miss": 0, "cache_hit": 0,
                            "output": 0, "req": 0
                        }
                    if type_ == "input_cache_miss_tokens":
                        records[key]["input_miss"] = amount
                    elif type_ == "input_cache_hit_tokens":
                        records[key]["cache_hit"] = amount
                    elif type_ == "output_tokens":
                        records[key]["output"] = amount
                    elif type_ == "request_count":
                        records[key]["req"] = amount

        for fname in glob.glob(os.path.join(tmpdir, "cost-*.csv")):
            with open(fname, newline="", encoding="utf-8-sig") as f:
                reader = csv.reader(f)
                next(reader)
                for row in reader:
                    if len(row) < 5:
                        continue
                    utc_date = row[1]
                    model = row[2]
                    cost = float(row[4])
                    key = (utc_date, model)
                    if key not in cost_data:
                        cost_data[key] = 0.0
                    cost_data[key] += cost
    finally:
        shutil.rmtree(tmpdir, ignore_errors=True)

    return records, cost_data


def recover_deepseek_export(existing_daily_totals, existing_dm_pairs, dry_run=False):
    """Convert DeepSeek daily aggregates to usage.jsonl records.
    
    Conservative dedup: only add records for (date, model) combinations
    where there are NO existing records at all. This avoids double-counting
    with Pi session records or original usage.jsonl records.
    """
    ds_records, ds_cost = load_deepseek_export()
    new_records = []
    skipped = 0

    # Process each (date, model, api_key_name) group
    for key in sorted(ds_records.keys()):
        date, model, api_key_name = key
        r = ds_records[key]

        # Skip if we already have ANY records for this (date, model)
        dm_key = (date, model)
        if dm_key in existing_dm_pairs:
            skipped += 1
            continue

        provider = API_KEY_MAP.get(api_key_name, "deepseek")
        source = SOURCE_MAP.get(api_key_name, "deepseek")

        # Build synthetic daily aggregate record
        time_str = f"{date}T12:00:00Z"
        total_tokens = r["input_miss"] + r["cache_hit"] + r["output"]

        record = {
            "date": date,
            "time": time_str,
            "apiKeyPrefix": f"deepseek-export:{api_key_name}",
            "provider": provider,
            "model": model,
            "inputTokens": r["input_miss"],
            "outputTokens": r["output"],
            "cacheReadTokens": r["cache_hit"],
            "cacheWriteTokens": 0,
            "totalTokens": total_tokens,
            "cost": 0.0,  # Let pricing module compute
            "source": source,
        }

        new_records.append(record)

    return new_records, skipped


def main():
    dry_run = "--dry-run" in sys.argv
    has_flag = any(a.startswith("--") for a in sys.argv[1:] if a != "--dry-run")
    do_pi = "--pi" in sys.argv or (not has_flag)
    do_deepseek = "--deepseek" in sys.argv or (not has_flag)

    print(f"Recovery mode: {'DRY RUN' if dry_run else 'LIVE'}")
    print(f"Sources: pi={do_pi}, deepseek={do_deepseek}")
    print()

    # Load existing data for dedup
    print("Loading existing usage.jsonl for dedup...")
    existing_keys, daily_totals = load_existing()
    print(f"  {sum(existing_keys.values())} existing records, {len(existing_keys)} unique keys")
    print(f"  {len(daily_totals)} daily deepseek totals")
    
    # Also compute existing (date, model) pairs for deepseek
    existing_dm_pairs = set()
    for (date, model) in daily_totals.keys():
        existing_dm_pairs.add((date, model))
    print(f"  {len(existing_dm_pairs)} existing (date, model) pairs for deepseek")
    print()

    all_new_records = []

    # Source 1: Pi sessions
    if do_pi:
        print("Recovering from Pi session JSONL files...")
        pi_records, pi_skipped = recover_pi_sessions(existing_keys, dry_run)
        print(f"  Found {len(pi_records)} new records, skipped {pi_skipped} duplicates")
        all_new_records.extend(pi_records)
        print()

    # Source 2: DeepSeek export
    if do_deepseek:
        print("Recovering from DeepSeek platform export...")
        ds_records, ds_skipped = recover_deepseek_export(daily_totals, existing_dm_pairs, dry_run)
        print(f"  Found {len(ds_records)} new daily aggregates, skipped {ds_skipped} matching days")
        all_new_records.extend(ds_records)
        print()

    if not all_new_records:
        print("No new records to insert. Everything is already covered!")
        return

    # Summary
    print(f"=== Summary ===")
    print(f"Total new records: {len(all_new_records)}")

    by_source = defaultdict(int)
    by_date = defaultdict(int)
    by_provider = defaultdict(int)
    for r in all_new_records:
        by_source[r.get("source", "unknown")] += 1
        by_date[r.get("date", "unknown")[:10]] += 1
        by_provider[r.get("provider", "unknown")] += 1

    print(f"By source: {dict(by_source)}")
    print(f"By provider: {dict(by_provider)}")
    print(f"By date:")
    for d in sorted(by_date.keys()):
        print(f"  {d}: {by_date[d]}")
    print()

    # Preview first 5 records
    print("Preview (first 5 records):")
    for r in all_new_records[:5]:
        print(f"  {r['date']} | {r['provider']}/{r['model']} | "
              f"in={r['inputTokens']} out={r['outputTokens']} "
              f"cache={r['cacheReadTokens']} total={r['totalTokens']}")
    if len(all_new_records) > 5:
        print(f"  ... and {len(all_new_records) - 5} more")
    print()

    if dry_run:
        print("DRY RUN - no changes made. Run without --dry-run to apply.")
        return

    # Append to usage.jsonl
    print(f"Appending {len(all_new_records)} records to {USAGE_JSONL}...")
    with open(USAGE_JSONL, "a") as f:
        for r in all_new_records:
            f.write(json.dumps(r, ensure_ascii=False) + "\n")

    print("Done! Records appended successfully.")
    print(f"Run 'wc -l {USAGE_JSONL}' to verify.")


if __name__ == "__main__":
    main()
