//! Portable loaded records exercise callback identity through source and the real hit recorder.
use crate::common::*;
use fnv_esm::{LoadOrder, Plugin, Schema};
use fnv_game::combat::HitInfo;
use fnv_game::game::{COMPILED_BASE, GameSnapshot, InstKey, LAMBDA_BASE, RunCtx, event};
use fnv_game::trace::EventKind;
use fnv_game::{Game, GameConfig, GameData};
use fnv_script::vm::{Value, XVal};
use fnv_script::{FunctionTable, VarType};
use fnv_sim::{FormId, FormStore, RefTable, SimConfig, World};

const HIT: FormId = FormId(0xFEF0_0440);
const REGISTRAR: FormId = FormId(0xFEF0_0430);
const QUEST: FormId = FormId(0xFEF0_0431);
const MAKER: FormId = FormId(0x0000_0900);
const TARGET: FormId = FormId(0x0000_0903);
const COUNT: FormId = FormId(0x800);
const WHO: FormId = FormId(0x801);
const BY: FormId = FormId(0x802);
const DAMAGE: FormId = FormId(0x803);
const CAPTURED: FormId = FormId(0x804);
const ORDINARY: FormId = FormId(0x905);

struct Env;
impl fnv_script::compile::Env for Env {
    fn form(&self, name: &str) -> Option<fnv_script::compile::FormInfo> {
        let (form, sig) = match name.to_ascii_lowercase().as_str() {
            "ownedhit" => (HIT.0, *b"SCPT"),
            "hitcount" => (COUNT.0, *b"GLOB"),
            "hitwho" => (WHO.0, *b"GLOB"),
            "hitby" => (BY.0, *b"GLOB"),
            "hitdamage" => (DAMAGE.0, *b"GLOB"),
            "capturedcount" => (CAPTURED.0, *b"GLOB"),
            _ => return None,
        };
        Some(fnv_script::compile::FormInfo { form, sig })
    }
    fn info(&self, _: u32) -> Option<fnv_script::compile::FormInfo> {
        None
    }
    fn editor_id(&self, _: u32) -> Option<String> {
        None
    }
    fn local_form(&self, _: u32) -> Option<u32> {
        None
    }
    fn script_vars(&self, _: u32) -> Option<Vec<fnv_script::VarDecl>> {
        None
    }
}
fn script(table: &FunctionTable, id: u32, name: &str, source: &str, quest: bool) -> Vec<u8> {
    let c = fnv_script::compile::compile(source, table, &Env, None);
    assert!(c.errors().next().is_none(), "owned script {name}: {:?}", c.diags);
    let vars: Vec<_> = c
        .variables
        .iter()
        .map(|v| {
            // The accepted SLSD enumeration keeps the disposal control typed as string.
            let kind = match &v.ty {
                VarType::Int => 1,
                VarType::Other(t) if t == "string_var" => 2,
                VarType::Other(t) if t == "array_var" => 3,
                _ => 0,
            };
            (v.index, v.name.as_str(), kind)
        })
        .collect();
    let mut sr = unit(u16::from(quest), &c.bytes, &vars, &[]);
    sr[0].data[4..8].copy_from_slice(&(c.refs.len() as u32).to_le_bytes());
    for r in &c.refs {
        sr.push(match r {
            fnv_script::unit::RefSlot::Form(f) | fnv_script::unit::RefSlot::Unmapped(f) => sub(b"SCRO", f.to_le_bytes().to_vec()),
            fnv_script::unit::RefSlot::Local(v) => sub(b"SCRV", v.to_le_bytes().to_vec()),
        });
    }
    sr.extend([sub(b"EDID", z(name)), sub(b"SCTX", source.as_bytes().to_vec())]);
    record(b"SCPT", 0, id, &sr)
}
struct Fixture {
    data: GameData,
    forms: FormStore,
    refs: RefTable,
}
fn fixture(collisions: bool) -> Fixture {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schemas");
    let path = root.join("script/functions.json");
    let table = FunctionTable::load(&path).unwrap();
    let json = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let globals = [
        (COUNT, "HitCount"),
        (WHO, "HitWho"),
        (BY, "HitBy"),
        (DAMAGE, "HitDamage"),
        (CAPTURED, "CapturedCount"),
    ]
    .into_iter()
    .flat_map(|(id, name)| {
        record(
            b"GLOB",
            0,
            id.0,
            &[
                sub(b"EDID", z(name)),
                sub(b"FNAM", vec![b'f']),
                sub(b"FLTV", 0f32.to_le_bytes().to_vec()),
            ],
        )
    })
    .collect::<Vec<_>>();
    let npcs = [(FormId::PLAYER_BASE.0, "Player"), (0x901, "OwnedActor")]
        .into_iter()
        .flat_map(|(id, name)| {
            record(
                b"NPC_",
                0,
                id,
                &[
                    sub(b"EDID", z(name)),
                    sub(b"DATA", [500i32.to_le_bytes().to_vec(), vec![5; 7]].concat()),
                ],
            )
        })
        .collect::<Vec<_>>();
    let placed = record(
        b"ACHR",
        0,
        TARGET.0,
        &[
            sub(b"EDID", z("OwnedTarget")),
            sub(b"NAME", 0x901u32.to_le_bytes().to_vec()),
            sub(b"DATA", vec![0; 24]),
        ],
    );
    let cell = record(b"CELL", 0, 0x902, &[sub(b"EDID", z("OwnedCell")), sub(b"DATA", vec![1])]);
    let cells = group(
        0i32.to_le_bytes(),
        2,
        &group(
            0i32.to_le_bytes(),
            3,
            &[cell, group(0x902u32.to_le_bytes(), 6, &group(0x902u32.to_le_bytes(), 9, &placed))].concat(),
        ),
    );
    let maker = script(
        &table,
        MAKER.0,
        "OwnedMaker",
        "scn OwnedMaker\nint n\nBegin Function {}\nlet n := 40\nSetFunctionValue (Begin Function {}\nlet n := n + 1\nlet CapturedCount := n\nEnd)\nEnd\n",
        false,
    );
    let ordinary = script(
        &table,
        ORDINARY.0,
        "OwnedEcho",
        "scn OwnedEcho\nint n\nBegin Function {n}\nSetFunctionValue n + GetSelf\nEnd\n",
        false,
    );
    let first = [
        tes4(),
        top(b"GLOB", &globals),
        top(b"NPC_", &npcs),
        top(b"CELL", &cells),
        top(b"SCPT", &[maker, ordinary].concat()),
    ]
    .concat();
    let mut plugins = vec![Plugin::parse(&first, "OwnedBase.esm".into()).unwrap()];
    for i in 1..254 {
        plugins.push(Plugin::parse(&tes4(), format!("OwnedPadding{i:03}.esm")).unwrap());
    }
    let mut header = vec![sub(
        b"HEDR",
        [1.34f32.to_le_bytes(), 0u32.to_le_bytes(), 0x00f00500u32.to_le_bytes()].concat(),
    )];
    for p in &plugins {
        header.extend([sub(b"MAST", z(&p.name)), sub(b"DATA", vec![0; 8])]);
    }
    let mut scripts = vec![
        script(
            &table,
            HIT.0,
            "OwnedHit",
            "scn OwnedHit\nstring_var scratch\nBegin Function {}\nlet scratch := sv_Construct \"portable\"\nlet HitCount := HitCount + 1\nlet HitWho := GetSelf\nlet HitBy := GetHitAttacker\nlet HitDamage := GetHitHealthDamage\nEnd\n",
            false,
        ),
        script(
            &table,
            REGISTRAR.0,
            "OwnedRegistrar",
            "scn OwnedRegistrar\nBegin GameMode\nSetOnHitEventHandler OwnedHit 1\nEnd\n",
            true,
        ),
    ];
    if collisions {
        for (id, name, value) in [(LAMBDA_BASE, "OwnedLambdaSlot", 71), (COMPILED_BASE, "OwnedCompiledSlot", 83)] {
            scripts.push(script(
                &table,
                id,
                name,
                &format!("scn {name}\nBegin Function {{}}\nSetFunctionValue {value}\nEnd\n"),
                false,
            ));
        }
    }
    let quest = record(
        b"QUST",
        0,
        QUEST.0,
        &[
            sub(b"EDID", z("OwnedRegistrarQuest")),
            sub(b"SCRI", REGISTRAR.0.to_le_bytes().to_vec()),
            sub(b"DATA", vec![0, 50]),
        ],
    );
    let last = [
        record(b"TES4", 0, 0, &header),
        top(b"SCPT", &scripts.concat()),
        top(b"QUST", &quest),
    ]
    .concat();
    plugins.push(Plugin::parse(&last, "OwnedCallbacks.esp".into()).unwrap());
    let lo = LoadOrder::new(&plugins).unwrap();
    assert_eq!(lo.to_global(254, HIT.0), Some(HIT.0));
    let schema = Schema::load_dir(root.join("fnv")).unwrap();
    let forms = FormStore::build(&lo, &schema);
    let refs = RefTable::build(&lo, &schema);
    let data = GameData::build(&lo, &schema, table, &json, &refs);
    assert_eq!(data.sig(HIT), Some(*b"SCPT"));
    assert!(data.script(HIT).is_some());
    Fixture { data, forms, refs }
}
fn game(f: &Fixture) -> Game<'_> {
    Game::new(
        &f.data,
        World::new(&f.forms, &f.refs, SimConfig::default()),
        GameConfig {
            strict: true,
            ..Default::default()
        },
    )
}
fn q(g: &mut Game<'_>, body: &str) -> XVal {
    g.run_query(&format!("Begin Function {{}}\n{body}\nEnd\n"), RunCtx::default())
        .unwrap_or_else(|e| panic!("owned source {body}: {e}"))
}
fn value(g: &Game<'_>, id: FormId) -> f64 {
    g.world.global(id).unwrap()
}
fn hit(g: &mut Game<'_>, on: FormId, damage: f64) {
    let result = g.hit(
        on,
        damage,
        HitInfo {
            attacker: Some(FormId::PLAYER_REF),
            weapon: None,
            location: 0,
            critical: true,
            limb: None,
        },
    );
    assert!((result.dealt - damage).abs() < 0.001, "real Game hit must deal its damage");
}
fn no_script_errors(g: &Game<'_>) {
    assert!(
        !g.events.iter().any(|e| matches!(e.kind, EventKind::ScriptError { .. })),
        "callback failure: {:?}",
        g.events
    );
}
fn register_record(g: &mut Game<'_>) {
    assert!(g.run_event(
        InstKey::Quest { quest: QUEST },
        REGISTRAR,
        event::GAME_MODE,
        None,
        RunCtx {
            quest: Some(QUEST),
            ..Default::default()
        }
    ));
}

