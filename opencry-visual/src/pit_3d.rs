//! The 3D trading floor: nine bowl-shaped octagonal pits on one floor, each driven by its
//! own `PitMarket` in the engine.
//!
//! Rendering budget (WebGL2 in a browser tab is the target):
//! * The focused pit gets the full articulated UniMate rigs (about 60 meshes per trader).
//!   Rigs are spawned when a pit gains focus and despawned when it loses it, so there is
//!   ever only one set alive.
//! * Every other pit shows proxy figures: five meshes per trader (legs, body, head, two
//!   hands) that still convey the two things that matter from a distance: hands up or
//!   down, and which side the hands show.
//! * Each pit has a glowing well whose hue is realised flow and brightness is sound, and a
//!   projected nameplate (see `pit_3d_overlay`), so the zoomed-out floor is a market map.

use bevy::asset::RenderAssetUsages;
use bevy::light::CascadeShadowConfigBuilder;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;

use crate::camera::FloorViewMode;
use crate::market::{District, Instrument};
use crate::pit_engine::{
    ParticipantRole, Phase, PitEngines, PitEventKind, PitMarket, SLOTS, Side, role_for_slot,
};
use crate::ui::Selection;
use crate::unimate::{
    JacketStyle, UnimateClip, UnimateMaterialCache, UnimateMeshCache, UnimateTrader,
    spawn_unimate_trader_entity,
};

// ---------------------------------------------------------------------------
// Layout
// ---------------------------------------------------------------------------

/// Base of 3D floor space, far above the 2D floor's coordinates.
pub const PIT_3D_ORIGIN: Vec3 = Vec3::new(0.0, 5000.0, 0.0);

/// Where the camera looks in market view: the middle of the nine pits.
pub const FLOOR_3D_CENTER: Vec3 = Vec3::new(0.0, 5000.0, 0.0);

/// Row depth of the three district rows (metals at the back, indices across the middle,
/// forex and crypto at the front, as seen from the default camera on +Z).
pub const DISTRICT_ROW_Z: f32 = 105.0;
/// Centre-to-centre spacing of neighbouring pits in one row (rim to rim: 38 units of concourse).
pub const PIT_PITCH: f32 = 114.0;

/// World position of a pit's centre. The pits are grouped by district (Paul, 2026-10-23:
/// "each pit should be moved so they are grouped together, metals, indices, forex"): the two
/// metals share the back row, the four indices the middle row, the two FX pairs the front row
/// with Bitcoin set apart at its end.
pub fn pit_3d_origin(inst: Instrument) -> Vec3 {
    let (x, z) = match inst {
        Instrument::Gold => (-PIT_PITCH * 0.5, -DISTRICT_ROW_Z),
        Instrument::Silver => (PIT_PITCH * 0.5, -DISTRICT_ROW_Z),
        Instrument::Uk100 => (-PIT_PITCH * 1.5, 0.0),
        Instrument::Sp500 => (-PIT_PITCH * 0.5, 0.0),
        Instrument::Nasdaq => (PIT_PITCH * 0.5, 0.0),
        Instrument::Us30 => (PIT_PITCH * 1.5, 0.0),
        Instrument::EurUsd => (-PIT_PITCH * 1.0, DISTRICT_ROW_Z),
        Instrument::GbpUsd => (0.0, DISTRICT_ROW_Z),
        Instrument::Bitcoin => (PIT_PITCH * 1.5, DISTRICT_ROW_Z),
    };
    PIT_3D_ORIGIN + Vec3::new(x, 0.0, z)
}

/// Where a district's floor sign stands: on the concourse behind the row (further from the
/// default camera), centred on the district's pits.
pub fn district_sign_origin(d: District) -> Vec3 {
    let pits: Vec<Vec3> = d.pits().map(pit_3d_origin).collect();
    let n = pits.len().max(1) as f32;
    let cx = pits.iter().map(|p| p.x).sum::<f32>() / n;
    let back_z = pits.iter().map(|p| p.z).fold(f32::INFINITY, f32::min);
    Vec3::new(cx, PIT_3D_ORIGIN.y, back_z - BOWL_RIM_RADIUS - 10.0)
}

/// The pit whose centre is nearest a point on the floor, if within `max_dist`.
pub fn nearest_pit(point: Vec3, max_dist: f32) -> Option<Instrument> {
    let mut best: Option<(Instrument, f32)> = None;
    for inst in Instrument::ALL {
        let o = pit_3d_origin(inst);
        let d = Vec2::new(point.x - o.x, point.z - o.z).length();
        if d <= max_dist && best.is_none_or(|(_, bd)| d < bd) {
            best = Some((inst, d));
        }
    }
    best.map(|(i, _)| i)
}

// Bowl geometry. Steps descend from the rim to the well.
pub const BOWL_RIM_RADIUS: f32 = 38.0;
/// (inner radius, outer radius, top height) for each step, rim first.
pub const BOWL_STEPS: [(f32, f32, f32); 4] = [
    (31.0, 38.0, 4.8),
    (24.0, 31.0, 3.6),
    (17.0, 24.0, 2.4),
    (10.0, 17.0, 1.2),
];
pub const WELL_RADIUS: f32 = 10.0;
/// Standing rings: brokers and runners on the top step, locals one step above the well.
pub const TOP_STEP_RADIUS: f32 = 34.5;
pub const LOCAL_STEP_RADIUS: f32 = 20.5;
/// Pulpit on the north rim.
pub const PULPIT_OFFSET_Z: f32 = -41.0;
pub const PULPIT_TOP_Y: f32 = 7.4;

/// Height of the step surface at a horizontal distance from the pit centre.
pub fn step_height_at(radius: f32) -> f32 {
    for &(inner, _outer, top) in BOWL_STEPS.iter() {
        if radius >= inner {
            return top;
        }
    }
    0.0
}

/// Pure function: home position and facing for a slot given how many slots are present.
/// Each ring spreads its active members evenly, leaving the north arc to the pulpit and the
/// lead broker, so a thin pit is a sparse ring rather than a clump.
pub fn compute_pit_home_position(
    crowd_index: usize,
    active_count: usize,
    pit_origin: Vec3,
) -> (Vec3, Quat) {
    use std::f32::consts::{FRAC_PI_2, TAU};
    let face_center = |p: Vec3| {
        let to_center = -Vec2::new(p.x - pit_origin.x, p.z - pit_origin.z);
        Quat::from_rotation_y(to_center.x.atan2(to_center.y))
    };
    let on_ring = |angle: f32, radius: f32, y: f32| {
        pit_origin + Vec3::new(angle.cos() * radius, y, angle.sin() * radius)
    };
    match crowd_index {
        0 => {
            // Lead broker on the top step just east of the pulpit, looking down into the bowl.
            let p = on_ring(-FRAC_PI_2 + 0.42, TOP_STEP_RADIUS, 4.8);
            (p, face_center(p))
        }
        1 => (
            pit_origin + Vec3::new(0.0, PULPIT_TOP_Y, PULPIT_OFFSET_Z),
            Quat::IDENTITY,
        ),
        2..=9 => {
            let count = active_count.clamp(3, 10) - 2;
            let idx = crowd_index - 2;
            let angle = -FRAC_PI_2 + (idx as f32 + 0.5) / count as f32 * TAU;
            let p = on_ring(angle, LOCAL_STEP_RADIUS, 2.4);
            (p, face_center(p))
        }
        _ => {
            let count = active_count.saturating_sub(10).max(1);
            let idx = crowd_index - 10;
            // Leave a gap at north for the pulpit and the lead broker.
            let gap = 1.3;
            let angle = -FRAC_PI_2 + 0.9 + (idx as f32 + 0.5) / count as f32 * (TAU - gap);
            let p = on_ring(angle, TOP_STEP_RADIUS, 4.8);
            (p, face_center(p))
        }
    }
}

// ---------------------------------------------------------------------------
// Components and resources
// ---------------------------------------------------------------------------

/// Root entity of one pit's static geometry.
#[derive(Component)]
pub struct Pit3dRoot {
    pub instrument: Instrument,
}

/// The glowing disc in the well of a pit.
#[derive(Component)]
pub struct Pit3dWellGlow {
    pub instrument: Instrument,
}

/// Tape screen on a pit's pulpit.
#[derive(Component)]
pub struct Pit3dPulpitScreen {
    pub instrument: Instrument,
}

