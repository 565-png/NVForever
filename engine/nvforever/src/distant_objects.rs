//! Distant objects (#3014; docs/design/graphics-enhanced.md §3.18): with the settings
//! page's Distant objects row at 8 or 12 cells, static scenery the distant LOD blocks do
//! not hold is drawn beyond the loaded cells (and beyond the full-detail ring,
//! full_detail.rs), out to that many grid steps from the camera's cell: from the
//! reference's own `<model>_lod.nif` when the load order has one beside its full model,
//! else from its full model when it is large enough for its distance. Off by default and
//! in both presets. Everything here is NVForever's own design, not the original's
//! behaviour.
//!
//! - **The original's rule** [documented: GECK wiki "Static", "World LOD"]: a base flagged
//!   "Visible When Distant" (record header flag 0x8000) has its references baked into the
//!   generated object LOD blocks when a `<model>_lod.nif` exists in the same folder. The
//!   blocks are made for one load order, so plugins and LOD packs added later change
//!   nothing until the blocks are made again.
//! - **What is drawn** ([`choose`]): exterior STAT and SCOL references that pass the
//!   ring's scenery filter (not deleted, not an imposter, primitive or marker, not hidden,
//!   not drawn with a destruction model). Conservative first rule, free of doubling while
//!   the blocks match the load order: a reference flagged visible when distant (its own
//!   flag or its base's, in any plugin's version: `fnv_world::placed::visible_when_distant`)
//!   is left to the block while its cell's segment is not empty or not known yet
//!   (`lod::BlockContents`). Every other reference is drawn from its LOD model when one
//!   exists ([`lod_model`]: `<model>_lod.nif`, else `<model>_lod_0.nif`), else from its
//!   full model when its scaled bound radius (OBND) is at least [`SIZE_RATIO`] of its
//!   distance from the centre of the loaded grid's middle cell, else not at all.
//! - **Triangle budget** ([`fit_budget`], [`Budget`]): the full models drawn may sum to at
//!   most [`FULL_TRIANGLE_BUDGET`] triangles; past it the size ratio is raised for the
//!   farthest cells first (by [`RATIO_RAISE`] up to [`RATIO_MAX`]), and a full model that
//!   would still pass the cap is left out ([`admits`]). Logged when it bites. The sum is
//!   never kept as a running count: it is read from the cells alive (their retained
//!   models, the reservations of cells being spawned, the predecessors still fading out),
//!   so a cell dropped half-spawned or retired gives its share back by leaving.
//! - **Draw-only**, as the ring's cells: a distant cell's entity carries [`DistantCell`]
//!   and `nvf_play::DrawOnlyCell`, never `world::CellEntity` or `nvf_play::RenderCell`; its
//!   models keep `RefVisual` (the simulation's enable state, `activate::sync_visuals` on
//!   every `RefChanged`, and moved references reach them) and carry `nvf_play::DrawOnly`
//!   (`world::SpawnMode::Scenery`). No collider, light, emitter or script; no sun shadow
//!   (`NotShadowCaster` on every mesh). The draw-distance sliders leave them alone
//!   (`draw_distance::DrawClass::Distant`).
//! - **Streaming** ([`stream`]): the cells [`distant_cells`] names, from one step beyond
//!   max(loaded radius, ring radius) out to the reach, are listed on the worker threads
//!   (never on this thread: a cell's placements are read there) and the lists kept within
//!   [`KEEP_LISTS`] steps beyond the reach. Lists start only while the loaded cells' and
//!   the ring's read-ahead are idle; cells are spawned nearest first within
//!   [`SPAWN_BUDGET`] a frame, after the ring, only while the loaded grid and the ring are
//!   complete, and only with models already prepared on the workers (a cell waits for
//!   its models; the next [`PREFETCH_CELLS`] cells' models are asked for meanwhile). A
//!   travel, another worldspace, going indoors or turning the distant LOD off drops
//!   everything at once.
//! - **Fades** ([`distant_range`], [`fade_distant`]): a distant cell in a square drawn by
//!   a loaded or ring cell draws the complement of that cell's fade step (the same
//!   references in full) and is held at rest as the ring's cells are (`full_detail::hand_off`),
//!   so each pixel is drawn once; alone it fades in and out by its own step, and at rest
//!   dithers out with distance between `reach - 1` and `reach` cells from the camera.
//!   Cells leave by fading out past `reach + 1` (or out of a smaller new reach), or once
//!   the loaded or ring cell over them is fully drawn.
//! - **Re-judging**: when the loaded grid's centre has moved [`REJUDGE_STEPS`] cells, or a
//!   block becomes known with an empty segment, a cell whose drawn set changes is built
//!   again and crossfaded with the old one (the new one's step against the old one's
//!   complement). A full model already drawn stays while its radius is at least
//!   [`HYSTERESIS`] of the threshold, so nothing flickers at it.
//! - **Logs** (only while on): `distant objects: reach N cells (ratio R)` when it comes on;
//!   at the exit `distant objects summary: <cells> cells, <drawn> drawn (<lod> lod, <full>
//!   full), <held> held by blocks, <unknown> block unknown, <small> too small, <t> ms on
//!   the main thread`.

use crate::emittance::EmittanceState;
use crate::fade::{self, Fading};
use crate::full_detail::{self, covered, fades_out, hand_off, steps, FullDetailRing, Grid};
use crate::graphics::Graphics;
use crate::lod::{BlockContents, LodState};
use crate::world::{self, SpawnMode, StreamStats, Streamer, WorldData, WorldRoot};
use bevy::camera::visibility::VisibilityRange;
use bevy::light::NotShadowCaster;
use bevy::math::Affine3A;
use bevy::prelude::*;
use bevy::tasks::{futures::check_ready, Task};
use fnv_esm::{LoadOrder, Schema as EsmSchema};
use fnv_world::alt_textures::AltTexture;
use fnv_world::placed::{base_object, placement, placement_visible_when_distant, BaseObject, Placement};
use fnv_world::CELL_SIZE;
use nvf_assets::model::ModelCtx;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// The settings page's Distant objects row (`Graphics::distant_objects`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DistantObjects {
    /// Past the loaded cells only the LOD blocks (as the original).
    #[default]
    Off,
    /// Out to [`REACH_NEAR`] grid steps.
    Eight,
    /// Out to [`REACH_FAR`] grid steps.
    Twelve,
}

/// The nearer reach (grid steps from the camera's cell) [design].
pub const REACH_NEAR: i32 = 8;
/// The farther reach [design].
pub const REACH_FAR: i32 = 12;

impl DistantObjects {
    /// The row's choices in its order, by the names it shows and saves.
    pub const CHOICES: [(&'static str, DistantObjects); 3] = [("Off", DistantObjects::Off), ("8 cells", DistantObjects::Eight), ("12 cells", DistantObjects::Twelve)];

    /// Grid steps from the camera's cell the distant objects reach (0: off).
    pub fn reach(self) -> i32 {
        match self {
            DistantObjects::Off => 0,
            DistantObjects::Eight => REACH_NEAR,
            DistantObjects::Twelve => REACH_FAR,
        }
    }

    /// The row's name of the choice.
    pub fn name(self) -> &'static str {
        Self::CHOICES.iter().find(|(_, d)| *d == self).map_or("Off", |(n, _)| n)
    }

    /// For `Graphics::describe` (empty while off).
    pub fn describe(self) -> &'static str {
        match self {
            DistantObjects::Off => "",
            DistantObjects::Eight => "distant objects 8 cells",
            DistantObjects::Twelve => "distant objects 12 cells",
        }
    }
}

/// A reference without a LOD model is drawn from its full model when its scaled bound
/// radius is at least this part of its distance: 328 units at 8 cells, 492 at 12
/// [engineering choice; the angular rule of `shadow_thin.rs`].
pub const SIZE_RATIO: f32 = 0.01;

/// A full model already drawn stays while its radius is at least this part of the
/// threshold ([`size_passes`]): the hysteresis that keeps a model on the edge of the rule
/// from coming and going as the centre moves [design].
pub const HYSTERESIS: f32 = 0.8;

/// The size rule is judged again when the loaded grid's centre has moved this many grid
/// steps since the last judgement [design].
pub const REJUDGE_STEPS: i32 = 2;

/// The most triangles the full models drawn may sum to ([`fit_budget`]) [engineering
/// choice: the study counted 1,900-2,700 qualifying references within 12 cells at
/// Goodsprings, Primm and Novac, against a median full model of 4,968 triangles].
pub const FULL_TRIANGLE_BUDGET: u64 = 4_000_000;

/// The triangles counted for a full model not prepared yet (the median full model with a
/// LOD sibling in the base game, measured from the data) [engineering choice].
pub const UNKNOWN_TRIANGLES: u64 = 5_000;

/// Each raise of a cell ring's size ratio when the budget is passed, and the highest
/// ratio it is raised to [design].
pub const RATIO_RAISE: f32 = 1.5;
pub const RATIO_MAX: f32 = 0.08;

