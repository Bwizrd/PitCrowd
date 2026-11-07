#!/usr/bin/env bash
#
# start-floor.sh — start the OpenCry floor and its services on this machine, offline.
#
# Nothing here places an order and nothing needs an account: the default market is synthetic
# (floor_tape), Jev is optional (without a key the floor uses its rule-of-thumb stand-in), and
# the backtests run on history the tape writes. docs/DATA_SOURCES.md says how to feed it your
# own prices instead.
#
# Services, in dependency order:
#   tape   :8083  floor_tape      prices: synthetic, your CSVs, or anything you POST to it
#   relay  :8099  jev_relay       forwards Jev calls if you have a key; logs paper trades and
#                                 serves the paper desk's board either way
#   desk   (none) paper_desk      the paper traders, headless, around the clock (skipped if not built)
#   evolve (none) evolve.py loop  spawns, backtests, seats and cuts genomes hourly (evolve/README.md)
#   trunk  :8180  trunk serve     the floor in the browser — never restarted by this script
#
# Everything listens on this machine only (127.0.0.1): open http://localhost:8180.
#
# Each service runs under nohup with its own log, is found by the port it listens on (lsof), and
# is stopped by the pid holding that port, never by a `pkill -f` pattern.
#
# Usage:
#   start-floor.sh                  # start whatever is missing, in order
#   start-floor.sh --status         # table: port, pid, uptime, health
#   start-floor.sh --restart NAME   # tape | relay | desk | evolve
#   start-floor.sh --stop NAME      # tape | relay | desk | evolve | floor (= all but trunk)
#   start-floor.sh --dry-run        # print what a plain run would do
#
# Env:
#   FLOOR_TAPE_ARGS        extra floor_tape arguments, e.g. "--csv data/recorded --speed 10"
#                          or "--ingest BTCUSD" (see floor_tape/README.md)
#   FLOOR_NO_TAPE=1        don't start floor_tape: you run your own tape on FLOOR_TAPE_PORT
#   FLOOR_TAPE_PORT        8083
#   FLOOR_RELAY_PORT       8099
#   FLOOR_TRUNK_PORT       8180     (other ports: open http://localhost:PORT/?tape=PORT&relay=PORT)
#   FLOOR_NO_TRUNK=1       never start trunk (e.g. you serve a `trunk build` yourself)
#   FLOOR_JEV=live|rule|none   the desk's Jev mode (default: live with a key, rule without)
#   FLOOR_PROTECTED_PORTS  space-separated ports this script must never stop
#   OPENCRY_TARGET_DIR     a CARGO_TARGET_DIR the Rust binaries were built into
#   OPENCRY_TRADER_NAME    the ticket trader's name (default "Guest")

set -uo pipefail

# Fill in the usual tool folders for a bare environment (cron, launchd), AFTER the user's own
# PATH, so a toolchain the user put first stays first.
export PATH="$PATH:$HOME/.cargo/bin:/opt/homebrew/bin:/usr/local/bin"

FLOOR_ROOT="${FLOOR_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)}"
TAPE_DIR="$FLOOR_ROOT/floor_tape"
RELAY_DIR="$FLOOR_ROOT/jev_relay"
VISUAL_DIR="$FLOOR_ROOT/opencry-visual"
EVOLVE_DIR="$FLOOR_ROOT/evolve"
KEY_ENV="$FLOOR_ROOT/jevfloor_learning_day/.env"   # may hold JEV_KEY; never printed
CACHE_DIR="${OPENCRY_EDGE_CACHE:-$FLOOR_ROOT/data/cache}"

TAPE_PORT="${FLOOR_TAPE_PORT:-8083}"
RELAY_PORT="${FLOOR_RELAY_PORT:-8099}"
TRUNK_PORT="${FLOOR_TRUNK_PORT:-8180}"

