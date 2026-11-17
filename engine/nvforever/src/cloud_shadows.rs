//! Cloud shadows (#2670, stage C1): the producer of the cloud transmittance the sun's
//! direct light is multiplied by. An enhanced-look engineering choice, not a parity item
//! (`docs/design/graphics-enhanced.md`); the original casts no cloud shadows.
//!
//! What the weather's clouds are today (sky.rs, fnv_world::sky):
//! - Four dome shapes from `meshes\sky\clouds.nif`, each drawn twice (two sets): set 0
//!   shows the current weather's layer at the fade's alpha, set 1 the previous weather's at
//!   `1 - fade`; with no fade running only set 0 shows, at alpha 1 (sky.rs `update_clouds`).
//!   A layer's on-screen alpha is texture alpha × the shape's vertex alpha (a fade towards
//!   the horizon) × that weight (sky.rs `setup_sky`, sky.wesl `fragment`).
//! - Each layer scrolls in U only, by `ONAM speed / 25,600` texture repeats per elapsed
//!   real second, each weather at its own speeds (fnv_world::sky `CLOUD_SCROLL_PER_UNIT`,
//!   a hypothesis there).
//! - The dome follows the camera and is hidden indoors (sky.rs `update_clouds`).
//!
//! What this module makes from them:
//! - **Composition** [engineering choice]: one 512 × 512 `R8Unorm` repeat-wrapped image
//!   holding the transmission `Π (1 - alpha × weight)` over the drawn layers of both sets
//!   (1 = fully lit), from each texture's ALPHA (decoded from the DDS file's top level by
//!   `nvf_assets::texture::alpha_map`: the GPU copies the texture cache makes keep no CPU
//!   data, `RenderAssetUsages::RENDER_WORLD`). Composed on the compute pool, never on the
//!   main thread. The shape's vertex alpha is left out: it fades the layer towards the
//!   dome's horizon, a property of where on the dome a texel is drawn, which a world plane
//!   does not have [engineering choice].
//! - **Scrolling**: the reference layer (the heaviest) scrolls by `uv_offset`, every frame,
//!   without recomposing; layers moving at other speeds (one weather's layers differ, and
//!   two weathers during a fade) are shifted against it inside the image, recomposed when
//!   one has moved by [`RECOMPOSE_STEP_TEXELS`] and at most every [`MIN_RECOMPOSE_SECS`].
//!   At the largest possible speed difference (ONAM 255 against 0: 0.01 repeats/s, about
//!   5 texels a second) that is 4 compositions a second; at NVWastelandClear's 65 against
//!   0 about 1.3.
//! - **Projection** (consumed by the sun-visibility shader, C0): `hit = P + L ×
//!   (plane_height - P.y) / L.y`, `uv = hit.xz × uv_scale + uv_offset`
//!   ([`projected_uv`] is the reference). World-anchored: no term follows the camera, so
//!   the pattern moves only with the layers' drift. The dome is camera-relative and its UV
//!   layout is the mesh's, so the ground pattern cannot register with the dome exactly
//!   [hypothesis, fork-notes astra-plan §4]; the plane, the repeat and the strength are
//!   constants to calibrate from pictures.
//! - **Drift direction** [engineering choice]: the dome's +U offset (which moves its
//!   clouds towards -U on the mesh) is mapped to the weather's wind direction
//!   (`tree::wind_vector`: fixed, or a script's SetWindDirection), so the ground pattern
//!   drifts downwind at the layers' own speeds, one texture repeat per [`REPEAT_UNITS`]
//!   (a lead, not a mechanism: the GECK wiki's Weather page says the wind speed "affects
//!   cloud, grass, tree movement" [documented: geck-wiki snapshot 2026-10-28, Weather]).
//!   The texture's own axes stay on world X/Z (a rotated repeat would not tile).
//!
//! It applies outdoors only, day and night alike (at night the directional light is the
//! moon's, attenuated the same way, no strength added); off, indoors, or with no drawn,
//! textured cloud layer, the output is `None`. Off by default ([`CloudShadows`]; C2 adds
//! the settings row); `NVF_PLAY_DEBUG_CLOUD_SHADOWS=1` turns it on. Off, nothing runs
//! beyond the run condition and no image is held.
//!
//! Test aids: `NVF_PLAY_DEBUG_CLOUD_SHADOW=<plane height>,<repeat>,<strength>[,<direction
//! deg>]` overrides the calibration constants (direction in game degrees from +x towards
//! +y, in place of the wind's), and `NVF_PLAY_DEBUG_CLOUD_PHASE=<seconds>` freezes the
//! clouds' scroll clock, the dome's and this one ([`cloud_seconds`]), for screenshots.

use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::tasks::{futures::check_ready, AsyncComputeTaskPool, Task};
use fnv_world::sky::Weather;
use nvf_assets::GameData;
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

