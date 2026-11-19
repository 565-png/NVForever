//! Authored PBR material sets as materials use them (#2981, docs/spec-notes/pbr-materials.md).
//!
//! - The run's switch ([`enabled`]): the settings' Authored materials row, read once at the
//!   start ([`set_enabled`], nvforever `main.rs`) and fixed for the run, so cached models
//!   never mix the two states. Off, nothing here asks for the index (`GameData::pbr_index`)
//!   or reads a file under `textures\pbr\`.
//! - [`find`]: a shape's set by its resolved diffuse key ([`PbrSet`]: authored, or height
//!   only), the members loaded by the exact paths the index names, the scalar file read once
//!   per mount ([`scalars_cached`], `GameData::pbr_scalars`).
//! - [`apply`] / [`restore`]: a set on a material and back, from the legacy look every
//!   eligible shape's material keeps ([`LegacyLook`]); [`retexture`] is the one rule for a
//!   lit copy whose diffuse is replaced later.
//!
//! [design] The rules are NVForever's own (the format is ours); none describes the original.

use crate::data::GameData;
use crate::texture::TextureUsage;
use bevy::pbr::ParallaxMappingMethod;
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use fnv_pbr::{Found, Member, PbrIndex, Scalars, SetState};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex, OnceLock};

/// The run's state of the Authored materials setting (module notes).
static RUN: OnceLock<bool> = OnceLock::new();

/// Fix the run's state of the setting, before the first material is made. Called once at the
/// start; a later call that disagrees is refused with a warning (the state is the run's).
pub fn set_enabled(on: bool) {
    if RUN.set(on).is_err() && enabled() != on {
        warn!("authored materials: already {} for this run; the setting takes effect at the next start", if enabled() { "on" } else { "off" });
    }
}

/// Whether authored sets shape materials in this run: as [`set_enabled`] fixed it, else on
/// (the default; the viewer never sets it).
pub fn enabled() -> bool {
    *RUN.get_or_init(|| true)
}

/// An authored set found for a shape's diffuse, its maps loaded.
#[derive(Clone, Debug, PartialEq)]
pub struct AuthoredSet {
    /// The set's stem (`textures\pbr\...`, without suffix).
    pub stem: String,
    /// `_orm`: occlusion red, roughness green, metallic blue (linear).
    pub orm: Handle<Image>,
    /// `_n` in the two-channel role, when the set has one and it loaded.
    pub normal: Option<Handle<Image>>,
    /// `_p` as a depth map with its settings ([`Parallax::height`]), when the set has one, it
    /// loaded and its depth is above 0.
    pub height: Option<Parallax>,
    /// The scalar file's values (all `None` without one).
    pub scalars: Scalars,
}

/// A height-only set (`_p` without `_orm`) found for a shape's diffuse, its `_p` loaded: the
/// game's own material with the height map, nothing else changed [design,
/// docs/spec-notes/pbr-materials.md "Layout"].
#[derive(Clone, Debug, PartialEq)]
pub struct HeightSet {
    /// The set's stem (`textures\pbr\...`, without suffix).
    pub stem: String,
    /// `_p` as a depth map with its settings ([`Parallax::height`]).
    pub height: Parallax,
}

/// What [`find`] found for a shape's diffuse: the one active set the index's lookup names
/// (`PbrIndex::lookup`, with its first-person rule), by its state.
#[derive(Clone, Debug, PartialEq)]
pub enum PbrSet {
    /// A set with `_orm` (and its `_n`, `_p` and scalars where it has them).
    Authored(AuthoredSet),
    /// A set with `_p` and no `_orm`.
    HeightOnly(HeightSet),
}

impl PbrSet {
    pub fn stem(&self) -> &str {
        match self {
            PbrSet::Authored(s) => &s.stem,
            PbrSet::HeightOnly(s) => &s.stem,
        }
    }

    /// The authored part (`None` for a height-only set).
    pub fn authored(&self) -> Option<&AuthoredSet> {
        match self {
            PbrSet::Authored(s) => Some(s),
            PbrSet::HeightOnly(_) => None,
        }
    }

    /// The height map, from either kind of set.
    pub fn height(&self) -> Option<&Parallax> {
        match self {
            PbrSet::Authored(s) => s.height.as_ref(),
            PbrSet::HeightOnly(s) => Some(&s.height),
        }
    }
}

impl From<AuthoredSet> for PbrSet {
    fn from(set: AuthoredSet) -> PbrSet {
        PbrSet::Authored(set)
    }
}

/// The parallax method a set's height map is drawn with: Bevy's occlusion method (a march
/// through layers, then one interpolation) [design, the stage 4 review's starting point;
/// relief costs a binary search on top].
pub const PARALLAX_METHOD: ParallaxMappingMethod = ParallaxMappingMethod::Occlusion;

/// The most layers the march takes (Bevy's default is 16; at a head-on view it takes one)
/// [design, the review's starting point; no frame-time budget is measured for it].
pub const MAX_PARALLAX_LAYERS: f32 = 8.0;

