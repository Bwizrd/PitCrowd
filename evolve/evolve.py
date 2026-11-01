#!/usr/bin/env python3
"""The evolving pit: spawn genomes, backtest them, seat the survivors on the paper desk, and cut
or graduate them on their forward record. The rules are fixed in README.md before any result;
this file implements them and nothing else.

    python3 -I evolve.py cycle            # one cycle: spawn, backtest, review, seat
    python3 -I evolve.py cycle --spawn 400
    python3 -I evolve.py status           # the pit, the bench, the record
    python3 -I evolve.py rebacktest       # after a fix to the backtest: same genomes, same N
    python3 -I evolve.py loop --every 3600

State, all in this directory:
    genomes.jsonl   every genome ever tried, with its backtest numbers (the N the bar counts)
    pit.json        the seated genomes; the paper desk reloads it every minute
    registry.jsonl  every event: tried, seated, cut, graduated, with the rule that fired
"""
import datetime as D, json, math, os, random, subprocess, sys, time

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
GENOMES = os.path.join(HERE, "genomes.jsonl")
PIT = os.path.join(HERE, "pit.json")
REGISTRY = os.path.join(HERE, "registry.jsonl")
TRADES = os.path.join(ROOT, "opencry-visual/log/paper_desk_trades.jsonl")
JEV_LOG = os.path.join(ROOT, "opencry-visual/log/genome_jev.jsonl")
# The backtester: $OPENCRY_TARGET_DIR (a CARGO_TARGET_DIR the binaries were built into) first,
# then the crate's own target/.
BACKTEST = [p for p in (os.path.join(os.environ["OPENCRY_TARGET_DIR"], "release/genome_backtest") if os.environ.get("OPENCRY_TARGET_DIR") else "",
                        os.path.join(ROOT, "opencry-visual/target/release/genome_backtest")) if p]
sys.path.insert(0, HERE)   # python3 -I leaves the script's folder off the path
from dsr import deflated_sharpe, expected_max_sharpe   # the referee (evolve/dsr.py)

# ---- the pre-registered rules (README.md says the same; change both or neither) -------------
SEATS = 40
HOLDOUT_FROM = int(D.datetime(2025, 1, 1, tzinfo=D.UTC).timestamp() * 1000)
GATE = dict(is_n=200, is_t=2.0, is_mean=0.03, ho_n=60, ho_mean=0.0, ho_t=1.0, per_weekday=1.0)
# The three trade-count cuts of 2026-10-25 were replaced on 2026-10-26 by Wald's SPRT (README,
# "Forward cuts"): they cut a genome with Jax9's own backtest edge 81% of the time.
CUT = [  # (min forward trades, rule, label)
    (30, lambda n, m, t: t <= -3.0, "t <= -3 after 30+ trades"),
]
SPRT = dict(alpha=0.05, beta=0.20)
IDLE_DAYS, IDLE_MIN = 21, 20
GRADUATE = dict(n=300, mean=0.05, dsr=0.95)
TWIN = dict(checks=(60, 150, 300), n=300, min_no_share=0.15, yes_mean=0.05, z=1.645)
# ---------------------------------------------------------------------------------------------

# ---- the show floor (README "Show floor"; written 2026-10-27, before any show seat traded) ---
# Seats to watch, apart from the gate's: never counted in N's seated_ever, never graduated.
SHOW = dict(genomes=16, holders=24, adapter_pairs=4, tactics=5)
SHOW_GENOME = dict(is_n=200, ho_n=60, per_weekday=0.5, per_kind=2)     # both halves' means > 0
SHOW_HOLDER = dict(is_n=20, ho_n=8, per_year=4.0, per_symbol=5, overlap=0.5)
SHOW_CUT = dict(n=40, t=-2.5)    # plus the SPRT for show genomes and holders
HOLDER_RESULTS = os.path.join(HERE, "holders", "results.jsonl")
HOLDER_TRADES = os.path.join(HERE, "holders", "trades")
STARS = ("Vega Rigel Deneb Altair Mira Lyra Sirius Pollux Castor Spica Antares Capella Electra Maia Merope Nova "
         "Orion Atlas Hydra Draco Carina Ara Norma Pavo Tucana Indus Volans Lepus Corvus Crater Aquila Cygnus "
         "Perseus Cetus Ursa Leo Virgo Libra Lupus Musca Pyxis Vela Puppis Fornax Sculptor Phoenix Grus Dorado "
         "Mensa Octans Pictor Reticula Caelum Columba Lynx Vulpecula Sagitta Delphinus Equuleus Lacerta").split()
CHAMELEONS = "Kameleon Proteus Morph Shifter Mimic Tinker Turncoat Weathervane Drifter Switch Juggler Mosaic".split()
# ---------------------------------------------------------------------------------------------

