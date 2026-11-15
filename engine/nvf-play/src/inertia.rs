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
//! - **Switches** (the settings page's NVF Features › Advanced movement): the main
//!   "Advanced movement" switch over all of them ([`MovementInertia::advanced`]), then one
//!   each for the ground inertia here, the eased crouch, the smooth weapon animations, the
//!   weapon's weight in its handling, the soft recoil and the landing weight; the weapon
//!   sway's, the head bob's and the breath's are nvforever's `PlayerBody`. (An "Inertia
//!   weight" dropdown scaled every time until the owner's eleventh playtest, 2026-11-15:
//!   "remove the inertia weight option - current default will be the one we use"; 1.)
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

/// Seconds to reach a new, faster speed from rest (95%).
pub const START_SECONDS: f32 = 0.5;
/// Seconds to glide down to 5% of the speed, or to brake before a turn back (95%): half
/// the start [design: the owner's second playtest, 2026-11-13,
/// "stopping feels like the character is sliding on ice - make the stop time 2x speed of
/// the start one"; 0.9 s before].
pub const STOP_SECONDS: f32 = 0.25;
/// Drawing and putting away a weapon take 1 ÷ this as long as their clips,
/// played along the settling curve (about the clip's own speed at first, very
/// slow at the end), so a switch has weight; the game's readiness waits for it
/// [design: the owner's sixth playtest, 2026-11-14, "weapon switching/holstering: way too
/// fast"; 0.6 measured 0.62 s for a 10mm pistol's draw, still too quick].
pub const EQUIP_PACE: f32 = 0.45;
/// With smooth weapon animations, aiming takes this long to come up and to go down,
/// along the settling curve (`crate::ease::settle`: quick first, very slow at the
/// end): the aim and block clips' cross-over, the true iron sights' move to the eye and
/// the view's zoom all run on these, so they are one motion [design: the owner's seventh
/// playtest, 2026-11-14: "fast at the start but very slow and smooth at the end - make
/// the zoom in use the same curve … so it matches the animation"].
pub const AIM_IN_SECONDS: f32 = 0.6;
pub const AIM_OUT_SECONDS: f32 = 0.55;
/// The first-person view and arms take this long into a crouch and out of one: the
/// standing and sneaking clips cross over that long along the
/// settling curve, so the view sinks and the arms change their hold together.
pub const CROUCH_DOWN_SECONDS: f32 = 1.0;
pub const CROUCH_UP_SECONDS: f32 = 1.1;
/// The first-person arms' standing, walking and running clips cross over this long
/// along the settling curve (their own times are about 0.2 s, straight).
pub const MOVE_FADE_SECONDS: f32 = 0.45;
/// The first-person landing clip (the view dips and stands up again) takes this many times
/// its own length, its place along the settling curve: the dip comes
/// about as fast as the clip's own, the standing up after it slowly and to a soft end. A
/// hop's landing (up to [`LANDING_SOFT_SPEED`]) takes the first, a hard one (from
/// [`LANDING_HARD_SPEED`]) the second, smoothly between ([`land_stretch`]) [design: the
/// owner's eighth playtest, 2026-11-14, "fall-scaled landing"].
pub const LAND_STRETCH: f32 = 1.8;
pub const HARD_LAND_STRETCH: f32 = 3.2;
/// After a reload, a jam or an attack the first-person arms go back to the hold over this
/// long (× the weapon's handling), along the settling curve; the clips
/// themselves start at once and the arms ease across the step to them [design: the owner's
/// eighth playtest, 2026-11-14: "try improving reloads with springs"].
pub const RELOAD_OUT_SECONDS: f32 = 0.4;
pub const ATTACK_OUT_SECONDS: f32 = 0.3;
/// With soft recoil the view's kick of a shot comes over this long (95 %):
/// most of it at once, the rest softly (`crate::combat`). The climb is the
/// same, only spread over a few frames [design: the owner's eighth playtest, 2026-11-14:
/// "fast start, slow end … the jump is not instant to the next recoil point"].
pub const KICK_EASE_SECONDS: f32 = 0.16;
/// With soft recoil this share of each shot's view kick settles back down again,
/// over this long (95 %) on a critically damped spring: the gun
/// rises and comes back down a little instead of the whole kick staying in the view
/// [design: the owner's ninth playtest, 2026-11-14: "make the gun smoothly return down a
/// bit so the recoil does not just force the whole camera up. Also make the end of the
/// recoil smoother - slower ending" (the rise was 0.1 s)].
pub const KICK_BACK_SHARE: f32 = 0.4;
pub const KICK_BACK_SECONDS: f32 = 0.6;
/// How many times its base times the first-person hands take to draw, put away and aim a
/// weapon of `weight` (pounds) and to come back from its reload: light ones (1 lb and under)
/// 0.85, a pistol of about 3 lb 1, from 20 lb 1.5, smoothly between [design: the owner's
/// eighth playtest, 2026-11-14, "weapon weight slows handling"; the 10mm pistol the earlier
/// times were tuned with weighs 3].
pub fn handling(weight: f32) -> f32 {
    use crate::ease::smoothstep;
    0.85 + 0.15 * smoothstep(0.5, 3.0, weight) + 0.5 * smoothstep(3.0, 20.0, weight)
}

