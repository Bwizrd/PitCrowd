//! Board chart and price arrows for the 3D pit board: candles for the selected pit and
//! timeframe with every fill plotted on them, drawn as plain UI nodes so it works wherever
//! the UI does (the default font has no arrow glyphs, so arrows are pixel stacks too).
//! `candle_layout` is shared with the wall board so both views agree.

use std::collections::VecDeque;

use bevy::prelude::*;

use crate::candles::{Candle, CandleStore, ChartSettings, wall_clock_ms};
use crate::live_feed::{FeedMode, LiveFeed, PitFeedStatus};
use crate::market::Instrument;
use crate::pit::{format_price_with_commas, instrument_decimals, set_ui_text};
use crate::pit_engine::{PitEngines, PitEventKind, Side, tick_size};
use crate::ui::Selection;

/// Deals older than this fall off the charts.
pub const PRINT_KEEP_SECS: f64 = 3.0 * 3600.0;
pub const CHART_W: f32 = 520.0;
/// The board panel's horizontal padding either side of the chart.
pub const BOARD_PAD_X: f32 = 18.0;
/// The board panel is exactly as wide as its chart plus padding and borders, and never
/// wider: long text lines wrap inside it instead of pushing the panel out (the BOOK sizes
/// did that once the touch line showed the broker's ladder).
pub const BOARD_PANEL_W: f32 = CHART_W + 2.0 + 2.0 * BOARD_PAD_X + 2.0;
/// Characters of the board's 13 px font that fit across the chart (about 7.8 px each).
pub const BOARD_LINE_COLS: usize = (CHART_W / 7.8) as usize;
/// Whole chart: candles above, tick delta and tick count panels below.
pub const CHART_H: f32 = 176.0;
pub const DELTA_H: f32 = 34.0;
pub const RATE_H: f32 = 24.0;
const PANEL_GAP: f32 = 4.0;
/// Bottom of the candle area, measured from the chart's bottom edge.
pub const CANDLE_BASE: f32 = RATE_H + PANEL_GAP + DELTA_H + PANEL_GAP;
pub const CANDLE_H: f32 = CHART_H - CANDLE_BASE;
/// Bars on the board chart: fewer than the wall so each candle gets a clear pixel gap.
pub const BOARD_BARS: usize = 60;
const PRINT_NODES: usize = 160;
pub const PAD_PX: f32 = 4.0;
/// Everything in the board panel that is not the chart: the header row, the price, the touch
/// and book lines, the tape, the state and Jev lines, paddings and borders. The expanded
/// chart takes what is left of the viewport under it.
pub const BOARD_CHROME_H: f32 = 165.0;
/// The expanded chart never shrinks below this, whatever the window.
const MIN_EXPANDED_W: f32 = 240.0;

// ---------------------------------------------------------------------------
// Layout source of truth
// ---------------------------------------------------------------------------

/// The one place the board chart's pixel size lives. Compact is the `CHART_W` x `CHART_H`
/// default; expanded fills the viewport inside the panel-column margins. Everything that
/// positions a chart child (candles, flow bars, level bands and tags, labels, the chart node
/// itself) reads this resource instead of the constants, so the overlays stay on the candles
/// at either size.
#[derive(Resource, Debug, Clone, Copy, PartialEq)]
pub struct ChartLayout {
    pub chart_w: f32,
    pub chart_h: f32,
    /// Vertical padding inside the candle area (`candle_layout` keeps the extremes off the edge).
    pub pad: f32,
    pub expanded: bool,
}

impl Default for ChartLayout {
    fn default() -> Self {
        Self::compact()
    }
}

impl ChartLayout {
    pub const fn compact() -> Self {
        Self {
            chart_w: CHART_W,
            chart_h: CHART_H,
            pad: PAD_PX,
            expanded: false,
        }
    }

    /// Layout for a window of `win_w` x `win_h` logical pixels. Expanded: the panel spans
    /// the columns' left and right margins and runs from the column top to the bottom
    /// button rows, and the chart is the panel minus its chrome. Candle count never changes
    /// (`BOARD_BARS`): more pixels per bar, not more bars.
    pub fn for_viewport(win_w: f32, win_h: f32, expanded: bool) -> Self {
        if !expanded {
            return Self::compact();
        }
        use crate::panels::{COLUMN_BOTTOM, COLUMN_SIDE, COLUMN_TOP};
        let chrome_w = BOARD_PANEL_W - CHART_W;
        let chart_w = (win_w - 2.0 * COLUMN_SIDE - chrome_w).max(MIN_EXPANDED_W);
        let chart_h = (win_h - COLUMN_TOP - COLUMN_BOTTOM - BOARD_CHROME_H).max(CHART_H);
        Self {
            chart_w,
            chart_h,
            pad: PAD_PX,
            expanded: true,
        }
    }

