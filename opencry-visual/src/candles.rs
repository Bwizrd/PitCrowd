//! Candle store shared by the three chart views: OHLC bars per pit and timeframe from the
//! bridge's trendbar stream, backfilled over its REST API so a chart is full at startup, plus
//! a separate simulated series built from the pit's own prints so the two are never mixed.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use bevy::prelude::*;

use crate::live_feed::{FeedMode, FeedState, LiveFeed};
use crate::market::Instrument;
use crate::pit_engine::PitEngines;
use crate::scenario::MarketScenarioController;
use crate::ui::Selection;

pub const MAX_BARS: usize = 400;
/// Bars drawn on the board and wall charts.
pub const BARS_SHOWN: usize = 90;
/// Backfill hits cTrader's historical API through the bridge, which the live strategies
/// share and which rate-limits hard. So: one request at a time, only for the series a chart
/// is actually showing, never more often than this, and backing off on failure.
const BACKFILL_MIN_GAP_SECS: f64 = 2.0;
const BACKFILL_FIRST_RETRY_SECS: f64 = 45.0;
const BACKFILL_MAX_ATTEMPTS: u32 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Timeframe {
    #[default]
    M1,
    M5,
    M15,
}

impl Timeframe {
    pub const ALL: [Timeframe; 3] = [Timeframe::M1, Timeframe::M5, Timeframe::M15];

