//! Leveled spawns' placeholder creatures (docs/spec-notes/dead-on-spawn.md §4.1, #3228,
//! #3139): a placed creature whose base takes its script, stats and base data from a
//! leveled list (template flags "Script", "Stats", "Base Data") runs the script of the
//! list's pick. A placeholder pick (Health 0, Respawn, Is Ghost, a script whose OnLoad
//! kills it and sets it destroyed) starts dead, comes back alive at a cell reset (its
//! Respawn flag) and its OnLoad kills it again at once. On a synthetic plugin (no game
//! data); the last test reads the master when `FNVLAB_DATA_DIR` is set.

use crate::common;

use common::*;
use fnv_esm::{LoadOrder, Plugin, Schema};
use fnv_game::game::{event, Game, GameConfig};
use fnv_game::GameData;
use fnv_script::FunctionTable;
use fnv_sim::{FormId, FormStore, RefTable, SimConfig, World};
use serde_json::Value as Json;
use std::path::{Path, PathBuf};

/// The placeholder: Health 0, Respawn, Is Ghost, the script below.
const PLACEHOLDER: u32 = 0x840;
/// The leveled list whose only entry is the placeholder (the pick is never in doubt).
const LIST: u32 = 0x841;
/// The placed base: Script, Stats and Base Data from the list.
const SPAWN: u32 = 0x842;
/// A creature with its own script and no template, for comparison.
const OWN_SCRIPTED: u32 = 0x843;
/// A spawn base whose list holds only an unscripted creature.
const PLAIN: u32 = 0x844;
const PLAIN_LIST: u32 = 0x845;
const PLAIN_SPAWN: u32 = 0x846;
const PLACEHOLDER_SCRIPT: u32 = 0x850;
const SPAWN_REF: u32 = 0x300;
const OWN_SCRIPTED_REF: u32 = 0x301;
const PLAIN_SPAWN_REF: u32 = 0x302;
const CELL_A: u32 = 0x200;
const CELL_B: u32 = 0x201;
const KILL_ACTOR: u16 = 0x108B;
const SET_DESTROYED: u16 = 0x10CC;
/// ACBS flags: Respawn (bit 3), Is Ghost (bit 29) [documented: xEdit; schema gameplay.json].
const RESPAWN: u32 = 0x8;
const IS_GHOST: u32 = 0x2000_0000;
/// ACBS template flags: Stats, Base Data, Script.
const STATS: u16 = 0x2;
const BASE_DATA: u16 = 0x80;
const SCRIPT: u16 = 0x200;

fn acbs(flags: u32, template_flags: u16) -> fnv_esm::raw::Subrecord {
    let mut d = flags.to_le_bytes().to_vec();
    d.extend([0u8; 16]);
    d.extend(0i16.to_le_bytes());
    d.extend(template_flags.to_le_bytes());
    sub(b"ACBS", d)
}

/// CREA DATA: type, combat, magic and stealth skill, health, unused, damage, attributes.
fn crea_data(health: i16) -> fnv_esm::raw::Subrecord {
    sub(b"DATA", [vec![0u8, 50, 50, 50], health.to_le_bytes().to_vec(), vec![0; 2], 0i16.to_le_bytes().to_vec(), vec![1; 7]].concat())
}

fn list(fid: u32, edid: &str, entry: u32) -> Vec<u8> {
    let lvlo = [1i16.to_le_bytes().to_vec(), vec![0; 2], entry.to_le_bytes().to_vec(), 1i16.to_le_bytes().to_vec(), vec![0; 2]].concat();
    record(b"LVLC", 0, fid, &[sub(b"EDID", z(edid)), sub(b"LVLD", vec![0]), sub(b"LVLF", vec![0]), sub(b"LVLO", lvlo)])
}

fn glob(fid: u32, edid: &str, v: f32) -> Vec<u8> {
    record(b"GLOB", 0, fid, &[sub(b"EDID", z(edid)), sub(b"FNAM", vec![b'f']), sub(b"FLTV", v.to_le_bytes().to_vec())])
}

fn placed(fid: u32, base: u32) -> Vec<u8> {
    record(b"ACRE", 0, fid, &[sub(b"NAME", base.to_le_bytes().to_vec()), sub(b"DATA", vec![0; 24])])
}

fn cell(fid: u32, edid: &str, refs: &[u8]) -> Vec<u8> {
    let c = record(b"CELL", 0, fid, &[sub(b"EDID", z(edid))]);
    [c, group(fid.to_le_bytes(), 6, &group(fid.to_le_bytes(), 9, refs))].concat()
}

