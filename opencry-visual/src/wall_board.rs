//! Option 3: a candlestick wall board floating above the selected pit in the 3D scene,
//! turned to face the camera. Candles are cuboids from a shared unit cube; axis labels are
//! UI nodes projected from the board's corners, like the nameplates.
//!
//! The labels are anchored, not offset: a price label ends a fixed gap before its corner
//! (its node is positioned from the right edge, so its width does not matter) and the title
//! is centred between the two bottom corners. Projection uses the camera's transform of
//! this frame, the same one the scene is drawn with, so the labels sit on the board while it
//! orbits as well as at rest. Fixed pixel offsets from a projected point drifted as soon as
//! the board's size on screen changed with the angle or the distance.

use bevy::prelude::*;

use crate::camera::{FloorViewMode, OrbitCamera, PitCamera3d};
use crate::candles::{CandleStore, ChartSettings};
use crate::live_feed::LiveFeed;
use crate::market::Instrument;
use crate::pit::{format_price_with_commas, instrument_decimals, set_ui_text};
use crate::pit_3d::pit_3d_origin;
use crate::pit_engine::tick_size;
use crate::tape_chart::{BOARD_BARS, TapeHistory, candle_layout, chart_source};
use crate::ui::Selection;

/// Board size and placement differ by view: behind the crowd when a pit is focused, high
/// above the pit when the whole floor is in view.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BoardPlacement {
    pub w: f32,
    pub h: f32,
    /// Distance behind the pit centre, away from the camera.
    pub back: f32,
    pub y: f32,
}

pub const FOCUS_PLACEMENT: BoardPlacement = BoardPlacement {
    w: 62.0,
    h: 20.0,
    back: 52.0,
    y: 15.0,
};
pub const MARKET_PLACEMENT: BoardPlacement = BoardPlacement {
    w: 110.0,
    h: 38.0,
    back: 0.0,
    y: 58.0,
};
const DEAL_NODES: usize = 160;

/// How a label hangs off the board's projected corner(s), in logical viewport px.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LabelAnchor {
    /// The text ends `gap` px before this point and is centred on it vertically.
    EndsAt { at: Vec2, gap: f32 },
    /// The text is centred on the segment between these two points.
    CentredBetween(Vec2, Vec2),
}

/// Absolute node insets for an anchored label `line_h` px tall: exactly one of `left` and
/// `right` is set; `width` only for a centred label.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LabelBox {
    pub left: Option<f32>,
    pub right: Option<f32>,
    pub width: Option<f32>,
    pub top: f32,
}

/// Where an anchored label's node goes on a viewport `viewport_w` px wide.
pub fn label_box(anchor: LabelAnchor, viewport_w: f32, line_h: f32) -> LabelBox {
    match anchor {
        LabelAnchor::EndsAt { at, gap } => LabelBox {
            left: None,
            right: Some(viewport_w - at.x + gap),
            width: None,
            top: at.y - line_h * 0.5,
        },
        LabelAnchor::CentredBetween(a, b) => LabelBox {
            left: Some(a.x.min(b.x)),
            right: None,
            width: Some((a.x - b.x).abs()),
            top: (a.y + b.y) * 0.5 - line_h * 0.5,
        },
    }
}

/// Apply a `LabelBox` to a node (only the fields that changed, so the UI does not relayout
/// every frame for nothing).
fn place_label(node: &mut Node, b: LabelBox) {
    let set = |slot: &mut Val, v: Option<f32>| {
        let want = v.map(Val::Px).unwrap_or(Val::Auto);
        if *slot != want {
            *slot = want;
        }
    };
    set(&mut node.left, b.left);
    set(&mut node.right, b.right);
    set(&mut node.width, b.width);
    set(&mut node.top, Some(b.top));
    if node.display != Display::Flex {
        node.display = Display::Flex;
    }
}

const LABEL_FONT: f32 = 11.0;
/// The label node's height: one line of `LABEL_FONT`.
const LABEL_LINE_H: f32 = 14.0;
const LABEL_GAP: f32 = 5.0;
/// The symbol caption above the board.
const SYMBOL_FONT: f32 = 30.0;
const SYMBOL_LINE_H: f32 = 36.0;
/// Lowest the caption's top may sit on screen (just under the header bar).
const SYMBOL_MIN_TOP: f32 = 46.0;

