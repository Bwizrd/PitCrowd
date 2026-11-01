use bevy::prelude::*;
use crate::market::Instrument;
use crate::camera::FloorViewMode;
use crate::pit_engine::{PitEngines, PitSignals};

/// Standard isometric tile dimension ratio (2:1 width to height, 30-degree dimetric)
pub const TILE_WIDTH: f32 = 120.0;
pub const TILE_HEIGHT: f32 = 60.0;

/// Pure function: Converts 2D logical grid coordinates to 2.5D isometric screen coordinates.
pub fn grid_to_isometric(grid_pos: Vec2, tile_size: Vec2) -> Vec2 {
    let half_w = tile_size.x * 0.5;
    let half_h = tile_size.y * 0.5;
    Vec2::new(
        (grid_pos.x - grid_pos.y) * half_w,
        (grid_pos.x + grid_pos.y) * half_h,
    )
}

/// Pure function: Inverts isometric screen coordinates back to logical grid coordinates.
pub fn isometric_to_grid(iso_pos: Vec2, tile_size: Vec2) -> Vec2 {
    let x_term = iso_pos.x / tile_size.x;
    let y_term = iso_pos.y / tile_size.y;
    Vec2::new(x_term + y_term, y_term - x_term)
}

/// Pit component marking a physical trading floor ring/station
#[derive(Component, Debug, Clone)]
pub struct Pit {
    pub instrument: Instrument,
    pub grid_pos: Vec2,
    pub center_screen: Vec2,
    pub width: f32,
    pub height: f32,
    pub is_selected: bool,
}

/// Bounding dimensions for collision and steering containment inside a pit
#[derive(Component, Debug, Clone, Copy)]
pub struct PitBounds {
    pub center: Vec2,
    pub width: f32,
    pub height: f32,
}

impl PitBounds {
    pub fn new(center: Vec2, width: f32, height: f32) -> Self {
        Self {
            center,
            width,
            height,
        }
    }

    pub fn contains(&self, point: Vec2) -> bool {
        let half_w = self.width * 0.5;
        let half_h = self.height * 0.5;
        point.x >= self.center.x - half_w
            && point.x <= self.center.x + half_w
            && point.y >= self.center.y - half_h
            && point.y <= self.center.y + half_h
    }

    pub fn clamp_point(&self, point: Vec2, margin: f32) -> Vec2 {
        let half_w = (self.width * 0.5 - margin).max(10.0);
        let half_h = (self.height * 0.5 - margin).max(10.0);
        Vec2::new(
            point.x.clamp(self.center.x - half_w, self.center.x + half_w),
            point.y.clamp(self.center.y - half_h, self.center.y + half_h),
        )
    }
}

/// Marker component for dynamic pit activity label text
#[derive(Component)]
pub struct PitActivityLabel {
    pub instrument: Instrument,
}

/// Marker component for pit floor border visual entity
#[derive(Component)]
pub struct PitBorderVisual {
    pub instrument: Instrument,
}

/// Marker component for dynamic reactive floorboard surface plate
#[derive(Component)]
pub struct PitFloorSurface {
    pub instrument: Instrument,
}

/// Marker component for the central raised Market Maker podium / pulpit
#[derive(Component)]
pub struct MarketMakerPodium {
    pub instrument: Instrument,
}

/// Marker component for the official price print terminal on the central pulpit
#[derive(Component)]
pub struct PodiumPriceDisplay {
    pub instrument: Instrument,
}

/// Expanding institutional liquidity wave ripple
#[derive(Component, Debug, Clone)]
pub struct InstitutionalWave {
    pub instrument: Instrument,
    pub current_radius: f32,
    pub max_radius: f32,
    pub speed: f32,
    pub base_color: Color,
}

/// Marker component for acoustic decibel equalizer bar above pit
#[derive(Component)]
pub struct DecibelBar {
    pub instrument: Instrument,
    pub bar_index: usize,
    pub base_pos: Vec2,
}

/// Blueprint definition for positioning pits on the logical floor grid
#[derive(Debug, Clone)]
pub struct PitLayoutDefinition {
    pub instrument: Instrument,
    pub grid_pos: Vec2,
    pub width: f32,
    pub height: f32,
}

impl PitLayoutDefinition {
    pub fn default_floor_plan() -> Vec<PitLayoutDefinition> {
        vec![
            // District 1 (North): Metals District (Grand Octagonal Amphitheaters)
            PitLayoutDefinition {
                instrument: Instrument::Gold,
                grid_pos: Vec2::new(-3.0, 15.0), // iso: (-1080, 360)
                width: 540.0,
                height: 270.0,
            },
            PitLayoutDefinition {
                instrument: Instrument::Silver,
                grid_pos: Vec2::new(15.0, -3.0), // iso: (1080, 360)
                width: 540.0,
                height: 270.0,
            },
            // District 2 (Center): Equities District
            PitLayoutDefinition {
                instrument: Instrument::Nasdaq,
                grid_pos: Vec2::new(-11.5, 6.5), // iso: (-1080, -150)
                width: 540.0,
                height: 270.0,
            },
            PitLayoutDefinition {
                instrument: Instrument::Sp500, // Central Grand Flagship Octagonal Arena
                grid_pos: Vec2::new(0.0, 0.0),  // iso: (0, 0)
                width: 580.0,
                height: 290.0,
            },
            PitLayoutDefinition {
                instrument: Instrument::Us30,
                grid_pos: Vec2::new(6.5, -11.5), // iso: (1080, -150)
                width: 540.0,
                height: 270.0,
            },
            // District 3 (South): FX & Crypto District (Generously spaced)
            PitLayoutDefinition {
                instrument: Instrument::Uk100,
                grid_pos: Vec2::new(-19.5, -1.5), // iso: (-1080, -630)
                width: 520.0,
                height: 260.0,
            },
            PitLayoutDefinition {
                instrument: Instrument::EurUsd,
                grid_pos: Vec2::new(-12.5, -5.5), // iso: (-420, -540)
                width: 520.0,
                height: 260.0,
            },
            PitLayoutDefinition {
                instrument: Instrument::Bitcoin,
                grid_pos: Vec2::new(-5.5, -12.5), // iso: (420, -540)
                width: 540.0,
                height: 270.0,
            },
            PitLayoutDefinition {
                instrument: Instrument::GbpUsd,
                grid_pos: Vec2::new(-1.5, -19.5), // iso: (1080, -630)
                width: 520.0,
                height: 260.0,
            },
        ]
    }
}

