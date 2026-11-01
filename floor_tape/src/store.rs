//! One-minute bars per symbol, built from ticks (or loaded as bars), and the queries the
//! tape's `/bars` answers: any whole number of minutes, hours or days over a trailing range,
//! with the tick delta (upticks, downticks, mean spread) when asked.

use std::collections::VecDeque;

use serde_json::{json, Value};

use crate::sim::{Tick, MINUTE_MS};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bar {
    pub t: i64,
    pub o: f64,
    pub h: f64,
    pub l: f64,
    pub c: f64,
    /// Ticks in the bar (the tape's volume: CFD feeds have no traded size).
    pub n: u32,
    pub up: u32,
    pub down: u32,
    pub spread_sum: f64,
    /// Ticks the spread was measured on (bars loaded as bars have none).
    pub spread_n: u32,
}

impl Bar {
    fn open(t: i64, tick: &Tick) -> Self {
        Bar { t, o: tick.bid, h: tick.bid, l: tick.bid, c: tick.bid, n: 0, up: 0, down: 0, spread_sum: 0.0, spread_n: 0 }
    }
}

#[derive(Debug, Default)]
pub struct Series {
    pub bars: VecDeque<Bar>,
    last_bid: Option<f64>,
}

pub fn minute_start(t: i64) -> i64 {
    t.div_euclid(MINUTE_MS) * MINUTE_MS
}

impl Series {
    /// Fold one tick in. A tick older than the newest bar's minute is dropped: the tape is a
    /// live record, and history goes in as bars.
    pub fn add_tick(&mut self, tick: &Tick) -> bool {
        let m = minute_start(tick.t);
        match self.bars.back() {
            Some(b) if m < b.t => return false,
            Some(b) if m == b.t => {}
            _ => self.bars.push_back(Bar::open(m, tick)),
        }
        let b = self.bars.back_mut().unwrap();
        b.h = b.h.max(tick.bid);
        b.l = b.l.min(tick.bid);
        b.c = tick.bid;
        b.n += 1;
        b.spread_sum += tick.ask - tick.bid;
        b.spread_n += 1;
        if let Some(prev) = self.last_bid {
            if tick.bid > prev {
                b.up += 1;
            } else if tick.bid < prev {
                b.down += 1;
            }
        }
        self.last_bid = Some(tick.bid);
        true
    }

    /// Load history bars (one-minute, any order). A bar for a minute already held replaces it.
    pub fn add_bars(&mut self, mut bars: Vec<Bar>) {
        bars.sort_by_key(|b| b.t);
        let mut all: Vec<Bar> = self.bars.drain(..).collect();
        all.extend(bars);
        all.sort_by_key(|b| b.t);
        all.dedup_by(|later, earlier| {
            if later.t == earlier.t {
                *earlier = *later;
                true
            } else {
                false
            }
        });
        self.bars = all.into();
        self.last_bid = self.bars.back().map(|b| b.c);
    }

    /// Drop bars older than `keep_from`.
    pub fn trim(&mut self, keep_from: i64) {
        while self.bars.front().is_some_and(|b| b.t < keep_from) {
            self.bars.pop_front();
        }
    }

    /// Bars of `tf_ms` from `from_ms` on, as the tape's `/bars` rows.
    pub fn query(&self, from_ms: i64, tf_ms: i64, delta: bool) -> Vec<Value> {
        let start = self.bars.partition_point(|b| b.t < from_ms);
        let mut out: Vec<Bar> = Vec::new();
        for b in self.bars.range(start..) {
            let t = b.t.div_euclid(tf_ms) * tf_ms;
            match out.last_mut() {
                Some(a) if a.t == t => {
                    a.h = a.h.max(b.h);
                    a.l = a.l.min(b.l);
                    a.c = b.c;
                    a.n += b.n;
                    a.up += b.up;
                    a.down += b.down;
                    a.spread_sum += b.spread_sum;
                    a.spread_n += b.spread_n;
                }
                _ => out.push(Bar { t, ..*b }),
            }
        }
        out.iter()
            .map(|b| {
                let mut row = json!({"timestamp": b.t, "open": b.o, "high": b.h, "low": b.l, "close": b.c, "volume": b.n as f64});
                if delta && b.spread_n > 0 {
                    row["upticks"] = json!(b.up as f64);
                    row["downticks"] = json!(b.down as f64);
                    row["spread_mean"] = json!(b.spread_sum / b.spread_n as f64);
                }
                row
            })
            .collect()
    }
}

