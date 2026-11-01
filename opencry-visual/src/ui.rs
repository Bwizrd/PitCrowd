use bevy::prelude::*;
use crate::camera::{CameraGoal, CameraRequest, FloorViewMode, MainCamera, OrbitCamera, LEGACY_2D_FLOOR};
use crate::market::{Instrument, MarketActivitySim};
use crate::panels::{spawn_panel_header, Panel, PanelId, PanelState};
use crate::pit::{pit_state_label, set_ui_text, Pit, PitBounds};
use crate::live_feed::{sim_only_guard, FeedMode, LiveFeed};
use crate::pit_engine::{PitEngines, Side};
use crate::scenario::{MarketScenario, MarketScenarioController};
use crate::trader::Trader;

/// Resource tracking currently clicked/inspected pit
#[derive(Resource, Debug, Clone)]
pub struct Selection {
    pub selected_instrument: Option<Instrument>,
    /// Set by a click on empty floor: the panels keep the last pit, but the wall board over
    /// it comes down until a pit is picked again (or the wall board is switched on).
    pub wall_board_dismissed: bool,
}

impl Default for Selection {
    fn default() -> Self {
        Self {
            selected_instrument: Some(Instrument::Silver),
            wall_board_dismissed: false,
        }
    }
}

impl Selection {
    /// Pick a pit: it becomes the inspected one and its wall board is allowed back up.
    pub fn pick(&mut self, inst: Instrument) {
        self.selected_instrument = Some(inst);
        self.wall_board_dismissed = false;
    }

    /// A click that landed on no pit: the wall board goes away, the selection stays.
    pub fn click_away(&mut self) {
        self.wall_board_dismissed = true;
    }
}

/// Resource controlling developer F1 diagnostic overlay (default false on mobile for clean screen)
#[derive(Resource, Debug, Clone)]
pub struct DebugOverlayState {
    pub visible: bool,
}

impl Default for DebugOverlayState {
    fn default() -> Self {
        Self { visible: false }
    }
}

/// Resource controlling whether the detailed HUD metrics drawer is expanded or collapsed
#[derive(Resource, Debug, Clone)]
pub struct HudExpandedState {
    pub visible: bool,
}

impl Default for HudExpandedState {
    fn default() -> Self {
        Self { visible: false }
    }
}

/// Resource controlling whether the participant Role Key / Legend is visible
#[derive(Resource, Debug, Clone)]
pub struct RoleKeyVisibleState {
    pub visible: bool,
}

impl Default for RoleKeyVisibleState {
    fn default() -> Self {
        Self { visible: true }
    }
}