    /// Height of the candle area (the flow panels under it are fixed-height).
    pub fn candle_h(&self) -> f32 {
        self.chart_h - CANDLE_BASE
    }

    /// The board panel's outer width for this chart: chart + its border + padding + panel border.
    pub fn panel_w(&self) -> f32 {
        self.chart_w + (BOARD_PANEL_W - CHART_W)
    }

    /// Horizontal pixels per bar slot.
    pub fn slot(&self) -> f32 {
        self.chart_w / BOARD_BARS as f32
    }

    /// Candle body width: whole pixels so neighbouring candles never touch.
    pub fn body_w(&self) -> f32 {
        (self.slot() * 0.6).floor().max(2.0)
    }
}

// ---------------------------------------------------------------------------
// Board text
// ---------------------------------------------------------------------------

/// A ladder size short enough for the board: 2300000 -> "2.3M", 15000 -> "15k", 250 -> "250".
/// One decimal only while the mantissa is a single digit (2.3k, not 123.5M).
pub fn abbrev_size(size: f32) -> String {
    let trim = |s: String| s.trim_end_matches('0').trim_end_matches('.').to_string();
    let mantissa = |m: f32| {
        if m.abs() >= 10.0 {
            format!("{m:.0}")
        } else {
            trim(format!("{m:.1}"))
        }
    };
    let a = size.abs();
    if a >= 1e6 {
        format!("{}M", mantissa(size / 1e6))
    } else if a >= 1e3 {
        format!("{}k", mantissa(size / 1e3))
    } else if a >= 10.0 || a == 0.0 {
        format!("{size:.0}")
    } else {
        trim(format!("{size:.2}"))
    }
}

/// The board's touch line: the real bid and ask with the spread in pit ticks, and under it,
/// when the broker's ladder is known, its sizes abbreviated so the line fits the panel.
pub fn touch_lines(
    bid: &str,
    ask: &str,
    spread_ticks: i64,
    bids: &[(f32, f32)],
    asks: &[(f32, f32)],
) -> String {
    let sizes = |side: &[(f32, f32)]| {
        side.iter()
            .map(|(_, s)| abbrev_size(*s))
            .collect::<Vec<_>>()
            .join("/")
    };
    let touch = format!("BID {bid}   ASK {ask}   ({spread_ticks} ticks)");
    if bids.is_empty() && asks.is_empty() {
        touch
    } else {
        format!("{touch}\nBOOK {} x {}", sizes(bids), sizes(asks))
    }
}

// ---------------------------------------------------------------------------
// Deal history
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Print {
    /// App clock, seconds.
    pub at: f64,
    /// Wall clock, Unix ms, so a deal lands on the right bar.
    pub wall_ms: i64,
    pub price: f32,
    pub side: Side,
}

#[derive(Debug, Clone, Default)]
pub struct PitTape {
    pub prints: VecDeque<Print>,
    /// Bumped on every change so charts redraw only when there is something new.
    pub revision: u64,
}

impl PitTape {
    pub fn record(&mut self, now: f64, wall_ms: i64, fills: &[(f32, Side)]) {
        for &(price, side) in fills {
            self.prints.push_back(Print {
                at: now,
                wall_ms,
                price,
                side,
            });
            self.revision += 1;
        }
        while self
            .prints
            .front()
            .is_some_and(|p| now - p.at > PRINT_KEEP_SECS)
        {
            self.prints.pop_front();
        }
        while self.prints.len() > PRINT_NODES {
            self.prints.pop_front();
        }
    }

    pub fn marks(&self) -> Vec<(i64, f32, Side)> {
        self.prints
            .iter()
            .map(|p| (p.wall_ms, p.price, p.side))
            .collect()
    }
}

#[derive(Resource)]
pub struct TapeHistory {
    pub pits: Vec<PitTape>,
}

impl Default for TapeHistory {
    fn default() -> Self {
        Self {
            pits: Instrument::ALL.iter().map(|_| PitTape::default()).collect(),
        }
    }
}

impl TapeHistory {
    pub fn tape(&self, inst: Instrument) -> &PitTape {
        &self.pits[inst as usize]
    }
}

/// System: records every pit's fills. Runs after the engine step so this frame's fills are
/// still in the recent list.
pub fn record_tape(time: Res<Time>, engines: Res<PitEngines>, mut history: ResMut<TapeHistory>) {
    let now = time.elapsed_secs_f64();
    let wall = wall_clock_ms();
    for (i, inst) in Instrument::ALL.iter().enumerate() {
        let pit = engines.pit(*inst);
        let fills: Vec<(f32, Side)> = pit
            .recent_events()
            .iter()
            .filter_map(|e| match e.kind {
                PitEventKind::Fill {
                    side, price_ticks, ..
                } => Some((pit.price_of(price_ticks), side)),
                _ => None,
            })
            .collect();
        history.pits[i].record(now, wall, &fills);
    }
}

