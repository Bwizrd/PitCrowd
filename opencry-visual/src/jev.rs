//! Jev on the floor.
//!
//! A `DecisionModel` reads a `MarketObservation` (the selected pit's bars, tape, real spread
//! and book, and the resting-order levels near price; never the simulated crowd, whose numbers
//! are made from the same tape) and answers typed questions: direction, conviction,
//! continuation, exhaustion, and how price is behaving at the nearest level. The
//! floor turns the answers into a one-line read on the board, a widening of the locals' quotes
//! against the read, and the well glow. It never places a trade and never moves price: the
//! tape does that.
//!
//! Providers behind the trait: `JevModel` (TypeSafe System One through `jev_relay`, which holds
//! the key; the browser never sees it), `DeterministicModel` (the fallback whenever the relay
//! is down, rate-limited or slow, so the floor keeps a read), and `MockModel` for tests.
//! Every inference is recorded by the relay with the observation hash, so a session can be
//! replayed from recorded inferences later.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::candles::{CandleStore, ChartSettings, Timeframe, wall_clock_ms};
use crate::levels::{Level, LevelStore};
use crate::live_feed::{FeedMode, LiveFeed, PitFeedStatus, TickFlow};
use crate::market::Instrument;
use crate::pit::{instrument_decimals, set_ui_text};
use crate::pit_engine::{FloorRead, PitEngines, tick_size};
use crate::scenario::MarketScenarioController;
use crate::ui::Selection;

/// Seconds between reads of the focused pit. One pit, six reads a minute.
pub const JEV_CADENCE_SECS: f64 = 10.0;
/// A read older than this is dropped as late; the fallback takes over until the next one.
pub const JEV_TIMEOUT_SECS: f64 = 25.0;
/// Bars handed to the model, oldest first.
pub const OBS_BARS: usize = 20;
/// Port of `jev_relay` (see `jev_relay/README.md`).
pub const RELAY_PORT: u16 = crate::endpoints::RELAY_PORT;

// ---------------------------------------------------------------------------
// Observation
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct ObsBar {
    pub time_ms: i64,
    pub open: f32,
    pub high: f32,
    pub low: f32,
    pub close: f32,
    /// Upticks minus downticks, `None` for bars from before this session (no tick history).
    pub delta: Option<f32>,
    /// Largest single move in ticks inside the bar.
    pub sweep: f32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Default)]
pub struct ObsTape {
    pub ticks_per_sec: f32,
    pub up_share: f32,
    pub ticks: u32,
    pub burst_ticks: f32,
    pub direction: f32,
}

/// The real touch: spread in ticks and the broker's ladder sizes, best first.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ObsMarket {
    pub spread_ticks: f32,
    pub bid_sizes: Vec<f32>,
    pub ask_sizes: Vec<f32>,
}

/// A resting-order level near price, as the floor infers it (see `levels.rs`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ObsLevel {
    pub kind: String,
    pub lo: f32,
    pub hi: f32,
    /// Signed distance from the last price to the level's nearest edge, in ticks. Negative =
    /// the level is below price.
    pub distance_ticks: f32,
    pub touched: bool,
}

/// Levels closer than this are "at" the level for the reaction question.
pub const AT_LEVEL_TICKS: f32 = 3.0;
pub const OBS_LEVELS: usize = 6;

/// Why a read was asked, when it was not the timer: price has just touched a level
/// (`kind` "touch"), or it touched one `secs_since` seconds ago and this is the follow-up
/// ("follow"). Goes into the state text as a TRIGGER line, so the relay log carries it and the
/// scorer can split touch-triggered reads from timed ones.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ObsTrigger {
    pub kind: String,
    /// `LevelKind::tag`.
    pub level: String,
    pub lo: f32,
    pub hi: f32,
    /// "from above" or "from below".
    pub side: String,
    pub secs_since: f32,
}

/// Everything the model is told. Serialisable so the same observation can be logged, hashed
/// and replayed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MarketObservation {
    pub symbol: String,
    pub name: String,
    pub at_ms: i64,
    pub timeframe: String,
    pub price: f64,
    pub decimals: usize,
    pub tick_size: f32,
    pub bars: Vec<ObsBar>,
    pub tape: ObsTape,
    pub market: ObsMarket,
    pub levels: Vec<ObsLevel>,
    #[serde(default)]
    pub trigger: Option<ObsTrigger>,
}

impl MarketObservation {
    #[allow(clippy::too_many_arguments)]
    pub fn from_parts(
        inst: Instrument,
        at_ms: i64,
        timeframe: &str,
        price: f64,
        bars: Vec<ObsBar>,
        flow: &TickFlow,
        rate: f32,
        market: ObsMarket,
        levels: &[Level],
    ) -> Self {
        let tick = tick_size(inst);
        let mut obs_levels: Vec<ObsLevel> = levels
            .iter()
            .map(|l| {
                let px = price as f32;
                // Where the level sits relative to price: above is positive, below negative.
                let distance = if px < l.lo {
                    (l.lo - px) / tick
                } else if px > l.hi {
                    (l.hi - px) / tick
                } else {
                    0.0
                };
                ObsLevel {
                    kind: l.kind.tag().to_string(),
                    lo: l.lo,
                    hi: l.hi,
                    distance_ticks: distance,
                    touched: l.touched,
                }
            })
            .collect();
        obs_levels.sort_by(|a, b| {
            a.distance_ticks
                .abs()
                .partial_cmp(&b.distance_ticks.abs())
                .unwrap()
        });
        obs_levels.truncate(OBS_LEVELS);
        Self {
            symbol: inst.symbol().to_string(),
            name: inst.name().to_string(),
            at_ms,
            timeframe: timeframe.to_string(),
            price,
            decimals: instrument_decimals(inst),
            tick_size: tick,
            bars,
            tape: ObsTape {
                ticks_per_sec: rate,
                up_share: flow.up_share,
                ticks: flow.ticks,
                burst_ticks: flow.burst_ticks,
                direction: flow.direction,
            },
            market,
            levels: obs_levels,
            trigger: None,
        }
    }

    /// The level price is at, if any.
    pub fn level_at_price(&self) -> Option<&ObsLevel> {
        self.levels
            .iter()
            .find(|l| l.distance_ticks.abs() <= AT_LEVEL_TICKS)
    }

    /// Net change over the bars, in ticks.
    pub fn net_change_ticks(&self) -> f32 {
        match (self.bars.first(), self.bars.last()) {
            (Some(a), Some(b)) if self.tick_size > 0.0 => (b.close - a.open) / self.tick_size,
            _ => 0.0,
        }
    }

    /// The `state` Jev reads: compact, line-oriented, numbers at the precision the market uses.
    pub fn state_text(&self) -> String {
        let d = self.decimals;
        let mut s = String::with_capacity(2048);
        s.push_str(&format!(
            "INSTRUMENT: {} ({}), tick {}\n",
            self.name,
            self.symbol,
            fmt_f(self.tick_size as f64, d)
        ));
        s.push_str(&format!(
            "TIME: {} UTC, bars of {}\n",
            hhmm(self.at_ms),
            self.timeframe
        ));
        s.push_str(&format!("LAST PRICE: {}\n", fmt_f(self.price, d)));
        s.push_str(&format!(
            "LAST {} BARS, oldest first (open high low close | delta = upticks minus downticks | sweep = largest move in ticks):\n",
            self.bars.len()
        ));
        for b in &self.bars {
            let delta = match b.delta {
                Some(x) => format!("{:+.0}", x),
                None => "n/a".to_string(),
            };
            s.push_str(&format!(
                " {} {} {} {} {} | delta {} | sweep {:.0}\n",
                hhmm(b.time_ms),
                fmt_f(b.open as f64, d),
                fmt_f(b.high as f64, d),
                fmt_f(b.low as f64, d),
                fmt_f(b.close as f64, d),
                delta,
                b.sweep
            ));
        }
        s.push_str(&format!(
            "NET CHANGE OVER THESE BARS: {:+.0} ticks\n",
            self.net_change_ticks()
        ));
        let t = &self.tape;
        if t.ticks == 0 {
            s.push_str("TAPE (last 20 s): no ticks\n");
        } else {
            s.push_str(&format!(
                "TAPE (last 20 s): {:.1} ticks/s, upticks {:.0}% of {} ticks, largest single move {:.0} ticks\n",
                t.ticks_per_sec,
                t.up_share * 100.0,
                t.ticks,
                t.burst_ticks
            ));
        }
        let m = &self.market;
        if m.bid_sizes.is_empty() && m.ask_sizes.is_empty() {
            s.push_str(&format!("TOUCH: spread {:.0} ticks\n", m.spread_ticks));
        } else {
            let sizes = |v: &[f32]| {
                v.iter()
                    .map(|x| format!("{x}"))
                    .collect::<Vec<_>>()
                    .join("/")
            };
            s.push_str(&format!(
                "TOUCH: spread {:.0} ticks, broker ladder sizes bid {} ask {} (best first)\n",
                m.spread_ticks,
                sizes(&m.bid_sizes),
                sizes(&m.ask_sizes)
            ));
        }
        if self.levels.is_empty() {
            s.push_str("LEVELS: none marked in this window\n");
        } else {
            s.push_str("LEVELS NEAR PRICE (where resting orders probably are; distance in ticks from last price, negative = below):\n");
            for l in &self.levels {
                s.push_str(&format!(
                    " {} {}..{} at {:+.0} ticks{}\n",
                    l.kind,
                    fmt_f(l.lo as f64, d),
                    fmt_f(l.hi as f64, d),
                    l.distance_ticks,
                    if l.touched {
                        " (already traded into once)"
                    } else {
                        ""
                    }
                ));
            }
        }
        if let Some(t) = &self.trigger {
            let band = format!(
                "{} {}..{} {}",
                t.level,
                fmt_f(t.lo as f64, d),
                fmt_f(t.hi as f64, d),
                t.side
            );
            if t.kind == "follow" {
                s.push_str(&format!("TRIGGER: {:.0} s after price touched {band}; this read was asked to see how the reaction at that level is developing\n", t.secs_since));
            } else {
                s.push_str(&format!("TRIGGER: price has just touched {band}; this read was asked because of that touch, so answer level_reaction for it\n"));
            }
        }
        s
    }

