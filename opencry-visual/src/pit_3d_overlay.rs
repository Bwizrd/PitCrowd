//! 3D pit overlay: a price board and floating deal values, drawn as UI nodes projected
//! from world positions. Everything shown here is an engine event or signal.

use bevy::prelude::*;

use crate::camera::{FloorViewMode, OrbitCamera, PitCamera3d};
use crate::candles::ChartSettings;
use crate::market::{District, Instrument};
use crate::panels::{BoardExpandText, Panel, PanelId, spawn_close_button};
use crate::pit::{format_price_with_commas, instrument_decimals, pit_state_label, set_ui_text};
use crate::pit_3d::{Pit3dCrowdMember, district_sign_origin, pit_3d_origin};
use crate::pit_engine::{PitEngines, PitEventKind, PitMarket, Side, contract_multiplier};
use crate::tape_chart::{
    BOARD_PAD_X, BOARD_PANEL_W, PriceArrow, set_arrow, spawn_price_arrows, spawn_tape_chart,
    touch_lines,
};
use crate::ui::{Selection, TouchButtonAction};

/// The board panel's root: a `Panel(PanelId::Board)` in the right column. `panels::sync_panels`
/// shows it in the 3D pit view when the board is open and sizes it from `ChartLayout`.
#[derive(Component)]
pub struct Pit3dOverlayRoot;

#[derive(Component)]
pub struct Pit3dBoardInstrument;

#[derive(Component)]
pub struct Pit3dBoardPrice;

#[derive(Component)]
pub struct Pit3dBoardTouch;

#[derive(Component)]
pub struct Pit3dBoardTape;

#[derive(Component)]
pub struct Pit3dBoardState;

/// Projected nameplate over a pit: instrument, last print, state. The market map when zoomed out.
#[derive(Component, Debug, Clone, Copy)]
pub struct Pit3dNameplate {
    pub instrument: Instrument,
}

#[derive(Component)]
pub struct Pit3dNameplatePrice;

/// Which pit a nameplate price text belongs to (it sits inside a row, not directly under the plate).
#[derive(Component, Clone, Copy)]
pub struct NameplatePriceOf(pub Instrument);

/// Arrow beside a nameplate price; tagged with its pit.
#[derive(Component, Clone, Copy)]
pub struct NameplateArrow(pub Instrument);

/// Arrow beside the board price.
#[derive(Component)]
pub struct BoardArrow;

#[derive(Component)]
pub struct Pit3dNameplateState;

/// A label anchored to a world position that rises and fades.
#[derive(Component, Debug, Clone)]
pub struct FloatingDealLabel {
    pub world_pos: Vec3,
    pub age: f32,
    pub lifetime: f32,
    pub rise_speed: f32,
}

/// Pure helper: short money format for notionals ("$1.2M", "$340k", "$9,800").
pub fn format_money(value: f64) -> String {
    let abs = value.abs();
    if abs >= 1.0e9 {
        format!("${:.2}B", value / 1.0e9)
    } else if abs >= 1.0e6 {
        format!("${:.1}M", value / 1.0e6)
    } else if abs >= 1.0e4 {
        format!("${:.0}k", value / 1.0e3)
    } else {
        format!("${}", format_price_with_commas(value, 0))
    }
}

/// Pure helper: the two lines shown over a fill. Aggressor's side names the trade.
pub fn deal_label_text(
    pit: &PitMarket,
    side: Side,
    qty: u32,
    price_ticks: i32,
) -> (String, String) {
    let price = pit.price_of(price_ticks);
    let price_str = format_price_with_commas(price as f64, instrument_decimals(pit.instrument));
    let verb = if side.is_buy() { "BOUGHT" } else { "SOLD" };
    let notional = qty as f64 * contract_multiplier(pit.instrument) as f64 * price as f64;
    (
        format!("{verb} {qty} @ {price_str}"),
        format_money(notional),
    )
}

/// Startup: price board, and one nameplate per pit, shown only in 3D.
/// A district's name standing on the concourse behind its row of pits.
#[derive(Component)]
pub struct DistrictSign(pub District);

