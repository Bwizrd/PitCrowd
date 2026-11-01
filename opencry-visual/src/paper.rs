//! The paper book: the traders with different ideas, one honest scoreboard.
//!
//! Nine are the original archetypes (three on Jev, five on structure, one coin flip). The
//! rest are the Jev crew ([`JEV_CREW`]). Six of them trade Jev's common read (its lean, with
//! it or against it, its flat calls, or its level reaction: Lena fades the lean on absorption,
//! Kit on a sweep). The others each own a *question* of their own
//! ([`crate::jev::CREW_ASKS`]): a narrow, typed thing Jev is asked on every read, with a rule
//! that turns the answer into a side. Several are asked only when price has just touched a
//! level (the desk's touch-triggered reads carry a TRIGGER line), because that is where a
//! rapid classification can say something checkable: reject, break, sweep or stall. The
//! scorer grades each question on its own, so a question earns or loses its trader. They keep
//! the whole entry window and the cost floors but no other restriction.
//!
//! Every trader sees the same real inputs (the tape, bars built from it, the resting-order
//! levels, and Jev's read where one is fresh), pays the real spread on every fill (buys at the
//! ask, sells at the bid), runs the same session rules, and is scored in ticks at a flat stake
//! so pits are comparable. One of them, the control, flips a coin. If nobody beats the control
//! after costs over weeks, the floor has learned something cheaply. Trades and decisions are
//! appended to the relay's JSONL logs so the record outlives the browser tab.
//!
//! Cost discipline. The first replay showed the spread, not the ideas, deciding the table: the
//! gap filler paid 12 R of spread on 45 trades because his stops were a spread or two wide. So
//! every trader now works under [`Rules`]: the initial stop is never less than
//! [`MIN_STOP_SPREADS`] spreads (a tighter stop is widened to the floor and the trade's R is
//! measured on the widened stop), and an entry whose own target is less than
//! [`MIN_TARGET_SPREADS`] spreads away is vetoed outright. The stop floor began at three
//! spreads; the first live day of the full desk (2026-10-22, 1,700 trades) still paid 0.15 R
//! a trade in spread, two thirds of the net loss, so on Paul's call it is now twenty: the
//! spread is at most 5% of what a trade risks. An archetype may raise either floor
//! for itself, never lower it (`rules_respect_the_global_floors` enforces that).
//!
//! Sessions. Each archetype also names the UTC windows in which it will enter, because a
//! breakout idea at 19:00 is not the same idea as at 07:20. The control takes its coin flip at
//! the same cadence as before but only inside the union of the other traders' sessions: it
//! must have the same opportunity set as the widest trader and no more, otherwise beating it
//! would say nothing about the idea and something about the clock. Today that union is the
//! whole entry window, because Harold trades whenever Jev's read is fresh.
//!
//! Nothing here moves price or touches the pit engine.

use std::collections::VecDeque;

use bevy::prelude::*;
use serde::Serialize;

use crate::candles::{wall_clock_ms, Candle, CandleStore, Timeframe};
use crate::jev::{relay_url, AskRule, AskSide, CrewAsk, Direction, JevAsk, JevDesk, LevelSide, MarketInterpretation, ReadSource, ASK_ABSORBING, ASK_BAR_SHAPE, ASK_CARRY_THROUGH, ASK_DELTA_DIVERGENCE, ASK_MAGNET, ASK_STOP_RUN, ASK_TAPE_STATE, ASK_TOUCH_OUTCOME};
use crate::levels::{Level, LevelKind, LevelStore};
use crate::live_feed::{FeedMode, LiveFeed, PitFeedStatus, TickFlow};
use crate::market::Instrument;
use crate::pit::{format_price_with_commas, instrument_decimals, set_ui_text};
use crate::pit_engine::{tick_size, Side, XorShift};
use crate::scenario::MarketScenarioController;

/// Pounds per tick, every trader, every pit. Flat so the leaderboard compares ideas, not sizes.
pub const STAKE_PER_TICK: f32 = 1.0;
/// A Jev read older than this is not acted on.
pub const READ_FRESH_SECS: f64 = 30.0;
/// Entries allowed from .. to (UTC minutes of day); everyone is flat by `FLAT_AT`.
pub const ENTRY_FROM: i64 = 7 * 60;
pub const ENTRY_UNTIL: i64 = 20 * 60 + 30;
pub const FLAT_AT: i64 = 20 * 60 + 55;
/// Risk is sized in units of the pit's recent 1m bar range, never raw ticks, so a stop means
/// the same thing on Nasdaq as on EUR/USD. The unit is never smaller than this many ticks.
pub const MIN_UNIT_TICKS: f32 = 4.0;
/// A spread wider than this fraction of the unit is left alone by everyone.
pub const MAX_SPREAD_UNITS: f32 = 0.5;
/// Every target is at least this far from entry relative to the stop.
pub const MIN_REWARD_TO_RISK: f32 = 1.0;
/// Global cost floors, in spreads at entry (the spread is floored at one tick for this). No
/// trade risks less than `MIN_STOP_SPREADS` spreads; no trade is taken whose own target is
/// under `MIN_TARGET_SPREADS` spreads away. Archetypes may raise these, never lower them.
/// Twenty spreads caps the spread at 5% of the risk; since every target is pushed out to at
/// least `MIN_REWARD_TO_RISK` times the stop, it also means no trade is a scalp any more.
pub const MIN_STOP_SPREADS: f32 = 20.0;
pub const MIN_TARGET_SPREADS: f32 = 4.0;
/// 1m bars handed to a view: enough for a session VWAP and the opening ranges. The candle
/// store keeps at most 400 bars, so late in the day "session" means "since the oldest bar".
pub const VIEW_BARS: usize = 840;
/// Session opens that define an opening range, UTC minutes of day, and the range's length.
pub const LONDON_OPEN: i64 = 7 * 60;
pub const NEW_YORK_OPEN: i64 = 13 * 60 + 30;
pub const OPENING_RANGE_MINS: i64 = 15;
/// An opening range needs this many of its minutes to have printed a bar.
pub const OPENING_RANGE_MIN_BARS: usize = 10;
const WHY_KEEP: usize = 6;

// ---------------------------------------------------------------------------
// Sessions and rules
// ---------------------------------------------------------------------------

/// A half-open window of UTC minutes of day in which an archetype may enter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Session {
    pub from: i64,
    pub until: i64,
}

impl Session {
    pub const fn new(from_h: i64, from_m: i64, until_h: i64, until_m: i64) -> Self {
        Session { from: from_h * 60 + from_m, until: until_h * 60 + until_m }
    }

    pub fn contains(&self, minute: i64) -> bool {
        minute >= self.from && minute < self.until
    }
}

/// The whole entry window.
pub const ALL_DAY: Session = Session { from: ENTRY_FROM, until: ENTRY_UNTIL };
const LONDON: Session = Session::new(7, 0, 11, 0);
const NEW_YORK: Session = Session::new(13, 30, 16, 30);
const LONDON_KILL_ZONE: Session = Session::new(7, 0, 10, 0);
const NEW_YORK_KILL_ZONE: Session = Session::new(13, 30, 16, 0);
const LONDON_FIRST_HOUR: Session = Session::new(7, 0, 8, 0);
const NEW_YORK_FIRST_HOUR: Session = Session::new(13, 30, 14, 30);
const CORE_DAY: Session = Session::new(8, 0, 16, 30);
/// After the opening range has formed, for ninety minutes.
const LONDON_BREAKOUT: Session = Session::new(7, 15, 8, 45);
const NEW_YORK_BREAKOUT: Session = Session::new(13, 45, 15, 15);

/// What an archetype must respect regardless of its idea.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rules {
    pub min_stop_spreads: f32,
    pub min_target_spreads: f32,
    pub sessions: &'static [Session],
}

impl Rules {
    const fn standard(sessions: &'static [Session]) -> Self {
        Rules { min_stop_spreads: MIN_STOP_SPREADS, min_target_spreads: MIN_TARGET_SPREADS, sessions }
    }

    pub fn in_session(&self, minute: i64) -> bool {
        self.sessions.iter().any(|s| s.contains(minute))
    }

    /// The session this minute falls in, if any.
    pub fn session_at(&self, minute: i64) -> Option<Session> {
        self.sessions.iter().copied().find(|s| s.contains(minute))
    }

    /// Seconds until the current session ends (a cooldown of this length means "once per
    /// session"); zero outside every session.
    pub fn secs_to_session_end(&self, minute: i64) -> f64 {
        self.session_at(minute).map(|s| ((s.until - minute) * 60) as f64).unwrap_or(0.0)
    }
}

// ---------------------------------------------------------------------------
// Archetypes
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Archetype {
    MomentumScalper,
    TrendRider,
    ExhaustionFader,
    IctLevelTrader,
    GapFiller,
    OpeningRangeBreakout,
    SessionExtremeFader,
    VwapPullback,
    Control,
    // The Jev crew, see `JEV_CREW`: six on the common read...
    JevContrarian,
    JevDoubtFader,
    JevFlatFader,
    JevEverything,
    JevAbsorber,
    JevSweeper,
    // ...and the question traders, each on an ask of its own.
    JevOracle,
    JevAbsorption,
    JevStopRun,
    JevCarry,
    JevTape,
    JevClimax,
    JevLeg,
    JevMagnet,
    JevDivergence,
    /// A trader from the evolving pit: a registered genome (`crate::genome`), by seat. Not in
    /// `ALL`; the paper desk seats and retires them as `evolve/pit.json` says.
    Genome(u16),
}

/// Rows carry the archetype as its variant name, as the derive used to write it; a genome is
/// just "Genome" (its id travels in the trade's `genome` field).
impl Serialize for Archetype {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Archetype::Genome(_) => s.serialize_str("Genome"),
            other => s.serialize_str(&format!("{other:?}")),
        }
    }
}

impl Archetype {
    pub const ALL: [Archetype; 24] = [
        Archetype::MomentumScalper,
        Archetype::TrendRider,
        Archetype::ExhaustionFader,
        Archetype::IctLevelTrader,
        Archetype::GapFiller,
        Archetype::OpeningRangeBreakout,
        Archetype::SessionExtremeFader,
        Archetype::VwapPullback,
        Archetype::Control,
        Archetype::JevContrarian,
        Archetype::JevDoubtFader,
        Archetype::JevFlatFader,
        Archetype::JevEverything,
        Archetype::JevAbsorber,
        Archetype::JevSweeper,
        Archetype::JevOracle,
        Archetype::JevAbsorption,
        Archetype::JevStopRun,
        Archetype::JevCarry,
        Archetype::JevTape,
        Archetype::JevClimax,
        Archetype::JevLeg,
        Archetype::JevMagnet,
        Archetype::JevDivergence,
    ];

    /// The crew member's style, for those who trade Jev as data.
    pub fn crew(self) -> Option<&'static JevStyle> {
        JEV_CREW.iter().find(|s| s.arch == self)
    }

    pub fn name(self) -> &'static str {
        if let Some(s) = self.crew() {
            return s.name;
        }
        match self {
            Archetype::Genome(i) => crate::genome::seat_name(i),
            Archetype::MomentumScalper => "Vinnie",
            Archetype::TrendRider => "Grace",
            Archetype::ExhaustionFader => "Harold",
            Archetype::IctLevelTrader => "Priya",
            Archetype::GapFiller => "Tomasz",
            Archetype::OpeningRangeBreakout => "Maggie",
            Archetype::SessionExtremeFader => "Dev",
            Archetype::VwapPullback => "Rosa",
            Archetype::Control => "Colin",
            _ => "crew",
        }
    }

    pub fn label(self) -> &'static str {
        if let Some(s) = self.crew() {
            return s.label;
        }
        match self {
            Archetype::Genome(i) => crate::genome::seat_tag(i),
            Archetype::MomentumScalper => "scalper",
            Archetype::TrendRider => "trend",
            Archetype::ExhaustionFader => "fader",
            Archetype::IctLevelTrader => "ICT",
            Archetype::GapFiller => "gaps",
            Archetype::OpeningRangeBreakout => "ORB",
            Archetype::SessionExtremeFader => "extremes",
            Archetype::VwapPullback => "VWAP",
            Archetype::Control => "CONTROL",
            _ => "crew",
        }
    }

    pub fn describe(self) -> &'static str {
        if let Some(s) = self.crew() {
            return s.creed;
        }
        match self {
            Archetype::Genome(i) => crate::genome::seat_creed(i),
            Archetype::MomentumScalper => "Jev says go with conviction and no exhaustion: in for 8 ticks, out at 6, three minutes max.",
            Archetype::TrendRider => "Jev agrees with the last 20 bars: rides it with a trailing stop, up to an hour.",
            Archetype::ExhaustionFader => "Jev sees the side in control tiring: fades it toward the next level.",
            Archetype::IctLevelTrader => "No Jev. A swept swing low, then a return into a fresh order block or gap: stop under it, target the next swing.",
            Archetype::GapFiller => "No Jev. Trades the first return into an unfilled fair value gap, stop beyond it.",
            Archetype::OpeningRangeBreakout => "No Jev. The first 15 minutes of London and New York draw the box: first 1m close outside it, stop at the box's middle, 1.5 R, once a session.",
            Archetype::SessionExtremeFader => "No Jev. A new session high or low on fading tick rate and a rejection wick: fades it back to VWAP, unless a block or gap is in the way.",
            Archetype::VwapPullback => "No Jev. Twenty minutes a unit clear of VWAP is a trend: buys the first touch back to VWAP, stop a unit behind it, target the session extreme.",
            Archetype::Control => "Flips a coin every few minutes inside the others' hours, same costs, same exits. The bar everyone must clear.",
            _ => "Trades Jev's read.",
        }
    }

    pub fn uses_jev(self) -> bool {
        self.crew().is_some() || matches!(self, Archetype::MomentumScalper | Archetype::TrendRider | Archetype::ExhaustionFader)
    }

    /// Cost floors and entry hours. The control's hours are the union of everyone else's (see
    /// the module doc); `the_control_shares_the_widest_opportunity_set` keeps that true.
    pub fn rules(self) -> Rules {
        const VINNIE: [Session; 2] = [LONDON, NEW_YORK];
        const GRACE: [Session; 1] = [CORE_DAY];
        const HAROLD: [Session; 1] = [ALL_DAY];
        const PRIYA: [Session; 2] = [LONDON_KILL_ZONE, NEW_YORK_KILL_ZONE];
        const TOMASZ: [Session; 2] = [LONDON_FIRST_HOUR, NEW_YORK_FIRST_HOUR];
        const MAGGIE: [Session; 2] = [LONDON_BREAKOUT, NEW_YORK_BREAKOUT];
        const DEV: [Session; 1] = [CORE_DAY];
        const ROSA: [Session; 1] = [CORE_DAY];
        const COLIN: [Session; 1] = [ALL_DAY];
        const CREW: [Session; 1] = [ALL_DAY];
        if self.crew().is_some() {
            return Rules::standard(&CREW);
        }
        match self {
            Archetype::MomentumScalper => Rules::standard(&VINNIE),
            Archetype::TrendRider => Rules::standard(&GRACE),
            Archetype::ExhaustionFader => Rules::standard(&HAROLD),
            Archetype::IctLevelTrader => Rules::standard(&PRIYA),
            Archetype::GapFiller => Rules::standard(&TOMASZ),
            Archetype::OpeningRangeBreakout => Rules::standard(&MAGGIE),
            Archetype::SessionExtremeFader => Rules::standard(&DEV),
            Archetype::VwapPullback => Rules::standard(&ROSA),
            Archetype::Control => Rules::standard(&COLIN),
            _ => Rules::standard(&CREW),
        }
    }

    pub fn color(self) -> Color {
        if let Some(s) = self.crew() {
            return Color::srgb(s.color.0, s.color.1, s.color.2);
        }
        match self {
            Archetype::Genome(i) => {
                let (r, g, b) = crate::genome::seat_rgb(i);
                Color::srgb(r, g, b)
            }
            Archetype::MomentumScalper => Color::srgb(0.95, 0.60, 0.25),
            Archetype::TrendRider => Color::srgb(0.40, 0.80, 1.0),
            Archetype::ExhaustionFader => Color::srgb(0.85, 0.55, 0.95),
            Archetype::IctLevelTrader => Color::srgb(0.35, 0.95, 0.60),
            Archetype::GapFiller => Color::srgb(0.98, 0.85, 0.35),
            Archetype::OpeningRangeBreakout => Color::srgb(0.98, 0.45, 0.45),
            Archetype::SessionExtremeFader => Color::srgb(0.50, 0.90, 0.88),
            Archetype::VwapPullback => Color::srgb(0.95, 0.70, 0.90),
            Archetype::Control => Color::srgb(0.75, 0.78, 0.85),
            _ => Color::WHITE,
        }
    }
}

// ---------------------------------------------------------------------------
// The Jev crew: twenty traders who all act on Jev's read, each differently
// ---------------------------------------------------------------------------

/// With Jev's direction or against it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lean {
    Follow,
    Fade,
}

