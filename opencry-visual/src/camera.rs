use crate::market::Instrument;
use crate::pit_3d::{FLOOR_3D_CENTER, nearest_pit, pit_3d_origin};
use crate::ui::Selection;
use bevy::input::mouse::{MouseMotion, MouseScrollUnit, MouseWheel};
use bevy::prelude::*;

/// Whether the retired 2D isometric floor is compiled in and reachable.
pub const LEGACY_2D_FLOOR: bool = cfg!(feature = "legacy_2d_floor");

/// Active visual presentation mode for the trading exchange floor
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub enum FloorViewMode {
    FloorOverview2D,
    PitDeepDive3D,
}

impl Default for FloorViewMode {
    fn default() -> Self {
        if LEGACY_2D_FLOOR {
            FloorViewMode::FloorOverview2D
        } else {
            FloorViewMode::PitDeepDive3D
        }
    }
}

/// What the UI asks of the 3D camera. Consumed by `camera_orbit_3d` each frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CameraGoal {
    /// Frame the inspected pit.
    Pit,
    /// Frame the whole floor.
    Market,
    /// Whichever of the two is not current.
    TogglePitMarket,
}

#[derive(Resource, Debug, Clone, Copy, Default)]
pub struct CameraRequest {
    pub goal: Option<CameraGoal>,
    /// Multiplicative dolly request from the +/- buttons; 1.0 means none.
    pub zoom_factor: f32,
}

impl CameraRequest {
    pub fn goal(&mut self, goal: CameraGoal) {
        self.goal = Some(goal);
    }

    pub fn zoom(&mut self, factor: f32) {
        let f = if self.zoom_factor == 0.0 {
            1.0
        } else {
            self.zoom_factor
        };
        self.zoom_factor = f * factor;
    }
}

impl FloorViewMode {
    pub fn is_3d(&self) -> bool {
        matches!(self, FloorViewMode::PitDeepDive3D)
    }

    /// Swap views. Without the legacy floor compiled in there is only one view, so this is a no-op.
    pub fn toggle(&mut self) {
        if !LEGACY_2D_FLOOR {
            *self = FloorViewMode::PitDeepDive3D;
            return;
        }
        *self = match self {
            FloorViewMode::FloorOverview2D => FloorViewMode::PitDeepDive3D,
            FloorViewMode::PitDeepDive3D => FloorViewMode::FloorOverview2D,
        };
    }
}

/// Marker for the main 2D world camera
#[derive(Component)]
pub struct MainCamera;

/// Marker component for the 3D Orbit Camera inspecting the active pit
#[derive(Component)]
pub struct PitCamera3d;

/// 3D orbit camera with eased goals. Zooming out past `MARKET_VIEW_RADIUS` re-targets the
/// whole floor; zooming back in returns to the inspected pit.
#[derive(Component, Debug, Clone)]
pub struct OrbitCamera {
    pub radius: f32,
    pub min_radius: f32,
    pub max_radius: f32,
    pub yaw: f32,   // azimuth around Y
    pub pitch: f32, // elevation above the horizontal plane
    pub target: Vec3,
    pub goal_radius: f32,
    pub goal_pitch: f32,
    pub goal_target: Vec3,
    /// The pit the camera last framed, so an external selection change can re-aim it.
    pub framed: Option<Instrument>,
    pub market_view: bool,
    /// Mouse travel since the last button press, to tell a click from a drag.
    pub drag_travel: f32,
    /// A paper trader the camera is following (a row of the Traders panel was clicked): its
    /// avatar stays the target until the camera is sent to a pit or the floor.
    pub follow: Option<crate::paper::Archetype>,
}

/// Orbit radius when following one trader: close enough to see the figure and its label.
pub const FOLLOW_RADIUS: f32 = 58.0;

/// Beyond this orbit radius the camera frames the whole floor instead of one pit.
pub const MARKET_VIEW_RADIUS: f32 = 240.0;
pub const PIT_VIEW_RADIUS: f32 = 96.0;
pub const FLOOR_VIEW_RADIUS: f32 = 470.0;

