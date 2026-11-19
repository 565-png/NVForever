//! The glowing parts of a model, for the distant lights of statics (nvforever
//! `distant_lights.rs`, docs/design/graphics-enhanced.md §3.17, #3013): an enhanced-look
//! feature of NVForever's own, not a fact about the original game.
//!
//! A model qualifies by its shapes' properties, as the renderer already reads them
//! (`nif_spawn::shape_props`, `shape_alpha_mode`, `shape_emissive_mult`), never by its file
//! name ([`glow_shape`]):
//!
//! - an unlit shape (BSShaderNoLightingProperty) added to the frame (NiAlphaProperty
//!   destination ONE, `AlphaMode::Add`): glow cards and light beams, the shapes drawn with
//!   the soft intersections and the display-value sum of additive shapes (nif.md §6
//!   "Additive blend");
//! - an unlit shape, opaque, cut out or blended, whose emitted light is over white after
//!   the picture rule (emissive colour × the multiplier the renderer uses, over 1 in the
//!   brightest channel: what `nif_material.wesl` burns as a hot source and the neon bright
//!   pass picks up, nif.md §6 "Emissive colour of unlit shapes"), not a picture or smoke;
//! - a lit shape whose emissive colour × multiplier reaches 1: self-lit at full emission,
//!   over its glow map if it has one, else over its whole surface (a lamp's bulb, the glow
//!   panel of a sign) [design: the threshold is ours].
//!
//! Apart from the added unlit shapes (glow cards and beams, which may be long), a shape
//! whose bounding sphere is wider than [`MAX_GLOW_RADIUS`] does not count: a lit window band
//! across a building's whole wall is the building's own picture, not a lamp [design]. A
//! model with one small glowing part among ordinary geometry (a lamp post) qualifies
//! through that part, and its glow sits at that part ([`ModelGlow::centre`]).
//!
//! Helper geometry the renderer leaves out (`Props::is_helper`) and hidden shapes do not
//! count, nor a model whose glowing shapes together show less than [`MIN_VISIBLE`]; a
//! shape whose texture is known dark shows nothing ([`shape_light`]).
//!
//! Read on a worker thread from the file alone (no meshes, materials or textures made for
//! the GPU): the NIF's blocks, and of the glowing shapes' textures one small mip level
//! ([`texture_look`]).

use crate::data::GameData;
use crate::nif_spawn::{affine, is_editor_marker, shaded_model, shape_alpha_mode, shape_emissive_mult, shape_props};
use bevy::math::{Affine3A, Vec3};
use bevy::prelude::AlphaMode;
use fnv_dds::{to_rgba8, Dds, Format};
use fnv_nif::scene::{scene_objects, Decoded, SceneObject, SHAPE_TYPES};
use fnv_nif::NifFile;
use std::collections::HashMap;

/// The widest glowing shape (bounding-sphere radius, game units) that counts as a light
/// [design: a lamp's glow card or bulb is tens of units; 512 is an eighth of a cell].
pub const MAX_GLOW_RADIUS: f32 = 512.0;

/// What [`glow_shape`] reads of one shape.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShapeTraits {
    pub unlit: bool,
    pub alpha: AlphaMode,
    /// The stored emissive colour (display values) and the multiplier the renderer uses
    /// (`shape_emissive_mult`: held for unlit pictures).
    pub emissive: [f32; 3],
    pub mult: f32,
    /// The shape's bounding-sphere radius in the model's space.
    pub radius: f32,
}

/// Whether a shape is a light source by its properties (module notes).
pub fn glow_shape(t: &ShapeTraits) -> bool {
    let light = t.emissive.iter().copied().fold(0.0f32, f32::max) * t.mult;
    let small = t.radius <= MAX_GLOW_RADIUS;
    match (t.unlit, t.alpha) {
        (_, AlphaMode::Multiply) => false,
        (true, AlphaMode::Add) => true,
        (true, _) => small && light > 1.0,
        (false, _) => small && light >= 1.0,
    }
}