/// The guide (`static/explainer.html`, copied next to the app by trunk) in a new tab. Native
/// runs have no browser to hand, so they say where the file is.
pub fn open_guide() {
    #[cfg(target_arch = "wasm32")]
    {
        if let Some(w) = web_sys::window() {
            if w.open_with_url_and_target("explainer.html", "_blank").is_err() {
                warn!("guide: the browser blocked the new tab; open /explainer.html by hand");
            }
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    info!("guide: opencry-visual/static/explainer.html (served as /explainer.html by trunk)");
}

/// Reload the page: used when the desk's board names traders this build does not know,
/// which means a newer build is being served than the one this tab loaded.
pub fn reload_page() {
    #[cfg(target_arch = "wasm32")]
    {
        if let Some(w) = web_sys::window() {
            let _ = w.location().reload();
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    info!("reload requested: this build does not know every trader on the desk");
}

/// Action types for on-screen touch and click buttons
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub enum TouchButtonAction {
    Scenario(MarketScenario),
    Shock(Instrument),
    ZoomIn,
    ZoomOut,
    ResetCamera,
    ToggleDiagnostics,
    ToggleHudDetails,
    ToggleRoleKey,
    ToggleViewMode,
    /// Charts: the board candles, the web chart, the wall board, and the timeframe cycle.
    ChartBoard,
    ChartWeb,
    ChartWall,
    ChartTimeframe,
    ChartDeals,
    /// Resting-order levels on the charts (O).
    ChartLevels,
    /// The guide page in a new browser tab (G).
    Guide,
    /// Jev reads of the focused pit on/off (J).
    JevToggle,
    /// The paper traders' leaderboard (T).
    TradersToggle,
    /// The HUD status panel (the `[LIVE] ...` line and its details drawer).
    ToggleStatus,
    /// The board chart fills the viewport / goes back to its column (E).
    BoardExpand,
    /// The trade ticket: manual paper trades on the pit in view.
    ToggleTicket,
    /// Your open and closed paper trades today.
    TogglePositions,
    /// Bitcoin's level 2 order book panel.
    ToggleDepth,
}

/// Marker for the View Mode (2D Floor / 3D Pit) toggle button container
#[derive(Component)]
pub struct ViewModeButton;

/// Marker for the View Mode toggle button text
#[derive(Component)]
pub struct ViewModeButtonText;

/// Marker for the on-screen Role Key / Legend panel container
#[derive(Component)]
pub struct RoleKeyContainer;

/// Marker for HUD selected pit panel text (detailed drawer)
#[derive(Component)]
pub struct HudInfoText;

/// Marker for compact 1-line selected pit summary in top header
#[derive(Component)]
pub struct HudCompactSummaryText;

/// Marker for active scenario header text
#[derive(Component)]
pub struct ScenarioHeaderText;

/// Marker for F1 debug overlay panel text
#[derive(Component)]
pub struct DebugOverlayText;

/// Marker for F1 debug panel container
#[derive(Component)]
pub struct DebugOverlayContainer;

/// Marker for expandable HUD details drawer container
#[derive(Component)]
pub struct HudDetailsContainer;

/// Marker for the HUD status panel (left column, under the traders): the compact pit
/// summary line and the details drawer.
#[derive(Component)]
pub struct HudStatusContainer;

/// Marker for bottom announcement toast container
#[derive(Component)]
pub struct AnnouncementToastContainer;

/// Marker for bottom announcement toast text
#[derive(Component)]
pub struct AnnouncementToastText;

/// Helper: drops a burst of one-sided block paper onto a pit's lead broker and inspects that pit.
/// The engine decides what happens next; nothing about the outcome is scripted here.
pub fn trigger_manual_shock(
    inst: Instrument,
    sim: &mut MarketActivitySim,
    engines: &mut PitEngines,
    controller: &mut MarketScenarioController,
    selection: &mut Selection,
) {
    sim.trigger_shock(inst);
    let (side, label) = match inst {
        Instrument::Silver => (Side::Buy, "[SHOCK] SILVER: BLOCK BUYING HITS THE RAIL"),
        Instrument::Gold => (Side::Buy, "[SHOCK] GOLD: BLOCK BUYING HITS THE RAIL"),
        Instrument::Nasdaq => (Side::Sell, "[SHOCK] NASDAQ: BLOCK SELLING HITS THE RAIL"),
        Instrument::Sp500 => (Side::Sell, "[SHOCK] S&P: BLOCK SELLING HITS THE RAIL"),
        _ => (Side::Buy, "[SHOCK] BLOCK PAPER HITS THE RAIL"),
    };
    engines.shock(inst, side);
    controller.announcement_banner = label.to_string();
    controller.banner_timer = 5.0;
    selection.pick(inst);
}

/// Startup system: Spawns responsive HUD overlay nodes and mobile touch action bar
pub fn setup_ui(mut commands: Commands) {
    // ---------------------------------------------------------
    // 1. Top Responsive Header & Details Card (fits mobile upright & landscape)
    // ---------------------------------------------------------
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(8.0),
                left: Val::Px(8.0),
                right: Val::Px(8.0),
                max_width: Val::Px(520.0),
                flex_direction: FlexDirection::Column,
                padding: UiRect::axes(Val::Px(10.0), Val::Px(6.0)),
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(Val::Px(6.0)),
                row_gap: Val::Px(4.0),
                ..default()
            },
            BackgroundColor(Color::srgba(0.03, 0.05, 0.08, 1.0)),
            BorderColor::all(Color::srgba(0.22, 0.38, 0.55, 0.65)),
        ))
        .with_children(|parent| {
            // Top row: Title + Active Scenario Badge + Details toggle button
            parent
                .spawn(Node {
                    flex_direction: FlexDirection::Row,
                    justify_content: JustifyContent::SpaceBetween,
                    align_items: AlignItems::Center,
                    width: Val::Percent(100.0),
                    ..default()
                })
                .with_children(|row| {
                    row.spawn((
                        Text::new("OPENCRY FLOOR"),
                        TextFont {
                            font_size: 12.5,
                            ..default()
                        },
                        TextColor(Color::srgb(0.95, 0.95, 0.98)),
                    ));

                    row.spawn((
                        ScenarioHeaderText,
                        Text::new("Normal Flow"),
                        TextFont {
                            font_size: 11.5,
                            ..default()
                        },
                        TextColor(Color::srgb(0.38, 0.85, 0.98)),
                    ));

                    // 3D Pit Deep-Dive / 2D Floor View Mode toggle button
                    row.spawn((
                        Button,
                        TouchButtonAction::ToggleViewMode,
                        ViewModeButton,
                        Node {
                            padding: UiRect::axes(Val::Px(10.0), Val::Px(3.5)),
                            border: UiRect::all(Val::Px(1.0)),
                            border_radius: BorderRadius::all(Val::Px(4.0)),
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::Center,
                            margin: UiRect::left(Val::Px(6.0)),
                            ..default()
                        },
                        BackgroundColor(Color::srgba(0.08, 0.32, 0.50, 0.95)),
                        BorderColor::all(Color::srgb(0.22, 0.78, 0.98)),
                    ))
                    .with_children(|btn| {
                        btn.spawn((
                            ViewModeButtonText,
                            Text::new("3D PIT VIEW [SPACE]"),
                            TextFont {
                                font_size: 10.5,
                                ..default()
                            },
                            TextColor(Color::srgb(0.95, 0.98, 1.0)),
                        ));
                    });
                });
        });

    // ---------------------------------------------------------
    // 1B. HUD status panel: left column under the traders. The compact pit summary line
    //     and its details drawer; `sync_panels` shows and hides it (Status button).
    // ---------------------------------------------------------
    commands
        .spawn((
            HudStatusContainer,
            Panel(PanelId::Status),
            Node {
                width: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                padding: UiRect::axes(Val::Px(10.0), Val::Px(6.0)),
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(Val::Px(6.0)),
                row_gap: Val::Px(4.0),
                ..default()
            },
            BackgroundColor(Color::srgba(0.03, 0.05, 0.08, 1.0)),
            BorderColor::all(Color::srgba(0.22, 0.38, 0.55, 0.65)),
        ))
        .with_children(|parent| {
            let me = parent.target_entity();
            spawn_panel_header(parent, me, "PIT STATUS", Color::srgb(0.70, 0.82, 0.95), |row| {
                // Details expand/collapse button
                row.spawn((
                    Button,
                    TouchButtonAction::ToggleHudDetails,
                    Node {
                        padding: UiRect::axes(Val::Px(8.0), Val::Px(1.5)),
                        border: UiRect::all(Val::Px(1.0)),
                        border_radius: BorderRadius::all(Val::Px(4.0)),
                        justify_content: JustifyContent::Center,
                        align_items: AlignItems::Center,
                        flex_shrink: 0.0,
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.08, 0.14, 0.22, 0.90)),
                    BorderColor::all(Color::srgba(0.30, 0.45, 0.65, 0.70)),
                ))
                .with_children(|btn| {
                    btn.spawn((
                        Text::new("Details v"),
                        TextFont {
                            font_size: 10.5,
                            ..default()
                        },
                        TextColor(Color::srgb(0.70, 0.82, 0.95)),
                    ));
                });
            });

            // Compact 1-line summary
            parent.spawn((
                HudCompactSummaryText,
                Text::new("SILVER (XAG) | Act: 72% | Flow: +10% | Pop: 24"),
                TextFont {
                    font_size: 11.0,
                    ..default()
                },
                TextColor(Color::srgb(0.80, 0.88, 0.96)),
            ));

            // Expandable details drawer container (closed by default on mobile)
            parent
                .spawn((
                    HudDetailsContainer,
                    Node {
                        display: Display::None,
                        flex_direction: FlexDirection::Column,
                        row_gap: Val::Px(3.0),
                        margin: UiRect::top(Val::Px(4.0)),
                        padding: UiRect::top(Val::Px(4.0)),
                        border: UiRect::top(Val::Px(1.0)),
                        ..default()
                    },
                    BorderColor::all(Color::srgba(0.20, 0.32, 0.45, 0.50)),
                ))
                .with_children(|drawer| {
                    drawer.spawn((
                        HudInfoText,
                        Text::new("Initializing pit inspection..."),
                        TextFont {
                            font_size: 11.0,
                            ..default()
                        },
                        TextColor(Color::srgb(0.70, 0.78, 0.86)),
                    ));

                    drawer.spawn((
                        Text::new("[WASD/Drag]: Pan  [Wheel/Pinch]: Zoom  [Tap/Click]: Inspect"),
                        TextFont {
                            font_size: 9.5,
                            ..default()
                        },
                        TextColor(Color::srgb(0.45, 0.55, 0.65)),
                    ));
                });
        });

    // ---------------------------------------------------------
    // 2. F1 Diagnostics / Details panel: right column under the board (closed by default)
    // ---------------------------------------------------------
    commands
        .spawn((
            DebugOverlayContainer,
            Panel(PanelId::Details),
            Node {
                display: Display::None,
                width: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                padding: UiRect::all(Val::Px(10.0)),
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(Val::Px(6.0)),
                row_gap: Val::Px(3.0),
                ..default()
            },
            BackgroundColor(Color::srgba(0.04, 0.06, 0.09, 1.0)),
            BorderColor::all(Color::srgba(0.20, 0.35, 0.45, 0.60)),
        ))
        .with_children(|parent| {
            let me = parent.target_entity();
            spawn_panel_header(parent, me, "DIAGNOSTICS / DETAILS (F1)", Color::srgb(0.40, 0.85, 0.95), |_| {});

            parent.spawn((
                DebugOverlayText,
                Text::new("Calculating telemetry..."),
                TextFont {
                    font_size: 10.5,
                    ..default()
                },
                TextColor(Color::srgb(0.70, 0.78, 0.85)),
            ));
        });

    // ---------------------------------------------------------
    // 2B. Role Key & Hand Signals Legend Overlay Card
    // ---------------------------------------------------------
    commands
        .spawn((
            RoleKeyContainer,
            Panel(PanelId::RoleKey),
            Node {
                width: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                padding: UiRect::all(Val::Px(8.0)),
                border: UiRect::all(Val::Px(1.2)),
                border_radius: BorderRadius::all(Val::Px(6.0)),
                row_gap: Val::Px(4.0),
                ..default()
            },
            BackgroundColor(Color::srgba(0.04, 0.06, 0.10, 0.94)),
            BorderColor::all(Color::srgba(0.85, 0.70, 0.25, 0.85)),
        ))
        .with_children(|parent| {
            // Header with Close Button
            let me = parent.target_entity();
            spawn_panel_header(parent, me, "PARTICIPANT ROLE KEY [K]", Color::srgb(0.96, 0.85, 0.30), |_| {});

            // 1. Lead Floor Broker
            parent.spawn((
                Text::new("LEAD BROKER (Royal Blue & Gold Badge)"),
                TextFont {
                    font_size: 10.0,
                    ..default()
                },
                TextColor(Color::srgb(0.40, 0.72, 1.0)),
            ));
            parent.spawn((
                Text::new("  • PALMS IN (Green) = Buy Imbalance\n  • PALMS OUT (Red)  = Sell Imbalance"),
                TextFont {
                    font_size: 9.0,
                    ..default()
                },
                TextColor(Color::srgb(0.82, 0.90, 0.98)),
            ));

            // 2. Local Scalpers
            parent.spawn((
                Text::new("LOCAL SCALPERS (Multi-Color Coats)"),
                TextFont {
                    font_size: 10.0,
                    ..default()
                },
                TextColor(Color::srgb(0.35, 0.95, 0.55)),
            ));
            parent.spawn((
                Text::new("  Market makers providing 2-way bids & offers"),
                TextFont {
                    font_size: 9.0,
                    ..default()
                },
                TextColor(Color::srgb(0.70, 0.78, 0.86)),
            ));

            // 3. Price Reporter
            parent.spawn((
                Text::new("PRICE REPORTER (Exchange Burgundy)"),
                TextFont {
                    font_size: 10.0,
                    ..default()
                },
                TextColor(Color::srgb(0.95, 0.45, 0.45)),
            ));
            parent.spawn((
                Text::new("  Official on central pulpit logging prints to tape"),
                TextFont {
                    font_size: 9.0,
                    ..default()
                },
                TextColor(Color::srgb(0.70, 0.78, 0.86)),
            ));

            // 4. Floor Runner
            parent.spawn((
                Text::new("FLOOR RUNNER (Bright Concourse Vests)"),
                TextFont {
                    font_size: 10.0,
                    ..default()
                },
                TextColor(Color::srgb(0.95, 0.85, 0.35)),
            ));
            parent.spawn((
                Text::new("  Sprint order slips along concourses between pits"),
                TextFont {
                    font_size: 9.0,
                    ..default()
                },
                TextColor(Color::srgb(0.70, 0.78, 0.86)),
            ));

            // 5. Concourse Loungers
            parent.spawn((
                Text::new("CONCOURSE LOUNGERS (Aisle Pairs)"),
                TextFont {
                    font_size: 10.0,
                    ..default()
                },
                TextColor(Color::srgb(0.75, 0.70, 0.95)),
            ));
            parent.spawn((
                Text::new("  Resting/chatting outside pit; swarm in during surges"),
                TextFont {
                    font_size: 9.0,
                    ..default()
                },
                TextColor(Color::srgb(0.70, 0.78, 0.86)),
            ));
        });

    // ---------------------------------------------------------
    // 3. Bottom Announcement Toast Banner (placed above touch bar)
    // ---------------------------------------------------------
    commands
        .spawn((
            AnnouncementToastContainer,
            Visibility::Hidden,
            Node {
                position_type: PositionType::Absolute,
                bottom: Val::Px(88.0),
                left: Val::Px(16.0),
                right: Val::Px(16.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                padding: UiRect::axes(Val::Px(14.0), Val::Px(7.0)),
                border: UiRect::all(Val::Px(1.2)),
                border_radius: BorderRadius::all(Val::Px(6.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.06, 0.10, 0.18, 0.95)),
            BorderColor::all(Color::srgba(0.40, 0.70, 0.95, 0.85)),
        ))
        .with_children(|parent| {
            parent.spawn((
                AnnouncementToastText,
                Text::new("OpenCry Floor Initialized - Ready"),
                TextFont {
                    font_size: 12.0,
                    ..default()
                },
                TextColor(Color::srgb(0.95, 0.95, 1.0)),
            ));
        });

    // ---------------------------------------------------------
    // 4. Bottom Mobile Touch Action Bar (F2-F5, Shocks, Zoom, Reset)
    // ---------------------------------------------------------
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                bottom: Val::Px(6.0),
                left: Val::Px(6.0),
                right: Val::Px(6.0),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(4.0),
                padding: UiRect::all(Val::Px(5.0)),
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(Val::Px(8.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.03, 0.05, 0.08, 0.82)),
            BorderColor::all(Color::srgba(0.18, 0.30, 0.45, 0.60)),
        ))
        .with_children(|parent| {
            // Row 1: Scenario touch buttons (standing in for F2-F5)
            parent
                .spawn(Node {
                    flex_direction: FlexDirection::Row,
                    column_gap: Val::Px(4.0),
                    justify_content: JustifyContent::SpaceBetween,
                    width: Val::Percent(100.0),
                    ..default()
                })
                .with_children(|row| {
                    let scenarios = [
                        (MarketScenario::FedShock, "F2 Fed"),
                        (MarketScenario::SilverSqueeze, "F3 Sqz"),
                        (MarketScenario::LunchChop, "F4 Chop"),
                        (MarketScenario::Normal, "F5 Flow"),
                    ];

                    for (scen, label) in scenarios {
                        row.spawn((
                            Button,
                            TouchButtonAction::Scenario(scen),
                            Node {
                                flex_grow: 1.0,
                                flex_shrink: 1.0,
                                flex_basis: Val::Px(0.0),
                                height: Val::Px(34.0),
                                justify_content: JustifyContent::Center,
                                align_items: AlignItems::Center,
                                border: UiRect::all(Val::Px(1.0)),
                                border_radius: BorderRadius::all(Val::Px(5.0)),
                                ..default()
                            },
                            BackgroundColor(Color::srgba(0.06, 0.09, 0.15, 0.88)),
                            BorderColor::all(Color::srgba(0.20, 0.32, 0.46, 0.60)),
                        ))
                        .with_children(|btn| {
                            btn.spawn((
                                Text::new(label),
                                TextFont {
                                    font_size: 11.0,
                                    ..default()
                                },
                                TextColor(Color::srgb(0.92, 0.96, 1.0)),
                            ));
                        });
                    }
                });

            // Row 2: Hotkey Shocks, Zoom & Utility touch buttons
            parent
                .spawn(Node {
                    flex_direction: FlexDirection::Row,
                    column_gap: Val::Px(3.0),
                    justify_content: JustifyContent::SpaceBetween,
                    width: Val::Percent(100.0),
                    ..default()
                })
                .with_children(|row| {
                    let actions = [
                        (TouchButtonAction::Shock(Instrument::Silver), "Ag", Color::srgb(0.98, 0.90, 0.45)),
                        (TouchButtonAction::Shock(Instrument::Gold), "Au", Color::srgb(0.98, 0.85, 0.35)),
                        (TouchButtonAction::Shock(Instrument::Nasdaq), "NQ", Color::srgb(0.98, 0.55, 0.45)),
                        (TouchButtonAction::ToggleViewMode, "3D", Color::srgb(0.98, 0.75, 0.25)),
                        (TouchButtonAction::ZoomOut, "-", Color::srgb(0.85, 0.90, 0.98)),
                        (TouchButtonAction::ZoomIn, "+", Color::srgb(0.85, 0.90, 0.98)),
                        (TouchButtonAction::ResetCamera, "Ctr", Color::srgb(0.38, 0.85, 0.98)),
                    ];

                    for (action, label, text_color) in actions {
                        row.spawn((
                            Button,
                            action,
                            Node {
                                flex_grow: 1.0,
                                flex_shrink: 1.0,
                                flex_basis: Val::Px(0.0),
                                height: Val::Px(30.0),
                                justify_content: JustifyContent::Center,
                                align_items: AlignItems::Center,
                                border: UiRect::all(Val::Px(1.0)),
                                border_radius: BorderRadius::all(Val::Px(4.0)),
                                ..default()
                            },
                            BackgroundColor(Color::srgba(0.06, 0.09, 0.14, 0.85)),
                            BorderColor::all(Color::srgba(0.18, 0.28, 0.40, 0.55)),
                        ))
                        .with_children(|btn| {
                            btn.spawn((
                                Text::new(label),
                                TextFont {
                                    font_size: 11.0,
                                    ..default()
                                },
                                TextColor(text_color),
                            ));
                        });
                    }
                });

            // Row 3: chart toggles, one per chart, the timeframe cycle, and one button per
            // closeable panel (lit while open, reopens it when closed). Wraps on narrow screens.
            parent
                .spawn(Node {
                    flex_direction: FlexDirection::Row,
                    flex_wrap: FlexWrap::Wrap,
                    column_gap: Val::Px(3.0),
                    row_gap: Val::Px(3.0),
                    justify_content: JustifyContent::FlexStart,
                    width: Val::Percent(100.0),
                    ..default()
                })
                .with_children(|row| {
                    let actions = [
                        (TouchButtonAction::ChartBoard, "Board chart", Color::srgb(0.92, 0.96, 1.0)),
                        (TouchButtonAction::ChartWeb, "Web chart", Color::srgb(0.92, 0.96, 1.0)),
                        (TouchButtonAction::ChartWall, "Wall board", Color::srgb(0.92, 0.96, 1.0)),
                        (TouchButtonAction::ChartTimeframe, "5m", Color::srgb(0.98, 0.85, 0.35)),
                        (TouchButtonAction::ChartDeals, "Pit deals", Color::srgb(0.75, 0.80, 0.88)),
                        (TouchButtonAction::ChartLevels, "Levels", Color::srgb(0.92, 0.96, 1.0)),
                        (TouchButtonAction::JevToggle, "Jev read", Color::srgb(0.98, 0.85, 0.35)),
                        (TouchButtonAction::Guide, "Guide [G]", Color::srgb(0.98, 0.85, 0.35)),
                        (TouchButtonAction::TradersToggle, "Traders", Color::srgb(0.92, 0.96, 1.0)),
                        (TouchButtonAction::ToggleTicket, "Trade", Color::srgb(0.55, 1.0, 0.65)),
                        (TouchButtonAction::TogglePositions, "Positions", Color::srgb(0.55, 1.0, 0.65)),
                        (TouchButtonAction::ToggleDepth, "Level 2", Color::srgb(0.55, 0.85, 1.0)),
                        (TouchButtonAction::ToggleStatus, "Status", Color::srgb(0.92, 0.96, 1.0)),
                        (TouchButtonAction::ToggleDiagnostics, "Details", Color::srgb(0.65, 0.75, 0.85)),
                        (TouchButtonAction::ToggleRoleKey, "Role key", Color::srgb(0.95, 0.88, 0.35)),
                    ];
                    for (action, label, text_color) in actions {
                        row.spawn((
                            Button,
                            action,
                            Node {
                                // 14 buttons on one row at 1600 px (Trade and Positions joined it).
                                width: Val::Px(94.0),
                                height: Val::Px(26.0),
                                justify_content: JustifyContent::Center,
                                align_items: AlignItems::Center,
                                border: UiRect::all(Val::Px(1.0)),
                                border_radius: BorderRadius::all(Val::Px(4.0)),
                                ..default()
                            },
                            BackgroundColor(Color::srgba(0.06, 0.09, 0.14, 0.85)),
                            BorderColor::all(Color::srgba(0.18, 0.28, 0.40, 0.55)),
                        ))
                        .with_children(|btn| {
                            btn.spawn((
                                Text::new(label),
                                TextFont { font_size: 11.0, ..default() },
                                TextColor(text_color),
                            ));
                        });
                    }
                });
        });
}