    pub fn label(self) -> &'static str {
        match self {
            Timeframe::M1 => "1m",
            Timeframe::M5 => "5m",
            Timeframe::M15 => "15m",
        }
    }

    pub fn from_label(s: &str) -> Option<Timeframe> {
        Timeframe::ALL.iter().copied().find(|t| t.label() == s)
    }

    pub fn millis(self) -> i64 {
        match self {
            Timeframe::M1 => 60_000,
            Timeframe::M5 => 300_000,
            Timeframe::M15 => 900_000,
        }
    }

    pub fn next(self) -> Timeframe {
        match self {
            Timeframe::M1 => Timeframe::M5,
            Timeframe::M5 => Timeframe::M15,
            Timeframe::M15 => Timeframe::M1,
        }
    }

    /// Bridge `range` parameter for a backfill that covers the bars shown with room to scroll.
    pub fn backfill_range(self) -> &'static str {
        match self {
            Timeframe::M1 => "-6h",
            Timeframe::M5 => "-2d",
            Timeframe::M15 => "-5d",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Candle {
    /// Bar open time, Unix milliseconds.
    pub time_ms: i64,
    pub open: f32,
    pub high: f32,
    pub low: f32,
    pub close: f32,
    /// cTrader tick volume: updates in the bar. Present on backfilled bars too.
    pub volume: f32,
    /// Upticks minus downticks in this bar (tick rule on the bid). Counted live from the
    /// tape's ticks; a tape backfill (`/bars?delta=1`) brings it for history too. Zero on
    /// bars the bridge backfilled, because the broker's history keeps no tick direction.
    pub delta: f32,
    /// Largest single-tick move in the bar, in pit ticks. Session only.
    pub sweep: f32,
}

impl Candle {
    pub fn is_up(&self) -> bool {
        self.close >= self.open
    }
}

#[derive(Debug, Clone, Default)]
pub struct Series {
    /// Oldest first, unique open times.
    pub bars: VecDeque<Candle>,
    pub revision: u64,
    pub backfilled: bool,
}

impl Series {
    /// Insert or replace the bar with this open time, keeping the series sorted.
    pub fn upsert(&mut self, mut c: Candle) {
        // Price fields come from the feed; session flow (delta, sweep) lives only here, so a
        // replacement bar keeps what the old one had accumulated.
        let merge = |old: &Candle, c: &mut Candle| {
            c.delta += old.delta;
            c.sweep = c.sweep.max(old.sweep);
            if c.volume <= 0.0 {
                c.volume = old.volume;
            }
        };
        match self.bars.back() {
            None => self.bars.push_back(c),
            Some(last) if c.time_ms > last.time_ms => self.bars.push_back(c),
            Some(last) if c.time_ms == last.time_ms => {
                merge(last, &mut c);
                *self.bars.back_mut().unwrap() = c;
            }
            Some(_) => {
                // Older than the newest: a backfill arriving after live bars. Merge in place.
                match self.bars.binary_search_by_key(&c.time_ms, |b| b.time_ms) {
                    Ok(i) => {
                        merge(&self.bars[i], &mut c);
                        self.bars[i] = c;
                    }
                    Err(i) => self.bars.insert(i, c),
                }
            }
        }
        while self.bars.len() > MAX_BARS {
            self.bars.pop_front();
        }
        self.revision += 1;
    }

    /// Insert a backfilled bar. When the source counted the bar's ticks itself
    /// (`flow_known`), its delta is the whole bar's and replaces whatever this session had
    /// counted so far, instead of being added to it; otherwise the session's count stands.
    pub fn upsert_backfill(&mut self, c: Candle, flow_known: bool) {
        let (time_ms, delta) = (c.time_ms, c.delta);
        self.upsert(c);
        if flow_known && let Ok(i) = self.bars.binary_search_by_key(&time_ms, |b| b.time_ms) {
            self.bars[i].delta = delta;
        }
    }

    /// Fold one traded price into the bar that contains `time_ms` (used for simulated flow).
    /// `dir` is +1 for an uptick, -1 for a downtick, 0 for unchanged.
    pub fn apply_print(&mut self, time_ms: i64, price: f32, tf: Timeframe, dir: i32) {
        let open_ms = time_ms - time_ms.rem_euclid(tf.millis());
        match self.bars.back_mut() {
            Some(last) if last.time_ms == open_ms => {
                last.high = last.high.max(price);
                last.low = last.low.min(price);
                last.close = price;
                last.volume += 1.0;
                last.delta += dir as f32;
                self.revision += 1;
            }
            _ => self.upsert(Candle {
                time_ms: open_ms,
                open: price,
                high: price,
                low: price,
                close: price,
                volume: 1.0,
                delta: dir as f32,
                sweep: 0.0,
            }),
        }
    }

    /// Credit one real tick to the bar that contains `time_ms`: `d_ticks` is the signed move
    /// in pit ticks (unrounded). A bar that has not arrived from the feed yet is opened at
    /// `price` so the tick is not lost; the feed's own bar replaces its prices when it comes.
    /// A real tick: opens the bar if it is the first of its period, otherwise extends it,
    /// counts it, and signs it by the tick rule.
    pub fn apply_tick(&mut self, time_ms: i64, price: f32, d_ticks: f32, tf: Timeframe) {
        let open_ms = time_ms - time_ms.rem_euclid(tf.millis());
        let dir = if d_ticks > 1e-3 {
            1.0
        } else if d_ticks < -1e-3 {
            -1.0
        } else {
            0.0
        };
        let idx = self.bars.iter().rposition(|b| b.time_ms == open_ms);
        match idx {
            Some(i) => {
                let b = &mut self.bars[i];
                b.high = b.high.max(price);
                b.low = b.low.min(price);
                b.close = price;
                b.volume += 1.0;
                b.delta += dir;
                b.sweep = b.sweep.max(d_ticks.abs());
                self.revision += 1;
            }
            None if self.bars.back().is_none_or(|b| open_ms > b.time_ms) => self.upsert(Candle {
                time_ms: open_ms,
                open: price,
                high: price,
                low: price,
                close: price,
                volume: 1.0,
                delta: dir,
                sweep: d_ticks.abs(),
            }),
            None => {}
        }
    }

    /// The newest bars a chart draws, oldest first.
    pub fn shown(&self) -> Vec<Candle> {
        let n = self.bars.len().min(BARS_SHOWN);
        self.bars
            .iter()
            .skip(self.bars.len() - n)
            .copied()
            .collect()
    }
}

/// What the user is looking at: timeframe for all charts, and which optional views are on.
#[derive(Resource, Debug, Clone, PartialEq)]
pub struct ChartSettings {
    pub timeframe: Timeframe,
    pub board_chart: bool,
    pub wall_board: bool,
    pub web_chart: bool,
    /// Debug overlay: the pit's own (synthetic) fills on the charts. Off by default because
    /// they are the model's output, not market data.
    pub deal_markers: bool,
    /// Resting-order levels on the charts: swings, blocks, gaps, OANDA clusters.
    pub levels: bool,
}

impl Default for ChartSettings {
    fn default() -> Self {
        Self {
            timeframe: Timeframe::M5,
            board_chart: false, // panels start closed (`panels::PanelPrefs::default`)
            wall_board: true,
            web_chart: false,
            deal_markers: false,
            levels: true,
        }
    }
}

impl ChartSettings {
    pub fn cycle_timeframe(&mut self, controller: &mut MarketScenarioController) {
        self.timeframe = self.timeframe.next();
        controller.announcement_banner =
            format!("CHARTS: {} BARS", self.timeframe.label().to_uppercase());
        controller.banner_timer = 1.5;
    }

    pub fn toggle_board(&mut self, controller: &mut MarketScenarioController) {
        self.board_chart = !self.board_chart;
        controller.announcement_banner = if self.board_chart {
            "BOARD CHART ON".into()
        } else {
            "BOARD CHART OFF".into()
        };
        controller.banner_timer = 1.5;
    }

    pub fn toggle_wall(&mut self, controller: &mut MarketScenarioController) {
        self.wall_board = !self.wall_board;
        controller.announcement_banner = if self.wall_board {
            "WALL BOARD ON".into()
        } else {
            "WALL BOARD OFF".into()
        };
        controller.banner_timer = 1.5;
    }

    pub fn toggle_deals(&mut self, controller: &mut MarketScenarioController) {
        self.deal_markers = !self.deal_markers;
        controller.announcement_banner = if self.deal_markers {
            "PIT DEALS ON CHARTS (DEBUG - NOT MARKET DATA)".into()
        } else {
            "PIT DEALS OFF CHARTS".into()
        };
        controller.banner_timer = 2.0;
    }

    pub fn toggle_web(&mut self, controller: &mut MarketScenarioController) {
        self.web_chart = !self.web_chart;
        controller.announcement_banner = if self.web_chart {
            if cfg!(target_arch = "wasm32") {
                "WEB CHART ON".into()
            } else {
                "WEB CHART IS BROWSER-ONLY".into()
            }
        } else {
            "WEB CHART OFF".into()
        };
        controller.banner_timer = 2.0;
    }
}

/// Bars that came back, and whether their tick delta is the source's (tape) or unknown (bridge).
type Pending = Arc<Mutex<Vec<(Instrument, Timeframe, Result<(Vec<Candle>, bool), String>)>>>;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Backfill {
    InFlight,
    Done,
    /// Failed `attempts` times; try again at `retry_at` (app seconds).
    Failed {
        attempts: u32,
        retry_at: f64,
    },
    GaveUp,
}

#[derive(Resource)]
pub struct CandleStore {
    live: Vec<[Series; 3]>,
    sim: Vec<[Series; 3]>,
    pending: Pending,
    backfill: HashMap<(usize, usize), Backfill>,
    next_request_at: f64,
    pub backfill_errors: u32,
}

impl Default for CandleStore {
    fn default() -> Self {
        Self {
            live: Instrument::ALL.iter().map(|_| Default::default()).collect(),
            sim: Instrument::ALL.iter().map(|_| Default::default()).collect(),
            pending: Arc::new(Mutex::new(Vec::new())),
            backfill: HashMap::new(),
            next_request_at: 0.0,
            backfill_errors: 0,
        }
    }
}

impl CandleStore {
    pub fn series(&self, inst: Instrument, tf: Timeframe, mode: FeedMode) -> &Series {
        let set = match mode {
            FeedMode::Live => &self.live,
            FeedMode::Simulated => &self.sim,
        };
        &set[inst as usize][tf as usize]
    }

    pub fn live_mut(&mut self, inst: Instrument, tf: Timeframe) -> &mut Series {
        &mut self.live[inst as usize][tf as usize]
    }

    /// One real tick from the tape, folded into every timeframe's current bar.
    pub fn apply_live_tick(
        &mut self,
        inst: Instrument,
        bar_time_ms: i64,
        price: f32,
        d_ticks: f32,
    ) {
        for tf in Timeframe::ALL {
            self.live[inst as usize][tf as usize].apply_tick(bar_time_ms, price, d_ticks, tf);
        }
    }

    pub fn apply_sim_print(&mut self, inst: Instrument, time_ms: i64, price: f32, dir: i32) {
        for tf in Timeframe::ALL {
            self.sim[inst as usize][tf as usize].apply_print(time_ms, price, tf, dir);
        }
    }

    /// Whether the series a chart wants still needs (and may) be fetched right now.
    fn backfill_due(&self, key: (usize, usize), now: f64) -> bool {
        match self.backfill.get(&key) {
            None => true,
            Some(Backfill::Failed { retry_at, .. }) => now >= *retry_at,
            Some(Backfill::InFlight | Backfill::Done | Backfill::GaveUp) => false,
        }
    }

    /// Fetch history for one series if a chart is showing it and nothing is in flight.
    /// Returns true if a request went out.
    pub fn ensure_backfilled(&mut self, inst: Instrument, tf: Timeframe, now: f64) -> bool {
        let key = (inst as usize, tf as usize);
        if now < self.next_request_at || !self.backfill_due(key, now) {
            return false;
        }
        if self.backfill.values().any(|b| *b == Backfill::InFlight) {
            return false;
        }
        let Some(id) = crate::live_feed::symbol_id_for(inst) else {
            return false;
        };
        let attempts = match self.backfill.get(&key) {
            Some(Backfill::Failed { attempts, .. }) => *attempts,
            _ => 0,
        };
        self.backfill.insert(key, Backfill::InFlight);
        self.next_request_at = now + BACKFILL_MIN_GAP_SECS;
        let bridge = bridge_url();
        let source = backfill_source(inst, tf, attempts, bridge.is_some());
        let url = match source {
            BackfillSource::Bridge => {
                format!(
                    "{}/getData?pair={id}&timeframe={}&range={}",
                    bridge.unwrap_or_default(),
                    tf.label(),
                    tf.backfill_range()
                )
            }
            // `delta=1`: the tape counts each bar's upticks and downticks from its own ticks.
            BackfillSource::Tape => format!(
                "{}/bars?symbol={}&timeframe={}&range={}&delta=1",
                crate::levels::tape_url(),
                crate::levels::tape_symbol(inst),
                tf.label(),
                tf.backfill_range()
            ),
        };
        info!(
            "candles: backfill {} {} from the {source:?} (attempt {})",
            inst.symbol(),
            tf.label(),
            attempts + 1
        );
        let pending = self.pending.clone();
        ehttp::fetch(ehttp::Request::get(url), move |res| {
            let out = match res {
                Ok(r) if r.ok => Ok(parse_backfill_flow(r.text().unwrap_or(""))),
                Ok(r) => Err(format!(
                    "HTTP {} {}",
                    r.status,
                    r.text().unwrap_or("").chars().take(120).collect::<String>()
                )),
                Err(e) => Err(e),
            };
            if let Ok(mut p) = pending.lock() {
                p.push((inst, tf, out));
            }
        });
        true
    }

    /// One line for the HUD: what backfill is doing, if anything is wrong.
    pub fn backfill_note(&self, inst: Instrument, tf: Timeframe) -> Option<&'static str> {
        match self.backfill.get(&(inst as usize, tf as usize)) {
            Some(Backfill::InFlight) | None => Some("loading history"),
            Some(Backfill::Failed { .. }) => Some("history unavailable, retrying"),
            Some(Backfill::GaveUp) => Some("history unavailable (bridge and tape both failed)"),
            Some(Backfill::Done) => None,
        }
    }

    fn drain_pending(&mut self, now: f64) {
        let done: Vec<_> = match self.pending.lock() {
            Ok(mut p) => p.drain(..).collect(),
            Err(_) => return,
        };
        for (inst, tf, res) in done {
            let key = (inst as usize, tf as usize);
            match res {
                Ok((bars, flow_known)) => {
                    let n = {
                        let s = self.live_mut(inst, tf);
                        for b in bars {
                            s.upsert_backfill(b, flow_known);
                        }
                        s.backfilled = true;
                        s.bars.len()
                    };
                    self.backfill.insert(key, Backfill::Done);
                    info!(
                        "candles: backfilled {} {} ({n} bars)",
                        inst.symbol(),
                        tf.label()
                    );
                }
                Err(e) => {
                    self.backfill_errors += 1;
                    let attempts = match self.backfill.get(&key) {
                        Some(Backfill::Failed { attempts, .. }) => attempts + 1,
                        _ => 1,
                    };
                    let state = if attempts >= BACKFILL_MAX_ATTEMPTS {
                        Backfill::GaveUp
                    } else {
                        Backfill::Failed {
                            attempts,
                            retry_at: now
                                + BACKFILL_FIRST_RETRY_SECS * 2f64.powi(attempts as i32 - 1),
                        }
                    };
                    self.backfill.insert(key, state);
                    warn!(
                        "candles: backfill {} {} failed ({attempts}): {e}",
                        inst.symbol(),
                        tf.label()
                    );
                }
            }
        }
    }
}

