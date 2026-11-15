// The NifSurface extension's values and textures for one fragment, bindless or not
// (nif_material.rs "Bindless"; the Rust side: `NifSurface`, `NifSurfaceUniform`). Used by
// nif_material.wesl and nif_prepass.wesl.


import bevy_pbr::render::mesh_bindings::mesh;
@if(BINDLESS) import bevy_render::bindless::{bindless_textures_2d, bindless_samplers_filtering};
@if(BINDLESS) import bevy_pbr::render::pbr_bindings::{material_indices, material_array};
@if(BINDLESS) import bevy_pbr::render::pbr_types::StandardMaterial;
@if(!BINDLESS) import bevy_pbr::render::pbr_bindings;
@if(!BINDLESS) import bevy_pbr::render::pbr_types::StandardMaterial;

struct NifSurface {
    // x start angle (cosine), y stop angle (cosine), z start opacity, w stop opacity;
    // (1, 0, 1, 1) when the shape has no falloff.
    falloff: vec4<f32>,
    // Fragments with a lower alpha are not drawn (the alpha test of a shape that is
    // drawn blended); 0 on every other shape.
    alpha_cut: f32,
    // 1: an additive shape with source factor ONE, which adds its colour whole (only the
    // view-angle falloff fades it); 0 otherwise.
    add_whole: f32,
    // REFRACT: the screen shift (UV) per unit of the view-space normal (the strength ×
    // nif_material.rs REFRACTION_SCREEN); 0 otherwise.
    refraction: f32,
    // REFRACT: seconds for the normal map to scroll once up its texture (fire
    // refraction); 0: it does not move.
    fire_period: f32,
    // x 1 skips the image-space emissive scale; y holds each channel on the soft path
    // (0: no hold); z, w unused [hypothesis, nif.md §6 "Unlit shapes that are pictures,
    // and cut-out signs"]. Same fields in the same order as NifSurfaceUniform.
    emissive_rule: vec4<f32>,
    // Holes: x the value (alpha units, negative: no holes), y the edge band's width.
    holes: vec4<f32>,
    // The edge band's colour (linear).
    holes_edge: vec4<f32>,
    // Soft intersections: x the fade distance (game units), 0 for none.
    soft: vec4<f32>,
    // PIPBOY_SCREEN (nif_material.rs NifSurface::pipboy_a): x the roll up (share of the
    // canvas's height), y the wash (gain 1 - y on display values), z display value added,
    // w the canvas's share of the texture's height.
    pipboy_a: vec4<f32>,
    // PIPBOY_SCREEN (NifSurface::pipboy_b): x the softening's reach (view pixels), y the
    // noise's reach (display values), z the canvas's share of the texture's width; w unused.
    pipboy_b: vec4<f32>,
    // [design] The enabled blend type (nif_surface_blend.rs): band, view, contact shape,
    // shading; the cache's southwest cell in origin.xy.
    @if(NIF_SURFACE_BLEND) mesh_terrain: vec4<f32>,
    @if(NIF_SURFACE_BLEND) mesh_terrain_origin: vec4<f32>,
}