SYMBOLS = ["XAUUSD", "XAGUSD", "GBPUSD", "EURUSD", "UK100", "US500", "NAS100", "US30"]
BASKETS = [["EURUSD", "GBPUSD"], ["XAUUSD", "XAGUSD"], ["US500", "NAS100", "US30"], ["UK100"],
           ["US500", "NAS100", "US30", "UK100"], ["EURUSD", "GBPUSD", "XAUUSD"], SYMBOLS]
WINDOWS = ["all", "london", "new_york", "core", "london_open", "new_york_open"]
NAMES = ("Abe Ada Alf Amos Ari Aston Bart Bess Bram Cass Cleo Cora Dale Dot Drew Edda Effie Elm Enzo Esme "
         "Faye Fern Finn Flo Gene Gil Gwen Hal Hettie Hugo Ida Iggy Ines Jago Jem Juno Kai Kip Kizzy Lars "
         "Leni Lou Mabel Mack Moss Nell Ned Noor Oki Olive Otto Pax Peg Pip Quill Ray Rhea Rune Saul Sid "
         "Skye Tad Tilly Ugo Una Val Vic Wes Wren Wyn Yara Yves Zed Zia Bo Dov Eve Ike Jax Lyle Max "
         "Nico Oz Pru Rex Sam Tam Vi Wil Xav Yan Zoe").split()


def now_ms():
    return int(time.time() * 1000)


def log(event, **kw):
    with open(REGISTRY, "a") as f:
        f.write(json.dumps({"ts": D.datetime.now(D.UTC).isoformat(timespec="seconds"), "event": event, **kw}) + "\n")


def load_jsonl(p):
    return [json.loads(l) for l in open(p)] if os.path.exists(p) else []


def stats(r):
    n = len(r)
    if n < 2:
        return {"n": n, "mean": (r[0] if r else 0.0), "t": 0.0}
    m = sum(r) / n
    sd = math.sqrt(sum((x - m) ** 2 for x in r) / (n - 1)) or 1e-9
    return {"n": n, "mean": round(m, 4), "t": round(m / sd * math.sqrt(n), 2)}


# ---- spawning ------------------------------------------------------------------------------

# The first six families ran alone for the first 5,000 genomes; the seven added on 2026-10-25
# are drawn twice as often until they have had a comparable share (README, "Idea families").
FAMILIES = [("breakout", 1), ("band", 1), ("rsi", 1), ("run", 1), ("orb", 1), ("prev_day", 1),
            ("prev_week", 2), ("asia", 2), ("cross", 2), ("squeeze", 2), ("wick", 2), ("engulf", 2), ("drive", 2),
            ("edge_lab", 6)]
# edge-lab's intraday strategies that passed edge_lab.py's checks (README, "edge-lab strategies").
EDGE_SIGNALS = os.path.join(HERE, "edge_lab", "signals")
EDGE_ELIGIBLE = os.path.join(HERE, "edge_lab", "eligible.json")


EDGE_TF = {"15m": 15, "30m": 30, "1h": 60}


def edge_keys():
    """(key, tf) for each eligible strategy on its own timeframe. The first build ran every
    strategy on 15m bars; those signals stay (so the genomes tried on them can be re-run) but
    only a strategy's own timeframe is drawn."""
    if not os.path.exists(EDGE_ELIGIBLE):
        return []
    return sorted((e["key"], e["tf"]) for e in json.load(open(EDGE_ELIGIBLE))["eligible"] if e["tf"] == EDGE_TF[e["interval"]])


def random_signal(rng):
    fams = [(f, w) for f, w in FAMILIES if f != "edge_lab" or edge_keys()]
    k = rng.choices([f for f, _ in fams], weights=[w for _, w in fams])[0]
    if k == "edge_lab":
        key, tf = rng.choice(edge_keys())
        # 15m is the default, left out so a genome is the same shape as one tried before tf existed.
        return {"kind": k, "key": key, **({"tf": tf} if tf != 15 else {})}
    if k == "breakout": return {"kind": k, "n": rng.choice([8, 16, 32, 64, 96, 192])}
    if k == "band": return {"kind": k, "n": rng.choice([20, 48, 96, 192]), "k": rng.choice([1.5, 2.0, 2.5, 3.0])}
    if k == "rsi": return {"kind": k, "n": rng.choice([2, 3, 5, 14]), "lo": rng.choice([5.0, 10.0, 20.0, 30.0])}
    if k == "run": return {"kind": k, "k": rng.choice([3, 4, 5, 6])}
    if k == "orb": return {"kind": k, "m": rng.choice([1, 2, 4])}
    if k == "cross":
        fast = rng.choice([8, 20, 50])
        return {"kind": k, "fast": fast, "slow": rng.choice([s for s in (32, 100, 200, 400) if s > fast])}
    if k == "squeeze": return {"kind": k, "k": rng.choice([4, 7, 12, 20])}
    if k == "wick": return {"kind": k, "w": rng.choice([0.5, 0.6, 0.7])}
    if k == "drive": return {"kind": k, "k": rng.choice([1, 2, 4]), "x": rng.choice([0.5, 1.0, 1.5, 2.0])}
    return {"kind": k}


