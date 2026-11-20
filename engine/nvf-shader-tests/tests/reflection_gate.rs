//! The reflection gate (nvforever lights/reflection_gate.rs, #2981 reflections slice 1):
//! `pbr_functions` with the display-value patch, the ambient occlusion's hook and the gate,
//! composed through Bevy's shader cache and validated by Naga. No render device is created.
//!
//! In a view with an environment map (`ENVIRONMENT_MAP`) the specular cube (group 1,
//! binding 1, the single-map layout) is read only by the NIF fragment that opts in with
//! `NVF_PBR_REFLECT`; every other lit shader the view draws (Bevy's own fragment: tree bark,
//! impact decals; terrain, grass, tree leaves; the NIF fragment without the opt-in) compiles
//! without reading it, as it did without `ENVIRONMENT_MAP`.

mod support;
use support::{Compiled, Compiler};

/// Whether the module's fragment entry point reads the global at `group`, `binding`.
fn fragment_reads(compiled: &Compiled, group: u32, binding: u32) -> bool {
    let Some((handle, _)) = compiled
        .module
        .global_variables
        .iter()
        .find(|(_, g)| g.binding.as_ref().is_some_and(|b| b.group == group && b.binding == binding))
    else {
        return false;
    };
    compiled
        .module
        .entry_points
        .iter()
        .enumerate()
        .filter(|(_, e)| e.stage == naga::ShaderStage::Fragment)
        .any(|(i, _)| !compiled.info.get_entry_point(i)[handle].is_empty())
}

const SPECULAR_CUBE: (u32, u32) = (1, 1);

#[test]
fn only_the_opted_in_nif_fragment_reads_the_view_cube() {
    let mut compiler = Compiler::with_reflection_gate();
    for (shader, extra, reads) in [
        // The NIF surface's main pass with the opt-in: both lighting calls, the plain one
        // and the first-person shadow correction's.
        ("nvf_assets/nif_material.wesl", &["ENVIRONMENT_MAP", "NVF_PBR_REFLECT"][..], true),
        ("nvf_assets/nif_material.wesl", &["ENVIRONMENT_MAP", "NVF_PBR_REFLECT", "MAY_DISCARD"][..], true),
        ("nvf_assets/nif_material.wesl", &["ENVIRONMENT_MAP", "NVF_PBR_REFLECT", "FIRST_PERSON_DEPTH", "SHADE_LIGHT"][..], true),
        // Without the opt-in (the switch off) the view's cube is not read.
        ("nvf_assets/nif_material.wesl", &["ENVIRONMENT_MAP"][..], false),
        // The reflection channel view (NVF_PBR_VIEW=reflection) reads it in the opted-in
        // fragment only; with the reflections off it draws the authored view's colours.
        ("nvf_assets/nif_material.wesl", &["ENVIRONMENT_MAP", "NVF_PBR_REFLECT", "NVF_PBR_VIEW", "NVF_PBR_VIEW_REFLECTION"][..], true),
        ("nvf_assets/nif_material.wesl", &["ENVIRONMENT_MAP", "NVF_PBR_VIEW", "NVF_PBR_VIEW_REFLECTION"][..], false),
        ("nvf_assets/nif_material.wesl", &["NVF_PBR_REFLECT", "NVF_PBR_VIEW", "NVF_PBR_VIEW_REFLECTION"][..], false),
        // Every other lit shader of a view with a cube.
        ("bevy_pbr/render/pbr.wesl", &["ENVIRONMENT_MAP"][..], false),
        ("nvforever/terrain.wesl", &["ENVIRONMENT_MAP"][..], false),
        ("nvforever/grass.wesl", &["ENVIRONMENT_MAP"][..], false),
        ("nvforever/tree.wesl", &["ENVIRONMENT_MAP", "TREE_LEAVES"][..], false),
    ] {
        let compiled = compiler.compile(shader, extra);
        assert_eq!(fragment_reads(&compiled, SPECULAR_CUBE.0, SPECULAR_CUBE.1), reads, "{shader} {extra:?}: the specular cube");
    }
}

#[test]
fn the_opted_in_nif_fragment_compiles_in_a_view_without_a_cube() {
    // The switch on, a camera without a cube (the water mirror, the scope, the field's
    // second camera): the def alone, metallic held at 0 in the shader.
    let mut compiler = Compiler::with_reflection_gate();
    for extra in [&["NVF_PBR_REFLECT"][..], &["NVF_PBR_REFLECT", "FIRST_PERSON_DEPTH", "SHADE_LIGHT"][..]] {
        let compiled = compiler.compile("nvf_assets/nif_material.wesl", extra);
        assert!(!fragment_reads(&compiled, SPECULAR_CUBE.0, SPECULAR_CUBE.1), "{extra:?}");
        assert!(compiled.module.entry_points.iter().any(|e| e.stage == naga::ShaderStage::Fragment), "{extra:?}: a fragment entry point");
    }
}

#[test]
fn the_metal_lift_experiment_composes_in_the_opted_in_fragment() {
    // NVF_PBR_METAL_LIFT (nvf-assets authored.rs `metal_lift`): the switch def and the gain in
    // thousandths as a valued def, read as constants::NVF_PBR_METAL_LIFT_E3; with a cube, the
    // plain and the first-person shade-light variants; without a cube the lift is not compiled.
    let mut compiler = Compiler::with_reflection_gate();
    let gain = [bevy_shader::ShaderDefVal::UInt("NVF_PBR_METAL_LIFT_E3".into(), 3000)];
    for extra in [
        &["ENVIRONMENT_MAP", "NVF_PBR_REFLECT", "NVF_PBR_METAL_LIFT"][..],
        &["ENVIRONMENT_MAP", "NVF_PBR_REFLECT", "NVF_PBR_METAL_LIFT", "FIRST_PERSON_DEPTH", "SHADE_LIGHT"][..],
        &["NVF_PBR_REFLECT", "NVF_PBR_METAL_LIFT"][..],
    ] {
        let compiled = compiler.compile_with_values("nvf_assets/nif_material.wesl", extra, &gain);
        assert!(compiled.module.entry_points.iter().any(|e| e.stage == naga::ShaderStage::Fragment), "{extra:?}: a fragment entry point");
        assert_eq!(fragment_reads(&compiled, SPECULAR_CUBE.0, SPECULAR_CUBE.1), extra.contains(&"ENVIRONMENT_MAP"), "{extra:?}: the cube read as without the lift");
    }
}
