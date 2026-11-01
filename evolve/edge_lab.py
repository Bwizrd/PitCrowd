#!/usr/bin/env python3
"""edge-lab's intraday strategies as genome signals (README, "edge-lab strategies").

An edge-lab strategy is `positions(bars) -> [-1, 0, +1] per bar`. It runs on its own
timeframe (edge-lab's best for it: 15m, 30m or 1h), on bars built from the 15m ones; a genome
of kind `edge_lab` enters at the close of the 15m bar that closes a bar where the strategy's
position turns long (short) from anything else, and from there
it is an ordinary genome: its own window, filters and exit, the desk's costs and gate. The
strategy's own exit is not used.

    python3 -I edge_lab.py select           # which strategies qualify -> edge_lab/candidates.json
    python3 -I edge_lab.py build [--jobs 8] # run them on the 15m history -> edge_lab/signals/
    python3 -I edge_lab.py live KEY TF < bars  # the desk: the entry on the last of these 15m bars

Strategies are read from edge-lab and run here, each in its own process with a time limit.
Bars come straight from edge-lab's cache files (never `data.bars`, which can refetch from the
bridge), as [t, o, h, l, c, 0] rows: the volume column is always zero, exactly as the desk
passes them live, and a strategy whose positions change when it is not zero is left out.
"""
import json, os, random, subprocess, sys, time, importlib.util

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(HERE, "edge_lab")
SIGNALS = os.path.join(OUT, "signals")
CANDIDATES = os.path.join(OUT, "candidates.json")
ELIGIBLE = os.path.join(OUT, "eligible.json")
# edge-lab is an external strategy lab (not part of this repo, and optional): point EDGE_LAB_DIR
# at a checkout to seat its strategies. Without one, `select` finds none and the pit runs on
# genomes alone. The 15m bars are the floor's own cache (opencry_visual::edge_cache_dir).
EDGE = os.environ.get("EDGE_LAB_DIR", "")
GENERATED = os.path.join(EDGE, "strategies", "generated") if EDGE else ""
CACHE = os.environ.get("OPENCRY_EDGE_CACHE") or (os.path.join(EDGE, "data_cache") if EDGE else os.path.join(os.path.dirname(HERE), "data", "cache"))
SYMBOLS = ["XAUUSD", "XAGUSD", "GBPUSD", "EURUSD", "UK100", "US500", "NAS100", "US30"]

# ---- the pre-registered rules (README says the same; change both or neither) -------------
INTRADAY = {"15m", "30m", "1h"}           # edge-lab's best timeframe for the strategy
TF = {"15m": 15, "30m": 30, "1h": 60}     # ... and the bars it runs on here
RAN_CLEANLY = {"noise", "weak", "promising", "SHORTLIST", "is_killed"}   # not rejected / too_few / suspicious / too_slow
WINDOW = 2500                             # bars the desk keeps, so bars a live call sees
CHECK_BARS = 60                           # sampled bars per symbol for the replay check
CHECK_SYMBOLS = ["US500", "EURUSD"]
MIN_AGREE = 0.95                          # windowed entries must match the full history's
MAX_LIVE_SECS = 1.0                       # one live call on WINDOW bars
FULL_TIMEOUT = 300                        # seconds for one strategy over all eight symbols
# --------------------------------------------------------------------------------------------


def load_fn(key):
    spec = importlib.util.spec_from_file_location(f"edge_{key}", os.path.join(GENERATED, f"{key}.py"))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod.positions


def entries(pos):
    """+1 / -1 on the bar a position turns long / short from anything else, else 0."""
    out = [0] * len(pos)
    for i in range(1, len(pos)):
        p, q = pos[i], pos[i - 1]
        if p and p != q:
            out[i] = 1 if p > 0 else -1
    return out


def resample(b, tf):
    """`tf`-minute bars from 15m ones, grouped on UTC boundaries, with the index of each
    group's last 15m bar and whether that bar closes the group (a group cut short by a
    market close does not, and never enters)."""
    if tf == 15:
        return b, list(range(len(b))), [True] * len(b)
    ms = tf * 60_000
    out, last, full = [], [], []
    for i, r in enumerate(b):
        g = r[0] // ms * ms
        if out and out[-1][0] == g:
            o = out[-1]
            o[2], o[3], o[4] = max(o[2], r[2]), min(o[3], r[3]), r[4]
            last[-1] = i
        else:
            out.append([g, r[1], r[2], r[3], r[4], 0.0])
            last.append(i)
        full_now = r[0] + 900_000 == g + ms
        if len(full) < len(out):
            full.append(full_now)
        else:
            full[-1] = full_now
    return out, last, full


