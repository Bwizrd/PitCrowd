#!/usr/bin/env python3
"""Real Bitcoin prices for the floor, from Kraken's public REST API (no account, no key).

An example of a live adapter: it reads a source, and POSTs what it reads to floor_tape. Copy it
for any other source; the only parts that change are `history()` and `quote()`.

    floor_tape --ingest BTCUSD                 # BTC from here, the other eight synthetic
    python3 adapters/kraken_btc.py             # backfills 12h of 1m bars, then a quote a second

With real BTC, real Level 2 lines up with the pit too: open the floor with ?l2=kraken.

Options: --tape http://localhost:8083  --every 1.0  --pair XBTUSD  --symbol BTCUSD
Stdlib only. Be gentle with a free API: Kraken allows about one public call a second.
"""
import argparse, json, sys, time, urllib.request

KRAKEN = "https://api.kraken.com/0/public"


def get(url):
    with urllib.request.urlopen(urllib.request.Request(url, headers={"User-Agent": "opencry-floor-adapter"}), timeout=10) as r:
        body = json.load(r)
    if body.get("error"):
        raise RuntimeError(", ".join(body["error"]))
    return body["result"]


def post(tape, path, payload):
    req = urllib.request.Request(tape + path, data=json.dumps(payload).encode(), headers={"Content-Type": "application/json"}, method="POST")
    with urllib.request.urlopen(req, timeout=10) as r:
        return json.load(r)


def history(pair):
    """The last 720 one-minute bars: [[t_ms, open, high, low, close, trades], ...]."""
    result = get(f"{KRAKEN}/OHLC?pair={pair}&interval=1")
    rows = next(v for k, v in result.items() if k != "last")
    return [[int(r[0]) * 1000, float(r[1]), float(r[2]), float(r[3]), float(r[4]), int(r[7])] for r in rows]


def quote(pair):
    """(bid, ask) now."""
    t = next(iter(get(f"{KRAKEN}/Ticker?pair={pair}").values()))
    return float(t["b"][0]), float(t["a"][0])


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--tape", default="http://localhost:8083")
    ap.add_argument("--every", type=float, default=1.0)
    ap.add_argument("--pair", default="XBTUSD")
    ap.add_argument("--symbol", default="BTCUSD")
    a = ap.parse_args()
    tape = a.tape.rstrip("/")

    bars = history(a.pair)
    r = post(tape, "/ingest/bars", {"symbol": a.symbol, "bars": bars})
    if not r.get("ok"):
        sys.exit(f"tape refused the history: {r.get('errors')}")
    print(f"backfilled {r['taken']} one-minute bars of {a.symbol}", flush=True)

    last = None
    while True:
        try:
            bid, ask = quote(a.pair)
            if (bid, ask) != last:
                r = post(tape, "/ingest", {"symbol": a.symbol, "bid": bid, "ask": ask})
                if not r.get("ok"):
                    print(f"tape refused: {r.get('errors')}", file=sys.stderr, flush=True)
                last = (bid, ask)
        except Exception as e:   # a dropped call is skipped, not fatal
            print(f"skipped: {e}", file=sys.stderr, flush=True)
        time.sleep(a.every)


if __name__ == "__main__":
    main()
