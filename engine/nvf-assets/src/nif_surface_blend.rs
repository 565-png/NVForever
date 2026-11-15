//! [design] Enabled-only interface; ordinary NifSurface keeps main's interface.
//!
//! The blend type has the same bindings as the ordinary [`NifSurface`]: uniform 50 (a
//! longer uniform), texture 51, sampler 52 and texture 53, index table 50..54. Where the
//! ordinary type binds its holes texture, that texture's sampler and the FaceGen offset,
//! the blend type binds the landscape colour atlas, its sampler and the height atlas.
//! Blend clones are made only from materials without holes or FaceGen
//! (nvforever `mesh_terrain_blend.rs::eligible_material`), so neither ordinary use is
//! needed, and a blend pipeline binds no more textures in any stage than the ordinary
//! NIF pipeline it replaces (the view and mesh groups are the same mesh pipeline's).
use super::nif_material::*;
use bevy::prelude::*;
use bevy::pbr::{ExtendedMaterial, MaterialExtension, MaterialExtensionKey, MaterialExtensionPipeline};
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::render::render_resource::{AsBindGroup, ShaderType, RenderPipelineDescriptor, SpecializedMeshPipelineError};
use bevy::shader::{ShaderRef, ShaderDefVal};
// [design] Shared CPU packing and shader specialization dimensions. The colour atlas's
// resolution is not one of them: the shader reads it from the bound texture.
pub const GRID: usize = fnv_world::land::SIDE - 1;
pub const HEIGHT_TILE_WIDTH: usize = GRID * 6;
pub const HEIGHT_TILE_HEIGHT: usize = GRID + 1;
/// [design] Colour texels per cell side the aid accepts; the default is the middle one.
pub const RESOLUTIONS: [u32; 3] = [512, 1024, 2048];
pub const DEFAULT_RESOLUTION: u32 = 1024;
/// [design] NVForever's contact constants (initial tuning choices, not taken from another
/// renderer): surfaces turned at least 35 degrees from the ground get the full contact
/// weight, those within 15 degrees none; 2 units below the ground count as penetrating.
/// Millionths, so the shader (via [`blend_shader_defs`]) and [`contact_weight`] share
/// the same numbers.
pub const CONTACT_COS_FULL_E6: u32 = 819_152; // cos 35 degrees
pub const CONTACT_COS_NONE_E6: u32 = 965_926; // cos 15 degrees
pub const PENETRATION_E6: u32 = 2_000_000;
/// The landscape's own material values (nvforever `terrain.rs::surface` uses these), the
/// targets of the full shading blend.
pub const LAND_PERCEPTUAL_ROUGHNESS: f32 = 0.95;
pub const LAND_REFLECTANCE: f32 = 0.2;
pub fn micro(value: u32) -> f32 { value as f32 / 1e6 }
fn to_micro(value: f32) -> u32 { (value * 1e6).round() as u32 }

pub fn blend_shader_defs() -> Vec<ShaderDefVal> {
    vec![
        ShaderDefVal::UInt("MESH_TERRAIN_GRID".into(), GRID as u32),
        ShaderDefVal::UInt("MESH_TERRAIN_HEIGHT_TILE_WIDTH".into(), HEIGHT_TILE_WIDTH as u32),
        ShaderDefVal::UInt("MESH_TERRAIN_HEIGHT_TILE_HEIGHT".into(), HEIGHT_TILE_HEIGHT as u32),
        ShaderDefVal::UInt("MESH_TERRAIN_CELL_SIZE".into(), fnv_world::CELL_SIZE as u32),
        ShaderDefVal::UInt("MESH_TERRAIN_SPACING".into(), fnv_world::land::SPACING as u32),
        ShaderDefVal::UInt("MESH_TERRAIN_COS_FULL_E6".into(), CONTACT_COS_FULL_E6),
        ShaderDefVal::UInt("MESH_TERRAIN_COS_NONE_E6".into(), CONTACT_COS_NONE_E6),
        ShaderDefVal::UInt("MESH_TERRAIN_PENETRATION_E6".into(), PENETRATION_E6),
        ShaderDefVal::UInt("MESH_TERRAIN_ROUGHNESS_E6".into(), to_micro(LAND_PERCEPTUAL_ROUGHNESS)),
        ShaderDefVal::UInt("MESH_TERRAIN_REFLECTANCE_E6".into(), to_micro(LAND_REFLECTANCE)),
    ]
}

