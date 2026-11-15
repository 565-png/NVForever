//! [design] NVForever's aid-only mesh/landscape colour blend: rigid exterior STATs,
//! a 24-unit default contact band, no noise, and immutable source materials.

use crate::terrain_surface::{SurfaceCache, update_cache};
use bevy::prelude::*;
use nvf_assets::nif_material::{NifMaterial, Falloff};
use nvf_assets::nif_surface_blend::{NifBlendMaterial, NifSurfaceBlend};
use std::{collections::HashMap, sync::OnceLock};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Variant { Surface, Weight, Normal, Selection }

impl Variant {
    fn name(self) -> &'static str {
        match self { Self::Surface => "surface", Self::Weight => "weight", Self::Normal => "normal", Self::Selection => "selection" }
    }
    fn number(self) -> f32 {
        match self { Self::Surface => 0.0, Self::Weight => 1.0, Self::Normal => 2.0, Self::Selection => 3.0 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlendConfig { pub band: f32, pub variant: Variant }

pub fn parse(value: &str) -> Result<BlendConfig, &'static str> {
    let (band, variant) = match value.split_once(':') {
        Some(("", variant)) => (24.0, variant),
        Some((band, variant)) => (band.parse::<f32>().map_err(|_| "band must be a finite positive number")?, variant),
        None => (24.0, value),
    };
    if !band.is_finite() || band <= 0.0 { return Err("band must be a finite positive number"); }
    let variant = match variant {
        "surface" => Variant::Surface, "weight" => Variant::Weight,
        "normal" => Variant::Normal, "selection" => Variant::Selection,
        _ => return Err("expected [positive band]:surface|weight|normal|selection"),
    };
    Ok(BlendConfig { band, variant })
}

pub fn config() -> Option<BlendConfig> {
    static CONFIG: OnceLock<Option<BlendConfig>> = OnceLock::new();
    *CONFIG.get_or_init(|| {
        let value = match std::env::var("NVF_PLAY_TEST_MESH_BLEND") {
            Ok(value) => value,
            Err(std::env::VarError::NotPresent) => return None,
            Err(_) => { warn!("mesh terrain blend: rejected NVF_PLAY_TEST_MESH_BLEND: value is not Unicode"); return None; }
        };
        match parse(&value) {
            Ok(c) => {
                info!("mesh terrain blend: start band={} variant={} resolution={} noise=0", c.band, c.variant.name(), crate::terrain_surface::RESOLUTION);
                if c.variant == Variant::Normal {
                    info!("mesh terrain blend: normal not yet implemented (round 2)");
                    None
                } else { Some(c) }
            }
            Err(e) => { warn!("mesh terrain blend: rejected NVF_PLAY_TEST_MESH_BLEND={value:?}: {e}"); None }
        }
    })
}

// [design] Selection rules, not asset-path guesses. Callers supply code facts from
// world.rs::spawn_model, fnv-world placed.rs/index.rs and model.rs::PreparedModel.
pub fn eligible_reference(sig: &[u8; 4], exterior: bool, rigid: bool, destructible: bool, distant: bool) -> bool {
    sig == b"STAT" && exterior && rigid && !destructible && !distant
}

pub fn eligible_material(m: &NifMaterial) -> bool {
    let s = &m.extension;
    matches!(m.base.alpha_mode, AlphaMode::Opaque | AlphaMode::Mask(_))
        && !m.base.unlit && !s.refract && !s.decal && !s.first_person && !s.depth_write
        && m.base.specular_transmission == 0.0 && m.base.diffuse_transmission == 0.0
        && s.falloff == Falloff::NONE.params && s.alpha_cut == 0.0
        && s.holes == nvf_assets::nif_material::NO_HOLES && s.holes_texture.is_none()
        && s.facegen_offset.is_none() && !s.pipboy_screen && !s.add_linear
        && s.soft == Vec4::ZERO && m.base.depth_bias == 0.0
}

#[derive(Component)]
pub struct BlendShape {
    pub original: Handle<NifMaterial>,
    enabled: Option<Handle<NifBlendMaterial>>,
}

impl BlendShape {
    pub fn new(original: Handle<NifMaterial>) -> Self { Self { original, enabled: None } }
}

#[derive(Resource, Default)]
struct Variants {
    generation: u64,
    materials: HashMap<AssetId<NifMaterial>, Handle<NifBlendMaterial>>,
    reported: Option<u64>,
}

pub struct MeshTerrainBlendPlugin;

impl Plugin for MeshTerrainBlendPlugin {
    fn build(&self, app: &mut App) {
        if config().is_none() { return; }
        app.add_plugins((MaterialPlugin::<NifBlendMaterial>::default(), crate::terrain_surface::SurfaceBakePlugin))
            .init_resource::<SurfaceCache>().init_resource::<Variants>()
            .add_systems(Update, (update_cache, apply_variants).chain().after(crate::world::stream));
    }
}

fn enabled_material(source: &NifMaterial, c: BlendConfig, surface: &crate::terrain_surface::PublishedSurface) -> NifBlendMaterial {
    NifBlendMaterial {
        base: source.base.clone(),
        extension: NifSurfaceBlend::from_source(&source.extension, c.band, c.variant.number(),
            Vec4::new(surface.origin.0 as f32, surface.origin.1 as f32, 0.0, 0.0), surface.colour.clone(), surface.height.clone()),
    }
}

fn apply_variants(
    mut commands: Commands,
    cache: Res<SurfaceCache>,
    mut variants: ResMut<Variants>,
    materials: Res<Assets<NifMaterial>>,
    mut blends: ResMut<Assets<NifBlendMaterial>>,
    mut shapes: Query<(Entity, &mut BlendShape, Option<&MeshMaterial3d<NifMaterial>>)>,
) {
    let Some(c) = config() else { return };
    if variants.generation != cache.generation {
        variants.generation = cache.generation;
        variants.materials.clear();
    }
    let mut eligible = 0;
    for (entity, mut shape, drawn) in &mut shapes {
        // [design] Respect a material override, retaining its actual source handle too.
        if let Some(drawn) = drawn {
            shape.original = drawn.0.clone();
        }
        let allowed = materials.get(&shape.original).is_some_and(eligible_material);
        eligible += usize::from(allowed);
        let Some(surface) = cache.published.as_ref().filter(|_| allowed) else {
            if shape.enabled.is_some() {
                commands.entity(entity).remove::<MeshMaterial3d<NifBlendMaterial>>().insert(MeshMaterial3d(shape.original.clone()));
            }
            shape.enabled = None;
            continue;
        };
        let source = shape.original.id();
        let handle = if let Some(handle) = variants.materials.get(&source) { handle.clone() } else {
            let enabled = enabled_material(materials.get(source).unwrap(), c, surface);
            let handle = blends.add(enabled);
            variants.materials.insert(source, handle.clone());
            handle
        };
        if shape.enabled.as_ref() != Some(&handle) || drawn.is_some() {
            commands.entity(entity).remove::<MeshMaterial3d<NifMaterial>>().insert(MeshMaterial3d(handle.clone()));
        }
        shape.enabled = Some(handle);
    }
    if let Some(surface) = &cache.published {
        if eligible > 0 && variants.reported != Some(cache.generation) {
            info!("mesh terrain blend: ready eligible-shapes={eligible} resident-cells={} generation={}", surface.cells, cache.generation);
            variants.reported = Some(cache.generation);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nvf_assets::nif_material::NifSurface;

    #[test]
    fn aid_parsing() {
        assert_eq!(parse("surface").unwrap(), BlendConfig { band: 24.0, variant: Variant::Surface });
        assert_eq!(parse(":selection").unwrap().band, 24.0);
        assert_eq!(parse("12:weight").unwrap().variant, Variant::Weight);
        assert_eq!(parse("24:normal").unwrap().variant, Variant::Normal);
        for value in ["", "24", "0:surface", "-1:weight", "NaN:surface", "inf:surface", "a:surface", "24:bad", "24:surface:weight"] {
            assert!(parse(value).is_err(), "{value}");
        }
    }

    #[test]
    fn selection_excludes_other_records_interiors_animation_destruction_and_lod() {
        assert!(eligible_reference(b"STAT", true, true, false, false));
        for sig in [b"MSTT", b"SCOL", b"DOOR", b"TREE", b"MISC", b"NPC_", b"PWAT"] {
            assert!(!eligible_reference(sig, true, true, false, false));
        }
        for (exterior, rigid, destructible, distant) in [(false, true, false, false), (true, false, false, false), (true, true, true, false), (true, true, false, true)] {
            assert!(!eligible_reference(b"STAT", exterior, rigid, destructible, distant));
        }
    }

    #[test]
    fn enabled_clone_preserves_source_and_alpha_coverage_descriptors() {
        let mut images = Assets::<Image>::default();
        let surface = crate::terrain_surface::PublishedSurface { colour: images.add(Image::default()), height: images.add(Image::default()), origin: (-2, -1), cells: 1 };
        for alpha_mode in [AlphaMode::Opaque, AlphaMode::Mask(0.4)] {
            let source = NifMaterial { base: StandardMaterial { alpha_mode, double_sided: true, cull_mode: None, ..default() }, extension: NifSurface::default() };
            assert!(eligible_material(&source));
            let enabled = enabled_material(&source, parse("24:surface").unwrap(), &surface);
            assert_eq!(source.base.alpha_mode, enabled.base.alpha_mode);
            assert_eq!(source.base.cull_mode, enabled.base.cull_mode);
            assert_eq!(source.base.double_sided, enabled.base.double_sided);
            assert_eq!(source.base.depth_bias, enabled.base.depth_bias);
            assert_eq!(source.extension.alpha_cut, enabled.extension.alpha_cut);
            assert_eq!(source.extension.holes, enabled.extension.holes);
            assert!(enabled.extension.mesh_terrain_colour.is_some());
            assert_eq!(nvf_assets::nif_material::NifSurfaceKey::from(&source.extension), nvf_assets::nif_surface_blend::NifSurfaceBlendKey::from(&enabled.extension).0);
        }
        for alpha_mode in [AlphaMode::Blend, AlphaMode::Add, AlphaMode::Multiply, AlphaMode::AlphaToCoverage] {
            assert!(!eligible_material(&NifMaterial { base: StandardMaterial { alpha_mode, ..default() }, extension: default() }));
        }
        for extension in [NifSurface { refract: true, ..default() }, NifSurface { decal: true, ..default() }, NifSurface { first_person: true, ..default() }, NifSurface { holes: Vec4::ZERO, ..default() }] {
            assert!(!eligible_material(&NifMaterial { base: default(), extension }));
        }
        assert!(!eligible_material(&NifMaterial { base: StandardMaterial { unlit: true, ..default() }, extension: default() }));
    }
}
