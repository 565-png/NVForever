//! Authored portable player-command consumers; no game data or native parity claim.
use crate::common::*;
use fnv_esm::{LoadOrder, Plugin, Schema};
use fnv_game::{actor_values as av, functions, game::{GameSnapshot, RunCtx}, trace::EventKind, Game, GameConfig, GameData};
use fnv_script::{vm::XVal, FunctionTable};
use fnv_sim::{FormId, FormStore, RefTable, SimConfig, World};

const REP: FormId = FormId(0x850);
const OTHER_REP: FormId = FormId(0x851);
const NPC: FormId = FormId(0x300);
const ROOM: FormId = FormId(0x200);
struct Fixture { data: GameData, forms: FormStore, refs: RefTable }

fn fixture() -> Fixture {
    let npc = |id, name, strength| record(b"NPC_", 0, id, &[
        sub(b"EDID", z(name)),
        sub(b"DATA", [100i32.to_le_bytes().to_vec(), vec![strength, 5, 5, 5, 5, 5, 5]].concat()),
    ]);
    let rep = |id, name| record(b"REPU", 0, id, &[sub(b"EDID", z(name)), sub(b"DATA", 20f32.to_le_bytes().to_vec())]);
    let placed = record(b"ACHR", 0, NPC.0, &[sub(b"EDID", z("ProgressionNeighborRef")), sub(b"NAME", 0x800u32.to_le_bytes().to_vec()), sub(b"DATA", vec![0; 24])]);
    let cell = record(b"CELL", 0, ROOM.0, &[sub(b"EDID", z("ProgressionRoom"))]);
    let children = group(ROOM.0.to_le_bytes(), 6, &group(ROOM.0.to_le_bytes(), 9, &placed));
    let bytes = [tes4(),
        top(b"NPC_", &[npc(7, "Player", 5), npc(0x800, "ProgressionNeighbor", 8)].concat()),
        top(b"REPU", &[rep(REP.0, "ProgressionRep"), rep(OTHER_REP.0, "ProgressionOtherRep")].concat()),
        top(b"CELL", &group(0i32.to_le_bytes(), 2, &group(0i32.to_le_bytes(), 3, &[cell, children].concat()))),
    ].concat();
    let plugins = vec![Plugin::parse(&bytes, "PlayerSourceControls.esm".into()).unwrap()];
    let order = LoadOrder::new(&plugins).unwrap();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schemas");
    let schema = Schema::load_dir(root.join("fnv")).unwrap();
    let path = root.join("script/functions.json");
    let catalogue = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let forms = FormStore::build(&order, &schema);
    let refs = RefTable::build(&order, &schema);
    let data = GameData::build(&order, &schema, FunctionTable::load(&path).unwrap(), &catalogue, &refs);
    Fixture { data, forms, refs }
}

fn game(f: &Fixture) -> Game<'_> {
    let mut g = Game::new(&f.data, World::new(&f.forms, &f.refs, SimConfig::default()), GameConfig { strict: true, ..Default::default() });
    g.player_entered(Some(ROOM));
    g
}

// run_query compiles source to SCDA and decodes it before running the VM. A named
// function's neutral result alone is insufficient: reject skips and unread arguments.
fn query(g: &mut Game<'_>, body: &str) -> Result<XVal, String> {
    g.run_query(&format!("Begin Function {{}}\n{body}\nEnd\n"), RunCtx::default())
}
fn success(g: &mut Game<'_>, body: &str) -> XVal {
    let before = g.events.len();
    let result = query(g, body).unwrap_or_else(|e| panic!("{body}: {e}"));
    assert!(!g.events[before..].iter().any(|e| matches!(e.kind,
        EventKind::Stub { .. } | EventKind::Unhandled { .. } | EventKind::Skipped { .. }
        | EventKind::CallFailed { .. } | EventKind::ScriptError { .. } | EventKind::ArgUnread { .. }
    )), "{body}: {:?}", &g.events[before..]);
    result
}
fn invalid(g: &mut Game<'_>, body: &str) {
    let before = g.events.len();
    let result = query(g, body);
    assert!(result.is_err() || g.events[before..].iter().any(|e| matches!(e.kind, EventKind::CallFailed { .. } | EventKind::ScriptError { .. })), "invalid call silently succeeded: {body}");
}

