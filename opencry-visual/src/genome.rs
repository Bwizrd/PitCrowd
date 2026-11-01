//! Genomes: paper traders made of data instead of code, so the evolving pit can spawn them.
//!
//! A genome is one idea on 15m bars: a signal (breakout, band, RSI, run, opening range,
//! yesterday's or last week's high and low, the Asian range, a moving-average cross, a squeeze,
//! a rejection wick, an engulfing bar, a session-open drive), followed or faded, up to two
//! filters (trend, volatility, the day's direction), an
//! entry window, and an exit (stop in multiples of the 15m ATR, target in R, a time stop in
//! bars), on a basket of symbols. `evolve/evolve.py` writes genomes, `genome_backtest` tests
//! them on edge-lab's years of 15m history, and the paper desk seats the survivors on the live
//! tape. The rule is this file in all three places, so a genome trades the history exactly as
//! it trades the tape.
//!
//! The backtest keeps the desk's discipline: bars are bid prices, a buy pays the spread on the
//! way in and a sell on the way out, the stop is never under `MIN_STOP_SPREADS` spreads, the
//! target never under `MIN_REWARD_TO_RISK` times the stop, entries only inside the desk's
//! entry window, flat by `FLAT_AT`, one position per trader at a time across its basket, and
//! the stop is taken first when one bar spans both stop and target.
//!
//! Nothing here touches Bevy, so the backtest binary and the desk share it as plain code.

use std::collections::HashMap;
use std::sync::RwLock;

use serde::{Deserialize, Serialize};

use crate::market::Instrument;
use crate::paper::{ENTRY_FROM, ENTRY_UNTIL, FLAT_AT, MIN_REWARD_TO_RISK, MIN_STOP_SPREADS};
use crate::pit_engine::{tick_size, Side};

pub const BAR_MS: i64 = 15 * 60 * 1000;
/// The ATR every stop is measured in.
pub const STOP_ATR: usize = 14;
/// The slow ATR the volatility filter compares it with (a day of 15m bars).
pub const SLOW_ATR: usize = 96;

/// One 15m bar of bid prices, opening at `t` (Unix ms).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bar {
    pub t: i64,
    pub o: f64,
    pub h: f64,
    pub l: f64,
    pub c: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Signal {
    /// The close clears the highest high (lowest low) of the previous `n` bars.
    Breakout { n: usize },
    /// The close is more than `k` ATR(n) from the n-bar average.
    Band { n: usize, k: f64 },
    /// RSI(n) above 100 - `lo` (below `lo`).
    Rsi { n: usize, lo: f64 },
    /// The `k`th close in a row higher (lower) than the one before.
    Run { k: usize },
    /// The first close outside the box drawn by the first `m` bars of London or New York.
    Orb { m: usize },
    /// Yesterday's high (low) taken out today, the first time. Followed: on a close beyond it.
    /// Faded: on a bar that trades through it and closes back inside.
    PrevDay,
    /// The same as `PrevDay` for last week's high (low); weeks start on Sunday, UTC.
    PrevWeek,
    /// The same for today's Asian range (00:00-07:00 UTC), from 07:00 on: the London
    /// breakout when followed, the sweep of the Asian range when faded.
    Asia,
    /// The `fast`-bar average crosses above (below) the `slow`-bar average on this bar.
    Cross { fast: usize, slow: usize },
    /// The previous bar was the narrowest of the last `k`, and this bar closes beyond it.
    Squeeze { k: usize },
    /// A rejection: a bar of at least half an ATR whose lower (upper) wick is at least `w` of its
    /// range and which closes in its top (bottom) third. Its direction is away from the wick.
    Wick { w: f64 },
    /// A bar of at least half an ATR whose body engulfs the opposite-coloured body before it.
    Engulf,
    /// At the close of the `k`th bar of London or New York, price is `x` ATR or more from the
    /// session's opening price.
    Drive { k: usize, x: f64 },
    /// One of edge-lab's strategies (`evolve/edge_lab.py`) run on `tf`-minute bars built from
    /// the 15m ones: the 15m bar that closes the `tf` bar where its position turns long
    /// (short) from anything else. The entries are computed by edge-lab's own Python and
    /// handed in through `Features::set_entries`; only the entry is the strategy's.
    EdgeLab {
        key: String,
        #[serde(default = "fifteen")]
        tf: u32,
    },
}

fn fifteen() -> u32 {
    15
}

impl Signal {
    /// The name an edge-lab signal's entries go by: its file, and its slot in `Features`.
    pub fn edge_lab_id(&self) -> Option<String> {
        match self {
            Signal::EdgeLab { key, tf } => Some(format!("{key}@{tf}")),
            _ => None,
        }
    }
}

/// The signal's own direction is "with the move"; a fader takes the other side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Follow,
    Fade,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Filter {
    /// The close is above (below) the n-bar average, and the trade goes with that (`with`) or
    /// against it.
    Trend { n: usize, with: bool },
    /// ATR(14) above (`high`) or below the day-long ATR(96).
    Vol { high: bool },
    /// The close is above (below) today's first open (UTC day), and the trade goes with that
    /// (`with`) or against it.
    Day { with: bool },
}

/// UTC hours in which the genome may enter, judged at the bar's close.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Window {
    All,
    London,
    NewYork,
    Core,
    LondonOpen,
    NewYorkOpen,
}

