//! Distant lights at night (`distant-lights`, docs/design/graphics-enhanced.md §3.17,
//! #3013): an enhanced-look switch of NVForever's own, off by default and in both presets,
//! never a parity item. Nothing here is a fact about the original game; every number is an
//! engineering choice made by reasoning, still to be judged by eye [design].
//!
//! Placed lights are spawned only with their cell (world.rs, lights.rs), so beyond the
//! loaded grid no lamp exists and a town at night is a dark shape until its cells load.
//! With the switch on, the LIGH references of the exterior cells within [`REACH_CELLS`] of
//! the camera's cell are drawn as small additive glows: sprites only, no light entity, no
//! shadow, no game state.
//!
//! - **Source.** When the camera's cell changes, the cells not read yet are listed on a
//!   worker thread (the read pool, as world.rs reads cells ahead): each one's own and
//!   persistent references whose base is a LIGH, enabled at the start (`world::enabled`),
//!   neither negative nor off by default, with a radius and a colour (the record's DATA,
//!   `fnv_world::lighting`; the flags as the GECK wiki "GetLightFlag" lists them
//!   [documented]). Kept per cell. On this thread the lights the simulation holds disabled
//!   now are left out (`SimState::present`), checked again every [`RECHECK_SECS`].
//! - **Hand-over.** A cell that is loaded draws its own lights: its glows are given
//!   1 − its fade level (`Streamer::cell_level`, fade.rs), so they dim as the cell fades in
//!   and come back as it fades out; a fully drawn cell gives none.
//! - **Budget.** Lights of a cell within [`MERGE_DISTANCE`] of each other are one glow;
//!   of all glows the [`CAP`] with the most light for their distance are drawn ([`select`]).
//! - **Drawing.** One mesh for all glows (distant_lights.wesl): camera-facing cards, sized
//!   from the light's radius ([`half_size`]) and never under [`MIN_PIXELS`] in half size,
//!   faded over the reach's last [`FADE_CELLS`] cells ([`reach_fade`]) and by the view's fog.
//!   Colours are display values, as the lit world's (lights.rs), peaking at [`PEAK`].
//! - **Daylight.** The world view's exposure only ever brightens (eye_adapt.rs), so an
//!   additive dot would show on a sunlit hillside; the glows are scaled by the sun's
//!   elevation instead ([`night_share`]) [design: a fitted choice, not the original's].
//!
//! Log: `distant lights: on (reach 12 cells, cap 400)` when the switch turns on, and at the
//! exit `distant lights summary: <cells> cells listed, <lights> lights, <drawn> drawn at
//! most` if it was ever on. Nothing with the switch off.

use crate::graphics::Graphics;
use crate::sky::SkyState;
use crate::world::{bevy_to_game, Streamer, WorldData, WorldRoot};
use bevy::asset::{embedded_asset, RenderAssetUsages};
use bevy::camera::visibility::NoFrustumCulling;
use bevy::light::{NotShadowCaster, NotShadowReceiver};
use bevy::mesh::{Indices, MeshVertexBufferLayoutRef, PrimitiveTopology};
use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError};
use bevy::shader::ShaderRef;
use bevy::tasks::{futures::check_ready, Task};
use fnv_esm::{LoadOrder, Schema as EsmSchema};
use fnv_world::lighting::{light_data, light_flags, placed_light_radius, ref_xrds, LightData};
use fnv_world::placed::placement;
use fnv_world::{cell_of, CELL_SIZE};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

type Grid = (i32, i32);