    /// FNV-1a 64 of the state text; the relay logs the same hash, so the two can be joined.
    pub fn hash(&self) -> String {
        fnv64(&self.state_text())
    }
}

fn fmt_f(v: f64, decimals: usize) -> String {
    format!("{v:.decimals$}")
}

/// "HH:MM" UTC from Unix ms.
fn hhmm(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let day = secs.rem_euclid(86_400);
    format!("{:02}:{:02}", day / 3600, (day % 3600) / 60)
}

pub fn fnv64(s: &str) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

// ---------------------------------------------------------------------------
// Questions and answers (System One wire format)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum JevQuestion {
    Noul {
        instructions: String,
    },
    Choice {
        instructions: String,
        criteria: HashMap<String, String>,
    },
    Score {
        instructions: String,
        criteria: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum JevAnswer {
    Noul {
        #[serde(alias = "probability")]
        noul: f64,
    },
    Choice {
        choice: String,
        #[serde(default)]
        confidence: f64,
        #[serde(default)]
        probabilities: HashMap<String, f64>,
    },
    Score {
        score: f64,
        #[serde(default)]
        confidence: f64,
    },
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct JevUsage {
    #[serde(default)]
    pub input_tokens: u32,
    #[serde(default)]
    pub output_tokens: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct JevResponse {
    #[serde(default)]
    pub model: String,
    pub answers: HashMap<String, JevAnswer>,
    #[serde(default)]
    pub usage: JevUsage,
}

#[derive(Serialize)]
struct SystemOneBody<'a> {
    state: String,
    questions: &'a HashMap<String, JevQuestion>,
}

/// The fixed question set. Typed outputs only: the floor never parses prose.
pub fn floor_questions() -> HashMap<String, JevQuestion> {
    let mut q = HashMap::new();
    let mut dir = HashMap::new();
    dir.insert(
        "up".to_string(),
        "price works higher over the next few bars".to_string(),
    );
    dir.insert(
        "down".to_string(),
        "price works lower over the next few bars".to_string(),
    );
    dir.insert(
        "flat".to_string(),
        "price chops or pauses; no clear direction".to_string(),
    );
    q.insert(
        "direction".to_string(),
        JevQuestion::Choice {
            instructions: "Read the bars, the tape and the pit together. Where is price most likely to go over the next two or three bars of this timeframe?".into(),
            criteria: dir,
        },
    );
    q.insert(
        "conviction".to_string(),
        JevQuestion::Score {
            instructions: "How strong and one-sided is the current directional read?".into(),
            criteria: vec![
                "weak: mixed tape, two-way pit, small moves".into(),
                "moderate: a lean in the tape or the pit, not both".into(),
                "strong: tape, pit and bars agree and the move is paying".into(),
            ],
        },
    );
    q.insert(
        "continuation".to_string(),
        JevQuestion::Noul {
            instructions:
                "Is the move of the last few bars more likely to continue than to reverse?".into(),
        },
    );
    q.insert(
        "exhaustion".to_string(),
        JevQuestion::Noul {
            instructions: "Does the tape show exhaustion: the side that was in control is running out (shrinking delta, fading tick rate, sweeps that do not follow through)?".into(),
        },
    );
    let mut reaction = HashMap::new();
    reaction.insert("absorbing".to_string(), "price is at a level, the tick rate is up, and price is not getting through: something is taking everything".to_string());
    reaction.insert(
        "rejecting".to_string(),
        "price touched a level and is turning away from it quickly".to_string(),
    );
    reaction.insert(
        "sweeping".to_string(),
        "price pushed a few ticks through a level, stalled, and is coming back: a stop run"
            .to_string(),
    );
    reaction.insert(
        "breaking".to_string(),
        "price is going through a level cleanly with follow-through".to_string(),
    );
    reaction.insert(
        "no_level".to_string(),
        "price is not at any of the listed levels".to_string(),
    );
    q.insert(
        "level_reaction".to_string(),
        JevQuestion::Choice {
            instructions: "If the last price is within a few ticks of one of the listed levels, how is price behaving there? Otherwise answer no_level.".into(),
            criteria: reaction,
        },
    );
    q
}

// ---------------------------------------------------------------------------
// The crew's own questions
// ---------------------------------------------------------------------------
//
// Each question is narrow, typed and checkable within minutes. A crew member owns a question
// (several may share one) and a rule that turns its answer into a side; the scorer grades
// every question on its own against what price did, so a question earns or loses its trader.

/// Which way a trade goes relative to a touched level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LevelSide {
    /// Back the way price came.
    Away,
    /// On through the level.
    Through,
}

/// A side, without the paper book's types (this module sits below `paper`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AskSide {
    Buy,
    Sell,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AskKind {
    Noul,
    /// (choice, what it means)
    Choice(&'static [(&'static str, &'static str)]),
    Score(&'static [&'static str]),
}

/// A question put to Jev on every read, in System One's typed form.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct JevAsk {
    pub key: &'static str,
    pub kind: AskKind,
    pub instructions: &'static str,
    /// Only meaningful on a touch read (a TRIGGER line names the level); a trader on it
    /// passes on timed reads.
    pub touch_only: bool,
}

/// How a crew member turns an answer into a side.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AskRule {
    /// A choice names a direction relative to the touched level; unlisted choices pass, and
    /// so does a choice Jev holds under `min` (a choice is only the largest of several
    /// probabilities, so a 9% "break" is not a call).
    LevelChoice {
        map: &'static [(&'static str, LevelSide)],
        min: f32,
    },
    /// A choice names an absolute side; unlisted or under-confident choices pass.
    Choice {
        map: &'static [(&'static str, AskSide)],
        min: f32,
    },
    /// A yes-probability at or over `min` trades relative to the touched level.
    NoulLevel { min: f32, side: LevelSide },
    /// A yes-probability at or over `min` trades with Jev's direction lean, or against it.
    NoulLean { min: f32, with_lean: bool },
}

/// A crew member's question and rule together.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CrewAsk {
    pub question: &'static JevAsk,
    pub rule: AskRule,
}

