//! The world camera's reflection cube: what authored materials reflect (#2981 reflections;
//! docs/spec-notes/pbr-materials.md "Reflections on authored materials") [design].
//! NVForever's own rendering: the original draws no ambient reflection (docs/design/layer3.md),
//! and nothing here describes it. The cube pair, its upload and Bevy's component are
//! nvf_assets `reflection_cube.rs`; this module decides whether the world camera (`FlyCam`)
//! gets one and writes its content.
//!
//! - **Whether** ([`decide`]): the run's Reflections on authored materials row on, the
//!   Authored materials row on, the mount holding at least one authored set
//!   (nvf_assets `authored::mount_has_sets`) and the reflection gate installed in
//!   `pbr_functions` (nvforever lights.rs). Then one cube is made for the world camera and no
//!   other; slice 1's rule stays: Bevy's `EnvironmentMapLight` goes on only once the first
//!   fill is on the GPU (`reflection_cube::attach_lights`). Otherwise no cube is allocated.
//! - **What** ([`SkyContent`]), per world direction d and level, with e the sine of d's
//!   elevation (Bevy's +Y is up):
//!   - outdoors, above the horizon: the sky dome's colour at e, the weather's horizon,
//!     sky-lower and sky-upper colours (blended across a weather fade,
//!     `SkyState::color`) mixed by the dome's own weights for that height ([`DomeProfile`],
//!     read from `atmosphere.nif`'s vertices when the sky is made) the way the dome mixes
//!     them (sky.rs `dome_color`); below it a ground colour, the weather's ambient colour
//!     times [`GROUND_FACTOR`]; and a broad lobe toward the directional light in its colour
//!     and strength (the light as update_sky aims it, the moon's at night), no disc;
//!   - indoors: the cell's ambient colour above, [`GROUND_FACTOR`] of it below, and a band of
//!     the cell's fog colour at the horizon ([`FOG_BAND`]), as the water's indoor sky takes
//!     the fog colour (water.rs `update_water_sky`);
//!   - each level is the same function with a wider kernel, not a filter of the level above:
//!     each hemisphere's colour mixes toward that hemisphere's mean by level / 5, the edge
//!     between sky and ground widens ([`HORIZON_EDGE`] to [`HORIZON_EDGE_TOP`]), and the
//!     lobe widens with its total kept ([`SUN_LOBE_POWER`] halved per level).
//!
//!   No clouds, no sun disc, no scene objects, no stars or moon disc.
//! - **Units** ([`texel`]): the colours are taken to display values with the share the
//!   ambient light has (sky.rs `AMBIENT_SHARE` outdoors, `INTERIOR_AMBIENT_SHARE` indoors;
//!   the dome's linear colours through `lights::linear_to_display`, the interior's bytes / 255),
//!   then converted as the fork converts the ambient highlight, v x max(v, 1e-6)^1.2. With the
//!   cube's intensity `light_unit()` at the default exposure this makes a direction whose
//!   colour equals the ambient light reflect exactly the converted ambient
//!   (nvf_assets `ReflectionCube`, "Units"). The lobe is the directional light's display
//!   colour times its strength in display units (illuminance / (pi x light_unit), as sky.rs
//!   sets it), converted the same way. Sources add in linear light.
//! - **When**: checked at most every [`CHECK_INTERVAL`] seconds of real time and rewritten
//!   only when an input moved by more than a step from the inputs of the last fill
//!   ([`Inputs::moved`]: a colour by [`COLOUR_STEP`] in display value, the light's direction
//!   by [`SUN_STEP_DEGREES`], indoors or out, the interior cell): a weather fade or the
//!   clock rewrites it at most four times a second. One log line at the first fill, one
//!   summary at exit (`reflection sky summary: ...`).

use bevy::prelude::*;
use nvf_assets::reflection_cube::{self, GateState, ReflectionCube, SPECULAR_LEVELS};
use std::f32::consts::PI;

use crate::fly::FlyCam;
use crate::sky::{SkyState, Sun};
use fnv_world::sky::ColorType;