/// Pure helper: Computes the 8 vertices of a 2.5D isometric octagon given center and dimensions
pub fn compute_octagon_vertices(center: Vec2, width: f32, height: f32) -> [Vec2; 8] {
    let half_w = width * 0.5;
    let half_h = height * 0.5;
    let flat_w = half_w * 0.414;
    let flat_h = half_h * 0.414;

    [
        Vec2::new(center.x - flat_w, center.y + half_h), // 0: Top-Left (North-West corner)
        Vec2::new(center.x + flat_w, center.y + half_h), // 1: Top-Right (North-East corner)
        Vec2::new(center.x + half_w, center.y + flat_h), // 2: Right-Top (East-North corner)
        Vec2::new(center.x + half_w, center.y - flat_h), // 3: Right-Bottom (East-South corner)
        Vec2::new(center.x + flat_w, center.y - half_h), // 4: Bottom-Right (South-East corner)
        Vec2::new(center.x - flat_w, center.y - half_h), // 5: Bottom-Left (South-West corner)
        Vec2::new(center.x - half_w, center.y - flat_h), // 6: Left-Bottom (West-South corner)
        Vec2::new(center.x - half_w, center.y + flat_h), // 7: Left-Top (West-North corner)
    ]
}

/// Startup system: Spawns the continuous 53x53 tiled isometric exchange floor,
/// perimeter architectural walls, electronic LED tickers, concourses, and 9 grand octagonal amphitheater pits
pub fn spawn_pits(mut commands: Commands) {
    let tile_size = Vec2::new(TILE_WIDTH, TILE_HEIGHT);
    let floor_plan = PitLayoutDefinition::default_floor_plan();

    // -------------------------------------------------------------
    // 0. Grand Floor Foundation Slab & Dark Void Underlay
    // -------------------------------------------------------------
    commands.spawn((
        Sprite {
            color: Color::srgb(0.025, 0.035, 0.055),
            custom_size: Some(Vec2::new(7200.0, 4200.0)),
            ..default()
        },
        Transform::from_xyz(0.0, -20.0, -0.6),
    ));

    // -------------------------------------------------------------
    // 1. One Continuous 53x53 Isometric Tiled Floor Grid (SimCity / Theme Park Style)
    // -------------------------------------------------------------
    let diamond_size = Vec2::new(84.85, 84.85);
    let diamond_rot = Quat::from_rotation_z(std::f32::consts::FRAC_PI_4);
    let diamond_scale = Vec3::new(1.0, 0.5, 1.0);

    for gx in -26_i32..=26_i32 {
        for gy in -26_i32..=26_i32 {
            let pos = grid_to_isometric(Vec2::new(gx as f32, gy as f32), tile_size);

            // Grand concourse avenues connecting the trading pits
            let is_main_concourse = gx.abs() <= 1 || gy.abs() <= 1 || (gx + gy).abs() <= 1 || (gx - gy).abs() <= 1;

            let tile_color = if is_main_concourse {
                // Polished ceremonial walkway with warm amber teak
                Color::srgb(0.24, 0.17, 0.11)
            } else if (gx + gy) % 2 == 0 {
                // Warm parquet teak tile
                Color::srgb(0.18, 0.13, 0.09)
            } else {
                // Dark oak parquet tile
                Color::srgb(0.14, 0.10, 0.07)
            };

            commands.spawn((
                Sprite {
                    color: tile_color,
                    custom_size: Some(diamond_size),
                    ..default()
                },
                Transform {
                    translation: Vec3::new(pos.x, pos.y, -0.5),
                    rotation: diamond_rot,
                    scale: diamond_scale,
                },
            ));
        }
    }

    // -------------------------------------------------------------
    // 2. North-West & North-East Perimeter Architectural Walls & Electronic Tickers
    // -------------------------------------------------------------
    // North-West Wall (Isometric Back-Left Wall)
    for i in -26..=26 {
        let wall_pos = grid_to_isometric(Vec2::new(i as f32, 26.6), tile_size);
        // Mahogany wainscoting bottom half
        commands.spawn((
            Sprite {
                color: Color::srgb(0.11, 0.07, 0.04),
                custom_size: Some(Vec2::new(65.0, 36.0)),
                ..default()
            },
            Transform::from_xyz(wall_pos.x, wall_pos.y + 18.0, -0.3),
        ));
        // Classic stone upper wall
        commands.spawn((
            Sprite {
                color: Color::srgb(0.16, 0.19, 0.25),
                custom_size: Some(Vec2::new(65.0, 56.0)),
                ..default()
            },
            Transform::from_xyz(wall_pos.x, wall_pos.y + 64.0, -0.3),
        ));
        // Polished brass chair-rail moulding
        commands.spawn((
            Sprite {
                color: Color::srgb(0.85, 0.70, 0.28),
                custom_size: Some(Vec2::new(65.0, 3.0)),
                ..default()
            },
            Transform::from_xyz(wall_pos.x, wall_pos.y + 36.0, -0.28),
        ));
        // Architectural arched window every 3 panels
        if i % 3 == 0 {
            commands.spawn((
                Sprite {
                    color: Color::srgb(0.24, 0.36, 0.52),
                    custom_size: Some(Vec2::new(26.0, 36.0)),
                    ..default()
                },
                Transform::from_xyz(wall_pos.x, wall_pos.y + 68.0, -0.27),
            ));
        }
    }

    // North-East Wall (Isometric Back-Right Wall)
    for i in -26..=26 {
        let wall_pos = grid_to_isometric(Vec2::new(26.6, i as f32), tile_size);
        commands.spawn((
            Sprite {
                color: Color::srgb(0.09, 0.06, 0.03),
                custom_size: Some(Vec2::new(65.0, 36.0)),
                ..default()
            },
            Transform::from_xyz(wall_pos.x, wall_pos.y + 18.0, -0.3),
        ));
        commands.spawn((
            Sprite {
                color: Color::srgb(0.13, 0.16, 0.22),
                custom_size: Some(Vec2::new(65.0, 56.0)),
                ..default()
            },
            Transform::from_xyz(wall_pos.x, wall_pos.y + 64.0, -0.3),
        ));
        commands.spawn((
            Sprite {
                color: Color::srgb(0.78, 0.64, 0.25),
                custom_size: Some(Vec2::new(65.0, 3.0)),
                ..default()
            },
            Transform::from_xyz(wall_pos.x, wall_pos.y + 36.0, -0.28),
        ));
        if i % 3 == 0 {
            commands.spawn((
                Sprite {
                    color: Color::srgb(0.20, 0.32, 0.46),
                    custom_size: Some(Vec2::new(26.0, 36.0)),
                    ..default()
                },
                Transform::from_xyz(wall_pos.x, wall_pos.y + 68.0, -0.27),
            ));
        }
    }

    // Wall-Mounted Electronic LED Price Ticker Boards
    let ticker_left = grid_to_isometric(Vec2::new(0.0, 26.6), tile_size);
    commands.spawn((
        Text2d::new("=== CME / CBOT GRAND OPEN-CRY FLOOR // REALTIME ORDERFLOW ENGINE ==="),
        TextFont {
            font_size: 18.0,
            ..default()
        },
        TextColor(Color::srgb(0.20, 1.0, 0.40)),
        Transform::from_xyz(ticker_left.x - 260.0, ticker_left.y + 90.0, 2.0),
    ));

    let ticker_right = grid_to_isometric(Vec2::new(26.6, 0.0), tile_size);
    commands.spawn((
        Text2d::new("GOLD 2,738.40 + | SILVER 31.85 + | NQ 20,450 + | ES 5,860.25 + | YM 52,600 - | BTC 68,400 +"),
        TextFont {
            font_size: 17.0,
            ..default()
        },
        TextColor(Color::srgb(1.0, 0.85, 0.25)),
        Transform::from_xyz(ticker_right.x + 220.0, ticker_right.y + 90.0, 2.0),
    ));

    // South-West and South-East Foundation Drop Edges with Brass Stanchions
    for i in -26..=26 {
        let edge_sw = grid_to_isometric(Vec2::new(i as f32, -26.6), tile_size);
        commands.spawn((
            Sprite {
                color: Color::srgb(0.08, 0.09, 0.12),
                custom_size: Some(Vec2::new(65.0, 26.0)),
                ..default()
            },
            Transform::from_xyz(edge_sw.x, edge_sw.y - 13.0, -0.45),
        ));
        commands.spawn((
            Sprite {
                color: Color::srgb(0.85, 0.72, 0.28),
                custom_size: Some(Vec2::new(3.5, 15.0)),
                ..default()
            },
            Transform::from_xyz(edge_sw.x, edge_sw.y + 5.0, 0.05),
        ));

        let edge_se = grid_to_isometric(Vec2::new(-26.6, i as f32), tile_size);
        commands.spawn((
            Sprite {
                color: Color::srgb(0.06, 0.07, 0.10),
                custom_size: Some(Vec2::new(65.0, 26.0)),
                ..default()
            },
            Transform::from_xyz(edge_se.x, edge_se.y - 13.0, -0.45),
        ));
        commands.spawn((
            Sprite {
                color: Color::srgb(0.85, 0.72, 0.28),
                custom_size: Some(Vec2::new(3.5, 15.0)),
                ..default()
            },
            Transform::from_xyz(edge_se.x, edge_se.y + 5.0, 0.05),
        ));
    }

    // -------------------------------------------------------------
    // 3. Grand Sunken Octagonal Amphitheater Pits Carved Directly into the Shared Floor
    // -------------------------------------------------------------
    for def in floor_plan {
        let screen_pos = grid_to_isometric(def.grid_pos, tile_size);
        let accent = def.instrument.accent_color();

        let _pit_entity = commands
            .spawn((
                Pit {
                    instrument: def.instrument,
                    grid_pos: def.grid_pos,
                    center_screen: screen_pos,
                    width: def.width,
                    height: def.height,
                    is_selected: false,
                },
                PitBounds::new(screen_pos, def.width, def.height),
                Transform::from_xyz(screen_pos.x, screen_pos.y, 0.0),
                Visibility::default(),
            ))
            .id();

        // Calculate authentic 8 vertices for each concentric octagonal amphitheater tier
        let verts_outer = compute_octagon_vertices(screen_pos, def.width, def.height);
        let verts_t1 = compute_octagon_vertices(screen_pos, def.width * 0.84, def.height * 0.84);
        let verts_t2 = compute_octagon_vertices(screen_pos, def.width * 0.68, def.height * 0.68);
        let core_w = def.width * 0.50;
        let core_h = def.height * 0.50;

        // ---------------------------------------------------------
        // A. Outer Octagonal Parapet & Brass Upper Gallery Rail (8 sides)
        // ---------------------------------------------------------
        for i in 0..8 {
            let v_a = verts_outer[i];
            let v_b = verts_outer[(i + 1) % 8];
            let mid = (v_a + v_b) * 0.5;
            let dir = v_b - v_a;
            let len = dir.length();
            let rot = Quat::from_rotation_z(dir.y.atan2(dir.x));

            // Outer mahogany parapet wall
            commands.spawn((
                Sprite {
                    color: Color::srgb(0.18, 0.11, 0.07),
                    custom_size: Some(Vec2::new(len + 12.0, 16.0)),
                    ..default()
                },
                Transform {
                    translation: Vec3::new(mid.x, mid.y, 0.01),
                    rotation: rot,
                    ..default()
                },
            ));

            // Polished brass upper gallery rail
            commands.spawn((
                Sprite {
                    color: Color::srgb(0.85, 0.72, 0.28),
                    custom_size: Some(Vec2::new(len + 6.0, 4.0)),
                    ..default()
                },
                Transform {
                    translation: Vec3::new(mid.x, mid.y, 0.02),
                    rotation: rot,
                    ..default()
                },
            ));

            // Dynamic border visual highlight (illuminates white on selection)
            commands.spawn((
                PitBorderVisual {
                    instrument: def.instrument,
                },
                Sprite {
                    color: accent.with_alpha(0.55),
                    custom_size: Some(Vec2::new(len + 8.0, 5.0)),
                    ..default()
                },
                Transform {
                    translation: Vec3::new(mid.x, mid.y, 0.025),
                    rotation: rot,
                    ..default()
                },
            ));
        }

        // 8 Polished Brass Corner Posts at the 8 Octagonal Vertices
        for &v in &verts_outer {
            commands.spawn((
                Sprite {
                    color: Color::srgba(0.0, 0.0, 0.0, 0.45),
                    custom_size: Some(Vec2::new(10.0, 6.0)),
                    ..default()
                },
                Transform::from_xyz(v.x + 1.5, v.y - 2.5, 0.028),
            ));
            commands.spawn((
                Sprite {
                    color: Color::srgb(0.95, 0.85, 0.35),
                    custom_size: Some(Vec2::new(9.0, 9.0)),
                    ..default()
                },
                Transform::from_xyz(v.x, v.y, 0.03),
            ));
        }

        // ---------------------------------------------------------
        // B. Tier 1: Outer Sunken Step Floorplate & 8 Riser Facets
        // ---------------------------------------------------------
        // Seamless solid floorplate filling the outer tier
        commands.spawn((
            Sprite {
                color: Color::srgb(0.12, 0.14, 0.19),
                custom_size: Some(Vec2::new(def.width * 0.94, def.height * 0.38)),
                ..default()
            },
            Transform::from_xyz(screen_pos.x, screen_pos.y, 0.035),
        ));
        commands.spawn((
            Sprite {
                color: Color::srgb(0.12, 0.14, 0.19),
                custom_size: Some(Vec2::new(def.width * 0.38, def.height * 0.94)),
                ..default()
            },
            Transform::from_xyz(screen_pos.x, screen_pos.y, 0.035),
        ));
        commands.spawn((
            Sprite {
                color: Color::srgb(0.12, 0.14, 0.19),
                custom_size: Some(Vec2::new(def.width * 0.66, def.height * 0.66)),
                ..default()
            },
            Transform {
                translation: Vec3::new(screen_pos.x, screen_pos.y, 0.035),
                rotation: diamond_rot,
                scale: diamond_scale,
            },
        ));

        // 8 Step Rim Nosings & 3D Drop Walls for Tier 1
        for i in 0..8 {
            let v_a = verts_t1[i];
            let v_b = verts_t1[(i + 1) % 8];
            let mid = (v_a + v_b) * 0.5;
            let dir = v_b - v_a;
            let len = dir.length();
            let rot = Quat::from_rotation_z(dir.y.atan2(dir.x));

            // 3D vertical drop wall on South/East facing facets (edges 2, 3, 4, 5)
            if i >= 2 && i <= 5 {
                commands.spawn((
                    Sprite {
                        color: Color::srgb(0.08, 0.05, 0.04),
                        custom_size: Some(Vec2::new(len + 6.0, 10.0)),
                        ..default()
                    },
                    Transform {
                        translation: Vec3::new(mid.x, mid.y - 3.5, 0.038),
                        rotation: rot,
                        ..default()
                    },
                ));
            }

            // Polished brass step nosing trim
            commands.spawn((
                Sprite {
                    color: Color::srgb(0.85, 0.72, 0.28),
                    custom_size: Some(Vec2::new(len + 4.0, 3.0)),
                    ..default()
                },
                Transform {
                    translation: Vec3::new(mid.x, mid.y, 0.045),
                    rotation: rot,
                    ..default()
                },
            ));
        }

        // ---------------------------------------------------------
        // C. Tier 2: Middle Sunken Step Floorplate & 8 Riser Facets
        // ---------------------------------------------------------
        // Seamless solid floorplate filling the middle tier
        commands.spawn((
            Sprite {
                color: Color::srgb(0.09, 0.11, 0.15),
                custom_size: Some(Vec2::new(def.width * 0.76, def.height * 0.31)),
                ..default()
            },
            Transform::from_xyz(screen_pos.x, screen_pos.y, 0.05),
        ));
        commands.spawn((
            Sprite {
                color: Color::srgb(0.09, 0.11, 0.15),
                custom_size: Some(Vec2::new(def.width * 0.31, def.height * 0.76)),
                ..default()
            },
            Transform::from_xyz(screen_pos.x, screen_pos.y, 0.05),
        ));
        commands.spawn((
            Sprite {
                color: Color::srgb(0.09, 0.11, 0.15),
                custom_size: Some(Vec2::new(def.width * 0.54, def.height * 0.54)),
                ..default()
            },
            Transform {
                translation: Vec3::new(screen_pos.x, screen_pos.y, 0.05),
                rotation: diamond_rot,
                scale: diamond_scale,
            },
        ));

        // 8 Step Rim Nosings & 3D Drop Walls for Tier 2
        for i in 0..8 {
            let v_a = verts_t2[i];
            let v_b = verts_t2[(i + 1) % 8];
            let mid = (v_a + v_b) * 0.5;
            let dir = v_b - v_a;
            let len = dir.length();
            let rot = Quat::from_rotation_z(dir.y.atan2(dir.x));

            if i >= 2 && i <= 5 {
                commands.spawn((
                    Sprite {
                        color: Color::srgb(0.05, 0.06, 0.09),
                        custom_size: Some(Vec2::new(len + 6.0, 9.0)),
                        ..default()
                    },
                    Transform {
                        translation: Vec3::new(mid.x, mid.y - 3.0, 0.052),
                        rotation: rot,
                        ..default()
                    },
                ));
            }

            commands.spawn((
                Sprite {
                    color: Color::srgb(0.80, 0.68, 0.25),
                    custom_size: Some(Vec2::new(len + 4.0, 2.5)),
                    ..default()
                },
                Transform {
                    translation: Vec3::new(mid.x, mid.y, 0.058),
                    rotation: rot,
                    ..default()
                },
            ));
        }

        // ---------------------------------------------------------
        // D. Sunken Pit Arena Core Well & Dynamic Reactive Floorboards
        // ---------------------------------------------------------
        // Octagonal floorboards plate (glows dynamically on volume & delta surges)
        commands.spawn((
            PitFloorSurface {
                instrument: def.instrument,
            },
            Sprite {
                color: Color::srgba(0.06, 0.08, 0.12, 0.94),
                custom_size: Some(Vec2::new(core_w, core_h * 0.414)),
                ..default()
            },
            Transform::from_xyz(screen_pos.x, screen_pos.y, 0.075),
        ));
        commands.spawn((
            PitFloorSurface {
                instrument: def.instrument,
            },
            Sprite {
                color: Color::srgba(0.06, 0.08, 0.12, 0.94),
                custom_size: Some(Vec2::new(core_w * 0.414, core_h)),
                ..default()
            },
            Transform::from_xyz(screen_pos.x, screen_pos.y, 0.075),
        ));
        commands.spawn((
            PitFloorSurface {
                instrument: def.instrument,
            },
            Sprite {
                color: Color::srgba(0.06, 0.08, 0.12, 0.94),
                custom_size: Some(Vec2::new(core_w * 0.707, core_h * 0.707)),
                ..default()
            },
            Transform {
                translation: Vec3::new(screen_pos.x, screen_pos.y, 0.075),
                rotation: diamond_rot,
                scale: diamond_scale,
            },
        ));

        // Deep North cast shadow on floorboards
        commands.spawn((
            Sprite {
                color: Color::srgba(0.01, 0.01, 0.02, 0.65),
                custom_size: Some(Vec2::new(core_w * 0.90, 14.0)),
                ..default()
            },
            Transform::from_xyz(screen_pos.x, screen_pos.y + (core_h * 0.5) - 12.0, 0.08),
        ));

        // ---------------------------------------------------------
        // E. Raised 3D Octagonal Market Maker Podium / Pulpit
        // ---------------------------------------------------------
        let podium_w = 72.0;
        let podium_h = 36.0;

        // Base cast drop shadow
        commands.spawn((
            Sprite {
                color: Color::srgba(0.0, 0.0, 0.0, 0.50),
                custom_size: Some(Vec2::new(podium_w + 12.0, podium_h * 0.65)),
                ..default()
            },
            Transform::from_xyz(screen_pos.x, screen_pos.y - 12.0, 0.085),
        ));

        // Front 3D vertical mahogany riser face
        commands.spawn((
            Sprite {
                color: Color::srgb(0.18, 0.10, 0.06),
                custom_size: Some(Vec2::new(podium_w * 0.90, 14.0)),
                ..default()
            },
            Transform::from_xyz(screen_pos.x, screen_pos.y - 4.5, 0.09),
        ));

        // Brass podium kickplate trim
        commands.spawn((
            Sprite {
                color: Color::srgb(0.78, 0.62, 0.22),
                custom_size: Some(Vec2::new(podium_w * 0.92, 3.0)),
                ..default()
            },
            Transform::from_xyz(screen_pos.x, screen_pos.y - 11.5, 0.095),
        ));

        // Upper standing stage deck
        commands.spawn((
            Sprite {
                color: Color::srgb(0.28, 0.17, 0.10),
                custom_size: Some(Vec2::new(podium_w, podium_h)),
                ..default()
            },
            Transform::from_xyz(screen_pos.x, screen_pos.y + 3.5, 0.10),
        ));

        // Polished brass podium handrail rim
        commands.spawn((
            Sprite {
                color: Color::srgb(0.92, 0.78, 0.28),
                custom_size: Some(Vec2::new(podium_w - 6.0, podium_h - 4.0)),
                ..default()
            },
            Transform::from_xyz(screen_pos.x, screen_pos.y + 3.5, 0.11),
        ));

        // Specialist center deck pad
        commands.spawn((
            MarketMakerPodium {
                instrument: def.instrument,
            },
            Sprite {
                color: Color::srgb(0.13, 0.09, 0.07),
                custom_size: Some(Vec2::new(podium_w - 18.0, podium_h - 14.0)),
                ..default()
            },
            Transform::from_xyz(screen_pos.x, screen_pos.y + 3.5, 0.12),
        ));

        // ---------------------------------------------------------
        // F. Elevated Overhead Jumbotron Scoreboard Banner (PRICE BOX IS ABOVE!)
        // ---------------------------------------------------------
        // Positioned safely above the pit amphitheater so it NEVER obscures the trading floor
        let header_y = screen_pos.y + def.height * 0.5 + 48.0;

        // High-contrast acrylic backdrop pill for scoreboard
        commands.spawn((
            Sprite {
                color: Color::srgba(0.02, 0.03, 0.06, 0.96),
                custom_size: Some(Vec2::new(380.0, 72.0)),
                ..default()
            },
            Transform::from_xyz(screen_pos.x, header_y, 4.8),
        ));
        // Polished brass frame trim
        commands.spawn((
            Sprite {
                color: Color::srgb(0.85, 0.72, 0.28),
                custom_size: Some(Vec2::new(384.0, 76.0)),
                ..default()
            },
            Transform::from_xyz(screen_pos.x, header_y, 4.75),
        ));
        // Inner accent colored pinstripe
        commands.spawn((
            Sprite {
                color: accent.with_alpha(0.85),
                custom_size: Some(Vec2::new(378.0, 2.5)),
                ..default()
            },
            Transform::from_xyz(screen_pos.x, header_y + 33.0, 4.85),
        ));

        // Row 1: Large bold instrument title
        commands.spawn((
            Text2d::new(format!("{} / {}", def.instrument.name(), def.instrument.symbol())),
            TextFont {
                font_size: 17.5,
                ..default()
            },
            TextColor(accent),
            Transform::from_xyz(screen_pos.x, header_y + 18.0, 5.0),
        ));

        // Row 2: THE PRICE BOX (Prominent official real-time print readout above the pit!)
        commands.spawn((
            PodiumPriceDisplay {
                instrument: def.instrument,
            },
            Text2d::new(format_price_with_commas(def.instrument.base_price() as f64, instrument_decimals(def.instrument))),
            TextFont {
                font_size: 22.0,
                ..default()
            },
            TextColor(Color::srgb(1.0, 0.88, 0.25)),
            Transform::from_xyz(screen_pos.x, header_y - 2.0, 5.0),
        ));

        // Row 3: Highly legible Activity & Delta Status Label
        commands.spawn((
            PitActivityLabel {
                instrument: def.instrument,
            },
            Text2d::new(format!(
                "{} | ACT: {:.0}%",
                def.instrument.district().label(),
                def.instrument.default_activity() * 100.0
            )),
            TextFont {
                font_size: 11.5,
                ..default()
            },
            TextColor(Color::srgb(0.75, 0.82, 0.90)),
            Transform::from_xyz(screen_pos.x, header_y - 22.0, 5.0),
        ));

        // Acoustic Decibel Equalizer Bars (4 mini bars above the scoreboard)
        let eq_y = header_y + 44.0;
        let bar_spacing = 7.5;
        let start_x = screen_pos.x - (bar_spacing * 1.5);

        for bar_idx in 0..4 {
            let bar_x = start_x + (bar_idx as f32 * bar_spacing);
            commands.spawn((
                DecibelBar {
                    instrument: def.instrument,
                    bar_index: bar_idx,
                    base_pos: Vec2::new(bar_x, eq_y),
                },
                Sprite {
                    color: Color::srgb(0.20, 0.85, 0.40),
                    custom_size: Some(Vec2::new(4.5, 6.0)),
                    ..default()
                },
                Transform::from_xyz(bar_x, eq_y, 5.0),
            ));
        }
    }
}