/// The parallax fields of a material, as Bevy's standard material has them: the depth map,
/// its depth, the method and the most layers. What a set's `_p` puts on a material
/// ([`Parallax::height`]) and what the legacy look keeps to put back ([`LegacyLook::parallax`]).
#[derive(Clone, Debug, PartialEq, Reflect)]
pub struct Parallax {
    pub depth_map: Option<Handle<Image>>,
    pub depth_scale: f32,
    pub method: ParallaxMappingMethod,
    pub max_layers: f32,
}

impl Default for Parallax {
    /// The fields of a material without a depth map (Bevy's defaults).
    fn default() -> Parallax {
        Parallax::of(&StandardMaterial::default())
    }
}

impl Parallax {
    /// A set's height map: `image` (the `_p`, linear) as the depth map at `depth_scale` UV
    /// units, [`PARALLAX_METHOD`], [`MAX_PARALLAX_LAYERS`].
    pub fn height(image: Handle<Image>, depth_scale: f32) -> Parallax {
        Parallax { depth_map: Some(image), depth_scale, method: PARALLAX_METHOD, max_layers: MAX_PARALLAX_LAYERS }
    }

    /// The fields as `material` has them.
    pub fn of(material: &StandardMaterial) -> Parallax {
        Parallax {
            depth_map: material.depth_map.clone(),
            depth_scale: material.parallax_depth_scale,
            method: material.parallax_mapping_method,
            max_layers: material.max_parallax_layer_count,
        }
    }

    /// Put the fields on `material`.
    pub fn put(&self, material: &mut StandardMaterial) {
        material.depth_map = self.depth_map.clone();
        material.parallax_depth_scale = self.depth_scale;
        material.parallax_mapping_method = self.method;
        material.max_parallax_layer_count = self.max_layers;
    }
}

/// The files [`find`] and the decoders already warned about (one warning per file).
#[derive(Default)]
pub struct Memo {
    warned: Mutex<HashSet<String>>,
}

/// The scalar files of one mount by path, each read once ([`scalars_cached`]); kept beside the
/// mount's index (`GameData`), so a new mount starts empty.
pub type ScalarCache = Mutex<HashMap<String, Arc<OnceLock<Scalars>>>>;

/// The scalars of the file at `path`, from `cache`: the first caller reads (`read`), parses
/// and logs its warnings while any other caller for the same path waits on that path's cell,
/// so a file is read and warned about once; other paths are not held up.
pub(crate) fn scalars_cached(cache: &ScalarCache, path: &str, read: impl FnOnce() -> Option<Vec<u8>>) -> Scalars {
    let cell = cache.lock().expect("authored scalars").entry(path.to_string()).or_default().clone();
    *cell.get_or_init(|| match read() {
        Some(bytes) => {
            let file = fnv_pbr::parse_scalars(&bytes);
            for w in &file.warnings {
                warn!("authored materials: {path}: {w}");
            }
            file.scalars
        }
        None => {
            warn!("authored materials: {path} could not be read; the defaults apply");
            Scalars::default()
        }
    })
}

impl Memo {
    /// Log `message` the first time `path` is warned about.
    pub(crate) fn warn_once(&self, path: &str, message: impl FnOnce() -> String) {
        if self.warned.lock().expect("authored warnings").insert(path.to_string()) {
            warn!("{}", message());
        }
    }
}

/// The authored set (state [`SetState::Authored`]) for the resolved diffuse `key`: the one
/// active set the index finds (`PbrIndex::lookup`, with its first-person rule), if it is
/// authored. A height-only set means "no authored set" for the shape's ORM part, also when
/// it is a first-person twin's own set: the twin's set wins the lookup and the plain
/// texture's set is not used behind it ([`find`] gives the twin's height only) [design: the
/// first-person rule of docs/spec-notes/pbr-materials.md "Layout"].
pub fn authored_found(index: &PbrIndex, key: &str) -> Option<Found> {
    index.lookup(key).filter(|f| f.state() == SetState::Authored)
}

/// A member loaded by [`find`]'s caller: its handle and, where the caller knows it, the
/// format it went up in.
pub(crate) type Loaded = (Handle<Image>, Option<TextureFormat>);

/// The set for the resolved diffuse `key` with its maps loaded by `load` (path as the index
/// names it, exactly: never resolved again, so a `\xbox\` member stays the one listed; role),
/// or `None`: the setting off (`enabled`; nothing is asked of the index then), no active
/// set, an authored set whose `_orm` does not load (one warning per file; the shape keeps the
/// game's material), or a height-only set without a height map to draw ([`height_map`]). An
/// `_n` that does not load leaves the game's normal map in use; an authored set's `_p` that
/// gives no height map leaves the rest of the set in use.
pub(crate) fn find(data: &GameData, enabled: bool, key: &str, memo: &Memo, mut load: impl FnMut(&str, TextureUsage) -> Option<Loaded>) -> Option<PbrSet> {
    if !enabled {
        return None;
    }
    let found = data.pbr_index().lookup(key)?;
    let scalars = |found: &Found| found.path(Member::Scalars).map(|path| data.pbr_scalars(&path)).unwrap_or_default();
    match found.state() {
        SetState::Authored => {
            let orm_path = found.path(Member::Orm)?;
            let Some((orm, _)) = load(&orm_path, TextureUsage::Linear) else {
                Stats::add(&stats().orm_failed);
                memo.warn_once(&orm_path, || format!("authored materials: {orm_path} could not be loaded; surfaces drawn with {key} keep the game's material"));
                return None;
            };
            let normal = found.path(Member::Normal).and_then(|path| {
                let h = load(&path, TextureUsage::NormalTwoChannel).map(|(h, _)| h);
                if h.is_none() {
                    memo.warn_once(&path, || format!("authored materials: {path} could not be loaded; the game's normal map is used"));
                }
                h
            });
            let scalars = scalars(&found);
            let height = height_map(&found, &scalars, memo, &mut load);
            Some(PbrSet::Authored(AuthoredSet { stem: found.stem, orm, normal, height, scalars }))
        }
        SetState::HeightOnly => {
            // Of the scalars only the depth applies to a height-only set.
            let height = height_map(&found, &scalars(&found), memo, &mut load)?;
            Some(PbrSet::HeightOnly(HeightSet { stem: found.stem, height }))
        }
        SetState::Incomplete => None,
    }
}