/// A fully rigged trader in the focused pit.
#[derive(Component, Debug, Clone)]
pub struct Pit3dCrowdMember {
    pub instrument: Instrument,
    pub crowd_index: usize,
    pub default_rotation: Quat,
    pub home_position: Vec3,
}

/// Luminous beam between two counterparties mid-trade (focused pit only).
#[derive(Component)]
pub struct Pit3dDealLink {
    pub link_index: usize,
}

/// A proxy figure for a trader in a non-focused pit.
#[derive(Component, Debug, Clone, Copy)]
pub struct Pit3dProxy {
    pub instrument: Instrument,
    pub slot: usize,
}

#[derive(Component, Debug, Clone, Copy)]
pub struct Pit3dProxyHand {
    pub is_left: bool,
}

#[derive(Component)]
pub struct Pit3dProxyBody;

/// Which pit currently owns the full rigs.
#[derive(Resource, Debug, Clone, Copy, Default)]
pub struct Pit3dRigFocus {
    pub instrument: Option<Instrument>,
}

/// Shared materials for proxy hands.
#[derive(Resource, Clone)]
pub struct Pit3dProxyMaterials {
    pub hand_idle: Handle<StandardMaterial>,
    pub hand_buy: Handle<StandardMaterial>,
    pub hand_sell: Handle<StandardMaterial>,
    pub hand_point: Handle<StandardMaterial>,
}

/// Jacket for a slot: the same palette the rigs use, so a pit looks the same near and far.
pub fn jacket_for_slot(slot: usize) -> JacketStyle {
    const JACKETS: [JacketStyle; SLOTS] = [
        JacketStyle::DeanWitterBlue,
        JacketStyle::ReporterBurgundy,
        JacketStyle::CargillRed,
        JacketStyle::CarrGreen,
        JacketStyle::FirstOptionsGold,
        JacketStyle::RefcoCrimson,
        JacketStyle::SunOrange,
        JacketStyle::ScalperGreen,
        JacketStyle::BrokerBlue,
        JacketStyle::CargillRed,
        JacketStyle::RunnerGold,
        JacketStyle::DeanWitterBlue,
        JacketStyle::CarrGreen,
        JacketStyle::RefcoCrimson,
        JacketStyle::RunnerGold,
        JacketStyle::FirstOptionsGold,
        JacketStyle::SunOrange,
        JacketStyle::CargillRed,
    ];
    JACKETS[slot.min(SLOTS - 1)]
}

/// Demographic seed for a slot (matches the original roster).
pub fn seed_for_slot(slot: usize) -> u64 {
    const SEEDS: [u64; SLOTS] = [
        101, 85, 12, 95, 44, 55, 88, 77, 801, 22, 1, 98, 319, 92, 2, 104, 755, 199,
    ];
    SEEDS[slot.min(SLOTS - 1)]
}

// ---------------------------------------------------------------------------
// Pure behaviour helpers (shared by rigs and proxies)
// ---------------------------------------------------------------------------

/// Pure function: Computes the reconciled active 3D crowd count from the 2D pit population
pub fn reconcile_3d_crowd_count(count_2d: usize, fallback_activity_count: usize) -> usize {
    if count_2d == 0 {
        fallback_activity_count.clamp(3, 18)
    } else {
        count_2d.clamp(3, 18)
    }
}

/// Pure function: Filters deals so that only active visible crowd members interact
pub fn filter_active_deals(
    deals: &[(usize, usize, UnimateClip, UnimateClip, bool)],
    active_count: usize,
) -> Vec<(usize, usize, UnimateClip, UnimateClip, bool)> {
    deals
        .iter()
        .filter(|&&(a, b, _, _, _)| a < active_count && b < active_count)
        .copied()
        .collect()
}

/// Pure function: step-forward targets for two counterparties. Each takes one or two paces toward
/// the other, never closer than `min_gap`. Heights are left to `step_height_at` by the caller.
pub fn compute_deal_meeting_positions(home_a: Vec3, home_b: Vec3, min_gap: f32) -> (Vec3, Vec3) {
    let delta = home_b - home_a;
    let dist_xz = (delta.x * delta.x + delta.z * delta.z).sqrt();
    if dist_xz < 0.1 {
        return (home_a, home_b);
    }
    let dir = Vec3::new(delta.x, 0.0, delta.z) / dist_xz;
    let max_step = ((dist_xz - min_gap).max(0.0)) * 0.5;
    let step_dist = (dist_xz * 0.16).clamp(1.2, 2.4).min(max_step);
    let target_a = Vec3::new(
        home_a.x + dir.x * step_dist,
        home_a.y,
        home_a.z + dir.z * step_dist,
    );
    let target_b = Vec3::new(
        home_b.x - dir.x * step_dist,
        home_b.y,
        home_b.z - dir.z * step_dist,
    );
    (target_a, target_b)
}

/// Pure function: natural idle shift and pit scan rotation for non-transacting traders
pub fn compute_idle_movement(
    home_position: Vec3,
    default_rotation: Quat,
    crowd_index: usize,
    elapsed: f32,
) -> (Vec3, Quat) {
    let phase = elapsed * 1.3 + (crowd_index as f32 * 1.73);
    let shift_x = phase.sin() * 0.45;
    let shift_z = (phase * 0.7).cos() * 0.45;
    let target_pos = home_position + Vec3::new(shift_x, 0.0, shift_z);
    let scan_angle = (phase * 0.6).sin() * 0.32;
    (
        target_pos,
        default_rotation * Quat::from_rotation_y(scan_angle),
    )
}

/// Pure function: where a non-transacting trader stands and looks. Roams the step, steps out of
/// a deal's sightline, watches the action; hands stay down because the engine gave no reason.
pub fn compute_pit_bystander_behavior(
    pit_origin: Vec3,
    home_position: Vec3,
    default_rotation: Quat,
    crowd_index: usize,
    elapsed: f32,
    active_deals: &[(usize, usize, UnimateClip, UnimateClip, bool)],
    current_positions: &[Vec3; 18],
) -> (Vec3, Quat, UnimateClip) {
    let wander_t = elapsed * 0.40 + (crowd_index as f32 * 1.618);
    let step_wander_tangent = wander_t.sin() * 2.2;
    let step_wander_radial = (wander_t * 0.7).cos() * 0.9;

    let rad_vec = Vec2::new(
        home_position.x - pit_origin.x,
        home_position.z - pit_origin.z,
    );
    let rad_len = rad_vec.length();
    let (radial_dir, tangent_dir) = if rad_len > 0.1 {
        let r_norm = rad_vec / rad_len;
        (r_norm, Vec2::new(-r_norm.y, r_norm.x))
    } else {
        (Vec2::X, Vec2::Y)
    };

    let offset_x = tangent_dir.x * step_wander_tangent + radial_dir.x * step_wander_radial;
    let offset_z = tangent_dir.y * step_wander_tangent + radial_dir.y * step_wander_radial;
    let mut target_pos = home_position + Vec3::new(offset_x, 0.0, offset_z);

    for &(a, b, _, _, _) in active_deals {
        if a >= 18 || b >= 18 {
            continue;
        }
        let pos_a = current_positions[a];
        let pos_b = current_positions[b];
        let deal_mid = (pos_a + pos_b) * 0.5;
        let ab = Vec2::new(pos_b.x - pos_a.x, pos_b.z - pos_a.z);
        let ab_len_sq = ab.length_squared();
        if ab_len_sq > 0.01 {
            let ca = Vec2::new(target_pos.x - pos_a.x, target_pos.z - pos_a.z);
            let t_proj = (ca.dot(ab) / ab_len_sq).clamp(0.0, 1.0);
            let nearest_pt = Vec2::new(pos_a.x + ab.x * t_proj, pos_a.z + ab.y * t_proj);
            let to_bystander = Vec2::new(target_pos.x, target_pos.z) - nearest_pt;
            let dist_to_corridor = to_bystander.length();
            if dist_to_corridor < 4.2 && t_proj > 0.05 && t_proj < 0.95 {
                let push_dir = if dist_to_corridor > 0.1 {
                    to_bystander / dist_to_corridor
                } else {
                    Vec2::new(-ab.y, ab.x).normalize()
                };
                let yield_dist = (4.5 - dist_to_corridor).max(2.4);
                target_pos = Vec3::new(
                    target_pos.x + push_dir.x * yield_dist,
                    target_pos.y,
                    target_pos.z + push_dir.y * yield_dist,
                );
                let diff_to_deal = deal_mid - target_pos;
                let target_rot = if diff_to_deal.length_squared() > 0.01 {
                    Quat::from_rotation_y(diff_to_deal.x.atan2(diff_to_deal.z))
                } else {
                    default_rotation
                };
                return (target_pos, target_rot, UnimateClip::Idle);
            }
        }
    }

    let default_angle = default_rotation.to_euler(EulerRot::YXZ).0;
    let scan_angle = (wander_t * 0.8).sin() * 0.38;
    let target_rot = if let Some(&(a, b, _, _, _)) = active_deals.first() {
        let deal_mid = (current_positions[a] + current_positions[b]) * 0.5;
        let diff_to_deal = deal_mid - target_pos;
        let deal_angle = diff_to_deal.x.atan2(diff_to_deal.z);
        let look_factor = ((wander_t * 0.5).sin() * 0.5 + 0.5).clamp(0.0, 1.0);
        let blended_angle = default_angle + (deal_angle - default_angle) * (look_factor * 0.7);
        Quat::from_rotation_y(blended_angle)
    } else {
        Quat::from_rotation_y(default_angle + scan_angle)
    };

    (target_pos, target_rot, UnimateClip::Idle)
}