/// The least visible light a model's glowing shapes must give together to count as a
/// light: the sum over them of brightness (the glowing texture's mean, times the emission)
/// × radius² [design: a bright bulb 5 units in radius, or a soft glow card of mean 0.05 and
/// radius 23; a speck of emissive trim gives less].
pub const MIN_VISIBLE: f32 = 25.0;

/// One glowing shape: its centre and radius in the model's space (below the root, as
/// `PreparedModel` shapes are), its colour (display values, brightest channel 1), and how
/// much light it shows (brightness × radius²).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlowPart {
    pub centre: Vec3,
    pub radius: f32,
    pub colour: [f32; 3],
    pub visible: f32,
}

/// A model's glow: where its glowing shapes sit (their centre weighted by the light they
/// show, in the model's space below the root: a placed reference's transform takes it to the
/// world), how far they reach from it, their colour (display values, brightest channel 1),
/// and how many shapes glow.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModelGlow {
    pub centre: [f32; 3],
    pub radius: f32,
    pub colour: [f32; 3],
    pub shapes: usize,
}

/// The glow of a model's glowing parts; `None` without any, or when together they show less
/// than [`MIN_VISIBLE`].
pub fn combine(parts: &[GlowPart]) -> Option<ModelGlow> {
    let total: f32 = parts.iter().map(|p| p.visible.max(0.0)).sum();
    if parts.is_empty() || total.is_nan() || total < MIN_VISIBLE {
        return None;
    }
    let weight = |p: &GlowPart| p.visible.max(0.0);
    let centre = parts.iter().fold(Vec3::ZERO, |a, p| a + p.centre * weight(p)) / total;
    let radius = parts.iter().map(|p| p.centre.distance(centre) + p.radius).fold(0.0f32, f32::max);
    let colour = hue(parts.iter().fold(Vec3::ZERO, |a, p| a + Vec3::from_array(p.colour) * weight(p)) / total);
    Some(ModelGlow { centre: centre.to_array(), radius, colour, shapes: parts.len() })
}

/// A colour scaled so its brightest channel is 1 (white for black).
fn hue(c: Vec3) -> [f32; 3] {
    let top = c.max_element();
    if top > 0.0 { (c / top).to_array() } else { [1.0; 3] }
}

/// What a texture shows, read from one of its mip levels ([`texture_look`]): its mean
/// luminance, with and without its alpha as a weight, and the matching mean hues (weighted
/// by luminance, or luminance × alpha; white where nothing is lit).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextureLook {
    pub lum: f32,
    pub lum_alpha: f32,
    pub hue: [f32; 3],
    pub hue_alpha: [f32; 3],
}

impl TextureLook {
    /// The brightness and hue a shape shows: weighted by the alpha when it blends or adds.
    pub fn seen(&self, blended: bool) -> (f32, [f32; 3]) {
        if blended { (self.lum_alpha, self.hue_alpha) } else { (self.lum, self.hue) }
    }
}

/// A texture darker than this on average shows nothing (one byte step) [design].
pub const DARK: f32 = 1.0 / 255.0;

/// Texels read at most: the first mip level with no more than this many is read whole
/// (256 x 256), not the top level subsampled [design].
pub const LOOK_TEXELS: usize = 65_536;

/// The four colours of a DXT colour block and its sixteen texels' colours (alpha 1, or 0
/// for DXT1's transparent selector in its three-colour mode) [documented: Microsoft,
/// "Block Compression (Direct3D 10)", BC1: c0 ≤ c1 selects the three-colour mode with
/// index 3 transparent black; BC2 and BC3 colour blocks always use four colours].
fn dxt_colours(b: &[u8], dxt1: bool) -> [[f32; 4]; 16] {
    let rgb = |v: u16| Vec3::new(((v >> 11) & 31) as f32 / 31.0, ((v >> 5) & 63) as f32 / 63.0, (v & 31) as f32 / 31.0);
    let (r0, r1) = (u16::from_le_bytes([b[0], b[1]]), u16::from_le_bytes([b[2], b[3]]));
    let (c0, c1) = (rgb(r0), rgb(r1));
    let palette: [[f32; 4]; 4] = if !dxt1 || r0 > r1 {
        [c0.extend(1.0).to_array(), c1.extend(1.0).to_array(), ((2.0 * c0 + c1) / 3.0).extend(1.0).to_array(), ((c0 + 2.0 * c1) / 3.0).extend(1.0).to_array()]
    } else {
        [c0.extend(1.0).to_array(), c1.extend(1.0).to_array(), ((c0 + c1) / 2.0).extend(1.0).to_array(), [0.0; 4]]
    };
    let sel = u32::from_le_bytes([b[4], b[5], b[6], b[7]]);
    std::array::from_fn(|i| palette[((sel >> (2 * i)) & 3) as usize])
}

