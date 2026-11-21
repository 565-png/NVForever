//! Actors other than the player swimming (owner's request 2026-10-18): while an actor's
//! character swims (`fnv_phys::Character::swimming`, the rules of
//! docs/spec-notes/physics.md §6.6), its locomotion plays the swimming clips of its
//! skeleton's folder in place of the standing ones (`locomotion::Locomotion::set_swimming`),
//! and walks again once its character is back on the ground.
//!
//! - **Which clips**: people use `characters\_male\swimmt…` (idle, the four directions
//!   walking and running, the turns), the clips the player's body swims on too
//!   (`fnv_actor::player_clips`, `Stance::Swim`); the mirelurks, giant rat, spinebreaker
//!   and dog have their own (`locomotion::SWIM_STANCES`) [observed: the base meshes
//!   archive listing]. A move without a swimming clip keeps its standing clip; skeletons
//!   without any swimming movement clip swim on their standing clips.
//! - **Weapons**: a swimmer moves unarmed, as the player's body holsters in water
//!   [design].
//! - **Standing on the bed** (#2980): an actor whose character swims but stands still
//!   with its feet on the bed ([`rests_now`]) plays its standing clips, not its
//!   swimming idle. Actors give no vertical input, so an actor with a live character on
//!   a lake bed deeper than `wade_depth` swims from that character's first step, held
//!   at the bed with no buoyancy (physics.md §6.6, *Walking in again*), and played its
//!   swimming idle (the mirelurk king's `swimidle.kf` for a lakelurk) whenever it stood
//!   still there. Only an actor on its way or in a fight keeps a character:
//!   `residents.rs` takes it from one that stands idle, which then plays its standing
//!   set whatever the water. So this is not shown to be the T-pose of #2980 (a lakelurk
//!   idle until it fought) [hypothesis]; the pose watch below and the base layer
//!   warning (`locomotion.rs`) are there to tell. "Swimming is when your feet do not
//!   touch solid ground" [documented: GECK wiki fSwimRadiationDamageMult]; whether the
//!   original stands such an actor on the bed is observer check 250 [hypothesis]. It
//!   comes to rest only once its stop has faded into the swimming idle, so the outgoing
//!   walk is not cut off [design]. Moving, it swims on its swimming clips as before, so
//!   actors that follow the player into the water still swim [design]. The physics
//!   (`fnv_phys`, the player's and every swimmer's thresholds) is unchanged.
//! - **Pose watch** (test aid `NVF_PLAY_TEST_POSE_WATCH=<text>[,<text>...]`): for each
//!   actor whose name (`Creature <ref> <EDID>`, `Actor <ref> <EDID>`) contains one of
//!   the texts (any case), a line whenever what it plays changes: the clip on its base
//!   layer (layer 0) and its weight, the set it plays from, its idle layers, whether it
//!   has a character (`PhysState::npcs`) and whether that swims and rests on the bed
//!   ([`pose_line`]).

use crate::limbs::ActorRig;
use crate::locomotion::Locomotion;
use bevy::prelude::*;
use nvf_assets::anim_play::BlendPlayer;
use fnv_actor::locomotion::{Move, STAND_SPEED};
use nvf_play::phys::PhysState;
use std::collections::BTreeSet;

/// Feet at most this far above the support under them stand on the bed, game units: the
/// character's `ground_gap`, which is 0 for a swimmer held against the bed (within the
/// contact shell) and at least 9 for one floating at `float_depth` over a bed deep enough
/// to keep it swimming (`wade_depth` 115 less `float_depth` 102.5, less the 3.5 shell)
/// [design].
pub const BED_GAP: f32 = 2.0;

/// Whether a swimming actor stands still on the bed and so plays its standing clips: its
/// feet within [`BED_GAP`] of the support under them (`ground_gap`, `None`: nothing in
/// reach), slower than [`STAND_SPEED`] and with no walk or run playing (`mv`; a turn on
/// the spot counts as standing) [design, #2980].
pub fn rests_on_bed(ground_gap: Option<f32>, speed: f32, mv: Move) -> bool {
    ground_gap.is_some_and(|g| g <= BED_GAP) && speed < STAND_SPEED && !mv.is_moving()
}