bin_path() {  # $1 crate dir, $2 binary: OPENCRY_TARGET_DIR first, then the crate's target/
  if [[ -n "${OPENCRY_TARGET_DIR:-}" && -x "$OPENCRY_TARGET_DIR/release/$2" ]]; then echo "$OPENCRY_TARGET_DIR/release/$2"; else echo "$1/target/release/$2"; fi
}
TAPE_BIN="$(bin_path "$TAPE_DIR" floor_tape)"
RELAY_BIN="$(bin_path "$RELAY_DIR" jev_relay)"
DESK_BIN="$(bin_path "$VISUAL_DIR" paper_desk)"

TAPE_LOG="$TAPE_DIR/log/floor_tape.out"
RELAY_LOG="$RELAY_DIR/log/relay.out"
DESK_LOG="$VISUAL_DIR/log/paper_desk.out"
DESK_PID_FILE="$VISUAL_DIR/log/paper_desk.pid"
EVOLVE_LOG="$EVOLVE_DIR/evolve.out"
EVOLVE_PID_FILE="$EVOLVE_DIR/evolve.pid"
TRUNK_LOG="$VISUAL_DIR/log/trunk.out"

read -r -a PROTECTED_PORTS <<< "${FLOOR_PROTECTED_PORTS:-}"
read -r -a TAPE_ARGS <<< "${FLOOR_TAPE_ARGS:-}"

DRY_RUN=false

log()  { printf '\033[1;35m[start-floor]\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33m[start-floor]\033[0m %s\n' "$*" >&2; }
die()  { warn "$*"; exit 1; }

have_key() { grep -qs '^JEV_KEY=.\|^TYPESAFE_API_KEY=.' "$KEY_ENV" || [[ -n "${JEV_KEY:-}${TYPESAFE_API_KEY:-}" ]]; }

# -----------------------------------------------------------------------------
# Detection
# -----------------------------------------------------------------------------

port_pid() { lsof -nP -iTCP:"$1" -sTCP:LISTEN -t 2>/dev/null | head -1; }

port_is_protected() {
  local p
  for p in "${PROTECTED_PORTS[@]:-}"; do [[ -n "$p" && "$1" == "$p" ]] && return 0; done
  return 1
}

pid_from_file() {  # $1 pid file, $2 pgrep pattern
  local pid
  if [[ -f "$1" ]]; then
    pid=$(cat "$1" 2>/dev/null)
    if [[ -n "$pid" ]] && kill -0 "$pid" 2>/dev/null; then echo "$pid"; return; fi
  fi
  pgrep -f "$2" 2>/dev/null | head -1
}
# The fallback patterns carry this checkout's own paths, so another copy's desk or evolve loop on
# the same machine is never mistaken for this one's (and never stopped by it).
desk_pid()   { pid_from_file "$DESK_PID_FILE" "^$DESK_BIN( |$)"; }
evolve_pid() { local pid; pid=$(pid_from_file "$EVOLVE_PID_FILE" "^__none__"); [[ -n "$pid" ]] && echo "$pid"; }

pid_uptime() { [[ -n "${1:-}" ]] && ps -o etime= -p "$1" 2>/dev/null | tr -d ' ' || echo "-"; }

http_code() { curl -s -o /dev/null --max-time 4 -w '%{http_code}' "$1" 2>/dev/null || echo 000; }

tape_health() {
  local body
  body=$(curl -s --max-time 4 "http://localhost:$TAPE_PORT/health" 2>/dev/null) || { echo "no-answer"; return; }
  if grep -q '"connected":true' <<<"$body"; then echo "ok"; else echo "disconnected"; fi
}

relay_health() {  # "ok" when /health answers; the key is reported, not required
  local body
  body=$(curl -s --max-time 4 "http://localhost:$RELAY_PORT/health" 2>/dev/null) || { echo "no-answer"; return; }
  [[ -n "$body" ]] && echo "ok" || echo "no-answer"
}
relay_key() {
  curl -s --max-time 4 "http://localhost:$RELAY_PORT/health" 2>/dev/null | grep -q '"key_loaded":true' && echo "jev" || echo "no-key (rule stand-in)"
}

