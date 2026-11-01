//! The paper traders on the floor: avatars (one per `Archetype::ALL`) you can pick out of the crowd and click.
//!
//! The crowd in the pits is simulation. The paper traders (`paper::Archetype::ALL`, 29 of them) are
//! the experiment, so they get bodies of their own: built from the same voxel vocabulary as
//! the proxy figures (legs, body, head, two hands) but a head taller, in a bright solid jacket
//! of the trader's panel colour, with a white badge on the chest and a coloured halo at the
//! feet. Colin, the coin flip, wears grey and carries a coin. Over each head a UI label shows
//! `NAME  +1.3R` for the day (since 00:00 UTC; green up, red down, grey flat) and, under it,
//! the open position when there is one.
//!
//! Where they go. A trader with an open position stands at that pit's rail facing the pulpit
//! with hands up on the side of the trade; one with no position visits the pit of its last
//! trade or decision; one with nothing yet wanders the concourse between pits. Walking is a
//! grid A* over the concourse (cells inside a pit's rim-plus-margin are blocked), pulled
//! straight where the line is clear, at a walking pace, with a short shuffle on arrival.
//! Everything is deterministic from the trader's seed; there is no physics.
//!
//! Where the numbers come from. While the headless desk is alive (`desk_feed`) its board
//! carries the day block, the last ten trades and the open position per trader; otherwise
//! the same is computed from the in-tab `PaperBook`. The pure layer (`TraderSnapshot`,
//! `pick_goal`, `day_stats_from_trades`, `label_text`, `route`) is what the tests cover.
//!
//! Clicking an avatar (the 3D picker in `camera::camera_orbit_3d` asks `AvatarPicking` first)
//! opens the detail panel bottom-left: creed, rules, today's stats, the open position with
//! its mark, the last ten trades and the source. The X, Escape, or clicking another avatar
//! closes or swaps it. For scripted screenshots the browser build publishes each avatar's
//! screen position on `window.__opencryAvatars` as `[{name, x, y, open}]`.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use bevy::prelude::*;

use crate::camera::{FloorViewMode, OrbitCamera, PitCamera3d};
use crate::candles::wall_clock_ms;
use crate::desk_feed::{DeskBoard, DeskFeed, DeskPass, DeskTrade, DeskTrader, LAST_TRADES_KEEP};
use crate::jev::JevDesk;
use crate::levels::tape_symbol;
use crate::live_feed::LiveFeed;
use crate::market::Instrument;
use crate::panels::{spawn_close_button, PanelClose};
use crate::paper::{day_start_ms, Archetype, PaperBook, Session, Trade};
use crate::pit::{format_price_with_commas, instrument_decimals, set_ui_text};
use crate::pit_3d::{pit_3d_origin, PIT_3D_ORIGIN, PULPIT_OFFSET_Z};
use crate::pit_engine::XorShift;

// ---------------------------------------------------------------------------
// What a trader looks like right now, from either source
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Desk,
    InTab,
}

impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Source::Desk => "DESK (headless, 24/7)",
            Source::InTab => "IN-TAB BOOK (this browser tab)",
        }
    }
}

/// The day so far: the same columns as the leaderboard, counted since 00:00 UTC.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DayStats {
    pub trades: u32,
    pub wins: u32,
    pub losses: u32,
    pub net_r: f32,
    pub spread_paid_r: f32,
    pub max_drawdown_r: f32,
}

impl DayStats {
    pub fn win_rate(&self) -> f32 {
        let n = self.wins + self.losses;
        if n == 0 { 0.0 } else { self.wins as f32 / n as f32 }
    }
}

/// Day totals from closed trades in any order, counting those closed at or after
/// `day_start`. Drawdown is measured over the day's own sequence from zero, like the desk's
/// windowed board, so "max DD" means the same thing on the label and in the panel.
pub fn day_stats_from_trades(trades: &[Trade], day_start: i64) -> DayStats {
    let mut today: Vec<&Trade> = trades.iter().filter(|t| t.closed_ms >= day_start).collect();
    today.sort_by_key(|t| t.closed_ms);
    let mut d = DayStats::default();
    let mut peak = 0.0f32;
    for t in today {
        d.trades += 1;
        if t.pnl_r > 0.0 { d.wins += 1 } else { d.losses += 1 }
        d.net_r += t.pnl_r;
        d.spread_paid_r += t.spread_ticks_in / t.stop_ticks.max(1.0);
        peak = peak.max(d.net_r);
        d.max_drawdown_r = d.max_drawdown_r.max(peak - d.net_r);
    }
    d
}

#[derive(Debug, Clone, PartialEq)]
pub struct OpenPosition {
    /// None when the symbol on the wire is not one of the nine pits.
    pub inst: Option<Instrument>,
    pub symbol: String,
    /// "long" / "short".
    pub side: String,
    pub entry: f32,
    pub stop: f32,
    pub target: Option<f32>,
    /// Mark-to-market in R at the source's last look.
    pub open_r: f32,
    pub opened_ms: i64,
    pub why: String,
}