/// Cells around the camera's whose lights are drawn (closest point within this many cells)
/// [design, #3013].
pub const REACH_CELLS: i32 = 12;
/// Glows drawn at most [design: "a few hundred", #3013].
pub const CAP: usize = 400;
/// Lights of one cell closer than this (game units) are drawn as one glow [design].
pub const MERGE_DISTANCE: f32 = 192.0;
/// A glow's half size per unit of the light's radius, and its bounds (game units) [design].
pub const SIZE_PER_RADIUS: f32 = 0.25;
pub const MIN_HALF: f32 = 24.0;
pub const MAX_HALF: f32 = 160.0;
/// The smallest half size on screen, in pixels: a far light stays a pixel or two [design].
pub const MIN_PIXELS: f32 = 1.0;
/// The least light per pixel a card widened to [`MIN_PIXELS`] keeps (its own area over the
/// area drawn, held to this) [design].
pub const FAR_FLOOR: f32 = 0.35;
/// The reach's last cells, over which the glows fade out [design].
pub const FADE_CELLS: f32 = 2.0;
/// The brightest a glow's core is, as a share of its colour (display values, the world's
/// tonemapper clips at 1, lights.rs `GAME_CLIP`) [design: under 1 so a merged or adapted
/// glow keeps its colour rather than burning white].
pub const PEAK: f32 = 0.8;
/// The most strength a glow gets from merged lights [design].
pub const MERGED_MAX: f32 = 2.0;
/// Sun elevations (degrees): the glows are full at or below the first and gone at or above
/// the second [design: fitted, a choice of ours; the original lights nothing beyond its
/// loaded cells].
pub const NIGHT: (f32, f32) = (-4.0, 6.0);
/// How deep the shimmer of flickering and pulsing lights goes [design].
pub const SHIMMER: f32 = 0.15;
/// Seconds between checks of the simulation's enabled states [design].
pub const RECHECK_SECS: f32 = 2.0;

/// The reach in game units.
pub fn reach_units() -> f32 {
    REACH_CELLS as f32 * CELL_SIZE
}

/// One placed light as listed from the records (game space).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SourceLight {
    pub form_id: u32,
    pub pos: [f32; 3],
    /// Display values (bytes / 255).
    pub color: [f32; 3],
    pub radius: f32,
    /// The record's FNAM fade, held to 0..2.
    pub strength: f32,
    /// 0 steady, 1 flicker, 2 pulse (`lights::LightMotion`).
    pub motion: u8,
}

/// One glow to draw: a light or several merged.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Glow {
    pub pos: [f32; 3],
    pub color: [f32; 3],
    pub radius: f32,
    pub strength: f32,
    pub motion: u8,
    /// 0..1, keeping glows out of step.
    pub seed: f32,
}

impl Glow {
    /// How much light the glow gives, for the budget.
    fn weight(&self) -> f32 {
        self.strength * self.color.iter().copied().fold(0.0, f32::max) * half_size(self.radius)
    }
}

/// The glow a LIGH record's light gives, placed at `pos`; `None` for a light that shows
/// nothing: negative, off by default, without a radius or without a colour of its own (a
/// colour from the model is not read for distant lights) [design].
pub fn source_light(form_id: u32, pos: [f32; 3], l: &LightData, xrds: Option<f32>) -> Option<SourceLight> {
    let radius = placed_light_radius(l.radius, xrds);
    if l.flags & (light_flags::NEGATIVE | light_flags::OFF_BY_DEFAULT) != 0 || radius <= 0.0 || l.color == [0; 3] || !(l.fade > 0.0) {
        return None;
    }
    let motion = match crate::lights::LightMotion::of(l.flags) {
        None => 0,
        Some(crate::lights::LightMotion::Flicker | crate::lights::LightMotion::FlickerSlow) => 1,
        Some(_) => 2,
    };
    Some(SourceLight { form_id, pos, color: l.color.map(|b| b as f32 / 255.0), radius, strength: l.fade.min(2.0), motion })
}

/// The lights of `refs` (a cell's own and persistent references), enabled at the start.
/// Runs on a worker thread.
fn list_lights(lo: &LoadOrder, schema: &EsmSchema, refs: &[u32], bases: &mut HashMap<u32, Option<LightData>>) -> Vec<SourceLight> {
    let mut out = Vec::new();
    for &r in refs {
        let Some(p) = placement(lo, schema, r) else { continue };
        let Some(base) = p.base else { continue };
        let Some(l) = *bases.entry(base).or_insert_with(|| light_data(lo, schema, base)) else { continue };
        if !crate::world::enabled(lo, schema, &p, 0) {
            continue;
        }
        if let Some(s) = source_light(p.form_id, p.position, &l, ref_xrds(lo, schema, r)) {
            out.push(s);
        }
    }
    out
}