pub static ASK_TOUCH_OUTCOME: JevAsk = JevAsk {
    key: "touch_outcome",
    touch_only: true,
    kind: AskKind::Choice(&[
        (
            "reject",
            "price never trades through the far edge of the level and is at least half a unit back the way it came within fifteen minutes",
        ),
        (
            "break",
            "price is at least half a unit beyond the far edge within fifteen minutes and stays there",
        ),
        (
            "sweep",
            "price trades through the far edge by a tick or more, then comes back to the near side",
        ),
        (
            "stall",
            "price is still sitting at the level after fifteen minutes, within half a unit either side",
        ),
    ]),
    instructions: "Price has just touched the level named in TRIGGER. From the tape, the bars and the ladder, what will price have done at that level fifteen minutes from now? A unit is the mean bar range.",
};
pub static ASK_ABSORBING: JevAsk = JevAsk {
    key: "absorbing",
    touch_only: true,
    kind: AskKind::Noul,
    instructions: "At the level named in TRIGGER, is resting size absorbing the arrivals: the tick rate is up, price is not getting through, and the broker ladder on that side is holding or refilling?",
};
pub static ASK_STOP_RUN: JevAsk = JevAsk {
    key: "stop_run",
    touch_only: true,
    kind: AskKind::Noul,
    instructions: "Is the move into the level named in TRIGGER a stop run: a burst of ticks that pushed onto or through it, with the burst already fading and no fresh size behind it?",
};
pub static ASK_CARRY_THROUGH: JevAsk = JevAsk {
    key: "carry_through",
    touch_only: true,
    kind: AskKind::Noul,
    instructions: "If price goes through the level named in TRIGGER, will it carry at least one unit (the mean bar range) beyond it within fifteen minutes?",
};
pub static ASK_TAPE_STATE: JevAsk = JevAsk {
    key: "tape_state",
    touch_only: false,
    kind: AskKind::Choice(&[
        (
            "one_way_buying",
            "the last 20 seconds are mostly upticks at a rising rate: the asks are being lifted",
        ),
        (
            "one_way_selling",
            "the last 20 seconds are mostly downticks at a rising rate: the bids are being hit",
        ),
        ("two_way", "ticks both ways, nobody in charge"),
        ("dead", "few ticks, nothing happening"),
    ]),
    instructions: "Classify the last 20 seconds of the TAPE line alone, ignoring the bars.",
};
pub static ASK_BAR_SHAPE: JevAsk = JevAsk {
    key: "bar_shape",
    touch_only: false,
    kind: AskKind::Choice(&[
        (
            "climax_up",
            "an accelerating rise into the last bars with the biggest ranges at the end: a buying climax",
        ),
        (
            "climax_down",
            "an accelerating fall into the last bars with the biggest ranges at the end: a selling climax",
        ),
        (
            "trend_up",
            "steady higher highs and higher lows with ordinary bar ranges",
        ),
        (
            "trend_down",
            "steady lower highs and lower lows with ordinary bar ranges",
        ),
        (
            "compression",
            "ranges shrinking bar by bar inside a narrowing band",
        ),
        ("chop", "none of those: overlapping bars going nowhere"),
    ]),
    instructions: "Name the shape of the last twenty bars.",
};
pub static ASK_MAGNET: JevAsk = JevAsk {
    key: "magnet",
    touch_only: false,
    kind: AskKind::Choice(&[
        (
            "above",
            "the nearest listed level above price is reached first",
        ),
        (
            "below",
            "the nearest listed level below price is reached first",
        ),
        ("neither", "price reaches neither within fifteen minutes"),
    ]),
    instructions: "Of the listed levels, which will price reach first within fifteen minutes: the nearest above or the nearest below?",
};
pub static ASK_DELTA_DIVERGENCE: JevAsk = JevAsk {
    key: "delta_divergence",
    touch_only: false,
    kind: AskKind::Noul,
    instructions: "Is price making a new high or low over the last bars while the delta and the tick rate shrink: the move going on with less and less behind it?",
};

/// Every question the crew asks, in the order the scorer reports them.
pub static CREW_ASKS: [&JevAsk; 8] = [
    &ASK_TOUCH_OUTCOME,
    &ASK_ABSORBING,
    &ASK_STOP_RUN,
    &ASK_CARRY_THROUGH,
    &ASK_TAPE_STATE,
    &ASK_BAR_SHAPE,
    &ASK_MAGNET,
    &ASK_DELTA_DIVERGENCE,
];

impl JevAsk {
    pub fn question(&self) -> JevQuestion {
        match self.kind {
            AskKind::Noul => JevQuestion::Noul {
                instructions: self.instructions.into(),
            },
            AskKind::Choice(opts) => JevQuestion::Choice {
                instructions: self.instructions.into(),
                criteria: opts
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
            },
            AskKind::Score(c) => JevQuestion::Score {
                instructions: self.instructions.into(),
                criteria: c.iter().map(|x| x.to_string()).collect(),
            },
        }
    }

    /// The answer in a few words, for a trade's `why` and the logs.
    pub fn answer_text(answer: &JevAnswer) -> String {
        match answer {
            JevAnswer::Choice {
                choice,
                confidence,
                probabilities,
            } => {
                let c = if *confidence > 0.0 {
                    *confidence
                } else {
                    probabilities.get(choice).copied().unwrap_or(0.0)
                };
                format!("{choice} {:.0}%", c * 100.0)
            }
            JevAnswer::Noul { noul } => format!("yes {:.0}%", noul * 100.0),
            JevAnswer::Score { score, .. } => format!("score {score:.1}"),
        }
    }

    /// The label the scorer buckets an answer under.
    pub fn bucket(answer: &JevAnswer) -> String {
        match answer {
            JevAnswer::Choice { choice, .. } => choice.clone(),
            JevAnswer::Noul { noul } => {
                if *noul >= 0.6 {
                    "yes>=60%".into()
                } else if *noul >= 0.4 {
                    "40-60%".into()
                } else {
                    "no<40%".into()
                }
            }
            JevAnswer::Score { score, .. } => format!("score {}", score.round() as i64),
        }
    }
}

/// The floor's five questions plus the crew's, in one body: one call answers everyone.
pub fn all_questions() -> HashMap<String, JevQuestion> {
    let mut q = floor_questions();
    for a in CREW_ASKS.iter() {
        q.insert(a.key.to_string(), a.question());
    }
    q
}

/// A choice answer's pick and how sure Jev is of it (its stated confidence, else the pick's
/// probability).
fn choice_and_confidence<'a>(key: &str, answer: &'a JevAnswer) -> Result<(&'a str, f32), String> {
    let JevAnswer::Choice {
        choice,
        confidence,
        probabilities,
    } = answer
    else {
        return Err(format!("{key} is not a choice"));
    };
    let conf = if *confidence > 0.0 {
        *confidence
    } else {
        probabilities.get(choice).copied().unwrap_or(0.0)
    };
    Ok((choice.as_str(), conf as f32))
}

/// Buy or sell, for a trade relative to a touched level.
pub fn level_side_to_ask(trigger: &ObsTrigger, side: LevelSide) -> AskSide {
    let from_above = trigger.side.contains("above");
    match (side, from_above) {
        (LevelSide::Away, true) | (LevelSide::Through, false) => AskSide::Buy,
        (LevelSide::Away, false) | (LevelSide::Through, true) => AskSide::Sell,
    }
}

impl AskRule {
    /// The side this answer implies, or why the trader passes.
    pub fn side(
        &self,
        key: &str,
        answer: &JevAnswer,
        trigger: Option<&ObsTrigger>,
        lean: Direction,
    ) -> Result<AskSide, String> {
        match self {
            AskRule::LevelChoice { map, min } => {
                let t = trigger.ok_or_else(|| "no touch to trade".to_string())?;
                let (choice, conf) = choice_and_confidence(key, answer)?;
                let ls = map
                    .iter()
                    .find(|(c, _)| *c == choice)
                    .map(|(_, s)| *s)
                    .ok_or_else(|| format!("Jev {key} = {choice}: no trade"))?;
                if conf < *min {
                    return Err(format!(
                        "Jev {key} = {choice} at {:.0}% < {:.0}%",
                        conf * 100.0,
                        min * 100.0
                    ));
                }
                Ok(level_side_to_ask(t, ls))
            }
            AskRule::Choice { map, min } => {
                let (choice, conf) = choice_and_confidence(key, answer)?;
                let side = map
                    .iter()
                    .find(|(c, _)| *c == choice)
                    .map(|(_, s)| *s)
                    .ok_or_else(|| format!("Jev {key} = {choice}: no trade"))?;
                if conf < *min {
                    return Err(format!(
                        "Jev {key} = {choice} at {:.0}% < {:.0}%",
                        conf * 100.0,
                        min * 100.0
                    ));
                }
                Ok(side)
            }
            AskRule::NoulLevel { min, side } => {
                let t = trigger.ok_or_else(|| "no touch to trade".to_string())?;
                let JevAnswer::Noul { noul } = answer else {
                    return Err(format!("{key} is not a yes/no"));
                };
                if (*noul as f32) < *min {
                    return Err(format!(
                        "Jev {key} {:.0}% < {:.0}%",
                        noul * 100.0,
                        min * 100.0
                    ));
                }
                Ok(level_side_to_ask(t, *side))
            }
            AskRule::NoulLean { min, with_lean } => {
                let JevAnswer::Noul { noul } = answer else {
                    return Err(format!("{key} is not a yes/no"));
                };
                if (*noul as f32) < *min {
                    return Err(format!(
                        "Jev {key} {:.0}% < {:.0}%",
                        noul * 100.0,
                        min * 100.0
                    ));
                }
                let lean_side = match lean {
                    Direction::Up => AskSide::Buy,
                    Direction::Down => AskSide::Sell,
                    Direction::Flat => {
                        return Err(format!("Jev {key} says yes but has no lean to trade"));
                    }
                };
                Ok(if *with_lean {
                    lean_side
                } else {
                    match lean_side {
                        AskSide::Buy => AskSide::Sell,
                        AskSide::Sell => AskSide::Buy,
                    }
                })
            }
        }
    }

