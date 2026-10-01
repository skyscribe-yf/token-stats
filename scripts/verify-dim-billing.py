#!/usr/bin/env python3
# verify-dim-billing.py — 用平台自己的账本核对 dim 源的计价。
#
# 为什么需要它：dim 源**不存原始 cost**（`display_cost()` 走"衍生源"分支），
# 所以 `[[dim_model]]` 写错一个数字不会报错，只会让成本静默偏掉。已踩过的两次：
#   1. `mimo-v2.6-flash` 2026-09-28 之前的结算口径与公开价目卡字段顺序**相反**
#      （output 2.8 / cache_read 280），照价目卡登记会低估 16.9 倍；
#   2. 同一个模型平台在 2026-09-29 又改成价目卡口径（output 280 / cache_read 2.8），
#      只登记单一费率会让当天成本偏高 30.3 倍——所以它必须保留 `effective_from` 分段。
# 结论：**任何新模型 / 任何改价，都要用本脚本按账本复核，别只信价目卡。**
#
# 用法:
#   ./scripts/verify-dim-billing.py                       # 全部 dim 模型，最近 14 天（按天）
#   ./scripts/verify-dim-billing.py --interval hourly     # 按小时桶，能验证当天（无需等隔日结算）
#   ./scripts/verify-dim-billing.py --both                # 天 + 小时都跑
#   ./scripts/verify-dim-billing.py --model mimo-v2.6-flash --days 30
#   ./scripts/verify-dim-billing.py --json
#   ./scripts/verify-dim-billing.py --cny-per-credit 139/22200   # 换套餐口径
#
# 退出码: 0 = 所有已结算桶都在容差内；1 = 有偏差 / 有未登记价格的模型 / 记录有洞。
#
# 语义与 Rust 侧严格对齐（改动时两处一起改）:
#   - 模型选段: 同名 `[[dim_model]]` 按 effective_from 分段，取记录时刻之前最新一段
#     (pricing.rs `ModelPrice::select_segment`)
#   - 忙时双倍: `peak_hours_utc` 半开区间 [start, end)，`peak_weekdays_only` 时周末不翻倍
#     (pricing.rs `is_peak_hour`；dim 侧**不要**设 peak_weekdays_only，见 pitfalls 21)
#   - entitlement 折扣: `dim-entitlement.json` 取记录时刻之前最新一段的 rate，
#     再乘以命中的最深窗口折扣 (dim_entitlement.rs `rate_for`)
#   - 全站闲时窗口: `[[special.dim_offpeak_windows]]`（国庆等）命中时一律按基础价，
#     忙时双倍不生效 (pricing.rs `in_offpeak_window` → `force_off_peak`)
#   - 平台按 **CST(UTC+8)** 分桶；账本的 `prompt_tokens` **含 cache**，
#     对应我们已减去 cache 的 `input_tokens`
#   - 结算滞后: 只取该桶**前 request_count 条**记录与账本比对，
#     避免在途请求与落账延迟造成假偏差
#
# 凭据: cookie 取 `DIMAGENT_SESSION_COOKIE`，回退解析
# `~/.config/token-stats/deploy-env.sh`（scripts/refresh-dimagent-cookie.sh 写的就是这里）。
# cookie 过期时本脚本会明确报错，而不是静默跳过。

import argparse
import calendar
import json
import os
import sqlite3
import sys
import time
import tomllib
import urllib.error
import urllib.request

CONSOLE_API = "https://dimagent.cn/api"
CST_OFFSET = 8 * 3600
DEFAULT_CNY_PER_CREDIT = 70.0 / 11000.0  # Lite 套餐 ¥70/11000 积分，见 pricing.toml
TOLERANCE = 0.001  # 0.1%；残差来自平台按请求取整到毫积分


# ─── 时间 ────────────────────────────────────────────────────────────────────

