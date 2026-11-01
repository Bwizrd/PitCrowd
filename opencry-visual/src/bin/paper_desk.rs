//! The headless paper desk: the floor's six paper traders on the live tape, around the clock,
//! with no browser. It is the same feed, candle, level, Jev and book code the app runs
//! (`src/paper.rs`), driven by a plain loop instead of Bevy, so the experiment Paul cares
//! about (do any of them beat the coin flip after the spread, over weeks?) accumulates
//! whether or not a tab is open.
//!
//!   cargo run --release --bin paper_desk -- --jev live --jev-every 60
//!   nohup $CARGO_TARGET_DIR/release/paper_desk --jev live --jev-every 60 \
//!        > opencry-visual/log/paper_desk.out 2>&1 &
//!
//! Flags:
//!   --jev live|rule|none    live asks Jev through the relay (default); rule uses the floor
//!                           rule of thumb (free); none leaves the three Jev traders flat
//!   --jev-every 60          seconds between reads of each pit; the pits are staggered across
//!                           the cadence so calls never bunch at the relay's 2 s gate
//!   --touch-reads on|off    ask Jev the moment price touches a level (default on): the read
//!                           carries a TRIGGER line naming the level and the side price came
//!                           from, and counts as that pit's timed read
//!   --touch-follow 30       seconds after a touch to ask again how the reaction is developing
//!                           (0 = never)
//!   --touch-cooldown 60     at most one touch read per pit per this many seconds
//!   --symbols all|XAGUSD,EURUSD
//!   --seed 42               the control's coin
//!   --log-dir <dir>         local JSONL copies (default opencry-visual/log, env OPENCRY_DESK_LOG_DIR)
//!   --heartbeat-every 10    seconds between `POST /paper/heartbeat` to the relay
//!   --backfill -6h          tape `/bars` range loaded at start (and after a long outage)
//!   --no-relay-log          keep trades and decisions local only
//!   --keep Gus,Lena,Colin   seat only these of the built-in traders (default: all of them)
//!   --pit <file>            the evolving pit's seats (`evolve/pit.json`), reloaded every
//!                           minute: each genome trades 15m bar closes on its basket, and one
//!                           that leaves the file retires once it is flat
//!   --print-questions       print the question set every read carries (JSON) and exit
//!
//! Env: OPENCRY_TAPE_URL (http://host:8083; the websocket address is derived from it, or set
//! OPENCRY_FEED_URL to a ws:// address directly) and OPENCRY_JEV_URL (the relay, :8099).
//!
//! What it writes: every trade and decision to the relay's `/log/paper_trades` and
//! `/log/paper_decisions` exactly as the app does, plus `source: "desk"` (the app's rows
//! carry no source), and local copies `paper_desk_trades.jsonl` / `paper_desk_decisions.jsonl`.
//! The heartbeat carries a rolling 24 h leaderboard seeded from the local trade log at start,
//! so a restart does not blank the board.

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    if let Err(e) = desk::run() {
        eprintln!("paper_desk: {e}");
        std::process::exit(1);
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod desk {
    use std::collections::HashMap;
    use std::fs;
    use std::io::Write;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{mpsc, Arc};
    use std::time::{Duration, Instant};

    use opencry_visual::candles::{parse_backfill, wall_clock_ms, Candle, CandleStore, Timeframe};
    use opencry_visual::desk_feed::{DeskBoard, DeskJevSplit, DeskOpen, DeskPass, DeskPit, DeskRead, DeskSeat, DeskTrade, DeskTrader, BOARD_WINDOW_HOURS, LAST_TRADES_KEEP};
    use opencry_visual::jev::{
        all_questions, relay_url, system_one_request, system_one_result, DeterministicModel, JevDesk, JevQuestion, MarketInterpretation,
        MarketObservation, ModelError, ObsTrigger, ReadSource,
    };
    use opencry_visual::levels::{tape_symbol, tape_url, LevelStore};
    use opencry_visual::live_feed::{parse_tape, FeedMode, FeedState, LiveFeed, PitFeedStatus, TapeMessage};
    use opencry_visual::market::Instrument;
    use opencry_visual::genome::{self, Bar, Features, Genome, BAR_MS};
    use opencry_visual::paper::{Archetype, Entry, PaperBook, Trade, View, READ_FRESH_SECS};
    use opencry_visual::pit_engine::tick_size;
    use opencry_visual::reaction::{unit_ticks, Step, Touch, TouchTracker};
    use opencry_visual::tape_chart::BOARD_BARS;

    /// Marks every row this process writes, so the app's rows (no source) can be told apart.
    const SOURCE: &str = "desk";
    const RECONNECT_MIN_SECS: f64 = 1.0;
    const RECONNECT_MAX_SECS: f64 = 30.0;
    /// The relay allows one Jev call per 2 s; the worker keeps a little clear of it.
    const RELAY_GAP: Duration = Duration::from_millis(2100);
    const JEV_RETRIES: u32 = 4;
    /// After an outage this long the 1m bars are reloaded from the tape to close the hole.
    const REBACKFILL_AFTER_SECS: f64 = 120.0;
    const STATUS_EVERY_SECS: f64 = 60.0;
    /// A pit that cannot be observed (no ticks yet, no bars) is asked again this soon.
    const OBSERVE_RETRY_SECS: f64 = 5.0;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum JevMode {
        None,
        Rule,
        Live,
    }

    impl JevMode {
        fn label(self) -> &'static str {
            match self {
                JevMode::None => "none",
                JevMode::Rule => "rule",
                JevMode::Live => "live",
            }
        }
    }

    struct Args {
        symbols: Vec<Instrument>,
        jev: JevMode,
        jev_every: f64,
        seed: u64,
        log_dir: PathBuf,
        heartbeat_every: f64,
        backfill: String,
        relay_log: bool,
        touch_reads: bool,
        touch_follow: f64,
        touch_cooldown: f64,
        keep: Option<Vec<String>>,
        pit: Option<PathBuf>,
    }

    /// Where the local records go when nothing says otherwise: `opencry-visual/log` next to
    /// the binary's `target/` tree, found from the executable at run time. The crate's manifest
    /// dir is only the fallback, because it is baked in at compile time: a binary built in a
    /// worktree and copied into the main checkout would otherwise keep writing to, and
    /// reloading its history from, the worktree.
    pub fn default_log_dir(exe: Option<PathBuf>) -> PathBuf {
        if let Some(exe) = exe {
            // .../opencry-visual/target/<profile>/paper_desk  ->  .../opencry-visual/log
            let mut up = exe.as_path();
            for _ in 0..3 {
                match up.parent() {
                    Some(p) => up = p,
                    None => break,
                }
            }
            if up.join("Cargo.toml").is_file() {
                return up.join("log");
            }
        }
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("log")
    }

    fn arg(name: &str) -> Option<String> {
        let args: Vec<String> = std::env::args().collect();
        args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
    }

    fn flag(name: &str) -> bool {
        std::env::args().any(|a| a == name)
    }

    fn parse_symbols(list: Option<&str>) -> Result<Vec<Instrument>, String> {
        match list {
            None | Some("all") => Ok(Instrument::ALL.to_vec()),
            Some(list) => list
                .split(',')
                .map(|s| {
                    let want = s.trim().to_uppercase().replace('/', "");
                    Instrument::ALL.iter().copied().find(|i| tape_symbol(*i) == want).ok_or(format!("unknown symbol {s}"))
                })
                .collect(),
        }
    }

    fn parse_args() -> Result<Args, String> {
        let symbols = parse_symbols(arg("--symbols").as_deref())?;
        let jev = match arg("--jev").as_deref().unwrap_or("live") {
            "none" => JevMode::None,
            "rule" => JevMode::Rule,
            "live" => JevMode::Live,
            other => return Err(format!("--jev {other}: none | rule | live")),
        };
        let jev_every = arg("--jev-every").and_then(|v| v.parse().ok()).unwrap_or(60.0f64).max(10.0);
        let seed = arg("--seed").and_then(|v| v.parse().ok()).unwrap_or(42);
        let log_dir = arg("--log-dir")
            .or_else(|| std::env::var("OPENCRY_DESK_LOG_DIR").ok())
            .map(PathBuf::from)
            .unwrap_or_else(|| default_log_dir(std::env::current_exe().ok()));
        let heartbeat_every = arg("--heartbeat-every").and_then(|v| v.parse().ok()).unwrap_or(3.0f64).max(1.0);
        let backfill = arg("--backfill").unwrap_or_else(|| "-6h".into());
        let touch_reads = match arg("--touch-reads").as_deref().unwrap_or("on") {
            "on" => true,
            "off" => false,
            other => return Err(format!("--touch-reads {other}: on | off")),
        };
        let touch_follow = arg("--touch-follow").and_then(|v| v.parse().ok()).unwrap_or(30.0f64).max(0.0);
        let touch_cooldown = arg("--touch-cooldown").and_then(|v| v.parse().ok()).unwrap_or(60.0f64).max(5.0);
        let keep = arg("--keep").map(|v| v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect());
        let pit = arg("--pit").map(PathBuf::from);
        Ok(Args { symbols, jev, jev_every, seed, log_dir, heartbeat_every, backfill, relay_log: !flag("--no-relay-log"), touch_reads, touch_follow, touch_cooldown, keep, pit })
    }

    /// `ws://` address of the tape from its `http://` one.
    fn ws_from_http(http: &str) -> String {
        if let Some(rest) = http.strip_prefix("https://") {
            format!("wss://{rest}")
        } else if let Some(rest) = http.strip_prefix("http://") {
            format!("ws://{rest}")
        } else {
            http.to_string()
        }
    }

    fn feed_ws_url() -> String {
        std::env::var("OPENCRY_FEED_URL").ok().filter(|v| !v.is_empty()).unwrap_or_else(|| ws_from_http(&tape_url()))
    }

    fn fmt_ms(ms: i64) -> String {
        let secs = ms.div_euclid(1000);
        let days = secs.div_euclid(86_400);
        let day = secs.rem_euclid(86_400);
        let z = days + 719_468;
        let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let y = yoe + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = doy - (153 * mp + 2) / 5 + 1;
        let m = if mp < 10 { mp + 3 } else { mp - 9 };
        let y = if m <= 2 { y + 1 } else { y };
        format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", day / 3600, (day % 3600) / 60, day % 60)
    }

    fn log_line(msg: &str) {
        eprintln!("{} paper_desk: {msg}", fmt_ms(wall_clock_ms()));
    }

    // -----------------------------------------------------------------------
    // The rolling leaderboard
    // -----------------------------------------------------------------------

    /// What the board needs from a closed trade: the scoring fields, plus what the floor's
    /// avatar panel shows about it (`DeskTrade`).
    #[derive(Debug, Clone, PartialEq, Default)]
    pub struct Closed {
        pub trader: String,
        pub closed_ms: i64,
        pub pnl_r: f32,
        pub spread_r: f32,
        pub shown: DeskTrade,
    }

    pub fn closed_from_trade(t: &Trade) -> Closed {
        Closed {
            trader: t.trader.to_string(),
            closed_ms: t.closed_ms,
            pnl_r: t.pnl_r,
            spread_r: t.spread_ticks_in / t.stop_ticks.max(1.0),
            shown: DeskTrade::from_trade(t),
        }
    }

    /// A row of the local trade log (a serialised `Trade` plus `source`).
    pub fn closed_from_json(v: &serde_json::Value) -> Option<Closed> {
        let f = |k: &str| v.get(k).and_then(|x| x.as_f64());
        let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
        let closed_ms = v.get("closed_ms")?.as_i64()?;
        let pnl_r = f("pnl_r")? as f32;
        Some(Closed {
            trader: v.get("trader")?.as_str()?.to_string(),
            closed_ms,
            pnl_r,
            spread_r: (f("spread_ticks_in").unwrap_or(0.0) / f("stop_ticks").unwrap_or(1.0).max(1.0)) as f32,
            shown: DeskTrade {
                symbol: s("symbol"),
                side: s("side"),
                entry: f("entry").unwrap_or(0.0) as f32,
                exit: f("exit").unwrap_or(0.0) as f32,
                pnl_r,
                opened_ms: v.get("opened_ms").and_then(|x| x.as_i64()).unwrap_or(0),
                closed_ms,
                why_in: s("why_in"),
                why_out: s("why_out"),
            },
        })
    }

    /// A trader's most recent closed trades, newest first, at most `LAST_TRADES_KEEP`.
    pub fn last_trades(history: &[Closed], trader: &str) -> Vec<DeskTrade> {
        let mut mine: Vec<&Closed> = history.iter().filter(|c| c.trader == trader).collect();
        mine.sort_by_key(|c| std::cmp::Reverse(c.closed_ms));
        mine.iter().take(LAST_TRADES_KEEP).map(|c| c.shown.clone()).collect()
    }

    /// Per-trader totals over the trades closed in the window, in `Archetype::ALL` order.
    /// Drawdown is measured over the window's own sequence from zero, like `Trader::settle`.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn leaderboard(history: &[Closed], now_ms: i64, window_ms: i64) -> Vec<DeskTrader> {
        let roster: Vec<(&str, &str)> = Archetype::ALL.iter().map(|a| (a.name(), a.label())).collect();
        leaderboard_for(&roster, history, now_ms, window_ms)
    }

    /// The board for these traders, in this order: (name, label) per row.
    pub fn leaderboard_for(roster: &[(&str, &str)], history: &[Closed], now_ms: i64, window_ms: i64) -> Vec<DeskTrader> {
        let mut rows: Vec<DeskTrader> = roster.iter().map(|(n, l)| DeskTrader { trader: n.to_string(), archetype: l.to_string(), ..Default::default() }).collect();
        let mut peaks = vec![0.0f32; rows.len()];
        let mut window: Vec<&Closed> = history.iter().filter(|c| c.closed_ms > now_ms - window_ms && c.closed_ms <= now_ms).collect();
        window.sort_by_key(|c| c.closed_ms);
        for c in window {
            let Some(i) = roster.iter().position(|(n, _)| *n == c.trader) else { continue };
            let r = &mut rows[i];
            r.trades += 1;
            if c.pnl_r > 0.0 { r.wins += 1 } else { r.losses += 1 }
            r.net_r += c.pnl_r;
            r.spread_paid_r += c.spread_r;
            peaks[i] = peaks[i].max(r.net_r);
            r.max_drawdown_r = r.max_drawdown_r.max(peaks[i] - r.net_r);
        }
        rows
    }

    fn load_history(path: &PathBuf, now_ms: i64, window_ms: i64) -> Vec<Closed> {
        let Ok(text) = fs::read_to_string(path) else { return Vec::new() };
        text.lines()
            .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
            .filter_map(|v| closed_from_json(&v))
            .filter(|c| c.closed_ms > now_ms - window_ms)
            .collect()
    }

    // -----------------------------------------------------------------------
    // Records: the same rows the app writes, plus `source`
    // -----------------------------------------------------------------------

    pub fn trade_json(t: &Trade) -> serde_json::Value {
        let mut v = serde_json::to_value(t).unwrap_or(serde_json::Value::Null);
        if let serde_json::Value::Object(m) = &mut v {
            m.insert("source".into(), serde_json::Value::String(SOURCE.into()));
        }
        v
    }

    #[allow(clippy::too_many_arguments)]
    pub fn decision_json(
        trader: &str,
        archetype: Archetype,
        symbol: &str,
        at_ms: i64,
        action: &str,
        side: &str,
        price: f32,
        bid: f32,
        ask: f32,
        why: &str,
        observation_hash: Option<&str>,
    ) -> serde_json::Value {
        serde_json::json!({
            "trader": trader, "archetype": archetype, "symbol": symbol, "at_ms": at_ms, "action": action,
            "side": side, "price": price, "bid": bid, "ask": ask, "why": why, "observation_hash": observation_hash,
            "source": SOURCE,
        })
    }

    fn side_str(side: opencry_visual::pit_engine::Side) -> &'static str {
        if side.is_buy() { "long" } else { "short" }
    }

    /// What the book looked like before a step, per trader: trades closed and the open position.
    #[derive(Debug, Clone, PartialEq)]
    pub struct Snapshot {
        trades: Vec<usize>,
        open: Vec<Option<(Instrument, i64)>>,
    }

    impl Snapshot {
        pub fn of(book: &PaperBook) -> Self {
            Self {
                trades: book.traders.iter().map(|t| t.trades.len()).collect(),
                open: book.traders.iter().map(|t| t.position.as_ref().map(|p| (p.inst, p.opened_ms))).collect(),
            }
        }

        /// New closed trades and new entries since this snapshot, as (trader index, ...).
        pub fn diff<'a>(&self, book: &'a PaperBook) -> (Vec<(usize, &'a Trade)>, Vec<usize>) {
            let mut closed = Vec::new();
            let mut opened = Vec::new();
            for (t, tr) in book.traders.iter().enumerate() {
                for trade in tr.trades.iter().skip(self.trades[t]) {
                    closed.push((t, trade));
                }
                let now_open = tr.position.as_ref().map(|p| (p.inst, p.opened_ms));
                if now_open.is_some() && now_open != self.open[t] {
                    opened.push(t);
                }
            }
            (closed, opened)
        }
    }

    // -----------------------------------------------------------------------
    // Relay I/O
    // -----------------------------------------------------------------------

    /// Fire-and-forget POST; failures are counted, not waited for.
    struct Poster {
        relay: String,
        enabled: bool,
        failures: Arc<AtomicU64>,
        posted: Arc<AtomicU64>,
        /// Last HTTP status of a heartbeat (0 = none yet, 404 = relay without the route,
        /// 1 = relay unreachable). Heartbeats are not counted in `posted` / `failures`: those
        /// are the trade and decision records, and a 404 heartbeat is expected on an old relay.
        heartbeat_status: Arc<AtomicU64>,
    }

    impl Poster {
        fn post(&self, path: &str, value: &serde_json::Value, status_out: Option<Arc<AtomicU64>>) {
            if !self.enabled {
                return;
            }
            let Ok(body) = serde_json::to_vec(value) else { return };
            let mut req = ehttp::Request::post(format!("{}{path}", self.relay), body);
            req.headers = ehttp::Headers::new(&[("Accept", "application/json"), ("Content-Type", "application/json")]);
            let failures = self.failures.clone();
            let posted = self.posted.clone();
            ehttp::fetch(req, move |res| {
                if let Some(s) = status_out {
                    s.store(res.as_ref().map(|r| r.status as u64).unwrap_or(1), Ordering::Relaxed);
                    return;
                }
                match res {
                    Ok(r) if r.ok => {
                        posted.fetch_add(1, Ordering::Relaxed);
                    }
                    _ => {
                        failures.fetch_add(1, Ordering::Relaxed);
                    }
                }
            });
        }

        fn log(&self, name: &str, value: &serde_json::Value) {
            self.post(&format!("/log/{name}"), value, None);
        }

        fn heartbeat(&self, board: &DeskBoard) {
            let v = serde_json::to_value(board).unwrap_or(serde_json::Value::Null);
            self.post("/paper/heartbeat", &v, Some(self.heartbeat_status.clone()));
        }
    }

    fn append_jsonl(path: &PathBuf, value: &serde_json::Value) {
        if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(f, "{value}");
        }
    }

    // -----------------------------------------------------------------------
    // Jev on a worker thread, paced to the relay
    // -----------------------------------------------------------------------

    struct JevJob {
        inst: Instrument,
        obs: MarketObservation,
    }

    struct JevDone {
        inst: Instrument,
        res: Result<MarketInterpretation, ModelError>,
        calls: u32,
        /// The touch the read was asked about, carried back so the crew can trade it.
        trigger: Option<ObsTrigger>,
    }

    /// Send an observation for a read in the configured mode. Returns whether it went.
    fn dispatch(mode: JevMode, desk: &mut JevDesk, job_tx: &mpsc::Sender<JevJob>, queued: &mut [bool], n_reads: &mut u64, inst: Instrument, obs: MarketObservation, now: f64) -> bool {
        match mode {
            JevMode::Rule => {
                let mut r = DeterministicModel::read(&obs);
                r.at = now;
                desk.set_read(inst, r);
                *n_reads += 1;
                true
            }
            JevMode::Live => {
                if job_tx.send(JevJob { inst, obs }).is_ok() {
                    queued[inst as usize] = true;
                    true
                } else {
                    false
                }
            }
            JevMode::None => false,
        }
    }

    fn trigger_for(t: &Touch, kind: &str, secs_since: f64) -> ObsTrigger {
        ObsTrigger { kind: kind.into(), level: t.kind.tag().into(), lo: t.lo, hi: t.hi, side: t.side.label().into(), secs_since: secs_since as f32 }
    }

    fn spawn_jev_worker(relay: String, questions: HashMap<String, JevQuestion>) -> (mpsc::Sender<JevJob>, mpsc::Receiver<JevDone>) {
        let (job_tx, job_rx) = mpsc::channel::<JevJob>();
        let (done_tx, done_rx) = mpsc::channel::<JevDone>();
        std::thread::Builder::new()
            .name("jev".into())
            .spawn(move || {
                let mut last_call = Instant::now() - RELAY_GAP;
                for job in job_rx {
                    let hash = job.obs.hash();
                    let trigger = job.obs.trigger.clone();
                    let mut calls = 0;
                    let res = loop {
                        let since = last_call.elapsed();
                        if since < RELAY_GAP {
                            std::thread::sleep(RELAY_GAP - since);
                        }
                        let req = match system_one_request(&relay, &job.obs, &questions) {
                            Ok(r) => r,
                            Err(e) => break Err(e),
                        };
                        let sent = Instant::now();
                        let res = ehttp::fetch_blocking(&req);
                        last_call = Instant::now();
                        calls += 1;
                        match system_one_result(res, &hash, sent.elapsed().as_millis() as u64) {
                            Err(ModelError::RateLimited(ms)) if calls <= JEV_RETRIES => {
                                std::thread::sleep(Duration::from_millis(ms + 100));
                            }
                            other => break other,
                        }
                    };
                    if done_tx.send(JevDone { inst: job.inst, res, calls, trigger }).is_err() {
                        return;
                    }
                }
            })
            .expect("jev worker thread");
        (job_tx, done_rx)
    }

    // -----------------------------------------------------------------------
    // The tape socket
    // -----------------------------------------------------------------------

    struct Socket {
        url: String,
        pair: Option<(ewebsock::WsSender, ewebsock::WsReceiver)>,
        next_connect_at: f64,
        backoff: f64,
        /// When the socket went down, so a long outage can trigger a re-backfill.
        down_since: Option<f64>,
        reconnects: u32,
    }

    impl Socket {
        fn new(url: String) -> Self {
            Self { url, pair: None, next_connect_at: 0.0, backoff: RECONNECT_MIN_SECS, down_since: Some(0.0), reconnects: 0 }
        }

        fn ensure_connected(&mut self, now: f64) {
            if self.pair.is_some() || now < self.next_connect_at {
                return;
            }
            match ewebsock::connect(self.url.clone(), ewebsock::Options::default()) {
                Ok(pair) => {
                    self.pair = Some(pair);
                    log_line(&format!("connecting to {}", self.url));
                }
                Err(e) => {
                    log_line(&format!("connect failed: {e}; retry in {:.0}s", self.backoff));
                    self.next_connect_at = now + self.backoff;
                    self.backoff = (self.backoff * 2.0).min(RECONNECT_MAX_SECS);
                }
            }
        }

        fn drop(&mut self, now: f64, why: &str) {
            self.pair = None;
            if self.down_since.is_none() {
                self.down_since = Some(now);
            }
            self.next_connect_at = now + self.backoff;
            log_line(&format!("socket {why}; reconnect in {:.0}s", self.backoff));
            self.backoff = (self.backoff * 2.0).min(RECONNECT_MAX_SECS);
        }

        fn drain(&mut self) -> Vec<ewebsock::WsEvent> {
            let mut out = Vec::new();
            if let Some((_, rx)) = self.pair.as_ref() {
                while let Some(ev) = rx.try_recv() {
                    out.push(ev);
                    if out.len() > 4096 {
                        break;
                    }
                }
            }
            out
        }
    }

    // -----------------------------------------------------------------------
    // Backfill from the tape (never the bridge: its history API is rate-limited)
    // -----------------------------------------------------------------------

    fn backfill(store: &mut CandleStore, inst: Instrument, range: &str) -> Result<usize, String> {
        let url = format!("{}/bars?symbol={}&timeframe=1m&range={range}", tape_url(), tape_symbol(inst));
        let res = ehttp::fetch_blocking(&ehttp::Request::get(url)).map_err(|e| format!("tape: {e}"))?;
        if !res.ok {
            return Err(format!("tape HTTP {} {}", res.status, res.text().unwrap_or("").chars().take(120).collect::<String>()));
        }
        let bars = parse_backfill(res.text().unwrap_or(""));
        let s = store.live_mut(inst, Timeframe::M1);
        for b in bars {
            s.upsert(b);
        }
        s.backfilled = true;
        Ok(s.bars.len())
    }

    fn backfill_all(store: &mut CandleStore, symbols: &[Instrument], range: &str) {
        for inst in symbols {
            match backfill(store, *inst, range) {
                Ok(n) => log_line(&format!("backfilled {} 1m from the tape ({n} bars)", tape_symbol(*inst))),
                Err(e) => log_line(&format!("backfill {} failed: {e} (bars will build from ticks)", tape_symbol(*inst))),
            }
        }
    }

    // -----------------------------------------------------------------------
    // The evolving pit: genomes on 15m bar closes
    // -----------------------------------------------------------------------

    const GENOME_BARS_KEEP: usize = 2500;

    fn edge_cache() -> String {
        opencry_visual::edge_cache_dir()
    }
    const PIT_RELOAD_SECS: f64 = 60.0;

    /// Ask edge-lab's own Python (`evolve/edge_lab.py live`, next to the pit file) whether its
    /// strategy, on `tf`-minute bars built from these, enters on the last of them: +1, -1 or 0, or None if it fails or takes
    /// over ten seconds (a strategy is only eligible if one call takes under a second).
    fn edge_lab_entry(pit: &std::path::Path, key: &str, tf: u32, bars: &[Bar]) -> Option<i8> {
        use std::io::Write as _;
        let script = pit.parent()?.join("edge_lab.py");
        let rows: Vec<[f64; 5]> = bars.iter().map(|b| [b.t as f64, b.o, b.h, b.l, b.c]).collect();
        let mut child = std::process::Command::new("python3")
            .arg("-I")
            .arg(&script)
            .arg("live")
            .arg(key)
            .arg(tf.to_string())
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .ok()?;
        child.stdin.take()?.write_all(serde_json::to_string(&rows).ok()?.as_bytes()).ok()?;
        let t0 = std::time::Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if t0.elapsed().as_secs_f64() < 10.0 => std::thread::sleep(std::time::Duration::from_millis(20)),
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
            }
        }
        let out = child.wait_with_output().ok()?;
        let v: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
        (v["t"].as_i64()? == bars.last()?.t).then(|| v["d"].as_i64().unwrap_or(0) as i8)
    }

    /// Ask edge-lab's Python (`evolve/holders.py live`, next to the pit file) for each holder's
    /// position at the end of its last finished bar, on a thread: one process for every holder
    /// of one pit, given the desk's 15m bars (it reads the older history from edge-lab's cache).
    fn ask_holders(tx: mpsc::Sender<Vec<HolderAnswer>>, pit: PathBuf, jobs: Vec<(u16, String, String, u32, Instrument)>, bars: Vec<Bar>, asked_ms: i64) {
        std::thread::spawn(move || {
            use std::io::Write as _;
            let rows: Vec<[f64; 5]> = bars.iter().map(|b| [b.t as f64, b.o, b.h, b.l, b.c]).collect();
            let input: Vec<serde_json::Value> = jobs.iter().map(|(_, _, key, tf, inst)| serde_json::json!({"key": key, "tf": tf, "symbol": tape_symbol(*inst), "bars": rows})).collect();
            let fail = |e: String| jobs.iter().map(|(seat, id, _, _, inst)| HolderAnswer { seat: *seat, id: id.clone(), inst: *inst, asked_ms, g: 0, pos: 0, unit: 0.0, error: Some(e.clone()) }).collect::<Vec<_>>();
            let Some(script) = pit.parent().map(|d| d.join("holders.py")) else { return };
            let child = std::process::Command::new("python3").arg("-I").arg(&script).arg("live").stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null()).spawn();
            let mut child = match child {
                Ok(c) => c,
                Err(e) => {
                    let _ = tx.send(fail(format!("spawn: {e}")));
                    return;
                }
            };
            if let Some(mut w) = child.stdin.take() {
                let _ = w.write_all(serde_json::to_string(&input).unwrap_or_default().as_bytes());
            }
            let t0 = Instant::now();
            loop {
                match child.try_wait() {
                    Ok(Some(_)) => break,
                    Ok(None) if t0.elapsed() < Duration::from_secs(120) => std::thread::sleep(Duration::from_millis(50)),
                    _ => {
                        let _ = child.kill();
                        let _ = child.wait();
                        let _ = tx.send(fail("holders.py took over 120 s".into()));
                        return;
                    }
                }
            }
            let out = child.wait_with_output().map(|o| o.stdout).unwrap_or_default();
            let Ok(v) = serde_json::from_slice::<Vec<serde_json::Value>>(&out) else {
                let _ = tx.send(fail(format!("holders.py: {}", String::from_utf8_lossy(&out).chars().take(120).collect::<String>())));
                return;
            };
            let answers = jobs
                .iter()
                .zip(v.iter())
                .map(|((seat, id, _, _, inst), a)| HolderAnswer {
                    seat: *seat,
                    id: id.clone(),
                    inst: *inst,
                    asked_ms,
                    g: a.get("g").and_then(|x| x.as_i64()).unwrap_or(0),
                    pos: a.get("pos").and_then(|x| x.as_i64()).unwrap_or(0).clamp(-1, 1) as i8,
                    unit: a.get("unit").and_then(|x| x.as_f64()).unwrap_or(0.0),
                    error: a.get("error").and_then(|x| x.as_str()).map(str::to_string),
                })
                .collect();
            let _ = tx.send(answers);
        });
    }

    /// An adapter's tactics scored on the desk's own bars: (tactic, trades, mean R) over the last
    /// `ADAPT_DAYS` days, and the code's pick (the best mean with 3 or more trades).
    fn score_tactics(g: &Genome, bars: &[Vec<Bar>], wall_ms: i64) -> (Vec<(usize, usize, f64)>, Option<usize>) {
        let from = wall_ms - genome::ADAPT_DAYS * 86_400_000;
        let mut data: HashMap<String, Features> = HashMap::new();
        for t in &g.tactics {
            for sym in &t.symbols {
                if data.contains_key(sym) {
                    continue;
                }
                if let Some(inst) = genome::instrument_of(sym) {
                    let b = &bars[inst as usize];
                    if !b.is_empty() {
                        data.insert(sym.clone(), Features::new(b.clone()));
                    }
                }
            }
        }
        let scores: Vec<(usize, usize, f64)> = g
            .tactics
            .iter()
            .enumerate()
            .map(|(k, t)| {
                let r: Vec<f64> = genome::backtest(t, &mut data).into_iter().filter(|x| x.t >= from).map(|x| x.r).collect();
                let m = if r.is_empty() { 0.0 } else { r.iter().sum::<f64>() / r.len() as f64 };
                (k, r.len(), m)
            })
            .collect();
        let pick = scores.iter().filter(|s| s.1 >= 3).max_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal)).map(|s| s.0);
        (scores, pick)
    }

    /// What an adapter shows Jev: the markets its tactics trade, from the desk's 15m bars, and
    /// each tactic with its last 20 days.
    fn adapt_state(g: &Genome, bars: &[Vec<Bar>], scores: &[(usize, usize, f64)]) -> String {
        let mut syms: Vec<&String> = g.tactics.iter().flat_map(|t| t.symbols.iter()).collect();
        syms.sort();
        syms.dedup();
        let mut lines = vec!["A trading bot chooses ONE tactic to trade today, on 15-minute charts. The markets, from their last 20 trading days of 15-minute bars:".to_string()];
        for sym in syms {
            let Some(inst) = genome::instrument_of(sym) else { continue };
            let b = &bars[inst as usize];
            // Daily closes and ranges from the 15m bars, by UTC day.
            let mut days: Vec<(i64, f64, f64, f64)> = Vec::new();
            for x in b {
                let d = x.t.div_euclid(86_400_000);
                match days.last_mut() {
                    Some(l) if l.0 == d => {
                        l.1 = x.c;
                        l.2 = l.2.max(x.h);
                        l.3 = l.3.min(x.l);
                    }
                    _ => days.push((d, x.c, x.h, x.l)),
                }
            }
            let n = days.len();
            if n < 22 {
                continue;
            }
            let ch = |k: usize| (days[n - 1].1 / days[n - 1 - k].1 - 1.0) * 100.0;
            let rng = |d: &(i64, f64, f64, f64)| (d.2 - d.3) / d.1;
            let avg20 = days[n - 21..n - 1].iter().map(rng).sum::<f64>() / 20.0;
            let last5 = days[n - 5..].iter().map(rng).sum::<f64>() / 5.0;
            let ups = days[n - 20..].windows(2).filter(|w| w[1].1 > w[0].1).count();
            lines.push(format!(
                "- {sym}: last day {:+.2}%, 5 days {:+.2}%, 20 days {:+.2}%; up on {ups} of the last 19 days; daily range over the last 5 days {:.1}x its 20-day average",
                ch(1),
                ch(5),
                ch(20),
                if avg20 > 0.0 { last5 / avg20 } else { 1.0 }
            ));
        }
        lines.push("The tactics, each with its record over the last 20 days of these same bars (R = multiples of the risk taken):".into());
        for (k, t) in g.tactics.iter().enumerate() {
            let (n, m) = scores.iter().find(|s| s.0 == k).map(|s| (s.1, s.2)).unwrap_or((0, 0.0));
            lines.push(format!("{}. {}  |  last 20 days: {n} trades, {:+.2}R a trade, {:+.1}R in all", k + 1, t.rule_text(), m, m * n as f64));
        }
        lines.join("\n")
    }

    fn ask_adapter(tx: mpsc::Sender<AdaptAnswer>, mut a: AdaptAnswer, state: String, k: usize) {
        std::thread::spawn(move || {
            let mut qs = serde_json::Map::new();
            for i in 0..k {
                qs.insert(format!("tactic_{}", i + 1), serde_json::json!({"type": "noul", "instructions": format!("How likely is tactic {} to make money if the bot trades it today, given these markets and its recent record? 1 = very likely, 0 = very unlikely.", i + 1)}));
            }
            let body = serde_json::json!({"state": state, "questions": qs});
            let started = Instant::now();
            loop {
                let mut req = ehttp::Request::post(format!("{}/jev", relay_url()), serde_json::to_vec(&body).unwrap_or_default());
                req.headers = ehttp::Headers::new(&[("Accept", "application/json"), ("Content-Type", "application/json")]);
                match ehttp::fetch_blocking(&req) {
                    Ok(res) if res.status == 429 && started.elapsed() < Duration::from_secs(120) => {
                        let wait = serde_json::from_slice::<serde_json::Value>(&res.bytes).ok().and_then(|v| v.get("retry_after_ms").and_then(|x| x.as_u64())).unwrap_or(2000);
                        std::thread::sleep(Duration::from_millis(wait + 200));
                    }
                    Ok(res) if res.ok => {
                        let v: serde_json::Value = serde_json::from_slice(&res.bytes).unwrap_or_default();
                        let ps: Option<Vec<f32>> = (0..k).map(|i| v.pointer(&format!("/answers/tactic_{}/noul", i + 1)).and_then(|x| x.as_f64()).map(|x| x as f32)).collect();
                        if ps.is_none() {
                            a.error = Some(format!("no answer in {}", String::from_utf8_lossy(&res.bytes).chars().take(160).collect::<String>()));
                        }
                        a.jev = ps;
                        break;
                    }
                    Ok(res) => {
                        a.error = Some(format!("relay HTTP {}", res.status));
                        break;
                    }
                    Err(e) => {
                        a.error = Some(format!("relay: {e}"));
                        break;
                    }
                }
            }
            let _ = tx.send(a);
        });
    }

    struct GenomePit {
        path: PathBuf,
        next_reload: f64,
        /// Seat index by genome id, for every genome this process has seated.
        seated: HashMap<String, u16>,
        retiring: std::collections::HashSet<u16>,
        /// Completed 15m bars per pit, oldest first.
        bars: Vec<Vec<Bar>>,
        /// The 15m bucket (wall ms / 15 min) each pit last closed.
        bucket: Vec<i64>,
        /// pit.json as last read, for the board.
        last: Option<serde_json::Value>,
        /// Twin seat by its genome's seat, for genomes seated with a twin.
        twins: HashMap<u16, u16>,
        /// Jev's answers coming back for the twins, from the ask threads.
        answers_tx: mpsc::Sender<TwinAnswer>,
        answers_rx: mpsc::Receiver<TwinAnswer>,
        /// Answers back but for another pit's tick.
        ready: Vec<TwinAnswer>,
        jev_log: PathBuf,
        /// Holders: the strategy's last finished bar and position, by "id@SYMBOL", as acted on.
        holder_state: HashMap<String, (i64, i8)>,
        /// Holders asked once since start, so a first answer can set their state.
        holder_asked: std::collections::HashSet<String>,
        holder_tx: mpsc::Sender<Vec<HolderAnswer>>,
        holder_rx: mpsc::Receiver<Vec<HolderAnswer>>,
        holder_ready: Vec<HolderAnswer>,
        /// Holder state and open holder positions, so a restart picks them up.
        holder_file: PathBuf,
        restored: bool,
        /// Adapters: the UTC day each last chose a tactic.
        adapt_day: HashMap<u16, i64>,
        adapt_tx: mpsc::Sender<AdaptAnswer>,
        adapt_rx: mpsc::Receiver<AdaptAnswer>,
        adapt_log: PathBuf,
        /// The trade ticket: its trader's seat, its orders, and what became of the last one.
        manual_seat: Option<u16>,
        /// Kept so tests can queue orders; the poller holds its own clone.
        #[cfg_attr(not(test), allow(dead_code))]
        manual_tx: mpsc::Sender<ManualOrder>,
        manual_rx: mpsc::Receiver<ManualOrder>,
        manual_ready: Vec<ManualOrder>,
        manual_last: Option<String>,
    }

    /// A manual paper order from the page's trade ticket, through the relay.
    struct ManualOrder {
        seq: u64,
        action: String,
        symbol: String,
        stop_atr: f64,
        target_r: f64,
        placed_ms: i64,
        /// "paper" (the desk's own book). This build books nothing anywhere else.
        account: String,
        /// The name the ticket's user goes by (set with its Change name button).
        trader: Option<String>,
    }

    /// Poll the relay for the trade ticket's orders, every second, on a thread. Orders queued
    /// before the desk started are skipped, never replayed.
    fn poll_orders(tx: mpsc::Sender<ManualOrder>) {
        std::thread::spawn(move || {
            let mut after: Option<u64> = None;
            loop {
                let url = format!("{}/paper/orders?after={}", relay_url(), after.unwrap_or(0));
                if let Ok(res) = ehttp::fetch_blocking(&ehttp::Request::get(url)) {
                    if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&res.bytes) {
                        let last = v.get("last_seq").and_then(|x| x.as_u64()).unwrap_or(0);
                        if after.is_some() {
                            for o in v.get("orders").and_then(|x| x.as_array()).into_iter().flatten() {
                                let f = |k: &str, d: f64| o.get(k).and_then(|x| x.as_f64()).unwrap_or(d);
                                let _ = tx.send(ManualOrder {
                                    seq: o.get("seq").and_then(|x| x.as_u64()).unwrap_or(0),
                                    action: o.get("action").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                                    symbol: o.get("symbol").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                                    stop_atr: f("stop_atr", 1.0).clamp(0.25, 10.0),
                                    target_r: f("target_r", 2.0).clamp(0.5, 20.0),
                                    placed_ms: o.get("placed_ms").and_then(|x| x.as_i64()).unwrap_or(0),
                                    account: o.get("account").and_then(|x| x.as_str()).unwrap_or("paper").to_string(),
                                    trader: o.get("trader").and_then(|x| x.as_str()).map(str::to_string),
                                });
                            }
                        }
                        after = Some(last.max(after.unwrap_or(0)));
                    }
                }
                std::thread::sleep(Duration::from_secs(1));
            }
        });
    }

    /// ATR(14) of the last 15m bars, in price.
    fn atr15(bars: &[Bar]) -> Option<f64> {
        let n = bars.len();
        if n < 15 {
            return None;
        }
        Some(bars[n - 14..].iter().zip(&bars[n - 15..n - 1]).map(|(b, p)| b.h.max(p.c) - b.l.min(p.c)).sum::<f64>() / 14.0)
    }

    /// The trader the trade ticket trades as. The ticket's Change name button sets it (an order with
    /// action "name"), and it is kept in `<log dir>/ticket_trader.txt` across restarts; before
    /// that, `OPENCRY_TRADER_NAME`, else the author's name. The seat's id stays "manual", so
    /// renaming never splits its trade log.
    const MANUAL_ID: &str = "manual";
    static MANUAL_NAME: std::sync::Mutex<Option<(String, Option<PathBuf>)>> = std::sync::Mutex::new(None);

    fn init_manual_name(log_dir: &std::path::Path) {
        let file = log_dir.join("ticket_trader.txt");
        let saved = fs::read_to_string(&file).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        let name = opencry_visual::ticket::trader_name_from(saved.as_deref().or(std::env::var("OPENCRY_TRADER_NAME").ok().as_deref()));
        *MANUAL_NAME.lock().unwrap_or_else(|e| e.into_inner()) = Some((name, Some(file)));
    }

    fn manual_name() -> String {
        match &*MANUAL_NAME.lock().unwrap_or_else(|e| e.into_inner()) {
            Some((n, _)) => n.clone(),
            None => opencry_visual::ticket::trader_name_from(std::env::var("OPENCRY_TRADER_NAME").ok().as_deref()),
        }
    }

    /// Rename the ticket's trader; returns the new name if it changed.
    fn set_manual_name(raw: &str) -> Option<String> {
        let name = opencry_visual::ticket::trader_name_from(Some(raw));
        let mut g = MANUAL_NAME.lock().unwrap_or_else(|e| e.into_inner());
        let file = g.as_ref().and_then(|(_, f)| f.clone());
        if g.as_ref().is_some_and(|(n, _)| *n == name) {
            return None;
        }
        if let Some(f) = &file {
            let _ = fs::write(f, format!("{name}\n"));
        }
        *g = Some((name.clone(), file));
        Some(name)
    }

    /// One holder's strategy, asked at the end of one of its bars.
    struct HolderAnswer {
        seat: u16,
        id: String,
        inst: Instrument,
        asked_ms: i64,
        /// The finished bar's open (ms), its position and one R in price.
        g: i64,
        pos: i8,
        unit: f64,
        error: Option<String>,
    }

    /// One adapter's choice for the day, with the scores it chose from.
    struct AdaptAnswer {
        seat: u16,
        day: i64,
        scores: Vec<(usize, usize, f64)>,
        code_pick: Option<usize>,
        jev: Option<Vec<f32>>,
        error: Option<String>,
    }

    /// One twin's question at one genome entry, and Jev's answer.
    struct TwinAnswer {
        twin: u16,
        genome_id: String,
        inst: Instrument,
        decided_ms: i64,
        side: opencry_visual::pit_engine::Side,
        stop: f64,
        why: String,
        p: Option<f32>,
        model: Option<String>,
        error: Option<String>,
    }

    /// Ask Jev a twin's question through the relay, on a thread of its own: a genome entry is a
    /// few a day, so no pool. Retries the relay's 429 (its one-call gate is shared with the
    /// pit reads) for up to a minute.
    fn ask_twin(tx: mpsc::Sender<TwinAnswer>, mut a: TwinAnswer, state: String, question: String) {
        std::thread::spawn(move || {
            let body = serde_json::json!({"state": state, "questions": {"twin_gate": {"type": "noul", "instructions": question}}});
            let started = Instant::now();
            loop {
                let mut req = ehttp::Request::post(format!("{}/jev", relay_url()), serde_json::to_vec(&body).unwrap_or_default());
                req.headers = ehttp::Headers::new(&[("Accept", "application/json"), ("Content-Type", "application/json")]);
                match ehttp::fetch_blocking(&req) {
                    Ok(res) if res.status == 429 && started.elapsed() < Duration::from_secs(60) => {
                        let wait = serde_json::from_slice::<serde_json::Value>(&res.bytes).ok().and_then(|v| v.get("retry_after_ms").and_then(|x| x.as_u64())).unwrap_or(2000);
                        std::thread::sleep(Duration::from_millis(wait + 200));
                    }
                    Ok(res) if res.ok => {
                        let v: serde_json::Value = serde_json::from_slice(&res.bytes).unwrap_or_default();
                        a.p = v.pointer("/answers/twin_gate/noul").and_then(|x| x.as_f64()).map(|x| x as f32);
                        a.model = v.get("model").and_then(|m| m.as_str()).map(str::to_string);
                        if a.p.is_none() {
                            a.error = Some(format!("no answer in {}", String::from_utf8_lossy(&res.bytes).chars().take(160).collect::<String>()));
                        }
                        break;
                    }
                    Ok(res) => {
                        a.error = Some(format!("relay HTTP {}", res.status));
                        break;
                    }
                    Err(e) => {
                        a.error = Some(format!("relay: {e}"));
                        break;
                    }
                }
            }
            let _ = tx.send(a);
        });
    }

    impl GenomePit {
        fn new(path: PathBuf, wall_ms: i64, jev_log: PathBuf) -> Self {
            let n = Instrument::ALL.len();
            let (answers_tx, answers_rx) = mpsc::channel();
            let (holder_tx, holder_rx) = mpsc::channel();
            let (adapt_tx, adapt_rx) = mpsc::channel();
            let (manual_tx, manual_rx) = mpsc::channel();
            if !cfg!(test) {
                poll_orders(manual_tx.clone());
            }
            let dir = jev_log.parent().map(|d| d.to_path_buf()).unwrap_or_default();
            GenomePit {
                path,
                next_reload: 0.0,
                seated: HashMap::new(),
                retiring: Default::default(),
                bars: vec![Vec::new(); n],
                bucket: vec![wall_ms.div_euclid(BAR_MS); n],
                last: None,
                twins: HashMap::new(),
                answers_tx,
                answers_rx,
                ready: Vec::new(),
                jev_log,
                holder_state: HashMap::new(),
                holder_asked: Default::default(),
                holder_tx,
                holder_rx,
                holder_ready: Vec::new(),
                holder_file: dir.join("holders_state.json"),
                restored: false,
                adapt_day: HashMap::new(),
                adapt_tx,
                adapt_rx,
                adapt_log: dir.join("adapt.jsonl"),
                manual_seat: None,
                manual_tx,
                manual_rx,
                manual_ready: Vec::new(),
                manual_last: None,
            }
        }

        /// 15m history from the tape, once at start and after an outage. Never the bridge.
        fn backfill(&mut self, symbols: &[Instrument], wall_ms: i64) {
            for inst in symbols {
                if genome::typical_spread_ticks(*inst) <= 0.0 || *inst == Instrument::Bitcoin {
                    continue;
                }
                // The tape keeps about 200 15m bars, too few for a 400-bar trend filter, so the
                // first fill comes from edge-lab's cache on disk (the backtest's own history,
                // no API call), and the tape tops it up from where the cache stops.
                if self.bars[*inst as usize].is_empty() {
                    let path = format!("{}/ctrader_{}_SB_15m.json", edge_cache(), tape_symbol(*inst));
                    if let Ok(rows) = fs::read_to_string(&path).map_err(|e| e.to_string()).and_then(|t| serde_json::from_str::<Vec<Vec<f64>>>(&t).map_err(|e| e.to_string())) {
                        let mut bars: Vec<Bar> = rows.iter().filter(|r| r.len() >= 5).map(|r| Bar { t: r[0] as i64, o: r[1], h: r[2], l: r[3], c: r[4] }).filter(|b| b.t + BAR_MS <= wall_ms).collect();
                        bars.sort_by_key(|b| b.t);
                        bars.dedup_by_key(|b| b.t);
                        let n = bars.len();
                        self.bars[*inst as usize] = bars.split_off(n.saturating_sub(GENOME_BARS_KEEP));
                    }
                }
                let url = format!("{}/bars?symbol={}&timeframe=15m&range=-21d", tape_url(), tape_symbol(*inst));
                match ehttp::fetch_blocking(&ehttp::Request::get(url)) {
                    Ok(res) if res.ok => {
                        let mut bars: Vec<Bar> = parse_backfill(res.text().unwrap_or(""))
                            .into_iter()
                            .filter(|c| c.time_ms + BAR_MS <= wall_ms)
                            .map(|c| Bar { t: c.time_ms, o: c.open as f64, h: c.high as f64, l: c.low as f64, c: c.close as f64 })
                            .collect();
                        bars.sort_by_key(|b| b.t);
                        bars.dedup_by_key(|b| b.t);
                        let keep = &mut self.bars[*inst as usize];
                        let newest = keep.last().map(|b| b.t).unwrap_or(i64::MIN);
                        if keep.is_empty() {
                            *keep = bars;
                        } else {
                            keep.extend(bars.into_iter().filter(|b| b.t > newest));
                        }
                        log_line(&format!("backfilled {} 15m from the tape ({} bars)", tape_symbol(*inst), keep.len()));
                    }
                    Ok(res) => log_line(&format!("15m backfill {} failed: HTTP {}", tape_symbol(*inst), res.status)),
                    Err(e) => log_line(&format!("15m backfill {} failed: {e}", tape_symbol(*inst))),
                }
                std::thread::sleep(Duration::from_millis(500));
            }
        }

        /// Seat new genomes, retire the ones that left the file (once they are flat).
        fn reload(&mut self, book: &mut PaperBook, now: f64) {
            if now < self.next_reload {
                return;
            }
            self.next_reload = now + PIT_RELOAD_SECS;
            let Ok(text) = fs::read_to_string(&self.path) else { return };
            let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else {
                log_line(&format!("pit: {} is not JSON; keeping the seats as they are", self.path.display()));
                return;
            };
            self.last = Some(v.clone());
            let mut want = std::collections::HashSet::new();
            let show = v.get("show").and_then(|s| s.as_array()).into_iter().flatten();
            for seat in v.get("seats").and_then(|s| s.as_array()).into_iter().flatten().chain(show) {
                let Some(mut g) = seat.get("genome").and_then(|g| serde_json::from_value::<Genome>(g.clone()).ok()) else { continue };
                g.jev = None; // the genome never asks; its twin does
                let with_twin = seat.get("twin").and_then(|t| t.as_bool()).unwrap_or(false);
                let seat_one = |g: &Genome, book: &mut PaperBook, seated: &mut HashMap<String, u16>| -> u16 {
                    if let Some(i) = seated.get(&g.id) {
                        return *i;
                    }
                    let idx = genome::register(g);
                    seated.insert(g.id.clone(), idx);
                    book.add_trader(Archetype::Genome(idx));
                    log_line(&format!("pit: seated {} ({}): {}", g.name, g.id, g.describe()));
                    idx
                };
                want.insert(g.id.clone());
                let base = seat_one(&g, book, &mut self.seated);
                if with_twin {
                    let t = genome::twin_of(&g);
                    want.insert(t.id.clone());
                    let twin = seat_one(&t, book, &mut self.seated);
                    self.twins.insert(base, twin);
                }
            }
            for (id, idx) in &self.seated {
                if !want.contains(id) && self.retiring.insert(*idx) {
                    log_line(&format!("pit: {} ({id}) retires once flat", genome::seat_name(*idx)));
                }
            }
            if !self.restored {
                self.restored = true;
                self.restore_holders(book, now);
            }
            if self.manual_seat.is_none() {
                let g = Genome {
                    id: MANUAL_ID.into(),
                    name: manual_name(),
                    signal: genome::Signal::Run { k: 3 },
                    mode: genome::Mode::Follow,
                    filters: vec![],
                    window: genome::Window::All,
                    exit: genome::Exit { stop_atr: 1.0, target_r: 2.0, max_bars: 0 },
                    symbols: vec![],
                    jev: None,
                    parent: None,
                    floor: Some("manual".into()),
                    holds: false,
                    tactics: vec![],
                    adapt: None,
                };
                let idx = genome::register(&g);
                book.add_trader(Archetype::Genome(idx));
                self.manual_seat = Some(idx);
            }
            self.save_holders(book);
            self.sweep(book);
        }

        /// Holder state and open positions from the last run.
        fn restore_holders(&mut self, book: &mut PaperBook, now: f64) {
            let Ok(v) = fs::read_to_string(&self.holder_file).map_err(|e| e.to_string()).and_then(|t| serde_json::from_str::<serde_json::Value>(&t).map_err(|e| e.to_string())) else { return };
            for (k, x) in v.get("state").and_then(|s| s.as_object()).into_iter().flatten() {
                if let (Some(g), Some(p)) = (x.get(0).and_then(|g| g.as_i64()), x.get(1).and_then(|p| p.as_i64())) {
                    self.holder_state.insert(k.clone(), (g, p as i8));
                }
            }
            let mut n = 0;
            for o in v.get("open").and_then(|s| s.as_array()).into_iter().flatten() {
                let (Some(id), Some(sym)) = (o.get("id").and_then(|x| x.as_str()), o.get("symbol").and_then(|x| x.as_str())) else { continue };
                let (Some(&idx), Some(inst)) = (self.seated.get(id), genome::instrument_of(sym)) else { continue };
                let Some(t) = book.traders.iter().position(|tr| tr.archetype == Archetype::Genome(idx)) else { continue };
                if book.traders[t].position.is_some() {
                    continue;
                }
                let f = |k: &str| o.get(k).and_then(|x| x.as_f64()).unwrap_or(0.0);
                let side = if o.get("side").and_then(|x| x.as_str()) == Some("buy") { opencry_visual::pit_engine::Side::Buy } else { opencry_visual::pit_engine::Side::Sell };
                book.traders[t].position = Some(opencry_visual::paper::Position {
                    inst,
                    side,
                    entry: f("entry") as f32,
                    stop: f("stop") as f32,
                    target: None,
                    trail: None,
                    opened_ms: o.get("opened_ms").and_then(|x| x.as_i64()).unwrap_or(0),
                    opened_at: now,
                    time_stop_secs: f64::INFINITY,
                    best: f("entry") as f32,
                    stop_ticks: f("stop_ticks") as f32,
                    spread_ticks_in: f("spread_ticks_in") as f32,
                    why: o.get("why").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                    observation_hash: String::new(),
                    read_source: None,
                    overnight: true,
                });
                n += 1;
            }
            log_line(&format!("pit: holders restored: {} states, {n} open positions", self.holder_state.len()));
        }

        fn save_holders(&self, book: &PaperBook) {
            if !self.restored {
                return;
            }
            let open: Vec<serde_json::Value> = book
                .traders
                .iter()
                .filter_map(|tr| {
                    let Archetype::Genome(i) = tr.archetype else { return None };
                    let p = tr.position.as_ref().filter(|p| p.overnight)?;
                    Some(serde_json::json!({"id": genome::seat_id(i)?, "symbol": tape_symbol(p.inst), "side": side_str(p.side), "entry": p.entry, "stop": p.stop,
                        "stop_ticks": p.stop_ticks, "opened_ms": p.opened_ms, "spread_ticks_in": p.spread_ticks_in, "why": p.why}))
                })
                .collect();
            let state: serde_json::Map<String, serde_json::Value> = self.holder_state.iter().map(|(k, (g, p))| (k.clone(), serde_json::json!([g, p]))).collect();
            let tmp = self.holder_file.with_extension("json.tmp");
            if fs::write(&tmp, serde_json::json!({"state": state, "open": open}).to_string()).is_ok() {
                let _ = fs::rename(&tmp, &self.holder_file);
            }
        }

        /// Jev's answers for twins: log every one; a yes for this pit enters the twin at this
        /// tick's touch if it is flat and not retiring. Answers for another pit wait for its tick;
        /// any older than 90 s are logged and dropped (the moment has gone).
        fn take_answers(&mut self, book: &mut PaperBook, v: &View) {
            while let Ok(a) = self.answers_rx.try_recv() {
                append_jsonl(&self.jev_log, &serde_json::json!({
                    "genome": a.genome_id, "twin": genome::seat_id(a.twin), "symbol": a.inst.symbol(), "decided_ms": a.decided_ms,
                    "answered_ms": v.wall_ms, "side": side_str(a.side), "p": a.p, "model": a.model, "error": a.error, "source": SOURCE,
                }));
                log_line(&format!("pit: Jev for {} on {}: {}", genome::seat_name(a.twin), a.inst.symbol(), a.p.map(|p| format!("{:.0}%", p * 100.0)).unwrap_or_else(|| a.error.clone().unwrap_or_default())));
                self.ready.push(a);
            }
            let ready = std::mem::take(&mut self.ready);
            for a in ready {
                if v.wall_ms - a.decided_ms > 90_000 {
                    continue;
                }
                if a.inst != v.inst {
                    self.ready.push(a);
                    continue;
                }
                if a.p.is_none_or(|p| p < 0.5) || self.retiring.contains(&a.twin) {
                    continue;
                }
                let Some(t) = book.traders.iter().position(|tr| tr.archetype == Archetype::Genome(a.twin)) else { continue };
                if book.traders[t].position.is_some() {
                    continue;
                }
                let Some(g) = genome::seat(a.twin).and_then(|s| s.genome) else { continue };
                let tick = v.tick as f64;
                let (d, reach) = genome::stop_and_target(&g, a.stop, v.spread_ticks as f64 * tick, tick);
                let entry = if a.side.is_buy() { v.ask as f64 } else { v.bid as f64 };
                let e = Entry {
                    side: a.side,
                    stop_ticks: (d / tick) as f32,
                    target: Some((if a.side.is_buy() { entry + reach } else { entry - reach }) as f32),
                    trail: None,
                    // Out when the genome's own time stop would take it: the clock started at the decision.
                    time_stop_secs: (g.exit.max_bars as f64 * BAR_MS as f64 - (v.wall_ms - a.decided_ms) as f64) / 1000.0,
                    cooldown_secs: 0.0,
                    why: format!("{} | Jev yes {:.0}%", a.why, a.p.unwrap_or(0.0) * 100.0),
                };
                if let Err(e) = book.enter_genome(t, v, e, false) {
                    log_line(&format!("pit: {} passed on {}: {e}", genome::seat_name(a.twin), a.inst.symbol()));
                }
            }
        }

        /// At a 15m close on this pit: ask the strategy of every holder of this pit whose own bar has
        /// just finished (the 15m bar closed it, or a gap means it ended earlier), and of any
        /// not asked since start. A retiring holder in a position closes here.
        fn holders_at_close(&mut self, book: &mut PaperBook, v: &View, bar_t: i64, prev_t: Option<i64>) {
            let sym = tape_symbol(v.inst);
            let mut jobs = Vec::new();
            for t in 0..book.traders.len() {
                let Archetype::Genome(idx) = book.traders[t].archetype else { continue };
                let Some(g) = genome::seat(idx).and_then(|s| s.genome) else { continue };
                if !g.holds || g.symbols.first() != Some(&sym) {
                    continue;
                }
                if self.retiring.contains(&idx) {
                    if book.close_now(t, v, false) {
                        log_line(&format!("pit: {} closed on {sym}: retiring", g.name));
                    }
                    continue;
                }
                let genome::Signal::EdgeLab { key, tf } = &g.signal else { continue };
                let tf_ms = *tf as i64 * 60_000;
                let closes = (bar_t + BAR_MS).rem_euclid(tf_ms) == 0;
                let gap = prev_t.is_some_and(|p| p.div_euclid(tf_ms) != bar_t.div_euclid(tf_ms) && (p + BAR_MS).rem_euclid(tf_ms) != 0);
                let k = format!("{}@{sym}", g.id);
                let first = self.holder_asked.insert(k);
                if closes || gap || first {
                    jobs.push((idx, g.id.clone(), key.clone(), *tf, v.inst));
                }
            }
            if !jobs.is_empty() {
                ask_holders(self.holder_tx.clone(), self.path.clone(), jobs, self.bars[v.inst as usize].clone(), v.wall_ms);
            }
        }

        /// Holders' answers, acted on at a tick of their own pit: a change in the strategy's
        /// position closes what the holder holds and opens the new side. The first answer since
        /// start (with no saved state) only records where the strategy is: a holder joins at the
        /// strategy's next change, as in the backtest. A rollover-wide spread waits.
        fn take_holder_answers(&mut self, book: &mut PaperBook, v: &View) {
            while let Ok(batch) = self.holder_rx.try_recv() {
                self.holder_ready.extend(batch);
            }
            let ready = std::mem::take(&mut self.holder_ready);
            let mut changed = false;
            for a in ready {
                if v.wall_ms - a.asked_ms > 4 * 3_600_000 {
                    log_line(&format!("pit: holder {} on {}: answer dropped, over 4 h old", a.id, a.inst.symbol()));
                    continue;
                }
                if a.inst != v.inst {
                    self.holder_ready.push(a);
                    continue;
                }
                let k = format!("{}@{}", a.id, tape_symbol(a.inst));
                if let Some(e) = &a.error {
                    log_line(&format!("pit: holder {k}: {e}"));
                    self.holder_asked.remove(&k);
                    continue;
                }
                let Some(&(g0, p0)) = self.holder_state.get(&k) else {
                    log_line(&format!("pit: holder {k}: strategy at {} (bar {}); joins at its next change", a.pos, a.g));
                    self.holder_state.insert(k, (a.g, a.pos));
                    changed = true;
                    continue;
                };
                if a.g <= g0 {
                    continue;
                }
                if a.pos == p0 {
                    self.holder_state.insert(k, (a.g, a.pos));
                    changed = true;
                    continue;
                }
                if v.spread_ticks > 3.0 * genome::typical_spread_ticks(v.inst) as f32 {
                    self.holder_ready.push(a);
                    continue;
                }
                let Some(t) = book.traders.iter().position(|tr| tr.archetype == Archetype::Genome(a.seat)) else { continue };
                let name = genome::seat_name(a.seat);
                book.close_now(t, v, false);
                if a.pos != 0 && !self.retiring.contains(&a.seat) {
                    let side = if a.pos > 0 { opencry_visual::pit_engine::Side::Buy } else { opencry_visual::pit_engine::Side::Sell };
                    let unit = (a.unit / v.tick as f64) as f32;
                    let key = genome::seat(a.seat).and_then(|s| s.genome).map(|g| g.signal.edge_lab_id().unwrap_or_default()).unwrap_or_default();
                    let why = format!("edge-lab {key} turned {} at the close of its bar", if a.pos > 0 { "long" } else { "short" });
                    if let Err(e) = book.enter_holder(t, v, side, unit, unit * genome::HOLD_SAFETY_R as f32, why) {
                        log_line(&format!("pit: {name} passed on {}: {e}", a.inst.symbol()));
                    }
                }
                self.holder_state.insert(k, (a.g, a.pos));
                changed = true;
            }
            if changed {
                self.save_holders(book);
            }
        }

        /// Once a UTC day, at the first 15m close from `ADAPT_AT`, every adapter scores its tactics
        /// on the desk's bars; a code adapter takes the best at once, a Jev adapter asks Jev.
        fn adapters_at_close(&mut self, book: &PaperBook, wall_ms: i64) {
            let day = wall_ms.div_euclid(86_400_000);
            if genome::minute_of_day(wall_ms) < genome::ADAPT_AT {
                return;
            }
            for tr in &book.traders {
                let Archetype::Genome(idx) = tr.archetype else { continue };
                if self.adapt_day.get(&idx) == Some(&day) || self.retiring.contains(&idx) {
                    continue;
                }
                let Some(g) = genome::seat(idx).and_then(|s| s.genome) else { continue };
                let Some(mode) = g.adapt else { continue };
                if g.tactics.is_empty() {
                    continue;
                }
                self.adapt_day.insert(idx, day);
                let (scores, code_pick) = score_tactics(&g, &self.bars, wall_ms);
                let a = AdaptAnswer { seat: idx, day, scores: scores.clone(), code_pick, jev: None, error: None };
                match mode {
                    genome::Adapt::Code => {
                        let _ = self.adapt_tx.send(a);
                    }
                    genome::Adapt::Jev => ask_adapter(self.adapt_tx.clone(), a, adapt_state(&g, &self.bars, &scores), g.tactics.len()),
                }
            }
        }

        /// Adapters' choices: switch the seat to the chosen tactic and log the choice.
        fn take_adapt_answers(&mut self, book: &mut PaperBook, v: &View) {
            while let Ok(a) = self.adapt_rx.try_recv() {
                let Some(g) = genome::seat(a.seat).and_then(|s| s.genome) else { continue };
                let from = genome::tactic_in_use(&g);
                let jev_pick = a.jev.as_ref().and_then(|ps| ps.iter().enumerate().max_by(|x, y| x.1.partial_cmp(y.1).unwrap_or(std::cmp::Ordering::Equal)).map(|(k, _)| k));
                let pick = match g.adapt {
                    Some(genome::Adapt::Jev) => jev_pick,
                    _ => a.code_pick,
                };
                if let Some(k) = pick {
                    if Some(k) != from && genome::set_tactic(a.seat, k) {
                        if let Some(t) = book.traders.iter_mut().find(|tr| tr.archetype == Archetype::Genome(a.seat)) {
                            t.note(format!("switched to tactic {} of {}: {}", k + 1, g.tactics.len(), g.tactics[k].rule_text()));
                        }
                        log_line(&format!("pit: {} switches to tactic {}: {}", g.name, k + 1, g.tactics[k].rule_text()));
                    }
                }
                append_jsonl(&self.adapt_log, &serde_json::json!({
                    "adapter": g.id, "name": g.name, "mode": g.adapt, "day": a.day, "decided_ms": v.wall_ms,
                    "scores": a.scores.iter().map(|(k, n, m)| serde_json::json!({"tactic": k + 1, "n": n, "mean": (m * 1e4).round() / 1e4})).collect::<Vec<_>>(),
                    "code_pick": a.code_pick.map(|k| k + 1), "jev": a.jev, "from": from.map(|k| k + 1), "to": pick.map(|k| k + 1), "error": a.error, "source": SOURCE,
                }));
            }
        }

        /// The trade ticket's orders, filled at this pit's touch as paper trades for its trader.
        /// An order older than 30 s is dropped (the moment has gone).
        fn take_manual_orders(&mut self, book: &mut PaperBook, v: &View) {
            while let Ok(o) = self.manual_rx.try_recv() {
                self.manual_ready.push(o);
            }
            let Some(seat) = self.manual_seat else { return };
            let ready = std::mem::take(&mut self.manual_ready);
            let sym = tape_symbol(v.inst);
            for o in ready {
                // The ticket's user named themselves: rename the seat (any order may carry it).
                if let Some(name) = o.trader.as_deref().and_then(set_manual_name) {
                    opencry_visual::genome::rename(seat, &name);
                    let note = format!("the ticket now trades as {name}");
                    log_line(&format!("ticket: {note}"));
                    self.manual_last = Some(note);
                }
                if o.action == "name" {
                    continue;
                }
                if v.wall_ms - o.placed_ms > 30_000 {
                    self.manual_last = Some(format!("#{} {} {}: not filled, no price from that pit within 30 s", o.seq, o.action, o.symbol));
                    continue;
                }
                if o.symbol != sym {
                    self.manual_ready.push(o);
                    continue;
                }
                if o.account != "paper" {
                    let note = format!("#{} {} {}: only paper orders exist in this build", o.seq, o.action, o.symbol);
                    log_line(&format!("ticket: {note}"));
                    self.manual_last = Some(note);
                    continue;
                }
                let Some(t) = book.traders.iter().position(|tr| tr.archetype == Archetype::Genome(seat)) else { continue };
                let px = |x: f32| opencry_visual::pit::format_price_with_commas(x as f64, opencry_visual::pit::instrument_decimals(v.inst));
                let note = match o.action.as_str() {
                    "close" => match book.traders[t].position.clone().filter(|p| p.inst == v.inst) {
                        Some(p) => {
                            book.close_at_touch(t, v, opencry_visual::paper::ExitReason::Manual, false);
                            let r = book.traders[t].trades.last().map(|x| x.pnl_r).unwrap_or(0.0);
                            format!("#{} closed {} {} from {} at {}: {:+.2}R", o.seq, side_str(p.side), sym, px(p.entry), px(if p.side.is_buy() { v.bid } else { v.ask }), r)
                        }
                        None => format!("#{} close {sym}: nothing open there", o.seq),
                    },
                    "buy" | "sell" => {
                        if let Some(p) = &book.traders[t].position {
                            format!("#{} {} {sym}: already {} {}; close it first", o.seq, o.action, side_str(p.side), p.inst.symbol())
                        } else if let Some(atr) = atr15(&self.bars[v.inst as usize]) {
                            let side = if o.action == "buy" { opencry_visual::pit_engine::Side::Buy } else { opencry_visual::pit_engine::Side::Sell };
                            let tick = v.tick as f64;
                            let d = o.stop_atr * atr;
                            let entry = if side.is_buy() { v.ask as f64 } else { v.bid as f64 };
                            let e = Entry {
                                side,
                                stop_ticks: (d / tick) as f32,
                                target: Some((if side.is_buy() { entry + o.target_r * d } else { entry - o.target_r * d }) as f32),
                                trail: None,
                                time_stop_secs: f64::INFINITY,
                                cooldown_secs: 0.0,
                                why: format!("by hand: stop {} ATR, target {} R", o.stop_atr, o.target_r),
                            };
                            match book.enter_genome(t, v, e, false) {
                                Ok(()) => {
                                    let p = book.traders[t].position.as_mut().expect("just entered");
                                    p.overnight = true; // yours until your stop, target or Close
                                    format!("#{} {} {sym} at {}, stop {}, target {}", o.seq, if side.is_buy() { "bought" } else { "sold" }, px(p.entry), px(p.stop), p.target.map(px).unwrap_or_default())
                                }
                                Err(e) => format!("#{} {} {sym}: refused, {e}", o.seq, o.action),
                            }
                        } else {
                            format!("#{} {} {sym}: no 15m bars yet for a stop", o.seq, o.action)
                        }
                    }
                    a => format!("#{} {a}: unknown action", o.seq),
                };
                log_line(&format!("ticket: {note}"));
                self.manual_last = Some(note);
            }
        }

        /// The board's view of the pit, from the file as last read.
        fn summary(&self) -> Option<DeskPit> {
            let v = self.last.as_ref()?;
            let f = |x: Option<&serde_json::Value>| x.and_then(|x| x.as_f64()).unwrap_or(0.0) as f32;
            let show = v.get("show").and_then(|s| s.as_array()).into_iter().flatten();
            let seats: Vec<DeskSeat> = v.get("seats").and_then(|s| s.as_array()).into_iter().flatten().chain(show).filter_map(|s| {
                let g = s.get("genome")?;
                let (bt, fw) = (s.get("backtest"), s.get("forward"));
                let parsed = serde_json::from_value::<Genome>(g.clone()).ok();
                // An adapter's rule is the tactic it is using now, which only the desk knows.
                let live = parsed.as_ref().and_then(|g| self.seated.get(&g.id)).map(|i| genome::seat_creed(*i).to_string());
                let creed = live.or_else(|| parsed.as_ref().map(|g| g.describe())).unwrap_or_default();
                let tag = parsed.as_ref().map(|g| genome::tag_of(g).to_string());
                let jev = s.get("twin").and_then(|t| t.as_bool()).unwrap_or(false).then(|| {
                    let tw = s.get("twin_split");
                    let num = |p: &str| f(tw.and_then(|t| t.pointer(p)));
                    DeskJevSplit {
                        yes_n: num("/yes/n") as u32,
                        yes_mean: num("/yes/mean"),
                        no_n: num("/no/n") as u32,
                        no_mean: num("/no/mean"),
                        t: num("/t"),
                        verdict: s.pointer("/twin_verdict/adds").and_then(|a| a.as_bool()).map(|a| if a { "adds something".into() } else { "adds nothing".into() }),
                    }
                });
                Some(DeskSeat {
                    id: g.get("id")?.as_str()?.to_string(),
                    name: g.get("name")?.as_str()?.to_string(),
                    creed,
                    seated_ms: s.get("seated_ms").and_then(|x| x.as_i64()).unwrap_or(0),
                    bt_is_mean: f(bt.and_then(|b| b.pointer("/is/mean"))),
                    bt_is_t: f(bt.and_then(|b| b.pointer("/is/t"))),
                    bt_ho_mean: f(bt.and_then(|b| b.pointer("/ho/mean"))),
                    bt_ho_t: f(bt.and_then(|b| b.pointer("/ho/t"))),
                    fwd_n: f(fw.and_then(|x| x.get("n"))) as u32,
                    fwd_mean: f(fw.and_then(|x| x.get("mean"))),
                    fwd_t: f(fw.and_then(|x| x.get("t"))),
                    graduated: s.get("graduated").is_some_and(|x| !x.is_null()),
                    jev,
                    twin_of: None,
                    floor: g.get("floor").and_then(|x| x.as_str()).map(str::to_string),
                    tag,
                })
            })
            .flat_map(|base| {
                // A twin gets a row of its own on the page, with its own forward record.
                let twin = base.jev.clone().map(|j| {
                    let tf = v.get("seats").and_then(|s| s.as_array()).and_then(|a| a.iter().find(|s| s.pointer("/genome/id").and_then(|x| x.as_str()) == Some(base.id.as_str()))).and_then(|s| s.pointer("/twin_split/twin_forward")).cloned();
                    let g = |k: &str| tf.as_ref().and_then(|t| t.get(k)).and_then(|x| x.as_f64()).unwrap_or(0.0) as f32;
                    DeskSeat {
                        id: format!("{}-J", base.id),
                        name: format!("{}-J", base.name),
                        creed: base.creed.clone(),
                        seated_ms: base.seated_ms,
                        fwd_n: g("n") as u32,
                        fwd_mean: g("mean"),
                        fwd_t: g("t"),
                        jev: Some(j),
                        twin_of: Some(base.name.clone()),
                        ..Default::default()
                    }
                });
                std::iter::once(base).chain(twin)
            })
            .collect();
            let mut seats = seats;
            if self.manual_seat.is_some() {
                seats.push(DeskSeat {
                    id: MANUAL_ID.into(),
                    name: manual_name(),
                    creed: "manual: your own paper trades from the trade ticket".into(),
                    floor: Some("manual".into()),
                    tag: Some("manual".into()),
                    ..Default::default()
                });
            }
            let retired = v.get("retired").and_then(|r| r.as_array()).cloned().unwrap_or_default();
            let recent_retired = retired.iter().rev().take(3).filter_map(|r| {
                let name = r.get("name")?.as_str()?;
                let why = r.get("why").and_then(|w| w.as_str()).map(str::to_string).or_else(|| r.get("forward").filter(|f| !f.is_null()).map(|f| format!("{} trades {:+.3}R", f.get("n").and_then(|x| x.as_u64()).unwrap_or(0), f.get("mean").and_then(|x| x.as_f64()).unwrap_or(0.0))));
                Some(format!("{name}: {}", why.unwrap_or_default()))
            }).collect();
            Some(DeskPit {
                n_tried: v.get("n_tried").and_then(|x| x.as_u64()).unwrap_or(0),
                seated_ever: v.get("seated_ever").and_then(|x| x.as_u64()).unwrap_or(0),
                retired: retired.len() as u64,
                updated_ms: v.get("updated_ms").and_then(|x| x.as_i64()).unwrap_or(0),
                seats,
                recent_retired,
            })
        }

        /// Take retiring genomes that are flat off the book. Between ticks only.
        fn sweep(&mut self, book: &mut PaperBook) {
            let retiring = &self.retiring;
            let before = book.traders.len();
            book.retain_traders(|t| !(matches!(t.archetype, Archetype::Genome(i) if retiring.contains(&i)) && t.position.is_none()));
            if book.traders.len() != before {
                log_line(&format!("pit: {} retired", before - book.traders.len()));
            }
        }

        /// On every tick of a pit: if a 15m bar has just closed, build it from the 1m bars
        /// and let every seated genome trading this pit decide at its close.
        fn on_tick(&mut self, book: &mut PaperBook, store: &CandleStore, v: &View) {
            self.take_answers(book, v);
            self.take_holder_answers(book, v);
            self.take_adapt_answers(book, v);
            self.take_manual_orders(book, v);
            let i = v.inst as usize;
            let bucket = v.wall_ms.div_euclid(BAR_MS);
            if bucket <= self.bucket[i] {
                return;
            }
            self.bucket[i] = bucket;
            let open = (bucket - 1) * BAR_MS;
            let ones: Vec<Candle> = store.series(v.inst, Timeframe::M1, FeedMode::Live).bars.iter().filter(|c| c.time_ms >= open && c.time_ms < open + BAR_MS).copied().collect();
            let (Some(first), Some(last)) = (ones.first(), ones.last()) else { return };
            let bar = Bar {
                t: open,
                o: first.open as f64,
                h: ones.iter().map(|c| c.high as f64).fold(f64::MIN, f64::max),
                l: ones.iter().map(|c| c.low as f64).fold(f64::MAX, f64::min),
                c: last.close as f64,
            };
            let bars = &mut self.bars[i];
            match bars.last() {
                Some(b) if b.t == bar.t => *bars.last_mut().unwrap() = bar,
                Some(b) if b.t > bar.t => return,
                _ => bars.push(bar),
            }
            if bars.len() > GENOME_BARS_KEEP {
                bars.drain(..bars.len() - GENOME_BARS_KEEP);
            }
            let prev_t = (bars.len() >= 2).then(|| bars[bars.len() - 2].t);
            let mut f = Features::new(bars.clone());
            let k = bars.len() - 1;
            let sym = tape_symbol(v.inst);
            self.holders_at_close(book, v, bar.t, prev_t);
            self.adapters_at_close(book, v.wall_ms);
            let n_genomes = book.traders.iter().filter(|t| matches!(t.archetype, Archetype::Genome(_))).count();
            log_line(&format!("15m {} {} o {} h {} l {} c {} ({} bars, {n_genomes} genomes)", sym, bar.t, bar.o, bar.h, bar.l, bar.c, k + 1));
            if v.spread_ticks > 3.0 * genome::typical_spread_ticks(v.inst) as f32 {
                return; // a rollover-wide spread: the backtest never paid one, so nobody enters
            }
            for t in 0..book.traders.len() {
                let Archetype::Genome(idx) = book.traders[t].archetype else { continue };
                if book.traders[t].position.is_some() || self.retiring.contains(&idx) {
                    continue;
                }
                let Some(seat) = genome::seat(idx) else { continue };
                let Some(g) = seat.genome.as_ref() else { continue };
                if g.jev.is_some() {
                    continue; // a twin never decides at the bar close: it waits for its genome's entry and Jev
                }
                if g.holds || g.floor.as_deref() == Some("manual") {
                    continue; // a holder acts on its strategy's own bars; the ticket's trader on orders
                }
                if !g.symbols.iter().any(|s| *s == sym) {
                    continue;
                }
                if let (genome::Signal::EdgeLab { key, tf }, Some(id)) = (&g.signal, g.signal.edge_lab_id()) {
                    if !f.has_entries(&id) {
                        let d = edge_lab_entry(&self.path, key, *tf, &self.bars[i]);
                        log_line(&format!("15m {sym} edge-lab {id}: {}", d.map_or("no answer".to_string(), |d| d.to_string())));
                        f.set_entries(&id, HashMap::from([(k, d.unwrap_or(0))]));
                    }
                }
                let Some((side, stop, why)) = genome::evaluate(g, &mut f, k) else { continue };
                let tick = v.tick as f64;
                let (d, reach) = genome::stop_and_target(g, stop, v.spread_ticks as f64 * tick, tick);
                let entry = if side.is_buy() { v.ask as f64 } else { v.bid as f64 };
                let target = if side.is_buy() { entry + reach } else { entry - reach };
                let e = Entry {
                    side,
                    stop_ticks: (d / tick) as f32,
                    target: Some(target as f32),
                    trail: None,
                    time_stop_secs: g.exit.max_bars as f64 * BAR_MS as f64 / 1000.0,
                    cooldown_secs: 0.0,
                    why,
                };
                match book.enter_genome(t, v, e, false) {
                    Err(e) => log_line(&format!("pit: {} passed on {sym}: {e}", seat.name)),
                    Ok(()) => {
                        if let Some(&twin) = self.twins.get(&idx) {
                            let q = genome::seat(twin).and_then(|s| s.genome.and_then(|g| g.jev)).unwrap_or_default();
                            let a = TwinAnswer { twin, genome_id: g.id.clone(), inst: v.inst, decided_ms: v.wall_ms, side, stop, why: book.traders[t].position.as_ref().map(|p| p.why.clone()).unwrap_or_default(), p: None, model: None, error: None };
                            ask_twin(self.answers_tx.clone(), a, genome::jev_state(&self.bars[i], side), q);
                        }
                    }
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Run
    // -----------------------------------------------------------------------

    pub fn run() -> Result<(), String> {
        if flag("--print-questions") {
            println!("{}", serde_json::to_string_pretty(&all_questions()).map_err(|e| e.to_string())?);
            return Ok(());
        }
        let args = parse_args()?;
        fs::create_dir_all(&args.log_dir).map_err(|e| format!("cannot create {}: {e}", args.log_dir.display()))?;
        init_manual_name(&args.log_dir);
        let trades_path = args.log_dir.join("paper_desk_trades.jsonl");
        let decisions_path = args.log_dir.join("paper_desk_decisions.jsonl");
        let relay = relay_url();
        let ws_url = feed_ws_url();
        let started = Instant::now();
        let started_ms = wall_clock_ms();
        let window_ms = (BOARD_WINDOW_HOURS * 3_600_000.0) as i64;
        log_line(&format!(
            "desk up: tape {ws_url}  relay {relay}  symbols {}  jev {} every {:.0}s  heartbeat {:.0}s  log {}",
            args.symbols.iter().map(|i| tape_symbol(*i)).collect::<Vec<_>>().join(","),
            args.jev.label(),
            args.jev_every,
            args.heartbeat_every,
            args.log_dir.display()
        ));

        // State: exactly what the app holds.
        let mut feed = LiveFeed::default();
        let mut store = CandleStore::default();
        let mut levels = LevelStore::default();
        let mut desk = JevDesk::with_model(Box::new(DeterministicModel));
        desk.set_session_start_ms(started_ms);
        let mut book = PaperBook::default();
        book.log_to_relay = false;
        book.show = false;
        book.reseed(args.seed);
        let mut history = load_history(&trades_path, started_ms, window_ms);
        if !history.is_empty() {
            log_line(&format!("{} closed trades in the last {BOARD_WINDOW_HOURS:.0}h reloaded from {}", history.len(), trades_path.display()));
        }

        if let Some(keep) = &args.keep {
            book.retain_traders(|t| keep.iter().any(|k| k == t.name()));
            log_line(&format!("keeping {} built-in traders: {}", book.traders.len(), book.traders.iter().map(|t| t.name()).collect::<Vec<_>>().join(", ")));
        }

        backfill_all(&mut store, &args.symbols, &args.backfill);
        let mut gpit = args.pit.clone().map(|p| GenomePit::new(p, wall_clock_ms(), args.log_dir.join("genome_jev.jsonl")));
        if let Some(g) = gpit.as_mut() {
            g.backfill(&args.symbols, wall_clock_ms());
        }

        let poster = Poster {
            relay: relay.clone(),
            enabled: args.relay_log,
            failures: Arc::new(AtomicU64::new(0)),
            posted: Arc::new(AtomicU64::new(0)),
            heartbeat_status: Arc::new(AtomicU64::new(0)),
        };
        let (job_tx, done_rx) = spawn_jev_worker(relay.clone(), desk.questions());
        let mut socket = Socket::new(ws_url);

        let clock = || started.elapsed().as_secs_f64();
        let n_pits = Instrument::ALL.len();
        let mut prev_bid: Vec<f32> = vec![0.0; n_pits];
        let mut seen: Vec<bool> = vec![false; n_pits];
        let mut next_read_at: Vec<f64> = vec![f64::INFINITY; n_pits];
        let mut queued: Vec<bool> = vec![false; n_pits];
        // Touch-triggered reads: a tracker per pit, the earliest next touch read, and the
        // follow-up waiting to be asked (the touch, when it is due, when it happened).
        let mut trackers: Vec<TouchTracker> = (0..n_pits).map(|_| TouchTracker::default()).collect();
        let mut touch_ok_at: Vec<f64> = vec![0.0; n_pits];
        let mut follow: Vec<Option<(Touch, f64, f64)>> = vec![None; n_pits];
        let (mut n_touches, mut n_touch_reads) = (0u64, 0u64);
        for (k, inst) in args.symbols.iter().enumerate() {
            // Spread the pits across the cadence so the relay sees a steady trickle.
            next_read_at[*inst as usize] = 2.0 + k as f64 * args.jev_every / args.symbols.len() as f64;
        }
        let fresh_secs = READ_FRESH_SECS.max(args.jev_every + 5.0);
        let mut next_heartbeat = 0.0;
        let mut next_status = STATUS_EVERY_SECS;
        let mut heartbeat_404_said = false;
        let (mut n_ticks, mut n_depth, mut n_reads, mut n_calls, mut n_failures) = (0u64, 0u64, 0u64, 0u64, 0u64);

        loop {
            let now = clock();
            let wall_ms = wall_clock_ms();
            socket.ensure_connected(now);
            if let Some(g) = gpit.as_mut() {
                g.reload(&mut book, now);
            }

            // The tape.
            let events = socket.drain();
            let idle = events.is_empty();
            for ev in events {
                match ev {
                    ewebsock::WsEvent::Opened => {
                        feed.note_message(now);
                        socket.backoff = RECONNECT_MIN_SECS;
                        socket.reconnects += 1;
                        let outage = socket.down_since.take().map(|t| now - t).unwrap_or(0.0);
                        log_line(&format!("socket open {} (outage {outage:.0}s)", socket.url));
                        if outage > REBACKFILL_AFTER_SECS {
                            backfill_all(&mut store, &args.symbols, &args.backfill);
                            if let Some(g) = gpit.as_mut() {
                                g.backfill(&args.symbols, wall_ms);
                            }
                        }
                    }
                    ewebsock::WsEvent::Message(ewebsock::WsMessage::Text(text)) => {
                        feed.note_message(now);
                        match parse_tape(&text) {
                            Some(TapeMessage::Tick(inst, tick)) => {
                                if !args.symbols.contains(&inst) {
                                    continue;
                                }
                                n_ticks += 1;
                                let i = inst as usize;
                                let prev = feed.quote(inst).map(|q| q.price);
                                feed.ingest_tick(inst, &tick, now);
                                let tick_sz = tick_size(inst);
                                let d_ticks = prev.map(|p| ((tick.bid - p) / tick_sz as f64) as f32).unwrap_or(0.0);
                                store.apply_live_tick(inst, tick.t, tick.bid as f32, d_ticks);

                                // Levels from the 1m bars, like the app.
                                let series = store.series(inst, Timeframe::M1, FeedMode::Live);
                                let n = series.bars.len();
                                let bars60: Vec<Candle> = series.bars.iter().skip(n.saturating_sub(BOARD_BARS)).copied().collect();
                                let rev = series.revision;
                                let bar_open_ms = series.bars.back().map(|b| b.time_ms).unwrap_or(0);
                                levels.refresh_trading(inst, &bars60, rev, now);

                                // A touch of a level is what Jev is asked about, the moment it happens.
                                if args.touch_reads && args.jev != JevMode::None {
                                    let unit = unit_ticks(&bars60, tick_sz);
                                    let touches = trackers[i].step(levels.for_trading(inst), Step::tick(tick.bid as f32), wall_ms, bar_open_ms, tick_sz, unit);
                                    if let Some(t) = touches.into_iter().next() {
                                        n_touches += 1;
                                        if now >= touch_ok_at[i] {
                                            touch_ok_at[i] = now + args.touch_cooldown;
                                            follow[i] = (args.touch_follow > 0.0).then(|| (t.clone(), now + args.touch_follow, now));
                                            if !queued[i] {
                                                if let Some(mut obs) = desk.observe(inst, &store, Timeframe::M1, &feed, levels.for_trading(inst), now, wall_ms) {
                                                    obs.trigger = Some(trigger_for(&t, "touch", 0.0));
                                                    if dispatch(args.jev, &mut desk, &job_tx, &mut queued, &mut n_reads, inst, obs, now) {
                                                        n_touch_reads += 1;
                                                        next_read_at[i] = now + args.jev_every;
                                                        log_line(&format!("touch: {} {} {}..{} {} -> Jev asked", inst.symbol(), t.kind.tag(), t.lo, t.hi, t.side.label()));
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }

                                // The book.
                                let PitFeedStatus::Live { rate, .. } = feed.pit_status(inst, now) else { continue };
                                let q = feed.quote(inst).expect("quote after ingest");
                                let bars40: Vec<Candle> = bars60.iter().rev().take(40).rev().copied().collect();
                                let read = desk.read(inst).filter(|r| match args.jev {
                                    JevMode::None => false,
                                    JevMode::Rule => now - r.at <= fresh_secs,
                                    JevMode::Live => r.source == ReadSource::Jev && now - r.at <= fresh_secs,
                                });
                                let bid = q.price as f32;
                                let ask = q.ask as f32;
                                let view = View {
                                    inst,
                                    bid,
                                    ask,
                                    tick: tick_sz,
                                    spread_ticks: (q.spread / tick_sz as f64) as f32,
                                    now,
                                    wall_ms,
                                    bars: &bars40,
                                    flow: q.flow(now),
                                    rate,
                                    levels: levels.for_trading(inst),
                                    read,
                                    prev_bid: if seen[i] { prev_bid[i] } else { bid },
                                };
                                let before = Snapshot::of(&book);
                                book.step_pit(&view, false);
                                if let Some(g) = gpit.as_mut() {
                                    g.on_tick(&mut book, &store, &view);
                                }
                                prev_bid[i] = bid;
                                seen[i] = true;

                                // Records, same rows as the app plus `source`.
                                let (closed, opened) = before.diff(&book);
                                for (t, trade) in closed {
                                    let tj = trade_json(trade);
                                    append_jsonl(&trades_path, &tj);
                                    poster.log("paper_trades", &tj);
                                    let dj = decision_json(trade.trader, trade.archetype, trade.symbol, wall_ms, "exit", trade.side, trade.exit, bid, ask, &trade.why_out, None);
                                    append_jsonl(&decisions_path, &dj);
                                    poster.log("paper_decisions", &dj);
                                    history.push(closed_from_trade(trade));
                                    log_line(&format!(
                                        "{} ({}) {} {} closed on {}: {:+.2}R ({:+.0} ticks)  [{}]",
                                        trade.trader, book.traders[t].archetype.label(), trade.symbol, trade.side, trade.why_out, trade.pnl_r, trade.pnl_ticks, trade.why_in
                                    ));
                                }
                                for t in opened {
                                    let tr = &book.traders[t];
                                    let Some(p) = tr.position.as_ref() else { continue };
                                    let hash = (!p.observation_hash.is_empty()).then_some(p.observation_hash.as_str());
                                    let dj = decision_json(tr.name(), tr.archetype, p.inst.symbol(), wall_ms, "enter", side_str(p.side), p.entry, bid, ask, &p.why, hash);
                                    append_jsonl(&decisions_path, &dj);
                                    poster.log("paper_decisions", &dj);
                                    log_line(&format!(
                                        "{} ({}) {} {} at {}  stop {}  target {}  |  {}",
                                        tr.name(),
                                        tr.archetype.label(),
                                        p.inst.symbol(),
                                        side_str(p.side),
                                        p.entry,
                                        p.stop,
                                        p.target.map(|t| t.to_string()).unwrap_or_else(|| "trail".into()),
                                        p.why
                                    ));
                                }
                            }
                            Some(TapeMessage::Depth(inst, depth)) => {
                                n_depth += 1;
                                feed.ingest_depth(inst, &depth);
                            }
                            None => {}
                        }
                    }
                    ewebsock::WsEvent::Message(_) => feed.note_message(now),
                    ewebsock::WsEvent::Error(e) => {
                        socket.drop(now, &format!("error: {e}"));
                        feed.state = FeedState::Down;
                    }
                    ewebsock::WsEvent::Closed => {
                        socket.drop(now, "closed");
                        feed.state = FeedState::Down;
                    }
                }
            }
            feed.check_staleness(now);

            // Jev: answers back from the worker, then new reads on the schedule.
            while let Ok(done) = done_rx.try_recv() {
                let i = done.inst as usize;
                queued[i] = false;
                n_calls += done.calls as u64;
                match done.res {
                    Ok(mut r) => {
                        r.at = now;
                        r.trigger = done.trigger.clone();
                        log_line(&format!("Jev {}: {}", done.inst.symbol(), r.headline()));
                        desk.set_read(done.inst, r);
                        n_reads += 1;
                    }
                    Err(e) => {
                        n_failures += 1;
                        log_line(&format!("Jev {} failed: {}", done.inst.symbol(), e.short()));
                    }
                }
            }
            if args.jev != JevMode::None {
                // Follow-ups: how is the reaction at the touched level developing?
                for inst in &args.symbols {
                    let i = *inst as usize;
                    let Some((_, due, _)) = follow[i] else { continue };
                    if now < due || queued[i] {
                        continue;
                    }
                    let (t, _, touched_at) = follow[i].take().unwrap();
                    if let Some(mut obs) = desk.observe(*inst, &store, Timeframe::M1, &feed, levels.for_trading(*inst), now, wall_ms) {
                        obs.trigger = Some(trigger_for(&t, "follow", now - touched_at));
                        if dispatch(args.jev, &mut desk, &job_tx, &mut queued, &mut n_reads, *inst, obs, now) {
                            n_touch_reads += 1;
                            next_read_at[i] = now + args.jev_every;
                        }
                    }
                }
                for inst in &args.symbols {
                    let i = *inst as usize;
                    if now < next_read_at[i] || queued[i] {
                        continue;
                    }
                    let Some(obs) = desk.observe(*inst, &store, Timeframe::M1, &feed, levels.for_trading(*inst), now, wall_ms) else {
                        next_read_at[i] = now + OBSERVE_RETRY_SECS;
                        continue;
                    };
                    next_read_at[i] = now + args.jev_every;
                    dispatch(args.jev, &mut desk, &job_tx, &mut queued, &mut n_reads, *inst, obs, now);
                }
            }

            // Heartbeat: the rolling board plus open positions.
            if now >= next_heartbeat {
                next_heartbeat = now + args.heartbeat_every;
                let hb_status = poster.heartbeat_status.load(Ordering::Relaxed);
                if hb_status == 404 && !heartbeat_404_said {
                    heartbeat_404_said = true;
                    log_line("relay answers 404 on /paper/heartbeat (older relay); trades still log, the board waits for a redeploy");
                } else if hb_status == 200 && heartbeat_404_said {
                    heartbeat_404_said = false;
                    log_line("relay now accepts /paper/heartbeat");
                }
                history.retain(|c| c.closed_ms > wall_ms - 2 * window_ms);
                let mut board = build_board(&args, &book, &feed, &desk, &history, started_ms, wall_ms, window_ms, now, n_ticks, n_reads, n_calls, n_failures);
                board.pit = gpit.as_ref().and_then(|g| g.summary());
                board.manual = gpit.as_ref().and_then(|g| g.manual_last.clone());
                poster.heartbeat(&board);
            }

            if now >= next_status {
                next_status = now + STATUS_EVERY_SECS;
                let (live, rate) = feed.live_summary(now);
                let open: Vec<String> = book.traders.iter().filter_map(|t| t.position.as_ref().map(|p| format!("{} {} {}", t.name(), p.inst.symbol(), side_str(p.side)))).collect();
                log_line(&format!(
                    "status: feed {:?} {live} pits live {rate:.1} t/s  ticks {n_ticks} books {n_depth}  reads {n_reads} calls {n_calls} failed {n_failures}  touches {n_touches} touch reads {n_touch_reads}  trades {}  open [{}]  records posted {} failed {}  heartbeat HTTP {}",
                    feed.state,
                    book.traders.iter().map(|t| t.trades.len()).sum::<usize>(),
                    open.join(", "),
                    poster.posted.load(Ordering::Relaxed),
                    poster.failures.load(Ordering::Relaxed),
                    poster.heartbeat_status.load(Ordering::Relaxed)
                ));
            }

            if idle {
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn build_board(
        args: &Args,
        book: &PaperBook,
        feed: &LiveFeed,
        jev: &JevDesk,
        history: &[Closed],
        started_ms: i64,
        wall_ms: i64,
        window_ms: i64,
        now: f64,
        ticks: u64,
        reads: u64,
        jev_calls: u64,
        jev_failures: u64,
    ) -> DeskBoard {
        // One row per trader on the book, in the book's order (the roster is trimmed and
        // genomes come and go, so it is not `Archetype::ALL`).
        let roster: Vec<(&str, &str)> = book.traders.iter().map(|t| (t.name(), t.archetype.label())).collect();
        let mut traders = leaderboard_for(&roster, history, wall_ms, window_ms);
        // The day block is the same arithmetic over a window that opened at 00:00 UTC.
        let day = leaderboard_for(&roster, history, wall_ms, wall_ms - opencry_visual::paper::day_start_ms(wall_ms));
        for (t, row) in traders.iter_mut().enumerate() {
            let tr = &book.traders[t];
            row.open = tr.position.as_ref().map(|p| DeskOpen {
                symbol: p.inst.symbol().to_string(),
                side: side_str(p.side).to_string(),
                entry: p.entry,
                open_r: book.open_r(t, feed).unwrap_or(0.0),
                opened_ms: p.opened_ms,
                why: p.why.clone(),
                stop: p.stop,
                target: p.target,
            });
            let d = &day[t];
            row.day_r = d.net_r;
            row.day_trades = d.trades;
            row.day_wins = d.wins;
            row.day_losses = d.losses;
            row.day_spread_paid_r = d.spread_paid_r;
            row.day_max_drawdown_r = d.max_drawdown_r;
            row.last_trades = last_trades(history, &row.trader);
            row.waiting = tr.passes().into_iter().map(|(inst, why)| DeskPass::new(inst.symbol(), why)).collect();
        }
        let (live, _) = feed.live_summary(now);
        let jev_reads = args
            .symbols
            .iter()
            .filter_map(|&inst| jev.read(inst).map(|r| DeskRead { symbol: inst.symbol().to_string(), headline: r.headline(), age_secs: (now - r.at).max(0.0) as f32 }))
            .collect();
        DeskBoard {
            jev_reads,
            desk_alive: false,
            heartbeat_age_ms: None,
            started_ms,
            sent_ms: wall_ms,
            window_hours: BOARD_WINDOW_HOURS,
            jev_mode: args.jev.label().to_string(),
            jev_every_secs: args.jev_every,
            symbols: args.symbols.iter().map(|i| tape_symbol(*i)).collect(),
            feed: format!("{}, {live} of {} pits ticking", format!("{:?}", feed.state).to_lowercase(), args.symbols.len()),
            ticks,
            reads,
            jev_calls,
            jev_failures,
            traders,
            pit: None,
            manual: None,
            ticket_trader: Some(manual_name()),
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use opencry_visual::paper::Position;
        use opencry_visual::pit_engine::Side;

        /// The desk's live edge-lab call against the real script and edge-lab's cache: on the
        /// last 2,500 bars up to each of a strategy's last entries, it must answer that entry.
        /// Needs edge-lab and a built `evolve/edge_lab/` on this machine, so it is not run by
        /// default: `cargo test --release --bin paper_desk -- --ignored edge_lab`.
        #[test]
        #[ignore]
        fn the_live_edge_lab_call_answers_what_the_backtest_entered() {
            let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("evolve");
            let eligible: serde_json::Value = serde_json::from_str(&fs::read_to_string(root.join("edge_lab/eligible.json")).unwrap()).unwrap();
            let pick = eligible["eligible"].as_array().unwrap().iter().find(|e| e["tf"] == 60 && e["n_entries"].as_u64().unwrap_or(0) > 400).unwrap();
            let (key, tf) = (pick["key"].as_str().unwrap(), 60u32);
            let sig: HashMap<String, Vec<(i64, i8)>> = serde_json::from_str(&fs::read_to_string(root.join(format!("edge_lab/signals/{key}@{tf}.json"))).unwrap()).unwrap();
            let rows: Vec<Vec<f64>> = serde_json::from_str(&fs::read_to_string(format!("{}/ctrader_US500_SB_15m.json", opencry_visual::edge_cache_dir())).unwrap()).unwrap();
            let mut bars: Vec<Bar> = rows.iter().map(|r| Bar { t: r[0] as i64, o: r[1], h: r[2], l: r[3], c: r[4] }).collect();
            bars.sort_by_key(|b| b.t);
            bars.dedup_by_key(|b| b.t);
            let entries = &sig["US500"];
            for &(t, d) in &entries[entries.len() - 3..] {
                let i = bars.binary_search_by_key(&t, |b| b.t).unwrap();
                assert_eq!(edge_lab_entry(&root.join("pit.json"), key, tf, &bars[i + 1 - GENOME_BARS_KEEP..=i]), Some(d), "{key}@{tf} at {t}");
                assert_eq!(edge_lab_entry(&root.join("pit.json"), key, tf, &bars[i - GENOME_BARS_KEEP..i]), Some(0), "no entry on the bar before");
            }
        }

        fn closed(trader: &str, closed_ms: i64, pnl_r: f32) -> Closed {
            let shown = DeskTrade { symbol: "XAG/USD".into(), side: "long".into(), pnl_r, closed_ms, ..Default::default() };
            Closed { trader: trader.into(), closed_ms, pnl_r, spread_r: 0.25, shown }
        }

        /// A pit file with one genome seated with its twin, read into a fresh book.
        fn pit_with_twin(dir: &std::path::Path) -> (GenomePit, PaperBook) {
            let pit = dir.join("pit.json");
            let g = serde_json::json!({"id": "gtw1", "name": "Twtest", "signal": {"kind": "run", "k": 3}, "mode": "follow", "window": "all",
                "exit": {"stop_atr": 1.0, "target_r": 2.0, "max_bars": 8}, "symbols": ["XAUUSD"]});
            fs::write(&pit, serde_json::json!({"seats": [{"genome": g, "seated_ms": 0, "twin": true}]}).to_string()).unwrap();
            let mut gp = GenomePit::new(pit, 0, dir.join("genome_jev.jsonl"));
            let mut book = PaperBook::default();
            book.log_to_relay = false;
            book.retain_traders(|_| false);
            gp.reload(&mut book, 0.0);
            (gp, book)
        }

        fn gold_view(bars: &[Candle], wall_ms: i64) -> View<'_> {
            View {
                inst: Instrument::Gold,
                bid: 2000.0,
                ask: 2000.2,
                tick: 0.1,
                spread_ticks: 2.0,
                now: 10.0,
                wall_ms,
                bars,
                flow: Default::default(),
                rate: 1.0,
                levels: &[],
                read: None,
                prev_bid: 2000.0,
            }
        }

        fn answer(gp: &GenomePit, p: Option<f32>, decided_ms: i64) -> TwinAnswer {
            TwinAnswer { twin: gp.twins.values().copied().next().unwrap(), genome_id: "gtw1".into(), inst: Instrument::Gold, decided_ms, side: Side::Buy, stop: 5.0, why: "test".into(), p, model: None, error: None }
        }

        #[test]
        fn a_twin_takes_its_genomes_trade_only_on_a_fresh_jev_yes() {
            let dir = std::env::temp_dir().join(format!("twin_test_{}", std::process::id()));
            fs::create_dir_all(&dir).unwrap();
            let (mut gp, mut book) = pit_with_twin(&dir);
            let names: Vec<&str> = book.traders.iter().map(|t| t.name()).collect();
            assert_eq!(names, vec!["Twtest", "Twtest-J", "Paul Leppard"], "the genome and its twin are seated, and the ticket's trader");
            let twin = book.traders.iter().position(|t| t.name() == "Twtest-J").unwrap();
            let bars: Vec<Candle> = Vec::new();
            let at = 1_792_800_000_000 - 1_792_800_000_000 % 86_400_000 + 10 * 3_600_000; // 10:00 UTC

            // No: stays out. Late yes (over 90 s): stays out.
            gp.answers_tx.send(answer(&gp, Some(0.3), at)).unwrap();
            gp.answers_tx.send(answer(&gp, Some(0.9), at - 100_000)).unwrap();
            gp.take_answers(&mut book, &gold_view(&bars, at + 2_000));
            assert!(book.traders[twin].position.is_none());

            // A yes for another pit waits for that pit.
            let mut other = answer(&gp, Some(0.8), at);
            other.inst = Instrument::Silver;
            gp.answers_tx.send(other).unwrap();
            gp.take_answers(&mut book, &gold_view(&bars, at + 2_000));
            assert!(book.traders[twin].position.is_none());
            assert_eq!(gp.ready.len(), 1, "silver's yes waits for a silver tick");

            // A fresh yes: in, at the ask, with the genome's target in R and the remaining time stop.
            gp.answers_tx.send(answer(&gp, Some(0.7), at)).unwrap();
            gp.take_answers(&mut book, &gold_view(&bars, at + 2_000));
            let p = book.traders[twin].position.clone().expect("the twin is in");
            assert_eq!(p.entry, 2000.2);
            assert!((p.stop_ticks - 50.0).abs() < 1e-3, "stop 5.0 = 50 ticks, above the 20-spread floor: {}", p.stop_ticks);
            assert!((p.target.unwrap() - (2000.2 + 10.0)).abs() < 1e-2, "target 2 R");
            assert!((p.time_stop_secs - (8.0 * 900.0 - 2.0)).abs() < 1e-6);
            assert!(p.why.contains("Jev yes 70%"), "{}", p.why);

            // Busy: a second yes does not stack a position.
            gp.answers_tx.send(answer(&gp, Some(0.95), at + 1_000)).unwrap();
            gp.take_answers(&mut book, &gold_view(&bars, at + 3_000));
            assert_eq!(book.traders[twin].position.as_ref().unwrap().entry, 2000.2);

            // Every answer is logged, the late one and the busy one included.
            let logged = fs::read_to_string(dir.join("genome_jev.jsonl")).unwrap();
            assert_eq!(logged.lines().count(), 5, "{logged}");
            let _ = fs::remove_dir_all(&dir);
        }

        fn holder_answer(seat: u16, g: i64, pos: i8, asked_ms: i64) -> HolderAnswer {
            HolderAnswer { seat, id: "h_test".into(), inst: Instrument::Gold, asked_ms, g, pos, unit: 30.0, error: None }
        }

        #[test]
        fn a_holder_acts_only_on_its_strategys_changes_and_holds_overnight() {
            let dir = std::env::temp_dir().join(format!("holder_test_{}", std::process::id()));
            fs::create_dir_all(&dir).unwrap();
            let pit = dir.join("pit.json");
            let g = serde_json::json!({"id": "h_test", "name": "Vegatest", "signal": {"kind": "edge_lab", "key": "yt_x", "tf": 1440}, "mode": "follow",
                "window": "all", "exit": {"stop_atr": 2.0, "target_r": 1.0, "max_bars": 0}, "symbols": ["XAUUSD"], "holds": true, "floor": "show"});
            fs::write(&pit, serde_json::json!({"seats": [], "show": [{"genome": g, "kind": "holder", "seated_ms": 0}]}).to_string()).unwrap();
            let mut gp = GenomePit::new(pit.clone(), 0, dir.join("genome_jev.jsonl"));
            let mut book = PaperBook::default();
            book.log_to_relay = false;
            book.retain_traders(|_| false);
            gp.reload(&mut book, 0.0);
            assert_eq!(book.traders.len(), 2, "the show floor is seated, and the ticket's trader");
            assert_eq!(book.traders[0].archetype.label(), "holder");
            let seat = *gp.seated.get("h_test").unwrap();
            let bars: Vec<Candle> = Vec::new();
            let day = 86_400_000;
            let at = 1_792_800_000_000 - 1_792_800_000_000 % day;

            // First answer since start: records the strategy, does not join mid-trade.
            gp.holder_tx.send(vec![holder_answer(seat, at - day, 1, at)]).unwrap();
            gp.take_holder_answers(&mut book, &gold_view(&bars, at + 1_000));
            assert!(book.traders[0].position.is_none(), "joins at the next change, not mid-trade");
            // Same position next bar: nothing. Then it turns short: in, at the bid, R = 30 = 300 ticks.
            gp.holder_tx.send(vec![holder_answer(seat, at, 1, at + day)]).unwrap();
            gp.take_holder_answers(&mut book, &gold_view(&bars, at + day + 1_000));
            assert!(book.traders[0].position.is_none());
            gp.holder_tx.send(vec![holder_answer(seat, at + day, -1, at + 2 * day)]).unwrap();
            gp.take_holder_answers(&mut book, &gold_view(&bars, at + 2 * day + 1_000));
            let p = book.traders[0].position.clone().expect("in on the change");
            assert_eq!(p.entry, 2000.0);
            assert!((p.stop_ticks - 300.0).abs() < 1e-3 && (p.stop - (2000.0 + 150.0)).abs() < 1e-2, "1 R = 300 ticks, safety stop 5 R: {p:?}");
            assert!(p.overnight && p.target.is_none());
            // The desk's flat time does not close it.
            let mut q = p.clone();
            assert!(opencry_visual::paper::exit_check(&mut q, 2000.0, 2000.2, 0.1, 1e9, 21 * 60).is_none(), "held past FLAT_AT");
            // A restart restores it, and the state it acted on.
            let mut gp2 = GenomePit::new(pit, 0, dir.join("genome_jev.jsonl"));
            let mut book2 = PaperBook::default();
            book2.log_to_relay = false;
            book2.retain_traders(|_| false);
            gp2.reload(&mut book2, 0.0);
            assert_eq!(book2.traders[0].position.as_ref().map(|p| (p.entry, p.overnight)), Some((2000.0, true)), "restored");
            assert_eq!(gp2.holder_state.get("h_test@XAUUSD"), Some(&(at + day, -1)));
            // Strategy goes flat: a strategy exit, at the ask.
            gp.holder_tx.send(vec![holder_answer(seat, at + 2 * day, 0, at + 3 * day)]).unwrap();
            gp.take_holder_answers(&mut book, &gold_view(&bars, at + 3 * day + 1_000));
            assert!(book.traders[0].position.is_none());
            let tr = book.traders[0].trades.last().unwrap();
            assert_eq!((tr.why_out.as_str(), tr.exit), ("strategy exit", 2000.2));
            assert!((tr.pnl_r - (-2.0 / 300.0)).abs() < 1e-4, "paid the spread: {}", tr.pnl_r);
            let _ = fs::remove_dir_all(&dir);
        }

        #[test]
        fn the_trade_ticket_buys_closes_and_refuses_as_paper_trades() {
            let dir = std::env::temp_dir().join(format!("ticket_test_{}", std::process::id()));
            fs::create_dir_all(&dir).unwrap();
            let (mut gp, mut book) = pit_with_twin(&dir);
            let paul = book.traders.iter().position(|t| t.name() == "Paul Leppard").expect("the ticket's trader is seated");
            assert_eq!(book.traders[paul].archetype.label(), "manual");
            let at = 1_792_800_000_000 - 1_792_800_000_000 % 86_400_000 + 10 * 3_600_000;
            // 15m gold bars with a range of 2.0: ATR 2.0.
            gp.bars[Instrument::Gold as usize] = (0..20).map(|k| Bar { t: at - (20 - k) * BAR_MS, o: 2000.0, h: 2001.0, l: 1999.0, c: 2000.0 }).collect();
            let order = |seq, action: &str| ManualOrder { seq, action: action.into(), symbol: "XAUUSD".into(), stop_atr: 3.0, target_r: 2.0, placed_ms: at, account: "paper".into(), trader: None };
            let bars: Vec<Candle> = Vec::new();

            gp.manual_tx.send(order(1, "buy")).unwrap();
            gp.take_manual_orders(&mut book, &gold_view(&bars, at + 1_000));
            let p = book.traders[paul].position.clone().expect("bought");
            assert_eq!(p.entry, 2000.2, "at the ask");
            assert!((p.stop_ticks - 60.0).abs() < 1e-3, "3 ATR = 6.0 = 60 ticks, above the desk's 20-spread floor: {}", p.stop_ticks);
            assert!((p.target.unwrap() - 2012.2).abs() < 1e-2 && p.overnight, "2 R");
            assert!(gp.manual_last.as_deref().unwrap().starts_with("#1 bought XAUUSD at 2,000.20"), "{:?}", gp.manual_last);

            gp.manual_tx.send(order(2, "sell")).unwrap();
            gp.take_manual_orders(&mut book, &gold_view(&bars, at + 2_000));
            assert!(gp.manual_last.as_deref().unwrap().contains("close it first"));

            // An order for another pit waits for that pit; one over 30 s old is dropped.
            let mut other = order(3, "buy");
            other.symbol = "XAGUSD".into();
            gp.manual_tx.send(other).unwrap();
            gp.take_manual_orders(&mut book, &gold_view(&bars, at + 3_000));
            assert_eq!(gp.manual_ready.len(), 1);
            gp.take_manual_orders(&mut book, &gold_view(&bars, at + 40_000));
            assert!(gp.manual_ready.is_empty() && gp.manual_last.as_deref().unwrap().contains("not filled"));

            let mut close = order(4, "close");
            close.placed_ms = at + 40_500;
            gp.manual_tx.send(close).unwrap();
            gp.take_manual_orders(&mut book, &gold_view(&bars, at + 41_000));
            assert!(book.traders[paul].position.is_none());
            let tr = book.traders[paul].trades.last().unwrap();
            assert_eq!((tr.why_out.as_str(), tr.genome), ("closed by hand", Some("manual")));
            assert!(gp.manual_last.as_deref().unwrap().starts_with("#4 closed long XAUUSD from 2,000.20 at 2,000.00"), "{:?}", gp.manual_last);
            let _ = fs::remove_dir_all(&dir);
        }

        #[test]
        fn an_adapter_switches_to_the_tactic_it_picks_and_logs_the_choice() {
            let dir = std::env::temp_dir().join(format!("adapter_test_{}", std::process::id()));
            fs::create_dir_all(&dir).unwrap();
            let pit = dir.join("pit.json");
            let t1 = serde_json::json!({"id": "g1", "name": "T1", "signal": {"kind": "run", "k": 3}, "mode": "follow", "window": "all",
                "exit": {"stop_atr": 1.0, "target_r": 2.0, "max_bars": 8}, "symbols": ["XAUUSD"]});
            let t2 = serde_json::json!({"id": "g2", "name": "T2", "signal": {"kind": "orb", "m": 2}, "mode": "fade", "window": "london",
                "exit": {"stop_atr": 1.5, "target_r": 1.0, "max_bars": 16}, "symbols": ["US500"]});
            let mut g = t1.clone();
            g["id"] = "a001c".into();
            g["name"] = "Kameleon".into();
            g["tactics"] = serde_json::json!([t1, t2]);
            g["adapt"] = "code".into();
            g["floor"] = "show".into();
            fs::write(&pit, serde_json::json!({"seats": [], "show": [{"genome": g, "kind": "adapter", "pair": 1, "seated_ms": 0}]}).to_string()).unwrap();
            let mut gp = GenomePit::new(pit, 0, dir.join("genome_jev.jsonl"));
            let mut book = PaperBook::default();
            book.log_to_relay = false;
            book.retain_traders(|_| false);
            gp.reload(&mut book, 0.0);
            let seat = *gp.seated.get("a001c").unwrap();
            assert_eq!(book.traders[0].archetype.label(), "adapter");
            assert!(genome::seat_creed(seat).starts_with("adapter (code): picks one of 2 tactics"), "{}", genome::seat_creed(seat));
            gp.adapt_tx.send(AdaptAnswer { seat, day: 1, scores: vec![(0, 5, -0.2), (1, 4, 0.3)], code_pick: Some(1), jev: None, error: None }).unwrap();
            let bars: Vec<Candle> = Vec::new();
            gp.take_adapt_answers(&mut book, &gold_view(&bars, 1_000));
            let now = genome::seat(seat).unwrap().genome.unwrap();
            assert_eq!((now.id.as_str(), now.symbols.clone(), genome::tactic_in_use(&now)), ("a001c", vec!["US500".to_string()], Some(1)));
            assert!(genome::seat_creed(seat).contains("opening range"), "{}", genome::seat_creed(seat));
            let logged = fs::read_to_string(dir.join("adapt.jsonl")).unwrap();
            let row: serde_json::Value = serde_json::from_str(logged.lines().next().unwrap()).unwrap();
            assert_eq!((row["from"].as_u64(), row["to"].as_u64()), (Some(1), Some(2)));
            let _ = fs::remove_dir_all(&dir);
        }

        #[test]
        fn the_default_log_dir_sits_beside_the_binarys_crate_not_where_it_was_compiled() {
            let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
            let exe = crate_dir.join("target").join("release").join("paper_desk");
            assert_eq!(default_log_dir(Some(exe)), crate_dir.join("log"));
            // A binary that is not under a crate's target tree falls back to the manifest dir.
            assert_eq!(default_log_dir(Some(PathBuf::from("/usr/local/bin/paper_desk"))), crate_dir.join("log"));
            assert_eq!(default_log_dir(None), crate_dir.join("log"));
        }

        #[test]
        fn the_day_block_is_the_window_since_midnight_and_last_trades_come_newest_first() {
            let day0 = opencry_visual::paper::day_start_ms(1_792_657_000_000);
            let h = vec![
                closed("Grace", day0 - 3_600_000, -2.0), // yesterday: in the 24 h window, not the day
                closed("Grace", day0 + 1_000, 1.0),
                closed("Grace", day0 + 2_000, -0.5),
                closed("Colin", day0 + 1_500, 0.3),
            ];
            let now = day0 + 10_000;
            let window = leaderboard(&h, now, 24 * 3_600_000);
            let day = leaderboard(&h, now, now - day0);
            let g = Archetype::ALL.iter().position(|a| a.name() == "Grace").unwrap();
            assert_eq!(window[g].trades, 3);
            assert_eq!(day[g].trades, 2);
            assert!((day[g].net_r - 0.5).abs() < 1e-6);
            assert!((window[g].net_r + 1.5).abs() < 1e-6);
            let lt = last_trades(&h, "Grace");
            assert_eq!(lt.len(), 3);
            assert_eq!(lt[0].closed_ms, day0 + 2_000, "newest first");
            assert_eq!(lt[2].closed_ms, day0 - 3_600_000);
            assert!(last_trades(&h, "Priya").is_empty());
            let many: Vec<Closed> = (0..15).map(|k| closed("Colin", day0 + k * 1000, 0.1)).collect();
            assert_eq!(last_trades(&many, "Colin").len(), LAST_TRADES_KEEP);
        }

        #[test]
        fn the_leaderboard_is_windowed_and_measures_drawdown_within_the_window() {
            let h = vec![
                closed("Grace", 1_000, -5.0), // outside the window
                closed("Grace", 50_000, 1.0),
                closed("Grace", 60_000, -0.5),
                closed("Grace", 70_000, -0.7),
                closed("Grace", 80_000, 2.0),
                closed("Colin", 65_000, -1.0),
                closed("Nobody", 66_000, 9.0), // unknown trader is ignored
            ];
            let rows = leaderboard(&h, 100_000, 90_000);
            assert_eq!(rows.len(), Archetype::ALL.len());
            let grace = rows.iter().find(|r| r.trader == "Grace").unwrap();
            assert_eq!(grace.archetype, "trend");
            assert_eq!((grace.trades, grace.wins, grace.losses), (4, 2, 2));
            assert!((grace.net_r - 1.8).abs() < 1e-5, "{}", grace.net_r);
            assert!((grace.spread_paid_r - 1.0).abs() < 1e-5);
            assert!((grace.max_drawdown_r - 1.2).abs() < 1e-5, "peak 1.0 to -0.2: {}", grace.max_drawdown_r);
            let colin = rows.iter().find(|r| r.trader == "Colin").unwrap();
            assert_eq!((colin.trades, colin.losses), (1, 1));
            assert!((colin.max_drawdown_r - 1.0).abs() < 1e-5);
            assert_eq!(rows.iter().find(|r| r.trader == "Priya").unwrap().trades, 0);
            assert!(rows.iter().all(|r| r.open.is_none()));
        }

        #[test]
        fn a_logged_trade_row_reloads_into_history() {
            let v: serde_json::Value = serde_json::from_str(
                r#"{"trader":"Tomasz","archetype":"GapFiller","symbol":"US500","side":"long","entry":7770.7,"exit":7770.8,
                    "opened_ms":1,"closed_ms":1792608079362,"pnl_ticks":0.4,"pnl_gbp":0.4,"pnl_r":0.08,"stop_ticks":5.0,
                    "spread_ticks_in":1.6,"why_in":"x","why_out":"target","observation_hash":"","read_source":null,"source":"desk"}"#,
            )
            .unwrap();
            let c = closed_from_json(&v).unwrap();
            assert_eq!(c.trader, "Tomasz");
            assert_eq!(c.closed_ms, 1792608079362);
            assert!((c.spread_r - 0.32).abs() < 1e-5);
            assert!(closed_from_json(&serde_json::json!({"trader": "x"})).is_none());
        }

        #[test]
        fn records_carry_the_source_and_the_apps_fields() {
            let d = decision_json("Vinnie", Archetype::MomentumScalper, "XAG/USD", 5, "enter", "long", 57.8, 57.79, 57.8, "why", Some("abc"));
            assert_eq!(d["source"], "desk");
            assert_eq!(d["archetype"], "MomentumScalper");
            assert_eq!(d["observation_hash"], "abc");
            for k in ["trader", "symbol", "at_ms", "action", "side", "price", "bid", "ask", "why"] {
                assert!(d.get(k).is_some(), "{k}");
            }
            let exit = decision_json("Vinnie", Archetype::MomentumScalper, "XAG/USD", 6, "exit", "long", 57.9, 57.9, 57.92, "target", None);
            assert!(exit["observation_hash"].is_null());
        }

        #[test]
        fn a_snapshot_diff_sees_new_entries_and_new_closes() {
            let mut book = PaperBook::default();
            book.log_to_relay = false;
            let before = Snapshot::of(&book);
            let p = Position {
                inst: Instrument::Silver,
                side: Side::Buy,
                entry: 57.8,
                stop: 57.7,
                target: Some(58.0),
                trail: None,
                opened_ms: 100,
                opened_at: 1.0,
                time_stop_secs: 60.0,
                best: 57.8,
                stop_ticks: 10.0,
                spread_ticks_in: 2.0,
                why: "test".into(),
                observation_hash: String::new(),
                read_source: None,
                overnight: false,
            };
            book.traders[1].position = Some(p.clone());
            book.traders[3].trades.push(Trade {
                trader: "Priya",
                archetype: Archetype::IctLevelTrader,
                symbol: "XAG/USD",
                side: "short",
                entry: 1.0,
                exit: 0.9,
                opened_ms: 1,
                closed_ms: 2,
                pnl_ticks: 10.0,
                pnl_gbp: 10.0,
                pnl_r: 1.0,
                stop_ticks: 10.0,
                spread_ticks_in: 1.0,
                why_in: "in".into(),
                why_out: "target".into(),
                observation_hash: String::new(),
                read_source: None,
                genome: None,
            });
            let (closed, opened) = before.diff(&book);
            assert_eq!(opened, vec![1]);
            assert_eq!(closed.len(), 1);
            assert_eq!(closed[0].0, 3);
            assert_eq!(trade_json(closed[0].1)["source"], "desk");
            // The same position again is not a new entry; a new opened_ms is.
            let again = Snapshot::of(&book);
            assert!(again.diff(&book).1.is_empty());
            book.traders[1].position = Some(Position { opened_ms: 200, ..p });
            assert_eq!(again.diff(&book).1, vec![1]);
        }

        #[test]
        fn the_websocket_address_comes_from_the_tape_url_and_symbols_parse() {
            assert_eq!(ws_from_http("http://localhost:8083"), "ws://localhost:8083");
            assert_eq!(ws_from_http("https://tape.example:443"), "wss://tape.example:443");
            assert_eq!(ws_from_http("ws://already"), "ws://already");
            assert_eq!(parse_symbols(None).unwrap().len(), 9);
            assert_eq!(parse_symbols(Some("xagusd,EUR/USD")).unwrap(), vec![Instrument::Silver, Instrument::EurUsd]);
            assert!(parse_symbols(Some("DOGE")).is_err());
        }
    }
}
