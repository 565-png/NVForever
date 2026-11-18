//! The ambient occlusion's text hook in bevy_pbr's `pbr_functions` (ssao_lighting.rs module
//! notes). Plain text and the standard library only, so the shader compile checks
//! (engine/nvf-shader-tests) apply the very same edits before composing the shaders that
//! read the occlusion.

/// Where an edit's text goes against its anchor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    Before,
    After,
    Replace,
}

/// The display-value ambient line of lights/display_shader.rs `DISPLAY_EDITS`, as it
/// stands in the patched `apply_pbr_lighting`.
pub const AMBIENT_LINE: &str = "        let nvf_ambient = view_bindings::lights.ambient_color.rgb * view_bindings::view.exposure * diffuse_occlusion;\n";

/// The same line kept without the def, and its twin with the occlusion under it. `-view_z`
/// is the fragment's view distance from its own world position (so the first-person arms,
/// drawn at a squeezed depth, still compare their real distance with the texels', which the
/// occlusion pass unsqueezed).
pub const AMBIENT_LINE_HOOKED: &str = "        // NVForever ambient occlusion (nvforever ssao_lighting.rs): on the display-value
        // ambient alone, never on the direct lights nor on Bevy's ambient highlight above.
        @if(!NVF_AMBIENT_OCCLUSION)
        let nvf_ambient = view_bindings::lights.ambient_color.rgb * view_bindings::view.exposure * diffuse_occlusion;
        @if(NVF_AMBIENT_OCCLUSION)
        let nvf_ambient = view_bindings::lights.ambient_color.rgb * view_bindings::view.exposure * diffuse_occlusion * nvf_ambient_visibility(in.frag_coord.xy, -view_z);
";

/// The end of `main_pass_post_lighting_processing`: after the fog, the tonemapping in the
/// shader and the dither, before the alpha is premultiplied.
pub const DEBUG_ANCHOR: &str = "    @if(PREMULTIPLY_ALPHA)\n    output_color = premultiply_alpha(pbr_input.material.flags, output_color);\n";

/// The function [`DEBUG_ANCHOR`] must lie in.
pub const DEBUG_FUNCTION: &str = "\nfn main_pass_post_lighting_processing(";

/// The debug view (`NVF_PLAY_DEBUG_AO=view-lighting`): the factor this fragment's ambient
/// took, drawn in place of the lit colour at the very end of the material's colour (after
/// whatever the material did to the lit colour, the fog and the dither), so what is seen is
/// exactly what `nvf_ambient_visibility` gave the fragment: the same position, the same
/// distance (`view_z` as `apply_pbr_lighting` forms it).
pub const DEBUG_TEXT: &str = "    // NVForever ambient occlusion (nvforever ssao_lighting.rs): the debug view draws the
    // factor this fragment's ambient light took, after everything else.
    @if(NVF_AMBIENT_OCCLUSION)
    if nvf_ao_debug() {
        let nvf_view_z = dot(vec4<f32>(
            view_bindings::view.view_from_world[0].z,
            view_bindings::view.view_from_world[1].z,
            view_bindings::view.view_from_world[2].z,
            view_bindings::view.view_from_world[3].z
        ), pbr_input.world_position);
        output_color = vec4<f32>(vec3<f32>(nvf_ambient_visibility(pbr_input.frag_coord.xy, -nvf_view_z)), output_color.a);
    }
";

/// The hook in bevy_pbr's `pbr_functions` [documented:
/// engine/vendor/bevy_pbr/src/render/pbr_functions.wesl (bevy_pbr 0.20.1) with lights.rs
/// `DISPLAY_EDITS` (and, in the game, sun_visibility.rs `HOOK_EDITS`) made; the tests apply
/// them to that file], in order, each at the first occurrence of its anchor after the last
/// edit: (anchor, text, where the text goes). Deliberately left alone: Bevy's
/// `ambient::ambient_light` call and `let diffuse_occlusion = in.diffuse_occlusion;`, since
/// the NIF and terrain shaders subtract exactly that ambient highlight using the
/// un-occluded `diffuse_occlusion` (nvf_assets nif_material.wesl, terrain.wesl), and an
/// occluded highlight would be over-subtracted there.
pub const HOOK_EDITS: [(&str, &str, Place); 3] = [
    (
        "import bevy_render::maths::{E, powsafe};\n",
        "// NVForever ambient occlusion (nvforever ssao_lighting.rs).
@if(NVF_AMBIENT_OCCLUSION) import nvforever::ssao_lighting::{nvf_ambient_visibility, nvf_ao_debug};
",
        Place::After,
    ),
    (AMBIENT_LINE, AMBIENT_LINE_HOOKED, Place::Replace),
    (DEBUG_ANCHOR, DEBUG_TEXT, Place::Before),
];

/// The index in [`HOOK_EDITS`] of the debug view's edit.
pub const DEBUG_EDIT: usize = 2;

/// `source` (lights.rs's display-value patch, and in the game the sun visibility's hook,
/// made) with [`HOOK_EDITS`]; `None` when an anchor is missing or the debug view's anchor
/// is not in `main_pass_post_lighting_processing` (Bevy or the display patch changed), and
/// the caller keeps `source`.
pub fn hook_patched(source: &str) -> Option<String> {
    let mut out = String::with_capacity(source.len() + 2048);
    let mut rest = source;
    for (i, (anchor, text, place)) in HOOK_EDITS.into_iter().enumerate() {
        let at = rest.find(anchor)?;
        if i == DEBUG_EDIT {
            let head = &rest[..at];
            if !head.rfind("\nfn ").is_some_and(|f| head[f..].starts_with(DEBUG_FUNCTION)) {
                return None;
            }
        }
        out.push_str(&rest[..at]);
        match place {
            Place::Before => {
                out.push_str(text);
                out.push_str(anchor);
            }
            Place::After => {
                out.push_str(anchor);
                out.push_str(text);
            }
            Place::Replace => out.push_str(text),
        }
        rest = &rest[at + anchor.len()..];
    }
    out.push_str(rest);
    Some(out)
}