    /// The rule in words.
    pub fn text(&self) -> String {
        let ls = |s: &LevelSide| match s {
            LevelSide::Away => "away from the level",
            LevelSide::Through => "through the level",
        };
        match self {
            AskRule::LevelChoice { map, min } => format!(
                "{}; the answer must be at least {:.0}% sure",
                map.iter()
                    .map(|(c, s)| format!("{c}: {}", ls(s)))
                    .collect::<Vec<_>>()
                    .join(", "),
                min * 100.0
            ),
            AskRule::Choice { map, min } => format!(
                "{}; the answer must be at least {:.0}% sure",
                map.iter()
                    .map(|(c, s)| format!(
                        "{c}: {}",
                        if *s == AskSide::Buy { "buys" } else { "sells" }
                    ))
                    .collect::<Vec<_>>()
                    .join(", "),
                min * 100.0
            ),
            AskRule::NoulLevel { min, side } => {
                format!("yes at {:.0}% or more: trades {}", min * 100.0, ls(side))
            }
            AskRule::NoulLean { min, with_lean } => format!(
                "yes at {:.0}% or more: trades {} Jev's lean",
                min * 100.0,
                if *with_lean { "with" } else { "against" }
            ),
        }
    }
}

// ---------------------------------------------------------------------------
// Interpretation
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Direction {
    Up,
    Down,
    Flat,
}

impl Direction {
    pub fn label(self) -> &'static str {
        match self {
            Direction::Up => "UP",
            Direction::Down => "DOWN",
            Direction::Flat => "FLAT",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReadSource {
    Jev,
    Deterministic,
    Mock,
}

impl ReadSource {
    pub fn label(self) -> &'static str {
        match self {
            ReadSource::Jev => "JEV",
            ReadSource::Deterministic => "FLOOR RULE",
            ReadSource::Mock => "MOCK",
        }
    }
}

/// Typed answers plus provenance. `at` is app time (seconds) when it landed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MarketInterpretation {
    pub direction: Direction,
    pub direction_confidence: f32,
    pub p_up: f32,
    pub p_down: f32,
    pub p_flat: f32,
    /// 0 weak .. 2 strong.
    pub conviction: f32,
    pub continuation: f32,
    pub exhaustion: f32,
    /// absorbing | rejecting | sweeping | breaking | no_level.
    pub reaction: String,
    pub reaction_confidence: f32,
    pub model: String,
    pub source: ReadSource,
    pub latency_ms: u64,
    pub observation_hash: String,
    pub at: f64,
    pub input_tokens: u32,
    pub output_tokens: u32,
    /// Every answer in the response, keyed by question, so each crew member reads its own.
    #[serde(default)]
    pub answers: HashMap<String, JevAnswer>,
    /// The touch this read was asked about, when it was a touch read (the desk sets it).
    #[serde(default)]
    pub trigger: Option<ObsTrigger>,
}

impl MarketInterpretation {
    /// Signed lean -1..1 from the direction probabilities.
    pub fn lean(&self) -> f32 {
        (self.p_up - self.p_down).clamp(-1.0, 1.0)
    }

    /// What the crowd takes from the read: how far to lean and how sure to be.
    pub fn floor_read(&self) -> FloorRead {
        FloorRead {
            lean: self.lean(),
            confidence: (self.direction_confidence * (0.4 + 0.3 * self.conviction)).clamp(0.0, 1.0),
            exhaustion: self.exhaustion.clamp(0.0, 1.0),
        }
    }

    /// Board line: the numbers, then the words.
    pub fn headline(&self) -> String {
        format!(
            "{}: {} {:.0}%  |  conviction {:.1}/2  |  continuation {:.0}%  |  exhaustion {:.0}%  |  {}",
            self.source.label(),
            self.direction.label(),
            self.direction_confidence * 100.0,
            self.conviction,
            self.continuation * 100.0,
            self.exhaustion * 100.0,
            self.narrate()
        )
    }

