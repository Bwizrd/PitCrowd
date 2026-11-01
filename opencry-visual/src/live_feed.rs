//! Live prices from the market tape (`floor_tape`, or any server speaking its protocol, port
//! 8083; docs/DATA_SOURCES.md): every bid/ask tick and every change of the 5-level book, as
//! they happen.
//!
//! The stream broadcasts every symbol's live bars to every client on each tick, so the app
//! simply connects and keeps the 1-minute bar of the symbols that map to pits. Each update
//! re-anchors that pit's fair value to the real bid, and the update rate sets how busy the
//! pit is. The participants, their quotes and their deals are still the engine's: what is
//! real is the price they trade around and how fast paper arrives.
//!
//! The rule Paul set: if the feed drops, the action stops and the screen says so. A pit only
//! steps while the socket is live and that pit has had a tick recently. There is no quiet
//! fallback to simulation; the simulated mode exists (key L) but wears a permanent banner.

use std::collections::VecDeque;
use std::sync::Mutex;

use bevy::prelude::*;
use serde::Deserialize;

use crate::market::Instrument;
use crate::pit_engine::PitEngines;
use crate::scenario::MarketScenarioController;

/// Seconds without any message on a live socket before the feed is declared stale.
pub const STALE_AFTER_SECS: f64 = 20.0;
/// Seconds without a tick before a pit is halted as quiet (its symbol is not trading).
pub const QUIET_AFTER_SECS: f64 = 90.0;
/// Window over which a pit's tick rate is measured.
pub const RATE_WINDOW_SECS: f64 = 10.0;
/// Window over which upticks and downticks are balanced into a direction.
pub const FLOW_WINDOW_SECS: f64 = 20.0;
/// Window over which the largest single-tick jump counts as sweep evidence.
pub const BURST_WINDOW_SECS: f64 = 5.0;
/// Weight of a zero tick (bid unchanged) carrying the previous direction.
const ZERO_TICK_WEIGHT: f32 = 0.25;
/// Seconds between reconnect attempts.
const RECONNECT_AFTER_SECS: f64 = 3.0;
pub const FEED_PORT: u16 = crate::endpoints::TAPE_PORT;

// ---------------------------------------------------------------------------
// Wire format
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct TapeEnvelope {
    #[serde(rename = "type")]
    kind: String,
    data: Option<serde_json::Value>,
}

/// One bid/ask update from the tape. `t` is the tape's receipt time in Unix ms.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct TapeTick {
    #[serde(rename = "symbolId")]
    pub symbol_id: u32,
    pub symbol: String,
    pub t: i64,
    pub bid: f64,
    pub ask: f64,
    #[serde(default)]
    pub spread: f64,
}

/// The broker's ladder after one change: (price, size) best first, up to five a side.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct TapeDepth {
    #[serde(rename = "symbolId")]
    pub symbol_id: u32,
    pub t: i64,
    pub bids: Vec<(f32, f32)>,
    pub asks: Vec<(f32, f32)>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TapeMessage {
    Tick(Instrument, TapeTick),
    Depth(Instrument, TapeDepth),
}

/// The tape's symbol ids (they began as a cTrader account's; floor_tape stamps the same). Every pit maps to a
/// symbol; `None` is kept so a pit without one would say so rather than trade on nothing.
pub fn symbol_id_for(inst: Instrument) -> Option<u32> {
    match inst {
        Instrument::Gold => Some(241),
        Instrument::Silver => Some(238),
        Instrument::Nasdaq => Some(205),
        Instrument::Sp500 => Some(220),
        Instrument::EurUsd => Some(185),
        Instrument::GbpUsd => Some(199),
        Instrument::Bitcoin => Some(160),
        Instrument::Us30 => Some(219),
        Instrument::Uk100 => Some(217),
    }
}

pub fn instrument_for_symbol_id(id: u32) -> Option<Instrument> {
    Instrument::ALL.iter().copied().find(|i| symbol_id_for(*i) == Some(id))
}

/// Parse one text frame from the tape into a tick or a book change for a pit we run.
pub fn parse_tape(text: &str) -> Option<TapeMessage> {
    let env: TapeEnvelope = serde_json::from_str(text).ok()?;
    let data = env.data?;
    match env.kind.as_str() {
        "tick" => {
            let tick: TapeTick = serde_json::from_value(data).ok()?;
            let inst = instrument_for_symbol_id(tick.symbol_id)?;
            Some(TapeMessage::Tick(inst, tick))
        }
        "depth" => {
            let depth: TapeDepth = serde_json::from_value(data).ok()?;
            let inst = instrument_for_symbol_id(depth.symbol_id)?;
            Some(TapeMessage::Depth(inst, depth))
        }
        _ => None,
    }
}