/// System: Handles pit selection via mouse clicks or mobile touch taps in world space
pub fn handle_pit_selection_clicks(
    mouse_button: Res<ButtonInput<MouseButton>>,
    touches: Res<Touches>,
    windows: Query<&Window>,
    camera_query: Query<(&Camera, &GlobalTransform), With<MainCamera>>,
    mut selection: ResMut<Selection>,
    mut view_mode: ResMut<FloorViewMode>,
    mut pits: Query<(&mut Pit, &PitBounds)>,
) {
    if view_mode.is_3d() {
        return;
    }

    let Ok(window) = windows.single() else {
        return;
    };
    let Ok((camera, camera_transform)) = camera_query.single() else {
        return;
    };

    let mut click_pos = None;

    // 1. Desktop mouse click
    if mouse_button.just_pressed(MouseButton::Left) {
        if let Some(cursor_pos) = window.cursor_position() {
            click_pos = Some(cursor_pos);
        }
    }

    // 2. Mobile touch tap: detect tap release with minimal finger movement (< 15px)
    if click_pos.is_none() {
        for touch in touches.iter_just_released() {
            let moved = touch.start_position().distance(touch.position());
            let pos = touch.position();
            // Ignore taps directly over top header (<= 65px) or bottom control bar (>= height - 95px)
            if moved < 15.0 && pos.y > 65.0 && pos.y < window.height() - 95.0 {
                click_pos = Some(pos);
                break;
            }
        }
    }

    let Some(screen_pos) = click_pos else {
        return;
    };

    let Ok(world_pos) = camera.viewport_to_world_2d(camera_transform, screen_pos) else {
        return;
    };

    let mut clicked_instrument = None;
    for (mut pit, bounds) in pits.iter_mut() {
        if bounds.contains(world_pos) {
            clicked_instrument = Some(pit.instrument);
            pit.is_selected = true;
        } else {
            pit.is_selected = false;
        }
    }

    if let Some(inst) = clicked_instrument {
        selection.pick(inst);
        *view_mode = FloorViewMode::PitDeepDive3D;
    }
}

