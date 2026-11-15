//! [design] CPU-only WESL composition and naga validation of the production binding/
//! blend module. Minimal mesh/PBR declarations stand in for the unrelated renderer.
//! Further tests compose the complete forward and NIF depth-prepass shaders, the folded
//! terrain bake and the single-texture capture using repository-vendored modules, and
//! count the textures each declares per bind group. None executes fragments or creates
//! GPU pipelines.

use bevy::prelude::*;
use bevy::shader::{ShaderCache, ShaderCacheSource, ShaderDefVal};
use wgpu::naga;

/// A CPU shader cache whose module-creation callback parses and validates with naga.
fn validating_cache() -> ShaderCache<String, ()> {
    ShaderCache::new((), |_: &(), source, _| {
        let ShaderCacheSource::Wgsl(source) = source else { panic!("expected WGSL") };
        let module = naga::front::wgsl::parse_str(&source)
            .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
            .validate(&module).unwrap_or_else(|e| panic!("{e:?}\n{source}"));
        Ok(source)
    })
}

fn sources(directory: &std::path::Path, package: &str, root: &std::path::Path, out: &mut Vec<(String, String)>) {
    for entry in std::fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() { sources(&path, package, root, out); }
        else if path.extension().is_some_and(|e| e == "wesl") {
            let relative = path.strip_prefix(root).unwrap().with_extension("").to_string_lossy().replace('\\', "/");
            out.push((format!("embedded://{package}/{relative}.wesl"), std::fs::read_to_string(path).unwrap()));
        }
    }
}

/// The vendored Bevy PBR/render modules, plus the fullscreen vertex IO type imported by
/// Bevy's SSR library (bevy_core_pipeline is not vendored; no tested entry point uses it).
fn vendored_modules() -> Vec<(String, String)> {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut modules = Vec::new();
    for package in ["bevy_pbr", "bevy_render"] {
        let root = manifest.join("../vendor").join(package).join("src");
        sources(&root, package, &root, &mut modules);
    }
    modules.push(("embedded://bevy_core_pipeline/fullscreen_vertex_shader.wesl".into(),
        "struct FullscreenVertexOutput { @builtin(position) position: vec4<f32>, @location(0) uv: vec2<f32> }".into()));
    modules
}

/// Composes `entry` from `modules` with `defs` (naga-validated), and returns its source.
fn compose(modules: &[(String, String)], entry: &str, defs: &[ShaderDefVal]) -> String {
    let mut cache = validating_cache();
    let mut assets = Assets::<Shader>::default();
    let mut root = None;
    for (path, source) in modules {
        let shader = Shader::from_wesl(source.clone(), path.clone());
        let handle = assets.add(shader.clone());
        cache.set_shader(handle.id(), shader);
        if path == entry { root = Some(handle); }
    }
    cache.get(0, root.unwrap_or_else(|| panic!("no module {entry}")).id(), defs)
        .map(std::sync::Arc::unwrap_or_clone).unwrap_or_else(|e| panic!("{entry} {defs:?}: {e:?}"))
}

/// The (group, binding) of every sampled texture (plain or binding array) the composed
/// module declares. Composition keeps only what the entry point reaches, so this is what
/// the shader uses, a subset of its pipeline layout (which Rust's AsBindGroup makes).
fn texture_bindings(source: &str) -> std::collections::BTreeSet<(u32, u32)> {
    let module = naga::front::wgsl::parse_str(source).unwrap();
    let mut bindings = std::collections::BTreeSet::new();
    for (_, variable) in module.global_variables.iter() {
        let Some(binding) = &variable.binding else { continue };
        let inner = match &module.types[variable.ty].inner {
            naga::TypeInner::BindingArray { base, .. } => &module.types[*base].inner,
            inner => inner,
        };
        if matches!(inner, naga::TypeInner::Image { class: naga::ImageClass::Sampled { .. } | naga::ImageClass::Depth { .. }, .. }) {
            bindings.insert((binding.group, binding.binding));
        }
    }
    bindings
}