/// Pit activity (0..1) from the tick rate, in updates per second. A quiet pair at 0.2 ticks/s
/// keeps a few people trading slowly; two ticks a second is a full, busy pit.
pub fn activity_for_rate(rate: f32) -> f32 {
    (0.12 + rate * 0.44).clamp(0.12, 1.0)
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedMode {
    Live,
    Simulated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedState {
    Connecting,
    Live,
    Stale,
    Down,
}

/// Why a pit is, or is not, moving right now.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PitFeedStatus {
    /// Simulated mode: the engine runs on synthetic flow.
    Simulated,
    /// Ticking: seconds since the last tick and the current tick rate.
    Live { age: f32, rate: f32 },
    /// Socket is live but this symbol has not ticked for a while.
    Quiet { age: f32 },
    /// The account has no symbol for this pit.
    NoSymbol,
    /// Socket down or stale: nothing moves.
    FeedDown,
    /// Socket live, first tick for this pit not yet seen.
    Waiting,
}

#[derive(Debug, Clone)]
pub struct LiveQuote {
    /// Bid: the price the pit anchors to and the tick rule reads.
    pub price: f64,
    pub prev_price: f64,
    pub ask: f64,
    /// Ask minus bid in price units.
    pub spread: f64,
    /// Tape time of the last tick, Unix ms.
    pub tick_ms: i64,
    pub last_update: f64,
    pub updates: u64,
    /// Broker ladder, best first: (price, size).
    pub bids: Vec<(f32, f32)>,
    pub asks: Vec<(f32, f32)>,
    recent: VecDeque<f64>,
    /// Tick-rule classifications: (time, signed weight, |move| in ticks).
    flow: VecDeque<(f64, f32, f32)>,
    /// Last non-zero tick direction, carried onto zero ticks.
    last_dir: f32,
}

/// Tier A flow read off the tape with the tick rule: an uptick is a buyer lifting the offer,
/// a downtick a seller hitting the bid, an unchanged bid carries the last direction weakly.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct TickFlow {
    /// Signed balance -1..1, shrunk toward 0 when few ticks have been seen.
    pub direction: f32,
    /// Upticks as a share of classified ticks, 0..1 (0.5 when none).
    pub up_share: f32,
    /// Classified ticks in the flow window.
    pub ticks: u32,
    /// Largest one-update move in ticks inside the burst window.
    pub burst_ticks: f32,
}

impl LiveQuote {
    pub fn rate(&self, now: f64) -> f32 {
        let n = self.recent.iter().filter(|t| now - **t <= RATE_WINDOW_SECS).count();
        n as f32 / RATE_WINDOW_SECS as f32
    }

    /// Classify one update against the previous bid. `d_ticks` is the signed move in pit
    /// ticks, unrounded: the screen moves in finer steps than the pit (gold 0.01 against a
    /// 0.10 pit tick), and a small real move is still an uptick, not a zero tick.
    fn classify(&mut self, now: f64, d_ticks: f32) {
        let (dir, weight) = if d_ticks > 1e-3 {
            (1.0, 1.0 + 0.5 * (d_ticks - 1.0).clamp(0.0, 6.0))
        } else if d_ticks < -1e-3 {
            (-1.0, 1.0 + 0.5 * (-d_ticks - 1.0).clamp(0.0, 6.0))
        } else {
            (self.last_dir, ZERO_TICK_WEIGHT)
        };
        if dir != 0.0 {
            self.last_dir = dir;
            self.flow.push_back((now, dir * weight, d_ticks.abs()));
        }
        while self.flow.front().is_some_and(|f| now - f.0 > FLOW_WINDOW_SECS) {
            self.flow.pop_front();
        }
    }

    pub fn flow(&self, now: f64) -> TickFlow {
        let (mut up, mut down, mut n, mut burst) = (0.0f32, 0.0f32, 0u32, 0.0f32);
        for &(t, w, mv) in self.flow.iter() {
            if now - t > FLOW_WINDOW_SECS {
                continue;
            }
            n += 1;
            if w > 0.0 { up += w } else { down -= w }
            if now - t <= BURST_WINDOW_SECS {
                burst = burst.max(mv);
            }
        }
        let total = up + down;
        if total <= 0.0 {
            return TickFlow { direction: 0.0, up_share: 0.5, ticks: 0, burst_ticks: 0.0 };
        }
        let raw = (up - down) / total;
        // Two upticks are a lean, not a sweep: shrink toward neutral for small samples.
        let shrink = n as f32 / (n as f32 + 3.0);
        TickFlow { direction: raw * shrink, up_share: up / total, ticks: n, burst_ticks: burst }
    }
}

/// Map the tape's read onto the engine's flow inputs. Pure, so the mapping is testable and
/// easy to retune after the London-open comparison.
pub fn live_flow_for(flow: &TickFlow, rate: f32) -> crate::pit_engine::LiveFlow {
    crate::pit_engine::LiveFlow {
        arrivals_per_sec: (0.5 + 1.3 * rate).clamp(0.5, 4.0),
        buy_prob: (0.5 + 0.42 * flow.direction).clamp(0.08, 0.92),
        block_prob: (0.04 + 0.08 * (flow.burst_ticks - 1.0).max(0.0)).min(0.40),
    }
}