/// Pure function: minimum personal separation across active crowd members (slot 1 is on the pulpit).
pub fn enforce_pit_crowd_separation(
    positions: &mut [Vec3; 18],
    active_count: usize,
    min_separation: f32,
) {
    for _iteration in 0..2 {
        for i in 0..active_count.min(18) {
            if i == 1 {
                continue;
            }
            for j in (i + 1)..active_count.min(18) {
                if j == 1 {
                    continue;
                }
                let delta = positions[j] - positions[i];
                let dist_xz = (delta.x * delta.x + delta.z * delta.z).sqrt();
                if dist_xz < min_separation && dist_xz > 0.01 {
                    let overlap = min_separation - dist_xz;
                    let dir = Vec3::new(delta.x, 0.0, delta.z) / dist_xz;
                    let push = dir * (overlap * 0.5);
                    positions[i] -= push;
                    positions[j] += push;
                }
            }
        }
    }
}

/// Pure helper: which clip the 3D rig plays for a participant phase.
pub fn clip_for_phase(phase: &Phase, role: ParticipantRole) -> UnimateClip {
    if role == ParticipantRole::Reporter {
        return UnimateClip::Pulpit;
    }
    match phase {
        Phase::Idle | Phase::Missed { .. } | Phase::BackingAway { .. } => UnimateClip::Idle,
        Phase::Quoting { side, age, .. } => {
            let cycle = age % 2.4;
            if cycle > 1.7 {
                UnimateClip::QuantityChin
            } else if side.is_buy() {
                UnimateClip::Buy
            } else {
                UnimateClip::Sell
            }
        }
        Phase::Working { side, age, .. } => {
            let cycle = age % 2.0;
            if cycle > 1.5 {
                UnimateClip::QuantityChin
            } else if side.is_buy() {
                UnimateClip::Buy
            } else {
                UnimateClip::Sell
            }
        }
        Phase::Hitting { .. } | Phase::Confirming { .. } => UnimateClip::PointDeal,
        Phase::Endorsing { .. } => UnimateClip::Endorse,
        Phase::Disputing { .. } => UnimateClip::Dispute,
        Phase::Running { .. } => UnimateClip::Sprint,
        Phase::Printing { .. } => UnimateClip::Pulpit,
    }
}

/// Pure helper: counterparty pairs that should show a deal beam this frame.
pub fn deal_pairs(pit: &PitMarket) -> Vec<(usize, usize, bool)> {
    let mut pairs: Vec<(usize, usize, bool)> = Vec::new();
    for slot in 0..pit.active_count {
        let phase = pit.participant(slot).phase;
        let Some(cp) = phase.counterparty() else {
            continue;
        };
        if cp >= pit.active_count {
            continue;
        }
        let (a, b) = if slot < cp { (slot, cp) } else { (cp, slot) };
        let beam = !matches!(phase, Phase::Disputing { .. });
        if let Some(existing) = pairs.iter_mut().find(|p| p.0 == a && p.1 == b) {
            existing.2 |= beam;
        } else {
            pairs.push((a, b, beam));
        }
    }
    pairs
}

/// Most recent print direction in the pit, if any print has happened.
pub fn last_print_up(pit: &PitMarket) -> Option<bool> {
    pit.events.iter().rev().find_map(|e| match e.kind {
        PitEventKind::Print { up, .. } => Some(up),
        _ => None,
    })
}

/// Pure helper: where a participant stands given their phase, before separation and step
/// heights. Shared by rigs and proxies so both views agree.
pub fn phase_target_position(
    pit_origin: Vec3,
    slot: usize,
    phase: &Phase,
    home_positions: &[Vec3; 18],
) -> Vec3 {
    let home = home_positions[slot];
    let radial = {
        let r = Vec2::new(home.x - pit_origin.x, home.z - pit_origin.z);
        if r.length_squared() > 0.01 {
            r.normalize()
        } else {
            Vec2::Y
        }
    };
    match phase {
        Phase::Hitting { counterparty, .. }
        | Phase::Confirming { counterparty, .. }
        | Phase::Disputing { counterparty, .. } => {
            let (mine, _) =
                compute_deal_meeting_positions(home, home_positions[*counterparty], 7.5);
            mine
        }
        Phase::Quoting { .. } | Phase::Working { .. } => {
            home + Vec3::new(-radial.x, 0.0, -radial.y) * 1.6
        }
        Phase::BackingAway { .. } => home + Vec3::new(radial.x, 0.0, radial.y) * 2.6,
        Phase::Running { remaining } => {
            let tangent = Vec3::new(-radial.y, 0.0, radial.x);
            home + tangent * ((remaining * 2.2).sin()) * 4.0
        }
        _ => home,
    }
}

/// Snap a standing position onto the step surface under it.
pub fn settle_on_steps(pit_origin: Vec3, pos: Vec3) -> Vec3 {
    let r = Vec2::new(pos.x - pit_origin.x, pos.z - pit_origin.z).length();
    Vec3::new(pos.x, pit_origin.y + step_height_at(r), pos.z)
}

// ---------------------------------------------------------------------------
// Meshes
// ---------------------------------------------------------------------------

/// Pure function: Generates a procedural 8-sided regular prism (octagon)
pub fn create_octagon_step_mesh(radius: f32, height: f32) -> Mesh {
    create_octagon_ring_mesh(0.0, radius, height)
}

