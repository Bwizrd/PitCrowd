//! The tape without a server: the synthetic market, its books and its history, driven by the
//! caller's clock. The floor's in-browser demo runs one of these inside the page, so the floor
//! works from a static web page with no tape running anywhere.
//!
//! It produces the same messages as the server (`src/main.rs`): `hello`, `tick` and `depth`
//! for the tape socket, Kraken-v2-shaped Level 2 messages for Bitcoin, and the bodies of
//! `/bars`, `/book` and `/orderbook`. The floor parses them with the code it already uses for
//! a real tape, so the demo exercises the real path, not a separate one.
//!
//! Like the server's synthetic mode, it is invented and has no edge (see `sim.rs`), and it says
//! so: every symbol's source is `synthetic`.

use serde_json::json;

use crate::l2::L2;
use crate::rng::Rng;
use crate::sim::{MINUTE_MS, Sim, Tick, minute_of};
use crate::store::{Series, parse_range, parse_timeframe};
use crate::symbols::{SYMBOLS, by_name, round_to};
use crate::synth::{BTC, LEVELS, Ladder, synthetic_ladder, synthetic_orderbook};

/// After this many minutes without a `step` (a hidden browser tab), the missed minutes go
/// into the history quietly instead of being replayed as a burst of ticks.
const CATCH_UP_MINUTES: i64 = 2;

/// What one `step` produced: messages for the tape socket and for the Level 2 socket.
#[derive(Debug, Default)]
pub struct Step {
    pub tape: Vec<String>,
    pub l2: Vec<String>,
}

/// One minute in play: its minute number, its ticks and how many have been sent.
struct Live {
    minute: i64,
    ticks: Vec<Tick>,
    next: usize,
}

pub struct InPageTape {
    sim: Sim,
    seed: u64,
    rng: Rng,
    history_ms: i64,
    series: Vec<Series>,
    quote: Vec<Option<Tick>>,
    ladder: Vec<Option<Ladder>>,
    live: Vec<Live>,
    l2: Option<L2>,
    last_trim: i64,
}

impl InPageTape {
    /// The market up to `now` (Unix ms), with `history_ms` of one-minute bars behind it. The
    /// path starts a day before the history, so warming up costs about a day's walk. The
    /// history is made as bars (`Sim::range_bars`), not ticks: days of it cost milliseconds.
    pub fn new(seed: u64, now: i64, history_ms: i64) -> Self {
        let now_m = minute_of(now);
        let from_m = minute_of(now - history_ms);
        let mut sim = Sim::starting_on(seed, from_m.div_euclid(1440) - 1);
        let mut series: Vec<Series> = (0..SYMBOLS.len()).map(|_| Series::default()).collect();
        let mut quote = vec![None; SYMBOLS.len()];
        let mut live = Vec::with_capacity(SYMBOLS.len());
        for (sym, s) in series.iter_mut().enumerate() {
            let mut bars = Vec::new();
            sim.range_bars(sym, from_m, now_m, |b| bars.push(b));
            s.add_bars(bars);
            let cur = sim.minute(sym, now_m);
            let done = cur.partition_point(|tk| tk.t <= now);
            cur[..done].iter().for_each(|tk| _ = s.add_tick(tk));
            quote[sym] = cur[..done]
                .last()
                .copied()
                .or_else(|| s.bars.back().map(quote_of));
            live.push(Live {
                minute: now_m,
                ticks: cur,
                next: done,
            });
        }
        let l2 = quote[BTC].map(|q| {
            let ask = if q.ask > q.bid {
                q.ask
            } else {
                q.bid + SYMBOLS[BTC].spread
            };
            L2::new(seed, q.bid, ask)
        });
        InPageTape {
            sim,
            seed,
            rng: Rng::new(seed ^ 0xD3),
            history_ms,
            series,
            quote,
            ladder: vec![None; SYMBOLS.len()],
            live,
            l2,
            last_trim: now,
        }
    }

    /// The tape socket's first message.
    pub fn hello(&self) -> String {
        let symbols: Vec<_> = SYMBOLS
            .iter()
            .map(|s| json!({"symbol": s.name, "id": s.id, "source": "synthetic"}))
            .collect();
        json!({"type": "hello", "source": "floor_tape (in page)", "symbols": symbols, "levels": LEVELS})
            .to_string()
    }