#[test]
fn last_slot_record_registration_runs_on_real_hit_receiver_without_becoming_a_lambda() {
    let f = fixture(true);
    let mut g = game(&f);
    register_record(&mut g);
    for (i, on, damage) in [(1, FormId::PLAYER_REF, 17.0), (2, TARGET, 23.0), (3, TARGET, 11.0)] {
        hit(&mut g, on, damage);
        assert_eq!(value(&g, COUNT), f64::from(i));
        assert_eq!(value(&g, WHO), f64::from(on.0));
        assert_eq!(value(&g, BY), f64::from(FormId::PLAYER_REF.0));
        assert_eq!(value(&g, DAMAGE), damage);
        assert!(
            g.ext.is_empty(),
            "record UDF temporary strings must be disposed, including high IDs"
        );
    }
    no_script_errors(&g);
    assert_eq!(
        g.run_function(&XVal::Ref(LAMBDA_BASE), None, vec![], None, Default::default())
            .unwrap(),
        XVal::Num(71.0)
    );
    assert_eq!(q(&mut g, "SetFunctionValue (EvaluateString \"1 == 1\")"), XVal::Num(1.0));
    assert_eq!(
        g.run_function(&XVal::Ref(COMPILED_BASE), None, vec![], None, Default::default())
            .unwrap(),
        XVal::Num(83.0)
    );
}