/// Pure function: an octagonal ring prism (annulus) from `inner` to `outer` radius. Inner radius 0
/// gives a solid prism. Height is centred on y = 0.
pub fn create_octagon_ring_mesh(inner: f32, outer: f32, height: f32) -> Mesh {
    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut normals: Vec<[f32; 3]> = Vec::new();
    let mut uvs: Vec<[f32; 2]> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    let half_h = height * 0.5;
    let segments = 8usize;
    let angle_of = |i: usize| {
        (i as f32 / segments as f32) * std::f32::consts::TAU + std::f32::consts::PI / 8.0
    };

    // Top and bottom caps as quad strips between the inner and outer octagons.
    for (y, n) in [(half_h, [0.0, 1.0, 0.0]), (-half_h, [0.0, -1.0, 0.0])] {
        let base = positions.len() as u32;
        for i in 0..segments {
            let a = angle_of(i);
            positions.push([a.cos() * outer, y, a.sin() * outer]);
            normals.push(n);
            uvs.push([0.5 + 0.5 * a.cos(), 0.5 + 0.5 * a.sin()]);
            positions.push([a.cos() * inner, y, a.sin() * inner]);
            normals.push(n);
            uvs.push([0.5 + 0.25 * a.cos(), 0.5 + 0.25 * a.sin()]);
        }
        for i in 0..segments {
            let o0 = base + (2 * i) as u32;
            let i0 = o0 + 1;
            let o1 = base + (2 * ((i + 1) % segments)) as u32;
            let i1 = o1 + 1;
            if y > 0.0 {
                indices.extend_from_slice(&[o0, i0, o1, i0, i1, o1]);
            } else {
                indices.extend_from_slice(&[o0, o1, i0, i0, o1, i1]);
            }
        }
    }

    // Walls: the outer faces outward, the inner faces inward.
    for (radius, outward) in [(outer, true), (inner, false)] {
        if radius <= 0.0 {
            continue;
        }
        for i in 0..segments {
            let a0 = angle_of(i);
            let a1 = angle_of((i + 1) % segments);
            let p0 = [a0.cos() * radius, half_h, a0.sin() * radius];
            let p1 = [a1.cos() * radius, half_h, a1.sin() * radius];
            let p2 = [a1.cos() * radius, -half_h, a1.sin() * radius];
            let p3 = [a0.cos() * radius, -half_h, a0.sin() * radius];
            let mid = (a0 + a1) * 0.5;
            let mut n = [mid.cos(), 0.0, mid.sin()];
            if !outward {
                n = [-n[0], 0.0, -n[2]];
            }
            let base = positions.len() as u32;
            for p in [p0, p1, p2, p3] {
                positions.push(p);
                normals.push(n);
                uvs.push([0.0, 0.0]);
            }
            if outward {
                indices.extend_from_slice(&[base, base + 2, base + 1, base, base + 3, base + 2]);
            } else {
                indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
            }
        }
    }

    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

/// Spawns a brass octagonal rail with stanchions at the given radius.
pub fn spawn_brass_octagon_rail(
    parent: &mut ChildSpawnerCommands,
    rail_mesh: &Handle<Mesh>,
    post_mesh: &Handle<Mesh>,
    brass_mat: Handle<StandardMaterial>,
    radius: f32,
    y_pos: f32,
    stanchion_bottom_y: f32,
) {
    let segments = 8;
    let mut pts = Vec::new();
    for i in 0..segments {
        let angle =
            (i as f32 / segments as f32) * std::f32::consts::TAU + (std::f32::consts::PI / 8.0);
        pts.push(Vec3::new(angle.cos() * radius, y_pos, angle.sin() * radius));
    }
    for i in 0..segments {
        let p_start = pts[i];
        let p_end = pts[(i + 1) % segments];
        let mid = (p_start + p_end) * 0.5;
        let diff = p_end - p_start;
        let len = diff.length();
        parent.spawn((
            Mesh3d(rail_mesh.clone()),
            MeshMaterial3d(brass_mat.clone()),
            Transform::from_translation(mid)
                .with_rotation(Quat::from_rotation_arc(Vec3::Y, diff.normalize()))
                .with_scale(Vec3::new(1.0, len, 1.0)),
        ));
        let post_height = y_pos - stanchion_bottom_y;
        if post_height > 0.2 {
            parent.spawn((
                Mesh3d(post_mesh.clone()),
                MeshMaterial3d(brass_mat.clone()),
                Transform::from_xyz(p_start.x, (y_pos + stanchion_bottom_y) * 0.5, p_start.z)
                    .with_scale(Vec3::new(1.0, post_height, 1.0)),
            ));
        }
    }
}

// ---------------------------------------------------------------------------
// Setup
// ---------------------------------------------------------------------------

/// Startup system: builds the floor, nine bowls, proxy figures and deal beams.
pub fn setup_pit_3d(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    // Shared materials.
    let concourse = materials.add(StandardMaterial {
        base_color: Color::srgb_u8(0x14, 0x1a, 0x26),
        perceptual_roughness: 0.85,
        metallic: 0.05,
        ..default()
    });
    let step_mats: Vec<Handle<StandardMaterial>> = [0x2a_u8, 0x24, 0x1e, 0x18]
        .iter()
        .map(|&v| {
            materials.add(StandardMaterial {
                base_color: Color::srgb_u8(v, v + 8, v + 18),
                perceptual_roughness: 0.70,
                metallic: 0.15,
                ..default()
            })
        })
        .collect();
    let well_mat = materials.add(StandardMaterial {
        base_color: Color::srgb_u8(0x10, 0x14, 0x1c),
        perceptual_roughness: 0.6,
        ..default()
    });
    let mahogany = materials.add(StandardMaterial {
        base_color: Color::srgb_u8(0x3d, 0x1c, 0x24),
        perceptual_roughness: 0.45,
        metallic: 0.25,
        ..default()
    });
    let brass = materials.add(StandardMaterial {
        base_color: Color::srgb_u8(0xd4, 0xaf, 0x37),
        perceptual_roughness: 0.28,
        metallic: 0.85,
        ..default()
    });
    let desk = materials.add(StandardMaterial {
        base_color: Color::srgb_u8(0x1f, 0x24, 0x2e),
        perceptual_roughness: 0.50,
        ..default()
    });

    // Shared meshes.
    let step_meshes: Vec<Handle<Mesh>> = BOWL_STEPS
        .iter()
        .map(|&(inner, outer, top)| meshes.add(create_octagon_ring_mesh(inner, outer, top)))
        .collect();
    let well_mesh = meshes.add(create_octagon_step_mesh(WELL_RADIUS + 0.2, 0.3));
    let glow_mesh = meshes.add(create_octagon_step_mesh(WELL_RADIUS - 0.6, 0.12));
    let apron_mesh = meshes.add(create_octagon_ring_mesh(
        BOWL_RIM_RADIUS,
        BOWL_RIM_RADIUS + 3.5,
        0.3,
    ));
    let pulpit_mesh = meshes.add(create_octagon_step_mesh(4.6, PULPIT_TOP_Y));
    let desk_mesh = meshes.add(Cuboid::new(3.4, 0.4, 1.8));
    let screen_mesh = meshes.add(Cuboid::new(2.2, 1.2, 0.15));
    let rail_mesh = meshes.add(Cylinder::new(0.16, 1.0));
    let post_mesh = meshes.add(Cylinder::new(0.14, 1.0));

    // Floor plate under everything.
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(520.0, 460.0))),
        MeshMaterial3d(concourse),
        Transform::from_translation(FLOOR_3D_CENTER - Vec3::new(0.0, 0.05, 0.0)),
    ));

    for inst in Instrument::ALL {
        let origin = pit_3d_origin(inst);
        let accent = inst.accent_color();
        let apron_mat = materials.add(StandardMaterial {
            base_color: accent,
            emissive: LinearRgba::from(accent) * 0.25,
            perceptual_roughness: 0.6,
            ..default()
        });
        let glow_mat = materials.add(StandardMaterial {
            base_color: Color::srgb(0.05, 0.07, 0.10),
            emissive: LinearRgba::from(Color::srgb(0.10, 0.14, 0.20)),
            perceptual_roughness: 0.3,
            unlit: true,
            ..default()
        });
        let screen_mat = materials.add(StandardMaterial {
            base_color: Color::srgb_u8(0x04, 0x08, 0x10),
            emissive: LinearRgba::from(Color::srgb(0.0, 0.85, 0.40)),
            perceptual_roughness: 0.30,
            ..default()
        });

        commands
            .spawn((
                Pit3dRoot { instrument: inst },
                Transform::from_translation(origin),
                Visibility::Inherited,
            ))
            .with_children(|root| {
                for (idx, &(_, _, top)) in BOWL_STEPS.iter().enumerate() {
                    root.spawn((
                        Mesh3d(step_meshes[idx].clone()),
                        MeshMaterial3d(step_mats[idx].clone()),
                        Transform::from_xyz(0.0, top * 0.5, 0.0),
                    ));
                }
                root.spawn((
                    Mesh3d(well_mesh.clone()),
                    MeshMaterial3d(well_mat.clone()),
                    Transform::from_xyz(0.0, 0.15, 0.0),
                ));
                root.spawn((
                    Pit3dWellGlow { instrument: inst },
                    Mesh3d(glow_mesh.clone()),
                    MeshMaterial3d(glow_mat),
                    Transform::from_xyz(0.0, 0.36, 0.0),
                ));
                root.spawn((
                    Mesh3d(apron_mesh.clone()),
                    MeshMaterial3d(apron_mat),
                    Transform::from_xyz(0.0, 0.15, 0.0),
                ));

                // Pulpit on the north rim with desk and tape screen.
                root.spawn((
                    Mesh3d(pulpit_mesh.clone()),
                    MeshMaterial3d(mahogany.clone()),
                    Transform::from_xyz(0.0, PULPIT_TOP_Y * 0.5, PULPIT_OFFSET_Z),
                ));
                root.spawn((
                    Mesh3d(desk_mesh.clone()),
                    MeshMaterial3d(desk.clone()),
                    Transform::from_xyz(0.0, PULPIT_TOP_Y + 3.2, PULPIT_OFFSET_Z + 2.6),
                ));
                root.spawn((
                    Pit3dPulpitScreen { instrument: inst },
                    Mesh3d(screen_mesh.clone()),
                    MeshMaterial3d(screen_mat),
                    Transform::from_xyz(0.0, PULPIT_TOP_Y + 4.1, PULPIT_OFFSET_Z + 3.0)
                        .with_rotation(Quat::from_rotation_x(-0.2)),
                ));

                spawn_brass_octagon_rail(
                    root,
                    &rail_mesh,
                    &post_mesh,
                    brass.clone(),
                    BOWL_RIM_RADIUS + 0.6,
                    4.8 + 3.4,
                    4.8,
                );
            });
    }

    // Lighting for the whole floor.
    commands.spawn((
        DirectionalLight {
            color: Color::srgb(1.0, 0.97, 0.92),
            illuminance: 12000.0,
            shadows_enabled: true,
            ..default()
        },
        CascadeShadowConfigBuilder {
            num_cascades: 3,
            first_cascade_far_bound: 80.0,
            maximum_distance: 720.0,
            ..default()
        }
        .build(),
        Transform::from_translation(FLOOR_3D_CENTER + Vec3::new(120.0, 220.0, 160.0))
            .looking_at(FLOOR_3D_CENTER, Vec3::Y),
    ));
    commands.spawn((
        PointLight {
            color: Color::srgb(0.55, 0.70, 0.95),
            intensity: 4_000_000.0,
            range: 500.0,
            shadows_enabled: false,
            ..default()
        },
        Transform::from_translation(FLOOR_3D_CENTER + Vec3::new(-150.0, 140.0, -120.0)),
    ));

    // Proxy figures for every slot of every pit.
    let proxy_mats = Pit3dProxyMaterials {
        hand_idle: materials.add(StandardMaterial {
            base_color: Color::srgb(0.95, 0.78, 0.62),
            ..default()
        }),
        hand_buy: materials.add(StandardMaterial {
            base_color: Color::srgb(0.18, 1.0, 0.45),
            emissive: LinearRgba::from(Color::srgb(0.10, 0.9, 0.35)) * 3.0,
            ..default()
        }),
        hand_sell: materials.add(StandardMaterial {
            base_color: Color::srgb(1.0, 0.22, 0.22),
            emissive: LinearRgba::from(Color::srgb(0.9, 0.15, 0.15)) * 3.0,
            ..default()
        }),
        hand_point: materials.add(StandardMaterial {
            base_color: Color::srgb(0.98, 0.82, 0.25),
            emissive: LinearRgba::from(Color::srgb(0.95, 0.75, 0.15)) * 3.0,
            ..default()
        }),
    };
    let body_mesh = meshes.add(Cuboid::new(3.6, 7.0, 2.2));
    let legs_mesh = meshes.add(Cuboid::new(3.0, 8.0, 1.8));
    let head_mesh = meshes.add(Cuboid::new(1.9, 2.1, 1.9));
    let hand_mesh = meshes.add(Cuboid::new(1.1, 1.3, 0.5));
    let legs_mat = materials.add(StandardMaterial {
        base_color: Color::srgb_u8(0x2a, 0x2c, 0x34),
        ..default()
    });
    let mut jacket_mats: std::collections::HashMap<u8, Handle<StandardMaterial>> =
        std::collections::HashMap::new();
    let mut skin_mats: std::collections::HashMap<usize, Handle<StandardMaterial>> =
        std::collections::HashMap::new();

    for inst in Instrument::ALL {
        let origin = pit_3d_origin(inst);
        for slot in 0..SLOTS {
            let jacket = jacket_for_slot(slot);
            let (primary, _, _) = jacket.colors();
            let jacket_mat = jacket_mats
                .entry(jacket as u8)
                .or_insert_with(|| {
                    materials.add(StandardMaterial {
                        base_color: primary,
                        perceptual_roughness: 0.75,
                        ..default()
                    })
                })
                .clone();
            let skin_mat = skin_mats
                .entry(slot)
                .or_insert_with(|| {
                    let (_, skin, _) = crate::unimate::sample_demographics(seed_for_slot(slot));
                    materials.add(StandardMaterial {
                        base_color: skin,
                        perceptual_roughness: 0.65,
                        ..default()
                    })
                })
                .clone();
            let (home, rot) = compute_pit_home_position(slot, SLOTS, origin);
            commands
                .spawn((
                    Pit3dProxy {
                        instrument: inst,
                        slot,
                    },
                    Transform::from_translation(home).with_rotation(rot),
                    Visibility::Hidden,
                ))
                .with_children(|p| {
                    p.spawn((
                        Mesh3d(legs_mesh.clone()),
                        MeshMaterial3d(legs_mat.clone()),
                        Transform::from_xyz(0.0, 4.0, 0.0),
                    ));
                    p.spawn((
                        Pit3dProxyBody,
                        Mesh3d(body_mesh.clone()),
                        MeshMaterial3d(jacket_mat.clone()),
                        Transform::from_xyz(0.0, 11.0, 0.0),
                    ));
                    p.spawn((
                        Mesh3d(head_mesh.clone()),
                        MeshMaterial3d(skin_mat.clone()),
                        Transform::from_xyz(0.0, 15.8, 0.0),
                    ));
                    for is_left in [true, false] {
                        let x = if is_left { -2.6 } else { 2.6 };
                        p.spawn((
                            Pit3dProxyHand { is_left },
                            Mesh3d(hand_mesh.clone()),
                            MeshMaterial3d(proxy_mats.hand_idle.clone()),
                            Transform::from_xyz(x, 7.0, 0.4),
                        ));
                    }
                });
        }
    }
    commands.insert_resource(proxy_mats);
    commands.insert_resource(Pit3dRigFocus::default());

    // Deal beams (focused pit only).
    let deal_beam_mat = materials.add(StandardMaterial {
        base_color: Color::srgba(0.98, 0.85, 0.25, 0.95),
        emissive: LinearRgba::from(Color::srgb(0.98, 0.82, 0.20)) * 4.0,
        perceptual_roughness: 0.1,
        unlit: true,
        ..default()
    });
    let deal_beam_mesh = meshes.add(Cylinder::new(0.08, 1.0));
    for link_idx in 0..4 {
        commands.spawn((
            Pit3dDealLink {
                link_index: link_idx,
            },
            Mesh3d(deal_beam_mesh.clone()),
            MeshMaterial3d(deal_beam_mat.clone()),
            Transform::from_translation(PIT_3D_ORIGIN),
            Visibility::Hidden,
        ));
    }
}

