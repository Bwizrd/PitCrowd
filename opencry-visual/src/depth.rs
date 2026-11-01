//! Level 2: Bitcoin's live order book from Kraken's public websocket (no account, no key),
//! drawn two ways, as a foundation for more:
//!
//! - **the heatmap panel**: price up the side, time along the bottom, each pixel how much rests
//!   at that price at that second (dark = little, yellow-white = a wall), with the mid price
//!   as a line and every trade as a dot (green bought, red sold); the way Bookmap draws it;
//! - **liquidity walls in the BTC pit**: a column per $10 of price on the pit's back rim, bids
//!   to the left of the pulpit and asks to the right, nearest the pulpit nearest the price,
//!   height the size resting there. A big order added flashes its column white; one pulled
//!   before it traded flashes it amber, and the event is listed in the panel.
//!
//! The page reads Kraken's websocket v2 messages (channels `book` at depth 500 and `trade`) from
//! wherever `endpoints::l2_url` says: by default the tape's `/l2`, which floor_tape fills with a
//! synthetic book around its own BTCUSD (so the walls sit where the pit's price is); with
//! `?l2=kraken`, Kraken's real public book (`wss://ws.kraken.com/v2`, no account, no key),
//! which only lines up with the pit when the tape's BTCUSD is real too; `?l2=off` for none.
//!
//! Pure parts (`OrderBook`, the bucketing, the event rules, the colour ramp) are tested; the
//! systems only move data between the socket, the book and the textures.

use std::collections::{BTreeMap, VecDeque};
use std::sync::Mutex;

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

use crate::market::Instrument;
use crate::panels::{spawn_close_button, Panel, PanelId};
use crate::pit::set_ui_text;
use crate::pit_3d::{pit_3d_origin, BOWL_RIM_RADIUS};

pub const KRAKEN_WS: &str = crate::endpoints::KRAKEN_WS;

/// What to call a Level 2 source on screen.
pub fn source_name(url: Option<&str>) -> &'static str {
    match url {
        None => "off",
        Some(u) if u == KRAKEN_WS => "Kraken",
        Some(u) if u.trim_end_matches('/').ends_with("/l2") => "the tape",
        Some(_) => "L2 feed",
    }
}
pub const SYMBOL: &str = "BTC/USD";
pub const DEPTH: u32 = 500;
/// Kraken's BTC/USD price step.
pub const TICK: f64 = 0.1;

/// Heatmap: one column a second for five minutes, one row per $10, 120 rows (+-$600).
pub const HEAT_W: usize = 300;
pub const HEAT_H: usize = 120;
pub const HEAT_BUCKET: f64 = 10.0;
/// Walls: a column per $10, 30 each side.
pub const WALL_BUCKETS: usize = 30;
pub const WALL_BUCKET: f64 = 10.0;
pub const WALL_MAX_H: f32 = 24.0;
/// A change of this many BTC at one price is an event worth showing. (At 3 BTC a market maker
/// re-posting the same 3.23 BTC quote filled the list; 5 leaves the real walls.)
pub const BIG_BTC: f64 = 5.0;

// ---------------------------------------------------------------------------
// The book (pure)
// ---------------------------------------------------------------------------

/// One side's levels keyed by price in ticks.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OrderBook {
    pub bids: BTreeMap<i64, f64>,
    pub asks: BTreeMap<i64, f64>,
}

pub fn key(price: f64) -> i64 {
    (price / TICK).round() as i64
}

pub fn price_of(k: i64) -> f64 {
    k as f64 * TICK
}

/// What a level change means, for the events list and the walls.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BookEventKind {
    /// A big order joined the book.
    Added,
    /// A big order left the book without trading: cancelled.
    Pulled,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BookEvent {
    pub kind: BookEventKind,
    pub bid: bool,
    pub price: f64,
    pub qty: f64,
    pub at: f64,
    /// The same order (kind, side, size) seen again within a few seconds is folded into one
    /// event: a quote being re-posted, which is worth one line, not twenty.
    pub repeats: u32,
}

impl OrderBook {
    pub fn clear(&mut self) {
        self.bids.clear();
        self.asks.clear();
    }

    /// Set one level (qty 0 removes it). Returns the size change.
    pub fn set(&mut self, bid: bool, price: f64, qty: f64) -> f64 {
        let side = if bid { &mut self.bids } else { &mut self.asks };
        let k = key(price);
        let before = side.get(&k).copied().unwrap_or(0.0);
        if qty <= 0.0 {
            side.remove(&k);
        } else {
            side.insert(k, qty);
        }
        qty.max(0.0) - before
    }