/// Side of the transmittance image in texels [engineering choice, astra-plan §4: 256 KB,
/// one texel about 117 units at [`REPEAT_UNITS`]].
pub const MAP_SIZE: usize = 512;
/// Height of the cloud plane above the worldspace's ground reference, game units
/// [engineering choice: to be calibrated from pictures].
pub const PLANE_ABOVE_GROUND: f32 = 20_000.0;
/// World size of one texture repeat on the plane, game units [engineering choice: to be
/// calibrated from pictures].
pub const REPEAT_UNITS: f32 = 60_000.0;
/// How much of the sun's direct light a fully opaque cloud takes away: K = mix(1, T,
/// strength) [engineering choice: to be calibrated from pictures].
pub const STRENGTH: f32 = 0.6;
/// The ground reference (the camera's height when the worldspace's first output is made)
/// is rounded to this, game units [engineering choice: one LAND height step band, so a
/// start a few units higher gives the same plane].
const GROUND_SNAP: f32 = 1024.0;
/// A layer shifted against the reference by this many texels since the last composition
/// is composed again [engineering choice: a quarter texel, about 29 units on the ground,
/// too small to see as a step in a soft cloud edge].
pub const RECOMPOSE_STEP_TEXELS: f64 = 0.25;
/// Least real time between two compositions, seconds [engineering choice: 4 a second at
/// most, each a few milliseconds on the compute pool and a 256 KB upload].
pub const MIN_RECOMPOSE_SECS: f64 = 0.25;
/// A layer weight moved by this much is composed again [engineering choice: a 30 s
/// weather fade then composes about 64 times, under the rate cap].
const WEIGHT_STEP: f32 = 1.0 / 64.0;
/// Weights are logged again when one moved by this much [engineering choice: four lines
/// over a fade].
const LOG_WEIGHT_STEP: f32 = 0.25;
/// The published offset is renewed when it moved by this much of a repeat (1/8 texel,
/// about 15 units) [engineering choice: the consumer's uniform is not touched every frame
/// for an invisible drift].
const PUBLISH_OFFSET_STEP: f32 = 1.0 / 4096.0;
/// Below this height of the direction to the light (sine of its elevation) the plane is
/// not reached [engineering choice: the light has faded out within 0.1 rad of the horizon,
/// sky.rs `HORIZON_FADE`].
#[cfg_attr(not(test), allow(dead_code))] // used by `projected_uv`, the shader contract's reference
pub const MIN_LIGHT_Y: f32 = 0.02;

/// Whether cloud shadows are made: off by default (C2 adds the settings row);
/// `NVF_PLAY_DEBUG_CLOUD_SHADOWS=1` turns them on for now.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CloudShadows {
    pub enabled: bool,
}

/// Run condition: the producer runs only while cloud shadows are on.
pub fn cloud_shadows_on(s: Option<Res<CloudShadows>>) -> bool {
    s.is_some_and(|s| s.enabled)
}

/// What the sun-visibility plumbing samples: the fields of C0's `CloudVisibility`.
#[derive(Clone, Debug, PartialEq)]
pub struct CloudShadowOutput {
    /// `R8Unorm`, repeat-wrapped, linear-filtered; 1 = fully lit.
    pub image: Handle<Image>,
    /// World (Bevy x, z) to texture coordinates: 1 / [`REPEAT_UNITS`] on both axes.
    pub uv_scale: Vec2,
    /// The reference layer's drift, 0..1 on both axes.
    pub uv_offset: Vec2,
    /// Bevy y of the cloud plane.
    pub plane_height: f32,
    pub strength: f32,
}

/// The published output: `None` when off, indoors, or with no textured cloud layer.
#[derive(Resource, Default, Debug)]
pub struct CloudShadowPublished(pub Option<CloudShadowOutput>);

// ---------------------------------------------------------------------------------------
// ADAPTER, to connect when this branch merges with C0's `sun_visibility.rs`:
//
//     impl crate::cloud_shadows::CloudSink for SunVisibility {
//         fn set_cloud(&mut self, c: Option<&CloudShadowOutput>) {
//             self.cloud = c.map(|c| CloudVisibility { image: c.image.clone(), uv_scale: c.uv_scale,
//                 uv_offset: c.uv_offset, plane_height: c.plane_height, strength: c.strength });
//         }
//     }
//     app.add_systems(Update, cloud_shadows::publish_cloud_shadows::<SunVisibility>
//         .after(cloud_shadows::produce_cloud_shadows));
//
// The sink is written only when the published output changed (`uv_offset` steps of
// [`PUBLISH_OFFSET_STEP`] while the clouds drift).
// ---------------------------------------------------------------------------------------

/// A receiver of the published output (C0's `SunVisibility`).
#[allow(dead_code)] // connected when merged with C0 (the ADAPTER note above)
pub trait CloudSink: Resource {
    fn set_cloud(&mut self, cloud: Option<&CloudShadowOutput>);
}

/// Copy the published output into the sun-visibility resource when it changed.
#[allow(dead_code)] // connected when merged with C0 (the ADAPTER note above)
pub fn publish_cloud_shadows<S: CloudSink>(published: Res<CloudShadowPublished>, mut sink: ResMut<S>) {
    if published.is_changed() {
        sink.set_cloud(published.0.as_ref());
    }
}

pub struct CloudShadowsPlugin;

impl Plugin for CloudShadowsPlugin {
    fn build(&self, app: &mut App) {
        let enabled = nvf_play::dialogue::debug_switch(std::env::var("NVF_PLAY_DEBUG_CLOUD_SHADOWS").ok().as_deref());
        app.insert_resource(CloudShadows { enabled })
            .init_resource::<CloudShadowPublished>()
            .init_resource::<CloudShadowWork>()
            .add_systems(
                Update,
                (produce_cloud_shadows.run_if(cloud_shadows_on), release_cloud_shadows.run_if(resource_changed::<CloudShadows>))
                    .chain()
                    .after(crate::sky::update_clouds),
            );
    }
}

// ---------------------------------------------------------------------------------------
// Test aids and calibration
// ---------------------------------------------------------------------------------------

/// Seconds of the clouds' scroll clock: elapsed time, or the frozen phase
/// `NVF_PLAY_DEBUG_CLOUD_PHASE=<seconds>` (the dome, sky.rs `update_clouds`, uses it too).
pub fn cloud_seconds(time: &Time) -> f64 {
    frozen_phase().unwrap_or_else(|| time.elapsed_secs_f64())
}

