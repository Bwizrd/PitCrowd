#!/usr/bin/env python3
"""edge-lab's daily and 4h strategies as holders: bots that trade a strategy exactly as edge-lab
wrote it, entries AND exits, holding overnight (README, "Show floor").

A holder is one strategy on one symbol. `positions(bars)` runs on its own timeframe (4h or 1d,
built from the 15m history on UTC boundaries, as edge_lab.py does); whenever its position for a
finished bar differs from what the holder holds, the holder closes and/or opens at the next 15m
bar's open. Costs are the desk's: bars are bid prices, a buy pays the typical spread. One R is
two ATR(14) of the strategy's own bars at entry; a safety stop sits at 5 R and is the only exit
that is not the strategy's (after one, the holder waits for the strategy's next change).

    python3 -I holders.py select              # which strategies qualify -> holders/candidates.json
    python3 -I holders.py build [--jobs 6]    # backtest them on all eight symbols -> holders/results.jsonl
    python3 -I holders.py live < jobs.json    # the desk: each holder's position for its last finished bar

Bars come straight from edge-lab's cache files (never `data.bars`, which can refetch from the
bridge). The desk passes its own recent 15m bars and the older history is read from the cache.
"""
import json, math, os, random, subprocess, sys, time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from edge_lab import EDGE, GENERATED, SYMBOLS, RAN_CLEANLY, load_fn, resample, bars_of, no_lab   # noqa: E402

OUT = os.path.join(HERE, "holders")
CANDIDATES = os.path.join(OUT, "candidates.json")
RESULTS = os.path.join(OUT, "results.jsonl")
TRADES = os.path.join(OUT, "trades")

# ---- the pre-registered rules (README "Show floor" says the same; change both or neither) ----
TF = {"4h": 240, "1d": 1440}
R_ATR = 2.0          # one R, in ATR(14) of the strategy's own bars
SAFETY_R = 5.0       # the safety stop, in R (at 2 R it took 21% of exits, at 5 R 5%)
CHECK_BARS = 40      # sampled bars per check symbol: positions on history up to the bar must
CHECK_SYMBOLS = ["US500", "EURUSD"]   # equal positions on the whole history (no look-ahead)
MIN_AGREE = 0.95
FULL_TIMEOUT = 300
HOLDOUT_FROM = 1735689600000          # 2025-01-01, as the gate
# ------------------------------------------------------------------------------------------------

TICK = {"XAUUSD": 0.10, "XAGUSD": 0.01, "GBPUSD": 0.0001, "EURUSD": 0.0001, "UK100": 0.5,
        "US500": 0.25, "NAS100": 0.25, "US30": 1.0}
SPREAD_TICKS = {"XAUUSD": 1.9, "XAGUSD": 4.0, "GBPUSD": 0.8, "EURUSD": 0.5, "UK100": 2.6,
                "US500": 1.6, "NAS100": 4.4, "US30": 2.1}   # genome.rs typical_spread_ticks


def atr14(hb, g):
    lo = max(1, g - 13)
    tr = [max(hb[k][2], hb[k - 1][4]) - min(hb[k][3], hb[k - 1][4]) for k in range(lo, g + 1)]
    return sum(tr) / len(tr) if tr else 0.0


def decisions(fn, b, tf):
    """(15m index the decision is acted on, fill on that bar's open or its close, position, R unit)
    for every finished bar of the strategy's own timeframe. A bar that its last 15m bar closes
    is acted on at the next 15m bar's open; one cut short by a market close is seen only when the
    next 15m bar closes (that is when the desk learns its bar is over), so it fills at that close."""
    hb, last, full = resample(b, tf)
    pos = [int(x) for x in fn(hb)]
    out = []
    for g in range(len(hb)):
        k = last[g] + 1
        if k >= len(b):
            break
        out.append((k, "open" if full[g] else "close", max(-1, min(1, pos[g])), R_ATR * atr14(hb, g), hb[g][0]))
    return out


