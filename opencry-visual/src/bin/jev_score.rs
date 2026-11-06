//! Jev scorer: are Jev's reads better than chance?
//!
//! Joins the relay's inference log (`jev_relay/log/inferences.jsonl`) to the tick bucket and
//! scores every read against what price did 1, 5, 15 and 60 minutes later. Nothing here calls
//! Jev; it only reads what the relay already recorded. The log defaults to this checkout's
//! `jev_relay/log/inferences.jsonl`, else the main checkout's (worktrees run no relay).
//!
//!   cargo run --release --bin jev_score                       # every read in the log
//!   cargo run --release --bin jev_score -- --last 24h --symbols XAGUSD,XAUUSD
//!   cargo run --release --bin jev_score -- --from 2026-10-21T16:00:00Z --to 2026-10-21T22:00:00Z \
//!        --horizons 1,5,15,60 --source influx --permutations 1000
//!
//! What is measured, per horizon, overall and per symbol:
//!   hit rate     up/down calls against the sign of the forward move (a zero move is a miss),
//!                with a Wilson 95% interval, the exact two-sided binomial p against 0.5 and a
//!                permutation control (direction labels shuffled 1000 times over the same
//!                outcomes; where the real hit rate sits in that null). Flat calls are scored
//!                separately: a hit when |move| stays under half a unit.
//!   Brier        three-class Brier of (p_up, p_down, p_flat) against the realised class
//!                (up/down when the move clears half a unit, flat otherwise), next to the
//!                uniform forecast (0.667) and the base-rate forecast so the number means
//!                something.
//!   conviction   mean forward move by direction x conviction bucket (score rounded to 0/1/2).
//!   levels       for reads taken within AT_LEVEL_TICKS of a level, what price did relative to
//!                that level: moved away (>= half a unit the other way), went through (closed
//!                beyond the far edge) or held, per reaction call.
//!   calibration  predicted probability decile vs realised frequency over the three classes.
//!
//! Units: moves are reported in ticks (`pit_engine::tick_size`) and in "units", one unit being
//! the mean 1m bar range of the bars Jev was shown, so instruments are comparable.
//!
//! Time: a live read's observation time is the relay stamp minus the model latency. A read
//! whose state TIME disagrees with the relay stamp by more than two minutes came from a replay
//! (`replay --jev live`), so its minute comes from the state and the exact second is pinned to
//! the first tick in that minute printing the state's LAST PRICE.
//!
//! Below `--min-n` (30) up/down calls, or below `--min-n` distinct symbol x horizon windows
//! (reads a minute apart share most of a forward window, so the window count is the honest
//! n), p-values and permutation percentiles print as "n too small"; hit rates and Wilson intervals are still shown because they stay honest at
//! any n. Output: a text report (stdout and `report.txt`), `summary.json` and `reads.csv` in
//! `opencry-visual/replay_out/jev_score_<stamp>/` (git-ignored).
//!
//! InfluxDB credentials as in `replay.rs`: INFLUX_URL / INFLUX_TOKEN / INFLUX_ORG, else
//! INFLUXDB_URL / INFLUXDB_TOKEN2 / INFLUXDB_ORG from the repo-root .env or OPENCRY_ENV_FILE (never
//! printed). `--source tape` uses the market tape's `GET :8083/bars` 1m bars instead (opens and
//! closes only: minute resolution).

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    if let Err(e) = jev_score::run() {
        eprintln!("jev_score: {e}");
        std::process::exit(1);
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod jev_score {
    use std::collections::{BTreeMap, HashMap};
    use std::fs;
    use std::io::Write;
    use std::path::PathBuf;
    use std::time::Instant;

    use opencry_visual::candles::{parse_iso_ms, wall_clock_ms};
    use opencry_visual::jev::{
        AT_LEVEL_TICKS, AskSide, CREW_ASKS, Direction, JevAnswer, JevAsk, JevResponse,
        MarketInterpretation, ObsTrigger, ReadSource, interpret,
    };
    use opencry_visual::levels::{LevelKind, tape_symbol, tape_url};
    use opencry_visual::market::Instrument;
    use opencry_visual::paper::JEV_CREW;
    use opencry_visual::pit_engine::tick_size;
    use opencry_visual::reaction::{
        Outcome as ReactionOutcome, Side as TouchSide, Step, Touch, classify,
    };

    // -----------------------------------------------------------------------
    // Scoring maths: pure functions, unit-tested below
    // -----------------------------------------------------------------------

    pub mod stats {
        use opencry_visual::jev::Direction;

        /// Half a unit either side of the start is "flat".
        pub const FLAT_BAND_UNITS: f64 = 0.5;

        #[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
        #[serde(rename_all = "lowercase")]
        pub enum Outcome {
            Up,
            Down,
            Flat,
        }

        impl Outcome {
            pub fn label(self) -> &'static str {
                match self {
                    Outcome::Up => "up",
                    Outcome::Down => "down",
                    Outcome::Flat => "flat",
                }
            }
            pub fn index(self) -> usize {
                match self {
                    Outcome::Up => 0,
                    Outcome::Down => 1,
                    Outcome::Flat => 2,
                }
            }
        }

        /// The realised class of a forward move measured in units of the recent 1m range.
        pub fn outcome(move_units: f64) -> Outcome {
            if move_units >= FLAT_BAND_UNITS {
                Outcome::Up
            } else if move_units <= -FLAT_BAND_UNITS {
                Outcome::Down
            } else {
                Outcome::Flat
            }
        }

        /// Did the call pay? Up needs a positive move, down a negative one (zero is a miss:
        /// a call that did not move price did not pay), flat needs the move to stay inside the
        /// band.
        pub fn hit(dir: Direction, move_ticks: f64, move_units: f64) -> bool {
            match dir {
                Direction::Up => move_ticks > 0.0,
                Direction::Down => move_ticks < 0.0,
                Direction::Flat => move_units.abs() < FLAT_BAND_UNITS,
            }
        }

        /// Wilson 95% interval for a proportion.
        pub fn wilson(hits: usize, n: usize) -> (f64, f64) {
            if n == 0 {
                return (0.0, 1.0);
            }
            let z = 1.959_964f64;
            let nf = n as f64;
            let p = hits as f64 / nf;
            let denom = 1.0 + z * z / nf;
            let centre = (p + z * z / (2.0 * nf)) / denom;
            let half = z * (p * (1.0 - p) / nf + z * z / (4.0 * nf * nf)).sqrt() / denom;
            ((centre - half).max(0.0), (centre + half).min(1.0))
        }

        /// Exact two-sided binomial test against p = 0.5: the total probability of every
        /// outcome at least as unlikely as the one observed. O(n), log space, fine for
        /// tens of thousands.
        pub fn binom_p_two_sided(hits: usize, n: usize) -> f64 {
            if n == 0 {
                return 1.0;
            }
            let mut ln_fact = vec![0.0f64; n + 1];
            for k in 1..=n {
                ln_fact[k] = ln_fact[k - 1] + (k as f64).ln();
            }
            let ln2n = n as f64 * std::f64::consts::LN_2;
            let lp = |k: usize| ln_fact[n] - ln_fact[k] - ln_fact[n - k] - ln2n;
            let obs = lp(hits);
            let mut p = 0.0;
            for k in 0..=n {
                let v = lp(k);
                if v <= obs + 1e-9 {
                    p += v.exp();
                }
            }
            p.min(1.0)
        }

        /// Three-class Brier: sum over classes of (p - realised)^2. 0 is perfect, the uniform
        /// forecast scores 2/3, a confident wrong call scores up to 2.
        pub fn brier3(p: [f64; 3], realised: Outcome) -> f64 {
            let mut s = 0.0;
            for (i, pi) in p.iter().enumerate() {
                let o = if realised.index() == i { 1.0 } else { 0.0 };
                s += (pi - o) * (pi - o);
            }
            s
        }

        /// Conviction score 0..2 into the nearest bucket.
        pub fn conviction_bucket(score: f32) -> u8 {
            (score.round().max(0.0) as u8).min(2)
        }

        /// xorshift64*: deterministic shuffles without pulling in a crate.
        pub struct Rng(u64);

        impl Rng {
            pub fn new(seed: u64) -> Self {
                Rng(seed.max(1) ^ 0x9E37_79B9_7F4A_7C15)
            }
            pub fn next_u64(&mut self) -> u64 {
                let mut x = self.0;
                x ^= x >> 12;
                x ^= x << 25;
                x ^= x >> 27;
                self.0 = x;
                x.wrapping_mul(0x2545_F491_4F6C_DD1D)
            }
            pub fn below(&mut self, n: usize) -> usize {
                (self.next_u64() % n as u64) as usize
            }
            pub fn shuffle<T>(&mut self, v: &mut [T]) {
                for i in (1..v.len()).rev() {
                    let j = self.below(i + 1);
                    v.swap(i, j);
                }
            }
        }

        #[derive(Debug, Clone, serde::Serialize)]
        pub struct Permutation {
            pub permutations: usize,
            pub real_rate: f64,
            pub null_mean: f64,
            pub null_p2_5: f64,
            pub null_p97_5: f64,
            /// Share of shuffles whose hit rate was at least the real one (one-sided p).
            pub p_at_least: f64,
            /// Where the real rate sits: share of shuffles strictly below it.
            pub percentile: f64,
            /// All labels one way means the shuffle cannot move the rate at all.
            pub degenerate: bool,
        }

        /// Shuffle the up/down labels over the same forward signs `n_perm` times.
        /// `signs` are the signs of the forward moves (-1, 0, +1); `calls` the direction labels
        /// (Flat must already be filtered out).
        pub fn permutation_control(
            calls: &[Direction],
            signs: &[i8],
            n_perm: usize,
            seed: u64,
        ) -> Option<Permutation> {
            if calls.is_empty() || calls.len() != signs.len() {
                return None;
            }
            let rate = |labels: &[Direction]| -> f64 {
                let hits = labels
                    .iter()
                    .zip(signs)
                    .filter(|(d, s)| matches!((d, s), (Direction::Up, 1) | (Direction::Down, -1)))
                    .count();
                hits as f64 / labels.len() as f64
            };
            let real = rate(calls);
            let ups = calls.iter().filter(|d| **d == Direction::Up).count();
            let degenerate = ups == 0 || ups == calls.len();
            let mut labels = calls.to_vec();
            let mut rng = Rng::new(seed);
            let mut null = Vec::with_capacity(n_perm);
            for _ in 0..n_perm {
                rng.shuffle(&mut labels);
                null.push(rate(&labels));
            }
            null.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let q = |f: f64| null[((null.len() - 1) as f64 * f).round() as usize];
            let at_least = null.iter().filter(|r| **r >= real - 1e-12).count();
            let below = null.iter().filter(|r| **r < real - 1e-12).count();
            Some(Permutation {
                permutations: n_perm,
                real_rate: real,
                null_mean: null.iter().sum::<f64>() / null.len() as f64,
                null_p2_5: q(0.025),
                null_p97_5: q(0.975),
                p_at_least: at_least as f64 / n_perm as f64,
                percentile: below as f64 / n_perm as f64,
                degenerate,
            })
        }

        #[derive(Debug, Clone, serde::Serialize)]
        pub struct CalibrationBucket {
            pub lo: f64,
            pub hi: f64,
            pub n: usize,
            pub mean_predicted: f64,
            pub realised: f64,
        }

        /// Ten deciles of predicted probability against the realised frequency.
        pub fn calibration(entries: &[(f64, bool)]) -> Vec<CalibrationBucket> {
            let mut n = [0usize; 10];
            let mut sum_p = [0.0f64; 10];
            let mut sum_o = [0usize; 10];
            for (p, o) in entries {
                let i = ((p.clamp(0.0, 1.0) * 10.0).floor() as usize).min(9);
                n[i] += 1;
                sum_p[i] += p;
                sum_o[i] += *o as usize;
            }
            (0..10)
                .map(|i| CalibrationBucket {
                    lo: i as f64 / 10.0,
                    hi: (i + 1) as f64 / 10.0,
                    n: n[i],
                    mean_predicted: if n[i] > 0 {
                        sum_p[i] / n[i] as f64
                    } else {
                        0.0
                    },
                    realised: if n[i] > 0 {
                        sum_o[i] as f64 / n[i] as f64
                    } else {
                        0.0
                    },
                })
                .collect()
        }

        pub fn mean(v: &[f64]) -> f64 {
            if v.is_empty() {
                0.0
            } else {
                v.iter().sum::<f64>() / v.len() as f64
            }
        }
    }

    // -----------------------------------------------------------------------
    // The relay log
    // -----------------------------------------------------------------------

    #[derive(Debug, Clone, PartialEq, serde::Serialize)]
    pub struct NearLevel {
        pub kind: String,
        pub lo: f64,
        pub hi: f64,
        pub distance_ticks: f64,
    }

    #[derive(Debug, Clone, PartialEq)]
    pub struct Read {
        /// Relay stamp.
        pub ts: i64,
        /// When the observation was taken (see module doc).
        pub obs_ms: i64,
        /// "relay" or "state".
        pub clock: &'static str,
        pub inst: Instrument,
        pub price: f64,
        pub tick: f64,
        /// Mean 1m bar range of the bars Jev saw, in ticks (at least 1).
        pub unit_ticks: f64,
        pub spread_ticks: Option<f64>,
        pub read: MarketInterpretation,
        /// The nearest listed level.
        pub level: Option<NearLevel>,
        /// "touch" or "follow" when the desk asked because price touched a level; None for a
        /// timed read.
        pub trigger: Option<String>,
        /// The touched level from the TRIGGER line.
        pub touch: Option<TouchLevel>,
        /// Every answer in the response, the crew's own questions included.
        pub answers: HashMap<String, JevAnswer>,
    }

    #[derive(Debug, Clone, PartialEq)]
    pub struct TouchLevel {
        pub kind: String,
        pub lo: f64,
        pub hi: f64,
        pub from_above: bool,
    }

    impl TouchLevel {
        /// `"price has just touched SWING HI 57.180..57.190 from below; ..."` or
        /// `"30 s after price touched SWING HI 57.180..57.190 from below; ..."`.
        pub fn parse(trigger_text: &str) -> Option<TouchLevel> {
            let rest = trigger_text.split("touched ").nth(1)?;
            let (left, right) = rest.split_once(" from ")?;
            let (kind, range) = left.trim().rsplit_once(' ')?;
            let (lo, hi) = range.split_once("..")?;
            Some(TouchLevel {
                kind: kind.trim().to_string(),
                lo: lo.parse().ok()?,
                hi: hi.parse().ok()?,
                from_above: right.trim_start().starts_with("above"),
            })
        }

        pub fn as_trigger(&self, kind: &str) -> ObsTrigger {
            ObsTrigger {
                kind: kind.into(),
                level: self.kind.clone(),
                lo: self.lo as f32,
                hi: self.hi as f32,
                side: if self.from_above {
                    "from above".into()
                } else {
                    "from below".into()
                },
                secs_since: 0.0,
            }
        }
    }

    impl Read {
        pub fn at_level(&self) -> bool {
            self.level
                .as_ref()
                .map(|l| l.distance_ticks.abs() <= AT_LEVEL_TICKS as f64)
                .unwrap_or(false)
        }
        /// +1 when the level is above price, -1 below, None when price is inside it dead centre.
        pub fn level_side(&self) -> Option<f64> {
            let l = self.level.as_ref()?;
            if l.distance_ticks > 0.0 {
                Some(1.0)
            } else if l.distance_ticks < 0.0 {
                Some(-1.0)
            } else {
                let mid = (l.lo + l.hi) / 2.0;
                if mid > self.price {
                    Some(1.0)
                } else if mid < self.price {
                    Some(-1.0)
                } else {
                    None
                }
            }
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
    pub enum Skip {
        NotJson,
        NoState,
        OldStateFormat,
        UnknownInstrument(String),
        NoPrice,
        NoResponse,
        BadResponse(String),
    }

    impl Skip {
        fn label(&self) -> String {
            match self {
                Skip::NotJson => "not json".into(),
                Skip::NoState => "no request.state".into(),
                Skip::OldStateFormat => "old prose state format".into(),
                Skip::UnknownInstrument(s) => format!("unknown instrument {s}"),
                Skip::NoPrice => "no LAST PRICE line".into(),
                Skip::NoResponse => "no response".into(),
                Skip::BadResponse(e) => format!("response not an interpretation ({e})"),
            }
        }
    }

    fn hhmm_minutes(s: &str) -> Option<i64> {
        let (h, m) = s.trim().split_once(':')?;
        let h: i64 = h.parse().ok()?;
        let m: i64 = m.get(..2)?.parse().ok()?;
        (h < 24 && m < 60).then_some(h * 60 + m)
    }

    /// `" SWING HI 57.398..57.408 at +1 ticks (already traded into once)"`.
    fn parse_level_line(line: &str) -> Option<NearLevel> {
        let (left, right) = line.trim().split_once(" at ")?;
        let (kind, range) = left.rsplit_once(' ')?;
        let (lo, hi) = range.split_once("..")?;
        let distance = right.split_whitespace().next()?.parse::<f64>().ok()?;
        Some(NearLevel {
            kind: kind.to_string(),
            lo: lo.parse().ok()?,
            hi: hi.parse().ok()?,
            distance_ticks: distance,
        })
    }

    /// `" 19:00 57.112 57.157 57.106 57.152 | delta +1 | sweep 2"` -> (high - low).
    fn parse_bar_range(line: &str) -> Option<f64> {
        let mut it = line.split_whitespace();
        hhmm_minutes(it.next()?)?;
        let _open: f64 = it.next()?.parse().ok()?;
        let high: f64 = it.next()?.parse().ok()?;
        let low: f64 = it.next()?.parse().ok()?;
        let _close: f64 = it.next()?.parse().ok()?;
        Some(high - low)
    }

    /// One relay log line into a scored-to-be read. Pure: the tick pinning happens later.
    pub fn parse_line(line: &str) -> Result<Read, Skip> {
        let v: serde_json::Value = serde_json::from_str(line).map_err(|_| Skip::NotJson)?;
        let ts = v.get("ts").and_then(|t| t.as_i64()).ok_or(Skip::NotJson)?;
        let state = v
            .pointer("/request/state")
            .and_then(|s| s.as_str())
            .ok_or(Skip::NoState)?;
        let first = state.lines().next().unwrap_or("");
        if !first.starts_with("INSTRUMENT:") {
            return Err(Skip::OldStateFormat);
        }
        let sym = first
            .split('(')
            .nth(1)
            .and_then(|s| s.split(')').next())
            .unwrap_or("")
            .to_string();
        let inst = Instrument::ALL
            .iter()
            .copied()
            .find(|i| i.symbol() == sym)
            .ok_or(Skip::UnknownInstrument(sym))?;
        let mut price: Option<f64> = None;
        let mut state_min: Option<i64> = None;
        let mut spread_ticks: Option<f64> = None;
        let mut ranges: Vec<f64> = Vec::new();
        let mut level: Option<NearLevel> = None;
        let mut trigger: Option<String> = None;
        let mut touch: Option<TouchLevel> = None;
        let mut in_levels = false;
        for l in state.lines() {
            if let Some(t) = l.strip_prefix("TRIGGER:") {
                in_levels = false;
                trigger = Some(if t.contains("after price touched") {
                    "follow".to_string()
                } else {
                    "touch".to_string()
                });
                touch = TouchLevel::parse(t);
            } else if let Some(p) = l.strip_prefix("LAST PRICE:") {
                price = p.trim().parse().ok();
            } else if let Some(t) = l.strip_prefix("TIME:") {
                state_min = hhmm_minutes(t.split_whitespace().next().unwrap_or(""));
            } else if let Some(t) = l.strip_prefix("TOUCH: spread") {
                spread_ticks = t.split_whitespace().next().and_then(|x| x.parse().ok());
            } else if l.starts_with("LEVELS") {
                in_levels = true;
            } else if in_levels && l.starts_with(' ') {
                if level.is_none() {
                    level = parse_level_line(l);
                }
            } else if l.starts_with(' ')
                && let Some(r) = parse_bar_range(l)
            {
                ranges.push(r);
            }
        }
        let price = price.ok_or(Skip::NoPrice)?;
        let resp = v.get("response").ok_or(Skip::NoResponse)?;
        let parsed: JevResponse =
            serde_json::from_value(resp.clone()).map_err(|e| Skip::BadResponse(e.to_string()))?;
        let hash = v
            .get("observation_hash")
            .and_then(|h| h.as_str())
            .unwrap_or("")
            .to_string();
        let latency = v.get("latency_ms").and_then(|l| l.as_u64()).unwrap_or(0);
        let read = interpret(&parsed, &hash, latency, ReadSource::Jev)
            .map_err(|e| Skip::BadResponse(e.short()))?;
        let answers = parsed.answers;
        // The f32 tick snapped to a clean f64 so moves in ticks land on exact values.
        let tick = (tick_size(inst) as f64 * 1e8).round() / 1e8;
        let unit_ticks = if ranges.is_empty() {
            1.0
        } else {
            ((stats::mean(&ranges) / tick).max(1.0) * 1e6).round() / 1e6
        };
        let (obs_ms, clock) = observation_time(ts, latency as i64, state_min);
        Ok(Read {
            ts,
            obs_ms,
            clock,
            inst,
            price,
            tick,
            unit_ticks,
            spread_ticks,
            read,
            level,
            trigger,
            touch,
            answers,
        })
    }

    /// Relay stamp minus latency when the state's TIME agrees with it; otherwise the state's
    /// minute on the relay's day (a replayed inference).
    pub fn observation_time(
        ts: i64,
        latency_ms: i64,
        state_min: Option<i64>,
    ) -> (i64, &'static str) {
        let relay_ms = ts - latency_ms.max(0);
        let Some(m) = state_min else {
            return (relay_ms, "relay");
        };
        let relay_min = relay_ms.div_euclid(60_000).rem_euclid(1440);
        let diff = (m - relay_min)
            .rem_euclid(1440)
            .min((relay_min - m).rem_euclid(1440));
        if diff <= 2 {
            return (relay_ms, "relay");
        }
        let day_start = relay_ms.div_euclid(86_400_000) * 86_400_000;
        let mut obs = day_start + m * 60_000;
        if obs > relay_ms + 120_000 {
            obs -= 86_400_000;
        }
        (obs, "state")
    }

    pub fn load_reads(path: &PathBuf) -> Result<(Vec<Read>, BTreeMap<String, usize>), String> {
        let text =
            fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let mut reads = Vec::new();
        let mut skipped: BTreeMap<String, usize> = BTreeMap::new();
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            match parse_line(line) {
                Ok(r) => reads.push(r),
                Err(s) => *skipped.entry(s.label()).or_default() += 1,
            }
        }
        Ok((reads, skipped))
    }

    // -----------------------------------------------------------------------
    // Ticks
    // -----------------------------------------------------------------------

    /// Bid prints, sorted by time.
    #[derive(Default, Clone)]
    pub struct Series {
        pub t: Vec<i64>,
        pub px: Vec<f64>,
    }

    impl Series {
        pub fn from_unsorted(mut v: Vec<(i64, f64)>) -> Self {
            v.sort_by_key(|x| x.0);
            Series {
                t: v.iter().map(|x| x.0).collect(),
                px: v.iter().map(|x| x.1).collect(),
            }
        }
        pub fn len(&self) -> usize {
            self.t.len()
        }
        /// Last print at or before `t`.
        pub fn asof(&self, t: i64) -> Option<(i64, f64)> {
            let i = self.t.partition_point(|x| *x <= t);
            (i > 0).then(|| (self.t[i - 1], self.px[i - 1]))
        }
        /// The tape was alive past `t`: a print exists at or after it.
        pub fn covers(&self, t: i64) -> bool {
            self.t.last().map(|last| *last >= t).unwrap_or(false)
        }
        /// First print in [from, to) within `tol` of `price`.
        pub fn first_print_of(&self, from: i64, to: i64, price: f64, tol: f64) -> Option<i64> {
            let a = self.t.partition_point(|x| *x < from);
            let b = self.t.partition_point(|x| *x < to);
            (a..b)
                .find(|i| (self.px[*i] - price).abs() <= tol)
                .map(|i| self.t[i])
        }
    }

    struct Influx {
        url: String,
        token: String,
        org: String,
        bucket: String,
    }

    /// Read one KEY=value from the repo-root `.env` (or `OPENCRY_ENV_FILE`) without printing anything.
    fn dotenv_value(key: &str) -> Option<String> {
        let path = std::env::var("OPENCRY_ENV_FILE")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.env"));
        let text = fs::read_to_string(path).ok()?;
        text.lines().find_map(|l| {
            l.trim()
                .strip_prefix(&format!("{key}="))
                .map(|v| v.trim().trim_matches(|c| c == '"' || c == '\'').to_string())
        })
    }

    fn influx_config() -> Result<Influx, String> {
        let get = |a: &str, b: &str| {
            std::env::var(a)
                .ok()
                .filter(|v| !v.is_empty())
                .or_else(|| dotenv_value(b))
        };
        Ok(Influx {
            url: get("INFLUX_URL", "INFLUXDB_URL")
                .unwrap_or_else(|| "http://localhost:8086".into()),
            token: get("INFLUX_TOKEN", "INFLUXDB_TOKEN2").ok_or(
                "no InfluxDB token: set INFLUX_TOKEN or INFLUXDB_TOKEN2 in .env (OPENCRY_ENV_FILE)",
            )?,
            org: get("INFLUX_ORG", "INFLUXDB_ORG").ok_or(
                "no InfluxDB org: set INFLUX_ORG or INFLUXDB_ORG in .env (OPENCRY_ENV_FILE)",
            )?,
            bucket: std::env::var("INFLUX_TICK_BUCKET").unwrap_or_else(|_| "tick_data".into()),
        })
    }

    fn flux(cfg: &Influx, query: &str) -> Result<String, String> {
        let mut req = ehttp::Request::post(
            format!("{}/api/v2/query?org={}", cfg.url, cfg.org),
            query.as_bytes().to_vec(),
        );
        req.headers = ehttp::Headers::new(&[
            ("Authorization", &format!("Token {}", cfg.token)),
            ("Content-Type", "application/vnd.flux"),
            ("Accept", "application/csv"),
        ]);
        let res = ehttp::fetch_blocking(&req).map_err(|e| format!("influx: {e}"))?;
        if !res.ok {
            return Err(format!(
                "influx HTTP {}: {}",
                res.status,
                res.text()
                    .unwrap_or("")
                    .chars()
                    .take(200)
                    .collect::<String>()
            ));
        }
        Ok(res.text().unwrap_or("").to_string())
    }

    /// Influx CSV (one or more tables, each with its own header row) into rows keyed by column.
    pub fn parse_csv(text: &str) -> Vec<HashMap<String, String>> {
        let mut rows = Vec::new();
        let mut header: Vec<String> = Vec::new();
        for line in text.lines() {
            let line = line.trim_end_matches('\r');
            if line.is_empty() {
                header.clear();
                continue;
            }
            if line.starts_with('#') {
                continue;
            }
            let cells: Vec<&str> = line.split(',').collect();
            if header.is_empty() || cells.get(1) == Some(&"result") {
                header = cells.iter().map(|c| c.to_string()).collect();
                continue;
            }
            let row: HashMap<String, String> = header
                .iter()
                .cloned()
                .zip(cells.iter().map(|c| c.to_string()))
                .collect();
            rows.push(row);
        }
        rows
    }

    /// RFC3339 with any fraction length (Influx writes nanoseconds) to Unix ms.
    pub fn time_ms(s: &str) -> Option<i64> {
        let s = s.trim_end_matches('Z');
        let (whole, frac) = match s.split_once('.') {
            Some((w, f)) => (w, f),
            None => (s, ""),
        };
        let mut frac3: String = frac.chars().take(3).collect();
        while frac3.len() < 3 {
            frac3.push('0');
        }
        parse_iso_ms(&format!("{whole}.{frac3}Z"))
    }

    fn load_influx(
        cfg: &Influx,
        inst: Instrument,
        from_ms: i64,
        to_ms: i64,
    ) -> Result<Series, String> {
        let sym = tape_symbol(inst);
        let q = format!(
            r#"from(bucket:"{}") |> range(start: time(v: {}), stop: time(v: {})) |> filter(fn:(r) => r._measurement == "tick" and r.symbol == "{sym}" and r._field == "bid") |> keep(columns:["_time","_value"]) |> sort(columns:["_time"])"#,
            cfg.bucket,
            from_ms * 1_000_000,
            to_ms * 1_000_000
        );
        let rows = parse_csv(&flux(cfg, &q)?);
        let v: Vec<(i64, f64)> = rows
            .iter()
            .filter_map(|r| {
                Some((
                    r.get("_time").and_then(|v| time_ms(v))?,
                    r.get("_value").and_then(|v| v.parse::<f64>().ok())?,
                ))
            })
            .collect();
        Ok(Series::from_unsorted(v))
    }

    /// The tape's 1m bars as prints: the open at the bar's start, the close at its end.
    fn load_tape(inst: Instrument, from_ms: i64) -> Result<Series, String> {
        let hours = ((wall_clock_ms() - from_ms) as f64 / 3_600_000.0)
            .ceil()
            .max(1.0) as i64
            + 1;
        let range = if hours <= 48 {
            format!("-{hours}h")
        } else {
            format!("-{}d", hours / 24 + 1)
        };
        let url = format!(
            "{}/bars?symbol={}&timeframe=1m&range={range}",
            tape_url(),
            tape_symbol(inst)
        );
        let res =
            ehttp::fetch_blocking(&ehttp::Request::get(&url)).map_err(|e| format!("tape: {e}"))?;
        if !res.ok {
            return Err(format!("tape HTTP {} for {url}", res.status));
        }
        let v: serde_json::Value =
            serde_json::from_slice(&res.bytes).map_err(|e| format!("tape: {e}"))?;
        let mut out = Vec::new();
        if let Some(rows) = v.get("data").and_then(|d| d.as_array()) {
            for r in rows {
                let (Some(t), Some(o), Some(c)) = (
                    r.get("timestamp").and_then(|x| x.as_i64()),
                    r.get("open").and_then(|x| x.as_f64()),
                    r.get("close").and_then(|x| x.as_f64()),
                ) else {
                    continue;
                };
                out.push((t, o));
                out.push((t + 59_999, c));
            }
        }
        Ok(Series::from_unsorted(out))
    }

    // -----------------------------------------------------------------------
    // Forward moves
    // -----------------------------------------------------------------------

    #[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
    pub struct Forward {
        pub px: f64,
        pub move_ticks: f64,
        pub move_units: f64,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
    #[serde(rename_all = "lowercase")]
    pub enum Unresolved {
        /// No print at or after the target time yet: the window has not closed.
        Pending,
        /// No print within two minutes before the observation: the tape was not there.
        Gap,
    }

    /// Price at `obs + h`: the last print at or before it, provided the tape ran past the
    /// target and was alive at the observation.
    pub fn forward(series: &Series, read: &Read, horizon_min: u32) -> Result<Forward, Unresolved> {
        let target = read.obs_ms + horizon_min as i64 * 60_000;
        if !series.covers(target) {
            return Err(Unresolved::Pending);
        }
        let (t, px) = series.asof(target).ok_or(Unresolved::Gap)?;
        if t < read.obs_ms - 120_000 {
            return Err(Unresolved::Gap);
        }
        let move_ticks = ((px - read.price) / read.tick * 1e6).round() / 1e6;
        Ok(Forward {
            px,
            move_ticks,
            move_units: move_ticks / read.unit_ticks,
        })
    }

    // -----------------------------------------------------------------------
    // The crew's questions
    // -----------------------------------------------------------------------

    /// What price did at the touched level over the horizon after a touch read: the same
    /// four outcomes the reaction prior counts, on the tick path.
    pub fn touch_outcome(series: &Series, r: &Read, horizon_min: u32) -> Option<ReactionOutcome> {
        let t = r.touch.as_ref()?;
        let kind = LevelKind::from_tag(&t.kind)?;
        let target = r.obs_ms + horizon_min as i64 * 60_000;
        if !series.covers(target) {
            return None;
        }
        let a = series.t.partition_point(|x| *x <= r.obs_ms);
        let b = series.t.partition_point(|x| *x <= target);
        let path: Vec<Step> = series.px[a..b]
            .iter()
            .map(|p| Step::tick(*p as f32))
            .collect();
        let touch = Touch {
            kind,
            lo: t.lo as f32,
            hi: t.hi as f32,
            side: if t.from_above {
                TouchSide::FromAbove
            } else {
                TouchSide::FromBelow
            },
            at_ms: r.obs_ms,
            price: r.price as f32,
        };
        Some(classify(&touch, &path, r.tick as f32, r.unit_ticks as f32))
    }

    /// Per question: what Jev answered and what price did; per trader on that question: how
    /// often the side its rule implied paid.
    pub fn question_report(
        scored: &[Scored],
        series: &HashMap<Instrument, Series>,
        horizons: &[u32],
        min_n: usize,
    ) -> String {
        let mut out = String::new();
        let mut w = |s: String| {
            out.push_str(&s);
            out.push('\n');
        };
        w(String::new());
        w("QUESTIONS: the crew's own asks (desk reads carry them; per answer bucket the forward move in units, and for touch reads the level's outcome on the tick path)".into());
        for ask in CREW_ASKS.iter() {
            let asked: Vec<&Scored> = scored
                .iter()
                .filter(|s| s.read.answers.contains_key(ask.key))
                .collect();
            w(String::new());
            w(format!(
                "== {}{}: asked on {} reads   \"{}\"",
                ask.key,
                if ask.touch_only {
                    " (touch reads only)"
                } else {
                    ""
                },
                asked.len(),
                ask.instructions
            ));
            if asked.is_empty() {
                continue;
            }
            for (hi, h) in horizons.iter().enumerate() {
                let mut buckets: BTreeMap<String, Vec<(f64, Option<ReactionOutcome>)>> =
                    BTreeMap::new();
                for s in &asked {
                    if ask.touch_only && s.read.touch.is_none() {
                        continue;
                    }
                    let Ok(f) = &s.fwd[hi] else { continue };
                    let o = if ask.touch_only {
                        series
                            .get(&s.read.inst)
                            .and_then(|ser| touch_outcome(ser, &s.read, *h))
                    } else {
                        None
                    };
                    buckets
                        .entry(JevAsk::bucket(&s.read.answers[ask.key]))
                        .or_default()
                        .push((f.move_units, o));
                }
                for (b, v) in &buckets {
                    let mean = stats::mean(&v.iter().map(|x| x.0).collect::<Vec<_>>());
                    let up = v.iter().filter(|x| x.0 > 0.0).count();
                    let mut line = format!(
                        "  {:>3}m {:<18} n {:>5}  mean move {:+.2} u  up {}",
                        h,
                        b,
                        v.len(),
                        mean,
                        pct(up as f64 / v.len().max(1) as f64)
                    );
                    if ask.touch_only {
                        let count =
                            |o: ReactionOutcome| v.iter().filter(|x| x.1 == Some(o)).count();
                        let known = v.iter().filter(|x| x.1.is_some()).count().max(1);
                        line.push_str(&format!(
                            "   at the level: reject {} break {} sweep {} stall {}",
                            pct(count(ReactionOutcome::Reject) as f64 / known as f64),
                            pct(count(ReactionOutcome::Break) as f64 / known as f64),
                            pct(count(ReactionOutcome::Sweep) as f64 / known as f64),
                            pct(count(ReactionOutcome::Stall) as f64 / known as f64)
                        ));
                    }
                    w(line);
                }
            }
            for style in JEV_CREW
                .iter()
                .filter(|s| s.ask.map(|a| a.question.key == ask.key).unwrap_or(false))
            {
                let a = style.ask.unwrap();
                w(format!(
                    "  -- {} ({}): {}",
                    style.name,
                    style.label,
                    a.rule.text()
                ));
                for (hi, h) in horizons.iter().enumerate() {
                    let (mut n, mut hits, mut edge) = (0usize, 0usize, Vec::new());
                    for s in &asked {
                        let Ok(f) = &s.fwd[hi] else { continue };
                        let trig =
                            s.read.touch.as_ref().map(|t| {
                                t.as_trigger(s.read.trigger.as_deref().unwrap_or("touch"))
                            });
                        let Ok(side) = a.rule.side(
                            ask.key,
                            &s.read.answers[ask.key],
                            trig.as_ref(),
                            s.read.read.direction,
                        ) else {
                            continue;
                        };
                        n += 1;
                        let sign = if side == AskSide::Buy { 1.0 } else { -1.0 };
                        if f.move_ticks * sign > 0.0 {
                            hits += 1;
                        }
                        edge.push(f.move_units * sign);
                    }
                    if n == 0 {
                        w(format!("     {:>3}m no signals", h));
                        continue;
                    }
                    let (lo, hi_) = stats::wilson(hits, n);
                    let p = if n >= min_n {
                        format!("{:.3}", stats::binom_p_two_sided(hits, n))
                    } else {
                        "n too small".into()
                    };
                    w(format!(
                        "     {:>3}m signals {:>5}  hits {:>5} ({})  wilson {}-{}  p vs .5 {}  mean move with the call {:+.2} u",
                        h,
                        n,
                        hits,
                        pct(hits as f64 / n as f64),
                        pct(lo),
                        pct(hi_),
                        p,
                        stats::mean(&edge)
                    ));
                }
            }
        }
        out
    }

    // -----------------------------------------------------------------------
    // Aggregation
    // -----------------------------------------------------------------------

    pub struct Scored {
        pub read: Read,
        /// Indexed like `horizons`.
        pub fwd: Vec<Result<Forward, Unresolved>>,
    }

    #[derive(Debug, Clone, serde::Serialize)]
    pub struct DirConv {
        pub direction: Direction,
        pub conviction: u8,
        pub n: usize,
        pub mean_move_units: f64,
        pub mean_move_ticks: f64,
        pub hit_rate: f64,
    }

    #[derive(Debug, Clone, serde::Serialize)]
    pub struct LevelCall {
        pub reaction: String,
        pub n: usize,
        pub away: usize,
        pub through: usize,
        pub held: usize,
    }

    #[derive(Debug, Clone, serde::Serialize)]
    pub struct HorizonStats {
        pub horizon_min: u32,
        pub resolved: usize,
        pub pending: usize,
        pub gap: usize,
        pub n_directional: usize,
        /// Distinct (symbol, horizon-aligned window) buckets among the up/down calls: reads a
        /// minute apart share most of their forward window, so this is the honest sample size.
        pub independent_windows: usize,
        pub hits: usize,
        pub hit_rate: f64,
        pub wilson_lo: f64,
        pub wilson_hi: f64,
        pub reliable: bool,
        pub binomial_p: Option<f64>,
        pub ties_zero_move: usize,
        pub n_flat_calls: usize,
        pub flat_hits: usize,
        pub brier: Option<f64>,
        pub brier_uniform: f64,
        pub brier_base_rate: Option<f64>,
        pub base_rate: [f64; 3],
        pub mean_abs_move_units: f64,
        pub by_direction_conviction: Vec<DirConv>,
        pub level_calls: Vec<LevelCall>,
        pub calibration: Vec<stats::CalibrationBucket>,
        pub permutation: Option<stats::Permutation>,
    }

    pub fn score_horizon(
        rows: &[&Scored],
        hi: usize,
        horizon_min: u32,
        min_n: usize,
        n_perm: usize,
        seed: u64,
    ) -> HorizonStats {
        let (mut pending, mut gap) = (0, 0);
        let mut resolved: Vec<(&Read, Forward)> = Vec::new();
        for s in rows {
            match s.fwd[hi] {
                Ok(f) => resolved.push((&s.read, f)),
                Err(Unresolved::Pending) => pending += 1,
                Err(Unresolved::Gap) => gap += 1,
            }
        }
        let mut hits = 0;
        let mut n_dir = 0;
        let mut ties = 0;
        let (mut n_flat, mut flat_hits) = (0, 0);
        let mut briers = Vec::new();
        let mut class_count = [0usize; 3];
        let mut calls = Vec::new();
        let mut signs = Vec::new();
        let mut windows: std::collections::HashSet<(Instrument, i64)> =
            std::collections::HashSet::new();
        let mut cal: Vec<(f64, bool)> = Vec::new();
        let mut dc: BTreeMap<(u8, u8), Vec<(f64, f64, bool)>> = BTreeMap::new();
        let mut lv: BTreeMap<String, (usize, usize, usize, usize)> = BTreeMap::new();
        let mut abs_units = Vec::new();
        for (r, f) in &resolved {
            let out = stats::outcome(f.move_units);
            class_count[out.index()] += 1;
            let hit = stats::hit(r.read.direction, f.move_ticks, f.move_units);
            abs_units.push(f.move_units.abs());
            match r.read.direction {
                Direction::Flat => {
                    n_flat += 1;
                    flat_hits += hit as usize;
                }
                d => {
                    n_dir += 1;
                    hits += hit as usize;
                    if f.move_ticks == 0.0 {
                        ties += 1;
                    }
                    calls.push(d);
                    windows.insert((r.inst, r.obs_ms.div_euclid(horizon_min as i64 * 60_000)));
                    signs.push(if f.move_ticks > 0.0 {
                        1i8
                    } else if f.move_ticks < 0.0 {
                        -1
                    } else {
                        0
                    });
                }
            }
            let p = [
                r.read.p_up as f64,
                r.read.p_down as f64,
                r.read.p_flat as f64,
            ];
            if p.iter().sum::<f64>() > 0.0 {
                briers.push(stats::brier3(p, out));
                cal.push((p[0], out == stats::Outcome::Up));
                cal.push((p[1], out == stats::Outcome::Down));
                cal.push((p[2], out == stats::Outcome::Flat));
            }
            let dkey = match r.read.direction {
                Direction::Up => 0,
                Direction::Down => 1,
                Direction::Flat => 2,
            };
            dc.entry((dkey, stats::conviction_bucket(r.read.conviction)))
                .or_default()
                .push((f.move_units, f.move_ticks, hit));
            if r.at_level()
                && let Some(side) = r.level_side()
            {
                let l = r.level.as_ref().unwrap();
                let away = f.move_units * side <= -stats::FLAT_BAND_UNITS;
                let through = if side > 0.0 { f.px > l.hi } else { f.px < l.lo };
                let key = match r.trigger.as_deref() {
                    Some(t) => format!("{t} {}", r.read.reaction),
                    None => r.read.reaction.clone(),
                };
                let e = lv.entry(key).or_default();
                e.0 += 1;
                if through {
                    e.2 += 1;
                } else if away {
                    e.1 += 1;
                } else {
                    e.3 += 1;
                }
            }
        }
        let n_res = resolved.len();
        let base_rate = if n_res > 0 {
            [
                class_count[0] as f64 / n_res as f64,
                class_count[1] as f64 / n_res as f64,
                class_count[2] as f64 / n_res as f64,
            ]
        } else {
            [0.0; 3]
        };
        let brier_base_rate = (n_res > 0 && !briers.is_empty()).then(|| {
            stats::mean(
                &resolved
                    .iter()
                    .map(|(_, f)| stats::brier3(base_rate, stats::outcome(f.move_units)))
                    .collect::<Vec<_>>(),
            )
        });
        let (wl, wh) = stats::wilson(hits, n_dir);
        let reliable = n_dir >= min_n && windows.len() >= min_n;
        HorizonStats {
            horizon_min,
            resolved: n_res,
            pending,
            gap,
            n_directional: n_dir,
            independent_windows: windows.len(),
            hits,
            hit_rate: if n_dir > 0 {
                hits as f64 / n_dir as f64
            } else {
                0.0
            },
            wilson_lo: wl,
            wilson_hi: wh,
            reliable,
            binomial_p: (n_dir > 0).then(|| stats::binom_p_two_sided(hits, n_dir)),
            ties_zero_move: ties,
            n_flat_calls: n_flat,
            flat_hits,
            brier: (!briers.is_empty()).then(|| stats::mean(&briers)),
            brier_uniform: 2.0 / 3.0,
            brier_base_rate,
            base_rate,
            mean_abs_move_units: stats::mean(&abs_units),
            by_direction_conviction: dc
                .into_iter()
                .map(|((d, c), v)| DirConv {
                    direction: [Direction::Up, Direction::Down, Direction::Flat][d as usize],
                    conviction: c,
                    n: v.len(),
                    mean_move_units: stats::mean(&v.iter().map(|x| x.0).collect::<Vec<_>>()),
                    mean_move_ticks: stats::mean(&v.iter().map(|x| x.1).collect::<Vec<_>>()),
                    hit_rate: v.iter().filter(|x| x.2).count() as f64 / v.len() as f64,
                })
                .collect(),
            level_calls: lv
                .into_iter()
                .map(|(reaction, (n, away, through, held))| LevelCall {
                    reaction,
                    n,
                    away,
                    through,
                    held,
                })
                .collect(),
            calibration: stats::calibration(&cal),
            permutation: stats::permutation_control(&calls, &signs, n_perm, seed),
        }
    }

    // -----------------------------------------------------------------------
    // Arguments and the run
    // -----------------------------------------------------------------------

    struct Args {
        from_ms: Option<i64>,
        to_ms: Option<i64>,
        symbols: Vec<Instrument>,
        horizons: Vec<u32>,
        inferences: PathBuf,
        out: Option<PathBuf>,
        source: Source,
        permutations: usize,
        seed: u64,
        min_n: usize,
        calibration_horizon: u32,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Source {
        Influx,
        Tape,
    }

    fn arg(name: &str) -> Option<String> {
        let args: Vec<String> = std::env::args().collect();
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1).cloned())
    }

    fn parse_duration_secs(s: &str) -> Result<i64, String> {
        if s.len() < 2 {
            return Err(format!("bad duration {s}"));
        }
        let (num, unit) = s.split_at(s.len() - 1);
        let n: i64 = num.parse().map_err(|_| format!("bad duration {s}"))?;
        Ok(match unit {
            "s" => n,
            "m" => n * 60,
            "h" => n * 3600,
            "d" => n * 86_400,
            _ => return Err(format!("bad duration unit in {s} (s, m, h, d)")),
        })
    }

    fn parse_args() -> Result<Args, String> {
        let now = wall_clock_ms();
        let (from_ms, to_ms) = if let Some(last) = arg("--last") {
            (Some(now - parse_duration_secs(&last)? * 1000), Some(now))
        } else {
            let from = match arg("--from") {
                Some(f) => Some(parse_iso_ms(&f).ok_or(format!("bad --from {f}"))?),
                None => None,
            };
            let to = match arg("--to") {
                Some(t) if t == "now" => Some(now),
                Some(t) => Some(parse_iso_ms(&t).ok_or(format!("bad --to {t}"))?),
                None => None,
            };
            (from, to)
        };
        if let (Some(a), Some(b)) = (from_ms, to_ms)
            && b <= a
        {
            return Err("--to must be after --from".into());
        }
        let symbols = match arg("--symbols").as_deref() {
            None | Some("all") => Instrument::ALL.to_vec(),
            Some(list) => list
                .split(',')
                .map(|s| {
                    let want = s.trim().to_uppercase().replace('/', "");
                    Instrument::ALL
                        .iter()
                        .copied()
                        .find(|i| tape_symbol(*i) == want)
                        .ok_or(format!("unknown symbol {s}"))
                })
                .collect::<Result<Vec<_>, _>>()?,
        };
        let mut horizons: Vec<u32> = match arg("--horizons") {
            None => vec![1, 5, 15, 60],
            Some(list) => list
                .split(',')
                .map(|h| {
                    h.trim()
                        .parse::<u32>()
                        .map_err(|_| format!("bad horizon {h}"))
                })
                .collect::<Result<_, _>>()?,
        };
        horizons.retain(|h| *h > 0);
        horizons.sort_unstable();
        horizons.dedup();
        if horizons.is_empty() {
            return Err("--horizons needs at least one positive minute count".into());
        }
        let source = match arg("--source").as_deref().unwrap_or("influx") {
            "influx" => Source::Influx,
            "tape" => Source::Tape,
            other => return Err(format!("--source {other}: influx | tape")),
        };
        let calibration_horizon = arg("--calibration-horizon")
            .and_then(|v| v.parse().ok())
            .unwrap_or(5);
        let calibration_horizon = *horizons
            .iter()
            .min_by_key(|h| (**h as i64 - calibration_horizon as i64).abs())
            .unwrap();
        Ok(Args {
            from_ms,
            to_ms,
            symbols,
            horizons,
            inferences: arg("--inferences")
                .map(PathBuf::from)
                .unwrap_or_else(default_inferences),
            out: arg("--out").map(PathBuf::from),
            source,
            permutations: arg("--permutations")
                .and_then(|v| v.parse().ok())
                .unwrap_or(1000),
            seed: arg("--seed").and_then(|v| v.parse().ok()).unwrap_or(7),
            min_n: arg("--min-n").and_then(|v| v.parse().ok()).unwrap_or(30),
            calibration_horizon,
        })
    }

    /// This checkout's relay log, else the main checkout's (a worktree has no relay running).
    fn default_inferences() -> PathBuf {
        let local =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../jev_relay/log/inferences.jsonl");
        if local.exists() {
            return local;
        }
        std::env::var("OPENCRY_INFERENCES")
            .map(PathBuf::from)
            .unwrap_or(local)
    }

    pub fn fmt_ms(ms: i64) -> String {
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
        format!(
            "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
            day / 3600,
            (day % 3600) / 60,
            day % 60
        )
    }

    fn pct(x: f64) -> String {
        format!("{:.0}%", x * 100.0)
    }

    fn p_text(h: &HorizonStats, min_n: usize) -> String {
        if !h.reliable {
            return if h.n_directional < min_n {
                format!("n too small (<{min_n})")
            } else {
                format!("windows too few (<{min_n})")
            };
        }
        format!("{:.3}", h.binomial_p.unwrap_or(1.0))
    }

    pub fn run() -> Result<(), String> {
        let args = parse_args()?;
        let started = Instant::now();
        let (all_reads, skipped) = load_reads(&args.inferences)?;
        let total_lines = all_reads.len() + skipped.values().sum::<usize>();
        let mut reads: Vec<Read> = all_reads
            .into_iter()
            .filter(|r| args.symbols.contains(&r.inst))
            .filter(|r| {
                args.from_ms.map(|f| r.obs_ms >= f).unwrap_or(true)
                    && args.to_ms.map(|t| r.obs_ms <= t).unwrap_or(true)
            })
            .collect();
        reads.sort_by_key(|r| r.obs_ms);
        if reads.is_empty() {
            return Err(format!(
                "no scorable reads in {} for that window/symbols ({} lines, skipped: {:?})",
                args.inferences.display(),
                total_lines,
                skipped
            ));
        }
        let max_h = *args.horizons.last().unwrap() as i64;
        let (first, last) = (reads.first().unwrap().obs_ms, reads.last().unwrap().obs_ms);

        // Ticks per instrument that has reads, over the span the forward windows need.
        let mut series: HashMap<Instrument, Series> = HashMap::new();
        let mut tick_counts: Vec<(String, usize)> = Vec::new();
        let cfg = if args.source == Source::Influx {
            Some(influx_config()?)
        } else {
            None
        };
        for inst in Instrument::ALL {
            if !reads.iter().any(|r| r.inst == inst) {
                continue;
            }
            let lo = reads
                .iter()
                .filter(|r| r.inst == inst)
                .map(|r| r.obs_ms)
                .min()
                .unwrap()
                - 180_000;
            let hi = reads
                .iter()
                .filter(|r| r.inst == inst)
                .map(|r| r.obs_ms)
                .max()
                .unwrap()
                + max_h * 60_000
                + 60_000;
            let s = match (&cfg, args.source) {
                (Some(c), Source::Influx) => load_influx(c, inst, lo, hi)?,
                _ => load_tape(inst, lo)?,
            };
            tick_counts.push((tape_symbol(inst), s.len()));
            series.insert(inst, s);
        }

        // Replayed reads: pin the minute to the print that matches the state's price.
        let mut pinned = 0;
        for r in reads.iter_mut().filter(|r| r.clock == "state") {
            if let Some(s) = series.get(&r.inst)
                && let Some(t) =
                    s.first_print_of(r.obs_ms, r.obs_ms + 60_000, r.price, r.tick * 0.5)
            {
                r.obs_ms = t;
                pinned += 1;
            }
        }

        let scored: Vec<Scored> = reads
            .into_iter()
            .map(|read| {
                let s = &series[&read.inst];
                let fwd = args
                    .horizons
                    .iter()
                    .map(|h| forward(s, &read, *h))
                    .collect();
                Scored { read, fwd }
            })
            .collect();

        let all: Vec<&Scored> = scored.iter().collect();
        let overall: Vec<HorizonStats> = args
            .horizons
            .iter()
            .enumerate()
            .map(|(i, h)| score_horizon(&all, i, *h, args.min_n, args.permutations, args.seed))
            .collect();
        let mut per_symbol: BTreeMap<String, Vec<HorizonStats>> = BTreeMap::new();
        for inst in Instrument::ALL {
            let rows: Vec<&Scored> = scored.iter().filter(|s| s.read.inst == inst).collect();
            if rows.is_empty() {
                continue;
            }
            per_symbol.insert(
                tape_symbol(inst),
                args.horizons
                    .iter()
                    .enumerate()
                    .map(|(i, h)| {
                        score_horizon(&rows, i, *h, args.min_n, args.permutations, args.seed)
                    })
                    .collect(),
            );
        }

        // ---- text report ----
        let mut out = String::new();
        let w = |out: &mut String, s: String| {
            out.push_str(&s);
            out.push('\n');
        };
        w(
            &mut out,
            format!(
                "JEV SCORE  reads {} -> {}  ({} scorable of {} log lines; {} replayed reads re-timed from their state, {} pinned to a print)",
                fmt_ms(first),
                fmt_ms(last),
                scored.len(),
                total_lines,
                scored.iter().filter(|s| s.read.clock == "state").count(),
                pinned
            ),
        );
        if !skipped.is_empty() {
            w(
                &mut out,
                format!(
                    "skipped: {}",
                    skipped
                        .iter()
                        .map(|(k, v)| format!("{v} {k}"))
                        .collect::<Vec<_>>()
                        .join("; ")
                ),
            );
        }
        w(
            &mut out,
            format!(
                "ticks ({:?}): {}   horizons {} min   flat band +-{} unit (unit = mean 1m range of the bars shown)",
                args.source,
                tick_counts
                    .iter()
                    .map(|(s, n)| format!("{s} {n}"))
                    .collect::<Vec<_>>()
                    .join(", "),
                args.horizons
                    .iter()
                    .map(|h| h.to_string())
                    .collect::<Vec<_>>()
                    .join(","),
                stats::FLAT_BAND_UNITS
            ),
        );
        let calls: BTreeMap<&str, usize> = scored.iter().fold(BTreeMap::new(), |mut m, s| {
            *m.entry(s.read.read.direction.label()).or_default() += 1;
            m
        });
        w(
            &mut out,
            format!(
                "calls: {}   at a level (<= {} ticks): {}",
                calls
                    .iter()
                    .map(|(k, v)| format!("{k} {v}"))
                    .collect::<Vec<_>>()
                    .join(", "),
                AT_LEVEL_TICKS,
                scored.iter().filter(|s| s.read.at_level()).count()
            ),
        );
        w(&mut out, String::new());
        w(&mut out, "DIRECTION (up/down calls vs sign of the forward move; zero move = miss; windows = distinct symbol x horizon buckets, the honest n)".into());
        w(
            &mut out,
            format!(
                "{:>4} {:>5} {:>7} {:>5} {:>6} {:>13} {:>18} {:>26} {:>7} {:>9} {:>17} {:>8}",
                "min",
                "n",
                "windows",
                "hits",
                "rate",
                "wilson 95%",
                "binomial p vs .5",
                "permutation pctl",
                "flat n",
                "flat hit",
                "brier/unif/base",
                "pending"
            ),
        );
        for h in &overall {
            let perm = match &h.permutation {
                Some(p) if h.reliable => format!(
                    "{} ({}% >= real{})",
                    pct(p.percentile),
                    (p.p_at_least * 100.0).round(),
                    if p.degenerate { ", degenerate" } else { "" }
                ),
                Some(_) => "n too small".into(),
                None => "-".into(),
            };
            w(
                &mut out,
                format!(
                    "{:>4} {:>5} {:>7} {:>5} {:>6} {:>13} {:>18} {:>26} {:>7} {:>9} {:>17} {:>8}",
                    h.horizon_min,
                    h.n_directional,
                    h.independent_windows,
                    h.hits,
                    pct(h.hit_rate),
                    format!("{}..{}", pct(h.wilson_lo), pct(h.wilson_hi)),
                    p_text(h, args.min_n),
                    perm,
                    h.n_flat_calls,
                    if h.n_flat_calls > 0 {
                        pct(h.flat_hits as f64 / h.n_flat_calls as f64)
                    } else {
                        "-".into()
                    },
                    match (h.brier, h.brier_base_rate) {
                        (Some(b), Some(c)) => format!("{b:.3}/{:.3}/{c:.3}", h.brier_uniform),
                        _ => "-".into(),
                    },
                    format!(
                        "{}{}",
                        h.pending,
                        if h.gap > 0 {
                            format!("+{}gap", h.gap)
                        } else {
                            String::new()
                        }
                    )
                ),
            );
        }
        w(
            &mut out,
            format!(
                "realised class base rates (up/down/flat) per horizon: {}",
                overall
                    .iter()
                    .map(|h| format!(
                        "{}m {:.0}/{:.0}/{:.0}",
                        h.horizon_min,
                        h.base_rate[0] * 100.0,
                        h.base_rate[1] * 100.0,
                        h.base_rate[2] * 100.0
                    ))
                    .collect::<Vec<_>>()
                    .join("  ")
            ),
        );
        w(
            &mut out,
            format!(
                "ties (zero move counted as a miss): {}",
                overall
                    .iter()
                    .map(|h| format!("{}m {}", h.horizon_min, h.ties_zero_move))
                    .collect::<Vec<_>>()
                    .join("  ")
            ),
        );

        w(&mut out, String::new());
        w(
            &mut out,
            "PER SYMBOL  rate (n) [wilson]  per horizon".into(),
        );
        for (sym, hs) in &per_symbol {
            w(
                &mut out,
                format!(
                    "{:<8} {}",
                    sym,
                    hs.iter()
                        .map(|h| format!(
                            "{}m {} ({}) [{}..{}]{}",
                            h.horizon_min,
                            pct(h.hit_rate),
                            h.n_directional,
                            pct(h.wilson_lo),
                            pct(h.wilson_hi),
                            if h.reliable {
                                format!(" p={}", p_text(h, args.min_n))
                            } else {
                                String::new()
                            }
                        ))
                        .collect::<Vec<_>>()
                        .join("   ")
                ),
            );
        }

        w(&mut out, String::new());
        w(
            &mut out,
            "MEAN FORWARD MOVE by direction x conviction bucket  (units; n)  per horizon".into(),
        );
        w(
            &mut out,
            format!(
                "{:<10} {}",
                "call",
                args.horizons
                    .iter()
                    .map(|h| format!("{:>26}", format!("{h}m")))
                    .collect::<String>()
            ),
        );
        for d in [Direction::Up, Direction::Down, Direction::Flat] {
            for c in 0u8..=2 {
                let cells: Vec<String> = overall
                    .iter()
                    .map(|h| {
                        match h
                            .by_direction_conviction
                            .iter()
                            .find(|x| x.direction == d && x.conviction == c)
                        {
                            Some(x) => format!(
                                "{:>26}",
                                format!(
                                    "{:+.2} ({}; hit {})",
                                    x.mean_move_units,
                                    x.n,
                                    pct(x.hit_rate)
                                )
                            ),
                            None => format!("{:>26}", "-"),
                        }
                    })
                    .collect();
                if cells.iter().any(|c| !c.trim().eq("-")) {
                    w(
                        &mut out,
                        format!("{:<10} {}", format!("{} c{}", d.label(), c), cells.concat()),
                    );
                }
            }
        }

        w(&mut out, String::new());
        w(&mut out, "LEVEL REACTION calls taken at a level: away / through / held  (through = beyond the far edge; away = >= half a unit the other way)".into());
        w(&mut out, "  rows prefixed touch/follow were asked because price had just touched the level (desk --touch-reads); unprefixed rows are timed reads".into());
        let mut any_level = false;
        for h in &overall {
            for l in &h.level_calls {
                any_level = true;
                w(
                    &mut out,
                    format!(
                        "{:>4}m {:<18} n {:>4}  away {:>4} ({})  through {:>4} ({})  held {:>4}",
                        h.horizon_min,
                        l.reaction,
                        l.n,
                        l.away,
                        pct(l.away as f64 / l.n as f64),
                        l.through,
                        pct(l.through as f64 / l.n as f64),
                        l.held
                    ),
                );
            }
        }
        if !any_level {
            w(&mut out, "  (no resolved reads taken at a level)".into());
        }

        w(&mut out, String::new());
        let ch = overall
            .iter()
            .find(|h| h.horizon_min == args.calibration_horizon)
            .unwrap();
        w(
            &mut out,
            format!(
                "CALIBRATION at {}m  (each read contributes p_up, p_down, p_flat against the realised class)",
                ch.horizon_min
            ),
        );
        w(
            &mut out,
            format!(
                "{:>10} {:>6} {:>10} {:>10}",
                "decile", "n", "predicted", "realised"
            ),
        );
        for b in &ch.calibration {
            if b.n > 0 {
                w(
                    &mut out,
                    format!(
                        "{:>10} {:>6} {:>10} {:>10}",
                        format!("{:.1}-{:.1}", b.lo, b.hi),
                        b.n,
                        pct(b.mean_predicted),
                        pct(b.realised)
                    ),
                );
            }
        }

        w(&mut out, String::new());
        w(
            &mut out,
            format!(
                "CONTROL  direction labels shuffled {} times over the same outcomes",
                args.permutations
            ),
        );
        for h in &overall {
            match &h.permutation {
                Some(p) => w(
                    &mut out,
                    format!(
                        "{:>4}m real {}  null mean {} [{}..{}]  real sits at the {} percentile; {} of shuffles did as well or better{}{}",
                        h.horizon_min,
                        pct(p.real_rate),
                        pct(p.null_mean),
                        pct(p.null_p2_5),
                        pct(p.null_p97_5),
                        pct(p.percentile),
                        pct(p.p_at_least),
                        if p.degenerate {
                            "  (all calls one way: the shuffle cannot move the rate)"
                        } else {
                            ""
                        },
                        if h.reliable {
                            ""
                        } else {
                            "  [n too small to lean on]"
                        }
                    ),
                ),
                None => w(
                    &mut out,
                    format!("{:>4}m no up/down calls resolved", h.horizon_min),
                ),
            }
        }

        w(&mut out, String::new());
        let verdict = {
            let h = ch;
            if !h.reliable {
                format!(
                    "VERDICT at {}m: n too small ({} up/down calls in {} independent windows, need {} of each). Hit rate {} [{}..{}] is reported, not tested.",
                    h.horizon_min,
                    h.n_directional,
                    h.independent_windows,
                    args.min_n,
                    pct(h.hit_rate),
                    pct(h.wilson_lo),
                    pct(h.wilson_hi)
                )
            } else {
                let p = h.binomial_p.unwrap_or(1.0);
                let perm = h.permutation.as_ref().map(|p| p.p_at_least).unwrap_or(1.0);
                let better = h.hit_rate > 0.5 && p < 0.05 && perm < 0.05;
                let worse = h.hit_rate < 0.5 && p < 0.05;
                format!(
                    "VERDICT at {}m over {} up/down calls ({} independent windows): hit rate {} [{}..{}], binomial p {:.3}, permutation {} of shuffles >= real: {}",
                    h.horizon_min,
                    h.n_directional,
                    h.independent_windows,
                    pct(h.hit_rate),
                    pct(h.wilson_lo),
                    pct(h.wilson_hi),
                    p,
                    pct(perm),
                    if better {
                        "better than chance"
                    } else if worse {
                        "worse than chance"
                    } else {
                        "not distinguishable from chance"
                    }
                )
            }
        };
        w(&mut out, verdict.clone());

        // ---- files ----
        let stamp = fmt_ms(wall_clock_ms()).replace([':', '-'], "");
        let dir = args.out.clone().unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("replay_out")
                .join(format!("jev_score_{stamp}"))
        });
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let config = serde_json::json!({
            "inferences": args.inferences.display().to_string(),
            "from": args.from_ms.map(fmt_ms), "to": args.to_ms.map(fmt_ms),
            "first_read": fmt_ms(first), "last_read": fmt_ms(last),
            "symbols": args.symbols.iter().map(|i| tape_symbol(*i)).collect::<Vec<_>>(),
            "horizons_min": args.horizons, "source": format!("{:?}", args.source),
            "permutations": args.permutations, "seed": args.seed, "min_n": args.min_n,
            "flat_band_units": stats::FLAT_BAND_UNITS, "at_level_ticks": AT_LEVEL_TICKS,
            "log_lines": total_lines, "scored": scored.len(), "skipped": skipped,
            "ticks": tick_counts.iter().cloned().collect::<BTreeMap<_, _>>(),
            "elapsed_secs": started.elapsed().as_secs_f32(),
        });
        fs::write(dir.join("summary.json"), serde_json::to_string_pretty(&serde_json::json!({"config": config, "verdict": verdict, "overall": overall, "per_symbol": per_symbol})).unwrap()).map_err(|e| e.to_string())?;

        out.push_str(&question_report(
            &scored,
            &series,
            &args.horizons,
            args.min_n,
        ));

        let mut csv = fs::File::create(dir.join("reads.csv")).map_err(|e| e.to_string())?;
        let mut head = vec![
            "ts",
            "obs_time",
            "clock",
            "symbol",
            "price",
            "tick",
            "unit_ticks",
            "spread_ticks",
            "direction",
            "p_up",
            "p_down",
            "p_flat",
            "confidence",
            "conviction",
            "conviction_bucket",
            "continuation",
            "exhaustion",
            "reaction",
            "reaction_confidence",
            "level_kind",
            "level_lo",
            "level_hi",
            "level_distance_ticks",
            "at_level",
            "trigger",
        ]
        .into_iter()
        .map(String::from)
        .collect::<Vec<_>>();
        for h in &args.horizons {
            for k in ["status", "px", "move_ticks", "move_units", "outcome", "hit"] {
                head.push(format!("{k}_{h}m"));
            }
        }
        head.extend(CREW_ASKS.iter().map(|a| format!("ask_{}", a.key)));
        head.extend(["model", "latency_ms", "observation_hash"].map(String::from));
        writeln!(csv, "{}", head.join(",")).map_err(|e| e.to_string())?;
        for s in &scored {
            let r = &s.read;
            let j = &r.read;
            let mut row = vec![
                fmt_ms(r.ts),
                fmt_ms(r.obs_ms),
                r.clock.to_string(),
                tape_symbol(r.inst),
                format!("{}", r.price),
                format!("{}", r.tick),
                format!("{:.3}", r.unit_ticks),
                r.spread_ticks.map(|x| format!("{x}")).unwrap_or_default(),
                j.direction.label().to_lowercase(),
                format!("{:.3}", j.p_up),
                format!("{:.3}", j.p_down),
                format!("{:.3}", j.p_flat),
                format!("{:.3}", j.direction_confidence),
                format!("{:.3}", j.conviction),
                stats::conviction_bucket(j.conviction).to_string(),
                format!("{:.3}", j.continuation),
                format!("{:.3}", j.exhaustion),
                j.reaction.clone(),
                format!("{:.3}", j.reaction_confidence),
                r.level.as_ref().map(|l| l.kind.clone()).unwrap_or_default(),
                r.level
                    .as_ref()
                    .map(|l| format!("{}", l.lo))
                    .unwrap_or_default(),
                r.level
                    .as_ref()
                    .map(|l| format!("{}", l.hi))
                    .unwrap_or_default(),
                r.level
                    .as_ref()
                    .map(|l| format!("{}", l.distance_ticks))
                    .unwrap_or_default(),
                r.at_level().to_string(),
                r.trigger.clone().unwrap_or_default(),
            ];
            for f in &s.fwd {
                match f {
                    Ok(f) => {
                        row.push("ok".into());
                        row.push(format!("{}", f.px));
                        row.push(format!("{:.2}", f.move_ticks));
                        row.push(format!("{:.3}", f.move_units));
                        row.push(stats::outcome(f.move_units).label().into());
                        row.push(stats::hit(j.direction, f.move_ticks, f.move_units).to_string());
                    }
                    Err(u) => {
                        row.push(format!("{u:?}").to_lowercase());
                        row.extend(std::iter::repeat_n(String::new(), 5));
                    }
                }
            }
            row.extend(
                CREW_ASKS
                    .iter()
                    .map(|a| r.answers.get(a.key).map(JevAsk::bucket).unwrap_or_default()),
            );
            row.push(j.model.clone());
            row.push(j.latency_ms.to_string());
            row.push(j.observation_hash.clone());
            writeln!(csv, "{}", row.join(",")).map_err(|e| e.to_string())?;
        }
        fs::write(dir.join("report.txt"), &out).map_err(|e| e.to_string())?;
        w(
            &mut out,
            format!(
                "written: {}  ({:.1}s)",
                dir.display(),
                started.elapsed().as_secs_f32()
            ),
        );
        print!("{out}");
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Tests: hand-built reads and tick series
    // -----------------------------------------------------------------------

    #[cfg(test)]
    mod tests {
        use super::stats::*;
        use super::*;

        fn line(
            ts: i64,
            time: &str,
            price: &str,
            dir: &str,
            p: (f64, f64, f64),
            conviction: f64,
            reaction: &str,
            levels: &str,
        ) -> String {
            let state = format!(
                "INSTRUMENT: SILVER (XAG/USD), tick 0.010\nTIME: {time} UTC, bars of 1m\nLAST PRICE: {price}\nLAST 3 BARS, oldest first (open high low close | delta = upticks minus downticks | sweep = largest move in ticks):\n 19:00 57.100 57.140 57.100 57.120 | delta +1 | sweep 2\n 19:01 57.120 57.160 57.120 57.150 | delta +9 | sweep 2\n 19:02 57.150 57.170 57.130 57.160 | delta +2 | sweep 3\nNET CHANGE OVER THESE BARS: +6 ticks\nTAPE (last 20 s): 1.5 ticks/s, upticks 43% of 30 ticks, largest single move 1 ticks\nTOUCH: spread 4 ticks, broker ladder sizes bid 50/50 ask 50/50 (best first)\n{levels}"
            );
            let resp = serde_json::json!({"model":"jev-test","answers":{
                "direction":{"type":"choice","choice":dir,"confidence":p.0.max(p.1).max(p.2),"probabilities":{"up":p.0,"down":p.1,"flat":p.2}},
                "conviction":{"type":"score","score":conviction},
                "continuation":{"type":"noul","noul":0.5},
                "exhaustion":{"type":"noul","noul":0.3},
                "level_reaction":{"type":"choice","choice":reaction,"confidence":0.6}}});
            serde_json::json!({"ts": ts, "observation_hash": "h", "model": "jev-latest", "latency_ms": 200, "status": 200,
                "request": {"state": state, "questions": {}, "model": "jev-latest"}, "response": resp})
            .to_string()
        }

        const T0: i64 = 1_792_609_200_000; // 2026-10-21T19:00:00Z
        const NO_LEVELS: &str = "LEVELS: none marked in this window\n";
        const AT_LEVEL: &str = "LEVELS NEAR PRICE (where resting orders probably are; distance in ticks from last price, negative = below):\n SWING HI 57.180..57.190 at +2 ticks\n OB BULL 57.000..57.050 at -11 ticks\n";

        #[test]
        fn a_live_log_line_parses_through_the_library() {
            let r = parse_line(&line(
                T0 + 200,
                "19:00",
                "57.160",
                "up",
                (0.7, 0.1, 0.2),
                1.4,
                "no_level",
                NO_LEVELS,
            ))
            .unwrap();
            assert_eq!(r.inst, Instrument::Silver);
            assert_eq!(r.price, 57.160);
            assert_eq!(r.clock, "relay");
            assert_eq!(r.obs_ms, T0, "relay stamp minus latency");
            assert_eq!(r.read.direction, Direction::Up);
            assert!((r.read.p_up - 0.7).abs() < 1e-6);
            assert!(
                (r.unit_ticks - 4.0).abs() < 1e-6,
                "mean bar range 4/4/4 ticks: {}",
                r.unit_ticks
            );
            assert_eq!(r.spread_ticks, Some(4.0));
            assert!(r.level.is_none() && !r.at_level());
            assert_eq!(conviction_bucket(r.read.conviction), 1);
        }

        #[test]
        fn a_replayed_line_takes_its_minute_from_the_state() {
            // Logged at 19:25:04 but the state says 19:02: a replay. Minute from the state.
            let r = parse_line(&line(
                T0 + 25 * 60_000 + 4_000,
                "19:02",
                "57.160",
                "flat",
                (0.3, 0.3, 0.4),
                0.2,
                "no_level",
                NO_LEVELS,
            ))
            .unwrap();
            assert_eq!(r.clock, "state");
            assert_eq!(r.obs_ms, T0 + 2 * 60_000);
            // Across midnight: state 23:59, relay 00:00:30 next day -> previous day's minute.
            let (t, c) = observation_time(86_400_000 * 20_000 + 30_000, 0, Some(23 * 60 + 59));
            assert_eq!(
                c, "relay",
                "one minute apart across midnight is still the relay clock"
            );
            assert_eq!(t, 86_400_000 * 20_000 + 30_000);
            let (t, c) = observation_time(86_400_000 * 20_000 + 10 * 60_000, 0, Some(23 * 60 + 50));
            assert_eq!(c, "state");
            assert_eq!(t, 86_400_000 * 20_000 - 10 * 60_000);
        }

        #[test]
        fn levels_and_old_lines_are_handled() {
            let r = parse_line(&line(
                T0,
                "19:00",
                "57.160",
                "up",
                (0.6, 0.2, 0.2),
                1.0,
                "rejecting",
                AT_LEVEL,
            ))
            .unwrap();
            let l = r.level.as_ref().unwrap();
            assert_eq!(l.kind, "SWING HI");
            assert_eq!((l.lo, l.hi, l.distance_ticks), (57.18, 57.19, 2.0));
            assert!(r.at_level());
            assert_eq!(r.level_side(), Some(1.0));
            let old = serde_json::json!({"ts": T0, "request": {"state": "XAG/USD 1m closes: 48.10 48.12. Pit: buyers lifting."}, "response": {}}).to_string();
            assert_eq!(parse_line(&old), Err(Skip::OldStateFormat));
            assert_eq!(parse_line("nonsense"), Err(Skip::NotJson));
        }

        #[test]
        fn forward_moves_come_from_the_asof_print_and_know_when_they_are_unresolved() {
            let r = parse_line(&line(
                T0 + 200,
                "19:00",
                "57.160",
                "up",
                (0.6, 0.2, 0.2),
                1.0,
                "no_level",
                NO_LEVELS,
            ))
            .unwrap();
            // Prints every 30 s for 6 minutes, rising one tick each.
            let prints: Vec<(i64, f64)> = (0..13)
                .map(|i| (T0 + i * 30_000, 57.160 + 0.01 * i as f64))
                .collect();
            let s = Series::from_unsorted(prints);
            let f = forward(&s, &r, 1).unwrap();
            assert!((f.px - 57.18).abs() < 1e-6);
            assert!((f.move_ticks - 2.0).abs() < 1e-6);
            assert!((f.move_units - 0.5).abs() < 1e-6);
            assert_eq!(outcome(f.move_units), Outcome::Up);
            assert!(hit(Direction::Up, f.move_ticks, f.move_units));
            assert!(
                !hit(Direction::Flat, f.move_ticks, f.move_units),
                "half a unit is the edge of flat"
            );
            assert_eq!(
                forward(&s, &r, 15),
                Err(Unresolved::Pending),
                "no print at or after 19:15 yet"
            );
            // A tape that only starts long after the read is a gap, not a move.
            let late =
                Series::from_unsorted(vec![(T0 + 20 * 60_000, 57.0), (T0 + 61 * 60_000, 57.0)]);
            assert_eq!(forward(&late, &r, 5), Err(Unresolved::Gap));
            // Price pinning finds the print that matches the state's LAST PRICE.
            assert_eq!(
                s.first_print_of(T0, T0 + 60_000, 57.17, 0.005),
                Some(T0 + 30_000)
            );
            assert_eq!(s.first_print_of(T0, T0 + 60_000, 57.5, 0.005), None);
        }

        #[test]
        fn wilson_and_binomial_match_known_values() {
            let (lo, hi) = wilson(0, 0);
            assert_eq!((lo, hi), (0.0, 1.0));
            let (lo, hi) = wilson(50, 100);
            assert!(
                (lo - 0.4038).abs() < 0.001 && (hi - 0.5962).abs() < 0.001,
                "{lo} {hi}"
            );
            let (lo, hi) = wilson(10, 10);
            assert!((lo - 0.722).abs() < 0.002 && hi > 0.999_999, "{lo} {hi}");
            assert!((binom_p_two_sided(5, 10) - 1.0).abs() < 1e-9);
            assert!((binom_p_two_sided(10, 10) - 2.0 / 1024.0).abs() < 1e-12);
            assert!((binom_p_two_sided(0, 10) - 2.0 / 1024.0).abs() < 1e-12);
            // 60 of 100: two-sided p = 0.0569
            assert!(
                (binom_p_two_sided(60, 100) - 0.0569).abs() < 0.0005,
                "{}",
                binom_p_two_sided(60, 100)
            );
            // 5200 of 10000 (z = 4): tiny, and fast.
            assert!(binom_p_two_sided(5200, 10_000) < 1e-4);
        }

        #[test]
        fn brier_and_calibration_behave() {
            assert!((brier3([1.0, 0.0, 0.0], Outcome::Up)).abs() < 1e-12);
            assert!((brier3([1.0, 0.0, 0.0], Outcome::Down) - 2.0).abs() < 1e-12);
            let third = 1.0 / 3.0;
            assert!((brier3([third; 3], Outcome::Flat) - 2.0 / 3.0).abs() < 1e-12);
            let entries: Vec<(f64, bool)> = (0..100)
                .map(|i| (0.85, i % 10 < 8))
                .chain((0..50).map(|i| (0.12, i % 10 == 0)))
                .collect();
            let c = calibration(&entries);
            assert_eq!(c.len(), 10);
            assert_eq!(c[8].n, 100);
            assert!(
                (c[8].realised - 0.8).abs() < 1e-9 && (c[8].mean_predicted - 0.85).abs() < 1e-9
            );
            assert_eq!(c[1].n, 50);
            assert!((c[1].realised - 0.1).abs() < 1e-9);
            assert_eq!(c[5].n, 0);
            assert_eq!(conviction_bucket(0.49), 0);
            assert_eq!(conviction_bucket(0.5), 1);
            assert_eq!(conviction_bucket(1.76), 2);
            assert_eq!(conviction_bucket(9.0), 2);
        }

        #[test]
        fn permutation_control_places_a_perfect_caller_at_the_top_and_a_coin_flip_in_the_middle() {
            // Perfect: 20 up on +1, 20 down on -1.
            let calls: Vec<Direction> = (0..40)
                .map(|i| {
                    if i % 2 == 0 {
                        Direction::Up
                    } else {
                        Direction::Down
                    }
                })
                .collect();
            let signs: Vec<i8> = (0..40).map(|i| if i % 2 == 0 { 1 } else { -1 }).collect();
            let p = permutation_control(&calls, &signs, 1000, 1).unwrap();
            assert_eq!(p.real_rate, 1.0);
            assert!(p.percentile > 0.99 && p.p_at_least < 0.01, "{p:?}");
            assert!((p.null_mean - 0.5).abs() < 0.05, "{p:?}");
            assert!(!p.degenerate);
            // Labels unrelated to outcomes: the real rate sits inside the null.
            let calls: Vec<Direction> = (0..200)
                .map(|i| {
                    if (i / 7) % 2 == 0 {
                        Direction::Up
                    } else {
                        Direction::Down
                    }
                })
                .collect();
            let signs: Vec<i8> = (0..200)
                .map(|i| if (i / 3) % 2 == 0 { 1 } else { -1 })
                .collect();
            let p = permutation_control(&calls, &signs, 1000, 3).unwrap();
            assert!(p.percentile > 0.02 && p.percentile < 0.98, "{p:?}");
            assert!(
                p.null_p2_5 <= p.real_rate && p.real_rate <= p.null_p97_5,
                "{p:?}"
            );
            // All calls up: shuffling changes nothing.
            let p = permutation_control(
                &[Direction::Up; 10],
                &[1, 1, 1, -1, -1, -1, 1, 1, 0, 1],
                100,
                1,
            )
            .unwrap();
            assert!(p.degenerate);
            assert_eq!(p.null_p2_5, p.real_rate);
            assert!(permutation_control(&[], &[], 10, 1).is_none());
            // Deterministic for a seed.
            let a = permutation_control(&calls, &signs, 200, 11).unwrap();
            let b = permutation_control(&calls, &signs, 200, 11).unwrap();
            assert_eq!(a.percentile, b.percentile);
        }

        #[test]
        fn a_horizon_is_scored_end_to_end_on_hand_built_reads_and_ticks() {
            // Three reads at 19:00, 19:01, 19:02; the tape rises 1 tick / 30 s then collapses after 19:10.
            let mut prints: Vec<(i64, f64)> = (0..21)
                .map(|i| (T0 + i * 30_000, 57.160 + 0.01 * i as f64))
                .collect();
            prints.push((T0 + 11 * 60_000, 56.0));
            prints.push((T0 + 70 * 60_000, 56.0));
            let s = Series::from_unsorted(prints);
            let lines = [
                line(
                    T0 + 200,
                    "19:00",
                    "57.160",
                    "up",
                    (0.8, 0.1, 0.1),
                    1.8,
                    "breaking",
                    AT_LEVEL,
                ),
                line(
                    T0 + 60_200,
                    "19:01",
                    "57.180",
                    "down",
                    (0.2, 0.6, 0.2),
                    0.9,
                    "rejecting",
                    AT_LEVEL,
                ),
                line(
                    T0 + 120_200,
                    "19:02",
                    "57.190",
                    "flat",
                    (0.3, 0.3, 0.4),
                    0.1,
                    "no_level",
                    NO_LEVELS,
                ),
            ];
            let horizons = [1u32, 5, 60];
            let scored: Vec<Scored> = lines
                .iter()
                .map(|l| {
                    let read = parse_line(l).unwrap();
                    let fwd = horizons.iter().map(|h| forward(&s, &read, *h)).collect();
                    Scored { read, fwd }
                })
                .collect();
            let rows: Vec<&Scored> = scored.iter().collect();
            let h1 = score_horizon(&rows, 0, 1, 30, 200, 1);
            assert_eq!((h1.resolved, h1.pending, h1.gap), (3, 0, 0));
            assert_eq!((h1.n_directional, h1.hits), (2, 1), "up paid, down did not");
            assert_eq!(
                (h1.n_flat_calls, h1.flat_hits),
                (1, 0),
                "+3 ticks on a 4-tick unit is not flat"
            );
            assert!(!h1.reliable && h1.binomial_p.is_some());
            assert_eq!(
                h1.independent_windows, 2,
                "two up/down calls a minute apart: two 1m windows"
            );
            assert_eq!(h1.base_rate, [1.0, 0.0, 0.0]);
            let up = h1
                .by_direction_conviction
                .iter()
                .find(|x| x.direction == Direction::Up && x.conviction == 2)
                .unwrap();
            assert!((up.mean_move_units - 0.5).abs() < 1e-6 && up.n == 1);
            // Level calls: price is 57.160 under SWING HI 57.180..57.190 (+2 ticks); after 1 min 57.180 is not beyond 57.190 -> held.
            let breaking = h1
                .level_calls
                .iter()
                .find(|l| l.reaction == "breaking")
                .unwrap();
            assert_eq!(
                (breaking.n, breaking.through, breaking.away, breaking.held),
                (1, 0, 0, 1)
            );
            let h5 = score_horizon(&rows, 1, 5, 30, 200, 1);
            assert_eq!(
                h5.independent_windows, 1,
                "both up/down calls share one 5m window"
            );
            let breaking = h5
                .level_calls
                .iter()
                .find(|l| l.reaction == "breaking")
                .unwrap();
            assert_eq!(
                (breaking.through, breaking.held),
                (1, 0),
                "57.260 after 5 min is through the swing high"
            );
            let rejecting = h5
                .level_calls
                .iter()
                .find(|l| l.reaction == "rejecting")
                .unwrap();
            assert_eq!((rejecting.through, rejecting.away), (1, 0));
            // 60 min: the tape collapsed to 56.0, so up misses, down hits, flat misses; Brier is worst for the confident up call.
            let h60 = score_horizon(&rows, 2, 60, 30, 200, 1);
            assert_eq!((h60.n_directional, h60.hits), (2, 1));
            assert!(
                h60.brier.unwrap() > h60.brier_uniform,
                "confident wrong calls score worse than the uniform forecast"
            );
            assert_eq!(h60.calibration.iter().map(|b| b.n).sum::<usize>(), 9);
            assert!(h60.permutation.is_some());
        }

        #[test]
        fn influx_csv_and_times_parse() {
            let csv = "#group,false,false\n#datatype,string,long\n#default,_result,,\n,result,table,_time,_value\n,_result,0,2026-10-21T16:46:20.134000005Z,57.556\n,_result,0,2026-10-21T16:46:21Z,57.56\n\n";
            let rows = parse_csv(csv);
            assert_eq!(rows.len(), 2);
            assert_eq!(
                time_ms(&rows[0]["_time"]),
                Some(parse_iso_ms("2026-10-21T16:46:20.134Z").unwrap())
            );
            assert_eq!(
                time_ms(&rows[1]["_time"]),
                Some(parse_iso_ms("2026-10-21T16:46:21.000Z").unwrap())
            );
            assert_eq!(fmt_ms(T0), "2026-10-21T19:00:00Z");
        }
    }
}