/// The sixteen alphas of a DXT3 (explicit 4-bit) alpha block.
fn dxt3_alpha(b: &[u8]) -> [f32; 16] {
    let bits = u64::from_le_bytes(b[..8].try_into().expect("8 bytes"));
    std::array::from_fn(|i| ((bits >> (4 * i)) & 15) as f32 / 15.0)
}

/// The sixteen alphas of a DXT5 (interpolated) alpha block [documented: Microsoft, "Block
/// Compression (Direct3D 10)", BC3: eight values between the two ends when a0 > a1, else six
/// with 0 and 255].
fn dxt5_alpha(b: &[u8]) -> [f32; 16] {
    let (a0, a1) = (b[0] as f32, b[1] as f32);
    let mut pal = [0.0f32; 8];
    pal[0] = a0;
    pal[1] = a1;
    if a0 > a1 {
        for (i, v) in pal.iter_mut().enumerate().skip(2) {
            let k = (i - 1) as f32;
            *v = ((7.0 - k) * a0 + k * a1) / 7.0;
        }
    } else {
        for (i, v) in pal.iter_mut().enumerate().skip(2).take(4) {
            let k = (i - 1) as f32;
            *v = ((5.0 - k) * a0 + k * a1) / 5.0;
        }
        pal[6] = 0.0;
        pal[7] = 255.0;
    }
    let mut bits = 0u64;
    for (k, &x) in b[2..8].iter().enumerate() {
        bits |= (x as u64) << (8 * k);
    }
    std::array::from_fn(|i| pal[((bits >> (3 * i)) & 7) as usize] / 255.0)
}