/// The ground below the horizon: the ambient colour times this, outdoors and indoors
/// [design: a darker floor than the sky, so a metal shows a horizon; not measured against
/// anything].
pub const GROUND_FACTOR: f32 = 0.5;
/// Indoors, the fog band's half-width in sine of elevation (about 11.5 degrees), a Gaussian
/// weight peaking at 1 on the horizon [design].
pub const FOG_BAND: f32 = 0.2;
/// The lobe toward the directional light: its peak at level 0 as a share of the light's
/// converted colour [design].
pub const SUN_LOBE_PEAK: f32 = 0.5;
/// The lobe's power at level 0 (cos^16 halves at about 16.5 degrees); halved at each level,
/// the peak scaled by (power + 1) / (SUN_LOBE_POWER + 1) so the lobe keeps its total [design].
pub const SUN_LOBE_POWER: f32 = 16.0;
/// The half-width (sine of elevation) of the edge between ground and sky at level 0, about a
/// texel of the 32-texel face [design].
pub const HORIZON_EDGE: f32 = 0.05;
/// The edge's half-width at the last level [design].
pub const HORIZON_EDGE_TOP: f32 = 0.5;
/// A colour input counts as changed past this step in display value [design].
pub const COLOUR_STEP: f32 = 1.0 / 128.0;
/// The directional light's direction counts as changed past this angle [design].
pub const SUN_STEP_DEGREES: f32 = 3.0;
/// Real seconds between two checks of the inputs [design: at most four fills a second].
pub const CHECK_INTERVAL: f64 = 0.25;
/// Samples of each hemisphere's profile over the sine of elevation 0..1.
const SAMPLES: usize = 33;

/// The units identity (module notes): a display value `v` (per channel, with its light's
/// share) as the cube's linear texel, `v x max(v, 1e-6)^1.2`, the fork's world conversion of
/// the ambient highlight (lights/display_shader.rs `nvf_ambient_ratio`). With the cube's
/// intensity `light_unit()` and the default exposure E = 1 / light_unit(), Bevy reflects
/// `texel x intensity x E` = this, so a direction whose display colour is the view's exposed
/// ambient A reflects A x max(A, 1e-6)^1.2, about A^2.2. Negative input is held at 0.
pub fn texel(display: Vec3) -> Vec3 {
    display.map(|a| {
        let a = a.max(0.0);
        a * a.max(1e-6).powf(1.2)
    })
}

/// The dome's weights of the horizon, sky-lower and sky-upper colours by height
/// ([`DOME_PROFILE_BINS`] equal steps of the sine of elevation from the horizon to the
/// zenith, equal solid angles): the mean of `atmosphere.nif`'s vertex colours in each step,
/// as sky.rs colours the dome from them (`dome_color`). Inserted when the sky is made.
#[derive(Resource, Clone, Debug, PartialEq)]
pub struct DomeProfile(pub [[f32; 3]; DOME_PROFILE_BINS]);

pub const DOME_PROFILE_BINS: usize = 16;

impl DomeProfile {
    /// From the dome's vertices (game space, z up, centred on the camera as drawn) and their
    /// colours (red horizon, green sky-lower, blue sky-upper). A step no vertex falls in takes
    /// the steps around it, linearly; with no vertex above the horizon, [`fallback`](Self::fallback).
    pub fn from_vertices(positions: &[[f32; 3]], colours: &[[f32; 4]]) -> DomeProfile {
        let mut sum = [[0.0f32; 3]; DOME_PROFILE_BINS];
        let mut count = [0u32; DOME_PROFILE_BINS];
        for (p, c) in positions.iter().zip(colours) {
            let len = Vec3::from_array(*p).length();
            if len <= 0.0 || !len.is_finite() {
                continue;
            }
            let e = p[2] / len;
            if e < 0.0 {
                continue;
            }
            let bin = ((e * DOME_PROFILE_BINS as f32) as usize).min(DOME_PROFILE_BINS - 1);
            for k in 0..3 {
                sum[bin][k] += c[k];
            }
            count[bin] += 1;
        }
        let filled: Vec<usize> = (0..DOME_PROFILE_BINS).filter(|&i| count[i] > 0).collect();
        if filled.is_empty() {
            return DomeProfile::fallback();
        }
        let mean = |i: usize| sum[i].map(|s| s / count[i] as f32);
        DomeProfile(std::array::from_fn(|i| {
            if count[i] > 0 {
                return mean(i);
            }
            let below = filled.iter().rev().find(|&&j| j < i).copied();
            let above = filled.iter().find(|&&j| j > i).copied();
            match (below, above) {
                (Some(a), Some(b)) => {
                    let t = (i - a) as f32 / (b - a) as f32;
                    let (wa, wb) = (mean(a), mean(b));
                    std::array::from_fn(|k| wa[k] + (wb[k] - wa[k]) * t)
                }
                (Some(a), None) => mean(a),
                (None, Some(b)) => mean(b),
                (None, None) => [0.0, 1.0, 0.0],
            }
        }))
    }

