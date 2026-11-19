//! Distant lights at night (`distant-lights`, docs/design/graphics-enhanced.md §3.17,
//! #3013): an enhanced-look switch of NVForever's own, off by default and in both presets,
//! never a parity item. Nothing here is a fact about the original game; every number is an
//! engineering choice, judged by eye [design].
//!
//! Placed lights are spawned only with their cell (world.rs, lights.rs), so beyond the
//! loaded grid no lamp exists and a town at night is a dark shape until its cells load.
//! With the switch on, the LIGH references of the exterior cells within [`REACH_CELLS`] of
//! the camera's cell are drawn as small additive glows: sprites only, no light entity, no
//! shadow, no game state.
//!
//! - **Source.** When the camera's cell changes, the cells not read yet are listed on a
//!   worker thread (the read pool, as world.rs reads cells ahead), from the same references
//!   world.rs spawns a loaded cell from ([`cell_refs`]: the cell's own and the persistent
//!   ones standing in it): those whose base is a LIGH, not deleted, neither negative nor off
//!   by default, with a radius and a colour (the record's DATA, `fnv_world::lighting`; the
//!   flags as the GECK wiki "GetLightFlag" lists them [documented]). Lights disabled at the
//!   start are kept with that state. Kept per cell. Whether a light shows is the
//!   simulation's present state (`SimState::present`; without a simulation, the state at
//!   the start), checked in slices of [`CHECKS_PER_FRAME`] lights a frame, a pass every
//!   [`RECHECK_SECS`]; a cell whose set of present lights changed is merged again.
//! - **Glow statics.** Statics and moveable statics (STAT, MSTT; not the parts of static
//!   collections) whose model glows by its shapes' properties (`nvf_assets::model_glow`:
//!   added glow cards and beams, unlit shapes emitting over white, lit shapes self-lit at
//!   full emission; read on the worker once per model and kept, [`cached_glow`]) are sources too
//!   ([`static_source`]): at the glowing shapes' centre (a lamp post's bulb), of their
//!   colour, their radius × [`GLOW_RADIUS_SCALE`] standing in for a light's radius. They go
//!   through the same present state, merge, cap, night share, fog and hand-over.
//! - **Hand-over.** No glow for a cell inside the loaded grid ([`cell_share`]): its own
//!   lights draw, or will once it is loaded. A cell outside it that is still drawn (fading
//!   out) gives its glows 1 − its fade level (`Streamer::cell_level`, fade.rs).
//! - **Budget.** Lights of a cell within [`MERGE_DISTANCE`] of each other are one glow
//!   (a bucket grid, [`merge`]); of all glows the [`CAP`] with the most light for their
//!   distance are drawn ([`select`], a partial selection, not a full sort).
//! - **Drawing.** One mesh for all glows (distant_lights.wesl): camera-facing cards, sized
//!   from the light's radius ([`half_size`]) and never under [`MIN_PIXELS`] output pixels
//!   (nor one render pixel) in half size, faded over the reach's last [`FADE_CELLS`] cells
//!   ([`reach_fade`]) and by the view's fog. Colours are display values (the record's
//!   bytes / 255), peaking at [`PEAK`], added to the frame in display values as the NIF
//!   materials' additive shapes are (nvf_assets `add_display`).
//! - **Daylight.** The world view's exposure only ever brightens (eye_adapt.rs), so an
//!   additive dot would show on a sunlit hillside; the glows are scaled by the sun's
//!   elevation instead ([`night_share`]) [design: a fitted choice, not the original's]. While
//!   the share is 0 nothing is listed, checked or built, and the mesh is hidden.
//!
//! Log: `distant lights: on (reach 12 cells, cap 400)` when the switch turns on, `distant
//! lights: off` when it is turned off again, and at the exit `distant lights summary: …`
//! ([`Stats::summary`]) if it was ever on. Nothing with the switch off.

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
use fnv_world::placed::{base_object, placement};
use nvf_assets::model_glow::ModelGlow;
use fnv_world::{cell_of, CELL_SIZE};
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};

type Grid = (i32, i32);

/// Cells around the camera's whose lights are drawn (closest point within this many cells)
/// [design, #3013].
pub const REACH_CELLS: i32 = 12;
/// Glows drawn at most [design: "a few hundred", #3013].
pub const CAP: usize = 400;
/// Lights of one cell closer than this (game units) are drawn as one glow [design].
pub const MERGE_DISTANCE: f32 = 192.0;
/// A glow's half size per unit of the light's radius, and its bounds (game units) [design].
pub const SIZE_PER_RADIUS: f32 = 0.5;
pub const MIN_HALF: f32 = 24.0;
pub const MAX_HALF: f32 = 320.0;
/// The smallest half size on screen, in output pixels: a far light stays a few pixels wide
/// [design: fitted by pictures, 2026-11-18; at 1 the glows of a town 10 cells off could not
/// be told from stars].
pub const MIN_PIXELS: f32 = 2.0;
/// The smallest half size in render pixels, under an upscaler whose render size is below the
/// output's: a card under a render pixel would flicker between samples [design].
pub const MIN_RENDER_PIXELS: f32 = 1.0;
/// The least light per pixel a card widened to the pixel floor keeps (its own area over the
/// area drawn, held to this) [design: 0.85 since the glows are added in display values,
/// review round 2: a floored glow's centre pixel at about 0.5 display before fog].
pub const FAR_FLOOR: f32 = 0.85;
/// The reach's last cells, over which the glows fade out [design].
pub const FADE_CELLS: f32 = 2.0;
/// The brightest a glow's core is, as a share of its colour (display values; the sum with
/// the frame behind is held as the NIF additive shapes' is) [design: 1.0 since the glows
/// are added in display values, where 0.8 left a few-pixel glow's centre near 0.5; still to
/// fit by picture].
pub const PEAK: f32 = 1.0;
/// The most strength a glow gets from merged lights [design].
pub const MERGED_MAX: f32 = 2.0;
/// Sun elevations (degrees): the glows are full at or below the first and gone at or above
/// the second [design: fitted, a choice of ours; the original lights nothing beyond its
/// loaded cells].
pub const NIGHT: (f32, f32) = (-4.0, 6.0);
/// How deep the shimmer of flickering and pulsing lights goes [design].
pub const SHIMMER: f32 = 0.15;
/// Seconds between passes over the simulation's present states [design].
pub const RECHECK_SECS: f32 = 2.0;
/// A glow static's light reach per unit of its glowing shapes' radius, for the size and
/// the budget rules a LIGH light's radius feeds: a glow card 40 units across reads like a
/// light of radius 80 [design].
pub const GLOW_RADIUS_SCALE: f32 = 4.0;
/// Source lights checked against the simulation per frame at most (new cells first, then
/// the running pass) [design: a few hundred microseconds at worst].
pub const CHECKS_PER_FRAME: usize = 2048;

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
    /// Enabled at the start of a game (`world::enabled`): what shows without a simulation.
    pub start_enabled: bool,
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

