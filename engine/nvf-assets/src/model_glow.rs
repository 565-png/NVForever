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
//! across a building's whole wall is the building's own picture, not a lamp [design]. A model with one small glowing part among ordinary geometry (a lamp post)
//! qualifies through that part, and its glow sits at that part ([`ModelGlow::centre`]).
//!
//! Read on a worker thread from the file alone (no meshes, materials or textures made for
//! the GPU): the NIF's blocks, and of the glowing shapes' textures only a mean colour from
//! the top level's blocks ([`texture_hue`]).

use crate::data::GameData;
use crate::nif_spawn::{affine, is_editor_marker, shape_alpha_mode, shape_emissive_mult, shape_props};
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

/// One glowing shape: its centre and radius in the model's space (below the root, as
/// `PreparedModel` shapes are), its colour (display values, brightest channel 1).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlowPart {
    pub centre: Vec3,
    pub radius: f32,
    pub colour: [f32; 3],
}

/// A model's glow: where its glowing shapes sit (their centre weighted by their area, in
/// the model's space below the root: a placed reference's transform takes it to the
/// world), how far they reach from it, their colour (display values, brightest channel
/// 1), and how many shapes glow.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModelGlow {
    pub centre: [f32; 3],
    pub radius: f32,
    pub colour: [f32; 3],
    pub shapes: usize,
}