/// A hard landing pushes the view and the hands this many times as hard as its speed alone
/// would (from [`LANDING_HARD_SPEED`]; a hop's push, below [`LANDING_SOFT_SPEED`], is as
/// it was), so a long drop lands heavily [design: the owner's eighth playtest, 2026-11-14].
pub const HARD_LANDING_GAIN: f32 = 1.6;

/// A take-off's or a landing's push (a change of vertical speed, units/s) as the view's
/// and the hands' springs take it ([`HARD_LANDING_GAIN`]).
pub fn hard_landing(push: f32) -> f32 {
    push * (1.0 + (HARD_LANDING_GAIN - 1.0) * crate::ease::smoothstep(LANDING_SOFT_SPEED, LANDING_HARD_SPEED, push.abs()))
}

/// How many times its own length the first-person landing clip takes for a landing at
/// `fall_speed` (units/s).
pub fn land_stretch(fall_speed: f32) -> f32 {
    LAND_STRETCH + (HARD_LAND_STRETCH - LAND_STRETCH) * crate::ease::smoothstep(LANDING_SOFT_SPEED, LANDING_HARD_SPEED, fall_speed)
}

/// Landing compression: landings slower than the first fall speed (units/s) keep their
/// speed, from the second on they lose [`LANDING_MOST_LOST`] of it, smoothly between.
pub const LANDING_SOFT_SPEED: f32 = 350.0;
pub const LANDING_HARD_SPEED: f32 = 900.0;
pub const LANDING_MOST_LOST: f32 = 0.6;
/// Ground speed below which a glide no longer plays the movement clips, units/s.
pub const GLIDE_CLIP_SPEED: f32 = 30.0;
/// A move in one frame this far across the ground (units) is a placement, not motion:
/// the spring starts again from rest.
const PLACED_DISTANCE: f32 = 500.0;
/// Within this of the keys' velocity (units/s) and slower to change than [`SNAP_RATE`]
/// (units/s²), the spring is the keys' velocity exactly.
const SNAP_SPEED: f32 = 0.3;
const SNAP_RATE: f32 = 3.0;

/// What a start or a stop took, for the log (a smoke check).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Report {
    /// From standing to 95% of `speed` (units/s) in `seconds`.
    Started { seconds: f32, speed: f32 },
    /// From `speed` (units/s) when the keys were let go to a standstill: `distance`
    /// units in `seconds`.
    Stopped { seconds: f32, distance: f32, speed: f32 },
}

