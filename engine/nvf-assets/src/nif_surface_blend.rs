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
/// [design] Colour texels per cell side the aid accepts; the default (and the settings
/// row's) is the middle one.
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
/// [design] Hybrid v2's band-width limits (NVForever's round-6 tuning choices): a surface
/// parallel to the ground gets the flat width, ramping smoothly to the shallow width (the
/// band) at 5 degrees (the flat width is not held over 0-5 degrees), those turned 5 to 35
/// degrees the band, and those turned 75 degrees or more the steep width; the steep width
/// ramps in between 35 and 75 degrees ([`CONTACT_COS_FULL_E6`] is cos 35).
pub const V2_COS_PARALLEL_E6: u32 = 996_195; // cos 5 degrees
pub const V2_COS_STEEP_E6: u32 = 258_819; // cos 75 degrees
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
        ShaderDefVal::UInt("MESH_TERRAIN_V2_COS_PARALLEL_E6".into(), V2_COS_PARALLEL_E6),
        ShaderDefVal::UInt("MESH_TERRAIN_V2_COS_STEEP_E6".into(), V2_COS_STEEP_E6),
        ShaderDefVal::UInt("MESH_TERRAIN_ROUGHNESS_E6".into(), to_micro(LAND_PERCEPTUAL_ROUGHNESS)),
        ShaderDefVal::UInt("MESH_TERRAIN_REFLECTANCE_E6".into(), to_micro(LAND_REFLECTANCE)),
    ]
}

/// [design] How the contact weight is formed (`mesh_terrain_weight` in
/// nif_surface_bindings.wesl, [`contact_weight`] here). `Hybrid` is hybrid v2 (round 6,
/// an angle-dependent band width); `HybridV1` keeps round 5's hybrid for comparisons.
/// The shader numbers stay stable; the legacy mode is appended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContactShape { Height, Penetration, Hybrid, HybridV1 }
impl ContactShape {
    pub fn name(self) -> &'static str {
        match self { Self::Height => "height", Self::Penetration => "penetration", Self::Hybrid => "hybrid", Self::HybridV1 => "hybrid-v1" }
    }
    pub fn number(self) -> f32 {
        match self { Self::Height => 0.0, Self::Penetration => 1.0, Self::Hybrid => 2.0, Self::HybridV1 => 3.0 }
    }
    /// The weight formula's version the logs name: 2 for hybrid v2, 1 for the formulas
    /// round 5 introduced and round 6 left unchanged.
    pub fn weight_version(self) -> u32 {
        if self == Self::Hybrid { 2 } else { 1 }
    }
}

