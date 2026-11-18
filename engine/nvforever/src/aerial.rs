//! Aerial perspective (`aerial-perspective`, docs/design/graphics-enhanced.md §3.17, #2993):
//! an enhanced-look switch, off by default and in every preset, never a parity item. Far
//! scenery takes on the sky's colours with distance instead of fading to the one fog colour:
//! bluer with distance under a blue sky, warmer under a dusk sky, brighter toward the sun, so
//! successive ridgelines separate. Nothing here is a fact about the original game [design].
//!
//! **Technique.** After Hoffman and Preetham, "Rendering Outdoor Light Scattering in Real
//! Time" (GDC 2002) [documented: cited paper]: the light reaching the eye is the surface's
//! colour times the extinction plus the in-scattered light times one minus it, the extinction
//! differing per colour channel and the in-scattered light depending on the view ray's
//! direction (a phase function toward the sun). Our own code from that description, no code
//! from any other project. The amounts are the weather's, not a physical atmosphere's:
//! - *Per-channel share*: the weather's fog share f becomes f_c = 1 − (1 − f)^(w_c) per
//!   channel. The weights w_c come from the chroma of the weather's horizon colour at this hour
//!   ([`chroma_weights`]); their luminance-weighted mean is 1, each lies in
//!   [`WEIGHT_LOW`]..[`WEIGHT_HIGH`] (the chroma's pull is scaled down, never clamped, to stay
//!   there), so the weather's fog keeps its amount and only its colour balance over distance
//!   changes.
//! - *In-scatter colour*: instead of the fog colour, the fog colour moved toward the sky colour
//!   in the ray's direction (horizon to sky-lower by the ray's elevation, as the dome mixes
//!   them, sky.rs `dome_color`), then toward the weather's sun colour by a normalised
//!   Henyey-Greenstein lobe ([`lobe`]; Henyey and Greenstein, "Diffuse radiation in the
//!   galaxy", ApJ 93, 1941) toward the real sun (the light turns to the moon's side below the
//!   horizon; the shader turns it back), fading out as the sun sets. Mixes only: the colour
//!   stays within the weather's own colours, the sky's as the dome draws them (#2915: nothing
//!   is pushed above what the sky itself shows).
//! - Both scale with the weight the desert haze uses ([`crate::haze::weight`]): full under a
//!   clear weather, none as the weather's own fog gets short or dense (a hazy weather, the dust
//!   storm keep their look exactly).
//!
//! **Where it sits.** Per fragment in the world shaders' fog path, no pass: `nvf_aerial` (put
//! before Bevy's `apply_fog`) wraps the desert haze's call of the atmospheric fog (haze.rs
//! `HAZE_FOG_CALL`; composed, Bevy's own call is not replaced again), works out the in-scatter
//! colour for the fragment's view ray and hands it, with the three weights, to the game's fog
//! (fog.rs `GAME_FOG`) in a copy of the fog uniform; `GAME_FOG` then blends once, with the haze
//! over it if that is on. Display values (the pow(1/2.2) encoding `GAME_FOG` blends in
//! outdoors) throughout; the main view is HDR, but the fog blend is in that encoding.
//!
//! **Carried** in three vec4s appended to Bevy's fog uniform (`DistanceFog::nvf_params`, a
//! patch in engine/vendor/bevy_pbr; the view bind group does not grow): [`pack`]. All zero is
//! off: `nvf_aerial` makes Bevy's call and `GAME_FOG` runs its old lines.
//!
//! **What follows the weather** (live from `SkyState` every frame, blended through fades): the
//! weight from its fog numbers, the weights from its horizon colour, the in-scatter colours
//! from its fog, horizon, sky-lower and sun colours and the dome's sky level, the lobe from the
//! sun's height. No weather names, no hour table, no result kept past a change.

use bevy::pbr::DistanceFog;
use bevy::prelude::*;
use bevy::shader::Shader;
use fnv_world::sky::{smoothstep, ColorType};
use std::sync::atomic::{AtomicU8, Ordering};

use crate::fly::FlyCam;
use crate::sky::SkyState;

// ---- Constants (the one place; every value an engineering choice by eye [design]) ----

/// How strongly the horizon colour's chroma sets the channel weights: w = 1 + gain × (ρ − 1),
/// ρ the channel over the colour's luminance (held to the weight range below).
pub const CHROMA_GAIN: f32 = 1.0;
/// The smallest channel weight: that channel fogs in as the weather's share to the power 0.5.
pub const WEIGHT_LOW: f32 = 0.5;
/// The largest channel weight.
pub const WEIGHT_HIGH: f32 = 2.0;
/// How far the in-scatter colour moves from the weather's fog colour toward the sky colour in
/// the ray's direction (at full weight): a far ridge meets mostly the sky drawn behind it.
pub const DOME_MIX: f32 = 0.8;
/// The ray's elevation (sine) at which the sky colour is the sky-lower colour; at and below
/// the horizon it is the horizon colour (sin 20°: far land is seen within a few degrees).
pub const LOWER_SIN: f32 = 0.342_020_14;
/// The Henyey-Greenstein asymmetry of the sun lobe (forward scattering).
pub const HG_G: f32 = 0.6;
/// How far the in-scatter colour moves toward the sun colour looking straight at the sun (at
/// full weight, the sun up).
pub const LOBE_MOST: f32 = 0.35;
/// The sun lobe fades in as the sun's elevation rises through these degrees, as the sky's disc
/// and glare do (sky.rs `SkyState::light`): nothing at night.
pub const SUN_FADE_DEG: (f32, f32) = (-2.0, 3.0);
/// Rec. 709 luminance weights (as sky.rs `SkyState::sample`).
pub const LUMA: [f32; 3] = [0.2126, 0.7152, 0.0722];
/// The debug multiplier's largest value.
pub const DEBUG_MOST: f32 = 4.0;

/// The debug aid: `NVF_PLAY_DEBUG_AERIAL=<multiplier>` scales the weight (1 as designed, 0 to
/// 4; for pictures at several strengths). It does not turn the switch on.
pub const DEBUG_VAR: &str = "NVF_PLAY_DEBUG_AERIAL";

// ---- The pure parts ----

/// A linear colour channel as the display value `GAME_FOG` blends in (pow 1/2.2); 0 for a
/// negative or not finite value.
pub fn display(v: f32) -> f32 {
    if v.is_finite() && v > 0.0 { v.powf(1.0 / 2.2) } else { 0.0 }
}

/// Rec. 709 luminance.
pub fn luminance(c: [f32; 3]) -> f32 {
    LUMA[0] * c[0] + LUMA[1] * c[1] + LUMA[2] * c[2]
}

