//! Panel layout: two flex columns of closeable panels under the top bar, a board chart that
//! expands to the viewport, and the open/closed state kept across reloads.
//!
//! Why columns: the panels used to be pinned at hand-picked offsets (`top 300 right 8`,
//! `left 8 top 176`) and overlapped as soon as one grew. Now each panel root carries
//! `Panel(id)` and `assemble_panel_columns` (PostStartup) parents it into the left or right
//! column in `PanelId` order, so the panels keep spawning from their own modules and the
//! column reflows when one is hidden. The columns clip at the bottom button rows so nothing
//! lands on the buttons at phone heights.
//!
//! One owner per flag: a panel's `Display` is set only by `sync_panels`, from the flag that
//! already lived for it (`ChartSettings::board_chart`, `DebugOverlayState`,
//! `RoleKeyVisibleState`, `HudExpandedState`) or from `PanelState` (status panel, expanded
//! board). The paper traders panel is the exception: `update_paper_panel` owns its display
//! from `PaperBook::show`, so this module only flips that flag.
//!
//! Closing: `PanelClose { target }` on any `Button` hides `target` when pressed. When the
//! target carries `Panel(id)` the matching open flag is cleared too, so the bottom-row button
//! shows it as closed and can reopen it; any other target is simply set to `Display::None`
//! (other modules can use it for their own panels). `spawn_close_button` draws the X.
//!
//! Persistence: `PanelPrefs` is the serialisable snapshot of every flag. On wasm it is written
//! to `localStorage["opencry.panels"]` whenever it changes and read back at startup; every
//! storage call ignores its `Result` (private mode, quota, blocked storage). Native ignores it.

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::camera::{FloorViewMode, OrbitCamera, PitCamera3d};
use crate::candles::ChartSettings;
use crate::paper::PaperBook;
use crate::scenario::MarketScenarioController;
use crate::tape_chart::{BOARD_PANEL_W, ChartLayout};
use crate::ui::{
    DebugOverlayState, HudDetailsContainer, HudExpandedState, RoleKeyVisibleState,
    TouchButtonAction,
};

/// The columns start under the top bar...
pub const COLUMN_TOP: f32 = 58.0;
/// ...sit this far in from the window's sides...
pub const COLUMN_SIDE: f32 = 8.0;
/// ...and stop above the bottom button rows (three rows plus the bar's padding and gaps).
pub const COLUMN_BOTTOM: f32 = 140.0;
/// Vertical gap between panels in a column.
pub const COLUMN_GAP: f32 = 6.0;
/// The left column is as wide as the board panel so the two sides match.
pub const LEFT_COLUMN_W: f32 = BOARD_PANEL_W;

pub const STORAGE_KEY: &str = "opencry.panels";

/// Every closeable panel. The order within each side is the top-to-bottom order.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PanelId {
    /// The 2D candle board for the inspected pit (`pit_3d_overlay`).
    Board,
    /// Diagnostics / details (F1).
    Details,
    /// Participant role key (K).
    RoleKey,
    /// Paper traders leaderboard (T).
    Traders,
    /// HUD status: the `[LIVE] XAG/USD ... STEADY AUCTION` line and its details drawer.
    Status,
    /// The trade ticket: buy, sell or close a paper trade on the pit in view (`ticket`).
    Ticket,
    /// Your paper trades today, open and closed, with the totals (`positions`).
    Positions,
    /// Bitcoin's level 2 order book from Kraken: the heatmap and big-order events (`depth`).
    Depth,
}

impl PanelId {
    pub const RIGHT: [PanelId; 5] = [
        PanelId::Board,
        PanelId::Ticket,
        PanelId::Positions,
        PanelId::Details,
        PanelId::RoleKey,
    ];
    pub const LEFT: [PanelId; 3] = [PanelId::Traders, PanelId::Status, PanelId::Depth];
}

