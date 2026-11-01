use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageSampler, ImageSamplerDescriptor};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

/// Demographic racial representation (80% White, 10% Black, 10% Brown)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ethnicity {
    White,
    Black,
    Brown,
}

impl Ethnicity {
    pub fn label(&self) -> &'static str {
        match self {
            Ethnicity::White => "White (80%)",
            Ethnicity::Black => "Black (10%)",
            Ethnicity::Brown => "Brown (10%)",
        }
    }
}

/// CME Member Firm Trading Jackets: Iconic Vertical Striped & Solid Blazers
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum JacketStyle {
    // 6 Iconic Vertical-Striped CME Member Firm Coats
    CargillRed,        // Candy Stripe: Cargill Red / White
    DeanWitterBlue,    // Dean Witter: Royal Navy / White
    CarrGreen,         // Carr Futures: Kelly Green / White
    FirstOptionsGold,  // First Options of Chicago: Vivid Gold / Royal Blue
    RefcoCrimson,      // Refco: Crimson Red / Charcoal Black
    SunOrange,         // Sun Trading: Safety Orange / Dark Navy

    // Official Floor Role Solid Blazers
    BrokerBlue,        // Lead Floor Broker: Royal Blue
    ScalperGreen,      // Local Scalper: Kelly Green
    RunnerGold,        // Concourse Runner: Vivid Gold
    ReporterBurgundy,  // Price Reporter: Exchange Burgundy
}

impl JacketStyle {
    pub const ALL_STRIPED: [JacketStyle; 6] = [
        JacketStyle::CargillRed,
        JacketStyle::DeanWitterBlue,
        JacketStyle::CarrGreen,
        JacketStyle::FirstOptionsGold,
        JacketStyle::RefcoCrimson,
        JacketStyle::SunOrange,
    ];

    pub fn is_striped(&self) -> bool {
        matches!(
            self,
            JacketStyle::CargillRed
                | JacketStyle::DeanWitterBlue
                | JacketStyle::CarrGreen
                | JacketStyle::FirstOptionsGold
                | JacketStyle::RefcoCrimson
                | JacketStyle::SunOrange
        )
    }

    pub fn colors(&self) -> (Color, Color, u32) {
        match self {
            JacketStyle::CargillRed => (
                Color::srgb(0.86, 0.15, 0.15), // #dc2626 Cargill Red
                Color::srgb(0.97, 0.98, 0.99), // #f8fafc Pure White
                8,
            ),
            JacketStyle::DeanWitterBlue => (
                Color::srgb(0.11, 0.31, 0.85), // #1d4ed8 Royal Navy
                Color::srgb(0.97, 0.98, 0.99), // #f8fafc Pure White
                8,
            ),
            JacketStyle::CarrGreen => (
                Color::srgb(0.08, 0.50, 0.24), // #15803d Kelly Green
                Color::srgb(0.97, 0.98, 0.99), // #f8fafc Pure White
                8,
            ),
            JacketStyle::FirstOptionsGold => (
                Color::srgb(0.92, 0.70, 0.03), // #eab308 Vivid Gold
                Color::srgb(0.12, 0.23, 0.54), // #1e3a8a Royal Navy
                8,
            ),
            JacketStyle::RefcoCrimson => (
                Color::srgb(0.60, 0.11, 0.11), // #991b1b Crimson Red
                Color::srgb(0.07, 0.09, 0.15), // #111827 Charcoal Black
                8,
            ),
            JacketStyle::SunOrange => (
                Color::srgb(0.92, 0.35, 0.05), // #ea580c Safety Orange
                Color::srgb(0.06, 0.09, 0.16), // #0f172a Dark Navy
                8,
            ),
            JacketStyle::BrokerBlue => (
                Color::srgb(0.06, 0.23, 0.61), // #103b9b
                Color::srgb(0.06, 0.23, 0.61),
                1,
            ),
            JacketStyle::ScalperGreen => (
                Color::srgb(0.08, 0.50, 0.24), // #15803d
                Color::srgb(0.08, 0.50, 0.24),
                1,
            ),
            JacketStyle::RunnerGold => (
                Color::srgb(0.92, 0.70, 0.03), // #eab308
                Color::srgb(0.92, 0.70, 0.03),
                1,
            ),
            JacketStyle::ReporterBurgundy => (
                Color::srgb(0.51, 0.09, 0.26), // #831843
                Color::srgb(0.51, 0.09, 0.26),
                1,
            ),
        }
    }

    pub fn tie_color(&self) -> Color {
        match self {
            JacketStyle::CargillRed => Color::srgb(0.12, 0.23, 0.54),       // Navy tie
            JacketStyle::DeanWitterBlue => Color::srgb(0.96, 0.62, 0.04),   // Gold tie
            JacketStyle::CarrGreen => Color::srgb(0.94, 0.27, 0.27),        // Red tie
            JacketStyle::FirstOptionsGold => Color::srgb(0.98, 0.98, 0.98), // White tie
            JacketStyle::RefcoCrimson => Color::srgb(0.98, 0.80, 0.08),     // Gold tie
            JacketStyle::SunOrange => Color::srgb(0.22, 0.74, 0.97),        // Sky blue tie
            JacketStyle::BrokerBlue => Color::srgb(0.96, 0.62, 0.04),       // Gold tie
            JacketStyle::ScalperGreen => Color::srgb(0.94, 0.27, 0.27),      // Red tie
            JacketStyle::RunnerGold => Color::srgb(0.01, 0.52, 0.78),       // Blue tie
            JacketStyle::ReporterBurgundy => Color::srgb(0.98, 0.80, 0.08), // Gold tie
        }
    }
}

/// Motion clip state for 3D UniMate characters
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UnimateClip {
    #[default]
    Idle,           // Resting stance: Arms hang down naturally at sides, gentle breathing & weight shift
    Buy,            // Palms In: Hands raised to chin, palms facing trader chest, waving white buy slip
    Sell,           // Palms Out: Arms pushed wide into pit crowd, palms facing crowd, waving pink sell slip
    PointDeal,      // Match Point: Right arm points straight at counterparty, sharp affirmative head nod
    Endorse,        // Card Scribble: Head down -30°, left hand holds card on chest, right hand scribbles rapidly
    QuantityChin,   // 10+ Lots: Knuckles/index finger touching chin/jaw, signaling block size
    PriceSlice,     // Horizontal Par/Fraction: Flat hand slicing across chest/throat signaling price level
    SpreadCross,    // Calendar Spread: Forearms crossed in front of sternum forming an "X"
    Dispute,        // Out-Trade: Leaning forward across step, waving pencil high, agitated head shake
    Sprint,         // Concourse sprint: Athletic forward lean, counter-twisting torso, running stride
    Pulpit,         // Price Reporter on central pulpit logging tape prints
}

/// Demographic skin and hair palette generator adhering to 80% White, 10% Black, 10% Brown
pub fn sample_demographics(seed: u64) -> (Ethnicity, Color, Color) {
    let roll = seed % 100;
    if roll < 80 {
        // 80% White
        let skins = [
            Color::srgb_u8(0xf5, 0xc7, 0x9e),
            Color::srgb_u8(0xfa, 0xd8, 0xbe),
            Color::srgb_u8(0xec, 0xd0, 0xb9),
            Color::srgb_u8(0xf3, 0xce, 0xb4),
            Color::srgb_u8(0xe8, 0xbf, 0x9b),
        ];
        let hairs = [
            Color::srgb_u8(0x26, 0x1a, 0x15), // Dark Brown
            Color::srgb_u8(0x4a, 0x2e, 0x1b), // Medium Brown
            Color::srgb_u8(0x8c, 0x5b, 0x36), // Auburn
            Color::srgb_u8(0xb4, 0x8a, 0x52), // Blond
            Color::srgb_u8(0x1a, 0x1a, 0x1a), // Jet Black
        ];
        let skin_idx = (seed as usize / 5) % skins.len();
        let hair_idx = (seed as usize / 13) % hairs.len();
        (Ethnicity::White, skins[skin_idx], hairs[hair_idx])
    } else if roll < 90 {
        // 10% Black
        let skins = [
            Color::srgb_u8(0x52, 0x32, 0x23),
            Color::srgb_u8(0x6e, 0x45, 0x30),
            Color::srgb_u8(0x41, 0x26, 0x19),
            Color::srgb_u8(0x5a, 0x38, 0x25),
            Color::srgb_u8(0x7c, 0x4e, 0x38),
        ];
        let hairs = [
            Color::srgb_u8(0x0a, 0x0a, 0x0a),
            Color::srgb_u8(0x14, 0x14, 0x14),
            Color::srgb_u8(0x05, 0x05, 0x05),
        ];
        let skin_idx = (seed as usize / 7) % skins.len();
        let hair_idx = (seed as usize / 11) % hairs.len();
        (Ethnicity::Black, skins[skin_idx], hairs[hair_idx])
    } else {
        // 10% Brown
        let skins = [
            Color::srgb_u8(0xb8, 0x7d, 0x56),
            Color::srgb_u8(0xa7, 0x68, 0x40),
            Color::srgb_u8(0xc6, 0x8c, 0x62),
            Color::srgb_u8(0x9b, 0x63, 0x3b),
            Color::srgb_u8(0x8c, 0x55, 0x32),
        ];
        let hairs = [
            Color::srgb_u8(0x11, 0x11, 0x11),
            Color::srgb_u8(0x1f, 0x14, 0x0e),
            Color::srgb_u8(0x2d, 0x1a, 0x10),
        ];
        let skin_idx = (seed as usize / 3) % skins.len();
        let hair_idx = (seed as usize / 17) % hairs.len();
        (Ethnicity::Brown, skins[skin_idx], hairs[hair_idx])
    }
}

