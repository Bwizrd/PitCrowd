//! The reaction prior: a year of 1m bars, every level the floor would have marked, every touch
//! of one, and what price did in the minutes after. Writes `assets/reaction_prior.json`, which
//! the library compiles in (`reaction::PriorTable::builtin`), and a readable report.
//!
//!   cargo run --release --bin reaction_prior                      # all pits, all history
//!   cargo run --release --bin reaction_prior -- --symbols US500,XAUUSD --from 2026-06-01
//!   cargo run --release --bin reaction_prior -- --days 90 --out /tmp/prior.json
//!
//! Flags: --symbols all|US500,... (Bitcoin has no bars and is skipped), --from / --to ISO dates
//! (default 2025-09-18, the start of the 1m history, to now), --days N (instead of --from),
//! --horizons 5,15 (minutes), --out <json> (default assets/reaction_prior.json),
//! --report-dir <dir> (default replay_out/reaction_prior_<stamp>), --no-cache.
//!
//! Method, the same code the live desk runs (`reaction.rs`): at every bar the levels are
//! detected from the last 60 bars (`levels::detect_levels` + `compose`, as the paper traders
//! see them), the bar is fed to a `TouchTracker`, and each touch is classified by the path of
//! the next H bars (`reaction::classify`). A touch whose path spans a gap longer than three
//! times the horizon (a weekend) is dropped. Bars come from InfluxDB `market_data/trendbar`
//! and are cached under `replay_out/bars_cache/` as plain text.

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    if let Err(e) = prior::run() {
        eprintln!("reaction_prior: {e}");
        std::process::exit(1);
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod prior {
    use std::collections::BTreeMap;
    use std::fs;
    use std::io::Write;
    use std::path::PathBuf;
    use std::time::Instant;

    use opencry_visual::candles::{parse_iso_ms, wall_clock_ms, Candle};
    use opencry_visual::influx;
    use opencry_visual::levels::{compose, detect_levels, round_step, tape_symbol, LevelKind, LEVEL_NODES};
    use opencry_visual::market::Instrument;
    use opencry_visual::pit_engine::tick_size;
    use opencry_visual::reaction::{classify, unit_ticks, wilson, Outcome, PriorTable, Side, Step, Tally, TouchTracker, PRIOR_HORIZONS_MIN};
    use opencry_visual::report::Session;
    use opencry_visual::tape_chart::BOARD_BARS;

    const HISTORY_START: &str = "2025-09-18T00:00:00Z";

    fn arg(name: &str) -> Option<String> {
        let a: Vec<String> = std::env::args().collect();
        a.iter().position(|x| x == name).and_then(|i| a.get(i + 1).cloned())
    }

    fn flag(name: &str) -> bool {
        std::env::args().any(|x| x == name)
    }

    fn parse_when(s: &str) -> Result<i64, String> {
        let full = if s.len() == 10 { format!("{s}T00:00:00Z") } else { s.to_string() };
        parse_iso_ms(&full).ok_or(format!("bad time {s}; want YYYY-MM-DD or RFC3339"))
    }

    fn stamp(ms: i64) -> String {
        let s = ms.div_euclid(1000);
        let (days, rem) = (s.div_euclid(86_400), s.rem_euclid(86_400));
        // civil from days (Howard Hinnant)
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let y = yoe + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = doy - (153 * mp + 2) / 5 + 1;
        let m = if mp < 10 { mp + 3 } else { mp - 9 };
        let y = if m <= 2 { y + 1 } else { y };
        format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
    }

    // -----------------------------------------------------------------------
    // Bars, cached
    // -----------------------------------------------------------------------

    fn cache_path(dir: &PathBuf, sb: &str) -> PathBuf {
        dir.join(format!("{sb}_1m.txt"))
    }

    fn read_cache(path: &PathBuf, from_ms: i64, to_ms: i64) -> Option<Vec<Candle>> {
        let text = fs::read_to_string(path).ok()?;
        let mut lines = text.lines();
        let head = lines.next()?;
        let mut hp = head.split_whitespace();
        if hp.next()? != "range" {
            return None;
        }
        let (a, b): (i64, i64) = (hp.next()?.parse().ok()?, hp.next()?.parse().ok()?);
        if a > from_ms || b < to_ms {
            return None;
        }
        let mut out = Vec::new();
        for l in lines {
            let mut p = l.split_whitespace();
            let (Some(t), Some(o), Some(h), Some(lo), Some(c), Some(v)) = (p.next(), p.next(), p.next(), p.next(), p.next(), p.next()) else { continue };
            let t: i64 = t.parse().ok()?;
            if t < from_ms || t >= to_ms {
                continue;
            }
            out.push(Candle { time_ms: t, open: o.parse().ok()?, high: h.parse().ok()?, low: lo.parse().ok()?, close: c.parse().ok()?, volume: v.parse().ok()?, delta: 0.0, sweep: 0.0 });
        }
        Some(out)
    }

    fn write_cache(path: &PathBuf, from_ms: i64, to_ms: i64, bars: &[Candle]) {
        if let Some(d) = path.parent() {
            let _ = fs::create_dir_all(d);
        }
        let mut s = String::with_capacity(bars.len() * 48);
        s.push_str(&format!("range {from_ms} {to_ms}\n"));
        for b in bars {
            s.push_str(&format!("{} {} {} {} {} {}\n", b.time_ms, b.open, b.high, b.low, b.close, b.volume));
        }
        let _ = fs::write(path, s);
    }

    // -----------------------------------------------------------------------
    // The walk
    // -----------------------------------------------------------------------

    pub struct Walk {
        touches: u64,
        pending: u64,
        gapped: u64,
    }

    /// Feed one instrument's bars through the live level and touch code, adding every resolved
    /// touch to `table`.
    pub fn walk(inst: Instrument, bars: &[Candle], horizons: &[u32], table: &mut PriorTable) -> Walk {
        let tick = tick_size(inst);
        let round = round_step(inst);
        let sym = tape_symbol(inst);
        let mut tracker = TouchTracker::default();
        let mut w = Walk { touches: 0, pending: 0, gapped: 0 };
        let max_h = *horizons.iter().max().unwrap_or(&15) as usize;
        for i in 1..bars.len() {
            let start = i + 1 - BOARD_BARS.min(i + 1);
            let window = &bars[start..=i];
            let levels = compose(detect_levels(window, tick, round), LEVEL_NODES * 2);
            let unit = unit_ticks(window, tick);
            let bar = &bars[i];
            let touches = tracker.step(&levels, Step::bar(bar), bar.time_ms, bar.time_ms, tick, unit);
            if touches.is_empty() {
                continue;
            }
            let session = Session::of_ms(bar.time_ms);
            for t in touches {
                w.touches += 1;
                if i + max_h >= bars.len() {
                    w.pending += 1;
                    continue;
                }
                for &h in horizons {
                    let h = h as usize;
                    let end = &bars[i + h];
                    if end.time_ms - bar.time_ms > 3 * h as i64 * 60_000 {
                        w.gapped += 1;
                        continue;
                    }
                    let path: Vec<Step> = bars[i + 1..=i + h].iter().map(Step::bar).collect();
                    let o = classify(&t, &path, tick, unit);
                    table.add(&sym, t.kind, t.side, session, h as u32, o);
                }
            }
        }
        w
    }

    // -----------------------------------------------------------------------
    // Report
    // -----------------------------------------------------------------------

    fn pct(x: f32) -> String {
        format!("{:>3.0}%", x * 100.0)
    }

    fn row(label: &str, t: &Tally) -> String {
        let (lo, hi) = wilson(t.n - t.brk, t.n);
        format!(
            "{label:<34} n {:>6}  reject {}  break {}  sweep {}  stall {}   hold {} ({}-{})",
            t.n,
            pct(t.rate(Outcome::Reject)),
            pct(t.rate(Outcome::Break)),
            pct(t.rate(Outcome::Sweep)),
            pct(t.rate(Outcome::Stall)),
            pct(t.hold_rate()),
            pct(lo),
            pct(hi)
        )
    }

    pub fn report(table: &PriorTable, horizons: &[u32], symbols: &[Instrument]) -> String {
        let mut out = String::new();
        let mut w = |s: String| {
            out.push_str(&s);
            out.push('\n');
        };
        w(format!("REACTION PRIOR  {} .. {}  ({} bars)   built {}", table.from, table.to, table.bars, table.built_at));
        w("A touch is price arriving at a level from outside; the outcome is the path of the next H minutes.".into());
        w("hold = anything but a break, with its Wilson 95% interval.".into());
        let kinds = [
            LevelKind::BullOrderBlock,
            LevelKind::BearOrderBlock,
            LevelKind::BullFvg,
            LevelKind::BearFvg,
            LevelKind::SwingHigh,
            LevelKind::SwingLow,
            LevelKind::SessionHigh,
            LevelKind::SessionLow,
            LevelKind::Round,
        ];
        for &h in horizons {
            w(String::new());
            w(format!("== {h} minutes after the touch"));
            w(row("ALL TOUCHES", &table.tally(None, None, None, None, h)));
            w(String::new());
            w("-- by level kind and approach side (all pits, all sessions)".into());
            for k in kinds {
                for side in [Side::FromAbove, Side::FromBelow] {
                    let t = table.tally(None, Some(k.tag()), Some(side), None, h);
                    if t.n > 0 {
                        w(row(&format!("{} {}", k.tag(), side.label()), &t));
                    }
                }
            }
            w(String::new());
            w("-- by pit".into());
            for inst in symbols {
                let t = table.tally(Some(&tape_symbol(*inst)), None, None, None, h);
                if t.n > 0 {
                    w(row(&tape_symbol(*inst), &t));
                }
            }
            w(String::new());
            w("-- by session (UTC)".into());
            for s in Session::ALL {
                let t = table.tally(None, None, None, Some(s), h);
                if t.n > 0 {
                    w(row(&format!("{} {}", s.label(), s.hours()), &t));
                }
            }
        }
        // The cells that differ most from the base rate at the longest horizon: where the
        // prior actually says something.
        if let Some(&h) = horizons.iter().max() {
            let base = table.tally(None, None, None, None, h).hold_rate();
            w(String::new());
            w(format!("== pit x kind x side cells at {h} min with n >= 100, ranked by hold rate against the base {}", pct(base)));
            let mut cells: Vec<(String, Tally)> = Vec::new();
            for inst in symbols {
                let sym = tape_symbol(*inst);
                for k in kinds {
                    for side in [Side::FromAbove, Side::FromBelow] {
                        let t = table.tally(Some(&sym), Some(k.tag()), Some(side), None, h);
                        if t.n >= 100 {
                            cells.push((format!("{sym} {} {}", k.tag(), side.label()), t));
                        }
                    }
                }
            }
            cells.sort_by(|a, b| b.1.hold_rate().partial_cmp(&a.1.hold_rate()).unwrap());
            let n = cells.len();
            for (label, t) in cells.iter().take(12) {
                w(row(label, t));
            }
            if n > 24 {
                w("   ...".into());
            }
            for (label, t) in cells.iter().skip(n.saturating_sub(12).max(12.min(n))) {
                w(row(label, t));
            }
        }
        out
    }

    // -----------------------------------------------------------------------
    // Run
    // -----------------------------------------------------------------------

    pub fn run() -> Result<(), String> {
        let t0 = Instant::now();
        let symbols: Vec<Instrument> = match arg("--symbols").as_deref() {
            None | Some("all") => Instrument::ALL.iter().copied().filter(|i| *i != Instrument::Bitcoin).collect(),
            Some(list) => list
                .split(',')
                .map(|s| {
                    let want = s.trim().to_uppercase().replace('/', "");
                    Instrument::ALL.iter().copied().find(|i| tape_symbol(*i) == want).ok_or(format!("unknown symbol {s}"))
                })
                .collect::<Result<_, _>>()?,
        };
        let to_ms = match arg("--to") {
            Some(s) => parse_when(&s)?,
            None => wall_clock_ms(),
        };
        let from_ms = match (arg("--from"), arg("--days")) {
            (Some(s), _) => parse_when(&s)?,
            (None, Some(d)) => to_ms - d.parse::<i64>().map_err(|e| format!("--days: {e}"))? * 86_400_000,
            (None, None) => parse_when(HISTORY_START)?,
        };
        let horizons: Vec<u32> = match arg("--horizons") {
            Some(s) => s.split(',').map(|x| x.trim().parse::<u32>().map_err(|e| format!("--horizons: {e}"))).collect::<Result<_, _>>()?,
            None => PRIOR_HORIZONS_MIN.to_vec(),
        };
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let out_path = arg("--out").map(PathBuf::from).unwrap_or_else(|| manifest.join("assets/reaction_prior.json"));
        let cache_dir = manifest.join("replay_out/bars_cache");
        let report_dir = arg("--report-dir").map(PathBuf::from).unwrap_or_else(|| manifest.join(format!("replay_out/reaction_prior_{}", stamp(wall_clock_ms()).replace([':', '-'], "").trim_end_matches('Z'))));
        fs::create_dir_all(&report_dir).map_err(|e| format!("{}: {e}", report_dir.display()))?;
        let use_cache = !flag("--no-cache");

        let cfg = influx::config()?;
        let mut table = PriorTable { built_at: stamp(wall_clock_ms()), from: stamp(from_ms), to: stamp(to_ms), bars: 0, rows: Vec::new() };
        let mut per_symbol: BTreeMap<String, (usize, Walk)> = BTreeMap::new();
        for inst in &symbols {
            let sb = format!("{}_SB", tape_symbol(*inst));
            let cache = cache_path(&cache_dir, &sb);
            let bars = match if use_cache { read_cache(&cache, from_ms, to_ms) } else { None } {
                Some(b) => {
                    eprintln!("{sb}: {} bars from cache", b.len());
                    b
                }
                None => {
                    eprint!("{sb}: loading 1m bars from InfluxDB ");
                    let b = influx::load_trendbars(&cfg, &sb, "1m", from_ms, to_ms, |k, n| {
                        if k % 8 == 0 || k == n {
                            eprint!("{k}/{n} ");
                        }
                    })?;
                    eprintln!("-> {} bars", b.len());
                    write_cache(&cache, from_ms, to_ms, &b);
                    b
                }
            };
            let t1 = Instant::now();
            let w = walk(*inst, &bars, &horizons, &mut table);
            eprintln!("{sb}: {} touches ({} still open, {} across a gap) in {:.1}s", w.touches, w.pending, w.gapped, t1.elapsed().as_secs_f64());
            table.bars += bars.len() as u64;
            per_symbol.insert(sb, (bars.len(), w));
        }

        let json = serde_json::to_string(&table).map_err(|e| e.to_string())?;
        fs::write(&out_path, &json).map_err(|e| format!("{}: {e}", out_path.display()))?;
        fs::write(report_dir.join("reaction_prior.json"), &json).map_err(|e| e.to_string())?;
        let mut text = report(&table, &horizons, &symbols);
        text.push('\n');
        for (sb, (n, w)) in &per_symbol {
            text.push_str(&format!("{sb:<12} bars {n:>8}  touches {:>7}  open {:>4}  gapped {:>5}\n", w.touches, w.pending, w.gapped));
        }
        text.push_str(&format!("\nwrote {} ({} cells) in {:.0}s\n", out_path.display(), table.rows.len(), t0.elapsed().as_secs_f64()));
        let mut f = fs::File::create(report_dir.join("report.txt")).map_err(|e| e.to_string())?;
        let _ = f.write_all(text.as_bytes());
        print!("{text}");
        println!("report: {}", report_dir.join("report.txt").display());
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        /// Twenty flat bars, a swing high at 48.30 (bar 10), a pullback and a return to it that
        /// holds: one touch from below, rejected at both horizons.
        #[test]
        fn a_return_to_a_swing_high_that_holds_is_one_rejected_touch() {
            let mut bars: Vec<Candle> = Vec::new();
            let mut px = 48.00f32;
            for i in 0..80i64 {
                let (o, h, l, c) = match i {
                    10 => (48.20, 48.30, 48.18, 48.22),
                    11..=14 => (48.20, 48.24, 48.10, 48.12),
                    15 => (48.12, 48.30, 48.12, 48.29), // back up into the swing high
                    16..=40 => (48.20, 48.26, 48.14, 48.16), // holds, drifts away
                    _ => {
                        px += 0.0;
                        (px, px + 0.04, px - 0.04, px)
                    }
                };
                bars.push(Candle { time_ms: i * 60_000, open: o, high: h, low: l, close: c, volume: 1.0, delta: 0.0, sweep: 0.0 });
            }
            let mut table = PriorTable::default();
            let w = walk(Instrument::Silver, &bars, &[5, 15], &mut table);
            let t = table.tally(Some("XAGUSD"), Some("SWING HI"), Some(Side::FromBelow), None, 15);
            assert!(t.n >= 1, "{w:?} {table:?}");
            assert_eq!(t.reject, t.n, "the swing high held: {t:?}");
            assert_eq!(table.tally(None, Some("SWING HI"), Some(Side::FromBelow), None, 15).brk, 0);
            let r = report(&table, &[5, 15], &[Instrument::Silver]);
            assert!(r.contains("SWING HI from below") && r.contains("XAGUSD"), "{r}");
        }
    }

    impl std::fmt::Debug for Walk {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "touches {} pending {} gapped {}", self.touches, self.pending, self.gapped)
        }
    }
}
