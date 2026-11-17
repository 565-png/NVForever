//! Terrain shadow heights (#2671, stage T1 of `terrain-shadows`): from the sun heightfield
//! (stage T0, `sun_heightfield.rs`) a GPU pass computes the shadow-height map S, the height
//! under which a receiver over a texel lies in the landscape's own shadow:
//!
//! `S(x, z) = max over t > 0 of [H((x, z) + t·L) − t·tan(elevation)]`, L the horizontal unit
//! direction towards the light. A receiver at height y is in terrain shadow when y < S
//! ([`crate::sun_heightfield::shadowed`]). Nothing samples S yet: stage T2 binds it on
//! receivers. This stage ends at a published resource ([`TerrainShadows`]) and debug aids.
//!
//! **Notes for T2: sampling S on a receiver** (plain words; the map's facts below):
//! - The map: [`TerrainShadows::image`], `Rg32Float`, R = S in world units (Bevy y), G = the
//!   horizontal distance in world units from the receiver texel's centre to the texel that
//!   gave S (0 when nothing casts). Where nothing casts, R is [`NO_SHADOW`] (−1e30, the
//!   heightfield's own `INVALID_HEIGHT`), so `y < S` is simply false: no separate mask.
//!   Rg32Float is not filterable without an optional feature: read it with `textureLoad`
//!   (or a non-filtering sampler) and, for a soft edge, filter the *comparisons* of a few
//!   texels by hand, never S itself (S jumps by hundreds of units at a ridge's shadow end).
//! - Where: `uv = bevy.xz · uv_scale + uv_offset` ([`TerrainShadows::world_to_uv`]), uv in
//!   0..1 with v = 0 the southernmost row; texel = floor(uv · side). Outside 0..1 there is no
//!   terrain data: no terrain shadow. Fade the factor to 1 over the last few texels of the
//!   map (an edge fade), as the map's edge is where marches end early and nothing is known
//!   beyond it.
//! - Bias: the heightfield keeps each texel's *highest* point (`sun_heightfield.rs`,
//!   `downsample_max`), so on a slope facing the light the next texel sunward stands up to
//!   the relief inside one texel above the receiver's own ground, and S over a sloped texel
//!   can exceed the ground there by up to that relief (test
//!   `slope_overshoot_is_within_a_texel_of_relief`). Compare `y + bias < S` with a bias that
//!   grows with the receiver's slope, about `texel · |slope|` (256 units of texel times the
//!   ground's rise per unit), plus a small constant; flat ground needs none. Too large a bias
//!   detaches shadows from the ridge foot: keep it to that slope term. The receiver's own
//!   texel is already left out of the march (as in the CPU reference).
//! - Penumbra: G times the light's angular size (radians) is a penumbra width in world
//!   units at the receiver, if a later stage softens the edge by distance.
//! - Freshness: [`TerrainShadows::version`] changes with every publication and the image
//!   alternates between two textures; bind by handle each frame (or on version change).
//!   `image == None` means "no terrain shadow anywhere" (off, indoors, no heightfield yet,
//!   the light at the horizon): use a factor of 1 then.
//!
//! **The pass** [engineering choice]: a compute pass, `terrain_shadows.wesl`, one invocation
//! per texel in 8 × 8 workgroups, reading T0's `Rg32Float` image with `textureLoad` (R
//! height, G validity) and writing a `Rg32Float` storage texture (`write` access).
//! `Rg32Float` write-only storage and `R32Float` read-write storage are guaranteed formats
//! in wgpu 30 without optional features (`wgpu-types` `TextureFormat::guaranteed_format_features`:
//! Rg32Float `s_ro_wo`, storage usages) and in WebGPU core, and compute shaders are part of
//! every backend the game runs on (Vulkan by default, DX12 by `WGPU_BACKEND`, docs/dlss.md)
//! [documented: wgpu-types 30.0.2 texture/format.rs]. A fragment pass would need the same
//! format as a render target, which is also guaranteed; compute needs no vertex stage or
//! render pass for a full-map write. It runs in the render graph's `Begin` set (before any
//! view draws), only on a frame with a new job.
//!
//! **The march** ("The march" in docs/design/terrain-shadows.md): from the receiver texel's
//! centre along L, at most [`STEPS`] samples at distances `t_i = STEP_MIN·i + b·i²` texels,
//! b chosen so the last sample lands on the ray's reach: the nearer of the map's edge and the
//! distance `(max − min) / tan(elevation)` past which nothing can rise above the map's lowest
//! valid height. So high sun is nearly free (reach a few texels), and at low sun the steps
//! grow with distance (as the square root of it) to cover the whole map. Each sampled texel
//! is scored exactly as the CPU reference scores it, `H − entry·tan(e)` with the ray's exact
//! entry distance into that texel, and the march stops as soon as the ray, falling from the
//! map's highest height, cannot beat the best so far. The receiver's own texel is left out.
//! **Miss bound**: the march only visits texels the reference visits and scores them alike,
//! so it is never darker than the reference; it is lighter only where it steps over a
//! texel. A caster at least as long along the ray as the local step (at most [`max_step`]:
//! 0.5 texel until the reach exceeds `STEPS·STEP_MIN` = 48 texels, 14.5 texels = 3,700
//! units at the full 724-texel diagonal) is always sampled, and for a flat-topped caster
//! the result is within `tan(e)·step` of the reference; a narrower one can be missed. A
//! max-height pyramid (one level per step size, a sample reads the level whose texel
//! covers its step) would remove that miss: [`march_texel`] and the shader's `march` are
//! one function each with the texel read in one place, so a pyramid read replaces it there.
//!
//! **Cadence** ([`update_reason`]): a new map only when (a) T0's `version` changed, or
//! (b) the light moved enough that the predicted shadow-edge displacement exceeds half a
//! texel (128 units) since the last publication, and never sooner than [`MIN_INTERVAL_SECS`]
//! after the last one. The displacement ([`edge_displacement`]) for a blocker of height h
//! (the map's `max − min`, so a bound for every blocker on it): the shadow's length
//! `L = h·cot(e)` (at most the map's diagonal) changes by `|ΔL|`, to first order
//! `h·csc²(e)·Δe`, and its tip swings by `L·Δazimuth`. The light is the `sky::Sun`
//! entity's own transform (it turns every frame), not the sky's quantized hour; at night
//! it is the moon, handled alike. While the light is within the horizon fade that stops the
//! sun's own shadows (`sky.rs` `HORIZON_FADE` 0.1 rad, `graphics.rs` `SHADOW_FADED` 0.1:
//! the light's sine under 0.1·sin 0.1, [`MIN_LIGHT_SIN`]) nothing is computed and
//! "no terrain shadow" is published.
//!
//! **Double buffering**: two target images, made on the first job; a job writes the one not
//! published, and the map is published only when the render world has recorded the whole
//! dispatch ([`JobsDone`], read by the main world a frame or two later), so a receiver never
//! sees a half-written map. Each job binds T0's image by handle at dispatch, so it follows
//! T0's new GPU texture on every publication.
//!
//! **The published resource**: [`TerrainShadows`], a main-world resource extracted to the
//! render world whenever it changes (T2 reads it there for its bind group).
//!
//! **Off path**: everything behind [`TerrainShadowsWanted`] (false by default), which also
//! sets T0's `SunHeightfieldWanted`. While false: no job, no target, no pipeline compiled,
//! the render system's run condition fails. `NVF_PLAY_DEBUG_TERRAIN_SHADOWS=1` turns both on.
//!
//! **Debug aids** (`NVF_PLAY_DEBUG_*`): `NVF_PLAY_DEBUG_TERRAIN_SHADOW_DUMP=<path.png>`
//! reads back the first map published at least [`DUMP_AFTER_SECS`] into the run and writes
//! it with the heightfield it was made from as greyscale PNGs, north up: `<path>.png` (S),
//! `<stem>-height.png` (H) and `<stem>-mask.png` (valid ground lit 255, in shadow 80,
//! invalid 0), with a log line giving the range mapped to 0..255. Each publication logs
//! `terrain shadows: published v<n> for sun elevation <deg> azimuth <deg>, heightfield v<m>,
//! <steps> steps, <reason>`, at most one a second; azimuth in degrees clockwise from north.