def random_filter(rng):
    u = rng.random()
    if u < 0.5:
        return {"kind": "trend", "n": rng.choice([50, 100, 200, 400, 800]), "with": rng.random() < 0.5}
    if u < 0.75:
        return {"kind": "day", "with": rng.random() < 0.5}
    return {"kind": "vol", "high": rng.random() < 0.5}


def random_genome(rng):
    filters = [random_filter(rng) for _ in range(rng.choice([0, 0, 1, 1, 2]))]
    return {"signal": random_signal(rng), "mode": rng.choice(["follow", "fade"]), "filters": filters,
            "window": rng.choice(WINDOWS),
            "exit": {"stop_atr": rng.choice([0.75, 1.0, 1.5, 2.0, 3.0]), "target_r": rng.choice([1.0, 1.5, 2.0, 3.0]),
                     "max_bars": rng.choice([4, 8, 16, 32, 64])},
            "symbols": list(rng.choice(BASKETS))}


def mutate(parent, rng):
    g = json.loads(json.dumps({k: parent[k] for k in ("signal", "mode", "filters", "window", "exit", "symbols")}))
    what = rng.choice(["signal_param", "mode", "filter", "window", "stop", "target", "bars", "symbols"])
    if what == "signal_param":
        # A fresh draw of the same kind (another kind's parameter of the same name can be the
        # wrong type: a band's k = 1.5 is not a run's k; two such genomes were tried, and failed).
        for _ in range(200):
            fresh = random_signal(rng)
            if fresh["kind"] == g["signal"]["kind"]:
                g["signal"] = fresh
                break
    elif what == "mode": g["mode"] = "fade" if g["mode"] == "follow" else "follow"
    elif what == "filter":
        if g["filters"] and rng.random() < 0.4: g["filters"].pop(rng.randrange(len(g["filters"])))
        elif len(g["filters"]) < 2: g["filters"].append(random_filter(rng))
        else: g["filters"][rng.randrange(2)] = random_filter(rng)
    elif what == "window": g["window"] = rng.choice(WINDOWS)
    elif what == "stop": g["exit"]["stop_atr"] = rng.choice([0.75, 1.0, 1.5, 2.0, 3.0])
    elif what == "target": g["exit"]["target_r"] = rng.choice([1.0, 1.5, 2.0, 3.0])
    elif what == "bars": g["exit"]["max_bars"] = rng.choice([4, 8, 16, 32, 64])
    else: g["symbols"] = list(rng.choice(BASKETS))
    g["parent"] = parent["id"]
    return g


def shape(g):
    """Genomes that are the same genome, whatever their name (never backtested twice)."""
    return json.dumps({k: g[k] for k in ("signal", "mode", "filters", "window", "exit", "symbols")}, sort_keys=True)


def idea(g):
    """Genomes that take the same entries, whatever their exit: one seat per idea."""
    return json.dumps({k: g[k] for k in ("signal", "mode", "filters", "window", "symbols")}, sort_keys=True)


# ---- backtest and gate ---------------------------------------------------------------------

def run_backtest(genomes):
    exe = next((p for p in BACKTEST if os.path.exists(p)), None)
    if not exe:
        raise SystemExit("genome_backtest not built (cd opencry-visual && cargo build --release --bin genome_backtest)")
    inp = "".join(json.dumps(g) + "\n" for g in genomes)
    out = subprocess.run([exe, "--edge-dir", EDGE_SIGNALS], input=inp, capture_output=True, text=True, check=True).stdout
    return {d["id"]: d.get("trades", []) for d in map(json.loads, out.splitlines()) if "id" in d}


def judge(trades, n_tried):
    is_r = [r for t, r in trades if t < HOLDOUT_FROM]
    ho_r = [r for t, r in trades if t >= HOLDOUT_FROM]
    s_is, s_ho = stats(is_r), stats(ho_r)
    span_days = (trades[-1][0] - trades[0][0]) / 864e5 if len(trades) > 1 else 1
    per_weekday = len(trades) / max(1.0, span_days * 5 / 7)
    G = GATE
    passed = (s_is["n"] >= G["is_n"] and s_is["t"] >= G["is_t"] and s_is["mean"] >= G["is_mean"] and
              s_ho["n"] >= G["ho_n"] and s_ho["mean"] > G["ho_mean"] and s_ho["t"] >= G["ho_t"] and
              per_weekday >= G["per_weekday"])
    return {"is": s_is, "ho": s_ho, "per_weekday": round(per_weekday, 2),
            "dsr_is": round(deflated_sharpe(is_r, n_tried), 4) if len(is_r) >= 20 else 0.0, "pass": passed}


# ---- forward review --------------------------------------------------------------------------

def forward_trades():
    by = {}
    for row in load_jsonl(TRADES):
        gid = row.get("genome")
        if gid:
            by.setdefault(gid, []).append(row)
    return by