/// The player's ground velocity under inertia, and the switches (all on by default). Each
/// switch's own field is what the settings page shows; whether it acts is its method, under
/// the main switch.
#[derive(Resource, Debug, Clone)]
pub struct MovementInertia {
    /// The "Advanced movement" main switch: off, none of the switches here acts, nor the
    /// weapon sway's, the head bob's and the breath's (nvforever `PlayerBody`).
    pub advanced: bool,
    /// The "Movement inertia" switch: the ground speed's springs ([`Self::ground`]).
    pub on: bool,
    /// The "Eased crouch" switch ([`Self::eased_crouch`]).
    pub crouch: bool,
    /// The "Smooth weapon animations" switch ([`Self::smooth_hands`]): the first-person
    /// clips along the settling curve, the slower draw, the aim's and the zoom's curve.
    pub smooth: bool,
    /// The "Soft recoil" switch ([`Self::soft_recoil`]): the view's kick eased in and
    /// partly settling back, the gun's recoil springs.
    pub recoil: bool,
    /// The "Landing weight" switch ([`Self::landing_weight`]): hard landings dip deeper,
    /// stand up slower and take speed.
    pub landing: bool,
    /// The "Weapon weight in handling" switch and the weapon in hand's [`handling`] (1 with
    /// it or Smooth weapon animations off), kept by [`follow_weapon`].
    pub weapon_handling: bool,
    pub handling: f32,
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
    /// Where the game last put the player (`crate::player::Player::placed`).
    last_placed: Option<[f32; 3]>,
}

impl Default for MovementInertia {
    fn default() -> Self {
        MovementInertia { advanced: true, on: true, crouch: true, smooth: true, recoil: true, landing: true, weapon_handling: true, handling: 1.0, axes: [Spring::REST; 2], last_key: [0.0; 2], starting: None, gliding: None, reports: Vec::new(), ran: true, grounded: true, last_feet: None, last_placed: None }
    }
}

impl MovementInertia {
    /// At rest, switched on or off.
    pub fn switched(on: bool) -> MovementInertia {
        MovementInertia { on, ..default() }
    }

    /// The ground speed rides its springs ("Movement inertia" under the main switch).
    pub fn ground(&self) -> bool {
        self.advanced && self.on
    }

    /// The first-person view eases into a crouch and out ("Eased crouch").
    pub fn eased_crouch(&self) -> bool {
        self.advanced && self.crouch
    }

    /// The first-person weapon clips follow the settling curve ("Smooth weapon animations").
    pub fn smooth_hands(&self) -> bool {
        self.advanced && self.smooth
    }

    /// The view's kick eases in and partly settles back, the gun recoils on springs
    /// ("Soft recoil").
    pub fn soft_recoil(&self) -> bool {
        self.advanced && self.recoil
    }

    /// Hard landings land heavier ("Landing weight").
    pub fn landing_weight(&self) -> bool {
        self.advanced && self.landing
    }

