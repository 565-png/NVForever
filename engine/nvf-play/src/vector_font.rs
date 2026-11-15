//! The menus' sharp letters (the owner's menu overhaul, 2026-11-15): a vector font drawn by
//! Bevy's text at the screen's own resolution, in place of the game's raster fonts, whose
//! texels are stretched over two pixels at 4K [observed: the owner's screenshots; the
//! interface is 960 units high and a font texel is one unit].
//!
//! **The font** is Roboto Condensed (variable, weight axis), SIL Open Font License 1.1,
//! Copyright 2011 The Roboto Project Authors, from github.com/google/fonts `ofl/robotocondensed`
//! [documented: its `OFL.txt` beside the file, `engine/nvf-play/assets/fonts/`]; the owner's
//! pick of 2026-11-15 among OFL fonts with Latin and Cyrillic (Fallout 4's own interface
//! font). The game's Monofonto cannot be shipped (a commercial font; its files are game data).
//! The OFL lets the font be bundled with any program; NVForever stays GPL-3.0-or-later and the
//! font stays OFL (its licence text ships with it, tools/release/third_party_licenses.py).
//!
//! **Measured as the raster fonts are**: the pages lay their text out from an
//! `fnv_font::Font` table (`width`, `lines`), never from the texture. [`metric_table`] fills
//! that table from the font file's own advances for this run's code page, so every width,
//! wrap and cut on the page stays as it is. For the drawn text to be as wide as measured,
//! Bevy is asked for no kerning and no ligatures ([`features`]) and the weight is the file's
//! default (400), whose advances are the `hmtx` table's. [`read`] reads only what that needs:
//! the `head`, `hhea`, `hmtx`, `cmap` and `OS/2` tables (no new dependency).

use bevy::prelude::*;
use fnv_font::{Font, Glyph};

/// The font file, built into the program.
pub const FONT_BYTES: &[u8] = include_bytes!("../assets/fonts/RobotoCondensed[wght].ttf");

/// What the page needs of a TrueType font: its em, the line metrics Bevy's text places a
/// line by, the cap and x heights, and each character's advance.
#[derive(Debug, Clone, PartialEq)]
pub struct Metrics {
    pub units_per_em: f32,
    /// Above and below the baseline (both positive), as Bevy's text (skrifa) takes them: the
    /// `OS/2` typographic ones where its USE_TYPO_METRICS flag is set, else `hhea`'s.
    pub ascent: f32,
    pub descent: f32,
    pub cap_height: f32,
    pub x_height: f32,
    advances: Vec<u16>,
    /// Character ranges and how each maps to glyphs (cmap format 4 or 12).
    map: Vec<Range>,
}

#[derive(Debug, Clone, PartialEq)]
struct Range {
    start: u32,
    end: u32,
    /// The glyph of `start` (format 12), or the format 4 rule.
    to: Map,
}

#[derive(Debug, Clone, PartialEq)]
enum Map {
    /// Glyph = code + delta (format 4 without a glyph array; format 12 as start glyph).
    Delta(i64),
    /// Glyph read from a format-4 glyph array, by offset from `start`.
    Array(Vec<u16>, i32),
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes([*b.get(at)?, *b.get(at + 1)?]))
}

fn i16_at(b: &[u8], at: usize) -> Option<i16> {
    u16_at(b, at).map(|v| v as i16)
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes([*b.get(at)?, *b.get(at + 1)?, *b.get(at + 2)?, *b.get(at + 3)?]))
}

/// The table `tag`'s bytes.
fn table<'a>(b: &'a [u8], tag: &[u8; 4]) -> Option<&'a [u8]> {
    let n = u16_at(b, 4)? as usize;
    (0..n).find_map(|i| {
        let at = 12 + 16 * i;
        (b.get(at..at + 4)? == tag).then_some(())?;
        let (off, len) = (u32_at(b, at + 8)? as usize, u32_at(b, at + 12)? as usize);
        b.get(off..off + len)
    })
}