    /// Play every tick due by `now` (Unix ms).
    pub fn step(&mut self, now: i64) -> Step {
        let m = minute_of(now);
        let mut due: Vec<(usize, Tick)> = Vec::new();
        for sym in 0..self.live.len() {
            if m - self.live[sym].minute > CATCH_UP_MINUTES {
                self.skip_to(sym, m);
            }
            let lv = &mut self.live[sym];
            while lv.minute <= m {
                while lv.next < lv.ticks.len() && lv.ticks[lv.next].t <= now {
                    due.push((sym, lv.ticks[lv.next]));
                    lv.next += 1;
                }
                if lv.minute == m {
                    break;
                }
                lv.minute += 1;
                lv.ticks = self.sim.minute(sym, lv.minute);
                lv.next = 0;
            }
        }
        due.sort_by_key(|(_, t)| t.t);
        let mut out = Step::default();
        for (sym, tk) in due {
            self.publish(sym, tk, &mut out);
        }
        if now - self.last_trim > 10 * MINUTE_MS {
            self.last_trim = now;
            let keep = now - self.history_ms;
            self.series.iter_mut().for_each(|s| s.trim(keep));
        }
        out
    }

    /// Fold the minutes before `m` into the history (as bars) without sending them, and
    /// resume at `m`.
    fn skip_to(&mut self, sym: usize, m: i64) {
        let lv = &mut self.live[sym];
        let s = &mut self.series[sym];
        lv.ticks[lv.next..].iter().for_each(|tk| _ = s.add_tick(tk));
        let mut bars = Vec::new();
        self.sim.range_bars(sym, lv.minute + 1, m, |b| bars.push(b));
        let last = bars.last().map(quote_of).or(lv.ticks.last().copied());
        s.add_bars(bars);
        if last.is_some() {
            self.quote[sym] = last;
        }
        lv.minute = m;
        lv.ticks = self.sim.minute(sym, m);
        lv.next = 0;
    }

    /// One tick: into the bars, out as `tick` and `depth`, and on to Bitcoin's Level 2 book.
    fn publish(&mut self, sym: usize, tick: Tick, out: &mut Step) {
        if !self.series[sym].add_tick(&tick) {
            return;
        }
        let spec = &SYMBOLS[sym];
        self.quote[sym] = Some(tick);
        let (bids, asks) = synthetic_ladder(sym, &tick, &mut self.rng);
        self.ladder[sym] = Some((bids.clone(), asks.clone()));
        let spread = round_to(tick.ask - tick.bid, spec.digits + 1);
        out.tape.push(json!({"type": "tick", "data": {"symbolId": spec.id, "symbol": spec.name, "t": tick.t, "bid": tick.bid, "ask": tick.ask, "spread": spread}}).to_string());
        let row = |v: &[(f64, f64)]| v.iter().map(|(p, s)| json!([p, s])).collect::<Vec<_>>();
        out.tape.push(json!({"type": "depth", "data": {"symbolId": spec.id, "symbol": spec.name, "t": tick.t, "bids": row(&bids), "asks": row(&asks)}}).to_string());
        if sym == BTC
            && let Some(book) = self.l2.as_mut()
        {
            out.l2.extend(book.on_quote(tick.bid, tick.ask, tick.t));
        }
    }

    /// The body of `GET /bars?symbol=&timeframe=&range=[&delta=1]`, as the server answers it.
    pub fn bars(
        &self,
        symbol: &str,
        timeframe: &str,
        range: &str,
        delta: bool,
        now: i64,
    ) -> Result<String, String> {
        let sym = by_name(symbol).ok_or_else(|| format!("unknown symbol {symbol}"))?;
        let tf =
            parse_timeframe(timeframe).ok_or("timeframe must look like 1m, 5m, 15m, 1h, 1d")?;
        let back = parse_range(range).ok_or("range must look like -6h, -2d, -1w")?;
        let data = self.series[sym].query(now - back, tf, delta);
        Ok(json!({"symbol": SYMBOLS[sym].name, "timeframe": timeframe, "range": range, "source": "synthetic", "delta": delta, "count": data.len(), "data": data}).to_string())
    }

    /// The body of `GET /book?symbol=`: the five-level ladder now.
    pub fn book(&self, symbol: &str) -> Option<String> {
        let sym = by_name(symbol)?;
        let (q, (b, a)) = (self.quote[sym]?, self.ladder[sym].clone()?);
        Some(json!({"symbol": SYMBOLS[sym].name, "bid": q.bid, "ask": q.ask, "t": q.t, "bids": b, "asks": a}).to_string())
    }

    /// The body of `GET /orderbook?symbol=`: a retail order book (FX pairs and metals only).
    pub fn orderbook(&self, symbol: &str, now: i64) -> Option<String> {
        let sym = by_name(symbol)?;
        synthetic_orderbook(self.seed, sym, self.quote[sym]?.bid, now).map(|v| v.to_string())
    }

    /// The Level 2 socket's first message: Bitcoin's whole book.
    pub fn l2_snapshot(&self, now: i64) -> Option<String> {
        self.l2.as_ref().map(|b| b.snapshot(now))
    }
}

