//! Integration tests for replaced NVSE stubs (Batch 3): `SetSoundFlag`/`GetSoundFlag`
//! (with `TempCloneForm` `SOUN` isolation), `StopSoundAlt`, `IsRadioRefPlaying`, and
//! `EndVATScam`.

use crate::common::*;
use fnv_esm::{LoadOrder, Plugin, Schema};
use fnv_game::game::{Game, GameConfig, RunCtx};
use fnv_game::nvse_gaps::RadioSite;
use fnv_game::nvse_itr1::RadioPlaying;
use fnv_game::vats::{VatsShot, MODE_OFF, MODE_PLAYBACK, MODE_TARGET_SELECT};
use fnv_game::GameData;
use fnv_script::vm::XVal;
use fnv_script::FunctionTable;
use fnv_sim::{FormId, FormStore, RefTable, SimConfig, World};
use fnv_world::radio::{Broadcast, BroadcastRange};
use std::path::Path;

const SOUN_SNDD: u32 = 0x0000_0C40;
const SOUN_SNDX: u32 = 0x0000_0C41;
const STAT_OBJ: u32 = 0x0000_0C42;
const ACTI_RADIO: u32 = 0x0000_0C43;
const TACT_STATION: u32 = 0x0000_0C44;
const NPC_ACTOR: u32 = 0x0000_0C45;

const REFR_STAT: u32 = 0x0000_0C50;
const REFR_RADIO: u32 = 0x0000_0C51;
const REFR_STATION: u32 = 0x0000_0C52;
const REFR_ACTOR_A: u32 = 0x0000_0C53;
const REFR_ACTOR_B: u32 = 0x0000_0C54;
const CELL_MAIN: u32 = 0x0000_0C55;

struct Fixture {
    data: GameData,
    forms: FormStore,
    refs: RefTable,
}