def trades_of(b, dec, sym):
    sp = SPREAD_TICKS[sym] * TICK[sym]
    out, cur = [], None   # cur: [side, entry_px, unit, k_in, stop]

    def close(k, px, why):
        side, entry, unit, k_in, _ = cur
        r = ((px - entry) if side > 0 else (entry - px)) / unit
        out.append([b[k_in][0], round(r, 4), sym, "buy" if side > 0 else "sell", why, b[k][0]])

    k_prev, strat = 0, 0
    for k, at, p, unit, _ in dec:
        # The safety stop, on the 15m bars since the last decision.
        if cur:
            for j in range(max(k_prev, cur[3]), k):
                side, stop = cur[0], cur[4]
                o, h, l = b[j][1], b[j][2], b[j][3]
                if side > 0 and l <= stop:
                    close(j, min(o, stop) if j > cur[3] else stop, "safety stop"); cur = None; break
                if side < 0 and h + sp >= stop:
                    close(j, max(o + sp, stop) if j > cur[3] else stop, "safety stop"); cur = None; break
        k_prev = k
        # Only a change in the strategy's position is acted on: after a safety stop the holder
        # waits for the strategy to turn, as the desk does.
        if p == strat:
            continue
        strat = p
        bid = b[k][1] if at == "open" else b[k][4]
        if cur:
            close(k, bid if cur[0] > 0 else bid + sp, "strategy exit")
            cur = None
        if p and unit > 0:
            entry = bid + sp if p > 0 else bid
            stop = entry - SAFETY_R * unit if p > 0 else entry + SAFETY_R * unit
            # A fill on a bar's close is checked against the stop from the next bar on.
            cur = [p, entry, unit, k if at == "open" else k + 1, stop]
            if cur[3] >= len(b):
                cur = None
    return out


def stats(r):
    n = len(r)
    if n < 2:
        return {"n": n, "mean": round(r[0], 4) if r else 0.0, "t": 0.0}
    m = sum(r) / n
    sd = math.sqrt(sum((x - m) ** 2 for x in r) / (n - 1)) or 1e-9
    return {"n": n, "mean": round(m, 4), "t": round(m / sd * math.sqrt(n), 2), "sd": round(sd, 4)}


def select():
    rows = [json.loads(l) for l in open(os.path.join(EDGE, "registry.jsonl")) if l.strip()]
    last = {}
    for r in rows:
        if r.get("event") == "batch_candidate":
            last[r["strategy"]] = r
    fwd = json.load(open(os.path.join(EDGE, "forward_tests.json")))
    keep, why = [], {}
    for key, r in sorted(last.items()):
        if r.get("interval") not in TF: why["not 4h/1d"] = why.get("not 4h/1d", 0) + 1; continue
        if r.get("result") not in RAN_CLEANLY: why["did not run cleanly"] = why.get("did not run cleanly", 0) + 1; continue
        path = os.path.join(GENERATED, f"{key}.py")
        if not os.path.exists(path): why["no file"] = why.get("no file", 0) + 1; continue
        src = open(path).read()
        if "def positions(bars)" not in src: why["needs a volume profile"] = why.get("needs a volume profile", 0) + 1; continue
        if "[5]" in src or "volume" in src.lower(): why["reads volume"] = why.get("reads volume", 0) + 1; continue
        f = fwd.get(key, {})
        keep.append({"key": key, "title": r.get("title") or f.get("title", ""), "interval": r["interval"], "tf": TF[r["interval"]],
                     "edge_lab_class": r["result"], "edge_lab_forward": (f.get("verdict") or "").split(" [")[0] or None})
    # Built in this order, so the strategies edge-lab rates best are ready first.
    rank = {"SHORTLIST": 0, "promising": 1, "weak": 2, "is_killed": 3, "noise": 4}
    keep.sort(key=lambda c: (c["edge_lab_forward"] != "CONFIRMED (forward)", rank.get(c["edge_lab_class"], 5), c["key"]))
    os.makedirs(OUT, exist_ok=True)
    json.dump(keep, open(CANDIDATES, "w"), indent=0)
    print(f"{len(last)} strategies judged by edge-lab; {len(keep)} daily/4h candidates; left out: {why}")


def build_one(key, tf):
    fn = load_fn(key)
    hb = resample(bars_of("US500"), tf)[0]
    noisy = [r[:5] + [random.Random(r[0]).uniform(1, 1e4)] for r in hb]
    if [int(x) for x in fn(hb)] != [int(x) for x in fn(noisy)]:
        return {"key": key, "tf": tf, "excluded": "reads volume"}
    rng = random.Random(key)
    agree = total = 0
    per = {}
    for sym in SYMBOLS:
        b = bars_of(sym)
        dec = decisions(fn, b, tf)
        tr = trades_of(b, dec, sym)
        os.makedirs(os.path.join(TRADES, f"{key}@{tf}"), exist_ok=True)
        json.dump(tr, open(os.path.join(TRADES, f"{key}@{tf}", f"{sym}.json"), "w"))
        is_r = [t[1] for t in tr if t[0] < HOLDOUT_FROM]
        ho_r = [t[1] for t in tr if t[0] >= HOLDOUT_FROM]
        span = (b[-1][0] - b[0][0]) / 864e5 / 365.25
        per[sym] = {"is": stats(is_r), "ho": stats(ho_r), "all": stats(is_r + ho_r), "per_year": round(len(tr) / span, 1),
                    "safety_stops": sum(1 for t in tr if t[4] == "safety stop")}
        if sym in CHECK_SYMBOLS:
            hb2 = resample(b, tf)[0]
            full = [int(x) for x in fn(hb2)]
            for g in rng.sample(range(300, len(hb2)), min(CHECK_BARS, len(hb2) - 300)):
                total += 1
                agree += int(fn(hb2[:g + 1])[-1]) == full[g]
    a = agree / total if total else 0.0
    out = {"key": key, "tf": tf, "agree": round(a, 3), "symbols": per}
    if a < MIN_AGREE:
        out["excluded"] = f"looks ahead: history-to-date agrees with the whole history on {a:.0%} of {total} bars"
    return out