/// Generates a crisp procedural vertical striped RGBA texture
pub fn generate_striped_image(color_a: Color, color_b: Color, num_stripes: u32, width: u32, height: u32) -> Image {
    let mut data = Vec::with_capacity((width * height * 4) as usize);
    let stripe_w = (width as f32 / num_stripes as f32).max(1.0);

    let [r_a, g_a, b_a, _] = color_a.to_srgba().to_u8_array();
    let [r_b, g_b, b_b, _] = color_b.to_srgba().to_u8_array();

    for _y in 0..height {
        for x in 0..width {
            let stripe_idx = (x as f32 / stripe_w).floor() as u32;
            let is_a = stripe_idx % 2 == 0;
            if is_a {
                data.push(r_a);
                data.push(g_a);
                data.push(b_a);
                data.push(255);
            } else {
                data.push(r_b);
                data.push(g_b);
                data.push(b_b);
                data.push(255);
            }
        }
    }

    let mut img = Image::new(
        Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
    );
    img.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor::nearest());
    img
}

/// Shared geometry mesh cache for UniMate characters
#[derive(Resource)]
pub struct UnimateMeshCache {
    pub torso: Handle<Mesh>,
    pub button: Handle<Mesh>,
    pub shirt: Handle<Mesh>,
    pub tie: Handle<Mesh>,
    pub badge: Handle<Mesh>,
    pub head: Handle<Mesh>,
    pub ear: Handle<Mesh>,
    pub hair_top: Handle<Mesh>,
    pub hair_back: Handle<Mesh>,
    pub hair_side: Handle<Mesh>,
    pub shades_rim: Handle<Mesh>,
    pub lens: Handle<Mesh>,
    pub temple: Handle<Mesh>,
    pub shoulder_pad: Handle<Mesh>,
    pub bicep: Handle<Mesh>,
    pub elbow_cap: Handle<Mesh>,
    pub forearm: Handle<Mesh>,
    pub sleeve_cuff: Handle<Mesh>,
    pub shirt_cuff: Handle<Mesh>,
    pub palm: Handle<Mesh>,
    pub thumb: Handle<Mesh>,
    pub fingers: [Handle<Mesh>; 4], // Index, Middle, Ring, Pinky
    pub ticket: Handle<Mesh>,
    pub thigh: Handle<Mesh>,
    pub shin: Handle<Mesh>,
    pub foot: Handle<Mesh>,
}

impl FromWorld for UnimateMeshCache {
    fn from_world(world: &mut World) -> Self {
        let mut meshes = world.resource_mut::<Assets<Mesh>>();

        let fingers = [
            meshes.add(Cuboid::new(0.22, 0.72, 0.32)), // Index
            meshes.add(Cuboid::new(0.22, 0.84, 0.32)), // Middle
            meshes.add(Cuboid::new(0.22, 0.76, 0.32)), // Ring
            meshes.add(Cuboid::new(0.20, 0.62, 0.32)), // Pinky
        ];

        Self {
            torso: meshes.add(Cuboid::new(3.6, 4.4, 2.2)),
            button: meshes.add(Cylinder::new(0.1, 0.08)),
            shirt: meshes.add(Cuboid::new(1.4, 3.8, 0.25)),
            tie: meshes.add(Cuboid::new(0.55, 2.8, 0.15)),
            badge: meshes.add(Cuboid::new(0.85, 0.55, 0.1)),
            head: meshes.add(Cuboid::new(1.8, 2.1, 1.8)),
            ear: meshes.add(Cuboid::new(0.18, 0.45, 0.28)),
            hair_top: meshes.add(Cuboid::new(1.9, 0.45, 1.6)),
            hair_back: meshes.add(Cuboid::new(1.9, 1.5, 0.55)),
            hair_side: meshes.add(Cuboid::new(0.16, 0.8, 0.45)),
            shades_rim: meshes.add(Cuboid::new(1.75, 0.42, 0.22)),
            lens: meshes.add(Cuboid::new(0.68, 0.34, 0.06)),
            temple: meshes.add(Cuboid::new(0.08, 0.08, 0.65)),
            shoulder_pad: meshes.add(Cuboid::new(1.22, 0.85, 1.22)),
            bicep: meshes.add(Cuboid::new(1.15, 2.7, 1.15)),
            elbow_cap: meshes.add(Cuboid::new(1.08, 0.6, 1.08)),
            forearm: meshes.add(Cuboid::new(1.0, 2.4, 1.0)),
            sleeve_cuff: meshes.add(Cuboid::new(1.08, 0.28, 1.08)),
            shirt_cuff: meshes.add(Cuboid::new(1.02, 0.16, 1.02)),
            palm: meshes.add(Cuboid::new(1.15, 1.1, 0.42)),
            thumb: meshes.add(Cuboid::new(0.32, 0.6, 0.34)),
            fingers,
            ticket: meshes.add(Cuboid::new(0.85, 1.5, 0.04)),
            thigh: meshes.add(Cuboid::new(1.2, 4.2, 1.3)),
            shin: meshes.add(Cuboid::new(1.1, 4.0, 1.2)),
            foot: meshes.add(Cuboid::new(1.25, 0.9, 2.2)),
        }
    }
}

/// Shared standard material cache for jackets, accessories, and tickets
#[derive(Resource)]
pub struct UnimateMaterialCache {
    pub jacket_torso: std::collections::HashMap<JacketStyle, Handle<StandardMaterial>>,
    pub jacket_sleeve: std::collections::HashMap<JacketStyle, Handle<StandardMaterial>>,
    pub pants_navy: Handle<StandardMaterial>,
    pub pants_charcoal: Handle<StandardMaterial>,
    pub shoes: Handle<StandardMaterial>,
    pub tie_materials: std::collections::HashMap<JacketStyle, Handle<StandardMaterial>>,
    pub collar_white: Handle<StandardMaterial>,
    pub gold_badge: Handle<StandardMaterial>,
    pub shades_rim: Handle<StandardMaterial>,
    pub shades_lens: Handle<StandardMaterial>,
    pub button: Handle<StandardMaterial>,
    pub ticket_buy_white: Handle<StandardMaterial>,
    pub ticket_sell_pink: Handle<StandardMaterial>,
}