/// System: keeps exactly one pit rigged. When the inspected pit changes, the old rigs go and
/// new ones are spawned at their home positions.
pub fn sync_pit_3d_rig_focus(
    mut commands: Commands,
    selection: Res<Selection>,
    mut focus: ResMut<Pit3dRigFocus>,
    unimate_meshes: Res<UnimateMeshCache>,
    unimate_mats: Res<UnimateMaterialCache>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    rigs: Query<Entity, With<Pit3dCrowdMember>>,
) {
    let wanted = selection.selected_instrument;
    if focus.instrument == wanted {
        return;
    }
    for e in rigs.iter() {
        commands.entity(e).despawn();
    }
    focus.instrument = wanted;
    let Some(inst) = wanted else { return };
    let origin = pit_3d_origin(inst);
    for slot in 0..SLOTS {
        let role = role_for_slot(slot);
        let (pos, default_rotation) = compute_pit_home_position(slot, SLOTS, origin);
        let clip = if role == ParticipantRole::Reporter {
            UnimateClip::Pulpit
        } else {
            UnimateClip::Idle
        };
        let facing_y = default_rotation.to_euler(EulerRot::YXZ).0;
        let entity = spawn_unimate_trader_entity(
            &mut commands,
            &unimate_meshes,
            &unimate_mats,
            &mut materials,
            pos,
            facing_y,
            clip,
            jacket_for_slot(slot),
            seed_for_slot(slot),
        );
        commands.entity(entity).insert(Pit3dCrowdMember {
            instrument: inst,
            crowd_index: slot,
            default_rotation: Quat::from_rotation_y(facing_y),
            home_position: pos,
        });
    }
}

