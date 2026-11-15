//! [design] NVForever's mesh/landscape blend (#2661, docs/design/graphics-enhanced.md
//! §3.12): rigid exterior STAT and SCOL shapes, no noise, immutable source materials.
//!
//! On with the Graphics settings row `mesh-terrain-blend` ("Blend meshes into ground";
//! off by default, off in both looks, not part of `enhanced`), which draws
//! [`setting_config`]: an 8-unit band of the `height` shape, 1024 colour texels per cell,
//! full shading. The test aid `NVF_PLAY_TEST_MESH_BLEND=[band:]view[:shape[:resolution
//! [:shading[:flat,steep]]]]` overrides the row for comparison runs (it is not stored);
//! omitted parts take the same defaults, and hybrid v2's widths are `band / 16` and
//! `6 band`. These are NVForever's own choices.
//!
//! [design] Live switching ([`follow_setting`]): turning the blend on opens its material
//! gate ([`BlendGate`]), records the census and the landscape captures of what is already
//! drawn (world.rs::blend_backfill, terrain_surface.rs::backfill_cells) and lets the cache
//! bake; turning it off puts every shape's ordinary material back ([`stop`], in PostUpdate
//! after the material producers), drops the census, the captures, the atlas and the
//! enabled clones, and closes the gate once their materials are gone ([`close_gate`]).
//! While it is off no bake is scheduled and nothing is allocated; what still runs is
//! listed in docs/design/graphics-enhanced.md §3.12.
//!
//! [design] Census (round 6): every spawned STAT or SCOL shape carries a [`BlendCensus`]
//! with the reasons its reference and shape cannot blend, recorded before any filtering
//! (world.rs::spawn_model); the conversion adds the material and ownership reasons each
//! frame and reports counts on the ready line and bounded `census-shape` lines. The census
//! is separate from [`BlendShape`]: only shapes without reference or shape reasons get a
//! `BlendShape`, so tracking a shape never makes it convertible.

use crate::terrain_surface::{SurfaceCache, update_cache};
use bevy::light::NotShadowCaster;
use bevy::pbr::MaterialGate;
use bevy::prelude::*;
use nvf_assets::nif_material::{Falloff, MaterialOverride, NifMaterial, SeeThroughNoShadow};
use nvf_assets::nif_surface_blend::{ContactShape, ContactWidths, NifBlendMaterial, NifSurfaceBlend, Shading, DEFAULT_RESOLUTION, RESOLUTIONS};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Variant { Surface, Weight, Normal, Selection, Status }

impl Variant {
    fn name(self) -> &'static str {
        match self { Self::Surface => "surface", Self::Weight => "weight", Self::Normal => "normal", Self::Selection => "selection", Self::Status => "status" }
    }
    fn number(self) -> f32 {
        match self { Self::Surface => 0.0, Self::Weight => 1.0, Self::Normal => 2.0, Self::Selection => 3.0, Self::Status => 4.0 }
    }
    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "surface" => Self::Surface, "weight" => Self::Weight, "normal" => Self::Normal,
            "selection" => Self::Selection, "status" => Self::Status, _ => return None,
        })
    }
}

/// [design] Defaults of the parts the aid's value leaves out, and the settings row's look
/// ([`setting_config`]). Round 7 made `height` the default shape: at a road shoulder it
/// turned the broken asphalt edge into ground-lit dirt and left the road top alone, where
/// the hybrid shapes lost the effect or striped the road's lip (the user's comparison
/// pictures of round 7 [observed], #2661). `hybrid`, `hybrid-v1` and `penetration` stay
/// for the aid.
pub const DEFAULT_BAND: f32 = 8.0;
pub const DEFAULT_SHAPE: ContactShape = ContactShape::Height;
pub const DEFAULT_SHADING: Shading = Shading::Full;

/// `flat` and `steep` are hybrid v2's widths (resolved: `band / 16` and `6 band` unless
/// the value gives them); the other shapes do not read them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlendConfig { pub band: f32, pub variant: Variant, pub shape: ContactShape, pub resolution: u32, pub shading: Shading, pub flat: f32, pub steep: f32 }

impl BlendConfig {
    /// The enabled material's controls (nif_surface_bindings.wesl `mesh_terrain`): band,
    /// view, contact shape, shading.
    pub fn controls(&self) -> Vec4 {
        Vec4::new(self.band, self.variant.number(), self.shape.number(), self.shading.number())
    }
    /// The enabled material's `mesh_terrain_widths`: flat and steep widths, z and w unused.
    pub fn width_controls(&self) -> Vec4 {
        Vec4::new(self.flat, self.steep, 0.0, 0.0)
    }
    #[cfg(test)]
    pub fn widths(&self) -> ContactWidths {
        ContactWidths { flat: self.flat, band: self.band, steep: self.steep }
    }
}

const SYNTAX: &str = "expected [positive band:]surface|weight|normal|selection|status[:height|penetration|hybrid|hybrid-v1[:512|1024|2048[:albedo|full[:flat,steep]]]]";
const WIDTHS: &str = "widths must be flat,steep: finite positive numbers with flat <= band <= steep, for the hybrid shape only";

/// [design] `[band:]view[:shape[:resolution[:shading[:flat,steep]]]]`. The first part is
/// the view when it names one, otherwise the band (empty: the default). An empty later
/// part takes its default, except the widths: a sixth part must be a `flat,steep` pair,
/// and only with the hybrid (v2) shape. Old short forms (`surface`, `:selection`,
/// `24:status`) keep their meaning, with the new defaults for what they leave out.
pub fn parse(value: &str) -> Result<BlendConfig, &'static str> {
    let mut parts: Vec<&str> = value.split(':').collect();
    let band = if Variant::parse(parts[0]).is_some() { DEFAULT_BAND } else {
        let band = parts.remove(0);
        if band.is_empty() { DEFAULT_BAND } else { band.parse::<f32>().map_err(|_| "band must be a finite positive number")? }
    };
    if !band.is_finite() || band <= 0.0 { return Err("band must be a finite positive number"); }
    let mut parts = parts.into_iter();
    let variant = parts.next().and_then(Variant::parse).ok_or(SYNTAX)?;
    let shape = match parts.next().unwrap_or("") {
        "" => DEFAULT_SHAPE, "height" => ContactShape::Height, "penetration" => ContactShape::Penetration,
        "hybrid" => ContactShape::Hybrid, "hybrid-v1" => ContactShape::HybridV1, _ => return Err(SYNTAX),
    };
    let resolution = match parts.next().unwrap_or("") {
        "" => DEFAULT_RESOLUTION,
        r => r.parse::<u32>().ok().filter(|r| RESOLUTIONS.contains(r)).ok_or("resolution must be 512, 1024 or 2048")?,
    };
    let shading = match parts.next().unwrap_or("") {
        "" => DEFAULT_SHADING, "albedo" => Shading::Albedo, "full" => Shading::Full, _ => return Err(SYNTAX),
    };
    let widths = match parts.next() {
        None => ContactWidths::from_band(band),
        Some(pair) => {
            if shape != ContactShape::Hybrid { return Err(WIDTHS); }
            let (flat, steep) = pair.split_once(',').ok_or(WIDTHS)?;
            let number = |w: &str| w.parse::<f32>().map_err(|_| WIDTHS);
            ContactWidths { flat: number(flat)?, band, steep: number(steep)? }
        }
    };
    if shape == ContactShape::Hybrid && !widths.valid() { return Err(WIDTHS); }
    if parts.next().is_some() { return Err(SYNTAX); }
    Ok(BlendConfig { band, variant, shape, resolution, shading, flat: widths.flat, steep: widths.steep })
}

/// The test aid's value, read once; `None` when it is not set or rejected (said once in the
/// log), and the settings row decides then.
pub fn aid() -> Option<BlendConfig> {
    static CONFIG: OnceLock<Option<BlendConfig>> = OnceLock::new();
    *CONFIG.get_or_init(|| {
        let value = match std::env::var("NVF_PLAY_TEST_MESH_BLEND") {
            Ok(value) => value,
            Err(std::env::VarError::NotPresent) => return None,
            Err(_) => { warn!("mesh terrain blend: rejected NVF_PLAY_TEST_MESH_BLEND: value is not Unicode; the settings row decides"); return None; }
        };
        parse(&value).map_err(|e| warn!("mesh terrain blend: rejected NVF_PLAY_TEST_MESH_BLEND={value:?}: {e}; the settings row decides")).ok()
    })
}

/// [design] What the settings row draws: the aid's defaults, `8:surface:height:1024:full`.
pub fn setting_config() -> BlendConfig {
    parse("surface").expect("the default value parses")
}

/// [design] The blend wanted now: the aid's value when given (comparison runs), else the
/// row's look while the row is on, else off.
pub fn desired(aid: Option<BlendConfig>, row: bool) -> Option<BlendConfig> {
    aid.or_else(|| row.then(setting_config))
}

/// Whether the blend is on, for the spawns that record its census and captures
/// (world.rs::blend_census, terrain.rs::spawn_terrain); set by [`follow_setting`] only.
static RECORDING: AtomicBool = AtomicBool::new(false);

/// [design] The blend is on: a spawn records its census and landscape captures now.
pub fn recording() -> bool {
    RECORDING.load(Ordering::Relaxed)
}

/// [design] The blend's state while the game runs.
#[derive(Resource, Default)]
pub struct BlendState {
    /// The look drawn; `None` while off.
    pub active: Option<BlendConfig>,
    /// The census of the shapes drawn while it was off is still to be taken
    /// (world.rs::blend_backfill), set as it turns on.
    pub shapes_pending: bool,
    /// The cells whose capture was tried since it turned on
    /// (terrain_surface.rs::backfill_cells).
    pub tried: HashSet<Entity>,
    /// Switched off this frame: [`stop`] puts the ordinary materials back in PostUpdate.
    stopping: bool,
    /// Off, the material gate still open: frames in a row with no blend or capture
    /// material left ([`close_gate`]).
    draining: Option<u32>,
}

/// Run condition: the blend is on.
pub fn active(state: Res<BlendState>) -> bool {
    state.active.is_some()
}

fn stopping(state: Res<BlendState>) -> bool {
    state.stopping
}

fn draining(state: Res<BlendState>) -> bool {
    state.draining.is_some()
}

/// [design] The gate of the blend's two material types, the enabled clones'
/// [`NifBlendMaterial`] and the bake's `SurfaceCaptureMaterial`, and of the bake view's
/// extraction (vendored bevy_pbr `MaterialGate`, engine/vendor/README.md). Open from the
/// moment the blend turns on until [`close_gate`] finds nothing of either type left after
/// it turned off; closed in a run that never turns it on.
#[derive(Resource, Clone)]
pub struct BlendGate(pub MaterialGate);

/// Hybrid v2's resolved widths and the weight formula's version, appended to the start
/// and ready lines: ` flat=... steep=... weight-version=2`, or ` weight-version=1` for the
/// shapes round 6 left unchanged.
fn weight_fields(c: &BlendConfig) -> String {
    if c.shape == ContactShape::Hybrid {
        format!(" flat={} steep={} weight-version={}", c.flat, c.steep, c.shape.weight_version())
    } else {
        format!(" weight-version={}", c.shape.weight_version())
    }
}

/// The resolved configuration; `start band=... variant=...` stays the line's prefix and
/// round 5's fields keep their order before the appended weight fields.
fn start_line(c: &BlendConfig) -> String {
    format!("mesh terrain blend: start band={} variant={} resolution={} noise=0 shape={} shading={}{}", c.band, c.variant.name(), c.resolution, c.shape.name(), c.shading.name(), weight_fields(c))
}

/// [design] Why a tracked shape cannot blend, one bit each; the order is the first-reason
/// precedence of the `census-shape` lines and the order of the ready line's counts.
/// Reference reasons (world.rs::spawn_model), shape reasons, the material's state and
/// ownership, then each material predicate (from `world.rs`, fnv-world `placed.rs`,
/// nvf-assets `model.rs::PreparedModel` and `nif_material.rs::NifSurface`).
pub mod reason {
    pub const WORLD: u32 = 1 << 0;
    pub const RIG: u32 = 1 << 1;
    pub const PARTICLES: u32 = 1 << 2;
    pub const POSED: u32 = 1 << 3;
    pub const DAMAGE_STAGES: u32 = 1 << 4;
    pub const DEST: u32 = 1 << 5;
    pub const DISTANT: u32 = 1 << 6;
    pub const EMITTANCE: u32 = 1 << 7;
    pub const MORPH: u32 = 1 << 8;
    pub const MISSING_SOURCE: u32 = 1 << 9;
    pub const TEMPORARY: u32 = 1 << 10;
    pub const ALPHA_MODE: u32 = 1 << 11;
    pub const UNLIT: u32 = 1 << 12;
    pub const REFRACTION: u32 = 1 << 13;
    pub const FIRST_PERSON: u32 = 1 << 14;
    pub const DEPTH_WRITE: u32 = 1 << 15;
    pub const SPECULAR_TRANSMISSION: u32 = 1 << 16;
    pub const DIFFUSE_TRANSMISSION: u32 = 1 << 17;
    pub const FALLOFF: u32 = 1 << 18;
    pub const HOLES_VALUE: u32 = 1 << 19;
    pub const HOLES_TEXTURE: u32 = 1 << 20;
    pub const FACEGEN: u32 = 1 << 21;
    pub const PIPBOY: u32 = 1 << 22;
    pub const ADD_LINEAR: u32 = 1 << 23;
    pub const SOFT: u32 = 1 << 24;
    pub const DEPTH_BIAS: u32 = 1 << 25;
    /// Bits and the names the logs use (`excluded-<name>=`).
    pub const NAMES: [(u32, &str); 26] = [
        (WORLD, "world"), (RIG, "rig"), (PARTICLES, "particles"), (POSED, "posed"),
        (DAMAGE_STAGES, "damage-stages"), (DEST, "dest"), (DISTANT, "distant"),
        (EMITTANCE, "emittance"), (MORPH, "morph"), (MISSING_SOURCE, "missing-source"),
        (TEMPORARY, "temporary"), (ALPHA_MODE, "alpha-mode"), (UNLIT, "unlit"),
        (REFRACTION, "refraction"), (FIRST_PERSON, "first-person"), (DEPTH_WRITE, "depth-write"),
        (SPECULAR_TRANSMISSION, "specular-transmission"), (DIFFUSE_TRANSMISSION, "diffuse-transmission"),
        (FALLOFF, "falloff"), (HOLES_VALUE, "holes-value"), (HOLES_TEXTURE, "holes-texture"),
        (FACEGEN, "facegen"), (PIPBOY, "pipboy"), (ADD_LINEAR, "add-linear"), (SOFT, "soft"),
        (DEPTH_BIAS, "depth-bias-nonfinite"),
    ];
    /// The first reason in precedence order, `none` for an empty mask.
    pub fn first(mask: u32) -> &'static str {
        NAMES.iter().find(|(bit, _)| mask & bit != 0).map_or("none", |(_, name)| name)
    }
    /// Every reason, `+`-joined, `none` for an empty mask.
    pub fn list(mask: u32) -> String {
        let names: Vec<&str> = NAMES.iter().filter(|(bit, _)| mask & bit != 0).map(|(_, name)| *name).collect();
        if names.is_empty() { "none".into() } else { names.join("+") }
    }
}