/// Write a Text2d only when the content changes, so the layout pass is not redone every frame.
pub fn set_text2d(text: &mut Text2d, value: String) {
    if **text != value {
        **text = value;
    }
}

/// Write a UI Text only when the content changes.
pub fn set_ui_text(text: &mut Text, value: String) {
    if **text != value {
        **text = value;
    }
}

/// Pure helper: one-line reading of a pit from its signals. What a floor trader would say
/// if you asked "what's it doing?" and they had three seconds.
pub fn pit_state_label(sig: &PitSignals) -> &'static str {
    if sig.fast_market && sig.vacuum > 0.3 {
        "FAST / NO BIDS"
    } else if sig.fast_market {
        "FAST MARKET"
    } else if sig.pressure > 0.5 && sig.imbalance > 0.25 {
        "BUYERS SWEEPING"
    } else if sig.pressure < -0.5 && sig.imbalance < -0.25 {
        "SELLERS HITTING"
    } else if sig.pressure.abs() > 0.5 {
        "ONE-WAY PAPER, THIN"
    } else if sig.sound > 0.6 {
        "LOUD TWO-WAY"
    } else if sig.sound > 0.3 {
        "STEADY AUCTION"
    } else {
        "QUIET"
    }
}

/// System: pit label shows the engine's read of the pit, not a scripted mood
pub fn update_pit_labels(
    view_mode: Res<FloorViewMode>,
    engines: Res<PitEngines>,
    mut query: Query<(&PitActivityLabel, &mut Text2d, &mut TextColor)>,
) {
    if view_mode.is_3d() {
        return;
    }
    for (label, mut text, mut color) in query.iter_mut() {
        let pit = engines.pit(label.instrument);
        let sig = &pit.signals;
        let state = pit_state_label(sig);
        let sound_pct = (sig.sound * 100.0).round() as u32;
        let imb_pct = (sig.imbalance * 100.0).round() as i32;

        set_text2d(
            &mut text,
            format!(
                "{} | SOUND {}% | FLOW {:+}% | {}/{} IN [{}]",
                label.instrument.district().label(),
                sound_pct,
                imb_pct,
                pit.active_count,
                crate::pit_engine::SLOTS,
                state
            ),
        );

        let new_color = if sig.fast_market {
            Color::srgb(1.0, 0.85, 0.30)
        } else if sig.imbalance > 0.25 {
            Color::srgb(0.25, 1.0, 0.45)
        } else if sig.imbalance < -0.25 {
            Color::srgb(1.0, 0.35, 0.35)
        } else {
            Color::srgb(0.65, 0.72, 0.80)
        };
        if color.0 != new_color {
            *color = TextColor(new_color);
        }
    }
}

