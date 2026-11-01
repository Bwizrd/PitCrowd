use bevy::prelude::*;
use std::collections::HashMap;
use crate::market::{Firm, Instrument};
use crate::pit::{Pit, PitBounds};
use crate::camera::FloorViewMode;
use crate::pit_engine::{role_for_slot, ParticipantRole, Phase, PitEngines, PitEventKind, PitMarket, Side};

/// Concourse waypoints connecting all trading districts across the shared floor
pub const CONCOURSE_WAYPOINTS: [Vec2; 14] = [
    Vec2::new(0.0, 380.0),     // North Concourse between Gold & Silver
    Vec2::new(720.0, 180.0),   // North-East Concourse above US30
    Vec2::new(1080.0, -390.0), // East Concourse between US30 & GBP
    Vec2::new(750.0, -580.0),  // South-East Concourse between BTC & GBP
    Vec2::new(0.0, -540.0),    // South Grand Plaza between EUR & BTC
    Vec2::new(-750.0, -580.0), // South-West Concourse between UK100 & EUR
    Vec2::new(-1080.0, -390.0),// West Concourse between Nasdaq & UK100
    Vec2::new(-720.0, 180.0),  // North-West Concourse above Nasdaq
    Vec2::new(0.0, 200.0),     // Center Hub North
    Vec2::new(480.0, 0.0),     // Center Hub East
    Vec2::new(0.0, -200.0),    // Center Hub South
    Vec2::new(-480.0, 0.0),    // Center Hub West
    Vec2::new(-350.0, -250.0), // South-West Inner Aisle
    Vec2::new(350.0, -250.0),  // South-East Inner Aisle
];

/// Visual trader entity participating in pit order flow
#[derive(Component, Debug, Clone)]
pub struct Trader {
    pub instrument: Instrument,
    pub firm: Firm,
    pub pit_entity: Entity,
}

/// Which engine slot (0..18) this pit trader renders. Shared numbering with the 3D crowd.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct PitSlot(pub usize);

/// Pure helper: the 2D role that draws an engine role.
pub fn trader_role_for(role: ParticipantRole) -> TraderRole {
    match role {
        ParticipantRole::Reporter => TraderRole::PriceReporter,
        ParticipantRole::LeadBroker | ParticipantRole::Broker => TraderRole::FloorBroker,
        ParticipantRole::Local | ParticipantRole::Runner => TraderRole::LocalScalper,
    }
}

/// Dynamic velocity vector of a trader
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct Velocity(pub Vec2);

/// Movement waypoint steering logic with directional phalanx targeting
#[derive(Component, Debug, Clone)]
pub struct TraderMovement {
    pub target: Vec2,
    pub base_speed: f32,
    pub wait_timer: f32,
    pub facing_x: f32,
    pub is_runner: bool,
    pub runner_waypoint_idx: usize,
}

/// Open-outcry hand gesture and trade execution flash state
#[derive(Component, Debug, Clone)]
pub struct TraderGesture {
    pub flash_timer: f32,
    pub shout_cooldown: f32,
    pub is_buying: bool,
}

impl Default for TraderGesture {
    fn default() -> Self {
        Self {
            flash_timer: 0.0,
            shout_cooldown: 0.0,
            is_buying: true,
        }
    }
}

/// Marker component for trader drop shadow on floorboards
#[derive(Component)]
pub struct TraderShadowVisual;

/// Marker component for trader left or right articulated leg
#[derive(Component)]
pub struct TraderLegVisual {
    pub is_left: bool,
}

/// Marker component for trader shoes
#[derive(Component)]
pub struct TraderShoeVisual {
    pub is_left: bool,
}

/// Marker component for trader torso / firm jacket
#[derive(Component)]
pub struct TraderJacketVisual;

/// Marker component for trader shirt collar / lapel
#[derive(Component)]
pub struct TraderCollarVisual;

/// Marker component for trader necktie / execution badge
#[derive(Component)]
pub struct TraderTieVisual;

/// Marker component for trader head
#[derive(Component)]
pub struct TraderHeadVisual;

/// Marker component for trader hair cap
#[derive(Component)]
pub struct TraderHairVisual;

/// Marker component for trader retro sunglasses / eye visor
#[derive(Component)]
pub struct TraderSunglassesVisual;

/// Marker component for trader arms and open-cry gestures
#[derive(Component)]
pub struct TraderArmVisual {
    pub is_left: bool,
}

/// Marker component for paper buy/sell order tickets held in hand
#[derive(Component, Debug, Clone)]
pub struct TraderOrderSlipVisual {
    pub instrument: Instrument,
    pub is_left: bool,
    pub trader_entity: Entity,
}

/// Floating retro trade shout popup (Theme Park / The Sims style)
#[derive(Component, Debug, Clone)]
pub struct TradeShoutPopup {
    pub timer: f32,
    pub max_lifetime: f32,
    pub velocity_y: f32,
}

/// The specialized operational role of a participant on the trading floor
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraderRole {
    /// Exchange Official / Price Reporter stationed on the central pulpit logging prints
    PriceReporter,
    /// Lead Floor Broker executing institutional orders with hand signals
    FloorBroker,
    /// Independent local scalpers providing two-way liquidity around the pit
    LocalScalper,
    /// Floor runner moving tickets between pits along the concourses
    FloorRunner,
    /// Concourse Lounger outside pits resting, chatting, or awaiting order flow surges
    ConcourseLounger,
}

impl TraderRole {
    pub fn label(&self) -> &'static str {
        match self {
            TraderRole::PriceReporter => "Price Reporter (Exchange)",
            TraderRole::FloorBroker => "Lead Floor Broker (Institutional)",
            TraderRole::LocalScalper => "Local Scalper (Market Maker)",
            TraderRole::FloorRunner => "Floor Runner (Concourse)",
            TraderRole::ConcourseLounger => "Concourse Lounger (Resting/Chatting)",
        }
    }
}

/// Marker component for trader open-outcry articulated hands and palms (on EVERY trader)
#[derive(Component, Debug, Clone)]
pub struct TraderHandVisual {
    pub instrument: Instrument,
    pub role: TraderRole,
    pub is_left: bool,
    pub trader_entity: Entity,
}

/// Marker component for inner palm and finger directional facet
#[derive(Component, Debug, Clone)]
pub struct TraderHandFingerVisual {
    pub instrument: Instrument,
    pub is_left: bool,
    pub trader_entity: Entity,
}

/// Marker component for glowing hand signal palm aura during active trade impulses
#[derive(Component, Debug, Clone)]
pub struct TraderHandAuraVisual {
    pub instrument: Instrument,
    pub is_left: bool,
    pub trader_entity: Entity,
}

/// Marker component for dark contrast rim/outline behind hand
#[derive(Component, Debug, Clone)]
pub struct TraderHandOutlineVisual {
    pub is_left: bool,
}

/// Legacy marker component for broker's hand signal
#[derive(Component)]
pub struct BrokerHandSignalVisual {
    pub is_left: bool,
}

/// Pure helper: assigned step position for an engine slot on the octagonal amphitheater tiers.
/// Slot 0 is the lead broker on the top rail, slot 1 the reporter on the pulpit,
/// slots 2..=9 the middle step and 10..=17 the outer step, one per facet.
pub fn compute_tier_step_position(bounds: &PitBounds, index: usize) -> Vec2 {
    match index {
        0 => bounds.center + Vec2::new(bounds.width * 0.18, bounds.height * 0.28),
        1 => bounds.center + Vec2::new(0.0, 3.5),
        _ => {
            let (facet, radius_w, radius_h) = if index < 10 {
                (index - 2, bounds.width * 0.32, bounds.height * 0.30)
            } else {
                (index - 10, bounds.width * 0.39, bounds.height * 0.36)
            };
            let angle = facet as f32 * std::f32::consts::FRAC_PI_4 + std::f32::consts::FRAC_PI_8;
            let offset = Vec2::new(radius_w * angle.cos(), radius_h * angle.sin());
            bounds.clamp_point(bounds.center + offset, 16.0)
        }
    }
}