/// Main-thread time a frame spends spawning distant cells, the bookkeeping included
/// [engineering choice: two thirds of the ring's; never waited for].
pub const SPAWN_BUDGET: Duration = Duration::from_millis(1);

/// Cells after a waiting one whose models are asked of the worker threads meanwhile.
pub const PREFETCH_CELLS: usize = 4;

/// Cell lists kept this many steps beyond the reach (a step back and forth reads none
/// again).
pub const KEEP_LISTS: i32 = 2;

/// Cell lists read on the worker threads at once (the loaded cells' read-ahead shares the
/// pool) [engineering choice].
pub const MAX_LISTING: usize = 8;

/// On a distant cell's entity: the grid square it draws (module notes).
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct DistantCell {
    pub grid: Grid,
}

/// The LOD model of a full model (`meshes\...\name.nif`): `name_lod.nif` beside it when
/// `exists` says so, else `name_lod_0.nif` (one community pack's naming), else none.
pub fn lod_model(full: &str, exists: impl Fn(&str) -> bool) -> Option<String> {
    let stem = full.strip_suffix(".nif").or_else(|| full.strip_suffix(".NIF"))?;
    if stem.to_ascii_lowercase().ends_with("_lod") {
        return None;
    }
    [format!("{stem}_lod.nif"), format!("{stem}_lod_0.nif")].into_iter().find(|p| exists(p))
}

/// One reference of a cell list, as a worker thread found it.
#[derive(Debug, Clone)]
pub struct Entry {
    pub p: Placement,
    pub base: Arc<BaseObject>,
    /// The full model's archive path.
    pub full: String,
    /// The LOD model's, when the load order has one ([`lod_model`]).
    pub lod: Option<String>,
    /// Visible when distant in any version of the reference or its base.
    pub flagged: bool,
    /// The bound radius (OBND) times the reference's scale; 0 without bounds.
    pub radius: f32,
    /// Enabled at the start of a game.
    pub enabled: bool,
}

/// A cell's references that may be drawn, read on a worker thread.
#[derive(Debug, Default)]
pub struct CellList {
    pub entries: Vec<Entry>,
    /// References left out on the worker: no LOD model and too small for the nearest
    /// distance a distant reference can be at (`inner`).
    pub small: usize,
    /// The inner radius (grid steps) the list was read for.
    pub inner: i32,
}

/// The nearest a reference of a distant cell can be from the loaded grid's middle cell's
/// centre with `inner` the radius it starts beyond (half a cell short of `inner + 1`).
pub fn nearest(inner: i32) -> f32 {
    (inner as f32 + 0.5) * CELL_SIZE
}

/// Read cell list: the STAT and SCOL references of `refs` that pass the scenery filter
/// (module notes), each with what [`choose`] needs. Runs on a worker thread.
pub fn list_cell(lo: &LoadOrder, schema: &EsmSchema, refs: &[u32], hide: &[String], inner: i32, exists: impl Fn(&str) -> bool) -> CellList {
    let mut out = CellList { inner, ..default() };
    let mut bases: HashMap<u32, Option<(Arc<BaseObject>, String, Option<String>)>> = HashMap::new();
    let small = SIZE_RATIO * HYSTERESIS * nearest(inner);
    for &r in refs {
        let Some(p) = placement(lo, schema, r) else { continue };
        if p.deleted() || p.imposter || !p.draws_base_model() {
            continue;
        }
        let Some(b) = p.base else { continue };
        let found = bases.entry(b).or_insert_with(|| {
            let base = base_object(lo, schema, b)?;
            if !matches!(&base.sig, b"STAT" | b"SCOL") || base.is_speedtree() {
                return None;
            }
            let full = base.world_model_path()?;
            if world::is_drawn_marker(base.form_id, &full) || hide.iter().any(|h| full.contains(h.as_str())) {
                return None;
            }
            let lod = lod_model(&full, &exists);
            Some((Arc::new(base), full, lod))
        });
        let Some((base, full, lod)) = found.clone() else { continue };
        let flagged = placement_visible_when_distant(lo, &p);
        let radius = base.bound_radius().unwrap_or(0.0) * p.scale;
        if !flagged && lod.is_none() && radius < small {
            out.small += 1;
            continue;
        }
        let enabled = world::enabled(lo, schema, &p, 0);
        out.entries.push(Entry { p, base, full, lod, flagged, radius, enabled });
    }
    out
}

/// What [`choose`] made of a reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Choice {
    /// Drawn from its LOD model.
    Lod,
    /// Drawn from its full model.
    Full,
    /// Left out: its cell's block holds it.
    Held,
    /// Left out: flagged, and its cell's block is not known yet.
    Unknown,
    /// Left out: no LOD model and too small for its distance (or past the budget).
    Small,
}

impl Choice {
    pub fn drawn(self) -> bool {
        matches!(self, Choice::Lod | Choice::Full)
    }
}

/// What [`choose`] reads of a reference.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Judge {
    pub flagged: bool,
    pub has_lod: bool,
    pub radius: f32,
}

/// The size rule: `radius` at least `ratio` of `distance`, or [`HYSTERESIS`] of that for
/// a model already drawn full; never without a radius.
pub fn size_passes(radius: f32, distance: f32, ratio: f32, was_full: bool) -> bool {
    let need = ratio * distance * if was_full { HYSTERESIS } else { 1.0 };
    radius > 0.0 && radius >= need
}

/// The rule (module notes): `segment` is whether the block of the reference's cell has a
/// non-empty segment for it (`None`: not known yet); `distance` from the loaded grid's
/// middle cell's centre; `was_full`: drawn full before ([`HYSTERESIS`]).
pub fn choose(j: Judge, segment: Option<bool>, distance: f32, ratio: f32, was_full: bool) -> Choice {
    if j.flagged {
        match segment {
            None => return Choice::Unknown,
            Some(true) => return Choice::Held,
            Some(false) => {}
        }
    }
    if j.has_lod {
        Choice::Lod
    } else if size_passes(j.radius, distance, ratio, was_full) {
        Choice::Full
    } else {
        Choice::Small
    }
}

/// A full-model candidate for the budget: its cell's steps from the centre, its radius
/// over its distance (the highest ratio it passes) and its triangles.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Candidate {
    pub steps: i32,
    pub size: f32,
    pub triangles: u64,
}

/// The size ratio of each cell ring (index: grid steps from the centre, up to `reach`)
/// that keeps the candidates' full models within `cap` triangles: `base` everywhere, then
/// raised by [`RATIO_RAISE`] (up to [`RATIO_MAX`]) on the farthest ring that still draws
/// a full model and is below the most, again and again, until the sum fits or no ring can
/// be raised. Also whether any ratio was raised.
pub fn fit_budget(candidates: &[Candidate], reach: i32, base: f32, cap: u64) -> (Vec<f32>, bool) {
    let n = reach.max(0) as usize + 1;
    let mut ratios = vec![base; n];
    // Per ring, sizes sorted from largest, with the running sum of triangles.
    let mut rings: Vec<Vec<(f32, u64)>> = vec![Vec::new(); n];
    for c in candidates.iter().filter(|c| c.steps >= 0 && (c.steps as usize) < n) {
        rings[c.steps as usize].push((c.size, c.triangles));
    }
    for r in &mut rings {
        r.sort_by(|a, b| b.0.total_cmp(&a.0));
        let mut sum = 0;
        for e in r.iter_mut() {
            sum += e.1;
            e.1 = sum;
        }
    }
    let at = |ring: &Vec<(f32, u64)>, ratio: f32| -> u64 {
        let k = ring.partition_point(|e| e.0 >= ratio);
        if k == 0 { 0 } else { ring[k - 1].1 }
    };
    let total = |ratios: &[f32]| -> u64 { rings.iter().zip(ratios).map(|(r, &q)| at(r, q)).sum() };
    let mut raised = false;
    while total(&ratios) > cap {
        let Some(i) = (0..n).rev().find(|&i| ratios[i] < RATIO_MAX && at(&rings[i], ratios[i]) > 0) else { break };
        ratios[i] = (ratios[i] * RATIO_RAISE).min(RATIO_MAX);
        raised = true;
    }
    (ratios, raised)
}

/// The size ratio for a cell `s` steps out ([`fit_budget`]'s table, else [`SIZE_RATIO`]).
fn ratio_at(ratios: &[f32], s: i32) -> f32 {
    usize::try_from(s).ok().and_then(|i| ratios.get(i)).copied().unwrap_or(SIZE_RATIO)
}

/// Nearest first, as the ring orders its cells.
fn order_key(c: Grid, centre: Grid) -> i32 {
    steps(c, centre) * 1000 + (c.0 - centre.0).abs() + (c.1 - centre.1).abs()
}

/// The centre of the loaded grid's middle cell (game x, y): distances are taken from it.
pub fn centre_point(centre: Grid) -> Vec2 {
    Vec2::new((centre.0 as f32 + 0.5) * CELL_SIZE, (centre.1 as f32 + 0.5) * CELL_SIZE)
}