use crate::sun_heightfield::{SunHeightfield, SunHeightfieldWanted, INVALID_HEIGHT};
use crate::world::bevy_to_game;
use bevy::asset::RenderAssetUsages;
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::diagnostic::RecordDiagnostics;
use bevy::render::extract_resource::{ExtractResource, ExtractResourcePlugin};
use bevy::render::gpu_readback::{ReadbackComplete, ReadbackOnce};
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::binding_types::{texture_2d, texture_storage_2d, uniform_buffer_sized};
use bevy::render::render_resource::{
    BindGroupEntries, BindGroupLayoutDescriptor, BindGroupLayoutEntries, BufferInitDescriptor, BufferUsages, CachedComputePipelineId, ComputePassDescriptor,
    ComputePipelineDescriptor, Extent3d, PipelineCache, ShaderStages, StorageTextureAccess, TextureDimension, TextureFormat, TextureSampleType, TextureUsages,
};
use bevy::render::renderer::{RenderContext, RenderDevice, RenderGraph, RenderGraphSystems};
use bevy::render::texture::GpuImage;
use bevy::render::RenderApp;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// R where nothing casts: the heightfield's invalid height, so `y < S` is false.
pub const NO_SHADOW: f32 = INVALID_HEIGHT;
/// The most samples per texel [engineering choice, #2671 plan: a fixed-step first version].
pub const STEPS: u32 = 96;
/// Length of the first steps in texels [engineering choice: half a texel, so a caster a
/// texel long is never stepped over while the reach is under 48 texels].
pub const STEP_MIN: f32 = 0.5;
/// The shortest time between two computations (real seconds) [engineering choice, #2671
/// plan: 0.1 s].
pub const MIN_INTERVAL_SECS: f32 = 0.1;
/// A new map when the predicted shadow-edge movement exceeds this many texels
/// [engineering choice, #2671 plan: half a texel].
pub const EDGE_TOLERANCE_TEXELS: f32 = 0.5;
/// Below this sine of its elevation the light casts no terrain shadow: the fade under which
/// the sun's own shadow maps stop (`sky.rs` `HORIZON_FADE` = 0.1 rad, `graphics.rs`
/// `SHADOW_FADED` = 0.1: the light at a tenth of its strength, sine 0.1·sin 0.1)
/// [design: those files; mirrored here as they are private].
pub const MIN_LIGHT_SIN: f32 = 0.1 * 0.099_833_42;
/// Below this horizontal share the light is overhead: no direction to march in.
pub const MIN_HORIZONTAL: f32 = 1.0e-4;
/// The output image's format: R shadow height, G caster distance.
pub const OUTPUT_FORMAT: TextureFormat = TextureFormat::Rg32Float;
/// Invocations per workgroup side (`terrain_shadows.wesl` `@workgroup_size`).
pub const WORKGROUP: u32 = 8;
/// Bytes of the shader's `Params` uniform.
pub const PARAMS_BYTES: usize = 48;
/// The dump aid waits this long into the run (real seconds).
pub const DUMP_AFTER_SECS: f32 = 3.0;
/// At most one publication log line this often (real seconds).
pub const LOG_INTERVAL_SECS: f32 = 1.0;

/// The march's inputs, in texel units; the shader's `Params`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MarchParams {
    /// Unit horizontal direction towards the light: x east (+column), y north (+row).
    pub dir: Vec2,
    /// Height lost per texel of horizontal distance: tan(elevation) · texel.
    pub tan_texel: f32,
    /// World units per texel.
    pub texel: f32,
    /// Highest and lowest valid height of the map.
    pub top: f32,
    pub bottom: f32,
    pub side: u32,
    pub steps: u32,
    pub step_min: f32,
}

/// Elevation (radians above the horizon) and azimuth (radians clockwise from north) of a
/// direction towards the light in game axes (x east, y north, z up); with its unit
/// horizontal direction. `None` when it has no horizontal part.
pub fn light_angles(toward: Vec3) -> Option<(Vec2, f32, f32)> {
    let v = toward.normalize_or_zero();
    let flat = v.truncate();
    let horizontal = flat.length();
    if horizontal < MIN_HORIZONTAL {
        return None;
    }
    Some((flat / horizontal, v.z.atan2(horizontal), v.x.atan2(v.y)))
}

/// The march's inputs for a light `toward` (game axes, towards the light) over a map of
/// `side` texels of `texel` units whose valid heights span `min..max`; the reason there is
/// no terrain shadow otherwise.
pub fn march_params(toward: Vec3, texel: f32, side: u32, min: f32, max: f32) -> Result<MarchParams, &'static str> {
    let v = toward.normalize_or_zero();
    if v.z < MIN_LIGHT_SIN {
        return Err("light at or under the horizon");
    }
    let Some((dir, elevation, _)) = light_angles(v) else { return Err("light overhead") };
    Ok(MarchParams { dir, tan_texel: elevation.tan() * texel, texel, top: max, bottom: min, side, steps: STEPS, step_min: STEP_MIN })
}

/// Where the ray from `p` along `d` enters the span `[c, c + 1]` on one axis (texel units).
fn axis_entry(c: f32, p: f32, d: f32) -> f32 {
    if d.abs() < 1.0e-6 {
        return -1.0e30;
    }
    let edge = if d > 0.0 { c } else { c + 1.0 };
    (edge - p) / d
}

/// Horizontal distance (texels) from `p` along `d` to the map's edge.
fn edge_distance(p: Vec2, d: Vec2, side: f32) -> f32 {
    let mut t = 1.0e30f32;
    for (p, d) in [(p.x, d.x), (p.y, d.y)] {
        if d > 1.0e-6 {
            t = t.min((side - p) / d);
        } else if d < -1.0e-6 {
            t = t.min(-p / d);
        }
    }
    t
}

/// How far (texels) the march from texel (`col`, `row`) goes: the map's edge, or where the
/// ray falling from the highest height is below the lowest.
pub fn texel_reach(p: &MarchParams, (col, row): (usize, usize)) -> f32 {
    let pos = Vec2::new(col as f32 + 0.5, row as f32 + 0.5);
    edge_distance(pos, p.dir, p.side as f32).min((p.top - p.bottom).max(0.0) / p.tan_texel)
}

/// The steps' growth `b` for a march of `reach` texels.
fn step_growth(p: &MarchParams, reach: f32) -> f32 {
    let n = p.steps as f32;
    (reach - n * p.step_min).max(0.0) / (n * n)
}

/// The longest step (texels) of a march of `reach` texels: its last one.
#[cfg_attr(not(test), allow(dead_code))] // the miss bound the tests hold the march to
pub fn max_step(p: &MarchParams, reach: f32) -> f32 {
    p.step_min + step_growth(p, reach) * (2.0 * p.steps as f32 - 1.0)
}

/// CPU mirror of `terrain_shadows.wesl` `march`, line for line: (S, caster distance in world
/// units) for receiver texel (`col`, `row`); [`NO_SHADOW`], 0 where nothing casts.
pub fn march_texel(heights: &[f32], valid: &[u8], (col, row): (usize, usize), p: &MarchParams) -> (f32, f32) {
    let side = p.side as i64;
    let pos = Vec2::new(col as f32 + 0.5, row as f32 + 0.5);
    let d = p.dir;
    let reach = texel_reach(p, (col, row));
    let n = p.steps.min((reach.max(0.0) / p.step_min).ceil() as u32);
    let b = step_growth(p, reach);
    let mut best = NO_SHADOW;
    let mut best_t = 0.0;
    let mut last = (col as i64, row as i64);
    for i in 1..=n {
        let fi = i as f32;
        let t = p.step_min * fi + b * fi * fi;
        let q = pos + t * d;
        let c = (q.x.floor() as i64, q.y.floor() as i64);
        if c.0 < 0 || c.1 < 0 || c.0 >= side || c.1 >= side {
            break;
        }
        if c == last {
            continue;
        }
        last = c;
        let entry = axis_entry(c.0 as f32, pos.x, d.x).max(axis_entry(c.1 as f32, pos.y, d.y)).max(0.0);
        let fall = entry * p.tan_texel;
        if p.top - fall <= best {
            break;
        }
        let at = (c.1 * side + c.0) as usize;
        if valid[at] != 0 {
            let s = heights[at] - fall;
            if s > best {
                best = s;
                best_t = entry;
            }
        }
    }
    if best <= NO_SHADOW {
        (NO_SHADOW, 0.0)
    } else {
        (best, best_t * p.texel)
    }
}

