//! Measuring aids for the open shape question of docs/spec-notes/speedtree.md §7 (#3126:
//! EuonymusBush01 generated too wide): the generated tree's extents under several
//! measuring rules, and the tree generated under other readings of the level
//! parameters. Not used by the renderer; `fnvtool spt shape` prints them.

use crate::{generate, Geometry, Tree};

/// How leaf cards count towards the extents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cards {
    /// At their full width and up to their top edge, as `Geometry::width` and
    /// `Geometry::top` count them (what the renderer scales by).
    Full,
    /// Only the cards' attachment points.
    Pivots,
    /// Not at all: the wood alone.
    None,
}

/// Extents of a generated tree, Z up, trunk base at the origin.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Extents {
    /// Span along x and along y (a side view along y sees the x span, and so on).
    pub x: f32,
    pub y: f32,
    /// Highest point.
    pub top: f32,
}

impl Extents {
    /// The larger of the two spans: `Geometry::width`'s rule.
    pub fn width(&self) -> f32 {
        self.x.max(self.y)
    }
    /// Width : height, the figure the note compares with the billboard's opaque part.
    pub fn aspect(&self) -> f32 {
        if self.top > 0.0 { self.width() / self.top } else { 0.0 }
    }
}

/// The extents of `g` with leaf cards counted as `cards` says.
pub fn extents(g: &Geometry, cards: Cards) -> Extents {
    let mut lo = [f32::MAX; 2];
    let mut hi = [f32::MIN; 2];
    let mut top = 0.0f32;
    let mut grow = |p: [f32; 3], r: f32, up: f32| {
        for k in 0..2 {
            lo[k] = lo[k].min(p[k] - r);
            hi[k] = hi[k].max(p[k] + r);
        }
        top = top.max(p[2] + up);
    };
    for &p in &g.positions {
        grow(p, 0.0, 0.0);
    }
    for l in &g.leaves {
        match cards {
            Cards::Full => grow(l.position, l.size[0] * l.pivot[0].max(1.0 - l.pivot[0]), l.size[1] * (1.0 - l.pivot[1])),
            Cards::Pivots => grow(l.position, 0.0, 0.0),
            Cards::None => {}
        }
    }
    if lo[0] > hi[0] {
        return Extents { x: 0.0, y: 0.0, top };
    }
    Extents { x: hi[0] - lo[0], y: hi[1] - lo[1], top }
}

/// A reading of the level parameters other than the generator's own (speedtree.md §4).
/// Each is expressed as an edit of the parsed parameters, so the generator itself is
/// unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reading {
    /// The generator's readings as they are.
    AsIs,
    /// Level i's 6007 angle measured from the parent's axis instead of from the plane
    /// across it (only for that level).
    AngleFromAxis(usize),
    /// No random spread on any level's 6004 length.
    NoLengthSpread,
    /// No random bend (6000 variance) on any level.
    NoBend,
    /// 6012 children per unit of the part of the branch between 6010 and 6011, not of
    /// the whole branch (§7).
    FrequencyOverSpan,
}

impl Reading {
    pub fn name(&self) -> String {
        match self {
            Reading::AsIs => "as_is".into(),
            Reading::AngleFromAxis(i) => format!("angle_from_axis_level_{i}"),
            Reading::NoLengthSpread => "no_length_spread".into(),
            Reading::NoBend => "no_bend".into(),
            Reading::FrequencyOverSpan => "frequency_over_span".into(),
        }
    }
}

/// The tree's parameters under `reading`.
pub fn apply(tree: &Tree, reading: Reading) -> Tree {
    let mut t = tree.clone();
    match reading {
        Reading::AsIs => {}
        Reading::AngleFromAxis(i) => {
            if let Some(lv) = t.levels.get_mut(i) {
                // The generator turns a child 90° − value away from the parent's axis;
                // with the curve mirrored about 90° it turns the value itself (the
                // spread is symmetric, so it carries over).
                let c = &mut lv.curves[7];
                (c.min, c.max) = (90.0 - c.min, 90.0 - c.max);
            }
        }
        Reading::NoLengthSpread => t.levels.iter_mut().for_each(|lv| lv.curves[4].variance = 0.0),
        Reading::NoBend => t.levels.iter_mut().for_each(|lv| lv.curves[0].variance = 0.0),
        Reading::FrequencyOverSpan => t.levels.iter_mut().for_each(|lv| lv.frequency *= (lv.last - lv.first).abs()),
    }
    t
}

/// Every reading worth trying on `tree`: as is, the angle from the axis on each wood
/// level that grows from another (not the trunk, not the leaf level), and the rest.
pub fn readings(tree: &Tree) -> Vec<Reading> {
    let mut out = vec![Reading::AsIs];
    out.extend((1..tree.levels.len().saturating_sub(1)).map(Reading::AngleFromAxis));
    out.extend([Reading::NoLengthSpread, Reading::NoBend, Reading::FrequencyOverSpan]);
    out
}

/// One generated variant and its extents.
#[derive(Debug, Clone, PartialEq)]
pub struct Shape {
    pub reading: Reading,
    pub leaves: usize,
    pub full: Extents,
    pub pivots: Extents,
    pub wood: Extents,
}

/// The tree generated under every reading of [`readings`].
pub fn shapes(tree: &Tree) -> Vec<Shape> {
    readings(tree)
        .into_iter()
        .map(|reading| {
            let g = generate(&apply(tree, reading));
            Shape {
                reading,
                leaves: g.leaves.len(),
                full: extents(&g, Cards::Full),
                pivots: extents(&g, Cards::Pivots),
                wood: extents(&g, Cards::None),
            }
        })
        .collect()
}