impl OpenPosition {
    pub fn is_long(&self) -> bool {
        self.side == "long"
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TraderSnapshot {
    pub arch: Archetype,
    pub source: Source,
    pub day: DayStats,
    pub open: Option<OpenPosition>,
    /// Newest first.
    pub last_trades: Vec<DeskTrade>,
    /// The pit of the most recent trade or decision, for a trader with nothing open.
    pub last_pit: Option<Instrument>,
    /// Per pit, why the trader passed at its last look there.
    pub waiting: Vec<DeskPass>,
    /// False for a trader the desk is not running (it keeps only some of `ALL` and fills the
    /// other seats from the evolving pit): its avatar is hidden rather than left wandering.
    pub on_desk: bool,
    /// The Traders table's own figures for this trader, (net R, trades) today (since 00:00
    /// UTC), when the snapshot comes from the desk's board.
    pub board: Option<(f32, u32)>,
}

impl TraderSnapshot {
    pub fn empty(arch: Archetype, source: Source) -> Self {
        TraderSnapshot { arch, source, day: DayStats::default(), open: None, last_trades: Vec::new(), last_pit: None, waiting: Vec::new(), on_desk: true, board: None }
    }

    /// The R and trade count shown over the figure and on its panel: the table's own numbers
    /// when there is a table row (so the two always agree), else the day's.
    pub fn headline(&self) -> (f32, u32) {
        self.board.unwrap_or((self.day.net_r, self.day.trades))
    }

    pub fn open_pit(&self) -> Option<Instrument> {
        self.open.as_ref().and_then(|o| o.inst)
    }
}

/// A pit from either name it goes by: the floor's ("XAG/USD") or the tape's ("XAGUSD").
pub fn instrument_for_symbol(s: &str) -> Option<Instrument> {
    Instrument::ALL.iter().copied().find(|i| i.symbol() == s || tape_symbol(*i) == s)
}

/// The in-tab book notes decisions as "<symbol> <side> ..." lines; the symbol leads.
fn instrument_from_note(note: &str) -> Option<Instrument> {
    Instrument::ALL.iter().copied().find(|i| note.starts_with(i.symbol()))
}

/// One trader's snapshot from its row on the desk's board.
fn snapshot_of(arch: Archetype, t: &DeskTrader) -> TraderSnapshot {
    let open = t.open.as_ref().map(|o| OpenPosition {
        inst: instrument_for_symbol(&o.symbol),
        symbol: o.symbol.clone(),
        side: o.side.clone(),
        entry: o.entry,
        stop: o.stop,
        target: o.target,
        open_r: o.open_r,
        opened_ms: o.opened_ms,
        why: o.why.clone(),
    });
    let last_pit = t.last_trades.first().and_then(|lt| instrument_for_symbol(&lt.symbol));
    TraderSnapshot {
        arch,
        source: Source::Desk,
        day: DayStats {
            trades: t.day_trades,
            wins: t.day_wins,
            losses: t.day_losses,
            net_r: t.day_r,
            spread_paid_r: t.day_spread_paid_r,
            max_drawdown_r: t.day_max_drawdown_r,
        },
        open,
        last_trades: t.last_trades.iter().take(LAST_TRADES_KEEP).cloned().collect(),
        last_pit,
        waiting: t.waiting.clone(),
        on_desk: true,
        board: Some((t.day_r, t.day_trades)),
    }
}

/// The `Archetype` a genome on the board goes by in this tab: registered for display from
/// the board's pit block (id and description), or by name alone from an older desk.
pub fn genome_arch(board: &DeskBoard, name: &str) -> Archetype {
    let seat = board.pit.as_ref().and_then(|p| p.seat(name));
    let id = seat.map(|s| s.id.clone()).unwrap_or_else(|| format!("name:{name}"));
    let creed = seat.map(|s| s.creed.as_str()).unwrap_or("");
    Archetype::Genome(crate::genome::register_display(&id, name, creed, seat.and_then(|s| s.tag.as_deref())))
}

/// Snapshots in `Archetype::ALL` order from the desk's board, then one per genome on it. A
/// built-in trader the board does not have (an older desk, or one the desk does not keep) is
/// empty and marked off the desk.
pub fn snapshots_from_board(board: &DeskBoard) -> Vec<TraderSnapshot> {
    let mut snaps: Vec<TraderSnapshot> = Archetype::ALL
        .iter()
        .map(|&arch| match board.trader(arch) {
            Some(t) => TraderSnapshot { on_desk: t.is_active(), ..snapshot_of(arch, t) },
            None => TraderSnapshot { on_desk: false, ..TraderSnapshot::empty(arch, Source::Desk) },
        })
        .collect();
    // Only traders who have traded in the window or hold a position walk the floor; an idle
    // genome gets no body until it trades.
    snaps.extend(board.traders.iter().filter(|t| t.is_genome() && t.is_active()).map(|t| snapshot_of(genome_arch(board, &t.trader), t)));
    snaps
}

/// The same from the in-tab book, marked at the feed's current touch.
pub fn snapshots_from_book(book: &PaperBook, feed: &LiveFeed, now_ms: i64) -> Vec<TraderSnapshot> {
    let day_start = day_start_ms(now_ms);
    Archetype::ALL
        .iter()
        .map(|&arch| {
            let Some((i, tr)) = book.traders.iter().enumerate().find(|(_, t)| t.archetype == arch) else {
                return TraderSnapshot::empty(arch, Source::InTab);
            };
            let open = tr.position.as_ref().map(|p| OpenPosition {
                inst: Some(p.inst),
                symbol: p.inst.symbol().to_string(),
                side: if p.side.is_buy() { "long" } else { "short" }.to_string(),
                entry: p.entry,
                stop: p.stop,
                target: p.target,
                open_r: book.open_r(i, feed).unwrap_or(0.0),
                opened_ms: p.opened_ms,
                why: p.why.clone(),
            });
            let last_trades: Vec<DeskTrade> = tr.trades.iter().rev().take(LAST_TRADES_KEEP).map(DeskTrade::from_trade).collect();
            let last_pit = tr
                .whys
                .front()
                .and_then(|w| instrument_from_note(w))
                .or_else(|| tr.trades.last().and_then(|t| instrument_for_symbol(t.symbol)));
            let waiting = tr.passes().into_iter().map(|(inst, why)| DeskPass::new(inst.symbol(), why)).collect();
            TraderSnapshot { arch, source: Source::InTab, day: day_stats_from_trades(&tr.trades, day_start), open, last_trades, last_pit, waiting, on_desk: true, board: None }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Where an avatar should be
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Goal {
    /// Holding a position there: at the rail, facing the pulpit, hands up.
    AtRail(Instrument),
    /// Nothing open: standing by the pit of the last trade or decision.
    Visit(Instrument),
    /// Nothing yet today: strolling the concourse.
    Wander,
}

impl Goal {
    pub fn pit(self) -> Option<Instrument> {
        match self {
            Goal::AtRail(i) | Goal::Visit(i) => Some(i),
            Goal::Wander => None,
        }
    }
}

/// Open position wins, then the last pit, otherwise wander.
pub fn pick_goal(open: Option<Instrument>, last: Option<Instrument>) -> Goal {
    match (open, last) {
        (Some(i), _) => Goal::AtRail(i),
        (None, Some(i)) => Goal::Visit(i),
        (None, None) => Goal::Wander,
    }
}

// ---------------------------------------------------------------------------
// The overhead label
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Up,
    Down,
    Flat,
}

impl Tone {
    pub fn color(self) -> Color {
        match self {
            Tone::Up => Color::srgb(0.30, 1.0, 0.50),
            Tone::Down => Color::srgb(1.0, 0.38, 0.38),
            Tone::Flat => Color::srgb(0.72, 0.75, 0.80),
        }
    }
}

/// Grey until the first trade of the day and when the day rounds to flat.
pub fn tone_for(day_r: f32, day_trades: u32) -> Tone {
    if day_trades == 0 || day_r.abs() < 0.05 {
        Tone::Flat
    } else if day_r > 0.0 {
        Tone::Up
    } else {
        Tone::Down
    }
}

/// `GRACE  +1.3R`; a trader with no trades yet reads `0.0R` without a sign.
/// The figure's name line, with R to two places as the Traders table shows it.
pub fn label_text(name: &str, r: f32, trades: u32) -> String {
    if trades == 0 {
        format!("{}  0.00R", name.to_uppercase())
    } else {
        format!("{}  {:+.2}R", name.to_uppercase(), r)
    }
}

/// The second line while a position is open: `LONG XAG/USD  +0.35R`.
pub fn open_line(open: &OpenPosition) -> String {
    format!("{} {}  {:+.2}R", open.side.to_uppercase(), open.symbol, open.open_r)
}

// ---------------------------------------------------------------------------
// The concourse: where the avatars may walk
// ---------------------------------------------------------------------------

/// A figure's centre never comes closer than this to a pit's centre: the rim is at 38, the
/// apron ends at 41.5, the brass rail stands at 38.6. The margin over the apron covers a
/// route cutting a cell corner (half a cell diagonal is 5.7 units).
pub const AVOID_RADIUS: f32 = 44.5;
/// Standing at the rail means this far from the pit centre, on the concourse.
pub const RAIL_STAND_RADIUS: f32 = 46.5;
/// Floor units per second: about 1.4 m/s at the floor's ten units to the metre.
pub const WALK_SPEED: f32 = 14.0;
/// How long the arrival shuffle lasts.
pub const SHUFFLE_SECS: f32 = 3.0;

const CELL: f32 = 8.0;
const GRID_MIN: Vec2 = Vec2::new(-252.0, -196.0);
const GRID_W: usize = 63;
const GRID_H: usize = 56;

/// A pit's centre in floor-local XZ (the floor plane's `y` is dropped).
pub fn pit_xz(inst: Instrument) -> Vec2 {
    let o = pit_3d_origin(inst) - PIT_3D_ORIGIN;
    Vec2::new(o.x, o.z)
}

/// Floor-local XZ to the world point an avatar's feet stand on.
pub fn world_of(p: Vec2) -> Vec3 {
    PIT_3D_ORIGIN + Vec3::new(p.x, 0.0, p.y)
}

fn in_bounds(p: Vec2) -> bool {
    let max = GRID_MIN + Vec2::new(GRID_W as f32, GRID_H as f32) * CELL;
    p.x >= GRID_MIN.x && p.y >= GRID_MIN.y && p.x <= max.x && p.y <= max.y
}

/// On the concourse: inside the floor and clear of every pit.
pub fn is_free(p: Vec2) -> bool {
    in_bounds(p) && Instrument::ALL.iter().all(|i| p.distance(pit_xz(*i)) >= AVOID_RADIUS)
}

/// The straight line stays on the concourse (sampled every two units).
pub fn segment_clear(a: Vec2, b: Vec2) -> bool {
    let n = (a.distance(b) / 2.0).ceil().max(1.0) as usize;
    (0..=n).all(|k| is_free(a.lerp(b, k as f32 / n as f32)))
}

fn cell_of(p: Vec2) -> (usize, usize) {
    let c = ((p - GRID_MIN) / CELL).floor();
    (c.x.clamp(0.0, (GRID_W - 1) as f32) as usize, c.y.clamp(0.0, (GRID_H - 1) as f32) as usize)
}

fn cell_centre(cx: usize, cy: usize) -> Vec2 {
    GRID_MIN + Vec2::new(cx as f32 + 0.5, cy as f32 + 0.5) * CELL
}

fn cell_free(cx: usize, cy: usize) -> bool {
    is_free(cell_centre(cx, cy))
}

/// The nearest free cell to a point (the point itself may sit in the margin, e.g. a rail stand
/// whose cell centre is a shade inside `AVOID_RADIUS`).
fn nearest_free_cell(p: Vec2) -> (usize, usize) {
    let (cx, cy) = cell_of(p);
    if cell_free(cx, cy) {
        return (cx, cy);
    }
    let mut best = None::<((usize, usize), f32)>;
    for r in 1..=3isize {
        for dy in -r..=r {
            for dx in -r..=r {
                let (x, y) = (cx as isize + dx, cy as isize + dy);
                if x < 0 || y < 0 || x >= GRID_W as isize || y >= GRID_H as isize {
                    continue;
                }
                let (x, y) = (x as usize, y as usize);
                if cell_free(x, y) {
                    let d = cell_centre(x, y).distance(p);
                    if best.is_none_or(|(_, bd)| d < bd) {
                        best = Some(((x, y), d));
                    }
                }
            }
        }
        if let Some((c, _)) = best {
            return c;
        }
    }
    (cx, cy)
}

/// A walkable path from `from` to `to` along the concourse: A* over the cell grid (eight
/// neighbours, no corner cutting), then pulled straight wherever the line between two
/// waypoints is clear. The last waypoint is `to` itself. Empty only if the grid is severed,
/// which the floor plan does not allow.
pub fn route(from: Vec2, to: Vec2) -> Vec<Vec2> {
    let start = nearest_free_cell(from);
    let goal = nearest_free_cell(to);
    let idx = |(x, y): (usize, usize)| y * GRID_W + x;
    let mut g = vec![u32::MAX; GRID_W * GRID_H];
    let mut came = vec![usize::MAX; GRID_W * GRID_H];
    let mut open = BinaryHeap::new();
    let h = |(x, y): (usize, usize)| {
        let dx = (x as i32 - goal.0 as i32).unsigned_abs();
        let dy = (y as i32 - goal.1 as i32).unsigned_abs();
        10 * dx.max(dy) + 4 * dx.min(dy)
    };
    g[idx(start)] = 0;
    open.push(Reverse((h(start), idx(start))));
    let mut found = false;
    while let Some(Reverse((_, cur))) = open.pop() {
        let c = (cur % GRID_W, cur / GRID_W);
        if c == goal {
            found = true;
            break;
        }
        for dy in -1i32..=1 {
            for dx in -1i32..=1 {
                if dx == 0 && dy == 0 {
                    continue;
                }
                let (nx, ny) = (c.0 as i32 + dx, c.1 as i32 + dy);
                if nx < 0 || ny < 0 || nx >= GRID_W as i32 || ny >= GRID_H as i32 {
                    continue;
                }
                let n = (nx as usize, ny as usize);
                if !cell_free(n.0, n.1) {
                    continue;
                }
                // Diagonals need both orthogonal neighbours free or the figure clips a rim.
                if dx != 0 && dy != 0 && !(cell_free(n.0, c.1) && cell_free(c.0, n.1)) {
                    continue;
                }
                let step = if dx != 0 && dy != 0 { 14 } else { 10 };
                let ng = g[cur].saturating_add(step);
                if ng < g[idx(n)] {
                    g[idx(n)] = ng;
                    came[idx(n)] = cur;
                    open.push(Reverse((ng + h(n), idx(n))));
                }
            }
        }
    }
    if !found {
        return Vec::new();
    }
    let mut cells = vec![goal];
    let mut cur = idx(goal);
    while cur != idx(start) {
        cur = came[cur];
        cells.push((cur % GRID_W, cur / GRID_W));
    }
    cells.reverse();
    let mut pts: Vec<Vec2> = cells.iter().map(|&(x, y)| cell_centre(x, y)).collect();
    pts.push(to);
    pts[0] = from;
    // String pulling: from each kept point jump to the furthest one it can see.
    let mut out = vec![pts[0]];
    let mut i = 0;
    while i + 1 < pts.len() {
        let mut j = pts.len() - 1;
        while j > i + 1 && !segment_clear(pts[i], pts[j]) {
            j -= 1;
        }
        out.push(pts[j]);
        i = j;
    }
    out.remove(0);
    out
}

/// The `k`th stand at a pit's rail: north-east first, then north-west, then further round
/// each side, so several traders at one pit line the rail rather than stack. The north side
/// is deliberate: the pit camera looks north from the south rim, so a figure there is seen
/// from the front in the free upper-middle of the screen (the south rail projects under the
/// pit overlay panel), and in market view its label clears the pit's nameplate. Returns the
/// stand and the point to face (the pulpit on the north rim).
pub fn rail_stand(inst: Instrument, k: usize) -> (Vec2, Vec2) {
    use std::f32::consts::FRAC_PI_2;
    let side = if k % 2 == 0 { 1.0 } else { -1.0 };
    let angle = -FRAC_PI_2 + side * (0.75 + 0.30 * (k / 2) as f32);
    let c = pit_xz(inst);
    (c + Vec2::new(angle.cos(), angle.sin()) * RAIL_STAND_RADIUS, c + Vec2::new(0.0, PULPIT_OFFSET_Z))
}

/// Standing inside the pit while in a trade: on the top step, at the same angle as the
/// trader's rail stand, facing the well. The walk goes to the rail first (the concourse
/// grid keeps pit interiors blocked) and then straight down onto the step.
pub const PIT_STAND_RADIUS: f32 = 29.0;

pub fn pit_stand(inst: Instrument, k: usize) -> (Vec2, Vec2) {
    let (rail, _) = rail_stand(inst, k);
    let c = pit_xz(inst);
    let dir = (rail - c).normalize_or_zero();
    (c + dir * PIT_STAND_RADIUS, c)
}

/// The two aisles between the district rows (metals | indices | forex), midway between rows.
const AISLE_Z: [f32; 2] = [-crate::pit_3d::DISTRICT_ROW_Z * 0.5, crate::pit_3d::DISTRICT_ROW_Z * 0.5];

/// Where the avatars first appear: spaced along the two aisles, alternating, so the crowd
/// opens on the concourse rather than in anyone's pit.
pub fn spawn_spot(i: usize) -> Vec2 {
    let n = Archetype::ALL.len() + GENOME_SLOTS;
    let per_aisle = n.div_ceil(2);
    let k = i / 2;
    let x = -196.0 + 392.0 * k as f32 / (per_aisle - 1).max(1) as f32;
    Vec2::new(x, AISLE_Z[i % 2])
}

/// Places a wandering trader may head for: the aisles, the row ends and the open floor
/// behind the metals and in front of the forex pits.
pub fn wander_spots() -> Vec<Vec2> {
    let mut spots: Vec<Vec2> = (0..Archetype::ALL.len()).map(spawn_spot).collect();
    let row = crate::pit_3d::DISTRICT_ROW_Z;
    let mut extra: Vec<(f32, f32)> = Vec::new();
    for &z in &AISLE_Z {
        for x in [-228.0, -114.0, 0.0, 114.0, 228.0] {
            extra.push((x, z));
        }
    }
    for z in [-row, 0.0, row] {
        extra.push((-236.0, z));
        extra.push((236.0, z));
    }
    for x in [-140.0, 0.0, 140.0] {
        extra.push((x, -row - 62.0));
        extra.push((x, row + 62.0));
    }
    spots.extend(extra.iter().map(|&(x, z)| Vec2::new(x, z)));
    spots.retain(|p| is_free(*p));
    spots.dedup_by(|a, b| a.distance(*b) < 1.0);
    spots
}

/// Yaw that turns the figure's front (+Z) toward `dir` on the floor.
pub fn yaw_toward(dir: Vec2) -> f32 {
    dir.x.atan2(dir.y)
}

fn wrap_angle(a: f32) -> f32 {
    use std::f32::consts::{PI, TAU};
    (a + PI).rem_euclid(TAU) - PI
}

fn fmt_hm(ms: i64) -> String {
    let m = ms.div_euclid(1000).rem_euclid(86_400) / 60;
    format!("{:02}:{:02}", m / 60, m % 60)
}

fn fmt_session(s: &Session) -> String {
    format!("{:02}:{:02}-{:02}:{:02}", s.from / 60, s.from % 60, s.until / 60, s.until % 60)
}

fn fmt_px(inst: Option<Instrument>, px: f32) -> String {
    match inst {
        Some(i) => format_price_with_commas(px as f64, instrument_decimals(i)),
        None => format!("{px:.2}"),
    }
}

/// Reasons are cut so a trade stays on one line of the panel.
fn short(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        s.chars().take(n.saturating_sub(1)).collect::<String>() + "\u{2026}"
    }
}

/// How a line of the detail panel is coloured.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LineTone {
    Plain,
    /// A section heading.
    Head,
    /// A P&L in R: green above zero, red below.
    Pnl(f32),
}

impl LineTone {
    pub fn color(self) -> Color {
        match self {
            LineTone::Plain => Color::srgb(0.80, 0.84, 0.90),
            LineTone::Head => Color::srgb(0.95, 0.82, 0.40),
            LineTone::Pnl(r) if r > 0.0 => Color::srgb(0.35, 0.95, 0.50),
            LineTone::Pnl(r) if r < 0.0 => Color::srgb(1.0, 0.42, 0.42),
            LineTone::Pnl(_) => Color::srgb(0.80, 0.84, 0.90),
        }
    }
}

/// The detail panel's lines with their colours: each past trade, the open position and the
/// day's total by the sign of their R; the section headings in gold.
pub fn detail_lines_toned(s: &TraderSnapshot, mark: Option<f32>, heartbeat: Option<String>, outlook: &[String]) -> Vec<(String, LineTone)> {
    let lines = detail_lines(s, mark, heartbeat, outlook);
    let trades_at = lines.iter().position(|l| l.starts_with("Last ") && l.contains("trades (newest first"));
    lines
        .into_iter()
        .enumerate()
        .map(|(i, l)| {
            let tone = match trades_at {
                Some(h) if i == h => LineTone::Head,
                Some(h) if i > h && i - h - 1 < s.last_trades.len() => LineTone::Pnl(s.last_trades[i - h - 1].pnl_r),
                _ if l.starts_with("OPEN:") => LineTone::Pnl(s.open.as_ref().map(|o| o.open_r).unwrap_or(0.0)),
                _ if l.starts_with("Today (since") && s.day.trades > 0 => LineTone::Pnl(s.day.net_r),
                _ if l.starts_with("Passing now (") || l.starts_with("Strategy:") || l.starts_with("Rules:") => LineTone::Head,
                _ => LineTone::Plain,
            };
            (l, tone)
        })
        .collect()
}

/// The detail panel's body, one line per entry.
pub fn detail_lines(s: &TraderSnapshot, mark: Option<f32>, heartbeat: Option<String>, outlook: &[String]) -> Vec<String> {
    let arch = s.arch;
    let rules = arch.rules();
    let mut lines = Vec::new();
    lines.push(format!("\"{}\"", arch.describe()));
    lines.push(format!("Strategy: {}", arch.strategy_text()));
    let sessions: Vec<String> = rules.sessions.iter().map(fmt_session).collect();
    lines.push(format!(
        "Rules: enters {} UTC  |  stop >= {:.0} spreads, target >= {:.0} spreads  |  Jev: {}",
        sessions.join(", "),
        rules.min_stop_spreads,
        rules.min_target_spreads,
        if arch.uses_jev() { "yes" } else { "no" }
    ));
    lines.extend(outlook.iter().cloned());
    let d = &s.day;
    if d.trades == 0 {
        lines.push("Today (since 00:00 UTC): no trades yet".into());
    } else {
        lines.push(format!(
            "Today (since 00:00 UTC): {} trades, {:.0}% win, {:+.2}R net, spread paid {:.2}R, max DD {:.2}R",
            d.trades,
            d.win_rate() * 100.0,
            d.net_r,
            d.spread_paid_r,
            d.max_drawdown_r
        ));
    }
    match &s.open {
        Some(o) => {
            let target = o.target.map(|t| fmt_px(o.inst, t)).unwrap_or_else(|| "none".into());
            let mark = mark.map(|m| format!("  mark {}", fmt_px(o.inst, m))).unwrap_or_default();
            lines.push(format!(
                "OPEN: {} {} from {}  stop {}  target {}{}  {:+.2}R  since {} UTC",
                o.side.to_uppercase(),
                o.symbol,
                fmt_px(o.inst, o.entry),
                fmt_px(o.inst, o.stop),
                target,
                mark,
                o.open_r,
                fmt_hm(o.opened_ms)
            ));
            lines.push(format!("      why: {}", short(&o.why, 96)));
        }
        None => lines.push("Flat: no open position".into()),
    }
    // Why not now: the idea's own words at its last look at each pit. This is the line to
    // read when a trader has booked nothing all day.
    if s.waiting.is_empty() {
        if s.open.is_none() {
            lines.push("Passing now: no pit looked at yet".into());
        }
    } else {
        lines.push("Passing now (last look per pit):".into());
        for w in &s.waiting {
            lines.push(format!(" {:<7} {}", w.symbol, short(&w.why, 80)));
        }
    }
    if s.last_trades.is_empty() {
        lines.push("Last trades: none on record".into());
    } else {
        lines.push(format!("Last {} trades (newest first, UTC):", s.last_trades.len()));
        for t in &s.last_trades {
            let inst = instrument_for_symbol(&t.symbol);
            // One line each beside the portrait (about 78 monospace characters).
            lines.push(format!(
                " {} {:<7} {:<5} {} -> {} {:+.2}R {} | {}",
                fmt_hm(t.closed_ms),
                t.symbol,
                t.side,
                fmt_px(inst, t.entry),
                fmt_px(inst, t.exit),
                t.pnl_r,
                t.why_out,
                short(&t.why_in, 14)
            ));
        }
    }
    let src = match (s.source, heartbeat) {
        (Source::Desk, Some(hb)) => format!("Source: {}  |  {hb}", s.source.label()),
        (src, _) => format!("Source: {}", src.label()),
    };
    lines.push(src);
    lines
}

// ---------------------------------------------------------------------------
// Resources and components
// ---------------------------------------------------------------------------

/// The latest snapshot of every trader, refreshed a few times a second from whichever
/// source is live.
#[derive(Resource)]
pub struct PaperAvatars {
    pub snapshots: Vec<TraderSnapshot>,
    pub source: Source,
    pub revision: u64,
    next_refresh: f64,
}

impl Default for PaperAvatars {
    fn default() -> Self {
        Self {
            snapshots: Archetype::ALL.iter().map(|a| TraderSnapshot::empty(*a, Source::InTab)).collect(),
            source: Source::InTab,
            revision: 0,
            next_refresh: 0.0,
        }
    }
}

impl PaperAvatars {
    pub fn snapshot(&self, arch: Archetype) -> Option<&TraderSnapshot> {
        self.snapshots.iter().find(|s| s.arch == arch)
    }

    /// Goal for a trader, and its rank among the traders heading for the same pit (so
    /// several at one rail spread out).
    pub fn goal_and_rank(&self, arch: Archetype) -> (Goal, usize) {
        let goal_of = |s: &TraderSnapshot| pick_goal(s.open_pit(), s.last_pit);
        let Some(me) = self.snapshot(arch) else { return (Goal::Wander, 0) };
        let goal = goal_of(me);
        let rank = match goal.pit() {
            Some(pit) => self.snapshots.iter().take_while(|s| s.arch != arch).filter(|s| goal_of(s).pit() == Some(pit)).count(),
            None => 0,
        };
        (goal, rank)
    }
}

/// Screen-space picking for the avatars and the open detail panel. Written by
/// `project_avatar_labels` / `update_trader_detail_panel`, read by the camera's click path.
#[derive(Resource, Default)]
pub struct AvatarPicking {
    /// Each avatar's screen rectangle (figure plus label), logical px.
    pub hits: Vec<(Archetype, Rect)>,
    /// The detail panel's rectangle while it is open.
    pub panel: Option<Rect>,
    /// Set by the picker, consumed by `handle_avatar_clicks`.
    pub clicked: Option<Archetype>,
}

impl AvatarPicking {
    /// The avatar under a point, nearest centre first when rectangles overlap.
    pub fn hit_at(&self, p: Vec2) -> Option<Archetype> {
        self.hits
            .iter()
            .filter(|(_, r)| r.contains(p))
            .min_by(|a, b| a.1.center().distance(p).partial_cmp(&b.1.center().distance(p)).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(a, _)| *a)
    }

    /// The camera's click path asks this first: true means the click was ours (an avatar,
    /// which opens its panel, or the open panel itself) and must not focus a pit.
    pub fn consume_click(&mut self, p: Vec2) -> bool {
        if self.panel.is_some_and(|r| r.contains(p)) {
            return true;
        }
        match self.hit_at(p) {
            Some(a) => {
                self.clicked = Some(a);
                true
            }
            None => false,
        }
    }
}

/// The detail panel's scrolling area, its scrollbar track and thumb, and one text span per
/// line of its body (so each line can have its own colour).
#[derive(Component)]
pub struct TraderDetailScroll;
#[derive(Component)]
pub struct TraderDetailTrack;
#[derive(Component)]
pub struct TraderDetailThumb;
#[derive(Component)]
pub struct TraderDetailLine(pub usize);
/// Spans kept for the body's lines; a longer body is cut here.
const DETAIL_LINES: usize = 80;
/// Pixels scrolled per wheel notch.
const SCROLL_LINE_PX: f32 = 36.0;

/// Which trader's panel is open.
#[derive(Resource, Default)]
pub struct TraderDetail {
    pub open: Option<Archetype>,
}

#[derive(Component)]
pub struct PaperAvatar {
    pub arch: Archetype,
    pub index: usize,
}

/// Bodies kept for the evolving pit's genomes, beyond the built-in traders'. A slot with no
/// genome on it wears a placeholder `Archetype::Genome` no seat has, so it has no snapshot
/// and stays hidden.
pub const GENOME_SLOTS: usize = 64;

/// On a genome slot's body and on its label: which slot, so a reassignment moves both.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct AvatarSlot(pub usize);

/// The materials a body wears, kept so a genome slot can be repainted for its new owner.
#[derive(Component, Debug, Clone)]
pub struct AvatarPaint {
    pub jacket: Handle<StandardMaterial>,
    pub halo: Handle<StandardMaterial>,
}

fn slot_placeholder(slot: usize) -> Archetype {
    Archetype::Genome(u16::MAX - slot as u16)
}

fn paint(materials: &mut Assets<StandardMaterial>, p: &AvatarPaint, colour: Color) {
    if let Some(m) = materials.get_mut(&p.jacket) {
        m.base_color = colour;
        m.emissive = LinearRgba::from(colour) * 0.35;
    }
    if let Some(m) = materials.get_mut(&p.halo) {
        m.base_color = colour;
        m.emissive = LinearRgba::from(colour) * 2.5;
    }
}

/// Walking state, floor-local.
#[derive(Component)]
pub struct AvatarMotion {
    pub goal: Goal,
    pub waypoints: Vec<Vec2>,
    pub pos: Vec2,
    pub yaw: f32,
    /// What to face once arrived.
    pub face: Option<Vec2>,
    pub arrived_at: Option<f32>,
    pub next_wander_at: f32,
    pub walk_phase: f32,
    pub rng: XorShift,
}

impl AvatarMotion {
    pub fn is_walking(&self) -> bool {
        !self.waypoints.is_empty()
    }
}

#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub enum AvatarPart {
    LegLeft,
    LegRight,
    Body,
    Head,
    HandLeft,
    HandRight,
    /// A sleeve from the shoulder to the hand, re-laid every frame as the hand moves.
    ArmLeft,
    ArmRight,
    /// The diamond floating over a paper trader's head: what marks them out from the locals.
    Marker,
}

/// Shoulder joints, in the figure's frame, that the sleeves hang from.
const SHOULDER_Y: f32 = 16.2;
const SHOULDER_X: f32 = 2.45;

/// Where a hand is headed: up on the side of the trade at the rail, else hanging, swinging.
fn hand_target(left: bool, open: bool, t: f32, swing: f32) -> Vec3 {
    let x = if left { -2.75 } else { 2.75 };
    if open {
        let bobh = ((t * 5.0) + if left { 0.0 } else { 1.5 }).sin() * 0.35;
        Vec3::new(x, 16.0 + bobh, 1.5)
    } else {
        Vec3::new(x, 8.6, 0.4 + if left { -swing } else { swing } * 2.5)
    }
}

/// A sleeve laid from the shoulder to the hand: a unit cuboid stretched along the arm.
fn sleeve_between(shoulder: Vec3, hand: Vec3) -> Transform {
    let d = hand - shoulder;
    let len = d.length().max(0.5);
    Transform::from_translation(shoulder + d * 0.5).with_rotation(Quat::from_rotation_arc(Vec3::NEG_Y, d / len)).with_scale(Vec3::new(1.0, len, 1.0))
}

#[derive(Resource, Clone)]
pub struct AvatarMaterials {
    pub hand_idle: Handle<StandardMaterial>,
    pub hand_long: Handle<StandardMaterial>,
    pub hand_short: Handle<StandardMaterial>,
}

#[derive(Component)]
pub struct AvatarLabel(pub Archetype);

#[derive(Component)]
pub struct AvatarLabelLine(pub u8);

#[derive(Component)]
pub struct TraderDetailPanel;

#[derive(Component)]
pub struct TraderDetailTitle;

#[derive(Component)]
pub struct TraderDetailBody;

/// The portrait at the left of the detail panel: jacket and halo recolour per trader.
#[derive(Component)]
pub struct TraderPortraitJacket;

#[derive(Component)]
pub struct TraderPortraitHalo;


const LABEL_W: f32 = 170.0;
const LABEL_H: f32 = 32.0;
const PANEL_LEFT: f32 = 8.0;
const PANEL_BOTTOM: f32 = 140.0;
/// As wide as the Traders panel above it, so a trade with its reason fits one line.
const PANEL_W: f32 = 560.0;
const PORTRAIT_W: f32 = 54.0;
const REFRESH_SECS: f64 = 0.5;
const HEAD_TOP: f32 = 19.1;

fn jacket_color(arch: Archetype) -> Color {
    match arch {
        Archetype::Control => Color::srgb(0.52, 0.54, 0.58),
        a => a.color(),
    }
}

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

/// Startup: one figure per archetype on the spawn ring, their labels, and the (hidden) detail panel.
pub fn setup_paper_avatars(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>, mut materials: ResMut<Assets<StandardMaterial>>) {
    let legs_mesh = meshes.add(Cuboid::new(1.4, 9.5, 1.9));
    let body_mesh = meshes.add(Cuboid::new(3.9, 7.5, 2.4));
    let head_mesh = meshes.add(Cuboid::new(1.9, 2.1, 1.9));
    let hand_mesh = meshes.add(Cuboid::new(1.1, 1.3, 0.5));
    let badge_mesh = meshes.add(Cuboid::new(1.8, 1.1, 0.14));
    let halo_mesh = meshes.add(Torus::new(4.0, 4.4));
    let coin_mesh = meshes.add(Cylinder::new(0.75, 0.16));
    let shoe_mesh = meshes.add(Cuboid::new(1.5, 0.7, 2.6));
    let shoulder_mesh = meshes.add(Cuboid::new(1.4, 1.1, 2.5));
    let sleeve_mesh = meshes.add(Cuboid::new(1.15, 1.0, 1.15));
    let shirt_mesh = meshes.add(Cuboid::new(1.3, 3.2, 0.1));
    let tie_mesh = meshes.add(Cuboid::new(0.5, 2.7, 0.12));
    let piping_mesh = meshes.add(Cuboid::new(0.2, 3.7, 0.1));
    let hair_top_mesh = meshes.add(Cuboid::new(2.05, 0.5, 2.05));
    let hair_back_mesh = meshes.add(Cuboid::new(2.05, 1.5, 0.5));
    let ear_mesh = meshes.add(Cuboid::new(0.2, 0.5, 0.3));
    let band_mesh = meshes.add(Cuboid::new(2.4, 0.24, 0.32));
    let earcup_mesh = meshes.add(Cuboid::new(0.4, 0.85, 0.85));
    let mic_mesh = meshes.add(Cuboid::new(0.12, 0.12, 1.3));
    let marker_mesh = meshes.add(Cuboid::new(0.95, 0.95, 0.95));

    let trousers = materials.add(StandardMaterial { base_color: Color::srgb_u8(0x1c, 0x1e, 0x26), perceptual_roughness: 0.8, ..default() });
    let shoes = materials.add(StandardMaterial { base_color: Color::srgb_u8(0x0c, 0x0c, 0x0e), perceptual_roughness: 0.3, metallic: 0.2, ..default() });
    let shirt = materials.add(StandardMaterial { base_color: Color::srgb(0.95, 0.95, 0.97), perceptual_roughness: 0.6, ..default() });
    let tie = materials.add(StandardMaterial { base_color: Color::srgb_u8(0x14, 0x15, 0x1a), perceptual_roughness: 0.4, ..default() });
    let headset = materials.add(StandardMaterial { base_color: Color::srgb_u8(0x22, 0x24, 0x2a), perceptual_roughness: 0.35, metallic: 0.5, ..default() });
    let badge = materials.add(StandardMaterial {
        base_color: Color::WHITE,
        emissive: LinearRgba::from(Color::srgb(0.9, 0.9, 0.9)) * 0.8,
        perceptual_roughness: 0.4,
        ..default()
    });
    let coin = materials.add(StandardMaterial {
        base_color: Color::srgb_u8(0xf0, 0xc4, 0x3a),
        emissive: LinearRgba::from(Color::srgb(0.95, 0.75, 0.15)) * 1.5,
        metallic: 0.9,
        perceptual_roughness: 0.25,
        ..default()
    });
    let mats = AvatarMaterials {
        hand_idle: materials.add(StandardMaterial { base_color: Color::srgb(0.95, 0.78, 0.62), ..default() }),
        hand_long: materials.add(StandardMaterial {
            base_color: Color::srgb(0.18, 1.0, 0.45),
            emissive: LinearRgba::from(Color::srgb(0.10, 0.9, 0.35)) * 3.0,
            ..default()
        }),
        hand_short: materials.add(StandardMaterial {
            base_color: Color::srgb(1.0, 0.22, 0.22),
            emissive: LinearRgba::from(Color::srgb(0.9, 0.15, 0.15)) * 3.0,
            ..default()
        }),
    };

    let roster: Vec<(Archetype, Option<usize>)> =
        Archetype::ALL.iter().map(|a| (*a, None)).chain((0..GENOME_SLOTS).map(|k| (slot_placeholder(k), Some(k)))).collect();
    for (index, &(arch, slot)) in roster.iter().enumerate() {
        let colour = jacket_color(arch);
        let jacket = materials.add(StandardMaterial {
            base_color: colour,
            emissive: LinearRgba::from(colour) * 0.35,
            perceptual_roughness: 0.55,
            ..default()
        });
        let halo = materials.add(StandardMaterial {
            base_color: colour,
            emissive: LinearRgba::from(colour) * 2.5,
            unlit: true,
            ..default()
        });
        let (_, skin_colour, hair_colour) = crate::unimate::sample_demographics(700 + index as u64 * 37);
        let skin = materials.add(StandardMaterial { base_color: skin_colour, perceptual_roughness: 0.65, ..default() });
        let hair = materials.add(StandardMaterial { base_color: hair_colour, perceptual_roughness: 0.85, ..default() });
        let spot = spawn_spot(index);
        let face = pit_xz(Instrument::Sp500) - spot;
        let paint = AvatarPaint { jacket: jacket.clone(), halo: halo.clone() };
        let mut root = commands.spawn(paint);
        if let Some(k) = slot {
            // Hidden until a genome takes the slot (`assign_genome_avatars`).
            root.insert(AvatarSlot(k));
        }
        root
            .insert((
                PaperAvatar { arch, index },
                AvatarMotion {
                    goal: Goal::Wander,
                    waypoints: Vec::new(),
                    pos: spot,
                    yaw: yaw_toward(face),
                    face: None,
                    arrived_at: Some(0.0),
                    next_wander_at: 4.0 + index as f32 * 1.7,
                    walk_phase: 0.0,
                    rng: XorShift::new(0xa7a7_0000 + index as u64 * 7919),
                },
                Transform::from_translation(world_of(spot)).with_rotation(Quat::from_rotation_y(yaw_toward(face))),
                if slot.is_some() { Visibility::Hidden } else { Visibility::Inherited },
            ))
            .with_children(|p| {
                // A paper trader dresses as a desk trader, not a local: a suit in their own colour
                // with glowing piping, white shirt and dark tie, a headset, and a diamond overhead.
                // The locals wear exchange trading jackets and shades.
                for (part, x) in [(AvatarPart::LegLeft, -0.9), (AvatarPart::LegRight, 0.9)] {
                    p.spawn((part, Mesh3d(legs_mesh.clone()), MeshMaterial3d(trousers.clone()), Transform::from_xyz(x, 4.75, 0.0)))
                        .with_children(|l| {
                            l.spawn((Mesh3d(shoe_mesh.clone()), MeshMaterial3d(shoes.clone()), Transform::from_xyz(0.0, -4.55, 0.35)));
                        });
                }
                p.spawn((AvatarPart::Body, Mesh3d(body_mesh.clone()), MeshMaterial3d(jacket.clone()), Transform::from_xyz(0.0, 13.25, 0.0)))
                    .with_children(|b| {
                        for x in [-2.35, 2.35] {
                            b.spawn((Mesh3d(shoulder_mesh.clone()), MeshMaterial3d(jacket.clone()), Transform::from_xyz(x, 2.95, 0.0)));
                        }
                        b.spawn((Mesh3d(shirt_mesh.clone()), MeshMaterial3d(shirt.clone()), Transform::from_xyz(0.0, 2.1, 1.22)));
                        b.spawn((Mesh3d(tie_mesh.clone()), MeshMaterial3d(tie.clone()), Transform::from_xyz(0.0, 1.85, 1.3)));
                        for (x, a) in [(-0.82, -0.22f32), (0.82, 0.22)] {
                            b.spawn((Mesh3d(piping_mesh.clone()), MeshMaterial3d(halo.clone()), Transform::from_xyz(x, 1.95, 1.27).with_rotation(Quat::from_rotation_z(a))));
                        }
                        // White badge low on the left front, under the piping.
                        b.spawn((Mesh3d(badge_mesh.clone()), MeshMaterial3d(badge.clone()), Transform::from_xyz(-1.3, -0.4, 1.27)));
                    });
                p.spawn((AvatarPart::Head, Mesh3d(head_mesh.clone()), MeshMaterial3d(skin.clone()), Transform::from_xyz(0.0, HEAD_TOP - 1.05, 0.0)))
                    .with_children(|h| {
                        h.spawn((Mesh3d(hair_top_mesh.clone()), MeshMaterial3d(hair.clone()), Transform::from_xyz(0.0, 1.22, -0.05)));
                        h.spawn((Mesh3d(hair_back_mesh.clone()), MeshMaterial3d(hair.clone()), Transform::from_xyz(0.0, 0.35, -0.95)));
                        for x in [-1.02, 1.02] {
                            h.spawn((Mesh3d(ear_mesh.clone()), MeshMaterial3d(skin.clone()), Transform::from_xyz(x, 0.0, 0.0)));
                        }
                        h.spawn((Mesh3d(band_mesh.clone()), MeshMaterial3d(headset.clone()), Transform::from_xyz(0.0, 1.5, 0.05)));
                        for x in [-1.2, 1.2] {
                            h.spawn((Mesh3d(earcup_mesh.clone()), MeshMaterial3d(halo.clone()), Transform::from_xyz(x, 0.05, 0.05)));
                        }
                        h.spawn((Mesh3d(mic_mesh.clone()), MeshMaterial3d(headset.clone()), Transform::from_xyz(-1.05, -0.55, 0.7).with_rotation(Quat::from_rotation_y(0.35))));
                    });
                for (part, x) in [(AvatarPart::HandLeft, -2.75), (AvatarPart::HandRight, 2.75)] {
                    let mut hand = p.spawn((part, Mesh3d(hand_mesh.clone()), MeshMaterial3d(mats.hand_idle.clone()), Transform::from_xyz(x, 8.6, 0.4)));
                    if arch == Archetype::Control && part == AvatarPart::HandRight {
                        hand.with_children(|h| {
                            h.spawn((
                                Mesh3d(coin_mesh.clone()),
                                MeshMaterial3d(coin.clone()),
                                Transform::from_xyz(0.0, 1.1, 0.4).with_rotation(Quat::from_rotation_x(std::f32::consts::FRAC_PI_2)),
                            ));
                        });
                    }
                }
                for (part, left) in [(AvatarPart::ArmLeft, true), (AvatarPart::ArmRight, false)] {
                    let x = if left { -SHOULDER_X } else { SHOULDER_X };
                    let tf = sleeve_between(Vec3::new(x, SHOULDER_Y, 0.0), hand_target(left, false, 0.0, 0.0) + Vec3::Y * 0.6);
                    p.spawn((part, Mesh3d(sleeve_mesh.clone()), MeshMaterial3d(jacket.clone()), tf));
                }
                p.spawn((
                    AvatarPart::Marker,
                    Mesh3d(marker_mesh.clone()),
                    MeshMaterial3d(halo.clone()),
                    Transform::from_xyz(0.0, HEAD_TOP + 1.9, 0.0).with_rotation(Quat::from_rotation_x(std::f32::consts::FRAC_PI_4) * Quat::from_rotation_z(std::f32::consts::FRAC_PI_4)),
                ));
                p.spawn((Mesh3d(halo_mesh.clone()), MeshMaterial3d(halo), Transform::from_xyz(0.0, 0.3, 0.0)));
            });

        // Overhead label: name and the day's R, then the open position.
        let mut label = commands.spawn_empty();
        if let Some(k) = slot {
            label.insert(AvatarSlot(k));
        }
        label
            .insert((
                AvatarLabel(arch),
                Node {
                    position_type: PositionType::Absolute,
                    width: Val::Px(LABEL_W),
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    row_gap: Val::Px(0.0),
                    ..default()
                },
                Visibility::Hidden,
                // Floor-attached: under the panel columns and the detail panel.
                GlobalZIndex(-1),
            ))
            .with_children(|l| {
                l.spawn((
                    AvatarLabelLine(0),
                    Text::new(label_text(arch.name(), 0.0, 0)),
                    TextFont { font_size: 13.0, ..default() },
                    TextColor(Tone::Flat.color()),
                    Node { padding: UiRect::axes(Val::Px(5.0), Val::Px(1.0)), border_radius: BorderRadius::all(Val::Px(3.0)), ..default() },
                    BackgroundColor(Color::srgba(0.02, 0.03, 0.06, 0.72)),
                ));
                l.spawn((
                    AvatarLabelLine(1),
                    Text::new(""),
                    TextFont { font_size: 10.5, ..default() },
                    TextColor(Tone::Flat.color()),
                    Node {
                        display: Display::None,
                        padding: UiRect::axes(Val::Px(4.0), Val::Px(0.0)),
                        border_radius: BorderRadius::all(Val::Px(3.0)),
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.02, 0.03, 0.06, 0.72)),
                ));
            });
    }
    commands.insert_resource(mats);