/// CPU mirror of the shader's entry point for one texel: an invalid receiver, or no light
/// (steps 0), gets no shadow.
pub fn produce_texel(heights: &[f32], valid: &[u8], (col, row): (usize, usize), p: &MarchParams) -> (f32, f32) {
    if valid[row * p.side as usize + col] == 0 || p.steps == 0 {
        return (NO_SHADOW, 0.0);
    }
    march_texel(heights, valid, (col, row), p)
}

/// The whole map on the CPU (tests; the GPU pass's mirror).
#[cfg_attr(not(test), allow(dead_code))]
pub fn produce_map(heights: &[f32], valid: &[u8], p: &MarchParams) -> Vec<(f32, f32)> {
    let side = p.side as usize;
    (0..side * side).map(|i| produce_texel(heights, valid, (i % side, i / side), p)).collect()
}

/// The shader's `Params` bytes (little-endian, the WGSL uniform layout: offsets 0 dir, 8
/// tan_texel, 12 texel, 16 top, 20 bottom, 24 side, 28 steps, 32 step_min, 36..48 padding).
pub fn params_bytes(p: &MarchParams) -> [u8; PARAMS_BYTES] {
    let words = [p.dir.x.to_bits(), p.dir.y.to_bits(), p.tan_texel.to_bits(), p.texel.to_bits(), p.top.to_bits(), p.bottom.to_bits(), p.side, p.steps, p.step_min.to_bits(), 0, 0, 0];
    let mut out = [0u8; PARAMS_BYTES];
    for (i, w) in words.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
    }
    out
}

/// Predicted shadow-edge displacement (world units) when the light turns from `from` to
/// `to` (game axes, towards the light), for blockers up to `relief` high on a map whose
/// shadows can be at most `longest` long (module notes, "Cadence"): the change of the
/// shadow's length `h·cot(e)` plus its tip's swing `L·Δazimuth`. Infinite when either
/// direction has no elevation or no horizontal part (always update).
pub fn edge_displacement(from: Vec3, to: Vec3, relief: f32, longest: f32) -> f32 {
    let (Some((_, e0, a0)), Some((_, e1, a1))) = (light_angles(from), light_angles(to)) else { return f32::INFINITY };
    if e0 <= 0.0 || e1 <= 0.0 {
        return f32::INFINITY;
    }
    let length = |e: f32| (relief.max(0.0) / e.tan()).min(longest);
    let mut da = (a1 - a0).abs() % std::f32::consts::TAU;
    if da > std::f32::consts::PI {
        da = std::f32::consts::TAU - da;
    }
    (length(e0) - length(e1)).abs() + length(e0.min(e1)) * da
}

/// What the published map was computed for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Basis {
    /// Towards the light, game axes.
    pub toward: Vec3,
    pub heightfield_version: u64,
    /// When its computation was asked for (real seconds).
    pub requested_at: f32,
}

/// Why a new map is computed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Reason {
    /// Nothing is published yet (or the light just came back).
    First,
    /// T0 published new heights.
    Heights,
    /// The light moved: the predicted edge displacement in world units.
    SunMoved(f32),
}

impl std::fmt::Display for Reason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Reason::First => write!(f, "first"),
            Reason::Heights => write!(f, "heightfield changed"),
            Reason::SunMoved(d) => write!(f, "sun moved: edge {d:.0} units"),
        }
    }
}

/// The cadence rule (module notes, "Cadence"): whether to compute a new map now, against
/// the published one's `basis`, for heightfield `version`, the light `toward` (game axes)
/// and the map's `relief` (max − min), `texel` and longest shadow `longest` (world units).
pub fn update_reason(basis: Option<&Basis>, now: f32, version: u64, toward: Vec3, relief: f32, texel: f32, longest: f32) -> Option<Reason> {
    let Some(b) = basis else { return Some(Reason::First) };
    if now - b.requested_at < MIN_INTERVAL_SECS {
        return None;
    }
    if b.heightfield_version != version {
        return Some(Reason::Heights);
    }
    let d = edge_displacement(b.toward, toward, relief, longest);
    (d > EDGE_TOLERANCE_TEXELS * texel).then_some(Reason::SunMoved(d))
}

/// Whether terrain shadows are computed (later: the settings row). Off by default;
/// `NVF_PLAY_DEBUG_TERRAIN_SHADOWS=1` turns it on. Turning it on turns T0's heightfield on.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TerrainShadowsWanted(pub bool);

/// The last complete shadow-height map (module notes, "Notes for T2"). A main-world
/// resource, extracted to the render world whenever it changes.
#[derive(Resource, Clone, Debug, Default)]
#[allow(dead_code)] // the fields T2's receivers read (#2671)
pub struct TerrainShadows {
    /// The map ([`OUTPUT_FORMAT`], `side` × `side`, row 0 the southernmost); `None`: no
    /// terrain shadow anywhere (`reason` says why).
    pub image: Option<Handle<Image>>,
    /// Game x, y of texel (0, 0)'s south-west corner (Bevy x, −z), from T0.
    pub origin: Vec2,
    /// World units per texel.
    pub texel: f32,
    /// Texels per side.
    pub side: u32,
    /// `uv = bevy.xz · uv_scale + uv_offset` ([`TerrainShadows::world_to_uv`]).
    pub uv_scale: Vec2,
    pub uv_offset: Vec2,
    /// The light it was computed for, towards the light, Bevy axes.
    pub light_dir: Vec3,
    /// That light's elevation and azimuth (radians; azimuth clockwise from north).
    pub elevation: f32,
    pub azimuth: f32,
    /// When that light direction was taken (real seconds since start).
    pub computed_at: f32,
    /// T0's version it was computed from.
    pub heightfield_version: u64,
    /// Bumped with every change of this resource (publication or going dark).
    pub version: u64,
    /// The heightfield's valid height range it was computed with.
    pub min: f32,
    pub max: f32,
    /// The most samples per texel.
    pub steps: u32,
    /// Why there is no map ("" while there is one).
    pub reason: &'static str,
}

impl TerrainShadows {
    /// The world-to-uv transform for a map with south-west corner `origin` (game x, y) of
    /// `side` texels of `texel` units: Bevy (x, z) is game (x, −y), v grows northwards.
    pub fn uv_transform(origin: Vec2, texel: f32, side: u32) -> (Vec2, Vec2) {
        let extent = texel * side as f32;
        (Vec2::new(1.0 / extent, -1.0 / extent), Vec2::new(-origin.x / extent, -origin.y / extent))
    }

    /// The map's uv of a Bevy world position (0..1 inside the map).
    #[cfg_attr(not(test), allow(dead_code))] // T2's receivers do this in the shader
    pub fn world_to_uv(&self, bevy: Vec3) -> Vec2 {
        Vec2::new(bevy.x, bevy.z) * self.uv_scale + self.uv_offset
    }

    pub fn is_active(&self) -> bool {
        self.image.is_some()
    }
}

/// A computation for the render world.
#[derive(Clone, Debug)]
struct Job {
    id: u64,
    heights: Handle<Image>,
    target: Handle<Image>,
    params: MarchParams,
}

#[derive(Resource, Clone, Debug, Default)]
struct ShadowJob(Option<Job>);

impl ExtractResource<RenderApp> for ShadowJob {
    type Source = ShadowJob;