pub fn setup_pit_3d_overlay(mut commands: Commands) {
    for d in District::ALL {
        commands
            .spawn((
                DistrictSign(d),
                Node {
                    position_type: PositionType::Absolute,
                    padding: UiRect::axes(Val::Px(14.0), Val::Px(2.0)),
                    border: UiRect::all(Val::Px(1.0)),
                    border_radius: BorderRadius::all(Val::Px(3.0)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.02, 0.03, 0.06, 0.70)),
                BorderColor::all(d.colour().with_alpha(0.7)),
                Visibility::Hidden,
                GlobalZIndex(-2),
            ))
            .with_children(|p| {
                p.spawn((
                    Text::new(d.sign()),
                    TextFont {
                        font_size: 22.0,
                        ..default()
                    },
                    TextColor(d.colour()),
                ));
            });
    }
    for inst in Instrument::ALL {
        commands
            .spawn((
                Pit3dNameplate { instrument: inst },
                Node {
                    position_type: PositionType::Absolute,
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    padding: UiRect::axes(Val::Px(8.0), Val::Px(3.0)),
                    border: UiRect::all(Val::Px(1.0)),
                    border_radius: BorderRadius::all(Val::Px(4.0)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.02, 0.03, 0.06, 0.80)),
                BorderColor::all(inst.accent_color().with_alpha(0.8)),
                Visibility::Hidden,
                GlobalZIndex(-1),
            ))
            .with_children(|p| {
                p.spawn((
                    Text::new(inst.name()),
                    TextFont {
                        font_size: 11.0,
                        ..default()
                    },
                    TextColor(inst.accent_color()),
                ));
                p.spawn(Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: Val::Px(5.0),
                    ..default()
                })
                .with_children(|row| {
                    row.spawn((
                        Pit3dNameplatePrice,
                        NameplatePriceOf(inst),
                        Text::new("--"),
                        TextFont {
                            font_size: 16.0,
                            ..default()
                        },
                        TextColor(Color::srgb(1.0, 0.85, 0.25)),
                    ));
                    spawn_price_arrows(row, 1.5);
                });
                p.spawn((
                    Pit3dNameplateState,
                    Text::new(""),
                    TextFont {
                        font_size: 10.0,
                        ..default()
                    },
                    TextColor(Color::srgb(0.70, 0.76, 0.84)),
                ));
            });
    }

    commands
        .spawn((
            Pit3dOverlayRoot,
            Panel(PanelId::Board),
            Node {
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                padding: UiRect::axes(Val::Px(BOARD_PAD_X), Val::Px(8.0)),
                border: UiRect::all(Val::Px(1.0)),
                // Fixed: the chart sets the width and text wraps to it (`sync_panels` widens
                // it to the viewport while the board is expanded).
                width: Val::Px(BOARD_PANEL_W),
                border_radius: BorderRadius::all(Val::Px(6.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.02, 0.03, 0.06, 1.0)),
            BorderColor::all(Color::srgba(0.85, 0.70, 0.25, 0.7)),
        ))
        .with_children(|board| {
            let me = board.target_entity();
            // Header: instrument line, Expand/Restore, close.
            board
                .spawn(Node {
                    flex_direction: FlexDirection::Row,
                    justify_content: JustifyContent::SpaceBetween,
                    align_items: AlignItems::Center,
                    width: Val::Percent(100.0),
                    column_gap: Val::Px(6.0),
                    ..default()
                })
                .with_children(|row| {
                    row.spawn((
                        Pit3dBoardInstrument,
                        Text::new("SILVER  XAG/USD"),
                        TextFont {
                            font_size: 13.0,
                            ..default()
                        },
                        TextColor(Color::srgb(0.85, 0.80, 0.60)),
                        Node {
                            flex_grow: 1.0,
                            flex_shrink: 1.0,
                            min_width: Val::Px(0.0),
                            ..default()
                        },
                    ));
                    row.spawn((
                        Button,
                        TouchButtonAction::BoardExpand,
                        Node {
                            padding: UiRect::axes(Val::Px(6.0), Val::Px(1.5)),
                            border: UiRect::all(Val::Px(1.0)),
                            border_radius: BorderRadius::all(Val::Px(3.0)),
                            flex_shrink: 0.0,
                            ..default()
                        },
                        BackgroundColor(Color::srgba(0.06, 0.09, 0.14, 0.85)),
                        BorderColor::all(Color::srgba(0.18, 0.28, 0.40, 0.55)),
                    ))
                    .with_children(|b| {
                        b.spawn((
                            BoardExpandText,
                            Text::new("EXPAND [E]"),
                            TextFont {
                                font_size: 9.5,
                                ..default()
                            },
                            TextColor(Color::srgb(0.95, 0.88, 0.35)),
                        ));
                    });
                    spawn_close_button(row, me);
                });
            board
                .spawn(Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: Val::Px(10.0),
                    ..default()
                })
                .with_children(|row| {
                    row.spawn((
                        Pit3dBoardPrice,
                        Text::new("--"),
                        TextFont {
                            font_size: 34.0,
                            ..default()
                        },
                        TextColor(Color::srgb(1.0, 0.85, 0.25)),
                    ));
                    spawn_price_arrows(row, 3.0);
                });
            board.spawn((
                Pit3dBoardTouch,
                Text::new("-- / --"),
                TextFont {
                    font_size: 13.0,
                    ..default()
                },
                TextColor(Color::srgb(0.75, 0.80, 0.88)),
            ));
            board.spawn((
                Pit3dBoardTape,
                Text::new(""),
                TextFont {
                    font_size: 12.0,
                    ..default()
                },
                TextColor(Color::srgb(0.60, 0.66, 0.74)),
            ));
            board.spawn((
                Pit3dBoardState,
                Text::new(""),
                TextFont {
                    font_size: 12.0,
                    ..default()
                },
                TextColor(Color::srgb(0.95, 0.80, 0.35)),
            ));
            board.spawn((
                crate::jev::Pit3dBoardJev,
                Text::new("JEV: no read yet"),
                TextFont {
                    font_size: 12.0,
                    ..default()
                },
                TextColor(Color::srgb(0.55, 0.58, 0.66)),
            ));
            spawn_tape_chart(board);
        });
}

