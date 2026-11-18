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
//! - **Held maps, as the sun's held cascades.** A point light's cube, or a spot light's
//!   map, is drawn when it starts casting and then kept (`bevy::pbr::HoldLightShadowMaps`, a
//!   patched bevy_pbr: engine/vendor/README.md "held light shadow maps"). Between draws the
//!   light's maps are switched off for Bevy (`shadow_maps_enabled` false: no views, no
//!   culling, nothing queued, which is what a shadowed point light costs, about 1 ms a frame
//!   each on the CPU, measured), and the patched renderer still samples them. Each light
//!   keeps a slot while it casts, so others coming and going do not move it: a point light
//!   a cube of the cube texture, a spot light a layer of the sun's texture past its
//!   cascades. Each texture keeps room for its slots, grown in fours ([`grown_room`]; the
//!   spot lights' also for the other shadowed spot lights, the flashlight,
//!   [`MapTextureInputs`]). A texture the renderer makes anew (the room grew, the Shadow
//!   resolution row, the sun's cascades changed) holds no light's maps: its lights draw
//!   again as if new.
//! - **Redrawn where something changed.** A face is marked when, since it was drawn, a
//!   shadow-casting shape moved, appeared or was shown or hidden with its bounds in the
//!   face, an actor's skeleton moved there, or the light itself moved more than
//!   [`LIGHT_MOVE_EPS`] from where its maps were last marked or drawn in full (all its
//!   faces then); something removed
//!   (a taken item) marks the lights around the camera in full; every light is redrawn in
//!   full every [`Tuning::refresh`] frames anyway, each at its own time.
//! - **Staggered, as the sun's cascades.** A light with marked faces is switched on to
//!   draw them on every [`Tuning::near_every`] frame when its light reaches within [`NEAR`]
//!   of the camera, every [`Tuning::far_every`] frame otherwise (each light on frames of
//!   its own), and no more than [`Tuning::draws`] lights a frame, those that never drew
//!   first, then the nearest; the rest wait, and so a moving thing's lamp shadow can lag a
//!   frame or more. Of a light switched on, only the marked faces are drawn; the others
//!   keep their maps. A light that has never drawn casts nothing until it has.
//! - **Other lights** ([`ShadowSource`], owner 2026-11-18: all light sources cast). A
//!   grass fire's light (grass_fire_fx.rs) and a light a script attached to a node (JIP
//!   AttachLight, nif_mirror.rs) rank and count with the placed lights. Muzzle flashes and
//!   explosions' lights (shot_fx.rs) are transient ([`SourceKind::transient`]): chosen apart
//!   from the placed lights, nearest first, when their light reaches within
//!   [`TRANSIENT_REACH`] of the camera (and the sun's shadow distance) and into the camera's
//!   view this frame ([`in_view`]: a new light was not seen last frame), up to
//!   [`Tuning::transients`], with their own [`Tuning::transient_draws`] a frame; so a
//!   flash's cube is drawn in the frame it appears, and a firefight and the lamps do not
//!   starve each other. A transient draws its cube once and holds it for the rest of its
//!   short life (redrawn as recoil moved it, a flash drew 3.5 times on average). The effects' own shapes are `NotShadowCaster` (shot_fx.rs
//!   `spawn_effect`, `spawn_playing_itself`), and the gun a flash sits at is a small shape
//!   it sits against ([`skip_fixtures`]).
//! - **Spot lights** (few among the placed ones) are held as the point lights are: switched
//!   on only on the frames their map draws (marked and their turn come, never-drawn first),
//!   counting against [`Tuning::draws`] (a transient's against [`Tuning::transient_draws`]).
//! - **Fading** ([`FADE_SECS`]). A light's shadow fades in once its maps were first drawn.
//!   A light no longer chosen (out of range, swapped at the cap) is not dropped at once: it
//!   stays held, its slot kept and nothing drawn, while its shadow fades out, and does not
//!   count against [`Tuning::lights`] or [`Tuning::transients`]; then it is dropped
//!   ([`leaving_fade`]). A light despawned is dropped at once. Transients do not fade: a
//!   muzzle flash lasts about a tenth of a second. The fade goes to the renderer in
//!   `HoldLightShadowMaps::fade`, which scales the shadow term (`mix(1, shadow, fade)`).
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
//!   4096 (`PointLightShadowMap`); a spot light draws one layer of the sun's texture (its
//!   side, so each slot of the spot room costs a layer of 2048 or 4096 texels square).
//! - The diffuse light of the display-value sum takes the shadow
//!   (`lights::display_shader`), as the flashlight's always has. The flashlight is not one
//!   of these lights: it casts whenever it is on (flashlight.rs). Nor are the lights of the
//!   menu and preview scenes (creator_view.rs, vigor_view.rs, poker_view.rs) or the sky's.
//! - Test aid `NVF_PLAY_TEST_LIGHT_SHADOW_HOLD=0`: every casting light draws every face
//!   every frame, for comparing the picture and the cost.

