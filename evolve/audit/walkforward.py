#!/usr/bin/env python3
"""Folds instead of one holdout.

    python3 -I evolve/audit/walkforward.py

1. The seated genomes half-year by half-year (one holdout is one draw; this shows the draws).
2. The selection itself, walked forward. Half-year folds; for each test fold the gate's
   in-sample rules (200+ trades, t >= 2.0, mean >= +0.03 R, 1+ trade a weekday) pick genomes
   from the training folds, with 5 days embargoed either side of the test fold (genomes are
   flat by the close, so nothing straddles a day; the embargo guards slower effects). Then
   what those picks earn in the test fold, which none of them saw:
     - anchored: training is every fold before the test fold (the desk's own situation);
     - k-fold: training is every other fold (more draws, the future is used to pick).
   Over all 15,000 genomes tried, on out/trials_trades.jsonl.
"""
import json, math, os
import datetime as D

HERE = os.path.dirname(os.path.abspath(__file__))
DAY = 86_400_000
EMBARGO = 5 * DAY
SEATED = {"g00815": "Jax9", "g10941": "Rex119", "g14368": "Edda157"}


def stats(r):
    n = len(r)
    if n < 2:
        return n, (r[0] if r else 0.0), 0.0
    m = sum(r) / n
    sd = math.sqrt(sum((x - m) ** 2 for x in r) / (n - 1)) or 1e-9
    return n, m, m / sd * math.sqrt(n)


def folds():
    out = []
    for y in range(2021, 2027):
        for h in (1, 7):
            a = int(D.datetime(y, h, 1, tzinfo=D.UTC).timestamp() * 1000)
            b = int(D.datetime(y + (h == 7), 7 if h == 1 else 1, 1, tzinfo=D.UTC).timestamp() * 1000)
            out.append((f"{y}{'H1' if h == 1 else 'H2'}", a, b))
    return out


def picks(trades, ids, keep):
    """The gate's in-sample rules on the trades `keep` allows."""
    chosen = []
    for i in ids:
        tr = [(t, r) for t, r in trades[i] if keep(t)]
        if len(tr) < 200:
            continue
        n, m, t = stats([r for _, r in tr])
        span = (tr[-1][0] - tr[0][0]) / DAY
        if t >= 2.0 and m >= 0.03 and n / max(1.0, span * 5 / 7) >= 1.0:
            chosen.append((t, i))
    return sorted(chosen, reverse=True)


def main():
    trades = {}
    for l in open(os.path.join(HERE, "out", "trials_trades.jsonl")):
        d = json.loads(l)
        if "id" in d and d["trades"]:
            trades[d["id"]] = [tuple(x) for x in d["trades"]]
    ids = sorted(trades)
    F = [f for f in folds() if any(f[1] <= t < f[2] for t, _ in trades["g00815"])]
    print("1. The seated genomes by half-year (n, R/trade, t)")
    print("   " + " ".join(f"{f[0]:>15}" for f in F))
    for gid, name in SEATED.items():
        cells = []
        for _, a, b in F:
            n, m, t = stats([r for tt, r in trades[gid] if a <= tt < b])
            cells.append(f"{n:4d} {m:+.3f} {t:+.1f}")
        print(f"   {name:8}" + " ".join(f"{c:>15}" for c in cells))
    report = {"folds": [f[0] for f in F], "anchored": [], "kfold": []}
    for mode in ("anchored", "kfold"):
        print(f"\n2. Selection walked forward ({mode}): the gate's picks on the training folds, in the test fold")
        all_r, best_r = [], []
        for k, (name, a, b) in enumerate(F):
            if mode == "anchored":
                if k < 2:
                    continue
                keep = lambda t, a=a: t < a - EMBARGO
            else:
                keep = lambda t, a=a, b=b: t < a - EMBARGO or t >= b + EMBARGO
            ch = picks(trades, ids, keep)
            test = lambda i: [r for t, r in trades[i] if a <= t < b]
            pr = [r for _, i in ch for r in test(i)]
            br = test(ch[0][1]) if ch else []
            all_r += pr; best_r += br
            n, m, t = stats(pr)
            nb, mb, tb = stats(br)
            ideas = len({i for _, i in ch})
            print(f"   {name}: {len(ch):3d} picks; their test trades n {n:5d} {m:+.4f} R (t {t:+.1f}); "
                  f"the best pick {ch[0][1] if ch else '-':7} n {nb:4d} {mb:+.4f} R")
            report[mode].append({"fold": name, "picks": len(ch), "pick_ids": [i for _, i in ch][:20], "test_n": n, "test_mean": round(m, 4),
                                 "best": ch[0][1] if ch else None, "best_test_n": nb, "best_test_mean": round(mb, 4)})
        n, m, t = stats(all_r); nb, mb, tb = stats(best_r)
        print(f"   pooled over folds: every pick {n} trades {m:+.4f} R/trade (t {t:+.2f}); best pick only {nb} trades {mb:+.4f} R/trade (t {tb:+.2f})")
        report[mode + "_pooled"] = {"all": [n, round(m, 4), round(t, 2)], "best": [nb, round(mb, 4), round(tb, 2)]}
    json.dump(report, open(os.path.join(HERE, "out", "walkforward.json"), "w"), indent=1)


if __name__ == "__main__":
    main()
