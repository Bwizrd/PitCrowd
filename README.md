# PitCrowd

**A live trading floor in Rust: the crowd in each pit reacts to the price.**

A 3D open-outcry trading floor in Rust and Bevy (native and wasm). Simulated pit crowds trade
around a market price, paper traders (hand-written, evolved "genomes" and LLM-classifier-assisted
ones) stand on the floor and walk to the pits they trade, and a level-2 feed draws Bitcoin's
order book as a heatmap and as liquidity walls in the pit.

It runs offline out of the box: a synthetic market, no account, no API key. Real prices plug in
through one small protocol (CSV recordings, a live feed POSTed from any language, or your own
server); see [`docs/DATA_SOURCES.md`](docs/DATA_SOURCES.md).

(Inside the code the project is still called OpenCry: the crates are `opencry-visual` and so on.)

Created by Paul Leppard to learn Rust and Jev, and open-sourced to see how far others can take
it. If you build on it, a link back is appreciated.

> **Status: work in progress, published for collaboration.** This is a research and
> visualisation project, not trading software. It places no real orders (paper trading only).
> Nothing here is financial advice, and nothing in it has been shown to make money. It is
> maintained in spare time: issues and pull requests are welcome, and replies may take a few
> days.

## Quick start

```sh
# Rust (https://rustup.rs), then trunk and the wasm target for the browser build
cargo install trunk && rustup target add wasm32-unknown-unknown
./start-floor.sh          # builds the tape and relay, writes synthetic history, starts it all
# first time only, for the headless paper desk and the evolving pit (a longer build):
(cd opencry-visual && cargo build --release --bin paper_desk --bin genome_backtest) && ./start-floor.sh
```

Open <http://localhost:8180> once trunk has built the page (`opencry-visual/log/trunk.out`; the
first build takes several minutes). `./start-floor.sh --status` shows each service, and
[`docs/SERVICES.md`](docs/SERVICES.md) explains them.

`start-floor.sh` is the easy way in, not a requirement: it builds what is missing and starts the
five services in order with matching ports. `docs/SERVICES.md` shows how to run each by hand.

If Rust also came from Homebrew (`brew install rust`), that cargo cannot build for the browser;
the launcher then uses rustup's toolchain for the page, and says so. To watch real Bitcoin instead of the synthetic one:

```sh
FLOOR_TAPE_ARGS="--ingest BTCUSD" ./start-floor.sh --restart tape
python3 adapters/kraken_btc.py          # Kraken's public API, no key
# and open http://localhost:8180/?l2=kraken for Kraken's real order book in the BTC pit
```

**About the synthetic market:** it has no edge, by construction (minute returns are
uncorrelated). Whatever the paper traders or the evolving pit "earn" on it is luck, which makes
it a measuring stick for false positives, not a place to find a strategy.

**Trading as yourself:** the trade ticket books paper trades as "Guest" until you press
**Change name** (next to "Your name" on the ticket) and type yours. Your browser remembers it and
the paper desk keeps it across restarts. You can also open the floor with
`?trader=Your%20Name`, or start the desk with `OPENCRY_TRADER_NAME="Your Name"`.

## Two open invitations

1. **Find better ways to use Jev.** Jev is TypeSafe's "System One" classifier. The repo has
   an evaluation harness (`evolve/`, `evolve/audit/`, `opencry-visual/src/bin/jev_score.rs`)
   and a record of what was tried. Nothing tried so far has shown Jev adding an edge over a
   simpler rule, but these are a newcomer's first experiments, and someone who knows Jev
   better may well do much more with it.
   [`research/web-search-2026-10-25.md`](research/web-search-2026-10-25.md) is what a web
   search found about others' attempts. Jev is optional: without a key the floor uses a free
   rule of thumb in its place.
2. **Make the pits more real with level-2 data.** `opencry-visual/docs/LEVEL2.md` describes
   the Bitcoin order book feed (the tape's synthetic book, or Kraken's public websocket with
   `?l2=kraken`) and lists ideas the data already supports.

More directions, and small pieces to start with, are in [`ROADMAP.md`](ROADMAP.md).

## Contributing

Contributions of any size are welcome. [`CONTRIBUTING.md`](CONTRIBUTING.md) covers running it,
the checks to run before a pull request, and the few house rules (it runs offline with no key,
no secrets, no paid calls in tests, no real orders). Security problems: see
[`SECURITY.md`](SECURITY.md).

## Layout

| Path | What |
|---|---|
| `opencry-visual/` | The floor (Bevy app, `trunk serve` for the browser) and the headless tools: `paper_desk`, `genome_backtest`, `replay`, `jev_score` |
| `floor_tape/` | The market tape: synthetic prices, CSV replay or your own feed, on the protocol the floor reads |
| `adapters/` | Example live feeds for the tape (`kraken_btc.py`: real Bitcoin, no key) |
| `jev_relay/` | A small local HTTP relay that holds the Jev API key so it never reaches the browser, and logs the paper trades |
| `evolve/` | The evolving pit: spawns, backtests, seats and cuts genomes under pre-registered rules |
| `jevfloor_learning_day/` | A Rust-plus-Jev learning course (lessons and examples) |
| `docs/` | Data sources, services, and the design notes the floor grew from |
| `research/` | What a web search found about trading with Jev (2026-10-25) |
| `ci/`, `.github/` | The checks CI runs (see [Checks](#checks)), and the issue and pull request templates |

## Configuration

Nothing is required. Optional settings:

- `JEV_KEY=` in `jevfloor_learning_day/.env` (git-ignored) to use Jev itself; only the relay
  reads it. Start from the example: `cp jevfloor_learning_day/.env.example jevfloor_learning_day/.env`.
- `FLOOR_TAPE_ARGS` for the tape's source (`--csv DIR`, `--ingest SYMS`, `--seed N`), and
  `FLOOR_*_PORT` to move any service (`docs/SERVICES.md`).
- `OPENCRY_EDGE_CACHE` to backtest on your own 15m history instead of `data/cache`.

- `INFLUX_URL` / `INFLUX_TOKEN` / `INFLUX_ORG` in a repo-root `.env` (or `OPENCRY_ENV_FILE`)
  only for the `replay` and `jev_score` research tools, which read recorded ticks from InfluxDB.

## Checks

CI (`.github/workflows/ci.yml`) runs on every push and pull request. It needs no account or
key and makes no paid call. To see a change pass before you push, run the same checks:

```sh
# in each crate (floor_tape, jev_relay, opencry-visual, jevfloor_learning_day)
cargo fmt --check
cargo clippy --all-targets -- -D warnings   # the learning-day lessons are exempt from -D
cargo test --all-targets
# and opencry-visual for the browser
cargo clippy --target wasm32-unknown-unknown --lib --bin opencry-visual -- -D warnings

node ci/check_pages.mjs   # syntax of the JavaScript inside the HTML pages
ci/smoke.sh               # the launcher starts the tape and the relay, and they answer
```

`ci/smoke.sh` runs on ports of its own (28xxx), so it is safe beside a running floor, and it
stops only the tape and relay on those ports. On Linux, `opencry-visual` needs Bevy's system
libraries (`libasound2-dev libudev-dev libwayland-dev libxkbcommon-dev`). The workflow also
runs shellcheck and gitleaks.

## Licence

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion
in the work by you, as defined in the Apache-2.0 license, shall be dual licensed as above,
without any additional terms or conditions.

`.agents/skills/typesafe-ai/` is TypeSafe AI's own skill, under its own MIT licence (included
there).