use bevy::camera::primitives::{Aabb, CubemapFrusta, Frustum, Sphere};
use bevy::camera::visibility::{CubemapVisibleEntities, RenderLayers, VisibilitySystems, VisibleMeshEntities};
use bevy::ecs::entity::EntityHashMap;
use bevy::light::{CascadeShadowConfig, DirectionalLightShadowMap, NotShadowCaster, PointLightShadowMap, SimulationLightSystems};
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
    /// At most this many lights (point or spot) switched on to draw in a frame.
    pub draws: usize,
    /// A marked light within [`NEAR`] of the camera draws on every this many frames.
    pub near_every: u64,
    /// A farther one on every this many frames.
    pub far_every: u64,
    /// Every light is drawn again in full every this many frames anyway.
    pub refresh: u64,
    /// At most this many transient lights (muzzle flashes, explosions: [`SourceKind`]) cast
    /// at once, nearest first; they do not count against [`Tuning::lights`].
    pub transients: usize,
    /// At most this many transient lights switched on to draw in a frame, besides
    /// [`Tuning::draws`], so a firefight does not starve the lamps nor the lamps it.
    pub transient_draws: usize,
}

/// The tuning of a choice; `None` for Off.
pub fn tuning(choice: usize) -> Option<Tuning> {
    match choice {
        1 => Some(Tuning { lights: 16, draws: 1, near_every: 2, far_every: 4, refresh: 1200, transients: 1, transient_draws: 1 }),
        2 => Some(Tuning { lights: 32, draws: 3, near_every: 1, far_every: 2, refresh: 600, transients: 2, transient_draws: 2 }),
        _ => None,
    }
}

/// Lights whose light reaches within this far of the camera count as near (game units,
/// about 21 m) [engineering choice: the room around the player].
pub const NEAR: f32 = 1500.0;

/// A light already casting keeps its place against one this many times nearer.
pub const KEEP: f32 = 1.25;

/// Shadow near plane of a placed light, game units (about 11 cm, as the flashlight's)
/// [engineering choice, 2026-11-18: 16 at first, to clear the bulbs and shades lights sit
/// in; since the fixtures are skipped (`LightFixture`, [`FIXTURE_MARGIN`]) 8 and 4 drew
/// the same pictures in the General Store and the Prospector Saloon, and a wall nearer
/// than 16 let the light through].
pub const NEAR_Z: f32 = 8.0;

/// A shape or a skeleton counts as moved for the lamps' maps once it is this far from where
/// it last marked them (game units, about 6 cm) [engineering choice, 2026-11-18: in the
/// Prospector Saloon the actors' idle skeletons and the head parts that follow them (eyes,
/// mouth, hat) marked about 116 times a frame, every lamp near an actor every frame].
pub const MOVE_EPS: f32 = 4.0;

/// A casting light counts as moved once it is this far from where its maps were last
/// marked or drawn in full (game units, about 11 cm) [engineering choice, 2026-11-18: a
/// light an animation or script sways by a few units is not drawn again each frame].
/// Transients are not redrawn at all once drawn.
pub const LIGHT_MOVE_EPS: f32 = 8.0;

/// Transient lights cast only when their light reaches within this far of the camera
/// (game units, about 43 m), and within the sun's shadow distance [engineering choice: a
/// firefight around the player; farther flashes are small on screen and gone in a tenth of
/// a second].
pub const TRANSIENT_REACH: f32 = 3000.0;

/// A light's shadow fades in over this long once its maps were first drawn, and out over
/// this long once it stops casting (real seconds) [engineering choice: no popping when a
/// light comes into range, leaves it or is swapped at the cap; short enough that a light
/// coming into range shows its shadow at once]. Transients do not fade.
pub const FADE_SECS: f32 = 0.3;

/// On a light other than a placed one that casts lamp shadows (main world): what it is.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShadowSource(pub SourceKind);

/// The lights besides the placed LIGH lights that cast lamp shadows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    /// A weapon's muzzle flash (shot_fx.rs): transient, moves with the gun.
    MuzzleFlash,
    /// An explosion's fading light (shot_fx.rs): transient, still.
    Explosion,
    /// A burning grass patch's light (grass_fire_fx.rs): ranked with the placed lights.
    GrassFire,
    /// A light a script attached to a node (JIP AttachLight, nif_mirror.rs): ranked with the
    /// placed lights, moves with its node.
    Attached,
}

