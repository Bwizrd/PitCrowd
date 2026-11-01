# opencry-visual

The OpenCry trading floor: a Bevy app compiled to wasm with trunk (nine pits over the market
tape on :8083), plus two native binaries that run the same library code without a browser.

| Binary | What |
|---|---|
| `opencry-visual` | The floor itself (`trunk serve` on :8180, `cargo run` natively) |
| `replay` | Headless replay of saved ticks from InfluxDB, scored in R (`src/bin/replay.rs`) |
| `paper_desk` | The paper book on the live tape, around the clock (`src/bin/paper_desk.rs`) |
| `jev_score` | Are Jev's reads better than chance? Joins the relay log to the tick history (`src/bin/jev_score.rs`) |
| `reaction_prior` | A year of 1m bars: every touch of a floor level and what price did next, written to `assets/reaction_prior.json` and compiled in as `reaction::PriorTable` (`src/bin/reaction_prior.rs`) |
| `explain` | Regenerates the in-app guide, `static/explainer.html` |

## The desk board page

`http://<host>:8180/desk.html` (`static/desk.html`, copied into the bundle by trunk) is the live
board of the desk: every trader's closes and progress over today / 24 h / 7 d / all, equity
curves, open positions marked to market, breakdowns by pit and group, what each question trader
traded, and the latest closes with their reasons. Plain HTML and JavaScript, no build step: it
polls the relay's `GET /paper/board` every 2 s and `GET /paper/trades?since_ms=` every 10 s
(both through the door on :8099, so it works from any machine on your network).

## The paper desk

The paper traders (`src/paper.rs`, plus the evolving pit's genomes from `src/genome.rs` on the desk) in the browser only run while a tab is open. The desk
runs the same book natively on the live tape: it connects to the tape websocket, backfills 1m
bars from the tape's `/bars`, steps `PaperBook::step_pit` on every tick exactly as the app does,
asks Jev about every pit on a fixed cadence through the relay, and posts every trade and
decision to the relay's `/log/paper_trades` and `/log/paper_decisions` (same rows as the app,
plus `source: "desk"`). Every 3 s it posts a heartbeat with a rolling 24 h leaderboard to
`POST /paper/heartbeat`; the app polls `GET /paper/board` and, while the desk is alive, the
Traders panel (T) shows "DESK (24h, all pits)" and the in-tab book stops opening positions so
the two never double count. Stop the desk and the app goes back to its own book.

Build and run (release, all nine pits, live Jev once a minute per pit):

```sh
cd opencry-visual
export CARGO_TARGET_DIR=$PWD/../.agent_target   # optional; leave unset for ./target
cargo build --release --bin paper_desk
mkdir -p log
nohup $CARGO_TARGET_DIR/release/paper_desk --jev live --jev-every 60 > log/paper_desk.out 2>&1 &
```

Check it is working:

```sh
tail -f opencry-visual/log/paper_desk.out          # socket open, backfills, Jev reads, trades, a status line a minute
curl -s localhost:8099/paper/board | python3 -m json.tool   # desk_alive true, heartbeat_age_ms small
tail -3 jev_relay/log/paper_decisions.jsonl         # rows with "source":"desk"
```

Stop it: `pkill -f 'release/paper_desk'` (or `kill <pid>` from `pgrep -fl paper_desk`).
Open positions die with the process; closed trades are already on disk.

Flags (all optional):

| Flag | Default | Meaning |
|---|---|---|
| `--jev live\|rule\|none` | `live` | Jev through the relay; the free floor rule; or the three Jev traders stay flat |
| `--jev-every N` | 60 | seconds between reads of each pit (min 10); pits are staggered across the cadence |
| `--symbols all\|XAGUSD,EURUSD` | `all` | which pits to run |
| `--seed N` | 42 | the control's coin |
| `--log-dir DIR` | `opencry-visual/log` | local JSONL copies (`paper_desk_trades.jsonl`, `paper_desk_decisions.jsonl`); env `OPENCRY_DESK_LOG_DIR` |
| `--heartbeat-every N` | 10 | seconds between heartbeats |
| `--backfill -6h` | `-6h` | tape `/bars` range loaded at start and after an outage over 2 min |
| `--no-relay-log` | off | keep trades and decisions local only |
| `--touch-reads on\|off` | `on` | ask Jev the moment price touches a level (`reaction::TouchTracker` on the ticks); the read carries a `TRIGGER:` line and counts as that pit's timed read |
| `--touch-follow N` | 30 | seconds after a touch to ask again how the reaction is developing (0 = never) |
| `--touch-cooldown N` | 60 | at most one touch read per pit per N seconds (min 5); with the 60 s timer this roughly doubles the call rate on a busy floor |
| `--print-questions` | | print the question set every read carries (the floor's five plus the crew's own, `jev::CREW_ASKS`) as JSON and exit |

Env: `OPENCRY_TAPE_URL` (`http://host:8083`; the websocket address is derived), `OPENCRY_FEED_URL`
(a `ws://` address, overrides), `OPENCRY_JEV_URL` (the relay, default `http://localhost:8099`).

Behaviour worth knowing:

- Session rules are the book's own (entries 07:00–20:30 UTC, flat by 20:55), so overnight the
  desk mostly reads Jev and heartbeats; trades start with London.
- The tape dropping is survived: reconnect with backoff (1 s doubling to 30 s), and after an
  outage longer than two minutes the 1m bars are reloaded from the tape to close the hole.
- The relay allows one Jev call per 2 s and answers 429 with `retry_after_ms`; the desk's Jev
  worker paces itself to that and retries a 429 up to four times. Nine pits at 60 s is a call
  every ~6.7 s, well inside it, even with the app reading its focused pit at the same time.
- A relay that predates `/paper/heartbeat` answers 404: the desk says so once and carries on;
  trades still log through `/log/*`, and the board appears once the relay is redeployed.
- The leaderboard in the heartbeat is a rolling 24 h window seeded from the local trade log at
  start, so restarting the desk does not blank the board. R is profit over the initial stop;
  `spread_paid_r` is the spread paid at entry over the stop, summed.