/// The distant cells around `centre`: `inner + 1` to `reach` steps out whose nearest point
/// lies within `reach` cell widths of the middle cell's centre (the rest are past the
/// distance fade, [`edge_range`]), nearest first.
pub fn distant_cells(centre: Grid, inner: i32, reach: i32) -> Vec<Grid> {
    let p = centre_point(centre);
    let mut out: Vec<Grid> = (-reach..=reach)
        .flat_map(|dx| (-reach..=reach).map(move |dy| (centre.0 + dx, centre.1 + dy)))
        .filter(|&c| steps(c, centre) > inner && crate::lod::square_distance(p, c, 1) < reach as f32 * CELL_SIZE)
        .collect();
    out.sort_by_key(|&c| order_key(c, centre));
    out
}

/// Which range a distant cell's meshes are drawn with ([`distant_range`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DistantRange {
    /// Its own fade's step (`fade::drawn`).
    Own(u8),
    /// Under a loaded or ring cell of the same square: the complement of that cell's step.
    Under(u8),
    /// At rest and alone: fully drawn, dithering out with distance ([`edge_range`]).
    Edge,
}

impl DistantRange {
    pub fn range(self, reach: i32) -> VisibilityRange {
        match self {
            DistantRange::Own(k) => fade::drawn(k),
            DistantRange::Under(k) => fade::complement(k),
            DistantRange::Edge => edge_range(reach),
        }
    }

    /// The step of the cell's own pixels, for the old cell a rebuilt one replaces (drawn as
    /// its complement): under a cover or at rest, all of them.
    fn own_step(self) -> u8 {
        match self {
            DistantRange::Own(k) => k,
            DistantRange::Under(_) | DistantRange::Edge => fade::STEPS,
        }
    }
}

/// The range for a distant cell at `own` (0 … 1) whose square has a loaded or ring cell
/// drawn at `cover` (the larger of their levels), if any.
pub fn distant_range(own: f32, cover: Option<f32>) -> DistantRange {
    match cover {
        Some(l) => DistantRange::Under(fade::step(l)),
        None => match fade::step(own) {
            k if k >= fade::STEPS => DistantRange::Edge,
            k => DistantRange::Own(k),
        },
    }
}

/// Fully drawn within `reach - 1` cell widths of the camera, dithering out to nothing at
/// `reach`.
pub fn edge_range(reach: i32) -> VisibilityRange {
    let inner = (reach - 1).max(1) as f32;
    fade::drawn_within(Some(inner * CELL_SIZE), 1.0 / inner)
}

/// One drawn distant cell.
#[derive(Debug)]
pub struct CellState {
    pub entity: Entity,
    /// 0 hidden … 1 fully drawn (its own fade).
    pub level: f32,
    /// Fading out, to be retired at 0.
    pub leaving: bool,
    /// What it drew of each reference.
    plan: HashMap<u32, Choice>,
    /// The loaded grid's centre its plan was judged for, and its block's answer then.
    judged: Grid,
    segment: Option<bool>,
    /// It holds flagged references (a block becoming known may change it).
    flagged: bool,
    /// Triangles of its full models.
    full_tris: u64,
    /// The cell it replaces, drawn as the complement of this one's step until it rests,
    /// and the triangles of that cell's full models (still drawn, still counted).
    old: Option<(Entity, u64)>,
    applied: Option<(DistantRange, i32)>,
    old_applied: Option<u8>,
}

/// A distant cell whose references are being spawned (hidden until all are in).
struct CellSpawn {
    entity: Entity,
    grid: Grid,
    /// It replaces the drawn cell of its square (a re-judgement).
    replaces: bool,
    list: Arc<CellList>,
    plan: Vec<(usize, Choice)>,
    next: usize,
    /// References whose model was released since the cell's models were found prepared.
    later: Vec<(usize, Choice)>,
    origin: Vec3,
    judged: Grid,
    segment: Option<bool>,
    /// What was drawn of each reference so far.
    drawn: HashMap<u32, Choice>,
    full_tris: u64,
}

impl CellSpawn {
    fn done(&self) -> bool {
        self.next >= self.plan.len() && self.later.is_empty()
    }
}

/// What the distant objects did over the run, for the exit summary.
#[derive(Default, Debug, Clone, Copy)]
pub struct DistantStats {
    pub cells: usize,
    pub lod: usize,
    pub full: usize,
    pub held: usize,
    pub unknown: usize,
    pub small: usize,
    pub ms: f64,
}

/// The distant cells (module notes).
#[derive(Resource, Default)]
pub struct DistantState {
    /// Grid steps reached (0: off).
    pub reach: i32,
    world: u32,
    travels: u64,
    pub cells: HashMap<Grid, CellState>,
    spawning: HashMap<Grid, CellSpawn>,
    /// Drawn cells to build again (re-judged).
    rebuild: HashSet<Grid>,
    dying: Vec<Entity>,
    listing: HashMap<Grid, Task<CellList>>,
    listed: HashMap<Grid, Arc<CellList>>,
    /// Lists came in since the budget was last fitted.
    lists_changed: bool,
    /// The centre the size rule was last judged for, and its ratios per ring.
    judged: Option<Grid>,
    ratios: Vec<f32>,
    /// The centre the budget's raise was last logged for.
    budget_logged: Option<Grid>,
    /// Triangles of each full model drawn so far, by path.
    tris_of: HashMap<String, u64>,
    /// `world::spawn_model`'s counts for the distant models.
    models: StreamStats,
    pub stats: DistantStats,
    /// The last reach that was on (the exit summary is logged only then).
    used: Option<i32>,
}

impl DistantState {
    fn idle(&self) -> bool {
        self.reach == 0 && self.cells.is_empty() && self.spawning.is_empty() && self.dying.is_empty() && self.listing.is_empty() && self.listed.is_empty()
    }

    /// Drop everything now: a travel, another worldspace, an interior, the LOD off.
    fn clear(&mut self, commands: &mut Commands) {
        let olds: Vec<Entity> = self.cells.values().filter_map(|s| s.old.map(|o| o.0)).collect();
        for e in self.cells.drain().map(|(_, s)| s.entity).chain(self.spawning.drain().map(|(_, s)| s.entity)).chain(self.dying.drain(..)).chain(olds) {
            commands.entity(e).try_despawn();
        }
        self.rebuild.clear();
        self.listing.clear();
        self.listed.clear();
        self.judged = None;
    }

    /// The triangle budget's parts, read from the cells alive now ([`Budget`]).
    pub fn budget(&self) -> Budget {
        Budget {
            retained: self.cells.values().map(|s| s.full_tris).sum(),
            reserved: self.spawning.values().map(|s| s.full_tris).sum(),
            fading: self.cells.values().filter_map(|s| s.old.map(|o| o.1)).sum(),
        }
    }

    /// Hide a cell's entity and despawn it over the next frames.
    fn retire(&mut self, commands: &mut Commands, e: Entity) {
        commands.entity(e).insert(Visibility::Hidden);
        self.dying.push(e);
    }

    /// Retire a drawn cell with the one it replaces.
    fn retire_cell(&mut self, commands: &mut Commands, s: CellState) {
        if let Some((o, _)) = s.old {
            self.retire(commands, o);
        }
        self.retire(commands, s.entity);
    }
}

/// The full models' triangles, in three parts (module notes): those of the drawn cells
/// (`retained`), those cells being spawned have drawn so far (`reserved`) and those of
/// the cells a rebuilt cell replaces while they fade out (`fading`). Each part is read
/// from the cells alive, never counted up and down.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Budget {
    pub retained: u64,
    pub reserved: u64,
    pub fading: u64,
}

impl Budget {
    pub fn total(self) -> u64 {
        self.retained + self.reserved + self.fading
    }
}

/// Whether a full model of `tris` triangles may be drawn by a spawn within `cap`: against
/// everything drawn, reserved and fading, except, for a cell rebuilt in place, the
/// `predecessor`'s retained models (they go once it rests; counting them would drop
/// models the new cell keeps). While the two crossfade the total can pass the cap by at
/// most that one cell's models; every other spawn meanwhile counts both [design].
pub fn admits(b: Budget, predecessor: u64, tris: u64, cap: u64) -> bool {
    b.retained.saturating_sub(predecessor) + b.reserved + b.fading + tris <= cap
}

/// The plan of cell `c` from its list: each entry's [`Choice`] (module notes).
fn plan_cell(list: &CellList, c: Grid, centre: Grid, ratios: &[f32], segment: Option<bool>, before: Option<&HashMap<u32, Choice>>) -> Vec<(usize, Choice)> {
    let p = centre_point(centre);
    let ratio = ratio_at(ratios, steps(c, centre));
    list.entries
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let d = Vec2::new(e.p.position[0], e.p.position[1]).distance(p);
            let was_full = before.and_then(|b| b.get(&e.p.form_id)) == Some(&Choice::Full);
            (i, choose(Judge { flagged: e.flagged, has_lod: e.lod.is_some(), radius: e.radius }, segment, d, ratio, was_full))
        })
        .collect()
}