    // The detail panel, bottom-left above the button rows, hidden until a click.
    commands
        .spawn((
            TraderDetailPanel,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(PANEL_LEFT),
                top: Val::Px(crate::panels::COLUMN_TOP),
                width: Val::Px(PANEL_W),
                overflow: Overflow::clip(),
                display: Display::None,
                flex_direction: FlexDirection::Column,
                padding: UiRect::axes(Val::Px(10.0), Val::Px(7.0)),
                row_gap: Val::Px(4.0),
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(Val::Px(6.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.02, 0.03, 0.06, 1.0)),
            BorderColor::all(Color::srgba(0.85, 0.70, 0.25, 0.55)),
            GlobalZIndex(9),
        ))
        .with_children(|panel| {
            let root = panel.target_entity();
            panel
                .spawn(Node {
                    flex_direction: FlexDirection::Row,
                    justify_content: JustifyContent::SpaceBetween,
                    align_items: AlignItems::Center,
                    column_gap: Val::Px(8.0),
                    ..default()
                })
                .with_children(|row| {
                    row.spawn((TraderDetailTitle, Text::new(""), TextFont { font_size: 12.5, ..default() }, TextColor(Color::WHITE)));
                    // The same X every other panel has (`panels::PanelClose`); its target is
                    // this panel's root, which `handle_avatar_clicks` watches for.
                    spawn_close_button(row, root);
                });
            // The body scrolls (wheel over the panel, or drag the bar), with the bar beside it.
            panel
                .spawn(Node { flex_direction: FlexDirection::Row, flex_grow: 1.0, flex_shrink: 1.0, min_height: Val::Px(0.0), column_gap: Val::Px(4.0), ..default() })
                .with_children(|frame| {
            frame
                .spawn((
                    TraderDetailScroll,
                    ScrollPosition::default(),
                    Node { flex_direction: FlexDirection::Column, flex_grow: 1.0, flex_shrink: 1.0, min_height: Val::Px(0.0), overflow: Overflow::scroll_y(), ..default() },
                ))
                .with_children(|scroll| {
            // Portrait (a flat version of the voxel figure) beside the text.
            scroll
                .spawn(Node { flex_direction: FlexDirection::Row, column_gap: Val::Px(10.0), align_items: AlignItems::FlexStart, flex_shrink: 0.0, ..default() })
                .with_children(|row| {
                    row.spawn(Node {
                        width: Val::Px(PORTRAIT_W),
                        flex_shrink: 0.0,
                        flex_direction: FlexDirection::Column,
                        align_items: AlignItems::Center,
                        row_gap: Val::Px(2.0),
                        padding: UiRect::top(Val::Px(4.0)),
                        ..default()
                    })
                    .with_children(|fig| {
                        fig.spawn((Node { width: Val::Px(18.0), height: Val::Px(20.0), border_radius: BorderRadius::all(Val::Px(3.0)), ..default() }, BackgroundColor(Color::srgb(0.88, 0.72, 0.60))));
                        fig.spawn((
                            TraderPortraitJacket,
                            Node { width: Val::Px(36.0), height: Val::Px(40.0), border_radius: BorderRadius::all(Val::Px(4.0)), justify_content: JustifyContent::Center, padding: UiRect::top(Val::Px(6.0)), ..default() },
                            BackgroundColor(Color::WHITE),
                        ))
                        .with_children(|body| {
                            body.spawn((Node { width: Val::Px(12.0), height: Val::Px(7.0), border_radius: BorderRadius::all(Val::Px(1.5)), ..default() }, BackgroundColor(Color::srgb(0.95, 0.95, 0.95))));
                        });
                        fig.spawn(Node { flex_direction: FlexDirection::Row, column_gap: Val::Px(4.0), ..default() }).with_children(|legs| {
                            for _ in 0..2 {
                                legs.spawn((Node { width: Val::Px(11.0), height: Val::Px(44.0), border_radius: BorderRadius::all(Val::Px(2.0)), ..default() }, BackgroundColor(Color::srgb_u8(0x1c, 0x1e, 0x26))));
                            }
                        });
                        fig.spawn((TraderPortraitHalo, Node { width: Val::Px(46.0), height: Val::Px(4.0), border_radius: BorderRadius::all(Val::Px(2.0)), ..default() }, BackgroundColor(Color::WHITE)));
                    });
                    row.spawn((
                        TraderDetailBody,
                        Text::new(""),
                        TextFont { font_size: 10.0, ..default() },
                        TextColor(Color::srgb(0.80, 0.84, 0.90)),
                        Node { flex_grow: 1.0, flex_shrink: 1.0, min_width: Val::Px(0.0), max_width: Val::Px(PANEL_W - PORTRAIT_W - 40.0), ..default() },
                    ))
                    .with_children(|text| {
                        for i in 0..DETAIL_LINES {
                            text.spawn((TraderDetailLine(i), TextSpan::new(""), TextFont { font_size: 10.0, ..default() }, TextColor(LineTone::Plain.color())));
                        }
                    });
                });
                });
            // The scrollbar: a track the height of the scrolling area, its thumb placed by
            // `update_trader_detail_scrollbar`. Pressing or dragging on it scrolls there.
            frame
                .spawn((
                    TraderDetailTrack,
                    Button,
                    Node { width: Val::Px(8.0), flex_shrink: 0.0, border_radius: BorderRadius::all(Val::Px(4.0)), ..default() },
                    BackgroundColor(Color::srgba(0.30, 0.36, 0.45, 0.35)),
                ))
                .with_children(|track| {
                    track.spawn((
                        TraderDetailThumb,
                        Node { position_type: PositionType::Absolute, left: Val::Px(0.0), width: Val::Px(8.0), top: Val::Percent(0.0), height: Val::Percent(100.0), border_radius: BorderRadius::all(Val::Px(4.0)), ..default() },
                        BackgroundColor(Color::srgba(0.75, 0.82, 0.92, 0.80)),
                    ));
                });
                });
        });
}