fn fixture() -> Fixture {
    // SNDD (36 bytes): flags u32 at offset 4..8 = Loop (bit 4) | 2D (bit 6) = 0x50.
    let mut sndd = vec![0u8; 36];
    sndd[4..8].copy_from_slice(&0x0000_0050u32.to_le_bytes());

    // SNDX (12 bytes): flags u16 at offset 4..6 = Environment Ignored (bit 2) = 0x04.
    let mut sndx = vec![0u8; 12];
    sndx[4..6].copy_from_slice(&0x0004u16.to_le_bytes());

    // XRDO (16 bytes): radius f32, broadcast range u32 (3 = Everywhere), static f32, pos ref u32.
    let mut xrdo = Vec::with_capacity(16);
    xrdo.extend(1000.0f32.to_le_bytes());
    xrdo.extend(3u32.to_le_bytes());
    xrdo.extend(0.0f32.to_le_bytes());
    xrdo.extend(0u32.to_le_bytes());

    let player = record(b"NPC_", 0, 7, &[sub(b"EDID", z("Player")), sub(b"DATA", vec![0; 11])]);
    let npc = record(b"NPC_", 0, NPC_ACTOR, &[sub(b"EDID", z("OwnSoundNpc")), sub(b"DATA", vec![0; 11])]);
    let soun_sndd = record(b"SOUN", 0, SOUN_SNDD, &[sub(b"EDID", z("OwnSnddSound")), sub(b"SNDD", sndd)]);
    let soun_sndx = record(b"SOUN", 0, SOUN_SNDX, &[sub(b"EDID", z("OwnSndxSound")), sub(b"SNDX", sndx)]);
    let stat_obj = record(b"STAT", 0, STAT_OBJ, &[sub(b"EDID", z("OwnPlainStat"))]);
    let acti_radio = record(b"ACTI", 0, ACTI_RADIO, &[sub(b"EDID", z("OwnTableRadio"))]);
    let tact_station = record(b"TACT", 0x0002_0000, TACT_STATION, &[sub(b"EDID", z("OwnStationBase"))]);

    let refr_stat = record(b"REFR", 0x400, REFR_STAT, &[sub(b"EDID", z("OwnPlainStatRef")), sub(b"NAME", STAT_OBJ.to_le_bytes().to_vec()), sub(b"DATA", vec![0; 24])]);
    let refr_radio = record(b"REFR", 0x400, REFR_RADIO, &[sub(b"EDID", z("OwnTableRadioRef")), sub(b"NAME", ACTI_RADIO.to_le_bytes().to_vec()), sub(b"DATA", vec![0; 24])]);
    let refr_station = record(
        b"REFR",
        0x400,
        REFR_STATION,
        &[sub(b"EDID", z("OwnStationRef")), sub(b"NAME", TACT_STATION.to_le_bytes().to_vec()), sub(b"XRDO", xrdo), sub(b"DATA", vec![0; 24])],
    );
    let refr_actor_a = record(b"ACHR", 0x400, REFR_ACTOR_A, &[sub(b"EDID", z("OwnSoundActorA")), sub(b"NAME", NPC_ACTOR.to_le_bytes().to_vec()), sub(b"DATA", vec![0; 24])]);
    let refr_actor_b = record(b"ACHR", 0x400, REFR_ACTOR_B, &[sub(b"EDID", z("OwnSoundActorB")), sub(b"NAME", NPC_ACTOR.to_le_bytes().to_vec()), sub(b"DATA", vec![0; 24])]);

    let cell = record(b"CELL", 0, CELL_MAIN, &[sub(b"EDID", z("OwnBatch3Cell")), sub(b"DATA", vec![1])]);
    let placed = group(
        CELL_MAIN.to_le_bytes(),
        6,
        &group(CELL_MAIN.to_le_bytes(), 9, &[refr_stat, refr_radio, refr_station, refr_actor_a, refr_actor_b].concat()),
    );

    let mut bytes = tes4();
    bytes.extend(top(b"NPC_", &[player, npc].concat()));
    bytes.extend(top(b"SOUN", &[soun_sndd, soun_sndx].concat()));
    bytes.extend(top(b"STAT", &stat_obj));
    bytes.extend(top(b"ACTI", &acti_radio));
    bytes.extend(top(b"TACT", &tact_station));
    bytes.extend(top(b"CELL", &group(0i32.to_le_bytes(), 2, &group(0i32.to_le_bytes(), 3, &[cell, placed].concat()))));

    let plugins = vec![Plugin::parse(&bytes, "OwnedNvseStubsBatch3.esm".into()).unwrap()];
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
fn set_sound_flag_and_get_sound_flag_support_sndd_sndx_validation_and_temp_clone_isolation() {
    let f = fixture();
    let mut g = game(&f);
    let sndd = FormId(SOUN_SNDD);
    let sndx = FormId(SOUN_SNDX);

    // Authored baseline (SNDD: bits 4 & 6; SNDX: bit 2).
    assert_eq!(n(&mut g, "GetSoundFlag OwnSnddSound 4"), 1.0);
    assert_eq!(n(&mut g, "GetSoundFlag OwnSnddSound 6"), 1.0);
    assert_eq!(n(&mut g, "GetSoundFlag OwnSnddSound 12"), 0.0);
    assert_eq!(n(&mut g, "GetSoundFlag OwnSndxSound 2"), 1.0);
    assert_eq!(n(&mut g, "GetSoundFlag OwnSndxSound 4"), 0.0);
    assert_eq!(g.sound_flags_override(sndd), None);

    // Early clone before any edit inherits authored flags.
    let XVal::Ref(c_early_raw) = call(&mut g, "tempcloneform", None, &[XVal::Ref(sndd.0), XVal::Num(1.0)]) else {
        panic!("expected early SOUN clone");
    };
    let c_early = FormId(c_early_raw);
    assert!(g.sound_flag(c_early, 4));
    assert!(!g.sound_flag(c_early, 12));

    // SetSoundFlag mutates target SOUN (clearing bit 4, setting bit 12) and returns 0.
    assert_eq!(n(&mut g, "SetSoundFlag OwnSnddSound 4 0"), 0.0);
    assert_eq!(n(&mut g, "SetSoundFlag OwnSnddSound 12 1"), 0.0);
    assert_eq!(n(&mut g, "GetSoundFlag OwnSnddSound 4"), 0.0);
    assert_eq!(n(&mut g, "GetSoundFlag OwnSnddSound 6"), 1.0);
    assert_eq!(n(&mut g, "GetSoundFlag OwnSnddSound 12"), 1.0);
    assert_eq!(g.sound_flags_override(sndd), Some((1 << 6) | (1 << 12)));

    // SNDX record can also be mutated.
    q(&mut g, "SetSoundFlag OwnSndxSound 4 1");
    assert!(g.sound_flag(sndx, 2));
    assert!(g.sound_flag(sndx, 4));

    // Early clone stays isolated from post-clone edits on orig.
    assert!(g.sound_flag(c_early, 4));
    assert!(!g.sound_flag(c_early, 12));
    assert_eq!(g.sound_flags_override(c_early), None);

    // Late clone snapshots orig's active flag overrides and stays isolated from later edits.
    let XVal::Ref(c_late_raw) = call(&mut g, "tempcloneform", None, &[XVal::Ref(sndd.0), XVal::Num(1.0)]) else {
        panic!("expected late SOUN clone");
    };
    let c_late = FormId(c_late_raw);
    assert!(!g.sound_flag(c_late, 4));
    assert!(g.sound_flag(c_late, 12));
    q(&mut g, "SetSoundFlag OwnSnddSound 12 0");
    assert!(!g.sound_flag(sndd, 12));
    assert!(g.sound_flag(c_late, 12));
    call(&mut g, "setsoundflag", None, &[XVal::Ref(c_late.0), XVal::Num(0.0), XVal::Num(1.0)]);
    assert!(g.sound_flag(c_late, 0));
    assert!(!g.sound_flag(sndd, 0));

    // Invalid flag IDs and non-SOUN forms are rejected cleanly without mutating state.
    assert!(!g.set_sound_flag(sndd, 14, true));
    assert!(!g.set_sound_flag(FormId(STAT_OBJ), 4, true));
    assert_eq!(call(&mut g, "setsoundflag", None, &[XVal::Ref(sndd.0), XVal::Num(14.0), XVal::Num(1.0)]), XVal::Num(0.0));
    assert_eq!(call(&mut g, "setsoundflag", None, &[XVal::Ref(sndd.0), XVal::Num(1.5), XVal::Num(1.0)]), XVal::Num(0.0));
    assert_eq!(call(&mut g, "getsoundflag", None, &[XVal::Ref(sndd.0), XVal::Num(14.0)]), XVal::Num(0.0));
    assert_eq!(call(&mut g, "getsoundflag", None, &[XVal::Ref(STAT_OBJ), XVal::Num(4.0)]), XVal::Num(0.0));
}

#[test]
fn stop_sound_alt_stops_matching_source_only_and_queues_fade_stop() {
    let f = fixture();
    let mut g = game(&f);
    let sound = FormId(SOUN_SNDD);
    let actor_a = FormId(REFR_ACTOR_A);
    let actor_b = FormId(REFR_ACTOR_B);

    // Nothing playing yet -> StopSoundAlt returns 0 and queues nothing.
    assert_eq!(n(&mut g, "StopSoundAlt OwnSnddSound OwnSoundActorA 0.5"), 0.0);
    assert!(g.gaps.sounds.stops_alt.is_empty());

    // Play the same SOUN at actor A, actor B, and non-locationally (PlaySound).
    q(&mut g, "OwnSoundActorA.PlaySound3D OwnSnddSound\nOwnSoundActorB.PlaySound3D OwnSnddSound\nPlaySound OwnSnddSound");
    // Advance two frames so IsSoundPlaying counts them as active (`frame + 1 < now`).
    g.tick();
    g.tick();
    assert_eq!(n(&mut g, "IsSoundPlaying OwnSnddSound OwnSoundActorA"), 1.0);
    assert_eq!(n(&mut g, "IsSoundPlaying OwnSnddSound OwnSoundActorB"), 1.0);
    assert_eq!(n(&mut g, "IsSoundPlaying OwnSnddSound"), 1.0);

    // StopSoundAlt on actor A returns 1, enqueues SoundStopAlt with fade, and stops only actor A.
    assert_eq!(n(&mut g, "StopSoundAlt OwnSnddSound OwnSoundActorA 0.5"), 1.0);
    assert_eq!(g.gaps.sounds.stops_alt.len(), 1);
    assert_eq!(g.gaps.sounds.stops_alt[0].sound, sound);
    assert_eq!(g.gaps.sounds.stops_alt[0].source, actor_a);
    assert_eq!(g.gaps.sounds.stops_alt[0].fade_seconds, 0.5);
    // Already stopped -> second StopSoundAlt returns 0.
    assert_eq!(n(&mut g, "StopSoundAlt OwnSnddSound OwnSoundActorA"), 0.0);

    // Next frame: actor A's sound is gone, actor B's and non-locational stay playing.
    g.tick();
    assert_eq!(n(&mut g, "IsSoundPlaying OwnSnddSound OwnSoundActorA"), 0.0);
    assert_eq!(n(&mut g, "IsSoundPlaying OwnSnddSound OwnSoundActorB"), 1.0);
    assert_eq!(n(&mut g, "IsSoundPlaying OwnSnddSound"), 1.0);
    assert_eq!(g.gaps.sounds.played.iter().filter(|p| p.source == Some(actor_b)).count(), 1);
}

#[test]
fn is_radio_ref_playing_tracks_enabled_stations_tuned_radios_and_engine_playback() {
    let f = fixture();
    let mut g = game(&f);
    let stat_ref = FormId(REFR_STAT);
    let radio_ref = FormId(REFR_RADIO);
    let station_ref = FormId(REFR_STATION);

    // Non-radio reference is never playing.
    assert_eq!(n(&mut g, "OwnPlainStatRef.IsRadioRefPlaying"), 0.0);
    assert!(!g.is_radio_ref_playing(stat_ref));

    // Enabled station with XRDO reads 1; disabling it flips IsRadioRefPlaying to 0.
    assert_eq!(n(&mut g, "OwnStationRef.IsRadioRefPlaying"), 1.0);
    q(&mut g, "OwnStationRef.Disable");
    assert_eq!(n(&mut g, "OwnStationRef.IsRadioRefPlaying"), 0.0);
    q(&mut g, "OwnStationRef.Enable");
    assert_eq!(n(&mut g, "OwnStationRef.IsRadioRefPlaying"), 1.0);

    // Untuned tabletop radio reads 0; after TuneRadioRef to an enabled station it reads 1,
    // and follows the station's disabled/enabled state.
    assert_eq!(n(&mut g, "OwnTableRadioRef.IsRadioRefPlaying"), 0.0);
    q(&mut g, "OwnTableRadioRef.TuneRadioRef OwnStationRef");
    assert_eq!(n(&mut g, "OwnTableRadioRef.IsRadioRefPlaying"), 1.0);
    q(&mut g, "OwnStationRef.Disable");
    assert_eq!(n(&mut g, "OwnTableRadioRef.IsRadioRefPlaying"), 0.0);
    q(&mut g, "OwnStationRef.Enable");
    assert_eq!(n(&mut g, "OwnTableRadioRef.IsRadioRefPlaying"), 1.0);

    // Explicit engine playback override and RadioSite / RadioPlaying integration.
    g.set_radio_ref_playing(radio_ref, false);
    assert_eq!(n(&mut g, "OwnTableRadioRef.IsRadioRefPlaying"), 0.0);
    g.set_radio_ref_playing(radio_ref, true);
    assert_eq!(n(&mut g, "OwnTableRadioRef.IsRadioRefPlaying"), 1.0);
    q(&mut g, "OwnTableRadioRef.Disable");
    assert_eq!(n(&mut g, "OwnTableRadioRef.IsRadioRefPlaying"), 0.0);

    g.gaps.radio_sites = Some(vec![RadioSite {
        station: station_ref,
        initially_disabled: false,
        on_pipboy: true,
        broadcast: Broadcast { range: BroadcastRange::Everywhere, radius: 1000.0, static_percent: 0.0, cell: CELL_MAIN, world: None, exterior: None },
    }]);
    g.other.itr.radio_playing = Some(RadioPlaying { track: Some(FormId(SOUN_SNDD)), file: "song.mp3".into(), station: Some(station_ref) });
    assert!(g.is_radio_ref_playing(station_ref));
    g.set_radio_ref_playing(station_ref, false);
    assert!(!g.is_radio_ref_playing(station_ref));
}

#[test]
fn is_radio_ref_playing_breaks_tune_radio_ref_cycles_cleanly() {
    let f = fixture();
    let mut g = game(&f);

    // 1-node self-loop: A -> A.
    q(&mut g, "OwnTableRadioRef.TuneRadioRef OwnTableRadioRef");
    assert_eq!(n(&mut g, "OwnTableRadioRef.IsRadioRefPlaying"), 0.0);

    // 2-node cycle: A -> B -> A.
    q(&mut g, "OwnTableRadioRef.TuneRadioRef OwnPlainStatRef\nOwnPlainStatRef.TuneRadioRef OwnTableRadioRef");
    assert_eq!(n(&mut g, "OwnTableRadioRef.IsRadioRefPlaying"), 0.0);
    assert_eq!(n(&mut g, "OwnPlainStatRef.IsRadioRefPlaying"), 0.0);

    // 3-node cycle: A -> B -> C -> A.
    q(&mut g, "OwnPlainStatRef.TuneRadioRef OwnSoundActorA\nOwnSoundActorA.TuneRadioRef OwnTableRadioRef");
    assert_eq!(n(&mut g, "OwnTableRadioRef.IsRadioRefPlaying"), 0.0);
    assert_eq!(n(&mut g, "OwnSoundActorA.IsRadioRefPlaying"), 0.0);
}

#[test]
fn end_vats_cam_clears_vats_state_and_queues_one_shot_engine_request() {
    let f = fixture();
    let mut g = game(&f);

    assert!(!g.end_vats_cam_requested());
    assert!(!g.take_end_vats_cam_request());

    // Enter VATS playback with a live shot + classic VATS UI active.
    g.ui_vats = true;
    g.set_vats(
        MODE_PLAYBACK,
        Some(VatsShot { weapon: FormId(STAT_OBJ), target: FormId(REFR_ACTOR_A), distance: 300.0, condition: Some(25), success: true, height: 60.0 }),
    );
    assert_eq!(n(&mut g, "GetVATSMode"), f64::from(MODE_PLAYBACK));

    // EndVATScam immediately clears VATS mode/shot/ui_vats and queues a one-shot request.
    q(&mut g, "EndVATScam");
    assert_eq!(n(&mut g, "GetVATSMode"), f64::from(MODE_OFF));
    assert_eq!(g.vats.shot, None);
    assert!(!g.ui_vats);
    assert!(g.end_vats_cam_requested());
    assert!(g.take_end_vats_cam_request());
    assert!(!g.end_vats_cam_requested());
    assert!(!g.take_end_vats_cam_request());

    // Also exits target-select mode cleanly.
    g.set_vats(MODE_TARGET_SELECT, None);
    assert_eq!(n(&mut g, "GetVATSMode"), f64::from(MODE_TARGET_SELECT));
    q(&mut g, "EndVATScam");
    assert_eq!(n(&mut g, "GetVATSMode"), f64::from(MODE_OFF));
    assert!(g.take_end_vats_cam_request());
}
