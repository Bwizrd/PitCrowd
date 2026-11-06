//! Where resting orders probably are.
//!
//! Nobody can see a CFD broker's customers' orders, so the floor marks the places they tend to
//! be left and lets price prove or disprove each one: unbroken swing highs and lows (stops and
//! breakout orders), the session's extremes, round numbers, ICT-style order blocks (the last
//! opposing candle before an impulse that broke structure) and fair value gaps (three-candle
//! imbalances), plus OANDA's published retail order book when the tape has it. Each is a band
//! with a kind, a strength and the bar it came from. The board chart and the wall board draw
//! them; later the paper traders will trade around them and the scoreboard will say which
//! kinds were worth anything.

use std::sync::{Arc, Mutex};

use bevy::prelude::*;

use crate::candles::{Candle, CandleStore, ChartSettings, Timeframe};
use crate::live_feed::{FeedMode, LiveFeed};
use crate::market::Instrument;
use crate::pit::set_ui_text;
use crate::pit_engine::tick_size;
use crate::scenario::MarketScenarioController;
use crate::tape_chart::{
    BOARD_BARS, CANDLE_BASE, CHART_W, ChartLayout, TapeHistory, candle_layout, chart_source,
};
use crate::ui::Selection;

/// Bands drawn at once per chart.
pub const LEVEL_NODES: usize = 14;
/// Port of the market tape (`floor_tape`, docs/DATA_SOURCES.md).
pub const TAPE_PORT: u16 = crate::endpoints::TAPE_PORT;
/// OANDA republishes every 20 minutes; the tape caches it, so asking every 5 is plenty.
pub const OANDA_REFRESH_SECS: f64 = 300.0;
/// Bars either side a swing point must beat.
const SWING_K: usize = 2;
const MIN_FVG_TICKS: f32 = 2.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LevelKind {
    SwingHigh,
    SwingLow,
    SessionHigh,
    SessionLow,
    Round,
    BullOrderBlock,
    BearOrderBlock,
    BullFvg,
    BearFvg,
    OandaLongOrders,
    OandaShortOrders,
}

impl LevelKind {
    pub const ALL: [LevelKind; 11] = [
        LevelKind::SwingHigh,
        LevelKind::SwingLow,
        LevelKind::SessionHigh,
        LevelKind::SessionLow,
        LevelKind::Round,
        LevelKind::BullOrderBlock,
        LevelKind::BearOrderBlock,
        LevelKind::BullFvg,
        LevelKind::BearFvg,
        LevelKind::OandaLongOrders,
        LevelKind::OandaShortOrders,
    ];

    /// The kind whose `tag` this is (the text the state and the TRIGGER line carry).
    pub fn from_tag(tag: &str) -> Option<LevelKind> {
        LevelKind::ALL
            .iter()
            .copied()
            .find(|k| k.tag() == tag.trim())
    }