impl SourceKind {
    /// Lights that live a moment and are chosen apart from the placed ones
    /// ([`Tuning::transients`], [`TRANSIENT_REACH`]).
    pub fn transient(self) -> bool {
        matches!(self, Self::MuzzleFlash | Self::Explosion)
    }
}

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
/// (game units, about 23 cm): a bulb or shade of another name the light sits
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
    /// Where the light was when its maps were last marked or drawn in full.
    at: Vec3,
    /// A muzzle flash or an explosion ([`SourceKind::transient`]).
    transient: bool,
    /// A spot light (its map is a layer of the sun's texture), not a point light.
    spot: bool,
    /// How much of its shadow it casts, 0..1 ([`FADE_SECS`]).
    fade: f32,
    /// No longer chosen: held, not drawn, while its fade falls to 0.
    leaving: bool,
}

/// The casting lights' slots and states.
#[derive(Resource, Default)]
struct LampShadowState {
    lamps: EntityHashMap<LampState>,
    /// Stable slots of the casting point lights in the cube texture.
    slots: EntityHashMap<u32>,
    /// The cube texture's room, in lights; it only grows while a mode stays on.
    capacity: u32,
    /// Stable slots of the casting spot lights past the sun's cascades in its texture, and
    /// the room kept there for them; it only grows while a mode stays on.
    spot_slots: EntityHashMap<u32>,
    spot_capacity: u32,
    /// What the cube texture (its side, its room) and the sun's texture (its side, the spot
    /// room, the cascades) were made for last frame: when either changes the renderer takes
    /// a new texture, which holds no light's maps.
    point_maps_key: Option<(usize, u32)>,
    spot_maps_key: Option<(usize, u32, usize)>,
    frame: u64,
    /// Lights switched on and faces drawn since the last summary, and frames counted.
    switched: u64,
    drawn: u64,
    /// The same for the transient lights, kept apart: those that started casting, were
    /// switched on, and their faces drawn, since the last summary.
    transients_cast: u64,
    transients_switched: u64,
    transients_drawn: u64,
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
            .init_resource::<MapTextureInputs>()
            .init_resource::<HoldLightShadowMaps>()
            .add_plugins(ExtractResourcePlugin::<HoldLightShadowMaps>::default())
            .add_systems(PostUpdate, watch_map_textures.before(shadow_lights))
            .add_systems(
                PostUpdate,
                // Before Bevy resets this frame's view visibility, so the lights' and the
                // shapes' `ViewVisibility` still say what the last frame saw.
                // After the camera's frustum is updated: a transient draws in its first frame
                // when its light reaches into this frame's view.
                shadow_lights
                    .after(TransformSystems::Propagate)
                    .after(VisibilitySystems::UpdateFrusta)
                    .before(VisibilitySystems::VisibilityPropagate)
                    .before(SimulationLightSystems::AssignLightsToClusters)
                    .before(SimulationLightSystems::UpdateLightFrusta),
            )
            .add_systems(PostUpdate, skip_fixtures.after(SimulationLightSystems::CheckLightVisibility));
    }
}

/// What else decides the size of the sun's shadow texture the spot lights' maps are in
/// (main world): the other shadowed spot lights (the flashlight), which take slots of their
/// own past the lamps' (bevy_pbr's patched `prepare_lights`), and the sun's cascades, which
/// come before them.
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq, Eq)]
struct MapTextureInputs {
    other_spots: u32,
    cascades: usize,
}

/// A directional light draws at most this many cascades (Bevy's `MAX_CASCADES_PER_LIGHT`)
/// [documented: engine/vendor/bevy_pbr/src/render/light.rs].
const MAX_CASCADES: usize = 4;

/// Count what [`MapTextureInputs`] holds. Separate from [`shadow_lights`], whose query
/// holds the lamps' `SpotLight`s mutably.
fn watch_map_textures(
    mut inputs: ResMut<MapTextureInputs>,
    spots: Query<&SpotLight, (Without<SessionLight>, Without<ShadowSource>)>,
    suns: Query<(&DirectionalLight, &CascadeShadowConfig)>,
) {
    let now = MapTextureInputs {
        other_spots: spots.iter().filter(|s| s.shadow_maps_enabled).count() as u32,
        cascades: suns.iter().filter(|(d, _)| d.shadow_maps_enabled).map(|(_, c)| c.bounds.len().min(MAX_CASCADES)).sum(),
    };
    if *inputs != now {
        *inputs = now;
    }
}

/// A light's fade after `dt` real seconds, rising (it casts) or falling (it is leaving),
/// over [`FADE_SECS`]; within 0..1.
pub fn fade_step(fade: f32, rising: bool, dt: f32) -> f32 {
    let step = dt / FADE_SECS;
    (if rising { fade + step } else { fade - step }).clamp(0.0, 1.0)
}

