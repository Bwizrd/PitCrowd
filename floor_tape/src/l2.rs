//! A synthetic Level 2 book for BTC/USD around the tape's own BTCUSD quote, sent in the same
//! messages as Kraken's public websocket v2 (`book` snapshot/update, `trade` update), so the
//! floor's heatmap and pit walls run offline on the very code that reads Kraken.
//!
//! What it does each tick: levels the price crosses trade away (a `trade` is sent first, so
//! the floor knows they were not pulled), the touch is re-quoted, a few levels further out
//! change size, and now and then a large order (a "wall") is posted at a round number or
//! pulled before price reaches it. Only the shape is realistic; the sizes are invented.

use std::collections::BTreeMap;

use serde_json::{json, Value};

use crate::rng::Rng;

pub const TICK: f64 = 0.1;
pub const DEPTH: usize = 500;
const WALL_MIN: f64 = 5.0;

fn key(p: f64) -> i64 {
    (p / TICK).round() as i64
}

fn price(k: i64) -> f64 {
    (k as f64 * TICK * 10.0).round() / 10.0
}

pub struct L2 {
    bids: BTreeMap<i64, f64>,
    asks: BTreeMap<i64, f64>,
    rng: Rng,
    trade_id: u64,
}

fn iso(t_ms: i64) -> String {
    // RFC 3339 from Unix ms without a date library (days-from-civil, Howard Hinnant).
    let secs = t_ms.div_euclid(1000);
    let (days, rem) = (secs.div_euclid(86400), secs.rem_euclid(86400));
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{:03}Z", rem / 3600, rem % 3600 / 60, rem % 60, t_ms.rem_euclid(1000))
}

impl L2 {
    pub fn new(seed: u64, bid: f64, ask: f64) -> Self {
        let mut l2 = L2 { bids: BTreeMap::new(), asks: BTreeMap::new(), rng: Rng::new(seed ^ 0x12),  trade_id: 0 };
        let (kb, ka) = (key(bid), key(ask).max(key(bid) + 1));
        for (is_bid, touch) in [(true, kb), (false, ka)] {
            let mut k = touch;
            for _ in 0..DEPTH {
                let q = l2.size();
                l2.side(is_bid).insert(k, q);
                let gap = 1 + (-(l2.rng.unit().max(1e-9)).ln() * 11.0) as i64;
                k += if is_bid { -gap } else { gap };
            }
            // Walls at some round $50s.
            for n in 1..=12 {
                if l2.rng.unit() < 0.35 {
                    let p = ((price(touch) / 50.0).round() + if is_bid { -(n as f64) } else { n as f64 }) * 50.0;
                    let q = WALL_MIN + 15.0 * l2.rng.unit();
                    l2.side(is_bid).insert(key(p), q);
                }
            }
        }
        l2
    }

    fn side(&mut self, bid: bool) -> &mut BTreeMap<i64, f64> {
        if bid { &mut self.bids } else { &mut self.asks }
    }

    fn size(&mut self) -> f64 {
        ((self.rng.normal() - 1.2).exp() * 100_000.0).round() / 100_000.0
    }

    fn levels(map: &BTreeMap<i64, f64>, bid: bool, n: usize) -> Vec<Value> {
        let it: Box<dyn Iterator<Item = (&i64, &f64)>> = if bid { Box::new(map.iter().rev()) } else { Box::new(map.iter()) };
        it.take(n).map(|(k, q)| json!({"price": price(*k), "qty": q})).collect()
    }

    pub fn snapshot(&self, t_ms: i64) -> String {
        json!({"channel": "book", "type": "snapshot", "data": [{
            "symbol": "BTC/USD", "bids": Self::levels(&self.bids, true, DEPTH), "asks": Self::levels(&self.asks, false, DEPTH),
            "checksum": 0, "timestamp": iso(t_ms)}]})
        .to_string()
    }