/// The channel weights from a sky colour (linear) and a gain: ρ_c = c_c / luminance, whose
/// luminance-weighted mean is 1, then w_c = 1 + k (ρ_c − 1) with k the gain, or less where
/// the gain would take a weight out of [`WEIGHT_LOW`]..[`WEIGHT_HIGH`] (w is linear in k and 1
/// at k = 0, so the mean stays 1 and the hue's direction stays). A grey sky, a black one, a
/// gain of 0 or anything not finite: 1, 1, 1.
pub fn chroma_weights(sky: [f32; 3], gain: f32) -> [f32; 3] {
    let c = sky.map(|v| if v.is_finite() { v.max(0.0) } else { 0.0 });
    let y = luminance(c);
    if !(y > 1e-6 && y.is_finite() && gain.is_finite() && gain > 0.0) {
        return [1.0; 3];
    }
    let rho = c.map(|v| v / y);
    let mut k = gain;
    for r in rho {
        let d = r - 1.0;
        if d > 1e-6 {
            k = k.min((WEIGHT_HIGH - 1.0) / d);
        } else if d < -1e-6 {
            k = k.min((WEIGHT_LOW - 1.0) / d);
        }
    }
    rho.map(|r| 1.0 + k * (r - 1.0))
}

/// Each channel's share of the in-scatter colour for the weather's share `f`: 1 − (1 − f)^w
/// (what `GAME_FOG`'s aerial branch computes). 0 at f = 0, 1 at f = 1, rising with f.
#[cfg_attr(not(test), allow(dead_code))]
pub fn channel_shares(f: f32, w: [f32; 3]) -> [f32; 3] {
    let kept = 1.0 - if f.is_finite() { f.clamp(0.0, 1.0) } else { 0.0 };
    w.map(|w| if kept > 0.0 { 1.0 - kept.powf(w.max(0.0)) } else { 1.0 })
}

/// The Henyey-Greenstein phase function for the cosine between the view ray and the sun,
/// divided by its value straight at the sun: ((1 − g)² / (1 + g² − 2 g cos))^1.5, 1 at the sun,
/// within 0..1 everywhere (what `nvf_aerial` computes).
#[cfg_attr(not(test), allow(dead_code))]
pub fn lobe(cos: f32) -> f32 {
    let g = HG_G;
    let c = if cos.is_finite() { cos.clamp(-1.0, 1.0) } else { -1.0 };
    let q = (1.0 - g) * (1.0 - g) / (1.0 + g * g - 2.0 * g * c).max(1e-6);
    q * q.sqrt()
}

/// The sky's colours now (linear RGB, `SkyState::color`) and what else the aerial terms read.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SkyColours {
    pub fog: [f32; 3],
    pub horizon: [f32; 3],
    pub lower: [f32; 3],
    pub sun: [f32; 3],
    /// The dome's sky level (`SkyState::sky_scale`, by which sky.wesl scales the dome).
    pub sky_scale: f32,
    /// The sun's elevation in degrees.
    pub sun_elevation: f32,
    /// Whether the directional light points at the sun (its own clock's sun at or above the
    /// horizon) rather than the opposite side (sky.rs `aim_sun` turns it to the moon's side
    /// below the horizon).
    pub light_is_sun: bool,
}

/// What the CPU works out each frame for the uniform; colours in display values.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aerial {
    /// The channel weights.
    pub weights: [f32; 3],
    /// How far the in-scatter colour moved toward the sky colour.
    pub sky_share: f32,
    /// The in-scatter colour looking at the horizon and up at the sky-lower height.
    pub horizon: [f32; 3],
    pub lower: [f32; 3],
    /// The sun colour and the lobe's strength looking straight at the sun (0..1).
    pub sun: [f32; 3],
    pub lobe: f32,
    /// The directional light points at the sun (else away from it).
    pub light_is_sun: bool,
}

/// The aerial terms for `strength` (the weight times the debug multiplier) under `sky`;
/// `None` (off, the uniform all zero) for a strength not above 0.
pub fn aerial(strength: f32, sky: &SkyColours) -> Option<Aerial> {
    if !(strength.is_finite() && strength > 0.0) {
        return None;
    }
    let a = strength.min(DEBUG_MOST);
    let scale = if sky.sky_scale.is_finite() { sky.sky_scale.max(0.0) } else { 1.0 };
    let fog = sky.fog.map(display);
    let sky_share = (a * DOME_MIX).min(1.0);
    let toward = |c: [f32; 3]| -> [f32; 3] {
        std::array::from_fn(|i| {
            let d = display(c[i] * scale);
            fog[i] + (d - fog[i]) * sky_share
        })
    };
    let up = if sky.sun_elevation.is_finite() { smoothstep(SUN_FADE_DEG.0, SUN_FADE_DEG.1, sky.sun_elevation) } else { 0.0 };
    Some(Aerial {
        weights: chroma_weights(sky.horizon, a * CHROMA_GAIN),
        sky_share,
        horizon: toward(sky.horizon),
        lower: toward(sky.lower),
        sun: sky.sun.map(display),
        lobe: (a * LOBE_MOST).min(1.0) * up,
        light_is_sun: sky.light_is_sun,
    })
}

/// The fog uniform's three vectors (`DistanceFog::nvf_params`): [0] the sun colour and the
/// lobe's strength (negative while the directional light points away from the sun, -1..1),
/// [1] the in-scatter colour at the horizon and the red weight, [2] the one at
/// the sky-lower height and the blue weight (green follows from the mean of 1, in the
/// shader). All zero: off.
pub fn pack(a: Option<&Aerial>) -> [Vec4; 3] {
    let Some(a) = a else { return [Vec4::ZERO; 3] };
    [
        Vec4::new(a.sun[0], a.sun[1], a.sun[2], if a.light_is_sun { a.lobe } else { -a.lobe }),
        Vec4::new(a.horizon[0], a.horizon[1], a.horizon[2], a.weights[0]),
        Vec4::new(a.lower[0], a.lower[1], a.lower[2], a.weights[2]),
    ]
}

/// What `nvf_aerial` resolves from the uniform for a view ray whose elevation's sine is
/// `ray_y` and whose cosine to the directional light is `toward_light`: the in-scatter colour (display values)
/// and the three weights; `None` when off (the shader's mirror, for the tests).
#[cfg(test)]
pub fn resolve(p: &[Vec4; 3], ray_y: f32, toward_light: f32) -> Option<([f32; 3], [f32; 3])> {
    let red = p[1].w;
    if red <= 0.0 {
        return None;
    }
    let blue = p[2].w;
    let green = ((1.0 - LUMA[0] * red - LUMA[2] * blue) / LUMA[1]).max(0.0);
    let t = smoothstep(0.0, LOWER_SIN, ray_y);
    let mut air: [f32; 3] = std::array::from_fn(|i| p[1][i] + (p[2][i] - p[1][i]) * t);
    if p[0].w != 0.0 {
        let toward_sun = if p[0].w > 0.0 { toward_light } else { -toward_light };
        let k = (p[0].w.abs() * lobe(toward_sun)).clamp(0.0, 1.0);
        air = std::array::from_fn(|i| air[i] + (p[0][i] - air[i]) * k);
    }
    Some((air, [red, green, blue]))
}