// ---------------------------------------------------------------------------
// Layout (shared with the wall board)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CandleBox {
    pub x: f32,
    pub half_w: f32,
    pub wick_lo: f32,
    pub wick_hi: f32,
    pub body_lo: f32,
    pub body_hi: f32,
    pub up: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CandleLayout {
    pub candles: Vec<CandleBox>,
    /// (x, y from bottom, side)
    pub prints: Vec<(f32, f32, Side)>,
    pub hi: f32,
    pub lo: f32,
}

/// Lay the newest `slots` bars across `w` x `h`, newest at the right, with deals placed by
/// wall-clock time on the same axis. `min_span` stops a flat market collapsing the scale.
pub fn candle_layout(
    bars: &[Candle],
    prints: &[(i64, f32, Side)],
    tf_ms: i64,
    w: f32,
    h: f32,
    min_span: f32,
    slots: usize,
) -> Option<CandleLayout> {
    if bars.is_empty() {
        return None;
    }
    let n = bars.len().min(slots);
    let bars = &bars[bars.len() - n..];
    let t0 = bars[0].time_ms;
    let t_end = bars[n - 1].time_ms + tf_ms;
    let prints: Vec<&(i64, f32, Side)> = prints
        .iter()
        .filter(|p| p.0 >= t0 && p.0 <= t_end)
        .collect();

    let (mut lo, mut hi) = (f32::MAX, f32::MIN);
    for b in bars {
        lo = lo.min(b.low);
        hi = hi.max(b.high);
    }
    for p in &prints {
        lo = lo.min(p.1);
        hi = hi.max(p.1);
    }
    if hi - lo < min_span {
        let mid = (hi + lo) * 0.5;
        lo = mid - min_span * 0.5;
        hi = mid + min_span * 0.5;
    }
    let slot = w / slots as f32;
    let half_w = (slot * 0.36).max(1.0);
    let y_of = |p: f32| PAD_PX + (p - lo) / (hi - lo) * (h - 2.0 * PAD_PX);
    let x_center = |i: f32| w - (n as f32 - i - 0.5) * slot;
    let candles = bars
        .iter()
        .enumerate()
        .map(|(i, b)| CandleBox {
            x: x_center(i as f32),
            half_w,
            wick_lo: y_of(b.low),
            wick_hi: y_of(b.high),
            body_lo: y_of(b.open.min(b.close)),
            body_hi: y_of(b.open.max(b.close)),
            up: b.is_up(),
        })
        .collect();
    let prints = prints
        .iter()
        .map(|p| {
            let frac = (p.0 - t0) as f32 / tf_ms as f32;
            (
                (x_center(0.0) - 0.5 * slot + frac * slot).clamp(0.0, w),
                y_of(p.1),
                p.2,
            )
        })
        .collect();
    Some(CandleLayout {
        candles,
        prints,
        hi,
        lo,
    })
}

/// Which series a chart should show for a pit right now, and how to describe it.
pub fn chart_source(feed: &LiveFeed, inst: Instrument, now: f64) -> (FeedMode, &'static str) {
    match feed.mode {
        FeedMode::Live => match feed.pit_status(inst, now) {
            PitFeedStatus::Live { .. } => (FeedMode::Live, "LIVE tape"),
            _ => (FeedMode::Live, "LIVE (HALTED - last bars)"),
        },
        FeedMode::Simulated => (FeedMode::Simulated, "SIM - pit prints"),
    }
}

// ---------------------------------------------------------------------------
// Board chart nodes
// ---------------------------------------------------------------------------

#[derive(Component)]
pub struct TapeChart;

#[derive(Component)]
pub struct ChartWick(pub usize);

#[derive(Component)]
pub struct ChartBody(pub usize);

#[derive(Component)]
pub struct ChartPrintDot(pub usize);

#[derive(Component)]
pub struct ChartDeltaBar(pub usize);

#[derive(Component)]
pub struct ChartRateBar(pub usize);

#[derive(Component, Clone, Copy, PartialEq, Eq)]
pub enum ChartLabel {
    Hi,
    Lo,
    Note,
    Delta,
    Rate,
}

/// Heights of the delta and tick-count bars for the shown candles, scaled to their panels.
/// Delta is signed (up from the panel's centre line); rate is from the panel floor.
#[derive(Debug, Clone, PartialEq)]
pub struct FlowLayout {
    /// Signed bar height in px, |h| <= DELTA_H / 2.
    pub delta: Vec<f32>,
    /// Bar height in px, <= RATE_H - 2.
    pub rate: Vec<f32>,
    pub max_delta: f32,
    pub max_rate: f32,
}

