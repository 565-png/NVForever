//! Movement inertia: the player character speeds up and slows down on the ground instead
//! of going from standing to full speed, and back, in one step [design: the owner's
//! request of 2026-11-13, "when player lets go of the movement key the character does not
//! stop instantly"; the original's player is at full speed from the first loop, physics.md
//! §6.3, so this departs from it on purpose, behind the "Movement inertia" switch].
//!
//! - **Where**: `crate::player`'s fixed step shapes the velocity the movement keys ask
//!   for before the character controller gets it. The controller (`fnv_phys`), the
//!   people's movement and the parity checks are untouched: only the player moves this way.
//! - **How**: each axis of the ground velocity rides a critically damped spring
//!   ([`crate::ease::Spring`]) towards the keys' velocity: about [`START_SECONDS`] to
//!   reach a new speed, walking or running, and [`STOP_SECONDS`] (half that) to glide down
//!   to a twentieth of it, after which the last of the glide eases out (a run's glide is
//!   about 36 units, half a metre); braking before a turn back takes the stopping time.
//!   From rest the spring starts gently, speeds up and settles without overshoot; solved
//!   exactly per step, it is the same at any physics tick rate. [design: the owner's
//!   playtests of 2026-11-13: first "slow the movement inertia down", then "stopping feels
//!   like … sliding on ice - make the stop time 2x speed of the start one".]
//! - **Inertia weight** ([`WEIGHTS`], the settings page's dropdown): every time here, and
//!   the weapon's and the crouch's (nvforever `view_sway.rs`, `crouch_eye.rs`), times
//!   this: 0.5 twice as quick, 2 twice as slow.
//! - **The air keeps its own rules** (`fnv_phys::air`, the game's measured air control):
//!   off the ground and swimming the keys' velocity goes to the controller as before, and
//!   the spring follows what the character does, so a landing carries the air speed into
//!   the ground speed. The game's half-second landing speed profile is left out while the
//!   switch is on: the landing lands into the inertia instead.
//! - **Clips**: while the body glides with no key held, or brakes against the keys, the
//!   movement clips follow the way it slides ([`MovementInertia::clip_intent`]). What the
//!   game's rules read (detection, power attacks, scripts) still reads the keys.

use crate::ease::{omega_for, Spring};
use bevy::prelude::*;

/// Seconds to reach a new, faster speed from rest (95%), at inertia weight 1.
pub const START_SECONDS: f32 = 0.5;
/// Seconds to glide down to 5% of the speed, or to brake before a turn back (95%), at
/// inertia weight 1: half the start [design: the owner's second playtest, 2026-11-13,
/// "stopping feels like the character is sliding on ice - make the stop time 2x speed of
/// the start one"; 0.9 s before].
pub const STOP_SECONDS: f32 = 0.25;
/// The inertia weights the settings page offers, by name: how many times the times
/// above (and the weapon's and the crouch's) are taken [design: the owner, 2026-11-13].
pub const WEIGHTS: [(f32, &str); 4] = [(0.5, "0.5 - 100% faster"), (1.0, "1 - normal speed"), (1.5, "1.5 - 1.5x slower"), (2.0, "2 - 2x slower")];
/// Ground speed below which a glide no longer plays the movement clips, units/s.
pub const GLIDE_CLIP_SPEED: f32 = 30.0;
/// A move in one frame this far across the ground (units) is a placement, not motion:
/// the spring starts again from rest.
const PLACED_DISTANCE: f32 = 500.0;
/// Within this of the keys' velocity (units/s) and slower to change than [`SNAP_RATE`]
/// (units/s²), the spring is the keys' velocity exactly.
const SNAP_SPEED: f32 = 0.3;
const SNAP_RATE: f32 = 3.0;

/// The inertia weight of a name or a number (`"1.5"`, or the name the page shows).
pub fn weight_by_name(s: &str) -> Option<f32> {
    let s = s.trim();
    WEIGHTS.iter().find(|(w, name)| *name == s || s.parse::<f32>().is_ok_and(|v| (v - w).abs() < 1e-3)).map(|(w, _)| *w)
}