/// What a DDS texture shows ([`TextureLook`]), from the first mip level of at most
/// [`LOOK_TEXELS`] texels, every texel of it (without such a level, a grid of the smallest
/// level's blocks or texels of about that many), blocks decoded with their selectors and
/// an edge block's texels past the image left out. `None` when the file does not parse or its format is not
/// read here (BC5): unreadable, which is not the same as dark.
pub fn texture_look(bytes: &[u8]) -> Option<TextureLook> {
    let dds = Dds::parse(bytes).ok()?;
    let mips = dds.levels.first()?;
    let (w0, h0) = (dds.header.width as usize, dds.header.height as usize);
    let side = |m: usize| ((w0 >> m).max(1), (h0 >> m).max(1));
    // The first level small enough is read whole; without one, a grid of the smallest
    // level's blocks or texels, at most about LOOK_TEXELS texels.
    let small = (0..mips.len()).find(|&m| side(m).0 * side(m).1 <= LOOK_TEXELS);
    let m = small.unwrap_or(mips.len().checked_sub(1)?);
    let (level, (w, h)) = (mips[m], side(m));
    let stride = |units: usize, per: usize| if small.is_some() { 1 } else { ((units as f64 / (LOOK_TEXELS / per).max(1) as f64).sqrt().ceil() as usize).max(1) };
    let (mut n, mut lum, mut lum_a, mut sum, mut sum_a) = (0usize, 0.0f32, 0.0f32, Vec3::ZERO, Vec3::ZERO);
    let mut add = |c: [f32; 4]| {
        let rgb = Vec3::new(c[0], c[1], c[2]);
        let l = 0.2126 * rgb.x + 0.7152 * rgb.y + 0.0722 * rgb.z;
        n += 1;
        lum += l;
        lum_a += l * c[3];
        sum += rgb * l;
        sum_a += rgb * (l * c[3]);
    };
    match dds.format {
        Format::Dxt1 | Format::Dxt3 | Format::Dxt5 => {
            let size = dds.format.block_bytes()?;
            let (across, down) = (w.div_ceil(4), h.div_ceil(4));
            let step = stride(across * down, 16);
            for by in (0..down).step_by(step) {
                for bx in (0..across).step_by(step) {
                    let Some(b) = level.get((by * across + bx) * size..(by * across + bx + 1) * size) else { continue };
                    let (colours, alpha) = match dds.format {
                        Format::Dxt1 => (dxt_colours(b, true), None),
                        Format::Dxt3 => (dxt_colours(&b[8..16], false), Some(dxt3_alpha(b))),
                        _ => (dxt_colours(&b[8..16], false), Some(dxt5_alpha(b))),
                    };
                    for (i, mut c) in colours.into_iter().enumerate() {
                        // An edge block's texels past the image are padding.
                        if bx * 4 + i % 4 >= w || by * 4 + i / 4 >= h {
                            continue;
                        }
                        if let Some(a) = alpha {
                            c[3] = a[i];
                        }
                        add(c);
                    }
                }
            }
        }
        Format::Bc5U | Format::Bc5S => return None,
        f if small.is_some() => {
            let rgba = to_rgba8(f, level)?;
            for p in rgba.as_chunks::<4>().0 {
                add([p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0, p[3] as f32 / 255.0]);
            }
        }
        f => {
            // The sampled texels' bytes gathered once and converted in one call.
            let bpp = f.level_size(1, 1);
            let step = stride(w * h, 1);
            let mut picked = Vec::with_capacity(LOOK_TEXELS * bpp);
            for y in (0..h).step_by(step) {
                for x in (0..w).step_by(step) {
                    let at = (y * w + x) * bpp;
                    if let Some(t) = level.get(at..at + bpp) {
                        picked.extend_from_slice(t);
                    }
                }
            }
            for p in to_rgba8(f, &picked)?.as_chunks::<4>().0 {
                add([p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0, p[3] as f32 / 255.0]);
            }
        }
    }
    let n = n.max(1) as f32;
    Some(TextureLook { lum: lum / n, lum_alpha: lum_a / n, hue: hue(sum), hue_alpha: hue(sum_a) })
}

/// The glow of the model at `path` (an archive path, `meshes\…`), read from its file;
/// `None` when no shape glows or the file cannot be read. Runs on a worker thread.
pub fn model_glow(data: &GameData, path: &str) -> Option<ModelGlow> {
    crate::guard::guarded(|| Ok::<_, String>(read_glow(data, path))).ok().flatten()
}

/// The brightness a glowing shape shows from its texture (`look`: `None` when it names
/// none or the texture is unreadable, which counts as fully bright) and its emission (an
/// unlit shape's surface colour at least, a lit shape's emitted light), and its hue; `None`
/// when the texture is known dark (it shows nothing).
pub fn shape_light(look: Option<TextureLook>, blended: bool, unlit: bool, light: f32) -> Option<(f32, [f32; 3])> {
    let (bright, tex_hue) = match look {
        Some(l) => l.seen(blended),
        None => (1.0, [1.0; 3]),
    };
    if look.is_some() && bright < DARK {
        return None;
    }
    let emission = if unlit { light.max(1.0) } else { light };
    Some((bright * emission, tex_hue))
}