/// The set's `_p` as a height map at the scalars' depth ([`Scalars::parallax_depth_or_default`]),
/// or `None`: no `_p`, a depth of 0 (the file is then not loaded), a `_p` that does not
/// load, or a signed one ([`signed_format`]; refused, as `fnvtool pbr check` refuses it), one
/// warning per file either way.
fn height_map(found: &Found, scalars: &Scalars, memo: &Memo, load: &mut impl FnMut(&str, TextureUsage) -> Option<Loaded>) -> Option<Parallax> {
    let path = found.path(Member::Height)?;
    let depth = scalars.parallax_depth_or_default();
    if depth <= 0.0 {
        return None;
    }
    let Some((image, format)) = load(&path, TextureUsage::Linear) else {
        Stats::add(&stats().height_failed);
        memo.warn_once(&path, || format!("authored materials: {path} could not be loaded; no height map is drawn from it"));
        return None;
    };
    if format.is_some_and(signed_format) {
        Stats::add(&stats().height_signed);
        memo.warn_once(&path, || format!("authored materials: {path} is signed BC4; a height map is unsigned (0 low, 1 high), so it is not drawn (export it as BC4_UNORM)"));
        return None;
    }
    Some(Parallax::height(image, depth))
}

/// Whether a member went up in a signed format (`_SNORM`: values -1..1). A height map holds
/// 0 (lowest) to 1 (highest); the parallax march reads `1 - height` as depth, which a negative
/// sample would push below the deepest layer, so a signed `_p` is refused [design, the
/// format's rule: docs/spec-notes/pbr-materials.md "Compression and sizes"].
pub fn signed_format(format: TextureFormat) -> bool {
    matches!(format, TextureFormat::Bc4RSnorm | TextureFormat::R8Snorm | TextureFormat::Bc5RgSnorm | TextureFormat::Rg8Snorm)
}

/// The value of `NifSurface::emissive_rule.w` that marks a material shaded from an authored
/// set (0 on every other material; the lane is otherwise unused, and survives the mesh
/// terrain blend's conversion).
pub const AUTHORED_MARK: f32 = 1.0;

/// What a lit shape's material is without a set (its "legacy look"), kept on the material
/// (`NifSurface::legacy`) for every shape that may take a set, so a later change of its
/// diffuse (a script's texture swap, the blackjack cards, the Vigor Tester's digits) can put
/// the new diffuse's set on it or go back to the game's values ([`retexture`]). Changed
/// after the material is made only by a script's glossiness edit ([`set_legacy_roughness`]);
/// copies carry it along.
#[derive(Clone, Debug, PartialEq, Reflect)]
pub struct LegacyLook {
    /// The game's normal map in use (after the tangent check).
    pub normal: Option<Handle<Image>>,
    /// The specular mask (the normal map, when its alpha is one: `nif_spawn::specular`).
    pub specular: Option<Handle<Image>>,
    pub roughness: f32,
    pub reflectance: f32,
    pub metallic: f32,
    /// The drawn mesh has tangents, so a set's `_n` can be applied to it even where the game
    /// gives no normal map.
    pub tangents: bool,
    /// The specular tint (Bevy multiplies the reflectance by it): white as built, a script's
    /// specular colour once one is set ([`set_legacy_specular_tint`]).
    pub specular_tint: Color,
    /// The parallax fields as built (no depth map on any game material today; the game's
    /// own height slot would set them, upstream issue #2979: [`game_height_map`]).
    pub parallax: Parallax,
}

/// Where the game's own height map would give a material its baseline parallax (the
/// texture set's slot 3 with shader flag bit 11, upstream issue #2979, which wants an
/// observation of the original first): this branch reads neither and returns `None` always,
/// so [`LegacyLook::parallax`] stays Bevy's no-depth-map defaults. The one place that source
/// would plug in, beside the authored `_p` [design]. Unused.
pub fn game_height_map(_height_slot: Option<&str>, _parallax_flag: bool) -> Option<Parallax> {
    None
}

