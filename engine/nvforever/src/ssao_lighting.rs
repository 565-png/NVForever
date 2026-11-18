//! Ambient occlusion on the ambient light, inside the lighting (#2897 stage two,
//! docs/design/graphics-enhanced.md §3.1): instead of multiplying the finished picture
//! (ssao.rs's passes), the occlusion factor scales only the display-value ambient term of
//! the patched `apply_pbr_lighting` (lights/display_shader.rs `nvf_ambient`), so direct
//! light, the sun and every highlight stay as they are. An engineering choice, not the
//! original game's.
//!
//! The parts, built the way the sun visibility's are (sun_visibility.rs):
//! - bevy_pbr's view component `NvfAmbientOcclusionBindings` (vendored, engine/vendor/README.md):
//!   a uniform ([`AmbientOcclusionLightingUniform`]) and the half-resolution occlusion at
//!   mesh view bindings 43-44, which gives the view's depth-writing pipelines the shader def
//!   `NVF_AMBIENT_OCCLUSION`;
//! - ssao_lighting.wesl, imported as `nvforever::ssao_lighting` under that def:
//!   `nvf_ambient_visibility`, a depth-aware 2 x 2 upsample of the half-resolution texels;
//! - [`HOOK_EDITS`], patched into `apply_pbr_lighting` by lights.rs right after the sun
//!   visibility's hook: every statement it adds is under `@if(NVF_AMBIENT_OCCLUSION)`, and
//!   the one line it replaces is kept word for word under `@if(!NVF_AMBIENT_OCCLUSION)`, so
//!   without the def the shader text a pipeline compiles is what it was before.
//!
//! Nothing inserts the bindings yet: the render-side producer (the pass that fills the
//! texture, the extraction that inserts the component) comes with the next stages, and
//! until then no pipeline has the def.

use bevy::asset::{load_internal_asset, uuid_handle};
use bevy::prelude::*;
use bevy::render::render_resource::ShaderType;
use bevy::shader::Shader;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::sun_visibility::Place;

/// ssao_lighting.wesl, imported by the patched `apply_pbr_lighting` as
/// `nvforever::ssao_lighting`.
const SHADER: Handle<Shader> = uuid_handle!("8d1e6a2c-3f47-4b9e-a5c1-6e02d7f4b318");

/// The exponent the factor is raised to before it scales the display-value ambient: the
/// display sum is raised to 2.2 afterwards (display_shader.rs), so in a pixel lit by the
/// ambient alone the light is scaled by the factor itself, as stage one scaled it
/// [engineering choice, #2897 stage two plan §5; the uniform carries it so a picture pair can
/// settle it].
pub const EXPONENT: f32 = 1.0 / 2.2;

/// The share of the fragment's distance (plus one unit) within which a texel's distance
/// counts as the same surface: stage one's fallback tolerance (ssao.wesl
/// `FALLBACK_TOLERANCE`) [engineering choice].
pub const DISTANCE_TOLERANCE: f32 = 0.05;

/// `NvfAmbientOcclusion` of ssao_lighting.wesl (field for field).
#[derive(Clone, Copy, Debug, Default, PartialEq, ShaderType)]
#[cfg_attr(not(test), allow(dead_code))] // the render-side producer (#2897 stage two, C4) fills it
pub struct AmbientOcclusionLightingUniform {
    /// Exponent, debug view (1 on), fade end (game units), distance tolerance.
    pub params: Vec4,
    /// Strength (for a later multibounce); the rest unused.
    pub extra: Vec4,
}

#[cfg_attr(not(test), allow(dead_code))] // the render-side producer (#2897 stage two, C4) fills it
impl AmbientOcclusionLightingUniform {
    /// The uniform for an occlusion faded out at `fade_end` (game units), made with
    /// `strength`, with or without the debug view.
    pub fn new(fade_end: f32, strength: f32, debug: bool) -> Self {
        AmbientOcclusionLightingUniform {
            params: Vec4::new(EXPONENT, if debug { 1.0 } else { 0.0 }, fade_end, DISTANCE_TOLERANCE),
            extra: Vec4::new(strength, 0.0, 0.0, 0.0),
        }
    }
}

/// Set once lights.rs has put [`HOOK_EDITS`] into `apply_pbr_lighting`; without it the
/// occlusion must stay on the picture (stage one's placement).
static LIGHTING_HOOKED: AtomicBool = AtomicBool::new(false);