#[test]
fn composed_production_blend_module_validates_with_key_on_and_off() {
    for bindless in [false, true] {
        for blend in [false, true] {
            let mut modules: Vec<(String, String)> = vec![
                ("embedded://bevy_pbr/render/mesh_bindings.wesl".into(), "struct Mesh { material_and_lightmap_bind_group_slot: u32 } @group(2) @binding(0) var<storage> mesh: array<Mesh>;".into()),
                ("embedded://bevy_pbr/render/pbr_types.wesl".into(), "struct StandardMaterial { base_color: vec4<f32> }".into()),
                ("embedded://bevy_pbr/render/pbr_bindings.wesl".into(), r#"
                    import bevy_pbr::render::pbr_types::StandardMaterial;
                    struct Indices { material: u32, base_color_texture: u32, base_color_sampler: u32, normal_map_texture: u32, normal_map_sampler: u32 }
                    @if(BINDLESS) @group(3) @binding(0) var<storage> material_indices: array<Indices>;
                    @if(BINDLESS) @group(3) @binding(1) var<storage> material_array: array<StandardMaterial>;
                    @if(!BINDLESS) @group(3) @binding(0) var<uniform> material: StandardMaterial;
                    @if(!BINDLESS) @group(3) @binding(1) var base_color_texture: texture_2d<f32>;
                    @if(!BINDLESS) @group(3) @binding(2) var base_color_sampler: sampler;
                    @if(!BINDLESS) @group(3) @binding(3) var normal_map_texture: texture_2d<f32>;
                    @if(!BINDLESS) @group(3) @binding(4) var normal_map_sampler: sampler;
                "#.into()),
                // As engine/vendor/bevy_render/src/bindless.wesl: binding arrays need the
                // enable directive.
                ("embedded://bevy_render/bindless.wesl".into(), r#"
                    @if(BINDLESS)
                    enable wgpu_binding_array;
                    @if(BINDLESS) @group(3) @binding(10) var bindless_textures_2d: binding_array<texture_2d<f32>>;
                    @if(BINDLESS) @group(3) @binding(11) var bindless_samplers_filtering: binding_array<sampler>;
                "#.into()),
                ("embedded://nvf_assets/nif_surface_bindings.wesl".into(), include_str!("../../nvf-assets/src/nif_surface_bindings.wesl").into()),
            ];
            modules.push(("embedded://nvforever/blend_validation.wesl".into(), r#"
                import nvf_assets::nif_surface_bindings::{nif_surface, holes_alpha, facegen_size, facegen_texel};
                @if(MESH_TERRAIN_BLEND) import nvf_assets::nif_surface_bindings::{mesh_terrain_lookup, mesh_terrain_diagnostic, mesh_terrain_view, mesh_terrain_shading, mesh_terrain_blend_normal, LAND_ROUGHNESS, LAND_REFLECTANCE};
                @fragment fn fragment(@location(0) world: vec3<f32>, @location(1) normal: vec3<f32>) -> @location(0) vec4<f32> {
                    var colour = nif_surface(0u).falloff.xyz * holes_alpha(0u, world.xy) + facegen_texel(0u, vec2<i32>(facegen_size(0u)));
                    @if(MESH_TERRAIN_BLEND) {
                        let t = mesh_terrain_lookup(0u, world, normal);
                        colour = mix(colour, t.colour, t.weight) + mesh_terrain_blend_normal(normal, t.normal, t.weight) * LAND_ROUGHNESS * LAND_REFLECTANCE;
                        if mesh_terrain_shading(0u) == 0u { colour = mesh_terrain_diagnostic(mesh_terrain_view(0u), t, colour); }
                    }
                    return vec4(colour, 1.0);
                }
            "#.into()));
            let mut defs = vec![
                ShaderDefVal::Bool("BINDLESS".into(), bindless),
                ShaderDefVal::Bool("NIF_SURFACE_BLEND".into(), blend),
                ShaderDefVal::Bool("MESH_TERRAIN_BLEND".into(), blend),
                ShaderDefVal::UInt("MATERIAL_BIND_GROUP".into(), 3),
            ];
            if blend { defs.extend(nvf_assets::nif_surface_blend::blend_shader_defs()); }
            let source = compose(&modules, "embedded://nvforever/blend_validation.wesl", &defs);
            let module = naga::front::wgsl::parse_str(&source).unwrap();
            let mut surface_sizes = Vec::new();
            let mut index_sizes = Vec::new();
            for (_, ty) in module.types.iter() {
                if let naga::TypeInner::Struct { ref members, span } = ty.inner {
                    if members.first().is_some_and(|m| m.name.as_deref() == Some("falloff")) { surface_sizes.push(span); }
                    if members.first().is_some_and(|m| m.name.as_deref() == Some("surface")) { index_sizes.push(span); }
                }
            }
            assert_eq!(surface_sizes, vec![if blend { 160 } else { 128 }]);
            // The blend type reuses the ordinary table and slots (nif_surface_blend.rs notes).
            if bindless { assert_eq!(index_sizes, vec![16]); }
            if !bindless {
                let bound = |binding| module.global_variables.iter().any(|(_, v)| v.binding.as_ref().is_some_and(|b| b.group == 3 && b.binding == binding));
                for binding in [50, 51, 52, 53] { assert!(bound(binding), "bindless={bindless} blend={blend} binding {binding}"); }
                for binding in [54, 55, 56] { assert!(!bound(binding), "no binding past main's 53"); }
                let extension: Vec<_> = texture_bindings(&source).into_iter().filter(|&(g, b)| g == 3 && b >= 50).collect();
                assert_eq!(extension, vec![(3, 51), (3, 53)], "bindless={bindless} blend={blend}: textures only in slots 51 and 53");
            }
        }
    }
    let material = include_str!("../../nvf-assets/src/nif_material.wesl");
    assert!(material.contains("let terrain = mesh_terrain_lookup(in.instance_index, in.world_position.xyz, pbr_input.world_normal);"), "the weight uses the unperturbed normal");
    assert!(material.find("let terrain = mesh_terrain_lookup(").unwrap() > material.find("alpha_discard(pbr_input.material.flags").unwrap(), "after the coverage tests");
    assert!(material.find("let terrain = mesh_terrain_lookup(").unwrap() < material.find("out.color = apply_pbr_lighting(pbr_input);").unwrap(), "before lighting");
}

fn nif_defs(bindless: bool, blend: bool, prepass: bool, soft_edges: bool) -> Vec<ShaderDefVal> {
    let mut defs = vec![
        ShaderDefVal::Bool("BINDLESS".into(), bindless),
        ShaderDefVal::Bool("NIF_SURFACE_BLEND".into(), blend),
        ShaderDefVal::Bool("MESH_TERRAIN_BLEND".into(), blend && !prepass),
        ShaderDefVal::Bool("PREPASS_PIPELINE".into(), prepass),
        ShaderDefVal::Bool("SOFT_EDGES".into(), soft_edges),
        "VERTEX_POSITIONS".into(), "VERTEX_NORMALS".into(), "VERTEX_UVS_A".into(),
        "VERTEX_TANGENTS".into(), "VERTEX_COLORS".into(), "VERTEX_OUTPUT_INSTANCE_INDEX".into(), "MAY_DISCARD".into(),
        "AVAILABLE_STORAGE_BUFFER_BINDINGS__GE_3".into(), "AVAILABLE_STORAGE_BUFFER_BINDINGS__GE_6".into(),
        ShaderDefVal::UInt("MATERIAL_BIND_GROUP".into(), 3),
        ShaderDefVal::UInt("MAX_DIRECTIONAL_LIGHTS".into(), 10),
        ShaderDefVal::UInt("MAX_CASCADES_PER_LIGHT".into(), 4),
        ShaderDefVal::UInt("MAX_RECT_LIGHTS".into(), 1),
    ];
    if blend && !prepass { defs.extend(nvf_assets::nif_surface_blend::blend_shader_defs()); }
    defs
}

#[test]
fn complete_nif_forward_and_prepass_compose_with_both_interfaces() {
    // [design] Use the real WESL compiler through Bevy's CPU ShaderCache, replacing
    // only its GPU-module creation callback with naga parsing and validation.
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut modules = vendored_modules();
    let root = manifest.join("../nvf-assets/src");
    sources(&root, "nvf_assets", &root, &mut modules);
    for bindless in [false, true] { for blend in [false, true] { for prepass in [false, true] {
        let entry = if prepass { "embedded://nvf_assets/nif_prepass.wesl" } else { "embedded://nvf_assets/nif_material.wesl" };
        // SOFT_EDGES is a main-pass variant; the unlit diagnostics write its coverage.
        let soft: &[bool] = if prepass { &[false] } else { &[false, true] };
        for &soft_edges in soft {
            let blended = compose(&modules, entry, &nif_defs(bindless, blend, prepass, soft_edges));
            if blend && !bindless {
                // [design] Every texture the blend pipeline's shader uses is a binding of the
                // ordinary NIF pipeline's layout for the same key: the same view and mesh
                // bindings, and in the material group only StandardMaterial's and the
                // ordinary extension's slots 51 and 53 (which the ordinary shader may leave
                // unused, its layout still has them: nif_material.rs NifSurface).
                let mut ordinary = texture_bindings(&compose(&modules, entry, &nif_defs(bindless, false, prepass, soft_edges)));
                ordinary.extend([(3, 51), (3, 53)]);
                let blended = texture_bindings(&blended);
                assert!(blended.is_subset(&ordinary), "prepass={prepass} soft_edges={soft_edges}: {:?}", blended.difference(&ordinary).collect::<Vec<_>>());
            }
        }
    }}}
}

#[test]
fn terrain_surface_bake_composes_for_the_folded_base() {
    // [design] terrain.wesl with the ordinary and folded-bake variants (terrain.rs
    // TerrainLayersKey), through the same CPU ShaderCache and naga validation.
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut modules = vendored_modules();
    for name in ["terrain", "terrain_surface"] {
        modules.push((format!("embedded://nvforever/{name}.wesl"), std::fs::read_to_string(manifest.join("src").join(format!("{name}.wesl"))).unwrap()));
    }
    for bindless in [false, true] { for bake in [false, true] {
        let defs = vec![
            ShaderDefVal::Bool("BINDLESS".into(), bindless),
            ShaderDefVal::Bool("SURFACE_BAKE".into(), bake),
            "VERTEX_POSITIONS".into(), "VERTEX_NORMALS".into(), "VERTEX_UVS_A".into(), "VERTEX_UVS_B".into(),
            "VERTEX_TANGENTS".into(), "VERTEX_COLORS".into(), "VERTEX_OUTPUT_INSTANCE_INDEX".into(),
            "AVAILABLE_STORAGE_BUFFER_BINDINGS__GE_3".into(), "AVAILABLE_STORAGE_BUFFER_BINDINGS__GE_6".into(),
            ShaderDefVal::UInt("MATERIAL_BIND_GROUP".into(), 3),
            ShaderDefVal::UInt("MAX_DIRECTIONAL_LIGHTS".into(), 10),
            ShaderDefVal::UInt("MAX_CASCADES_PER_LIGHT".into(), 4),
            ShaderDefVal::UInt("MAX_RECT_LIGHTS".into(), 1),
        ];
        compose(&modules, "embedded://nvforever/terrain.wesl", &defs);
    }}
    assert!(!include_str!("terrain.wesl").contains("SURFACE_BAKE_LAYER"), "layer copies use the single-texture capture");
}

fn capture_source(colours: bool, uvs: bool) -> String {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut modules = vendored_modules();
    modules.push(("embedded://nvforever/terrain_capture.wesl".into(), std::fs::read_to_string(manifest.join("src/terrain_capture.wesl")).unwrap()));
    let defs = vec![
        ShaderDefVal::Bool("VERTEX_COLORS".into(), colours), ShaderDefVal::Bool("VERTEX_UVS_A".into(), uvs),
        "VERTEX_POSITIONS".into(), "VERTEX_NORMALS".into(), "VERTEX_TANGENTS".into(),
        ShaderDefVal::UInt("MATERIAL_BIND_GROUP".into(), 3),
    ];
    compose(&modules, "embedded://nvforever/terrain_capture.wesl", &defs)
}

#[test]
fn single_texture_capture_composes_for_base_and_layer_meshes() {
    // [design] The landscape's base and layer meshes carry colours and coordinates
    // (terrain.rs quadrant_mesh); the variants without them must still compose.
    for colours in [false, true] { for uvs in [false, true] {
        let source = capture_source(colours, uvs);
        let textures = texture_bindings(&source);
        assert!(textures.iter().all(|&(group, binding)| group == 3 && binding == 1), "{textures:?}: only the diffuse texture");
        assert_eq!(textures.len(), usize::from(uvs), "sampled where the mesh has coordinates");
    }}
    let capture = include_str!("terrain_capture.wesl");
    // Base: alpha 1; layer: material x vertex x diffuse alpha. Missing texture: white.
    assert!(capture.contains("out.color = vec4(colour.rgb, select(1.0, colour.a, surface_capture.uv_offset_flags.w > 0.5));"));
    assert!(capture.contains("colour *= select(vec4(1.0), texel, surface_capture.uv_offset_flags.z > 0.5);"));
    assert!(capture.find("colour *= in.color;").unwrap() < capture.find("out.color =").unwrap());
}

/// [design] The 16-texture stage limit of a GPU without bindless textures, against the
/// material groups the aid creates (from the production types' resource tables, where
/// every texture becomes a binding without bindless, and the composed capture shader).
/// The view and mesh groups are the mesh pipeline's, as for every ordinary material:
/// the blend type equals the ordinary NIF type's count (also per composed group in
/// `complete_nif_forward_and_prepass_compose_with_both_interfaces`); the capture binds
/// one texture; the folded TerrainMaterial's group alone exceeds the limit, so it is used
/// only where TerrainMaterials::folds found bindless textures, and every other draw is
/// captured with the single-texture material (terrain_surface.rs tests).
#[test]
fn material_groups_fit_a_16_texture_stage_without_bindless() {
    use bevy::render::render_resource::{AsBindGroup, BindlessDescriptor, BindlessResourceType as R};
    const STAGE_LIMIT: usize = 16;
    let textures = |d: Option<BindlessDescriptor>| d.unwrap().resources.iter().filter(|r| matches!(r, R::Texture1d | R::Texture2d | R::Texture2dArray | R::Texture3d | R::TextureCube | R::TextureCubeArray)).count();
    let standard = textures(StandardMaterial::bindless_descriptor());
    let ordinary = standard + textures(nvf_assets::nif_material::NifSurface::bindless_descriptor());
    let blend = standard + textures(nvf_assets::nif_surface_blend::NifSurfaceBlend::bindless_descriptor());
    let folded = standard + textures(crate::terrain::TerrainLayers::bindless_descriptor());
    assert_eq!(blend, ordinary, "the blend adds no texture to the ordinary NIF material group");
    assert!(ordinary <= STAGE_LIMIT);
    assert!(folded > STAGE_LIMIT, "{folded}: the folded material needs bindless textures");
    let capture = texture_bindings(&capture_source(true, true)).len();
    assert_eq!(capture, 1);
    assert!(capture < ordinary, "the capture's group is smaller than any ordinary NIF shape's");
}
