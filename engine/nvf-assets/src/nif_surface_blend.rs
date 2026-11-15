//! [design] Enabled-only interface; ordinary NifSurface keeps main's interface.
use super::nif_material::*;
use bevy::prelude::*;
use bevy::pbr::{ExtendedMaterial, MaterialExtension, MaterialExtensionKey, MaterialExtensionPipeline};
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::render::render_resource::{AsBindGroup, ShaderType, RenderPipelineDescriptor, SpecializedMeshPipelineError};
use bevy::shader::{ShaderRef, ShaderDefVal};
// [design] Shared CPU packing and shader specialization dimensions.
pub const RESOLUTION: u32 = 512;
pub const GRID: usize = fnv_world::land::SIDE - 1;
pub const HEIGHT_TILE_WIDTH: usize = GRID * 6;
pub const HEIGHT_TILE_HEIGHT: usize = GRID + 1;
pub fn blend_shader_defs() -> Vec<ShaderDefVal> {
    vec![
        ShaderDefVal::UInt("MESH_TERRAIN_RESOLUTION".into(), RESOLUTION),
        ShaderDefVal::UInt("MESH_TERRAIN_GRID".into(), GRID as u32),
        ShaderDefVal::UInt("MESH_TERRAIN_HEIGHT_TILE_WIDTH".into(), HEIGHT_TILE_WIDTH as u32),
        ShaderDefVal::UInt("MESH_TERRAIN_HEIGHT_TILE_HEIGHT".into(), HEIGHT_TILE_HEIGHT as u32),
        ShaderDefVal::UInt("MESH_TERRAIN_CELL_SIZE".into(), fnv_world::CELL_SIZE as u32),
        ShaderDefVal::UInt("MESH_TERRAIN_SPACING".into(), fnv_world::land::SPACING as u32),
    ]
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
#[bindless(limit(512), index_table(range(50..57), binding(100)))]
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
    #[texture(51)]
    #[sampler(52)]
    pub holes_texture: Option<Handle<Image>>,
    #[texture(53)]
    pub facegen_offset: Option<Handle<Image>>,
    pub mesh_terrain: Vec4,
    pub mesh_terrain_origin: Vec4,
    #[texture(54)]
    #[sampler(55)]
    pub mesh_terrain_colour: Option<Handle<Image>>,
    #[texture(56)]
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
            facegen: s.facegen_offset.is_some(),
            depth_write: s.depth_write,
            first_person: s.first_person,
            add_linear: s.add_linear,
            pipboy_screen: s.pipboy_screen,
        })
    }
}
impl NifSurfaceBlend {
    pub fn from_source(source: &NifSurface, band: f32, variant: f32, origin: Vec4, colour: Handle<Image>, height: Handle<Image>) -> Self {
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
            holes_texture: source.holes_texture.clone(),
            facegen_offset: source.facegen_offset.clone(),
            mesh_terrain: Vec4::new(band, variant, 0.0, 0.0),
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
    use bevy::render::render_resource::{FragmentState, MultisampleState};
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
}