/// System: floorboard glow. Hue is realised flow (who is getting filled), brightness is
/// unfilled pressure (what the rail is trying to do). A bright glow with a neutral hue is a vacuum.
pub fn update_pit_floor_glow(
    view_mode: Res<FloorViewMode>,
    engines: Res<PitEngines>,
    mut query: Query<(&PitFloorSurface, &mut Sprite)>,
) {
    if view_mode.is_3d() {
        return;
    }
    for (surface, mut sprite) in query.iter_mut() {
        let sig = &engines.pit(surface.instrument).signals;
        let intensity = (0.35 + sig.pressure.abs() * 0.65).clamp(0.0, 1.0);
        let imb = sig.imbalance;
        sprite.color = if imb > 0.15 {
            let g = (0.10 + imb * 0.22 * intensity).clamp(0.10, 0.32);
            Color::srgba(0.04, g, 0.08, 0.92)
        } else if imb < -0.15 {
            let r = (0.12 + (-imb) * 0.24 * intensity).clamp(0.12, 0.34);
            Color::srgba(r, 0.04, 0.06, 0.92)
        } else {
            // Neutral hue; pressure alone brightens toward amber: paper is working, nobody is filling
            let a = sig.pressure.abs();
            Color::srgba(0.06 + a * 0.14, 0.08 + a * 0.08, 0.12 - a * 0.04, 0.92)
        };
    }
}