/// Board transform: centred `back` units behind the pit from the camera's point of view,
/// `y` up, turned square-on to the camera so it reads from any angle.
pub fn board_transform(origin: Vec3, cam_pos: Vec3, p: BoardPlacement) -> Transform {
    let mut away = origin - cam_pos;
    away.y = 0.0;
    let away = away.normalize_or(Vec3::Z);
    let centre = origin + away * p.back + Vec3::new(0.0, p.y, 0.0);
    // Local +Z (the candle side) faces the camera: look away from it.
    Transform::from_translation(centre).looking_to(centre - cam_pos, Vec3::Y)
}

#[derive(Component)]
pub struct WallBoard;

#[derive(Component, Clone, Copy, PartialEq, Eq)]
pub enum WallPart {
    Backing,
    Rail(bool),
    Wick(usize),
    Body(usize),
    Deal(usize),
    /// A resting-order band across the board.
    Level(usize),
}

#[derive(Component, Clone, Copy, PartialEq, Eq)]
pub enum WallLabel {
    Hi,
    Lo,
    Title,
    /// The pit's symbol in large letters above the board (Paul, 2026-10-23: "Above the wall
    /// board should be the Symbol in obvious letters").
    Symbol,
}

impl WallLabel {
    fn font(self) -> f32 {
        if self == Self::Symbol {
            SYMBOL_FONT
        } else {
            LABEL_FONT
        }
    }

    fn line_h(self) -> f32 {
        if self == Self::Symbol {
            SYMBOL_LINE_H
        } else {
            LABEL_LINE_H
        }
    }

    fn centred(self) -> bool {
        matches!(self, Self::Title | Self::Symbol)
    }
}

#[derive(Resource)]
pub struct WallMaterials {
    up: Handle<StandardMaterial>,
    down: Handle<StandardMaterial>,
    buy: Handle<StandardMaterial>,
    sell: Handle<StandardMaterial>,
    level_bull: Handle<StandardMaterial>,
    level_bear: Handle<StandardMaterial>,
    level_flat: Handle<StandardMaterial>,
    oanda_long: Handle<StandardMaterial>,
    oanda_short: Handle<StandardMaterial>,
}