impl Default for OrbitCamera {
    fn default() -> Self {
        let target = pit_3d_origin(Instrument::Silver) + Vec3::new(0.0, 4.0, 0.0);
        Self {
            radius: PIT_VIEW_RADIUS,
            min_radius: 24.0,
            max_radius: 620.0,
            yaw: 0.0,
            pitch: 36.0f32.to_radians(),
            target,
            goal_radius: PIT_VIEW_RADIUS,
            goal_pitch: 36.0f32.to_radians(),
            goal_target: target,
            framed: Some(Instrument::Silver),
            market_view: false,
            drag_travel: 0.0,
            follow: None,
        }
    }
}

impl OrbitCamera {
    pub fn compute_position(&self) -> Vec3 {
        let x = self.target.x + self.radius * self.pitch.cos() * self.yaw.sin();
        let y = self.target.y + self.radius * self.pitch.sin();
        let z = self.target.z + self.radius * self.pitch.cos() * self.yaw.cos();
        Vec3::new(x, y, z)
    }

    /// Frame one pit from the rim.
    pub fn focus_pit(&mut self, inst: Instrument) {
        self.goal_target = pit_3d_origin(inst) + Vec3::new(0.0, 4.0, 0.0);
        self.goal_radius = PIT_VIEW_RADIUS;
        self.goal_pitch = 36.0f32.to_radians();
        self.framed = Some(inst);
        self.market_view = false;
        self.follow = None;
    }

    /// Frame the whole floor from high up: the market at a glance.
    pub fn market_view(&mut self) {
        self.goal_target = FLOOR_3D_CENTER + Vec3::new(0.0, 4.0, 0.0);
        self.goal_radius = FLOOR_VIEW_RADIUS;
        self.goal_pitch = 58.0f32.to_radians();
        self.market_view = true;
        self.follow = None;
    }

    /// Fly to a trader's avatar, in front of it (`facing` is its yaw), and keep it framed as
    /// it walks.
    pub fn follow_trader(&mut self, arch: crate::paper::Archetype, at: Vec3, facing: f32) {
        self.follow = Some(arch);
        self.goal_target = at + Vec3::new(0.0, 10.0, 0.0);
        self.goal_radius = FOLLOW_RADIUS;
        self.goal_pitch = 22.0f32.to_radians();
        self.yaw = facing;
        self.market_view = false;
    }

    /// Ease toward goals; call once per frame.
    pub fn ease(&mut self, dt: f32) {
        let k = (dt * 4.0).min(1.0);
        self.radius += (self.goal_radius - self.radius) * k;
        self.pitch += (self.goal_pitch - self.pitch) * k;
        self.target = self.target.lerp(self.goal_target, k);
    }
}

/// Semantic Zoom Level of Detail (LOD) tier (Section 20)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SemanticLodTier {
    /// Zoomed in close: focus on individual traders, firms, and pit details
    MicroDetail,
    /// Standard floor operating view
    Standard,
    /// Macro overview: high-level aggregate crowds and district heat
    MacroOverview,
}

/// Pure function: Computes semantic zoom tier from camera orthographic scale
pub fn semantic_lod_tier(zoom_scale: f32) -> SemanticLodTier {
    if zoom_scale < 0.75 {
        SemanticLodTier::MicroDetail
    } else if zoom_scale > 1.65 {
        SemanticLodTier::MacroOverview
    } else {
        SemanticLodTier::Standard
    }
}

/// Startup system: Spawns the main 2D camera and the 3D orbit camera
pub fn setup_camera(mut commands: Commands) {
    // 1. Main 2D Orthographic Camera
    commands.spawn((
        Camera2d,
        MainCamera,
        IsDefaultUiCamera,
        Camera {
            order: 0,
            is_active: true,
            ..default()
        },
        Transform::from_xyz(0.0, -10.0, 999.0),
        Projection::Orthographic(OrthographicProjection {
            scale: 1.05,
            ..OrthographicProjection::default_2d()
        }),
    ));

    // 2. 3D Orbit Camera for Pit Deep-Dive
    let orbit = OrbitCamera::default();
    let initial_pos = orbit.compute_position();
    let target = orbit.target;

    commands.spawn((
        Camera3d::default(),
        PitCamera3d,
        Camera {
            order: 0,
            is_active: false, // Initially false in 2D Floor mode
            ..default()
        },
        orbit,
        Transform::from_translation(initial_pos).looking_at(target, Vec3::Y),
        Projection::Perspective(PerspectiveProjection {
            fov: 45.0f32.to_radians(),
            ..default()
        }),
    ));
}