// The extension's index table (indices 50-53 of NifSurface's bindless attribute). The
// enabled blend type (nif_surface_blend.rs module notes) has the same table, with its
// landscape colour atlas at 51, that atlas's sampler at 52 and its height atlas at 53.
@if(BINDLESS) struct NifSurfaceIndices {
    surface: u32,          // 50
    holes_texture: u32,    // 51
    holes_sampler: u32,    // 52
    facegen_offset: u32,   // 53
}
@if(BINDLESS) @group(constants::MATERIAL_BIND_GROUP) @binding(100) var<storage> nif_indices: array<NifSurfaceIndices>;
@if(BINDLESS) @group(constants::MATERIAL_BIND_GROUP) @binding(101) var<storage> nif_surfaces: array<NifSurface>;
@if(!BINDLESS) @group(constants::MATERIAL_BIND_GROUP) @binding(50) var<uniform> nif_surface_values: NifSurface;
@if(!BINDLESS && !NIF_SURFACE_BLEND) @group(constants::MATERIAL_BIND_GROUP) @binding(51) var holes_texture: texture_2d<f32>;
@if(!BINDLESS && !NIF_SURFACE_BLEND) @group(constants::MATERIAL_BIND_GROUP) @binding(52) var holes_sampler: sampler;
@if(!BINDLESS && !NIF_SURFACE_BLEND) @group(constants::MATERIAL_BIND_GROUP) @binding(53) var facegen_offset: texture_2d<f32>;
@if(!BINDLESS && NIF_SURFACE_BLEND) @group(constants::MATERIAL_BIND_GROUP) @binding(51) var mesh_terrain_colour: texture_2d<f32>;
@if(!BINDLESS && NIF_SURFACE_BLEND) @group(constants::MATERIAL_BIND_GROUP) @binding(52) var mesh_terrain_sampler: sampler;
@if(!BINDLESS && NIF_SURFACE_BLEND) @group(constants::MATERIAL_BIND_GROUP) @binding(53) var mesh_terrain_height: texture_2d<f32>;

// [design] Values come from nif_surface_blend.rs::blend_shader_defs, shared with packing
// and with that file's CPU statements of the weight (`contact_weight`) and normal.
@if(MESH_TERRAIN_BLEND) const GRID: i32 = i32(constants::MESH_TERRAIN_GRID);
@if(MESH_TERRAIN_BLEND) const HEIGHT_TILE: vec2<i32> = vec2(i32(constants::MESH_TERRAIN_HEIGHT_TILE_WIDTH), i32(constants::MESH_TERRAIN_HEIGHT_TILE_HEIGHT));
@if(MESH_TERRAIN_BLEND) const CELL_SIZE: f32 = f32(constants::MESH_TERRAIN_CELL_SIZE);
@if(MESH_TERRAIN_BLEND) const SPACING: f32 = f32(constants::MESH_TERRAIN_SPACING);
@if(MESH_TERRAIN_BLEND) const COS_FULL: f32 = f32(constants::MESH_TERRAIN_COS_FULL_E6) / 1e6;
@if(MESH_TERRAIN_BLEND) const COS_NONE: f32 = f32(constants::MESH_TERRAIN_COS_NONE_E6) / 1e6;
@if(MESH_TERRAIN_BLEND) const PENETRATION: f32 = f32(constants::MESH_TERRAIN_PENETRATION_E6) / 1e6;
@if(MESH_TERRAIN_BLEND) const LAND_ROUGHNESS: f32 = f32(constants::MESH_TERRAIN_ROUGHNESS_E6) / 1e6;
@if(MESH_TERRAIN_BLEND) const LAND_REFLECTANCE: f32 = f32(constants::MESH_TERRAIN_REFLECTANCE_E6) / 1e6;

// [design] Lookup classifications, in diagnostic precedence order.
@if(MESH_TERRAIN_BLEND) const LOOKUP_OUT_OF_RANGE: u32 = 0u;
@if(MESH_TERRAIN_BLEND) const LOOKUP_INVALID_HEIGHT: u32 = 1u;
@if(MESH_TERRAIN_BLEND) const LOOKUP_INVALID_COLOUR: u32 = 2u;
@if(MESH_TERRAIN_BLEND) const LOOKUP_REJECTED_FOOTPRINT: u32 = 3u;
@if(MESH_TERRAIN_BLEND) const LOOKUP_SUCCESS: u32 = 4u;

// [design] One fragment's landscape lookup: its classification, contact weight (zero
// unless it succeeded), the cached colour (sampled only with a positive weight), and the
// ground triangle's geometric normal in game space (x east, y north, z up) and Bevy world
// space, valid from LOOKUP_INVALID_COLOUR on.
@if(MESH_TERRAIN_BLEND) struct MeshTerrainLookup {
    status: u32,
    weight: f32,
    colour: vec3<f32>,
    game_normal: vec3<f32>,
    normal: vec3<f32>,
}