def parse_ts(ts: str) -> float:
    """RFC3339 → epoch 秒；裸 "YYYY-MM-DD" 按 00:00 CST 解析（与 pricing.rs 同）。"""
    if len(ts) == 10:
        return calendar.timegm(time.strptime(ts, "%Y-%m-%d")) - CST_OFFSET
    base = calendar.timegm(time.strptime(ts[:19], "%Y-%m-%dT%H:%M:%S"))
    tail = ts[19:]
    if tail[:1] in ("+", "-"):
        sign = 1 if tail[0] == "+" else -1
        hh, _, mm = tail[1:].partition(":")
        base -= sign * (int(hh) * 3600 + int(mm or 0) * 60)
    return float(base)


def cst_key(ts: str, interval: str) -> str:
    """账本桶键：daily = CST 日期，hourly = CST 日期 + 小时。"""
    shifted = time.gmtime(parse_ts(ts) + CST_OFFSET)
    day = time.strftime("%Y-%m-%d", shifted)
    return day if interval == "daily" else f"{day} {shifted.tm_hour:02d}:00"


# ─── 计价（镜像 pricing.rs / dim_entitlement.rs）─────────────────────────────

def pick_config(entries: list, ts: str) -> dict:
    """记录时刻之前最新的一段；全部未生效时回落最早一段。

    对应 pricing.rs `ModelPrice::select_segment`（段按 effective_from 升序，
    基线段无 effective_from 恒生效，取最后一个满足条件者）。
    """
    moment = parse_ts(ts)
    chosen = None
    for e in entries:
        eff = e.get("effective_from")
        if eff is None or parse_ts(eff) <= moment:
            chosen = e
    return chosen or entries[0]


def load_dim_offpeak() -> list:
    """`[[special.dim_offpeak_windows]]` → 已解析的 [from, to) 区间。

    对应 pricing.rs `in_offpeak_window`：平台全站闲时窗口（国庆等），窗口内
    一律按基础价，`peak_hours_utc` 的忙时双倍不生效。
    """
    path = os.environ.get("PRICING_CONFIG") or repo_path("backend", "pricing.toml")
    with open(path, "rb") as fh:
        cfg = tomllib.load(fh)
    out = []
    for w in cfg.get("special", {}).get("dim_offpeak_windows", []) or []:
        start = parse_ts(w["from"])
        end = parse_ts(w["to"]) if w.get("to") else None
        out.append((start, end, w["from"], w.get("to")))
    return out


def in_offpeak(windows: list, ts: str) -> bool:
    """记录时刻是否落在任一全站闲时窗口内（半开区间）。"""
    moment = parse_ts(ts)
    return any(start <= moment and (end is None or moment < end)
               for start, end, _, _ in windows)


def rate_of(cfg: dict, base_key: str, peak_key: str, peak: bool) -> float:
    """忙时取 peak_*（缺省回落基础价），否则取基础价。

    对应 pricing.rs `tier.peak_input_cny.or(tier.input_cny)`。
    """
    if peak:
        peak_value = cfg.get(peak_key)
        if peak_value is not None:
            return peak_value
    return cfg.get(base_key, 0.0)


def is_peak(cfg: dict, ts: str, windows: list | None = None) -> bool:
    # 平台全站闲时窗口（国庆等）优先：窗口内一律不算忙时，对应 pricing.rs 里
    # `compute_cny(..., force_off_peak)` 的短路。
    if windows and in_offpeak(windows, ts):
        return False
    hours = cfg.get("peak_hours_utc") or []
    if not hours:
        return False
    hour = time.gmtime(parse_ts(ts)).tm_hour
    if cfg.get("peak_weekdays_only") and time.gmtime(parse_ts(ts)).tm_wday >= 5:
        return False
    for start, end in hours:
        if (start <= end and start <= hour < end) or (start > end and (hour >= start or hour < end)):
            return True
    return False