/// [design] Code facts of a placed reference (world.rs::spawn_model, fnv-world
/// placed.rs/index.rs, model.rs::PreparedModel); no asset-name selection.
#[derive(Clone, Copy, Debug, Default)]
pub struct ReferenceFacts {
    /// In an exterior cell of the world being drawn.
    pub exterior: bool,
    /// A rig or skinned shapes.
    pub rigged: bool,
    pub particles: bool,
    /// Jointed, loose bodies, or drawn in a stored pose.
    pub posed: bool,
    pub damage_stages: bool,
    /// The base record has DEST (fnv-game/src/data.rs).
    pub destructible: bool,
    /// Drawn as a distant imposter.
    pub distant: bool,
}

/// Bits whose condition holds, combined.
fn mask_of<const N: usize>(conditions: [(bool, u32); N]) -> u32 {
    conditions.into_iter().filter(|c| c.0).fold(0, |mask, (_, bit)| mask | bit)
}

pub fn reference_reasons(f: ReferenceFacts) -> u32 {
    use reason::*;
    mask_of([(!f.exterior, WORLD), (f.rigged, RIG), (f.particles, PARTICLES), (f.posed, POSED),
        (f.damage_stages, DAMAGE_STAGES), (f.destructible, DEST), (f.distant, DISTANT)])
}

/// [design] A shape driven by the reference's emittance, or a registered morph target.
pub fn shape_reasons(external_emittance: bool, morph: bool) -> u32 {
    mask_of([(external_emittance, reason::EMITTANCE), (morph, reason::MORPH)])
}

/// [design] The base record types whose rigid shapes can blend: STAT, and SCOL, a static
/// collection drawn as the one model the editor generated from its statics
/// [documented: GECK wiki "Static Collection", revision 40901, snapshot 2026-10-28:
/// "composed of Static forms", "do not react to Havok physics and cannot be destroyed",
/// its Model "the NIF file used for the object", usually generated on creation; GECK wiki
/// "Bethsoft Tutorial Static Collections", revision 38886: grouping "creates a new .NIF"].
/// world.rs draws a SCOL reference as that one model, as it draws a STAT (`spawn_model`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BaseType { Stat, Scol }
impl BaseType {
    /// The census's type of a base record signature; `None` for every other record.
    pub fn of(sig: &[u8; 4]) -> Option<Self> {
        match sig { b"STAT" => Some(Self::Stat), b"SCOL" => Some(Self::Scol), _ => None }
    }
    /// The record signature, as the `census-shape` lines name it.
    pub fn name(self) -> &'static str {
        match self { Self::Stat => "STAT", Self::Scol => "SCOL" }
    }
}

/// Selection of the reference: a STAT or SCOL without reference reasons (the census
/// records only those; world.rs::blend_census).
#[cfg(test)]
pub fn eligible_reference(sig: &[u8; 4], facts: ReferenceFacts) -> bool {
    BaseType::of(sig).is_some() && reference_reasons(facts) == 0
}

/// [design] Each material predicate the blend cannot take, one reason each. Admitted since
/// round 6: ordinary `Blend` (its transparent phase, blend state, sorting and depth-write
/// state stay the shared specialization's), a finite depth bias (copied exactly), the lit
/// mesh decal flag (the shared specialization keeps its bias) and `alpha_cut` (the discard
/// runs before terrain shading; copied unchanged); `Mask` was already admitted. Add,
/// Multiply, Premultiplied and AlphaToCoverage stay out; holes and FaceGen always, because
/// the blend type repurposes their slots (nif_surface_blend.rs module notes).
pub fn material_reasons(m: &NifMaterial) -> u32 {
    use reason::*;
    let (b, s) = (&m.base, &m.extension);
    mask_of([
        (!matches!(b.alpha_mode, AlphaMode::Opaque | AlphaMode::Mask(_) | AlphaMode::Blend), ALPHA_MODE),
        (b.unlit, UNLIT), (s.refract, REFRACTION), (s.first_person, FIRST_PERSON), (s.depth_write, DEPTH_WRITE),
        (b.specular_transmission != 0.0, SPECULAR_TRANSMISSION), (b.diffuse_transmission != 0.0, DIFFUSE_TRANSMISSION),
        (s.falloff != Falloff::NONE.params, FALLOFF),
        (s.holes != nvf_assets::nif_material::NO_HOLES, HOLES_VALUE), (s.holes_texture.is_some(), HOLES_TEXTURE),
        (s.facegen_offset.is_some(), FACEGEN), (s.pipboy_screen, PIPBOY), (s.add_linear, ADD_LINEAR),
        (s.soft != Vec4::ZERO, SOFT), (!b.depth_bias.is_finite(), DEPTH_BIAS),
    ])
}

/// The conversion reads the full mask ([`shape_mask`]); this is its material part alone.
#[cfg(test)]
pub fn eligible_material(m: &NifMaterial) -> bool {
    material_reasons(m) == 0
}

pub use nvf_assets::nif_surface_blend::BlendShape;

/// [design] Metadata of one spawned STAT or SCOL shape while the blend is on
/// (world.rs::spawn_model, or blend_backfill for what was drawn while it was off),
/// recorded before any filtering: its reference, base, base type and model, the shape's
/// index in the model (in a rigged model, its index among the drawn shapes), and its
/// reference and shape reasons. Diagnostics only: conversion follows [`BlendShape`] alone.
#[derive(Component, Clone, Debug)]
pub struct BlendCensus {
    pub reference: u32,
    pub base: u32,
    pub base_type: BaseType,
    pub model: Arc<str>,
    pub shape: usize,
    pub reasons: u32,
}

/// What an enabled clone was made from, compared each frame so that a producer editing
/// the source under the same handle (blended cut-outs flipping Blend and Mask, NIF block
/// edits) rebuilds the clone without inserting a new ordinary material component.
#[derive(Clone, Copy, Debug, PartialEq)]
struct SourceStamp { alpha_mode: AlphaMode, alpha_cut: u32, alpha: u32, depth_bias: u32, decal: bool, roughness: u32, blended_cutout: Option<u32> }
impl SourceStamp {
    fn of(m: &NifMaterial) -> Self {
        Self {
            alpha_mode: m.base.alpha_mode, alpha_cut: m.extension.alpha_cut.to_bits(), alpha: m.base.base_color.alpha().to_bits(),
            depth_bias: m.base.depth_bias.to_bits(), decal: m.extension.decal, roughness: m.base.perceptual_roughness.to_bits(),
            blended_cutout: m.extension.blended_cutout.map(f32::to_bits),
        }
    }
}

/// [design] The census of the currently resident tracked shapes (not unique materials or
/// cumulative spawns): `stat_shapes` and `scol_shapes` by base type, `excluded`,
/// `pending` and `reasons` over both; `reasons` overlap, one count per [`reason::NAMES`]
/// entry; `admitted` counts the enabled shapes whose source is Blend, has a depth bias,
/// the decal flag, is Mask, or has an alpha cut.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Census {
    /// The prefix's `eligible-shapes` (compared for refreshes, printed by `ready_line`).
    eligible: usize,
    stat_shapes: usize,
    scol_shapes: usize,
    excluded: usize,
    pending: usize,
    reasons: [usize; reason::NAMES.len()],
    admitted: [usize; 5],
}
impl Census {
    fn count(&mut self, base_type: BaseType, mask: u32) {
        match base_type {
            BaseType::Stat => self.stat_shapes += 1,
            BaseType::Scol => self.scol_shapes += 1,
        }
        self.excluded += usize::from(mask != 0);
        self.pending += usize::from(mask & reason::MISSING_SOURCE != 0);
        for (n, (bit, _)) in self.reasons.iter_mut().zip(reason::NAMES) {
            *n += usize::from(mask & bit != 0);
        }
    }
    fn admit(&mut self, m: &NifMaterial) {
        let flags = [
            m.base.alpha_mode == AlphaMode::Blend, m.base.depth_bias != 0.0, m.extension.decal,
            matches!(m.base.alpha_mode, AlphaMode::Mask(_)), m.extension.alpha_cut != 0.0,
        ];
        for (n, f) in self.admitted.iter_mut().zip(flags) { *n += usize::from(f); }
    }
    /// Appended to the ready line after round 5's fields and the weight fields; round 7's
    /// `scol-shapes` comes last, after the admitted counts, so the earlier fields keep
    /// their order and neighbours.
    fn fields(&self) -> String {
        use std::fmt::Write;
        let mut out = format!(" stat-shapes={} excluded-shapes={} pending-material-shapes={}", self.stat_shapes, self.excluded, self.pending);
        for ((_, name), n) in reason::NAMES.iter().zip(self.reasons) {
            let _ = write!(out, " excluded-{name}={n}");
        }
        for (name, n) in ["blend", "bias", "decal", "mask", "alpha-cut"].iter().zip(self.admitted) {
            let _ = write!(out, " admitted-{name}={n}");
        }
        let _ = write!(out, " scol-shapes={}", self.scol_shapes);
        out
    }
}

/// [design] Census-only changes refresh the ready line at most once per this many frames
/// (a publication reports at once), so cell streaming does not log every frame.
const REFRESH_FRAMES: u32 = 60;
/// [design] At most this many `census-shape` lines per run; each line is a distinct base
/// form, shape index and reason mask (the first reference seen of it).
const SHAPE_LINES: usize = 2048;

#[derive(Resource, Default)]
struct Variants {
    generation: u64,
    readiness: Option<BlendReadiness>,
    publications: HashMap<AssetId<NifBlendMaterial>, crate::terrain_surface::PublishedSurface>,
    ordinary_seen: HashMap<Entity, u32>,
    materials: HashMap<AssetId<NifMaterial>, (Handle<NifBlendMaterial>, SourceStamp)>,
    reported: Option<u64>,
    reported_census: Option<Census>,
    frames_since_report: u32,
    logged: HashSet<(u32, usize, u32)>,
    shape_lines: usize,
}

/// [design] Asset IDs alone cross this acknowledgement boundary; main-world handles
/// in candidates and BlendShape keep both image generations alive until the switch.
#[derive(Resource, Clone, Default)]
pub(crate) struct BlendReadiness {
    ready: Arc<std::sync::Mutex<HashSet<AssetId<NifBlendMaterial>>>>,
    settled: Arc<std::sync::atomic::AtomicU64>,
}
impl BlendReadiness {
    pub(crate) fn usable(&self, id: AssetId<NifBlendMaterial>) -> bool { self.ready.lock().unwrap().contains(&id) }
    pub(crate) fn settled(&self, revision: u64) -> bool { self.settled.load(Ordering::Acquire) == revision }
}
#[derive(Resource, Default)]
struct ReadinessInput { ids: HashSet<AssetId<NifBlendMaterial>>, ages: HashMap<AssetId<NifBlendMaterial>, u32>, delay: u32 }

fn install_readiness(app: &mut App, gate: MaterialGate) {
    use bevy::render::{ExtractSchedule, Render, RenderApp, RenderSystems};
    let shared = BlendReadiness::default();
    app.insert_resource(shared.clone()).insert_resource(Variants { readiness: Some(shared.clone()), ..default() });
    if let Some(render) = app.get_sub_app_mut(RenderApp) {
        let delay = std::env::var("NVF_PLAY_TEST_MESH_BLEND_READY_DELAY").ok().and_then(|s| s.parse::<u32>().ok()).unwrap_or(0).min(120);
        let extract_gate = gate.clone();
        render.insert_resource(shared).insert_resource(ReadinessInput { delay, ..default() })
            .add_systems(ExtractSchedule, extract_readiness.run_if(move || extract_gate.is_open()))
            .add_systems(Render, acknowledge_materials.run_if(move || gate.is_open()).in_set(RenderSystems::Cleanup));
    }
}
fn extract_readiness(mut input: ResMut<ReadinessInput>, variants: bevy::render::Extract<Res<Variants>>) {
    input.ids = variants.materials.values().map(|(h, _)| h.id()).chain(variants.publications.keys().copied()).collect();
}
fn acknowledge_materials(
    mut input: ResMut<ReadinessInput>, shared: Res<BlendReadiness>,
    materials: Res<bevy::render::erased_render_asset::ErasedRenderAssets<bevy::pbr::PreparedMaterial>>,
    allocators: Res<bevy::render::material_bind_groups::MaterialBindGroupAllocators>,
) {
    let ids = input.ids.clone();
    input.ages.retain(|id, _| ids.contains(id));
    let mut ready = shared.ready.lock().unwrap();
    ready.clear();
    for id in ids {
        let age = input.ages.entry(id).or_default();
        *age = age.saturating_add(1);
        if *age <= input.delay { continue; }
        let Some(material) = materials.get(id) else { continue };
        let bound = allocators.get(&std::any::TypeId::of::<NifBlendMaterial>()).and_then(|a| a.get(material.binding.group)).is_some_and(|b| b.bind_group().is_some());
        if bound { ready.insert(id); }
    }
}

#[derive(SystemSet, Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct BlendConversion;

/// [design] Registered in every run, so the settings row can switch the blend while the
/// game runs: the materials' plugins behind the closed [`BlendGate`] and empty resources;
/// every system that captures, bakes or converts runs only while the blend is on
/// ([`active`]), the restoration only in the frame it turns off, the gate's closing only
/// after that. What still runs while it is off: graphics-enhanced.md §3.12 "Off".
pub struct MeshTerrainBlendPlugin;