/// The drawn part of a plan (reference, choice), to tell whether a cell must be built
/// again.
fn drawn_set(list: &CellList, plan: &[(usize, Choice)]) -> BTreeMap<u32, Choice> {
    plan.iter().filter(|(_, ch)| ch.drawn()).map(|&(i, ch)| (list.entries[i].p.form_id, ch)).collect()
}

/// The model (path, alternate textures) a planned reference draws.
fn model_of(e: &Entry, ch: Choice) -> Option<(&str, &[AltTexture])> {
    match ch {
        Choice::Lod => e.lod.as_deref().map(|l| (l, &[][..])),
        Choice::Full => Some((e.full.as_str(), e.base.world_alt_textures.as_slice())),
        _ => None,
    }
}

/// The distant cells' streaming each frame (module notes); after the ring, before
/// [`fade_distant`].
#[allow(clippy::too_many_arguments)]
pub fn stream(
    mut commands: Commands,
    world: Res<WorldData>,
    graphics: Res<Graphics>,
    streamer: Res<Streamer>,
    (ring, lod, blocks): (Res<FullDetailRing>, Res<LodState>, Res<BlockContents>),
    mut state: ResMut<DistantState>,
    root: Query<Entity, With<WorldRoot>>,
    time: Res<Time>,
    mut models: ModelCtx,
    mut emittance: ResMut<EmittanceState>,
    (sim, loading): (Option<Res<nvf_play::sim::SimState>>, Option<Res<nvf_play::loading::Loading>>),
    children: Query<&Children>,
) {
    let st = &mut *state;
    let reach = if lod.enabled { graphics.distant_objects.reach() } else { 0 };
    if reach == 0 && st.idle() {
        return;
    }
    world::despawn_dying(&mut commands, &mut st.dying, &children);
    let centre = streamer.centre.filter(|_| streamer.interior.is_none());
    if st.world != world.world || st.travels != streamer.travels || centre.is_none() || !lod.enabled {
        st.clear(&mut commands);
        st.world = world.world;
        st.travels = streamer.travels;
    }
    let shrank = reach < st.reach;
    if reach != st.reach {
        st.reach = reach;
        if reach > 0 {
            info!("distant objects: reach {reach} cells (ratio {SIZE_RATIO})");
            st.used = Some(reach);
        } else {
            st.listing.clear();
            st.listed.clear();
        }
        st.judged = None;
    }
    let Some(centre) = centre else { return };
    let Ok(root) = root.single() else { return };
    let inner = streamer.radius.max(graphics.full_detail.radius());
    let fading = streamer.fade_secs > 0.0;
    let cover = |c: &Grid| (streamer.loaded.contains_key(c) || ring.cells.contains_key(c)).then(|| full_detail::tree_level(streamer.cell_level(*c), Some(&ring.cells), *c));
    let cover_full = |c: &Grid| (streamer.loaded.contains_key(c) && !streamer.fades.contains_key(c)) || ring.cells.get(c).is_some_and(|s| s.full());

    // The hand-off with the loaded and ring cells, then the cells that go.
    let mut gone: Vec<Grid> = Vec::new();
    for (c, s) in &mut st.cells {
        let k = steps(*c, centre);
        match hand_off(s.level, s.leaving, cover(c).is_some()) {
            None => gone.push(*c),
            Some(_) if covered(k, inner, cover_full(c)) => gone.push(*c),
            Some(level) => {
                s.level = level;
                if fades_out(k, reach, shrank) {
                    if fading {
                        s.leaving = true;
                    } else {
                        gone.push(*c);
                    }
                }
            }
        }
    }
    let want = if reach > 0 { distant_cells(centre, inner, reach) } else { Vec::new() };
    for c in &want {
        if let Some(s) = st.cells.get_mut(c) {
            s.leaving = false;
        }
    }
    let rate = if fading { time.delta_secs() / streamer.fade_secs } else { 1.0 };
    let mut rested: Vec<Entity> = Vec::new();
    for (c, s) in &mut st.cells {
        s.level = (s.level + if s.leaving { -rate } else { rate }).clamp(0.0, 1.0);
        if s.leaving && s.level <= 0.0 {
            gone.push(*c);
        }
        // A rebuilt cell at rest or under a cover: the one it replaced goes.
        if s.old.is_some() && (s.level >= 1.0 || cover(c).is_some() || s.leaving) {
            rested.extend(s.old.take().map(|o| o.0));
            s.old_applied = None;
        }
    }
    for e in rested {
        st.retire(&mut commands, e);
    }
    for c in gone {
        if let Some(s) = st.cells.remove(&c) {
            st.retire_cell(&mut commands, s);
        }
        st.rebuild.remove(&c);
        if let Some(sp) = st.spawning.remove(&c) {
            st.retire(&mut commands, sp.entity);
        }
    }
    let dropped: Vec<Grid> = st.spawning.keys().filter(|&&c| fades_out(steps(c, centre), reach, shrank) || steps(c, centre) <= inner).copied().collect();
    for c in dropped {
        if let Some(sp) = st.spawning.remove(&c) {
            st.retire(&mut commands, sp.entity);
        }
    }
    if reach == 0 {
        return;
    }

    let started = Instant::now();
    let deadline = started + SPAWN_BUDGET;
    let behind_screen = loading.is_some_and(|l| l.unseen_frames());
    let loaded = streamer.radius;
    let complete = (-loaded..=loaded).all(|dx| (-loaded..=loaded).all(|dy| streamer.loaded.contains_key(&(centre.0 + dx, centre.1 + dy))));
    let idle = streamer.settled && complete && streamer.spawning.is_empty() && !behind_screen && ring.idle(centre, loaded);
    list_cells(&world, st, &want, centre, inner, idle && !streamer.ahead.busy(), &streamer.hide, &models.data.vfs);
    if !idle {
        st.stats.ms += started.elapsed().as_secs_f64() * 1000.0;
        return;
    }

    // The size rule's ratios, fitted to the budget when the centre moved or lists came in.
    let rejudge = st.judged.is_none_or(|j| steps(j, centre) >= REJUDGE_STEPS);
    if rejudge || st.lists_changed {
        let judge_at = if rejudge { centre } else { st.judged.unwrap_or(centre) };
        fit(st, &want, judge_at, &blocks);
        if rejudge {
            st.judged = Some(centre);
            queue_rebuilds(st, centre, &blocks, &cover, true);
        }
    }
    // Blocks that became known with an empty segment change what flagged cells draw.
    let judged = st.judged.unwrap_or(centre);
    queue_rebuilds(st, judged, &blocks, &cover, false);

    let ctx = SpawnCtx { world: &world, streamer: &streamer, simulated: sim.is_some() };
    // The budget as the cells alive leave it; what this frame spawns is added as it goes.
    let mut budget = st.budget();
    let mut under_way: Vec<Grid> = st.spawning.keys().copied().collect();
    under_way.sort_by_key(|&c| order_key(c, centre));
    for c in under_way {
        if Instant::now() >= deadline {
            break;
        }
        let Some(mut sp) = st.spawning.remove(&c) else { continue };
        spawn_refs(&mut commands, &ctx, st, &mut sp, &mut budget, deadline, &mut models, &mut emittance);
        if sp.done() {
            finish(&mut commands, st, c, sp, fading);
        } else {
            st.spawning.insert(c, sp);
        }
    }
    // Cells not drawn yet nearest first, then cells to build again.
    let mut todo: Vec<(Grid, bool)> = want.iter().filter(|c| !st.cells.contains_key(c) && !st.spawning.contains_key(c)).map(|&c| (c, false)).collect();
    todo.sort_by_key(|&(c, _)| (cover(&c).is_some(), order_key(c, centre)));
    let mut again: Vec<Grid> = st.rebuild.iter().filter(|c| !st.spawning.contains_key(c)).copied().collect();
    again.sort_by_key(|&c| order_key(c, centre));
    todo.extend(again.into_iter().map(|c| (c, true)));
    let mut waiting = 0usize;
    for (c, replaces) in todo {
        if Instant::now() >= deadline || waiting >= PREFETCH_CELLS {
            break;
        }
        // Nearest first: a cell waits for its list.
        let Some(list) = st.listed.get(&c).cloned() else {
            if replaces {
                st.rebuild.remove(&c);
                continue;
            }
            break;
        };
        let segment = blocks.segment(c);
        let before = if replaces { st.cells.get(&c).map(|s| &s.plan) } else { None };
        let plan = plan_cell(&list, c, judged, &st.ratios, segment, before);
        if replaces {
            st.rebuild.remove(&c);
            let same = st.cells.get(&c).is_some_and(|s| drawn_set(&list, &plan) == s.plan.iter().filter(|(_, ch)| ch.drawn()).map(|(r, ch)| (*r, *ch)).collect::<BTreeMap<u32, Choice>>());
            if same {
                if let Some(s) = st.cells.get_mut(&c) {
                    s.judged = judged;
                    s.segment = segment;
                }
                continue;
            }
        }
        let ready = models.models_ready(plan.iter().filter_map(|&(i, ch)| model_of(&list.entries[i], ch)));
        if !ready || waiting > 0 {
            // Its models are asked of the worker threads; later cells only ask for theirs.
            waiting += 1;
            if replaces {
                st.rebuild.insert(c);
            }
            continue;
        }
        let origin = cell_origin(c);
        let entity = commands.spawn((Name::new(format!("Distant cell {},{}", c.0, c.1)), DistantCell { grid: c }, nvf_play::DrawOnlyCell, Transform::from_translation(origin), Visibility::Hidden)).id();
        commands.entity(root).add_child(entity);
        let mut sp = CellSpawn { entity, grid: c, replaces, list, plan, next: 0, later: Vec::new(), origin, judged, segment, drawn: HashMap::new(), full_tris: 0 };
        spawn_refs(&mut commands, &ctx, st, &mut sp, &mut budget, deadline, &mut models, &mut emittance);
        if sp.done() {
            finish(&mut commands, st, c, sp, fading);
        } else {
            st.spawning.insert(c, sp);
        }
    }
    st.stats.ms += started.elapsed().as_secs_f64() * 1000.0;
}