/// The socket, kept behind a mutex so the resource is `Sync`. On wasm the handles wrap JS
/// objects that are not `Send`; wasm32 without threads is single-threaded, so that is sound.
#[derive(Default)]
struct Socket(Mutex<Option<(ewebsock::WsSender, ewebsock::WsReceiver)>>);

#[cfg(target_arch = "wasm32")]
unsafe impl Send for Socket {}
#[cfg(target_arch = "wasm32")]
unsafe impl Sync for Socket {}

#[derive(Resource)]
pub struct LiveFeed {
    pub mode: FeedMode,
    pub state: FeedState,
    pub state_since: f64,
    pub last_message_at: f64,
    pub messages: u64,
    pub last_error: String,
    pub url: String,
    pub quotes: Vec<Option<LiveQuote>>,
    socket: Socket,
    next_connect_at: f64,
}

impl Default for LiveFeed {
    fn default() -> Self {
        Self {
            mode: FeedMode::Live,
            state: FeedState::Connecting,
            state_since: 0.0,
            last_message_at: 0.0,
            messages: 0,
            last_error: String::new(),
            url: default_feed_url(),
            quotes: (0..Instrument::ALL.len()).map(|_| None).collect(),
            socket: Socket::default(),
            next_connect_at: 0.0,
        }
    }
}

/// The tape's socket: the page's host on :8083 unless `?tape=` says otherwise; natively
/// `OPENCRY_FEED_URL` or `OPENCRY_TAPE_URL` (see `endpoints`).
pub fn default_feed_url() -> String {
    crate::endpoints::feed_url()
}

impl LiveFeed {
    pub fn quote(&self, inst: Instrument) -> Option<&LiveQuote> {
        self.quotes[inst as usize].as_ref()
    }

    fn set_state(&mut self, state: FeedState, now: f64) {
        if self.state != state {
            self.state = state;
            self.state_since = now;
        }
    }

    pub fn socket_live(&self) -> bool {
        self.state == FeedState::Live
    }

    /// What a pit should be doing now.
    pub fn pit_status(&self, inst: Instrument, now: f64) -> PitFeedStatus {
        if self.mode == FeedMode::Simulated {
            return PitFeedStatus::Simulated;
        }
        if symbol_id_for(inst).is_none() {
            return PitFeedStatus::NoSymbol;
        }
        if !self.socket_live() {
            return PitFeedStatus::FeedDown;
        }
        match self.quote(inst) {
            None => PitFeedStatus::Waiting,
            Some(q) => {
                let age = (now - q.last_update).max(0.0) as f32;
                if age as f64 > QUIET_AFTER_SECS {
                    PitFeedStatus::Quiet { age }
                } else {
                    PitFeedStatus::Live { age, rate: q.rate(now) }
                }
            }
        }
    }

    /// `Some(activity)` if the pit may step this frame, `None` if it is halted.
    pub fn pit_activity(&self, inst: Instrument, now: f64) -> Option<f32> {
        match self.pit_status(inst, now) {
            PitFeedStatus::Live { rate, .. } => Some(activity_for_rate(rate)),
            _ => None,
        }
    }

    /// The tape's read of a pit's flow, only while that pit is live.
    pub fn pit_flow(&self, inst: Instrument, now: f64) -> Option<crate::pit_engine::LiveFlow> {
        match self.pit_status(inst, now) {
            PitFeedStatus::Live { rate, .. } => {
                self.quote(inst).map(|q| live_flow_for(&q.flow(now), rate))
            }
            _ => None,
        }
    }

    /// Pits currently ticking, and their combined tick rate.
    pub fn live_summary(&self, now: f64) -> (usize, f32) {
        let mut n = 0;
        let mut rate = 0.0;
        for inst in Instrument::ALL {
            if let PitFeedStatus::Live { rate: r, .. } = self.pit_status(inst, now) {
                n += 1;
                rate += r;
            }
        }
        (n, rate)
    }

    /// Record a bar for a pit. Returns true if the price changed.
    pub fn ingest_tick(&mut self, inst: Instrument, tick: &TapeTick, now: f64) -> bool {
        let slot = &mut self.quotes[inst as usize];
        let q = slot.get_or_insert_with(|| LiveQuote {
            price: tick.bid,
            prev_price: tick.bid,
            ask: tick.ask,
            spread: tick.ask - tick.bid,
            tick_ms: tick.t,
            last_update: now,
            updates: 0,
            bids: Vec::new(),
            asks: Vec::new(),
            recent: VecDeque::new(),
            flow: VecDeque::new(),
            last_dir: 0.0,
        });
        let changed = (tick.bid - q.price).abs() > f64::EPSILON;
        let d_ticks = ((tick.bid - q.price) / crate::pit_engine::tick_size(inst) as f64) as f32;
        if q.updates > 0 {
            q.classify(now, d_ticks);
        }
        q.prev_price = q.price;
        q.price = tick.bid;
        q.ask = tick.ask;
        q.spread = tick.ask - tick.bid;
        q.tick_ms = tick.t;
        q.last_update = now;
        q.updates += 1;
        q.recent.push_back(now);
        while q.recent.front().is_some_and(|t| now - *t > RATE_WINDOW_SECS) {
            q.recent.pop_front();
        }
        changed
    }