trunk_health() { [[ "$(http_code "http://localhost:$TRUNK_PORT/")" == 200 ]] && echo "ok" || echo "no-answer"; }

# -----------------------------------------------------------------------------
# Stop
# -----------------------------------------------------------------------------

wait_port_free() {
  local port="$1" timeout="${2:-20}" elapsed=0
  while [[ -n "$(port_pid "$port")" ]]; do
    sleep 1; elapsed=$((elapsed + 1))
    (( elapsed >= timeout )) && return 1
  done
}

stop_port_service() {  # $1 name, $2 port
  local name="$1" port="$2" pid
  port_is_protected "$port" && die "refusing to touch :$port (FLOOR_PROTECTED_PORTS)"
  pid=$(port_pid "$port")
  if [[ -z "$pid" ]]; then log "$name: nothing listening on :$port"; return 0; fi
  log "$name: stopping (pid $pid on :$port)"
  $DRY_RUN && return 0
  kill "$pid" 2>/dev/null || true
  if ! wait_port_free "$port" 20; then
    warn "$name: :$port still held after 20s — SIGKILL"
    kill -9 "$pid" 2>/dev/null || true
    wait_port_free "$port" 10 || die "$name: could not free :$port"
  fi
}

stop_pid_service() {  # $1 name, $2 pid fn, $3 pid file
  local pid; pid=$($2)
  if [[ -z "$pid" ]]; then log "$1: not running"; return 0; fi
  log "$1: stopping (pid $pid)"
  $DRY_RUN && return 0
  kill "$pid" 2>/dev/null || true
  local i=0
  while kill -0 "$pid" 2>/dev/null && (( i < 15 )); do sleep 1; i=$((i + 1)); done
  kill -0 "$pid" 2>/dev/null && kill -9 "$pid" 2>/dev/null
  rm -f "$3"
}
stop_desk()   { stop_pid_service desk desk_pid "$DESK_PID_FILE"; }
stop_evolve() { stop_pid_service evolve evolve_pid "$EVOLVE_PID_FILE"; }
stop_relay()  { stop_port_service relay "$RELAY_PORT"; }
stop_tape()   { stop_port_service tape "$TAPE_PORT"; }

# -----------------------------------------------------------------------------
# Start
# -----------------------------------------------------------------------------

# Detach a service with every descriptor on its log, so nothing holds this script's stdout.
SPAWNED=""
spawn() {  # $1 log, $2 cwd, $3.. command
  local logf="$1" cwd="$2"; shift 2
  mkdir -p "$(dirname "$logf")"
  ( cd "$cwd" && exec nohup "$@" ) >> "$logf" 2>&1 < /dev/null &
  SPAWNED=$!
  disown "$SPAWNED" 2>/dev/null || true
}

wait_for() {  # $1 label, $2 health fn, $3 want, $4 timeout secs
  local label="$1" fn="$2" want="$3" timeout="${4:-30}" elapsed=0
  while [[ "$($fn)" != "$want" ]]; do
    sleep 1; elapsed=$((elapsed + 1))
    if (( elapsed >= timeout )); then warn "  $label: not healthy after ${timeout}s ($($fn))"; return 1; fi
  done
  log "  ✓ $label healthy (${elapsed}s)"
}

build_if_missing() {  # $1 label, $2 binary, $3 crate dir, $4.. cargo args
  local label="$1" bin="$2" dir="$3"; shift 3
  [[ -x "$bin" ]] && return 0
  command -v cargo >/dev/null || die "$label: $bin missing and cargo is not installed (https://rustup.rs)"
  log "$label: building (cd ${dir#"$FLOOR_ROOT"/} && cargo build --release $*)"
  # Built into the crate's own target/ on purpose: an inherited CARGO_TARGET_DIR would put the
  # binary somewhere this script does not look.
  $DRY_RUN || ( cd "$dir" && env -u CARGO_TARGET_DIR cargo build --release "$@" ) || die "$label: build failed"
}

ensure_cache() {  # the 15m history the desk warms up from and the backtests run on
  [[ -n "${EDGE_LAB_DIR:-}" ]] && return 0
  ls "$CACHE_DIR"/ctrader_*_SB_15m.json >/dev/null 2>&1 && return 0
  log "cache: writing synthetic 15m history to ${CACHE_DIR#"$FLOOR_ROOT"/} (once; about half a minute)"
  $DRY_RUN || "$TAPE_BIN" --write-cache "$CACHE_DIR" || warn "cache: floor_tape --write-cache failed"
}

start_tape() {
  if [[ -n "$(port_pid "$TAPE_PORT")" ]]; then log "tape: already up on :$TAPE_PORT"; return 0; fi
  if [[ "${FLOOR_NO_TAPE:-0}" == 1 ]]; then warn "tape: FLOOR_NO_TAPE=1 and nothing on :$TAPE_PORT — start your tape there"; return 0; fi
  log "tape: starting  (floor_tape --bind 127.0.0.1 --port $TAPE_PORT ${TAPE_ARGS[*]:-} >> ${TAPE_LOG#"$FLOOR_ROOT"/})"
  $DRY_RUN && return 0
  spawn "$TAPE_LOG" "$FLOOR_ROOT" "$TAPE_BIN" --bind 127.0.0.1 --port "$TAPE_PORT" ${TAPE_ARGS[@]+"${TAPE_ARGS[@]}"}
  wait_for "tape" tape_health ok 60 || warn "tape: check $TAPE_LOG"
}

start_relay() {
  if [[ -n "$(port_pid "$RELAY_PORT")" ]]; then log "relay: already up on :$RELAY_PORT ($(relay_health))"; return 0; fi
  have_key || log "relay: no Jev key (JEV_KEY in jevfloor_learning_day/.env) — paper logs and the desk board work, Jev reads use the rule-of-thumb stand-in"
  log "relay: starting  (JEV_RELAY_BIND=127.0.0.1:$RELAY_PORT jev_relay >> ${RELAY_LOG#"$FLOOR_ROOT"/})"
  $DRY_RUN && return 0
  spawn "$RELAY_LOG" "$RELAY_DIR" env JEV_RELAY_BIND="127.0.0.1:$RELAY_PORT" "$RELAY_BIN"
  wait_for "relay" relay_health ok 20 || warn "relay: check $RELAY_LOG"
}

start_desk() {
  if [[ -n "$(desk_pid)" ]]; then log "desk: already running (pid $(desk_pid))"; return 0; fi
  if [[ ! -x "$DESK_BIN" ]]; then
    log "desk: skipped — not built (cd opencry-visual && cargo build --release --bin paper_desk --bin genome_backtest; a first build takes a while)"
    return 0
  fi
  local jev="${FLOOR_JEV:-}"
  if [[ -z "$jev" ]]; then if have_key; then jev=live; else jev=rule; fi; fi
  local args=(--jev "$jev" --jev-every 60 --pit "$EVOLVE_DIR/pit.json")
  log "desk: starting  (paper_desk ${args[*]} >> ${DESK_LOG#"$FLOOR_ROOT"/})"
  $DRY_RUN && return 0
  spawn "$DESK_LOG" "$VISUAL_DIR" env OPENCRY_TAPE_URL="http://localhost:$TAPE_PORT" OPENCRY_JEV_URL="http://localhost:$RELAY_PORT" "$DESK_BIN" "${args[@]}"
  echo "$SPAWNED" > "$DESK_PID_FILE"
  sleep 2
  if [[ -n "$(desk_pid)" ]]; then log "  ✓ desk running (pid $(desk_pid))"; else warn "desk: exited at once — see $DESK_LOG"; fi
}

start_evolve() {
  if [[ -n "$(evolve_pid)" ]]; then log "evolve: already running (pid $(evolve_pid))"; return 0; fi
  if [[ ! -x "$(bin_path "$VISUAL_DIR" genome_backtest)" ]]; then
    log "evolve: skipped — genome_backtest not built (cd opencry-visual && cargo build --release --bin genome_backtest)"
    return 0
  fi
  log "evolve: starting  (python3 -I -u evolve.py loop --every 3600 >> ${EVOLVE_LOG#"$FLOOR_ROOT"/})"
  $DRY_RUN && return 0
  spawn "$EVOLVE_LOG" "$EVOLVE_DIR" env OPENCRY_TARGET_DIR="${OPENCRY_TARGET_DIR:-}" /usr/bin/env python3 -I -u evolve.py loop --every 3600
  echo "$SPAWNED" > "$EVOLVE_PID_FILE"
  sleep 1
  if [[ -n "$(evolve_pid)" ]]; then log "  ✓ evolve running (pid $(evolve_pid))"; else warn "evolve: exited at once — see $EVOLVE_LOG"; fi
}

page_url() {
  local q=""
  [[ "$TAPE_PORT" != 8083 ]] && q="tape=$TAPE_PORT"
  [[ "$RELAY_PORT" != 8099 ]] && q="${q:+$q&}relay=$RELAY_PORT"
  echo "http://localhost:$TRUNK_PORT/${q:+?$q}"
}

# Where to find a Rust toolchain that can build for the browser. Prints nothing when the first
# cargo on PATH can; prints rustup's toolchain bin directory when only that one can (Rust from
# Homebrew first on PATH, rustup's with the wasm target behind it); fails when neither can.
wasm_toolchain() {
  local lib rc
  lib=$(rustc --print target-libdir --target wasm32-unknown-unknown 2>/dev/null) \
    && compgen -G "$lib/libcore-*" >/dev/null && return 0
  command -v rustup >/dev/null || return 1
  rc=$(rustup which rustc 2>/dev/null) || return 1
  lib=$("$rc" --print target-libdir --target wasm32-unknown-unknown 2>/dev/null) \
    && compgen -G "$lib/libcore-*" >/dev/null && { dirname "$rc"; return 0; }
  return 1
}

start_trunk() {
  local pid; pid=$(port_pid "$TRUNK_PORT")
  if [[ -n "$pid" ]]; then log "trunk: already up on :$TRUNK_PORT (pid $pid) — leaving it alone, a browser may be attached"; return 0; fi
  if [[ "${FLOOR_NO_TRUNK:-0}" == 1 ]]; then log "trunk: FLOOR_NO_TRUNK=1 — not starting"; return 0; fi
  command -v trunk >/dev/null || { warn "trunk: not installed (cargo install trunk; rustup target add wasm32-unknown-unknown), then: cd opencry-visual && trunk serve --port $TRUNK_PORT"; return 0; }
  local tc
  if ! tc=$(wasm_toolchain); then
    warn "trunk: not starting — no Rust toolchain here can build for the browser. Run: rustup target add wasm32-unknown-unknown"
    warn "  (if Rust also came from Homebrew, rustup's cargo has to be the one trunk finds; see the README)"
    return 0
  fi
  [[ -n "$tc" ]] && log "trunk: the first cargo on PATH cannot build for the browser; using rustup's ($tc)"
  log "trunk: starting  (cd opencry-visual && trunk serve --port $TRUNK_PORT >> ${TRUNK_LOG#"$FLOOR_ROOT"/})"
  $DRY_RUN && return 0
  spawn "$TRUNK_LOG" "$VISUAL_DIR" env PATH="${tc:+$tc:}$PATH" trunk serve --port "$TRUNK_PORT"
  log "  trunk builds in the background (a first wasm build takes minutes); watch $TRUNK_LOG"
}

# -----------------------------------------------------------------------------
# Status
# -----------------------------------------------------------------------------

row() { printf '%-8s %-6s %-7s %-12s %-24s %s\n' "$@"; }

status() {
  local tp rp dp ep kp
  tp=$(port_pid "$TAPE_PORT"); rp=$(port_pid "$RELAY_PORT"); dp=$(desk_pid); ep=$(evolve_pid); kp=$(port_pid "$TRUNK_PORT")
  local desk_state=not-built; [[ -x "$DESK_BIN" ]] && { [[ -n "$dp" ]] && desk_state=running || desk_state=down; }
  row SERVICE PORT PID UPTIME HEALTH LOG
  row tape "$TAPE_PORT" "${tp:--}" "$(pid_uptime "$tp")" "$([[ -n "$tp" ]] && tape_health || echo down)" "${TAPE_LOG#"$FLOOR_ROOT"/}"
  row relay "$RELAY_PORT" "${rp:--}" "$(pid_uptime "$rp")" "$([[ -n "$rp" ]] && echo "ok, $(relay_key)" || echo down)" "${RELAY_LOG#"$FLOOR_ROOT"/}"
  row desk "-" "${dp:--}" "$(pid_uptime "$dp")" "$desk_state" "${DESK_LOG#"$FLOOR_ROOT"/}"
  row evolve "-" "${ep:--}" "$(pid_uptime "$ep")" "$([[ -n "$ep" ]] && echo running || echo down)" "${EVOLVE_LOG#"$FLOOR_ROOT"/}"
  row trunk "$TRUNK_PORT" "${kp:--}" "$(pid_uptime "$kp")" "$([[ -n "$kp" ]] && trunk_health || echo down)" "${TRUNK_LOG#"$FLOOR_ROOT"/}"
  echo
  log "the floor: $(page_url)"
}

# -----------------------------------------------------------------------------
# Main
# -----------------------------------------------------------------------------

restart_one() {
  case "$1" in
    tape)   stop_tape; start_tape ;;
    relay)  stop_relay; start_relay ;;
    desk)   stop_desk; start_desk ;;
    evolve) stop_evolve; start_evolve ;;
    trunk)  die "refusing to restart trunk on :$TRUNK_PORT — a browser may be attached to it. Do it by hand if you mean it." ;;
    *)      die "unknown service '$1' (tape | relay | desk | evolve)" ;;
  esac
}