/// The name the page shows for an inertia weight (the nearest offered).
pub fn weight_name(weight: f32) -> &'static str {
    WEIGHTS.iter().min_by(|a, b| (a.0 - weight).abs().total_cmp(&(b.0 - weight).abs())).map_or("1 - normal speed", |(_, n)| n)
}

/// What a start or a stop took, for the log (a smoke check).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Report {
    /// From standing to 95% of `speed` (units/s) in `seconds`.
    Started { seconds: f32, speed: f32 },
    /// From `speed` (units/s) when the keys were let go to a standstill: `distance`
    /// units in `seconds`.
    Stopped { seconds: f32, distance: f32, speed: f32 },
}

/// The player's ground velocity under inertia, and the switch.
#[derive(Resource, Debug, Clone)]
pub struct MovementInertia {
    /// The "Movement inertia" switch (on by default).
    pub on: bool,
    /// The "Inertia weight" dropdown ([`WEIGHTS`]; 1 by default).
    pub weight: f32,
    /// Horizontal velocity (game x, y), units/s, and its rate of change.
    axes: [Spring; 2],
    /// The keys' velocity at the last step.
    last_key: [f32; 2],
    /// A start being timed (seconds so far), a glide being measured (seconds, distance,
    /// the speed it began at).
    starting: Option<f32>,
    gliding: Option<(f32, f32, f32)>,
    /// Starts and stops finished since the log last took them.
    pub reports: Vec<Report>,
    /// Whether the last movement asked for ran (not walked): the glide's clips run too.
    pub ran: bool,
    /// On the ground (and not swimming) after the last step.
    pub grounded: bool,
    /// Feet at the end of the last step, to tell a placement from motion.
    last_feet: Option<[f32; 3]>,
}

impl Default for MovementInertia {
    fn default() -> Self {
        MovementInertia { on: true, weight: 1.0, axes: [Spring::REST; 2], last_key: [0.0; 2], starting: None, gliding: None, reports: Vec::new(), ran: true, grounded: true, last_feet: None }
    }
}

impl MovementInertia {
    /// At rest, switched on or off.
    pub fn switched(on: bool) -> MovementInertia {
        MovementInertia { on, ..default() }
    }

    /// The ground velocity now (game x, y), units/s.
    pub fn velocity(&self) -> [f32; 2] {
        [self.axes[0].value, self.axes[1].value]
    }

    /// Back to rest (placed somewhere, held by furniture, flying with `tcl`).
    pub fn reset(&mut self) {
        self.axes = [Spring::REST; 2];
        self.last_feet = None;
        self.forget_timing();
    }

    fn forget_timing(&mut self) {
        self.last_key = [0.0; 2];
        self.starting = None;
        self.gliding = None;
    }

    /// Before a frame's steps: a character moved further than it can walk in a frame was
    /// placed (a door, a script's move) and starts from rest.
    pub fn check_placed(&mut self, feet: [f32; 3]) {
        if self.last_feet.is_some_and(|l| (feet[0] - l[0]).hypot(feet[1] - l[1]) > PLACED_DISTANCE) {
            self.axes = [Spring::REST; 2];
            self.forget_timing();
        }
    }

    /// Times starts from standing and measures glides to a stop, after a step from the
    /// speed `before` to `v`.
    fn time(&mut self, key: [f32; 2], before: f32, v: [f32; 2], dt: f32) {
        let moving = |k: [f32; 2]| k != [0.0, 0.0];
        let (want, speed) = (key[0].hypot(key[1]), v[0].hypot(v[1]));
        if moving(key) && !moving(self.last_key) && speed <= want * 0.2 {
            self.starting = Some(0.0);
        }
        if !moving(key) && moving(self.last_key) && before > GLIDE_CLIP_SPEED {
            self.gliding = Some((0.0, 0.0, before));
        }
        if !moving(key) {
            self.starting = None;
        } else {
            self.gliding = None;
        }
        if let Some(t) = self.starting.as_mut() {
            *t += dt;
            if speed >= 0.95 * want {
                self.reports.push(Report::Started { seconds: *t, speed: want });
                self.starting = None;
            }
        }
        if let Some((t, d, from)) = self.gliding.as_mut() {
            *t += dt;
            *d += (before + speed) / 2.0 * dt;
            if !moving(v) {
                self.reports.push(Report::Stopped { seconds: *t, distance: *d, speed: *from });
                self.gliding = None;
            }
        }
        self.last_key = key;
    }

