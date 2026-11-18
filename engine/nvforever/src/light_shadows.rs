//! Shadows of placed lights, the settings row Lamp shadows (`light-shadows`;
//! docs/design/graphics-enhanced.md §3.15). An addition of NVForever's own, not a fact
//! about the original game: there placed lights cast none, and no LIGH record of the
//! masters sets the Spot Shadow flag [observed: docs/handoff/esm-data-audit-2026-10-25.md,
//! item 6]. Off by default. Built as the sun's shadows are (shadow_rate.rs), owner
//! 2026-11-18: "look how sun shadows are calculated and do the same", "use the same
//! performance optimizations also for the lamp shadows".
//!
//! - **Which lights: by distance, as the sun's shadows.** Every drawn placed LIGH light
//!   ([`SessionLight`] with a point or spot light) whose light reaches within the sun's
//!   shadow distance of the camera (`sky::SkyState::shadow_distance` × its reach: the
//!   Shadow distance and Longer sun shadows rows) casts, nearest first, up to
//!   [`Tuning::lights`] (a safety cap for the Strip at night). Not by view: a light behind
//!   the camera keeps casting, so turning around draws nothing anew. A light already
//!   casting keeps its place against one [`KEEP`] times nearer.
//! - **Held maps, as the sun's held cascades.** A point light's cube is drawn when it
//!   starts casting and then kept (`bevy::pbr::HoldLightShadowMaps`, a patched bevy_pbr:
//!   engine/vendor/README.md "held light shadow maps"). Between draws the light's maps are
//!   switched off for Bevy (`shadow_maps_enabled` false: no views, no culling, nothing
//!   queued, which is what a shadowed point light costs, about 1 ms a frame each on the
//!   CPU, measured), and the patched renderer still samples its cube. Each light keeps a
//!   slot of the cube texture while it casts, so others coming and going do not move it.
//! - **Redrawn where something changed.** A face is marked when, since it was drawn, a
//!   shadow-casting shape moved, appeared or was shown or hidden with its bounds in the
//!   face, an actor's skeleton moved there, or the light itself moved; something removed
//!   (a taken item) marks the lights around the camera in full; every light is redrawn in
//!   full every [`Tuning::refresh`] frames anyway, each at its own time.
//! - **Staggered, as the sun's cascades.** A light with marked faces is switched on to
//!   draw them on every [`Tuning::near_every`] frame when its light reaches within [`NEAR`]
//!   of the camera, every [`Tuning::far_every`] frame otherwise (each light on frames of
//!   its own), and no more than [`Tuning::draws`] lights a frame, those that never drew
//!   first, then the nearest; the rest wait, and so a moving thing's lamp shadow can lag a
//!   frame or more. Of a light switched on, only the marked faces are drawn; the others
//!   keep their maps. A light that has never drawn casts nothing until it has.
//! - **Spot lights** (few among the placed ones) keep their maps switched on while they
//!   cast; their map is drawn only on the frames it is marked and its turn comes.
//! - **What casts**: the sun's layers (`sky::SHADOW_LAYER`, `sky::BODY_LAYER` and the
//!   world's layer 0), so the player's body casts a lamp's shadow as it casts the sun's;
//!   the first-person arms do not.
//! - **Light fixtures cast none** ([`LightFixture`], [`skip_fixtures`]): the shapes of a
//!   model named as a lamp, and of a light's own model, are taken out of every casting
//!   light's maps, as is a small shape a light sits in or against. The game hangs its
//!   lights under their lamps, so a lamp's housing threw its shadow, larger than the lamp,
//!   onto the ceiling over it (owner 2026-11-18). The sun's shadows keep them.
//! - **Near plane** [`NEAR_Z`]: what lies nearer the light than that casts nothing from it,
//!   so the bulb, shade or flames a light sits in do not black out the room
//!   [engineering choice; `NVF_LIGHT_SHADOW_NEAR` overrides it for a run].
//! - **Maps**: the cubes' side follows the Shadow resolution row: 512 with 2048, 1024 with
//!   4096 (`PointLightShadowMap`); a spot light draws one layer of the sun's texture.
//! - The diffuse light of the display-value sum takes the shadow
//!   (`lights::display_shader`), as the flashlight's always has. The flashlight is not one
//!   of these lights: it casts whenever it is on (flashlight.rs).
//! - Test aid `NVF_PLAY_TEST_LIGHT_SHADOW_HOLD=0`: every casting light draws every face
//!   every frame, for comparing the picture and the cost.

use bevy::camera::primitives::{Aabb, CubemapFrusta, Frustum, Sphere};
use bevy::camera::visibility::{CubemapVisibleEntities, RenderLayers, VisibilitySystems, VisibleMeshEntities};
use bevy::ecs::entity::EntityHashMap;
use bevy::light::{DirectionalLightShadowMap, NotShadowCaster, PointLightShadowMap, SimulationLightSystems};
use bevy::mesh::skinning::SkinnedMesh;
use bevy::pbr::HoldLightShadowMaps;
use bevy::prelude::*;
use bevy::render::extract_resource::ExtractResourcePlugin;
use bevy::transform::TransformSystems;

use crate::lights::{MovingLight, SessionLight};

/// `--light-shadows=<mode>`: the mode lamp shadows start in.
#[derive(clap::ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum LightShadowMode {
    Off,
    Performance,
    Quality,
}

/// The row's choices, in the order the page and the INI keep them (the Sun shadows row's).
pub const CHOICES: [&str; 3] = ["Off", "Performance", "Quality"];