/// Where a backfill request goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackfillSource {
    /// The cTrader bridge's `/getData` (the broker's history API, rate-limited, years deep).
    Bridge,
    /// The market tape's `/bars`, aggregated from its own tick bucket (30 days deep, no limit).
    Tape,
}

/// With no bridge configured, everything comes from the tape. With one: 1m bars come from
/// the tape first (it costs nothing against a broker's history limit, and is the only source
/// that knows each bar's tick delta); 5m and 15m go to the bridge first (deeper history) and
/// fall over to the tape. Each source fails over to the other on retry. Bitcoin only ever
/// uses the tape: the bridge's history route has no entry for it.
pub fn backfill_source(
    inst: Instrument,
    tf: Timeframe,
    attempts: u32,
    bridge: bool,
) -> BackfillSource {
    if !bridge || inst == Instrument::Bitcoin {
        return BackfillSource::Tape;
    }
    let tape_first = tf == Timeframe::M1;
    if tape_first == attempts.is_multiple_of(2) {
        BackfillSource::Tape
    } else {
        BackfillSource::Bridge
    }
}

/// Base URL of a broker bridge's REST API, only when one is configured (`?bridge=` in the
/// browser, `OPENCRY_BRIDGE_URL` natively). Without one every backfill goes to the tape.
pub fn bridge_url() -> Option<String> {
    crate::endpoints::bridge_url()
}