impl Plugin for MeshTerrainBlendPlugin {
    fn build(&self, app: &mut App) {
        let gate = MaterialGate::new();
        install_readiness(app, gate.clone());
        crate::terrain_surface::install_continuity_probe(app);
        app.add_plugins((MaterialPlugin::<NifBlendMaterial>::gated(gate.clone()), crate::terrain_surface::SurfaceBakePlugin { gate: gate.clone() }))
            .insert_resource(BlendGate(gate))
            .init_resource::<SurfaceCache>().init_resource::<Variants>().init_resource::<BlendState>()
            .add_systems(Update, (
                follow_setting.before(crate::world::stream),
                close_gate.after(follow_setting).run_if(draining),
                (crate::world::blend_backfill, crate::terrain_surface::backfill_cells).after(follow_setting).run_if(active),
            ))
            .add_systems(PostUpdate, conversion_order((crate::terrain_surface::invalidate_context.run_if(active), stop.run_if(stopping), apply_variants.run_if(active)).chain().in_set(BlendConversion)))
            // Source edits (including this frame's producers) must be versioned before
            // accepting a bake receipt. Conversion consumes this publication next frame.
            .add_systems(PostUpdate, (crate::terrain_surface::refresh_captures, update_cache).chain()
                .after(bevy::asset::AssetEventSystems).after(bevy::transform::TransformSystems::Propagate).run_if(active));
    }
}