/// System: Handles on-screen touch and click button interactions
pub fn handle_touch_buttons(
    mut interaction_query: Query<
        (&Interaction, &TouchButtonAction),
        (Changed<Interaction>, With<Button>),
    >,
    mut sim: ResMut<MarketActivitySim>,
    mut engines: ResMut<PitEngines>,
    mut controller: ResMut<MarketScenarioController>,
    mut selection: ResMut<Selection>,
    mut view_mode: ResMut<FloorViewMode>,
    mut camera_request: ResMut<CameraRequest>,
    mut debug_state: ResMut<DebugOverlayState>,
    mut hud_expanded: ResMut<HudExpandedState>,
    mut role_key_state: ResMut<RoleKeyVisibleState>,
    feed: Res<LiveFeed>,
    mut camera_query: Query<(&mut Transform, &mut Projection), With<MainCamera>>,
) {
    for (interaction, action) in interaction_query.iter_mut() {
        if *interaction == Interaction::Pressed {
            match action {
                TouchButtonAction::Scenario(scen) => {
                    if sim_only_guard(&feed, &mut controller) {
                        controller.switch_scenario(*scen, &mut sim);
                    }
                }
                TouchButtonAction::Shock(inst) => {
                    if sim_only_guard(&feed, &mut controller) {
                        trigger_manual_shock(*inst, &mut sim, &mut engines, &mut controller, &mut selection);
                    }
                    *view_mode = FloorViewMode::PitDeepDive3D;
                }
                TouchButtonAction::ToggleViewMode => {
                    if LEGACY_2D_FLOOR {
                        view_mode.toggle();
                    } else {
                        camera_request.goal(CameraGoal::TogglePitMarket);
                    }
                }
                TouchButtonAction::ZoomIn => {
                    if view_mode.is_3d() {
                        camera_request.zoom(0.8);
                    } else if let Ok((_, mut projection)) = camera_query.single_mut() {
                        if let Projection::Orthographic(ref mut ortho) = *projection {
                            ortho.scale = (ortho.scale - 0.20).clamp(0.40, 4.20);
                        }
                    }
                }
                TouchButtonAction::ZoomOut => {
                    if view_mode.is_3d() {
                        camera_request.zoom(1.25);
                    } else if let Ok((_, mut projection)) = camera_query.single_mut() {
                        if let Projection::Orthographic(ref mut ortho) = *projection {
                            ortho.scale = (ortho.scale + 0.20).clamp(0.40, 4.20);
                        }
                    }
                }
                TouchButtonAction::ResetCamera => {
                    if view_mode.is_3d() {
                        camera_request.goal(CameraGoal::Pit);
                    } else if let Ok((mut transform, mut projection)) = camera_query.single_mut() {
                        transform.translation = Vec3::new(0.0, -10.0, 999.0);
                        if let Projection::Orthographic(ref mut ortho) = *projection {
                            ortho.scale = 1.05;
                        }
                    }
                }
                // Handled by their own modules (candles, jev, paper, panels).
                TouchButtonAction::ChartBoard | TouchButtonAction::ChartWeb | TouchButtonAction::ChartWall | TouchButtonAction::ChartTimeframe | TouchButtonAction::ChartDeals | TouchButtonAction::ChartLevels | TouchButtonAction::JevToggle | TouchButtonAction::TradersToggle | TouchButtonAction::ToggleStatus | TouchButtonAction::BoardExpand | TouchButtonAction::Guide | TouchButtonAction::ToggleTicket | TouchButtonAction::TogglePositions | TouchButtonAction::ToggleDepth => {}
                // Flags only: `panels::sync_panels` applies them to the nodes.
                TouchButtonAction::ToggleDiagnostics => debug_state.visible = !debug_state.visible,
                TouchButtonAction::ToggleHudDetails => hud_expanded.visible = !hud_expanded.visible,
                TouchButtonAction::ToggleRoleKey => role_key_state.visible = !role_key_state.visible,
            }
        }
    }
}