impl FromWorld for UnimateMaterialCache {
    fn from_world(world: &mut World) -> Self {
        let all_styles = [
            JacketStyle::CargillRed,
            JacketStyle::DeanWitterBlue,
            JacketStyle::CarrGreen,
            JacketStyle::FirstOptionsGold,
            JacketStyle::RefcoCrimson,
            JacketStyle::SunOrange,
            JacketStyle::BrokerBlue,
            JacketStyle::ScalperGreen,
            JacketStyle::RunnerGold,
            JacketStyle::ReporterBurgundy,
        ];

        // 1. Generate all procedural stripe images first
        let mut torso_images = Vec::new();
        let mut sleeve_images = Vec::new();
        for &style in &all_styles {
            let (col_a, col_b, num_stripes) = style.colors();
            if style.is_striped() {
                let img_torso = generate_striped_image(col_a, col_b, num_stripes, 128, 128);
                let sleeve_stripes = (num_stripes as f32 / 2.5).round().max(3.0) as u32;
                let img_sleeve = generate_striped_image(col_a, col_b, sleeve_stripes, 128, 128);
                torso_images.push((style, img_torso));
                sleeve_images.push((style, img_sleeve));
            }
        }

        // 2. Insert images into Assets<Image>
        let mut torso_handles = std::collections::HashMap::new();
        let mut sleeve_handles = std::collections::HashMap::new();
        {
            let mut images = world.resource_mut::<Assets<Image>>();
            for (style, img) in torso_images {
                torso_handles.insert(style, images.add(img));
            }
            for (style, img) in sleeve_images {
                sleeve_handles.insert(style, images.add(img));
            }
        }

        // 3. Insert materials into Assets<StandardMaterial>
        let mut materials = world.resource_mut::<Assets<StandardMaterial>>();
        let mut jacket_torso = std::collections::HashMap::new();
        let mut jacket_sleeve = std::collections::HashMap::new();
        let mut tie_materials = std::collections::HashMap::new();

        for &style in &all_styles {
            let (col_a, _, _) = style.colors();
            if style.is_striped() {
                let h_torso = torso_handles.get(&style).unwrap().clone();
                jacket_torso.insert(
                    style,
                    materials.add(StandardMaterial {
                        base_color_texture: Some(h_torso),
                        perceptual_roughness: 0.55,
                        metallic: 0.05,
                        ..default()
                    }),
                );

                let h_sleeve = sleeve_handles.get(&style).unwrap().clone();
                jacket_sleeve.insert(
                    style,
                    materials.add(StandardMaterial {
                        base_color_texture: Some(h_sleeve),
                        perceptual_roughness: 0.55,
                        metallic: 0.05,
                        ..default()
                    }),
                );
            } else {
                let mat = materials.add(StandardMaterial {
                    base_color: col_a,
                    perceptual_roughness: 0.55,
                    metallic: 0.05,
                    ..default()
                });
                jacket_torso.insert(style, mat.clone());
                jacket_sleeve.insert(style, mat);
            }

            tie_materials.insert(
                style,
                materials.add(StandardMaterial {
                    base_color: style.tie_color(),
                    perceptual_roughness: 0.40,
                    metallic: 0.30,
                    ..default()
                }),
            );
        }

        Self {
            jacket_torso,
            jacket_sleeve,
            pants_navy: materials.add(StandardMaterial {
                base_color: Color::srgb_u8(0x1a, 0x21, 0x30),
                perceptual_roughness: 0.70,
                ..default()
            }),
            pants_charcoal: materials.add(StandardMaterial {
                base_color: Color::srgb_u8(0x22, 0x26, 0x30),
                perceptual_roughness: 0.70,
                ..default()
            }),
            shoes: materials.add(StandardMaterial {
                base_color: Color::srgb_u8(0x0f, 0x17, 0x2a),
                perceptual_roughness: 0.40,
                ..default()
            }),
            tie_materials,
            collar_white: materials.add(StandardMaterial {
                base_color: Color::srgb(0.97, 0.98, 0.99),
                perceptual_roughness: 0.50,
                ..default()
            }),
            gold_badge: materials.add(StandardMaterial {
                base_color: Color::srgb(0.98, 0.80, 0.08),
                perceptual_roughness: 0.25,
                metallic: 0.80,
                ..default()
            }),
            shades_rim: materials.add(StandardMaterial {
                base_color: Color::srgb_u8(0x05, 0x08, 0x10),
                perceptual_roughness: 0.10,
                metallic: 0.90,
                ..default()
            }),
            shades_lens: materials.add(StandardMaterial {
                base_color: Color::srgb_u8(0x01, 0x03, 0x07),
                perceptual_roughness: 0.05,
                metallic: 0.95,
                ..default()
            }),
            button: materials.add(StandardMaterial {
                base_color: Color::srgb_u8(0x1f, 0x24, 0x2e),
                perceptual_roughness: 0.30,
                metallic: 0.50,
                ..default()
            }),
            ticket_buy_white: materials.add(StandardMaterial {
                base_color: Color::srgb_u8(0xf0, 0xf9, 0xff),
                perceptual_roughness: 0.40,
                ..default()
            }),
            ticket_sell_pink: materials.add(StandardMaterial {
                base_color: Color::srgb_u8(0xfd, 0xa4, 0xaf),
                perceptual_roughness: 0.40,
                ..default()
            }),
        }
    }
}

/// Tag component marking a 3D UniMate articulated trader
#[derive(Component, Debug, Clone)]
pub struct UnimateTrader {
    pub clip: UnimateClip,
    pub ethnicity: Ethnicity,
    pub jacket: JacketStyle,
    pub phase_offset: f32,
    pub palm_intensity: f32,
}

/// Identifies specific articulated kinematic joints on the UniMate skeleton
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub enum JointType {
    Pelvis,
    Torso,
    Neck,
    Head,
    LeftShoulder,
    RightShoulder,
    LeftUpperArm,
    RightUpperArm,
    LeftElbow,
    RightElbow,
    LeftWrist,
    RightWrist,
    LeftFingers,
    RightFingers,
    LeftThumb,
    RightThumb,
    LeftTicket,
    RightTicket,
    LeftPalmLight,
    RightPalmLight,
    LeftHip,
    RightHip,
    LeftKnee,
    RightKnee,
}