/// System: Updates acoustic decibel visual equalizer bars above each pit
pub fn update_decibel_equalizers(
    time: Res<Time>,
    view_mode: Res<FloorViewMode>,
    engines: Res<PitEngines>,
    mut query: Query<(&DecibelBar, &mut Sprite, &mut Transform)>,
) {
    if view_mode.is_3d() {
        return;
    }
    let t = time.elapsed_secs();

    for (bar, mut sprite, mut transform) in query.iter_mut() {
        // The bars are the pit's sound level, which the engine derives from who is shouting.
        let effective = engines.pit(bar.instrument).signals.sound.clamp(0.03, 1.0);

        let phase = t * 14.0 + (bar.bar_index as f32 * 1.57);
        let osc = (phase.sin() * 0.25 + 0.75).abs();
        let target_height = (effective * 18.0 * osc).clamp(2.0, 18.0);

        sprite.custom_size = Some(Vec2::new(3.2, target_height));
        transform.translation.y = bar.base_pos.y + (target_height * 0.5);

        // Equalizer color shifts green -> yellow -> red
        if target_height > 12.0 {
            sprite.color = Color::srgb(1.0, 0.25, 0.30); // Roar (Red)
        } else if target_height > 7.0 {
            sprite.color = Color::srgb(1.0, 0.85, 0.20); // Active (Yellow)
        } else {
            sprite.color = Color::srgb(0.25, 0.90, 0.45); // Calm (Green)
        }
    }
}