/// One cell's lights as glows: each light, brightest first, joins the first glow within
/// `within` of it, else starts one. A merged glow sits at the strength-weighted mean of its
/// lights with their weighted colour, the largest radius and the summed strength (held to
/// [`MERGED_MAX`]); it shimmers if its first light does.
pub fn merge(lights: &[SourceLight], within: f32) -> Vec<Glow> {
    let mut order: Vec<&SourceLight> = lights.iter().collect();
    let w = |l: &SourceLight| l.strength * l.color.iter().copied().fold(0.0, f32::max) * half_size(l.radius);
    order.sort_by(|a, b| w(b).total_cmp(&w(a)).then(a.form_id.cmp(&b.form_id)));
    // Sums per glow: position and colour weighted by strength, the weight.
    let mut sums: Vec<(Vec3, Vec3, f32)> = Vec::new();
    let mut out: Vec<Glow> = Vec::new();
    for l in order {
        let p = Vec3::from_array(l.pos);
        let near = out.iter().position(|g| Vec3::from_array(g.pos).distance(p) < within);
        match near {
            Some(i) => {
                let s = &mut sums[i];
                s.0 += p * l.strength;
                s.1 += Vec3::from_array(l.color) * l.strength;
                s.2 += l.strength;
                let g = &mut out[i];
                g.pos = (s.0 / s.2).to_array();
                g.color = (s.1 / s.2).to_array();
                g.radius = g.radius.max(l.radius);
                g.strength = s.2.min(MERGED_MAX);
            }
            None => {
                sums.push((p * l.strength, Vec3::from_array(l.color) * l.strength, l.strength));
                out.push(Glow { pos: l.pos, color: l.color, radius: l.radius, strength: l.strength, motion: l.motion, seed: seed(l.form_id) });
            }
        }
    }
    out
}

/// A phase 0..1 from a reference's FormID.
fn seed(form_id: u32) -> f32 {
    (form_id.wrapping_mul(2_654_435_761) >> 8) as f32 / 16_777_216.0
}

/// The glows to draw for a camera at `eye` (game space): of each cell's glows (`cells`:
/// the cell, its merged glows, its fade level 0..1), cells fully drawn give none and the
/// others' glows keep 1 − the level of their strength; then the `cap` with the most light
/// for their distance, brightest first.
pub fn select<'a>(cells: impl IntoIterator<Item = (Grid, &'a [Glow], f32)>, eye: Vec3, cap: usize) -> Vec<Glow> {
    let mut all: Vec<(f32, Glow)> = Vec::new();
    for (_, glows, level) in cells {
        let keep = 1.0 - level.clamp(0.0, 1.0);
        if keep <= 0.0 {
            continue;
        }
        for g in glows {
            let g = Glow { strength: g.strength * keep, ..*g };
            let d2 = Vec3::from_array(g.pos).distance_squared(eye);
            all.push((g.weight() / (d2 + CELL_SIZE * CELL_SIZE), g));
        }
    }
    all.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.pos[0].total_cmp(&b.1.pos[0])).then(a.1.pos[1].total_cmp(&b.1.pos[1])));
    all.truncate(cap);
    all.into_iter().map(|(_, g)| g).collect()
}

/// A glow's own half size (game units) for a light of `radius`.
pub fn half_size(radius: f32) -> f32 {
    (radius * SIZE_PER_RADIUS).clamp(MIN_HALF, MAX_HALF)
}

/// The share of its light a glow keeps at distance `d`: 1, then down to 0 over the reach's
/// last [`FADE_CELLS`] cells (the shader's own copy: distant_lights.wesl).
#[cfg_attr(not(test), allow(dead_code))]
pub fn reach_fade(d: f32, reach: f32) -> f32 {
    1.0 - fnv_world::sky::smoothstep(reach - FADE_CELLS * CELL_SIZE, reach, d)
}

/// The half size drawn and the light per pixel kept for a glow of half size `own` where a
/// pixel is `pixel` units wide (the shader's own copy).
#[cfg_attr(not(test), allow(dead_code))]
pub fn drawn_size(own: f32, pixel: f32) -> (f32, f32) {
    let own = own.max(1e-3);
    let half = own.max(MIN_PIXELS * pixel);
    (half, ((own / half) * (own / half)).clamp(FAR_FLOOR, 1.0))
}