/// The glow of a model's glowing parts; `None` without any.
pub fn combine(parts: &[GlowPart]) -> Option<ModelGlow> {
    if parts.is_empty() {
        return None;
    }
    let weight = |p: &GlowPart| (p.radius * p.radius).max(1.0);
    let total: f32 = parts.iter().map(weight).sum();
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

/// The mean colour of a DDS texture's top level, weighted by each part's brightness and
/// alpha, as a hue (brightest channel 1): from at most about 16,000 blocks or texels
/// spread over it. Compressed blocks count the middle of their two end colours. `None`
/// for files that do not parse, formats not read here, or a black texture.
pub fn texture_hue(bytes: &[u8]) -> Option<[f32; 3]> {
    let dds = Dds::parse(bytes).ok()?;
    let top = *dds.levels.first()?.first()?;
    let (w, h) = (dds.header.width as usize, dds.header.height as usize);
    let mut sum = Vec3::ZERO;
    let mut add = |c: Vec3, a: f32| {
        let lum = 0.2126 * c.x + 0.7152 * c.y + 0.0722 * c.z;
        sum += c * (lum * a);
    };
    let rgb565 = |v: u16| Vec3::new(((v >> 11) & 31) as f32 / 31.0, ((v >> 5) & 63) as f32 / 63.0, (v & 31) as f32 / 31.0);
    match dds.format {
        Format::Dxt1 | Format::Dxt3 | Format::Dxt5 => {
            let size = dds.format.block_bytes()?;
            let blocks = (w.div_ceil(4) * h.div_ceil(4)).min(top.len() / size);
            let step = blocks.div_ceil(16_384).max(1);
            for b in top.chunks_exact(size).take(blocks).step_by(step) {
                let (colour, alpha) = match dds.format {
                    Format::Dxt1 => (&b[..8], 1.0),
                    // Explicit 4-bit alphas: their mean.
                    Format::Dxt3 => (&b[8..16], b[..8].iter().map(|&x| (x & 15) as f32 + (x >> 4) as f32).sum::<f32>() / (16.0 * 15.0)),
                    // Interpolated alpha: the middle of its two ends.
                    _ => (&b[8..16], (b[0] as f32 + b[1] as f32) / (2.0 * 255.0)),
                };
                let c0 = rgb565(u16::from_le_bytes([colour[0], colour[1]]));
                let c1 = rgb565(u16::from_le_bytes([colour[2], colour[3]]));
                add((c0 + c1) * 0.5, alpha);
            }
        }
        Format::Bc5U | Format::Bc5S => return None,
        f => {
            let rgba = to_rgba8(f, top)?;
            let texels = rgba.len() / 4;
            let step = texels.div_ceil(16_384).max(1);
            for p in rgba.as_chunks::<4>().0.iter().step_by(step) {
                add(Vec3::new(p[0] as f32, p[1] as f32, p[2] as f32) / 255.0, p[3] as f32 / 255.0);
            }
        }
    }
    (sum.max_element() > 0.0).then(|| hue(sum))
}

/// The glow of the model at `path` (an archive path, `meshes\…`), read from its file;
/// `None` when no shape glows or the file cannot be read. Runs on a worker thread.
pub fn model_glow(data: &GameData, path: &str) -> Option<ModelGlow> {
    crate::guard::guarded(|| Ok::<_, String>(read_glow(data, path))).ok().flatten()
}

fn read_glow(data: &GameData, path: &str) -> Option<ModelGlow> {
    let bytes = data.read(path)?;
    let schema = data.schema.clone();
    let file = NifFile::parse(&bytes, Some(&schema)).ok()?;
    let dec = Decoded::new(&file, &bytes, &schema);
    let objects = scene_objects(&file, &dec);
    let by_block: HashMap<usize, &SceneObject> = objects.iter().map(|o| (o.block, o)).collect();
    let hidden = |b: usize| dec.blocks[b].as_ref().ok().and_then(|s| s.int("flags")).unwrap_or(0) & 1 != 0 || is_editor_marker(by_block.get(&b).and_then(|o| o.name.as_deref()));
    let mut textures: HashMap<String, Option<[f32; 3]>> = HashMap::new();
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
        let alpha = shape_alpha_mode(&props);
        let (emissive, _) = props.emissive_parts();
        let traits = ShapeTraits { unlit: props.unlit_part(), alpha, emissive, mult: shape_emissive_mult(&props, alpha), radius };
        if !glow_shape(&traits) {
            continue;
        }
        // The colour: the emissive hue times the hue of the texture that glows (a lit
        // shape's glow map, else its own texture), or the emissive hue alone.
        let raw = if traits.unlit { None } else { props.glow_part() }.or(props.textures.first().map(String::as_str));
        let tex = raw.filter(|r| !r.is_empty()).and_then(|r| {
            let p = fnv_vfs::texture_path(r);
            *textures.entry(p.clone()).or_insert_with(|| data.read(&p).and_then(|b| texture_hue(&b)))
        });
        let e = if emissive.iter().any(|&c| c > 0.0) { Vec3::from_array(hue(Vec3::from_array(emissive))) } else { Vec3::ONE };
        let colour = hue(e * Vec3::from_array(tex.unwrap_or([1.0; 3])));
        parts.push(GlowPart { centre: t.transform_point3(Vec3::from_array(centre)), radius, colour });
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

    #[test]
    fn a_lamp_post_glows_at_its_bulb() {
        // The post is no glow part; the bulb, 300 up, is.
        let bulb = GlowPart { centre: Vec3::new(0.0, 0.0, 300.0), radius: 10.0, colour: [1.0, 0.8, 0.5] };
        let g = combine(&[bulb]).unwrap();
        assert_eq!((g.centre, g.radius, g.colour, g.shapes), ([0.0, 0.0, 300.0], 10.0, [1.0, 0.8, 0.5], 1));
        // Two cards: weighted by area, reaching over both.
        let a = GlowPart { centre: Vec3::ZERO, radius: 30.0, colour: [1.0, 1.0, 1.0] };
        let b = GlowPart { centre: Vec3::new(100.0, 0.0, 0.0), radius: 10.0, colour: [1.0, 0.0, 0.0] };
        let g = combine(&[a, b]).unwrap();
        assert!((g.centre[0] - 10.0).abs() < 1e-4, "{:?}", g.centre);
        assert!((g.radius - 100.0).abs() < 1e-4);
        assert_eq!(g.colour[0], 1.0);
        assert!(g.colour[1] < 1.0 && g.colour[1] > 0.8);
        assert!(combine(&[]).is_none());
    }

    #[test]
    fn texture_hue_of_compressed_blocks() {
        // Two DXT1 blocks of 4 x 4: orange (both ends 0xFC00 + green 0x20 → r 1, g ~0.5) and
        // black (no weight).
        let orange = (31u16 << 11) | (32 << 5);
        let mut payload = Vec::new();
        payload.extend(orange.to_le_bytes());
        payload.extend(orange.to_le_bytes());
        payload.extend([0u8; 4]);
        payload.extend([0u8; 8]);
        let bytes = crate::texture::bc5_tests::dds(b"DXT1", 8, 4, 1, &payload);
        let h = texture_hue(&bytes).expect("a hue");
        assert_eq!(h[0], 1.0);
        assert!((h[1] - 32.0 / 63.0).abs() < 1e-4 && h[2] == 0.0, "{h:?}");
        let black = crate::texture::bc5_tests::dds(b"DXT1", 4, 4, 1, &[0u8; 8]);
        assert!(texture_hue(&black).is_none());
        assert!(texture_hue(b"not a dds").is_none());
    }
}