/// System updating pit border visuals when selection state changes
pub fn update_pit_border_visuals(
    pits: Query<&Pit>,
    mut borders: Query<(&PitBorderVisual, &mut Sprite)>,
) {
    for (border, mut sprite) in borders.iter_mut() {
        if let Some(pit) = pits.iter().find(|p| p.instrument == border.instrument) {
            if pit.is_selected {
                sprite.color = Color::srgb(0.95, 0.95, 1.0);
            } else {
                sprite.color = border.instrument.accent_color().with_alpha(0.55);
            }
        }
    }
}

/// System: Animates expanding institutional money waves across the floorboards
pub fn update_institutional_waves(
    mut commands: Commands,
    time: Res<Time>,
    mut query: Query<(Entity, &mut InstitutionalWave, &mut Sprite)>,
) {
    let dt = time.delta_secs();
    for (entity, mut wave, mut sprite) in query.iter_mut() {
        wave.current_radius += wave.speed * dt;
        let progress = (wave.current_radius / wave.max_radius).clamp(0.0, 1.0);
        if progress >= 1.0 {
            commands.entity(entity).despawn();
        } else {
            let alpha = (1.0 - progress) * 0.75;
            sprite.color = wave.base_color.with_alpha(alpha);
            sprite.custom_size = Some(Vec2::new(wave.current_radius * 2.0, wave.current_radius * 1.1));
        }
    }
}