def rate_for(segments: list, ts: str) -> float:
    """entitlement 折扣 = 段 rate × 命中最深窗口 rate；无段落为 1.0。"""
    moment = parse_ts(ts)
    seg = None
    for s in segments:
        if moment >= parse_ts(s["effective_from"]):
            seg = s
    if seg is None:
        return 1.0
    minute = time.gmtime(parse_ts(ts) + CST_OFFSET).tm_hour * 60
    window = 1.0
    for w in seg.get("windows") or []:
        sh, _, sm = w["start"].partition(":")
        eh, _, em = w["end"].partition(":")
        start_min = int(sh) * 60 + int(sm or 0)
        end_min = int(eh) * 60 + int(em or 0)
        inside = (
            (minute >= start_min or minute < end_min)
            if start_min > end_min
            else (start_min <= minute < end_min)
        )
        if inside:
            window = min(window, w["rate"])
    return seg["rate"] * window


class Billing:
    def __init__(self, models: dict, entitlement: dict, cny_per_credit: float,
                 offpeak: list | None = None):
        self.models = models
        self.entitlement = entitlement
        self.cny_per_credit = cny_per_credit
        self.offpeak = offpeak or []

    def cost_cny(self, rec: dict) -> float | None:
        """单条 dim 记录的 CNY 成本；模型未登记 `[[dim_model]]` 时返回 None（N/A）。"""
        entries = self.models.get(rec["model"])
        if not entries:
            return None
        cfg = pick_config(entries, rec["time"])
        peak = is_peak(cfg, rec["time"], self.offpeak)
        total = (
            rec["input_tokens"] * rate_of(cfg, "input_cny", "peak_input_cny", peak)
            + rec["output_tokens"] * rate_of(cfg, "output_cny", "peak_output_cny", peak)
            + rec["cache_read_tokens"] * rate_of(cfg, "cache_read_cny", "peak_cache_read_cny", peak)
            + rec["cache_write_tokens"] * rate_of(cfg, "cache_write_cny", "peak_cache_write_cny", peak)
        ) / 1_000_000.0
        return total * rate_for(self.entitlement.get(rec["model"], []), rec["time"])

    def milli_credits(self, rec: dict) -> float | None:
        cny = self.cost_cny(rec)
        return None if cny is None else cny / self.cny_per_credit * 1000.0


# ─── 数据源 ──────────────────────────────────────────────────────────────────

def repo_path(*parts: str) -> str:
    return os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", *parts)


def load_dim_models() -> dict:
    """`[[dim_model]]` → {model: [config, ...]}，按 effective_from 升序（基线在前）。"""
    path = os.environ.get("PRICING_CONFIG") or repo_path("backend", "pricing.toml")
    with open(path, "rb") as fh:
        cfg = tomllib.load(fh)
    models: dict = {}
    for entry in cfg.get("dim_model", []):
        models.setdefault(entry["name"], []).append(entry)
    for entries in models.values():
        entries.sort(key=lambda e: e.get("effective_from") or "")
    return models


def load_entitlement() -> dict:
    path = os.environ.get("DIM_ENTITLEMENT_STATE_PATH") or os.path.expanduser(
        "~/.config/token-stats/dim-entitlement.json"
    )
    if not os.path.exists(path):
        return {}
    with open(path) as fh:
        return json.load(fh).get("models", {})


def load_records(model_filter: str | None) -> list:
    db = os.environ.get("TOKEN_STATS_DB_PATH") or os.path.expanduser(
        "~/.config/token-stats/token-stats.db"
    )
    if not os.path.exists(db):
        sys.exit(f"ERROR: store not found: {db} (set TOKEN_STATS_DB_PATH)")
    conn = sqlite3.connect(f"file:{db}?mode=ro", uri=True)
    sql = (
        "select time, model, input_tokens, output_tokens, cache_read_tokens,"
        " cache_write_tokens from token_records where source='dim'"
    )
    args: tuple = ()
    if model_filter:
        sql += " and model = ?"
        args = (model_filter,)
    rows = conn.execute(sql + " order by time", args).fetchall()
    conn.close()
    return [
        {
            "time": r[0],
            "model": r[1],
            "input_tokens": r[2],
            "output_tokens": r[3],
            "cache_read_tokens": r[4],
            "cache_write_tokens": r[5],
        }
        for r in rows
    ]


