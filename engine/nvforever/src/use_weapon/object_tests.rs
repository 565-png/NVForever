//! [design, #2652] Run the real shooter over authored references, without assets.
//! Hit delivery below enters Game::harmless_hit directly; physical collision is
//! covered by the mainframe smoke scenario, not this fixture.

use super::*;
use bevy::ecs::system::RunSystemOnce;
use fnv_combat::data::WeaponStats;
use fnv_game::combat::HitInfo;
use fnv_game::game::event;
use fnv_game::use_weapon::WeaponUse;
use fnv_script::vm::Value;
use std::sync::Arc;

#[path = "../../../../crates/fnv-game/tests/common/mod.rs"]
mod bytes;
use bytes::*;

const SHOOTER: FormId = FormId(0x801);
const TARGET: FormId = FormId(0x802);
const GUN: FormId = FormId(0x803);
const BASE: u32 = 0x804;
const SCRIPT: u32 = 0x805;
const CELL: u32 = 0x806;

fn fixture(kind: &[u8; 4], bounds: Option<[i16; 6]>) -> bevy::prelude::World {
    let placed = |sig, id, base, pos: [f32; 3]| record(sig, 0, id, &[
        sub(b"NAME", u32s(&[base])),
        sub(b"DATA", [pos.into_iter().flat_map(f32::to_le_bytes).collect::<Vec<_>>(), vec![0; 12]].concat()),
    ]);
    let mut base = vec![sub(b"EDID", z("AuthoredTarget"))];
    if let Some(bounds) = bounds {
        base.push(sub(b"OBND", bounds.into_iter().flat_map(i16::to_le_bytes).collect()));
    }
    if kind == b"ACTI" {
        base.push(sub(b"SCRI", u32s(&[SCRIPT])));
    }
    // [design: fnv-game/tests/script_events.rs] One OnHit parameter selects the
    // shooter's SCRO entry; only that shooter increments the authored local.
    let parameter = [le16(1), vec![b'r'], le16(1)].concat();
    let bump = set(var(b's', 1), ex(&[var(b's', 1), a("1"), a("+")]));
    let code = unit(0, &script(&[(event::ON_HIT, Some(parameter), bump)]), &[(1, "hits", 1)], &[SHOOTER.0]);
    let refs = [placed(b"ACHR", SHOOTER.0, 0x807, [0.0; 3]), placed(b"REFR", TARGET.0, BASE, [0.0, 500.0, 0.0])].concat();
    let children = group(CELL.to_le_bytes(), 6, &group(CELL.to_le_bytes(), 9, &refs));
    let cells = group([0; 4], 2, &group([0; 4], 3, &[record(b"CELL", 0, CELL, &[]), children].concat()));
    let raw = [tes4(), top(b"NPC_", &record(b"NPC_", 0, 0x807, &[sub(b"EDID", z("AuthoredShooter"))])),
        top(kind, &record(kind, 0, BASE, &base)), top(b"SCPT", &record(b"SCPT", 0, SCRIPT, &code)), top(b"CELL", &cells)].concat();
    let plugins = Box::leak(Box::new(vec![fnv_esm::Plugin::parse(&raw, "Authored.esm".into()).unwrap()]));
    let script = ScriptState::over_plugins(plugins);
    let g = &script.rt.as_ref().unwrap().game;
    let sim = SimState { forms: g.world.forms, refs: g.world.placed, world: fnv_sim::World::new(g.world.forms, g.world.placed, Default::default()) };
    let lo = Box::leak(Box::new(fnv_esm::LoadOrder::new(plugins).unwrap()));
    let schema = Box::leak(Box::new(fnv_esm::Schema::load_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/../../schemas/fnv")).unwrap()));
    let play = nvf_play::PlayData {
        lo, schema, index: Box::leak(Box::default()), world: 0, interior: Some(CELL), eye: [0.0; 3],
        heading_deg: 0.0, pitch_deg: 0.0, auto_walk: 0.0, start_at_cell_spot: false, packages: false,
    };
    let mut combat = Combat::over_plugins(play);
    combat.forms.weapons.insert(GUN, WeaponStats {
        projectile: Some(FormId(0x808)), automatic: true, fire_rate: 5.0, clip_size: 20,
        ammo_use: 1, attack_mult: 1.0, anim_mult: 1.0, ..Default::default()
    });
    let mut world = bevy::prelude::World::new();
    let mut time = Time::<()>::default();
    time.advance_by(std::time::Duration::from_secs_f32(0.1));
    world.insert_resource(time);
    world.insert_resource(script);
    world.insert_resource(sim);
    world.insert_resource(combat);
    world.insert_resource(Shooters::default());
    world.insert_resource(AttackTimes::default());
    world.init_resource::<Messages<ActorFire>>();
    world.init_resource::<Messages<ActorStrike>>();
    world.init_resource::<Messages<PlayAttack>>();
    world.spawn((ActorRig {
        reference: SHOOTER.0, base: 0x807, body: 0x807, skeleton: String::new(),
        skel: Arc::new(nvf_assets::actor::PreparedSkeleton::default()), nodes: vec![], base_scale: 1.0,
    }, GlobalTransform::from(crate::world::game_to_bevy()), InheritedVisibility::VISIBLE));
    world
}

fn start(world: &mut bevy::prelude::World, repeat: bool) {
    let mut use_ = WeaponUse::of_script(Some(GUN), None, TARGET, 3, false, false, true, true, None);
    use_.in_place = true;
    if repeat {
        use_.source = Source::Package(FormId(0x809));
        use_.volleys = None;
    }
    world.resource_mut::<ScriptState>().rt.as_mut().unwrap().game.weapon_uses.insert(SHOOTER, use_);
}

fn fire(world: &mut bevy::prelude::World) -> Vec<ActorFire> {
    for _ in 0..50 {
        world.run_system_once(shoot).unwrap();
    }
    world.resource_mut::<Messages<ActorFire>>().drain().collect()
}

#[test]
fn activator_gets_always_hit_harmless_shots_and_shooter_filtered_on_hit() {
    let mut world = fixture(b"ACTI", Some([-20, -20, 20, 20, 20, 100]));
    start(&mut world, false);
    let shots = fire(&mut world);
    assert_eq!(shots.len(), 3);
    let mut script = world.resource_mut::<ScriptState>();
    let g = &mut script.rt.as_mut().unwrap().game;
    let info = |attacker| HitInfo { attacker: Some(attacker), weapon: Some(GUN), location: -1, critical: false, limb: None };
    assert!(!g.harmless_hit(TARGET, info(FormId::PLAYER_REF)), "the OnHit block filters the shooter");
    for shot in shots {
        assert!(shot.harmless);
        assert_eq!(shot.cone, 0.0);
        let hit = vec::add(shot.origin, vec::scale(shot.aim, (500.0 - shot.origin[1]) / shot.aim[1]));
        assert!((hit[2] - 60.0).abs() < 0.001, "aim at OBND centre: {hit:?}");
        assert!(g.harmless_hit(TARGET, info(shot.shooter)));
    }
    let (key, _) = g.instance_of(TARGET).unwrap();
    assert_eq!(g.var(key, 1), Value::Num(3.0));
    assert!(g.weapon_use(SHOOTER).is_none());
}

#[test]
fn marker_script_fires_its_shots_and_ends_without_a_hit() {
    let mut world = fixture(b"STAT", None);
    start(&mut world, false);
    let shots = fire(&mut world);
    assert_eq!(shots.len(), 3);
    for shot in shots {
        let hit = vec::add(shot.origin, vec::scale(shot.aim, (500.0 - shot.origin[1]) / shot.aim[1]));
        assert!(hit[2].abs() < 0.001, "a marker has no chest offset: {hit:?}");
    }
    assert!(world.resource::<ScriptState>().rt.as_ref().unwrap().game.weapon_use(SHOOTER).is_none());
    assert_eq!(world.resource::<Shooters>().stats.done, 1);
}

#[test]
fn static_target_gets_shots_and_repeat_fire_keeps_going() {
    let mut world = fixture(b"STAT", Some([-40, -10, -2, 40, 10, 202]));
    start(&mut world, true);
    let shots = fire(&mut world);
    assert!(shots.len() > 3);
    let use_ = world.resource::<ScriptState>().rt.as_ref().unwrap().game.weapon_use(SHOOTER).unwrap();
    assert_eq!(use_.fired as usize, shots.len());
    assert!(!use_.done);
}

#[test]
fn drawn_actor_keeps_its_posed_chest_aim_instead_of_object_bounds() {
    let mut world = fixture(b"ACTI", Some([0, 0, 0, 0, 0, 200]));
    let bone = world.spawn(GlobalTransform::from_translation(Vec3::new(5.0, 80.0, -500.0))).id();
    let mut skel = nvf_assets::actor::PreparedSkeleton::default();
    skel.by_name.insert("Bip01 Spine1".into(), 0);
    world.spawn((ActorRig {
        reference: TARGET.0, base: BASE, body: BASE, skeleton: String::new(),
        skel: Arc::new(skel), nodes: vec![bone], base_scale: 1.0,
    }, GlobalTransform::from(Transform { translation: Vec3::new(0.0, 0.0, -500.0), ..crate::world::game_to_bevy() }), InheritedVisibility::VISIBLE));
    start(&mut world, false);
    let shots = fire(&mut world);
    assert_eq!(shots.len(), 3);
    for shot in shots {
        let at = vec::add(shot.origin, vec::scale(shot.aim, (500.0 - shot.origin[1]) / shot.aim[1]));
        assert!((at[0] - 5.0).abs() < 0.001 && (at[2] - 80.0).abs() < 0.001, "posed chest: {at:?}");
    }
}

#[test]
fn fallback_tracks_moved_references_and_rejects_dead_disabled_or_missing_targets() {
    let mut world = fixture(b"ACTI", Some([10, -20, 0, 30, 20, 100]));
    let mut script = world.resource_mut::<ScriptState>();
    let g = &mut script.rt.as_mut().unwrap().game;
    assert_eq!(target_place(g, TARGET), Some(([0.0, 500.0, 0.0], [20.0, 500.0, 50.0])));
    g.world.get(TARGET).unwrap().pos = [100.0, 200.0, 300.0];
    assert_eq!(target_place(g, TARGET), Some(([100.0, 200.0, 300.0], [120.0, 200.0, 350.0])));
    g.extra.entry(TARGET).or_default().dead = true;
    assert!(target_place(g, TARGET).is_none());
    g.extra.entry(TARGET).or_default().dead = false;
    g.world.set_enabled(TARGET, false).unwrap();
    assert!(target_place(g, TARGET).is_none());
    assert!(target_place(g, FormId(0xFFFF)).is_none());
}