/// Whether a swimming actor (`swims`) rests on the bed now: [`rests_on_bed`] by its
/// character's `ground_gap` and its locomotion's speed and move. It comes to rest only
/// with no fade running (`blend.prev`: its stop has faded into the swimming idle), so the
/// walk fading out is not dropped from under it; once resting (`was_resting`), a fade
/// within the standing set (a turn ending) keeps it so [design].
pub fn rests_now(swims: bool, ground_gap: Option<f32>, lo: &Locomotion, was_resting: bool) -> bool {
    swims && rests_on_bed(ground_gap, lo.velocity[0].hypot(lo.velocity[1]), lo.blend.cur.mv) && (was_resting || lo.blend.prev.is_none())
}

/// What [`follow_water`] did to an actor's locomotion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaterChange {
    None,
    /// It went into the water: the swimming set plays.
    Swims,
    /// It left the water: the standing set plays.
    Leaves,
    /// Its character swims, standing still on the bed: the standing set plays.
    Rests,
    /// It sets off from the bed, still in the water: the swimming set plays again.
    SetsOff,
    /// Its character swims and its skeleton has no swimming clips.
    NoClips,
}

/// Follow the actor's character into and out of the water (`swims`: it swims; `rests`:
/// [`rests_on_bed`]; `was_resting`: it rested last time).
pub fn follow_water(lo: &mut Locomotion, swims: bool, rests: bool, was_resting: bool) -> WaterChange {
    let want = swims && !rests;
    if want == lo.swimming {
        return WaterChange::None;
    }
    if !lo.set_swimming(want) {
        return if want { WaterChange::NoClips } else { WaterChange::None };
    }
    match (want, rests || was_resting) {
        (true, false) => WaterChange::Swims,
        (false, false) => WaterChange::Leaves,
        (true, true) => WaterChange::SetsOff,
        (false, true) => WaterChange::Rests,
    }
}

pub struct SwimmingPlugin;

impl Plugin for SwimmingPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SwimStats>()
            .add_systems(Update, swim.after(nvf_play::PlaySet::Scripts).before(crate::locomotion::walk_test))
            .add_systems(Update, pose_watch.after(crate::locomotion::drive).run_if(|| watch_texts().is_some()))
            .add_systems(Last, report);
    }
}

#[derive(Resource, Default)]
struct SwimStats {
    /// Times an actor started swimming (into the water, or setting off from the bed), and
    /// left the water; actors that swam without swimming clips (counted once each).
    started: usize,
    stopped: usize,
    no_clips: BTreeSet<u32>,
    /// Actors that stood still on the bed while their character swam (counted once
    /// each), and those standing so now.
    rested: BTreeSet<u32>,
    resting: BTreeSet<u32>,
}

fn swim(phys: Option<Res<PhysState>>, mut actors: Query<(&ActorRig, &mut Locomotion)>, mut stats: ResMut<SwimStats>) {
    let Some(phys) = phys else { return };
    for (rig, mut lo) in &mut actors {
        let r = rig.reference;
        let ch = phys.npcs.get(&r).map(|n| &n.ch);
        let swims = ch.is_some_and(|c| c.swimming);
        let was_resting = stats.resting.contains(&r);
        let rests = rests_now(swims, ch.and_then(|c| c.ground_gap), &lo, was_resting);
        if rests {
            stats.resting.insert(r);
        } else {
            stats.resting.remove(&r);
        }
        if rests && stats.rested.insert(r) {
            info!("swimming: {r:08X} rests on the bed under water: it stands on its standing clips");
        }
        match follow_water(&mut lo, swims, rests, was_resting) {
            WaterChange::Swims => {
                stats.started += 1;
                info!("swimming: {r:08X} swims");
            }
            WaterChange::Leaves => {
                stats.stopped += 1;
                info!("swimming: {r:08X} is out of the water");
            }
            WaterChange::SetsOff => {
                stats.started += 1;
                debug!("swimming: {r:08X} sets off from the bed, swimming");
            }
            WaterChange::Rests => debug!("swimming: {r:08X} stands still on the bed"),
            WaterChange::NoClips => {
                if stats.no_clips.insert(r) {
                    info!("swimming: {r:08X} swims but has no swimming clips: it keeps its standing clips");
                }
            }
            WaterChange::None => {}
        }
    }
}

/// The texts of `NVF_PLAY_TEST_POSE_WATCH`, lower case (`None`: the aid is off).
fn watch_texts() -> Option<&'static [String]> {
    static TEXTS: std::sync::OnceLock<Option<Vec<String>>> = std::sync::OnceLock::new();
    TEXTS
        .get_or_init(|| {
            let v = std::env::var("NVF_PLAY_TEST_POSE_WATCH").ok()?;
            let texts: Vec<String> = v.split(',').map(|t| t.trim().to_ascii_lowercase()).filter(|t| !t.is_empty()).collect();
            (!texts.is_empty()).then_some(texts)
        })
        .as_deref()
}

