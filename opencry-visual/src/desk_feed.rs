//! The headless paper desk's board, read back from the relay.
//!
//! The paper book in the browser only runs while a tab is open. `src/bin/paper_desk.rs` runs
//! the same six traders on the live tape around the clock and posts a heartbeat to the relay
//! every few seconds with its per-trader totals and open positions; the relay serves the last
//! one at `GET /paper/board` with `desk_alive` set while the heartbeat is fresh. This module
//! polls that route, and while the desk is alive the Traders panel shows the desk's board and
//! the in-tab book stops opening positions, so the same tape is never scored twice. When the
//! desk is gone (or the relay predates the route) the app behaves as it always did.
//!
//! The structs here are the wire format: the desk serialises `DeskBoard`, the relay adds
//! `desk_alive` / `heartbeat_age_ms`, the app deserialises it. Every field defaults so an
//! older or newer desk still parses.
//!
//! Besides the rolling 24 h board each trader row carries what the floor avatars
//! (`paper_avatars.rs`) show: the day's totals since 00:00 UTC (`day_*`), the last ten closed
//! trades, and the open position's stop and target. A desk that predates them sends none and
//! the avatars read zero / empty, never an error.

use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::jev::relay_url;
use crate::paper::{Archetype, PaperBook, Trade};

/// How often the app asks the relay for the board.
pub const DESK_POLL_SECS: f64 = 2.0;
/// Without a good answer for this long the desk is treated as gone. The relay already says
/// `desk_alive: false` 30 s after the last heartbeat; this covers the relay itself being down.
pub const DESK_LOST_AFTER_SECS: f64 = 20.0;
/// The desk scores a rolling window so the board reads the same at 03:00 and at 15:00.
pub const BOARD_WINDOW_HOURS: f32 = 24.0;
/// A poll that has not answered in this long is given up on.
const POLL_TIMEOUT_SECS: f64 = 15.0;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct DeskOpen {
    pub symbol: String,
    pub side: String,
    pub entry: f32,
    /// Mark-to-market in R at the desk's last touch.
    pub open_r: f32,
    pub opened_ms: i64,
    pub why: String,
    /// Current stop (trails move it) and the target if the idea has one. Zero / absent from
    /// a desk that predates them.
    #[serde(default)]
    pub stop: f32,
    #[serde(default)]
    pub target: Option<f32>,
}

/// One closed trade as the avatars' detail panel shows it: what, which way, in, out, result,
/// when, and the reasons. A `paper::Trade` minus the scoring internals.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct DeskTrade {
    pub symbol: String,
    pub side: String,
    pub entry: f32,
    pub exit: f32,
    pub pnl_r: f32,
    pub opened_ms: i64,
    pub closed_ms: i64,
    pub why_in: String,
    pub why_out: String,
}

/// Reasons longer than this are cut on the wire; the panel shows one line anyway.
pub const WHY_WIRE_CHARS: usize = 120;
/// How many closed trades a heartbeat carries per trader.
pub const LAST_TRADES_KEEP: usize = 10;

fn cut(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        s.chars().take(n.saturating_sub(1)).collect::<String>() + "\u{2026}"
    }
}

impl DeskTrade {
    pub fn from_trade(t: &Trade) -> Self {
        DeskTrade {
            symbol: t.symbol.to_string(),
            side: t.side.to_string(),
            entry: t.entry,
            exit: t.exit,
            pnl_r: t.pnl_r,
            opened_ms: t.opened_ms,
            closed_ms: t.closed_ms,
            why_in: cut(&t.why_in, WHY_WIRE_CHARS),
            why_out: cut(&t.why_out, WHY_WIRE_CHARS),
        }
    }
}

/// Why a trader is out of one pit: the last reason its idea gave for passing there.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct DeskPass {
    pub symbol: String,
    pub why: String,
}

impl DeskPass {
    pub fn new(symbol: &str, why: &str) -> Self {
        DeskPass {
            symbol: symbol.to_string(),
            why: cut(why, WHY_WIRE_CHARS),
        }
    }
}

