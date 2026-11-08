#!/usr/bin/env python3
"""A starting point for your own live feed: fill in `history()` and `quote()`, nothing else.

    floor_tape --ingest XAUUSD                       # the tape takes this symbol from you
    python3 adapters/my_feed.py --symbol XAUUSD      # your copy of this file

Then check it arrived: `curl localhost:8083/health` names the symbol's source as ingest.
Walkthrough: docs/DATA_SOURCES.md, "Adding your own feed". A complete working example of the
same shape is adapters/kraken_btc.py.

Use one of the floor's nine names (XAUUSD XAGUSD NAS100 US500 EURUSD GBPUSD BTCUSD US30 UK100).
Your instrument may be something else: map it onto the nearest pit.
Stdlib only, so it runs anywhere Python 3 does.
"""
import argparse, json, sys, time, urllib.request


def history(symbol):
    """Recent one-minute bars, oldest first: [[t_ms, open, high, low, close, volume], ...].

    t_ms is the bar's open in Unix milliseconds, UTC. A few hours is plenty; charts and the
    paper desk backfill from it. Return [] if your source has no history: the floor still runs,
    the charts just start empty.
    """
    raise NotImplementedError("fetch recent 1m bars from your source")


def quote(symbol):
    """The price now, as (bid, ask). With only a last price, return (last, last)."""
    raise NotImplementedError("fetch the current bid and ask from your source")


def post(tape, path, payload):
    req = urllib.request.Request(tape + path, data=json.dumps(payload).encode(),
                                 headers={"Content-Type": "application/json"}, method="POST")
    with urllib.request.urlopen(req, timeout=10) as r:
        return json.load(r)


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--tape", default="http://localhost:8083")
    ap.add_argument("--symbol", required=True, help="one of the floor's nine names")
    ap.add_argument("--every", type=float, default=1.0, help="seconds between quotes")
    a = ap.parse_args()
    tape = a.tape.rstrip("/")

    # 1. History first: the tape drops a tick older than its newest bar, so send bars before quotes.
    bars = history(a.symbol)
    if bars:
        r = post(tape, "/ingest/bars", {"symbol": a.symbol, "bars": bars})
        if not r.get("ok"):
            sys.exit(f"tape refused the history: {r.get('errors')}")
        print(f"backfilled {r['taken']} one-minute bars of {a.symbol}", flush=True)

    # 2. Then quotes, for as long as it runs. A pit with no tick for 90 s halts as quiet.
    last = None
    while True:
        try:
            bid, ask = quote(a.symbol)
            if (bid, ask) != last:
                r = post(tape, "/ingest", {"symbol": a.symbol, "bid": bid, "ask": ask})
                if not r.get("ok"):
                    print(f"tape refused: {r.get('errors')}", file=sys.stderr, flush=True)
                last = (bid, ask)
        except NotImplementedError:
            raise
        except Exception as e:   # a dropped call is skipped, not fatal
            print(f"skipped: {e}", file=sys.stderr, flush=True)
        time.sleep(a.every)


if __name__ == "__main__":
    main()