def welch(a, b):
    if len(a) < 2 or len(b) < 2:
        return 0.0
    ma, mb = sum(a) / len(a), sum(b) / len(b)
    va = sum((x - ma) ** 2 for x in a) / (len(a) - 1)
    vb = sum((x - mb) ** 2 for x in b) / (len(b) - 1)
    return (ma - mb) / math.sqrt(va / len(a) + vb / len(b) or 1e-12)


def twin_split(gid, rows):
    """The genome's forward trades tagged with Jev's answer at their entry (README, Jev twins)."""
    asks = [a for a in load_jsonl(JEV_LOG) if a.get("genome") == gid and a.get("p") is not None]
    yes, no = [], []
    for t in rows:
        a = next((a for a in asks if a.get("symbol") == t.get("symbol") and abs(a["decided_ms"] - t["opened_ms"]) <= 120_000), None)
        if a is not None:
            (yes if a["p"] >= 0.5 else no).append(t["pnl_r"])
    s_yes, s_no = stats(yes), stats(no)
    return {"n": len(yes) + len(no), "yes": s_yes, "no": s_no, "t": round(welch(yes, no), 2)}


def review(pit, n_seated_ever):
    fwd = forward_trades()
    keep, out = [], []
    for seat in pit:
        g = seat["genome"]
        rows = [r for r in fwd.get(g["id"], []) if r.get("opened_ms", 0) >= seat["seated_ms"]]
        r = [x["pnl_r"] for x in rows]
        s = stats(r)
        seat["forward"] = s
        if seat.get("twin"):
            tw = twin_split(g["id"], rows)
            twin_rows = [x for x in fwd.get(g["id"] + "-J", []) if x.get("opened_ms", 0) >= seat["seated_ms"]]
            tw["twin_forward"] = stats([x["pnl_r"] for x in twin_rows])
            for c in TWIN["checks"]:
                if tw["n"] >= c and c not in seat.get("twin_checks", []):
                    seat.setdefault("twin_checks", []).append(c)
                    log("twin_check", id=g["id"], name=g["name"], at=c, split=tw)
            if tw["n"] >= TWIN["n"] and "twin_verdict" not in seat:
                n_judged = 1 + sum(1 for x in pit if x.get("twin_verdict"))
                no_share = tw["no"]["n"] / max(1, tw["n"])
                z = tw["t"] - expected_max(n_judged)
                adds = no_share >= TWIN["min_no_share"] and tw["yes"]["mean"] >= TWIN["yes_mean"] and z >= TWIN["z"]
                seat["twin_verdict"] = {"adds": adds, "z": round(z, 2), "no_share": round(no_share, 3), "n_judged": n_judged, "ms": now_ms()}
                log("twin_verdict", id=g["id"], name=g["name"], adds=adds, split=tw, z=round(z, 2), n_judged=n_judged)
            seat["twin_split"] = tw
        days = (now_ms() - seat["seated_ms"]) / 864e5
        verdict = None
        for min_n, rule, label in CUT:
            if s["n"] >= min_n and rule(s["n"], s["mean"], s["t"]):
                verdict = ("cut", label)
        sp = sprt_state(seat, r)
        if verdict is None and sp["state"] == "zero":
            verdict = ("cut", f"SPRT accepts no edge (LLR {sp['llr']:+.2f} <= {sp['lo']:+.2f})")
        if sp["state"] == "confirmed" and "sprt_confirmed" not in seat:
            seat["sprt_confirmed"] = now_ms()
            log("sprt_confirmed", id=g["id"], name=g["name"], forward=s, sprt=sp)
        if verdict is None and days >= IDLE_DAYS and s["n"] < IDLE_MIN:
            verdict = ("cut", f"idle: under {IDLE_MIN} trades in {IDLE_DAYS} days")
        if verdict is None and s["n"] >= GRADUATE["n"] and s["mean"] >= GRADUATE["mean"] and not seat.get("graduated"):
            dsr = deflated_sharpe(r, max(1, n_seated_ever))
            seat["forward"]["dsr"] = round(dsr, 4)
            if dsr >= GRADUATE["dsr"]:
                seat["graduated"] = now_ms()
                log("graduated", id=g["id"], name=g["name"], forward=s, dsr=round(dsr, 4), n_seated_ever=n_seated_ever)
        if verdict:
            log("cut", id=g["id"], name=g["name"], rule=verdict[1], forward=s, days=round(days, 1))
            out.append(seat)
        else:
            keep.append(seat)
    return keep, out


