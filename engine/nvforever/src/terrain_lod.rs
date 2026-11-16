//! 4 × 4 near (#2676): the loaded landscape drawn at 4 × 4 near the camera and 2 × 2 further
//! away, on the one 4 × 4 surface the collision uses (`nvf_play::smooth_terrain`, the
//! settings page's Smooth terrain row). [design, #2676; NVForever's own rule, not the
//! original's behaviour; the distances are to be tuned from pictures.]
//!
//! - **Levels**: each loaded quadrant is drawn with every vertex of the 4 × 4 surface
//!   (`QuadrantDraw::FULL`) or every other one (`QuadrantDraw::HALF`, exactly the 2 × 2
//!   surface: `fnv_world::land_surface::QuadrantDraw`).
//! - **Rule** (`nvf_play::smooth_terrain::DistanceRule`): a quadrant becomes 4 × 4 when the
//!   camera comes within `near` units of its square (seen from above; 0 over it) and 2 × 2
//!   again beyond `far` = `near` + 512, keeping its level in between, so one at the
//!   boundary does not flicker. A cell's quadrants are first drawn 2 × 2 (its meshes are
//!   built with it, on the read pool, `first_draw`); new cells load at the edge of the
//!   loaded grid, far from the camera.
//! - **No cracks**: a 4 × 4 quadrant's side next to a 2 × 2 quadrant, or next to a quadrant
//!   not loaded (it loads as 2 × 2), is drawn coarse: its in-between vertices on the 2 × 2
//!   side's straight segments. A level change therefore also redraws the 4 × 4 neighbours
//!   whose sides depend on it.
//! - **Redraws** run as one batch at a time: the quadrants whose wanted drawing differs from
//!   the drawn one are rebuilt on the read pool (`nvf_assets::read_pool`, the workers that
//!   read cells ahead), from what the cell was spawned with (`crate::terrain::TerrainSource`;
//!   no record is read again). When the whole batch is built, its meshes are spawned hidden
//!   and, [`SWAP_FRAMES`] frames later (time for their upload), shown while the old ones are
//!   despawned, all in one frame: neighbours that depend on each other change together, and
//!   a quadrant is never without ground. Levels are looked at again once a batch is in.
//! - **Collision** stays on the 4 × 4 surface for every loaded cell: actors near the player
//!   stand on what is drawn; at a distance the drawn 2 × 2 ground is within about a unit of
//!   it (world.md §3 "Smoothing").
//! - **Log**: `terrain levels: first redraw: …` at the first batch, then at most every
//!   [`LOG_EVERY`] seconds while redraws happen `terrain levels: <f> quadrants drawn 4 x 4,
//!   <c> 2 x 2; <b> redraws of <n> quadrants in the last <s> s (build <ms> ms on the read
//!   pool, longest batch <ms> ms from start to swap)`; the exit's terrain summary adds the
//!   totals ([`TerrainLod::summary`]).

use crate::terrain::{build_quadrant, spawn_quadrant, QuadMeshes, TerrainCtx, TerrainMaterials, TerrainSource};
use crate::world::{bevy_to_game, CellEntity, WorldData};
use bevy::prelude::*;
use bevy::tasks::{futures::check_ready, Task};
use fnv_world::land_surface::{QuadrantDraw, QUADRANT_EAST, QUADRANT_NORTH, QUADRANT_SOUTH, QUADRANT_WEST};
use nvf_assets::model::ModelCtx;
use nvf_play::smooth_terrain::DistanceRule;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

/// Frames a redrawn quadrant's new meshes stay hidden before they replace the old ones
/// [engineering choice: the GPU upload limit sends a frame's worth of assets at a time
/// (world.rs `UPLOAD_BYTES_PER_FRAME`, 24 MiB, more than a batch's meshes), so the new
/// meshes are on the GPU by then and the swap shows no hole].
pub const SWAP_FRAMES: u32 = 3;