/// Record that the hook is in place (lights.rs `patch_display_sum`).
pub(crate) fn set_lighting_hooked() {
    LIGHTING_HOOKED.store(true, Ordering::Relaxed);
}

/// Whether the hook is in place, so the occlusion can be put in the lighting.
#[allow(dead_code)] // read by the mode logic in ssao.rs (#2897 stage two, C4)
pub(crate) fn lighting_hooked() -> bool {
    LIGHTING_HOOKED.load(Ordering::Relaxed)
}

pub struct SsaoLightingPlugin;

impl Plugin for SsaoLightingPlugin {
    fn build(&self, app: &mut App) {
        load_internal_asset!(app, SHADER, "ssao_lighting.wesl", |s, _| Shader::from_wesl(s, "embedded://nvforever/ssao_lighting.wesl"));
    }
}

/// The display-value ambient line of lights/display_shader.rs `DISPLAY_EDITS`, as it
/// stands in the patched `apply_pbr_lighting`.
const AMBIENT_LINE: &str = "        let nvf_ambient = view_bindings::lights.ambient_color.rgb * view_bindings::view.exposure * diffuse_occlusion;\n";

/// The same line kept without the def, and its twin with the occlusion under it. `-view_z`
/// is the fragment's view distance from its own world position (so the first-person arms,
/// drawn at a squeezed depth, still compare their real distance with the texels', which the
/// occlusion pass unsqueezed).
const AMBIENT_LINE_HOOKED: &str = "        // NVForever ambient occlusion (nvforever ssao_lighting.rs): on the display-value
        // ambient alone, never on the direct lights nor on Bevy's ambient highlight above.
        @if(!NVF_AMBIENT_OCCLUSION)
        let nvf_ambient = view_bindings::lights.ambient_color.rgb * view_bindings::view.exposure * diffuse_occlusion;
        @if(NVF_AMBIENT_OCCLUSION)
        let nvf_ambient = view_bindings::lights.ambient_color.rgb * view_bindings::view.exposure * diffuse_occlusion * nvf_ambient_visibility(in.frag_coord.xy, -view_z);
";

/// The hook in bevy_pbr's `apply_pbr_lighting` [documented:
/// engine/vendor/bevy_pbr/src/render/pbr_functions.wesl (bevy_pbr 0.20.1) with lights.rs
/// `DISPLAY_EDITS` and sun_visibility.rs `HOOK_EDITS` made; the tests here apply all three
/// to that file], in order, each at the first occurrence of its anchor after the last edit:
/// (anchor, text, where the text goes). Deliberately left alone: Bevy's
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
    (
        "    return output_color;\n}\n",
        "    // NVForever ambient occlusion: the debug view draws the factor that reached the
    // ambient (fog still follows).
    @if(NVF_AMBIENT_OCCLUSION)
    if nvf_ao_debug() {
        output_color = vec4<f32>(vec3<f32>(nvf_ambient_visibility(in.frag_coord.xy, -view_z)), output_color.a);
    }

",
        Place::Before,
    ),
];

/// The index in [`HOOK_EDITS`] of the debug view's edit at the lighting's return line.
const RETURN_EDIT: usize = 2;

