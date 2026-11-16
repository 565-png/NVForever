//! References whose bounds reach into exterior grid squares other than the one whose cell
//! lists them (#2795).
//!
//! Physics loads the squares around the player and, in each, the references its cell
//! record lists. A reference placed in one square can stretch over several:
//! HeliosPowerPlant (REFR 0x000B0A0E, placed in square 5,-1) is the front plaza of HELIOS
//! One down to y -8801, inside square 5,-3 [observed: `fnvtool phys cell` rays, #2795], so
//! a player in 5,-3 stood on the terrain under the plaza. This index lists, for each
//! square, the references placed elsewhere whose bounds cover part of it, so a square
//! loads them too [design, #2795].
//!
//! A reference's bounds are its base record's OBND box (min, max corners in the model's
//! space [documented: xEdit wbDefinitionsCommon.pas wbObjectBounds,
//! schemas/fnv/common.json]) scaled, turned by the reference's full rotation
//! ([`crate::transform::ref_rotation`]) and moved to its position; the squares are those
//! the box's horizontal extent covers. That the OBND box covers the model's collision is
//! [hypothesis]: HeliosPowerPlant's does (OBND y -4750..1142 around its origin, its
//! collision y -2913..-8801 placed [observed: #2795]); a model whose collision reaches
//! past its OBND is still missed past the bounds.
//!
//! Only references whose box reaches beyond the square that lists them get entries, and
//! only placed REFRs of the worldspace's exterior cells: persistent references load by
//! the square they stand in, as before, and actors have their own bodies.

use crate::fields::{num, winner, Fields};
use crate::placed::placement;
use crate::transform::ref_rotation;
use crate::{cell_of, WorldIndex};
use fnv_esm::{LoadOrder, Schema};
use std::collections::HashMap;

/// An OBND box: (min, max) corners in the model's space.
pub type Bounds = ([f32; 3], [f32; 3]);

/// The inclusive range of grid squares (min corner square, max corner square).
pub type Squares = ((i32, i32), (i32, i32));

/// A base record's OBND, when it has one.
pub fn base_bounds(lo: &LoadOrder, schema: &Schema, base: u32) -> Option<Bounds> {
    let (pi, r) = winner(lo, base)?;
    let f = Fields::new(r, pi, schema).ok()?;
    let b = f.get(b"OBND")?;
    let corner = |k: &str| -> Option<[f32; 3]> {
        let c = b.get(k)?;
        Some([num(c, "x")? as f32, num(c, "y")? as f32, num(c, "z")? as f32])
    };
    Some((corner("min")?, corner("max")?))
}

/// The grid squares the horizontal extent of `bounds` covers once placed at `position`,
/// turned by `rotation` (stored Euler angles, radians) and scaled by `scale`.
pub fn squares_reached(position: [f32; 3], rotation: [f32; 3], scale: f32, bounds: Bounds) -> Squares {
    let m = ref_rotation(rotation);
    let (lo, hi) = bounds;
    let (mut min, mut max) = ([f32::INFINITY; 2], [f32::NEG_INFINITY; 2]);
    for i in 0..8 {
        let c = [if i & 1 == 0 { lo[0] } else { hi[0] }, if i & 2 == 0 { lo[1] } else { hi[1] }, if i & 4 == 0 { lo[2] } else { hi[2] }];
        for (k, (mn, mx)) in min.iter_mut().zip(max.iter_mut()).enumerate() {
            let v = position[k] + scale * (m[k * 3] * c[0] + m[k * 3 + 1] * c[1] + m[k * 3 + 2] * c[2]);
            *mn = mn.min(v);
            *mx = mx.max(v);
        }
    }
    (cell_of(min[0], min[1]), cell_of(max[0], max[1]))
}

/// For each exterior grid square of one worldspace, the references placed in another
/// square whose bounds reach into it, by FormID.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ReachIndex {
    by_square: HashMap<(i32, i32), Vec<u32>>,
    /// References with entries (each reaches beyond its own square).
    pub refs: usize,
}

/// A reference reaching further than this many squares from its own is logged (a WARN
/// naming it and its reach): the largest buildings of FalloutNV.esm reach two or three
/// [observed: #2795's table], so such a box more likely comes from a broken record. It
/// is indexed all the same [design, #2795].
const LONG_REACH: i32 = 32;

/// One placed reference as [`ReachIndex::build`] reads it: FormID, the square whose cell
/// lists it, its base, position, rotation and scale.
struct Placed {
    form_id: u32,
    own: (i32, i32),
    base: u32,
    position: [f32; 3],
    rotation: [f32; 3],
    scale: f32,
}

