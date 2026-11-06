//! The positions pane: your paper trades from the trade ticket today, open and closed, each
//! coloured by its R, with the totals. The open position comes from the desk's board (marked
//! at this tab's own feed); the closed ones from the relay's trade log (`/paper/trades`,
//! filtered to the ticket's trader, since 00:00 UTC), polled every few seconds.

use std::sync::{Arc, Mutex};

use bevy::prelude::*;

use crate::candles::wall_clock_ms;
use crate::desk_feed::DeskFeed;
use crate::jev::relay_url;
use crate::live_feed::LiveFeed;
use crate::panels::{Panel, PanelId, spawn_close_button};
use crate::paper::day_start_ms;
use crate::paper_avatars::{LineTone, instrument_for_symbol};
use crate::pit::{format_price_with_commas, instrument_decimals};
use crate::pit_engine::tick_size;
use crate::ticket::{DEFAULT_TICKET_TRADER, Ticket, ticket_trader};

const POLL_SECS: f64 = 3.0;
/// Text spans kept for the pane's lines; closed trades past this many are summed, not listed.
const LINES: usize = 40;

#[derive(Component)]
pub struct PositionsPanel;
#[derive(Component)]
pub struct PositionsLine(pub usize);

/// One closed trade as the relay logs it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ClosedTrade {
    pub symbol: String,
    pub side: String,
    pub entry: f64,
    pub exit: f64,
    pub closed_ms: i64,
    pub pnl_r: f64,
    pub why_out: String,
}

impl ClosedTrade {
    fn from_json(v: &serde_json::Value) -> Option<Self> {
        Some(ClosedTrade {
            symbol: v.get("symbol")?.as_str()?.to_string(),
            side: v.get("side")?.as_str()?.to_string(),
            entry: v.get("entry")?.as_f64()?,
            exit: v.get("exit")?.as_f64()?,
            closed_ms: v.get("closed_ms")?.as_i64()?,
            pnl_r: v.get("pnl_r")?.as_f64()?,
            why_out: v
                .get("why_out")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string(),
        })
    }
}

#[derive(Resource, Default)]
pub struct PositionsFeed {
    pub closed: Vec<ClosedTrade>,
    pending: Arc<Mutex<Option<Result<String, String>>>>,
    in_flight: bool,
    next_poll: f64,
    pub last_error: Option<String>,
}

/// An open position for the pane: (symbol, side, entry, stop, target, R now).
pub type OpenLine = (String, String, f32, f32, Option<f32>, f32);

fn px(symbol: &str, x: f64) -> String {
    match instrument_for_symbol(symbol) {
        Some(i) => format_price_with_commas(x, instrument_decimals(i)),
        None => format!("{x:.2}"),
    }
}

fn hm(ms: i64) -> String {
    let m = ms.div_euclid(60_000).rem_euclid(1440);
    format!("{:02}:{:02}", m / 60, m % 60)
}

/// The pane's lines with their colours. Pure, for tests.
pub fn positions_lines(open: Option<&OpenLine>, closed: &[ClosedTrade]) -> Vec<(String, LineTone)> {
    let mut out = Vec::new();
    match open {
        Some((sym, side, entry, stop, target, r)) => {
            out.push((format!("OPEN (1)   open P&L {r:+.2}R"), LineTone::Pnl(*r)));
            let t = target
                .map(|t| px(sym, t as f64))
                .unwrap_or_else(|| "none".into());
            out.push((
                format!(
                    "  {sym} {side} from {}  stop {}  target {t}   {r:+.2}R",
                    px(sym, *entry as f64),
                    px(sym, *stop as f64)
                ),
                LineTone::Pnl(*r),
            ));
        }
        None => {
            out.push(("OPEN (0)".into(), LineTone::Head));
            out.push((
                "  none: use the trade ticket to open one".into(),
                LineTone::Plain,
            ));
        }
    }
    let total: f64 = closed.iter().map(|c| c.pnl_r).sum();
    let wins = closed.iter().filter(|c| c.pnl_r > 0.0).count();
    if closed.is_empty() {
        out.push(("CLOSED TODAY (0)".into(), LineTone::Head));
        out.push(("  none yet".into(), LineTone::Plain));
    } else {
        out.push((
            format!(
                "CLOSED TODAY ({})   {wins} won   total {total:+.2}R",
                closed.len()
            ),
            LineTone::Pnl(total as f32),
        ));
        let room = LINES.saturating_sub(out.len() + 2);
        for c in closed.iter().rev().take(room) {
            out.push((
                format!(
                    "  {} {:<7} {:<5} {} -> {}  {:+.2}R  {}",
                    hm(c.closed_ms),
                    c.symbol,
                    c.side,
                    px(&c.symbol, c.entry),
                    px(&c.symbol, c.exit),
                    c.pnl_r,
                    c.why_out
                ),
                LineTone::Pnl(c.pnl_r as f32),
            ));
        }
        if closed.len() > room {
            out.push((
                format!("  ... and {} earlier, in the total", closed.len() - room),
                LineTone::Plain,
            ));
        }
    }
    let open_r = open.map(|o| o.5 as f64).unwrap_or(0.0);
    out.push((
        format!("TODAY IN ALL (closed + open): {:+.2}R", total + open_r),
        LineTone::Pnl((total + open_r) as f32),
    ));
    out
}

