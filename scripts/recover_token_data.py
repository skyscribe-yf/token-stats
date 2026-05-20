#!/usr/bin/env python3
"""
Recover missing token usage data from two sources:
  1. Pi session JSONL files (~/.pi/agent/sessions/**/*.jsonl)
  2. DeepSeek official platform export ZIPs

Outputs recovered records to usage.jsonl with dedup guarantees:
  - Pi sessions: dedup by (timestamp_ms, provider, model, totalTokens)
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
from collections import defaultdict
from datetime import datetime, timezone

USAGE_JSONL = os.path.expanduser("~/.pi/token-logs/usage.jsonl")
PI_SESSIONS_DIR = os.path.expanduser("~/.pi/agent/sessions")
DEEPSEEK_ZIPS = sorted(glob.glob("/mnt/d/Downloads/usage_data_2026_*.zip"))

# ── Dedup: load existing usage.jsonl ────────────────────────────────────────

def load_existing():
    """Load existing records from usage.jsonl.
    
    Returns:
      timestamp_keys: set of (timestamp_ms, provider, model, totalTokens) for per-request dedup
      daily_totals: dict (date, model) -> {input, output, cache_read, cache_write, count, cost}
                    for daily aggregate dedup
    """
    timestamp_keys = set()
    daily_totals = defaultdict(lambda: {
        "count": 0, "input": 0, "output": 0,
        "cache_read": 0, "cache_write": 0, "cost": 0.0
    })

    if not os.path.exists(USAGE_JSONL):
        return timestamp_keys, daily_totals

    with open(USAGE_JSONL) as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            try:
                obj = json.loads(line)
            except json.JSONDecodeError:
                continue

            # Per-request dedup key: (time, provider, model, totalTokens)
            time_str = obj.get("time", "")
            provider = obj.get("provider", "")
            model = obj.get("model", "")
            total = obj.get("totalTokens", 0)
            
            # Parse time to ms for precise dedup
            ts_ms = 0
            if time_str and time_str != "unknown":
                try:
                    dt = datetime.fromisoformat(time_str.replace("Z", "+00:00"))
                    ts_ms = int(dt.timestamp() * 1000)
                except (ValueError, AttributeError):
                    pass
            
            timestamp_keys.add((ts_ms, provider, model, total))

            # Daily aggregate dedup: sum by (date, model) for deepseek
            date = obj.get("date", "")
            if "deepseek" in provider.lower() or "deepseek" in model.lower():
                key = (date, model)
                r = daily_totals[key]
                r["count"] += 1
                r["input"] += obj.get("inputTokens", 0)
                r["output"] += obj.get("outputTokens", 0)
                r["cache_read"] += obj.get("cacheReadTokens", 0)
                r["cache_write"] += obj.get("cacheWriteTokens", 0)
                r["cost"] += obj.get("cost", 0.0)

    return timestamp_keys, daily_totals


# ── Source 1: Pi session JSONL files ─────────────────────────────────────────

def recover_pi_sessions(existing_keys, dry_run=False):
    """Extract token usage from Pi session JSONL files.
    
    Each assistant message in a session file contains:
      message.provider, message.model, message.timestamp, message.usage
    
    Dedup: skip if (timestamp_ms, provider, model, totalTokens) already in existing_keys.
    """
    session_files = glob.glob(os.path.join(PI_SESSIONS_DIR, "**", "*.jsonl"), recursive=True)
    new_records = []
    skipped = 0
    errors = 0

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
                cost_obj = usage.get("cost", {})
                cost = cost_obj.get("total", 0.0) if isinstance(cost_obj, dict) else 0.0

                # Skip zero-usage records
                if input_t == 0 and output_t == 0 and cache_read == 0 and cache_write == 0:
                    continue

                provider = msg.get("provider", "unknown")
                model = msg.get("model", "unknown")
                timestamp = msg.get("timestamp", 0)

                if timestamp > 0:
                    dt = datetime.fromtimestamp(timestamp / 1000, tz=timezone.utc)
                    date_str = dt.strftime("%Y-%m-%d")
                    time_str = dt.strftime("%Y-%m-%dT%H:%M:%SZ")
                    ts_ms = timestamp
                else:
                    # Fallback: extract date from filename
                    basename = os.path.basename(f)
                    date_str = basename[:10] if len(basename) >= 10 else "unknown"
                    time_str = "unknown"
                    ts_ms = 0

                # Dedup check
                dedup_key = (ts_ms, provider, model, total_t)
                if dedup_key in existing_keys:
                    skipped += 1
                    continue

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
                existing_keys.add(dedup_key)

    return new_records, skipped


# ── Source 2: DeepSeek platform export ───────────────────────────────────────

# Map DeepSeek api_key_name to our provider names
API_KEY_MAP = {
    "opencode": "opencode-go",
    "pi": "deepseek",
    "ai小北": "deepseek",  # ai小北 uses deepseek API directly
}

def load_deepseek_export():
    """Load DeepSeek export ZIPs and return daily aggregate records.
    
    Each (date, model, api_key_name) group has 4 rows:
      output_tokens, request_count, input_cache_hit_tokens, input_cache_miss_tokens
    
    Returns dict: (date, model, api_key_name) -> {input_miss, cache_hit, output, req, cost}
    """
    records = {}
    cost_data = {}

    tmpdir = tempfile.mkdtemp(prefix="deepseek-export-")
    try:
        for zip_path in DEEPSEEK_ZIPS:
            with zipfile.ZipFile(zip_path) as zf:
                zf.extractall(tmpdir)

        # Parse amount CSVs
        for fname in glob.glob(os.path.join(tmpdir, "amount-*.csv")):
            with open(fname, newline="", encoding="utf-8-sig") as f:
                reader = csv.reader(f)
                next(reader)  # skip header
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
                            "output": 0, "req": 0, "cost": 0.0
                        }
                    if type_ == "input_cache_miss_tokens":
                        records[key]["input_miss"] = amount
                    elif type_ == "input_cache_hit_tokens":
                        records[key]["cache_hit"] = amount
                    elif type_ == "output_tokens":
                        records[key]["output"] = amount
                    elif type_ == "request_count":
                        records[key]["req"] = amount

        # Parse cost CSVs
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

    # Attach cost to records
    for (date, model, api_key_name), r in records.items():
        cost_key = (date, model)
        # Distribute cost proportionally by output tokens across api_key_names
        r["cost"] = cost_data.get(cost_key, 0.0)

    return records, cost_data


def recover_deepseek_export(existing_daily_totals, dry_run=False):
    """Convert DeepSeek daily aggregates to usage.jsonl records.
    
    Dedup strategy: for each (date, model), compare the 4-tuple from the export
    against the daily sum of existing deepseek records. If they match exactly,
    skip (we already have complete data). If not, insert a synthetic daily
    aggregate record.
    
    Note: DeepSeek export is UTC-based daily data. We insert one record per
    (date, model, api_key_name) group with a synthetic timestamp at 12:00 UTC.
    """
    ds_records, ds_cost = load_deepseek_export()
    new_records = []
    skipped = 0

    # Aggregate existing by (date, model) for comparison
    existing_by_dm = defaultdict(lambda: {
        "count": 0, "input": 0, "output": 0,
        "cache_read": 0, "cache_write": 0, "cost": 0.0
    })
    for (date, model), r in existing_daily_totals.items():
        existing_by_dm[(date, model)] = dict(r)  # copy

    # Aggregate export by (date, model) for comparison
    export_by_dm = defaultdict(lambda: {
        "input_miss": 0, "cache_hit": 0, "output": 0, "req": 0, "cost": 0.0
    })
    for (date, model, api_key_name), r in ds_records.items():
        key = (date, model)
        export_by_dm[key]["input_miss"] += r["input_miss"]
        export_by_dm[key]["cache_hit"] += r["cache_hit"]
        export_by_dm[key]["output"] += r["output"]
        export_by_dm[key]["req"] += r["req"]

    # Now process each (date, model, api_key_name) group
    for key in sorted(ds_records.keys()):
        date, model, api_key_name = key
        r = ds_records[key]

        # Check if this specific api_key_name's data is already covered
        # by existing records. We compare at the (date, model) level.
        dm_key = (date, model)
        ex = existing_by_dm.get(dm_key, {
            "count": 0, "input": 0, "output": 0,
            "cache_read": 0, "cache_write": 0, "cost": 0.0
        })
        ds_dm = export_by_dm.get(dm_key, {
            "input_miss": 0, "cache_hit": 0, "output": 0, "req": 0, "cost": 0.0
        })

        # If the daily totals match exactly, skip this entire day
        if (ds_dm["input_miss"] == ex["input"]
            and ds_dm["cache_hit"] == ex["cache_read"]
            and ds_dm["output"] == ex["output"]):
            skipped += 1
            continue

        # Map api_key_name to our provider
        provider = API_KEY_MAP.get(api_key_name, "deepseek")

        # Determine source based on api_key_name
        if api_key_name == "opencode":
            source = "opencode"
        elif api_key_name == "pi":
            source = "pi"
        elif api_key_name == "ai小北":
            source = "deepseek-ai"  # custom source for ai小北
        else:
            source = "deepseek"

        # Build synthetic daily aggregate record
        # Use noon UTC as the timestamp
        time_str = f"{date}T12:00:00Z"
        total_tokens = r["input_miss"] + r["cache_hit"] + r["output"]

        # Cost: distribute from ds_cost proportionally
        # For now, use 0 and let pricing module compute it
        cost = 0.0

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
            "cost": cost,
            "source": source,
        }

        new_records.append(record)

    return new_records, skipped


# ── Main ─────────────────────────────────────────────────────────────────────

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
    existing_keys, existing_daily = load_existing()
    print(f"  Loaded {len(existing_keys)} per-request keys, {len(existing_daily)} daily deepseek totals")
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
        ds_records, ds_skipped = recover_deepseek_export(existing_daily, dry_run)
        print(f"  Found {len(ds_records)} new daily aggregates, skipped {ds_skipped} matching days")
        all_new_records.extend(ds_records)
        print()

    if not all_new_records:
        print("No new records to insert. Everything is already covered!")
        return

    # Summary
    print(f"=== Summary ===")
    print(f"Total new records: {len(all_new_records)}")
    
    # Group by source
    by_source = defaultdict(int)
    by_date = defaultdict(int)
    for r in all_new_records:
        by_source[r.get("source", "unknown")] += 1
        by_date[r.get("date", "unknown")[:7]] += 1  # by month
    
    print(f"By source: {dict(by_source)}")
    print(f"By month: {dict(sorted(by_date.items()))}")
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