    /// One sentence a floor broker might say, composed from the typed answers (never from prose).
    pub fn narrate(&self) -> String {
        let who = match self.direction {
            Direction::Up => "buyers have it",
            Direction::Down => "sellers have it",
            Direction::Flat => "nobody has it",
        };
        let strength = if self.conviction >= 1.5 {
            "and they mean it"
        } else if self.conviction >= 0.75 {
            "for now"
        } else {
            "barely"
        };
        let tail = if self.exhaustion >= 0.6 {
            "; looks tired"
        } else if self.continuation >= 0.65 {
            "; more to come"
        } else if self.continuation <= 0.35 {
            "; fade risk"
        } else {
            ""
        };
        let at_level = match self.reaction.as_str() {
            "absorbing" => "; absorbing at the level",
            "rejecting" => "; rejecting the level",
            "sweeping" => "; sweeping the level",
            "breaking" => "; breaking the level",
            _ => "",
        };
        if self.direction == Direction::Flat {
            format!("{who}{tail}{at_level}")
        } else {
            format!("{who} {strength}{tail}{at_level}")
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ModelError {
    Transport(String),
    Status(u16, String),
    Malformed(String),
    RateLimited(u64),
}

impl ModelError {
    pub fn short(&self) -> String {
        match self {
            ModelError::Transport(e) => format!(
                "relay unreachable ({})",
                e.chars().take(60).collect::<String>()
            ),
            ModelError::Status(code, body) => {
                format!("HTTP {code} {}", body.chars().take(60).collect::<String>())
            }
            ModelError::Malformed(e) => {
                format!("bad answer ({})", e.chars().take(60).collect::<String>())
            }
            ModelError::RateLimited(ms) => format!("rate limited, retry in {ms} ms"),
        }
    }
}

pub type Done = Box<dyn FnOnce(Result<MarketInterpretation, ModelError>) + Send + 'static>;

/// Provider-neutral boundary. `analyse` must not block: it hands the result to `done`, now or
/// later, from any thread.
pub trait DecisionModel: Send + Sync {
    fn label(&self) -> &'static str;
    fn analyse(&self, obs: &MarketObservation, done: Done);
}

/// Convert System One's answers into an interpretation. Pure, so it is testable without Jev.
pub fn interpret(
    resp: &JevResponse,
    obs_hash: &str,
    latency_ms: u64,
    source: ReadSource,
) -> Result<MarketInterpretation, ModelError> {
    let (direction, direction_confidence, p_up, p_down, p_flat) = match resp
        .answers
        .get("direction")
    {
        Some(JevAnswer::Choice {
            choice,
            confidence,
            probabilities,
        }) => {
            let p = |k: &str| probabilities.get(k).copied().unwrap_or(0.0) as f32;
            let dir = match choice.as_str() {
                "up" => Direction::Up,
                "down" => Direction::Down,
                "flat" => Direction::Flat,
                other => return Err(ModelError::Malformed(format!("direction choice {other:?}"))),
            };
            let conf = if *confidence > 0.0 {
                *confidence as f32
            } else {
                p(choice).max(0.0)
            };
            (dir, conf.clamp(0.0, 1.0), p("up"), p("down"), p("flat"))
        }
        Some(other) => return Err(ModelError::Malformed(format!("direction is {other:?}"))),
        None => return Err(ModelError::Malformed("no direction answer".into())),
    };
    let conviction = match resp.answers.get("conviction") {
        Some(JevAnswer::Score { score, .. }) => (*score as f32).clamp(0.0, 2.0),
        _ => return Err(ModelError::Malformed("no conviction score".into())),
    };
    let noul = |name: &str| -> Result<f32, ModelError> {
        match resp.answers.get(name) {
            Some(JevAnswer::Noul { noul }) => Ok((*noul as f32).clamp(0.0, 1.0)),
            _ => Err(ModelError::Malformed(format!("no {name} noul"))),
        }
    };
    // The reaction question is newer than the first recorded inferences: missing means no_level.
    let (reaction, reaction_confidence) = match resp.answers.get("level_reaction") {
        Some(JevAnswer::Choice {
            choice,
            confidence,
            probabilities,
        }) => {
            let conf = if *confidence > 0.0 {
                *confidence as f32
            } else {
                probabilities.get(choice).copied().unwrap_or(0.0) as f32
            };
            (choice.clone(), conf.clamp(0.0, 1.0))
        }
        _ => ("no_level".to_string(), 0.0),
    };
    Ok(MarketInterpretation {
        direction,
        direction_confidence,
        p_up,
        p_down,
        p_flat,
        conviction,
        continuation: noul("continuation")?,
        exhaustion: noul("exhaustion")?,
        reaction,
        reaction_confidence,
        answers: resp.answers.clone(),
        trigger: None,
        model: resp.model.clone(),
        source,
        latency_ms,
        observation_hash: obs_hash.to_string(),
        at: 0.0,
        input_tokens: resp.usage.input_tokens,
        output_tokens: resp.usage.output_tokens,
    })
}

// ---------------------------------------------------------------------------
// Providers
// ---------------------------------------------------------------------------

/// System One through the relay. The body is `{state, questions}`; the relay adds the model
/// name and the key.
pub struct JevModel {
    pub url: String,
    questions: HashMap<String, JevQuestion>,
}

impl JevModel {
    pub fn new(url: String) -> Self {
        Self {
            url,
            questions: all_questions(),
        }
    }
}

impl DecisionModel for JevModel {
    fn label(&self) -> &'static str {
        "Jev"
    }

    fn analyse(&self, obs: &MarketObservation, done: Done) {
        let req = match system_one_request(&self.url, obs, &self.questions) {
            Ok(r) => r,
            Err(e) => return done(Err(e)),
        };
        let hash = obs.hash();
        let started = wall_clock_ms();
        ehttp::fetch(req, move |res| {
            let latency = (wall_clock_ms() - started).max(0) as u64;
            done(system_one_result(res, &hash, latency));
        });
    }
}

/// The relay request for one observation. Shared by the app (async) and the replay (blocking).
pub fn system_one_request(
    relay: &str,
    obs: &MarketObservation,
    questions: &HashMap<String, JevQuestion>,
) -> Result<ehttp::Request, ModelError> {
    let body = SystemOneBody {
        state: obs.state_text(),
        questions,
    };
    let bytes =
        serde_json::to_vec(&body).map_err(|e| ModelError::Malformed(format!("encode: {e}")))?;
    let mut req = ehttp::Request::post(format!("{relay}/jev"), bytes);
    req.headers = ehttp::Headers::new(&[
        ("Accept", "application/json"),
        ("Content-Type", "application/json"),
    ]);
    Ok(req)
}

/// The relay's answer, typed.
pub fn system_one_result(
    res: ehttp::Result<ehttp::Response>,
    hash: &str,
    latency_ms: u64,
) -> Result<MarketInterpretation, ModelError> {
    match res {
        Ok(r) if r.ok => match serde_json::from_slice::<JevResponse>(&r.bytes) {
            Ok(parsed) => interpret(&parsed, hash, latency_ms, ReadSource::Jev),
            Err(e) => Err(ModelError::Malformed(e.to_string())),
        },
        Ok(r) if r.status == 429 => {
            let retry = serde_json::from_slice::<serde_json::Value>(&r.bytes)
                .ok()
                .and_then(|v| v.get("retry_after_ms").and_then(|x| x.as_u64()))
                .unwrap_or(2000);
            Err(ModelError::RateLimited(retry))
        }
        Ok(r) => Err(ModelError::Status(
            r.status,
            r.text().unwrap_or("").to_string(),
        )),
        Err(e) => Err(ModelError::Transport(e)),
    }
}

/// The fallback: a floor rule of thumb from the tape and the pit. Answers immediately.
#[derive(Default)]
pub struct DeterministicModel;

impl DeterministicModel {
    pub fn read(obs: &MarketObservation) -> MarketInterpretation {
        let tape = obs.tape.direction;
        let bars = (obs.net_change_ticks() / 12.0).clamp(-1.0, 1.0);
        // Delta of the last three bars, as a second read of the same tape over a longer window.
        let recent: f32 = obs.bars.iter().rev().take(3).filter_map(|b| b.delta).sum();
        let recent = (recent / 15.0).clamp(-1.0, 1.0);
        let lean = (0.5 * tape + 0.3 * recent + 0.2 * bars).clamp(-1.0, 1.0);
        let p_up = (0.33 + 0.5 * lean).clamp(0.02, 0.96);
        let p_down = (0.33 - 0.5 * lean).clamp(0.02, 0.96);
        let p_flat = (1.0 - p_up - p_down).max(0.02);
        let direction = if lean > 0.15 {
            Direction::Up
        } else if lean < -0.15 {
            Direction::Down
        } else {
            Direction::Flat
        };
        let agree =
            (tape.signum() == recent.signum()) as i32 + (tape.signum() == bars.signum()) as i32;
        let conviction = (lean.abs() * 1.6 + agree as f32 * 0.2).clamp(0.0, 2.0);
        let continuation = if direction == Direction::Flat {
            0.5
        } else {
            (0.5 + 0.5 * lean.abs() * bars.signum() * lean.signum()).clamp(0.05, 0.95)
        };
        // Exhaustion: the last bars shrink in delta while the move persists, or a sweep that did not pay.
        let last_deltas: Vec<f32> = obs
            .bars
            .iter()
            .rev()
            .take(3)
            .filter_map(|b| b.delta)
            .collect();
        let shrinking = last_deltas.len() == 3 && last_deltas[0].abs() < last_deltas[2].abs() * 0.5;
        let exhaustion = if shrinking { 0.65 } else { 0.25 };
        // At a level: a busy tape that is not moving is absorption; a lean away from the level
        // is rejection; a lean through it is a break (a sweep needs the next read to tell).
        let reaction = match obs.level_at_price() {
            None => "no_level",
            Some(l) => {
                let toward = if l.distance_ticks < 0.0 {
                    -1.0
                } else if l.distance_ticks > 0.0 {
                    1.0
                } else {
                    0.0
                };
                if obs.tape.ticks >= 12 && tape.abs() < 0.2 {
                    "absorbing"
                } else if toward != 0.0 && tape * toward < -0.3 {
                    "rejecting"
                } else if toward != 0.0 && tape * toward > 0.3 {
                    "breaking"
                } else {
                    "absorbing"
                }
            }
        };
        MarketInterpretation {
            direction,
            direction_confidence: p_up.max(p_down).max(p_flat),
            p_up,
            p_down,
            p_flat,
            conviction,
            continuation,
            exhaustion,
            reaction: reaction.to_string(),
            reaction_confidence: if reaction == "no_level" { 1.0 } else { 0.5 },
            answers: HashMap::new(),
            trigger: None,
            model: "floor-rule-v2".into(),
            source: ReadSource::Deterministic,
            latency_ms: 0,
            observation_hash: obs.hash(),
            at: 0.0,
            input_tokens: 0,
            output_tokens: 0,
        }
    }
}

impl DecisionModel for DeterministicModel {
    fn label(&self) -> &'static str {
        "floor rule"
    }

    fn analyse(&self, obs: &MarketObservation, done: Done) {
        done(Ok(Self::read(obs)));
    }
}

/// Predetermined answers for tests; never calls anything.
pub struct MockModel(pub Mutex<Vec<Result<MarketInterpretation, ModelError>>>);

impl DecisionModel for MockModel {
    fn label(&self) -> &'static str {
        "mock"
    }

    fn analyse(&self, _obs: &MarketObservation, done: Done) {
        let next = self.0.lock().ok().and_then(|mut v| {
            if v.is_empty() {
                None
            } else {
                Some(v.remove(0))
            }
        });
        done(next.unwrap_or_else(|| Err(ModelError::Transport("mock exhausted".into()))));
    }
}

/// Where the relay is: the page's host on :8099 unless `?relay=` says otherwise; natively
/// `OPENCRY_JEV_URL` (see `endpoints`).
pub fn relay_url() -> String {
    crate::endpoints::relay_url()
}

// ---------------------------------------------------------------------------
// The desk: one resource that asks, waits, falls back and remembers
// ---------------------------------------------------------------------------

type Pending = Arc<Mutex<Vec<(Instrument, u64, Result<MarketInterpretation, ModelError>)>>>;

#[derive(Resource)]
pub struct JevDesk {
    pub enabled: bool,
    model: Box<dyn DecisionModel>,
    pending: Pending,
    /// (pit, request id, app time sent).
    in_flight: Option<(Instrument, u64, f64)>,
    next_request_at: f64,
    request_seq: u64,
    /// Latest read per pit, indexed by `Instrument as usize`.
    pub reads: Vec<Option<MarketInterpretation>>,
    pub last_error: String,
    pub calls: u32,
    pub failures: u32,
    pub fallbacks: u32,
    pub late: u32,
    /// Wall clock when the desk opened: bars before it carry no tick delta.
    session_start_ms: i64,
    /// Which pit got the previous read, so a new selection is read at once.
    last_read_inst: Option<Instrument>,
}

impl Default for JevDesk {
    fn default() -> Self {
        Self::with_model(Box::new(JevModel::new(relay_url())))
    }
}

impl JevDesk {
    pub fn with_model(model: Box<dyn DecisionModel>) -> Self {
        Self {
            enabled: true,
            model,
            pending: Arc::new(Mutex::new(Vec::new())),
            in_flight: None,
            next_request_at: 0.0,
            request_seq: 0,
            reads: vec![None; Instrument::ALL.len()],
            last_error: String::new(),
            calls: 0,
            failures: 0,
            fallbacks: 0,
            late: 0,
            session_start_ms: wall_clock_ms(),
            last_read_inst: None,
        }
    }

