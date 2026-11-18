//! The full-detail scenery ring (#2701, Stage 1; docs/design/graphics-enhanced.md §3.17):
//! with the settings page's Full-detail scenery row at 7 x 7 or 9 x 9, the full models of
//! static scenery are drawn in the cells just beyond the loaded 5 x 5, out to 3 or 4 grid
//! steps from the camera's cell, where the distant LOD blocks would stand in for them.
//! Off by default and in both presets. Everything here is NVForever's own design, not the
//! original's behaviour.
//!
//! - **Draw-only.** The game keeps loading exactly the cells it loads without the ring:
//!   ring cells are never in `Streamer::loaded`, so `Game::loaded_cells`, physics, AI
//!   process levels, water, rooms, reflections, reports and readiness checks are as they
//!   were. A ring cell's entity carries [`RingCell`] and neither `world::CellEntity` nor
//!   `nvf_play::RenderCell`, so nvf-play draws no dropped or script-enabled references into
//!   it, moves nothing in it and the terrain wireframe, decals and the mesh terrain blend
//!   leave it alone. Its models keep `RefVisual` (the simulation's enable state, deletion,
//!   texture swaps and fades still reach them) and carry `nvf_play::DrawOnly`, which the
//!   picks, the crosshair, the loaded counts and the scripts' NIF functions leave out. No
//!   collider is made anywhere.
//! - **What it draws** ([`ring_choice`]): STAT, SCOL and MSTT references (a moveable
//!   static with its idle sequence playing), SpeedTrees (their generated models; the
//!   distant tree billboards crossfade with ring cells as with loaded ones, treelod.rs),
//!   and any other model-bearing reference flagged "visible when distant" (REFR record flag
//!   0x8000 [documented: GECK wiki "Reference": the reference is added to an object or
//!   tree LOD; GetReferenceFlag 32768]), drawn as plain static scenery
//!   (`world::SpawnMode::Scenery`: no other rig, no light, emitter or collision), and the
//!   cell's landscape at full resolution (under 4 x 4 near the 2 x 2 first drawing the
//!   loaded cells start with, which meets the loaded edge without cracks). With the game
//!   running, references not enabled at the start (but not deleted) are drawn too and the
//!   simulation shows or hides them (`RefVisual`, `activate::sync_visuals` on every
//!   `RefChanged`, which the game sends for any reference, loaded or not); moved and
//!   destroyed references follow the game (nvf-play `moved.rs` through
//!   `nvf_play::DrawOnlyCell`, destroyed.rs). Left out: imposters, primitives, markers,
//!   deleted references, actors, light sources, sounds, placed water, water and grass, and
//!   every non-flagged activator, door, container and item.
//! - **LOD hand-off** ([`DrawnCells`]): an object LOD block's segment holds the LOD of its
//!   cell's flagged references (world.md §7), so lod.rs hides the segments of ring cells
//!   and cuts the LOD terrain out under them exactly as for loaded cells, crossfading with
//!   their fade. A cell keeps its segment when a flagged reference in it is not drawn here
//!   (an actor, placed water, one disabled at the start that the game has enabled, one
//!   whose model drew nothing): then the segment shows beside the full models of the
//!   cell's other references, which doubles those (a kept segment holds the LOD of all
//!   the cell's flagged references), rather than leave a hole. Elsewhere no doubling at
//!   rest. The hand-off with a loaded cell follows one rule ([`hand_off`]): the LOD
//!   segment, the ring cell and the loaded cell draw each pixel once at every moment.
//! - **Streaming** ([`stream`]): ring cells are the cells `r + 1 ..= R` steps from the
//!   loaded grid's centre (`r` the loaded radius, `R` the ring's), faded out beyond
//!   `R + 1` (and, when the row is turned off or made smaller, those out of its new
//!   reach), and dropped once their square is loaded and fully drawn within the loaded
//!   radius. In the loaded grid's one cell of hysteresis a ring cell may stand by under a
//!   loaded cell, drawn as the complement of its fade, so it takes over when that cell
//!   fades out. New ring cells are read ahead on the worker threads only while the loaded
//!   read-ahead is idle and spawned, nearest first, within [`SPAWN_BUDGET`] a frame (the
//!   read-ahead's bookkeeping included; a cell's landscape or one reference is never
//!   split, so a frame can pass the budget by one of them), only while the loaded grid is
//!   complete. No model or landscape is prepared on the main thread for the ring: a
//!   reference whose model (a destruction stage's too) is not prepared waits for the
//!   worker threads. A door's travel (`Streamer::travels`), a change of worldspace or
//!   going indoors drops the ring and its readings at once and never waits for it.
//! - **Shadows**: ring meshes cast no sun shadow while the sun's cascades end inside the
//!   ring's inner edge ([`ring_casts_shadows`]: the default 8,000 units against the
//!   8,192 a ring cell is at least from the camera); with longer reach they cast.
//! - **Logs**: `full detail: 9 x 9 ring (radius 4)` when the ring comes on, and at the
//!   exit `full detail summary: 9x9, <n> cells, <m> references drawn, <k> skipped by type,
//!   <t> ms on the main thread, <s> too small for their distance`. Nothing while it is off.

use crate::emittance::EmittanceState;
use crate::fade::{self, Fading};
use crate::graphics::Graphics;
use crate::terrain::TerrainMaterials;
use crate::world::{self, classify_ref, AheadArea, DrawnAs, ReadAhead, RefKind, SpawnMode, StreamStats, Streamer, WorldData, WorldRoot};
use bevy::camera::visibility::VisibilityRange;
use bevy::ecs::system::SystemParam;
use bevy::light::NotShadowCaster;
use bevy::math::Affine3A;
use bevy::prelude::*;
use fnv_world::placed::{base_object, placement, ref_flags, BaseObject, Placement};
use fnv_world::CELL_SIZE;
use nvf_assets::model::ModelCtx;
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

pub type Grid = (i32, i32);

/// The settings page's Full-detail scenery row (`Graphics::full_detail`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FullDetail {
    /// Only the loaded cells draw full models (as the original).
    #[default]
    Off,
    /// Out to 3 grid steps: 7 x 7 cells.
    Seven,
    /// Out to 4 grid steps: 9 x 9 cells.
    Nine,
}

impl FullDetail {
    /// The row's choices in its order, by the names it shows and saves.
    pub const CHOICES: [(&'static str, FullDetail); 3] = [("Off", FullDetail::Off), ("7 x 7", FullDetail::Seven), ("9 x 9", FullDetail::Nine)];

    /// Grid steps from the camera's cell the ring reaches (0: off) [design].
    pub fn radius(self) -> i32 {
        match self {
            FullDetail::Off => 0,
            FullDetail::Seven => 3,
            FullDetail::Nine => 4,
        }
    }

    /// Cells along a side of the square the ring reaches.
    pub fn side(self) -> i32 {
        2 * self.radius() + 1
    }

    /// The row's name of the choice.
    pub fn name(self) -> &'static str {
        Self::CHOICES.iter().find(|(_, f)| *f == self).map_or("Off", |(n, _)| n)
    }

    /// For `Graphics::describe` (empty while off).
    pub fn describe(self) -> &'static str {
        match self {
            FullDetail::Off => "",
            FullDetail::Seven => "full-detail scenery 7 x 7",
            FullDetail::Nine => "full-detail scenery 9 x 9",
        }
    }
}