/// Where the scroll goes when the track is pressed at `frac` (0 top, 1 bottom) of its height,
/// for a body `content` tall in a window `view` tall: that point of the body to the middle.
pub fn track_scroll(frac: f32, content: f32, view: f32) -> f32 {
    (frac * content - view * 0.5).clamp(0.0, (content - view).max(0.0))
}

/// System: the wheel over the trader panel scrolls its body (the camera ignores it there),
/// and pressing or dragging on its bar scrolls to that point.
#[allow(clippy::type_complexity)]
pub fn scroll_trader_detail(
    mut wheel: MessageReader<bevy::input::mouse::MouseWheel>,
    windows: Query<&Window>,
    picking: Res<AvatarPicking>,
    mut scroll: Query<(&mut ScrollPosition, &ComputedNode), With<TraderDetailScroll>>,
    track: Query<(&Interaction, &ComputedNode, &bevy::ui::UiGlobalTransform), (With<TraderDetailTrack>, Without<TraderDetailScroll>)>,
    detail: Res<TraderDetail>,
    mut shown: Local<Option<Archetype>>,
) {
    let cursor = windows.single().ok().and_then(|w| w.cursor_position());
    let Ok((mut pos, node)) = scroll.single_mut() else {
        wheel.clear();
        return;
    };
    // Another trader's panel starts at the top.
    if *shown != detail.open {
        *shown = detail.open;
        pos.y = 0.0;
    }
    let k = node.inverse_scale_factor();
    let (view, content) = (node.size().y * k, node.content_size().y * k);
    let max = (content - view).max(0.0);
    let over = matches!((picking.panel, cursor), (Some(r), Some(c)) if r.contains(c));
    for ev in wheel.read() {
        if !over {
            continue;
        }
        let dy = match ev.unit {
            bevy::input::mouse::MouseScrollUnit::Line => ev.y * SCROLL_LINE_PX,
            bevy::input::mouse::MouseScrollUnit::Pixel => ev.y,
        };
        pos.y = (pos.y - dy).clamp(0.0, max);
    }
    if let (Ok((Interaction::Pressed, tn, tt)), Some(c)) = (track.single(), cursor) {
        let tk = tn.inverse_scale_factor();
        let (h, top) = (tn.size().y * tk, tt.translation.y * tk - tn.size().y * tk * 0.5);
        if h > 0.0 {
            pos.y = track_scroll(((c.y - top) / h).clamp(0.0, 1.0), content, view);
        }
    }
}