/// Marker component for broker's distinctive order deck / clipboard
#[derive(Component)]
pub struct BrokerOrderDeckVisual;

/// Marker component for broker's floating overhead palm signal badge chassis
#[derive(Component)]
pub struct BrokerPalmBadgeVisual;

/// Marker component for broker's floating overhead palm signal badge text
#[derive(Component)]
pub struct BrokerPalmBadgeText;

/// Component indicating whether this agent is the specialist market maker anchored on the center podium
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct IsMarketMaker(pub bool);

/// Pure function: Maps normalized activity [0.0, 1.0] to visual agent population
pub fn activity_to_agent_count(activity: f32, min_agents: usize, max_agents: usize) -> usize {
    let clamped = activity.clamp(0.0, 1.0);
    let span = (max_agents - min_agents) as f32;
    // Slight quadratic curve so high excitement really swells the pit crowd
    let curve = clamped * clamped * 0.5 + clamped * 0.5;
    min_agents + (span * curve).round() as usize
}

/// Pure function: Maps normalized activity [0.0, 1.0] to trader movement speed in pixels/sec
pub fn activity_to_speed(activity: f32, min_speed: f32, max_speed: f32) -> f32 {
    let clamped = activity.clamp(0.0, 1.0);
    min_speed + (max_speed - min_speed) * clamped
}

/// Pure deterministic pseudo-random helper for unit testing & waypoint picking without external crates
pub fn pseudo_random_offset(seed: u64, max_radius_x: f32, max_radius_y: f32) -> Vec2 {
    let next_x = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    let next_y = next_x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    let norm_x = ((next_x >> 32) as u32 as f32 / u32::MAX as f32) * 2.0 - 1.0;
    let norm_y = ((next_y >> 32) as u32 as f32 / u32::MAX as f32) * 2.0 - 1.0;
    Vec2::new(norm_x * max_radius_x, norm_y * max_radius_y)
}

/// Pure function: Computes directional phalanx target based on market delta aggression
///
/// In OpenCry isometric pits:
/// - Offer Rail (Asking / Selling price): Top-Right quadrant (+X, +Y)
/// - Bid Rail (Bidding / Buying price): Bottom-Left quadrant (-X, -Y)
/// - Neutral / Chop: Centered dispersion
pub fn compute_phalanx_target(
    bounds: &PitBounds,
    delta: f32,
    seed: u64,
    is_market_maker: bool,
) -> Vec2 {
    let clamped_delta = delta.clamp(-1.0, 1.0);
    let offer_rail_bias = Vec2::new(bounds.width * 0.28, bounds.height * 0.25);
    let bid_rail_bias = Vec2::new(-bounds.width * 0.28, -bounds.height * 0.25);

    let cluster_center = if is_market_maker {
        // Market makers anchor near center balancing both rails
        bounds.center + Vec2::new(0.0, 2.0)
    } else if clamped_delta > 0.25 {
        // Aggressive buyers form a unified phalanx surging toward the Offer rail
        let weight = ((clamped_delta - 0.25) / 0.75).clamp(0.0, 1.0);
        bounds.center + offer_rail_bias * weight
    } else if clamped_delta < -0.25 {
        // Aggressive sellers surge down toward the Bid rail
        let weight = ((-clamped_delta - 0.25) / 0.75).clamp(0.0, 1.0);
        bounds.center + bid_rail_bias * weight
    } else {
        bounds.center
    };

    if is_market_maker {
        return cluster_center;
    }

    // Add randomized spread around the cluster center for floor traders
    let spread_factor = (1.0 - clamped_delta.abs() * 0.45).clamp(0.40, 1.0);
    let offset = pseudo_random_offset(
        seed,
        bounds.width * 0.34 * spread_factor,
        bounds.height * 0.30 * spread_factor,
    );

    bounds.clamp_point(cluster_center + offset, 12.0)
}

/// System: Dynamically reconciles pit trader count with market activity, and maintains
/// floor runners moving across the shared aisles between pits
pub fn sync_crowd_density(
    mut commands: Commands,
    engines: Res<PitEngines>,
    pits: Query<(Entity, &Pit, &PitBounds)>,
    traders_query: Query<(Entity, &Trader, &TraderMovement, Option<&TraderRole>)>,
) {
    let mut pit_traders: std::collections::HashMap<Entity, Vec<Entity>> =
        std::collections::HashMap::new();
    let mut runner_count = 0;
    let mut lounger_count = 0;

    for (trader_entity, trader, movement, role) in traders_query.iter() {
        if movement.is_runner {
            runner_count += 1;
        } else if role == Some(&TraderRole::ConcourseLounger) {
            lounger_count += 1;
        } else {
            pit_traders
                .entry(trader.pit_entity)
                .or_default()
                .push(trader_entity);
        }
    }

    let mut seed = (engines.pits[0].clock * 1000.0) as u64;

    // 1. Reconcile pit traders with the engine's headcount, slot for slot
    for (pit_entity, pit, bounds) in pits.iter() {
        let target_count = engines.pit(pit.instrument).active_count.clamp(3, 18);

        let current_traders = pit_traders.entry(pit_entity).or_default();
        let current_count = current_traders.len();

        if current_count < target_count {
            let to_spawn = (target_count - current_count).min(4);

            for i in 0..to_spawn {
                seed = seed.wrapping_add(i as u64 + 1);
                let idx = current_traders.len();
                let role = trader_role_for(role_for_slot(idx));

                let is_mm = role == TraderRole::PriceReporter;
                let firm = match role {
                    TraderRole::PriceReporter => Firm::Apex,
                    TraderRole::FloorBroker => Firm::Apex,
                    _ => Firm::ALL[(seed as usize) % Firm::ALL.len()],
                };

                // Compute authentic assigned step position on the amphitheater tiers
                let step_spot = compute_tier_step_position(bounds, idx);
                let zd = if role == TraderRole::PriceReporter {
                    2.5
                } else {
                    1.25 + (600.0 - step_spot.y) * 0.001
                };

                let (spawn_pos, target_pos, speed, z_depth, coat_color, collar_color, tie_color, hair_color) = match role {
                    TraderRole::PriceReporter => (
                        step_spot,
                        step_spot,
                        0.0,
                        2.5,
                        Color::srgb(0.55, 0.12, 0.18), // Exchange Burgundy
                        Color::srgb(0.96, 0.96, 0.98), // White collar
                        Color::srgb(0.96, 0.82, 0.22), // Gold official medallion
                        Color::srgb(0.18, 0.12, 0.08),
                    ),
                    TraderRole::FloorBroker => (
                        step_spot,
                        step_spot,
                        0.0,
                        zd,
                        Color::srgb(0.10, 0.25, 0.75), // Royal Blue Blazer
                        Color::srgb(0.96, 0.82, 0.22), // Gold Trim Lapels
                        Color::srgb(1.0, 0.88, 0.28), // Bright Gold Execution Badge
                        Color::srgb(0.12, 0.12, 0.14),
                    ),
                    TraderRole::LocalScalper => {
                        let tie_colors = [
                            Color::srgb(0.88, 0.15, 0.18),
                            Color::srgb(0.15, 0.35, 0.88),
                            Color::srgb(0.88, 0.72, 0.15),
                            Color::srgb(0.18, 0.68, 0.35),
                        ];
                        let hair_colors = [
                            Color::srgb(0.18, 0.12, 0.08),
                            Color::srgb(0.08, 0.08, 0.10),
                            Color::srgb(0.85, 0.70, 0.30),
                            Color::srgb(0.65, 0.26, 0.12),
                        ];
                        (
                            step_spot,
                            step_spot,
                            0.0,
                            zd,
                            firm.color(),
                            Color::srgb(0.96, 0.96, 0.98),
                            tie_colors[(seed as usize) % tie_colors.len()],
                            hair_colors[(seed as usize) % hair_colors.len()],
                        )
                    },
                    _ => unreachable!(),
                };

                let trader_id = spawn_avatar_entity(
                    &mut commands,
                    pit.instrument,
                    firm,
                    pit_entity,
                    spawn_pos,
                    target_pos,
                    speed,
                    z_depth,
                    role,
                    is_mm,
                    false,
                    0,
                    coat_color,
                    collar_color,
                    hair_color,
                    tie_color,
                );
                commands.entity(trader_id).insert(PitSlot(idx));

                current_traders.push(trader_id);
            }
        } else if current_count > target_count && current_count > 3 {
            // Keep at least the 3 core floor officers during quiet periods
            if let Some(despawn_entity) = current_traders.pop() {
                commands.entity(despawn_entity).despawn();
            }
        }
    }

    // 2. Maintain Floor Runners (jogging between pits along the concourses)
    let target_runners = 8;
    if runner_count < target_runners {
        if let Some((first_pit_entity, first_pit, _)) = pits.iter().next() {
            let wp_idx = runner_count % CONCOURSE_WAYPOINTS.len();
            let start_pos = CONCOURSE_WAYPOINTS[wp_idx];
            let next_wp_idx = (wp_idx + 1) % CONCOURSE_WAYPOINTS.len();
            let target_pos = CONCOURSE_WAYPOINTS[next_wp_idx];

            let firm = Firm::ALL[runner_count % Firm::ALL.len()];
            let z_depth = 1.2 + (600.0 - start_pos.y) * 0.001;

            spawn_avatar_entity(
                &mut commands,
                first_pit.instrument,
                firm,
                first_pit_entity,
                start_pos,
                target_pos,
                92.0,
                z_depth,
                TraderRole::FloorRunner,
                false,
                true,
                next_wp_idx,
                firm.color(),
                Color::srgb(0.96, 0.96, 0.98),
                Color::srgb(0.15, 0.10, 0.08),
                Color::srgb(0.85, 0.20, 0.20),
            );
        }
    }

    // 3. Maintain Concourse Loungers (pairs of traders resting/chatting on concourses outside pits)
    let target_loungers = 12;
    if lounger_count < target_loungers {
        if let Some((first_pit_entity, first_pit, _)) = pits.iter().next() {
            let wp_idx = (lounger_count / 2) % CONCOURSE_WAYPOINTS.len();
            let base_wp = CONCOURSE_WAYPOINTS[wp_idx];
            let is_second = lounger_count % 2 == 1;
            let offset = if is_second { Vec2::new(14.0, 0.0) } else { Vec2::new(-14.0, 0.0) };
            let pos = base_wp + offset;
            let firm = Firm::ALL[(lounger_count * 2) % Firm::ALL.len()];
            let z_depth = 1.1 + (600.0 - pos.y) * 0.001;

            spawn_avatar_entity(
                &mut commands,
                first_pit.instrument,
                firm,
                first_pit_entity,
                pos,
                pos,
                0.0,
                z_depth,
                TraderRole::ConcourseLounger,
                false,
                false,
                0,
                firm.color(),
                Color::srgb(0.96, 0.96, 0.98),
                Color::srgb(0.18, 0.12, 0.08),
                Color::srgb(0.20, 0.55, 0.85),
            );
        }
    }
}

