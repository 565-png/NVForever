// Distant lights at night (distant_lights.rs, docs/design/graphics-enhanced.md §3.17): an
// enhanced-look switch of NVForever's own, not a fact about the original game.
//
// One batched mesh of glows; each glow is four vertices at its centre (game space under
// the world root) with UV_0 the corner (-1..1), UV_1.x the glow's own half size (world
// units) and UV_1.y its motion code (0 steady, 1 flicker, 2 pulse) plus a phase in its
// fraction, and COLOR its colour as display values times its strength (merged lights,
// the record's fade, the hand-over with its cell's fade).
//
// Vertex: the card is spread in the plane of the view (always facing the camera), at least
// `glow.shape.w` output pixels and `glow.extra.z` render pixels in half size (the two differ
// under an upscaler) so a far light stays a few pixels wide; a card widened by that floor
// gives less light per pixel (its own area over the area drawn, held to `glow.extra.x`).
// Faded out over the reach's last cells (`glow.shape.yz`), by the night (`glow.shape.x`,
// with the peak), and a slow shimmer for flickering and pulsing lights.
//
// Fragment: a soft round spot, unlit, as the NIF materials' additive shapes are drawn
// (nvf_assets nif_material.wesl, nif.md §6 "Additive blend", "Fog on added light"): the
// colour, a display value, is made linear (raised to 2.2, `display_emissive`), taken through
// the view's fog and, in a view without HDR (Light glow off), its tonemapping, less black
// through the same (the fog's own colour is not added: the frame behind holds it), and
// added to the opaque frame behind it in display values (`add_display`, the view
// transmission texture the anchor of nif_material.rs keeps in every view). Returned as the
// linear difference with alpha 0 (Bevy's premultiplied blend for AlphaMode::Add adds it).

import bevy_pbr::{
    render::mesh_functions,
    render::mesh_view_bindings::{view, globals},
    render::view_transformations::position_world_to_clip,
};
import bevy_pbr::render::mesh_view_bindings::{view_transmission_texture, view_transmission_sampler};
@if(DISTANCE_FOG) import bevy_pbr::render::mesh_view_bindings::fog;
@if(DISTANCE_FOG) import bevy_pbr::render::pbr_functions::apply_fog;
@if(TONEMAP_IN_SHADER) import bevy_core_pipeline::tonemapping::tone_mapping;

struct GlowParams {
    // x: night share times the peak; y, z: distance where the reach fade starts and ends;
    // w: the smallest half size in pixels.
    shape: vec4<f32>,
    // x: the least light per pixel a widened card keeps; y: shimmer depth; z: the smallest
    // half size in render pixels; w unused.
    extra: vec4<f32>,
}

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> glow: GlowParams;

struct GlowVertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(2) corner: vec2<f32>,
    @location(3) size: vec2<f32>,
    @location(5) color: vec4<f32>,
}

struct GlowOut {
    @builtin(position) position: vec4<f32>,
    @location(0) world_position: vec3<f32>,
    @location(1) corner: vec2<f32>,
    @location(2) color: vec4<f32>,
}

@vertex
fn vertex(vertex: GlowVertex) -> GlowOut {
    var out: GlowOut;
    let world_from_local = mesh_functions::get_world_from_local(vertex.instance_index);
    let centre = mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(vertex.position, 1.0)).xyz;
    let d = distance(centre, view.world_position.xyz);
    // One output pixel's and one render pixel's size at this distance (the main pass's own
    // height under an upscaler; both the same without one). CPU copy: `drawn_size`.
    let f = max(view.clip_from_view[1][1], 1e-4);
    let pixel = 2.0 * d / (f * max(view.viewport.w, 1.0));
    let render_pixel = 2.0 * d / (f * max(view.main_pass_viewport.w, 1.0));
    let own = max(vertex.size.x, 1e-3);
    let half = max(own, max(glow.shape.w * pixel, glow.extra.z * render_pixel));
    let share = clamp((own / half) * (own / half), glow.extra.x, 1.0);
    let reach = 1.0 - smoothstep(glow.shape.y, glow.shape.z, d);
    let motion = floor(vertex.size.y);
    let phase = fract(vertex.size.y) * 6.2831853;
    var shimmer: f32 = 1.0;
    if motion > 0.5 {
        // Flicker a little quicker than pulse; both slow (seen from afar).
        let speed = select(0.9, 2.3, motion < 1.5);
        shimmer = 1.0 - glow.extra.y * (0.5 + 0.5 * sin(globals.time * speed + phase));
    }
    let right = view.world_from_view[0].xyz;
    let up = view.world_from_view[1].xyz;
    let world = centre + (right * vertex.corner.x + up * vertex.corner.y) * half;
    out.position = position_world_to_clip(world);
    out.world_position = world;
    out.corner = vertex.corner;
    out.color = vec4<f32>(vertex.color.rgb * (glow.shape.x * share * reach * shimmer), 1.0);
    return out;
}

// What ordinary materials' post-lighting step does to a colour (bevy_pbr
// `main_pass_post_lighting_processing`): the view's fog, then, without HDR, the tonemapping.
fn post(c: vec3<f32>, world_position: vec3<f32>, frag: vec2<f32>) -> vec3<f32> {
    var o = vec4<f32>(c, 1.0);
    @if(DISTANCE_FOG) {
        o = apply_fog(fog, o, world_position, view.world_position.xyz, frag);
    }
    @if(TONEMAP_IN_SHADER) {
        o = tone_mapping(o, view.color_grading);
    }
    return o.rgb;
}

// The frame behind a fragment: the view transmission texture at its pixel (the main pass's
// own region under an upscaler; as nif_material.wesl `transmission_uv`).
fn behind(frag: vec2<f32>) -> vec3<f32> {
    let lo = view.main_pass_viewport.xy + 0.5;
    let hi = view.main_pass_viewport.xy + view.main_pass_viewport.zw - 0.5;
    let uv = clamp(frag, lo, hi) / vec2<f32>(textureDimensions(view_transmission_texture));
    return textureSampleLevel(view_transmission_texture, view_transmission_sampler, uv, 0.0).rgb;
}

// The linear light added over the frame `under` (linear) for the linear colour `colour`:
// the sum formed in display values (gamma 2.2), held to 2.0 or the frame's own value if
// brighter (nif_material.wesl `add_display` with alpha 1; CPU mirror nvf_assets
// `nif_material::add_display`).
fn add_display(colour: vec3<f32>, under: vec3<f32>) -> vec3<f32> {
    let b = max(under, vec3<f32>(0.0));
    let shown = pow(b, vec3<f32>(1.0 / 2.2));
    let added = pow(max(colour, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.2));
    let sum = min(shown + added, max(shown, vec3<f32>(2.0)));
    return max(pow(sum, vec3<f32>(2.2)) - b, vec3<f32>(0.0));
}

@fragment
fn fragment(in: GlowOut) -> @location(0) vec4<f32> {
    let r2 = dot(in.corner, in.corner);
    if r2 >= 1.0 {
        discard;
    }
    // A bright core easing to nothing at the card's edge, in display values.
    let display = in.color.rgb * (exp(-4.0 * r2) * (1.0 - r2));
    let linear = pow(max(display, vec3<f32>(0.0)), vec3<f32>(2.2));
    let added = max(post(linear, in.world_position, in.position.xy) - post(vec3<f32>(0.0), in.world_position, in.position.xy), vec3<f32>(0.0));
    return vec4<f32>(add_display(added, behind(in.position.xy)), 0.0);
}