/// Spawns a full 18-part articulated 3D UniMate trader entity hierarchy
pub fn spawn_unimate_trader_entity(
    commands: &mut Commands,
    meshes: &UnimateMeshCache,
    mats: &UnimateMaterialCache,
    material_assets: &mut Assets<StandardMaterial>,
    position: Vec3,
    facing_y: f32,
    clip: UnimateClip,
    jacket: JacketStyle,
    seed: u64,
) -> Entity {
    let (ethnicity, skin_color, hair_color) = sample_demographics(seed);

    let skin_mat = material_assets.add(StandardMaterial {
        base_color: skin_color,
        perceptual_roughness: 0.65,
        ..default()
    });
    let hair_mat = material_assets.add(StandardMaterial {
        base_color: hair_color,
        perceptual_roughness: 0.85,
        ..default()
    });

    let coat_torso_mat = mats.jacket_torso.get(&jacket).unwrap().clone();
    let coat_sleeve_mat = mats.jacket_sleeve.get(&jacket).unwrap().clone();
    let tie_mat = mats.tie_materials.get(&jacket).unwrap().clone();
    let pants_mat = if seed % 2 == 0 {
        mats.pants_navy.clone()
    } else {
        mats.pants_charcoal.clone()
    };

    let root = commands
        .spawn((
            UnimateTrader {
                clip,
                ethnicity,
                jacket,
                phase_offset: (seed as f32 % 100.0) * 0.05,
                palm_intensity: 0.0,
            },
            Transform::from_translation(position).with_rotation(Quat::from_rotation_y(facing_y)),
            Visibility::Inherited,
        ))
        .with_children(|root_builder| {
            // 1. Pelvis Root (Y = 8.5)
            root_builder
                .spawn((
                    JointType::Pelvis,
                    Transform::from_xyz(0.0, 8.5, 0.0),
                    Visibility::Inherited,
                ))
                .with_children(|pelvis| {
                    // 2. Spine & Torso (Y = 0.5)
                    pelvis
                        .spawn((
                            JointType::Torso,
                            Transform::from_xyz(0.0, 0.5, 0.0),
                            Visibility::Inherited,
                        ))
                        .with_children(|torso| {
                            // Jacket Blazer Mesh
                            torso.spawn((
                                Mesh3d(meshes.torso.clone()),
                                MeshMaterial3d(coat_torso_mat),
                                Transform::from_xyz(0.0, 2.2, 0.0),
                            ));

                            // CME Blazer Buttons
                            for y_off in [-0.75, 0.0, 0.75] {
                                torso.spawn((
                                    Mesh3d(meshes.button.clone()),
                                    MeshMaterial3d(mats.button.clone()),
                                    Transform::from_xyz(0.0, 2.2 + y_off, 1.14)
                                        .with_rotation(Quat::from_rotation_x(std::f32::consts::FRAC_PI_2)),
                                ));
                            }

                            // Shirt Collar
                            torso.spawn((
                                Mesh3d(meshes.shirt.clone()),
                                MeshMaterial3d(mats.collar_white.clone()),
                                Transform::from_xyz(0.0, 2.5, 1.1),
                            ));

                            // Necktie
                            torso.spawn((
                                Mesh3d(meshes.tie.clone()),
                                MeshMaterial3d(tie_mat),
                                Transform::from_xyz(0.0, 2.2, 1.25),
                            ));

                            // CME Gold Execution Badge
                            torso.spawn((
                                Mesh3d(meshes.badge.clone()),
                                MeshMaterial3d(mats.gold_badge.clone()),
                                Transform::from_xyz(1.0, 3.2, 1.22),
                            ));

                            // 3. Neck & Head (Y = 4.4)
                            torso
                                .spawn((
                                    JointType::Neck,
                                    Transform::from_xyz(0.0, 4.4, 0.0),
                                    Visibility::Inherited,
                                ))
                                .with_children(|neck| {
                                    // Head
                                    neck.spawn((
                                        JointType::Head,
                                        Mesh3d(meshes.head.clone()),
                                        MeshMaterial3d(skin_mat.clone()),
                                        Transform::from_xyz(0.0, 1.25, 0.0),
                                    ));

                                    // Ears
                                    neck.spawn((
                                        Mesh3d(meshes.ear.clone()),
                                        MeshMaterial3d(skin_mat.clone()),
                                        Transform::from_xyz(-0.95, 1.25, -0.05),
                                    ));
                                    neck.spawn((
                                        Mesh3d(meshes.ear.clone()),
                                        MeshMaterial3d(skin_mat.clone()),
                                        Transform::from_xyz(0.95, 1.25, -0.05),
                                    ));

                                    // Hair Cap
                                    neck.spawn((
                                        Mesh3d(meshes.hair_top.clone()),
                                        MeshMaterial3d(hair_mat.clone()),
                                        Transform::from_xyz(0.0, 2.22, -0.15),
                                    ));
                                    neck.spawn((
                                        Mesh3d(meshes.hair_back.clone()),
                                        MeshMaterial3d(hair_mat.clone()),
                                        Transform::from_xyz(0.0, 1.45, -0.75),
                                    ));
                                    neck.spawn((
                                        Mesh3d(meshes.hair_side.clone()),
                                        MeshMaterial3d(hair_mat.clone()),
                                        Transform::from_xyz(-0.92, 1.65, 0.05),
                                    ));
                                    neck.spawn((
                                        Mesh3d(meshes.hair_side.clone()),
                                        MeshMaterial3d(hair_mat),
                                        Transform::from_xyz(0.92, 1.65, 0.05),
                                    ));

                                    // Retro Sunglasses / Visor
                                    neck.spawn((
                                        Mesh3d(meshes.shades_rim.clone()),
                                        MeshMaterial3d(mats.shades_rim.clone()),
                                        Transform::from_xyz(0.0, 1.35, 0.96),
                                    ));
                                    neck.spawn((
                                        Mesh3d(meshes.lens.clone()),
                                        MeshMaterial3d(mats.shades_lens.clone()),
                                        Transform::from_xyz(-0.45, 1.35, 1.08),
                                    ));
                                    neck.spawn((
                                        Mesh3d(meshes.lens.clone()),
                                        MeshMaterial3d(mats.shades_lens.clone()),
                                        Transform::from_xyz(0.45, 1.35, 1.08),
                                    ));
                                    neck.spawn((
                                        Mesh3d(meshes.temple.clone()),
                                        MeshMaterial3d(mats.shades_rim.clone()),
                                        Transform::from_xyz(-0.9, 1.35, 0.6),
                                    ));
                                    neck.spawn((
                                        Mesh3d(meshes.temple.clone()),
                                        MeshMaterial3d(mats.shades_rim.clone()),
                                        Transform::from_xyz(0.9, 1.35, 0.6),
                                    ));
                                });

                            // 4. Arms (Left and Right Shoulders, Hinges, Forearms, Hands)
                            for is_left in [true, false] {
                                let sign = if is_left { -1.0 } else { 1.0 };
                                let shoulder_joint = if is_left { JointType::LeftShoulder } else { JointType::RightShoulder };
                                let upper_arm_joint = if is_left { JointType::LeftUpperArm } else { JointType::RightUpperArm };
                                let elbow_joint = if is_left { JointType::LeftElbow } else { JointType::RightElbow };
                                let wrist_joint = if is_left { JointType::LeftWrist } else { JointType::RightWrist };
                                let fingers_joint = if is_left { JointType::LeftFingers } else { JointType::RightFingers };
                                let thumb_joint = if is_left { JointType::LeftThumb } else { JointType::RightThumb };
                                let ticket_joint = if is_left { JointType::LeftTicket } else { JointType::RightTicket };
                                let palm_light_joint = if is_left { JointType::LeftPalmLight } else { JointType::RightPalmLight };

                                torso
                                    .spawn((
                                        shoulder_joint,
                                        Transform::from_xyz(sign * 2.1, 4.0, 0.0),
                                        Visibility::Inherited,
                                    ))
                                    .with_children(|shoulder| {
                                        // Shoulder Pad
                                        shoulder.spawn((
                                            Mesh3d(meshes.shoulder_pad.clone()),
                                            MeshMaterial3d(coat_sleeve_mat.clone()),
                                            Transform::from_xyz(0.0, -0.15, 0.0),
                                        ));

                                        // Upper Arm
                                        shoulder
                                            .spawn((
                                                upper_arm_joint,
                                                Transform::IDENTITY,
                                                Visibility::Inherited,
                                            ))
                                            .with_children(|upper_arm| {
                                                upper_arm.spawn((
                                                    Mesh3d(meshes.bicep.clone()),
                                                    MeshMaterial3d(coat_sleeve_mat.clone()),
                                                    Transform::from_xyz(0.0, -1.35, 0.0),
                                                ));

                                                // Elbow Hinge (Y = -2.7)
                                                upper_arm
                                                    .spawn((
                                                        elbow_joint,
                                                        Transform::from_xyz(0.0, -2.7, 0.0),
                                                        Visibility::Inherited,
                                                    ))
                                                    .with_children(|elbow| {
                                                        elbow.spawn((
                                                            Mesh3d(meshes.elbow_cap.clone()),
                                                            MeshMaterial3d(coat_sleeve_mat.clone()),
                                                            Transform::IDENTITY,
                                                        ));

                                                        elbow.spawn((
                                                            Mesh3d(meshes.forearm.clone()),
                                                            MeshMaterial3d(coat_sleeve_mat.clone()),
                                                            Transform::from_xyz(0.0, -1.2, 0.0),
                                                        ));

                                                        elbow.spawn((
                                                            Mesh3d(meshes.sleeve_cuff.clone()),
                                                            MeshMaterial3d(coat_sleeve_mat.clone()),
                                                            Transform::from_xyz(0.0, -2.25, 0.0),
                                                        ));

                                                        elbow.spawn((
                                                            Mesh3d(meshes.shirt_cuff.clone()),
                                                            MeshMaterial3d(mats.collar_white.clone()),
                                                            Transform::from_xyz(0.0, -2.42, 0.0),
                                                        ));

                                                        // Wrist Joint (Y = -2.5)
                                                        elbow
                                                            .spawn((
                                                                wrist_joint,
                                                                Transform::from_xyz(0.0, -2.5, 0.0),
                                                                Visibility::Inherited,
                                                            ))
                                                            .with_children(|wrist| {
                                                                // Chunky Articulated Palm
                                                                wrist.spawn((
                                                                    Mesh3d(meshes.palm.clone()),
                                                                    MeshMaterial3d(skin_mat.clone()),
                                                                    Transform::from_xyz(0.0, -0.55, 0.0),
                                                                ));

                                                                // Opposable Thumb
                                                                wrist
                                                                    .spawn((
                                                                        thumb_joint,
                                                                        Transform::from_xyz(sign * 0.58, -0.4, 0.08)
                                                                            .with_rotation(Quat::from_euler(EulerRot::XYZ, -0.15, sign * -0.3, 0.0)),
                                                                        Visibility::Inherited,
                                                                    ))
                                                                    .with_children(|thumb| {
                                                                        thumb.spawn((
                                                                            Mesh3d(meshes.thumb.clone()),
                                                                            MeshMaterial3d(skin_mat.clone()),
                                                                            Transform::from_xyz(sign * 0.08, -0.22, 0.08),
                                                                        ));
                                                                    });

                                                                // 4 Articulated Fingers
                                                                wrist
                                                                    .spawn((
                                                                        fingers_joint,
                                                                        Transform::from_xyz(0.0, -1.1, 0.0),
                                                                        Visibility::Inherited,
                                                                    ))
                                                                    .with_children(|fingers_node| {
                                                                        let x_offsets = [-0.33, -0.11, 0.11, 0.33];
                                                                        for (f_idx, &x_off) in x_offsets.iter().enumerate() {
                                                                            let f_len = match f_idx {
                                                                                0 => 0.72,
                                                                                1 => 0.84,
                                                                                2 => 0.76,
                                                                                _ => 0.62,
                                                                            };
                                                                            fingers_node.spawn((
                                                                                Mesh3d(meshes.fingers[f_idx].clone()),
                                                                                MeshMaterial3d(skin_mat.clone()),
                                                                                Transform::from_xyz(sign * x_off, -f_len / 2.0, 0.0),
                                                                            ));
                                                                        }
                                                                    });

                                                                // CME Order Ticket (White = Buy, Pink = Sell)
                                                                wrist.spawn((
                                                                    ticket_joint,
                                                                    Mesh3d(meshes.ticket.clone()),
                                                                    MeshMaterial3d(mats.ticket_buy_white.clone()),
                                                                    Transform::from_xyz(sign * 0.25, -0.75, 0.22),
                                                                    Visibility::Hidden, // Initially hidden while idle
                                                                ));

                                                                // Dynamic Palm Point Light Aura
                                                                wrist.spawn((
                                                                    palm_light_joint,
                                                                    PointLight {
                                                                        color: Color::srgb(0.13, 0.77, 0.37),
                                                                        intensity: 0.0,
                                                                        range: 8.0,
                                                                        shadows_enabled: false,
                                                                        ..default()
                                                                    },
                                                                    Transform::from_xyz(0.0, -0.55, 0.35),
                                                                ));
                                                            });
                                                    });
                                            });
                                    });
                            }
                        });

                    // 5. Legs (Left and Right Hip Hinges)
                    for is_left in [true, false] {
                        let sign = if is_left { -1.0 } else { 1.0 };
                        let hip_joint = if is_left { JointType::LeftHip } else { JointType::RightHip };
                        let knee_joint = if is_left { JointType::LeftKnee } else { JointType::RightKnee };

                        pelvis
                            .spawn((
                                hip_joint,
                                Transform::from_xyz(sign * 1.05, 0.0, 0.0),
                                Visibility::Inherited,
                            ))
                            .with_children(|hip| {
                                hip.spawn((
                                    Mesh3d(meshes.thigh.clone()),
                                    MeshMaterial3d(pants_mat.clone()),
                                    Transform::from_xyz(0.0, -2.1, 0.0),
                                ));

                                // Knee Hinge (Y = -4.2)
                                hip
                                    .spawn((
                                        knee_joint,
                                        Transform::from_xyz(0.0, -4.2, 0.0),
                                        Visibility::Inherited,
                                    ))
                                    .with_children(|knee| {
                                        knee.spawn((
                                            Mesh3d(meshes.shin.clone()),
                                            MeshMaterial3d(pants_mat.clone()),
                                            Transform::from_xyz(0.0, -2.0, 0.0),
                                        ));

                                        // Dress Shoe
                                        knee.spawn((
                                            Mesh3d(meshes.foot.clone()),
                                            MeshMaterial3d(mats.shoes.clone()),
                                            Transform::from_xyz(0.0, -4.2, 0.4),
                                        ));
                                    });
                            });
                    }
                });
        })
        .id();

    root
}

