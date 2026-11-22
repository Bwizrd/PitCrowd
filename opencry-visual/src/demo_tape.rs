//! The browser demo's market: `floor_tape`'s synthetic market running inside the page, so the
//! floor works from a static web page (GitHub Pages) with no tape anywhere.
//!
//! It is on when `endpoints::demo_mode()` says so (`?tape=demo`, or any `*.github.io` page).
//! The tape's messages come out as the same JSON a real tape sends, and `live_feed` parses
//! them with the code it uses for a socket, so the demo runs the real path. Bitcoin's Level 2
//! messages are queued here for whoever reads them.
//!
//! It is invented and has no edge, and the floor says so: the status chip reads DEMO.

use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};

use floor_tape::demo::InPageTape;

/// The demo's seed: the same market for every visitor who arrives at the same moment.
pub const SEED: u64 = 42;
/// One-minute bars of history made at start-up: five days, the longest a chart asks for
/// (15m bars over `-5d`). It is made as bars, not ticks, so it costs milliseconds.
pub const HISTORY_MS: i64 = 5 * 24 * 60 * 60 * 1000;
/// Level 2 messages kept for a reader that has not drained them yet.
const L2_QUEUE_MAX: usize = 5_000;

pub struct DemoTape {
    tape: Mutex<InPageTape>,
    l2: Mutex<VecDeque<String>>,
}

static DEMO: OnceLock<Option<DemoTape>> = OnceLock::new();

/// The page's demo market, made on first use, or `None` when the floor reads a real tape.
pub fn demo() -> Option<&'static DemoTape> {
    DEMO.get_or_init(|| {
        crate::endpoints::demo_mode().then(|| DemoTape::new(crate::candles::wall_clock_ms()))
    })
    .as_ref()
}

impl DemoTape {
    pub fn new(now_ms: i64) -> Self {
        DemoTape {
            tape: Mutex::new(InPageTape::new(SEED, now_ms, HISTORY_MS)),
            l2: Mutex::new(VecDeque::new()),
        }
    }

    /// The tape socket's first message.
    pub fn hello(&self) -> String {
        self.tape.lock().map(|t| t.hello()).unwrap_or_default()
    }

    /// Every tape message due by `now_ms`; Bitcoin's Level 2 messages go to the queue.
    pub fn step(&self, now_ms: i64) -> Vec<String> {
        let Ok(mut tape) = self.tape.lock() else {
            return Vec::new();
        };
        let out = tape.step(now_ms);
        if let Ok(mut q) = self.l2.lock() {
            q.extend(out.l2);
            let over = q.len().saturating_sub(L2_QUEUE_MAX);
            q.drain(..over);
        }
        out.tape
    }

    /// Bitcoin's whole Level 2 book now, then everything queued since.
    pub fn l2_snapshot(&self, now_ms: i64) -> Option<String> {
        if let Ok(mut q) = self.l2.lock() {
            q.clear();
        }
        self.tape.lock().ok()?.l2_snapshot(now_ms)
    }

    /// Level 2 messages since the last call.
    pub fn take_l2(&self) -> Vec<String> {
        self.l2
            .lock()
            .map(|mut q| q.drain(..).collect())
            .unwrap_or_default()
    }

    /// The body a tape would give for `/bars`.
    pub fn bars(
        &self,
        symbol: &str,
        timeframe: &str,
        range: &str,
        delta: bool,
        now_ms: i64,
    ) -> Result<String, String> {
        self.tape
            .lock()
            .map_err(|_| "demo tape unavailable".to_string())?
            .bars(symbol, timeframe, range, delta, now_ms)
    }

    /// The body a tape would give for `/orderbook`.
    pub fn orderbook(&self, symbol: &str, now_ms: i64) -> Option<String> {
        self.tape.lock().ok()?.orderbook(symbol, now_ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::live_feed::{TapeMessage, parse_tape};

    const NOW: i64 = 1_763_380_800_000;

    #[test]
    fn the_floor_reads_the_demo_with_its_own_tape_parser() {
        let demo = DemoTape::new(NOW);
        let mut ticks = 0;
        let mut books = 0;
        for i in 1..=100 {
            for m in demo.step(NOW + i * 50) {
                match parse_tape(&m) {
                    Some(TapeMessage::Tick(..)) => ticks += 1,
                    Some(TapeMessage::Depth(..)) => books += 1,
                    None => {}
                }
            }
        }
        assert!(ticks > 0 && books > 0, "ticks {ticks}, books {books}");
        assert!(demo.l2_snapshot(NOW).is_some());
        assert!(demo.bars("XAUUSD", "1m", "-1h", true, NOW).is_ok());
    }

    #[test]
    fn the_level_2_queue_is_bounded() {
        let demo = DemoTape::new(NOW);
        for i in 1..=2_000 {
            demo.step(NOW + i * 50);
        }
        assert!(demo.take_l2().len() <= L2_QUEUE_MAX);
        assert!(demo.take_l2().is_empty());
    }
}
