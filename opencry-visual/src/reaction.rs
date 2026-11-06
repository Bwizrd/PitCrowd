//! Touches of levels, and what price did next.
//!
//! Nobody on this feed can see the size resting at a level, so the floor does the next best
//! thing: it remembers. Every level the floor marks (`levels::detect_levels`, the same code the
//! paper traders use) is watched for a *touch*: price arriving at the band from outside. What
//! price does over the minutes after a touch is one of four outcomes:
//!
//! * **reject**: it never traded through the far edge and ended at least half a unit back the
//!   way it came;
//! * **break**: it ended at least half a unit beyond the far edge;
//! * **sweep**: it traded through the far edge by a tick or more and came back;
//! * **stall**: none of those; it is still sitting at the level.
//!
//! Counted over a year of 1m bars per instrument, level kind, approach side and session, those
//! outcomes are the *reaction prior* (`PriorTable`, built by `src/bin/reaction_prior.rs` and
//! compiled in from `assets/reaction_prior.json`): how often this kind of level, approached
//! this way, at this time of day, has held. The same `TouchTracker` runs live in the paper
//! desk on ticks, and a touch there is what triggers a Jev read, so the model is asked about
//! a reaction while it is happening and scored against the same four outcomes.
//!
//! A unit is the mean 1m bar range of the last twenty bars, never under
//! `paper::MIN_UNIT_TICKS` ticks, exactly as the paper book sizes risk.

use std::collections::HashMap;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::candles::Candle;
use crate::levels::{Level, LevelKind};
use crate::paper::MIN_UNIT_TICKS;
use crate::report::Session;

/// After a touch, price must close this far beyond the band (in units, at least
/// `REARM_MIN_TICKS`) before the same level can be touched again.
pub const REARM_UNITS: f32 = 0.5;
pub const REARM_MIN_TICKS: f32 = 2.0;
/// A rejection has to carry price this far back from the near edge, in units.
pub const AWAY_UNITS: f32 = 0.5;
/// A break has to carry price this far beyond the far edge, in units.
pub const BREAK_UNITS: f32 = 0.5;
/// Horizons the prior is measured at, in minutes after the touch.
pub const PRIOR_HORIZONS_MIN: [u32; 2] = [5, 15];
/// A level unseen for this many steps is forgotten by the tracker.
const FORGET_AFTER_STEPS: u64 = 600;

/// Which way price came at the level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Side {
    /// Price fell onto it: the far edge is the band's low.
    FromAbove,
    /// Price rose into it: the far edge is the band's high.
    FromBelow,
}

impl Side {
    pub fn label(self) -> &'static str {
        match self {
            Side::FromAbove => "from above",
            Side::FromBelow => "from below",
        }
    }

    pub fn parse(s: &str) -> Option<Side> {
        match s.trim() {
            "from above" | "above" | "FromAbove" => Some(Side::FromAbove),
            "from below" | "below" | "FromBelow" => Some(Side::FromBelow),
            _ => None,
        }
    }
}

/// Price arriving at a level from outside it.
#[derive(Debug, Clone, PartialEq)]
pub struct Touch {
    pub kind: LevelKind,
    pub lo: f32,
    pub hi: f32,
    pub side: Side,
    pub at_ms: i64,
    /// Where price was when the touch registered.
    pub price: f32,
}

/// One step of price: the range it covered and where it ended. A tick is `Step::tick(p)`; a
/// bar is its low, high and close.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Step {
    pub lo: f32,
    pub hi: f32,
    pub close: f32,
}

impl Step {
    pub fn tick(p: f32) -> Self {
        Step {
            lo: p,
            hi: p,
            close: p,
        }
    }

    pub fn bar(c: &Candle) -> Self {
        Step {
            lo: c.low,
            hi: c.high,
            close: c.close,
        }
    }
}