/// Why a light reference gives no glow ([`source_light`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Skip {
    /// Negative or off by default.
    NegativeOff,
    /// No radius left (DATA radius plus XRDS at or under 0).
    NoRadius,
    /// No colour of the record's own (black; a model's colour is not read) or no fade.
    NoColour,
}

/// What the listing read and left out, for the exit summary.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Stats {
    pub cells: usize,
    /// References read in those cells.
    pub refs: usize,
    /// Lights listed, and how many of them (and of the glow statics) were disabled at the
    /// start (kept).
    pub lights: usize,
    pub disabled: usize,
    /// References of statics whose model glows (`nvf_assets::model_glow`), listed.
    pub glow_statics: usize,
    /// Static models read for their glow, and how many of them glow (from the cache).
    pub models_read: usize,
    pub glow_models: usize,
    pub deleted: usize,
    pub no_colour: usize,
    pub negative_off: usize,
    pub no_radius: usize,
    pub most_drawn: usize,
}

impl Stats {
    fn add(&mut self, o: &Stats) {
        self.cells += o.cells;
        self.refs += o.refs;
        self.lights += o.lights;
        self.disabled += o.disabled;
        self.glow_statics += o.glow_statics;
        self.deleted += o.deleted;
        self.no_colour += o.no_colour;
        self.negative_off += o.negative_off;
        self.no_radius += o.no_radius;
    }

    /// The exit line.
    pub fn summary(&self) -> String {
        format!(
            "distant lights summary: {} cells listed ({} references), {} lights, {} glow statics ({} glowing models of {} read), {} disabled at the start, {} drawn at most, skipped: {} deleted, {} no colour, {} negative/off, {} no radius",
            self.cells, self.refs, self.lights, self.glow_statics, self.glow_models, self.models_read, self.disabled, self.most_drawn, self.deleted, self.no_colour, self.negative_off, self.no_radius
        )
    }
}

/// The glow a LIGH record's light gives, placed at `pos`, or why it gives none: negative,
/// off by default, without a radius, or without a colour of its own (a colour from the
/// model is not read for distant lights) [design].
pub fn source_light(form_id: u32, pos: [f32; 3], l: &LightData, xrds: Option<f32>, start_enabled: bool) -> Result<SourceLight, Skip> {
    let radius = placed_light_radius(l.radius, xrds);
    if l.flags & (light_flags::NEGATIVE | light_flags::OFF_BY_DEFAULT) != 0 {
        return Err(Skip::NegativeOff);
    }
    if radius <= 0.0 {
        return Err(Skip::NoRadius);
    }
    if l.color == [0; 3] || !(l.fade > 0.0) {
        return Err(Skip::NoColour);
    }
    let motion = match crate::lights::LightMotion::of(l.flags) {
        None => 0,
        Some(crate::lights::LightMotion::Flicker | crate::lights::LightMotion::FlickerSlow) => 1,
        Some(_) => 2,
    };
    Ok(SourceLight { form_id, pos, color: l.color.map(|b| b as f32 / 255.0), radius, strength: l.fade.min(2.0), motion, start_enabled })
}

/// The references of exterior cell `g` as world.rs spawns them when it loads the cell
/// (`load_cell`): the cell's own, then the worldspace's persistent ones standing in it.
pub fn cell_refs(world: &WorldData, g: Grid) -> Vec<u32> {
    let mut refs = world.grid_cell(g).map(|c| c.refs.clone()).unwrap_or_default();
    refs.extend(world.persistent_by_grid.get(&g).into_iter().flatten());
    refs
}

/// What a reference's base is to the listing, read once per base.
#[derive(Debug, Clone, PartialEq)]
pub enum BaseKind {
    Light(LightData),
    /// A static or moveable static with this model (an archive path): a glow static if
    /// the model glows.
    Static(String),
    Other,
}

/// What base `base` is to the listing.
fn base_kind(lo: &LoadOrder, schema: &EsmSchema, base: u32) -> BaseKind {
    if let Some(l) = light_data(lo, schema, base) {
        return BaseKind::Light(l);
    }
    match base_object(lo, schema, base) {
        Some(b) if matches!(&b.sig, b"STAT" | b"MSTT") => b.world_model_path().map_or(BaseKind::Other, BaseKind::Static),
        _ => BaseKind::Other,
    }
}

/// The glow source of a static placed by `p` whose model glows as `g`: at the glowing
/// shapes' centre taken by the reference's transform, its radius scaled by the reference's
/// scale and [`GLOW_RADIUS_SCALE`], the model's colour, steady, strength 1.
pub fn static_source(p: &fnv_world::placed::Placement, g: &ModelGlow, start_enabled: bool) -> SourceLight {
    let at = crate::world::ref_affine(p).transform_point3(Vec3::from_array(g.centre));
    SourceLight { form_id: p.form_id, pos: at.to_array(), color: g.colour, radius: (g.radius * p.scale * GLOW_RADIUS_SCALE).max(1.0), strength: 1.0, motion: 0, start_enabled }
}

/// The glow sources of `refs` (a cell's references, [`cell_refs`]): LIGH lights and the
/// statics whose model glows (`glow`, by model path), with what was left out counted in
/// `stats`. Deleted references are left out; disabled ones are kept with their start
/// state. Runs on a worker thread.
pub fn list_lights(lo: &LoadOrder, schema: &EsmSchema, refs: &[u32], bases: &mut HashMap<u32, BaseKind>, glow: &mut impl FnMut(&str) -> Option<ModelGlow>, stats: &mut Stats) -> Vec<SourceLight> {
    let mut out = Vec::new();
    stats.refs += refs.len();
    for &r in refs {
        let Some(p) = placement(lo, schema, r) else { continue };
        let Some(base) = p.base else { continue };
        let kind = bases.entry(base).or_insert_with(|| base_kind(lo, schema, base)).clone();
        let source = match kind {
            BaseKind::Light(l) => Some(Ok(l)),
            BaseKind::Static(path) => glow(&path).map(Err),
            BaseKind::Other => None,
        };
        let Some(source) = source else { continue };
        if p.deleted() {
            stats.deleted += 1;
            continue;
        }
        let start = crate::world::enabled(lo, schema, &p, 0);
        match source {
            Ok(l) => match source_light(p.form_id, p.position, &l, ref_xrds(lo, schema, r), start) {
                Ok(s) => {
                    stats.lights += 1;
                    stats.disabled += usize::from(!start);
                    out.push(s);
                }
                Err(Skip::NegativeOff) => stats.negative_off += 1,
                Err(Skip::NoRadius) => stats.no_radius += 1,
                Err(Skip::NoColour) => stats.no_colour += 1,
            },
            Err(g) => {
                stats.glow_statics += 1;
                stats.disabled += usize::from(!start);
                out.push(static_source(&p, &g, start));
            }
        }
    }
    out
}