/// The Lamp shadows row's choice: an index into [`CHOICES`] (0: off).
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub struct LightShadows {
    pub choice: usize,
}

impl LightShadows {
    pub fn of(mode: LightShadowMode) -> Self {
        Self { choice: mode as usize }
    }
}

/// How a mode spends its frame time [engineering choices, measured in
/// graphics-enhanced.md §3.15].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tuning {
    /// At most this many lights cast at once, nearest first.
    pub lights: usize,
    /// At most this many point lights switched on to draw in a frame.
    pub draws: usize,
    /// A marked light within [`NEAR`] of the camera draws on every this many frames.
    pub near_every: u64,
    /// A farther one on every this many frames.
    pub far_every: u64,
    /// Every light is drawn again in full every this many frames anyway.
    pub refresh: u64,
}

/// The tuning of a choice; `None` for Off.
pub fn tuning(choice: usize) -> Option<Tuning> {
    match choice {
        1 => Some(Tuning { lights: 16, draws: 1, near_every: 2, far_every: 4, refresh: 1200 }),
        2 => Some(Tuning { lights: 32, draws: 3, near_every: 1, far_every: 2, refresh: 600 }),
        _ => None,
    }
}

/// Lights whose light reaches within this far of the camera count as near (game units,
/// about 21 m) [engineering choice: the room around the player].
pub const NEAR: f32 = 1500.0;

/// A light already casting keeps its place against one this many times nearer.
pub const KEEP: f32 = 1.25;

/// Shadow near plane of a placed light, game units (about 23 cm) [engineering choice:
/// clears the bulbs, shades and fire barrels' rims lights sit in; the flashlight's is 8].
pub const NEAR_Z: f32 = 16.0;

/// A shape or a skeleton counts as moved for the lamps' maps once it is this far from where
/// it last marked them (game units, about 6 cm) [engineering choice, 2026-11-18: in the
/// Prospector Saloon the actors' idle skeletons and the head parts that follow them (eyes,
/// mouth, hat) marked about 116 times a frame, every lamp near an actor every frame].
pub const MOVE_EPS: f32 = 4.0;

/// On a shape of a light fixture: a model named as a lamp ([`is_fixture_model`]) or a
/// light's own model (a LIGH with a model). It casts no lamp shadows; the sun's it still
/// casts. Owner 2026-11-18: a hanging box lamp threw its housing's shadow, larger than the
/// lamp, onto the ceiling above it. The game hangs its lights below their fixtures: in the
/// Goodsprings General Store the light under the box lamp `OffRmLightHanging01` sits 86
/// units under the lamp's reference; over the masters' interiors 3,909 of the 7,543 LIGH
/// references without a model of their own have a lamp model's reference within 160 units,
/// half of them within 89 [observed 2026-11-18: `fnvtool world refs --interiors`].
#[derive(Component)]
pub struct LightFixture;

/// The words in a model's file name that make it a light fixture [engineering choice from
/// the census in [`LightFixture`]: `offrmlight01`, `lightceilingvaultr02on`,
/// `shackhanginglight02`, `lamplightlamp`, `neoclassicalsconce`, …].
const FIXTURE_WORDS: [&str; 7] = ["light", "lamp", "lantern", "chandelier", "sconce", "bulb", "candle"];

/// Whether a model (a path under `meshes\`) is a light fixture: its file name holds one of
/// [`FIXTURE_WORDS`].
pub fn is_fixture_model(path: &str) -> bool {
    let file = path.rsplit(['\\', '/']).next().unwrap_or(path).to_ascii_lowercase();
    FIXTURE_WORDS.iter().any(|w| file.contains(w))
}

/// A shape not named as a fixture is one for a light that lies within this far of its box
/// (game units, about 23 cm, [`NEAR_Z`]): a bulb or shade of another name the light sits
/// in [engineering choice].
pub const FIXTURE_MARGIN: f32 = 16.0;

/// Such a shape's bounding sphere is no larger than this (game units, about 1.8 m), so a
/// wall, floor, ceiling or room piece next to a light is never one [engineering choice: the
/// interior kits' pieces are 256 units and more].
pub const FIXTURE_RADIUS: f32 = 128.0;

/// The faces of a point light's cube (bit `i` for face `i`).
const ALL_FACES: u8 = 0b11_1111;

/// On a placed light while it casts shadows.
#[derive(Component)]
pub struct CastingLight;

/// What a casting light's maps need (main world).
#[derive(Debug, Clone, Copy)]
struct LampState {
    /// Faces changed since they were last drawn (bit 0 for a spot light).
    dirty: u8,
    /// It has not drawn yet (its maps hold nothing of it).
    fresh: bool,
    /// Frame its maps were last drawn in full.
    refreshed: u64,
    /// A number of its own, for its turn in the staggered schedule.
    phase: u64,
}

/// The casting lights' slots and states.
#[derive(Resource, Default)]
struct LampShadowState {
    lamps: EntityHashMap<LampState>,
    /// Stable slots of the casting point lights in the cube texture.
    slots: EntityHashMap<u32>,
    /// The cube texture's room, in lights; it only grows while a mode stays on.
    capacity: u32,
    frame: u64,
    /// Lights switched on and faces drawn since the last summary, and frames counted.
    switched: u64,
    drawn: u64,
    /// What marked faces since the last summary: shapes, skeletons, removals, the lights
    /// themselves (moved or their full redraw due); and a few of the shapes' names.
    causes: [u64; 4],
    names: Vec<String>,
    /// Where each shape (its bounds' centre) and each skeleton (its joints' box) was when
    /// it last marked the lamps, and the frame: moves under [`MOVE_EPS`] add up until they
    /// reach it.
    shapes_at: EntityHashMap<(Vec3, u64)>,
    skeletons_at: EntityHashMap<(Vec3, Vec3, u64)>,
    frames: u64,
    since: f32,
}

