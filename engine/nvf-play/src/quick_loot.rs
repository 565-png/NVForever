//! **Quick Loot** [design: owner 2026-11-17, docs/design/quick-loot.md]: a container, a body
//! or (sneaking) a person under the crosshair shows what it holds in a small list beside the
//! crosshair, without the menu that stops the game.
//!
//! - **What shows it**: the activation target (`crate::activate::Target`) is a container
//!   that opens (`Action::Open` on a CONT; a locked one keeps its lock prompt), a body or ash
//!   pile to search (`Action::Search`) or a pocket the player may pick (`Action::Steal`
//!   with no refusal, `fnv_game::Game::pickpocket_refusal`), and the NVF Features row Quick
//!   Loot is on (off by default). Not for a reference whose script has an OnActivate block,
//!   a reference set off by its activate parents only or one that cannot be activated: the
//!   game's own activation decides those, so their prompt stays as it is.
//! - **Keys** (the player's bindings): Activate (E) takes the selected stack, Wait (T)
//!   takes everything (not from a pocket: each item is its own roll), Ready Item (R) opens
//!   the full container or pickpocket menu as Activate would without Quick Loot, and the
//!   mouse wheel moves the selection. While the list shows, those keys do nothing else:
//!   no reload, no Wait menu, no camera zoom ([`holds_input`]).
//! - **The rules are the container menu's** (`crate::menu::model::MenuModel`, opened on the
//!   target as the menu opens it, its pickpocket rolls included): a take from someone
//!   else's container is theft (`crate::crime::ItemsTaken`), a pocket's take rolls its
//!   chance and a failure is caught (`crate::pickpocket::Pickpocketed`).
//! - **Drawn** in the HUD colour (`crate::interface::amber`, recoloured with it), the crime
//!   colour for theft, at the HUD's scale (`crate::interface::hud_units`), in the font the
//!   Menu Font row picks (`crate::pause::MenuFont`: the game's font 7 or the sharp one).
//!   Columns for weight, value and value per weight each have a row of their own (off by
//!   default); with any column (or a pocket's chance) the count goes beside the name, else
//!   it is right-aligned in a column of its own [design: owner 2026-11-17].
//! - **Logs** (smoke): `quick loot: shown on 0x… "<name>" (<kind>, <n> rows)`,
//!   `quick loot: took …`, `quick loot: the full menu`, and per drawing `quick loot: drawn
//!   "<name>" … font Original|Sharp …: no overlaps` (or what overlaps, [`overlaps`]).

use crate::activate::outcome::Action;
use crate::activate::Target;
use crate::bindings::Control;
use crate::interface::{amber, at, bc_supported, hud_units, set_line, Face};
use crate::menu::model::{Command, MenuModel, Row, Side};
use crate::start_menu::{base_in, Faces};
use crate::sim::SimState;
use crate::{PlayState, RefChanged};
use bevy::image::CompressedImageFormatSupport;
use bevy::input::mouse::AccumulatedMouseScroll;
use bevy::prelude::*;
use fnv_sim::forms::BaseData;
use fnv_sim::FormId;
use fnv_game::game::event;
use nvf_assets::GameData;
use std::sync::atomic::{AtomicBool, Ordering};

// ---- the settings -----------------------------------------------------------------------

static ON: AtomicBool = AtomicBool::new(false);
static WEIGHT: AtomicBool = AtomicBool::new(false);
static VALUE: AtomicBool = AtomicBool::new(false);
static PER_WEIGHT: AtomicBool = AtomicBool::new(false);
/// The list shows and has its keys this frame ([`holds_input`]).
static HOLDS: AtomicBool = AtomicBool::new(false);
/// Ready Item asked for the full menu: `crate::activate::activate` activates the target as
/// Activate would ([`take_full_request`]).
static FULL: AtomicBool = AtomicBool::new(false);

/// Whether the Quick Loot list shows and has Activate, Wait, Ready Item and the mouse wheel:
/// the systems that read those leave them alone while it does (set in `PlaySet::Activation`;
/// the input systems before it see the last frame's).
pub fn holds_input() -> bool {
    HOLDS.load(Ordering::Relaxed)
}

/// Ready Item asked for the full menu this frame (read once).
pub fn take_full_request() -> bool {
    FULL.swap(false, Ordering::Relaxed)
}

/// Which columns are on (the rows' switches) and whether a pocket's chance shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Columns {
    pub weight: bool,
    pub value: bool,
    pub per_weight: bool,
    pub chance: bool,
}

impl Columns {
    pub fn any(&self) -> bool {
        self.weight || self.value || self.per_weight || self.chance
    }

    fn chosen(chance: bool) -> Columns {
        Columns { weight: WEIGHT.load(Ordering::Relaxed), value: VALUE.load(Ordering::Relaxed), per_weight: PER_WEIGHT.load(Ordering::Relaxed), chance }
    }
}

// ---- what is shown ----------------------------------------------------------------------

/// What the list is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Container,
    Body,
    Pocket,
}

/// Rows shown at once; the list scrolls to keep the selection in them [design: owner
/// 2026-11-17, design A].
pub const VISIBLE: usize = 6;

/// The list on one target.
#[derive(Debug, Clone)]
pub struct Shown {
    /// The reference under the crosshair, and the one whose items these are (a body's for
    /// its ash pile).
    pub target: u32,
    pub holder: FormId,
    pub kind: Kind,
    /// Taking is theft (someone else's container, any pocket).
    pub crime: bool,
    pub title: String,
    pub model: MenuModel,
    /// Seconds since the rows were read from the simulation.
    since_read: f32,
}