impl Window {
    pub fn contains(self, minute: i64) -> bool {
        let (a, b) = match self {
            Window::All => (ENTRY_FROM, ENTRY_UNTIL),
            Window::London => (7 * 60, 11 * 60),
            Window::NewYork => (13 * 60 + 30, 16 * 60 + 30),
            Window::Core => (8 * 60, 16 * 60 + 30),
            Window::LondonOpen => (7 * 60, 8 * 60),
            Window::NewYorkOpen => (13 * 60 + 30, 14 * 60 + 30),
        };
        minute >= a && minute < b && minute >= ENTRY_FROM && minute < ENTRY_UNTIL
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Exit {
    /// The stop, in ATR(14) of 15m bars (then floored at `MIN_STOP_SPREADS` spreads).
    pub stop_atr: f64,
    /// The target, in multiples of the stop (floored at `MIN_REWARD_TO_RISK`).
    pub target_r: f64,
    /// Out at market after this many bars.
    pub max_bars: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Genome {
    pub id: String,
    pub name: String,
    pub signal: Signal,
    pub mode: Mode,
    #[serde(default)]
    pub filters: Vec<Filter>,
    pub window: Window,
    pub exit: Exit,
    /// Tape symbols without the slash: "EURUSD", "XAUUSD", ...
    pub symbols: Vec<String>,
    /// A Jev question asked before every entry, for a genome's Jev twin. Not used yet.
    #[serde(default)]
    pub jev: Option<String>,
    #[serde(default)]
    pub parent: Option<String>,
    /// "show" for a seat on the show floor (README, "Show floor"): seated on a looser rule, kept
    /// apart from the gate's seats, never counted toward graduation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub floor: Option<String>,
    /// A holder: an edge-lab strategy on its own 4h or daily bars, its own entries and exits,
    /// held overnight, on one symbol (`evolve/holders.py`). `exit.stop_atr` is one R in ATR(14)
    /// of those bars and the safety stop is `HOLD_SAFETY_R` of them; target and time stop unused.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub holds: bool,
    /// An adapter: a set of tactics (ordinary genomes) it switches between once a day, choosing
    /// by their last 20 days (`Adapt::Code`) or by asking Jev (`Adapt::Jev`). Its own rule fields
    /// are the tactic in use.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tactics: Vec<Genome>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adapt: Option<Adapt>,
}

/// How an adapter picks its tactic for the day.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Adapt {
    /// The tactic with the best R per trade over the last `ADAPT_DAYS` days of the desk's bars.
    Code,
    /// The tactic Jev rates likeliest to make money today, shown the same records and the market.
    Jev,
}

/// The safety stop of a holder, in its R.
pub const HOLD_SAFETY_R: f64 = 5.0;
/// Days of the desk's own bars an adapter scores its tactics on.
pub const ADAPT_DAYS: i64 = 20;
/// When an adapter chooses (UTC minute of day), before London opens.
pub const ADAPT_AT: i64 = 6 * 60 + 45;

impl Genome {
    pub fn describe(&self) -> String {
        if self.holds {
            let (key, tf) = match &self.signal {
                Signal::EdgeLab { key, tf } => (key.as_str(), *tf),
                _ => ("?", 0),
            };
            let bars = if tf >= 1440 { "daily".to_string() } else { format!("{}h", tf / 60) };
            return format!(
                "holder: edge-lab strategy {key} on {bars} bars, its own entries and exits, held overnight; 1 R = {} ATR of those bars, safety stop {HOLD_SAFETY_R} R; {}",
                self.exit.stop_atr,
                self.symbols.join(" ")
            );
        }
        if let Some(a) = self.adapt {
            return format!(
                "adapter ({}): picks one of {} tactics every morning by {}; now: {}",
                if a == Adapt::Jev { "Jev" } else { "code" },
                self.tactics.len(),
                if a == Adapt::Jev { "asking Jev" } else { "their last 20 days" },
                self.rule_text()
            );
        }
        self.rule_text()
    }

    /// The rule alone, in words.
    pub fn rule_text(&self) -> String {
        let sig = match &self.signal {
            Signal::Breakout { n } => format!("{n}-bar breakout"),
            Signal::Band { n, k } => format!("close {k} ATR off the {n}-bar mean"),
            Signal::Rsi { n, lo } => format!("RSI({n}) beyond {lo}/{}", 100.0 - lo),
            Signal::Run { k } => format!("{k} closes in a row"),
            Signal::Orb { m } => format!("{}-minute opening range", m * 15),
            Signal::PrevDay => "yesterday's high/low".into(),
            Signal::PrevWeek => "last week's high/low".into(),
            Signal::Asia => "the Asian range".into(),
            Signal::Cross { fast, slow } => format!("{fast}/{slow}-bar average cross"),
            Signal::Squeeze { k } => format!("break of the narrowest bar in {k}"),
            Signal::Wick { w } => format!("{:.0}% rejection wick", w * 100.0),
            Signal::Engulf => "engulfing bar".into(),
            Signal::Drive { k, x } => format!("{x} ATR drive in the first {} minutes", k * 15),
            Signal::EdgeLab { key, tf } => format!("edge-lab strategy {key} on {tf}m bars"),
        };
        let mode = match self.mode {
            Mode::Follow => "follow",
            Mode::Fade => "fade",
        };
        let filters: Vec<String> = self
            .filters
            .iter()
            .map(|f| match f {
                Filter::Trend { n, with } => format!("{} the {n}-bar trend", if *with { "with" } else { "against" }),
                Filter::Vol { high } => format!("{} volatility", if *high { "high" } else { "low" }),
                Filter::Day { with } => format!("{} the day's move", if *with { "with" } else { "against" }),
            })
            .collect();
        let filt = if filters.is_empty() { String::new() } else { format!(", {}", filters.join(", ")) };
        format!(
            "15m {mode} {sig}{filt}, {:?} hours; stop {} ATR, target {} R, out after {} bars; {}",
            self.window,
            self.exit.stop_atr,
            self.exit.target_r,
            self.exit.max_bars,
            self.symbols.join(" ")
        )
    }
}

pub fn instrument_of(symbol: &str) -> Option<Instrument> {
    Instrument::ALL.into_iter().find(|i| i.symbol().replace('/', "") == symbol)
}

/// Typical live spread in ticks (median of the desk's fills, 23-24 Oct 2026); the backtest's
/// cost. The desk charges the real spread of the moment.
pub fn typical_spread_ticks(inst: Instrument) -> f64 {
    match inst {
        Instrument::Gold => 1.9,
        Instrument::Silver => 4.0,
        Instrument::GbpUsd => 0.8,
        Instrument::EurUsd => 0.5,
        Instrument::Uk100 => 2.6,
        Instrument::Sp500 => 1.6,
        Instrument::Nasdaq => 4.4,
        Instrument::Us30 => 2.1,
        Instrument::Bitcoin => 3.0,
    }
}

pub fn minute_of_day(ms: i64) -> i64 {
    ms.div_euclid(60_000).rem_euclid(1440)
}

fn day_of(ms: i64) -> i64 {
    ms.div_euclid(86_400_000)
}

/// Weeks that start on Sunday (UTC), so the Sunday-evening open belongs to the week it opens.
fn week_of(ms: i64) -> i64 {
    (day_of(ms) + 4).div_euclid(7)
}

// ---------------------------------------------------------------------------
// Jev twins: the same genome, asking Jev one question before every entry
// ---------------------------------------------------------------------------

/// The twin's question, by the genome's mode. A follower asks whether the move it is joining
/// has legs; a fader asks whether the move it is fading is spent. Either way a yes is a
/// reason to take the trade, so the twin is the genome minus the entries Jev doubts.
pub fn jev_question(mode: Mode) -> &'static str {
    match mode {
        Mode::Follow => "Is the move this trade joins strong and clean, likely to carry on for the next few bars rather than tiring or reversing?",
        Mode::Fade => "Is the move this trade fades tiring and likely to turn: stretched, slowing, or already being rejected?",
    }
}

/// Bars a twin shows Jev.
pub const JEV_BARS: usize = 24;

/// What Jev sees: the last `JEV_BARS` 15m bars up to the decision, with no symbol and no
/// dates, prices rebased so the last close is 1000.00 (so a chart cannot be recognised, the
/// lesson of htf_jev), the trade's side, and the hour of day.
pub fn jev_state(bars: &[Bar], side: Side) -> String {
    let n = bars.len();
    let from = n.saturating_sub(JEV_BARS);
    let last = bars[n - 1].c;
    let k = if last != 0.0 { 1000.0 / last } else { 1.0 };
    let rows: Vec<String> = bars[from..]
        .iter()
        .enumerate()
        .map(|(i, b)| format!("{:>2}  {:.2}  {:.2}  {:.2}  {:.2}", i + 1, b.o * k, b.h * k, b.l * k, b.c * k))
        .collect();
    let hour = minute_of_day(bars[n - 1].t + BAR_MS) / 60;
    format!(
        "A trade on a 15-minute chart, decided at the close of the last bar ({hour:02}:00 UTC hour).\nTrade: {}.\nPrices are rebased so the last close is 1000.00.\nBars, oldest first: number, open, high, low, close\n{}",
        if side == Side::Buy { "BUY (betting price goes up)" } else { "SELL (betting price goes down)" },
        rows.join("\n")
    )
}

/// The twin of a genome: same rule, id and name with "-J", and the question set.
pub fn twin_of(g: &Genome) -> Genome {
    Genome { id: format!("{}-J", g.id), name: format!("{}-J", g.name), jev: Some(jev_question(g.mode).to_string()), ..g.clone() }
}

// ---------------------------------------------------------------------------
// Seats: the genomes the desk has registered, by index, for `paper::Archetype::Genome`
// ---------------------------------------------------------------------------

/// A registered genome. The strings are leaked: the book's trader names are `&'static str`,
/// and a desk registers a few hundred genomes over weeks, a few kilobytes in all.
#[derive(Debug, Clone)]
pub struct Seat {
    /// The rule, on the desk. A page registers genomes from the board for display only and
    /// has their name and description but not the rule.
    pub genome: Option<Genome>,
    pub id: &'static str,
    pub name: &'static str,
    pub creed: &'static str,
    /// The board's short label: "genome", "show", "holder", "adapter", "adapter+J".
    pub tag: &'static str,
}

/// The board label of a genome, from what kind of seat it is.
pub fn tag_of(g: &Genome) -> &'static str {
    match (g.adapt, g.holds, g.floor.as_deref()) {
        (Some(Adapt::Jev), _, _) => "adapter+J",
        (Some(Adapt::Code), _, _) => "adapter",
        (None, true, _) => "holder",
        (None, false, Some("show")) => "show",
        (None, false, Some("manual")) => "manual",
        _ => "genome",
    }
}