    fn extract_resource(source: &Self::Source) -> Self {
        source.clone()
    }
}

impl ExtractResource<RenderApp> for TerrainShadows {
    type Source = TerrainShadows;

    fn extract_resource(source: &Self::Source) -> Self {
        source.clone()
    }
}

/// The last job the render world has recorded completely (shared by both worlds). Ids only
/// grow, so a job is done once this reaches its id.
#[derive(Resource, Clone, Default)]
struct JobsDone(Arc<AtomicU64>);

/// The job in flight and what it was asked for.
#[derive(Clone, Debug)]
struct Pending {
    id: u64,
    target: usize,
    toward_bevy: Vec3,
    toward_game: Vec3,
    params: MarchParams,
    heightfield_version: u64,
    origin: Vec2,
    requested_at: f32,
    reason: String,
}

/// The main world's side of the producer.
#[derive(Resource, Default)]
struct Producer {
    /// The two targets (made on the first job) and their side.
    targets: Option<[Handle<Image>; 2]>,
    target_side: u32,
    /// The published target.
    front: Option<usize>,
    pending: Option<Pending>,
    /// What the published map was computed for.
    basis: Option<Basis>,
    /// The last job id given out (never reset: [`JobsDone`] keeps the last one done).
    last_id: u64,
    last_log: Option<f32>,
}

/// `NVF_PLAY_DEBUG_TERRAIN_SHADOW_DUMP` (module notes, "Debug aids").
#[derive(Resource, Default)]
struct DumpAid {
    path: Option<PathBuf>,
    asked: bool,
}

pub struct TerrainShadowsPlugin;

impl Plugin for TerrainShadowsPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "terrain_shadows.wesl");
        if nvf_play::dialogue::debug_switch(std::env::var("NVF_PLAY_DEBUG_TERRAIN_SHADOWS").ok().as_deref()) {
            app.insert_resource(TerrainShadowsWanted(true)).insert_resource(SunHeightfieldWanted(true));
        } else {
            app.init_resource::<TerrainShadowsWanted>();
        }
        let path = std::env::var_os("NVF_PLAY_DEBUG_TERRAIN_SHADOW_DUMP").filter(|v| !v.is_empty()).map(PathBuf::from);
        let done = JobsDone::default();
        let wanted = |w: Res<TerrainShadowsWanted>| w.0;
        app.insert_resource(done.clone())
            .insert_resource(DumpAid { path, asked: false })
            .init_resource::<TerrainShadows>()
            .init_resource::<ShadowJob>()
            .init_resource::<Producer>()
            .add_plugins((ExtractResourcePlugin::<ShadowJob>::default(), ExtractResourcePlugin::<TerrainShadows>::default()))
            .add_systems(
                Update,
                (
                    follow_wanted.before(crate::sun_heightfield::stop_heightfield).before(crate::sun_heightfield::update_heightfield),
                    (
                        stop_terrain_shadows.run_if(|w: Res<TerrainShadowsWanted>| w.is_changed() && !w.0),
                        update_terrain_shadows.run_if(wanted),
                        dump_terrain_shadows.run_if(wanted).run_if(|aid: Res<DumpAid>| aid.path.is_some() && !aid.asked),
                    )
                        .chain()
                        .after(crate::sun_heightfield::update_heightfield),
                ),
            );
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else { return };
        render_app.insert_resource(done).init_resource::<ProducerPipeline>().add_systems(
            RenderGraph,
            // Before every view's passes, so a map published later is complete on the GPU
            // before anything samples it.
            produce_terrain_shadows.in_set(RenderGraphSystems::Begin).run_if(|job: Option<Res<ShadowJob>>| job.is_some_and(|j| j.0.is_some())),
        );
    }
}

/// Terrain shadows need T0's heightfield: switching them on switches it on, switching them
/// off switches it off (T0's own debug switch still works while they were never on).
fn follow_wanted(wanted: Res<TerrainShadowsWanted>, mut heightfield: ResMut<SunHeightfieldWanted>) {
    if !wanted.is_changed() {
        return;
    }
    if wanted.0 {
        if !heightfield.0 {
            heightfield.0 = true;
        }
    } else if !wanted.is_added() && heightfield.0 {
        heightfield.0 = false;
    }
}

/// Switched off: nothing published, no job, the targets freed.
fn stop_terrain_shadows(mut producer: ResMut<Producer>, mut job: ResMut<ShadowJob>, mut out: ResMut<TerrainShadows>, mut images: ResMut<Assets<Image>>) {
    let had = producer.targets.is_some() || out.is_active() || job.0.is_some();
    if job.0.is_some() {
        job.0 = None;
    }
    if let Some(targets) = producer.targets.take() {
        for t in &targets {
            images.remove(t);
        }
    }
    let last_id = producer.last_id;
    *producer = Producer { last_id, ..default() };
    if had {
        let version = out.version + 1;
        *out = TerrainShadows { version, reason: "off", ..default() };
        info!("terrain shadows: off");
    }
}

/// A target: `side` × `side`, written by the pass, sampled with loads or nearest.
fn target_image(side: u32) -> Image {
    let mut image = Image::new_uninit(Extent3d { width: side, height: side, depth_or_array_layers: 1 }, TextureDimension::D2, OUTPUT_FORMAT, RenderAssetUsages::RENDER_WORLD);
    image.texture_descriptor.usage |= TextureUsages::STORAGE_BINDING;
    image.sampler = ImageSampler::nearest();
    image
}

/// No terrain shadow for `why`: drop the job in flight, publish "none" once.
fn go_dark(why: &'static str, producer: &mut Producer, job: &mut ResMut<ShadowJob>, out: &mut ResMut<TerrainShadows>) {
    producer.pending = None;
    producer.basis = None;
    if job.0.is_some() {
        job.0 = None;
    }
    if out.is_active() || out.reason != why {
        out.image = None;
        out.reason = why;
        out.version += 1;
        info!("terrain shadows: none ({why})");
    }
}

#[allow(clippy::too_many_arguments)]
fn update_terrain_shadows(
    field: Res<SunHeightfield>,
    sun: Query<&Transform, With<crate::sky::Sun>>,
    time: Res<Time<Real>>,
    done: Res<JobsDone>,
    mut producer: ResMut<Producer>,
    mut job: ResMut<ShadowJob>,
    mut out: ResMut<TerrainShadows>,
    mut images: ResMut<Assets<Image>>,
) {
    let now = time.elapsed_secs();
    let producer = &mut *producer;
    // A job the render world has recorded becomes the publication.
    if producer.pending.as_ref().is_some_and(|p| done.0.load(Ordering::Acquire) >= p.id) {
        if let (Some(p), Some(targets)) = (producer.pending.take(), producer.targets.as_ref()) {
            publish(&p, targets[p.target].clone(), now, producer, &mut out);
        }
        if job.0.is_some() {
            job.0 = None;
        }
    }
    let Ok(sun) = sun.single() else { return go_dark("no sun light", producer, &mut job, &mut out) };
    let toward_bevy = sun.back().as_vec3();
    let toward_game = bevy_to_game(toward_bevy);
    let Some(heights) = field.image.clone().filter(|_| !field.is_empty() && field.valid_texels > 0) else {
        return go_dark("no heightfield", producer, &mut job, &mut out);
    };
    let side = field.side as u32;
    let params = match march_params(toward_game, field.texel, side, field.min, field.max) {
        Ok(p) => p,
        Err(why) => return go_dark(why, producer, &mut job, &mut out),
    };
    // One job at a time; one asked for older heights is asked again with the new ones.
    if let Some(p) = producer.pending.as_mut() {
        if p.heightfield_version != field.version {
            producer.last_id += 1;
            p.id = producer.last_id;
            p.params = params;
            p.heightfield_version = field.version;
            p.origin = field.origin;
            p.toward_bevy = toward_bevy;
            p.toward_game = toward_game;
            p.requested_at = now;
            p.reason = "heightfield changed while computing".into();
            if let Some(targets) = producer.targets.as_ref() {
                job.0 = Some(Job { id: p.id, heights, target: targets[p.target].clone(), params });
            }
        }
        return;
    }
    let basis = if out.is_active() { producer.basis.as_ref() } else { None };
    let longest = field.side as f32 * field.texel * std::f32::consts::SQRT_2;
    let Some(reason) = update_reason(basis, now, field.version, toward_game, field.max - field.min, field.texel, longest) else { return };
    if producer.targets.is_none() || producer.target_side != side {
        producer.targets = Some([images.add(target_image(side)), images.add(target_image(side))]);
        producer.target_side = side;
        producer.front = None;
    }
    let Some(targets) = producer.targets.as_ref() else { return };
    let target = producer.front.map_or(0, |f| 1 - f);
    producer.last_id += 1;
    let id = producer.last_id;
    job.0 = Some(Job { id, heights, target: targets[target].clone(), params });
    producer.pending = Some(Pending {
        id,
        target,
        toward_bevy,
        toward_game,
        params,
        heightfield_version: field.version,
        origin: field.origin,
        requested_at: now,
        reason: reason.to_string(),
    });
}