    /// A book change: kept on the quote, does not count as a tick.
    pub fn ingest_depth(&mut self, inst: Instrument, depth: &TapeDepth) {
        if let Some(q) = self.quotes[inst as usize].as_mut() {
            q.bids = depth.bids.clone();
            q.asks = depth.asks.clone();
        }
    }

    /// Mark a message received; a socket that speaks is live, whatever it was before. The
    /// desk marks the feed `Down` itself when its socket errors and then reconnects without
    /// passing through `Connecting`, so `Down` has to recover here too (2026-10-22: a tape
    /// restart left the desk reading ticks for an hour with every pit halted).
    pub fn note_message(&mut self, now: f64) {
        self.messages += 1;
        self.last_message_at = now;
        if self.state != FeedState::Live {
            self.set_state(FeedState::Live, now);
        }
    }

    /// Age out a silent socket.
    pub fn check_staleness(&mut self, now: f64) {
        if self.state == FeedState::Live && now - self.last_message_at > STALE_AFTER_SECS {
            self.set_state(FeedState::Stale, now);
        }
    }

    fn drop_socket(&mut self, now: f64, reason: String) {
        if let Ok(mut guard) = self.socket.0.lock() {
            *guard = None;
        }
        self.last_error = reason;
        self.set_state(FeedState::Down, now);
        self.next_connect_at = now + RECONNECT_AFTER_SECS;
    }

    fn ensure_connected(&mut self, now: f64) {
        let connected = self.socket.0.lock().map(|g| g.is_some()).unwrap_or(false);
        if connected || now < self.next_connect_at {
            return;
        }
        match ewebsock::connect(self.url.clone(), ewebsock::Options::default()) {
            Ok(pair) => {
                if let Ok(mut guard) = self.socket.0.lock() {
                    *guard = Some(pair);
                }
                self.set_state(FeedState::Connecting, now);
                info!("live feed: connecting to {}", self.url);
            }
            Err(e) => {
                self.drop_socket(now, format!("connect failed: {e}"));
            }
        }
    }

    /// Switch mode. Leaving live mode releases the engines from their anchors.
    pub fn set_mode(&mut self, mode: FeedMode, engines: &mut PitEngines, controller: &mut MarketScenarioController, now: f64) {
        if mode == self.mode {
            return;
        }
        self.mode = mode;
        match mode {
            FeedMode::Simulated => {
                for pit in engines.pits.iter_mut() {
                    pit.clear_live_anchor();
                }
                controller.announcement_banner = "SIMULATED FLOW - NOT MARKET DATA  (L returns to live)".to_string();
                controller.banner_timer = 4.0;
                warn!("live feed: switched to SIMULATED flow");
            }
            FeedMode::Live => {
                controller.announcement_banner = "LIVE FEED - pits re-anchor on the next tick".to_string();
                controller.banner_timer = 3.0;
                self.next_connect_at = now;
                info!("live feed: switched to LIVE");
            }
        }
    }
}

/// Scenario and shock keys inject synthetic paper; refuse them while live and say why.
pub fn sim_only_guard(feed: &LiveFeed, controller: &mut MarketScenarioController) -> bool {
    if feed.mode == FeedMode::Live {
        controller.announcement_banner = "SCENARIOS AND SHOCKS ARE SIMULATION-ONLY  (press L for simulated flow)".to_string();
        controller.banner_timer = 3.0;
        false
    } else {
        true
    }
}

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

/// Drain the socket, record quotes, anchor the engines. Runs before `step_pit_engines`.
pub fn poll_live_feed(
    time: Res<Time>,
    mut feed: ResMut<LiveFeed>,
    mut engines: ResMut<PitEngines>,
    mut candles: ResMut<crate::candles::CandleStore>,
) {
    let now = time.elapsed_secs_f64();
    // The socket stays connected in simulated mode too, so switching back is instant.
    feed.ensure_connected(now);

    let mut events = Vec::new();
    if let Ok(guard) = feed.socket.0.lock() {
        if let Some((_, rx)) = guard.as_ref() {
            while let Some(ev) = rx.try_recv() {
                events.push(ev);
                if events.len() > 4096 {
                    break;
                }
            }
        }
    }

    for ev in events {
        match ev {
            ewebsock::WsEvent::Opened => {
                feed.note_message(now);
                info!("live feed: socket open {}", feed.url);
            }
            ewebsock::WsEvent::Message(ewebsock::WsMessage::Text(text)) => {
                feed.note_message(now);
                match parse_tape(&text) {
                    Some(TapeMessage::Tick(inst, tick)) => {
                        let prev = feed.quote(inst).map(|q| q.price);
                        feed.ingest_tick(inst, &tick, now);
                        // The candles are built from the tape itself: the first tick of a minute
                        // opens the bar, every tick moves it, and the tick rule signs it.
                        let d_ticks = prev.map(|p| ((tick.bid - p) / crate::pit_engine::tick_size(inst) as f64) as f32).unwrap_or(0.0);
                        candles.apply_live_tick(inst, tick.t, tick.bid as f32, d_ticks);
                        if feed.mode == FeedMode::Live {
                            let pit = engines.pit_mut(inst);
                            let ticks = pit.ticks_for_price(tick.bid as f32);
                            pit.set_live_anchor(ticks);
                        }
                    }
                    Some(TapeMessage::Depth(inst, depth)) => feed.ingest_depth(inst, &depth),
                    None => {}
                }
            }
            ewebsock::WsEvent::Message(_) => feed.note_message(now),
            ewebsock::WsEvent::Error(e) => {
                warn!("live feed: error: {e}");
                feed.drop_socket(now, e);
            }
            ewebsock::WsEvent::Closed => {
                warn!("live feed: socket closed");
                feed.drop_socket(now, "socket closed".to_string());
            }
        }
    }
    feed.check_staleness(now);
}