/// Wall clock in Unix milliseconds, for stamping deals against bar times.
pub fn wall_clock_ms() -> i64 {
    #[cfg(target_arch = "wasm32")]
    {
        js_sys::Date::now() as i64
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0)
    }
}

/// "2026-10-21T09:40:00.000Z" -> Unix ms. Only the bridge's own format is accepted.
pub fn parse_iso_ms(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() < 19 {
        return None;
    }
    let num = |a: usize, n: usize| s.get(a..a + n)?.parse::<i64>().ok();
    let (y, mo, d) = (num(0, 4)?, num(5, 2)?, num(8, 2)?);
    let (h, mi, sec) = (num(11, 2)?, num(14, 2)?, num(17, 2)?);
    let ms = if b.len() >= 23 && b[19] == b'.' {
        num(20, 3)?
    } else {
        0
    };
    Some(((days_from_civil(y, mo, d) * 86_400 + h * 3600 + mi * 60 + sec) * 1000) + ms)
}

/// Days since 1970-01-01 for a proleptic Gregorian date (Howard Hinnant's algorithm).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The bridge answers either a bare array or `{data: [...]}`, with `timestamp` in ms.
pub fn parse_backfill(text: &str) -> Vec<Candle> {
    parse_backfill_flow(text).0
}

/// `parse_backfill`, also saying whether the bars carry their own tick delta: the tape's
/// `/bars?delta=1` rows have `upticks` and `downticks`, and `delta` is their difference,
/// the same quantity `Series::apply_tick` accumulates live (+1 per uptick, -1 per downtick).
/// Rows without them (the bridge, or tape bars past its delta window) get delta 0, and the
/// flag is false unless at least one row had counts.
pub fn parse_backfill_flow(text: &str) -> (Vec<Candle>, bool) {
    let v: serde_json::Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(_) => return (Vec::new(), false),
    };
    let rows = match &v {
        serde_json::Value::Array(a) => a.as_slice(),
        serde_json::Value::Object(o) => match o.get("data") {
            Some(serde_json::Value::Array(a)) => a.as_slice(),
            _ => &[],
        },
        _ => &[],
    };
    let f = |r: &serde_json::Value, k: &str| r.get(k).and_then(|x| x.as_f64());
    let mut flow_known = false;
    let bars = rows
        .iter()
        .filter_map(|r| {
            let delta = match (f(r, "upticks"), f(r, "downticks")) {
                (Some(up), Some(down)) => {
                    flow_known = true;
                    (up - down) as f32
                }
                _ => 0.0,
            };
            Some(Candle {
                time_ms: f(r, "timestamp")? as i64,
                open: f(r, "open")? as f32,
                high: f(r, "high")? as f32,
                low: f(r, "low")? as f32,
                close: f(r, "close")? as f32,
                volume: f(r, "volume").unwrap_or(0.0) as f32,
                delta,
                sweep: 0.0,
            })
        })
        .collect();
    (bars, flow_known)
}

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