/// `1m`, `15m`, `4h`, `1d` → milliseconds.
pub fn parse_timeframe(s: &str) -> Option<i64> {
    let (n, unit) = s.split_at(s.find(|c: char| !c.is_ascii_digit())?);
    let n: i64 = n.parse().ok().filter(|n| *n > 0)?;
    Some(n * match unit {
        "m" => MINUTE_MS,
        "h" => 60 * MINUTE_MS,
        "d" => 1440 * MINUTE_MS,
        _ => return None,
    })
}

/// `-6h`, `-2d`, `-1w`, `-90m` → milliseconds back from now.
pub fn parse_range(s: &str) -> Option<i64> {
    let s = s.strip_prefix('-')?;
    let (n, unit) = s.split_at(s.find(|c: char| !c.is_ascii_digit())?);
    let n: i64 = n.parse().ok().filter(|n| *n > 0)?;
    Some(n * match unit {
        "m" => MINUTE_MS,
        "h" => 60 * MINUTE_MS,
        "d" => 1440 * MINUTE_MS,
        "w" => 7 * 1440 * MINUTE_MS,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tick(t: i64, bid: f64) -> Tick {
        Tick { t, bid, ask: bid + 0.5 }
    }

    #[test]
    fn ticks_make_bars_and_bars_aggregate() {
        let mut s = Series::default();
        for (i, px) in [10.0, 11.0, 9.0, 10.5, 12.0, 11.0].iter().enumerate() {
            s.add_tick(&tick(i as i64 * 30_000, *px));
        }
        assert_eq!(s.bars.len(), 3);
        let rows = s.query(0, 3 * MINUTE_MS, true);
        assert_eq!(rows.len(), 1);
        let r = &rows[0];
        assert_eq!((r["open"].as_f64(), r["high"].as_f64(), r["low"].as_f64(), r["close"].as_f64()), (Some(10.0), Some(12.0), Some(9.0), Some(11.0)));
        assert_eq!(r["volume"].as_f64(), Some(6.0));
        assert_eq!((r["upticks"].as_f64(), r["downticks"].as_f64()), (Some(3.0), Some(2.0)));
        assert_eq!(r["spread_mean"].as_f64(), Some(0.5));
    }

    #[test]
    fn a_late_tick_is_dropped_and_loaded_bars_merge() {
        let mut s = Series::default();
        s.add_tick(&tick(120_000, 5.0));
        assert!(!s.add_tick(&tick(10_000, 4.0)));
        s.add_bars(vec![Bar { t: 0, o: 1.0, h: 2.0, l: 0.5, c: 1.5, n: 3, up: 0, down: 0, spread_sum: 0.0, spread_n: 0 }]);
        assert_eq!(s.bars.iter().map(|b| b.t).collect::<Vec<_>>(), vec![0, 120_000]);
        assert!(s.query(0, MINUTE_MS, true)[0].get("upticks").is_none(), "bars loaded as bars carry no delta");
    }

    #[test]
    fn timeframes_and_ranges_parse_like_the_tape() {
        assert_eq!(parse_timeframe("15m"), Some(15 * MINUTE_MS));
        assert_eq!(parse_timeframe("1d"), Some(1440 * MINUTE_MS));
        assert_eq!(parse_timeframe("m"), None);
        assert_eq!(parse_range("-6h"), Some(360 * MINUTE_MS));
        assert_eq!(parse_range("-1w"), Some(7 * 1440 * MINUTE_MS));
        assert_eq!(parse_range("6h"), None);
    }
}
