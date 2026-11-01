//! The nine symbols the floor has pits for, with the ids the floor maps them by.
//!
//! The ids are the floor's own keys (`live_feed::symbol_id_for`): a feed you wire in only has
//! to use these names; the tape stamps the ids. The other numbers drive the synthetic market
//! only: a starting price, a typical daily move, a typical spread and a tick rate, each of
//! the order a retail CFD broker shows for the instrument.

#[derive(Debug, Clone, Copy)]
pub struct Spec {
    pub name: &'static str,
    pub id: u32,
    /// Synthetic: the price the path starts from (and is weakly pulled back towards).
    pub anchor: f64,
    /// Synthetic: standard deviation of a day's log return, as a fraction.
    pub daily_vol: f64,
    /// Typical spread in price units.
    pub spread: f64,
    /// Decimal places the broker quotes.
    pub digits: u32,
    /// Synthetic: average ticks a second at normal activity.
    pub tick_rate: f64,
    /// Trades round the clock (crypto) rather than following the FX day's rhythm.
    pub always_busy: bool,
}

pub const SYMBOLS: [Spec; 9] = [
    Spec { name: "XAUUSD", id: 241, anchor: 3900.0, daily_vol: 0.015, spread: 0.30, digits: 2, tick_rate: 3.0, always_busy: false },
    Spec { name: "XAGUSD", id: 238, anchor: 48.0, daily_vol: 0.025, spread: 0.030, digits: 3, tick_rate: 2.4, always_busy: false },
    Spec { name: "NAS100", id: 205, anchor: 26000.0, daily_vol: 0.012, spread: 1.5, digits: 1, tick_rate: 3.2, always_busy: false },
    Spec { name: "US500", id: 220, anchor: 7000.0, daily_vol: 0.009, spread: 0.5, digits: 1, tick_rate: 1.8, always_busy: false },
    Spec { name: "EURUSD", id: 185, anchor: 1.12, daily_vol: 0.004, spread: 0.00008, digits: 5, tick_rate: 1.5, always_busy: false },
    Spec { name: "GBPUSD", id: 199, anchor: 1.33, daily_vol: 0.0045, spread: 0.00012, digits: 5, tick_rate: 1.5, always_busy: false },
    Spec { name: "BTCUSD", id: 160, anchor: 100000.0, daily_vol: 0.025, spread: 2.0, digits: 1, tick_rate: 2.5, always_busy: true },
    Spec { name: "US30", id: 219, anchor: 48000.0, daily_vol: 0.009, spread: 2.5, digits: 1, tick_rate: 2.3, always_busy: false },
    Spec { name: "UK100", id: 217, anchor: 9800.0, daily_vol: 0.009, spread: 1.0, digits: 1, tick_rate: 1.5, always_busy: false },
];

/// Look a symbol up by name, forgiving case and a slash (`EUR/USD`, `eurusd`).
pub fn by_name(name: &str) -> Option<usize> {
    let n: String = name.chars().filter(|c| *c != '/').collect::<String>().to_uppercase();
    SYMBOLS.iter().position(|s| s.name == n)
}

pub fn round_to(x: f64, digits: u32) -> f64 {
    let m = 10f64.powi(digits as i32);
    (x * m).round() / m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_found_with_or_without_a_slash() {
        assert_eq!(by_name("EUR/USD"), by_name("EURUSD"));
        assert_eq!(by_name("btcusd").map(|i| SYMBOLS[i].id), Some(160));
        assert_eq!(by_name("DOGE"), None);
    }
}