#[test]
fn live_lambda_collision_avoidance_keeps_captures_and_saved_handle_identity() {
    let f = fixture(true);
    let mut g = game(&f);
    register_record(&mut g);
    let callback = q(
        &mut g,
        "ref cb\nlet cb := Call OwnedMaker\nSetOnHitEventHandler cb 1\nSetFunctionValue cb",
    );
    let XVal::Ref(handle) = callback else {
        panic!("owned lambda handle")
    };
    assert_ne!(handle, LAMBDA_BASE);
    hit(&mut g, TARGET, 7.0);
    assert_eq!(value(&g, CAPTURED), 41.0);
    hit(&mut g, TARGET, 9.0);
    assert_eq!(value(&g, CAPTURED), 42.0);
    assert_eq!(value(&g, COUNT), 2.0);
    no_script_errors(&g);
    let snap = g.snapshot();
    let json = serde_json::to_value(&snap).unwrap();
    assert_eq!(json["lambdas"][0]["handle"], handle);
    let mut restored = Game::restore(
        &f.data,
        World::new(&f.forms, &f.refs, Default::default()),
        Default::default(),
        &snap,
    )
    .unwrap();
    assert_eq!(q(&mut restored, "SetFunctionValue (Call OwnedMaker)"), XVal::Ref(handle));
    let before = value(&restored, COUNT);
    hit(&mut restored, TARGET, 5.0);
    assert_eq!(
        value(&restored, COUNT),
        before,
        "JIP hit registrations do not persist with the snapshot"
    );
    register_record(&mut restored);
    hit(&mut restored, TARGET, 6.0);
    assert_eq!(value(&restored, COUNT), before + 1.0);
    no_script_errors(&restored);
}