fn frozen_phase() -> Option<f64> {
    static PHASE: OnceLock<Option<f64>> = OnceLock::new();
    *PHASE.get_or_init(|| {
        let phase = parse_phase(std::env::var("NVF_PLAY_DEBUG_CLOUD_PHASE").ok().as_deref());
        if let Some(p) = phase {
            info!("clouds: scroll clock frozen at {p} s (NVF_PLAY_DEBUG_CLOUD_PHASE)");
        }
        phase
    })
}

fn parse_phase(value: Option<&str>) -> Option<f64> {
    value.and_then(|v| v.trim().parse::<f64>().ok()).filter(|p| p.is_finite() && *p >= 0.0)
}

/// The projection's calibration ([`PLANE_ABOVE_GROUND`], [`REPEAT_UNITS`], [`STRENGTH`]),
/// and a drift direction in place of the wind's.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Calibration {
    pub plane_above_ground: f32,
    pub repeat: f32,
    pub strength: f32,
    /// Game degrees from +x towards +y.
    pub direction_deg: Option<f32>,
}

impl Default for Calibration {
    fn default() -> Self {
        Self { plane_above_ground: PLANE_ABOVE_GROUND, repeat: REPEAT_UNITS, strength: STRENGTH, direction_deg: None }
    }
}

/// `<height>,<repeat>,<strength>[,<direction deg>]`; an empty or unreadable part keeps
/// its default.
pub fn parse_calibration(value: Option<&str>) -> Calibration {
    let mut c = Calibration::default();
    let Some(value) = value else { return c };
    let parts: Vec<Option<f32>> = value.split(',').map(|p| p.trim().parse::<f32>().ok().filter(|v| v.is_finite())).collect();
    let part = |i: usize| parts.get(i).copied().flatten();
    if let Some(h) = part(0) {
        c.plane_above_ground = h;
    }
    if let Some(r) = part(1).filter(|r| *r > 1.0) {
        c.repeat = r;
    }
    if let Some(s) = part(2) {
        c.strength = s.clamp(0.0, 1.0);
    }
    c.direction_deg = part(3);
    c
}

fn calibration() -> Calibration {
    static CAL: OnceLock<Calibration> = OnceLock::new();
    *CAL.get_or_init(|| parse_calibration(std::env::var("NVF_PLAY_DEBUG_CLOUD_SHADOW").ok().as_deref()))
}

// ---------------------------------------------------------------------------------------
// Pure parts
// ---------------------------------------------------------------------------------------

/// One cloud layer to compose: its texture, its transition weight and its scroll rate.
#[derive(Clone, Debug, PartialEq)]
pub struct LayerInput {
    pub texture: String,
    pub weight: f32,
    /// Texture repeats per real second (fnv_world::sky `Weather::cloud_scroll`).
    pub scroll: f64,
}

/// The textured layers the dome draws, weighted as sky.rs `update_clouds` weights its two
/// sets: the current weather at `fade` and the previous at `1 - fade` while a fade runs,
/// the current alone at 1 otherwise. `drawn`: the layers the dome has a shape for.
pub fn active_layers(current: &Weather, prev: Option<&Weather>, fade: f32, drawn: [bool; 4]) -> Vec<LayerInput> {
    let fade = fade.clamp(0.0, 1.0);
    let sets: [(Option<&Weather>, f32); 2] = match prev {
        None => [(Some(current), 1.0), (None, 0.0)],
        Some(p) => [(Some(current), fade), (Some(p), 1.0 - fade)],
    };
    let mut out = Vec::new();
    for (w, weight) in sets {
        let Some(w) = w else { continue };
        if weight <= 0.0 {
            continue;
        }
        for (layer, on) in drawn.iter().enumerate() {
            let Some(texture) = w.cloud_textures[layer].as_ref().filter(|_| *on) else { continue };
            out.push(LayerInput { texture: texture.clone(), weight, scroll: f64::from(w.cloud_scroll(layer)) });
        }
    }
    out
}

/// A layer's alpha resampled to [`MAP_SIZE`]² (box filter when the source is larger,
/// wrapped bilinear when smaller), rows along V from the top.
pub fn resample_alpha(src: &[u8], width: usize, height: usize) -> Vec<u8> {
    let n = MAP_SIZE;
    if width == 0 || height == 0 || src.len() < width * height {
        return vec![0; n * n];
    }
    let (xs, ys) = (axis_taps(width, n), axis_taps(height, n));
    let mut out = vec![0u8; n * n];
    for (y, ty) in ys.iter().enumerate() {
        for (x, tx) in xs.iter().enumerate() {
            let mut acc = 0.0f32;
            for &(sy, wy) in ty {
                for &(sx, wx) in tx {
                    acc += f32::from(src[sy * width + sx]) * wx * wy;
                }
            }
            out[y * n + x] = acc.round().clamp(0.0, 255.0) as u8;
        }
    }
    out
}

/// For each of `n` target texels along an axis, the source texels and their weights.
fn axis_taps(src: usize, n: usize) -> Vec<Vec<(usize, f32)>> {
    if src >= n {
        let r = src as f32 / n as f32;
        (0..n)
            .map(|i| {
                let (a, b) = (i as f32 * r, (i + 1) as f32 * r);
                let mut taps = Vec::new();
                let mut k = a.floor() as usize;
                while (k as f32) < b && k < src {
                    let (lo, hi) = ((k as f32).max(a), ((k + 1) as f32).min(b));
                    if hi > lo {
                        taps.push((k, (hi - lo) / r));
                    }
                    k += 1;
                }
                taps
            })
            .collect()
    } else {
        (0..n)
            .map(|i| {
                let p = (i as f32 + 0.5) * src as f32 / n as f32 - 0.5;
                let f = p.floor();
                let k0 = (f as i64).rem_euclid(src as i64) as usize;
                vec![(k0, 1.0 - (p - f)), ((k0 + 1) % src, p - f)]
            })
            .collect()
    }
}