/// [design] Hybrid v2's band widths in game units: `flat` for surfaces parallel to the
/// ground, `band` (the aid's band, the shallow width) for shoulders turned 5-35 degrees,
/// `steep` for faces turned 75 degrees or more. Omitted widths are `band / 16` and
/// `6 band` (0.5 and 48 at the default band of 8); NVForever's own choices.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ContactWidths { pub flat: f32, pub band: f32, pub steep: f32 }
impl ContactWidths {
    pub const FLAT_SHARE: f32 = 1.0 / 16.0;
    pub const STEEP_FACTOR: f32 = 6.0;
    pub fn from_band(band: f32) -> Self {
        Self { flat: band * Self::FLAT_SHARE, band, steep: band * Self::STEEP_FACTOR }
    }
    /// Finite, positive and ordered `flat <= band <= steep`.
    pub fn valid(&self) -> bool {
        [self.flat, self.band, self.steep].iter().all(|w| w.is_finite() && *w > 0.0)
            && self.flat <= self.band && self.band <= self.steep
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
/// [design] Hybrid v2's band width for the cosine `cos_angle` between the unperturbed mesh
/// normal and the ground triangle's normal: `abs` treats opposite parallel normals alike;
/// `shallow` rises from 0 at the parallel to 1 at 5 degrees, `steep` from 0 at 35 to 1 at
/// 75 degrees, so the width is continuous in the angle with no on/off gate.
pub fn v2_band(widths: ContactWidths, cos_angle: f32) -> f32 {
    let c = cos_angle.abs().clamp(0.0, 1.0);
    let shallow = 1.0 - smoothstep(micro(V2_COS_PARALLEL_E6), 1.0, c);
    let steep = 1.0 - smoothstep(micro(V2_COS_STEEP_E6), micro(CONTACT_COS_FULL_E6), c);
    widths.flat + (widths.band - widths.flat) * shallow + (widths.steep - widths.band) * steep
}

/// [design] The CPU statement of the shader's contact weight, before the residency fade:
/// `d` is the height above the ground (negative below it), `cos_angle` the cosine between
/// the unperturbed mesh normal and the ground triangle's normal. Height, penetration and
/// hybrid v1 use only `widths.band`; hybrid v2 also the flat and steep widths.
pub fn contact_weight(shape: ContactShape, widths: ContactWidths, d: f32, cos_angle: f32) -> f32 {
    let height = 1.0 - smoothstep(0.0, widths.band, d.max(0.0));
    let penetration = 1.0 - smoothstep(-micro(PENETRATION_E6), 0.0, d);
    match shape {
        ContactShape::Height => height,
        ContactShape::Penetration => penetration,
        ContactShape::HybridV1 => {
            let turned = 1.0 - smoothstep(micro(CONTACT_COS_FULL_E6), micro(CONTACT_COS_NONE_E6), cos_angle.clamp(-1.0, 1.0));
            height * turned.max(penetration)
        }
        ContactShape::Hybrid => {
            let band = v2_band(widths, cos_angle);
            (1.0 - smoothstep(0.0, band, d.max(0.0))).max(penetration)
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
    /// Hybrid v2's flat and steep widths in x and y (game units); z, w unused
    /// (`BlendConfig::widths`). Enabled-only: the ordinary uniform is unchanged.
    pub mesh_terrain_widths: Vec4,
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
    pub mesh_terrain_widths: Vec4,
}

impl From<&NifSurfaceBlend> for NifSurfaceBlendUniform {
    fn from(s: &NifSurfaceBlend) -> Self {
        NifSurfaceBlendUniform { falloff: s.falloff, alpha_cut: s.alpha_cut, add_whole: s.add_whole, refraction: s.refraction * REFRACTION_SCREEN, fire_period: s.fire_period, emissive_rule: s.emissive_rule, holes: s.holes, holes_edge: s.holes_edge, soft: s.soft, pipboy_a: s.pipboy_a, pipboy_b: s.pipboy_b, mesh_terrain: s.mesh_terrain, mesh_terrain_origin: s.mesh_terrain_origin, mesh_terrain_widths: s.mesh_terrain_widths }
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
    /// contact shape and shading, `widths` hybrid v2's flat and steep widths.
    pub fn from_source(source: &NifSurface, controls: Vec4, widths: Vec4, origin: Vec4, colour: Handle<Image>, height: Handle<Image>) -> Self {
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
            mesh_terrain_widths: widths,
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
        assert_eq!(NifSurfaceBlendUniform::min_size().get(), 176, "one vec4 of widths after round 5's 160 bytes");
        assert_eq!(crate::nif_material::NifSurfaceUniform::min_size().get(), 128, "the ordinary uniform is unchanged");
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
        assert_eq!((blend.buffers[0].binding_number.0, blend.buffers[0].bindless_index.0, blend.buffers[0].size), (101, 50, Some(176)));
        let source = include_str!("nif_surface_blend.rs");
        let fields = source.split("pub struct NifSurfaceBlend {").nth(1).unwrap().split("\n}").next().unwrap();
        assert_eq!(fields.matches("#[texture(").count(), 2);
        assert!(fields.contains("#[texture(51)]") && fields.contains("#[sampler(52)]") && fields.contains("#[texture(53)]"));
        assert!(!fields.contains("#[texture(54)]") && !fields.contains("holes_texture") && !fields.contains("facegen_offset"));
    }

    #[test]
    fn contact_weight_follows_band_orientation_and_penetration() {
        // Round 5's shapes, unchanged in round 6 (hybrid v1 is round 5's hybrid).
        let cos = |deg: f32| deg.to_radians().cos();
        let (parallel, sloped, upright) = (1.0, cos(25.0), 0.0);
        // Heights -2, 0, 4, 8, 12 with the default band of 8.
        let heights = [-2.0, 0.0, 4.0, 8.0, 12.0];
        let w = ContactWidths::from_band(8.0);
        let height = heights.map(|d| contact_weight(ContactShape::Height, w, d, parallel));
        assert_eq!(height, [1.0, 1.0, 0.5, 0.0, 0.0]);
        let penetration = heights.map(|d| contact_weight(ContactShape::Penetration, w, d, upright));
        assert_eq!(penetration, [1.0, 0.0, 0.0, 0.0, 0.0]);
        assert_eq!(contact_weight(ContactShape::Penetration, w, -1.0, upright), 0.5, "continuous between -2 and 0");
        // The flat and steep widths do not reach the old shapes.
        let odd = ContactWidths { flat: 0.01, band: 8.0, steep: 500.0 };
        for shape in [ContactShape::Height, ContactShape::Penetration, ContactShape::HybridV1] {
            for d in heights { for c in [parallel, sloped, upright] {
                assert_eq!(contact_weight(shape, odd, d, c), contact_weight(shape, w, d, c));
            }}
        }
        let v1 = ContactShape::HybridV1;
        // Hybrid v1: a parallel top above ground gets nothing, coplanar included.
        assert_eq!(heights.map(|d| contact_weight(v1, w, d, parallel)), [1.0, 0.0, 0.0, 0.0, 0.0]);
        // An upright face gets the full band above ground, and full weight below.
        assert_eq!(heights.map(|d| contact_weight(v1, w, d, upright)), [1.0, 1.0, 0.5, 0.0, 0.0]);
        // 25 degrees lies between the 15 and 35 degree limits: part weight.
        let part = contact_weight(v1, w, 0.0, sloped);
        assert!(part > 0.0 && part < 1.0, "{part}");
        assert!((contact_weight(v1, w, 0.0, cos(35.0)) - 1.0).abs() < 1e-6);
        assert!(contact_weight(v1, w, 0.0, cos(15.0)) < 1e-6);
        // A road top 10-15 units above the ground gets nothing at the default band.
        for d in [10.0, 15.0] { assert_eq!(contact_weight(v1, w, d, upright), 0.0); }
        // Out-of-range cosines are clamped; a buried parallel surface still saturates.
        assert_eq!(contact_weight(v1, w, -3.0, 2.0), 1.0);
        assert_eq!((ContactShape::HybridV1.number(), ContactShape::HybridV1.name()), (3.0, "hybrid-v1"));
        assert_eq!([ContactShape::Height, ContactShape::Penetration, ContactShape::Hybrid].map(|s| s.number()), [0.0, 1.0, 2.0], "numbers stay stable");
        assert_eq!([ContactShape::Hybrid, ContactShape::HybridV1, ContactShape::Height].map(|s| s.weight_version()), [2, 1, 1]);
    }

    #[test]
    fn hybrid_v2_band_has_flat_shallow_and_steep_plateaus() {
        let cos = |deg: f32| deg.to_radians().cos();
        let w = ContactWidths::from_band(8.0);
        assert_eq!(w, ContactWidths { flat: 0.5, band: 8.0, steep: 48.0 }, "the default widths");
        let close = |a: f32, b: f32| (a - b).abs() < 1e-3;
        // Parallel (either facing) gets the flat width; 5-35 degrees the band; 75-90 the steep.
        assert!(close(v2_band(w, 1.0), 0.5) && close(v2_band(w, -1.0), 0.5));
        for deg in [5.0, 10.0, 20.0, 35.0] { assert!(close(v2_band(w, cos(deg)), 8.0), "{deg}: {}", v2_band(w, cos(deg))); }
        for deg in [75.0, 80.0, 90.0, 105.0] { assert!(close(v2_band(w, cos(deg)), 48.0), "{deg}"); }
        // Ramps in between, monotonic in the angle.
        let mut last = 0.0;
        for tenth in 0..=900 {
            let b = v2_band(w, cos(tenth as f32 / 10.0));
            assert!(b >= last - 1e-4, "{tenth}: {b} < {last}");
            assert!((0.5 - 1e-4..=48.0 + 1e-4).contains(&b));
            last = b;
        }
        let mid = v2_band(w, cos(55.0));
        assert!(mid > 8.0 && mid < 48.0, "{mid}");
        // Explicit widths change only the flat and steep plateaus.
        let wide = ContactWidths { flat: 0.5, band: 8.0, steep: 64.0 };
        assert!(close(v2_band(wide, cos(20.0)), 8.0) && close(v2_band(wide, 0.0), 64.0));
    }

    #[test]
    fn hybrid_v2_weight_is_continuous_saturates_below_and_leaves_tops_alone() {
        let cos = |deg: f32| deg.to_radians().cos();
        let w = ContactWidths::from_band(8.0);
        let v2 = |d: f32, c: f32| contact_weight(ContactShape::Hybrid, w, d, c);
        // Below the ground every orientation saturates (H = 1); penetration is redundant there.
        for deg in [0.0, 3.0, 20.0, 60.0, 90.0] { for d in [-5.0, -2.0, -0.5, 0.0] {
            assert_eq!(v2(d, cos(deg)), 1.0, "{deg} {d}");
        }}
        // A parallel asphalt top 10-15 units above the ground (and anything over 0.5) gets none;
        // so does a top tilted by a few degrees.
        for d in [0.5, 1.0, 10.0, 12.5, 15.0] { assert_eq!(v2(d, 1.0), 0.0, "{d}"); }
        for d in [10.0, 12.5, 15.0] { for deg in [1.0, 3.0] {
            assert_eq!(v2(d, cos(deg)), 0.0, "{deg} {d}");
        }}
        // Shoulders: round 5's successful 8-unit band; rock faces: 48 units.
        assert_eq!(v2(4.0, cos(20.0)), contact_weight(ContactShape::Height, w, 4.0, 1.0));
        assert!((v2(24.0, cos(90.0)) - 0.5).abs() < 1e-6 && v2(48.0, cos(90.0)) == 0.0);
        // Continuous in height and angle: small steps change the weight by a little.
        for deg10 in 0..900 { for d10 in -30..600 {
            let (deg, d) = (deg10 as f32 / 10.0, d10 as f32 / 10.0);
            let here = v2(d, cos(deg));
            assert!((0.0..=1.0).contains(&here));
            assert!((v2(d + 0.1, cos(deg)) - here).abs() < 0.35, "height jump at {deg} {d}");
            assert!((v2(d, cos(deg + 0.1)) - here).abs() < 0.35, "angle jump at {deg} {d}");
        }}
        // The degenerate-normal fallback (cosine 1) is the flat width.
        assert_eq!(v2(1.0, 1.0), 0.0);
    }

    #[test]
    fn contact_widths_validate_order_and_finiteness() {
        assert!(ContactWidths::from_band(8.0).valid());
        assert!(ContactWidths { flat: 8.0, band: 8.0, steep: 8.0 }.valid(), "equal widths are ordered");
        for bad in [
            ContactWidths { flat: 0.0, band: 8.0, steep: 48.0 }, ContactWidths { flat: -1.0, band: 8.0, steep: 48.0 },
            ContactWidths { flat: 9.0, band: 8.0, steep: 48.0 }, ContactWidths { flat: 0.5, band: 8.0, steep: 4.0 },
            ContactWidths { flat: f32::NAN, band: 8.0, steep: 48.0 }, ContactWidths { flat: 0.5, band: 8.0, steep: f32::INFINITY },
            ContactWidths::from_band(f32::MAX),
        ] { assert!(!bad.valid(), "{bad:?}"); }
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
        assert_eq!(uint("MESH_TERRAIN_V2_COS_PARALLEL_E6"), V2_COS_PARALLEL_E6);
        assert_eq!(uint("MESH_TERRAIN_V2_COS_STEEP_E6"), V2_COS_STEEP_E6);
        assert!((micro(V2_COS_PARALLEL_E6) - 5f32.to_radians().cos()).abs() < 1e-6);
        assert!((micro(V2_COS_STEEP_E6) - 75f32.to_radians().cos()).abs() < 1e-6);
        assert!(!defs.iter().any(|d| matches!(d, ShaderDefVal::UInt(n, _) if n.contains("RESOLUTION"))), "the shader reads the atlas size from the texture");
    }
}
