//! Integration tests for replaced NVSE stubs: `UnforceAV`, `IsInCharGen`,
//! `GetStringHash`, and `HasKeywordCond`.

use crate::common::*;
use fnv_esm::{LoadOrder, Plugin, Schema};
use fnv_game::game::{menus, Game, GameConfig, RaceMenu, RunCtx};
use fnv_game::nvse_jip12::string_hash;
use fnv_game::GameData;
use fnv_script::vm::XVal;
use fnv_script::FunctionTable;
use fnv_sim::{FormId, FormStore, RefTable, SimConfig, World};
use std::path::Path;

const WEAP_ITEM: u32 = 0x0000_0C10;
const WEAP_REFR: u32 = 0x0000_0C11;
const NPC_BASE: u32 = 0x0000_0C12;
const NPC_REFR: u32 = 0x0000_0C13;
const CELL_MAIN: u32 = 0x0000_0C14;

struct Fixture {
    data: GameData,
    forms: FormStore,
    refs: RefTable,
}

fn fixture() -> Fixture {
    let player = record(b"NPC_", 0, 7, &[sub(b"EDID", z("Player")), sub(b"DATA", vec![0; 11])]);
    let npc = record(b"NPC_", 0, NPC_BASE, &[sub(b"EDID", z("OwnKeywordActor")), sub(b"DATA", vec![0; 11])]);
    let weap = record(b"WEAP", 0, WEAP_ITEM, &[sub(b"EDID", z("OwnKeywordWeapon"))]);
    let weap_ref = record(b"REFR", 0x400, WEAP_REFR, &[sub(b"EDID", z("OwnKeywordWeaponRef")), sub(b"NAME", WEAP_ITEM.to_le_bytes().to_vec()), sub(b"DATA", vec![0; 24])]);
    let npc_ref = record(b"ACHR", 0x400, NPC_REFR, &[sub(b"EDID", z("OwnKeywordActorRef")), sub(b"NAME", NPC_BASE.to_le_bytes().to_vec()), sub(b"DATA", vec![0; 24])]);
    let cell = record(b"CELL", 0, CELL_MAIN, &[sub(b"EDID", z("OwnStubsCell")), sub(b"DATA", vec![1])]);
    let placed = group(CELL_MAIN.to_le_bytes(), 6, &group(CELL_MAIN.to_le_bytes(), 9, &[weap_ref, npc_ref].concat()));

    let mut bytes = tes4();
    bytes.extend(top(b"NPC_", &[player, npc].concat()));
    bytes.extend(top(b"WEAP", &weap));
    bytes.extend(top(b"CELL", &group(0i32.to_le_bytes(), 2, &group(0i32.to_le_bytes(), 3, &[cell, placed].concat()))));

    let plugins = vec![Plugin::parse(&bytes, "OwnedNvseStubs.esm".into()).unwrap()];
    let lo = LoadOrder::new(&plugins).unwrap();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schemas");
    let schema = Schema::load_dir(root.join("fnv")).unwrap();
    let functions = root.join("script/functions.json");
    let json: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&functions).unwrap()).unwrap();
    let forms = FormStore::build(&lo, &schema);
    let refs = RefTable::build(&lo, &schema);
    let data = GameData::build(&lo, &schema, FunctionTable::load(&functions).unwrap(), &json, &refs);
    Fixture { data, forms, refs }
}

fn game(f: &Fixture) -> Game<'_> {
    Game::new(&f.data, World::new(&f.forms, &f.refs, SimConfig::default()), GameConfig { strict: true, ..Default::default() })
}

fn q(g: &mut Game, body: &str) -> XVal {
    g.run_query(&format!("Begin Function {{}}\n{body}\nEnd"), RunCtx::default()).unwrap()
}

fn n(g: &mut Game, expr: &str) -> f64 {
    match q(g, &format!("SetFunctionValue ({expr})")) {
        XVal::Num(x) => x,
        other => panic!("expected Num for `{expr}`, got {other:?}"),
    }
}