/// Helper function to spawn the multi-part Theme Park avatar hierarchy
fn spawn_avatar_entity(
    commands: &mut Commands,
    instrument: Instrument,
    firm: Firm,
    pit_entity: Entity,
    spawn_pos: Vec2,
    target_pos: Vec2,
    speed: f32,
    z_depth: f32,
    role: TraderRole,
    is_mm: bool,
    is_runner: bool,
    runner_waypoint_idx: usize,
    coat_color: Color,
    collar_color: Color,
    hair_color: Color,
    tie_color: Color,
) -> Entity {
    let trader_id = commands
        .spawn((
            Trader {
                instrument,
                firm,
                pit_entity,
            },
            role,
            Velocity(Vec2::ZERO),
            TraderMovement {
                target: target_pos,
                base_speed: speed,
                wait_timer: if is_mm { 999999.0 } else { 0.0 },
                facing_x: 1.0,
                is_runner,
                runner_waypoint_idx,
            },
            TraderGesture::default(),
            IsMarketMaker(is_mm),
            Transform::from_xyz(spawn_pos.x, spawn_pos.y, z_depth),
            Visibility::default(),
        ))
        .id();

    commands.entity(trader_id).with_children(|parent| {
            // 1. Soft Elliptical Drop Shadow on floorboards
            parent.spawn((
                TraderShadowVisual,
                Sprite {
                    color: Color::srgba(0.0, 0.0, 0.0, 0.38),
                    custom_size: Some(Vec2::new(26.0, 10.0)),
                    ..default()
                },
                Transform::from_xyz(0.0, -15.0, -0.01),
            ));

            // 2. Left Leg & Shoe
            parent.spawn((
                TraderLegVisual { is_left: true },
                Sprite {
                    color: Color::srgb(0.12, 0.14, 0.20),
                    custom_size: Some(Vec2::new(5.5, 12.0)),
                    ..default()
                },
                Transform::from_xyz(-4.5, -7.5, 0.01),
            )).with_children(|leg| {
                leg.spawn((
                    TraderShoeVisual { is_left: true },
                    Sprite {
                        color: Color::srgb(0.06, 0.05, 0.05),
                        custom_size: Some(Vec2::new(6.5, 4.0)),
                        ..default()
                    },
                    Transform::from_xyz(0.0, -6.5, 0.005),
                ));
            });

            // 3. Right Leg & Shoe
            parent.spawn((
                TraderLegVisual { is_left: false },
                Sprite {
                    color: Color::srgb(0.12, 0.14, 0.20),
                    custom_size: Some(Vec2::new(5.5, 12.0)),
                    ..default()
                },
                Transform::from_xyz(4.5, -7.5, 0.01),
            )).with_children(|leg| {
                leg.spawn((
                    TraderShoeVisual { is_left: false },
                    Sprite {
                        color: Color::srgb(0.06, 0.05, 0.05),
                        custom_size: Some(Vec2::new(6.5, 4.0)),
                        ..default()
                    },
                    Transform::from_xyz(0.0, -6.5, 0.005),
                ));
            });

            // 4. Torso & Firm / Role Colored Blazer
            parent.spawn((
                TraderJacketVisual,
                Sprite {
                    color: coat_color,
                    custom_size: Some(Vec2::new(22.0, 18.0)),
                    ..default()
                },
                Transform::from_xyz(0.0, 4.5, 0.02),
            )).with_children(|torso| {
                torso.spawn((
                    TraderCollarVisual,
                    Sprite {
                        color: collar_color,
                        custom_size: Some(Vec2::new(8.0, 5.0)),
                        ..default()
                    },
                    Transform::from_xyz(0.0, 5.5, 0.005),
                ));

                // Lead Broker: Distinctive Order Deck / Clipboard
                if role == TraderRole::FloorBroker {
                    torso.spawn((
                        BrokerOrderDeckVisual,
                        Sprite {
                            color: Color::srgb(0.85, 0.72, 0.35),
                            custom_size: Some(Vec2::new(12.0, 15.0)),
                            ..default()
                        },
                        Transform::from_xyz(0.0, -1.5, 0.025),
                    ));
                }
            });

            // 5. Necktie / Execution Badge
            parent.spawn((
                TraderTieVisual,
                Sprite {
                    color: tie_color,
                    custom_size: Some(if role == TraderRole::FloorBroker { Vec2::new(4.5, 8.5) } else { Vec2::new(3.2, 7.5) }),
                    ..default()
                },
                Transform::from_xyz(0.0, 4.5, 0.03),
            ));

            // 6. Chunky Head, Hair & Retro Sunglasses / Visor
            parent.spawn((
                TraderHeadVisual,
                Sprite {
                    color: Color::srgb(0.93, 0.76, 0.62),
                    custom_size: Some(Vec2::new(13.0, 13.0)),
                    ..default()
                },
                Transform::from_xyz(0.0, 18.0, 0.04),
            )).with_children(|head| {
                head.spawn((
                    TraderHairVisual,
                    Sprite {
                        color: hair_color,
                        custom_size: Some(Vec2::new(14.0, 5.0)),
                        ..default()
                    },
                    Transform::from_xyz(0.0, 4.5, 0.005),
                ));

                let visor_color = match role {
                    TraderRole::PriceReporter => Color::srgb(0.18, 0.22, 0.30), // Headset
                    TraderRole::FloorBroker => Color::srgb(0.08, 0.08, 0.12),
                    _ => Color::srgb(0.10, 0.10, 0.13),
                };

                head.spawn((
                    TraderSunglassesVisual,
                    Sprite {
                        color: visor_color,
                        custom_size: Some(Vec2::new(11.0, 3.5)),
                        ..default()
                    },
                    Transform::from_xyz(0.0, 0.5, 0.01),
                ));
            });

            // 7. Left Arm: Shoulder pivot entity with sleeve fabric, palm aura, ticket, and hand hanging down at hip
            parent.spawn((
                TraderArmVisual { is_left: true },
                Transform::from_xyz(-9.5, 10.5, 0.035),
                Visibility::default(),
            )).with_children(|arm| {
                // Sleeve fabric: hangs down from shoulder (0, 0) to wrist (0, -11)
                arm.spawn((
                    Sprite {
                        color: coat_color,
                        custom_size: Some(Vec2::new(5.5, 12.0)),
                        ..default()
                    },
                    Transform::from_xyz(0.0, -5.5, 0.01),
                ));

                // Blooming palm aura (illuminates emerald on buy, crimson on sell)
                arm.spawn((
                    TraderHandAuraVisual { instrument, is_left: true, trader_entity: trader_id },
                    Sprite {
                        color: Color::srgba(0.0, 0.0, 0.0, 0.0),
                        custom_size: Some(Vec2::new(22.0, 22.0)),
                        ..default()
                    },
                    Transform::from_xyz(0.0, -11.0, 0.005),
                ));

                // Color-coded buy (white/cyan) or sell (pink) execution ticket held in hand
                arm.spawn((
                    TraderOrderSlipVisual { instrument, is_left: true, trader_entity: trader_id },
                    Sprite {
                        color: Color::srgb(0.92, 0.96, 1.0),
                        custom_size: Some(Vec2::new(7.0, 11.0)),
                        ..default()
                    },
                    Transform::from_xyz(-2.0, -11.0, 0.015),
                ));

                // High-contrast hand shadow outline
                arm.spawn((
                    TraderHandOutlineVisual { is_left: true },
                    Sprite {
                        color: Color::srgb(0.08, 0.08, 0.10),
                        custom_size: Some(Vec2::new(13.5, 13.5)),
                        ..default()
                    },
                    Transform::from_xyz(0.0, -11.0, 0.02),
                ));

                // Main chunky articulated hand palm (at wrist, pointing down by hip when idle)
                arm.spawn((
                    TraderHandVisual { instrument, role, is_left: true, trader_entity: trader_id },
                    Sprite {
                        color: Color::srgb(0.95, 0.78, 0.62),
                        custom_size: Some(if role == TraderRole::FloorBroker { Vec2::new(13.0, 13.0) } else { Vec2::new(11.0, 11.0) }),
                        ..default()
                    },
                    Transform::from_xyz(0.0, -11.0, 0.025),
                ));

                // Articulated finger pads (at fingertips below palm)
                arm.spawn((
                    TraderHandFingerVisual { instrument, is_left: true, trader_entity: trader_id },
                    Sprite {
                        color: Color::srgb(0.98, 0.88, 0.75),
                        custom_size: Some(Vec2::new(7.5, 5.0)),
                        ..default()
                    },
                    Transform::from_xyz(0.0, -13.5, 0.03),
                ));
            });

            // 8. Right Arm: Shoulder pivot entity with sleeve fabric, palm aura, ticket, and hand hanging down at hip
            parent.spawn((
                TraderArmVisual { is_left: false },
                Transform::from_xyz(9.5, 10.5, 0.035),
                Visibility::default(),
            )).with_children(|arm| {
                // Sleeve fabric: hangs down from shoulder (0, 0) to wrist (0, -11)
                arm.spawn((
                    Sprite {
                        color: coat_color,
                        custom_size: Some(Vec2::new(5.5, 12.0)),
                        ..default()
                    },
                    Transform::from_xyz(0.0, -5.5, 0.01),
                ));

                // Blooming palm aura (illuminates emerald on buy, crimson on sell)
                arm.spawn((
                    TraderHandAuraVisual { instrument, is_left: false, trader_entity: trader_id },
                    Sprite {
                        color: Color::srgba(0.0, 0.0, 0.0, 0.0),
                        custom_size: Some(Vec2::new(22.0, 22.0)),
                        ..default()
                    },
                    Transform::from_xyz(0.0, -11.0, 0.005),
                ));

                // Color-coded buy (white/cyan) or sell (pink) execution ticket held in hand
                arm.spawn((
                    TraderOrderSlipVisual { instrument, is_left: false, trader_entity: trader_id },
                    Sprite {
                        color: Color::srgb(0.92, 0.96, 1.0),
                        custom_size: Some(Vec2::new(7.0, 11.0)),
                        ..default()
                    },
                    Transform::from_xyz(2.0, -11.0, 0.015),
                ));

                // High-contrast hand shadow outline
                arm.spawn((
                    TraderHandOutlineVisual { is_left: false },
                    Sprite {
                        color: Color::srgb(0.08, 0.08, 0.10),
                        custom_size: Some(Vec2::new(13.5, 13.5)),
                        ..default()
                    },
                    Transform::from_xyz(0.0, -11.0, 0.02),
                ));

                // Main chunky articulated hand palm (at wrist, pointing down by hip when idle)
                arm.spawn((
                    TraderHandVisual { instrument, role, is_left: false, trader_entity: trader_id },
                    Sprite {
                        color: Color::srgb(0.95, 0.78, 0.62),
                        custom_size: Some(if role == TraderRole::FloorBroker { Vec2::new(13.0, 13.0) } else { Vec2::new(11.0, 11.0) }),
                        ..default()
                    },
                    Transform::from_xyz(0.0, -11.0, 0.025),
                ));

                // Articulated finger pads (at fingertips below palm)
                arm.spawn((
                    TraderHandFingerVisual { instrument, is_left: false, trader_entity: trader_id },
                    Sprite {
                        color: Color::srgb(0.98, 0.88, 0.75),
                        custom_size: Some(Vec2::new(7.5, 5.0)),
                        ..default()
                    },
                    Transform::from_xyz(0.0, -13.5, 0.03),
                ));
            });

            // 9. Floating Overhead Palm Signal Status Badge for Lead Broker
            if role == TraderRole::FloorBroker {
                parent.spawn((
                    BrokerPalmBadgeVisual,
                    Sprite {
                        color: Color::srgba(0.04, 0.18, 0.08, 0.94),
                        custom_size: Some(Vec2::new(118.0, 19.0)),
                        ..default()
                    },
                    Transform::from_xyz(0.0, 36.0, 0.10),
                )).with_children(|badge| {
                    badge.spawn((
                        BrokerPalmBadgeText,
                        Text2d::new("✋ PALMS IN // BUY"),
                        TextFont {
                            font_size: 10.0,
                            ..default()
                        },
                        TextColor(Color::srgb(0.20, 1.0, 0.45)),
                        Transform::from_xyz(0.0, 0.0, 0.01),
                    ));
                });
            }
        });

    trader_id
}