/// The unit in ticks: mean range of the last twenty bars, floored.
pub fn unit_ticks(bars: &[Candle], tick: f32) -> f32 {
    let k = bars.len().min(20);
    if k == 0 || tick <= 0.0 {
        return MIN_UNIT_TICKS;
    }
    let mean = bars[bars.len() - k..]
        .iter()
        .map(|b| (b.high - b.low) / tick)
        .sum::<f32>()
        / k as f32;
    mean.max(MIN_UNIT_TICKS)
}

/// Watches a pit's levels step by step and reports each touch once. A level is armed while
/// price is outside it; a touch disarms it until price has closed `REARM_UNITS` away.
#[derive(Default)]
pub struct TouchTracker {
    prev_close: Option<f32>,
    /// (kind, lo, hi in ticks) -> (armed, last step seen)
    armed: HashMap<(LevelKind, i64, i64), (bool, u64)>,
    step_no: u64,
}

impl TouchTracker {
    /// Feed the next step. `unit` is in ticks. Levels born at or after `bar_open_ms` (the open
    /// of the bar this step belongs to) are skipped: a fresh session high is the current price,
    /// not a level price has arrived at. Returns the touches it produced, in the order of
    /// `levels` (which `levels::compose` sorts most useful first).
    pub fn step(
        &mut self,
        levels: &[Level],
        s: Step,
        at_ms: i64,
        bar_open_ms: i64,
        tick: f32,
        unit: f32,
    ) -> Vec<Touch> {
        self.step_no += 1;
        let mut out = Vec::new();
        let rearm = (REARM_UNITS * unit).max(REARM_MIN_TICKS) * tick;
        let tol = 0.5 * tick;
        if let Some(prev) = self.prev_close {
            for l in levels {
                if l.time_ms != 0 && l.time_ms >= bar_open_ms {
                    continue;
                }
                let key = (
                    l.kind,
                    (l.lo / tick).round() as i64,
                    (l.hi / tick).round() as i64,
                );
                let outside_prev = prev > l.hi + tol || prev < l.lo - tol;
                let e = self
                    .armed
                    .entry(key)
                    .or_insert((outside_prev, self.step_no));
                e.1 = self.step_no;
                if e.0 {
                    if outside_prev && s.hi >= l.lo - tol && s.lo <= l.hi + tol {
                        out.push(Touch {
                            kind: l.kind,
                            lo: l.lo,
                            hi: l.hi,
                            side: if prev > l.hi {
                                Side::FromAbove
                            } else {
                                Side::FromBelow
                            },
                            at_ms,
                            price: s.close,
                        });
                        e.0 = false;
                    }
                } else if s.close >= l.hi + rearm || s.close <= l.lo - rearm {
                    e.0 = true;
                }
            }
        }
        let n = self.step_no;
        self.armed.retain(|_, v| n - v.1 <= FORGET_AFTER_STEPS);
        self.prev_close = Some(s.close);
        out
    }

    /// Whether a level is currently armed (for tests and the HUD).
    pub fn is_armed(&self, l: &Level, tick: f32) -> bool {
        self.armed
            .get(&(
                l.kind,
                (l.lo / tick).round() as i64,
                (l.hi / tick).round() as i64,
            ))
            .map(|v| v.0)
            .unwrap_or(true)
    }
}

/// What price did after a touch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    Reject,
    Break,
    Sweep,
    Stall,
}

impl Outcome {
    pub const ALL: [Outcome; 4] = [
        Outcome::Reject,
        Outcome::Break,
        Outcome::Sweep,
        Outcome::Stall,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Outcome::Reject => "reject",
            Outcome::Break => "break",
            Outcome::Sweep => "sweep",
            Outcome::Stall => "stall",
        }
    }
}