static SEATS: RwLock<Vec<Seat>> = RwLock::new(Vec::new());

/// The seat index for a genome, registering it the first time its id is seen.
pub fn register(g: &Genome) -> u16 {
    let mut seats = SEATS.write().unwrap_or_else(|e| e.into_inner());
    if let Some(i) = seats.iter().position(|s| s.id == g.id) {
        return i as u16;
    }
    let leak = |s: String| -> &'static str { Box::leak(s.into_boxed_str()) };
    seats.push(Seat { genome: Some(g.clone()), id: leak(g.id.clone()), name: leak(g.name.clone()), creed: leak(g.describe()), tag: tag_of(g) });
    (seats.len() - 1) as u16
}

/// An adapter switches tactic: the seat keeps its id and name and takes the tactic's rule.
/// Returns false if the seat is not an adapter or the tactic is out of range.
pub fn set_tactic(i: u16, k: usize) -> bool {
    let mut seats = SEATS.write().unwrap_or_else(|e| e.into_inner());
    let Some(seat) = seats.get_mut(i as usize) else { return false };
    let Some(g) = seat.genome.as_mut() else { return false };
    let Some(t) = g.tactics.get(k).cloned() else { return false };
    g.signal = t.signal;
    g.mode = t.mode;
    g.filters = t.filters;
    g.window = t.window;
    g.exit = t.exit;
    g.symbols = t.symbols;
    seat.creed = Box::leak(g.describe().into_boxed_str());
    true
}

/// Which of its tactics an adapter is using now, by its rule fields.
pub fn tactic_in_use(g: &Genome) -> Option<usize> {
    g.tactics.iter().position(|t| t.signal == g.signal && t.mode == g.mode && t.filters == g.filters && t.window == g.window && t.exit == g.exit && t.symbols == g.symbols)
}

/// Register a genome the page knows only by id, name and description (from the desk's
/// board), for its avatar and its rows. The same id always gets the same seat.
pub fn register_display(id: &str, name: &str, creed: &str, tag: Option<&str>) -> u16 {
    let mut seats = SEATS.write().unwrap_or_else(|e| e.into_inner());
    let leak = |s: &str| -> &'static str { Box::leak(s.to_string().into_boxed_str()) };
    if let Some(i) = seats.iter().position(|s| s.id == id) {
        // A seat can be renamed (the trade ticket's trader); follow the board.
        if seats[i].name != name && !name.is_empty() {
            seats[i].name = leak(name);
        }
        return i as u16;
    }
    let tag = match tag {
        Some("holder") => "holder",
        Some("adapter+J") => "adapter+J",
        Some("adapter") => "adapter",
        Some("show") => "show",
        Some("manual") => "manual",
        _ => "genome",
    };
    seats.push(Seat { genome: None, id: leak(id), name: leak(name), creed: leak(if creed.is_empty() { "A genome from the evolving pit." } else { creed }), tag });
    (seats.len() - 1) as u16
}

/// The seat of a registered genome, by its trader name.
pub fn seat_by_name(name: &str) -> Option<u16> {
    SEATS.read().unwrap_or_else(|e| e.into_inner()).iter().position(|s| s.name == name).map(|i| i as u16)
}

pub fn seat(i: u16) -> Option<Seat> {
    SEATS.read().unwrap_or_else(|e| e.into_inner()).get(i as usize).cloned()
}

/// Give a seat a new display name (the trade ticket's trader, when its user renames it). The id,
/// and so every logged trade, stays the same.
pub fn rename(i: u16, name: &str) {
    let mut seats = SEATS.write().unwrap_or_else(|e| e.into_inner());
    if let Some(seat) = seats.get_mut(i as usize) {
        if seat.name != name {
            seat.name = Box::leak(name.to_string().into_boxed_str());
        }
    }
}

pub fn seat_name(i: u16) -> &'static str {
    SEATS.read().unwrap_or_else(|e| e.into_inner()).get(i as usize).map(|s| s.name).unwrap_or("genome")
}

pub fn seat_id(i: u16) -> Option<&'static str> {
    SEATS.read().unwrap_or_else(|e| e.into_inner()).get(i as usize).map(|s| s.id)
}

pub fn seat_tag(i: u16) -> &'static str {
    SEATS.read().unwrap_or_else(|e| e.into_inner()).get(i as usize).map(|s| s.tag).unwrap_or("genome")
}

pub fn seat_creed(i: u16) -> &'static str {
    SEATS.read().unwrap_or_else(|e| e.into_inner()).get(i as usize).map(|s| s.creed).unwrap_or("A genome from the evolving pit.")
}

/// A colour of its own for each genome, from its id: golden-angle hues, never grey.
pub fn seat_rgb(i: u16) -> (f32, f32, f32) {
    let h = (i as f32 * 137.508) % 360.0 / 60.0;
    let x = 1.0 - (h % 2.0 - 1.0).abs();
    let (r, g, b) = match h as u32 {
        0 => (1.0, x, 0.0),
        1 => (x, 1.0, 0.0),
        2 => (0.0, 1.0, x),
        3 => (0.0, x, 1.0),
        4 => (x, 0.0, 1.0),
        _ => (1.0, 0.0, x),
    };
    (0.35 + 0.6 * r, 0.35 + 0.6 * g, 0.35 + 0.6 * b)
}

// ---------------------------------------------------------------------------
// Features, computed once per symbol and cached by parameter
// ---------------------------------------------------------------------------

/// One symbol's bars and the indicator series genomes have asked for, built on first use.
pub struct Features {
    pub bars: Vec<Bar>,
    sma: HashMap<usize, Vec<f64>>,
    atr: HashMap<usize, Vec<f64>>,
    rsi: HashMap<usize, Vec<f64>>,
    hh: HashMap<usize, Vec<f64>>,
    ll: HashMap<usize, Vec<f64>>,
    orb: HashMap<usize, Vec<i8>>,
    runs: Option<Vec<i32>>,
    prev_day: Option<(Vec<i8>, Vec<i8>)>,
    prev_week: Option<(Vec<i8>, Vec<i8>)>,
    asia: Option<(Vec<i8>, Vec<i8>)>,
    drive: HashMap<usize, Vec<f64>>,
    day_open: Option<Vec<f64>>,
    /// edge-lab entries by strategy key: bar index -> +1 / -1, only where it enters.
    entries: HashMap<String, HashMap<usize, i8>>,
    /// The spread of each bar (ask minus bid at its open, in price), when the history has one;
    /// otherwise the backtest charges `typical_spread_ticks`.
    pub spreads: Option<Vec<f64>>,
}