/// A light no longer chosen: its fade after `dt` while it is held fading out, `None` once it
/// is dropped (at once when it despawned, `alive` false; never drew; or is a transient,
/// which does not fade).
pub fn leaving_fade(fade: f32, drawn: bool, transient: bool, alive: bool, dt: f32) -> Option<f32> {
    if !alive || !drawn || transient {
        return None;
    }
    let fade = fade_step(fade, false, dt);
    (fade > 0.0).then_some(fade)
}

/// The room a texture keeps for `slots` and `others` lights besides them: `room`, grown in
/// fours until it holds the highest slot and the others.
pub fn grown_room(room: u32, slots: impl Iterator<Item = u32>, others: u32) -> u32 {
    let needed = slots.map(|s| s + 1).max().unwrap_or(0) + others;
    room.max(needed.div_ceil(4) * 4)
}

/// `e`'s slot in `slots`: the one it has, else the lowest free one.
pub fn assign_slot(slots: &mut EntityHashMap<u32>, e: Entity) -> u32 {
    if let Some(&slot) = slots.get(&e) {
        return slot;
    }
    let slot = free_slot(slots.values().copied());
    slots.insert(e, slot);
    slot
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

/// How near a transient light's edge must come to the camera to cast: [`TRANSIENT_REACH`],
/// or the sun's shadow distance when that is shorter.
pub fn transient_reach(shadow_reach: f32) -> f32 {
    shadow_reach.min(TRANSIENT_REACH)
}

/// Whether a light's sphere at `at` with `range` reaches into the camera's view (the far
/// plane left out, as Bevy's culling of lights does); without a frustum, yes. A light whose
/// sphere does is one Bevy's culling finds visible this frame (its box holds the sphere), so
/// its maps are drawn.
pub fn in_view(frustum: Option<&Frustum>, at: Vec3, range: f32) -> bool {
    frustum.is_none_or(|f| f.intersects_sphere(&Sphere { center: at.into(), radius: range }, false))
}

/// Whether a light now at `now` moved more than [`LIGHT_MOVE_EPS`] from `drawn_at`, where
/// its maps were last marked or drawn in full.
pub fn light_moved(drawn_at: Vec3, now: Vec3) -> bool {
    drawn_at.distance_squared(now) > LIGHT_MOVE_EPS * LIGHT_MOVE_EPS
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
    &'a GlobalTransform,
    &'a InheritedVisibility,
    &'a ViewVisibility,
    Option<&'a mut PointLight>,
    Option<&'a mut SpotLight>,
    Option<&'a MovingLight>,
    Option<&'a CubemapFrusta>,
    Option<&'a Frustum>,
    Has<CastingLight>,
    Option<&'a ShadowSource>,
);

/// The lights that may cast: the placed ones and the [`ShadowSource`]s.
type Casters = Or<(With<SessionLight>, With<ShadowSource>)>;

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
    transient: bool,
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
    (mut out, inputs): (ResMut<HoldLightShadowMaps>, Res<MapTextureInputs>),
    camera: Query<(&GlobalTransform, Option<&Frustum>), crate::fly::WorldCamera>,
    mut lights: Query<Placed, Casters>,
    moved: Query<MovedCasters, MovedFilter>,
    skinned: Query<Skinned, Without<NotShadowCaster>>,
    names: Query<&Name>,
    joints: Query<Ref<GlobalTransform>>,
    mut removed: RemovedComponents<Mesh3d>,
    mut logged: Local<Option<(usize, usize, usize)>>,
) {
    let tune = tuning(settings.choice).map(|t| Tuning { refresh: refresh_override().unwrap_or(t.refresh), ..t });
    let reach = sky.as_ref().map_or(0.0, |s| s.shadow_distance * s.shadow_reach.max(1.0));
    let (at, view) = camera.iter().next().map_or((None, None), |(c, f)| (Some(c.translation()), f));
    let state = &mut *state;
    state.frame += 1;
    let frame = state.frame;

    // 1. Which lights cast: the placed ones (with grass fires and attached lights) up to
    // their cap; apart from them, the transients the camera sees, up to theirs within a
    // shorter reach.
    let (chosen, transients): (Vec<Entity>, Vec<Entity>) = match (tune, at) {
        (Some(t), Some(at)) if reach > 0.0 => {
            let mut placed: Vec<(Entity, (f32, f32))> = Vec::new();
            let mut quick: Vec<(Entity, (f32, f32))> = Vec::new();
            for (e, place, vis, _, point, spot, moving, _, _, casting, source) in &lights {
                if !vis.get() {
                    continue;
                }
                let (range, intensity) = match (&point, &spot) {
                    (Some(p), _) => (p.range, p.intensity),
                    (None, Some(s)) => (s.range, s.intensity),
                    _ => continue,
                };
                if range <= 0.0 || moving.map_or(intensity, |m| m.full) <= 0.0 {
                    continue;
                }
                let r = rank(place.translation().distance(at), range, casting);
                if source.is_some_and(|s| s.0.transient()) {
                    if in_view(view, place.translation(), range) {
                        quick.push((e, r));
                    }
                } else {
                    placed.push((e, r));
                }
            }
            let transients = pick(&mut quick, transient_reach(reach), t.transients);
            let mut chosen = pick(&mut placed, reach, t.lights);
            chosen.extend_from_slice(&transients);
            (chosen, transients)
        }
        _ => (Vec::new(), Vec::new()),
    };
    if tune.is_some() {
        let side = cube_side(directional.as_ref().map_or(2048, |d| d.size));
        if cube.size != side {
            cube.size = side;
        }
    }
    // Their states and slots. A light no longer chosen is held, its slot kept and nothing
    // drawn, while its shadow fades out; then it loses them, as does a light despawned.
    let mut lamps: Vec<Lamp> = Vec::with_capacity(chosen.len());
    let mut alive: Vec<Entity> = Vec::new();
    for (e, t, _, seen, point, spot, ..) in &lights {
        if point.is_none() && spot.is_none() {
            continue;
        }
        if !chosen.contains(&e) {
            if state.lamps.contains_key(&e) {
                alive.push(e);
            }
            continue;
        }
        let is_point = point.is_some();
        let range = point.map(|p| p.range).or(spot.map(|s| s.range)).unwrap_or(0.0);
        let transient = transients.contains(&e);
        if transient && !state.lamps.contains_key(&e) {
            state.transients_cast += 1;
        }
        let entry = state.lamps.entry(e).or_insert(LampState {
            dirty: 0,
            fresh: true,
            refreshed: 0,
            phase: e.index_u32() as u64,
            at: t.translation(),
            transient,
            spot: !is_point,
            fade: 0.0,
            leaving: false,
        });
        if entry.fresh {
            // Its full redraws fall at a time of its own.
            entry.refreshed = frame.saturating_sub(entry.phase % tune.map_or(1, |t| t.refresh).max(1));
        }
        if entry.leaving {
            // Chosen again while fading out: what changed meanwhile was not marked.
            entry.leaving = false;
            entry.dirty = ALL_FACES;
        }
        assign_slot(if is_point { &mut state.slots } else { &mut state.spot_slots }, e);
        let gap = at.map_or(0.0, |a| (t.translation().distance(a) - range).max(0.0));
        // A transient was chosen only if the camera sees it this frame (a new light's
        // `ViewVisibility` is still false).
        lamps.push(Lamp { entity: e, at: t.translation(), range, spot: !is_point, seen: transient || seen.get(), gap, transient });
    }
    // The fades: rising once a light's maps were first drawn (a transient's full at once),
    // falling while it leaves (with a mode on; turned off, every light is dropped at once).
    let dt = time.delta_secs();
    state.lamps.retain(|e, s| {
        if chosen.contains(e) {
            s.fade = if s.transient {
                1.0
            } else if s.fresh {
                0.0
            } else {
                fade_step(s.fade, true, dt)
            };
            return true;
        }
        match leaving_fade(s.fade, !s.fresh, s.transient, tune.is_some() && alive.contains(e), dt) {
            Some(fade) => {
                s.fade = fade;
                s.leaving = true;
                true
            }
            None => false,
        }
    });
    state.slots.retain(|e, _| state.lamps.get(e).is_some_and(|s| !s.spot));
    state.spot_slots.retain(|e, _| state.lamps.get(e).is_some_and(|s| s.spot));
    state.capacity = match tune {
        Some(_) => grown_room(state.capacity, state.slots.values().copied(), 0),
        None => 0,
    };
    // The spot lights' room also holds the other shadowed spot lights (the flashlight), so
    // switching one on or off does not change the texture; none while no lamp spot casts.
    state.spot_capacity = match tune {
        Some(_) if !state.spot_slots.is_empty() || state.spot_capacity > 0 => grown_room(state.spot_capacity, state.spot_slots.values().copied(), inputs.other_spots),
        _ => 0,
    };
    // A texture made anew holds no light's maps: its lights draw again as if new, and those
    // leaving are dropped.
    let point_key = (cube.size, state.capacity);
    let spot_key = (directional.as_ref().map_or(2048, |d| d.size), state.spot_capacity, inputs.cascades);
    let new_point = state.point_maps_key.is_some_and(|k| k != point_key);
    let new_spot = state.spot_maps_key.is_some_and(|k| k != spot_key);
    (state.point_maps_key, state.spot_maps_key) = match tune {
        Some(_) => (Some(point_key), Some(spot_key)),
        None => (None, None),
    };
    if new_point || new_spot {
        state.lamps.retain(|_, s| {
            let made_anew = if s.spot { new_spot } else { new_point };
            if !made_anew {
                return true;
            }
            if s.leaving {
                return false;
            }
            (s.fresh, s.dirty) = (true, ALL_FACES);
            if !s.transient {
                s.fade = 0.0;
            }
            true
        });
        state.slots.retain(|e, _| state.lamps.contains_key(e));
        state.spot_slots.retain(|e, _| state.lamps.contains_key(e));
    }

    // 2. What changed in their maps since last frame.
    let layers = casting_layers();
    {
        let frusta: EntityHashMap<(Option<CubemapFrusta>, Option<Frustum>)> = lights
            .iter()
            .filter(|q| chosen.contains(&q.0))
            .map(|(e, _, _, _, _, spot, _, cube, frustum, ..)|(e, (cube.cloned(), if spot.is_some() { frustum.copied() } else { None })))
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
    // The light moved past LIGHT_MOVE_EPS, or its turn for a full redraw came.
    for l in &lamps {
        if let (Some(s), Some(tune)) = (state.lamps.get_mut(&l.entity), tune) {
            if !l.transient && (light_moved(s.at, l.at) || frame.saturating_sub(s.refreshed) >= tune.refresh) {
                s.dirty = ALL_FACES;
                s.refreshed = frame;
                s.at = l.at;
                state.causes[3] += 1;
            }
        }
    }

    // 3. Which lights draw this frame: those never drawn first, then the nearest marked ones
    // on their turn; at most `draws` lights (point or spot), and apart from them
    // `transient_draws` transient ones.
    lamps.sort_by(|a, b| b.seen.cmp(&a.seen).then(a.gap.total_cmp(&b.gap)));
    let hold_all = !holding();
    let mut order: Vec<&Lamp> = lamps.iter().collect();
    order.sort_by_key(|l| !state.lamps.get(&l.entity).is_some_and(|s| s.fresh));
    let (mut draws, mut transient_draws) = tune.map_or((0, 0), |t| (t.draws, t.transient_draws));
    let mut drawing: EntityHashMap<u8> = EntityHashMap::default();
    out.held.clear();
    for l in order {
        let Some(s) = state.lamps.get_mut(&l.entity) else { continue };
        let all = if l.spot { 1 } else { ALL_FACES };
        s.dirty &= all;
        let Some(t) = tune else { continue };
        let every = if l.gap <= NEAR { t.near_every } else { t.far_every };
        // A transient (a muzzle flash, 0.1 s; an explosion's light, still while it fades)
        // draws once and is held for the rest of its short life: a flash moved by the gun's
        // recoil was drawn 3.5 times on average, measured in the Prospector Saloon gunfight
        // [engineering choice, 2026-11-18].
        let want = if hold_all || s.fresh {
            all
        } else if l.transient {
            0
        } else if its_turn(frame, s.phase, every) {
            s.dirty
        } else {
            0
        };
        // Only a light the camera saw last frame (a transient: sees this frame) is drawn by
        // Bevy this frame.
        let budget = if l.transient { &mut transient_draws } else { &mut draws };
        let draw = if want != 0 && l.seen && (hold_all || *budget > 0) {
            if !hold_all {
                *budget -= 1;
            }
            want
        } else {
            0
        };
        if draw != 0 {
            s.dirty &= !draw;
            s.fresh = false;
            if draw == all {
                s.at = l.at;
            }
            if l.transient {
                state.transients_drawn += u64::from(draw.count_ones());
                state.transients_switched += u64::from(!l.spot);
            } else {
                state.drawn += u64::from(draw.count_ones());
                state.switched += u64::from(!l.spot);
            }
            drawing.insert(l.entity, draw);
        }
        // A light is switched on only to draw, a point light's other faces held.
        if draw != 0 {
            out.held.insert(l.entity, all & !draw);
        }
    }

    // 4. Set the lights up: shadows switched on to draw, the casters' layers, the near plane
    // and biases.
    let (near, bias) = (near_z(), biases());
    let (mut points, mut spots) = (0, 0);
    for (e, _, _, _, point, spot, _, _, _, casting, _) in &mut lights {
        let cast = chosen.contains(&e);
        // The counts logged leave the transients out, or each muzzle flash wrote a line.
        let counted = cast && !transients.contains(&e);
        if let Some(mut p) = point {
            points += usize::from(counted);
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
            spots += usize::from(counted);
            let on = cast && drawing.contains_key(&e);
            if s.shadow_maps_enabled != on {
                s.shadow_maps_enabled = on;
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
    if out.spot_slots != state.spot_slots {
        out.spot_slots.clone_from(&state.spot_slots);
    }
    if out.spot_capacity != state.spot_capacity {
        out.spot_capacity = state.spot_capacity;
    }
    // The fades below 1 (a light absent casts in full).
    out.fade.clear();
    out.fade.extend(state.lamps.iter().filter(|(_, s)| s.fade < 1.0).map(|(e, s)| (*e, s.fade)));
    removed.clear();

    let now = (settings.choice, points, spots);
    if *logged != Some(now) {
        *logged = Some(now);
        match tune {
            None => info!("light shadows: off"),
            Some(t) => info!(
                "light shadows: {} cast ({points} point, {spots} spot lights) within {reach:.0} units, {} at most; cubes {}x{}, room for {}; spot maps room for {}; transients (muzzle flashes, explosions) within {:.0} units, {} at most",
                points + spots,
                t.lights,
                cube.size,
                cube.size,
                state.capacity,
                state.spot_capacity,
                transient_reach(reach),
                t.transients
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
                "light shadows: {:.2} point lights switched on and {:.1} faces drawn a frame over {} frames ({} lights casting); marked a frame by shapes {:.2}, skeletons {:.2}, removals {:.2}, lights {:.2}; shapes such as {}; transients {} cast, {} switched on, {} faces drawn",
                per(state.switched),
                per(state.drawn),
                state.frames,
                state.lamps.values().filter(|l| !l.transient && !l.leaving).count(),
                per(state.causes[0]),
                per(state.causes[1]),
                per(state.causes[2]),
                per(state.causes[3]),
                if state.names.is_empty() { "none".to_string() } else { state.names.join(", ") },
                state.transients_cast,
                state.transients_switched,
                state.transients_drawn
            );
            (state.switched, state.drawn, state.frames, state.since, state.causes) = (0, 0, 0, 0.0, [0; 4]);
            (state.transients_cast, state.transients_switched, state.transients_drawn) = (0, 0, 0);
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
        assert!(q.transients > p.transients && q.transient_draws >= p.transient_draws, "and more transients");
        assert!(p.transients >= 1 && p.transient_draws >= 1, "Performance casts a muzzle flash too");
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
    fn transients_are_muzzle_flashes_and_explosions_picked_near_by_their_own_cap() {
        assert!(SourceKind::MuzzleFlash.transient() && SourceKind::Explosion.transient());
        assert!(!SourceKind::GrassFire.transient() && !SourceKind::Attached.transient(), "ranked with the placed lights");
        // The reach: TRANSIENT_REACH, or the sun's shadow distance when shorter.
        assert_eq!(transient_reach(8000.0), TRANSIENT_REACH);
        assert_eq!(transient_reach(2000.0), 2000.0);
        let reach = transient_reach(8000.0);
        // Three flashes and an explosion (range 300 and 800): nearest edge first, the cap,
        // and none whose light ends beyond the reach.
        let mut c = vec![
            (1, rank(2000.0, 300.0, false)),
            (2, rank(600.0, 300.0, false)),
            (3, rank(3500.0, 800.0, false)),
            (4, rank(3400.0, 300.0, false)),
        ];
        assert_eq!(pick(&mut c, reach, 8), vec![2, 1, 3], "the explosion's light reaches 2,700 units: in; the far flash's 3,100: out");
        assert_eq!(pick(&mut c, reach, 2), vec![2, 1], "Quality");
        assert_eq!(pick(&mut c, reach, 1), vec![2], "Performance");
        assert_eq!(pick(&mut c, transient_reach(1500.0), 8), vec![2], "the sun's shadow distance shorter");
    }

    #[test]
    fn a_transient_casts_when_its_light_reaches_into_the_view() {
        // A camera at the origin looking down -z, 90 degrees wide, near plane 10.
        use bevy::camera::CameraProjection;
        let lens = bevy::camera::PerspectiveProjection { fov: std::f32::consts::FRAC_PI_2, aspect_ratio: 1.0, near: 10.0, ..default() };
        let clip = lens.get_clip_from_view();
        let view = Frustum(bevy::shape::ViewFrustum::from_clip_from_world(&clip));
        assert!(in_view(Some(&view), Vec3::new(0.0, 0.0, -1000.0), 100.0), "ahead");
        assert!(!in_view(Some(&view), Vec3::new(0.0, 0.0, 1000.0), 100.0), "behind");
        assert!(in_view(Some(&view), Vec3::new(0.0, 0.0, 1000.0), 1200.0), "behind, its light reaching past the camera");
        assert!(!in_view(Some(&view), Vec3::new(2000.0, 0.0, -100.0), 300.0), "off to the side");
        assert!(in_view(Some(&view), Vec3::new(0.0, 0.0, -1.0e6), 10.0), "no far plane");
        assert!(in_view(None, Vec3::new(0.0, 0.0, 1000.0), 1.0), "no frustum: the distance alone");
    }

    #[test]
    fn a_light_marks_its_maps_in_full_only_once_it_moved_past_the_threshold() {
        let drawn = Vec3::new(100.0, 50.0, 0.0);
        assert!(!light_moved(drawn, drawn));
        assert!(!light_moved(drawn, drawn + Vec3::X * (LIGHT_MOVE_EPS - 0.5)), "a gun's sway");
        assert!(!light_moved(drawn, drawn + Vec3::new(5.0, 5.0, 0.0)), "7.1 units, diagonal");
        assert!(light_moved(drawn, drawn + Vec3::new(6.0, 6.0, 0.0)), "8.5 units");
        assert!(light_moved(drawn, drawn - Vec3::Y * 40.0), "a running shooter's tenth of a second");
        // Small moves add up: measured from where it was drawn, not from last frame.
        let path: Vec<Vec3> = (1..=5).map(|i| drawn + Vec3::X * 3.0 * i as f32).collect();
        let first = path.iter().position(|p| light_moved(drawn, *p));
        assert_eq!(first, Some(2), "9 units after three steps of 3");
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
    fn a_shadow_fades_in_and_out_over_the_fade_time() {
        let half = FADE_SECS * 0.5;
        assert!((fade_step(0.0, true, half) - 0.5).abs() < 1e-5);
        assert_eq!(fade_step(0.9, true, half), 1.0, "no more than full");
        assert!((fade_step(1.0, false, half) - 0.5).abs() < 1e-5);
        assert_eq!(fade_step(0.2, false, half), 0.0, "no less than none");
        // At 60 frames a second it takes the fade time to come in full.
        let mut fade = 0.0;
        let frames = (0..100).position(|_| {
            fade = fade_step(fade, true, 1.0 / 60.0);
            fade >= 1.0
        });
        let expected = (FADE_SECS * 60.0).round() as usize;
        assert!(frames.is_some_and(|f| (f + 1).abs_diff(expected) <= 1), "{frames:?} frames, {expected} expected (a frame either way: rounding)");
    }

    #[test]
    fn a_light_no_longer_chosen_is_kept_while_its_shadow_fades_out() {
        let dt = 1.0 / 60.0;
        // A drawn lamp, alive: kept, fading, until its fade reaches 0.
        let mut fade = 1.0;
        let mut frames = 0usize;
        while let Some(f) = leaving_fade(fade, true, false, true, dt) {
            assert!(f < fade, "it falls");
            fade = f;
            frames += 1;
        }
        let expected = (FADE_SECS * 60.0).round() as usize;
        assert!((frames + 1).abs_diff(expected) <= 1, "dropped once at 0: {frames} frames kept, {expected} expected");
        // Half faded in when it left: half the time.
        let mut fade = 0.5;
        let mut half = 0;
        while let Some(f) = leaving_fade(fade, true, false, true, dt) {
            fade = f;
            half += 1;
        }
        assert!(half <= frames / 2 + 1, "{half} frames from half, {frames} from full");
        // Dropped at once: despawned, never drawn, a transient.
        assert_eq!(leaving_fade(1.0, true, false, false, dt), None, "despawned");
        assert_eq!(leaving_fade(0.0, false, false, true, dt), None, "never drew");
        assert_eq!(leaving_fade(1.0, true, true, true, dt), None, "a muzzle flash");
    }

    #[test]
    fn spot_lights_keep_stable_slots_and_their_room_grows_in_fours() {
        let e = |n: u32| Entity::from_raw_u32(n).unwrap();
        let mut slots: EntityHashMap<u32> = EntityHashMap::default();
        assert_eq!(assign_slot(&mut slots, e(10)), 0);
        assert_eq!(assign_slot(&mut slots, e(11)), 1);
        assert_eq!(assign_slot(&mut slots, e(12)), 2);
        assert_eq!(assign_slot(&mut slots, e(10)), 0, "kept");
        // One leaves: the others keep theirs, a new one takes the freed slot.
        slots.remove(&e(10));
        assert_eq!(assign_slot(&mut slots, e(13)), 0);
        assert_eq!((slots[&e(11)], slots[&e(12)]), (1, 2));
        // The room: in fours, never shrinking, with the other shadowed spot lights.
        assert_eq!(grown_room(0, std::iter::empty(), 0), 0);
        assert_eq!(grown_room(0, [0, 1, 2].into_iter(), 0), 4);
        assert_eq!(grown_room(0, [0, 1, 2].into_iter(), 1), 4, "and the flashlight");
        assert_eq!(grown_room(0, [0, 1, 2].into_iter(), 2), 8);
        assert_eq!(grown_room(8, [0].into_iter(), 0), 8, "never shrinks");
        assert_eq!(grown_room(4, [4].into_iter(), 0), 8);
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