impl Shown {
    pub fn rows(&self) -> &[Row] {
        &self.model.list(Side::Container).rows
    }

    pub fn selected(&self) -> usize {
        self.model.selected[Side::Container as usize].min(self.rows().len().saturating_sub(1))
    }

    /// The first row shown: the selection kept two rows from the top where it can be.
    pub fn first(&self) -> usize {
        let n = self.rows().len();
        self.selected().saturating_sub(2).min(n.saturating_sub(VISIBLE))
    }
}

#[derive(Resource, Default)]
pub struct QuickLoot {
    pub shown: Option<Shown>,
    /// The target a caught pickpocketing closed the list on: not shown on it again until
    /// the crosshair leaves it.
    closed_on: Option<u32>,
}

/// The rows' words: what [`layout`] lays out.
#[derive(Clone, Debug, PartialEq)]
pub struct Spec {
    pub title: String,
    /// "3/8": the selection's place and the rows.
    pub position: String,
    pub rows: Vec<RowSpec>,
    pub more_up: bool,
    pub more_down: bool,
    pub columns: Columns,
    /// The keys' lines ("E) Take").
    pub prompts: Vec<String>,
    pub crime: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RowSpec {
    pub name: String,
    pub count: i32,
    pub weight: Option<f32>,
    pub value: Option<i32>,
    pub chance: Option<i32>,
    pub selected: bool,
}

/// What an empty list says [design].
pub const EMPTY: &str = "Empty";
const TAKE: &str = "Take";
const STEAL: &str = "Steal";
const TAKE_ALL: &str = "Take All";
const STEAL_ALL: &str = "Steal All";
const SEARCH: &str = "Search";
const WG: &str = "WG";
const VAL: &str = "VAL";
const PER_WG: &str = "V/W";
const CHANCE: &str = "CHANCE";

impl Spec {
    pub fn of(s: &Shown, columns: Columns, key: impl Fn(Control) -> String) -> Spec {
        let rows = s.rows();
        let (first, sel) = (s.first(), s.selected());
        let chance_of = |r: &Row| s.model.steal.as_ref().map(|st| st.chance(r, r.count));
        let shown: Vec<RowSpec> = rows
            .iter()
            .enumerate()
            .skip(first)
            .take(VISIBLE)
            .map(|(i, r)| RowSpec { name: r.name.clone(), count: r.count, weight: r.weight, value: r.value, chance: chance_of(r), selected: i == sel })
            .collect();
        let steal = s.crime;
        let mut prompts = vec![format!("{} {}", key(Control::Activate), if steal { STEAL } else { TAKE })];
        if s.kind != Kind::Pocket {
            prompts.push(format!("{} {}", key(Control::Rest), if steal { STEAL_ALL } else { TAKE_ALL }));
        }
        prompts.push(format!("{} {SEARCH}", key(Control::ReadyItem)));
        Spec {
            title: s.title.clone(),
            position: if rows.is_empty() { "0/0".into() } else { format!("{}/{}", sel + 1, rows.len()) },
            more_up: first > 0,
            more_down: first + VISIBLE < rows.len(),
            rows: shown,
            columns,
            prompts,
            crime: steal,
        }
    }
}

// ---- layout -----------------------------------------------------------------------------

/// The list's measures, interface units (960 to the view's height) [design: owner
/// 2026-11-17, design A]: its left edge right of the crosshair, its top above it.
pub const LEFT: f32 = 90.0;
pub const TOP: f32 = -170.0;
const PAD_X: f32 = 16.0;
const PAD_Y: f32 = 8.0;
const TITLE_H: f32 = 40.0;
const RULE: f32 = 1.5;
const HEADER_H: f32 = 26.0;
const MORE_H: f32 = 12.0;
pub const ROW_H: f32 = 40.0;
const ROW_GAP: f32 = 2.0;
/// Between a row's text and its box, and between columns.
const CELL: f32 = 10.0;
pub const GAP: f32 = 22.0;
const NAME_MIN: f32 = 240.0;
const NAME_MAX: f32 = 380.0;
/// Kept clear of the view's right edge: a list too wide for a narrow view moves left.
const EDGE: f32 = 20.0;
const PROMPT_GAP: f32 = 8.0;
const PROMPT_H: f32 = 40.0;
const PROMPT_SPACE: f32 = 30.0;
const FRAME: f32 = 3.0;
const TICK: f32 = 14.0;

/// Which of the faces a text is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Which {
    Entry,
    Title,
    Small,
}