impl Features {
    pub fn new(bars: Vec<Bar>) -> Self {
        Features { bars, sma: HashMap::new(), atr: HashMap::new(), rsi: HashMap::new(), hh: HashMap::new(), ll: HashMap::new(), orb: HashMap::new(), runs: None, prev_day: None, prev_week: None, asia: None, drive: HashMap::new(), day_open: None, entries: HashMap::new(), spreads: None }
    }

    fn sma(&mut self, n: usize) -> &Vec<f64> {
        let bars = &self.bars;
        self.sma.entry(n).or_insert_with(|| {
            let mut out = vec![f64::NAN; bars.len()];
            let mut sum = 0.0;
            for i in 0..bars.len() {
                sum += bars[i].c;
                if i >= n {
                    sum -= bars[i - n].c;
                }
                if i + 1 >= n {
                    out[i] = sum / n as f64;
                }
            }
            out
        })
    }

    /// Mean true range over `n` bars.
    fn atr(&mut self, n: usize) -> &Vec<f64> {
        let bars = &self.bars;
        self.atr.entry(n).or_insert_with(|| {
            let tr: Vec<f64> = (0..bars.len())
                .map(|i| {
                    let b = bars[i];
                    if i == 0 { b.h - b.l } else { (b.h - b.l).max((b.h - bars[i - 1].c).abs()).max((b.l - bars[i - 1].c).abs()) }
                })
                .collect();
            let mut out = vec![f64::NAN; bars.len()];
            let mut sum = 0.0;
            for i in 0..bars.len() {
                sum += tr[i];
                if i >= n {
                    sum -= tr[i - n];
                }
                if i + 1 >= n {
                    out[i] = sum / n as f64;
                }
            }
            out
        })
    }

    /// Wilder's RSI.
    fn rsi(&mut self, n: usize) -> &Vec<f64> {
        let bars = &self.bars;
        self.rsi.entry(n).or_insert_with(|| {
            let mut out = vec![f64::NAN; bars.len()];
            let (mut up, mut dn) = (0.0, 0.0);
            for i in 1..bars.len() {
                let d = bars[i].c - bars[i - 1].c;
                let (u, w) = (d.max(0.0), (-d).max(0.0));
                if i <= n {
                    up += u / n as f64;
                    dn += w / n as f64;
                } else {
                    up = (up * (n as f64 - 1.0) + u) / n as f64;
                    dn = (dn * (n as f64 - 1.0) + w) / n as f64;
                }
                if i >= n {
                    out[i] = if dn == 0.0 { 100.0 } else { 100.0 - 100.0 / (1.0 + up / dn) };
                }
            }
            out
        })
    }

    /// Highest high of the `n` bars before this one.
    fn hh(&mut self, n: usize) -> &Vec<f64> {
        let bars = &self.bars;
        self.hh.entry(n).or_insert_with(|| rolling(bars, n, |b| b.h, f64::max))
    }

    fn ll(&mut self, n: usize) -> &Vec<f64> {
        let bars = &self.bars;
        self.ll.entry(n).or_insert_with(|| rolling(bars, n, |b| b.l, f64::min))
    }

    /// +1 / -1 on the first close above / below the opening box of London or New York, built
    /// from the session's first `m` bars, at most once per session.
    fn orb(&mut self, m: usize) -> &Vec<i8> {
        let bars = &self.bars;
        self.orb.entry(m).or_insert_with(|| {
            let mut out = vec![0i8; bars.len()];
            let mut box_: Option<(i64, i64, f64, f64, usize, bool)> = None; // (day, open minute, hi, lo, bars in, fired)
            for i in 0..bars.len() {
                let b = bars[i];
                let (day, start) = (day_of(b.t), minute_of_day(b.t));
                for open in [7 * 60, 13 * 60 + 30] {
                    if start == open {
                        box_ = Some((day, open, b.h, b.l, 1, false));
                    }
                }
                let Some((d, open, hi, lo, k, fired)) = box_ else { continue };
                if d != day || start >= open + 6 * 60 {
                    box_ = None;
                    continue;
                }
                if start == open {
                    continue;
                }
                if k < m {
                    box_ = Some((d, open, hi.max(b.h), lo.min(b.l), k + 1, fired));
                    continue;
                }
                if !fired && (b.c > hi || b.c < lo) {
                    out[i] = if b.c > hi { 1 } else { -1 };
                    box_ = Some((d, open, hi, lo, k, true));
                }
            }
            out
        })
    }

    /// Length of the run of higher (+) or lower (-) closes ending at each bar.
    fn runs(&mut self) -> &Vec<i32> {
        let bars = &self.bars;
        self.runs.get_or_insert_with(|| {
            let mut out = vec![0i32; bars.len()];
            for i in 1..bars.len() {
                let p = out[i - 1];
                out[i] = if bars[i].c > bars[i - 1].c {
                    if p > 0 { p + 1 } else { 1 }
                } else if bars[i].c < bars[i - 1].c {
                    if p < 0 { p - 1 } else { -1 }
                } else {
                    0
                };
            }
            out
        })
    }

    /// Yesterday's high or low taken out, the first time today: (closed beyond, traded
    /// through and closed back inside), each +1 at the high and -1 at the low.
    fn prev_day(&mut self) -> &(Vec<i8>, Vec<i8>) {
        let bars = &self.bars;
        self.prev_day.get_or_insert_with(|| {
            let (mut follow, mut fade) = (vec![0i8; bars.len()], vec![0i8; bars.len()]);
            let (mut day, mut hi, mut lo) = (i64::MIN, f64::MIN, f64::MAX);
            let mut prev: Option<(f64, f64)> = None;
            let mut seen = [false; 4];
            for i in 0..bars.len() {
                let b = bars[i];
                let d = day_of(b.t);
                if d != day {
                    if day != i64::MIN {
                        prev = Some((hi, lo));
                    }
                    day = d;
                    hi = b.h;
                    lo = b.l;
                    seen = [false; 4];
                } else {
                    hi = hi.max(b.h);
                    lo = lo.min(b.l);
                }
                let Some((ph, pl)) = prev else { continue };
                if b.c > ph && !seen[0] {
                    follow[i] = 1;
                    seen[0] = true;
                }
                if b.c < pl && !seen[1] {
                    follow[i] = -1;
                    seen[1] = true;
                }
                if b.h > ph && b.c < ph && !seen[2] {
                    fade[i] = 1;
                    seen[2] = true;
                }
                if b.l < pl && b.c > pl && !seen[3] {
                    fade[i] = -1;
                    seen[3] = true;
                }
            }
            (follow, fade)
        })
    }
}

impl Features {
    /// Hand in an edge-lab strategy's entries for these bars (index -> +1 / -1).
    pub fn set_entries(&mut self, key: &str, at: HashMap<usize, i8>) {
        self.entries.insert(key.to_string(), at);
    }