/// System: Synchronizes camera activation states with FloorViewMode
pub fn sync_camera_view_mode(
    mut commands: Commands,
    view_mode: Res<FloorViewMode>,
    mut cam_2d: Query<(Entity, &mut Camera), (With<MainCamera>, Without<PitCamera3d>)>,
    mut cam_3d: Query<(Entity, &mut Camera), (With<PitCamera3d>, Without<MainCamera>)>,
) {
    if !view_mode.is_changed() {
        return;
    }

    let is_3d = view_mode.is_3d();

    // The UI follows the active camera, so the HUD, buttons and overlays exist in both views.
    for (entity, mut cam) in cam_2d.iter_mut() {
        cam.is_active = !is_3d;
        if is_3d {
            commands.entity(entity).remove::<IsDefaultUiCamera>();
        } else {
            commands.entity(entity).insert(IsDefaultUiCamera);
        }
    }

    for (entity, mut cam) in cam_3d.iter_mut() {
        cam.is_active = is_3d;
        if is_3d {
            commands.entity(entity).insert(IsDefaultUiCamera);
        } else {
            commands.entity(entity).remove::<IsDefaultUiCamera>();
        }
    }
}

/// System: Handles WASD / Arrow keyboard camera panning, gentle mouse wheel zooming,
/// middle mouse button drag panning, and mobile touch navigation in 2D Floor mode
pub fn camera_pan_zoom(
    time: Res<Time>,
    view_mode: Res<FloorViewMode>,
    keyboard: Res<ButtonInput<KeyCode>>,
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    touches: Res<Touches>,
    windows: Query<&Window>,
    mut mouse_wheel: MessageReader<MouseWheel>,
    mut mouse_motion: MessageReader<MouseMotion>,
    mut query: Query<(&mut Transform, &mut Projection), With<MainCamera>>,
) {
    if view_mode.is_3d() {
        return;
    }

    let Ok((mut transform, mut projection)) = query.single_mut() else {
        return;
    };

    let dt = time.delta_secs();

    // 1. Mouse wheel zoom
    let mut zoom_multiplier: f32 = 1.0;
    for ev in mouse_wheel.read() {
        let delta = match ev.unit {
            MouseScrollUnit::Line => ev.y * 0.05,
            MouseScrollUnit::Pixel => ev.y * 0.001,
        };
        zoom_multiplier *= (1.0 - delta).clamp(0.88, 1.12);
    }

    let mut current_scale = 1.0;
    if let Projection::Orthographic(ref mut ortho) = *projection {
        if (zoom_multiplier - 1.0).abs() > 0.0001 {
            ortho.scale = (ortho.scale * zoom_multiplier).clamp(0.40, 4.20);
        }
        current_scale = ortho.scale;
    }

    // 2. Middle Mouse Button Pan
    let is_middle_panning = mouse_buttons.pressed(MouseButton::Middle);
    for ev in mouse_motion.read() {
        if is_middle_panning {
            transform.translation.x -= ev.delta.x * current_scale;
            transform.translation.y += ev.delta.y * current_scale;
        }
    }

    // 3. Keyboard pan
    let pan_speed = 550.0 * current_scale;
    let mut move_vec = Vec2::ZERO;

    if keyboard.pressed(KeyCode::KeyW) || keyboard.pressed(KeyCode::ArrowUp) {
        move_vec.y += 1.0;
    }
    if keyboard.pressed(KeyCode::KeyS) || keyboard.pressed(KeyCode::ArrowDown) {
        move_vec.y -= 1.0;
    }
    if keyboard.pressed(KeyCode::KeyA) || keyboard.pressed(KeyCode::ArrowLeft) {
        move_vec.x -= 1.0;
    }
    if keyboard.pressed(KeyCode::KeyD) || keyboard.pressed(KeyCode::ArrowRight) {
        move_vec.x += 1.0;
    }

    if move_vec != Vec2::ZERO {
        let dir = move_vec.normalize();
        transform.translation.x += dir.x * pan_speed * dt;
        transform.translation.y += dir.y * pan_speed * dt;
    }

    // 4. Touch Navigation for mobile: Single-finger drag pan & two-finger pinch zoom
    let win_height = windows.single().map(|w| w.height()).unwrap_or(800.0);
    let is_on_ui = |pos: Vec2| -> bool { pos.y <= 65.0 || pos.y >= win_height - 95.0 };

    let active_touch_count = touches.iter().count();
    if active_touch_count == 1 {
        if let Some(touch) = touches.iter().next()
            && !is_on_ui(touch.start_position())
        {
            let delta = touch.delta();
            transform.translation.x -= delta.x * current_scale;
            transform.translation.y += delta.y * current_scale;
        }
    } else if active_touch_count >= 2 {
        let mut iter = touches.iter();
        if let (Some(t0), Some(t1)) = (iter.next(), iter.next()) {
            let p0 = t0.position();
            let p1 = t1.position();
            let prev_p0 = t0.previous_position();
            let prev_p1 = t1.previous_position();

            let curr_dist = p0.distance(p1);
            let prev_dist = prev_p0.distance(prev_p1);

            if prev_dist > 2.0 && curr_dist > 2.0 {
                let pinch_ratio = prev_dist / curr_dist;
                if let Projection::Orthographic(ref mut ortho) = *projection {
                    ortho.scale = (ortho.scale * pinch_ratio).clamp(0.40, 4.20);
                    current_scale = ortho.scale;
                }
            }

            let curr_mid = (p0 + p1) * 0.5;
            let prev_mid = (prev_p0 + prev_p1) * 0.5;
            let mid_delta = curr_mid - prev_mid;
            transform.translation.x -= mid_delta.x * current_scale;
            transform.translation.y += mid_delta.y * current_scale;
        }
    }
}