/// [design] How the contact weight is formed (`mesh_terrain_weight` in
/// nif_surface_bindings.wesl, [`contact_weight`] here).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContactShape { Height, Penetration, Hybrid }
impl ContactShape {
    pub fn name(self) -> &'static str {
        match self { Self::Height => "height", Self::Penetration => "penetration", Self::Hybrid => "hybrid" }
    }
    pub fn number(self) -> f32 {
        match self { Self::Height => 0.0, Self::Penetration => 1.0, Self::Hybrid => 2.0 }
    }
}
/// [design] What the weight blends: the colour alone, or colour, normal and material.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shading { Albedo, Full }
impl Shading {
    pub fn name(self) -> &'static str {
        match self { Self::Albedo => "albedo", Self::Full => "full" }
    }
    pub fn number(self) -> f32 {
        match self { Self::Albedo => 0.0, Self::Full => 1.0 }
    }
}

/// WGSL's `smoothstep` for `low < high`.
pub fn smoothstep(low: f32, high: f32, x: f32) -> f32 {
    let t = ((x - low) / (high - low)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}
/// [design] The CPU statement of the shader's contact weight, before the residency fade:
/// `d` is the height above the ground (negative below it), `cos_angle` the cosine between
/// the unperturbed mesh normal and the ground triangle's normal.
pub fn contact_weight(shape: ContactShape, band: f32, d: f32, cos_angle: f32) -> f32 {
    let height = 1.0 - smoothstep(0.0, band, d.max(0.0));
    let penetration = 1.0 - smoothstep(-micro(PENETRATION_E6), 0.0, d);
    match shape {
        ContactShape::Height => height,
        ContactShape::Penetration => penetration,
        ContactShape::Hybrid => {
            let turned = 1.0 - smoothstep(micro(CONTACT_COS_FULL_E6), micro(CONTACT_COS_NONE_E6), cos_angle.clamp(-1.0, 1.0));
            height * turned.max(penetration)
        }
    }
}
/// [design] The geometric normal of a ground triangle in game space (x east, y north, z
/// up) from its plane's per-square slopes (`terrain_surface.rs::height_planes`).
pub fn terrain_game_normal(dx: f32, dy: f32) -> Vec3 {
    let spacing = fnv_world::land::SPACING;
    Vec3::new(-dx / spacing, -dy / spacing, 1.0).normalize()
}
/// Game-space direction to Bevy world space (nvforever `world.rs::game_to_bevy`).
pub fn game_to_world_normal(n: Vec3) -> Vec3 { Vec3::new(n.x, n.z, -n.y) }
/// [design] The shading normal moved towards the ground's by `w`; `w <= 0` keeps the mesh
/// normal exactly, and a mix that cancels out falls back to the ground's normal.
pub fn blend_normal(mesh: Vec3, terrain: Vec3, w: f32) -> Vec3 {
    if w <= 0.0 { return mesh; }
    let mixed = mesh.lerp(terrain, w.min(1.0));
    if mixed.length_squared() < 1e-12 { terrain } else { mixed.normalize() }
}

pub type NifBlendMaterial = ExtendedMaterial<StandardMaterial, NifSurfaceBlend>;

// [design] The source remains accessible to gameplay material producers even while
// the renderer uses the separate blend type. Fade/range state stays on the entity.
#[derive(Component)]
pub struct BlendShape {
    pub original: Handle<NifMaterial>,
    pub enabled: Option<Handle<NifBlendMaterial>>,
}
impl BlendShape {
    pub fn new(original: Handle<NifMaterial>) -> Self { Self { original, enabled: None } }
}
pub fn source_material<'a>(ordinary: Option<&'a MeshMaterial3d<NifMaterial>>, blend: Option<&'a BlendShape>) -> Option<&'a Handle<NifMaterial>> {
    ordinary.map(|m| &m.0).or_else(|| blend.map(|b| &b.original))
}
#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
#[data(50, NifSurfaceBlendUniform, binding_array(101))]
#[bindless(limit(512), index_table(range(50..54), binding(100)))]
#[bind_group_data(NifSurfaceBlendKey)]
pub struct NifSurfaceBlend {
    pub falloff: Vec4,
    pub alpha_cut: f32,
    pub add_whole: f32,
    pub refraction: f32,
    pub fire_period: f32,
    pub emissive_rule: Vec4,
    pub refract: bool,
    pub decal: bool,
    pub hard_edges: bool,
    pub shade_light: bool,
    pub depth_write: bool,
    pub first_person: bool,
    pub add_linear: bool,
    pub pipboy_screen: bool,
    pub pipboy_a: Vec4,
    pub pipboy_b: Vec4,
    pub blended_cutout: Option<f32>,
    pub holes: Vec4,
    pub holes_edge: Vec4,
    pub soft: Vec4,
    /// Band, view, contact shape, shading (the aid's numbers, nvforever
    /// `mesh_terrain_blend.rs::BlendConfig::controls`).
    pub mesh_terrain: Vec4,
    pub mesh_terrain_origin: Vec4,
    /// In the ordinary type's holes texture and sampler slots (module notes).
    #[texture(51)]
    #[sampler(52)]
    pub mesh_terrain_colour: Option<Handle<Image>>,
    /// In the ordinary type's FaceGen offset slot (module notes).
    #[texture(53)]
    pub mesh_terrain_height: Option<Handle<Image>>,
}

#[derive(Clone, Default, ShaderType)]
pub struct NifSurfaceBlendUniform {
    pub falloff: Vec4,
    pub alpha_cut: f32,
    pub add_whole: f32,
    pub refraction: f32,
    pub fire_period: f32,
    pub emissive_rule: Vec4,
    pub holes: Vec4,
    pub holes_edge: Vec4,
    pub soft: Vec4,
    pub pipboy_a: Vec4,
    pub pipboy_b: Vec4,
    pub mesh_terrain: Vec4,
    pub mesh_terrain_origin: Vec4,
}

impl From<&NifSurfaceBlend> for NifSurfaceBlendUniform {
    fn from(s: &NifSurfaceBlend) -> Self {
        NifSurfaceBlendUniform { falloff: s.falloff, alpha_cut: s.alpha_cut, add_whole: s.add_whole, refraction: s.refraction * REFRACTION_SCREEN, fire_period: s.fire_period, emissive_rule: s.emissive_rule, holes: s.holes, holes_edge: s.holes_edge, soft: s.soft, pipboy_a: s.pipboy_a, pipboy_b: s.pipboy_b, mesh_terrain: s.mesh_terrain, mesh_terrain_origin: s.mesh_terrain_origin }
    }
}


#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NifSurfaceBlendKey(pub NifSurfaceKey);
impl From<&NifSurfaceBlend> for NifSurfaceBlendKey {
    fn from(s: &NifSurfaceBlend) -> Self {
        Self(NifSurfaceKey {
            decal: s.decal,
            refract: s.refract,
            hard_edges: s.hard_edges,
            shade_light: s.shade_light,
            // No FaceGen: slot 53 holds the height atlas (module notes).
            facegen: false,
            depth_write: s.depth_write,
            first_person: s.first_person,
            add_linear: s.add_linear,
            pipboy_screen: s.pipboy_screen,
        })
    }
}
impl NifSurfaceBlend {
    /// `source` has neither holes nor FaceGen (eligibility); `controls` are band, view,
    /// contact shape and shading.
    pub fn from_source(source: &NifSurface, controls: Vec4, origin: Vec4, colour: Handle<Image>, height: Handle<Image>) -> Self {
        debug_assert!(source.holes_texture.is_none() && source.facegen_offset.is_none(), "blend clones are made only without holes or FaceGen");
        Self {
            falloff: source.falloff,
            alpha_cut: source.alpha_cut,
            add_whole: source.add_whole,
            refraction: source.refraction,
            fire_period: source.fire_period,
            emissive_rule: source.emissive_rule,
            refract: source.refract,
            decal: source.decal,
            hard_edges: source.hard_edges,
            shade_light: source.shade_light,
            depth_write: source.depth_write,
            first_person: source.first_person,
            add_linear: source.add_linear,
            pipboy_screen: source.pipboy_screen,
            pipboy_a: source.pipboy_a,
            pipboy_b: source.pipboy_b,
            blended_cutout: source.blended_cutout,
            holes: source.holes,
            holes_edge: source.holes_edge,
            soft: source.soft,
            mesh_terrain: controls,
            mesh_terrain_origin: origin,
            mesh_terrain_colour: Some(colour),
            mesh_terrain_height: Some(height),
        }
    }
}
impl MaterialExtension for NifSurfaceBlend {
    fn fragment_shader() -> ShaderRef { NifSurface::fragment_shader() }
    fn deferred_fragment_shader() -> ShaderRef { NifSurface::deferred_fragment_shader() }
    fn specialize(pipeline: &MaterialExtensionPipeline, descriptor: &mut RenderPipelineDescriptor, layout: &MeshVertexBufferLayoutRef, key: MaterialExtensionKey<Self>) -> Result<(), SpecializedMeshPipelineError> {
        NifSurface::specialize(pipeline, descriptor, layout, MaterialExtensionKey::<NifSurface> { mesh_key: key.mesh_key, bind_group_data: key.bind_group_data.0 })?;
        blend_interface(descriptor);
        Ok(())
    }
}

fn blend_interface(descriptor: &mut RenderPipelineDescriptor) {
    // The separate layout is also declared in the otherwise identical NIF prepass.
    descriptor.vertex.shader_defs.push("NIF_SURFACE_BLEND".into());
    let prepass = descriptor.vertex.shader_defs.contains(&ShaderDefVal::from("PREPASS_PIPELINE"));
    if let Some(fragment) = descriptor.fragment.as_mut() {
        fragment.shader_defs.push("NIF_SURFACE_BLEND".into());
        if !prepass {
            fragment.shader_defs.push("MESH_TERRAIN_BLEND".into());
            fragment.shader_defs.extend(blend_shader_defs());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::render::render_resource::{BindlessDescriptor, BindlessResourceType as R, FragmentState, MultisampleState};
    #[test]
    fn enabled_interface_preserves_descriptors_and_keeps_blend_out_of_prepass() {
        for samples in [1, 4, 8] { for prepass in [false, true] {
            let mut descriptor = RenderPipelineDescriptor {
                fragment: Some(FragmentState::default()),
                multisample: MultisampleState { count: samples, ..default() },
                ..default()
            };
            if prepass { descriptor.vertex.shader_defs.push("PREPASS_PIPELINE".into()); }
            let original = descriptor.clone();
            blend_interface(&mut descriptor);
            assert_eq!(descriptor.depth_stencil, original.depth_stencil);
            assert_eq!(descriptor.multisample, original.multisample);
            assert_eq!(descriptor.primitive, original.primitive);
            assert_eq!(descriptor.vertex.shader, original.vertex.shader);
            assert_eq!(descriptor.fragment.as_ref().unwrap().shader, original.fragment.as_ref().unwrap().shader);
            assert_eq!(descriptor.fragment.as_ref().unwrap().targets, original.fragment.as_ref().unwrap().targets);
            assert_eq!(descriptor.fragment.as_ref().unwrap().shader_defs.contains(&"MESH_TERRAIN_BLEND".into()), !prepass);
        }}
        assert_eq!(NifSurfaceBlendUniform::min_size().get(), 160);
    }

    fn textures(descriptor: &BindlessDescriptor) -> usize {
        descriptor.resources.iter().filter(|r| matches!(r, R::Texture1d | R::Texture2d | R::Texture2dArray | R::Texture3d | R::TextureCube | R::TextureCubeArray)).count()
    }
    /// [design] Without bindless every resource of these tables is its own binding, so the
    /// texture entries are the material group's sampled textures in the fragment stage.
    /// The blend pipeline's other groups (view, mesh) are the same mesh pipeline's as the
    /// ordinary NIF pipeline's, so equal material groups mean equal per-stage totals: the
    /// blend can never exceed a 16-texture stage limit the ordinary shape fits.
    #[test]
    fn blend_layout_binds_exactly_the_ordinary_slots_and_texture_count() {
        let ordinary = NifSurface::bindless_descriptor().unwrap();
        let blend = NifSurfaceBlend::bindless_descriptor().unwrap();
        // Both extend the same StandardMaterial base.
        assert_eq!(textures(&blend), textures(&ordinary));
        assert_eq!(textures(&blend), 2);
        assert_eq!(&blend.resources[50..], &ordinary.resources[50..], "same slot types: 50 data, 51 texture, 52 sampler, 53 texture");
        assert_eq!(blend.index_tables.len(), 1);
        let (b, o) = (&blend.index_tables[0], &ordinary.index_tables[0]);
        assert_eq!((b.indices.start.0, b.indices.end.0, b.binding_number.0), (o.indices.start.0, o.indices.end.0, o.binding_number.0));
        assert_eq!((blend.buffers[0].binding_number.0, blend.buffers[0].bindless_index.0, blend.buffers[0].size), (101, 50, Some(160)));
        let source = include_str!("nif_surface_blend.rs");
        let fields = source.split("pub struct NifSurfaceBlend {").nth(1).unwrap().split("\n}").next().unwrap();
        assert_eq!(fields.matches("#[texture(").count(), 2);
        assert!(fields.contains("#[texture(51)]") && fields.contains("#[sampler(52)]") && fields.contains("#[texture(53)]"));
        assert!(!fields.contains("#[texture(54)]") && !fields.contains("holes_texture") && !fields.contains("facegen_offset"));
    }

    #[test]
    fn contact_weight_follows_band_orientation_and_penetration() {
        let cos = |deg: f32| deg.to_radians().cos();
        let (parallel, sloped, upright) = (1.0, cos(25.0), 0.0);
        // Heights -2, 0, 4, 8, 12 with the default band of 8.
        let heights = [-2.0, 0.0, 4.0, 8.0, 12.0];
        let band = 8.0;
        let height = heights.map(|d| contact_weight(ContactShape::Height, band, d, parallel));
        assert_eq!(height, [1.0, 1.0, 0.5, 0.0, 0.0]);
        let penetration = heights.map(|d| contact_weight(ContactShape::Penetration, band, d, upright));
        assert_eq!(penetration, [1.0, 0.0, 0.0, 0.0, 0.0]);
        assert_eq!(contact_weight(ContactShape::Penetration, band, -1.0, upright), 0.5, "continuous between -2 and 0");
        // Hybrid: a parallel top above ground gets nothing, coplanar included.
        assert_eq!(heights.map(|d| contact_weight(ContactShape::Hybrid, band, d, parallel)), [1.0, 0.0, 0.0, 0.0, 0.0]);
        // An upright face gets the full band above ground, and full weight below.
        assert_eq!(heights.map(|d| contact_weight(ContactShape::Hybrid, band, d, upright)), [1.0, 1.0, 0.5, 0.0, 0.0]);
        // 25 degrees lies between the 15 and 35 degree limits: part weight.
        let part = contact_weight(ContactShape::Hybrid, band, 0.0, sloped);
        assert!(part > 0.0 && part < 1.0, "{part}");
        assert!((contact_weight(ContactShape::Hybrid, band, 0.0, cos(35.0)) - 1.0).abs() < 1e-6);
        assert!(contact_weight(ContactShape::Hybrid, band, 0.0, cos(15.0)) < 1e-6);
        // A road top 10-15 units above the ground gets nothing at the default band.
        for d in [10.0, 15.0] { assert_eq!(contact_weight(ContactShape::Hybrid, band, d, upright), 0.0); }
        // Out-of-range cosines are clamped; a buried parallel surface still saturates.
        assert_eq!(contact_weight(ContactShape::Hybrid, band, -3.0, 2.0), 1.0);
    }

    #[test]
    fn terrain_normal_matches_plane_slopes_and_converts_to_world_space() {
        let spacing = fnv_world::land::SPACING;
        assert_eq!(terrain_game_normal(0.0, 0.0), Vec3::Z);
        // A square rising 128 units eastwards (a 1:1 slope in x only).
        let n = terrain_game_normal(spacing, 0.0);
        assert!((n - Vec3::new(-1.0, 0.0, 1.0).normalize()).length() < 1e-6);
        // Perpendicular to both triangles' edges for arbitrary heights (height_planes'
        // coefficients: [a, east-a, ne-east] and [a, ne-north, north-a]).
        let (a, east, north, ne) = (10.0, 40.0, -5.0, 22.0);
        for (dx, dy, corners) in [
            (east - a, ne - east, [Vec3::new(0.0, 0.0, a), Vec3::new(spacing, 0.0, east), Vec3::new(spacing, spacing, ne)]),
            (ne - north, north - a, [Vec3::new(0.0, 0.0, a), Vec3::new(spacing, spacing, ne), Vec3::new(0.0, spacing, north)]),
        ] {
            let n = terrain_game_normal(dx, dy);
            assert!(n.dot(corners[1] - corners[0]).abs() < 1e-3 && n.dot(corners[2] - corners[0]).abs() < 1e-3);
            assert!(n.z > 0.0);
        }
        // Game (x east, y north, z up) -> Bevy (x, z, -y): up is +Y, north is -Z.
        assert_eq!(game_to_world_normal(Vec3::Z), Vec3::Y);
        assert_eq!(game_to_world_normal(Vec3::Y), Vec3::NEG_Z);
        assert_eq!(game_to_world_normal(Vec3::X), Vec3::X);
    }

    #[test]
    fn blended_normal_is_exact_at_zero_and_safe_when_opposed() {
        let mesh = Vec3::new(0.3, 0.4, 0.866).normalize();
        assert_eq!(blend_normal(mesh, Vec3::Y, 0.0), mesh, "w = 0 keeps the input exactly");
        assert_eq!(blend_normal(mesh, Vec3::Y, -0.5), mesh);
        assert!((blend_normal(mesh, Vec3::Y, 1.0) - Vec3::Y).length() < 1e-6);
        let half = blend_normal(Vec3::X, Vec3::Y, 0.5);
        assert!((half.length() - 1.0).abs() < 1e-6 && (half - Vec3::new(1.0, 1.0, 0.0).normalize()).length() < 1e-6);
        // Opposed normals cancel at w = 0.5: the ground normal stands in.
        assert_eq!(blend_normal(Vec3::NEG_Y, Vec3::Y, 0.5), Vec3::Y);
        assert_eq!(blend_normal(Vec3::ZERO, Vec3::Y, 0.5).length(), 1.0);
    }

    #[test]
    fn shared_constants_reach_the_shader_definitions() {
        let defs = blend_shader_defs();
        let uint = |name: &str| defs.iter().find_map(|d| match d { ShaderDefVal::UInt(n, v) if n == name => Some(*v), _ => None }).unwrap();
        assert_eq!(uint("MESH_TERRAIN_ROUGHNESS_E6"), 950_000);
        assert_eq!(uint("MESH_TERRAIN_REFLECTANCE_E6"), 200_000);
        assert_eq!(uint("MESH_TERRAIN_COS_FULL_E6"), CONTACT_COS_FULL_E6);
        assert!((micro(CONTACT_COS_FULL_E6) - 35f32.to_radians().cos()).abs() < 1e-6);
        assert!((micro(CONTACT_COS_NONE_E6) - 15f32.to_radians().cos()).abs() < 1e-6);
        assert!(!defs.iter().any(|d| matches!(d, ShaderDefVal::UInt(n, _) if n.contains("RESOLUTION"))), "the shader reads the atlas size from the texture");
    }
}