/// One layer as composed: its [`MAP_SIZE`]² alpha (`None`: missing, not loaded, or without
/// alpha: no cover), its weight, and where it is sampled against the image, in texture
/// coordinates (any value; wrapped).
#[derive(Clone, Debug)]
pub struct ComposeLayer {
    pub alpha: Option<Arc<[u8]>>,
    pub weight: f32,
    pub shift: [f64; 2],
}

/// The transmittance image: texel (x, y) is `Π (1 - alpha_i(uv + shift_i) × weight_i)`
/// at its centre uv, each layer sampled bilinearly and wrapped; 255 = fully lit.
pub fn compose(layers: &[ComposeLayer]) -> Vec<u8> {
    let n = MAP_SIZE;
    let mut t = vec![1.0f32; n * n];
    for l in layers {
        let weight = l.weight.clamp(0.0, 1.0);
        let Some(a) = l.alpha.as_deref().filter(|a| a.len() == n * n) else { continue };
        if weight <= 0.0 {
            continue;
        }
        // Texel x samples the layer at texel coordinate x + s (centres at integers).
        let split = |s: f64| {
            let s = s.rem_euclid(1.0) * n as f64;
            let f = s.floor();
            ((f as usize) % n, (s - f) as f32)
        };
        let ((ix, fx), (iy, fy)) = (split(l.shift[0]), split(l.shift[1]));
        for y in 0..n {
            let (r0, r1) = (((y + iy) % n) * n, ((y + iy + 1) % n) * n);
            let row = &mut t[y * n..(y + 1) * n];
            for (x, v) in row.iter_mut().enumerate() {
                let (c0, c1) = ((x + ix) % n, (x + ix + 1) % n);
                let top = f32::from(a[r0 + c0]) + (f32::from(a[r0 + c1]) - f32::from(a[r0 + c0])) * fx;
                let bottom = f32::from(a[r1 + c0]) + (f32::from(a[r1 + c1]) - f32::from(a[r1 + c0])) * fx;
                let alpha = (top + (bottom - top) * fy) / 255.0;
                *v *= 1.0 - alpha * weight;
            }
        }
    }
    t.iter().map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8).collect()
}

/// The texture-space drift of a layer scrolling `scroll` repeats per second after `secs`
/// seconds along the unit direction `dir` (texture u, v = world x, z), wrapped to 0..1 on
/// each axis. Worked unwrapped in f64, so a direction off the axes has no jump when the
/// drift passes a whole repeat.
pub fn drift(secs: f64, scroll: f64, dir: Vec2) -> [f64; 2] {
    let d = secs * scroll;
    [(d * f64::from(dir.x)).rem_euclid(1.0), (d * f64::from(dir.y)).rem_euclid(1.0)]
}

/// `uv_offset` for the reference layer: the ground pattern moves along `wind` (Bevy x, z,
/// unit) as the dome's offset grows, so its texture offset runs against it.
pub fn uv_offset(secs: f64, ref_scroll: f64, wind: Vec2) -> Vec2 {
    let [u, v] = drift(secs, ref_scroll, -wind);
    Vec2::new(u as f32, v as f32)
}

/// Where layer `scroll` is sampled against the image composed for the reference layer
/// `ref_scroll`: its drift beyond the reference's.
pub fn layer_shift(secs: f64, scroll: f64, ref_scroll: f64, wind: Vec2) -> [f64; 2] {
    drift(secs, scroll - ref_scroll, -wind)
}

/// Distance between two wrapped texture coordinates, in texels.
fn wrapped_texels(a: f64, b: f64) -> f64 {
    let d = (a - b).rem_euclid(1.0);
    d.min(1.0 - d) * MAP_SIZE as f64
}

// The shader contract's reference: C0's shader implements it, its tests may call it.
#[cfg_attr(not(test), allow(dead_code))]
/// Reference of the shader contract (C0): where a receiver at Bevy `world` samples the
/// image, for `to_light` the unit direction towards the light. `None`: the light is at or
/// below [`MIN_LIGHT_Y`], or the receiver is above the plane (fully lit).
pub fn projected_uv(world: Vec3, to_light: Vec3, plane_height: f32, uv_scale: Vec2, uv_offset: Vec2) -> Option<Vec2> {
    if to_light.y <= MIN_LIGHT_Y {
        return None;
    }
    let t = (plane_height - world.y) / to_light.y;
    if t < 0.0 {
        return None;
    }
    let hit = world + to_light * t;
    Some(Vec2::new(hit.x, hit.z) * uv_scale + uv_offset)
}

/// The drift direction (Bevy x, z, unit): `direction_deg` (game degrees) when given, else
/// the weather's wind (`tree::wind_vector`).
fn drift_direction(sky: &crate::sky::SkyState, direction_deg: Option<f32>) -> Vec2 {
    let d = match direction_deg {
        // Game (x, y) is Bevy (x, -z).
        Some(deg) => Vec2::new(deg.to_radians().cos(), -deg.to_radians().sin()),
        None => {
            let w = crate::tree::wind_vector(sky);
            Vec2::new(w.x, w.z)
        }
    };
    d.try_normalize().unwrap_or(Vec2::X)
}

