#!/usr/bin/env bash
# Starts the floor with the launcher, checks the tape and the relay answer as documented, then
# stops them and checks the ports are free. CI runs it on a clean Linux box; it is safe locally
# too, on its own ports (28xxx by default, so it leaves a running floor alone).
#
#   ci/smoke.sh                     # FLOOR_*_PORT move it, as for the launcher
#
# Needs curl, lsof and cargo. The paper desk, evolve and trunk are not part of it: the desk and
# genome_backtest are covered by the opencry-visual tests, and trunk is left to the wasm job.

set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

export FLOOR_TAPE_PORT="${FLOOR_TAPE_PORT:-28083}" FLOOR_RELAY_PORT="${FLOOR_RELAY_PORT:-28099}"
export FLOOR_TRUNK_PORT="${FLOOR_TRUNK_PORT:-28180}" FLOOR_NO_TRUNK=1 FLOOR_JEV=none
TAPE="http://127.0.0.1:$FLOOR_TAPE_PORT" RELAY="http://127.0.0.1:$FLOOR_RELAY_PORT"

fails=0
ok()   { printf '  ok    %s\n' "$*"; }
bad()  { printf '  FAIL  %s\n' "$*"; fails=$((fails + 1)); }
check() { local what="$1"; shift; if "$@"; then ok "$what"; else bad "$what"; fi; }
wait_for() { # url, seconds: until it answers 200
  local i; for ((i = 0; i < $2; i++)); do
    [[ "$(curl -s -o /dev/null -w '%{http_code}' --max-time 2 "$1")" == 200 ]] && return 0
    perl -e 'select(undef,undef,undef,1)'
  done; return 1
}
dump_logs() {
  for f in floor_tape/log/floor_tape.out jev_relay/log/relay.out; do
    [[ -f "$ROOT/$f" ]] && { echo "--- $f (last 30 lines)"; tail -30 "$ROOT/$f"; }
  done
}

echo "== start"
"$ROOT/start-floor.sh" || { bad "start-floor.sh exited $?"; }
check "tape answers /health"  wait_for "$TAPE/health" 120
check "relay answers /health" wait_for "$RELAY/health" 30

echo "== tape"
health="$(curl -s --max-time 5 "$TAPE/health")"
check "prices are flowing (\"connected\":true)" grep -q '"connected":true' <<<"$health"
check "all nine symbols are on the tape" \
  python3 -c 'import json,sys; s=json.load(sys.stdin)["symbols"]; sys.exit(len(s) != 9)' <<<"$health"
bars="$(curl -s --max-time 10 "$TAPE/bars?symbol=XAUUSD&timeframe=1m&range=-1h&delta=1")"
check "/bars gives an hour of gold, with flow fields" python3 -c '
import json, sys
d = json.load(sys.stdin)["data"]
b = d[-1]
sys.exit(not (50 <= len(d) <= 61 and b["low"] <= b["close"] <= b["high"] and "upticks" in b))' <<<"$bars"
check "/book has a five-level book" python3 -c '
import json, sys; d = json.load(sys.stdin); sys.exit(not (len(d["bids"]) == 5 and d["bid"] < d["ask"]))' \
  <<<"$(curl -s --max-time 5 "$TAPE/book?symbol=EURUSD")"
check "the websocket says hello and ticks" node -e '
const ws = new WebSocket(process.argv[1]); let hello = false;
const t = setTimeout(() => process.exit(1), 15000);
ws.onmessage = (e) => { const m = JSON.parse(e.data);
  if (m.type === "hello") hello = m.symbols.length === 9;
  if (m.type === "tick" && hello) { clearTimeout(t); process.exit(0); } };
ws.onerror = () => process.exit(1);' "ws://127.0.0.1:$FLOOR_TAPE_PORT/"
check "the backtest cache was written" test -s "$ROOT/data/cache/ctrader_XAUUSD_SB_15m.json"

echo "== relay"
# Only without a key: with one, this would be a real (paid) Jev call.
if curl -s --max-time 5 "$RELAY/health" | grep -q '"key_loaded":false'; then
  check "with no key, /jev answers 503 at once" test "$(curl -s -o /dev/null -w '%{http_code}' \
    --max-time 5 -X POST -d '{"state":"gold, quiet","questions":{}}' "$RELAY/jev")" = 503
else
  ok "/jev skipped: this relay has a key"
fi
check "the desk board answers" \
  test "$(curl -s -o /dev/null -w '%{http_code}' --max-time 5 "$RELAY/paper/board")" = 200

echo "== stop"
# tape and relay only: in a working copy, the desk and evolve may be someone's running floor.
"$ROOT/start-floor.sh" --stop relay >/dev/null
"$ROOT/start-floor.sh" --stop tape >/dev/null
perl -e 'select(undef,undef,undef,1)'
for p in "$FLOOR_TAPE_PORT" "$FLOOR_RELAY_PORT"; do
  check "port $p is free" bash -c "! lsof -nP -iTCP:$p -sTCP:LISTEN -t >/dev/null 2>&1"
done

if ((fails)); then dump_logs; echo "smoke: $fails failed"; exit 1; fi
echo "smoke: all passed"