/// The glows of static models by model path, read once on a worker and kept for the run
/// (`None`: the model has no glowing shape or could not be read).
pub type GlowModels = Arc<Mutex<HashMap<String, Option<ModelGlow>>>>;

/// The glow of model `path` from `cache`, worked out by `read` on the first ask.
pub fn cached_glow(cache: &Mutex<HashMap<String, Option<ModelGlow>>>, path: &str, read: impl FnOnce(&str) -> Option<ModelGlow>) -> Option<ModelGlow> {
    if let Some(g) = cache.lock().ok().and_then(|c| c.get(path).copied()) {
        return g;
    }
    let g = read(path);
    if let Ok(mut c) = cache.lock() {
        c.insert(path.to_string(), g);
    }
    g
}

/// Models read and models that glow, from the cache.
fn model_counts(cache: &Mutex<HashMap<String, Option<ModelGlow>>>) -> (usize, usize) {
    cache.lock().map_or((0, 0), |c| (c.len(), c.values().filter(|g| g.is_some()).count()))
}

/// The lights of `list` that show now (`shows`), and a key of which they are: two sets of
/// the same size have different keys.
pub fn present_lights(list: &[SourceLight], shows: impl Fn(&SourceLight) -> bool) -> (Vec<SourceLight>, u64) {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    let here: Vec<SourceLight> = list.iter().filter(|l| shows(l)).copied().collect();
    here.len().hash(&mut h);
    for l in &here {
        l.form_id.hash(&mut h);
    }
    (here, h.finish())
}

