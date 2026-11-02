# The floor's services

The OpenCry floor runs on a handful of processes of its own. None of them places an order and
none needs an account: the default market is synthetic, Jev is optional, and everything runs
offline. Where the prices come from, and how to give it yours, is in
[`DATA_SOURCES.md`](DATA_SOURCES.md).

`start-floor.sh` at the repo root is the launcher. It is a convenience, not a requirement (see
[By hand](#by-hand)). No tmux: each service is detached under `nohup` with its own log, found by
the port it listens on, and stopped by the pid that holds that port.

```sh
./start-floor.sh                # build what is missing, start what is down, in order
./start-floor.sh --status       # port, pid, uptime, health, log for each
./start-floor.sh --restart tape # also: relay, desk, evolve     (trunk is refused on purpose)
./start-floor.sh --stop floor   # also: tape, relay, desk, evolve (floor = all but trunk)
./start-floor.sh --dry-run      # say what a plain run would do
```

| Service | Port | What | Log |
|---|---|---|---|
| **tape** (`floor_tape`) | 8083 | Prices for the nine pits: synthetic by default, or your CSVs (`--csv`), or whatever you POST (`--ingest`). A websocket of ticks and five-level books, `/bars`, `/book`, `/orderbook`, `/health`, and BTC Level 2 on `/l2`. Arguments from `FLOOR_TAPE_ARGS`. | `floor_tape/log/floor_tape.out` |
| **relay** (`jev_relay`) | 8099 | Forwards `POST /jev` to Jev with a 2 s minimum gap when it has a key, and answers 503 at once when it has none (the floor then uses its rule of thumb). Either way: appends paper trades and decisions (`POST /log/<name>`), serves the desk's board (`/paper/board`) and trade queries (`/paper/trades`). | `jev_relay/log/relay.out` |
| **desk** (`paper_desk`) | none | The paper traders, headless, around the clock, on the tape: the built-in traders plus the evolving pit's genomes (`--pit evolve/pit.json`, re-read every minute). `--jev live` with a key, `--jev rule` without (`FLOOR_JEV` overrides). Skipped with a message when not built. | `opencry-visual/log/paper_desk.out` |
| **evolve** (`evolve.py loop`) | none | Every hour: judges seated genomes on their forward desk trades, cuts failures, backtests new genomes with `genome_backtest` on `data/cache`, and seats any that pass. Rules are pre-registered in `evolve/README.md`. Skipped when `genome_backtest` is not built. | `evolve/evolve.out` |
| **trunk** (`trunk serve`) | 8180 | Builds the wasm app and serves it. Started only when nothing holds the port, and never restarted or stopped by the launcher (a browser is usually attached). | `opencry-visual/log/trunk.out` |

On first run the launcher also builds `floor_tape` and `jev_relay` if they are missing, and
writes the synthetic 15m history (`data/cache`, about half a minute) unless `EDGE_LAB_DIR` or
`OPENCRY_EDGE_CACHE` points elsewhere. The paper desk and `genome_backtest` are part of the
`opencry-visual` crate and take longer to build the first time, so the launcher leaves that to
you:

```sh
cd opencry-visual && cargo build --release --bin paper_desk --bin genome_backtest
```

Everything listens on this machine only (`127.0.0.1`) unless you ask for more (next section).

## From another device

```sh
FLOOR_LAN=1 ./start-floor.sh
```

opens the floor to other devices: a phone, another computer, or the same over a VPN such as
Tailscale. The launcher prints the address to open. On Linux and elsewhere the services simply
listen on every address. On macOS they cannot: the application firewall silently blocks
incoming connections to a freshly built, ad-hoc signed binary started headless (nobody sees its
dialog, and every rebuild resets the answer), while node is allowed. So with `FLOOR_LAN=1` on a
Mac, three small node forwarders ("doors", raw TCP, so websockets pass through) take the outside
ports and the services move behind them on loopback:

| Door | Outside port | Forwards to | Log |
|---|---|---|---|
| tape door (`floor_tape/tape_door.mjs`) | 8083 | the tape on 127.0.0.1:8183 | `floor_tape/log/tape_door.out` |
| relay door (`jev_relay/relay_door.mjs`) | 8099 | the relay on 127.0.0.1:8100 | `jev_relay/log/relay_door.out` |
| page door (the tape door's script again) | **8188** | trunk on 8180 | `opencry-visual/log/page_door.out` |

From another device on a Mac, open port **8188**. Without node the launcher falls back to binding
the services on every address, and the firewall may then block them. Like trunk, the page door
is never stopped by the launcher. Run `--stop` and `--restart` with or without `FLOOR_LAN`: they
clear the doors and the services behind them either way.

## Ports

Every port moves with an environment variable, and the page follows with a query parameter:

```sh
FLOOR_TAPE_PORT=18083 FLOOR_RELAY_PORT=18099 FLOOR_TRUNK_PORT=18180 ./start-floor.sh
# then open http://localhost:18180/?tape=18083&relay=18099 (the launcher prints it)
```

Behind the macOS doors, `FLOOR_TAPE_BIND_PORT` (8183), `FLOOR_RELAY_BIND_PORT` (8100) and
`FLOOR_PAGE_DOOR_PORT` (trunk's port + 8) move too. `FLOOR_PROTECTED_PORTS="8000 8086"` lists ports the launcher must never stop, whatever the
other settings say: put your other services' ports there.

## Rules that are not obvious from the code

**The Jev key** lives in `jevfloor_learning_day/.env` as `JEV_KEY=` (the relay also accepts
`TYPESAFE_API_KEY`, the environment, or `jev_relay/.env`). Only the relay reads it. It is never
printed, never put in code, and never reaches the wasm bundle: the page calls the relay, the
relay calls Jev. Jev is paid per call (about 1,900 input tokens a read); `--jev rule` is free.

**A pit halts when its prices stop.** The floor never fills a gap with invented prices for a
symbol on a real source: no tick for 90 s halts the pit as quiet, no message at all for 20 s
marks the feed stale and the screen says so. (The engine's own simulation, key `L`, wears a
permanent banner.)

**trunk on 8180 is left alone.** Killing it loses the browser session and a rebuild takes
minutes. If you must, do it by hand.

## By hand

```sh
# tape
cd floor_tape && cargo build --release
nohup ./target/release/floor_tape --bind 127.0.0.1 >> log/floor_tape.out 2>&1 &
curl -s localhost:8083/health | head -c 200

# relay
cd jev_relay && cargo build --release
JEV_RELAY_BIND=127.0.0.1:8099 nohup ./target/release/jev_relay >> log/relay.out 2>&1 &
curl -s localhost:8099/health

# desk
cd opencry-visual && cargo build --release --bin paper_desk
nohup ./target/release/paper_desk --jev rule --jev-every 60 --pit ../evolve/pit.json >> log/paper_desk.out 2>&1 &

# the floor
cd opencry-visual && trunk serve --port 8180
```

## Checking on it

```sh
./start-floor.sh --status
curl -s localhost:8083/health | python3 -c 'import json,sys; d=json.load(sys.stdin); print([(s["symbol"], s["source"], s["lastTickAgeMs"]) for s in d["symbols"]])'
curl -s localhost:8099/health                  # calls, failures, key_loaded
tail -5 jev_relay/log/paper_trades.jsonl       # what the paper book did
tail -3 opencry-visual/log/paper_desk.out
python3 evolve/evolve.py status                # the evolving pit
```

Expect `lastTickAgeMs` in the seconds for every synthetic symbol (the synthetic market never
closes) and for real ones during their trading hours.