    /// The fallback sphere's rule (sky.rs `fallback_dome`): sky-lower to sky-upper over the
    /// lower half of the upper hemisphere, t = sqrt(e / 0.5).
    pub fn fallback() -> DomeProfile {
        DomeProfile(std::array::from_fn(|i| {
            let e = (i as f32 + 0.5) / DOME_PROFILE_BINS as f32;
            let t = (e / 0.5).clamp(0.0, 1.0).sqrt();
            [0.0, 1.0 - t, t]
        }))
    }

    /// The weights at sine of elevation `e` (held to 0..1), linear between the steps' centres.
    pub fn weights(&self, e: f32) -> [f32; 3] {
        let x = (e.clamp(0.0, 1.0) * DOME_PROFILE_BINS as f32 - 0.5).clamp(0.0, (DOME_PROFILE_BINS - 1) as f32);
        let i = (x.floor() as usize).min(DOME_PROFILE_BINS - 2);
        let t = x - i as f32;
        let (a, b) = (self.0[i], self.0[i + 1]);
        std::array::from_fn(|k| a[k] + (b[k] - a[k]) * t)
    }
}

/// What the cube is made from (module notes). Colours are display values without the
/// ambient's share (the content applies it), except the dome colours, linear as
/// `SkyState::color` gives them, and the light, already in display units with its strength.
#[derive(Clone, Debug, PartialEq)]
pub enum Inputs {
    Outdoors {
        horizon: Vec3,
        lower: Vec3,
        upper: Vec3,
        ambient: Vec3,
        /// The directional light's display colour times its strength.
        sun: Vec3,
        /// Toward the light (unit, Bevy world).
        sun_dir: Vec3,
    },
    Indoors {
        /// The interior cell (world.rs `Streamer::interior`).
        cell: Option<u32>,
        ambient: Vec3,
        fog: Vec3,
    },
}

impl Inputs {
    /// Whether these inputs differ from `last` (the last fill's) by more than a step
    /// (module notes, "When"): indoors against outdoors, another cell, a colour channel by
    /// more than [`COLOUR_STEP`] in display value, the light's direction by more than
    /// [`SUN_STEP_DEGREES`] while either light is above a step.
    pub fn moved(&self, last: &Inputs) -> bool {
        let colour = |a: Vec3, b: Vec3| (a - b).abs().max_element() > COLOUR_STEP;
        let dome = |v: Vec3| v.map(crate::lights::linear_to_display);
        match (self, last) {
            (
                Inputs::Outdoors { horizon, lower, upper, ambient, sun, sun_dir },
                Inputs::Outdoors { horizon: h0, lower: l0, upper: u0, ambient: a0, sun: s0, sun_dir: d0 },
            ) => {
                let lit = sun.max_element() > COLOUR_STEP || s0.max_element() > COLOUR_STEP;
                colour(dome(*horizon), dome(*h0))
                    || colour(dome(*lower), dome(*l0))
                    || colour(dome(*upper), dome(*u0))
                    || colour(*ambient, *a0)
                    || colour(*sun, *s0)
                    || (lit && sun_dir.dot(*d0) < SUN_STEP_DEGREES.to_radians().cos())
            }
            (Inputs::Indoors { cell, ambient, fog }, Inputs::Indoors { cell: c0, ambient: a0, fog: f0 }) => cell != c0 || colour(*ambient, *a0) || colour(*fog, *f0),
            _ => true,
        }
    }