/// Pure helper: what a participant cries for an engine event, in pit convention.
/// Bids are "price for quantity", offers are "quantity at price", and the aggressor
/// closes with "SOLD" (hitting a bid) or "BUY 'EM" (lifting an offer).
pub fn cry_for_event(pit: &PitMarket, kind: &PitEventKind) -> Option<(usize, String, bool)> {
    match *kind {
        PitEventKind::Quote { slot, side, price_ticks, qty } => {
            let d = pit.price_digit(price_ticks);
            let text = match side {
                Side::Buy => format!("{d} FOR {qty}!"),
                Side::Sell => format!("{qty} AT {d}!"),
            };
            Some((slot, text, side.is_buy()))
        }
        PitEventKind::Fill { aggressor, side, qty, .. } => {
            let text = match side {
                Side::Buy => format!("BUY 'EM! {qty}"),
                Side::Sell => format!("SOLD! {qty}"),
            };
            Some((aggressor, text, side.is_buy()))
        }
        PitEventKind::OutTrade { b, .. } => Some((b, "OUT-TRADE!".to_string(), false)),
        PitEventKind::Print { price_ticks, up } => {
            let price = crate::pit::format_price_with_commas(
                pit.price_of(price_ticks) as f64,
                crate::pit::instrument_decimals(pit.instrument),
            );
            Some((1, format!("{price} {}", if up { "+" } else { "-" }), up))
        }
        PitEventKind::BackAway { slot } => Some((slot, "OUT!".to_string(), false)),
        _ => None,
    }
}