    pub fn provider(&self) -> &'static str {
        self.model.label()
    }

    pub fn read(&self, inst: Instrument) -> Option<&MarketInterpretation> {
        self.reads.get(inst as usize).and_then(|r| r.as_ref())
    }

    pub fn toggle(&mut self, controller: &mut MarketScenarioController) {
        self.enabled = !self.enabled;
        controller.announcement_banner = if self.enabled {
            format!(
                "JEV READS ON ({} every {:.0} s for the focused pit)",
                self.provider(),
                JEV_CADENCE_SECS
            )
        } else {
            "JEV READS OFF".to_string()
        };
    }

    /// One line for the board.
    pub fn board_line(&self, inst: Instrument, now: f64, live: bool) -> String {
        if !self.enabled {
            return "JEV: off  (J)".to_string();
        }
        if !live {
            return "JEV: waits for the live tape".to_string();
        }
        match self.read(inst) {
            Some(r) => {
                let age = (now - r.at).max(0.0);
                let mut line = r.headline();
                line.push_str(&format!("  |  {:.0} s ago", age));
                if r.source != ReadSource::Jev && !self.last_error.is_empty() {
                    line.push_str(&format!("  |  Jev: {}", self.last_error));
                }
                line
            }
            None if self.in_flight.is_some() => "JEV: reading the floor...".to_string(),
            None if !self.last_error.is_empty() => {
                format!("JEV: no read yet  |  {}", self.last_error)
            }
            None => "JEV: no read yet".to_string(),
        }
    }

    /// Build what the model is told about a pit right now. `at_ms` is the observation's wall
    /// time (real clock in the app, replay clock in the replay).
    #[allow(clippy::too_many_arguments)]
    pub fn observe(
        &self,
        inst: Instrument,
        store: &CandleStore,
        tf: Timeframe,
        feed: &LiveFeed,
        levels: &[Level],
        now: f64,
        at_ms: i64,
    ) -> Option<MarketObservation> {
        let PitFeedStatus::Live { rate, .. } = feed.pit_status(inst, now) else {
            return None;
        };
        let quote = feed.quote(inst)?;
        let flow = quote.flow(now);
        let series = store.series(inst, tf, FeedMode::Live);
        let n = series.bars.len();
        let session_start = self.session_start_ms;
        let bars: Vec<ObsBar> = series
            .bars
            .iter()
            .skip(n.saturating_sub(OBS_BARS))
            .map(|c| ObsBar {
                time_ms: c.time_ms,
                open: c.open,
                high: c.high,
                low: c.low,
                close: c.close,
                delta: if c.time_ms + tf.millis() > session_start {
                    Some(c.delta)
                } else {
                    None
                },
                sweep: c.sweep,
            })
            .collect();
        if bars.is_empty() {
            return None;
        }
        let market = ObsMarket {
            spread_ticks: (quote.spread / tick_size(inst) as f64) as f32,
            bid_sizes: quote.bids.iter().map(|b| b.1).collect(),
            ask_sizes: quote.asks.iter().map(|a| a.1).collect(),
        };
        Some(MarketObservation::from_parts(
            inst,
            at_ms,
            tf.label(),
            quote.price,
            bars,
            &flow,
            rate,
            market,
            levels,
        ))
    }

    /// Store a read that came from elsewhere (a recorded inference, the replay's own call).
    pub fn set_read(&mut self, inst: Instrument, read: MarketInterpretation) {
        self.reads[inst as usize] = Some(read);
    }

    /// Bars before this wall time carry no tick delta (the app opened after them).
    pub fn set_session_start_ms(&mut self, ms: i64) {
        self.session_start_ms = ms;
    }

    /// The floor's questions and the crew's: one body per read.
    pub fn questions(&self) -> HashMap<String, JevQuestion> {
        all_questions()
    }

    fn send(&mut self, inst: Instrument, obs: &MarketObservation, now: f64) {
        self.request_seq += 1;
        let id = self.request_seq;
        let pending = self.pending.clone();
        self.in_flight = Some((inst, id, now));
        self.next_request_at = now + JEV_CADENCE_SECS;
        self.calls += 1;
        self.last_read_inst = Some(inst);
        self.model.analyse(
            obs,
            Box::new(move |res| {
                if let Ok(mut p) = pending.lock() {
                    p.push((inst, id, res));
                }
            }),
        );
    }

    /// Drain finished reads. Late or failed ones fall back to the floor rule on the
    /// observation we have now, so the board always carries a read with its source named.
    fn settle(&mut self, now: f64, fallback_obs: Option<&MarketObservation>) {
        let done: Vec<_> = match self.pending.lock() {
            Ok(mut p) => p.drain(..).collect(),
            Err(_) => return,
        };
        for (inst, id, res) in done {
            let current = self.in_flight.map(|(_, cur, _)| cur == id).unwrap_or(false);
            if current {
                self.in_flight = None;
            }
            match res {
                Ok(mut r) if current => {
                    r.at = now;
                    self.last_error.clear();
                    self.reads[inst as usize] = Some(r);
                }
                Ok(_) => self.late += 1,
                Err(e) => {
                    self.failures += 1;
                    self.last_error = e.short();
                    if let ModelError::RateLimited(ms) = e {
                        self.next_request_at = self.next_request_at.max(now + ms as f64 / 1000.0);
                    }
                    self.fall_back(inst, fallback_obs, now);
                }
            }
        }
        if let Some((inst, _, since)) = self.in_flight
            && now - since > JEV_TIMEOUT_SECS
        {
            self.in_flight = None;
            self.late += 1;
            self.last_error = format!("no answer in {JEV_TIMEOUT_SECS:.0} s");
            self.fall_back(inst, fallback_obs, now);
        }
    }

    fn fall_back(&mut self, inst: Instrument, obs: Option<&MarketObservation>, now: f64) {
        let Some(obs) = obs.filter(|o| o.symbol == inst.symbol()) else {
            return;
        };
        let mut r = DeterministicModel::read(obs);
        r.at = now;
        self.fallbacks += 1;
        self.reads[inst as usize] = Some(r);
    }
}

/// System: every `JEV_CADENCE_SECS`, read the focused pit if it is live; settle answers; hand
/// the pit its floor read. Never blocks the frame.
pub fn update_jev_desk(
    time: Res<Time>,
    feed: Res<LiveFeed>,
    store: Res<CandleStore>,
    settings: Res<ChartSettings>,
    selection: Res<Selection>,
    levels: Res<LevelStore>,
    mut engines: ResMut<PitEngines>,
    mut desk: ResMut<JevDesk>,
) {
    let now = time.elapsed_secs_f64();
    let selected = selection.selected_instrument;
    let obs = selected.filter(|_| desk.enabled).and_then(|inst| {
        desk.observe(
            inst,
            &store,
            settings.timeframe,
            &feed,
            levels.for_pit(inst),
            now,
            wall_clock_ms(),
        )
    });
    desk.settle(now, obs.as_ref());

    if let (Some(inst), Some(obs)) = (selected, obs.as_ref()) {
        let new_pit = desk.last_read_inst != Some(inst);
        let due = now >= desk.next_request_at || (new_pit && desk.in_flight.is_none());
        if due && desk.in_flight.is_none() {
            desk.send(inst, obs, now);
        }
    }

    // The crowd hears the read only in the pit it is about, and only while that pit is live.
    for inst in Instrument::ALL {
        let live = matches!(feed.pit_status(inst, now), PitFeedStatus::Live { .. });
        let read = if desk.enabled && live && selected == Some(inst) {
            desk.read(inst)
                .filter(|r| now - r.at < JEV_CADENCE_SECS * 3.0)
                .map(|r| r.floor_read())
        } else {
            None
        };
        engines.pit_mut(inst).set_floor_read(read);
    }
}

/// Marker for the Jev line on the pit board.
#[derive(Component)]
pub struct Pit3dBoardJev;

pub fn update_jev_board(
    time: Res<Time>,
    feed: Res<LiveFeed>,
    selection: Res<Selection>,
    desk: Res<JevDesk>,
    mut texts: Query<(&mut Text, &mut TextColor), With<Pit3dBoardJev>>,
) {
    let now = time.elapsed_secs_f64();
    let Some(inst) = selection.selected_instrument else {
        return;
    };
    let live = matches!(feed.pit_status(inst, now), PitFeedStatus::Live { .. });
    let line = desk.board_line(inst, now, live);
    let color = match desk.read(inst) {
        Some(r) if desk.enabled && live => match (r.source, r.direction) {
            (ReadSource::Jev, Direction::Up) => Color::srgb(0.35, 0.95, 0.55),
            (ReadSource::Jev, Direction::Down) => Color::srgb(1.0, 0.45, 0.45),
            (ReadSource::Jev, Direction::Flat) => Color::srgb(0.85, 0.85, 0.70),
            _ => Color::srgb(0.70, 0.70, 0.78),
        },
        _ => Color::srgb(0.55, 0.58, 0.66),
    };
    for (mut t, mut c) in texts.iter_mut() {
        set_ui_text(&mut t, line.clone());
        if c.0 != color {
            *c = TextColor(color);
        }
    }
}

pub fn handle_jev_keys(
    keyboard: Res<ButtonInput<KeyCode>>,
    mut desk: ResMut<JevDesk>,
    mut controller: ResMut<MarketScenarioController>,
) {
    if keyboard.just_pressed(KeyCode::KeyJ) {
        desk.toggle(&mut controller);
    }
}

