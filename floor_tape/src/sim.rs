//! The synthetic market: a price path per symbol that is a pure function of the seed and the
//! minute, so the history the tape serves, the cache it writes for backtests and the live
//! ticks it streams are one and the same path, on any machine.
//!
//! Each minute moves the log price by `sigma * v * season * z`, where
//! - `v` is a slowly wandering volatility level (clustering, plus a rare news-like jump),
//! - `season` follows the FX day (quiet Asia, busy London/New York overlap, quiet late),
//! - and a very weak pull back to the symbol's anchor keeps a long run within sight of it
//!   (a half-life of sixty days: nothing a strategy could trade inside a day).
//!
//! **There is no edge in it, by construction.** Returns are uncorrelated from one minute to
//! the next (only their size clusters), so any strategy that makes money on this market did so
//! by luck. That makes it a null world: a paper trader or a genome that "works" here is the
//! false positive to beat on real data. (An earlier draft had a wandering trend term; it gave
//! hourly returns an autocorrelation of 0.15, a free edge for trend followers, and was cut.)
//!
//! Within a minute, the ticks are a Brownian bridge from the previous close to this close,
//! at random times, with as many ticks as the activity says. None of this is calibrated to
//! any real market beyond the order of magnitude in `symbols.rs`: it is there so the floor,
//! the paper desk and the backtests have something honest-looking to run on, offline.

use crate::rng::Rng;
use crate::symbols::{round_to, Spec, SYMBOLS};

/// The path starts at 2021-01-01T00:00:00Z, so the backtests have years before their 2025
/// holdout (evolve/evolve.py `HOLDOUT_FROM`) as well as after it.
pub const T0_MS: i64 = 1_609_459_200_000;
pub const MINUTE_MS: i64 = 60_000;
const DAY_MINUTES: i64 = 1440;
/// A minute's noise. The volatility level's mean square (about 1.2 with clustering and the
/// jumps) is divided out, so a day moves by the symbol's `daily_vol`.
fn minute_sigma(spec: &Spec) -> f64 {
    spec.daily_vol / (DAY_MINUTES as f64 * 1.25).sqrt()
}

/// Pull towards the anchor: a half-life of about sixty days.
const KAPPA: f64 = std::f64::consts::LN_2 / (60.0 * 1440.0);

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tick {
    pub t: i64,
    pub bid: f64,
    pub ask: f64,
}

#[derive(Debug, Clone, Copy)]
struct State {
    logp: f64,
    logv: f64,
}

impl State {
    fn start(spec: &Spec) -> Self {
        State { logp: spec.anchor.ln(), logv: 0.0 }
    }
}

/// Activity by time of day (UTC), mean square close to one over the day.
pub fn season(spec: &Spec, minute_of_day: i64) -> f64 {
    let h = minute_of_day as f64 / 60.0;
    if spec.always_busy {
        return if (13.0..21.0).contains(&h) { 1.1 } else { 0.85 };
    }
    match h {
        h if h < 7.0 => 0.55,
        h if h < 12.0 => 1.05,
        h if h < 16.5 => 1.45,
        h if h < 21.0 => 1.0,
        _ => 0.45,
    }
}

/// Move `st` across minute `m`; returns the minute's generator (positioned for its ticks),
/// its volatility level and its season.
fn advance(seed: u64, sym: usize, m: i64, st: &mut State) -> (Rng, f64, f64) {
    let spec = &SYMBOLS[sym];
    let mut rng = Rng::keyed(seed, sym as u64 + 1, m as u64);
    let (zv, z) = (rng.normal(), rng.normal());
    st.logv = 0.998 * st.logv + 0.02 * zv;
    if rng.unit() < 1.0 / (3.0 * 1440.0) {
        st.logv += 0.8;
    }
    let s = season(spec, m.rem_euclid(DAY_MINUTES));
    let v = st.logv.exp();
    let sigma = minute_sigma(spec);
    st.logp += sigma * v * s * z - KAPPA * (st.logp - spec.anchor.ln());
    (rng, v, s)
}

/// The ticks of one minute: a bridge from `open` to `close` (log prices).
fn bridge(spec: &Spec, m: i64, open: f64, close: f64, v: f64, s: f64, rng: &mut Rng) -> Vec<Tick> {
    let lambda = spec.tick_rate * 60.0 * s * v.sqrt().min(3.0);
    let k = rng.poisson(lambda).clamp(1, 600) as usize;
    let mut times: Vec<i64> = (0..k).map(|_| (rng.unit() * MINUTE_MS as f64) as i64).collect();
    times.sort_unstable();
    let step = minute_sigma(spec) * v * s / (k as f64).sqrt();
    let mut cum = Vec::with_capacity(k);
    let mut acc = 0.0;
    for _ in 0..k {
        acc += rng.normal();
        cum.push(acc);
    }
    let unit = 10f64.powi(-(spec.digits as i32));
    let t_start = T0_MS + m * MINUTE_MS;
    (0..k)
        .map(|i| {
            let f = (i + 1) as f64 / k as f64;
            let x = open + (cum[i] - f * acc) * step + f * (close - open);
            let bid = round_to(x.exp(), spec.digits);
            let spread = (spec.spread * (0.8 + 0.4 * rng.unit()) * v.sqrt().clamp(0.7, 3.0)).max(unit);
            Tick { t: t_start + times[i], bid, ask: round_to(bid + spread, spec.digits) }
        })
        .collect()
}

