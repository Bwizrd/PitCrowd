#!/usr/bin/env python3
"""How the forward rules behave on edges the size the pit actually finds.

Draws forward trade sequences by resampling a seated genome's own backtest trades (shifted so
the true mean is a chosen value: 0 = no edge, the holdout mean, the full backtest mean) and
runs each rule set over them trade by trade, as review() does. Prints, per rule set and true
mean: how often the genome is cut, how often it graduates/confirms, and the median trades to
a verdict.

    python3 -I evolve/audit/forward_rules.py <backtest-output.jsonl> [--paths 4000]
"""
import json, math, sys
import numpy as np

HERE = __import__("os").path.dirname(__import__("os").path.abspath(__file__))
sys.path.insert(0, __import__("os").path.join(HERE, ".."))
from dsr import expected_max_sharpe as _expected_max_sharpe   # evolve/dsr.py

HORIZON = 3000


def running_t(cum, cum2, n):
    m = cum / n
    var = np.maximum((cum2 - n * m * m) / np.maximum(n - 1, 1), 1e-12)
    return m, m / np.sqrt(var) * np.sqrt(n)


def old_rules(x, n_seated):
    """README's cuts and graduation as of 2026-10-25."""
    P, H = x.shape
    n = np.arange(1, H + 1)
    cum, cum2 = np.cumsum(x, 1), np.cumsum(x * x, 1)
    m, t = running_t(cum, cum2, n)
    cut = ((n >= 30) & (t <= -2)) | ((n >= 60) & (m < 0)) | ((n >= 150) & (m < 0.05))
    grad = (n >= 300) & (m >= 0.05) & (t - _expected_max_sharpe(max(1, n_seated)) >= 1.645)
    return first(cut, grad)


def sprt(x, mu1, sigma, alpha=0.05, beta=0.20):
    """Wald's SPRT on the mean, H0 mu=0 vs H1 mu=mu1, normal with the backtest's sigma."""
    llr = np.cumsum(mu1 / sigma ** 2 * (x - mu1 / 2), 1)
    lo, hi = math.log(beta / (1 - alpha)), math.log((1 - beta) / alpha)
    return first(llr <= lo, llr >= hi)


def first(cut, win):
    """Per path: ('cut'|'win'|'open', trade index of the verdict)."""
    H = cut.shape[1]
    ic = np.where(cut.any(1), cut.argmax(1), H)
    iw = np.where(win.any(1), win.argmax(1), H)
    v = np.where((ic < iw), 1, np.where(iw < ic, 2, 0))
    return v, np.minimum(ic, iw) + 1


def report(label, v, at):
    out = []
    for code, name in ((1, "cut"), (2, "confirmed"), (0, f"open at {HORIZON}")):
        sel = v == code
        med = int(np.median(at[sel])) if sel.any() and code else None
        out.append(f"{name} {sel.mean():5.1%}" + (f" (median {med})" if med else ""))
    print(f"    {label:26} " + "   ".join(out))


def main():
    path = sys.argv[1]
    paths = int(sys.argv[sys.argv.index("--paths") + 1]) if "--paths" in sys.argv else 4000
    rng = np.random.default_rng(20261026)
    gen = {json.loads(l)["genome"]["id"]: json.loads(l)["result"] for l in open(f"{HERE}/../genomes.jsonl")}
    for line in open(path):
        d = json.loads(line)
        r = np.array([x[1] for x in d["trades"]])
        res = gen.get(d["id"], {})
        ho = res.get("ho", {}).get("mean", r.mean())
        sigma = r.std(ddof=1)
        per_week = len(r) / ((d["trades"][-1][0] - d["trades"][0][0]) / 6048e5)
        print(f"{d['id']}: backtest n {len(r)}, mean {r.mean():+.4f}, holdout {ho:+.4f}, sd {sigma:.3f}, {per_week:.1f} trades/week")
        for z, zname in ((1.645, "95% one-sided"),):
            for mu in (r.mean(), ho):
                need = ((z + 0.8416) * sigma / mu) ** 2
                print(f"  fixed-n test, {zname}, 80% power, true mean {mu:+.4f}: {need:,.0f} trades = {need / per_week / 52:.1f} years")
        for true_mu, name in ((0.0, "no edge"), (ho, "holdout mean"), (r.mean(), "backtest mean")):
            x = rng.choice(r - r.mean() + true_mu, size=(paths, HORIZON))
            print(f"  true mean {true_mu:+.4f} ({name}):")
            report("old cuts + graduation", *old_rules(x, 4))
            report("SPRT a=.05 b=.20, mu1=HO", *sprt(x, ho, sigma))
        print()


if __name__ == "__main__":
    main()