/// Whether a shape may take an authored set, by what is known when its material is made:
/// lit, not refracting, not hair or skin (FaceGen), drawn neither additive nor multiplied
/// (the final alpha mode), with texture coordinates, and not excluded by its caller
/// (`no_authored`: particles, object LOD) [design, docs/spec-notes/pbr-materials.md "What a
/// set changes"]. The key is checked by [`key_eligible`].
pub fn shape_eligible(unlit: bool, refracts: bool, hair: bool, skin: bool, alpha_mode: AlphaMode, has_uv: bool, no_authored: bool) -> bool {
    !unlit && !refracts && !hair && !skin && has_uv && !no_authored && !matches!(alpha_mode, AlphaMode::Add | AlphaMode::Multiply)
}

/// Whether a resolved diffuse key may take a set: not the Pip-Boy's or the interface's
/// textures [design].
pub fn key_eligible(key: &str) -> bool {
    !key.starts_with("textures\\pipboy3000\\") && !key.starts_with("textures\\interface\\")
}

/// The metallic value an authored material is built with: 0 (the ORM's blue channel then has
/// no effect: no reflection source exists yet, docs/spec-notes/pbr-materials.md "Metallic"),
/// or 1 in the metallic channel view ([`PbrView::Metallic`]), so the material's sample of
/// the blue channel is the per-texel value the view shows, and under the metallic debug
/// switch ([`metallic_debug`]), so the blue channel takes effect.
pub fn authored_metallic() -> f32 {
    if view() == PbrView::Metallic || metallic_debug() { 1.0 } else { 0.0 }
}

/// The environment variable of the metallic debug switch ([`metallic_debug`]).
pub const METALLIC_ENV: &str = "NVF_PBR_METALLIC";

/// The metallic debug switch (a calibration aid, off by default; `NVF_PBR_METALLIC=1`, read
/// once at the start): authored materials are built with metallic 1, so the ORM's blue
/// channel takes effect, and nif_material.wesl's removal of Bevy's ambient highlight keeps
/// the metal's share of it on authored materials ((1 - m) of it is removed, m the sampled
/// metallic), so a metal shows the ambient light in its colour instead of drawing black where
/// no lamp's highlight falls (`NVF_PBR_METALLIC` shader def) [design, DESIGN option (b)].
/// Every other material is drawn as without it.
pub fn metallic_debug() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| {
        let on = std::env::var(METALLIC_ENV).is_ok_and(|v| v.trim() == "1");
        if on {
            info!("authored materials: metallic debug switch ({METALLIC_ENV}=1): authored materials are metallic by their ORM's blue channel and keep a metal's ambient highlight");
        }
        on
    })
}

/// The environment variable that picks a channel view ([`PbrView`]).
pub const VIEW_ENV: &str = "NVF_PBR_VIEW";

/// A debug view of the material inputs of NIF surfaces (a calibration aid for authored sets,
/// off by default): the lit colour of forward-drawn lit NIF surfaces is replaced by one
/// input, read just after the standard material's sampling (nif_material.wesl,
/// "NVF_PBR_VIEW"). Refracting shapes return before it; additive draws add it by their alpha
/// and multiplied ones stay neutral, so effects fade out of the view. Picked once at
/// the start by `NVF_PBR_VIEW` = `roughness` | `metallic` | `occlusion` | `normal` |
/// `authored` | `height` [design]. Not a `NVF_PLAY_TEST_*` aid: it does not make a run
/// unattended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PbrView {
    Off,
    /// Perceptual roughness after the scale (the material's value × the ORM's green).
    Roughness,
    /// The ORM's blue channel (authored materials are built with metallic 1 in this view).
    Metallic,
    /// The ORM's red channel, sampled on its own (no screen-space occlusion); 1 without one.
    Occlusion,
    /// The world-space normal as 0.5 + 0.5 N.
    Normal,
    /// Authored shapes tinted orange, every other shape grey, by its base colour's brightness.
    Authored,
    /// The height map as stored (0 black = low, 1 white = high), read at the unshifted
    /// texture coordinates: the original height, not the depth the march reads; dark blue on
    /// a shape without a depth map (#2981 stage 4).
    Height,
}

impl PbrView {
    pub const ALL: [PbrView; 6] = [PbrView::Roughness, PbrView::Metallic, PbrView::Occlusion, PbrView::Normal, PbrView::Authored, PbrView::Height];

    pub fn name(self) -> &'static str {
        match self {
            PbrView::Off => "off",
            PbrView::Roughness => "roughness",
            PbrView::Metallic => "metallic",
            PbrView::Occlusion => "occlusion",
            PbrView::Normal => "normal",
            PbrView::Authored => "authored",
            PbrView::Height => "height",
        }
    }

    /// The view named `text` (any case; empty or `off`: none); `None` for another word.
    pub fn parse(text: &str) -> Option<PbrView> {
        let t = text.trim().to_ascii_lowercase();
        if t.is_empty() || t == "off" {
            return Some(PbrView::Off);
        }
        PbrView::ALL.into_iter().find(|v| v.name() == t)
    }

    /// The fragment shader's defs for the view: `NVF_PBR_VIEW` and the mode's own.
    pub fn shader_defs(self) -> &'static [&'static str] {
        match self {
            PbrView::Off => &[],
            PbrView::Roughness => &["NVF_PBR_VIEW", "NVF_PBR_VIEW_ROUGHNESS"],
            PbrView::Metallic => &["NVF_PBR_VIEW", "NVF_PBR_VIEW_METALLIC"],
            PbrView::Occlusion => &["NVF_PBR_VIEW", "NVF_PBR_VIEW_OCCLUSION"],
            PbrView::Normal => &["NVF_PBR_VIEW", "NVF_PBR_VIEW_NORMAL"],
            PbrView::Authored => &["NVF_PBR_VIEW", "NVF_PBR_VIEW_AUTHORED"],
            PbrView::Height => &["NVF_PBR_VIEW", "NVF_PBR_VIEW_HEIGHT"],
        }
    }
}

