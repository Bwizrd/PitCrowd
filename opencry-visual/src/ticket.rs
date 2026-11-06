//! The trade ticket: buy, sell or close a PAPER trade on the pit in view, by hand.
//!
//! The ticket posts the order to the relay (`POST /paper/order`); the headless paper desk
//! polls the relay every second and fills it at that pit's touch as a trade for the ticket's
//! trader, with the desk's own costs and rules, so it is listed, walks the floor and is logged
//! like any other trader's. Nothing here or on the desk talks to a broker.
//!
//! The trader is "Guest" until you name yourself: **Change name**, next
//! to "Your name" on the ticket, opens a box on the page (`#namebox` in `index.html`; not
//! `window.prompt`, which browsers may suppress). This browser remembers the name, and the desk renames the seat and keeps the
//! name across restarts. `?trader=Your%20Name` in the address does the same without a click,
//! and `OPENCRY_TRADER_NAME` sets the desk's starting name.
//!
//! Stop: a multiple of the 15m ATR (the desk's 20-spread floor still applies). Target: a
//! multiple of the stop. Both cycle on a click. One position at a time: Close it first.

use std::sync::{Arc, Mutex};

use bevy::prelude::*;

use crate::desk_feed::DeskFeed;
use crate::jev::relay_url;
use crate::levels::tape_symbol;
use crate::live_feed::LiveFeed;
use crate::market::Instrument;
use crate::panels::{Panel, PanelId, spawn_close_button};
use crate::pit::{format_price_with_commas, instrument_decimals, set_ui_text};
use crate::ui::Selection;

/// Stops on offer, in ATR(14) of 15m bars, and targets, in R.
pub const STOPS: [f32; 6] = [0.5, 1.0, 1.5, 2.0, 3.0, 5.0];
pub const TARGETS: [f32; 5] = [1.0, 1.5, 2.0, 3.0, 5.0];
/// The trader the desk books the ticket's orders to, unless `OPENCRY_TRADER_NAME` names
/// another when the desk starts. The project's author, so the name is a credit too.
pub const DEFAULT_TICKET_TRADER: &str = "Guest";

/// The ticket's trader as the running desk names it, else the default.
pub fn ticket_trader(board: Option<&crate::desk_feed::DeskBoard>) -> &str {
    board
        .and_then(|b| b.ticket_trader.as_deref())
        .filter(|n| !n.is_empty())
        .unwrap_or(DEFAULT_TICKET_TRADER)
}

/// A trader's name from the environment value: trimmed, at most 32 characters, else the
/// default. Pure, for tests.
pub fn trader_name_from(v: Option<&str>) -> String {
    match v.map(str::trim).filter(|n| !n.is_empty()) {
        Some(n) => n.chars().take(32).collect(),
        None => DEFAULT_TICKET_TRADER.to_string(),
    }
}

#[derive(Component)]
pub struct TicketPanel;

#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub enum TicketButton {
    Buy,
    Sell,
    Close,
    Stop,
    Target,
    Name,
}

#[derive(Component)]
pub struct TicketTitle;
#[derive(Component)]
pub struct TicketQuote;
#[derive(Component)]
pub struct TicketStatus;
#[derive(Component)]
pub struct TicketStopText;
#[derive(Component)]
pub struct TicketTargetText;
#[derive(Component)]
pub struct TicketNameText;

#[derive(Resource)]
pub struct Ticket {
    pub stop: usize,
    pub target: usize,
    /// What the relay said to the last order sent from this tab.
    sent: Arc<Mutex<Option<String>>>,
    pub last_sent: String,
    /// The name this browser's user chose (Change name or `?trader=`), if any.
    pub name: Option<String>,
    /// The name last sent to the desk, so a returning user's name is sent once, not every frame.
    name_sent: Option<String>,
}

impl Default for Ticket {
    fn default() -> Self {
        Ticket {
            stop: 1,
            target: 2,
            sent: Arc::new(Mutex::new(None)),
            last_sent: String::new(),
            name: stored_name(),
            name_sent: None,
        }
    }
}

#[cfg(target_arch = "wasm32")]
const NAME_KEY: &str = "opencry.ticket_trader";

