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
    // [design] Band, variant, unused; cache southwest cell in origin.xy.
    mesh_terrain: vec4<f32>,
    mesh_terrain_origin: vec4<f32>,
}

// The extension's index table (indices 50-56 of nif_material.rs NifSurface's bindless attribute).
@if(BINDLESS) struct NifSurfaceIndices {
    surface: u32,          // 50
    holes_texture: u32,    // 51
    holes_sampler: u32,    // 52
    facegen_offset: u32,   // 53
    mesh_terrain_colour: u32, // 54
    mesh_terrain_sampler: u32, // 55
    mesh_terrain_height: u32, // 56
}
@if(BINDLESS) @group(constants::MATERIAL_BIND_GROUP) @binding(100) var<storage> nif_indices: array<NifSurfaceIndices>;
@if(BINDLESS) @group(constants::MATERIAL_BIND_GROUP) @binding(101) var<storage> nif_surfaces: array<NifSurface>;
@if(!BINDLESS) @group(constants::MATERIAL_BIND_GROUP) @binding(50) var<uniform> nif_surface_values: NifSurface;
@if(!BINDLESS) @group(constants::MATERIAL_BIND_GROUP) @binding(51) var holes_texture: texture_2d<f32>;
@if(!BINDLESS) @group(constants::MATERIAL_BIND_GROUP) @binding(52) var holes_sampler: sampler;
@if(!BINDLESS) @group(constants::MATERIAL_BIND_GROUP) @binding(53) var facegen_offset: texture_2d<f32>;
@if(!BINDLESS) @group(constants::MATERIAL_BIND_GROUP) @binding(54) var mesh_terrain_colour: texture_2d<f32>;
@if(!BINDLESS) @group(constants::MATERIAL_BIND_GROUP) @binding(55) var mesh_terrain_sampler: sampler;
@if(!BINDLESS) @group(constants::MATERIAL_BIND_GROUP) @binding(56) var mesh_terrain_height: texture_2d<f32>;

// [design] Encoded heights preserve all original f32 bits without requiring filterable R32F.
@if(MESH_TERRAIN_BLEND) fn terrain_height_texel(instance_index: u32, p: vec2<i32>) -> f32 {
    @if(BINDLESS) let v = textureLoad(bindless_textures_2d[nif_indices[material_slot(instance_index)].mesh_terrain_height], p, 0);
    @if(!BINDLESS) let v = textureLoad(mesh_terrain_height, p, 0);
    let b = vec4<u32>(round(v * 255.0));
    return bitcast<f32>(b.x | (b.y << 8u) | (b.z << 16u) | (b.w << 24u));
}

@if(MESH_TERRAIN_BLEND) fn terrain_colour_texel(instance_index: u32, uv: vec2<f32>) -> vec3<f32> {
    @if(BINDLESS) let i = nif_indices[material_slot(instance_index)];
    @if(BINDLESS) return textureSampleLevel(bindless_textures_2d[i.mesh_terrain_colour], bindless_samplers_filtering[i.mesh_terrain_sampler], uv, 0.0).rgb;
    @if(!BINDLESS) return textureSampleLevel(mesh_terrain_colour, mesh_terrain_sampler, uv, 0.0).rgb;
}

@if(MESH_TERRAIN_BLEND) fn terrain_cell_resident(instance_index: u32, slot: vec2<i32>) -> bool {
    if any(slot < vec2(0)) || any(slot >= vec2(3)) { return false; }
    return terrain_height_texel(instance_index, slot * 33) > -1e30;
}

// [design] Colour only; coverage, alpha, vertices and depth stay with nif_material.wesl.
@if(MESH_TERRAIN_BLEND) fn mesh_terrain_blend(instance_index: u32, world: vec3<f32>, colour: vec3<f32>) -> vec3<f32> {
    let s = nif_surface(instance_index);
    let variant = u32(s.mesh_terrain.y);
    if variant == 3u { return mix(colour, vec3(0.1, 0.8, 0.25), 0.6); }
    let game = vec3(world.x, -world.z, world.y);
    let cell = vec2<i32>(floor(game.xy / 4096.0));
    let slot = cell - vec2<i32>(s.mesh_terrain_origin.xy);
    if any(slot < vec2(0)) || any(slot >= vec2(3)) { return colour; }
    let local = game.xy / 128.0 - vec2<f32>(cell) * 32.0;
    let square = min(vec2<i32>(floor(local)), vec2(31));
    let f = local - vec2<f32>(square);
    let base = slot * 33 + square;
    let a = terrain_height_texel(instance_index, base);
    let b = terrain_height_texel(instance_index, base + vec2(1, 0));
    let c = terrain_height_texel(instance_index, base + vec2(0, 1));
    let d = terrain_height_texel(instance_index, base + vec2(1, 1));
    // [design] Missing slots contain a negative sentinel outside the supported height range.
    if a < -1e30 || b < -1e30 || c < -1e30 || d < -1e30 { return colour; }
    // terrain.rs::quadrant_mesh triangles: SW/E/NE and SW/NE/N.
    var height = a + f.x * (b - a) + f.y * (d - b);
    if f.y > f.x { height = a + f.x * (d - c) + f.y * (c - a); }
    // [design] Fade over one LAND grid interval next to missing slots, before filtering
    // can reach an empty colour texel. Mips/gutters and distance fading remain later work.
    let within = game.xy - vec2<f32>(cell) * 4096.0;
    var border = 4096.0;
    if !terrain_cell_resident(instance_index, slot + vec2(-1, 0)) { border = min(border, within.x); }
    if !terrain_cell_resident(instance_index, slot + vec2(1, 0)) { border = min(border, 4096.0 - within.x); }
    if !terrain_cell_resident(instance_index, slot + vec2(0, -1)) { border = min(border, within.y); }
    if !terrain_cell_resident(instance_index, slot + vec2(0, 1)) { border = min(border, 4096.0 - within.y); }
    let w = (1.0 - smoothstep(0.0, s.mesh_terrain.x, max(game.z - height, 0.0))) * smoothstep(0.0, 128.0, border);
    if variant == 1u { return vec3(w, 0.0, 1.0 - w); }
    if w <= 0.0 { return colour; }
    let atlas = (game.xy / 4096.0 - s.mesh_terrain_origin.xy) / 3.0;
    let uv = vec2(atlas.x, 1.0 - atlas.y);
    return mix(colour, terrain_colour_texel(instance_index, uv), w);
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
    @if(!BINDLESS) return textureSampleLevel(holes_texture, holes_sampler, uv, 0.0).a;
}

// The size of the FaceGen offset image.
fn facegen_size(instance_index: u32) -> vec2<u32> {
    @if(BINDLESS) return textureDimensions(bindless_textures_2d[nif_indices[material_slot(instance_index)].facegen_offset]);
    @if(!BINDLESS) return textureDimensions(facegen_offset);
}

// One texel of the FaceGen offset image.
fn facegen_texel(instance_index: u32, p: vec2<i32>) -> vec3<f32> {
    @if(BINDLESS) return textureLoad(bindless_textures_2d[nif_indices[material_slot(instance_index)].facegen_offset], p, 0).rgb;
    @if(!BINDLESS) return textureLoad(facegen_offset, p, 0).rgb;
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