    /// Move the book to a new quote; returns the messages to send, trades first.
    pub fn on_quote(&mut self, bid: f64, ask: f64, t_ms: i64) -> Vec<String> {
        let (kb, ka) = (key(bid), key(ask).max(key(bid) + 1));
        let mut trades: Vec<Value> = Vec::new();
        let mut bchg: BTreeMap<i64, f64> = BTreeMap::new();
        let mut achg: BTreeMap<i64, f64> = BTreeMap::new();
        let ts = iso(t_ms);
        // Levels the quote crossed trade away.
        let crossed_asks: Vec<(i64, f64)> = self.asks.range(..ka).map(|(k, q)| (*k, *q)).collect();
        for (k, q) in crossed_asks {
            self.trade_id += 1;
            trades.push(json!({"symbol": "BTC/USD", "side": "buy", "price": price(k), "qty": q, "ord_type": "market", "trade_id": self.trade_id, "timestamp": ts}));
            self.asks.remove(&k);
            achg.insert(k, 0.0);
        }
        let crossed_bids: Vec<(i64, f64)> = self.bids.range(kb + 1..).map(|(k, q)| (*k, *q)).collect();
        for (k, q) in crossed_bids {
            self.trade_id += 1;
            trades.push(json!({"symbol": "BTC/USD", "side": "sell", "price": price(k), "qty": q, "ord_type": "market", "trade_id": self.trade_id, "timestamp": ts}));
            self.bids.remove(&k);
            bchg.insert(k, 0.0);
        }
        // A print or two at the touch, part-filling it.
        for _ in 0..1 + (self.rng.unit() * 2.0) as usize {
            let buy = self.rng.unit() < 0.5;
            let k = if buy { ka } else { kb };
            let q = (self.size() * 0.3 * 100_000.0).round() / 100_000.0;
            self.trade_id += 1;
            trades.push(json!({"symbol": "BTC/USD", "side": if buy { "buy" } else { "sell" }, "price": price(k), "qty": q, "ord_type": "market", "trade_id": self.trade_id, "timestamp": ts}));
        }
        // Re-quote the touch and fill any hole the move left behind it.
        for (is_bid, touch) in [(true, kb), (false, ka)] {
            let q = self.size();
            self.side(is_bid).insert(touch, q);
            let chg = if is_bid { &mut bchg } else { &mut achg };
            chg.insert(touch, q);
            let mut k = touch;
            for _ in 0..40 {
                let gap = 1 + (-(self.rng.unit().max(1e-9)).ln() * 11.0) as i64;
                k += if is_bid { -gap } else { gap };
                let next = if is_bid { self.bids.range(..touch).next_back().map(|(k, _)| *k) } else { self.asks.range(touch + 1..).next().map(|(k, _)| *k) };
                let exists = self.side(is_bid).contains_key(&k);
                if next.is_some_and(|n| if is_bid { n >= k } else { n <= k }) {
                    break;
                }
                if !exists {
                    let q = self.size();
                    self.side(is_bid).insert(k, q);
                    (if is_bid { &mut bchg } else { &mut achg }).insert(k, q);
                }
            }
        }
        // Churn further out.
        for _ in 0..6 {
            let is_bid = self.rng.unit() < 0.5;
            let d = 1 + (-(self.rng.unit().max(1e-9)).ln() * 1500.0) as i64;
            let k = if is_bid { kb - d } else { ka + d };
            let r = self.rng.unit();
            let had = self.side(is_bid).get(&k).copied();
            let q = match had {
                Some(q) if q >= WALL_MIN => continue,
                Some(_) if r < 0.2 => 0.0,
                _ => self.size(),
            };
            if q == 0.0 {
                self.side(is_bid).remove(&k);
            } else {
                self.side(is_bid).insert(k, q);
            }
            (if is_bid { &mut bchg } else { &mut achg }).insert(k, q);
        }
        // Now and then a wall is posted at a round $10, or one is pulled.
        if self.rng.unit() < 0.01 {
            let is_bid = self.rng.unit() < 0.5;
            let dist = 40.0 + 360.0 * self.rng.unit();
            let p = ((if is_bid { price(kb) - dist } else { price(ka) + dist }) / 10.0).round() * 10.0;
            let q = ((WALL_MIN + 1.0 + 19.0 * self.rng.unit()) * 1000.0).round() / 1000.0;
            self.side(is_bid).insert(key(p), q);
            (if is_bid { &mut bchg } else { &mut achg }).insert(key(p), q);
        }
        if self.rng.unit() < 0.004 {
            let is_bid = self.rng.unit() < 0.5;
            let walls: Vec<i64> = self.side(is_bid).iter().filter(|(_, q)| **q >= WALL_MIN).map(|(k, _)| *k).collect();
            if !walls.is_empty() {
                let k = walls[(self.rng.unit() * walls.len() as f64) as usize % walls.len()];
                self.side(is_bid).remove(&k);
                (if is_bid { &mut bchg } else { &mut achg }).insert(k, 0.0);
            }
        }
        // Keep the subscribed depth.
        while self.bids.len() > DEPTH {
            let k = *self.bids.keys().next().unwrap();
            self.bids.remove(&k);
        }
        while self.asks.len() > DEPTH {
            let k = *self.asks.keys().next_back().unwrap();
            self.asks.remove(&k);
        }
        let mut out = Vec::new();
        if !trades.is_empty() {
            out.push(json!({"channel": "trade", "type": "update", "data": trades}).to_string());
        }
        let row = |m: &BTreeMap<i64, f64>| m.iter().map(|(k, q)| json!({"price": price(*k), "qty": q})).collect::<Vec<_>>();
        out.push(json!({"channel": "book", "type": "update", "data": [{"symbol": "BTC/USD", "bids": row(&bchg), "asks": row(&achg), "checksum": 0, "timestamp": ts}]}).to_string());
        out
    }

