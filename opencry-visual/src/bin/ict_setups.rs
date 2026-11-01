//! The ICT setups as trades, before Jev is asked anything about them.
//!
//! A setup is three things in order, all from bars:
//!
//! 1. **Sweep**: a bar trades through the extreme of the last `LOOKBACK` bars (a resting high or
//!    low, at least three bars old) and closes back inside: the stops there have been taken.
//! 2. **Displacement**: within `DISPLACE_BARS` bars, price moves hard the other way and leaves a
//!    fair value gap whose middle bar's body is at least a unit (mean range of twenty bars).
//!    It is a market-structure shift as well when a close in that move clears the last swing
//!    on the other side.
//! 3. **Return**: within `RETURN_BARS` bars, price comes back to the gap's near edge, where a
//!    limit fills. The stop sits beyond the sweep's extreme (never under
//!    `paper::MIN_STOP_SPREADS` spreads), the target a multiple of the risk, out at market
//!    after `HOLD_BARS` bars. Stop first when a bar spans both, the fill bar included.
//!
//! Bars are built from the 1m cache at 5m, 15m and 1h. Results are split at `SPLIT` so a
//! filter that only worked in the first nine months shows up as one.
//!
//! cargo run --release --bin ict_setups

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    if let Err(e) = ict::run() {
        eprintln!("ict_setups: {e}");
        std::process::exit(1);
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod ict {
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::PathBuf;

    use opencry_visual::candles::{parse_iso_ms, Candle};
    use opencry_visual::levels::tape_symbol;
    use opencry_visual::market::Instrument;
    use opencry_visual::paper::MIN_STOP_SPREADS;
    use opencry_visual::pit_engine::tick_size;
    use opencry_visual::report::Session;

    const TIMEFRAMES: [i64; 3] = [5, 15, 60];
    const LOOKBACK: usize = 20;
    const DISPLACE_BARS: usize = 4;
    const RETURN_BARS: usize = 10;
    const HOLD_BARS: usize = 40;
    const TARGETS: [f32; 3] = [1.0, 2.0, 3.0];
    /// In-sample before, out-of-sample from.
    const SPLIT: &str = "2026-07-01T00:00:00Z";

    /// Typical live spread in ticks (median of the desk's fills, 23-24 Oct 2026).
    fn spread_ticks(inst: Instrument) -> f32 {
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

    #[derive(Default, Clone, Copy)]
    struct Acc {
        n: u32,
        wins: u32,
        r: f64,
    }

    impl Acc {
        fn add(&mut self, r: f32) {
            self.n += 1;
            self.wins += (r > 0.0) as u32;
            self.r += r as f64;
        }
        fn show(&self) -> String {
            if self.n == 0 {
                return format!("{:>22}", "-");
            }
            format!("n {:>5} win {:>3.0}% R {:+.3}", self.n, 100.0 * self.wins as f64 / self.n as f64, self.r / self.n as f64)
        }
    }

    fn read_cache(path: &PathBuf) -> Result<Vec<Candle>, String> {
        let text = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut out = Vec::new();
        for l in text.lines().skip(1) {
            let v: Vec<&str> = l.split_whitespace().collect();
            if v.len() < 6 {
                continue;
            }
            let p = |i: usize| v[i].parse::<f32>().unwrap_or(0.0);
            out.push(Candle { time_ms: v[0].parse().unwrap_or(0), open: p(1), high: p(2), low: p(3), close: p(4), volume: p(5), delta: 0.0, sweep: 0.0 });
        }
        Ok(out)
    }

    /// 1m bars into `tf`-minute bars aligned on the clock.
    fn aggregate(bars: &[Candle], tf: i64) -> Vec<Candle> {
        let span = tf * 60_000;
        let mut out: Vec<Candle> = Vec::new();
        for b in bars {
            let t = b.time_ms - b.time_ms.rem_euclid(span);
            match out.last_mut() {
                Some(c) if c.time_ms == t => {
                    c.high = c.high.max(b.high);
                    c.low = c.low.min(b.low);
                    c.close = b.close;
                    c.volume += b.volume;
                }
                _ => out.push(Candle { time_ms: t, ..*b }),
            }
        }
        out
    }

    /// One setup, found: which way it trades and the prices it trades at.
    struct Setup {
        long: bool,
        /// Bar the gap completed on; the limit rests from the next bar.
        gap_bar: usize,
        entry: f32,
        sweep_extreme: f32,
        mss: bool,
        sweep_ms: i64,
    }

    fn mean_range(bars: &[Candle]) -> f32 {
        bars.iter().map(|b| b.high - b.low).sum::<f32>() / bars.len().max(1) as f32
    }

    /// Every setup in the series. A sweep produces at most one.
    fn find(bars: &[Candle]) -> Vec<Setup> {
        let mut out = Vec::new();
        for i in LOOKBACK..bars.len().saturating_sub(DISPLACE_BARS + 1) {
            let back = &bars[i - LOOKBACK..i];
            let unit = mean_range(back);
            if unit <= 0.0 {
                continue;
            }
            let b = &bars[i];
            let (lo_at, lo) = back.iter().enumerate().fold((0, f32::MAX), |a, (k, x)| if x.low < a.1 { (k, x.low) } else { a });
            let (hi_at, hi) = back.iter().enumerate().fold((0, f32::MIN), |a, (k, x)| if x.high > a.1 { (k, x.high) } else { a });
            for long in [true, false] {
                let swept = if long { lo_at + 3 <= LOOKBACK && b.low < lo && b.close > lo } else { hi_at + 3 <= LOOKBACK && b.high > hi && b.close < hi };
                if !swept {
                    continue;
                }
                // The swing on the other side: the extreme of the last ten bars before the sweep.
                let other = if long { bars[i - 10..i].iter().map(|x| x.high).fold(f32::MIN, f32::max) } else { bars[i - 10..i].iter().map(|x| x.low).fold(f32::MAX, f32::min) };
                let extreme = if long { b.low } else { b.high };
                for j in (i + 2)..=(i + DISPLACE_BARS).min(bars.len() - 1) {
                    let (a, m, c) = (&bars[j - 2], &bars[j - 1], &bars[j]);
                    // The sweep must stay the extreme.
                    if (long && bars[i + 1..=j].iter().any(|x| x.low < extreme)) || (!long && bars[i + 1..=j].iter().any(|x| x.high > extreme)) {
                        break;
                    }
                    let gap = if long { c.low > a.high } else { c.high < a.low };
                    let body = if long { m.close - m.open } else { m.open - m.close };
                    if gap && body >= unit {
                        let mss = bars[i..=j].iter().any(|x| if long { x.close > other } else { x.close < other });
                        out.push(Setup { long, gap_bar: j, entry: if long { c.low } else { c.high }, sweep_extreme: extreme, mss, sweep_ms: b.time_ms });
                        break;
                    }
                }
            }
        }
        out
    }

    /// Fill and resolve: None when the limit never fills.
    fn trade(bars: &[Candle], s: &Setup, stop: f32, target_r: f32, cost_px: f32) -> Option<f32> {
        let risk = (s.entry - stop).abs();
        let target = if s.long { s.entry + target_r * risk } else { s.entry - target_r * risk };
        let last = (s.gap_bar + RETURN_BARS).min(bars.len() - 1);
        let fill = (s.gap_bar + 1..=last).find(|&k| if s.long { bars[k].low <= s.entry } else { bars[k].high >= s.entry })?;
        let cost = cost_px / risk;
        let end = (fill + HOLD_BARS).min(bars.len() - 1);
        for (k, b) in bars.iter().enumerate().take(end + 1).skip(fill) {
            let hit_stop = if s.long { b.low <= stop } else { b.high >= stop };
            // On the fill bar only the stop counts: the bar's path after the fill is unknown.
            let hit_target = k > fill && if s.long { b.high >= target } else { b.low <= target };
            if hit_stop {
                return Some(-1.0 - cost);
            }
            if hit_target {
                return Some(target_r - cost);
            }
        }
        let c = bars[end].close;
        Some((if s.long { c - s.entry } else { s.entry - c }) / risk - cost)
    }

    fn killzone(ms: i64) -> &'static str {
        let m = ms.rem_euclid(86_400_000) / 60_000;
        if (7 * 60..10 * 60).contains(&m) {
            "London open 07-10"
        } else if (13 * 60 + 30..16 * 60).contains(&m) {
            "NY open 13:30-16"
        } else {
            "outside"
        }
    }

    pub fn run() -> Result<(), String> {
        let split = parse_iso_ms(SPLIT).ok_or("bad SPLIT")?;
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        // (group, target, half) -> acc ; half 0 = in-sample, 1 = out-of-sample
        let mut acc: BTreeMap<(String, String, usize), Acc> = BTreeMap::new();
        let mut found: BTreeMap<String, (u32, u32)> = BTreeMap::new();
        for inst in Instrument::ALL.iter().copied().filter(|i| *i != Instrument::Bitcoin) {
            let sym = tape_symbol(inst);
            let m1 = read_cache(&manifest.join(format!("replay_out/bars_cache/{sym}_SB_1m.txt")))?;
            let (tick, spread) = (tick_size(inst), spread_ticks(inst));
            for tf in TIMEFRAMES {
                let bars = aggregate(&m1, tf);
                let setups = find(&bars);
                let mut filled = 0;
                for s in &setups {
                    let unit = mean_range(&bars[s.gap_bar.saturating_sub(LOOKBACK)..s.gap_bar]);
                    let raw = if s.long { s.sweep_extreme - 0.1 * unit } else { s.sweep_extreme + 0.1 * unit };
                    let min_risk = MIN_STOP_SPREADS * spread * tick;
                    let stop = if s.long { raw.min(s.entry - min_risk) } else { raw.max(s.entry + min_risk) };
                    let half = (s.sweep_ms >= split) as usize;
                    let mut any = false;
                    for tr in TARGETS {
                        let Some(r) = trade(&bars, s, stop, tr, spread * tick) else { continue };
                        any = true;
                        let t = format!("{tr:.0}R");
                        let base = format!("{tf:>2}m");
                        for g in [
                            format!("{base} all"),
                            format!("{base} {}", if s.mss { "with MSS" } else { "no MSS" }),
                            format!("{base} {} {}", if s.long { "long" } else { "short" }, if s.mss { "MSS" } else { "-" }),
                            format!("{base} MSS {}", killzone(s.sweep_ms)),
                            format!("{base} MSS {:?}", Session::of_ms(s.sweep_ms)),
                            format!("{base} MSS {sym}"),
                        ] {
                            if g.contains("MSS ") && !g.contains("with") && !s.mss && !g.ends_with('-') {
                                continue;
                            }
                            acc.entry((g, t.clone(), half)).or_default().add(r);
                        }
                    }
                    filled += any as u32;
                }
                let f = found.entry(format!("{tf:>2}m")).or_default();
                f.0 += setups.len() as u32;
                f.1 += filled;
            }
            eprintln!("{sym} done");
        }
        println!("ICT SETUPS  sweep -> displacement gap -> limit at the gap; stop beyond the sweep (>= {MIN_STOP_SPREADS} spreads), {HOLD_BARS}-bar hold, R after spread");
        println!("in-sample before {SPLIT}, out-of-sample after\n");
        for (tf, (n, f)) in &found {
            println!("{tf}: {n} setups, {f} filled");
        }
        let mut groups: Vec<String> = acc.keys().map(|k| k.0.clone()).collect();
        groups.sort();
        groups.dedup();
        println!("\n{:<34} {:<6} {:<34} {}", "group", "target", "in-sample", "out-of-sample");
        for g in &groups {
            for tr in TARGETS {
                let t = format!("{tr:.0}R");
                let a = acc.get(&(g.clone(), t.clone(), 0)).copied().unwrap_or_default();
                let b = acc.get(&(g.clone(), t.clone(), 1)).copied().unwrap_or_default();
                println!("{g:<34} {t:<6} {:<34} {}", a.show(), b.show());
            }
        }
        Ok(())
    }
}