pub struct LightShadowsPlugin(pub LightShadowMode);

impl Plugin for LightShadowsPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(LightShadows::of(self.0))
            .init_resource::<LampShadowState>()
            .init_resource::<HoldLightShadowMaps>()
            .add_plugins(ExtractResourcePlugin::<HoldLightShadowMaps>::default())
            .add_systems(
                PostUpdate,
                // Before Bevy resets this frame's view visibility, so the lights' and the
                // shapes' `ViewVisibility` still say what the last frame saw.
                shadow_lights
                    .after(TransformSystems::Propagate)
                    .before(VisibilitySystems::VisibilityPropagate)
                    .before(SimulationLightSystems::AssignLightsToClusters)
                    .before(SimulationLightSystems::UpdateLightFrusta),
            )
            .add_systems(PostUpdate, skip_fixtures.after(SimulationLightSystems::CheckLightVisibility));
    }
}

/// The order of a light among the candidates: the distance from the camera to the edge of
/// its light (0 inside it), then to the light; a casting light's divided by [`KEEP`].
pub fn rank(distance: f32, range: f32, casting: bool) -> (f32, f32) {
    let k = if casting { KEEP } else { 1.0 };
    ((distance - range).max(0.0) / k, distance / k)
}

/// The lights that cast: the `cap` first of `(id, rank)` with a rank's edge distance
/// within `reach`, nearest first; ties keep the order given.
pub fn pick<T: Copy>(candidates: &mut [(T, (f32, f32))], reach: f32, cap: usize) -> Vec<T> {
    candidates.sort_by(|a, b| a.1 .0.total_cmp(&b.1 .0).then(a.1 .1.total_cmp(&b.1 .1)));
    candidates.iter().filter(|c| c.1 .0 <= reach).take(cap).map(|c| c.0).collect()
}

/// The lowest slot not in `used`.
pub fn free_slot(used: impl Iterator<Item = u32>) -> u32 {
    let mut taken: Vec<u32> = used.collect();
    taken.sort_unstable();
    let mut slot = 0;
    for t in taken {
        if t == slot {
            slot += 1;
        } else if t > slot {
            break;
        }
    }
    slot
}

/// Whether a light with these turns draws on `frame`.
pub fn its_turn(frame: u64, phase: u64, every: u64) -> bool {
    (frame + phase).is_multiple_of(every.max(1))
}

/// Which of a point light's faces (or a spot light's map, bit 0) a sphere reaches.
fn faces_reached(sphere: &Sphere, cube: Option<&CubemapFrusta>, spot: Option<&Frustum>) -> u8 {
    if let Some(f) = spot {
        return u8::from(f.intersects_sphere(sphere, true));
    }
    let Some(cube) = cube else { return ALL_FACES };
    cube.frusta.iter().enumerate().fold(0, |m, (i, f)| if f.intersects_sphere(sphere, true) { m | 1 << i } else { m })
}

/// Whether the light's sphere at `light` with `range` and a caster's sphere meet.
fn reaches(light: Vec3, range: f32, sphere: &Sphere) -> bool {
    Vec3::from(sphere.center).distance_squared(light) <= (range + sphere.radius) * (range + sphere.radius)
}

/// How far `light` lies outside the box `aabb` placed by `to_world` (game units, 0 inside),
/// when the box is small enough to be a fixture ([`FIXTURE_RADIUS`]); `None` for a larger one.
pub fn fixture_gap(light: Vec3, to_world: &bevy::math::Affine3A, aabb: &Aabb) -> Option<f32> {
    let m = to_world.matrix3;
    let scale = Vec3::new(m.x_axis.length(), m.y_axis.length(), m.z_axis.length());
    let half = Vec3::from(aabb.half_extents) * scale;
    let radius = half.length();
    if radius > FIXTURE_RADIUS {
        return None;
    }
    let centre = Vec3::from(to_world.transform_point3a(aabb.center));
    // Far from its sphere: no need for the box.
    let far = centre.distance(light) - radius;
    if far > FIXTURE_MARGIN {
        return Some(far);
    }
    let local = Vec3::from(to_world.inverse().transform_point3a(light.into()) - aabb.center);
    let outside = (local.abs() * scale - half).max(Vec3::ZERO);
    Some(outside.length())
}

/// `NVF_LIGHT_SHADOW_FIXTURES=1` (a test aid): log, once for each light when it starts
/// casting, the shapes taken as its fixture and the small ones near it that are not.
fn log_fixtures() -> bool {
    static LOG: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *LOG.get_or_init(|| std::env::var("NVF_LIGHT_SHADOW_FIXTURES").is_ok_and(|v| v.trim() == "1"))
}

/// `NVF_LIGHT_SHADOW_REFRESH=<frames>` (a measuring aid): every light is drawn again in
/// full this often instead of [`Tuning::refresh`], to measure what switching lights on costs.
fn refresh_override() -> Option<u64> {
    static REFRESH: std::sync::OnceLock<Option<u64>> = std::sync::OnceLock::new();
    *REFRESH.get_or_init(|| {
        let v = std::env::var("NVF_LIGHT_SHADOW_REFRESH").ok()?.trim().parse::<u64>().ok().filter(|v| *v > 0)?;
        info!("light shadows: NVF_LIGHT_SHADOW_REFRESH={v}");
        Some(v)
    })
}