fn read_glow(data: &GameData, path: &str) -> Option<ModelGlow> {
    let bytes = data.read(path)?;
    let schema = data.schema.clone();
    let file = NifFile::parse(&bytes, Some(&schema)).ok()?;
    let dec = Decoded::new(&file, &bytes, &schema);
    let objects = scene_objects(&file, &dec);
    let by_block: HashMap<usize, &SceneObject> = objects.iter().map(|o| (o.block, o)).collect();
    let hidden = |b: usize| dec.blocks[b].as_ref().ok().and_then(|s| s.int("flags")).unwrap_or(0) & 1 != 0 || is_editor_marker(by_block.get(&b).and_then(|o| o.name.as_deref()));
    // The renderer's own exclusion of helper geometry (model.rs, `Props::is_helper`).
    let shaded = shaded_model(&file, &dec);
    let mut textures: HashMap<String, Option<TextureLook>> = HashMap::new();
    let mut parts = Vec::new();
    for o in objects.iter().filter(|o| !o.is_node && SHAPE_TYPES.contains(&o.type_name.as_str())) {
        // The transform below the root's own, as model.rs's shapes have it; hidden
        // shapes (or under a hidden node) do not count.
        let mut t = if o.parent.is_some() { affine(&o.transform) } else { Affine3A::IDENTITY };
        let mut shown = !hidden(o.block);
        let mut cur = o.parent;
        let mut depth = 0;
        while let Some(p) = cur {
            let Some(po) = by_block.get(&p) else { break };
            if po.parent.is_some() {
                t = affine(&po.transform) * t;
            }
            shown &= !hidden(p);
            cur = po.parent;
            depth += 1;
            if depth > 512 {
                break;
            }
        }
        if !shown {
            continue;
        }
        let Ok(shape) = &dec.blocks[o.block] else { continue };
        let Some(sphere) = shape.block_ref("data").and_then(|r| dec.get(r)).and_then(|d| d.strukt("bounding_sphere")) else { continue };
        let centre: [f32; 3] = sphere.floats("center").and_then(|c| c.try_into().ok()).unwrap_or([0.0; 3]);
        let scale = t.matrix3.x_axis.length();
        let radius = sphere.float("radius").unwrap_or(0.0) * scale;
        let props = shape_props(&file, &dec, o, &by_block);
        if props.is_helper(shaded) {
            continue;
        }
        let alpha = shape_alpha_mode(&props);
        let (emissive, _) = props.emissive_parts();
        let traits = ShapeTraits { unlit: props.unlit_part(), alpha, emissive, mult: shape_emissive_mult(&props, alpha), radius };
        if !glow_shape(&traits) {
            continue;
        }
        // The texture that glows (a lit shape's glow map, else its own texture): its look,
        // or none when the shape names none or it cannot be read.
        let raw = if traits.unlit { None } else { props.glow_part() }.or(props.textures.first().map(String::as_str));
        let look = raw.filter(|r| !r.is_empty()).and_then(|r| {
            let p = fnv_vfs::texture_path(r);
            *textures.entry(p.clone()).or_insert_with(|| data.read(&p).and_then(|b| texture_look(&b)))
        });
        let blended = matches!(alpha, AlphaMode::Add | AlphaMode::Blend | AlphaMode::Premultiplied);
        let light = emissive.iter().copied().fold(0.0f32, f32::max) * traits.mult;
        let Some((bright, tex_hue)) = shape_light(look, blended, traits.unlit, light) else { continue };
        let e = if emissive.iter().any(|&c| c > 0.0) { Vec3::from_array(hue(Vec3::from_array(emissive))) } else { Vec3::ONE };
        let colour = hue(e * Vec3::from_array(tex_hue));
        parts.push(GlowPart { centre: t.transform_point3(Vec3::from_array(centre)), radius, colour, visible: bright * radius * radius });
    }
    combine(&parts)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn traits(unlit: bool, alpha: AlphaMode, emissive: [f32; 3], mult: f32) -> ShapeTraits {
        ShapeTraits { unlit, alpha, emissive, mult, radius: 40.0 }
    }

    #[test]
    fn light_sources_by_their_properties() {
        // A glow card: unlit and added; a long light beam too.
        assert!(glow_shape(&traits(true, AlphaMode::Add, [0.0; 3], 1.0)));
        assert!(glow_shape(&ShapeTraits { radius: 1300.0, ..traits(true, AlphaMode::Add, [1.0; 3], 1.0) }));
        // Neon: unlit, emitting over white.
        assert!(glow_shape(&traits(true, AlphaMode::Opaque, [1.0, 0.3, 0.2], 25.0)));
        // A lamp's bulb or a sign's glow panel: lit at full emission, blended or not.
        assert!(glow_shape(&traits(false, AlphaMode::Opaque, [1.0, 0.9, 0.7], 1.0)));
        assert!(glow_shape(&traits(false, AlphaMode::Blend, [1.0; 3], 1.0)));
        // An ordinary static: lit, no emission.
        assert!(!glow_shape(&traits(false, AlphaMode::Opaque, [0.0; 3], 1.0)));
        // Dim emission, a picture (unlit, held to 1), smoke (unlit, blended, not emitting
        // over white), a multiply shadow: none.
        assert!(!glow_shape(&traits(false, AlphaMode::Opaque, [0.3; 3], 1.0)));
        assert!(!glow_shape(&traits(true, AlphaMode::Opaque, [1.0; 3], 1.0)));
        assert!(!glow_shape(&traits(true, AlphaMode::Blend, [0.5; 3], 1.0)));
        assert!(!glow_shape(&traits(true, AlphaMode::Multiply, [1.0; 3], 25.0)));
        // A self-lit band as wide as a building: its own picture, not a lamp.
        assert!(!glow_shape(&ShapeTraits { radius: 2000.0, ..traits(false, AlphaMode::Opaque, [1.0; 3], 1.0) }));
    }

    fn part(centre: Vec3, radius: f32, colour: [f32; 3], bright: f32) -> GlowPart {
        GlowPart { centre, radius, colour, visible: bright * radius * radius }
    }

    #[test]
    fn a_lamp_post_glows_at_its_bulb() {
        // The post is no glow part; the bulb, 300 up, is.
        let bulb = part(Vec3::new(0.0, 0.0, 300.0), 10.0, [1.0, 0.8, 0.5], 1.0);
        let g = combine(&[bulb]).unwrap();
        assert_eq!((g.centre, g.radius, g.colour, g.shapes), ([0.0, 0.0, 300.0], 10.0, [1.0, 0.8, 0.5], 1));
        // Two cards: weighted by the light they show, reaching over both.
        let a = part(Vec3::ZERO, 30.0, [1.0, 1.0, 1.0], 1.0);
        let b = part(Vec3::new(100.0, 0.0, 0.0), 10.0, [1.0, 0.0, 0.0], 1.0);
        let g = combine(&[a, b]).unwrap();
        assert!((g.centre[0] - 10.0).abs() < 1e-4, "{:?}", g.centre);
        assert!((g.radius - 100.0).abs() < 1e-4);
        assert_eq!(g.colour[0], 1.0);
        assert!(g.colour[1] < 1.0 && g.colour[1] > 0.8);
        assert!(combine(&[]).is_none());
    }

    /// Emissive trim or a speck of a helper's size shows too little to be a light.
    #[test]
    fn a_tiny_or_dim_glow_does_not_count() {
        assert!(combine(&[part(Vec3::ZERO, 2.0, [1.0; 3], 1.0)]).is_none(), "4 under 25");
        assert!(combine(&[part(Vec3::ZERO, 100.0, [1.0; 3], 0.001)]).is_none(), "10 under 25");
        assert!(combine(&[part(Vec3::ZERO, 5.0, [1.0; 3], 1.0)]).is_some(), "a bright bulb 5 in radius");
    }

    /// A known-dark texture shows nothing; an unreadable one (or none) counts as bright.
    #[test]
    fn dark_is_not_unreadable() {
        let dark = TextureLook { lum: 0.0, lum_alpha: 0.0, hue: [1.0; 3], hue_alpha: [1.0; 3] };
        assert_eq!(shape_light(Some(dark), true, true, 0.0), None);
        assert_eq!(shape_light(None, true, true, 0.0), Some((1.0, [1.0; 3])));
        // Transparent but bright: dark when the shape blends, lit when it is opaque.
        let clear = TextureLook { lum: 0.5, lum_alpha: 0.0, hue: [1.0, 0.5, 0.0], hue_alpha: [1.0; 3] };
        assert_eq!(shape_light(Some(clear), true, true, 0.0), None);
        assert_eq!(shape_light(Some(clear), false, false, 2.0), Some((1.0, [1.0, 0.5, 0.0])));
    }

    fn dxt1(c0: u16, c1: u16, sel: u32) -> Vec<u8> {
        [c0.to_le_bytes().as_slice(), &c1.to_le_bytes(), &sel.to_le_bytes()].concat()
    }

    #[test]
    fn texture_look_reads_the_selectors() {
        let orange = (31u16 << 11) | (32 << 5);
        // Ends orange and black, every texel selecting the orange end: orange at full
        // luminance; the midpoint of the ends would have given half.
        let bytes = crate::texture::bc5_tests::dds(b"DXT1", 4, 4, 1, &dxt1(orange, 0, 0));
        let l = texture_look(&bytes).expect("read");
        assert_eq!(l.hue[0], 1.0);
        assert!((l.hue[1] - 32.0 / 63.0).abs() < 1e-4 && l.hue[2] == 0.0, "{:?}", l.hue);
        let full = 0.2126 + 0.7152 * 32.0 / 63.0;
        assert!((l.lum - full).abs() < 1e-4, "{}", l.lum);
        // Every texel selecting the black end: known dark, not unreadable.
        let black = crate::texture::bc5_tests::dds(b"DXT1", 4, 4, 1, &dxt1(orange, 0, 0x5555_5555));
        assert!(texture_look(&black).expect("read").lum < DARK);
        // Three-colour mode (c0 <= c1), selector 3: transparent black.
        let clear = crate::texture::bc5_tests::dds(b"DXT1", 4, 4, 1, &dxt1(0, orange, 0xFFFF_FFFF));
        let c = texture_look(&clear).expect("read");
        assert!(c.lum < DARK && c.lum_alpha < DARK);
        assert!(texture_look(b"not a dds").is_none(), "unreadable");
    }

    /// A 2 x 2 texture's one block: its 12 padding texels do not count.
    #[test]
    fn edge_block_padding_is_left_out() {
        let orange = (31u16 << 11) | (32 << 5);
        // Texels 0, 1, 4, 5 (inside 2 x 2) select orange, the rest black.
        let sel: u32 = (0..16).filter(|i| !matches!(i, 0 | 1 | 4 | 5)).fold(0, |s, i| s | (1 << (2 * i)));
        let bytes = crate::texture::bc5_tests::dds(b"DXT1", 2, 2, 1, &dxt1(orange, 0, sel));
        let l = texture_look(&bytes).expect("read");
        let full = 0.2126 + 0.7152 * 32.0 / 63.0;
        assert!((l.lum - full).abs() < 1e-4, "{} against {full}: padding counted", l.lum);
    }

    /// A large texture without small mips is sampled on a grid, not decoded whole.
    #[test]
    fn a_large_texture_without_mips_is_sampled() {
        let orange = (31u16 << 11) | (32 << 5);
        let block = dxt1(orange, 0, 0);
        let payload: Vec<u8> = std::iter::repeat_n(block.as_slice(), 256 * 256).flatten().copied().collect();
        let bytes = crate::texture::bc5_tests::dds(b"DXT1", 1024, 1024, 1, &payload);
        let l = texture_look(&bytes).expect("read");
        assert!((l.lum - (0.2126 + 0.7152 * 32.0 / 63.0)).abs() < 1e-4);
        assert_eq!(l.hue[0], 1.0);
    }

    #[test]
    fn dxt5_alpha_palette() {
        // a0 255, a1 0, texel 0 index 0 (255), texel 1 index 1 (0), texel 2 index 7 (1/7 of 255).
        let bits: u64 = (1 << 3) | (7 << 6);
        let mut b = vec![255u8, 0];
        b.extend(&bits.to_le_bytes()[..6]);
        let a = dxt5_alpha(&b);
        assert_eq!((a[0], a[1]), (1.0, 0.0));
        assert!((a[2] - 1.0 / 7.0).abs() < 1e-5, "{}", a[2]);
        // a0 <= a1: index 6 is 0 and 7 is 255.
        let bits: u64 = 6 | (7 << 3);
        let mut b = vec![10u8, 20];
        b.extend(&bits.to_le_bytes()[..6]);
        let a = dxt5_alpha(&b);
        assert_eq!((a[0], a[1]), (0.0, 1.0));
    }
}
