//! Option 2: TradingView Lightweight Charts in the web page, over the canvas. The JavaScript
//! side (index.html) owns the chart, its socket to the market tape (:8083, ticks → candles,
//! same rule as `candles.rs`) and its backfill from the tape's `/bars`; this module only tells
//! it what to show — the cTrader symbol id for matching ticks, the tape's symbol name for
//! `/bars` — and hands it the pit's deals as markers. Browser only; a no-op natively.

use bevy::prelude::*;

use crate::candles::{wall_clock_ms, CandleStore, ChartSettings, Timeframe};
use crate::levels::tape_symbol;
use crate::live_feed::{symbol_id_for, FeedMode, LiveFeed};
use crate::market::Instrument;
use crate::pit::instrument_decimals;
use crate::pit_engine::{PitEngines, PitEventKind};
use crate::ui::Selection;

#[cfg(target_arch = "wasm32")]
mod js {
    use wasm_bindgen::prelude::*;
    #[wasm_bindgen]
    extern "C" {
        #[wasm_bindgen(js_namespace = ["window", "__opencryChart"], js_name = show)]
        pub fn show(symbol_id: u32, symbol: &str, title: &str, timeframe: &str, decimals: u32);
        #[wasm_bindgen(js_namespace = ["window", "__opencryChart"], js_name = hide)]
        pub fn hide();
        #[wasm_bindgen(js_namespace = ["window", "__opencryChart"], js_name = addDeal)]
        pub fn add_deal(symbol_id: u32, time_ms: f64, price: f64, is_buy: bool);
        #[wasm_bindgen(js_namespace = ["window", "__opencryChart"], js_name = updateFlow)]
        pub fn update_flow(symbol_id: u32, time_ms: f64, delta: f64, ticks: f64);
        #[wasm_bindgen(js_namespace = ["window", "__opencryChart"], js_name = setDeals)]
        pub fn set_deals(on: bool);
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod js {
    pub fn show(_symbol_id: u32, _symbol: &str, _title: &str, _timeframe: &str, _decimals: u32) {}
    pub fn hide() {}
    pub fn add_deal(_symbol_id: u32, _time_ms: f64, _price: f64, _is_buy: bool) {}
    pub fn update_flow(_symbol_id: u32, _time_ms: f64, _delta: f64, _ticks: f64) {}
    pub fn set_deals(_on: bool) {}
}

/// System: keep the page's chart in step with the selection, timeframe and toggle, and
/// push this frame's deals to it.
pub fn sync_web_chart(
    feed: Res<LiveFeed>,
    selection: Res<Selection>,
    settings: Res<ChartSettings>,
    engines: Res<PitEngines>,
    store: Res<CandleStore>,
    mut shown: Local<Option<(Instrument, Timeframe)>>,
    mut flow_sent: Local<Option<(u64, bool)>>,
) {
    // Only real data goes on the web chart: it has no notion of the simulated pit.
    let want = match (settings.web_chart, feed.mode, selection.selected_instrument) {
        (true, FeedMode::Live, Some(inst)) => symbol_id_for(inst).map(|_| (inst, settings.timeframe)),
        _ => None,
    };
    if want != *shown {
        match want {
            Some((inst, tf)) => {
                let id = symbol_id_for(inst).unwrap_or(0);
                js::show(id, &tape_symbol(inst), &format!("{} ({})", inst.name(), inst.symbol()), tf.label(), instrument_decimals(inst) as u32);
            }
            None => js::hide(),
        }
        *shown = want;
    }
    let Some((inst, tf)) = *shown else {
        *flow_sent = None;
        return;
    };
    let Some(id) = symbol_id_for(inst) else { return };
    // Flow panel: the newest bar's delta and tick count whenever the series changes.
    let series = store.series(inst, tf, FeedMode::Live);
    if flow_sent.map(|f| f.0) != Some(series.revision) {
        if let Some(b) = series.bars.back() {
            js::update_flow(id, b.time_ms as f64, b.delta as f64, b.volume as f64);
        }
    }
    if flow_sent.map(|f| f.1) != Some(settings.deal_markers) {
        js::set_deals(settings.deal_markers);
    }
    *flow_sent = Some((series.revision, settings.deal_markers));
    if !settings.deal_markers {
        return;
    }
    let pit = engines.pit(inst);
    let now_ms = wall_clock_ms() as f64;
    for e in pit.recent_events() {
        if let PitEventKind::Fill { side, price_ticks, .. } = e.kind {
            js::add_deal(id, now_ms, pit.price_of(price_ticks) as f64, side.is_buy());
        }
    }
}