// [design] Encoded coefficients preserve their f32 bits without requiring filterable R32F.
@if(MESH_TERRAIN_BLEND) fn terrain_height_texel(instance_index: u32, p: vec2<i32>) -> f32 {
    @if(BINDLESS) let v = textureLoad(bindless_textures_2d[nif_indices[material_slot(instance_index)].facegen_offset], p, 0);
    @if(!BINDLESS) let v = textureLoad(mesh_terrain_height, p, 0);
    let b = vec4<u32>(round(v * 255.0));
    return bitcast<f32>(b.x | (b.y << 8u) | (b.z << 16u) | (b.w << 24u));
}

@if(MESH_TERRAIN_BLEND) fn terrain_colour_texel(instance_index: u32, uv: vec2<f32>) -> vec3<f32> {
    @if(BINDLESS) let i = nif_indices[material_slot(instance_index)];
    @if(BINDLESS) return textureSampleLevel(bindless_textures_2d[i.holes_texture], bindless_samplers_filtering[i.holes_sampler], uv, 0.0).rgb;
    @if(!BINDLESS) return textureSampleLevel(mesh_terrain_colour, mesh_terrain_sampler, uv, 0.0).rgb;
}

// [design] Colour texels per cell side: the bound atlas is three cells wide
// (terrain_surface.rs, 512, 1024 or 2048 per cell).
@if(MESH_TERRAIN_BLEND) fn terrain_resolution(instance_index: u32) -> i32 {
    @if(BINDLESS) let size = textureDimensions(bindless_textures_2d[nif_indices[material_slot(instance_index)].holes_texture], 0);
    @if(!BINDLESS) let size = textureDimensions(mesh_terrain_colour, 0);
    return i32(size.x) / 3;
}

@if(MESH_TERRAIN_BLEND) fn terrain_cell_resident(instance_index: u32, slot: vec2<i32>) -> bool {
    if any(slot < vec2(0)) || any(slot >= vec2(3)) { return false; }
    return terrain_height_texel(instance_index, slot * HEIGHT_TILE) > -1e30;
}

@if(MESH_TERRAIN_BLEND) fn terrain_colour_valid(instance_index: u32, slot: vec2<i32>, quadrant: u32) -> bool {
    let mask = u32(terrain_height_texel(instance_index, slot * HEIGHT_TILE + vec2(1, GRID)));
    return (mask & (1u << quadrant)) != 0u;
}

// [design] nif_surface_blend.rs::terrain_game_normal and game_to_world_normal.
@if(MESH_TERRAIN_BLEND) fn terrain_game_normal(dx: f32, dy: f32) -> vec3<f32> {
    return normalize(vec3(-dx / SPACING, -dy / SPACING, 1.0));
}
@if(MESH_TERRAIN_BLEND) fn game_to_world_normal(n: vec3<f32>) -> vec3<f32> {
    return vec3(n.x, n.z, -n.y);
}

// [design] nif_surface_blend.rs::contact_weight: shape 0 height band, 1 penetration,
// 2 hybrid (the band times the larger of the orientation and penetration terms).
@if(MESH_TERRAIN_BLEND) fn mesh_terrain_weight(shape: u32, band: f32, d: f32, cos_angle: f32) -> f32 {
    let height = 1.0 - smoothstep(0.0, band, max(d, 0.0));
    let penetration = 1.0 - smoothstep(-PENETRATION, 0.0, d);
    if shape == 0u { return height; }
    if shape == 1u { return penetration; }
    let turned = 1.0 - smoothstep(COS_FULL, COS_NONE, clamp(cos_angle, -1.0, 1.0));
    return height * max(turned, penetration);
}

// [design] nif_surface_blend.rs::blend_normal: exact at w <= 0, the ground normal where
// the mix cancels out.
@if(MESH_TERRAIN_BLEND) fn mesh_terrain_blend_normal(mesh: vec3<f32>, terrain: vec3<f32>, w: f32) -> vec3<f32> {
    if w <= 0.0 { return mesh; }
    let mixed = mix(mesh, terrain, min(w, 1.0));
    if dot(mixed, mixed) < 1e-12 { return terrain; }
    return normalize(mixed);
}