impl ReachIndex {
    /// From (reference, the square whose cell lists it, the squares its bounds cover):
    /// the reference is listed under every covered square but its own that lies within
    /// `extent` (the worldspace's grid of cells: no square outside it has a cell). A
    /// reference whose bounds stay in its own square gives no entry. A reach cut by
    /// `extent`, or longer than [`LONG_REACH`] squares, is logged as a warning.
    pub fn from_entries(entries: impl IntoIterator<Item = (u32, (i32, i32), Squares)>, extent: Squares) -> ReachIndex {
        let mut ix = ReachIndex::default();
        let ((ex0, ey0), (ex1, ey1)) = extent;
        for (r, own, reach) in entries {
            let ((x0, y0), (x1, y1)) = reach;
            let long = (own.0 - x0).max(x1 - own.0).max(own.1 - y0).max(y1 - own.1) > LONG_REACH;
            let cut = x0 < ex0 || y0 < ey0 || x1 > ex1 || y1 > ey1;
            let (x0, y0, x1, y1) = (x0.max(ex0), y0.max(ey0), x1.min(ex1), y1.min(ey1));
            if long || cut {
                tracing::warn!(
                    "reach: 0x{r:08X} (placed in square {},{}) has bounds over squares {},{} to {},{}{}{}",
                    own.0,
                    own.1,
                    reach.0 .0,
                    reach.0 .1,
                    reach.1 .0,
                    reach.1 .1,
                    if long { format!(", more than {LONG_REACH} squares from its own (a broken record?)") } else { String::new() },
                    if cut { format!("; indexed within the worldspace's grid {ex0},{ey0} to {ex1},{ey1} only") } else { String::new() },
                );
            }
            let mut any = false;
            for y in y0..=y1 {
                for x in x0..=x1 {
                    if (x, y) != own {
                        ix.by_square.entry((x, y)).or_default().push(r);
                        any = true;
                    }
                }
            }
            ix.refs += any as usize;
        }
        for v in ix.by_square.values_mut() {
            v.sort_unstable();
            v.dedup();
        }
        ix
    }

    /// The index of worldspace `world`: every placed REFR of its exterior cell records
    /// (not its persistent cell's), with its base's OBND. Reads each reference once, on
    /// as many threads as the machine has (up to 8).
    pub fn build(lo: &LoadOrder, schema: &Schema, ix: &WorldIndex, world: u32) -> ReachIndex {
        let Some(ws) = ix.worldspaces.get(&world) else { return ReachIndex::default() };
        let cells: Vec<((i32, i32), &[u32])> =
            ws.grid.iter().filter_map(|(&g, id)| ix.cells.get(id).map(|c| (g, c.refs.as_slice()))).collect();
        // The worldspace's grid of cells: no square outside it has a cell.
        let Some(extent) = cells.iter().map(|c| c.0).fold(None, |e: Option<Squares>, (x, y)| {
            Some(e.map_or(((x, y), (x, y)), |((x0, y0), (x1, y1))| ((x0.min(x), y0.min(y)), (x1.max(x), y1.max(y)))))
        }) else {
            return ReachIndex::default();
        };
        let placed_refs = |part: &[((i32, i32), &[u32])]| -> Vec<Placed> {
            let mut out = Vec::new();
            for &(own, refs) in part {
                for &r in refs {
                    if !ix.refs.get(&r).is_some_and(|e| &e.sig == b"REFR" && !e.persistent) {
                        continue;
                    }
                    let Some(pl) = placement(lo, schema, r).filter(|p| !p.deleted()) else { continue };
                    let Some(base) = pl.base else { continue };
                    out.push(Placed { form_id: r, own, base, position: pl.position, rotation: pl.rotation, scale: pl.scale });
                }
            }
            out
        };
        let threads = std::thread::available_parallelism().map_or(1, |n| n.get()).clamp(1, 8);
        let chunk = cells.len().div_ceil(threads).max(1);
        let placed: Vec<_> = std::thread::scope(|s| {
            let handles: Vec<_> = cells.chunks(chunk).map(|part| s.spawn(move || placed_refs(part))).collect();
            handles.into_iter().flat_map(|h| h.join().expect("reach index thread")).collect()
        });
        let mut bases: Vec<u32> = placed.iter().map(|p| p.base).collect();
        bases.sort_unstable();
        bases.dedup();
        let bchunk = bases.len().div_ceil(threads).max(1);
        let bounds: HashMap<u32, Bounds> = std::thread::scope(|s| {
            let handles: Vec<_> = bases
                .chunks(bchunk)
                .map(|part| s.spawn(move || part.iter().filter_map(|&b| base_bounds(lo, schema, b).map(|v| (b, v))).collect::<Vec<_>>()))
                .collect();
            handles.into_iter().flat_map(|h| h.join().expect("reach index thread")).collect()
        });
        let entries = placed.into_iter().filter_map(|p| {
            let b = bounds.get(&p.base)?;
            Some((p.form_id, p.own, squares_reached(p.position, p.rotation, p.scale, *b)))
        });
        ReachIndex::from_entries(entries, extent)
    }