/// The near plane in use: [`NEAR_Z`], or `NVF_LIGHT_SHADOW_NEAR` (game units) for a run.
fn near_z() -> f32 {
    static NEAR_PLANE: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    *NEAR_PLANE.get_or_init(|| match std::env::var("NVF_LIGHT_SHADOW_NEAR").ok().and_then(|v| v.trim().parse::<f32>().ok()).filter(|v| *v > 0.0) {
        Some(v) => {
            info!("light shadows: NVF_LIGHT_SHADOW_NEAR={v}");
            v
        }
        None => NEAR_Z,
    })
}

/// `NVF_LIGHT_SHADOW_BIAS=<depth>,<normal>` (a test aid): the shadow biases of the casting
/// lights in place of Bevy's defaults, for comparing.
fn biases() -> Option<(f32, f32)> {
    static BIAS: std::sync::OnceLock<Option<(f32, f32)>> = std::sync::OnceLock::new();
    *BIAS.get_or_init(|| {
        let v = std::env::var("NVF_LIGHT_SHADOW_BIAS").ok()?;
        let (d, n) = v.split_once(',')?;
        let b = (d.trim().parse().ok()?, n.trim().parse().ok()?);
        info!("light shadows: NVF_LIGHT_SHADOW_BIAS depth {} normal {}", b.0, b.1);
        Some(b)
    })
}

/// `NVF_PLAY_TEST_LIGHT_SHADOW_HOLD=0`: nothing held, every casting light draws in full
/// every frame.
fn holding() -> bool {
    static HOLD: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *HOLD.get_or_init(|| {
        let off = std::env::var("NVF_PLAY_TEST_LIGHT_SHADOW_HOLD").is_ok_and(|v| v.trim() == "0");
        if off {
            info!("light shadows: test aid NVF_PLAY_TEST_LIGHT_SHADOW_HOLD=0, every map drawn every frame");
        }
        !off
    })
}

/// The layers a casting light draws its shadow casters from (the sun's, sky.rs).
fn casting_layers() -> RenderLayers {
    RenderLayers::from_layers(&[0, crate::sky::SHADOW_LAYER, crate::sky::BODY_LAYER])
}

/// The cube side for the Shadow resolution row's side.
pub fn cube_side(directional: usize) -> usize {
    if directional >= 4096 { 1024 } else { 512 }
}

type Placed<'a> = (
    Entity,
    Ref<'a, GlobalTransform>,
    &'a InheritedVisibility,
    &'a ViewVisibility,
    Option<&'a mut PointLight>,
    Option<&'a mut SpotLight>,
    Option<&'a MovingLight>,
    Option<&'a CubemapFrusta>,
    Option<&'a Frustum>,
    Has<CastingLight>,
);

/// Shapes whose place or visibility changed (skinned ones go by their skeleton).
type MovedFilter = (With<Mesh3d>, Without<NotShadowCaster>, Without<SkinnedMesh>, Or<(Changed<GlobalTransform>, Changed<InheritedVisibility>)>);

type Skinned<'a> = (Entity, &'a SkinnedMesh, &'a GlobalTransform, &'a InheritedVisibility, Option<&'a RenderLayers>);

type MovedCasters<'a> = (Entity, &'a GlobalTransform, &'a Aabb, Ref<'a, InheritedVisibility>, Option<&'a RenderLayers>, Option<&'a Name>, Option<&'a ChildOf>);

/// A light this frame: what [`shadow_lights`] works out for it.
struct Lamp {
    entity: Entity,
    at: Vec3,
    range: f32,
    spot: bool,
    seen: bool,
    gap: f32,
}