fn call(g: &mut Game, name: &str, subject: Option<FormId>, args: &[XVal]) -> XVal {
    fnv_game::nvse::call(g, &RunCtx::default(), name, subject, args).unwrap().unwrap()
}

#[test]
fn unforce_av_reverses_force_av_and_preserves_mod_av_base_and_damage() {
    let f = fixture();
    let mut g = game(&f);

    let hp0 = n(&mut g, "player.GetActorValue Health");
    // Apply ModAV (+20) and DamageAV (5): current should be hp0 + 15.
    q(&mut g, "player.ModActorValue Health 20\nplayer.DamageActorValue Health 5");
    assert_eq!(n(&mut g, "player.GetActorValue Health"), hp0 + 15.0);

    // UnforceAV without a prior ForceAV is a no-op (ModAV and damage stay).
    q(&mut g, "player.UnforceAV Health");
    assert_eq!(n(&mut g, "player.GetActorValue Health"), hp0 + 15.0);

    // ForceAV twice (plain ForceAV then ForceActorValueAlt), then UnforceAV restores hp0 + 15.
    q(&mut g, "player.ForceActorValue Health 40\nplayer.ForceActorValueAlt 16 75");
    assert_eq!(n(&mut g, "player.GetActorValue Health"), 75.0);
    q(&mut g, "player.UnforceAV Health");
    assert_eq!(n(&mut g, "player.GetActorValue Health"), hp0 + 15.0);

    // Second UnforceAV is idempotent.
    q(&mut g, "player.UnforceAV Health");
    assert_eq!(n(&mut g, "player.GetActorValue Health"), hp0 + 15.0);

    // Works on SPECIAL attributes (synced through World) and on non-player actors too.
    let npc_str = n(&mut g, "OwnKeywordActorRef.GetActorValue Strength");
    q(&mut g, "OwnKeywordActorRef.ForceActorValue Strength 9");
    assert_eq!(n(&mut g, "OwnKeywordActorRef.GetActorValue Strength"), 9.0);
    q(&mut g, "OwnKeywordActorRef.UnforceAV Strength");
    assert_eq!(n(&mut g, "OwnKeywordActorRef.GetActorValue Strength"), npc_str);
}

#[test]
fn is_in_chargen_tracks_set_in_chargen_and_chargen_menus() {
    let f = fixture();
    let mut g = game(&f);

    assert!(!g.is_in_chargen());
    assert_eq!(n(&mut g, "IsInCharGen"), 0.0);

    // SetInCharGen 1 / 0 directly toggles IsInCharGen.
    q(&mut g, "SetInCharGen 1");
    assert!(g.is_in_chargen());
    assert_eq!(n(&mut g, "IsInCharGen"), 1.0);
    q(&mut g, "SetInCharGen 0");
    assert!(!g.is_in_chargen());
    assert_eq!(n(&mut g, "IsInCharGen"), 0.0);

    // Open character-creation menus also count, whereas barber/surgeon/barter/text-input do not.
    for code in [menus::TAG_SKILLS, menus::LOVE_TESTER, menus::TRAITS, menus::NAME] {
        g.menu = Some(code);
        assert_eq!(n(&mut g, "IsInCharGen"), 1.0, "menu {code} should report IsInCharGen = 1");
    }
    g.menu = Some(menus::RACE);
    g.race_menu = RaceMenu::Creator;
    assert_eq!(n(&mut g, "IsInCharGen"), 1.0);
    g.race_menu = RaceMenu::Barber;
    assert_eq!(n(&mut g, "IsInCharGen"), 0.0);
    g.race_menu = RaceMenu::Surgeon;
    assert_eq!(n(&mut g, "IsInCharGen"), 0.0);

    g.menu = Some(menus::BARTER);
    assert_eq!(n(&mut g, "IsInCharGen"), 0.0);
    g.menu = None;
    assert_eq!(n(&mut g, "IsInCharGen"), 0.0);
}