/// A name typed by a user: trimmed, at most 32 characters, `None` if blank. Pure, for tests.
pub fn clean_name(raw: &str) -> Option<String> {
    let n = raw.trim();
    (!n.is_empty()).then(|| n.chars().take(32).collect())
}

/// `?trader=` first (and remembered), else what this browser saved.
fn stored_name() -> Option<String> {
    #[cfg(target_arch = "wasm32")]
    {
        let storage = web_sys::window().and_then(|w| w.local_storage().ok().flatten());
        if let Some(n) = crate::endpoints::page_param("trader")
            .as_deref()
            .and_then(clean_name)
        {
            if let Some(s) = &storage {
                let _ = s.set_item(NAME_KEY, &n);
            }
            return Some(n);
        }
        storage
            .and_then(|s| s.get_item(NAME_KEY).ok().flatten())
            .as_deref()
            .and_then(clean_name)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        None
    }
}

/// Open the page's name box (`__opencryAskName` in `index.html`). It does not block: the name
/// typed arrives later through [`take_chosen_name`]. `false` natively or without the box.
fn open_name_box(current: &str) -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen::{JsCast, JsValue};
        let Some(w) = web_sys::window() else {
            return false;
        };
        let Ok(f) = js_sys::Reflect::get(&w, &JsValue::from_str("__opencryAskName")) else {
            return false;
        };
        let Some(f) = f.dyn_ref::<js_sys::Function>() else {
            return false;
        };
        f.call1(&JsValue::NULL, &JsValue::from_str(current)).is_ok()
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = current;
        false
    }
}

/// The name saved in the page's name box since the last call, if any (it also saved it to
/// this browser's storage).
fn take_chosen_name() -> Option<String> {
    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen::JsValue;
        let w = web_sys::window()?;
        let key = JsValue::from_str("__opencryNameChosen");
        let v = js_sys::Reflect::get(&w, &key).ok()?.as_string()?;
        let _ = js_sys::Reflect::set(&w, &key, &JsValue::UNDEFINED);
        clean_name(&v)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        None
    }
}

/// The name the ticket shows: the running desk's, else this browser's, else the author's.
pub fn shown_trader<'a>(
    board: Option<&'a crate::desk_feed::DeskBoard>,
    local: Option<&'a str>,
) -> &'a str {
    board
        .and_then(|b| b.ticket_trader.as_deref())
        .filter(|n| !n.is_empty())
        .or(local)
        .unwrap_or(DEFAULT_TICKET_TRADER)
}

/// The order body for the relay. Pure, for tests.
pub fn order_body(
    action: &str,
    inst: Instrument,
    stop_atr: f32,
    target_r: f32,
) -> serde_json::Value {
    serde_json::json!({"action": action, "symbol": tape_symbol(inst), "stop_atr": stop_atr, "target_r": target_r, "from": "ticket", "account": "paper"})
}

/// Post one body to the relay's order queue; `what` names it in the status line.
fn post_order(body: serde_json::Value, what: String, sent: Arc<Mutex<Option<String>>>) {
    let mut req = ehttp::Request::post(
        format!("{}/paper/order", relay_url()),
        serde_json::to_vec(&body).unwrap_or_default(),
    );
    req.headers = ehttp::Headers::new(&[
        ("Accept", "application/json"),
        ("Content-Type", "application/json"),
    ]);
    let rename = body.get("action").and_then(|a| a.as_str()) == Some("name");
    ehttp::fetch(req, move |res| {
        let note = match res {
            Ok(r) if r.ok && rename => {
                format!("{what}: sent; the desk renames your seat within a second or two")
            }
            Ok(r) if r.ok => {
                let seq = serde_json::from_slice::<serde_json::Value>(&r.bytes)
                    .ok()
                    .and_then(|v| v.get("seq").and_then(|s| s.as_u64()))
                    .unwrap_or(0);
                format!("{what} queued as #{seq}; the desk fills it at the next tick")
            }
            Ok(r) => format!("{what}: relay said HTTP {}", r.status),
            Err(e) => format!("{what}: relay unreachable ({e})"),
        };
        if let Ok(mut s) = sent.lock() {
            *s = Some(note);
        }
    });
}