/// Main-thread time a frame spends spawning ring cells [engineering choice: half the
/// loaded cells' 3 ms (`world::SPAWN_BUDGET`); the ring is never waited for].
pub const SPAWN_BUDGET: Duration = Duration::from_micros(1500);

/// In the ring's outermost cells, a reference that is not flagged visible when distant is
/// left out when its model's bounding radius is under this part of its distance from the
/// centre of the loaded grid: about 57 units at 4 cells [engineering choice; 0 turns the
/// cut off]. A flagged reference is never cut: its LOD segment is hidden.
pub const SIZE_CUT: f32 = 0.004;

/// On a ring cell's entity: the grid square it draws (module notes).
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct RingCell {
    pub grid: Grid,
}

/// The `NotShadowCaster` this module put on a ring mesh (only those are taken off again).
#[derive(Component)]
pub struct RingNoShadow;

/// Which range a ring cell's meshes are drawn with ([`ring_range`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RingRange {
    /// Its own fade's step (`fade::drawn`).
    Own(u8),
    /// Under a loaded cell of the same square: the complement of that cell's step
    /// (`fade::complement`), so the two never draw the same pixel.
    Under(u8),
}

impl RingRange {
    pub fn range(self) -> VisibilityRange {
        match self {
            RingRange::Own(k) => fade::drawn(k),
            RingRange::Under(k) => fade::complement(k),
        }
    }

    /// Nothing of the cell is drawn.
    pub fn hidden(self) -> bool {
        matches!(self, RingRange::Under(k) if k >= fade::STEPS) || self == RingRange::Own(0)
    }
}

/// The range for a ring cell at `own` (0 … 1) whose square has a loaded cell at
/// `loaded` (its level, `Streamer::cell_level`), if any.
pub fn ring_range(own: f32, loaded: Option<f32>) -> RingRange {
    match loaded {
        Some(l) => RingRange::Under(fade::step(l)),
        None => RingRange::Own(fade::step(own)),
    }
}

/// How a ring cell and the loaded cell of its square share the pixels, so that the LOD
/// segment, the ring cell and the loaded cell cover each pixel once at every moment
/// [design]: a ring cell under a loaded cell is held at rest (its level 1, so its LOD
/// segment's level is 1 and the segment draws nothing) and draws the complement of the
/// loaded cell's step ([`ring_range`]); a ring cell that is fading out when a loaded cell
/// comes is dropped at once, leaving the loaded cell and the LOD to share the pixels as
/// they do without the ring. `level`, `leaving`: the ring cell's; `loaded`: a loaded cell
/// is in its square. The ring cell's new level, or `None` to drop it.
pub fn hand_off(level: f32, leaving: bool, loaded: bool) -> Option<f32> {
    match (loaded, leaving) {
        (false, _) => Some(level),
        (true, true) => None,
        (true, false) => Some(1.0),
    }
}

/// One drawn ring cell.
#[derive(Debug)]
pub struct RingCellState {
    pub entity: Entity,
    /// 0 hidden … 1 fully drawn (its own fade).
    pub level: f32,
    /// Fading out, to be retired at 0.
    pub leaving: bool,
    /// A flagged reference here is not drawn: the cell's LOD segment stays (module notes).
    pub keep_segment: bool,
    /// References the size cut left out while the cell was in the ring's outermost step,
    /// spawned once it is not ([`SIZE_CUT`]).
    small: Vec<u32>,
    applied: Option<RingRange>,
    shadow: Option<bool>,
}

impl RingCellState {
    fn new(entity: Entity, level: f32, keep_segment: bool, small: Vec<u32>) -> RingCellState {
        RingCellState { entity, level, leaving: false, keep_segment, small, applied: None, shadow: None }
    }

    /// Fully drawn and staying.
    pub fn full(&self) -> bool {
        !self.leaving && fade::step(self.level) == fade::STEPS
    }
}

/// A ring cell whose references are being spawned (hidden until all are in).
struct RingSpawn {
    entity: Entity,
    refs: Vec<u32>,
    next: usize,
    /// References whose model was not prepared yet: tried again once `refs` are done.
    later: Vec<u32>,
    origin: Vec3,
    keep_segment: bool,
    /// In the ring's outermost step ([`SIZE_CUT`]), and what the cut left out.
    outer: bool,
    small: Vec<u32>,
}

impl RingSpawn {
    fn done(&self) -> bool {
        self.next >= self.refs.len() && self.later.is_empty()
    }
}

/// What the ring did over the run, for the exit summary.
#[derive(Default, Debug, Clone, Copy)]
pub struct RingStats {
    pub cells: usize,
    pub skipped_type: usize,
    pub too_small: usize,
    pub ms: f64,
}

/// The ring's cells (module notes).
#[derive(Resource, Default)]
pub struct FullDetailRing {
    /// Grid steps it reaches (0: off).
    pub radius: i32,
    /// The worldspace its cells are of, and `Streamer::travels` when they were drawn.
    world: u32,
    travels: u64,
    pub cells: HashMap<Grid, RingCellState>,
    spawning: HashMap<Grid, RingSpawn>,
    dying: Vec<Entity>,
    ahead: ReadAhead,
    /// `world::spawn_model`'s counts for the ring's models.
    models: StreamStats,
    pub stats: RingStats,
    /// The last choice that was on (the exit summary is logged only then).
    used: Option<FullDetail>,
}

impl FullDetailRing {
    /// Drop every ring cell and every reading now: a door's travel, a change of
    /// worldspace, going indoors (the loaded cells go at once then too).
    fn clear(&mut self, commands: &mut Commands) {
        for e in self.cells.drain().map(|(_, s)| s.entity).chain(self.spawning.drain().map(|(_, s)| s.entity)).chain(self.dying.drain(..)) {
            commands.entity(e).try_despawn();
        }
        self.ahead = ReadAhead::default();
    }

    /// `world::spawn_model`'s counts for a ring copy drawn again (destroyed.rs).
    pub(crate) fn redraw_stats(&mut self) -> &mut StreamStats {
        &mut self.models
    }

    /// Ring cell `grid` got new meshes (destroyed.rs): [`fade_ring`] gives them their range
    /// and shadow rule.
    pub(crate) fn redrawn(&mut self, grid: Grid) {
        if let Some(s) = self.cells.get_mut(&grid) {
            s.applied = None;
            s.shadow = None;
        }
    }

    /// Hide a ring cell and despawn it over the next frames.
    fn retire(&mut self, commands: &mut Commands, e: Entity) {
        commands.entity(e).insert(Visibility::Hidden);
        self.dying.push(e);
    }
}

/// Grid steps between two cells (the larger of the two axes).
pub fn steps(c: Grid, centre: Grid) -> i32 {
    (c.0 - centre.0).abs().max((c.1 - centre.1).abs())
}

/// Nearest first, as `world::stream` orders cells.
fn order_key(c: Grid, centre: Grid) -> i32 {
    steps(c, centre) * 1000 + (c.0 - centre.0).abs() + (c.1 - centre.1).abs()
}

/// The ring's cells around `centre`: `loaded + 1` to `ring` steps out, nearest first.
pub fn ring_cells(centre: Grid, loaded: i32, ring: i32) -> Vec<Grid> {
    let mut out: Vec<Grid> = (-ring..=ring).flat_map(|dx| (-ring..=ring).map(move |dy| (centre.0 + dx, centre.1 + dy))).filter(|&c| steps(c, centre) > loaded).collect();
    out.sort_by_key(|&c| order_key(c, centre));
    out
}