/// The run's channel view, read from [`VIEW_ENV`] the first time (the NIF material plugin asks
/// at the start) and fixed for the run.
pub fn view() -> PbrView {
    static VIEW: OnceLock<PbrView> = OnceLock::new();
    *VIEW.get_or_init(|| {
        let Some(text) = std::env::var_os(VIEW_ENV) else { return PbrView::Off };
        let text = text.to_string_lossy();
        match PbrView::parse(&text) {
            Some(PbrView::Off) => PbrView::Off,
            Some(v) => {
                info!("authored materials: channel view {} ({VIEW_ENV}): NIF surfaces show that input instead of their lit colour", v.name());
                v
            }
            None => {
                warn!("authored materials: {VIEW_ENV}={text} is none of roughness, metallic, occlusion, normal, authored, height; no channel view");
                PbrView::Off
            }
        }
    })
}

/// Put `set` on `material` from its legacy look: the ORM as metallic-roughness and occlusion
/// map (one image in both slots), no specular mask, the scalars' roughness scale and
/// reflectance (their defaults without a file), [`authored_metallic`], the set's `_n` when it
/// has one and the drawn mesh has tangents (else the game's normal map, its alpha no longer a
/// mask), and the mark ([`AUTHORED_MARK`]). Everything else (base colour, alpha, glow,
/// decal, the extension's other values) stays. Whether the set's `_n` was applied.
pub fn apply(material: &mut crate::NifMaterial, set: &AuthoredSet, legacy: &LegacyLook) -> bool {
    let b = &mut material.base;
    b.metallic_roughness_texture = Some(set.orm.clone());
    b.occlusion_texture = Some(set.orm.clone());
    b.specular_texture = None;
    b.perceptual_roughness = set.scalars.roughness_scale_or_default();
    b.reflectance = set.scalars.reflectance_or_default();
    b.metallic = authored_metallic();
    // A set's highlight strength is its reflectance alone, never scaled by a stored or
    // scripted specular colour of the game's (#2981).
    b.specular_tint = Color::WHITE;
    let normal = set.normal.clone().filter(|_| legacy.tangents);
    if normal.is_none() && set.normal.is_some() {
        Stats::add(&stats().normal_without_tangents);
        no_tangents_once(&set.stem);
    }
    let applied = normal.is_some();
    b.normal_map_texture = normal.or_else(|| legacy.normal.clone());
    material.extension.emissive_rule.w = AUTHORED_MARK;
    applied
}

/// Put the legacy look back on `material` (the inverse of [`apply`], and of
/// [`apply_height`]: the parallax fields go back too).
pub fn restore(material: &mut crate::NifMaterial, legacy: &LegacyLook) {
    let b = &mut material.base;
    b.metallic_roughness_texture = None;
    b.occlusion_texture = None;
    b.specular_texture = legacy.specular.clone();
    b.normal_map_texture = legacy.normal.clone();
    b.perceptual_roughness = legacy.roughness;
    b.reflectance = legacy.reflectance;
    b.metallic = legacy.metallic;
    b.specular_tint = legacy.specular_tint;
    legacy.parallax.put(b);
    material.extension.emissive_rule.w = 0.0;
}

/// Why a set's height map is not drawn on a material ([`height_gate`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeightSkip {
    /// Alpha-tested or blended (cut-outs, fades, holes made it `Mask`).
    NotOpaque,
    DoubleSided,
    Decal,
    /// The drawn mesh has no tangents (a copy of a drawn mesh, a swap onto one).
    NoTangents,
    /// Unlit, refracting, or eaten by an effect shader's holes.
    Other,
}

/// Whether `material` may draw a height map now, its drawn mesh having tangents (`tangents`):
/// opaque, single-sided, not a decal, lit, not refracting, without holes [design, the stage 4
/// review A3]. Why: the depth prepass, the shadow passes and the holes test read alpha at the
/// unshifted texture coordinates, so a cut-out's or a blend's coverage would not follow the
/// shifted texels; the march takes the incoming tangent frame without turning it for back
/// faces; decals already lean on a depth bias. Asked when a material is built, when a swap
/// puts a set on it ([`retexture`]) and when a copy changes how it draws ([`recheck_height`]).
pub fn height_gate(material: &crate::NifMaterial, tangents: bool) -> Result<(), HeightSkip> {
    let (b, e) = (&material.base, &material.extension);
    if b.unlit || e.refract || (e.holes.x >= 0.0 && e.holes.x < 255.0) {
        Err(HeightSkip::Other)
    } else if !matches!(b.alpha_mode, AlphaMode::Opaque) {
        Err(HeightSkip::NotOpaque)
    } else if b.double_sided {
        Err(HeightSkip::DoubleSided)
    } else if e.decal {
        Err(HeightSkip::Decal)
    } else if !tangents {
        Err(HeightSkip::NoTangents)
    } else {
        Ok(())
    }
}