    pub fn has_entries(&self, key: &str) -> bool {
        self.entries.contains_key(key)
    }

    /// Last week's high or low taken out, the first time this week (as `prev_day`).
    fn prev_week(&mut self) -> &(Vec<i8>, Vec<i8>) {
        let bars = &self.bars;
        self.prev_week.get_or_insert_with(|| {
            let mut refs = vec![None; bars.len()];
            let (mut week, mut hi, mut lo, mut prev) = (i64::MIN, f64::MIN, f64::MAX, None);
            for (i, b) in bars.iter().enumerate() {
                let w = week_of(b.t);
                if w != week {
                    if week != i64::MIN {
                        prev = Some((hi, lo));
                    }
                    (week, hi, lo) = (w, b.h, b.l);
                } else {
                    (hi, lo) = (hi.max(b.h), lo.min(b.l));
                }
                refs[i] = prev.map(|(h, l)| (w, h, l));
            }
            takeouts(bars, &refs)
        })
    }

    /// Today's Asian range (bars opening 00:00-07:00 UTC) taken out, the first time, from 07:00.
    fn asia(&mut self) -> &(Vec<i8>, Vec<i8>) {
        let bars = &self.bars;
        self.asia.get_or_insert_with(|| {
            let mut refs = vec![None; bars.len()];
            let (mut day, mut hi, mut lo) = (i64::MIN, f64::MIN, f64::MAX);
            for (i, b) in bars.iter().enumerate() {
                let (d, m) = (day_of(b.t), minute_of_day(b.t));
                if d != day {
                    (day, hi, lo) = (d, f64::MIN, f64::MAX);
                }
                if m < 7 * 60 {
                    (hi, lo) = (hi.max(b.h), lo.min(b.l));
                } else if hi > lo {
                    refs[i] = Some((d, hi, lo));
                }
            }
            takeouts(bars, &refs)
        })
    }

    /// Price minus the session's opening price, at the close of the `k`th bar of London or
    /// New York; NaN on every other bar.
    fn drive(&mut self, k: usize) -> &Vec<f64> {
        let bars = &self.bars;
        self.drive.entry(k).or_insert_with(|| {
            let mut out = vec![f64::NAN; bars.len()];
            let mut open: Option<(i64, i64, f64)> = None; // (day, open minute, price)
            for (i, b) in bars.iter().enumerate() {
                let (d, m) = (day_of(b.t), minute_of_day(b.t));
                if m == 7 * 60 || m == 13 * 60 + 30 {
                    open = Some((d, m, b.o));
                }
                if let Some((od, om, px)) = open {
                    if od == d && m == om + (k as i64 - 1) * 15 {
                        out[i] = b.c - px;
                    }
                }
            }
            out
        })
    }

    /// The first open of each bar's UTC day.
    fn day_open(&mut self) -> &Vec<f64> {
        let bars = &self.bars;
        self.day_open.get_or_insert_with(|| {
            let (mut day, mut px) = (i64::MIN, f64::NAN);
            bars.iter()
                .map(|b| {
                    if day_of(b.t) != day {
                        (day, px) = (day_of(b.t), b.o);
                    }
                    px
                })
                .collect()
        })
    }
}

/// For each bar, a reference high and low (with the period it belongs to): the first close
/// beyond each (+1 high, -1 low) and the first bar to trade through each and close back
/// inside, once per side per period. The shape `prev_day` has, for any reference.
fn takeouts(bars: &[Bar], refs: &[Option<(i64, f64, f64)>]) -> (Vec<i8>, Vec<i8>) {
    let (mut follow, mut fade) = (vec![0i8; bars.len()], vec![0i8; bars.len()]);
    let (mut period, mut seen) = (i64::MIN, [false; 4]);
    for (i, b) in bars.iter().enumerate() {
        let Some((p, hi, lo)) = refs[i] else { continue };
        if p != period {
            (period, seen) = (p, [false; 4]);
        }
        if b.c > hi && !seen[0] {
            follow[i] = 1;
            seen[0] = true;
        }
        if b.c < lo && !seen[1] {
            follow[i] = -1;
            seen[1] = true;
        }
        if b.h > hi && b.c < hi && !seen[2] {
            fade[i] = 1;
            seen[2] = true;
        }
        if b.l < lo && b.c > lo && !seen[3] {
            fade[i] = -1;
            seen[3] = true;
        }
    }
    (follow, fade)
}

fn rolling(bars: &[Bar], n: usize, f: impl Fn(&Bar) -> f64, pick: fn(f64, f64) -> f64) -> Vec<f64> {
    // n is at most a few hundred, so the plain window is fast enough and obviously right.
    let mut out = vec![f64::NAN; bars.len()];
    for i in n..bars.len() {
        out[i] = bars[i - n..i].iter().map(&f).fold(f(&bars[i - n]), pick);
    }
    out
}

