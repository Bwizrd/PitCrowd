#!/usr/bin/env python3
"""Calibrate the seating gate: run the pit's own search, unchanged, on worlds with no edge in them,
and count how many genomes pass.

    python3 -I evolve/audit/null_worlds.py make  <kind> <seed>       # build a world's 15m bars
    python3 -I evolve/audit/null_worlds.py search <kind> <seed> [--genomes 15000] [--procs 5]
    python3 -I evolve/audit/null_worlds.py summary
    python3 -I evolve/audit/null_worlds.py gates        # passes per 1,000 at other thresholds
    python3 -I evolve/audit/null_worlds.py ideas        # passers clustered by shared entries

Worlds (all keep every real timestamp, so sessions, gaps, weekends and the holdout split are
the real ones; spreads are the backtest's usual ticks):
  real   the snapshot in data/real/ of edge-lab's 15m cache (the real search, re-run)
  flip   each bar is mirrored with probability 1/2: its gap from the last close and its
         open-to-high/low/close moves change sign together. Volatility, its clustering and
         its time-of-day shape are exactly the real ones; direction carries no information,
         and the drift is gone.
  block  a stationary bootstrap aligned to the clock: runs of real bars (mean 8 bars, 2 hours)
         drawn from random days at the same time of day, chained as returns. Short-range
         structure inside a run survives; anything longer than a run, or tied to a day's
         earlier bars, is broken. A weaker null than flip.

The search is evolve.py's: 400 genomes a cycle, 30% of them mutants of the 30 best in-sample
genomes with 100+ trades, judge() with N = genomes tried so far. edge_lab genomes are not drawn
(their entries come from edge-lab's Python, which would have to run on every world).
"""
import json, math, os, random, subprocess, sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.dirname(HERE))
import evolve as E   # noqa: E402

DATA, OUT = os.path.join(HERE, "data"), os.path.join(HERE, "out")
SYMBOLS = E.SYMBOLS
BAR = 15 * 60 * 1000


def load(sym, d=os.path.join(HERE, "data", "real")):
    rows = json.load(open(f"{d}/ctrader_{sym}_SB_15m.json"))
    rows = sorted({r[0]: r for r in rows if len(r) >= 5}.values())
    return rows


def shapes(rows):
    """Each bar as log moves: gap from the last close, then open to high, low, close."""
    out, prev = [], rows[0][1]
    for t, o, h, l, c, *_ in rows:
        out.append((math.log(o / prev), math.log(h / o), math.log(l / o), math.log(c / o)))
        prev = c
    return out


def chain(rows, moves):
    p, out = rows[0][1], []
    for (t, *_), (g, dh, dl, dc) in zip(rows, moves):
        o = p * math.exp(g)
        h, l, c = o * math.exp(dh), o * math.exp(dl), o * math.exp(dc)
        out.append([t, round(o, 6), round(max(h, o, c), 6), round(min(l, o, c), 6), round(c, 6), 0.0])
        p = c
    return out


def flip(moves, rng):
    out = []
    for g, dh, dl, dc in moves:
        out.append((-g, -dl, -dh, -dc) if rng.random() < 0.5 else (g, dh, dl, dc))
    return out


def block(rows, moves, rng, mean=8):
    by_tod = {}
    for i, r in enumerate(rows):
        by_tod.setdefault(r[0] % 86_400_000, []).append(i)
    out, j = [], None
    for i, r in enumerate(rows):
        cont = (j is not None and j + 1 < len(rows) and rng.random() > 1 / mean
                and rows[j + 1][0] - rows[j][0] == r[0] - rows[i - 1][0])
        j = j + 1 if cont else rng.choice(by_tod[r[0] % 86_400_000])
        out.append(moves[j])
    return out


def world_dir(kind, seed):
    return os.path.join(DATA, "real") if kind == "real" else os.path.join(DATA, f"{kind}_{seed}")


def make(kind, seed):
    d = world_dir(kind, seed)
    if kind == "real" or os.path.exists(os.path.join(d, "done")):
        return d
    os.makedirs(d, exist_ok=True)
    for k, sym in enumerate(SYMBOLS):
        rng = random.Random(f"{kind}-{seed}-{sym}")
        rows = load(sym)
        m = shapes(rows)
        m = flip(m, rng) if kind == "flip" else block(rows, m, rng)
        json.dump(chain(rows, m), open(f"{d}/ctrader_{sym}_SB_15m.json", "w"))
    open(os.path.join(d, "done"), "w").close()
    return d