// [design] Coverage, alpha, vertices and depth stay with nif_material.wesl. `mesh_normal`
// is the interpolated vertex normal before normal mapping (pbr_input.world_normal), so
// the weight does not follow the normal map's texture.
@if(MESH_TERRAIN_BLEND) fn mesh_terrain_lookup(instance_index: u32, world: vec3<f32>, mesh_normal: vec3<f32>) -> MeshTerrainLookup {
    let s = nif_surface(instance_index);
    var r = MeshTerrainLookup(LOOKUP_OUT_OF_RANGE, 0.0, vec3(0.0), vec3(0.0, 0.0, 1.0), vec3(0.0, 1.0, 0.0));
    let game = vec3(world.x, -world.z, world.y);
    let cell = vec2<i32>(floor(game.xy / CELL_SIZE));
    let slot = cell - vec2<i32>(s.mesh_terrain_origin.xy);
    if any(slot < vec2(0)) || any(slot >= vec2(3)) { return r; }
    let local = game.xy / SPACING - vec2<f32>(cell) * f32(GRID);
    let square = min(vec2<i32>(floor(local)), vec2(GRID - 1));
    let f = local - vec2<f32>(square);
    let triangle = select(0, 1, f.y > f.x);
    let base = slot * HEIGHT_TILE + vec2(square.x * 6 + triangle * 3, square.y);
    // Coefficients are built by terrain_surface.rs::height_planes from the real mesh diagonal.
    let a = terrain_height_texel(instance_index, base);
    let dx = terrain_height_texel(instance_index, base + vec2(1, 0));
    let dy = terrain_height_texel(instance_index, base + vec2(2, 0));
    if a < -1e30 { r.status = LOOKUP_INVALID_HEIGHT; return r; }
    r.game_normal = terrain_game_normal(dx, dy);
    r.normal = game_to_world_normal(r.game_normal);
    let height = a + f.x * dx + f.y * dy;
    let quadrant = select(0u, 1u, local.x >= f32(GRID) * 0.5) + select(0u, 2u, local.y >= f32(GRID) * 0.5);
    if !terrain_colour_valid(instance_index, slot, quadrant) { r.status = LOOKUP_INVALID_COLOUR; return r; }
    // [design] Fade over one LAND grid interval next to missing slots, before filtering
    // can reach an empty colour texel. Mips/gutters and distance fading remain later work.
    let within = game.xy - vec2<f32>(cell) * CELL_SIZE;
    var border = CELL_SIZE;
    if !terrain_cell_resident(instance_index, slot + vec2(-1, 0)) { border = min(border, within.x); }
    if !terrain_cell_resident(instance_index, slot + vec2(1, 0)) { border = min(border, CELL_SIZE - within.x); }
    if !terrain_cell_resident(instance_index, slot + vec2(0, -1)) { border = min(border, within.y); }
    if !terrain_cell_resident(instance_index, slot + vec2(0, 1)) { border = min(border, CELL_SIZE - within.y); }
    // Validate every cell touched by the exact mip-0 bilinear footprint, including
    // diagonals. The atlas Y axis is reversed by the bake camera.
    let resolution = terrain_resolution(instance_index);
    let atlas_width = 3 * resolution;
    let atlas = (game.xy / CELL_SIZE - s.mesh_terrain_origin.xy) / 3.0;
    let uv = vec2(atlas.x, 1.0 - atlas.y);
    let tap = vec2<i32>(floor(uv * f32(atlas_width) - vec2(0.5)));
    let residency = u32(terrain_height_texel(instance_index, slot * HEIGHT_TILE + vec2(0, GRID)));
    for (var y = 0; y < 2; y++) {
        for (var x = 0; x < 2; x++) {
            let pixel = clamp(tap + vec2(x, y), vec2(0), vec2(atlas_width - 1));
            let tap_slot = vec2(pixel.x / resolution, 2 - pixel.y / resolution);
            let offset = tap_slot - slot + vec2(1);
            let bit = u32(offset.y * 3 + offset.x);
            let tap_quadrant = select(0u, 1u, pixel.x % resolution >= resolution / 2)
                + select(0u, 2u, pixel.y % resolution < resolution / 2);
            if (residency & (1u << bit)) == 0u || !terrain_colour_valid(instance_index, tap_slot, tap_quadrant) {
                r.status = LOOKUP_REJECTED_FOOTPRINT;
                return r;
            }
        }
    }
    let n_length = length(mesh_normal);
    let cos_angle = select(dot(mesh_normal / max(n_length, 1e-6), r.normal), 1.0, n_length < 1e-6);
    r.status = LOOKUP_SUCCESS;
    r.weight = mesh_terrain_weight(u32(s.mesh_terrain.z), s.mesh_terrain.x, game.z - height, cos_angle) * smoothstep(0.0, SPACING, border);
    if r.weight > 0.0 { r.colour = terrain_colour_texel(instance_index, uv); }
    return r;
}

