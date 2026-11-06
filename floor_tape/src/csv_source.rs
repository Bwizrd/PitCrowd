//! Replay your own recorded prices: one CSV per symbol, named after it (`XAUUSD.csv`,
//! `EURUSD.csv`, ...), either ticks or bars, detected from the header.
//!
//!   time,bid,ask                       ticks (ask optional: the symbol's usual spread is added)
//!   time,open,high,low,close[,volume]  bars of any length (each becomes four ticks: open,
//!                                      the nearer extreme, the farther one, close)
//!
//! `time` may be Unix seconds, Unix milliseconds or ISO 8601 in UTC (`2026-10-21T07:00:00Z`,
//! `2026-10-21 07:00:00`); the column may also be called `timestamp`, `date`, `datetime` or `t`.
//!
//! The recording is laid on today's clock: the first `warmup` of it becomes history (what
//! `/bars` serves for backfill), and the rest plays from now at `speed`, looping at the end.

use std::collections::HashMap;
use std::path::Path;

use crate::sim::Tick;
use crate::symbols::{SYMBOLS, by_name, round_to};

/// Parse a time cell into Unix milliseconds.
pub fn parse_time(s: &str) -> Option<i64> {
    let s = s.trim().trim_matches('"');
    if let Ok(x) = s.parse::<f64>() {
        return Some(if x > 1e14 {
            (x / 1000.0) as i64
        } else if x > 1e11 {
            x as i64
        } else {
            (x * 1000.0) as i64
        });
    }
    // YYYY-MM-DD[T ]HH:MM[:SS[.fff]][Z]
    let b = s.as_bytes();
    if b.len() < 16 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    let num = |a: usize, n: usize| s.get(a..a + n)?.parse::<i64>().ok();
    let (y, mo, d, h, mi) = (num(0, 4)?, num(5, 2)?, num(8, 2)?, num(11, 2)?, num(14, 2)?);
    let sec = if b.len() >= 19 && b[16] == b':' {
        s.get(17..)?
            .trim_end_matches('Z')
            .split(['+', 'Z'])
            .next()?
            .parse::<f64>()
            .ok()?
    } else {
        0.0
    };
    // days from civil (Howard Hinnant)
    let y2 = if mo <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let doy = (153 * (if mo > 2 { mo - 3 } else { mo + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400_000 + h * 3_600_000 + mi * 60_000 + (sec * 1000.0).round() as i64)
}

/// Read one CSV into ticks for symbol `sym`.
pub fn read_csv(text: &str, sym: usize) -> Result<Vec<Tick>, String> {
    let spec = &SYMBOLS[sym];
    let mut lines = text.lines().filter(|l| !l.trim().is_empty());
    let header: Vec<String> = lines
        .next()
        .ok_or("empty file")?
        .split(',')
        .map(|h| h.trim().trim_matches('"').to_lowercase())
        .collect();
    let col = |names: &[&str]| header.iter().position(|h| names.contains(&h.as_str()));
    let t = col(&["time", "timestamp", "date", "datetime", "t"])
        .ok_or("no time column (time, timestamp, date, datetime or t)")?;
    let bid = col(&["bid"]);
    let ask = col(&["ask"]);
    let ohlc = (
        col(&["open", "o"]),
        col(&["high", "h"]),
        col(&["low", "l"]),
        col(&["close", "c"]),
    );
    let mut rows: Vec<(i64, Vec<f64>)> = Vec::new();
    for (i, line) in lines.enumerate() {
        let cells: Vec<&str> = line.split(',').collect();
        let get = |c: Option<usize>| {
            c.and_then(|c| cells.get(c))
                .and_then(|v| v.trim().trim_matches('"').parse::<f64>().ok())
        };
        let tm = cells
            .get(t)
            .and_then(|c| parse_time(c))
            .ok_or(format!("line {}: bad time", i + 2))?;
        let vals = if bid.is_some() {
            vec![
                get(bid).ok_or(format!("line {}: bad bid", i + 2))?,
                get(ask).unwrap_or(f64::NAN),
            ]
        } else if let (Some(_), Some(_), Some(_), Some(_)) = ohlc {
            let v = [get(ohlc.0), get(ohlc.1), get(ohlc.2), get(ohlc.3)];
            if v.iter().any(|x| x.is_none()) {
                return Err(format!("line {}: bad open/high/low/close", i + 2));
            }
            v.iter().map(|x| x.unwrap()).collect()
        } else {
            return Err("need either a bid column or open, high, low and close".into());
        };
        rows.push((tm, vals));
    }
    rows.sort_by_key(|r| r.0);
    let unit = 10f64.powi(-(spec.digits as i32));
    let with_ask = |t: i64, b: f64, a: f64| Tick {
        t,
        bid: b,
        ask: if a.is_finite() && a > b {
            a
        } else {
            round_to(b + spec.spread.max(unit), spec.digits)
        },
    };
    if bid.is_some() {
        return Ok(rows
            .into_iter()
            .map(|(t, v)| with_ask(t, v[0], v[1]))
            .collect());
    }
    // Bars: the typical spacing sets where the four ticks fall.
    let mut gaps: Vec<i64> = rows
        .windows(2)
        .map(|w| w[1].0 - w[0].0)
        .filter(|g| *g > 0)
        .collect();
    gaps.sort_unstable();
    let len = gaps
        .get(gaps.len() / 2)
        .copied()
        .unwrap_or(60_000)
        .min(86_400_000);
    let mut out = Vec::with_capacity(rows.len() * 4);
    for (t, v) in rows {
        let (o, h, l, c) = (v[0], v[1], v[2], v[3]);
        let (first, second) = if c >= o { (l, h) } else { (h, l) };
        for (f, px) in [(0.0, o), (0.3, first), (0.6, second), (0.9, c)] {
            out.push(with_ask(t + (len as f64 * f) as i64, px, f64::NAN));
        }
    }
    Ok(out)
}

/// Every `<SYMBOL>.csv` in `dir` the floor has a pit for.
pub fn load_dir(dir: &Path) -> Result<HashMap<usize, Vec<Tick>>, String> {
    let mut out = HashMap::new();
    let entries = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for e in entries.flatten() {
        let p = e.path();
        if p.extension()
            .and_then(|x| x.to_str())
            .map(|x| x.eq_ignore_ascii_case("csv"))
            != Some(true)
        {
            continue;
        }
        let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        let Some(sym) = by_name(stem) else {
            eprintln!(
                "floor_tape: skipping {} (no pit for {stem}; names are {})",
                p.display(),
                SYMBOLS.map(|s| s.name).join(", ")
            );
            continue;
        };
        let text = std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?;
        let ticks = read_csv(&text, sym).map_err(|e| format!("{}: {e}", p.display()))?;
        if !ticks.is_empty() {
            out.insert(sym, ticks);
        }
    }
    if out.is_empty() {
        return Err(format!(
            "no usable CSV in {} (files are named after a symbol, e.g. XAUUSD.csv)",
            dir.display()
        ));
    }
    Ok(out)
}

/// The recording laid on the wall clock.
pub struct Replay {
    pub data: HashMap<usize, Vec<Tick>>,
    next: HashMap<usize, usize>,
    /// Data time where live play starts.
    cursor0: i64,
    end: i64,
    speed: f64,
    looping: bool,
    /// Wall time live play (re)started.
    wall0: i64,
    pub done: bool,
}

impl Replay {
    pub fn new(
        data: HashMap<usize, Vec<Tick>>,
        warmup_ms: i64,
        speed: f64,
        looping: bool,
        now: i64,
    ) -> Self {
        let start = data
            .values()
            .filter_map(|v| v.first())
            .map(|t| t.t)
            .min()
            .unwrap_or(0);
        let end = data
            .values()
            .filter_map(|v| v.last())
            .map(|t| t.t)
            .max()
            .unwrap_or(0);
        // At most four fifths of the recording is history, so there is always some to play.
        let cursor0 = (start + warmup_ms).min(start + (end - start) * 4 / 5);
        Replay {
            data,
            next: HashMap::new(),
            cursor0,
            end,
            speed: speed.max(0.01),
            looping,
            wall0: now,
            done: false,
        }
    }

    /// The ticks before the cursor, on today's clock: the history.
    pub fn history(&mut self) -> Vec<(usize, Tick)> {
        let offset = self.wall0 - self.cursor0;
        let mut out = Vec::new();
        for (sym, ticks) in &self.data {
            let n = ticks.partition_point(|t| t.t < self.cursor0);
            self.next.insert(*sym, n);
            out.extend(ticks[..n].iter().map(|t| {
                (
                    *sym,
                    Tick {
                        t: t.t + offset,
                        ..*t
                    },
                )
            }));
        }
        out.sort_by_key(|(_, t)| t.t);
        out
    }

    /// The ticks due by wall time `now`, stamped with the wall time they fall at.
    pub fn poll(&mut self, now: i64) -> Vec<(usize, Tick)> {
        if self.done {
            return Vec::new();
        }
        let cursor = self.cursor0 + ((now - self.wall0) as f64 * self.speed) as i64;
        let mut out = Vec::new();
        for (sym, ticks) in &self.data {
            let i = self.next.entry(*sym).or_insert(0);
            while *i < ticks.len() && ticks[*i].t <= cursor {
                let t = ticks[*i];
                let wall = self.wall0 + ((t.t - self.cursor0) as f64 / self.speed) as i64;
                out.push((
                    *sym,
                    Tick {
                        t: wall.max(self.wall0),
                        ..t
                    },
                ));
                *i += 1;
            }
        }
        if cursor > self.end {
            if self.looping {
                self.wall0 = now;
                for (sym, ticks) in &self.data {
                    self.next
                        .insert(*sym, ticks.partition_point(|t| t.t < self.cursor0));
                }
            } else {
                self.done = true;
            }
        }
        out.sort_by_key(|(_, t)| t.t);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_parse_in_every_form() {
        let ms = 1_761_030_000_000; // 2025-10-21T07:00:00Z
        assert_eq!(parse_time("1761030000"), Some(ms));
        assert_eq!(parse_time("1761030000000"), Some(ms));
        assert_eq!(parse_time("2025-10-21T07:00:00Z"), Some(ms));
        assert_eq!(parse_time("2025-10-21 07:00:00"), Some(ms));
        assert_eq!(parse_time("2025-10-21T07:00"), Some(ms));
        assert_eq!(parse_time("2025-10-21T07:00:00.250Z"), Some(ms + 250));
        assert_eq!(parse_time("yesterday"), None);
    }

    #[test]
    fn ticks_and_bars_are_both_read() {
        let eur = by_name("EURUSD").unwrap();
        let t = read_csv(
            "time,bid,ask\n1761030001,1.1,1.10008\n1761030000,1.09,\n",
            eur,
        )
        .unwrap();
        assert_eq!(t.len(), 2);
        assert!(
            t[0].t < t[1].t && t[0].ask > t[0].bid,
            "sorted, and a missing ask gets the usual spread"
        );
        let b = read_csv("timestamp,open,high,low,close,volume\n2025-10-21T07:00:00Z,1,3,0.5,2,10\n2025-10-21T07:01:00Z,2,2.5,1.5,1.8,4\n", eur).unwrap();
        assert_eq!(b.len(), 8);
        assert_eq!(
            b[..4].iter().map(|t| t.bid).collect::<Vec<_>>(),
            vec![1.0, 0.5, 3.0, 2.0],
            "up bar: open, low, high, close"
        );
        assert!(read_csv("when,price\n1,2\n", eur).is_err());
    }

    #[test]
    fn the_recording_splits_into_history_and_live_play() {
        let sym = 0;
        let ticks: Vec<Tick> = (0..100)
            .map(|i| Tick {
                t: i * 1000,
                bid: 1.0 + i as f64,
                ask: 2.0 + i as f64,
            })
            .collect();
        let mut r = Replay::new(HashMap::from([(sym, ticks)]), 50_000, 2.0, false, 1_000_000);
        let h = r.history();
        assert_eq!(h.len(), 50);
        assert_eq!(
            h.last().unwrap().1.t,
            1_000_000 - 1000,
            "history ends just before now"
        );
        let now_due = r.poll(1_000_000);
        assert_eq!(now_due.len(), 1, "the tick at the cursor plays at once");
        let later = r.poll(1_010_000);
        assert_eq!(
            later.len(),
            20,
            "ten seconds at 2x is twenty one-second ticks"
        );
        assert!(later.iter().all(|(_, t)| t.t <= 1_010_000));
        r.poll(2_000_000);
        assert!(r.done);
    }
}