/// Read a TrueType font's metrics (module notes); the reason when it is not one.
pub fn read(b: &[u8]) -> Result<Metrics, String> {
    let need = |tag: &[u8; 4]| table(b, tag).ok_or_else(|| format!("no {} table", String::from_utf8_lossy(tag)));
    let head = need(b"head")?;
    let hhea = need(b"hhea")?;
    let hmtx = need(b"hmtx")?;
    let cmap = need(b"cmap")?;
    let os2 = table(b, b"OS/2");
    let units_per_em = f32::from(u16_at(head, 18).ok_or("short head")?);
    let metrics = hhea.len() >= 36;
    if !metrics || units_per_em <= 0.0 {
        return Err("short hhea or no em".into());
    }
    let n_metrics = u16_at(hhea, 34).unwrap_or(0) as usize;
    let advances: Vec<u16> = (0..n_metrics).filter_map(|i| u16_at(hmtx, 4 * i)).collect();
    let (mut ascent, mut descent) = (f32::from(i16_at(hhea, 4).unwrap_or(0)), -f32::from(i16_at(hhea, 6).unwrap_or(0)));
    let (mut cap_height, mut x_height) = (0.7 * units_per_em, 0.5 * units_per_em);
    if let Some(os2) = os2 {
        let selection = u16_at(os2, 62).unwrap_or(0);
        if selection & (1 << 7) != 0 {
            ascent = f32::from(i16_at(os2, 68).unwrap_or(0));
            descent = -f32::from(i16_at(os2, 70).unwrap_or(0));
        }
        if u16_at(os2, 0).unwrap_or(0) >= 2 {
            x_height = f32::from(i16_at(os2, 86).unwrap_or(0)).max(1.0);
            cap_height = f32::from(i16_at(os2, 88).unwrap_or(0)).max(1.0);
        }
    }
    Ok(Metrics { units_per_em, ascent, descent, cap_height, x_height, advances, map: read_cmap(cmap)? })
}

/// The Unicode subtable's ranges: format 12 (3,10 or 0,4…) before format 4 (3,1 or 0,3).
fn read_cmap(c: &[u8]) -> Result<Vec<Range>, String> {
    let n = u16_at(c, 2).ok_or("short cmap")? as usize;
    let subtables: Vec<(u16, u16, usize)> = (0..n).filter_map(|i| Some((u16_at(c, 4 + 8 * i)?, u16_at(c, 6 + 8 * i)?, u32_at(c, 8 + 8 * i)? as usize))).collect();
    let unicode = |p: u16, e: u16| p == 0 || (p == 3 && (e == 1 || e == 10));
    let format = |off: usize| u16_at(c, off).unwrap_or(0);
    if let Some(&(_, _, off)) = subtables.iter().find(|(p, e, off)| unicode(*p, *e) && format(*off) == 12) {
        let groups = u32_at(c, off + 12).ok_or("short cmap 12")? as usize;
        return Ok((0..groups)
            .filter_map(|g| {
                let at = off + 16 + 12 * g;
                let (start, end, glyph) = (u32_at(c, at)?, u32_at(c, at + 4)?, u32_at(c, at + 8)?);
                Some(Range { start, end, to: Map::Delta(i64::from(glyph) - i64::from(start)) })
            })
            .collect());
    }
    let &(_, _, off) = subtables.iter().find(|(p, e, off)| unicode(*p, *e) && format(*off) == 4).ok_or("no Unicode cmap (format 4 or 12)")?;
    let segs = u16_at(c, off + 6).ok_or("short cmap 4")? as usize / 2;
    let ends = off + 14;
    let starts = ends + 2 * segs + 2;
    let deltas = starts + 2 * segs;
    let offsets = deltas + 2 * segs;
    let mut out = Vec::new();
    for s in 0..segs {
        let (Some(end), Some(start), Some(delta), Some(range_off)) = (u16_at(c, ends + 2 * s), u16_at(c, starts + 2 * s), i16_at(c, deltas + 2 * s), u16_at(c, offsets + 2 * s)) else { continue };
        if start == 0xFFFF {
            continue;
        }
        let to = if range_off == 0 {
            Map::Delta(i64::from(delta))
        } else {
            // The glyph array entry of `start` lies `range_off` bytes after this offset's slot.
            let first = offsets + 2 * s + range_off as usize;
            let glyphs = (0..=(end - start) as usize).map(|k| u16_at(c, first + 2 * k).unwrap_or(0)).collect();
            Map::Array(glyphs, i32::from(delta))
        };
        out.push(Range { start: u32::from(start), end: u32::from(end), to });
    }
    Ok(out)
}

impl Metrics {
    /// The glyph of character `c` (0, the missing glyph, when the font has none).
    pub fn glyph(&self, c: char) -> u16 {
        let u = c as u32;
        self.map
            .iter()
            .find(|r| (r.start..=r.end).contains(&u))
            .map_or(0, |r| match &r.to {
                Map::Delta(d) => ((i64::from(u) + d) & 0xFFFF) as u16,
                Map::Array(glyphs, d) => match glyphs.get((u - r.start) as usize) {
                    Some(0) | None => 0,
                    Some(&g) => ((i32::from(g) + d) & 0xFFFF) as u16,
                },
            })
    }