fn plugin_bytes() -> Vec<u8> {
    // Begin OnLoad: Kill, SetDestroyed 1, as the masters' placeholder script does.
    let on_load = [st(KILL_ACTOR, &le16(0)), st(SET_DESTROYED, &[le16(1), vec![b'n'], 1i32.to_le_bytes().to_vec()].concat())].concat();
    let scpt = record(b"SCPT", 0, PLACEHOLDER_SCRIPT, &[vec![sub(b"EDID", z("PlaceholderScript"))], unit(0, &script(&[(event::ON_LOAD, None, on_load)]), &[], &[])].concat());
    let scri = |s: u32| sub(b"SCRI", s.to_le_bytes().to_vec());
    let tplt = |t: u32| sub(b"TPLT", t.to_le_bytes().to_vec());
    let creatures = [
        record(b"CREA", 0, PLACEHOLDER, &[sub(b"EDID", z("TestVoidPlaceholder")), acbs(RESPAWN | IS_GHOST, 0), scri(PLACEHOLDER_SCRIPT), crea_data(0)]),
        record(b"CREA", 0, SPAWN, &[sub(b"EDID", z("TestSpawnTier1Small75")), acbs(0, STATS | BASE_DATA | SCRIPT), tplt(LIST), crea_data(50)]),
        record(b"CREA", 0, OWN_SCRIPTED, &[sub(b"EDID", z("TestScriptedCritter")), acbs(0, 0), scri(PLACEHOLDER_SCRIPT), crea_data(0)]),
        record(b"CREA", 0, PLAIN, &[sub(b"EDID", z("TestCalf")), acbs(RESPAWN, 0), crea_data(20)]),
        record(b"CREA", 0, PLAIN_SPAWN, &[sub(b"EDID", z("TestSpawnCalf")), acbs(0, STATS | BASE_DATA | SCRIPT), tplt(PLAIN_LIST), crea_data(20)]),
    ]
    .concat();
    let lists = [list(LIST, "TestEncSmall75", PLACEHOLDER), list(PLAIN_LIST, "TestEncCalf", PLAIN)].concat();
    let globs = [
        glob(0x900, "GameYear", 2281.0),
        glob(0x901, "GameMonth", 9.0),
        glob(0x902, "GameDay", 19.0),
        glob(0x903, "GameHour", 9.0),
        glob(0x904, "GameDaysPassed", 1.0),
    ]
    .concat();
    let refs = [placed(SPAWN_REF, SPAWN), placed(OWN_SCRIPTED_REF, OWN_SCRIPTED), placed(PLAIN_SPAWN_REF, PLAIN_SPAWN)].concat();
    let cells = [cell(CELL_A, "TestHerdCell", &refs), cell(CELL_B, "TestAwayCell", &[])].concat();
    [
        tes4(),
        top(b"GLOB", &globs),
        top(b"SCPT", &scpt),
        top(b"CREA", &creatures),
        top(b"LVLC", &lists),
        top(b"CELL", &group(0i32.to_le_bytes(), 2, &group(0i32.to_le_bytes(), 3, &cells))),
    ]
    .concat()
}

struct Fixture {
    data: GameData,
    forms: FormStore,
    refs: RefTable,
}

fn build(plugins: &[Plugin]) -> Fixture {
    let lo = LoadOrder::new(plugins).unwrap();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let schema = Schema::load_dir(root.join("schemas/fnv")).unwrap();
    let p = root.join("schemas/script/functions.json");
    let j: Json = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
    let table = FunctionTable::load(&p).unwrap();
    let forms = FormStore::build(&lo, &schema);
    let refs = RefTable::build(&lo, &schema);
    let data = GameData::build(&lo, &schema, table, &j, &refs);
    Fixture { data, forms, refs }
}

fn fixture() -> Fixture {
    build(&[Plugin::parse(&plugin_bytes(), "Placeholders.esm".into()).unwrap()])
}

fn game(f: &Fixture) -> Game<'_> {
    let world = World::new(&f.forms, &f.refs, SimConfig::default());
    Game::new(&f.data, world, GameConfig::default())
}

fn destroyed(g: &Game, r: u32) -> bool {
    g.extra.get(&FormId(r)).is_some_and(|e| e.destroyed)
}

/// A reference whose base gets its script only through a leveled list's pick counts as
/// scripted (its OnLoad runs at a cell load); one whose list can give no script does not.
#[test]
fn a_script_from_the_leveled_pick_makes_the_reference_scripted() {
    let f = fixture();
    assert!(f.data.base_may_run_script(FormId(SPAWN)), "the list's placeholder has a script");
    assert!(f.data.base_may_run_script(FormId(OWN_SCRIPTED)));
    assert!(!f.data.base_may_run_script(FormId(PLAIN_SPAWN)), "the list's calf has none");
    assert_eq!(f.data.scripted_by_cell.get(&FormId(CELL_A)), Some(&vec![FormId(SPAWN_REF), FormId(OWN_SCRIPTED_REF)]));
    let g = game(&f);
    assert_eq!(g.ref_script(FormId(SPAWN_REF)), Some(FormId(PLACEHOLDER_SCRIPT)));
    assert_eq!(g.ref_script(FormId(PLAIN_SPAWN_REF)), None);
}