/// The quote a bar leaves behind: its close, with its average spread.
fn quote_of(b: &crate::store::Bar) -> Tick {
    let spread = if b.spread_n > 0 {
        b.spread_sum / b.spread_n as f64
    } else {
        0.0
    };
    Tick {
        t: b.t + MINUTE_MS - 1,
        bid: b.c,
        ask: b.c + spread,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_763_380_800_000; // 2025-11-17T12:00:00Z
    const HOUR: i64 = 60 * MINUTE_MS;

    fn kinds(msgs: &[String]) -> Vec<String> {
        msgs.iter()
            .map(|m| {
                let v: serde_json::Value = serde_json::from_str(m).unwrap();
                v["type"].as_str().unwrap_or("").to_string()
            })
            .collect()
    }

    #[test]
    fn it_starts_with_a_quote_and_history_for_every_symbol() {
        let tape = InPageTape::new(42, NOW, HOUR);
        for s in SYMBOLS {
            let body = tape.bars(s.name, "1m", "-1h", true, NOW).unwrap();
            let v: serde_json::Value = serde_json::from_str(&body).unwrap();
            assert_eq!(v["source"], "synthetic");
            assert!(v["count"].as_u64().unwrap() >= 59, "{}: {body}", s.name);
        }
        assert!(tape.l2_snapshot(NOW).is_some());
        let hello: serde_json::Value = serde_json::from_str(&tape.hello()).unwrap();
        assert_eq!(hello["symbols"].as_array().unwrap().len(), SYMBOLS.len());
    }

    #[test]
    fn steps_send_ticks_and_books_in_time_order() {
        let mut tape = InPageTape::new(42, NOW, HOUR);
        let mut sent = Vec::new();
        for i in 1..=200 {
            sent.extend(tape.step(NOW + i * 50).tape);
        }
        let ks = kinds(&sent);
        assert!(ks.iter().any(|k| k == "tick") && ks.iter().any(|k| k == "depth"));
        let times: Vec<i64> = sent
            .iter()
            .map(|m| {
                serde_json::from_str::<serde_json::Value>(m).unwrap()["data"]["t"]
                    .as_i64()
                    .unwrap()
            })
            .collect();
        assert!(times.windows(2).all(|w| w[0] <= w[1]));
        assert!(
            times
                .iter()
                .all(|t| *t > NOW - MINUTE_MS && *t <= NOW + 10_000)
        );
        assert!(tape.book("XAUUSD").is_some());
        assert!(tape.orderbook("XAUUSD", NOW).is_some());
        assert!(tape.orderbook("US500", NOW).is_none());
    }

    #[test]
    fn it_sends_its_own_market_tick_for_tick() {
        let mut tape = InPageTape::new(7, NOW, HOUR);
        let mut sim = Sim::starting_on(7, minute_of(NOW - HOUR).div_euclid(1440) - 1);
        let next = minute_of(NOW) + 1;
        let expected: Vec<f64> = sim.minute(0, next).iter().map(|t| t.bid).collect();
        let mut got = Vec::new();
        for m in tape.step(NOW + 2 * MINUTE_MS - 1).tape {
            let v: serde_json::Value = serde_json::from_str(&m).unwrap();
            if v["type"] == "tick" && v["data"]["symbol"] == SYMBOLS[0].name {
                let t = v["data"]["t"].as_i64().unwrap();
                if minute_of(t) == next {
                    got.push(v["data"]["bid"].as_f64().unwrap());
                }
            }
        }
        assert_eq!(got, expected);
    }

    #[test]
    fn a_long_pause_catches_up_quietly() {
        let mut tape = InPageTape::new(42, NOW, 2 * HOUR);
        let later = NOW + HOUR;
        let burst = tape.step(later).tape.len();
        // Only the minute in progress is sent: at most a minute of ticks per symbol, not an hour.
        assert!(
            burst < 2 * 9 * 600,
            "sent {burst} messages after an hour away"
        );
        let body = tape.bars("XAUUSD", "1m", "-90m", false, later).unwrap();
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert!(
            v["count"].as_u64().unwrap() >= 89,
            "the missed hour is in the history"
        );
    }

    #[test]
    fn days_of_history_fill_the_longest_chart_quickly() {
        let started = std::time::Instant::now();
        let tape = InPageTape::new(42, NOW, 5 * 24 * HOUR);
        let made = started.elapsed();
        let body = tape.bars("US500", "15m", "-5d", true, NOW).unwrap();
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert!(v["count"].as_u64().unwrap() >= 479, "{}", v["count"]);
        // Generous for a debug build: release makes the five days in a few milliseconds.
        assert!(made.as_secs_f64() < 5.0, "took {made:?}");
    }
}
