//! Sway and bob of the first-person arms and weapon (the owner's request, 2026-10-20: the
//! arms and weapon "stay rigid throughout movement"; reworked for the movement-inertia
//! request of 2026-11-13: "the gun … moves with the direction using a ramp curve - slow
//! start, ramps up to a faster movement then smoothly slows").
//!
//! The game's first-person movement clips do not move the arms: the `mt` clips of
//! `characters\_1stperson\locomotion` drive `Bip01` (the root motion, 102.3 units over
//! `mtforward.kf`'s 1.2 s) and leave `Bip01 NonAccum` still, the `1hp` ones add
//! `Camera1st`, `Bip01 Translate` and `Bip01 Rotate` tracks without keys that move them,
//! and the folder has no `2hr`, `2ha` or `2hh` movement clips at all [observed:
//! `fnvtool nif decode` of `mtforward.kf` and `1hpforward.kf`, the folder's file list].
//! The camera node and the arms both hang from `Bip01 Looking`, so whatever moved that
//! chain would move the view with the arms. The executable holds the settings
//! `fFirstPersonHandFollowMult`, `fFirstPersonHandChaseSeconds` and
//! `fFirstPersonHandChaseSecondsAttack` [observed: their names in `FalloutNV.exe`'s
//! strings; not in `FalloutNV.esm` or `Fallout_default.ini`], which reads as the hands
//! chasing the view after a while [hypothesis: their meaning from the names; their
//! values are unknown]. So the sway is the engine's own, and this one is NVForever's
//! [design; every constant below an engineering choice, tuned by eye].
//!
//! The weapon hangs from the view on critically damped springs (`nvf_play::ease::Spring`,
//! solved exactly per frame, so the same at any frame rate): it starts to follow a change
//! slowly, speeds up and settles without overshoot, and its own motion never jumps.
//!
//! - **Turn lag**: the weapon chases the view: a steady turn leaves it behind by the turn
//!   rate × [`CHASE_SECONDS`], up to [`MAX_LAG_YAW`] and [`MAX_LAG_PITCH`]; when the view
//!   stops it carries on, slows and comes to rest on it. A sideways lag tilts the weapon a
//!   little. The view itself turns exactly as the mouse says.
//! - **Movement lag**: the weapon is held as by a sprung hand, pushed by the body's
//!   acceleration: starting, stopping, strafing and turning while running leave it a
//!   little behind, and it comes back while the pace holds. It leans into a strafe.
//! - **Jumps and landings**: the same hand, pushed by the body's vertical acceleration: a
//!   take-off dips the weapon, it rides a little high through the fall, and a landing
//!   dips it again, more the harder the landing; the muzzle tips with the dip.
//! - **Walking bob**: two dips a stride, one at each footfall, and one sideways swing
//!   a stride, at the phase of the movement clips the arms rig plays (their footfalls,
//!   `m:L` and `m:R`, fall about half a cycle apart: 0.40 s and 1.03 s of
//!   `mtforward.kf`'s 1.2 s [observed: audio.md]). It grows with the ground speed up to
//!   a run's, less sneaking, little down the sights, none in the air.
//! - **Breathing**: standing still the weapon drifts in a slow, small figure of eight.
//! - Down the sights and sneaking the sway is [`SIGHTS_SWAY`] of itself.
//! - **Weapon weight** ([`heft`], its own switch, off by default): a heavier weapon's
//!   springs are slower and its sway larger, up to half as much again.
//!
//! The pose turns and moves `Bip01 Translate` (the node above the arms, under `Bip01
//! Looking`) about the eye, in the view's own frame, so the camera node (`Camera1st`,
//! beside it under `Bip01 Looking`) and the view stay where they were.

use bevy::math::{Quat, Vec2, Vec3};
use nvf_play::ease::{smoothstep, Spring};