/// Seconds between two `terrain levels:` lines while redraws happen [design].
pub const LOG_EVERY: f64 = 10.0;

/// A quadrant's side length (units): half a cell.
const QUADRANT_SIZE: f32 = fnv_world::CELL_SIZE / 2.0;

/// The drawing every quadrant of a cell starts with: 2 × 2 under 4 × 4 near (module
/// notes), in full otherwise.
pub fn first_draw() -> QuadrantDraw {
    if nvf_play::smooth_terrain::distance_rule().is_some() {
        QuadrantDraw::HALF
    } else {
        QuadrantDraw::FULL
    }
}

/// Horizontal distance (units) from game-space point `p` to quadrant `q` of the cell whose
/// south-west corner is `origin`: 0 over it.
pub fn quadrant_distance(origin: Vec2, q: u8, p: Vec2) -> f32 {
    let min = origin + Vec2::new((q % 2) as f32, (q / 2) as f32) * QUADRANT_SIZE;
    let max = min + Vec2::splat(QUADRANT_SIZE);
    (p - p.clamp(min, max)).length()
}

/// The quadrant across `side` of quadrant `q` (0 south-west … 3 north-east): the grid offset
/// of its cell (0, 0 for the same cell) and its quadrant there.
pub fn neighbour(q: u8, side: usize) -> ((i32, i32), u8) {
    let (col, row) = (q % 2, q / 2);
    match side {
        QUADRANT_EAST if col == 0 => ((0, 0), q + 1),
        QUADRANT_EAST => ((1, 0), q - 1),
        QUADRANT_WEST if col == 1 => ((0, 0), q - 1),
        QUADRANT_WEST => ((-1, 0), q + 1),
        QUADRANT_NORTH if row == 0 => ((0, 0), q + 2),
        QUADRANT_NORTH => ((0, 1), q - 2),
        _ if row == 1 => ((0, 0), q - 2),
        _ => ((0, -1), q + 2),
    }
}

/// How a quadrant is to be drawn: 4 × 4 (`fine`) with each side coarse where the quadrant
/// across it is not 4 × 4 (`neighbour_fine`; `None`: not loaded, drawn 2 × 2 when it loads),
/// or 2 × 2.
pub fn wanted_draw(fine: bool, neighbour_fine: [Option<bool>; 4]) -> QuadrantDraw {
    if !fine {
        return QuadrantDraw::HALF;
    }
    QuadrantDraw { step: 1, coarse: neighbour_fine.map(|n| n != Some(true)) }
}

/// One loaded cell under 4 × 4 near.
struct LodCell {
    grid: (i32, i32),
    source: Arc<TerrainSource>,
    /// Each quadrant's level by the rule (4 × 4: true).
    fine: [bool; 4],
    /// How each quadrant is drawn now, and its mesh entities.
    drawn: [QuadrantDraw; 4],
    entities: [Vec<Entity>; 4],
}

/// A redraw under way: being built, or built and waiting to be swapped in.
enum Batch {
    Building { started: Instant, jobs: Vec<(Entity, u8, QuadrantDraw, Task<QuadMeshes>)>, built: Vec<(Entity, u8, QuadrantDraw, QuadMeshes)> },
    Swapping { started: Instant, built_ms: f64, frames: u32, new: Vec<(Entity, u8, QuadrantDraw, Vec<Entity>)> },
}

/// Counts for the log.
#[derive(Default, Clone, Copy)]
struct Counts {
    batches: u64,
    quadrants: u64,
    build_ms: f64,
    longest_ms: f64,
}

/// 4 × 4 near's state (`crate::terrain::TerrainCache::lod`).
pub struct TerrainLod {
    rule: DistanceRule,
    cells: HashMap<Entity, LodCell>,
    by_grid: HashMap<(i32, i32), Entity>,
    batch: Option<Batch>,
    total: Counts,
    since_log: Counts,
    last_log: Option<Instant>,
}