/// System: the scrollbar's thumb shows how much of the body is in view, and where; the bar
/// hides when it all fits.
pub fn update_trader_detail_scrollbar(
    scroll: Query<(&ScrollPosition, &ComputedNode), With<TraderDetailScroll>>,
    mut track: Query<&mut Node, (With<TraderDetailTrack>, Without<TraderDetailThumb>)>,
    mut thumb: Query<&mut Node, (With<TraderDetailThumb>, Without<TraderDetailTrack>)>,
) {
    let Ok((pos, node)) = scroll.single() else { return };
    let k = node.inverse_scale_factor();
    let (view, content) = (node.size().y * k, node.content_size().y * k);
    let fits = content <= view + 1.0;
    if let Ok(mut t) = track.single_mut() {
        let want = if fits { Display::None } else { Display::Flex };
        if t.display != want {
            t.display = want;
        }
    }
    if fits {
        return;
    }
    if let Ok(mut th) = thumb.single_mut() {
        let h = (view / content * 100.0).clamp(8.0, 100.0);
        let top = (pos.y / content * 100.0).clamp(0.0, 100.0 - h);
        if th.height != Val::Percent(h) || th.top != Val::Percent(top) {
            th.height = Val::Percent(h);
            th.top = Val::Percent(top);
        }
    }
}

/// System: refresh the snapshots from the desk board while it is alive, else from the in-tab
/// book, twice a second.
pub fn update_paper_avatars(time: Res<Time>, feed: Res<LiveFeed>, book: Res<PaperBook>, desk: Res<DeskFeed>, mut avatars: ResMut<PaperAvatars>) {
    let now = time.elapsed_secs_f64();
    if now < avatars.next_refresh {
        return;
    }
    avatars.next_refresh = now + REFRESH_SECS;
    let (snaps, source) = match desk.live_board(now) {
        Some(board) => (snapshots_from_board(board), Source::Desk),
        None => (snapshots_from_book(&book, &feed, wall_clock_ms()), Source::InTab),
    };
    if snaps != avatars.snapshots || source != avatars.source {
        if source != avatars.source {
            info!("paper avatars: reading the {:?} now", source);
        }
        avatars.snapshots = snaps;
        avatars.source = source;
        avatars.revision += 1;
    }
}

/// System: give each genome on the board a body from the slot pool, keep it while the genome
/// stays, and free the slot when it retires. A newly taken slot is repainted in the genome's
/// colour and its label follows it.
#[allow(clippy::type_complexity)]
pub fn assign_genome_avatars(
    avatars: Res<PaperAvatars>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut roots: Query<(&AvatarSlot, &mut PaperAvatar, &AvatarPaint)>,
    mut labels: Query<(&AvatarSlot, &mut AvatarLabel), Without<PaperAvatar>>,
) {
    if !avatars.is_changed() {
        return;
    }
    let genomes: Vec<Archetype> = avatars.snapshots.iter().map(|s| s.arch).filter(|a| matches!(a, Archetype::Genome(_))).collect();
    let mut seated: Vec<Archetype> = Vec::new();
    let mut moves: Vec<(usize, Archetype)> = Vec::new();
    // Keep the ones still on the board; free the rest.
    for (slot, mut a, _) in roots.iter_mut() {
        if genomes.contains(&a.arch) {
            seated.push(a.arch);
        } else if a.arch != slot_placeholder(slot.0) {
            a.arch = slot_placeholder(slot.0);
            moves.push((slot.0, a.arch));
        }
    }
    // New genomes take free slots, lowest first, so a reload seats them in the same bodies.
    let mut waiting = genomes.into_iter().filter(|g| !seated.contains(g));
    let mut free: Vec<(usize, Mut<PaperAvatar>, &AvatarPaint)> = roots.iter_mut().filter(|(s, a, _)| a.arch == slot_placeholder(s.0)).map(|(s, a, p)| (s.0, a, p)).collect();
    free.sort_by_key(|(k, _, _)| *k);
    for (k, mut a, p) in free {
        let Some(g) = waiting.next() else { break };
        a.arch = g;
        paint(&mut materials, p, jacket_color(g));
        moves.push((k, g));
        info!("paper avatars: {} takes genome slot {k}", g.name());
    }
    for (slot, mut label) in labels.iter_mut() {
        if let Some((_, g)) = moves.iter().find(|(k, _)| *k == slot.0) {
            label.0 = *g;
        }
    }
}

/// The closest two standing paper traders get, centre to centre (a body is about 4 wide).
pub const MIN_SEPARATION: f32 = 6.0;

/// How far each of two standing traders steps to stop sharing a spot: half the overlap each,
/// apart along the line between them (a fixed sideways direction if they are exactly on top
/// of one another), so neither stands inside the other.
pub fn separation_push(a: Vec2, b: Vec2, tie: usize) -> Option<Vec2> {
    let d = a.distance(b);
    if d >= MIN_SEPARATION {
        return None;
    }
    let dir = if d > 1e-3 { (a - b) / d } else { Vec2::from_angle(tie as f32 * 2.399) };
    Some(dir * (MIN_SEPARATION - d) * 0.5)
}

/// System: standing paper traders that have ended up on the same spot (two given the same
/// place at a pit, or the same wander spot) step apart, a little each frame. A step that would
/// take one off the concourse into a pit, or out of the pit it stands in, is not taken.
pub fn separate_paper_avatars(time: Res<Time>, mut roots: Query<(&PaperAvatar, &mut AvatarMotion, &Visibility)>) {
    let k = (time.delta_secs() * 4.0).min(1.0);
    let standing: Vec<(usize, Vec2)> = roots.iter().filter(|(_, m, v)| **v != Visibility::Hidden && !m.is_walking()).map(|(a, m, _)| (a.index, m.pos)).collect();
    let mut push: std::collections::HashMap<usize, Vec2> = std::collections::HashMap::new();
    for (x, &(ia, pa)) in standing.iter().enumerate() {
        for &(ib, pb) in &standing[x + 1..] {
            if let Some(p) = separation_push(pa, pb, ia.min(ib)) {
                *push.entry(ia).or_default() += p;
                *push.entry(ib).or_default() -= p;
            }
        }
    }
    for (a, mut m, _) in roots.iter_mut() {
        let Some(p) = push.get(&a.index) else { continue };
        let to = m.pos + *p * k;
        if is_free(to) == is_free(m.pos) {
            m.pos = to;
        }
    }
}


/// System: show only the avatars of traders the live source is running.
pub fn show_seated_avatars(avatars: Res<PaperAvatars>, mut roots: Query<(&PaperAvatar, &mut Visibility)>) {
    if !avatars.is_changed() {
        return;
    }
    for (a, mut vis) in roots.iter_mut() {
        let seated = match avatars.snapshot(a.arch) {
            Some(s) => s.on_desk,
            None => !matches!(a.arch, Archetype::Genome(_)),
        };
        let want = if seated { Visibility::Inherited } else { Visibility::Hidden };
        if *vis != want {
            *vis = want;
        }
    }
}

/// System: walk each avatar toward its goal and animate the figure. Runs every frame.
#[allow(clippy::type_complexity)]
pub fn move_paper_avatars(
    time: Res<Time>,
    view_mode: Res<FloorViewMode>,
    avatars: Res<PaperAvatars>,
    mats: Res<AvatarMaterials>,
    mut roots: Query<(&PaperAvatar, &mut AvatarMotion, &mut Transform, &Children), Without<AvatarPart>>,
    mut parts: Query<(&AvatarPart, &mut Transform, &mut MeshMaterial3d<StandardMaterial>), Without<PaperAvatar>>,
) {
    if !view_mode.is_3d() {
        return;
    }
    let dt = time.delta_secs().min(0.1);
    let t = time.elapsed_secs();
    for (avatar, mut m, mut tf, children) in roots.iter_mut() {
        let (goal, rank) = avatars.goal_and_rank(avatar.arch);
        if goal != m.goal {
            m.goal = goal;
            let path = match goal {
                // In a trade: to the rail, then down onto the pit's top step, facing the well.
                Goal::AtRail(inst) => {
                    let (rail, _) = rail_stand(inst, rank);
                    let (stand, face) = pit_stand(inst, rank);
                    m.face = Some(face);
                    let mut wps = route(m.pos, rail);
                    wps.push(stand);
                    Some(wps)
                }
                Goal::Visit(inst) => {
                    let (stand, face) = rail_stand(inst, rank);
                    m.face = Some(face);
                    Some(route(m.pos, stand))
                }
                Goal::Wander => {
                    m.face = None;
                    None
                }
            };
            if let Some(wps) = path {
                m.waypoints = wps;
                m.arrived_at = None;
                info!("paper avatars: {} heads for {:?} (rank {rank})", avatar.arch.name(), goal);
            } else {
                m.next_wander_at = t + 2.0;
            }
        }
        // A wanderer picks a new spot after a dwell.
        if m.goal == Goal::Wander && !m.is_walking() && t >= m.next_wander_at {
            let spots = wander_spots();
            let pick = spots[m.rng.range_usize(spots.len())];
            if pick.distance(m.pos) > 10.0 {
                m.waypoints = route(m.pos, pick);
                m.arrived_at = None;
            }
            m.next_wander_at = t + 6.0 + m.rng.unit() * 12.0;
        }

        // Walk.
        let mut moving = false;
        if let Some(&wp) = m.waypoints.first() {
            let to = wp - m.pos;
            let dist = to.length();
            let step = WALK_SPEED * dt;
            if dist <= step.max(0.6) {
                m.pos = wp;
                m.waypoints.remove(0);
                if m.waypoints.is_empty() {
                    m.arrived_at = Some(t);
                    if m.face.is_none() {
                        let a = m.rng.unit() * std::f32::consts::TAU;
                        m.face = Some(m.pos + Vec2::new(a.cos(), a.sin()) * 10.0);
                    }
                }
            } else {
                m.pos += to / dist * step;
                let want = yaw_toward(to);
                m.yaw += wrap_angle(want - m.yaw) * (dt * 6.0).min(1.0);
                moving = true;
            }
        }
        if !moving {
            if let Some(face) = m.face {
                let want = yaw_toward(face - m.pos);
                m.yaw += wrap_angle(want - m.yaw) * (dt * 3.0).min(1.0);
            }
        }
        m.walk_phase = if moving { m.walk_phase + dt * 9.0 } else { 0.0 };

        // Arrival shuffle: a step to each side, then stand with a slow sway.
        let shuffle = match m.arrived_at {
            Some(at) if t - at < SHUFFLE_SECS => ((t - at) * 4.0).sin() * 1.2 * (1.0 - (t - at) / SHUFFLE_SECS),
            Some(_) => (t * 0.8 + avatar.index as f32).sin() * 0.25,
            None => 0.0,
        };
        let side = Vec2::new(m.yaw.cos(), -m.yaw.sin());
        let p = m.pos + side * shuffle;
        let bob = if moving { m.walk_phase.sin().abs() * 0.35 } else { 0.0 };
        tf.translation = world_of(p) + Vec3::new(0.0, bob, 0.0);
        tf.rotation = Quat::from_rotation_y(m.yaw);

        // Limbs: swing while walking; hands up on the side of the trade at the rail.
        let open_side = match (m.goal, avatars.snapshot(avatar.arch).and_then(|s| s.open.as_ref())) {
            (Goal::AtRail(_), Some(o)) => Some(o.is_long()),
            _ => None,
        };
        let swing = if moving { m.walk_phase.sin() * 0.45 } else { 0.0 };
        for child in children.iter() {
            let Ok((part, mut pt, mut pm)) = parts.get_mut(child) else { continue };
            match part {
                AvatarPart::LegLeft => pt.rotation = Quat::from_rotation_x(swing),
                AvatarPart::LegRight => pt.rotation = Quat::from_rotation_x(-swing),
                AvatarPart::Body => pt.rotation = pt.rotation.slerp(Quat::from_rotation_x(if open_side.is_some() { 0.10 } else { 0.0 }), (dt * 5.0).min(1.0)),
                AvatarPart::Head => {}
                AvatarPart::ArmLeft | AvatarPart::ArmRight => {
                    // The sleeve's far end is where the hand is: the hand eases toward its target
                    // at the same rate, so the end eases toward the same target.
                    let left = *part == AvatarPart::ArmLeft;
                    let x = if left { -SHOULDER_X } else { SHOULDER_X };
                    let shoulder = Vec3::new(x, SHOULDER_Y, 0.0);
                    let end_now = pt.translation * 2.0 - shoulder;
                    let want = hand_target(left, open_side.is_some(), t, swing) + Vec3::Y * 0.6;
                    let end = end_now.lerp(want, (dt * 10.0).min(1.0));
                    *pt = sleeve_between(shoulder, end);
                }
                AvatarPart::Marker => {
                    let bob = (t * 1.6 + avatar.index as f32).sin() * 0.3;
                    pt.translation.y = HEAD_TOP + 1.9 + bob;
                    pt.rotation = Quat::from_rotation_y(t * 1.2) * Quat::from_rotation_x(std::f32::consts::FRAC_PI_4) * Quat::from_rotation_z(std::f32::consts::FRAC_PI_4);
                }
                AvatarPart::HandLeft | AvatarPart::HandRight => {
                    let left = *part == AvatarPart::HandLeft;
                    let target = hand_target(left, open_side.is_some(), t, swing);
                    pt.translation = pt.translation.lerp(target, (dt * 10.0).min(1.0));
                    let want = match open_side {
                        Some(true) => &mats.hand_long,
                        Some(false) => &mats.hand_short,
                        None => &mats.hand_idle,
                    };
                    if pm.0 != *want {
                        pm.0 = want.clone();
                    }
                }
            }
        }
    }
}