/// Put a set's height map on `material` (the depth map, its depth, method and layers) when
/// [`height_gate`] passes with the legacy look's tangents; nothing else changes (a height-only
/// set leaves the material the game's own, the authored mark clear). Counts a skip by its
/// reason. Whether it was put on.
pub fn apply_height(material: &mut crate::NifMaterial, height: &Parallax, legacy: &LegacyLook) -> Result<(), HeightSkip> {
    match height_gate(material, legacy.tangents) {
        Ok(()) => {
            height.put(&mut material.base);
            Ok(())
        }
        Err(why) => {
            Stats::add(match why {
                HeightSkip::NotOpaque => &stats().height_not_opaque,
                HeightSkip::DoubleSided => &stats().height_double_sided,
                HeightSkip::Decal => &stats().height_decal,
                HeightSkip::NoTangents => &stats().height_no_tangents,
                HeightSkip::Other => &stats().height_other,
            });
            Err(why)
        }
    }
}

/// Whether a set's height map is on `material`: its depth map is not the legacy look's
/// (tracked apart from [`is_authored`]: a height-only set leaves the mark clear).
pub fn height_on(material: &crate::NifMaterial, legacy: &LegacyLook) -> bool {
    material.base.depth_map.is_some() && material.base.depth_map != legacy.parallax.depth_map
}

/// Take whatever set is on `material` off: the authored fields and the height map
/// ([`restore`]) when it is authored, else the height map alone (a height-only set), back to
/// the legacy look; a material wearing neither is left as it is.
pub fn take_off(material: &mut crate::NifMaterial, legacy: &LegacyLook) {
    if is_authored(material) {
        restore(material, legacy);
    } else if height_on(material, legacy) {
        legacy.parallax.put(&mut material.base);
    }
}

/// A copy of a material whose drawing changed after it was made (an effect shader's holes
/// making an opaque copy `Mask(0)`, a fade making it `Blend`: nvforever `effect_shaders.rs`,
/// nvf-play `fade.rs`): if it carries a depth map that [`height_gate`] no longer allows, the
/// depth map comes off and the legacy look's other parallax fields go back. Whether it took
/// one off.
pub fn recheck_height(material: &mut crate::NifMaterial) -> bool {
    if material.base.depth_map.is_none() || height_gate(material, true).is_ok() {
        return false;
    }
    let baseline = material.extension.legacy.as_ref().map(|l| l.parallax.clone()).unwrap_or_default();
    Parallax { depth_map: None, ..baseline }.put(&mut material.base);
    Stats::add(&stats().height_dropped);
    true
}

/// A script's change of a shape's glossiness (nvforever `nif_mirror.rs`, the material
/// property mirror) as perceptual roughness: the legacy look takes it, so the game's value is
/// the one restored later, and the drawn roughness takes it only while no set is on the
/// material; a set's roughness (its scale on the ORM's green) stays in force meanwhile
/// [design].
pub fn set_legacy_roughness(material: &mut crate::NifMaterial, roughness: f32) {
    if let Some(legacy) = material.extension.legacy.as_mut() {
        legacy.roughness = roughness;
    }
    if !is_authored(material) {
        material.base.perceptual_roughness = roughness;
    }
}

/// A script's specular colour (nvforever `nif_mirror.rs`): the legacy look takes it, so it is
/// the one restored later; the drawn tint takes it when no set is on the material, or when
/// the colour is what the script changed (`changed`). The mirror writes every value back
/// after any one change, so an edit of another property leaves a set's highlight (Bevy draws
/// `specular_tint x reflectance`) as it is [design].
pub fn set_legacy_specular_tint(material: &mut crate::NifMaterial, tint: Color, changed: bool) {
    if let Some(legacy) = material.extension.legacy.as_mut() {
        legacy.specular_tint = tint;
    }
    if changed || !is_authored(material) {
        material.base.specular_tint = tint;
    }
}

/// Whether any shape of `data`'s mount can take a set: the run's setting on (`enabled`) and
/// at least one active set (authored or height only) in the mount's index (built here if it
/// was not).
/// Callers that make shapes legacy for good once their role is known (nvforever `actors.rs`)
/// skip that work when it is false: no set can reach such a shape in this mount, through a
/// material build or a later swap alike. A new mount (`GameData::set_vfs`) would need those
/// materials made again; no production path mounts again (only tests call `set_vfs`).
pub fn mount_has_sets(data: &GameData) -> bool {
    mount_has_sets_when(data, enabled())
}

fn mount_has_sets_when(data: &GameData, on: bool) -> bool {
    on && data.pbr_index().counts().active() > 0
}