/// Whether a drawn ring cell `steps` out goes at once: inside the loaded radius where its
/// loaded cell is fully drawn.
pub fn covered(steps: i32, loaded: i32, loaded_full: bool) -> bool {
    steps <= loaded && loaded_full
}

/// Whether a drawn ring cell `steps` out fades out: past the ring's one step of
/// hysteresis (`ring + 1`), the ring off (`ring` 0), or, in the frame the ring shrank,
/// past its new reach.
pub fn fades_out(steps: i32, ring: i32, shrank: bool) -> bool {
    ring == 0 || steps > ring + 1 || (shrank && steps > ring)
}

/// Whether a cell whose references the size cut left out should get them now: it is no
/// longer in the ring's outermost step.
pub fn small_due(steps: i32, ring: i32, left_out: usize) -> bool {
    left_out > 0 && steps < ring
}

/// What the ring does with a reference ([`ring_choice`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RingRef {
    /// Drawn as static scenery.
    Draw,
    /// Left out for its type (counted in the summary).
    SkipType,
    /// Left out (drawn elsewhere or not at all: imposters, disabled references).
    Skip,
    /// Flagged visible when distant but not drawn here: its cell keeps its LOD segment.
    KeepSegment,
}

/// The ring's type filter (module notes): `kind` from `world::classify_ref`, `sig` the
/// base's record type, `flagged` the reference's "visible when distant" flag.
pub fn ring_choice(kind: RefKind, sig: Option<&[u8; 4]>, flagged: bool) -> RingRef {
    let flag = |yes| if flagged { yes } else { RingRef::SkipType };
    match kind {
        RefKind::Imposter => RingRef::Skip,
        RefKind::Primitive | RefKind::Skipped => RingRef::SkipType,
        // Decided before the type (`spawn_ring_ref`: drawn for the simulation to show).
        RefKind::Disabled => RingRef::Skip,
        RefKind::Actor => flag(RingRef::KeepSegment),
        RefKind::Drawn { how: DrawnAs::Tree, .. } => RingRef::Draw,
        RefKind::Drawn { how: DrawnAs::PlacedWater, .. } => flag(RingRef::KeepSegment),
        RefKind::Drawn { how: DrawnAs::Model, .. } if matches!(sig, Some(b"STAT" | b"SCOL" | b"MSTT")) => RingRef::Draw,
        RefKind::Drawn { how: DrawnAs::Model, .. } => flag(RingRef::Draw),
    }
}

/// The reference's "visible when distant" flag (module notes).
pub fn flagged(p: &Placement) -> bool {
    p.flags & ref_flags::VISIBLE_WHEN_DISTANT != 0
}

/// The models a ring cell's read-ahead prepares (`world::read_ahead_cells`): SpeedTrees
/// are not models (tree.rs builds them).
fn ring_models(p: &Placement, base: &BaseObject) -> bool {
    !base.is_speedtree() && ring_choice(classify_ref(p, Some(base), || true), Some(&base.sig), flagged(p)) == RingRef::Draw
}

/// Whether ring meshes cast sun shadows: only while the cascades (`reach` units) reach
/// past the ring's inner edge, `loaded` cells out from the camera's cell's edge.
pub fn ring_casts_shadows(shadows: bool, reach: f32, loaded: i32) -> bool {
    shadows && reach > loaded.max(0) as f32 * CELL_SIZE
}