/// Marker on a panel's root node.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct Panel(pub PanelId);

/// A close button: pressing it hides `target` (see the module docs).
#[derive(Component, Debug, Clone, Copy)]
pub struct PanelClose {
    pub target: Entity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnSide {
    Left,
    Right,
}

/// Marker on a column container.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct PanelColumn(pub ColumnSide);

/// The label inside the board's Expand / Restore button.
#[derive(Component)]
pub struct BoardExpandText;

/// The flags that had no home before: the status panel and the expanded board.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub struct PanelState {
    pub status: bool,
    pub expanded: bool,
    pub ticket: bool,
    pub positions: bool,
    pub depth: bool,
}

impl Default for PanelState {
    fn default() -> Self {
        Self {
            status: true,
            expanded: false,
            ticket: true,
            positions: true,
            depth: true,
        }
    }
}

impl PanelState {
    /// E / the board's button: expand or restore. Expanding a closed board opens it first.
    pub fn toggle_expanded(
        &mut self,
        settings: &mut ChartSettings,
        controller: &mut MarketScenarioController,
    ) {
        self.expanded = !self.expanded;
        if self.expanded && !settings.board_chart {
            settings.board_chart = true;
        }
        controller.announcement_banner = if self.expanded {
            "BOARD EXPANDED  (E / ESC restores)".into()
        } else {
            "BOARD RESTORED".into()
        };
        controller.banner_timer = 1.5;
    }
}

// ---------------------------------------------------------------------------
// Persisted snapshot
// ---------------------------------------------------------------------------

/// Open/closed state of every panel plus the expanded board, as saved. Missing fields take
/// the defaults, so an older saved layout still loads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PanelPrefs {
    pub board: bool,
    pub traders: bool,
    pub details: bool,
    pub role_key: bool,
    pub status: bool,
    pub expanded: bool,
    pub ticket: bool,
    pub positions: bool,
    pub depth: bool,
}

impl Default for PanelPrefs {
    fn default() -> Self {
        Self {
            board: true,
            traders: true,
            details: false,
            role_key: true,
            status: true,
            expanded: false,
            ticket: true,
            positions: true,
            depth: true,
        }
    }
}

impl PanelPrefs {
    pub fn capture(
        settings: &ChartSettings,
        book: &PaperBook,
        debug: &DebugOverlayState,
        key: &RoleKeyVisibleState,
        state: &PanelState,
    ) -> Self {
        Self {
            board: settings.board_chart,
            traders: book.show,
            details: debug.visible,
            role_key: key.visible,
            status: state.status,
            expanded: state.expanded,
            ticket: state.ticket,
            positions: state.positions,
            depth: state.depth,
        }
    }

    /// Write the snapshot back into the live flags. An expanded board that is closed makes
    /// no sense (nothing would show), so `expanded` only survives with `board`.
    pub fn apply(
        &self,
        settings: &mut ChartSettings,
        book: &mut PaperBook,
        debug: &mut DebugOverlayState,
        key: &mut RoleKeyVisibleState,
        state: &mut PanelState,
    ) {
        settings.board_chart = self.board;
        book.show = self.traders;
        debug.visible = self.details;
        key.visible = self.role_key;
        state.status = self.status;
        state.expanded = self.expanded && self.board;
        state.ticket = self.ticket;
        state.positions = self.positions;
        state.depth = self.depth;
    }

    pub fn is_open(&self, id: PanelId) -> bool {
        match id {
            PanelId::Board => self.board,
            PanelId::Details => self.details,
            PanelId::RoleKey => self.role_key,
            PanelId::Traders => self.traders,
            PanelId::Status => self.status,
            PanelId::Ticket => self.ticket,
            PanelId::Positions => self.positions,
            PanelId::Depth => self.depth,
        }
    }
}