pub fn flow_layout(bars: &[Candle], delta_h: f32, rate_h: f32) -> FlowLayout {
    let max_delta = bars
        .iter()
        .map(|b| b.delta.abs())
        .fold(0.0f32, f32::max)
        .max(1.0);
    let max_rate = bars
        .iter()
        .map(|b| b.volume)
        .fold(0.0f32, f32::max)
        .max(1.0);
    FlowLayout {
        delta: bars
            .iter()
            .map(|b| b.delta / max_delta * (delta_h * 0.5 - 1.0))
            .collect(),
        rate: bars
            .iter()
            .map(|b| b.volume / max_rate * (rate_h - 3.0))
            .collect(),
        max_delta,
        max_rate,
    }
}

pub const UP_COLOR: Color = Color::srgb(0.25, 1.0, 0.45);
pub const DOWN_COLOR: Color = Color::srgb(1.0, 0.32, 0.32);
const RATE_COLOR: Color = Color::srgb(0.36, 0.52, 0.70);
const BUY_MARK: Color = Color::srgb(0.55, 1.0, 0.95);
const SELL_MARK: Color = Color::srgb(1.0, 0.75, 0.30);

/// Spawn the chart inside a board column. Call from the board's `with_children` closure.
pub fn spawn_tape_chart(parent: &mut ChildSpawnerCommands) {
    parent
        .spawn((
            TapeChart,
            Node {
                width: Val::Px(CHART_W),
                height: Val::Px(CHART_H),
                margin: UiRect::top(Val::Px(6.0)),
                border: UiRect::all(Val::Px(1.0)),
                overflow: Overflow::clip(),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.45)),
            BorderColor::all(Color::srgba(0.85, 0.70, 0.25, 0.35)),
        ))
        .with_children(|chart| {
            for i in 0..BOARD_BARS {
                chart.spawn((
                    ChartWick(i),
                    Node {
                        position_type: PositionType::Absolute,
                        width: Val::Px(1.0),
                        display: Display::None,
                        ..default()
                    },
                    BackgroundColor(UP_COLOR),
                ));
                chart.spawn((
                    ChartBody(i),
                    Node {
                        position_type: PositionType::Absolute,
                        display: Display::None,
                        ..default()
                    },
                    BackgroundColor(UP_COLOR),
                ));
            }
            for i in 0..BOARD_BARS {
                chart.spawn((
                    ChartDeltaBar(i),
                    Node {
                        position_type: PositionType::Absolute,
                        display: Display::None,
                        ..default()
                    },
                    BackgroundColor(UP_COLOR),
                ));
                chart.spawn((
                    ChartRateBar(i),
                    Node {
                        position_type: PositionType::Absolute,
                        display: Display::None,
                        ..default()
                    },
                    BackgroundColor(RATE_COLOR),
                ));
            }
            // Panel separators and the delta centre line.
            for bottom in [RATE_H + PANEL_GAP * 0.5, CANDLE_BASE - PANEL_GAP * 0.5] {
                chart.spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(0.0),
                        bottom: Val::Px(bottom),
                        width: Val::Percent(100.0),
                        height: Val::Px(1.0),
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.85, 0.70, 0.25, 0.25)),
                ));
            }
            chart.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(0.0),
                    bottom: Val::Px(RATE_H + PANEL_GAP + DELTA_H * 0.5),
                    width: Val::Percent(100.0),
                    height: Val::Px(1.0),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.75, 0.80, 0.88, 0.18)),
            ));
            for i in 0..PRINT_NODES {
                chart.spawn((
                    ChartPrintDot(i),
                    Node {
                        position_type: PositionType::Absolute,
                        width: Val::Px(4.0),
                        height: Val::Px(4.0),
                        display: Display::None,
                        ..default()
                    },
                    BackgroundColor(BUY_MARK),
                ));
            }
            crate::levels::spawn_level_nodes(chart);
            let label = |chart: &mut ChildSpawnerCommands,
                         which: ChartLabel,
                         top: Option<f32>,
                         bottom: Option<f32>,
                         right: bool| {
                chart.spawn((
                    which,
                    Text::new(""),
                    TextFont {
                        font_size: 9.0,
                        ..default()
                    },
                    TextColor(Color::srgba(0.75, 0.80, 0.88, 0.9)),
                    Node {
                        position_type: PositionType::Absolute,
                        top: top.map(Val::Px).unwrap_or(Val::Auto),
                        bottom: bottom.map(Val::Px).unwrap_or(Val::Auto),
                        left: if right { Val::Auto } else { Val::Px(3.0) },
                        right: if right { Val::Px(3.0) } else { Val::Auto },
                        ..default()
                    },
                ));
            };
            label(chart, ChartLabel::Hi, Some(1.0), None, false);
            label(chart, ChartLabel::Lo, None, Some(CANDLE_BASE + 1.0), false);
            label(chart, ChartLabel::Note, Some(1.0), None, true);
            label(
                chart,
                ChartLabel::Delta,
                None,
                Some(RATE_H + PANEL_GAP + DELTA_H - 11.0),
                false,
            );
            label(chart, ChartLabel::Rate, None, Some(RATE_H - 11.0), false);
        });
}