/// Startup (after `setup_pit_3d_overlay`): tags each nameplate's arrows with its pit, and the
/// board's with `BoardArrow`, so the update systems can find them without walking children.
pub fn tag_price_arrows(
    mut commands: Commands,
    plates: Query<(Entity, &Pit3dNameplate)>,
    arrows: Query<Entity, With<PriceArrow>>,
    parents: Query<&ChildOf>,
) {
    for arrow in arrows.iter() {
        let mut tagged = false;
        let mut cur = arrow;
        while let Ok(parent) = parents.get(cur) {
            cur = parent.parent();
            if let Ok((_, plate)) = plates.get(cur) {
                commands
                    .entity(arrow)
                    .insert(NameplateArrow(plate.instrument));
                tagged = true;
                break;
            }
        }
        if !tagged {
            commands.entity(arrow).insert(BoardArrow);
        }
    }
}

/// System: spawns floating labels for this frame's fills and prints in the inspected pit.
pub fn spawn_3d_deal_labels(
    mut commands: Commands,
    view_mode: Res<FloorViewMode>,
    engines: Res<PitEngines>,
    selection: Res<Selection>,
    camera: Query<&OrbitCamera, With<PitCamera3d>>,
    crowd: Query<(&Pit3dCrowdMember, &Transform)>,
) {
    // In market view the nameplates are the signal; per-fill labels would pile onto one pit.
    if !view_mode.is_3d() || camera.single().map(|o| o.market_view).unwrap_or(false) {
        return;
    }
    let Some(inst) = selection.selected_instrument else {
        return;
    };
    let pit = engines.pit(inst);
    if pit.recent_events().is_empty() {
        return;
    }
    let mut positions = [None::<Vec3>; crate::pit_engine::SLOTS];
    for (member, transform) in crowd.iter() {
        if member.crowd_index < positions.len() {
            positions[member.crowd_index] = Some(transform.translation);
        }
    }

    for ev in pit.recent_events() {
        match ev.kind {
            PitEventKind::Fill {
                aggressor,
                side,
                price_ticks,
                qty,
                ..
            } => {
                let Some(pos) = positions.get(aggressor).copied().flatten() else {
                    continue;
                };
                let (line1, line2) = deal_label_text(pit, side, qty, price_ticks);
                let color = if side.is_buy() {
                    Color::srgb(0.25, 1.0, 0.45)
                } else {
                    Color::srgb(1.0, 0.32, 0.32)
                };
                spawn_label(
                    &mut commands,
                    pos + Vec3::new(0.0, 13.0, 0.0),
                    &line1,
                    Some(&line2),
                    color,
                    2.6,
                );
            }
            PitEventKind::Print { price_ticks, up } => {
                let Some(pos) = positions.get(1).copied().flatten() else {
                    continue;
                };
                let price = format_price_with_commas(
                    pit.price_of(price_ticks) as f64,
                    instrument_decimals(inst),
                );
                let text = format!("{price} {}", if up { "+" } else { "-" });
                spawn_label(
                    &mut commands,
                    pos + Vec3::new(0.0, 14.0, 0.0),
                    &text,
                    None,
                    Color::srgb(1.0, 0.85, 0.25),
                    2.0,
                );
            }
            PitEventKind::OutTrade { b, .. } => {
                let Some(pos) = positions.get(b).copied().flatten() else {
                    continue;
                };
                spawn_label(
                    &mut commands,
                    pos + Vec3::new(0.0, 13.0, 0.0),
                    "OUT-TRADE!",
                    None,
                    Color::srgb(1.0, 0.55, 0.15),
                    2.2,
                );
            }
            _ => {}
        }
    }
}