    pub fn tag(self) -> &'static str {
        match self {
            LevelKind::SwingHigh => "SWING HI",
            LevelKind::SwingLow => "SWING LO",
            LevelKind::SessionHigh => "SESSION HI",
            LevelKind::SessionLow => "SESSION LO",
            LevelKind::Round => "ROUND",
            LevelKind::BullOrderBlock => "OB BULL",
            LevelKind::BearOrderBlock => "OB BEAR",
            LevelKind::BullFvg => "FVG BULL",
            LevelKind::BearFvg => "FVG BEAR",
            LevelKind::OandaLongOrders => "OANDA LONG",
            LevelKind::OandaShortOrders => "OANDA SHORT",
        }
    }

    /// +1 buyers expected to defend it, -1 sellers, 0 neutral.
    pub fn bias(self) -> i8 {
        match self {
            LevelKind::BullOrderBlock
            | LevelKind::BullFvg
            | LevelKind::SwingLow
            | LevelKind::SessionLow
            | LevelKind::OandaLongOrders => 1,
            LevelKind::BearOrderBlock
            | LevelKind::BearFvg
            | LevelKind::SwingHigh
            | LevelKind::SessionHigh
            | LevelKind::OandaShortOrders => -1,
            LevelKind::Round => 0,
        }
    }

    pub fn color(self) -> Color {
        match self {
            LevelKind::BullOrderBlock | LevelKind::BullFvg => Color::srgb(0.25, 0.85, 0.55),
            LevelKind::BearOrderBlock | LevelKind::BearFvg => Color::srgb(0.95, 0.40, 0.40),
            LevelKind::SwingHigh | LevelKind::SessionHigh => Color::srgb(0.95, 0.70, 0.35),
            LevelKind::SwingLow | LevelKind::SessionLow => Color::srgb(0.95, 0.70, 0.35),
            LevelKind::Round => Color::srgb(0.70, 0.74, 0.82),
            LevelKind::OandaLongOrders => Color::srgb(0.30, 0.75, 1.0),
            LevelKind::OandaShortOrders => Color::srgb(1.0, 0.40, 0.80),
        }
    }

    pub fn is_oanda(self) -> bool {
        matches!(
            self,
            LevelKind::OandaLongOrders | LevelKind::OandaShortOrders
        )
    }

    /// Band alpha on the board chart. OANDA clusters are drawn solid enough to read at a
    /// glance (they are the only levels that come from other people's orders); the
    /// structure levels stay translucent so candles show through them.
    pub fn band_alpha(self, strength: f32) -> f32 {
        if self.is_oanda() {
            0.40 + 0.40 * strength
        } else {
            0.10 + 0.18 * strength
        }
    }

    /// Minimum band thickness in logical px on the board chart.
    pub fn min_band_px(self) -> f32 {
        if self.is_oanda() { 5.0 } else { 1.0 }
    }

    /// Alpha multiplier for a band `h` px tall: OANDA clusters stay solid up to 16 px and fade
    /// toward 0.15 as they get taller, so a cluster wider than the chart's range never hides
    /// the candles. Structure levels are untouched.
    pub fn tall_band_fade(self, h: f32) -> f32 {
        if !self.is_oanda() || h <= 16.0 {
            1.0
        } else {
            (16.0 / h).clamp(0.15, 1.0)
        }
    }

    /// Lower draws first when there are more levels than nodes.
    fn priority(self) -> u8 {
        match self {
            LevelKind::BullOrderBlock | LevelKind::BearOrderBlock => 0,
            LevelKind::BullFvg | LevelKind::BearFvg => 1,
            LevelKind::OandaLongOrders | LevelKind::OandaShortOrders => 2,
            LevelKind::SwingHigh | LevelKind::SwingLow => 3,
            LevelKind::SessionHigh | LevelKind::SessionLow => 4,
            LevelKind::Round => 5,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Level {
    pub kind: LevelKind,
    pub lo: f32,
    pub hi: f32,
    /// 0..1: how much to trust it (an untouched block beats a touched one; a big OANDA cluster
    /// beats a small one).
    pub strength: f32,
    /// Bar the level came from (0 for OANDA and round numbers).
    pub time_ms: i64,
    /// Price has already traded into it once.
    pub touched: bool,
    /// OANDA: percentage of orders in the bucket. Zero otherwise.
    pub pct: f32,
}

impl Level {
    pub fn mid(&self) -> f32 {
        (self.lo + self.hi) * 0.5
    }

    pub fn label(&self) -> String {
        match self.kind {
            LevelKind::OandaLongOrders | LevelKind::OandaShortOrders => {
                format!("{} {:.1}%", self.kind.tag(), self.pct)
            }
            _ if self.touched => format!("{} (touched)", self.kind.tag()),
            _ => self.kind.tag().to_string(),
        }
    }
}

/// Spacing of the round numbers worth marking, per instrument.
pub fn round_step(inst: Instrument) -> f32 {
    match inst {
        Instrument::EurUsd | Instrument::GbpUsd => 0.005,
        Instrument::Gold => 10.0,
        Instrument::Silver => 0.5,
        Instrument::Sp500 => 25.0,
        Instrument::Nasdaq | Instrument::Us30 => 100.0,
        Instrument::Uk100 => 50.0,
        Instrument::Bitcoin => 500.0,
    }
}

fn is_swing_high(bars: &[Candle], i: usize) -> bool {
    if i < SWING_K || i + SWING_K >= bars.len() {
        return false;
    }
    let h = bars[i].high;
    (1..=SWING_K).all(|k| bars[i - k].high < h && bars[i + k].high < h)
}

fn is_swing_low(bars: &[Candle], i: usize) -> bool {
    if i < SWING_K || i + SWING_K >= bars.len() {
        return false;
    }
    let l = bars[i].low;
    (1..=SWING_K).all(|k| bars[i - k].low > l && bars[i + k].low > l)
}

/// Levels from the bars alone (newest-first inside each kind). `bars` oldest first.
pub fn detect_levels(bars: &[Candle], tick: f32, round: f32) -> Vec<Level> {
    let n = bars.len();
    let mut out = Vec::new();
    if n < 3 || tick <= 0.0 {
        return out;
    }
    let swing_highs: Vec<usize> = (0..n).filter(|&i| is_swing_high(bars, i)).collect();
    let swing_lows: Vec<usize> = (0..n).filter(|&i| is_swing_low(bars, i)).collect();

    // Order blocks: a close through a swing is the break of structure; the block is the last
    // candle against the move before it. Invalidated by a close through the block's far side.
    let mut bull_obs: Vec<Level> = Vec::new();
    for &s in &swing_highs {
        let h = bars[s].high;
        let Some(b) = (s + 1..n).find(|&b| bars[b].close > h) else {
            continue;
        };
        let Some(j) = (s..b).rev().find(|&j| bars[j].close < bars[j].open) else {
            continue;
        };
        let (lo, hi) = (bars[j].low, bars[j].high);
        if bars[b + 1..].iter().any(|x| x.close < lo) {
            continue;
        }
        if bull_obs.iter().any(|o| o.time_ms == bars[j].time_ms) {
            continue;
        }
        let touched = bars[b + 1..].iter().any(|x| x.low <= hi);
        bull_obs.push(Level {
            kind: LevelKind::BullOrderBlock,
            lo,
            hi,
            strength: if touched { 0.5 } else { 0.9 },
            time_ms: bars[j].time_ms,
            touched,
            pct: 0.0,
        });
    }
    let mut bear_obs: Vec<Level> = Vec::new();
    for &s in &swing_lows {
        let l = bars[s].low;
        let Some(b) = (s + 1..n).find(|&b| bars[b].close < l) else {
            continue;
        };
        let Some(j) = (s..b).rev().find(|&j| bars[j].close > bars[j].open) else {
            continue;
        };
        let (lo, hi) = (bars[j].low, bars[j].high);
        if bars[b + 1..].iter().any(|x| x.close > hi) {
            continue;
        }
        if bear_obs.iter().any(|o| o.time_ms == bars[j].time_ms) {
            continue;
        }
        let touched = bars[b + 1..].iter().any(|x| x.high >= lo);
        bear_obs.push(Level {
            kind: LevelKind::BearOrderBlock,
            lo,
            hi,
            strength: if touched { 0.5 } else { 0.9 },
            time_ms: bars[j].time_ms,
            touched,
            pct: 0.0,
        });
    }
    out.extend(newest(bull_obs, 2));
    out.extend(newest(bear_obs, 2));

    // Fair value gaps: a three-candle hole price never traded through, shrunk by any partial fill.
    let mut bull_fvg = Vec::new();
    let mut bear_fvg = Vec::new();
    for i in 1..n - 1 {
        let later = &bars[(i + 2).min(n)..];
        let (glo, ghi) = (bars[i - 1].high, bars[i + 1].low);
        if ghi - glo >= MIN_FVG_TICKS * tick * 0.999 && !later.iter().any(|x| x.low <= glo) {
            let hi = later.iter().map(|x| x.low).fold(ghi, f32::min);
            bull_fvg.push(Level {
                kind: LevelKind::BullFvg,
                lo: glo,
                hi,
                strength: if hi < ghi { 0.5 } else { 0.8 },
                time_ms: bars[i].time_ms,
                touched: hi < ghi,
                pct: 0.0,
            });
        }
        let (glo, ghi) = (bars[i + 1].high, bars[i - 1].low);
        if ghi - glo >= MIN_FVG_TICKS * tick * 0.999 && !later.iter().any(|x| x.high >= ghi) {
            let lo = later.iter().map(|x| x.high).fold(glo, f32::max);
            bear_fvg.push(Level {
                kind: LevelKind::BearFvg,
                lo,
                hi: ghi,
                strength: if lo > glo { 0.5 } else { 0.8 },
                time_ms: bars[i].time_ms,
                touched: lo > glo,
                pct: 0.0,
            });
        }
    }
    out.extend(newest(bull_fvg, 2));
    out.extend(newest(bear_fvg, 2));

    // Unbroken swings: where the stops and breakout orders sit.
    let mut highs: Vec<Level> = swing_highs
        .iter()
        .rev()
        .filter(|&&i| bars[i + 1..].iter().all(|x| x.close <= bars[i].high))
        .map(|&i| Level {
            kind: LevelKind::SwingHigh,
            lo: bars[i].high - tick * 0.5,
            hi: bars[i].high + tick * 0.5,
            strength: 0.6,
            time_ms: bars[i].time_ms,
            touched: false,
            pct: 0.0,
        })
        .collect();
    let mut lows: Vec<Level> = swing_lows
        .iter()
        .rev()
        .filter(|&&i| bars[i + 1..].iter().all(|x| x.close >= bars[i].low))
        .map(|&i| Level {
            kind: LevelKind::SwingLow,
            lo: bars[i].low - tick * 0.5,
            hi: bars[i].low + tick * 0.5,
            strength: 0.6,
            time_ms: bars[i].time_ms,
            touched: false,
            pct: 0.0,
        })
        .collect();
    dedup_near(&mut highs, tick * 2.0);
    dedup_near(&mut lows, tick * 2.0);
    highs.truncate(3);
    lows.truncate(3);
    out.extend(highs);
    out.extend(lows);

    // The window's extremes.
    let (mut lo, mut hi) = (f32::MAX, f32::MIN);
    let (mut lo_t, mut hi_t) = (0i64, 0i64);
    for b in bars {
        if b.low < lo {
            lo = b.low;
            lo_t = b.time_ms;
        }
        if b.high > hi {
            hi = b.high;
            hi_t = b.time_ms;
        }
    }
    out.push(Level {
        kind: LevelKind::SessionHigh,
        lo: hi - tick * 0.5,
        hi: hi + tick * 0.5,
        strength: 0.5,
        time_ms: hi_t,
        touched: false,
        pct: 0.0,
    });
    out.push(Level {
        kind: LevelKind::SessionLow,
        lo: lo - tick * 0.5,
        hi: lo + tick * 0.5,
        strength: 0.5,
        time_ms: lo_t,
        touched: false,
        pct: 0.0,
    });

    // Round numbers inside the window, nearest to the last close first.
    if round > 0.0 {
        let last = bars[n - 1].close;
        let mut k = (lo / round).ceil();
        let mut rounds = Vec::new();
        while k * round <= hi {
            rounds.push(k * round);
            k += 1.0;
        }
        rounds.sort_by(|a, b| (a - last).abs().partial_cmp(&(b - last).abs()).unwrap());
        for px in rounds.into_iter().take(4) {
            out.push(Level {
                kind: LevelKind::Round,
                lo: px - tick * 0.5,
                hi: px + tick * 0.5,
                strength: 0.4,
                time_ms: 0,
                touched: false,
                pct: 0.0,
            });
        }
    }
    out
}

fn newest(mut v: Vec<Level>, keep: usize) -> Vec<Level> {
    v.sort_by_key(|l| std::cmp::Reverse(l.time_ms));
    v.truncate(keep);
    v
}

/// Drop levels within `gap` of an earlier (more recent) one in the list.
fn dedup_near(v: &mut Vec<Level>, gap: f32) {
    let mut kept: Vec<Level> = Vec::new();
    for l in v.drain(..) {
        if !kept.iter().any(|k| (k.mid() - l.mid()).abs() <= gap) {
            kept.push(l);
        }
    }
    *v = kept;
}

/// Order the levels for drawing: most useful kinds first, newest first inside a kind, at most
/// `cap`.
pub fn compose(mut levels: Vec<Level>, cap: usize) -> Vec<Level> {
    levels.sort_by(|a, b| {
        a.kind
            .priority()
            .cmp(&b.kind.priority())
            .then(b.time_ms.cmp(&a.time_ms))
    });
    levels.truncate(cap);
    levels
}

// ---------------------------------------------------------------------------
// OANDA order book (served by the market tape)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Default)]
pub struct OandaBook {
    pub time: String,
    pub price: f32,
    pub bucket_width: f32,
    /// (price, long %, short %), as OANDA publishes it: the share of open orders in the bucket.
    pub buckets: Vec<(f32, f32, f32)>,
}

/// The tape's `/orderbook` payload.
pub fn parse_oanda(text: &str) -> Result<OandaBook, String> {
    let v: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    if let Some(err) = v.get("error").and_then(|e| e.as_str()) {
        return Err(err.to_string());
    }
    if v.get("pending").and_then(|p| p.as_bool()) == Some(true) {
        return Err("pending".into());
    }
    let price = v.get("price").and_then(|p| p.as_f64()).ok_or("no price")? as f32;
    let bucket_width = v.get("bucketWidth").and_then(|p| p.as_f64()).unwrap_or(0.0) as f32;
    let buckets = v
        .get("buckets")
        .and_then(|b| b.as_array())
        .ok_or("no buckets")?
        .iter()
        .filter_map(|row| {
            let r = row.as_array()?;
            Some((
                r.first()?.as_f64()? as f32,
                r.get(1)?.as_f64()? as f32,
                r.get(2)?.as_f64()? as f32,
            ))
        })
        .collect();
    Ok(OandaBook {
        time: v
            .get("time")
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .to_string(),
        price,
        bucket_width,
        buckets,
    })
}

/// The biggest order clusters inside the chart's range, up to three a side.
///
/// OANDA buckets are narrow (0.05% of price) and a popular level usually spans several of
/// them, so neighbouring buckets within `OANDA_CLUSTER_GAP` widths of each other are merged
/// into one band whose percentage is their sum. A cluster is kept when it holds at least
/// `OANDA_MIN_SHARE` of the side's biggest cluster, so three bands never show when one
/// dominates.
pub const OANDA_CLUSTER_GAP: f32 = 2.5;
pub const OANDA_MIN_SHARE: f32 = 0.35;

pub fn oanda_levels(book: &OandaBook, lo: f32, hi: f32) -> Vec<Level> {
    let width = book.bucket_width.max(0.0);
    let half = width * 0.5;
    let in_range: Vec<&(f32, f32, f32)> = book
        .buckets
        .iter()
        .filter(|b| b.0 >= lo && b.0 <= hi)
        .collect();
    let mut out = Vec::new();
    for (kind, pick) in [
        (LevelKind::OandaLongOrders, 1usize),
        (LevelKind::OandaShortOrders, 2usize),
    ] {
        let pct_of = |b: &(f32, f32, f32)| if pick == 1 { b.1 } else { b.2 };
        let clusters = cluster_buckets(&in_range, |b| pct_of(b), width * OANDA_CLUSTER_GAP);
        let max_pct = clusters.iter().map(|c| c.2).fold(0.0f32, f32::max);
        if max_pct <= 0.0 {
            continue;
        }
        let mut rows: Vec<(f32, f32, f32)> = clusters
            .into_iter()
            .filter(|c| c.2 >= max_pct * OANDA_MIN_SHARE)
            .collect();
        rows.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap());
        for (clo, chi, pct) in rows.into_iter().take(3) {
            out.push(Level {
                kind,
                lo: clo - half,
                hi: chi + half,
                strength: (pct / max_pct).clamp(0.2, 1.0),
                time_ms: 0,
                touched: false,
                pct,
            });
        }
    }
    out
}

/// Merge buckets with a non-zero share into runs whose neighbours sit within `gap` of each
/// other: `(lowest price, highest price, summed share)`, in price order.
fn cluster_buckets(
    buckets: &[&(f32, f32, f32)],
    share: impl Fn(&(f32, f32, f32)) -> f32,
    gap: f32,
) -> Vec<(f32, f32, f32)> {
    let mut rows: Vec<(f32, f32)> = buckets
        .iter()
        .map(|b| (b.0, share(b)))
        .filter(|r| r.1 > 0.0)
        .collect();
    rows.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    let mut out: Vec<(f32, f32, f32)> = Vec::new();
    for (price, pct) in rows {
        match out.last_mut() {
            Some(last) if price - last.1 <= gap + 1e-9 => {
                last.1 = price;
                last.2 += pct;
            }
            _ => out.push((price, price, pct)),
        }
    }
    out
}

/// Where the tape is: the page's host on :8083 unless `?tape=` says otherwise; natively
/// `OPENCRY_TAPE_URL` (see `endpoints`).
pub fn tape_url() -> String {
    crate::endpoints::tape_url()
}

/// cTrader name without the slash, which is what the tape uses.
pub fn tape_symbol(inst: Instrument) -> String {
    inst.symbol().replace('/', "")
}

// ---------------------------------------------------------------------------
// Resource and systems
// ---------------------------------------------------------------------------

type Pending = Arc<Mutex<Vec<(Instrument, Result<OandaBook, String>)>>>;

#[derive(Resource)]
pub struct LevelStore {
    /// Drawn levels per pit, indexed by `Instrument as usize`.
    pub levels: Vec<Vec<Level>>,
    pub revision: u64,
    computed: Vec<Option<(u64, Timeframe, FeedMode, u64)>>,
    /// Levels the paper traders use: every live pit, 1m bars, refreshed at most every 2 s.
    trade_levels: Vec<Vec<Level>>,
    trade_computed: Vec<u64>,
    trade_next: Vec<f64>,
    oanda: Vec<Option<OandaBook>>,
    oanda_rev: u64,
    oanda_next: Vec<f64>,
    oanda_inflight: Vec<bool>,
    pending: Pending,
    pub oanda_status: String,
}

impl Default for LevelStore {
    fn default() -> Self {
        let n = Instrument::ALL.len();
        Self {
            levels: vec![Vec::new(); n],
            revision: 0,
            computed: vec![None; n],
            trade_levels: vec![Vec::new(); n],
            trade_computed: vec![u64::MAX; n],
            trade_next: vec![0.0; n],
            oanda: vec![None; n],
            oanda_rev: 0,
            oanda_next: vec![0.0; n],
            oanda_inflight: vec![false; n],
            pending: Arc::new(Mutex::new(Vec::new())),
            oanda_status: String::new(),
        }
    }
}

impl LevelStore {
    pub fn for_pit(&self, inst: Instrument) -> &[Level] {
        &self.levels[inst as usize]
    }

    /// What the paper traders trade around (1m bars, no OANDA).
    pub fn for_trading(&self, inst: Instrument) -> &[Level] {
        &self.trade_levels[inst as usize]
    }

    /// Recompute a pit's trading levels from its 1m bars (at most every 2 s).
    pub fn refresh_trading(
        &mut self,
        inst: Instrument,
        bars: &[Candle],
        series_rev: u64,
        now: f64,
    ) {
        let i = inst as usize;
        if self.trade_computed[i] == series_rev || now < self.trade_next[i] {
            return;
        }
        self.trade_computed[i] = series_rev;
        self.trade_next[i] = now + 2.0;
        self.trade_levels[i] = compose(
            detect_levels(bars, tick_size(inst), round_step(inst)),
            LEVEL_NODES * 2,
        );
    }

    pub fn oanda(&self, inst: Instrument) -> Option<&OandaBook> {
        self.oanda[inst as usize].as_ref()
    }

    fn drain(&mut self, now: f64) {
        let done: Vec<_> = match self.pending.lock() {
            Ok(mut p) => p.drain(..).collect(),
            Err(_) => return,
        };
        for (inst, res) in done {
            let i = inst as usize;
            self.oanda_inflight[i] = false;
            match res {
                Ok(book) => {
                    if self.oanda[i].as_ref() != Some(&book) {
                        self.oanda[i] = Some(book);
                        self.oanda_rev += 1;
                    }
                    self.oanda_status = "OANDA book on".into();
                    self.oanda_next[i] = now + OANDA_REFRESH_SECS;
                }
                Err(e) if e == "pending" => {
                    self.oanda_status = "OANDA book loading".into();
                    self.oanda_next[i] = now + 30.0;
                }
                Err(e) => {
                    self.oanda_status =
                        format!("OANDA: {}", e.chars().take(70).collect::<String>());
                    self.oanda_next[i] = now + OANDA_REFRESH_SECS * 2.0;
                }
            }
        }
    }

    fn request_oanda(&mut self, inst: Instrument, now: f64) {
        let i = inst as usize;
        if self.oanda_inflight[i] || now < self.oanda_next[i] {
            return;
        }
        self.oanda_inflight[i] = true;
        self.oanda_next[i] = now + OANDA_REFRESH_SECS;
        let pending = self.pending.clone();
        let url = format!("{}/orderbook?symbol={}", tape_url(), tape_symbol(inst));
        ehttp::fetch(ehttp::Request::get(url), move |res| {
            let out = match res {
                Ok(r) if r.ok || r.status == 202 => parse_oanda(r.text().unwrap_or("")),
                Ok(r) if r.status == 503 => Err("not configured on the tape".into()),
                Ok(r) => Err(format!("HTTP {}", r.status)),
                Err(e) => Err(format!(
                    "tape unreachable ({})",
                    e.chars().take(40).collect::<String>()
                )),
            };
            if let Ok(mut p) = pending.lock() {
                p.push((inst, out));
            }
        });
    }

    /// Recompute a pit's levels if its bars or its OANDA book changed.
    fn refresh(
        &mut self,
        inst: Instrument,
        bars: &[Candle],
        series_rev: u64,
        tf: Timeframe,
        mode: FeedMode,
    ) {
        let i = inst as usize;
        let key = (series_rev, tf, mode, self.oanda_rev);
        if self.computed[i] == Some(key) {
            return;
        }
        self.computed[i] = Some(key);
        let tick = tick_size(inst);
        let mut levels = detect_levels(bars, tick, round_step(inst));
        if let (Some(book), Some(first)) = (self.oanda[i].as_ref(), bars.first()) {
            let (lo, hi) = bars.iter().fold((first.low, first.high), |(l, h), b| {
                (l.min(b.low), h.max(b.high))
            });
            levels.extend(oanda_levels(book, lo, hi));
        }
        self.levels[i] = compose(levels, LEVEL_NODES);
        self.revision += 1;
    }
}

/// System: keep the selected pit's levels current and its OANDA book fresh.
pub fn update_levels(
    time: Res<Time>,
    feed: Res<LiveFeed>,
    selection: Res<Selection>,
    settings: Res<ChartSettings>,
    store: Res<CandleStore>,
    mut levels: ResMut<LevelStore>,
) {
    let now = time.elapsed_secs_f64();
    levels.drain(now);
    // Trading levels for every pit that is ticking, from its 1m bars.
    if feed.mode == FeedMode::Live {
        for inst in Instrument::ALL {
            if !matches!(
                feed.pit_status(inst, now),
                crate::live_feed::PitFeedStatus::Live { .. }
            ) {
                continue;
            }
            let series = store.series(inst, Timeframe::M1, FeedMode::Live);
            let n = series.bars.len();
            let bars: Vec<Candle> = series
                .bars
                .iter()
                .skip(n.saturating_sub(BOARD_BARS))
                .copied()
                .collect();
            let rev = series.revision;
            levels.refresh_trading(inst, &bars, rev, now);
        }
    }
    if !settings.levels {
        return;
    }
    let Some(inst) = selection.selected_instrument else {
        return;
    };
    let (mode, _) = chart_source(&feed, inst, now);
    if mode == FeedMode::Live {
        levels.request_oanda(inst, now);
    }
    let tf = settings.timeframe;
    let series = store.series(inst, tf, mode);
    let shown = series.shown();
    let bars = &shown[shown.len().saturating_sub(BOARD_BARS)..];
    let rev = series.revision;
    levels.refresh(inst, bars, rev, tf, mode);
}

// ---------------------------------------------------------------------------
// Board chart bands
// ---------------------------------------------------------------------------

#[derive(Component)]
pub struct ChartLevelBand(pub usize);

#[derive(Component)]
pub struct ChartLevelTag(pub usize);

/// Spawn the band and tag nodes inside the board chart (called from `spawn_tape_chart`).
pub fn spawn_level_nodes(chart: &mut ChildSpawnerCommands) {
    for i in 0..LEVEL_NODES {
        chart.spawn((
            ChartLevelBand(i),
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                width: Val::Px(CHART_W),
                display: Display::None,
                ..default()
            },
            BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.2)),
        ));
        chart.spawn((
            ChartLevelTag(i),
            Text::new(""),
            TextFont {
                font_size: 8.0,
                ..default()
            },
            TextColor(Color::WHITE),
            Node {
                position_type: PositionType::Absolute,
                right: Val::Px(3.0),
                display: Display::None,
                ..default()
            },
        ));
    }
}