/// How a text or a bar is coloured.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ink {
    /// The HUD colour, full.
    Hud,
    /// The HUD colour, dim.
    Dim,
    /// A row's text: the crime colour for theft, else the HUD's; full when selected.
    Row(bool),
    /// The selection's fill under its row.
    Fill,
    /// The dark backing.
    Backing,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Piece {
    pub text: String,
    pub which: Which,
    pub x: f32,
    pub base: f32,
    pub width: f32,
    pub ink: Ink,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bar {
    pub rect: Rect,
    pub ink: Ink,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Laid {
    pub size: Vec2,
    /// The framed box's height (the prompts are under it).
    pub box_h: f32,
    pub pieces: Vec<Piece>,
    pub bars: Vec<Bar>,
    /// Each row's band (the selection's box), top to bottom.
    pub bands: Vec<Rect>,
}

fn face(faces: &Faces, w: Which) -> &Face {
    match w {
        Which::Entry => &faces.entry,
        Which::Title => &faces.title,
        Which::Small => &faces.small,
    }
}

/// A weight per unit of value as written: whole above 10, else one place; "--" for nothing
/// carried or nothing worth [design].
pub fn per_weight_text(weight: Option<f32>, value: Option<i32>) -> String {
    match (weight.filter(|w| *w > 0.0), value.filter(|v| *v > 0)) {
        (Some(w), Some(v)) => {
            let r = v as f32 / w;
            if r >= 10.0 {
                format!("{}", r.round() as i64)
            } else {
                format!("{r:.1}")
            }
        }
        _ => "--".into(),
    }
}

/// The list laid out from its top left corner, measured by `faces` (the drawn fonts).
pub fn layout(spec: &Spec, faces: &Faces) -> Laid {
    let width = |w: Which, s: &str| face(faces, w).font.width(s);
    let c = spec.columns;
    let counted = !c.any();
    // Each row's words: the name (with its count where columns show), the count, the columns.
    let label = |r: &RowSpec| if !counted && r.count > 1 { format!("{} ({})", r.name, r.count) } else { r.name.clone() };
    let count = |r: &RowSpec| if counted && r.count > 1 { format!("({})", r.count) } else { String::new() };
    let mut cols: Vec<(&str, Vec<String>)> = Vec::new();
    if c.weight {
        cols.push((WG, spec.rows.iter().map(|r| crate::reticle::weight_text(r.weight)).collect()));
    }
    if c.value {
        cols.push((VAL, spec.rows.iter().map(|r| crate::reticle::value_text(r.value)).collect()));
    }
    if c.per_weight {
        cols.push((PER_WG, spec.rows.iter().map(|r| per_weight_text(r.weight, r.value)).collect()));
    }
    if c.chance {
        cols.push((CHANCE, spec.rows.iter().map(|r| r.chance.map_or_else(|| "--".into(), |p| format!("{p}%"))).collect()));
    }
    let col_w: Vec<f32> = cols.iter().map(|(h, cells)| cells.iter().map(|t| width(Which::Entry, t)).fold(width(Which::Small, h), f32::max)).collect();
    let count_w = spec.rows.iter().map(|r| width(Which::Entry, &count(r))).fold(0.0, f32::max);
    let names = spec.rows.iter().map(|r| width(Which::Entry, &label(r))).fold(width(Which::Entry, EMPTY), f32::max);
    let name_room = names.clamp(NAME_MIN, NAME_MAX);
    let mut inner = name_room + col_w.iter().map(|w| GAP + w).sum::<f32>();
    if count_w > 0.0 {
        inner += GAP + count_w;
    }
    // The heading's name and place need their room too.
    let position_w = width(Which::Small, &spec.position);
    inner = inner.max(NAME_MIN + GAP + position_w);
    let w = inner + 2.0 * (PAD_X + CELL);
    let (x0, x1) = (PAD_X + CELL, w - PAD_X - CELL);
    let mut pieces = Vec::new();
    let mut bars = Vec::new();
    let mut bands = Vec::new();
    let mut piece = |text: String, which: Which, x: f32, base: f32, ink: Ink| {
        let width = width(which, &text);
        pieces.push(Piece { text, which, x, base, width, ink });
    };
    let mut y = PAD_Y;
    // The heading: the name, its place in the rows at the right.
    let title_room = x1 - x0 - GAP - position_w;
    let title = crate::pause::fit_text(|t| width(Which::Title, t), &spec.title, title_room);
    piece(title, Which::Title, x0, y + base_in(&faces.title, TITLE_H), Ink::Hud);
    piece(spec.position.clone(), Which::Small, x1 - position_w, y + base_in(&faces.small, TITLE_H), Ink::Dim);
    y += TITLE_H;
    bars.push(Bar { rect: Rect::new(PAD_X, y, w - PAD_X, y + RULE), ink: Ink::Dim });
    y += RULE + 2.0;
    // The columns' heads, right-aligned over their figures.
    let mut col_end = Vec::new();
    let mut end = x1;
    for cw in col_w.iter().rev() {
        col_end.push(end);
        end -= cw + GAP;
    }
    col_end.reverse();
    let count_end = end;
    if !cols.is_empty() {
        let base = y + base_in(&faces.small, HEADER_H);
        for ((h, _), e) in cols.iter().zip(&col_end) {
            piece((*h).to_string(), Which::Small, e - width(Which::Small, h), base, Ink::Dim);
        }
        y += HEADER_H;
    }
    if spec.more_up {
        bars.push(Bar { rect: Rect::new(w / 2.0 - 16.0, y + MORE_H / 2.0 - 1.5, w / 2.0 + 16.0, y + MORE_H / 2.0 + 1.5), ink: Ink::Dim });
    }
    y += MORE_H;
    let name_end = if count_w > 0.0 { count_end - count_w - GAP } else { count_end };
    for (i, r) in spec.rows.iter().enumerate() {
        let band = Rect::new(PAD_X, y, w - PAD_X, y + ROW_H);
        bands.push(band);
        if r.selected {
            bars.push(Bar { rect: band, ink: Ink::Fill });
            let edge = |rect: Rect| Bar { rect, ink: Ink::Row(true) };
            bars.push(edge(Rect::new(band.min.x, band.min.y, band.max.x, band.min.y + RULE)));
            bars.push(edge(Rect::new(band.min.x, band.max.y - RULE, band.max.x, band.max.y)));
            bars.push(edge(Rect::new(band.min.x, band.min.y, band.min.x + RULE, band.max.y)));
            bars.push(edge(Rect::new(band.max.x - RULE, band.min.y, band.max.x, band.max.y)));
        }
        let base = y + base_in(&faces.entry, ROW_H);
        let ink = Ink::Row(r.selected);
        let name = crate::pause::fit_text(|t| width(Which::Entry, t), &label(r), name_end - x0);
        piece(name, Which::Entry, x0, base, ink);
        let n = count(r);
        if !n.is_empty() {
            let nw = width(Which::Entry, &n);
            piece(n, Which::Entry, count_end - nw, base, ink);
        }
        for ((_, cells), e) in cols.iter().zip(&col_end) {
            let t = cells[i].clone();
            let tw = width(Which::Entry, &t);
            piece(t, Which::Entry, e - tw, base, ink);
        }
        y += ROW_H + ROW_GAP;
    }
    if spec.rows.is_empty() {
        bands.push(Rect::new(PAD_X, y, w - PAD_X, y + ROW_H));
        piece(EMPTY.into(), Which::Entry, x0, y + base_in(&faces.entry, ROW_H), Ink::Dim);
        y += ROW_H + ROW_GAP;
    }
    if spec.more_down {
        bars.push(Bar { rect: Rect::new(w / 2.0 - 16.0, y + MORE_H / 2.0 - 1.5, w / 2.0 + 16.0, y + MORE_H / 2.0 + 1.5), ink: Ink::Dim });
    }
    y += MORE_H + PAD_Y;
    let box_h = y;
    // The frame: the backing, top and bottom lines with ticks down and up at their ends.
    let mut frame = vec![
        Bar { rect: Rect::new(0.0, 0.0, w, box_h), ink: Ink::Backing },
        Bar { rect: Rect::new(0.0, 0.0, w, FRAME), ink: Ink::Hud },
        Bar { rect: Rect::new(0.0, box_h - FRAME, w, box_h), ink: Ink::Hud },
    ];
    for (x, y0) in [(0.0, 0.0), (w - FRAME, 0.0), (0.0, box_h - TICK), (w - FRAME, box_h - TICK)] {
        frame.push(Bar { rect: Rect::new(x, y0, x + FRAME, y0 + TICK), ink: Ink::Hud });
    }
    frame.extend(bars);
    // The keys under the box.
    let base = box_h + PROMPT_GAP + base_in(&faces.entry, PROMPT_H);
    let mut x = PAD_X / 2.0;
    for (i, p) in spec.prompts.iter().enumerate() {
        let ink = if i == 0 || (i == 1 && spec.prompts.len() == 3) { Ink::Row(true) } else { Ink::Hud };
        piece(p.clone(), Which::Entry, x, base, ink);
        x += width(Which::Entry, p) + PROMPT_SPACE;
    }
    let prompts_w = x - PROMPT_SPACE;
    Laid { size: Vec2::new(w.max(prompts_w), box_h + PROMPT_GAP + PROMPT_H), box_h, pieces, bars: frame, bands }
}

/// A text's box: its line's room from the top of its capitals to under its baseline.
pub fn text_box(p: &Piece, faces: &Faces) -> Rect {
    let f = &face(faces, p.which).font;
    let cap = f.glyph(b'H').ascent.max(f.size * 0.5);
    Rect::new(p.x, p.base - cap, p.x + p.width, p.base + cap * 0.3)
}

/// What overlaps in a laid-out list, as words for the log: two texts whose boxes meet, a
/// row's text outside its band, a text outside the box (the prompts outside the list's
/// size) [design: owner 2026-11-17, "no elements should overlap"].
pub fn overlaps(l: &Laid, faces: &Faces) -> Vec<String> {
    let mut out = Vec::new();
    let boxes: Vec<Rect> = l.pieces.iter().map(|p| text_box(p, faces)).collect();
    for i in 0..boxes.len() {
        for j in i + 1..boxes.len() {
            let (a, b) = (boxes[i], boxes[j]);
            if a.width() > 0.0 && b.width() > 0.0 && !a.intersect(b).is_empty() {
                out.push(format!("\"{}\" and \"{}\"", l.pieces[i].text, l.pieces[j].text));
            }
        }
        let p = &l.pieces[i];
        let inside = if p.base < l.box_h { Rect::new(PAD_X, 0.0, l.size.x - PAD_X, l.box_h) } else { Rect::new(0.0, l.box_h, l.size.x, l.size.y) };
        if boxes[i].width() > 0.0 && (boxes[i].min.x < inside.min.x - 0.01 || boxes[i].max.x > inside.max.x + 0.01) {
            out.push(format!("\"{}\" outside the list", p.text));
        }
        if matches!(p.ink, Ink::Row(_)) && p.base < l.box_h && !l.bands.iter().any(|b| p.base > b.min.y && p.base < b.max.y) {
            out.push(format!("\"{}\" off its row", p.text));
        }
    }
    out
}

// ---- following the target ---------------------------------------------------------------

/// How often the rows are read again while the list stays on one target (a script or a
/// companion may change what it holds), seconds [design].
const REREAD: f32 = 0.5;

/// The list's kind for target `t`, or none; `container`: its base is a container.
fn wanted(t: &crate::activate::Targeted, container: bool, g: &mut fnv_game::Game) -> Option<Kind> {
    let kind = match t.action {
        Action::Search => Kind::Body,
        Action::Open if container => Kind::Container,
        Action::Steal => Kind::Pocket,
        _ => return None,
    };
    let id = FormId(t.form_id);
    if kind == Kind::Pocket {
        return g.pickpocket_refusal(id).is_none().then_some(kind);
    }
    // The game's own activation decides these (module notes).
    if g.activation_refused(id) || g.parent_activate_only(id) {
        return None;
    }
    let scripted = g.ref_script(id).and_then(|s| g.data.script(s)).is_some_and(|u| u.has_block(event::ON_ACTIVATE));
    (!scripted).then_some(kind)
}

fn open(t: &crate::activate::Targeted, kind: Kind, sim: &mut SimState, rt: &mut crate::script::Runtime) -> Option<Shown> {
    let id = FormId(t.form_id);
    let holder = match kind {
        Kind::Body => rt.game.ash_pile_source(id).unwrap_or(id),
        _ => id,
    };
    let opened = match kind {
        Kind::Pocket => {
            let (rules, (thief, target), worn, overhaul) = crate::script::with_world(rt, sim, |rt| (rt.game.pickpocket_settings(), rt.game.pickpocket_skills(id), rt.game.pickpocket_worn(id), rt.game.pickpocket_overhaul(id)));
            MenuModel::open_container(&mut sim.world, id).and_then(|mut m| {
                let mut steal = crate::menu::model::Steal::new(id, rules, thief, target, crate::menu::model::steal_seed_now(id));
                steal.worn = worn;
                steal.overhaul = overhaul;
                m.steal = Some(steal);
                m.refresh(&mut sim.world)?;
                Ok(m)
            })
        }
        _ => MenuModel::open_container(&mut sim.world, holder),
    };
    match opened {
        Ok(model) => Some(Shown { target: t.form_id, holder, kind, crime: t.crime, title: t.name.clone(), model, since_read: 0.0 }),
        Err(e) => {
            warn!("quick loot: 0x{:08X} {}: cannot be listed: {e}", t.form_id, t.name);
            None
        }
    }
}

type Overlays<'w> = (
    Option<Res<'w, crate::weapon_wheel::WeaponWheel>>,
    Option<Res<'w, crate::vats_mode::VatsMode>>,
    Option<Res<'w, crate::creator::CreatorMenu>>,
    Option<Res<'w, crate::player::ViewDriven>>,
    Option<Res<'w, crate::kill_cam::KillCam>>,
    Option<Res<'w, crate::control::ControlLocks>>,
);

type Out<'w> = (
    MessageWriter<'w, crate::crime::ItemsTaken>,
    MessageWriter<'w, crate::pickpocket::Pickpocketed>,
    MessageWriter<'w, RefChanged>,
    MessageWriter<'w, crate::ui_sounds::UiSound>,
);