/// One crew member's way of trading Jev, as data: which reads it acts on and how it gets
/// out. Thresholds are set against what Jev actually says (9,515 reads in the 24 h to
/// 2026-10-22 17:00 UTC: 52% of reads lean up or down; direction confidence median 0.28,
/// p75 0.41, p90 0.55; conviction median 0.37, p75 0.62, p90 0.94; continuation median
/// 0.42, p90 0.52; exhaustion median 0.48, p90 0.56; level reaction absorbing 38%,
/// no_level 28%, sweeping 20%, breaking 8%, rejecting 6%), so each member fires on a known
/// slice of reads. The original three Jev traders ask for conviction 1.0 to 1.2 and
/// exhaustion 0.65, which Jev gives a few times a day; this table is why they sit out.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct JevStyle {
    pub arch: Archetype,
    pub name: &'static str,
    pub label: &'static str,
    pub creed: &'static str,
    pub color: (f32, f32, f32),
    pub lean: Lean,
    /// Direction confidence floor, 0..1 (a flat read never passes unless `flat_only`).
    pub min_confidence: f32,
    /// Conviction window, 0..2.
    pub min_conviction: f32,
    pub max_conviction: f32,
    pub min_continuation: f32,
    /// Exhaustion window, 0..1: a floor for the tired-faders, a ceiling for the fresh-legs.
    pub min_exhaustion: f32,
    pub max_exhaustion: f32,
    /// Only these level reactions, when non-empty.
    pub reactions: &'static [&'static str],
    /// Trades only Jev's flat reads, against the last five bars' move.
    pub flat_only: bool,
    /// Acts only within this many seconds of a read arriving (0: any fresh read).
    pub first_mover_secs: f64,
    /// The member's own question and rule. When set, the filters above are ignored: the
    /// answer alone decides the side (the read's lean only matters to a `NoulLean` rule).
    pub ask: Option<CrewAsk>,
    /// Risk and exits in units (`View::unit`, the mean 1m range).
    pub stop_units: f32,
    /// 0: no target; the trail or the clock closes it.
    pub target_units: f32,
    /// 0: no trail.
    pub trail_units: f32,
    pub time_stop_secs: f64,
    pub cooldown_secs: f64,
}

const CREW_BASE: JevStyle = JevStyle {
    arch: Archetype::JevEverything,
    name: "",
    label: "",
    creed: "",
    color: (1.0, 1.0, 1.0),
    lean: Lean::Follow,
    min_confidence: 0.0,
    min_conviction: 0.0,
    max_conviction: 2.0,
    min_continuation: 0.0,
    min_exhaustion: 0.0,
    max_exhaustion: 1.0,
    reactions: &[],
    flat_only: false,
    first_mover_secs: 0.0,
    ask: None,
    stop_units: 1.0,
    target_units: 1.0,
    trail_units: 0.0,
    time_stop_secs: 600.0,
    cooldown_secs: 60.0,
};

pub static JEV_CREW: [JevStyle; 15] = [
    // On the common read.
    JevStyle { arch: Archetype::JevContrarian, name: "Gus", label: "contrarian", color: (0.60, 0.45, 1.0), lean: Lean::Fade, min_confidence: 0.30, time_stop_secs: 900.0,
        creed: "Whatever Jev says at 30% or better, Gus does the opposite. If Jev is right, Gus pays for it.", ..CREW_BASE },
    JevStyle { arch: Archetype::JevDoubtFader, name: "Ivo", label: "doubt fader", color: (1.0, 0.45, 0.70), lean: Lean::Fade, min_confidence: 0.20, max_conviction: 0.30, time_stop_secs: 600.0, cooldown_secs: 45.0,
        creed: "Fades a lean Jev holds without conviction (0.3 or less): a weak read is noise to lean against.", ..CREW_BASE },
    JevStyle { arch: Archetype::JevFlatFader, name: "Sol", label: "flat fader", color: (0.80, 0.80, 0.90), flat_only: true, time_stop_secs: 900.0,
        creed: "Trades only Jev's flat reads: after a five-bar move of half a unit or more, bets it comes back.", ..CREW_BASE },
    JevStyle { arch: Archetype::JevEverything, name: "Tess", label: "everything", color: (1.0, 0.60, 0.85), min_confidence: 0.10, time_stop_secs: 600.0, cooldown_secs: 20.0,
        creed: "Every lean Jev has, however faint: the raw read, traded as often as the cooldown allows.", ..CREW_BASE },
    // Lena and Kit were retired on 2026-10-23 with the other filter clones and reinstated on
    // 2026-10-24 at Paul's call: under the 20-spread stops they were the two best Jev traders on
    // the desk (Lena 16 trades +6.5 R, Kit 11 trades +2.1 R), small samples to be confirmed.
    JevStyle { arch: Archetype::JevAbsorber, name: "Lena", label: "absorber", color: (0.75, 0.85, 0.55), lean: Lean::Fade, reactions: &["absorbing"], min_confidence: 0.20, time_stop_secs: 900.0,
        creed: "Only when Jev sees absorption at a level: someone big is taking the other side, so she joins them.", ..CREW_BASE },
    JevStyle { arch: Archetype::JevSweeper, name: "Kit", label: "sweeper", color: (0.90, 0.60, 0.50), lean: Lean::Fade, reactions: &["sweeping"], target_units: 1.5, time_stop_secs: 1200.0,
        creed: "Only on a sweep: stops were run, so Kit fades the lean back through the level.", ..CREW_BASE },
    // Each on a question of its own. The touch questions are answered only on touch reads.
    JevStyle { arch: Archetype::JevOracle, name: "Vera", label: "oracle", color: (0.95, 0.85, 0.30), first_mover_secs: 60.0, target_units: 1.5, time_stop_secs: 900.0,
        ask: Some(CrewAsk { question: &ASK_TOUCH_OUTCOME, rule: AskRule::LevelChoice { map: &[("reject", LevelSide::Away), ("sweep", LevelSide::Away), ("break", LevelSide::Through)], min: 0.45 } }),
        creed: "Asks Jev what the touched level will have done in fifteen minutes, and trades an answer held at 45% or better: away on reject or sweep, through on break.", ..CREW_BASE },
    JevStyle { arch: Archetype::JevAbsorption, name: "Wren", label: "absorption", color: (0.55, 0.90, 0.60), first_mover_secs: 60.0, time_stop_secs: 900.0,
        ask: Some(CrewAsk { question: &ASK_ABSORBING, rule: AskRule::NoulLevel { min: 0.6, side: LevelSide::Away } }),
        creed: "Asks whether size is absorbing the arrivals at the touched level; at 60% yes, trades away from it with whoever is absorbing.", ..CREW_BASE },
    JevStyle { arch: Archetype::JevStopRun, name: "Xi", label: "stop run", color: (0.95, 0.55, 0.55), first_mover_secs: 60.0, target_units: 1.5, time_stop_secs: 900.0,
        ask: Some(CrewAsk { question: &ASK_STOP_RUN, rule: AskRule::NoulLevel { min: 0.6, side: LevelSide::Away } }),
        creed: "Asks whether the push into the level was a stop run; at 60% yes, fades it back the way it came.", ..CREW_BASE },
    JevStyle { arch: Archetype::JevCarry, name: "Yusuf", label: "carry", color: (0.40, 0.80, 1.0), first_mover_secs: 60.0, target_units: 0.0, trail_units: 1.0, time_stop_secs: 2400.0, cooldown_secs: 90.0,
        ask: Some(CrewAsk { question: &ASK_CARRY_THROUGH, rule: AskRule::NoulLevel { min: 0.65, side: LevelSide::Through } }),
        creed: "Asks whether a break of the touched level would carry a unit; at 65% yes, goes through it on a one-unit trail.", ..CREW_BASE },
    JevStyle { arch: Archetype::JevTape, name: "Zara", label: "tape", color: (0.98, 0.70, 0.35), first_mover_secs: 30.0, time_stop_secs: 600.0, cooldown_secs: 45.0,
        ask: Some(CrewAsk { question: &ASK_TAPE_STATE, rule: AskRule::Choice { map: &[("one_way_buying", AskSide::Buy), ("one_way_selling", AskSide::Sell)], min: 0.5 } }),
        creed: "Asks only about the last twenty seconds of tape; joins one-way buying or selling, a unit each way, ten minutes.", ..CREW_BASE },
    JevStyle { arch: Archetype::JevClimax, name: "Ana", label: "climax", color: (0.85, 0.45, 0.95), target_units: 1.5, time_stop_secs: 1200.0,
        ask: Some(CrewAsk { question: &ASK_BAR_SHAPE, rule: AskRule::Choice { map: &[("climax_up", AskSide::Sell), ("climax_down", AskSide::Buy)], min: 0.4 } }),
        creed: "Asks for the shape of the last twenty bars and fades a climax: sells a buying climax, buys a selling one.", ..CREW_BASE },
    JevStyle { arch: Archetype::JevLeg, name: "Ben", label: "leg", color: (0.35, 0.95, 0.85), target_units: 0.0, trail_units: 1.0, time_stop_secs: 1800.0, cooldown_secs: 90.0,
        ask: Some(CrewAsk { question: &ASK_BAR_SHAPE, rule: AskRule::Choice { map: &[("trend_up", AskSide::Buy), ("trend_down", AskSide::Sell)], min: 0.4 } }),
        creed: "Same question as Ana, the other answer: joins a steady trend leg on a one-unit trail.", ..CREW_BASE },
    JevStyle { arch: Archetype::JevMagnet, name: "Cy", label: "magnet", color: (0.70, 0.75, 0.95), time_stop_secs: 900.0,
        ask: Some(CrewAsk { question: &ASK_MAGNET, rule: AskRule::Choice { map: &[("above", AskSide::Buy), ("below", AskSide::Sell)], min: 0.5 } }),
        creed: "Asks which listed level price reaches first, the one above or the one below, and trades toward it when Jev is at least half sure.", ..CREW_BASE },
    JevStyle { arch: Archetype::JevDivergence, name: "Dee", label: "divergence", color: (0.95, 0.95, 0.55), target_units: 1.5, time_stop_secs: 900.0,
        ask: Some(CrewAsk { question: &ASK_DELTA_DIVERGENCE, rule: AskRule::NoulLean { min: 0.6, with_lean: false } }),
        creed: "Asks whether the move is making new extremes on shrinking delta and tick rate; at 60% yes, fades Jev's lean.", ..CREW_BASE },
];

fn fmt_secs(s: f64) -> String {
    if s >= 60.0 { format!("{:.0} min", s / 60.0) } else { format!("{s:.0} s") }
}

impl JevStyle {
    /// The style's filters and exits in words, for the trader panel and the guide.
    pub fn rules_text(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if let Some(a) = self.ask {
            parts.push(format!(
                "on Jev's answer to its own question{} \"{}\" ({})",
                if a.question.touch_only { ", asked when price has just touched a level," } else { "" },
                a.question.instructions,
                a.rule.text()
            ));
        } else if self.flat_only {
            parts.push("only on a FLAT read, fading the last five bars' move when it is half a unit or more".into());
        } else {
            parts.push(format!("{} Jev's lean", if self.lean == Lean::Follow { "with" } else { "against" }));
            if self.min_confidence > 0.0 {
                parts.push(format!("direction confidence >= {:.0}%", self.min_confidence * 100.0));
            }
            if self.min_conviction > 0.0 {
                parts.push(format!("conviction >= {:.2} of 2", self.min_conviction));
            }
            if self.max_conviction < 2.0 {
                parts.push(format!("conviction <= {:.2} of 2", self.max_conviction));
            }
            if self.min_continuation > 0.0 {
                parts.push(format!("continuation >= {:.0}%", self.min_continuation * 100.0));
            }
            if self.min_exhaustion > 0.0 {
                parts.push(format!("exhaustion >= {:.0}%", self.min_exhaustion * 100.0));
            }
            if self.max_exhaustion < 1.0 {
                parts.push(format!("exhaustion <= {:.0}%", self.max_exhaustion * 100.0));
            }
        }
        if !self.reactions.is_empty() {
            parts.push(format!("only when Jev's level reaction is {}", self.reactions.join(" or ")));
        }
        if self.first_mover_secs > 0.0 {
            parts.push(format!("only in the first {:.0} s after a read arrives", self.first_mover_secs));
        }
        let mut exits = vec![format!("stop {:.2} units", self.stop_units)];
        if self.target_units > 0.0 {
            exits.push(format!("target {:.2} units", self.target_units));
        }
        if self.trail_units > 0.0 {
            exits.push(format!("trail {:.2} units behind the best price once a unit onside", self.trail_units));
        }
        exits.push(format!("time stop {}", fmt_secs(self.time_stop_secs)));
        exits.push(format!("{} cooldown after a trade", fmt_secs(self.cooldown_secs)));
        format!("Enters {}. Exits: {}. A unit is the mean 1m bar range of the last 20 bars.", parts.join(", "), exits.join(", "))
    }
}

impl Archetype {
    /// The rule in words: the crew's table row, or the original's hand-written idea.
    pub fn strategy_text(self) -> String {
        if let Some(s) = self.crew() {
            return s.rules_text();
        }
        match self {
            Archetype::Genome(i) if crate::genome::seat_name(i).ends_with("-J") => "A Jev twin from the evolving pit: whenever its genome enters, it asks Jev one question about the 15m chart (no symbol, no dates) and takes the same trade only on a yes. Never cut on its own. The test is the genome's trades split by Jev's answer: Jev adds something only if, at 300 tagged trades, the yes trades beat the no trades by a margin that survives the number of twins judged.".into(),
            Archetype::Genome(i) if crate::genome::seat_tag(i) == "manual" => "Your own paper trades, booked by hand from the trade ticket on the pit you are looking at: buy or sell at the touch, stop in ATR of 15m bars, target in R, held until the stop, the target or your Close. Paper only: nothing reaches a broker.".into(),
            Archetype::Genome(i) if crate::genome::seat_tag(i) == "holder" => "A holder on the show floor: one of edge-lab's daily or 4h strategies, traded exactly as edge-lab wrote it, its own entries and its own exits, holding overnight, on one symbol. One R is two ATR of its own bars; a safety stop at 5 R is the only exit that is not the strategy's. Seated because its backtest made money in 2021-24 and in 2025-26; unproven, and it never counts toward graduation.".into(),
            Archetype::Genome(i) if crate::genome::seat_tag(i).starts_with("adapter") => "An adapter on the show floor: it carries a handful of tactics (genomes) and every morning at 06:45 UTC picks the one to trade today, either the one with the best record over the last 20 days of bars (code) or the one Jev rates likeliest to make money (Jev). Every switch is logged. Adapters come in pairs with the same tactics, one code and one Jev, so the pair shows whether Jev chooses better than the scoreboard.".into(),
            Archetype::Genome(i) if crate::genome::seat_tag(i) == "show" => "A genome on the show floor: an ordinary genome that made money in 2021-24 and in 2025-26 but did not pass the gate. Unproven, seated to be watched; it never counts toward graduation. Cut by the same sequential test as the gate's genomes, or at t <= -2.5 after 40 trades.".into(),
            Archetype::Genome(_) => "A genome from the evolving pit: no Jev, decides at each 15m bar close, in its own hours (above). Seated because its backtest passed the gate (2021-24 t >= 2, 2025-26 holdout positive). Judged forward by Wald's sequential test against its own holdout edge, with a safety cut at t <= -3 after 30 trades, or idle 21 days; graduates at 300 trades with +0.05R and a deflated Sharpe of 0.95.".into(),
            Archetype::MomentumScalper => "Needs a fresh Jev read leaning up or down with conviction >= 1.2 of 2, continuation >= 60% and exhaustion under 50%, and a spread under 35% of the unit. Buys the ask / sells the bid for a one-unit target with a 0.75-unit stop, three-minute time stop, one-minute cooldown. London and New York hours.".into(),
            Archetype::TrendRider => "Needs Jev's lean to agree with the last 20 bars (drift of 8 ticks or more the same way), conviction >= 1.0 and exhaustion under 40%. Stop behind the nearest swing (0.75 to 2.5 units), no target: a one-unit trail once a unit onside, up to an hour, two-minute cooldown. Core day.".into(),
            Archetype::ExhaustionFader => "Needs Jev's exhaustion >= 65%: the side in control is tiring, so he trades against Jev's lean toward the next level. Whole entry window.".into(),
            Archetype::IctLevelTrader => "No Jev. Waits for a swing low (or high) to be swept, then a return into a fresh order block or fair value gap: stop beyond the block, target the next swing. London and New York kill zones.".into(),
            Archetype::GapFiller => "No Jev. Trades the first return into an unfilled fair value gap, stop beyond the gap. First hour of London and of New York only.".into(),
            Archetype::OpeningRangeBreakout => "No Jev. The first 15 minutes of London (07:00 UTC) and New York (13:30 UTC) draw a box; the first 1m close outside it is the entry, stop at the box's middle, target 1.5 R, time stop 90 minutes, once a session.".into(),
            Archetype::SessionExtremeFader => "No Jev. A new session high or low printed on fading tick counts with a rejection wick: fades it, stop a unit beyond the extreme, target VWAP (or two units if VWAP is further), unless an order block or gap is in the way. Core day.".into(),
            Archetype::VwapPullback => "No Jev. Twenty minutes with price a unit clear of session VWAP is a trend; buys (or sells) the first touch back to VWAP, stop a unit beyond VWAP, target the session extreme. Core day.".into(),
            Archetype::Control => "Flips a coin every three to eight minutes in any pit, inside the union of everyone else's hours. One-unit stop, one-unit target, ten-minute time stop. Same fills, same costs: the bar the others must clear.".into(),
            _ => self.describe().to_string(),
        }
    }
}

