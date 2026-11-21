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
//!   with its feet on the bed ([`rests_on_bed`]) plays its standing clips, not its
//!   swimming idle. Actors give no vertical input, so one placed on a lake bed deeper
//!   than `wade_depth` swims from its first step, held at the bed with no buoyancy
//!   (physics.md §6.6, *Walking in again*), and played its swimming idle there for good:
//!   the lakelurks of Lake Mead stood on the bottom in the mirelurk king's floating
//!   pose, read as a T-pose, until a fight laid the aim and attack clips over it
//!   [hypothesis: the cause of #2980, from the code; not yet seen running]. "Swimming is
//!   when your feet do not touch solid ground" [documented: GECK wiki
//!   fSwimRadiationDamageMult]; whether the original stands such an actor on the bed is
//!   observer check 250 [hypothesis]. Moving, it swims on its swimming clips as before,
//!   so actors that follow the player into the water still swim [design]. The physics
//!   (`fnv_phys`, the player's and every swimmer's thresholds) is unchanged.

use crate::limbs::ActorRig;
use crate::locomotion::Locomotion;
use bevy::prelude::*;
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
        let speed = lo.velocity[0].hypot(lo.velocity[1]);
        let rests = swims && rests_on_bed(ch.and_then(|c| c.ground_gap), speed, lo.blend.cur.mv);
        let was_resting = if rests { !stats.resting.insert(r) } else { stats.resting.remove(&r) };
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

    /// A set of an idle, a walk and the turns, its files named `<prefix><move>.kf`
    /// (infos only: choosing and blending need no tracks).
    fn clips(prefix: &str) -> Arc<ClipSet> {
        let bound = Arc::new(BoundClip {
            clip: Arc::new(nvf_assets::anim_play::Clip {
                name: String::new(), start: 0.0, stop: 1.0, frequency: 1.0, cycle_type: 0, tracks: Vec::new(), priorities: Vec::new(),
                skipped: 0, accum_root: None, text_keys: Vec::new(), visibility: Vec::new(), emitters: Vec::new(), materials: Vec::new(),
                free: Vec::new(), free_visibility: Vec::new(), lights: Vec::new(),
            }),
            tracks: Vec::new(), root: None, root_track: None, carried: None, hold_xy: None, unbound: Default::default(),
        });
        let mut s = ClipSet::default();
        for (m, duration, speed) in [(Move::Idle, 3.3, 0.0), (Move::Forward, 1.6, 30.16), (Move::TurnLeft, 1.0, 0.0), (Move::TurnRight, 1.0, 0.0)] {
            s.clips.insert(m, SetClip { bound: bound.clone(), info: ClipInfo { duration, speed, foot_phase: 0.0, blend_time: 0.2 }, path: format!("{prefix}{}.kf", m.group().to_ascii_lowercase()) });
        }
        Arc::new(s)
    }

    fn idle_path(lo: &Locomotion) -> &str {
        &lo.set.clips[&Move::Idle].path
    }

    /// One frame of [`swim`] for one actor, `resting` its state from the frame before.
    fn frame(lo: &mut Locomotion, swims: bool, ground_gap: Option<f32>, resting: &mut bool) -> WaterChange {
        let rests = swims && rests_on_bed(ground_gap, lo.velocity[0].hypot(lo.velocity[1]), lo.blend.cur.mv);
        let change = follow_water(lo, swims, rests, *resting);
        *resting = rests;
        change
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

    /// #2980: a lakelurk placed on a lake bed deeper than `wade_depth` swims from its
    /// first step, held at the bed; it played its swimming idle there for good. It now
    /// stands on its standing idle, swims on its swimming clips once it moves, stands
    /// again when it stops, and floats on its swimming idle off the bed.
    #[test]
    fn a_creature_standing_on_a_deep_bed_plays_its_standing_idle() {
        let (stand, swim) = (clips("mt"), clips("swim"));
        let mut lo = Locomotion::new(stand.clone(), 0.0);
        lo.swim = Some(swim.clone());
        lo.blend.set_off_at_once = true;
        let mut resting = false;
        // Its character swims at the bed (ground gap 0), standing still.
        for _ in 0..10 {
            assert_eq!(frame(&mut lo, true, Some(0.0), &mut resting), WaterChange::None);
            lo.step(1.0 / 60.0);
        }
        assert!(resting && !lo.swimming);
        assert!(Arc::ptr_eq(&lo.set, &stand));
        assert_eq!(idle_path(&lo), "mtidle.kf", "the standing idle, not swimidle.kf");
        // It sets off along the bed: the swimming walk.
        lo.velocity = [0.0, 30.0];
        assert_eq!(frame(&mut lo, true, Some(0.0), &mut resting), WaterChange::SetsOff);
        assert!(lo.swimming && Arc::ptr_eq(&lo.set, &swim));
        lo.step(1.0 / 60.0);
        assert_eq!(lo.blend.cur.mv, Move::Forward);
        // It stops: it swims on while the walk plays, then stands once the idle plays.
        lo.velocity = [0.0; 2];
        for _ in 0..60 {
            assert_eq!(frame(&mut lo, true, Some(0.0), &mut resting), WaterChange::None, "the walk still plays");
            assert!(Arc::ptr_eq(&lo.set, &swim));
            lo.step(1.0 / 60.0);
            if lo.blend.cur.mv == Move::Idle {
                break;
            }
        }
        assert_eq!(lo.blend.cur.mv, Move::Idle);
        assert_eq!(frame(&mut lo, true, Some(0.0), &mut resting), WaterChange::Rests);
        assert!(!lo.swimming && Arc::ptr_eq(&lo.set, &stand));
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
}