/// The ring's streaming each frame (module notes); after `world::stream`, before
/// [`fade_ring`].
#[allow(clippy::too_many_arguments)]
pub fn stream(
    mut commands: Commands,
    world: Res<WorldData>,
    graphics: Res<Graphics>,
    streamer: Res<Streamer>,
    mut ring: ResMut<FullDetailRing>,
    root: Query<Entity, With<WorldRoot>>,
    time: Res<Time>,
    mut models: ModelCtx,
    mut terrain: TerrainMaterials,
    mut emittance: ResMut<EmittanceState>,
    (sim, loading): (Option<Res<nvf_play::sim::SimState>>, Option<Res<nvf_play::loading::Loading>>),
    children: Query<&Children>,
    (mut trees, distances): (crate::tree::TreeCtx, Option<ResMut<crate::draw_distance::DrawDistances>>),
) {
    let ring = &mut *ring;
    let choice = graphics.full_detail;
    // The Object distance slider reaches the ring's edge while it is on (draw_distance.rs).
    if let Some(mut d) = distances {
        let reach = (choice.radius() > streamer.radius).then(|| crate::draw_distance::full_reach(choice.radius()));
        if d.ring_reach != reach {
            d.ring_reach = reach;
        }
    }
    if choice.radius() == 0 && ring.radius == 0 && ring.cells.is_empty() && ring.spawning.is_empty() && ring.dying.is_empty() {
        return;
    }
    world::despawn_dying(&mut commands, &mut ring.dying, &children);
    // Gone at once with the loaded cells: a travel, another worldspace, an interior.
    let centre = streamer.centre.filter(|_| streamer.interior.is_none());
    if ring.world != world.world || ring.travels != streamer.travels || centre.is_none() {
        ring.clear(&mut commands);
        ring.world = world.world;
        ring.travels = streamer.travels;
    }
    // The row changed: cells out of the new reach fade out (below), as cells leaving do.
    let shrank = choice.radius() < ring.radius;
    if choice.radius() != ring.radius {
        ring.radius = choice.radius();
        if ring.radius > 0 {
            info!("full detail: {0} x {0} ring (radius {1})", choice.side(), ring.radius);
            ring.used = Some(choice);
        } else {
            ring.ahead = ReadAhead::default();
        }
    }
    let Some(centre) = centre else { return };
    let Ok(root) = root.single() else { return };
    let (loaded, outer) = (streamer.radius, ring.radius);
    let fading = streamer.fade_secs > 0.0;
    let loaded_full = |c: &Grid| streamer.loaded.contains_key(c) && !streamer.fades.contains_key(c);

    // The hand-off with the loaded cells ([`hand_off`]), then the cells that go.
    let mut gone: Vec<Grid> = Vec::new();
    for (c, s) in &mut ring.cells {
        let st = steps(*c, centre);
        match hand_off(s.level, s.leaving, streamer.loaded.contains_key(c)) {
            None => gone.push(*c),
            Some(_) if covered(st, loaded, loaded_full(c)) => gone.push(*c),
            Some(level) => {
                s.level = level;
                if fades_out(st, outer, shrank) {
                    if fading {
                        s.leaving = true;
                    } else {
                        gone.push(*c);
                    }
                }
            }
        }
    }
    let want = if outer > 0 { ring_cells(centre, loaded, outer) } else { Vec::new() };
    for c in &want {
        if let Some(s) = ring.cells.get_mut(c) {
            s.leaving = false;
        }
    }
    let rate = if fading { time.delta_secs() / streamer.fade_secs } else { 1.0 };
    for (c, s) in &mut ring.cells {
        s.level = (s.level + if s.leaving { -rate } else { rate }).clamp(0.0, 1.0);
        if s.leaving && s.level <= 0.0 {
            gone.push(*c);
        }
    }
    for c in gone {
        if let Some(s) = ring.cells.remove(&c) {
            ring.retire(&mut commands, s.entity);
        }
    }
    // A cell being spawned is not drawn yet: dropped at once.
    let dropped: Vec<Grid> = ring.spawning.keys().filter(|&&c| fades_out(steps(c, centre), outer, shrank) || steps(c, centre) <= loaded).copied().collect();
    for c in dropped {
        if let Some(sp) = ring.spawning.remove(&c) {
            ring.retire(&mut commands, sp.entity);
        }
    }
    if outer == 0 {
        return;
    }

    // New work only while the loaded grid is complete and nothing of it is spawning. The
    // budget's clock starts before the read-ahead's bookkeeping, which is part of it.
    let started = Instant::now();
    let deadline = started + SPAWN_BUDGET;
    let behind_screen = loading.is_some_and(|l| l.unseen_frames());
    let complete = (-loaded..=loaded).all(|dx| (-loaded..=loaded).all(|dy| streamer.loaded.contains_key(&(centre.0 + dx, centre.1 + dy))));
    let idle = streamer.settled && complete && streamer.spawning.is_empty() && !behind_screen;
    let tile = terrain.settings.tile;
    {
        let FullDetailRing { cells, spawning, ahead, .. } = ring;
        let area = AheadArea { centre, inner: loaded + 1, ring: outer, start: idle && !streamer.ahead.busy() };
        world::read_ahead_cells(&world, ahead, area, |c| cells.contains_key(c) || spawning.contains_key(c), &streamer.hide, ring_models, tile, &mut models);
    }
    if !idle {
        ring.stats.ms += started.elapsed().as_secs_f64() * 1000.0;
        return;
    }
    let sim = sim.as_deref();
    let ctx = RefCtx { world: &world, streamer: &streamer, centre, sim, tree_builds: std::cell::Cell::new(0) };
    let mut under_way: Vec<Grid> = ring.spawning.keys().copied().collect();
    under_way.sort_by_key(|&c| order_key(c, centre));
    for c in under_way {
        if Instant::now() >= deadline {
            break;
        }
        let Some(mut sp) = ring.spawning.remove(&c) else { continue };
        spawn_refs(&mut commands, &ctx, &mut sp, deadline, &mut models, &mut emittance, &mut trees, ring);
        if sp.done() {
            finish(&mut commands, ring, c, sp, fading);
        } else {
            ring.spawning.insert(c, sp);
        }
    }
    // Drawn cells no longer outermost get what the size cut left out of them.
    let mut due: Vec<Grid> = ring.cells.iter().filter(|(c, s)| !s.leaving && small_due(steps(**c, centre), outer, s.small.len())).map(|(c, _)| *c).collect();
    due.sort_by_key(|&c| order_key(c, centre));
    for c in due {
        if Instant::now() >= deadline {
            break;
        }
        let Some(mut s) = ring.cells.remove(&c) else { continue };
        let mut sp = RingSpawn { entity: s.entity, refs: std::mem::take(&mut s.small), next: 0, later: Vec::new(), origin: cell_origin(c), keep_segment: s.keep_segment, outer: false, small: Vec::new() };
        spawn_refs(&mut commands, &ctx, &mut sp, deadline, &mut models, &mut emittance, &mut trees, ring);
        s.small = sp.refs.split_off(sp.next.min(sp.refs.len()));
        s.small.extend(sp.later);
        s.keep_segment = sp.keep_segment;
        // The new meshes get the cell's range and shadow rule (fade_ring walks it again).
        s.applied = None;
        s.shadow = None;
        ring.cells.insert(c, s);
    }
    // Cells not drawn yet, nearest first; those standing by under a loaded cell last.
    let mut missing: Vec<Grid> = want.into_iter().filter(|c| !ring.cells.contains_key(c) && !ring.spawning.contains_key(c)).collect();
    missing.sort_by_key(|&c| (streamer.loaded.contains_key(&c), order_key(c, centre)));
    for c in missing {
        if Instant::now() >= deadline || !world::cell_ready(&world, &mut ring.ahead, c, &mut models) {
            // Drawn once read ahead (nearest first: a farther cell does not jump the queue).
            break;
        }
        let prep = ring.ahead.take_terrain(c);
        let origin = cell_origin(c);
        let entity = commands.spawn((Name::new(format!("Ring cell {},{}", c.0, c.1)), RingCell { grid: c }, nvf_play::DrawOnlyCell, Transform::from_translation(origin), Visibility::Hidden)).id();
        commands.entity(root).add_child(entity);
        let mut refs = Vec::new();
        if let Some(cell) = world.grid_cell(c) {
            // Only from the read-ahead: a landscape is never read on this thread here.
            if prep.is_some() {
                world::spawn_cell_terrain(&mut commands, &world, entity, c, cell, prep, tile, &mut models, &mut terrain, true);
            }
            refs = cell.refs.clone();
        }
        refs.extend(world.persistent_by_grid.get(&c).into_iter().flatten());
        let mut sp = RingSpawn { entity, refs, next: 0, later: Vec::new(), origin, keep_segment: false, outer: steps(c, centre) == outer, small: Vec::new() };
        spawn_refs(&mut commands, &ctx, &mut sp, deadline, &mut models, &mut emittance, &mut trees, ring);
        if sp.done() {
            finish(&mut commands, ring, c, sp, fading);
        } else {
            ring.spawning.insert(c, sp);
        }
    }
    ring.stats.ms += started.elapsed().as_secs_f64() * 1000.0;
}

/// A grid square's south-west corner (game space).
fn cell_origin(c: Grid) -> Vec3 {
    Vec3::new(c.0 as f32 * CELL_SIZE, c.1 as f32 * CELL_SIZE, 0.0)
}

/// What a ring reference's spawn reads.
struct RefCtx<'a> {
    world: &'a WorldData,
    streamer: &'a Streamer,
    centre: Grid,
    sim: Option<&'a nvf_play::sim::SimState>,
    /// TREE geometry built on this thread this frame ([`TREE_BUILDS_PER_FRAME`]).
    tree_builds: std::cell::Cell<u32>,
}

/// TREE bases whose geometry the ring builds on the main thread in one frame, when the
/// loaded cells have not built it already: tree.rs has no worker-thread path for a
/// SpeedTree, so a tree not built yet waits for the next frame beyond this [design].
pub const TREE_BUILDS_PER_FRAME: u32 = 1;

/// Spawn a ring cell's references until all are in or the deadline has passed; those
/// whose model is not prepared go to `later`, tried again (on a later frame) once the
/// rest are in; those the size cut leaves out to `small`.
#[allow(clippy::too_many_arguments)]
fn spawn_refs(commands: &mut Commands, ctx: &RefCtx, sp: &mut RingSpawn, deadline: Instant, models: &mut ModelCtx, emittance: &mut EmittanceState, trees: &mut crate::tree::TreeCtx, ring: &mut FullDetailRing) {
    let target = Target { entity: sp.entity, origin: sp.origin, outer: sp.outer };
    while sp.next < sp.refs.len() && Instant::now() < deadline {
        let r = sp.refs[sp.next];
        sp.next += 1;
        match spawn_ring_ref(commands, ctx, target, &mut sp.keep_segment, r, models, emittance, trees, ring) {
            Spawned::Done => {}
            Spawned::Later => sp.later.push(r),
            Spawned::Small => sp.small.push(r),
        }
    }
    if sp.next >= sp.refs.len() && !sp.later.is_empty() {
        sp.refs = std::mem::take(&mut sp.later);
        sp.next = 0;
    }
}

/// The ring cell a reference is spawned into.
#[derive(Clone, Copy)]
struct Target {
    entity: Entity,
    origin: Vec3,
    outer: bool,
}

/// What became of a ring reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Spawned {
    /// Drawn, or left out for good.
    Done,
    /// Its model is being prepared on a worker thread: try again later.
    Later,
    /// Left out by the size cut ([`SIZE_CUT`]).
    Small,
}