/// Classify the path after a touch (the steps that followed it, oldest first; the touch's own
/// step is not included). Break beats sweep beats reject beats stall.
pub fn classify(t: &Touch, path: &[Step], tick: f32, unit: f32) -> Outcome {
    let Some(end) = path.last() else {
        return Outcome::Stall;
    };
    let u = unit * tick;
    let beyond = tick * 0.999;
    match t.side {
        Side::FromAbove => {
            let through = path.iter().any(|s| s.lo <= t.lo - beyond);
            if end.close <= t.lo - BREAK_UNITS * u {
                Outcome::Break
            } else if through {
                Outcome::Sweep
            } else if end.close >= t.hi + AWAY_UNITS * u {
                Outcome::Reject
            } else {
                Outcome::Stall
            }
        }
        Side::FromBelow => {
            let through = path.iter().any(|s| s.hi >= t.hi + beyond);
            if end.close >= t.hi + BREAK_UNITS * u {
                Outcome::Break
            } else if through {
                Outcome::Sweep
            } else if end.close <= t.lo - AWAY_UNITS * u {
                Outcome::Reject
            } else {
                Outcome::Stall
            }
        }
    }
}

// ---------------------------------------------------------------------------
// The prior
// ---------------------------------------------------------------------------

/// Counts of the four outcomes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Tally {
    pub n: u32,
    pub reject: u32,
    #[serde(rename = "break")]
    pub brk: u32,
    pub sweep: u32,
    pub stall: u32,
}

impl Tally {
    pub fn add(&mut self, o: Outcome) {
        self.n += 1;
        match o {
            Outcome::Reject => self.reject += 1,
            Outcome::Break => self.brk += 1,
            Outcome::Sweep => self.sweep += 1,
            Outcome::Stall => self.stall += 1,
        }
    }

    pub fn merge(&mut self, o: &Tally) {
        self.n += o.n;
        self.reject += o.reject;
        self.brk += o.brk;
        self.sweep += o.sweep;
        self.stall += o.stall;
    }

    pub fn count(&self, o: Outcome) -> u32 {
        match o {
            Outcome::Reject => self.reject,
            Outcome::Break => self.brk,
            Outcome::Sweep => self.sweep,
            Outcome::Stall => self.stall,
        }
    }

    pub fn rate(&self, o: Outcome) -> f32 {
        if self.n == 0 {
            0.0
        } else {
            self.count(o) as f32 / self.n as f32
        }
    }

    /// Share of touches the level survived (anything but a break).
    pub fn hold_rate(&self) -> f32 {
        1.0 - self.rate(Outcome::Break)
    }
}

/// Wilson 95% interval for k of n.
pub fn wilson(k: u32, n: u32) -> (f32, f32) {
    if n == 0 {
        return (0.0, 1.0);
    }
    let (k, n) = (k as f64, n as f64);
    let z = 1.96f64;
    let p = k / n;
    let d = 1.0 + z * z / n;
    let c = p + z * z / (2.0 * n);
    let h = z * ((p * (1.0 - p) + z * z / (4.0 * n)) / n).sqrt();
    (((c - h) / d) as f32, ((c + h) / d) as f32)
}

/// One cell of the prior.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PriorRow {
    pub symbol: String,
    /// `LevelKind::tag`.
    pub kind: String,
    pub side: Side,
    pub session: Session,
    pub horizon_min: u32,
    #[serde(flatten)]
    pub tally: Tally,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct PriorTable {
    pub built_at: String,
    pub from: String,
    pub to: String,
    #[serde(default)]
    pub bars: u64,
    pub rows: Vec<PriorRow>,
}

static BUILTIN_PRIOR_JSON: &str = include_str!("../assets/reaction_prior.json");

impl PriorTable {
    /// The table compiled in from `assets/reaction_prior.json` (empty if the file is empty or
    /// unreadable, so nothing that consults it can fail).
    pub fn builtin() -> &'static PriorTable {
        static CELL: OnceLock<PriorTable> = OnceLock::new();
        CELL.get_or_init(|| serde_json::from_str(BUILTIN_PRIOR_JSON).unwrap_or_default())
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn add(
        &mut self,
        symbol: &str,
        kind: LevelKind,
        side: Side,
        session: Session,
        horizon_min: u32,
        o: Outcome,
    ) {
        let tag = kind.tag();
        if let Some(r) = self.rows.iter_mut().find(|r| {
            r.symbol == symbol
                && r.kind == tag
                && r.side == side
                && r.session == session
                && r.horizon_min == horizon_min
        }) {
            r.tally.add(o);
        } else {
            let mut tally = Tally::default();
            tally.add(o);
            self.rows.push(PriorRow {
                symbol: symbol.to_string(),
                kind: tag.to_string(),
                side,
                session,
                horizon_min,
                tally,
            });
        }
    }

