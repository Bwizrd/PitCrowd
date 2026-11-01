#!/usr/bin/env python3
"""A second feed: OANDA's 15m bid and ask candles for the four index symbols, fetched slowly
(one request at a time, 1.5 s apart, 5,000 candles each), then written as a backtest cache.

    python3 -I evolve/audit/oanda.py fetch      # data/oanda/raw_<instrument>.jsonl, resumable
    python3 -I evolve/audit/oanda.py cache      # data/oanda/ctrader_<SYM>_SB_15m.json (bid bars)
                                                #   + spreads_<SYM>.json {bar ms: ask open - bid open}

The token is read from cTraderApi/.env and never printed. The other four desk symbols are
copied from data/real (no index genome trades them; the backtest needs all eight files).
"""
import datetime as D, json, os, shutil, sys, time, urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(HERE, "data", "oanda")
ENV = os.environ.get("OPENCRY_ENV_FILE", os.path.join(HERE, "..", "..", ".env"))   # holds OANDA_TOKEN=
HOST = "https://api-fxpractice.oanda.com"
INSTRUMENTS = {"US500": "SPX500_USD", "NAS100": "NAS100_USD", "US30": "US30_USD", "UK100": "UK100_GBP"}
START = "2021-01-03T00:00:00Z"
PAUSE = 1.5


def token():
    return next(l.split("=", 1)[1].strip().strip("\"'") for l in open(ENV) if l.startswith("OANDA_TOKEN="))


def get(inst, frm):
    u = f"{HOST}/v3/instruments/{inst}/candles?price=BA&granularity=M15&from={frm}&count=5000"
    req = urllib.request.Request(u, headers={"Authorization": "Bearer " + token()})
    return json.load(urllib.request.urlopen(req, timeout=60))["candles"]


def fetch():
    os.makedirs(OUT, exist_ok=True)
    for sym, inst in INSTRUMENTS.items():
        path = os.path.join(OUT, f"raw_{inst}.jsonl")
        last = None
        if os.path.exists(path):
            for line in open(path):
                last = json.loads(line)["time"]
        frm = last or START
        with open(path, "a") as f:
            while True:
                cs = [c for c in get(inst, frm) if c["complete"] and c["time"] != last]
                time.sleep(PAUSE)
                if not cs:
                    break
                for c in cs:
                    f.write(json.dumps(c) + "\n")
                last = frm = cs[-1]["time"]
                print(f"{inst}: to {last[:16]}", flush=True)
                if len(cs) < 100:
                    break


def ms(t):
    return int(D.datetime.strptime(t[:19], "%Y-%m-%dT%H:%M:%S").replace(tzinfo=D.UTC).timestamp() * 1000)


def cache():
    for sym, inst in INSTRUMENTS.items():
        rows, spreads = {}, {}
        for line in open(os.path.join(OUT, f"raw_{inst}.jsonl")):
            c = json.loads(line)
            t, b, a = ms(c["time"]), c["bid"], c["ask"]
            rows[t] = [t, float(b["o"]), float(b["h"]), float(b["l"]), float(b["c"]), 0.0]
            spreads[t] = round(float(a["o"]) - float(b["o"]), 6)
        json.dump([rows[t] for t in sorted(rows)], open(os.path.join(OUT, f"ctrader_{sym}_SB_15m.json"), "w"))
        json.dump(spreads, open(os.path.join(OUT, f"spreads_{sym}.json"), "w"))
        print(sym, len(rows), "bars")
    for sym in ("XAUUSD", "XAGUSD", "GBPUSD", "EURUSD"):
        shutil.copy(os.path.join(HERE, "data", "real", f"ctrader_{sym}_SB_15m.json"), OUT)


if __name__ == "__main__":
    {"fetch": fetch, "cache": cache}[sys.argv[1]]()