/// Pure: whether a panel is on screen. `board_in_view` is false in the market view and in
/// 2D, where the board would only cover a pit; the expanded board hides the other panels
/// (they would sit under it) but only while it is itself in view.
pub fn panel_shown(id: PanelId, prefs: &PanelPrefs, board_in_view: bool) -> bool {
    let expanded = prefs.expanded && prefs.board && board_in_view;
    match id {
        PanelId::Board => prefs.board && board_in_view,
        PanelId::Traders => prefs.traders && !expanded,
        _ => prefs.is_open(id) && !expanded,
    }
}

#[cfg(target_arch = "wasm32")]
fn storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok().flatten()
}

/// Save to `localStorage` on wasm; a no-op natively. Failures are ignored on purpose.
pub fn save_prefs(prefs: &PanelPrefs) {
    #[cfg(target_arch = "wasm32")]
    {
        if let (Some(store), Ok(json)) = (storage(), serde_json::to_string(prefs)) {
            let _ = store.set_item(STORAGE_KEY, &json);
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = prefs;
    }
}

/// The saved layout, if there is one that parses. Always `None` natively.
pub fn load_prefs() -> Option<PanelPrefs> {
    #[cfg(target_arch = "wasm32")]
    {
        let json = storage()?.get_item(STORAGE_KEY).ok().flatten()?;
        serde_json::from_str(&json).ok()
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        None
    }
}

// ---------------------------------------------------------------------------
// Spawning
// ---------------------------------------------------------------------------

/// A small X button that hides `target` when pressed. Call inside the panel's header row.
pub fn spawn_close_button(parent: &mut ChildSpawnerCommands, target: Entity) {
    parent
        .spawn((
            Button,
            PanelClose { target },
            Node {
                padding: UiRect::axes(Val::Px(6.0), Val::Px(1.5)),
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(Val::Px(3.0)),
                margin: UiRect::left(Val::Px(4.0)),
                flex_shrink: 0.0,
                ..default()
            },
            BackgroundColor(Color::srgba(0.15, 0.20, 0.30, 0.80)),
            BorderColor::all(Color::srgba(0.50, 0.60, 0.75, 0.60)),
        ))
        .with_children(|b| {
            b.spawn((
                Text::new("X"),
                TextFont {
                    font_size: 9.5,
                    ..default()
                },
                TextColor(Color::srgb(0.85, 0.90, 0.98)),
            ));
        });
}

/// A panel header row: a title on the left, the close button on the right, and room for
/// extra buttons between them (spawned by `extra`).
pub fn spawn_panel_header(
    parent: &mut ChildSpawnerCommands,
    target: Entity,
    title: impl Into<String>,
    color: Color,
    extra: impl FnOnce(&mut ChildSpawnerCommands),
) {
    parent
        .spawn(Node {
            flex_direction: FlexDirection::Row,
            justify_content: JustifyContent::SpaceBetween,
            align_items: AlignItems::Center,
            width: Val::Percent(100.0),
            column_gap: Val::Px(6.0),
            margin: UiRect::bottom(Val::Px(2.0)),
            ..default()
        })
        .with_children(|row| {
            row.spawn((
                Text::new(title),
                TextFont {
                    font_size: 11.0,
                    ..default()
                },
                TextColor(color),
                Node {
                    flex_grow: 1.0,
                    flex_shrink: 1.0,
                    min_width: Val::Px(0.0),
                    ..default()
                },
            ));
            extra(row);
            spawn_close_button(row, target);
        });
}

fn column_node(side: ColumnSide) -> Node {
    let mut node = Node {
        position_type: PositionType::Absolute,
        top: Val::Px(COLUMN_TOP),
        bottom: Val::Px(COLUMN_BOTTOM),
        flex_direction: FlexDirection::Column,
        align_items: AlignItems::Stretch,
        row_gap: Val::Px(COLUMN_GAP),
        overflow: Overflow::clip(),
        ..default()
    };
    match side {
        ColumnSide::Left => {
            node.left = Val::Px(COLUMN_SIDE);
            node.width = Val::Px(LEFT_COLUMN_W);
        }
        ColumnSide::Right => {
            node.right = Val::Px(COLUMN_SIDE);
            node.width = Val::Px(BOARD_PANEL_W);
        }
    }
    node
}

/// Startup: the two empty columns. They do not block clicks, so the floor under the empty
/// part of a column still picks pits.
pub fn setup_panel_columns(mut commands: Commands) {
    // Above every floor-attached label (pit nameplates 3, deal and avatar labels 2).
    for (side, z) in [(ColumnSide::Left, 8), (ColumnSide::Right, 7)] {
        commands.spawn((
            PanelColumn(side),
            column_node(side),
            Pickable {
                should_block_lower: false,
                is_hoverable: false,
            },
            GlobalZIndex(z),
        ));
    }
}

/// PostStartup: parent every `Panel` root into its column, top to bottom in `PanelId` order.
pub fn assemble_panel_columns(
    mut commands: Commands,
    columns: Query<(Entity, &PanelColumn)>,
    panels: Query<(Entity, &Panel)>,
) {
    for (column, side) in columns.iter() {
        let order: &[PanelId] = match side.0 {
            ColumnSide::Left => &PanelId::LEFT,
            ColumnSide::Right => &PanelId::RIGHT,
        };
        let children: Vec<Entity> = order
            .iter()
            .filter_map(|id| panels.iter().find(|(_, p)| p.0 == *id).map(|(e, _)| e))
            .collect();
        if !children.is_empty() {
            commands.entity(column).add_children(&children);
        }
    }
}

/// Startup: apply the saved layout, if any.
pub fn load_panel_prefs(
    mut settings: ResMut<ChartSettings>,
    mut book: ResMut<PaperBook>,
    mut debug: ResMut<DebugOverlayState>,
    mut key: ResMut<RoleKeyVisibleState>,
    mut state: ResMut<PanelState>,
) {
    if let Some(prefs) = load_prefs() {
        info!("panels: restored layout {prefs:?}");
        prefs.apply(&mut settings, &mut book, &mut debug, &mut key, &mut state);
    }
}

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

/// System: the chart's pixel size from the window and the expanded flag.
pub fn update_chart_layout(
    windows: Query<&Window>,
    state: Res<PanelState>,
    mut layout: ResMut<ChartLayout>,
) {
    let Ok(window) = windows.single() else { return };
    let want = ChartLayout::for_viewport(window.width(), window.height(), state.expanded);
    if *layout != want {
        *layout = want;
    }
}

fn set_display(node: &mut Node, shown: bool) {
    let want = if shown { Display::Flex } else { Display::None };
    if node.display != want {
        node.display = want;
    }
}

/// System: every panel's `Display` from its flag, the board's width from the chart layout,
/// and the columns' shape (the right column spans the window while the board is expanded).
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn sync_panels(
    view_mode: Res<FloorViewMode>,
    camera: Query<&OrbitCamera, With<PitCamera3d>>,
    settings: Res<ChartSettings>,
    book: Res<PaperBook>,
    debug: Res<DebugOverlayState>,
    key: Res<RoleKeyVisibleState>,
    hud: Res<HudExpandedState>,
    state: Res<PanelState>,
    layout: Res<ChartLayout>,
    mut panels: Query<(&Panel, &mut Node), (Without<PanelColumn>, Without<HudDetailsContainer>)>,
    mut columns: Query<(&PanelColumn, &mut Node), (Without<Panel>, Without<HudDetailsContainer>)>,
    mut drawer: Query<
        &mut Node,
        (
            With<HudDetailsContainer>,
            Without<Panel>,
            Without<PanelColumn>,
        ),
    >,
) {
    let prefs = PanelPrefs::capture(&settings, &book, &debug, &key, &state);
    let market_view = camera.single().map(|o| o.market_view).unwrap_or(false);
    let board_in_view = view_mode.is_3d() && !market_view;
    let expanded = prefs.expanded && prefs.board && board_in_view;

    for (panel, mut node) in panels.iter_mut() {
        match panel.0 {
            // `update_paper_panel` owns this one's display from `PaperBook::show`.
            PanelId::Traders => {}
            PanelId::Board => {
                set_display(
                    &mut node,
                    panel_shown(PanelId::Board, &prefs, board_in_view),
                );
                let want = Val::Px(layout.panel_w());
                if node.width != want {
                    node.width = want;
                }
            }
            id => set_display(&mut node, panel_shown(id, &prefs, board_in_view)),
        }
    }
    for (column, mut node) in columns.iter_mut() {
        match column.0 {
            ColumnSide::Left => set_display(&mut node, !expanded),
            ColumnSide::Right => {
                let (left, width) = if expanded {
                    (Val::Px(COLUMN_SIDE), Val::Auto)
                } else {
                    (Val::Auto, Val::Px(BOARD_PANEL_W))
                };
                if node.left != left || node.width != width {
                    node.left = left;
                    node.width = width;
                }
            }
        }
    }
    for mut node in drawer.iter_mut() {
        set_display(&mut node, hud.visible);
    }
}

/// System: close buttons.
#[allow(clippy::too_many_arguments)]
pub fn handle_panel_close(
    buttons: Query<(&Interaction, &PanelClose), (Changed<Interaction>, With<Button>)>,
    panels: Query<&Panel>,
    mut nodes: Query<&mut Node>,
    mut settings: ResMut<ChartSettings>,
    mut book: ResMut<PaperBook>,
    mut debug: ResMut<DebugOverlayState>,
    mut key: ResMut<RoleKeyVisibleState>,
    mut state: ResMut<PanelState>,
    mut controller: ResMut<MarketScenarioController>,
) {
    for (interaction, close) in buttons.iter() {
        if *interaction != Interaction::Pressed {
            continue;
        }
        match panels.get(close.target) {
            Ok(panel) => match panel.0 {
                PanelId::Board => {
                    state.expanded = false;
                    if settings.board_chart {
                        settings.toggle_board(&mut controller);
                    }
                }
                PanelId::Traders => {
                    if book.show {
                        book.toggle_show(&mut controller);
                    }
                }
                PanelId::Details => debug.visible = false,
                PanelId::RoleKey => key.visible = false,
                PanelId::Status => state.status = false,
                PanelId::Ticket => state.ticket = false,
                PanelId::Positions => state.positions = false,
                PanelId::Depth => state.depth = false,
            },
            Err(_) => {
                if let Ok(mut node) = nodes.get_mut(close.target) {
                    node.display = Display::None;
                }
            }
        }
    }
}

/// System: the bottom-row Status button and the board's Expand button.
pub fn handle_panel_buttons(
    interaction: Query<(&Interaction, &TouchButtonAction), (Changed<Interaction>, With<Button>)>,
    mut state: ResMut<PanelState>,
    mut settings: ResMut<ChartSettings>,
    mut controller: ResMut<MarketScenarioController>,
) {
    for (i, action) in interaction.iter() {
        if *i != Interaction::Pressed {
            continue;
        }
        match action {
            TouchButtonAction::ToggleStatus => state.status = !state.status,
            TouchButtonAction::ToggleTicket => state.ticket = !state.ticket,
            TouchButtonAction::TogglePositions => state.positions = !state.positions,
            TouchButtonAction::ToggleDepth => state.depth = !state.depth,
            TouchButtonAction::BoardExpand => state.toggle_expanded(&mut settings, &mut controller),
            _ => {}
        }
    }
}

/// System: E expands or restores the board. (Escape restores too; `handle_keyboard_actions`
/// owns Escape because it otherwise sends the camera to the market view.)
pub fn handle_panel_keys(
    keyboard: Res<ButtonInput<KeyCode>>,
    mut state: ResMut<PanelState>,
    mut settings: ResMut<ChartSettings>,
    mut controller: ResMut<MarketScenarioController>,
) {
    if keyboard.just_pressed(KeyCode::KeyE) {
        state.toggle_expanded(&mut settings, &mut controller);
    }
}

/// System: the panel buttons light up while their panel is open; the board's Expand button
/// reads Restore while expanded.
pub fn update_panel_button_styles(
    debug: Res<DebugOverlayState>,
    key: Res<RoleKeyVisibleState>,
    state: Res<PanelState>,
    mut buttons: Query<(&TouchButtonAction, &mut BackgroundColor, &mut BorderColor), With<Button>>,
    mut labels: Query<&mut Text, With<BoardExpandText>>,
) {
    if !(debug.is_changed() || key.is_changed() || state.is_changed()) {
        return;
    }
    for (action, mut bg, mut border) in buttons.iter_mut() {
        let on = match action {
            TouchButtonAction::ToggleDiagnostics => debug.visible,
            TouchButtonAction::ToggleRoleKey => key.visible,
            TouchButtonAction::ToggleStatus => state.status,
            TouchButtonAction::ToggleTicket => state.ticket,
            TouchButtonAction::TogglePositions => state.positions,
            TouchButtonAction::ToggleDepth => state.depth,
            TouchButtonAction::BoardExpand => state.expanded,
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
    for mut text in labels.iter_mut() {
        crate::pit::set_ui_text(
            &mut text,
            if state.expanded {
                "RESTORE [E]".to_string()
            } else {
                "EXPAND [E]".to_string()
            },
        );
    }
}

/// System: save the layout whenever a flag changes (wasm only; see `save_prefs`).
pub fn persist_panel_prefs(
    settings: Res<ChartSettings>,
    book: Res<PaperBook>,
    debug: Res<DebugOverlayState>,
    key: Res<RoleKeyVisibleState>,
    state: Res<PanelState>,
    mut last: Local<Option<PanelPrefs>>,
) {
    if !(settings.is_changed()
        || book.is_changed()
        || debug.is_changed()
        || key.is_changed()
        || state.is_changed())
    {
        return;
    }
    let prefs = PanelPrefs::capture(&settings, &book, &debug, &key, &state);
    if *last == Some(prefs) {
        return;
    }
    *last = Some(prefs);
    save_prefs(&prefs);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefs_round_trip_through_json_and_missing_fields_take_defaults() {
        let p = PanelPrefs {
            board: false,
            traders: true,
            details: true,
            role_key: false,
            status: false,
            expanded: true,
            ticket: false,
            positions: false,
            depth: false,
        };
        let json = serde_json::to_string(&p).unwrap();
        assert!(json.contains("\"role_key\":false"));
        let back: PanelPrefs = serde_json::from_str(&json).unwrap();
        assert_eq!(back, p);

        let partial: PanelPrefs = serde_json::from_str(r#"{"details":true}"#).unwrap();
        assert_eq!(
            partial,
            PanelPrefs {
                details: true,
                ..PanelPrefs::default()
            }
        );
        let empty: PanelPrefs = serde_json::from_str("{}").unwrap();
        assert_eq!(empty, PanelPrefs::default());
        assert!(serde_json::from_str::<PanelPrefs>("not json").is_err());
        assert!(load_prefs().is_none(), "native has no storage");
    }

    #[test]
    fn prefs_capture_and_apply_mirror_the_live_flags() {
        let mut settings = ChartSettings::default();
        let mut book = PaperBook::default();
        let mut debug = DebugOverlayState::default();
        let mut key = RoleKeyVisibleState::default();
        let mut state = PanelState::default();
        assert_eq!(
            PanelPrefs::capture(&settings, &book, &debug, &key, &state),
            PanelPrefs::default(),
            "defaults agree with the resources"
        );

        let saved = PanelPrefs {
            board: false,
            traders: false,
            details: true,
            role_key: false,
            status: false,
            expanded: true,
            ticket: false,
            positions: false,
            depth: false,
        };
        saved.apply(&mut settings, &mut book, &mut debug, &mut key, &mut state);
        assert!(
            !settings.board_chart && !book.show && debug.visible && !key.visible && !state.status
        );
        assert!(
            !state.expanded,
            "an expanded but closed board is not restored as expanded"
        );
        let mut again = saved;
        again.board = true;
        again.apply(&mut settings, &mut book, &mut debug, &mut key, &mut state);
        assert!(state.expanded);
        assert_eq!(
            PanelPrefs::capture(&settings, &book, &debug, &key, &state),
            again
        );
    }

    #[test]
    fn panel_visibility_follows_the_flags_the_view_and_the_expanded_board() {
        let d = PanelPrefs::default();
        assert!(panel_shown(PanelId::Board, &d, true));
        assert!(
            !panel_shown(PanelId::Board, &d, false),
            "no board in the market view"
        );
        assert!(
            panel_shown(PanelId::RoleKey, &d, false),
            "the key stays in the market view"
        );
        assert!(
            !panel_shown(PanelId::Details, &d, true),
            "details closed by default"
        );
        assert!(panel_shown(PanelId::Traders, &d, true) && panel_shown(PanelId::Status, &d, true));

        let e = PanelPrefs {
            expanded: true,
            details: true,
            ..d
        };
        for id in [
            PanelId::Details,
            PanelId::RoleKey,
            PanelId::Traders,
            PanelId::Status,
        ] {
            assert!(
                !panel_shown(id, &e, true),
                "{id:?} hides under the expanded board"
            );
            assert!(
                panel_shown(id, &e, false),
                "{id:?} comes back in the market view"
            );
        }
        assert!(panel_shown(PanelId::Board, &e, true));
        let closed = PanelPrefs { board: false, ..e };
        assert!(!panel_shown(PanelId::Board, &closed, true));
        assert!(
            panel_shown(PanelId::RoleKey, &closed, true),
            "a closed board cannot hide anything"
        );
    }

    #[test]
    fn expanding_opens_a_closed_board_and_closing_the_board_is_not_expanded() {
        let mut settings = ChartSettings::default();
        let mut controller = MarketScenarioController::default();
        let mut state = PanelState::default();
        settings.board_chart = false;
        state.toggle_expanded(&mut settings, &mut controller);
        assert!(state.expanded && settings.board_chart);
        assert!(controller.announcement_banner.contains("EXPANDED"));
        state.toggle_expanded(&mut settings, &mut controller);
        assert!(
            !state.expanded && settings.board_chart,
            "restoring leaves the board open"
        );
    }

    #[test]
    fn column_order_is_traders_over_status_and_board_over_details_over_key() {
        assert_eq!(
            PanelId::LEFT,
            [PanelId::Traders, PanelId::Status, PanelId::Depth]
        );
        assert_eq!(
            PanelId::RIGHT,
            [
                PanelId::Board,
                PanelId::Ticket,
                PanelId::Positions,
                PanelId::Details,
                PanelId::RoleKey
            ]
        );
        let right = column_node(ColumnSide::Right);
        assert_eq!(
            (right.top, right.bottom, right.right),
            (
                Val::Px(COLUMN_TOP),
                Val::Px(COLUMN_BOTTOM),
                Val::Px(COLUMN_SIDE)
            )
        );
        assert_eq!(right.width, Val::Px(BOARD_PANEL_W));
        let left = column_node(ColumnSide::Left);
        assert_eq!(left.left, Val::Px(COLUMN_SIDE));
        assert_eq!(
            left.overflow,
            Overflow::clip(),
            "columns clip above the bottom buttons"
        );
    }
}