/// Jev's latest read of one pit, as the desk saw it when it sent the heartbeat.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct DeskRead {
    pub symbol: String,
    /// `MarketInterpretation::headline()`: numbers then words.
    pub headline: String,
    pub age_secs: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct DeskTrader {
    pub trader: String,
    /// `Archetype::label()`.
    pub archetype: String,
    pub trades: u32,
    pub wins: u32,
    pub losses: u32,
    pub net_r: f32,
    /// Spread paid at entry as a fraction of the stop, summed over the window.
    pub spread_paid_r: f32,
    pub max_drawdown_r: f32,
    #[serde(default)]
    pub open: Option<DeskOpen>,
    /// The day so far, since 00:00 UTC: what the avatar's overhead label shows.
    #[serde(default)]
    pub day_r: f32,
    #[serde(default)]
    pub day_trades: u32,
    #[serde(default)]
    pub day_wins: u32,
    #[serde(default)]
    pub day_losses: u32,
    #[serde(default)]
    pub day_spread_paid_r: f32,
    #[serde(default)]
    pub day_max_drawdown_r: f32,
    /// Newest first, at most `LAST_TRADES_KEEP`.
    #[serde(default)]
    pub last_trades: Vec<DeskTrade>,
    /// Per pit, why the trader passed at its last look there (empty from an older desk).
    #[serde(default)]
    pub waiting: Vec<DeskPass>,
}

impl DeskTrader {
    pub fn win_rate(&self) -> f32 {
        let n = self.wins + self.losses;
        if n == 0 {
            0.0
        } else {
            self.wins as f32 / n as f32
        }
    }

    pub fn archetype(&self) -> Option<Archetype> {
        Archetype::ALL
            .iter()
            .copied()
            .find(|a| a.name() == self.trader)
    }

    /// Traded today (since 00:00 UTC) or holding a position now. Only these are listed and
    /// walk the floor; the rest are still running on the desk, waiting for a signal.
    pub fn is_active(&self) -> bool {
        self.day_trades > 0 || self.open.is_some()
    }

    pub fn day_win_rate(&self) -> f32 {
        let n = self.day_wins + self.day_losses;
        if n == 0 {
            0.0
        } else {
            self.day_wins as f32 / n as f32
        }
    }