// [design] The aid's view: 0 surface, 1 weight, 2 normal, 3 selection, 4 status.
@if(MESH_TERRAIN_BLEND) fn mesh_terrain_view(instance_index: u32) -> u32 {
    return u32(nif_surface(instance_index).mesh_terrain.y);
}
// [design] 0 albedo, 1 full (colour, normal, roughness, reflectance, metallic, emissive).
@if(MESH_TERRAIN_BLEND) fn mesh_terrain_shading(instance_index: u32) -> u32 {
    return u32(nif_surface(instance_index).mesh_terrain.w);
}
// [design] The weight, normal and status views are drawn unlit, as they are, so their
// colours read without the scene's light (nif_material.wesl).
@if(MESH_TERRAIN_BLEND) fn mesh_terrain_unlit_view(view: u32) -> bool {
    return view == 1u || view == 2u || view == 4u;
}

// [design] The diagnostic colour of a view other than surface; `colour` is the mesh's own
// (unlit) colour. Status precedence: magenta out of range, red invalid height, orange
// invalid colour quadrant, cyan rejected footprint, green success with brightness
// 0.2 + 0.8 weight. Weight: blue (0) to red (1), the mesh's colour where the lookup fails.
// Normal: the ground triangle's game-space normal as 0.5 + 0.5 n (flat ground (0.5,
// 0.5, 1)), black without a valid height. Selection: a green tint (drawn lit).
@if(MESH_TERRAIN_BLEND) fn mesh_terrain_diagnostic(view: u32, t: MeshTerrainLookup, colour: vec3<f32>) -> vec3<f32> {
    if view == 3u { return mix(colour, vec3(0.1, 0.8, 0.25), 0.6); }
    if view == 2u {
        if t.status < LOOKUP_INVALID_COLOUR { return vec3(0.0); }
        return t.game_normal * 0.5 + vec3(0.5);
    }
    if view == 1u {
        if t.status != LOOKUP_SUCCESS { return colour; }
        return vec3(t.weight, 0.0, 1.0 - t.weight);
    }
    if t.status == LOOKUP_OUT_OF_RANGE { return vec3(1.0, 0.0, 1.0); }
    if t.status == LOOKUP_INVALID_HEIGHT { return vec3(1.0, 0.0, 0.0); }
    if t.status == LOOKUP_INVALID_COLOUR { return vec3(1.0, 0.35, 0.0); }
    if t.status == LOOKUP_REJECTED_FOOTPRINT { return vec3(0.0, 1.0, 1.0); }
    return vec3(0.0, 0.2 + 0.8 * t.weight, 0.0);
}

// The material's slot in the bindless tables (unused without bindless).
fn material_slot(instance_index: u32) -> u32 {
    return mesh[instance_index].material_and_lightmap_bind_group_slot & 0xffffu;
}

fn nif_surface(instance_index: u32) -> NifSurface {
    @if(BINDLESS) return nif_surfaces[nif_indices[material_slot(instance_index)].surface];
    @if(!BINDLESS) return nif_surface_values;
}

// The standard part of the material.
fn standard_material(instance_index: u32) -> StandardMaterial {
    @if(BINDLESS) return material_array[material_indices[material_slot(instance_index)].material];
    @if(!BINDLESS) return pbr_bindings::material;
}