/// The crew's rule: Jev's read through the member's filters, then its exits.
fn decide_crew(s: &JevStyle, v: &View) -> Result<Entry, String> {
    let r = v.read.ok_or_else(|| "no fresh Jev read".to_string())?;
    let age = v.now - r.at;
    if s.first_mover_secs > 0.0 && age > s.first_mover_secs {
        return Err(format!("read is {age:.0}s old; acts in its first {:.0}s", s.first_mover_secs));
    }
    if let Some(a) = s.ask {
        return decide_ask(s, a, r, v);
    }
    if !s.reactions.is_empty() && !s.reactions.contains(&r.reaction.as_str()) {
        return Err(format!("level {}; wants {}", r.reaction, s.reactions.join("/")));
    }
    let u = v.unit();
    let pct = r.direction_confidence * 100.0;
    let (side, why) = if s.flat_only {
        if r.direction != Direction::Flat {
            return Err(format!("Jev leans {} {pct:.0}%; waits for flat", r.direction.label()));
        }
        let d = v.net_change_ticks(5);
        if d.abs() < 0.5 * u {
            return Err(format!("Jev flat and the last 5 bars moved {d:+.0} ticks, under half a unit ({:.0})", 0.5 * u));
        }
        let side = if d > 0.0 { Side::Sell } else { Side::Buy };
        (side, format!("Jev flat {pct:.0}% after a {d:+.0}-tick 5-bar move: fading it"))
    } else {
        let lean_side = match r.direction {
            Direction::Up => Side::Buy,
            Direction::Down => Side::Sell,
            Direction::Flat => return Err(format!("Jev: flat {pct:.0}%")),
        };
        if r.direction_confidence < s.min_confidence {
            return Err(format!("Jev {} at {pct:.0}% < {:.0}%", r.direction.label(), s.min_confidence * 100.0));
        }
        if r.conviction < s.min_conviction {
            return Err(format!("conviction {:.2} < {:.2}", r.conviction, s.min_conviction));
        }
        if r.conviction > s.max_conviction {
            return Err(format!("conviction {:.2} > {:.2}: too sure to fade", r.conviction, s.max_conviction));
        }
        if r.continuation < s.min_continuation {
            return Err(format!("continuation {:.0}% < {:.0}%", r.continuation * 100.0, s.min_continuation * 100.0));
        }
        if r.exhaustion > s.max_exhaustion {
            return Err(format!("exhaustion {:.0}% > {:.0}%", r.exhaustion * 100.0, s.max_exhaustion * 100.0));
        }
        if r.exhaustion < s.min_exhaustion {
            return Err(format!("exhaustion {:.0}% < {:.0}%: not tired yet", r.exhaustion * 100.0, s.min_exhaustion * 100.0));
        }
        let side = match s.lean {
            Lean::Follow => lean_side,
            Lean::Fade => lean_side.opposite(),
        };
        let how = if s.lean == Lean::Follow { "with" } else { "against" };
        (
            side,
            format!(
                "{how} Jev {} {pct:.0}% (conviction {:.2}, continuation {:.0}%, exhaustion {:.0}%, {})",
                r.direction.label(),
                r.conviction,
                r.continuation * 100.0,
                r.exhaustion * 100.0,
                r.reaction
            ),
        )
    };
    Ok(crew_entry(s, v, side, s.stop_units * u, why))
}

/// A question trader: its own answer decides the side; the touch, when there is one, sets
/// the stop beyond the level's far edge (a quarter unit past it) if that is wider than the
/// style's own stop.
fn decide_ask(s: &JevStyle, a: CrewAsk, r: &MarketInterpretation, v: &View) -> Result<Entry, String> {
    if a.question.touch_only && r.trigger.is_none() {
        return Err("waits for a touch read".into());
    }
    let ans = r.answers.get(a.question.key).ok_or_else(|| format!("Jev was not asked {}", a.question.key))?;
    let side = match a.rule.side(a.question.key, ans, r.trigger.as_ref(), r.direction)? {
        AskSide::Buy => Side::Buy,
        AskSide::Sell => Side::Sell,
    };
    let u = v.unit();
    let entry = if side.is_buy() { v.ask } else { v.bid };
    let mut stop_ticks = s.stop_units * u;
    let mut at = String::new();
    if let Some(t) = &r.trigger {
        // Away from the level: the stop goes past its far edge. Through it: past the near edge.
        let far = if t.side.contains("above") { t.lo } else { t.hi };
        let near = if t.side.contains("above") { t.hi } else { t.lo };
        let beyond = if (side.is_buy() && entry > t.hi) || (!side.is_buy() && entry < t.lo) { far } else { near };
        let needed = (entry - beyond).abs() / v.tick + 0.25 * u;
        stop_ticks = stop_ticks.max(needed);
        at = format!(" at {} {}..{} {}", t.level, fmt_px(v.inst, t.lo), fmt_px(v.inst, t.hi), t.side);
    }
    let why = format!("Jev {} = {}{at}", a.question.key, JevAsk::answer_text(ans));
    Ok(crew_entry(s, v, side, stop_ticks, why))
}

/// The style's exits around a side.
fn crew_entry(s: &JevStyle, v: &View, side: Side, stop_ticks: f32, why: String) -> Entry {
    let u = v.unit();
    let entry = if side.is_buy() { v.ask } else { v.bid };
    let target = (s.target_units > 0.0).then(|| if side.is_buy() { entry + s.target_units * u * v.tick } else { entry - s.target_units * u * v.tick });
    let trail = (s.trail_units > 0.0).then(|| Trail { arm_ticks: s.trail_units * u, lock_ticks: 0.25 * u, trail_ticks: s.trail_units * u });
    Entry { side, stop_ticks, target, trail, time_stop_secs: s.time_stop_secs, cooldown_secs: s.cooldown_secs, why }
}

// ---------------------------------------------------------------------------
// Positions and trades
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Trail {
    /// Once the trade is this many ticks onside...
    pub arm_ticks: f32,
    /// ...move the stop to entry plus this,
    pub lock_ticks: f32,
    /// and from then on keep it this far behind the best price.
    pub trail_ticks: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Position {
    pub inst: Instrument,
    pub side: Side,
    pub entry: f32,
    pub stop: f32,
    pub target: Option<f32>,
    pub trail: Option<Trail>,
    pub opened_ms: i64,
    pub opened_at: f64,
    pub time_stop_secs: f64,
    pub best: f32,
    /// Initial stop distance in ticks: one R.
    pub stop_ticks: f32,
    pub spread_ticks_in: f32,
    pub why: String,
    pub observation_hash: String,
    pub read_source: Option<ReadSource>,
    /// A holder's position: not closed at `FLAT_AT`; it ends on a stop or on its strategy's exit.
    pub overnight: bool,
}

impl Position {
    /// Mark-to-market in ticks at the price the position would close at now.
    pub fn open_ticks(&self, bid: f32, ask: f32, tick: f32) -> f32 {
        match self.side {
            Side::Buy => (bid - self.entry) / tick,
            Side::Sell => (self.entry - ask) / tick,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Trade {
    pub trader: &'static str,
    pub archetype: Archetype,
    pub symbol: &'static str,
    pub side: &'static str,
    pub entry: f32,
    pub exit: f32,
    pub opened_ms: i64,
    pub closed_ms: i64,
    pub pnl_ticks: f32,
    pub pnl_gbp: f32,
    /// Profit in multiples of the initial stop: the number that compares pits and traders.
    pub pnl_r: f32,
    pub stop_ticks: f32,
    pub spread_ticks_in: f32,
    pub why_in: String,
    pub why_out: String,
    pub observation_hash: String,
    pub read_source: Option<ReadSource>,
    /// The genome's id, for a trader from the evolving pit; how `evolve.py` finds its trades.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub genome: Option<&'static str>,
}

#[derive(Debug, Clone, Serialize)]
struct Decision {
    trader: &'static str,
    archetype: Archetype,
    symbol: &'static str,
    at_ms: i64,
    action: &'static str,
    side: Option<&'static str>,
    price: f32,
    bid: f32,
    ask: f32,
    why: String,
    observation_hash: Option<String>,
}

fn side_str(side: Side) -> &'static str {
    if side.is_buy() { "long" } else { "short" }
}

// ---------------------------------------------------------------------------
// What a trader sees for one pit on one tick
// ---------------------------------------------------------------------------

pub struct View<'a> {
    pub inst: Instrument,
    pub bid: f32,
    pub ask: f32,
    pub tick: f32,
    pub spread_ticks: f32,
    pub now: f64,
    pub wall_ms: i64,
    /// Newest last, 1m bars built from the tape (and backfill).
    pub bars: &'a [Candle],
    pub flow: TickFlow,
    pub rate: f32,
    pub levels: &'a [Level],
    /// A fresh Jev read of this pit, if there is one.
    pub read: Option<&'a MarketInterpretation>,
    pub prev_bid: f32,
}

impl View<'_> {
    /// The risk unit: mean 1m bar range over the last 20 bars, in ticks, floored.
    pub fn unit(&self) -> f32 {
        let k = self.bars.len().min(20);
        if k == 0 {
            return MIN_UNIT_TICKS;
        }
        let mean = self.bars[self.bars.len() - k..].iter().map(|b| (b.high - b.low) / self.tick).sum::<f32>() / k as f32;
        mean.max(MIN_UNIT_TICKS)
    }

    pub fn net_change_ticks(&self, n: usize) -> f32 {
        let k = self.bars.len().min(n);
        if k < 2 {
            return 0.0;
        }
        (self.bars[self.bars.len() - 1].close - self.bars[self.bars.len() - k].open) / self.tick
    }

    fn levels_of(&self, kind: LevelKind) -> impl Iterator<Item = &Level> {
        self.levels.iter().filter(move |l| l.kind == kind)
    }

    /// Nearest level of these kinds strictly above (for longs) or below (for shorts) `from`,
    /// at least `min_ticks` away.
    pub fn next_level(&self, side: Side, from: f32, kinds: &[LevelKind], min_ticks: f32) -> Option<f32> {
        let mut best: Option<f32> = None;
        for l in self.levels.iter().filter(|l| kinds.contains(&l.kind)) {
            let px = if side.is_buy() { l.lo } else { l.hi };
            let dist = if side.is_buy() { (px - from) / self.tick } else { (from - px) / self.tick };
            if dist >= min_ticks && best.is_none_or(|b| if side.is_buy() { px < b } else { px > b }) {
                best = Some(px);
            }
        }
        best
    }

    /// Price is inside or within `within` ticks of the level.
    pub fn at_level(&self, l: &Level, within: f32) -> bool {
        self.bid >= l.lo - within * self.tick && self.bid <= l.hi + within * self.tick
    }

    /// A recent bar wicked below a swing low (or above a swing high) and closed back: a sweep.
    pub fn swept(&self, side: Side, lookback: usize) -> Option<&Level> {
        let kind = if side.is_buy() { LevelKind::SwingLow } else { LevelKind::SwingHigh };
        let recent = &self.bars[self.bars.len().saturating_sub(lookback)..];
        self.levels_of(kind).find(|l| {
            recent.iter().any(|b| if side.is_buy() { b.low < l.lo && b.close > l.hi } else { b.high > l.hi && b.close < l.lo })
        })
    }

    /// When the floor's day opened: `ENTRY_FROM` on the view's UTC day.
    pub fn session_open_ms(&self) -> i64 {
        day_start_ms(self.wall_ms) + ENTRY_FROM * 60_000
    }

    /// The bars of the floor's day so far (bars are oldest first).
    pub fn session_bars(&self) -> &[Candle] {
        let open = self.session_open_ms();
        &self.bars[self.bars.len() - self.bars.iter().rev().take_while(|b| b.time_ms >= open).count()..]
    }

    /// Session VWAP: close weighted by tick count over the session's bars, the forming bar
    /// included. None before the first session bar with volume.
    pub fn vwap(&self) -> Option<f32> {
        vwap_of(self.session_bars())
    }

    /// Running VWAP after each completed session bar, paired with that bar's close.
    pub fn vwap_path(&self) -> Vec<(f32, f32)> {
        let (mut pv, mut vol) = (0f64, 0f64);
        completed_bars(self.session_bars(), self.wall_ms)
            .iter()
            .filter_map(|b| {
                pv += b.close as f64 * b.volume as f64;
                vol += b.volume as f64;
                (vol > 0.0).then_some((b.close, (pv / vol) as f32))
            })
            .collect()
    }

    /// (high, low) of the floor's day so far.
    pub fn session_extremes(&self) -> Option<(f32, f32)> {
        extremes_of(self.session_bars())
    }

    /// (high, low) of the `OPENING_RANGE_MINS` after `open_minute` (UTC minutes of day), once
    /// the range has finished forming and enough of its minutes printed a bar.
    pub fn opening_range(&self, open_minute: i64) -> Option<(f32, f32)> {
        let from = day_start_ms(self.wall_ms) + open_minute * 60_000;
        let until = from + OPENING_RANGE_MINS * 60_000;
        if self.wall_ms < until {
            return None;
        }
        let range: Vec<Candle> = self.bars.iter().filter(|b| b.time_ms >= from && b.time_ms < until).copied().collect();
        if range.len() < OPENING_RANGE_MIN_BARS {
            return None;
        }
        extremes_of(&range)
    }
}

pub fn day_start_ms(ms: i64) -> i64 {
    ms.div_euclid(86_400_000) * 86_400_000
}

/// The bars that have closed: the one for the current minute is still forming.
pub fn completed_bars(bars: &[Candle], wall_ms: i64) -> &[Candle] {
    let current = wall_ms.div_euclid(60_000) * 60_000;
    match bars.last() {
        Some(b) if b.time_ms >= current => &bars[..bars.len() - 1],
        _ => bars,
    }
}

/// Close weighted by tick count. Summed in f64 so Bitcoin-sized prices keep their ticks.
pub fn vwap_of(bars: &[Candle]) -> Option<f32> {
    let (mut pv, mut vol) = (0f64, 0f64);
    for b in bars {
        pv += b.close as f64 * b.volume as f64;
        vol += b.volume as f64;
    }
    (vol > 0.0).then(|| (pv / vol) as f32)
}

fn extremes_of(bars: &[Candle]) -> Option<(f32, f32)> {
    bars.iter().fold(None, |acc, b| match acc {
        None => Some((b.high, b.low)),
        Some((h, l)) => Some((h.max(b.high), l.min(b.low))),
    })
}

/// Resting-interest zones that a mean-reversion target must not have to trade through.
const BLOCKS_AND_GAPS: [LevelKind; 4] = [LevelKind::BullOrderBlock, LevelKind::BearOrderBlock, LevelKind::BullFvg, LevelKind::BearFvg];

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub side: Side,
    pub stop_ticks: f32,
    pub target: Option<f32>,
    pub trail: Option<Trail>,
    pub time_stop_secs: f64,
    pub cooldown_secs: f64,
    pub why: String,
}

