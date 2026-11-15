// [design] terrain.wesl evaluates the same stitched surfaces for both draws, then this
// endpoint stores linear unlit colour without scorch, lighting, fog or exposure. `alpha` is
// 1 for a base copy and the layer's own opacity for a blended layer copy, which the
// pipeline's alpha blend composites over the base as the near terrain's layer draws do.
import bevy_pbr::render::forward_io::FragmentOutput;

fn surface_bake(colour: vec3<f32>, alpha: f32) -> FragmentOutput {
    var out: FragmentOutput;
    out.color = vec4(colour, alpha);
    return out;
}