/// What a genome wants to do at the close of bar `i`: a side and the stop distance in price
/// (before the spread floor), with the reason, or None.
pub fn evaluate(g: &Genome, f: &mut Features, i: usize) -> Option<(Side, f64, String)> {
    let b = *f.bars.get(i)?;
    if !g.window.contains(minute_of_day(b.t + BAR_MS)) {
        return None;
    }
    let atr = *f.atr(STOP_ATR).get(i)?;
    if !atr.is_finite() || atr <= 0.0 {
        return None;
    }
    let up = |x: bool| if x { 1i8 } else { -1i8 };
    let (raw, why): (i8, String) = match &g.signal {
        Signal::Breakout { n } => {
            let (hh, ll) = (f.hh(*n)[i], f.ll(*n)[i]);
            if !hh.is_finite() {
                return None;
            }
            if b.c > hh {
                (1, format!("closed above the {n}-bar high"))
            } else if b.c < ll {
                (-1, format!("closed below the {n}-bar low"))
            } else {
                return None;
            }
        }
        Signal::Band { n, k } => {
            let (m, a) = (f.sma(*n)[i], f.atr(*n)[i]);
            if !m.is_finite() || !a.is_finite() {
                return None;
            }
            let z = (b.c - m) / a;
            if z.abs() <= *k {
                return None;
            }
            (up(z > 0.0), format!("close {z:+.1} ATR from the {n}-bar mean"))
        }
        Signal::Rsi { n, lo } => {
            let r = f.rsi(*n)[i];
            if !r.is_finite() {
                return None;
            }
            if r > 100.0 - lo {
                (1, format!("RSI({n}) {r:.0}"))
            } else if r < *lo {
                (-1, format!("RSI({n}) {r:.0}"))
            } else {
                return None;
            }
        }
        Signal::Run { k } => {
            let r = f.runs()[i];
            if r.unsigned_abs() as usize != *k {
                return None;
            }
            (up(r > 0), format!("{k} closes in a row {}", if r > 0 { "higher" } else { "lower" }))
        }
        Signal::Orb { m } => {
            let d = f.orb(*m)[i];
            if d == 0 {
                return None;
            }
            (d, format!("first close {} the {}-minute opening range", if d > 0 { "above" } else { "below" }, m * 15))
        }
        Signal::PrevDay => {
            let (follow, fade) = f.prev_day();
            let d = if g.mode == Mode::Follow { follow[i] } else { fade[i] };
            if d == 0 {
                return None;
            }
            let what = if g.mode == Mode::Follow { "closed beyond" } else { "swept and closed back inside" };
            (d, format!("{what} yesterday's {}", if d > 0 { "high" } else { "low" }))
        }
        Signal::PrevWeek | Signal::Asia => {
            let weekly = g.signal == Signal::PrevWeek;
            let (follow, fade) = if weekly { f.prev_week() } else { f.asia() };
            let d = if g.mode == Mode::Follow { follow[i] } else { fade[i] };
            if d == 0 {
                return None;
            }
            let what = if g.mode == Mode::Follow { "closed beyond" } else { "swept and closed back inside" };
            let level = if weekly { "last week's" } else { "the Asian range's" };
            (d, format!("{what} {level} {}", if d > 0 { "high" } else { "low" }))
        }
        Signal::Cross { fast, slow } => {
            if i == 0 {
                return None;
            }
            let (a0, a1) = (f.sma(*fast)[i - 1], f.sma(*fast)[i]);
            let (b0, b1) = (f.sma(*slow)[i - 1], f.sma(*slow)[i]);
            if !(a0.is_finite() && b0.is_finite()) {
                return None;
            }
            let (was, is) = (a0 > b0, a1 > b1);
            if was == is {
                return None;
            }
            (up(is), format!("{fast}-bar average crossed {} the {slow}-bar", if is { "above" } else { "below" }))
        }
        Signal::Squeeze { k } => {
            if *k < 2 || i < *k {
                return None;
            }
            let p = f.bars[i - 1];
            let narrowest = f.bars[i - *k..i - 1].iter().all(|x| x.h - x.l > p.h - p.l);
            if !narrowest {
                return None;
            }
            if b.c > p.h {
                (1, format!("closed above the narrowest bar in {k}"))
            } else if b.c < p.l {
                (-1, format!("closed below the narrowest bar in {k}"))
            } else {
                return None;
            }
        }
        Signal::Wick { w } => {
            let range = b.h - b.l;
            if range < 0.5 * atr {
                return None;
            }
            let (lower, upper) = (b.o.min(b.c) - b.l, b.h - b.o.max(b.c));
            if lower >= w * range && b.c >= b.l + range * 2.0 / 3.0 {
                (1, format!("rejected the low, wick {:.0}% of the bar", lower / range * 100.0))
            } else if upper >= w * range && b.c <= b.l + range / 3.0 {
                (-1, format!("rejected the high, wick {:.0}% of the bar", upper / range * 100.0))
            } else {
                return None;
            }
        }
        Signal::Engulf => {
            if i == 0 || b.h - b.l < 0.5 * atr {
                return None;
            }
            let p = f.bars[i - 1];
            let (body, prev) = (b.c - b.o, p.c - p.o);
            if body == 0.0 || prev == 0.0 || (body > 0.0) == (prev > 0.0) {
                return None;
            }
            if b.o.min(b.c) > p.o.min(p.c) || b.o.max(b.c) < p.o.max(p.c) {
                return None;
            }
            (up(body > 0.0), format!("{} engulfing bar", if body > 0.0 { "bullish" } else { "bearish" }))
        }
        Signal::EdgeLab { .. } => {
            let id = g.signal.edge_lab_id().unwrap_or_default();
            let d = f.entries.get(&id).and_then(|m| m.get(&i)).copied().unwrap_or(0);
            if d == 0 {
                return None;
            }
            (d, format!("edge-lab {id} turned {}", if d > 0 { "long" } else { "short" }))
        }
        Signal::Drive { k, x } => {
            let mv = f.drive(*k)[i];
            if !mv.is_finite() || mv.abs() < x * atr {
                return None;
            }
            (up(mv > 0.0), format!("{:+.1} ATR from the session open after {} minutes", mv / atr, k * 15))
        }
    };
    // A faded PrevDay (PrevWeek, Asia) already picked the sweep bar; its trade is against the take-out.
    let dir = if g.mode == Mode::Fade { -raw } else { raw };
    for flt in &g.filters {
        match flt {
            Filter::Trend { n, with } => {
                let m = f.sma(*n)[i];
                if !m.is_finite() {
                    return None;
                }
                let trend = up(b.c > m);
                if (trend == dir) != *with {
                    return None;
                }
            }
            Filter::Vol { high } => {
                let (fast, slow) = (f.atr(STOP_ATR)[i], f.atr(SLOW_ATR)[i]);
                if !slow.is_finite() || (fast > slow) != *high {
                    return None;
                }
            }
            Filter::Day { with } => {
                let open = f.day_open()[i];
                if b.c == open || (up(b.c > open) == dir) != *with {
                    return None;
                }
            }
        }
    }
    let side = if dir > 0 { Side::Buy } else { Side::Sell };
    let mode = if g.mode == Mode::Fade { "fade" } else { "follow" };
    Some((side, g.exit.stop_atr * atr, format!("{} {mode}: {why}", g.name)))
}

/// The stop distance in price after the desk's spread floor, and the target distance.
pub fn stop_and_target(g: &Genome, stop: f64, spread_px: f64, tick: f64) -> (f64, f64) {
    let d = stop.max(MIN_STOP_SPREADS as f64 * spread_px.max(tick));
    (d, d * g.exit.target_r.max(MIN_REWARD_TO_RISK as f64))
}

// ---------------------------------------------------------------------------
// Backtest
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct BtTrade {
    pub t: i64,
    pub symbol: String,
    pub side: &'static str,
    pub r: f64,
    pub exit: &'static str,
}

/// Run a genome over its basket, one position at a time, the way the desk runs it.
/// `data` maps tape symbols to their features.
pub fn backtest(g: &Genome, data: &mut HashMap<String, Features>) -> Vec<BtTrade> {
    backtest_slipped(g, data, 0.0)
}