// ---------------------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------------------

/// A cloud texture's alpha: being read, or read (`None`: missing or without alpha).
enum Alpha {
    Loading(Task<Option<Arc<[u8]>>>),
    Ready(Option<Arc<[u8]>>),
}

/// What an image holds (or a running job will).
#[derive(Clone, Debug, PartialEq)]
struct Composed {
    layers: Vec<ComposedLayer>,
    ref_scroll: f64,
    dir: Vec2,
}

#[derive(Clone, Debug, PartialEq)]
struct ComposedLayer {
    texture: String,
    weight: f32,
    /// The texture's alpha was in.
    covered: bool,
    /// Repeats per second (logged).
    scroll: f64,
    shift: [f64; 2],
}

impl Composed {
    /// Whether `want` differs from this enough to compose again.
    fn differs(&self, want: &Composed) -> bool {
        self.layers.len() != want.layers.len()
            || (self.ref_scroll - want.ref_scroll).abs() > 1e-9
            || (self.dir - want.dir).length() > 1e-4
            || self.layers.iter().zip(&want.layers).any(|(a, b)| {
                a.texture != b.texture
                    || a.covered != b.covered
                    || (a.weight - b.weight).abs() >= WEIGHT_STEP
                    || (a.weight != b.weight && b.weight >= 1.0)
                    || wrapped_texels(a.shift[0], b.shift[0]) >= RECOMPOSE_STEP_TEXELS
                    || wrapped_texels(a.shift[1], b.shift[1]) >= RECOMPOSE_STEP_TEXELS
            })
    }
}

struct Job {
    task: Task<Vec<u8>>,
    composed: Composed,
}

/// The producer's working state (private to this module).
#[derive(Resource, Default)]
pub struct CloudShadowWork {
    image: Option<Handle<Image>>,
    alphas: HashMap<String, Alpha>,
    job: Option<Job>,
    /// What the image holds.
    shown: Option<Composed>,
    /// When the last composition started (cloud clock's real time).
    composed_at: f64,
    /// (worldspace, Bevy y of the ground reference).
    ground: Option<(Option<u32>, f32)>,
    /// Why the output is `None`, as last logged.
    off_reason: Option<&'static str>,
    /// Layer count, weights (in [`LOG_WEIGHT_STEP`] steps) and textures last logged.
    logged: Option<(usize, Vec<i32>, Vec<String>)>,
}

impl CloudShadowWork {
    /// Drop the image and the composition (and the alphas unless `keep_alphas`), and
    /// publish `None` (logged once per reason).
    fn go_off(&mut self, reason: &'static str, keep_alphas: bool, published: &mut CloudShadowPublished, images: &mut Assets<Image>) {
        if let Some(h) = self.image.take() {
            images.remove(&h);
        }
        if !keep_alphas {
            self.alphas.clear();
        }
        self.job = None;
        self.shown = None;
        self.logged = None;
        if published.0.is_some() {
            published.0 = None;
        }
        if self.off_reason != Some(reason) {
            self.off_reason = Some(reason);
            info!("cloud shadows: none ({reason})");
        }
    }
}

fn new_image() -> Image {
    let mut image = Image::new_fill(
        Extent3d { width: MAP_SIZE as u32, height: MAP_SIZE as u32, depth_or_array_layers: 1 },
        TextureDimension::D2,
        &[255],
        TextureFormat::R8Unorm,
        RenderAssetUsages::default(),
    );
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        ..ImageSamplerDescriptor::linear()
    });
    image
}

/// A texture's alpha, once read (`None` too while it is being read).
fn alpha_of(alphas: &HashMap<String, Alpha>, path: &str) -> Option<Arc<[u8]>> {
    match alphas.get(path) {
        Some(Alpha::Ready(a)) => a.clone(),
        _ => None,
    }
}

/// Read and decode one cloud texture's alpha off the main thread.
fn load_alpha(data: &GameData, path: &str) -> Task<Option<Arc<[u8]>>> {
    let (data, path) = (data.clone(), path.to_string());
    nvf_assets::read_pool::read_pool().spawn(async move {
        let resolved = nvf_assets::texture::resolved_path(&path, |q| data.vfs.exists(q));
        let Some(bytes) = data.read(&resolved) else {
            debug!("cloud shadows: {path}: in no archive: no cover");
            return None;
        };
        let Some(map) = nvf_assets::texture::alpha_map(&bytes) else {
            info!("cloud shadows: {path}: no alpha channel read: no cover");
            return None;
        };
        Some(Arc::from(resample_alpha(&map.alpha, map.width, map.height)))
    })
}