/// System: lay the levels over the board candles, on the same scale as the candles, at
/// whatever size the chart is (`ChartLayout`).
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn update_board_levels(
    time: Res<Time>,
    feed: Res<LiveFeed>,
    selection: Res<Selection>,
    settings: Res<ChartSettings>,
    size: Res<ChartLayout>,
    store: Res<CandleStore>,
    history: Res<TapeHistory>,
    levels: Res<LevelStore>,
    mut drawn: Local<Option<(Instrument, u64, u64, u64, bool, bool)>>,
    mut bands: Query<(&ChartLevelBand, &mut Node, &mut BackgroundColor), Without<ChartLevelTag>>,
    mut tags: Query<
        (&ChartLevelTag, &mut Node, &mut Text, &mut TextColor),
        Without<ChartLevelBand>,
    >,
) {
    let Some(inst) = selection.selected_instrument else {
        return;
    };
    let now = time.elapsed_secs_f64();
    let (mode, _) = chart_source(&feed, inst, now);
    let tf = settings.timeframe;
    let series = store.series(inst, tf, mode);
    let tape = history.tape(inst);
    let show = settings.board_chart && settings.levels;
    let key = (
        inst,
        series.revision,
        tape.revision,
        levels.revision,
        show,
        settings.deal_markers,
    );
    if *drawn == Some(key) && !size.is_changed() {
        return;
    }
    *drawn = Some(key);
    let hide_all = |bands: &mut Query<
        (&ChartLevelBand, &mut Node, &mut BackgroundColor),
        Without<ChartLevelTag>,
    >,
                    tags: &mut Query<
        (&ChartLevelTag, &mut Node, &mut Text, &mut TextColor),
        Without<ChartLevelBand>,
    >| {
        for (_, mut n, _) in bands.iter_mut() {
            n.display = Display::None;
        }
        for (_, mut n, _, _) in tags.iter_mut() {
            n.display = Display::None;
        }
    };
    if !show {
        hide_all(&mut bands, &mut tags);
        return;
    }
    let bars = series.shown();
    let marks = if settings.deal_markers {
        tape.marks()
    } else {
        Vec::new()
    };
    let (chart_w, candle_h, pad) = (size.chart_w, size.candle_h(), size.pad);
    let Some(layout) = candle_layout(
        &bars,
        &marks,
        tf.millis(),
        chart_w,
        candle_h,
        tick_size(inst) * 6.0,
        BOARD_BARS,
    ) else {
        hide_all(&mut bands, &mut tags);
        return;
    };
    let span = (layout.hi - layout.lo).max(1e-6);
    let y_of = |p: f32| {
        (CANDLE_BASE + pad + (p - layout.lo) / span * (candle_h - 2.0 * pad))
            .clamp(CANDLE_BASE, CANDLE_BASE + candle_h)
    };
    let shown = &bars[bars.len() - layout.candles.len()..];
    // A level starts at the bar it came from and runs to the right edge, like a trader draws it.
    let x_start = |l: &Level| level_start_x(l, shown, &layout.candles, size.slot());
    let list = levels.for_pit(inst);
    for (band, mut node, mut color) in bands.iter_mut() {
        match list.get(band.0) {
            Some(l) if l.hi >= layout.lo && l.lo <= layout.hi => {
                let (y0, y1) = (y_of(l.lo), y_of(l.hi));
                let x0 = x_start(l);
                node.display = Display::Flex;
                node.left = Val::Px(x0.round());
                node.width = Val::Px((chart_w - x0).round().max(1.0));
                node.bottom = Val::Px(y0.round());
                let min_h = l.kind.min_band_px();
                let h = (y1 - y0).round().max(min_h);
                node.bottom = Val::Px((y0 - (h - (y1 - y0)) * 0.5).round().max(CANDLE_BASE));
                node.height = Val::Px(h);
                // A cluster wider than a few pips on a tight FX chart would otherwise paint
                // the whole panel: past 16 px tall an OANDA band fades toward a tint.
                color.0 = l
                    .kind
                    .color()
                    .with_alpha(l.kind.band_alpha(l.strength) * l.kind.tall_band_fade(h));
            }
            _ => node.display = Display::None,
        }
    }
    for (tag, mut node, mut text, mut tc) in tags.iter_mut() {
        match list.get(tag.0) {
            Some(l) if l.hi >= layout.lo && l.lo <= layout.hi => {
                node.display = Display::Flex;
                node.left = Val::Px((x_start(l) + 2.0).round().min(chart_w - 70.0));
                node.right = Val::Auto;
                node.bottom = Val::Px((y_of(l.hi) + 1.0).round().min(CANDLE_BASE + candle_h - 9.0));
                set_ui_text(&mut text, l.label());
                let c = l.kind.color().with_alpha(0.9);
                if tc.0 != c {
                    *tc = TextColor(c);
                }
            }
            _ => node.display = Display::None,
        }
    }
}