/// The archetype's rule. Pure: a view in, an entry or a reason out.
pub fn decide(arch: Archetype, v: &View, rng: &mut XorShift) -> Result<Entry, String> {
    if let Some(style) = arch.crew() {
        return decide_crew(style, v);
    }
    let fresh_read = || v.read.ok_or_else(|| "no fresh Jev read".to_string());
    match arch {
        Archetype::MomentumScalper => {
            let r = fresh_read()?;
            let side = match r.direction {
                Direction::Up => Side::Buy,
                Direction::Down => Side::Sell,
                Direction::Flat => return Err("Jev: flat".into()),
            };
            if r.conviction < 1.2 {
                return Err(format!("conviction {:.1} < 1.2", r.conviction));
            }
            if r.continuation < 0.6 {
                return Err(format!("continuation {:.0}% < 60%", r.continuation * 100.0));
            }
            if r.exhaustion >= 0.5 {
                return Err(format!("exhaustion {:.0}%", r.exhaustion * 100.0));
            }
            let u = v.unit();
            if v.spread_ticks > 0.35 * u {
                return Err(format!("spread {:.0} ticks is {:.0}% of the unit: too wide to scalp", v.spread_ticks, v.spread_ticks / u * 100.0));
            }
            let target = if side.is_buy() { v.ask + u * v.tick } else { v.bid - u * v.tick };
            Ok(Entry {
                side,
                stop_ticks: 0.75 * u,
                target: Some(target),
                trail: None,
                time_stop_secs: 180.0,
                cooldown_secs: 60.0,
                why: format!("Jev {} {:.0}%, conviction {:.1}, continuation {:.0}%", r.direction.label(), r.direction_confidence * 100.0, r.conviction, r.continuation * 100.0),
            })
        }
        Archetype::TrendRider => {
            let r = fresh_read()?;
            let drift = v.net_change_ticks(20);
            let side = match r.direction {
                Direction::Up if drift >= 8.0 => Side::Buy,
                Direction::Down if drift <= -8.0 => Side::Sell,
                Direction::Flat => return Err("Jev: flat".into()),
                _ => return Err(format!("Jev {} but 20-bar drift {:+.0} ticks", r.direction.label(), drift)),
            };
            if r.conviction < 1.0 || r.exhaustion >= 0.4 {
                return Err(format!("conviction {:.1}, exhaustion {:.0}%", r.conviction, r.exhaustion * 100.0));
            }
            let swing_kinds = [LevelKind::SwingLow, LevelKind::SwingHigh];
            let behind = v.next_level(side.opposite(), if side.is_buy() { v.bid } else { v.ask }, &swing_kinds, 3.0);
            let u = v.unit();
            let stop_ticks = behind
                .map(|px| ((if side.is_buy() { v.ask - px } else { px - v.bid }) / v.tick + 0.25 * u).clamp(0.75 * u, 2.5 * u))
                .unwrap_or(1.5 * u);
            Ok(Entry {
                side,
                stop_ticks,
                target: None,
                trail: Some(Trail { arm_ticks: u, lock_ticks: 0.25 * u, trail_ticks: u }),
                time_stop_secs: 3600.0,
                cooldown_secs: 120.0,
                why: format!("Jev {} with 20-bar drift {:+.0} ticks; stop behind the swing {:.0} ticks away", r.direction.label(), drift, stop_ticks),
            })
        }
        Archetype::ExhaustionFader => {
            let r = fresh_read()?;
            if r.exhaustion < 0.65 {
                return Err(format!("exhaustion {:.0}% < 65%", r.exhaustion * 100.0));
            }
            let side = match r.direction {
                Direction::Up => Side::Sell,
                Direction::Down => Side::Buy,
                Direction::Flat => return Err("Jev: flat, nothing to fade".into()),
            };
            if r.direction_confidence < 0.5 {
                return Err("no clear side to fade".into());
            }
            let kinds = [LevelKind::BullOrderBlock, LevelKind::BearOrderBlock, LevelKind::BullFvg, LevelKind::BearFvg, LevelKind::SwingLow, LevelKind::SwingHigh];
            let from = if side.is_buy() { v.ask } else { v.bid };
            let u = v.unit();
            let target = v.next_level(side, from, &kinds, 0.5 * u).map(|px| {
                let cap = 2.5 * u * v.tick;
                if side.is_buy() { px.min(from + cap) } else { px.max(from - cap) }
            });
            Ok(Entry {
                side,
                stop_ticks: u,
                target: Some(target.unwrap_or(if side.is_buy() { from + 1.2 * u * v.tick } else { from - 1.2 * u * v.tick })),
                trail: None,
                time_stop_secs: 1200.0,
                cooldown_secs: 180.0,
                why: format!("fading Jev {}: exhaustion {:.0}%, reaction {}", r.direction.label(), r.exhaustion * 100.0, r.reaction),
            })
        }
        Archetype::IctLevelTrader => {
            if v.bars.len() < 10 {
                return Err("fewer than 10 bars".into());
            }
            for side in [Side::Buy, Side::Sell] {
                let Some(swept) = v.swept(side, 15) else { continue };
                let (block_kind, gap_kind) = if side.is_buy() {
                    (LevelKind::BullOrderBlock, LevelKind::BullFvg)
                } else {
                    (LevelKind::BearOrderBlock, LevelKind::BearFvg)
                };
                let zone = v.levels.iter().find(|l| (l.kind == block_kind || l.kind == gap_kind) && !l.touched && v.at_level(l, 2.0));
                let Some(zone) = zone else { continue };
                let from = if side.is_buy() { v.ask } else { v.bid };
                let stop = if side.is_buy() { zone.lo - 2.0 * v.tick } else { zone.hi + 2.0 * v.tick };
                let stop_ticks = ((from - stop).abs() / v.tick).clamp(0.5 * v.unit(), 4.0 * v.unit());
                let target_kinds = [LevelKind::SwingHigh, LevelKind::SwingLow, LevelKind::SessionHigh, LevelKind::SessionLow];
                let target = v
                    .next_level(side, from, &target_kinds, stop_ticks * 1.5)
                    .unwrap_or(if side.is_buy() { from + 2.0 * stop_ticks * v.tick } else { from - 2.0 * stop_ticks * v.tick });
                return Ok(Entry {
                    side,
                    stop_ticks,
                    target: Some(target),
                    trail: None,
                    time_stop_secs: 5400.0,
                    cooldown_secs: 600.0,
                    why: format!("{} swept at {}, price back in a fresh {} {}..{}", swept.kind.tag(), swept.mid(), zone.kind.tag(), zone.lo, zone.hi),
                });
            }
            Err("no sweep-and-return setup".into())
        }
        Archetype::GapFiller => {
            for (side, kind) in [(Side::Buy, LevelKind::BullFvg), (Side::Sell, LevelKind::BearFvg)] {
                let Some(gap) = v.levels_of(kind).find(|l| !l.touched && v.bid >= l.lo && v.bid <= l.hi) else { continue };
                // First return only: the previous tick was still outside the gap.
                let was_outside = if side.is_buy() { v.prev_bid > gap.hi } else { v.prev_bid < gap.lo };
                if !was_outside {
                    continue;
                }
                let from = if side.is_buy() { v.ask } else { v.bid };
                let height = (gap.hi - gap.lo).max(2.0 * v.tick);
                let stop = if side.is_buy() { gap.lo - 2.0 * v.tick } else { gap.hi + 2.0 * v.tick };
                let stop_ticks = ((from - stop).abs() / v.tick).clamp(0.4 * v.unit(), 3.0 * v.unit());
                let target = if side.is_buy() { gap.hi + height } else { gap.lo - height };
                return Ok(Entry {
                    side,
                    stop_ticks,
                    target: Some(target),
                    trail: None,
                    time_stop_secs: 2700.0,
                    cooldown_secs: 300.0,
                    why: format!("first return into {} {}..{}", gap.kind.tag(), gap.lo, gap.hi),
                });
            }
            Err("no fresh gap under price".into())
        }
        Archetype::OpeningRangeBreakout => {
            // The most recent open whose range has finished forming.
            let minute = minute_of_day_utc(v.wall_ms);
            let open = [LONDON_OPEN, NEW_YORK_OPEN]
                .into_iter()
                .filter(|o| minute >= o + OPENING_RANGE_MINS)
                .max()
                .ok_or_else(|| "no opening range yet".to_string())?;
            let (hi, lo) = v.opening_range(open).ok_or_else(|| format!("{:02}:{:02} range still forming", open / 60, open % 60))?;
            let done = completed_bars(v.bars, v.wall_ms);
            let [.., prev, last] = done else { return Err("fewer than two completed bars".into()) };
            // The first close outside the box only: the bar before it closed inside.
            let side = if last.close > hi && prev.close <= hi {
                Side::Buy
            } else if last.close < lo && prev.close >= lo {
                Side::Sell
            } else {
                return Err(format!("no first close outside the {}..{} opening range", lo, hi));
            };
            let mid = (hi + lo) * 0.5;
            let from = if side.is_buy() { v.ask } else { v.bid };
            let stop_ticks = (from - mid).abs() / v.tick;
            let reach = 1.5 * stop_ticks * v.tick;
            Ok(Entry {
                side,
                stop_ticks,
                target: Some(if side.is_buy() { from + reach } else { from - reach }),
                trail: None,
                time_stop_secs: 90.0 * 60.0,
                // Once per session per pit: the cooldown runs to the end of this session.
                cooldown_secs: arch.rules().secs_to_session_end(minute).max(60.0),
                why: format!("1m close {} outside the {:02}:{:02} opening range {}..{}; stop at its middle {}", last.close, open / 60, open % 60, lo, hi, mid),
            })
        }
        Archetype::SessionExtremeFader => {
            let done = completed_bars(v.session_bars(), v.wall_ms);
            if done.len() < 6 {
                return Err(format!("{} completed session bars, need 6", done.len()));
            }
            let (before, last3) = done.split_at(done.len() - 3);
            let u = v.unit();
            let mut why_not = String::from("no new session extreme in the last three bars");
            for side in [Side::Sell, Side::Buy] {
                // A new session extreme printed inside the last three bars.
                let (extreme, prior) = if side.is_buy() {
                    (last3.iter().map(|b| b.low).fold(f32::MAX, f32::min), before.iter().map(|b| b.low).fold(f32::MAX, f32::min))
                } else {
                    (last3.iter().map(|b| b.high).fold(f32::MIN, f32::max), before.iter().map(|b| b.high).fold(f32::MIN, f32::max))
                };
                let is_new = if side.is_buy() { extreme < prior } else { extreme > prior };
                if !is_new {
                    continue;
                }
                let tag = if side.is_buy() { "session low" } else { "session high" };
                // Tick rate dropping: the newest bar is the quietest of the three and at most
                // three quarters of the busiest.
                let vols = [last3[0].volume, last3[1].volume, last3[2].volume];
                if !(vols[2] < vols[0] && vols[2] < vols[1] && vols[2] <= 0.75 * vols[0].max(vols[1])) {
                    why_not = format!("new {tag} {extreme} but tick counts {:.0}/{:.0}/{:.0} are not fading", vols[0], vols[1], vols[2]);
                    continue;
                }
                // The bar that printed the extreme was rejected: its wick on that side is at
                // least half its range.
                let xb = last3.iter().find(|b| if side.is_buy() { b.low <= extreme } else { b.high >= extreme }).expect("extreme came from these bars");
                let range = xb.high - xb.low;
                let wick = if side.is_buy() { xb.open.min(xb.close) - xb.low } else { xb.high - xb.open.max(xb.close) };
                if range <= 0.0 || wick < 0.5 * range {
                    why_not = format!("new {tag} {extreme} but no rejection wick");
                    continue;
                }
                // And price has backed off it.
                let from = if side.is_buy() { v.ask } else { v.bid };
                if if side.is_buy() { from <= extreme } else { from >= extreme } {
                    why_not = format!("new {tag} {extreme} still printing");
                    continue;
                }
                // Nothing of interest within a unit on the way to the target.
                let ut = u * v.tick;
                let blocked = v.levels.iter().any(|l| {
                    BLOCKS_AND_GAPS.contains(&l.kind) && if side.is_buy() { l.lo < from + ut && l.hi > from } else { l.hi > from - ut && l.lo < from }
                });
                if blocked {
                    why_not = format!("new {tag} {extreme} rejected, but a block or gap sits within a unit");
                    continue;
                }
                let stop = if side.is_buy() { extreme - ut } else { extreme + ut };
                let stop_ticks = (stop - from).abs() / v.tick;
                let two_units = if side.is_buy() { from + 2.0 * ut } else { from - 2.0 * ut };
                let target = match v.vwap() {
                    Some(w) if side.is_buy() && w > from => w.min(two_units),
                    Some(w) if !side.is_buy() && w < from => w.max(two_units),
                    _ => two_units,
                };
                // The book would push a nearer target out past the stop, which is no longer a
                // fade to VWAP: only take it when price is still close to the extreme.
                let reach = (target - from).abs() / v.tick;
                if reach < stop_ticks * MIN_REWARD_TO_RISK {
                    why_not = format!("new {tag} {extreme} rejected, but price is too far from it: {reach:.0} ticks to the target against a {stop_ticks:.0}-tick stop");
                    continue;
                }
                return Ok(Entry {
                    side,
                    stop_ticks,
                    target: Some(target),
                    trail: None,
                    time_stop_secs: 40.0 * 60.0,
                    cooldown_secs: 300.0,
                    why: format!("fading a new {tag} {extreme}: tick counts {:.0}/{:.0}/{:.0}, rejection wick; stop a unit beyond, target {}", vols[0], vols[1], vols[2], if target == two_units { "two units".to_string() } else { format!("VWAP {target}") }),
                });
            }
            Err(why_not)
        }
        Archetype::VwapPullback => {
            let path = v.vwap_path();
            if path.len() < 25 {
                return Err(format!("{} completed session bars, need 25", path.len()));
            }
            let w = v.vwap().ok_or_else(|| "no VWAP".to_string())?;
            let u = v.unit();
            let ut = u * v.tick;
            let band = 0.25 * ut;
            let mut why_not = String::from("no trend a unit clear of VWAP");
            for side in [Side::Buy, Side::Sell] {
                let away = |(close, vw): &(f32, f32)| if side.is_buy() { close - vw >= ut } else { vw - close >= ut };
                // Up to five bars of pullback, after a run of twenty or more a unit clear.
                let pull = path.iter().rev().take_while(|p| !away(p)).count();
                if pull > 5 {
                    continue;
                }
                let run = path[..path.len() - pull].iter().rev().take_while(|p| away(p)).count();
                if run < 20 {
                    why_not = format!("only {run} bars {} VWAP", if side.is_buy() { "above" } else { "below" });
                    continue;
                }
                // First touch: the last completed bar still closed outside the band, and the
                // price is inside it now.
                let last_close = path[path.len() - 1].0;
                let outside_before = if side.is_buy() { last_close > w + band } else { last_close < w - band };
                let inside_now = (v.bid - w).abs() <= band;
                if !outside_before || !inside_now {
                    why_not = format!("{run}-bar trend {} VWAP {w:.5}, waiting for the first touch", if side.is_buy() { "above" } else { "below" });
                    continue;
                }
                let from = if side.is_buy() { v.ask } else { v.bid };
                let stop = if side.is_buy() { w - ut } else { w + ut };
                let stop_ticks = (from - stop).abs() / v.tick;
                let Some((hi, lo)) = v.session_extremes() else { continue };
                let target = if side.is_buy() { hi } else { lo };
                if if side.is_buy() { target <= from } else { target >= from } {
                    why_not = "session extreme is behind the entry".into();
                    continue;
                }
                return Ok(Entry {
                    side,
                    stop_ticks,
                    target: Some(target),
                    trail: None,
                    time_stop_secs: 3600.0,
                    cooldown_secs: 600.0,
                    why: format!("{run} bars {} VWAP {w:.5}, first touch back; stop a unit beyond VWAP, target the session {}", if side.is_buy() { "above" } else { "below" }, if side.is_buy() { "high" } else { "low" }),
                });
            }
            Err(why_not)
        }
        Archetype::Control => {
            let side = if rng.chance(0.5) { Side::Buy } else { Side::Sell };
            let u = v.unit();
            Ok(Entry {
                side,
                stop_ticks: u,
                target: Some(if side.is_buy() { v.ask + u * v.tick } else { v.bid - u * v.tick }),
                trail: None,
                time_stop_secs: 600.0,
                cooldown_secs: 180.0 + rng.unit() as f64 * 300.0,
                why: "coin flip".into(),
            })
        }
        _ => Err("not a rule".into()),
    }
}

/// The cost floors, applied to an entry at the price it would fill. A stop under
/// `min_stop_spreads` spreads is widened to it (and the trade's R measured on the widened
/// stop); an entry whose own target (the stop, for a trailing trade) is under
/// `min_target_spreads` spreads away is refused. The spread counts as at least one tick.
pub fn apply_cost_rules(e: &mut Entry, rules: &Rules, spread_ticks: f32, entry_px: f32, tick: f32) -> Result<(), String> {
    // Distances come out of f32 price arithmetic: a thousandth of a tick is noise, not a veto.
    const EPS: f32 = 1e-3;
    let spread = spread_ticks.max(1.0);
    let reach_ticks = e.target.map(|t| (t - entry_px).abs() / tick).unwrap_or(e.stop_ticks);
    let min_reach = rules.min_target_spreads * spread;
    if reach_ticks + EPS < min_reach {
        return Err(format!("target {:.1} ticks away is under {} spreads ({:.1} ticks)", reach_ticks, rules.min_target_spreads, min_reach));
    }
    let floor = rules.min_stop_spreads * spread;
    if e.stop_ticks + EPS < floor {
        e.why = format!("{} | stop widened from {:.1} to {:.1} ticks ({} spreads)", e.why, e.stop_ticks, floor, rules.min_stop_spreads);
        e.stop_ticks = floor;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Traders and the book
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Trader {
    pub archetype: Archetype,
    pub position: Option<Position>,
    pub trades: Vec<Trade>,
    pub pnl_ticks: f32,
    /// Sum of R over closed trades. The leaderboard ranks on this.
    pub pnl_r: f32,
    pub peak_r: f32,
    pub max_drawdown_r: f32,
    pub wins: u32,
    pub losses: u32,
    pub whys: VecDeque<String>,
    cooldown_until: Vec<f64>,
    last_pass: Vec<String>,
}

impl Trader {
    fn new(archetype: Archetype) -> Self {
        let n = Instrument::ALL.len();
        Self {
            archetype,
            position: None,
            trades: Vec::new(),
            pnl_ticks: 0.0,
            pnl_r: 0.0,
            peak_r: 0.0,
            max_drawdown_r: 0.0,
            wins: 0,
            losses: 0,
            whys: VecDeque::new(),
            cooldown_until: vec![0.0; n],
            last_pass: vec![String::new(); n],
        }
    }

    /// Why this trader is not in each pit right now: the reason its idea last gave for
    /// passing, per pit, pits with nothing recorded left out. "outside session hours" when
    /// the clock, not the market, is the reason.
    pub fn passes(&self) -> Vec<(Instrument, &str)> {
        Instrument::ALL
            .iter()
            .filter_map(|&i| {
                let s = self.last_pass[i as usize].as_str();
                (!s.is_empty()).then_some((i, s))
            })
            .collect()
    }

    pub fn name(&self) -> &'static str {
        self.archetype.name()
    }

    pub fn win_rate(&self) -> f32 {
        let n = self.wins + self.losses;
        if n == 0 { 0.0 } else { self.wins as f32 / n as f32 }
    }

    pub fn note(&mut self, why: String) {
        self.whys.push_front(why);
        while self.whys.len() > WHY_KEEP {
            self.whys.pop_back();
        }
    }

    fn settle(&mut self, pnl_ticks: f32, pnl_r: f32) {
        self.pnl_ticks += pnl_ticks;
        self.pnl_r += pnl_r;
        if pnl_ticks > 0.0 { self.wins += 1 } else { self.losses += 1 }
        self.peak_r = self.peak_r.max(self.pnl_r);
        self.max_drawdown_r = self.max_drawdown_r.max(self.peak_r - self.pnl_r);
    }
}

/// Why a position was closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitReason {
    Stop,
    Target,
    TimeStop,
    SessionClose,
    /// A holder's strategy turned flat or the other way.
    Strategy,
    /// Closed by hand from the trade ticket.
    Manual,
}

impl ExitReason {
    fn label(self) -> &'static str {
        match self {
            ExitReason::Stop => "stop",
            ExitReason::Target => "target",
            ExitReason::TimeStop => "time stop",
            ExitReason::SessionClose => "session close",
            ExitReason::Strategy => "strategy exit",
            ExitReason::Manual => "closed by hand",
        }
    }
}

/// Exit check for one position against the current touch. Pure.
pub fn exit_check(p: &mut Position, bid: f32, ask: f32, tick: f32, now: f64, minute_of_day: i64) -> Option<(ExitReason, f32)> {
    let mark = if p.side.is_buy() { bid } else { ask };
    p.best = if p.side.is_buy() { p.best.max(mark) } else { p.best.min(mark) };
    if let Some(t) = p.trail {
        let onside = p.open_ticks(bid, ask, tick).max(if p.side.is_buy() { (p.best - p.entry) / tick } else { (p.entry - p.best) / tick });
        if onside >= t.arm_ticks {
            let locked = if p.side.is_buy() { p.entry + t.lock_ticks * tick } else { p.entry - t.lock_ticks * tick };
            let trailed = if p.side.is_buy() { p.best - t.trail_ticks * tick } else { p.best + t.trail_ticks * tick };
            p.stop = if p.side.is_buy() { p.stop.max(locked).max(trailed) } else { p.stop.min(locked).min(trailed) };
        }
    }
    if minute_of_day >= FLAT_AT && !p.overnight {
        return Some((ExitReason::SessionClose, mark));
    }
    let stopped = if p.side.is_buy() { bid <= p.stop } else { ask >= p.stop };
    if stopped {
        return Some((ExitReason::Stop, mark));
    }
    if let Some(t) = p.target {
        let hit = if p.side.is_buy() { bid >= t } else { ask <= t };
        if hit {
            return Some((ExitReason::Target, mark));
        }
    }
    if now - p.opened_at >= p.time_stop_secs {
        return Some((ExitReason::TimeStop, mark));
    }
    None
}

pub fn minute_of_day_utc(wall_ms: i64) -> i64 {
    (wall_ms.div_euclid(1000).rem_euclid(86_400)) / 60
}