/// Pure helper: Decimals precision per instrument for authentic ticker display
/// Decimal places shown for each instrument: the cTrader chart's own digits, so a price on
/// the floor reads the same as the one on the screen beside it.
pub fn instrument_decimals(inst: Instrument) -> usize {
    match inst {
        Instrument::EurUsd | Instrument::GbpUsd => 5,
        Instrument::Silver => 3,
        Instrument::Gold | Instrument::Bitcoin => 2,
        Instrument::Nasdaq | Instrument::Sp500 | Instrument::Us30 | Instrument::Uk100 => 1,
    }
}

/// Pure helper: Formats a number with thousands separators and exact decimal precision
pub fn format_price_with_commas(val: f64, decimals: usize) -> String {
    let formatted = format!("{:.prec$}", val, prec = decimals);
    let parts: Vec<&str> = formatted.split('.').collect();
    let int_part = parts[0];
    let mut with_commas = String::with_capacity(int_part.len() + 4);
    let len = int_part.len();
    for (i, ch) in int_part.chars().enumerate() {
        with_commas.push(ch);
        let remaining = len - 1 - i;
        if remaining > 0 && remaining % 3 == 0 {
            with_commas.push(',');
        }
    }
    if parts.len() > 1 {
        format!("{}.{}", with_commas, parts[1])
    } else {
        with_commas
    }
}

