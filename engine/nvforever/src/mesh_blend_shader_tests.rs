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
                // Stand-ins with the signatures of Bevy's TBN and normal-map helpers (the
                // complete composition below uses the vendored ones).
                ("embedded://bevy_pbr/render/pbr_functions.wesl".into(), r#"
                    fn calculate_tbn_mikktspace(world_normal: vec3<f32>, world_tangent: vec4<f32>) -> mat3x3<f32> {
                        return mat3x3(world_tangent.xyz, world_tangent.w * cross(world_normal, world_tangent.xyz), world_normal);
                    }
                    fn apply_normal_mapping(flags: u32, tbn: mat3x3<f32>, double_sided: bool, is_front: bool, nt: vec3<f32>) -> vec3<f32> {
                        return normalize(tbn * (nt * 2.0 - 1.0) * f32(flags + 1u));
                    }
                "#.into()),
            ];
            modules.push(("embedded://nvforever/blend_validation.wesl".into(), r#"
                import nvf_assets::nif_surface_bindings::{nif_surface, holes_alpha, facegen_size, facegen_texel};
                @if(MESH_TERRAIN_BLEND) import nvf_assets::nif_surface_bindings::{mesh_terrain_lookup, mesh_terrain_diagnostic, mesh_terrain_view, mesh_terrain_shading, mesh_terrain_blend_normal, mesh_terrain_target_colour, mesh_terrain_target_normal, MeshTerrainInputs, LAND_ROUGHNESS, LAND_REFLECTANCE};
                @fragment fn fragment(@location(0) world: vec3<f32>, @location(1) normal: vec3<f32>) -> @location(0) vec4<f32> {
                    var colour = nif_surface(0u).falloff.xyz * holes_alpha(0u, world.xy) + facegen_texel(0u, vec2<i32>(facegen_size(0u)));
                    @if(MESH_TERRAIN_BLEND) {
                        let xy = vec2(world.x, -world.z);
                        let inputs = MeshTerrainInputs(length(world), dpdx(xy), dpdy(xy), 0.0, mesh_terrain_shading(0u));
                        let t = mesh_terrain_lookup(0u, world, normal, inputs);
                        colour = mix(colour, mesh_terrain_target_colour(t), t.weight) + mesh_terrain_blend_normal(normal, mesh_terrain_target_normal(t), t.weight) * LAND_ROUGHNESS * LAND_REFLECTANCE;
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
            if blend {
                assert!(source.contains("terrain_covered"), "enabled coverage survives bindless and ordinary composition");
                assert!(source.contains("mesh_terrain_origin.z"), "coverage reuses the enabled uniform without another binding");
                // Round 10a: the layer detail only where the palette exists (bindless).
                assert_eq!(source.contains("textureSampleGrad"), bindless, "bindless={bindless}: detail reads with explicit gradients, none without bindless");
            }
            // Round 9 reuses origin.z; round 6's enabled width and ordinary layouts stay fixed.
            assert_eq!(surface_sizes, vec![if blend { 176 } else { 128 }]);
            // The blend type reuses the ordinary table and slots (nif_surface_blend.rs notes);
            // round 10a's bindless blend row is longer by the palette's 400 entries.
            if bindless { assert_eq!(index_sizes, vec![if blend { 16 + 4 * nvf_assets::nif_surface_blend::PALETTE_ENTRIES as u32 } else { 16 }]); }
            assert_eq!(nvf_assets::nif_surface_blend::PALETTE_ENTRIES, 400, "nif_surface_bindings.wesl spells out array<u32, 400>");
            if !bindless {
                let bound = |binding| module.global_variables.iter().any(|(_, v)| v.binding.as_ref().is_some_and(|b| b.group == 3 && b.binding == binding));
                // Round 10b review: the blend reads its colour atlas with textureLoad (weighted
                // in the shader), so it need not reach its sampler (52, still in the layout
                // AsBindGroup makes); the ordinary type's holes sampler is reached.
                for binding in [50, 51, 53] { assert!(bound(binding), "bindless={bindless} blend={blend} binding {binding}"); }
                if !blend { assert!(bound(52), "bindless={bindless} blend={blend} binding 52"); }
                for binding in [54, 55, 56] { assert!(!bound(binding), "no binding past main's 53"); }
                let extension: Vec<_> = texture_bindings(&source).into_iter().filter(|&(g, b)| g == 3 && b >= 50).collect();
                assert_eq!(extension, vec![(3, 51), (3, 53)], "bindless={bindless} blend={blend}: textures only in slots 51 and 53");
            }
        }
    }
    let material = include_str!("../../nvf-assets/src/nif_material.wesl");
    assert!(material.contains("let terrain = mesh_terrain_lookup(in.instance_index, in.world_position.xyz, pbr_input.world_normal, terrain_inputs);"), "the weight uses the unperturbed normal");
    assert!(material.find("let terrain = mesh_terrain_lookup(").unwrap() > material.find("alpha_discard(pbr_input.material.flags").unwrap(), "after the coverage tests");
    assert!(material.find("let terrain = mesh_terrain_lookup(").unwrap() < material.find("out.color = apply_pbr_lighting(pbr_input);").unwrap(), "before lighting");
    // [design] Round 10a: the game-xy derivatives are taken before the lookup, at the
    // function's top level (uniform control flow), never inside its branches.
    let inputs = material.find("let terrain_inputs = MeshTerrainInputs(distance(view.world_position, in.world_position.xyz), dpdx(terrain_xy), dpdy(terrain_xy), view.mip_bias,").expect("the derivatives");
    assert!(inputs < material.find("let terrain = mesh_terrain_lookup(").unwrap());
    let line_start = material[..inputs].rfind('\n').unwrap();
    assert_eq!(material[line_start + 1..inputs].trim(), "@if(MESH_TERRAIN_BLEND && !PREPASS_PIPELINE)", "a top-level statement, not inside a branch");
    // [design] Zero weight leaves every shading input as it was: the surface blend sits
    // inside the positive-weight branch, which hybrid v2 keeps.
    let branch = material.find("} else if terrain.status == LOOKUP_SUCCESS && terrain.weight > 0.0 {").expect("the positive-weight branch");
    for input in ["pbr_input.material.base_color = vec4(mix(pbr_input.material.base_color.rgb, mesh_terrain_target_colour(terrain), w)", "pbr_input.N = mesh_terrain_blend_normal(pbr_input.N, mesh_terrain_target_normal(terrain), w)", "pbr_input.material.perceptual_roughness = mix(", "pbr_input.material.emissive = vec4(pbr_input.material.emissive.rgb * (1.0 - w)"] {
        assert!(material.find(input).unwrap() > branch, "{input}");
    }
    // [design] The layer detail is read only inside the lookup's positive-weight, positive-
    // share branch, with explicit gradients from the inputs, and normal maps only for full
    // shading; never from the screen-space prepass normal.
    let bindings = include_str!("../../nvf-assets/src/nif_surface_bindings.wesl").replace("\r\n", "\n");
    let positive = bindings.find("    if r.weight > 0.0 {\n        if colour_valid { r.colour = terrain_colour_mips(instance_index, slot, uv, resolution, levels); }").expect("the positive-weight branch");
    assert!(bindings.find("@if(BINDLESS) if r.detail > 0.0 {").unwrap() > positive);
    assert!(bindings.contains("let ddx = axes * inputs.ddx * scale;") && bindings.contains("let scale = exp2(inputs.bias) / tile;"));
    assert!(bindings.contains("if full && (flags & 8u) != 0u {") && bindings.contains("        if full {\n            var layer_n = plain;"));
    assert!(!bindings.contains("prepass_normal") && !bindings.contains("prepass_utils"), "the target never comes from the prepass normal");
    let detail = bindings.split("fn mesh_terrain_detail(").nth(1).unwrap().split("\n}\n").next().unwrap();
    let sample = bindings.split("fn detail_sample(").nth(1).unwrap().split("\n}\n").next().unwrap();
    for body in [detail, sample] {
        assert!(!body.contains("textureSample(") && !body.contains("textureSampleBias(") && !body.contains("dpdx") && !body.contains("fwidth"), "no implicit derivatives in the divergent detail branch");
    }
    assert!(sample.contains("textureSampleGrad("));
    // [design] Round 10b: the colour cache is read at explicit levels chosen from the same
    // derivatives (no implicit derivatives inside the lookup's branches) and both
    // contributing levels' footprints validated. Round 10b review: each level is the
    // bilinear footprint at the fragment's position across valid neighbouring cells, its
    // taps loaded and weighted in the shader, unreadable ones given no weight and the rest
    // renormalized (terrain_surface.rs `colour_read_taps`).
    for name in ["terrain_colour_tap_valid", "terrain_colour_taps", "terrain_colour_level", "terrain_colour_mips", "terrain_colour_lod", "terrain_colour_level_valid", "terrain_colour_footprint"] {
        let body = bindings.split(&format!("fn {name}(")).nth(1).unwrap_or_else(|| panic!("{name}")).split("\n}\n").next().unwrap();
        assert!(!body.contains("textureSample") && !body.contains("dpdx") && !body.contains("fwidth"), "{name}: no implicit derivatives, no sampler");
    }
    assert!(bindings.contains("let levels = terrain_colour_levels(resolution, terrain_colour_lod(resolution, inputs));"));
    assert!(bindings.contains("let colour_valid = colour_quadrant && terrain_colour_footprint(instance_index, slot, uv, resolution, levels);"));
    assert!(bindings.contains("let footprint = max(length(inputs.ddx), length(inputs.ddy)) * f32(resolution) / CELL_SIZE;")
        && bindings.contains("return clamp(log2(max(footprint, 1e-6)) + inputs.bias, 0.0, f32(terrain_last_level(resolution)));"));
    let taps = bindings.split("fn terrain_colour_taps(").nth(1).unwrap().split("\n}\n").next().unwrap();
    for line in [
        "let p = uv * f32(3 * size) - vec2(0.5);",
        "var weights = vec4((1.0 - f.x) * (1.0 - f.y), f.x * (1.0 - f.y), (1.0 - f.x) * f.y, f.x * f.y);",
        "if !valid { weights[i] = 0.0; }",
        "return TerrainColourTaps(first, weights / max(total, 1e-6), has_own && own_valid && total > 0.0);",
    ] { assert!(taps.contains(line), "{line}"); }
    assert!(!taps.contains("clamp("), "the footprint is not clamped into the cell");
    assert!(bindings.contains("if t.weights[i] > 0.0 { colour += t.weights[i] * terrain_colour_load(instance_index, t.first + vec2(i % 2, i / 2), level); }"));
    assert!(bindings.contains("return l.share <= 0.0 || l.high == l.low || terrain_colour_level_valid(instance_index, slot, uv, resolution, l.high);"));
}

/// [design] Round 10b: the colour atlas's mip filter (terrain_surface.rs `MipPipeline`)
/// parses and validates with naga, has the bindings the pipeline's layout gives it (a
/// sampled level, a write-only RGBA16F storage level, the cells' uniform) and averages the
/// 2 by 2 texels under each texel of the cells it is given, nothing else.
#[test]
fn colour_mip_filter_validates_and_reads_only_its_cell() {
    let source = crate::terrain_surface::MIP_SHADER;
    let module = naga::front::wgsl::parse_str(source).unwrap_or_else(|e| panic!("{}", e.emit_to_string(source)));
    naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
        .validate(&module).unwrap_or_else(|e| panic!("{e:?}"));
    let entry = module.entry_points.iter().find(|e| e.name == "downsample").expect("the entry point");
    assert_eq!(entry.stage, naga::ShaderStage::Compute);
    assert_eq!(entry.workgroup_size, [crate::terrain_surface::MIP_WORKGROUP, crate::terrain_surface::MIP_WORKGROUP, 1]);
    let mut bindings: Vec<_> = module.global_variables.iter().filter_map(|(_, v)| v.binding.as_ref().map(|b| (b.group, b.binding, module.types[v.ty].inner.clone()))).collect();
    bindings.sort_by_key(|b| b.1);
    assert_eq!(bindings.iter().map(|b| (b.0, b.1)).collect::<Vec<_>>(), vec![(0, 0), (0, 1), (0, 2)]);
    assert!(matches!(bindings[0].2, naga::TypeInner::Image { class: naga::ImageClass::Sampled { .. }, .. }));
    assert!(matches!(bindings[1].2, naga::TypeInner::Image { class: naga::ImageClass::Storage { format: naga::StorageFormat::Rgba16Float, access }, .. } if access == naga::StorageAccess::STORE));
    for line in [
        "let side = textureDimensions(destination).x / 3u;",
        "let texel = cells.slots[id.z].xy * side + id.xy;",
        "let below = 2u * texel;",
        "textureStore(destination, texel, sum * 0.25);",
    ] {
        assert!(source.contains(line), "{line}");
    }
}

/// [design] Round 10a: the shader's barycentric interpolation and record layout transcribe
/// terrain.rs's mesh triangles and nif_surface_blend.rs's packing.
#[test]
fn shader_detail_transcribes_the_mesh_triangles_and_the_record_layout() {
    let bindings = include_str!("../../nvf-assets/src/nif_surface_bindings.wesl");
    for line in [
        // Below the diagonal (SW, SE, NE), above it (SW, NE, NW), as quadrant_mesh's indices.
        "var vertices = vec3(sw, sw + 1u, sw + side + 1u);",
        "var weights = vec3(1.0 - f.x, f.x - f.y, f.y);",
        "vertices = vec3(sw, sw + side + 1u, sw + side);",
        "weights = vec3(1.0 - f.y, f.x, f.y - f.x);",
        // Records: colour (unorm), normal and tangent (snorm), then opacities from value 9.
        "m.tint += t.weights[i] * vec3(detail_unorm(w0), detail_unorm(w0 >> 16u), detail_unorm(w1));",
        "m.normal += t.weights[i] * vec3(detail_snorm(w1 >> 16u), detail_snorm(w2), detail_snorm(w2 >> 16u));",
        "m.tangent += t.weights[i] * vec3(detail_snorm(w3), detail_snorm(w3 >> 16u), detail_snorm(w4));",
        "let h = 9u + k;",
        // Ordered opacities: saturate(opacity) x the diffuse alpha, folded overlays tinted by
        // the base material's colour, overflow ones by white.
        "let share = saturate(detail_opacity(instance_index, t, k)) * texel.a;",
        "colour = mix(colour, texel.rgb * a.tint * select(vec3(1.0), base_rgb, k < folded), share);",
        "normal = mix(normal, layer_n, share);",
        "return TerrainDetail(colour, normalize(normal));",
        // Table words.
        "let total = min((flags >> 12u) & 15u, DETAIL_MAX_OVERLAYS);",
        "let folded = min((flags >> 8u) & 15u, total);",
        "let pair = terrain_detail_word(instance_index, q + 15u + k);",
        "let handedness = bitcast<f32>(terrain_detail_word(instance_index, q + 27u));",
        "let local = select(place, place - vec2<f32>(cell) * CELL_SIZE, (flags & 2u) != 0u);",
        // Bevy's helpers, the interpolated (not renormalized) normal and tangent.
        "let tbn = calculate_tbn_mikktspace(world_n, vec4(game_to_world_normal(a.tangent), handedness));",
        "@if(NIF_SURFACE_BLEND) palette: array<u32, 400>, // 54-453",
    ] {
        assert!(bindings.contains(line), "{line}");
    }
    use nvf_assets::nif_surface_blend::*;
    let defs = blend_shader_defs();
    let uint = |name: &str| defs.iter().find_map(|d| match d { ShaderDefVal::UInt(n, v) if n == name => Some(*v), _ => None }).unwrap_or_else(|| panic!("{name}"));
    assert_eq!(uint("MESH_TERRAIN_DETAIL_ROW") as usize, HEIGHT_ROWS);
    assert_eq!(uint("MESH_TERRAIN_HEIGHT_WIDTH") as usize, HEIGHT_WIDTH);
    assert_eq!(uint("MESH_TERRAIN_DETAIL_VERTEX_WORDS") as usize, DETAIL_VERTEX_WORDS);
    assert_eq!(uint("MESH_TERRAIN_DETAIL_QUADRANT_WORDS") as usize, DETAIL_QUADRANT_WORDS);
    assert_eq!(uint("MESH_TERRAIN_PALETTE_IMAGES") as usize, PALETTE_IMAGES);
    assert_eq!((uint("MESH_TERRAIN_DETAIL_NEAR"), uint("MESH_TERRAIN_DETAIL_FAR")), (2048, 4096));
    assert_eq!(uint("MESH_TERRAIN_FLAG_TWO_COMPONENT"), bevy::pbr::StandardMaterialFlags::TWO_COMPONENT_NORMAL_MAP.bits());
    assert_eq!(uint("MESH_TERRAIN_FLAG_FLIP_Y"), bevy::pbr::StandardMaterialFlags::FLIP_NORMAL_MAP_Y.bits());
}

/// [design] The WESL weight transcribes nif_surface_blend.rs::contact_weight and v2_band:
/// stable shape numbers with hybrid v1 appended as 3, the widths read from the enabled
/// uniform as (flat, band, steep), and the shared cosine definitions.
#[test]
fn shader_weight_matches_the_cpu_statement_of_hybrid_v2() {
    let bindings = include_str!("../../nvf-assets/src/nif_surface_bindings.wesl").replace("\r\n", "\n");
    for line in [
        "let c = clamp(abs(cos_angle), 0.0, 1.0);",
        "let shallow = 1.0 - smoothstep(V2_COS_PARALLEL, 1.0, c);",
        "let steep = 1.0 - smoothstep(V2_COS_STEEP, COS_FULL, c);",
        "return widths.x + (widths.y - widths.x) * shallow + (widths.z - widths.y) * steep;",
        "if shape == 0u { return height; }",
        "if shape == 1u { return penetration; }",
        "if shape == 3u {",
        "return max(1.0 - smoothstep(0.0, band, max(d, 0.0)), penetration);",
        "let widths = vec3(s.mesh_terrain_widths.x, s.mesh_terrain.x, s.mesh_terrain_widths.y);",
        "@if(NIF_SURFACE_BLEND) mesh_terrain_widths: vec4<f32>,",
    ] {
        assert!(bindings.contains(line), "{line}");
    }
    use nvf_assets::nif_surface_blend::ContactShape;
    assert_eq!([ContactShape::Height, ContactShape::Penetration, ContactShape::Hybrid, ContactShape::HybridV1].map(|s| s.number()), [0.0, 1.0, 2.0, 3.0]);
    let defs = nvf_assets::nif_surface_blend::blend_shader_defs();
    for name in ["MESH_TERRAIN_V2_COS_PARALLEL_E6", "MESH_TERRAIN_V2_COS_STEEP_E6", "MESH_TERRAIN_COS_FULL_E6"] {
        assert!(defs.iter().any(|d| matches!(d, ShaderDefVal::UInt(n, _) if n == name)), "{name}");
        assert!(bindings.contains(&format!("constants::{name}")), "{name} reaches the shader");
    }
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
    // [design] Without bindless the blend type's layout is its colour interface's
    // (nif_surface_blend.rs: round 10a's palette exists only in the bindless table).
    let blend = standard + textures(nvf_assets::nif_surface_blend::NifSurfaceBlendColour::bindless_descriptor());
    let folded = standard + textures(crate::terrain::TerrainLayers::bindless_descriptor());
    assert_eq!(blend, ordinary, "the blend adds no texture to the ordinary NIF material group");
    assert!(ordinary <= STAGE_LIMIT);
    assert!(folded > STAGE_LIMIT, "{folded}: the folded material needs bindless textures");
    let capture = texture_bindings(&capture_source(true, true)).len();
    assert_eq!(capture, 1);
    assert!(capture < ordinary, "the capture's group is smaller than any ordinary NIF shape's");
}
