//! The reflection gate in bevy_pbr's `pbr_functions` (#2981 reflections, slice 1;
//! docs/spec-notes/pbr-materials.md "Reflections on authored materials, slice 1") [design].
//! Plain text and the standard library only, so the shader compile checks
//! (engine/nvf-shader-tests) apply the very same edits before composing.
//!
//! Bevy's `apply_pbr_lighting` adds a view's environment map light (the cube of
//! `EnvironmentMapLight`, shader def `ENVIRONMENT_MAP`, which bevy_pbr sets from the view's
//! key) for every lit shader of that view: terrain, trees and their bark (Bevy's own
//! fragment), grass, water, impact decals and every NIF surface alike. NVForever wants it on
//! authored materials only, so both environment blocks of the function are compiled only
//! where the fragment opts in with the def `NVF_PBR_REFLECT` as well, which only the NIF
//! surface's main pass sets (nvf_assets nif_material.rs, while the run's switch is on). The
//! first block, the ordinary reflection, also skips its cube lookup where its weight
//! (`specular_occlusion`, which the NIF fragment sets to the authored mark times its other
//! factors) is 0, so a legacy NIF shape of a view with a cube reads nothing. The second, the
//! transmitted environment light, needs a transmission def no NIF pipeline has.
//!
//! Without `NVF_PBR_REFLECT` the edited text compiles to what it compiled to without
//! `ENVIRONMENT_MAP`: nothing of the environment map is read, whatever the view has.

/// The edits [documented: engine/vendor/bevy_pbr/src/render/pbr_functions.wesl, bevy_pbr
/// 0.20.1, after lights/display_shader.rs `DISPLAY_EDITS`, the sun visibility's and the
/// ambient occlusion's hooks, none of which touches these lines], in order, each at the
/// first occurrence of its anchor after the last edit: (anchor, replacement).
pub const GATE_EDITS: [(&str, &str); 3] = [
    (
        "    // Environment map light (indirect)\n@if(ENVIRONMENT_MAP) {\n",
        "    // Environment map light (indirect)
    // NVForever reflection gate (nvforever lights/reflection_gate.rs): an opted-in fragment
    // only (NVF_PBR_REFLECT), and no cube lookup where its weight is 0.
@if(ENVIRONMENT_MAP && NVF_PBR_REFLECT) {
    if specular_occlusion > 0.0 {
",
    ),
    (
        "        indirect_light += environment_light.specular * specular_occlusion;\n    }\n}\n",
        "        indirect_light += environment_light.specular * specular_occlusion;\n    }\n    }\n}\n",
    ),
    (
        "@if(ENVIRONMENT_MAP) {\n\n@if(STANDARD_MATERIAL_DIFFUSE_OR_SPECULAR_TRANSMISSION) {\n",
        "// NVForever reflection gate (nvforever lights/reflection_gate.rs): opted-in fragments only.
@if(ENVIRONMENT_MAP && NVF_PBR_REFLECT) {

@if(STANDARD_MATERIAL_DIFFUSE_OR_SPECULAR_TRANSMISSION) {
",
    ),
];

/// `source` (lights.rs's patched `pbr_functions`) with [`GATE_EDITS`]; `None` when an anchor
/// is missing (Bevy or an earlier patch changed), and the caller keeps `source` and leaves
/// reflections off. Also `None` if anything but the module's import is left under a plain
/// `@if(ENVIRONMENT_MAP)` afterwards (a new block in a later Bevy would leak the cube to
/// every lit shader).
pub fn gate_patched(source: &str) -> Option<String> {
    let mut out = String::with_capacity(source.len() + 512);
    let mut rest = source;
    for (anchor, text) in GATE_EDITS {
        let at = rest.find(anchor)?;
        out.push_str(&rest[..at]);
        out.push_str(text);
        rest = &rest[at + anchor.len()..];
    }
    out.push_str(rest);
    ungated_blocks(&out).is_empty().then_some(out)
}

/// The byte offsets of every `@if(ENVIRONMENT_MAP)` in `source` that does not guard the
/// environment map module's import.
pub fn ungated_blocks(source: &str) -> Vec<usize> {
    const PLAIN: &str = "@if(ENVIRONMENT_MAP)";
    source.match_indices(PLAIN).map(|(at, _)| at).filter(|&at| !source[at + PLAIN.len()..].starts_with(" import ")).collect()
}