fn spawn_label(
    commands: &mut Commands,
    world_pos: Vec3,
    line1: &str,
    line2: Option<&str>,
    color: Color,
    lifetime: f32,
) {
    commands
        .spawn((
            FloatingDealLabel {
                world_pos,
                age: 0.0,
                lifetime,
                rise_speed: 3.2,
            },
            Node {
                position_type: PositionType::Absolute,
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                padding: UiRect::axes(Val::Px(8.0), Val::Px(3.0)),
                border_radius: BorderRadius::all(Val::Px(4.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.02, 0.03, 0.06, 0.82)),
            // Floor-attached: stays under the panel columns (z 4/5) and the detail panel (6).
            GlobalZIndex(-1),
        ))
        .with_children(|p| {
            p.spawn((
                Text::new(line1),
                TextFont {
                    font_size: 15.0,
                    ..default()
                },
                TextColor(color),
            ));
            if let Some(l2) = line2 {
                p.spawn((
                    Text::new(l2),
                    TextFont {
                        font_size: 12.0,
                        ..default()
                    },
                    TextColor(Color::srgb(0.95, 0.88, 0.55)),
                ));
            }
        });
}

/// System: board text, label projection, rise and fade. The board panel's own show/hide is
/// `panels::sync_panels`; the text is skipped while it is closed or the market view is up.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn update_pit_3d_overlay(
    mut commands: Commands,
    time: Res<Time>,
    feed: Res<crate::live_feed::LiveFeed>,
    view_mode: Res<FloorViewMode>,
    engines: Res<PitEngines>,
    selection: Res<Selection>,
    settings: Res<ChartSettings>,
    camera: Query<(&Camera, &GlobalTransform, &OrbitCamera), With<PitCamera3d>>,
    mut labels: Query<(
        Entity,
        &mut FloatingDealLabel,
        &mut Node,
        &mut BackgroundColor,
        &Children,
    )>,
    mut texts: Query<&mut TextColor, Without<Pit3dBoardPrice>>,
    mut board_inst: Query<
        &mut Text,
        (
            With<Pit3dBoardInstrument>,
            Without<Pit3dBoardPrice>,
            Without<Pit3dBoardTouch>,
            Without<Pit3dBoardTape>,
            Without<Pit3dBoardState>,
        ),
    >,
    mut board_price: Query<
        (&mut Text, &mut TextColor),
        (
            With<Pit3dBoardPrice>,
            Without<Pit3dBoardInstrument>,
            Without<Pit3dBoardTouch>,
            Without<Pit3dBoardTape>,
            Without<Pit3dBoardState>,
            Without<FloatingDealLabel>,
        ),
    >,
    mut board_touch: Query<
        &mut Text,
        (
            With<Pit3dBoardTouch>,
            Without<Pit3dBoardInstrument>,
            Without<Pit3dBoardPrice>,
            Without<Pit3dBoardTape>,
            Without<Pit3dBoardState>,
        ),
    >,
    mut board_tape: Query<
        &mut Text,
        (
            With<Pit3dBoardTape>,
            Without<Pit3dBoardInstrument>,
            Without<Pit3dBoardPrice>,
            Without<Pit3dBoardTouch>,
            Without<Pit3dBoardState>,
        ),
    >,
    mut board_state: Query<
        &mut Text,
        (
            With<Pit3dBoardState>,
            Without<Pit3dBoardInstrument>,
            Without<Pit3dBoardPrice>,
            Without<Pit3dBoardTouch>,
            Without<Pit3dBoardTape>,
        ),
    >,
    mut board_arrows: Query<
        (&PriceArrow, &mut Node),
        (With<BoardArrow>, Without<FloatingDealLabel>),
    >,
) {
    let is_3d = view_mode.is_3d();
    let market_view = camera
        .single()
        .map(|(_, _, o)| o.market_view)
        .unwrap_or(false);
    if !is_3d {
        for (entity, ..) in labels.iter() {
            commands.entity(entity).despawn();
        }
        return;
    }

    // Board (in market view the nameplates carry the numbers and the board is hidden).
    let board_shown = settings.board_chart && !market_view;
    if let Some(inst) = selection.selected_instrument.filter(|_| board_shown) {
        let pit = engines.pit(inst);
        let sig = &pit.signals;
        let decimals = instrument_decimals(inst);
        let fmt = |ticks: i32| format_price_with_commas(pit.price_of(ticks) as f64, decimals);
        let prints: Vec<(i32, bool)> = pit
            .events
            .iter()
            .rev()
            .filter_map(|e| match e.kind {
                PitEventKind::Print { price_ticks, up } => Some((price_ticks, up)),
                _ => None,
            })
            .take(8)
            .collect();
        let last_up = prints.first().map(|p| p.1);

        let status = feed.pit_status(inst, time.elapsed_secs_f64());
        let halted = crate::live_feed::halt_label(status);
        for mut t in board_inst.iter_mut() {
            let tag = match status {
                crate::live_feed::PitFeedStatus::Simulated => "SIM".to_string(),
                crate::live_feed::PitFeedStatus::Live { rate, .. } => {
                    let tape = feed
                        .quote(inst)
                        .map(|q| q.flow(time.elapsed_secs_f64()))
                        .unwrap_or_default();
                    format!(
                        "LIVE {rate:.1} t/s  |  {}",
                        crate::live_feed::tape_label(&tape)
                    )
                }
                _ => "HALTED".to_string(),
            };
            set_ui_text(
                &mut t,
                format!(
                    "{}  {}  |  {} in  |  {tag}",
                    inst.name(),
                    inst.symbol(),
                    pit.active_count
                ),
            );
        }
        for (arrow, mut node) in board_arrows.iter_mut() {
            set_arrow(arrow, &mut node, last_up);
        }
        for (mut t, mut c) in board_price.iter_mut() {
            // Live: the board shows the feed's own price, not the pit's print of it.
            let shown = match (status, feed.quote(inst)) {
                (crate::live_feed::PitFeedStatus::Live { .. }, Some(q)) => {
                    format_price_with_commas(q.price, decimals)
                }
                _ => fmt(sig.last_ticks),
            };
            set_ui_text(&mut t, shown);
            let new_color = if halted.is_some() {
                Color::srgb(0.55, 0.55, 0.55)
            } else {
                match last_up {
                    Some(true) => Color::srgb(0.25, 1.0, 0.45),
                    Some(false) => Color::srgb(1.0, 0.32, 0.32),
                    None => Color::srgb(1.0, 0.85, 0.25),
                }
            };
            if c.0 != new_color {
                *c = TextColor(new_color);
            }
        }
        for mut t in board_touch.iter_mut() {
            if let Some(halt) = &halted {
                set_ui_text(&mut t, format!("HALTED: {halt}"));
            } else if let (crate::live_feed::PitFeedStatus::Live { .. }, Some(q)) =
                (status, feed.quote(inst))
            {
                // Live: the real touch and the broker's ladder sizes, not the pit's quotes.
                let spread_ticks =
                    (q.spread / crate::pit_engine::tick_size(inst) as f64).round() as i64;
                set_ui_text(
                    &mut t,
                    touch_lines(
                        &format_price_with_commas(q.price, decimals),
                        &format_price_with_commas(q.ask, decimals),
                        spread_ticks,
                        &q.bids,
                        &q.asks,
                    ),
                );
            } else {
                let bid = sig.best_bid_ticks.map(fmt).unwrap_or_else(|| "--".into());
                let ask = sig.best_ask_ticks.map(fmt).unwrap_or_else(|| "--".into());
                set_ui_text(&mut t, format!("BID {bid}   ASK {ask}"));
            }
        }
        for mut t in board_tape.iter_mut() {
            let tape: Vec<String> = prints
                .iter()
                .rev()
                .map(|(px, up)| format!("{}{}", fmt(*px), if *up { "+" } else { "-" }))
                .collect();
            set_ui_text(
                &mut t,
                if tape.is_empty() {
                    "tape: no prints yet".to_string()
                } else {
                    format!("tape: {}", tape.join("  "))
                },
            );
        }
        for mut t in board_state.iter_mut() {
            set_ui_text(
                &mut t,
                format!(
                    "{}  |  sound {}%  paper {:+}%  filled {:+}%",
                    pit_state_label(sig),
                    (sig.sound * 100.0).round() as i32,
                    (sig.pressure * 100.0).round() as i32,
                    (sig.imbalance * 100.0).round() as i32
                ),
            );
        }
    }

    // Labels.
    let Ok((cam, cam_tf, _)) = camera.single() else {
        return;
    };
    let dt = time.delta_secs();
    for (entity, mut label, mut node, mut bg, children) in labels.iter_mut() {
        label.age += dt;
        if label.age >= label.lifetime {
            commands.entity(entity).despawn();
            continue;
        }
        let progress = label.age / label.lifetime;
        let alpha = (1.0 - progress * progress).clamp(0.0, 1.0);
        let world = label.world_pos + Vec3::new(0.0, label.rise_speed * label.age, 0.0);
        match cam.world_to_viewport(cam_tf, world) {
            Ok(vp) => {
                node.left = Val::Px(vp.x - 60.0);
                node.top = Val::Px(vp.y - 14.0);
                node.display = Display::Flex;
            }
            Err(_) => {
                node.display = Display::None;
            }
        }
        bg.0 = bg.0.with_alpha(0.82 * alpha);
        for child in children.iter() {
            if let Ok(mut tc) = texts.get_mut(child) {
                tc.0 = tc.0.with_alpha(alpha);
            }
        }
    }
}