stop_one() {
  case "$1" in
    tape)   stop_tape ;;
    relay)  stop_relay ;;
    desk)   stop_desk ;;
    evolve) stop_evolve ;;
    floor)  stop_evolve; stop_desk; stop_relay; [[ "${FLOOR_NO_TAPE:-0}" == 1 ]] || stop_tape ;;
    trunk)  die "refusing to stop trunk on :$TRUNK_PORT — kill it by hand if you mean it." ;;
    *)      die "unknown service '$1' (tape | relay | desk | evolve | floor)" ;;
  esac
}

usage() { awk 'NR == 1 { next } /^#/ { sub(/^# ?/, ""); print; next } { exit }' "${BASH_SOURCE[0]}"; }

main() {
  command -v lsof >/dev/null || die "lsof not installed"
  command -v curl >/dev/null || die "curl not installed"
  local action="start" target=""
  while (( $# )); do
    case "$1" in
      --status)  action="status" ;;
      --restart) action="restart"; target="${2:-}"; shift ;;
      --stop)    action="stop";    target="${2:-}"; shift ;;
      --dry-run) DRY_RUN=true ;;
      -h|--help) usage; exit 0 ;;
      *) die "unknown argument: $1 (try --help)" ;;
    esac
    shift
  done
  for p in "$TAPE_PORT" "$RELAY_PORT" "$TRUNK_PORT"; do
    port_is_protected "$p" && die "port $p is in FLOOR_PROTECTED_PORTS; pick another with FLOOR_*_PORT"
  done
  case "$action" in
    status)  status ;;
    restart) [[ -n "$target" ]] || die "--restart needs a name (tape | relay | desk | evolve)"; restart_one "$target"; echo; status ;;
    stop)    [[ -n "$target" ]] || die "--stop needs a name (tape | relay | desk | evolve | floor)"; stop_one "$target"; echo; status ;;
    start)
      $DRY_RUN && log "DRY RUN — nothing will be started."
      log "Floor root: $FLOOR_ROOT"
      if [[ "${FLOOR_NO_TAPE:-0}" != 1 ]]; then
        build_if_missing tape "$TAPE_BIN" "$TAPE_DIR"
        TAPE_BIN="$(bin_path "$TAPE_DIR" floor_tape)"
        ensure_cache
      fi
      build_if_missing relay "$RELAY_BIN" "$RELAY_DIR"
      RELAY_BIN="$(bin_path "$RELAY_DIR" jev_relay)"
      start_tape
      start_relay
      start_desk
      start_evolve
      start_trunk
      echo
      status
      ;;
  esac
}

main "$@"