/// Helper: Spawns a floating trade shout popup above a trader
fn spawn_trade_shout(commands: &mut Commands, pos: Vec3, role: TraderRole, text: &str, is_buying: bool) {
    let shout_color = match role {
        TraderRole::PriceReporter => Color::srgb(0.96, 0.85, 0.30),
        _ => {
            if is_buying {
                Color::srgb(0.25, 1.0, 0.45)
            } else {
                Color::srgb(1.0, 0.30, 0.30)
            }
        }
    };
    let width = (text.chars().count() as f32 * 7.2 + 14.0).max(40.0);

    commands
        .spawn((
            TradeShoutPopup {
                timer: 0.0,
                max_lifetime: 1.1,
                velocity_y: 28.0,
            },
            Transform::from_xyz(pos.x, pos.y + 36.0, 6.0),
            Visibility::default(),
        ))
        .with_children(|p| {
            p.spawn((
                Sprite {
                    color: Color::srgba(0.02, 0.03, 0.06, 0.94),
                    custom_size: Some(Vec2::new(width, 18.0)),
                    ..default()
                },
                Transform::from_xyz(0.0, 0.0, 0.01),
            ));
            p.spawn((
                Text2d::new(text),
                TextFont {
                    font_size: 12.5,
                    ..default()
                },
                TextColor(shout_color),
                Transform::from_xyz(0.0, 0.0, 0.02),
            ));
        });
}