/// System: renders the engine's participant state with the full rigs in the focused pit.
pub fn sync_pit_3d_with_market(
    time: Res<Time>,
    view_mode: Res<FloorViewMode>,
    engines: Res<PitEngines>,
    focus: Res<Pit3dRigFocus>,
    mut crowd: Query<
        (
            &Pit3dCrowdMember,
            &mut UnimateTrader,
            &mut Transform,
            &mut Visibility,
        ),
        Without<Pit3dDealLink>,
    >,
    mut deal_links: Query<
        (&Pit3dDealLink, &mut Transform, &mut Visibility),
        Without<Pit3dCrowdMember>,
    >,
) {
    if !view_mode.is_3d() {
        return;
    }
    let Some(inst) = focus.instrument else {
        for (_, _, mut vis) in deal_links.iter_mut() {
            *vis = Visibility::Hidden;
        }
        return;
    };
    let pit = engines.pit(inst);
    let origin = pit_3d_origin(inst);
    let dt = time.delta_secs();
    let t = time.elapsed_secs();
    let active_count = pit.active_count.clamp(3, 18);

    let mut home_positions = [Vec3::ZERO; 18];
    let mut default_rotations = [Quat::IDENTITY; 18];
    let mut current_positions = [Vec3::ZERO; 18];
    for (member, _, transform, _) in crowd.iter() {
        if member.instrument != inst || member.crowd_index >= 18 {
            continue;
        }
        let (home, rot) = compute_pit_home_position(member.crowd_index, active_count, origin);
        home_positions[member.crowd_index] = home;
        default_rotations[member.crowd_index] = rot;
        current_positions[member.crowd_index] = transform.translation;
    }

    let pairs = deal_pairs(pit);
    let active_deals: Vec<(usize, usize, UnimateClip, UnimateClip, bool)> = pairs
        .iter()
        .map(|&(a, b, beam)| (a, b, UnimateClip::PointDeal, UnimateClip::PointDeal, beam))
        .collect();

    let mut target_positions = [Vec3::ZERO; 18];
    let mut target_rotations = [Quat::IDENTITY; 18];
    let mut target_clips = [UnimateClip::Idle; 18];

    for (member, _, _, _) in crowd.iter() {
        let slot = member.crowd_index;
        if member.instrument != inst || slot >= 18 {
            continue;
        }
        if slot >= active_count {
            target_positions[slot] = home_positions[slot];
            target_rotations[slot] = default_rotations[slot];
            continue;
        }
        let participant = pit.participant(slot);
        let phase = participant.phase;
        target_clips[slot] = clip_for_phase(&phase, participant.role);

        if participant.role == ParticipantRole::Reporter {
            target_positions[slot] = home_positions[slot];
            target_rotations[slot] = default_rotations[slot];
            continue;
        }

        match phase {
            Phase::Idle | Phase::Printing { .. } => {
                let (pos, rot, _) = compute_pit_bystander_behavior(
                    origin,
                    home_positions[slot],
                    default_rotations[slot],
                    slot,
                    t,
                    &active_deals,
                    &current_positions,
                );
                target_positions[slot] = pos;
                target_rotations[slot] = rot;
            }
            _ => {
                target_positions[slot] =
                    phase_target_position(origin, slot, &phase, &home_positions);
                target_rotations[slot] = match phase.counterparty() {
                    Some(cp) => {
                        let to_partner = current_positions[cp] - current_positions[slot];
                        if to_partner.length_squared() > 0.01 {
                            Quat::from_rotation_y(to_partner.x.atan2(to_partner.z))
                        } else {
                            default_rotations[slot]
                        }
                    }
                    None => match phase {
                        Phase::BackingAway { .. } => {
                            default_rotations[slot] * Quat::from_rotation_y(0.55)
                        }
                        Phase::Missed { .. } => {
                            default_rotations[slot] * Quat::from_rotation_y(-0.45)
                        }
                        Phase::Endorsing { .. } => {
                            default_rotations[slot] * Quat::from_rotation_y(0.2)
                        }
                        _ => default_rotations[slot],
                    },
                };
            }
        }
    }

    enforce_pit_crowd_separation(&mut target_positions, active_count, 5.2);

    for (member, mut trader, mut transform, mut vis) in crowd.iter_mut() {
        let slot = member.crowd_index;
        if member.instrument != inst || slot >= 18 {
            continue;
        }
        if slot >= active_count {
            *vis = Visibility::Hidden;
            trader.clip = UnimateClip::Idle;
            transform.translation = home_positions[slot];
            continue;
        }
        *vis = Visibility::Inherited;
        trader.clip = target_clips[slot];
        let phase = pit.participant(slot).phase;
        trader.palm_intensity = match phase {
            Phase::Working { .. } => 1.0,
            Phase::Quoting { .. } => 0.75,
            Phase::Hitting { .. } | Phase::Confirming { .. } => 0.9,
            _ => 0.0,
        };
        let target = if slot == 1 {
            target_positions[slot]
        } else {
            settle_on_steps(origin, target_positions[slot])
        };
        let move_rate = if matches!(phase, Phase::Hitting { .. } | Phase::Running { .. }) {
            6.5
        } else {
            4.0
        };
        transform.translation = transform
            .translation
            .lerp(target, (dt * move_rate).min(1.0));
        transform.rotation = transform
            .rotation
            .slerp(target_rotations[slot], (dt * 6.0).min(1.0));
    }

    for (link, mut link_transform, mut link_vis) in deal_links.iter_mut() {
        if let Some(&(a, b, beam)) = pairs.get(link.link_index)
            && beam
            && a < active_count
            && b < active_count
        {
            let pos_a = current_positions[a] + Vec3::new(0.0, 7.0, 0.0);
            let pos_b = current_positions[b] + Vec3::new(0.0, 7.0, 0.0);
            let diff = pos_b - pos_a;
            let len = diff.length();
            if len > 0.1 {
                link_transform.translation = (pos_a + pos_b) * 0.5;
                link_transform.rotation = Quat::from_rotation_arc(Vec3::Y, diff / len);
                link_transform.scale = Vec3::new(1.0, len, 1.0);
                *link_vis = Visibility::Inherited;
                continue;
            }
        }
        *link_vis = Visibility::Hidden;
    }
}