/// One reference of a ring cell ([`ring_choice`], [`SIZE_CUT`]); `keep`: its cell keeps
/// its LOD segment. Never prepares a model on this thread: one not prepared yet is asked
/// of the worker threads and the reference waits ([`Spawned::Later`]).
#[allow(clippy::too_many_arguments)]
fn spawn_ring_ref(commands: &mut Commands, ctx: &RefCtx, sp: Target, keep: &mut bool, r: u32, models: &mut ModelCtx, emittance: &mut EmittanceState, trees: &mut crate::tree::TreeCtx, ring: &mut FullDetailRing) -> Spawned {
    let world = ctx.world;
    let Some(p) = placement(world.lo, world.schema, r) else { return Spawned::Done };
    // Not enabled at the start: drawn when the game runs (its simulation hides it while it
    // is disabled, `activate::sync_visuals`), else (the viewer) left out; deleted: out.
    if !drawn_for_enable_state(world.enabled(&p, 0), p.deleted(), ctx.sim.is_some()) {
        return Spawned::Done;
    }
    let base = p.base.and_then(|b| base_object(world.lo, world.schema, b));
    let flag = flagged(&p);
    match ring_choice(classify_ref(&p, base.as_ref(), || true), base.as_ref().map(|b| &b.sig), flag) {
        RingRef::Draw => {}
        RingRef::SkipType => {
            ring.stats.skipped_type += 1;
            return Spawned::Done;
        }
        RingRef::Skip => return Spawned::Done,
        RingRef::KeepSegment => {
            *keep = true;
            return Spawned::Done;
        }
    }
    let Some(base) = base else { return Spawned::Done };
    let rt = Affine3A::from_translation(-sp.origin) * world::ref_affine(&p);
    if base.is_speedtree() {
        return spawn_ring_tree(commands, ctx, sp, r, &p, &base, rt, models, trees, ring);
    }
    // A destroyed reference: its destruction stage's model (destroyed.rs), as the loaded
    // cells draw it at the time.
    let base = match ctx.streamer.model_overrides.get(&r) {
        Some(m) => world::base_object_with_model(base, m),
        None => base,
    };
    if let Some(path) = base.world_model_path() {
        if world::is_drawn_marker(base.form_id, &path) {
            ring.stats.skipped_type += 1;
            return Spawned::Done;
        }
        if !models.models_ready([(path.as_str(), base.world_alt_textures.as_slice())]) {
            return Spawned::Later;
        }
        if sp.outer && !flag && SIZE_CUT > 0.0 {
            let radius = models.model_swapped(&path, false, &base.world_alt_textures).and_then(|m| m.bounds).map(|(lo, hi)| (hi - lo).length() * 0.5 * p.scale);
            let centre = Vec2::new((ctx.centre.0 as f32 + 0.5) * CELL_SIZE, (ctx.centre.1 as f32 + 0.5) * CELL_SIZE);
            let distance = Vec2::new(p.position[0], p.position[1]).distance(centre);
            if radius.is_some_and(|radius| radius < SIZE_CUT * distance) {
                ring.stats.too_small += 1;
                return Spawned::Small;
            }
        }
    }
    let drawn = world::spawn_model(commands, sp.entity, world, &p, r, &base, rt, models, emittance, &ctx.streamer.hide, &mut ring.models, SpawnMode::Scenery);
    // A flagged reference whose model drew nothing keeps its cell's LOD segment.
    *keep |= flag && !drawn;
    Spawned::Done
}

/// Whether a reference is drawn in the ring for its enable state: enabled at the start, or
/// (with the game's simulation to show and hide it) not deleted.
pub fn drawn_for_enable_state(enabled: bool, deleted: bool, simulated: bool) -> bool {
    enabled || (simulated && !deleted)
}

/// A SpeedTree of a ring cell: its generated geometry (tree.rs), else its billboard, as
/// the loaded cells draw it; drawn only. Its geometry is built on this thread when the
/// loaded cells have not built it yet, [`TREE_BUILDS_PER_FRAME`] at most.
#[allow(clippy::too_many_arguments)]
fn spawn_ring_tree(commands: &mut Commands, ctx: &RefCtx, sp: Target, r: u32, p: &Placement, base: &BaseObject, rt: Affine3A, models: &mut ModelCtx, trees: &mut crate::tree::TreeCtx, ring: &mut FullDetailRing) -> Spawned {
    if !trees.built(base.form_id) {
        if ctx.tree_builds.get() >= TREE_BUILDS_PER_FRAME {
            return Spawned::Later;
        }
        ctx.tree_builds.set(ctx.tree_builds.get() + 1);
    }
    let tag = (nvf_play::RefVisual(r), nvf_play::DrawOnly);
    if let Some(v) = trees.visual(ctx.world, base.form_id, models) {
        ring.models.shapes += crate::tree::spawn(commands, sp.entity, &v, Mat4::from(rt), tag);
    } else if let Some((mesh, mat)) = world::billboard(ctx.world, base.form_id, models) {
        let at = Vec3::from_array(p.position) - sp.origin;
        let t = Transform::from_translation(at).with_scale(Vec3::splat(p.scale));
        commands.entity(sp.entity).with_child((Mesh3d(mesh), MeshMaterial3d(mat), t, world::TreeBillboard, tag));
        ring.models.shapes += 1;
    }
    ring.models.refs_speedtree += 1;
    ring.models.refs_drawn += 1;
    Spawned::Done
}

/// A ring cell is all in: shown (its meshes get their range in [`fade_ring`]).
fn finish(commands: &mut Commands, ring: &mut FullDetailRing, c: Grid, sp: RingSpawn, fading: bool) {
    commands.entity(sp.entity).insert(Visibility::Inherited);
    ring.cells.insert(c, RingCellState::new(sp.entity, if fading { 0.0 } else { 1.0 }, sp.keep_segment, sp.small));
    ring.stats.cells += 1;
}

/// Give each ring cell's meshes their range (its own fade, or the complement of the
/// loaded cell in its square, [`hand_off`]; with cell fades off, hidden under a loaded
/// cell) and their shadow rule. Runs after [`stream`], before `fade::fade_cells`.
#[allow(clippy::type_complexity)]
pub fn fade_ring(
    mut commands: Commands,
    streamer: Res<Streamer>,
    mut ring: ResMut<FullDetailRing>,
    sky: Res<crate::sky::SkyState>,
    children: Query<&Children>,
    meshes: Query<(), Fading>,
    casters: Query<(Has<NotShadowCaster>, Has<RingNoShadow>), With<Mesh3d>>,
) {
    if ring.cells.is_empty() {
        return;
    }
    let casts = ring_casts_shadows(sky.shadows, sky.shadow_distance * sky.shadow_reach, streamer.radius);
    let fading = streamer.fade_secs > 0.0;
    for (c, s) in &mut ring.cells {
        let want = ring_range(s.level, streamer.loaded.contains_key(c).then(|| streamer.cell_level(*c)));
        if s.applied != Some(want) {
            let was = s.applied.replace(want);
            if fading {
                let range = want.range();
                for e in children.iter_descendants(s.entity).filter(|&e| meshes.contains(e)) {
                    commands.entity(e).insert(range.clone());
                }
            } else if was.is_none_or(|w| w.hidden() != want.hidden()) {
                commands.entity(s.entity).insert(if want.hidden() { Visibility::Hidden } else { Visibility::Inherited });
            }
        }
        if s.shadow != Some(casts) {
            s.shadow = Some(casts);
            for e in children.iter_descendants(s.entity) {
                match casters.get(e) {
                    Ok((false, _)) if !casts => {
                        commands.entity(e).insert((NotShadowCaster, RingNoShadow));
                    }
                    Ok((_, true)) if casts => {
                        commands.entity(e).remove::<(NotShadowCaster, RingNoShadow)>();
                    }
                    _ => {}
                }
            }
        }
    }
}