/// System: 3D camera. Drag to orbit, wheel or pinch to dolly, click a pit to inspect it,
/// Tab for the next pit, M for the whole market. Zooming out past the market threshold frames
/// the floor; zooming back in returns to the inspected pit.
pub fn camera_orbit_3d(
    time: Res<Time>,
    view_mode: Res<FloorViewMode>,
    keyboard: Res<ButtonInput<KeyCode>>,
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    touches: Res<Touches>,
    windows: Query<&Window>,
    mut selection: ResMut<Selection>,
    mut request: ResMut<CameraRequest>,
    mut mouse_wheel: MessageReader<MouseWheel>,
    mut mouse_motion: MessageReader<MouseMotion>,
    mut avatars: ResMut<crate::paper_avatars::AvatarPicking>,
    mut query: Query<
        (&mut Transform, &mut OrbitCamera, &Camera, &GlobalTransform),
        With<PitCamera3d>,
    >,
    ui: Query<&Interaction>,
) {
    if !view_mode.is_3d() {
        mouse_wheel.clear();
        mouse_motion.clear();
        *request = CameraRequest::default();
        return;
    }
    let Ok((mut transform, mut orbit, camera, cam_tf)) = query.single_mut() else {
        return;
    };
    let dt = time.delta_secs();
    let Ok(window) = windows.single() else { return };
    let win_height = window.height();
    let is_on_ui = |pos: Vec2| -> bool { pos.y <= 65.0 || pos.y >= win_height - 95.0 };

    // External selection change (digit keys, 2D click then Space): re-aim at that pit.
    if selection.selected_instrument != orbit.framed
        && let Some(inst) = selection.selected_instrument
    {
        orbit.focus_pit(inst);
    }

    if keyboard.just_pressed(KeyCode::KeyM) {
        orbit.market_view();
    }
    // Requests from the HUD buttons and keys.
    let req = std::mem::take(&mut *request);
    match req.goal {
        Some(CameraGoal::Market) => orbit.market_view(),
        Some(CameraGoal::Pit) => {
            let inst = selection.selected_instrument.unwrap_or(Instrument::Silver);
            orbit.focus_pit(inst);
        }
        Some(CameraGoal::TogglePitMarket) => {
            if orbit.market_view {
                let inst = selection.selected_instrument.unwrap_or(Instrument::Silver);
                orbit.focus_pit(inst);
            } else {
                orbit.market_view();
            }
        }
        None => {}
    }
    if req.zoom_factor > 0.0 && (req.zoom_factor - 1.0).abs() > 1e-4 {
        let (lo, hi) = (orbit.min_radius, orbit.max_radius);
        orbit.goal_radius = (orbit.goal_radius * req.zoom_factor).clamp(lo, hi);
    }
    if keyboard.just_pressed(KeyCode::Tab) {
        let current = selection.selected_instrument.unwrap_or(Instrument::Silver);
        let idx = Instrument::ALL
            .iter()
            .position(|i| *i == current)
            .unwrap_or(0);
        let next = Instrument::ALL[(idx + 1) % Instrument::ALL.len()];
        selection.pick(next);
        orbit.focus_pit(next);
    }

    // Wheel dolly. Crossing the market threshold swaps the framing.
    // The wheel over a trader's panel scrolls the panel (`paper_avatars::scroll_trader_detail`).
    let over_panel =
        matches!((avatars.panel, window.cursor_position()), (Some(r), Some(c)) if r.contains(c));
    for ev in mouse_wheel.read() {
        if over_panel {
            continue;
        }
        let delta = match ev.unit {
            MouseScrollUnit::Line => ev.y * 0.08,
            MouseScrollUnit::Pixel => ev.y * 0.0016,
        };
        let (lo, hi) = (orbit.min_radius, orbit.max_radius);
        orbit.goal_radius = (orbit.goal_radius * (1.0 - delta)).clamp(lo, hi);
    }
    if orbit.goal_radius > MARKET_VIEW_RADIUS && !orbit.market_view {
        let r = orbit.goal_radius;
        orbit.market_view();
        orbit.goal_radius = r;
    } else if orbit.goal_radius <= MARKET_VIEW_RADIUS && orbit.market_view {
        let r = orbit.goal_radius;
        let inst = selection.selected_instrument.unwrap_or(Instrument::Silver);
        orbit.focus_pit(inst);
        orbit.goal_radius = r;
    }

    // Drag orbit.
    if mouse_buttons.just_pressed(MouseButton::Left)
        || mouse_buttons.just_pressed(MouseButton::Middle)
    {
        orbit.drag_travel = 0.0;
    }
    let is_orbiting =
        mouse_buttons.pressed(MouseButton::Left) || mouse_buttons.pressed(MouseButton::Middle);
    for ev in mouse_motion.read() {
        if is_orbiting {
            orbit.drag_travel += ev.delta.length();
            orbit.yaw -= ev.delta.x * 0.007;
            orbit.goal_pitch = (orbit.goal_pitch + ev.delta.y * 0.007).clamp(0.10, 1.50);
            orbit.pitch = orbit.goal_pitch;
        }
    }

    // Click (not drag): a paper trader's avatar (or its open panel) first, else the pit under
    // the cursor is inspected.
    // A click on a button (a row of the Traders panel, say) is the button's, not the floor's.
    let on_button = ui.iter().any(|i| *i != Interaction::None);
    if mouse_buttons.just_released(MouseButton::Left)
        && orbit.drag_travel < 6.0
        && !on_button
        && let Some(cursor) = window.cursor_position()
        && !is_on_ui(cursor)
        && !avatars.consume_click(cursor)
    {
        let picked = camera
            .viewport_to_world(cam_tf, cursor)
            .ok()
            .and_then(|ray| {
                let plane_y = FLOOR_3D_CENTER.y + 2.0;
                let dir_y = ray.direction.y;
                if dir_y.abs() <= 1e-4 {
                    return None;
                }
                let t = (plane_y - ray.origin.y) / dir_y;
                (t > 0.0)
                    .then(|| nearest_pit(ray.origin + ray.direction * t, 48.0))
                    .flatten()
            });
        match picked {
            Some(inst) => {
                selection.pick(inst);
                orbit.focus_pit(inst);
            }
            // Empty floor (or sky): take the wall board down, keep the camera where it is.
            None => selection.click_away(),
        }
    }

    // Touch: one finger orbits, two fingers pinch.
    let active_touch_count = touches.iter().count();
    if active_touch_count == 1 {
        if let Some(touch) = touches.iter().next()
            && !is_on_ui(touch.start_position())
        {
            let delta = touch.delta();
            orbit.yaw -= delta.x * 0.008;
            orbit.goal_pitch = (orbit.goal_pitch + delta.y * 0.008).clamp(0.10, 1.50);
            orbit.pitch = orbit.goal_pitch;
        }
    } else if active_touch_count >= 2 {
        let mut iter = touches.iter();
        if let (Some(t0), Some(t1)) = (iter.next(), iter.next()) {
            let curr_dist = t0.position().distance(t1.position());
            let prev_dist = t0.previous_position().distance(t1.previous_position());
            if prev_dist > 2.0 && curr_dist > 2.0 {
                let scale = prev_dist / curr_dist;
                let (lo, hi) = (orbit.min_radius, orbit.max_radius);
                orbit.goal_radius = (orbit.goal_radius * scale).clamp(lo, hi);
            }
        }
    }

    orbit.ease(dt);
    let new_pos = orbit.compute_position();
    transform.translation = new_pos;
    transform.look_at(orbit.target, Vec3::Y);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_semantic_lod_tier_mapping() {
        assert_eq!(semantic_lod_tier(0.50), SemanticLodTier::MicroDetail);
        assert_eq!(semantic_lod_tier(1.00), SemanticLodTier::Standard);
        assert_eq!(semantic_lod_tier(1.50), SemanticLodTier::Standard);
        assert_eq!(semantic_lod_tier(2.00), SemanticLodTier::MacroOverview);
    }

    #[test]
    fn test_market_view_and_focus_goals() {
        let mut orbit = OrbitCamera::default();
        orbit.market_view();
        assert!(orbit.market_view);
        assert!(orbit.goal_radius > MARKET_VIEW_RADIUS);
        assert!((orbit.goal_target - FLOOR_3D_CENTER).length() < 10.0);
        orbit.focus_pit(Instrument::Gold);
        assert!(!orbit.market_view);
        assert_eq!(orbit.framed, Some(Instrument::Gold));
        assert!((orbit.goal_target - pit_3d_origin(Instrument::Gold)).length() < 10.0);
        for _ in 0..600 {
            orbit.ease(1.0 / 60.0);
        }
        assert!((orbit.radius - PIT_VIEW_RADIUS).abs() < 0.5);
    }

    #[test]
    fn test_floor_view_mode_toggle() {
        let mut mode = FloorViewMode::default();
        if LEGACY_2D_FLOOR {
            assert!(!mode.is_3d());
            mode.toggle();
            assert!(mode.is_3d());
            mode.toggle();
            assert!(!mode.is_3d());
        } else {
            // The 2D floor is retired: the app starts in 3D and toggling never leaves it.
            assert!(mode.is_3d());
            mode.toggle();
            assert!(mode.is_3d());
        }
    }

    #[test]
    fn test_camera_request_accumulates_zoom_and_goal() {
        let mut r = CameraRequest::default();
        r.zoom(0.8);
        r.zoom(0.8);
        assert!((r.zoom_factor - 0.64).abs() < 1e-5);
        r.goal(CameraGoal::Market);
        assert_eq!(r.goal, Some(CameraGoal::Market));
    }

    #[test]
    fn test_orbit_camera_position_calculation() {
        let orbit = OrbitCamera {
            radius: 50.0,
            yaw: 0.0,
            pitch: 0.0,
            target: Vec3::ZERO,
            ..OrbitCamera::default()
        };
        let pos = orbit.compute_position();
        assert!((pos.x - 0.0).abs() < 0.001);
        assert!((pos.y - 0.0).abs() < 0.001);
        assert!((pos.z - 50.0).abs() < 0.001);
    }
}