#[derive(Component)]
pub struct FeedStatusChip;

#[derive(Component)]
pub struct FeedStatusText;

/// Top-centre status chip: the one place that always says whether this is real.
pub fn setup_feed_status(mut commands: Commands) {
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                // Right end of the top bar: the centre now belongs to the market-view button.
                top: Val::Px(8.0),
                left: Val::Px(0.0),
                right: Val::Px(16.0),
                justify_content: JustifyContent::FlexEnd,
                ..default()
            },
            Pickable { should_block_lower: false, is_hoverable: false },
        ))
        .with_children(|row| {
            row.spawn((
                FeedStatusChip,
                Node {
                    padding: UiRect::axes(Val::Px(12.0), Val::Px(5.0)),
                    border: UiRect::all(Val::Px(1.0)),
                    border_radius: BorderRadius::all(Val::Px(4.0)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.10, 0.12, 0.16, 0.92)),
                BorderColor::all(Color::srgba(0.5, 0.5, 0.5, 0.7)),
                Pickable { should_block_lower: false, is_hoverable: false },
            ))
            .with_children(|chip| {
                chip.spawn((
                    FeedStatusText,
                    Text::new("CONNECTING TO FEED"),
                    TextFont { font_size: 13.0, ..default() },
                    TextColor(Color::srgb(0.85, 0.85, 0.85)),
                ));
            });
        });
}

fn mmss(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    format!("{:02}:{:02}", s / 60, s % 60)
}

/// Text and colours for the chip.
pub fn feed_status_line(feed: &LiveFeed, now: f64) -> (String, Color, Color) {
    match feed.mode {
        FeedMode::Simulated => (
            "SIMULATED FLOW - NOT MARKET DATA   [L: live]".to_string(),
            Color::srgba(0.55, 0.28, 0.02, 0.95),
            Color::srgb(1.0, 0.72, 0.25),
        ),
        FeedMode::Live => match feed.state {
            FeedState::Connecting => (
                format!("CONNECTING TO FEED  {}", feed.url),
                Color::srgba(0.16, 0.18, 0.22, 0.92),
                Color::srgb(0.8, 0.8, 0.8),
            ),
            FeedState::Live => {
                let (n, rate) = feed.live_summary(now);
                if n == 0 {
                    (
                        format!("FEED CONNECTED - NO TICKS {}  - ACTION HALTED", mmss(now - feed.state_since)),
                        Color::srgba(0.55, 0.40, 0.02, 0.95),
                        Color::srgb(1.0, 0.85, 0.35),
                    )
                } else {
                    (
                        format!("LIVE  tape  {n} pits ticking  {rate:.1} ticks/s"),
                        Color::srgba(0.03, 0.32, 0.14, 0.92),
                        Color::srgb(0.45, 1.0, 0.6),
                    )
                }
            }
            FeedState::Stale => (
                format!("FEED STALE {}  - ACTION HALTED", mmss(now - feed.state_since)),
                Color::srgba(0.55, 0.40, 0.02, 0.95),
                Color::srgb(1.0, 0.85, 0.35),
            ),
            FeedState::Down => (
                format!("FEED DOWN {}  - ACTION HALTED - reconnecting  ({})", mmss(now - feed.state_since), feed.last_error),
                Color::srgba(0.55, 0.06, 0.06, 0.95),
                Color::srgb(1.0, 0.45, 0.45),
            ),
        },
    }
}

pub fn update_feed_status_ui(
    time: Res<Time>,
    feed: Res<LiveFeed>,
    mut chip: Query<(&mut BackgroundColor, &mut BorderColor), With<FeedStatusChip>>,
    mut text: Query<(&mut Text, &mut TextColor), With<FeedStatusText>>,
) {
    let now = time.elapsed_secs_f64();
    let (line, bg, fg) = feed_status_line(&feed, now);
    for (mut t, mut c) in text.iter_mut() {
        crate::pit::set_ui_text(&mut t, line.clone());
        if c.0 != fg {
            *c = TextColor(fg);
        }
    }
    for (mut b, mut border) in chip.iter_mut() {
        if b.0 != bg {
            *b = BackgroundColor(bg);
            *border = BorderColor::all(fg.with_alpha(0.7));
        }
    }
}