/// The cells drawn in full detail, loaded or ring, for the distant LOD (lod.rs).
#[derive(SystemParam)]
pub struct DrawnCells<'w> {
    pub streamer: Res<'w, Streamer>,
    ring: Option<Res<'w, FullDetailRing>>,
}

impl DrawnCells<'_> {
    fn ring(&self) -> Option<&HashMap<Grid, RingCellState>> {
        self.ring.as_deref().map(|r| &r.cells)
    }

    /// Every drawn cell with whether it is fully drawn: the loaded ones (those not
    /// fading), then the ring's; for `blocks` (object LOD segments) without the ring cells
    /// that keep their segment.
    pub fn cells(&self, blocks: bool) -> Vec<(Grid, bool)> {
        drawn_cells(self.streamer.loaded.keys().map(|c| (*c, !self.streamer.fades.contains_key(c))), self.ring(), blocks)
    }

    /// The cells fully drawn: the LOD terrain is cut out there.
    pub fn full(&self) -> HashSet<Grid> {
        self.cells(false).into_iter().filter(|(_, full)| *full).map(|(c, _)| c).collect()
    }

    /// How much of cell `c` is drawn for its LOD segment's crossfade: the loaded cell's
    /// level or the ring cell's, whichever is more.
    pub fn block_level(&self, c: Grid) -> f32 {
        block_level(self.streamer.cell_level(c), self.ring(), c)
    }

    /// How much of cell `c`'s trees are drawn in full (loaded or ring), for the distant
    /// tree billboards' crossfade (treelod.rs): a kept LOD segment does not matter here.
    pub fn tree_level(&self, c: Grid) -> f32 {
        tree_level(self.streamer.cell_level(c), self.ring(), c)
    }
}

/// [`DrawnCells::tree_level`] from the loaded cell's level.
pub fn tree_level(loaded: f32, ring: Option<&HashMap<Grid, RingCellState>>, c: Grid) -> f32 {
    loaded.max(ring.and_then(|r| r.get(&c)).map_or(0.0, |s| s.level))
}

/// [`DrawnCells::cells`] from the loaded cells (cell, fully drawn) and the ring's.
pub fn drawn_cells(loaded: impl Iterator<Item = (Grid, bool)>, ring: Option<&HashMap<Grid, RingCellState>>, blocks: bool) -> Vec<(Grid, bool)> {
    let mut out: Vec<(Grid, bool)> = loaded.collect();
    out.extend(ring.into_iter().flatten().filter(|(_, s)| !(blocks && s.keep_segment)).map(|(c, s)| (*c, s.full())));
    out
}

/// [`DrawnCells::block_level`] from the loaded cell's level.
pub fn block_level(loaded: f32, ring: Option<&HashMap<Grid, RingCellState>>, c: Grid) -> f32 {
    let ring = ring.and_then(|r| r.get(&c)).filter(|s| !s.keep_segment).map_or(0.0, |s| s.level);
    loaded.max(ring)
}

/// The exit summary (module notes), only when the ring was on at some time.
pub fn report(mut exit: MessageReader<AppExit>, ring: Res<FullDetailRing>) {
    if exit.read().last().is_none() {
        return;
    }
    let Some(used) = ring.used else { return };
    let s = &ring.stats;
    info!(
        "full detail summary: {0}x{0}, {1} cells, {2} references drawn, {3} skipped by type, {4:.0} ms on the main thread, {5} too small for their distance",
        used.side(),
        s.cells,
        ring.models.refs_drawn,
        s.skipped_type + ring.models.refs_skipped_type,
        s.ms,
        s.too_small
    );
}

pub struct FullDetailPlugin;