/// A grid square's south-west corner (game space).
fn cell_origin(c: Grid) -> Vec3 {
    Vec3::new(c.0 as f32 * CELL_SIZE, c.1 as f32 * CELL_SIZE, 0.0)
}

/// Start listing the wanted cells not listed (nearest first, [`MAX_LISTING`] at a time,
/// only when `start`); take in the lists done; forget those far beyond the reach or read
/// for a larger inner radius than now.
#[allow(clippy::too_many_arguments)]
fn list_cells(world: &WorldData, st: &mut DistantState, want: &[Grid], centre: Grid, inner: i32, start: bool, hide: &[String], vfs: &Arc<fnv_vfs::Vfs>) {
    let keep = st.reach + KEEP_LISTS;
    st.listing.retain(|c, _| steps(*c, centre) <= keep);
    st.listed.retain(|c, l| steps(*c, centre) <= keep && l.inner <= inner);
    let mut done = Vec::new();
    st.listing.retain(|c, task| match check_ready(task) {
        Some(l) => {
            done.push((*c, l));
            false
        }
        None => true,
    });
    for (c, l) in done {
        // Read for a larger inner radius than now: read again.
        if l.inner <= inner {
            st.listed.insert(c, Arc::new(l));
            st.lists_changed = true;
        }
    }
    if !start {
        return;
    }
    for &c in want {
        if st.listing.len() >= MAX_LISTING {
            break;
        }
        if st.listing.contains_key(&c) || st.listed.contains_key(&c) {
            continue;
        }
        let mut refs: Vec<u32> = world.grid_cell(c).map(|cell| cell.refs.clone()).unwrap_or_default();
        refs.extend(world.persistent_by_grid.get(&c).into_iter().flatten());
        if refs.is_empty() {
            st.listed.insert(c, Arc::new(CellList { inner, ..default() }));
            continue;
        }
        let (lo, schema, hide, vfs) = (world.lo, world.schema, hide.to_vec(), vfs.clone());
        let task = nvf_assets::read_pool::read_pool().spawn(async move { list_cell(lo, schema, &refs, &hide, inner, |p| vfs.exists(p)) });
        st.listing.insert(c, task);
    }
}

/// Fit the size ratios to the budget over the listed wanted cells, judged from `centre`
/// ([`fit_budget`]); log once per centre when it bites.
fn fit(st: &mut DistantState, want: &[Grid], centre: Grid, blocks: &BlockContents) {
    st.lists_changed = false;
    let p = centre_point(centre);
    let mut candidates = Vec::new();
    for c in want {
        let Some(list) = st.listed.get(c) else { continue };
        let k = steps(*c, centre);
        let segment = blocks.segment(*c);
        for e in list.entries.iter().filter(|e| e.lod.is_none() && e.radius > 0.0 && !(e.flagged && segment != Some(false))) {
            let d = Vec2::new(e.p.position[0], e.p.position[1]).distance(p).max(1.0);
            let triangles = st.tris_of.get(&e.full).copied().unwrap_or(UNKNOWN_TRIANGLES);
            candidates.push(Candidate { steps: k, size: e.radius / d, triangles });
        }
    }
    let (ratios, raised) = fit_budget(&candidates, st.reach, SIZE_RATIO, FULL_TRIANGLE_BUDGET);
    if raised && st.budget_logged != Some(centre) {
        st.budget_logged = Some(centre);
        let from = ratios.iter().position(|&r| r > SIZE_RATIO).unwrap_or(0);
        let most = ratios.iter().copied().fold(SIZE_RATIO, f32::max);
        info!("distant objects: full models past {FULL_TRIANGLE_BUDGET} triangles; size ratio raised from {from} cells out (up to {most:.3})");
    }
    st.ratios = ratios;
}

/// Queue drawn cells to build again: with `all`, every cell at rest and alone (the size
/// rule judged anew, kept only when its drawn set changes, decided when it is built);
/// otherwise the cells holding flagged references whose block became known with an empty
/// segment.
fn queue_rebuilds(st: &mut DistantState, centre: Grid, blocks: &BlockContents, cover: &impl Fn(&Grid) -> Option<f32>, all: bool) {
    let mut queue = Vec::new();
    for (c, s) in &st.cells {
        if s.leaving || s.level < 1.0 || s.old.is_some() || cover(c).is_some() {
            continue;
        }
        let newly_empty = s.flagged && s.segment.is_none() && blocks.segment(*c) == Some(false);
        if (all && s.judged != centre) || newly_empty {
            queue.push(*c);
        }
    }
    st.rebuild.extend(queue);
}

/// What a distant reference's spawn reads.
struct SpawnCtx<'a> {
    world: &'a WorldData,
    streamer: &'a Streamer,
    simulated: bool,
}

/// Spawn a distant cell's references until all are in or the deadline has passed; those
/// whose model was released since go to `later`, tried again once the rest are in. Never
/// prepares a model on this thread.
#[allow(clippy::too_many_arguments)]
fn spawn_refs(commands: &mut Commands, ctx: &SpawnCtx, st: &mut DistantState, sp: &mut CellSpawn, budget: &mut Budget, deadline: Instant, models: &mut ModelCtx, emittance: &mut EmittanceState) {
    // A rebuilt cell's predecessor goes once it rests ([`admits`]).
    let predecessor = if sp.replaces { st.cells.get(&sp.grid).map_or(0, |s| s.full_tris) } else { 0 };
    while sp.next < sp.plan.len() && Instant::now() < deadline {
        let (i, ch) = sp.plan[sp.next];
        sp.next += 1;
        if !spawn_one(commands, ctx, st, sp, (budget, predecessor), i, ch, models, emittance) {
            sp.later.push((i, ch));
        }
    }
    if sp.next >= sp.plan.len() && !sp.later.is_empty() {
        sp.plan = std::mem::take(&mut sp.later);
        sp.next = 0;
    }
}

/// One planned reference; false when its model is not prepared (asked of the workers).
#[allow(clippy::too_many_arguments)]
fn spawn_one(commands: &mut Commands, ctx: &SpawnCtx, st: &mut DistantState, sp: &mut CellSpawn, (budget, predecessor): (&mut Budget, u64), i: usize, ch: Choice, models: &mut ModelCtx, emittance: &mut EmittanceState) -> bool {
    let list = sp.list.clone();
    let e = &list.entries[i];
    let r = e.p.form_id;
    // Drawn with a destruction model by the loaded cells, or not shown for its enable
    // state (the game's simulation shows and hides the rest): out.
    if !ch.drawn() || ctx.streamer.model_overrides.contains_key(&r) || !full_detail::drawn_for_enable_state(e.enabled, e.p.deleted(), ctx.simulated) {
        sp.drawn.insert(r, ch);
        return true;
    }
    let Some((path, alt)) = model_of(e, ch) else { return true };
    if !models.models_ready([(path, alt)]) {
        return false;
    }
    let mut tris = 0;
    let base = match ch {
        Choice::Full => {
            tris = models.model_swapped(path, false, alt).map_or(0, |m| m.triangles as u64);
            st.tris_of.insert(e.full.clone(), tris);
            if !admits(*budget, predecessor, tris, FULL_TRIANGLE_BUDGET) {
                sp.drawn.insert(r, Choice::Small);
                return true;
            }
            (*e.base).clone()
        }
        _ => BaseObject { world_model: Some(path.to_string()), world_alt_textures: Vec::new(), ..(*e.base).clone() },
    };
    let rt = Affine3A::from_translation(-sp.origin) * world::ref_affine(&e.p);
    let drawn = world::spawn_model(commands, sp.entity, ctx.world, &e.p, r, &base, rt, models, emittance, &ctx.streamer.hide, &mut st.models, SpawnMode::Scenery);
    // The cell's reservation (read back by [`DistantState::budget`]) and this frame's copy.
    if drawn && ch == Choice::Full {
        sp.full_tris += tris;
        budget.reserved += tris;
    }
    sp.drawn.insert(r, if drawn { ch } else { Choice::Small });
    true
}