/// System: proxy figures for every non-focused pit. Position, facing and hands from the engine.
pub fn sync_pit_3d_proxies(
    time: Res<Time>,
    view_mode: Res<FloorViewMode>,
    engines: Res<PitEngines>,
    focus: Res<Pit3dRigFocus>,
    proxy_mats: Res<Pit3dProxyMaterials>,
    mut proxies: Query<
        (&Pit3dProxy, &mut Transform, &mut Visibility, &Children),
        (Without<Pit3dProxyHand>, Without<Pit3dProxyBody>),
    >,
    mut hands: Query<
        (
            &Pit3dProxyHand,
            &mut Transform,
            &mut MeshMaterial3d<StandardMaterial>,
        ),
        (Without<Pit3dProxy>, Without<Pit3dProxyBody>),
    >,
    mut bodies: Query<
        &mut Transform,
        (
            With<Pit3dProxyBody>,
            Without<Pit3dProxy>,
            Without<Pit3dProxyHand>,
        ),
    >,
) {
    if !view_mode.is_3d() {
        return;
    }
    let dt = time.delta_secs();
    let t = time.elapsed_secs();
    let k = (dt * 4.0).min(1.0);

    // Per-pit home tables, computed once per pit per frame.
    let mut homes: Vec<[Vec3; 18]> = Vec::with_capacity(Instrument::ALL.len());
    let mut rots: Vec<[Quat; 18]> = Vec::with_capacity(Instrument::ALL.len());
    for inst in Instrument::ALL {
        let pit = engines.pit(inst);
        let origin = pit_3d_origin(inst);
        let mut h = [Vec3::ZERO; 18];
        let mut r = [Quat::IDENTITY; 18];
        for slot in 0..SLOTS {
            let (p, q) = compute_pit_home_position(slot, pit.active_count.clamp(3, 18), origin);
            h[slot] = p;
            r[slot] = q;
        }
        homes.push(h);
        rots.push(r);
    }

    for (proxy, mut transform, mut vis, children) in proxies.iter_mut() {
        let pit = engines.pit(proxy.instrument);
        let hidden = focus.instrument == Some(proxy.instrument) || proxy.slot >= pit.active_count;
        if hidden {
            if *vis != Visibility::Hidden {
                *vis = Visibility::Hidden;
            }
            continue;
        }
        if *vis != Visibility::Inherited {
            *vis = Visibility::Inherited;
        }
        let idx = proxy.instrument as usize;
        let origin = pit_3d_origin(proxy.instrument);
        let participant = pit.participant(proxy.slot);
        let phase = participant.phase;

        let (mut target, mut rot) = if matches!(phase, Phase::Idle) && proxy.slot != 1 {
            compute_idle_movement(homes[idx][proxy.slot], rots[idx][proxy.slot], proxy.slot, t)
        } else {
            (
                phase_target_position(origin, proxy.slot, &phase, &homes[idx]),
                rots[idx][proxy.slot],
            )
        };
        if let Some(cp) = phase.counterparty() {
            let to = homes[idx][cp] - target;
            if to.length_squared() > 0.01 {
                rot = Quat::from_rotation_y(to.x.atan2(to.z));
            }
        }
        if proxy.slot != 1 {
            target = settle_on_steps(origin, target);
        }
        transform.translation = transform.translation.lerp(target, k);
        transform.rotation = transform.rotation.slerp(rot, k);

        // Hands: up and coloured while the engine says they are showing something.
        let (hands_up, mat) = match phase {
            Phase::Quoting { side, .. } | Phase::Working { side, .. } => (
                true,
                if side.is_buy() {
                    &proxy_mats.hand_buy
                } else {
                    &proxy_mats.hand_sell
                },
            ),
            Phase::Hitting { .. } | Phase::Confirming { .. } => (true, &proxy_mats.hand_point),
            Phase::Disputing { .. } => (true, &proxy_mats.hand_sell),
            _ => (false, &proxy_mats.hand_idle),
        };
        let shouting = phase.is_shouting();
        for child in children.iter() {
            if let Ok((hand, mut ht, mut hm)) = hands.get_mut(child) {
                let x = if hand.is_left { -2.6 } else { 2.6 };
                let target_hand = if hands_up {
                    let bob = ((t * 6.0) + if hand.is_left { 0.0 } else { 1.5 }).sin() * 0.4;
                    Vec3::new(x, 14.5 + bob, 1.4)
                } else {
                    Vec3::new(x, 7.0, 0.4)
                };
                ht.translation = ht.translation.lerp(target_hand, (dt * 10.0).min(1.0));
                if hm.0 != *mat {
                    hm.0 = mat.clone();
                }
            } else if let Ok(mut bt) = bodies.get_mut(child) {
                let lean = if shouting { 0.18 } else { 0.0 };
                bt.rotation = bt
                    .rotation
                    .slerp(Quat::from_rotation_x(lean), (dt * 6.0).min(1.0));
            }
        }
    }
}

/// System: every pit's well glow and pulpit screen read the engine, so the zoomed-out floor is
/// a market map. Hue is realised flow, brightness is sound, and the screen shows the last print.
pub fn sync_pit_3d_wells(
    time: Res<Time>,
    view_mode: Res<FloorViewMode>,
    mut next_update: Local<f32>,
    engines: Res<PitEngines>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    wells: Query<(&Pit3dWellGlow, &MeshMaterial3d<StandardMaterial>)>,
    screens: Query<(&Pit3dPulpitScreen, &MeshMaterial3d<StandardMaterial>)>,
) {
    // Material edits re-upload to the GPU; ten times a second is plenty for a glow.
    if !view_mode.is_3d() || time.elapsed_secs() < *next_update {
        return;
    }
    *next_update = time.elapsed_secs() + 0.1;
    for (well, handle) in wells.iter() {
        let pit = engines.pit(well.instrument);
        let sig = &pit.signals;
        let Some(mat) = materials.get_mut(handle) else {
            continue;
        };
        // The desk's confidence in its read lights the well a little: a confident floor is a bright one.
        let read_glow = pit.floor_read.map(|r| r.confidence * 0.5).unwrap_or(0.0);
        let brightness = 0.25 + sig.sound * 1.6 + sig.pressure.abs() * 0.4 + read_glow;
        let imb = sig.imbalance;
        let base = if imb > 0.12 {
            Color::srgb(0.05, 0.55 + imb * 0.4, 0.18)
        } else if imb < -0.12 {
            Color::srgb(0.60 + (-imb) * 0.4, 0.06, 0.08)
        } else if sig.pressure.abs() > 0.4 {
            Color::srgb(0.55, 0.42, 0.10)
        } else {
            Color::srgb(0.12, 0.18, 0.28)
        };
        mat.emissive = LinearRgba::from(base) * brightness;
    }
    for (screen, handle) in screens.iter() {
        let pit = engines.pit(screen.instrument);
        let Some(mat) = materials.get_mut(handle) else {
            continue;
        };
        let glow = 1.0 + pit.signals.sound * 2.0;
        mat.emissive = match last_print_up(pit) {
            Some(true) => LinearRgba::from(Color::srgb(0.0, 0.95, 0.40)) * glow,
            Some(false) => LinearRgba::from(Color::srgb(0.95, 0.25, 0.25)) * glow,
            None => LinearRgba::from(Color::srgb(0.15, 0.75, 0.95)) * glow,
        };
    }
}