/// Whether `material` is shaded from an authored set.
pub fn is_authored(material: &crate::NifMaterial) -> bool {
    material.extension.emissive_rule.w == AUTHORED_MARK
}

/// Make `material` the game's own for good: its legacy look put back and its baseline
/// dropped, so no later swap gives it a set. For shapes found to be hair or a FaceGen tint
/// target only after their material was made (nvforever `actors.rs`, `facegen.rs`). Whether
/// anything changed.
pub fn make_legacy(material: &mut crate::NifMaterial) -> bool {
    let Some(legacy) = material.extension.legacy.take() else { return false };
    take_off(material, &legacy);
    Stats::add(&stats().made_legacy);
    true
}

/// Clear every authored map from an unlit copy (the Pip-Boy's glare and menus copies,
/// nvforever `pipboy_arm.rs`): the ORM in both slots, the depth map (Bevy applies parallax
/// before its unlit test) with the legacy look's other parallax fields put back, the mark and
/// the legacy look, so nothing of a set reaches it whatever its source wore [design, a second
/// guard beside the `textures\pipboy3000\` exclusion].
pub fn clear_for_unlit_copy(material: &mut crate::NifMaterial) {
    material.base.metallic_roughness_texture = None;
    material.base.occlusion_texture = None;
    let baseline = material.extension.legacy.take().map(|l| l.parallax).unwrap_or_default();
    Parallax { depth_map: None, ..baseline }.put(&mut material.base);
    material.extension.emissive_rule.w = 0.0;
}

/// What [`retexture`] did to a material.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Retextured {
    /// The material takes no set (no legacy look kept: not eligible, or made legacy).
    Untouched,
    /// The new diffuse's set is on it.
    Authored,
    /// The new diffuse's set is height only: the legacy look with its height map, where
    /// [`height_gate`] allows one.
    HeightOnly,
    /// No set for the new diffuse: the legacy look is on it (as it was, or put back).
    Legacy,
}

/// The one rule for a lit copy of a shape's material whose diffuse was just replaced by
/// `texture` (a NIF texture path; script `SwapTexture`, the blackjack cards, the Vigor
/// Tester's buttons and digits): whatever set the copy wore comes off first ([`take_off`]:
/// authored fields and height map alike), then, from the material's legacy look
/// ([`LegacyLook`], carried by the copy), the new diffuse's set is applied (its scalars'
/// defaults, the game's normal map where it has no `_n` or the drawn mesh has no tangents;
/// its height map where [`height_gate`] allows one with the drawn mesh's recorded tangents),
/// or a height-only set's height map alone, or nothing. Always from the legacy look, never
/// from what the copy wore: a copy of a material that wore another diffuse's set takes none
/// of it. Only the set's fields change; tint, animation, fade and every other value stay.
pub fn retexture(
    material: &mut crate::NifMaterial,
    texture: &str,
    textures: &mut crate::TextureCache,
    data: &GameData,
    images: &mut Assets<Image>,
    formats: Option<&bevy::image::CompressedImageFormatSupport>,
) -> Retextured {
    use crate::nif_spawn::TextureSource;
    let Some(legacy) = material.extension.legacy.clone() else { return Retextured::Untouched };
    let mut source = crate::nif_spawn::StoreTextures { textures, data, images, formats };
    let key = source.resolve(texture);
    let set = key_eligible(&key).then(|| source.pbr_set(&key)).flatten();
    take_off(material, &legacy);
    match set {
        Some(PbrSet::Authored(set)) => {
            apply(material, &set, &legacy);
            if let Some(height) = &set.height {
                let _ = apply_height(material, height, &legacy);
            }
            Stats::add(&stats().swaps_authored);
            Retextured::Authored
        }
        Some(PbrSet::HeightOnly(set)) => {
            let _ = apply_height(material, &set.height, &legacy);
            Stats::add(&stats().swaps_height);
            Retextured::HeightOnly
        }
        None => {
            Stats::add(&stats().swaps_legacy);
            Retextured::Legacy
        }
    }
}

/// What authored sets did this run, for the exit summary ([`report`]): counts of events, not
/// a census of visible shapes (a material rebuilt or discarded counts again). Relaxed
/// atomics; a set of stems only where a set is applied.
#[derive(Default)]
pub struct Stats {
    /// Material builds that applied a set (`nif_spawn::shape_material`).
    pub built: AtomicU64,
    /// Shapes the run's setting would serve that may take no set (unlit, refracting, hair,
    /// skin, additive or multiplied, no UVs, particles, landscape LOD).
    pub not_eligible: AtomicU64,
    /// Shapes whose diffuse is under `textures\pipboy3000\` or `textures\interface\`.
    pub excluded_texture: AtomicU64,
    /// Shapes whose set's `_orm` did not load (they stay the game's).
    pub orm_failed: AtomicU64,
    /// Sets put on a material without their `_n`, the drawn mesh having no tangents.
    pub normal_without_tangents: AtomicU64,
    /// Copies made the game's own after their material was built (a role found later:
    /// [`make_legacy`] calls that dropped a legacy look).
    pub made_legacy: AtomicU64,
    /// [`retexture`] calls on materials with a legacy look that put a set on, and that left or
    /// put back the legacy look (cached swaps do not call it again).
    pub swaps_authored: AtomicU64,
    pub swaps_legacy: AtomicU64,
    /// [`retexture`] calls that put a height-only set on.
    pub swaps_height: AtomicU64,
    /// Material builds that put a height map on (`nif_spawn::shape_material`), and of them
    /// those from a height-only set.
    pub height_built: AtomicU64,
    pub height_only_built: AtomicU64,
    /// Height maps a build or a swap did not put on, by [`HeightSkip`] reason.
    pub height_not_opaque: AtomicU64,
    pub height_double_sided: AtomicU64,
    pub height_decal: AtomicU64,
    pub height_no_tangents: AtomicU64,
    pub height_other: AtomicU64,
    /// Sets whose `_p` did not load (the rest of an authored set still applies).
    pub height_failed: AtomicU64,
    /// Sets whose `_p` is signed BC4, refused ([`signed_format`]).
    pub height_signed: AtomicU64,
    /// Later copies that dropped a height map ([`recheck_height`]).
    pub height_dropped: AtomicU64,
    stems: Mutex<HashSet<String>>,
}

