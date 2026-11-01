#!/usr/bin/env python3
"""How much of the pit's search is luck: every genome tried, judged together.

    evolve/audit/.venv/bin/python evolve/audit/snooping.py    (needs numpy, scipy, arch)

Input: out/trials_trades.jsonl, every one of the 15,000 genomes tried up to 2026-10-26 re-run on
the data/real snapshot (losers too: they are what the corrections are made of).

1. Effective number of independent trials, from the eigenvalues of the correlation matrix of
   the genomes' daily R (computed through the days x days dual, so 15,000 genomes are cheap):
   the participation ratio, the Li-Ji count, and the number of eigenvalues for 90% / 95% of
   the variance.
2. The deflated Sharpe ratio (Bailey & Lopez de Prado 2014) of each seated genome, per trade,
   with N = the effective trials and the benchmark scaled by the spread of the trials' own
   Sharpe ratios (edge-lab's harness assumes that spread is 1/sqrt(n) and N independent).
3. The probability of backtest overfitting by CSCV (Bailey, Borwein, Lopez de Prado, Zhu):
   16 blocks of days, all 12,870 halvings, choose the best in-sample genome by t, rank it out
   of sample. Over the genomes that trade at least once a weekday (the only ones the gate
   can pass), and over all.
4. Hansen's SPA and Romano-Wolf StepM (arch.bootstrap), stationary bootstrap of days: is the
   best daily-R genome better than not trading, given all that were tried?
"""
import itertools, json, math, os, sys
import numpy as np
from scipy import stats as st

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(HERE, "out")
HOLDOUT_FROM = 1735689600000
DAY = 86_400_000
SEATED = {"g00815": "Jax9", "g10941": "Rex119", "g14368": "Edda157"}
EULER = 0.5772156649


def emax(n):
    return 0.0 if n <= 1 else (1 - EULER) * st.norm.ppf(1 - 1 / n) + EULER * st.norm.ppf(1 - 1 / (n * math.e))


def load():
    meta = {}
    for l in open(os.path.join(HERE, "trials_15000.jsonl")):
        d = json.loads(l)
        meta[d["genome"]["id"]] = d
    trades = {}
    for l in open(os.path.join(OUT, "trials_trades.jsonl")):
        d = json.loads(l)
        if "id" in d:
            trades[d["id"]] = d["trades"]
    return meta, trades