/// `GAME_FOG`'s aerial branch in display values: the input `input` seen through the weather's
/// fog (share `fog_share`, in-scatter colour `air`, weights `w`) and, with `haze` (its colour
/// in display values and its share), the haze over it; the colour and each channel's total
/// share (the shader's mirror, for the tests).
#[cfg(test)]
pub fn blend(input: [f32; 3], air: [f32; 3], w: [f32; 3], fog_share: f32, haze: Option<([f32; 3], f32)>) -> ([f32; 3], [f32; 3]) {
    let air_share = channel_shares(fog_share, w);
    let (veil, veil_share) = match haze.filter(|(_, h)| *h > 0.0) {
        Some((colour, h)) => {
            let h = h.clamp(0.0, 1.0);
            let share: [f32; 3] = std::array::from_fn(|i| 1.0 - (1.0 - air_share[i]) * (1.0 - h));
            (std::array::from_fn(|i| (air[i] * air_share[i] + colour[i] * h * (1.0 - air_share[i])) / share[i].max(1e-6)), share)
        }
        None => (air, air_share),
    };
    (std::array::from_fn(|i| input[i] + (veil[i] - input[i]) * veil_share[i]), veil_share)
}

/// The debug multiplier from the variable's text: `None` for unset or empty (as designed),
/// else a finite number held to 0..[`DEBUG_MOST`]; `Err` for anything else.
pub fn debug_multiplier(text: Option<&str>) -> Result<Option<f32>, String> {
    let Some(t) = text.map(str::trim).filter(|t| !t.is_empty()) else { return Ok(None) };
    match t.parse::<f32>() {
        Ok(v) if v.is_finite() => Ok(Some(v.clamp(0.0, DEBUG_MOST))),
        _ => Err(format!("{DEBUG_VAR}={t}: expected a number (the aerial perspective's weight multiplier)")),
    }
}

// ---- The shader edits ----

/// The embedded asset path under which bevy_pbr registers `bevy_pbr::pbr_functions`.
const PBR_FUNCTIONS: &str = "embedded://bevy_pbr/render/pbr_functions.wesl";
/// Bevy's fog function's head [documented: engine/vendor/bevy_pbr/src/render/pbr_functions.wesl]; the haze's
/// function goes in front of it too, which leaves it there once.
const BEVY_APPLY_FOG: &str = "@if(DISTANCE_FOG)\nfn apply_fog(";
/// The haze's fog call (haze.rs `HAZE_FOG_CALL`) wrapped by `nvf_aerial`.
const AERIAL_FOG_CALL: &str = "        return nvf_aerial(fog_params, input_color, view_to_world, distance, nvf_haze(fog_params, view_to_world, view_world_position, distance, scattering));";

/// `nvf_aerial`, put before `apply_fog`. The `NVF_AERIAL_*` words are the Rust constants above,
/// written in by [`aerial_function`].
const AERIAL_FUNCTION: &str = "// NVForever (nvforever aerial.rs AERIAL_FUNCTION): aerial perspective, an enhanced-look switch.
// After Hoffman and Preetham, \"Rendering Outdoor Light Scattering in Real Time\" (GDC 2002): the
// in-scattered colour follows the view ray's direction (the sky toward it, a Henyey-Greenstein
// lobe toward the sun), the extinction differs per channel. The fog uniform's nvf_params hold
// (aerial.rs pack): [0] the sun colour (display values) and the lobe's strength (negative while
// the directional light points away from the sun: below the horizon it is the moon's side), [1]
// the colour toward the horizon and the red weight, [2] the colour toward the sky-lower height
// and the blue weight. All zero: off, Bevy's call as it was. On, the game's fog (fog.rs GAME_FOG)
// gets a copy of the uniform with [0] the colour resolved for this ray (alpha -2) and [1] the
// three weights.
@if(DISTANCE_FOG)
fn nvf_aerial(
    fog_params: mesh_view_types::Fog,
    input_color: vec4<f32>,
    view_to_world: vec3<f32>,
    distance: f32,
    scattering: vec3<f32>,
) -> vec4<f32> {
    let red_weight = fog_params.nvf_params[1].w;
    if red_weight <= 0.0 {
        return package::render::fog::atmospheric_fog(fog_params, input_color, distance, scattering);
    }
    let blue_weight = fog_params.nvf_params[2].w;
    let green_weight = max((1.0 - NVF_AERIAL_LUMA_R * red_weight - NVF_AERIAL_LUMA_B * blue_weight) / NVF_AERIAL_LUMA_G, 0.0);
    let ray = view_to_world / max(distance, 1e-6);
    var air = mix(fog_params.nvf_params[1].rgb, fog_params.nvf_params[2].rgb, smoothstep(0.0, NVF_AERIAL_LOWER_SIN, ray.y));
    let lobe_strength = fog_params.nvf_params[0].w;
    if lobe_strength != 0.0 && view_bindings::lights.n_directional_lights > 0u {
        let g: f32 = NVF_AERIAL_HG_G;
        let toward = sign(lobe_strength) * dot(ray, view_bindings::lights.directional_lights[0].direction_to_light);
        let q = (1.0 - g) * (1.0 - g) / max(1.0 + g * g - 2.0 * g * toward, 1e-6);
        air = mix(air, fog_params.nvf_params[0].rgb, clamp(abs(lobe_strength) * q * sqrt(q), 0.0, 1.0));
    }
    var resolved = fog_params;
    resolved.nvf_params[0] = vec4<f32>(air, -2.0);
    resolved.nvf_params[1] = vec4<f32>(red_weight, green_weight, blue_weight, 0.0);
    return package::render::fog::atmospheric_fog(resolved, input_color, distance, scattering);
}

";

/// A float as WGSL text (always with a decimal point).
fn wgsl_float(v: f32) -> String {
    format!("{v:.12}")
}

/// [`AERIAL_FUNCTION`] with the constants written in.
fn aerial_function() -> String {
    AERIAL_FUNCTION
        .replace("NVF_AERIAL_LUMA_R", &wgsl_float(LUMA[0]))
        .replace("NVF_AERIAL_LUMA_G", &wgsl_float(LUMA[1]))
        .replace("NVF_AERIAL_LUMA_B", &wgsl_float(LUMA[2]))
        .replace("NVF_AERIAL_LOWER_SIN", &wgsl_float(LOWER_SIN))
        .replace("NVF_AERIAL_HG_G", &wgsl_float(HG_G))
}

