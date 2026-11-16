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

/// Squares further than this from a reference's own square are left out: a box this
/// long (65 squares, 266,240 units) is a broken record, not a building [design, #2795].
const MAX_REACH: i32 = 32;

impl ReachIndex {
    /// From (reference, the square whose cell lists it, the squares its bounds cover):
    /// the reference is listed under every covered square but its own. A reference whose
    /// bounds stay in its own square gives no entry.
    pub fn from_entries(entries: impl IntoIterator<Item = (u32, (i32, i32), Squares)>) -> ReachIndex {
        let mut ix = ReachIndex::default();
        for (r, own, ((x0, y0), (x1, y1))) in entries {
            let (x0, y0) = (x0.max(own.0 - MAX_REACH), y0.max(own.1 - MAX_REACH));
            let (x1, y1) = (x1.min(own.0 + MAX_REACH), y1.min(own.1 + MAX_REACH));
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
        let placed_refs = |part: &[((i32, i32), &[u32])]| -> Vec<(u32, (i32, i32), u32, [f32; 3], [f32; 3], f32)> {
            let mut out = Vec::new();
            for &(own, refs) in part {
                for &r in refs {
                    if !ix.refs.get(&r).is_some_and(|e| &e.sig == b"REFR" && !e.persistent) {
                        continue;
                    }
                    let Some(pl) = placement(lo, schema, r).filter(|p| !p.deleted()) else { continue };
                    let Some(base) = pl.base else { continue };
                    out.push((r, own, base, pl.position, pl.rotation, pl.scale));
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
        let mut bases: Vec<u32> = placed.iter().map(|p| p.2).collect();
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
        ReachIndex::from_entries(placed.into_iter().filter_map(|(r, own, base, pos, rot, scale)| {
            let b = bounds.get(&base)?;
            Some((r, own, squares_reached(pos, rot, scale, *b)))
        }))
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
        let ix = ReachIndex::from_entries([(7, (0, 0), s)]);
        assert_eq!(ix.refs, 0);
        assert_eq!(ix.entries(), 0);
    }

    #[test]
    fn the_players_square_lists_a_reference_placed_two_squares_away_that_reaches_it() {
        // The player stands in (5, -3). The plant is placed in (5, -1) and reaches down to
        // (5, -3); a rock placed in (5, -1) reaches only (5, -2); a box in (7, -3) stays home.
        let ix = ReachIndex::from_entries([
            (0x0B0A0E, (5, -1), ((4, -3), (5, -1))),
            (0x000100, (5, -1), ((5, -2), (5, -1))),
            (0x000200, (7, -3), ((7, -3), (7, -3))),
        ]);
        assert_eq!(ix.reaching((5, -3)), &[0x0B0A0E]);
        assert_eq!(ix.reaching((4, -3)), &[0x0B0A0E]);
        assert_eq!(ix.reaching((5, -2)), &[0x000100, 0x0B0A0E]);
        assert_eq!(ix.reaching((5, -1)), &[] as &[u32], "never listed under its own square");
        assert_eq!(ix.reaching((7, -3)), &[] as &[u32]);
        assert_eq!(ix.refs, 2);
    }

    #[test]
    fn a_broken_box_is_cut_at_the_reach_limit() {
        let ix = ReachIndex::from_entries([(1, (0, 0), ((-1000, 0), (1000, 0)))]);
        assert_eq!(ix.entries(), 2 * MAX_REACH as usize);
        assert!(ix.reaching((MAX_REACH + 1, 0)).is_empty());
    }

    #[test]
    fn an_empty_world_index_builds_an_empty_reach_index() {
        let lo = LoadOrder::new(&[]).expect("empty load order");
        let ix = ReachIndex::build(&lo, &Schema::default(), &WorldIndex::default(), 0x3C);
        assert_eq!(ix, ReachIndex::default());
    }
}