fn button(
    parent: &mut ChildSpawnerCommands,
    which: TicketButton,
    label: &str,
    bg: Color,
    border: Color,
    width: f32,
) {
    parent
        .spawn((
            Button,
            which,
            Node {
                width: Val::Px(width),
                height: Val::Px(30.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(Val::Px(4.0)),
                ..default()
            },
            BackgroundColor(bg),
            BorderColor::all(border),
        ))
        .with_children(|b| {
            let mut t = b.spawn((
                Text::new(label),
                TextFont {
                    font_size: 12.0,
                    ..default()
                },
                TextColor(Color::WHITE),
            ));
            match which {
                TicketButton::Stop => {
                    t.insert(TicketStopText);
                }
                TicketButton::Target => {
                    t.insert(TicketTargetText);
                }
                _ => {}
            }
        });
}

/// Startup: the ticket panel, in the right column under the board.
pub fn setup_ticket_panel(mut commands: Commands) {
    commands
        .spawn((
            TicketPanel,
            Panel(PanelId::Ticket),
            Node {
                width: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                padding: UiRect::axes(Val::Px(10.0), Val::Px(6.0)),
                row_gap: Val::Px(5.0),
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
                        TicketTitle,
                        Text::new("TRADE"),
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
            panel.spawn((
                TicketQuote,
                Text::new(""),
                TextFont {
                    font_size: 13.0,
                    ..default()
                },
                TextColor(Color::WHITE),
            ));
            panel
                .spawn(Node {
                    flex_direction: FlexDirection::Row,
                    column_gap: Val::Px(6.0),
                    ..default()
                })
                .with_children(|row| {
                    button(
                        row,
                        TicketButton::Buy,
                        "BUY",
                        Color::srgba(0.05, 0.40, 0.18, 0.95),
                        Color::srgb(0.30, 0.95, 0.50),
                        110.0,
                    );
                    button(
                        row,
                        TicketButton::Sell,
                        "SELL",
                        Color::srgba(0.45, 0.06, 0.08, 0.95),
                        Color::srgb(1.0, 0.35, 0.35),
                        110.0,
                    );
                    button(
                        row,
                        TicketButton::Close,
                        "CLOSE",
                        Color::srgba(0.12, 0.14, 0.20, 0.95),
                        Color::srgba(0.55, 0.62, 0.75, 0.8),
                        90.0,
                    );
                });
            panel
                .spawn(Node {
                    flex_direction: FlexDirection::Row,
                    column_gap: Val::Px(6.0),
                    ..default()
                })
                .with_children(|row| {
                    let (bg, border) = (
                        Color::srgba(0.06, 0.09, 0.14, 0.9),
                        Color::srgba(0.18, 0.28, 0.40, 0.7),
                    );
                    button(row, TicketButton::Stop, "", bg, border, 100.0);
                    button(row, TicketButton::Target, "", bg, border, 100.0);
                });
            panel
                .spawn(Node {
                    flex_direction: FlexDirection::Row,
                    column_gap: Val::Px(8.0),
                    align_items: AlignItems::Center,
                    ..default()
                })
                .with_children(|row| {
                    row.spawn((
                        TicketNameText,
                        Text::new("Your name:"),
                        TextFont {
                            font_size: 12.0,
                            ..default()
                        },
                        TextColor(Color::srgb(0.85, 0.90, 0.96)),
                    ));
                    button(
                        row,
                        TicketButton::Name,
                        "Change name",
                        Color::srgba(0.05, 0.22, 0.12, 0.95),
                        Color::srgb(0.35, 0.85, 0.50),
                        110.0,
                    );
                });
            panel.spawn((
                TicketStatus,
                Text::new(""),
                TextFont {
                    font_size: 10.0,
                    ..default()
                },
                TextColor(Color::srgb(0.75, 0.80, 0.88)),
                Node {
                    max_width: Val::Px(420.0),
                    ..default()
                },
            ));
        });
}

/// System: the ticket's buttons. Buy, Sell and Close post to the relay; Stop and Target cycle.
pub fn handle_ticket_buttons(
    buttons: Query<(&Interaction, &TicketButton), (Changed<Interaction>, With<Button>)>,
    selection: Res<Selection>,
    time: Res<Time>,
    desk: Res<DeskFeed>,
    mut ticket: ResMut<Ticket>,
) {
    let board = desk.live_board(time.elapsed_secs_f64());
    if let Some(n) = take_chosen_name() {
        ticket.name = Some(n.clone());
        ticket.name_sent = Some(n.clone());
        ticket.last_sent = format!("Renaming you to {n}...");
        post_order(
            serde_json::json!({"action": "name", "trader": n, "from": "ticket"}),
            format!("Name {n}"),
            ticket.sent.clone(),
        );
    }
    // A name chosen in this browser (or with ?trader=) that the running desk does not have yet:
    // send it once, so a returning user is themselves again without clicking anything.
    if let (Some(b), Some(n)) = (board, ticket.name.clone())
        && b.ticket_trader.as_deref() != Some(n.as_str())
        && ticket.name_sent.as_deref() != Some(n.as_str())
    {
        ticket.name_sent = Some(n.clone());
        post_order(
            serde_json::json!({"action": "name", "trader": n, "from": "ticket"}),
            format!("Name {n}"),
            ticket.sent.clone(),
        );
    }
    for (i, which) in buttons.iter() {
        if *i != Interaction::Pressed {
            continue;
        }
        let action = match which {
            TicketButton::Name => {
                let current = shown_trader(board, ticket.name.as_deref()).to_string();
                if !open_name_box(&current) {
                    ticket.last_sent = if cfg!(target_arch = "wasm32") {
                        "Could not open the name box: add ?trader=Your%20Name to the address instead.".into()
                    } else {
                        "Natively, set OPENCRY_TRADER_NAME before starting the desk.".into()
                    };
                }
                continue;
            }
            TicketButton::Stop => {
                ticket.stop = (ticket.stop + 1) % STOPS.len();
                continue;
            }
            TicketButton::Target => {
                ticket.target = (ticket.target + 1) % TARGETS.len();
                continue;
            }
            TicketButton::Buy => "buy",
            TicketButton::Sell => "sell",
            TicketButton::Close => "close",
        };
        let Some(inst) = selection.selected_instrument else {
            ticket.last_sent = "Pick a pit first: the ticket trades the pit in view.".into();
            continue;
        };
        let mut body = order_body(action, inst, STOPS[ticket.stop], TARGETS[ticket.target]);
        if let Some(n) = &ticket.name {
            body["trader"] = serde_json::json!(n);
        }
        ticket.last_sent = format!("{} {} sent...", action.to_uppercase(), inst.symbol());
        post_order(
            body,
            format!("{} {}", action.to_uppercase(), inst.symbol()),
            ticket.sent.clone(),
        );
    }
}

/// System: the ticket's text, four times a second: the pit in view and its touch, your open
/// position (from the desk's board), the stop and target, and what became of your last order.
pub fn update_ticket_panel(
    time: Res<Time>,
    selection: Res<Selection>,
    feed: Res<LiveFeed>,
    desk: Res<DeskFeed>,
    mut ticket: ResMut<Ticket>,
    mut next: Local<f32>,
    mut texts: ParamSet<(
        Query<&mut Text, With<TicketTitle>>,
        Query<&mut Text, With<TicketQuote>>,
        Query<&mut Text, With<TicketStatus>>,
        Query<&mut Text, With<TicketStopText>>,
        Query<&mut Text, With<TicketTargetText>>,
        Query<&mut Text, With<TicketNameText>>,
    )>,
) {
    let taken = ticket.sent.lock().ok().and_then(|mut s| s.take());
    if let Some(note) = taken {
        ticket.last_sent = note;
    }
    if time.elapsed_secs() < *next {
        return;
    }
    *next = time.elapsed_secs() + 0.25;
    let inst = selection.selected_instrument;
    let board = desk.live_board(time.elapsed_secs_f64());
    for mut t in texts.p0().iter_mut() {
        set_ui_text(
            &mut t,
            match inst {
                Some(i) => format!("TRADE  {}  |  paper", i.symbol()),
                None => "TRADE  |  pick a pit".to_string(),
            },
        );
    }
    for mut t in texts.p1().iter_mut() {
        let line = inst
            .and_then(|i| feed.quote(i).map(|q| (i, q)))
            .map(|(i, q)| {
                let d = instrument_decimals(i);
                format!(
                    "bid {}   ask {}",
                    format_price_with_commas(q.price, d),
                    format_price_with_commas(q.ask, d)
                )
            });
        set_ui_text(&mut t, line.unwrap_or_else(|| "no price yet".into()));
    }
    for mut t in texts.p5().iter_mut() {
        set_ui_text(
            &mut t,
            format!("Your name: {}", shown_trader(board, ticket.name.as_deref())),
        );
    }
    for mut t in texts.p3().iter_mut() {
        set_ui_text(&mut t, format!("Stop {} ATR", STOPS[ticket.stop]));
    }
    for mut t in texts.p4().iter_mut() {
        set_ui_text(&mut t, format!("Target {} R", TARGETS[ticket.target]));
    }
    let mut lines = Vec::new();
    match board {
        None => lines.push("The desk is not running: orders will not be filled.".to_string()),
        Some(b) => {
            match b
                .traders
                .iter()
                .find(|t| t.trader == ticket_trader(Some(b)))
            {
                Some(me) => {
                    match &me.open {
                        Some(o) => lines.push(format!(
                            "OPEN {} {} from {}  stop {}  {:+.2}R",
                            o.side, o.symbol, o.entry, o.stop, o.open_r
                        )),
                        None => lines.push("Flat.".into()),
                    }
                    lines.push(format!(
                        "Today: {} trades, {:+.2}R",
                        me.day_trades, me.day_r
                    ));
                }
                None => lines.push("Flat.".into()),
            }
            if let Some(m) = &b.manual {
                lines.push(format!("Desk: {m}"));
            }
        }
    }
    if !ticket.last_sent.is_empty() {
        lines.push(ticket.last_sent.clone());
    }
    for mut t in texts.p2().iter_mut() {
        set_ui_text(&mut t, lines.join("\n"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_order_names_the_pit_by_its_tape_symbol_with_the_ticket_settings() {
        let b = order_body("buy", Instrument::Gold, 1.5, 2.0);
        assert_eq!(
            (
                b["action"].as_str(),
                b["symbol"].as_str(),
                b["stop_atr"].as_f64(),
                b["target_r"].as_f64()
            ),
            (Some("buy"), Some("XAUUSD"), Some(1.5), Some(2.0))
        );
    }

    #[test]
    fn the_traders_name_is_the_authors_unless_the_desk_names_another() {
        assert_eq!(trader_name_from(None), "Guest");
        assert_eq!(trader_name_from(Some("  ")), "Guest");
        assert_eq!(trader_name_from(Some(" Ada Lovelace ")), "Ada Lovelace");
        assert_eq!(trader_name_from(Some(&"x".repeat(50))).len(), 32);
        let mut b = crate::desk_feed::DeskBoard::default();
        assert_eq!(ticket_trader(Some(&b)), "Guest");
        b.ticket_trader = Some("Ada Lovelace".into());
        assert_eq!(ticket_trader(Some(&b)), "Ada Lovelace");
        assert_eq!(
            shown_trader(Some(&b), Some("Grace")),
            "Ada Lovelace",
            "the running desk's name wins"
        );
        assert_eq!(shown_trader(None, Some("Grace")), "Grace");
        assert_eq!(shown_trader(None, None), "Guest");
        assert_eq!(
            clean_name("  Grace Hopper  ").as_deref(),
            Some("Grace Hopper")
        );
        assert_eq!(clean_name("   "), None);
    }

    #[test]
    fn the_ticket_and_positions_systems_build_without_conflicting_queries() {
        // A conflicting query is a panic when the page starts (a blank page): build both here.
        use bevy::ecs::system::RunSystemOnce;
        let mut world = World::new();
        world.insert_resource(Time::<()>::default());
        world.insert_resource(Selection::default());
        world.insert_resource(LiveFeed::default());
        world.insert_resource(DeskFeed::default());
        world.insert_resource(Ticket::default());
        world.insert_resource(crate::positions::PositionsFeed::default());
        world.run_system_once(update_ticket_panel).unwrap();
        world.run_system_once(handle_ticket_buttons).unwrap();
        world
            .run_system_once(crate::positions::update_positions_panel)
            .unwrap();
    }
}