/// Seconds the arms lag behind a steady turn (turn rate × this is the lag).
pub const CHASE_SECONDS: f32 = 0.11;
/// Most the arms fall behind a turn: sideways and up or down, radians.
pub const MAX_LAG_YAW: f32 = 5.0 * std::f32::consts::PI / 180.0;
pub const MAX_LAG_PITCH: f32 = 3.5 * std::f32::consts::PI / 180.0;
/// The turn spring's state is held within this many times the shown limit, so a flick
/// comes back in about the same time as a fast turn.
const LAG_HOLD: f32 = 2.0;
/// Weapon tilt per radian of sideways lag.
const LAG_ROLL: f32 = 0.6;
/// The ground speed (units per second) at which the bob is full: about a run
/// [hypothesis: the player's walk is 85 units/s, `mtforward.kf`'s root motion].
const FULL_BOB_SPEED: f32 = 300.0;
/// The full bob: the dip, the sideways swing (units, at the eye's distance) and the
/// tilt with the swing (radians).
const BOB_DIP: f32 = 0.9;
const BOB_SWAY: f32 = 0.6;
const BOB_ROLL: f32 = 1.2 * std::f32::consts::PI / 180.0;
/// The first footfall's place in the movement cycle (0.40 s of 1.2 s).
pub const FOOTFALL_PHASE: f32 = 0.33;
/// Movement lag: the hand's spring rate (1/s), the most offset it shows for a sudden
/// change of velocity, units per unit/s, and the most offset at all.
const MOVE_OMEGA: f32 = 10.0;
const MOVE_PEAK_PER_SPEED: f32 = 0.006;
const MAX_MOVE_LAG: f32 = 1.2;
/// Jumps and landings: the spring rate (1/s), the most dip for a sudden change of
/// vertical velocity (units per unit/s: about 1 unit for a standing jump's 291 units/s),
/// the most offset at all, and the muzzle's tip per unit of dip (radians).
const LIFT_OMEGA: f32 = 12.0;
const LIFT_PEAK_PER_SPEED: f32 = 0.0034;
const MAX_LIFT: f32 = 2.0;
const LIFT_TIP: f32 = 0.6 * std::f32::consts::PI / 180.0;
/// Tilt into a strafe at a run's sideways speed, radians.
const STRAFE_ROLL: f32 = 1.5 * std::f32::consts::PI / 180.0;
/// Breathing: one figure of eight in this many seconds, its width (units; half as high),
/// below this ground speed (units/s), fading in and out over this long.
const BREATH_SECONDS: f32 = 4.0;
const BREATH_WIDTH: f32 = 0.18;
const BREATH_BELOW_SPEED: f32 = 20.0;
const BREATH_FADE_SECONDS: f32 = 0.8;
/// Shares down the sights and sneaking.
pub const SIGHTS_SWAY: f32 = 0.35;
const SIGHTS_BOB: f32 = 0.2;
const SNEAK_BOB: f32 = 0.6;
const SNEAK_BREATH: f32 = 0.35;
/// Seconds to fade the whole effect in or out (Pip-Boy, idles, the switch).
const FADE_SECONDS: f32 = 0.15;
/// Weapon weight: no more sway up to the first weight (pounds, the WEAP record's), half
/// as much again from the second: pistols as before, rifles a little heavier, the
/// miniguns and launchers clearly so [design: the owner, 2026-11-13].
pub const LIGHT_WEAPON: f32 = 3.0;
pub const HEAVY_WEAPON: f32 = 20.0;
pub const MOST_HEFT: f32 = 1.5;
/// `e`, for a spring's peak: a sudden change `Δ` of what drives it peaks at `Δ·k/(ω e)`.
const E: f32 = std::f32::consts::E;

/// How much slower and larger a weapon of `weight` pounds sways: 1 up to
/// [`LIGHT_WEAPON`], [`MOST_HEFT`] from [`HEAVY_WEAPON`], smoothly between.
pub fn heft(weight: f32) -> f32 {
    1.0 + (MOST_HEFT - 1.0) * smoothstep(LIGHT_WEAPON, HEAVY_WEAPON, weight)
}

