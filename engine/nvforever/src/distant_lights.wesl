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
// `glow.shape.w` pixels in half size so a far light stays a pixel or two; a card widened by
// that floor gives less light per pixel (its own area over the area drawn, held to
// `glow.extra.x`). Faded out over the reach's last cells (`glow.shape.yz`), by the night
// (`glow.shape.x`, with the peak), and a slow shimmer for flickering and pulsing lights.
//
// Fragment: a soft round spot, unlit, additive (alpha 0 under the premultiplied blend Bevy
// uses for AlphaMode::Add), seen through the view's fog: the share of light that passes is
// what the fog function gives white minus what it gives black, so the weather's fog curve
// and the desert haze (fog.rs, haze.rs) swallow the glows as they swallow the scenery.

import bevy_pbr::{
    render::mesh_functions,
    render::mesh_view_bindings::{view, globals},
    render::view_transformations::position_world_to_clip,
};
@if(DISTANCE_FOG) import bevy_pbr::render::mesh_view_bindings::fog;
@if(DISTANCE_FOG) import bevy_pbr::render::pbr_functions::apply_fog;

struct GlowParams {
    // x: night share times the peak; y, z: distance where the reach fade starts and ends;
    // w: the smallest half size in pixels.
    shape: vec4<f32>,
    // x: the least light per pixel a widened card keeps; y: shimmer depth; zw unused.
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
    // One pixel's size at this distance (the main pass's own height under upscaling).
    let pixel = 2.0 * d / (max(view.clip_from_view[1][1], 1e-4) * max(view.main_pass_viewport.w, 1.0));
    let own = max(vertex.size.x, 1e-3);
    let half = max(own, glow.shape.w * pixel);
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

@fragment
fn fragment(in: GlowOut) -> @location(0) vec4<f32> {
    let r2 = dot(in.corner, in.corner);
    if r2 >= 1.0 {
        discard;
    }
    // A bright core easing to nothing at the card's edge.
    var colour = in.color.rgb * (exp(-4.0 * r2) * (1.0 - r2));
    @if(DISTANCE_FOG) {
        let lit = apply_fog(fog, vec4<f32>(1.0, 1.0, 1.0, 1.0), in.world_position, view.world_position.xyz, in.position.xy);
        let dark = apply_fog(fog, vec4<f32>(0.0, 0.0, 0.0, 1.0), in.world_position, view.world_position.xyz, in.position.xy);
        colour *= clamp(lit.rgb - dark.rgb, vec3<f32>(0.0), vec3<f32>(1.0));
    }
    return vec4<f32>(colour, 0.0);
}