    /// A trader from the evolving pit, by its board label (`genome::tag_of`). Genomes come
    /// and go, so no build knows them by name, and an unknown genome never means the page is
    /// out of date.
    pub fn is_genome(&self) -> bool {
        matches!(
            self.archetype.as_str(),
            "genome" | "show" | "holder" | "adapter" | "adapter+J" | "manual"
        )
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct DeskBoard {
    /// Set by the relay, never by the desk.
    #[serde(default)]
    pub desk_alive: bool,
    #[serde(default)]
    pub heartbeat_age_ms: Option<u64>,
    #[serde(default)]
    pub started_ms: i64,
    #[serde(default)]
    pub sent_ms: i64,
    #[serde(default)]
    pub window_hours: f32,
    #[serde(default)]
    pub jev_mode: String,
    #[serde(default)]
    pub jev_every_secs: f64,
    #[serde(default)]
    pub symbols: Vec<String>,
    /// One line on the feed: "live, 8 of 9 pits ticking".
    #[serde(default)]
    pub feed: String,
    #[serde(default)]
    pub ticks: u64,
    #[serde(default)]
    pub reads: u64,
    #[serde(default)]
    pub jev_calls: u64,
    #[serde(default)]
    pub jev_failures: u64,
    #[serde(default)]
    pub traders: Vec<DeskTrader>,
    /// Jev's latest read per pit (empty from an older desk).
    #[serde(default)]
    pub jev_reads: Vec<DeskRead>,
    /// The evolving pit, when the desk runs one (absent from older desks).
    #[serde(default)]
    pub pit: Option<DeskPit>,
    /// What became of the last manual order from the trade ticket.
    #[serde(default)]
    pub manual: Option<String>,
    /// The name the desk books the ticket's trades under (`OPENCRY_TRADER_NAME`, else
    /// `ticket::DEFAULT_TICKET_TRADER`).
    #[serde(default)]
    pub ticket_trader: Option<String>,
}

impl DeskBoard {
    pub fn jev_read(&self, symbol: &str) -> Option<&DeskRead> {
        self.jev_reads.iter().find(|r| r.symbol == symbol)
    }

    /// Best realised R today first; active traders only.
    pub fn ranking(&self) -> Vec<usize> {
        let mut idx: Vec<usize> = (0..self.traders.len())
            .filter(|&t| self.traders[t].is_active())
            .collect();
        idx.sort_by(|a, b| {
            self.traders[*b]
                .day_r
                .partial_cmp(&self.traders[*a].day_r)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        idx
    }

    pub fn trader(&self, arch: Archetype) -> Option<&DeskTrader> {
        self.traders.iter().find(|t| t.trader == arch.name())
    }

    /// The panel header while the desk is showing.
    pub fn header_line(&self) -> String {
        let age = self
            .heartbeat_age_ms
            .map(|ms| format!("heartbeat {}s ago", ms / 1000))
            .unwrap_or_else(|| "no heartbeat".into());
        let idle = self.traders.iter().filter(|t| !t.is_active()).count();
        format!(
            "DESK TODAY (since 00:00 UTC, all pits)  |  realised R (profit / initial stop), spread paid  |  {age}  |  {idle} more waiting for a signal, not shown  |  T hides"
        )
    }

    /// One leaderboard row, same columns as the in-tab book. `live_open_r` is the open
    /// position marked at the tab's own feed, fresher than the heartbeat's number.
    pub fn row_line(&self, rank: usize, t: &DeskTrader, live_open_r: Option<f32>) -> String {
        let open = match &t.open {
            Some(p) => format!(
                "{} {} {:+.2}R",
                p.symbol,
                p.side,
                live_open_r.unwrap_or(p.open_r)
            ),
            None => "flat".to_string(),
        };
        // Compact enough that 29 rows fit the panel one line each: "12. Jo     breaker      +1.51R   1t 100% dd 0.0 sp 0.1 | GBP/USD long -0.17R".
        format!(
            "{:>2}. {:<6} {:<12} {:>+6.2}R {:>3}t {:>3.0}% dd{:>4.1} sp{:>4.1} | {}",
            rank + 1,
            t.trader,
            t.archetype,
            t.day_r,
            t.day_trades,
            t.day_win_rate() * 100.0,
            t.day_max_drawdown_r,
            t.day_spread_paid_r,
            open
        )
    }

    /// The detail block under the rows: a focused genome (by name) if one is picked, else
    /// the focused built-in trader; the evolving pit's line either way.
    pub fn detail_lines_for(&self, focus: Archetype, genome: Option<&str>) -> Vec<String> {
        let seat = genome.and_then(|n| self.pit.as_ref().and_then(|p| p.seat(n)));
        let mut lines = match seat {
            Some(seat) => {
                let mut l = vec![self.detail_lines(focus)[0].clone()];
                l.extend(seat.lines());
                if let Some(p) = self
                    .traders
                    .iter()
                    .find(|t| t.trader == seat.name)
                    .and_then(|t| t.open.as_ref())
                {
                    l.push(format!(
                        "OPEN {} {} from {}  {:+.2}R  |  {}",
                        p.symbol, p.side, p.entry, p.open_r, p.why
                    ));
                }
                l
            }
            None => self.detail_lines(focus),
        };
        if let Some(p) = &self.pit {
            lines.push(p.summary_line());
            if !p.recent_retired.is_empty() {
                lines.push(format!(
                    "retired lately: {}",
                    p.recent_retired.join("  |  ")
                ));
            }
        }
        lines
    }

    /// The detail block under the rows for the focused trader.
    pub fn detail_lines(&self, focus: Archetype) -> Vec<String> {
        let mut lines = vec![format!(
            "Desk: Jev {} every {:.0}s on {} pits, {} ticks, {} Jev calls ({} failed)  |  {}",
            self.jev_mode,
            self.jev_every_secs,
            self.symbols.len(),
            self.ticks,
            self.jev_calls,
            self.jev_failures,
            self.feed
        )];
        lines.push(format!(
            "{} ({}): {}",
            focus.name(),
            focus.label(),
            focus.describe()
        ));
        if let Some(p) = self.trader(focus).and_then(|t| t.open.as_ref()) {
            lines.push(format!(
                "OPEN {} {} from {}  {:+.2}R  |  {}",
                p.symbol, p.side, p.entry, p.open_r, p.why
            ));
        }
        lines
    }
}

/// The evolving pit as the desk last read it from `evolve/pit.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct DeskPit {
    #[serde(default)]
    pub n_tried: u64,
    #[serde(default)]
    pub seated_ever: u64,
    #[serde(default)]
    pub retired: u64,
    #[serde(default)]
    pub updated_ms: i64,
    #[serde(default)]
    pub seats: Vec<DeskSeat>,
    /// The latest retirements, newest first: "Name: rule".
    #[serde(default)]
    pub recent_retired: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct DeskSeat {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub creed: String,
    #[serde(default)]
    pub seated_ms: i64,
    #[serde(default)]
    pub bt_is_mean: f32,
    #[serde(default)]
    pub bt_is_t: f32,
    #[serde(default)]
    pub bt_ho_mean: f32,
    #[serde(default)]
    pub bt_ho_t: f32,
    #[serde(default)]
    pub fwd_n: u32,
    #[serde(default)]
    pub fwd_mean: f32,
    #[serde(default)]
    pub fwd_t: f32,
    #[serde(default)]
    pub graduated: bool,
    /// For a genome seated with a Jev twin, and for the twin itself: the genome's forward trades
    /// split by Jev's answer at their entry.
    #[serde(default)]
    pub jev: Option<DeskJevSplit>,
    /// Set on a twin: the genome it shadows.
    #[serde(default)]
    pub twin_of: Option<String>,
    /// "show" for a seat on the show floor: unproven, never counted toward graduation.
    #[serde(default)]
    pub floor: Option<String>,
    /// "genome", "show", "holder", "adapter" or "adapter+J".
    #[serde(default)]
    pub tag: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct DeskJevSplit {
    pub yes_n: u32,
    pub yes_mean: f32,
    pub no_n: u32,
    pub no_mean: f32,
    /// Welch t of yes minus no.
    pub t: f32,
    /// "adds something" / "adds nothing" once judged at 300 tagged trades.
    #[serde(default)]
    pub verdict: Option<String>,
}

/// The forward checkpoints from evolve/README.md, for "next check at N trades".
pub const PIT_CHECKS: [u32; 4] = [30, 60, 150, 300];

impl DeskPit {
    pub fn summary_line(&self) -> String {
        format!(
            "EVOLVING PIT  {} genomes tried, {} seated by the gate now ({} ever) plus {} Jev twins, {} retired  |  SHOW FLOOR {} genomes, {} holders, {} adapters (unproven, never graduate)  |  cut: sequential test, or t<=-3 at 30 trades, idle 21d  |  graduate: 300 trades, +0.05R, deflated Sharpe 0.95",
            self.n_tried,
            self.seats
                .iter()
                .filter(|s| s.twin_of.is_none() && s.floor.is_none())
                .count(),
            self.seated_ever,
            self.seats.iter().filter(|s| s.twin_of.is_some()).count(),
            self.retired,
            self.seats
                .iter()
                .filter(|s| s.floor.is_some() && s.tag.as_deref() == Some("show"))
                .count(),
            self.seats
                .iter()
                .filter(|s| s.tag.as_deref() == Some("holder"))
                .count(),
            self.seats
                .iter()
                .filter(|s| s.tag.as_deref().is_some_and(|t| t.starts_with("adapter")))
                .count(),
        )
    }

    pub fn seat(&self, name: &str) -> Option<&DeskSeat> {
        self.seats.iter().find(|s| s.name == name)
    }
}

impl DeskSeat {
    pub fn lines(&self) -> Vec<String> {
        let next = PIT_CHECKS
            .iter()
            .find(|&&c| c > self.fwd_n)
            .map(|c| format!("next check at {c} trades"))
            .unwrap_or_else(|| "past every checkpoint".into());
        let mut lines = self.base_lines(&next);
        if let Some(j) = &self.jev {
            lines.push(format!(
                "Jev twin test: of {}'s entries, Jev said yes to {} ({:+.3}R a trade) and no to {} ({:+.3}R), t {:+.2}{}  |  judged at 300 tagged trades",
                self.twin_of.as_deref().unwrap_or(&self.name),
                j.yes_n,
                j.yes_mean,
                j.no_n,
                j.no_mean,
                j.t,
                j.verdict.as_ref().map(|v| format!("  |  VERDICT: Jev {v}")).unwrap_or_default()
            ));
        }
        lines
    }

    fn base_lines(&self, next: &str) -> Vec<String> {
        if let Some(of) = &self.twin_of {
            return vec![
                format!(
                    "{} (Jev twin of {}): takes {}'s entries only when Jev says yes. {}",
                    self.name, of, of, self.creed
                ),
                format!(
                    "its own forward record: {} trades {:+.3}R (t {:.1}); shown, not judged: the test is the split below",
                    self.fwd_n, self.fwd_mean, self.fwd_t
                ),
            ];
        }
        let what = match self.tag.as_deref() {
            Some("holder") => "SHOW FLOOR holder",
            Some("adapter") | Some("adapter+J") => "SHOW FLOOR adapter",
            _ if self.floor.is_some() => "SHOW FLOOR genome",
            _ => "genome",
        };
        vec![
            format!("{} ({what} {}): {}", self.name, self.id, self.creed),
            format!(
                "backtest: 2021-24 {:+.3}R a trade (t {:.1}), 2025-26 holdout {:+.3}R (t {:.1})  |  forward: {} trades {:+.3}R (t {:.1}), {}{}",
                self.bt_is_mean,
                self.bt_is_t,
                self.bt_ho_mean,
                self.bt_ho_t,
                self.fwd_n,
                self.fwd_mean,
                self.fwd_t,
                next,
                if self.graduated {
                    "  |  GRADUATED: a live-book candidate (Paul's call)"
                } else {
                    ""
                }
            ),
        ]
    }
}

pub fn parse_board(text: &str) -> Result<DeskBoard, String> {
    serde_json::from_str(text).map_err(|e| format!("board: {e}"))
}

type Pending = Arc<Mutex<Option<Result<(u16, String), String>>>>;

#[derive(Resource)]
pub struct DeskFeed {
    /// The last board that parsed, alive or not.
    pub board: Option<DeskBoard>,
    pub last_ok_at: f64,
    pub last_error: String,
    pub polls: u32,
    next_poll_at: f64,
    in_flight_since: Option<f64>,
    pending: Pending,
}

impl Default for DeskFeed {
    fn default() -> Self {
        Self {
            board: None,
            last_ok_at: -1.0e9,
            last_error: String::new(),
            polls: 0,
            next_poll_at: 0.0,
            in_flight_since: None,
            pending: Arc::new(Mutex::new(None)),
        }
    }
}

impl DeskFeed {
    /// The desk is running and the relay has told us so recently.
    pub fn alive(&self, now: f64) -> bool {
        self.board.as_ref().is_some_and(|b| b.desk_alive)
            && now - self.last_ok_at <= DESK_LOST_AFTER_SECS
    }

    pub fn live_board(&self, now: f64) -> Option<&DeskBoard> {
        if self.alive(now) {
            self.board.as_ref()
        } else {
            None
        }
    }

    /// Fold a finished poll in. Pure apart from the clock, so it is testable.
    fn settle(&mut self, res: Result<(u16, String), String>, now: f64) {
        self.in_flight_since = None;
        match res {
            Ok((200, text)) => match parse_board(&text) {
                Ok(b) => {
                    self.board = Some(b);
                    self.last_ok_at = now;
                    self.last_error.clear();
                }
                Err(e) => self.last_error = e,
            },
            // 404 is a relay that predates the route: the app simply runs its own book.
            Ok((status, _)) => self.last_error = format!("HTTP {status}"),
            Err(e) => self.last_error = e,
        }
    }
}

/// System: ask the relay for the desk board every `DESK_POLL_SECS`; never blocks the frame.
pub fn poll_desk_board(time: Res<Time>, mut feed: ResMut<DeskFeed>) {
    let now = time.elapsed_secs_f64();
    let done = feed.pending.lock().ok().and_then(|mut p| p.take());
    if let Some(res) = done {
        feed.settle(res, now);
    }
    if feed
        .in_flight_since
        .is_some_and(|since| now - since > POLL_TIMEOUT_SECS)
    {
        feed.in_flight_since = None;
        feed.last_error = "board poll timed out".into();
    }
    if feed.in_flight_since.is_some() || now < feed.next_poll_at {
        return;
    }
    feed.next_poll_at = now + DESK_POLL_SECS;
    feed.in_flight_since = Some(now);
    feed.polls += 1;
    let pending = feed.pending.clone();
    let mut req = ehttp::Request::get(format!("{}/paper/board", relay_url()));
    req.headers = ehttp::Headers::new(&[("Accept", "application/json")]);
    ehttp::fetch(req, move |res| {
        let out = match res {
            Ok(r) => Ok((r.status, r.text().unwrap_or("").to_string())),
            Err(e) => Err(e),
        };
        if let Ok(mut p) = pending.lock() {
            *p = Some(out);
        }
    });
}

/// System: while the desk is alive the in-tab book opens nothing (its exits still run, so a
/// position opened before the desk appeared is still closed honestly).
pub fn gate_paper_book(time: Res<Time>, feed: Res<DeskFeed>, mut book: ResMut<PaperBook>) {
    let want = !feed.alive(time.elapsed_secs_f64());
    if book.enabled != want {
        book.enabled = want;
        if want {
            info!("paper book: desk gone, the in-tab book trades again");
        } else {
            info!("paper book: desk alive, the in-tab book stops opening positions");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{"desk_alive":true,"heartbeat_age_ms":4200,"received_ms":1792620000000,
        "started_ms":1792600000000,"sent_ms":1792619995800,"window_hours":24,"jev_mode":"live","jev_every_secs":60,
        "symbols":["XAGUSD","EURUSD"],"feed":"live, 2 of 2 pits ticking","ticks":1234,"reads":40,"jev_calls":42,"jev_failures":2,
        "traders":[
          {"trader":"Colin","archetype":"CONTROL","trades":3,"wins":1,"losses":2,"net_r":-0.8,"spread_paid_r":0.6,"max_drawdown_r":1.1,"open":null},
          {"trader":"Grace","archetype":"trend","trades":2,"wins":2,"losses":0,"net_r":2.4,"spread_paid_r":0.3,"max_drawdown_r":0.0,
           "open":{"symbol":"XAG/USD","side":"long","entry":57.84,"open_r":0.35,"opened_ms":1792619000000,"why":"Jev up with 20-bar drift +14 ticks",
                   "stop":57.74,"target":58.04},
           "day_r":1.3,"day_trades":1,"day_wins":1,"day_losses":0,"day_spread_paid_r":0.15,"day_max_drawdown_r":0.0,
           "last_trades":[{"symbol":"XAG/USD","side":"short","entry":57.9,"exit":57.77,"pnl_r":1.3,"opened_ms":1792610000000,"closed_ms":1792611000000,
                           "why_in":"Jev down, tiring buyers","why_out":"target"}]}
        ]}"#;

    #[test]
    fn a_relay_board_parses_and_ranks_best_first() {
        let b = parse_board(SAMPLE).unwrap();
        assert!(b.desk_alive);
        assert_eq!(b.heartbeat_age_ms, Some(4200));
        assert_eq!(b.traders.len(), 2);
        assert_eq!(
            b.ranking(),
            vec![1],
            "Grace traded today; Colin did not, so he is not listed"
        );
        let grace = &b.traders[1];
        assert_eq!(grace.archetype(), Some(Archetype::TrendRider));
        assert_eq!(grace.win_rate(), 1.0);
        assert_eq!(b.trader(Archetype::Control).unwrap().losses, 2);
        assert!(b.trader(Archetype::GapFiller).is_none());
    }

    #[test]
    fn the_day_block_last_trades_and_stop_target_ride_along_and_default_when_absent() {
        let b = parse_board(SAMPLE).unwrap();
        let grace = b.trader(Archetype::TrendRider).unwrap();
        assert_eq!(
            (grace.day_trades, grace.day_wins, grace.day_losses),
            (1, 1, 0)
        );
        assert!((grace.day_r - 1.3).abs() < 1e-6);
        assert_eq!(grace.last_trades.len(), 1);
        assert_eq!(grace.last_trades[0].why_out, "target");
        let open = grace.open.as_ref().unwrap();
        assert!((open.stop - 57.74).abs() < 1e-6 && open.target == Some(58.04));
        // Colin's row is in the old shape: everything new reads as zero / empty / None.
        let colin = b.trader(Archetype::Control).unwrap();
        assert_eq!(colin.day_trades, 0);
        assert!(colin.last_trades.is_empty() && colin.day_r == 0.0);
        let old_open: DeskOpen = serde_json::from_str(
            r#"{"symbol":"US30","side":"short","entry":1.0,"open_r":0.0,"opened_ms":1,"why":"x"}"#,
        )
        .unwrap();
        assert_eq!((old_open.stop, old_open.target), (0.0, None));
        // And the whole thing survives a serialise / parse cycle with the new fields intact.
        let again = parse_board(&serde_json::to_string(&b).unwrap()).unwrap();
        assert_eq!(again, b);
    }

    #[test]
    fn a_desk_trade_is_the_trade_minus_the_internals_with_long_reasons_cut() {
        let t = Trade {
            trader: "Priya",
            archetype: Archetype::IctLevelTrader,
            symbol: "US30",
            side: "short",
            entry: 52546.2,
            exit: 52554.2,
            opened_ms: 10,
            closed_ms: 20,
            pnl_ticks: -8.0,
            pnl_gbp: -8.0,
            pnl_r: -1.03,
            stop_ticks: 7.8,
            spread_ticks_in: 2.1,
            why_in: "x".repeat(300),
            why_out: "stop".into(),
            observation_hash: String::new(),
            read_source: None,
            genome: None,
        };
        let d = DeskTrade::from_trade(&t);
        assert_eq!(
            (d.symbol.as_str(), d.side.as_str(), d.opened_ms, d.closed_ms),
            ("US30", "short", 10, 20)
        );
        assert_eq!(
            d.why_in.chars().count(),
            WHY_WIRE_CHARS,
            "cut to the wire length including the ellipsis"
        );
        assert!(d.why_in.ends_with('\u{2026}'));
        assert_eq!(d.why_out, "stop");
    }

    #[test]
    fn a_dead_board_from_a_relay_without_a_desk_still_parses() {
        let b =
            parse_board(r#"{"desk_alive":false,"heartbeat_age_ms":null,"traders":[]}"#).unwrap();
        assert!(!b.desk_alive && b.traders.is_empty() && b.heartbeat_age_ms.is_none());
        assert!(parse_board("not json").is_err());
        assert!(parse_board("42").is_err());
    }

    #[test]
    fn the_wire_format_round_trips_from_the_desk_side() {
        let mut b = parse_board(SAMPLE).unwrap();
        b.desk_alive = false;
        b.heartbeat_age_ms = None;
        let text = serde_json::to_string(&b).unwrap();
        assert_eq!(parse_board(&text).unwrap(), b);
    }

    #[test]
    fn waiting_reasons_ride_the_heartbeat_and_default_to_empty() {
        let board: DeskBoard = serde_json::from_str(SAMPLE).unwrap();
        assert!(
            board
                .trader(Archetype::TrendRider)
                .unwrap()
                .waiting
                .is_empty(),
            "an older desk sends none"
        );
        let t = DeskTrader {
            waiting: vec![
                DeskPass::new("XAU/USD", &"x".repeat(300)),
                DeskPass::new("GBP/USD", "outside session hours"),
            ],
            ..Default::default()
        };
        let back: DeskTrader = serde_json::from_str(&serde_json::to_string(&t).unwrap()).unwrap();
        assert_eq!(back.waiting.len(), 2);
        assert_eq!(back.waiting[0].why.chars().count(), WHY_WIRE_CHARS);
        assert_eq!(back.waiting[1].why, "outside session hours");
    }

    #[test]
    fn panel_lines_carry_the_window_the_open_position_and_the_desk_stats() {
        let b = parse_board(SAMPLE).unwrap();
        let header = b.header_line();
        assert!(
            header.starts_with("DESK TODAY (since 00:00 UTC, all pits)")
                && header.contains("1 more waiting"),
            "{header}"
        );
        assert!(header.contains("heartbeat 4s ago"), "{header}");
        let row = b.row_line(0, &b.traders[1], None);
        assert!(
            row.starts_with(" 1. Grace  trend         +1.30R   1t 100% dd 0.0 sp"),
            "today's figures, not the 24 h ones: {row}"
        );
        assert!(row.ends_with("XAG/USD long +0.35R"), "{row}");
        assert!(
            b.row_line(0, &b.traders[1], Some(1.25))
                .ends_with("XAG/USD long +1.25R"),
            "a live mark overrides the heartbeat's"
        );
        let flat = b.row_line(1, &b.traders[0], None);
        assert!(flat.ends_with("flat") && flat.contains("CONTROL"), "{flat}");
        let detail = b.detail_lines(Archetype::TrendRider);
        assert!(
            detail[0].contains("Jev live every 60s on 2 pits")
                && detail[0].contains("42 Jev calls (2 failed)"),
            "{}",
            detail[0]
        );
        assert!(detail[2].starts_with("OPEN XAG/USD long"), "{:?}", detail);
        assert_eq!(
            b.detail_lines(Archetype::Control).len(),
            2,
            "no open line for a flat trader"
        );
    }

    #[test]
    fn alive_needs_a_fresh_good_answer_and_404_means_no_desk() {
        let mut f = DeskFeed::default();
        assert!(!f.alive(0.0));
        f.settle(Ok((200, SAMPLE.to_string())), 10.0);
        assert!(f.alive(10.0) && f.alive(10.0 + DESK_LOST_AFTER_SECS));
        assert!(
            !f.alive(10.0 + DESK_LOST_AFTER_SECS + 0.1),
            "the relay went quiet"
        );
        assert!(f.live_board(12.0).is_some() && f.live_board(60.0).is_none());

        f.settle(Ok((404, "{}".into())), 11.0);
        assert_eq!(f.last_error, "HTTP 404");
        assert!(
            f.alive(12.0),
            "an old board stays until it ages out, the error is just noted"
        );

        let mut g = DeskFeed::default();
        g.settle(
            Ok((200, r#"{"desk_alive":false,"traders":[]}"#.into())),
            5.0,
        );
        assert!(!g.alive(5.0), "the relay says the desk is gone");
        g.settle(Err("connection refused".into()), 6.0);
        assert_eq!(g.last_error, "connection refused");
    }
}