/// System: Updates the official price print readout on each central pit pulpit
pub fn update_podium_price_displays(
    view_mode: Res<FloorViewMode>,
    engines: Res<PitEngines>,
    mut query: Query<(&PodiumPriceDisplay, &mut Text2d, &mut TextColor)>,
) {
    if view_mode.is_3d() {
        return;
    }
    for (display, mut text, mut color) in query.iter_mut() {
        let pit = engines.pit(display.instrument);
        let last_up = pit.events.iter().rev().find_map(|e| match e.kind {
            crate::pit_engine::PitEventKind::Print { up, .. } => Some(up),
            _ => None,
        });
        let arrow = match last_up {
            Some(true) => "+",
            Some(false) => "-",
            None => "=",
        };

        let formatted = format_price_with_commas(pit.last_price() as f64, instrument_decimals(display.instrument));
        set_text2d(&mut text, format!("{} {}", formatted, arrow));

        let new_color = match last_up {
            Some(true) => Color::srgb(0.20, 1.0, 0.40),
            Some(false) => Color::srgb(1.0, 0.28, 0.28),
            None => Color::srgb(1.0, 0.85, 0.25),
        };
        if color.0 != new_color {
            *color = TextColor(new_color);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_isometric_projection_math() {
        let tile_size = Vec2::new(360.0, 180.0);
        let grid = Vec2::new(2.0, 1.0);
        let iso = grid_to_isometric(grid, tile_size);
        let back = isometric_to_grid(iso, tile_size);

        assert!((back.x - grid.x).abs() < 0.001);
        assert!((back.y - grid.y).abs() < 0.001);
    }

    #[test]
    fn test_default_layout_covers_all_instruments() {
        let layout = PitLayoutDefinition::default_floor_plan();
        assert_eq!(layout.len(), 9);
        for inst in Instrument::ALL.iter() {
            assert!(layout.iter().any(|d| d.instrument == *inst));
        }
    }

    #[test]
    fn test_pit_bounds_containment() {
        let bounds = PitBounds::new(Vec2::new(100.0, 50.0), 40.0, 20.0);
        assert!(bounds.contains(Vec2::new(100.0, 50.0)));
        assert!(bounds.contains(Vec2::new(115.0, 58.0)));
        assert!(!bounds.contains(Vec2::new(130.0, 50.0)));
        assert!(!bounds.contains(Vec2::new(100.0, 65.0)));
    }

    #[test]
    fn test_format_price_with_commas() {
        assert_eq!(format_price_with_commas(2738.40_f64, 2), "2,738.40");
        assert_eq!(format_price_with_commas(68400.0_f64, 1), "68,400.0");
        assert_eq!(format_price_with_commas(1.0865_f64, 4), "1.0865");
        assert_eq!(format_price_with_commas(1234567.89_f64, 2), "1,234,567.89");
    }

    #[test]
    fn test_instrument_decimals() {
        // cTrader digits: 5 on majors, 3 on silver, 2 on gold and bitcoin, 1 on indices.
        assert_eq!(instrument_decimals(Instrument::EurUsd), 5);
        assert_eq!(instrument_decimals(Instrument::Silver), 3);
        assert_eq!(instrument_decimals(Instrument::Bitcoin), 2);
        assert_eq!(instrument_decimals(Instrument::Gold), 2);
        assert_eq!(instrument_decimals(Instrument::Us30), 1);
    }

    #[test]
    fn test_compute_octagon_vertices() {
        let center = Vec2::new(100.0, 200.0);
        let width = 400.0;
        let height = 200.0;
        let verts = compute_octagon_vertices(center, width, height);

        assert_eq!(verts.len(), 8);

        // Top horizontal segment (NW to NE)
        assert!((verts[0].y - 300.0).abs() < 1e-4);
        assert!((verts[1].y - 300.0).abs() < 1e-4);
        assert!(verts[0].x < center.x);
        assert!(verts[1].x > center.x);

        // Right vertical segment (EN to ES)
        assert!((verts[2].x - 300.0).abs() < 1e-4);
        assert!((verts[3].x - 300.0).abs() < 1e-4);

        // Bottom horizontal segment (SE to SW)
        assert!((verts[4].y - 100.0).abs() < 1e-4);
        assert!((verts[5].y - 100.0).abs() < 1e-4);

        // Left vertical segment (WS to WN)
        assert!((verts[6].x - (-100.0)).abs() < 1e-4);
        assert!((verts[7].x - (-100.0)).abs() < 1e-4);

        // Symmetry around center
        let half_w = width * 0.5;
        let flat_w = half_w * 0.414;
        assert!((verts[0].x - (100.0 - flat_w)).abs() < 1e-4);
        assert!((verts[1].x - (100.0 + flat_w)).abs() < 1e-4);
    }
}