/// Publish a finished job's map and log it (at most one line a second).
fn publish(p: &Pending, image: Handle<Image>, now: f32, producer: &mut Producer, out: &mut TerrainShadows) {
    let (_, elevation, azimuth) = light_angles(p.toward_game).unwrap_or_default();
    let (uv_scale, uv_offset) = TerrainShadows::uv_transform(p.origin, p.params.texel, p.params.side);
    *out = TerrainShadows {
        image: Some(image),
        origin: p.origin,
        texel: p.params.texel,
        side: p.params.side,
        uv_scale,
        uv_offset,
        light_dir: p.toward_bevy,
        elevation,
        azimuth,
        computed_at: p.requested_at,
        heightfield_version: p.heightfield_version,
        version: out.version + 1,
        min: p.params.bottom,
        max: p.params.top,
        steps: p.params.steps,
        reason: "",
    };
    producer.front = Some(p.target);
    producer.basis = Some(Basis { toward: p.toward_game, heightfield_version: p.heightfield_version, requested_at: p.requested_at });
    if producer.last_log.is_none_or(|t| now - t >= LOG_INTERVAL_SECS) {
        producer.last_log = Some(now);
        info!(
            "terrain shadows: published v{} for sun elevation {:.2} azimuth {:.1}, heightfield v{}, {} steps, {}",
            out.version,
            elevation.to_degrees(),
            azimuth.to_degrees().rem_euclid(360.0),
            p.heightfield_version,
            p.params.steps,
            p.reason
        );
    }
}

/// The render world's side: the pipeline (asked for on the first job) and the last job run.
#[derive(Resource, Default)]
struct ProducerPipeline {
    layout: Option<BindGroupLayoutDescriptor>,
    id: Option<CachedComputePipelineId>,
    last_dispatched: u64,
    warned: bool,
}

/// The pass's bindings: 0 the heightfield (`texture_2d<f32>`, unfilterable), 1 the target
/// (`texture_storage_2d<rg32float, write>`), 2 the parameters (uniform, [`PARAMS_BYTES`]).
fn layout_descriptor() -> BindGroupLayoutDescriptor {
    BindGroupLayoutDescriptor::new(
        "terrain_shadows_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::COMPUTE,
            (
                texture_2d(TextureSampleType::Float { filterable: false }),
                texture_storage_2d(OUTPUT_FORMAT, StorageTextureAccess::WriteOnly),
                uniform_buffer_sized(false, std::num::NonZeroU64::new(PARAMS_BYTES as u64)),
            ),
        ),
    )
}

#[allow(clippy::too_many_arguments)]
fn produce_terrain_shadows(
    job: Res<ShadowJob>,
    done: Res<JobsDone>,
    mut pipe: ResMut<ProducerPipeline>,
    cache: Res<PipelineCache>,
    gpu_images: Res<RenderAssets<GpuImage>>,
    device: Res<RenderDevice>,
    assets: Res<AssetServer>,
    mut ctx: RenderContext,
) {
    let Some(job) = job.0.as_ref() else { return };
    if job.id <= pipe.last_dispatched {
        return;
    }
    let id = match pipe.id {
        Some(id) => id,
        None => {
            let layout = layout_descriptor();
            let id = cache.queue_compute_pipeline(ComputePipelineDescriptor {
                label: Some("terrain_shadows_pipeline".into()),
                layout: vec![layout.clone()],
                shader: assets.load("embedded://nvforever/terrain_shadows.wesl"),
                entry_point: Some("produce".into()),
                zero_initialize_workgroup_memory: false,
                ..default()
            });
            pipe.layout = Some(layout);
            pipe.id = Some(id);
            id
        }
    };
    let (Some(pipeline), Some(layout)) = (cache.get_compute_pipeline(id), pipe.layout.as_ref()) else { return };
    let (Some(heights), Some(target)) = (gpu_images.get(&job.heights), gpu_images.get(&job.target)) else { return };
    let side = job.params.side;
    let fits = heights.texture.width() == side
        && heights.texture.height() == side
        && heights.texture.format() == crate::sun_heightfield::IMAGE_FORMAT
        && target.texture.width() == side
        && target.texture.height() == side
        && target.texture.format() == OUTPUT_FORMAT
        && target.texture.usage().contains(TextureUsages::STORAGE_BINDING);
    if !fits {
        if !pipe.warned {
            pipe.warned = true;
            warn!(
                "terrain shadows: textures do not fit the job (side {side}; heights {}x{} {:?}, target {}x{} {:?}); not computed",
                heights.texture.width(),
                heights.texture.height(),
                heights.texture.format(),
                target.texture.width(),
                target.texture.height(),
                target.texture.format()
            );
        }
        return;
    }
    let layout = cache.get_bind_group_layout(layout);
    let buffer = device.create_buffer_with_data(&BufferInitDescriptor { label: Some("terrain_shadows_params"), contents: &params_bytes(&job.params), usage: BufferUsages::UNIFORM });
    let group = device.create_bind_group("terrain_shadows", &layout, &BindGroupEntries::sequential((&heights.texture_view, &target.texture_view, buffer.as_entire_binding())));
    let groups = side.div_ceil(WORKGROUP);
    let diagnostics = ctx.diagnostic_recorder();
    let diagnostics = diagnostics.as_deref();
    let mut pass = ctx.command_encoder().begin_compute_pass(&ComputePassDescriptor { label: Some("terrain_shadows"), timestamp_writes: None });
    let span = diagnostics.time_span(&mut pass, "terrain_shadows");
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, &group, &[]);
    pass.dispatch_workgroups(groups, groups, 1);
    span.end(&mut pass);
    pipe.last_dispatched = job.id;
    // Recorded in this frame's command buffers, submitted with them: from here on anything
    // drawn later reads a complete map.
    done.0.store(job.id, Ordering::Release);
}

/// The heightfield a dumped map was made from.
struct DumpSnapshot {
    heights: Vec<f32>,
    valid: Vec<u8>,
    side: usize,
    min: f32,
    max: f32,
    version: u64,
}

/// Read back the first map published [`DUMP_AFTER_SECS`] into the run (module notes).
fn dump_terrain_shadows(mut aid: ResMut<DumpAid>, out: Res<TerrainShadows>, field: Res<SunHeightfield>, time: Res<Time<Real>>, mut commands: Commands) {
    let Some(path) = aid.path.clone() else { return };
    let Some(image) = out.image.clone() else { return };
    if time.elapsed_secs() < DUMP_AFTER_SECS || field.version != out.heightfield_version || field.side as u32 != out.side {
        return;
    }
    aid.asked = true;
    let mut snapshot = Some(DumpSnapshot { heights: field.heights.clone(), valid: field.valid.clone(), side: field.side, min: out.min, max: out.max, version: out.version });
    info!("terrain shadows: reading back v{} for {}", out.version, path.display());
    commands.spawn((Name::new("Terrain shadow dump"), ReadbackOnce::texture(image))).observe(move |ev: On<ReadbackComplete>| {
        if let Some(s) = snapshot.take() {
            write_dump(&path, &ev.data, &s);
        }
    });
}

