//! The OpenCry floor: a Bevy app over the library in `lib.rs`.

use opencry_visual::{
    camera, candles, depth, desk_feed, jev, levels, live_feed, market, panels, paper,
    paper_avatars, pit, pit_3d, pit_3d_overlay, pit_engine, positions, scenario, tape_chart,
    ticket, trader, ui, unimate, wall_board, web_chart,
};

use bevy::diagnostic::FrameTimeDiagnosticsPlugin;
use bevy::prelude::*;
use camera::{
    CameraRequest, FloorViewMode, LEGACY_2D_FLOOR, camera_orbit_3d, camera_pan_zoom, setup_camera,
    sync_camera_view_mode,
};
use candles::{
    CandleStore, ChartSettings, handle_chart_buttons, handle_chart_keys, update_candles,
    update_chart_button_styles,
};
use desk_feed::{DeskFeed, gate_paper_book, poll_desk_board};
use jev::{
    JevDesk, handle_jev_buttons, handle_jev_keys, update_jev_board, update_jev_button_style,
    update_jev_desk,
};
use levels::{LevelStore, update_board_levels, update_levels};
use live_feed::{LiveFeed, poll_live_feed, setup_feed_status, update_feed_status_ui};
use market::{MarketActivitySim, update_market_simulation};
use panels::{
    PanelState, assemble_panel_columns, handle_panel_buttons, handle_panel_close,
    handle_panel_keys, load_panel_prefs, persist_panel_prefs, setup_panel_columns, sync_panels,
    update_chart_layout, update_panel_button_styles,
};
use paper::{
    PaperBook, handle_paper_buttons, handle_paper_keys, handle_paper_rows, setup_paper_panel,
    update_paper_book, update_paper_button_style, update_paper_panel,
};
use paper_avatars::{
    AvatarPicking, PaperAvatars, TraderDetail, assign_genome_avatars, follow_clicked_trader,
    handle_avatar_clicks, move_paper_avatars, project_avatar_labels, separate_paper_avatars,
    setup_paper_avatars, show_seated_avatars, update_paper_avatars, update_trader_detail_panel,
};
use pit::{
    spawn_pits, update_decibel_equalizers, update_institutional_waves, update_pit_border_visuals,
    update_pit_floor_glow, update_pit_labels, update_podium_price_displays,
};
use pit_3d::{
    setup_pit_3d, sync_pit_3d_proxies, sync_pit_3d_rig_focus, sync_pit_3d_wells,
    sync_pit_3d_with_market,
};
use pit_3d_overlay::{
    setup_pit_3d_overlay, spawn_3d_deal_labels, tag_price_arrows, update_pit_3d_nameplates,
    update_pit_3d_overlay,
};
use pit_engine::{PitEngines, step_pit_engines};
use scenario::{MarketScenarioController, handle_scenario_hotkeys, update_scenario_controller};
use tape_chart::{ChartLayout, TapeHistory, record_tape, update_tape_chart};
use trader::{
    sync_crowd_density, update_trade_shouts, update_trader_hand_signals, update_trader_movement,
};
use ui::{
    DebugOverlayState, HudExpandedState, RoleKeyVisibleState, Selection, handle_keyboard_actions,
    handle_pit_selection_clicks, handle_touch_buttons, setup_ui, update_announcement_toast,
    update_debug_overlay, update_hud_info, update_scenario_button_styles, update_view_mode_button,
};
use unimate::{UnimateMaterialCache, UnimateMeshCache, update_unimate_kinematics};
use wall_board::{setup_wall_board, update_wall_board};
use web_chart::sync_web_chart;