/// X where a level's band begins: the left edge of the bar it came from, or the chart's left
/// edge for levels with no bar (round numbers, OANDA clusters, bars scrolled off).
pub fn level_start_x(
    l: &Level,
    shown: &[Candle],
    candles: &[crate::tape_chart::CandleBox],
    slot: f32,
) -> f32 {
    if l.time_ms == 0 {
        return 0.0;
    }
    shown
        .iter()
        .position(|b| b.time_ms == l.time_ms)
        .and_then(|i| candles.get(i))
        .map(|c| (c.x - slot * 0.5).max(0.0))
        .unwrap_or(0.0)
}

pub fn toggle_levels(settings: &mut ChartSettings, controller: &mut MarketScenarioController) {
    settings.levels = !settings.levels;
    controller.announcement_banner = if settings.levels {
        "LEVELS ON: swings, blocks, gaps, OANDA orders".into()
    } else {
        "LEVELS OFF".into()
    };
    controller.banner_timer = 2.0;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bar(i: i64, o: f32, h: f32, l: f32, c: f32) -> Candle {
        Candle {
            time_ms: i * 60_000,
            open: o,
            high: h,
            low: l,
            close: c,
            volume: 1.0,
            ..Default::default()
        }
    }

    /// A base, a swing high at bar 2, a pullback whose last red candle is bar 5, then an impulse
    /// that closes above the swing at bar 7 and keeps going.
    fn bull_structure() -> Vec<Candle> {
        vec![
            bar(0, 100.0, 101.0, 99.5, 100.5),
            bar(1, 100.5, 102.0, 100.0, 101.5),
            bar(2, 101.5, 104.0, 101.0, 103.0), // swing high 104
            bar(3, 103.0, 103.5, 101.5, 102.0),
            bar(4, 102.0, 102.5, 100.5, 101.0),
            bar(5, 101.0, 101.5, 99.0, 99.5), // last bearish candle before the impulse: OB 99.0..101.5
            bar(6, 99.5, 102.5, 99.5, 102.3),
            bar(7, 102.3, 105.0, 102.0, 104.8), // BOS: close above 104
            bar(8, 104.8, 106.0, 104.5, 105.5),
            bar(9, 105.5, 107.5, 105.3, 107.0), // bars 7..9: FVG between bar7.high 105 and bar9.low 105.3
            bar(10, 107.0, 107.4, 106.4, 106.8),
        ]
    }

    #[test]
    fn finds_the_bull_order_block_behind_a_break_of_structure() {
        let lv = detect_levels(&bull_structure(), 0.1, 5.0);
        let ob = lv
            .iter()
            .find(|l| l.kind == LevelKind::BullOrderBlock)
            .expect("bull OB");
        assert_eq!((ob.lo, ob.hi), (99.0, 101.5));
        assert_eq!(ob.time_ms, 5 * 60_000);
        assert!(!ob.touched && ob.strength > 0.8);
        assert!(lv.iter().all(|l| l.kind != LevelKind::BearOrderBlock));
    }

    #[test]
    fn a_close_through_the_block_removes_it_and_a_dip_into_it_marks_it_touched() {
        let mut bars = bull_structure();
        bars.push(bar(11, 106.8, 106.9, 100.8, 101.2)); // trades into the block
        let lv = detect_levels(&bars, 0.1, 5.0);
        let ob = lv
            .iter()
            .find(|l| l.kind == LevelKind::BullOrderBlock)
            .expect("still there");
        assert!(ob.touched && ob.strength < 0.6);
        bars.push(bar(12, 101.2, 101.3, 98.0, 98.5)); // closes below it
        let lv = detect_levels(&bars, 0.1, 5.0);
        assert!(lv.iter().all(|l| l.kind != LevelKind::BullOrderBlock));
    }

    #[test]
    fn finds_an_unfilled_fair_value_gap_and_shrinks_a_partly_filled_one() {
        let lv = detect_levels(&bull_structure(), 0.1, 5.0);
        let gaps: Vec<&Level> = lv.iter().filter(|l| l.kind == LevelKind::BullFvg).collect();
        assert_eq!(
            gaps.len(),
            2,
            "newest two of the three gaps (5-7, 7-9, 8-10): {gaps:?}"
        );
        let gap = gaps
            .iter()
            .find(|l| (l.lo - 105.0).abs() < 1e-5)
            .expect("the 105.0..105.3 gap");
        assert!((gap.hi - 105.3).abs() < 1e-5 && !gap.touched, "{gap:?}");
        let mut bars = bull_structure();
        bars.push(bar(11, 106.8, 107.0, 105.1, 106.5)); // dips into the gap but not through it
        let lv = detect_levels(&bars, 0.1, 5.0);
        let gaps: Vec<&Level> = lv.iter().filter(|l| l.kind == LevelKind::BullFvg).collect();
        assert!(
            gaps.iter().all(|l| (l.lo - 106.0).abs() > 1e-3),
            "the 106.0..106.4 gap was filled by the dip: {gaps:?}"
        );
        let gap = gaps
            .iter()
            .find(|l| (l.lo - 105.0).abs() < 1e-5)
            .expect("shrunk gap");
        assert!((gap.hi - 105.1).abs() < 1e-4 && gap.touched, "{gap:?}");
    }

    #[test]
    fn swings_session_extremes_and_round_numbers_are_marked() {
        let lv = detect_levels(&bull_structure(), 0.1, 5.0);
        assert!(
            lv.iter()
                .any(|l| l.kind == LevelKind::SessionHigh && (l.mid() - 107.5).abs() < 1e-4)
        );
        assert!(
            lv.iter()
                .any(|l| l.kind == LevelKind::SessionLow && (l.mid() - 99.0).abs() < 1e-4)
        );
        let rounds: Vec<f32> = lv
            .iter()
            .filter(|l| l.kind == LevelKind::Round)
            .map(|l| l.mid())
            .collect();
        assert!(
            rounds.iter().any(|&r| (r - 105.0).abs() < 1e-4)
                && rounds.iter().any(|&r| (r - 100.0).abs() < 1e-4),
            "{rounds:?}"
        );
        // The 104 swing high was broken, so it is not a resting level any more.
        assert!(
            lv.iter()
                .all(|l| l.kind != LevelKind::SwingHigh || (l.mid() - 104.0).abs() > 0.2)
        );
        let low = lv
            .iter()
            .find(|l| l.kind == LevelKind::SwingLow)
            .expect("swing low");
        assert!((low.mid() - 99.0).abs() < 1e-4);
    }

    #[test]
    fn oanda_payload_becomes_long_and_short_clusters_inside_the_range() {
        let text = r#"{"symbol":"EURUSD","instrument":"EUR_USD","time":"2026-10-21T16:00:00Z","price":1.1230,"bucketWidth":0.0005,
            "buckets":[[1.1150,0.1,2.4],[1.1200,1.9,0.3],[1.1225,0.4,0.2],[1.1260,0.2,1.1],[1.1400,3.0,3.0]],"positions":[]}"#;
        let book = parse_oanda(text).unwrap();
        assert_eq!(book.buckets.len(), 5);
        let lv = oanda_levels(&book, 1.1100, 1.1300);
        let longs: Vec<&Level> = lv
            .iter()
            .filter(|l| l.kind == LevelKind::OandaLongOrders)
            .collect();
        let shorts: Vec<&Level> = lv
            .iter()
            .filter(|l| l.kind == LevelKind::OandaShortOrders)
            .collect();
        assert!((longs[0].mid() - 1.1200).abs() < 1e-5 && (longs[0].pct - 1.9).abs() < 1e-5);
        assert!((shorts[0].mid() - 1.1150).abs() < 1e-5);
        assert!(
            lv.iter().all(|l| l.mid() < 1.1300),
            "the 1.14 bucket is outside the chart"
        );
        assert_eq!(longs[0].label(), "OANDA LONG 1.9%");
        assert_eq!(
            longs.len(),
            1,
            "0.4% and 0.2% are under a third of the 1.9% cluster"
        );
        assert!(
            (shorts[0].hi - shorts[0].lo - 0.0005).abs() < 1e-6,
            "a lone bucket is one bucket wide"
        );
    }

    #[test]
    fn oanda_neighbouring_buckets_merge_into_one_fat_cluster() {
        let text = r#"{"symbol":"XAUUSD","instrument":"XAU_USD","time":"2026-10-22T15:00:00Z","price":4050.0,"bucketWidth":0.5,
            "buckets":[[4040.0,0.3,0.0],[4040.5,0.4,0.0],[4041.0,0.3,0.0],[4046.0,0.5,0.0],[4060.0,0.0,0.6],[4061.0,0.0,0.5],[4070.0,0.0,0.3]],"positions":[]}"#;
        let book = parse_oanda(text).unwrap();
        let lv = oanda_levels(&book, 4000.0, 4100.0);
        let longs: Vec<&Level> = lv
            .iter()
            .filter(|l| l.kind == LevelKind::OandaLongOrders)
            .collect();
        let shorts: Vec<&Level> = lv
            .iter()
            .filter(|l| l.kind == LevelKind::OandaShortOrders)
            .collect();
        // 4040..4041 is one cluster of 1.0% spanning three buckets; 4046 alone is 0.5%.
        assert_eq!(longs.len(), 2);
        assert!(
            (longs[0].pct - 1.0).abs() < 1e-5
                && (longs[0].lo - 4039.75).abs() < 1e-3
                && (longs[0].hi - 4041.25).abs() < 1e-3
        );
        assert!((longs[1].pct - 0.5).abs() < 1e-5);
        assert_eq!(longs[0].label(), "OANDA LONG 1.0%");
        // 4060 and 4061 are 2 widths apart (within the 2.5 gap) so they merge; 4070 is 0.3 of 1.1, under the share floor.
        assert_eq!(shorts.len(), 1);
        assert!((shorts[0].pct - 1.1).abs() < 1e-5);
        assert!(
            LevelKind::OandaLongOrders.band_alpha(1.0) > LevelKind::BullOrderBlock.band_alpha(1.0)
        );
        assert_eq!(LevelKind::OandaLongOrders.tall_band_fade(8.0), 1.0);
        assert!(
            (LevelKind::OandaShortOrders.tall_band_fade(160.0) - 0.15).abs() < 1e-6,
            "a band as tall as the chart is a tint"
        );
        assert_eq!(LevelKind::BullOrderBlock.tall_band_fade(300.0), 1.0);
        assert!(matches!(parse_oanda(r#"{"pending":true}"#), Err(e) if e == "pending"));
        assert!(parse_oanda(r#"{"error":"OANDA not configured"}"#).is_err());
    }

    #[test]
    fn compose_keeps_blocks_over_round_numbers_when_capped() {
        let lv = detect_levels(&bull_structure(), 0.1, 1.0); // many round numbers
        let kept = compose(lv, 4);
        assert_eq!(kept.len(), 4);
        assert_eq!(kept[0].kind, LevelKind::BullOrderBlock);
        assert!(kept.iter().all(|l| l.kind != LevelKind::Round));
    }

    #[test]
    fn tape_symbols_drop_the_slash() {
        assert_eq!(tape_symbol(Instrument::Silver), "XAGUSD");
        assert_eq!(tape_symbol(Instrument::Nasdaq), "NAS100");
    }
}
