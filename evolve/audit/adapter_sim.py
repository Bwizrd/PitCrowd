#!/usr/bin/env python3
"""What the code adapters would have done on the history: each day, the tactic with the best R per
trade over the 20 days before (3+ trades, else the one in use), and that tactic's trades that day.
Against each tactic alone and all five at once. Day boundaries at 06:45 UTC, as on the desk.

    python3 -I evolve/audit/adapter_sim.py        # the adapter pairs on the show floor now
"""
import json, math, os, subprocess, sys

HERE = os.path.dirname(os.path.abspath(__file__))
EVOLVE = os.path.dirname(HERE)
ROOT = os.path.dirname(EVOLVE)
EXE = os.path.join(ROOT, "opencry-visual/target/release/genome_backtest")
HOLDOUT = 1735689600000
DAY, SHIFT = 86_400_000, (6 * 60 + 45) * 60_000


def stats(r):
    n = len(r)
    if n < 2:
        return f"n {n:5d}"
    m = sum(r) / n
    sd = math.sqrt(sum((x - m) ** 2 for x in r) / (n - 1)) or 1e-9
    return f"n {n:5d} {m:+.4f} R/trade t {m / sd * math.sqrt(n):+5.2f}"


def main():
    pit = json.load(open(os.path.join(EVOLVE, "pit.json")))
    seen = set()
    for seat in pit.get("show", []):
        if seat["kind"] != "adapter" or seat["pair"] in seen:
            continue
        seen.add(seat["pair"])
        tactics = seat["genome"]["tactics"]
        inp = "".join(json.dumps(t) + "\n" for t in tactics)
        out = subprocess.run([EXE, "--edge-dir", os.path.join(EVOLVE, "edge_lab/signals"), "--detail"], input=inp, capture_output=True, text=True, check=True).stdout
        tr = {d["id"]: d["trades"] for d in map(json.loads, out.splitlines()) if "id" in d}
        by = [sorted(tr.get(t["id"], []), key=lambda x: x[0]) for t in tactics]
        day = lambda ms: (ms - SHIFT) // DAY
        days = sorted({day(x[0]) for b in by for x in b})
        cur, chosen = 0, []
        for d in days:
            lo, hi = (d - 20) * DAY + SHIFT, d * DAY + SHIFT
            sc = []
            for k, b in enumerate(by):
                r = [x[1] for x in b if lo <= x[0] < hi]
                if len(r) >= 3:
                    sc.append((sum(r) / len(r), k))
            if sc:
                cur = max(sc)[1]
            chosen += [x for x in by[cur] if day(x[0]) == d]
        print(f"pair {seat['pair']}: {', '.join(t['signal']['kind'] + '/' + t['mode'] for t in tactics)}")
        for name, rows in [("adapter (code)", chosen), ("all five", [x for b in by for x in b])] + [(f"tactic {k + 1} alone", b) for k, b in enumerate(by)]:
            print(f"  {name:16} all {stats([x[1] for x in rows])}  |  2021-24 {stats([x[1] for x in rows if x[0] < HOLDOUT])}  |  2025-26 {stats([x[1] for x in rows if x[0] >= HOLDOUT])}")


if __name__ == "__main__":
    main()