fn main() {
    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: "OpenCry Market Floor - Phase 2 Living Floor".into(),
            resolution: (1280, 800).into(),
            fit_canvas_to_parent: true,
            prevent_default_event_handling: true,
            ..default()
        }),
        ..default()
    }))
    .add_plugins(FrameTimeDiagnosticsPlugin::default())
    .init_resource::<MarketActivitySim>()
    .init_resource::<MarketScenarioController>()
    .init_resource::<PitEngines>()
    .init_resource::<Selection>()
    .init_resource::<FloorViewMode>()
    .init_resource::<CameraRequest>()
    .init_resource::<LiveFeed>()
    .init_resource::<TapeHistory>()
    .init_resource::<CandleStore>()
    .init_resource::<ChartSettings>()
    .init_resource::<JevDesk>()
    .init_resource::<LevelStore>()
    .init_resource::<PaperBook>()
    .init_resource::<DeskFeed>()
    .init_resource::<PaperAvatars>()
    .init_resource::<AvatarPicking>()
    .init_resource::<TraderDetail>()
    .init_resource::<DebugOverlayState>()
    .init_resource::<HudExpandedState>()
    .init_resource::<RoleKeyVisibleState>()
    .init_resource::<PanelState>()
    .init_resource::<ticket::Ticket>()
    .init_resource::<positions::PositionsFeed>()
    .init_resource::<depth::DepthFeed>()
    .init_resource::<ChartLayout>()
    .init_resource::<UnimateMeshCache>()
    .init_resource::<UnimateMaterialCache>()
    .add_systems(
        Startup,
        (
            setup_camera,
            setup_ui,
            setup_pit_3d,
            setup_pit_3d_overlay,
            setup_feed_status,
            setup_wall_board,
            setup_paper_panel,
            ticket::setup_ticket_panel,
            positions::setup_positions_panel,
            depth::setup_depth_panel,
            depth::setup_depth_walls,
            setup_panel_columns,
            load_panel_prefs,
        ),
    )
    // Panels spawn from their own modules; the columns adopt them once they all exist.
    .add_systems(PostStartup, (tag_price_arrows, assemble_panel_columns));

    // The retired 2D isometric floor: sprites, labels, shouts and its camera controls.
    // Compiled in behind `legacy_2d_floor`; otherwise none of it is spawned or scheduled.
    if LEGACY_2D_FLOOR {
        app.add_systems(Startup, spawn_pits);
        app.add_systems(
            Update,
            (
                update_pit_labels,
                update_pit_floor_glow,
                update_podium_price_displays,
                update_decibel_equalizers,
                update_pit_border_visuals,
                update_institutional_waves,
                sync_crowd_density,
                update_trader_movement.after(step_pit_engines),
                update_trader_hand_signals,
                update_trade_shouts,
                camera_pan_zoom,
                handle_pit_selection_clicks,
            ),
        );
    }

    // Native profiling hooks:
    //   OPENCRY_LOG_FPS=1          print frame-time diagnostics to the console
    //   OPENCRY_START_3D=market    start in the 3D market view (or `pit` for the inspected pit)
    //   OPENCRY_EXIT_AFTER_SECS=30 quit cleanly after N seconds so trace files flush
    #[cfg(not(target_arch = "wasm32"))]
    {
        if std::env::var_os("OPENCRY_LOG_FPS").is_some() {
            app.add_plugins(bevy::diagnostic::LogDiagnosticsPlugin::default());
            app.add_plugins(bevy::diagnostic::EntityCountDiagnosticsPlugin::default());
        }
        if let Ok(mode) = std::env::var("OPENCRY_START_3D") {
            let market = mode == "market";
            app.add_systems(
                Startup,
                move |mut view: ResMut<FloorViewMode>,
                      mut cams: Query<&mut camera::OrbitCamera>| {
                    *view = FloorViewMode::PitDeepDive3D;
                    if market {
                        for mut c in cams.iter_mut() {
                            c.market_view();
                            c.radius = c.goal_radius;
                            c.pitch = c.goal_pitch;
                            c.target = c.goal_target;
                        }
                    }
                },
            );
        }
        if let Some(secs) = std::env::var("OPENCRY_EXIT_AFTER_SECS")
            .ok()
            .and_then(|v| v.parse::<f32>().ok())
        {
            app.add_systems(
                Update,
                move |time: Res<Time>, mut exit: MessageWriter<AppExit>| {
                    if time.elapsed_secs() > secs {
                        exit.write(AppExit::Success);
                    }
                },
            );
        }
    }

    app.add_systems(
        Update,
        (
            update_market_simulation,
            update_scenario_controller,
            poll_live_feed,
            step_pit_engines,
        )
            .chain(),
    )
    .add_systems(
        Update,
        (
            camera_orbit_3d,
            sync_camera_view_mode,
            handle_touch_buttons,
            handle_keyboard_actions,
            handle_scenario_hotkeys,
        ),
    )
    .add_systems(
        Update,
        (
            update_hud_info,
            update_view_mode_button,
            update_debug_overlay,
            update_announcement_toast,
            update_scenario_button_styles,
            update_unimate_kinematics,
            sync_pit_3d_rig_focus,
            sync_pit_3d_with_market.after(sync_pit_3d_rig_focus),
            sync_pit_3d_proxies,
            sync_pit_3d_wells,
            spawn_3d_deal_labels.after(step_pit_engines),
            update_feed_status_ui,
            update_pit_3d_overlay,
            update_pit_3d_nameplates,
        ),
    )
    // Charts: candles from the feed, the board chart, the wall board and the web chart.
    .add_systems(
        Update,
        (
            record_tape,
            update_candles,
            handle_chart_keys,
            handle_chart_buttons,
            update_chart_button_styles,
            update_tape_chart.after(record_tape).after(update_candles),
            update_wall_board
                .after(record_tape)
                .after(update_candles)
                .after(update_levels)
                .after(camera_orbit_3d),
            update_levels.after(update_candles),
            update_board_levels
                .after(update_levels)
                .after(update_tape_chart),
            sync_web_chart,
        )
            .after(step_pit_engines),
    )
    // Panels: columns, close buttons, the expanded board and the saved layout.
    .add_systems(
        Update,
        (
            update_chart_layout,
            sync_panels.after(update_chart_layout),
            handle_panel_close,
            handle_panel_buttons,
            handle_panel_keys,
            update_panel_button_styles,
            persist_panel_prefs,
        ),
    )
    // Jev: the desk reads the focused pit, the board shows it, the crowd hears it.
    .add_systems(
        Update,
        (
            update_jev_desk,
            update_jev_board.after(update_jev_desk),
            handle_jev_keys,
            handle_jev_buttons,
            update_jev_button_style,
        )
            .after(step_pit_engines)
            .after(update_candles),
    )
    // The paper book: nine traders on the same tape, scored after the spread. While the
    // headless desk is alive (relay `/paper/board`) the panel shows its board instead and
    // the in-tab book opens nothing.
    .add_systems(
        Update,
        (
            poll_desk_board,
            gate_paper_book
                .after(poll_desk_board)
                .before(update_paper_book),
            update_paper_book
                .after(update_jev_desk)
                .after(update_levels),
            update_paper_panel.after(update_paper_book),
            ticket::handle_ticket_buttons,
            ticket::update_ticket_panel,
            positions::poll_positions,
            positions::update_positions_panel.after(positions::poll_positions),
            depth::poll_depth_feed,
            depth::update_depth_panel.after(depth::poll_depth_feed),
            depth::update_depth_walls.after(depth::poll_depth_feed),
            handle_paper_rows,
            handle_paper_keys,
            handle_paper_buttons,
            update_paper_button_style,
        ),
    )
    // The paper traders' avatars on the floor: snapshots from the desk or the in-tab book,
    // the walk, the overhead labels (projected after the orbit, same frame) and the
    // click-to-open detail panel.
    .add_systems(Startup, setup_paper_avatars)
    .add_systems(
        Update,
        (
            update_paper_avatars
                .after(poll_desk_board)
                .after(update_paper_book),
            separate_paper_avatars.after(update_paper_avatars),
            move_paper_avatars.after(separate_paper_avatars),
            assign_genome_avatars.after(update_paper_avatars),
            show_seated_avatars.after(assign_genome_avatars),
            project_avatar_labels
                .after(move_paper_avatars)
                .after(camera_orbit_3d),
            handle_avatar_clicks.after(camera_orbit_3d),
            follow_clicked_trader
                .after(handle_avatar_clicks)
                .after(move_paper_avatars),
            paper_avatars::scroll_trader_detail.after(handle_avatar_clicks),
            paper_avatars::update_trader_detail_scrollbar
                .after(paper_avatars::scroll_trader_detail),
            update_trader_detail_panel
                .after(handle_avatar_clicks)
                .after(update_paper_avatars),
        ),
    )
    .run();
}