/// One row of `window.__opencryAvatars`: the avatar's screen position, whether it holds a
/// position, and the pit it is heading for or standing at with that pit's projected centre
/// (so a script can click the floor there to focus it).
pub struct HookEntry {
    pub name: &'static str,
    pub x: f32,
    pub y: f32,
    pub open: bool,
    pub pit: Option<(&'static str, Vec2)>,
}

#[cfg(target_arch = "wasm32")]
fn publish_hook(entries: &[HookEntry]) {
    use wasm_bindgen::JsValue;
    let Some(win) = web_sys::window() else { return };
    let arr = js_sys::Array::new();
    for e in entries {
        let o = js_sys::Object::new();
        let _ = js_sys::Reflect::set(&o, &JsValue::from_str("name"), &JsValue::from_str(e.name));
        let _ = js_sys::Reflect::set(&o, &JsValue::from_str("x"), &JsValue::from_f64(e.x as f64));
        let _ = js_sys::Reflect::set(&o, &JsValue::from_str("y"), &JsValue::from_f64(e.y as f64));
        let _ = js_sys::Reflect::set(&o, &JsValue::from_str("open"), &JsValue::from_bool(e.open));
        if let Some((pit, at)) = e.pit {
            let _ = js_sys::Reflect::set(&o, &JsValue::from_str("pit"), &JsValue::from_str(pit));
            let _ = js_sys::Reflect::set(&o, &JsValue::from_str("pit_x"), &JsValue::from_f64(at.x as f64));
            let _ = js_sys::Reflect::set(&o, &JsValue::from_str("pit_y"), &JsValue::from_f64(at.y as f64));
        }
        arr.push(&o);
    }
    let _ = js_sys::Reflect::set(&win, &JsValue::from_str("__opencryAvatars"), &arr);
}

#[cfg(not(target_arch = "wasm32"))]
fn publish_hook(_entries: &[HookEntry]) {}

/// `window.__opencryTraderPanel = {left, top, right, bottom}` while the panel is open, else
/// null, so a script can find its X (top-right corner).
#[cfg(target_arch = "wasm32")]
fn publish_panel(rect: Option<Rect>) {
    use wasm_bindgen::JsValue;
    let Some(win) = web_sys::window() else { return };
    let v = match rect {
        Some(r) => {
            let o = js_sys::Object::new();
            for (k, x) in [("left", r.min.x), ("top", r.min.y), ("right", r.max.x), ("bottom", r.max.y)] {
                let _ = js_sys::Reflect::set(&o, &JsValue::from_str(k), &JsValue::from_f64(x as f64));
            }
            JsValue::from(o)
        }
        None => JsValue::NULL,
    };
    let _ = js_sys::Reflect::set(&win, &JsValue::from_str("__opencryTraderPanel"), &v);
}

#[cfg(not(target_arch = "wasm32"))]
fn publish_panel(_rect: Option<Rect>) {}

/// System: project each avatar's head to the screen and park its label there; record the
/// screen rectangles for picking. After the camera orbit and the walk, same frame.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub fn project_avatar_labels(
    time: Res<Time>,
    view_mode: Res<FloorViewMode>,
    avatars: Res<PaperAvatars>,
    camera: Query<(&Camera, &Transform, &OrbitCamera), (With<PitCamera3d>, Without<PaperAvatar>)>,
    roots: Query<(&PaperAvatar, &Transform, &AvatarMotion), (Without<PitCamera3d>, Without<AvatarLabel>)>,
    mut labels: Query<(&AvatarLabel, &mut Node, &mut Visibility, &Children), Without<AvatarLabelLine>>,
    mut lines: Query<(&AvatarLabelLine, &mut Text, &mut TextColor, &mut Node), Without<AvatarLabel>>,
    mut picking: ResMut<AvatarPicking>,
    mut hook_at: Local<f32>,
) {
    let cam = camera.single().ok().filter(|_| view_mode.is_3d());
    let Some((cam, cam_local, orbit)) = cam else {
        for (_, _, mut vis, _) in labels.iter_mut() {
            *vis = Visibility::Hidden;
        }
        picking.hits.clear();
        return;
    };
    // The camera is a root entity: this frame's `Transform` is what renders (we run after
    // the orbit); its `GlobalTransform` is last frame's.
    let cam_tf = GlobalTransform::from(*cam_local);
    let viewport = cam.logical_viewport_size().unwrap_or(Vec2::new(1280.0, 800.0));
    let scale = if orbit.market_view { 1.0 } else { 1.15 };
    let mut hits = Vec::with_capacity(Archetype::ALL.len());
    let mut hook: Vec<HookEntry> = Vec::new();
    for (label, mut node, mut vis, children) in labels.iter_mut() {
        let Some((_, tf, motion)) = roots.iter().find(|(a, _, _)| a.arch == label.0) else { continue };
        let seated = match avatars.snapshot(label.0) {
            Some(s) => s.on_desk,
            None => !matches!(label.0, Archetype::Genome(_)),
        };
        if !seated {
            *vis = Visibility::Hidden;
            continue;
        }
        let feet = tf.translation;
        let head = feet + Vec3::new(0.0, HEAD_TOP + 3.4, 0.0); // above the marker
        let (Ok(vp_head), Ok(vp_feet)) = (cam.world_to_viewport(&cam_tf, head), cam.world_to_viewport(&cam_tf, feet)) else {
            *vis = Visibility::Hidden;
            continue;
        };
        let on_screen = vp_head.x > -LABEL_W && vp_head.x < viewport.x + LABEL_W && vp_head.y > -LABEL_H && vp_feet.y < viewport.y + 40.0;
        if !on_screen {
            *vis = Visibility::Hidden;
            continue;
        }
        let snap = avatars.snapshot(label.0);
        let (day_r, day_trades) = snap.map(|s| s.headline()).unwrap_or((0.0, 0));
        let open = snap.and_then(|s| s.open.as_ref());
        let top = vp_head.y - LABEL_H;
        let left = vp_head.x - LABEL_W * 0.5;
        if node.left != Val::Px(left) {
            node.left = Val::Px(left);
        }
        if node.top != Val::Px(top) {
            node.top = Val::Px(top);
        }
        *vis = Visibility::Inherited;
        for child in children.iter() {
            let Ok((line, mut text, mut color, mut lnode)) = lines.get_mut(child) else { continue };
            match line.0 {
                0 => {
                    set_ui_text(&mut text, label_text(label.0.name(), day_r, day_trades));
                    let want = tone_for(day_r, day_trades).color();
                    if color.0 != want {
                        color.0 = want;
                    }
                }
                _ => match open {
                    Some(o) => {
                        set_ui_text(&mut text, open_line(o));
                        let want = if o.is_long() { Tone::Up.color() } else { Tone::Down.color() };
                        if color.0 != want {
                            color.0 = want;
                        }
                        if lnode.display != Display::Flex {
                            lnode.display = Display::Flex;
                        }
                    }
                    None => {
                        if lnode.display != Display::None {
                            lnode.display = Display::None;
                        }
                    }
                },
            }
        }
        // Hit rectangle: the figure from feet to label top, as wide as it is tall (at least
        // a thumb's width), scaled up a little in pit focus where the figure is big.
        let height = (vp_feet.y - top).max(24.0);
        let half_w = (height * 0.28 * scale).clamp(14.0, LABEL_W * 0.5);
        let centre_x = (vp_head.x + vp_feet.x) * 0.5;
        hits.push((label.0, Rect::new(centre_x - half_w, top, centre_x + half_w, vp_feet.y)));
        let pit = motion.goal.pit().and_then(|inst| {
            let at = cam.world_to_viewport(&cam_tf, pit_3d_origin(inst) + Vec3::new(0.0, 2.0, 0.0)).ok()?;
            Some((inst.symbol(), at))
        });
        hook.push(HookEntry { name: label.0.name(), x: centre_x, y: (vp_head.y + vp_feet.y) * 0.5, open: open.is_some(), pit });
    }
    picking.hits = hits;
    let now = time.elapsed_secs();
    if now - *hook_at >= 0.25 {
        *hook_at = now;
        publish_hook(&hook);
    }
}

/// System: clicks from the picker open / swap the panel; the X and Escape close it.
pub fn handle_avatar_clicks(
    keyboard: Res<ButtonInput<KeyCode>>,
    mut picking: ResMut<AvatarPicking>,
    mut detail: ResMut<TraderDetail>,
    mut book: ResMut<PaperBook>,
    close: Query<(&Interaction, &PanelClose), Changed<Interaction>>,
    root: Query<Entity, With<TraderDetailPanel>>,
) {
    // A row of the Traders panel opens the same panel as a click on the figure.
    if let Some(arch) = book.panel_request.take() {
        picking.clicked = Some(arch);
    }
    if let Some(arch) = picking.clicked.take() {
        if detail.open != Some(arch) {
            info!("paper avatars: {} clicked, detail panel open", arch.name());
        }
        detail.open = Some(arch);
    }
    let root = root.single().ok();
    let close_pressed = close.iter().any(|(i, c)| *i == Interaction::Pressed && Some(c.target) == root);
    if detail.open.is_some() && (close_pressed || keyboard.just_pressed(KeyCode::Escape)) {
        detail.open = None;
    }
}

/// System: a click on a row of the Traders panel flies the 3D camera to that trader's avatar,
/// and the camera keeps it framed while it walks, until the panel closes, another trader is
/// opened, or the camera is sent to a pit or the floor. A trader with no body on the floor
/// (not seated, or beyond the genome slots) only opens the panel.
pub fn follow_clicked_trader(
    mut book: ResMut<PaperBook>,
    detail: Res<TraderDetail>,
    avatars: Query<(&PaperAvatar, &GlobalTransform, &Visibility)>,
    mut camera: Query<&mut crate::camera::OrbitCamera, With<PitCamera3d>>,
) {
    let Ok(mut orbit) = camera.single_mut() else { return };
    let body = |arch: Archetype| avatars.iter().find(|(a, _, v)| a.arch == arch && **v != Visibility::Hidden).map(|(_, t, _)| t);
    let at = |arch: Archetype| body(arch).map(|t| t.translation());
    if let Some(arch) = book.fly_to.take() {
        match body(arch) {
            Some(t) => orbit.follow_trader(arch, t.translation(), t.rotation().to_euler(EulerRot::YXZ).0),
            None => info!("paper avatars: {} has no body on the floor to fly to", arch.name()),
        }
        return;
    }
    let Some(arch) = orbit.follow else { return };
    match (detail.open == Some(arch)).then(|| at(arch)).flatten() {
        Some(p) => orbit.goal_target = p + Vec3::new(0.0, 10.0, 0.0),
        None => orbit.follow = None,
    }
}

/// The least room the trader panel gets: if the list leaves less above the buttons, the
/// panel moves up over the list's bottom rows instead.
const DETAIL_MIN_H: f32 = 240.0;

/// Where the trader panel goes: (top edge, most height). Under the Traders list if it is
/// showing, else at the top of the column; it stops above the bottom buttons, cutting off its
/// last lines if it has more than fits, and only moves up over the list when the list leaves
/// it less than `DETAIL_MIN_H`.
pub fn detail_panel_place(below_list: Option<f32>, viewport_h: f32) -> (f32, f32) {
    let floor = viewport_h - PANEL_BOTTOM;
    let top = below_list.unwrap_or(crate::panels::COLUMN_TOP).min(floor - DETAIL_MIN_H).max(crate::panels::COLUMN_TOP);
    (top, (floor - top).max(60.0))
}