impl Stats {
    fn add(counter: &AtomicU64) {
        counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    fn get(counter: &AtomicU64) -> u64 {
        counter.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// The summary line (without its prefix).
    pub fn line(&self) -> String {
        format!(
            "{} material builds applied a set ({} distinct set stems at build); builds without one: {} not eligible, {} on excluded textures, {} whose _orm did not load; {} applied without their _n (no tangents); {} role copies made legacy; {} retexture calls ({} to a set, {} to a height-only set, {} to the legacy look); height maps: {} material builds applied one ({} from height-only sets); not put on at build or swap: {} not opaque, {} double-sided, {} decals, {} without tangents, {} unlit, refracting or holed; {} _p not loaded, {} signed _p refused; {} later copies dropped one",
            Self::get(&self.built),
            self.stems.lock().expect("authored stats").len(),
            Self::get(&self.not_eligible),
            Self::get(&self.excluded_texture),
            Self::get(&self.orm_failed),
            Self::get(&self.normal_without_tangents),
            Self::get(&self.made_legacy),
            Self::get(&self.swaps_authored) + Self::get(&self.swaps_height) + Self::get(&self.swaps_legacy),
            Self::get(&self.swaps_authored),
            Self::get(&self.swaps_height),
            Self::get(&self.swaps_legacy),
            Self::get(&self.height_built),
            Self::get(&self.height_only_built),
            Self::get(&self.height_not_opaque),
            Self::get(&self.height_double_sided),
            Self::get(&self.height_decal),
            Self::get(&self.height_no_tangents),
            Self::get(&self.height_other),
            Self::get(&self.height_failed),
            Self::get(&self.height_signed),
            Self::get(&self.height_dropped),
        )
    }
}

/// The run's counts ([`Stats`]).
pub fn stats() -> &'static Stats {
    static STATS: OnceLock<Stats> = OnceLock::new();
    STATS.get_or_init(Stats::default)
}

/// A shape the setting would serve skipped by [`shape_eligible`] (`texture`: by
/// [`key_eligible`] instead).
pub(crate) fn note_skipped(texture: bool) {
    Stats::add(if texture { &stats().excluded_texture } else { &stats().not_eligible });
}

/// A material built with a height map on (`height_only`: from a height-only set).
pub(crate) fn note_height_built(height_only: bool) {
    Stats::add(&stats().height_built);
    if height_only {
        Stats::add(&stats().height_only_built);
    }
}

/// A material built with `set`.
pub(crate) fn note_built(set: &AuthoredSet) {
    Stats::add(&stats().built);
    let mut stems = stats().stems.lock().expect("authored stats");
    if !stems.contains(&set.stem) {
        stems.insert(set.stem.clone());
    }
}

/// The exit summary (Last, on the app's exit, as the other systems' summary lines): one line,
/// `authored materials summary: ...`, or that the setting was off.
pub fn report(mut exit: MessageReader<AppExit>) {
    if exit.read().last().is_none() {
        return;
    }
    if enabled() {
        info!("authored materials summary: {}", stats().line());
    } else {
        info!("authored materials summary: off (setting)");
    }
}

/// Log once per set that its `_n` was not applied to a shape without tangents.
fn no_tangents_once(stem: &str) {
    static SEEN: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    if SEEN.get_or_init(Default::default).lock().expect("authored tangents").insert(stem.to_string()) {
        info!("authored materials: {stem}_n.dds not applied where the drawn mesh has no tangents (the game's normal state is kept)");
    }
}

/// The warning for a linear member whose DX10 header names an `_SRGB` format (once per file:
/// the decode of a (path, role) happens once).
pub(crate) fn srgb_warning(memo: &Memo, path: &str, usage: TextureUsage, srgb_code: bool) {
    if srgb_code && usage != TextureUsage::Color && path.starts_with(fnv_pbr::PBR_DIR) {
        memo.warn_once(path, || format!("authored materials: {path} names an sRGB format in its DX10 header; it is read as linear data, as its role is"));
    }
}

#[cfg(test)]
pub(crate) mod tests;