/// System: Updates scenario button styling to highlight active scenario with luminous cyan border
pub fn update_scenario_button_styles(
    controller: Res<MarketScenarioController>,
    mut buttons: Query<(&TouchButtonAction, &mut BackgroundColor, &mut BorderColor), With<Button>>,
) {
    for (action, mut bg, mut border) in buttons.iter_mut() {
        if let TouchButtonAction::Scenario(scen) = action {
            if *scen == controller.active_scenario {
                *bg = BackgroundColor(Color::srgba(0.08, 0.30, 0.45, 0.95));
                *border = BorderColor::all(Color::srgb(0.22, 0.74, 0.97));
            } else {
                *bg = BackgroundColor(Color::srgba(0.06, 0.09, 0.15, 0.88));
                *border = BorderColor::all(Color::srgba(0.20, 0.32, 0.46, 0.60));
            }
        }
    }
}

/// System: Handles keyboard hotkeys (1-8 for pit shocks, F1 for debug, K for role key, Space for 3D toggle, ESC to return or to restore the expanded board)
pub fn handle_keyboard_actions(
    keyboard: Res<ButtonInput<KeyCode>>,
    mut sim: ResMut<MarketActivitySim>,
    mut engines: ResMut<PitEngines>,
    mut selection: ResMut<Selection>,
    mut view_mode: ResMut<FloorViewMode>,
    mut camera_request: ResMut<CameraRequest>,
    mut controller: ResMut<MarketScenarioController>,
    mut debug_state: ResMut<DebugOverlayState>,
    mut role_key_state: ResMut<RoleKeyVisibleState>,
    mut feed: ResMut<LiveFeed>,
    mut panel_state: ResMut<PanelState>,
    time: Res<Time>,
) {
    // L: live feed <-> simulated flow. Simulated mode wears a permanent banner.
    if keyboard.just_pressed(KeyCode::KeyL) {
        let next = match feed.mode {
            FeedMode::Live => FeedMode::Simulated,
            FeedMode::Simulated => FeedMode::Live,
        };
        feed.set_mode(next, &mut engines, &mut controller, time.elapsed_secs_f64());
    }

    // Digit shocks drop synthetic paper: simulation only.
    let shock_pressed = (1..=8).any(|d| {
        let key = match d {
            1 => KeyCode::Digit1,
            2 => KeyCode::Digit2,
            3 => KeyCode::Digit3,
            4 => KeyCode::Digit4,
            5 => KeyCode::Digit5,
            6 => KeyCode::Digit6,
            7 => KeyCode::Digit7,
            _ => KeyCode::Digit8,
        };
        keyboard.just_pressed(key)
    });
    let shocks_allowed = !shock_pressed || sim_only_guard(&feed, &mut controller);

    if keyboard.just_pressed(KeyCode::Space) {
        if LEGACY_2D_FLOOR {
            view_mode.toggle();
        } else {
            camera_request.goal(CameraGoal::TogglePitMarket);
        }
    }

    if keyboard.just_pressed(KeyCode::Escape) {
        // An expanded board takes Escape first; the camera keeps its view.
        if panel_state.expanded {
            panel_state.expanded = false;
        } else if LEGACY_2D_FLOOR {
            *view_mode = FloorViewMode::FloorOverview2D;
        } else {
            camera_request.goal(CameraGoal::Market);
        }
    }

    if shocks_allowed && keyboard.just_pressed(KeyCode::Digit1) {
        trigger_manual_shock(Instrument::Silver, &mut sim, &mut engines, &mut controller, &mut selection);
        *view_mode = FloorViewMode::PitDeepDive3D;
    }
    if shocks_allowed && keyboard.just_pressed(KeyCode::Digit2) {
        trigger_manual_shock(Instrument::Gold, &mut sim, &mut engines, &mut controller, &mut selection);
        *view_mode = FloorViewMode::PitDeepDive3D;
    }
    if shocks_allowed && keyboard.just_pressed(KeyCode::Digit3) {
        trigger_manual_shock(Instrument::Nasdaq, &mut sim, &mut engines, &mut controller, &mut selection);
        *view_mode = FloorViewMode::PitDeepDive3D;
    }
    if shocks_allowed && keyboard.just_pressed(KeyCode::Digit4) {
        trigger_manual_shock(Instrument::Sp500, &mut sim, &mut engines, &mut controller, &mut selection);
        *view_mode = FloorViewMode::PitDeepDive3D;
    }
    if shocks_allowed && keyboard.just_pressed(KeyCode::Digit5) {
        trigger_manual_shock(Instrument::Us30, &mut sim, &mut engines, &mut controller, &mut selection);
        *view_mode = FloorViewMode::PitDeepDive3D;
    }
    if shocks_allowed && keyboard.just_pressed(KeyCode::Digit6) {
        trigger_manual_shock(Instrument::Uk100, &mut sim, &mut engines, &mut controller, &mut selection);
        *view_mode = FloorViewMode::PitDeepDive3D;
    }
    if shocks_allowed && keyboard.just_pressed(KeyCode::Digit7) {
        trigger_manual_shock(Instrument::EurUsd, &mut sim, &mut engines, &mut controller, &mut selection);
        *view_mode = FloorViewMode::PitDeepDive3D;
    }
    if shocks_allowed && keyboard.just_pressed(KeyCode::Digit8) {
        trigger_manual_shock(Instrument::Bitcoin, &mut sim, &mut engines, &mut controller, &mut selection);
        *view_mode = FloorViewMode::PitDeepDive3D;
    }

    // Flags only: `panels::sync_panels` applies them to the nodes.
    if keyboard.just_pressed(KeyCode::F1) {
        debug_state.visible = !debug_state.visible;
    }

    if keyboard.just_pressed(KeyCode::KeyK) {
        role_key_state.visible = !role_key_state.visible;
    }
}

