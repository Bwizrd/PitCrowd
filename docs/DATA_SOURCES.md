# Data sources: what the floor reads, and how to give it yours

Everything the floor shows comes through one door: **the tape**, a small server on port 8083
that speaks a simple protocol (below). This repo ships one, `floor_tape`, which needs no
account, no key and no internet. Out of the box it runs a synthetic market. To use real prices
you have three options, from least to most work:

| You have | Do this | Section |
|---|---|---|
| nothing (just want to see it run) | `./start-floor.sh` | [Synthetic](#1-synthetic-the-default) |
| recorded prices in files | `FLOOR_TAPE_ARGS="--csv data/recorded" ./start-floor.sh` | [CSV](#2-your-recordings-csv) |
| a live feed (an exchange, a broker API, anything) | `--ingest SYMBOL` + a small adapter that POSTs to the tape | [Ingest](#3-a-live-feed-post-ingest) |
| your own market-data server | speak the protocol on :8083 and skip floor_tape | [The protocol](#4-the-tape-protocol) |

```
  synthetic ─┐
  CSV files ─┼─► floor_tape :8083 ──ws ticks/books──► the floor (browser), the paper desk
  POST /ingest┘        │           ──/bars──────────► charts, backfill, the desk's warm-up
                       │           ──/l2────────────► the Level 2 heatmap and BTC pit walls
                       └─ --write-cache ─► data/cache/*.json ─► genome_backtest, evolve
```

Sources mix per symbol: `floor_tape --csv data/recorded --ingest BTCUSD` replays the symbols
that have a CSV, takes Bitcoin from an adapter, and runs the rest synthetic. `/health` and the
websocket `hello` say which source each symbol is on.

## The nine symbols

The floor has a pit per symbol. A source only has to use these names; the tape adds the ids.

| Name | id | Pit | Name | id | Pit |
|---|---|---|---|---|---|
| `XAUUSD` | 241 | gold | `GBPUSD` | 199 | sterling |
| `XAGUSD` | 238 | silver | `BTCUSD` | 160 | bitcoin |
| `NAS100` | 205 | Nasdaq 100 | `US30` | 219 | Dow |
| `US500` | 220 | S&P 500 | `UK100` | 217 | FTSE 100 |
| `EURUSD` | 185 | euro | | | |

Names are forgiving (`EUR/USD`, `eurusd`). Map your instrument onto the nearest pit: German DAX
data can trade in the `UK100` pit if that is what you have. A symbol with no source simply has
no ticks, and its pit halts as quiet: the floor never invents a price for a real-data pit.

## 1. Synthetic (the default)

`floor_tape` with no arguments. Each symbol's price is a pure function of the seed and the
minute, so the history, the live ticks and the backtest cache are one continuous path, the same
on every machine (`--seed N` for another). It has volatility clustering, a busy London/New York
day and quiet nights, realistic spreads and tick rates, a five-level book, a 500-level BTC
order book with walls that get posted and pulled, and a retail order book for gold, silver and
the two FX pairs.

**It has no edge, by construction.** Returns are uncorrelated from one minute to the next (a
test in `floor_tape/src/sim.rs` holds it to that), so anything that makes money on it did so by
luck. That makes it a useful null world: if a paper trader or an evolved genome "works" on
synthetic prices, that is the false-positive rate of your process, measured. Use real data to
look for real edges.

## 2. Your recordings (CSV)

```sh
floor_tape --csv data/recorded                # play from now at real speed, loop at the end
floor_tape --csv data/recorded --speed 20     # twenty times faster
floor_tape --csv data/recorded --warmup 3d --no-loop
floor_tape --write-csv data/sample --days 2   # sample files in this format, to copy
```

One file per symbol, named after it (`XAUUSD.csv`, `EURUSD.csv`, …). Either ticks or bars,
detected from the header:

```
time,bid,ask                         ticks; ask optional (the symbol's usual spread is added)
1761030000123,3890.12,3890.42
time,open,high,low,close,volume      bars of any length; volume optional
2025-10-21T07:00:00Z,1.1601,1.1609,1.1597,1.1605,412
```

`time` is Unix seconds, Unix milliseconds or ISO 8601 in UTC (`2025-10-21T07:00:00Z`,
`2025-10-21 07:00:00`); the column may also be called `timestamp`, `date`, `datetime` or `t`.
Each bar becomes four ticks: open, the nearer extreme, the farther one, close.

The recording is laid on today's clock: the first `--warmup` of it (default one day, at most
four fifths of it) becomes history, which is what charts and the desk backfill from, and the
rest plays from now. A loop jumps back to the start of the played part.

## 3. A live feed (POST /ingest)

Start the tape with the symbols your feed owns, then POST to it from anything:

```sh
floor_tape --ingest BTCUSD           # or --ingest XAUUSD,EURUSD, or --ingest all
```

```sh
# a quote (t optional: now)
curl -X POST localhost:8083/ingest -d '{"symbol":"BTCUSD","bid":77819.7,"ask":77819.8,"t":1793557003382}'
# many at once: a JSON array, or one object per line
# a book (up to five levels a side, best first)
curl -X POST localhost:8083/ingest -d '{"type":"depth","symbol":"BTCUSD","bids":[[77819.7,1.2],[77819.0,0.4]],"asks":[[77819.8,0.9]]}'
# history, so charts and the desk have something to backfill from (one-minute bars)
curl -X POST localhost:8083/ingest/bars -d '{"symbol":"BTCUSD","bars":[[1793556000000,77862.7,77876.5,77815.8,77819.7,1341]]}'
```

Bars may also be objects (`{"timestamp":…,"open":…,"high":…,"low":…,"close":…,"volume":…}`).
The answer says what was taken and why anything was not (`{"ok":true,"taken":1,"errors":[]}`).
POSTing a symbol that is not an ingest symbol is refused with the flag that would allow it.
A tick older than the newest bar's minute is dropped: send history as bars.

[`adapters/kraken_btc.py`](../adapters/kraken_btc.py) is a complete example: real Bitcoin from
Kraken's public API (no key), backfill then a quote a second, stdlib Python, sixty lines. Copy
it for any other source. Only `history()` and `quote()` change.

Ingest is off unless `--ingest` names symbols, because anyone who can reach the port could
post prices. Under `start-floor.sh` the tape listens on this machine only unless you start it
with `FLOOR_LAN=1`; run by hand, `floor_tape` listens on every address (`--bind 127.0.0.1`
keeps it to this machine).

## 4. The tape protocol

To replace `floor_tape` entirely, serve this on one port (run with `FLOOR_NO_TAPE=1` so the
launcher leaves the port to you). Every HTTP answer is JSON with
`Access-Control-Allow-Origin: *`, because the page fetches it from another port.

**Websocket `ws://host:8083/`**: on connect, a hello; then every change as it happens.

```json
{"type":"hello","symbols":[{"symbol":"XAUUSD","id":241}],"levels":5}
{"type":"tick","data":{"symbolId":241,"symbol":"XAUUSD","t":1793557003382,"bid":3890.12,"ask":3890.42,"spread":0.3}}
{"type":"depth","data":{"symbolId":241,"symbol":"XAUUSD","t":1793557003382,"bids":[[3890.12,40],[3889.97,61]],"asks":[[3890.42,35]]}}
```

`t` is Unix ms. A pit steps only while ticks arrive: no tick for 90 s halts it as quiet, no
message at all for 20 s marks the feed stale. Depth is optional.

**`GET /bars?symbol=XAUUSD&timeframe=1m&range=-6h[&delta=1]`**: OHLC of the bid. `timeframe`
is `Nm`, `Nh` or `Nd`; `range` is `-Nm`, `-Nh`, `-Nd` or `-Nw` back from now. The floor asks
for `1m`/`5m`/`15m` over up to `-21d`.

```json
{"symbol":"XAUUSD","timeframe":"1m","range":"-6h","count":360,
 "data":[{"timestamp":1793535600000,"open":3890.1,"high":3891.0,"low":3889.6,"close":3890.4,"volume":182,
          "upticks":88,"downticks":79,"spread_mean":0.31}]}
```

`timestamp` is the bar's open in ms, `volume` is the tick count, and the three flow fields come
only with `delta=1` (and only where the tape saw the ticks).

**`GET /health`**: anything with `"connected":true` while prices are flowing (the launcher
checks for that string). **`GET /book?symbol=`**: `{"symbol","bid","ask","t","bids","asks"}`.
**`GET /orderbook?symbol=`**: optional, a retail order book in OANDA's shape (`{"time",
"price","bucketWidth","buckets":[[price,long%,short%],…]}`), or 503 when there is none. The
levels panel draws its order clusters from it.

**Websocket `ws://host:8083/l2`**: optional, Level 2 for BTC/USD in Kraken's websocket v2
messages, so the same code reads either: a `{"channel":"book","type":"snapshot","data":[{"bids":
[{"price","qty"}…],"asks":[…]}]}`, then `book` updates (qty 0 removes a level) and `trade`
updates (`{"side":"buy"|"sell","price","qty"}`). Send a trade before the book update that
removes the level it took, so the floor knows the level traded rather than being pulled.

## 5. Pointing the page somewhere else

The page looks for each service on the host that served it, on its usual port. Change any of
them in the address bar:

| Parameter | Default | Examples |
|---|---|---|
| `?tape=` | `:8083` | `18083` (same host), `prices.local:8083`, `https://prices.example` |
| `?relay=` | `:8099` | same forms |
| `?l2=` | `tape` (the tape's `/l2`) | `kraken` (Kraken's real public book), `off`, any `wss://` URL speaking Kraken v2 |
| `?bridge=` | off | a server with a cTrader-bridge-style `/getData?pair=&timeframe=&range=` for deeper 5m/15m history |

`?l2=kraken` shows Kraken's real BTC book. It only lines up with the BTC pit when the tape's
BTCUSD is real too (the Kraken adapter); against synthetic BTC the walls are thousands of
dollars from the pit's price.

Natively (the paper desk and the tools), the same through `OPENCRY_TAPE_URL`,
`OPENCRY_FEED_URL`, `OPENCRY_JEV_URL`, `OPENCRY_L2_URL` and `OPENCRY_BRIDGE_URL`.

## 6. History for the backtests

`genome_backtest`, the evolving pit (`evolve/`) and the desk's warm start read 15m bars from
`data/cache/ctrader_<SYMBOL>_SB_15m.json` (the name is the format's; it began as a broker
cache): a JSON array of `[t_ms, open, high, low, close, ticks]`, oldest first. The launcher
writes synthetic history there on first run (`floor_tape --write-cache data/cache`, 2021 to now,
about half a minute). To backtest on your own history, write the same files from your data and
point `OPENCRY_EDGE_CACHE` at their directory. The evolve loop's holdout starts 2025-01-01
(`HOLDOUT_FROM` in `evolve/evolve.py`), so give it years on both sides.

## 7. Jev

Jev (TypeSafe's System One model) is optional and paid. Without a key, the relay still runs
the paper logs and the desk's board, `/jev` answers 503 at once, and the floor and the desk use
the rule-of-thumb stand-in (`DeterministicModel` in `src/jev.rs`), which is free and shown as
FLOOR RULE wherever a read's source is shown. With a key (`JEV_KEY=` in `jevfloor_learning_day/.env`), the relay holds it
and the page never sees it.