def sprt_state(seat, r):
    """Wald's SPRT on the forward R per trade: H0 mean 0 against H1 the genome's own holdout mean,
    normal with its backtest's standard deviation, both fixed the first time it is reviewed
    (README, "Forward cuts"). Once a boundary is crossed the test is over."""
    if "sprt" not in seat:
        g = seat["genome"]
        bt = [x for _, x in run_backtest([g]).get(g["id"], [])]
        m = sum(bt) / len(bt)
        seat["sprt"] = {"mu1": seat["backtest"]["ho"]["mean"],
                        "sigma": round(math.sqrt(sum((x - m) ** 2 for x in bt) / (len(bt) - 1)), 4)}
    mu1, sd = seat["sprt"]["mu1"], seat["sprt"]["sigma"]
    lo, hi = math.log(SPRT["beta"] / (1 - SPRT["alpha"])), math.log((1 - SPRT["beta"]) / SPRT["alpha"])
    llr, state = 0.0, "open"
    for x in r:
        llr += mu1 / sd ** 2 * (x - mu1 / 2)
        if llr <= lo or llr >= hi:
            state = "zero" if llr <= lo else "confirmed"
            break
    return {"llr": round(llr, 3), "lo": round(lo, 3), "hi": round(hi, 3), "state": state, "mu1": mu1, "sigma": sd}


# ---- the cycle -------------------------------------------------------------------------------

def expected_max(n):
    """E[max of n standard normals] (evolve/dsr.py)."""
    return expected_max_sharpe(n)


def load_pit():
    return json.load(open(PIT)) if os.path.exists(PIT) else {"seats": [], "retired": [], "seated_ever": 0}


def save_pit(p):
    tmp = PIT + ".tmp"
    json.dump(p, open(tmp, "w"), indent=1)
    os.replace(tmp, PIT)   # the desk never reads half a file


def cycle(spawn=400, seed=None):
    rng = random.Random(seed if seed is not None else time.time_ns())
    tried = load_jsonl(GENOMES)
    seen = {shape(g["genome"]) for g in tried}
    pit = load_pit()
    used_names = {g["genome"]["name"] for g in tried}
    # Parents: seated genomes doing well forward, then the best backtest passers.
    parents = [s["genome"] for s in pit["seats"] if s.get("forward", {}).get("n", 0) >= 30 and s["forward"]["mean"] > 0]
    # Then the best in-sample genomes, passers or not: mutation searches near what came close.
    # (Every mutant is a new try and counts in N like any other.)
    # A handful of identical trades has an enormous t and says nothing: parents need 100 trades.
    near = [g for g in tried if g["result"]["is"]["n"] >= 100]
    parents += [g["genome"] for g in sorted(near, key=lambda g: (-g["result"]["pass"], -g["result"]["is"]["t"]))[:30]]
    fresh, n0 = [], len(tried)
    while len(fresh) < spawn:
        g = mutate(rng.choice(parents), rng) if parents and rng.random() < 0.3 else random_genome(rng)
        if shape(g) in seen:
            continue
        seen.add(shape(g))
        k = n0 + len(fresh) + 1
        g["id"] = f"g{k:05d}"
        base = NAMES[(k - 1) % len(NAMES)]
        g["name"] = base if base not in used_names else f"{base}{(k - 1) // len(NAMES) + 1}"
        used_names.add(g["name"])
        fresh.append(g)
    results = run_backtest(fresh)
    n_tried = n0 + len(fresh)
    passed = 0
    with open(GENOMES, "a") as f:
        for g in fresh:
            res = judge(results.get(g["id"], []), n_tried) if results.get(g["id"]) else {"pass": False, "is": stats([]), "ho": stats([]), "per_weekday": 0, "dsr_is": 0}
            passed += res["pass"]
            f.write(json.dumps({"genome": g, "result": res, "tried_ms": now_ms()}) + "\n")
    log("tried", n=len(fresh), passed=passed, n_tried=n_tried)
    # Forward review, then fill empty seats with the best untried-on-desk passers.
    keep, cut = review(pit["seats"], pit["seated_ever"])
    pit["seats"] = keep
    pit["retired"] = (pit.get("retired", []) + [{"id": s["genome"]["id"], "name": s["genome"]["name"], "forward": s.get("forward"), "retired_ms": now_ms()} for s in cut])[-500:]
    on_desk = {s["genome"]["id"] for s in keep} | {r["id"] for r in pit["retired"]}
    seated_shapes = {idea(s["genome"]) for s in keep}
    bench = sorted((g for g in load_jsonl(GENOMES) if g["result"]["pass"] and g["genome"]["id"] not in on_desk),
                   key=lambda g: -g["result"]["dsr_is"] if g["result"]["dsr_is"] else -g["result"]["is"]["t"] / 100)
    for g in bench:
        if len(pit["seats"]) >= SEATS:
            break
        if idea(g["genome"]) in seated_shapes:
            continue
        pit["seats"].append({"genome": g["genome"], "seated_ms": now_ms(), "backtest": g["result"], "twin": True})
        seated_shapes.add(idea(g["genome"]))
        pit["seated_ever"] += 1
        log("seated", id=g["genome"]["id"], name=g["genome"]["name"], backtest=g["result"])
    show_cut = fill_show(pit, load_jsonl(GENOMES), rng)
    pit["updated_ms"] = now_ms()
    pit["n_tried"] = n_tried
    save_pit(pit)
    print(f"tried {len(fresh)} (total {n_tried}), {passed} passed the gate; cut {len(cut)}; pit {len(pit['seats'])}/{SEATS}; "
          f"show floor {len(pit['show'])} (cut {show_cut})")