/// Short label for nameplates and the board when a pit is not ticking.
pub fn halt_label(status: PitFeedStatus) -> Option<String> {
    match status {
        PitFeedStatus::Simulated | PitFeedStatus::Live { .. } => None,
        PitFeedStatus::Quiet { age } => Some(format!("NO TICKS {}", mmss(age as f64))),
        PitFeedStatus::NoSymbol => Some("NO FEED FOR THIS PIT".to_string()),
        PitFeedStatus::FeedDown => Some("FEED DOWN".to_string()),
        PitFeedStatus::Waiting => Some("WAITING FOR FIRST TICK".to_string()),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Short board text for the tape's read: how many of the recent ticks were upticks.
pub fn tape_label(flow: &TickFlow) -> String {
    if flow.ticks < 3 {
        return "TAPE THIN".to_string();
    }
    let pct = (flow.up_share * 100.0).round() as i32;
    let lean = if flow.direction > 0.45 {
        " BUYERS"
    } else if flow.direction < -0.45 {
        " SELLERS"
    } else {
        ""
    };
    format!("UPTICKS {pct}%{lean}")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{"type":"tick","data":{"symbolId":241,"symbol":"XAUUSD","t":1792601242826,"bid":4121.83,"ask":4122.03,"spread":0.2}}"#;
    const DEPTH: &str = r#"{"type":"depth","data":{"symbolId":241,"symbol":"XAUUSD","t":1792601242900,"bids":[[4121.83,1],[4121.73,1.5]],"asks":[[4122.03,1],[4122.13,1.5],[4122.25,2.5]]}}"#;

    #[test]
    fn parses_gold_tick_and_book() {
        let Some(TapeMessage::Tick(inst, tick)) = parse_tape(SAMPLE) else { panic!("tick") };
        assert_eq!(inst, Instrument::Gold);
        assert_eq!(tick.bid, 4121.83);
        assert_eq!(tick.ask, 4122.03);
        assert_eq!(tick.t, 1792601242826);
        let Some(TapeMessage::Depth(inst, depth)) = parse_tape(DEPTH) else { panic!("depth") };
        assert_eq!(inst, Instrument::Gold);
        assert_eq!(depth.bids.len(), 2);
        assert_eq!(depth.asks[2], (4122.25, 2.5));
    }

    #[test]
    fn ignores_other_frames() {
        assert!(parse_tape(r#"{"type":"hello","symbols":[]}"#).is_none());
        assert!(parse_tape(r#"{"type":"orderbook","data":{"symbol":"EURUSD"}}"#).is_none());
        let unknown = SAMPLE.replace("241", "9999");
        assert!(parse_tape(&unknown).is_none());
        assert!(parse_tape("not json").is_none());
    }

    #[test]
    fn book_rides_on_the_quote_and_spread_is_kept() {
        let mut feed = LiveFeed::default();
        feed.note_message(0.0);
        let Some(TapeMessage::Tick(_, tick)) = parse_tape(SAMPLE) else { panic!() };
        let Some(TapeMessage::Depth(_, depth)) = parse_tape(DEPTH) else { panic!() };
        feed.ingest_depth(Instrument::Gold, &depth);
        assert!(feed.quote(Instrument::Gold).is_none(), "a book before any tick has nowhere to live");
        feed.ingest_tick(Instrument::Gold, &tick, 1.0);
        feed.ingest_depth(Instrument::Gold, &depth);
        let q = feed.quote(Instrument::Gold).unwrap();
        assert!((q.spread - 0.2).abs() < 1e-9);
        assert_eq!(q.asks.len(), 3);
        assert_eq!(q.tick_ms, 1792601242826);
    }

    #[test]
    fn symbol_map_covers_every_pit() {
        assert_eq!(symbol_id_for(Instrument::Us30), Some(219));
        assert_eq!(symbol_id_for(Instrument::Uk100), Some(217));
        assert!(Instrument::ALL.iter().all(|i| symbol_id_for(*i).is_some()));
        assert_eq!(instrument_for_symbol_id(238), Some(Instrument::Silver));
    }

    #[test]
    fn activity_follows_tick_rate() {
        assert!(activity_for_rate(0.0) < activity_for_rate(0.5));
        assert!(activity_for_rate(0.5) < activity_for_rate(1.5));
        assert_eq!(activity_for_rate(10.0), 1.0);
    }

    fn bar_at(price: f64) -> TapeTick {
        let Some(TapeMessage::Tick(_, mut tick)) = parse_tape(SAMPLE) else { panic!() };
        tick.bid = price;
        tick.ask = price + 0.2;
        tick
    }

    #[test]
    fn tick_rule_reads_the_tape_direction() {
        // Gold, tick 0.10. Rising one tick every half second for ten seconds.
        let mut feed = LiveFeed::default();
        feed.note_message(0.0);
        for i in 0..=20 {
            feed.ingest_tick(Instrument::Gold, &bar_at(4120.0 + 0.1 * i as f64), 0.5 * i as f64);
        }
        let up = feed.quote(Instrument::Gold).unwrap().flow(10.0);
        assert!(up.direction > 0.7, "rising tape direction {:.2}", up.direction);
        assert!(up.up_share > 0.95, "up share {:.2}", up.up_share);
        assert_eq!(up.ticks, 20);

        // Then falling: the window forgets the rise.
        for i in 1..=20 {
            feed.ingest_tick(Instrument::Gold, &bar_at(4122.0 - 0.1 * i as f64), 10.0 + 0.5 * i as f64);
        }
        let down = feed.quote(Instrument::Gold).unwrap().flow(30.0);
        assert!(down.direction < -0.7, "falling tape direction {:.2}", down.direction);

        // Alternating ticks read as two-way.
        let mut feed = LiveFeed::default();
        feed.note_message(0.0);
        for i in 0..=20 {
            let px = if i % 2 == 0 { 4120.0 } else { 4120.1 };
            feed.ingest_tick(Instrument::Gold, &bar_at(px), 0.5 * i as f64);
        }
        let flat = feed.quote(Instrument::Gold).unwrap().flow(10.0);
        assert!(flat.direction.abs() < 0.15, "chop direction {:.2}", flat.direction);

        // One uptick alone is a lean, not a verdict.
        let mut feed = LiveFeed::default();
        feed.note_message(0.0);
        feed.ingest_tick(Instrument::Gold, &bar_at(4120.0), 0.0);
        feed.ingest_tick(Instrument::Gold, &bar_at(4120.1), 1.0);
        let one = feed.quote(Instrument::Gold).unwrap().flow(1.0);
        assert!(one.direction > 0.0 && one.direction < 0.4, "single tick direction {:.2}", one.direction);
    }

    #[test]
    fn sub_tick_screen_moves_still_have_a_direction() {
        // Gold pit tick 0.10; cTrader moves in 0.01. Three 0.03 downticks are selling.
        let mut feed = LiveFeed::default();
        feed.note_message(0.0);
        for (i, px) in [4120.00, 4119.97, 4119.94, 4119.91].iter().enumerate() {
            feed.ingest_tick(Instrument::Gold, &bar_at(*px), i as f64);
        }
        let f = feed.quote(Instrument::Gold).unwrap().flow(3.0);
        assert_eq!(f.ticks, 3);
        assert!(f.up_share < 0.01, "{f:?}");
        assert!(f.direction < -0.4, "{f:?}");
        assert!(f.burst_ticks < 1.0, "{f:?}");
    }

    #[test]
    fn zero_ticks_carry_the_last_direction_weakly_and_jumps_count_as_bursts() {
        let mut feed = LiveFeed::default();
        feed.note_message(0.0);
        feed.ingest_tick(Instrument::Gold, &bar_at(4120.0), 0.0);
        feed.ingest_tick(Instrument::Gold, &bar_at(4120.1), 1.0);
        for i in 2..6 {
            feed.ingest_tick(Instrument::Gold, &bar_at(4120.1), i as f64);
        }
        let f = feed.quote(Instrument::Gold).unwrap().flow(5.0);
        assert_eq!(f.ticks, 5);
        assert!(f.up_share > 0.99, "{f:?}");
        assert!(f.direction > 0.4, "{f:?}");
        assert!(f.burst_ticks <= 1.0, "{f:?}");
        feed.ingest_tick(Instrument::Gold, &bar_at(4120.6), 6.0);
        let f = feed.quote(Instrument::Gold).unwrap().flow(6.0);
        assert!((f.burst_ticks - 5.0).abs() < 0.01, "{f:?}");
        // Burst gone from the five-second window later, direction still remembered.
        let f = feed.quote(Instrument::Gold).unwrap().flow(12.0);
        assert!(f.burst_ticks < 1.0, "{f:?}");
        assert!(f.direction > 0.4, "{f:?}");
    }

    #[test]
    fn live_flow_mapping_is_bounded_and_monotone() {
        let up = TickFlow { direction: 1.0, up_share: 1.0, ticks: 30, burst_ticks: 1.0 };
        let down = TickFlow { direction: -1.0, ..up };
        let flat = TickFlow { direction: 0.0, up_share: 0.5, ..up };
        assert!((live_flow_for(&flat, 1.0).buy_prob - 0.5).abs() < 1e-6);
        assert!(live_flow_for(&up, 1.0).buy_prob >= 0.9);
        assert!(live_flow_for(&down, 1.0).buy_prob <= 0.1);
        assert!(live_flow_for(&up, 0.0).arrivals_per_sec < live_flow_for(&up, 1.0).arrivals_per_sec);
        assert!(live_flow_for(&up, 100.0).arrivals_per_sec <= 4.0);
        let sweep = TickFlow { burst_ticks: 6.0, ..up };
        assert!(live_flow_for(&sweep, 1.0).block_prob > live_flow_for(&up, 1.0).block_prob);
        assert!(live_flow_for(&TickFlow { burst_ticks: 100.0, ..up }, 1.0).block_prob <= 0.40);
    }

    #[test]
    fn tape_label_names_the_lean_only_when_it_is_one() {
        assert_eq!(tape_label(&TickFlow { ticks: 2, ..Default::default() }), "TAPE THIN");
        let up = TickFlow { direction: 0.8, up_share: 0.9, ticks: 20, burst_ticks: 1.0 };
        assert_eq!(tape_label(&up), "UPTICKS 90% BUYERS");
        let two_way = TickFlow { direction: 0.1, up_share: 0.55, ticks: 20, burst_ticks: 1.0 };
        assert_eq!(tape_label(&two_way), "UPTICKS 55%");
        let down = TickFlow { direction: -0.7, up_share: 0.15, ticks: 20, burst_ticks: 1.0 };
        assert_eq!(tape_label(&down), "UPTICKS 15% SELLERS");
    }

    #[test]
    fn pit_flow_is_only_offered_while_the_pit_is_live() {
        let mut feed = LiveFeed::default();
        feed.note_message(0.0);
        assert!(feed.pit_flow(Instrument::Gold, 1.0).is_none());
        feed.ingest_tick(Instrument::Gold, &bar_at(4120.0), 1.0);
        feed.ingest_tick(Instrument::Gold, &bar_at(4120.1), 2.0);
        assert!(feed.pit_flow(Instrument::Gold, 2.5).is_some_and(|f| f.buy_prob > 0.5));
        feed.check_staleness(2.0 + STALE_AFTER_SECS + 1.0);
        assert!(feed.pit_flow(Instrument::Gold, 2.0 + STALE_AFTER_SECS + 1.0).is_none());
    }

    #[test]
    fn halts_without_ticks_and_recovers() {
        let mut feed = LiveFeed::default();
        let bar = bar_at(4121.83);
        assert_eq!(feed.pit_status(Instrument::Gold, 0.0), PitFeedStatus::FeedDown);
        feed.note_message(1.0);
        assert_eq!(feed.state, FeedState::Live);
        assert_eq!(feed.pit_status(Instrument::Gold, 1.0), PitFeedStatus::Waiting);
        feed.ingest_tick(Instrument::Gold, &bar, 1.0);
        assert!(matches!(feed.pit_status(Instrument::Gold, 2.0), PitFeedStatus::Live { .. }));
        assert!(feed.pit_activity(Instrument::Gold, 2.0).is_some());
        assert_eq!(feed.pit_status(Instrument::Us30, 2.0), PitFeedStatus::Waiting);

        // Socket silent: stale, and every pit halts.
        feed.check_staleness(1.0 + STALE_AFTER_SECS + 1.0);
        assert_eq!(feed.state, FeedState::Stale);
        assert_eq!(feed.pit_status(Instrument::Gold, 25.0), PitFeedStatus::FeedDown);
        assert!(feed.pit_activity(Instrument::Gold, 25.0).is_none());

        // Socket speaks again but gold is quiet past the limit.
        feed.note_message(100.0);
        assert_eq!(feed.state, FeedState::Live);
        assert!(matches!(feed.pit_status(Instrument::Gold, 100.0), PitFeedStatus::Quiet { .. }));
        feed.ingest_tick(Instrument::Gold, &bar, 100.0);
        assert!(matches!(feed.pit_status(Instrument::Gold, 100.5), PitFeedStatus::Live { .. }));

        // The desk sets Down directly on a socket error; the first message after a reconnect
        // must bring the feed back without a Connecting step in between.
        feed.state = FeedState::Down;
        assert_eq!(feed.pit_status(Instrument::Gold, 101.0), PitFeedStatus::FeedDown);
        feed.note_message(101.0);
        assert_eq!(feed.state, FeedState::Live);
        feed.ingest_tick(Instrument::Gold, &bar, 101.0);
        assert!(matches!(feed.pit_status(Instrument::Gold, 101.5), PitFeedStatus::Live { .. }));
    }

    #[test]
    fn tick_rate_counts_recent_updates_only() {
        let mut feed = LiveFeed::default();
        let bar = bar_at(4121.83);
        feed.note_message(0.0);
        for i in 0..20 {
            feed.ingest_tick(Instrument::Gold, &bar, i as f64 * 0.5);
        }
        let q = feed.quote(Instrument::Gold).unwrap();
        assert!((q.rate(9.5) - 2.0).abs() < 0.2, "rate {}", q.rate(9.5));
        assert!(q.rate(40.0) < 0.01);
    }

    #[test]
    fn simulated_mode_is_never_mistaken_for_live() {
        let mut feed = LiveFeed::default();
        let mut engines = PitEngines::default();
        let mut controller = MarketScenarioController::default();
        feed.set_mode(FeedMode::Simulated, &mut engines, &mut controller, 0.0);
        assert_eq!(feed.pit_status(Instrument::Gold, 0.0), PitFeedStatus::Simulated);
        let (line, ..) = feed_status_line(&feed, 0.0);
        assert!(line.contains("SIMULATED"));
        assert!(!sim_only_guard(&LiveFeed::default(), &mut controller));
        assert!(sim_only_guard(&feed, &mut controller));
    }
}