/// The placeholder is laid out dead by a new game (Health 0 from the pick), comes back
/// alive at a cell reset (Respawn from the pick, "Base Data") and is killed and set
/// destroyed by its OnLoad in the same load: never alive when anything else runs (#3228:
/// before, its OnLoad never ran and it stayed alive at health 1, fought and hit the
/// player).
#[test]
fn a_placeholder_brought_back_by_a_cell_reset_is_killed_by_its_on_load() {
    let f = fixture();
    let mut g = game(&f);
    let spawn = FormId(SPAWN_REF);
    assert!(g.starts_dead(spawn), "Health 0 through the pick");
    assert!(g.respawns(spawn), "Respawn through the pick (Base Data)");
    assert!(g.is_ghost(spawn), "Is Ghost through the pick (Base Data)");
    g.new_game();
    g.player_entered(Some(FormId(CELL_A)));
    assert!(g.is_dead(spawn));
    assert!(destroyed(&g, SPAWN_REF), "its OnLoad ran at the first load");
    // The calf of the plain spawn is killed, to come back alive with it.
    assert!(g.kill(FormId(PLAIN_SPAWN_REF), Some(FormId::PLAYER_REF)));

    g.player_entered(Some(FormId(CELL_B)));
    g.world.advance_hours(72.0).unwrap();
    g.player_entered(Some(FormId(CELL_A)));
    let resets = g.take_cell_resets();
    assert_eq!(resets.len(), 1);
    assert_eq!(resets[0].respawned, vec![spawn, FormId(PLAIN_SPAWN_REF)], "both were recreated");
    assert!(g.is_dead(spawn), "the recreated placeholder's OnLoad killed it again");
    assert!(destroyed(&g, SPAWN_REF), "and set it destroyed (the reset had cleared that)");
    assert!(g.extra[&spawn].killer.is_none());
    assert!(!g.is_dead(FormId(PLAIN_SPAWN_REF)), "a real creature of a spawn stays alive");
}

fn data_dir() -> Option<PathBuf> {
    let Some(dir) = std::env::var_os("FNVLAB_DATA_DIR").map(PathBuf::from) else {
        eprintln!("skipped: FNVLAB_DATA_DIR is not set (no game data)");
        return None;
    };
    if !dir.join("FalloutNV.esm").is_file() {
        eprintln!("skipped: FalloutNV.esm is not in {}", dir.display());
        return None;
    }
    Some(dir)
}

/// The masters: the Camp Golf herd of #3228 (0x00168D44–0x00168D51, bighorner and
/// bloatfly spawns whose lists hold `VCrILikeOblivionChancedSpawns`) is scripted through
/// its picks. Prints how many references count as scripted only through a template.
#[test]
fn the_camp_golf_herd_runs_its_placeholder_script() {
    let Some(dir) = data_dir() else { return };
    let f = build(&[Plugin::open(dir.join("FalloutNV.esm")).unwrap()]);
    let placeholder = f.data.by_edid("VCrILikeOblivionChancedSpawns").expect("the placeholder");
    let herd: Vec<FormId> = (0x0016_8D44..=0x0016_8D51)
        .map(FormId)
        .filter(|&r| f.refs.get(r).is_some_and(|p| f.data.template_sources_any_pick(p.base, |a| a.template_script).contains(&placeholder)))
        .collect();
    assert!(herd.len() >= 4, "the herd's spawns that may be the placeholder: {herd:?}");
    let scripted: std::collections::BTreeSet<FormId> = f.data.scripted_by_cell.values().flatten().copied().collect();
    for r in &herd {
        assert!(scripted.contains(r), "{r} may be the placeholder: its OnLoad must run");
    }
    let through: Vec<FormId> = scripted.iter().copied().filter(|&r| f.refs.get(r).is_some_and(|p| f.data.script_of_base(p.base).is_none())).collect();
    eprintln!("{} scripted references, {} of them through a template's script", scripted.len(), through.len());
    // By the scripts their picks may run (a reference may count under several).
    let mut by_script: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for r in &through {
        let base = f.refs.get(*r).unwrap().base;
        let scripts: std::collections::BTreeSet<FormId> = f.data.template_sources_any_pick(base, |a| a.template_script).into_iter().filter_map(|s| f.data.script_of_base(s)).collect();
        for s in scripts {
            let game_mode = f.data.script(s).is_some_and(|u| u.has_block(event::GAME_MODE));
            *by_script.entry(format!("{}{}", f.data.name(s), if game_mode { " (GameMode)" } else { "" })).or_default() += 1;
        }
    }
    for (s, n) in &by_script {
        eprintln!("  {n:5} {s}");
    }
}