// The holes texture's alpha at `uv` (0-1).
fn holes_alpha(instance_index: u32, uv: vec2<f32>) -> f32 {
    @if(BINDLESS) let i = nif_indices[material_slot(instance_index)];
    @if(BINDLESS) return textureSampleLevel(bindless_textures_2d[i.holes_texture], bindless_samplers_filtering[i.holes_sampler], uv, 0.0).a;
    @if(!BINDLESS && !NIF_SURFACE_BLEND) return textureSampleLevel(holes_texture, holes_sampler, uv, 0.0).a;
    // [design] The blend type has no holes (nif_surface_blend.rs module notes): its holes
    // value is always none, so the callers' holes test never reads this.
    @if(!BINDLESS && NIF_SURFACE_BLEND) return 0.0;
}

// The size of the FaceGen offset image.
fn facegen_size(instance_index: u32) -> vec2<u32> {
    @if(BINDLESS) return textureDimensions(bindless_textures_2d[nif_indices[material_slot(instance_index)].facegen_offset]);
    @if(!BINDLESS && !NIF_SURFACE_BLEND) return textureDimensions(facegen_offset);
    // [design] The blend type never has FACEGEN_TINT (its key's facegen is false).
    @if(!BINDLESS && NIF_SURFACE_BLEND) return vec2(1u);
}

// One texel of the FaceGen offset image.
fn facegen_texel(instance_index: u32, p: vec2<i32>) -> vec3<f32> {
    @if(BINDLESS) return textureLoad(bindless_textures_2d[nif_indices[material_slot(instance_index)].facegen_offset], p, 0).rgb;
    @if(!BINDLESS && !NIF_SURFACE_BLEND) return textureLoad(facegen_offset, p, 0).rgb;
    @if(!BINDLESS && NIF_SURFACE_BLEND) return vec3(0.0);
}

// The base colour texture sampled at `uv` with the view's mip bias `bias`.
fn base_color_texel(instance_index: u32, uv: vec2<f32>, bias: f32) -> vec4<f32> {
    @if(BINDLESS) let i = material_indices[material_slot(instance_index)];
    @if(BINDLESS) return textureSampleBias(bindless_textures_2d[i.base_color_texture], bindless_samplers_filtering[i.base_color_sampler], uv, bias);
    @if(!BINDLESS) return textureSampleBias(pbr_bindings::base_color_texture, pbr_bindings::base_color_sampler, uv, bias);
}

// The base colour texture sampled at `uv` at mip level `level`, with no implicit
// derivatives (the Pip-Boy screen's taps, nif_material.wesl `pipboy_picture`).
fn base_color_texel_level(instance_index: u32, uv: vec2<f32>, level: f32) -> vec4<f32> {
    @if(BINDLESS) let i = material_indices[material_slot(instance_index)];
    @if(BINDLESS) return textureSampleLevel(bindless_textures_2d[i.base_color_texture], bindless_samplers_filtering[i.base_color_sampler], uv, level);
    @if(!BINDLESS) return textureSampleLevel(pbr_bindings::base_color_texture, pbr_bindings::base_color_sampler, uv, level);
}

// The size of the base colour texture (the Pip-Boy screen's taps, nif_material.wesl
// `pipboy_picture`).
fn base_color_size(instance_index: u32) -> vec2<u32> {
    @if(BINDLESS) return textureDimensions(bindless_textures_2d[material_indices[material_slot(instance_index)].base_color_texture]);
    @if(!BINDLESS) return textureDimensions(pbr_bindings::base_color_texture);
}

// The normal map sampled at `uv`.
fn normal_map_texel(instance_index: u32, uv: vec2<f32>) -> vec4<f32> {
    @if(BINDLESS) let i = material_indices[material_slot(instance_index)];
    @if(BINDLESS) return textureSample(bindless_textures_2d[i.normal_map_texture], bindless_samplers_filtering[i.normal_map_sampler], uv);
    @if(!BINDLESS) return textureSample(pbr_bindings::normal_map_texture, pbr_bindings::normal_map_sampler, uv);
}
