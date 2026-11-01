//! What a fade at a level actually pays, as a trade rather than as a reaction.
//!
//! The reaction prior counts a sweep (price through the far edge and back) as a hold, which is
//! right for "did the level survive" and wrong for "did my stop survive". This walks the same
//! cached 1m bars through the same level and touch code and, on every touch of a level, takes
//! the fade: enter at the touch bar's close, stop a buffer beyond the far edge (never under
//! `paper::MIN_STOP_SPREADS` spreads), target a multiple of the risk, out at market after an
//! hour. Stop first when a bar spans both. Spread is charged once, in R.
//!
//! Reads `replay_out/bars_cache` only (run `reaction_prior` first to fill it).
//!
//! cargo run --release --bin level_fade [-- --symbols XAUUSD,US30]

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    if let Err(e) = fade::run() {
        eprintln!("level_fade: {e}");
        std::process::exit(1);
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod fade {
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::PathBuf;

    use opencry_visual::candles::Candle;
    use opencry_visual::levels::{compose, detect_levels, round_step, tape_symbol, LEVEL_NODES};
    use opencry_visual::market::Instrument;
    use opencry_visual::paper::MIN_STOP_SPREADS;
    use opencry_visual::pit_engine::tick_size;
    use opencry_visual::reaction::{unit_ticks, Side, Step, TouchTracker};
    use opencry_visual::report::Session;
    use opencry_visual::tape_chart::BOARD_BARS;

    /// Stop buffers beyond the far edge, in units.
    const BUFFERS: [f32; 4] = [0.0, 0.5, 1.0, 2.0];
    /// Targets, in R.
    const TARGETS: [f32; 3] = [1.0, 1.5, 2.0];
    const HOLD_BARS: usize = 60;

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
        timed: u32,
        r: f64,
    }

    impl Acc {
        fn add(&mut self, r: f32, won: bool, timed: bool) {
            self.n += 1;
            self.wins += won as u32;
            self.timed += timed as u32;
            self.r += r as f64;
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
        if tf <= 1 {
            return bars.to_vec();
        }
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

    /// One fade: R after spread, whether it hit target, whether it timed out.
    fn trade(bars: &[Candle], i: usize, long: bool, entry: f32, stop: f32, target_r: f32, spread: f32, tick: f32) -> (f32, bool, bool) {
        let risk = (entry - stop).abs();
        let target = if long { entry + target_r * risk } else { entry - target_r * risk };
        let cost = spread * tick / risk;
        let end = (i + HOLD_BARS).min(bars.len() - 1);
        for b in &bars[i + 1..=end] {
            let (hit_stop, hit_target) = if long { (b.low <= stop, b.high >= target) } else { (b.high >= stop, b.low <= target) };
            if hit_stop {
                return (-1.0 - cost, false, false);
            }
            if hit_target {
                return (target_r - cost, true, false);
            }
        }
        let c = bars[end].close;
        let r = if long { (c - entry) / risk } else { (entry - c) / risk };
        (r - cost, r > 0.0, true)
    }

    pub fn run() -> Result<(), String> {
        let want: Option<Vec<String>> = std::env::args().skip_while(|a| a != "--symbols").nth(1).map(|s| s.split(',').map(|x| x.trim().to_uppercase()).collect());
        // `--floor 0` drops the minimum stop, to see the level's own stop.
        let floor: f32 = std::env::args().skip_while(|a| a != "--floor").nth(1).map(|s| s.parse().unwrap_or(MIN_STOP_SPREADS)).unwrap_or(MIN_STOP_SPREADS);
        // `--tf 15` builds 15m bars from the 1m cache; levels, units and the hold are then in those bars.
        let tf: i64 = std::env::args().skip_while(|a| a != "--tf").nth(1).and_then(|s| s.parse().ok()).unwrap_or(1);
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        // (config, group) -> acc
        let mut acc: BTreeMap<(String, String), Acc> = BTreeMap::new();
        for inst in Instrument::ALL.iter().copied().filter(|i| *i != Instrument::Bitcoin) {
            let sym = tape_symbol(inst);
            if want.as_ref().is_some_and(|w| !w.contains(&sym)) {
                continue;
            }
            let bars = aggregate(&read_cache(&manifest.join(format!("replay_out/bars_cache/{sym}_SB_1m.txt")))?, tf);
            let (tick, round, spread) = (tick_size(inst), round_step(inst), spread_ticks(inst));
            let mut tracker = TouchTracker::default();
            let mut touches = 0u64;
            for i in 1..bars.len().saturating_sub(HOLD_BARS + 1) {
                let start = i + 1 - BOARD_BARS.min(i + 1);
                let window = &bars[start..=i];
                let levels = compose(detect_levels(window, tick, round), LEVEL_NODES * 2);
                let unit = unit_ticks(window, tick);
                let bar = &bars[i];
                let ts = tracker.step(&levels, Step::bar(bar), bar.time_ms, bar.time_ms, tick, unit);
                if ts.is_empty() || bars[i + HOLD_BARS].time_ms - bar.time_ms > 3 * HOLD_BARS as i64 * tf * 60_000 {
                    continue;
                }
                for t in ts {
                    touches += 1;
                    let long = t.side == Side::FromAbove;
                    let entry = bar.close;
                    let fresh = levels.iter().find(|l| l.kind == t.kind && l.lo == t.lo && l.hi == t.hi).map(|l| !l.touched).unwrap_or(false);
                    let back = bars[i.saturating_sub(5)].close;
                    let speed = ((entry - back).abs() / tick) / unit;
                    let approach = if speed >= 2.0 { "fast" } else { "slow" };
                    let session = format!("{:?}", Session::of_ms(bar.time_ms));
                    let kind = format!("{} {}", t.kind.tag(), t.side.label());
                    for &buf in &BUFFERS {
                        let far = if long { t.lo - buf * unit * tick } else { t.hi + buf * unit * tick };
                        let min_risk = (floor * spread).max(1.0) * tick;
                        let stop = if long { far.min(entry - min_risk) } else { far.max(entry + min_risk) };
                        if (long && entry <= stop) || (!long && entry >= stop) {
                            continue;
                        }
                        for &tr in &TARGETS {
                            let (r, won, timed) = trade(&bars, i, long, entry, stop, tr, spread, tick);
                            let cfg = format!("stop +{buf:.1}u target {tr:.1}R");
                            for g in [
                                format!("kind  {kind}"),
                                format!("kfrsh {kind} {}", if fresh { "untouched" } else { "touched" }),
                                format!("kappr {kind} {approach}"),
                                format!("ksym  {kind} {sym}"),
                                format!("ksess {kind} {session}"),
                                format!("all"),
                            ] {
                                acc.entry((cfg.clone(), g)).or_default().add(r, won, timed);
                            }
                        }
                    }
                }
            }
            eprintln!("{sym}: {} bars, {touches} touches", bars.len());
        }
        let mut cfgs: Vec<&String> = acc.keys().map(|(c, _)| c).collect();
        cfgs.dedup();
        println!("LEVEL FADE  every touch, fade it: enter at the touch bar close, stop beyond the far edge (>= {floor} spreads), out after {HOLD_BARS} bars of {tf}m; R after spread\n");
        println!("== every config, all touches and the best kinds");
        for c in &cfgs {
            for ((cc, g), a) in &acc {
                if cc == *c && (g == "all" || (g.starts_with("kind ") && a.n >= 1000)) {
                    println!("{c:<22} {g:<36} n {:>7}  win {:>3.0}%  timed {:>3.0}%  R/tr {:+.3}", a.n, 100.0 * a.wins as f64 / a.n as f64, 100.0 * a.timed as f64 / a.n as f64, a.r / a.n as f64);
                }
            }
        }
        println!("\n== every cell with n >= 300, ranked by R per trade (top 60)");
        let mut cells: Vec<(&(String, String), &Acc)> = acc.iter().filter(|(_, a)| a.n >= 300).collect();
        cells.sort_by(|a, b| (b.1.r / b.1.n as f64).partial_cmp(&(a.1.r / a.1.n as f64)).unwrap());
        for ((c, g), a) in cells.iter().take(60) {
            println!("{c:<22} {g:<44} n {:>7}  win {:>3.0}%  R/tr {:+.3}", a.n, 100.0 * a.wins as f64 / a.n as f64, a.r / a.n as f64);
        }
        Ok(())
    }
}