/// A distant cell is all in: shown (its meshes get their range in [`fade_distant`]); a
/// rebuilt one takes over from the cell it replaces by crossfading.
fn finish(commands: &mut Commands, st: &mut DistantState, c: Grid, sp: CellSpawn, fading: bool) {
    let mut old = None;
    let mut level = if fading { 0.0 } else { 1.0 };
    if let Some(prev) = st.cells.remove(&c) {
        // Going already: it stays as it is and the rebuild is dropped.
        if prev.leaving {
            st.retire(commands, sp.entity);
            st.cells.insert(c, prev);
            return;
        }
        if let Some((o, _)) = prev.old {
            st.retire(commands, o);
        }
        if fading {
            old = Some((prev.entity, prev.full_tris));
        } else {
            st.retire(commands, prev.entity);
            level = 1.0;
        }
    }
    commands.entity(sp.entity).insert(Visibility::Inherited);
    for ch in sp.drawn.values() {
        match ch {
            Choice::Lod => st.stats.lod += 1,
            Choice::Full => st.stats.full += 1,
            Choice::Held => st.stats.held += 1,
            Choice::Unknown => st.stats.unknown += 1,
            Choice::Small => st.stats.small += 1,
        }
    }
    st.stats.small += sp.list.small;
    st.stats.cells += 1;
    let flagged = sp.list.entries.iter().any(|e| e.flagged);
    let state = CellState {
        entity: sp.entity,
        level,
        leaving: false,
        plan: sp.drawn,
        judged: sp.judged,
        segment: sp.segment,
        flagged,
        full_tris: sp.full_tris,
        old,
        applied: None,
        old_applied: None,
    };
    st.cells.insert(c, state);
}

/// Give each distant cell's meshes their range ([`distant_range`]: the complement of a
/// loaded or ring cell over it, its own fade, or the distance fade at rest) and no sun
/// shadow; the cell a rebuilt one replaces draws the complement of the new one's step.
/// Runs after [`stream`], before `fade::fade_cells`.
#[allow(clippy::type_complexity)]
pub fn fade_distant(
    mut commands: Commands,
    streamer: Res<Streamer>,
    ring: Res<FullDetailRing>,
    mut state: ResMut<DistantState>,
    children: Query<&Children>,
    meshes: Query<(), Fading>,
    casters: Query<Has<NotShadowCaster>, With<Mesh3d>>,
) {
    if state.cells.is_empty() {
        return;
    }
    let reach = state.reach.max(1);
    for (c, s) in &mut state.cells {
        let cover = (streamer.loaded.contains_key(c) || ring.cells.contains_key(c)).then(|| full_detail::tree_level(streamer.cell_level(*c), Some(&ring.cells), *c));
        let want = distant_range(s.level, cover);
        if s.applied != Some((want, reach)) {
            let first = s.applied.is_none();
            s.applied = Some((want, reach));
            let range = want.range(reach);
            for e in children.iter_descendants(s.entity) {
                if meshes.contains(e) {
                    commands.entity(e).insert(range.clone());
                }
                if first && matches!(casters.get(e), Ok(false)) {
                    commands.entity(e).insert(NotShadowCaster);
                }
            }
        }
        if let Some((old, _)) = s.old {
            let k = want.own_step();
            if s.old_applied != Some(k) {
                s.old_applied = Some(k);
                let range = fade::complement(k);
                for e in children.iter_descendants(old).filter(|&e| meshes.contains(e)) {
                    commands.entity(e).insert(range.clone());
                }
            }
        }
    }
}

/// The exit summary (module notes), only when the distant objects were on at some time.
pub fn report(mut exit: MessageReader<AppExit>, state: Res<DistantState>) {
    if exit.read().last().is_none() || state.used.is_none() {
        return;
    }
    let s = &state.stats;
    info!(
        "distant objects summary: {} cells, {} drawn ({} lod, {} full), {} held by blocks, {} block unknown, {} too small, {:.0} ms on the main thread",
        s.cells,
        s.lod + s.full,
        s.lod,
        s.full,
        s.held,
        s.unknown,
        s.small,
        s.ms
    );
}

pub struct DistantObjectsPlugin;