/// System: Drives directional phalanx steering, floor runners moving along aisles,
/// scissoring walking stride cycles, arm gestures, and trade shout popups
pub fn update_trader_movement(
    mut commands: Commands,
    time: Res<Time>,
    view_mode: Res<FloorViewMode>,
    engines: Res<PitEngines>,
    pits: Query<&PitBounds>,
    mut traders: Query<(
        Entity,
        &Trader,
        Option<&TraderRole>,
        Option<&PitSlot>,
        &IsMarketMaker,
        &mut TraderMovement,
        &mut Velocity,
        &mut TraderGesture,
        &mut Transform,
        &Children,
    )>,
    mut leg_query: Query<
        (&TraderLegVisual, &mut Transform),
        (
            Without<Trader>,
            Without<TraderJacketVisual>,
            Without<TraderHeadVisual>,
            Without<TraderArmVisual>,
        ),
    >,
    mut jacket_query: Query<
        &mut Transform,
        (
            With<TraderJacketVisual>,
            Without<Trader>,
            Without<TraderLegVisual>,
            Without<TraderHeadVisual>,
            Without<TraderArmVisual>,
        ),
    >,
    mut head_query: Query<
        &mut Transform,
        (
            With<TraderHeadVisual>,
            Without<Trader>,
            Without<TraderLegVisual>,
            Without<TraderJacketVisual>,
            Without<TraderArmVisual>,
        ),
    >,
    mut arm_query: Query<
        (&TraderArmVisual, &mut Transform),
        (
            Without<Trader>,
            Without<TraderLegVisual>,
            Without<TraderJacketVisual>,
            Without<TraderHeadVisual>,
        ),
    >,
    mut tie_query: Query<
        (&TraderTieVisual, &mut Sprite),
        (Without<Trader>, Without<BrokerPalmBadgeVisual>),
    >,
    mut palm_badge_query: Query<
        (&BrokerPalmBadgeVisual, &mut Sprite, &Children),
        (Without<Trader>, Without<TraderTieVisual>),
    >,
    mut palm_badge_text_query: Query<
        (&BrokerPalmBadgeText, &mut Text2d, &mut TextColor),
        Without<Trader>,
    >,
) {
    // The 2D floor is not visible in 3D mode; its sprites can stand still.
    if view_mode.is_3d() {
        return;
    }
    let dt = time.delta_secs();
    let elapsed = time.elapsed_secs();

    // Where every pit slot stands right now, so counterparties can find each other and
    // shouts can be placed over the right head.
    let mut slot_positions: HashMap<(Entity, usize), Vec3> = HashMap::new();
    let mut slot_roles: HashMap<(Entity, usize), TraderRole> = HashMap::new();
    for (_, trader, role, slot, _, _, _, _, transform, _) in traders.iter() {
        if let Some(slot) = slot {
            slot_positions.insert((trader.pit_entity, slot.0), transform.translation);
            slot_roles.insert((trader.pit_entity, slot.0), role.copied().unwrap_or(TraderRole::LocalScalper));
        }
    }

    // Shouts come from this frame's engine events, one per event, in pit convention.
    // Locals' routine quotes are thinned so the floor reads as a crowd, not a wall of text.
    for (pit_entity, _) in slot_positions.keys().map(|(e, _)| (*e, ())).collect::<HashMap<_, _>>() {
        // Find this pit's instrument via any trader in it.
        let Some((_, trader, _, _, _, _, _, _, _, _)) = traders.iter().find(|(_, t, _, s, ..)| t.pit_entity == pit_entity && s.is_some()) else {
            continue;
        };
        let pit = engines.pit(trader.instrument);
        for ev in pit.recent_events() {
            let Some((slot, text, is_buying)) = cry_for_event(pit, &ev.kind) else {
                continue;
            };
            let routine_local_quote = matches!(ev.kind, PitEventKind::Quote { slot, .. } if role_for_slot(slot) == ParticipantRole::Local);
            if routine_local_quote && ((ev.t * 1000.0) as u64 % 10) >= 4 {
                continue;
            }
            if let Some(pos) = slot_positions.get(&(pit_entity, slot)) {
                let role = slot_roles.get(&(pit_entity, slot)).copied().unwrap_or(TraderRole::LocalScalper);
                spawn_trade_shout(&mut commands, *pos, role, &text, is_buying);
            }
        }
    }

    for (
        entity,
        trader,
        role,
        slot,
        is_mm,
        mut movement,
        mut velocity,
        mut gesture,
        mut transform,
        children,
    ) in traders.iter_mut()
    {
        let pit = engines.pit(trader.instrument);
        let current_role = role.copied().unwrap_or(TraderRole::LocalScalper);
        let phase = slot.map(|s| pit.participant(s.0).phase).unwrap_or(Phase::Idle);
        let mut badge_text: Option<(&str, String)> = None;

        if current_role == TraderRole::ConcourseLounger {
            // Concourse Loungers: resting and chatting in pairs along the aisles outside pits
            velocity.0 = Vec2::ZERO;
            gesture.flash_timer = 0.0;
            let is_pair_right = transform.translation.x > movement.target.x;
            movement.facing_x = if is_pair_right { -1.0 } else { 1.0 };
            transform.translation.z = 1.1 + (600.0 - transform.translation.y) * 0.001;
        } else if movement.is_runner {
            // Floor Runner Logic: walking along concourses between pits
            let to_target = movement.target - transform.translation.xy();
            let dist = to_target.length();

            if dist < 12.0 {
                movement.wait_timer -= dt;
                if movement.wait_timer <= 0.0 {
                    movement.runner_waypoint_idx =
                        (movement.runner_waypoint_idx + 1) % CONCOURSE_WAYPOINTS.len();
                    movement.target = CONCOURSE_WAYPOINTS[movement.runner_waypoint_idx];
                    movement.wait_timer = 1.0;
                }
                velocity.0 = Vec2::ZERO;
            } else {
                let dir = to_target.normalize();
                velocity.0 = dir * movement.base_speed;
                if dir.x.abs() > 0.15 {
                    movement.facing_x = dir.x.signum();
                }
                transform.translation.x += velocity.0.x * dt;
                transform.translation.y += velocity.0.y * dt;
            }
            transform.translation.z = 1.2 + (600.0 - transform.translation.y) * 0.001;
        } else if current_role == TraderRole::PriceReporter || is_mm.0 {
            // Price Reporter on the pulpit: faces the last print, keys when a print happens
            let last_up = pit.events.iter().rev().find_map(|e| match e.kind {
                PitEventKind::Print { up, .. } => Some(up),
                _ => None,
            });
            movement.facing_x = if last_up.unwrap_or(true) { 1.0 } else { -1.0 };
            transform.translation.z = 2.5;
            velocity.0 = Vec2::ZERO;
            gesture.flash_timer = 0.0;
        } else {
            // Pit participant on a step: everything follows the engine phase
            let Ok(bounds) = pits.get(trader.pit_entity) else {
                continue;
            };
            let my_slot = slot.map(|s| s.0).unwrap_or(2);
            let home = movement.target;
            let to_center = bounds.center - home;
            let inward = if to_center.length_squared() > 1.0 { to_center.normalize() } else { Vec2::Y };

            // Hands: shown while bidding, offering or pointing; the side is the participant's own.
            match phase.signalled_side() {
                Some(side) => {
                    gesture.is_buying = side.is_buy();
                    gesture.flash_timer = 0.2;
                }
                None => gesture.flash_timer = 0.0,
            }

            let counterparty_pos = phase
                .counterparty()
                .and_then(|cp| slot_positions.get(&(trader.pit_entity, cp)))
                .map(|p| p.xy());

            let (target_pos, speed) = match phase {
                Phase::Hitting { .. } | Phase::Confirming { .. } | Phase::Disputing { .. } => {
                    let toward = counterparty_pos
                        .map(|cp| (cp - home).normalize_or_zero())
                        .unwrap_or(inward);
                    (home + toward * 14.0, 60.0)
                }
                Phase::Quoting { .. } | Phase::Working { .. } => (home + inward * 8.0, 45.0),
                Phase::BackingAway { .. } => (home - inward * 11.0, 40.0),
                Phase::Running { remaining } => {
                    let tangent = Vec2::new(-inward.y, inward.x);
                    (home + tangent * (remaining * 2.2).sin() * 26.0, 90.0)
                }
                _ => (home, 22.0),
            };

            let to_target = target_pos - transform.translation.xy();
            let dist = to_target.length();
            if dist > 1.0 {
                let dir = to_target.normalize();
                velocity.0 = dir * speed;
                transform.translation.x += velocity.0.x * dt;
                transform.translation.y += velocity.0.y * dt;
                if dir.x.abs() > 0.15 && counterparty_pos.is_none() {
                    movement.facing_x = dir.x.signum();
                }
            } else {
                transform.translation.x = target_pos.x;
                transform.translation.y = target_pos.y;
                velocity.0 = Vec2::ZERO;
            }

            // Face the counterparty while dealing, otherwise the pit centre.
            if let Some(cp) = counterparty_pos {
                let dx = cp.x - transform.translation.x;
                if dx.abs() > 2.0 {
                    movement.facing_x = dx.signum();
                }
            } else if matches!(phase, Phase::Quoting { .. } | Phase::Working { .. }) && inward.x.abs() > 0.2 {
                movement.facing_x = inward.x.signum();
            }

            let clamped_xy = bounds.clamp_point(transform.translation.xy(), 12.0);
            transform.translation.x = clamped_xy.x;
            transform.translation.y = clamped_xy.y;
            let z_base = if current_role == TraderRole::FloorBroker { 1.35 } else { 1.2 };
            transform.translation.z = z_base + (600.0 - clamped_xy.y) * 0.001;

            if current_role == TraderRole::FloorBroker {
                let deck = pit.participant(my_slot).deck.len();
                badge_text = Some(match phase {
                    Phase::Working { side: Side::Buy, qty_left, .. } => ("buy", format!("✋ IN: BUY {qty_left}")),
                    Phase::Working { side: Side::Sell, qty_left, .. } => ("sell", format!("✋ OUT: SELL {qty_left}")),
                    Phase::Hitting { side: Side::Buy, .. } => ("buy", "BUY 'EM!".to_string()),
                    Phase::Hitting { side: Side::Sell, .. } => ("sell", "SOLD!".to_string()),
                    Phase::Endorsing { .. } => ("deck", "📋 ENDORSING".to_string()),
                    _ if deck > 0 => ("deck", format!("📋 DECK: {deck} ORDERS")),
                    _ => ("deck", "📋 ORDER DECK".to_string()),
                });
            }
        }

        // Count down active gesture flash timer and shout cooldown
        if gesture.flash_timer > 0.0 {
            gesture.flash_timer = (gesture.flash_timer - dt).max(0.0);
        }

        if gesture.shout_cooldown > 0.0 {
            gesture.shout_cooldown = (gesture.shout_cooldown - dt).max(0.0);
        }

        // 6. Animate Child Avatar Components (Stride cycle when walking, calm breathing when planted)
        let phase = entity.to_bits() as f32 % std::f32::consts::TAU;
        let is_walking = movement.is_runner || (velocity.0.length() > 25.0);
        let stride = (elapsed * 10.0 + phase).sin();
        let idle_breath = (elapsed * 2.2 + phase).sin() * 0.35;

        for child in children.iter() {
            // Articulated legs: scissor only when walking; subtle weight shift when planted
            if let Ok((leg, mut leg_transform)) = leg_query.get_mut(child) {
                let leg_dir = if leg.is_left { 1.0 } else { -1.0 };
                if is_walking {
                    leg_transform.translation.y = -7.5 + stride * 3.0 * leg_dir;
                    leg_transform.translation.x =
                        (if leg.is_left { -4.5 } else { 4.5 }) - stride * 1.0 * leg_dir;
                } else {
                    let idle_weight = (elapsed * 1.8 + phase).sin();
                    leg_transform.translation.y = -7.5 + idle_weight * 0.4 * leg_dir;
                    leg_transform.translation.x = if leg.is_left { -4.5 } else { 4.5 };
                }
            }

            // Torso & Jacket: Stride bob cycle or calm idle breathing
            if let Ok(mut jacket_transform) = jacket_query.get_mut(child) {
                if is_walking {
                    let bob = stride.abs() * 1.2;
                    jacket_transform.translation.y = 4.5 + bob;
                } else {
                    jacket_transform.translation.y = 4.5 + idle_breath;
                }
            }

            // Head: Stride bob cycle or calm idle breathing
            if let Ok(mut head_transform) = head_query.get_mut(child) {
                if is_walking {
                    let bob = stride.abs() * 1.2;
                    head_transform.translation.y = 18.0 + bob;
                } else {
                    head_transform.translation.y = 18.0 + idle_breath;
                }
            }

            // Floating overhead badge (brokers): what the card says right now
            if let Ok((_badge, mut badge_sprite, badge_children)) =
                palm_badge_query.get_mut(child)
            {
                let (kind, text) = badge_text
                    .clone()
                    .unwrap_or(("deck", "📋 ORDER DECK".to_string()));
                let (bg_color, text_color) = match kind {
                    "buy" => (Color::srgba(0.03, 0.22, 0.08, 0.95), Color::srgb(0.20, 1.0, 0.45)),
                    "sell" => (Color::srgba(0.28, 0.04, 0.04, 0.95), Color::srgb(1.0, 0.28, 0.28)),
                    _ => (Color::srgba(0.14, 0.11, 0.05, 0.90), Color::srgb(1.0, 0.85, 0.30)),
                };

                badge_sprite.color = bg_color;
                for b_child in badge_children.iter() {
                    if let Ok((_, mut btext, mut color)) =
                        palm_badge_text_query.get_mut(b_child)
                    {
                        **btext = text.clone();
                        *color = TextColor(text_color);
                    }
                }
            }

            // Arms: Open-Cry Gestures & Hand Signals (Smooth slerp, Arms DOWN at rest, Arms UP in trade bursts)
            if let Ok((arm, mut arm_transform)) = arm_query.get_mut(child) {
                let arm_sign = if arm.is_left { 1.0 } else { -1.0 };
                let shoulder_x = if arm.is_left { -9.5 } else { 9.5 };
                let shoulder_y = 10.5 + idle_breath * 0.3;

                let (target_arm_pos, target_arm_rot) = if current_role == TraderRole::PriceReporter {
                    // Reporter taps keys on podium terminal: arms angled forward resting on desk
                    let tap = (elapsed * 5.0 + phase).sin() * 0.06;
                    (
                        Vec3::new(shoulder_x, 10.0, 0.035),
                        Quat::from_rotation_z(arm_sign * (0.65 + tap)),
                    )
                } else if gesture.flash_timer > 0.0 {
                    // ACTIVE OPEN-CRY SIGNAL BURST!
                    if gesture.is_buying {
                        // PALMS IN: Arms raised high, angled inward toward chest waving tickets (Pull toward self)
                        let wave = (elapsed * 6.0 + phase).sin() * 0.08;
                        (
                            Vec3::new(if arm.is_left { -8.5 } else { 8.5 }, 10.5, 0.035),
                            Quat::from_rotation_z(arm_sign * (2.70 + wave)),
                        )
                    } else {
                        // PALMS OUT: Arms thrust wide outward toward pit pushing orders away (Push away)
                        let thrust = (elapsed * 5.0 + phase).sin() * 0.06;
                        (
                            Vec3::new(if arm.is_left { -9.5 } else { 9.5 }, 10.5, 0.035),
                            Quat::from_rotation_z(-arm_sign * (1.65 + thrust)),
                        )
                    }
                } else if is_walking {
                    // WALKING / SPRINTING: Natural arm swing at side around hip height
                    let swing = stride * 0.28;
                    (
                        Vec3::new(shoulder_x, shoulder_y, 0.035),
                        Quat::from_rotation_z(arm_sign * swing),
                    )
                } else if current_role == TraderRole::FloorBroker {
                    // Broker resting posture: holding clipboard/order deck at waist/chest level
                    (
                        Vec3::new(shoulder_x, shoulder_y, 0.035),
                        Quat::from_rotation_z(arm_sign * 0.35),
                    )
                } else {
                    // Local Scalpers & Concourse Loungers: resting calmly down at sides with gentle breathing sway
                    let rest_sway = (elapsed * 1.8 + phase).sin() * 0.04;
                    (
                        Vec3::new(shoulder_x, shoulder_y, 0.035),
                        Quat::from_rotation_z(arm_sign * rest_sway),
                    )
                };

                // Smoothly slerp arm transform to eliminate any snapping or jitter
                arm_transform.translation = arm_transform.translation.lerp(target_arm_pos, (dt * 12.0).min(1.0));
                arm_transform.rotation = arm_transform.rotation.slerp(target_arm_rot, (dt * 12.0).min(1.0));
            }

            // Necktie / Badge Flash during executions
            if let Ok((_, mut tie_sprite)) = tie_query.get_mut(child) {
                if gesture.flash_timer > 0.0 {
                    if gesture.is_buying {
                        tie_sprite.color = Color::srgb(0.20, 1.0, 0.40);
                    } else {
                        tie_sprite.color = Color::srgb(1.0, 0.25, 0.25);
                    }
                    tie_sprite.custom_size = Some(Vec2::new(4.5, 9.5));
                } else {
                    tie_sprite.color = match current_role {
                        TraderRole::FloorBroker => Color::srgb(1.0, 0.88, 0.28), // Bright Gold badge
                        TraderRole::PriceReporter => Color::srgb(0.96, 0.82, 0.22), // Official medallion
                        _ => Color::srgb(0.85, 0.18, 0.20),
                    };
                    tie_sprite.custom_size = Some(if current_role == TraderRole::FloorBroker {
                        Vec2::new(4.5, 8.5)
                    } else {
                        Vec2::new(3.2, 7.5)
                    });
                }
            }
        }
    }
}