    /// One fixed step of `dt` seconds on the ground: the velocity towards the keys'
    /// velocity `key` (units/s), starting or braking; the velocity to move at.
    pub fn step(&mut self, key: [f32; 2], walk: bool, dt: f32) -> [f32; 2] {
        if key != [0.0, 0.0] {
            self.ran = !walk;
        }
        let v = self.velocity();
        let speed = v[0].hypot(v[1]);
        let want = key[0].hypot(key[1]);
        // Faster the same way: starting; slower, or against the way it goes: braking.
        let starting = want > speed && key[0] * v[0] + key[1] * v[1] >= 0.0;
        let omega = omega_for(self.weight.max(0.1) * if starting { START_SECONDS } else { STOP_SECONDS });
        for (s, k) in self.axes.iter_mut().zip(key) {
            s.step(k, omega, dt);
        }
        // Never faster than both the keys ask and it already went (the spring cannot
        // overshoot from rest; this holds when a change of rate meets a moving spring).
        let now = self.velocity();
        let limit = want.max(speed);
        let len = now[0].hypot(now[1]);
        if len > limit && len > 0.0 {
            for s in &mut self.axes {
                s.value *= limit / len;
            }
        }
        for (s, k) in self.axes.iter_mut().zip(key) {
            s.snap(k, SNAP_SPEED, SNAP_RATE);
        }
        let v = self.velocity();
        self.time(key, speed, v, dt);
        v
    }

    /// After a step off the ground or in the water (or with the switch off): the spring
    /// takes the velocity the character moved at, so the ground picks up from there.
    pub fn follow(&mut self, velocity: [f32; 2]) {
        self.axes = [Spring { value: velocity[0], rate: 0.0 }, Spring { value: velocity[1], rate: 0.0 }];
        self.forget_timing();
    }

    /// After a frame's steps.
    pub fn end_frame(&mut self, feet: [f32; 3], grounded: bool) {
        self.last_feet = Some(feet);
        self.grounded = grounded;
    }