pub fn setup_wall_board(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let cube = meshes.add(Cuboid::new(1.0, 1.0, 1.0));
    let emissive = |c: Color, k: f32| StandardMaterial {
        base_color: c,
        emissive: LinearRgba::from(c) * k,
        unlit: true,
        ..default()
    };
    let translucent = |c: Color| StandardMaterial {
        base_color: c.with_alpha(0.18),
        emissive: LinearRgba::from(c) * 0.35,
        alpha_mode: AlphaMode::Blend,
        unlit: true,
        ..default()
    };
    let mats = WallMaterials {
        up: materials.add(emissive(Color::srgb(0.25, 1.0, 0.45), 1.0)),
        down: materials.add(emissive(Color::srgb(1.0, 0.32, 0.32), 1.0)),
        buy: materials.add(emissive(Color::srgb(0.55, 1.0, 0.95), 1.0)),
        sell: materials.add(emissive(Color::srgb(1.0, 0.75, 0.30), 1.0)),
        level_bull: materials.add(translucent(Color::srgb(0.25, 0.85, 0.55))),
        level_bear: materials.add(translucent(Color::srgb(0.95, 0.40, 0.40))),
        level_flat: materials.add(translucent(Color::srgb(0.75, 0.78, 0.85))),
        oanda_long: materials.add(emissive(Color::srgb(0.30, 0.75, 1.0), 0.8)),
        oanda_short: materials.add(emissive(Color::srgb(1.0, 0.40, 0.80), 0.8)),
    };
    let backing = materials.add(StandardMaterial {
        base_color: Color::srgb(0.02, 0.03, 0.06),
        unlit: true,
        ..default()
    });
    let frame = materials.add(emissive(Color::srgb(0.85, 0.70, 0.25), 0.6));

    commands
        .spawn((WallBoard, Transform::IDENTITY, Visibility::Hidden))
        .with_children(|b| {
            b.spawn((
                WallPart::Backing,
                Mesh3d(cube.clone()),
                MeshMaterial3d(backing),
                Transform::IDENTITY,
            ));
            b.spawn((
                WallPart::Rail(true),
                Mesh3d(cube.clone()),
                MeshMaterial3d(frame.clone()),
                Transform::IDENTITY,
            ));
            b.spawn((
                WallPart::Rail(false),
                Mesh3d(cube.clone()),
                MeshMaterial3d(frame),
                Transform::IDENTITY,
            ));
            for i in 0..BOARD_BARS {
                b.spawn((
                    WallPart::Wick(i),
                    Mesh3d(cube.clone()),
                    MeshMaterial3d(mats.up.clone()),
                    Transform::IDENTITY,
                    Visibility::Hidden,
                ));
                b.spawn((
                    WallPart::Body(i),
                    Mesh3d(cube.clone()),
                    MeshMaterial3d(mats.up.clone()),
                    Transform::IDENTITY,
                    Visibility::Hidden,
                ));
            }
            for i in 0..DEAL_NODES {
                b.spawn((
                    WallPart::Deal(i),
                    Mesh3d(cube.clone()),
                    MeshMaterial3d(mats.buy.clone()),
                    Transform::IDENTITY,
                    Visibility::Hidden,
                ));
            }
            for i in 0..crate::levels::LEVEL_NODES {
                b.spawn((
                    WallPart::Level(i),
                    Mesh3d(cube.clone()),
                    MeshMaterial3d(mats.level_flat.clone()),
                    Transform::IDENTITY,
                    Visibility::Hidden,
                ));
            }
        });
    commands.insert_resource(mats);

    // Each label is a positioned box with the text inside, so the box can be anchored by its
    // right edge (prices) or centred (title) without knowing how wide the text is.
    for which in [
        WallLabel::Hi,
        WallLabel::Lo,
        WallLabel::Title,
        WallLabel::Symbol,
    ] {
        commands
            .spawn((
                which,
                Node {
                    position_type: PositionType::Absolute,
                    display: Display::None,
                    height: Val::Px(which.line_h()),
                    align_items: AlignItems::Center,
                    justify_content: if which.centred() {
                        JustifyContent::Center
                    } else {
                        JustifyContent::FlexEnd
                    },
                    ..default()
                },
                GlobalZIndex(3),
            ))
            .with_children(|b| {
                b.spawn((
                    Text::new(""),
                    TextFont {
                        font_size: which.font(),
                        ..default()
                    },
                    TextLayout::new_with_linebreak(LineBreak::NoWrap),
                    TextColor(Color::srgb(0.85, 0.80, 0.60)),
                    Node {
                        flex_shrink: 0.0,
                        ..default()
                    },
                ));
            });
    }
}