    /// Character `c`'s advance as a part of the em.
    pub fn advance(&self, c: char) -> f32 {
        let g = self.glyph(c) as usize;
        let a = self.advances.get(g).or(self.advances.last()).copied().unwrap_or(0);
        f32::from(a) / self.units_per_em
    }

    /// Whether the font has a glyph of its own for `c`.
    pub fn has(&self, c: char) -> bool {
        self.glyph(c) != 0
    }
}

/// The font's metrics, read once.
pub fn metrics() -> Option<&'static Metrics> {
    static READ: std::sync::OnceLock<Option<Metrics>> = std::sync::OnceLock::new();
    READ.get_or_init(|| match read(FONT_BYTES) {
        Ok(m) => Some(m),
        Err(e) => {
            warn!("interface: the menus' font does not read: {e}");
            None
        }
    })
    .as_ref()
}

/// A metric table of the font at `em` interface units for this run's code page, its lines
/// `line` apart (the raster font's), as `fnv_font::Font` measures (module notes): each
/// character's advance its width, a space's its room after; capitals and figures as tall as
/// the cap height, small letters as the x-height.
pub fn metric_table(m: &Metrics, em: f32, line: f32) -> Font {
    let page = fnv_util::text::code_page();
    let glyphs = (0..=255u8)
        .map(|code| {
            let c = page.char_of(code);
            let adv = m.advance(c) * em;
            let rise = if c.is_lowercase() { m.x_height } else { m.cap_height } / m.units_per_em * em;
            if c.is_whitespace() || code < 0x20 {
                Glyph { after: adv, ..default() }
            } else {
                Glyph { width: adv, height: rise, ascent: rise, ..default() }
            }
        })
        .collect();
    Font { size: line, texture: String::new(), glyphs }
}

/// The OpenType features turned off so the drawn text is as wide as [`metric_table`]
/// measures: kerning and the ligatures (Roboto has `fi`, `fl`).
pub fn features() -> bevy::text::FontFeatures {
    use bevy::text::{FontFeatureTag, FontFeatures};
    FontFeatures::builder()
        .set(FontFeatureTag::new(b"kern"), 0)
        .set(FontFeatureTag::STANDARD_LIGATURES, 0)
        .set(FontFeatureTag::CONTEXTUAL_LIGATURES, 0)
        .set(FontFeatureTag::CONTEXTUAL_ALTERNATES, 0)
        .build()
}

/// Where a line's baseline lies below the top of the text Bevy lays out at `em` with a line
/// height of one em: half the room the line leaves over the ascent and descent, then the
/// ascent [documented: parley 0.11 `line_break.rs`, baseline = ascent + half the leading].
pub fn baseline_below_top(m: &Metrics, em: f32) -> f32 {
    let (asc, desc) = (m.ascent / m.units_per_em * em, m.descent / m.units_per_em * em);
    (em - asc - desc) / 2.0 + asc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shipped_font_reads_and_has_latin_and_cyrillic() {
        let m = read(FONT_BYTES).expect("Roboto Condensed reads");
        assert_eq!(m.units_per_em, 2048.0);
        assert!(m.ascent > 1500.0 && m.descent > 300.0, "{m:?}");
        assert!((m.cap_height / m.units_per_em - 0.71).abs() < 0.02, "cap {}", m.cap_height);
        for c in ['A', 'a', 'Q', 'z', '0', '%', 'é', 'Ж', 'я', 'ß'] {
            assert!(m.has(c), "{c}");
            assert!(m.advance(c) > 0.1 && m.advance(c) < 1.0, "{c}: {}", m.advance(c));
        }
        // Figures are tabular; capitals wider than small letters.
        assert_eq!(m.advance('0'), m.advance('8'));
        assert!(m.advance('M') > m.advance('i'));
        assert!(!m.has('\u{E000}'), "a private-use code: the missing glyph");
    }

    #[test]
    fn the_table_measures_as_the_font_advances() {
        let m = read(FONT_BYTES).unwrap();
        let t = metric_table(&m, 32.0, 36.0);
        let text = "Ambient Occlusion 16x";
        let sum: f32 = text.chars().map(|c| m.advance(c) * 32.0).sum();
        assert!((t.width(text) - sum).abs() < 1e-3, "{} vs {sum}", t.width(text));
        assert!(!t.glyph(b' ').drawn() && t.glyph(b' ').advance() > 0.0, "a space has room, no picture");
        assert_eq!(t.size, 36.0);
        assert!(t.glyph(b'H').ascent > t.glyph(b'x').ascent);
        let base = baseline_below_top(&m, 32.0);
        assert!(base > 20.0 && base < 32.0, "{base}");
    }

    #[test]
    fn a_broken_file_says_why() {
        assert!(read(b"not a font").is_err());
        assert!(read(&[]).is_err());
    }
}