/// System: HUD shows the engine's read of the inspected pit: last print, touch, sound,
/// pressure versus realised flow, depth and headcount.
pub fn update_hud_info(
    selection: Res<Selection>,
    view_mode: Res<FloorViewMode>,
    engines: Res<PitEngines>,
    controller: Res<MarketScenarioController>,
    feed: Res<LiveFeed>,
    time: Res<Time>,
    panel_state: Res<PanelState>,
    mut hud_text: Query<&mut Text, (With<HudInfoText>, Without<ScenarioHeaderText>, Without<HudCompactSummaryText>)>,
    mut compact_text: Query<&mut Text, (With<HudCompactSummaryText>, Without<ScenarioHeaderText>, Without<HudInfoText>)>,
    mut scenario_text: Query<&mut Text, (With<ScenarioHeaderText>, Without<HudInfoText>, Without<HudCompactSummaryText>)>,
) {
    for mut text in scenario_text.iter_mut() {
        set_ui_text(&mut text, controller.active_scenario.name().to_string());
    }
    // The status panel's texts are not worth formatting while it is closed.
    if !panel_state.status {
        return;
    }

    if let Some(inst) = selection.selected_instrument {
        let pit = engines.pit(inst);
        let sig = &pit.signals;
        let decimals = crate::pit::instrument_decimals(inst);
        let fmt = |ticks: i32| crate::pit::format_price_with_commas(pit.price_of(ticks) as f64, decimals);
        let last = fmt(sig.last_ticks);
        let touch = match (sig.best_bid_ticks, sig.best_ask_ticks) {
            (Some(b), Some(a)) => format!("{} / {}", fmt(b), fmt(a)),
            (Some(b), None) => format!("{} / --", fmt(b)),
            (None, Some(a)) => format!("-- / {}", fmt(a)),
            (None, None) => "-- / --".to_string(),
        };
        let state = pit_state_label(sig);
        let sound = (sig.sound * 100.0).round() as i32;
        let pressure = (sig.pressure * 100.0).round() as i32;
        let flow = (sig.imbalance * 100.0).round() as i32;
        let depth = (sig.unfilled_interest * 100.0).round() as i32;

        for mut ctext in compact_text.iter_mut() {
            let now = time.elapsed_secs_f64();
            let prefix = match feed.pit_status(inst, now) {
                crate::live_feed::PitFeedStatus::Simulated => "[SIM] ",
                crate::live_feed::PitFeedStatus::Live { .. } => "[LIVE] ",
                _ => "[HALTED] ",
            };
            let tape = match feed.pit_status(inst, now) {
                crate::live_feed::PitFeedStatus::Live { .. } => feed
                    .quote(inst)
                    .map(|q| format!("{} | ", crate::live_feed::tape_label(&q.flow(now))))
                    .unwrap_or_default(),
                _ => String::new(),
            };
            let _ = view_mode.is_3d();
            set_ui_text(&mut ctext, format!(
                "{}{} {} | {} in | {}Sound {}% | Paper {:+}% | Filled {:+}% | {}",
                prefix,
                inst.symbol(),
                last,
                pit.active_count,
                tape,
                sound,
                pressure,
                flow,
                state
            ));
        }

        for mut dtext in hud_text.iter_mut() {
            set_ui_text(&mut dtext, format!(
                "INSPECTED: {} ({}) | District: {}\nLast {} | Bid / Ask {} | {}\nSound {}% | Paper working {:+}% | Flow filled {:+}% | Depth {}% | Vacuum {}%\nHeadcount {} / {} | Inventory stress {}% | Arrivals {:.1}/s | Fills {:.1}/s",
                inst.name(),
                inst.symbol(),
                inst.district().label(),
                last,
                touch,
                state,
                sound,
                pressure,
                flow,
                depth,
                (sig.vacuum * 100.0).round() as i32,
                pit.active_count,
                crate::pit_engine::SLOTS,
                (sig.inventory_stress * 100.0).round() as i32,
                sig.arrival_intensity,
                sig.fill_rate
            ));
        }
    } else {
        for mut ctext in compact_text.iter_mut() {
            set_ui_text(&mut ctext, "Tap or click any trading pit to inspect order flow".to_string());
        }
        for mut dtext in hud_text.iter_mut() {
            set_ui_text(&mut dtext, "Tap or click any trading pit on the floor to inspect order flow metrics.".to_string());
        }
    }
}

