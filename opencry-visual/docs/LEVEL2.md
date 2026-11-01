# Level 2: the order book on the floor

The floor shows Bitcoin's order book, read in Kraken's public websocket v2 messages from one
of two places (`?l2=` in the address bar, `src/endpoints.rs`):

- **the tape's `/l2`** (the default): `floor_tape` keeps a synthetic 500-level book around its
  own BTCUSD price, with walls that get posted, pulled and traded through, so it works offline
  and the walls sit where the pit's price is;
- **Kraken itself** (`?l2=kraken`, `wss://ws.kraken.com/v2`): the real book, no account and no
  key. It lines up with the pit only when the tape's BTCUSD is real too (`adapters/kraken_btc.py`).

Any socket speaking the same messages works (`?l2=wss://…`). It is the starting point for
drawing the book in the pits: two views are in place, and the data is ready for more.

## What is there

- **`src/depth.rs`** holds the code:
  - the connection (channels `book` at depth 500 and `trade`, with reconnects);
  - the book (`OrderBook`: price-keyed bids and asks);
  - the trades (`Print`);
  - one-second snapshots for history (`HeatColumn`);
  - big-order events (`BookEvent`: an order of 5 BTC or more added, or pulled before it traded,
    with re-posts of the same quote folded into one line).
- **The Level 2 panel** (bottom-row button *Level 2*) is a heatmap in the style of Bookmap:
  - price runs up the side in $10 rows and time along the bottom, five minutes in all;
  - colour shows the size resting at each price;
  - the mid price is a white line, and trades are dots: green bought, red sold;
  - a list of big orders added or pulled sits underneath.
- **The liquidity walls** stand on the BTC pit's back rim, either side of the pulpit:
  - bids are green and on the left, asks red and on the right;
  - each column is $10 of price, nearest the pulpit nearest the market;
  - a column's height is the size resting there;
  - a column flashes white when a big order joins and amber when one is pulled.

## Where to take it

Ideas that the data already supports:

- **Resting orders as people.** Traders queue on the pit steps at their price levels, and
  bigger orders mean more of them.
- **Pulled orders, shown as people.** The figure walks away, so spoofing is something you can
  watch. `BookEvent::Pulled` is the hook.
- **Absorption.** Aggressive trades hit a level that does not shrink: a scrum. Compare
  `prints` with the book at that price.
- **Imbalance.** The crowd leans toward the heavier side; `OrderBook::buckets` gives both sides.
- **A depth ladder** as a physical board beside the pulpit.
- **Other markets.** Any feed that produces the same `OrderBook` updates and `Print`s can drive
  the same views. Kraken has ETH/USD and others on the same channels, and other exchanges
  publish similar public books.

The pure parts (the book, the bucketing, the event rules, the colour ramp, the heatmap drawing)
are unit-tested in `depth.rs` with recorded Kraken messages, so changes can be checked without
a connection.