#[test]
fn get_string_hash_is_deterministic_signed_32bit_and_zero_for_empty() {
    let f = fixture();
    let mut g = game(&f);

    assert_eq!(n(&mut g, "GetStringHash"), 0.0);
    assert_eq!(n(&mut g, "GetStringHash \"\""), 0.0);
    assert_eq!(string_hash(""), 0);

    let h1 = n(&mut g, "GetStringHash \"NVForever\"");
    let h2 = n(&mut g, "GetStringHash \"NVForever\"");
    let h3 = n(&mut g, "GetStringHash \"OtherString\"");
    assert_eq!(h1, f64::from(string_hash("NVForever")));
    assert_eq!(h1, h2);
    assert_eq!(call(&mut g, "gethash", None, &[XVal::Str("NVForever".into())]), XVal::Num(h1));
    assert_ne!(h1, 0.0);
    assert_ne!(h1, h3);
    assert!((f64::from(i32::MIN)..=f64::from(i32::MAX)).contains(&h1));
    assert_eq!(h1.fract(), 0.0);
}

#[test]
fn has_keyword_cond_shares_keyword_table_with_has_keyword() {
    let f = fixture();
    let mut g = game(&f);

    let weap = FormId(WEAP_ITEM);
    let weap_ref = FormId(WEAP_REFR);
    let actor_ref = FormId(NPC_REFR);

    assert_eq!(call(&mut g, "haskeywordcond", Some(weap_ref), &[XVal::Str("OwnedTag".into())]), XVal::Num(0.0));
    assert_eq!(call(&mut g, "haskeywordcond", None, &[XVal::Ref(weap.0), XVal::Str("OwnedTag".into())]), XVal::Num(0.0));

    // Assign keyword on base weapon and test both calling conventions, case-insensitivity,
    // reference-to-base lookup, and TempCloneForm inheritance.
    q(&mut g, "AssignKeyword OwnKeywordWeapon \"OwnedTag\"");
    assert!(g.has_keyword(weap, "ownedtag"));
    assert!(g.has_keyword(weap_ref, "OWNEDTAG"));
    assert_eq!(call(&mut g, "haskeywordcond", Some(weap_ref), &[XVal::Str("ownedtag".into())]), XVal::Num(1.0));
    assert_eq!(call(&mut g, "haskeywordcond", None, &[XVal::Ref(weap.0), XVal::Str("OWNEDTAG".into())]), XVal::Num(1.0));
    assert_eq!(call(&mut g, "haskeywordcond", None, &[XVal::Ref(weap_ref.0), XVal::Str("OwnedTag".into())]), XVal::Num(1.0));
    assert_eq!(call(&mut g, "haskeywordcond", Some(actor_ref), &[XVal::Str("OwnedTag".into())]), XVal::Num(0.0));
    assert_eq!(call(&mut g, "haskeywordcond", Some(weap_ref), &[XVal::Str(String::new())]), XVal::Num(0.0));
    assert_eq!(call(&mut g, "haskeywordcond", None, &[]), XVal::Num(0.0));

    // TempCloneForm of keyworded weapon inherits original's keywords.
    let XVal::Ref(clone_raw) = call(&mut g, "tempcloneform", None, &[XVal::Ref(weap.0), XVal::Num(1.0)]) else {
        panic!("expected weapon clone");
    };
    assert_eq!(call(&mut g, "haskeywordcond", None, &[XVal::Ref(clone_raw), XVal::Str("OwnedTag".into())]), XVal::Num(1.0));

    // Removing the keyword clears both HasKeyword and HasKeywordCond.
    call(&mut g, "removekeyword", None, &[XVal::Ref(weap.0), XVal::Str("ownedtag".into())]);
    assert_eq!(n(&mut g, "HasKeyword OwnKeywordWeapon \"OwnedTag\""), 0.0);
    assert_eq!(call(&mut g, "haskeywordcond", Some(weap_ref), &[XVal::Str("OwnedTag".into())]), XVal::Num(0.0));
}
