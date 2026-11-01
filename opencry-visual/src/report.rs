//! Replay reporting: turns the paper book's closed trades into something a trader can read and
//! trust. The leaderboard ranks on net R; this module asks the harder question, whether a row
//! has *earned* belief: per-trader statistics, breakdowns by symbol and session window,
//! equity curves against the coin-flip control, and the edge-lab discipline (t-stat, seeded
//! bootstrap interval, deflated Sharpe ratio after Bailey & López de Prado 2014) so a lucky
//! afternoon does not read as an edge.
//!
//! Everything here is pure: `Vec<ReportTrade>` in, `Report` out, text and HTML rendered from
//! it. No Bevy, no I/O, so it compiles for wasm and is unit-tested with hand-built trades.
//! `ReportTrade` is an owned mirror of `paper::Trade` (which holds `&'static str`s and only
//! serialises) so a report can be rebuilt from `trades.jsonl` without the book.
//!
//! Conventions: R = profit / initial stop. A trade with R <= 0 is a loss (the book counts a
//! 0 R time stop as a loss too). "Spread R" is the spread paid at entry as a fraction of the
//! stop, summed; it is already inside R (fills are at ask/bid), so it is the part of the
//! before-cost result the spread ate, not an extra deduction.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::paper::{Archetype, Trade};

/// Below this many closed trades no verdict is given, whatever the numbers say.
pub const MIN_TRADES_FOR_VERDICT: usize = 20;
/// Bootstrap draws for the net R interval.
pub const BOOTSTRAP_DRAWS: usize = 2000;
/// Equally spaced samples of the equity curve used for the text sparkline.
const SPARK_COLS: usize = 48;

// ---------------------------------------------------------------------------
// Trades
// ---------------------------------------------------------------------------

/// Owned, deserialisable copy of `paper::Trade` carrying what the report needs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReportTrade {
    pub trader: String,
    pub archetype: String,
    pub symbol: String,
    pub side: String,
    pub opened_ms: i64,
    pub closed_ms: i64,
    pub pnl_r: f32,
    #[serde(default)]
    pub pnl_ticks: f32,
    #[serde(default)]
    pub stop_ticks: f32,
    #[serde(default)]
    pub spread_ticks_in: f32,
    #[serde(default)]
    pub why_in: String,
    #[serde(default)]
    pub why_out: String,
    #[serde(default)]
    pub read_source: Option<String>,
}

impl From<&Trade> for ReportTrade {
    fn from(t: &Trade) -> Self {
        ReportTrade {
            trader: t.trader.to_string(),
            archetype: archetype_key(t.archetype),
            symbol: t.symbol.to_string(),
            side: t.side.to_string(),
            opened_ms: t.opened_ms,
            closed_ms: t.closed_ms,
            pnl_r: t.pnl_r,
            pnl_ticks: t.pnl_ticks,
            stop_ticks: t.stop_ticks,
            spread_ticks_in: t.spread_ticks_in,
            why_in: t.why_in.clone(),
            why_out: t.why_out.clone(),
            read_source: t.read_source.map(|s| format!("{s:?}")),
        }
    }
}

impl ReportTrade {
    pub fn hold_minutes(&self) -> f64 {
        (self.closed_ms - self.opened_ms).max(0) as f64 / 60_000.0
    }

    /// Spread paid at entry in R: spread / stop. A zero stop cannot happen for a real trade;
    /// treat it as one tick so the number stays finite.
    pub fn spread_r(&self) -> f64 {
        self.spread_ticks_in as f64 / (self.stop_ticks.max(1.0) as f64)
    }

    pub fn session(&self) -> Session {
        Session::of_ms(self.opened_ms)
    }
}

/// The string `serde` writes for an archetype (its variant name), which is what `trades.jsonl`
/// carries.
pub fn archetype_key(a: Archetype) -> String {
    serde_json::to_value(a).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_else(|| format!("{a:?}"))
}

/// One trade per line, as `replay` writes them. Blank and unparseable lines are skipped and
/// counted so the caller can say so.
pub fn parse_jsonl(text: &str) -> (Vec<ReportTrade>, usize) {
    let mut out = Vec::new();
    let mut bad = 0;
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<ReportTrade>(line) {
            Ok(t) => out.push(t),
            Err(_) => bad += 1,
        }
    }
    (out, bad)
}

// ---------------------------------------------------------------------------
// Session windows (UTC, by entry time)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Session {
    /// 00:00–07:00
    Asia,
    /// 07:00–12:00
    London,
    /// 12:00–16:30, London and New York both open
    Overlap,
    /// 16:30–21:00
    NewYork,
    /// 21:00–24:00
    Late,
}

impl Session {
    pub const ALL: [Session; 5] = [Session::Asia, Session::London, Session::Overlap, Session::NewYork, Session::Late];

    pub fn of_ms(ms: i64) -> Session {
        let minute = ms.rem_euclid(86_400_000) / 60_000;
        match minute {
            m if m < 7 * 60 => Session::Asia,
            m if m < 12 * 60 => Session::London,
            m if m < 16 * 60 + 30 => Session::Overlap,
            m if m < 21 * 60 => Session::NewYork,
            _ => Session::Late,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Session::Asia => "Asia",
            Session::London => "London",
            Session::Overlap => "Overlap",
            Session::NewYork => "New York",
            Session::Late => "Late",
        }
    }

    pub fn hours(self) -> &'static str {
        match self {
            Session::Asia => "00:00-07:00",
            Session::London => "07:00-12:00",
            Session::Overlap => "12:00-16:30",
            Session::NewYork => "16:30-21:00",
            Session::Late => "21:00-24:00",
        }
    }

    /// Accepts the names a person types on the command line: `london`, `ny`, `newyork`,
    /// `new_york`, `overlap`, `asia`, `late`.
    pub fn parse(s: &str) -> Option<Session> {
        match s.trim().to_ascii_lowercase().replace(['_', '-', ' '], "").as_str() {
            "asia" | "tokyo" => Some(Session::Asia),
            "london" | "ldn" | "eu" => Some(Session::London),
            "overlap" | "lonny" => Some(Session::Overlap),
            "ny" | "newyork" | "us" => Some(Session::NewYork),
            "late" | "close" => Some(Session::Late),
            _ => None,
        }
    }

    /// `london,ny` -> the set, in `ALL` order, or the name that did not parse.
    pub fn parse_list(list: &str) -> Result<Vec<Session>, String> {
        let mut out = Vec::new();
        for part in list.split(',').filter(|p| !p.trim().is_empty()) {
            let s = Session::parse(part).ok_or_else(|| format!("unknown session {part:?} (asia, london, overlap, ny, late)"))?;
            if !out.contains(&s) {
                out.push(s);
            }
        }
        out.sort();
        Ok(out)
    }
}

// ---------------------------------------------------------------------------
// Statistics
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize)]
pub struct TraderStats {
    pub trades: usize,
    pub wins: usize,
    pub losses: usize,
    pub win_rate: f64,
    pub net_r: f64,
    pub mean_r: f64,
    pub median_r: f64,
    /// Sample standard deviation of R (n-1); 0 when fewer than two trades.
    pub std_r: f64,
    pub gross_win_r: f64,
    pub gross_loss_r: f64,
    /// Gross wins / gross losses; `inf` with wins and no losses, 0 with no wins.
    pub profit_factor: f64,
    /// Peak-to-trough fall of cumulative R, trades in exit order.
    pub max_drawdown_r: f64,
    /// Spread paid at entry, summed, in R.
    pub spread_r: f64,
    /// `spread_r / (net_r + spread_r)`: the share of the before-cost result the spread took.
    /// `None` when the before-cost result is not positive (there was nothing to take a share of).
    pub spread_share_of_gross: Option<f64>,
    pub avg_hold_min: f64,
    pub longest_losing_run: usize,
}

pub fn trader_stats(trades: &[ReportTrade]) -> TraderStats {
    if trades.is_empty() {
        return TraderStats::default();
    }
    let mut by_exit: Vec<&ReportTrade> = trades.iter().collect();
    by_exit.sort_by_key(|t| (t.closed_ms, t.opened_ms));
    let rs: Vec<f64> = by_exit.iter().map(|t| t.pnl_r as f64).collect();
    let n = rs.len();
    let net: f64 = rs.iter().sum();
    let mean = net / n as f64;
    let wins = rs.iter().filter(|r| **r > 0.0).count();
    let gross_win: f64 = rs.iter().filter(|r| **r > 0.0).sum();
    let gross_loss: f64 = -rs.iter().filter(|r| **r <= 0.0).sum::<f64>();
    let spread_r: f64 = by_exit.iter().map(|t| t.spread_r()).sum();
    let before_cost = net + spread_r;
    TraderStats {
        trades: n,
        wins,
        losses: n - wins,
        win_rate: wins as f64 / n as f64,
        net_r: net,
        mean_r: mean,
        median_r: median(&rs),
        std_r: sample_sd(&rs),
        gross_win_r: gross_win,
        gross_loss_r: gross_loss,
        profit_factor: if gross_loss > 0.0 {
            gross_win / gross_loss
        } else if gross_win > 0.0 {
            f64::INFINITY
        } else {
            0.0
        },
        max_drawdown_r: max_drawdown(&rs),
        spread_r,
        spread_share_of_gross: if before_cost > 0.0 { Some(spread_r / before_cost) } else { None },
        avg_hold_min: by_exit.iter().map(|t| t.hold_minutes()).sum::<f64>() / n as f64,
        longest_losing_run: longest_run(&rs, |r| r <= 0.0),
    }
}

pub fn median(xs: &[f64]) -> f64 {
    if xs.is_empty() {
        return 0.0;
    }
    let mut v = xs.to_vec();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = v.len();
    if n % 2 == 1 { v[n / 2] } else { (v[n / 2 - 1] + v[n / 2]) / 2.0 }
}