/// Choose the lights that cast, mark what changed in their maps, choose which draw this
/// frame, and set them up for Bevy and the patched renderer.
#[allow(clippy::too_many_arguments)]
fn shadow_lights(
    mut commands: Commands,
    time: Res<Time<Real>>,
    settings: Res<LightShadows>,
    sky: Option<Res<crate::sky::SkyState>>,
    directional: Option<Res<DirectionalLightShadowMap>>,
    mut cube: ResMut<PointLightShadowMap>,
    mut state: ResMut<LampShadowState>,
    mut out: ResMut<HoldLightShadowMaps>,
    camera: Query<&GlobalTransform, crate::fly::WorldCamera>,
    mut lights: Query<Placed, With<SessionLight>>,
    moved: Query<MovedCasters, MovedFilter>,
    skinned: Query<Skinned, Without<NotShadowCaster>>,
    names: Query<&Name>,
    joints: Query<Ref<GlobalTransform>>,
    mut removed: RemovedComponents<Mesh3d>,
    mut logged: Local<Option<(usize, usize, usize)>>,
) {
    let tune = tuning(settings.choice).map(|t| Tuning { refresh: refresh_override().unwrap_or(t.refresh), ..t });
    let reach = sky.as_ref().map_or(0.0, |s| s.shadow_distance * s.shadow_reach.max(1.0));
    let at = camera.iter().next().map(|c| c.translation());
    let state = &mut *state;
    state.frame += 1;
    let frame = state.frame;

    // 1. Which lights cast.
    let chosen: Vec<Entity> = match (tune, at) {
        (Some(t), Some(at)) if reach > 0.0 => {
            let mut candidates: Vec<(Entity, (f32, f32))> = lights
                .iter()
                .filter(|q| q.2.get())
                .filter_map(|(e, t, _, _, point, spot, moving, _, _, casting)| {
                    let (range, intensity) = match (&point, &spot) {
                        (Some(p), _) => (p.range, p.intensity),
                        (None, Some(s)) => (s.range, s.intensity),
                        _ => return None,
                    };
                    (range > 0.0 && moving.map_or(intensity, |m| m.full) > 0.0).then(|| (e, rank(t.translation().distance(at), range, casting)))
                })
                .collect();
            pick(&mut candidates, reach, t.lights)
        }
        _ => Vec::new(),
    };
    if tune.is_some() {
        let side = cube_side(directional.map_or(2048, |d| d.size));
        if cube.size != side {
            cube.size = side;
        }
    }
    // Their states and slots; the rest lose theirs.
    let mut lamps: Vec<Lamp> = Vec::with_capacity(chosen.len());
    for (e, t, _, seen, point, spot, ..) in &lights {
        if !chosen.contains(&e) {
            continue;
        }
        let is_point = point.is_some();
        let range = point.map(|p| p.range).or(spot.map(|s| s.range)).unwrap_or(0.0);
        let entry = state.lamps.entry(e).or_insert(LampState { dirty: 0, fresh: true, refreshed: 0, phase: e.index_u32() as u64 });
        if entry.fresh {
            // Its full redraws fall at a time of its own.
            entry.refreshed = frame.saturating_sub(entry.phase % tune.map_or(1, |t| t.refresh).max(1));
        }
        if is_point && !state.slots.contains_key(&e) {
            let slot = free_slot(state.slots.values().copied());
            state.slots.insert(e, slot);
        }
        let gap = at.map_or(0.0, |a| (t.translation().distance(a) - range).max(0.0));
        lamps.push(Lamp { entity: e, at: t.translation(), range, spot: !is_point, seen: seen.get(), gap });
    }
    state.lamps.retain(|e, _| chosen.contains(e));
    state.slots.retain(|e, _| chosen.contains(e) && lamps.iter().any(|l| l.entity == *e && !l.spot));
    state.capacity = match tune {
        Some(_) => state.capacity.max(state.slots.values().map(|s| s + 1).max().unwrap_or(0).div_ceil(4) * 4),
        None => 0,
    };

    // 2. What changed in their maps since last frame.
    let layers = casting_layers();
    {
        let frusta: EntityHashMap<(Option<CubemapFrusta>, Option<Frustum>)> = lights
            .iter()
            .filter(|q| chosen.contains(&q.0))
            .map(|(e, _, _, _, _, spot, _, cube, frustum, _)| (e, (cube.cloned(), if spot.is_some() { frustum.copied() } else { None })))
            .collect();
        // Marks the faces a sphere reaches; whether it reached any.
        let mark = |sphere: Sphere, lamps_state: &mut EntityHashMap<LampState>| {
            let mut any = false;
            for l in &lamps {
                if !reaches(l.at, l.range, &sphere) {
                    continue;
                }
                let (cube, spot) = frusta.get(&l.entity).map_or((None, None), |f| (f.0.as_ref(), f.1.as_ref()));
                if let Some(s) = lamps_state.get_mut(&l.entity) {
                    let faces = faces_reached(&sphere, cube, spot);
                    any |= faces != 0;
                    s.dirty |= faces;
                }
            }
            any
        };
        for (e, t, aabb, vis, rl, name, parent) in &moved {
            if rl.is_some_and(|l| !l.intersects(&layers)) {
                continue;
            }
            // Shown, hidden or new at once; moved once it went MOVE_EPS from its last mark.
            let centre = t.transform_point(Vec3::from(aabb.center));
            let moved_enough = state.shapes_at.get(&e).is_none_or(|(c, _)| c.distance(centre) > MOVE_EPS);
            if !vis.is_changed() && !moved_enough {
                continue;
            }
            state.shapes_at.insert(e, (centre, frame));
            let scale = t.compute_transform().scale.abs().max_element();
            if mark(Sphere { center: centre.into(), radius: Vec3::from(aabb.half_extents).length() * scale }, &mut state.lamps) {
                state.causes[0] += 1;
                if state.names.len() < 6 {
                    let parent = parent.and_then(|c| names.get(c.parent()).ok());
                    let n = format!("{}/{}", parent.map_or("-", |n| n.as_str()), name.map_or("-", |n| n.as_str()));
                    if !state.names.contains(&n) {
                        state.names.push(n);
                    }
                }
            }
        }
        for (e, skin, t, seen, rl) in &skinned {
            if !seen.get() || rl.is_some_and(|l| !l.intersects(&layers)) {
                continue;
            }
            let root = t.translation();
            if !lamps.iter().any(|l| l.at.distance_squared(root) <= (l.range + 512.0) * (l.range + 512.0)) {
                continue;
            }
            let (mut changed, mut lo, mut hi) = (false, Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
            for j in skin.joints.iter().filter_map(|&j| joints.get(j).ok()) {
                changed |= j.is_changed();
                lo = lo.min(j.translation());
                hi = hi.max(j.translation());
            }
            if !changed || lo.x > hi.x {
                continue;
            }
            let moved_enough = state
                .skeletons_at
                .get(&e)
                .is_none_or(|(l, h, _)| (*l - lo).abs().max_element() > MOVE_EPS || (*h - hi).abs().max_element() > MOVE_EPS);
            if !moved_enough {
                continue;
            }
            state.skeletons_at.insert(e, (lo, hi, frame));
            // The joints' box, grown by a limb's thickness [engineering choice].
            if mark(Sphere { center: ((lo + hi) * 0.5).into(), radius: (hi - lo).length() * 0.5 + 16.0 }, &mut state.lamps) {
                state.causes[1] += 1;
            }
        }
    }
    // Something removed: the lights around the camera, where the player takes, kills and
    // breaks things, are drawn again in full [engineering choice].
    if removed.read().count() > 0 {
        if let Some(a) = at {
            for l in lamps.iter().filter(|l| l.at.distance(a) <= l.range + 512.0) {
                if let Some(s) = state.lamps.get_mut(&l.entity) {
                    s.dirty = ALL_FACES;
                    state.causes[2] += 1;
                }
            }
        }
    }
    // The light moved, or its turn for a full redraw came.
    for (e, t, ..) in &lights {
        if let (Some(s), Some(tune)) = (state.lamps.get_mut(&e), tune) {
            if t.is_changed() || frame.saturating_sub(s.refreshed) >= tune.refresh {
                s.dirty = ALL_FACES;
                s.refreshed = frame;
                state.causes[3] += 1;
            }
        }
    }

    // 3. Which lights draw this frame: those never drawn first, then the nearest marked ones
    // on their turn; at most `draws` point lights.
    lamps.sort_by(|a, b| b.seen.cmp(&a.seen).then(a.gap.total_cmp(&b.gap)));
    let hold_all = !holding();
    let mut order: Vec<&Lamp> = lamps.iter().collect();
    order.sort_by_key(|l| !state.lamps.get(&l.entity).is_some_and(|s| s.fresh));
    let mut draws = tune.map_or(0, |t| t.draws);
    let mut drawing: EntityHashMap<u8> = EntityHashMap::default();
    out.held.clear();
    for l in order {
        let Some(s) = state.lamps.get_mut(&l.entity) else { continue };
        let all = if l.spot { 1 } else { ALL_FACES };
        s.dirty &= all;
        let Some(t) = tune else { continue };
        let every = if l.gap <= NEAR { t.near_every } else { t.far_every };
        let want = if hold_all || s.fresh {
            all
        } else if its_turn(frame, s.phase, every) {
            s.dirty
        } else {
            0
        };
        // Only a light the camera saw last frame is drawn by Bevy this frame.
        let draw = if want != 0 && l.seen && (l.spot || hold_all || draws > 0) {
            if !l.spot && !hold_all {
                draws -= 1;
            }
            want
        } else {
            0
        };
        if draw != 0 {
            s.dirty &= !draw;
            s.fresh = false;
            state.drawn += u64::from(draw.count_ones());
            state.switched += u64::from(!l.spot);
            drawing.insert(l.entity, draw);
        }
        // A spot light's map stays switched on and is held unless drawn; a point light is
        // switched on only to draw, its other faces held.
        if l.spot || draw != 0 {
            out.held.insert(l.entity, all & !draw);
        }
    }

    // 4. Set the lights up: shadows switched on to draw (spot lights while they cast), the
    // casters' layers, the near plane and biases.
    let (near, bias) = (near_z(), biases());
    let (mut points, mut spots) = (0, 0);
    for (e, _, _, _, point, spot, _, _, _, casting) in &mut lights {
        let cast = chosen.contains(&e);
        if let Some(mut p) = point {
            points += usize::from(cast);
            let on = cast && drawing.contains_key(&e);
            if p.shadow_maps_enabled != on {
                p.shadow_maps_enabled = on;
            }
            if cast && p.shadow_map_near_z != near {
                p.shadow_map_near_z = near;
            }
            if let (true, Some((d, n))) = (cast, bias) {
                if (p.shadow_depth_bias, p.shadow_normal_bias) != (d, n) {
                    (p.shadow_depth_bias, p.shadow_normal_bias) = (d, n);
                }
            }
        }
        if let Some(mut s) = spot {
            spots += usize::from(cast);
            if s.shadow_maps_enabled != cast {
                s.shadow_maps_enabled = cast;
            }
            if cast && s.shadow_map_near_z != near {
                s.shadow_map_near_z = near;
            }
            if let (true, Some((d, n))) = (cast, bias) {
                if (s.shadow_depth_bias, s.shadow_normal_bias) != (d, n) {
                    (s.shadow_depth_bias, s.shadow_normal_bias) = (d, n);
                }
            }
        }
        match (cast, casting) {
            (true, false) => {
                commands.entity(e).insert((CastingLight, casting_layers()));
            }
            (false, true) => {
                commands.entity(e).remove::<(CastingLight, RenderLayers)>();
            }
            _ => {}
        }
    }
    if out.slots != state.slots {
        out.slots.clone_from(&state.slots);
    }
    if out.point_capacity != state.capacity {
        out.point_capacity = state.capacity;
    }
    removed.clear();

    let now = (settings.choice, points, spots);
    if *logged != Some(now) {
        *logged = Some(now);
        match tune {
            None => info!("light shadows: off"),
            Some(t) => info!(
                "light shadows: {} cast ({points} point, {spots} spot lights) within {reach:.0} units, {} at most; cubes {}x{}, room for {}",
                points + spots,
                t.lights,
                cube.size,
                cube.size,
                state.capacity
            ),
        }
    }
    // A summary of what was drawn, every 10 s of real time.
    if tune.is_some() {
        state.frames += 1;
        state.since += time.delta_secs();
        if state.since >= 10.0 {
            let per = |n: u64| n as f32 / state.frames as f32;
            info!(
                "light shadows: {:.2} point lights switched on and {:.1} faces drawn a frame over {} frames ({} lights casting); marked a frame by shapes {:.2}, skeletons {:.2}, removals {:.2}, lights {:.2}; shapes such as {}",
                per(state.switched),
                per(state.drawn),
                state.frames,
                state.lamps.len(),
                per(state.causes[0]),
                per(state.causes[1]),
                per(state.causes[2]),
                per(state.causes[3]),
                if state.names.is_empty() { "none".to_string() } else { state.names.join(", ") }
            );
            (state.switched, state.drawn, state.frames, state.since, state.causes) = (0, 0, 0, 0.0, [0; 4]);
            state.names.clear();
            // Forget shapes and skeletons that have not marked for a while.
            let old = frame.saturating_sub(4800);
            state.shapes_at.retain(|_, (_, f)| *f >= old);
            state.skeletons_at.retain(|_, (_, _, f)| *f >= old);
        }
    }
}

type CasterLists<'a> = (Entity, &'a GlobalTransform, Option<&'a mut CubemapVisibleEntities>, Option<&'a mut VisibleMeshEntities>);