/// The pose watch's line for one actor after its layers were assembled, and the part of
/// it that says when to write it again (no weights). `character`: `None` without a
/// character, else whether it swims and whether it rests on the bed.
pub fn pose_line(lo: &Locomotion, player: &BlendPlayer, character: Option<(bool, bool)>) -> (String, String) {
    let clip = |k: usize| player.layers[k].as_ref().map(|l| (lo.clip_path(&l.clip).map_or_else(|| format!("sequence {:?}", l.clip.clip.name), str::to_string), l.weight));
    let base = clip(0).map_or_else(|| "none".to_string(), |(p, _)| p);
    let base_weight = clip(0).map_or_else(String::new, |(_, w)| format!(" (weight {w:.2})"));
    let fading = clip(1).map_or_else(String::new, |(p, w)| format!(", layer 1 {p} (weight {w:.2})"));
    let idle_layers = crate::idles::IDLE_LAYER..crate::idles::IDLE_LAYER + 2;
    let idles: Vec<String> = idle_layers.clone().filter_map(|k| player.layers[k].as_ref().map(|l| format!("{:?} (weight {:.2})", l.clip.clip.name, l.weight))).collect();
    let idle_names: Vec<&str> = idle_layers.filter_map(|k| player.layers[k].as_ref().map(|l| l.clip.clip.name.as_str())).collect();
    let character = match character {
        None => "no character",
        Some((false, _)) => "a character, not swimming",
        Some((true, false)) => "a character, swimming",
        Some((true, true)) => "a character, swimming, resting on the bed",
    };
    let set = lo.posture_name();
    let idle_text = if idles.is_empty() { "none".to_string() } else { idles.join(", ") };
    let line = format!("layer 0 {base}{base_weight}{fading} from its {set} set; idle layers {idle_text}; {character}");
    let key = format!("{base}|{set}|{}|{character}", idle_names.join(","));
    (line, key)
}

/// `NVF_PLAY_TEST_POSE_WATCH` ([`watch_texts`]): after the actors are posed, a line for
/// each watched actor whenever its [`pose_line`] changes.
fn pose_watch(
    phys: Option<Res<PhysState>>,
    actors: Query<(Entity, &ActorRig, &Name, &Locomotion, &BlendPlayer)>,
    stats: Res<SwimStats>,
    mut last: Local<std::collections::HashMap<Entity, String>>,
) {
    let Some(texts) = watch_texts() else { return };
    for (e, rig, name, lo, player) in &actors {
        let lower = name.as_str().to_ascii_lowercase();
        if !texts.iter().any(|t| lower.contains(t.as_str())) {
            continue;
        }
        let character = phys.as_deref().and_then(|p| p.npcs.get(&rig.reference)).map(|n| (n.ch.swimming, stats.resting.contains(&rig.reference)));
        let (line, key) = pose_line(lo, player, character);
        if last.get(&e) != Some(&key) {
            info!("pose watch: {}: {line}", name.as_str());
            last.insert(e, key);
        }
    }
}