# ---- the show floor ----------------------------------------------------------------------------

def min_t(r):
    return min(r["is"]["t"], r["ho"]["t"])


def show_genome_pool(tried, strict_ideas):
    """Genomes that made money in both halves (README "Show floor"), best first by the smaller
    of their two t's. Not edge-lab genomes (each would call Python at every bar)."""
    G = SHOW_GENOME
    pool = [g for g in tried if g["result"]["is"]["n"] >= G["is_n"] and g["result"]["is"]["mean"] > 0
            and g["result"]["ho"]["n"] >= G["ho_n"] and g["result"]["ho"]["mean"] > 0
            and g["result"]["per_weekday"] >= G["per_weekday"] and g["genome"]["signal"]["kind"] != "edge_lab"
            and idea(g["genome"]) not in strict_ideas]
    return sorted(pool, key=lambda g: -min_t(g["result"]))


def holder_pool():
    """(strategy, symbol) pairs that made money in both halves, best first by the smaller t."""
    if not os.path.exists(HOLDER_RESULTS):
        return []
    H, out = SHOW_HOLDER, []
    for r in load_jsonl(HOLDER_RESULTS):
        if r.get("excluded"):
            continue
        for sym, x in r["symbols"].items():
            if (x["is"]["n"] >= H["is_n"] and x["is"]["mean"] > 0 and x["ho"]["n"] >= H["ho_n"] and x["ho"]["mean"] > 0
                    and x["per_year"] >= H["per_year"]):
                out.append({"key": r["key"], "tf": r["tf"], "symbol": sym, "title": r.get("title", ""), "bt": x,
                            "edge_lab_forward": r.get("edge_lab_forward")})
    return sorted(out, key=lambda c: -min(c["bt"]["is"]["t"], c["bt"]["ho"]["t"]))


def holder_entries(c):
    try:
        return {t[0] for t in json.load(open(os.path.join(HOLDER_TRADES, f"{c['key']}@{c['tf']}", f"{c['symbol']}.json")))}
    except OSError:
        return set()


def make_tactics(pool, rng, k, prefer=()):
    """k tactics with different signal kinds: the preferred first, then drawn from the top of the pool."""
    out, kinds = [], set()
    for g in list(prefer) + rng.sample(pool[:80], min(80, len(pool))):
        if g["signal"]["kind"] in kinds or any(t["id"] == g["id"] for t in out):
            continue
        out.append({k2: g[k2] for k2 in ("id", "name", "signal", "mode", "filters", "window", "exit", "symbols")})
        kinds.add(g["signal"]["kind"])
        if len(out) == k:
            break
    return out


def review_show(show):
    """Forward cuts for the show floor: the SPRT (genomes and holders), or t <= -2.5 after 40 trades.
    An adapter pair leaves together, so the code-or-Jev comparison stays a pair."""
    fwd = forward_trades()
    keep, out = [], []
    for seat in show:
        g = seat["genome"]
        r = [x["pnl_r"] for x in fwd.get(g["id"], []) if x.get("opened_ms", 0) >= seat["seated_ms"]]
        s = stats(r)
        seat["forward"] = s
        why = None
        if s["n"] >= SHOW_CUT["n"] and s["t"] <= SHOW_CUT["t"]:
            why = f"t <= {SHOW_CUT['t']} after {SHOW_CUT['n']}+ trades"
        if seat["kind"] in ("genome", "holder"):
            sp = sprt_state(seat, r)
            seat["sprt_llr"] = sp["llr"]
            if why is None and sp["state"] == "zero":
                why = f"SPRT accepts no edge (LLR {sp['llr']:+.2f})"
            if sp["state"] == "confirmed" and "sprt_confirmed" not in seat:
                seat["sprt_confirmed"] = now_ms()
                log("show_confirmed", id=g["id"], name=g["name"], kind=seat["kind"], forward=s, sprt=sp)
        (out if why else keep).append(seat)
        if why:
            seat["why"] = why
    gone_pairs = {s["pair"] for s in out if s["kind"] == "adapter"}
    for s in [s for s in keep if s["kind"] == "adapter" and s["pair"] in gone_pairs]:
        keep.remove(s); out.append(s); s["why"] = "its pair-mate was cut"
    for s in out:
        log("show_cut", id=s["genome"]["id"], name=s["genome"]["name"], kind=s["kind"], rule=s["why"], forward=s["forward"])
    return keep, out


