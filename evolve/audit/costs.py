#!/usr/bin/env python3
"""The seated genomes on a second feed and at real spreads.

    python3 -I evolve/audit/costs.py      (after oanda.py fetch + cache)

For each seated genome: the gate's numbers on
  ctrader          the data/real snapshot, the backtest's typical spread (what seated it)
  ctrader +1 tick  every fill, in and out, one tick worse
  oanda            OANDA's bid bars, the same typical spread
  oanda spreads    OANDA's bid bars, each bar's own OANDA spread (ask open - bid open)
  oanda spr +1     the same, one tick worse each side
then OANDA's spread at the genome's own entries against the typical one, the desk's recorded
tape spreads against OANDA's on the same bars, and how closely the two feeds' bars agree.
"""
import json, math, os, subprocess, sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.dirname(HERE))
import evolve as E   # noqa: E402

REAL, OANDA = os.path.join(HERE, "data", "real"), os.path.join(HERE, "data", "oanda")
TICK = {"US500": 0.25, "NAS100": 0.25, "US30": 1.0, "UK100": 0.5}
TYPICAL = {"US500": 1.6, "NAS100": 4.4, "US30": 2.1, "UK100": 2.6}
SEATED = ["g00815", "g10941", "g14368"]
DESK_TRADES = os.path.join(os.path.dirname(os.path.dirname(HERE)), "opencry-visual/log/paper_desk_trades.jsonl")


def bt(genomes, *args):
    exe = next(p for p in E.BACKTEST if os.path.exists(p))
    out = subprocess.run([exe, *args], input="".join(json.dumps(g) + "\n" for g in genomes), capture_output=True, text=True, check=True).stdout
    return {d["id"]: d["trades"] for d in map(json.loads, out.splitlines()) if "id" in d}


def pct(xs, q):
    xs = sorted(xs)
    return xs[min(len(xs) - 1, int(q * len(xs)))]


def main():
    gs = {json.loads(l)["genome"]["id"]: json.loads(l)["genome"] for l in open(os.path.join(HERE, "trials_15000.jsonl"))}
    genomes = [gs[i] for i in SEATED]
    runs = {
        "ctrader": ["--cache", REAL],
        "ctrader +1 tick": ["--cache", REAL, "--slip-ticks", "1"],
        "oanda": ["--cache", OANDA],
        "oanda spreads": ["--cache", OANDA, "--spreads", OANDA],
        "oanda spr +1": ["--cache", OANDA, "--spreads", OANDA, "--slip-ticks", "1"],
    }
    res = {k: bt(genomes, *a, "--detail") for k, a in runs.items()}
    report = {}
    for g in genomes:
        print(f"{g['name']} ({g['id']}): {g['signal']} {g['mode']} {g['window']} {g['exit']}")
        report[g["id"]] = {}
        for k in runs:
            tr = [(t, r) for t, r, *_ in res[k][g["id"]]]
            j = E.judge(tr, 15000)
            allr = [r for _, r in tr]
            n, m = len(allr), sum(allr) / max(1, len(allr))
            print(f"  {k:16} n {n:5d}  all {m:+.4f}  IS {j['is']['mean']:+.4f} t {j['is']['t']:+.2f}  HO {j['ho']['mean']:+.4f} t {j['ho']['t']:+.2f}  "
                  f"{j['per_weekday']:.2f}/wkday  gate {'PASS' if j['pass'] else 'fail'}")
            report[g["id"]][k] = {"n": n, "mean": round(m, 4), "judge": j}
        # OANDA spread at the genome's own entry bars, in ticks.
        spreads = {s: json.load(open(f"{OANDA}/spreads_{s}.json")) for s in g["symbols"]}
        for s in g["symbols"]:
            at = [spreads[s].get(str(t)) for t, _, sym, *_ in res["oanda spreads"][g["id"]] if sym == s]
            at = [x / TICK[s] for x in at if x]
            allsp = [v / TICK[s] for v in spreads[s].values() if v]
            if at:
                print(f"    {s:6} OANDA spread at its entries: median {pct(at, .5):.1f} ticks, 90% {pct(at, .9):.1f} "
                      f"(every bar: median {pct(allsp, .5):.1f}; the backtest's typical {TYPICAL[s]})")
    # The desk's tape spreads against OANDA's on the same 15m bar.
    print("\nThe desk's recorded tape spread (cTrader) against OANDA's on the same 15m bar, index fills:")
    spreads = {s: json.load(open(f"{OANDA}/spreads_{s}.json")) for s in TICK}
    pairs = {s: [] for s in TICK}
    for l in open(DESK_TRADES):
        d = json.loads(l)
        s = d.get("symbol")
        if s in TICK and d.get("spread_ticks_in") is not None:
            bar = d["opened_ms"] // 900_000 * 900_000
            o = spreads[s].get(str(bar))
            if o:
                pairs[s].append((d["spread_ticks_in"], o / TICK[s]))
    for s, p in pairs.items():
        if p:
            print(f"  {s:6} {len(p):4d} fills: tape median {pct([a for a, _ in p], .5):.1f} ticks, OANDA median {pct([b for _, b in p], .5):.1f}")
    # The two feeds' bars.
    print("\nThe feeds' 15m bars where both have one:")
    for s in TICK:
        a = {r[0]: r for r in json.load(open(f"{REAL}/ctrader_{s}_SB_15m.json"))}
        b = {r[0]: r for r in json.load(open(f"{OANDA}/ctrader_{s}_SB_15m.json"))}
        common = sorted(set(a) & set(b))
        ra = [math.log(a[t][4] / a[t][1]) for t in common]
        rb = [math.log(b[t][4] / b[t][1]) for t in common]
        ma, mb = sum(ra) / len(ra), sum(rb) / len(rb)
        cov = sum((x - ma) * (y - mb) for x, y in zip(ra, rb))
        corr = cov / math.sqrt(sum((x - ma) ** 2 for x in ra) * sum((y - mb) ** 2 for y in rb))
        lvl = [b[t][4] / a[t][4] - 1 for t in common]
        print(f"  {s:6} cTrader {len(a)} bars, OANDA {len(b)}, both {len(common)}; open-to-close correlation {corr:.3f}; "
              f"OANDA/cTrader close median {pct(lvl, .5) * 1e4:+.1f} bp [10-90% {pct(lvl, .1) * 1e4:+.1f} to {pct(lvl, .9) * 1e4:+.1f}]")
    json.dump(report, open(os.path.join(HERE, "out", "costs.json"), "w"), indent=1)


if __name__ == "__main__":
    main()