/// What the sway reads each frame.
#[derive(Debug, Clone, Copy)]
pub struct SwayInput {
    pub dt: f32,
    /// The view's compass heading (clockwise) and pitch (up), radians.
    pub heading: f32,
    pub pitch: f32,
    /// Ground velocity in the view's frame: x to the right, y ahead (units/s).
    pub velocity: Vec2,
    /// The body's vertical velocity off the ground (units/s, up); 0 on it and swimming.
    pub vertical: f32,
    /// Where the movement clips are in their cycle (0–1).
    pub phase: f32,
    pub sights: bool,
    pub sneaking: bool,
    pub airborne: bool,
    /// The weapon's weight as sway ([`heft`]; 1 for none).
    pub heft: f32,
    /// Whether the arms may sway at all this frame.
    pub on: bool,
}

impl Default for SwayInput {
    fn default() -> Self {
        SwayInput { dt: 0.0, heading: 0.0, pitch: 0.0, velocity: Vec2::ZERO, vertical: 0.0, phase: 0.0, sights: false, sneaking: false, airborne: false, heft: 1.0, on: false }
    }
}

/// The pose to put on the arms: a turn and a move in the view's frame (x right, y ahead,
/// z up) about the eye.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SwayPose {
    pub rotation: Quat,
    pub offset: Vec3,
}

impl SwayPose {
    pub const NONE: SwayPose = SwayPose { rotation: Quat::IDENTITY, offset: Vec3::ZERO };
}

/// The sway's state across frames.
#[derive(Debug, Clone, Default)]
pub struct ViewSway {
    last: Option<(f32, f32)>,
    /// The weapon's way relative to the view's (heading, pitch: arms − view, radians) on
    /// its springs, and the weapon's own turn rate (radians/s).
    turn: [Spring; 2],
    turn_rate: Vec2,
    /// How far the arms are shown behind the view (the turn springs, held in the limits).
    lag: Vec2,
    /// The hand's offset from the body's acceleration (view frame, x right, y ahead) and
    /// from its vertical acceleration (up), and the velocities they were last pushed from.
    hand: [Spring; 2],
    lift: Spring,
    last_velocity: Option<Vec2>,
    last_vertical: f32,
    /// How much of the bob shows (follows the speed).
    bob: f32,
    /// Breathing: where in the figure of eight (0–1) and how much of it shows.
    breath_phase: f32,
    breath: f32,
    /// How much of everything shows.
    gain: f32,
    /// The most turn lag (sideways, up or down), bob dip and landing dip shown since the
    /// start (for the exit summary).
    pub peak_lag: Vec2,
    pub peak_dip: f32,
    pub peak_lift_dip: f32,
}

/// `x` towards `max` and never past it, smoothly.
fn soft_clamp(x: f32, max: f32) -> f32 {
    max * (x / max).tanh()
}

/// `a - b` as the shorter way round, radians.
fn angle_diff(a: f32, b: f32) -> f32 {
    use std::f32::consts::{PI, TAU};
    (a - b + PI).rem_euclid(TAU) - PI
}

/// A spring rate `omega` with the gain that makes a sudden change `Δ` of the velocity
/// pushing it peak at `peak_per_speed × Δ`: its target for an acceleration `accel`.
fn pushed_target(accel: f32, omega: f32, peak_per_speed: f32) -> f32 {
    -peak_per_speed * omega * E * accel / (omega * omega)
}

