# Adapters

Small programs that read prices from somewhere and POST them to `floor_tape`
(`floor_tape --ingest SYMBOL`). The shapes are in
[`docs/DATA_SOURCES.md`](../docs/DATA_SOURCES.md#3-a-live-feed-post-ingest).

| Adapter | Source | Needs |
|---|---|---|
| `kraken_btc.py` | Kraken's public REST API: BTC/USD backfill (720 one-minute bars) and a quote a second | Python 3, internet; no account or key |
| `template.py` | Yours: the same program with `history()` and `quote()` left to fill in | Python 3 |

```sh
floor_tape --ingest BTCUSD &
python3 adapters/kraken_btc.py
# then open the floor with ?l2=kraken: the real BTC book now lines up with the real BTC price
```

To write one for another source, copy `template.py` and fill in `history()` (one-minute bars,
`[t_ms, open, high, low, close, volume]`) and `quote()` (`(bid, ask)`). Use the floor's symbol
names; any language that can make an HTTP POST will do. The step-by-step, including how a
broker's streaming API (cTrader) fits, is in
[Adding your own feed](../docs/DATA_SOURCES.md#adding-your-own-feed).