def fill_show(pit, tried, rng):
    show = pit.setdefault("show", [])
    keep, cut = review_show(show)
    pit["show"] = keep
    pit["show_retired"] = (pit.get("show_retired", []) + [{"id": s["genome"]["id"], "name": s["genome"]["name"], "kind": s["kind"],
                           "why": s["why"], "forward": s.get("forward"), "retired_ms": now_ms()} for s in cut])[-500:]
    # A show genome the gate has since seated leaves the show floor (the gate's seat is the one that counts).
    strict_ideas = {idea(s["genome"]) for s in pit["seats"]}
    for s in [s for s in keep if s["kind"] == "genome" and idea(s["genome"]) in strict_ideas]:
        keep.remove(s)
        pit["show_retired"].append({"id": s["genome"]["id"], "name": s["genome"]["name"], "kind": "genome", "why": "seated by the gate",
                                    "forward": s.get("forward"), "retired_ms": now_ms()})
        log("show_cut", id=s["genome"]["id"], name=s["genome"]["name"], kind="genome", rule="seated by the gate")
    been = {s["genome"]["id"] for s in keep} | {r["id"] for r in pit["show_retired"]}
    used_names = {s["genome"]["name"] for s in keep} | {r["name"] for r in pit["show_retired"]}
    pool = show_genome_pool(tried, strict_ideas)

    def name_from(names, fallback):
        for n in names + [f"{n}{k}" for k in range(2, 50) for n in names]:
            if n not in used_names:
                used_names.add(n); return n
        return fallback

    # Genomes: at most two of a signal kind, one seat per idea.
    have = [s for s in keep if s["kind"] == "genome"]
    kinds = {}
    for s in have:
        kinds[s["genome"]["signal"]["kind"]] = kinds.get(s["genome"]["signal"]["kind"], 0) + 1
    ideas = {idea(s["genome"]) for s in have}
    for g in pool:
        if len(have) >= SHOW["genomes"]:
            break
        gg, kind = g["genome"], g["genome"]["signal"]["kind"]
        sid = "s_" + gg["id"]   # its own id on the desk, apart from the gate's seats
        if sid in been or idea(gg) in ideas or kinds.get(kind, 0) >= SHOW_GENOME["per_kind"]:
            continue
        gg = {**gg, "id": sid, "floor": "show"}
        seat = {"genome": gg, "kind": "genome", "seated_ms": now_ms(), "backtest": g["result"]}
        keep.append(seat); have.append(seat); ideas.add(idea(gg)); kinds[kind] = kinds.get(kind, 0) + 1
        log("show_seated", id=gg["id"], name=gg["name"], kind="genome", backtest=g["result"])

    # Holders: one per strategy, at most five per symbol, none sharing half its entries with another.
    hold = [s for s in keep if s["kind"] == "holder"]
    per_sym, keys = {}, {s["genome"]["signal"]["key"] for s in hold}
    entries = {s["genome"]["id"]: holder_entries({"key": s["genome"]["signal"]["key"], "tf": s["genome"]["signal"]["tf"], "symbol": s["genome"]["symbols"][0]}) for s in hold}
    for s in hold:
        per_sym[s["genome"]["symbols"][0]] = per_sym.get(s["genome"]["symbols"][0], 0) + 1
    for c in holder_pool():
        if len(hold) >= SHOW["holders"]:
            break
        hid = f"h_{c['key']}@{c['tf']}_{c['symbol']}"
        if hid in been or c["key"] in keys or per_sym.get(c["symbol"], 0) >= SHOW_HOLDER["per_symbol"]:
            continue
        e = holder_entries(c)
        if not e or any(s["genome"]["symbols"][0] == c["symbol"] and len(e & entries[s["genome"]["id"]]) >= SHOW_HOLDER["overlap"] * min(len(e), len(entries[s["genome"]["id"]]))
                        for s in hold):
            continue
        g = {"id": hid, "name": name_from(STARS, hid), "signal": {"kind": "edge_lab", "key": c["key"], "tf": c["tf"]}, "mode": "follow",
             "filters": [], "window": "all", "exit": {"stop_atr": 2.0, "target_r": 1.0, "max_bars": 0}, "symbols": [c["symbol"]],
             "holds": True, "floor": "show"}
        bt = c["bt"]
        seat = {"genome": g, "kind": "holder", "seated_ms": now_ms(), "title": c["title"], "edge_lab_forward": c["edge_lab_forward"],
                "backtest": {"is": bt["is"], "ho": bt["ho"], "per_year": bt["per_year"]},
                "sprt": {"mu1": bt["ho"]["mean"], "sigma": bt["all"].get("sd") or 1.0}}
        keep.append(seat); hold.append(seat); keys.add(c["key"]); entries[hid] = e
        per_sym[c["symbol"]] = per_sym.get(c["symbol"], 0) + 1
        log("show_seated", id=hid, name=g["name"], kind="holder", title=c["title"], backtest=seat["backtest"])

    # Adapters: pairs with the same tactics, one choosing by code, one by Jev. A new pair starts
    # from the two show genomes with the best forward record (20+ trades), then fresh draws.
    pairs = {s["pair"] for s in keep if s["kind"] == "adapter"}
    best = sorted((s for s in keep if s["kind"] == "genome" and s.get("forward", {}).get("n", 0) >= 20 and s["forward"]["mean"] > 0),
                  key=lambda s: -s["forward"]["mean"])[:2]
    pool_g = [g["genome"] for g in pool]
    next_pair = max([s.get("pair", 0) for s in keep] + [r.get("pair", 0) for r in pit["show_retired"]] + [pit.get("adapter_pairs_ever", 0)])
    while len(pairs) < SHOW["adapter_pairs"] and len(pool_g) >= SHOW["tactics"]:
        next_pair += 1
        tactics = make_tactics(pool_g, rng, SHOW["tactics"], prefer=[s["genome"] for s in best])
        base = name_from(CHAMELEONS, f"a{next_pair:03d}")
        for mode in ("code", "jev"):
            aid = f"a{next_pair:03d}{'j' if mode == 'jev' else 'c'}"
            nm = base + ("-J" if mode == "jev" else "")   # a pair shares a name, the Jev one with "-J"
            g = {"id": aid, "name": nm, **{k: tactics[0][k] for k in ("signal", "mode", "filters", "window", "exit", "symbols")},
                 "tactics": tactics, "adapt": mode, "floor": "show"}
            keep.append({"genome": g, "kind": "adapter", "pair": next_pair, "seated_ms": now_ms(), "backtest": {}})
            log("show_seated", id=aid, name=nm, kind="adapter", mode=mode, tactics=[t["id"] for t in tactics])
        pairs.add(next_pair)
        pit["adapter_pairs_ever"] = next_pair
    pit["show"] = keep
    return len(cut)