pub fn handle_jev_buttons(
    interaction_query: Query<
        (&Interaction, &crate::ui::TouchButtonAction),
        (Changed<Interaction>, With<Button>),
    >,
    mut desk: ResMut<JevDesk>,
    mut controller: ResMut<MarketScenarioController>,
) {
    for (interaction, action) in interaction_query.iter() {
        if *interaction == Interaction::Pressed
            && *action == crate::ui::TouchButtonAction::JevToggle
        {
            desk.toggle(&mut controller);
        }
    }
}

pub fn update_jev_button_style(
    desk: Res<JevDesk>,
    mut buttons: Query<
        (
            &crate::ui::TouchButtonAction,
            &mut BackgroundColor,
            &mut BorderColor,
        ),
        With<Button>,
    >,
) {
    if !desk.is_changed() {
        return;
    }
    for (action, mut bg, mut border) in buttons.iter_mut() {
        if *action != crate::ui::TouchButtonAction::JevToggle {
            continue;
        }
        if desk.enabled {
            *bg = BackgroundColor(Color::srgba(0.30, 0.20, 0.08, 0.95));
            *border = BorderColor::all(Color::srgb(0.95, 0.75, 0.30));
        } else {
            *bg = BackgroundColor(Color::srgba(0.06, 0.09, 0.14, 0.85));
            *border = BorderColor::all(Color::srgba(0.18, 0.28, 0.40, 0.55));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obs(lean_up: bool) -> MarketObservation {
        obs_at(lean_up, 48.12)
    }

    fn obs_at(lean_up: bool, price: f64) -> MarketObservation {
        let sign = if lean_up { 1.0 } else { -1.0 };
        let bars = (0..6)
            .map(|i| {
                let base = 48.0 + sign * 0.02 * i as f32;
                ObsBar {
                    time_ms: 1_792_575_600_000 + i * 60_000,
                    open: base,
                    high: base + 0.03,
                    low: base - 0.01,
                    close: base + sign * 0.02,
                    delta: Some(sign * 5.0),
                    sweep: 2.0,
                }
            })
            .collect();
        let flow = TickFlow {
            direction: sign * 0.6,
            up_share: if lean_up { 0.8 } else { 0.2 },
            ticks: 24,
            burst_ticks: 2.0,
        };
        let market = ObsMarket {
            spread_ticks: 2.0,
            bid_sizes: vec![50.0, 50.0, 150.0],
            ask_sizes: vec![50.0, 50.0, 150.0],
        };
        let levels = vec![
            Level {
                kind: crate::levels::LevelKind::BullOrderBlock,
                lo: 47.90,
                hi: 48.00,
                strength: 0.9,
                time_ms: 0,
                touched: false,
                pct: 0.0,
            },
            Level {
                kind: crate::levels::LevelKind::SwingHigh,
                lo: 48.30,
                hi: 48.31,
                strength: 0.6,
                time_ms: 0,
                touched: false,
                pct: 0.0,
            },
        ];
        MarketObservation::from_parts(
            Instrument::Silver,
            1_792_575_960_000,
            "1m",
            price,
            bars,
            &flow,
            2.4,
            market,
            &levels,
        )
    }

    #[test]
    fn state_text_carries_the_numbers_jev_needs() {
        let o = obs(true);
        let s = o.state_text();
        assert!(s.contains("INSTRUMENT: SILVER (XAG/USD)"));
        assert!(s.contains("TIME: 09:46 UTC, bars of 1m"));
        assert!(s.contains("upticks 80% of 24 ticks"));
        assert!(
            s.contains("TOUCH: spread 2 ticks, broker ladder sizes bid 50/50/150 ask 50/50/150")
        );
        assert!(s.contains("OB BULL 47.900..48.000 at -12 ticks"), "{s}");
        assert!(s.contains("SWING HI 48.300..48.310 at +18 ticks"), "{s}");
        assert!(
            !s.to_lowercase().contains("headcount") && !s.contains("PIT:"),
            "no simulated crowd in the observation"
        );
        assert!(o.level_at_price().is_none());
        assert!(s.lines().filter(|l| l.starts_with(" 09:")).count() == 6);
        assert_eq!(o.hash(), fnv64(&s));
    }

    #[test]
    fn a_touch_trigger_is_a_line_in_the_state_and_changes_the_hash() {
        let plain = obs(true);
        let mut touched = obs(true);
        touched.trigger = Some(ObsTrigger {
            kind: "touch".into(),
            level: "SWING HI".into(),
            lo: 48.30,
            hi: 48.31,
            side: "from below".into(),
            secs_since: 0.0,
        });
        let s = touched.state_text();
        assert!(
            s.contains("TRIGGER: price has just touched SWING HI 48.300..48.310 from below;"),
            "{s}"
        );
        assert_ne!(touched.hash(), plain.hash());
        touched.trigger.as_mut().unwrap().kind = "follow".into();
        touched.trigger.as_mut().unwrap().secs_since = 30.0;
        assert!(
            touched
                .state_text()
                .contains("TRIGGER: 30 s after price touched SWING HI 48.300..48.310 from below;")
        );
        // Older logged observations carry no trigger and still deserialise.
        let v: MarketObservation = serde_json::from_str(
            &serde_json::to_string(&plain)
                .unwrap()
                .replace(",\"trigger\":null", ""),
        )
        .unwrap();
        assert!(v.trigger.is_none());
    }

    #[test]
    fn crew_asks_turn_answers_into_sides() {
        let q = all_questions();
        assert_eq!(q.len(), 5 + CREW_ASKS.len());
        assert!(
            matches!(q.get("touch_outcome"), Some(JevQuestion::Choice { criteria, .. }) if criteria.len() == 4)
        );
        let from_above = ObsTrigger {
            kind: "touch".into(),
            level: "OB BULL".into(),
            lo: 47.90,
            hi: 48.00,
            side: "from above".into(),
            secs_since: 0.0,
        };
        let from_below = ObsTrigger {
            side: "from below".into(),
            ..from_above.clone()
        };
        let choice = |c: &str| JevAnswer::Choice {
            choice: c.into(),
            confidence: 0.7,
            probabilities: HashMap::new(),
        };
        let oracle = AskRule::LevelChoice {
            map: &[
                ("reject", LevelSide::Away),
                ("sweep", LevelSide::Away),
                ("break", LevelSide::Through),
            ],
            min: 0.45,
        };
        assert_eq!(
            oracle.side(
                "touch_outcome",
                &choice("reject"),
                Some(&from_above),
                Direction::Flat
            ),
            Ok(AskSide::Buy),
            "came down onto it and it held: buy"
        );
        assert_eq!(
            oracle.side(
                "touch_outcome",
                &choice("break"),
                Some(&from_above),
                Direction::Flat
            ),
            Ok(AskSide::Sell)
        );
        assert_eq!(
            oracle.side(
                "touch_outcome",
                &choice("reject"),
                Some(&from_below),
                Direction::Flat
            ),
            Ok(AskSide::Sell)
        );
        assert!(
            oracle
                .side(
                    "touch_outcome",
                    &choice("stall"),
                    Some(&from_above),
                    Direction::Flat
                )
                .unwrap_err()
                .contains("stall: no trade")
        );
        assert!(
            oracle
                .side("touch_outcome", &choice("reject"), None, Direction::Flat)
                .unwrap_err()
                .contains("no touch")
        );
        let faint = JevAnswer::Choice {
            choice: "break".into(),
            confidence: 0.09,
            probabilities: HashMap::new(),
        };
        assert!(
            oracle
                .side("touch_outcome", &faint, Some(&from_above), Direction::Flat)
                .unwrap_err()
                .contains("break at 9% < 45%"),
            "a 9% break is not a call"
        );
        let absorb = AskRule::NoulLevel {
            min: 0.6,
            side: LevelSide::Away,
        };
        assert_eq!(
            absorb.side(
                "absorbing",
                &JevAnswer::Noul { noul: 0.71 },
                Some(&from_below),
                Direction::Up
            ),
            Ok(AskSide::Sell)
        );
        assert!(
            absorb
                .side(
                    "absorbing",
                    &JevAnswer::Noul { noul: 0.4 },
                    Some(&from_below),
                    Direction::Up
                )
                .unwrap_err()
                .contains("40% < 60%")
        );
        let tape = AskRule::Choice {
            map: &[
                ("one_way_buying", AskSide::Buy),
                ("one_way_selling", AskSide::Sell),
            ],
            min: 0.5,
        };
        assert_eq!(
            tape.side(
                "tape_state",
                &choice("one_way_selling"),
                None,
                Direction::Up
            ),
            Ok(AskSide::Sell)
        );
        assert!(
            tape.side("tape_state", &choice("two_way"), None, Direction::Up)
                .is_err()
        );
        let fade = AskRule::NoulLean {
            min: 0.6,
            with_lean: false,
        };
        assert_eq!(
            fade.side(
                "delta_divergence",
                &JevAnswer::Noul { noul: 0.8 },
                None,
                Direction::Up
            ),
            Ok(AskSide::Sell)
        );
        assert!(
            fade.side(
                "delta_divergence",
                &JevAnswer::Noul { noul: 0.8 },
                None,
                Direction::Flat
            )
            .is_err()
        );
        assert_eq!(JevAsk::answer_text(&choice("reject")), "reject 70%");
        assert_eq!(JevAsk::bucket(&JevAnswer::Noul { noul: 0.65 }), "yes>=60%");
        assert!(
            oracle.text().contains("break: through the level"),
            "{}",
            oracle.text()
        );
        // Answers ride on the interpretation, so a trader can read its own.
        let resp: JevResponse = serde_json::from_str(r#"{"answers":{"direction":{"type":"choice","choice":"up","confidence":0.6,"probabilities":{"up":0.6,"down":0.2,"flat":0.2}},
            "conviction":{"type":"score","score":1.0},"continuation":{"type":"noul","noul":0.5},"exhaustion":{"type":"noul","noul":0.3},
            "touch_outcome":{"type":"choice","choice":"sweep","confidence":0.5}}}"#).unwrap();
        let r = interpret(&resp, "h", 1, ReadSource::Jev).unwrap();
        assert!(
            matches!(r.answers.get("touch_outcome"), Some(JevAnswer::Choice { choice, .. }) if choice == "sweep")
        );
    }

    #[test]
    fn bars_before_the_session_have_no_delta_in_the_text() {
        let mut o = obs(true);
        o.bars[0].delta = None;
        assert!(o.state_text().contains("delta n/a"));
    }

    #[test]
    fn deterministic_fallback_follows_the_tape() {
        let up = DeterministicModel::read(&obs(true));
        let down = DeterministicModel::read(&obs(false));
        assert_eq!(up.direction, Direction::Up);
        assert_eq!(down.direction, Direction::Down);
        assert!(up.lean() > 0.3 && down.lean() < -0.3);
        assert_eq!(up.source, ReadSource::Deterministic);
        assert!(
            up.continuation > 0.5,
            "move and lean agree: continuation {}",
            up.continuation
        );
    }

    #[test]
    fn system_one_answers_become_an_interpretation() {
        let json = r#"{"model":"jev-1.13.0","answers":{
            "conviction":{"type":"score","score":1.76,"confidence":0.62,"legend":{"0":"weak"},"probabilities":{"2":0.76}},
            "direction":{"type":"choice","choice":"up","confidence":0.98,"probabilities":{"down":0.01,"flat":0.0,"up":0.99}},
            "continuation":{"type":"noul","noul":0.71},
            "exhaustion":{"type":"noul","probability":0.12},
            "level_reaction":{"type":"choice","choice":"absorbing","confidence":0.8,"probabilities":{"absorbing":0.8,"no_level":0.2}}},
            "usage":{"input_tokens":444,"output_tokens":53}}"#;
        let resp: JevResponse = serde_json::from_str(json).unwrap();
        let r = interpret(&resp, "abc", 900, ReadSource::Jev).unwrap();
        assert_eq!(r.direction, Direction::Up);
        assert!((r.direction_confidence - 0.98).abs() < 1e-5);
        assert!((r.conviction - 1.76).abs() < 1e-5);
        assert!((r.continuation - 0.71).abs() < 1e-5);
        assert!((r.exhaustion - 0.12).abs() < 1e-5);
        assert_eq!(r.input_tokens, 444);
        assert_eq!(r.model, "jev-1.13.0");
        assert!(r.headline().starts_with("JEV: UP 98%"));
        assert_eq!(r.reaction, "absorbing");
        assert_eq!(
            r.narrate(),
            "buyers have it and they mean it; more to come; absorbing at the level"
        );
        let fr = r.floor_read();
        assert!(fr.lean > 0.9 && fr.confidence > 0.8);
    }

    #[test]
    fn at_a_level_the_floor_rule_names_a_reaction_and_old_answers_still_parse() {
        let o = obs_at(true, 48.0); // on the block's top edge
        let r = DeterministicModel::read(&o);
        assert!(o.level_at_price().is_some());
        assert_ne!(r.reaction, "no_level");
        let resp: JevResponse = serde_json::from_str(
            r#"{"answers":{
            "direction":{"type":"choice","choice":"flat","confidence":0.5},
            "conviction":{"type":"score","score":0.3},
            "continuation":{"type":"noul","noul":0.5},
            "exhaustion":{"type":"noul","noul":0.5}}}"#,
        )
        .unwrap();
        let r = interpret(&resp, "x", 0, ReadSource::Jev).unwrap();
        assert_eq!(r.reaction, "no_level");
    }

    #[test]
    fn a_missing_answer_is_malformed_not_a_panic() {
        let resp: JevResponse =
            serde_json::from_str(r#"{"answers":{"direction":{"type":"choice","choice":"up"}}}"#)
                .unwrap();
        assert!(matches!(
            interpret(&resp, "x", 0, ReadSource::Jev),
            Err(ModelError::Malformed(_))
        ));
        let resp: JevResponse = serde_json::from_str(
            r#"{"answers":{"direction":{"type":"choice","choice":"sideways"}}}"#,
        )
        .unwrap();
        assert!(matches!(
            interpret(&resp, "x", 0, ReadSource::Jev),
            Err(ModelError::Malformed(_))
        ));
    }

    #[test]
    fn request_body_is_what_system_one_expects() {
        let q = floor_questions();
        let body = SystemOneBody {
            state: "s".into(),
            questions: &q,
        };
        let v: serde_json::Value = serde_json::to_value(&body).unwrap();
        assert_eq!(v["questions"]["direction"]["type"], "choice");
        assert!(
            v["questions"]["direction"]["criteria"]["up"]
                .as_str()
                .unwrap()
                .contains("higher")
        );
        assert_eq!(v["questions"]["conviction"]["type"], "score");
        assert_eq!(
            v["questions"]["conviction"]["criteria"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
        assert_eq!(v["questions"]["continuation"]["type"], "noul");
        assert_eq!(
            v["questions"]["level_reaction"]["criteria"]
                .as_object()
                .unwrap()
                .len(),
            5
        );
        assert!(v.get("model").is_none(), "the relay names the model");
    }

    #[test]
    fn desk_falls_back_when_the_model_fails_and_keeps_jev_when_it_answers() {
        let good = {
            let mut r = DeterministicModel::read(&obs(true));
            r.source = ReadSource::Jev;
            r.model = "jev-test".into();
            r
        };
        let mock = MockModel(Mutex::new(vec![
            Err(ModelError::Transport("down".into())),
            Ok(good.clone()),
        ]));
        let mut desk = JevDesk::with_model(Box::new(mock));
        let o = obs(true);

        desk.send(Instrument::Silver, &o, 1.0);
        desk.settle(1.1, Some(&o));
        let r = desk.read(Instrument::Silver).expect("fallback read");
        assert_eq!(r.source, ReadSource::Deterministic);
        assert_eq!(desk.fallbacks, 1);
        assert!(desk.last_error.contains("relay unreachable"));
        assert!(desk.in_flight.is_none());

        desk.send(Instrument::Silver, &o, 12.0);
        desk.settle(12.5, Some(&o));
        let r = desk.read(Instrument::Silver).unwrap();
        assert_eq!(r.source, ReadSource::Jev);
        assert_eq!(r.model, "jev-test");
        assert!(desk.last_error.is_empty());
        assert_eq!(r.at, 12.5);
    }

    #[test]
    fn a_late_answer_times_out_into_the_fallback() {
        // The mock never answers: the Vec is empty, so analyse reports exhaustion... which is an
        // error, so use a model that swallows the callback instead.
        struct Silent;
        impl DecisionModel for Silent {
            fn label(&self) -> &'static str {
                "silent"
            }
            fn analyse(&self, _: &MarketObservation, _done: Done) {}
        }
        let mut desk = JevDesk::with_model(Box::new(Silent));
        let o = obs(false);
        desk.send(Instrument::Silver, &o, 0.0);
        desk.settle(JEV_TIMEOUT_SECS - 1.0, Some(&o));
        assert!(desk.read(Instrument::Silver).is_none());
        desk.settle(JEV_TIMEOUT_SECS + 1.0, Some(&o));
        let r = desk.read(Instrument::Silver).unwrap();
        assert_eq!(r.source, ReadSource::Deterministic);
        assert_eq!(r.direction, Direction::Down);
        assert_eq!(desk.late, 1);
    }

    #[test]
    fn board_line_names_the_state() {
        let desk = JevDesk::with_model(Box::new(DeterministicModel));
        assert_eq!(
            desk.board_line(Instrument::Gold, 0.0, false),
            "JEV: waits for the live tape"
        );
        assert_eq!(
            desk.board_line(Instrument::Gold, 0.0, true),
            "JEV: no read yet"
        );
        let mut off = JevDesk::with_model(Box::new(DeterministicModel));
        off.enabled = false;
        assert_eq!(off.board_line(Instrument::Gold, 0.0, true), "JEV: off  (J)");
    }
}
