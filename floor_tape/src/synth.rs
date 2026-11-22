//! The synthetic books: a five-level broker ladder around each quote, and a retail order book
//! in OANDA's shape for the FX pairs and metals. Both are invented, and labelled so wherever
//! they are served.

use serde_json::{Value, json};

use crate::rng::Rng;
use crate::sim::{MINUTE_MS, Tick};
use crate::symbols::{SYMBOLS, round_to};

/// Bitcoin's index in `SYMBOLS`: the one symbol with a Level 2 book.
pub const BTC: usize = 6;
/// Levels a side in the broker ladder.
pub const LEVELS: usize = 5;

/// Bids and asks, best first: (price, size).
pub type Ladder = (Vec<(f64, f64)>, Vec<(f64, f64)>);

/// A five-level broker ladder around the quote, sizes invented.
pub fn synthetic_ladder(sym: usize, tick: &Tick, rng: &mut Rng) -> Ladder {
    let spec = &SYMBOLS[sym];
    let unit = 10f64.powi(-(spec.digits as i32));
    let step = (spec.spread * 0.5).max(unit);
    let lot = match spec.name {
        "EURUSD" | "GBPUSD" => 500_000.0,
        "XAUUSD" => 50.0,
        "XAGUSD" => 2_000.0,
        "BTCUSD" => 2.0,
        _ => 20.0,
    };
    let mut side = |px: f64, dir: f64| -> Vec<(f64, f64)> {
        (0..LEVELS)
            .map(|i| {
                (
                    round_to(px + dir * step * i as f64, spec.digits),
                    (lot * (0.4 + 1.6 * rng.unit()) * (1.0 + i as f64 * 0.5)).round(),
                )
            })
            .collect()
    };
    let bids = side(tick.bid, -1.0);
    let asks = side(tick.ask, 1.0);
    (bids, asks)
}

/// A retail order book in OANDA's shape (`[price, long %, short %]` buckets within 4% of the
/// price) for the FX pairs and metals, refreshed every twenty minutes like OANDA's. Orders
/// cluster at round numbers. Synthetic symbols only: it is invented.
pub fn synthetic_orderbook(seed: u64, sym: usize, price: f64, now: i64) -> Option<Value> {
    let spec = &SYMBOLS[sym];
    let (width, round) = match spec.name {
        "EURUSD" | "GBPUSD" => (0.0005, 0.0050),
        "XAUUSD" => (1.0, 25.0),
        "XAGUSD" => (0.05, 0.5),
        _ => return None,
    };
    let window = now.div_euclid(20 * MINUTE_MS);
    let mut rng = Rng::keyed(seed, 100 + sym as u64, window as u64);
    let lo = ((price * 0.96) / width).ceil() as i64;
    let hi = ((price * 1.04) / width).floor() as i64;
    let buckets: Vec<Value> = (lo..=hi)
        .map(|k| {
            let px = round_to(k as f64 * width, spec.digits);
            let near_round = ((px / round).round() * round - px).abs() < width * 0.5;
            let boost = if near_round {
                3.0 + 3.0 * rng.unit()
            } else {
                1.0
            };
            let below = px < price;
            let (l, s) = (0.05 + 0.15 * rng.unit(), 0.05 + 0.15 * rng.unit());
            // Buy limits rest below the price and sell limits above, so the round numbers there swell.
            let (l, s) = if below {
                (l * boost, s)
            } else {
                (l, s * boost)
            };
            json!([
                px,
                (l * 1000.0f64).round() / 1000.0,
                (s * 1000.0f64).round() / 1000.0
            ])
        })
        .collect();
    let at = window * 20 * MINUTE_MS;
    Some(
        json!({"symbol": spec.name, "instrument": spec.name, "time": format!("{}", at), "price": price, "bucketWidth": width, "buckets": buckets, "positions": [], "fetchedAt": at, "source": "synthetic"}),
    )
}