/// One cell's lights as glows: each light, brightest first, joins the earliest glow within
/// `within` of it, else starts one. A merged glow sits at the strength-weighted mean of its
/// lights with their weighted colour, the largest radius and the summed strength (held to
/// [`MERGED_MAX`]); it shimmers if its first light does. Glows are kept in a grid of
/// buckets `within` wide by their current position, so a light looks at the 3 × 3 buckets
/// around it, not at every glow.
pub fn merge(lights: &[SourceLight], within: f32) -> Vec<Glow> {
    let within = within.max(1e-3);
    let mut order: Vec<&SourceLight> = lights.iter().collect();
    let w = |l: &SourceLight| l.strength * l.color.iter().copied().fold(0.0, f32::max) * half_size(l.radius);
    order.sort_by(|a, b| w(b).total_cmp(&w(a)).then(a.form_id.cmp(&b.form_id)));
    let bucket = |p: Vec3| ((p.x / within).floor() as i64, (p.y / within).floor() as i64);
    let mut buckets: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
    // Sums per glow: position and colour weighted by strength, the weight.
    let mut sums: Vec<(Vec3, Vec3, f32)> = Vec::new();
    let mut out: Vec<Glow> = Vec::new();
    for l in order {
        let p = Vec3::from_array(l.pos);
        let (bx, by) = bucket(p);
        let mut near: Option<usize> = None;
        for dx in -1..=1 {
            for dy in -1..=1 {
                for &i in buckets.get(&(bx + dx, by + dy)).into_iter().flatten() {
                    if Vec3::from_array(out[i].pos).distance(p) < within && near.is_none_or(|n| i < n) {
                        near = Some(i);
                    }
                }
            }
        }
        match near {
            Some(i) => {
                let before = bucket(Vec3::from_array(out[i].pos));
                let s = &mut sums[i];
                s.0 += p * l.strength;
                s.1 += Vec3::from_array(l.color) * l.strength;
                s.2 += l.strength;
                let g = &mut out[i];
                g.pos = (s.0 / s.2).to_array();
                g.color = (s.1 / s.2).to_array();
                g.radius = g.radius.max(l.radius);
                g.strength = s.2.min(MERGED_MAX);
                let after = bucket(Vec3::from_array(g.pos));
                if after != before {
                    if let Some(v) = buckets.get_mut(&before) {
                        v.retain(|&j| j != i);
                    }
                    buckets.entry(after).or_default().push(i);
                }
            }
            None => {
                buckets.entry((bx, by)).or_default().push(out.len());
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

/// Whether exterior cell `g` lies inside the loaded grid around `centre` (`Streamer::radius`
/// steps, the square world.rs loads): its own lights draw there, or will once it is loaded.
/// Hook for the full-detail ring (#2701, another branch): cells it draws are loaded cells
/// and hand over by their fade level, so nothing more is needed here.
pub fn in_loaded_grid(g: Grid, centre: Option<Grid>, radius: i32) -> bool {
    centre.is_some_and(|c| (g.0 - c.0).abs().max((g.1 - c.1).abs()) <= radius)
}

/// The share of their strength a cell's glows keep: none inside the loaded grid, else
/// 1 − the cell's fade level (a cell still drawn as it fades out).
pub fn cell_share(g: Grid, level: f32, centre: Option<Grid>, radius: i32) -> f32 {
    if in_loaded_grid(g, centre, radius) {
        0.0
    } else {
        1.0 - level.clamp(0.0, 1.0)
    }
}

/// The glows to draw for a camera at `eye` (game space): of each cell's glows (`cells`:
/// the cell, its merged glows, the share of strength they keep, [`cell_share`]), the `cap`
/// with the most light for their distance, brightest first. The cap is taken by a partial
/// selection and only the chosen are sorted.
pub fn select<'a>(cells: impl IntoIterator<Item = (Grid, &'a [Glow], f32)>, eye: Vec3, cap: usize) -> Vec<Glow> {
    if cap == 0 {
        return Vec::new();
    }
    let mut all: Vec<(f32, Glow)> = Vec::new();
    for (_, glows, keep) in cells {
        if keep <= 0.0 {
            continue;
        }
        for g in glows {
            let g = Glow { strength: g.strength * keep, ..*g };
            let d2 = Vec3::from_array(g.pos).distance_squared(eye);
            all.push((g.weight() / (d2 + CELL_SIZE * CELL_SIZE), g));
        }
    }
    let order = |a: &(f32, Glow), b: &(f32, Glow)| -> Ordering { b.0.total_cmp(&a.0).then(a.1.pos[0].total_cmp(&b.1.pos[0])).then(a.1.pos[1].total_cmp(&b.1.pos[1])) };
    if all.len() > cap {
        all.select_nth_unstable_by(cap - 1, order);
        all.truncate(cap);
    }
    all.sort_by(order);
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

/// The half size drawn and the light per pixel kept for a glow of half size `own` where an
/// output pixel is `pixel` units wide and a render pixel `render_pixel` (the same without
/// an upscaler; the shader's own copy).
#[cfg_attr(not(test), allow(dead_code))]
pub fn drawn_size(own: f32, pixel: f32, render_pixel: f32) -> (f32, f32) {
    let own = own.max(1e-3);
    let half = own.max(MIN_PIXELS * pixel).max(MIN_RENDER_PIXELS * render_pixel);
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
    /// x: [`FAR_FLOOR`]; y: [`SHIMMER`]; z: [`MIN_RENDER_PIXELS`].
    pub extra: Vec4,
}

impl GlowParams {
    fn at(night: f32) -> GlowParams {
        let reach = reach_units();
        GlowParams { shape: Vec4::new(night * PEAK, reach - FADE_CELLS * CELL_SIZE, reach, MIN_PIXELS), extra: Vec4::new(FAR_FLOOR, SHIMMER, MIN_RENDER_PIXELS, 0.0) }
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

/// One cell's glows from the lights present now, and the key of which those are
/// ([`present_lights`]).
struct CellGlows {
    glows: Vec<Glow>,
    key: u64,
}

/// What a listing task gives back.
type Listed = (Vec<(Grid, Vec<SourceLight>)>, Stats);

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
    /// Lights per cell (deleted ones left out).
    lists: HashMap<Grid, Arc<Vec<SourceLight>>>,
    glows: HashMap<Grid, CellGlows>,
    /// The worker's listing of cells not read yet (dropping it cancels it).
    listing: Option<Task<Listed>>,
    /// The running pass over the present states: the cells left to check.
    pass: Vec<Grid>,
    /// What the drawn mesh was built for: the centre and a digest of the cells' shares and
    /// keys.
    built: Option<(Grid, u64)>,
    since_check: f32,
    entity: Option<Entity>,
    /// The glows' entity is hidden (by day).
    hidden: bool,
    material: Option<Handle<GlowMaterial>>,
    night: f32,
    stats: Stats,
    /// The static models' glows, shared with the listing tasks.
    models: GlowModels,
}

impl DistantLights {
    /// Despawn the glows and forget what was chosen (the lists stay).
    fn clear(&mut self, commands: &mut Commands) {
        if let Some(e) = self.entity.take() {
            commands.entity(e).despawn();
        }
        self.centre = None;
        self.want.clear();
        self.pass.clear();
        self.built = None;
    }

    /// Start over for worldspace `world`: every list, glow, pass and listing of the last one
    /// goes. Returns the glows' entity to despawn.
    fn reset_for_world(&mut self, world: u32) -> Option<Entity> {
        self.world = Some(world);
        self.centre = None;
        self.want.clear();
        self.lists.clear();
        self.glows.clear();
        self.pass.clear();
        self.listing = None;
        self.built = None;
        self.entity.take()
    }

    /// Bring the cells' glows up to date with `shows`: the cells listed but not merged yet
    /// first, then the running pass, within `budget` source lights. Returns the lights
    /// checked.
    fn check_present(&mut self, shows: &impl Fn(&SourceLight) -> bool, budget: usize) -> usize {
        let mut used = 0;
        let fresh: Vec<Grid> = self.want.iter().filter(|g| self.lists.contains_key(g) && !self.glows.contains_key(g)).copied().collect();
        for g in fresh {
            if used >= budget {
                return used;
            }
            used += self.check_cell(g, shows);
        }
        while used < budget {
            let Some(g) = self.pass.pop() else { break };
            used += self.check_cell(g, shows);
        }
        used
    }

    /// Merge cell `g`'s present lights again if their set changed. Returns the lights read.
    fn check_cell(&mut self, g: Grid, shows: &impl Fn(&SourceLight) -> bool) -> usize {
        let Some(list) = self.lists.get(&g) else { return 0 };
        let (here, key) = present_lights(list, shows);
        if self.glows.get(&g).is_none_or(|c| c.key != key) {
            self.glows.insert(g, CellGlows { glows: merge(&here, MERGE_DISTANCE), key });
        }
        list.len().max(1)
    }

    /// The digest of what the glows depend on besides the centre: each wanted cell's share
    /// (as a fade step) and the key of its present lights.
    fn digest(&self, share: impl Fn(Grid) -> f32) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        for g in &self.want {
            if let Some(c) = self.glows.get(g) {
                (g, crate::fade::step(share(*g)), c.key).hash(&mut h);
            }
        }
        h.finish()
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

#[allow(clippy::too_many_arguments)]
fn update(
    mut commands: Commands,
    graphics: Option<Res<Graphics>>,
    world: Option<Res<WorldData>>,
    streamer: Option<Res<Streamer>>,
    sky: Option<Res<SkyState>>,
    sim: Option<Res<nvf_play::sim::SimState>>,
    data: Option<Res<nvf_assets::GameData>>,
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
            // An outstanding listing is cancelled (dropping the task).
            st.listing = None;
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
        if let Some(e) = st.reset_for_world(world.world) {
            commands.entity(e).despawn();
        }
    }
    // By day nothing is listed, checked or built; the mesh is hidden.
    let elevation = sky.as_deref().map_or(90.0, |s| s.sun_elevation());
    let night = night_share(elevation);
    let hide = night <= 0.0;
    if let Some(e) = st.entity {
        if hide != st.hidden {
            st.hidden = hide;
            commands.entity(e).insert(if hide { Visibility::Hidden } else { Visibility::Inherited });
        }
    }
    if hide {
        return;
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
        st.pass.retain(|g| keep.contains(g));
        st.built = None;
    }
    // Take in the worker's listing; ask for the cells still missing.
    if let Some(task) = st.listing.as_mut() {
        if let Some((done, stats)) = check_ready(task) {
            st.stats.add(&stats);
            for (g, lights) in done {
                st.lists.insert(g, Arc::new(lights));
            }
            st.listing = None;
        }
    }
    if st.listing.is_none() {
        let missing: Vec<Grid> = st.want.iter().filter(|g| !st.lists.contains_key(g)).copied().collect();
        if !missing.is_empty() {
            let jobs: Vec<(Grid, Vec<u32>)> = missing.iter().map(|&g| (g, cell_refs(&world, g))).collect();
            let (lo, schema): (&'static LoadOrder<'static>, &'static EsmSchema) = (world.lo, world.schema);
            let (data, models) = (data.as_deref().cloned(), st.models.clone());
            let task = nvf_assets::read_pool::read_pool().spawn(async move {
                let (mut bases, mut stats) = (HashMap::new(), Stats::default());
                // Without the game's files (no archives) no static is read for its glow.
                let mut glow = |path: &str| data.as_ref().and_then(|d| cached_glow(&models, path, |p| nvf_assets::model_glow::model_glow(d, p)));
                let lists = jobs
                    .into_iter()
                    .map(|(g, refs)| {
                        stats.cells += 1;
                        (g, list_lights(lo, schema, &refs, &mut bases, &mut glow, &mut stats))
                    })
                    .collect::<Vec<(Grid, Vec<SourceLight>)>>();
                (lists, stats)
            });
            st.listing = Some(task);
        }
    }
    // The lights present now: new cells at once, a pass over every cell every RECHECK_SECS,
    // both within CHECKS_PER_FRAME lights a frame.
    st.since_check += time.delta_secs();
    if st.since_check >= RECHECK_SECS && st.pass.is_empty() {
        st.since_check = 0.0;
        st.pass = st.want.iter().rev().copied().collect();
    }
    let shows = |l: &SourceLight| sim.as_deref().map_or(l.start_enabled, |s| s.present(l.form_id));
    st.check_present(&shows, CHECKS_PER_FRAME);
    // The material: made once, its night share kept current.
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
    // The mesh again when the centre, a cell's share or its present lights changed.
    let (grid, radius) = (streamer.centre, streamer.radius);
    let share = |g: Grid| cell_share(g, streamer.cell_level(g), grid, radius);
    let key = (c, st.digest(share));
    if st.built != Some(key) {
        st.built = Some(key);
        let cells = st.want.iter().filter_map(|g| st.glows.get(g).map(|cg| (*g, cg.glows.as_slice(), share(*g))));
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
                            Visibility::Inherited,
                            NoFrustumCulling,
                            NotShadowCaster,
                            NotShadowReceiver,
                            ChildOf(root),
                        ))
                        .id();
                    st.entity = Some(e);
                    st.hidden = false;
                }
            }
        }
    }
}

