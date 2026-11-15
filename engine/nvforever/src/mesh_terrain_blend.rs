//! [design] NVForever's aid-only mesh/landscape colour blend: rigid exterior STATs,
//! a 24-unit default contact band, no noise, and immutable source materials.

use crate::terrain_surface::{SurfaceCache, update_cache};
use bevy::prelude::*;
use nvf_assets::nif_material::{NifMaterial, Falloff};
use nvf_assets::nif_surface_blend::{NifBlendMaterial, NifSurfaceBlend};
use std::{collections::HashMap, sync::OnceLock};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Variant { Surface, Weight, Normal, Selection, Status }

impl Variant {
    fn name(self) -> &'static str {
        match self { Self::Surface => "surface", Self::Weight => "weight", Self::Normal => "normal", Self::Selection => "selection", Self::Status => "status" }
    }
    fn number(self) -> f32 {
        match self { Self::Surface => 0.0, Self::Weight => 1.0, Self::Normal => 2.0, Self::Selection => 3.0, Self::Status => 4.0 }
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
        "status" => Variant::Status,
        _ => return Err("expected [positive band]:surface|weight|normal|selection|status"),
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

pub use nvf_assets::nif_surface_blend::BlendShape;

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
            .add_systems(Update, update_cache.after(crate::world::stream))
            .add_systems(PostUpdate, conversion_order(apply_variants));
    }
}

/// [design] Where the conversion runs: ordinary producers (swaps, alpha, scripted fades)
/// finish in Update; in PostUpdate it follows the scripted NIF edits (`nif_mirror`, their
/// commands applied, so an edited source is never left beside the blend material) and the
/// see-through shadow bookkeeping (which must see a restored ordinary material before it
/// is swapped away, or a finished fade keeps its shadow markers), and precedes asset
/// events and mesh/material change marking.
fn conversion_order<M>(system: impl IntoScheduleConfigs<bevy::ecs::system::ScheduleSystem, M>) -> bevy::ecs::schedule::ScheduleConfigs<bevy::ecs::system::ScheduleSystem> {
    system
        .after(crate::nif_mirror::MirrorSystems)
        .after(nvf_assets::nif_material::SeeThroughShadowSystems)
        .before(bevy::asset::AssetEventSystems)
        .before(bevy::mesh::mark_3d_meshes_as_changed_if_their_assets_changed)
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
    apply_configured_variants(&mut commands, &cache, &mut variants, &materials, &mut blends, &mut shapes, c);
}

// [design] Explicit config boundary lets ECS tests publish without process-wide env state.
fn apply_configured_variants(
    commands: &mut Commands,
    cache: &SurfaceCache,
    variants: &mut Variants,
    materials: &Assets<NifMaterial>,
    blends: &mut Assets<NifBlendMaterial>,
    shapes: &mut Query<(Entity, &mut BlendShape, Option<&MeshMaterial3d<NifMaterial>>)>,
    c: BlendConfig,
) {
    if variants.generation != cache.generation {
        variants.generation = cache.generation;
        variants.materials.clear();
    }
    // A producer can edit an owned source in place and republish the same handle
    // (NIF block material edits). Evict once per source before creating shared clones.
    for (_, shape, drawn) in shapes.iter() {
        if shape.enabled.is_none() { continue; }
        if let Some(drawn) = drawn { variants.materials.remove(&drawn.0.id()); }
    }
    let mut eligible = 0;
    for (entity, mut shape, drawn) in shapes.iter_mut() {
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
            info!("mesh terrain blend: ready eligible-shapes={eligible} resident-cells={} generation={} variant={}", surface.cells, cache.generation, c.variant.name());
            variants.reported = Some(cache.generation);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nvf_assets::nif_material::NifSurface;

    fn configured(
        mut commands: Commands, cache: Res<SurfaceCache>, mut variants: ResMut<Variants>,
        materials: Res<Assets<NifMaterial>>, mut blends: ResMut<Assets<NifBlendMaterial>>,
        mut shapes: Query<(Entity, &mut BlendShape, Option<&MeshMaterial3d<NifMaterial>>)>,
    ) {
        apply_configured_variants(&mut commands, &cache, &mut variants, &materials, &mut blends, &mut shapes, parse("24:status").unwrap());
    }
    fn publish(app: &mut App) {
        let mut images = app.world_mut().resource_mut::<Assets<Image>>();
        let surface = crate::terrain_surface::PublishedSurface { colour: images.add(Image::default()), height: images.add(Image::default()), origin: (-18, -1), cells: 9 };
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
        assert_eq!(parse("surface").unwrap(), BlendConfig { band: 24.0, variant: Variant::Surface });
        assert_eq!(parse(":selection").unwrap().band, 24.0);
        assert_eq!(parse("12:weight").unwrap().variant, Variant::Weight);
        assert_eq!(parse("24:normal").unwrap().variant, Variant::Normal);
        assert_eq!(parse("24:status").unwrap().variant, Variant::Status);
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