/// Colour for a side, shared with overlays.
pub fn side_color(side: Side) -> Color {
    if side.is_buy() {
        Color::srgb(0.25, 1.0, 0.45)
    } else {
        Color::srgb(1.0, 0.32, 0.32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ring_mesh_attributes() {
        for mesh in [
            create_octagon_step_mesh(20.0, 1.5),
            create_octagon_ring_mesh(10.0, 20.0, 1.2),
        ] {
            assert_eq!(mesh.primitive_topology(), PrimitiveTopology::TriangleList);
            let positions = mesh.attribute(Mesh::ATTRIBUTE_POSITION).expect("positions");
            let normals = mesh.attribute(Mesh::ATTRIBUTE_NORMAL).expect("normals");
            let uvs = mesh.attribute(Mesh::ATTRIBUTE_UV_0).expect("uvs");
            assert!(!positions.is_empty());
            assert_eq!(positions.len(), normals.len());
            assert_eq!(positions.len(), uvs.len());
            let indices = mesh.indices().expect("indices");
            assert!(!indices.is_empty());
            assert_eq!(indices.len() % 3, 0);
        }
    }

    #[test]
    fn test_pit_3d_origin_isolation_and_spacing() {
        const { assert!(PIT_3D_ORIGIN.y >= 4000.0) };
        for a in Instrument::ALL {
            for b in Instrument::ALL {
                if a == b {
                    continue;
                }
                let d = (pit_3d_origin(a) - pit_3d_origin(b)).length();
                assert!(
                    d >= BOWL_RIM_RADIUS * 2.0 + 8.0,
                    "{:?} and {:?} are {d} apart",
                    a,
                    b
                );
            }
        }
    }

    #[test]
    fn pits_are_grouped_by_district_in_rows() {
        // Every pit of a district shares one row, and no two districts share a row except
        // forex and crypto, which share the front row with a gap between them.
        for d in District::ALL {
            let zs: Vec<f32> = d.pits().map(|i| pit_3d_origin(i).z).collect();
            assert!(
                zs.iter().all(|z| (z - zs[0]).abs() < 1e-3),
                "{d:?} is not on one row: {zs:?}"
            );
        }
        let row = |d: District| pit_3d_origin(d.pits().next().unwrap()).z;
        assert!(
            row(District::Metals) < row(District::Equities)
                && row(District::Equities) < row(District::Forex)
        );
        assert_eq!(row(District::Forex), row(District::Crypto));
        let gap = pit_3d_origin(Instrument::Bitcoin).x - pit_3d_origin(Instrument::GbpUsd).x;
        assert!(
            gap > PIT_PITCH * 1.25,
            "Bitcoin stands apart from the FX pair: {gap}"
        );
        // The floor centre is the middle of the layout.
        let xs: Vec<f32> = Instrument::ALL
            .iter()
            .map(|i| pit_3d_origin(*i).x)
            .collect();
        let zs: Vec<f32> = Instrument::ALL
            .iter()
            .map(|i| pit_3d_origin(*i).z)
            .collect();
        let mid = |v: &[f32]| {
            (v.iter().cloned().fold(f32::INFINITY, f32::min)
                + v.iter().cloned().fold(f32::NEG_INFINITY, f32::max))
                * 0.5
        };
        assert!(
            (mid(&xs) - FLOOR_3D_CENTER.x).abs() < 1.0
                && (mid(&zs) - FLOOR_3D_CENTER.z).abs() < 1.0
        );
    }

    #[test]
    fn district_signs_stand_behind_their_rows() {
        for d in District::ALL {
            let sign = district_sign_origin(d);
            for i in d.pits() {
                let o = pit_3d_origin(i);
                assert!(
                    sign.z < o.z - BOWL_RIM_RADIUS,
                    "{d:?} sign at z {} overlaps {i:?}",
                    sign.z
                );
                assert!((sign.x - o.x).abs() <= PIT_PITCH * 1.5 + 1.0);
            }
            // And clear of every other pit.
            assert_eq!(
                nearest_pit(sign, BOWL_RIM_RADIUS),
                None,
                "{d:?} sign stands in a pit"
            );
        }
    }

    #[test]
    fn test_nearest_pit() {
        let o = pit_3d_origin(Instrument::Gold);
        assert_eq!(
            nearest_pit(o + Vec3::new(5.0, 0.0, -3.0), 46.0),
            Some(Instrument::Gold)
        );
        assert_eq!(
            nearest_pit(PIT_3D_ORIGIN + Vec3::new(-70.0, 0.0, -300.0), 46.0),
            None
        );
    }

    #[test]
    fn test_step_heights_descend_to_the_well() {
        assert_eq!(step_height_at(36.0), 4.8);
        assert_eq!(step_height_at(27.0), 3.6);
        assert_eq!(step_height_at(20.5), 2.4);
        assert_eq!(step_height_at(12.0), 1.2);
        assert_eq!(step_height_at(3.0), 0.0);
    }

    #[test]
    fn test_home_positions_sit_on_their_steps_and_face_centre() {
        let origin = pit_3d_origin(Instrument::Silver);
        for active in [3usize, 7, 12, 18] {
            for slot in 0..active {
                let (p, rot) = compute_pit_home_position(slot, active, origin);
                let r = Vec2::new(p.x - origin.x, p.z - origin.z).length();
                if slot == 1 {
                    assert!((p.y - origin.y - PULPIT_TOP_Y).abs() < 1e-3);
                    continue;
                }
                assert!(
                    (p.y - origin.y - step_height_at(r)).abs() < 1e-3,
                    "slot {slot} floats at r {r}"
                );
                let fwd = rot * Vec3::Z;
                let to_center = (origin - p).with_y(0.0).normalize();
                assert!(
                    fwd.dot(to_center) > 0.95,
                    "slot {slot} should face the well"
                );
            }
        }
    }

    #[test]
    fn test_home_positions_keep_their_distance() {
        let origin = pit_3d_origin(Instrument::Gold);
        for active in [5usize, 9, 14, 18] {
            let pts: Vec<Vec3> = (0..active)
                .map(|s| compute_pit_home_position(s, active, origin).0)
                .collect();
            for i in 0..active {
                for j in (i + 1)..active {
                    let d = (pts[i] - pts[j]).with_y(0.0).length();
                    assert!(
                        d >= 6.0,
                        "active {active}: slots {i} and {j} only {d:.1} apart"
                    );
                }
            }
        }
    }

    #[test]
    fn test_counterparty_face_to_face_orientation_math() {
        let pos_a = Vec3::new(10.0, 0.0, 0.0);
        let pos_b = Vec3::new(-10.0, 0.0, 0.0);
        let rot_a = Quat::from_rotation_y((pos_b - pos_a).x.atan2((pos_b - pos_a).z));
        let rot_b = Quat::from_rotation_y((pos_a - pos_b).x.atan2((pos_a - pos_b).z));
        assert!(((rot_a * Vec3::Z).dot(rot_b * Vec3::Z) - (-1.0)).abs() < 0.001);
    }

    #[test]
    fn test_reconcile_3d_crowd_count() {
        assert_eq!(reconcile_3d_crowd_count(3, 10), 3);
        assert_eq!(reconcile_3d_crowd_count(25, 10), 18);
        assert_eq!(reconcile_3d_crowd_count(0, 5), 5);
        assert_eq!(reconcile_3d_crowd_count(0, 1), 3);
    }

    #[test]
    fn test_filter_active_deals() {
        let deals = vec![
            (0, 2, UnimateClip::Buy, UnimateClip::Sell, true),
            (4, 7, UnimateClip::Sell, UnimateClip::PointDeal, true),
        ];
        assert_eq!(filter_active_deals(&deals, 3).len(), 1);
        assert_eq!(filter_active_deals(&deals, 8).len(), 2);
    }

    #[test]
    fn test_compute_deal_meeting_positions() {
        let (a, b) = compute_deal_meeting_positions(
            Vec3::new(10.0, 2.0, 0.0),
            Vec3::new(-10.0, 2.0, 0.0),
            6.0,
        );
        assert!(a.distance(b) >= 6.0);
        assert!(a.x < 10.0 && b.x > -10.0);
        let (ca, cb) = compute_deal_meeting_positions(
            Vec3::new(3.5, 2.0, 0.0),
            Vec3::new(-3.5, 2.0, 0.0),
            6.0,
        );
        assert!((ca.distance(cb) - 6.0).abs() < 0.01);
    }

    #[test]
    fn test_compute_pit_bystander_evasion() {
        let mut current_positions = [Vec3::ZERO; 18];
        current_positions[2] = Vec3::new(10.0, 2.0, 0.0);
        current_positions[4] = Vec3::new(-10.0, 2.0, 0.0);
        let active_deals = vec![(2, 4, UnimateClip::Buy, UnimateClip::Sell, true)];
        let bystander_home = Vec3::new(0.0, 2.0, 0.5);
        let (target_pos, target_rot, clip) = compute_pit_bystander_behavior(
            Vec3::ZERO,
            bystander_home,
            Quat::IDENTITY,
            3,
            5.0,
            &active_deals,
            &current_positions,
        );
        assert!(target_pos.distance(bystander_home) >= 2.0);
        let fwd = target_rot * Vec3::Z;
        let to_deal = (Vec3::new(0.0, 2.0, 0.0) - target_pos).normalize();
        assert!(fwd.dot(to_deal) > 0.9);
        assert_eq!(clip, UnimateClip::Idle);
    }

    #[test]
    fn test_compute_pit_bystander_step_roaming_is_relative_to_pit_origin() {
        let current_positions = [Vec3::ZERO; 18];
        let origin = pit_3d_origin(Instrument::Us30);
        let home = origin + Vec3::new(LOCAL_STEP_RADIUS, 2.4, 0.0);
        let (p1, _, _) = compute_pit_bystander_behavior(
            origin,
            home,
            Quat::IDENTITY,
            2,
            1.0,
            &[],
            &current_positions,
        );
        let (p2, _, _) = compute_pit_bystander_behavior(
            origin,
            home,
            Quat::IDENTITY,
            2,
            8.0,
            &[],
            &current_positions,
        );
        assert!(p1.distance(home) > 0.1 && p1.distance(home) < 4.0);
        assert!(p1.distance(p2) > 0.5);
    }

    #[test]
    fn test_enforce_pit_crowd_separation() {
        let mut positions = [Vec3::ZERO; 18];
        positions[2] = Vec3::new(10.0, 2.0, 0.0);
        positions[3] = Vec3::new(11.0, 2.0, 0.0);
        enforce_pit_crowd_separation(&mut positions, 4, 4.0);
        assert!(positions[2].distance(positions[3]) >= 3.99);
    }

    #[test]
    fn test_settle_on_steps_moves_a_walker_down_the_bowl() {
        let origin = pit_3d_origin(Instrument::Sp500);
        let on_rim = settle_on_steps(origin, origin + Vec3::new(TOP_STEP_RADIUS, 99.0, 0.0));
        let one_in = settle_on_steps(origin, origin + Vec3::new(27.0, 99.0, 0.0));
        assert!((on_rim.y - origin.y - 4.8).abs() < 1e-3);
        assert!((one_in.y - origin.y - 3.6).abs() < 1e-3);
    }
}