def console_cookie() -> str:
    path = os.path.expanduser("~/.config/token-stats/deploy-env.sh")
    if os.path.exists(path):
        for line in open(path):
            if "DIMAGENT_SESSION_COOKIE=" in line:
                value = line.split("=", 1)[1].strip().strip("\"'")
                if len(value) > 20:
                    return value
    env = os.environ.get("DIMAGENT_SESSION_COOKIE")
    if env:
        return env
    sys.exit("ERROR: no DIMAGENT_SESSION_COOKIE — 先跑 scripts/refresh-dimagent-cookie.sh")


def fetch_ledger(days: int, interval: str) -> dict:
    now = int(time.time())
    url = (
        f"{CONSOLE_API}/user/daily-stats?start_time={now - days * 86400}"
        f"&end_time={now}&interval={interval}"
    )
    req = urllib.request.Request(
        url, headers={"Cookie": f"session={console_cookie()}", "Accept": "application/json"}
    )
    try:
        with urllib.request.urlopen(req, timeout=30) as resp:
            payload = json.load(resp)
    except urllib.error.HTTPError as exc:
        sys.exit(
            f"ERROR: daily-stats HTTP {exc.code} — cookie 过期，跑 scripts/refresh-dimagent-cookie.sh"
        )
    if not payload.get("success", True):
        sys.exit(f"ERROR: daily-stats rejected the request: {payload.get('message')}")
    return {d["date"]: d for d in payload["data"]}


# ─── 对账 ────────────────────────────────────────────────────────────────────

def reconcile(records: list, ledger: dict, billing: Billing, interval: str) -> tuple:
    """返回 (buckets, failures, unpriced_models)。"""
    by_key: dict = {}
    for rec in records:
        by_key.setdefault(cst_key(rec["time"], interval), []).append(rec)

    buckets, failures, gaps, unpriced = [], [], [], set()
    for key in sorted(by_key):
        row = ledger.get(key)
        if not row or not row["request_count"]:
            continue
        counted = row["request_count"]
        same = by_key[key][:counted]
        if len(same) < counted:
            buckets.append(
                {
                    "key": key,
                    "status": "gap",
                    "detail": f"账本 {counted} 次，本地只有 {len(same)} 次（缺 {counted - len(same)} 次）",
                }
            )
            gaps.append(key)
            continue
        milli, per_model = 0.0, {}
        for rec in same:
            credits = billing.milli_credits(rec)
            if credits is None:
                unpriced.add(rec["model"])
                continue
            milli += credits
            slot = per_model.setdefault(rec["model"], [0.0, 0])
            slot[0] += credits
            slot[1] += 1
        final, base = row["final_amount_minor"], row["base_amount_minor"]
        ratio = milli / final if final else 0.0
        status = "ok" if abs(ratio - 1.0) <= TOLERANCE else "mismatch"
        if status == "mismatch":
            failures.append(key)
        buckets.append(
            {
                "key": key,
                "status": status,
                "requests": counted,
                "local_requests": len(same),
                "predicted_milli_credits": round(milli, 1),
                "ledger_final_milli_credits": final,
                "ledger_base_milli_credits": base,
                "implied_entitlement_rate": round(final / base, 4) if base else None,
                "ratio": round(ratio, 4),
                "per_model": {
                    m: {"milli_credits": round(v[0], 1), "requests": v[1]}
                    for m, v in sorted(per_model.items())
                },
            }
        )
    return buckets, failures, gaps, unpriced


