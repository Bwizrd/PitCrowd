# floor_tape

The market tape the floor reads: a small Rust server on port 8083 that needs no account, no key
and no internet. Prices come from, per symbol:

- **synthetic** (the default): a seeded random market, the same on every machine, with no edge
  in it by construction (`src/sim.rs`);
- **your CSVs**: `--csv DIR`, one `<SYMBOL>.csv` each, ticks or bars, replayed on today's clock;
- **your live feed**: `--ingest SYMS`, then POST ticks, books and history to it from any
  language (`../adapters/kraken_btc.py` is a worked example).

```sh
cargo build --release
./target/release/floor_tape                                # all nine symbols synthetic
./target/release/floor_tape --csv ../data/recorded --speed 10
./target/release/floor_tape --ingest BTCUSD                # BTC from an adapter
./target/release/floor_tape --write-cache ../data/cache    # 15m history for the backtests
./target/release/floor_tape --write-csv ../data/sample --days 2
```

Options: `--port 8083` (or `FLOOR_TAPE_PORT`), `--bind 127.0.0.1`, `--seed 42`, `--history 30d`
(how much one-minute history `/bars` keeps), `--warmup 1d`, `--speed 1`, `--no-loop`,
`--only SYM,SYM` (synthetic for just these), `--from 2021-01-01` (with `--write-cache`).

It serves the tape protocol: a websocket of ticks and five-level books, `/bars`, `/book`,
`/orderbook`, `/health`, and Level 2 for BTC in Kraken's websocket v2 messages on `/l2`. The
contract, the CSV format and the ingest shapes are in
[`docs/DATA_SOURCES.md`](../docs/DATA_SOURCES.md).

Dependencies: `tungstenite` (websocket handshake and frames) and `serde_json`. The HTTP side is
a few dozen lines of std, one thread per connection, which is plenty for a handful of browser
tabs and a desk.