    pub fn indoors(&self) -> bool {
        matches!(self, Inputs::Indoors { .. })
    }
}

/// The cube's content for one set of inputs (module notes, "What"): each hemisphere's
/// colour by sine of elevation as converted texels, their means, and the lobe.
#[derive(Clone, Debug)]
pub struct SkyContent {
    /// Texels at sine of elevation i / (SAMPLES - 1), above the horizon.
    up: [Vec3; SAMPLES],
    /// The same below the horizon (index by the sine of the depression).
    down: [Vec3; SAMPLES],
    up_mean: Vec3,
    down_mean: Vec3,
    /// The lobe's converted colour at its level-0 peak.
    sun: Vec3,
    sun_dir: Vec3,
}

/// The mean of a profile sampled evenly over the sine of elevation (equal solid angles):
/// the trapezoid rule.
fn hemisphere_mean(p: &[Vec3; SAMPLES]) -> Vec3 {
    let inner: Vec3 = p[1..SAMPLES - 1].iter().copied().sum();
    (inner + 0.5 * (p[0] + p[SAMPLES - 1])) / (SAMPLES - 1) as f32
}

fn sample(p: &[Vec3; SAMPLES], e: f32) -> Vec3 {
    let x = e.clamp(0.0, 1.0) * (SAMPLES - 1) as f32;
    let i = (x.floor() as usize).min(SAMPLES - 2);
    p[i].lerp(p[i + 1], x - i as f32)
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

impl SkyContent {
    pub fn new(inputs: &Inputs, profile: &DomeProfile) -> SkyContent {
        let at = |i: usize| i as f32 / (SAMPLES - 1) as f32;
        match inputs {
            Inputs::Outdoors { horizon, lower, upper, ambient, sun, sun_dir } => {
                let share = crate::sky::AMBIENT_SHARE;
                let up = std::array::from_fn(|i| {
                    let linear = crate::sky::dome_color(profile.weights(at(i)), lower.to_array(), horizon.to_array(), upper.to_array());
                    texel(Vec3::from_array(linear).map(crate::lights::linear_to_display) * share)
                });
                let ground = texel(*ambient * share * GROUND_FACTOR);
                let down = [ground; SAMPLES];
                SkyContent { up_mean: hemisphere_mean(&up), down_mean: ground, up, down, sun: texel(*sun) * SUN_LOBE_PEAK, sun_dir: sun_dir.normalize_or_zero() }
            }
            Inputs::Indoors { ambient, fog, .. } => {
                let share = crate::sky::INTERIOR_AMBIENT_SHARE;
                let (sky, ground, fog) = (texel(*ambient * share), texel(*ambient * share * GROUND_FACTOR), texel(*fog * share));
                let band = |e: f32| (-(e / FOG_BAND).powi(2)).exp();
                let up = std::array::from_fn(|i| sky.lerp(fog, band(at(i))));
                let down = std::array::from_fn(|i| ground.lerp(fog, band(at(i))));
                SkyContent { up_mean: hemisphere_mean(&up), down_mean: hemisphere_mean(&down), up, down, sun: Vec3::ZERO, sun_dir: Vec3::ZERO }
            }
        }
    }

    /// The linear texel toward world direction `d` (unit) at `level` (module notes).
    pub fn colour(&self, d: Vec3, level: u32) -> Vec3 {
        let b = (level as f32 / (SPECULAR_LEVELS - 1) as f32).min(1.0);
        let e = d.y.clamp(-1.0, 1.0);
        let up = sample(&self.up, e.max(0.0)).lerp(self.up_mean, b);
        let down = sample(&self.down, (-e).max(0.0)).lerp(self.down_mean, b);
        let w = HORIZON_EDGE + (HORIZON_EDGE_TOP - HORIZON_EDGE) * b;
        let sky = smoothstep(-w, w, e);
        let power = SUN_LOBE_POWER / (1u32 << level.min(16)) as f32;
        let lobe = self.sun * ((power + 1.0) / (SUN_LOBE_POWER + 1.0)) * d.dot(self.sun_dir).max(0.0).powf(power);
        down.lerp(up, sky) + lobe * sky
    }

    /// The hemispheres' means (above, below), which the last level approaches.
    pub fn means(&self) -> (Vec3, Vec3) {
        (self.up_mean, self.down_mean)
    }
}

/// Why the world camera has no cube, or that it may have one ([`decide`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    /// Not yet known: the reflection gate is still to be installed, or there is no world
    /// camera yet.
    Wait,
    Off(&'static str),
    Allocate,
}

/// The attach conditions (module notes, "Whether"): `row` the run's Reflections on authored
/// materials, `authored` its Authored materials, `has_sets` whether the mount has an authored
/// set (asked only when both rows are on), `gate` the reflection gate's state, `camera`
/// whether the world camera exists.
pub fn decide(row: bool, authored: bool, has_sets: impl FnOnce() -> bool, gate: GateState, camera: bool) -> Decision {
    if !row {
        return Decision::Off("setting");
    }
    if !authored {
        return Decision::Off("Authored materials off");
    }
    if !has_sets() {
        return Decision::Off("no authored sets in the mount");
    }
    match gate {
        GateState::Failed => Decision::Off("reflection gate not installed"),
        GateState::Pending => Decision::Wait,
        GateState::Installed if camera => Decision::Allocate,
        GateState::Installed => Decision::Wait,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
enum Phase {
    #[default]
    Waiting,
    Off(&'static str),
    On,
}

/// The producer's state: the run's two rows (main.rs), the phase, the last fill's inputs
/// and the counts for the summary.
#[derive(Resource, Default)]
pub struct ReflectionSky {
    row: bool,
    authored: bool,
    phase: Phase,
    last: Option<Inputs>,
    last_check: Option<f64>,
    fills_outdoors: u64,
    fills_indoors: u64,
    unchanged: u64,
}

impl ReflectionSky {
    pub fn new(row: bool, authored: bool) -> ReflectionSky {
        ReflectionSky { row, authored, ..default() }
    }

    /// The summary line (without its prefix).
    pub fn line(&self) -> String {
        match self.phase {
            Phase::Off(reason) => format!("off ({reason})"),
            Phase::Waiting => "no cube made (waiting for the reflection gate or the world camera)".to_string(),
            Phase::On => format!(
                "{} fills ({} outdoors, {} indoors), {} checks skipped as unchanged",
                self.fills_outdoors + self.fills_indoors,
                self.fills_outdoors,
                self.fills_indoors,
                self.unchanged
            ),
        }
    }
}

/// The inputs now (module notes, "What"), from the sky's state, the directional light and
/// the interior cell.
fn gather(sky: &SkyState, sun: Option<(&DirectionalLight, &Transform)>, cell: Option<u32>) -> Inputs {
    match sky.interior {
        Some(l) => {
            let shown = |v: [u8; 3]| Vec3::from_array(v.map(|b| b as f32 / 255.0));
            Inputs::Indoors { cell, ambient: shown(l.ambient), fog: shown(l.fog_color) }
        }
        None => {
            let c = |t: ColorType| Vec3::from_array(sky.color(t));
            let (sun, sun_dir) = match sun {
                Some((light, t)) => {
                    let colour = light.color.to_linear();
                    let strength = light.illuminance / (PI * crate::sky::light_unit());
                    (Vec3::new(colour.red, colour.green, colour.blue) * strength, *t.back())
                }
                None => (Vec3::ZERO, Vec3::Y),
            };
            Inputs::Outdoors {
                horizon: c(ColorType::Horizon),
                lower: c(ColorType::SkyLower),
                upper: c(ColorType::SkyUpper),
                ambient: c(ColorType::Ambient).map(crate::lights::linear_to_display),
                sun,
                sun_dir,
            }
        }
    }
}

/// Make the world camera's cube when [`decide`] allows, then rewrite it when its inputs
/// moved (module notes).
#[allow(clippy::too_many_arguments)]
pub fn produce(
    mut commands: Commands,
    mut state: ResMut<ReflectionSky>,
    data: Option<Res<nvf_assets::GameData>>,
    sky: Option<Res<SkyState>>,
    streamer: Option<Res<crate::world::Streamer>>,
    profile: Option<Res<DomeProfile>>,
    sun: Query<(&DirectionalLight, &Transform), With<Sun>>,
    mut cams: Query<(Entity, Option<&mut ReflectionCube>), With<FlyCam>>,
    mut images: ResMut<Assets<Image>>,
    time: Res<Time<Real>>,
) {
    if matches!(state.phase, Phase::Off(_)) {
        return;
    }
    let Some(sky) = sky else { return };
    let camera = cams.single().ok().map(|(e, _)| e);
    if state.phase == Phase::Waiting {
        let (row, authored) = (state.row, state.authored);
        match decide(row, authored, || data.as_deref().is_some_and(nvf_assets::authored::mount_has_sets), reflection_cube::gate_state(), camera.is_some()) {
            Decision::Wait => return,
            Decision::Off(reason) => {
                state.phase = Phase::Off(reason);
                info!("reflection sky: off ({reason}); no reflection cube is made");
                return;
            }
            Decision::Allocate => {}
        }
    }
    let now = time.elapsed_secs_f64();
    if state.phase == Phase::On && state.last_check.is_some_and(|t| now - t < CHECK_INTERVAL) {
        return;
    }
    state.last_check = Some(now);
    let inputs = gather(&sky, sun.single().ok(), streamer.and_then(|s| s.interior));
    if state.last.as_ref().is_some_and(|last| !inputs.moved(last)) {
        state.unchanged += 1;
        return;
    }
    let fallback;
    let profile = match profile.as_deref() {
        Some(p) => p,
        None => {
            fallback = DomeProfile::fallback();
            &fallback
        }
    };
    let content = SkyContent::new(&inputs, profile);
    if inputs.indoors() {
        state.fills_indoors += 1;
    } else {
        state.fills_outdoors += 1;
    }
    let where_ = if inputs.indoors() { "indoors" } else { "outdoors" };
    state.last = Some(inputs);
    if state.phase == Phase::Waiting {
        let Some(entity) = camera else { return };
        let mut cube = ReflectionCube::new(&mut images);
        cube.fill(|d, level| content.colour(d, level));
        commands.entity(entity).insert(cube);
        state.phase = Phase::On;
        let (up, down) = content.means();
        info!(
            "reflection sky: first fill of the world camera's cube ({where_}; level 0 zenith {:.4?}, horizon {:.4?}, nadir {:.4?}; mean above {:.4?}, below {:.4?}), attached once on the GPU",
            content.colour(Vec3::Y, 0).to_array(),
            content.colour(Vec3::X, 0).to_array(),
            content.colour(Vec3::NEG_Y, 0).to_array(),
            up.to_array(),
            down.to_array()
        );
        return;
    }
    if let Ok((_, Some(mut cube))) = cams.single_mut() {
        cube.fill(|d, level| content.colour(d, level));
    }
}

/// The exit summary (Last, on the app's exit): one line, `reflection sky summary: ...`.
pub fn report(mut exit: MessageReader<AppExit>, state: Res<ReflectionSky>) {
    if exit.read().last().is_none() {
        return;
    }
    info!("reflection sky summary: {}", state.line());
}

/// The producer and its summary; `row` and `authored` are the run's two rows, fixed at the
/// start (main.rs).
pub struct ReflectionSkyPlugin {
    pub row: bool,
    pub authored: bool,
}

impl Plugin for ReflectionSkyPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(ReflectionSky::new(self.row, self.authored))
            .add_systems(Update, produce.after(crate::sky::update_sky))
            .add_systems(Last, report);
    }
}

#[cfg(test)]
mod tests;