/// The edits to the haze-patched `pbr_functions.wesl`, each to text there exactly once: the
/// function in front of `apply_fog`, the haze's fog call wrapped.
fn aerial_edits() -> [(&'static str, String); 2] {
    [(BEVY_APPLY_FOG, format!("{}{BEVY_APPLY_FOG}", aerial_function())), (crate::haze::HAZE_FOG_CALL, AERIAL_FOG_CALL.to_string())]
}

/// `source` (with the haze's edits) with [`aerial_edits`] made; `None` unless each anchor is
/// there exactly once (Bevy or the haze changed, or the haze's edits are not in it).
fn aerial_patched(source: &str) -> Option<String> {
    let edits = aerial_edits();
    if edits.iter().any(|(from, _)| source.matches(from).count() != 1) {
        return None;
    }
    let mut out = source.to_string();
    for (from, to) in &edits {
        out = out.replacen(from, to, 1);
    }
    Some(out)
}

/// The edits' state: not yet tried, in place, or not made.
static PATCH: AtomicU8 = AtomicU8::new(PATCH_PENDING);
const PATCH_PENDING: u8 = 0;
const PATCH_IN_PLACE: u8 = 1;
const PATCH_FAILED: u8 = 2;

#[derive(Resource)]
struct AerialShader {
    handle: Handle<Shader>,
    patched: bool,
}

/// Once the haze's edits of `pbr_functions` are in place (haze.rs `patch_haze`, run just
/// before), wrap its fog call (always: with the switch off the uniform's vectors are zero and
/// the call gives what it gave). Fails safe: text not as expected leaves the shader as the
/// haze left it and the aerial perspective off. No log line on success (the switch off keeps
/// the log as it was).
fn patch_aerial(mut state: ResMut<AerialShader>, mut shaders: ResMut<Assets<Shader>>) {
    if state.patched {
        return;
    }
    if crate::haze::edits_failed() {
        state.patched = true;
        PATCH.store(PATCH_FAILED, Ordering::Relaxed);
        return;
    }
    if !crate::haze::edits_in_place() {
        return;
    }
    let Some(shader) = shaders.get(&state.handle) else { return };
    state.patched = true;
    let bevy::shader::Source::Wesl(source) = &shader.source else {
        warn!("{PBR_FUNCTIONS}: not WESL; no aerial perspective");
        PATCH.store(PATCH_FAILED, Ordering::Relaxed);
        return;
    };
    let source = crate::lights::unix_lines(source);
    let Some(patched) = aerial_patched(&source) else {
        warn!("{PBR_FUNCTIONS}: the desert haze's fog call not as expected; no aerial perspective");
        PATCH.store(PATCH_FAILED, Ordering::Relaxed);
        return;
    };
    let patched = crate::fog::with_source(shader, patched);
    let _ = shaders.insert(&state.handle, patched);
    PATCH.store(PATCH_IN_PLACE, Ordering::Relaxed);
}

// ---- Following the sky ----

pub struct AerialPlugin;

impl Plugin for AerialPlugin {
    fn build(&self, app: &mut App) {
        let handle: Handle<Shader> = app.world().resource::<AssetServer>().load(PBR_FUNCTIONS);
        app.insert_resource(AerialShader { handle, patched: false });
        add_systems(app);
    }
}

/// The plugin's systems (apart from its shader handle, for the scheduling test). The 3D
/// scope's lens camera copies the main camera's fog after [`AerialSet`] (scope_view.rs
/// `ScopeViewPlugin`), and the reflection camera in PostUpdate (reflection.rs), so neither
/// shows a frame with the sun's side not yet turned at a horizon crossing.
fn add_systems(app: &mut App) {
    app.add_systems(PreUpdate, patch_aerial.after(crate::haze::patch_haze)).add_systems(Update, update_aerial.after(crate::sky::update_sky).in_set(AerialSet));
}

/// Where the aerial terms are written into the main camera's fog (for the cameras that copy it).
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct AerialSet;

/// What the log last said.
struct Logged {
    weather: Option<String>,
    weight: f32,
    weights: [f32; 3],
}

/// The state between frames (nothing weather-made is kept: all of it is worked out again
/// each frame).
#[derive(Default)]
struct AerialState {
    /// On outdoors (for the on and off lines).
    active: bool,
    /// The debug multiplier, read once.
    debug: Option<f32>,
    logged: Option<Logged>,
    said_not_installed: bool,
    /// The last derivation's inputs (the weight times the debug multiplier, the sky) and its
    /// result, used again while they stay the same.
    derived: Option<(f32, SkyColours, Option<Aerial>)>,
}

/// The figures for the log.
fn describe(weather: Option<&str>, weight: f32, multiplier: f32, a: Option<&Aerial>) -> String {
    let name = weather.unwrap_or("the weather");
    let debug = if multiplier != 1.0 { format!(", debug x{multiplier}") } else { String::new() };
    match a {
        Some(a) => format!(
            "{name}: weight {weight:.2}{debug}, channel weights {:.2}/{:.2}/{:.2}, sky share {:.2}, sun lobe {:.2}",
            a.weights[0], a.weights[1], a.weights[2], a.sky_share, a.lobe
        ),
        None => format!("{name}: weight {weight:.2}{debug}, none (the weather's own fog leaves no room)"),
    }
}

/// Every frame after the sky: with the switch on, outdoors, write the aerial terms into the
/// camera's fog from the sky as it is now; otherwise the vectors stay zero.
fn update_aerial(
    graphics: Option<Res<crate::graphics::Graphics>>,
    sky: Option<Res<SkyState>>,
    mut cams: Query<&mut DistanceFog, With<FlyCam>>,
    mut state: Local<AerialState>,
) {
    let wanted = graphics.as_deref().is_some_and(|g| g.aerial_perspective);
    let Ok(mut fog) = cams.single_mut() else { return };
    let installed = crate::fog::installed() && PATCH.load(Ordering::Relaxed) == PATCH_IN_PLACE;
    if wanted && PATCH.load(Ordering::Relaxed) == PATCH_FAILED && !state.said_not_installed {
        state.said_not_installed = true;
        warn!("aerial: the aerial perspective's shader edit is not in place; no aerial perspective");
    }
    let indoors = sky.as_deref().is_none_or(|s| s.interior.is_some());
    let Some(sky) = sky.filter(|_| wanted && installed && !indoors) else {
        if fog.nvf_params != [Vec4::ZERO; 3] {
            fog.nvf_params = [Vec4::ZERO; 3];
        }
        if state.active {
            state.active = false;
            info!("aerial: aerial perspective off{}", if wanted && indoors { " indoors" } else { "" });
        }
        state.logged = None;
        state.derived = None;
        return;
    };
    let multiplier = *state.debug.get_or_insert_with(|| match debug_multiplier(std::env::var(DEBUG_VAR).ok().as_deref()) {
        Ok(Some(m)) => {
            info!("debug switch {DEBUG_VAR}: the aerial perspective's weight x{m}");
            m
        }
        Ok(None) => 1.0,
        Err(e) => {
            warn!("{e}; the aerial perspective as designed");
            1.0
        }
    });

    // The weather's fog as the sky gave it to the fog uniform this frame (as haze.rs reads it).
    let range = sky.fog_override.map(|(n, f)| (n, f, 1.0)).or_else(|| sky.fog());
    let (near, far, strength) = range.unwrap_or((0.0, 0.0, 0.0));
    let weight = crate::haze::weight(near, far, sky.fog_power(), strength);
    let colours = SkyColours {
        fog: sky.color(ColorType::Fog),
        horizon: sky.color(ColorType::Horizon),
        lower: sky.color(ColorType::SkyLower),
        sun: sky.color(ColorType::Sun),
        sky_scale: sky.sky_scale(),
        sun_elevation: sky.sun_elevation(),
        light_is_sun: sky.sun_elevation_at(sky.sun_light_hour()) >= 0.0,
    };
    let strength = weight * multiplier;
    let now = match state.derived {
        Some((s, c, a)) if s == strength && c == colours => a,
        _ => {
            let a = aerial(strength, &colours);
            state.derived = Some((strength, colours, a));
            a
        }
    };
    let packed = pack(now.as_ref());
    if fog.nvf_params != packed {
        fog.nvf_params = packed;
    }

    // The log: on, and when the figures move notably (not during a time-lapse).
    let name = sky.weather.edid.as_deref();
    let weights = now.map_or([1.0; 3], |a| a.weights);
    if !state.active {
        state.active = true;
        info!("aerial: aerial perspective on: {}", describe(name, weight, multiplier, now.as_ref()));
        state.logged = Some(Logged { weather: name.map(str::to_string), weight, weights });
    } else if sky.time_scale <= 0.0 {
        let notable = state.logged.as_ref().is_none_or(|l| {
            l.weather.as_deref() != name || (l.weight - weight).abs() >= 0.1 || (l.weight > 0.0) != (weight > 0.0) || l.weights.iter().zip(weights).any(|(a, b)| (a - b).abs() >= 0.1)
        });
        if notable {
            info!("aerial: {}", describe(name, weight, multiplier, now.as_ref()));
            state.logged = Some(Logged { weather: name.map(str::to_string), weight, weights });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A clear noon's colours (linear), in the Mojave's manner: a pale blue horizon, a bluer
    /// sky-lower, a warm white sun.
    fn clear_noon() -> SkyColours {
        let l = fnv_world::sky::srgb_to_linear;
        let c = |r: u8, g: u8, b: u8| [l(r), l(g), l(b)];
        SkyColours { fog: c(150, 168, 190), horizon: c(162, 194, 204), lower: c(123, 156, 242), sun: c(255, 240, 200), sky_scale: 0.88, sun_elevation: 70.0, light_is_sun: true }
    }

    fn dusk() -> SkyColours {
        let l = fnv_world::sky::srgb_to_linear;
        let c = |r: u8, g: u8, b: u8| [l(r), l(g), l(b)];
        SkyColours { fog: c(170, 140, 120), horizon: c(230, 150, 100), lower: c(120, 110, 150), sun: c(255, 180, 90), sky_scale: 0.88, sun_elevation: 2.0, light_is_sun: true }
    }

    fn mean(w: [f32; 3]) -> f32 {
        luminance(w)
    }

    #[test]
    fn the_constants_say_what_their_notes_say() {
        assert!((LOWER_SIN - 20f32.to_radians().sin()).abs() < 1e-6);
        assert!((LUMA.iter().sum::<f32>() - 1.0).abs() < 1e-6);
        assert!(WEIGHT_LOW > 0.0 && WEIGHT_LOW < 1.0 && WEIGHT_HIGH > 1.0);
        assert!(DOME_MIX <= 1.0 && LOBE_MOST <= 1.0 && HG_G > 0.0 && HG_G < 1.0);
    }

    #[test]
    fn a_grey_or_black_sky_gives_even_weights() {
        assert_eq!(chroma_weights([0.4, 0.4, 0.4], 1.0), [1.0; 3]);
        assert_eq!(chroma_weights([0.0; 3], 1.0), [1.0; 3]);
        assert_eq!(chroma_weights([1e-9, 0.0, 0.0], 1.0), [1.0; 3], "as good as black");
        assert_eq!(chroma_weights([0.3, 0.5, 0.9], 0.0), [1.0; 3], "no gain");
        assert_eq!(chroma_weights([f32::NAN, 0.5, 0.9], f32::NAN), [1.0; 3]);
        let w = chroma_weights([0.4, 0.4, 0.4], 3.0);
        assert!(w.iter().all(|v| (v - 1.0).abs() < 1e-5), "{w:?}");
    }

    #[test]
    fn the_weights_keep_a_mean_of_one_and_their_range() {
        let skies = [
            [0.36, 0.54, 0.60],
            [0.79, 0.30, 0.13],
            [0.0, 0.5, 0.9],
            [1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.0, 1.0, 0.0],
            [5.0, 0.01, 0.0],
            [1e30, 1.0, 0.0],
            [-1.0, 0.5, f32::INFINITY],
        ];
        for sky in skies {
            for gain in [0.1, 1.0, 4.0, 100.0] {
                let w = chroma_weights(sky, gain);
                assert!(w.iter().all(|v| v.is_finite() && (WEIGHT_LOW - 1e-5..=WEIGHT_HIGH + 1e-5).contains(v)), "{sky:?} x{gain}: {w:?}");
                assert!((mean(w) - 1.0).abs() < 1e-4, "{sky:?} x{gain}: mean {}", mean(w));
            }
        }
        // One channel at 0: that channel weighs the least, the mean still 1.
        let w = chroma_weights([0.0, 0.5, 0.6], 1.0);
        assert!(w[0] < w[1] && w[0] < w[2], "{w:?}");
        // A blue sky fogs blue in first, a dusk sky red.
        let noon = chroma_weights(clear_noon().horizon, 1.0);
        assert!(noon[2] > noon[1] && noon[1] > noon[0], "{noon:?}");
        let dusk = chroma_weights(dusk().horizon, 1.0);
        assert!(dusk[0] > dusk[1] && dusk[1] > dusk[2], "{dusk:?}");
        // More gain moves them further, never past the range.
        let half = chroma_weights(clear_noon().horizon, 0.5);
        assert!((half[2] - 1.0) < (noon[2] - 1.0) && half[2] > 1.0);
    }

    #[test]
    fn the_shares_are_the_weathers_at_none_and_full_and_rise_with_it() {
        for w in [[1.0; 3], [0.5, 1.2, 2.0], [WEIGHT_LOW, 1.0, WEIGHT_HIGH]] {
            assert_eq!(channel_shares(0.0, w), [0.0; 3]);
            assert_eq!(channel_shares(1.0, w), [1.0; 3]);
            let mut last = [0.0f32; 3];
            for k in 0..=100 {
                let s = channel_shares(k as f32 / 100.0, w);
                assert!(s.iter().zip(last).all(|(a, b)| *a >= b - 1e-6 && (0.0..=1.0).contains(a)), "{k}: {s:?} after {last:?}");
                last = s;
            }
        }
        // Even weights: the weather's own share.
        let s = channel_shares(0.38, [1.0; 3]);
        assert!(s.iter().all(|v| (v - 0.38).abs() < 1e-6));
        // A heavier channel fogs in first.
        let s = channel_shares(0.38, [0.7, 1.0, 1.3]);
        assert!(s[0] < 0.38 && s[2] > 0.38);
        // Outside 0..1 or not finite: held.
        assert_eq!(channel_shares(2.0, [1.0; 3]), [1.0; 3]);
        assert_eq!(channel_shares(-1.0, [1.0; 3]), [0.0; 3]);
        assert_eq!(channel_shares(f32::NAN, [1.0; 3]), [0.0; 3]);
    }

    #[test]
    fn the_weight_comes_from_the_weathers_fog_as_the_hazes_does() {
        // Far 0 (the dust storm's day fog), a mod's 1,000,000, power 0 (read as linear).
        assert_eq!(crate::haze::weight(0.0, 0.0, 1.0, 1.0), 0.0);
        assert_eq!(crate::haze::weight(0.0, 1e6, 0.6, 1.0), 1.0);
        assert_eq!(crate::haze::weight(0.0, 200_000.0, 0.0, 1.0), crate::haze::weight(0.0, 200_000.0, 1.0, 1.0));
        // A hazy weather and the dust storm's linear fog: none, so off exactly.
        assert_eq!(crate::haze::weight(1_800.0, 30_000.0, 0.6, 1.0), 0.0);
        assert_eq!(crate::haze::weight(510.0, 55_000.0, 1.0, 1.0), 0.0);
        assert_eq!(aerial(0.0, &clear_noon()), None);
        assert_eq!(pack(aerial(0.0, &clear_noon()).as_ref()), [Vec4::ZERO; 3]);
    }

    #[test]
    fn the_lobe_peaks_at_the_sun_and_stays_within_one() {
        assert!((lobe(1.0) - 1.0).abs() < 1e-6);
        let mut last = 0.0;
        for k in -100..=100 {
            let v = lobe(k as f32 / 100.0);
            assert!(v >= last - 1e-7 && v <= 1.0 + 1e-6 && v >= 0.0, "{k}: {v}");
            last = v;
        }
        assert!(lobe(0.0) < 0.1, "little to the side");
        assert_eq!(lobe(5.0), lobe(1.0));
        assert_eq!(lobe(f32::NAN), lobe(-1.0));
    }

    /// The in-scatter colour for every ray stays within the weather's own colours (display
    /// values; the sky colours times the sky level, as the dome draws them), per channel.
    #[test]
    fn the_in_scatter_colour_stays_within_the_weathers_colours() {
        for sky in [clear_noon(), dusk(), SkyColours { sky_scale: 1.55, ..clear_noon() }, SkyColours { sun_elevation: -30.0, light_is_sun: false, ..dusk() }] {
            for strength in [0.1, 0.5, 1.0, 4.0] {
                let a = aerial(strength, &sky).expect("on");
                let p = pack(Some(&a));
                let scale = sky.sky_scale.max(0.0);
                for y in [-1.0, -0.1, 0.0, 0.05, 0.2, 0.5, 1.0] {
                    for c in [-1.0, 0.0, 0.5, 0.9, 1.0] {
                        let (air, w) = resolve(&p, y, c).expect("on");
                        for i in 0..3 {
                            let pts = [display(sky.fog[i]), display(sky.horizon[i] * scale), display(sky.lower[i] * scale), display(sky.sun[i])];
                            let (lo, hi) = pts.iter().fold((f32::MAX, f32::MIN), |(a, b), v| (a.min(*v), b.max(*v)));
                            assert!(air[i] >= lo - 1e-5 && air[i] <= hi + 1e-5, "{strength} {y} {c} ch {i}: {} not in {lo}..{hi}", air[i]);
                            if hi <= 1.0 {
                                assert!(air[i] <= 1.0 + 1e-6, "within the display range when the weather's colours are");
                            }
                        }
                        // The shader's green is the CPU's.
                        assert!((w[1] - a.weights[1]).abs() < 1e-4, "{w:?} vs {:?}", a.weights);
                    }
                }
            }
        }
        // No sun lobe at night; the sky colour by the ray's height.
        let night = aerial(1.0, &SkyColours { sun_elevation: -10.0, ..clear_noon() }).unwrap();
        assert_eq!(night.lobe, 0.0);
        let p = pack(Some(&night));
        assert_eq!(resolve(&p, 0.0, 1.0).unwrap().0, night.horizon);
        let up = resolve(&p, 1.0, 1.0).unwrap().0;
        assert!(up.iter().zip(night.lower).all(|(a, b)| (a - b).abs() < 1e-6), "{up:?}");
        // Off: nothing resolved.
        assert_eq!(resolve(&[Vec4::ZERO; 3], 0.0, 1.0), None);
    }

    /// The sun's direction as sky.rs works it out for an elevation (degrees) at a fixed
    /// heading, Bevy axes (y up), and the directional light's (aim_sun: the opposite side
    /// below the horizon).
    fn sun_and_light(elevation: f32) -> (Vec3, Vec3) {
        let e = elevation.to_radians();
        let sun = Vec3::new(e.cos(), e.sin(), 0.0);
        (sun, if sun.y >= 0.0 { sun } else { -sun })
    }

    /// Across the horizon the lobe follows the real sun, not the light that turns to the
    /// moon's side: the in-scatter colour is continuous through 0 degrees.
    #[test]
    fn the_sun_lobe_is_continuous_across_the_horizon() {
        let at = |elevation: f32, ray: Vec3| {
            let (_, light) = sun_and_light(elevation);
            let sky = SkyColours { sun_elevation: elevation, light_is_sun: elevation >= 0.0, ..dusk() };
            let p = pack(aerial(1.0, &sky).as_ref());
            resolve(&p, ray.y, ray.dot(light)).unwrap().0
        };
        let toward = Vec3::new(1.0, 0.0, 0.0);
        let away = Vec3::new(-1.0, 0.0, 0.0);
        for ray in [toward, away, Vec3::new(0.7, 0.1, 0.7).normalize()] {
            let (above, below) = (at(0.01, ray), at(-0.01, ray));
            assert!(above.iter().zip(below).all(|(a, b)| (a - b).abs() < 2e-3), "{ray:?}: {above:?} vs {below:?}");
        }
        // Below the horizon, still brighter toward the sun than away from it.
        let sky = SkyColours { sun_elevation: -1.0, light_is_sun: false, ..dusk() };
        let p = pack(aerial(1.0, &sky).as_ref());
        assert!(p[0].w < 0.0, "the light points away from the sun");
        let (_, light) = sun_and_light(-1.0);
        let sun_side = resolve(&p, 0.0, toward.dot(light)).unwrap().0;
        let moon_side = resolve(&p, 0.0, away.dot(light)).unwrap().0;
        let a = aerial(1.0, &sky).unwrap();
        assert!(sun_side.iter().zip(a.sun).zip(a.horizon).all(|((v, s), h)| (v - s).abs() <= (h - s).abs() + 1e-6), "toward the sun the colour is nearer the sun's: {sun_side:?}");
        assert!(moon_side.iter().zip(a.horizon).all(|(v, h)| (v - h).abs() < 1e-3), "away from it hardly any: {moon_side:?}");
    }

    /// Under a brighter image space the far land's target is the dome as drawn (its colour
    /// times the actual sky level), not held to the weather's colour.
    #[test]
    fn the_sky_colours_follow_the_actual_sky_level() {
        let dim = SkyColours { horizon: [0.2, 0.25, 0.3], lower: [0.15, 0.2, 0.35], sky_scale: 1.55, ..clear_noon() };
        let a = aerial(1.0 / DOME_MIX, &dim).expect("on");
        assert_eq!(a.sky_share, 1.0);
        for i in 0..3 {
            assert!((a.horizon[i] - display(dim.horizon[i] * 1.55)).abs() < 1e-6, "{:?}", a.horizon);
            assert!((a.lower[i] - display(dim.lower[i] * 1.55)).abs() < 1e-6, "{:?}", a.lower);
            assert!(a.horizon[i] < 1.0 && a.lower[i] < 1.0, "colours under 1 stay under 1");
        }
        // Not finite: as drawn at 1; below 0: black, never negative.
        let one = aerial(1.0, &SkyColours { sky_scale: f32::NAN, ..dim }).unwrap();
        let at_one = aerial(1.0, &SkyColours { sky_scale: 1.0, ..dim }).unwrap();
        assert_eq!(one.horizon, at_one.horizon);
        let none = aerial(1.0, &SkyColours { sky_scale: -2.0, ..dim }).unwrap();
        assert!(none.horizon.iter().all(|v| *v >= 0.0));
    }

    #[test]
    fn extreme_skies_give_finite_figures() {
        let odd = [
            SkyColours { fog: [0.0; 3], horizon: [0.0; 3], lower: [0.0; 3], sun: [0.0; 3], sky_scale: 0.0, sun_elevation: 0.0, light_is_sun: true },
            SkyColours { fog: [f32::NAN; 3], horizon: [f32::INFINITY, 0.0, -1.0], lower: [1e9; 3], sun: [-5.0; 3], sky_scale: f32::NAN, sun_elevation: f32::NAN, light_is_sun: false },
            SkyColours { fog: [1.0; 3], horizon: [1.0, 0.0, 0.0], lower: [0.0, 0.0, 1.0], sun: [1.0; 3], sky_scale: 1e9, sun_elevation: 90.0, light_is_sun: true },
            SkyColours { fog: [0.5; 3], horizon: [0.5; 3], lower: [0.5; 3], sun: [0.5; 3], sky_scale: -3.0, sun_elevation: 1.0, light_is_sun: false },
        ];
        for sky in odd {
            for strength in [1e-6, 1.0, 4.0, 1e9, f32::INFINITY, f32::NAN, -1.0] {
                let Some(a) = aerial(strength, &sky) else {
                    assert!(!(strength.is_finite() && strength > 0.0), "{strength}");
                    continue;
                };
                let p = pack(Some(&a));
                assert!(p.iter().all(|v| v.is_finite()) && p[0].truncate().min_element() >= 0.0 && p[1].min_element() >= 0.0 && p[2].min_element() >= 0.0, "{p:?}");
                assert!((-1.0..=1.0).contains(&p[0].w), "the lobe's strength and the sun's side: {p:?}");
                assert!(p[1].w > 0.0, "on is marked by a red weight above 0");
                let (air, w) = resolve(&p, 0.1, 0.9).unwrap();
                assert!(air.iter().chain(w.iter()).all(|v| v.is_finite() && *v >= 0.0));
                for f in [0.0, 0.3, 1.0] {
                    let (c, s) = blend([0.2; 3], air, w, f, Some(([0.5; 3], 0.4)));
                    assert!(c.iter().chain(s.iter()).all(|v| v.is_finite() && *v >= 0.0), "{c:?} {s:?}");
                }
            }
        }
    }

    /// Decision 3 with the plan check's rule: either alone and both together, every channel's
    /// total share is within 0..1, the haze never lowers it, and it is at least the haze's.
    #[test]
    fn aerial_and_haze_together_stay_within_one_blend() {
        let a = aerial(1.0, &clear_noon()).unwrap();
        let p = pack(Some(&a));
        let (air, w) = resolve(&p, 0.02, 0.3).unwrap();
        for k in 0..=20 {
            let f = k as f32 / 20.0;
            for h in [0.0, 0.05, 0.3, 0.75, 1.0] {
                let haze = (h > 0.0).then_some(([0.6, 0.55, 0.5], h));
                let (_, total) = blend([0.2, 0.15, 0.1], air, w, f, haze);
                let alone = channel_shares(f, w);
                for i in 0..3 {
                    assert!((0.0..=1.0 + 1e-6).contains(&total[i]), "{f} {h}: {total:?}");
                    assert!(total[i] >= alone[i] - 1e-6, "the haze lowered a share: {f} {h} {total:?} vs {alone:?}");
                    assert!(total[i] >= h - 1e-6, "{f} {h}: {total:?}");
                }
            }
        }
        // With the haze in the in-scatter colour: one mix with the combined share per channel.
        let (c, total) = blend([0.2; 3], air, w, 0.4, Some((air, 0.3)));
        for i in 0..3 {
            let want = 0.2 + (air[i] - 0.2) * total[i];
            assert!((c[i] - want).abs() < 1e-5, "{c:?}");
        }
        // Even weights and the fog colour as the in-scatter colour: the old blend exactly.
        let fog = [0.5, 0.55, 0.6];
        for f in [0.0, 0.1, 0.5, 1.0] {
            let (c, _) = blend([0.2, 0.15, 0.1], fog, [1.0; 3], f, None);
            let old: [f32; 3] = std::array::from_fn(|i| [0.2, 0.15, 0.1][i] + (fog[i] - [0.2, 0.15, 0.1][i]) * f);
            assert!(c.iter().zip(old).all(|(a, b)| (a - b).abs() < 1e-6));
        }
    }

    /// The cameras that clone the main camera's fog do so after it got this frame's aerial
    /// terms: at a horizon crossing the lens shows no frame with the lobe on the moon's side.
    /// (The scope's systems are added first, so the order comes from the constraint.)
    #[test]
    fn the_fog_copies_follow_the_aerial_terms() {
        let mut app = App::new();
        app.add_plugins(crate::scope_view::ScopeViewPlugin);
        add_systems(&mut app);
        app.world_mut().resource_scope(|world, mut schedules: Mut<bevy::ecs::schedule::Schedules>| {
            let update = schedules.get_mut(Update).expect("Update");
            update.initialize(world).expect("the Update schedule builds");
            let names: Vec<String> = update.systems().expect("initialized").map(|(_, s)| s.name().to_string()).collect();
            let at = |n: &str| names.iter().position(|s| s.ends_with(n)).unwrap_or_else(|| panic!("{n} not in {names:?}"));
            assert!(at("aerial::update_aerial") < at("scope_view::lens_camera"), "{names:?}");
        });
    }

    #[test]
    fn the_debug_multiplier_reads_a_number() {
        assert_eq!(debug_multiplier(None), Ok(None));
        assert_eq!(debug_multiplier(Some(" ")), Ok(None));
        assert_eq!(debug_multiplier(Some("2")), Ok(Some(2.0)));
        assert_eq!(debug_multiplier(Some("-1")), Ok(Some(0.0)));
        assert_eq!(debug_multiplier(Some("9")), Ok(Some(DEBUG_MOST)));
        assert!(debug_multiplier(Some("lots")).is_err());
        assert!(debug_multiplier(Some("inf")).is_err());
    }

    fn vendored(path: &str) -> String {
        let path = format!("{}/../vendor/bevy_pbr/src/render/{path}", env!("CARGO_MANIFEST_DIR"));
        crate::lights::unix_lines(&std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}")))
    }

    #[test]
    fn the_aerial_edits_apply_once_after_the_hazes() {
        let bevy = vendored("pbr_functions.wesl");
        assert!(aerial_patched(&bevy).is_none(), "without the haze's edits: none");
        let hazed = crate::haze::haze_patched(&bevy).expect("the haze's edits");
        for (from, _) in aerial_edits() {
            assert_eq!(hazed.matches(from).count(), 1, "{from}");
        }
        let patched = aerial_patched(&hazed).expect("both anchors");
        assert_eq!(patched.matches("fn nvf_aerial(").count(), 1);
        assert_eq!(patched.matches("fn nvf_haze(").count(), 1);
        assert_eq!(patched.matches(AERIAL_FOG_CALL).count(), 1);
        assert!(!patched.contains(crate::haze::HAZE_FOG_CALL));
        assert!(AERIAL_FOG_CALL.contains("nvf_haze(fog_params, view_to_world, view_world_position, distance, scattering)"), "the haze's call inside");
        assert!(!patched.contains("NVF_AERIAL_"), "every constant written in");
        let at = |s: &str| patched.find(s).unwrap();
        assert!(at("fn nvf_haze(") < at("fn nvf_aerial(") && at("fn nvf_aerial(") < at("fn apply_fog("));
        // Nothing else changed: taking the two edits back gives the haze's text.
        assert_eq!(patched.replacen(&aerial_function(), "", 1).replacen(AERIAL_FOG_CALL, crate::haze::HAZE_FOG_CALL, 1), hazed);
        // Twice: no second edit.
        assert!(aerial_patched(&patched).is_none());
        // Windows line ends still patch once made `\n`.
        assert!(aerial_patched(&crate::lights::unix_lines(&hazed.replace('\n', "\r\n"))).is_some());
    }

    #[test]
    fn the_game_fog_takes_the_aerial_branch_first_and_keeps_its_old_lines() {
        let fog = crate::fog::patched_vendored_fog();
        let at = |s: &str| fog.find(s).unwrap_or_else(|| panic!("{s}"));
        let aerial = at("if fog_params.nvf_params[0].w < -1.5 {");
        assert!(aerial > at("if fog_params.bi.x > 0.5 {"), "outdoors only");
        assert!(aerial < at("if scattering.x > 0.0 && fog_params.directional_light_color.a <= 0.0 {\n            // NVForever desert haze"));
        assert!(fog.contains("        let shown = mix(pow(max(input_color.rgb, vec3(0.0)), vec3(1.0 / 2.2)), pow(max(fog_color.rgb, vec3(0.0)), vec3(1.0 / 2.2)), fog_share);"));
        assert_eq!(fog.matches("nvf_params").count(), 3, "the marker, the weights, the colour");
    }

    /// The fog uniform grew by three vec4s at its end, in Rust and in the shader.
    #[test]
    fn the_fog_uniform_carries_three_more_vectors() {
        use bevy::render::render_resource::ShaderType;
        assert_eq!(<bevy::pbr::GpuFog as ShaderType>::min_size().get(), 64 + 3 * 16);
        assert_eq!(bevy::pbr::DistanceFog::default().nvf_params, [Vec4::ZERO; 3]);
        assert_eq!(crate::sky::initial_fog().nvf_params, [Vec4::ZERO; 3]);
        let types = vendored("mesh_view_types.wesl");
        let fog = &types[types.find("struct Fog {").unwrap()..];
        let fog = &fog[..fog.find("\n}\n").unwrap()];
        assert!(fog.find("    mode: u32,").unwrap() < fog.find("    nvf_params: array<vec4<f32>, 3>,").unwrap(), "{fog}");
    }

    /// The composed fog path (fog.rs GAME_FOG in the vendored fog.wesl, the haze's and this
    /// module's functions as written into the vendored pbr_functions.wesl, the edited call) is
    /// valid WGSL with the vendored `Fog` struct, and its layout is the Rust one. Only a run of
    /// the game proves the composed shader (the imports, the defs).
    #[test]
    fn the_patched_fog_path_is_valid_wgsl() {
        use wgpu::naga;
        let types = vendored("mesh_view_types.wesl");
        let fog_struct = {
            let s = &types[types.find("struct Fog {").unwrap()..];
            s[..s.find("\n}\n").unwrap() + 3].to_string()
        };
        let stand_ins = "struct DirectionalLight {
    color: vec4<f32>,
    direction_to_light: vec3<f32>,
    flags: u32,
}
struct Lights {
    directional_lights: array<DirectionalLight, 10>,
    n_directional_lights: u32,
}
@group(0) @binding(0) var<uniform> lights: Lights;
@group(0) @binding(1) var<uniform> fog_uniform: Fog;
";
        let fog = crate::fog::patched_vendored_fog();
        let start = fog.find("import package::render::{").expect("fog.wesl's imports");
        let end = start + fog[start..].find("};\n").expect("their end") + 3;
        let fog_functions = format!("{}{}", &fog[..start], &fog[end..]);
        let patched = aerial_patched(&crate::haze::haze_patched(&vendored("pbr_functions.wesl")).unwrap()).unwrap();
        let function = |name: &str| {
            let s = &patched[patched.find(&format!("fn {name}(")).unwrap()..];
            s[..s.find("\n}\n").unwrap() + 3].to_string()
        };
        let ours = format!("{}\n{}", function("nvf_haze"), function("nvf_aerial")).replace("mesh_view_types::Fog", "Fog").replace("view_bindings::lights", "lights").replace("package::render::fog::", "");
        let call = AERIAL_FOG_CALL.to_string();
        let source = format!(
            "{fog_struct}\n{stand_ins}\n{fog_functions}\n{ours}\nfn apply_fog_call(fog_params: Fog, input_color: vec4<f32>, view_to_world: vec3<f32>, view_world_position: vec3<f32>, distance: f32, scattering: vec3<f32>) -> vec4<f32> {{\n{call}\n}}\nfn use_uniform() -> vec4<f32> {{\n    return apply_fog_call(fog_uniform, vec4<f32>(0.5), vec3<f32>(1000.0, 10.0, 0.0), vec3<f32>(0.0), 1000.05, vec3<f32>(0.0));\n}}\n"
        );
        let module = naga::front::wgsl::parse_str(&source).unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::empty()).validate(&module).unwrap_or_else(|e| panic!("{e:?}"));
        let (_, fog_type) = module.types.iter().find(|(_, t)| t.name.as_deref() == Some("Fog")).expect("Fog");
        let naga::TypeInner::Struct { members, span } = &fog_type.inner else { panic!("Fog is a struct") };
        assert_eq!(*span, 112);
        let last = members.last().unwrap();
        assert_eq!((last.name.as_deref(), last.offset), (Some("nvf_params"), 64));
    }

    #[test]
    fn the_shader_reads_the_constants_as_written() {
        let f = aerial_function();
        assert!(f.contains(&format!("let g: f32 = {};", wgsl_float(HG_G))));
        assert!(f.contains(&format!("smoothstep(0.0, {}, ray.y)", wgsl_float(LOWER_SIN))));
        for v in [LUMA[0], LUMA[1], LUMA[2], LOWER_SIN, HG_G] {
            let back: f32 = wgsl_float(v).parse().unwrap();
            assert!((back - v).abs() <= v.abs() * 1e-6, "{v}");
        }
    }
}