#[test]
fn source_advance_and_alias_reach_the_existing_menu_without_awarding_xp() {
    let f = fixture(); let mut g = game(&f);
    g.player.xp = 123.5;
    let level = g.player_level();
    let menus = g.events.iter().filter(|e| matches!(e.kind, EventKind::MenuOpened { menu: fnv_game::game::menus::LEVEL_UP })).count();
    success(&mut g, "AdvancePCLevel");
    assert_eq!(g.player_level(), level + 1);
    assert_eq!(g.menu, Some(fnv_game::game::menus::LEVEL_UP));
    assert_eq!(g.player.xp, 123.5);
    success(&mut g, "AdvLevel");
    assert_eq!(g.player_level(), level + 2);
    assert_eq!(g.player.xp, 123.5);
    assert_eq!(g.events.iter().filter(|e| matches!(e.kind, EventKind::MenuOpened { menu: fnv_game::game::menus::LEVEL_UP })).count(), menus + 1, "advancing while already open retains the existing menu event policy");
}

#[test]
fn attributes_keep_modifier_damage_and_carry_consumers_and_tell_handlers() {
    let f = fixture(); let mut g = game(&f);
    success(&mut g, "player.ModAV Strength 2\nplayer.DamageAV Strength 1");
    success(&mut g, "SetOnActorValueChangeEventHandler 1 (Begin Function {int code, float before, float after}\nplayer.AuxVarSetFlt \"*progressionCode\" code 0\nplayer.AuxVarSetFlt \"*progressionChange\" (after - before) 0\nEnd) 0");
    let before = functions::actor_value_parts(&mut g, FormId::PLAYER_REF, av::STRENGTH).unwrap();
    let carry = g.world.encumbrance(FormId::PLAYER_REF).unwrap().capacity;
    let neighbor = functions::actor_value(&mut g, NPC, av::STRENGTH).unwrap();
    success(&mut g, "ProgressionNeighborRef.ModPCAttribute Strength 1");
    assert_eq!(functions::actor_value_parts(&mut g, FormId::PLAYER_REF, av::STRENGTH).unwrap(), [before[0] + 1.0, before[1], before[2]]);
    assert_eq!(functions::actor_value(&mut g, NPC, av::STRENGTH).unwrap(), neighbor);
    assert!(g.world.encumbrance(FormId::PLAYER_REF).unwrap().capacity > carry);
    assert!(g.world.changed().any(|(id, _)| id == FormId::PLAYER_REF));
    assert_eq!(success(&mut g, "SetFunctionValue (player.AuxVarGetFlt \"*progressionCode\" 0)"), XVal::Num(f64::from(av::STRENGTH)));
    assert_eq!(success(&mut g, "SetFunctionValue (player.AuxVarGetFlt \"*progressionChange\" 0)"), XVal::Num(1.0));
    success(&mut g, "ModPCAttribute Strength -1");
    assert_eq!(functions::actor_value_parts(&mut g, FormId::PLAYER_REF, av::STRENGTH).unwrap(), before);
}

#[test]
fn exact_removal_keeps_fractions_other_axis_other_reputation_and_events() {
    let f = fixture(); let mut g = game(&f);
    success(&mut g, "SetReputation ProgressionRep 0 3.75\nSetReputation ProgressionRep 1 11.5\nSetReputation ProgressionOtherRep 1 9.25");
    let start = g.events.len();
    success(&mut g, "RemoveReputationExact ProgressionRep 0 0.5");
    assert_eq!(g.player.reputation[&REP], [3.25, 11.5]);
    assert_eq!(g.player.reputation[&OTHER_REP], [0.0, 9.25]);
    assert!(g.events[start..].iter().any(|e| matches!(e.kind, EventKind::ReputationChanged { reputation, axis: 0, value } if reputation == REP && value == 3.25)));
    success(&mut g, "RemoveReputationExact ProgressionRep 1 100.25");
    assert_eq!(g.player.reputation[&REP], [3.25, 0.0]);
    success(&mut g, "SetReputation ProgressionRep 1 30\nRemoveReputationExact ProgressionRep 1 0.25");
    assert_eq!(g.player.reputation[&REP][1], 29.75, "loss keeps a value above the scale");
    assert_eq!(success(&mut g, "SetFunctionValue (GetReputation ProgressionRep 1)"), XVal::Num(29.75));
}