def entries15(fn, b, tf):
    """The strategy's entries on its own timeframe, placed on the 15m bars that close them."""
    hb, last, full = resample(b, tf)
    e = entries([int(x) for x in fn(hb)])
    out = [0] * len(b)
    for g, d in enumerate(e):
        if d and full[g]:
            out[last[g]] = d
    return out


def bars_of(sym):
    rows = json.load(open(os.path.join(CACHE, f"ctrader_{sym}_SB_15m.json")))
    rows = sorted({int(r[0]): r for r in rows if len(r) >= 5}.values(), key=lambda r: r[0])
    return [[int(r[0]), float(r[1]), float(r[2]), float(r[3]), float(r[4]), 0.0] for r in rows]


def select():
    rows = [json.loads(l) for l in open(os.path.join(EDGE, "registry.jsonl"))]
    last = {}
    for r in rows:
        if r.get("event") == "batch_candidate":
            last[r["strategy"]] = r
    keep, why = [], {}
    for key, r in sorted(last.items()):
        path = os.path.join(GENERATED, f"{key}.py")
        if r.get("interval") not in INTRADAY: why["not intraday"] = why.get("not intraday", 0) + 1; continue
        if r.get("result") not in RAN_CLEANLY: why["did not run cleanly"] = why.get("did not run cleanly", 0) + 1; continue
        if not os.path.exists(path): why["no file"] = why.get("no file", 0) + 1; continue
        src = open(path).read()
        if "def positions(bars)" not in src: why["needs a volume profile"] = why.get("needs a volume profile", 0) + 1; continue
        if "[5]" in src or "volume" in src.lower(): why["reads volume"] = why.get("reads volume", 0) + 1; continue
        keep.append({"key": key, "title": r.get("title", ""), "interval": r["interval"], "edge_lab_class": r["result"]})
    os.makedirs(OUT, exist_ok=True)
    json.dump(keep, open(CANDIDATES, "w"), indent=0)
    print(f"{len(last)} strategies judged by edge-lab; {len(keep)} candidates; left out: {why}")