pub fn sample_sd(xs: &[f64]) -> f64 {
    let n = xs.len();
    if n < 2 {
        return 0.0;
    }
    let mean = xs.iter().sum::<f64>() / n as f64;
    (xs.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n - 1) as f64).sqrt()
}

/// Largest peak-to-trough fall of the cumulative sum, starting from a peak of 0.
pub fn max_drawdown(rs: &[f64]) -> f64 {
    let (mut cum, mut peak, mut dd) = (0.0f64, 0.0f64, 0.0f64);
    for r in rs {
        cum += r;
        peak = peak.max(cum);
        dd = dd.max(peak - cum);
    }
    dd
}

pub fn longest_run(rs: &[f64], pred: impl Fn(f64) -> bool) -> usize {
    let (mut best, mut run) = (0, 0);
    for r in rs {
        if pred(*r) {
            run += 1;
            best = best.max(run);
        } else {
            run = 0;
        }
    }
    best
}

/// Student t of the mean against zero: mean / (sd / sqrt(n)). `None` below two trades or
/// with no dispersion.
pub fn t_stat(rs: &[f64]) -> Option<f64> {
    let n = rs.len();
    if n < 2 {
        return None;
    }
    let sd = sample_sd(rs);
    if sd <= 0.0 {
        return None;
    }
    let mean = rs.iter().sum::<f64>() / n as f64;
    Some(mean / (sd / (n as f64).sqrt()))
}

/// Per-trade Sharpe: mean R / sample sd of R. Not annualised; trades are the periods.
pub fn sharpe(rs: &[f64]) -> Option<f64> {
    let sd = sample_sd(rs);
    if rs.len() < 2 || sd <= 0.0 {
        return None;
    }
    Some(rs.iter().sum::<f64>() / rs.len() as f64 / sd)
}

/// Population skewness and (non-excess) kurtosis, as the PSR formula wants them.
pub fn skew_kurt(rs: &[f64]) -> (f64, f64) {
    let n = rs.len() as f64;
    if n < 2.0 {
        return (0.0, 3.0);
    }
    let mean = rs.iter().sum::<f64>() / n;
    let m2 = rs.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / n;
    if m2 <= 0.0 {
        return (0.0, 3.0);
    }
    let m3 = rs.iter().map(|x| (x - mean).powi(3)).sum::<f64>() / n;
    let m4 = rs.iter().map(|x| (x - mean).powi(4)).sum::<f64>() / n;
    (m3 / m2.powf(1.5), m4 / (m2 * m2))
}

// --- normal distribution -----------------------------------------------------

/// erf by Abramowitz & Stegun 7.1.26 (|error| < 1.5e-7).
fn erf(x: f64) -> f64 {
    let sign = if x < 0.0 { -1.0 } else { 1.0 };
    let x = x.abs();
    let t = 1.0 / (1.0 + 0.3275911 * x);
    let poly = t * (0.254829592 + t * (-0.284496736 + t * (1.421413741 + t * (-1.453152027 + t * 1.061405429))));
    sign * (1.0 - poly * (-x * x).exp())
}

pub fn norm_cdf(z: f64) -> f64 {
    0.5 * (1.0 + erf(z / std::f64::consts::SQRT_2))
}

/// Inverse normal CDF: Acklam's rational approximation (rel. error 1.15e-9). Clamps p into
/// (0,1).
pub fn norm_ppf(p: f64) -> f64 {
    let p = p.clamp(1e-12, 1.0 - 1e-12);
    const A: [f64; 6] = [-3.969683028665376e1, 2.209460984245205e2, -2.759285104469687e2, 1.383577518672690e2, -3.066479806614716e1, 2.506628277459239];
    const B: [f64; 5] = [-5.447609879822406e1, 1.615858368580409e2, -1.556989798598866e2, 6.680131188771972e1, -1.328068155288572e1];
    const C: [f64; 6] = [-7.784894002430293e-3, -3.223964580411365e-1, -2.400758277161838, -2.549732539343734, 4.374664141464968, 2.938163982698783];
    const D: [f64; 4] = [7.784695709041462e-3, 3.224671290700398e-1, 2.445134137142996, 3.754408661907416];
    const P_LOW: f64 = 0.02425;
    let x = if p < P_LOW {
        let q = (-2.0 * p.ln()).sqrt();
        (((((C[0] * q + C[1]) * q + C[2]) * q + C[3]) * q + C[4]) * q + C[5]) / ((((D[0] * q + D[1]) * q + D[2]) * q + D[3]) * q + 1.0)
    } else if p <= 1.0 - P_LOW {
        let q = p - 0.5;
        let r = q * q;
        (((((A[0] * r + A[1]) * r + A[2]) * r + A[3]) * r + A[4]) * r + A[5]) * q / (((((B[0] * r + B[1]) * r + B[2]) * r + B[3]) * r + B[4]) * r + 1.0)
    } else {
        let q = (-2.0 * (1.0 - p).ln()).sqrt();
        -(((((C[0] * q + C[1]) * q + C[2]) * q + C[3]) * q + C[4]) * q + C[5]) / ((((D[0] * q + D[1]) * q + D[2]) * q + D[3]) * q + 1.0)
    };
    // No Newton polish: Acklam's 1.15e-9 already beats the 1.5e-7 erf above, so a step
    // against `norm_cdf` could only drag it toward that error.
    x
}

// --- probabilistic / deflated Sharpe ------------------------------------------

/// Probabilistic Sharpe ratio (Bailey & López de Prado 2012): the probability that the true
/// Sharpe exceeds `sr_star`, given the observed per-period `sr` over `n` observations with
/// the returns' skewness and (non-excess) kurtosis:
///
///   PSR(SR*) = Φ( (SR − SR*)·√(n−1) / √(1 − γ3·SR + (γ4 − 1)/4·SR²) )
///
/// Worked example (hand-derived from the formula): SR = 0.1, n = 101, Gaussian returns
/// (γ3 = 0, γ4 = 3): numerator 0.1·10 = 1, denominator √(1 + 0.5·0.01) = √1.005 = 1.0025,
/// z = 0.99751, PSR = Φ(0.99751) = 0.8407.
pub fn psr(sr: f64, n: usize, skew: f64, kurt: f64, sr_star: f64) -> Option<f64> {
    if n < 2 {
        return None;
    }
    let var_term = 1.0 - skew * sr + (kurt - 1.0) / 4.0 * sr * sr;
    if !(var_term > 0.0) {
        return None;
    }
    Some(norm_cdf((sr - sr_star) * ((n - 1) as f64).sqrt() / var_term.sqrt()))
}

/// Expected maximum Sharpe among `n_trials` independent zero-skill trials whose Sharpe
/// estimates have variance `var_sr` (Bailey & López de Prado 2014, eq. for E[max_n SR]):
///
///   E[max SR] ≈ √V · ( (1 − γ)·Z⁻¹(1 − 1/N) + γ·Z⁻¹(1 − 1/(N·e)) ),  γ = 0.5772 (Euler–Mascheroni)
///
/// Worked example: N = 6, V = 1: Z⁻¹(0.8333) = 0.9674, Z⁻¹(1 − 1/(6e)) = Z⁻¹(0.93869) = 1.5439,
/// so 0.4228·0.9674 + 0.5772·1.5439 = 0.4090 + 0.8911 = 1.300. With one trial or no
/// dispersion the hurdle is 0 and the DSR collapses to PSR(0).
pub fn expected_max_sharpe(n_trials: usize, var_sr: f64) -> f64 {
    if n_trials < 2 || !(var_sr > 0.0) {
        return 0.0;
    }
    const EULER_GAMMA: f64 = 0.577_215_664_901_532_9;
    let n = n_trials as f64;
    var_sr.sqrt() * ((1.0 - EULER_GAMMA) * norm_ppf(1.0 - 1.0 / n) + EULER_GAMMA * norm_ppf(1.0 - 1.0 / (n * std::f64::consts::E)))
}

/// Deflated Sharpe ratio: PSR with the hurdle set to the Sharpe one would expect from the best
/// of `n_trials` skill-less strategies. `var_sr` is the cross-sectional variance of the
/// trials' Sharpe estimates; here the trials are the archetypes on the floor and the
/// variance is measured across them. Returns a probability; the edge-lab convention treats
/// ≥ 0.95 as "the edge survived the selection".
pub fn dsr(sr: f64, n: usize, skew: f64, kurt: f64, n_trials: usize, var_sr: f64) -> Option<f64> {
    psr(sr, n, skew, kurt, expected_max_sharpe(n_trials, var_sr))
}

// --- bootstrap ---------------------------------------------------------------

/// SplitMix64: tiny, seedable, good enough for resampling indices. Kept local so the report
/// does not depend on the pit engine's generator or its seeding.
pub struct SplitMix64(u64);

impl SplitMix64 {
    pub fn new(seed: u64) -> Self {
        SplitMix64(seed)
    }
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n.max(1) as u64) as usize
    }
}

fn resample_sum(rs: &[f64], rng: &mut SplitMix64) -> f64 {
    (0..rs.len()).map(|_| rs[rng.below(rs.len())]).sum()
}

fn percentile_pair(mut draws: Vec<f64>, lo: f64, hi: f64) -> (f64, f64) {
    draws.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let last = (draws.len() - 1) as f64;
    (draws[(lo * last).round() as usize], draws[(hi * last).round() as usize])
}

/// 95% percentile bootstrap interval of net R: resample the trades with replacement `draws`
/// times, sum each. Deterministic for a seed. `None` with no trades.
pub fn bootstrap_net_r(rs: &[f64], draws: usize, seed: u64) -> Option<(f64, f64)> {
    if rs.is_empty() || draws == 0 {
        return None;
    }
    let mut rng = SplitMix64::new(seed);
    let sums: Vec<f64> = (0..draws).map(|_| resample_sum(rs, &mut rng)).collect();
    Some(percentile_pair(sums, 0.025, 0.975))
}