#[derive(Component)]
pub struct PositionsTitle;

/// Startup: the pane, in the right column under the trade ticket.
pub fn setup_positions_panel(mut commands: Commands) {
    commands
        .spawn((
            PositionsPanel,
            Panel(PanelId::Positions),
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
            BorderColor::all(Color::srgba(0.35, 0.85, 0.50, 0.55)),
        ))
        .with_children(|panel| {
            let me = panel.target_entity();
            panel
                .spawn(Node {
                    flex_direction: FlexDirection::Row,
                    justify_content: JustifyContent::SpaceBetween,
                    align_items: AlignItems::Center,
                    width: Val::Percent(100.0),
                    ..default()
                })
                .with_children(|row| {
                    row.spawn((
                        PositionsTitle,
                        Text::new(format!(
                            "POSITIONS  |  paper, {DEFAULT_TICKET_TRADER}, today (UTC)"
                        )),
                        TextFont {
                            font_size: 11.0,
                            ..default()
                        },
                        TextColor(Color::srgb(0.55, 1.0, 0.65)),
                        Node {
                            flex_grow: 1.0,
                            ..default()
                        },
                    ));
                    spawn_close_button(row, me);
                });
            panel
                .spawn((
                    Text::new(""),
                    TextFont {
                        font_size: 10.0,
                        ..default()
                    },
                    TextColor(LineTone::Plain.color()),
                ))
                .with_children(|text| {
                    for i in 0..LINES {
                        text.spawn((
                            PositionsLine(i),
                            TextSpan::new(""),
                            TextFont {
                                font_size: 10.0,
                                ..default()
                            },
                            TextColor(LineTone::Plain.color()),
                        ));
                    }
                });
        });
}

/// System: poll the relay for today's closed trades of the ticket's trader.
pub fn poll_positions(time: Res<Time>, desk: Res<DeskFeed>, mut feed: ResMut<PositionsFeed>) {
    let now = time.elapsed_secs_f64();
    let done = feed.pending.lock().ok().and_then(|mut p| p.take());
    if let Some(res) = done {
        feed.in_flight = false;
        match res.and_then(|text| {
            serde_json::from_str::<serde_json::Value>(&text).map_err(|e| e.to_string())
        }) {
            Ok(v) => {
                feed.closed = v
                    .get("trades")
                    .and_then(|t| t.as_array())
                    .into_iter()
                    .flatten()
                    .filter_map(ClosedTrade::from_json)
                    .collect();
                feed.last_error = None;
            }
            Err(e) => feed.last_error = Some(e),
        }
    }
    if feed.in_flight || now < feed.next_poll {
        return;
    }
    feed.next_poll = now + POLL_SECS;
    feed.in_flight = true;
    let since = day_start_ms(wall_clock_ms());
    let name = url_encode(ticket_trader(desk.live_board(time.elapsed_secs_f64())));
    let req = ehttp::Request::get(format!(
        "{}/paper/trades?since_ms={since}&trader={name}&source=desk&limit=2000",
        relay_url()
    ));
    let pending = feed.pending.clone();
    ehttp::fetch(req, move |res| {
        let out = match res {
            Ok(r) if r.ok => Ok(r.text().unwrap_or("").to_string()),
            Ok(r) => Err(format!("relay HTTP {}", r.status)),
            Err(e) => Err(e),
        };
        if let Ok(mut p) = pending.lock() {
            *p = Some(out);
        }
    });
}