pub struct Sim {
    pub seed: u64,
    /// `checkpoints[sym][d]`: the state at the start of day `d`.
    checkpoints: Vec<Vec<State>>,
}

pub fn minute_of(t_ms: i64) -> i64 {
    (t_ms - T0_MS).div_euclid(MINUTE_MS)
}

impl Sim {
    pub fn new(seed: u64) -> Self {
        Sim { seed, checkpoints: SYMBOLS.iter().map(|s| vec![State::start(s)]).collect() }
    }

    fn ensure_day(&mut self, sym: usize, day: i64) {
        while (self.checkpoints[sym].len() as i64) <= day {
            let d = self.checkpoints[sym].len() as i64 - 1;
            let mut st = *self.checkpoints[sym].last().unwrap();
            for m in d * DAY_MINUTES..(d + 1) * DAY_MINUTES {
                advance(self.seed, sym, m, &mut st);
            }
            self.checkpoints[sym].push(st);
        }
    }

    /// Every minute in `from..to` with its ticks, in order. Minutes before the path starts
    /// have none.
    pub fn range(&mut self, sym: usize, from: i64, to: i64, mut f: impl FnMut(i64, Vec<Tick>)) {
        let from = from.max(0);
        if from >= to {
            return;
        }
        let day = from.div_euclid(DAY_MINUTES);
        self.ensure_day(sym, day);
        let mut st = self.checkpoints[sym][day as usize];
        for m in day * DAY_MINUTES..from {
            advance(self.seed, sym, m, &mut st);
        }
        for m in from..to {
            if m % DAY_MINUTES == 0 && (self.checkpoints[sym].len() as i64) == m / DAY_MINUTES {
                self.checkpoints[sym].push(st);
            }
            let open = st.logp;
            let (mut rng, v, s) = advance(self.seed, sym, m, &mut st);
            f(m, bridge(&SYMBOLS[sym], m, open, st.logp, v, s, &mut rng));
        }
    }

    pub fn minute(&mut self, sym: usize, m: i64) -> Vec<Tick> {
        let mut out = Vec::new();
        self.range(sym, m, m + 1, |_, t| out = t);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_minute_is_the_same_reached_alone_or_in_a_run() {
        let mut a = Sim::new(42);
        let mut run = Vec::new();
        a.range(0, 3000, 3005, |_, t| run.push(t));
        let mut b = Sim::new(42);
        assert_eq!(b.minute(0, 3003), run[3]);
        assert_ne!(Sim::new(43).minute(0, 3003), run[3]);
    }

    #[test]
    fn ticks_stay_inside_their_minute_and_ask_is_above_bid() {
        let mut s = Sim::new(1);
        for sym in 0..SYMBOLS.len() {
            let ticks = s.minute(sym, 50_000);
            assert!(!ticks.is_empty());
            let start = T0_MS + 50_000 * MINUTE_MS;
            assert!(ticks.iter().all(|t| t.t >= start && t.t < start + MINUTE_MS && t.ask > t.bid));
            assert!(ticks.windows(2).all(|w| w[0].t <= w[1].t));
        }
    }

    #[test]
    fn a_year_moves_like_its_daily_vol_and_stays_near_its_anchor() {
        let mut s = Sim::new(7);
        let sym = 3; // US500
        let mut closes = Vec::new();
        s.range(sym, 0, 365 * 1440, |m, t| {
            if m % 1440 == 1439 {
                closes.push(t.last().unwrap().bid);
            }
        });
        let rets: Vec<f64> = closes.windows(2).map(|w| (w[1] / w[0]).ln()).collect();
        let sd = (rets.iter().map(|r| r * r).sum::<f64>() / rets.len() as f64).sqrt();
        let spec = &SYMBOLS[sym];
        assert!(sd > spec.daily_vol * 0.6 && sd < spec.daily_vol * 1.8, "daily sd {sd}");
        let last = *closes.last().unwrap();
        let mut hourly = Vec::new();
        let mut prev: Option<f64> = None;
        s.range(sym, 0, 120 * 1440, |m, t| {
            if m % 60 == 59 {
                let c = t.last().unwrap().bid;
                if let Some(p) = prev {
                    hourly.push((c / p).ln());
                }
                prev = Some(c);
            }
        });
        let ac = hourly.windows(2).map(|w| w[0] * w[1]).sum::<f64>() / hourly.iter().map(|r| r * r).sum::<f64>();
        assert!(ac.abs() < 0.05, "hourly returns must not predict the next hour (no free edge): autocorrelation {ac}");
        assert!(last > spec.anchor * 0.6 && last < spec.anchor * 1.6, "drifted to {last}");
    }
}