def daily(trades, ids, lo=None, hi=None):
    days = sorted({t // DAY for i in ids for t, _ in trades[i] if (lo is None or t >= lo) and (hi is None or t < hi)})
    ix = {d: k for k, d in enumerate(days)}
    X = np.zeros((len(days), len(ids)), np.float32)
    for j, i in enumerate(ids):
        for t, r in trades[i]:
            if (lo is None or t >= lo) and (hi is None or t < hi):
                X[ix[t // DAY], j] += r
    return np.array(days), X


def effective_trials(X):
    Z = X - X.mean(0)
    sd = Z.std(0)
    keep = sd > 0
    Z = Z[:, keep] / sd[keep]
    K = (Z @ Z.T) / Z.shape[0]                 # days x days dual of the genomes' correlation
    lam = np.clip(np.linalg.eigvalsh(K.astype(np.float64)), 0, None)[::-1]
    m = keep.sum()
    pr = lam.sum() ** 2 / (lam ** 2).sum()
    liji = sum((1 if l >= 1 else 0) + (l - math.floor(l)) for l in lam)
    cum = np.cumsum(lam) / lam.sum()
    return {"genomes": int(m), "days": int(Z.shape[0]), "participation_ratio": round(float(pr), 1),
            "li_ji": round(float(liji), 1), "eig_90": int(np.searchsorted(cum, 0.90) + 1), "eig_95": int(np.searchsorted(cum, 0.95) + 1),
            "top_eigen_share": [round(float(x), 3) for x in (lam[:5] / lam.sum())]}


def dsr(r, sr0):
    r = np.asarray(r)
    n = len(r)
    sr = r.mean() / r.std(ddof=1)
    sk, ku = st.skew(r), st.kurtosis(r, fisher=False)
    z = (sr - sr0) * math.sqrt(n - 1) / math.sqrt(1 - sk * sr + (ku - 1) / 4 * sr ** 2)
    return float(st.norm.cdf(z)), float(sr)


def pbo(X, S=16, chunk=1500):
    T, M = X.shape
    edges = np.linspace(0, T, S + 1).astype(int)
    s1 = np.stack([X[edges[k]:edges[k + 1]].sum(0) for k in range(S)]).astype(np.float64)
    s2 = np.stack([(X[edges[k]:edges[k + 1]].astype(np.float64) ** 2).sum(0) for k in range(S)])
    nb = np.diff(edges).astype(np.float64)
    combos = np.array([[1.0 if k in c else 0.0 for k in range(S)] for c in itertools.combinations(range(S), S // 2)])
    logits, oos_of_best, is_of_best = [], [], []
    for a in range(0, len(combos), chunk):
        C = combos[a:a + chunk]
        for W in (C, 1 - C):
            pass
        def tstat(W):
            n = W @ nb
            m = (W @ s1) / n[:, None]
            v = (W @ s2) / n[:, None] - m ** 2
            return m / np.sqrt(np.maximum(v, 1e-12)) * np.sqrt(n)[:, None], m
        t_is, _ = tstat(C)
        t_oos, m_oos = tstat(1 - C)
        best = t_is.argmax(1)
        rows = np.arange(len(C))
        rank = (t_oos < t_oos[rows, best][:, None]).sum(1) + 0.5 * ((t_oos == t_oos[rows, best][:, None]).sum(1) - 1)
        w = (rank + 1) / (M + 1)
        logits.append(np.log(w / (1 - w)))
        oos_of_best.append(m_oos[rows, best]); is_of_best.append(t_is[rows, best])
    lg = np.concatenate(logits)
    oos = np.concatenate(oos_of_best)
    return {"pbo": round(float((lg <= 0).mean()), 3), "median_logit": round(float(np.median(lg)), 2),
            "best_is_oos_daily_R_median": round(float(np.median(oos)), 4), "best_is_oos_negative": round(float((oos < 0).mean()), 3),
            "combos": int(len(lg)), "genomes": int(M), "days": int(T)}


def spa(X, reps=2000, block=10, seed=20261026):
    from arch.bootstrap import SPA, StepM
    losses = -X.astype(np.float64)
    bench = np.zeros(X.shape[0])
    s = SPA(bench, losses, block_size=block, reps=reps, bootstrap="stationary", seed=seed)
    s.compute()
    sm = StepM(bench, losses, size=0.05, block_size=block, reps=reps, bootstrap="stationary", seed=seed)
    sm.compute()
    return {"pvalues": {k: round(float(v), 4) for k, v in s.pvalues.items()}, "stepm_superior": [int(x) for x in sm.superior_models]}


def main():
    meta, trades = load()
    ids = [i for i in meta if trades.get(i)]
    print(f"{len(meta)} genomes tried, {len(ids)} with trades on the snapshot")
    report = {"n_tried": len(meta), "with_trades": len(ids)}
    days, X = daily(trades, ids)
    eff = effective_trials(X)
    report["effective_trials"] = eff
    print("effective trials:", eff)
    elig = [k for k, i in enumerate(ids) if meta[i]["result"]["per_weekday"] >= 1.0]
    eff_e = effective_trials(X[:, elig])
    report["effective_trials_weekday_eligible"] = eff_e
    print("effective trials, genomes trading 1+/weekday:", eff_e)
    # Deflated Sharpe, per trade: sr0 = sd of the trials' per-trade Sharpe x E[max of N_eff].
    srs, ns = [], []
    for i in ids:
        r = np.array([x for _, x in trades[i] if _ < HOLDOUT_FROM])
        if len(r) >= 30 and r.std() > 0:
            srs.append(r.mean() / r.std(ddof=1)); ns.append(len(r))
    srs, ns = np.array(srs), np.array(ns)
    # The spread of Sharpe ratios that sampling alone gives is about 1/sqrt(n); report both.
    report["trial_sharpe"] = {"n_trials_30plus": int(len(srs)), "sd": round(float(srs.std()), 4),
                              "sd_of_t": round(float((srs * np.sqrt(ns)).std()), 3), "mean_t": round(float((srs * np.sqrt(ns)).mean()), 3)}
    print("trial in-sample per-trade Sharpe:", report["trial_sharpe"])
    report["dsr"] = {}
    for gid, name in SEATED.items():
        r_all = [x for _, x in trades[gid]]
        r_is = [x for t, x in trades[gid] if t < HOLDOUT_FROM]
        out = {}
        for label, N in (("N=15000", len(meta)), ("N=participation", eff["participation_ratio"]), ("N=li_ji", eff["li_ji"]), ("N=eig95", eff["eig_95"])):
            # Benchmark in t units: sd of the trials' t-stats times E[max of N], at this genome's n.
            t0 = report["trial_sharpe"]["sd_of_t"] * emax(N)
            p_is, sr = dsr(r_is, t0 / math.sqrt(len(r_is)))
            p_harness = float(st.norm.cdf(sr * math.sqrt(len(r_is)) - emax(N)))   # edge-lab's form, for comparison
            out[label] = {"N": N, "E_max": round(emax(N), 3), "benchmark_t": round(t0, 3), "dsr_is": round(p_is, 4), "harness_form": round(p_harness, 4)}
        out["is_t"] = round(float(np.mean(r_is) / np.std(r_is, ddof=1) * math.sqrt(len(r_is))), 3)
        report["dsr"][name] = out
        print(name, json.dumps(out))
    for label, cols in (("weekday_eligible", elig), ("all", list(range(len(ids))))):
        p = pbo(X[:, cols])
        report[f"pbo_{label}"] = p
        print(f"PBO ({label}):", p)
    # SPA / StepM over the weekday-eligible genomes (all daily series, whole history), and in-sample only.
    for label, lo, hi in (("all_history", None, None), ("in_sample", None, HOLDOUT_FROM), ("holdout", HOLDOUT_FROM, None)):
        d2, X2 = daily(trades, [ids[k] for k in elig], lo, hi)
        res = spa(X2)
        sup = [ids[elig[k]] for k in res["stepm_superior"]]
        res["stepm_superior"] = [(i, meta[i]["genome"]["name"]) for i in sup][:50]
        res["n_superior"] = len(sup)
        report[f"spa_{label}"] = res
        print(f"SPA ({label}, {X2.shape[1]} genomes, {X2.shape[0]} days):", res["pvalues"], f"StepM superior: {len(sup)}", res["stepm_superior"][:8])
    json.dump(report, open(os.path.join(OUT, "snooping.json"), "w"), indent=1)


if __name__ == "__main__":
    main()