fn report(mut exit: MessageReader<AppExit>, stats: Res<SwimStats>) {
    if exit.read().next().is_some() {
        info!(
            "swimming summary: actors started swimming {} times and left the water {} times; {} swam without swimming clips; {} stood still on the bed",
            stats.started,
            stats.stopped,
            stats.no_clips.len(),
            stats.rested.len()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::locomotion::{ClipSet, SetClip};
    use fnv_actor::locomotion::ClipInfo;
    use nvf_assets::anim_play::BoundClip;
    use std::sync::Arc;

    const DT: f32 = 1.0 / 60.0;

    /// A set of an idle, a walk and the turns, its files named `<prefix><move>.kf`, each
    /// clip its own (infos only: choosing and blending need no tracks).
    fn clips(prefix: &str) -> Arc<ClipSet> {
        let mut s = ClipSet::default();
        for (m, duration, speed) in [(Move::Idle, 3.3, 0.0), (Move::Forward, 1.6, 30.16), (Move::TurnLeft, 1.0, 0.0), (Move::TurnRight, 1.0, 0.0)] {
            let bound = Arc::new(BoundClip {
                clip: Arc::new(nvf_assets::anim_play::Clip {
                    name: String::new(), start: 0.0, stop: 1.0, frequency: 1.0, cycle_type: 0, tracks: Vec::new(), priorities: Vec::new(),
                    skipped: 0, accum_root: None, text_keys: Vec::new(), visibility: Vec::new(), emitters: Vec::new(), materials: Vec::new(),
                    free: Vec::new(), free_visibility: Vec::new(), lights: Vec::new(),
                }),
                tracks: Vec::new(), root: None, root_track: None, carried: None, hold_xy: None, unbound: Default::default(),
            });
            s.clips.insert(m, SetClip { bound, info: ClipInfo { duration, speed, foot_phase: 0.0, blend_time: 0.2 }, path: format!("{prefix}{}.kf", m.group().to_ascii_lowercase()) });
        }
        Arc::new(s)
    }

    fn idle_path(lo: &Locomotion) -> &str {
        &lo.set.clips[&Move::Idle].path
    }

    /// One frame of [`swim`] for one actor, `resting` its state from the frame before.
    fn frame(lo: &mut Locomotion, swims: bool, ground_gap: Option<f32>, resting: &mut bool) -> WaterChange {
        let rests = rests_now(swims, ground_gap, lo, *resting);
        let change = follow_water(lo, swims, rests, *resting);
        *resting = rests;
        change
    }

    /// The files and weights on layers 0 and 1 after the layers are assembled.
    fn shown(lo: &mut Locomotion, player: &mut BlendPlayer) -> Vec<(String, f32)> {
        lo.layers(player);
        (0..2).filter_map(|k| player.layers[k].as_ref().map(|l| (lo.clip_path(&l.clip).expect("a clip of the sets").to_string(), l.weight))).collect()
    }

    fn weight_of(shown: &[(String, f32)], path: &str) -> f32 {
        shown.iter().filter(|(p, _)| p == path).map(|(_, w)| w).sum()
    }

    #[test]
    fn on_the_bed_only_a_still_actor_stands() {
        assert!(rests_on_bed(Some(0.0), 0.0, Move::Idle));
        assert!(rests_on_bed(Some(BED_GAP), 4.9, Move::TurnLeft), "a turn on the spot stands");
        assert!(!rests_on_bed(Some(0.0), STAND_SPEED, Move::Idle), "moving");
        assert!(!rests_on_bed(Some(0.0), 0.0, Move::Forward), "a walk still playing");
        // Floating at float_depth over a bed deep enough to swim on: 9 units or more up.
        assert!(!rests_on_bed(Some(9.0), 0.0, Move::Idle));
        assert!(!rests_on_bed(None, 0.0, Move::Idle), "no bed in reach");
    }

    /// #2980: an actor with a live character on a lake bed deeper than `wade_depth` swims
    /// from that character's first step, held at the bed; standing still it played its
    /// swimming idle there. It now stands on its standing idle, swims on its swimming
    /// clips once it moves, and stands again when it stops, the walk fading out first.
    #[test]
    fn a_creature_standing_on_a_deep_bed_plays_its_standing_idle() {
        let (stand, swim) = (clips("mt"), clips("swim"));
        let mut lo = Locomotion::new(stand.clone(), 0.0);
        lo.swim = Some(swim.clone());
        lo.blend.set_off_at_once = true;
        let mut player = BlendPlayer::new(Vec::new(), Arc::new(Vec::new()));
        let mut resting = false;
        // Its character swims at the bed (ground gap 0), standing still.
        for _ in 0..10 {
            assert_eq!(frame(&mut lo, true, Some(0.0), &mut resting), WaterChange::None);
            lo.step(DT);
        }
        assert!(resting && !lo.swimming);
        assert!(Arc::ptr_eq(&lo.set, &stand));
        assert_eq!(idle_path(&lo), "mtidle.kf", "the standing idle, not swimidle.kf");
        assert_eq!(shown(&mut lo, &mut player), vec![("mtidle.kf".to_string(), 1.0)]);
        // It sets off along the bed: the swimming walk.
        lo.velocity = [0.0, 30.0];
        assert_eq!(frame(&mut lo, true, Some(0.0), &mut resting), WaterChange::SetsOff);
        assert!(lo.swimming && Arc::ptr_eq(&lo.set, &swim));
        for _ in 0..30 {
            lo.step(DT);
            assert_eq!(frame(&mut lo, true, Some(0.0), &mut resting), WaterChange::None);
        }
        assert_eq!(shown(&mut lo, &mut player), vec![("swimforward.kf".to_string(), 1.0)]);
        // It stops. The walk fades into the swimming idle first (no clip dropped while
        // it shows), then the swimming idle into the standing one: every frame the two
        // layers share the whole weight and no clip's weight jumps.
        lo.velocity = [0.0; 2];
        let mut before = shown(&mut lo, &mut player);
        let mut rested_at = None;
        for i in 0..120 {
            let change = frame(&mut lo, true, Some(0.0), &mut resting);
            if change == WaterChange::Rests {
                assert!(rested_at.is_none(), "rests once");
                rested_at = Some(i);
                assert_eq!(before, vec![("swimidle.kf".to_string(), 1.0)], "comes to rest from the swimming idle alone");
            } else {
                assert_eq!(change, WaterChange::None);
            }
            lo.step(DT);
            let now = shown(&mut lo, &mut player);
            let total: f32 = now.iter().map(|(_, w)| w).sum();
            assert!((total - 1.0).abs() < 1e-4, "frame {i}: {now:?}");
            for path in ["swimforward.kf", "swimidle.kf", "mtidle.kf"] {
                let (a, b) = (weight_of(&before, path), weight_of(&now, path));
                assert!((a - b).abs() <= 0.15, "frame {i}: {path} from {a} to {b}: {before:?} -> {now:?}");
            }
            before = now;
        }
        assert!(rested_at.is_some());
        assert!(!lo.swimming && Arc::ptr_eq(&lo.set, &stand));
        assert_eq!(before, vec![("mtidle.kf".to_string(), 1.0)]);
        // Turning on the spot while resting, and the turn's fade back to the idle, keep
        // it standing.
        lo.turn = 2.0;
        for _ in 0..30 {
            assert_eq!(frame(&mut lo, true, Some(0.0), &mut resting), WaterChange::None);
            lo.step(DT);
        }
        lo.turn = 0.0;
        for _ in 0..30 {
            assert_eq!(frame(&mut lo, true, Some(0.0), &mut resting), WaterChange::None);
            lo.step(DT);
        }
        assert!(resting && Arc::ptr_eq(&lo.set, &stand));
        // Out of the water nothing changes: it stands already.
        assert_eq!(frame(&mut lo, false, Some(0.0), &mut resting), WaterChange::None);
        assert!(!resting);
        // Another floating off the bed (nothing in reach below): the swimming idle, and
        // out of the water the standing one.
        let mut lo = Locomotion::new(stand.clone(), 0.0);
        lo.swim = Some(swim.clone());
        let mut resting = false;
        assert_eq!(frame(&mut lo, true, None, &mut resting), WaterChange::Swims);
        assert_eq!(idle_path(&lo), "swimidle.kf");
        assert_eq!(frame(&mut lo, true, Some(9.0), &mut resting), WaterChange::None, "floating 9 above a bed");
        assert_eq!(frame(&mut lo, false, Some(0.0), &mut resting), WaterChange::Leaves);
        assert_eq!(idle_path(&lo), "mtidle.kf");
    }

    #[test]
    fn a_swimmer_without_swimming_clips_keeps_its_standing_set() {
        let stand = clips("mt");
        let mut lo = Locomotion::new(stand.clone(), 0.0);
        let mut resting = false;
        lo.velocity = [0.0, 30.0];
        assert_eq!(frame(&mut lo, true, None, &mut resting), WaterChange::NoClips);
        assert!(!lo.swimming && Arc::ptr_eq(&lo.set, &stand));
    }

    #[test]
    fn the_pose_watch_names_the_base_clip_and_the_character() {
        let (stand, swim) = (clips("mt"), clips("swim"));
        let mut lo = Locomotion::new(stand.clone(), 0.0);
        lo.swim = Some(swim);
        let mut player = BlendPlayer::new(Vec::new(), Arc::new(Vec::new()));
        lo.layers(&mut player);
        let (line, key) = pose_line(&lo, &player, None);
        assert_eq!(line, "layer 0 mtidle.kf (weight 1.00) from its standing set; idle layers none; no character");
        let (line, key2) = pose_line(&lo, &player, Some((true, true)));
        assert!(line.ends_with("; a character, swimming, resting on the bed"), "{line}");
        assert_ne!(key, key2);
        player.layers[0].as_mut().expect("idle").weight = 0.5;
        assert_eq!(pose_line(&lo, &player, Some((true, true))).1, key2, "a weight alone is no change");
        player.layers[0] = None;
        assert!(pose_line(&lo, &player, None).0.starts_with("layer 0 none from its standing set"));
    }
}