    /// Kraken keeps the book to its subscribed depth: levels beyond it drop out of the feed
    /// without a removal message, so trim to the depth after every update.
    pub fn trim(&mut self, depth: usize) {
        while self.bids.len() > depth {
            let k = *self.bids.keys().next().unwrap();
            self.bids.remove(&k);
        }
        while self.asks.len() > depth {
            let k = *self.asks.keys().next_back().unwrap();
            self.asks.remove(&k);
        }
    }

    pub fn best_bid(&self) -> Option<(f64, f64)> {
        self.bids.iter().next_back().map(|(k, q)| (price_of(*k), *q))
    }

    pub fn best_ask(&self) -> Option<(f64, f64)> {
        self.asks.iter().next().map(|(k, q)| (price_of(*k), *q))
    }

    pub fn mid(&self) -> Option<f64> {
        Some((self.best_bid()?.0 + self.best_ask()?.0) * 0.5)
    }

    /// Size resting in `n` buckets of `width` dollars each side of `mid`, nearest first:
    /// (bids below, asks above).
    pub fn buckets(&self, mid: f64, width: f64, n: usize) -> (Vec<f64>, Vec<f64>) {
        let mut bids = vec![0.0; n];
        let mut asks = vec![0.0; n];
        for (k, q) in &self.bids {
            let d = ((mid - price_of(*k)) / width).floor();
            if d >= 0.0 && (d as usize) < n {
                bids[d as usize] += q;
            }
        }
        for (k, q) in &self.asks {
            let d = ((price_of(*k) - mid) / width).floor();
            if d >= 0.0 && (d as usize) < n {
                asks[d as usize] += q;
            }
        }
        (bids, asks)
    }

    /// Total size per absolute price bucket (bucket index = floor(price / width)), both sides.
    pub fn by_bucket(&self, width: f64) -> BTreeMap<i64, f64> {
        let mut out = BTreeMap::new();
        for (k, q) in self.bids.iter().chain(self.asks.iter()) {
            *out.entry((price_of(*k) / width).floor() as i64).or_insert(0.0) += q;
        }
        out
    }
}

/// Is a level change an event? A big add is one; a big removal is a pull unless it traded:
/// `traded` is the size printed at that price in the moment before.
pub fn classify(change: f64, traded: f64) -> Option<BookEventKind> {
    if change >= BIG_BTC {
        Some(BookEventKind::Added)
    } else if change <= -BIG_BTC && traded < -change * 0.5 {
        Some(BookEventKind::Pulled)
    } else {
        None
    }
}

/// The heatmap's colour for a resting size, as a share `x` (0..1) of the scale: black, deep
/// blue, cyan, yellow, white.
pub fn heat_rgb(x: f32) -> [u8; 3] {
    let stops: [(f32, [f32; 3]); 5] = [(0.0, [8.0, 10.0, 18.0]), (0.25, [20.0, 40.0, 140.0]), (0.5, [20.0, 170.0, 200.0]), (0.8, [250.0, 220.0, 40.0]), (1.0, [255.0, 255.0, 255.0])];
    let x = x.clamp(0.0, 1.0);
    for w in stops.windows(2) {
        let ((a, ca), (b, cb)) = (w[0], w[1]);
        if x <= b {
            let t = (x - a) / (b - a);
            return [0, 1, 2].map(|i| (ca[i] + (cb[i] - ca[i]) * t) as u8);
        }
    }
    [255, 255, 255]
}

// ---------------------------------------------------------------------------
// The feed
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Print {
    pub at: f64,
    pub price: f64,
    pub qty: f64,
    pub buy: bool,
}

/// One second of the book for the heatmap: size per absolute $10 bucket, the mid, and the
/// trades of that second.
#[derive(Debug, Clone, Default)]
pub struct HeatColumn {
    pub cells: BTreeMap<i64, f32>,
    pub mid: f64,
    pub prints: Vec<Print>,
}

/// The socket behind a mutex so the resource is `Sync`. On wasm the handles wrap JS objects
/// that are not `Send`; wasm32 without threads is single-threaded, so that is sound (as
/// `live_feed`'s tape socket).
struct Socket(Mutex<Option<(ewebsock::WsSender, ewebsock::WsReceiver)>>);