/// Follows the target, reads the keys: in `PlaySet::Activation`, after the target is found
/// and before `crate::activate::activate` reads Activate.
#[allow(clippy::too_many_arguments)]
pub(crate) fn follow(
    input: crate::bindings::ControlInput,
    scroll: Res<AccumulatedMouseScroll>,
    time: Res<Time>,
    target: Res<Target>,
    mut ql: ResMut<QuickLoot>,
    mut sim: ResMut<SimState>,
    script: Option<ResMut<crate::script::ScriptState>>,
    (wheel, vats, creator, driven, kill_cam, locks): Overlays,
    (mut taken, mut stolen, mut changed, mut sounds): Out,
    mut corner: Option<ResMut<crate::corner_messages::CornerMessages>>,
) {
    let mut script = script;
    let busy = wheel.is_some_and(|w| w.holds_input())
        || vats.is_some_and(|v| v.holds_input() || v.selected().is_some() || v.playing_back())
        || creator.is_some_and(|c| c.is_open())
        || driven.is_some_and(|d| d.0)
        || kill_cam.is_some_and(|k| k.active())
        || locks.is_some_and(|l| l.rollover || l.movement);
    let t = target.0.as_ref().filter(|_| ON.load(Ordering::Relaxed) && !busy);
    if ql.closed_on.is_some() && t.map(|t| t.form_id) != ql.closed_on {
        ql.closed_on = None;
    }
    let t = t.filter(|t| Some(t.form_id) != ql.closed_on);
    // The list on the target: kept while it stays, opened anew on another.
    let same = matches!((t, ql.shown.as_ref()), (Some(t), Some(s)) if s.target == t.form_id);
    if !same {
        let container = t.is_some_and(|t| sim.base_of(t.form_id).and_then(|b| sim.forms.get(b)).is_some_and(|b| matches!(b.data, BaseData::Container(_))));
        let opened = match (t, script.as_mut().and_then(|s| s.rt.as_mut())) {
            (Some(t), Some(rt)) => crate::script::with_world(rt, &mut sim, |rt| wanted(t, container, &mut rt.game)).and_then(|kind| open(t, kind, &mut sim, rt)),
            _ => None,
        };
        if let Some(s) = &opened {
            info!("quick loot: shown on 0x{:08X} \"{}\" ({:?}{}, {} rows)", s.target, s.title, s.kind, if s.crime { ", theft" } else { "" }, s.rows().len());
        } else if let Some(s) = &ql.shown {
            info!("quick loot: hidden (was on 0x{:08X} \"{}\")", s.target, s.title);
        }
        ql.shown = opened;
    }
    let shown_now = ql.shown.is_some();
    HOLDS.store(shown_now, Ordering::Relaxed);
    let Some(s) = ql.shown.as_mut() else { return };
    s.since_read += time.delta_secs();
    if s.since_read >= REREAD {
        s.since_read = 0.0;
        if let Err(e) = s.model.refresh(&mut sim.world) {
            warn!("quick loot: 0x{:08X} cannot be listed again: {e}", s.target);
        }
    }
    // The wheel moves the selection, up moving it up.
    let notches = scroll.delta.y;
    if notches != 0.0 && !input.suppressed() {
        let steps = if notches > 0.0 { -1 } else { 1 };
        s.model.apply(&mut sim.world, Command::Scroll(steps));
    }
    let cmd = if input.just_pressed(Control::Activate) {
        Some(Command::Stack)
    } else if input.just_pressed(Control::Rest) && s.kind != Kind::Pocket {
        Some(Command::TakeAll)
    } else {
        if input.just_pressed(Control::ReadyItem) {
            info!("quick loot: the full menu on 0x{:08X} \"{}\"", s.target, s.title);
            FULL.store(true, Ordering::Relaxed);
        }
        None
    };
    let Some(cmd) = cmd else { return };
    if s.rows().is_empty() {
        return;
    }
    let forms = sim.world.forms;
    s.model.focus = Side::Container;
    let a = s.model.apply(&mut sim.world, cmd);
    let status = s.model.status.clone().unwrap_or_default();
    info!("quick loot: {cmd:?} on 0x{:08X} \"{}\": {status}", s.target, s.title);
    if s.kind == Kind::Pocket {
        if !a.taken.is_empty() || a.caught.is_some() {
            stolen.write(crate::pickpocket::Pickpocketed { victim: s.holder, taken: a.taken.clone(), given: Vec::new(), placed: Vec::new(), caught: a.caught });
        }
    } else if !a.taken.is_empty() {
        // Taking owned items is theft (`crate::crime`; a body's never is).
        taken.write(crate::crime::ItemsTaken { container: s.holder, items: a.taken.clone() });
    }
    // A body's dropped weapon taken: picked up from the world (`menu::model::Row::reference`).
    for id in &a.picked {
        info!("quick loot: picked up reference {id} lying beside 0x{:08X}", s.target);
        changed.write(RefChanged::new(id.0));
    }
    if cmd == Command::TakeAll && !a.taken.is_empty() {
        sounds.write(crate::ui_sounds::UiSound { menu: crate::ui_sounds::UiMenu::Container, event: crate::ui_sounds::UiEvent::TakeAll });
    }
    // What came into the inventory, as a script's AddItem says it (`crate::script_events`).
    if let Some(c) = corner.as_deref_mut() {
        const MANY: usize = 4;
        if a.taken.len() >= MANY {
            let total: i32 = a.taken.iter().map(|(_, n)| n).sum();
            c.push(format!("{} items added ({total} in all)", a.taken.len()).into());
        } else {
            for (item, n) in &a.taken {
                let name = crate::menu::model::display_name(forms, *item);
                c.push(if *n > 1 { format!("{name} ({n}) added") } else { format!("{name} added") }.into());
            }
        }
    }
    if a.caught.is_some() {
        info!("quick loot: caught on 0x{:08X} \"{}\": hidden", s.target, s.title);
        ql.closed_on = Some(s.target);
        ql.shown = None;
        // `HOLDS` stays set this frame: the E that was caught is not Activate's as well
        // (it would open the pocket); the next frame lets it go.
        return;
    }
    if let Err(e) = s.model.refresh(&mut sim.world) {
        warn!("quick loot: 0x{:08X} cannot be listed again: {e}", s.target);
    }
}