    /// The movement the clips play for the keys held (`forward`, `right` as in
    /// `crate::player::MoveIntent`, `walk`) at the compass `heading`: the keys', but while
    /// the body glides on the ground with no key held or brakes against the keys, the way
    /// it slides, walking or running as it last moved.
    pub fn clip_intent(&self, forward: f32, right: f32, walk: bool, heading: f32) -> (f32, f32, bool) {
        let v = self.velocity();
        let speed = v[0].hypot(v[1]);
        if !self.on || !self.grounded || speed < GLIDE_CLIP_SPEED {
            return (forward, right, walk);
        }
        let (s, c) = heading.sin_cos();
        // Forward is (sin h, cos h), right (cos h, −sin h) (`crate::player::velocity`).
        let along = (v[0] * s + v[1] * c) / speed;
        let across = (v[0] * c - v[1] * s) / speed;
        let keys = forward != 0.0 || right != 0.0;
        let against = along * forward + across * right < 0.0;
        if keys && !against {
            return (forward, right, walk);
        }
        (along, across, !self.ran)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: f32 = 1.0 / 60.0;
    const RUN: f32 = 344.0;

    fn steps(m: &mut MovementInertia, key: [f32; 2], seconds: f32, dt: f32) -> [f32; 2] {
        let n = (seconds / dt).round() as usize;
        let mut v = m.velocity();
        for _ in 0..n {
            v = m.step(key, false, dt);
        }
        v
    }

    #[test]
    fn starts_gently_and_reaches_speed_in_time() {
        let mut m = MovementInertia::default();
        let first = m.step([0.0, RUN], false, DT);
        assert!(first[1] > 0.0 && first[1] < 0.2 * RUN, "a gentle first step: {first:?}");
        let v = steps(&mut m, [0.0, RUN], START_SECONDS - DT, DT);
        assert!(v[1] > 0.93 * RUN && v[1] <= RUN, "{v:?}");
        let v = steps(&mut m, [0.0, RUN], 1.5, DT);
        assert_eq!(v, [0.0, RUN], "then exactly the keys' speed");
    }

    #[test]
    fn walking_has_inertia_too() {
        let walk = 86.8;
        let mut m = MovementInertia::default();
        let v = steps(&mut m, [0.0, walk], 0.1, DT);
        assert!(v[1] < 0.3 * walk, "not at walking speed at once: {v:?}");
        steps(&mut m, [0.0, walk], 2.0, DT);
        let v = steps(&mut m, [0.0, 0.0], 0.1, DT);
        assert!(v[1] > 0.3 * walk, "still walking on after letting go: {v:?}");
    }

    #[test]
    fn glides_to_a_stop_creeping_at_the_end_and_ends_at_rest() {
        let mut m = MovementInertia::default();
        steps(&mut m, [RUN, 0.0], 2.0, DT);
        let v = m.step([0.0, 0.0], false, DT);
        assert!(v[0] > 0.95 * RUN, "still moving just after the key is let go: {v:?}");
        let v = steps(&mut m, [0.0, 0.0], STOP_SECONDS - DT, DT);
        assert!(v[0] > 0.0 && v[0] < 0.07 * RUN, "{v:?}");
        // The last of it eases out: still moving a moment later, slowly.
        let v = steps(&mut m, [0.0, 0.0], 0.15, DT);
        assert!(v[0] > 0.3 && v[0] < 0.02 * RUN, "{v:?}");
        let v = steps(&mut m, [0.0, 0.0], 2.0, DT);
        assert_eq!(v, [0.0, 0.0], "at rest exactly");
    }

    #[test]
    fn the_weight_scales_the_times() {
        let to_95 = |weight: f32| {
            let mut m = MovementInertia { weight, ..default() };
            let mut t = 0.0;
            while m.step([0.0, RUN], false, DT / 4.0)[1] < 0.95 * RUN {
                t += DT / 4.0;
            }
            t
        };
        let one = to_95(1.0);
        assert!((one - START_SECONDS).abs() < 0.01, "{one}");
        for (w, _) in WEIGHTS {
            assert!((to_95(w) - w * one).abs() < 0.01, "{w}: {}", to_95(w));
        }
        assert_eq!(weight_by_name("1.5"), Some(1.5));
        assert_eq!(weight_by_name("2 - 2x slower"), Some(2.0));
        assert_eq!(weight_by_name("3"), None);
        assert_eq!(weight_name(0.5), "0.5 - 100% faster");
        assert_eq!(weight_name(1.0), "1 - normal speed");
    }

    #[test]
    fn turning_back_brakes_first_and_never_overshoots() {
        let mut m = MovementInertia::default();
        steps(&mut m, [0.0, RUN], 2.0, DT);
        let mut last = RUN;
        let mut crossed = false;
        for _ in 0..300 {
            let v = m.step([0.0, -RUN], false, DT);
            assert!(v[1] <= last + 1e-3, "braking and then speeding up the other way only: {last} then {}", v[1]);
            assert!(v[1] >= -RUN - 1e-3, "never faster than asked: {v:?}");
            crossed |= v[1] < 0.0;
            last = v[1];
        }
        assert!(crossed);
        assert_eq!(m.velocity(), [0.0, -RUN]);
    }

    #[test]
    fn the_same_at_any_tick_rate() {
        // 1/12 s is a whole number of steps at each rate.
        for hz in [120.0, 144.0, 240.0] {
            let mut a = MovementInertia::default();
            let mut b = MovementInertia::default();
            let x = steps(&mut a, [RUN, 0.0], 1.0 / 12.0, DT)[0];
            let y = steps(&mut b, [RUN, 0.0], 1.0 / 12.0, 1.0 / hz)[0];
            assert!((x - y).abs() < 0.05, "{hz} Hz: {y} against {x}");
        }
    }

    #[test]
    fn the_air_hands_its_speed_to_the_ground_and_a_placement_starts_from_rest() {
        let mut m = MovementInertia::default();
        m.follow([0.0, 300.0]);
        let v = m.step([0.0, 0.0], false, DT);
        assert!(v[1] > 0.9 * 300.0, "lands gliding: {v:?}");
        m.end_frame([0.0, 0.0, 0.0], true);
        m.check_placed([10.0, 0.0, 0.0]);
        assert!(m.velocity()[1] > 0.0, "walking distance keeps it");
        m.check_placed([5000.0, 0.0, 0.0]);
        assert_eq!(m.velocity(), [0.0, 0.0]);
    }

    #[test]
    fn starts_and_glides_are_timed_for_the_log() {
        let mut m = MovementInertia::default();
        steps(&mut m, [0.0, RUN], 2.0, DT);
        steps(&mut m, [0.0, 0.0], 4.0, DT);
        let [Report::Started { seconds, speed }, Report::Stopped { seconds: stop, distance, speed: from }] = m.reports[..] else {
            panic!("{:?}", m.reports);
        };
        assert!((seconds - START_SECONDS).abs() < 0.03 && speed == RUN, "{seconds} {speed}");
        // A critically damped glide from v covers 2v/ω: about 36 units from a run.
        let glide = 2.0 * RUN / omega_for(STOP_SECONDS);
        assert!((distance - glide).abs() < 1.0 && (30.0..45.0).contains(&distance), "{distance} against {glide}");
        assert!(stop > 1.5 * STOP_SECONDS && stop < 1.0 && from == RUN, "{stop} {from}");
        assert!((STOP_SECONDS * 2.0 - START_SECONDS).abs() < 1e-6, "stopping takes half the start (the owner)");
        // Let go before full speed: no start reported, the glide still is.
        let mut m = MovementInertia::default();
        steps(&mut m, [0.0, RUN], 0.2, DT);
        steps(&mut m, [0.0, 0.0], 4.0, DT);
        assert!(matches!(m.reports[..], [Report::Stopped { .. }]), "{:?}", m.reports);
    }

    #[test]
    fn the_clips_follow_a_glide_and_the_keys_otherwise() {
        let mut m = MovementInertia::default();
        // Heading north; running north.
        steps(&mut m, [0.0, RUN], 2.0, DT);
        assert_eq!(m.clip_intent(1.0, 0.0, false, 0.0), (1.0, 0.0, false), "the keys while they agree");
        m.step([0.0, 0.0], false, DT);
        let (f, r, walk) = m.clip_intent(0.0, 0.0, true, 0.0);
        assert!((f - 1.0).abs() < 1e-4 && r.abs() < 1e-4 && !walk, "gliding forward, running: {f} {r} {walk}");
        // Facing east (π/2), the northward glide is to the left.
        let (f, r, _) = m.clip_intent(0.0, 0.0, false, std::f32::consts::FRAC_PI_2);
        assert!(f.abs() < 1e-4 && (r + 1.0).abs() < 1e-4, "{f} {r}");
        // Braking against Back: the clips still go forward.
        let (f, _, _) = m.clip_intent(-1.0, 0.0, false, 0.0);
        assert!(f > 0.9);
        // Stopped, in the air, or switched off: the keys.
        m.grounded = false;
        assert_eq!(m.clip_intent(0.0, 0.0, false, 0.0), (0.0, 0.0, false));
        m.grounded = true;
        m.on = false;
        assert_eq!(m.clip_intent(0.0, 0.0, false, 0.0), (0.0, 0.0, false));
        m.on = true;
        steps(&mut m, [0.0, 0.0], 3.0, DT);
        assert_eq!(m.clip_intent(0.0, 0.0, false, 0.0), (0.0, 0.0, false));
    }
}