    /// How many times their base times the first-person hands take (the weapon's
    /// handling).
    pub fn hands(&self) -> f32 {
        self.handling.max(0.1)
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

    /// Before a frame's steps: the game put the player somewhere new (`placed`: a door, the
    /// start, a load), however near, and the body starts from rest there. A door between
    /// two interiors whose coordinates lie close together is under [`PLACED_DISTANCE`]
    /// apart, and the run's speed came along into the new room (review, 2026-11-15).
    pub fn check_game_placed(&mut self, placed: Option<[f32; 3]>) {
        if placed.is_some() && placed != self.last_placed {
            self.reset();
        }
        self.last_placed = placed;
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
        let omega = omega_for(if starting { START_SECONDS } else { STOP_SECONDS });
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

    /// A landing at `fall_speed` (units/s, down): a hard one takes speed out of the
    /// ground velocity, which the spring then builds up again [design: the owner,
    /// 2026-11-14, "landing compression"]. A hop (about 290 units/s) takes none.
    pub fn land(&mut self, fall_speed: f32) {
        let hard = crate::ease::smoothstep(LANDING_SOFT_SPEED, LANDING_HARD_SPEED, fall_speed);
        let keep = 1.0 - LANDING_MOST_LOST * hard;
        for s in &mut self.axes {
            s.value *= keep;
            s.rate = 0.0;
        }
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
        if !self.ground() || !self.grounded || speed < GLIDE_CLIP_SPEED {
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


/// Keep [`MovementInertia::handling`] on the weapon the player holds (or is drawing).
pub fn follow_weapon(mut inertia: ResMut<MovementInertia>, combat: Option<Res<crate::combat::Combat>>) {
    let weight = combat.as_deref().and_then(|c| c.forms.weapons.get(&c.player_weapon()?).map(|w| w.weight));
    let handling = match (inertia.smooth_hands() && inertia.weapon_handling, weight) {
        (true, Some(w)) => handling(w),
        _ => 1.0,
    };
    if (inertia.handling - handling).abs() > 1e-4 {
        inertia.handling = handling;
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
    fn the_start_takes_its_time_and_the_main_switch_rules_every_switch() {
        let mut m = MovementInertia::default();
        let mut t = 0.0;
        while m.step([0.0, RUN], false, DT / 4.0)[1] < 0.95 * RUN {
            t += DT / 4.0;
        }
        assert!((t - START_SECONDS).abs() < 0.01, "{t}");
        let m = MovementInertia::default();
        assert!(m.ground() && m.eased_crouch() && m.smooth_hands() && m.soft_recoil() && m.landing_weight(), "all on by default");
        let off = MovementInertia { advanced: false, ..default() };
        assert!(!off.ground() && !off.eased_crouch() && !off.smooth_hands() && !off.soft_recoil() && !off.landing_weight());
        assert!(off.on && off.crouch && off.smooth && off.recoil && off.landing, "each keeps its own choice under it");
        let one = MovementInertia { crouch: false, ..default() };
        assert!(!one.eased_crouch() && one.ground() && one.smooth_hands(), "one switch off leaves the rest");
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
        // Put somewhere by the game, however near: from rest; the same place again keeps on.
        m.follow([0.0, 300.0]);
        m.check_game_placed(Some([10.0, 0.0, 0.0]));
        assert_eq!(m.velocity(), [0.0, 0.0]);
        m.follow([0.0, 300.0]);
        m.check_game_placed(Some([10.0, 0.0, 0.0]));
        m.check_game_placed(None);
        assert_eq!(m.velocity(), [0.0, 300.0], "walking on after it");
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
    fn heavy_weapons_handle_slower_and_hard_landings_take_longer() {
        assert!((handling(0.5) - 0.85).abs() < 1e-4);
        assert!((handling(3.0) - 1.0).abs() < 1e-4, "the 10mm pistol keeps the tuned times");
        assert!((handling(20.0) - 1.5).abs() < 1e-4);
        assert!(handling(7.0) > 1.0 && handling(7.0) < handling(15.0));
        assert_eq!(land_stretch(290.0), LAND_STRETCH, "a hop's landing as before");
        assert_eq!(land_stretch(2000.0), HARD_LAND_STRETCH);
        assert_eq!(hard_landing(291.0), 291.0, "a hop pushes as before");
        assert!((hard_landing(-1200.0) + 1200.0 * HARD_LANDING_GAIN).abs() < 1e-2);
        let m = MovementInertia { handling: 1.5, ..default() };
        assert_eq!(m.hands(), 1.5);
    }

    #[test]
    fn a_hard_landing_takes_speed_and_it_builds_up_again() {
        let mut m = MovementInertia::default();
        m.follow([0.0, RUN]);
        m.land(290.0);
        assert_eq!(m.velocity(), [0.0, RUN], "a hop keeps its speed");
        m.land(LANDING_HARD_SPEED);
        assert!((m.velocity()[1] - RUN * (1.0 - LANDING_MOST_LOST)).abs() < 1e-3);
        let v = steps(&mut m, [0.0, RUN], 2.0, DT);
        assert_eq!(v, [0.0, RUN], "back up to speed");
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
        m.advanced = false;
        assert_eq!(m.clip_intent(0.0, 0.0, false, 0.0), (0.0, 0.0, false), "the main switch off");
        m.advanced = true;
        steps(&mut m, [0.0, 0.0], 3.0, DT);
        assert_eq!(m.clip_intent(0.0, 0.0, false, 0.0), (0.0, 0.0, false));
    }
}