/// System: Updates the ViewMode button text and style depending on whether 2D floor or 3D pit is active
pub fn update_view_mode_button(
    view_mode: Res<FloorViewMode>,
    selection: Res<Selection>,
    orbit: Query<&OrbitCamera>,
    mut text_query: Query<&mut Text, With<ViewModeButtonText>>,
    mut button_query: Query<(&mut BackgroundColor, &mut BorderColor), With<ViewModeButton>>,
) {
    let Ok(mut text) = text_query.single_mut() else {
        return;
    };
    let Ok((mut bg, mut border)) = button_query.single_mut() else {
        return;
    };

    let selected_label = selection
        .selected_instrument
        .map(|i| i.symbol())
        .unwrap_or("PIT");

    let market_view = orbit.single().map(|o| o.market_view).unwrap_or(false);
    let (label, bg_color, border_color) = if !LEGACY_2D_FLOOR {
        if market_view {
            (format!("INSPECT {} PIT [SPACE]", selected_label), Color::srgba(0.08, 0.32, 0.50, 0.95), Color::srgb(0.22, 0.78, 0.98))
        } else {
            ("<- MARKET VIEW [ESC / M]".to_string(), Color::srgba(0.50, 0.22, 0.05, 0.95), Color::srgb(0.98, 0.68, 0.15))
        }
    } else if view_mode.is_3d() {
        (format!("<- FLOOR OVERVIEW [ESC] ({})", selected_label), Color::srgba(0.50, 0.22, 0.05, 0.95), Color::srgb(0.98, 0.68, 0.15))
    } else {
        (format!("3D {} PIT [SPACE]", selected_label), Color::srgba(0.08, 0.32, 0.50, 0.95), Color::srgb(0.22, 0.78, 0.98))
    };
    set_ui_text(&mut text, label);
    if bg.0 != bg_color {
        *bg = BackgroundColor(bg_color);
    }
    *border = BorderColor::all(border_color);
}