/// System: the detail panel's contents and its screen rectangle, four times a second.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub fn update_trader_detail_panel(
    time: Res<Time>,
    feed: Res<LiveFeed>,
    desk: Res<DeskFeed>,
    jev: Res<JevDesk>,
    avatars: Res<PaperAvatars>,
    detail: Res<TraderDetail>,
    camera: Query<&Camera, With<PitCamera3d>>,
    mut picking: ResMut<AvatarPicking>,
    mut next: Local<f32>,
    mut panel: Query<(&mut Node, &mut BorderColor, &ComputedNode), With<TraderDetailPanel>>,
    traders: Query<(&Node, &ComputedNode, &bevy::ui::UiGlobalTransform), (With<crate::paper::PaperPanel>, Without<TraderDetailPanel>)>,
    mut title: Query<(&mut Text, &mut TextColor), (With<TraderDetailTitle>, Without<TraderDetailBody>)>,
    body: Query<&mut Text, (With<TraderDetailBody>, Without<TraderDetailTitle>)>,
    mut spans: Query<(&TraderDetailLine, &mut TextSpan, &mut TextColor), (Without<TraderDetailTitle>, Without<TraderDetailBody>)>,
    mut portrait: Query<(&mut BackgroundColor, Has<TraderPortraitHalo>), Or<(With<TraderPortraitJacket>, With<TraderPortraitHalo>)>>,
) {
    let Ok((mut node, mut border, computed)) = panel.single_mut() else { return };
    let Some(arch) = detail.open else {
        if node.display != Display::None {
            node.display = Display::None;
            publish_panel(None);
        }
        picking.panel = None;
        return;
    };
    if node.display != Display::Flex {
        node.display = Display::Flex;
    }
    // Directly under the Traders list (over the panels below it), as tall as its content;
    // pushed up if it would run into the bottom buttons; at the top of the column when the
    // list is hidden.
    let viewport_h = camera.single().ok().and_then(|c| c.logical_viewport_size()).map(|s| s.y).unwrap_or(800.0);
    let below_list = traders.single().ok().filter(|(n, c, _)| n.display != Display::None && c.size().y > 0.0).map(|(_, c, t)| {
        (t.translation.y + c.size().y * 0.5) * c.inverse_scale_factor() + crate::panels::COLUMN_GAP
    });
    let (top, max_h) = detail_panel_place(below_list, viewport_h);
    if node.top != Val::Px(top) || node.max_height != Val::Px(max_h) {
        node.top = Val::Px(top);
        node.max_height = Val::Px(max_h);
    }
    let height = (computed.size().y * computed.inverse_scale_factor()).clamp(60.0, max_h);
    picking.panel = Some(Rect::new(PANEL_LEFT, top, PANEL_LEFT + PANEL_W, top + height));

    let now = time.elapsed_secs();
    if now < *next && !detail.is_changed() && !avatars.is_changed() {
        return;
    }
    *next = now + 0.25;
    publish_panel(picking.panel);
    let Some(snap) = avatars.snapshot(arch) else { return };
    let colour = jacket_color(arch);
    *border = BorderColor::all(colour.with_alpha(0.75));
    if let Ok((mut t, mut c)) = title.single_mut() {
        set_ui_text(&mut t, format!("{}  ({})  |  {}", arch.name().to_uppercase(), arch.label(), label_text(arch.name(), snap.headline().0, snap.headline().1).split("  ").last().unwrap_or("")));
        if c.0 != colour {
            c.0 = colour;
        }
    }
    let mark = snap.open.as_ref().and_then(|o| {
        let q = feed.quote(o.inst?)?;
        Some(if o.is_long() { q.price as f32 } else { q.ask as f32 })
    });
    for (mut bg, is_halo) in portrait.iter_mut() {
        let want = if is_halo { arch.color() } else { colour };
        if bg.0 != want {
            bg.0 = want;
        }
    }
    let now_s = time.elapsed_secs_f64();
    let board = desk.live_board(now_s);
    let heartbeat = board.and_then(|b| b.heartbeat_age_ms).map(|ms| format!("heartbeat {}s ago", ms / 1000));
    // Outlook: Jev's latest read of the pit this trader is in (or last looked at), and what
    // the trader made of it.
    let mut outlook = Vec::new();
    if let Some(pit) = snap.open_pit().or(snap.last_pit) {
        let read = match (snap.source, board) {
            (Source::Desk, Some(b)) => b.jev_read(pit.symbol()).map(|r| format!("{}  |  {:.0} s ago", r.headline, r.age_secs)),
            _ => jev.read(pit).map(|r| format!("{}  |  {:.0} s ago", r.headline(), (now_s - r.at).max(0.0))),
        };
        outlook.push(format!("Outlook on {}: {}", pit.symbol(), read.unwrap_or_else(|| "no Jev read of this pit yet".into())));
        let take = snap
            .open
            .as_ref()
            .filter(|o| o.inst == Some(pit))
            .map(|o| format!("in because: {}", o.why))
            .or_else(|| snap.waiting.iter().find(|w| w.symbol == pit.symbol()).map(|w| format!("out because: {}", w.why)));
        if let Some(t) = take {
            outlook.push(format!("  {}'s take: {}", arch.name(), t));
        }
    }
    let toned = detail_lines_toned(snap, mark, heartbeat, &outlook);
    let n = toned.len().min(DETAIL_LINES);
    for (line, mut span, mut color) in spans.iter_mut() {
        let (text, tone) = match toned.get(line.0).filter(|_| line.0 < n) {
            Some((t, tone)) => (if line.0 + 1 < n { format!("{t}\n") } else { t.clone() }, *tone),
            None => (String::new(), LineTone::Plain),
        };
        if span.0 != text {
            span.0 = text;
        }
        let c = tone.color();
        if color.0 != c {
            color.0 = c;
        }
    }
    let _ = body;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_traders_panel_click_flies_the_camera_to_the_avatar_and_follows_it() {
        use bevy::ecs::system::RunSystemOnce;
        let mut world = World::new();
        let arch = Archetype::Control;
        let mut book = PaperBook::default();
        book.fly_to = Some(arch);
        world.insert_resource(book);
        world.insert_resource(TraderDetail { open: Some(arch) });
        let body = world.spawn((PaperAvatar { arch, index: 0 }, GlobalTransform::from_translation(Vec3::new(50.0, 0.0, 20.0)), Visibility::Inherited)).id();
        let cam = world.spawn((crate::camera::OrbitCamera::default(), PitCamera3d)).id();
        world.run_system_once(follow_clicked_trader).unwrap();
        let o = world.get::<crate::camera::OrbitCamera>(cam).unwrap().clone();
        assert_eq!(o.follow, Some(arch));
        assert_eq!((o.goal_target, o.goal_radius), (Vec3::new(50.0, 10.0, 20.0), crate::camera::FOLLOW_RADIUS));
        // It walks: the camera keeps it framed.
        *world.get_mut::<GlobalTransform>(body).unwrap() = GlobalTransform::from_translation(Vec3::new(60.0, 0.0, 25.0));
        world.run_system_once(follow_clicked_trader).unwrap();
        assert_eq!(world.get::<crate::camera::OrbitCamera>(cam).unwrap().goal_target, Vec3::new(60.0, 10.0, 25.0));
        // The panel closes: the camera stays put and stops following.
        world.resource_mut::<TraderDetail>().open = None;
        world.run_system_once(follow_clicked_trader).unwrap();
        let o = world.get::<crate::camera::OrbitCamera>(cam).unwrap();
        assert_eq!((o.follow, o.goal_target), (None, Vec3::new(60.0, 10.0, 25.0)));
        // Sending the camera to a pit ends a follow too.
        let mut o = o.clone();
        o.follow = Some(arch);
        o.focus_pit(Instrument::Gold);
        assert_eq!(o.follow, None);
    }
    use crate::desk_feed::parse_board;

    #[test]
    fn a_genome_on_the_board_gets_a_snapshot_and_a_dropped_trader_is_off_the_desk() {
        use crate::desk_feed::{DeskPit, DeskSeat};
        let board = DeskBoard {
            traders: vec![
                DeskTrader { trader: "Gus".into(), archetype: "contrarian".into(), trades: 2, day_trades: 2, ..Default::default() },
                DeskTrader { trader: "Lena".into(), archetype: "absorber".into(), ..Default::default() },
                DeskTrader { trader: "Testgenome".into(), archetype: "genome".into(), day_trades: 3, day_r: 1.0, trades: 5, net_r: 2.5, ..Default::default() },
                DeskTrader { trader: "Vegatest".into(), archetype: "holder".into(), ..Default::default() },
            ],
            pit: Some(DeskPit { seats: vec![DeskSeat { id: "gtest1".into(), name: "Testgenome".into(), creed: "15m follow a test".into(), ..Default::default() }], ..Default::default() }),
            ..Default::default()
        };
        let snaps = snapshots_from_board(&board);
        assert_eq!(snaps.len(), Archetype::ALL.len() + 1, "the idle holder has no body until it trades");
        assert_eq!(board.ranking().iter().map(|&t| board.traders[t].trader.as_str()).collect::<Vec<_>>(), vec!["Testgenome", "Gus"], "idle traders are not listed");
        assert!(board.header_line().contains("2 more waiting for a signal"), "{}", board.header_line());
        assert!(!snaps.iter().find(|s| s.arch.name() == "Lena").unwrap().on_desk, "an idle built-in does not walk the floor");
        assert!(board.traders.iter().all(|t| t.archetype().is_some() || t.is_genome()), "the page would call itself out of date");
        let g = snaps.iter().find(|s| s.arch.name() == "Testgenome").unwrap();
        assert!(matches!(g.arch, Archetype::Genome(_)));
        assert_eq!(g.arch.name(), "Testgenome");
        assert_eq!(g.arch.describe(), "15m follow a test");
        assert_eq!(g.day.trades, 3);
        assert_eq!(g.headline(), (1.0, 3), "over the figure: the table's numbers, today's");
        assert_eq!(genome_arch(&board, "Testgenome"), g.arch, "the same genome is the same archetype every refresh");
        let colin = snaps.iter().find(|s| s.arch == Archetype::Control).unwrap();
        assert!(!colin.on_desk, "Colin is not on this board");
        assert!(snaps.iter().find(|s| s.arch.name() == "Gus").unwrap().on_desk);
    }

    #[test]
    fn past_trades_the_open_position_and_the_day_are_coloured_by_their_r() {
        let mut s = TraderSnapshot::empty(Archetype::Control, Source::Desk);
        s.day = DayStats { trades: 2, wins: 1, losses: 1, net_r: -0.4, ..Default::default() };
        s.last_trades = vec![
            DeskTrade { symbol: "US500".into(), side: "long".into(), pnl_r: 1.2, why_out: "target".into(), ..Default::default() },
            DeskTrade { symbol: "US500".into(), side: "short".into(), pnl_r: -1.6, why_out: "stop".into(), ..Default::default() },
        ];
        let toned = detail_lines_toned(&s, None, None, &[]);
        let tone = |p: &str| toned.iter().find(|(l, _)| l.starts_with(p)).map(|x| x.1);
        assert_eq!(tone("Today (since"), Some(LineTone::Pnl(-0.4)));
        assert_eq!(tone("Last 2 trades"), Some(LineTone::Head));
        let h = toned.iter().position(|(l, _)| l.starts_with("Last 2 trades")).unwrap();
        assert_eq!((toned[h + 1].1, toned[h + 2].1), (LineTone::Pnl(1.2), LineTone::Pnl(-1.6)), "each trade by its own R");
        assert_eq!(toned[h + 3].1, LineTone::Plain, "the source line after them is plain");
        assert_ne!(LineTone::Pnl(1.2).color(), LineTone::Pnl(-1.6).color());
        assert_eq!(detail_lines(&s, None, None, &[]), toned.into_iter().map(|x| x.0).collect::<Vec<_>>(), "same lines, only coloured");
    }

    #[test]
    fn pressing_the_scrollbar_brings_that_part_of_the_body_into_view() {
        assert_eq!(track_scroll(0.0, 1000.0, 300.0), 0.0);
        assert_eq!(track_scroll(1.0, 1000.0, 300.0), 700.0, "the bottom, never past it");
        assert_eq!(track_scroll(0.5, 1000.0, 300.0), 350.0, "the middle of the body to the middle of the window");
        assert_eq!(track_scroll(0.5, 200.0, 300.0), 0.0, "all of it fits");
    }

    #[test]
    fn two_traders_on_one_spot_step_apart_and_far_ones_stay() {
        let a = Vec2::new(10.0, 0.0);
        assert_eq!(separation_push(a, Vec2::new(30.0, 0.0), 0), None, "far apart: nothing");
        let p = separation_push(a, Vec2::new(12.0, 0.0), 0).unwrap();
        assert!((p - Vec2::new(-2.0, 0.0)).length() < 1e-4, "half the 4-unit overlap each, away from the other: {p:?}");
        let q = separation_push(a, a, 3).unwrap();
        assert!((q.length() - MIN_SEPARATION * 0.5).abs() < 1e-4, "exactly on top of each other: still pushed apart");
    }

    #[test]
    fn the_trader_panel_opens_under_the_list_and_never_on_the_buttons() {
        let floor = 1000.0 - PANEL_BOTTOM;
        assert_eq!(detail_panel_place(Some(340.0), 1000.0), (340.0, floor - 340.0), "right under the list, down to the buttons");
        assert_eq!(detail_panel_place(Some(700.0), 1000.0), (floor - DETAIL_MIN_H, DETAIL_MIN_H), "a long list: up over its last rows, never onto the buttons");
        assert_eq!(detail_panel_place(None, 1000.0).0, crate::panels::COLUMN_TOP, "top of the column with the list hidden");
        assert_eq!(detail_panel_place(Some(400.0), 300.0).0, crate::panels::COLUMN_TOP, "never above the column");
    }

    #[test]
    fn every_body_including_the_genome_slots_starts_on_the_floor() {
        for i in 0..Archetype::ALL.len() + GENOME_SLOTS {
            let p = spawn_spot(i);
            assert!(p.x.abs() <= 196.0 + 1e-3, "spot {i} at {p:?}");
        }
    }

    #[test]
    fn in_a_trade_the_stand_is_inside_the_pit_and_reached_through_the_rail() {
        for k in 0..6 {
            let (stand, face) = pit_stand(Instrument::Gold, k);
            let c = pit_xz(Instrument::Gold);
            assert!((stand.distance(c) - PIT_STAND_RADIUS).abs() < 1e-3);
            assert!(!is_free(stand), "the step is inside the blocked ring");
            assert_eq!(face, c, "faces the well");
            let (rail, _) = rail_stand(Instrument::Gold, k);
            assert!(rail.distance(stand) < 20.0, "a short straight step down from the rail");
            assert!(is_free(rail));
        }
        // Leaving the pit still routes: the path starts from the nearest free cell.
        let (stand, _) = pit_stand(Instrument::Gold, 0);
        let out = route(stand, spawn_spot(3));
        assert!(!out.is_empty() && out.iter().all(|p| is_free(*p)));
    }

    fn trade(symbol: &'static str, closed_ms: i64, pnl_r: f32) -> Trade {
        Trade {
            trader: "Grace",
            archetype: Archetype::TrendRider,
            symbol,
            side: "long",
            entry: 1.0,
            exit: 1.1,
            opened_ms: closed_ms - 60_000,
            closed_ms,
            pnl_ticks: pnl_r * 10.0,
            pnl_gbp: pnl_r * 10.0,
            pnl_r,
            stop_ticks: 10.0,
            spread_ticks_in: 2.0,
            why_in: "why".into(),
            why_out: if pnl_r > 0.0 { "target" } else { "stop" }.into(),
            observation_hash: String::new(),
            read_source: None,
            genome: None,
        }
    }

    const BOARD: &str = r#"{"desk_alive":true,"heartbeat_age_ms":3000,"traders":[
        {"trader":"Grace","archetype":"trend","trades":4,"wins":2,"losses":2,"net_r":1.8,"spread_paid_r":1.0,"max_drawdown_r":1.2,
         "day_r":-0.7,"day_trades":2,"day_wins":1,"day_losses":1,"day_spread_paid_r":0.5,"day_max_drawdown_r":1.0,
         "open":{"symbol":"XAG/USD","side":"long","entry":57.84,"open_r":0.35,"opened_ms":1792619000000,"why":"drift","stop":57.74,"target":58.04},
         "last_trades":[{"symbol":"US30","side":"short","entry":1.0,"exit":0.9,"pnl_r":1.0,"opened_ms":1,"closed_ms":2,"why_in":"a","why_out":"target"}]},
        {"trader":"Colin","archetype":"CONTROL","trades":0,"wins":0,"losses":0,"net_r":0.0,"spread_paid_r":0.0,"max_drawdown_r":0.0,"open":null,
         "last_trades":[{"symbol":"NAS100","side":"long","entry":1.0,"exit":0.9,"pnl_r":-1.0,"opened_ms":1,"closed_ms":2,"why_in":"coin","why_out":"stop"}]}
    ]}"#;

    #[test]
    fn the_goal_is_the_open_pit_then_the_last_pit_then_a_wander() {
        assert_eq!(pick_goal(Some(Instrument::Gold), Some(Instrument::Silver)), Goal::AtRail(Instrument::Gold));
        assert_eq!(pick_goal(None, Some(Instrument::Silver)), Goal::Visit(Instrument::Silver));
        assert_eq!(pick_goal(None, None), Goal::Wander);
        assert_eq!(Goal::Wander.pit(), None);
        assert_eq!(Goal::Visit(Instrument::Us30).pit(), Some(Instrument::Us30));
    }

    #[test]
    fn snapshots_from_a_board_carry_the_day_the_open_pit_and_the_last_pit() {
        let b = parse_board(BOARD).unwrap();
        let snaps = snapshots_from_board(&b);
        assert_eq!(snaps.len(), Archetype::ALL.len(), "every archetype, board or not");
        let grace = snaps.iter().find(|s| s.arch == Archetype::TrendRider).unwrap();
        assert_eq!(grace.source, Source::Desk);
        assert_eq!((grace.day.trades, grace.day.wins, grace.day.losses), (2, 1, 1));
        assert!((grace.day.net_r + 0.7).abs() < 1e-6);
        assert_eq!(grace.open_pit(), Some(Instrument::Silver));
        assert_eq!(grace.last_pit, Some(Instrument::Us30));
        assert_eq!(pick_goal(grace.open_pit(), grace.last_pit), Goal::AtRail(Instrument::Silver), "the open position wins");
        let colin = snaps.iter().find(|s| s.arch == Archetype::Control).unwrap();
        assert_eq!(pick_goal(colin.open_pit(), colin.last_pit), Goal::Visit(Instrument::Nasdaq), "flat: the last trade's pit");
        let priya = snaps.iter().find(|s| s.arch == Archetype::IctLevelTrader).unwrap();
        assert_eq!(pick_goal(priya.open_pit(), priya.last_pit), Goal::Wander, "not on the board at all");
        assert_eq!(priya.day, DayStats::default());

        let mut avatars = PaperAvatars { snapshots: snaps, source: Source::Desk, revision: 1, next_refresh: 0.0 };
        assert_eq!(avatars.goal_and_rank(Archetype::TrendRider), (Goal::AtRail(Instrument::Silver), 0));
        // Two traders at one pit take different stands.
        avatars.snapshots.iter_mut().find(|s| s.arch == Archetype::Control).unwrap().last_pit = Some(Instrument::Silver);
        assert_eq!(avatars.goal_and_rank(Archetype::Control), (Goal::Visit(Instrument::Silver), 1));
        let (a, _) = rail_stand(Instrument::Silver, 0);
        let (b2, _) = rail_stand(Instrument::Silver, 1);
        assert!(a.distance(b2) > 20.0);
    }

    #[test]
    fn the_day_block_counts_only_today_in_time_order_with_drawdown_from_zero() {
        let day_start = 12_000 * 86_400_000; // a real midnight
        let trades = vec![
            trade("XAG/USD", day_start - 1, 5.0), // yesterday
            trade("XAG/USD", day_start + 3000, -0.7),
            trade("US30", day_start + 1000, 1.0),
            trade("US30", day_start + 2000, -0.5),
            trade("NAS100", day_start + 4000, 2.0),
        ];
        let d = day_stats_from_trades(&trades, day_start);
        assert_eq!((d.trades, d.wins, d.losses), (4, 2, 2));
        assert!((d.net_r - 1.8).abs() < 1e-5, "{}", d.net_r);
        assert!((d.spread_paid_r - 0.8).abs() < 1e-5, "0.2 R of spread per trade");
        assert!((d.max_drawdown_r - 1.2).abs() < 1e-5, "peak 1.0 to -0.2 regardless of input order: {}", d.max_drawdown_r);
        assert!((d.win_rate() - 0.5).abs() < 1e-6);
        assert_eq!(day_stats_from_trades(&[], day_start), DayStats::default());
        assert_eq!(day_start_ms(day_start + 5), day_start_ms(day_start + 86_399_999));

        // The in-tab book path reads the same trades.
        let mut book = PaperBook::default();
        book.log_to_relay = false;
        let g = book.traders.iter().position(|t| t.archetype == Archetype::TrendRider).unwrap();
        book.traders[g].trades = trades;
        let feed = LiveFeed::default();
        let snaps = snapshots_from_book(&book, &feed, day_start + 10_000);
        let grace = &snaps[g];
        assert_eq!(grace.source, Source::InTab);
        assert_eq!(grace.day.trades, 4);
        assert_eq!(grace.last_trades.len(), 5);
        assert_eq!(grace.last_trades[0].symbol, "NAS100", "newest first");
        assert_eq!(grace.last_pit, Some(Instrument::Nasdaq));
        assert!(grace.open.is_none());
    }

    #[test]
    fn the_label_reads_name_and_day_r_in_the_right_colour() {
        assert_eq!(label_text("Grace", 1.34, 3), "GRACE  +1.34R");
        assert_eq!(label_text("Vinnie", -0.96, 1), "VINNIE  -0.96R");
        assert_eq!(label_text("Colin", 0.0, 0), "COLIN  0.00R");
        assert_eq!(tone_for(1.3, 3), Tone::Up);
        assert_eq!(tone_for(-0.2, 1), Tone::Down);
        assert_eq!(tone_for(0.0, 0), Tone::Flat);
        assert_eq!(tone_for(0.04, 2), Tone::Flat, "a rounding-to-flat day is grey");
        assert_eq!(tone_for(2.0, 0), Tone::Flat, "no trades means grey whatever the number says");
        let open = OpenPosition {
            inst: Some(Instrument::Silver),
            symbol: "XAG/USD".into(),
            side: "long".into(),
            entry: 57.84,
            stop: 57.74,
            target: Some(58.04),
            open_r: 0.35,
            opened_ms: 1792619000000,
            why: "drift".into(),
        };
        assert_eq!(open_line(&open), "LONG XAG/USD  +0.35R");
        assert_eq!(instrument_for_symbol("XAG/USD"), Some(Instrument::Silver));
        assert_eq!(instrument_for_symbol("XAGUSD"), Some(Instrument::Silver), "the tape's name too");
        assert_eq!(instrument_for_symbol("DOGE"), None);
    }

    #[test]
    fn detail_lines_name_the_rules_the_day_the_open_position_and_the_source() {
        let b = parse_board(BOARD).unwrap();
        let snaps = snapshots_from_board(&b);
        let grace = snaps.iter().find(|s| s.arch == Archetype::TrendRider).unwrap();
        let lines = detail_lines(grace, Some(57.91), Some("heartbeat 3s ago".into()), &["Outlook on XAG/USD: Jev: UP 40%".to_string()]);
        assert!(lines[1].starts_with("Strategy: Needs Jev's lean to agree with the last 20 bars"), "{}", lines[1]);
        assert!(lines[2].starts_with("Rules: enters 08:00-16:30 UTC"), "{}", lines[2]);
        assert!(lines[2].contains("stop >= 20 spreads, target >= 4 spreads") && lines[2].ends_with("Jev: yes"), "{}", lines[2]);
        assert_eq!(lines[3], "Outlook on XAG/USD: Jev: UP 40%", "the outlook follows the rules");
        assert!(lines[4].starts_with("Today (since 00:00 UTC): 2 trades, 50% win, -0.70R net"), "{}", lines[4]);
        assert!(lines[5].starts_with("OPEN: LONG XAG/USD from 57.840  stop 57.740  target 58.040  mark 57.910  +0.35R"), "{}", lines[5]);
        assert!(lines[5].ends_with("since 21:43 UTC"), "{}", lines[5]);
        assert!(lines.iter().any(|l| l.contains("US30    short 1.0 -> 0.9 +1.00R target | a")), "{:?}", lines);
        assert!(lines.iter().all(|l| !l.starts_with(" 00:") || l.chars().count() <= 90), "trade lines fit the panel: {:?}", lines);
        assert!(lines.last().unwrap().contains("DESK") && lines.last().unwrap().ends_with("heartbeat 3s ago"));
        let colin = snaps.iter().find(|s| s.arch == Archetype::Control).unwrap();
        let lines = detail_lines(colin, None, None, &[]);
        assert!(lines.iter().any(|l| l == "Today (since 00:00 UTC): no trades yet"));
        assert!(lines.iter().any(|l| l == "Flat: no open position"));
        assert!(lines[2].ends_with("Jev: no"));
        assert!(lines[1].contains("coin"), "{}", lines[1]);
    }

    #[test]
    fn the_concourse_is_free_the_stands_are_free_and_routes_stay_off_the_pits() {
        for i in 0..Archetype::ALL.len() {
            assert!(is_free(spawn_spot(i)), "spawn spot {i}");
        }
        let spots = wander_spots();
        assert!(spots.len() >= 20, "{}", spots.len());
        assert!(spots.iter().all(|p| is_free(*p)));
        for inst in Instrument::ALL {
            assert!(!is_free(pit_xz(inst)), "a pit centre is not walkable");
            for k in 0..4 {
                let (stand, face) = rail_stand(inst, k);
                assert!(is_free(stand), "{inst:?} stand {k}");
                assert!((stand.distance(pit_xz(inst)) - RAIL_STAND_RADIUS).abs() < 1e-3);
                assert!(face.y < pit_xz(inst).y, "faces the pulpit on the north rim");
            }
        }
        // Every spawn spot reaches every rail stand without crossing a pit.
        for i in 0..Archetype::ALL.len() {
            for inst in Instrument::ALL {
                let (stand, _) = rail_stand(inst, 0);
                let path = route(spawn_spot(i), stand);
                assert!(!path.is_empty(), "{i} -> {inst:?}");
                assert_eq!(*path.last().unwrap(), stand);
                let mut prev = spawn_spot(i);
                for &p in &path {
                    assert!(segment_clear(prev, p), "{i} -> {inst:?} clips a pit between {prev} and {p}");
                    prev = p;
                }
            }
        }
        // Gold's rail to the FTSE rail runs the long way round, never through Nasdaq.
        let path = route(rail_stand(Instrument::Gold, 0).0, rail_stand(Instrument::Uk100, 0).0);
        let mut prev = rail_stand(Instrument::Gold, 0).0;
        for &p in &path {
            assert!(segment_clear(prev, p));
            prev = p;
        }
        let straight = segment_clear(rail_stand(Instrument::Gold, 0).0, rail_stand(Instrument::Uk100, 0).0);
        assert!(!straight, "the direct line crosses the Nasdaq pit");
        assert!(path.len() >= 2, "so the route has at least one bend: {path:?}");
    }

    #[test]
    fn picking_prefers_the_nearest_centre_and_the_open_panel_swallows_clicks() {
        let mut p = AvatarPicking::default();
        p.hits = vec![
            (Archetype::TrendRider, Rect::new(100.0, 100.0, 140.0, 200.0)),
            (Archetype::Control, Rect::new(120.0, 100.0, 160.0, 200.0)),
        ];
        assert_eq!(p.hit_at(Vec2::new(105.0, 150.0)), Some(Archetype::TrendRider));
        assert_eq!(p.hit_at(Vec2::new(155.0, 150.0)), Some(Archetype::Control));
        assert_eq!(p.hit_at(Vec2::new(500.0, 150.0)), None);
        assert!(p.consume_click(Vec2::new(135.0, 150.0)));
        assert_eq!(p.clicked, Some(Archetype::Control), "nearer to Colin's centre");
        assert!(!p.consume_click(Vec2::new(500.0, 500.0)), "a floor click goes on to the pit picker");
        p.panel = Some(Rect::new(8.0, 400.0, 428.0, 660.0));
        p.clicked = None;
        assert!(p.consume_click(Vec2::new(200.0, 500.0)));
        assert_eq!(p.clicked, None, "the panel itself is not an avatar");
    }

    #[test]
    fn yaw_faces_the_target_and_angles_wrap() {
        assert!((yaw_toward(Vec2::new(0.0, 1.0))).abs() < 1e-6, "front is +Z");
        assert!((yaw_toward(Vec2::new(1.0, 0.0)) - std::f32::consts::FRAC_PI_2).abs() < 1e-6);
        assert!((wrap_angle(2.5 * std::f32::consts::PI) - 0.5 * std::f32::consts::PI).abs() < 1e-4);
        assert!((wrap_angle(-2.5 * std::f32::consts::PI) + 0.5 * std::f32::consts::PI).abs() < 1e-4);
        assert!((wrap_angle(-0.5) + 0.5).abs() < 1e-6);
        assert_eq!(fmt_hm(1792619000000), "21:43");
        assert_eq!(fmt_session(&Session::new(7, 15, 8, 45)), "07:15-08:45");
    }
}