def build(jobs):
    cands = json.load(open(CANDIDATES))
    done = {(r["key"], r["tf"]) for r in map(json.loads, open(RESULTS))} if os.path.exists(RESULTS) else set()
    todo = [c for c in cands if (c["key"], c["tf"]) not in done]
    print(f"{len(cands)} candidates, {len(todo)} to build, {jobs} at a time", flush=True)
    os.makedirs(OUT, exist_ok=True)
    running = []

    def reap(p, c, err):
        out, _ = p.communicate()
        err.seek(0); e = err.read(); err.close(); os.remove(err.name)
        if p.returncode != 0 or not out.strip():
            r = {"key": c["key"], "tf": c["tf"], "excluded": "failed: " + (e.strip().splitlines() or ["no output"])[-1][:120]}
        else:
            r = json.loads(out.strip().splitlines()[-1])
        r.update({k: c[k] for k in ("title", "interval", "edge_lab_class", "edge_lab_forward")})
        with open(RESULTS, "a") as f:
            f.write(json.dumps(r) + "\n")

    def poll():
        for item in list(running):
            p, c, st, err = item
            if p.poll() is not None:
                running.remove(item); reap(p, c, err)
            elif time.time() - st > FULL_TIMEOUT:
                p.kill(); p.communicate(); running.remove(item); err.close(); os.remove(err.name)
                with open(RESULTS, "a") as f:
                    f.write(json.dumps({"key": c["key"], "tf": c["tf"], "excluded": f"over {FULL_TIMEOUT} s"}) + "\n")

    for k, c in enumerate(todo):
        while len(running) >= jobs:
            poll(); time.sleep(0.2)
        err = open(os.path.join(OUT, f".{c['key']}.err"), "w+")
        p = subprocess.Popen([sys.executable, "-I", __file__, "_one", c["key"], str(c["tf"])], stdout=subprocess.PIPE, stderr=err, text=True)
        running.append((p, c, time.time(), err))
        if k % 25 == 0:
            print(f"  {k}/{len(todo)}", flush=True)
    while running:
        poll(); time.sleep(0.2)
    print("done", flush=True)


def live():
    """stdin: [{"key", "tf", "symbol", "bars": [[t, o, h, l, c], ...]}]; stdout: one answer per job,
    {"key", "tf", "symbol", "g": the finished bar's open (ms), "pos": -1/0/+1, "unit": one R in price}."""
    jobs = json.load(sys.stdin)
    hist, out = {}, []
    for j in jobs:
        sym = j["symbol"]
        if sym not in hist:
            try:
                hist[sym] = bars_of(sym)
            except Exception:
                hist[sym] = []
        desk = [[int(r[0]), float(r[1]), float(r[2]), float(r[3]), float(r[4]), 0.0] for r in j["bars"]]
        first = desk[0][0] if desk else 1 << 62
        b = [r for r in hist[sym] if r[0] < first] + desk
        try:
            hb, last, full = resample(b, j["tf"])
            g = len(hb) - 1 if full[-1] else len(hb) - 2
            p = max(-1, min(1, int(load_fn(j["key"])(hb[:g + 1])[-1])))
            out.append({"key": j["key"], "tf": j["tf"], "symbol": sym, "g": hb[g][0], "pos": p, "unit": R_ATR * atr14(hb, g)})
        except Exception as e:
            out.append({"key": j["key"], "tf": j["tf"], "symbol": sym, "error": f"{type(e).__name__}: {e}"[:160]})
    print(json.dumps(out))


if __name__ == "__main__":
    cmd = sys.argv[1] if len(sys.argv) > 1 else ""
    if cmd in ("select", "build") and no_lab(): sys.exit(0)
    if cmd == "select": select()
    elif cmd == "build": build(int(sys.argv[sys.argv.index("--jobs") + 1]) if "--jobs" in sys.argv else 6)
    elif cmd == "_one":
        real, sys.stdout = sys.stdout, open(os.devnull, "w")
        r = build_one(sys.argv[2], int(sys.argv[3]))
        sys.stdout = real
        print(json.dumps(r))
    elif cmd == "live":
        real, sys.stdout = sys.stdout, open(os.devnull, "w")
        try:
            buf = __import__("io").StringIO(); sys.stdout = buf; live(); res = buf.getvalue().strip().splitlines()[-1]
        finally:
            sys.stdout = real
        print(res)
    else: print(__doc__)