/// System: Dynamically updates open-outcry articulated hands, palm colors, glowing signal auras,
/// and buy/sell order slips across all traders on the floor
pub fn update_trader_hand_signals(
    time: Res<Time>,
    view_mode: Res<FloorViewMode>,
    gestures: Query<&TraderGesture>,
    mut hand_query: Query<(&TraderHandVisual, &mut Sprite, &mut Transform)>,
    mut finger_query: Query<
        (&TraderHandFingerVisual, &mut Sprite, &mut Transform),
        Without<TraderHandVisual>,
    >,
    mut aura_query: Query<
        (&TraderHandAuraVisual, &mut Sprite),
        (Without<TraderHandVisual>, Without<TraderHandFingerVisual>),
    >,
    mut slip_query: Query<
        (&TraderOrderSlipVisual, &mut Sprite),
        (
            Without<TraderHandVisual>,
            Without<TraderHandFingerVisual>,
            Without<TraderHandAuraVisual>,
        ),
    >,
) {
    if view_mode.is_3d() {
        return;
    }
    let elapsed = time.elapsed_secs();

    // 1. Articulated Hand & Palm Sprites (Vivid on EVERY trader)
    for (hand, mut sprite, mut transform) in hand_query.iter_mut() {
        let (is_active, is_buying) = gestures
            .get(hand.trader_entity)
            .map(|g| (g.flash_timer > 0.0, g.is_buying))
            .unwrap_or((false, true));

        let base_size = if hand.role == TraderRole::FloorBroker { 13.0 } else { 11.0 };

        if is_active {
            if is_buying {
                // PALMS IN: Vibrant Emerald Buy Palm
                sprite.color = Color::srgb(0.18, 1.0, 0.45);
                sprite.custom_size = Some(Vec2::new(base_size * 1.15, base_size * 1.15));
                let angle = if hand.is_left { 0.30 } else { -0.30 };
                transform.rotation = Quat::from_rotation_z(angle);
            } else {
                // PALMS OUT: Vibrant Crimson Sell Palm (wide splayed open hand)
                sprite.color = Color::srgb(1.0, 0.22, 0.22);
                sprite.custom_size = Some(Vec2::new(base_size * 1.25, base_size * 1.20));
                let angle = if hand.is_left { -0.45 } else { 0.45 };
                transform.rotation = Quat::from_rotation_z(angle);
            }
        } else {
            // Neutral: Warm natural skin tone with dark outline, zero angle
            sprite.color = Color::srgb(0.95, 0.78, 0.62);
            sprite.custom_size = Some(Vec2::new(base_size, base_size));
            transform.rotation = Quat::IDENTITY;
        }
    }

    // 2. Inner Palm / Finger Directional Facets
    for (finger, mut sprite, mut transform) in finger_query.iter_mut() {
        let (is_active, is_buying) = gestures
            .get(finger.trader_entity)
            .map(|g| (g.flash_timer > 0.0, g.is_buying))
            .unwrap_or((false, true));

        if is_active {
            if is_buying {
                // Inner palm glowing bright emerald core
                sprite.color = Color::srgb(0.85, 1.0, 0.90);
                sprite.custom_size = Some(Vec2::new(8.0, 6.0));
                let angle = if finger.is_left { 0.30 } else { -0.30 };
                transform.rotation = Quat::from_rotation_z(angle);
            } else {
                // Open splayed finger pads glowing bright coral
                sprite.color = Color::srgb(1.0, 0.88, 0.88);
                sprite.custom_size = Some(Vec2::new(9.0, 6.0));
                let angle = if finger.is_left { -0.45 } else { 0.45 };
                transform.rotation = Quat::from_rotation_z(angle);
            }
        } else {
            // Neutral fingernails / knuckles
            sprite.color = Color::srgb(0.98, 0.88, 0.75);
            sprite.custom_size = Some(Vec2::new(7.0, 5.0));
            transform.rotation = Quat::IDENTITY;
        }
    }

    // 3. Glowing Hand Signal Auras (Radiates outward during active order flow burst)
    for (aura, mut sprite) in aura_query.iter_mut() {
        let (is_active, is_buying) = gestures
            .get(aura.trader_entity)
            .map(|g| (g.flash_timer > 0.0, g.is_buying))
            .unwrap_or((false, true));

        if is_active {
            let pulse = (elapsed * 12.0).sin() * 2.0;
            if is_buying {
                // Pulsing Emerald Aura
                sprite.color = Color::srgba(0.15, 1.0, 0.40, 0.70);
                sprite.custom_size = Some(Vec2::new(22.0 + pulse, 22.0 + pulse));
            } else {
                // Pulsing Crimson Aura
                sprite.color = Color::srgba(1.0, 0.20, 0.20, 0.70);
                sprite.custom_size = Some(Vec2::new(22.0 + pulse, 22.0 + pulse));
            }
        } else {
            // Completely invisible when quiet/neutral
            sprite.color = Color::srgba(0.0, 0.0, 0.0, 0.0);
            sprite.custom_size = Some(Vec2::ZERO);
        }
    }

    // 4. Color-Coded Execution Order Slips (Authentic Blue/White Buy Slips vs Pink Sell Slips)
    for (slip, mut sprite) in slip_query.iter_mut() {
        let (is_active, is_buying) = gestures
            .get(slip.trader_entity)
            .map(|g| (g.flash_timer > 0.0, g.is_buying))
            .unwrap_or((false, true));

        if is_active {
            if is_buying {
                // Buy ticket: Crisp white / light cyan slip
                sprite.color = Color::srgb(0.92, 0.98, 1.0);
                sprite.custom_size = Some(Vec2::new(7.0, 11.0));
            } else {
                // Sell ticket: Authentic CBOT trading floor pink slip
                sprite.color = Color::srgb(1.0, 0.58, 0.68);
                sprite.custom_size = Some(Vec2::new(7.0, 11.0));
            }
        } else {
            // Neutral ticket held down at side
            sprite.color = Color::srgb(0.90, 0.93, 0.96);
            sprite.custom_size = Some(Vec2::new(6.0, 9.0));
        }
    }
}