/// System: lays the inspected pit's candles and deals out on the board whenever they change
/// or the chart changes size (`ChartLayout`).
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn update_tape_chart(
    time: Res<Time>,
    feed: Res<LiveFeed>,
    selection: Res<Selection>,
    settings: Res<ChartSettings>,
    layout: Res<ChartLayout>,
    store: Res<CandleStore>,
    history: Res<TapeHistory>,
    mut drawn: Local<
        Option<(
            Instrument,
            u64,
            u64,
            FeedMode,
            crate::candles::Timeframe,
            Option<&'static str>,
            bool,
        )>,
    >,
    mut wicks: Query<
        (&ChartWick, &mut Node, &mut BackgroundColor),
        (
            Without<ChartBody>,
            Without<ChartPrintDot>,
            Without<ChartLabel>,
        ),
    >,
    mut bodies: Query<
        (&ChartBody, &mut Node, &mut BackgroundColor),
        (
            Without<ChartWick>,
            Without<ChartPrintDot>,
            Without<ChartLabel>,
        ),
    >,
    mut print_dots: Query<
        (&ChartPrintDot, &mut Node, &mut BackgroundColor),
        (
            Without<ChartWick>,
            Without<ChartBody>,
            Without<ChartLabel>,
            Without<ChartDeltaBar>,
            Without<ChartRateBar>,
        ),
    >,
    mut delta_bars: Query<
        (&ChartDeltaBar, &mut Node, &mut BackgroundColor),
        (
            Without<ChartWick>,
            Without<ChartBody>,
            Without<ChartLabel>,
            Without<ChartPrintDot>,
            Without<ChartRateBar>,
        ),
    >,
    mut rate_bars: Query<
        (&ChartRateBar, &mut Node),
        (
            Without<ChartWick>,
            Without<ChartBody>,
            Without<ChartLabel>,
            Without<ChartPrintDot>,
            Without<ChartDeltaBar>,
        ),
    >,
    mut labels: Query<(&ChartLabel, &mut Text)>,
    mut chart_node: Query<
        &mut Node,
        (
            With<TapeChart>,
            Without<ChartWick>,
            Without<ChartBody>,
            Without<ChartPrintDot>,
            Without<ChartLabel>,
            Without<ChartDeltaBar>,
            Without<ChartRateBar>,
        ),
    >,
) {
    for mut node in chart_node.iter_mut() {
        let want = if settings.board_chart {
            Display::Flex
        } else {
            Display::None
        };
        if node.display != want {
            node.display = want;
        }
        if layout.is_changed() {
            node.width = Val::Px(layout.chart_w);
            node.height = Val::Px(layout.chart_h);
        }
    }
    if !settings.board_chart {
        return;
    }
    let Some(inst) = selection.selected_instrument else {
        return;
    };
    let now = time.elapsed_secs_f64();
    let (mode, source) = chart_source(&feed, inst, now);
    let tf = settings.timeframe;
    let series = store.series(inst, tf, mode);
    let tape = history.tape(inst);
    let key = (
        inst,
        series.revision,
        tape.revision,
        mode,
        tf,
        store.backfill_note(inst, tf),
        settings.deal_markers,
    );
    if *drawn == Some(key) && !layout.is_changed() {
        return;
    }
    *drawn = Some(key);
    let bars = series.shown();
    let marks = if settings.deal_markers {
        tape.marks()
    } else {
        Vec::new()
    };
    let size = *layout;
    let layout = candle_layout(
        &bars,
        &marks,
        tf.millis(),
        size.chart_w,
        size.candle_h(),
        tick_size(inst) * 6.0,
        BOARD_BARS,
    );
    let shown_bars = &bars[bars.len() - layout.as_ref().map_or(0, |l| l.candles.len())..];
    let flow = flow_layout(shown_bars, DELTA_H, RATE_H);
    let slot = size.slot();
    // UI nodes land on whole pixels; snap so neighbouring candles never touch.
    let body_w = size.body_w();

    for (wick, mut node, mut color) in wicks.iter_mut() {
        match layout.as_ref().and_then(|l| l.candles.get(wick.0)) {
            Some(c) => {
                node.display = Display::Flex;
                node.left = Val::Px(c.x.round());
                node.bottom = Val::Px((CANDLE_BASE + c.wick_lo).round());
                node.height = Val::Px((c.wick_hi - c.wick_lo).round().max(1.0));
                color.0 = if c.up { UP_COLOR } else { DOWN_COLOR };
            }
            None => node.display = Display::None,
        }
    }
    for (body, mut node, mut color) in bodies.iter_mut() {
        match layout.as_ref().and_then(|l| l.candles.get(body.0)) {
            Some(c) => {
                node.display = Display::Flex;
                node.left = Val::Px((c.x - body_w * 0.5).round());
                node.width = Val::Px(body_w);
                node.bottom = Val::Px((CANDLE_BASE + c.body_lo).round());
                node.height = Val::Px((c.body_hi - c.body_lo).round().max(1.0));
                color.0 = if c.up { UP_COLOR } else { DOWN_COLOR };
            }
            None => node.display = Display::None,
        }
    }
    for (dot, mut node, mut color) in print_dots.iter_mut() {
        match layout.as_ref().and_then(|l| l.prints.get(dot.0)) {
            Some(&(x, y, side)) => {
                node.display = Display::Flex;
                node.left = Val::Px(x - 2.0);
                node.bottom = Val::Px(CANDLE_BASE + y - 2.0);
                color.0 = if side.is_buy() { BUY_MARK } else { SELL_MARK };
            }
            None => node.display = Display::None,
        }
    }
    // Flow panels under the candles, one bar per candle slot.
    let centre = RATE_H + PANEL_GAP + DELTA_H * 0.5;
    for (bar, mut node, mut color) in delta_bars.iter_mut() {
        match (
            layout.as_ref().and_then(|l| l.candles.get(bar.0)),
            flow.delta.get(bar.0),
        ) {
            (Some(c), Some(&d)) if d.abs() >= 0.5 => {
                node.display = Display::Flex;
                node.left = Val::Px((c.x - body_w * 0.5).round());
                node.width = Val::Px(body_w);
                node.bottom = Val::Px(if d >= 0.0 { centre } else { centre + d }.round());
                node.height = Val::Px(d.abs().round().max(1.0));
                color.0 = if d >= 0.0 { UP_COLOR } else { DOWN_COLOR };
            }
            _ => node.display = Display::None,
        }
    }
    for (bar, mut node) in rate_bars.iter_mut() {
        match (
            layout.as_ref().and_then(|l| l.candles.get(bar.0)),
            flow.rate.get(bar.0),
        ) {
            (Some(c), Some(&h)) if h >= 0.5 => {
                node.display = Display::Flex;
                node.left = Val::Px((c.x - slot * 0.5).round() + 1.0);
                node.width = Val::Px((slot - 2.0).max(1.0));
                node.bottom = Val::Px(1.0);
                node.height = Val::Px(h.round().max(1.0));
            }
            _ => node.display = Display::None,
        }
    }

    let decimals = instrument_decimals(inst);
    for (which, mut text) in labels.iter_mut() {
        let value = match (which, layout.as_ref()) {
            (ChartLabel::Delta, Some(_)) => format!(
                "TICK DELTA (up - down per bar; history from the tape, 0 for bridge-only bars)  max {:.0}",
                flow.max_delta
            ),
            (ChartLabel::Rate, Some(_)) => format!("TICKS PER BAR  max {:.0}", flow.max_rate),
            (ChartLabel::Hi, Some(l)) => format_price_with_commas(l.hi as f64, decimals),
            (ChartLabel::Lo, Some(l)) => format_price_with_commas(l.lo as f64, decimals),
            (ChartLabel::Note, Some(_)) => {
                let deals = if settings.deal_markers {
                    "  |  PIT DEALS (debug): cyan buys, amber sells"
                } else {
                    ""
                };
                match store
                    .backfill_note(inst, tf)
                    .filter(|_| mode == FeedMode::Live)
                {
                    Some(note) => format!(
                        "{} x {}  {source}  |  {note}{deals}",
                        BOARD_BARS,
                        tf.label()
                    ),
                    None => format!("{} x {}  {source}{deals}", BOARD_BARS, tf.label()),
                }
            }
            (ChartLabel::Note, None) => format!("{}  {source}  |  waiting for bars", tf.label()),
            (_, None) => String::new(),
        };
        set_ui_text(&mut text, value);
    }
}

