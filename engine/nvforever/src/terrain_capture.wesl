// [design] Mesh terrain blend's single-texture capture (terrain_surface.rs
// `SurfaceCaptureMaterial`): the unlit colour of a landscape draw that is not a folded
// base (a fallback or non-bindless base, or a blended layer drawn over a base), with one
// diffuse texture, one sampler and one small uniform, so its material group stays small
// on a GPU without bindless textures. Colour: material x vertex x diffuse (a missing
// texture counts as white). A base copy writes alpha 1; a layer copy writes material x
// vertex x diffuse alpha, which its pipeline's alpha blend composites over the copies
// before it, in the near layers' order. No lighting, scorch, fog or exposure.
import bevy_pbr::render::forward_io::{VertexOutput, FragmentOutput};

struct SurfaceCapture {
    // Linear material colour, alpha the material's opacity.
    colour: vec4<f32>,
    // The texture coordinate transform's x axis (xy) and y axis (zw).
    uv_axes: vec4<f32>,
    // xy the transform's translation; z 1 with a diffuse texture; w 1 for a layer copy.
    uv_offset_flags: vec4<f32>,
}

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> surface_capture: SurfaceCapture;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var diffuse_texture: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var diffuse_sampler: sampler;

@fragment
fn fragment(in: VertexOutput) -> FragmentOutput {
    var colour = surface_capture.colour;
    @if(VERTEX_COLORS) colour *= in.color;
    @if(VERTEX_UVS_A) {
        let uv = mat2x2<f32>(surface_capture.uv_axes.xy, surface_capture.uv_axes.zw) * in.uv + surface_capture.uv_offset_flags.xy;
        // Sampled unconditionally (uniform control flow), then ignored without a texture.
        let texel = textureSample(diffuse_texture, diffuse_sampler, uv);
        colour *= select(vec4(1.0), texel, surface_capture.uv_offset_flags.z > 0.5);
    }
    var out: FragmentOutput;
    out.color = vec4(colour.rgb, select(1.0, colour.a, surface_capture.uv_offset_flags.w > 0.5));
    return out;
}