/// System: Animates and fades out floating retro trade shout pills (Theme Park / The Sims style)
pub fn update_trade_shouts(
    mut commands: Commands,
    time: Res<Time>,
    mut shouts: Query<(Entity, &mut TradeShoutPopup, &mut Transform, &Children)>,
    mut sprites: Query<&mut Sprite, (Without<TradeShoutPopup>, Without<Trader>)>,
    mut texts: Query<&mut TextColor, (Without<TradeShoutPopup>, Without<Trader>)>,
) {
    let dt = time.delta_secs();
    for (entity, mut shout, mut transform, children) in shouts.iter_mut() {
        shout.timer += dt;
        if shout.timer >= shout.max_lifetime {
            commands.entity(entity).despawn();
        } else {
            transform.translation.y += shout.velocity_y * dt;
            let progress = (shout.timer / shout.max_lifetime).clamp(0.0, 1.0);
            let alpha = (1.0 - progress * progress).clamp(0.0, 1.0);
            for child in children.iter() {
                if let Ok(mut sprite) = sprites.get_mut(child) {
                    let c = sprite.color;
                    sprite.color = c.with_alpha(alpha * 0.88);
                }
                if let Ok(mut text_color) = texts.get_mut(child) {
                    let c = text_color.0;
                    text_color.0 = c.with_alpha(alpha);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_phalanx_target_buying_phalanx() {
        let bounds = PitBounds::new(Vec2::ZERO, 200.0, 100.0);
        let target = compute_phalanx_target(&bounds, 0.90, 42, false);
        assert!(target.x > -20.0, "Expected positive X bias: {}", target.x);
        assert!(target.y > -10.0, "Expected positive Y bias: {}", target.y);
    }

    #[test]
    fn test_compute_phalanx_target_selling_phalanx() {
        let bounds = PitBounds::new(Vec2::ZERO, 200.0, 100.0);
        let target = compute_phalanx_target(&bounds, -0.90, 42, false);
        assert!(target.x < 20.0, "Expected negative X bias: {}", target.x);
        assert!(target.y < 10.0, "Expected negative Y bias: {}", target.y);
    }

    #[test]
    fn test_compute_phalanx_target_market_maker_anchored() {
        let bounds = PitBounds::new(Vec2::ZERO, 200.0, 100.0);
        let target = compute_phalanx_target(&bounds, 0.95, 42, true);
        assert!(target.length() < 70.0, "Market maker should remain near center post");
    }

    #[test]
    fn test_activity_to_agent_count() {
        let quiet = activity_to_agent_count(0.12, 4, 18);
        assert!(quiet <= 8);
        let frantic = activity_to_agent_count(0.95, 4, 18);
        assert!(frantic >= 16);
    }

    #[test]
    fn test_trader_roles_distinct() {
        assert_ne!(TraderRole::PriceReporter, TraderRole::FloorBroker);
        assert_ne!(TraderRole::FloorBroker, TraderRole::LocalScalper);
        assert_ne!(TraderRole::LocalScalper, TraderRole::FloorRunner);
        assert!(TraderRole::PriceReporter.label().contains("Price Reporter"));
        assert!(TraderRole::FloorBroker.label().contains("Lead Floor Broker"));
    }

    #[test]
    fn test_hand_visual_components() {
        let hand = TraderHandVisual {
            instrument: Instrument::Sp500,
            role: TraderRole::FloorBroker,
            is_left: true,
            trader_entity: Entity::PLACEHOLDER,
        };
        assert_eq!(hand.instrument, Instrument::Sp500);
        assert_eq!(hand.role, TraderRole::FloorBroker);
        assert!(hand.is_left);

        let aura = TraderHandAuraVisual {
            instrument: Instrument::Gold,
            is_left: false,
            trader_entity: Entity::PLACEHOLDER,
        };
        assert_eq!(aura.instrument, Instrument::Gold);
        assert!(!aura.is_left);
    }

    #[test]
    fn test_compute_tier_step_position() {
        let bounds = PitBounds::new(Vec2::new(100.0, 100.0), 300.0, 200.0);
        let pos_reporter = compute_tier_step_position(&bounds, 0);
        let pos_broker = compute_tier_step_position(&bounds, 1);
        let pos_local1 = compute_tier_step_position(&bounds, 3);
        let pos_local2 = compute_tier_step_position(&bounds, 4);

        assert_ne!(pos_reporter, pos_broker);
        assert_ne!(pos_local1, pos_local2);
        assert!(bounds.contains(pos_reporter));
        assert!(bounds.contains(pos_broker));
        assert!(bounds.contains(pos_local1));
    }

    #[test]
    fn test_concourse_lounger_role() {
        assert_eq!(
            TraderRole::ConcourseLounger.label(),
            "Concourse Lounger (Resting/Chatting)"
        );
        assert_ne!(TraderRole::ConcourseLounger, TraderRole::FloorRunner);
    }
}