#[cfg(target_arch = "wasm32")]
unsafe impl Send for Socket {}
#[cfg(target_arch = "wasm32")]
unsafe impl Sync for Socket {}

#[derive(Resource)]
pub struct DepthFeed {
    socket: Socket,
    pub book: OrderBook,
    pub live: bool,
    pub status: String,
    pub prints: VecDeque<Print>,
    pub heat: VecDeque<HeatColumn>,
    pub events: VecDeque<BookEvent>,
    pub updates: u64,
    rate_mark: (f64, u64),
    pub updates_per_sec: f64,
    next_connect_at: f64,
    backoff: f64,
    next_column_at: f64,
    pending_prints: Vec<Print>,
    /// The socket to read, `None` when Level 2 is switched off.
    pub url: Option<String>,
    pub source: &'static str,
}

impl Default for DepthFeed {
    fn default() -> Self {
        let url = crate::endpoints::l2_url();
        let source = source_name(url.as_deref());
        DepthFeed {
            url,
            source,
            socket: Socket(Mutex::new(None)),
            book: OrderBook::default(),
            live: false,
            status: format!("connecting to {source}..."),
            prints: VecDeque::new(),
            heat: VecDeque::new(),
            events: VecDeque::new(),
            updates: 0,
            rate_mark: (0.0, 0),
            updates_per_sec: 0.0,
            next_connect_at: 0.0,
            backoff: 5.0,
            next_column_at: 0.0,
            pending_prints: Vec::new(),
        }
    }
}

impl DepthFeed {
    /// Size printed at this price in the last `secs` seconds.
    fn traded_at(&self, price: f64, now: f64, secs: f64) -> f64 {
        let k = key(price);
        self.prints.iter().rev().take_while(|p| now - p.at <= secs).filter(|p| key(p.price) == k).map(|p| p.qty).sum()
    }

    /// One message from Kraken. Public so tests can feed recorded messages.
    pub fn handle(&mut self, text: &str, now: f64) {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(text) else { return };
        let channel = v.get("channel").and_then(|c| c.as_str()).unwrap_or("");
        let kind = v.get("type").and_then(|c| c.as_str()).unwrap_or("");
        let Some(data) = v.get("data").and_then(|d| d.as_array()) else { return };
        match channel {
            "book" => {
                if kind == "snapshot" {
                    self.book.clear();
                    self.live = true;
                    self.status = "live".into();
                }
                for d in data {
                    for (field, bid) in [("bids", true), ("asks", false)] {
                        for lvl in d.get(field).and_then(|x| x.as_array()).into_iter().flatten() {
                            let (Some(p), Some(q)) = (lvl.get("price").and_then(|x| x.as_f64()), lvl.get("qty").and_then(|x| x.as_f64())) else { continue };
                            let change = self.book.set(bid, p, q);
                            if kind == "update" {
                                if let Some(k) = classify(change, self.traded_at(p, now, 2.0)) {
                                    let q = change.abs();
                                    match self.events.back_mut().filter(|e| e.kind == k && e.bid == bid && (e.qty - q).abs() <= 0.01 * q && now - e.at < 5.0) {
                                        Some(e) => {
                                            e.repeats += 1;
                                            e.price = p;
                                            e.at = now;
                                        }
                                        None => self.events.push_back(BookEvent { kind: k, bid, price: p, qty: q, at: now, repeats: 0 }),
                                    }
                                    if self.events.len() > 40 {
                                        self.events.pop_front();
                                    }
                                }
                            }
                        }
                    }
                }
                self.book.trim(DEPTH as usize);
                self.updates += 1;
            }
            "trade" => {
                for d in data {
                    let (Some(p), Some(q)) = (d.get("price").and_then(|x| x.as_f64()), d.get("qty").and_then(|x| x.as_f64())) else { continue };
                    let pr = Print { at: now, price: p, qty: q, buy: d.get("side").and_then(|s| s.as_str()) == Some("buy") };
                    self.prints.push_back(pr);
                    self.pending_prints.push(pr);
                }
                while self.prints.len() > 2000 {
                    self.prints.pop_front();
                }
            }
            _ => {}
        }
    }