pub fn entries_allowed(minute_of_day: i64) -> bool {
    (ENTRY_FROM..ENTRY_UNTIL).contains(&minute_of_day)
}

#[derive(Resource)]
pub struct PaperBook {
    pub traders: Vec<Trader>,
    pub enabled: bool,
    pub show: bool,
    pub focus: usize,
    pub revision: u64,
    pub session_start_ms: i64,
    rng: XorShift,
    last_updates: Vec<u64>,
    prev_bid: Vec<f32>,
    next_decision_at: Vec<f64>,
    /// Logging to the relay can be off (tests, native runs without a relay).
    pub log_to_relay: bool,
    /// A click on a leaderboard row: the avatars' detail panel opens for this trader.
    pub panel_request: Option<Archetype>,
    /// A genome picked on the desk's board (by name): the detail block follows it.
    pub board_focus: Option<String>,
    /// A row of the Traders panel was clicked: the 3D camera flies to this trader's avatar.
    pub fly_to: Option<Archetype>,
}

impl Default for PaperBook {
    fn default() -> Self {
        let n = Instrument::ALL.len();
        Self {
            traders: Archetype::ALL.iter().map(|a| Trader::new(*a)).collect(),
            enabled: true,
            show: true,
            focus: 0,
            revision: 0,
            session_start_ms: wall_clock_ms(),
            rng: XorShift::new(0x5eed_7a9e_b00c_u64 ^ wall_clock_ms() as u64),
            last_updates: vec![0; n],
            prev_bid: vec![0.0; n],
            next_decision_at: vec![0.0; n],
            log_to_relay: true,
            panel_request: None,
            board_focus: None,
            fly_to: None,
        }
    }
}

impl PaperBook {
    /// Deterministic coin flips for the control (the replay wants reproducible runs).
    pub fn reseed(&mut self, seed: u64) {
        self.rng = XorShift::new(seed);
    }

    pub fn toggle_show(&mut self, controller: &mut MarketScenarioController) {
        self.show = !self.show;
        controller.announcement_banner = if self.show { "PAPER TRADERS SHOWN  (T hides)".into() } else { "PAPER TRADERS HIDDEN".into() };
        controller.banner_timer = 1.5;
    }

    /// Leaderboard order: best first, by realised plus open P&L is unknown here, so realised.
    pub fn ranking(&self) -> Vec<usize> {
        let mut idx: Vec<usize> = (0..self.traders.len()).collect();
        idx.sort_by(|a, b| self.traders[*b].pnl_r.partial_cmp(&self.traders[*a].pnl_r).unwrap());
        idx
    }

    fn log(&self, name: &str, value: &impl Serialize) {
        if !self.log_to_relay {
            return;
        }
        let Ok(body) = serde_json::to_vec(value) else { return };
        let mut req = ehttp::Request::post(format!("{}/log/{name}", relay_url()), body);
        req.headers = ehttp::Headers::new(&[("Accept", "application/json"), ("Content-Type", "application/json")]);
        ehttp::fetch(req, |_| {});
    }

    /// Close a trader's position at `exit` for `reason`: settle it, keep and log the trade.
    fn close_position(&mut self, t: usize, p: Position, v: &View, reason: ExitReason, exit: f32, log: bool) {
        let pnl_ticks = p.open_ticks(v.bid, v.ask, v.tick);
        let pnl_r = pnl_ticks / p.stop_ticks.max(1.0);
        let trade = Trade {
            trader: self.traders[t].name(),
            archetype: self.traders[t].archetype,
            symbol: v.inst.symbol(),
            side: side_str(p.side),
            entry: p.entry,
            exit,
            opened_ms: p.opened_ms,
            closed_ms: v.wall_ms,
            pnl_ticks,
            pnl_gbp: pnl_ticks * STAKE_PER_TICK,
            pnl_r,
            stop_ticks: p.stop_ticks,
            spread_ticks_in: p.spread_ticks_in,
            why_in: p.why.clone(),
            why_out: reason.label().into(),
            observation_hash: p.observation_hash.clone(),
            read_source: p.read_source,
            genome: match self.traders[t].archetype {
                Archetype::Genome(g) => crate::genome::seat_id(g),
                _ => None,
            },
        };
        let tr = &mut self.traders[t];
        tr.settle(pnl_ticks, pnl_r);
        tr.note(format!("{} {} {} closed on {}: {:+.2}R ({:+.0} ticks)", v.inst.symbol(), side_str(p.side), fmt_px(v.inst, p.entry), reason.label(), pnl_r, pnl_ticks));
        tr.position = None;
        tr.trades.push(trade.clone());
        self.revision += 1;
        if log {
            self.log("paper_trades", &trade);
            let d = Decision { trader: trade.trader, archetype: trade.archetype, symbol: trade.symbol, at_ms: v.wall_ms, action: "exit", side: Some(trade.side), price: exit, bid: v.bid, ask: v.ask, why: trade.why_out.clone(), observation_hash: None };
            self.log("paper_decisions", &d);
        }
    }

    /// Close a holder's position at the touch because its strategy says so.
    pub fn close_now(&mut self, t: usize, v: &View, log: bool) -> bool {
        self.close_at_touch(t, v, ExitReason::Strategy, log)
    }

    /// Close a position on this pit at the touch, for `reason`.
    pub fn close_at_touch(&mut self, t: usize, v: &View, reason: ExitReason, log: bool) -> bool {
        let Some(p) = self.traders[t].position.clone().filter(|p| p.inst == v.inst) else { return false };
        let exit = if p.side.is_buy() { v.bid } else { v.ask };
        self.close_position(t, p, v, reason, exit, log);
        true
    }

    /// A holder enters at the touch: one R is `unit_ticks`, the safety stop `safety_ticks` away,
    /// no target, no time stop, held overnight.
    pub fn enter_holder(&mut self, t: usize, v: &View, side: Side, unit_ticks: f32, safety_ticks: f32, why: String) -> Result<(), String> {
        if self.traders[t].position.is_some() {
            return Err("already in a position".into());
        }
        if unit_ticks < v.spread_ticks.max(1.0) * MIN_TARGET_SPREADS {
            return Err(format!("one R of {unit_ticks:.1} ticks is under {MIN_TARGET_SPREADS} spreads"));
        }
        let entry = if side.is_buy() { v.ask } else { v.bid };
        let stop = if side.is_buy() { entry - safety_ticks * v.tick } else { entry + safety_ticks * v.tick };
        let i = v.inst as usize;
        let p = Position {
            inst: v.inst,
            side,
            entry,
            stop,
            target: None,
            trail: None,
            opened_ms: v.wall_ms,
            opened_at: v.now,
            time_stop_secs: f64::INFINITY,
            best: entry,
            stop_ticks: unit_ticks,
            spread_ticks_in: v.spread_ticks,
            why: why.clone(),
            observation_hash: String::new(),
            read_source: None,
            overnight: true,
        };
        let tr = &mut self.traders[t];
        tr.cooldown_until[i] = v.now;
        tr.note(format!("{} {} at {}: {}", v.inst.symbol(), side_str(side), fmt_px(v.inst, entry), why));
        tr.position = Some(p);
        self.revision += 1;
        Ok(())
    }

    /// Run every trader over one pit's current tick. `view.read` must already be filtered
    /// for freshness and source.
    pub fn step_pit(&mut self, v: &View, log: bool) {
        let minute = minute_of_day_utc(v.wall_ms);
        let i = v.inst as usize;
        // Exits first, for everyone holding this pit.
        for t in 0..self.traders.len() {
            let Some(mut p) = self.traders[t].position.clone().filter(|p| p.inst == v.inst) else { continue };
            match exit_check(&mut p, v.bid, v.ask, v.tick, v.now, minute) {
                Some((reason, exit)) => self.close_position(t, p, v, reason, exit, log),
                None => self.traders[t].position = Some(p),
            }
        }

        // Entries, throttled to one look per pit per second.
        if !self.enabled || v.now < self.next_decision_at[i] {
            return;
        }
        self.next_decision_at[i] = v.now + 1.0;
        if !entries_allowed(minute) {
            return;
        }
        if v.spread_ticks > MAX_SPREAD_UNITS * v.unit() || v.spread_ticks < 0.0 {
            return;
        }
        for t in 0..self.traders.len() {
            if self.traders[t].position.is_some() || v.now < self.traders[t].cooldown_until[i] {
                continue;
            }
            // Genomes decide on 15m bar closes, through `enter_genome`.
            if matches!(self.traders[t].archetype, Archetype::Genome(_)) {
                continue;
            }
            let arch = self.traders[t].archetype;
            let rules = arch.rules();
            if !rules.in_session(minute) {
                self.traders[t].last_pass[i] = "outside session hours".into();
                continue;
            }
            let decided = decide(arch, v, &mut self.rng).and_then(|mut e| {
                let entry = if e.side.is_buy() { v.ask } else { v.bid };
                apply_cost_rules(&mut e, &rules, v.spread_ticks, entry, v.tick).map(|_| e)
            });
            match decided {
                Ok(e) => {
                    let entry = if e.side.is_buy() { v.ask } else { v.bid };
                    let stop = if e.side.is_buy() { entry - e.stop_ticks * v.tick } else { entry + e.stop_ticks * v.tick };
                    // A target nearer than the stop is a bad bet by construction: push it out.
                    let min_reach = e.stop_ticks * MIN_REWARD_TO_RISK * v.tick;
                    let target = e.target.map(|t| if e.side.is_buy() { t.max(entry + min_reach) } else { t.min(entry - min_reach) });
                    let p = Position {
                        inst: v.inst,
                        side: e.side,
                        entry,
                        stop,
                        target,
                        trail: e.trail,
                        opened_ms: v.wall_ms,
                        opened_at: v.now,
                        time_stop_secs: e.time_stop_secs,
                        best: entry,
                        stop_ticks: e.stop_ticks,
                        spread_ticks_in: v.spread_ticks,
                        why: e.why.clone(),
                        observation_hash: v.read.map(|r| r.observation_hash.clone()).unwrap_or_default(),
                        read_source: v.read.map(|r| r.source),
                        overnight: false,
                    };
                    let tr = &mut self.traders[t];
                    tr.cooldown_until[i] = v.now + e.cooldown_secs;
                    tr.note(format!("{} {} at {}: {}", v.inst.symbol(), side_str(e.side), fmt_px(v.inst, entry), e.why));
                    tr.position = Some(p);
                    self.revision += 1;
                    if log {
                        let d = Decision { trader: arch.name(), archetype: arch, symbol: v.inst.symbol(), at_ms: v.wall_ms, action: "enter", side: Some(side_str(e.side)), price: entry, bid: v.bid, ask: v.ask, why: e.why, observation_hash: v.read.map(|r| r.observation_hash.clone()) };
                        self.log("paper_decisions", &d);
                    }
                }
                Err(why) => {
                    if self.traders[t].last_pass[i] != why {
                        self.traders[t].last_pass[i] = why;
                    }
                }
            }
        }
    }

    /// Seat a trader (a genome from the evolving pit, or one of `ALL` the desk was told to
    /// keep). Returns its index.
    pub fn add_trader(&mut self, archetype: Archetype) -> usize {
        self.traders.push(Trader::new(archetype));
        self.revision += 1;
        self.traders.len() - 1
    }

    /// Keep only the traders `keep` says yes to. Call between ticks: indices move.
    pub fn retain_traders(&mut self, mut keep: impl FnMut(&Trader) -> bool) {
        self.traders.retain(|t| keep(t));
        self.focus = self.focus.min(self.traders.len().saturating_sub(1));
        self.revision += 1;
    }

    /// Open a genome's entry at this tick, under the same cost rules as everyone else. The
    /// genome's own rule (`crate::genome::evaluate`) has already said yes at the bar close.
    pub fn enter_genome(&mut self, t: usize, v: &View, mut e: Entry, log: bool) -> Result<(), String> {
        let i = v.inst as usize;
        let arch = self.traders[t].archetype;
        if self.traders[t].position.is_some() {
            return Err("already in a position".into());
        }
        let entry = if e.side.is_buy() { v.ask } else { v.bid };
        apply_cost_rules(&mut e, &arch.rules(), v.spread_ticks, entry, v.tick)?;
        let stop = if e.side.is_buy() { entry - e.stop_ticks * v.tick } else { entry + e.stop_ticks * v.tick };
        let min_reach = e.stop_ticks * MIN_REWARD_TO_RISK * v.tick;
        let target = e.target.map(|t| if e.side.is_buy() { t.max(entry + min_reach) } else { t.min(entry - min_reach) });
        let p = Position {
            inst: v.inst,
            side: e.side,
            entry,
            stop,
            target,
            trail: e.trail,
            opened_ms: v.wall_ms,
            opened_at: v.now,
            time_stop_secs: e.time_stop_secs,
            best: entry,
            stop_ticks: e.stop_ticks,
            spread_ticks_in: v.spread_ticks,
            why: e.why.clone(),
            observation_hash: String::new(),
            read_source: None,
            overnight: false,
        };
        let tr = &mut self.traders[t];
        tr.cooldown_until[i] = v.now + e.cooldown_secs;
        tr.note(format!("{} {} at {}: {}", v.inst.symbol(), side_str(e.side), fmt_px(v.inst, entry), e.why));
        tr.position = Some(p);
        self.revision += 1;
        if log {
            let d = Decision { trader: arch.name(), archetype: arch, symbol: v.inst.symbol(), at_ms: v.wall_ms, action: "enter", side: Some(side_str(e.side)), price: entry, bid: v.bid, ask: v.ask, why: e.why, observation_hash: None };
            self.log("paper_decisions", &d);
        }
        Ok(())
    }

    /// Open P&L in R for a trader, at the current touch of its pit.
    pub fn open_r(&self, t: usize, feed: &LiveFeed) -> Option<f32> {
        let p = self.traders[t].position.as_ref()?;
        let q = feed.quote(p.inst)?;
        Some(p.open_ticks(q.price as f32, q.ask as f32, tick_size(p.inst)) / p.stop_ticks.max(1.0))
    }
}

fn fmt_px(inst: Instrument, px: f32) -> String {
    format_price_with_commas(px as f64, instrument_decimals(inst))
}

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

/// System: every new tick on every live pit runs the book. After the desk and the levels.
pub fn update_paper_book(
    time: Res<Time>,
    feed: Res<LiveFeed>,
    store: Res<CandleStore>,
    levels: Res<LevelStore>,
    desk: Res<JevDesk>,
    mut book: ResMut<PaperBook>,
) {
    if feed.mode != FeedMode::Live {
        return;
    }
    let now = time.elapsed_secs_f64();
    let wall_ms = wall_clock_ms();
    for inst in Instrument::ALL {
        let i = inst as usize;
        let PitFeedStatus::Live { rate, .. } = feed.pit_status(inst, now) else { continue };
        let Some(q) = feed.quote(inst) else { continue };
        if q.updates == book.last_updates[i] {
            continue;
        }
        let prev_bid = if book.last_updates[i] == 0 { q.price as f32 } else { book.prev_bid[i] };
        book.last_updates[i] = q.updates;
        let tick = tick_size(inst);
        let series = store.series(inst, Timeframe::M1, FeedMode::Live);
        let bars: Vec<Candle> = series.bars.iter().rev().take(VIEW_BARS).rev().copied().collect();
        let read = desk
            .read(inst)
            .filter(|r| r.source == ReadSource::Jev && now - r.at <= READ_FRESH_SECS);
        let view = View {
            inst,
            bid: q.price as f32,
            ask: q.ask as f32,
            tick,
            spread_ticks: (q.spread / tick as f64) as f32,
            now,
            wall_ms,
            bars: &bars,
            flow: q.flow(now),
            rate,
            levels: levels.for_trading(inst),
            read,
            prev_bid,
        };
        book.step_pit(&view, true);
        book.prev_bid[i] = q.price as f32;
    }
}

#[derive(Component)]
pub struct PaperPanel;

#[derive(Component)]
pub struct PaperRow(pub usize);

#[derive(Component)]
pub struct PaperDetail;

#[derive(Component)]
pub struct PaperHeader;

/// The leaderboard panel: first in the left column (`panels`), with a close button in its
/// header; `update_paper_panel` shows and hides it from `PaperBook::show`.
pub fn setup_paper_panel(mut commands: Commands) {
    commands
        .spawn((
            PaperPanel,
            crate::panels::Panel(crate::panels::PanelId::Traders),
            Node {
                width: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                padding: UiRect::axes(Val::Px(10.0), Val::Px(6.0)),
                row_gap: Val::Px(2.0),
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(Val::Px(6.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.02, 0.03, 0.06, 1.0)),
            BorderColor::all(Color::srgba(0.85, 0.70, 0.25, 0.55)),
        ))
        .with_children(|panel| {
            let me = panel.target_entity();
            panel
                .spawn(Node {
                    flex_direction: FlexDirection::Row,
                    justify_content: JustifyContent::SpaceBetween,
                    align_items: AlignItems::FlexStart,
                    width: Val::Percent(100.0),
                    column_gap: Val::Px(6.0),
                    ..default()
                })
                .with_children(|row| {
                    row.spawn((
                        PaperHeader,
                        Text::new(PANEL_HEADER),
                        TextFont { font_size: 11.0, ..default() },
                        TextColor(Color::srgb(0.85, 0.80, 0.60)),
                        Node { flex_grow: 1.0, flex_shrink: 1.0, min_width: Val::Px(0.0), ..default() },
                    ));
                    crate::panels::spawn_close_button(row, me);
                });
            for i in 0..PANEL_ROWS {
                panel
                    .spawn((
                        Button,
                        PaperRow(i),
                        Node { padding: UiRect::axes(Val::Px(4.0), Val::Px(1.0)), border_radius: BorderRadius::all(Val::Px(3.0)), ..default() },
                        BackgroundColor(Color::NONE),
                    ))
                    .with_children(|row| {
                        row.spawn((PaperRow(i), Text::new(""), TextFont { font_size: 10.0, ..default() }, TextColor(Color::WHITE)));
                    });
            }
            panel.spawn((
                PaperDetail,
                Text::new(""),
                TextFont { font_size: 10.0, ..default() },
                TextColor(Color::srgb(0.75, 0.80, 0.88)),
                Node { margin: UiRect::top(Val::Px(4.0)), max_width: Val::Px(540.0), ..default() },
            ));
        });
}

/// Leaderboard rows: the original traders, or the desk's active traders (rows beyond them are
/// hidden, so the panel is as tall as its list).
const PANEL_ROWS: usize = 64;

/// Only as many rows as there are traders to list take up room in the panel.
fn show_rows(rows: &mut Query<(&PaperRow, &mut Node), (With<Button>, Without<PaperPanel>)>, n: usize) {
    for (row, mut node) in rows.iter_mut() {
        let want = if row.0 < n { Display::Flex } else { Display::None };
        if node.display != want {
            node.display = want;
        }
    }
}

/// A genome's colour on a page that does not hold the genome registry: from its name.
fn genome_color(name: &str) -> Color {
    let h = name.bytes().fold(7u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32));
    let (r, g, b) = crate::genome::seat_rgb((h % 1000) as u16);
    Color::srgb(r, g, b)
}