    #[cfg(test)]
    fn best(&self) -> (f64, f64) {
        (price(*self.bids.keys().next_back().unwrap()), price(*self.asks.keys().next().unwrap()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_book_follows_the_quote_and_crossed_levels_trade_first() {
        let mut l2 = L2::new(1, 100_000.0, 100_002.0);
        assert_eq!(l2.best(), (100_000.0, 100_002.0));
        let msgs = l2.on_quote(100_030.0, 100_031.0, 0);
        assert_eq!(l2.best(), (100_030.0, 100_031.0));
        let first: Value = serde_json::from_str(&msgs[0]).unwrap();
        assert_eq!(first["channel"], "trade");
        assert!(first["data"].as_array().unwrap().iter().any(|t| t["side"] == "buy" && t["price"].as_f64().unwrap() < 100_031.0));
        let last: Value = serde_json::from_str(msgs.last().unwrap()).unwrap();
        assert_eq!((last["channel"].as_str(), last["type"].as_str()), (Some("book"), Some("update")));
    }

    #[test]
    fn the_snapshot_is_kraken_shaped_and_deep() {
        let l2 = L2::new(2, 100_000.0, 100_001.0);
        let v: Value = serde_json::from_str(&l2.snapshot(0)).unwrap();
        let d = &v["data"][0];
        assert_eq!(d["bids"].as_array().unwrap().len(), DEPTH);
        assert!(d["bids"][0]["price"].as_f64() > d["bids"][1]["price"].as_f64(), "bids best first");
        assert!(d["asks"][0]["price"].as_f64() < d["asks"][1]["price"].as_f64(), "asks best first");
        let lo = d["bids"][DEPTH - 1]["price"].as_f64().unwrap();
        assert!(100_000.0 - lo > 400.0, "500 levels reach {lo}");
    }

    #[test]
    fn iso_times_are_rfc3339() {
        assert_eq!(iso(1_735_689_600_123), "2025-01-01T00:00:00.123Z");
        assert_eq!(iso(1_793_393_100_000), "2026-10-30T20:45:00.000Z");
    }
}