/// System: the pane's lines, twice a second.
pub fn update_positions_panel(
    time: Res<Time>,
    feed: Res<PositionsFeed>,
    live: Res<LiveFeed>,
    desk: Res<DeskFeed>,
    ticket: Res<Ticket>,
    mut next: Local<f32>,
    mut spans: Query<(&PositionsLine, &mut TextSpan, &mut TextColor)>,
    mut title: Query<&mut Text, With<PositionsTitle>>,
) {
    if time.elapsed_secs() < *next && !ticket.is_changed() {
        return;
    }
    *next = time.elapsed_secs() + 0.5;
    let board = desk.live_board(time.elapsed_secs_f64());
    for mut t in title.iter_mut() {
        crate::pit::set_ui_text(
            &mut t,
            format!("POSITIONS  |  paper, {}, today (UTC)", ticket_trader(board)),
        );
    }
    let open: Option<OpenLine> = board
        .and_then(|b| b.traders.iter().find(|t| t.trader == ticket_trader(board)))
        .and_then(|t| t.open.as_ref())
        .map(|o| {
            // Marked at this tab's own feed, like the Traders table.
            let r = instrument_for_symbol(&o.symbol)
                .and_then(|i| live.quote(i).map(|q| (i, q)))
                .filter(|_| o.stop != 0.0)
                .map(|(i, q)| {
                    let tick = tick_size(i);
                    let stop_ticks = ((o.entry - o.stop).abs() / tick).max(1.0);
                    let ticks = if o.side == "long" {
                        (q.price as f32 - o.entry) / tick
                    } else {
                        (o.entry - q.ask as f32) / tick
                    };
                    ticks / stop_ticks
                })
                .unwrap_or(o.open_r);
            (
                o.symbol.replace('/', ""),
                o.side.clone(),
                o.entry,
                o.stop,
                o.target,
                r,
            )
        });
    let mut lines = positions_lines(open.as_ref(), &feed.closed);
    if let Some(e) = &feed.last_error {
        lines.push((format!("closed trades: {e}"), LineTone::Plain));
    }
    write_lines(&mut spans, &lines);
}

fn write_lines(
    spans: &mut Query<(&PositionsLine, &mut TextSpan, &mut TextColor)>,
    lines: &[(String, LineTone)],
) {
    let n = lines.len().min(LINES);
    for (line, mut span, mut color) in spans.iter_mut() {
        let (text, tone) = match lines.get(line.0).filter(|_| line.0 < n) {
            Some((t, tone)) => (
                if line.0 + 1 < n {
                    format!("{t}\n")
                } else {
                    t.clone()
                },
                *tone,
            ),
            None => (String::new(), LineTone::Plain),
        };
        if span.0 != text {
            span.0 = text;
        }
        if color.0 != tone.color() {
            color.0 = tone.color();
        }
    }
}

/// Percent-encodes a query value (a name with a space in it), leaving the unreserved set.
fn url_encode(s: &str) -> String {
    s.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_with_a_space_survives_the_query() {
        assert_eq!(url_encode("Ada Lovelace"), "Ada%20Lovelace");
        assert_eq!(url_encode("Zoë&Co"), "Zo%C3%AB%26Co");
    }

    fn closed(r: f64, at: i64) -> ClosedTrade {
        ClosedTrade {
            symbol: "XAUUSD".into(),
            side: "long".into(),
            entry: 2000.2,
            exit: 2000.2 + r,
            closed_ms: at,
            pnl_r: r,
            why_out: "target".into(),
        }
    }

    #[test]
    fn open_and_closed_are_listed_with_their_totals_and_colours() {
        let open: OpenLine = (
            "US500".into(),
            "short".into(),
            6000.0,
            6010.0,
            Some(5980.0),
            -0.4,
        );
        let lines = positions_lines(Some(&open), &[closed(2.0, 1_000), closed(-1.0, 2_000)]);
        let text: Vec<&str> = lines.iter().map(|l| l.0.as_str()).collect();
        assert!(
            text[0].starts_with("OPEN (1)") && lines[0].1 == LineTone::Pnl(-0.4),
            "{text:?}"
        );
        assert!(
            text[2].starts_with("CLOSED TODAY (2)   1 won   total +1.00R"),
            "{text:?}"
        );
        assert!(
            text[3].contains("-1.00R") && text[4].contains("+2.00R"),
            "newest first: {text:?}"
        );
        assert_eq!(
            (lines[3].1, lines[4].1),
            (LineTone::Pnl(-1.0), LineTone::Pnl(2.0))
        );
        assert_eq!(
            text.last().unwrap(),
            &"TODAY IN ALL (closed + open): +0.60R"
        );
        let none = positions_lines(None, &[]);
        assert!(none[0].0.starts_with("OPEN (0)") && none[2].0.starts_with("CLOSED TODAY (0)"));
    }

    #[test]
    fn a_long_day_is_summed_even_where_it_is_not_listed() {
        let many: Vec<ClosedTrade> = (0..100).map(|k| closed(1.0, k)).collect();
        let lines = positions_lines(None, &many);
        assert!(lines.len() <= LINES + 2, "{}", lines.len());
        assert!(lines.iter().any(|l| l.0.contains("earlier, in the total")));
        assert!(lines.iter().any(|l| l.0.contains("total +100.00R")));
    }
}