/// `backtest` with every fill, in and out, `slip_ticks` ticks worse than the desk's price: the
/// cost sensitivity the audit asks for. The stop and target are still triggered by the bars.
pub fn backtest_slipped(g: &Genome, data: &mut HashMap<String, Features>, slip_ticks: f64) -> Vec<BtTrade> {
    let syms: Vec<String> = g.symbols.iter().filter(|s| data.contains_key(*s) && instrument_of(s).is_some()).cloned().collect();
    // Every bar close of every symbol in time order.
    let mut events: Vec<(i64, usize, usize)> = Vec::new();
    for (k, s) in syms.iter().enumerate() {
        for (i, b) in data[s].bars.iter().enumerate() {
            events.push((b.t, k, i));
        }
    }
    events.sort_unstable();
    let mut trades = Vec::new();
    let mut free_after = i64::MIN; // no entry decided before this bar-open time
    for &(t, k, i) in &events {
        if t < free_after {
            continue;
        }
        let s = &syms[k];
        let inst = instrument_of(s).unwrap();
        let f = data.get_mut(s).unwrap();
        let Some((side, stop, _)) = evaluate(g, f, i) else { continue };
        let Some(next) = f.bars.get(i + 1).copied() else { continue };
        if next.t != f.bars[i].t + BAR_MS {
            continue; // the next bar is after a gap (a close, a weekend): no fill to price
        }
        let tick = tick_size(inst) as f64;
        let typical = typical_spread_ticks(inst) * tick;
        let spread_at = |j: usize| f.spreads.as_ref().and_then(|v| v.get(j).copied()).filter(|x| x.is_finite() && *x > 0.0).unwrap_or(typical);
        let spr = spread_at(i + 1);
        let (d, reach) = stop_and_target(g, stop, spr, tick);
        let buy = side == Side::Buy;
        let entry = if buy { next.o + spr } else { next.o };
        let (stop_px, target_px) = if buy { (entry - d, entry + reach) } else { (entry + d, entry - reach) };
        let mut out: Option<(f64, &'static str, i64)> = None;
        let last = (i + g.exit.max_bars).min(f.bars.len() - 1);
        for j in i + 1..=last {
            let b = f.bars[j];
            let spr = spread_at(j);
            // Desk prices: a long exits at the bid (the bar), a short at the ask (bar + spread).
            let (lo, hi, open, close) = if buy { (b.l, b.h, b.o, b.c) } else { (b.l + spr, b.h + spr, b.o + spr, b.c + spr) };
            let stopped = if buy { lo <= stop_px } else { hi >= stop_px };
            if stopped {
                let px = if buy { open.min(stop_px) } else { open.max(stop_px) };
                out = Some((px, "stop", b.t));
                break;
            }
            let hit = if buy { hi >= target_px } else { lo <= target_px };
            if hit {
                let px = if buy { open.max(target_px) } else { open.min(target_px) };
                out = Some((px, "target", b.t));
                break;
            }
            let closes_at = b.t + BAR_MS;
            if minute_of_day(closes_at + BAR_MS) > FLAT_AT || day_of(closes_at) != day_of(next.t) {
                out = Some((close, "session close", b.t));
                break;
            }
            if j == i + g.exit.max_bars {
                out = Some((close, "time stop", b.t));
                break;
            }
        }
        let Some((px, why, at)) = out else { continue };
        // Slippage moves the fills, never the levels the desk set from its quote.
        let slip = slip_ticks * tick;
        let pnl = if buy { (px - slip) - (entry + slip) } else { (entry - slip) - (px + slip) };
        trades.push(BtTrade { t: next.t, symbol: s.clone(), side: if buy { "long" } else { "short" }, r: pnl / d, exit: why });
        free_after = at; // flat by the close of the exit bar, so it decides again there, as the desk does
    }
    trades
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bars(closes: &[f64], start_ms: i64) -> Vec<Bar> {
        closes
            .iter()
            .enumerate()
            .map(|(i, &c)| Bar { t: start_ms + i as i64 * BAR_MS, o: c, h: c + 0.0005, l: c - 0.0005, c })
            .collect()
    }

    fn genome(signal: Signal, mode: Mode) -> Genome {
        Genome {
            id: "g".into(),
            name: "Test".into(),
            signal,
            mode,
            filters: vec![],
            window: Window::All,
            exit: Exit { stop_atr: 1.0, target_r: 1.0, max_bars: 4 },
            symbols: vec!["EURUSD".into()],
            jev: None,
            parent: None,
            floor: None,
            holds: false,
            tactics: vec![],
            adapt: None,
        }
    }

    #[test]
    fn a_plain_genome_serialises_as_before_the_show_floor() {
        let g = genome(Signal::Run { k: 3 }, Mode::Follow);
        let v = serde_json::to_value(&g).unwrap();
        for k in ["floor", "holds", "tactics", "adapt"] {
            assert!(v.get(k).is_none(), "{k} must not appear on an ordinary genome: {v}");
        }
        assert_eq!(tag_of(&g), "genome");
        let h: Genome = serde_json::from_value(serde_json::json!({"id": "h", "name": "H", "signal": {"kind": "edge_lab", "key": "yt_k", "tf": 1440},
            "mode": "follow", "window": "all", "exit": {"stop_atr": 2.0, "target_r": 1.0, "max_bars": 0}, "symbols": ["US500"], "holds": true})).unwrap();
        assert_eq!(tag_of(&h), "holder");
        assert!(h.describe().starts_with("holder: edge-lab strategy yt_k on daily bars"), "{}", h.describe());
    }

    #[test]
    fn genomes_round_trip_through_json() {
        let g = genome(Signal::Band { n: 20, k: 2.0 }, Mode::Fade);
        let s = serde_json::to_string(&g).unwrap();
        assert!(s.contains("\"kind\":\"band\""), "{s}");
        assert_eq!(serde_json::from_str::<Genome>(&s).unwrap(), g);
    }

    #[test]
    fn a_run_fires_once_and_a_fader_takes_the_other_side() {
        // 08:00 UTC on a weekday, inside every window.
        let start = 1_792_800_000_000 - 1_792_800_000_000 % 86_400_000 + 8 * 3_600_000;
        let mut closes: Vec<f64> = vec![1.1; 20];
        closes.extend([1.101, 1.102, 1.103, 1.104]);
        let mut f = Features::new(bars(&closes, start));
        let follow = genome(Signal::Run { k: 3 }, Mode::Follow);
        let fade = genome(Signal::Run { k: 3 }, Mode::Fade);
        let n = closes.len();
        assert_eq!(evaluate(&follow, &mut f, n - 2).map(|e| e.0), Some(Side::Buy));
        assert_eq!(evaluate(&fade, &mut f, n - 2).map(|e| e.0), Some(Side::Sell));
        assert!(evaluate(&follow, &mut f, n - 1).is_none(), "a run of four is not the third close");
    }

    #[test]
    fn the_stop_is_floored_at_the_desk_spread_floor_and_the_target_at_one_r() {
        let mut g = genome(Signal::PrevDay, Mode::Follow);
        g.exit.target_r = 0.5;
        let (d, reach) = stop_and_target(&g, 0.00001, 0.00005, 0.0001);
        assert!((d - MIN_STOP_SPREADS as f64 * 0.0001).abs() < 1e-12, "spread floored at a tick, then 20 spreads");
        assert!((reach - d).abs() < 1e-12);
    }

    #[test]
    fn a_twin_asks_by_mode_and_its_chart_names_no_market() {
        let g = genome(Signal::Run { k: 3 }, Mode::Fade);
        let t = twin_of(&g);
        assert_eq!((t.id.as_str(), t.name.as_str()), ("g-J", "Test-J"));
        assert_eq!(t.jev.as_deref(), Some(jev_question(Mode::Fade)));
        assert_eq!(t.signal, g.signal);
        let start = 1_792_800_000_000 - 1_792_800_000_000 % 86_400_000 + 8 * 3_600_000;
        let b = bars(&vec![1.1234; 30], start);
        let st = jev_state(&b, Side::Sell);
        assert!(st.contains("SELL") && st.contains("1000.00"), "{st}");
        assert_eq!(st.lines().filter(|l| l.trim_start().starts_with(char::is_numeric)).count(), JEV_BARS);
        assert!(!st.contains("1.12") && !st.contains("EUR"), "no real prices, no symbol: {st}");
    }

    fn ohlc(rows: &[(f64, f64, f64, f64)], start_ms: i64) -> Vec<Bar> {
        rows.iter().enumerate().map(|(i, &(o, h, l, c))| Bar { t: start_ms + i as i64 * BAR_MS, o, h, l, c }).collect()
    }

    /// Midnight UTC of a Wednesday (2026-10-21).
    const WED: i64 = 1_792_540_800_000;

    #[test]
    fn the_new_kinds_round_trip_through_json() {
        for sig in [Signal::PrevWeek, Signal::Asia, Signal::Cross { fast: 20, slow: 50 }, Signal::Squeeze { k: 6 }, Signal::Wick { w: 0.6 }, Signal::Engulf, Signal::Drive { k: 2, x: 1.0 }] {
            let mut g = genome(sig, Mode::Follow);
            g.filters.push(Filter::Day { with: false });
            let s = serde_json::to_string(&g).unwrap();
            assert_eq!(serde_json::from_str::<Genome>(&s).unwrap(), g, "{s}");
            assert!(!g.describe().is_empty());
        }
    }

    #[test]
    fn weeks_start_on_sunday() {
        assert_eq!(WED % 86_400_000, 0);
        let sun = WED - 3 * 86_400_000 + 22 * 3_600_000;
        assert_eq!(week_of(sun), week_of(WED), "Sunday evening is the week it opens");
        assert_ne!(week_of(sun - 86_400_000), week_of(WED), "Saturday is the week before");
    }

    #[test]
    fn the_asian_range_breaks_once_and_its_sweep_is_faded() {
        // 28 flat Asian bars (00:00-07:00) between 1.0990 and 1.1010, then London.
        let mut rows = vec![(1.1, 1.101, 1.099, 1.1); 28];
        rows.push((1.1, 1.1015, 1.0995, 1.1005)); // 07:00 pokes above, closes inside: a sweep
        rows.push((1.1005, 1.1020, 1.1, 1.1015)); // 07:15 closes above: the break
        rows.push((1.1015, 1.1030, 1.1010, 1.1025)); // 07:30 above again, not the first
        let mut f = Features::new(ohlc(&rows, WED));
        let follow = genome(Signal::Asia, Mode::Follow);
        let fade = genome(Signal::Asia, Mode::Fade);
        assert_eq!(evaluate(&fade, &mut f, 28).map(|e| e.0), Some(Side::Sell));
        assert!(evaluate(&follow, &mut f, 28).is_none());
        assert_eq!(evaluate(&follow, &mut f, 29).map(|e| e.0), Some(Side::Buy));
        assert!(evaluate(&follow, &mut f, 30).is_none(), "once per day");
        assert!(evaluate(&follow, &mut f, 27).is_none(), "nothing inside the Asian hours");
    }

    #[test]
    fn a_cross_fires_on_the_crossing_bar_only() {
        let start = WED + 8 * 3_600_000;
        let mut closes = vec![1.0; 20];
        closes.extend([1.1, 1.2, 1.3]);
        let mut f = Features::new(bars(&closes, start));
        let g = genome(Signal::Cross { fast: 2, slow: 5 }, Mode::Follow);
        assert_eq!(evaluate(&g, &mut f, 20).map(|e| e.0), Some(Side::Buy));
        assert!(evaluate(&g, &mut f, 21).is_none());
    }

    #[test]
    fn candles_squeeze_wick_engulf() {
        let start = WED + 8 * 3_600_000;
        // Wide bars, then a narrow one, then a close above it.
        let mut rows = vec![(1.1, 1.102, 1.098, 1.1); 20];
        rows.push((1.1, 1.1003, 1.0998, 1.1001));
        rows.push((1.1001, 1.1012, 1.0999, 1.1010));
        let mut f = Features::new(ohlc(&rows, start));
        let sq = genome(Signal::Squeeze { k: 6 }, Mode::Follow);
        assert_eq!(evaluate(&sq, &mut f, 21).map(|e| e.0), Some(Side::Buy));
        assert!(evaluate(&sq, &mut f, 20).is_none(), "the narrow bar itself breaks nothing");

        // A long lower wick closing high: buyers rejected the low.
        let mut rows = vec![(1.1, 1.102, 1.098, 1.1); 20];
        rows.push((1.1, 1.1012, 1.0960, 1.1008));
        let mut f = Features::new(ohlc(&rows, start));
        let wk = genome(Signal::Wick { w: 0.6 }, Mode::Follow);
        assert_eq!(evaluate(&wk, &mut f, 20).map(|e| e.0), Some(Side::Buy));
        assert_eq!(evaluate(&genome(Signal::Wick { w: 0.6 }, Mode::Fade), &mut f, 20).map(|e| e.0), Some(Side::Sell));

        // A red bar swallowed by a green one.
        let mut rows = vec![(1.1, 1.102, 1.098, 1.1); 20];
        rows.push((1.1010, 1.1012, 1.0995, 1.0998));
        rows.push((1.0994, 1.1030, 1.0990, 1.1025));
        let mut f = Features::new(ohlc(&rows, start));
        let en = genome(Signal::Engulf, Mode::Follow);
        assert_eq!(evaluate(&en, &mut f, 21).map(|e| e.0), Some(Side::Buy));
        assert!(evaluate(&en, &mut f, 20).is_none());
    }

    #[test]
    fn a_drive_is_judged_at_its_bar_and_the_day_filter_reads_the_open() {
        // Flat until New York opens at 13:30, then two strong bars down.
        let start = WED + 8 * 3_600_000 + 30 * 60_000;
        let mut rows = vec![(1.1, 1.101, 1.099, 1.1); 20]; // 08:30-13:15
        rows.push((1.1, 1.1, 1.096, 1.097)); // 13:30
        rows.push((1.097, 1.097, 1.093, 1.094)); // 13:45
        let mut f = Features::new(ohlc(&rows, start));
        let dr = genome(Signal::Drive { k: 2, x: 1.0 }, Mode::Follow);
        assert_eq!(evaluate(&dr, &mut f, 21).map(|e| e.0), Some(Side::Sell));
        assert!(evaluate(&dr, &mut f, 20).is_none(), "the first bar is not the second");
        let mut with_day = dr.clone();
        with_day.filters.push(Filter::Day { with: true });
        assert_eq!(evaluate(&with_day, &mut f, 21).map(|e| e.0), Some(Side::Sell), "down on the day, selling with it");
        let mut against = dr.clone();
        against.filters.push(Filter::Day { with: false });
        assert!(evaluate(&against, &mut f, 21).is_none());
    }

    #[test]
    fn an_edge_lab_genome_enters_only_where_its_entries_say() {
        let start = WED + 8 * 3_600_000;
        let mut f = Features::new(bars(&vec![1.1; 30], start));
        let g = genome(Signal::EdgeLab { key: "yt_x".into(), tf: 60 }, Mode::Fade);
        assert!(evaluate(&g, &mut f, 20).is_none(), "no entries handed in: nothing");
        f.set_entries("yt_x@15", HashMap::from([(20, 1i8)]));
        assert!(evaluate(&g, &mut f, 20).is_none(), "another timeframe's entries are not this one's");
        f.set_entries("yt_x@60", HashMap::from([(20, 1i8)]));
        assert_eq!(evaluate(&g, &mut f, 20).map(|e| e.0), Some(Side::Sell), "a fader sells the long entry");
        assert!(evaluate(&g, &mut f, 21).is_none());
        let s = serde_json::to_string(&g).unwrap();
        assert!(s.contains("\"kind\":\"edge_lab\"") && s.contains("\"key\":\"yt_x\"") && s.contains("\"tf\":60"), "{s}");
        let old: Genome = serde_json::from_str(&s.replace(",\"tf\":60", "")).unwrap();
        assert_eq!(old.signal.edge_lab_id().as_deref(), Some("yt_x@15"), "genomes tried before tf existed ran on 15m bars");
    }

    #[test]
    fn nobody_enters_outside_the_window() {
        assert!(!Window::All.contains(6 * 60 + 59));
        assert!(Window::All.contains(7 * 60));
        assert!(!Window::All.contains(ENTRY_UNTIL));
        assert!(!Window::London.contains(12 * 60));
    }
}