/// Backfill from the bridge once the stream is up, a few requests a second, and fold in
/// what has come back. Simulated flow folds the pit's own prints into its own series.
pub fn update_candles(
    time: Res<Time>,
    feed: Res<LiveFeed>,
    engines: Res<PitEngines>,
    selection: Res<Selection>,
    settings: Res<ChartSettings>,
    mut store: ResMut<CandleStore>,
) {
    let now = time.elapsed_secs_f64();
    store.drain_pending(now);
    // Only the series on screen is worth a request against the rate-limited history API.
    let charts_on = settings.board_chart || settings.wall_board;
    if feed.state == FeedState::Live
        && feed.mode == FeedMode::Live
        && charts_on
        && let Some(inst) = selection.selected_instrument
    {
        store.ensure_backfilled(inst, settings.timeframe, now);
    }
    // Every live pit keeps a 1m history for the paper traders and their levels: one request
    // at a time, through the same gate and backoff as the chart's own backfill.
    if feed.mode == FeedMode::Live && feed.state == FeedState::Live {
        for inst in Instrument::ALL {
            if matches!(
                feed.pit_status(inst, now),
                crate::live_feed::PitFeedStatus::Live { .. }
            ) {
                store.ensure_backfilled(inst, Timeframe::M1, now);
            }
        }
    }
    if feed.mode == FeedMode::Simulated {
        let ms = wall_clock_ms();
        for inst in Instrument::ALL {
            let pit = engines.pit(inst);
            for e in pit.recent_events() {
                if let crate::pit_engine::PitEventKind::Print { up, .. } = e.kind {
                    store.apply_sim_print(inst, ms, pit.last_price(), if up { 1 } else { -1 });
                }
            }
        }
    }
}

/// C cycles the timeframe, B toggles the wall board, H toggles the web (HTML) chart.
pub fn handle_chart_keys(
    keyboard: Res<ButtonInput<KeyCode>>,
    mut settings: ResMut<ChartSettings>,
    mut controller: ResMut<MarketScenarioController>,
    mut selection: ResMut<Selection>,
) {
    if keyboard.just_pressed(KeyCode::KeyC) {
        settings.cycle_timeframe(&mut controller);
    }
    if keyboard.just_pressed(KeyCode::KeyB) {
        settings.toggle_wall(&mut controller);
        selection.wall_board_dismissed = false;
    }
    if keyboard.just_pressed(KeyCode::KeyH) {
        settings.toggle_web(&mut controller);
    }
    if keyboard.just_pressed(KeyCode::KeyN) {
        settings.toggle_board(&mut controller);
    }
    if keyboard.just_pressed(KeyCode::KeyO) {
        crate::levels::toggle_levels(&mut settings, &mut controller);
    }
    if keyboard.just_pressed(KeyCode::KeyG) {
        crate::ui::open_guide();
    }
}