impl TerrainLod {
    pub fn new(rule: DistanceRule) -> TerrainLod {
        TerrainLod { rule, cells: HashMap::new(), by_grid: HashMap::new(), batch: None, total: Counts::default(), since_log: Counts::default(), last_log: None }
    }

    /// A cell spawned with every quadrant drawn as `draw`, from `source`; `entities` its
    /// quadrants' mesh entities.
    pub fn add_cell(&mut self, cell: Entity, source: Arc<TerrainSource>, draw: QuadrantDraw, entities: [Vec<Entity>; 4]) {
        let grid = ((source.origin.x / fnv_world::CELL_SIZE).round() as i32, (source.origin.y / fnv_world::CELL_SIZE).round() as i32);
        let fine = draw.step == 1;
        if let Some(old) = self.by_grid.insert(grid, cell).filter(|old| *old != cell) {
            self.cells.remove(&old);
        }
        self.cells.insert(cell, LodCell { grid, source, fine: [fine; 4], drawn: [draw; 4], entities });
    }

    /// How a cell's quadrants are drawn now; `None`: not one of its cells.
    pub fn drawn(&self, cell: Entity) -> Option<[QuadrantDraw; 4]> {
        self.cells.get(&cell).map(|c| c.drawn)
    }

    /// Quadrants drawn 4 × 4 and 2 × 2 now.
    pub fn levels(&self) -> (usize, usize) {
        let fine = self.cells.values().flat_map(|c| c.drawn).filter(|d| d.step == 1).count();
        (fine, self.cells.len() * 4 - fine)
    }

    /// The exit summary's part: `4 x 4 near: <b> redraws, <n> quadrants rebuilt (build <ms>
    /// ms on the read pool), longest batch <ms> ms; now <f> 4 x 4, <c> 2 x 2`.
    pub fn summary(&self) -> String {
        let (fine, coarse) = self.levels();
        let t = self.total;
        format!("4 x 4 near: {} redraws, {} quadrants rebuilt (build {:.0} ms on the read pool), longest batch {:.0} ms; now {fine} 4 x 4, {coarse} 2 x 2", t.batches, t.quadrants, t.build_ms, t.longest_ms)
    }

    /// Forget the cells that are gone (`alive` false).
    fn prune(&mut self, alive: impl Fn(Entity) -> bool) {
        self.cells.retain(|e, _| alive(*e));
        let cells = &self.cells;
        self.by_grid.retain(|_, e| cells.contains_key(e));
    }

    /// Take the rule's levels for the camera at game-space `cam` (x, y) and list the
    /// quadrants whose wanted drawing differs from the drawn one.
    fn plan(&mut self, cam: Vec2) -> Vec<(Entity, u8, QuadrantDraw)> {
        let rule = self.rule;
        for c in self.cells.values_mut() {
            for q in 0..4u8 {
                let d = quadrant_distance(c.source.origin, q, cam);
                c.fine[q as usize] = rule.fine(d, c.fine[q as usize]);
            }
        }
        let mut out = Vec::new();
        for (&e, c) in &self.cells {
            for q in 0..4u8 {
                let across = [QUADRANT_NORTH, QUADRANT_EAST, QUADRANT_SOUTH, QUADRANT_WEST].map(|side| {
                    let ((dx, dy), q2) = neighbour(q, side);
                    let cell = if (dx, dy) == (0, 0) { Some(c) } else { self.by_grid.get(&(c.grid.0 + dx, c.grid.1 + dy)).and_then(|n| self.cells.get(n)) };
                    cell.map(|n| n.fine[q2 as usize])
                });
                let want = wanted_draw(c.fine[q as usize], across);
                if want != c.drawn[q as usize] {
                    out.push((e, q, want));
                }
            }
        }
        out
    }
}

