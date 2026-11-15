// [design] terrain.wesl evaluates the same stitched surfaces for both draws, then this
// endpoint stores linear unlit colour without scorch, lighting, fog or exposure.
import bevy_pbr::render::forward_io::FragmentOutput;

fn surface_bake(colour: vec3<f32>) -> FragmentOutput {
    var out: FragmentOutput;
    out.color = vec4(colour, 1.0);
    return out;
}