impl Plugin for DistantObjectsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DistantState>()
            .add_systems(Update, (stream.run_if(world::world_not_held), fade_distant).chain().after(full_detail::fade_ring).before(fade::fade_cells))
            .add_systems(Last, report);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const J: Judge = Judge { flagged: false, has_lod: false, radius: 400.0 };

    /// Every branch of the rule: a flagged reference is left to a block that holds it or
    /// is not known yet and drawn like the rest when its segment is empty; a LOD model wins
    /// over the size rule; the full model only when large enough for its distance, with the
    /// hysteresis for one drawn full before; no radius, never full.
    #[test]
    fn choose_takes_every_branch() {
        let flagged = Judge { flagged: true, ..J };
        let d = 8.0 * CELL_SIZE;
        assert_eq!(choose(flagged, None, d, SIZE_RATIO, false), Choice::Unknown);
        assert_eq!(choose(flagged, Some(true), d, SIZE_RATIO, false), Choice::Held);
        assert_eq!(choose(Judge { has_lod: true, ..flagged }, Some(true), d, SIZE_RATIO, false), Choice::Held, "held whatever its models");
        assert_eq!(choose(Judge { has_lod: true, ..flagged }, Some(false), d, SIZE_RATIO, false), Choice::Lod, "an empty segment: drawn");
        assert_eq!(choose(Judge { has_lod: true, ..J }, None, d, SIZE_RATIO, false), Choice::Lod, "unflagged: the block never holds it");
        assert_eq!(choose(Judge { has_lod: true, radius: 1.0, ..J }, Some(true), d, SIZE_RATIO, false), Choice::Lod, "a LOD model at any size");
        // 400 units at 8 cells (32,768): 0.0122 of the distance.
        assert_eq!(choose(J, Some(true), d, SIZE_RATIO, false), Choice::Full);
        assert_eq!(choose(J, Some(true), 12.0 * CELL_SIZE, SIZE_RATIO, false), Choice::Small, "0.0081 at 12 cells");
        assert_eq!(choose(J, Some(true), 12.0 * CELL_SIZE, SIZE_RATIO, true), Choice::Full, "drawn full before: within the hysteresis");
        assert_eq!(choose(J, Some(true), 14.0 * CELL_SIZE, SIZE_RATIO, true), Choice::Small, "0.0070, under 0.8 of the threshold");
        assert_eq!(choose(Judge { radius: 0.0, ..J }, None, 1.0, SIZE_RATIO, true), Choice::Small, "no bounds");
        assert_eq!(choose(J, None, d, 0.02, false), Choice::Small, "a raised ratio");
        assert!(Choice::Lod.drawn() && Choice::Full.drawn());
        assert!(![Choice::Held, Choice::Unknown, Choice::Small].iter().any(|c| c.drawn()));
    }

    /// The hysteresis band: a model enters at the threshold and leaves only under 0.8 of
    /// it, so a centre moving to and fro does not switch it.
    #[test]
    fn size_rule_has_hysteresis() {
        let d = 10_000.0;
        assert!(size_passes(100.0, d, 0.01, false));
        assert!(!size_passes(99.0, d, 0.01, false));
        assert!(size_passes(99.0, d, 0.01, true));
        assert!(size_passes(80.0, d, 0.01, true));
        assert!(!size_passes(79.0, d, 0.01, true));
        assert!(!size_passes(0.0, 0.0, 0.01, true), "no radius");
    }

    /// LOD models beside the full model: `_lod.nif` first, else `_lod_0.nif`, else none; a
    /// model that is itself a LOD model has none.
    #[test]
    fn lod_models_sit_beside_the_full_model() {
        let have = ["meshes\\rocks\\rock01_lod.nif", "meshes\\rocks\\rock01_lod_0.nif", "meshes\\signs\\sign_lod_0.nif"];
        let exists = |p: &str| have.contains(&p);
        assert_eq!(lod_model("meshes\\rocks\\rock01.nif", exists).as_deref(), Some("meshes\\rocks\\rock01_lod.nif"));
        assert_eq!(lod_model("meshes\\signs\\sign.nif", exists).as_deref(), Some("meshes\\signs\\sign_lod_0.nif"), "a pack's own naming");
        assert_eq!(lod_model("meshes\\shacks\\shack.nif", exists), None);
        assert_eq!(lod_model("meshes\\rocks\\rock01_lod.nif", exists), None, "already a LOD model");
        assert_eq!(lod_model("meshes\\x.kf", exists), None);
    }

    /// The budget: within it every ring keeps the base ratio; past it the farthest ring is
    /// raised first, up to the most, then the next; the sum then fits, or every ring with
    /// candidates is at the most.
    #[test]
    fn the_budget_raises_the_ratio_of_the_farthest_cells_first() {
        let c = |steps, size, triangles| Candidate { steps, size, triangles };
        let cands = [c(3, 0.05, 1000), c(3, 0.012, 1000), c(8, 0.05, 1000), c(8, 0.011, 1000), c(8, 0.02, 1000)];
        let (r, raised) = fit_budget(&cands, 8, 0.01, 10_000);
        assert!(!raised && r.len() == 9 && r.iter().all(|&q| q == 0.01), "within the budget");
        // 5,000 triangles against 4,000: ring 8 goes to 0.015, which leaves out its 0.011.
        let (r, raised) = fit_budget(&cands, 8, 0.01, 4000);
        assert!(raised);
        assert!((r[8] - 0.015).abs() < 1e-6, "{r:?}");
        assert_eq!(r[3], 0.01, "the nearer ring untouched");
        // 3,000: ring 8 rises past 0.02 (0.0225), leaving its 0.05 only; 3,000 fits.
        let (r, _) = fit_budget(&cands, 8, 0.01, 3000);
        assert!((r[8] - 0.0225).abs() < 1e-6 && r[3] == 0.01, "{r:?}");
        // 2,000: ring 8 rises past its 0.05 (0.0506) and draws no full model.
        let (r, _) = fit_budget(&cands, 8, 0.01, 2000);
        assert!(r[8] > 0.05 && r[3] == 0.01, "{r:?}");
        // 1,500: ring 8 draws none, so ring 3 is raised next, past its 0.012.
        let (r, _) = fit_budget(&cands, 8, 0.01, 1500);
        assert!(r[8] > 0.05 && r[3] > 0.012 && r[3] < 0.05, "{r:?}");
        // 0: both rings raised until they draw none; rings without candidates untouched.
        let (r, raised) = fit_budget(&cands, 8, 0.01, 0);
        assert!(raised && r[3] > 0.05 && r[8] > 0.05 && r[5] == 0.01, "{r:?}");
        // A model too large for any ratio: its ring at the most, and the loop ends.
        let (r, raised) = fit_budget(&[c(4, 0.5, 1000)], 8, 0.01, 0);
        assert!(raised && r[4] == RATIO_MAX && r[8] == 0.01, "{r:?}");
        assert_eq!(ratio_at(&r, 4), RATIO_MAX);
        assert_eq!(ratio_at(&r, 20), SIZE_RATIO, "past the table");
        assert_eq!(ratio_at(&r, -1), SIZE_RATIO);
    }

    /// The cells: one step beyond the inner radius out to the reach, none whose nearest
    /// point is past the reach's distance (the square's corners), nearest first.
    #[test]
    fn distant_cells_are_beyond_the_inner_radius_nearest_first() {
        let centre = (-17, 2);
        let cells = distant_cells(centre, 2, 8);
        assert!(cells.iter().all(|&c| (3..=8).contains(&steps(c, centre))));
        assert!(cells.contains(&(centre.0 + 8, centre.1)), "a side's middle at the reach");
        assert!(!cells.contains(&(centre.0 + 8, centre.1 + 8)), "a corner past the reach's distance");
        assert!(!cells.contains(&(centre.0 + 2, centre.1)), "a loaded cell");
        let keys: Vec<i32> = cells.iter().map(|&c| order_key(c, centre)).collect();
        assert!(keys.windows(2).all(|w| w[0] <= w[1]), "nearest first");
        assert_eq!(steps(cells[0], centre), 3);
        let p = centre_point(centre);
        assert!(cells.iter().all(|&c| crate::lod::square_distance(p, c, 1) < 8.0 * CELL_SIZE));
        // Beyond the 9 x 9 ring: from 5 steps.
        assert!(distant_cells(centre, 4, 8).iter().all(|&c| steps(c, centre) >= 5));
        assert!(distant_cells(centre, 4, 12).len() > distant_cells(centre, 4, 8).len());
        assert!(distant_cells(centre, 8, 8).is_empty());
        assert_eq!(nearest(2), 2.5 * CELL_SIZE);
    }

    /// CPU mirror of Bevy's visibility-range dither (as full_detail.rs's tests): whether a
    /// pixel with dither `threshold` is drawn by a mesh with range `r` at distance `d`.
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

    /// Pixel-once: a distant cell under a loaded or ring cell (drawn at any step), at any
    /// level of its own, fading in or out, together with that cell draws each pixel of a
    /// reference both draw once; with a loaded cell and a ring cell under it as well (the
    /// ring drawing the loaded cell's complement); a rebuilt cell and the one it replaces
    /// likewise; alone at rest it draws every pixel inside the reach's distance fade.
    #[test]
    fn distant_loaded_and_ring_cover_each_pixel_once() {
        let d = 20_000.0;
        let s = fade::STEPS as i32;
        for kd in 0..=s {
            for leaving in [false, true] {
                for kc in 0..=s {
                    let cover = kc as f32 / s as f32;
                    let Some(level) = hand_off(kd as f32 / s as f32, leaving, true) else { continue };
                    let mine = distant_range(level, Some(cover)).range(REACH_NEAR);
                    let theirs = fade::drawn(kc as u8);
                    for t in 0..16 {
                        let n = [&mine, &theirs].iter().filter(|r| kept(r, d, t)).count();
                        assert_eq!(n, 1, "distant step {kd} leaving {leaving}, cover step {kc}: pixel {t} drawn {n} times");
                    }
                    // A loaded cell at step kc with a ring cell under it (ring at rest, its
                    // complement): the cover level is the ring's 1, the distant cell culled.
                    let ring = full_detail::ring_range(1.0, Some(cover)).range();
                    let tree = full_detail::tree_level(cover, Some(&HashMap::from([((3, 0), ring_state(1.0))])), (3, 0));
                    let mine = distant_range(level, Some(tree)).range(REACH_NEAR);
                    for t in 0..16 {
                        let n = [&mine, &theirs, &ring].iter().filter(|r| kept(r, d, t)).count();
                        assert_eq!(n, 1, "with a ring cell under the loaded one, cover step {kc}: pixel {t} drawn {n} times");
                    }
                }
            }
        }
        // A rebuilt cell fading in over the one it replaces.
        for k in 0..=s {
            let new = distant_range(k as f32 / s as f32, None);
            let old = fade::complement(new.own_step());
            for t in 0..16 {
                let n = [&new.range(REACH_FAR), &old].iter().filter(|r| kept(r, d, t)).count();
                assert_eq!(n, 1, "rebuild step {k}: pixel {t} drawn {n} times");
            }
        }
        // At rest and alone: the distance fade.
        assert_eq!(distant_range(1.0, None), DistantRange::Edge);
        let edge = DistantRange::Edge.range(REACH_NEAR);
        assert!(fade::at_rest(&edge), "a fully drawn range (the sliders' test)");
        assert!((0..16).all(|t| kept(&edge, 6.5 * CELL_SIZE, t)), "inside reach - 1: every pixel");
        assert!((0..16).all(|t| !kept(&edge, 8.1 * CELL_SIZE, t)), "past the reach: none");
        let half = (0..16).filter(|&t| kept(&edge, 7.5 * CELL_SIZE, t)).count();
        assert!(half > 0 && half < 16, "between: dithering out ({half})");
        assert!((edge_range(REACH_FAR).end_margin.end - 12.0 * CELL_SIZE).abs() < 1.0);
        // The hand-off rule is the ring's: held at rest under a cover, dropped when leaving.
        assert_eq!(hand_off(0.3, false, true), Some(1.0));
        assert_eq!(hand_off(0.3, true, true), None);
        assert_eq!(distant_range(0.5, None), DistantRange::Own(8));
        assert_eq!(distant_range(0.5, Some(0.25)), DistantRange::Under(4));
    }

    fn ring_state(level: f32) -> full_detail::RingCellState {
        full_detail::RingCellState::for_test(level)
    }

    /// A plan judges each entry from the middle cell's centre with its ring's ratio; the
    /// drawn set tells whether a cell must be built again.
    #[test]
    fn plans_follow_the_rule_and_the_drawn_set() {
        let base = Arc::new(BaseObject { form_id: 0x200, sig: *b"STAT", edid: None, model: Some("rocks\\rock.nif".into()), world_model: None, world_alt_textures: vec![], worn_only: false, flags: 0, bounds: None });
        let entry = |fid: u32, x: f32, radius: f32, flagged: bool, lod: bool| Entry {
            p: Placement { form_id: fid, sig: *b"REFR", flags: 0, base: Some(0x200), position: [x, 0.5 * CELL_SIZE, 0.0], rotation: [0.0; 3], scale: 1.0, enable_parent: None, open_by_default: false, teleport: false, emittance: None, imposter: false, primitive: false },
            base: base.clone(),
            full: "meshes\\rocks\\rock.nif".into(),
            lod: lod.then(|| "meshes\\rocks\\rock_lod.nif".into()),
            flagged,
            radius,
            enabled: true,
        };
        // Cell (8, 0) from centre (0, 0): references about 8 cells out.
        let x = 8.5 * CELL_SIZE;
        let list = CellList { entries: vec![entry(1, x, 400.0, false, false), entry(2, x, 100.0, false, false), entry(3, x, 10.0, false, true), entry(4, x, 900.0, true, false)], small: 0, inner: 2 };
        let plan = plan_cell(&list, (8, 0), (0, 0), &[], Some(true), None);
        assert_eq!(plan.iter().map(|p| p.1).collect::<Vec<_>>(), [Choice::Full, Choice::Small, Choice::Lod, Choice::Held]);
        let set = drawn_set(&list, &plan);
        assert_eq!(set.len(), 2);
        // The block not known yet: the flagged one waits.
        assert_eq!(plan_cell(&list, (8, 0), (0, 0), &[], None, None)[3].1, Choice::Unknown);
        // Its segment empty: drawn by size.
        assert_eq!(plan_cell(&list, (8, 0), (0, 0), &[], Some(false), None)[3].1, Choice::Full);
        // A raised ratio for 8 steps leaves the 400-unit rock out unless drawn before.
        let mut ratios = vec![SIZE_RATIO; 9];
        ratios[8] = 0.0135;
        assert_eq!(plan_cell(&list, (8, 0), (0, 0), &ratios, Some(true), None)[0].1, Choice::Small);
        let before = HashMap::from([(1u32, Choice::Full)]);
        assert_eq!(plan_cell(&list, (8, 0), (0, 0), &ratios, Some(true), Some(&before))[0].1, Choice::Full, "hysteresis");
    }

    /// A drawn cell for the lifecycle tests, with its full models' triangles and those of
    /// the cell it replaces.
    fn drawn_cell(full_tris: u64, old: Option<u64>) -> CellState {
        CellState {
            entity: Entity::PLACEHOLDER,
            level: 1.0,
            leaving: false,
            plan: HashMap::new(),
            judged: (0, 0),
            segment: None,
            flagged: false,
            full_tris,
            old: old.map(|t| (Entity::PLACEHOLDER, t)),
            applied: None,
            old_applied: None,
        }
    }

    fn a_spawn(grid: Grid, replaces: bool, full_tris: u64) -> CellSpawn {
        CellSpawn {
            entity: Entity::PLACEHOLDER,
            grid,
            replaces,
            list: Arc::new(CellList::default()),
            plan: Vec::new(),
            next: 0,
            later: Vec::new(),
            origin: Vec3::ZERO,
            judged: (0, 0),
            segment: None,
            drawn: HashMap::new(),
            full_tris,
        }
    }

    /// The budget equals the sum over the cells alive at every step of a lifecycle: a cell
    /// spawning reserves, a cancelled one gives its share back by leaving, a finished one
    /// retains, a rebuilt one's predecessor counts as fading until it rests, a retired one
    /// counts no more. Nothing is left over once every cell is gone.
    #[test]
    fn the_budget_is_the_sum_over_the_cells_alive() {
        let sum = |st: &DistantState| -> u64 { st.cells.values().map(|s| s.full_tris + s.old.map_or(0, |o| o.1)).sum::<u64>() + st.spawning.values().map(|s| s.full_tris).sum::<u64>() };
        let world = World::new();
        let mut queue = bevy::ecs::world::CommandQueue::default();
        let mut commands = Commands::new(&mut queue, &world);
        let mut st = DistantState::default();
        // A half-spawned cell that is then dropped (the centre moved): nothing is left.
        st.spawning.insert((9, 0), a_spawn((9, 0), false, 1200));
        assert_eq!(st.budget(), Budget { retained: 0, reserved: 1200, fading: 0 });
        let sp = st.spawning.remove(&(9, 0)).unwrap();
        st.retire(&mut commands, sp.entity);
        assert_eq!(st.budget().total(), 0, "a cancelled cell gives its share back");
        // A finished cell retains its models.
        st.spawning.insert((9, 0), a_spawn((9, 0), false, 3000));
        let sp = st.spawning.remove(&(9, 0)).unwrap();
        finish(&mut commands, &mut st, (9, 0), sp, true);
        assert_eq!(st.budget(), Budget { retained: 3000, reserved: 0, fading: 0 });
        assert_eq!(st.budget().total(), sum(&st));
        // Rebuilt in place, crossfading: the predecessor counts as fading.
        st.cells.get_mut(&(9, 0)).unwrap().level = 1.0;
        let sp = a_spawn((9, 0), true, 2500);
        finish(&mut commands, &mut st, (9, 0), sp, true);
        assert_eq!(st.budget(), Budget { retained: 2500, reserved: 0, fading: 3000 });
        assert_eq!(st.budget().total(), sum(&st));
        // A rejected rebuild (its cell started leaving) is dropped with its reservation.
        st.cells.get_mut(&(9, 0)).unwrap().leaving = true;
        finish(&mut commands, &mut st, (9, 0), a_spawn((9, 0), true, 700), true);
        assert_eq!(st.budget().total(), 5500, "the leaving cell and its predecessor only");
        // Retired with its predecessor: nothing left.
        let s = st.cells.remove(&(9, 0)).unwrap();
        st.retire_cell(&mut commands, s);
        assert_eq!(st.budget(), Budget::default());
        // Cleared with cells and spawns alive: nothing left either.
        st.cells.insert((10, 0), drawn_cell(400, Some(100)));
        st.spawning.insert((11, 0), a_spawn((11, 0), false, 50));
        assert_eq!(st.budget().total(), 550);
        assert_eq!(st.budget().total(), sum(&st));
        st.clear(&mut commands);
        assert_eq!(st.budget(), Budget::default());
    }

    /// At the cap a cell rebuilt in place may keep the models of its predecessor (whose
    /// share goes once it rests); a new cell may not pass the cap, and a fading
    /// predecessor counts against every other spawn.
    #[test]
    fn a_replacement_at_the_cap_keeps_its_models() {
        let cap = 4000;
        let b = Budget { retained: 3000, reserved: 0, fading: 0 };
        assert!(admits(b, 3000, 3000, cap), "the rebuilt cell keeps what its predecessor drew");
        assert!(!admits(b, 0, 3000, cap), "a new cell would pass the cap");
        assert!(admits(b, 0, 1000, cap));
        let crossfading = Budget { retained: 3000, reserved: 0, fading: 3000 };
        assert!(!admits(crossfading, 0, 1, cap), "the fading predecessor still counts for other cells");
        assert!(admits(Budget { retained: 3000, reserved: 500, fading: 0 }, 3000, 3500, cap), "reservations count");
        assert!(!admits(Budget { retained: 3000, reserved: 600, fading: 0 }, 3000, 3500, cap));
    }

    /// Off in both looks; `--graphics-only distant-objects-8` / `distant-objects-12` turn
    /// it on; the choices name their reaches.
    #[test]
    fn off_in_both_presets_and_the_keys_parse() {
        use crate::graphics::Preset;
        assert_eq!(Graphics::preset(Preset::Vanilla).distant_objects, DistantObjects::Off);
        assert_eq!(Graphics::preset(Preset::Enhanced).distant_objects, DistantObjects::Off);
        assert_eq!(Graphics::only(&["distant-objects-8".into()]).unwrap().distant_objects, DistantObjects::Eight);
        let far = Graphics::only(&["distant-objects-12".into()]).unwrap();
        assert_eq!(far.distant_objects, DistantObjects::Twelve);
        assert_eq!(far.describe(), "light glow, distant objects 12 cells");
        assert!(Graphics::only(&["distant-objects-16".into()]).is_err());
        assert_eq!(DistantObjects::default(), DistantObjects::Off);
        assert_eq!(DistantObjects::CHOICES.map(|(_, d)| d.reach()), [0, REACH_NEAR, REACH_FAR]);
        assert_eq!(DistantObjects::Twelve.name(), "12 cells");
        assert_eq!(DistantObjects::Off.describe(), "");
    }

    /// A distant cell is no loaded or render cell, and the sliders leave its meshes alone.
    #[test]
    fn a_distant_cell_is_no_render_or_loaded_cell() {
        let mut world = World::new();
        world.spawn((DistantCell { grid: (9, 0) }, nvf_play::DrawOnlyCell, Transform::default(), Visibility::Hidden));
        assert_eq!(world.query::<&nvf_play::RenderCell>().iter(&world).count(), 0);
        assert_eq!(world.query_filtered::<(), With<world::CellEntity>>().iter(&world).count(), 0);
        assert_eq!(world.query::<&DistantCell>().iter(&world).count(), 1);
    }
}