/// Out of play (a menu, dialogue, a loading screen) the list goes; it is opened again on
/// the target when play comes back, so what the full menu changed is read.
fn leave_play(mut ql: ResMut<QuickLoot>) {
    ql.shown = None;
    HOLDS.store(false, Ordering::Relaxed);
}

pub struct QuickLootPlugin;

// ---- drawing ----------------------------------------------------------------------------

#[derive(Component)]
struct Root;

#[derive(Component)]
struct Panel;

/// The game's raster faces, loaded once (the sharp ones are made from them as the Menu
/// Font row says, [`crate::pause::MenuFont::faces`]).
#[derive(Default)]
struct Kit {
    raster: Option<Faces>,
}

/// What the drawing shows: the words, and whether in the sharp font.
type Drawn = Option<(Spec, bool)>;

fn colour(ink: Ink, crime: bool) -> Color {
    let row = |full: bool| {
        let c = if crime { crate::reticle::crime_colour() } else { amber(255.0) };
        if full {
            c
        } else {
            c.with_alpha(0.72)
        }
    };
    match ink {
        Ink::Hud => amber(255.0),
        Ink::Dim => amber(255.0).with_alpha(0.6),
        Ink::Row(full) => row(full),
        Ink::Fill => row(true).with_alpha(0.16),
        Ink::Backing => Color::BLACK.with_alpha(0.45),
    }
}