type Shapes<'a> = (&'a GlobalTransform, &'a Aabb, Has<LightFixture>, Option<&'a nvf_play::RefVisual>);

/// Take the light fixtures out of the shapes Bevy found for each casting light's maps this
/// frame: every shape marked [`LightFixture`], and a small shape the light sits in or
/// against ([`FIXTURE_MARGIN`], [`FIXTURE_RADIUS`]). Runs after Bevy's culling of the
/// lights' maps; only the lights drawing this frame have shapes listed.
fn skip_fixtures(mut lights: Query<CasterLists, With<CastingLight>>, shapes: Query<Shapes>, mut logged: Local<bevy::ecs::entity::EntityHashSet>) {
    let log = log_fixtures();
    for (light, t, cube, spot) in &mut lights {
        let at = t.translation();
        let mut lists: Vec<&mut Vec<Entity>> = Vec::with_capacity(6);
        if let Some(cube) = cube {
            lists.extend(cube.into_inner().iter_mut().map(|v| &mut v.entities));
        }
        if let Some(spot) = spot {
            lists.push(&mut spot.into_inner().entities);
        }
        // For the test aid: the shapes skipped, and the small ones kept near the light.
        let (mut skipped, mut kept): (Vec<Entity>, Vec<Entity>) = (Vec::new(), Vec::new());
        for list in lists {
            list.retain(|e| {
                let Ok((st, aabb, marked, _)) = shapes.get(*e) else { return true };
                let gap = if marked { None } else { fixture_gap(at, &st.affine(), aabb) };
                let skip = marked || gap.is_some_and(|g| g <= FIXTURE_MARGIN);
                if log {
                    if skip && !skipped.contains(e) {
                        skipped.push(*e);
                    } else if !skip && gap.is_some_and(|g| g <= 4.0 * FIXTURE_MARGIN) && !kept.contains(e) {
                        kept.push(*e);
                    }
                }
                !skip
            });
        }
        if log && (!skipped.is_empty() || !kept.is_empty()) && logged.insert(light) {
            // Each by its reference, whether it is marked, its size and its gap to the light.
            let shown = |v: &[Entity]| {
                let all: Vec<String> = v
                    .iter()
                    .filter_map(|e| shapes.get(*e).ok())
                    .map(|(st, aabb, marked, r)| {
                        let size = Vec3::from(aabb.half_extents) * 2.0 * st.compute_transform().scale.abs();
                        let gap = fixture_gap(at, &st.affine(), aabb).map_or("large".to_string(), |g| format!("{g:.0}"));
                        format!("{:08X}{} {:.0}x{:.0}x{:.0} gap {gap}", r.map_or(0, |r| r.0), if marked { " fixture" } else { "" }, size.x, size.y, size.z)
                    })
                    .collect();
                if all.is_empty() { "none".to_string() } else { all.join(", ") }
            };
            info!("light shadows: light {light} at {:.0} {:.0} {:.0} skips {}; small shapes near it kept: {}", at.x, -at.z, at.y, shown(&skipped), shown(&kept));
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_choices_are_the_sun_shadows_rows_and_each_on_choice_is_tuned() {
        assert_eq!(CHOICES, crate::shadow_rate::SUN_SHADOW_CHOICES);
        assert_eq!(tuning(0), None);
        let (p, q) = (tuning(1).unwrap(), tuning(2).unwrap());
        assert!(q.lights > p.lights && q.draws > p.draws, "Quality draws more");
        assert!(q.near_every <= p.near_every && q.far_every <= p.far_every, "and as often or more");
        assert_eq!(LightShadows::of(LightShadowMode::Quality).choice, 2);
        assert_eq!(LightShadows::of(LightShadowMode::Off).choice, 0);
    }

    #[test]
    fn lights_are_picked_by_how_near_their_light_reaches_within_the_shadow_distance() {
        // Inside a light (edge distance 0) before one whose light ends 100 units away.
        let mut c = vec![(1, rank(500.0, 400.0, false)), (2, rank(300.0, 400.0, false)), (3, rank(5000.0, 400.0, false))];
        assert_eq!(pick(&mut c, 8000.0, 8), vec![2, 1, 3]);
        assert_eq!(pick(&mut c, 1000.0, 8), vec![2, 1], "beyond the shadow distance: none");
        assert_eq!(pick(&mut c, 8000.0, 1), vec![2], "the cap");
        // A casting light keeps its place against one KEEP times nearer.
        let mut c = vec![(1, rank(1200.0, 400.0, true)), (2, rank(1050.0, 400.0, false))];
        assert_eq!(pick(&mut c, 8000.0, 1), vec![1]);
        let mut c = vec![(1, rank(1200.0, 400.0, true)), (2, rank(700.0, 400.0, false))];
        assert_eq!(pick(&mut c, 8000.0, 1), vec![2]);
    }

    #[test]
    fn slots_are_the_lowest_free_and_cubes_follow_the_shadow_resolution() {
        assert_eq!(free_slot([].into_iter()), 0);
        assert_eq!(free_slot([0, 1, 3].into_iter()), 2);
        assert_eq!(free_slot([3, 1, 0, 2].into_iter()), 4);
        assert_eq!(free_slot([1, 2].into_iter()), 0);
        assert_eq!(cube_side(2048), 512);
        assert_eq!(cube_side(4096), 1024);
    }

    #[test]
    fn turns_are_staggered_by_phase() {
        let every = 4;
        let frames: Vec<u64> = (0..12).filter(|&f| its_turn(f, 1, every)).collect();
        assert_eq!(frames, vec![3, 7, 11]);
        assert!((0..8).all(|f| its_turn(f, 5, 1)), "every frame");
        let a: Vec<bool> = (0..4).map(|f| its_turn(f, 0, 2)).collect();
        let b: Vec<bool> = (0..4).map(|f| its_turn(f, 1, 2)).collect();
        assert_ne!(a, b, "two lights on different frames");
    }

    #[test]
    fn a_sphere_dirties_the_faces_it_reaches() {
        let light = Vec3::ZERO;
        let sphere = |c: Vec3, r: f32| Sphere { center: c.into(), radius: r };
        assert!(reaches(light, 400.0, &sphere(Vec3::new(450.0, 0.0, 0.0), 60.0)));
        assert!(!reaches(light, 400.0, &sphere(Vec3::new(500.0, 0.0, 0.0), 60.0)));
        // Without frusta (a light that never drew) every face counts.
        assert_eq!(faces_reached(&sphere(Vec3::X * 100.0, 10.0), None, None), ALL_FACES);
    }

    #[test]
    fn lamp_models_are_fixtures_by_their_file_name() {
        for m in [
            r"meshes\dungeons\office\lights\offrmlighthanging01.nif",
            r"meshes\architecture\megaton\interior\shackinteriors\shackhanginglight03.nif",
            r"meshes\clutter\lights\LampLightLamp.nif",
            r"meshes\architecture\strip\neoclassicalsconce.nif",
            "meshes/clutter/lantern01.nif",
        ] {
            assert!(is_fixture_model(m), "{m}");
        }
        for m in [r"meshes\dungeons\office\lights\..\offrmwall01.nif", r"meshes\clutter\office\rubberfloormatlrg01.nif", r"meshes\light\shelf01.nif"] {
            assert!(!is_fixture_model(m), "{m}: only the file name counts");
        }
    }

    #[test]
    fn a_small_shape_the_light_sits_in_or_against_is_its_fixture_a_wall_never() {
        use bevy::math::{Affine3A, Quat};
        let is_fixture = |light: Vec3, to_world: &Affine3A, aabb: &Aabb| fixture_gap(light, to_world, aabb).is_some_and(|g| g <= FIXTURE_MARGIN);
        // A hanging box lamp 100 x 40 x 20 units, its middle 30 units over the floor point.
        let lamp = Aabb::from_min_max(Vec3::new(-50.0, -20.0, -10.0), Vec3::new(50.0, 20.0, 10.0));
        let at = Affine3A::from_translation(Vec3::new(0.0, 0.0, 30.0));
        assert!(is_fixture(Vec3::new(0.0, 0.0, 30.0), &at, &lamp), "inside");
        assert!(is_fixture(Vec3::new(0.0, 0.0, 8.0), &at, &lamp), "12 units under it");
        assert!(!is_fixture(Vec3::new(0.0, 0.0, -10.0), &at, &lamp), "30 units under it");
        assert!(!is_fixture(Vec3::new(80.0, 0.0, 30.0), &at, &lamp), "30 units beside it");
        // Turned a quarter about z and scaled 2: its long side runs along y, 200 units.
        let turned = Affine3A::from_scale_rotation_translation(Vec3::splat(2.0), Quat::from_rotation_z(std::f32::consts::FRAC_PI_2), Vec3::ZERO);
        let small = Aabb::from_min_max(Vec3::new(-25.0, -10.0, -5.0), Vec3::new(25.0, 10.0, 5.0));
        assert!(is_fixture(Vec3::new(0.0, 60.0, 0.0), &turned, &small), "along its long side");
        assert!(!is_fixture(Vec3::new(60.0, 0.0, 0.0), &turned, &small), "beyond its short side");
        // A wall piece 256 x 256 x 16 with the light 4 units off it: too large to be a fixture.
        let wall = Aabb::from_min_max(Vec3::new(-128.0, -8.0, -128.0), Vec3::new(128.0, 8.0, 128.0));
        assert_eq!(fixture_gap(Vec3::new(0.0, 12.0, 0.0), &Affine3A::IDENTITY, &wall), None);
        assert!(!is_fixture(Vec3::new(0.0, 12.0, 0.0), &Affine3A::IDENTITY, &wall));
    }
}