/// Make, compose and publish the cloud transmittance (runs while [`CloudShadows`] is on).
#[allow(clippy::too_many_arguments)]
pub fn produce_cloud_shadows(
    mut work: ResMut<CloudShadowWork>,
    mut published: ResMut<CloudShadowPublished>,
    mut images: ResMut<Assets<Image>>,
    sky: Option<Res<crate::sky::SkyState>>,
    data: Option<Res<GameData>>,
    world: Option<Res<crate::world::WorldData>>,
    time: Res<Time>,
    cam: Query<&Transform, With<crate::fly::FlyCam>>,
    dome: Query<&crate::sky::CloudLayer>,
) {
    let work = &mut *work;
    let (Some(sky), Some(data)) = (sky, data) else { return };
    if sky.interior.is_some() {
        work.go_off("indoors", false, &mut published, &mut images);
        return;
    }
    let mut drawn = [false; 4];
    for cl in &dome {
        if let Some(d) = drawn.get_mut(cl.layer()) {
            *d = true;
        }
    }
    let layers = active_layers(&sky.weather, sky.prev.as_ref(), sky.fade, drawn);
    if layers.is_empty() {
        work.go_off("no cloud layers", false, &mut published, &mut images);
        return;
    }
    // The ground reference: the camera's height when the worldspace's first output is
    // made, held until the worldspace changes, so the plane never follows the camera.
    let ws = world.as_deref().map(|w| w.world);
    if work.ground.is_none_or(|(w, _)| w != ws) {
        let Ok(cam) = cam.single() else { return };
        work.ground = Some((ws, (cam.translation.y / GROUND_SNAP).round() * GROUND_SNAP));
    }
    let ground = work.ground.map_or(0.0, |(_, g)| g);

    // Alphas: start the missing reads, take the finished ones, forget unused ones.
    for l in &layers {
        work.alphas.entry(l.texture.clone()).or_insert_with(|| Alpha::Loading(load_alpha(&data, &l.texture)));
    }
    work.alphas.retain(|path, _| layers.iter().any(|l| &l.texture == path));
    for a in work.alphas.values_mut() {
        if let Alpha::Loading(task) = a {
            if let Some(done) = check_ready(task) {
                *a = Alpha::Ready(done);
            }
        }
    }
    let loading = work.alphas.values().any(|a| matches!(a, Alpha::Loading(_)));
    if !loading && layers.iter().all(|l| alpha_of(&work.alphas, &l.texture).is_none()) {
        // Kept: the textures are not read again while the weather stays.
        work.go_off("no cloud alpha", true, &mut published, &mut images);
        return;
    }
    // While a texture is read, what is shown stays (a weather change), or nothing yet.
    if loading && work.shown.is_none() {
        return;
    }

    let cal = calibration();
    let secs = cloud_seconds(&time);
    let dir = drift_direction(&sky, cal.direction_deg);
    // The reference: the heaviest covering layer (the first of equals).
    let reference = layers
        .iter()
        .filter(|l| alpha_of(&work.alphas, &l.texture).is_some())
        .fold(None::<&LayerInput>, |best, l| if best.is_none_or(|b| l.weight > b.weight) { Some(l) } else { best })
        .unwrap_or(&layers[0]);
    let ref_scroll = reference.scroll;
    let want = Composed {
        layers: layers
            .iter()
            .map(|l| ComposedLayer {
                texture: l.texture.clone(),
                weight: l.weight,
                covered: alpha_of(&work.alphas, &l.texture).is_some(),
                scroll: l.scroll,
                shift: layer_shift(secs, l.scroll, ref_scroll, dir),
            })
            .collect(),
        ref_scroll,
        dir,
    };

    // Finished composition: into the image.
    if let Some(job) = work.job.as_mut() {
        if let Some(texels) = check_ready(&mut job.task) {
            let composed = work.job.take().map(|j| j.composed);
            if let Some(mut img) = work.image.as_ref().and_then(|h| images.get_mut(h)) {
                img.data = Some(texels);
                work.shown = composed;
            }
        }
    }
    // A new composition when what is wanted differs, one at a time, at a bounded rate.
    let now = time.elapsed_secs_f64();
    let due = work.shown.as_ref().is_none_or(|s| s.differs(&want));
    if due && !loading && work.job.is_none() && (work.shown.is_none() || now - work.composed_at >= MIN_RECOMPOSE_SECS) {
        if work.image.is_none() {
            work.image = Some(images.add(new_image()));
        }
        let input: Vec<ComposeLayer> =
            layers.iter().zip(&want.layers).map(|(l, w)| ComposeLayer { alpha: alpha_of(&work.alphas, &l.texture), weight: l.weight, shift: w.shift }).collect();
        let task = AsyncComputeTaskPool::get().spawn(async move { compose(&input) });
        work.job = Some(Job { task, composed: want.clone() });
        work.composed_at = now;
    }

    // Publish what the image holds, with its reference layer's drift now.
    let (Some(shown), Some(image)) = (work.shown.as_ref(), work.image.as_ref()) else { return };
    let out = CloudShadowOutput {
        image: image.clone(),
        uv_scale: Vec2::splat(1.0 / cal.repeat),
        uv_offset: uv_offset(secs, shown.ref_scroll, shown.dir),
        plane_height: ground + cal.plane_above_ground,
        strength: cal.strength,
    };
    let renew = match &published.0 {
        None => true,
        Some(p) => {
            let moved = (p.uv_offset - out.uv_offset).abs();
            let moved = moved.min(Vec2::ONE - moved);
            p.image != out.image || p.uv_scale != out.uv_scale || p.plane_height != out.plane_height || p.strength != out.strength || moved.max_element() >= PUBLISH_OFFSET_STEP
        }
    };
    if renew {
        published.0 = Some(out.clone());
    }
    work.off_reason = None;

    // Log when the layers, their weights (in quarter steps) or the projection change.
    let key = (
        shown.layers.len(),
        shown.layers.iter().map(|l| (l.weight / LOG_WEIGHT_STEP).round() as i32).collect::<Vec<_>>(),
        shown.layers.iter().map(|l| l.texture.clone()).collect::<Vec<_>>(),
    );
    if work.logged.as_ref() != Some(&key) {
        let weights: Vec<String> = shown.layers.iter().map(|l| format!("{:.2}", l.weight)).collect();
        let speeds: Vec<String> = shown.layers.iter().map(|l| format!("{:.5}", l.scroll)).collect();
        let textures: Vec<&str> = shown.layers.iter().map(|l| l.texture.as_str()).collect();
        info!(
            "cloud shadows: layers {} weights [{}] speeds [{}] offset ({:.4},{:.4}) strength {:.2} plane {:.0} (ground {:.0}) repeat {:.0} drift ({:.3},{:.3}) textures [{}]",
            shown.layers.len(),
            weights.join(", "),
            speeds.join(", "),
            out.uv_offset.x,
            out.uv_offset.y,
            out.strength,
            out.plane_height,
            ground,
            cal.repeat,
            shown.dir.x,
            shown.dir.y,
            textures.join(", ")
        );
        work.logged = Some(key);
    }
}