    /// Once a second: a heatmap column from the book as it stands.
    pub fn sample(&mut self, now: f64) {
        if now < self.next_column_at {
            return;
        }
        self.next_column_at = now + 1.0;
        if now - self.rate_mark.0 >= 5.0 {
            self.updates_per_sec = (self.updates - self.rate_mark.1) as f64 / (now - self.rate_mark.0).max(1e-6);
            self.rate_mark = (now, self.updates);
        }
        let Some(mid) = self.book.mid() else { return };
        let cells = self.book.by_bucket(HEAT_BUCKET).into_iter().map(|(k, q)| (k, q as f32)).collect();
        self.heat.push_back(HeatColumn { cells, mid, prints: std::mem::take(&mut self.pending_prints) });
        while self.heat.len() > HEAT_W {
            self.heat.pop_front();
        }
    }
}

/// System: keep the Kraken socket up, subscribe on open, fold every message in, sample the
/// heatmap. Reconnects with a growing wait (5 s to a minute); a new snapshot resets the book.
pub fn poll_depth_feed(time: Res<Time>, mut feed: ResMut<DepthFeed>) {
    let now = time.elapsed_secs_f64();
    let connected = feed.socket.0.lock().map(|g| g.is_some()).unwrap_or(false);
    let Some(url) = feed.url.clone() else {
        feed.status = "switched off (?l2=off); ?l2=tape or ?l2=kraken turns it on".into();
        return;
    };
    let source = feed.source;
    if !connected && now >= feed.next_connect_at {
        match ewebsock::connect(url, ewebsock::Options::default()) {
            Ok(pair) => {
                if let Ok(mut g) = feed.socket.0.lock() {
                    *g = Some(pair);
                }
                feed.status = format!("connecting to {source}...");
            }
            Err(e) => {
                feed.status = format!("{source}: {e}");
                feed.next_connect_at = now + feed.backoff;
                feed.backoff = (feed.backoff * 2.0).min(60.0);
            }
        }
    }
    let mut texts = Vec::new();
    let mut dropped = None;
    if let Ok(mut g) = feed.socket.0.lock() {
        if let Some((tx, rx)) = g.as_mut() {
            while let Some(ev) = rx.try_recv() {
                match ev {
                    ewebsock::WsEvent::Opened => {
                        for channel in ["book", "trade"] {
                            let mut params = serde_json::json!({"channel": channel, "symbol": [SYMBOL]});
                            if channel == "book" {
                                params["depth"] = DEPTH.into();
                            }
                            tx.send(ewebsock::WsMessage::Text(serde_json::json!({"method": "subscribe", "params": params}).to_string()));
                        }
                    }
                    ewebsock::WsEvent::Message(ewebsock::WsMessage::Text(t)) => texts.push(t),
                    ewebsock::WsEvent::Message(_) => {}
                    ewebsock::WsEvent::Error(e) => dropped = Some(format!("{source}: {e}")),
                    ewebsock::WsEvent::Closed => dropped = Some(format!("{source} closed the connection")),
                }
            }
        }
        if dropped.is_some() {
            *g = None;
        }
    }
    for t in &texts {
        feed.handle(t, now);
    }
    if !texts.is_empty() {
        feed.backoff = 5.0;
    }
    if let Some(why) = dropped {
        feed.live = false;
        feed.status = format!("{why}; reconnecting");
        feed.next_connect_at = now + feed.backoff;
        feed.backoff = (feed.backoff * 2.0).min(60.0);
    }
    feed.sample(now);
}

// ---------------------------------------------------------------------------
// The heatmap panel
// ---------------------------------------------------------------------------

#[derive(Component)]
pub struct DepthPanel;
#[derive(Component)]
pub struct DepthHeader;
#[derive(Component)]
pub struct DepthEvents;
#[derive(Resource)]
pub struct DepthTexture(pub Handle<Image>);