/// Look at the levels, build a batch of redraws on the read pool, and swap a built batch in
/// (module notes). Runs only under 4 × 4 near (the cache holds a [`TerrainLod`]).
pub fn update_terrain_lod(
    mut commands: Commands,
    world: Option<Res<WorldData>>,
    camera: Query<&GlobalTransform, crate::fly::WorldCamera>,
    alive: Query<(), With<CellEntity>>,
    mut models: ModelCtx,
    mut terrain: TerrainMaterials,
) {
    let Some(world) = world else { return };
    let Some(mut lod) = terrain.cache.lod.take() else { return };
    lod.prune(|e| alive.contains(e));
    match lod.batch.take() {
        None => {
            if let Ok(cam) = camera.single() {
                let g = bevy_to_game(cam.translation());
                let plan = lod.plan(Vec2::new(g.x, g.y));
                if !plan.is_empty() {
                    let jobs = plan
                        .into_iter()
                        .map(|(e, q, draw)| {
                            let src = lod.cells[&e].source.clone();
                            let task = nvf_assets::read_pool::read_pool().spawn(async move { build_quadrant(&src.land, &src.surface, &src.layers, q, draw, src.origin, src.tile) });
                            (e, q, draw, task)
                        })
                        .collect();
                    lod.batch = Some(Batch::Building { started: Instant::now(), jobs, built: Vec::new() });
                }
            }
        }
        Some(Batch::Building { started, mut jobs, mut built }) => {
            jobs.retain_mut(|(e, q, draw, task)| match check_ready(task) {
                Some(meshes) => {
                    built.push((*e, *q, *draw, meshes));
                    false
                }
                None => true,
            });
            if !jobs.is_empty() {
                lod.batch = Some(Batch::Building { started, jobs, built });
            } else {
                // Built: spawn hidden under their cells (a cell gone meanwhile is skipped).
                let built_ms = started.elapsed().as_secs_f64() * 1000.0;
                let mut new = Vec::new();
                let mut ctx = TerrainCtx { tm: &mut terrain, assets: &mut models };
                for (e, q, draw, meshes) in built {
                    let Some(c) = lod.cells.get(&e) else { continue };
                    let src = c.source.clone();
                    let spawned = spawn_quadrant(&mut commands, e, &world, &src.layers, q, meshes, src.tile, &mut ctx, Visibility::Hidden, false);
                    new.push((e, q, draw, spawned.entities));
                }
                lod.batch = Some(Batch::Swapping { started, built_ms, frames: SWAP_FRAMES, new });
            }
        }
        Some(Batch::Swapping { started, built_ms, frames, new }) => {
            if frames > 1 {
                lod.batch = Some(Batch::Swapping { started, built_ms, frames: frames - 1, new });
            } else {
                let n = new.len() as u64;
                for (e, q, draw, entities) in new {
                    let Some(c) = lod.cells.get_mut(&e) else { continue };
                    for old in std::mem::replace(&mut c.entities[q as usize], entities.clone()) {
                        commands.entity(old).try_despawn();
                    }
                    for e in entities {
                        commands.entity(e).insert(Visibility::Inherited);
                    }
                    c.drawn[q as usize] = draw;
                }
                let ms = started.elapsed().as_secs_f64() * 1000.0;
                for c in [&mut lod.total, &mut lod.since_log] {
                    c.batches += 1;
                    c.quadrants += n;
                    c.build_ms += built_ms;
                    c.longest_ms = c.longest_ms.max(ms);
                }
                log_levels(&mut lod, n, ms);
            }
        }
    }
    terrain.cache.lod = Some(lod);
}