/// The shapes the blend tracks, for switching it off, and whether a producer has put an
/// ordinary material on them.
type Switched<'w, 's> = Query<'w, 's, (Entity, Option<&'static BlendShape>, Has<MeshMaterial3d<NifMaterial>>), Or<(With<BlendShape>, With<BlendCensus>)>>;

/// [design] Each frame, before the world streams: the blend [`desired`] from the aid and
/// the settings row (`Graphics::mesh_terrain_blend`), switched on or off when it differs
/// from what is drawn. On: the gate opened, the start line, the atlas checked against the
/// device, the census of what is already drawn queued. Off: no more spawns record, and
/// [`stop`] puts the materials back in PostUpdate, after this frame's material producers;
/// a different look (not reachable today: the aid is read once, the row has one look)
/// starts the frame after. No restart either way.
fn follow_setting(
    graphics: Option<Res<crate::graphics::Graphics>>,
    device: Option<Res<bevy::render::renderer::RenderDevice>>,
    gate: Res<BlendGate>,
    mut state: ResMut<BlendState>,
    mut cache: ResMut<SurfaceCache>,
    mut variants: ResMut<Variants>,
) {
    let want = desired(aid(), graphics.is_some_and(|g| g.mesh_terrain_blend));
    if want == state.active || state.stopping { return; }
    if state.active.is_some() {
        RECORDING.store(false, Ordering::Relaxed);
        state.active = None;
        state.stopping = true;
        return;
    }
    let Some(c) = want else { return };
    gate.0.set_open(true);
    RECORDING.store(true, Ordering::Relaxed);
    info!("{}", start_line(&c));
    cache.start(c.resolution, device.map(|d| d.limits().max_texture_dimension_2d));
    *variants = Variants { readiness: variants.readiness.take(), ..default() };
    state.active = Some(c);
    state.shapes_pending = true;
    state.draining = None;
}

/// [design] The blend off, in PostUpdate where the conversion runs ([`conversion_order`]:
/// after the Update producers, texture swaps, fades, effect shaders and overrides, and the
/// PostUpdate ones, the NIF mirror and the blended cut-outs, with their commands applied).
/// Each converted shape leaves the blend material: an ordinary material a producer put on
/// it this frame stays (a texture swap's copy, whose generation the swap has consumed, or
/// a fade's or override's copy); only a shape with none gets [`BlendShape::original`] back
/// (the shared see-through shadow bookkeeping follows either change). Then the census and
/// the landscape captures are removed, the bake cancelled and the atlas and clones dropped
/// ([`SurfaceCache::release`]), one `stop` line, and [`close_gate`] waits for the
/// materials to go.
fn stop(mut commands: Commands, mut state: ResMut<BlendState>, mut cache: ResMut<SurfaceCache>, mut variants: ResMut<Variants>, shapes: Switched, cells: Query<Entity, With<crate::terrain_surface::SurfaceCell>>) {
    let (mut restored, mut kept) = (0, 0);
    for (entity, shape, ordinary) in shapes.iter() {
        let mut e = commands.entity(entity);
        if let Some(enabled) = shape.filter(|s| s.enabled.is_some()) {
            e.try_remove::<MeshMaterial3d<NifBlendMaterial>>();
            if ordinary {
                kept += 1;
            } else {
                e.try_insert(MeshMaterial3d(enabled.original.clone()));
            }
            restored += 1;
        }
        e.try_remove::<(BlendShape, BlendCensus)>();
    }
    for cell in cells.iter() {
        commands.entity(cell).try_remove::<crate::terrain_surface::SurfaceCell>();
    }
    let released = cells.iter().count();
    cache.release(&mut commands);
    *variants = Variants { readiness: variants.readiness.take(), ..default() };
    *state = BlendState { draining: Some(0), ..default() };
    info!("mesh terrain blend: stop restored-shapes={restored} tracked-shapes={} released-cells={released} kept-producer-materials={kept}", shapes.iter().count());
}

/// [design] Frames in a row with no blend or capture material, asset or component, before
/// the gate closes: the removals made by [`stop`] are extracted and unloaded in the render
/// world in the first of them (vendored bevy_pbr `MaterialGate`).
const IDLE_FRAMES: u32 = 3;

type BlendDrawn<'w, 's> = Query<'w, 's, (), Or<(With<MeshMaterial3d<NifBlendMaterial>>, With<MeshMaterial3d<crate::terrain_surface::SurfaceCaptureMaterial>>)>>;

/// [design] While off with the gate open: once nothing of the blend's two material types
/// has been left for [`IDLE_FRAMES`] frames, the gate closes; the render world then drops
/// their allocators and shader handles and puts Bevy's draw functions back
/// (terrain_surface.rs `uninstall_bake_receipts`), and an `idle` line says so. A switch
/// back on meanwhile keeps the gate open ([`follow_setting`]).
fn close_gate(
    mut state: ResMut<BlendState>,
    gate: Res<BlendGate>,
    blends: Res<Assets<NifBlendMaterial>>,
    captures: Res<Assets<crate::terrain_surface::SurfaceCaptureMaterial>>,
    drawn: BlendDrawn,
) {
    let Some(frames) = state.draining.as_mut() else { return };
    *frames = if blends.is_empty() && captures.is_empty() && drawn.is_empty() { *frames + 1 } else { 0 };
    if *frames >= IDLE_FRAMES {
        gate.0.set_open(false);
        state.draining = None;
        info!("mesh terrain blend: idle materials-released=true");
    }
}

/// [design] Where the conversion runs: ordinary producers (swaps, alpha, scripted fades)
/// finish in Update; in PostUpdate it follows the scripted NIF edits (`nif_mirror`, their
/// commands applied, so an edited source is never left beside the blend material), the
/// blended cut-outs' same-handle alpha-mode edits, and the see-through shadow bookkeeping
/// (which must see a restored ordinary material before it is swapped away, or a finished
/// fade keeps its shadow markers), and precedes asset events and mesh/material change
/// marking.
fn conversion_order<M>(system: impl IntoScheduleConfigs<bevy::ecs::system::ScheduleSystem, M>) -> bevy::ecs::schedule::ScheduleConfigs<bevy::ecs::system::ScheduleSystem> {
    system
        .after(crate::nif_mirror::MirrorSystems)
        .after(nvf_assets::nif_material::BlendedCutoutSystems)
        .after(nvf_assets::nif_material::SeeThroughShadowSystems)
        .before(bevy::asset::AssetEventSystems)
        .before(bevy::mesh::mark_3d_meshes_as_changed_if_their_assets_changed)
}

fn enabled_material(source: &NifMaterial, c: BlendConfig, surface: &crate::terrain_surface::PublishedSurface) -> NifBlendMaterial {
    NifBlendMaterial {
        base: source.base.clone(),
        extension: NifSurfaceBlend::from_source(&source.extension, c.controls(), c.width_controls(),
            Vec4::new(surface.origin.0 as f32, surface.origin.1 as f32, surface.coverage as f32, 0.0), surface.colour.clone(), surface.height.clone()),
    }
}

/// Temporary material owners (fades, actor alpha, overrides), whose copies never blend.
type Temporary = (Has<nvf_play::fade::Fading>, Has<crate::effect_shaders::Faded>, Has<MaterialOverride>);
/// The see-through shadow markers (nif_material.rs `see_through_casts_no_shadow`).
type ShadowMarks = (Has<NotShadowCaster>, Has<SeeThroughNoShadow>);
/// Every shape the blend tracks: convertible ones ([`BlendShape`]) and census-only ones.
type Tracked<'w, 's> = Query<'w, 's, (Entity, Option<&'static mut BlendShape>, Option<&'static BlendCensus>, Option<Ref<'static, MeshMaterial3d<NifMaterial>>>, Temporary, ShadowMarks), Or<(With<BlendShape>, With<BlendCensus>)>>;

fn apply_variants(
    mut commands: Commands,
    state: Res<BlendState>,
    cache: Res<SurfaceCache>,
    mut variants: ResMut<Variants>,
    materials: Res<Assets<NifMaterial>>,
    mut blends: ResMut<Assets<NifBlendMaterial>>,
    mut shapes: Tracked,
) {
    let Some(c) = state.active else { return };
    apply_configured_variants(&mut commands, &cache, &mut variants, &materials, &mut blends, &mut shapes, c);
}

/// The full reason mask of a tracked shape this frame: its recorded reference and shape
/// reasons, temporary ownership, and its source material's reasons (or that the source
/// is not loaded).
fn shape_mask(recorded: u32, temporary: bool, source: Option<&NifMaterial>) -> u32 {
    let owned = if temporary { reason::TEMPORARY } else { 0 };
    recorded | owned | source.map_or(reason::MISSING_SOURCE, material_reasons)
}

// [design] Explicit config boundary lets ECS tests publish without process-wide env state.
fn apply_configured_variants(
    commands: &mut Commands,
    cache: &SurfaceCache,
    variants: &mut Variants,
    materials: &Assets<NifMaterial>,
    blends: &mut Assets<NifBlendMaterial>,
    shapes: &mut Tracked,
    c: BlendConfig,
) {
    let revision = cache.published.as_ref().map_or(0, |p| p.revision);
    if variants.generation != revision {
        variants.generation = revision;
        variants.materials.clear();
    }
    // [design] Restrict old drawable clones too, while their replacement prepares.
    // Only uniform POD changes; the image handles remain fixed (no colour upload).
    for (id, publication) in &variants.publications {
        let coverage = cache.coverage_for(publication) as f32;
        if blends.get(*id).is_some_and(|m| m.extension.mesh_terrain_origin.z != coverage) {
            blends.get_mut(*id).unwrap().extension.mesh_terrain_origin.z = coverage;
        }
    }
    // A producer can edit an owned source in place and republish the same handle
    // (NIF block material edits). Evict once per source before creating shared clones.
    let mut ordinary_live = HashSet::new();
    for (entity, shape, _, drawn, _, _) in shapes.iter() {
        if shape.is_none() { continue; }
        if let Some(drawn) = drawn {
            ordinary_live.insert(entity);
            if variants.ordinary_seen.insert(entity, drawn.last_changed().get()) != Some(drawn.last_changed().get()) {
                variants.materials.remove(&drawn.0.id());
            }
        }
    }
    variants.ordinary_seen.retain(|e, _| ordinary_live.contains(e));
    let mut pending = false;
    let mut census = Census::default();
    for (entity, shape, tracked, drawn, (fading, faded, overridden), (no_shadow, see_through_mark)) in shapes.iter_mut() {
        let temporary = fading || faded || overridden;
        let recorded = tracked.map_or(0, |t| t.reasons);
        let Some(mut shape) = shape else {
            // Census only: never converted.
            let mask = shape_mask(recorded, temporary, drawn.as_ref().and_then(|d| materials.get(&d.0)));
            if let Some(t) = tracked {
                census.count(t.base_type, mask);
                log_shape(variants, t, mask);
            }
            continue;
        };
        // [design] Respect a material override, retaining its actual source handle too.
        if let Some(drawn) = &drawn {
            shape.original = drawn.0.clone();
        }
        let source = materials.get(&shape.original);
        let mask = shape_mask(recorded, temporary, source);
        if let Some(t) = tracked {
            census.count(t.base_type, mask);
            log_shape(variants, t, mask);
        }
        let allowed = mask == 0;
        census.eligible += usize::from(allowed);
        let (Some(surface), Some(source), true) = (cache.published.as_ref(), source, allowed) else {
            if shape.enabled.is_some() {
                commands.entity(entity).remove::<MeshMaterial3d<NifBlendMaterial>>().insert(MeshMaterial3d(shape.original.clone()));
            }
            shape.enabled = None;
            continue;
        };
        census.admit(source);
        let id = shape.original.id();
        let stamp = SourceStamp::of(source);
        let handle = match variants.materials.get(&id) {
            Some((handle, made)) if *made == stamp => handle.clone(),
            _ => {
                let handle = blends.add(enabled_material(source, c, surface));
                variants.publications.insert(handle.id(), surface.clone());
                variants.materials.insert(id, (handle.clone(), stamp));
                handle
            }
        };
        // Keep the old enabled handle until the render world acknowledges a prepared,
        // bound replacement. Source/temporary owners still take precedence above.
        if variants.readiness.as_ref().is_some_and(|r| !r.usable(handle.id())) {
            pending = true;
            // A non-temporary producer may have republished an ordinary source. Its
            // handle is recorded above; keep only the old drawable blend while the
            // replacement prepares, so two material types never compete for the entity.
            if shape.enabled.is_some() && drawn.is_some() { commands.entity(entity).remove::<MeshMaterial3d<NifMaterial>>(); }
            continue;
        }
        if shape.enabled.as_ref() != Some(&handle) || drawn.is_some() {
            commands.entity(entity).remove::<MeshMaterial3d<NifMaterial>>().insert(MeshMaterial3d(handle.clone()));
            // [design] The shared bookkeeping runs on ordinary material changes only; a
            // source edited under the same handle is checked here, by the same rule.
            match (nvf_assets::nif_material::see_through(source), no_shadow, see_through_mark) {
                (true, false, _) => { commands.entity(entity).try_insert((NotShadowCaster, SeeThroughNoShadow)); }
                (false, true, true) => { commands.entity(entity).try_remove::<(NotShadowCaster, SeeThroughNoShadow)>(); }
                _ => {}
            }
        }
        shape.enabled = Some(handle);
    }
    let sources: HashSet<_> = shapes.iter().filter_map(|(_, s, _, _, _, _)| s.map(|s| s.original.id())).collect();
    variants.materials.retain(|id, _| sources.contains(id));
    let live: HashSet<_> = shapes.iter().filter_map(|(_, s, _, _, _, _)| s.and_then(|s| s.enabled.as_ref().map(Handle::id))).chain(variants.materials.values().map(|(h, _)| h.id())).collect();
    variants.publications.retain(|id, _| live.contains(id));
    if let Some(readiness) = variants.readiness.as_ref().filter(|_| !pending) { readiness.settled.store(revision, Ordering::Release); }
    variants.frames_since_report = variants.frames_since_report.saturating_add(1);
    if let Some(surface) = &cache.published {
        let published = variants.reported != Some(revision);
        let refreshed = variants.reported_census.as_ref() != Some(&census) && variants.frames_since_report >= REFRESH_FRAMES;
        if published || refreshed {
            info!("{}", ready_line(surface, revision, &c, &census));
            variants.reported = Some(revision);
            variants.reported_census = Some(census);
            variants.frames_since_report = 0;
        }
    }
}

/// One bounded `census-shape` line per distinct base form, shape index and reason mask;
/// shapes whose source material is not loaded yet are left for later.
fn log_shape(variants: &mut Variants, t: &BlendCensus, mask: u32) {
    if mask & reason::MISSING_SOURCE != 0 || variants.shape_lines > SHAPE_LINES { return; }
    if !variants.logged.insert((t.base, t.shape, mask)) { return; }
    variants.shape_lines += 1;
    if variants.shape_lines > SHAPE_LINES {
        info!("mesh terrain blend: census-shape lines stopped at {SHAPE_LINES}; the ready line still counts every shape");
        return;
    }
    info!("{}", shape_line(t, mask));
}

/// `census-shape ref=0x... base=0x... model=... shape=N status=eligible|excluded
/// first-reason=... reasons=a+b base-type=STAT|SCOL` (reference: the first seen with this
/// base, shape and mask; round 7's `base-type` last, so the earlier fields keep their
/// order and neighbours).
fn shape_line(t: &BlendCensus, mask: u32) -> String {
    format!("mesh terrain blend: census-shape ref=0x{:08X} base=0x{:08X} model={} shape={} status={} first-reason={} reasons={} base-type={}",
        t.reference, t.base, t.model, t.shape, if mask == 0 { "eligible" } else { "excluded" }, reason::first(mask), reason::list(mask), t.base_type.name())
}

/// `ready eligible-shapes=... resident-cells=... generation=...` stays the line's prefix and
/// round 5's `variant= shape= resolution= shading=` follow unchanged; then the weight fields
/// and the census. The resolution is the published atlas's.
fn ready_line(surface: &crate::terrain_surface::PublishedSurface, generation: u64, c: &BlendConfig, census: &Census) -> String {
    format!("mesh terrain blend: ready eligible-shapes={} resident-cells={} generation={generation} variant={} shape={} resolution={} shading={}{}{}", census.eligible, surface.cells, c.variant.name(), c.shape.name(), surface.resolution, c.shading.name(), weight_fields(c), census.fields())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nvf_assets::nif_material::NifSurface;

    fn configured(
        mut commands: Commands, cache: Res<SurfaceCache>, mut variants: ResMut<Variants>,
        materials: Res<Assets<NifMaterial>>, mut blends: ResMut<Assets<NifBlendMaterial>>,
        mut shapes: Tracked,
    ) {
        apply_configured_variants(&mut commands, &cache, &mut variants, &materials, &mut blends, &mut shapes, parse("24:status").unwrap());
    }
    fn publish(app: &mut App) {
        let mut images = app.world_mut().resource_mut::<Assets<Image>>();
        let surface = crate::terrain_surface::PublishedSurface { colour: images.add(Image::default()), height: images.add(Image::default()), origin: (-18, -1), cells: 9, resolution: 1024, revision: 7, coverage: 511, ..default() };
        app.init_resource::<SurfaceCache>();
        let mut cache = app.world_mut().resource_mut::<SurfaceCache>();
        cache.generation = 7;
        cache.published = Some(surface);
    }
    fn simulation(app: &mut App) {
        simulation_with(app, fnv_sim::RefTable::default());
    }
    fn simulation_with(app: &mut App, refs: fnv_sim::RefTable) {
        let forms = Box::leak(Box::<fnv_sim::FormStore>::default());
        let refs = Box::leak(Box::new(refs));
        app.insert_resource(nvf_play::sim::SimState { forms, refs, world: fnv_sim::World::new(forms, refs, default()) });
    }
    /// A placed reference 42 that starts disabled: a Disable fade (nvf_play fade.rs) runs
    /// only for a reference on its way out.
    fn disabled_reference() -> fnv_sim::RefTable {
        let mut placed = crate::outfits::tests::placed_ref(42, *b"REFR");
        placed.flags |= fnv_sim::refs::FLAG_INITIALLY_DISABLED;
        let mut refs = fnv_sim::RefTable::default();
        refs.refs.insert(fnv_sim::FormId(42), placed);
        refs
    }
    #[test]
    fn publication_followed_by_scripted_swap_recreates_enabled_clone_and_restores_swap() {
        use fnv_game::functions::misc::TextureSwaps;
        #[derive(Resource, Default)] struct Swaps(TextureSwaps);
        let (mut app, _files) = crate::outfits::tests::asset_fixture();
        simulation(&mut app);
        app.insert_resource(crate::outfits::tests::empty_world_data())
            .init_resource::<Variants>().init_resource::<Assets<NifBlendMaterial>>()
            .init_resource::<crate::texture_swap::Swapped>().init_resource::<Swaps>()
            .add_systems(Update, |swaps: Res<Swaps>, sim: Res<nvf_play::sim::SimState>, world: Res<crate::world::WorldData>, mut drawing: crate::texture_swap::Swapping| drawing.sync(&swaps.0, &sim, &world))
            .add_systems(PostUpdate, conversion_order(configured));
        publish(&mut app);
        let original = app.world_mut().resource_mut::<Assets<NifMaterial>>().add(NifMaterial { base: default(), extension: default() });
        let range = crate::fade::drawn(8);
        let e = app.world_mut().spawn((BlendShape::new(original.clone()), MeshMaterial3d(original.clone()), nvf_play::RefVisual(42), nvf_assets::actor::ShapeName("road".into()), range.clone())).id();
        app.update();
        let before = app.world().get::<MeshMaterial3d<NifBlendMaterial>>(e).unwrap().0.clone();
        assert!(app.world().get::<MeshMaterial3d<NifMaterial>>(e).is_none());
        // Synthetic BC1 DDS through the production texture loader; no game data.
        let texture = "textures\\synthetic-blend-swap.dds";
        let mut dds = vec![0u8; 136]; dds[..4].copy_from_slice(b"DDS ");
        for (at, value) in [(4, 124u32), (8, 0x1007), (12, 4), (16, 4), (28, 1), (76, 32), (80, 4), (108, 0x1000)] { dds[at..at + 4].copy_from_slice(&value.to_le_bytes()); }
        dds[84..88].copy_from_slice(b"DXT1");
        app.world_mut().resource_mut::<nvf_assets::TextureCache>().offer(texture.into(), dds, 0.0);
        {
            let mut swaps = app.world_mut().resource_mut::<Swaps>();
            swaps.0.by_ref.insert(fnv_sim::FormId(42), vec![("road".into(), texture.into())]);
            swaps.0.generation = 1;
        }
        app.update();
        let shape = app.world().get::<BlendShape>(e).unwrap();
        assert_ne!(shape.original, original);
        let swapped = shape.original.clone();
        let after = app.world().get::<MeshMaterial3d<NifBlendMaterial>>(e).unwrap();
        assert_ne!(after.0, before);
        let image = app.world().resource::<Assets<NifMaterial>>().get(&swapped).unwrap().base.base_color_texture.clone().unwrap();
        assert_eq!(app.world().resource::<Assets<NifBlendMaterial>>().get(&after.0).unwrap().base.base_color_texture.as_ref(), Some(&image));
        assert!(app.world().resource::<Assets<NifMaterial>>().get(&original).unwrap().base.base_color_texture.is_none());
        assert!(app.world().get::<bevy::camera::visibility::VisibilityRange>(e) == Some(&range));
        app.update(); // The processed generation must keep the swap on enabled shapes.
        assert_eq!(app.world().get::<BlendShape>(e).unwrap().original, swapped);
        // NIF block material edits republish an owned source under the same handle.
        let before_edit = app.world().get::<MeshMaterial3d<NifBlendMaterial>>(e).unwrap().0.clone();
        app.world_mut().resource_mut::<Assets<NifMaterial>>().get_mut(&swapped).unwrap().base.perceptual_roughness = 0.31;
        app.world_mut().entity_mut(e).insert(MeshMaterial3d(swapped.clone()));
        app.update();
        let refreshed = &app.world().get::<MeshMaterial3d<NifBlendMaterial>>(e).unwrap().0;
        assert_ne!(refreshed, &before_edit);
        assert_eq!(app.world().resource::<Assets<NifBlendMaterial>>().get(refreshed).unwrap().base.perceptual_roughness, 0.31);
        app.world_mut().resource_mut::<SurfaceCache>().published = None;
        app.update();
        assert_eq!(app.world().get::<MeshMaterial3d<NifMaterial>>(e).unwrap().0, swapped);
        assert!(app.world().get::<MeshMaterial3d<NifBlendMaterial>>(e).is_none());
    }

    #[test]
    fn published_shape_joins_script_fade_and_returns_to_blending_afterwards() {
        // [design] nvf_play fade.rs `start` skips a fade out of a reference that is not
        // going out (present, not held by a script's Disable); an unknown reference is
        // present, so the reference here is a placed one that starts disabled.
        let mut app = App::new();
        simulation_with(&mut app, disabled_reference());
        assert!(!app.world().resource::<nvf_play::sim::SimState>().present(42));
        app.init_resource::<Assets<NifMaterial>>().init_resource::<Assets<NifBlendMaterial>>()
            .init_resource::<Assets<Image>>().init_resource::<Variants>().init_resource::<Time<Virtual>>()
            .add_plugins(nvf_play::fade::FadePlugin).add_systems(PostUpdate, conversion_order(configured));
        publish(&mut app);
        let original = app.world_mut().resource_mut::<Assets<NifMaterial>>().add(NifMaterial { base: default(), extension: default() });
        let e = app.world_mut().spawn((BlendShape::new(original.clone()), MeshMaterial3d(original), nvf_play::RefVisual(42), Visibility::Inherited)).id();
        app.update();
        assert!(app.world().get::<MeshMaterial3d<NifBlendMaterial>>(e).is_some());
        app.world_mut().write_message(nvf_play::fade::RefFade { form_id: 42, fade_in: false });
        app.update();
        assert!(app.world().get::<nvf_play::fade::Fading>(e).is_some());
        assert!(app.world().get::<MeshMaterial3d<NifBlendMaterial>>(e).is_none());
        app.world_mut().resource_mut::<Time<Virtual>>().advance_by(std::time::Duration::from_secs(1));
        app.update();
        let handle = &app.world().get::<MeshMaterial3d<NifMaterial>>(e).unwrap().0;
        assert_eq!(app.world().resource::<Assets<NifMaterial>>().get(handle).unwrap().base.base_color.alpha(), 0.5);
        app.world_mut().resource_mut::<Time<Virtual>>().advance_by(std::time::Duration::from_secs(2));
        app.update();
        assert!(app.world().get::<nvf_play::fade::Fading>(e).is_none());
        assert!(app.world().get::<MeshMaterial3d<NifBlendMaterial>>(e).is_some());
        assert_eq!(app.world().get::<Visibility>(e), Some(&Visibility::Hidden));
    }

    #[test]
    fn finished_fade_in_restores_shadows_before_blending_again() {
        use bevy::light::NotShadowCaster;
        use nvf_assets::nif_material::SeeThroughNoShadow;
        let mut app = App::new();
        simulation(&mut app); // reference 42 unknown, so present: a fade in runs
        app.init_resource::<Assets<NifMaterial>>().init_resource::<Assets<NifBlendMaterial>>()
            .init_resource::<Assets<Image>>().init_resource::<Variants>().init_resource::<Time<Virtual>>()
            .add_plugins(nvf_play::fade::FadePlugin)
            .add_systems(PostUpdate, (nvf_assets::nif_material::see_through_shadow_system(), conversion_order(configured)));
        publish(&mut app);
        let original = app.world_mut().resource_mut::<Assets<NifMaterial>>().add(NifMaterial { base: default(), extension: default() });
        let e = app.world_mut().spawn((BlendShape::new(original.clone()), MeshMaterial3d(original.clone()), nvf_play::RefVisual(42), Visibility::Inherited)).id();
        app.update();
        assert!(app.world().get::<MeshMaterial3d<NifBlendMaterial>>(e).is_some());
        assert!(app.world().get::<NotShadowCaster>(e).is_none());
        app.world_mut().write_message(nvf_play::fade::RefFade { form_id: 42, fade_in: true });
        app.update();
        // The see-through copy drawn while fading casts no shadow.
        assert!(app.world().get::<nvf_play::fade::Fading>(e).is_some());
        assert!(app.world().get::<MeshMaterial3d<NifBlendMaterial>>(e).is_none());
        assert!(app.world().get::<SeeThroughNoShadow>(e).is_some() && app.world().get::<NotShadowCaster>(e).is_some());
        app.world_mut().resource_mut::<Time<Virtual>>().advance_by(std::time::Duration::from_secs(3));
        app.update();
        assert!(app.world().get::<nvf_play::fade::Fading>(e).is_none());
        assert!(app.world().get::<MeshMaterial3d<NifBlendMaterial>>(e).is_some(), "blending again after the fade");
        assert!(app.world().get::<MeshMaterial3d<NifMaterial>>(e).is_none());
        assert_eq!(app.world().get::<BlendShape>(e).unwrap().original, original);
        assert!(app.world().get::<SeeThroughNoShadow>(e).is_none(), "the fade's shadow marker is gone");
        assert!(app.world().get::<NotShadowCaster>(e).is_none(), "the restored mesh casts its shadow again");
        assert_eq!(app.world().get::<Visibility>(e), Some(&Visibility::Inherited));
        app.update(); // and it stays so
        assert!(app.world().get::<NotShadowCaster>(e).is_none());
    }

    #[test]
    fn scripted_nif_edit_in_postupdate_is_converted_in_the_same_frame() {
        // [design] nif_mirror's material edits run in PostUpdate (MirrorSystems) and
        // republish the edited source by command; the conversion follows them, so the
        // frame never ends with the ordinary material beside the blend one.
        #[derive(Resource)] struct Edit(Option<(Entity, Handle<NifMaterial>)>);
        fn edit(mut commands: Commands, mut pending: ResMut<Edit>, mut materials: ResMut<Assets<NifMaterial>>) {
            let Some((e, source)) = pending.0.take() else { return };
            materials.get_mut(&source).unwrap().base.perceptual_roughness = 0.27;
            commands.entity(e).insert(MeshMaterial3d(source));
        }
        let mut app = App::new();
        app.init_resource::<Assets<NifMaterial>>().init_resource::<Assets<NifBlendMaterial>>()
            .init_resource::<Assets<Image>>().init_resource::<Variants>().insert_resource(Edit(None))
            .add_systems(PostUpdate, (edit.in_set(crate::nif_mirror::MirrorSystems), conversion_order(configured)));
        publish(&mut app);
        let source = app.world_mut().resource_mut::<Assets<NifMaterial>>().add(NifMaterial { base: default(), extension: default() });
        let e = app.world_mut().spawn((BlendShape::new(source.clone()), MeshMaterial3d(source.clone()))).id();
        app.update();
        let before = app.world().get::<MeshMaterial3d<NifBlendMaterial>>(e).unwrap().0.clone();
        app.world_mut().resource_mut::<Edit>().0 = Some((e, source.clone()));
        app.update();
        assert!(app.world().get::<MeshMaterial3d<NifMaterial>>(e).is_none(), "no ordinary material left beside the blend one");
        let after = app.world().get::<MeshMaterial3d<NifBlendMaterial>>(e).unwrap().0.clone();
        assert_ne!(after, before);
        assert_eq!(app.world().resource::<Assets<NifBlendMaterial>>().get(&after).unwrap().base.perceptual_roughness, 0.27);
    }

    #[test]
    fn aid_parsing() {
        let config = |band: f32, variant, shape, resolution, shading| BlendConfig { band, variant, shape, resolution, shading, flat: band / 16.0, steep: band * 6.0 };
        use ContactShape::*;
        // Old short forms keep working, with the new defaults for what they leave out
        // (round 7: the height shape).
        assert_eq!(parse("surface").unwrap(), config(8.0, Variant::Surface, Height, 1024, Shading::Full));
        assert_eq!(parse(":selection").unwrap(), config(8.0, Variant::Selection, Height, 1024, Shading::Full));
        assert_eq!(parse("12:weight").unwrap().variant, Variant::Weight);
        assert_eq!(parse("24:normal").unwrap().variant, Variant::Normal);
        assert_eq!(parse("24:status").unwrap(), config(24.0, Variant::Status, Height, 1024, Shading::Full));
        // The settings row's look is the defaults, written out.
        assert_eq!(setting_config(), parse("8:surface:height:1024:full").unwrap());
        assert_eq!(DEFAULT_SHAPE, Height);
        // The comparison sheet's values (handoff, round 5).
        assert_eq!(parse("24:surface:height:512:albedo").unwrap(), config(24.0, Variant::Surface, Height, 512, Shading::Albedo));
        assert_eq!(parse("8:surface:height:512:albedo").unwrap(), config(8.0, Variant::Surface, Height, 512, Shading::Albedo));
        assert_eq!(parse("8:surface:hybrid:512:albedo").unwrap(), config(8.0, Variant::Surface, Hybrid, 512, Shading::Albedo));
        assert_eq!(parse("8:surface:hybrid:512:full").unwrap(), config(8.0, Variant::Surface, Hybrid, 512, Shading::Full));
        assert_eq!(parse("8:surface:hybrid:1024:full").unwrap(), config(8.0, Variant::Surface, Hybrid, 1024, Shading::Full));
        assert_eq!(parse("8:surface:hybrid:2048:full").unwrap().resolution, 2048);
        assert_eq!(parse("8:surface:penetration:1024:full").unwrap().shape, Penetration);
        for view in ["weight", "status", "selection", "normal"] {
            assert_eq!(parse(&format!("8:{view}:hybrid:1024:full")).unwrap(), config(8.0, Variant::parse(view).unwrap(), Hybrid, 1024, Shading::Full));
        }
        // Without a band; empty parts take their defaults.
        assert_eq!(parse("weight:penetration").unwrap(), config(8.0, Variant::Weight, Penetration, 1024, Shading::Full));
        assert_eq!(parse("surface::2048").unwrap(), config(8.0, Variant::Surface, Height, 2048, Shading::Full));
        assert_eq!(parse("6.5:surface:::albedo").unwrap(), config(6.5, Variant::Surface, Height, 1024, Shading::Albedo));
        for value in ["", "24", "0:surface", "-1:weight", "NaN:surface", "inf:surface", "a:surface", "24:bad", "24:surface:weight",
            "surface:hybrid:768", "surface:hybrid:1024:shiny", "surface:hybrid:1024:full:extra", "8:surface:hybrid:1024:full:", "8:8:surface", "surface:24"] {
            assert!(parse(value).is_err(), "{value}");
        }
        // Hybrid v1 keeps round 5's hybrid; its widths are the unused defaults.
        assert_eq!(parse("8:surface:hybrid-v1:1024:full").unwrap(), config(8.0, Variant::Surface, HybridV1, 1024, Shading::Full));
    }

    #[test]
    fn aid_widths_extend_the_value_only_at_its_end() {
        let widths = |v: &str| parse(v).map(|c| (c.flat, c.band, c.steep));
        // Omitted: band / 16 and 6 band; the default band gives 0.5 and 48.
        assert_eq!(widths("surface"), Ok((0.5, 8.0, 48.0)));
        assert_eq!(widths("24:surface"), Ok((1.5, 24.0, 144.0)));
        // The comparison set (plan round 6, step 4).
        for (value, steep) in [("8:surface:hybrid:1024:full:0.5,32", 32.0), ("8:surface:hybrid:1024:full:0.5,48", 48.0),
            ("8:surface:hybrid:1024:full:0.5,64", 64.0), ("8:weight:hybrid:1024:full:0.5,48", 48.0)] {
            assert_eq!(widths(value), Ok((0.5, 8.0, steep)), "{value}");
        }
        assert_eq!(widths("8:surface:hybrid:::0.25,40"), Ok((0.25, 8.0, 40.0)), "empty parts before the widths take their defaults");
        assert!(parse("8:surface::::0.25,40").is_err(), "round 7: an empty shape part is height, which takes no widths");
        assert_eq!(widths("8:surface:hybrid:1024:full:8,8"), Ok((8.0, 8.0, 8.0)), "equal widths are ordered");
        assert_eq!(parse("8:surface:hybrid:1024:full:0.5,32").unwrap().width_controls(), Vec4::new(0.5, 32.0, 0.0, 0.0));
        assert_eq!(parse("8:surface:hybrid:1024:full:0.5,32").unwrap().widths(), ContactWidths { flat: 0.5, band: 8.0, steep: 32.0 });
        for value in [
            "8:surface:hybrid:1024:full:", "8:surface:hybrid:1024:full:0.5", "8:surface:hybrid:1024:full:0.5,", "8:surface:hybrid:1024:full:,48",
            "8:surface:hybrid:1024:full:0.5,48,64", "8:surface:hybrid:1024:full:0.5;48", "8:surface:hybrid:1024:full:a,48",
            "8:surface:hybrid:1024:full:0,48", "8:surface:hybrid:1024:full:-0.5,48", "8:surface:hybrid:1024:full:9,48",
            "8:surface:hybrid:1024:full:0.5,4", "8:surface:hybrid:1024:full:NaN,48", "8:surface:hybrid:1024:full:0.5,inf",
            "8:surface:hybrid:1024:full:0.5,48:extra",
            // Only hybrid v2 takes widths.
            "8:surface:height:1024:full:0.5,48", "8:surface:penetration:1024:full:0.5,48", "8:surface:hybrid-v1:1024:full:0.5,48",
            // A derived hybrid width that is not finite (round 7's default `height` derives
            // none, so the shape is named).
            "3e38:surface:hybrid",
        ] {
            assert!(parse(value).is_err(), "{value}");
        }
        // The old shapes keep accepting what they accepted.
        assert!(parse("3e38:surface:height").is_ok());
        assert!(parse("3e38:surface").is_ok(), "the default height shape derives no widths");
    }

    #[test]
    fn start_and_ready_lines_keep_their_prefixes_and_name_every_setting() {
        let c = parse("24:surface").unwrap();
        assert_eq!(start_line(&c), "mesh terrain blend: start band=24 variant=surface resolution=1024 noise=0 shape=height shading=full weight-version=1");
        assert!(start_line(&c).starts_with("mesh terrain blend: start band=24 variant=surface resolution=1024 noise=0 shape=height shading=full"), "round 5's prefix");
        assert_eq!(start_line(&parse("24:surface:hybrid").unwrap()), "mesh terrain blend: start band=24 variant=surface resolution=1024 noise=0 shape=hybrid shading=full flat=1.5 steep=144 weight-version=2");
        assert_eq!(start_line(&setting_config()), "mesh terrain blend: start band=8 variant=surface resolution=1024 noise=0 shape=height shading=full weight-version=1", "the settings row's look");
        assert_eq!(start_line(&parse("8:status:penetration:2048:albedo").unwrap()), "mesh terrain blend: start band=8 variant=status resolution=2048 noise=0 shape=penetration shading=albedo weight-version=1");
        assert_eq!(start_line(&parse("8:surface:hybrid:1024:full:0.5,32").unwrap()), "mesh terrain blend: start band=8 variant=surface resolution=1024 noise=0 shape=hybrid shading=full flat=0.5 steep=32 weight-version=2");
        let mut images = Assets::<Image>::default();
        let surface = crate::terrain_surface::PublishedSurface { colour: images.add(Image::default()), height: images.add(Image::default()), origin: (0, 0), cells: 9, resolution: 512, revision: 3, coverage: 511, ..default() };
        let mut census = Census::default();
        census.count(BaseType::Stat, 0);
        census.count(BaseType::Stat, reason::RIG | reason::PARTICLES);
        census.count(BaseType::Scol, reason::MISSING_SOURCE);
        census.admit(&NifMaterial { base: StandardMaterial { alpha_mode: AlphaMode::Blend, depth_bias: 2.0, ..default() }, extension: NifSurface { decal: true, alpha_cut: 0.1, ..default() } });
        census.eligible = 12;
        let line = ready_line(&surface, 3, &parse("8:status:height:512:albedo").unwrap(), &census);
        let prefix = "mesh terrain blend: ready eligible-shapes=12 resident-cells=9 generation=3 variant=status shape=height resolution=512 shading=albedo";
        assert!(line.starts_with(&format!("{prefix} weight-version=1 stat-shapes=2 excluded-shapes=2 pending-material-shapes=1 excluded-world=0 excluded-rig=1 excluded-particles=1 excluded-posed=0")), "{line}");
        assert!(line.contains(" excluded-missing-source=1 excluded-temporary=0 excluded-alpha-mode=0"), "{line}");
        assert!(line.ends_with(" excluded-depth-bias-nonfinite=0 admitted-blend=1 admitted-bias=1 admitted-decal=1 admitted-mask=0 admitted-alpha-cut=1 scol-shapes=1"), "round 7: SCOL shapes last: {line}");
        for (_, name) in reason::NAMES { assert!(line.contains(&format!(" excluded-{name}=")), "every reason is printed, zero included: {name}"); }
        let v2 = ready_line(&surface, 3, &parse("8:surface:hybrid").unwrap(), &Census::default());
        assert!(v2.starts_with("mesh terrain blend: ready eligible-shapes=0 resident-cells=9 generation=3 variant=surface shape=hybrid resolution=512 shading=full flat=0.5 steep=48 weight-version=2 stat-shapes=0 "), "{v2}");
        assert_eq!(parse("6.5:normal:penetration").unwrap().controls(), Vec4::new(6.5, 2.0, 1.0, 1.0));
        assert_eq!(parse("6.5:normal:hybrid-v1").unwrap().controls().z, 3.0);
    }

    #[test]
    fn shape_lines_name_reference_base_model_shape_and_first_reason() {
        let t = BlendCensus { reference: 0x0010_2A3B, base: 0x0001_F00D, base_type: BaseType::Stat, model: "landscape/roads/road01.nif".into(), shape: 2, reasons: 0 };
        assert_eq!(shape_line(&t, 0), "mesh terrain blend: census-shape ref=0x00102A3B base=0x0001F00D model=landscape/roads/road01.nif shape=2 status=eligible first-reason=none reasons=none base-type=STAT");
        assert_eq!(shape_line(&t, reason::SOFT | reason::UNLIT | reason::DEST), "mesh terrain blend: census-shape ref=0x00102A3B base=0x0001F00D model=landscape/roads/road01.nif shape=2 status=excluded first-reason=dest reasons=dest+unlit+soft base-type=STAT");
        let scol = BlendCensus { base_type: BaseType::Scol, model: "scol/synthetic_road.nif".into(), ..t.clone() };
        assert!(shape_line(&scol, 0).ends_with(" model=scol/synthetic_road.nif shape=2 status=eligible first-reason=none reasons=none base-type=SCOL"));
        // Bounded: one line per base, shape and mask; pending shapes wait; a hard cap.
        let mut variants = Variants::default();
        log_shape(&mut variants, &t, 0);
        log_shape(&mut variants, &BlendCensus { reference: 7, ..t.clone() }, 0);
        log_shape(&mut variants, &t, reason::MISSING_SOURCE);
        assert_eq!(variants.shape_lines, 1);
        log_shape(&mut variants, &t, reason::TEMPORARY);
        assert_eq!(variants.shape_lines, 2);
        for shape in 0..SHAPE_LINES + 10 { log_shape(&mut variants, &BlendCensus { shape: shape + 10, ..t.clone() }, 0); }
        assert_eq!(variants.shape_lines, SHAPE_LINES + 1, "stops after the cap and its one notice");
        // Every name is distinct and the precedence is the bit order.
        let names: HashSet<&str> = reason::NAMES.iter().map(|(_, n)| *n).collect();
        assert_eq!(names.len(), reason::NAMES.len());
        for (i, (bit, _)) in reason::NAMES.iter().enumerate() { assert_eq!(*bit, 1 << i); }
        assert!(!reason::NAMES.iter().any(|(_, n)| n.contains("error") || n.contains("shader")), "census lines never match the smoke error filter");
    }

    #[test]
    fn selection_excludes_other_records_interiors_animation_destruction_and_lod() {
        let plain = ReferenceFacts { exterior: true, ..default() };
        assert!(eligible_reference(b"STAT", plain));
        // Round 7: a static collection is one prebuilt model of statics, drawn as a STAT is.
        assert!(eligible_reference(b"SCOL", plain));
        assert_eq!((BaseType::of(b"STAT"), BaseType::of(b"SCOL")), (Some(BaseType::Stat), Some(BaseType::Scol)));
        for sig in [b"MSTT", b"DOOR", b"TREE", b"MISC", b"NPC_", b"PWAT", b"ACTI", b"FURN"] {
            assert!(!eligible_reference(sig, plain));
            assert_eq!(BaseType::of(sig), None);
        }
        // Every reference reason on its own, then all at once (independent bits).
        let cases = [
            (ReferenceFacts { exterior: false, ..plain }, reason::WORLD),
            (ReferenceFacts { rigged: true, ..plain }, reason::RIG),
            (ReferenceFacts { particles: true, ..plain }, reason::PARTICLES),
            (ReferenceFacts { posed: true, ..plain }, reason::POSED),
            (ReferenceFacts { damage_stages: true, ..plain }, reason::DAMAGE_STAGES),
            (ReferenceFacts { destructible: true, ..plain }, reason::DEST),
            (ReferenceFacts { distant: true, ..plain }, reason::DISTANT),
        ];
        for (facts, bit) in cases {
            assert_eq!(reference_reasons(facts), bit, "{facts:?}");
            assert!(!eligible_reference(b"STAT", facts));
            assert!(!eligible_reference(b"SCOL", facts), "the same reference rules for a SCOL");
        }
        let all = ReferenceFacts { exterior: false, rigged: true, particles: true, posed: true, damage_stages: true, destructible: true, distant: true };
        assert_eq!(reference_reasons(all), cases.iter().fold(0, |m, (_, b)| m | b));
        assert_eq!(reason::first(reference_reasons(all)), "world");
        assert_eq!(shape_reasons(false, false), 0);
        assert_eq!(shape_reasons(true, false), reason::EMITTANCE);
        assert_eq!(shape_reasons(false, true), reason::MORPH);
        assert_eq!(shape_reasons(true, true), reason::EMITTANCE | reason::MORPH);
    }

    #[test]
    fn every_material_predicate_has_its_own_reason() {
        let m = |base: StandardMaterial, extension: NifSurface| NifMaterial { base, extension };
        let image = Assets::<Image>::default().reserve_handle();
        let cases: Vec<(NifMaterial, u32)> = vec![
            (m(StandardMaterial { alpha_mode: AlphaMode::Add, ..default() }, default()), reason::ALPHA_MODE),
            (m(StandardMaterial { alpha_mode: AlphaMode::Multiply, ..default() }, default()), reason::ALPHA_MODE),
            (m(StandardMaterial { alpha_mode: AlphaMode::Premultiplied, ..default() }, default()), reason::ALPHA_MODE),
            (m(StandardMaterial { alpha_mode: AlphaMode::AlphaToCoverage, ..default() }, default()), reason::ALPHA_MODE),
            (m(StandardMaterial { unlit: true, ..default() }, default()), reason::UNLIT),
            (m(default(), NifSurface { refract: true, ..default() }), reason::REFRACTION),
            (m(default(), NifSurface { first_person: true, ..default() }), reason::FIRST_PERSON),
            (m(default(), NifSurface { depth_write: true, ..default() }), reason::DEPTH_WRITE),
            (m(StandardMaterial { specular_transmission: 0.5, ..default() }, default()), reason::SPECULAR_TRANSMISSION),
            (m(StandardMaterial { diffuse_transmission: 0.5, ..default() }, default()), reason::DIFFUSE_TRANSMISSION),
            (m(default(), NifSurface { falloff: Vec4::new(1.0, 0.0, 1.0, 0.0), ..default() }), reason::FALLOFF),
            (m(default(), NifSurface { holes: Vec4::ZERO, ..default() }), reason::HOLES_VALUE),
            (m(default(), NifSurface { holes_texture: Some(image.clone()), ..default() }), reason::HOLES_TEXTURE),
            (m(default(), NifSurface { facegen_offset: Some(image.clone()), ..default() }), reason::FACEGEN),
            (m(default(), NifSurface { pipboy_screen: true, ..default() }), reason::PIPBOY),
            (m(default(), NifSurface { add_linear: true, ..default() }), reason::ADD_LINEAR),
            (m(default(), NifSurface { soft: Vec4::X, ..default() }), reason::SOFT),
            (m(StandardMaterial { depth_bias: f32::NAN, ..default() }, default()), reason::DEPTH_BIAS),
        ];
        for (material, bit) in &cases {
            assert_eq!(material_reasons(material), *bit, "{}", reason::list(*bit));
            assert!(!eligible_material(material));
        }
        // Overlapping reasons stay separate bits.
        let both = m(StandardMaterial { unlit: true, alpha_mode: AlphaMode::Add, ..default() }, NifSurface { soft: Vec4::X, holes_texture: Some(image), ..default() });
        assert_eq!(material_reasons(&both), reason::UNLIT | reason::ALPHA_MODE | reason::SOFT | reason::HOLES_TEXTURE);
        // The ownership and missing-source states join the mask.
        assert_eq!(shape_mask(reason::DEST, true, None), reason::DEST | reason::TEMPORARY | reason::MISSING_SOURCE);
        assert_eq!(shape_mask(0, false, Some(&m(default(), default()))), 0);
    }

    #[test]
    fn enabled_clone_preserves_source_and_alpha_coverage_descriptors() {
        let mut images = Assets::<Image>::default();
        let surface = crate::terrain_surface::PublishedSurface { colour: images.add(Image::default()), height: images.add(Image::default()), origin: (-2, -1), cells: 1, resolution: 512, revision: 1, coverage: 511, ..default() };
        // Round 6 admissions: Blend, a finite depth bias, the lit decal flag and alpha_cut,
        // alone and combined, beside Opaque and Mask with hard and soft edges.
        let mut sources = Vec::new();
        for alpha_mode in [AlphaMode::Opaque, AlphaMode::Mask(0.4), AlphaMode::Blend] {
            for (depth_bias, decal, alpha_cut, hard_edges, blended_cutout) in [(0.0, false, 0.0, false, None), (3.5, true, 0.2, true, Some(0.5)), (-1.25, false, 0.1, false, None), (0.0, true, 0.0, true, None)] {
                sources.push(NifMaterial {
                    base: StandardMaterial { alpha_mode, double_sided: true, cull_mode: None, depth_bias, base_color: Color::srgba(0.5, 0.4, 0.3, 0.7), ..default() },
                    extension: NifSurface { decal, alpha_cut, hard_edges, blended_cutout, ..default() },
                });
            }
        }
        for source in &sources {
            assert!(eligible_material(source), "{:?} {:?}", source.base.alpha_mode, source.extension.decal);
            let c = parse("24:surface:hybrid:1024:full:1,100").unwrap();
            let enabled = enabled_material(source, c, &surface);
            assert_eq!(source.base.alpha_mode, enabled.base.alpha_mode);
            assert_eq!(source.base.cull_mode, enabled.base.cull_mode);
            assert_eq!(source.base.double_sided, enabled.base.double_sided);
            assert_eq!(source.base.depth_bias.to_bits(), enabled.base.depth_bias.to_bits(), "the exact bias");
            assert_eq!(source.base.base_color, enabled.base.base_color);
            assert_eq!(source.extension.alpha_cut.to_bits(), enabled.extension.alpha_cut.to_bits());
            assert_eq!(source.extension.decal, enabled.extension.decal);
            assert_eq!(source.extension.hard_edges, enabled.extension.hard_edges);
            assert_eq!(source.extension.depth_write, enabled.extension.depth_write);
            assert_eq!(source.extension.blended_cutout, enabled.extension.blended_cutout);
            assert_eq!(source.extension.holes, enabled.extension.holes);
            assert!(enabled.extension.mesh_terrain_colour.is_some());
            assert_eq!(enabled.extension.mesh_terrain, c.controls());
            assert_eq!(enabled.extension.mesh_terrain_widths, Vec4::new(1.0, 100.0, 0.0, 0.0));
            assert_eq!(nvf_assets::nif_material::NifSurfaceKey::from(&source.extension), nvf_assets::nif_surface_blend::NifSurfaceBlendKey::from(&enabled.extension).0);
        }
        for alpha_mode in [AlphaMode::Add, AlphaMode::Multiply, AlphaMode::Premultiplied, AlphaMode::AlphaToCoverage] {
            assert!(!eligible_material(&NifMaterial { base: StandardMaterial { alpha_mode, ..default() }, extension: default() }));
        }
        for extension in [NifSurface { refract: true, ..default() }, NifSurface { first_person: true, ..default() }, NifSurface { holes: Vec4::ZERO, ..default() }, NifSurface { depth_write: true, ..default() }] {
            assert!(!eligible_material(&NifMaterial { base: default(), extension }));
        }
        assert!(!eligible_material(&NifMaterial { base: StandardMaterial { unlit: true, ..default() }, extension: NifSurface { decal: true, ..default() } }), "an unlit decal stays out");
    }

    /// An app with the conversion and nothing else; the cache exists but is not published.
    fn conversion_app() -> App {
        let mut app = App::new();
        app.init_resource::<Assets<NifMaterial>>().init_resource::<Assets<NifBlendMaterial>>()
            .init_resource::<Assets<Image>>().init_resource::<Variants>().init_resource::<SurfaceCache>()
            .add_systems(PostUpdate, conversion_order(configured));
        app
    }
    fn tracked(shape: usize, reasons: u32) -> BlendCensus {
        BlendCensus { reference: 0x42, base: 0x99, base_type: BaseType::Stat, model: "synthetic.nif".into(), shape, reasons }
    }
    fn reported(app: &App) -> Census {
        app.world().resource::<Variants>().reported_census.clone().expect("a ready line")
    }
    fn reason_count(census: &Census, bit: u32) -> usize {
        census.reasons[reason::NAMES.iter().position(|(b, _)| *b == bit).unwrap()]
    }
    fn settle(app: &mut App) {
        for _ in 0..REFRESH_FRAMES { app.update(); }
    }

    #[test]
    fn census_counts_tracked_shapes_through_publication_arrivals_changes_and_removal() {
        let mut app = conversion_app();
        let (add, opaque, missing) = {
            let mut m = app.world_mut().resource_mut::<Assets<NifMaterial>>();
            let add = m.add(NifMaterial { base: StandardMaterial { alpha_mode: AlphaMode::Add, ..default() }, extension: default() });
            let opaque = m.add(NifMaterial { base: default(), extension: default() });
            (add, opaque, m.reserve_handle())
        };
        // A rigged STAT shape (census only), an Add material, a material not loaded yet.
        let rigged = app.world_mut().spawn((tracked(0, reason::RIG), MeshMaterial3d(opaque.clone()))).id();
        let added = app.world_mut().spawn((tracked(1, 0), BlendShape::new(add.clone()), MeshMaterial3d(add))).id();
        let pending = app.world_mut().spawn((tracked(2, 0), BlendShape::new(missing.clone()), MeshMaterial3d(missing.clone()))).id();
        app.update();
        assert!(app.world().resource::<Variants>().reported.is_none(), "nothing is reported before a publication");
        // A publication with no eligible shape is still reported.
        publish(&mut app);
        app.update();
        assert_eq!(app.world().resource::<Variants>().reported, Some(7));
        let c = reported(&app);
        assert_eq!((c.stat_shapes, c.excluded, c.pending), (3, 3, 1));
        assert_eq!((reason_count(&c, reason::RIG), reason_count(&c, reason::ALPHA_MODE), reason_count(&c, reason::MISSING_SOURCE)), (1, 1, 1));
        assert_eq!(c.admitted, [0; 5]);
        for e in [rigged, added, pending] { assert!(app.world().get::<MeshMaterial3d<NifBlendMaterial>>(e).is_none()); }
        assert!(app.world().get::<BlendShape>(rigged).is_none(), "tracking never adds a BlendShape");
        // A late arrival converts at once; the census line refreshes after the interval.
        let late = app.world_mut().spawn((tracked(3, 0), BlendShape::new(opaque.clone()), MeshMaterial3d(opaque.clone()))).id();
        app.update();
        assert!(app.world().get::<MeshMaterial3d<NifBlendMaterial>>(late).is_some());
        assert_eq!(reported(&app).stat_shapes, 3, "rate-limited");
        settle(&mut app);
        assert_eq!((reported(&app).stat_shapes, reported(&app).excluded), (4, 3));
        // The pending material loads: it converts and leaves the pending count.
        app.world_mut().resource_mut::<Assets<NifMaterial>>().insert(missing.id(), NifMaterial { base: StandardMaterial { alpha_mode: AlphaMode::Blend, ..default() }, extension: default() }).unwrap();
        settle(&mut app);
        assert!(app.world().get::<MeshMaterial3d<NifBlendMaterial>>(pending).is_some(), "an authored Blend material is admitted");
        let c = reported(&app);
        assert_eq!((c.stat_shapes, c.excluded, c.pending, c.admitted[0]), (4, 2, 0, 1));
        // A source edited under the same handle: the converted shape goes back to it.
        app.world_mut().resource_mut::<Assets<NifMaterial>>().get_mut(&opaque).unwrap().base.unlit = true;
        app.update();
        assert!(app.world().get::<MeshMaterial3d<NifBlendMaterial>>(late).is_none());
        assert_eq!(app.world().get::<MeshMaterial3d<NifMaterial>>(late).unwrap().0, opaque);
        settle(&mut app);
        assert_eq!(reason_count(&reported(&app), reason::UNLIT), 2, "the rigged shape shares the source");
        // Removal leaves the census.
        app.world_mut().despawn(rigged);
        settle(&mut app);
        let c = reported(&app);
        assert_eq!((c.stat_shapes, reason_count(&c, reason::RIG)), (3, 0));
    }

    #[test]
    fn authored_blend_is_admitted_while_temporary_owners_are_not() {
        let mut app = conversion_app();
        publish(&mut app);
        let authored = app.world_mut().resource_mut::<Assets<NifMaterial>>().add(NifMaterial { base: StandardMaterial { alpha_mode: AlphaMode::Blend, ..default() }, extension: default() });
        let e = app.world_mut().spawn((tracked(0, 0), BlendShape::new(authored.clone()), MeshMaterial3d(authored.clone()))).id();
        app.update();
        let enabled = app.world().get::<MeshMaterial3d<NifBlendMaterial>>(e).unwrap().0.clone();
        assert_eq!(app.world().resource::<Assets<NifBlendMaterial>>().get(&enabled).unwrap().base.alpha_mode, AlphaMode::Blend);
        // An override's copy is a Blend material too; ownership, not alpha, keeps it out.
        let cloak = app.world_mut().resource_mut::<Assets<NifMaterial>>().add(NifMaterial { base: StandardMaterial { alpha_mode: AlphaMode::Blend, base_color: Color::srgba(1.0, 1.0, 1.0, 0.3), ..default() }, extension: default() });
        assert!(eligible_material(app.world().resource::<Assets<NifMaterial>>().get(&cloak).unwrap()));
        app.world_mut().entity_mut(e).insert((MeshMaterial3d(cloak.clone()), MaterialOverride { source: authored.clone(), copy: cloak.clone() }));
        app.update();
        assert!(app.world().get::<MeshMaterial3d<NifBlendMaterial>>(e).is_none());
        assert_eq!(app.world().get::<MeshMaterial3d<NifMaterial>>(e).unwrap().0, cloak);
        assert_eq!(reason_count(&reported(&app), reason::TEMPORARY), 0, "not refreshed yet");
        settle(&mut app);
        assert_eq!(reason_count(&reported(&app), reason::TEMPORARY), 1);
        // The owner restores the authored material: it blends again.
        app.world_mut().entity_mut(e).remove::<MaterialOverride>().insert(MeshMaterial3d(authored.clone()));
        app.update();
        assert!(app.world().get::<MeshMaterial3d<NifBlendMaterial>>(e).is_some());
        assert_eq!(app.world().get::<BlendShape>(e).unwrap().original, authored);
    }

    #[test]
    fn authored_blend_leaves_for_a_script_fade_and_returns_after_it() {
        // As published_shape_joins_script_fade_and_returns_to_blending_afterwards, with an
        // authored Blend source: the fade's copy is material-eligible, Fading keeps it out.
        let mut app = App::new();
        simulation_with(&mut app, disabled_reference());
        app.init_resource::<Assets<NifMaterial>>().init_resource::<Assets<NifBlendMaterial>>()
            .init_resource::<Assets<Image>>().init_resource::<Variants>().init_resource::<Time<Virtual>>()
            .add_plugins(nvf_play::fade::FadePlugin).add_systems(PostUpdate, conversion_order(configured));
        publish(&mut app);
        let authored = app.world_mut().resource_mut::<Assets<NifMaterial>>().add(NifMaterial { base: StandardMaterial { alpha_mode: AlphaMode::Blend, ..default() }, extension: default() });
        let e = app.world_mut().spawn((BlendShape::new(authored.clone()), MeshMaterial3d(authored.clone()), nvf_play::RefVisual(42), Visibility::Inherited)).id();
        app.update();
        assert!(app.world().get::<MeshMaterial3d<NifBlendMaterial>>(e).is_some());
        app.world_mut().write_message(nvf_play::fade::RefFade { form_id: 42, fade_in: false });
        app.update();
        let copy = app.world().get::<nvf_play::fade::Fading>(e).unwrap().copy().clone();
        assert!(eligible_material(app.world().resource::<Assets<NifMaterial>>().get(&copy).unwrap()), "alpha mode alone would admit the copy");
        assert!(app.world().get::<MeshMaterial3d<NifBlendMaterial>>(e).is_none());
        app.world_mut().resource_mut::<Time<Virtual>>().advance_by(std::time::Duration::from_secs(3));
        app.update();
        assert!(app.world().get::<nvf_play::fade::Fading>(e).is_none());
        assert!(app.world().get::<MeshMaterial3d<NifBlendMaterial>>(e).is_some());
        assert_eq!(app.world().get::<BlendShape>(e).unwrap().original, authored, "the fade restored the authored source");
    }

    #[test]
    fn blended_cutout_flips_rebuild_the_clone_in_the_same_frame() {
        use nvf_assets::nif_material::BlendedCutouts;
        let mut app = conversion_app();
        app.init_resource::<BlendedCutouts>().add_message::<AssetEvent<NifMaterial>>()
            .add_systems(PostUpdate, nvf_assets::nif_material::blended_cutout_system());
        publish(&mut app);
        let source = app.world_mut().resource_mut::<Assets<NifMaterial>>().add(NifMaterial {
            base: StandardMaterial { alpha_mode: AlphaMode::Blend, ..default() },
            extension: NifSurface { blended_cutout: Some(0.5), ..default() },
        });
        let e = app.world_mut().spawn((tracked(0, 0), BlendShape::new(source.clone()), MeshMaterial3d(source.clone()))).id();
        app.update();
        let drawn = |app: &App| {
            let handle = app.world().get::<MeshMaterial3d<NifBlendMaterial>>(e).unwrap().0.clone();
            (app.world().resource::<Assets<NifBlendMaterial>>().get(&handle).unwrap().base.alpha_mode, handle)
        };
        let (mode, blended) = drawn(&app);
        assert_eq!(mode, AlphaMode::Blend);
        for (on, want) in [(true, AlphaMode::Mask(0.5)), (false, AlphaMode::Blend)] {
            app.world_mut().insert_resource(BlendedCutouts(on));
            app.update();
            assert_eq!(app.world().resource::<Assets<NifMaterial>>().get(&source).unwrap().base.alpha_mode, want, "edited under the same handle");
            let (mode, handle) = drawn(&app);
            assert_eq!(mode, want, "the clone follows in the same frame");
            assert_ne!(handle, blended);
            assert!(app.world().get::<MeshMaterial3d<NifMaterial>>(e).is_none());
            assert_eq!(app.world().get::<BlendShape>(e).unwrap().original, source, "no clone nesting: the source stays the original");
        }
    }

    #[test]
    fn same_handle_see_through_edits_refresh_the_shadow_markers() {
        let mut app = conversion_app();
        app.add_systems(PostUpdate, nvf_assets::nif_material::see_through_shadow_system());
        publish(&mut app);
        let source = app.world_mut().resource_mut::<Assets<NifMaterial>>().add(NifMaterial { base: default(), extension: default() });
        let e = app.world_mut().spawn((tracked(0, 0), BlendShape::new(source.clone()), MeshMaterial3d(source.clone()))).id();
        app.update();
        assert!(app.world().get::<NotShadowCaster>(e).is_none());
        app.world_mut().resource_mut::<Assets<NifMaterial>>().get_mut(&source).unwrap().base.alpha_mode = AlphaMode::Blend;
        app.update();
        assert!(app.world().get::<MeshMaterial3d<NifBlendMaterial>>(e).is_some());
        assert!(app.world().get::<SeeThroughNoShadow>(e).is_some() && app.world().get::<NotShadowCaster>(e).is_some(), "a see-through source casts no shadow");
        app.world_mut().resource_mut::<Assets<NifMaterial>>().get_mut(&source).unwrap().base.alpha_mode = AlphaMode::Opaque;
        app.update();
        assert!(app.world().get::<SeeThroughNoShadow>(e).is_none() && app.world().get::<NotShadowCaster>(e).is_none(), "and casts again when opaque");
    }

    #[test]
    fn request_restarts_keep_enabled_material_on_every_intermediate_frame() {
        let mut app = conversion_app();
        publish(&mut app);
        let source = app.world_mut().resource_mut::<Assets<NifMaterial>>().add(NifMaterial { base: default(), extension: default() });
        let entity = app.world_mut().spawn((BlendShape::new(source.clone()), MeshMaterial3d(source))).id();
        app.update();
        let old = app.world().get::<BlendShape>(entity).unwrap().enabled.clone().unwrap();
        for _ in 0..8 {
            app.world_mut().resource_mut::<SurfaceCache>().generation += 1;
            app.update();
            assert_eq!(app.world().get::<BlendShape>(entity).unwrap().enabled.as_ref(), Some(&old));
            assert_eq!(&app.world().get::<MeshMaterial3d<NifBlendMaterial>>(entity).unwrap().0, &old);
            assert!(app.world().get::<MeshMaterial3d<NifMaterial>>(entity).is_none());
            assert_eq!(app.world().resource::<Assets<NifBlendMaterial>>().len(), 1);
        }
    }
    #[test]
    fn replacement_keeps_old_clone_and_images_until_prepared_acknowledgement() {
        let mut app = conversion_app();
        publish(&mut app);
        let source = app.world_mut().resource_mut::<Assets<NifMaterial>>().add(NifMaterial { base: default(), extension: default() });
        let entity = app.world_mut().spawn((BlendShape::new(source.clone()), MeshMaterial3d(source))).id();
        app.update();
        let old = app.world().get::<BlendShape>(entity).unwrap().enabled.clone().unwrap();
        let old_colour = app.world().resource::<Assets<NifBlendMaterial>>().get(&old).unwrap().extension.mesh_terrain_colour.clone().unwrap();
        let ready = BlendReadiness::default();
        ready.ready.lock().unwrap().insert(old.id());
        app.world_mut().resource_mut::<Variants>().readiness = Some(ready.clone());
        publish(&mut app);
        app.world_mut().resource_mut::<SurfaceCache>().published.as_mut().unwrap().revision = 8;
        let mut replacement = None;
        for _ in 0..6 {
            app.update();
            assert_eq!(app.world().get::<BlendShape>(entity).unwrap().enabled.as_ref(), Some(&old));
            let variants = app.world().resource::<Variants>();
            let next = variants.materials.values().next().unwrap().0.clone();
            assert_ne!(next, old);
            assert_eq!(*replacement.get_or_insert(next.clone()), next, "the prepared candidate is stable while waiting");
            assert_eq!(variants.publications.len(), 2, "old images retained through preparation");
            assert_eq!(app.world().resource::<Assets<NifBlendMaterial>>().get(&old).unwrap().extension.mesh_terrain_colour.as_ref(), Some(&old_colour));
            assert!(!ready.settled(8));
        }
        let next = replacement.unwrap();
        ready.ready.lock().unwrap().insert(next.id());
        app.update();
        assert_eq!(app.world().get::<BlendShape>(entity).unwrap().enabled.as_ref(), Some(&next));
        assert_eq!(app.world().resource::<Variants>().publications.len(), 1);
        assert!(ready.settled(8));
    }
    #[test]
    fn first_clone_same_handle_emissive_edit_rejects_superseded_acknowledgement() {
        let mut app = conversion_app();
        publish(&mut app);
        let ready = BlendReadiness::default();
        app.world_mut().resource_mut::<Variants>().readiness = Some(ready.clone());
        let source = app.world_mut().resource_mut::<Assets<NifMaterial>>().add(NifMaterial::default());
        let entity = app.world_mut().spawn((BlendShape::new(source.clone()), MeshMaterial3d(source.clone()))).id();
        app.update();
        let stale = app.world().resource::<Variants>().materials[&source.id()].0.clone();
        assert!(app.world().get::<BlendShape>(entity).unwrap().enabled.is_none());
        let emission = LinearRgba::rgb(0.7, 0.2, 0.1);
        let stamp = SourceStamp::of(app.world().resource::<Assets<NifMaterial>>().get(&source).unwrap());
        app.world_mut().resource_mut::<Assets<NifMaterial>>().get_mut(&source).unwrap().base.emissive = emission;
        assert_eq!(SourceStamp::of(app.world().resource::<Assets<NifMaterial>>().get(&source).unwrap()), stamp, "the component tick must catch this edit");
        // The mirror republishes its owned source under the same handle.
        app.world_mut().entity_mut(entity).insert(MeshMaterial3d(source.clone()));
        ready.ready.lock().unwrap().insert(stale.id());
        app.update();
        let next = app.world().resource::<Variants>().materials[&source.id()].0.clone();
        assert_ne!(next, stale);
        for _ in 0..4 {
            assert!(app.world().get::<BlendShape>(entity).unwrap().enabled.is_none());
            assert!(app.world().get::<MeshMaterial3d<NifBlendMaterial>>(entity).is_none());
            assert_eq!(app.world().get::<MeshMaterial3d<NifMaterial>>(entity).unwrap().0, source);
            assert_eq!(app.world().resource::<Assets<NifBlendMaterial>>().get(&next).unwrap().base.emissive, emission);
            app.update();
            assert_eq!(app.world().resource::<Variants>().materials[&source.id()].0, next);
        }
        ready.ready.lock().unwrap().insert(next.id());
        app.update();
        assert_eq!(app.world().get::<MeshMaterial3d<NifBlendMaterial>>(entity).unwrap().0, next);
        assert!(app.world().get::<MeshMaterial3d<NifMaterial>>(entity).is_none());
    }
    #[test]
    fn producer_replacement_while_preparing_keeps_one_drawable_material_and_new_source() {
        let mut app = conversion_app();
        publish(&mut app);
        let source = app.world_mut().resource_mut::<Assets<NifMaterial>>().add(NifMaterial { base: default(), extension: default() });
        let entity = app.world_mut().spawn((BlendShape::new(source.clone()), MeshMaterial3d(source))).id();
        app.update();
        let old = app.world().get::<BlendShape>(entity).unwrap().enabled.clone().unwrap();
        let ready = BlendReadiness::default();
        app.world_mut().resource_mut::<Variants>().readiness = Some(ready.clone());
        let new_source = app.world_mut().resource_mut::<Assets<NifMaterial>>().add(NifMaterial { base: StandardMaterial { perceptual_roughness: 0.3, ..default() }, extension: default() });
        app.world_mut().entity_mut(entity).insert(MeshMaterial3d(new_source.clone()));
        app.update();
        let next = app.world().resource::<Variants>().materials[&new_source.id()].0.clone();
        for _ in 0..4 {
            assert!(app.world().get::<MeshMaterial3d<NifMaterial>>(entity).is_none());
            assert_eq!(app.world().get::<BlendShape>(entity).unwrap().original, new_source);
            assert_eq!(app.world().get::<MeshMaterial3d<NifBlendMaterial>>(entity).unwrap().0, old);
            app.update();
            assert_eq!(app.world().resource::<Variants>().materials[&new_source.id()].0, next);
        }
        ready.ready.lock().unwrap().insert(next.id());
        app.update();
        assert_eq!(app.world().get::<MeshMaterial3d<NifBlendMaterial>>(entity).unwrap().0, next);
    }

    #[test]
    fn off_drops_preparing_candidates_and_on_requires_fresh_acknowledgement() {
        let _switching = switching();
        let mut app = App::new();
        switch_systems(&mut app, true);
        let ready = BlendReadiness::default();
        app.world_mut().resource_mut::<Variants>().readiness = Some(ready.clone());
        app.update();
        publish(&mut app);
        let source = app.world_mut().resource_mut::<Assets<NifMaterial>>().add(NifMaterial { base: default(), extension: default() });
        let entity = app.world_mut().spawn((BlendShape::new(source.clone()), MeshMaterial3d(source.clone()))).id();
        app.update();
        let stale = app.world().resource::<Variants>().materials.values().next().unwrap().0.clone();
        assert!(app.world().get::<BlendShape>(entity).unwrap().enabled.is_none());
        set_row(&mut app, false); app.update();
        assert!(app.world().resource::<Variants>().materials.is_empty());
        assert!(app.world().resource::<Variants>().publications.is_empty());
        assert_eq!(app.world().get::<MeshMaterial3d<NifMaterial>>(entity).unwrap().0, source);
        ready.ready.lock().unwrap().insert(stale.id());
        set_row(&mut app, true); app.update();
        publish(&mut app);
        // The normal census backfill would add BlendShape again in the live plugin.
        app.world_mut().entity_mut(entity).insert(BlendShape::new(source));
        app.update();
        let candidate = app.world().resource::<Variants>().materials.values().next().unwrap().0.clone();
        assert_ne!(candidate, stale);
        assert!(app.world().get::<BlendShape>(entity).unwrap().enabled.is_none());
    }

    #[test]
    fn the_aid_overrides_the_row_and_the_row_draws_the_default_look() {
        let aid = parse("24:weight:hybrid").unwrap();
        assert_eq!(desired(None, false), None, "off by default");
        assert_eq!(desired(None, true), Some(setting_config()));
        assert_eq!(desired(Some(aid), false), Some(aid), "the aid turns it on for comparison runs");
        assert_eq!(desired(Some(aid), true), Some(aid), "and wins over the row");
    }

    /// Tests that switch the blend serialise on this: [`RECORDING`] is process-wide.
    static SWITCHING: std::sync::Mutex<()> = std::sync::Mutex::new(());
    fn switching() -> std::sync::MutexGuard<'static, ()> {
        SWITCHING.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The plugin's switching schedule ([`MeshTerrainBlendPlugin`] without the world's
    /// capture and bake systems), with the row as given.
    fn switch_systems(app: &mut App, row: bool) {
        use crate::graphics::{Graphics, Preset};
        let mut graphics = Graphics::preset(Preset::Vanilla);
        graphics.mesh_terrain_blend = row;
        app.insert_resource(graphics).insert_resource(BlendGate(MaterialGate::new()))
            .init_resource::<Assets<NifMaterial>>().init_resource::<Assets<NifBlendMaterial>>().init_resource::<Assets<Image>>()
            .init_resource::<Assets<crate::terrain_surface::SurfaceCaptureMaterial>>()
            .init_resource::<Variants>().init_resource::<SurfaceCache>().init_resource::<BlendState>()
            .add_systems(Update, (follow_setting, close_gate.after(follow_setting).run_if(draining)))
            .add_systems(PostUpdate, conversion_order((stop.run_if(stopping), apply_variants.run_if(active)).chain().in_set(BlendConversion)));
    }
    fn set_row(app: &mut App, on: bool) {
        app.world_mut().resource_mut::<crate::graphics::Graphics>().mesh_terrain_blend = on;
    }
    fn gate_open(app: &App) -> bool {
        app.world().resource::<BlendGate>().0.is_open()
    }
    /// What the asset plugin's tracking does with the dropped clones (not in these apps).
    fn drop_unused_blends(app: &mut App) {
        let mut blends = app.world_mut().resource_mut::<Assets<NifBlendMaterial>>();
        let ids: Vec<_> = blends.ids().collect();
        for id in ids { blends.remove(id); }
    }

    /// #2661 round 7: the settings row switches the blend while the game runs. Off, no
    /// system converts, nothing is allocated and the material gate stays closed; on, the
    /// gate opens, the row's look starts and the conversion runs; off again, every
    /// converted shape gets its ordinary material back and the census, the landscape
    /// captures, the published atlas and the clones go; round 8: the gate closes once no
    /// blend material has been left for [`IDLE_FRAMES`] frames. (The test aid is not set in
    /// unit tests: `aid()` is `None`.)
    #[test]
    fn the_row_switches_the_blend_on_and_off_while_the_game_runs() {
        use crate::terrain_surface::{CapturePath, SurfaceCell};
        let _switching = switching();
        let mut app = App::new();
        switch_systems(&mut app, false);
        let source = app.world_mut().resource_mut::<Assets<NifMaterial>>().add(NifMaterial { base: default(), extension: default() });
        // Off: a published surface and a tracked shape left over would still not convert.
        publish(&mut app);
        let e = app.world_mut().spawn((tracked(0, 0), BlendShape::new(source.clone()), MeshMaterial3d(source.clone()))).id();
        app.update();
        assert_eq!(app.world().resource::<BlendState>().active, None);
        assert!(app.world().get::<MeshMaterial3d<NifBlendMaterial>>(e).is_none(), "nothing converts while off");
        assert!(app.world().resource::<Assets<NifBlendMaterial>>().is_empty(), "no clone is made while off");
        assert!(!gate_open(&app), "a run that never turns the blend on keeps its materials' gate closed");
        // On.
        set_row(&mut app, true);
        app.update();
        let state = app.world().resource::<BlendState>();
        assert_eq!(state.active, Some(setting_config()));
        assert!(state.shapes_pending, "the census of what is drawn is queued");
        assert!(recording(), "spawns record the census and captures now");
        assert!(gate_open(&app), "the gate opens before the first clone is made");
        assert_eq!(app.world().resource::<SurfaceCache>().resolution, 1024);
        publish(&mut app);
        let cell = app.world_mut().spawn(SurfaceCell { world: 0, grid: (0, 0), heights: vec![], quadrants: vec![], paths: [CapturePath::HeightOnly("missing-base-mesh"); 4], content_revision: 0, sources: vec![], dependencies: default() }).id();
        app.update();
        assert!(app.world().get::<MeshMaterial3d<NifBlendMaterial>>(e).is_some(), "the shape blends");
        assert!(app.world().get::<MeshMaterial3d<NifMaterial>>(e).is_none());
        let generation = app.world().resource::<SurfaceCache>().generation;
        // Off again.
        set_row(&mut app, false);
        app.update();
        assert!(!recording());
        assert_eq!(app.world().get::<MeshMaterial3d<NifMaterial>>(e).map(|m| m.0.clone()), Some(source.clone()), "its ordinary material is back");
        assert!(app.world().get::<MeshMaterial3d<NifBlendMaterial>>(e).is_none());
        assert!(app.world().get::<BlendShape>(e).is_none() && app.world().get::<BlendCensus>(e).is_none(), "the census goes");
        assert!(app.world().get::<SurfaceCell>(cell).is_none(), "the landscape captures go");
        let cache = app.world().resource::<SurfaceCache>();
        assert!(cache.published.is_none() && cache.generation > generation, "the atlas is dropped; an old receipt cannot publish");
        assert!(app.world().resource::<Variants>().materials.is_empty(), "the clones are dropped");
        assert_eq!(app.world().resource::<BlendState>().active, None);
        // The gate waits while a blend material is still in the store, then for IDLE_FRAMES.
        for _ in 0..IDLE_FRAMES + 2 { app.update(); }
        assert!(gate_open(&app), "a clone not yet dropped keeps the gate open");
        drop_unused_blends(&mut app);
        for _ in 0..IDLE_FRAMES - 1 { app.update(); }
        assert!(gate_open(&app));
        app.update();
        assert!(!gate_open(&app), "nothing of the blend left: the gate closes");
        assert_eq!(app.world().resource::<BlendState>().draining, None);
        // A stale publication while off converts nothing.
        publish(&mut app);
        app.world_mut().entity_mut(e).insert((tracked(0, 0), BlendShape::new(source.clone())));
        app.update();
        assert!(app.world().get::<MeshMaterial3d<NifBlendMaterial>>(e).is_none());
        assert!(app.world().resource::<Assets<NifBlendMaterial>>().is_empty());
        // Back on while draining keeps the gate open.
        app.world_mut().entity_mut(e).remove::<(BlendShape, BlendCensus)>();
        set_row(&mut app, true);
        app.update();
        set_row(&mut app, false);
        app.update();
        assert_eq!(app.world().resource::<BlendState>().draining, Some(0));
        set_row(&mut app, true);
        app.update();
        assert!(gate_open(&app) && app.world().resource::<BlendState>().draining.is_none(), "back on before it closed");
    }

    /// #2661 round 8: switching off in the frame a texture swap publishes its copy keeps the
    /// copy. The swap consumed its generation, so a restoration of the old source would
    /// lose it for good (no `BlendShape` is left to bring it back).
    #[test]
    fn switching_off_during_a_texture_swap_keeps_the_swap() {
        use fnv_game::functions::misc::TextureSwaps;
        #[derive(Resource, Default)] struct Swaps(TextureSwaps);
        let _switching = switching();
        let (mut app, _files) = crate::outfits::tests::asset_fixture();
        simulation(&mut app);
        switch_systems(&mut app, true);
        app.insert_resource(crate::outfits::tests::empty_world_data())
            .init_resource::<crate::texture_swap::Swapped>().init_resource::<Swaps>()
            .add_systems(Update, |swaps: Res<Swaps>, sim: Res<nvf_play::sim::SimState>, world: Res<crate::world::WorldData>, mut drawing: crate::texture_swap::Swapping| drawing.sync(&swaps.0, &sim, &world));
        publish(&mut app);
        let original = app.world_mut().resource_mut::<Assets<NifMaterial>>().add(NifMaterial { base: default(), extension: default() });
        let e = app.world_mut().spawn((tracked(0, 0), BlendShape::new(original.clone()), MeshMaterial3d(original.clone()), nvf_play::RefVisual(42), nvf_assets::actor::ShapeName("road".into()))).id();
        app.update();
        assert!(app.world().get::<MeshMaterial3d<NifBlendMaterial>>(e).is_some(), "converted");
        // Synthetic BC1 DDS through the production texture loader; no game data.
        let texture = "textures\\synthetic-blend-swap-off.dds";
        let mut dds = vec![0u8; 136]; dds[..4].copy_from_slice(b"DDS ");
        for (at, value) in [(4, 124u32), (8, 0x1007), (12, 4), (16, 4), (28, 1), (76, 32), (80, 4), (108, 0x1000)] { dds[at..at + 4].copy_from_slice(&value.to_le_bytes()); }
        dds[84..88].copy_from_slice(b"DXT1");
        app.world_mut().resource_mut::<nvf_assets::TextureCache>().offer(texture.into(), dds, 0.0);
        {
            let mut swaps = app.world_mut().resource_mut::<Swaps>();
            swaps.0.by_ref.insert(fnv_sim::FormId(42), vec![("road".into(), texture.into())]);
            swaps.0.generation = 1;
        }
        // The swap and the switch in the same frame.
        set_row(&mut app, false);
        app.update();
        let drawn = app.world().get::<MeshMaterial3d<NifMaterial>>(e).expect("an ordinary material").0.clone();
        assert_ne!(drawn, original, "the swap's copy stays, not the old source");
        assert!(app.world().resource::<Assets<NifMaterial>>().get(&drawn).unwrap().base.base_color_texture.is_some(), "the swapped texture is drawn");
        assert!(app.world().get::<MeshMaterial3d<NifBlendMaterial>>(e).is_none());
        assert!(app.world().get::<BlendShape>(e).is_none());
        // The swap consumed generation 1: nothing applies it again unless the copy stays.
        app.update();
        assert_eq!(app.world().get::<MeshMaterial3d<NifMaterial>>(e).unwrap().0, drawn, "and stays");
    }

    /// #2661 round 8: switching off in the frame a temporary owner, a script fade or a
    /// material override, puts its copy on keeps the copy; the owner restores the source
    /// itself.
    #[test]
    fn switching_off_during_a_fade_or_an_override_keeps_the_owners_copy() {
        let _switching = switching();
        // A fade out starting in the same frame.
        let mut app = App::new();
        simulation_with(&mut app, disabled_reference());
        switch_systems(&mut app, true);
        app.init_resource::<Time<Virtual>>().add_plugins(nvf_play::fade::FadePlugin);
        publish(&mut app);
        let original = app.world_mut().resource_mut::<Assets<NifMaterial>>().add(NifMaterial { base: default(), extension: default() });
        let e = app.world_mut().spawn((tracked(0, 0), BlendShape::new(original.clone()), MeshMaterial3d(original.clone()), nvf_play::RefVisual(42), Visibility::Inherited)).id();
        app.update();
        assert!(app.world().get::<MeshMaterial3d<NifBlendMaterial>>(e).is_some(), "converted");
        app.world_mut().write_message(nvf_play::fade::RefFade { form_id: 42, fade_in: false });
        set_row(&mut app, false);
        app.update();
        let copy = app.world().get::<nvf_play::fade::Fading>(e).expect("the fade runs").copy().clone();
        assert_eq!(app.world().get::<MeshMaterial3d<NifMaterial>>(e).map(|m| m.0.clone()), Some(copy), "the fade's copy is drawn");
        assert!(app.world().get::<MeshMaterial3d<NifBlendMaterial>>(e).is_none() && app.world().get::<BlendShape>(e).is_none());
        app.world_mut().resource_mut::<Time<Virtual>>().advance_by(std::time::Duration::from_secs(3));
        app.update();
        assert_eq!(app.world().get::<MeshMaterial3d<NifMaterial>>(e).map(|m| m.0.clone()), Some(original.clone()), "the fade restores the source");

        // An override published in the same frame (before the switch's PostUpdate).
        let mut app = App::new();
        switch_systems(&mut app, true);
        publish(&mut app);
        let source = app.world_mut().resource_mut::<Assets<NifMaterial>>().add(NifMaterial { base: default(), extension: default() });
        let e = app.world_mut().spawn((tracked(0, 0), BlendShape::new(source.clone()), MeshMaterial3d(source.clone()))).id();
        app.update();
        assert!(app.world().get::<MeshMaterial3d<NifBlendMaterial>>(e).is_some(), "converted");
        let cloak = app.world_mut().resource_mut::<Assets<NifMaterial>>().add(NifMaterial { base: StandardMaterial { alpha_mode: AlphaMode::Blend, ..default() }, extension: default() });
        app.world_mut().entity_mut(e).insert((MeshMaterial3d(cloak.clone()), MaterialOverride { source: source.clone(), copy: cloak.clone() }));
        set_row(&mut app, false);
        app.update();
        assert_eq!(app.world().get::<MeshMaterial3d<NifMaterial>>(e).map(|m| m.0.clone()), Some(cloak), "the override's copy stays");
        assert!(app.world().get::<MaterialOverride>(e).is_some());
        assert!(app.world().get::<MeshMaterial3d<NifBlendMaterial>>(e).is_none() && app.world().get::<BlendShape>(e).is_none());
    }
}
