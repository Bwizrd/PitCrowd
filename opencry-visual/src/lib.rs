//! OpenCry floor as a library: the same feed, candle, level, Jev and paper-book code runs in
//! the Bevy app (`src/main.rs`) and in the headless replay (`src/bin/replay.rs`).

pub mod camera;
pub mod candles;
pub mod depth;
pub mod desk_feed;
pub mod endpoints;
pub mod genome;
pub mod influx;
pub mod jev;
pub mod levels;
pub mod live_feed;
pub mod market;
pub mod panels;
pub mod paper;
pub mod paper_avatars;
#[cfg_attr(not(feature = "legacy_2d_floor"), allow(dead_code))]
pub mod pit;
pub mod pit_3d;
pub mod pit_3d_overlay;
pub mod pit_engine;
pub mod positions;
pub mod reaction;
pub mod report;
pub mod scenario;
pub mod tape_chart;
pub mod ticket;
#[cfg_attr(not(feature = "legacy_2d_floor"), allow(dead_code))]
pub mod trader;
pub mod ui;
pub mod unimate;
pub mod wall_board;
pub mod web_chart;

/// Where the 15m bar history (`ctrader_<SYM>_SB_15m.json`, rows `[t_ms, o, h, l, c, ticks]`)
/// lives: `$OPENCRY_EDGE_CACHE`, else `$EDGE_LAB_DIR/data_cache` (an external strategy lab's
/// cache), else `data/cache` in this repo, which `floor_tape --write-cache data/cache` fills
/// (and `start-floor.sh` does on first run).
#[cfg(not(target_arch = "wasm32"))]
pub fn edge_cache_dir() -> String {
    std::env::var("OPENCRY_EDGE_CACHE").unwrap_or_else(|_| match std::env::var("EDGE_LAB_DIR") {
        Ok(lab) => format!("{lab}/data_cache"),
        Err(_) => concat!(env!("CARGO_MANIFEST_DIR"), "/../data/cache").into(),
    })
}