// ---------------------------------------------------------------------------
// Pixel arrows
// ---------------------------------------------------------------------------

/// A three-row pixel triangle. Two are spawned per price (one up, one down) and the
/// caller shows the one that applies.
#[derive(Component, Clone, Copy)]
pub struct PriceArrow {
    pub up: bool,
}

pub fn spawn_price_arrows(parent: &mut ChildSpawnerCommands, scale: f32) {
    for up in [true, false] {
        let color = if up { UP_COLOR } else { DOWN_COLOR };
        parent
            .spawn((
                PriceArrow { up },
                Node {
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    justify_content: JustifyContent::Center,
                    width: Val::Px(6.0 * scale),
                    height: Val::Px(6.0 * scale),
                    display: Display::None,
                    ..default()
                },
            ))
            .with_children(|a| {
                let widths: [f32; 3] = if up { [2.0, 4.0, 6.0] } else { [6.0, 4.0, 2.0] };
                for w in widths {
                    a.spawn((
                        Node {
                            width: Val::Px(w * scale),
                            height: Val::Px(2.0 * scale),
                            ..default()
                        },
                        BackgroundColor(color),
                    ));
                }
            });
    }
}

/// Show the arrow matching `up` (neither when `None`).
pub fn set_arrow(arrow: &PriceArrow, node: &mut Node, up: Option<bool>) {
    let want = if up == Some(arrow.up) {
        Display::Flex
    } else {
        Display::None
    };
    if node.display != want {
        node.display = want;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bar(t: i64, o: f32, h: f32, l: f32, c: f32) -> Candle {
        Candle {
            time_ms: t,
            open: o,
            high: h,
            low: l,
            close: c,
            volume: 1.0,
            ..Default::default()
        }
    }

    #[test]
    fn tape_keeps_recent_deals_and_bumps_revision() {
        let mut tape = PitTape::default();
        tape.record(0.0, 1_000, &[(10.0, Side::Buy)]);
        tape.record(1.0, 2_000, &[(10.5, Side::Sell), (10.4, Side::Sell)]);
        assert_eq!(tape.prints.len(), 3);
        assert_eq!(tape.revision, 3);
        tape.record(PRINT_KEEP_SECS + 2.0, 3_000, &[]);
        assert!(tape.prints.is_empty(), "old deals fall off");
    }

    #[test]
    fn layout_puts_newest_bar_at_the_right_and_scales_to_the_box() {
        let bars = vec![
            bar(0, 10.0, 11.0, 9.0, 10.5),
            bar(60_000, 10.5, 12.0, 10.0, 11.5),
            bar(120_000, 11.5, 14.0, 11.0, 12.0),
        ];
        let prints = vec![
            (30_000, 10.2, Side::Buy),
            (165_000, 13.0, Side::Sell),
            (-5, 9.5, Side::Buy),
        ];
        let l = candle_layout(&bars, &prints, 60_000, 300.0, 100.0, 0.1, 3).unwrap();
        assert_eq!(l.candles.len(), 3);
        assert!(l.candles[0].x < l.candles[1].x && l.candles[1].x < l.candles[2].x);
        assert!(
            (l.candles[2].x - 250.0).abs() < 1e-3,
            "newest bar centred in the last slot"
        );
        assert!(
            (l.candles[0].wick_lo - PAD_PX).abs() < 1e-3,
            "lowest low on the bottom pad"
        );
        assert!(
            (l.candles[2].wick_hi - (100.0 - PAD_PX)).abs() < 1e-3,
            "highest high on the top pad"
        );
        assert!(l.candles[0].up && l.candles[2].up);
        assert_eq!(l.prints.len(), 2, "a deal before the first bar is dropped");
        assert!(
            (l.prints[0].0 - 50.0).abs() < 1e-3,
            "a deal half way through bar 0 sits at its centre"
        );
        assert!(
            l.prints[1].0 > l.candles[2].x,
            "a deal in the newest bar's second half sits right of its centre"
        );
        assert!((l.lo - 9.0).abs() < 1e-6 && (l.hi - 14.0).abs() < 1e-6);
    }

    #[test]
    fn layout_shows_only_the_newest_slots_and_pads_flat_markets() {
        let bars: Vec<Candle> = (0..10)
            .map(|i| bar(i * 60_000, 5.0, 5.0, 5.0, 5.0))
            .collect();
        let l = candle_layout(&bars, &[], 60_000, 100.0, 50.0, 1.0, 4).unwrap();
        assert_eq!(l.candles.len(), 4);
        assert!((l.hi - l.lo - 1.0).abs() < 1e-6);
        assert!(
            (l.candles[0].body_lo - 25.0).abs() < 1e-3,
            "flat bars are drawn mid-box"
        );
        assert!(candle_layout(&[], &[], 60_000, 100.0, 50.0, 1.0, 4).is_none());
    }

    #[test]
    fn flow_panels_scale_to_the_largest_bar() {
        let mut a = bar(0, 1.0, 1.0, 1.0, 1.0);
        a.delta = 4.0;
        a.volume = 10.0;
        let mut b = bar(60_000, 1.0, 1.0, 1.0, 1.0);
        b.delta = -8.0;
        b.volume = 40.0;
        let f = flow_layout(&[a, b], 34.0, 24.0);
        assert_eq!(f.max_delta, 8.0);
        assert_eq!(f.max_rate, 40.0);
        assert!(
            (f.delta[1] + 16.0).abs() < 1e-3,
            "largest |delta| fills half the panel: {:?}",
            f.delta
        );
        assert!((f.delta[0] - 8.0).abs() < 1e-3);
        assert!((f.rate[1] - 21.0).abs() < 1e-3 && (f.rate[0] - 5.25).abs() < 1e-3);
        let empty = flow_layout(&[], 34.0, 24.0);
        assert!(empty.delta.is_empty() && empty.max_delta == 1.0);
    }

    #[test]
    fn ladder_sizes_abbreviate() {
        assert_eq!(abbrev_size(2_300_000.0), "2.3M");
        assert_eq!(abbrev_size(1_000_000.0), "1M");
        assert_eq!(abbrev_size(15_000.0), "15k");
        assert_eq!(abbrev_size(2_360.0), "2.4k");
        assert_eq!(abbrev_size(123_456_789.0), "123M");
        assert_eq!(abbrev_size(250.0), "250");
        assert_eq!(abbrev_size(0.5), "0.5");
        assert_eq!(abbrev_size(0.0), "0");
    }

    #[test]
    fn touch_lines_fit_the_panel_even_for_bitcoin_with_a_full_ladder() {
        let ladder: Vec<(f32, f32)> = (0..5)
            .map(|i| (84_663.79 - i as f32, 123_456_789.0))
            .collect();
        let text = touch_lines("84,663.79", "84,678.79", 150, &ladder, &ladder);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(
            lines.len(),
            2,
            "touch on one line, the book on the next: {text}"
        );
        assert!(lines[0].starts_with("BID 84,663.79   ASK 84,678.79   (150 ticks)"));
        assert_eq!(
            lines[1],
            "BOOK 123M/123M/123M/123M/123M x 123M/123M/123M/123M/123M"
        );
        for l in &lines {
            assert!(
                l.chars().count() <= BOARD_LINE_COLS,
                "{} cols > {BOARD_LINE_COLS}: {l}",
                l.chars().count()
            );
        }
        assert_eq!(
            touch_lines("57.630", "57.670", 4, &[], &[]),
            "BID 57.630   ASK 57.670   (4 ticks)"
        );
        assert!(
            (BOARD_PANEL_W - 560.0).abs() < 1e-6,
            "chart 520 + its border + 18 px padding each side + the panel border"
        );
    }

    #[test]
    fn chart_layout_compact_is_the_constants_and_expanded_fills_the_viewport() {
        let c = ChartLayout::default();
        assert_eq!(c, ChartLayout::compact());
        assert_eq!((c.chart_w, c.chart_h, c.pad), (CHART_W, CHART_H, PAD_PX));
        assert!((c.panel_w() - BOARD_PANEL_W).abs() < 1e-6);
        assert!((c.candle_h() - CANDLE_H).abs() < 1e-6);
        assert_eq!(
            ChartLayout::for_viewport(1280.0, 800.0, false),
            c,
            "compact ignores the window"
        );

        let e = ChartLayout::for_viewport(1280.0, 800.0, true);
        assert!(e.expanded);
        // Panel spans the 8 px margins: 1280 - 16 = 1264 outer, chart = outer - chrome (40).
        assert!((e.panel_w() - 1264.0).abs() < 1e-6, "panel {}", e.panel_w());
        assert!((e.chart_w - 1224.0).abs() < 1e-6, "chart {}", e.chart_w);
        // 800 - 58 top - 140 buttons - 165 chrome = 437.
        assert!((e.chart_h - 437.0).abs() < 1e-6, "height {}", e.chart_h);
        assert!(e.candle_h() > c.candle_h() && e.slot() > c.slot() && e.body_w() > c.body_w());
        assert_eq!(e.pad, PAD_PX);
    }

    #[test]
    fn chart_layout_never_shrinks_below_the_compact_chart_on_a_small_window() {
        let tiny = ChartLayout::for_viewport(200.0, 300.0, true);
        assert!(tiny.expanded);
        assert!((tiny.chart_w - MIN_EXPANDED_W).abs() < 1e-6);
        assert!(
            (tiny.chart_h - CHART_H).abs() < 1e-6,
            "height floors at the compact chart"
        );
        let phone = ChartLayout::for_viewport(390.0, 844.0, true);
        assert!((phone.chart_w - (390.0 - 16.0 - 40.0)).abs() < 1e-6);
        assert!((phone.chart_h - (844.0 - 58.0 - 140.0 - 165.0)).abs() < 1e-6);
    }

    #[test]
    fn arrows_show_one_direction_or_none() {
        let (up, down) = (PriceArrow { up: true }, PriceArrow { up: false });
        let mut n = Node::default();
        set_arrow(&up, &mut n, Some(true));
        assert_eq!(n.display, Display::Flex);
        set_arrow(&up, &mut n, Some(false));
        assert_eq!(n.display, Display::None);
        set_arrow(&down, &mut n, Some(false));
        assert_eq!(n.display, Display::Flex);
        set_arrow(&down, &mut n, None);
        assert_eq!(n.display, Display::None);
    }
}