/// The share of the glows shown at a sun elevation of `elevation` degrees ([`NIGHT`]).
pub fn night_share(elevation: f32) -> f32 {
    1.0 - fnv_world::sky::smoothstep(NIGHT.0, NIGHT.1, elevation)
}

/// One mesh of camera-facing cards for `glows` (distant_lights.wesl), positions relative
/// to `origin` (game space).
pub fn glow_mesh(glows: &[Glow], origin: Vec3) -> Mesh {
    let n = glows.len();
    let (mut pos, mut corner, mut size, mut color, mut idx) = (Vec::with_capacity(4 * n), Vec::with_capacity(4 * n), Vec::with_capacity(4 * n), Vec::with_capacity(4 * n), Vec::with_capacity(6 * n));
    for g in glows {
        let at = (Vec3::from_array(g.pos) - origin).to_array();
        let first = pos.len() as u32;
        let rgb = g.color.map(|c| c * g.strength);
        for c in [[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]] {
            pos.push(at);
            corner.push(c);
            size.push([half_size(g.radius), g.motion as f32 + g.seed.clamp(0.0, 0.999)]);
            color.push([rgb[0], rgb[1], rgb[2], 1.0]);
        }
        idx.extend([first, first + 1, first + 2, first, first + 2, first + 3]);
    }
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD);
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, corner);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, size);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, color);
    mesh.insert_indices(Indices::U32(idx));
    mesh
}

/// The `GlowParams` uniform of distant_lights.wesl.
#[derive(Clone, Copy, Debug, Default, PartialEq, ShaderType)]
pub struct GlowParams {
    /// x: night share × [`PEAK`]; y, z: reach fade start and end; w: [`MIN_PIXELS`].
    pub shape: Vec4,
    /// x: [`FAR_FLOOR`]; y: [`SHIMMER`].
    pub extra: Vec4,
}

impl GlowParams {
    fn at(night: f32) -> GlowParams {
        let reach = reach_units();
        GlowParams { shape: Vec4::new(night * PEAK, reach - FADE_CELLS * CELL_SIZE, reach, MIN_PIXELS), extra: Vec4::new(FAR_FLOOR, SHIMMER, 0.0, 0.0) }
    }
}

/// The glows' material: unlit, additive, depth-tested without writing depth (Bevy's
/// transparent pass), no prepass, no shadows.
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct GlowMaterial {
    #[uniform(0)]
    pub params: GlowParams,
}

const SHADER: &str = "embedded://nvforever/distant_lights.wesl";

impl Material for GlowMaterial {
    fn vertex_shader() -> ShaderRef {
        SHADER.into()
    }
    fn fragment_shader() -> ShaderRef {
        SHADER.into()
    }
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Add
    }
    fn enable_prepass() -> bool {
        false
    }
    fn enable_shadows() -> bool {
        false
    }
    fn specialize(_: &MaterialPipeline, descriptor: &mut RenderPipelineDescriptor, _: &MeshVertexBufferLayoutRef, _: MaterialPipelineKey<Self>) -> Result<(), SpecializedMeshPipelineError> {
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

/// The lights listed for the cells in reach, and what is drawn.
#[derive(Resource, Default)]
pub struct DistantLights {
    on: bool,
    ever_on: bool,
    /// The worldspace the lists are for.
    world: Option<u32>,
    centre: Option<Grid>,
    /// Cells in reach, nearest first.
    want: Vec<Grid>,
    /// Lights per cell, enabled at the start.
    lists: HashMap<Grid, Arc<Vec<SourceLight>>>,
    /// Per cell: its glows from the lights present now, and the present lights' count.
    glows: HashMap<Grid, (Vec<Glow>, usize)>,
    /// The worker's listing of cells not read yet.
    listing: Option<Task<Vec<(Grid, Vec<SourceLight>)>>>,
    /// What the drawn mesh was built for: the centre and a digest of the cells' fade steps
    /// and present counts.
    built: Option<(Grid, u64)>,
    since_check: f32,
    entity: Option<Entity>,
    /// The glows' entity is hidden (by day).
    hidden: bool,
    material: Option<Handle<GlowMaterial>>,
    night: f32,
    stats: Stats,
}

#[derive(Debug, Default, Clone, Copy)]
struct Stats {
    cells: usize,
    lights: usize,
    most_drawn: usize,
}

impl DistantLights {
    fn clear(&mut self, commands: &mut Commands) {
        if let Some(e) = self.entity.take() {
            commands.entity(e).despawn();
        }
        self.centre = None;
        self.want.clear();
        self.built = None;
    }
}

pub struct DistantLightsPlugin;

impl Plugin for DistantLightsPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "distant_lights.wesl");
        app.add_plugins(MaterialPlugin::<GlowMaterial>::default())
            .init_resource::<DistantLights>()
            .add_systems(Update, update.after(crate::fade::fade_cells))
            .add_systems(Last, report);
    }
}