def rebacktest():
    """Re-run every genome already tried through the current backtest (after a fix to the
    backtest itself). The same genomes, so N does not change."""
    tried = load_jsonl(GENOMES)
    results = run_backtest([t["genome"] for t in tried])
    n = len(tried)
    tmp = GENOMES + ".tmp"
    with open(tmp, "w") as f:
        for t in tried:
            tr = results.get(t["genome"]["id"], [])
            t["result"] = judge(tr, n) if tr else {"pass": False, "is": stats([]), "ho": stats([]), "per_weekday": 0, "dsr_is": 0}
            f.write(json.dumps(t) + "\n")
    os.replace(tmp, GENOMES)
    log("rebacktest", n=n, passed=sum(t["result"]["pass"] for t in tried))
    print(f"re-backtested {n}, {sum(t['result']['pass'] for t in tried)} pass")


def status():
    pit = load_pit()
    tried = load_jsonl(GENOMES)
    print(f"{len(tried)} genomes tried, {sum(g['result']['pass'] for g in tried)} passed the gate, "
          f"{pit['seated_ever']} seated ever, {len(pit['seats'])} in the pit, {len(pit.get('retired', []))} retired")
    for s in sorted(pit["seats"], key=lambda s: -s.get("forward", {}).get("mean", 0)):
        g, b, f = s["genome"], s["backtest"], s.get("forward", {"n": 0, "mean": 0, "t": 0})
        print(f"  {g['name']:8} {g['id']}  bt IS {b['is']['mean']:+.3f} t{b['is']['t']:+.1f} HO {b['ho']['mean']:+.3f}  "
              f"fwd n{f['n']:4d} {f['mean']:+.3f} t{f['t']:+.1f}  {'GRADUATED ' if s.get('graduated') else ''}"
              f"{g['signal']['kind']}/{g['mode']} {g['window']} {' '.join(g['symbols'])}")
        if s.get("twin"):
            tw = s.get("twin_split") or {"n": 0, "yes": stats([]), "no": stats([]), "t": 0.0}
            v = s.get("twin_verdict")
            print(f"           twin {g['name']}-J: Jev yes n{tw['yes']['n']} {tw['yes']['mean']:+.3f}R, no n{tw['no']['n']} {tw['no']['mean']:+.3f}R, "
                  f"t {tw['t']:+.2f}{'  VERDICT: Jev ' + ('adds something' if v['adds'] else 'adds nothing') if v else ''}")


if __name__ == "__main__":
    a = sys.argv[1:]
    if not a or a[0] == "status":
        status()
    elif a[0] == "rebacktest":
        rebacktest()
    elif a[0] == "show":
        pit = load_pit()
        n = fill_show(pit, load_jsonl(GENOMES), random.Random(time.time_ns()))
        pit["updated_ms"] = now_ms()
        save_pit(pit)
        print(f"show floor {len(pit['show'])} seats (cut {n})")
    elif a[0] == "cycle":
        cycle(spawn=int(a[a.index("--spawn") + 1]) if "--spawn" in a else 400)
    elif a[0] == "loop":
        every = int(a[a.index("--every") + 1]) if "--every" in a else 3600
        while True:
            try:
                cycle()
            except Exception as e:
                log("error", error=f"{type(e).__name__}: {e}")
                print("cycle failed:", e, flush=True)
            time.sleep(every)
