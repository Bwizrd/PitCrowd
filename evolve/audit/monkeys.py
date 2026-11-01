#!/usr/bin/env python3
"""Random-entry monkeys for a seated genome: its exits, window, basket and number of trades, with
the entries moved at random. If the genome's numbers are ordinary among its monkeys, its signal
adds nothing to the timing and exposure it already has.

    python3 -I evolve/audit/monkeys.py <genome-id> [--n 1000]

Three kinds, 1,000 each by default, on the data/real snapshot:
  side      the genome's own entry bars, the side a coin flip (does it pick the direction?)
  day       each of the genome's trading days moved whole to a random other day: the same
            symbols and times of day, the genome's own sides (does it pick the day? the drift
            and the long/short mix are kept)
  day+side  a random day and a coin-flip side

Monkeys run through the edge-lab entry hook (an entries file per monkey), so they are backtested
by the same code as every genome. Each is judged by the seating gate with N = 15,000.
"""
import json, os, random, subprocess, sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.dirname(HERE))
import evolve as E   # noqa: E402

REAL = os.path.join(HERE, "data", "real")
BAR = 15 * 60 * 1000
N_TRIED = 15000


def bt(genomes, edge_dir, detail=False):
    exe = next(p for p in E.BACKTEST if os.path.exists(p))
    out = subprocess.run([exe, "--cache", REAL, "--edge-dir", edge_dir] + (["--detail"] if detail else []), input="".join(json.dumps(g) + "\n" for g in genomes),
                         capture_output=True, text=True, check=True).stdout
    return {d["id"]: d["trades"] for d in map(json.loads, out.splitlines()) if "id" in d}


def main():
    gid = sys.argv[1]
    n = int(sys.argv[sys.argv.index("--n") + 1]) if "--n" in sys.argv else 1000
    g = next(json.loads(l)["genome"] for l in open(os.path.join(HERE, "trials_15000.jsonl")) if json.loads(l)["genome"]["id"] == gid)
    edge_dir = os.path.join(HERE, "data", "monkeys", gid)
    os.makedirs(edge_dir, exist_ok=True)
    real = bt([g], edge_dir, detail=True)[gid]
    own = [(sym, t - BAR) for t, _, sym, _, _ in real]          # the signal bar before each fill
    side = {(sym, t - BAR): 1 if sd == "long" else -1 for t, _, sym, sd, _ in real}
    real = [(t, r) for t, r, *_ in real]
    bars = {s: [r[0] for r in json.load(open(f"{REAL}/ctrader_{s}_SB_15m.json"))] for s in g["symbols"]}
    sets = {s: set(v) for s, v in bars.items()}
    base = {k: g[k] for k in ("window", "exit", "symbols")}
    base["mode"], base["filters"] = "follow", []
    DAY = 86_400_000
    by_day = {}
    for sym, t in own:
        by_day.setdefault(t // DAY, []).append((sym, t % DAY))
    def fits(day, trades):
        return all(day * DAY + tod in sets[sym] and day * DAY + tod + BAR in sets[sym] for sym, tod in trades)
    all_days = sorted({t // DAY for ts in bars.values() for t in ts})
    print(f"{gid} {g['name']}: {len(real)} trades, {sum(v > 0 for v in side.values())} long, {sum(v < 0 for v in side.values())} short")
    rng = random.Random(f"monkeys-{gid}")
    genomes = []
    for kind in ("side", "day", "day+side"):
        for k in range(n):
            ent = {s: [] for s in g["symbols"]}
            if kind == "side":
                for sym, t in own:
                    ent[sym].append([t, rng.choice((1, -1))])
            else:
                # Each of the genome's trading days moves whole to a different random day, so the
                # monkey keeps its spacing within the day and its number of trades.
                free = all_days[:]
                rng.shuffle(free)
                for day, trades in by_day.items():
                    to = next((i for i, d in enumerate(free) if fits(d, trades)), None)
                    if to is None:
                        continue
                    d2 = free.pop(to)
                    for sym, tod in trades:
                        t2 = d2 * DAY + tod
                        ent[sym].append([t2, side[(sym, day * DAY + tod)] if kind == "day" else rng.choice((1, -1))])
            for s in ent:
                ent[s].sort()
            key = f"{kind.replace('+', '_')}_{k:04d}"
            json.dump(ent, open(f"{edge_dir}/{key}@15.json", "w"))
            genomes.append({**base, "signal": {"kind": "edge_lab", "key": key}, "id": f"{kind}:{k}", "name": key})
    res = bt(genomes, edge_dir)
    res = {k: [tuple(x) for x in v] for k, v in res.items()}
    rows = []
    jr = E.judge(real, N_TRIED)
    full = lambda tr: sum(r for _, r in tr) / max(1, len(tr))
    print(f"  the genome: IS {jr['is']['mean']:+.4f} t {jr['is']['t']:+.2f}, HO {jr['ho']['mean']:+.4f} t {jr['ho']['t']:+.2f}, all {full(real):+.4f} R/trade")
    for kind in ("side", "day", "day+side"):
        js = [(E.judge(res[f"{kind}:{k}"], N_TRIED), full(res[f"{kind}:{k}"]), len(res[f"{kind}:{k}"])) for k in range(n)]
        p_all = sum(f >= full(real) for _, f, _ in js) / n
        p_is = sum(j["is"]["t"] >= jr["is"]["t"] for j, _, _ in js) / n
        passed = sum(j["pass"] for j, _, _ in js)
        fs = sorted(f for _, f, _ in js)
        print(f"  {kind:9} monkeys: trades {sorted(x for *_, x in js)[n // 2]}, R/trade median {fs[n // 2]:+.4f} "
              f"[5%-95% {fs[n // 20]:+.4f} to {fs[-n // 20]:+.4f}]; share >= the genome: all {p_all:.3f}, IS t {p_is:.3f}; "
              f"{passed} of {n} pass the gate")
        rows.append({"kind": kind, "p_all": p_all, "p_is_t": p_is, "pass": passed, "median": fs[n // 2], "q05": fs[n // 20], "q95": fs[-n // 20]})
    json.dump({"genome": gid, "real": jr, "real_all": full(real), "monkeys": rows},
              open(os.path.join(HERE, "out", f"monkeys_{gid}.json"), "w"), indent=1)


if __name__ == "__main__":
    main()