fn report(mut exit: MessageReader<AppExit>, state: Res<DistantLights>) {
    if exit.read().last().is_some() && state.ever_on {
        let mut s = state.stats;
        (s.models_read, s.glow_models) = model_counts(&state.models);
        info!("{}", s.summary());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::mesh::VertexAttributeValues;

    fn light(id: u32, pos: [f32; 3], strength: f32) -> SourceLight {
        SourceLight { form_id: id, pos, color: [1.0, 0.5, 0.25], radius: 512.0, strength, motion: 0, start_enabled: true }
    }

    fn data(flags: u32) -> LightData {
        LightData { radius: 256.0, color: [255, 128, 0], flags, falloff_exponent: 1.0, fov: 90.0, fade: 1.0 }
    }

    #[test]
    fn negative_off_dark_and_radiusless_lights_give_no_glow() {
        let s = source_light(7, [1.0, 2.0, 3.0], &data(0), Some(64.0), true).expect("a plain light");
        assert_eq!((s.radius, s.color, s.motion, s.start_enabled), (320.0, [1.0, 128.0 / 255.0, 0.0], 0, true));
        assert_eq!(source_light(7, [0.0; 3], &data(light_flags::NEGATIVE), None, true), Err(Skip::NegativeOff));
        assert_eq!(source_light(7, [0.0; 3], &data(light_flags::OFF_BY_DEFAULT), None, true), Err(Skip::NegativeOff));
        assert_eq!(source_light(7, [0.0; 3], &data(0), Some(-300.0), true), Err(Skip::NoRadius));
        assert_eq!(source_light(7, [0.0; 3], &LightData { color: [0; 3], ..data(0) }, None, true), Err(Skip::NoColour));
        assert_eq!(source_light(7, [0.0; 3], &LightData { fade: 0.0, ..data(0) }, None, true), Err(Skip::NoColour));
        assert_eq!(source_light(7, [0.0; 3], &data(light_flags::FLICKER), None, true).unwrap().motion, 1);
        assert_eq!(source_light(7, [0.0; 3], &data(1 << 7), None, true).unwrap().motion, 2, "pulse");
        assert_eq!(source_light(7, [0.0; 3], &LightData { fade: 5.0, ..data(0) }, None, true).unwrap().strength, 2.0);
        // Disabled at the start: listed, with that state.
        assert!(!source_light(7, [0.0; 3], &data(0), None, false).unwrap().start_enabled);
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
        // Across a bucket edge (192 / 191.9 sit in different buckets) and at negative
        // coordinates: still one glow.
        let edge = merge(&[light(1, [191.0, -5.0, 0.0], 1.0), light(2, [193.0, 5.0, 0.0], 1.0)], MERGE_DISTANCE);
        assert_eq!(edge.len(), 1);
        let negative = merge(&[light(1, [-10.0, -10.0, 0.0], 1.0), light(2, [10.0, 10.0, 0.0], 1.0)], MERGE_DISTANCE);
        assert_eq!(negative.len(), 1);
    }

    /// The bucket grid gives what the plain search over every glow gave: a line of lights
    /// 150 apart (each within reach of the glow before it as it moves) and a scatter.
    #[test]
    fn the_bucket_merge_matches_a_plain_search() {
        fn plain(lights: &[SourceLight], within: f32) -> Vec<Glow> {
            let mut order: Vec<&SourceLight> = lights.iter().collect();
            let w = |l: &SourceLight| l.strength * l.color.iter().copied().fold(0.0, f32::max) * half_size(l.radius);
            order.sort_by(|a, b| w(b).total_cmp(&w(a)).then(a.form_id.cmp(&b.form_id)));
            let mut sums: Vec<(Vec3, Vec3, f32)> = Vec::new();
            let mut out: Vec<Glow> = Vec::new();
            for l in order {
                let p = Vec3::from_array(l.pos);
                match out.iter().position(|g| Vec3::from_array(g.pos).distance(p) < within) {
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
        let line: Vec<SourceLight> = (0..20).map(|i| light(i, [i as f32 * 150.0, 0.0, 0.0], 1.0 + (i % 3) as f32 * 0.1)).collect();
        assert_eq!(merge(&line, MERGE_DISTANCE), plain(&line, MERGE_DISTANCE));
        let scatter: Vec<SourceLight> = (0..200u32)
            .map(|i| {
                let h = i.wrapping_mul(2_654_435_761);
                light(i, [(h % 4096) as f32, ((h >> 12) % 4096) as f32, 0.0], 0.5 + (h % 7) as f32 * 0.1)
            })
            .collect();
        assert_eq!(merge(&scatter, MERGE_DISTANCE), plain(&scatter, MERGE_DISTANCE));
    }

    fn glow(x: f32, strength: f32) -> Glow {
        Glow { pos: [x, 0.0, 0.0], color: [1.0; 3], radius: 512.0, strength, motion: 0, seed: 0.0 }
    }

    #[test]
    fn the_cap_keeps_the_nearest_and_brightest() {
        let a = [glow(10_000.0, 1.0), glow(40_000.0, 1.0)];
        let b = [glow(-20_000.0, 2.0), glow(-45_000.0, 0.1)];
        let chosen = select([((2, 0), &a[..], 1.0), ((-5, 0), &b[..], 1.0)], Vec3::ZERO, 2);
        assert_eq!(chosen.len(), 2);
        assert_eq!(chosen[0].pos[0], 10_000.0, "nearest first");
        assert_eq!(chosen[1].pos[0], -20_000.0, "a brighter light further away before a dimmer nearer one");
        assert_eq!(select([((2, 0), &a[..], 1.0)], Vec3::ZERO, 10).len(), 2, "under the cap all are drawn");
        assert!(select([((2, 0), &a[..], 1.0)], Vec3::ZERO, 0).is_empty());
        // The partial selection gives the same as a full sort and truncate.
        let many: Vec<Glow> = (0..500).map(|i| glow(5_000.0 + (i * 7919 % 40_000) as f32, 0.2 + (i % 5) as f32 * 0.3)).collect();
        let picked = select([((1, 0), &many[..], 1.0)], Vec3::ZERO, 40);
        let mut all: Vec<Glow> = many.clone();
        all.sort_by(|x, y| (y.weight() / (y.pos[0] * y.pos[0] + CELL_SIZE * CELL_SIZE)).total_cmp(&(x.weight() / (x.pos[0] * x.pos[0] + CELL_SIZE * CELL_SIZE))).then(x.pos[0].total_cmp(&y.pos[0])));
        assert_eq!(picked, all[..40].to_vec());
    }

    #[test]
    fn loaded_cells_hand_over_and_the_loaded_grid_gives_none() {
        let a = [glow(10_000.0, 1.0)];
        let centre = Some((0, 0));
        // Outside the grid (radius 2): as the cell's fade level leaves.
        assert_eq!(cell_share((5, 0), 0.0, centre, 2), 1.0);
        assert_eq!(cell_share((5, 0), 0.25, centre, 2), 0.75, "a cell still drawn dims its glows");
        assert_eq!(cell_share((5, 0), 1.0, centre, 2), 0.0, "a fully drawn cell draws its own lights");
        // Inside it: none, loaded or not yet (pending).
        assert_eq!(cell_share((2, -2), 0.0, centre, 2), 0.0, "a pending cell in the grid gives no glow");
        assert_eq!(cell_share((1, 0), 0.5, centre, 2), 0.0);
        assert!(in_loaded_grid((-2, 2), centre, 2) && !in_loaded_grid((3, 0), centre, 2));
        assert!(!in_loaded_grid((0, 0), None, 2), "no grid loaded: nothing inside it");
        let half = select([((5, 0), &a[..], cell_share((5, 0), 0.25, centre, 2))], Vec3::ZERO, CAP);
        assert_eq!(half[0].strength, 0.75);
        assert!(select([((1, 0), &a[..], cell_share((1, 0), 0.0, centre, 2))], Vec3::ZERO, CAP).is_empty());
    }

    /// Review #3013: one light disabled and another enabled between two checks leaves the
    /// count the same; the key and the glows still follow.
    #[test]
    fn swapping_lights_of_the_same_count_is_seen() {
        let list = [light(1, [0.0; 3], 1.0), light(2, [3000.0, 0.0, 0.0], 1.0)];
        let (a, ka) = present_lights(&list, |l| l.form_id == 1);
        let (b, kb) = present_lights(&list, |l| l.form_id == 2);
        assert_eq!((a.len(), b.len()), (1, 1));
        assert_ne!(ka, kb);
        let mut st = DistantLights { want: vec![(0, 0)], ..default() };
        st.lists.insert((0, 0), Arc::new(list.to_vec()));
        st.check_present(&|l: &SourceLight| l.form_id == 1, usize::MAX);
        let before = st.digest(|_| 1.0);
        assert_eq!(st.glows[&(0, 0)].glows[0].pos, [0.0; 3]);
        st.pass = vec![(0, 0)];
        st.check_present(&|l: &SourceLight| l.form_id == 2, usize::MAX);
        assert_eq!(st.glows[&(0, 0)].glows.len(), 1);
        assert_eq!(st.glows[&(0, 0)].glows[0].pos, [3000.0, 0.0, 0.0], "B drawn, A gone");
        assert_ne!(st.digest(|_| 1.0), before, "the mesh is rebuilt");
    }

    /// Review #3013: a light disabled at the start stays listed; without a simulation it
    /// shows its start state, and once the simulation enables it, it shows.
    #[test]
    fn a_light_disabled_at_the_start_shows_once_enabled() {
        let off = SourceLight { start_enabled: false, ..light(9, [0.0; 3], 1.0) };
        let mut st = DistantLights { want: vec![(0, 0)], ..default() };
        st.lists.insert((0, 0), Arc::new(vec![off]));
        let start = |l: &SourceLight| l.start_enabled;
        st.check_present(&start, usize::MAX);
        assert!(st.glows[&(0, 0)].glows.is_empty(), "not shown at the start");
        let mut enabled = HashSet::new();
        enabled.insert(9u32);
        st.pass = vec![(0, 0)];
        st.check_present(&|l: &SourceLight| enabled.contains(&l.form_id), usize::MAX);
        assert_eq!(st.glows[&(0, 0)].glows.len(), 1, "a script enabled it");
    }

    /// The present-state checks stop at the budget and carry on in the next frame.
    #[test]
    fn checks_are_sliced() {
        let mut st = DistantLights { want: vec![(0, 0), (1, 0), (2, 0)], ..default() };
        for g in [(0, 0), (1, 0), (2, 0)] {
            st.lists.insert(g, Arc::new((0..10).map(|i| light(i, [g.0 as f32 * 4096.0, 0.0, 0.0], 1.0)).collect()));
        }
        let all = |_: &SourceLight| true;
        assert_eq!(st.check_present(&all, 15), 20, "a cell is checked whole once started");
        assert_eq!(st.glows.len(), 2);
        st.check_present(&all, 15);
        assert_eq!(st.glows.len(), 3);
    }

    /// A worldspace change drops everything listed for the last one.
    #[test]
    fn a_world_change_clears_the_lists() {
        let mut st = DistantLights { world: Some(1), want: vec![(0, 0)], centre: Some((0, 0)), built: Some(((0, 0), 7)), pass: vec![(0, 0)], ..default() };
        st.lists.insert((0, 0), Arc::new(vec![light(1, [0.0; 3], 1.0)]));
        st.check_present(&|_: &SourceLight| true, usize::MAX);
        assert!(st.reset_for_world(2).is_none());
        assert_eq!(st.world, Some(2));
        assert!(st.lists.is_empty() && st.glows.is_empty() && st.want.is_empty() && st.pass.is_empty());
        assert!(st.centre.is_none() && st.built.is_none() && st.listing.is_none());
    }

    #[test]
    fn sizes_and_fades() {
        assert_eq!(half_size(512.0), 256.0);
        assert_eq!(half_size(10.0), MIN_HALF);
        assert_eq!(half_size(4096.0), MAX_HALF);
        let reach = reach_units();
        assert_eq!(reach_fade(0.0, reach), 1.0);
        assert_eq!(reach_fade(reach - FADE_CELLS * CELL_SIZE, reach), 1.0);
        assert_eq!(reach_fade(reach, reach), 0.0);
        let mid = reach_fade(reach - CELL_SIZE, reach);
        assert!(mid > 0.4 && mid < 0.6, "{mid}");
        // Near: the glow's own size and all its light; far: two pixels, less light per pixel.
        assert_eq!(drawn_size(128.0, 10.0, 10.0), (128.0, 1.0));
        let (h, share) = drawn_size(64.0, 128.0, 128.0);
        assert_eq!((h, share), (256.0, FAR_FLOOR), "a sixteenth of the light per pixel, held to the floor");
        let (h, share) = drawn_size(100.0, 52.0, 52.0);
        assert_eq!(h, 104.0);
        assert!((share - (100.0f32 / 104.0).powi(2)).abs() < 1e-6);
        assert_eq!(drawn_size(10.0, 1000.0, 1000.0).1, FAR_FLOOR);
        // Under an upscaler the floor is in output pixels, and never under a render pixel.
        assert_eq!(drawn_size(10.0, 100.0, 150.0).0, 200.0, "two output pixels");
        assert_eq!(drawn_size(10.0, 100.0, 300.0).0, 300.0, "one render pixel at a third of the size");
        assert_eq!(night_share(-30.0), 1.0);
        assert_eq!(night_share(45.0), 0.0);
        let dusk = night_share(1.0);
        assert!(dusk > 0.0 && dusk < 1.0);
        let p = GlowParams::at(1.0);
        assert_eq!((p.shape.x, p.shape.z, p.extra.x, p.extra.z), (PEAK, reach, FAR_FLOOR, MIN_RENDER_PIXELS));
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
        // Half size from the radius (512 × SIZE_PER_RADIUS), motion 2 + phase 0.5.
        assert_eq!(s[0], [half_size(512.0), 2.5]);
        assert_eq!(s[0][0], 256.0);
        let Some(VertexAttributeValues::Float32x4(c)) = m.attribute(Mesh::ATTRIBUTE_COLOR) else { panic!() };
        assert_eq!(c[0], [0.5, 0.5, 0.5, 1.0]);
    }

    #[test]
    fn the_summary_names_what_was_left_out() {
        let mut s = Stats { cells: 3, refs: 40, lights: 5, glow_statics: 12, disabled: 2, deleted: 1, no_colour: 4, negative_off: 6, no_radius: 7, most_drawn: 5, ..Stats::default() };
        let models: GlowModels = Arc::default();
        cached_glow(&models, "meshes\\a.nif", |_| Some(ModelGlow { centre: [0.0; 3], radius: 10.0, colour: [1.0; 3], shapes: 1 }));
        cached_glow(&models, "meshes\\b.nif", |_| None);
        (s.models_read, s.glow_models) = model_counts(&models);
        assert_eq!(
            s.summary(),
            "distant lights summary: 3 cells listed (40 references), 5 lights, 12 glow statics (1 glowing models of 2 read), 2 disabled at the start, 5 drawn at most, skipped: 1 deleted, 4 no colour, 6 negative/off, 7 no radius"
        );
    }

    /// A model's glow is read once, whatever it gave, and kept.
    #[test]
    fn model_glows_are_read_once() {
        let models: GlowModels = Arc::default();
        let reads = std::cell::Cell::new(0);
        let glow = ModelGlow { centre: [0.0, 0.0, 300.0], radius: 10.0, colour: [1.0, 0.8, 0.5], shapes: 1 };
        for _ in 0..3 {
            assert_eq!(cached_glow(&models, "meshes\\lamp.nif", |_| { reads.set(reads.get() + 1); Some(glow) }), Some(glow));
            assert_eq!(cached_glow(&models, "meshes\\rock.nif", |_| { reads.set(reads.get() + 1); None }), None);
        }
        assert_eq!(reads.get(), 2);
        assert_eq!(model_counts(&models), (2, 1));
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

    /// The listing on a synthetic plugin (no game data): a worldspace with a temporary light
    /// in cell (0, 0), a persistent one standing in cell (1, 0), one disabled at the start,
    /// one deleted, one off by default and a static.
    mod listing {
        use super::*;
        use fnv_esm::raw::{write_subrecords, Subrecord};
        use fnv_esm::{Plugin, Schema};
        use fnv_world::placed::ref_flags;
        use fnv_world::WorldIndex;

        const LAMP: u32 = 0x100;
        const OFF_LAMP: u32 = 0x101;
        const ROCK: u32 = 0x102;
        const WRLD: u32 = 0x200;
        const PERSISTENT_CELL: u32 = 0x201;
        const CELL_A: u32 = 0x202;
        const CELL_B: u32 = 0x203;
        const TEMP_REF: u32 = 0x301;
        const PERSISTENT_REF: u32 = 0x302;
        const DORMANT_REF: u32 = 0x303;
        const DELETED_REF: u32 = 0x304;
        const OFF_REF: u32 = 0x305;
        const ROCK_REF: u32 = 0x306;
        const POST: u32 = 0x103;
        const BOULDER: u32 = 0x104;
        const POST_REF: u32 = 0x307;
        const BOULDER_REF: u32 = 0x308;

        fn sub(sig: &[u8; 4], data: Vec<u8>) -> Subrecord {
            Subrecord { sig: *sig, data, extended: false }
        }

        fn record(sig: &[u8; 4], flags: u32, fid: u32, subs: &[Subrecord]) -> Vec<u8> {
            let payload = write_subrecords(subs);
            [sig.as_slice(), &(payload.len() as u32).to_le_bytes(), &flags.to_le_bytes(), &fid.to_le_bytes(), &[0; 4], &15u16.to_le_bytes(), &[0; 2], &payload].concat()
        }

        fn group(label: [u8; 4], gtype: i32, body: &[u8]) -> Vec<u8> {
            [b"GRUP".as_slice(), &((24 + body.len()) as u32).to_le_bytes(), &label, &gtype.to_le_bytes(), &[0; 8], body].concat()
        }

        /// LIGH DATA: time, radius, colour, flags, falloff exponent, fov, value, weight.
        fn ligh(fid: u32, flags: u32) -> Vec<u8> {
            let data = [0i32.to_le_bytes().as_slice(), &300u32.to_le_bytes(), &[255, 200, 100, 0], &flags.to_le_bytes(), &1f32.to_le_bytes(), &90f32.to_le_bytes(), &0u32.to_le_bytes(), &0f32.to_le_bytes()].concat();
            record(b"LIGH", 0, fid, &[sub(b"DATA", data), sub(b"FNAM", 1f32.to_le_bytes().to_vec())])
        }

        fn refr(fid: u32, flags: u32, base: u32, pos: [f32; 3]) -> Vec<u8> {
            let data: Vec<u8> = [pos[0], pos[1], pos[2], 0.0, 0.0, 0.0].iter().flat_map(|f| f.to_le_bytes()).collect();
            record(b"REFR", flags, fid, &[sub(b"NAME", base.to_le_bytes().to_vec()), sub(b"DATA", data)])
        }

        fn cell(fid: u32, grid: Option<(i32, i32)>) -> Vec<u8> {
            let mut subs = vec![sub(b"DATA", vec![0])];
            if let Some((x, y)) = grid {
                subs.push(sub(b"XCLC", [x.to_le_bytes(), y.to_le_bytes(), 0u32.to_le_bytes()].concat()));
            }
            record(b"CELL", 0, fid, &subs)
        }

        fn world_data() -> WorldData {
            let hedr = [1.34f32.to_le_bytes(), 0i32.to_le_bytes(), 0x800u32.to_le_bytes()].concat();
            let tes4 = record(b"TES4", 0, 0, &[sub(b"HEDR", hedr)]);
            let lights = group(*b"LIGH", 0, &[ligh(LAMP, 0), ligh(OFF_LAMP, light_flags::OFF_BY_DEFAULT)].concat());
            let stats = [
                record(b"STAT", 0, ROCK, &[sub(b"EDID", b"Rock\0".to_vec())]),
                record(b"STAT", 0, POST, &[sub(b"EDID", b"LampPost\0".to_vec()), sub(b"MODL", b"Lamp.nif\0".to_vec())]),
                record(b"STAT", 0, BOULDER, &[sub(b"EDID", b"Boulder\0".to_vec()), sub(b"MODL", b"Boulder.nif\0".to_vec())]),
            ];
            let stat = group(*b"STAT", 0, &stats.concat());
            let wrld = record(b"WRLD", 0, WRLD, &[sub(b"EDID", b"TestWorld\0".to_vec())]);
            // The persistent cell (directly in the world children), its persistent light
            // standing in grid square (1, 0).
            let persistent = [cell(PERSISTENT_CELL, Some((0, 0))), group(PERSISTENT_CELL.to_le_bytes(), 6, &group(PERSISTENT_CELL.to_le_bytes(), 8, &refr(PERSISTENT_REF, ref_flags::PERSISTENT, LAMP, [5000.0, 100.0, 0.0])))].concat();
            let temps = [
                refr(TEMP_REF, 0, LAMP, [100.0, 100.0, 50.0]),
                refr(DORMANT_REF, ref_flags::INITIALLY_DISABLED, LAMP, [2000.0, 100.0, 50.0]),
                refr(DELETED_REF, ref_flags::DELETED, LAMP, [3000.0, 100.0, 50.0]),
                refr(OFF_REF, 0, OFF_LAMP, [1000.0, 100.0, 50.0]),
                refr(ROCK_REF, 0, ROCK, [10.0, 10.0, 0.0]),
                refr(POST_REF, 0, POST, [400.0, 600.0, 20.0]),
                refr(BOULDER_REF, 0, BOULDER, [800.0, 600.0, 20.0]),
            ]
            .concat();
            let exterior = [
                cell(CELL_A, Some((0, 0))),
                group(CELL_A.to_le_bytes(), 6, &group(CELL_A.to_le_bytes(), 9, &temps)),
                cell(CELL_B, Some((1, 0))),
            ]
            .concat();
            let children = group(WRLD.to_le_bytes(), 1, &[persistent, group([0; 4], 4, &group([0; 4], 5, &exterior))].concat());
            let bytes = [tes4, lights, stat, group(*b"WRLD", 0, &[wrld, children].concat())].concat();
            let plugins: &'static [Plugin] = Box::leak(vec![Plugin::parse(&bytes, "Lights.esm".into()).unwrap()].into_boxed_slice());
            let lo: &'static LoadOrder<'static> = Box::leak(Box::new(LoadOrder::new(plugins).unwrap()));
            let schema: &'static Schema = Box::leak(Box::new(Schema::load_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/../../schemas/fnv")).unwrap()));
            let index: &'static WorldIndex = Box::leak(Box::new(WorldIndex::build(lo, schema)));
            let mut w = WorldData { lo, schema, index, ..crate::outfits::tests::empty_world_data() };
            w.set_world(WRLD);
            w
        }

        #[test]
        fn temporary_and_persistent_lights_of_their_cells_are_listed() {
            let w = world_data();
            assert!(w.index.errors.is_empty(), "{:?}", w.index.errors);
            let (mut bases, mut stats) = (HashMap::new(), Stats::default());
            // The lamp post's model glows at its bulb, 300 up; the boulder's does not.
            let bulb = ModelGlow { centre: [0.0, 0.0, 300.0], radius: 10.0, colour: [1.0, 0.8, 0.5], shapes: 1 };
            let mut asked = Vec::new();
            let mut glow = |path: &str| {
                asked.push(path.to_string());
                (path == "meshes\\lamp.nif").then_some(bulb)
            };
            let a = list_lights(w.lo, w.schema, &cell_refs(&w, (0, 0)), &mut bases, &mut glow, &mut stats);
            let b = list_lights(w.lo, w.schema, &cell_refs(&w, (1, 0)), &mut bases, &mut glow, &mut stats);
            let ids = |v: &[SourceLight]| v.iter().map(|l| (l.form_id, l.start_enabled)).collect::<Vec<_>>();
            // Cell (0, 0): its temporary light and the one disabled at the start (kept, with
            // that state), the lamp post as a glow static; not the deleted light, the one off
            // by default, the boulder or the static without a model.
            assert_eq!(ids(&a), vec![(TEMP_REF, true), (DORMANT_REF, false), (POST_REF, true)]);
            assert_eq!(a[0].pos, [100.0, 100.0, 50.0]);
            let post = a[2];
            assert_eq!(post.pos, [400.0, 600.0, 320.0], "at the bulb, taken by the reference");
            assert_eq!((post.color, post.radius, post.strength, post.motion), ([1.0, 0.8, 0.5], 10.0 * GLOW_RADIUS_SCALE, 1.0, 0));
            asked.sort();
            assert_eq!(asked, vec!["meshes\\boulder.nif".to_string(), "meshes\\lamp.nif".to_string()], "only statics with a model are asked");
            // Cell (1, 0): the persistent light standing in it.
            assert_eq!(ids(&b), vec![(PERSISTENT_REF, true)]);
            assert_eq!(stats, Stats { refs: 8, lights: 3, glow_statics: 1, disabled: 1, deleted: 1, negative_off: 1, ..Stats::default() });
        }
    }
}