def backtest(genomes, cache, procs):
    exe = next(p for p in E.BACKTEST if os.path.exists(p))
    chunks = [genomes[i::procs] for i in range(procs)]
    ps = [subprocess.Popen([exe, "--cache", cache], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True) for _ in chunks]
    for p, c in zip(ps, chunks):
        p.stdin.write("".join(json.dumps(g) + "\n" for g in c)); p.stdin.close()
    res = {}
    for p in ps:
        for line in p.stdout:
            d = json.loads(line)
            if "id" in d:
                res[d["id"]] = d["trades"]
        p.wait()
    return res


def no_edge_lab(rng):
    while True:
        g = E.random_genome(rng)
        if g["signal"]["kind"] != "edge_lab":
            return g


def search(kind, seed, total, procs, spawn=400):
    cache = make(kind, seed)
    rng = random.Random(f"search-{kind}-{seed}")
    path = os.path.join(OUT, f"search_{kind}_{seed}.jsonl")
    tried = [json.loads(l) for l in open(path)] if os.path.exists(path) else []   # resumable
    seen = {E.shape(t["genome"]) for t in tried}
    f = open(path, "a")
    while len(tried) < total:
        near = [t for t in tried if t["result"]["is"]["n"] >= 100]
        parents = [t["genome"] for t in sorted(near, key=lambda t: (-t["result"]["pass"], -t["result"]["is"]["t"]))[:30]]
        fresh = []
        while len(fresh) < min(spawn, total - len(tried)):
            g = E.mutate(rng.choice(parents), rng) if parents and rng.random() < 0.3 else no_edge_lab(rng)
            if g["signal"]["kind"] == "edge_lab" or E.shape(g) in seen:
                continue
            seen.add(E.shape(g))
            g["id"] = f"n{len(tried) + len(fresh) + 1:05d}"; g["name"] = g["id"]
            fresh.append(g)
        res = backtest(fresh, cache, procs)
        n_tried = len(tried) + len(fresh)
        for g in fresh:
            tr = res.get(g["id"], [])
            r = E.judge(tr, n_tried) if tr else {"pass": False, "is": E.stats([]), "ho": E.stats([]), "per_weekday": 0, "dsr_is": 0}
            row = {"genome": g, "result": r}
            tried.append(row); f.write(json.dumps(row) + "\n")
        f.flush()
        print(f"{kind}_{seed}: {len(tried)} tried, {sum(t['result']['pass'] for t in tried)} pass", flush=True)
    return path


def summary():
    rows = []
    for fn in sorted(os.listdir(OUT)):
        if fn.startswith("search_") and fn.endswith(".jsonl"):
            tried = [json.loads(l) for l in open(os.path.join(OUT, fn))]
            passed = [t for t in tried if t["result"]["pass"]]
            ideas = {E.idea(t["genome"]) for t in passed}
            best = max(tried, key=lambda t: t["result"]["is"]["t"] if t["result"]["is"]["n"] >= 200 else -9)
            rows.append((fn[7:-6], len(tried), len(passed), len(ideas), best["result"]["is"]["t"],
                         sorted({t["genome"]["signal"]["kind"] + "/" + t["genome"]["mode"] + "/" + t["genome"]["window"] for t in passed})))
    print(f"{'world':12} {'tried':>6} {'pass':>5} {'per 1k':>7} {'ideas':>5} {'best IS t (n>=200)':>18}  passing ideas")
    for w, n, p, i, bt, kinds in rows:
        print(f"{w:12} {n:6d} {p:5d} {1000 * p / max(n, 1):7.2f} {i:5d} {bt:18.2f}  {', '.join(kinds)}")


GRID = [  # (label, changes to the gate)
    ("the gate as written", {}),
    ("IS t >= 1.5", {"is_t": 1.5}),
    ("IS t >= 2.5", {"is_t": 2.5}),
    ("IS t >= 3.0", {"is_t": 3.0}),
    ("HO t >= 0.5", {"ho_t": 0.5}),
    ("HO t >= 1.5", {"ho_t": 1.5}),
    ("HO t >= 2.0", {"ho_t": 2.0}),
    ("0.5 trades a weekday", {"per_weekday": 0.5}),
    ("0.25 trades a weekday", {"per_weekday": 0.25}),
    ("IS t >= 1.5, HO t >= 0.5", {"is_t": 1.5, "ho_t": 0.5}),
]


def passes(r, g):
    i, h = r["is"], r["ho"]
    return (i["n"] >= g["is_n"] and i["t"] >= g["is_t"] and i["mean"] >= g["is_mean"] and h["n"] >= g["ho_n"]
            and h["mean"] > g["ho_mean"] and h["t"] >= g["ho_t"] and r["per_weekday"] >= g["per_weekday"])