pub fn setup_depth_panel(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    let image = Image::new_fill(
        Extent3d { width: HEAT_W as u32, height: HEAT_H as u32, depth_or_array_layers: 1 },
        TextureDimension::D2,
        &[8, 10, 18, 255],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    let handle = images.add(image);
    commands.insert_resource(DepthTexture(handle.clone()));
    commands
        .spawn((
            DepthPanel,
            Panel(PanelId::Depth),
            Node {
                width: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                padding: UiRect::axes(Val::Px(10.0), Val::Px(6.0)),
                row_gap: Val::Px(4.0),
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(Val::Px(6.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.02, 0.03, 0.06, 0.96)),
            BorderColor::all(Color::srgba(0.25, 0.75, 0.95, 0.55)),
        ))
        .with_children(|panel| {
            let me = panel.target_entity();
            panel
                .spawn(Node { flex_direction: FlexDirection::Row, justify_content: JustifyContent::SpaceBetween, align_items: AlignItems::FlexStart, width: Val::Percent(100.0), ..default() })
                .with_children(|row| {
                    row.spawn((DepthHeader, Text::new("LEVEL 2  |  BTC/USD order book"), TextFont { font_size: 10.5, ..default() }, TextColor(Color::srgb(0.55, 0.85, 1.0)), Node { flex_grow: 1.0, flex_shrink: 1.0, min_width: Val::Px(0.0), ..default() }));
                    spawn_close_button(row, me);
                });
            // Heatmap: price up, time along (newest at the right).
            panel.spawn((ImageNode::new(handle), Node { width: Val::Percent(100.0), height: Val::Px(200.0), ..default() }));
            panel.spawn((DepthEvents, Text::new(""), TextFont { font_size: 9.5, ..default() }, TextColor(Color::srgb(0.75, 0.80, 0.88))));
        });
}

/// Redraw the heatmap into an RGBA buffer, newest column at the right, centred on the latest
/// mid. Pure apart from the data, for tests.
pub fn draw_heatmap(heat: &VecDeque<HeatColumn>, buf: &mut [u8]) {
    for px in buf.chunks_mut(4) {
        px.copy_from_slice(&[8, 10, 18, 255]);
    }
    let Some(last) = heat.back() else { return };
    let centre = (last.mid / HEAT_BUCKET).floor() as i64;
    let top = centre + HEAT_H as i64 / 2;
    // A robust scale: the 98th percentile of the visible cells.
    let mut vals: Vec<f32> = heat.iter().flat_map(|c| c.cells.range(top - HEAT_H as i64 + 1..=top).map(|(_, q)| *q)).collect();
    vals.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let scale = vals.get(((vals.len() as f32) * 0.98) as usize).copied().unwrap_or(1.0).max(1e-6);
    let x0 = HEAT_W - heat.len();
    let mut put = |x: usize, y: usize, rgb: [u8; 3]| {
        if x < HEAT_W && y < HEAT_H {
            let i = (y * HEAT_W + x) * 4;
            buf[i..i + 3].copy_from_slice(&rgb);
        }
    };
    for (j, col) in heat.iter().enumerate() {
        let x = x0 + j;
        for (b, q) in col.cells.range(top - HEAT_H as i64 + 1..=top) {
            let y = (top - b) as usize;
            put(x, y, heat_rgb((q / scale).sqrt()));
        }
        for p in &col.prints {
            let y = top - (p.price / HEAT_BUCKET).floor() as i64;
            if y >= 0 {
                let rgb = if p.buy { [40, 255, 110] } else { [255, 70, 70] };
                put(x, y as usize, rgb);
                if p.qty >= 0.5 {
                    put(x, y as usize + 1, rgb);
                    if y > 0 {
                        put(x, y as usize - 1, rgb);
                    }
                }
            }
        }
        // The mid on top of everything.
        let my = top - (col.mid / HEAT_BUCKET).floor() as i64;
        if my >= 0 {
            put(x, my as usize, [255, 255, 255]);
        }
    }
}

/// System: the heatmap texture (once a second) and the panel's lines.
pub fn update_depth_panel(
    time: Res<Time>,
    feed: Res<DepthFeed>,
    tex: Option<Res<DepthTexture>>,
    mut images: ResMut<Assets<Image>>,
    mut next: Local<f64>,
    mut header: Query<&mut Text, (With<DepthHeader>, Without<DepthEvents>)>,
    mut events: Query<&mut Text, (With<DepthEvents>, Without<DepthHeader>)>,
) {
    let now = time.elapsed_secs_f64();
    if now < *next {
        return;
    }
    *next = now + 1.0;
    if let Some(tex) = tex {
        if let Some(img) = images.get_mut(&tex.0) {
            if let Some(data) = img.data.as_mut() {
                draw_heatmap(&feed.heat, data);
            }
        }
    }
    let b = &feed.book;
    let line = match (b.best_bid(), b.best_ask()) {
        (Some((bp, bq)), Some((ap, aq))) => format!(
            "LEVEL 2  |  BTC/USD order book, {} ({})  |  bid {bp:.1} x {bq:.3}  ask {ap:.1} x {aq:.3}  spread {:.1}  |  {} levels, {:.0} updates/s  |  rows $10, 5 min",
            feed.source,
            feed.status,
            ap - bp,
            b.bids.len() + b.asks.len(),
            feed.updates_per_sec
        ),
        _ => format!("LEVEL 2  |  BTC/USD order book, {}  |  {}", feed.source, feed.status),
    };
    for mut t in header.iter_mut() {
        set_ui_text(&mut t, line.clone());
    }
    let mut lines: Vec<String> = feed
        .events
        .iter()
        .rev()
        .take(5)
        .map(|e| {
            format!(
                "{:>4.0}s ago  {} {:.2} BTC {} at {:.1}{}",
                now - e.at,
                if e.kind == BookEventKind::Added { "ADDED " } else { "PULLED" },
                e.qty,
                if e.bid { "bid" } else { "offer" },
                e.price,
                if e.repeats > 0 { format!("  (x{}, re-posted)", e.repeats + 1) } else { String::new() }
            )
        })
        .collect();
    if lines.is_empty() {
        lines.push(format!("Big orders ({BIG_BTC} BTC or more) added or pulled will show here, and flash in the BTC pit's walls."));
    }
    for mut t in events.iter_mut() {
        set_ui_text(&mut t, lines.join("\n"));
    }
}

// ---------------------------------------------------------------------------
// Liquidity walls in the BTC pit
// ---------------------------------------------------------------------------

#[derive(Component)]
pub struct DepthBar {
    pub bid: bool,
    pub k: usize,
}

#[derive(Resource)]
pub struct DepthBarMaterials {
    bid: Handle<StandardMaterial>,
    ask: Handle<StandardMaterial>,
    added: Handle<StandardMaterial>,
    pulled: Handle<StandardMaterial>,
}

/// Where bar `k` of a side stands, on the floor plane relative to the pit centre: on an arc
/// just outside the rim, from beside the pulpit (north) round toward the sides.
pub fn bar_xz(bid: bool, k: usize) -> Vec2 {
    let r = BOWL_RIM_RADIUS + 2.2;
    let step = 2.7f32.to_radians();
    let a0 = -std::f32::consts::FRAC_PI_2; // the pulpit is due north (-z)
    let a = if bid { a0 - 0.22 - step * k as f32 } else { a0 + 0.22 + step * k as f32 };
    Vec2::new(a.cos() * r, a.sin() * r)
}

pub fn setup_depth_walls(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>, mut materials: ResMut<Assets<StandardMaterial>>) {
    let mesh = meshes.add(Cuboid::new(1.6, 1.0, 1.6));
    let mat = |c: Color, glow: f32, materials: &mut Assets<StandardMaterial>| materials.add(StandardMaterial { base_color: c, emissive: LinearRgba::from(c) * glow, perceptual_roughness: 0.4, ..default() });
    let m = DepthBarMaterials {
        bid: mat(Color::srgb(0.15, 0.85, 0.40), 0.8, &mut materials),
        ask: mat(Color::srgb(0.95, 0.25, 0.25), 0.8, &mut materials),
        added: mat(Color::srgb(1.0, 1.0, 1.0), 2.5, &mut materials),
        pulled: mat(Color::srgb(1.0, 0.65, 0.10), 2.5, &mut materials),
    };
    let o = pit_3d_origin(Instrument::Bitcoin);
    for bid in [true, false] {
        for k in 0..WALL_BUCKETS {
            let p = bar_xz(bid, k);
            commands.spawn((
                DepthBar { bid, k },
                Mesh3d(mesh.clone()),
                MeshMaterial3d(if bid { m.bid.clone() } else { m.ask.clone() }),
                Transform::from_translation(o + Vec3::new(p.x, 0.05, p.y)).with_scale(Vec3::new(1.0, 0.1, 1.0)),
            ));
        }
    }
    commands.insert_resource(m);
}

/// Bar heights for one side: the square root of the size (so a wall does not flatten the
/// rest), scaled so the biggest bucket on either side is `WALL_MAX_H`. Pure.
pub fn bar_heights(bids: &[f64], asks: &[f64]) -> (Vec<f32>, Vec<f32>) {
    let max = bids.iter().chain(asks).fold(0.0f64, |a, b| a.max(*b)).sqrt().max(1e-9);
    let h = |q: &f64| ((q.sqrt() / max) as f32 * WALL_MAX_H).max(0.1);
    (bids.iter().map(h).collect(), asks.iter().map(h).collect())
}

/// System: the walls follow the book (eased), and flash for recent events.
pub fn update_depth_walls(
    time: Res<Time>,
    feed: Res<DepthFeed>,
    mats: Option<Res<DepthBarMaterials>>,
    mut bars: Query<(&DepthBar, &mut Transform, &mut MeshMaterial3d<StandardMaterial>)>,
) {
    let Some(mats) = mats else { return };
    let Some(mid) = feed.book.mid() else { return };
    let now = time.elapsed_secs_f64();
    let (b, a) = feed.book.buckets(mid, WALL_BUCKET, WALL_BUCKETS);
    let (hb, ha) = bar_heights(&b, &a);
    let k_ease = (time.delta_secs() * 6.0).min(1.0);
    let base_y = pit_3d_origin(Instrument::Bitcoin).y + 0.05;
    for (bar, mut tf, mut mat) in bars.iter_mut() {
        let want = if bar.bid { hb[bar.k] } else { ha[bar.k] };
        let h = tf.scale.y + (want - tf.scale.y) * k_ease;
        tf.scale.y = h;
        tf.translation.y = base_y + h * 0.5;
        // A recent event in this bar's bucket flashes it.
        let flash = feed.events.iter().rev().take_while(|e| now - e.at < 0.8).find(|e| {
            e.bid == bar.bid && {
                let d = if e.bid { mid - e.price } else { e.price - mid };
                d >= 0.0 && (d / WALL_BUCKET).floor() as usize == bar.k
            }
        });
        let want_mat = match flash.map(|e| e.kind) {
            Some(BookEventKind::Added) => &mats.added,
            Some(BookEventKind::Pulled) => &mats.pulled,
            None if bar.bid => &mats.bid,
            None => &mats.ask,
        };
        if mat.0 != *want_mat {
            mat.0 = want_mat.clone();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SNAP: &str = r#"{"channel":"book","type":"snapshot","data":[{"symbol":"BTC/USD","bids":[{"price":77304.5,"qty":0.2},{"price":77290.0,"qty":6.0}],"asks":[{"price":77304.6,"qty":1.1},{"price":77330.0,"qty":2.0}],"checksum":1}]}"#;

    #[test]
    fn a_snapshot_and_updates_build_the_book() {
        let mut f = DepthFeed::default();
        f.handle(SNAP, 0.0);
        assert!(f.live);
        assert_eq!(f.book.best_bid(), Some((77304.5, 0.2)));
        assert_eq!(f.book.best_ask(), Some((77304.6, 1.1)));
        assert!((f.book.mid().unwrap() - 77304.55).abs() < 1e-9);
        f.handle(r#"{"channel":"book","type":"update","data":[{"symbol":"BTC/USD","bids":[{"price":77304.5,"qty":0}],"asks":[]}]}"#, 1.0);
        assert_eq!(f.book.best_bid(), Some((77290.0, 6.0)), "qty 0 removes the level");
    }

    #[test]
    fn big_adds_and_pulls_are_events_but_a_level_that_traded_away_is_not() {
        let mut f = DepthFeed::default();
        f.handle(SNAP, 0.0);
        f.handle(r#"{"channel":"book","type":"update","data":[{"bids":[{"price":77280.0,"qty":5.5}],"asks":[]}]}"#, 1.0);
        assert_eq!(f.events.back().map(|e| (e.kind, e.bid, e.qty)), Some((BookEventKind::Added, true, 5.5)));
        f.handle(r#"{"channel":"book","type":"update","data":[{"bids":[{"price":77290.0,"qty":0}],"asks":[]}]}"#, 2.0);
        assert_eq!(f.events.back().map(|e| e.kind), Some(BookEventKind::Pulled), "6 BTC gone with no trade: pulled");
        // The same 5.5 BTC bid re-posted a level lower, twice: one event, counted.
        f.handle(r#"{"channel":"book","type":"update","data":[{"bids":[{"price":77270.0,"qty":5.5}],"asks":[]}]}"#, 2.5);
        f.handle(r#"{"channel":"book","type":"update","data":[{"bids":[{"price":77260.0,"qty":5.5}],"asks":[]}]}"#, 3.0);
        let n_before = f.events.len();
        f.handle(r#"{"channel":"book","type":"update","data":[{"bids":[{"price":77250.0,"qty":5.5}],"asks":[]}]}"#, 3.2);
        assert_eq!((f.events.len(), f.events.back().unwrap().repeats, f.events.back().unwrap().price), (n_before, 2, 77250.0), "folded into one");
        // The 2 BTC offer at 77330 trades away: not a pull.
        f.handle(r#"{"channel":"trade","type":"update","data":[{"symbol":"BTC/USD","side":"buy","price":77330.0,"qty":2.0}]}"#, 3.0);
        let n = f.events.len();
        f.handle(r#"{"channel":"book","type":"update","data":[{"bids":[],"asks":[{"price":77330.0,"qty":0}]}]}"#, 3.1);
        assert_eq!(f.events.len(), n, "traded, not pulled (and under the size anyway)");
        assert_eq!(classify(-6.0, 6.0), None);
        assert_eq!(classify(-6.0, 0.0), Some(BookEventKind::Pulled));
        assert_eq!(classify(-4.0, 0.0), None, "under the size");
    }

    #[test]
    fn buckets_sum_the_size_by_distance_from_the_mid() {
        let mut f = DepthFeed::default();
        f.handle(SNAP, 0.0);
        let mid = f.book.mid().unwrap();
        let (b, a) = f.book.buckets(mid, 10.0, 5);
        assert_eq!((b[0], b[1]), (0.2, 6.0), "77304.5 in the first $10 below, 77290 in the second");
        assert_eq!((a[0], a[2]), (1.1, 2.0));
        let (hb, ha) = bar_heights(&b, &a);
        assert!((hb[1] - WALL_MAX_H).abs() < 1e-4 && ha[0] < hb[1] && hb[3] == 0.1);
    }

    #[test]
    fn the_heatmap_draws_the_mid_and_the_trades_on_a_dark_ground() {
        let mut f = DepthFeed::default();
        f.handle(SNAP, 0.0);
        f.handle(r#"{"channel":"trade","type":"update","data":[{"side":"sell","price":77290.0,"qty":1.0}]}"#, 0.5);
        f.sample(1.0);
        let mut buf = vec![0u8; HEAT_W * HEAT_H * 4];
        draw_heatmap(&f.heat, &mut buf);
        let at = |x: usize, y: usize| &buf[(y * HEAT_W + x) * 4..(y * HEAT_W + x) * 4 + 3];
        let top = (77304.55f64 / HEAT_BUCKET).floor() as i64 + HEAT_H as i64 / 2;
        let mid_y = (top - (77304.55f64 / HEAT_BUCKET).floor() as i64) as usize;
        assert_eq!(at(HEAT_W - 1, mid_y), &[255, 255, 255], "the mid line, newest column at the right");
        let sell_y = (top - (77290.0f64 / HEAT_BUCKET).floor() as i64) as usize;
        assert_eq!(at(HEAT_W - 1, sell_y), &[255, 70, 70], "a sell print in red");
        assert_eq!(at(HEAT_W - 1, sell_y + 1), &[255, 70, 70], "a big print is three pixels tall");
        assert_eq!(at(0, 0), &[8, 10, 18], "empty history is dark");
        assert_eq!(heat_rgb(0.0), [8, 10, 18]);
        assert_eq!(heat_rgb(1.0), [255, 255, 255]);
    }

    #[test]
    fn the_depth_systems_build_without_conflicting_queries() {
        use bevy::ecs::system::RunSystemOnce;
        let mut world = World::new();
        world.insert_resource(Time::<()>::default());
        world.insert_resource(DepthFeed::default());
        world.init_resource::<Assets<Image>>();
        world.run_system_once(update_depth_panel).unwrap();
        world.run_system_once(update_depth_walls).unwrap();
    }

    #[test]
    fn the_walls_start_beside_the_pulpit_and_spread_round_the_rim() {
        let near_bid = bar_xz(true, 0);
        let near_ask = bar_xz(false, 0);
        assert!(near_bid.x < 0.0 && near_ask.x > 0.0, "bids left of the pulpit, asks right");
        assert!(near_bid.y < 0.0 && near_ask.y < 0.0, "both on the pulpit's (north) side");
        assert!((bar_xz(true, 29).length() - near_bid.length()).abs() < 1e-3, "all on one arc");
        assert!(bar_xz(true, 29).x < near_bid.x, "further out round the rim");
    }
}