/// System: projects nameplates over every pit and fills them from the engine.
pub fn update_pit_3d_nameplates(
    time: Res<Time>,
    feed: Res<crate::live_feed::LiveFeed>,
    view_mode: Res<FloorViewMode>,
    engines: Res<PitEngines>,
    selection: Res<Selection>,
    camera: Query<(&Camera, &GlobalTransform, &OrbitCamera), With<PitCamera3d>>,
    mut plates: Query<(
        &Pit3dNameplate,
        &mut Node,
        &mut Visibility,
        &mut BorderColor,
        &Children,
    )>,
    mut prices: Query<
        (&NameplatePriceOf, &mut Text, &mut TextColor),
        (With<Pit3dNameplatePrice>, Without<Pit3dNameplateState>),
    >,
    mut states: Query<&mut Text, (With<Pit3dNameplateState>, Without<Pit3dNameplatePrice>)>,
    mut arrows: Query<(&PriceArrow, &NameplateArrow, &mut Node), Without<Pit3dNameplate>>,
    mut signs: Query<
        (&DistrictSign, &mut Node, &mut Visibility, &ComputedNode),
        (Without<Pit3dNameplate>, Without<NameplateArrow>),
    >,
) {
    let is_3d = view_mode.is_3d();
    let cam = camera.single().ok();
    // District signs: projected from their floor spot, centred on it; hidden off screen or in 2D.
    for (sign, mut node, mut vis, computed) in signs.iter_mut() {
        let placed = if is_3d {
            cam.and_then(|(cam, cam_tf, _)| {
                cam.world_to_viewport(
                    cam_tf,
                    district_sign_origin(sign.0) + Vec3::new(0.0, 3.0, 0.0),
                )
                .ok()
            })
        } else {
            None
        };
        match placed {
            Some(vp) => {
                let size = computed.size() * computed.inverse_scale_factor();
                node.left = Val::Px(vp.x - size.x * 0.5);
                node.top = Val::Px(vp.y - size.y * 0.5);
                if *vis != Visibility::Inherited {
                    *vis = Visibility::Inherited;
                }
            }
            None => {
                if *vis != Visibility::Hidden {
                    *vis = Visibility::Hidden;
                }
            }
        }
    }
    let mut last_up_by_pit = [None::<bool>; crate::pit_engine::SLOTS];
    for (i, inst) in Instrument::ALL.iter().enumerate() {
        last_up_by_pit[i] = engines
            .pit(*inst)
            .events
            .iter()
            .rev()
            .find_map(|e| match e.kind {
                PitEventKind::Print { up, .. } => Some(up),
                _ => None,
            });
    }
    for (arrow, tag, mut node) in arrows.iter_mut() {
        set_arrow(arrow, &mut node, last_up_by_pit[tag.0 as usize]);
    }
    if is_3d {
        for (of, mut t, mut c) in prices.iter_mut() {
            let pit = engines.pit(of.0);
            set_ui_text(
                &mut t,
                format_price_with_commas(pit.last_price() as f64, instrument_decimals(of.0)),
            );
            let new_color = match last_up_by_pit[of.0 as usize] {
                Some(true) => Color::srgb(0.25, 1.0, 0.45),
                Some(false) => Color::srgb(1.0, 0.32, 0.32),
                None => Color::srgb(1.0, 0.85, 0.25),
            };
            if c.0 != new_color {
                *c = TextColor(new_color);
            }
        }
    }
    for (plate, mut node, mut vis, mut border, children) in plates.iter_mut() {
        if !is_3d {
            *vis = Visibility::Hidden;
            continue;
        }
        let Some((cam, cam_tf, orbit)) = cam else {
            continue;
        };
        let selected = selection.selected_instrument == Some(plate.instrument);
        if selected && !orbit.market_view {
            // The board already shows this pit; a plate here would sit on the traders.
            *vis = Visibility::Hidden;
            continue;
        }
        let pit = engines.pit(plate.instrument);
        let world = pit_3d_origin(plate.instrument) + Vec3::new(0.0, 24.0, 0.0);
        match cam.world_to_viewport(cam_tf, world) {
            Ok(vp) => {
                node.left = Val::Px(vp.x - 58.0);
                node.top = Val::Px(vp.y - 22.0);
                *vis = Visibility::Inherited;
            }
            Err(_) => {
                *vis = Visibility::Hidden;
                continue;
            }
        }
        let want = BorderColor::all(if selected {
            Color::srgb(1.0, 1.0, 1.0)
        } else {
            plate.instrument.accent_color().with_alpha(0.8)
        });
        if *border != want {
            *border = want;
        }

        let sig = &pit.signals;
        for child in children.iter() {
            if let Ok(mut t) = states.get_mut(child) {
                let status = feed.pit_status(plate.instrument, time.elapsed_secs_f64());
                if let Some(halt) = crate::live_feed::halt_label(status) {
                    set_ui_text(&mut t, format!("HALTED: {halt}"));
                } else {
                    let bars = "|".repeat(((sig.sound * 8.0).round() as usize).clamp(0, 8));
                    set_ui_text(
                        &mut t,
                        format!("{} {}  {}/18", pit_state_label(sig), bars, pit.active_count),
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::market::Instrument;
    use crate::pit_engine::PitMarket;

    #[test]
    fn money_formats_scale() {
        assert_eq!(format_money(9_800.0), "$9,800");
        assert_eq!(format_money(340_000.0), "$340k");
        assert_eq!(format_money(1_250_000.0), "$1.2M");
        assert_eq!(format_money(2_500_000_000.0), "$2.50B");
    }

    #[test]
    fn deal_label_names_side_size_price_and_notional() {
        let pit = PitMarket::new(Instrument::Silver, 1);
        let (l1, l2) = deal_label_text(&pit, Side::Sell, 25, 0);
        assert_eq!(l1, "SOLD 25 @ 31.850");
        // 25 contracts x 5,000 oz x 31.85
        assert_eq!(l2, "$4.0M");
        let (l1, _) = deal_label_text(&pit, Side::Buy, 3, 2);
        assert!(l1.starts_with("BOUGHT 3 @ 31.87"));
    }
}