/// System: place and redraw the wall board for the selected pit.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn update_wall_board(
    time: Res<Time>,
    feed: Res<LiveFeed>,
    view_mode: Res<FloorViewMode>,
    selection: Res<Selection>,
    settings: Res<ChartSettings>,
    store: Res<CandleStore>,
    history: Res<TapeHistory>,
    mats: Res<WallMaterials>,
    levels: Res<crate::levels::LevelStore>,
    camera: Query<
        (&Camera, &Transform, &OrbitCamera),
        (With<PitCamera3d>, Without<WallBoard>, Without<WallPart>),
    >,
    mut board: Query<(&mut Transform, &mut Visibility), (With<WallBoard>, Without<WallPart>)>,
    mut parts: Query<
        (
            &WallPart,
            &mut Transform,
            &mut Visibility,
            &mut MeshMaterial3d<StandardMaterial>,
        ),
        Without<WallBoard>,
    >,
    mut labels: Query<(&WallLabel, &mut Node, &Children)>,
    mut label_texts: Query<(&mut Text, &mut TextColor)>,
    mut drawn: Local<
        Option<(
            Instrument,
            u64,
            u64,
            crate::live_feed::FeedMode,
            crate::candles::Timeframe,
            BoardPlacement,
            bool,
            u64,
            bool,
        )>,
    >,
) {
    let Ok((mut board_tf, mut board_vis)) = board.single_mut() else {
        return;
    };
    let now = time.elapsed_secs_f64();
    let shown = view_mode.is_3d() && settings.wall_board && !selection.wall_board_dismissed;
    let Some(inst) = selection.selected_instrument.filter(|_| shown) else {
        *board_vis = Visibility::Hidden;
        for (_, mut node, _) in labels.iter_mut() {
            node.display = Display::None;
        }
        return;
    };
    let Ok((cam, cam_local, orbit)) = camera.single() else {
        return;
    };
    // The camera is a root entity, so this frame's `Transform` is what the scene renders
    // with (this system runs after the orbit); the propagated `GlobalTransform` is last frame's.
    let cam_tf = GlobalTransform::from(*cam_local);

    let place = if orbit.market_view {
        MARKET_PLACEMENT
    } else {
        FOCUS_PLACEMENT
    };
    *board_tf = board_transform(pit_3d_origin(inst), cam_tf.translation(), place);
    *board_vis = Visibility::Inherited;
    let (bw, bh) = (place.w, place.h);

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
        place,
        settings.deal_markers,
        levels.revision,
        settings.levels,
    );
    let marks = if settings.deal_markers {
        tape.marks()
    } else {
        Vec::new()
    };
    let all_bars = series.shown();
    let layout = candle_layout(
        &all_bars,
        &marks,
        tf.millis(),
        bw,
        bh,
        tick_size(inst) * 6.0,
        BOARD_BARS,
    );
    let shown_bars: Vec<crate::candles::Candle> = layout
        .as_ref()
        .map(|l| all_bars[all_bars.len() - l.candles.len()..].to_vec())
        .unwrap_or_default();
    // Half a slot per body leaves a gap as wide as the candle; wicks are a hairline.
    let slot = bw / BOARD_BARS as f32;
    let (body_w, wick_w) = (slot * 0.5, (slot * 0.12).max(0.08));

    if *drawn != Some(key) {
        *drawn = Some(key);
        let half = Vec2::new(bw * 0.5, bh * 0.5);
        for (part, mut tf_, mut vis, mut mat) in parts.iter_mut() {
            match *part {
                WallPart::Backing => {
                    *tf_ = Transform::from_xyz(0.0, 0.0, -0.8).with_scale(Vec3::new(
                        bw + 3.0,
                        bh + 3.0,
                        0.4,
                    ));
                    *vis = Visibility::Inherited;
                    continue;
                }
                WallPart::Rail(top) => {
                    let y = if top { half.y + 1.5 } else { -half.y - 1.5 };
                    *tf_ =
                        Transform::from_xyz(0.0, y, -0.4).with_scale(Vec3::new(bw + 3.0, 0.4, 0.5));
                    *vis = Visibility::Inherited;
                    continue;
                }
                _ => {}
            }
            let drawn_part = match (*part, layout.as_ref()) {
                (WallPart::Wick(i), Some(l)) => l.candles.get(i).map(|c| {
                    let h = (c.wick_hi - c.wick_lo).max(0.2);
                    (
                        Vec3::new(c.x - half.x, c.wick_lo + h * 0.5 - half.y, 0.0),
                        Vec3::new(wick_w, h, 0.3),
                        if c.up { &mats.up } else { &mats.down },
                    )
                }),
                (WallPart::Body(i), Some(l)) => l.candles.get(i).map(|c| {
                    let h = (c.body_hi - c.body_lo).max(0.25);
                    (
                        Vec3::new(c.x - half.x, c.body_lo + h * 0.5 - half.y, 0.15),
                        Vec3::new(body_w, h, 0.6),
                        if c.up { &mats.up } else { &mats.down },
                    )
                }),
                (WallPart::Deal(i), Some(l)) => l.prints.get(i).map(|&(x, y, side)| {
                    (
                        Vec3::new(x - half.x, y - half.y, 0.9),
                        Vec3::splat((slot * 0.6).clamp(0.4, 1.0)),
                        if side.is_buy() { &mats.buy } else { &mats.sell },
                    )
                }),
                (WallPart::Level(i), Some(l)) => levels
                    .for_pit(inst)
                    .get(i)
                    .filter(|_| settings.levels)
                    .filter(|lv| lv.hi >= l.lo && lv.lo <= l.hi)
                    .map(|lv| {
                        let span = (l.hi - l.lo).max(1e-6);
                        let y_of = |p: f32| {
                            (crate::tape_chart::PAD_PX
                                + (p - l.lo) / span * (bh - 2.0 * crate::tape_chart::PAD_PX))
                                .clamp(0.0, bh)
                        };
                        let (y0, y1) = (y_of(lv.lo), y_of(lv.hi));
                        let x0 = crate::levels::level_start_x(lv, &shown_bars, &l.candles, slot);
                        // A cluster taller than a sixth of the board falls back to the translucent
                        // material so it never walls off the candles.
                        let tall = (y1 - y0) > bh / 6.0;
                        let m = match (lv.kind.is_oanda() && !tall, lv.kind.bias()) {
                            (true, 1) => &mats.oanda_long,
                            (true, _) => &mats.oanda_short,
                            (false, 1) => &mats.level_bull,
                            (false, -1) => &mats.level_bear,
                            _ => &mats.level_flat,
                        };
                        // OANDA clusters sit in front of the candles and never thinner than a wick.
                        let (z, min_h, depth) = if lv.kind.is_oanda() && !tall {
                            (0.5, bh * 0.012, 0.3)
                        } else {
                            (-0.3, 0.12, 0.15)
                        };
                        (
                            Vec3::new((x0 + bw) * 0.5 - half.x, (y0 + y1) * 0.5 - half.y, z),
                            Vec3::new((bw - x0).max(0.2), (y1 - y0).max(min_h), depth),
                            m,
                        )
                    }),
                _ => None,
            };
            match drawn_part {
                Some((pos, scale, m)) => {
                    *tf_ = Transform::from_translation(pos).with_scale(scale);
                    *vis = Visibility::Inherited;
                    if mat.0 != *m {
                        mat.0 = m.clone();
                    }
                }
                None => *vis = Visibility::Hidden,
            }
        }
    }

    // Labels anchored to the board's projected corners: prices end just left of the top and
    // bottom left corners, the title is centred under the bottom edge.
    let decimals = instrument_decimals(inst);
    let viewport_w = cam.logical_viewport_size().map(|s| s.x).unwrap_or(1280.0);
    let project = |local: Vec3| {
        cam.world_to_viewport(&cam_tf, board_tf.transform_point(local))
            .ok()
    };
    let title_y = -bh * 0.5 - 3.5;
    // The symbol caption sits a little above the top edge; the gap grows with the board so it
    // clears the high-price label in both placements.
    let symbol_y = bh * 0.5 + bh * 0.16 + 2.0;
    let symbol_colour = inst.accent_color();
    for (which, mut node, children) in labels.iter_mut() {
        let placed = match (which, layout.as_ref()) {
            (WallLabel::Symbol, _) => project(Vec3::new(-bw * 0.5, symbol_y, 0.0))
                .zip(project(Vec3::new(bw * 0.5, symbol_y, 0.0)))
                .map(|(a, b)| {
                    (
                        LabelAnchor::CentredBetween(a, b),
                        format!("{}   {}", inst.symbol(), inst.name()),
                    )
                }),
            (WallLabel::Hi, Some(l)) => project(Vec3::new(-bw * 0.5, bh * 0.5, 0.0)).map(|at| {
                (
                    LabelAnchor::EndsAt { at, gap: LABEL_GAP },
                    format_price_with_commas(l.hi as f64, decimals),
                )
            }),
            (WallLabel::Lo, Some(l)) => project(Vec3::new(-bw * 0.5, -bh * 0.5, 0.0)).map(|at| {
                (
                    LabelAnchor::EndsAt { at, gap: LABEL_GAP },
                    format_price_with_commas(l.lo as f64, decimals),
                )
            }),
            (WallLabel::Title, _) => project(Vec3::new(-bw * 0.5, title_y, 0.0))
                .zip(project(Vec3::new(bw * 0.5, title_y, 0.0)))
                .map(|(a, b)| {
                    (
                        LabelAnchor::CentredBetween(a, b),
                        format!("{}  {}  {source}  |  B hides", inst.name(), tf.label()),
                    )
                }),
            (_, None) => None,
        };
        match placed {
            Some((anchor, value)) => {
                let mut b = label_box(anchor, viewport_w, which.line_h());
                if *which == WallLabel::Symbol {
                    // In pit view the board's top edge reaches the top of the screen; keep the
                    // caption below the header bar rather than under it.
                    b.top = b.top.max(SYMBOL_MIN_TOP);
                }
                place_label(&mut node, b);
                if let Some((mut text, mut colour)) =
                    children.first().and_then(|c| label_texts.get_mut(*c).ok())
                {
                    set_ui_text(&mut text, value);
                    if *which == WallLabel::Symbol && colour.0 != symbol_colour {
                        colour.0 = symbol_colour;
                    }
                }
            }
            None => {
                if node.display != Display::None {
                    node.display = Display::None;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn price_labels_end_before_their_corner_and_the_title_is_centred_under_the_board() {
        let b = label_box(
            LabelAnchor::EndsAt {
                at: Vec2::new(300.0, 100.0),
                gap: 5.0,
            },
            1280.0,
            14.0,
        );
        assert_eq!(
            b.right,
            Some(1280.0 - 300.0 + 5.0),
            "measured from the right edge, so text width is irrelevant"
        );
        assert_eq!(b.left, None);
        assert_eq!(b.width, None);
        assert!((b.top - 93.0).abs() < 1e-6, "centred on the corner's y");

        let t = label_box(
            LabelAnchor::CentredBetween(Vec2::new(400.0, 310.0), Vec2::new(200.0, 300.0)),
            1280.0,
            14.0,
        );
        assert_eq!(t.left, Some(200.0), "corner order does not matter");
        assert_eq!(t.width, Some(200.0));
        assert_eq!(t.right, None);
        assert!((t.top - (305.0 - 7.0)).abs() < 1e-6);
    }

    #[test]
    fn board_faces_the_camera_and_keeps_its_top_up_from_any_angle() {
        let origin = Vec3::new(120.0, 0.0, -80.0);
        for &place in &[FOCUS_PLACEMENT, MARKET_PLACEMENT] {
            for pitch_deg in [6.0f32, 36.0, 58.0, 85.0] {
                for yaw_deg in [0.0f32, 45.0, 135.0, 200.0, 300.0] {
                    let (p, y) = (pitch_deg.to_radians(), yaw_deg.to_radians());
                    let r = 96.0;
                    let cam = origin
                        + Vec3::new(r * p.cos() * y.sin(), r * p.sin(), r * p.cos() * y.cos());
                    let tf = board_transform(origin, cam, place);
                    let to_cam = (cam - tf.translation).normalize();
                    assert!(
                        tf.forward().dot(to_cam) < -0.99,
                        "local +Z faces the camera at pitch {pitch_deg} yaw {yaw_deg}"
                    );
                    let hi = tf.transform_point(Vec3::new(-place.w * 0.5, place.h * 0.5, 0.0));
                    let lo = tf.transform_point(Vec3::new(-place.w * 0.5, -place.h * 0.5, 0.0));
                    assert!(hi.y > lo.y, "the high corner is above the low one");
                    let (bl, br) = (
                        tf.transform_point(Vec3::new(-place.w * 0.5, 0.0, 0.0)),
                        tf.transform_point(Vec3::new(place.w * 0.5, 0.0, 0.0)),
                    );
                    assert!((bl.y - br.y).abs() < 1e-3, "the board does not roll");
                    assert!((tf.translation.y - place.y).abs() < 1e-4);
                }
            }
        }
    }
}