def gates():
    worlds = {"REAL search": os.path.join(HERE, "trials_15000.jsonl")}
    for fn in sorted(os.listdir(OUT)):
        if fn.startswith("search_") and fn.endswith(".jsonl"):
            worlds[fn[7:-6]] = os.path.join(OUT, fn)
    res = {w: [json.loads(l)["result"] for l in open(p)] for w, p in worlds.items()}
    nulls = [w for w in res if w.split("_")[0] in ("flip", "block")]
    print(f"{'passes per 1,000 tried':28} {'REAL':>6} {'real_1':>7} {'nulls (all)':>12}   by null world")
    for label, ch in GRID:
        g = {**E.GATE, **ch}
        rate = lambda w: 1000 * sum(passes(r, g) for r in res[w]) / len(res[w])
        pooled = 1000 * sum(passes(r, g) for w in nulls for r in res[w]) / max(1, sum(len(res[w]) for w in nulls))
        print(f"{label:28} {rate('REAL search'):6.2f} {rate('real_1') if 'real_1' in res else 0:7.2f} {pooled:12.3f}   "
              + " ".join(f"{w}:{sum(passes(r, g) for r in res[w])}" for w in nulls))
    print(f"(null worlds: {sum(len(res[w]) for w in nulls)} genomes tried)")


def clusters(passers, cache, edge_dir=None):
    """Passers grouped by the entries they share: two are one idea when half or more of the
    smaller one's trades open on the same bar of the same symbol (exits do not matter)."""
    exe = next(p for p in E.BACKTEST if os.path.exists(p))
    args = [exe, "--cache", cache, "--detail"] + (["--edge-dir", edge_dir] if edge_dir else [])
    out = subprocess.run(args, input="".join(json.dumps(g) + "\n" for g in passers), capture_output=True, text=True, check=True).stdout
    ent = {d["id"]: {(t, s) for t, _, s, *_ in d["trades"]} for d in map(json.loads, out.splitlines()) if "id" in d}
    groups = []
    for g in passers:
        e = ent[g["id"]]
        home = next((c for c in groups if any(len(e & ent[o]) >= 0.5 * min(len(e), len(ent[o])) for o in c)), None)
        (home.append if home is not None else lambda x: groups.append([x]))(g["id"])
    return groups


def ideas():
    worlds = {"REAL search": (os.path.join(HERE, "trials_15000.jsonl"), os.path.join(DATA, "real"))}
    for fn in sorted(os.listdir(OUT)):
        if fn.startswith("search_") and fn.endswith(".jsonl"):
            k = fn[7:-6]
            worlds[k] = (os.path.join(OUT, fn), world_dir(*k.split("_")[0:1], int(k.split("_")[1])) if k.split("_")[0] != "real" else os.path.join(DATA, "real"))
    edge = os.path.join(os.path.dirname(HERE), "edge_lab", "signals")
    total = {}
    for w, (path, cache) in worlds.items():
        rows = [json.loads(l) for l in open(path)]
        P = [r["genome"] for r in rows if r["result"]["pass"]]
        groups = clusters(P, cache, edge) if P else []
        kind = "real" if w.startswith("REAL") or w.startswith("real") else w.split("_")[0]
        t = total.setdefault(kind, [0, 0, 0, 0])
        t[0] += len(rows); t[1] += len(P); t[2] += len(groups); t[3] += 1
        desc = []
        for c in groups:
            g = next(x for x in P if x["id"] == c[0])
            desc.append(f"{g['signal']['kind']}/{g['mode']}/{g['window']}/{'+'.join(g['symbols'])} x{len(c)}")
        print(f"{w:12} {len(rows):6d} tried, {len(P):3d} pass, {len(groups)} independent ideas: {'; '.join(desc)}")
    print()
    for k, (n, p, i, w) in total.items():
        print(f"{k:6} {w} worlds, {n} tried: {1000 * p / n:.2f} passes and {1000 * i / n:.3f} independent ideas per 1,000")


if __name__ == "__main__":
    a = sys.argv[1:]
    if a[0] == "make":
        print(make(a[1], int(a[2])))
    elif a[0] == "search":
        total = int(a[a.index("--genomes") + 1]) if "--genomes" in a else 15000
        procs = int(a[a.index("--procs") + 1]) if "--procs" in a else 5
        search(a[1], int(a[2]), total, procs)
    elif a[0] == "summary":
        summary()
    elif a[0] == "gates":
        gates()
    elif a[0] == "ideas":
        ideas()