/// Switched off: drop the image, the tasks and the output.
pub fn release_cloud_shadows(
    shadows: Res<CloudShadows>,
    mut work: ResMut<CloudShadowWork>,
    mut published: ResMut<CloudShadowPublished>,
    mut images: ResMut<Assets<Image>>,
) {
    if !shadows.enabled && (work.image.is_some() || published.0.is_some() || !work.alphas.is_empty() || work.job.is_some()) {
        work.go_off("switched off", false, &mut published, &mut images);
        work.ground = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn weather(textures: [Option<&str>; 4], speeds: [u8; 4]) -> Weather {
        Weather { cloud_textures: textures.map(|t| t.map(str::to_string)), cloud_speeds: speeds, ..Default::default() }
    }

    #[test]
    fn cloud_transition_endpoints() {
        let now = weather([Some("now0.dds"), None, None, Some("now3.dds")], [10, 0, 0, 20]);
        let was = weather([None, Some("was1.dds"), None, None], [0, 30, 0, 0]);
        let all = [true; 4];
        // No fade: the current weather alone, at 1.
        let l = active_layers(&now, None, 0.3, all);
        assert_eq!(l.iter().map(|l| (l.texture.as_str(), l.weight)).collect::<Vec<_>>(), [("now0.dds", 1.0), ("now3.dds", 1.0)]);
        // Fade 0: only the previous set; fade 1: only the current.
        let start = active_layers(&now, Some(&was), 0.0, all);
        assert_eq!(start.len(), 1);
        assert_eq!((start[0].texture.as_str(), start[0].weight), ("was1.dds", 1.0));
        let end = active_layers(&now, Some(&was), 1.0, all);
        assert_eq!(end.iter().map(|l| l.texture.as_str()).collect::<Vec<_>>(), ["now0.dds", "now3.dds"]);
        assert!(end.iter().all(|l| l.weight == 1.0));
        // Midway: both, weighted fade and 1 - fade, each at its own weather's speed.
        let mid = active_layers(&now, Some(&was), 0.25, all);
        assert_eq!(mid.len(), 3);
        assert_eq!(mid[0].weight, 0.25);
        assert_eq!(mid[2].weight, 0.75);
        assert_eq!(mid[2].scroll, f64::from(was.cloud_scroll(1)));
        // A layer the dome has no shape for is not composed.
        let l = active_layers(&now, None, 1.0, [true, true, true, false]);
        assert_eq!(l.len(), 1);
    }

    fn constant(a: u8) -> Option<Arc<[u8]>> {
        Some(Arc::from(vec![a; MAP_SIZE * MAP_SIZE]))
    }

    #[test]
    fn cloud_transmission_is_product() {
        let a = 128.0 / 255.0;
        let out = compose(&[
            ComposeLayer { alpha: constant(128), weight: 1.0, shift: [0.0; 2] },
            ComposeLayer { alpha: constant(128), weight: 0.5, shift: [0.3, 0.7] },
        ]);
        let want = ((1.0 - a) * (1.0 - 0.5 * a) * 255.0f32).round() as u8;
        assert!(out.iter().all(|&t| t == want), "{} vs {want}", out[0]);
        // Opaque at full weight is black; at weight 0 the layer is not there.
        assert!(compose(&[ComposeLayer { alpha: constant(255), weight: 1.0, shift: [0.0; 2] }]).iter().all(|&t| t == 0));
        assert!(compose(&[ComposeLayer { alpha: constant(255), weight: 0.0, shift: [0.0; 2] }]).iter().all(|&t| t == 255));
    }

    #[test]
    fn cloud_missing_texture_is_clear() {
        assert!(compose(&[]).iter().all(|&t| t == 255));
        let missing = ComposeLayer { alpha: None, weight: 1.0, shift: [0.0; 2] };
        let wrong_size = ComposeLayer { alpha: Some(Arc::from(vec![255u8; 16])), weight: 1.0, shift: [0.0; 2] };
        assert!(compose(&[missing, wrong_size]).iter().all(|&t| t == 255));
        // An unreadable source resamples to no cover.
        assert!(resample_alpha(&[], 0, 0).iter().all(|&a| a == 0));
    }

    #[test]
    fn cloud_shift_samples_ahead() {
        // One opaque texel at (10, 20); a shift of +2 texels in u brings it to (8, 20).
        let n = MAP_SIZE;
        let mut a = vec![0u8; n * n];
        a[20 * n + 10] = 255;
        let out = compose(&[ComposeLayer { alpha: Some(Arc::from(a)), weight: 1.0, shift: [2.0 / n as f64, 0.0] }]);
        assert_eq!(out[20 * n + 8], 0);
        assert_eq!(out.iter().filter(|&&t| t != 255).count(), 1);
        // A whole repeat is no shift.
        let mut b = vec![0u8; n * n];
        b[3 * n + 5] = 255;
        let out = compose(&[ComposeLayer { alpha: Some(Arc::from(b)), weight: 1.0, shift: [1.0, -2.0] }]);
        assert_eq!(out[3 * n + 5], 0);
    }

    #[test]
    fn cloud_resample_keeps_mean() {
        // 1024 wide stripes of 0 and 255 average to the middle; a 256 source keeps its value.
        let src: Vec<u8> = (0..1024 * 1024).map(|i| if i % 2 == 0 { 0 } else { 255 }).collect();
        let out = resample_alpha(&src, 1024, 1024);
        assert!(out.iter().all(|&a| a == 128 || a == 127), "{}", out[0]);
        let flat = resample_alpha(&[200u8; 256 * 128], 256, 128);
        assert!(flat.iter().all(|&a| a == 200));
    }

    #[test]
    fn cloud_projection_accounts_for_height() {
        let (scale, offset, plane) = (Vec2::splat(1.0 / REPEAT_UNITS), Vec2::new(0.25, 0.5), 20_000.0);
        let to_light = Vec3::new(0.5, 0.8, -0.2).normalize();
        // Two receivers on one ray towards the light share one point of the plane.
        let low = Vec3::new(1_000.0, 0.0, -3_000.0);
        let high = low + to_light * 900.0;
        let (a, b) = (projected_uv(low, to_light, plane, scale, offset).unwrap(), projected_uv(high, to_light, plane, scale, offset).unwrap());
        assert!((a - b).length() < 1e-5, "{a} {b}");
        // A raised receiver straight above the low one does not.
        let above = projected_uv(low + Vec3::Y * 900.0, to_light, plane, scale, offset).unwrap();
        assert!((above - a).length() > 1e-3);
        // Straight overhead light: x, z scaled plus the offset; the camera plays no part.
        let up = projected_uv(low, Vec3::Y, plane, scale, offset).unwrap();
        assert!((up - (Vec2::new(low.x, low.z) * scale + offset)).length() < 1e-6);
        // Above the plane, or the light at the horizon: not reached.
        assert!(projected_uv(Vec3::Y * 25_000.0, to_light, plane, scale, offset).is_none());
        assert!(projected_uv(low, Vec3::X, plane, scale, offset).is_none());
    }

    #[test]
    fn cloud_drift_follows_wind_without_jumps() {
        let wind = Vec2::new(0.6, -0.8);
        let scroll = 65.0 / 25_600.0;
        // Across a whole repeat of drift the offset moves smoothly (wrapped per axis).
        let one = 1.0 / scroll;
        for secs in [0.0, one - 0.5, one, 7.3 * one, 36_000.0] {
            let (a, b) = (uv_offset(secs, scroll, wind), uv_offset(secs + 0.5, scroll, wind));
            let d = (a - b).abs();
            let d = d.min(Vec2::ONE - d);
            assert!(d.length() < 0.5 * scroll as f32 * 1.01 + 1e-6, "{secs}: {a} {b}");
        }
        // The pattern moves downwind: a texel of the image lands further along the wind.
        let (t0, t1) = (uv_offset(10.0, scroll, wind), uv_offset(20.0, scroll, wind));
        let world_move = -(t1 - t0) * REPEAT_UNITS;
        assert!(world_move.dot(wind) > 0.0);
        // Layers at the reference's speed are not shifted; others by their extra drift.
        assert_eq!(layer_shift(1234.5, scroll, scroll, wind), [0.0, 0.0]);
        let s = layer_shift(100.0, 2.0 * scroll, scroll, wind);
        let o = uv_offset(100.0, scroll, wind);
        assert!(((s[0] as f32) - o.x).abs() < 1e-5 && ((s[1] as f32) - o.y).abs() < 1e-5);
    }

    #[test]
    fn cloud_recompose_needs_a_material_change() {
        let layer = |weight: f32, shift: [f64; 2]| ComposedLayer { texture: "a.dds".into(), weight, covered: true, scroll: 0.001, shift };
        let base = Composed { layers: vec![layer(0.5, [0.0, 0.0])], ref_scroll: 0.001, dir: Vec2::X };
        let mut same = base.clone();
        same.layers[0].shift = [0.1 / MAP_SIZE as f64, 0.0];
        same.layers[0].weight = 0.5 + WEIGHT_STEP * 0.5;
        assert!(!base.differs(&same));
        let mut moved = base.clone();
        moved.layers[0].shift = [1.0 - 0.3 / MAP_SIZE as f64, 0.0];
        assert!(base.differs(&moved));
        let mut done = base.clone();
        done.layers[0].weight = 1.0;
        assert!(base.differs(&done));
        let mut other = base.clone();
        other.layers[0].texture = "b.dds".into();
        assert!(base.differs(&other));
    }

    #[test]
    fn cloud_calibration_parses() {
        assert_eq!(parse_calibration(None), Calibration::default());
        let c = parse_calibration(Some("15000, 40000,0.8"));
        assert_eq!((c.plane_above_ground, c.repeat, c.strength, c.direction_deg), (15_000.0, 40_000.0, 0.8, None));
        let c = parse_calibration(Some(",,2,90"));
        assert_eq!((c.plane_above_ground, c.repeat, c.strength, c.direction_deg), (PLANE_ABOVE_GROUND, REPEAT_UNITS, 1.0, Some(90.0)));
        assert_eq!(parse_calibration(Some("x,0,y")).repeat, REPEAT_UNITS);
        assert_eq!(parse_phase(Some(" 120.5 ")), Some(120.5));
        assert_eq!(parse_phase(Some("-1")), None);
        assert_eq!(parse_phase(Some("soon")), None);
    }
}