#[test]
fn bad_arguments_and_explicit_empty_receivers_do_not_mutate_player_state() {
    let f = fixture(); let mut g = game(&f);
    let level = g.player_level();
    let strength = functions::actor_value_parts(&mut g, FormId::PLAYER_REF, av::STRENGTH).unwrap();
    for body in ["ModPCAttribute Repair 1", "ModPCAttribute NoSuchAttribute 1", "ModPCAttribute Strength \"bad\"", "RemoveReputationExact Player 0 1", "AdvancePCLevel 1",
        "ref receiver\nreceiver.AdvancePCLevel", "ref receiver\nreceiver.ModPCAttribute Strength 1", "ref receiver\nreceiver.RemoveReputationExact ProgressionRep 0 1"] {
        invalid(&mut g, body);
        assert_eq!(g.player_level(), level, "{body}");
        assert_eq!(functions::actor_value_parts(&mut g, FormId::PLAYER_REF, av::STRENGTH).unwrap(), strength, "{body}");
        assert!(g.player.reputation.is_empty(), "{body}");
    }
}

#[test]
fn exact_removal_invalid_axis_retains_the_existing_script_return_policy() {
    let f = fixture(); let mut g = game(&f);
    success(&mut g, "SetReputation ProgressionRep 0 2.5");
    let _ = query(&mut g, "RemoveReputationExact ProgressionRep 2 0.5\nModPCAttribute Strength 1");
    assert_eq!(g.player.reputation[&REP], [2.5, 0.0]);
    assert_eq!(functions::actor_value_parts(&mut g, FormId::PLAYER_REF, av::STRENGTH).unwrap(), [5.0, 0.0, 0.0]);
}

#[test]
fn player_changes_roundtrip_through_the_real_snapshot_and_resume_source_calls() {
    let f = fixture(); let mut g = game(&f);
    success(&mut g, "ModPCAttribute Strength 2\nSetReputation ProgressionRep 1 7.75\nRemoveReputationExact ProgressionRep 1 0.5\nAdvancePCLevel");
    let level = g.player_level();
    let snapshot: GameSnapshot = serde_json::from_str(&serde_json::to_string(&g.checked_snapshot().unwrap()).unwrap()).unwrap();
    let world = World::restore(&f.forms, &f.refs, &snapshot.sim).unwrap();
    let mut restored = Game::restore(&f.data, world, g.cfg, &snapshot).unwrap();
    assert_eq!(restored.player_level(), level);
    assert_eq!(functions::actor_value_parts(&mut restored, FormId::PLAYER_REF, av::STRENGTH).unwrap(), [7.0, 0.0, 0.0]);
    assert_eq!(restored.player.reputation[&REP], [0.0, 7.25]);
    success(&mut restored, "ModPCAttribute Strength -1\nRemoveReputationExact ProgressionRep 1 0.25");
    assert_eq!(restored.player.reputation[&REP], [0.0, 7.0]);
    assert_eq!(functions::actor_value_parts(&mut restored, FormId::PLAYER_REF, av::STRENGTH).unwrap(), [6.0, 0.0, 0.0]);
}

#[test]
fn focused_vanilla_census_keeps_source_support_separate_from_native_encoding() {
    let f = fixture(); let mut g = game(&f);
    for (name, source) in [("AdvancePCLevel", "AdvancePCLevel"), ("ModPCAttribute", "ModPCAttribute Strength 0"), ("RemoveReputationExact", "RemoveReputationExact ProgressionRep 0 0.25")] {
        assert!(g.data.table.unnumbered.contains_key(&name.to_ascii_lowercase()));
        assert_eq!(g.data.table.by_name(name), None, "native encoding is unmeasured");
        success(&mut g, source);
    }
}