impl ViewSway {
    pub fn update(&mut self, i: SwayInput) -> SwayPose {
        let dt = i.dt.max(0.0);
        let follow = |t: f32| if dt <= 0.0 { 0.0 } else { 1.0 - (-dt / t).exp() };
        let heft = i.heft.max(1.0);
        self.gain += (f32::from(u8::from(i.on)) - self.gain) * follow(FADE_SECONDS);

        // Turn lag: the view turned at `rate` this frame; the weapon chases it on a spring
        // that lags a steady turn by `rate × CHASE_SECONDS`. A jump of the view (a
        // teleport, an idle letting go) is not a turn: the weapon goes with it.
        let mut rate = Vec2::ZERO;
        if let (Some((h, p)), true) = (self.last, dt > 0.0) {
            let d = Vec2::new(angle_diff(i.heading, h), i.pitch - p);
            if d.length() < 0.5 {
                rate = d / dt;
            }
        }
        self.last = Some((i.heading, i.pitch));
        let chase = CHASE_SECONDS * heft;
        let omega = 2.0 / chase;
        let limit = Vec2::new(MAX_LAG_YAW, MAX_LAG_PITCH) * heft;
        for k in 0..2 {
            let s = &mut self.turn[k];
            // The spring holds the lag; its rate is the weapon's turn less the view's.
            s.rate = self.turn_rate[k] - rate[k];
            s.step(-rate[k] * chase, omega, dt);
            let hold = LAG_HOLD * limit[k];
            if s.value.abs() > hold {
                s.value = s.value.clamp(-hold, hold);
                if s.rate * s.value > 0.0 {
                    s.rate = 0.0;
                }
            }
            self.turn_rate[k] = s.rate + rate[k];
        }
        self.lag = Vec2::new(soft_clamp(self.turn[0].value, limit.x), soft_clamp(self.turn[1].value, limit.y));

        // The sprung hand, pushed by the body's acceleration across the ground and up.
        let accel = match (self.last_velocity, dt > 0.0) {
            (Some(v), true) => (i.velocity - v) / dt,
            _ => Vec2::ZERO,
        };
        self.last_velocity = Some(i.velocity);
        let move_omega = MOVE_OMEGA / heft;
        let move_peak = MOVE_PEAK_PER_SPEED * heft;
        for (s, a) in self.hand.iter_mut().zip([accel.x, accel.y]) {
            s.step(pushed_target(a, move_omega, move_peak), move_omega, dt);
        }
        let up_accel = if dt > 0.0 { (i.vertical - self.last_vertical) / dt } else { 0.0 };
        self.last_vertical = i.vertical;
        let lift_omega = LIFT_OMEGA / heft;
        self.lift.step(pushed_target(up_accel, lift_omega, LIFT_PEAK_PER_SPEED * heft), lift_omega, dt);

        // The bob follows the speed; breathing shows standing still.
        let speed = i.velocity.length();
        let bob_target = if i.airborne { 0.0 } else { (speed / FULL_BOB_SPEED).min(1.0) };
        self.bob += (bob_target - self.bob) * follow(0.2);
        let still = !i.airborne && speed < BREATH_BELOW_SPEED;
        self.breath += (f32::from(u8::from(still)) - self.breath) * follow(BREATH_FADE_SECONDS);
        self.breath_phase = (self.breath_phase + dt / (BREATH_SECONDS * heft)).fract();

        let g = self.gain;
        let sway_share = g * if i.sights { SIGHTS_SWAY } else { 1.0 };
        let bob_share = g * self.bob * if i.sights { SIGHTS_BOB } else { 1.0 } * if i.sneaking { SNEAK_BOB } else { 1.0 };
        let breath_share = g * self.breath * heft * if i.sights { SIGHTS_SWAY } else if i.sneaking { SNEAK_BREATH } else { 1.0 };
        let lag = self.lag * sway_share;
        let lift = soft_clamp(self.lift.value, MAX_LIFT * heft) * sway_share;
        // Heading grows clockwise, a turn about +z anticlockwise: arms behind a turn to
        // the right point left of it. A dip tips the muzzle down.
        let turn = Quat::from_rotation_z(-lag.x) * Quat::from_rotation_x(lag.y + lift * LIFT_TIP);
        let a = std::f32::consts::TAU * (i.phase - FOOTFALL_PHASE);
        // Down at each footfall (twice a cycle), across once a cycle.
        let dip = -BOB_DIP * 0.5 * (1.0 + (2.0 * a).cos()) * bob_share;
        let swing = a.sin();
        let lean = (i.velocity.x / FULL_BOB_SPEED).clamp(-1.0, 1.0);
        let roll = lag.x * LAG_ROLL + BOB_ROLL * swing * bob_share + STRAFE_ROLL * lean * sway_share;
        let hand = Vec2::new(self.hand[0].value, self.hand[1].value);
        let most = MAX_MOVE_LAG * heft;
        let hand = if hand.length() > 1e-6 { hand * (soft_clamp(hand.length(), most) / hand.length()) } else { Vec2::ZERO };
        let b = std::f32::consts::TAU * self.breath_phase;
        let breath = Vec2::new(b.sin(), 0.5 * (2.0 * b).sin()) * BREATH_WIDTH * breath_share;
        let offset = Vec3::new(BOB_SWAY * swing * bob_share + hand.x * sway_share + breath.x, hand.y * sway_share, dip + lift + breath.y);
        self.peak_lag = self.peak_lag.max(lag.abs());
        self.peak_dip = self.peak_dip.max(-dip);
        self.peak_lift_dip = self.peak_lift_dip.max(-lift);
        SwayPose { rotation: turn * Quat::from_rotation_y(roll), offset }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(heading: f32, velocity: Vec2, phase: f32) -> SwayInput {
        SwayInput { dt: 1.0 / 60.0, heading, velocity, phase, on: true, ..SwayInput::default() }
    }

    fn settle(s: &mut ViewSway, i: SwayInput, frames: usize) -> SwayPose {
        let mut p = SwayPose::NONE;
        for _ in 0..frames {
            p = s.update(i);
        }
        p
    }

    #[test]
    fn standing_still_only_breathes() {
        let mut s = ViewSway::default();
        let mut most = 0.0f32;
        for _ in 0..600 {
            let p = s.update(input(1.0, Vec2::ZERO, 0.0));
            assert!(p.rotation.angle_between(Quat::IDENTITY) < 1e-4, "{p:?}");
            assert!(p.offset.y.abs() < 1e-4);
            most = most.max(p.offset.x.abs());
        }
        assert!(most > 0.8 * BREATH_WIDTH && most <= BREATH_WIDTH + 1e-4, "a small drift: {most}");
    }

    #[test]
    fn breathing_stops_while_moving_and_calms_down_the_sights() {
        let still = |sights: bool, v: Vec2| {
            let mut s = ViewSway::default();
            (0..600).map(|_| s.update(SwayInput { sights, ..input(0.0, v, 0.0) }).offset.x.abs()).skip(300).fold(0.0f32, f32::max)
        };
        let free = still(false, Vec2::ZERO);
        let aimed = still(true, Vec2::ZERO);
        assert!((aimed / free - SIGHTS_SWAY).abs() < 0.02, "{aimed} of {free}");
        let mut s = ViewSway::default();
        settle(&mut s, input(0.0, Vec2::new(0.0, 300.0), FOOTFALL_PHASE), 600);
        assert!(s.breath < 1e-3, "{}", s.breath);
    }

    #[test]
    fn turning_right_leaves_the_arms_to_the_left_and_they_catch_up() {
        let mut s = ViewSway::default();
        settle(&mut s, input(0.0, Vec2::ZERO, 0.0), 60);
        // A steady turn to the right at 0.3 rad/s: a lag of rate × the chase time.
        let mut h = 0.0;
        let mut p = SwayPose::NONE;
        for _ in 0..60 {
            h += 0.3 / 60.0;
            p = s.update(input(h, Vec2::ZERO, 0.0));
        }
        let ahead = p.rotation * Vec3::Y;
        assert!(ahead.x < -0.01, "the weapon points left of the view: {ahead:?}");
        assert!((s.turn[0].value + 0.3 * CHASE_SECONDS).abs() < 2e-3, "{}", s.turn[0].value);
        let p = settle(&mut s, input(h, Vec2::ZERO, 0.0), 120);
        assert!(p.rotation.angle_between(Quat::IDENTITY) < 1e-3, "back on the view: {p:?}");
    }

    #[test]
    fn the_weapon_starts_slowly_and_comes_to_rest_without_swinging_past() {
        let mut s = ViewSway::default();
        settle(&mut s, input(0.0, Vec2::ZERO, 0.0), 60);
        // The view starts turning at once; the weapon's own turn builds up.
        let mut h = 0.0;
        let mut rates = Vec::new();
        for _ in 0..30 {
            h += 0.5 / 60.0;
            s.update(input(h, Vec2::ZERO, 0.0));
            rates.push(s.turn_rate.x);
        }
        assert!(rates[0] < 0.25 * 0.5, "a slow start: {}", rates[0]);
        assert!(rates.windows(2).all(|w| w[1] >= w[0] - 1e-5), "then faster: {rates:?}");
        // The view stops; the weapon slows down and settles on it, never past it.
        let mut last = s.turn_rate.x;
        for _ in 0..120 {
            s.update(input(h, Vec2::ZERO, 0.0));
            assert!(s.turn[0].value <= 1e-6, "past the view: {}", s.turn[0].value);
            assert!(s.turn_rate.x <= last + 1e-5 || s.turn_rate.x < 0.05, "it slows down");
            last = s.turn_rate.x;
        }
    }

    #[test]
    fn the_turn_lag_is_the_same_at_any_frame_rate() {
        let lag_after = |hz: f32| {
            let mut s = ViewSway::default();
            let dt = 1.0 / hz;
            let mut h = 0.0;
            s.update(SwayInput { dt, ..input(h, Vec2::ZERO, 0.0) });
            for _ in 0..(hz / 4.0) as usize {
                h += 0.4 * dt;
                s.update(SwayInput { dt, ..input(h, Vec2::ZERO, 0.0) });
            }
            s.turn[0].value
        };
        let at60 = lag_after(60.0);
        for hz in [120.0, 144.0, 240.0] {
            assert!((lag_after(hz) - at60).abs() < 1e-3, "{hz}: {} against {at60}", lag_after(hz));
        }
    }

    #[test]
    fn a_fast_flick_stays_within_the_limit() {
        let mut s = ViewSway::default();
        let mut h = 0.0;
        s.update(input(h, Vec2::ZERO, 0.0));
        for _ in 0..30 {
            h += 0.3;
            s.update(input(h, Vec2::ZERO, 0.0));
        }
        assert!(s.lag.x.abs() <= MAX_LAG_YAW + 1e-6 && s.lag.x.abs() > MAX_LAG_YAW * 0.9, "{}", s.lag.x);
    }

    #[test]
    fn starting_to_run_leaves_the_weapon_behind_and_a_steady_pace_brings_it_back() {
        let mut s = ViewSway::default();
        settle(&mut s, input(0.0, Vec2::ZERO, 0.0), 60);
        // The body's speed ramps up over 0.12 s (`nvf_play::inertia`), then holds.
        let mut most = 0.0f32;
        for f in 0..60 {
            let v = 344.0 * (f as f32 / 7.0).min(1.0);
            let p = s.update(input(0.0, Vec2::new(0.0, v), 0.0));
            most = most.min(p.offset.y);
        }
        assert!((-MAX_MOVE_LAG..-0.3).contains(&most), "behind while speeding up: {most}");
        let p = settle(&mut s, input(0.0, Vec2::new(0.0, 344.0), 0.0), 120);
        assert!(p.offset.y.abs() < 1e-2, "back at a steady pace: {p:?}");
    }

    #[test]
    fn walking_dips_at_the_footfalls() {
        let mut s = ViewSway::default();
        let v = Vec2::new(0.0, 300.0);
        settle(&mut s, input(0.0, v, FOOTFALL_PHASE), 120);
        let at_footfall = s.update(input(0.0, v, FOOTFALL_PHASE)).offset.z;
        let between = s.update(input(0.0, v, FOOTFALL_PHASE + 0.25)).offset.z;
        let other = s.update(input(0.0, v, FOOTFALL_PHASE + 0.5)).offset.z;
        assert!(at_footfall < -0.5 * BOB_DIP && other < -0.5 * BOB_DIP, "{at_footfall} {other}");
        assert!(between > -0.05, "{between}");
    }

    /// A standing jump of the player (`fnv_phys`: 61.6 units high under 686.7 units/s²)
    /// and its landing, a frame at a time: the lowest and highest offset while rising,
    /// near the top, and just after landing.
    fn jump(land_speed: f32) -> (f32, f32, f32) {
        let g = 686.7;
        let up = (2.0f32 * g * 61.6).sqrt();
        let mut s = ViewSway::default();
        let dt = 1.0 / 60.0;
        let air = |vz: f32| SwayInput { vertical: vz, airborne: true, ..input(0.0, Vec2::ZERO, 0.0) };
        settle(&mut s, input(0.0, Vec2::ZERO, 0.0), 60);
        let mut take_off = 0.0f32;
        let mut top = f32::MIN;
        let mut vz = up;
        while vz > -land_speed {
            let z = s.update(air(vz)).offset.z;
            if vz > up * 0.5 {
                take_off = take_off.min(z);
            }
            if vz.abs() < up * 0.2 {
                top = top.max(z);
            }
            vz -= g * dt;
        }
        let mut landing = 0.0f32;
        for _ in 0..30 {
            landing = landing.min(s.update(input(0.0, Vec2::ZERO, 0.0)).offset.z);
        }
        (take_off, top, landing)
    }

    #[test]
    fn a_jump_dips_at_the_take_off_floats_at_the_top_and_dips_on_landing() {
        let (take_off, top, landing) = jump(291.0);
        assert!(take_off < -0.5, "dips at the take-off: {take_off}");
        assert!(top > 0.0, "rides high near the top: {top}");
        assert!((-MAX_LIFT..-0.4).contains(&landing), "dips on landing: {landing}");
        let (_, _, hard) = jump(700.0);
        assert!(hard < landing - 0.2 && hard >= -MAX_LIFT, "a harder landing dips more: {hard} against {landing}");
    }

    #[test]
    fn the_sights_and_the_air_calm_it() {
        let v = Vec2::new(0.0, 300.0);
        let mut free = ViewSway::default();
        let mut sights = ViewSway::default();
        let mut air = ViewSway::default();
        let a = settle(&mut free, input(0.0, v, FOOTFALL_PHASE), 120).offset.z;
        let b = settle(&mut sights, SwayInput { sights: true, ..input(0.0, v, FOOTFALL_PHASE) }, 120).offset.z;
        let c = settle(&mut air, SwayInput { airborne: true, ..input(0.0, v, FOOTFALL_PHASE) }, 120).offset.z;
        assert!(b > a * 0.5 && b < 0.0, "{a} {b}");
        assert!(c.abs() < 1e-3, "{c}");
    }

    #[test]
    fn a_heavy_weapon_lags_further_and_slower() {
        assert_eq!(heft(0.0), 1.0);
        assert_eq!(heft(LIGHT_WEAPON), 1.0);
        assert_eq!(heft(HEAVY_WEAPON), MOST_HEFT);
        assert_eq!(heft(40.0), MOST_HEFT);
        assert!(heft(11.5) > 1.2 && heft(11.5) < 1.3);
        let steady = |heft: f32| {
            let mut s = ViewSway::default();
            let mut h = 0.0;
            for _ in 0..120 {
                h += 0.2 / 60.0;
                s.update(SwayInput { heft, ..input(h, Vec2::ZERO, 0.0) });
            }
            s.turn[0].value
        };
        assert!((steady(MOST_HEFT) / steady(1.0) - MOST_HEFT).abs() < 0.02);
    }

    #[test]
    fn switched_off_it_fades_out() {
        let mut s = ViewSway::default();
        let v = Vec2::new(0.0, 300.0);
        settle(&mut s, input(0.0, v, FOOTFALL_PHASE), 120);
        let p = settle(&mut s, SwayInput { on: false, ..input(0.0, v, FOOTFALL_PHASE) }, 120);
        assert!(p.offset.length() < 1e-3, "{p:?}");
    }
}