impl Plugin for FullDetailPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<FullDetailRing>()
            .add_systems(Update, (stream.run_if(world::world_not_held), fade_ring).chain().after(world::stream).before(fade::fade_cells))
            .add_systems(Last, report);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(level: f32, leaving: bool, keep: bool) -> RingCellState {
        RingCellState { entity: Entity::PLACEHOLDER, level, leaving, keep_segment: keep, small: Vec::new(), applied: None, shadow: None }
    }

    /// The 9 x 9 ring around a loaded 5 x 5: the 56 cells 3 and 4 steps out, nearest
    /// first (the 24 at 3 steps, the edge's middles before its corners); 7 x 7: the 24.
    #[test]
    fn the_ring_is_the_cells_beyond_the_loaded_ones_nearest_first() {
        let centre = (-17, 2);
        let nine = ring_cells(centre, 2, 4);
        assert_eq!(nine.len(), 81 - 25);
        assert!(nine.iter().all(|&c| (3..=4).contains(&steps(c, centre))));
        assert!(nine[..24].iter().all(|&c| steps(c, centre) == 3), "the 3-step cells first");
        let keys: Vec<i32> = nine.iter().map(|&c| order_key(c, centre)).collect();
        assert!(keys.windows(2).all(|w| w[0] <= w[1]), "nearest first");
        assert_eq!(nine[0].0.abs_diff(centre.0) + nine[0].1.abs_diff(centre.1), 3, "a side's middle first");
        assert_eq!(ring_cells(centre, 2, 3).len(), 49 - 25);
        assert!(ring_cells(centre, 2, 2).is_empty(), "a ring no wider than the loaded cells is empty");
        assert_eq!(ring_cells(centre, 3, 4).len(), 81 - 49, "with --radius 3");
    }

    /// Ring cells stay one step beyond the ring (hysteresis), fade out past it, when the
    /// row is turned off or past a smaller new reach; they go at once once their loaded
    /// cell is fully drawn inside the loaded radius, and stand by under a loaded cell in
    /// the loaded grid's own hysteresis step.
    #[test]
    fn ring_cells_retire_with_hysteresis() {
        let (loaded, ring) = (2, 4);
        assert!(!fades_out(4, ring, false));
        assert!(!fades_out(5, ring, false), "one step of hysteresis");
        assert!(fades_out(6, ring, false));
        assert!(fades_out(3, 0, false) && fades_out(4, 0, true), "turned off: every cell fades out");
        assert!(fades_out(4, 3, true) && !fades_out(3, 3, true), "9 x 9 to 7 x 7: the 4-step cells fade out");
        assert!(!fades_out(4, 3, false), "afterwards the hysteresis holds again");
        assert!(!covered(3, loaded, true), "standing by under a loaded cell in its hysteresis");
        assert!(!covered(2, loaded, false), "kept until its loaded cell is fully drawn");
        assert!(covered(2, loaded, true));
        assert!(covered(0, loaded, true));
    }

    /// The size cut never goes stale: references it left out of an outermost cell are due
    /// once the cell is nearer, never while it is outermost.
    #[test]
    fn left_out_references_come_back_when_the_cell_is_nearer() {
        assert!(!small_due(4, 4, 12), "still outermost");
        assert!(small_due(3, 4, 12), "now in the inner ring step");
        assert!(!small_due(3, 4, 0), "nothing left out");
        assert!(small_due(3, 4, 1));
        assert!(!small_due(3, 3, 5) && small_due(2, 3, 5), "7 x 7: outermost is 3");
    }

    /// CPU mirror of Bevy's `get_visibility_range_dither_level` and `visibility_range_dither`
    /// (as fade.rs's tests): whether a pixel with dither `threshold` is drawn by a mesh with
    /// range `r` at distance `d`.
    fn kept(r: &VisibilityRange, d: f32, threshold: i32) -> bool {
        let (x, y, z, w) = (r.start_margin.start, r.start_margin.end, r.end_margin.start, r.end_margin.end);
        let (offset, a, b) = if d >= z { (0, z, w) } else { (-16, x, y) };
        let level = offset + (((d - a) / (b - a) * 16.0).round() as i32).clamp(0, 16);
        if level == 0 {
            return true;
        }
        if level <= -16 || level >= 16 {
            return false;
        }
        !((level >= 0 && level + threshold >= 16) || (level < 0 && 1 + level + threshold <= 0))
    }

    /// The three-way hand-off ([`hand_off`]): for a ring cell at any step, fading in or
    /// out, with or without a loaded cell in its square at any step, the LOD segment (at
    /// `block_level`, drawn with the complement of its step), the ring cell (`ring_range`)
    /// and the loaded cell (`fade::drawn` of its step) together draw every pixel once.
    #[test]
    fn lod_ring_and_loaded_cover_each_pixel_once() {
        let d = 20_000.0;
        let steps = fade::STEPS as i32;
        for kr in 0..=steps {
            for leaving in [false, true] {
                for loaded in std::iter::once(None).chain((0..=steps).map(Some)) {
                    let loaded_level = loaded.map(|k| k as f32 / steps as f32);
                    let ring_level = hand_off(kr as f32 / steps as f32, leaving, loaded.is_some());
                    let mut cells = HashMap::new();
                    if let Some(l) = ring_level {
                        cells.insert((3, 0), state(l, leaving, false));
                    }
                    let block = fade::complement(fade::step(block_level(loaded_level.unwrap_or(0.0), Some(&cells), (3, 0))));
                    let ring = ring_level.map(|l| ring_range(l, loaded_level).range());
                    let full = loaded.map(|k| fade::drawn(k as u8));
                    for t in 0..16 {
                        let n = [Some(block.clone()), ring.clone(), full.clone()].iter().flatten().filter(|r| kept(r, d, t)).count();
                        assert_eq!(n, 1, "ring step {kr} leaving {leaving}, loaded {loaded:?}: pixel {t} drawn {n} times");
                    }
                }
            }
        }
        // The rule itself: a resting ring cell under a loaded one is held at rest, a
        // leaving one is dropped, one alone keeps its level.
        assert_eq!(hand_off(0.3, false, true), Some(1.0));
        assert_eq!(hand_off(0.3, true, true), None);
        assert_eq!(hand_off(0.3, true, false), Some(0.3));
    }

    /// The type filter (module notes).
    #[test]
    fn the_ring_draws_statics_and_flagged_models_only() {
        let model = RefKind::Drawn { light: false, how: DrawnAs::Model };
        for sig in [b"STAT", b"SCOL", b"MSTT"] {
            assert_eq!(ring_choice(model, Some(sig), false), RingRef::Draw);
            assert_eq!(ring_choice(model, Some(sig), true), RingRef::Draw);
        }
        for sig in [b"DOOR", b"ACTI", b"CONT", b"FURN", b"MISC", b"WEAP", b"TERM", b"PROJ"] {
            assert_eq!(ring_choice(model, Some(sig), false), RingRef::SkipType, "{}", String::from_utf8_lossy(sig));
            assert_eq!(ring_choice(model, Some(sig), true), RingRef::Draw, "flagged {}", String::from_utf8_lossy(sig));
        }
        let light = RefKind::Drawn { light: true, how: DrawnAs::Model };
        assert_eq!(ring_choice(light, Some(b"LIGH"), false), RingRef::SkipType);
        assert_eq!(ring_choice(light, Some(b"LIGH"), true), RingRef::Draw, "a flagged light's model, no light");
        let tree = RefKind::Drawn { light: false, how: DrawnAs::Tree };
        assert_eq!(ring_choice(tree, Some(b"TREE"), false), RingRef::Draw, "stage 2: full SpeedTrees");
        assert_eq!(ring_choice(model, Some(b"MSTT"), false), RingRef::Draw, "stage 2: moveable statics with their idle");
        let water = RefKind::Drawn { light: false, how: DrawnAs::PlacedWater };
        assert_eq!(ring_choice(water, Some(b"PWAT"), false), RingRef::SkipType);
        assert_eq!(ring_choice(water, Some(b"PWAT"), true), RingRef::KeepSegment);
        assert_eq!(ring_choice(RefKind::Actor, Some(b"NPC_"), false), RingRef::SkipType);
        assert_eq!(ring_choice(RefKind::Actor, Some(b"CREA"), true), RingRef::KeepSegment);
        assert_eq!(ring_choice(RefKind::Imposter, Some(b"STAT"), true), RingRef::Skip);
        assert_eq!(ring_choice(RefKind::Primitive, Some(b"ACTI"), true), RingRef::SkipType);
        assert_eq!(ring_choice(RefKind::Skipped, Some(b"SOUN"), true), RingRef::SkipType);
        assert_eq!(ring_choice(RefKind::Disabled, Some(b"STAT"), true), RingRef::Skip);
    }

    /// Stage 2: references not enabled at the start are drawn while the game's simulation
    /// is there to hide and show them; deleted ones never; in the viewer only enabled ones.
    #[test]
    fn disabled_references_are_drawn_for_the_simulation() {
        assert!(drawn_for_enable_state(true, false, false));
        assert!(drawn_for_enable_state(false, false, true), "the simulation hides or shows it");
        assert!(!drawn_for_enable_state(false, true, true), "deleted");
        assert!(!drawn_for_enable_state(false, false, false), "the viewer: nothing enables it");
    }

    /// Stage 2: the distant tree billboards crossfade against ring cells' trees as against
    /// loaded cells' (a kept LOD segment does not matter for trees).
    #[test]
    fn tree_billboards_follow_the_ring() {
        let mut ring = HashMap::new();
        ring.insert((3, 0), state(0.25, false, true));
        assert_eq!(tree_level(0.0, Some(&ring), (3, 0)), 0.25);
        assert_eq!(block_level(0.0, Some(&ring), (3, 0)), 0.0, "the kept segment");
        assert_eq!(tree_level(0.5, Some(&ring), (3, 0)), 0.5);
        assert_eq!(tree_level(0.0, None, (3, 0)), 0.0, "without the ring: the loaded level as before");
    }

    /// classify_ref with the ring's filter, on placements built by hand: imposters,
    /// primitives and disabled references come first, whatever their base.
    #[test]
    fn the_filter_follows_the_loaded_cells_classification() {
        let p = |flags: u32, imposter: bool, primitive: bool| Placement {
            form_id: 0x100, sig: *b"REFR", flags, base: Some(0x200), position: [0.0; 3], rotation: [0.0; 3], scale: 1.0,
            enable_parent: None, open_by_default: false, teleport: false, emittance: None, imposter, primitive,
        };
        let base = |sig: [u8; 4], model: &str| BaseObject { form_id: 0x200, sig, edid: None, model: Some(model.into()), world_model: None, world_alt_textures: vec![], worn_only: false };
        let vwd = ref_flags::VISIBLE_WHEN_DISTANT;
        assert!(ring_models(&p(0, false, false), &base(*b"STAT", "rocks\\rock01.nif")));
        assert!(!ring_models(&p(0, false, false), &base(*b"DOOR", "doors\\door01.nif")));
        assert!(ring_models(&p(vwd, false, false), &base(*b"DOOR", "doors\\door01.nif")));
        assert!(!ring_models(&p(vwd, true, false), &base(*b"STAT", "rocks\\rock01.nif")), "an imposter");
        assert!(!ring_models(&p(vwd, false, true), &base(*b"ACTI", "pipes\\pipe.nif")), "a primitive");
        assert!(!ring_models(&p(vwd, false, false), &base(*b"TREE", "trees\\joshua01.spt")), "a SpeedTree: built by tree.rs, not read ahead");
        assert!(ring_models(&p(0, false, false), &base(*b"MSTT", "effects\\smoke.nif")));
        assert!(!ring_models(&p(vwd, false, false), &base(*b"NPC_", "")), "an actor");
        assert_eq!(classify_ref(&p(vwd, false, false), Some(&base(*b"STAT", "a.nif")), || false), RefKind::Disabled);
        assert!(flagged(&p(vwd | 0x400, false, false)) && !flagged(&p(0x400, false, false)));
    }

    /// The hand-off: under a loaded cell at any fade step the ring cell draws exactly its
    /// complement (fade.rs: the two never share a pixel), nothing once it is fully drawn;
    /// alone, its own step.
    #[test]
    fn under_a_loaded_cell_the_ring_draws_the_complement_at_every_step() {
        for k in 0..=fade::STEPS {
            let level = k as f32 / fade::STEPS as f32;
            let r = ring_range(1.0, Some(level));
            assert_eq!(r, RingRange::Under(k));
            assert!(r.range() == fade::complement(k), "step {k}");
            assert_eq!(r.hidden(), k == fade::STEPS);
            assert!(r.range().is_visible_at_all(5000.0) == (k < fade::STEPS), "culled only once the loaded cell is fully drawn");
            let own = ring_range(level, None);
            assert_eq!(own, RingRange::Own(k));
            assert!(own.range() == fade::drawn(k));
            assert_eq!(own.hidden(), k == 0);
        }
        // The ring's own level does not matter under a loaded cell.
        assert_eq!(ring_range(0.3, Some(0.5)), ring_range(1.0, Some(0.5)));
    }

    /// The LOD sees ring cells as loaded ones: present, fully drawn once at rest, their
    /// level the larger of the two; a cell keeping its segment counts for the terrain only.
    #[test]
    fn drawn_cells_join_the_loaded_and_the_ring() {
        let loaded = [((0, 0), true), ((1, 0), false)];
        assert_eq!(drawn_cells(loaded.into_iter(), None, true), loaded.to_vec(), "without the ring: the loaded cells as before");
        let mut ring = HashMap::new();
        ring.insert((3, 0), state(1.0, false, false));
        ring.insert((4, 0), state(0.5, false, false));
        ring.insert((3, 1), state(1.0, true, false));
        ring.insert((-3, 0), state(1.0, false, true));
        let mut terrain = drawn_cells(loaded.into_iter(), Some(&ring), false);
        terrain.sort();
        assert_eq!(terrain, vec![((-3, 0), true), ((0, 0), true), ((1, 0), false), ((3, 0), true), ((3, 1), false), ((4, 0), false)]);
        let mut blocks = drawn_cells(loaded.into_iter(), Some(&ring), true);
        blocks.sort();
        assert!(!blocks.iter().any(|(c, _)| *c == (-3, 0)), "keeps its segment");
        assert_eq!(blocks.len(), 5);
        assert_eq!(block_level(0.0, Some(&ring), (4, 0)), 0.5);
        assert_eq!(block_level(0.25, Some(&ring), (3, 0)), 1.0, "the ring cell under a loaded one fading in");
        assert_eq!(block_level(0.0, Some(&ring), (-3, 0)), 0.0, "its segment stays");
        assert_eq!(block_level(0.75, None, (9, 9)), 0.75);
        // The masks lod.rs builds from them: bit y * 4 + x of each level-4 square.
        let masks = crate::lod::block_masks(&blocks, (0, 0));
        // Square (0, 0): loaded (0, 0) full (bit 0), loaded (1, 0) fading (bit 1), ring
        // (3, 0) at rest (bit 3), ring (3, 1) leaving (bit 1 * 4 + 3 = 7).
        assert_eq!(masks[&(0, 0)], (0b1001, 0b1000_1011));
        assert_eq!(masks[&(0, 0)], (9, 139));
        assert_eq!(masks[&(4, 0)], (0, 1), "ring (4, 0) fading in");
        assert!(!masks.contains_key(&(-4, 0)));
    }

    #[test]
    fn shadows_reach_the_ring_only_past_its_inner_edge() {
        assert!(!ring_casts_shadows(true, 8000.0, 2), "the default shadow distance ends inside");
        assert!(ring_casts_shadows(true, 12_000.0, 2), "longer shadows reach it");
        assert!(!ring_casts_shadows(false, 12_000.0, 2));
    }

    /// Off in both looks; `--graphics-only full-detail-7x7` / `full-detail-9x9` turn it on.
    #[test]
    fn off_in_both_presets_and_the_keys_parse() {
        use crate::graphics::Preset;
        assert_eq!(Graphics::preset(Preset::Vanilla).full_detail, FullDetail::Off);
        assert_eq!(Graphics::preset(Preset::Enhanced).full_detail, FullDetail::Off);
        assert_eq!(Graphics::only(&["full-detail-7x7".into()]).unwrap().full_detail, FullDetail::Seven);
        let nine = Graphics::only(&["full-detail-9x9".into()]).unwrap();
        assert_eq!(nine.full_detail, FullDetail::Nine);
        assert_eq!(nine.describe(), "light glow, full-detail scenery 9 x 9");
        assert_eq!(Graphics::preset(Preset::Vanilla).describe(), "light glow", "nothing said while off");
        assert!(Graphics::only(&["full-detail-11x11".into()]).is_err());
    }

    #[test]
    fn the_choices_name_their_sizes() {
        assert_eq!(FullDetail::default(), FullDetail::Off);
        assert_eq!(FullDetail::CHOICES.map(|(_, f)| (f.radius(), f.side())), [(0, 1), (3, 7), (4, 9)]);
        assert_eq!(FullDetail::Nine.name(), "9 x 9");
        assert_eq!(FullDetail::Off.describe(), "");
    }

    /// A ring cell is no loaded cell to anything that draws into or reads loaded cells
    /// (nvf-play's dropped and script-drawn references, moved references, the terrain
    /// wireframe, decals, the mesh terrain blend): no `RenderCell`, no `CellEntity`.
    #[test]
    fn a_ring_cell_is_no_render_or_loaded_cell() {
        let mut world = World::new();
        world.spawn((Name::new("Ring cell 3,0"), RingCell { grid: (3, 0) }, Transform::default(), Visibility::Hidden));
        assert_eq!(world.query::<&nvf_play::RenderCell>().iter(&world).count(), 0);
        assert_eq!(world.query_filtered::<(), With<world::CellEntity>>().iter(&world).count(), 0);
        world.spawn((RingCell { grid: (4, 0) }, nvf_play::DrawOnlyCell));
        assert_eq!(world.query_filtered::<(), With<nvf_play::DrawOnlyCell>>().iter(&world).count(), 1, "nvf-play's moved.rs finds ring cells by this mark");
        assert_eq!(world.query::<&RingCell>().iter(&world).count(), 2);
    }
}