    /// The references placed elsewhere whose bounds reach into `square`, by FormID.
    pub fn reaching(&self, square: (i32, i32)) -> &[u32] {
        self.by_square.get(&square).map_or(&[], Vec::as_slice)
    }

    /// Entries over every square (a reference counts once per square it reaches into).
    pub fn entries(&self) -> usize {
        self.by_square.values().map(Vec::len).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CELL_SIZE;

    /// An extent wider than any test's squares.
    const WIDE: Squares = ((-100, -100), (100, 100));

    /// The centre of grid square `g`.
    fn square_centre(g: (i32, i32)) -> [f32; 2] {
        [(g.0 as f32 + 0.5) * CELL_SIZE, (g.1 as f32 + 0.5) * CELL_SIZE]
    }

    #[test]
    fn a_box_two_squares_long_covers_three_squares() {
        // Placed in the middle of square (5, -1), reaching 9,000 units south (OBND y
        // -9000..500): it covers 5,-3 to 5,-1.
        let c = square_centre((5, -1));
        let b = ([-200.0, -9000.0, 0.0], [200.0, 500.0, 100.0]);
        assert_eq!(squares_reached([c[0], c[1], 0.0], [0.0; 3], 1.0, b), ((5, -3), (5, -1)));
        // Turned a quarter about z it lies east-west instead.
        let ((x0, y0), (x1, y1)) = squares_reached([c[0], c[1], 0.0], [0.0, 0.0, std::f32::consts::FRAC_PI_2], 1.0, b);
        assert_eq!((y0, y1), (-1, -1));
        assert_eq!(x1 - x0, 2, "three squares east-west");
        // Half the size: one square south only.
        assert_eq!(squares_reached([c[0], c[1], 0.0], [0.0; 3], 0.5, b), ((5, -2), (5, -1)));
    }

    #[test]
    fn a_box_in_its_own_square_gives_no_entry() {
        let c = square_centre((0, 0));
        let s = squares_reached([c[0], c[1], 0.0], [0.0, 0.0, 0.7], 1.0, ([-100.0; 3], [100.0; 3]));
        assert_eq!(s, ((0, 0), (0, 0)));
        let ix = ReachIndex::from_entries([(7, (0, 0), s)], WIDE);
        assert_eq!(ix.refs, 0);
        assert_eq!(ix.entries(), 0);
    }

    #[test]
    fn the_players_square_lists_a_reference_placed_two_squares_away_that_reaches_it() {
        // The player stands in (5, -3). The plant is placed in (5, -1) and reaches down to
        // (5, -3); a rock placed in (5, -1) reaches only (5, -2); a box in (7, -3) stays home.
        let ix = ReachIndex::from_entries(
            [(0x0B0A0E, (5, -1), ((4, -3), (5, -1))), (0x000100, (5, -1), ((5, -2), (5, -1))), (0x000200, (7, -3), ((7, -3), (7, -3)))],
            WIDE,
        );
        assert_eq!(ix.reaching((5, -3)), &[0x0B0A0E]);
        assert_eq!(ix.reaching((4, -3)), &[0x0B0A0E]);
        assert_eq!(ix.reaching((5, -2)), &[0x000100, 0x0B0A0E]);
        assert_eq!(ix.reaching((5, -1)), &[] as &[u32], "never listed under its own square");
        assert_eq!(ix.reaching((7, -3)), &[] as &[u32]);
        assert_eq!(ix.refs, 2);
    }

    #[test]
    fn a_very_long_box_is_indexed_over_the_worldspaces_grid() {
        // A reference reaching 1,000 squares either way in a worldspace whose cells span
        // -40..40 by -30..30: listed under every square of the grid but its own, none
        // outside it.
        let grid = ((-40, -30), (40, 30));
        let ix = ReachIndex::from_entries([(1, (0, 0), ((-1000, -1000), (1000, 1000)))], grid);
        assert_eq!(ix.entries(), 81 * 61 - 1);
        assert_eq!(ix.reaching((40, -30)), &[1]);
        assert_eq!(ix.reaching((-40, 30)), &[1]);
        assert!(ix.reaching((41, 0)).is_empty() && ix.reaching((0, 0)).is_empty());
        // A reach past the grid's edge keeps its squares inside it.
        let ix = ReachIndex::from_entries([(2, (40, 0), ((39, 0), (42, 0)))], grid);
        assert_eq!(ix.reaching((39, 0)), &[2]);
        assert_eq!(ix.entries(), 1);
    }

    #[test]
    fn an_empty_world_index_builds_an_empty_reach_index() {
        let lo = LoadOrder::new(&[]).expect("empty load order");
        let ix = ReachIndex::build(&lo, &Schema::default(), &WorldIndex::default(), 0x3C);
        assert_eq!(ix, ReachIndex::default());
    }
}