const PANEL_HEADER: &str = "PAPER TRADERS  |  realised R (profit / initial stop), spread paid  |  T hides";

/// System: the leaderboard, four times a second. While the headless desk is alive its board
/// is shown instead of the in-tab book (see `desk_feed`).
pub fn update_paper_panel(
    time: Res<Time>,
    feed: Res<LiveFeed>,
    book: Res<PaperBook>,
    desk: Res<crate::desk_feed::DeskFeed>,
    mut next: Local<f32>,
    mut root: Query<&mut Node, With<PaperPanel>>,
    mut header: Query<&mut Text, (With<PaperHeader>, Without<PaperRow>, Without<PaperDetail>)>,
    mut rows: Query<(&PaperRow, &mut Text, &mut TextColor), (Without<PaperDetail>, Without<PaperHeader>)>,
    mut detail: Query<&mut Text, (With<PaperDetail>, Without<PaperRow>)>,
    mut row_nodes: Query<(&PaperRow, &mut Node), (With<Button>, Without<PaperPanel>)>,
    mut stale_since: Local<Option<f64>>,
    mut last_reload_at: Local<f64>,
) {
    for mut node in root.iter_mut() {
        let want = if book.show { Display::Flex } else { Display::None };
        if node.display != want {
            node.display = want;
        }
    }
    if !book.show || time.elapsed_secs() < *next {
        return;
    }
    *next = time.elapsed_secs() + 0.25;
    if let Some(board) = desk.live_board(time.elapsed_secs_f64()) {
        // A board naming traders this build does not know means a newer build is being
        // served than the one this tab loaded (2026-10-22: a tab from before the Jev crew
        // showed nine rows and "only the control trading" for an hour). Say so, and after
        // twenty seconds reload, at most once every three minutes in case the new bundle is
        // still compiling.
        let now = time.elapsed_secs_f64();
        let unknown = board.traders.iter().filter(|t| t.archetype().is_none() && !t.is_genome()).count();
        let header_line = if unknown > 0 {
            let since = *stale_since.get_or_insert(now);
            if now - since > 20.0 && now - *last_reload_at > 180.0 {
                *last_reload_at = now;
                warn!("paper panel: the desk has {unknown} traders this build does not know; reloading the page");
                crate::ui::reload_page();
            }
            format!("THIS PAGE IS OUT OF DATE: the desk runs {unknown} traders this build does not know. Reloading shortly, or press Cmd-R.")
        } else {
            *stale_since = None;
            board.header_line()
        };
        for mut text in header.iter_mut() {
            set_ui_text(&mut text, header_line.clone());
        }
        let ranking = board.ranking();
        show_rows(&mut row_nodes, ranking.len());
        let focus = book.traders[book.focus].archetype;
        for (row, mut text, mut color) in rows.iter_mut() {
            let Some(&t) = ranking.get(row.0) else {
                set_ui_text(&mut text, String::new());
                continue;
            };
            let tr = &board.traders[t];
            // Mark the open position at this tab's own feed: the heartbeat's number is up to
            // a few seconds old, the tape is live.
            let live_r = tr.open.as_ref().and_then(|o| {
                if o.stop == 0.0 {
                    return None;
                }
                let inst = crate::paper_avatars::instrument_for_symbol(&o.symbol)?;
                let q = feed.quote(inst)?;
                let tick = tick_size(inst);
                let stop_ticks = ((o.entry - o.stop).abs() / tick).max(1.0);
                let open_ticks = if o.side == "long" { (q.price as f32 - o.entry) / tick } else { (o.entry - q.ask as f32) / tick };
                Some(open_ticks / stop_ticks)
            });
            set_ui_text(&mut text, board.row_line(row.0, tr, live_r));
            let picked = if tr.is_genome() { book.board_focus.as_deref() == Some(tr.trader.as_str()) } else { book.board_focus.is_none() && tr.archetype() == Some(focus) };
            let c = if picked { Color::WHITE } else if tr.is_genome() { genome_color(&tr.trader) } else { tr.archetype().map(|a| a.color()).unwrap_or(Color::WHITE) };
            if color.0 != c {
                *color = TextColor(c);
            }
        }
        for mut text in detail.iter_mut() {
            set_ui_text(&mut text, board.detail_lines_for(focus, book.board_focus.as_deref()).join("\n"));
        }
        return;
    }
    for mut text in header.iter_mut() {
        set_ui_text(&mut text, PANEL_HEADER.to_string());
    }
    let ranking = book.ranking();
    show_rows(&mut row_nodes, ranking.len());
    for (row, mut text, mut color) in rows.iter_mut() {
        let Some(&t) = ranking.get(row.0) else { continue };
        let tr = &book.traders[t];
        let open = match (&tr.position, book.open_r(t, &feed)) {
            (Some(p), Some(r)) => format!("{} {} {:+.2}R", p.inst.symbol(), side_str(p.side), r),
            _ => "flat".to_string(),
        };
        let line = format!(
            "{:>2}. {:<6} {:<12} {:>+6.2}R {:>3}t {:>3.0}% dd{:>4.1} | {}",
            row.0 + 1,
            tr.name(),
            tr.archetype.label(),
            tr.pnl_r,
            tr.trades.len(),
            tr.win_rate() * 100.0,
            tr.max_drawdown_r,
            open
        );
        set_ui_text(&mut text, line);
        let c = if book.focus == t { Color::WHITE } else { tr.archetype.color() };
        if color.0 != c {
            *color = TextColor(c);
        }
    }
    let tr = &book.traders[book.focus];
    let mut lines = vec![format!("{} ({}): {}", tr.name(), tr.archetype.label(), tr.archetype.describe())];
    if let Some(p) = &tr.position {
        lines.push(format!(
            "OPEN {} {} from {}  stop {}  target {}  |  {}",
            p.inst.symbol(),
            side_str(p.side),
            fmt_px(p.inst, p.entry),
            fmt_px(p.inst, p.stop),
            p.target.map(|t| fmt_px(p.inst, t)).unwrap_or_else(|| "trail".into()),
            p.why
        ));
    }
    for w in tr.whys.iter().take(4) {
        lines.push(format!("- {w}"));
    }
    if tr.whys.is_empty() {
        let waiting: Vec<String> = Instrument::ALL
            .iter()
            .filter_map(|i| {
                let s = &tr.last_pass[*i as usize];
                (!s.is_empty()).then(|| format!("{}: {}", i.symbol(), s))
            })
            .take(3)
            .collect();
        lines.push(if waiting.is_empty() { "- waiting for the first setup".into() } else { format!("- passing: {}", waiting.join("; ")) });
    }
    for mut text in detail.iter_mut() {
        set_ui_text(&mut text, lines.join("\n"));
    }
}

pub fn handle_paper_rows(
    time: Res<Time>,
    interaction: Query<(&Interaction, &PaperRow), (Changed<Interaction>, With<Button>)>,
    desk: Res<crate::desk_feed::DeskFeed>,
    mut book: ResMut<PaperBook>,
) {
    for (i, row) in interaction.iter() {
        if *i != Interaction::Pressed {
            continue;
        }
        // Rows follow whichever board is showing; traders sit in `Archetype::ALL` order.
        if let Some(board) = desk.live_board(time.elapsed_secs_f64()) {
            let Some(picked) = board.ranking().get(row.0).map(|&t| &board.traders[t]) else { continue };
            if picked.is_genome() {
                // Genomes have no in-tab twin: the detail block follows them by name, and the
                // avatar's panel opens as it does for anyone else.
                book.board_focus = Some(picked.trader.clone());
                book.panel_request = Some(crate::paper_avatars::genome_arch(board, &picked.trader));
                book.fly_to = book.panel_request;
                continue;
            }
            book.board_focus = None;
            let focus = picked.archetype().and_then(|a| book.traders.iter().position(|tr| tr.archetype == a));
            if let Some(t) = focus {
                book.focus = t;
                book.panel_request = Some(book.traders[t].archetype);
                book.fly_to = book.panel_request;
            }
            continue;
        }
        let ranking = book.ranking();
        if let Some(&t) = ranking.get(row.0) {
            book.focus = t;
            book.panel_request = Some(book.traders[t].archetype);
            book.fly_to = book.panel_request;
        }
    }
}

pub fn handle_paper_keys(
    keyboard: Res<ButtonInput<KeyCode>>,
    mut book: ResMut<PaperBook>,
    mut controller: ResMut<MarketScenarioController>,
) {
    if keyboard.just_pressed(KeyCode::KeyT) {
        book.toggle_show(&mut controller);
    }
}

pub fn handle_paper_buttons(
    interaction: Query<(&Interaction, &crate::ui::TouchButtonAction), (Changed<Interaction>, With<Button>)>,
    mut book: ResMut<PaperBook>,
    mut controller: ResMut<MarketScenarioController>,
) {
    for (i, action) in interaction.iter() {
        if *i == Interaction::Pressed && *action == crate::ui::TouchButtonAction::TradersToggle {
            book.toggle_show(&mut controller);
        }
    }
}