/// Chart buttons on the bottom bar: one per chart, plus the timeframe cycle.
pub fn handle_chart_buttons(
    interaction_query: Query<
        (&Interaction, &crate::ui::TouchButtonAction),
        (Changed<Interaction>, With<Button>),
    >,
    mut settings: ResMut<ChartSettings>,
    mut controller: ResMut<MarketScenarioController>,
    mut selection: ResMut<Selection>,
) {
    use crate::ui::TouchButtonAction as A;
    for (interaction, action) in interaction_query.iter() {
        if *interaction != Interaction::Pressed {
            continue;
        }
        match action {
            A::ChartBoard => settings.toggle_board(&mut controller),
            A::ChartWeb => settings.toggle_web(&mut controller),
            A::ChartWall => {
                settings.toggle_wall(&mut controller);
                selection.wall_board_dismissed = false;
            }
            A::ChartTimeframe => settings.cycle_timeframe(&mut controller),
            A::ChartDeals => settings.toggle_deals(&mut controller),
            A::ChartLevels => crate::levels::toggle_levels(&mut settings, &mut controller),
            A::Guide => crate::ui::open_guide(),
            _ => {}
        }
    }
}

/// Chart buttons light up when their chart is on; the timeframe button names the timeframe.
pub fn update_chart_button_styles(
    settings: Res<ChartSettings>,
    mut buttons: Query<
        (
            &crate::ui::TouchButtonAction,
            &mut BackgroundColor,
            &mut BorderColor,
            &Children,
        ),
        With<Button>,
    >,
    mut texts: Query<&mut Text>,
) {
    if !settings.is_changed() {
        return;
    }
    use crate::ui::TouchButtonAction as A;
    for (action, mut bg, mut border, children) in buttons.iter_mut() {
        let on = match action {
            A::ChartBoard => settings.board_chart,
            A::ChartWeb => settings.web_chart,
            A::ChartWall => settings.wall_board,
            A::ChartDeals => settings.deal_markers,
            A::ChartLevels => settings.levels,
            A::Guide => false,
            A::ChartTimeframe => {
                for child in children.iter() {
                    if let Ok(mut t) = texts.get_mut(child) {
                        crate::pit::set_ui_text(
                            &mut t,
                            format!("{} bars", settings.timeframe.label()),
                        );
                    }
                }
                continue;
            }
            _ => continue,
        };
        if on {
            *bg = BackgroundColor(Color::srgba(0.08, 0.32, 0.20, 0.95));
            *border = BorderColor::all(Color::srgb(0.35, 0.85, 0.50));
        } else {
            *bg = BackgroundColor(Color::srgba(0.06, 0.09, 0.14, 0.85));
            *border = BorderColor::all(Color::srgba(0.18, 0.28, 0.40, 0.55));
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn one_minute_bars_come_from_the_tape_first_and_each_source_fails_over() {
        use super::{BackfillSource, Timeframe, backfill_source};
        use crate::market::Instrument;
        for tf in Timeframe::ALL {
            for attempt in 0..4 {
                assert_eq!(
                    backfill_source(Instrument::Bitcoin, tf, attempt, true),
                    BackfillSource::Tape,
                    "Bitcoin never asks the bridge"
                );
            }
        }
        assert_eq!(
            backfill_source(Instrument::Silver, Timeframe::M1, 0, true),
            BackfillSource::Tape
        );
        assert_eq!(
            backfill_source(Instrument::Silver, Timeframe::M1, 1, true),
            BackfillSource::Bridge
        );
        assert_eq!(
            backfill_source(Instrument::Silver, Timeframe::M1, 2, true),
            BackfillSource::Tape
        );
        assert_eq!(
            backfill_source(Instrument::Silver, Timeframe::M5, 0, true),
            BackfillSource::Bridge
        );
        assert_eq!(
            backfill_source(Instrument::Silver, Timeframe::M5, 1, true),
            BackfillSource::Tape
        );
        assert_eq!(
            backfill_source(Instrument::Gold, Timeframe::M15, 0, true),
            BackfillSource::Bridge
        );
        assert_eq!(
            backfill_source(Instrument::Gold, Timeframe::M15, 1, true),
            BackfillSource::Tape
        );
        for attempt in 0..4 {
            assert_eq!(
                backfill_source(Instrument::Gold, Timeframe::M15, attempt, false),
                BackfillSource::Tape,
                "no bridge, no bridge requests"
            );
        }
    }

    #[test]
    fn tape_bars_with_tick_counts_carry_delta_and_bridge_bars_do_not() {
        use super::parse_backfill_flow;
        // Two rows as `/bars?delta=1` returns them, the second past the tape's delta window.
        let tape = r#"{"symbol":"XAGUSD","timeframe":"1m","delta":true,"count":2,"data":[
            {"timestamp":1792620120000,"open":57.85,"high":57.87,"low":57.84,"close":57.86,"volume":55,"upticks":31,"downticks":23,"spread_mean":0.0403},
            {"timestamp":1792620060000,"open":57.84,"high":57.85,"low":57.83,"close":57.85,"volume":32}]}"#;
        let (bars, flow_known) = parse_backfill_flow(tape);
        assert!(flow_known);
        assert_eq!(bars.len(), 2);
        assert_eq!(
            bars[0].delta, 8.0,
            "upticks minus downticks, like apply_tick's +1/-1 per tick"
        );
        assert_eq!(bars[0].volume, 55.0);
        assert_eq!(bars[1].delta, 0.0, "no counts, no delta");
        let bridge = r#"[{"timestamp":1792564860000,"open":57.193,"high":57.206,"low":57.183,"close":57.195,"volume":168}]"#;
        let (bars, flow_known) = parse_backfill_flow(bridge);
        assert!(!flow_known);
        assert_eq!(bars[0].delta, 0.0);
    }

    #[test]
    fn tape_backfill_replaces_the_session_count_and_bridge_backfill_keeps_it() {
        use super::{Candle, Series, Timeframe};
        let mut s = Series::default();
        // Three live ticks in the open bar: +1 +1 -1.
        s.apply_tick(300_000, 10.0, 1.0, Timeframe::M5);
        s.apply_tick(310_000, 10.1, 1.0, Timeframe::M5);
        s.apply_tick(320_000, 10.0, -1.0, Timeframe::M5);
        assert_eq!(s.bars[0].delta, 1.0);
        let bar = |delta: f32| Candle {
            time_ms: 300_000,
            open: 9.9,
            high: 10.2,
            low: 9.8,
            close: 10.0,
            volume: 40.0,
            delta,
            ..Default::default()
        };
        // The tape counted the whole bar, including ticks before this session: its figure wins.
        s.upsert_backfill(bar(7.0), true);
        assert_eq!(s.bars.len(), 1);
        assert_eq!(s.bars[0].delta, 7.0);
        assert_eq!(s.bars[0].volume, 40.0);
        // The bridge knows nothing about direction: the count so far stands.
        s.upsert_backfill(bar(0.0), false);
        assert_eq!(s.bars[0].delta, 7.0);
        // Live ticks keep accumulating on top of the backfilled figure.
        s.apply_tick(330_000, 10.1, 1.0, Timeframe::M5);
        assert_eq!(s.bars[0].delta, 8.0);
        // A tape bar for a period the session never saw lands with its delta intact.
        s.upsert_backfill(
            Candle {
                time_ms: 0,
                open: 9.0,
                high: 9.5,
                low: 8.9,
                close: 9.4,
                volume: 12.0,
                delta: -3.0,
                ..Default::default()
            },
            true,
        );
        assert_eq!(s.bars[0].delta, -3.0);
    }

    use super::*;

    #[test]
    fn iso_times_parse_to_unix_ms() {
        assert_eq!(parse_iso_ms("1970-01-01T00:00:00.000Z"), Some(0));
        assert_eq!(
            parse_iso_ms("2026-10-21T09:40:00.000Z"),
            Some(1_792_575_600_000)
        );
        assert_eq!(
            parse_iso_ms("2026-10-21T09:40:00Z"),
            Some(1_792_575_600_000)
        );
        assert_eq!(parse_iso_ms("garbage"), None);
    }

    #[test]
    fn backfill_accepts_both_response_shapes() {
        let bare = r#"[{"timestamp":1792564860000,"open":57.193,"high":57.206,"low":57.183,"close":57.195,"volume":168}]"#;
        let wrapped = format!(r#"{{"entriesCount":1,"symbolName":"XAGUSD_SB","data":{bare}}}"#);
        assert_eq!(parse_backfill(bare).len(), 1);
        assert_eq!(parse_backfill(&wrapped)[0].time_ms, 1_792_564_860_000);
        assert!(parse_backfill("nope").is_empty());
    }

    #[test]
    fn series_upserts_in_time_order_and_merges_late_backfill() {
        let mut s = Series::default();
        let c = |t: i64, px: f32| Candle {
            time_ms: t,
            open: px,
            high: px,
            low: px,
            close: px,
            volume: 1.0,
            ..Default::default()
        };
        s.upsert(c(300_000, 3.0));
        s.upsert(c(360_000, 4.0));
        s.upsert(c(360_000, 4.5)); // same bar updated
        s.upsert(c(240_000, 2.0)); // late backfill, older
        s.upsert(c(300_000, 3.5)); // backfill overwriting a live bar
        let times: Vec<i64> = s.bars.iter().map(|b| b.time_ms).collect();
        assert_eq!(times, vec![240_000, 300_000, 360_000]);
        assert_eq!(s.bars[1].close, 3.5);
        assert_eq!(s.bars[2].close, 4.5);
        assert_eq!(s.revision, 5);
    }

    #[test]
    fn prints_fold_into_bars_by_timeframe() {
        let mut s = Series::default();
        s.apply_print(61_000, 10.0, Timeframe::M1, 1);
        s.apply_print(70_000, 11.0, Timeframe::M1, 1);
        s.apply_print(90_000, 9.0, Timeframe::M1, -1);
        s.apply_print(125_000, 9.5, Timeframe::M1, 1);
        assert_eq!(s.bars.len(), 2);
        let b = s.bars[0];
        assert_eq!(
            (b.time_ms, b.open, b.high, b.low, b.close),
            (60_000, 10.0, 11.0, 9.0, 9.0)
        );
        assert_eq!(b.delta, 1.0, "two upticks and a downtick");
        assert_eq!(s.bars[1].time_ms, 120_000);
    }

    #[test]
    fn live_ticks_credit_delta_and_sweep_and_survive_feed_updates() {
        let mut s = Series::default();
        // Ticks arrive before the feed's 5m bar: the bar is opened for them.
        s.apply_tick(300_000, 10.0, 0.3, Timeframe::M5);
        s.apply_tick(310_000, 10.1, 1.0, Timeframe::M5);
        s.apply_tick(320_000, 10.1, 0.0, Timeframe::M5);
        s.apply_tick(330_000, 9.7, -4.0, Timeframe::M5);
        assert_eq!(s.bars.len(), 1);
        assert_eq!(s.bars[0].delta, 1.0, "+1 +1 0 -1");
        assert_eq!(s.bars[0].sweep, 4.0);
        // The feed's own bar replaces prices but keeps the session flow.
        s.upsert(Candle {
            time_ms: 300_000,
            open: 10.0,
            high: 10.2,
            low: 9.6,
            close: 9.7,
            volume: 40.0,
            ..Default::default()
        });
        assert_eq!(s.bars.len(), 1);
        assert_eq!(s.bars[0].delta, 1.0);
        assert_eq!(s.bars[0].volume, 40.0);
        // A tick for a bar older than the newest is ignored rather than inserted out of order.
        s.apply_tick(600_000, 9.8, 0.5, Timeframe::M5);
        s.apply_tick(100_000, 9.0, 0.5, Timeframe::M5);
        assert_eq!(s.bars.len(), 2);
    }

    #[test]
    fn backfill_is_one_at_a_time_and_backs_off_after_failure() {
        let mut store = CandleStore::default();
        let key = (Instrument::Gold as usize, Timeframe::M5 as usize);
        assert!(store.backfill_due(key, 0.0));
        store.backfill.insert(key, Backfill::InFlight);
        assert!(!store.backfill_due(key, 10.0));
        assert!(
            !store.ensure_backfilled(Instrument::Silver, Timeframe::M5, 10.0),
            "nothing else while one is in flight"
        );
        store.pending.lock().unwrap().push((
            Instrument::Gold,
            Timeframe::M5,
            Err("HTTP 500".into()),
        ));
        store.drain_pending(10.0);
        assert!(matches!(
            store.backfill.get(&key),
            Some(Backfill::Failed { attempts: 1, .. })
        ));
        assert!(!store.backfill_due(key, 20.0), "not before the retry time");
        assert!(store.backfill_due(key, 10.0 + BACKFILL_FIRST_RETRY_SECS + 0.1));
        for n in 2..=BACKFILL_MAX_ATTEMPTS {
            store.pending.lock().unwrap().push((
                Instrument::Gold,
                Timeframe::M5,
                Err("HTTP 500".into()),
            ));
            store.drain_pending(100.0 * n as f64);
        }
        assert_eq!(store.backfill.get(&key), Some(&Backfill::GaveUp));
        assert!(!store.backfill_due(key, 1e9));
        assert_eq!(
            store.backfill_note(Instrument::Gold, Timeframe::M5),
            Some("history unavailable (bridge and tape both failed)")
        );
        // A success lands bars and marks the series done.
        let k2 = (Instrument::Silver as usize, Timeframe::M1 as usize);
        store.backfill.insert(k2, Backfill::InFlight);
        let bar = Candle {
            time_ms: 60_000,
            open: 1.0,
            high: 1.0,
            low: 1.0,
            close: 1.0,
            volume: 1.0,
            ..Default::default()
        };
        store.pending.lock().unwrap().push((
            Instrument::Silver,
            Timeframe::M1,
            Ok((vec![bar], false)),
        ));
        store.drain_pending(500.0);
        assert_eq!(store.backfill.get(&k2), Some(&Backfill::Done));
        assert_eq!(
            store
                .series(Instrument::Silver, Timeframe::M1, FeedMode::Live)
                .bars
                .len(),
            1
        );
    }

    #[test]
    fn timeframes_cycle_and_name_themselves() {
        assert_eq!(Timeframe::M1.next().next().next(), Timeframe::M1);
        assert_eq!(Timeframe::from_label("15m"), Some(Timeframe::M15));
        assert_eq!(Timeframe::from_label("4h"), None);
        assert_eq!(Timeframe::M5.millis(), 300_000);
    }
}