/// The `terrain levels:` lines (module notes).
fn log_levels(lod: &mut TerrainLod, n: u64, ms: f64) {
    let (fine, coarse) = lod.levels();
    let now = Instant::now();
    match lod.last_log {
        None => {
            info!("terrain levels: first redraw: {n} quadrants rebuilt in {ms:.0} ms; {fine} quadrants drawn 4 x 4, {coarse} 2 x 2 (near {}, far {})", lod.rule.near, lod.rule.far);
        }
        Some(t) if now.duration_since(t).as_secs_f64() >= LOG_EVERY => {
            let s = lod.since_log;
            info!(
                "terrain levels: {fine} quadrants drawn 4 x 4, {coarse} 2 x 2; {} redraws of {} quadrants in the last {:.0} s (build {:.0} ms on the read pool, longest batch {:.0} ms from start to swap)",
                s.batches,
                s.quadrants,
                now.duration_since(t).as_secs_f64(),
                s.build_ms,
                s.longest_ms
            );
        }
        Some(_) => return,
    }
    lod.last_log = Some(now);
    lod.since_log = Counts::default();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neighbours_across_each_side() {
        assert_eq!(neighbour(0, QUADRANT_EAST), ((0, 0), 1));
        assert_eq!(neighbour(1, QUADRANT_EAST), ((1, 0), 0));
        assert_eq!(neighbour(0, QUADRANT_WEST), ((-1, 0), 1));
        assert_eq!(neighbour(3, QUADRANT_WEST), ((0, 0), 2));
        assert_eq!(neighbour(0, QUADRANT_NORTH), ((0, 0), 2));
        assert_eq!(neighbour(2, QUADRANT_NORTH), ((0, 1), 0));
        assert_eq!(neighbour(3, QUADRANT_SOUTH), ((0, 0), 1));
        assert_eq!(neighbour(1, QUADRANT_SOUTH), ((0, -1), 3));
        // Across and back again.
        for q in 0..4u8 {
            for side in 0..4 {
                let (d, q2) = neighbour(q, side);
                let (back, q3) = neighbour(q2, (side + 2) % 4);
                assert_eq!((q3, (d.0 + back.0, d.1 + back.1)), (q, (0, 0)), "q{q} side {side}");
            }
        }
    }

    #[test]
    fn distance_to_a_quadrant_seen_from_above() {
        let o = Vec2::new(4096.0, -4096.0);
        assert_eq!(quadrant_distance(o, 0, Vec2::new(5000.0, -3000.0)), 0.0);
        assert_eq!(quadrant_distance(o, 1, Vec2::new(5000.0, -3000.0)), 6144.0 - 5000.0);
        assert_eq!(quadrant_distance(o, 3, Vec2::new(4096.0 + 2048.0 - 300.0, -4096.0 + 2048.0 - 400.0)), 500.0);
    }

    /// 4 × 4 sides are coarse where the quadrant across is 2 × 2 or not loaded; 2 × 2
    /// quadrants never move a side.
    #[test]
    fn the_wanted_drawing() {
        assert_eq!(wanted_draw(false, [Some(true); 4]), QuadrantDraw::HALF);
        assert_eq!(wanted_draw(true, [Some(true); 4]), QuadrantDraw::FULL);
        assert_eq!(wanted_draw(true, [Some(true), Some(false), None, Some(true)]), QuadrantDraw { step: 1, coarse: [false, true, true, false] });
    }

    fn source(grid: (i32, i32)) -> Arc<TerrainSource> {
        let heights = vec![0.0; fnv_world::land::SIDE * fnv_world::land::SIDE];
        let land = fnv_world::land::Land { heights: Some(heights.clone()), ..Default::default() };
        let surface = fnv_world::land_surface::Surface::stored(&heights);
        Arc::new(TerrainSource { land, surface, layers: Vec::new(), origin: Vec2::new(grid.0 as f32, grid.1 as f32) * fnv_world::CELL_SIZE, tile: 256.0 })
    }

    fn lod_with(cells: &[(u32, (i32, i32))]) -> TerrainLod {
        let mut lod = TerrainLod::new(DistanceRule::with_near(2048.0));
        for &(e, g) in cells {
            lod.add_cell(Entity::from_raw_u32(e).unwrap(), source(g), QuadrantDraw::HALF, Default::default());
        }
        lod
    }

    /// Over the middle of cell (0, 0) of a 3 × 3 block: its four quadrants and the eight
    /// quadrants touching it within 2048 units become 4 × 4, each with its sides coarse
    /// toward the quadrants that stay 2 × 2; drawn as wanted, nothing more is planned; the
    /// camera wobbling in the hysteresis band plans nothing; leaving plans them back.
    #[test]
    fn the_plan_follows_the_camera_with_hysteresis() {
        let grid: Vec<(u32, (i32, i32))> = (0..9).map(|k| (k as u32 + 1, ((k % 3) as i32 - 1, (k / 3) as i32 - 1))).collect();
        let mut lod = lod_with(&grid);
        let centre = Vec2::splat(fnv_world::CELL_SIZE / 2.0);
        let plan = lod.plan(centre);
        // The centre cell's 4 quadrants (distance 0) and the 8 edge-sharing quadrants of the
        // cells around it at exactly 2048 (not within: < near) stay 2 x 2 but for the four
        // at distance 0.
        assert_eq!(plan.len(), 4, "{plan:?}");
        let mid = Entity::from_raw_u32(5).unwrap();
        for (e, _, d) in &plan {
            assert_eq!(*e, mid);
            assert_eq!(d.step, 1);
            // Each centre quadrant: its two sides toward the other centre quadrants fine,
            // the two outward coarse.
            assert_eq!(d.coarse.iter().filter(|c| **c).count(), 2, "{d:?}");
        }
        for (e, q, d) in plan {
            lod.cells.get_mut(&e).unwrap().drawn[q as usize] = d;
        }
        assert!(lod.plan(centre).is_empty(), "drawn as wanted");
        assert_eq!(lod.levels(), (4, 32));
        // Moved 300 units east: the quadrants of the east cell's west half come within 2048.
        let east = centre + Vec2::new(300.0, 0.0);
        let plan = lod.plan(east);
        let east_cell = Entity::from_raw_u32(6).unwrap();
        assert!(plan.iter().any(|(e, q, d)| *e == east_cell && (*q == 0 || *q == 2) && d.step == 1), "{plan:?}");
        // The centre's east quadrants now meet 4 x 4 quadrants on their east sides: redrawn
        // with those sides no longer coarse.
        assert!(plan.iter().any(|(e, q, d)| *e == mid && *q == 1 && !d.coarse[QUADRANT_EAST]), "{plan:?}");
        for (e, q, d) in plan {
            lod.cells.get_mut(&e).unwrap().drawn[q as usize] = d;
        }
        // Back 200 units west: still within the far distance of the east cell's quadrants
        // (2048 + 512): nothing changes.
        assert!(lod.plan(centre + Vec2::new(100.0, 0.0)).is_empty());
        // Far away: everything back to 2 x 2.
        let plan = lod.plan(Vec2::new(1.0e6, 0.0));
        assert!(plan.iter().all(|(_, _, d)| *d == QuadrantDraw::HALF) && !plan.is_empty());
    }

    /// Cells gone are forgotten, and a cell spawned again on the same grid square replaces
    /// the old one.
    #[test]
    fn cells_come_and_go() {
        let mut lod = lod_with(&[(1, (0, 0)), (2, (1, 0))]);
        lod.prune(|e| e == Entity::from_raw_u32(2).unwrap());
        assert_eq!((lod.cells.len(), lod.by_grid.len()), (1, 1));
        lod.add_cell(Entity::from_raw_u32(3).unwrap(), source((1, 0)), QuadrantDraw::HALF, Default::default());
        assert_eq!((lod.cells.len(), lod.by_grid.get(&(1, 0)).copied()), (1, Some(Entity::from_raw_u32(3).unwrap())));
        assert!(lod.summary().starts_with("4 x 4 near: 0 redraws, 0 quadrants rebuilt"));
    }
}