/// `<stem>-<tag>.png` beside `path`.
fn sibling(path: &Path, tag: &str) -> PathBuf {
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "terrain-shadows".into());
    path.with_file_name(format!("{stem}-{tag}.png"))
}

fn write_grey(path: &Path, pixels: &[u8], side: u32) -> Result<(), String> {
    fnv_install::guard_write(path, None).map_err(|e| e.to_string())?;
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let mut bytes = Vec::new();
    let mut e = png::Encoder::new(&mut bytes, side, side);
    e.set_color(png::ColorType::Grayscale);
    e.set_depth(png::BitDepth::Eight);
    let mut w = e.write_header().map_err(|e| e.to_string())?;
    w.write_image_data(pixels).map_err(|e| e.to_string())?;
    w.finish().map_err(|e| e.to_string())?;
    std::fs::write(path, bytes).map_err(|e| e.to_string())
}

/// The S map, its heightfield and the shaded-ground mask as greyscale PNGs, north up.
fn write_dump(path: &Path, data: &[u8], s: &DumpSnapshot) {
    let (side, n) = (s.side, s.side * s.side);
    if data.len() < n * 8 || s.heights.len() < n || s.valid.len() < n {
        warn!("terrain shadows: dump read back {} bytes for a {side}x{side} map; nothing written", data.len());
        return;
    }
    let shadow_at = |i: usize| f32::from_le_bytes([data[i * 8], data[i * 8 + 1], data[i * 8 + 2], data[i * 8 + 3]]);
    let (lo, hi) = (s.min, s.max.max(s.min + 1.0));
    let grey = |v: f32| (((v - lo) / (hi - lo)).clamp(0.0, 1.0) * 255.0).round() as u8;
    let (mut shadow, mut height, mut mask) = (vec![0u8; n], vec![0u8; n], vec![0u8; n]);
    let (mut valid, mut shaded) = (0usize, 0usize);
    for row in 0..side {
        for col in 0..side {
            let (i, o) = (row * side + col, (side - 1 - row) * side + col);
            let sv = shadow_at(i);
            if sv > NO_SHADOW * 0.1 {
                shadow[o] = grey(sv);
            }
            if s.valid[i] != 0 {
                valid += 1;
                height[o] = grey(s.heights[i]);
                let dark = crate::sun_heightfield::shadowed(s.heights[i], sv);
                shaded += usize::from(dark);
                mask[o] = if dark { 80 } else { 255 };
            }
        }
    }
    let files = [(path.to_path_buf(), shadow), (sibling(path, "height"), height), (sibling(path, "mask"), mask)];
    for (file, pixels) in &files {
        if let Err(e) = write_grey(file, pixels, side as u32) {
            warn!("terrain shadows: cannot write {}: {e}", file.display());
            return;
        }
    }
    info!(
        "terrain shadows: dumped v{} to {} (and -height, -mask): S and heights {lo:.0}..{hi:.0} mapped to 0..255 (no shadow and invalid 0), north up; mask lit 255, shaded 80, invalid 0; {:.1}% of valid ground shaded",
        s.version,
        path.display(),
        if valid == 0 { 0.0 } else { 100.0 * shaded as f32 / valid as f32 }
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sun_heightfield::{shadow_height_reference, shadowed, HeightGrid, SIDE, TEXEL};
    use fnv_world::CELL_SIZE;

    /// Towards the light at `elevation` and `azimuth` (radians, clockwise from north), game axes.
    fn toward(elevation: f32, azimuth: f32) -> Vec3 {
        Vec3::new(azimuth.sin() * elevation.cos(), azimuth.cos() * elevation.cos(), elevation.sin())
    }

    fn range(heights: &[f32], valid: &[u8]) -> (f32, f32) {
        let mut it = heights.iter().zip(valid).filter(|(_, &v)| v != 0).map(|(&h, _)| h);
        let first = it.next().unwrap_or(0.0);
        it.fold((first, first), |(lo, hi), h| (lo.min(h), hi.max(h)))
    }

    fn params_for(heights: &[f32], valid: &[u8], side: usize, light: Vec3) -> MarchParams {
        let (lo, hi) = range(heights, valid);
        march_params(light, TEXEL, side as u32, lo, hi).unwrap()
    }

    #[test]
    fn update_error_bound() {
        let (h, longest) = (2_000.0f32, 1.0e9f32);
        for deg in [5.0f32, 15.0, 45.0] {
            let e = deg.to_radians();
            let de = 1.0e-4;
            let from = toward(e, 1.0);
            // Elevation: first order h·csc²(e)·Δe.
            let d = edge_displacement(from, toward(e + de, 1.0), h, longest);
            let first = h / e.sin().powi(2) * de;
            assert!((d - first).abs() / first < 0.01, "{deg}°: {d} against {first}");
            // Azimuth: the tip swings by h·cot(e)·Δa (linear in Δa: a larger step keeps the
            // f32 noise of the recomputed angles well under the tolerance).
            let da = 1.0e-3;
            let d = edge_displacement(from, toward(e, 1.0 + da), h, longest);
            let swing = h / e.tan() * da;
            assert!((d - swing).abs() / swing < 0.01, "{deg}°: {d} against {swing}");
            assert_eq!(edge_displacement(from, from, h, longest), 0.0);
        }
        // Across north the azimuth difference wraps.
        let d = edge_displacement(toward(0.3, 0.01), toward(0.3, std::f32::consts::TAU - 0.01), h, longest);
        assert!((d - h / 0.3f32.tan() * 0.02).abs() < 1.0, "{d}");
        // A shadow cannot be longer than the map: at a grazing light the bound stops growing.
        let map = SIDE as f32 * TEXEL * std::f32::consts::SQRT_2;
        assert!(edge_displacement(toward(0.002, 0.0), toward(0.003, 0.0), h, map) <= map);
        assert_eq!(edge_displacement(toward(0.001, 0.0), toward(0.0011, 0.0), h, map), 0.0, "both capped");
        // No horizontal part: always update.
        assert!(edge_displacement(Vec3::Z, toward(1.0, 0.0), h, map).is_infinite());
    }

    #[test]
    fn no_update_while_sun_and_heights_stand() {
        let light = toward(0.4, 2.0);
        let basis = Basis { toward: light, heightfield_version: 7, requested_at: 0.0 };
        let (relief, longest) = (3_000.0, 185_000.0);
        assert_eq!(update_reason(None, 0.0, 7, light, relief, TEXEL, longest), Some(Reason::First));
        assert_eq!(update_reason(Some(&basis), 10.0, 7, light, relief, TEXEL, longest), None);
        // A move whose edge shift stays under half a texel.
        let small = toward(0.4 + 1.0e-5, 2.0);
        assert!(edge_displacement(light, small, relief, longest) < 0.5 * TEXEL);
        assert_eq!(update_reason(Some(&basis), 10.0, 7, small, relief, TEXEL, longest), None);
        // New heights: a new map, but not within the floor.
        assert_eq!(update_reason(Some(&basis), 10.0, 8, light, relief, TEXEL, longest), Some(Reason::Heights));
        assert_eq!(update_reason(Some(&basis), 0.05, 8, light, relief, TEXEL, longest), None);
        // A move past half a texel.
        let big = toward(0.41, 2.0);
        assert!(matches!(update_reason(Some(&basis), 10.0, 7, big, relief, TEXEL, longest), Some(Reason::SunMoved(d)) if d > 0.5 * TEXEL));
    }

    /// The smallest elevation step (radians) that asks for a new map at `deg` degrees.
    fn allowed_step(deg: f32, relief: f32) -> f32 {
        let e = deg.to_radians();
        let basis = Basis { toward: toward(e, 1.0), heightfield_version: 1, requested_at: 0.0 };
        let (mut lo, mut hi) = (0.0f32, 0.5f32);
        for _ in 0..60 {
            let mid = 0.5 * (lo + hi);
            if update_reason(Some(&basis), 1.0, 1, toward(e + mid, 1.0), relief, TEXEL, 1.0e9).is_some() {
                hi = mid;
            } else {
                lo = mid;
            }
        }
        hi
    }

    #[test]
    fn low_sun_updates_more_often() {
        let relief = 2_000.0;
        let steps: Vec<f32> = [5.0, 15.0, 45.0].iter().map(|&d| allowed_step(d, relief)).collect();
        assert!(steps[0] < steps[1] && steps[1] < steps[2], "{steps:?}");
        // Half a texel over h·csc²(e): 0.028°, 0.246°, 1.83° for 2,000 units of relief.
        for (got, want) in steps.iter().zip([0.0279f32, 0.2456, 1.833]) {
            assert!((got.to_degrees() - want).abs() / want < 0.05, "{} against {want}", got.to_degrees());
        }
        // Twice the relief, half the step.
        assert!((allowed_step(15.0, 2.0 * relief) / steps[1] - 0.5).abs() < 0.02);
    }

    #[test]
    fn world_to_shadow_uv_matches_heightfield() {
        let mut g = HeightGrid::new(TEXEL, SIDE);
        g.follow((-3, -2));
        let origin = g.origin().unwrap();
        let (uv_scale, uv_offset) = TerrainShadows::uv_transform(origin, TEXEL, SIDE as u32);
        let map = TerrainShadows { origin, texel: TEXEL, side: SIDE as u32, uv_scale, uv_offset, ..default() };
        let c = CELL_SIZE;
        let points = [
            (-1.0, -1.0),
            (-c + 3.5 * TEXEL, -c + 2.5 * TEXEL),
            (0.0, 0.0),
            (-19.0 * c + 0.1, -18.0 * c + 0.1),
            (13.0 * c - 0.5, 14.0 * c - 0.5),
            (5_000.0, -60_000.0),
        ];
        for (gx, gy) in points {
            // Game (x, y) is Bevy (x, −z); the height does not matter.
            let uv = map.world_to_uv(Vec3::new(gx, 123.0, -gy));
            assert!((0.0..1.0).contains(&uv.x) && (0.0..1.0).contains(&uv.y), "({gx}, {gy}): {uv}");
            let (col, row) = ((uv.x * SIDE as f32).floor() as usize, (uv.y * SIDE as f32).floor() as usize);
            assert_eq!(Some(row * SIDE + col), g.texel_index(gx, gy), "({gx}, {gy})");
        }
        // North is +v, east is +u: Bevy −z is north.
        let a = map.world_to_uv(Vec3::new(0.0, 0.0, 0.0));
        let b = map.world_to_uv(Vec3::new(TEXEL, 0.0, -TEXEL));
        assert!(((b - a) * SIDE as f32 - Vec2::ONE).length() < 1e-3);
        // Outside the window: outside 0..1.
        for (gx, gy) in [(-19.0 * c - 1.0, 0.0), (13.0 * c, 0.0), (0.0, 14.0 * c + 1.0)] {
            let uv = map.world_to_uv(Vec3::new(gx, 0.0, -gy));
            assert!(!((0.0..1.0).contains(&uv.x) && (0.0..1.0).contains(&uv.y)), "({gx}, {gy}): {uv}");
            assert_eq!(g.texel_index(gx, gy), None);
        }
    }

    /// `side` × `side` texels at height 0 with flat-topped ridges: columns `cols` at `h`.
    fn ridges(side: usize, bands: &[(std::ops::RangeInclusive<usize>, f32)]) -> (Vec<f32>, Vec<u8>) {
        let heights = (0..side * side)
            .map(|i| bands.iter().filter(|(c, _)| c.contains(&(i % side))).map(|&(_, h)| h).fold(0.0f32, f32::max))
            .collect();
        (heights, vec![1; side * side])
    }

    #[test]
    fn march_matches_reference_on_ridge() {
        let side = 96;
        // A two-texel ridge and a wide one, both flat-topped, at least as long along every
        // ray as the march's longest step here (asserted below).
        let (heights, valid) = ridges(side, &[(40..=41, 500.0), (60..=67, 800.0)]);
        let lights = [
            toward(0.25f32.atan(), -std::f32::consts::FRAC_PI_2),  // from the west
            toward(0.12f32.atan(), -1.9),                          // west-south-west, oblique
            toward(0.03f32.atan(), -1.4),                          // low, west-north-west
            toward(0.06f32.atan(), std::f32::consts::FRAC_PI_2),   // from the east
        ];
        for (k, light) in lights.into_iter().enumerate() {
            let p = params_for(&heights, &valid, side, light);
            let reference = shadow_height_reference(&heights, &valid, side, TEXEL, light);
            let ours = produce_map(&heights, &valid, &p);
            let mut shaded = 0;
            for row in 0..side {
                for col in 0..side {
                    let i = row * side + col;
                    let reach = texel_reach(&p, (col, row));
                    let step = max_step(&p, reach);
                    assert!(step <= 2.0, "light {k}: step {step} longer than the narrow ridge");
                    // Below the lowest height (0) nothing is shaded: compare above it.
                    let (a, r) = (ours[i].0.max(0.0), reference[i].max(0.0));
                    assert!(a <= r + 1e-2, "light {k} ({col}, {row}): march {a} darker than the reference {r}");
                    let bound = p.tan_texel * step + 1e-2;
                    assert!(r - a <= bound, "light {k} ({col}, {row}): march {a}, reference {r}, bound {bound}");
                    if k == 0 {
                        // Along a row with half-texel steps every texel is visited: exact.
                        assert!((a - r).abs() < 1e-2, "({col}, {row}): {a} against {r}");
                    }
                    assert!(!shadowed(0.0, ours[i].0) || shadowed(0.0, reference[i]), "light {k} ({col}, {row}): shaded where the reference is lit");
                    shaded += usize::from(shadowed(0.0, ours[i].0));
                    // G: the caster's distance, consistent with S's fall.
                    if ours[i].0 > NO_SHADOW {
                        let h = heights.iter().copied().fold(0.0f32, f32::max);
                        assert!(ours[i].1 >= 0.0 && ours[i].0 <= h - ours[i].1 * p.tan_texel / TEXEL + 1e-2);
                    } else {
                        assert_eq!(ours[i].1, 0.0);
                    }
                }
            }
            assert!(shaded > side * 4, "light {k}: the ridges shade something ({shaded})");
        }
    }

    #[test]
    fn march_never_shadows_flat_ground() {
        let side = 48;
        let mut heights = vec![100.0f32; side * side];
        let mut valid = vec![1u8; side * side];
        // A hole of unknown terrain: casts nothing, gets nothing.
        for row in 10..20 {
            for col in 30..40 {
                heights[row * side + col] = INVALID_HEIGHT;
                valid[row * side + col] = 0;
            }
        }
        for light in [toward(0.05, 0.3), toward(0.2, -2.0), toward(1.0, 3.0), toward(0.011, 1.57)] {
            let p = params_for(&heights, &valid, side, light);
            let map = produce_map(&heights, &valid, &p);
            assert!(map.iter().all(|&(s, g)| !shadowed(100.0, s) && s == NO_SHADOW && g == 0.0), "light {light}");
        }
        // A step of terrain far lower than the plateau's edge does not matter to the plateau.
        heights.iter_mut().zip(&valid).filter(|(_, &v)| v != 0).take(side).for_each(|(h, _)| *h = 0.0);
        let p = params_for(&heights, &valid, side, toward(0.1, 0.0));
        let map = produce_map(&heights, &valid, &p);
        for (i, &(s, _)) in map.iter().enumerate() {
            if valid[i] != 0 && heights[i] == 100.0 {
                assert!(!shadowed(100.0, s), "texel {i}: {s}");
            }
        }
        // No light to march towards: nothing anywhere.
        assert_eq!(march_params(toward(0.005, 0.0), TEXEL, side as u32, 0.0, 100.0), Err("light at or under the horizon"));
        assert_eq!(march_params(Vec3::Z, TEXEL, side as u32, 0.0, 100.0), Err("light overhead"));
        let off = MarchParams { steps: 0, ..p };
        assert!(produce_map(&heights, &valid, &off).iter().all(|&(s, _)| s == NO_SHADOW));
    }

    #[test]
    fn slope_overshoot_is_within_a_texel_of_relief() {
        // Ground rising towards the light at slope g < tan(e): never shaded in truth, but each
        // texel holds its highest (sunward) edge, so S may stand above the receiver's own
        // texel by up to the relief inside one texel (module notes, "Bias").
        let side = 40;
        let (g, tan_e) = (0.08f32, 0.2f32);
        let heights: Vec<f32> = (0..side * side).map(|i| ((i % side) as f32 + 1.0) * TEXEL * g).collect();
        let valid = vec![1u8; side * side];
        let p = params_for(&heights, &valid, side, toward(tan_e.atan(), std::f32::consts::FRAC_PI_2));
        let map = produce_map(&heights, &valid, &p);
        for row in 0..side {
            for col in 0..side - 1 {
                let i = row * side + col;
                let (s, own) = (map[i].0, heights[i]);
                assert!(s <= own + g * TEXEL + 1e-2, "({col}, {row}): S {s} over {own} by more than a texel's relief");
                // The true ground at the texel's centre lies half a texel's relief under the
                // texel's maximum: unbiased it reads as shaded (acne) ...
                let ground = own - 0.5 * g * TEXEL;
                assert!(shadowed(ground, s), "({col}, {row}): S {s}, ground {ground}");
                // ... and a bias of texel · slope lights it.
                assert!(!shadowed(ground + g * TEXEL, s), "({col}, {row})");
            }
        }
    }

    #[test]
    fn march_ends_at_the_map_edge() {
        let side = 32;
        // A wall along the west edge, the light in the west.
        let (heights, valid) = ridges(side, &[(0..=0, 1_000.0)]);
        let west = toward(0.1f32.atan(), -std::f32::consts::FRAC_PI_2);
        let p = params_for(&heights, &valid, side, west);
        let reference = shadow_height_reference(&heights, &valid, side, TEXEL, west);
        for row in 0..side {
            // The wall itself has nothing west of it inside the map: no shadow.
            assert_eq!(produce_texel(&heights, &valid, (0, row), &p), (NO_SHADOW, 0.0));
            for col in 1..side {
                let (s, g) = produce_texel(&heights, &valid, (col, row), &p);
                let d = (col as f32 - 0.5) * TEXEL;
                assert!((s - reference[row * side + col]).abs() < 1e-2 || (s.max(0.0) - reference[row * side + col].max(0.0)).abs() < 1e-2, "({col}, {row})");
                if s > 0.0 {
                    assert!((s - (1_000.0 - d * 0.1)).abs() < 1e-1 && (g - d).abs() < 1e-1, "({col}, {row}): {s} {g}");
                }
            }
        }
        // The light in the east: the march from the east column leaves the map at once.
        let east = toward(0.1f32.atan(), std::f32::consts::FRAC_PI_2);
        let p = params_for(&heights, &valid, side, east);
        assert!((texel_reach(&p, (side - 1, 5)) - 0.5).abs() < 1e-4);
        for row in 0..side {
            assert_eq!(produce_texel(&heights, &valid, (side - 1, row), &p), (NO_SHADOW, 0.0));
            // The wall is behind every receiver: nothing shades them.
            for col in 1..side {
                assert!(!shadowed(0.0, produce_texel(&heights, &valid, (col, row), &p).0));
            }
        }
        // Diagonal lights towards every corner: the march stays inside (an index outside
        // the map would panic in the mirror).
        for az in [0.7f32, 2.4, -2.4, -0.7] {
            let p = params_for(&heights, &valid, side, toward(0.02, az));
            let _ = produce_map(&heights, &valid, &p);
        }
    }

    #[test]
    fn steps_grow_with_reach() {
        let p = MarchParams { dir: Vec2::X, tan_texel: 1.0, texel: TEXEL, top: 0.0, bottom: 0.0, side: 512, steps: STEPS, step_min: STEP_MIN };
        assert_eq!(max_step(&p, 30.0), STEP_MIN);
        assert_eq!(max_step(&p, 48.0), STEP_MIN);
        let full = 512.0 * std::f32::consts::SQRT_2;
        assert!((max_step(&p, full) - 14.5).abs() < 0.1, "{}", max_step(&p, full));
        // The last sample lands on the reach.
        let b = step_growth(&p, full);
        let n = STEPS as f32;
        assert!((STEP_MIN * n + b * n * n - full).abs() < 1e-2);
    }

    #[test]
    fn shader_is_valid_wgsl() {
        use wgpu::naga;
        // No shader definitions: this one text is the only variant.
        let source = include_str!("terrain_shadows.wesl");
        let module = naga::front::wgsl::parse_str(source).unwrap_or_else(|e| panic!("{}", e.emit_to_string(source)));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::empty()).validate(&module).unwrap_or_else(|e| panic!("{e:?}"));
        // The bindings the Rust layout makes (`layout_descriptor`), in its order.
        let binding = |name: &str| {
            let (_, v) = module.global_variables.iter().find(|(_, v)| v.name.as_deref() == Some(name)).unwrap_or_else(|| panic!("{name}"));
            let b = v.binding.as_ref().unwrap();
            (b.group, b.binding, v.ty)
        };
        assert_eq!((binding("heights").0, binding("heights").1), (0, 0));
        assert_eq!((binding("shadow_out").0, binding("shadow_out").1), (0, 1));
        let (group, slot, ty) = binding("params");
        assert_eq!((group, slot), (0, 2));
        match &module.types[ty].inner {
            naga::TypeInner::Struct { members, span } => {
                assert_eq!(*span as usize, PARAMS_BYTES);
                let offsets: Vec<(&str, u32)> = members.iter().map(|m| (m.name.as_deref().unwrap_or(""), m.offset)).collect();
                for (name, offset) in [("dir", 0), ("tan_texel", 8), ("texel", 12), ("top", 16), ("bottom", 20), ("side", 24), ("steps", 28), ("step_min", 32)] {
                    assert!(offsets.contains(&(name, offset)), "{name} at {offset}: {offsets:?}");
                }
            }
            other => panic!("params is {other:?}"),
        }
        match &module.types[binding("shadow_out").2].inner {
            naga::TypeInner::Image { class: naga::ImageClass::Storage { format, access }, .. } => {
                assert_eq!(*format, naga::StorageFormat::Rg32Float);
                assert!(access.contains(naga::StorageAccess::STORE));
            }
            other => panic!("shadow_out is {other:?}"),
        }
        let entry = module.entry_points.iter().find(|e| e.name == "produce").expect("entry point `produce`");
        assert_eq!(entry.stage, naga::ShaderStage::Compute);
        assert_eq!(entry.workgroup_size, [WORKGROUP, WORKGROUP, 1]);
        // The CPU mirror's constants.
        assert!(source.contains("const NO_SHADOW: f32 = -1.0e30;"));
        assert_eq!(NO_SHADOW, -1.0e30);
        // The uniform bytes put each field where the shader reads it.
        let p = MarchParams { dir: Vec2::new(0.6, -0.8), tan_texel: 51.2, texel: TEXEL, top: 900.0, bottom: -20.0, side: 512, steps: STEPS, step_min: STEP_MIN };
        let bytes = params_bytes(&p);
        let f = |o: usize| f32::from_le_bytes(bytes[o..o + 4].try_into().unwrap());
        let u = |o: usize| u32::from_le_bytes(bytes[o..o + 4].try_into().unwrap());
        assert_eq!((f(0), f(4), f(8), f(12), f(16), f(20), u(24), u(28), f(32)), (0.6, -0.8, 51.2, TEXEL, 900.0, -20.0, 512, STEPS, STEP_MIN));
    }
}
