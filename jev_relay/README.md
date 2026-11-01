# jev_relay

The browser build of `opencry-visual` cannot hold the TypeSafe (Jev) API key: anything in the
wasm or the page is readable by whoever loads it. This relay runs on the dev machine, holds the
key, and forwards System One calls. It is deliberately separate from the trading bridge on :8000
so the author's live trading never shares a process, a rate limit or a failure with the floor.

    cd jev_relay && cargo run --release        # listens on 0.0.0.0:8099 when run by hand

Under `start-floor.sh` the binary listens on `127.0.0.1:8100` (`JEV_RELAY_BIND`) and
`relay_door.mjs`, a node forwarder, owns `0.0.0.0:8099` in front of it. The macOS
application firewall blocks incoming connections to a freshly built cargo binary started
headless (nobody sees its dialog, and every rebuild resets the answer); node is already
allowed, and the firewall judges the binary, not the script. Clients keep using `:8099`.

Key: `JEV_KEY` or `TYPESAFE_API_KEY` in the environment, else `jev_relay/.env`, else
`jevfloor_learning_day/.env`. The key is never logged and never returned.

Routes (CORS `*`):

| Route                    | What                                                                 |
|--------------------------|----------------------------------------------------------------------|
| `GET /health`            | `{ok, key_loaded, desk_alive, calls, ok_calls, failed, in_flight, last_error}` |
| `POST /jev`              | body `{state, questions, model?}` → forwarded to System One verbatim |
| `POST /log/<name>`       | append one JSON object to `log/<name>.jsonl` (paper trades, decisions) |
| `POST /paper/heartbeat`  | the paper desk's board (`{traders: [...], ...}`), kept in memory      |
| `GET /paper/board`       | the last heartbeat plus `desk_alive` (fresh within 30 s), `heartbeat_age_ms`, `received_ms`; with no heartbeat `{desk_alive:false, traders:[]}` |
| `POST /paper/order`      | a manual order from the page's trade ticket, `{action: buy\|sell\|close, symbol, stop_atr?, target_r?}`; queued in memory (last 200) with a `seq` |
| `GET /paper/orders`      | `?after=<seq>`: the queued orders since then, `{orders, last_seq}`; the paper desk polls it every second. Only paper orders exist: the relay never talks to a broker |
| `GET /paper/trades`      | `?since_ms=<unix ms>&limit=<n>&source=desk&trader=<name>`: closed trades from `log/paper_trades.jsonl` with `closed_ms >= since_ms`, oldest first, the last `limit` (default 4000, max 20000); the desk board page (`opencry-visual/static/desk.html`) polls it |

The paper desk (`opencry-visual/src/bin/paper_desk.rs`) posts a heartbeat every 10 s; the app
polls the board every 5 s and shows it instead of its own book while the desk is alive. The
relay computes nothing for the board, it only serves the last heartbeat with its age.

Guard rails: one upstream call at a time, a minimum gap between calls (`JEV_RELAY_MIN_GAP_MS`,
default 2000), 30 s upstream timeout, 256 KiB body cap. Every call is appended to
`log/inferences.jsonl` as `{ts, observation_hash, model, latency_ms, status, request, response}`
so a run can be replayed from recorded inferences.

Env: `JEV_RELAY_BIND`, `JEV_RELAY_MIN_GAP_MS`, `JEV_RELAY_LOG`, `JEV_MODEL` (default `jev-latest`).