def build_one(key, tf):
    """Child process: full-history entries per symbol, then the replay check. One JSON line."""
    fn = load_fn(key)
    hb = resample(bars_of("US500"), tf)[0]
    noisy = [r[:5] + [random.Random(r[0]).uniform(1, 1e4)] for r in hb]
    if [int(x) for x in fn(hb)] != [int(x) for x in fn(noisy)]:
        return {"key": key, "reads_volume": True}
    sig, n_entries = {}, 0
    for sym in SYMBOLS:
        b = bars_of(sym)
        e = entries15(fn, b, tf)
        sig[sym] = [[b[i][0], e[i]] for i in range(len(b)) if e[i]]
        n_entries += len(sig[sym])
        if sym in CHECK_SYMBOLS:
            sig["_" + sym] = (b, e)
    rng = random.Random(key)
    agree = total = 0
    worst = 0.0
    for sym in CHECK_SYMBOLS:
        b, e = sig.pop("_" + sym)
        hits = [i for i in range(WINDOW, len(b)) if e[i]]
        quiet = [i for i in range(WINDOW, len(b)) if not e[i]]
        sample = rng.sample(hits, min(len(hits), CHECK_BARS // 2)) + rng.sample(quiet, min(len(quiet), CHECK_BARS // 2))
        for i in sample:
            t0 = time.time()
            w = entries15(fn, b[i - WINDOW + 1:i + 1], tf)
            worst = max(worst, time.time() - t0)
            total += 1
            agree += w[-1] == e[i]
    # Written here, not printed: a parent reading a pipe only at exit would deadlock on it.
    os.makedirs(os.path.join(OUT, "pending"), exist_ok=True)
    json.dump(sig, open(os.path.join(OUT, "pending", f"{key}@{tf}.json"), "w"))
    return {"key": key, "n_entries": n_entries, "agree": agree / total if total else 0.0,
            "checked": total, "live_secs": round(worst, 3)}


def build(jobs):
    cands = json.load(open(CANDIDATES))
    os.makedirs(SIGNALS, exist_ok=True)
    done = json.load(open(ELIGIBLE)) if os.path.exists(ELIGIBLE) else {"eligible": [], "excluded": {}}
    seen = {f'{e["key"]}@{e["tf"]}' for e in done["eligible"]} | set(done["excluded"])
    todo = [{**c, "tf": TF[c["interval"]]} for c in cands if f'{c["key"]}@{TF[c["interval"]]}' not in seen]
    print(f"{len(cands)} candidates, {len(todo)} to build, {jobs} at a time", flush=True)
    running = []

    def reap(p, c, started):
        out, _ = p.communicate()
        p.err.seek(0); err = p.err.read(); p.err.close(); os.remove(p.err.name)
        if p.returncode != 0 or not out.strip():
            done["excluded"][f'{c["key"]}@{c["tf"]}'] = "failed: " + (err.strip().splitlines() or ["no output"])[-1][:120]
            return
        r = json.loads(out.strip().splitlines()[-1])
        if r.get("reads_volume"): done["excluded"][f'{c["key"]}@{c["tf"]}'] = "reads volume"; return
        pending = os.path.join(OUT, "pending", f"{c['key']}@{c['tf']}.json")
        if r["n_entries"] == 0: done["excluded"][f'{c["key"]}@{c["tf"]}'] = "never enters"; return
        if r["agree"] < MIN_AGREE: done["excluded"][f'{c["key"]}@{c["tf"]}'] = f"replay agrees on {r['agree']:.0%} of {r['checked']} bars"; return
        if r["live_secs"] > MAX_LIVE_SECS: done["excluded"][f'{c["key"]}@{c["tf"]}'] = f"live call {r['live_secs']} s"; return
        os.replace(pending, os.path.join(SIGNALS, f"{c['key']}@{c['tf']}.json"))
        done["eligible"].append({**c, "n_entries": r["n_entries"], "agree": round(r["agree"], 3), "live_secs": r["live_secs"]})

    def save():
        json.dump(done, open(ELIGIBLE, "w"), indent=0)

    for k, c in enumerate(todo):
        while len(running) >= jobs:
            for item in list(running):
                p, cc, st = item
                if p.poll() is not None:
                    running.remove(item); reap(p, cc, st)
                elif time.time() - st > FULL_TIMEOUT:
                    p.kill(); p.communicate(); running.remove(item); done["excluded"][f'{cc["key"]}@{cc["tf"]}'] = f"over {FULL_TIMEOUT} s"
            time.sleep(0.2)
        # stdout is one short line (the strategy's own prints are silenced in the child);
        # stderr goes to a file so a noisy failure cannot fill a pipe either.
        os.makedirs(os.path.join(OUT, "pending"), exist_ok=True)
        err = open(os.path.join(OUT, "pending", f"{c['key']}@{c['tf']}.err"), "w+")
        p = subprocess.Popen([sys.executable, "-I", __file__, "_one", c["key"], str(c["tf"])], stdout=subprocess.PIPE, stderr=err, text=True)
        p.err = err
        running.append((p, c, time.time()))
        if k % 25 == 0:
            save(); print(f"  {k}/{len(todo)}: {len(done['eligible'])} eligible, {len(done['excluded'])} excluded", flush=True)
    while running:
        for item in list(running):
            p, cc, st = item
            if p.poll() is not None:
                running.remove(item); reap(p, cc, st)
            elif time.time() - st > FULL_TIMEOUT:
                p.kill(); p.communicate(); running.remove(item); done["excluded"][f'{cc["key"]}@{cc["tf"]}'] = f"over {FULL_TIMEOUT} s"
        time.sleep(0.2)
    save()
    print(f"done: {len(done['eligible'])} eligible, {len(done['excluded'])} excluded")


def live(key, tf):
    b = [[int(r[0]), float(r[1]), float(r[2]), float(r[3]), float(r[4]), 0.0] for r in json.load(sys.stdin)]
    e = entries15(load_fn(key), b, tf) if len(b) > 1 else [0]
    print(json.dumps({"key": key, "t": b[-1][0] if b else None, "d": e[-1]}))


def no_lab():
    """True (and says so) when there is no edge-lab to read strategies from."""
    if EDGE and os.path.isdir(GENERATED):
        return False
    print("edge-lab not found (set EDGE_LAB_DIR to a checkout): no edge-lab strategies; the pit runs on genomes alone")
    return True


if __name__ == "__main__":
    cmd = sys.argv[1] if len(sys.argv) > 1 else ""
    if cmd in ("select", "build") and no_lab(): sys.exit(0)
    if cmd == "select": select()
    elif cmd == "build": build(int(sys.argv[sys.argv.index("--jobs") + 1]) if "--jobs" in sys.argv else 8)
    elif cmd == "_one":
        real, sys.stdout = sys.stdout, open(os.devnull, "w")
        r = build_one(sys.argv[2], int(sys.argv[3]))
        sys.stdout = real
        print(json.dumps(r))
    elif cmd == "live": live(sys.argv[2], int(sys.argv[3]) if len(sys.argv) > 3 else 15)
    else: print(__doc__)