/// 95% bootstrap interval of (trader net R − control net R), both resampled independently in
/// each draw, so the control's own luck is in the interval too.
pub fn bootstrap_diff_r(rs: &[f64], control: &[f64], draws: usize, seed: u64) -> Option<(f64, f64)> {
    if rs.is_empty() || control.is_empty() || draws == 0 {
        return None;
    }
    let mut rng = SplitMix64::new(seed ^ 0xD1FF_0000_0000_0001);
    let diffs: Vec<f64> = (0..draws).map(|_| resample_sum(rs, &mut rng) - resample_sum(control, &mut rng)).collect();
    Some(percentile_pair(diffs, 0.025, 0.975))
}

// ---------------------------------------------------------------------------
// Edge and verdict
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Verdict {
    /// The control itself.
    Control,
    /// Fewer than `MIN_TRADES_FOR_VERDICT` trades.
    TooFewTrades,
    /// The whole bootstrap interval of (trader − control) net R is above zero.
    Yes,
    No,
    /// No control trades to compare against.
    NoControl,
}

impl Verdict {
    pub fn label(self) -> &'static str {
        match self {
            Verdict::Control => "(control)",
            Verdict::TooFewTrades => "too few trades",
            Verdict::Yes => "yes",
            Verdict::No => "no",
            Verdict::NoControl => "no control to compare",
        }
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Edge {
    pub t_stat: Option<f64>,
    pub sharpe: Option<f64>,
    pub skew: f64,
    pub kurt: f64,
    pub boot_net_r: Option<(f64, f64)>,
    pub boot_vs_control: Option<(f64, f64)>,
    pub psr0: Option<f64>,
    pub expected_max_sharpe: f64,
    pub dsr: Option<f64>,
    pub verdict: Option<Verdict>,
}

pub fn verdict(is_control: bool, trades: usize, vs_control: Option<(f64, f64)>) -> Verdict {
    if is_control {
        Verdict::Control
    } else if trades < MIN_TRADES_FOR_VERDICT {
        Verdict::TooFewTrades
    } else {
        match vs_control {
            None => Verdict::NoControl,
            Some((lo, _)) if lo > 0.0 => Verdict::Yes,
            Some(_) => Verdict::No,
        }
    }
}

// ---------------------------------------------------------------------------
// Report assembly
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct Breakdown {
    pub key: String,
    pub trades: usize,
    pub win_rate: f64,
    pub net_r: f64,
    pub mean_r: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct TraderReport {
    pub name: String,
    pub archetype: String,
    pub label: String,
    pub is_control: bool,
    pub stats: TraderStats,
    pub edge: Edge,
    pub by_symbol: Vec<Breakdown>,
    pub by_session: Vec<Breakdown>,
    /// (exit ms, cumulative R) in exit order, starting at (window start, 0).
    pub curve: Vec<(i64, f64)>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DayRow {
    pub day: String,
    /// (trades, net R) per trader, in `Report::traders` order.
    pub cells: Vec<(usize, f64)>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub title: String,
    pub from_ms: i64,
    pub to_ms: i64,
    pub n_trials: usize,
    pub seed: u64,
    pub traders: Vec<TraderReport>,
    pub control: Option<usize>,
    pub days: Vec<DayRow>,
    pub total_trades: usize,
    #[serde(skip)]
    pub trades: Vec<ReportTrade>,
}

fn breakdown<K: Ord + Clone>(trades: &[&ReportTrade], key: impl Fn(&ReportTrade) -> K, name: impl Fn(&K) -> String) -> Vec<Breakdown> {
    let mut groups: BTreeMap<K, Vec<f64>> = BTreeMap::new();
    for t in trades {
        groups.entry(key(t)).or_default().push(t.pnl_r as f64);
    }
    groups
        .into_iter()
        .map(|(k, rs)| {
            let n = rs.len();
            let net: f64 = rs.iter().sum();
            Breakdown { key: name(&k), trades: n, win_rate: rs.iter().filter(|r| **r > 0.0).count() as f64 / n as f64, net_r: net, mean_r: net / n as f64 }
        })
        .collect()
}

impl Report {
    /// `n_trials` is how many strategies were tried in parallel (the archetypes on the floor);
    /// it sets the DSR hurdle. `seed` fixes the bootstrap.
    pub fn build(title: &str, trades: Vec<ReportTrade>, from_ms: i64, to_ms: i64, n_trials: usize, seed: u64) -> Report {
        let to_ms = to_ms.max(from_ms + 1);
        // Trader rows in `Archetype::ALL` order first (so every archetype shows, trades or
        // not), then any archetype the file names that the enum does not know.
        let mut keys: Vec<(String, String, String, bool)> = Archetype::ALL
            .iter()
            .map(|a| (archetype_key(*a), a.name().to_string(), a.label().to_string(), *a == Archetype::Control))
            .collect();
        for t in &trades {
            if !keys.iter().any(|k| k.0 == t.archetype) {
                keys.push((t.archetype.clone(), t.trader.clone(), t.archetype.clone(), false));
            }
        }
        let control_rs: Vec<f64> = {
            let mut c: Vec<&ReportTrade> = trades.iter().filter(|t| t.archetype == archetype_key(Archetype::Control)).collect();
            c.sort_by_key(|t| t.closed_ms);
            c.iter().map(|t| t.pnl_r as f64).collect()
        };

        // Cross-sectional variance of the per-trade Sharpe estimates: the "V" in E[max SR].
        let sharpes: Vec<f64> = keys
            .iter()
            .filter_map(|k| {
                let rs: Vec<f64> = trades.iter().filter(|t| t.archetype == k.0).map(|t| t.pnl_r as f64).collect();
                sharpe(&rs)
            })
            .collect();
        let var_sr = if sharpes.len() >= 2 { sample_sd(&sharpes).powi(2) } else { 0.0 };
        let hurdle = expected_max_sharpe(n_trials, var_sr);

        let mut traders: Vec<TraderReport> = keys
            .iter()
            .enumerate()
            .map(|(i, (key, name, label, is_control))| {
                let mut mine: Vec<&ReportTrade> = trades.iter().filter(|t| &t.archetype == key).collect();
                mine.sort_by_key(|t| (t.closed_ms, t.opened_ms));
                let owned: Vec<ReportTrade> = mine.iter().map(|t| (*t).clone()).collect();
                let stats = trader_stats(&owned);
                let rs: Vec<f64> = mine.iter().map(|t| t.pnl_r as f64).collect();
                let (skew, kurt) = skew_kurt(&rs);
                let sr = sharpe(&rs);
                let trader_seed = seed.wrapping_add(i as u64 * 7919);
                let boot_vs_control = if *is_control { None } else { bootstrap_diff_r(&rs, &control_rs, BOOTSTRAP_DRAWS, trader_seed) };
                let edge = Edge {
                    t_stat: t_stat(&rs),
                    sharpe: sr,
                    skew,
                    kurt,
                    boot_net_r: bootstrap_net_r(&rs, BOOTSTRAP_DRAWS, trader_seed),
                    boot_vs_control,
                    psr0: sr.and_then(|s| psr(s, rs.len(), skew, kurt, 0.0)),
                    expected_max_sharpe: hurdle,
                    dsr: sr.and_then(|s| psr(s, rs.len(), skew, kurt, hurdle)),
                    verdict: Some(verdict(*is_control, rs.len(), boot_vs_control)),
                };
                let mut curve = vec![(from_ms, 0.0)];
                let mut cum = 0.0;
                for t in &mine {
                    cum += t.pnl_r as f64;
                    curve.push((t.closed_ms, cum));
                }
                TraderReport {
                    name: name.clone(),
                    archetype: key.clone(),
                    label: label.clone(),
                    is_control: *is_control,
                    stats,
                    edge,
                    by_symbol: breakdown(&mine, |t| t.symbol.clone(), |k| k.clone()),
                    by_session: breakdown(&mine, |t| t.session(), |s| s.label().to_string()),
                    curve,
                }
            })
            .collect();
        // Leaderboard order: net R, the control wherever it lands.
        traders.sort_by(|a, b| b.stats.net_r.partial_cmp(&a.stats.net_r).unwrap_or(std::cmp::Ordering::Equal));
        let control = traders.iter().position(|t| t.is_control);

        let mut by_day: BTreeMap<String, Vec<(usize, f64)>> = BTreeMap::new();
        for t in &trades {
            let day = fmt_day(t.opened_ms);
            let row = by_day.entry(day).or_insert_with(|| vec![(0, 0.0); traders.len()]);
            if let Some(i) = traders.iter().position(|tr| tr.archetype == t.archetype) {
                row[i].0 += 1;
                row[i].1 += t.pnl_r as f64;
            }
        }
        let days = by_day.into_iter().map(|(day, cells)| DayRow { day, cells }).collect();

        Report { title: title.to_string(), from_ms, to_ms, n_trials, seed, traders, control, days, total_trades: trades.len(), trades }
    }

    pub fn spans_days(&self) -> bool {
        fmt_day(self.from_ms) != fmt_day(self.to_ms - 1) || self.days.len() > 1
    }

    fn control_net(&self) -> Option<f64> {
        self.control.map(|i| self.traders[i].stats.net_r)
    }

    /// Cumulative R of a curve at time `ms` (step function, holds the last exit's value).
    pub fn curve_at(curve: &[(i64, f64)], ms: i64) -> f64 {
        curve.iter().take_while(|(t, _)| *t <= ms).last().map(|(_, v)| *v).unwrap_or(0.0)
    }

    // ---------------------------------------------------------------------
    // Text
    // ---------------------------------------------------------------------

    pub fn text(&self) -> String {
        let mut s = String::new();
        let w = |x: f64| if x.is_finite() { format!("{x:+.2}") } else { "inf".into() };
        s.push_str(&format!("{}\n", self.title));
        s.push_str(&format!(
            "window {} -> {} UTC   {} trades   {} strategies tried (DSR hurdle E[max SR] {:.3})   bootstrap {} draws seed {}\n",
            fmt_ms(self.from_ms),
            fmt_ms(self.to_ms),
            self.total_trades,
            self.n_trials,
            self.traders.first().map(|t| t.edge.expected_max_sharpe).unwrap_or(0.0),
            BOOTSTRAP_DRAWS,
            self.seed
        ));
        s.push_str("R = profit / initial stop (fills at ask/bid, so costs are inside R); a 0 R time stop counts as a loss.\n\n");

        // 1. Per trader.
        s.push_str("PER TRADER\n");
        s.push_str(&format!(
            "{:<8} {:<8} {:>6} {:>5} {:>8} {:>7} {:>7} {:>6} {:>8} {:>8} {:>7} {:>8} {:>7}\n",
            "trader", "type", "trades", "win%", "net R", "mean R", "med R", "PF", "max ddR", "spreadR", "spr%", "hold min", "lose run"
        ));
        for t in &self.traders {
            let st = &t.stats;
            s.push_str(&format!(
                "{:<8} {:<8} {:>6} {:>4.0}% {:>8} {:>7} {:>7} {:>6} {:>8.2} {:>8.2} {:>7} {:>8.1} {:>7}\n",
                t.name,
                t.label,
                st.trades,
                st.win_rate * 100.0,
                w(st.net_r),
                w(st.mean_r),
                w(st.median_r),
                if st.profit_factor.is_finite() { format!("{:.2}", st.profit_factor) } else { "inf".into() },
                st.max_drawdown_r,
                st.spread_r,
                st.spread_share_of_gross.map(|x| format!("{:.0}%", x * 100.0)).unwrap_or_else(|| "n/a".into()),
                st.avg_hold_min,
                st.longest_losing_run
            ));
        }
        s.push_str("spr% = spread paid / before-cost result (n/a when that result is not positive)\n\n");

        // 2. Edge.
        s.push_str("EDGE  (does the row earn belief?)\n");
        s.push_str(&format!(
            "{:<8} {:>6} {:>6} {:>7} {:>18} {:>18} {:>6} {:>6}  {}\n",
            "trader", "trades", "t", "SR/trd", "net R 95% CI", "vs control 95% CI", "PSR0", "DSR", "beats control after costs"
        ));
        let ci = |x: Option<(f64, f64)>| x.map(|(a, b)| format!("[{a:+.2}, {b:+.2}]")).unwrap_or_else(|| "-".into());
        let opt = |x: Option<f64>, d: usize| x.map(|v| format!("{:.*}", d, v)).unwrap_or_else(|| "-".into());
        for t in &self.traders {
            let e = &t.edge;
            s.push_str(&format!(
                "{:<8} {:>6} {:>6} {:>7} {:>18} {:>18} {:>6} {:>6}  {}\n",
                t.name,
                t.stats.trades,
                opt(e.t_stat, 2),
                opt(e.sharpe, 3),
                ci(e.boot_net_r),
                ci(e.boot_vs_control),
                opt(e.psr0, 3),
                opt(e.dsr, 3),
                e.verdict.map(|v| v.label()).unwrap_or("-")
            ));
        }
        s.push_str(&format!(
            "verdict: yes when the whole bootstrap interval of (trader − control) net R is above 0; needs ≥ {} trades. \
             PSR0 = P(true Sharpe > 0); DSR = P(true Sharpe > best-of-{} luck). Control net R {}.\n\n",
            MIN_TRADES_FOR_VERDICT,
            self.n_trials,
            self.control_net().map(|c| format!("{c:+.2}")).unwrap_or_else(|| "n/a".into())
        ));

        // 3. Symbol and session breakdowns.
        s.push_str("BY SYMBOL  (trades / win% / net R)\n");
        let symbols: Vec<String> = {
            let mut v: Vec<String> = self.trades.iter().map(|t| t.symbol.clone()).collect();
            v.sort();
            v.dedup();
            v
        };
        s.push_str(&format!("{:<8}", "trader"));
        for sym in &symbols {
            s.push_str(&format!(" {:>16}", sym));
        }
        s.push('\n');
        for t in &self.traders {
            s.push_str(&format!("{:<8}", t.name));
            for sym in &symbols {
                s.push_str(&format!(" {:>16}", cell(t.by_symbol.iter().find(|b| &b.key == sym))));
            }
            s.push('\n');
        }
        s.push('\n');

        s.push_str("BY SESSION (UTC, entry time; trades / win% / net R)\n");
        s.push_str(&format!("{:<8}", "trader"));
        for ses in Session::ALL {
            s.push_str(&format!(" {:>20}", format!("{} {}", ses.label(), ses.hours())));
        }
        s.push('\n');
        for t in &self.traders {
            s.push_str(&format!("{:<8}", t.name));
            for ses in Session::ALL {
                s.push_str(&format!(" {:>20}", cell(t.by_session.iter().find(|b| b.key == ses.label()))));
            }
            s.push('\n');
        }
        s.push('\n');

        // 4. Daily.
        if self.spans_days() {
            s.push_str("BY DAY (UTC, entry day; trades / net R)\n");
            s.push_str(&format!("{:<11}", "day"));
            for t in &self.traders {
                s.push_str(&format!(" {:>13}", t.name));
            }
            s.push('\n');
            for d in &self.days {
                s.push_str(&format!("{:<11}", d.day));
                for (n, r) in &d.cells {
                    s.push_str(&format!(" {:>13}", if *n == 0 { "-".to_string() } else { format!("{n} / {r:+.2}") }));
                }
                s.push('\n');
            }
            s.push('\n');
        }

        // 5. Equity sparklines.
        s.push_str(&format!("EQUITY (cumulative R by exit time, {} columns across the window; control last)\n", SPARK_COLS));
        for t in &self.traders {
            let (spark, lo, hi) = sparkline(&t.curve, self.from_ms, self.to_ms);
            s.push_str(&format!("{:<8} {} min {:+.2} max {:+.2} end {:+.2}\n", t.name, spark, lo, hi, t.stats.net_r));
        }
        s
    }

    // ---------------------------------------------------------------------
    // HTML
    // ---------------------------------------------------------------------

    pub fn html(&self) -> String {
        let mut h = String::new();
        h.push_str("<!doctype html>\n<html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n");
        h.push_str(&format!("<title>{}</title>\n", esc(&self.title)));
        h.push_str(HTML_CSS);
        h.push_str("</head><body>\n");
        h.push_str(&format!("<h1>{}</h1>\n", esc(&self.title)));
        h.push_str(&format!(
            "<p class=\"sub\">{} → {} UTC · {} trades · {} strategies tried, DSR hurdle E[max SR] {:.3} · bootstrap {} draws, seed {}</p>\n",
            fmt_ms(self.from_ms),
            fmt_ms(self.to_ms),
            self.total_trades,
            self.n_trials,
            self.traders.first().map(|t| t.edge.expected_max_sharpe).unwrap_or(0.0),
            BOOTSTRAP_DRAWS,
            self.seed
        ));
        h.push_str("<p class=\"sub\">R = profit / initial stop; fills at ask/bid so costs are inside R; a 0 R time stop counts as a loss. Verdict: <em>yes</em> when the whole bootstrap interval of (trader − control) net R is above 0, with at least ");
        h.push_str(&format!("{} trades.</p>\n", MIN_TRADES_FOR_VERDICT));

        // Verdict tiles.
        h.push_str("<section><h2>Verdicts</h2><div class=\"tiles\">\n");
        for t in &self.traders {
            let v = t.edge.verdict.unwrap_or(Verdict::TooFewTrades);
            let class = match v {
                Verdict::Yes => "yes",
                Verdict::No => "no",
                _ => "meh",
            };
            h.push_str(&format!(
                "<div class=\"tile {class}\"><div class=\"who\"><span class=\"swatch\" style=\"background:{}\"></span>{} <small>{}</small></div><div class=\"big\">{:+.2}<small> R</small></div><div class=\"line\">{} trades · win {:.0}% · DSR {}</div><div class=\"line\">beats control after costs: <b>{}</b></div></div>\n",
                self.colour(t),
                esc(&t.name),
                esc(&t.label),
                t.stats.net_r,
                t.stats.trades,
                t.stats.win_rate * 100.0,
                t.edge.dsr.map(|d| format!("{d:.3}")).unwrap_or_else(|| "–".into()),
                v.label()
            ));
        }
        h.push_str("</div></section>\n");

        // Equity curves: one panel per trader with the control dashed alongside.
        h.push_str("<section><h2>Equity in R</h2><p class=\"sub\">Each panel: the trader (solid) and Colin the coin flip (dashed grey). Step at each exit. Touch or hover for the value.</p><div class=\"grid\">\n");
        let control_curve = self.control.map(|i| self.traders[i].curve.clone());
        for t in &self.traders {
            h.push_str(&self.svg_curve(t, control_curve.as_deref()));
        }
        h.push_str("</div></section>\n");

        // Per-trader table.
        h.push_str("<section><h2>Per trader</h2><div class=\"wrap\"><table><thead><tr><th>trader</th><th>type</th><th>trades</th><th>win%</th><th>net R</th><th>mean R</th><th>median R</th><th>PF</th><th>max DD R</th><th>spread R</th><th>spread % of gross</th><th>avg hold min</th><th>longest losing run</th></tr></thead><tbody>\n");
        for t in &self.traders {
            let st = &t.stats;
            h.push_str(&format!(
                "<tr><td><span class=\"swatch\" style=\"background:{}\"></span>{}</td><td>{}</td><td>{}</td><td>{:.0}%</td><td class=\"{}\">{:+.2}</td><td>{:+.2}</td><td>{:+.2}</td><td>{}</td><td>{:.2}</td><td>{:.2}</td><td>{}</td><td>{:.1}</td><td>{}</td></tr>\n",
                self.colour(t),
                esc(&t.name),
                esc(&t.label),
                st.trades,
                st.win_rate * 100.0,
                sign_class(st.net_r),
                st.net_r,
                st.mean_r,
                st.median_r,
                if st.profit_factor.is_finite() { format!("{:.2}", st.profit_factor) } else { "∞".into() },
                st.max_drawdown_r,
                st.spread_r,
                st.spread_share_of_gross.map(|x| format!("{:.0}%", x * 100.0)).unwrap_or_else(|| "n/a".into()),
                st.avg_hold_min,
                st.longest_losing_run
            ));
        }
        h.push_str("</tbody></table></div></section>\n");

        // Edge table.
        h.push_str("<section><h2>Edge</h2><p class=\"sub\">t = mean R / standard error. SR/trade = mean R / sd R. PSR0 = P(true Sharpe &gt; 0). DSR = P(true Sharpe &gt; the best of ");
        h.push_str(&format!("{} skill-less strategies), after Bailey &amp; López de Prado; the edge-lab bar is 0.95.</p><div class=\"wrap\"><table><thead><tr><th>trader</th><th>trades</th><th>t</th><th>SR/trade</th><th>net R 95% CI</th><th>vs control 95% CI</th><th>PSR0</th><th>DSR</th><th>beats control after costs</th></tr></thead><tbody>\n", self.n_trials));
        for t in &self.traders {
            let e = &t.edge;
            let ci = |x: Option<(f64, f64)>| x.map(|(a, b)| format!("[{a:+.2}, {b:+.2}]")).unwrap_or_else(|| "–".into());
            let opt = |x: Option<f64>, d: usize| x.map(|v| format!("{:.*}", d, v)).unwrap_or_else(|| "–".into());
            let v = e.verdict.unwrap_or(Verdict::TooFewTrades);
            h.push_str(&format!(
                "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td class=\"v-{}\">{}</td></tr>\n",
                esc(&t.name),
                t.stats.trades,
                opt(e.t_stat, 2),
                opt(e.sharpe, 3),
                ci(e.boot_net_r),
                ci(e.boot_vs_control),
                opt(e.psr0, 3),
                opt(e.dsr, 3),
                match v {
                    Verdict::Yes => "yes",
                    Verdict::No => "no",
                    _ => "meh",
                },
                v.label()
            ));
        }
        h.push_str("</tbody></table></div></section>\n");

        // Breakdowns.
        let symbols: Vec<String> = {
            let mut v: Vec<String> = self.trades.iter().map(|t| t.symbol.clone()).collect();
            v.sort();
            v.dedup();
            v
        };
        h.push_str("<section><h2>By symbol</h2><p class=\"sub\">trades / win% / net R</p><div class=\"wrap\"><table><thead><tr><th>trader</th>");
        for sym in &symbols {
            h.push_str(&format!("<th>{}</th>", esc(sym)));
        }
        h.push_str("</tr></thead><tbody>\n");
        for t in &self.traders {
            h.push_str(&format!("<tr><td>{}</td>", esc(&t.name)));
            for sym in &symbols {
                let b = t.by_symbol.iter().find(|b| &b.key == sym);
                h.push_str(&format!("<td class=\"{}\">{}</td>", b.map(|b| sign_class(b.net_r)).unwrap_or(""), cell(b)));
            }
            h.push_str("</tr>\n");
        }
        h.push_str("</tbody></table></div></section>\n");

        h.push_str("<section><h2>By session</h2><p class=\"sub\">UTC, by entry time; trades / win% / net R</p><div class=\"wrap\"><table><thead><tr><th>trader</th>");
        for ses in Session::ALL {
            h.push_str(&format!("<th>{}<br><small>{}</small></th>", ses.label(), ses.hours()));
        }
        h.push_str("</tr></thead><tbody>\n");
        for t in &self.traders {
            h.push_str(&format!("<tr><td>{}</td>", esc(&t.name)));
            for ses in Session::ALL {
                let b = t.by_session.iter().find(|b| b.key == ses.label());
                h.push_str(&format!("<td class=\"{}\">{}</td>", b.map(|b| sign_class(b.net_r)).unwrap_or(""), cell(b)));
            }
            h.push_str("</tr>\n");
        }
        h.push_str("</tbody></table></div></section>\n");

        if self.spans_days() {
            h.push_str("<section><h2>By day</h2><p class=\"sub\">UTC, by entry day; trades / net R</p><div class=\"wrap\"><table><thead><tr><th>day</th>");
            for t in &self.traders {
                h.push_str(&format!("<th>{}</th>", esc(&t.name)));
            }
            h.push_str("</tr></thead><tbody>\n");
            for d in &self.days {
                h.push_str(&format!("<tr><td>{}</td>", d.day));
                for (n, r) in &d.cells {
                    if *n == 0 {
                        h.push_str("<td>–</td>");
                    } else {
                        h.push_str(&format!("<td class=\"{}\">{n} / {r:+.2}</td>", sign_class(*r)));
                    }
                }
                h.push_str("</tr>\n");
            }
            h.push_str("</tbody></table></div></section>\n");
        }

        // Trade list.
        const MAX_ROWS: usize = 400;
        let mut trades: Vec<&ReportTrade> = self.trades.iter().collect();
        trades.sort_by_key(|t| (t.opened_ms, t.closed_ms));
        h.push_str(&format!(
            "<section><h2>Trades</h2><p class=\"sub\">{} of {} shown, entry order.</p><div class=\"wrap\"><table class=\"trades\"><thead><tr><th>opened</th><th>trader</th><th>symbol</th><th>side</th><th>session</th><th>R</th><th>hold min</th><th>spread R</th><th>read</th><th>in</th><th>out</th></tr></thead><tbody>\n",
            trades.len().min(MAX_ROWS),
            trades.len()
        ));
        for t in trades.iter().take(MAX_ROWS) {
            h.push_str(&format!(
                "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td class=\"{}\">{:+.2}</td><td>{:.1}</td><td>{:.2}</td><td>{}</td><td class=\"why\">{}</td><td class=\"why\">{}</td></tr>\n",
                &fmt_ms(t.opened_ms)[5..16].replace('T', " "),
                esc(&t.trader),
                esc(&t.symbol),
                esc(&t.side),
                t.session().label(),
                sign_class(t.pnl_r as f64),
                t.pnl_r,
                t.hold_minutes(),
                t.spread_r(),
                t.read_source.as_deref().map(esc).unwrap_or_else(|| "–".into()),
                esc(&t.why_in),
                esc(&t.why_out)
            ));
        }
        h.push_str("</tbody></table></div></section>\n");
        h.push_str(HTML_JS);
        h.push_str("</body></html>\n");
        h
    }

    /// Categorical colour by archetype, fixed order (never by rank). The control is drawn in
    /// the muted ink so it reads as the baseline, not a competitor.
    fn colour(&self, t: &TraderReport) -> &'static str {
        if t.is_control {
            return "#898781";
        }
        let i = Archetype::ALL.iter().position(|a| archetype_key(*a) == t.archetype).unwrap_or(5);
        SERIES[i.min(SERIES.len() - 1)]
    }

    fn svg_curve(&self, t: &TraderReport, control: Option<&[(i64, f64)]>) -> String {
        const W: f64 = 320.0;
        const H: f64 = 170.0;
        const L: f64 = 36.0;
        const R: f64 = 12.0;
        const T: f64 = 10.0;
        const B: f64 = 22.0;
        let span = (self.to_ms - self.from_ms).max(1) as f64;
        let mut lo = 0.0f64;
        let mut hi = 0.0f64;
        for (_, v) in t.curve.iter().chain(control.into_iter().flatten()) {
            lo = lo.min(*v);
            hi = hi.max(*v);
        }
        if hi - lo < 1.0 {
            hi = lo + 1.0;
        }
        let pad = (hi - lo) * 0.08;
        let (lo, hi) = (lo - pad, hi + pad);
        let x = |ms: i64| L + (ms - self.from_ms).clamp(0, self.to_ms - self.from_ms) as f64 / span * (W - L - R);
        let y = |v: f64| T + (hi - v) / (hi - lo) * (H - T - B);
        let path = |c: &[(i64, f64)]| -> String {
            let mut d = String::new();
            let mut last_y: Option<f64> = None;
            for (ms, v) in c {
                let (px, py) = (x(*ms), y(*v));
                match last_y {
                    None => d.push_str(&format!("M{px:.1},{py:.1}")),
                    Some(ly) => {
                        d.push_str(&format!("H{px:.1}"));
                        if (py - ly).abs() >= 0.05 {
                            d.push_str(&format!("V{py:.1}"));
                        }
                    }
                }
                last_y = Some(py);
            }
            // Hold to the window end.
            d.push_str(&format!("H{:.1}", x(self.to_ms)));
            d
        };
        let mut s = String::new();
        let pts: String = t.curve.iter().map(|(ms, v)| format!("{ms}:{v:.3}")).collect::<Vec<_>>().join(",");
        s.push_str(&format!(
            "<figure class=\"panel\"><figcaption><span class=\"swatch\" style=\"background:{}\"></span>{} <small>{}</small> <span class=\"end {}\">{:+.2} R</span></figcaption>\n<svg viewBox=\"0 0 {W} {H}\" data-from=\"{}\" data-to=\"{}\" data-l=\"{L}\" data-r=\"{R}\" data-pts=\"{}\" data-name=\"{}\" role=\"img\" aria-label=\"equity curve of {} in R\">\n",
            self.colour(t),
            esc(&t.name),
            esc(&t.label),
            sign_class(t.stats.net_r),
            t.stats.net_r,
            self.from_ms,
            self.to_ms,
            pts,
            esc(&t.name),
            esc(&t.name)
        ));
        // Grid: zero line plus top/bottom ticks.
        for v in nice_ticks(lo, hi) {
            let yy = y(v);
            let cls = if v == 0.0 { "zero" } else { "grid" };
            s.push_str(&format!("<line class=\"{cls}\" x1=\"{L}\" x2=\"{:.1}\" y1=\"{yy:.1}\" y2=\"{yy:.1}\"/><text class=\"tick\" x=\"{:.1}\" y=\"{:.1}\" text-anchor=\"end\">{v:+.0}</text>\n", W - R, L - 4.0, yy + 3.0));
        }
        // X labels: start, end.
        s.push_str(&format!("<text class=\"tick\" x=\"{L}\" y=\"{:.1}\">{}</text><text class=\"tick\" x=\"{:.1}\" y=\"{:.1}\" text-anchor=\"end\">{}</text>\n", H - 6.0, hhmm(self.from_ms), W - R, H - 6.0, hhmm(self.to_ms)));
        if let Some(c) = control {
            if !t.is_control {
                s.push_str(&format!("<path class=\"control\" d=\"{}\"/>\n", path(c)));
            }
        }
        s.push_str(&format!("<path class=\"series\" style=\"stroke:{}\" d=\"{}\"/>\n", self.colour(t), path(&t.curve)));
        s.push_str(&format!("<line class=\"cursor\" x1=\"0\" x2=\"0\" y1=\"{T}\" y2=\"{:.1}\" visibility=\"hidden\"/><text class=\"tip\" x=\"0\" y=\"{:.1}\" visibility=\"hidden\"></text>\n", H - B, T + 10.0));
        s.push_str("</svg></figure>\n");
        s
    }
}

/// Up to four round ticks spanning `lo..hi`, always including 0 when it is in range.
fn nice_ticks(lo: f64, hi: f64) -> Vec<f64> {
    if !(hi > lo) {
        return vec![0.0];
    }
    let raw = (hi - lo) / 3.0;
    let mag = 10f64.powf(raw.log10().floor());
    let step = [1.0, 2.0, 5.0, 10.0].iter().map(|m| m * mag).find(|s| *s >= raw).unwrap_or(mag * 10.0);
    let mut v = Vec::new();
    let mut t = (lo / step).ceil() * step;
    while t <= hi + 1e-9 {
        v.push(if t.abs() < 1e-9 { 0.0 } else { t });
        t += step;
    }
    if lo < 0.0 && hi > 0.0 && !v.iter().any(|x| *x == 0.0) {
        v.push(0.0);
    }
    v
}

fn cell(b: Option<&Breakdown>) -> String {
    match b {
        None => "-".into(),
        Some(b) => format!("{} / {:.0}% / {:+.2}", b.trades, b.win_rate * 100.0, b.net_r),
    }
}

fn sign_class(v: f64) -> &'static str {
    if v > 0.0 {
        "pos"
    } else if v < 0.0 {
        "neg"
    } else {
        ""
    }
}

fn sparkline(curve: &[(i64, f64)], from_ms: i64, to_ms: i64) -> (String, f64, f64) {
    const BLOCKS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let samples: Vec<f64> = (0..SPARK_COLS)
        .map(|i| {
            let ms = from_ms + ((to_ms - from_ms) as f64 * (i as f64 + 1.0) / SPARK_COLS as f64) as i64;
            Report::curve_at(curve, ms)
        })
        .collect();
    let lo = samples.iter().cloned().fold(0.0f64, f64::min);
    let hi = samples.iter().cloned().fold(0.0f64, f64::max);
    let range = (hi - lo).max(1e-9);
    let line: String = samples.iter().map(|v| BLOCKS[(((v - lo) / range) * 7.0).round().clamp(0.0, 7.0) as usize]).collect();
    (line, lo, hi)
}

pub fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Categorical dark-surface palette (validated adjacent-pair order; see the dataviz
/// reference palette). Index = position in `Archetype::ALL`; the control is drawn grey.
const SERIES: [&str; 6] = ["#3987e5", "#d95926", "#199e70", "#c98500", "#d55181", "#008300"];

// ---------------------------------------------------------------------------
// Time formatting (UTC, no chrono)
// ---------------------------------------------------------------------------

/// Civil date from days since the epoch (Howard Hinnant's algorithm).
fn civil(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

pub fn fmt_ms(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let (y, m, d) = civil(secs.div_euclid(86_400));
    let day = secs.rem_euclid(86_400);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", day / 3600, (day % 3600) / 60, day % 60)
}

pub fn fmt_day(ms: i64) -> String {
    let (y, m, d) = civil(ms.div_euclid(86_400_000));
    format!("{y:04}-{m:02}-{d:02}")
}

fn hhmm(ms: i64) -> String {
    let m = ms.rem_euclid(86_400_000) / 60_000;
    format!("{:02}:{:02}", m / 60, m % 60)
}

// ---------------------------------------------------------------------------
// HTML chrome
// ---------------------------------------------------------------------------

const HTML_CSS: &str = r#"<style>
:root{color-scheme:dark;--page:#0d0d0d;--surface:#1a1a19;--ink:#f0efec;--ink2:#c3c2b7;--muted:#898781;--grid:#2c2c2a;--axis:#383835;--pos:#0ca30c;--neg:#e66767;--ring:rgba(255,255,255,.10)}
*{box-sizing:border-box}
body{margin:0;padding:16px;background:var(--page);color:var(--ink);font:14px/1.45 system-ui,-apple-system,"Segoe UI",sans-serif}
h1{font-size:20px;margin:0 0 4px;overflow-wrap:anywhere}h2{font-size:16px;margin:28px 0 6px;color:var(--ink)}
.sub{color:var(--ink2);margin:2px 0 8px;font-size:13px;overflow-wrap:anywhere}
.tiles{display:grid;grid-template-columns:repeat(auto-fill,minmax(180px,1fr));gap:10px}
.tile{background:var(--surface);border:1px solid var(--ring);border-left:4px solid var(--muted);border-radius:8px;padding:10px 12px}
.tile.yes{border-left-color:var(--pos)}.tile.no{border-left-color:var(--neg)}
.tile .who{font-weight:600}.tile .big{font-size:26px;font-weight:600;margin:4px 0;font-variant-numeric:tabular-nums}
.tile .line{font-size:12px;color:var(--ink2)}.tile small,.panel small{color:var(--muted);font-weight:400}
.grid{display:grid;grid-template-columns:repeat(auto-fill,minmax(280px,1fr));gap:12px}
.panel{margin:0;background:var(--surface);border:1px solid var(--ring);border-radius:8px;padding:8px 8px 4px}
.panel figcaption{font-size:13px;font-weight:600;padding:0 4px 4px;display:flex;gap:6px;align-items:baseline}
.panel .end{margin-left:auto;font-variant-numeric:tabular-nums}
.panel svg{width:100%;height:auto;display:block;touch-action:none}
svg .grid{stroke:var(--grid);stroke-width:1}svg .zero{stroke:var(--axis);stroke-width:1.5}
svg .tick{fill:var(--muted);font-size:9px}
svg .series{fill:none;stroke-width:2;stroke-linejoin:round}
svg .control{fill:none;stroke:var(--muted);stroke-width:1.5;stroke-dasharray:4 3}
svg .cursor{stroke:var(--ink2);stroke-width:1;stroke-dasharray:2 2}svg .tip{fill:var(--ink);font-size:10px;font-variant-numeric:tabular-nums}
.swatch{display:inline-block;width:10px;height:10px;border-radius:2px;margin-right:6px;vertical-align:baseline}
.wrap{overflow-x:auto;-webkit-overflow-scrolling:touch;background:var(--surface);border:1px solid var(--ring);border-radius:8px}
table{border-collapse:collapse;width:100%;font-size:13px;font-variant-numeric:tabular-nums;white-space:nowrap}
th,td{padding:6px 10px;text-align:right;border-bottom:1px solid var(--grid)}th{color:var(--ink2);font-weight:600;position:sticky;top:0;background:var(--surface)}
th:first-child,td:first-child{text-align:left}tr:last-child td{border-bottom:0}
td.pos{color:var(--pos)}td.neg{color:var(--neg)}.end.pos{color:var(--pos)}.end.neg{color:var(--neg)}
td.v-yes{color:var(--pos);font-weight:600}td.v-no{color:var(--neg);font-weight:600}td.v-meh{color:var(--muted)}
table.trades td.why{text-align:left;max-width:260px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;color:var(--ink2)}
em{color:var(--pos);font-style:normal}
</style>
"#;

const HTML_JS: &str = r#"<script>
// Crosshair + value readout on each equity panel. Pure DOM; no libraries.
document.querySelectorAll('.panel svg').forEach(function(svg){
  var from=+svg.dataset.from,to=+svg.dataset.to,L=+svg.dataset.l,R=+svg.dataset.r,W=svg.viewBox.baseVal.width;
  var pts=(svg.dataset.pts||'').split(',').filter(Boolean).map(function(p){var a=p.split(':');return [+a[0],+a[1]];});
  var cur=svg.querySelector('.cursor'),tip=svg.querySelector('.tip');
  function hh(ms){var m=Math.floor((ms%86400000)/60000);return (m/60|0).toString().padStart(2,'0')+':'+(m%60).toString().padStart(2,'0');}
  function at(ms){var v=0;for(var i=0;i<pts.length;i++){if(pts[i][0]<=ms)v=pts[i][1];else break;}return v;}
  function show(ev){
    var r=svg.getBoundingClientRect(),cx=(ev.touches?ev.touches[0].clientX:ev.clientX)-r.left;
    var x=cx/r.width*W;if(x<L)x=L;if(x>W-R)x=W-R;
    var ms=from+(x-L)/(W-L-R)*(to-from),v=at(ms);
    cur.setAttribute('x1',x);cur.setAttribute('x2',x);cur.setAttribute('visibility','visible');
    tip.textContent=hh(ms)+'  '+(v>=0?'+':'')+v.toFixed(2)+' R';
    var anchorEnd=x>W*0.6;tip.setAttribute('text-anchor',anchorEnd?'end':'start');tip.setAttribute('x',anchorEnd?x-4:x+4);tip.setAttribute('visibility','visible');
  }
  function hide(){cur.setAttribute('visibility','hidden');tip.setAttribute('visibility','hidden');}
  svg.addEventListener('mousemove',show);svg.addEventListener('mouseleave',hide);
  svg.addEventListener('touchstart',show,{passive:true});svg.addEventListener('touchmove',show,{passive:true});svg.addEventListener('touchend',hide);
});
</script>
"#;

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jev::ReadSource;

    const T0: i64 = 1_792_627_200_000; // 2026-10-22T00:00:00Z

    fn rt(archetype: Archetype, symbol: &str, opened_min: i64, hold_min: i64, pnl_r: f32, stop: f32, spread: f32) -> ReportTrade {
        ReportTrade {
            trader: archetype.name().into(),
            archetype: archetype_key(archetype),
            symbol: symbol.into(),
            side: "long".into(),
            opened_ms: T0 + opened_min * 60_000,
            closed_ms: T0 + (opened_min + hold_min) * 60_000,
            pnl_r,
            pnl_ticks: pnl_r * stop,
            stop_ticks: stop,
            spread_ticks_in: spread,
            why_in: "test".into(),
            why_out: "test".into(),
            read_source: None,
        }
    }

    fn series(archetype: Archetype, rs: &[f32]) -> Vec<ReportTrade> {
        rs.iter().enumerate().map(|(i, r)| rt(archetype, "XAG/USD", i as i64 * 10, 5, *r, 10.0, 1.0)).collect()
    }

    #[test]
    fn session_windows_by_utc_entry_minute() {
        let at = |h: i64, m: i64| T0 + (h * 60 + m) * 60_000;
        assert_eq!(Session::of_ms(at(0, 0)), Session::Asia);
        assert_eq!(Session::of_ms(at(6, 59)), Session::Asia);
        assert_eq!(Session::of_ms(at(7, 0)), Session::London);
        assert_eq!(Session::of_ms(at(11, 59)), Session::London);
        assert_eq!(Session::of_ms(at(12, 0)), Session::Overlap);
        assert_eq!(Session::of_ms(at(16, 29)), Session::Overlap);
        assert_eq!(Session::of_ms(at(16, 30)), Session::NewYork);
        assert_eq!(Session::of_ms(at(20, 59)), Session::NewYork);
        assert_eq!(Session::of_ms(at(21, 0)), Session::Late);
        assert_eq!(Session::of_ms(at(23, 59)), Session::Late);
        // Day boundaries and negative epochs do not matter: time of day only.
        assert_eq!(Session::of_ms(at(24, 0)), Session::Asia);
        assert_eq!(Session::of_ms(-60_000), Session::Late);
    }

    #[test]
    fn session_names_parse() {
        assert_eq!(Session::parse_list("london,ny").unwrap(), vec![Session::London, Session::NewYork]);
        assert_eq!(Session::parse_list("NY, Overlap,new_york").unwrap(), vec![Session::Overlap, Session::NewYork]);
        assert!(Session::parse_list("london,mars").is_err());
    }

    #[test]
    fn stats_on_hand_built_trades() {
        // Exit order: +1, -1, +2, -0.5, 0 (time stop), +0.5
        let rs = [1.0, -1.0, 2.0, -0.5, 0.0, 0.5];
        let trades = series(Archetype::TrendRider, &rs);
        let st = trader_stats(&trades);
        assert_eq!(st.trades, 6);
        assert_eq!(st.wins, 3);
        assert_eq!(st.losses, 3);
        assert!((st.win_rate - 0.5).abs() < 1e-9);
        assert!((st.net_r - 2.0).abs() < 1e-6);
        assert!((st.mean_r - 2.0 / 6.0).abs() < 1e-6);
        assert!((st.median_r - 0.25).abs() < 1e-6); // sorted: -1 -0.5 0 0.5 1 2 -> (0+0.5)/2
        assert!((st.gross_win_r - 3.5).abs() < 1e-6);
        assert!((st.gross_loss_r - 1.5).abs() < 1e-6);
        assert!((st.profit_factor - 3.5 / 1.5).abs() < 1e-6);
        // cum: 1, 0, 2, 1.5, 1.5, 2 -> peak 2, trough 1.5 after it, and 1->0 earlier: dd 1.0
        assert!((st.max_drawdown_r - 1.0).abs() < 1e-6);
        // spread 1 tick / stop 10 ticks = 0.1 R each, six trades
        assert!((st.spread_r - 0.6).abs() < 1e-6);
        // before-cost 2.6, spread share 0.6/2.6
        assert!((st.spread_share_of_gross.unwrap() - 0.6 / 2.6).abs() < 1e-6);
        assert!((st.avg_hold_min - 5.0).abs() < 1e-9);
        assert_eq!(st.longest_losing_run, 2); // -0.5 then 0
    }

    #[test]
    fn stats_edge_cases() {
        assert_eq!(trader_stats(&[]).trades, 0);
        let all_win = trader_stats(&series(Archetype::GapFiller, &[1.0, 2.0]));
        assert!(all_win.profit_factor.is_infinite());
        assert_eq!(all_win.longest_losing_run, 0);
        assert_eq!(all_win.max_drawdown_r, 0.0);
        let all_lose = trader_stats(&series(Archetype::GapFiller, &[-1.0, -2.0, -0.5]));
        assert_eq!(all_lose.profit_factor, 0.0);
        assert_eq!(all_lose.longest_losing_run, 3);
        assert!((all_lose.max_drawdown_r - 3.5).abs() < 1e-6);
        assert!(all_lose.spread_share_of_gross.is_none());
    }

    #[test]
    fn drawdown_and_runs_direct() {
        assert!((max_drawdown(&[1.0, 1.0, -3.0, 2.0, -1.0, -1.0]) - 3.0).abs() < 1e-9);
        assert_eq!(longest_run(&[-1.0, -1.0, 1.0, -1.0, -1.0, -1.0, 1.0], |r| r <= 0.0), 3);
        assert!((median(&[3.0, 1.0, 2.0]) - 2.0).abs() < 1e-9);
        assert!((median(&[4.0, 1.0, 2.0, 3.0]) - 2.5).abs() < 1e-9);
    }

    #[test]
    fn t_stat_hand_example() {
        // mean 1.5, sample sd 1.0, n 4 -> se 0.5 -> t 3
        let t = t_stat(&[1.0, 1.0, 1.0, 3.0]).unwrap();
        assert!((t - 3.0).abs() < 1e-9);
        assert!(t_stat(&[1.0]).is_none());
        assert!(t_stat(&[2.0, 2.0, 2.0]).is_none());
        assert!((sharpe(&[1.0, 1.0, 1.0, 3.0]).unwrap() - 1.5).abs() < 1e-9);
    }

    #[test]
    fn normal_cdf_and_inverse() {
        assert!((norm_cdf(0.0) - 0.5).abs() < 1e-7);
        assert!((norm_cdf(1.959_964) - 0.975).abs() < 1e-5);
        assert!((norm_cdf(-1.0) - 0.158_655).abs() < 1e-5);
        assert!((norm_ppf(0.975) - 1.959_964).abs() < 1e-5);
        assert!((norm_ppf(0.5)).abs() < 1e-9);
        assert!((norm_ppf(0.8333333) - 0.967_42).abs() < 1e-4);
        assert!((norm_ppf(0.01) + 2.326_35).abs() < 1e-4);
        for p in [0.001, 0.05, 0.3, 0.7, 0.95, 0.999] {
            assert!((norm_cdf(norm_ppf(p)) - p).abs() < 1e-6, "roundtrip {p}");
        }
    }

    #[test]
    fn psr_worked_example() {
        // SR 0.1 over 101 Gaussian observations: z = 1/sqrt(1.005) = 0.99751 -> 0.8407 (see doc).
        let p = psr(0.1, 101, 0.0, 3.0, 0.0).unwrap();
        assert!((p - 0.8407).abs() < 5e-4, "{p}");
        // Negative skew and fat tails widen the denominator and lower the probability.
        let fat = psr(0.1, 101, -1.0, 6.0, 0.0).unwrap();
        assert!(fat < p);
        // A hurdle above the observed Sharpe gives < 0.5.
        assert!(psr(0.1, 101, 0.0, 3.0, 0.2).unwrap() < 0.5);
        assert!(psr(0.1, 1, 0.0, 3.0, 0.0).is_none());
    }

    #[test]
    fn expected_max_sharpe_worked_example() {
        // N = 6, V = 1 -> 1.300 (see doc comment).
        let e = expected_max_sharpe(6, 1.0);
        assert!((e - 1.300).abs() < 2e-3, "{e}");
        // Scales with sqrt(V).
        assert!((expected_max_sharpe(6, 4.0) - 2.0 * e).abs() < 1e-9);
        // Degenerate: a single trial or no dispersion has no selection hurdle.
        assert_eq!(expected_max_sharpe(1, 1.0), 0.0);
        assert_eq!(expected_max_sharpe(6, 0.0), 0.0);
        // More trials, higher hurdle.
        assert!(expected_max_sharpe(100, 1.0) > e);
    }

    #[test]
    fn dsr_collapses_to_psr_without_selection() {
        let a = dsr(0.1, 101, 0.0, 3.0, 1, 0.5).unwrap();
        let b = psr(0.1, 101, 0.0, 3.0, 0.0).unwrap();
        assert!((a - b).abs() < 1e-12);
        // With six trials of spread 0.05² the hurdle is ~0.065, so the same SR deflates.
        let c = dsr(0.1, 101, 0.0, 3.0, 6, 0.0025).unwrap();
        assert!(c < b && c > 0.5, "{c}");
    }

    #[test]
    fn skew_kurt_of_symmetric_and_skewed() {
        let (s, k) = skew_kurt(&[-1.0, 0.0, 1.0]);
        assert!(s.abs() < 1e-12);
        assert!((k - 1.5).abs() < 1e-12); // m4 = 2/3, m2 = 2/3 -> 0.6667/0.4444
        let (s, _) = skew_kurt(&[0.0, 0.0, 0.0, 10.0]);
        assert!(s > 0.0);
        assert_eq!(skew_kurt(&[1.0]), (0.0, 3.0));
    }

    #[test]
    fn bootstrap_is_seeded_and_sane() {
        let rs: Vec<f64> = (0..40).map(|i| if i % 3 == 0 { -1.0 } else { 0.8 }).collect();
        let a = bootstrap_net_r(&rs, 2000, 7).unwrap();
        let b = bootstrap_net_r(&rs, 2000, 7).unwrap();
        assert_eq!(a, b, "same seed, same interval");
        let c = bootstrap_net_r(&rs, 2000, 8).unwrap();
        let net: f64 = rs.iter().sum();
        assert!(a.0 < net && net < a.1, "interval {a:?} should bracket the net {net}");
        assert!(c.0 < net && net < c.1, "interval {c:?} should bracket the net {net}");
        assert!(a.0 < a.1);
        // A constant series has no sampling variance at all.
        let flat = bootstrap_net_r(&[1.0; 25], 500, 1).unwrap();
        assert_eq!(flat, (25.0, 25.0));
        assert!(bootstrap_net_r(&[], 2000, 1).is_none());
    }

    #[test]
    fn bootstrap_difference_against_control() {
        let good = vec![1.0; 30];
        let coin: Vec<f64> = (0..30).map(|i| if i % 2 == 0 { 1.0 } else { -1.0 }).collect();
        let (lo, hi) = bootstrap_diff_r(&good, &coin, 2000, 3).unwrap();
        assert!(lo > 0.0, "a sure +1R trader beats a coin flip: {lo} {hi}");
        let (lo, hi) = bootstrap_diff_r(&coin, &coin, 2000, 3).unwrap();
        assert!(lo < 0.0 && hi > 0.0, "a coin flip does not beat itself: {lo} {hi}");
    }

    #[test]
    fn verdict_rules() {
        assert_eq!(verdict(true, 100, None), Verdict::Control);
        assert_eq!(verdict(false, MIN_TRADES_FOR_VERDICT - 1, Some((1.0, 2.0))), Verdict::TooFewTrades);
        assert_eq!(verdict(false, MIN_TRADES_FOR_VERDICT, Some((0.1, 2.0))), Verdict::Yes);
        assert_eq!(verdict(false, MIN_TRADES_FOR_VERDICT, Some((-0.1, 2.0))), Verdict::No);
        assert_eq!(verdict(false, MIN_TRADES_FOR_VERDICT, None), Verdict::NoControl);
    }

    #[test]
    fn report_assembles_breakdowns_curves_and_days() {
        let mut trades = Vec::new();
        // Grace: London XAG +1, Overlap NAS -0.5, next day Asia XAG +2.
        trades.push(rt(Archetype::TrendRider, "XAG/USD", 8 * 60, 30, 1.0, 10.0, 1.0));
        trades.push(rt(Archetype::TrendRider, "NAS100", 13 * 60, 10, -0.5, 10.0, 1.0));
        trades.push(rt(Archetype::TrendRider, "XAG/USD", 24 * 60 + 60, 10, 2.0, 10.0, 1.0));
        // Colin: one flat trade.
        trades.push(rt(Archetype::Control, "XAG/USD", 9 * 60, 10, 0.0, 10.0, 1.0));
        let n = Archetype::ALL.len();
        let rep = Report::build("test", trades, T0, T0 + 2 * 86_400_000, n, 1);
        assert_eq!(rep.traders.len(), n, "every archetype has a row");
        let grace = rep.traders.iter().find(|t| t.name == "Grace").unwrap();
        assert_eq!(grace.stats.trades, 3);
        assert!((grace.stats.net_r - 2.5).abs() < 1e-6);
        assert_eq!(grace.by_symbol.iter().map(|b| (b.key.as_str(), b.trades)).collect::<Vec<_>>(), vec![("NAS100", 1), ("XAG/USD", 2)]);
        let sessions: Vec<(&str, usize)> = grace.by_session.iter().map(|b| (b.key.as_str(), b.trades)).collect();
        assert_eq!(sessions, vec![("Asia", 1), ("London", 1), ("Overlap", 1)]);
        assert_eq!(grace.edge.verdict, Some(Verdict::TooFewTrades));
        // Curve: start, then cumulative by exit: +1, +0.5, +2.5.
        assert_eq!(grace.curve.len(), 4);
        assert!((grace.curve[1].1 - 1.0).abs() < 1e-9 && (grace.curve[3].1 - 2.5).abs() < 1e-9);
        assert!((Report::curve_at(&grace.curve, T0 + 12 * 3_600_000) - 1.0).abs() < 1e-9);
        assert!(rep.spans_days());
        assert_eq!(rep.days.len(), 2);
        assert_eq!(rep.days[0].day, "2026-10-22");
        let gi = rep.traders.iter().position(|t| t.name == "Grace").unwrap();
        assert_eq!(rep.days[0].cells[gi], (2, 0.5));
        assert_eq!(rep.days[1].cells[gi].0, 1);
        assert_eq!(rep.traders[0].name, "Grace", "leaderboard order by net R");
        assert!(rep.control.is_some());
        let colin = &rep.traders[rep.control.unwrap()];
        assert_eq!(colin.edge.verdict, Some(Verdict::Control));
        // Renders without panicking and mentions the verdict line.
        let text = rep.text();
        assert!(text.contains("beats control after costs"));
        assert!(text.contains("BY DAY"));
        let html = rep.html();
        assert!(html.starts_with("<!doctype html>"));
        assert!(html.contains("<svg"));
        assert!(!html.contains("http://") && !html.contains("https://"), "self-contained");
    }

    #[test]
    fn verdict_yes_in_a_full_report() {
        let mut trades = series(Archetype::IctLevelTrader, &[1.0; 30]);
        trades.extend(series(Archetype::Control, &[1.0, -1.0, 1.0, -1.0, 1.0, -1.0, 1.0, -1.0, 1.0, -1.0, 1.0, -1.0, 1.0, -1.0, 1.0, -1.0, 1.0, -1.0, 1.0, -1.0, 1.0, -1.0]));
        let rep = Report::build("t", trades, T0, T0 + 3_600_000, 6, 42);
        let priya = rep.traders.iter().find(|t| t.name == "Priya").unwrap();
        assert_eq!(priya.edge.verdict, Some(Verdict::Yes));
        assert_eq!(priya.edge.boot_net_r, Some((30.0, 30.0)));
        // A constant series has no Sharpe, so no DSR; the control's does exist.
        assert!(priya.edge.dsr.is_none());
        let colin = &rep.traders[rep.control.unwrap()];
        assert!(colin.edge.dsr.is_some());
    }

    #[test]
    fn trade_roundtrips_through_jsonl() {
        let t = Trade {
            trader: "Grace",
            archetype: Archetype::TrendRider,
            symbol: "NAS100",
            side: "short",
            entry: 100.0,
            exit: 99.0,
            opened_ms: T0,
            closed_ms: T0 + 90_000,
            pnl_ticks: 10.0,
            pnl_gbp: 10.0,
            pnl_r: 0.5,
            stop_ticks: 20.0,
            spread_ticks_in: 2.0,
            why_in: "first pullback".into(),
            why_out: "target <x>".into(),
            observation_hash: "abc".into(),
            read_source: Some(ReadSource::Jev),
            genome: None,
        };
        let line = serde_json::to_string(&t).unwrap();
        let (parsed, bad) = parse_jsonl(&format!("{line}\n\nnot json\n"));
        assert_eq!(bad, 1);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0], ReportTrade::from(&t));
        assert_eq!(parsed[0].archetype, "TrendRider");
        assert_eq!(parsed[0].read_source.as_deref(), Some("Jev"));
        assert!((parsed[0].hold_minutes() - 1.5).abs() < 1e-9);
        assert!((parsed[0].spread_r() - 0.1).abs() < 1e-9);
        assert_eq!(esc(&parsed[0].why_out), "target &lt;x&gt;");
    }

    #[test]
    fn time_formatting() {
        assert_eq!(fmt_ms(T0), "2026-10-22T00:00:00Z");
        assert_eq!(fmt_day(T0 - 1), "2026-10-21");
        assert_eq!(hhmm(T0 + (16 * 60 + 30) * 60_000), "16:30");
        assert_eq!(fmt_ms(0), "1970-01-01T00:00:00Z");
    }

    #[test]
    fn sparkline_and_ticks() {
        let curve = vec![(T0, 0.0), (T0 + 1_000, 2.0), (T0 + 2_000, -1.0)];
        let (line, lo, hi) = sparkline(&curve, T0, T0 + 3_000);
        assert_eq!(line.chars().count(), SPARK_COLS);
        assert_eq!((lo, hi), (-1.0, 2.0));
        // First sample is still at 0 R (▃ on a -1..2 scale), the +2 plateau is full height, the end is the trough.
        assert!(line.starts_with('▃') && line.contains('█') && line.ends_with('▁'), "{line}");
        let t = nice_ticks(-1.2, 3.4);
        assert!(t.contains(&0.0) && t.iter().all(|v| *v >= -1.2 && *v <= 3.4));
    }
}