/// Forward kinematics animation system evaluating authentic human pit gestures
pub fn update_unimate_kinematics(
    time: Res<Time>,
    view_mode: Res<crate::camera::FloorViewMode>,
    mats: Res<UnimateMaterialCache>,
    traders: Query<(&UnimateTrader, &Children, Option<&Visibility>), Without<JointType>>,
    mut joints: Query<(
        &JointType,
        &mut Transform,
        Option<&mut Visibility>,
        Option<&mut PointLight>,
        Option<&mut MeshMaterial3d<StandardMaterial>>,
        Option<&Children>,
    ), Without<UnimateTrader>>,
) {
    if !view_mode.is_3d() {
        return;
    }
    let t_base = time.elapsed_secs();

    for (trader, trader_children, opt_vis) in traders.iter() {
        if let Some(vis) = opt_vis {
            if *vis == Visibility::Hidden {
                continue;
            }
        }
        let t = t_base + trader.phase_offset;

        // Traverse hierarchy from root trader down through joints
        let mut stack: Vec<Entity> = trader_children.iter().collect();

        while let Some(entity) = stack.pop() {
            if let Ok((joint_type, mut transform, opt_vis, opt_light, opt_mat, opt_children)) = joints.get_mut(entity) {
                match joint_type {
                    JointType::Pelvis => match trader.clip {
                        UnimateClip::Idle => {
                            let breath = (t * 2.2).sin() * 0.08;
                            transform.translation.y = 8.5 + breath * 0.5;
                            transform.rotation = Quat::IDENTITY;
                        }
                        UnimateClip::Buy => {
                            transform.translation.y = 8.6;
                            transform.rotation = Quat::IDENTITY;
                        }
                        UnimateClip::Sell => {
                            transform.translation.y = 8.35;
                            transform.rotation = Quat::IDENTITY;
                        }
                        UnimateClip::PointDeal => {
                            transform.translation.y = 8.52;
                            transform.rotation = Quat::IDENTITY;
                        }
                        UnimateClip::Endorse => {
                            transform.translation.y = 8.44;
                            transform.rotation = Quat::IDENTITY;
                        }
                        UnimateClip::QuantityChin => {
                            transform.translation.y = 8.58;
                            transform.rotation = Quat::IDENTITY;
                        }
                        UnimateClip::PriceSlice => {
                            transform.translation.y = 8.50;
                            transform.rotation = Quat::IDENTITY;
                        }
                        UnimateClip::SpreadCross => {
                            transform.translation.y = 8.50;
                            transform.rotation = Quat::IDENTITY;
                        }
                        UnimateClip::Dispute => {
                            let shake = (t * 8.0).sin() * 0.08;
                            transform.translation.y = 8.42 + shake.abs() * 0.1;
                            transform.rotation = Quat::IDENTITY;
                        }
                        UnimateClip::Sprint => {
                            let stride = (t * 10.0).sin();
                            transform.translation.y = 8.3 + stride.abs() * 0.6;
                            transform.rotation = Quat::IDENTITY;
                        }
                        UnimateClip::Pulpit => {
                            transform.translation.y = 8.5;
                            transform.rotation = Quat::IDENTITY;
                        }
                    },
                    JointType::Torso => match trader.clip {
                        UnimateClip::Idle => {
                            let breath = (t * 2.2).sin() * 0.08;
                            let weight_shift = (t * 0.9).sin() * 0.25;
                            transform.translation.y = 0.5 + breath;
                            transform.rotation = Quat::from_euler(EulerRot::XYZ, 0.0, 0.0, weight_shift * 0.04);
                        }
                        UnimateClip::Buy => {
                            let lean = 0.24 + (t * 6.0).sin() * 0.06;
                            let twist = (t * 3.0).sin() * 0.10;
                            transform.translation.y = 0.5;
                            transform.rotation = Quat::from_euler(EulerRot::XYZ, lean, twist, 0.0);
                        }
                        UnimateClip::Sell => {
                            let brace = -0.14;
                            let twist = (t * 3.5).sin() * 0.12;
                            transform.translation.y = 0.5;
                            transform.rotation = Quat::from_euler(EulerRot::XYZ, brace, twist, 0.0);
                        }
                        UnimateClip::PointDeal => {
                            transform.translation.y = 0.5;
                            transform.rotation = Quat::from_euler(EulerRot::XYZ, 0.14, 0.05, 0.0);
                        }
                        UnimateClip::Endorse => {
                            let breath = (t * 3.0).sin() * 0.03;
                            transform.translation.y = 0.48;
                            transform.rotation = Quat::from_euler(EulerRot::XYZ, 0.26 + breath, 0.0, 0.0);
                        }
                        UnimateClip::QuantityChin => {
                            transform.translation.y = 0.52;
                            transform.rotation = Quat::from_euler(EulerRot::XYZ, -0.06, 0.0, 0.0);
                        }
                        UnimateClip::PriceSlice => {
                            let slice_twist = (t * 4.5).sin() * 0.10;
                            transform.translation.y = 0.5;
                            transform.rotation = Quat::from_euler(EulerRot::XYZ, 0.06, slice_twist, 0.0);
                        }
                        UnimateClip::SpreadCross => {
                            transform.translation.y = 0.5;
                            transform.rotation = Quat::from_euler(EulerRot::XYZ, 0.08, 0.0, 0.0);
                        }
                        UnimateClip::Dispute => {
                            let agitated = (t * 12.0).sin() * 0.08;
                            transform.translation.y = 0.5;
                            transform.rotation = Quat::from_euler(EulerRot::XYZ, 0.32, agitated, 0.0);
                        }
                        UnimateClip::Sprint => {
                            let stride = (t * 10.0).sin();
                            transform.translation.y = 0.5;
                            transform.rotation = Quat::from_euler(EulerRot::XYZ, 0.32, -stride * 0.18, 0.0);
                        }
                        UnimateClip::Pulpit => {
                            transform.translation.y = 0.5;
                            transform.rotation = Quat::from_euler(EulerRot::XYZ, 0.15, (t * 0.5).sin() * 0.08, 0.0);
                        }
                    },
                    JointType::Neck => match trader.clip {
                        UnimateClip::Idle => {
                            let pan = (t * 0.9).sin() * 0.22;
                            transform.rotation = Quat::from_rotation_y(pan);
                        }
                        UnimateClip::Buy => {
                            transform.rotation = Quat::from_rotation_x(-0.18);
                        }
                        UnimateClip::Sell => {
                            transform.rotation = Quat::from_rotation_x(0.10);
                        }
                        UnimateClip::PointDeal => {
                            let nod = (t * 6.0).sin().max(0.0) * 0.18;
                            transform.rotation = Quat::from_rotation_x(0.08 + nod);
                        }
                        UnimateClip::Endorse => {
                            transform.rotation = Quat::from_rotation_x(0.38);
                        }
                        UnimateClip::QuantityChin => {
                            transform.rotation = Quat::from_euler(EulerRot::XYZ, -0.14, 0.10, 0.0);
                        }
                        UnimateClip::PriceSlice => {
                            transform.rotation = Quat::from_rotation_x(-0.06);
                        }
                        UnimateClip::SpreadCross => {
                            transform.rotation = Quat::from_rotation_x(-0.04);
                        }
                        UnimateClip::Dispute => {
                            let shake = (t * 12.0).sin() * 0.22;
                            transform.rotation = Quat::from_euler(EulerRot::XYZ, 0.15, shake, 0.0);
                        }
                        UnimateClip::Sprint => {
                            transform.rotation = Quat::from_rotation_x(-0.25);
                        }
                        UnimateClip::Pulpit => {
                            transform.rotation = Quat::from_rotation_x(0.12);
                        }
                    },
                    JointType::Head => match trader.clip {
                        UnimateClip::Idle => {
                            let look = (t * 0.9).sin() * 0.25;
                            let tilt = (t * 0.45).cos() * 0.08;
                            transform.rotation = Quat::from_euler(EulerRot::XYZ, tilt, look, 0.0);
                        }
                        UnimateClip::PointDeal => {
                            let nod = (t * 6.0).sin().max(0.0) * 0.28;
                            transform.rotation = Quat::from_rotation_x(nod);
                        }
                        UnimateClip::Endorse => {
                            transform.rotation = Quat::from_rotation_x(0.32);
                        }
                        UnimateClip::QuantityChin => {
                            transform.rotation = Quat::from_rotation_x(-0.10);
                        }
                        UnimateClip::Dispute => {
                            let bob = (t * 10.0).sin() * 0.14;
                            transform.rotation = Quat::from_rotation_x(bob);
                        }
                        _ => transform.rotation = Quat::IDENTITY,
                    },
                    JointType::LeftShoulder => match trader.clip {
                        UnimateClip::Idle => transform.rotation = Quat::from_euler(EulerRot::XYZ, 0.0, 0.0, -0.06),
                        UnimateClip::Buy => transform.rotation = Quat::from_euler(EulerRot::XYZ, -0.15, 0.1, -0.05),
                        UnimateClip::Sell => transform.rotation = Quat::from_euler(EulerRot::XYZ, -0.1, -0.15, -0.15),
                        UnimateClip::PointDeal => transform.rotation = Quat::from_euler(EulerRot::XYZ, -0.08, 0.08, -0.06),
                        UnimateClip::Endorse => transform.rotation = Quat::from_euler(EulerRot::XYZ, -0.20, 0.16, -0.10),
                        UnimateClip::QuantityChin => transform.rotation = Quat::from_euler(EulerRot::XYZ, 0.0, 0.0, -0.06),
                        UnimateClip::PriceSlice => transform.rotation = Quat::from_euler(EulerRot::XYZ, 0.0, 0.0, -0.06),
                        UnimateClip::SpreadCross => transform.rotation = Quat::from_euler(EulerRot::XYZ, -0.22, 0.18, -0.12),
                        UnimateClip::Dispute => transform.rotation = Quat::from_euler(EulerRot::XYZ, -0.18, 0.20, -0.20),
                        UnimateClip::Sprint => transform.rotation = Quat::IDENTITY,
                        UnimateClip::Pulpit => transform.rotation = Quat::from_euler(EulerRot::XYZ, -0.2, 0.1, 0.0),
                    },
                    JointType::RightShoulder => match trader.clip {
                        UnimateClip::Idle => transform.rotation = Quat::from_euler(EulerRot::XYZ, 0.0, 0.0, 0.06),
                        UnimateClip::Buy => transform.rotation = Quat::from_euler(EulerRot::XYZ, -0.15, -0.1, 0.05),
                        UnimateClip::Sell => transform.rotation = Quat::from_euler(EulerRot::XYZ, -0.1, 0.15, 0.15),
                        UnimateClip::PointDeal => transform.rotation = Quat::from_euler(EulerRot::XYZ, -0.25, -0.18, 0.14),
                        UnimateClip::Endorse => transform.rotation = Quat::from_euler(EulerRot::XYZ, -0.20, -0.16, 0.10),
                        UnimateClip::QuantityChin => transform.rotation = Quat::from_euler(EulerRot::XYZ, -0.26, -0.12, 0.16),
                        UnimateClip::PriceSlice => transform.rotation = Quat::from_euler(EulerRot::XYZ, -0.16, -0.12, 0.16),
                        UnimateClip::SpreadCross => transform.rotation = Quat::from_euler(EulerRot::XYZ, -0.22, -0.18, 0.12),
                        UnimateClip::Dispute => transform.rotation = Quat::from_euler(EulerRot::XYZ, -0.32, -0.22, 0.26),
                        UnimateClip::Sprint => transform.rotation = Quat::IDENTITY,
                        UnimateClip::Pulpit => transform.rotation = Quat::from_euler(EulerRot::XYZ, -0.2, -0.1, 0.0),
                    },
                    JointType::LeftUpperArm => match trader.clip {
                        UnimateClip::Idle => transform.rotation = Quat::IDENTITY,
                        UnimateClip::Buy => {
                            let wave = (t * 12.0).sin() * 0.12;
                            transform.rotation = Quat::from_euler(EulerRot::XYZ, -0.85 + wave * 0.4, 0.0, -0.22);
                        }
                        UnimateClip::Sell => {
                            let push = (t * 9.0).sin() * 0.12;
                            transform.rotation = Quat::from_euler(EulerRot::XYZ, -1.25 + push * 0.5, 0.0, -0.42);
                        }
                        UnimateClip::PointDeal => {
                            transform.rotation = Quat::from_euler(EulerRot::XYZ, -0.60, 0.18, -0.24);
                        }
                        UnimateClip::Endorse => {
                            transform.rotation = Quat::from_euler(EulerRot::XYZ, -0.70, 0.38, -0.32);
                        }
                        UnimateClip::QuantityChin => {
                            transform.rotation = Quat::from_euler(EulerRot::XYZ, -0.40, 0.12, -0.18);
                        }
                        UnimateClip::PriceSlice => {
                            transform.rotation = Quat::from_euler(EulerRot::XYZ, -0.30, 0.0, -0.12);
                        }
                        UnimateClip::SpreadCross => {
                            transform.rotation = Quat::from_euler(EulerRot::XYZ, -0.92, 0.48, -0.36);
                        }
                        UnimateClip::Dispute => {
                            let jitter = (t * 12.0).sin() * 0.08;
                            transform.rotation = Quat::from_euler(EulerRot::XYZ, -0.82 + jitter, 0.32, -0.48);
                        }
                        UnimateClip::Sprint => {
                            let arm_swing = (t * 10.0).sin();
                            transform.rotation = Quat::from_rotation_x(arm_swing * 0.9);
                        }
                        UnimateClip::Pulpit => transform.rotation = Quat::from_euler(EulerRot::XYZ, -0.6, 0.0, -0.15),
                    },
                    JointType::RightUpperArm => match trader.clip {
                        UnimateClip::Idle => transform.rotation = Quat::IDENTITY,
                        UnimateClip::Buy => {
                            let wave = (t * 12.0).sin() * 0.12;
                            transform.rotation = Quat::from_euler(EulerRot::XYZ, -0.85 - wave * 0.4, 0.0, 0.22);
                        }
                        UnimateClip::Sell => {
                            let push = (t * 9.0).sin() * 0.12;
                            transform.rotation = Quat::from_euler(EulerRot::XYZ, -1.25 + push * 0.5, 0.0, 0.42);
                        }
                        UnimateClip::PointDeal => {
                            let jolt = (t * 6.0).sin().max(0.0) * 0.08;
                            transform.rotation = Quat::from_euler(EulerRot::XYZ, -1.48 - jolt, -0.06, 0.08);
                        }
                        UnimateClip::Endorse => {
                            transform.rotation = Quat::from_euler(EulerRot::XYZ, -0.70, -0.38, 0.32);
                        }
                        UnimateClip::QuantityChin => {
                            transform.rotation = Quat::from_euler(EulerRot::XYZ, -1.90, -0.28, 0.46);
                        }
                        UnimateClip::PriceSlice => {
                            let slice = (t * 4.5).sin();
                            transform.rotation = Quat::from_euler(EulerRot::XYZ, -1.18, slice * 0.42, 0.36);
                        }
                        UnimateClip::SpreadCross => {
                            transform.rotation = Quat::from_euler(EulerRot::XYZ, -0.92, -0.48, 0.36);
                        }
                        UnimateClip::Dispute => {
                            let wave = (t * 12.0).sin() * 0.28;
                            transform.rotation = Quat::from_euler(EulerRot::XYZ, -2.15 + wave, -0.22, 0.42);
                        }
                        UnimateClip::Sprint => {
                            let arm_swing = (t * 10.0).sin();
                            transform.rotation = Quat::from_rotation_x(-arm_swing * 0.9);
                        }
                        UnimateClip::Pulpit => transform.rotation = Quat::from_euler(EulerRot::XYZ, -0.6, 0.0, 0.15),
                    },
                    JointType::LeftElbow => match trader.clip {
                        UnimateClip::Idle => transform.rotation = Quat::from_rotation_x(-0.12),
                        UnimateClip::Buy => {
                            let wave = (t * 12.0).sin() * 0.12;
                            transform.rotation = Quat::from_rotation_x(-1.65 + wave * 0.35);
                        }
                        UnimateClip::Sell => {
                            let push = (t * 9.0).sin() * 0.12;
                            transform.rotation = Quat::from_rotation_x(-0.38 - push * 0.25);
                        }
                        UnimateClip::PointDeal => transform.rotation = Quat::from_rotation_x(-1.42),
                        UnimateClip::Endorse => transform.rotation = Quat::from_rotation_x(-1.62),
                        UnimateClip::QuantityChin => transform.rotation = Quat::from_rotation_x(-0.48),
                        UnimateClip::PriceSlice => transform.rotation = Quat::from_rotation_x(-0.38),
                        UnimateClip::SpreadCross => transform.rotation = Quat::from_rotation_x(-1.52),
                        UnimateClip::Dispute => transform.rotation = Quat::from_rotation_x(-0.52),
                        UnimateClip::Sprint => transform.rotation = Quat::from_rotation_x(-1.4),
                        UnimateClip::Pulpit => transform.rotation = Quat::from_rotation_x(-1.1),
                    },
                    JointType::RightElbow => match trader.clip {
                        UnimateClip::Idle => transform.rotation = Quat::from_rotation_x(-0.12),
                        UnimateClip::Buy => {
                            let wave = (t * 12.0).sin() * 0.12;
                            transform.rotation = Quat::from_rotation_x(-1.65 - wave * 0.35);
                        }
                        UnimateClip::Sell => {
                            let push = (t * 9.0).sin() * 0.12;
                            transform.rotation = Quat::from_rotation_x(-0.38 - push * 0.25);
                        }
                        UnimateClip::PointDeal => transform.rotation = Quat::from_rotation_x(-0.14),
                        UnimateClip::Endorse => transform.rotation = Quat::from_rotation_x(-1.68),
                        UnimateClip::QuantityChin => transform.rotation = Quat::from_rotation_x(-2.38),
                        UnimateClip::PriceSlice => transform.rotation = Quat::from_rotation_x(-1.28),
                        UnimateClip::SpreadCross => transform.rotation = Quat::from_rotation_x(-1.52),
                        UnimateClip::Dispute => {
                            let wave = (t * 12.0).sin() * 0.20;
                            transform.rotation = Quat::from_rotation_x(-1.15 + wave);
                        }
                        UnimateClip::Sprint => transform.rotation = Quat::from_rotation_x(-1.4),
                        UnimateClip::Pulpit => transform.rotation = Quat::from_rotation_x(-1.1),
                    },
                    JointType::LeftWrist => match trader.clip {
                        UnimateClip::Idle => {
                            transform.rotation = Quat::from_euler(EulerRot::XYZ, 0.0, std::f32::consts::FRAC_PI_2, 0.0);
                        }
                        UnimateClip::Buy => {
                            let wave_fast = (t * 22.0).sin() * 0.06;
                            let wave = (t * 12.0).sin() * 0.12;
                            transform.rotation = Quat::from_euler(EulerRot::XYZ, -0.50 + wave_fast, -0.34, wave * 0.25);
                        }
                        UnimateClip::Sell => {
                            let push = (t * 9.0).sin() * 0.12;
                            transform.rotation = Quat::from_euler(EulerRot::XYZ, -1.35, 2.85, -push * 0.15);
                        }
                        UnimateClip::PointDeal => {
                            transform.rotation = Quat::from_euler(EulerRot::XYZ, -0.22, 0.42, 0.0);
                        }
                        UnimateClip::Endorse => {
                            transform.rotation = Quat::from_euler(EulerRot::XYZ, -0.32, 0.62, 0.0);
                        }
                        UnimateClip::QuantityChin => {
                            transform.rotation = Quat::from_euler(EulerRot::XYZ, 0.0, 0.52, 0.0);
                        }
                        UnimateClip::PriceSlice => {
                            transform.rotation = Quat::IDENTITY;
                        }
                        UnimateClip::SpreadCross => {
                            transform.rotation = Quat::from_euler(EulerRot::XYZ, -0.32, 0.72, 0.0);
                        }
                        UnimateClip::Dispute => {
                            transform.rotation = Quat::from_euler(EulerRot::XYZ, -0.82, 0.0, 0.0);
                        }
                        UnimateClip::Sprint => transform.rotation = Quat::IDENTITY,
                        UnimateClip::Pulpit => transform.rotation = Quat::from_euler(EulerRot::XYZ, -0.3, 0.0, 0.0),
                    },
                    JointType::RightWrist => match trader.clip {
                        UnimateClip::Idle => {
                            transform.rotation = Quat::from_euler(EulerRot::XYZ, 0.0, -std::f32::consts::FRAC_PI_2, 0.0);
                        }
                        UnimateClip::Buy => {
                            let wave_fast = (t * 22.0).sin() * 0.06;
                            let wave = (t * 12.0).sin() * 0.12;
                            transform.rotation = Quat::from_euler(EulerRot::XYZ, -0.50 - wave_fast, 0.34, -wave * 0.25);
                        }
                        UnimateClip::Sell => {
                            let push = (t * 9.0).sin() * 0.12;
                            transform.rotation = Quat::from_euler(EulerRot::XYZ, -1.35, -2.85, push * 0.15);
                        }
                        UnimateClip::PointDeal => {
                            transform.rotation = Quat::IDENTITY;
                        }
                        UnimateClip::Endorse => {
                            let scribble = (t * 26.0).sin() * 0.22;
                            transform.rotation = Quat::from_euler(EulerRot::XYZ, -0.42 + scribble, -0.32, scribble * 0.5);
                        }
                        UnimateClip::QuantityChin => {
                            transform.rotation = Quat::from_euler(EulerRot::XYZ, -0.48, -0.22, 0.12);
                        }
                        UnimateClip::PriceSlice => {
                            transform.rotation = Quat::from_euler(EulerRot::XYZ, -1.22, -1.57, 0.0);
                        }
                        UnimateClip::SpreadCross => {
                            transform.rotation = Quat::from_euler(EulerRot::XYZ, -0.32, -0.72, 0.0);
                        }
                        UnimateClip::Dispute => {
                            let flick = (t * 15.0).sin() * 0.35;
                            transform.rotation = Quat::from_euler(EulerRot::XYZ, -0.32 + flick, 0.0, flick * 0.4);
                        }
                        UnimateClip::Sprint => transform.rotation = Quat::IDENTITY,
                        UnimateClip::Pulpit => transform.rotation = Quat::from_euler(EulerRot::XYZ, -0.3, 0.0, 0.0),
                    },
                    JointType::LeftFingers | JointType::RightFingers => match trader.clip {
                        UnimateClip::Idle => transform.rotation = Quat::from_rotation_x(-0.25),
                        UnimateClip::Buy => transform.rotation = Quat::from_rotation_x(-0.10),
                        UnimateClip::Sell => transform.rotation = Quat::from_rotation_x(0.05),
                        UnimateClip::PointDeal => transform.rotation = Quat::from_rotation_x(0.0),
                        UnimateClip::Endorse => transform.rotation = Quat::from_rotation_x(-0.90),
                        UnimateClip::QuantityChin => transform.rotation = Quat::from_rotation_x(-1.10),
                        UnimateClip::PriceSlice => transform.rotation = Quat::from_rotation_x(0.0),
                        UnimateClip::SpreadCross => transform.rotation = Quat::from_rotation_x(-0.60),
                        UnimateClip::Dispute => transform.rotation = Quat::from_rotation_x(-0.90),
                        UnimateClip::Sprint => transform.rotation = Quat::from_rotation_x(-1.0),
                        UnimateClip::Pulpit => transform.rotation = Quat::from_rotation_x((t * 8.0).sin() * 0.15),
                    },
                    JointType::LeftThumb => match trader.clip {
                        UnimateClip::Idle => transform.rotation = Quat::from_euler(EulerRot::XYZ, -0.15, 0.2, 0.0),
                        UnimateClip::Buy => transform.rotation = Quat::from_euler(EulerRot::XYZ, -0.3, 0.35, 0.0),
                        UnimateClip::Sell => transform.rotation = Quat::from_euler(EulerRot::XYZ, -0.2, 0.2, 0.0),
                        UnimateClip::Endorse => transform.rotation = Quat::from_euler(EulerRot::XYZ, -0.4, 0.4, 0.0),
                        _ => transform.rotation = Quat::IDENTITY,
                    },
                    JointType::RightThumb => match trader.clip {
                        UnimateClip::Idle => transform.rotation = Quat::from_euler(EulerRot::XYZ, -0.15, -0.2, 0.0),
                        UnimateClip::Buy => transform.rotation = Quat::from_euler(EulerRot::XYZ, -0.3, -0.35, 0.0),
                        UnimateClip::Sell => transform.rotation = Quat::from_euler(EulerRot::XYZ, -0.2, -0.2, 0.0),
                        UnimateClip::PointDeal => transform.rotation = Quat::from_euler(EulerRot::XYZ, -0.35, -0.2, 0.0),
                        UnimateClip::Endorse => transform.rotation = Quat::from_euler(EulerRot::XYZ, -0.45, -0.35, 0.0),
                        _ => transform.rotation = Quat::IDENTITY,
                    },
                    JointType::LeftTicket | JointType::RightTicket => {
                        if let Some(mut vis) = opt_vis {
                            match trader.clip {
                                UnimateClip::Buy | UnimateClip::Sell | UnimateClip::PointDeal | UnimateClip::Endorse | UnimateClip::QuantityChin | UnimateClip::PriceSlice => {
                                    *vis = Visibility::Inherited;
                                }
                                _ => *vis = Visibility::Hidden,
                            }
                        }
                        if let Some(mut mat) = opt_mat {
                            match trader.clip {
                                UnimateClip::Buy | UnimateClip::PointDeal | UnimateClip::Endorse | UnimateClip::QuantityChin => {
                                    *mat = MeshMaterial3d(mats.ticket_buy_white.clone());
                                }
                                UnimateClip::Sell | UnimateClip::PriceSlice | UnimateClip::Dispute => {
                                    *mat = MeshMaterial3d(mats.ticket_sell_pink.clone());
                                }
                                _ => {}
                            }
                        }
                    }
                    JointType::LeftPalmLight | JointType::RightPalmLight => {
                        if let Some(mut light) = opt_light {
                            match trader.clip {
                                UnimateClip::Buy => {
                                    light.color = Color::srgb(0.13, 0.77, 0.37);
                                    light.intensity = 150000.0 + (t * 12.0).sin() * 50000.0;
                                }
                                UnimateClip::Sell => {
                                    light.color = Color::srgb(0.94, 0.27, 0.27);
                                    light.intensity = 150000.0 + (t * 10.0).sin() * 50000.0;
                                }
                                UnimateClip::PointDeal => {
                                    light.color = Color::srgb(0.98, 0.75, 0.15);
                                    light.intensity = 200000.0;
                                }
                                _ => {
                                    light.intensity = 0.0;
                                }
                            }
                        }
                    }
                    JointType::LeftHip => match trader.clip {
                        UnimateClip::Idle => {
                            let weight_shift = (t * 1.2).sin();
                            transform.rotation = Quat::from_euler(EulerRot::XYZ, -weight_shift * 0.12, 0.0, -0.04);
                        }
                        UnimateClip::Buy => transform.rotation = Quat::from_euler(EulerRot::XYZ, 0.2, 0.0, -0.15),
                        UnimateClip::Sell => transform.rotation = Quat::from_euler(EulerRot::XYZ, -0.25, 0.0, -0.35),
                        UnimateClip::PointDeal => transform.rotation = Quat::from_euler(EulerRot::XYZ, -0.05, 0.0, -0.05),
                        UnimateClip::Endorse => transform.rotation = Quat::from_euler(EulerRot::XYZ, 0.08, 0.0, -0.04),
                        UnimateClip::QuantityChin => transform.rotation = Quat::IDENTITY,
                        UnimateClip::PriceSlice => transform.rotation = Quat::IDENTITY,
                        UnimateClip::SpreadCross => transform.rotation = Quat::IDENTITY,
                        UnimateClip::Dispute => transform.rotation = Quat::from_euler(EulerRot::XYZ, 0.25, 0.0, -0.10),
                        UnimateClip::Sprint => {
                            let stride = (t * 10.0).sin();
                            transform.rotation = Quat::from_rotation_x(stride * 0.8);
                        }
                        UnimateClip::Pulpit => transform.rotation = Quat::IDENTITY,
                    },
                    JointType::RightHip => match trader.clip {
                        UnimateClip::Idle => {
                            let weight_shift = (t * 1.2).sin();
                            transform.rotation = Quat::from_euler(EulerRot::XYZ, weight_shift * 0.12, 0.0, 0.04);
                        }
                        UnimateClip::Buy => transform.rotation = Quat::from_euler(EulerRot::XYZ, -0.35, 0.0, 0.15),
                        UnimateClip::Sell => transform.rotation = Quat::from_euler(EulerRot::XYZ, 0.25, 0.0, 0.35),
                        UnimateClip::PointDeal => transform.rotation = Quat::from_euler(EulerRot::XYZ, 0.05, 0.0, 0.05),
                        UnimateClip::Endorse => transform.rotation = Quat::from_euler(EulerRot::XYZ, -0.08, 0.0, 0.04),
                        UnimateClip::QuantityChin => transform.rotation = Quat::IDENTITY,
                        UnimateClip::PriceSlice => transform.rotation = Quat::IDENTITY,
                        UnimateClip::SpreadCross => transform.rotation = Quat::IDENTITY,
                        UnimateClip::Dispute => transform.rotation = Quat::from_euler(EulerRot::XYZ, -0.20, 0.0, 0.10),
                        UnimateClip::Sprint => {
                            let stride = (t * 10.0).sin();
                            transform.rotation = Quat::from_rotation_x(-stride * 0.8);
                        }
                        UnimateClip::Pulpit => transform.rotation = Quat::IDENTITY,
                    },
                    JointType::LeftKnee => match trader.clip {
                        UnimateClip::Idle => {
                            let weight_shift = (t * 1.2).sin();
                            let flex = (weight_shift * 0.15).max(0.0);
                            transform.rotation = Quat::from_rotation_x(0.04 + flex);
                        }
                        UnimateClip::Buy => transform.rotation = Quat::from_rotation_x(0.35),
                        UnimateClip::Sell => transform.rotation = Quat::from_rotation_x(0.20),
                        UnimateClip::PointDeal => transform.rotation = Quat::from_rotation_x(0.12),
                        UnimateClip::Endorse => transform.rotation = Quat::from_rotation_x(0.18),
                        UnimateClip::QuantityChin => transform.rotation = Quat::from_rotation_x(0.05),
                        UnimateClip::PriceSlice => transform.rotation = Quat::from_rotation_x(0.10),
                        UnimateClip::SpreadCross => transform.rotation = Quat::from_rotation_x(0.10),
                        UnimateClip::Dispute => transform.rotation = Quat::from_rotation_x(0.38),
                        UnimateClip::Sprint => {
                            let stride = (t * 10.0).sin();
                            transform.rotation = Quat::from_rotation_x((stride * 0.8).max(0.0) * 1.2);
                        }
                        UnimateClip::Pulpit => transform.rotation = Quat::IDENTITY,
                    },
                    JointType::RightKnee => match trader.clip {
                        UnimateClip::Idle => {
                            let weight_shift = (t * 1.2).sin();
                            let flex = (-weight_shift * 0.15).max(0.0);
                            transform.rotation = Quat::from_rotation_x(0.04 + flex);
                        }
                        UnimateClip::Buy => transform.rotation = Quat::from_rotation_x(0.10),
                        UnimateClip::Sell => transform.rotation = Quat::from_rotation_x(0.30),
                        UnimateClip::PointDeal => transform.rotation = Quat::from_rotation_x(0.12),
                        UnimateClip::Endorse => transform.rotation = Quat::from_rotation_x(0.18),
                        UnimateClip::QuantityChin => transform.rotation = Quat::from_rotation_x(0.05),
                        UnimateClip::PriceSlice => transform.rotation = Quat::from_rotation_x(0.10),
                        UnimateClip::SpreadCross => transform.rotation = Quat::from_rotation_x(0.10),
                        UnimateClip::Dispute => transform.rotation = Quat::from_rotation_x(0.12),
                        UnimateClip::Sprint => {
                            let stride = (t * 10.0).sin();
                            transform.rotation = Quat::from_rotation_x((-stride * 0.8).max(0.0) * 1.2);
                        }
                        UnimateClip::Pulpit => transform.rotation = Quat::IDENTITY,
                    }
                }

                if let Some(children) = opt_children {
                    stack.extend(children.iter());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_demographic_distribution() {
        let mut white_count = 0;
        let mut black_count = 0;
        let mut brown_count = 0;

        for seed in 0..1000 {
            let (eth, _, _) = sample_demographics(seed);
            match eth {
                Ethnicity::White => white_count += 1,
                Ethnicity::Black => black_count += 1,
                Ethnicity::Brown => brown_count += 1,
            }
        }

        // Exact 80% / 10% / 10% expectation across mod 100
        assert_eq!(white_count, 800);
        assert_eq!(black_count, 100);
        assert_eq!(brown_count, 100);
    }

    #[test]
    fn test_procedural_striped_image() {
        let col_a = Color::srgb(1.0, 0.0, 0.0);
        let col_b = Color::srgb(1.0, 1.0, 1.0);
        let img = generate_striped_image(col_a, col_b, 8, 128, 128);

        assert_eq!(img.texture_descriptor.size.width, 128);
        assert_eq!(img.texture_descriptor.size.height, 128);

        let data = img.data.as_ref().expect("Image should contain pixel data");
        assert_eq!(data.len(), 128 * 128 * 4);

        // Check first stripe pixel (should be Red: 255, 0, 0, 255)
        assert_eq!(data[0], 255);
        assert_eq!(data[1], 0);
        assert_eq!(data[2], 0);
        assert_eq!(data[3], 255);

        // Check second stripe pixel around x = 20 (stripe width is 128/8 = 16px, so x=20 is stripe 1 = White)
        let idx = (20 * 4) as usize;
        assert_eq!(data[idx], 255);
        assert_eq!(data[idx + 1], 255);
        assert_eq!(data[idx + 2], 255);
        assert_eq!(data[idx + 3], 255);
    }

    #[test]
    fn test_cme_jacket_configurations() {
        for &style in &JacketStyle::ALL_STRIPED {
            assert!(style.is_striped());
            let (c_a, c_b, num) = style.colors();
            assert_eq!(num, 8);
            assert_ne!(c_a, c_b);
        }

        assert!(!JacketStyle::BrokerBlue.is_striped());
        assert!(!JacketStyle::ScalperGreen.is_striped());
        assert!(!JacketStyle::RunnerGold.is_striped());
        assert!(!JacketStyle::ReporterBurgundy.is_striped());
    }

    #[test]
    fn test_unimate_clip_variants() {
        assert_eq!(UnimateClip::default(), UnimateClip::Idle);

        let clips = [
            UnimateClip::Idle,
            UnimateClip::Buy,
            UnimateClip::Sell,
            UnimateClip::PointDeal,
            UnimateClip::Endorse,
            UnimateClip::QuantityChin,
            UnimateClip::PriceSlice,
            UnimateClip::SpreadCross,
            UnimateClip::Dispute,
            UnimateClip::Sprint,
            UnimateClip::Pulpit,
        ];
        assert_eq!(clips.len(), 11);

        for (i, c1) in clips.iter().enumerate() {
            for (j, c2) in clips.iter().enumerate() {
                if i == j {
                    assert_eq!(c1, c2);
                } else {
                    assert_ne!(c1, c2);
                }
            }
        }
    }
}