#[test]
fn legacy_lambda_snapshots_and_high_record_session_callbacks_keep_existing_lifetimes() {
    let f = fixture(false);
    let mut g = game(&f);
    let callback = q(&mut g, "SetFunctionValue (Call OwnedMaker)");
    assert_eq!(callback, XVal::Ref(LAMBDA_BASE));
    q(
        &mut g,
        "SetEventHandler \"owned_event\" OwnedHit\nSetEventHandler \"owned_lambda\" (Begin Function {}\nlet CapturedCount := 99\nEnd)\nCallFunctionNextFrame OwnedHit 1 0\nSetGameMainLoopCallback OwnedHit 1 1 0\nSetGameMainLoopCallback (Begin Function {}\nlet CapturedCount := 100\nEnd) 1 1 0",
    );
    let session = g.take_session();
    let snap = g.snapshot();
    let mut json = serde_json::to_value(&snap).unwrap();
    for item in json["lambdas"].as_array_mut().unwrap() {
        item.as_object_mut().unwrap().remove("handle");
    }
    let legacy: GameSnapshot = serde_json::from_value(json).unwrap();
    let mut restored = Game::restore(
        &f.data,
        World::new(&f.forms, &f.refs, Default::default()),
        Default::default(),
        &legacy,
    )
    .unwrap();
    restored.put_session(session);
    assert_eq!(q(&mut restored, "SetFunctionValue (Call OwnedMaker)"), callback);
    assert_eq!(
        restored.dispatch_event("owned_lambda", vec![], Some(TARGET)),
        0,
        "old-game lambda event callbacks are not session functions"
    );
    assert_eq!(restored.dispatch_event("owned_event", vec![], Some(TARGET)), 1);
    let before = value(&restored, COUNT);
    restored.tick();
    assert_eq!(
        value(&restored, COUNT),
        before + 2.0,
        "kept deferred and main-loop high-record callbacks run after restore"
    );
    no_script_errors(&restored);
}

#[test]
fn record_filter_receiver_and_ordinary_function_neighbors_keep_their_admission() {
    let f = fixture(true);
    let mut g = game(&f);
    q(
        &mut g,
        "ref target\nlet target := StringToRef \"00000903\"\nSetOnHitEventHandler OwnedHit 1 target",
    );
    hit(&mut g, FormId::PLAYER_REF, 3.0);
    assert_eq!(value(&g, COUNT), 0.0);
    hit(&mut g, TARGET, 4.0);
    assert_eq!(value(&g, COUNT), 1.0);
    assert_eq!(value(&g, WHO), f64::from(TARGET.0));
    let ctx = RunCtx {
        self_ref: Some(TARGET),
        ..Default::default()
    };
    assert_eq!(
        g.run_function(&XVal::Ref(ORDINARY.0), None, vec![XVal::Num(2.0)], None, ctx)
            .unwrap(),
        XVal::Num(f64::from(TARGET.0) + 2.0)
    );
    assert_eq!(
        g.run_function(&XVal::Ref(ORDINARY.0), Some(Value::Ref(0)), vec![XVal::Num(2.0)], None, ctx)
            .unwrap(),
        XVal::Num(2.0),
        "explicit empty receiver is not the caller"
    );
    assert!(g.run_function(&XVal::Ref(0xFEF0_0999), None, vec![], None, ctx).is_err());
    no_script_errors(&g);
}

#[test]
fn snapshot_handle_collisions_are_rejected_without_retargeting_loaded_records() {
    let f = fixture(true);
    let mut g = game(&f);
    let callback = q(&mut g, "SetFunctionValue (Call OwnedMaker)");
    let mut json = serde_json::to_value(g.snapshot()).unwrap();
    json["lambdas"][0]["handle"] = serde_json::json!(LAMBDA_BASE);
    let ambiguous: GameSnapshot = serde_json::from_value(json).unwrap();
    let result = Game::restore(
        &f.data,
        World::new(&f.forms, &f.refs, Default::default()),
        Default::default(),
        &ambiguous,
    );
    assert!(matches!(result,Err(ref e) if e.contains("unambiguous runtime callable")));
    assert_eq!(
        g.run_function(&XVal::Ref(LAMBDA_BASE), None, vec![], None, Default::default())
            .unwrap(),
        XVal::Num(71.0)
    );
    g.run_function(&callback, None, vec![], None, Default::default()).unwrap();
    assert_eq!(value(&g, CAPTURED), 41.0);
}

#[test]
fn high_record_and_owned_lambda_arity_are_checked_before_jip_dispatch() {
    let f = fixture(true);
    let mut g = game(&f);
    q(
        &mut g,
        "SetOnMenuClickEventHandler OwnedHit 1 \"portable\"\nSetOnMenuClickEventHandler (Begin Function {}\nlet HitCount := 100\nEnd) 1 \"portable\"",
    );
    assert_eq!(g.jip_menu_click("portable", "portable", 1.0, 2.0, "leaf"), 0);
    assert_eq!(value(&g, COUNT), 0.0);
    no_script_errors(&g);
    let error = g
        .run_function(&XVal::Ref(REGISTRAR.0), None, vec![], None, Default::default())
        .unwrap_err();
    assert!(
        error.contains("has no Function block"),
        "an ordinary script keeps its non-UDF admission: {error}"
    );
}