fn spawn_panel(p: &mut ChildSpawnerCommands, laid: &Laid, faces: &Faces, crime: bool) {
    for b in &laid.bars {
        p.spawn((at(b.rect.min.x, b.rect.min.y, b.rect.width(), b.rect.height()), BackgroundColor(colour(b.ink, crime)), Pickable::IGNORE));
    }
    for piece in &laid.pieces {
        let f = face(faces, piece.which);
        let line = f.font.lines(&piece.text, None).into_iter().next().unwrap_or_default();
        set_line(p, f, &line, piece.x, piece.base, colour(piece.ink, crime));
    }
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn draw(
    mut commands: Commands,
    state: Res<State<PlayState>>,
    ql: Res<QuickLoot>,
    bindings: Option<Res<crate::bindings::Bindings>>,
    (menu_font, mut fonts, hidden): (Option<ResMut<crate::pause::MenuFont>>, Option<ResMut<Assets<bevy::text::Font>>>, Option<Res<crate::hud::HudHidden>>),
    (data, mut images, formats): (Option<Res<GameData>>, ResMut<Assets<Image>>, Option<Res<CompressedImageFormatSupport>>),
    mut kit: Local<Kit>,
    mut drawn: Local<Drawn>,
    mut root: Query<&mut Visibility, With<Root>>,
    mut panels: Query<(Entity, &mut Node), With<Panel>>,
    window: Query<&Window, With<bevy::window::PrimaryWindow>>,
) {
    let _hud = hud_units();
    let playing = *state.get() == PlayState::Playing;
    let shown = ql.shown.as_ref().filter(|_| playing && !hidden.is_some_and(|h| h.on));
    if kit.raster.is_none() {
        if shown.is_none() {
            return;
        }
        kit.raster = Some(Faces::load(data.as_deref(), &mut images, bc_supported(formats.as_deref())));
        commands
            .spawn((Root, Node { position_type: PositionType::Absolute, left: Val::Percent(50.0), top: Val::Percent(50.0), width: Val::Px(0.0), height: Val::Px(0.0), ..default() }, Visibility::Hidden, Pickable::IGNORE))
            .with_children(|r| {
                r.spawn((Panel, at(LEFT, TOP, 0.0, 0.0), Pickable::IGNORE));
            });
        return;
    }
    let Ok(mut vis) = root.single_mut() else { return };
    let want = if shown.is_some() { Visibility::Inherited } else { Visibility::Hidden };
    if *vis != want {
        *vis = want;
    }
    let Some(s) = shown else {
        *drawn = None;
        return;
    };
    let mut menu_font = menu_font;
    let sharp = menu_font.as_deref().is_some_and(crate::pause::MenuFont::drawn_sharp);
    let key = |c: Control| bindings.as_deref().map_or_else(|| "?)".into(), |b| b.prompt(c));
    let spec = Spec::of(s, Columns::chosen(s.kind == Kind::Pocket), key);
    let now = Some((spec, sharp));
    if *drawn == now {
        return;
    }
    let (spec, _) = now.clone().expect("set above");
    let raster = kit.raster.as_ref().expect("loaded above");
    let faces = match menu_font.as_deref_mut() {
        Some(m) => m.faces(raster, fonts.as_deref_mut()),
        None => raster.clone(),
    };
    let laid = layout(&spec, &faces);
    // Logged when the words or the font change, not as the selection moves.
    let new_words = drawn.as_ref().is_none_or(|(d, f)| d.title != spec.title || d.columns != spec.columns || *f != sharp || d.rows.len() != spec.rows.len());
    if new_words {
        let bad = overlaps(&laid, &faces);
        let font = if sharp { "Sharp" } else { "Original" };
        if bad.is_empty() {
            info!("quick loot: drawn \"{}\" ({} rows, columns {:?}, font {font}, {:.0} x {:.0} units): no overlaps", spec.title, spec.rows.len(), spec.columns, laid.size.x, laid.size.y);
        } else {
            warn!("quick loot: drawn \"{}\" (font {font}): overlaps: {}", spec.title, bad.join(", "));
        }
    }
    // Right of the crosshair, moved left where the view is too narrow for it.
    let half = window.single().map_or(480.0 * 16.0 / 9.0, |w| 480.0 * w.width() / w.height().max(1.0));
    let left = LEFT.min(half - EDGE - laid.size.x);
    for (e, mut node) in &mut panels {
        let place = crate::interface::u(left);
        if node.left != place {
            node.left = place;
        }
        commands.entity(e).despawn_related::<Children>();
        let crime = spec.crime;
        let (laid, faces) = (laid.clone(), faces.clone());
        commands.entity(e).with_children(|p| spawn_panel(p, &laid, &faces, crime));
    }
    *drawn = now;
}

impl Plugin for QuickLootPlugin {
    fn build(&self, app: &mut App) {
        use crate::settings::{add_setting, Setting};
        let group = crate::pause::GROUP_MENUS;
        add_setting(
            app,
            Setting {
                key: "quick-loot",
                group,
                label: "Quick loot",
                note: "On: a container, body or (sneaking) pocket under the crosshair lists its items without pausing. E takes, T takes all, R opens the menu.",
                switches: "",
                get: |_| ON.load(Ordering::Relaxed),
                set: |_, on| ON.store(on, Ordering::Relaxed),
                slider: None,
            },
        );
        add_setting(
            app,
            Setting {
                key: "quick-loot-weight",
                group,
                label: "Show weight",
                note: "Quick Loot lists each item's weight.",
                switches: "",
                get: |_| WEIGHT.load(Ordering::Relaxed),
                set: |_, on| WEIGHT.store(on, Ordering::Relaxed),
                slider: None,
            },
        );
        add_setting(
            app,
            Setting {
                key: "quick-loot-value",
                group,
                label: "Show value",
                note: "Quick Loot lists each item's value.",
                switches: "",
                get: |_| VALUE.load(Ordering::Relaxed),
                set: |_, on| VALUE.store(on, Ordering::Relaxed),
                slider: None,
            },
        );
        add_setting(
            app,
            Setting {
                key: "quick-loot-value-per-weight",
                group,
                label: "Show value per weight",
                note: "Quick Loot lists each item's value divided by its weight.",
                switches: "",
                get: |_| PER_WEIGHT.load(Ordering::Relaxed),
                set: |_, on| PER_WEIGHT.store(on, Ordering::Relaxed),
                slider: None,
            },
        );
        app.init_resource::<QuickLoot>()
            .add_systems(OnExit(PlayState::Playing), leave_play)
            .add_systems(Update, draw.in_set(crate::PlaySet::Ui));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(name: &str, count: i32, selected: bool) -> RowSpec {
        RowSpec { name: name.into(), count, weight: Some(3.0), value: Some(225), chance: None, selected }
    }

    fn spec(columns: Columns, rows: Vec<RowSpec>) -> Spec {
        let mut prompts = vec!["E) Take".to_string()];
        if !columns.chance {
            prompts.push("T) Take All".into());
        }
        prompts.push("R) Search".into());
        Spec { title: "Footlocker".into(), position: "3/8".into(), more_up: true, more_down: true, rows, columns, prompts, crime: false }
    }

    fn rows() -> Vec<RowSpec> {
        vec![
            row("10mm Pistol", 1, false),
            row("10mm Round", 240, false),
            row("Stimpak", 2, true),
            row("Purified Water", 3, false),
            row("Sunset Sarsaparilla Star Bottle Cap", 1057, false),
            row("Scrap Metal", 1, false),
        ]
    }

    /// The faces the list is drawn in: the stand-ins of the game's font 7, and the sharp
    /// font as the Menu Font row's Sharp makes them.
    fn both_fonts() -> Vec<(&'static str, Faces)> {
        let plain = Faces::plain();
        let mut out = vec![("Original", plain.clone())];
        if let Some(m) = crate::vector_font::metrics() {
            out.push(("Sharp", plain.sharp(m, Handle::default())));
        }
        out
    }

    const ALL: Columns = Columns { weight: true, value: true, per_weight: true, chance: false };

    #[test]
    fn nothing_overlaps_in_either_font_with_any_columns() {
        assert_eq!(both_fonts().len(), 2, "the sharp font's metrics read");
        for (font, faces) in both_fonts() {
            for bits in 0..16u8 {
                let c = Columns { weight: bits & 1 != 0, value: bits & 2 != 0, per_weight: bits & 4 != 0, chance: bits & 8 != 0 };
                let l = layout(&spec(c, rows()), &faces);
                assert_eq!(overlaps(&l, &faces), Vec::<String>::new(), "{font}, {c:?}");
                assert!(LEFT + l.size.x <= 480.0 * 16.0 / 9.0 - EDGE, "{font}, {c:?}: {} units wide stays in a 16:9 view's right half", l.size.x);
            }
            let empty = layout(&spec(Columns::default(), Vec::new()), &faces);
            assert_eq!(overlaps(&empty, &faces), Vec::<String>::new(), "{font}: an empty list");
            assert!(empty.pieces.iter().any(|p| p.text == EMPTY));
        }
    }

    #[test]
    fn counts_stand_right_without_columns_and_beside_the_name_with_them() {
        for (font, faces) in both_fonts() {
            let l = layout(&spec(Columns::default(), rows()), &faces);
            let counts: Vec<&Piece> = l.pieces.iter().filter(|p| p.text.starts_with('(')).collect();
            assert_eq!(counts.len(), 4, "{font}: the four stacks' counts");
            let ends: Vec<f32> = counts.iter().map(|p| p.x + p.width).collect();
            assert!(ends.iter().all(|e| (e - ends[0]).abs() < 0.01), "{font}: the counts right-aligned: {ends:?}");
            assert!(l.pieces.iter().any(|p| p.text == "Stimpak"), "{font}: the name alone");
            let with = layout(&spec(ALL, rows()), &faces);
            assert!(with.pieces.iter().any(|p| p.text == "Stimpak (2)"), "{font}: the count beside the name");
            assert!(!with.pieces.iter().any(|p| p.text == "(2)"), "{font}: no count column");
            for h in [WG, VAL, PER_WG] {
                let head = with.pieces.iter().find(|p| p.text == h).expect("the column's head");
                let cells: Vec<&Piece> = with.pieces.iter().filter(|p| p.which == Which::Entry && (p.x + p.width - head.x - head.width).abs() < 0.01).collect();
                assert_eq!(cells.len(), 6, "{font}: {h}'s figures right-aligned under it");
            }
        }
    }

    #[test]
    fn a_long_name_is_cut_and_a_pocket_has_no_take_all() {
        let faces = Faces::plain();
        let mut r = rows();
        r[0].name = "A very long item name that runs on and on past any list".into();
        let l = layout(&spec(Columns::default(), r), &faces);
        assert!(l.pieces.iter().any(|p| p.text.ends_with("...")), "cut with the mark");
        assert_eq!(overlaps(&l, &faces), Vec::<String>::new());
        let pocket = spec(Columns { chance: true, ..Columns::default() }, rows());
        assert!(!pocket.prompts.iter().any(|p| p.contains(TAKE_ALL)));
        let l = layout(&pocket, &faces);
        assert!(l.pieces.iter().any(|p| p.text == CHANCE));
    }

    #[test]
    fn value_per_weight() {
        assert_eq!(per_weight_text(Some(3.0), Some(225)), "75");
        assert_eq!(per_weight_text(Some(2.0), Some(5)), "2.5");
        assert_eq!(per_weight_text(None, Some(5)), "--");
        assert_eq!(per_weight_text(Some(1.0), None), "--");
    }
}