/// `source` (lights.rs's display-value patch and the sun visibility's hook made) with
/// [`HOOK_EDITS`]; `None` when an anchor is missing (Bevy or the display patch changed),
/// and the caller keeps `source`.
pub fn hook_patched(source: &str) -> Option<String> {
    let mut out = String::with_capacity(source.len() + 2048);
    let mut rest = source;
    for (i, (anchor, text, place)) in HOOK_EDITS.into_iter().enumerate() {
        let at = rest.find(anchor)?;
        // The debug view's return line is `apply_pbr_lighting`'s own: no other function
        // begins between the ambient line and it (the next function that ends so is
        // `main_pass_post_lighting_processing`).
        if i == RETURN_EDIT && rest[..at].contains("\nfn ") {
            return None;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lights::unix_lines;
    use crate::sun_visibility::tests::{balanced, block_after, code, code_lines, resolve_known};

    /// ssao_lighting.wesl's `NVF_AO_MIN_WEIGHT`: below it the factor is 1.
    const MIN_WEIGHT: f32 = 0.001;

    fn vendored(file: &str) -> String {
        let path = format!("{}/../vendor/bevy_pbr/src/render/{file}", env!("CARGO_MANIFEST_DIR"));
        unix_lines(&std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}")))
    }

    /// The vendored source with the display-value patch and the sun visibility's hook (as
    /// lights.rs makes them), then this hook.
    fn hooked() -> (String, String) {
        let display = crate::lights::display_patched(&vendored("pbr_functions.wesl")).expect("display patch applies");
        let sun = crate::sun_visibility::hook_patched(&display).expect("sun hook applies");
        let hooked = hook_patched(&sun).expect("ambient occlusion hook applies");
        (sun, hooked)
    }

    fn wesl() -> String {
        unix_lines(include_str!("ssao_lighting.wesl"))
    }

    /// Without the def nothing changes: each added text is comments and statements under
    /// `@if(NVF_AMBIENT_OCCLUSION)`, the replaced line is kept under its negation, and taking
    /// the additions out gives the input back word for word.
    #[test]
    fn hook_identity_without_the_def() {
        let (input, hooked) = hooked();
        assert_ne!(input, hooked);
        for (_, text, place) in HOOK_EDITS {
            if place == Place::Replace {
                continue;
            }
            let code: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with("//")).collect();
            assert!(code[0].starts_with("@if(NVF_AMBIENT_OCCLUSION)"), "{text}");
        }
        assert!(AMBIENT_LINE_HOOKED.contains(&format!("        @if(!NVF_AMBIENT_OCCLUSION)\n{AMBIENT_LINE}")), "the line kept word for word under the negation");
        let mut back = hooked.clone();
        for (anchor, text, place) in HOOK_EDITS {
            match place {
                Place::Before => back = back.replacen(&format!("{text}{anchor}"), anchor, 1),
                Place::After => back = back.replacen(&format!("{anchor}{text}"), anchor, 1),
                Place::Replace => back = back.replacen(text, anchor, 1),
            }
        }
        assert_eq!(back, input);
        // Resolved without the def, the code lines are the input's, with the sun's def on
        // or off.
        for sun in [false, true] {
            let known = [("NVF_AMBIENT_OCCLUSION", false), ("NVF_SUN_VISIBILITY", sun)];
            let want = resolve_known(&input, &known[1..]);
            let got = resolve_known(&hooked, &known);
            assert_eq!(code_lines(&got), code_lines(&want), "sun {sun}");
            assert!(!code(&got).contains("nvf_ambient_visibility") && !code(&got).contains("nvf_ao_debug"), "sun {sun}");
        }
    }

    /// Each anchor occurs once where the hook looks for it (in the display- and
    /// sun-patched source, from the previous edit on), and a missing one leaves the source
    /// alone.
    #[test]
    fn each_anchor_occurs_once() {
        let (input, _) = hooked();
        assert_eq!(input.matches(HOOK_EDITS[0].0).count(), 1, "the import line");
        assert_eq!(input.matches(AMBIENT_LINE).count(), 1, "the display-value ambient line");
        // The return line ends apply_pbr_lighting, the first function after the ambient
        // line that returns `output_color` so; the other functions that end so lie elsewhere.
        let after_ambient = &input[input.find(AMBIENT_LINE).expect("ambient line")..];
        let ret = after_ambient.find(HOOK_EDITS[2].0).expect("the return after the ambient line");
        assert!(after_ambient[..ret].find("\nfn ").is_none(), "the return is the same function's");
        for (anchor, _, _) in HOOK_EDITS {
            assert!(hook_patched(&input.replacen(anchor, "", 1)).is_none(), "missing {anchor:?}");
        }
    }

    /// The two lines left alone (module notes, HOOK_EDITS) are the same before and after.
    #[test]
    fn bevy_ambient_and_diffuse_occlusion_untouched() {
        let (input, hooked) = hooked();
        for wanted in ["ambient::ambient_light(in.world_position, in.N, in.V, NdotV,", "    let diffuse_occlusion = in.diffuse_occlusion;\n"] {
            let lines = |s: &str| s.lines().filter(|l| l.contains(wanted.trim_end_matches('\n'))).map(str::to_string).collect::<Vec<_>>();
            let before = lines(&input);
            assert_eq!(before.len(), 1, "{wanted} once in the input");
            assert_eq!(lines(&hooked), before, "{wanted}");
        }
    }

    /// With the def the occlusion reaches the ambient line and the debug view alone: once
    /// in the ambient, once in the debug view, after `view_z` is declared, the import before
    /// the function; brackets close in every variant; the names imported are the WESL's.
    #[test]
    fn hook_resolves_with_and_without_the_def() {
        let (_, hooked) = hooked();
        assert!(balanced(&hooked));
        for (_, text, _) in HOOK_EDITS {
            assert!(balanced(text), "{text}");
        }
        for on in [false, true] {
            for sun in [false, true] {
                let known = [("NVF_AMBIENT_OCCLUSION", on), ("NVF_SUN_VISIBILITY", sun)];
                let r = resolve_known(&hooked, &known);
                let case = format!("ambient occlusion {on}, sun {sun}");
                assert!(!code(&r).contains("NVF_AMBIENT_OCCLUSION"), "{case}: every condition resolved");
                assert!(balanced(&r), "{case}");
                let lighting = block_after(&r, "fn nvf_apply_pbr_lighting(");
                assert!(!lighting.contains("\nfn "), "{case}: the lighting function ends where it should");
                assert_eq!(lighting.matches("let nvf_ambient = ").count(), 1, "{case}: one ambient line");
                assert_eq!(lighting.matches("nvf_ambient_visibility(in.frag_coord.xy, -view_z)").count(), if on { 2 } else { 0 }, "{case}");
                if on {
                    let at = |s: &str| lighting.find(s).unwrap_or_else(|| panic!("{case}: {s}"));
                    assert!(at("let view_z") < at("* nvf_ambient_visibility("), "{case}: view_z before the ambient line");
                    assert!(at("* nvf_ambient_visibility(") < at("    // Total light\n"), "{case}");
                    assert!(at("    // Total light\n") < at("if nvf_ao_debug()"), "{case}: the debug view after the sum");
                    if sun {
                        assert!(at("vec3<f32>(nvf_sun_debug)") < at("if nvf_ao_debug()"), "{case}: the debug view after the sun's, so it wins");
                    }
                    assert!(r.find("import nvforever::ssao_lighting::{nvf_ambient_visibility, nvf_ao_debug};").expect("import") < r.find("fn nvf_apply_pbr_lighting(").unwrap());
                }
            }
        }
        let wesl = wesl();
        for f in ["fn nvf_ambient_visibility(frag: vec2<f32>, dist: f32) -> f32", "fn nvf_ao_debug() -> bool"] {
            assert!(wesl.contains(f), "ssao_lighting.wesl has {f}");
        }
    }

    /// The WESL's bindings are the vendored layout's (mesh_view_bindings.rs 43-44, nobody
    /// else's in group 0), and its uniform the Rust struct, field for field.
    #[test]
    fn bindings_and_uniform_match() {
        let wesl = wesl();
        let bindings = vendored("mesh_view_bindings.rs");
        assert!(wesl.contains("@group(0) @binding(43) var<uniform> nvf_ao: NvfAmbientOcclusion;"));
        assert!(wesl.contains("@group(0) @binding(44) var nvf_ao_texture: texture_2d<f32>;"));
        assert!(bindings.contains("(43, uniform_buffer_sized(false, None)),"), "binding 43 a uniform");
        assert!(bindings.contains("                44,\n                texture_2d(TextureSampleType::Float { filterable: false }),"), "binding 44 an unfilterable float 2D texture");
        assert!(bindings.contains("(43, ao.uniform.as_entire_binding()), (44, &ao.texture)"), "the bind group's entries");
        // 43 and 44 are used once each in the layout (no other feature has them).
        for n in ["(43,", "                44,\n"] {
            assert_eq!(bindings.matches(n).count(), if n.starts_with('(') { 2 } else { 1 }, "{n:?}");
        }
        // The uniform: two vec4s in the Rust order.
        let fields: Vec<usize> = ["params: vec4<f32>,", "extra: vec4<f32>,"].iter().map(|f| wesl.find(f).unwrap_or_else(|| panic!("{f}"))).collect();
        assert!(fields.windows(2).all(|w| w[0] < w[1]));
        assert_eq!(AmbientOcclusionLightingUniform::min_size().get(), 32, "two vec4s, as ssao_lighting.wesl's struct");
        let u = AmbientOcclusionLightingUniform::new(7000.0, 0.4, true);
        assert_eq!(u.params, Vec4::new(EXPONENT, 1.0, 7000.0, DISTANCE_TOLERANCE));
        assert_eq!(u.extra, Vec4::new(0.4, 0.0, 0.0, 0.0));
        assert_eq!(AmbientOcclusionLightingUniform::new(7000.0, 0.4, false).params.y, 0.0);
        assert!(wesl.contains(&format!("const NVF_AO_MIN_WEIGHT: f32 = {MIN_WEIGHT};")), "the WESL's minimum weight is the Rust mirror's");
        // A zeroed uniform (fade end 0) reads as open everywhere.
        assert_eq!(upsample(&Texels::plane(4, 4, 0.5, 100.0), Vec2::new(2.5, 2.5), 100.0, &AmbientOcclusionLightingUniform::default()), 1.0);
    }

    /// ssao_lighting.wesl is valid WGSL by itself (it imports nothing, so no stand-ins are
    /// needed): naga parses and validates it, and both functions are there.
    #[test]
    fn the_shader_is_valid_wgsl() {
        use wgpu::naga;
        let source = wesl();
        assert!(!source.lines().any(|l| l.starts_with("import ") || l.trim_start().starts_with("@if")), "nothing for the test to resolve");
        let module = naga::front::wgsl::parse_str(&source).unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::default())
            .validate(&module)
            .unwrap_or_else(|e| panic!("{e:?}"));
        for f in ["nvf_ambient_visibility", "nvf_ao_debug"] {
            assert!(module.functions.iter().any(|(_, func)| func.name.as_deref() == Some(f)), "{f}");
        }
    }

    /// A half-resolution texture for the mirror: (how open, distance) per texel.
    struct Texels {
        size: (i32, i32),
        data: Vec<(f32, f32)>,
    }

    impl Texels {
        fn plane(w: i32, h: i32, open: f32, dist: f32) -> Self {
            Texels { size: (w, h), data: vec![(open, dist); (w * h) as usize] }
        }

        fn load(&self, x: i32, y: i32) -> (f32, f32) {
            self.data[(y * self.size.0 + x) as usize]
        }

        fn set(&mut self, x: i32, y: i32, v: (f32, f32)) {
            self.data[(y * self.size.0 + x) as usize] = v;
        }
    }

    /// ssao_lighting.wesl `nvf_ambient_visibility`, step for step.
    fn upsample(tex: &Texels, frag: Vec2, dist: f32, u: &AmbientOcclusionLightingUniform) -> f32 {
        // The shader's `!(dist > 0.0)`: NaN too.
        if dist.is_nan() || dist <= 0.0 || dist >= u.params.z {
            return 1.0;
        }
        let t = (frag - 0.5) * 0.5;
        let base = t.floor();
        let f = t - base;
        let first = (base.x as i32, base.y as i32);
        let tolerance = u.params.w * dist + 1.0;
        let (mut total, mut weight) = (0.0, 0.0);
        for y in 0..2 {
            for x in 0..2 {
                let q = ((first.0 + x).clamp(0, tex.size.0 - 1), (first.1 + y).clamp(0, tex.size.1 - 1));
                let (open, d) = tex.load(q.0, q.1);
                let bilinear = (if x == 1 { f.x } else { 1.0 - f.x }) * (if y == 1 { f.y } else { 1.0 - f.y });
                let surface = if d > 0.0 { (1.0 - (d - dist).abs() / tolerance).max(0.0) } else { 0.0 };
                let w = bilinear * surface;
                total += open * w;
                weight += w;
            }
        }
        if weight.is_nan() || weight < MIN_WEIGHT {
            return 1.0;
        }
        let open = total / weight;
        let safe = if open >= 0.0 { open.clamp(0.0001, 1.0) } else { 1.0 };
        safe.powf(u.params.x)
    }

    /// The factor as the display sum takes it back: a fully shaded pixel's light scales by
    /// (g)^2.2.
    fn linear(g: f32) -> f32 {
        g.powf(2.2)
    }

    fn uniform() -> AmbientOcclusionLightingUniform {
        AmbientOcclusionLightingUniform::new(7000.0, 0.4, false)
    }

    /// On a plane every fragment gets the plane's factor exactly (after the exponent and
    /// the display gamma, the factor itself), at even and odd pixels and at the borders.
    #[test]
    fn a_plane_gives_its_factor() {
        let tex = Texels::plane(8, 6, 0.7, 500.0);
        for frag in [Vec2::new(4.5, 4.5), Vec2::new(5.5, 4.5), Vec2::new(5.5, 5.5), Vec2::new(0.5, 0.5), Vec2::new(15.5, 11.5), Vec2::new(16.5, 12.5)] {
            let g = upsample(&tex, frag, 500.0, &uniform());
            assert!((linear(g) - 0.7).abs() < 1e-5, "{frag}: {}", linear(g));
        }
        // A slightly different distance on the same plane (a slope) still matches.
        let g = upsample(&tex, Vec2::new(5.5, 5.5), 510.0, &uniform());
        assert!((linear(g) - 0.7).abs() < 1e-5);
    }

    /// Across a depth edge each side keeps its own texels: the background beside a near
    /// box ignores the box's dark texels, and the box ignores the background's.
    #[test]
    fn across_an_edge_each_surface_keeps_its_own() {
        // Columns 0-1 a box at 200 units, closed (0.2); columns 2-3 a wall at 2000, open (1).
        let mut tex = Texels::plane(4, 4, 1.0, 2000.0);
        for y in 0..4 {
            for x in 0..2 {
                tex.set(x, y, (0.2, 200.0));
            }
        }
        // Full pixel 3.5 lies between texel 1 (box) and texel 2 (wall).
        let edge = Vec2::new(3.5, 3.5);
        let wall = upsample(&tex, edge, 2000.0, &uniform());
        assert_eq!(wall, 1.0, "the wall's fragment ignores the box");
        let boxed = upsample(&tex, edge, 200.0, &uniform());
        assert!((linear(boxed) - 0.2).abs() < 1e-4, "the box's fragment ignores the wall: {}", linear(boxed));
    }

    /// Where no texel holds the fragment's surface (something the depth prepass does not
    /// have: water over the bed, a pole a pixel wide) the factor is 1, not a neighbour's.
    #[test]
    fn no_match_gives_one() {
        let tex = Texels::plane(4, 4, 0.1, 3000.0);
        assert_eq!(upsample(&tex, Vec2::new(3.5, 3.5), 300.0, &uniform()), 1.0);
        // A texel on the surface but with no bilinear weight (pixel 2q exactly, its own
        // texel a different surface) does not count either.
        let mut tex = Texels::plane(4, 4, 0.1, 300.0);
        tex.set(1, 1, (0.1, 3000.0));
        assert_eq!(upsample(&tex, Vec2::new(2.5, 2.5), 300.0, &uniform()), 1.0);
    }

    /// From the fade's end on (where the occlusion pass writes 1 anyway) nothing is read.
    #[test]
    fn beyond_the_fade_gives_one() {
        let tex = Texels::plane(4, 4, 0.1, 7000.0);
        assert_eq!(upsample(&tex, Vec2::new(2.5, 2.5), 7000.0, &uniform()), 1.0);
        assert_eq!(upsample(&tex, Vec2::new(2.5, 2.5), 60000.0, &uniform()), 1.0);
        let tex = Texels::plane(4, 4, 0.1, 6999.0);
        assert!(upsample(&tex, Vec2::new(2.5, 2.5), 6999.0, &uniform()) < 1.0, "just inside the fade it is read");
        // No distance (or not a number) reads as open too.
        assert_eq!(upsample(&tex, Vec2::new(2.5, 2.5), 0.0, &uniform()), 1.0);
        assert_eq!(upsample(&tex, Vec2::new(2.5, 2.5), f32::NAN, &uniform()), 1.0);
    }

    /// A texture never written (all zero: distance 0) darkens nothing, even for a fragment
    /// right in front of the camera.
    #[test]
    fn zero_texels_give_one() {
        let tex = Texels::plane(4, 4, 0.0, 0.0);
        for dist in [0.5, 1.0, 5.0, 100.0, 6000.0] {
            assert_eq!(upsample(&tex, Vec2::new(3.5, 2.5), dist, &uniform()), 1.0, "{dist}");
        }
    }

    /// The exponent undoes the display gamma: (f^(1/2.2))^2.2 = f across the range.
    #[test]
    fn the_exponent_undoes_the_display_gamma() {
        for i in 1..=20 {
            let f = i as f32 / 20.0;
            assert!((linear(f.powf(EXPONENT)) - f).abs() < 1e-5, "{f}");
        }
    }
}