pub fn update_paper_button_style(
    book: Res<PaperBook>,
    mut buttons: Query<(&crate::ui::TouchButtonAction, &mut BackgroundColor, &mut BorderColor), With<Button>>,
) {
    if !book.is_changed() {
        return;
    }
    for (action, mut bg, mut border) in buttons.iter_mut() {
        if *action != crate::ui::TouchButtonAction::TradersToggle {
            continue;
        }
        if book.show {
            *bg = BackgroundColor(Color::srgba(0.08, 0.32, 0.20, 0.95));
            *border = BorderColor::all(Color::srgb(0.35, 0.85, 0.50));
        } else {
            *bg = BackgroundColor(Color::srgba(0.06, 0.09, 0.14, 0.85));
            *border = BorderColor::all(Color::srgba(0.18, 0.28, 0.40, 0.55));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TICK: f32 = 0.01;
    /// 2026-10-21T00:00:00Z.
    const MIDNIGHT: i64 = 1_792_540_800_000;

    fn bars(n: usize, start: f32, step: f32) -> Vec<Candle> {
        (0..n)
            .map(|i| {
                let o = start + step * i as f32;
                // Range 0.08 for a 0.01 step: the risk unit is 8 ticks.
                Candle { time_ms: i as i64 * 60_000, open: o, high: o.max(o + step) + 0.035, low: o.min(o + step) - 0.035, close: o + step, volume: 10.0, delta: step.signum() * 4.0, sweep: 1.0 }
            })
            .collect()
    }

    fn read(dir: Direction, conviction: f32, continuation: f32, exhaustion: f32) -> MarketInterpretation {
        MarketInterpretation {
            direction: dir,
            direction_confidence: 0.9,
            p_up: if dir == Direction::Up { 0.9 } else { 0.05 },
            p_down: if dir == Direction::Down { 0.9 } else { 0.05 },
            p_flat: 0.05,
            conviction,
            continuation,
            exhaustion,
            reaction: "no_level".into(),
            reaction_confidence: 1.0,
            model: "test".into(),
            source: ReadSource::Jev,
            latency_ms: 1,
            observation_hash: "h".into(),
            at: 0.0,
            input_tokens: 0,
            output_tokens: 0,
            answers: Default::default(),
            trigger: None,
        }
    }

    fn view<'a>(bars: &'a [Candle], levels: &'a [Level], read: Option<&'a MarketInterpretation>, bid: f32, now: f64) -> View<'a> {
        View {
            inst: Instrument::Silver,
            bid,
            ask: bid + 2.0 * TICK,
            tick: TICK,
            spread_ticks: 2.0,
            now,
            wall_ms: MIDNIGHT + 9 * 3_600_000, // 09:00 UTC
            bars,
            flow: TickFlow { direction: 0.5, up_share: 0.7, ticks: 20, burst_ticks: 1.0 },
            rate: 2.0,
            levels,
            read,
            prev_bid: bid,
        }
    }

    fn book() -> PaperBook {
        let mut b = PaperBook::default();
        b.log_to_relay = false;
        b.rng = XorShift::new(7);
        b
    }

    fn trader(b: &PaperBook, a: Archetype) -> &Trader {
        b.traders.iter().find(|t| t.archetype == a).unwrap()
    }

    #[test]
    fn the_risk_unit_is_the_mean_bar_range_floored() {
        let b = bars(30, 48.0, 0.01);
        let v = view(&b, &[], None, 48.30, 10.0);
        assert!((v.unit() - 8.0).abs() < 0.3, "unit {}", v.unit());
        let flat = bars(30, 48.0, 0.0);
        let v = view(&flat, &[], None, 48.0, 10.0);
        assert!((v.unit() - 7.0).abs() < 0.3);
        let v = view(&[], &[], None, 48.0, 10.0);
        assert_eq!(v.unit(), MIN_UNIT_TICKS);
    }

    #[test]
    fn a_long_pays_the_spread_on_the_way_in_and_out() {
        let b = bars(30, 48.0, 0.01);
        let r = read(Direction::Up, 1.6, 0.8, 0.1);
        let mut book = book();
        book.step_pit(&view(&b, &[], Some(&r), 48.30, 10.0), false);
        let v = trader(&book, Archetype::MomentumScalper);
        let p = v.position.as_ref().expect("Vinnie is long");
        assert_eq!(p.side, Side::Buy);
        assert!((p.entry - 48.32).abs() < 1e-5, "filled at the ask");
        assert!((p.open_ticks(48.30, 48.32, TICK) + 2.0).abs() < 1e-3, "two ticks down the moment it fills");
        // Vinnie's idea is a 6-tick stop and a 48.40 target; on a 2-tick spread the floor widens
        // the stop to 40 ticks and the target is pushed out to a full R.
        assert!((p.stop - 47.92).abs() < 1e-5 && (p.target.unwrap() - 48.72).abs() < 1e-5, "{p:?}");
        assert!(p.why.contains("widened from 6.0 to 40.0"), "{}", p.why);

        // Short of the target nothing happens.
        book.step_pit(&view(&b, &[], Some(&r), 48.40, 11.0), false);
        assert!(trader(&book, Archetype::MomentumScalper).position.is_some());
        // Target: bid reaches 48.72.
        book.step_pit(&view(&b, &[], Some(&r), 48.72, 12.0), false);
        let v = trader(&book, Archetype::MomentumScalper);
        assert!(v.position.is_none());
        assert_eq!(v.trades.len(), 1);
        assert!((v.trades[0].pnl_ticks - 40.0).abs() < 1e-3, "{:?}", v.trades[0]);
        assert!((v.trades[0].pnl_r - 1.0).abs() < 1e-3, "40 ticks on a 40-tick stop: {:?}", v.trades[0]);
        assert_eq!(v.trades[0].why_out, "target");
        assert_eq!(v.wins, 1);
    }

    #[test]
    fn a_stop_closes_at_the_bid_and_the_control_needs_no_read() {
        let b = bars(30, 48.0, -0.01);
        let r = read(Direction::Down, 1.6, 0.8, 0.1);
        let mut book = book();
        book.step_pit(&view(&b, &[], Some(&r), 48.00, 10.0), false);
        let v = trader(&book, Archetype::MomentumScalper);
        let p = v.position.as_ref().expect("short");
        assert_eq!(p.side, Side::Sell);
        assert!((p.entry - 48.00).abs() < 1e-5, "sold at the bid");
        assert!(trader(&book, Archetype::Control).position.is_some(), "Colin flipped a coin");
        assert!((p.stop - 48.40).abs() < 1e-5, "20 spreads of 2 ticks: {p:?}");
        // The ask runs through the short's stop (48.40): exit at the ask.
        book.step_pit(&view(&b, &[], Some(&r), 48.39, 11.0), false);
        let v = trader(&book, Archetype::MomentumScalper);
        assert_eq!(v.trades.len(), 1);
        assert_eq!(v.trades[0].why_out, "stop");
        assert!((v.trades[0].pnl_ticks + 41.0).abs() < 1e-3, "{:?}", v.trades[0]);
    }

    #[test]
    fn nobody_enters_outside_the_session_and_everyone_is_flat_at_the_close() {
        let b = bars(30, 48.0, 0.01);
        let r = read(Direction::Up, 1.6, 0.8, 0.1);
        let mut book = book();
        let mut v = view(&b, &[], Some(&r), 48.30, 10.0);
        v.wall_ms = MIDNIGHT + 5 * 3_600_000; // 05:00 UTC
        book.step_pit(&v, false);
        assert!(book.traders.iter().all(|t| t.position.is_none()));

        let mut v = view(&b, &[], Some(&r), 48.30, 20.0);
        v.wall_ms = MIDNIGHT + 10 * 3_600_000;
        book.step_pit(&v, false);
        assert!(book.traders.iter().any(|t| t.position.is_some()));

        let mut v = view(&b, &[], Some(&r), 48.31, 30.0);
        v.wall_ms = MIDNIGHT + 20 * 3_600_000 + 56 * 60_000; // 20:56 UTC
        book.step_pit(&v, false);
        assert!(book.traders.iter().all(|t| t.position.is_none()));
        assert!(book.traders.iter().flat_map(|t| t.trades.iter()).all(|t| t.why_out == "session close"));
    }

    #[test]
    fn the_trend_rider_trails_and_the_fader_needs_exhaustion() {
        let b = bars(30, 48.0, 0.01); // +30 ticks drift
        let r = read(Direction::Up, 1.3, 0.7, 0.2);
        let mut book = book();
        book.step_pit(&view(&b, &[], Some(&r), 48.30, 10.0), false);
        let g = trader(&book, Archetype::TrendRider);
        let p = g.position.as_ref().expect("Grace rides");
        assert!(p.trail.is_some() && p.target.is_none());
        let stop0 = p.stop;
        assert!(trader(&book, Archetype::ExhaustionFader).position.is_none(), "Harold waits for exhaustion");
        // +12 ticks: stop locks above entry.
        book.step_pit(&view(&b, &[], Some(&r), 48.44, 11.0), false);
        let p = trader(&book, Archetype::TrendRider).position.as_ref().unwrap();
        assert!(p.stop > stop0 && p.stop > p.entry, "stop {} entry {}", p.stop, p.entry);

        let tired = read(Direction::Up, 1.0, 0.4, 0.8);
        let mut book2 = book_with_seed(9);
        book2.step_pit(&view(&b, &[], Some(&tired), 48.30, 10.0), false);
        let h = trader(&book2, Archetype::ExhaustionFader);
        assert_eq!(h.position.as_ref().map(|p| p.side), Some(Side::Sell));
    }

    fn book_with_seed(seed: u64) -> PaperBook {
        let mut b = book();
        b.rng = XorShift::new(seed);
        b
    }

    #[test]
    fn ict_needs_a_sweep_and_a_fresh_block_and_gaps_need_a_first_return() {
        // Swing low at 47.90 swept by a wick to 47.85 that closed at 47.95; a fresh bull OB
        // 47.88..47.92 under price; price now 47.92 (at the block).
        let mut b = bars(20, 48.0, 0.0);
        b[15] = Candle { time_ms: 15 * 60_000, open: 47.93, high: 47.96, low: 47.85, close: 47.95, volume: 10.0, delta: -2.0, sweep: 3.0 };
        let swing = Level { kind: LevelKind::SwingLow, lo: 47.895, hi: 47.905, strength: 0.6, time_ms: 5 * 60_000, touched: false, pct: 0.0 };
        let block = Level { kind: LevelKind::BullOrderBlock, lo: 47.88, hi: 47.92, strength: 0.9, time_ms: 4 * 60_000, touched: false, pct: 0.0 };
        let high = Level { kind: LevelKind::SwingHigh, lo: 48.295, hi: 48.305, strength: 0.6, time_ms: 2 * 60_000, touched: false, pct: 0.0 };
        let levels = vec![swing, block, high];
        let mut book = book();
        let v = view(&b, &levels, None, 47.92, 10.0);
        let mut rng = XorShift::new(1);
        let e = decide(Archetype::IctLevelTrader, &v, &mut rng).expect("Priya buys the return");
        assert_eq!(e.side, Side::Buy);
        assert!((e.stop_ticks - 8.0).abs() < 1e-3, "stop two ticks under the block, eight under the ask: {}", e.stop_ticks);
        assert!((e.target.unwrap() - 48.295).abs() < 1e-4, "target the swing high: {:?}", e.target);
        book.step_pit(&v, false);
        let p = trader(&book, Archetype::IctLevelTrader).position.as_ref().expect("Priya buys the return");
        assert!((p.stop_ticks - 40.0).abs() < 1e-3 && (p.target.unwrap() - 48.34).abs() < 1e-4, "the floor widens the stop to 20 spreads and pushes the target to 1 R: {p:?}");
        assert!(trader(&book, Archetype::MomentumScalper).position.is_none(), "no Jev read, no scalp");

        // Without the sweep there is no trade.
        let plain = bars(20, 48.0, 0.0);
        let mut book = book_with_seed(3);
        book.step_pit(&view(&plain, &levels, None, 47.92, 10.0), false);
        assert!(trader(&book, Archetype::IctLevelTrader).position.is_none());

        // Gap filler: first tick inside an untouched bull gap from above, in his first hour.
        let gap = Level { kind: LevelKind::BullFvg, lo: 47.90, hi: 47.99, strength: 0.8, time_ms: 6 * 60_000, touched: false, pct: 0.0 };
        let mut book = book_with_seed(4);
        let mut v = view(&plain, std::slice::from_ref(&gap), None, 47.98, 10.0);
        v.wall_ms = at(7, 30);
        v.prev_bid = 48.02;
        book.step_pit(&v, false);
        let p = trader(&book, Archetype::GapFiller).position.as_ref().expect("Tomasz buys the gap");
        assert_eq!(p.side, Side::Buy);
        // Gap top plus its height is 48.08, eight ticks out (four spreads: allowed) but inside
        // the twelve-tick stop: pushed out to 1R.
        let r = (p.entry - p.stop).abs();
        assert!((p.target.unwrap() - (p.entry + r)).abs() < 1e-4, "target {:?} entry {} stop {}", p.target, p.entry, p.stop);
        let mut book = book_with_seed(5);
        let mut v = view(&plain, std::slice::from_ref(&gap), None, 47.98, 10.0);
        v.wall_ms = at(7, 30);
        v.prev_bid = 47.97; // already inside: not the first return
        book.step_pit(&v, false);
        assert!(trader(&book, Archetype::GapFiller).position.is_none());
    }

    /// Wall clock at a UTC time of day on the test's day.
    fn at(h: i64, m: i64) -> i64 {
        MIDNIGHT + (h * 60 + m) * 60_000
    }

    /// A 1m bar opening at `at(h, m)`.
    fn bar(h: i64, m: i64, open: f32, high: f32, low: f32, close: f32, volume: f32) -> Candle {
        Candle { time_ms: at(h, m), open, high, low, close, volume, delta: 0.0, sweep: 0.0 }
    }

    #[test]
    fn rules_respect_the_global_floors() {
        for a in Archetype::ALL {
            let r = a.rules();
            assert!(r.min_stop_spreads >= MIN_STOP_SPREADS, "{} risks under {MIN_STOP_SPREADS} spreads", a.name());
            assert!(r.min_target_spreads >= MIN_TARGET_SPREADS, "{} aims under {MIN_TARGET_SPREADS} spreads", a.name());
            assert!(!r.sessions.is_empty(), "{} has no session", a.name());
            for s in r.sessions {
                assert!(s.from < s.until && s.from >= ENTRY_FROM && s.until <= ENTRY_UNTIL, "{} session {s:?} is outside the entry window", a.name());
            }
        }
    }

    #[test]
    fn the_jev_crew_are_distinct_traders_on_jev_all_day() {
        assert_eq!(JEV_CREW.len(), 15);
        let mut names: Vec<&str> = JEV_CREW.iter().map(|s| s.name).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), JEV_CREW.len(), "crew names repeat");
        let mut colors: Vec<String> = JEV_CREW.iter().map(|s| format!("{:?}", s.color)).collect();
        colors.sort();
        colors.dedup();
        assert_eq!(colors.len(), JEV_CREW.len(), "crew colours repeat");
        for s in JEV_CREW {
            assert!(Archetype::ALL.contains(&s.arch), "{} is not in ALL", s.name);
            assert_eq!(s.arch.crew().map(|c| c.name), Some(s.name));
            assert_eq!(s.arch.name(), s.name);
            assert_eq!(s.arch.label(), s.label);
            assert_eq!(s.arch.describe(), s.creed);
            assert!(s.arch.uses_jev());
            assert_eq!(s.arch.rules().sessions, &[ALL_DAY], "{} has hours other than the whole window", s.name);
            assert!(s.stop_units > 0.0 && (s.target_units > 0.0 || s.trail_units > 0.0 || s.time_stop_secs > 0.0), "{} has no way out", s.name);
        }
        assert_eq!(Archetype::ALL.len(), 9 + JEV_CREW.len());
        for s in JEV_CREW.iter() {
            assert_eq!(Archetype::ALL.iter().filter(|a| **a == s.arch).count(), 1, "{} is in ALL once", s.name);
        }
        assert!(Archetype::Control.crew().is_none() && Archetype::MomentumScalper.crew().is_none());
    }

    #[test]
    fn crew_rules_read_back_in_words() {
        let tess = Archetype::JevEverything.strategy_text();
        assert!(tess.starts_with("Enters with Jev's lean, direction confidence >= 10%."), "{tess}");
        assert!(tess.contains("stop 1.00 units, target 1.00 units, time stop 10 min, 20 s cooldown"), "{tess}");
        let sol = Archetype::JevFlatFader.strategy_text();
        assert!(sol.starts_with("Enters only on a FLAT read"), "{sol}");
        let vera = Archetype::JevOracle.strategy_text();
        assert!(vera.starts_with("Enters on Jev's answer to its own question, asked when price has just touched a level,"), "{vera}");
        assert!(vera.contains("reject: away from the level") && vera.contains("break: through the level"), "{vera}");
        let yusuf = Archetype::JevCarry.strategy_text();
        assert!(yusuf.contains("trail 1.00 units") && !yusuf.contains("target") && yusuf.contains("yes at 65% or more"), "{yusuf}");
        assert!(Archetype::Control.strategy_text().contains("coin"));
    }

    #[test]
    fn the_crew_trades_the_reads_the_originals_sit_out() {
        let b = bars(30, 48.0, 0.01);
        let mut rng = XorShift::new(3);
        // A typical read: up at 30%, conviction 0.4, continuation 0.42, exhaustion 0.48.
        let mut r = read(Direction::Up, 0.4, 0.42, 0.48);
        r.direction_confidence = 0.30;
        let v = view(&b, &[], Some(&r), 48.30, 20.0);
        assert!(decide(Archetype::MomentumScalper, &v, &mut rng).is_err(), "Vinnie wants conviction 1.2");
        let tess = decide(Archetype::JevEverything, &v, &mut rng).unwrap();
        assert_eq!(tess.side, Side::Buy);
        assert!(tess.why.starts_with("with Jev UP 30%"), "{}", tess.why);
        assert!((tess.stop_ticks - 8.0).abs() < 1e-2 && (tess.target.unwrap() - (v.ask + 8.0 * TICK)).abs() < 1e-3, "a unit each way: {tess:?}");
        let gus = decide(Archetype::JevContrarian, &v, &mut rng).unwrap();
        assert_eq!(gus.side, Side::Sell, "Gus fades the same read");
        assert!(gus.why.starts_with("against Jev UP"));
        assert!(decide(Archetype::JevDoubtFader, &v, &mut rng).unwrap_err().contains("too sure to fade"));
        assert!(decide(Archetype::JevFlatFader, &v, &mut rng).unwrap_err().contains("waits for flat"));
        assert!(decide(Archetype::JevSweeper, &v, &mut rng).unwrap_err().contains("level no_level; wants sweeping"));
        let mut rs = r.clone();
        rs.reaction = "sweeping".into();
        let vs = view(&b, &[], Some(&rs), 48.30, 20.0);
        assert_eq!(decide(Archetype::JevSweeper, &vs, &mut rng).unwrap().side, Side::Sell, "Kit fades the up lean on a sweep");
        assert!(decide(Archetype::JevAbsorber, &vs, &mut rng).is_err(), "Lena wants absorbing");
        // The question traders were not asked anything on this read: they all pass.
        assert!(decide(Archetype::JevOracle, &v, &mut rng).unwrap_err().contains("waits for a touch read"));
        assert!(decide(Archetype::JevTape, &v, &mut rng).unwrap_err().contains("was not asked tape_state"));

        // Flat read after a rising five bars: Sol sells; everyone else on a lean passes.
        let mut rf = read(Direction::Flat, 0.2, 0.4, 0.5);
        rf.direction_confidence = 0.5;
        let vf = view(&b, &[], Some(&rf), 48.30, 20.0);
        let sol = decide(Archetype::JevFlatFader, &vf, &mut rng).unwrap();
        assert_eq!(sol.side, Side::Sell);
        assert!(sol.why.contains("fading it"), "{}", sol.why);
        assert!(decide(Archetype::JevEverything, &vf, &mut rng).unwrap_err().contains("flat"));
        let flat_bars = bars(30, 48.0, 0.0);
        let vq = view(&flat_bars, &[], Some(&rf), 48.0, 20.0);
        assert!(decide(Archetype::JevFlatFader, &vq, &mut rng).unwrap_err().contains("under half a unit"));
        // No read, nobody in the crew trades.
        let vn = view(&b, &[], None, 48.30, 20.0);
        for s in JEV_CREW {
            assert_eq!(decide(s.arch, &vn, &mut rng).unwrap_err(), "no fresh Jev read", "{}", s.name);
        }
    }

    #[test]
    fn the_question_traders_act_on_their_own_answers() {
        use crate::jev::{JevAnswer, ObsTrigger};
        let b = bars(30, 48.0, 0.01);
        let mut rng = XorShift::new(3);
        let choice = |c: &str| JevAnswer::Choice { choice: c.into(), confidence: 0.7, probabilities: Default::default() };
        // A touch read: price came down onto a bull block 47.90..48.00 and Jev says it rejects.
        let mut r = read(Direction::Flat, 0.4, 0.5, 0.5);
        r.trigger = Some(ObsTrigger { kind: "touch".into(), level: "OB BULL".into(), lo: 47.90, hi: 48.00, side: "from above".into(), secs_since: 0.0 });
        r.answers.insert("touch_outcome".into(), choice("reject"));
        r.answers.insert("absorbing".into(), JevAnswer::Noul { noul: 0.75 });
        r.answers.insert("stop_run".into(), JevAnswer::Noul { noul: 0.2 });
        r.answers.insert("carry_through".into(), JevAnswer::Noul { noul: 0.3 });
        r.answers.insert("tape_state".into(), choice("one_way_selling"));
        r.answers.insert("bar_shape".into(), choice("climax_up"));
        r.answers.insert("magnet".into(), choice("neither"));
        r.answers.insert("delta_divergence".into(), JevAnswer::Noul { noul: 0.9 });
        let v = view(&b, &[], Some(&r), 48.02, 20.0); // bid 48.02, ask 48.04, unit 8 ticks
        let vera = decide(Archetype::JevOracle, &v, &mut rng).unwrap();
        assert_eq!(vera.side, Side::Buy, "it held, so away from it is up");
        assert!(vera.why.starts_with("Jev touch_outcome = reject 70% at OB BULL 47.900..48.000 from above"), "{}", vera.why);
        // The stop sits a quarter unit under the block's far edge: 48.04 - 47.90 = 14 ticks + 2.
        assert!((vera.stop_ticks - 16.0).abs() < 1e-3, "{}", vera.stop_ticks);
        assert!((vera.target.unwrap() - (48.04 + 12.0 * TICK)).abs() < 1e-3, "1.5 units: {:?}", vera.target);
        assert_eq!(decide(Archetype::JevAbsorption, &v, &mut rng).unwrap().side, Side::Buy);
        assert!(decide(Archetype::JevStopRun, &v, &mut rng).unwrap_err().contains("20% < 60%"));
        assert!(decide(Archetype::JevCarry, &v, &mut rng).unwrap_err().contains("30% < 65%"));
        assert_eq!(decide(Archetype::JevTape, &v, &mut rng).unwrap().side, Side::Sell);
        let ana = decide(Archetype::JevClimax, &v, &mut rng).unwrap();
        assert_eq!(ana.side, Side::Sell);
        assert!(ana.why.starts_with("Jev bar_shape = climax_up 70%"), "{}", ana.why);
        assert!(decide(Archetype::JevLeg, &v, &mut rng).unwrap_err().contains("climax_up: no trade"), "Ben reads the same answer and passes");
        assert!(decide(Archetype::JevMagnet, &v, &mut rng).unwrap_err().contains("neither: no trade"));
        let mut faint = r.clone();
        faint.answers.insert("magnet".into(), JevAnswer::Choice { choice: "below".into(), confidence: 0.38, probabilities: Default::default() });
        let vf = view(&b, &[], Some(&faint), 48.02, 20.0);
        assert!(decide(Archetype::JevMagnet, &vf, &mut rng).unwrap_err().contains("below at 38% < 50%"), "Cy needs Jev half sure");
        assert!(decide(Archetype::JevDivergence, &v, &mut rng).unwrap_err().contains("no lean"), "a flat read gives Dee nothing to fade");
        // Through the level: Vera sells a break from above, stop past the near edge.
        r.answers.insert("touch_outcome".into(), choice("break"));
        let v2 = view(&b, &[], Some(&r), 47.95, 20.0);
        let vera = decide(Archetype::JevOracle, &v2, &mut rng).unwrap();
        assert_eq!(vera.side, Side::Sell);
        // 47.95 to the near edge 48.00 is 5 ticks plus a quarter unit (7): narrower than the style's one-unit stop, so the unit stands.
        assert!((vera.stop_ticks - 8.0).abs() < 1e-3, "{}", vera.stop_ticks);
        // Too late: Vera acts within a minute of a touch read.
        let v3 = view(&b, &[], Some(&r), 47.95, 80.0);
        assert!(decide(Archetype::JevOracle, &v3, &mut rng).unwrap_err().contains("80s old"));
        // A timed read with a lean: Dee fades it, the touch traders wait.
        let mut rt = read(Direction::Up, 0.4, 0.5, 0.5);
        rt.answers.insert("delta_divergence".into(), JevAnswer::Noul { noul: 0.9 });
        rt.answers.insert("touch_outcome".into(), choice("reject"));
        let vt = view(&b, &[], Some(&rt), 48.02, 20.0);
        assert_eq!(decide(Archetype::JevDivergence, &vt, &mut rng).unwrap().side, Side::Sell);
        assert!(decide(Archetype::JevOracle, &vt, &mut rng).unwrap_err().contains("waits for a touch"));
    }

    #[test]
    fn the_control_shares_the_widest_opportunity_set() {
        let colin = Archetype::Control.rules();
        let others: Vec<Rules> = Archetype::ALL.iter().filter(|a| **a != Archetype::Control).map(|a| a.rules()).collect();
        for minute in 0..1440 {
            let union = others.iter().any(|r| r.in_session(minute));
            assert_eq!(colin.in_session(minute), union, "minute {minute}: the control's hours must be exactly the union of the others'");
        }
        assert_eq!(Archetype::OpeningRangeBreakout.rules().secs_to_session_end(7 * 60 + 16), (89 * 60) as f64);
        assert_eq!(Archetype::OpeningRangeBreakout.rules().secs_to_session_end(12 * 60), 0.0);
    }

    #[test]
    fn the_session_filter_keeps_vinnie_out_at_lunch() {
        let b = bars(30, 48.0, 0.01);
        let r = read(Direction::Up, 1.6, 0.8, 0.1);
        let mut book = book();
        let mut v = view(&b, &[], Some(&r), 48.30, 10.0);
        v.wall_ms = at(12, 0);
        book.step_pit(&v, false);
        let vinnie = trader(&book, Archetype::MomentumScalper);
        assert!(vinnie.position.is_none(), "London is shut and New York has not opened");
        assert_eq!(vinnie.last_pass[Instrument::Silver as usize], "outside session hours");
        assert!(trader(&book, Archetype::Control).position.is_some(), "the control is on whenever anyone is");

        let mut book = book_with_seed(8);
        v.wall_ms = at(14, 0);
        book.step_pit(&v, false);
        assert!(trader(&book, Archetype::MomentumScalper).position.is_some(), "New York is open");
    }

    #[test]
    fn cost_rules_widen_tight_stops_and_veto_small_targets() {
        let rules = Archetype::GapFiller.rules();
        let entry = |stop_ticks: f32, target: Option<f32>| Entry { side: Side::Buy, stop_ticks, target, trail: None, time_stop_secs: 60.0, cooldown_secs: 60.0, why: "test".into() };
        // Two-tick stop on a two-tick spread: widened to forty, and the record says so.
        let mut e = entry(2.0, Some(48.20));
        apply_cost_rules(&mut e, &rules, 2.0, 48.00, TICK).unwrap();
        assert!((e.stop_ticks - 40.0).abs() < 1e-5, "{}", e.stop_ticks);
        assert!(e.why.contains("widened from 2.0 to 40.0"), "{}", e.why);
        // A target five ticks away on a two-tick spread is under four spreads: refused.
        let mut e = entry(10.0, Some(48.05));
        let err = apply_cost_rules(&mut e, &rules, 2.0, 48.00, TICK).unwrap_err();
        assert!(err.contains("under 4 spreads"), "{err}");
        // A trailing trade's reach is its stop.
        assert!(apply_cost_rules(&mut entry(10.0, None), &rules, 2.0, 48.00, TICK).is_ok());
        assert!(apply_cost_rules(&mut entry(6.0, None), &rules, 2.0, 48.00, TICK).is_err());
        // The spread counts as at least one tick.
        let mut e = entry(2.0, Some(48.20));
        apply_cost_rules(&mut e, &rules, 0.0, 48.00, TICK).unwrap();
        assert!((e.stop_ticks - 20.0).abs() < 1e-5);
        // Through the book: Vinnie's eight-tick target on a 2.5-tick spread is nine ticks short.
        let b = bars(30, 48.0, 0.01);
        let r = read(Direction::Up, 1.6, 0.8, 0.1);
        let mut book = book();
        let mut v = view(&b, &[], Some(&r), 48.30, 10.0);
        v.ask = 48.325;
        v.spread_ticks = 2.5;
        book.step_pit(&v, false);
        let vinnie = trader(&book, Archetype::MomentumScalper);
        assert!(vinnie.position.is_none());
        assert!(vinnie.last_pass[Instrument::Silver as usize].contains("under 4 spreads"), "{}", vinnie.last_pass[Instrument::Silver as usize]);
    }

    #[test]
    fn vwap_and_session_extremes_count_only_the_session() {
        let b = vec![
            bar(6, 30, 100.0, 120.0, 90.0, 100.0, 100.0), // before the floor's day
            bar(7, 0, 10.0, 12.0, 9.0, 10.0, 1.0),
            bar(7, 1, 10.0, 21.0, 10.0, 20.0, 3.0),
        ];
        let mut v = view(&b, &[], None, 20.0, 10.0);
        v.wall_ms = at(7, 1) + 30_000;
        assert_eq!(v.session_bars().len(), 2);
        assert!((v.vwap().unwrap() - 17.5).abs() < 1e-5, "{:?}", v.vwap());
        assert_eq!(v.session_extremes(), Some((21.0, 9.0)));
        // The 07:01 bar is still forming: the path has one completed bar.
        assert_eq!(v.vwap_path(), vec![(10.0, 10.0)]);
        v.wall_ms = at(7, 2);
        assert_eq!(v.vwap_path().len(), 2);
        assert!((v.vwap_path()[1].1 - 17.5).abs() < 1e-5);
        // No volume, no VWAP; no session bars, no extremes.
        let empty = vec![bar(7, 0, 10.0, 12.0, 9.0, 10.0, 0.0)];
        let v = view(&empty, &[], None, 10.0, 10.0);
        assert_eq!(v.vwap(), None);
        let v = view(&b[..1], &[], None, 10.0, 10.0);
        assert_eq!(v.session_extremes(), None);
        assert_eq!(v.vwap(), None);
        assert_eq!(completed_bars(&b, at(7, 1)).len(), 2);
        assert_eq!(completed_bars(&b, at(7, 1) + 1).len(), 2);
        assert_eq!(completed_bars(&b, at(7, 5)).len(), 3);
    }

    /// Fifteen bars from 07:00 boxed in 48.00..48.10, then one closing at `brk` at 07:15.
    fn opening_range_bars(brk: f32) -> Vec<Candle> {
        let mut b: Vec<Candle> = (0..15).map(|i| bar(7, i, 48.03, 48.10 - 0.002 * i as f32, 48.00 + 0.002 * i as f32, 48.05, 10.0)).collect();
        b.push(bar(7, 15, 48.06, brk.max(48.09), brk.min(48.01), brk, 12.0));
        b
    }

    #[test]
    fn the_opening_range_breakout_fires_on_the_first_close_outside() {
        let b = opening_range_bars(48.12);
        let mut book = book();
        let mut v = view(&b, &[], None, 48.12, 10.0);
        v.wall_ms = at(7, 16) + 30_000;
        assert_eq!(v.opening_range(LONDON_OPEN), Some((48.10, 48.00)));
        assert_eq!(v.opening_range(NEW_YORK_OPEN), None);
        let mut rng = XorShift::new(1);
        let e = decide(Archetype::OpeningRangeBreakout, &v, &mut rng).expect("Maggie buys the break");
        assert_eq!(e.side, Side::Buy);
        assert!((e.stop_ticks - 9.0).abs() < 1e-3, "stop at the range middle, nine ticks under the 48.14 ask: {e:?}");
        assert!((e.target.unwrap() - (48.14 + 0.135)).abs() < 1e-4, "1.5 R: {:?}", e.target);
        book.step_pit(&v, false);
        let m = trader(&book, Archetype::OpeningRangeBreakout);
        let p = m.position.as_ref().expect("Maggie buys the break");
        assert!((p.entry - 48.14).abs() < 1e-5 && (p.stop_ticks - 40.0).abs() < 1e-3 && (p.target.unwrap() - 48.54).abs() < 1e-4, "widened to 20 spreads, target at 1 R: {p:?}");
        assert!((p.time_stop_secs - 5400.0).abs() < 1e-6);
        // Once a session: the cooldown runs to 08:45.
        assert!((m.cooldown_until[Instrument::Silver as usize] - (10.0 + 89.0 * 60.0)).abs() < 1e-6);

        let mut rng = XorShift::new(1);
        // A close inside the box is nothing.
        let inside = opening_range_bars(48.08);
        let mut v = view(&inside, &[], None, 48.08, 10.0);
        v.wall_ms = at(7, 16) + 30_000;
        assert!(decide(Archetype::OpeningRangeBreakout, &v, &mut rng).unwrap_err().contains("no first close outside"));
        // The second bar outside is not the first.
        let mut second = opening_range_bars(48.12);
        second.push(bar(7, 16, 48.12, 48.16, 48.11, 48.15, 9.0));
        let mut v = view(&second, &[], None, 48.15, 10.0);
        v.wall_ms = at(7, 17) + 30_000;
        assert!(decide(Archetype::OpeningRangeBreakout, &v, &mut rng).unwrap_err().contains("no first close outside"));
        // Before 07:15 there is no range, and with too few of its minutes printed there is none either.
        let mut v = view(&b, &[], None, 48.12, 10.0);
        v.wall_ms = at(7, 10);
        assert!(decide(Archetype::OpeningRangeBreakout, &v, &mut rng).unwrap_err().contains("no opening range yet"));
        let sparse: Vec<Candle> = b.iter().filter(|c| c.time_ms < at(7, 5) || c.time_ms >= at(7, 15)).copied().collect();
        let mut v = view(&sparse, &[], None, 48.12, 10.0);
        v.wall_ms = at(7, 16) + 30_000;
        assert!(decide(Archetype::OpeningRangeBreakout, &v, &mut rng).unwrap_err().contains("still forming"));
        // A short, through the bid.
        let down = opening_range_bars(47.97);
        let mut v = view(&down, &[], None, 47.97, 10.0);
        v.wall_ms = at(7, 16) + 30_000;
        let e = decide(Archetype::OpeningRangeBreakout, &v, &mut rng).unwrap();
        assert_eq!(e.side, Side::Sell);
        assert!((e.stop_ticks - 8.0).abs() < 1e-3, "{}", e.stop_ticks);
    }

    /// Ten quiet bars at `base` from 07:00, then a spike to a new session high at 48.45 with
    /// a long upper wick, on tick counts 14, 11 and `last_vol`.
    fn extreme_bars(base: f32, spike_close: f32, last_vol: f32) -> Vec<Candle> {
        let mut b: Vec<Candle> = (0..10).map(|i| bar(7, i, base, base + 0.04, base - 0.04, base, 10.0)).collect();
        b.push(bar(7, 10, 48.15, 48.45, 48.13, spike_close, 14.0));
        b.push(bar(7, 11, 48.25, 48.33, 48.21, 48.27, 11.0));
        b.push(bar(7, 12, 48.27, 48.30, 48.19, 48.23, last_vol));
        b
    }

    #[test]
    fn the_session_extreme_fader_needs_a_fading_tick_rate_and_a_wick() {
        let b = extreme_bars(48.10, 48.25, 6.0);
        let mut book = book();
        let mut v = view(&b, &[], None, 48.38, 10.0);
        v.wall_ms = at(8, 13) + 30_000;
        let w = v.vwap().unwrap();
        let u = v.unit();
        let mut rng = XorShift::new(1);
        let e = decide(Archetype::SessionExtremeFader, &v, &mut rng).expect("Dev fades the high");
        assert_eq!(e.side, Side::Sell);
        assert!((e.stop_ticks - (7.0 + u)).abs() < 1e-3, "a unit beyond the 48.45 extreme from the 48.38 bid: {e:?}");
        assert!(w < 48.38 - 2.0 * u * TICK, "VWAP {w} is the far side of two units here");
        assert!((e.target.unwrap() - (48.38 - 2.0 * u * TICK)).abs() < 1e-4, "two units: {:?}", e.target);
        assert!(e.why.contains("rejection wick") && e.why.contains("two units"), "{}", e.why);
        book.step_pit(&v, false);
        let p = trader(&book, Archetype::SessionExtremeFader).position.as_ref().expect("Dev fades the high");
        assert!((p.entry - 48.38).abs() < 1e-5);
        assert!((p.stop_ticks - 40.0).abs() < 1e-3 && (p.target.unwrap() - 47.98).abs() < 1e-4, "widened to 20 spreads, target at 1 R: {p:?}");

        // With VWAP inside two units, VWAP is the target.
        let near = extreme_bars(48.19, 48.25, 6.0);
        let mut v = view(&near, &[], None, 48.39, 10.0);
        v.wall_ms = at(8, 13) + 30_000;
        let w = v.vwap().unwrap();
        assert!(w > 48.39 - 2.0 * v.unit() * TICK);
        let e = decide(Archetype::SessionExtremeFader, &v, &mut rng).unwrap();
        assert_eq!(e.side, Side::Sell);
        assert!((e.target.unwrap() - w).abs() < 1e-5 && e.why.contains("VWAP"), "{e:?}");
        // Too far below the high: the target would sit inside the stop.
        let mut v = view(&b, &[], None, 48.18, 10.0);
        v.wall_ms = at(8, 13) + 30_000;
        assert!(decide(Archetype::SessionExtremeFader, &v, &mut rng).unwrap_err().contains("too far from it"));
        // Tick rate not fading.
        let busy = extreme_bars(48.10, 48.25, 12.0);
        let mut v = view(&busy, &[], None, 48.38, 10.0);
        v.wall_ms = at(8, 13) + 30_000;
        assert!(decide(Archetype::SessionExtremeFader, &v, &mut rng).unwrap_err().contains("not fading"));
        // No rejection: the spike bar closed on its high.
        let strong = extreme_bars(48.10, 48.44, 6.0);
        let mut v = view(&strong, &[], None, 48.38, 10.0);
        v.wall_ms = at(8, 13) + 30_000;
        assert!(decide(Archetype::SessionExtremeFader, &v, &mut rng).unwrap_err().contains("no rejection wick"));
        // A bull order block just under price is in the way.
        let block = Level { kind: LevelKind::BullOrderBlock, lo: 48.30, hi: 48.34, strength: 0.9, time_ms: 0, touched: false, pct: 0.0 };
        let mut v = view(&b, std::slice::from_ref(&block), None, 48.38, 10.0);
        v.wall_ms = at(8, 13) + 30_000;
        assert!(decide(Archetype::SessionExtremeFader, &v, &mut rng).unwrap_err().contains("within a unit"));
        // Still printing the high: nothing to fade yet.
        let mut v = view(&b, &[], None, 48.46, 10.0);
        v.wall_ms = at(8, 13) + 30_000;
        assert!(decide(Archetype::SessionExtremeFader, &v, &mut rng).unwrap_err().contains("still printing"));
    }

    /// `flat` bars at 48.00 from 08:00, then `trend` bars stepping up 2 ticks a minute, then
    /// one pullback bar closing at `pull_close`.
    fn trend_bars(flat: usize, trend: usize, pull_close: f32) -> Vec<Candle> {
        let mut b: Vec<Candle> = (0..flat).map(|i| bar(8, i as i64, 48.00, 48.04, 47.96, 48.00, 10.0)).collect();
        for k in 0..trend {
            let close = 48.10 + 0.02 * k as f32;
            b.push(bar(8, (flat + k) as i64, close - 0.02, close + 0.03, close - 0.05, close, 10.0));
        }
        let last = b.len() as i64;
        b.push(bar(8, last, pull_close + 0.03, pull_close + 0.05, pull_close - 0.03, pull_close, 10.0));
        b
    }

    #[test]
    fn the_vwap_pullback_buys_the_first_touch_after_twenty_bars_clear() {
        let w0 = vwap_of(&trend_bars(5, 25, 48.30)[..30]).unwrap();
        let b = trend_bars(5, 25, w0 + 0.05);
        let w = vwap_of(&b).unwrap();
        let mut book = book();
        let mut v = view(&b, &[], None, w, 10.0);
        v.wall_ms = at(8, 31) + 30_000;
        let u = v.unit();
        let mut rng = XorShift::new(1);
        let e = decide(Archetype::VwapPullback, &v, &mut rng).expect("Rosa buys the touch");
        assert_eq!(e.side, Side::Buy);
        assert!((e.stop_ticks - (u + 2.0)).abs() < 1e-3, "a unit beyond VWAP {w} from the ask two ticks above it: {e:?}");
        let (hi, _) = v.session_extremes().unwrap();
        assert!((e.target.unwrap() - hi).abs() < 1e-4, "the session high: {:?} vs {hi}", e.target);
        assert!(e.why.contains("25 bars above VWAP"), "{}", e.why);
        book.step_pit(&v, false);
        let p = trader(&book, Archetype::VwapPullback).position.as_ref().expect("Rosa buys the touch");
        assert!((p.stop_ticks - 40.0).abs() < 1e-3 && (p.target.unwrap() - (p.entry + 0.40)).abs() < 1e-4, "widened to 20 spreads, the session high is inside 1 R so the target is pushed out: {p:?}");

        // The last bar already closed inside the band: not the first touch.
        let late = trend_bars(5, 25, w0 + 0.01);
        let mut v = view(&late, &[], None, vwap_of(&late).unwrap(), 10.0);
        v.wall_ms = at(8, 31) + 30_000;
        assert!(decide(Archetype::VwapPullback, &v, &mut rng).unwrap_err().contains("first touch"));
        // Price not at VWAP yet.
        let mut v = view(&b, &[], None, w + 0.05, 10.0);
        v.wall_ms = at(8, 31) + 30_000;
        assert!(decide(Archetype::VwapPullback, &v, &mut rng).unwrap_err().contains("first touch"));
        // Fifteen bars clear is not a trend.
        let short = trend_bars(10, 15, 48.20);
        let ws = vwap_of(&short).unwrap();
        let mut v = view(&short, &[], None, ws, 10.0);
        v.wall_ms = at(8, 26) + 30_000;
        assert!(decide(Archetype::VwapPullback, &v, &mut rng).unwrap_err().contains("only 15 bars above"));
    }

    #[test]
    fn ranking_puts_the_best_realised_first_and_minutes_parse() {
        let mut book = book();
        book.traders[2].settle(12.0, 1.5);
        book.traders[4].settle(-3.0, -0.4);
        let r = book.ranking();
        assert_eq!(r[0], 2);
        assert_eq!(*r.last().unwrap(), 4);
        assert_eq!(minute_of_day_utc(MIDNIGHT + 9 * 3_600_000 + 40 * 60_000), 9 * 60 + 40);
        assert!(entries_allowed(7 * 60) && !entries_allowed(20 * 60 + 30) && !entries_allowed(6 * 60 + 59));
    }
}