/// The digest of what the glows depend on besides the centre: each wanted cell's fade step
/// (loaded cells only) and the count of its lights present.
fn digest(want: &[Grid], streamer: &Streamer, glows: &HashMap<Grid, (Vec<Glow>, usize)>) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for g in want {
        if streamer.loaded.contains_key(g) {
            (g, crate::fade::step(streamer.cell_level(*g))).hash(&mut h);
        }
        if let Some((_, n)) = glows.get(g) {
            (g, *n).hash(&mut h);
        }
    }
    h.finish()
}

#[allow(clippy::too_many_arguments)]
fn update(
    mut commands: Commands,
    graphics: Option<Res<Graphics>>,
    world: Option<Res<WorldData>>,
    streamer: Option<Res<Streamer>>,
    sky: Option<Res<SkyState>>,
    sim: Option<Res<nvf_play::sim::SimState>>,
    time: Res<Time>,
    mut state: ResMut<DistantLights>,
    root: Query<Entity, With<WorldRoot>>,
    camera: Query<&GlobalTransform, crate::fly::WorldCamera>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<GlowMaterial>>,
) {
    let st = &mut *state;
    let wanted = graphics.as_deref().is_some_and(|g| g.distant_lights);
    if wanted != st.on {
        st.on = wanted;
        if wanted {
            st.ever_on = true;
            info!("distant lights: on (reach {REACH_CELLS} cells, cap {CAP})");
        } else {
            st.clear(&mut commands);
            info!("distant lights: off");
        }
    }
    if !wanted {
        return;
    }
    let (Some(world), Some(streamer)) = (world, streamer) else { return };
    if streamer.interior.is_some() {
        if st.entity.is_some() || st.centre.is_some() {
            st.clear(&mut commands);
        }
        return;
    }
    if st.world != Some(world.world) {
        st.clear(&mut commands);
        st.world = Some(world.world);
        st.lists.clear();
        st.glows.clear();
        st.listing = None;
    }
    let (Ok(root), Ok(cam)) = (root.single(), camera.single()) else { return };
    let eye = bevy_to_game(cam.translation());
    let c = cell_of(eye.x, eye.y);
    if st.centre != Some(c) {
        st.centre = Some(c);
        st.want = crate::treelod::cells_within(eye.truncate(), reach_units());
        let keep: HashSet<Grid> = st.want.iter().copied().collect();
        // Keep the lists of cells a little beyond the reach.
        let r = REACH_CELLS + 3;
        st.lists.retain(|g, _| (g.0 - c.0).abs() <= r && (g.1 - c.1).abs() <= r);
        st.glows.retain(|g, _| keep.contains(g));
        st.built = None;
    }
    // Take in the worker's listing; ask for the cells still missing.
    if let Some(task) = st.listing.as_mut() {
        if let Some(done) = check_ready(task) {
            for (g, lights) in done {
                st.stats.cells += 1;
                st.stats.lights += lights.len();
                st.lists.insert(g, Arc::new(lights));
            }
            st.listing = None;
        }
    }
    if st.listing.is_none() {
        let missing: Vec<Grid> = st.want.iter().filter(|g| !st.lists.contains_key(g)).copied().collect();
        if !missing.is_empty() {
            let jobs: Vec<(Grid, Vec<u32>)> = missing
                .iter()
                .map(|&g| {
                    let mut refs = world.grid_cell(g).map(|c| c.refs.clone()).unwrap_or_default();
                    refs.extend(world.persistent_by_grid.get(&g).into_iter().flatten());
                    (g, refs)
                })
                .collect();
            let (lo, schema): (&'static LoadOrder<'static>, &'static EsmSchema) = (world.lo, world.schema);
            let task = nvf_assets::read_pool::read_pool().spawn(async move {
                let mut bases = HashMap::new();
                jobs.into_iter().map(|(g, refs)| (g, list_lights(lo, schema, &refs, &mut bases))).collect::<Vec<(Grid, Vec<SourceLight>)>>()
            });
            st.listing = Some(task);
        }
    }
    // The lights present now, merged per cell: new cells at once, every cell again every
    // RECHECK_SECS (only those whose count of present lights changed are merged again).
    st.since_check += time.delta_secs();
    let recheck = st.since_check >= RECHECK_SECS;
    if recheck {
        st.since_check = 0.0;
    }
    let present = |id: u32| sim.as_deref().is_none_or(|s| s.present(id));
    for g in &st.want {
        let Some(list) = st.lists.get(g) else { continue };
        if !recheck && st.glows.contains_key(g) {
            continue;
        }
        let here: Vec<SourceLight> = list.iter().filter(|l| present(l.form_id)).copied().collect();
        if st.glows.get(g).is_some_and(|(_, n)| *n == here.len()) {
            continue;
        }
        let n = here.len();
        st.glows.insert(*g, (merge(&here, MERGE_DISTANCE), n));
    }
    // The material: made once, its night share kept current.
    let elevation = sky.as_deref().map_or(90.0, |s| s.sun_elevation());
    let night = night_share(elevation);
    let material = match &st.material {
        Some(h) => h.clone(),
        None => {
            let h = materials.add(GlowMaterial { params: GlowParams::at(night) });
            st.night = night;
            st.material = Some(h.clone());
            h
        }
    };
    if (night - st.night).abs() > 0.002 {
        st.night = night;
        if let Some(mut m) = materials.get_mut(&material) {
            m.params = GlowParams::at(night);
        }
    }
    // The mesh again when the centre, a cell's fade step or its present lights changed.
    let key = (c, digest(&st.want, &streamer, &st.glows));
    if st.built != Some(key) {
        st.built = Some(key);
        let cells = st.want.iter().filter_map(|g| st.glows.get(g).map(|(glows, _)| (*g, glows.as_slice(), streamer.cell_level(*g))));
        let chosen = select(cells, eye, CAP);
        st.stats.most_drawn = st.stats.most_drawn.max(chosen.len());
        let origin = Vec3::new(c.0 as f32 * CELL_SIZE, c.1 as f32 * CELL_SIZE, 0.0);
        if chosen.is_empty() {
            if let Some(e) = st.entity.take() {
                commands.entity(e).despawn();
            }
        } else {
            let mesh = Mesh3d(meshes.add(glow_mesh(&chosen, origin)));
            match st.entity {
                Some(e) => {
                    commands.entity(e).insert((mesh, Transform::from_translation(origin)));
                }
                None => {
                    let e = commands
                        .spawn((
                            Name::new("Distant lights"),
                            mesh,
                            MeshMaterial3d(material),
                            Transform::from_translation(origin),
                            if night > 0.0 { Visibility::Inherited } else { Visibility::Hidden },
                            NoFrustumCulling,
                            NotShadowCaster,
                            NotShadowReceiver,
                            ChildOf(root),
                        ))
                        .id();
                    st.entity = Some(e);
                    st.hidden = night <= 0.0;
                }
            }
        }
    }
    // Hidden by day (nothing to draw).
    if let Some(e) = st.entity {
        let hide = night <= 0.0;
        if hide != st.hidden {
            st.hidden = hide;
            commands.entity(e).insert(if hide { Visibility::Hidden } else { Visibility::Inherited });
        }
    }
}

fn report(mut exit: MessageReader<AppExit>, state: Res<DistantLights>) {
    if exit.read().last().is_some() && state.ever_on {
        let s = state.stats;
        info!("distant lights summary: {} cells listed, {} lights, {} drawn at most", s.cells, s.lights, s.most_drawn);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::mesh::VertexAttributeValues;

    fn light(id: u32, pos: [f32; 3], strength: f32) -> SourceLight {
        SourceLight { form_id: id, pos, color: [1.0, 0.5, 0.25], radius: 512.0, strength, motion: 0 }
    }

    fn data(flags: u32) -> LightData {
        LightData { radius: 256.0, color: [255, 128, 0], flags, falloff_exponent: 1.0, fov: 90.0, fade: 1.0 }
    }

    #[test]
    fn negative_off_dark_and_radiusless_lights_give_no_glow() {
        let s = source_light(7, [1.0, 2.0, 3.0], &data(0), Some(64.0)).expect("a plain light");
        assert_eq!((s.radius, s.color, s.motion), (320.0, [1.0, 128.0 / 255.0, 0.0], 0));
        assert!(source_light(7, [0.0; 3], &data(light_flags::NEGATIVE), None).is_none());
        assert!(source_light(7, [0.0; 3], &data(light_flags::OFF_BY_DEFAULT), None).is_none());
        assert!(source_light(7, [0.0; 3], &data(0), Some(-300.0)).is_none(), "no radius left");
        assert!(source_light(7, [0.0; 3], &LightData { color: [0; 3], ..data(0) }, None).is_none());
        assert!(source_light(7, [0.0; 3], &LightData { fade: 0.0, ..data(0) }, None).is_none());
        assert_eq!(source_light(7, [0.0; 3], &data(light_flags::FLICKER), None).unwrap().motion, 1);
        assert_eq!(source_light(7, [0.0; 3], &data(1 << 7), None).unwrap().motion, 2, "pulse");
        assert_eq!(source_light(7, [0.0; 3], &LightData { fade: 5.0, ..data(0) }, None).unwrap().strength, 2.0);
    }

    #[test]
    fn lights_close_together_merge_and_far_ones_stay_apart() {
        let lights = [light(1, [0.0, 0.0, 0.0], 1.0), light(2, [100.0, 0.0, 0.0], 1.0), light(3, [1000.0, 0.0, 0.0], 1.0)];
        let glows = merge(&lights, MERGE_DISTANCE);
        assert_eq!(glows.len(), 2);
        let pair = glows.iter().find(|g| g.pos[0] < 500.0).unwrap();
        assert_eq!(pair.pos, [50.0, 0.0, 0.0], "the mean of the two");
        assert_eq!(pair.strength, 2.0);
        // Many lamps in one place are held to MERGED_MAX.
        let crowd: Vec<SourceLight> = (0..10).map(|i| light(i, [i as f32, 0.0, 0.0], 1.0)).collect();
        let one = merge(&crowd, MERGE_DISTANCE);
        assert_eq!(one.len(), 1);
        assert_eq!(one[0].strength, MERGED_MAX);
        assert!(merge(&[], MERGE_DISTANCE).is_empty());
    }

    fn glow(x: f32, strength: f32) -> Glow {
        Glow { pos: [x, 0.0, 0.0], color: [1.0; 3], radius: 512.0, strength, motion: 0, seed: 0.0 }
    }

    #[test]
    fn the_cap_keeps_the_nearest_and_brightest() {
        let a = [glow(10_000.0, 1.0), glow(40_000.0, 1.0)];
        let b = [glow(-20_000.0, 2.0), glow(-45_000.0, 0.1)];
        let chosen = select([((2, 0), &a[..], 0.0), ((-5, 0), &b[..], 0.0)], Vec3::ZERO, 2);
        assert_eq!(chosen.len(), 2);
        assert_eq!(chosen[0].pos[0], 10_000.0, "nearest first");
        assert_eq!(chosen[1].pos[0], -20_000.0, "a brighter light further away before a dimmer nearer one");
        assert_eq!(select([((2, 0), &a[..], 0.0)], Vec3::ZERO, 10).len(), 2, "under the cap all are drawn");
    }

    #[test]
    fn loaded_cells_hand_over_by_their_fade() {
        let a = [glow(10_000.0, 1.0)];
        assert!(select([((2, 0), &a[..], 1.0)], Vec3::ZERO, CAP).is_empty(), "a fully drawn cell draws its own lights");
        let half = select([((2, 0), &a[..], 0.25)], Vec3::ZERO, CAP);
        assert_eq!(half[0].strength, 0.75, "a cell fading in dims its glows");
        assert_eq!(select([((2, 0), &a[..], 0.0)], Vec3::ZERO, CAP)[0].strength, 1.0);
    }

    #[test]
    fn sizes_and_fades() {
        assert_eq!(half_size(512.0), 128.0);
        assert_eq!(half_size(10.0), MIN_HALF);
        assert_eq!(half_size(4096.0), MAX_HALF);
        let reach = reach_units();
        assert_eq!(reach_fade(0.0, reach), 1.0);
        assert_eq!(reach_fade(reach - FADE_CELLS * CELL_SIZE, reach), 1.0);
        assert_eq!(reach_fade(reach, reach), 0.0);
        let mid = reach_fade(reach - CELL_SIZE, reach);
        assert!(mid > 0.4 && mid < 0.6, "{mid}");
        // Near: the glow's own size and all its light; far: a pixel, less light per pixel.
        assert_eq!(drawn_size(128.0, 10.0), (128.0, 1.0));
        let (h, share) = drawn_size(64.0, 128.0);
        assert_eq!((h, share), (128.0, FAR_FLOOR), "a quarter of the light per pixel, held to the floor");
        let (h, share) = drawn_size(100.0, 120.0);
        assert_eq!(h, 120.0);
        assert!((share - (100.0f32 / 120.0).powi(2)).abs() < 1e-6);
        assert_eq!(drawn_size(10.0, 1000.0).1, FAR_FLOOR);
        assert_eq!(night_share(-30.0), 1.0);
        assert_eq!(night_share(45.0), 0.0);
        let dusk = night_share(1.0);
        assert!(dusk > 0.0 && dusk < 1.0);
        let p = GlowParams::at(1.0);
        assert_eq!((p.shape.x, p.shape.z, p.extra.x), (PEAK, reach, FAR_FLOOR));
    }

    #[test]
    fn mesh_layout() {
        let glows = [Glow { motion: 2, seed: 0.5, ..glow(5000.0, 0.5) }];
        let m = glow_mesh(&glows, Vec3::new(4096.0, 0.0, 0.0));
        assert_eq!(m.count_vertices(), 4);
        assert_eq!(m.indices().unwrap().len(), 6);
        let Some(VertexAttributeValues::Float32x3(p)) = m.attribute(Mesh::ATTRIBUTE_POSITION) else { panic!() };
        assert!(p.iter().all(|v| *v == [904.0, 0.0, 0.0]));
        let Some(VertexAttributeValues::Float32x2(s)) = m.attribute(Mesh::ATTRIBUTE_UV_1) else { panic!() };
        assert_eq!(s[0], [128.0, 2.5]);
        let Some(VertexAttributeValues::Float32x4(c)) = m.attribute(Mesh::ATTRIBUTE_COLOR) else { panic!() };
        assert_eq!(c[0], [0.5, 0.5, 0.5, 1.0]);
    }

    #[test]
    fn the_switch_is_off_in_both_looks_and_has_its_own_key() {
        use crate::graphics::Preset;
        assert!(!Graphics::preset(Preset::Vanilla).distant_lights && !Graphics::preset(Preset::Enhanced).distant_lights);
        let on = Graphics::only(&["distant-lights".into()]).expect("distant-lights");
        assert!(on.distant_lights);
        assert_eq!(on.describe(), "light glow, distant lights");
        let mut g = Graphics::preset(Preset::Vanilla);
        *g.switch("distant-lights").expect("settings key") = true;
        assert_eq!(g, on);
        assert!(!Graphics::preset(Preset::Enhanced).describe().contains("distant lights"));
    }
}