def print_buckets(buckets: list, failures: list, gaps: list, unpriced: set, interval: str, args) -> None:
    unit = "天" if interval == "daily" else "小时桶"
    print(
        f"dim 计价对账（按{unit}，积分折算 ¥{args.cny_per_credit:.9f}/积分，"
        f"容差 {args.tolerance:.3%}）"
    )
    if not buckets:
        print(f"  没有可对账的已结算{unit}")
    for b in buckets:
        if b["status"] == "gap":
            print(f"  {b['key']}  ⚠ {b['detail']}")
            continue
        mark = "✅" if b["status"] == "ok" else "❌"
        rate = b["implied_entitlement_rate"]
        rate_txt = f"，entitlement {rate}" if rate not in (None, 1.0) else ""
        print(
            f"  {b['key']}  {mark} {b['requests']:>5} 次  算得 {b['predicted_milli_credits']:>12.1f}"
            f" / 账本 {b['ledger_final_milli_credits']:>9} 毫积分  ({b['ratio']:.4f}x{rate_txt})"
        )
        for model, v in b["per_model"].items():
            print(f"        {model:<28} {v['requests']:>5} 次  {v['milli_credits']:>12.1f} 毫积分")
    if unpriced:
        print(f"\n  ❌ 未登记 [[dim_model]] 的模型（成本 N/A，且被聚合剔除）：{', '.join(sorted(unpriced))}")
        print("     登记方法见 docs/agents/data-sources.md「平台新增计费模型」")
    if gaps:
        print(
            f"\n  ⚠ 记录有洞的{unit}（账本次数 > 本地记录，**不是计价错**，是 dim 源漏了请求）："
            f"{', '.join(gaps)}"
        )
        print("     cookie 过期 / 轮询中断都会这样，跑 scripts/refresh-dimagent-cookie.sh")
    if failures:
        print(f"\n  ❌ 计价未对上的{unit}：{', '.join(failures)}")
        print("     多半是 [[dim_model]] 费率/分段错了，或 entitlement 分段没学到")


def main() -> int:
    ap = argparse.ArgumentParser(
        description="用 DimAgent 账本核对 dim 源计价",
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    ap.add_argument("--model", help="只核对某个模型（默认全部 dim 模型）")
    ap.add_argument("--days", type=int, default=14, help="拉取最近多少天（默认 14）")
    ap.add_argument(
        "--interval", choices=["daily", "hourly"], default="daily", help="对账粒度（默认 daily）"
    )
    ap.add_argument("--both", action="store_true", help="daily 与 hourly 都跑")
    ap.add_argument("--tolerance", type=float, default=TOLERANCE, help="相对容差（默认 0.001）")
    ap.add_argument(
        "--cny-per-credit",
        type=float,
        default=DEFAULT_CNY_PER_CREDIT,
        help="积分折 CNY 单价（默认 Lite 的 70/11000；Lite+ 用 139/22200）",
    )
    ap.add_argument("--json", action="store_true", help="输出 JSON")
    args = ap.parse_args()

    billing = Billing(load_dim_models(), load_entitlement(), args.cny_per_credit,
                    load_dim_offpeak())
    records = load_records(args.model)

    intervals = ["daily", "hourly"] if args.both else [args.interval]
    report, failures, gaps, unpriced = {}, [], [], set()
    for interval in intervals:
        ledger = fetch_ledger(args.days, interval)
        buckets, bad, missing, unpriced_here = reconcile(records, ledger, billing, interval)
        report[interval] = buckets
        failures += [f"{interval}:{k}" for k in bad]
        gaps += [f"{interval}:{k}" for k in missing]
        unpriced |= unpriced_here

    if args.json:
        print(
            json.dumps(
                {
                    "buckets": report,
                    "coverage_gaps": gaps,
                    "unpriced_models": sorted(unpriced),
                    "cny_per_credit": args.cny_per_credit,
                },
                ensure_ascii=False,
                indent=2,
            )
        )
        return 1 if failures or unpriced or gaps else 0

    for interval in intervals:
        if len(intervals) > 1:
            print(f"\n=== {interval} ===")
        prefix = interval + ":"
        print_buckets(
            report[interval],
            [f[len(prefix):] for f in failures if f.startswith(prefix)],
            [g[len(prefix):] for g in gaps if g.startswith(prefix)],
            unpriced,
            interval,
            args,
        )
    return 1 if failures or unpriced or gaps else 0


if __name__ == "__main__":
    sys.exit(main())