    /// Sum the cells that match; `None` matches everything on that axis.
    pub fn tally(
        &self,
        symbol: Option<&str>,
        kind: Option<&str>,
        side: Option<Side>,
        session: Option<Session>,
        horizon_min: u32,
    ) -> Tally {
        let mut t = Tally::default();
        for r in &self.rows {
            if r.horizon_min != horizon_min {
                continue;
            }
            if symbol.is_some_and(|s| s != r.symbol)
                || kind.is_some_and(|k| k != r.kind)
                || side.is_some_and(|s| s != r.side)
                || session.is_some_and(|s| s != r.session)
            {
                continue;
            }
            t.merge(&r.tally);
        }
        t
    }

    /// The most specific cell with at least `min_n` touches: symbol+kind+side+session, then
    /// without the session, then without the symbol, then the kind alone. Returns the tally and
    /// a note on which it was.
    pub fn lookup(
        &self,
        symbol: &str,
        kind: LevelKind,
        side: Side,
        session: Session,
        horizon_min: u32,
        min_n: u32,
    ) -> (Tally, &'static str) {
        let k = kind.tag();
        let tries: [(
            Option<&str>,
            Option<&str>,
            Option<Side>,
            Option<Session>,
            &'static str,
        ); 4] = [
            (
                Some(symbol),
                Some(k),
                Some(side),
                Some(session),
                "this pit, kind, side and session",
            ),
            (
                Some(symbol),
                Some(k),
                Some(side),
                None,
                "this pit, kind and side, all sessions",
            ),
            (
                None,
                Some(k),
                Some(side),
                None,
                "all pits, this kind and side",
            ),
            (None, Some(k), None, None, "all pits, this kind"),
        ];
        for (s, kk, sd, ss, note) in tries {
            let t = self.tally(s, kk, sd, ss, horizon_min);
            if t.n >= min_n {
                return (t, note);
            }
        }
        (
            self.tally(None, Some(k), None, None, horizon_min),
            "all pits, this kind (thin)",
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TICK: f32 = 0.01;

    fn level(kind: LevelKind, lo: f32, hi: f32) -> Level {
        Level {
            kind,
            lo,
            hi,
            strength: 0.6,
            time_ms: 0,
            touched: false,
            pct: 0.0,
        }
    }

    fn touch(side: Side) -> Touch {
        Touch {
            kind: LevelKind::SwingHigh,
            lo: 48.295,
            hi: 48.305,
            side,
            at_ms: 0,
            price: 48.30,
        }
    }

    #[test]
    fn a_level_is_touched_once_until_price_has_left_it() {
        let lv = [level(LevelKind::SwingHigh, 48.295, 48.305)];
        let mut tr = TouchTracker::default();
        let unit = 8.0;
        assert!(
            tr.step(&lv, Step::tick(48.20), 0, 0, TICK, unit).is_empty(),
            "the first step only sets the previous close"
        );
        assert!(
            tr.step(&lv, Step::tick(48.25), 1, 0, TICK, unit).is_empty(),
            "still below it"
        );
        let t = tr.step(&lv, Step::tick(48.30), 2, 0, TICK, unit);
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].side, Side::FromBelow);
        assert_eq!((t[0].kind, t[0].at_ms), (LevelKind::SwingHigh, 2));
        // Chopping inside and just around it is the same touch.
        assert!(tr.step(&lv, Step::tick(48.31), 3, 0, TICK, unit).is_empty());
        assert!(tr.step(&lv, Step::tick(48.29), 4, 0, TICK, unit).is_empty());
        assert!(tr.step(&lv, Step::tick(48.30), 5, 0, TICK, unit).is_empty());
        // Re-armed once price closes half a unit (4 ticks) clear; the next arrival counts, from above this time.
        assert!(tr.step(&lv, Step::tick(48.35), 6, 0, TICK, unit).is_empty());
        assert!(tr.is_armed(&lv[0], TICK));
        let t = tr.step(
            &lv,
            Step {
                lo: 48.30,
                hi: 48.34,
                close: 48.32,
            },
            7,
            0,
            TICK,
            unit,
        );
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].side, Side::FromAbove);
        // A level born with price already inside it is not a touch until price leaves and returns.
        let inside = [level(LevelKind::Round, 48.315, 48.325)];
        let mut tr2 = TouchTracker::default();
        tr2.step(&inside, Step::tick(48.32), 0, 0, TICK, unit);
        assert!(
            tr2.step(&inside, Step::tick(48.32), 1, 0, TICK, unit)
                .is_empty()
        );
        assert!(
            tr2.step(&inside, Step::tick(48.40), 2, 0, TICK, unit)
                .is_empty()
        );
        assert_eq!(
            tr2.step(&inside, Step::tick(48.32), 3, 0, TICK, unit).len(),
            1
        );
        // A level born in the current bar (a fresh session high) is not touched by that bar.
        let fresh = [Level {
            time_ms: 5_000,
            ..level(LevelKind::SessionHigh, 48.395, 48.405)
        }];
        let mut tr3 = TouchTracker::default();
        tr3.step(&fresh, Step::tick(48.30), 4_000, 4_000, TICK, unit);
        assert!(
            tr3.step(
                &fresh,
                Step {
                    lo: 48.30,
                    hi: 48.40,
                    close: 48.38
                },
                5_000,
                5_000,
                TICK,
                unit
            )
            .is_empty()
        );
        assert!(
            tr3.step(&fresh, Step::tick(48.30), 6_000, 6_000, TICK, unit)
                .is_empty(),
            "left it"
        );
        assert_eq!(
            tr3.step(&fresh, Step::tick(48.40), 7_000, 7_000, TICK, unit)
                .len(),
            1,
            "the next bar can touch it"
        );
    }

    #[test]
    fn the_four_outcomes_from_above_and_below() {
        let unit = 8.0; // half a unit = 4 ticks = 0.04
        let t = touch(Side::FromAbove); // band 48.295..48.305, far edge 48.295
        assert_eq!(
            classify(
                &t,
                &[Step::tick(48.31), Step::tick(48.33), Step::tick(48.35)],
                TICK,
                unit
            ),
            Outcome::Reject
        );
        assert_eq!(
            classify(
                &t,
                &[Step::tick(48.29), Step::tick(48.26), Step::tick(48.25)],
                TICK,
                unit
            ),
            Outcome::Break
        );
        assert_eq!(
            classify(
                &t,
                &[
                    Step {
                        lo: 48.28,
                        hi: 48.31,
                        close: 48.31
                    },
                    Step::tick(48.33)
                ],
                TICK,
                unit
            ),
            Outcome::Sweep,
            "through by a tick and back"
        );
        assert_eq!(
            classify(
                &t,
                &[Step::tick(48.30), Step::tick(48.31), Step::tick(48.30)],
                TICK,
                unit
            ),
            Outcome::Stall
        );
        assert_eq!(classify(&t, &[], TICK, unit), Outcome::Stall);
        let t = touch(Side::FromBelow); // far edge 48.305
        assert_eq!(
            classify(
                &t,
                &[Step::tick(48.29), Step::tick(48.26), Step::tick(48.25)],
                TICK,
                unit
            ),
            Outcome::Reject
        );
        assert_eq!(
            classify(&t, &[Step::tick(48.33), Step::tick(48.35)], TICK, unit),
            Outcome::Break
        );
        assert_eq!(
            classify(
                &t,
                &[
                    Step {
                        lo: 48.29,
                        hi: 48.32,
                        close: 48.29
                    },
                    Step::tick(48.27)
                ],
                TICK,
                unit
            ),
            Outcome::Sweep
        );
        // Through and then far beyond is a break, not a sweep.
        assert_eq!(
            classify(
                &t,
                &[
                    Step {
                        lo: 48.29,
                        hi: 48.32,
                        close: 48.32
                    },
                    Step::tick(48.36)
                ],
                TICK,
                unit
            ),
            Outcome::Break
        );
    }

    #[test]
    fn the_unit_is_the_recent_bar_range_with_a_floor() {
        let bars: Vec<Candle> = (0..30)
            .map(|i| Candle {
                time_ms: i,
                open: 48.0,
                high: 48.0 + 0.10,
                low: 48.0,
                close: 48.0,
                volume: 1.0,
                delta: 0.0,
                sweep: 0.0,
            })
            .collect();
        assert!((unit_ticks(&bars, TICK) - 10.0).abs() < 1e-3);
        let tiny: Vec<Candle> = bars
            .iter()
            .map(|b| Candle {
                high: b.low + 0.01,
                ..*b
            })
            .collect();
        assert_eq!(unit_ticks(&tiny, TICK), MIN_UNIT_TICKS);
        assert_eq!(unit_ticks(&[], TICK), MIN_UNIT_TICKS);
    }

    #[test]
    fn the_prior_table_sums_cells_and_falls_back_to_wider_ones() {
        let mut p = PriorTable::default();
        for _ in 0..7 {
            p.add(
                "US500",
                LevelKind::SwingHigh,
                Side::FromBelow,
                Session::London,
                15,
                Outcome::Reject,
            );
        }
        for _ in 0..3 {
            p.add(
                "US500",
                LevelKind::SwingHigh,
                Side::FromBelow,
                Session::London,
                15,
                Outcome::Break,
            );
        }
        p.add(
            "XAUUSD",
            LevelKind::SwingHigh,
            Side::FromBelow,
            Session::Overlap,
            15,
            Outcome::Sweep,
        );
        p.add(
            "XAUUSD",
            LevelKind::SwingHigh,
            Side::FromBelow,
            Session::Overlap,
            5,
            Outcome::Stall,
        );
        assert_eq!(p.rows.len(), 3);
        let t = p.tally(
            Some("US500"),
            Some("SWING HI"),
            Some(Side::FromBelow),
            Some(Session::London),
            15,
        );
        assert_eq!((t.n, t.reject, t.brk), (10, 7, 3));
        assert!((t.hold_rate() - 0.7).abs() < 1e-6);
        assert_eq!(p.tally(None, Some("SWING HI"), None, None, 15).n, 11);
        assert_eq!(p.tally(None, None, None, None, 5).n, 1);
        // Lookup: the gold cell is thin, so it widens to all pits of this kind and side.
        let (t, note) = p.lookup(
            "XAUUSD",
            LevelKind::SwingHigh,
            Side::FromBelow,
            Session::Overlap,
            15,
            5,
        );
        assert_eq!(t.n, 11);
        assert_eq!(note, "all pits, this kind and side");
        let (t, note) = p.lookup(
            "US500",
            LevelKind::SwingHigh,
            Side::FromBelow,
            Session::London,
            15,
            5,
        );
        assert_eq!((t.n, note), (10, "this pit, kind, side and session"));
        // Round trip through JSON, the shape the asset file has.
        let s = serde_json::to_string(&p).unwrap();
        assert!(s.contains("\"break\":3"), "{s}");
        let back: PriorTable = serde_json::from_str(&s).unwrap();
        assert_eq!(back, p);
        let (lo, hi) = wilson(7, 10);
        assert!(
            lo > 0.39 && lo < 0.41 && hi > 0.88 && hi < 0.90,
            "{lo} {hi}"
        );
    }

    #[test]
    fn the_builtin_prior_parses() {
        let p = PriorTable::builtin();
        assert!(
            p.rows
                .iter()
                .all(|r| r.horizon_min == 5 || r.horizon_min == 15)
        );
    }
}