/// System: Updates the announcement banner toast at bottom of screen
pub fn update_announcement_toast(
    controller: Res<MarketScenarioController>,
    mut toast_container: Query<&mut Visibility, With<AnnouncementToastContainer>>,
    mut toast_text: Query<&mut Text, With<AnnouncementToastText>>,
) {
    let Ok(mut vis) = toast_container.single_mut() else {
        return;
    };
    let Ok(mut text) = toast_text.single_mut() else {
        return;
    };

    if controller.banner_timer > 0.0 {
        *vis = Visibility::Inherited;
        **text = controller.announcement_banner.clone();
    } else {
        *vis = Visibility::Hidden;
    }
}

/// System: Updates the F1 diagnostic overlay
pub fn update_debug_overlay(
    sim: Res<MarketActivitySim>,
    engines: Res<PitEngines>,
    selection: Res<Selection>,
    controller: Res<MarketScenarioController>,
    diagnostics: Res<bevy::diagnostic::DiagnosticsStore>,
    debug_state: Res<DebugOverlayState>,
    all_entities: Query<Entity>,
    traders: Query<&Trader>,
    camera_query: Query<&Projection, With<MainCamera>>,
    mut debug_text: Query<&mut Text, With<DebugOverlayText>>,
) {
    if !debug_state.visible {
        return;
    }
    let Ok(mut text) = debug_text.single_mut() else {
        return;
    };

    let fps = diagnostics
        .get(&bevy::diagnostic::FrameTimeDiagnosticsPlugin::FPS)
        .and_then(|d| d.smoothed())
        .unwrap_or(0.0);
    let frame_ms = diagnostics
        .get(&bevy::diagnostic::FrameTimeDiagnosticsPlugin::FRAME_TIME)
        .and_then(|d| d.smoothed())
        .unwrap_or(0.0);
    let entity_count = all_entities.iter().count();
    let total_traders = traders.iter().count();
    let zoom = camera_query
        .single()
        .map(|proj| match proj {
            Projection::Orthographic(ortho) => ortho.scale,
            _ => 1.0,
        })
        .unwrap_or(1.0);
    let selected = selection
        .selected_instrument
        .map(|i| i.symbol())
        .unwrap_or("None");

    let events_total: usize = engines.pits.iter().map(|p| p.events.len()).sum();
    **text = format!(
        "FPS: {:.0} ({:.1} ms/frame)\nEntities: {}\nScenario: {}\nTotal Floor Agents: {}\nSelected Pit: {}\nCamera Zoom: {:.2}x\nSim Clock: {:.1}s\nEngine events buffered: {}",
        fps,
        frame_ms,
        entity_count,
        controller.active_scenario.name(),
        total_traders,
        selected,
        zoom,
        sim.elapsed_secs,
        events_total,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_selection_default() {
        let sel = Selection::default();
        assert_eq!(sel.selected_instrument, Some(Instrument::Silver));
    }

    #[test]
    fn clicking_away_drops_the_wall_board_until_a_pit_is_picked() {
        let mut sel = Selection::default();
        assert!(!sel.wall_board_dismissed);
        sel.click_away();
        assert!(sel.wall_board_dismissed);
        assert_eq!(sel.selected_instrument, Some(Instrument::Silver), "the panels keep showing the last pit");
        sel.pick(Instrument::Gold);
        assert!(!sel.wall_board_dismissed);
        assert_eq!(sel.selected_instrument, Some(Instrument::Gold));
    }

    #[test]
    fn test_debug_overlay_default() {
        let debug = DebugOverlayState::default();
        assert!(!debug.visible);
    }

    #[test]
    fn test_hud_expanded_default() {
        let state = HudExpandedState::default();
        assert!(!state.visible);
    }

    #[test]
    fn test_role_key_visible_default() {
        let state = RoleKeyVisibleState::default();
        assert!(state.visible);
    }

    #[test]
    fn test_trigger_manual_shock() {
        let mut sim = MarketActivitySim::default();
        let mut engines = PitEngines::default();
        let mut controller = MarketScenarioController::default();
        let mut selection = Selection::default();

        trigger_manual_shock(Instrument::Gold, &mut sim, &mut engines, &mut controller, &mut selection);
        assert_eq!(selection.selected_instrument, Some(Instrument::Gold));
        assert!(!engines.pit(Instrument::Gold).participant(0).deck.is_empty(), "block paper lands on the rail");
        assert!(controller.announcement_banner.contains("GOLD"));
    }
}

