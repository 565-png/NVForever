//! [design] CPU-only WESL composition and naga validation of the production binding/
//! blend module. Minimal mesh/PBR declarations stand in for the unrelated renderer.
//! A second test composes the complete forward and NIF depth-prepass shaders using
//! repository-vendored modules. Neither test executes fragments or creates GPU pipelines.

use bevy::prelude::*;
use bevy::shader::{ShaderCache, ShaderCacheSource, ShaderDefVal};
use wgpu::naga;

#[test]
fn composed_production_blend_module_validates_with_key_on_and_off() {
    for bindless in [false, true] {
        for blend in [false, true] {
            let mut cache = ShaderCache::new((), |_: &(), source, _| {
                let ShaderCacheSource::Wgsl(source) = source else { panic!("expected WGSL") };
                let module = naga::front::wgsl::parse_str(&source)
                    .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
                naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
                    .validate(&module).unwrap_or_else(|e| panic!("{e:?}\n{source}"));
                Ok(source)
            });
            let mut assets = Assets::<Shader>::default();
            let mut add = |path: &str, source: &'static str| {
                let shader = Shader::from_wesl(source, format!("embedded://{path}.wesl"));
                let handle = assets.add(shader.clone());
                cache.set_shader(handle.id(), shader);
                handle
            };
            add("bevy_pbr/render/mesh_bindings", "struct Mesh { material_and_lightmap_bind_group_slot: u32 } @group(2) @binding(0) var<storage> mesh: array<Mesh>;");
            add("bevy_pbr/render/pbr_types", "struct StandardMaterial { base_color: vec4<f32> }");
            add("bevy_pbr/render/pbr_bindings", r#"
                import bevy_pbr::render::pbr_types::StandardMaterial;
                struct Indices { material: u32, base_color_texture: u32, base_color_sampler: u32, normal_map_texture: u32, normal_map_sampler: u32 }
                @if(BINDLESS) @group(3) @binding(0) var<storage> material_indices: array<Indices>;
                @if(BINDLESS) @group(3) @binding(1) var<storage> material_array: array<StandardMaterial>;
                @if(!BINDLESS) @group(3) @binding(0) var<uniform> material: StandardMaterial;
                @if(!BINDLESS) @group(3) @binding(1) var base_color_texture: texture_2d<f32>;
                @if(!BINDLESS) @group(3) @binding(2) var base_color_sampler: sampler;
                @if(!BINDLESS) @group(3) @binding(3) var normal_map_texture: texture_2d<f32>;
                @if(!BINDLESS) @group(3) @binding(4) var normal_map_sampler: sampler;
            "#);
            add("bevy_render/bindless", r#"
                @group(3) @binding(10) var bindless_textures_2d: binding_array<texture_2d<f32>>;
                @group(3) @binding(11) var bindless_samplers_filtering: binding_array<sampler>;
            "#);
            add("nvf_assets/nif_surface_bindings", include_str!("../../nvf-assets/src/nif_surface_bindings.wesl"));
            let root = add("nvforever/blend_validation", r#"
                import nvf_assets::nif_surface_bindings::nif_surface;
                @if(MESH_TERRAIN_BLEND) import nvf_assets::nif_surface_bindings::mesh_terrain_blend;
                @fragment fn fragment(@location(0) world: vec3<f32>) -> @location(0) vec4<f32> {
                    var colour = nif_surface(0u).falloff.xyz;
                    @if(MESH_TERRAIN_BLEND) colour = mesh_terrain_blend(0u, world, colour);
                    return vec4(colour, 1.0);
                }
            "#);
            let mut defs = vec![
                ShaderDefVal::Bool("BINDLESS".into(), bindless),
                ShaderDefVal::Bool("NIF_SURFACE_BLEND".into(), blend),
                ShaderDefVal::Bool("MESH_TERRAIN_BLEND".into(), blend),
                ShaderDefVal::UInt("MATERIAL_BIND_GROUP".into(), 3),
            ];
            if blend { defs.extend(nvf_assets::nif_surface_blend::blend_shader_defs()); }
            let source = cache.get(0, root.id(), &defs)
                .unwrap_or_else(|e| panic!("bindless={bindless} blend={blend}: {e:?}"));
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
            if bindless { assert_eq!(index_sizes, vec![if blend { 28 } else { 16 }]); }
            if !bindless {
                for binding in [54, 55, 56] {
                    assert_eq!(module.global_variables.iter().any(|(_, v)| v.binding.as_ref().is_some_and(|b| b.group == 3 && b.binding == binding)), blend);
                }
            }
        }
    }
    assert!(include_str!("../../nvf-assets/src/nif_material.wesl").contains("@if(MESH_TERRAIN_BLEND && !PREPASS_PIPELINE) pbr_input.material.base_color = vec4(mesh_terrain_blend("));
}

#[test]
fn complete_nif_forward_and_prepass_compose_with_both_interfaces() {
    // [design] Use the real WESL compiler through Bevy's CPU ShaderCache, replacing
    // only its GPU-module creation callback with naga parsing and validation.
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
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut modules = Vec::new();
    for package in ["bevy_pbr", "bevy_render"] {
        let root = manifest.join("../vendor").join(package).join("src");
        sources(&root, package, &root, &mut modules);
    }
    let root = manifest.join("../nvf-assets/src");
    sources(&root, "nvf_assets", &root, &mut modules);
    // The fullscreen vertex IO type is imported by Bevy's SSR library; it is not
    // used by a NIF entry point. bevy_core_pipeline is not vendored in this repo.
    modules.push(("embedded://bevy_core_pipeline/fullscreen_vertex_shader.wesl".into(),
        "struct FullscreenVertexOutput { @builtin(position) position: vec4<f32>, @location(0) uv: vec2<f32> }".into()));
    for bindless in [false, true] { for blend in [false, true] { for prepass in [false, true] {
        let mut cache = ShaderCache::new((), |_: &(), source, _| {
            let ShaderCacheSource::Wgsl(source) = source else { panic!("expected WGSL") };
            let module = naga::front::wgsl::parse_str(&source)
                .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
            naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
                .validate(&module).unwrap_or_else(|e| panic!("{e:?}\n{source}"));
            Ok(source)
        });
        let mut assets = Assets::<Shader>::default();
        let mut entry = None;
        let entry_path = if prepass { "embedded://nvf_assets/nif_prepass.wesl" } else { "embedded://nvf_assets/nif_material.wesl" };
        for (path, source) in &modules {
            let shader = Shader::from_wesl(source.clone(), path.clone());
            let handle = assets.add(shader.clone());
            cache.set_shader(handle.id(), shader);
            if path == entry_path { entry = Some(handle); }
        }
        let mut defs = vec![
            ShaderDefVal::Bool("BINDLESS".into(), bindless),
            ShaderDefVal::Bool("NIF_SURFACE_BLEND".into(), blend),
            ShaderDefVal::Bool("MESH_TERRAIN_BLEND".into(), blend && !prepass),
            ShaderDefVal::Bool("PREPASS_PIPELINE".into(), prepass),
            "VERTEX_POSITIONS".into(), "VERTEX_NORMALS".into(), "VERTEX_UVS_A".into(),
            "VERTEX_TANGENTS".into(), "VERTEX_COLORS".into(), "VERTEX_OUTPUT_INSTANCE_INDEX".into(), "MAY_DISCARD".into(),
            "AVAILABLE_STORAGE_BUFFER_BINDINGS__GE_3".into(), "AVAILABLE_STORAGE_BUFFER_BINDINGS__GE_6".into(),
            ShaderDefVal::UInt("MATERIAL_BIND_GROUP".into(), 3),
            ShaderDefVal::UInt("MAX_DIRECTIONAL_LIGHTS".into(), 10),
            ShaderDefVal::UInt("MAX_CASCADES_PER_LIGHT".into(), 4),
            ShaderDefVal::UInt("MAX_RECT_LIGHTS".into(), 1),
        ];
        if blend && !prepass { defs.extend(nvf_assets::nif_surface_blend::blend_shader_defs()); }
        cache.get(0, entry.unwrap().id(), &defs)
            .unwrap_or_else(|e| panic!("bindless={bindless} blend={blend} prepass={prepass}: {e:?}"));
    }}}
}
